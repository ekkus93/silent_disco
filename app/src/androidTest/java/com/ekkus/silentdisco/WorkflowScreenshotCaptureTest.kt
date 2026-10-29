package com.ekkus.silentdisco

import android.graphics.Bitmap
import android.net.Uri
import android.os.SystemClock
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.test.platform.app.InstrumentationRegistry
import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.HostFormState
import com.ekkus.silentdisco.app.ConnectionProgressState
import com.ekkus.silentdisco.app.HostQueueItem
import com.ekkus.silentdisco.app.StorageInitializationState
import com.ekkus.silentdisco.core.model.AppRole
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.JoinRequest
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.PlaybackState
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.SelectedAudioFile
import com.ekkus.silentdisco.feature.host.HostAccessSetupScreen
import com.ekkus.silentdisco.feature.host.HostDashboardScreen
import com.ekkus.silentdisco.feature.host.HostMusicSetupScreen
import com.ekkus.silentdisco.feature.host.HostReadyScreen
import com.ekkus.silentdisco.feature.host.HostQueueManagementScreen
import com.ekkus.silentdisco.feature.listener.ManualEndpointScreen
import com.ekkus.silentdisco.feature.listener.ClientWorkflowStage
import com.ekkus.silentdisco.feature.listener.ClientWorkflowStepper
import com.ekkus.silentdisco.feature.listener.SessionJoinScreen
import com.ekkus.silentdisco.feature.startup.StartupGateScreen
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import java.io.ByteArrayOutputStream
import java.io.File
import java.security.MessageDigest
import org.junit.Rule
import org.junit.Test

/**
 * Pixel-review input, NOT approved golden baselines. All content is synthetic.
 * CI pulls the PNGs and SHA-256 manifest from app-private external files.
 * Fixed logical viewports make the API 29 capture dimensions reproducible.
 */
class WorkflowScreenshotCaptureTest {
    @get:Rule val compose = createComposeRule()

    private enum class Scene(val basename: String, val width: Int, val height: Int, val fontScale: Float) {
        MUSIC_EMPTY("host_music_empty", 360, 640, 1f),
        MUSIC_FILLED("host_music_filled", 360, 640, 1f),
        ACCESS_CODE("host_access_code", 360, 640, 1f),
        READY("host_ready", 360, 640, 1f),
        READY_RECOVERY("host_ready_permission_recovery", 320, 480, 1.5f),
        EMPTY_QUEUE("host_queue_empty", 360, 640, 1f),
        PLAYING_QUEUE("host_queue_playing", 360, 640, 1f),
        QUEUE_LARGE_TEXT("host_queue_large_text", 320, 480, 2f),
        LIVE_PLAYING("host_live_playing", 360, 640, 1f),
        LIVE_PAUSED("host_live_paused", 360, 640, 1f),
        LIVE_BUFFERING("host_live_buffering", 360, 640, 1f),
        LIVE_ERROR("host_live_playback_error", 360, 640, 1f),
        LIVE_ENDED("host_live_queue_ended", 360, 640, 1f),
        LIVE_LANDSCAPE("host_live_landscape", 640, 360, 1f),
        PENDING_APPROVAL("host_pending_approval", 360, 640, 1f),
        CLIENT_INVITE_CODE("client_invite_code", 360, 640, 1f),
        CLIENT_INVITE_CODE_LARGE("client_invite_code_large_text", 320, 480, 2f),
        CLIENT_RECOVERY("client_recovery_large_text", 320, 480, 2f),
        STARTUP_RECOVERY("startup_recovery_large_text", 320, 480, 2f),
        STARTUP_FATAL("startup_fatal", 360, 640, 1f),
        CLIENT_FIND("client_stage_find", 360, 640, 1f),
        CLIENT_CONNECT("client_stage_connect", 360, 640, 1f),
        CLIENT_LISTEN("client_stage_listen", 360, 640, 1f),
        MANUAL_COMPACT("client_manual_compact", 320, 400, 1.8f),
        MANUAL_LANDSCAPE("client_manual_landscape", 640, 360, 1.4f),
    }

