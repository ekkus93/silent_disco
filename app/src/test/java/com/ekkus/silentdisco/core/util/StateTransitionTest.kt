package com.ekkus.silentdisco.core.util

import com.google.common.truth.Truth.assertThat
import com.ekkus.silentdisco.app.ConnectionProgressState
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
}
