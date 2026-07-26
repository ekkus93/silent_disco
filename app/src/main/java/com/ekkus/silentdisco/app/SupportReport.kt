package com.ekkus.silentdisco.app

private const val REDACTED = "[redacted]"

fun AppUiState.buildSupportReport(
    appVersion: String,
    generatedAt: String,
): String {
    val sensitiveValues = buildSet {
        hostForm.inviteCode.takeIf(String::isNotBlank)?.let(::add)
        hostDiagnostics.sessionId.takeIf(String::isNotBlank)?.let(::add)
        listenerDiagnostics.sessionId.takeIf(String::isNotBlank)?.let(::add)
        selectedSession?.id?.takeIf(String::isNotBlank)?.let(::add)
        hostForm.selectedAudio?.uri?.toString()?.takeIf(String::isNotBlank)?.let(::add)
    }

    fun safe(value: String?): String = sensitiveValues
        .sortedByDescending(String::length)
        .fold(value ?: "none") { result, sensitive ->
            result.replace(sensitive, REDACTED, ignoreCase = false)
        }

    return buildString {
        appendLine("Silent Disco support report")
        appendLine("Generated: ${safe(generatedAt)}")
        appendLine("App version: ${safe(appVersion)}")
        appendLine()
        appendLine("Summary")
        appendLine("Role: ${selectedRole?.name?.lowercase() ?: "not selected"}")
        appendLine("Host status: ${hostState.label()}")
        appendLine("Listener status: ${listenerStateLabel()}")
        appendLine("Host playback: ${hostPlaybackState.label()}")
        appendLine("Listener playback: ${listenerPlaybackState.label()}")
        appendLine("Last visible problem: ${safe(lastError)}")
        appendLine()
        appendLine("--- Advanced diagnostics ---")
        appendLine("Host listeners: ${hostDiagnostics.connectedListenerCount}/${hostDiagnostics.listenerCount}")
        appendLine("Host pending requests: ${hostDiagnostics.pendingJoinCount}")
        appendLine("Host desynced listeners: ${hostDiagnostics.desyncedListenerCount}")
        appendLine("Host packet sends: ${hostDiagnostics.packetSendCount}")
        appendLine("Host packet budget: ${safe(hostDiagnostics.packetBudgetSummary)}")
        appendLine("Host metrics: ${safe(hostDiagnostics.metricsSummary)}")
        appendLine("Listener offset: ${listenerDiagnostics.hostOffsetMs} ms")
        appendLine("Listener RTT: ${listenerDiagnostics.rttMs} ms")
        appendLine("Listener jitter: ${listenerDiagnostics.jitterMs} ms")
        appendLine("Listener buffer depth: ${listenerDiagnostics.bufferDepthMs} ms")
        appendLine("Listener packet loss: ${listenerDiagnostics.packetLossCount}")
        appendLine("Listener late drops: ${listenerDiagnostics.lateDropCount}")
        appendLine("Listener underruns: ${listenerDiagnostics.underrunCount}")
        appendLine("Listener invalid packets: ${listenerDiagnostics.invalidPacketCount}")
        appendLine("Listener concealed packets: ${listenerDiagnostics.concealedPacketCount}")
        appendLine("Listener reconnects: ${listenerDiagnostics.reconnectCount}")
        appendLine("Listener resyncs: ${listenerDiagnostics.resyncCount}")
        appendLine("Listener metrics: ${safe(listenerDiagnostics.metricsSummary)}")
        appendLine("Tuning: ${tuningSettings.summary()}")
        appendLine()
        appendLine("Privacy note: invite codes, internal session identifiers, device identifiers, and file paths are omitted or redacted.")
    }
}
