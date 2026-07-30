import { useEffect, useMemo, useState } from "react";

import { coreActions } from "../app/coreSlice";
import {
  selectCoreSnapshot,
  selectLatestCoreError,
  selectPendingCommandReceipts,
} from "../app/selectors";
import { useAppDispatch, useAppSelector } from "../app/store";
import { selectAudioSource } from "../core/audioSourceClient";
import {
  createHostSession,
  selectHostRole,
  toDesktopBridgeError,
  updateHostDraft,
} from "../core/client";
import type { UpdateHostDraftRequest } from "../core/generated/desktop-bindings";

type ApprovalMode = "manual" | "trusted_devices" | "invite_code";

interface EditableDraft {
  sessionName: string;
  approvalMode: ApprovalMode;
  inviteCode: string;
  rememberApprovedDevices: boolean;
}

function editableFromSnapshot(
  snapshot: NonNullable<ReturnType<typeof selectCoreSnapshot>>,
): EditableDraft {
  return {
    sessionName: snapshot.hostDraft.sessionName,
    approvalMode: snapshot.hostDraft.approvalMode as ApprovalMode,
    inviteCode: snapshot.hostDraft.inviteCode ?? "",
    rememberApprovedDevices: snapshot.hostDraft.rememberApprovedDevices,
  };
}

function draftMatchesSnapshot(
  draft: EditableDraft,
  snapshot: NonNullable<ReturnType<typeof selectCoreSnapshot>>,
): boolean {
  return (
    draft.sessionName === snapshot.hostDraft.sessionName &&
    draft.approvalMode === snapshot.hostDraft.approvalMode &&
    (draft.approvalMode === "invite_code" ? draft.inviteCode : "") ===
      (snapshot.hostDraft.inviteCode ?? "") &&
    draft.rememberApprovedDevices === snapshot.hostDraft.rememberApprovedDevices
  );
}

