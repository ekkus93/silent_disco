import type { DesktopErrorDto, HostInvitationDto } from "../../core/generated/desktop-bindings";
import { formatWallClock, isInvitationExpired } from "./domain";
import { CopyButton, Detail, ErrorAlert } from "./shared";

interface QrInvitationPanelProps {
  invitation: HostInvitationDto | null;
  invitationQrDataUrl: string | null;
  invitationPending: boolean;
  invitationError: DesktopErrorDto | null;
  onRefreshInvitation: () => void;
  onCopy: (label: string, value: string) => void;
}

export function QrInvitationPanel({
  invitation,
  invitationQrDataUrl,
  invitationPending,
  invitationError,
  onRefreshInvitation,
  onCopy,
}: QrInvitationPanelProps) {
  return (
    <section
      aria-labelledby="invitation-title"
      className="mt-6 rounded-2xl border border-slate-700 bg-slate-950/70 p-5"
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 id="invitation-title" className="text-xl font-semibold">
            QR invitation
          </h2>
          <p className="mt-2 text-sm text-slate-400">
            A signed, time-limited invitation a phone can scan to join directly -- a convenience
            alongside the manual details above, not a replacement for them.
          </p>
        </div>
        <button
          type="button"
          onClick={onRefreshInvitation}
          disabled={invitationPending}
          className="rounded-lg border border-cyan-500/60 px-4 py-2 text-sm font-semibold text-cyan-100 hover:bg-cyan-950/40 disabled:opacity-50"
        >
          {invitationPending
            ? "Creating…"
            : invitation
              ? "Refresh QR invitation"
              : "Create QR invitation"}
        </button>
      </div>

      {invitationError ? <ErrorAlert error={invitationError} /> : null}

      {invitation ? (
        isInvitationExpired(invitation) ? (
          <p
            role="status"
            className="mt-4 rounded-xl border border-amber-500/50 bg-amber-950/30 p-4 text-amber-100"
          >
            This invitation expired. Create a new one -- an expired invitation is never reused
            automatically.
          </p>
        ) : (
          <div className="mt-4 flex flex-wrap items-start gap-4">
            {invitationQrDataUrl ? (
              <img
                src={invitationQrDataUrl}
                alt="Signed Silent Disco join QR code"
                className="h-56 w-56 rounded-xl bg-white p-2"
              />
            ) : (
              <p
                role="alert"
                className="rounded-xl border border-rose-500/60 bg-rose-950/40 p-4 text-rose-100"
              >
                The signed invitation was created, but this app could not render its QR image. Use
                the text fallback below instead.
              </p>
            )}
            <div className="min-w-[16rem] flex-1 space-y-3">
              <Detail label="Expires" value={formatWallClock(invitation.expiresAtMs)} />
              <CopyButton
                label="Copy invitation text"
                onClick={() => onCopy("Invitation text", invitation.payload)}
              />
            </div>
          </div>
        )
      ) : (
        <p className="mt-4 text-sm text-slate-400">
          No invitation created yet. This does not affect manual connections above.
        </p>
      )}
    </section>
  );
}
