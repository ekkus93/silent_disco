package com.ekkus.silentdisco.core.transport

import com.google.common.truth.Truth.assertThat
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Test

class MdnsDiscoveryLedgerTest {
    private fun ledger(pending: Int = 3, sessions: Int = 2, lost: Int = 2) =
        MdnsDiscoveryLedger<String, String>(pending, sessions, lost)

    @Test
    fun fullResolveQueueRefusesNewcomersReportsOnceAndKeepsExistingOrder() {
        val ledger = ledger()
        listOf("a", "b", "c").forEach {
            assertThat(ledger.offerPending(it, "p-$it")).isEqualTo(LedgerAdmission.ACCEPTED)
        }

        assertThat(ledger.offerPending("d", "p-d")).isEqualTo(LedgerAdmission.REJECTED_FIRST)
        assertThat(ledger.offerPending("e", "p-e")).isEqualTo(LedgerAdmission.REJECTED_REPEATED)
        assertThat(ledger.rejectedPendingCount).isEqualTo(2)

        // Existing entries are never evicted by a newcomer, and keep FIFO order.
        assertThat(ledger.pollPending()).isEqualTo("p-a")
        // Room again: admission resumes and a later overflow is reported anew.
        assertThat(ledger.offerPending("d", "p-d")).isEqualTo(LedgerAdmission.ACCEPTED)
        assertThat(ledger.offerPending("f", "p-f")).isEqualTo(LedgerAdmission.REJECTED_FIRST)
        assertThat(generateSequence { ledger.pollPending() }.toList()).containsExactly("p-b", "p-c", "p-d").inOrder()
    }

    @Test
    fun repeatAnnouncementOfAPendingNameKeepsItsPlaceAndUsesNoExtraCapacity() {
        val ledger = ledger(pending = 2)
        ledger.offerPending("a", "first")
        ledger.offerPending("b", "other")

        assertThat(ledger.offerPending("a", "second")).isEqualTo(LedgerAdmission.ACCEPTED)

        assertThat(ledger.pollPending()).isEqualTo("second")
        assertThat(ledger.pollPending()).isEqualTo("other")
    }

    @Test
    fun fullSessionListRefusesNewNamesButStillUpdatesAnExistingOne() {
        val ledger = ledger(sessions = 2)
        assertThat(ledger.admitSession("x", "x1")).isEqualTo(LedgerAdmission.ACCEPTED)
        assertThat(ledger.admitSession("y", "y1")).isEqualTo(LedgerAdmission.ACCEPTED)

        assertThat(ledger.admitSession("z", "z1")).isEqualTo(LedgerAdmission.REJECTED_FIRST)
        assertThat(ledger.admitSession("x", "x2")).isEqualTo(LedgerAdmission.ACCEPTED)
        assertThat(ledger.sessionsSnapshot()).containsExactly("x2", "y1")

        assertThat(ledger.markLost("x")).isTrue()
        assertThat(ledger.admitSession("z", "z1")).isEqualTo(LedgerAdmission.ACCEPTED)
        assertThat(ledger.rejectedSessionCount).isEqualTo(1)
    }

    @Test
    fun lostNameMemoryIsBoundedAndForgetsTheOldestFirst() {
        val ledger = ledger(lost = 2)
        listOf("a", "b", "c").forEach { ledger.markLost(it) }

        assertThat(ledger.isLost("a")).isFalse()
        assertThat(ledger.isLost("b")).isTrue()
        assertThat(ledger.isLost("c")).isTrue()
    }

    @Test
    fun aLostServiceThatReappearsIsNoLongerLostAndMarkLostDropsItsPendingResolve() {
        val ledger = ledger()
        ledger.offerPending("a", "p-a")
        assertThat(ledger.markLost("a")).isFalse()
        assertThat(ledger.isLost("a")).isTrue()
        assertThat(ledger.pollPending()).isNull()

        ledger.offerPending("a", "p-a2")
        assertThat(ledger.isLost("a")).isFalse()
    }

    @Test
    fun clearEmptiesEverythingAndResetsOverflowReporting() {
        val ledger = ledger(pending = 1)
        ledger.offerPending("a", "p")
        assertThat(ledger.offerPending("b", "p")).isEqualTo(LedgerAdmission.REJECTED_FIRST)
        ledger.admitSession("x", "s")
        ledger.markLost("q")

        ledger.clear()

        assertThat(ledger.pollPending()).isNull()
        assertThat(ledger.sessionsSnapshot()).isEmpty()
        assertThat(ledger.isLost("q")).isFalse()
        ledger.offerPending("a", "p")
        assertThat(ledger.offerPending("b", "p")).isEqualTo(LedgerAdmission.REJECTED_FIRST)
    }

    @Test
    fun announcementFloodRetainsExactlyTheCapAndReportsExactlyOnce() {
        val ledger = MdnsDiscoveryLedger<String, String>()
        val outcomes = (1..10_000).map { ledger.offerPending("svc-$it", "p-$it") }

        assertThat(outcomes.count { it == LedgerAdmission.ACCEPTED }).isEqualTo(MAX_PENDING_MDNS_RESOLVES)
        assertThat(outcomes.count { it == LedgerAdmission.REJECTED_FIRST }).isEqualTo(1)
        assertThat(ledger.rejectedPendingCount).isEqualTo((10_000 - MAX_PENDING_MDNS_RESOLVES).toLong())
        assertThat(generateSequence { ledger.pollPending() }.count()).isEqualTo(MAX_PENDING_MDNS_RESOLVES)

        repeat(10_000) { ledger.markLost("gone-$it") }
        assertThat(ledger.isLost("gone-0")).isFalse()
        assertThat(ledger.isLost("gone-9999")).isTrue()
    }

    @Test
    fun concurrentAnnouncementsNeverExceedTheCap() {
        val ledger = MdnsDiscoveryLedger<String, String>(maxPending = 16)
        val accepted = AtomicInteger()
        val pool = Executors.newFixedThreadPool(8)
        val done = CountDownLatch(8)
        repeat(8) { worker ->
            pool.execute {
                repeat(500) { index ->
                    if (ledger.offerPending("w$worker-$index", "p") == LedgerAdmission.ACCEPTED) {
                        accepted.incrementAndGet()
                    }
                }
                done.countDown()
            }
        }
        assertThat(done.await(10, TimeUnit.SECONDS)).isTrue()
        pool.shutdownNow()

        assertThat(accepted.get()).isEqualTo(16)
        assertThat(generateSequence { ledger.pollPending() }.count()).isEqualTo(16)
    }
}
