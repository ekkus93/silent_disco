package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import org.junit.Assert.assertEquals
import org.junit.Test

class TransportFailureStateTest {

    @Test
    fun classifyRole_hostFailureWhenHostingActive() {
        val hostingStates = listOf(
            HostLifecycleState.CREATING_SESSION,
            HostLifecycleState.WAITING_FOR_LISTENERS,
            HostLifecycleState.READY,
            HostLifecycleState.STREAMING,
        )
        hostingStates.forEach { hostState ->
            assertEquals(
                "hostState=$hostState should be HOST_FAILURE",
                TransportSnapshotRole.HOST_FAILURE,
                classifyTransportSnapshotRole(hostState, ListenerLifecycleState.IDLE),
            )
        }
    }

    @Test
    fun classifyRole_listenerFailureWhenListenerActive() {
        val activeListenerStates = listOf(
            ListenerLifecycleState.SCANNING,
            ListenerLifecycleState.SESSION_SELECTED,
            ListenerLifecycleState.JOIN_REQUESTED,
            ListenerLifecycleState.CONNECTING,
            ListenerLifecycleState.AWAITING_APPROVAL,
            ListenerLifecycleState.APPROVED,
            ListenerLifecycleState.SYNCING_CLOCK,
            ListenerLifecycleState.BUFFERING,
            ListenerLifecycleState.PLAYING,
            ListenerLifecycleState.RECONNECTING,
            ListenerLifecycleState.DESYNCED,
        )
        activeListenerStates.forEach { listenerState ->
            assertEquals(
                "listenerState=$listenerState should be LISTENER_FAILURE",
                TransportSnapshotRole.LISTENER_FAILURE,
                classifyTransportSnapshotRole(HostLifecycleState.IDLE, listenerState),
            )
        }
    }

    @Test
    fun classifyRole_ignoreWhenNeitherHostNorListenerActive() {
        assertEquals(
            TransportSnapshotRole.IGNORE,
            classifyTransportSnapshotRole(HostLifecycleState.IDLE, ListenerLifecycleState.IDLE),
        )
        assertEquals(
            TransportSnapshotRole.IGNORE,
            classifyTransportSnapshotRole(HostLifecycleState.IDLE, ListenerLifecycleState.DISCONNECTED),
        )
        assertEquals(
            TransportSnapshotRole.IGNORE,
            classifyTransportSnapshotRole(HostLifecycleState.IDLE, ListenerLifecycleState.ERROR),
        )
    }

    @Test
    fun classifyRole_hostTakesPriorityWhenBothActive() {
        assertEquals(
            TransportSnapshotRole.HOST_FAILURE,
            classifyTransportSnapshotRole(HostLifecycleState.STREAMING, ListenerLifecycleState.PLAYING),
        )
    }
}
