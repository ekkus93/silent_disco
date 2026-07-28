import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "./App";

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

describe("App", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("renders the real shared-core result", async () => {
    invokeMock.mockResolvedValue({
      major: 0,
      minor: 1,
      patch: 0,
      smoke: "6000001225524396033",
    });

    render(<App />);

    expect(screen.getByRole("status")).toHaveTextContent("Verifying the shared core");
    expect(await screen.findByText("0.1.0")).toBeVisible();
    expect(screen.getByText("6000001225524396033")).toBeVisible();
    expect(invokeMock).toHaveBeenCalledWith("get_core_smoke", { input: 42 });
  });

  it("keeps bridge failure visible", async () => {
    invokeMock.mockRejectedValue(new Error("native bridge unavailable"));

    render(<App />);

    expect(await screen.findByRole("alert")).toHaveTextContent("Shared core verification failed");
    expect(screen.getByRole("alert")).toHaveTextContent("native bridge unavailable");
  });
});
