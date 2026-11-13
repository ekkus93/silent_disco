package com.ekkus.silentdisco.app.controller

import com.ekkus.silentdisco.core.audio.OboeBridge
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.rust.RustCoreBridge
import com.ekkus.silentdisco.core.rust.RustSyncEstimator
import com.ekkus.silentdisco.core.rust.RustSyncObservation
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackConfig
import com.ekkus.silentdisco.core.uniffi.FfiListenerPlaybackHandle
import com.ekkus.silentdisco.core.uniffi.FfiListenerSyncHandover
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.CoroutineScope

private const val LISTENER_REBUFFER_TARGET_MS: ULong = 400uL
private const val LISTENER_RING_CAPACITY_FRAMES: UInt = 48_000u
private const val LISTENER_RING_TARGET_FILL_FRAMES: UInt = 19_200u
private const val LISTENER_RING_WRITE_LEAD_MS: ULong = 400uL
private const val LISTENER_MAX_RING_PREFILL_MS: ULong = 800uL

/** `nativeOboeOpen` success status. */
private const val LISTENER_OBOE_STATUS_OK = 0

/** A stream's clock lock, tagged with the session it belongs to. */
internal class SessionSyncHandover(val sessionId: SessionId, val handover: FfiListenerSyncHandover)

/**
 * The carried value to open [session]'s next stream from: only a handover
 * from that same session. Another session's host has an unrelated clock, so
 * its offset would place packets on the wrong timeline.
 */
internal fun <T> handoverForSession(storedSession: SessionId?, stored: T?, session: SessionId): T? =
    stored?.takeIf { storedSession == session }

/** The stream a `StreamStarted` event announced. */
internal data class ListenerStreamStart(
    val sessionId: SessionId,
    val streamId: StreamId,
    val sampleRate: UInt,
    val hostStartTimeMs: ULong,
    val samplesPerPacket: UInt,
    val channels: UShort,
    val startupBufferMs: Long,
)

/** What [ListenerPlaybackController.startNow] did, for the coordinator to report. */
internal sealed interface ListenerStreamStartResult {
    /** Each failure of an earlier stream's stop, in order; each is reported on its own. */
    val stopFailures: List<Throwable>

    /** The running runtime followed a same-stream resume in place. */
    data class Reanchored(val failure: Throwable?) : ListenerStreamStartResult {
        override val stopFailures: List<Throwable> get() = emptyList()
    }

    data class Started(
        val runtime: FfiListenerPlaybackHandle,
        override val stopFailures: List<Throwable>,
    ) : ListenerStreamStartResult

    data class Failed(val error: Throwable, override val stopFailures: List<Throwable>) : ListenerStreamStartResult
}

/**
 * The listener's Rust playback runtime and its clock (God-file refactor spec
 * §4): opening and stopping the runtime on its own single control thread, the
 * native output, the same-session sync handover between streams, and the
 * pre-runtime sync estimator with its probe ids.
 *
 * Everything between an arriving packet and the render ring belongs to the
 * runtime. This class only opens it, points the native output at its ring
 * and closes it again. It never writes UI state: each operation returns what
 * happened and the coordinator reports it.
 *
 * The native seams are injectable because JVM tests cannot load the native
 * libraries.
 */
