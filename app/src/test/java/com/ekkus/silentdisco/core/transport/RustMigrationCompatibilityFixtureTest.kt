package com.ekkus.silentdisco.core.transport

import com.ekkus.silentdisco.app.AppUiState
import com.ekkus.silentdisco.app.TuningField
import com.ekkus.silentdisco.app.TuningSettings
import com.ekkus.silentdisco.app.adjust
import com.ekkus.silentdisco.app.canSelectSession
import com.ekkus.silentdisco.app.classifyTransportSnapshotRole
import com.ekkus.silentdisco.app.nextStateForSyncProbe
import com.ekkus.silentdisco.app.requireHostSessionForPlayback
import com.ekkus.silentdisco.core.audio.AudioFormatSpec
import com.ekkus.silentdisco.core.audio.DecodedAudioChunk
import com.ekkus.silentdisco.core.audio.PcmPacketizer
import com.ekkus.silentdisco.core.model.ApprovalMode
import com.ekkus.silentdisco.core.model.HostLifecycleState
import com.ekkus.silentdisco.core.model.ListenerLifecycleState
import com.ekkus.silentdisco.core.model.SessionInfo
import com.ekkus.silentdisco.core.model.SyncQualityBadge
import com.ekkus.silentdisco.core.persistence.LegacyPreferencesContract
import com.ekkus.silentdisco.core.protocol.AudioPacket
import com.ekkus.silentdisco.core.protocol.ControlMessage
import com.ekkus.silentdisco.core.protocol.DeviceIdentity
import com.ekkus.silentdisco.core.protocol.SessionId
import com.ekkus.silentdisco.core.protocol.StreamId
import com.ekkus.silentdisco.core.sync.ClockSyncEstimator
import com.ekkus.silentdisco.core.sync.ClockSyncSample
import com.google.common.truth.Truth.assertThat
import java.security.MessageDigest
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.double
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import org.junit.Test

class RustMigrationCompatibilityFixtureTest {
    private val json = Json {
        ignoreUnknownKeys = false
        classDiscriminator = "type"
    }

    @Test
    fun controlMessageFixturesMatchProductionSerialization() {
        val fixture = fixture("protocol/control_messages_v1.json")
        assertThat(fixture.int("fixtureVersion")).isEqualTo(1)
        val codec = JsonMessageCodec(ControlMessage.serializer())

        fixture.getValue("messages").jsonArray.forEach { entryElement ->
            val entry = entryElement.jsonObject
            val name = entry.string("name")
            val expectedWire = entry.getValue("wire").jsonObject
            val message = controlMessage(name, expectedWire)
            val encoded = codec.encode(message)
            val actualWire = json.parseToJsonElement(encoded.decodeToString())

            assertThat(actualWire).isEqualTo(expectedWire)
            assertThat(codec.decode(encoded)).isEqualTo(message)
        }
    }

    @Test
    fun clockSyncFixturesMatchProductionEstimator() {
        val fixture = fixture("sync/clock_sync_v1.json")
        val estimator = ClockSyncEstimator(
            maxSamples = fixture.int("maxSamples"),
            maxAcceptedRttMs = fixture.double("maxAcceptedRttMs"),
        )

        fixture.getValue("samples").jsonArray.forEachIndexed { index, sampleElement ->
            val sampleFixture = sampleElement.jsonObject
            val sample = ClockSyncSample(
                t1 = sampleFixture.long("t1"),
                t2 = sampleFixture.long("t2"),
                t3 = sampleFixture.long("t3"),
                t4 = sampleFixture.long("t4"),
            )
            assertThat(sample.rttMs)
                .isWithin(0.000001)
                .of(sampleFixture.double("expectedRttMs"))
            assertThat(sample.offsetMs)
                .isWithin(0.000001)
                .of(sampleFixture.double("expectedOffsetMs"))

            val before = estimator.snapshot()
            val after = estimator.observe(sample)
            if (!sampleFixture.boolean("accepted")) {
                assertThat(after).isEqualTo(before)
            }
        }

        val expected = fixture.getValue("expectedFinalState").jsonObject
        val actual = estimator.snapshot()
        assertThat(actual.offsetMs).isWithin(0.000001).of(expected.double("offsetMs"))
        assertThat(actual.rttMs).isWithin(0.000001).of(expected.double("rttMs"))
        assertThat(actual.jitterMs).isWithin(0.000001).of(expected.double("jitterMs"))
        assertThat(actual.confidence).isEqualTo(SyncQualityBadge.valueOf(expected.string("confidence")))
    }

