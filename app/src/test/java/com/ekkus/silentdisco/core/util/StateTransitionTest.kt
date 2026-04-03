package com.ekkus.silentdisco.core.util

import com.google.common.truth.Truth.assertThat
import com.ekkus.silentdisco.app.ConnectionProgressState
import com.ekkus.silentdisco.app.TuningField
import com.ekkus.silentdisco.app.TuningSettings
import com.ekkus.silentdisco.app.adjust
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import org.junit.Test

class StateTransitionTest {
    @Test
    fun `connection progress records playback path`() {
        val state = ConnectionProgressState(
            currentState = ListenerLifecycleState.PLAYING,
            discovered = true,
            requested = true,
            approved = true,
            connected = true,
            synced = true,
            playing = true,
        )

        assertThat(state.discovered).isTrue()
        assertThat(state.playing).isTrue()
        assertThat(state.currentState).isEqualTo(ListenerLifecycleState.PLAYING)
    }

    @Test
    fun `tuning settings keep hard resync above late threshold`() {
        val tuned = TuningSettings(
            latePacketThresholdMs = 110,
            hardResyncThresholdMs = 120,
        ).adjust(TuningField.LatePacketThresholdMs, 1)

        assertThat(tuned.latePacketThresholdMs >= 10L).isTrue()
        assertThat(tuned.hardResyncThresholdMs >= tuned.latePacketThresholdMs + 20L).isTrue()
    }
}
