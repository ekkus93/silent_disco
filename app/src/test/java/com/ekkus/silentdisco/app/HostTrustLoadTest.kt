package com.ekkus.silentdisco.app

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Before
import org.junit.Test

/**
 * Remediation F1: a host's listener trust comes only from its own stored
 * records, so starting a host session must load them for the host TLS key
 * first, and a failed load must stop the start visibly.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class HostTrustLoadTest {
    private val dispatcher = StandardTestDispatcher()

    @Before
    fun setUpMain() {
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun resetMain() {
        Dispatchers.resetMain()
    }

    /** The start runs part of its work on [Dispatchers.IO]; wait for [done] with a bound. */
    private fun TestScope.settle(done: () -> Boolean) {
        repeat(400) {
            advanceUntilIdle()
            if (done()) return
            Thread.sleep(5)
        }
        advanceUntilIdle()
    }

    @Test
    fun startingAHostLoadsItsTrustRecordsForTheHostTlsKeyBeforeCreatingTheSession() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.domainStore.databaseHandle = 7L
        harness.hostCoreController.loadHostTrustCount = 2u
        advanceUntilIdle()

        harness.viewModel.createRustHostSession()
        settle { harness.hostCoreController.configureAndCreateCalls.isNotEmpty() }

        val call = harness.hostCoreController.loadHostTrustCalls.single()
        assertThat(call.first).isEqualTo(7L)
        assertThat(call.second).isEqualTo(FixedHostTlsSigner.publicKeyDer)
        assertThat(harness.hostCoreController.configureAndCreateCalls).hasSize(1)
    }

    @Test
    fun aFailedTrustLoadStopsTheStartWithAVisibleError() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.hostCoreController.loadHostTrustFailure = IllegalStateException("WrongHostIdentity")
        advanceUntilIdle()

        harness.viewModel.createRustHostSession()
        settle { harness.viewModel.uiState.value.lastError?.contains("trusted listeners") == true }

        assertThat(harness.viewModel.uiState.value.lastError)
            .isEqualTo("Could not load trusted listeners: WrongHostIdentity")
        assertThat(harness.hostCoreController.configureAndCreateCalls).isEmpty()
    }

    @Test
    fun aDatabaseThatIsNotOpenStopsTheStartWithAVisibleError() = runTest(dispatcher) {
        val harness = newTestMainViewModelHarness()
        harness.domainStore.databaseHandleFailure = IllegalStateException("database closed")
        advanceUntilIdle()

        harness.viewModel.createRustHostSession()
        settle { harness.viewModel.uiState.value.lastError?.contains("trusted listeners") == true }

        assertThat(harness.viewModel.uiState.value.lastError)
            .isEqualTo("Could not load trusted listeners: database closed")
        assertThat(harness.hostCoreController.loadHostTrustCalls).isEmpty()
        assertThat(harness.hostCoreController.configureAndCreateCalls).isEmpty()
    }
}
