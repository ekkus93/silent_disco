import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { DesktopErrorDto, HostInvitationDto } from "../../core/generated/desktop-bindings";
import { QrInvitationPanel } from "./QrInvitationPanel";

const invitation: HostInvitationDto = {
  payload: "signed-invitation",
  expiresAtMs: "11000",
};

const props = {
  invitation,
  invitationQrDataUrl: "data:image/png;base64,qr",
  invitationPending: false,
  invitationError: null as DesktopErrorDto | null,
  onRefreshInvitation: vi.fn(),
  onCopy: vi.fn(),
};

describe("QrInvitationPanel", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("hides the QR as soon as its invitation expires without another render trigger", () => {
    vi.useFakeTimers();
    vi.setSystemTime(10_000);

    render(<QrInvitationPanel {...props} />);
    expect(screen.getByAltText("Signed Silent Disco join QR code")).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(1_000);
    });

    expect(screen.getByRole("status")).toHaveTextContent(/This invitation expired/);
    expect(screen.queryByAltText("Signed Silent Disco join QR code")).not.toBeInTheDocument();
  });
});
