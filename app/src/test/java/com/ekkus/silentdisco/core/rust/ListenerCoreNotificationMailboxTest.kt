package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import java.util.concurrent.CompletableFuture
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import org.junit.Test

/** Production listener classification and bounded slow-consumer semantics. */
class ListenerCoreNotificationMailboxTest {
    @Test
    fun snapshotsConflateOutsideQueueAndTelemetryFloodCannotLoseJoinApprovalOrStop() {
        var latestSnapshot: String? = null
        val mailbox = ListenerCoreNotificationMailbox<String>(
            onSnapshot = { event ->
                if (event.startsWith("snapshot:")) {
                    latestSnapshot = event
                    true
                } else {
                    false
                }
            },
            isDiagnostic = { it.startsWith("diagnostic:") },
            capacity = 3,
        )
        try {
            repeat(500) { mailbox.publish("snapshot:$it") }
            assertThat(latestSnapshot).isEqualTo("snapshot:499")
            mailbox.publish("join")
            mailbox.publish("approval")
            repeat(1_000) { mailbox.publish("diagnostic:$it") }
            assertThat(mailbox.droppedDiagnostics).isEqualTo(999)
            val waitingStop = CompletableFuture.runAsync { mailbox.publish("stop") }
            Thread.sleep(40)
            assertThat(waitingStop.isDone).isFalse()
            val observed = runBlocking { mailbox.events.take(4).toList() }
            waitingStop.get(2, TimeUnit.SECONDS)
            assertThat(observed)
                .containsExactly("join", "approval", "diagnostic:0", "stop").inOrder()
            assertThat(mailbox.criticalTimeouts).isEqualTo(0)
        } finally {
            mailbox.close()
        }
    }

    @Test
    fun stalledListenerCollectorFailsCriticalOperationWithinFiniteDeadline() {
        val mailbox = ListenerCoreNotificationMailbox<String>(
            onSnapshot = { false },
            isDiagnostic = { it.startsWith("diagnostic:") },
            capacity = 1,
            criticalTimeoutMs = 30,
        )
        try {
            mailbox.publish("join")
            val failure = runCatching { mailbox.publish("stop") }.exceptionOrNull()
            assertThat(failure).isInstanceOf(IllegalStateException::class.java)
            assertThat(failure?.message).contains("critical notification queue saturated")
            assertThat(mailbox.criticalTimeouts).isEqualTo(1)
            assertThat(runBlocking { mailbox.events.take(1).toList() })
                .containsExactly("join")
        } finally {
            mailbox.close()
        }
    }

    @Test
    fun closingListenerReleasesBlockedEffectAndDrainsAcceptedJoin() {
        val mailbox = ListenerCoreNotificationMailbox<String>(
            onSnapshot = { false },
            isDiagnostic = { false },
            capacity = 1,
            criticalTimeoutMs = 2_000,
        )
        mailbox.publish("join")
        val pending = CompletableFuture.supplyAsync {
            runCatching { mailbox.publish("stop") }.exceptionOrNull()
        }
        Thread.sleep(40)
        assertThat(pending.isDone).isFalse()
        mailbox.close()
        assertThat(pending.get(2, TimeUnit.SECONDS)).isNotNull()
        assertThat(mailbox.criticalTimeouts).isEqualTo(0)
        assertThat(runBlocking { mailbox.events.toList() }).containsExactly("join")
        mailbox.publish("retired-generation-approval")
        mailbox.close()
    }
}
