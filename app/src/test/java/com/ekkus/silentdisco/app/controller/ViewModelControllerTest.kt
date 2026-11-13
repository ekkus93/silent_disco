package com.ekkus.silentdisco.app.controller

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineExceptionHandler
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertThrows
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class ViewModelControllerTest {
    private class Recording(parent: CoroutineScope, private val log: MutableList<String>, private val name: String) :
        ViewModelController(parent) {
        var released = 0
        fun startWork(): Job = scope.launch { awaitCancellation() }
        fun startFailingWork(): Job = scope.launch { error("boom") }
        override fun release() {
            released += 1
            log += name
        }
    }

    private fun TestScope.parent(): CoroutineScope =
        CoroutineScope(SupervisorJob() + StandardTestDispatcher(testScheduler))

    @Test
    fun closeCancelsTheControllersWorkThenReleases() = runTest {
        val parent = parent()
        val controller = Recording(parent, mutableListOf(), "a")
        val work = controller.startWork()
        advanceUntilIdle()

        controller.close()
        advanceUntilIdle()

        assertThat(work.isCancelled).isTrue()
        assertThat(controller.released).isEqualTo(1)
        parent.cancel()
    }

    @Test
    fun clearingTheParentCancelsTheControllersWork() = runTest {
        val parent = parent()
        val work = Recording(parent, mutableListOf(), "a").startWork()
        advanceUntilIdle()

        parent.cancel()
        advanceUntilIdle()

        assertThat(work.isCancelled).isTrue()
    }

    @Test
    fun oneFailingJobDoesNotCancelItsSiblings() = runTest {
        val failures = mutableListOf<Throwable>()
        // As on Android, an uncaught failure reaches the scope's handler.
        val parent = CoroutineScope(
            SupervisorJob() + StandardTestDispatcher(testScheduler) +
                CoroutineExceptionHandler { _, error -> failures += error },
        )
        val controller = Recording(parent, mutableListOf(), "a")
        val sibling = controller.startWork()
        val failing = controller.startFailingWork()
        advanceUntilIdle()

        assertThat(failures.map { it.message }).containsExactly("boom")
        assertThat(failing.isCancelled).isTrue()
        assertThat(sibling.isActive).isTrue()
        parent.cancel()
    }

    @Test
    fun controllersCloseInReverseOrderAndAllCloseDespiteAFailure() {
        val log = mutableListOf<String>()
        val controllers = ViewModelControllers()
        controllers.add(AutoCloseable { log += "first" })
        controllers.add(AutoCloseable {
            log += "second"
            error("second failed")
        })
        controllers.add(AutoCloseable {
            log += "third"
            error("third failed")
        })

        val error = assertThrows(IllegalStateException::class.java) { controllers.closeAll() }

        assertThat(log).containsExactly("third", "second", "first").inOrder()
        assertThat(error.message).isEqualTo("third failed")
        assertThat(error.suppressed.map { it.message }).containsExactly("second failed")
    }
}
