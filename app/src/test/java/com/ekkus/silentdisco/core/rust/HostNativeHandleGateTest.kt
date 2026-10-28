package com.ekkus.silentdisco.core.rust

import com.google.common.truth.Truth.assertThat
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Test

class HostNativeHandleGateTest {
    @Test
    fun closeWaitsForInFlightNativeSendBeforeDisposingHandle() {
        val gate = HostNativeHandleGate<Any>()
        val handle = Any()
        val sendsStarted = CountDownLatch(1)
        val finishSend = CountDownLatch(1)
        val closeStarted = CountDownLatch(1)
        val disposed = AtomicInteger()
        val executor = Executors.newFixedThreadPool(2)
        try {
            gate.replace(retire = {}, open = { handle }, start = {})
            val send = executor.submit {
                gate.withCurrent {
                    sendsStarted.countDown()
                    check(finishSend.await(5, TimeUnit.SECONDS))
                    assertThat(disposed.get()).isEqualTo(0)
                }
            }
            assertThat(sendsStarted.await(5, TimeUnit.SECONDS)).isTrue()
            val close = executor.submit {
                closeStarted.countDown()
                gate.close { if (it != null) disposed.incrementAndGet() }
            }
            assertThat(closeStarted.await(5, TimeUnit.SECONDS)).isTrue()
            assertThat(disposed.get()).isEqualTo(0)
            finishSend.countDown()
            send.get(5, TimeUnit.SECONDS)
            close.get(5, TimeUnit.SECONDS)
            assertThat(disposed.get()).isEqualTo(1)
            assertThat(gate.currentHandle()).isNull()
            assertThat(gate.withCurrent { "should not run" }).isNull()
        } finally {
            finishSend.countDown()
            executor.shutdownNow()
        }
    }

    @Test
    fun rebindSerializesWithInFlightSendAndRetiresOnlyTheOldGeneration() {
        val gate = HostNativeHandleGate<Any>()
        val old = Any()
        val replacement = Any()
        val sendStarted = CountDownLatch(1)
        val finishSend = CountDownLatch(1)
        val disposed = mutableListOf<Any>()
        val executor = Executors.newFixedThreadPool(2)
        try {
            gate.replace(retire = {}, open = { old }, start = {})
            val send = executor.submit {
                gate.withCurrent {
                    assertThat(it).isSameInstanceAs(old)
                    sendStarted.countDown()
                    check(finishSend.await(5, TimeUnit.SECONDS))
                    assertThat(disposed).isEmpty()
                }
            }
            assertThat(sendStarted.await(5, TimeUnit.SECONDS)).isTrue()
            val rebind = executor.submit {
                gate.replace(
                    retire = { if (it != null) disposed.add(it) },
                    open = { replacement },
                    start = { assertThat(it).isSameInstanceAs(replacement) },
                )
            }
            finishSend.countDown()
            send.get(5, TimeUnit.SECONDS)
            rebind.get(5, TimeUnit.SECONDS)
            assertThat(disposed).containsExactly(old)
            assertThat(gate.withCurrent { it }).isSameInstanceAs(replacement)
            var staleCommandRan = false
            assertThat(gate.withExpected(old) { staleCommandRan = true }).isNull()
            assertThat(staleCommandRan).isFalse()
            assertThat(gate.withExpected(replacement) { "current session" })
                .isEqualTo("current session")
            gate.close { if (it != null) disposed.add(it) }
            assertThat(disposed).containsExactly(old, replacement).inOrder()
        } finally {
            finishSend.countDown()
            executor.shutdownNow()
        }
    }

    @Test
    fun failedRebindOrPollerStartNeverPublishesAnOrphanHandle() {
        val gate = HostNativeHandleGate<Any>()
        val old = Any()
        val orphan = Any()
        val disposed = mutableListOf<Any>()
        val retire: (Any?) -> Unit = { if (it != null) disposed.add(it) }
        gate.replace(retire = retire, open = { old }, start = {})
        val failedOpen = runCatching {
            gate.replace(retire = retire, open = { error("bind failed") }, start = {})
        }
        assertThat(failedOpen.isFailure).isTrue()
        assertThat(disposed).containsExactly(old)
        assertThat(gate.currentHandle()).isNull()
        val failedStart = runCatching {
            gate.replace(retire = retire, open = { orphan }, start = { error("poll failed") })
        }
        assertThat(failedStart.isFailure).isTrue()
        assertThat(disposed).containsExactly(old, orphan).inOrder()
        assertThat(gate.currentHandle()).isNull()
    }

    @Test
    fun approvalAndRouteAuthorizationStayOnOriginalHandleAcrossConcurrentRebind() {
        val gate = HostNativeHandleGate<Any>()
        val old = Any()
        val replacement = Any()
        val started = CountDownLatch(1)
        val release = CountDownLatch(1)
        val retired = mutableListOf<Any>()
        val pool = Executors.newFixedThreadPool(2)
        try {
            gate.replace(retire = {}, open = { old }, start = {})
            val approval = pool.submit<HostJoinAuthorizationResult<String>?> {
                gate.deliverAndAuthorize(
                    expected = old,
                    deliver = {
                        assertThat(it).isSameInstanceAs(old)
                        started.countDown()
                        check(release.await(5, TimeUnit.SECONDS))
                        "approved"
                    },
                    wasDelivered = { it == "approved" },
                    authorize = { assertThat(it).isSameInstanceAs(old) },
                    disconnect = { error("successful approval must not disconnect") },
                )
            }
            assertThat(started.await(5, TimeUnit.SECONDS)).isTrue()
            val rebind = pool.submit {
                gate.replace(
                    retire = { if (it != null) retired.add(it) },
                    open = { replacement },
                    start = {},
                )
            }
            release.countDown()
            assertThat(approval.get(5, TimeUnit.SECONDS)?.authorized).isTrue()
            rebind.get(5, TimeUnit.SECONDS)
            assertThat(retired).containsExactly(old)
            assertThat(gate.deliverAndAuthorize(
                expected = old,
                deliver = { error("stale approval must not be sent") },
                wasDelivered = { true },
                authorize = { error("stale authorization must not run") },
                disconnect = {},
            )).isNull()
            assertThat(gate.currentHandle()).isSameInstanceAs(replacement)
        } finally {
            release.countDown()
            pool.shutdownNow()
        }
    }

    @Test
    fun failedRouteAuthorizationRetiresPeerAndNeverReportsAdmissionSuccess() {
        val gate = HostNativeHandleGate<Any>()
        val handle = Any()
        val calls = mutableListOf<String>()
        gate.replace(retire = {}, open = { handle }, start = {})
        val result = gate.deliverAndAuthorize(
            expected = handle,
            deliver = { calls.add("approval"); "delivered" },
            wasDelivered = { true },
            authorize = {
                calls.add("authorize")
                error("UDP route rejected")
            },
            disconnect = { calls.add("disconnect") },
        )
        assertThat(result?.authorized).isFalse()
        assertThat(result?.failure).contains("UDP route rejected")
        assertThat(calls).containsExactly("approval", "authorize", "disconnect").inOrder()

        calls.clear()
        val undelivered = gate.deliverAndAuthorize(
            expected = handle,
            deliver = { calls.add("approval"); "failed" },
            wasDelivered = { false },
            authorize = { calls.add("authorize") },
            disconnect = { calls.add("disconnect") },
        )
        assertThat(undelivered?.authorized).isFalse()
        assertThat(calls).containsExactly("approval")
        gate.close {}
    }
}