internal class ListenerPlaybackController(
    parentScope: CoroutineScope,
    private val attachOutput: (FfiListenerPlaybackHandle) -> Unit,
    private val detachOutput: () -> Unit,
    private val openOutput: (engineToken: Long) -> Int = OboeBridge::nativeOboeOpen,
    private val closeOutput: () -> Unit = OboeBridge::nativeOboeClose,
    private val openRuntime: (FfiListenerPlaybackConfig, FfiListenerSyncHandover?) -> FfiListenerPlaybackHandle =
        { config, handover ->
            if (handover != null) {
                FfiListenerPlaybackHandle.openContinuing(config, handover)
            } else {
                FfiListenerPlaybackHandle.open(config)
            }
        },
    private val openEstimator: (maxSamples: Int) -> RustSyncEstimator =
        { RustCoreBridge.openSyncEstimator(maxSamples = it) },
) : ViewModelController(parentScope) {
    /** Start and stop run here, one at a time and in order. Keep it single-threaded. */
    val executor: ExecutorService = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "silent-disco-listener-playback-control").apply { isDaemon = true }
    }
    private val runtimeRef = AtomicReference<FfiListenerPlaybackHandle?>(null)
    private val released = AtomicBoolean(false)

    /** The running runtime, if any. */
    var runtime: FfiListenerPlaybackHandle?
        get() = runtimeRef.get()
        set(value) = runtimeRef.set(value)

    /** Session of [runtime], so its clock lock is only reused within that session. */
    private var runtimeSessionId: SessionId? = null

    /**
     * The last ended stream's clock and sync estimate, keyed by its session.
     * The next stream of the same session opens from it, so it plays from its
     * first packet instead of re-acquiring sync and dropping ~0.8 s of audio.
     * Only touched on [executor].
     */
    internal var syncHandover: SessionSyncHandover? = null
        private set

    /**
     * Rust-owned pre-runtime clock-sync estimator. It holds a real native
     * handle, so every teardown path closes it with [closeEstimator].
     */
    var syncEstimator: RustSyncEstimator? = null
        private set

    /** The pre-runtime probe a response must answer, if one is outstanding. */
    var pendingSyncCorrelationId: Long? = null

    /** Monotonic probe ids, shared by the pre-runtime and runtime-owned paths. */
    private var nextSyncCorrelationId: Long = 1

    fun execute(block: () -> Unit) = executor.execute(block)

    fun nextProbeId(): Long = nextSyncCorrelationId++

    /**
     * Starts playback of [start]'s stream. Runs on [executor].
     *
     * With [reanchorRunning], a running runtime follows a same-stream resume
     * in place. Otherwise the old stream ends where this one begins on the
     * host timeline, and a later stream of the same session continues on the
     * previous stream's clock and sync estimate.
     */
    fun startNow(start: ListenerStreamStart, reanchorRunning: Boolean): ListenerStreamStartResult {
        val running = runtime
        if (running != null && reanchorRunning) {
            return ListenerStreamStartResult.Reanchored(
                runCatching { running.reanchorPresentationTime(start.hostStartTimeMs) }.exceptionOrNull(),
            )
        }
        val stopFailures = listOfNotNull(stopNow(cutAtHostMs = start.hostStartTimeMs))
        val stored = syncHandover
        syncHandover = null
        val handover = handoverForSession(stored?.sessionId, stored?.handover, start.sessionId)
        val opened = runCatching { openRuntime(start.toConfig(), handover) }
            .also { stored?.handover?.close() }
            .getOrElse { error -> return ListenerStreamStartResult.Failed(error, stopFailures) }
        runtime = opened
        runtimeSessionId = start.sessionId

        val failure = runCatching {
            val oboeStatus = openOutput(opened.engineToken())
            check(oboeStatus == LISTENER_OBOE_STATUS_OK) { "Oboe stream failed to open (status=$oboeStatus)" }
            attachOutput(opened)
        }.exceptionOrNull()
        if (failure != null) {
            return ListenerStreamStartResult.Failed(failure, stopFailures + listOfNotNull(stopNow()))
        }
        return ListenerStreamStartResult.Started(opened, stopFailures)
    }

    /**
     * Stops the runtime and the native output. Runs on [executor]. With
     * [cutAtHostMs], buffered audio presented after that host time is dropped
     * instead of played out. Every step is attempted; returns the first
     * failure with the rest suppressed.
     */
    fun stopNow(cutAtHostMs: ULong? = null): Throwable? {
        val running = runtimeRef.getAndSet(null) ?: return null
        var firstFailure: Throwable? = null
        fun capture(block: () -> Unit) {
            try {
                block()
            } catch (error: Throwable) {
                val first = firstFailure
                if (first == null) firstFailure = error else first.addSuppressed(error)
            }
        }
        capture { detachOutput() }
        capture { if (cutAtHostMs != null) running.stopAt(cutAtHostMs) else running.stop() }
        capture { closeOutput() }
        // Keep this stream's clock lock for the session's next stream. It is
        // only ever reused by a stream of the same session (see
        // [handoverForSession]), so a disconnect cannot leak it elsewhere.
        val session = runtimeSessionId
        runtimeSessionId = null
        if (session != null) {
            capture {
                val next = SessionSyncHandover(session, running.syncHandover())
                syncHandover?.handover?.close()
                syncHandover = next
            }
        }
        capture { running.close() }
        return firstFailure
    }

    /** Feeds one completed four-timestamp exchange to the estimator, opening it if needed. */
    fun observeSync(
        t1LocalSendMs: Long,
        t2HostReceiveMs: Long,
        t3HostSendMs: Long,
        t4LocalReceiveMs: Long,
        maxSamples: Int,
    ): RustSyncObservation {
        val estimator = syncEstimator ?: openEstimator(maxSamples).also { syncEstimator = it }
        return estimator.observe(t1LocalSendMs, t2HostReceiveMs, t3HostSendMs, t4LocalReceiveMs)
    }

    /** Replaces the estimator with an empty one for a new sample window. */
    fun reopenEstimator(maxSamples: Int) {
        syncEstimator = openEstimator(maxSamples)
    }

    /** Closes and clears the estimator, if open; returns a close failure. */
    fun closeEstimator(): Throwable? {
        val estimator = syncEstimator ?: return null
        syncEstimator = null
        return runCatching { estimator.close() }.exceptionOrNull()
    }

    /**
     * Releases the handover after any queued stop, stops accepting work and
     * closes the estimator. `MainViewModel.onCleared` calls this at its
     * original point; the registry's later close is a no-op.
     */
    override fun release() {
        if (!released.compareAndSet(false, true)) return
        // Queued after any stop on the same thread, so it also releases the
        // handover that stop captures.
        executor.execute {
            syncHandover?.handover?.close()
            syncHandover = null
        }
        executor.shutdown()
        closeEstimator()
    }
}

private fun ListenerStreamStart.toConfig() = FfiListenerPlaybackConfig(
    sessionId = sessionId.value,
    streamId = streamId.value,
    sampleRate = sampleRate,
    hostStartTimeMs = hostStartTimeMs,
    samplesPerPacket = samplesPerPacket,
    channels = channels,
    startupBufferTargetMs = startupBufferMs.toULong(),
    // Clamped by the scheduler to the startup target above, so a tuning
    // profile with a shallower startup buffer keeps its fast recovery.
    rebufferTargetMs = LISTENER_REBUFFER_TARGET_MS,
    ringCapacityFrames = LISTENER_RING_CAPACITY_FRAMES,
    ringTargetFillFrames = LISTENER_RING_TARGET_FILL_FRAMES,
    writeLeadMs = LISTENER_RING_WRITE_LEAD_MS,
    maxPrefillMs = LISTENER_MAX_RING_PREFILL_MS,
    volume = 1.0f,
)
