import { invoke } from "@tauri-apps/api/core";

import { toDesktopBridgeError } from "./client";
import type {
  CommandReceiptDto,
  DesktopErrorDto,
  RevisionCommandRequest,
} from "./generated/desktop-bindings";

export class DesktopAudioSourceError extends Error implements DesktopErrorDto {
  readonly code: string;
  readonly subsystem: string;
  readonly severity: string;
  readonly retryable: boolean;

  constructor(failure: DesktopErrorDto) {
    super(failure.message);
    this.name = "DesktopAudioSourceError";
    this.code = failure.code;
    this.subsystem = failure.subsystem;
    this.severity = failure.severity;
    this.retryable = failure.retryable;
  }
}

export async function selectAudioSource(
  expectedRevision: string,
): Promise<CommandReceiptDto | null> {
  const request: RevisionCommandRequest = { expectedRevision };
  try {
    return await invoke<CommandReceiptDto | null>("select_audio_source", { request });
  } catch (error: unknown) {
    throw new DesktopAudioSourceError(toDesktopBridgeError(error, "select_audio_source"));
  }
}