    @Test
    fun exportsSyntheticReviewedStatesAsPngsAndSha256Manifest() {
        var scene by mutableStateOf(Scene.EMPTY_QUEUE)
        val session = SessionInfo(
            id = "visual-fixture-session",
            name = "Warehouse Silent Disco",
            hostDeviceName = "Host phone",
            approvalMode = ApprovalMode.INVITE_CODE,
            inviteCodeRequired = true,
        )
        val pending = JoinRequest(
            requestId = "visual-fixture-request",
            sessionId = session.id,
            listenerId = "visual-fixture-listener",
            listenerName = "Sample listener",
            inviteCode = null,
            // The approval card uses an OS-clock ticker. Pin this synthetic
            // request safely in the future so two screenshot passes cannot
            // change its elapsed-time label as the emulator suite runs.
            requestedAtMs = SystemClock.elapsedRealtime() + 3_600_000L,
        )
        val state = AppUiState(storageState = StorageInitializationState.READY)
        val staged = listOf("Opening track", "Next track", "Final track").mapIndexed { i, name ->
            SelectedAudioFile(
                uri = Uri.parse("content://fixture/song-$i.wav"),
                displayName = name,
                mimeType = "audio/wav",
                sizeBytes = 1_024,
            )
        }
        val setup = state.copy(
            hostForm = HostFormState(
                sessionName = "Warehouse Silent Disco",
                selectedAudio = staged.first(),
                additionalAudio = staged.drop(1),
                approvalMode = ApprovalMode.INVITE_CODE,
                inviteCode = "4826",
            ),
        )
        val queued = listOf(
            HostQueueItem("current", "source-1", "Opening track", null, null),
            HostQueueItem("second", "source-2", "Next track", null, null),
            HostQueueItem("third", "source-3", "Final track", null, null),
        )
        compose.setContent {
            CompositionLocalProvider(
                LocalDensity provides Density(density = 1f, fontScale = scene.fontScale),
            ) {
                SilentDiscoTheme {
                    Box(
                        Modifier.size(scene.width.dp, scene.height.dp)
                            .testTag("visual-review-canvas"),
                    ) {
                        when (scene) {
                            Scene.MUSIC_EMPTY, Scene.MUSIC_FILLED -> HostMusicSetupScreen(
                                uiState = if (scene == Scene.MUSIC_EMPTY) state else setup,
                                onBack = {},
                                onSessionNameChanged = {},
                                onChooseAudio = {},
                                onRemoveAudio = {},
                                onMoveAudio = { _, _ -> },
                                onNext = {},
                            )
                            Scene.ACCESS_CODE -> HostAccessSetupScreen(
                                uiState = setup,
                                onBack = {},
                                onApprovalModeChanged = {},
                                onInviteCodeChanged = {},
                                onGenerateCode = {},
                                onNext = {},
                            )
                            Scene.READY, Scene.READY_RECOVERY -> HostReadyScreen(
                                uiState = if (scene == Scene.READY_RECOVERY) {
                                    setup.copy(
                                        hostState = HostLifecycleState.ERROR,
                                        lastError = "Nearby-device permission missing",
                                    )
                                } else {
                                    setup
                                },
                                onBack = {},
                                onStartSession = {},
                                onOpenSettings = {},
                                onShareSupportReport = {},
                            )
                            Scene.EMPTY_QUEUE, Scene.PLAYING_QUEUE, Scene.QUEUE_LARGE_TEXT ->
                                HostQueueManagementScreen(
                                uiState = state.copy(
                                    hostQueue = if (scene == Scene.EMPTY_QUEUE) emptyList() else queued,
                                    hostPlaybackState = if (scene == Scene.EMPTY_QUEUE) {
                                        PlaybackState.STOPPED
                                    } else {
                                        PlaybackState.PLAYING
                                    },
                                ),
                                onBack = {},
                                onAddSongs = {},
                                onReorderQueue = {},
                                onRemoveQueueEntry = {},
                            )
                            Scene.PENDING_APPROVAL, Scene.LIVE_PLAYING, Scene.LIVE_PAUSED,
                            Scene.LIVE_LANDSCAPE, Scene.LIVE_BUFFERING, Scene.LIVE_ERROR,
                            Scene.LIVE_ENDED -> HostDashboardScreen(
                                uiState = state.copy(
                                    selectedRole = AppRole.HOST,
                                    hostState = HostLifecycleState.READY,
                                    hostQueue = if (scene == Scene.PENDING_APPROVAL ||
                                        scene == Scene.LIVE_ENDED) {
                                        emptyList()
                                    } else {
                                        queued
                                    },
                                    hostPlaybackState = when (scene) {
                                        Scene.LIVE_PAUSED -> PlaybackState.PAUSED
                                        Scene.LIVE_BUFFERING -> PlaybackState.BUFFERING
                                        Scene.LIVE_ERROR -> PlaybackState.ERROR
                                        Scene.PENDING_APPROVAL, Scene.LIVE_ENDED -> PlaybackState.STOPPED
                                        else -> PlaybackState.PLAYING
                                    },
                                    lastError = if (scene == Scene.LIVE_ERROR) {
                                        "Synthetic decoder failure: retry this song"
                                    } else {
                                        null
                                    },
                                    pendingJoinRequests = if (scene == Scene.PENDING_APPROVAL) {
                                        listOf(pending)
                                    } else {
                                        emptyList()
                                    },
                                ),
                                onBackRequest = {},
                                onInvite = {},
                                onApproval = { _, _ -> },
                                onRemoveListener = {},
                                onPlayPause = {},
                                onStop = {},
                                onEndSessionRequest = {},
                                onOpenConnectionHelp = {},
                                onAddDemoJoinRequest = {},
                            )
                            Scene.CLIENT_INVITE_CODE, Scene.CLIENT_INVITE_CODE_LARGE,
                            Scene.CLIENT_RECOVERY -> SessionJoinScreen(
                                uiState = state.copy(
                                    selectedRole = AppRole.LISTENER,
                                    selectedSession = session,
                                    discoveredSessions = listOf(session),
                                    listenerState = if (scene == Scene.CLIENT_RECOVERY) {
                                        ListenerLifecycleState.ERROR
                                    } else {
                                        ListenerLifecycleState.SESSION_SELECTED
                                    },
                                    connectionProgress = ConnectionProgressState(
                                        currentState = if (scene == Scene.CLIENT_RECOVERY) {
                                            ListenerLifecycleState.ERROR
                                        } else {
                                            ListenerLifecycleState.SESSION_SELECTED
                                        },
                                        discovered = true,
                                        requested = scene == Scene.CLIENT_RECOVERY,
                                        inviteCode = if (scene == Scene.CLIENT_RECOVERY) "1111" else "4826",
                                    ),
                                    lastError = if (scene == Scene.CLIENT_RECOVERY) {
                                        "Incorrect invite code"
                                    } else {
                                        null
                                    },
                                ),
                                onInviteCodeChanged = {},
                                onJoin = {},
                                onCancel = {},
                                onRetry = {},
                                onReturnToSessions = {},
                                onOpenSettings = {},
                                onShareSupportReport = {},
                            )
                            Scene.MANUAL_COMPACT, Scene.MANUAL_LANDSCAPE -> ManualEndpointScreen(
                                uiState = state,
                                onBack = {},
                                onInputChanged = {},
                                onInviteCodeChanged = {},
                                onConnect = {},
                                onCancel = {},
                            )
                            Scene.STARTUP_RECOVERY, Scene.STARTUP_FATAL -> StartupGateScreen(
                                uiState = AppUiState(
                                    storageState = if (scene == Scene.STARTUP_RECOVERY) {
                                        StorageInitializationState.RECOVERABLE_FAILURE
                                    } else {
                                        StorageInitializationState.FATAL_FAILURE
                                    },
                                    storageError = "Synthetic startup failure",
                                ),
                                onRetry = {},
                                onShareSupportReport = {},
                            )
                            Scene.CLIENT_FIND, Scene.CLIENT_CONNECT, Scene.CLIENT_LISTEN ->
                                ClientWorkflowStepper(
                                    stage = when (scene) {
                                        Scene.CLIENT_FIND -> ClientWorkflowStage.FIND
                                        Scene.CLIENT_CONNECT -> ClientWorkflowStage.CONNECT
                                        else -> ClientWorkflowStage.LISTEN
                                    },
                                )
                        }
                    }
                }
            }
        }

        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val directory = requireNotNull(context.getExternalFilesDir("workflow-screenshots")) {
            "Synthetic screenshots need an app-private external destination"
        }
        assertThat(directory.isDirectory || directory.mkdirs()).isTrue()
        directory.listFiles()?.forEach { assertThat(it.delete()).isTrue() }
        val manifest = mutableListOf("scene\twidth_px\theight_px\tsha256")
        val hashes = mutableSetOf<String>()
        val originalDigests = mutableMapOf<Scene, String>()
        for (target in Scene.entries) {
            compose.runOnIdle { scene = target }
            compose.waitForIdle()
            if (target == Scene.MANUAL_COMPACT || target == Scene.MANUAL_LANDSCAPE) {
                compose.onNodeWithTag("manual-endpoint-connect").assertIsDisplayed()
                compose.onNodeWithTag("manual-endpoint-cancel").assertIsDisplayed()
            }
            val bitmap = compose.onNodeWithTag("visual-review-canvas")
                .captureToImage().asAndroidBitmap()
            assertThat(bitmap.width).isEqualTo(target.width)
            assertThat(bitmap.height).isEqualTo(target.height)
            val file = File(directory, "${target.basename}.png")
            file.outputStream().use { stream ->
                assertThat(bitmap.compress(Bitmap.CompressFormat.PNG, 100, stream)).isTrue()
            }
            val bytes = file.readBytes()
            assertThat(bytes.size).isGreaterThan(256)
            val digest = MessageDigest.getInstance("SHA-256").digest(bytes)
                .joinToString("") { byte -> "%02x".format(byte.toInt() and 0xff) }
            assertThat(hashes.add(digest)).isTrue()
            originalDigests[target] = digest
            manifest += "${target.basename}\t${bitmap.width}\t${bitmap.height}\t$digest"
        }
        File(directory, "manifest.tsv").writeText(manifest.joinToString("\n") + "\n")
        assertThat(directory.listFiles()?.size).isEqualTo(Scene.entries.size + 1)

        // A single screenshot pass can hide retained Compose state or a
        // transition that has not settled. Revisit every state in reverse
        // order, without resetting the composition, and require identical
        // rendered PNG bytes. The approved pixel-golden review is separate.
        for (target in Scene.entries.reversed()) {
            compose.runOnIdle { scene = target }
            compose.waitForIdle()
            val bitmap = compose.onNodeWithTag("visual-review-canvas")
                .captureToImage().asAndroidBitmap()
            assertThat(bitmap.width).isEqualTo(target.width)
            assertThat(bitmap.height).isEqualTo(target.height)
            val encoded = ByteArrayOutputStream()
            assertThat(bitmap.compress(Bitmap.CompressFormat.PNG, 100, encoded)).isTrue()
            val repeatDigest = MessageDigest.getInstance("SHA-256")
                .digest(encoded.toByteArray())
                .joinToString("") { byte -> "%02x".format(byte.toInt() and 0xff) }
            assertWithMessage("repeat rendering of ${target.basename}")
                .that(repeatDigest)
                .isEqualTo(originalDigests.getValue(target))
        }
    }
}
