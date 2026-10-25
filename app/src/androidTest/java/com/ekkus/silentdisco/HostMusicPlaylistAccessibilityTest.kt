package com.ekkus.silentdisco

import android.net.Uri
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.assertContentDescriptionEquals
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.HostFormState
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.feature.host.HostMusicSetupScreen
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import com.google.common.truth.Truth.assertThat
import org.junit.Rule
import org.junit.Test

class HostMusicPlaylistAccessibilityTest {
    @get:Rule val compose = createComposeRule()

    @Test
    fun compactLargeTextPlaylistActionsStayNamedOrderedAndSafeAcrossRestoration() {
        val names = listOf("Opening track", "Next track", "Final track")
        val selected = names.mapIndexed { index, name ->
            SelectedAudioFile(
                uri = Uri.parse("content://fixture/song-$index.wav"),
                displayName = name,
                mimeType = "audio/wav",
                sizeBytes = 1_024,
            )
        }
        val state = AppUiState(
            hostForm = HostFormState(
                sessionName = "Accessibility check",
                selectedAudio = selected.first(),
                additionalAudio = selected.drop(1),
            ),
        )
        val moves = mutableListOf<Pair<Int, Int>>()
        val removals = mutableListOf<Int>()
        val restore = StateRestorationTester(compose)
        restore.setContent {
            CompositionLocalProvider(LocalDensity provides Density(1f, 1.5f)) {
                SilentDiscoTheme {
                    Box(Modifier.size(320.dp, 480.dp)) {
                        HostMusicSetupScreen(
                            uiState = state,
                            onBack = {},
                            onSessionNameChanged = {},
                            onChooseAudio = {},
                            onRemoveAudio = { removals += it },
                            onMoveAudio = { from, to -> moves += from to to },
                            onNext = {},
                        )
                    }
                }
            }
        }

        compose.onNodeWithTag("host-music-move-up-0").assertIsNotEnabled()
        compose.onNodeWithTag("host-music-move-down-2").assertIsNotEnabled()
        compose.onNodeWithTag("host-music-move-up-1")
            .assertContentDescriptionEquals("Move Next track up, song 2 of 3")
        compose.onNodeWithTag("host-music-move-down-1")
            .assertContentDescriptionEquals("Move Next track down, song 2 of 3")
        compose.onNodeWithTag("host-music-remove-1")
            .assertContentDescriptionEquals("Remove Next track, song 2 of 3")
        compose.onNodeWithTag("host-music-move-up-1").performScrollTo().performClick()
        compose.onNodeWithTag("host-music-remove-2").performScrollTo().performClick()
        compose.runOnIdle {
            assertThat(moves).containsExactly(1 to 0)
            assertThat(removals).containsExactly(2)
        }

        restore.emulateSavedInstanceStateRestore()
        compose.onNodeWithTag("host-music-move-up-0").assertIsNotEnabled()
        compose.onNodeWithTag("host-music-move-down-2").assertIsNotEnabled()
        compose.onNodeWithTag("host-music-remove-0")
            .assertContentDescriptionEquals("Remove Opening track, song 1 of 3")
        compose.runOnIdle {
            assertThat(moves).containsExactly(1 to 0)
            assertThat(removals).containsExactly(2)
        }
    }
}
