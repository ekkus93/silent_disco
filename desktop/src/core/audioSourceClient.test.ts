import { beforeEach, describe, expect, it, vi } from "vitest";

import { selectAudioSource } from "./audioSourceClient";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

describe("desktop audio source client", () => {
  beforeEach(() => invokeMock.mockReset());

  it("submits only the revision and preserves dialog cancellation", async () => {
    invokeMock.mockResolvedValue(null);

    await expect(selectAudioSource("17")).resolves.toBeNull();
    expect(invokeMock).toHaveBeenCalledWith("select_audio_source", {
      request: { expectedRevision: "17" },
    });
  });

  it("normalizes a structured native inspection failure", async () => {
    invokeMock.mockRejectedValue({
      code: "desktop.audio_source.permission_denied",
      subsystem: "audio_source",
      severity: "error",
      retryable: true,
      message: "permission denied",
    });

    try {
      await selectAudioSource("18");
      throw new Error("audio source selection unexpectedly succeeded");
    } catch (error: unknown) {
      expect(error).toBeInstanceOf(Error);
      const structured = error as Error & {
        code?: string;
        subsystem?: string;
        severity?: string;
        retryable?: boolean;
      };
      expect(structured.message).toBe("permission denied");
      expect(structured.code).toBe("desktop.audio_source.permission_denied");
      expect(structured.subsystem).toBe("audio_source");
      expect(structured.severity).toBe("error");
      expect(structured.retryable).toBe(true);
    }
  });
});
