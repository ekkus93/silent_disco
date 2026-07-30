import { invoke } from "@tauri-apps/api/core";

import type { CommandReceiptDto, RevisionCommandRequest } from "./generated/desktop-bindings";

export async function selectAudioSource(
  expectedRevision: string,
): Promise<CommandReceiptDto | null> {
  const request: RevisionCommandRequest = { expectedRevision };
  return invoke<CommandReceiptDto | null>("select_audio_source", { request });
}