export function HostSetupScreen() {
  const dispatch = useAppDispatch();
  const snapshot = useAppSelector(selectCoreSnapshot);
  const pendingCommands = useAppSelector(selectPendingCommandReceipts);
  const latestError = useAppSelector(selectLatestCoreError);
  const [draft, setDraft] = useState<EditableDraft | null>(null);
  const [draftRevision, setDraftRevision] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);
  const [selectingSource, setSelectingSource] = useState(false);
  const [showAdvancedTuning, setShowAdvancedTuning] = useState(false);

  useEffect(() => {
    if (snapshot === null || snapshot.revision === draftRevision) return;
    if (draft === null || !dirty || draftMatchesSnapshot(draft, snapshot)) {
      setDraft(editableFromSnapshot(snapshot));
      setDirty(false);
    }
    setDraftRevision(snapshot.revision);
  }, [dirty, draft, draftRevision, snapshot]);

  useEffect(() => {
    if (snapshot === null || snapshot.selectedRole !== null) return;
    let active = true;
    selectHostRole(snapshot.revision)
      .then((receipt) => {
        if (active) {
          dispatch(
            coreActions.commandPending({
              operationId: receipt.operationId,
              commandKind: "select_host_role",
              submittedAtRevision: receipt.acceptedAtRevision,
            }),
          );
        }
      })
      .catch((error: unknown) => {
        if (active) {
          dispatch(
            coreActions.commandInvocationFailed(toDesktopBridgeError(error, "select host role")),
          );
        }
      });
    return () => {
      active = false;
    };
  }, [dispatch, snapshot]);

  const pending = pendingCommands.length > 0;
  const validationByField = useMemo(() => {
    const map = new Map<string, string>();
    for (const validation of snapshot?.hostDraftValidation ?? []) {
      map.set(validation.field, validation.message);
    }
    return map;
  }, [snapshot]);

  if (snapshot === null || draft === null) {
    return <p role="status">Waiting for the authoritative Rust host draft…</p>;
  }

  const lifecycleAllowsSetup =
    snapshot.hostLifecycle === "idle" || snapshot.hostLifecycle === "error";
  const canSubmit = snapshot.selectedRole === "host" && lifecycleAllowsSetup && !pending;
  const canCreate = canSubmit && !dirty && snapshot.canCreateHostSession;
  const createPending = pendingCommands.some(
    (command) => command.commandKind === "create_host_session",
  );
  const sourcePending = pendingCommands.some(
    (command) => command.commandKind === "select_audio_source",
  );

  const submitDraft = async () => {
    const request: UpdateHostDraftRequest = {
      expectedRevision: snapshot.revision,
      sessionName: draft.sessionName,
      approvalMode: draft.approvalMode,
      inviteCode: draft.approvalMode === "invite_code" ? draft.inviteCode : null,
      rememberApprovedDevices: draft.rememberApprovedDevices,
    };
    try {
      const receipt = await updateHostDraft(request);
      dispatch(
        coreActions.commandPending({
          operationId: receipt.operationId,
          commandKind: "update_host_draft",
          submittedAtRevision: receipt.acceptedAtRevision,
        }),
      );
    } catch (error: unknown) {
      dispatch(
        coreActions.commandInvocationFailed(toDesktopBridgeError(error, "update host draft")),
      );
    }
  };

  const chooseSource = async () => {
    setSelectingSource(true);
    try {
      const receipt = await selectAudioSource(snapshot.revision);
      if (receipt !== null) {
        dispatch(
          coreActions.commandPending({
            operationId: receipt.operationId,
            commandKind: "select_audio_source",
            submittedAtRevision: receipt.acceptedAtRevision,
          }),
        );
      }
    } catch (error: unknown) {
      dispatch(
        coreActions.commandInvocationFailed(toDesktopBridgeError(error, "select audio source")),
      );
    } finally {
      setSelectingSource(false);
    }
  };

  const createSession = async () => {
    try {
      const receipt = await createHostSession(snapshot.revision);
      dispatch(
        coreActions.commandPending({
          operationId: receipt.operationId,
          commandKind: "create_host_session",
          submittedAtRevision: receipt.acceptedAtRevision,
        }),
      );
    } catch (error: unknown) {
      dispatch(
        coreActions.commandInvocationFailed(toDesktopBridgeError(error, "create host session")),
      );
    }
  };

  if (showAdvancedTuning) {
    return (
      <section aria-labelledby="advanced-tuning-title" className="space-y-5">
        <button
          type="button"
          onClick={() => setShowAdvancedTuning(false)}
          className="rounded-lg border border-violet-300/30 px-3 py-2 text-sm"
        >
          Back to host setup
        </button>
        <div>
          <p className="text-sm font-semibold uppercase tracking-[0.2em] text-cyan-300">
            Shared settings
          </p>
          <h2 id="advanced-tuning-title" className="mt-2 text-2xl font-bold">
            Advanced tuning
          </h2>
          <p className="mt-3 text-violet-100/75">
            Rust owns the active tuning values. Editing remains disabled until the dedicated typed
            tuning command surface is connected.
          </p>
        </div>
      </section>
    );
  }

  return (
    <section aria-labelledby="host-setup-title" className="space-y-7">
      <div>
        <p className="text-sm font-semibold uppercase tracking-[0.2em] text-cyan-300">
          Rust-authoritative host setup
        </p>
        <h2 id="host-setup-title" className="mt-2 text-3xl font-bold">
          Create a silent disco
        </h2>
        <p className="mt-3 max-w-2xl text-violet-100/75">
          Edit locally, then submit typed commands. The screen does not claim success until a newer
          Rust snapshot arrives.
        </p>
      </div>

      <div className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_18rem]">
        <form
          className="space-y-5 rounded-2xl border border-violet-200/15 bg-black/25 p-5"
          onSubmit={(event) => {
            event.preventDefault();
            void submitDraft();
          }}
        >
          <div>
            <label htmlFor="session-name" className="block text-sm font-semibold">
              Session name
            </label>
            <input
              id="session-name"
              value={draft.sessionName}
              onChange={(event) => {
                setDraft({ ...draft, sessionName: event.target.value });
                setDirty(true);
              }}
              aria-describedby={
                validationByField.has("sessionName") ? "session-name-error" : undefined
              }
              className="mt-2 w-full rounded-xl border border-violet-200/25 bg-slate-950 px-3 py-2"
            />
            {validationByField.has("sessionName") ? (
              <p id="session-name-error" className="mt-2 text-sm text-amber-200">
                {validationByField.get("sessionName")}
              </p>
            ) : null}
          </div>

          <div>
            <label htmlFor="approval-mode" className="block text-sm font-semibold">
              Listener approval
            </label>
            <select
              id="approval-mode"
              value={draft.approvalMode}
              onChange={(event) => {
                setDraft({ ...draft, approvalMode: event.target.value as ApprovalMode });
                setDirty(true);
              }}
              className="mt-2 w-full rounded-xl border border-violet-200/25 bg-slate-950 px-3 py-2"
            >
              <option value="manual">Approve each listener</option>
              <option value="trusted_devices">Automatically approve trusted devices</option>
              <option value="invite_code">Require an invite code</option>
            </select>
          </div>

          {draft.approvalMode === "invite_code" ? (
            <div>
              <label htmlFor="invite-code" className="block text-sm font-semibold">
                Invite code
              </label>
              <input
                id="invite-code"
                value={draft.inviteCode}
                onChange={(event) => {
                  setDraft({ ...draft, inviteCode: event.target.value });
                  setDirty(true);
                }}
                aria-describedby={
                  validationByField.has("inviteCode") ? "invite-code-error" : undefined
                }
                className="mt-2 w-full rounded-xl border border-violet-200/25 bg-slate-950 px-3 py-2"
              />
              {validationByField.has("inviteCode") ? (
                <p id="invite-code-error" className="mt-2 text-sm text-amber-200">
                  {validationByField.get("inviteCode")}
                </p>
              ) : null}
            </div>
          ) : null}

          <label className="flex items-start gap-3 rounded-xl border border-violet-200/15 p-3">
            <input
              aria-describedby="remember-approved-devices-help"
              aria-label="Remember approved devices"
              type="checkbox"
              checked={draft.rememberApprovedDevices}
              onChange={(event) => {
                setDraft({ ...draft, rememberApprovedDevices: event.target.checked });
                setDirty(true);
              }}
              className="mt-1"
            />
            <span>
              <span className="block font-semibold">Remember approved devices</span>
              <span
                className="mt-1 block text-sm text-violet-100/65"
                id="remember-approved-devices-help"
              >
                Rust applies the trusted-device policy; this checkbox does not persist trust itself.
              </span>
            </span>
          </label>

          <div className="flex flex-wrap gap-3">
            <button
              type="button"
              onClick={() => void chooseSource()}
              disabled={!canSubmit || selectingSource}
              className="rounded-xl border border-cyan-300/40 px-4 py-2 font-semibold disabled:cursor-not-allowed disabled:opacity-40"
            >
              {selectingSource ? "Opening audio picker…" : "Select audio file"}
            </button>
            <button
              type="submit"
              disabled={!canSubmit || !dirty}
              className="rounded-xl bg-cyan-300 px-4 py-2 font-semibold text-slate-950 disabled:cursor-not-allowed disabled:opacity-40"
            >
              Validate settings
            </button>
            <button
              type="button"
              onClick={() => void createSession()}
              disabled={!canCreate}
              className="rounded-xl bg-violet-300 px-4 py-2 font-semibold text-slate-950 disabled:cursor-not-allowed disabled:opacity-40"
            >
              Create session
            </button>
            <button
              type="button"
              onClick={() => setShowAdvancedTuning(true)}
              className="rounded-xl border border-violet-300/30 px-4 py-2 font-semibold"
            >
              Advanced tuning
            </button>
          </div>
        </form>

        <aside className="space-y-4" aria-label="Host setup summary">
          <SummaryCard title="Selected audio source">
            {snapshot.hostDraft.audioSource === null ? (
              <>
                <p>No inspected source selected.</p>
                <p className="mt-2 text-sm text-violet-100/65">
                  Select one WAV, FLAC, or MP3 file. Rust validates the regular-file status, size,
                  bounded name, canonical identity, and content signature before registration.
                </p>
              </>
            ) : (
              <>
                <p>{snapshot.hostDraft.audioSource.displayName}</p>
                {snapshot.hostDraft.audioSource.byteLength !== null ? (
                  <p className="mt-1 font-mono text-xs text-violet-100/55">
                    {snapshot.hostDraft.audioSource.byteLength} bytes
                  </p>
                ) : null}
              </>
            )}
            {validationByField.has("audioSource") ? (
              <p className="mt-2 text-sm text-amber-200">{validationByField.get("audioSource")}</p>
            ) : null}
          </SummaryCard>

          <SummaryCard title="Network interface policy">
            <p>
              {snapshot.capabilities.localNetworkAvailable
                ? "Automatic private-LAN selection; explicit interface policy arrives in Block 21."
                : "Local-network capability has not been confirmed by the platform runner."}
            </p>
          </SummaryCard>

          {snapshot.capabilities.audioOutputAvailable ? (
            <SummaryCard title="Local monitor">
              <label className="flex items-center gap-2 text-sm text-violet-100/65">
                <input type="checkbox" disabled />
                Enable local monitor after the audio-output adapter is connected
              </label>
            </SummaryCard>
          ) : null}
        </aside>
      </div>

      {pending || selectingSource ? (
        <p role="status" aria-live="polite" className="rounded-xl border border-cyan-300/25 p-3">
          {selectingSource
            ? "Waiting for the native audio file dialog…"
            : createPending
              ? "Create request accepted by the queue. Waiting for a newer Rust snapshot…"
              : sourcePending
                ? "Source registration accepted by the queue. Waiting for a newer Rust snapshot…"
                : "Host settings are pending authoritative Rust evidence…"}
        </p>
      ) : null}

      {latestError !== null ? (
        <div role="alert" className="rounded-xl border border-red-300/30 bg-red-950/35 p-4">
          <p className="font-semibold">Host setup command failed</p>
          <p className="mt-1 text-sm text-red-100/80">{latestError.message}</p>
        </div>
      ) : null}

      {!lifecycleAllowsSetup ? (
        <p role="status" className="rounded-xl border border-violet-300/25 p-3">
          Rust host lifecycle: {snapshot.hostLifecycle}. This screen has not locally advanced it.
        </p>
      ) : null}
    </section>
  );
}

function SummaryCard({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="rounded-2xl border border-violet-200/15 bg-black/25 p-4">
      <h3 className="font-semibold text-cyan-200">{title}</h3>
      <div className="mt-2 text-sm leading-6 text-violet-100/70">{children}</div>
    </section>
  );
}
