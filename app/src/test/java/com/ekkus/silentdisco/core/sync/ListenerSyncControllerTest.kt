package com.ekkus.silentdisco.core.sync

import com.ekkus.silentdisco.core.protocol.SessionId
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class ListenerSyncControllerTest {
    @Test
    fun `controller estimates skew and requests resync after drift threshold`() {
        val controller = ListenerSyncController(
            sessionId = SessionId("session"),
            config = SyncMaintenanceConfig(cadenceMs = 1_000, driftThresholdMs = 5.0),
            nowProvider = { 1_000L },
        )

        val initial = controller.newProbe(1)
        val stateOne = controller.onResponse(
            response = controllerResponse(initial, t2 = 10, t3 = 12),
            localReceiveTimeMs = 14,
        )
        val second = controller.newProbe(2)
        val stateTwo = controller.onResponse(
            response = controllerResponse(second, t2 = 1010, t3 = 1012),
            localReceiveTimeMs = 1018,
        )

        assertThat(stateTwo.skewPpm).isFinite()
        assertThat(controller.shouldResync(nowMs = 2_100, state = stateOne)).isTrue()
    }

    @Test
    fun `controller respects configurable sample history size`() {
        var nowMs = 0L
        val controller = ListenerSyncController(
            sessionId = SessionId("session"),
            estimator = ClockSyncEstimator(maxSamples = 4),
            config = SyncMaintenanceConfig(
                cadenceMs = 1_000,
                driftThresholdMs = 50.0,
                sampleHistorySize = 4,
            ),
            nowProvider = { nowMs },
        )

        repeat(6) { index ->
            nowMs = index * 100L
            val request = controller.newProbe(index.toLong())
            controller.onResponse(
                response = controllerResponse(request, t2 = nowMs + 5, t3 = nowMs + 6),
                localReceiveTimeMs = nowMs + 7,
            )
        }

        val request = controller.newProbe(99)
        val state = controller.onResponse(
            response = controllerResponse(request, t2 = nowMs + 8, t3 = nowMs + 9),
            localReceiveTimeMs = nowMs + 10,
        )

        assertThat(state.confidence).isNotNull()
        assertThat(state.rttMs).isAtMost(50.0)
    }

    private fun controllerResponse(
        request: com.ekkus.silentdisco.core.protocol.SyncRequestPacket,
        t2: Long,
        t3: Long,
    ) = com.ekkus.silentdisco.core.protocol.SyncResponsePacket(
        version = request.version,
        sessionId = request.sessionId,
        correlationId = request.correlationId,
        t1ListenerSendElapsedMs = request.t1ListenerSendElapsedMs,
        t2HostReceiveElapsedMs = t2,
        t3HostSendElapsedMs = t3,
    )
}
