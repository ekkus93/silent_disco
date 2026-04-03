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
