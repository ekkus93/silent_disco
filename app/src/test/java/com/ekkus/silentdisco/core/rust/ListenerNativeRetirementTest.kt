package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class ListenerNativeRetirementTest {
    @Test
    fun unexpectedLeaveShutdownAndCloseFailuresStillAttemptEveryStage() {
        val handle = Any()
        val order = mutableListOf<String>()
        val leave = IllegalStateException("leave failed")
        val shutdown = AssertionError("shutdown failed")
        val close = IllegalArgumentException("close failed")
        val failure = runCatching {
            retireListenerNativeHandle(
                handle = handle,
                reason = "Stop",
                sendDisconnect = { native, reason ->
                    assertThat(native).isSameInstanceAs(handle)
                    assertThat(reason).isEqualTo("Stop")
                    order += "leave"
                    throw leave
                },
                benignDisconnectFailure = { false },
                shutdown = { order += "shutdown"; throw shutdown },
                awaitPoll = { order += "drain" },
                close = { order += "close"; throw close },
            )
        }.exceptionOrNull()
        assertThat(order).containsExactly("leave", "shutdown", "drain", "close").inOrder()
        assertThat(failure).isSameInstanceAs(leave)
        assertThat(leave.suppressed.toList()).containsExactly(shutdown)
        assertThat(shutdown.suppressed.toList()).containsExactly(close)
    }

    @Test
    fun failedPollDrainNeverReleasesPossiblyInUseNativeHandle() {
        val order = mutableListOf<String>()
        val leave = IllegalStateException("leave failed")
        val poll = AssertionError("native poll still active")
        val failure = runCatching {
            retireListenerNativeHandle(
                handle = Any(),
                reason = "Stop",
                sendDisconnect = { _, _ -> order += "leave"; throw leave },
                benignDisconnectFailure = { false },
                shutdown = { order += "shutdown" },
                awaitPoll = { order += "drain"; throw poll },
                close = { order += "close" },
            )
        }.exceptionOrNull()
        assertThat(order).containsExactly("leave", "shutdown", "drain").inOrder()
        assertThat(failure).isSameInstanceAs(leave)
        assertThat(leave.suppressed.toList()).containsExactly(poll)
    }

    @Test
    fun alreadyClosedPeerLeaveErrorIsBenignButUnexpectedErrorsAreNot() {
        val order = mutableListOf<String>()
        val brokenPipe = IllegalStateException("known closed peer")
        retireListenerNativeHandle(
            handle = Any(),
            reason = "Leave",
            sendDisconnect = { _, _ -> order += "leave"; throw brokenPipe },
            benignDisconnectFailure = { it === brokenPipe },
            shutdown = { order += "shutdown" },
            awaitPoll = { order += "drain" },
            close = { order += "close" },
        )
        assertThat(order).containsExactly("leave", "shutdown", "drain", "close").inOrder()
    }

    @Test
    fun shutdownFailureWithoutLeaveStillDrainsAndClosesAndNullHandleIsNoOp() {
        val order = mutableListOf<String>()
        val failure = IllegalStateException("shutdown failed")
        val result = runCatching {
            retireListenerNativeHandle(
                handle = Any(),
                reason = null,
                sendDisconnect = { _, _ -> order += "unexpected leave" },
                benignDisconnectFailure = { false },
                shutdown = { order += "shutdown"; throw failure },
                awaitPoll = { order += "drain" },
                close = { order += "close" },
            )
        }.exceptionOrNull()
        assertThat(result).isSameInstanceAs(failure)
        assertThat(order).containsExactly("shutdown", "drain", "close").inOrder()

        retireListenerNativeHandle(
            handle = null as Any?,
            reason = "Leave",
            sendDisconnect = { _, _ -> error("null handle must not be used") },
            benignDisconnectFailure = { false },
            shutdown = { error("null handle must not be used") },
            awaitPoll = { error("null handle must not be used") },
            close = { error("null handle must not be used") },
        )
    }
}
