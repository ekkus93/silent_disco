#!/usr/bin/env python3
"""Validate and summarize raw Block 45 performance reports without hiding outliers."""

from __future__ import annotations

import argparse
import json
import math
import statistics
from pathlib import Path
from typing import Any


def load_object(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise AssertionError(f"{path}: JSON root must be an object")
    return data


def validate_core(report: dict[str, Any], path: Path, *, expect_soak: bool) -> None:
    assert report["schemaVersion"] == 1, f"{path}: unexpected core-probe schema"
    assert [entry["format"] for entry in report["decoder"]] == ["wav", "flac", "mp3"]
    transports = report["transport"]
    no_fault_counts = {
        entry["listenerCount"] for entry in transports if entry["configuredLossPermille"] == 0
    }
    assert no_fault_counts >= {1, 2, 5, 16}, f"{path}: incomplete listener matrix"
    loss_counts = {
        entry["listenerCount"] for entry in transports if entry["configuredLossPermille"] == 50
    }
    assert loss_counts >= {5, 16}, f"{path}: incomplete moderate-loss matrix"
    for entry in transports:
        assert entry["transportDeliveryFailures"] == 0
        assert entry["failedPeerDeliveries"] == 0
        if entry["configuredLossPermille"] == 0:
            assert entry["receivedAudioEvents"] == entry["expectedAudioEventsWithoutFaults"]
    assert report["reconnect"]["postReconnectAudioReceived"] is True
    assert report["database"]["readIterations"] > 0
    assert report["database"]["writeIterations"] > 0
    if expect_soak:
        soak = report["soak"]
        assert soak is not None
        assert soak["requestedSeconds"] > 0
        assert soak["listenerCount"] == 16
        assert soak["packetsBroadcast"] > 0
        assert soak["transportDeliveryFailures"] == 0
    else:
        assert report["soak"] is None


def validate_runtime(report: dict[str, Any], path: Path) -> None:
    assert report["schemaVersion"] == 1, f"{path}: unexpected runtime-probe schema"
    runtime = report["desktopRuntime"]
    queue = runtime["transportQueue"]
    assert 0 < queue["queuePeakDepth"] <= queue["queueCapacity"]
    assert queue["queueDepthAtEnd"] == 0
    assert queue["queueOverflows"] == 0
    assert queue["recipientsIntended"] == queue["recipientsDelivered"]
    assert queue["recipientsDelivered"] == queue["receivedAudioEvents"]
    assert queue["deliverySeverity"] == "ok"

    bridge = runtime["notificationBridge"]
    assert 0 < bridge["queuePeakDepth"] <= bridge["queueCapacity"]
    assert bridge["queueDepthAtEnd"] == 0
    assert bridge["notificationsDelivered"] == bridge["notificationsSubmitted"] + 1

    monitor = runtime["monitorCallback"]
    assert monitor["callbacksObserved"] == monitor["callbackIterations"] + 1
    assert monitor["framesSilenceFilled"] == monitor["framesPerCallback"]
    assert monitor["ringUnderrunCallbacks"] == 1

    scheduler = runtime["scheduler"]
    assert scheduler["concealedPackets"] == 1
    assert scheduler["concealmentDrivenRebuffers"] == 0

    sync = runtime["synchronization"]
    assert sync["samples"] == 12
    assert sync["confidence"] in {"good", "excellent"}
    assert math.isclose(sync["offsetMs"], 12.0, abs_tol=1e-9)
    assert math.isclose(sync["roundTripMs"], 20.0, abs_tol=1e-9)


def nested(report: dict[str, Any], *keys: str) -> Any:
    value: Any = report
    for key in keys:
        value = value[key]
    return value


def series_summary(values: list[int | float | None]) -> dict[str, Any]:
    present = [value for value in values if value is not None]
    summary: dict[str, Any] = {"allSamples": values}
    if present:
        summary.update(
            minimum=min(present),
            median=statistics.median(present),
            maximum=max(present),
        )
    else:
        summary.update(minimum=None, median=None, maximum=None)
    return summary


def metric_series(
    core_reports: list[dict[str, Any]], runtime_reports: list[dict[str, Any]]
) -> dict[str, dict[str, Any]]:
    series: dict[str, list[int | float | None]] = {
        "packetizer.packetsPerSecond": [
            nested(report, "packetizer", "packetsPerSecond") for report in core_reports
        ],
        "database.averageReadLatencyMicros": [
            nested(report, "database", "averageReadLatencyMicros") for report in core_reports
        ],
        "database.averageWriteLatencyMicros": [
            nested(report, "database", "averageWriteLatencyMicros") for report in core_reports
        ],
        "database.shutdownElapsedMs": [
            nested(report, "database", "shutdownElapsedMs") for report in core_reports
        ],
        "reconnect.elapsedMs": [
            nested(report, "reconnect", "reconnectElapsedMs") for report in core_reports
        ],
        "desktop.transportQueuePeakDepth": [
            nested(report, "desktopRuntime", "transportQueue", "queuePeakDepth")
            for report in runtime_reports
        ],
        "desktop.notificationBacklogPeak": [
            nested(report, "desktopRuntime", "notificationBridge", "queuePeakDepth")
            for report in runtime_reports
        ],
        "desktop.monitorCallbackAverageNanos": [
            nested(report, "desktopRuntime", "monitorCallback", "averageCallbackNanos")
            for report in runtime_reports
        ],
        "desktop.monitorCallbackMaximumNanos": [
            nested(report, "desktopRuntime", "monitorCallback", "maxCallbackNanos")
            for report in runtime_reports
        ],
        "desktop.transportShutdownMicros": [
            nested(report, "desktopRuntime", "transportQueue", "shutdownElapsedMicros")
            for report in runtime_reports
        ],
        "scheduler.concealmentElapsedMicros": [
            nested(report, "desktopRuntime", "scheduler", "elapsedMicros")
            for report in runtime_reports
        ],
    }
    for extension in ("wav", "flac", "mp3"):
        series[f"decoder.{extension}.realtimeMultipleMilli"] = [
            next(
                entry["realtimeMultipleMilli"]
                for entry in report["decoder"]
                if entry["format"] == extension
            )
            for report in core_reports
        ]
    for listeners, loss in ((1, 0), (2, 0), (5, 0), (16, 0), (5, 50), (16, 50)):
        label = f"transport.listeners{listeners}.loss{loss}permille.peerDeliveriesPerSecond"
        series[label] = [
            next(
                entry["peerDeliveriesPerSecond"]
                for entry in report["transport"]
                if entry["listenerCount"] == listeners
                and entry["configuredLossPermille"] == loss
            )
            for report in core_reports
        ]
    return {name: series_summary(values) for name, values in sorted(series.items())}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--baseline", nargs="+", required=True, type=Path)
    parser.add_argument("--runtime", nargs="+", required=True, type=Path)
    parser.add_argument("--soak", required=True, type=Path)
    parser.add_argument("--ui-cadence", required=True, type=Path)
    parser.add_argument("--jitter-evidence", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    core_reports = [load_object(path) for path in args.baseline]
    runtime_reports = [load_object(path) for path in args.runtime]
    if len(core_reports) < 3 or len(runtime_reports) != len(core_reports):
        raise AssertionError("Block 45 requires at least three paired baseline/runtime samples")
    for path, report in zip(args.baseline, core_reports, strict=True):
        validate_core(report, path, expect_soak=False)
    for path, report in zip(args.runtime, runtime_reports, strict=True):
        validate_runtime(report, path)

    soak_report = load_object(args.soak)
    validate_core(soak_report, args.soak, expect_soak=True)

    ui_cadence = load_object(args.ui_cadence)
    assert ui_cadence["pollIntervalMs"] == 2_000
    assert math.isclose(ui_cadence["pollsPerSecond"], 0.5, abs_tol=1e-12)
    jitter = load_object(args.jitter_evidence)
    assert jitter == {
        "fixedLatencyMs": 100,
        "jitterMs": 20,
        "seed": 7,
        "evidenceTest": "lab::fault::tests::jitter_keeps_the_deadline_within_its_configured_bound",
    }

    summary = {
        "schemaVersion": 1,
        "baselineRunCount": len(core_reports),
        "environmentSamples": [report["environment"] for report in core_reports],
        "metrics": metric_series(core_reports, runtime_reports),
        "selectedLongSoak": soak_report["soak"],
        "uiCadence": ui_cadence,
        "moderateJitterEvidence": jitter,
        "thresholdPolicy": (
            "No wall-clock performance thresholds are enforced until repeated measurements "
            "on the selected Linux baseline justify stable limits; correctness and bounded-queue "
            "invariants remain mandatory."
        ),
    }
    args.output.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(summary, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
