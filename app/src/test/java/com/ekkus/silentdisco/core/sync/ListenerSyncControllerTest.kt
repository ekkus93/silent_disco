package com.ekkus.silentdisco.core.sync

import com.ekkus.silentdisco.core.model.SyncQualityBadge
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
    fun `rejected high-RTT samples before the first accepted one do not poison the skew estimate`() {
        var nowMs = 0L
        val controller = ListenerSyncController(
            sessionId = SessionId("session"),
            config = SyncMaintenanceConfig(cadenceMs = 2_000),
            nowProvider = { nowMs },
        )

        // Host and listener clock epochs are unrelated (e.g. process start
        // vs. device boot), so a realistic offset is routinely hundreds of
        // millions of ms -- t2/t3 here deliberately mirror that. Each of
        // these responses has ~400ms RTT, above ClockSyncEstimator's
        // default 200ms acceptance threshold, so every one is rejected and
        // onResponse returns the still-default zero-offset state.
        repeat(4) { index ->
            nowMs = index * 2_000L
            val request = controller.newProbe(index.toLong())
            val state = controller.onResponse(
                response = controllerResponse(request, t2 = 500_000_000L, t3 = 500_000_500L),
                localReceiveTimeMs = nowMs + 900,
            )
            assertThat(state.confidence).isEqualTo(SyncQualityBadge.UNKNOWN)
        }

        // The first genuinely accepted sample: low RTT, realistic huge
        // cross-epoch offset.
        nowMs = 8_000L
        val request = controller.newProbe(99)
        val accepted = controller.onResponse(
            response = controllerResponse(request, t2 = 500_000_010L, t3 = 500_000_020L),
            localReceiveTimeMs = nowMs + 30,
        )

        assertThat(accepted.confidence).isNotEqualTo(SyncQualityBadge.UNKNOWN)
        // Regression guard: before this fix, the four rejected zero-offset
        // placeholders above were recorded in the skew-tracking history
        // alongside this first real, huge offset, producing a
        // near-vertical regression slope that -- multiplied by 1,000,000 to
        // express as ppm -- exploded into a physically impossible skew
        // (observed in the wild: -5e10 ppm). A single real sample can't
        // yet produce any skew estimate (needs >=3), so this must be 0.
        assertThat(accepted.skewPpm).isEqualTo(0.0)
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
