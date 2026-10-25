package com.ekkus.silentdisco

import android.net.Uri
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.Button
import androidx.compose.material3.Text
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import com.ekkus.silentdisco.app.AudioPickerDestination
import com.ekkus.silentdisco.app.dispatchAudioPickerResult
import com.ekkus.silentdisco.app.rememberAudioPickerDestination
import com.google.common.truth.Truth.assertThat
import org.junit.Rule
import org.junit.Test

/** The real picker callback uses the same saved route and one-shot dispatcher. */
class AudioPickerDestinationRestorationTest {
    @get:Rule val compose = createComposeRule()

    @Test
    fun livePickerResultAfterRecreationDoesNotReplaceSetupAndCancellationResetsRoute() {
        val restore = StateRestorationTester(compose)
        val setupBatches = mutableListOf<List<Uri>>()
        val liveBatches = mutableListOf<List<Uri>>()
        val song = Uri.parse("content://test/song.wav")
        restore.setContent {
            val destination = rememberAudioPickerDestination()
            Column {
                Text(destination.value.name, Modifier.testTag("picker-destination"))
                Button(
                    onClick = { destination.value = AudioPickerDestination.LIVE_QUEUE },
                    modifier = Modifier.testTag("pick-live"),
                ) { Text("Add live") }
                Button(
                    onClick = {
                        dispatchAudioPickerResult(
                            destination,
                            listOf(song),
                            onSetup = setupBatches::add,
                            onLiveQueue = liveBatches::add,
                        )
                    },
                    modifier = Modifier.testTag("deliver-result"),
                ) { Text("Deliver") }
                Button(
                    onClick = {
                        dispatchAudioPickerResult(
                            destination,
                            emptyList(),
                            onSetup = setupBatches::add,
                            onLiveQueue = liveBatches::add,
                        )
                    },
                    modifier = Modifier.testTag("cancel-picker"),
                ) { Text("Cancel") }
            }
        }

        compose.onNodeWithTag("pick-live").performClick()
        compose.onNodeWithTag("picker-destination").assertTextEquals("LIVE_QUEUE")
        restore.emulateSavedInstanceStateRestore()
        compose.onNodeWithTag("picker-destination").assertTextEquals("LIVE_QUEUE")
        compose.onNodeWithTag("deliver-result").performClick()
        compose.onNodeWithTag("picker-destination").assertTextEquals("SETUP")
        compose.runOnIdle {
            assertThat(liveBatches).containsExactly(listOf(song))
            assertThat(setupBatches).isEmpty()
        }

        // A later setup import must not inherit the previous live route.
        compose.onNodeWithTag("deliver-result").performClick()
        compose.onNodeWithTag("pick-live").performClick()
        restore.emulateSavedInstanceStateRestore()
        compose.onNodeWithTag("cancel-picker").performClick()
        compose.onNodeWithTag("picker-destination").assertTextEquals("SETUP")
        compose.onNodeWithTag("deliver-result").performClick()
        compose.runOnIdle {
            assertThat(liveBatches).containsExactly(listOf(song))
            assertThat(setupBatches).containsExactly(listOf(song), listOf(song))
        }
    }
}
