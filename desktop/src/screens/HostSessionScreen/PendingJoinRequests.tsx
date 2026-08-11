import type { DesktopErrorDto, PendingJoinRequestDto } from "../../core/generated/desktop-bindings";
import { formatAge, type PendingOperation } from "./domain";
import { ErrorAlert } from "./shared";

interface PendingJoinRequestsProps {
  requests: PendingJoinRequestDto[];
  decisionOperations: Record<string, PendingOperation>;
  decisionFailures: Record<string, DesktopErrorDto>;
  remember: Record<string, boolean>;
  setRemember: (updater: (current: Record<string, boolean>) => Record<string, boolean>) => void;
  onDecide: (requestId: string, kind: "approve" | "reject") => void;
}

export function PendingJoinRequests({
  requests,
  decisionOperations,
  decisionFailures,
  remember,
  setRemember,
  onDecide,
}: PendingJoinRequestsProps) {
  return (
    <section
      aria-labelledby="pending-title"
      className="mt-6 rounded-2xl border border-slate-700 bg-slate-950/70 p-5"
    >
      <h2 id="pending-title" className="text-xl font-semibold">
        Pending join requests
      </h2>
      {requests.length === 0 ? (
        <p className="mt-3 text-sm text-slate-400">No pending requests.</p>
      ) : (
        <ul className="mt-4 space-y-3">
          {requests.map((request) => {
            const operation = decisionOperations[request.requestId];
            const failure = decisionFailures[request.requestId];
            return (
              <li
                key={request.requestId}
                aria-busy={Boolean(operation)}
                className="rounded-xl bg-slate-900 p-4"
              >
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div>
                    <p className="font-semibold">{request.displayName}</p>
                    <p className="mt-1 font-mono text-xs text-slate-400">
                      Request {request.requestId}
                    </p>
                    <p className="font-mono text-xs text-slate-500">Device {request.deviceId}</p>
                  </div>
                  <div className="text-right text-xs text-slate-300">
                    <p>Age: {formatAge(request.ageMs)}</p>
                    <p>Trust: {request.trustState}</p>
                    <p>Invite: {request.inviteCodeValid ? "valid" : "not valid"}</p>
                  </div>
                </div>

                <fieldset
                  disabled={Boolean(operation)}
                  className="mt-4 flex flex-wrap items-center gap-3"
                >
                  <legend className="sr-only">Decision for {request.displayName}</legend>
                  <label className="flex items-center gap-2 text-sm text-slate-200">
                    <input
                      type="checkbox"
                      checked={remember[request.requestId] ?? false}
                      disabled={request.trustState === "trusted"}
                      onChange={(event) =>
                        setRemember((current) => ({
                          ...current,
                          [request.requestId]: event.target.checked,
                        }))
                      }
                    />
                    Remember this device
                  </label>
                  <button
                    type="button"
                    onClick={() => onDecide(request.requestId, "approve")}
                    className="rounded-lg bg-cyan-600 px-4 py-2 text-sm font-semibold text-white hover:bg-cyan-500 disabled:opacity-50"
                  >
                    Approve
                  </button>
                  <button
                    type="button"
                    onClick={() => onDecide(request.requestId, "reject")}
                    className="rounded-lg border border-rose-500/70 px-4 py-2 text-sm font-semibold text-rose-100 hover:bg-rose-950/40 disabled:opacity-50"
                  >
                    Reject
                  </button>
                </fieldset>
                {operation ? (
                  <p className="mt-3 text-sm text-cyan-200">
                    {operation.kind === "approve"
                      ? "Waiting for approval delivery confirmation…"
                      : "Waiting for rejection delivery confirmation…"}
                  </p>
                ) : null}
                {failure ? <ErrorAlert error={failure} /> : null}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
