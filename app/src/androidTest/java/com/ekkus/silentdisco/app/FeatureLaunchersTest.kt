package com.ekkus.silentdisco.app

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.net.Uri
import androidx.activity.compose.LocalActivityResultRegistryOwner
import androidx.activity.result.ActivityResultRegistry
import androidx.activity.result.ActivityResultRegistryOwner
import androidx.activity.result.contract.ActivityResultContract
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.core.app.ActivityOptionsCompat
import androidx.core.content.ContextCompat
import androidx.test.core.app.ApplicationProvider
import android.content.Context
import android.content.pm.PackageManager
import com.google.common.truth.Truth.assertThat
import com.google.zxing.client.android.Intents
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import org.junit.Rule
import org.junit.Test

/**
 * The feature launcher helpers (God-file refactor TODO §6.3) against a
 * recording [ActivityResultRegistry], so no other app is opened: each launch
 * is recorded and answered with a canned result.
 */
class FeatureLaunchersTest {
    @get:Rule
    val composeRule = createComposeRule()

    private class RecordingRegistry : ActivityResultRegistry() {
        val launches = mutableListOf<Pair<ActivityResultContract<*, *>, Any?>>()
        var answer: (contract: ActivityResultContract<*, *>, input: Any?) -> Pair<Int, Intent?>? = { _, _ -> null }
        val typedAnswers = mutableMapOf<Class<*>, Any?>()

        override fun <I, O> onLaunch(
            requestCode: Int,
            contract: ActivityResultContract<I, O>,
            input: I,
            options: ActivityOptionsCompat?,
        ) {
            launches += contract to input
            if (typedAnswers.containsKey(contract.javaClass)) {
                @Suppress("UNCHECKED_CAST")
                dispatchResult(requestCode, typedAnswers[contract.javaClass] as O)
                return
            }
            answer(contract, input)?.let { (code, data) -> dispatchResult(requestCode, code, data) }
        }
    }

    private val registry = RecordingRegistry()
    private val owner = object : ActivityResultRegistryOwner {
        override val activityResultRegistry: ActivityResultRegistry get() = registry
    }

    @Test
    fun theAudioPickerOpensAudioDocumentsAndRoutesToTheChosenDestination() {
        val setup = mutableListOf<List<Uri>>()
        val live = mutableListOf<List<Uri>>()
        lateinit var picker: AudioPicker
        val song = Uri.parse("content://silent-disco-test/song")
        registry.typedAnswers[ActivityResultContracts.OpenMultipleDocuments::class.java] = listOf(song)
        composeRule.setContent {
            CompositionLocalProvider(LocalActivityResultRegistryOwner provides owner) {
                picker = rememberAudioPicker(onSetup = { setup += it }, onLiveQueue = { live += it })
            }
        }

        composeRule.runOnIdle { picker.launch(AudioPickerDestination.LIVE_QUEUE) }
        composeRule.runOnIdle { picker.launch(AudioPickerDestination.SETUP) }

        composeRule.runOnIdle {
            assertThat(registry.launches.map { (it.second as Array<*>).toList() })
                .containsExactly(listOf("audio/*"), listOf("audio/*"))
            assertThat(live).containsExactly(listOf(song))
            assertThat(setup).containsExactly(listOf(song))
        }
    }

    @Test
    fun theQrScannerReportsAScanOrACancelAndAskingForTheCameraOpensItOnlyWhenGranted() {
        val scanned = mutableListOf<String>()
        var cancelled = 0
        var denied = 0
        var rationale = 0
        lateinit var scanner: QrScanner
        var scanResult: Pair<Int, Intent?> = Activity.RESULT_OK to Intent().putExtra(Intents.Scan.RESULT, "signed-payload")
        registry.answer = { contract, _ -> if (contract is ScanContract) scanResult else null }
        composeRule.setContent {
            CompositionLocalProvider(LocalActivityResultRegistryOwner provides owner) {
                scanner = rememberQrScanner(
                    onScanned = { scanned += it },
                    onCancelled = { cancelled += 1 },
                    onRationaleNeeded = { rationale += 1 },
                    onCameraDenied = { denied += 1 },
                )
            }
        }

        // The phone's real camera permission decides the branch; this test never changes it.
        val context = ApplicationProvider.getApplicationContext<Context>()
        val cameraGranted = ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
            PackageManager.PERMISSION_GRANTED
        composeRule.runOnIdle { scanner.launch() }
        composeRule.runOnIdle {
            if (cameraGranted) {
                assertThat(scanned).containsExactly("signed-payload")
                assertThat(rationale).isEqualTo(0)
                assertThat(registry.launches.single().second).isInstanceOf(ScanOptions::class.java)
            } else {
                assertThat(rationale).isEqualTo(1)
                assertThat(registry.launches).isEmpty()
            }
        }
        scanned.clear()

        // The scanner's own results, reached through the permission grant path.
        scanResult = Activity.RESULT_CANCELED to null
        registry.typedAnswers[ActivityResultContracts.RequestPermission::class.java] = true
        composeRule.runOnIdle { scanner.requestCameraPermission() }
        composeRule.runOnIdle { assertThat(cancelled).isEqualTo(1) }

        registry.typedAnswers[ActivityResultContracts.RequestPermission::class.java] = false
        composeRule.runOnIdle { scanner.requestCameraPermission() }
        composeRule.runOnIdle {
            assertThat(denied).isEqualTo(1)
            assertThat(registry.launches.last().second).isEqualTo(Manifest.permission.CAMERA)
        }

        registry.typedAnswers[ActivityResultContracts.RequestPermission::class.java] = true
        scanResult = Activity.RESULT_OK to Intent().putExtra(Intents.Scan.RESULT, "after-grant")
        composeRule.runOnIdle { scanner.requestCameraPermission() }
        composeRule.runOnIdle {
            assertThat(scanned).containsExactly("after-grant")
            assertThat(denied).isEqualTo(1)
        }
    }

    @Test
    fun runtimePermissionsCompleteTheRequestOnlyWhenEveryNeededPermissionIsHeld() {
        val recorded = mutableListOf<Pair<String, Boolean>>()
        val request = PermissionRequestState()
        lateinit var launcher: androidx.activity.compose.ManagedActivityResultLauncher<Array<String>, Map<String, Boolean>>
        val action = PermissionAction.LISTENER_PERMISSION_ONLY
        val needed = action.context.androidPermissions()
        composeRule.setContent {
            CompositionLocalProvider(LocalActivityResultRegistryOwner provides owner) {
                launcher = rememberRuntimePermissionLauncher(
                    onPermissionResult = { permission, granted -> recorded += permission to granted },
                    request = request,
                    uiState = AppUiState(),
                )
            }
        }

        registry.typedAnswers[ActivityResultContracts.RequestMultiplePermissions::class.java] =
            needed.associateWith { it != needed.first() }
        composeRule.runOnIdle {
            request.request(action)
            launcher.launch(needed)
        }
        composeRule.runOnIdle {
            assertThat(recorded.map { it.first }).containsExactlyElementsIn(needed.toList())
            assertThat(request.grantedAction).isNull()
        }

        registry.typedAnswers[ActivityResultContracts.RequestMultiplePermissions::class.java] =
            needed.associateWith { true }
        composeRule.runOnIdle {
            request.request(action)
            launcher.launch(needed)
        }
        composeRule.runOnIdle { assertThat(request.grantedAction).isEqualTo(action) }
    }
}
