package com.ekkus.silentdisco.app

import android.app.Application
import android.content.ContentResolver
import android.content.Context
import android.content.SharedPreferences
import android.net.wifi.WifiManager
import com.ekkus.silentdisco.core.rust.FakeHostCoreController
import com.ekkus.silentdisco.core.rust.FakeListenerCoreController
import com.ekkus.silentdisco.core.rust.FakeRustDomainStore
import com.ekkus.silentdisco.core.transport.FakeBleTransport
import com.ekkus.silentdisco.core.transport.FakeSessionTransport
import org.mockito.kotlin.any
import org.mockito.kotlin.anyOrNull
import org.mockito.kotlin.mock
import org.mockito.kotlin.whenever

/**
 * Everything a genuine `MainViewModel` effect-runner test needs: a real
 * `MainViewModel` wired entirely to recording fakes, so
 * `executeRustPlatformEffect`/`executeRustTransportEffect`/
 * `executeRustStorageEffect`/`executeRustListenerPlatformEffect` run as real
 * production code (driven via [FakeHostCoreController.emit]/
 * [FakeListenerCoreController.emit]) instead of being invoked reflectively
 * or re-implemented for the test.
 *
 * `hostTransportController`/`listenerTransportController` are deliberately
 * left as the real, unbound production classes (`HostTransportController`/
 * `ListenerTransportController`): with no bound native transport handle,
 * their send/broadcast methods genuinely return `null`/zero-peer deliveries
 * without needing a fake -- see the effect tests that assert on that exact,
 * real "zero peers delivered" fact. A bound handle needs a real native Rust
 * socket, which is out of reach for a JVM unit test; that path remains
 * instrumented/physical-device-only.
 */
class MainViewModelHarness(
    val viewModel: MainViewModel,
    val hostCoreController: FakeHostCoreController,
    val listenerCoreController: FakeListenerCoreController,
    val domainStore: FakeRustDomainStore,
    val bleService: FakeBleTransport,
    val wifiDirectService: FakeSessionTransport,
)

/**
 * Builds a real [Application] mock stubbed just enough for `MainViewModel`'s
 * constructor and `init` block to run without touching real Android
 * framework state -- every collaborator that would otherwise require a real
 * `Context` ([com.ekkus.silentdisco.core.transport.BleDiscoveryService],
 * [com.ekkus.silentdisco.core.transport.WifiDirectTransportService],
 * [com.ekkus.silentdisco.platform.persistence.AndroidRustDomainStore]) is
 * replaced by an injected fake instead, so this stub surface only needs to
 * cover the handful of Android calls `MainViewModel` still makes
 * unconditionally (`AudioFileDecoder`'s `ContentResolver`,
 * `WifiLowLatencyNetworkLock`'s `WifiManager`, and
 * `DeviceIdentityStore`'s `SharedPreferences`).
 */
private fun fakeApplication(): Application {
    val application = mock<Application>()
    whenever(application.contentResolver).thenReturn(mock<ContentResolver>())

    val wifiLock = mock<WifiManager.WifiLock>()
    val wifiManager = mock<WifiManager>()
    whenever(wifiManager.createWifiLock(any(), any())).thenReturn(wifiLock)
    whenever(application.getSystemService(Context.WIFI_SERVICE)).thenReturn(wifiManager)

    val sharedPreferences = mock<SharedPreferences>()
    whenever(sharedPreferences.getString(any(), anyOrNull())).thenReturn("test-device-id")
    whenever(application.getSharedPreferences(any(), any())).thenReturn(sharedPreferences)

    return application
}

/** Constructs a real [MainViewModel] wired entirely to recording fakes; see [MainViewModelHarness]. */
fun newTestMainViewModelHarness(): MainViewModelHarness {
    val hostCoreController = FakeHostCoreController()
    val listenerCoreController = FakeListenerCoreController()
    val domainStore = FakeRustDomainStore()
    val bleService = FakeBleTransport()
    val wifiDirectService = FakeSessionTransport()
    val viewModel = MainViewModel(
        application = fakeApplication(),
        domainStore = domainStore,
        hostCoreFactory = { hostCoreController },
        listenerCoreFactory = { listenerCoreController },
        bleService = bleService,
        wifiDirectService = wifiDirectService,
    )
    return MainViewModelHarness(
        viewModel = viewModel,
        hostCoreController = hostCoreController,
        listenerCoreController = listenerCoreController,
        domainStore = domainStore,
        bleService = bleService,
        wifiDirectService = wifiDirectService,
    )
}
