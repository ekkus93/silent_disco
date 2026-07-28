import { invoke } from "@tauri-apps/api/core";

export interface CoreSmokeDto {
  major: number;
  minor: number;
  patch: number;
  smoke: string;
}

export async function getCoreSmoke(input: number): Promise<CoreSmokeDto> {
  if (!Number.isSafeInteger(input) || input < 0) {
    throw new Error("Core smoke input must be a non-negative safe integer.");
  }

  return invoke<CoreSmokeDto>("get_core_smoke", { input });
}
