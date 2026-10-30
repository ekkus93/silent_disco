package com.ekkus.silentdisco.app

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class PendingHostJoinRoutesTest {
    @Test
    fun concurrentSameClaimedIdNeverBorrowsAnotherRequestsPortsOrHandle() {
        val first = Any()
        val second = Any()
        val routes = PendingHostJoinRoutes<Any>()
        assertThat(routes.record("request-a", "same-id", 101u, 102u, first)).isTrue()
        assertThat(routes.record("request-b", "same-id", 201u, 202u, second)).isTrue()

        val original = routes.take("request-a", "same-id")
        assertThat(original?.sourceHandle).isSameInstanceAs(first)
        assertThat(original?.syncPort).isEqualTo(101u.toUShort())
        assertThat(original?.audioPort).isEqualTo(102u.toUShort())
        assertThat(routes.take("request-a", "same-id")).isNull()

        val reconnect = routes.take("request-b", "same-id")
        assertThat(reconnect?.sourceHandle).isSameInstanceAs(second)
        assertThat(reconnect?.syncPort).isEqualTo(201u.toUShort())
        assertThat(reconnect?.audioPort).isEqualTo(202u.toUShort())
    }

    @Test
    fun disconnectRetiresAllSameIdClaimsBeforeLateApprovalOrRejoin() {
        val routes = PendingHostJoinRoutes<Any>()
        val old = Any()
        val current = Any()
        routes.record("old-a", "listener", 11u, 12u, old)
        routes.record("old-b", "listener", 13u, 14u, old)
        routes.record("other", "unrelated", 21u, 22u, old)
        routes.removeDevice("listener")
        assertThat(routes.take("old-a", "listener")).isNull()
        assertThat(routes.take("old-b", "listener")).isNull()
        assertThat(routes.record("fresh", "listener", 31u, 32u, current)).isTrue()
        assertThat(routes.take("old-a", "listener")).isNull()
        assertThat(routes.take("fresh", "listener")?.sourceHandle).isSameInstanceAs(current)
        assertThat(routes.take("other", "unrelated")).isNotNull()
    }

    @Test
    fun wrongIdDuplicateRequestAndCapacityPressureFailClosed() {
        val routes = PendingHostJoinRoutes<Any>(capacity = 2)
        val handle = Any()
        assertThat(routes.record("one", "a", 1u, 2u, handle)).isTrue()
        assertThat(routes.record("one", "b", 3u, 4u, handle)).isFalse()
        assertThat(routes.record("two", "b", 3u, 4u, handle)).isTrue()
        assertThat(routes.record("three", "c", 5u, 6u, handle)).isFalse()
        assertThat(routes.size).isEqualTo(2)
        assertThat(routes.take("one", "spoofed-a")).isNull()
        assertThat(routes.take("one", "a")).isNull()
        assertThat(routes.take("two", "b")).isNotNull()
        assertThat(routes.size).isEqualTo(0)
        routes.record("four", "a", 7u, 8u, handle)
        routes.clear()
        assertThat(routes.take("four", "a")).isNull()
    }
}