    @Test
    fun packetizationFixtureMatchesProductionPacketizerAndBinaryCodec() {
        val fixture = fixture("packetization/pcm_packetization_v1.json")
        val formatFixture = fixture.getValue("format").jsonObject
        val chunkFixture = fixture.getValue("chunk").jsonObject
        val format = AudioFormatSpec(
            sampleRate = formatFixture.int("sampleRate"),
            channelCount = formatFixture.int("channelCount"),
            bytesPerSample = formatFixture.int("bytesPerSample"),
        )
        val packets = PcmPacketizer(
            sessionId = SessionId(fixture.string("sessionId")),
            streamId = StreamId(fixture.string("streamId")),
            format = format,
            packetDurationMs = fixture.int("packetDurationMs"),
        ).packetize(
            chunk = DecodedAudioChunk(
                pcm16Le = hexToBytes(chunkFixture.string("pcm16LeHex")),
                firstSampleIndex = chunkFixture.long("firstSampleIndex"),
                frameCount = chunkFixture.int("frameCount"),
            ),
            hostPresentationStartMs = fixture.long("hostPresentationStartMs"),
        )
        val expectedPackets = fixture.getValue("expectedPackets").jsonArray
        assertThat(packets).hasSize(expectedPackets.size)

        packets.zip(expectedPackets).forEachIndexed { index, (packet, expectedElement) ->
            val expected = expectedElement.jsonObject
            assertPacket(index, packet, fixture, expected)

            val decoded = AudioPacketCodec.decode(AudioPacketCodec.encode(packet))
            assertThat(decoded.version).isEqualTo(packet.version)
            assertThat(decoded.sessionId).isEqualTo(packet.sessionId)
            assertThat(decoded.streamId).isEqualTo(packet.streamId)
            assertThat(decoded.sequenceNumber).isEqualTo(packet.sequenceNumber)
            assertThat(decoded.codec).isEqualTo(packet.codec)
            assertThat(decoded.sampleRate).isEqualTo(packet.sampleRate)
            assertThat(decoded.channelCount).isEqualTo(packet.channelCount)
            assertThat(decoded.samplesPerPacket).isEqualTo(packet.samplesPerPacket)
            assertThat(decoded.firstSampleIndex).isEqualTo(packet.firstSampleIndex)
            assertThat(decoded.hostPresentationTimeMs).isEqualTo(packet.hostPresentationTimeMs)
            assertThat(decoded.payload).isEqualTo(packet.payload)
            assertThat(decoded.checksum).isEqualTo(packet.checksum)
        }
    }

    @Test
    fun stateTransitionFixturesMatchProductionHelpers() {
        val fixture = fixture("state/state_transitions_v1.json")

        fixture.getValue("syncProbeTransitions").jsonArray.forEach { caseElement ->
            val case = caseElement.jsonObject
            val current = ListenerLifecycleState.valueOf(case.string("current"))
            val expected = ListenerLifecycleState.valueOf(case.string("expected"))
            assertThat(nextStateForSyncProbe(current)).isEqualTo(expected)
        }

        fixture.getValue("transportFailureClassifications").jsonArray.forEach { caseElement ->
            val case = caseElement.jsonObject
            val actual = classifyTransportSnapshotRole(
                hostState = HostLifecycleState.valueOf(case.string("hostState")),
                listenerState = ListenerLifecycleState.valueOf(case.string("listenerState")),
            )
            assertThat(actual.name).isEqualTo(case.string("expected"))
        }

        fixture.getValue("sessionSelection").jsonArray.forEach { caseElement ->
            val case = caseElement.jsonObject
            val selected = session(case.string("selectedSessionId"))
            val candidate = session(case.string("candidateSessionId"))
            val state = AppUiState(
                listenerState = ListenerLifecycleState.valueOf(case.string("listenerState")),
                selectedSession = selected,
            )
            assertThat(state.canSelectSession(candidate)).isEqualTo(case.boolean("expectedAllowed"))
        }

        fixture.getValue("hostPlaybackIdentity").jsonArray.forEach { caseElement ->
            val case = caseElement.jsonObject
            val sessionId = case.optionalString("sessionId")?.let(::SessionId)
            assertThat(requireHostSessionForPlayback(sessionId)).isEqualTo(case.optionalString("expectedError"))
        }
    }

    @Test
    fun tuningFixturesMatchProductionNormalization() {
        val fixture = fixture("tuning/tuning_normalization_v1.json")

        fixture.getValue("cases").jsonArray.forEach { caseElement ->
            val case = caseElement.jsonObject
            val input = tuningSettings(case.getValue("input").jsonObject)
            val actual = when (case.string("operation")) {
                "normalize" -> input.normalized()
                "adjust" -> input.adjust(
                    field = TuningField.valueOf(case.string("field")),
                    direction = case.int("direction"),
                )
                else -> error("Unknown tuning fixture operation ${case.string("operation")}")
            }
            assertTuningFields(case.string("name"), actual, case.getValue("expected").jsonObject)
        }
    }

