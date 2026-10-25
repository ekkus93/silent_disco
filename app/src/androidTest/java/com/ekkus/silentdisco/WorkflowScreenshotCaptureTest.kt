package com.ekkus.silentdisco

import android.graphics.Bitmap
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
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.test.platform.app.InstrumentationRegistry
import com.ekkus.silentdisco.app.AppUiState
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
import com.ekkus.silentdisco.feature.host.HostDashboardScreen
import com.ekkus.silentdisco.feature.host.HostQueueManagementScreen
import com.ekkus.silentdisco.feature.listener.ClientWorkflowStage
import com.ekkus.silentdisco.feature.listener.ClientWorkflowStepper
import com.ekkus.silentdisco.feature.listener.SessionJoinScreen
import com.ekkus.silentdisco.feature.startup.StartupGateScreen
import com.ekkus.silentdisco.ui.theme.SilentDiscoTheme
import com.google.common.truth.Truth.assertThat
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
        EMPTY_QUEUE("host_queue_empty", 360, 640, 1f),
        PLAYING_QUEUE("host_queue_playing", 360, 640, 1f),
        QUEUE_LARGE_TEXT("host_queue_large_text", 320, 480, 2f),
        LIVE_PLAYING("host_live_playing", 360, 640, 1f),
        LIVE_PAUSED("host_live_paused", 360, 640, 1f),
        LIVE_LANDSCAPE("host_live_landscape", 640, 360, 1f),
        PENDING_APPROVAL("host_pending_approval", 360, 640, 1f),
        CLIENT_RECOVERY("client_recovery_large_text", 320, 480, 2f),
        STARTUP_RECOVERY("startup_recovery_large_text", 320, 480, 2f),
        STARTUP_FATAL("startup_fatal", 360, 640, 1f),
        CLIENT_FIND("client_stage_find", 360, 640, 1f),
        CLIENT_CONNECT("client_stage_connect", 360, 640, 1f),
        CLIENT_LISTEN("client_stage_listen", 360, 640, 1f),
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
            requestedAtMs = 0L,
        )
        val state = AppUiState(storageState = StorageInitializationState.READY)
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
                            Scene.LIVE_LANDSCAPE -> HostDashboardScreen(
                                uiState = state.copy(
                                    selectedRole = AppRole.HOST,
                                    hostState = HostLifecycleState.READY,
                                    hostQueue = if (scene == Scene.PENDING_APPROVAL) {
                                        emptyList()
                                    } else {
                                        queued
                                    },
                                    hostPlaybackState = if (scene == Scene.LIVE_PAUSED) {
                                        PlaybackState.PAUSED
                                    } else if (scene == Scene.PENDING_APPROVAL) {
                                        PlaybackState.STOPPED
                                    } else {
                                        PlaybackState.PLAYING
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
                            Scene.CLIENT_RECOVERY -> SessionJoinScreen(
                                uiState = state.copy(
                                    selectedRole = AppRole.LISTENER,
                                    selectedSession = session,
                                    discoveredSessions = listOf(session),
                                    listenerState = ListenerLifecycleState.ERROR,
                                    connectionProgress = ConnectionProgressState(
                                        currentState = ListenerLifecycleState.ERROR,
                                        discovered = true,
                                        requested = true,
                                        inviteCode = "1111",
                                    ),
                                    lastError = "Incorrect invite code",
                                ),
                                onInviteCodeChanged = {},
                                onJoin = {},
                                onCancel = {},
                                onRetry = {},
                                onReturnToSessions = {},
                                onOpenSettings = {},
                                onShareSupportReport = {},
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
        for (target in Scene.entries) {
            compose.runOnIdle { scene = target }
            compose.waitForIdle()
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
            manifest += "${target.basename}\t${bitmap.width}\t${bitmap.height}\t$digest"
        }
        File(directory, "manifest.tsv").writeText(manifest.joinToString("\n") + "\n")
        assertThat(directory.listFiles()?.size).isEqualTo(Scene.entries.size + 1)
    }
}
