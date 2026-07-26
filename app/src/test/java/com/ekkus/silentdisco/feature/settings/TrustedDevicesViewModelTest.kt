package com.ekkus.silentdisco.feature.settings

import android.app.Application
import com.ekkus.silentdisco.core.rust.RustTrustedDevice
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.mockito.kotlin.mock

@OptIn(ExperimentalCoroutinesApi::class)
class TrustedDevicesViewModelTest {
    private val dispatcher = StandardTestDispatcher()

    @Before
    fun setUpMainDispatcher() {
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun resetMainDispatcher() {
        Dispatchers.resetMain()
    }

    @Test
    fun refreshLoadsAuthoritativeRustList() = runTest(dispatcher) {
        val device = RustTrustedDevice("listener-1", "Alex's phone", 100L)
        val store = FakeTrustedDeviceStore(devices = mutableListOf(device))
        val viewModel = TrustedDevicesViewModel(mock<Application>(), store)

        viewModel.refresh()
        advanceUntilIdle()

        assertThat(store.initializeCalls).isEqualTo(1)
        assertThat(store.listCalls).isEqualTo(1)
        assertThat(viewModel.uiState.value.devices).containsExactly(device)
        assertThat(viewModel.uiState.value.error).isNull()
    }

    @Test
    fun deletionReloadsRustListBeforeRemovingRowFromState() = runTest(dispatcher) {
        val removed = RustTrustedDevice("listener-1", "Alex's phone", 100L)
        val remaining = RustTrustedDevice("listener-2", "Riley's phone", 200L)
        val store = FakeTrustedDeviceStore(devices = mutableListOf(removed, remaining))
        val viewModel = TrustedDevicesViewModel(mock<Application>(), store)
        viewModel.refresh()
        advanceUntilIdle()

        viewModel.deleteDevice(removed.deviceId)
        assertThat(viewModel.uiState.value.devices).containsExactly(removed, remaining).inOrder()
        assertThat(viewModel.uiState.value.deletingDeviceId).isEqualTo(removed.deviceId)

        advanceUntilIdle()

        assertThat(store.deletedDeviceIds).containsExactly(removed.deviceId)
        assertThat(viewModel.uiState.value.devices).containsExactly(remaining)
        assertThat(viewModel.uiState.value.message).contains("need approval")
    }

    @Test
    fun deleteFailureRetainsLastAuthoritativeListAndShowsError() = runTest(dispatcher) {
        val device = RustTrustedDevice("listener-1", "Alex's phone", 100L)
        val store = FakeTrustedDeviceStore(
            devices = mutableListOf(device),
            deleteFailure = IllegalStateException("database busy"),
        )
        val viewModel = TrustedDevicesViewModel(mock<Application>(), store)
        viewModel.refresh()
        advanceUntilIdle()

        viewModel.deleteDevice(device.deviceId)
        advanceUntilIdle()

        assertThat(viewModel.uiState.value.devices).containsExactly(device)
        assertThat(viewModel.uiState.value.deletingDeviceId).isNull()
        assertThat(viewModel.uiState.value.error).contains("could not be removed")
    }

    private class FakeTrustedDeviceStore(
        val devices: MutableList<RustTrustedDevice>,
        private val deleteFailure: Throwable? = null,
    ) : TrustedDeviceStore {
        var initializeCalls = 0
        var listCalls = 0
        val deletedDeviceIds = mutableListOf<String>()

        override suspend fun initialize() {
            initializeCalls += 1
        }

        override suspend fun listTrustedDevices(): List<RustTrustedDevice> {
            listCalls += 1
            return devices.toList()
        }

        override suspend fun deleteTrustedDevice(deviceId: String): Boolean {
            deleteFailure?.let { throw it }
            deletedDeviceIds += deviceId
            return devices.removeAll { it.deviceId == deviceId }
        }

        override suspend fun close() = Unit
    }
}