    @Test
    fun persistenceFixtureMatchesProductionLegacyPreferenceContract() {
        val fixture = fixture("persistence/legacy_preferences_v1.json")
        val expectedKeys = mapOf(
            "syncSampleWindow" to LegacyPreferencesContract.SYNC_SAMPLE_WINDOW,
            "syncCadenceMs" to LegacyPreferencesContract.SYNC_CADENCE_MS,
            "startupBufferMs" to LegacyPreferencesContract.STARTUP_BUFFER_MS,
            "latePacketThresholdMs" to LegacyPreferencesContract.LATE_PACKET_THRESHOLD_MS,
            "hardResyncThresholdMs" to LegacyPreferencesContract.HARD_RESYNC_THRESHOLD_MS,
            "syncDriftThresholdBits" to LegacyPreferencesContract.SYNC_DRIFT_THRESHOLD_BITS,
        )

        fixture.getValue("tuning").jsonArray.forEach { entryElement ->
            val entry = entryElement.jsonObject
            assertThat(entry.string("key"))
                .isEqualTo(expectedKeys.getValue(entry.string("name")))
        }

        val trusted = fixture.getValue("trustedDevice").jsonObject
        assertThat(trusted.string("key"))
            .isEqualTo(LegacyPreferencesContract.trustedDeviceKey(trusted.string("deviceId")))
        assertThat(fixture.boolean("containsSecrets")).isFalse()
    }

    private fun assertPacket(index: Int, packet: AudioPacket, root: JsonObject, expected: JsonObject) {
        assertThat(packet.version).isEqualTo(expected.int("version"))
        assertThat(packet.sessionId.value).isEqualTo(root.string("sessionId"))
        assertThat(packet.streamId.value).isEqualTo(root.string("streamId"))
        assertThat(packet.sequenceNumber).isEqualTo(expected.long("sequenceNumber"))
        assertThat(packet.codec).isEqualTo(expected.string("codec"))
        assertThat(packet.sampleRate).isEqualTo(expected.int("sampleRate"))
        assertThat(packet.channelCount).isEqualTo(expected.int("channelCount"))
        assertThat(packet.samplesPerPacket).isEqualTo(expected.int("samplesPerPacket"))
        assertThat(packet.firstSampleIndex).isEqualTo(expected.long("firstSampleIndex"))
        assertThat(packet.hostPresentationTimeMs).isEqualTo(expected.long("hostPresentationTimeMs"))
        assertThat(packet.payload).isEqualTo(hexToBytes(expected.string("payloadHex")))
        assertThat(sha256(packet.payload)).isEqualTo(expected.string("payloadSha256"))
        assertThat(packet.checksum).isEqualTo(expected.int("checksum"))
    }

    private fun tuningSettings(overrides: JsonObject): TuningSettings = TuningSettings(
        syncSampleWindow = overrides.intOrDefault("syncSampleWindow", 12),
        syncCadenceMs = overrides.longOrDefault("syncCadenceMs", 2_000L),
        startupBufferMs = overrides.longOrDefault("startupBufferMs", 400L),
        latePacketThresholdMs = overrides.longOrDefault("latePacketThresholdMs", 40L),
        hardResyncThresholdMs = overrides.longOrDefault("hardResyncThresholdMs", 120L),
        syncDriftThresholdMs = overrides.doubleOrDefault("syncDriftThresholdMs", 18.0),
        scanWindowMs = overrides.longOrDefault("scanWindowMs", 3_000L),
    )

    private fun assertTuningFields(name: String, actual: TuningSettings, expected: JsonObject) {
        expected["syncSampleWindow"]?.let { assertThat(actual.syncSampleWindow).isEqualTo(it.jsonPrimitive.int) }
        expected["syncCadenceMs"]?.let { assertThat(actual.syncCadenceMs).isEqualTo(it.jsonPrimitive.long) }
        expected["startupBufferMs"]?.let { assertThat(actual.startupBufferMs).isEqualTo(it.jsonPrimitive.long) }
        expected["latePacketThresholdMs"]?.let {
            assertThat(actual.latePacketThresholdMs).isEqualTo(it.jsonPrimitive.long)
        }
        expected["hardResyncThresholdMs"]?.let {
            assertThat(actual.hardResyncThresholdMs).isEqualTo(it.jsonPrimitive.long)
        }
        expected["syncDriftThresholdMs"]?.let {
            assertThat(actual.syncDriftThresholdMs).isWithin(0.000001).of(it.jsonPrimitive.double)
        }
        expected["scanWindowMs"]?.let { assertThat(actual.scanWindowMs).isEqualTo(it.jsonPrimitive.long) }
    }

