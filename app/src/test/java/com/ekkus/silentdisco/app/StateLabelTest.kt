package com.ekkus.silentdisco.app

import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.JoinApprovalState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.model.TransportConnectionState
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class StateLabelTest {

    @Test
    fun `hostStateLabel covers all HostLifecycleState values without raw enum names`() {
        HostLifecycleState.entries.forEach { state ->
            val label = state.label()
            assertThat(label).isNotEmpty()
            assertThat(label).doesNotContain("_")
            assertThat(label).isNotEqualTo(state.name)
        }
    }

    @Test
    fun `hostStateLabel returns correct human-readable strings`() {
        assertThat(HostLifecycleState.IDLE.label()).isEqualTo("Idle")
        assertThat(HostLifecycleState.CREATING_SESSION.label()).isEqualTo("Creating session…")
        assertThat(HostLifecycleState.ADVERTISING.label()).isEqualTo("Advertising…")
        assertThat(HostLifecycleState.WAITING_FOR_LISTENERS.label()).isEqualTo("Waiting for listeners")
        assertThat(HostLifecycleState.READY.label()).isEqualTo("Ready")
        assertThat(HostLifecycleState.STREAMING.label()).isEqualTo("Streaming")
        assertThat(HostLifecycleState.PAUSED.label()).isEqualTo("Paused")
        assertThat(HostLifecycleState.ENDING_SESSION.label()).isEqualTo("Ending session…")
        assertThat(HostLifecycleState.ERROR.label()).isEqualTo("Error")
    }

    @Test
    fun `playbackStateLabel covers all PlaybackState values without raw enum names`() {
        PlaybackState.entries.forEach { state ->
            val label = state.label()
            assertThat(label).isNotEmpty()
            assertThat(label).doesNotContain("_")
            assertThat(label).isNotEqualTo(state.name)
        }
    }

    @Test
    fun `playbackStateLabel returns correct human-readable strings`() {
        assertThat(PlaybackState.STOPPED.label()).isEqualTo("Stopped")
        assertThat(PlaybackState.BUFFERING.label()).isEqualTo("Buffering…")
        assertThat(PlaybackState.READY.label()).isEqualTo("Ready")
        assertThat(PlaybackState.PLAYING.label()).isEqualTo("Playing")
        assertThat(PlaybackState.PAUSED.label()).isEqualTo("Paused")
        assertThat(PlaybackState.UNDERRUN.label()).isEqualTo("Underrun")
        assertThat(PlaybackState.ERROR.label()).isEqualTo("Error")
    }

    @Test
    fun `approvalModeLabel covers all ApprovalMode values without raw enum names`() {
        ApprovalMode.entries.forEach { mode ->
            val label = mode.label()
            assertThat(label).isNotEmpty()
            assertThat(label).doesNotContain("_")
            assertThat(label).isNotEqualTo(mode.name)
        }
    }

    @Test
    fun `approvalModeLabel returns correct human-readable strings`() {
        assertThat(ApprovalMode.MANUAL.label()).isEqualTo("Manual approval")
        assertThat(ApprovalMode.INVITE_CODE.label()).isEqualTo("Invite code")
        assertThat(ApprovalMode.TRUSTED_DEVICES_PLACEHOLDER.label()).isEqualTo("Trusted devices")
    }

    @Test
    fun `joinApprovalStateLabel covers all JoinApprovalState values without raw enum names`() {
        JoinApprovalState.entries.forEach { state ->
            val label = state.label()
            assertThat(label).isNotEmpty()
            assertThat(label).doesNotContain("_")
            assertThat(label).isNotEqualTo(state.name)
        }
    }

    @Test
    fun `joinApprovalStateLabel returns correct human-readable strings`() {
        assertThat(JoinApprovalState.DISCOVERED.label()).isEqualTo("Discovered")
        assertThat(JoinApprovalState.REQUESTED.label()).isEqualTo("Pending")
        assertThat(JoinApprovalState.APPROVED.label()).isEqualTo("Approved")
        assertThat(JoinApprovalState.REJECTED.label()).isEqualTo("Rejected")
        assertThat(JoinApprovalState.CANCELLED.label()).isEqualTo("Cancelled")
    }

    @Test
    fun `transportConnectionStateLabel covers all TransportConnectionState values without raw enum names`() {
        TransportConnectionState.entries.forEach { state ->
            val label = state.label()
            assertThat(label).isNotEmpty()
            assertThat(label).doesNotContain("_")
            assertThat(label).isNotEqualTo(state.name)
        }
    }

    @Test
    fun `transportConnectionStateLabel returns correct human-readable strings`() {
        assertThat(TransportConnectionState.IDLE.label()).isEqualTo("Idle")
        assertThat(TransportConnectionState.DISCOVERING.label()).isEqualTo("Discovering…")
        assertThat(TransportConnectionState.ADVERTISING.label()).isEqualTo("Advertising…")
        assertThat(TransportConnectionState.CONNECTING.label()).isEqualTo("Connecting…")
        assertThat(TransportConnectionState.CONNECTED.label()).isEqualTo("Connected")
        assertThat(TransportConnectionState.RETRYING.label()).isEqualTo("Retrying…")
        assertThat(TransportConnectionState.DISCONNECTED.label()).isEqualTo("Disconnected")
        assertThat(TransportConnectionState.FAILED.label()).isEqualTo("Failed")
    }

    @Test
    fun `syncQualityBadgeLabel covers all SyncQualityBadge values without raw enum names`() {
        SyncQualityBadge.entries.forEach { badge ->
            val label = badge.label()
            assertThat(label).isNotEmpty()
            assertThat(label).doesNotContain("_")
            assertThat(label).isNotEqualTo(badge.name)
        }
    }

    @Test
    fun `syncQualityBadgeLabel returns correct human-readable strings`() {
        assertThat(SyncQualityBadge.UNKNOWN.label()).isEqualTo("Unknown")
        assertThat(SyncQualityBadge.POOR.label()).isEqualTo("Poor")
        assertThat(SyncQualityBadge.FAIR.label()).isEqualTo("Fair")
        assertThat(SyncQualityBadge.GOOD.label()).isEqualTo("Good")
        assertThat(SyncQualityBadge.EXCELLENT.label()).isEqualTo("Excellent")
    }
}
