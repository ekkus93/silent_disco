package com.ekkus.silentdisco.feature.diagnostics

import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.ui.components.StatusTone
import com.google.common.truth.Truth.assertThat
import org.junit.Test

class ConnectionHelpPresentationTest {
    @Test
    fun healthyListenerIndicatorsArePositive() {
        val state = AppUiState(
            selectedRole = AppRole.LISTENER,
            listenerState = ListenerLifecycleState.PLAYING,
            listenerPlaybackState = PlaybackState.PLAYING,
        )

        val indicators = connectionHelpIndicators(state, hostContext = false)

        assertThat(indicators.map { it.value }).containsExactly("Good", "Good", "Playing").inOrder()
        assertThat(indicators.map { it.tone }).containsExactly(
            StatusTone.POSITIVE,
            StatusTone.POSITIVE,
            StatusTone.POSITIVE,
        ).inOrder()
    }

    @Test
    fun reconnectingListenerCannotAppearHealthy() {
        val state = AppUiState(
            selectedRole = AppRole.LISTENER,
            listenerState = ListenerLifecycleState.RECONNECTING,
            listenerPlaybackState = PlaybackState.PLAYING,
        )

        val indicators = connectionHelpIndicators(state, hostContext = false)

        assertThat(indicators.first().value).isEqualTo("Recovering")
        assertThat(indicators.first().tone).isEqualTo(StatusTone.IN_PROGRESS)
    }

    @Test
    fun hostErrorsAndDesyncedListenersAreVisible() {
        val baseline = AppUiState(
            selectedRole = AppRole.HOST,
            hostState = HostLifecycleState.ERROR,
            hostPlaybackState = PlaybackState.ERROR,
        )
        val state = baseline.copy(
            hostDiagnostics = baseline.hostDiagnostics.copy(desyncedListenerCount = 2),
        )

        val indicators = connectionHelpIndicators(state, hostContext = true)

        assertThat(indicators[0].tone).isEqualTo(StatusTone.CRITICAL)
        assertThat(indicators[1].value).isEqualTo("2 need attention")
        assertThat(indicators[1].tone).isEqualTo(StatusTone.ATTENTION)
        assertThat(indicators[2].tone).isEqualTo(StatusTone.CRITICAL)
    }
}