    private fun controlMessage(name: String, wire: JsonObject): ControlMessage {
        val sessionId = SessionId(wire.string("sessionId"))
        return when (name) {
            "hello" -> ControlMessage.Hello(
                version = wire.int("version"),
                sessionId = sessionId,
                sessionName = wire.string("sessionName"),
                hostName = wire.string("hostName"),
                approvalRequired = wire.boolean("approvalRequired"),
            )
            "join_request" -> {
                val device = wire.getValue("device").jsonObject
                ControlMessage.JoinRequest(
                    version = wire.int("version"),
                    sessionId = sessionId,
                    device = DeviceIdentity(
                        deviceId = device.string("deviceId"),
                        displayName = device.string("displayName"),
                    ),
                    inviteCode = wire.optionalString("inviteCode"),
                )
            }
            "join_approval" -> ControlMessage.JoinApproval(
                version = wire.int("version"),
                sessionId = sessionId,
                listenerId = wire.string("listenerId"),
                trustedForFuture = wire.boolean("trustedForFuture"),
            )
            "join_rejection" -> ControlMessage.JoinRejection(
                version = wire.int("version"),
                sessionId = sessionId,
                listenerId = wire.string("listenerId"),
                reason = wire.string("reason"),
            )
            "heartbeat" -> ControlMessage.Heartbeat(
                version = wire.int("version"),
                sessionId = sessionId,
                listenerId = wire.string("listenerId"),
                sentAtElapsedMs = wire.long("sentAtElapsedMs"),
            )
            "stream_start" -> ControlMessage.StreamStart(
                version = wire.int("version"),
                sessionId = sessionId,
                streamId = StreamId(wire.string("streamId")),
                hostStartTimeMs = wire.long("hostStartTimeMs"),
                sampleRate = wire.int("sampleRate"),
                channels = wire.int("channels"),
                samplesPerPacket = wire.int("samplesPerPacket"),
            )
            "pause" -> ControlMessage.Pause(
                version = wire.int("version"),
                sessionId = sessionId,
                streamId = StreamId(wire.string("streamId")),
                hostPauseTimeMs = wire.long("hostPauseTimeMs"),
            )
            "stop" -> ControlMessage.Stop(
                version = wire.int("version"),
                sessionId = sessionId,
                streamId = StreamId(wire.string("streamId")),
                hostStopTimeMs = wire.long("hostStopTimeMs"),
            )
            "disconnect" -> ControlMessage.Disconnect(
                version = wire.int("version"),
                sessionId = sessionId,
                listenerId = wire.string("listenerId"),
                reason = wire.string("reason"),
            )
            "resync_notice" -> ControlMessage.ResyncNotice(
                version = wire.int("version"),
                sessionId = sessionId,
                listenerId = wire.string("listenerId"),
                reason = wire.string("reason"),
            )
            else -> error("Unknown control fixture $name")
        }
    }

    private fun session(id: String): SessionInfo = SessionInfo(
        id = id,
        name = "Session $id",
        hostDeviceName = "Host $id",
        approvalMode = ApprovalMode.MANUAL,
        inviteCodeRequired = false,
    )

    private fun fixture(relativePath: String): JsonObject {
        val resourcePath = "rust-migration/$relativePath"
        val stream = checkNotNull(javaClass.classLoader?.getResourceAsStream(resourcePath)) {
            "Missing compatibility fixture $resourcePath"
        }
        return stream.bufferedReader().use { reader ->
            json.parseToJsonElement(reader.readText()).jsonObject
        }
    }

    private fun hexToBytes(value: String): ByteArray {
        require(value.length % 2 == 0) { "Hex fixture must contain complete bytes" }
        return value.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
    }

    private fun sha256(bytes: ByteArray): String = MessageDigest.getInstance("SHA-256")
        .digest(bytes)
        .joinToString(separator = "") { byte -> "%02x".format(byte.toInt() and 0xff) }

    private fun JsonObject.string(key: String): String = getValue(key).jsonPrimitive.content
    private fun JsonObject.optionalString(key: String): String? = when (val value: JsonElement? = get(key)) {
        null, JsonNull -> null
        else -> value.jsonPrimitive.contentOrNull
    }
    private fun JsonObject.int(key: String): Int = getValue(key).jsonPrimitive.int
    private fun JsonObject.long(key: String): Long = getValue(key).jsonPrimitive.long
    private fun JsonObject.double(key: String): Double = getValue(key).jsonPrimitive.double
    private fun JsonObject.boolean(key: String): Boolean = getValue(key).jsonPrimitive.boolean
    private fun JsonObject.intOrDefault(key: String, default: Int): Int = get(key)?.jsonPrimitive?.int ?: default
    private fun JsonObject.longOrDefault(key: String, default: Long): Long = get(key)?.jsonPrimitive?.long ?: default
    private fun JsonObject.doubleOrDefault(key: String, default: Double): Double =
        get(key)?.jsonPrimitive?.double ?: default
}
