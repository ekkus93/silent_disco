import { invoke } from "@tauri-apps/api/core";

import { toDesktopBridgeError } from "./client";
import type { CommandReceiptDto, RevisionCommandRequest } from "./generated/desktop-bindings";

export async function selectAudioSource(
  expectedRevision: string,
): Promise<CommandReceiptDto | null> {
  const request: RevisionCommandRequest = { expectedRevision };
  try {
    return await invoke<CommandReceiptDto | null>("select_audio_source", { request });
  } catch (error: unknown) {
    throw toDesktopBridgeError(error, "select_audio_source");
  }
}
