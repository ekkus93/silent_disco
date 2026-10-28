import { useEffect, useMemo, useRef, useState } from "react";

import { coreActions } from "../app/coreSlice";
import {
  selectCoreSnapshot,
  selectLatestCoreError,
  selectPendingCommandReceipts,
} from "../app/selectors";
import { useAppDispatch, useAppSelector } from "../app/store";
import { uiActions } from "../app/uiSlice";
import {
  addHostAudioSources,
  createHostSession,
  removeHostQueueEntry,
  reorderHostQueue,
  selectHostRole,
  toDesktopBridgeError,
  updateHostDraft,
} from "../core/client";
import type { UpdateHostDraftRequest } from "../core/generated/desktop-bindings";
import { HostNetworkReadinessSummary } from "./HostNetworkReadinessSummary";
import { ErrorAlert } from "../components/ErrorAlert";

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
  const [networkReady, setNetworkReady] = useState(false);
  const [showAdvancedTuning, setShowAdvancedTuning] = useState(false);
  const [sessionNameTouched, setSessionNameTouched] = useState(false);
  const [setupStage, setSetupStage] = useState<"music" | "access" | "ready">("music");
  const formRef = useRef<HTMLFormElement | null>(null);
  const advancedHeadingRef = useRef<HTMLHeadingElement | null>(null);
  const focusedViewRef = useRef<string | null>(null);
  const roleSelectionRequest = useRef<{
    revision: string;
    request: ReturnType<typeof selectHostRole>;
  } | null>(null);

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
    const existing = roleSelectionRequest.current;
    const request =
      existing?.revision === snapshot.revision
        ? existing.request
        : selectHostRole(snapshot.revision);
    roleSelectionRequest.current = { revision: snapshot.revision, request };
    let active = true;
    request
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
        if (roleSelectionRequest.current?.request === request) {
          roleSelectionRequest.current = null;
        }
      });
    return () => {
      active = false;
    };
  }, [dispatch, snapshot]);

  // A step change unmounts the control that triggered it (Enter in a field,
  // or a step button), which used to drop keyboard focus onto the page body.
  // Focus moves to the new step's first control instead; the initial render
  // leaves focus alone.
  const focusedView = showAdvancedTuning ? "advanced" : setupStage;
  const setupRendered = snapshot !== null && draft !== null;
  useEffect(() => {
    if (!setupRendered) return;
    const previous = focusedViewRef.current;
    focusedViewRef.current = focusedView;
    if (previous === null || previous === focusedView) return;
    if (focusedView === "advanced") {
      advancedHeadingRef.current?.focus();
    } else {
      formRef.current?.querySelector<HTMLElement>("[data-step-focus]")?.focus();
    }
  }, [focusedView, setupRendered]);

  const pending = pendingCommands.length > 0;
  const validationByField = useMemo(() => {
    const map = new Map<string, string>();
    for (const validation of snapshot?.hostDraftValidation ?? []) {
      map.set(validation.field, validation.message);
    }
    return map;
  }, [snapshot]);
  const sessionNameError = sessionNameTouched ? validationByField.get("sessionName") : undefined;

  if (snapshot === null || draft === null) {
    return <p role="status">Loading host setup…</p>;
  }

  const lifecycleAllowsSetup =
    snapshot.hostLifecycle === "idle" || snapshot.hostLifecycle === "error";
  const canSubmit = snapshot.selectedRole === "host" && lifecycleAllowsSetup && !pending;
  // Legacy v3 cannot verify public-key identities. Never offer or silently
  // persist "always allow" semantics on an unauthenticated control socket.
  const unverifiedTrust = draft.approvalMode === "trusted_devices" || draft.rememberApprovedDevices;
  const canCreate =
    canSubmit && !dirty && !unverifiedTrust && snapshot.canCreateHostSession && networkReady;
  const createPending = pendingCommands.some(
    (command) => command.commandKind === "create_host_session",
  );
  const sourcePending = pendingCommands.some(
    (command) => command.commandKind === "append_host_queue",
  );

  const submitDraft = async () => {
    if (unverifiedTrust) return false;
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
      return true;
    } catch (error: unknown) {
      dispatch(
        coreActions.commandInvocationFailed(toDesktopBridgeError(error, "update host draft")),
      );
      return false;
    }
  };

  const continueSetup = async () => {
    // The Music step must still let an old profile reach Access to repair
    // its unsupported trust settings; only leaving Access is blocked.
    if (pending || (setupStage === "access" && unverifiedTrust)) return;
    if (dirty && !(await submitDraft())) return;
    setSetupStage((stage) => (stage === "music" ? "access" : "ready"));
  };

  const chooseSource = async () => {
    setSelectingSource(true);
    try {
      const receipt = await addHostAudioSources();
      if (receipt !== null) {
        dispatch(
          coreActions.commandPending({
            operationId: receipt.operationId,
            commandKind: "append_host_queue",
            submittedAtRevision: receipt.acceptedAtRevision,
          }),
        );
      }
    } catch (error: unknown) {
      dispatch(coreActions.commandInvocationFailed(toDesktopBridgeError(error, "add songs")));
    } finally {
      setSelectingSource(false);
    }
  };

  const createSession = async () => {
    if (unverifiedTrust) return;
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
          <h2
            id="advanced-tuning-title"
            ref={advancedHeadingRef}
            tabIndex={-1}
            className="mt-2 text-2xl font-bold outline-none"
          >
            Advanced tuning
          </h2>
          <p className="mt-3 text-violet-100/75">Audio tuning is unavailable in this version.</p>
        </div>
      </section>
    );
  }

  const setupStages = [
    { id: "music", label: "Music" },
    { id: "access", label: "Access" },
    { id: "ready", label: "Ready" },
  ] as const;

  return (
    <section aria-labelledby="host-setup-title" className="space-y-7">
      <div>
        <p className="text-sm font-semibold uppercase tracking-[0.2em] text-cyan-300">Host setup</p>
        <h2 id="host-setup-title" className="mt-2 text-3xl font-bold">
          Create a silent disco
        </h2>
        <p className="mt-3 max-w-2xl text-violet-100/75">
          Choose a playlist, set who can join, then review the session before you start.
        </p>
      </div>

      <nav aria-label="Host setup steps" className="flex flex-wrap gap-3">
        {setupStages.map((stage, index) => (
          <span
            key={stage.id}
            aria-current={setupStage === stage.id ? "step" : undefined}
            className={`rounded-full border px-4 py-2 text-sm ${
              setupStage === stage.id
                ? "border-cyan-300 bg-cyan-300/10 text-cyan-100"
                : "border-violet-200/15 text-violet-100/65"
            }`}
          >
            {index + 1}. {stage.label}
          </span>
        ))}
      </nav>

      <div className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_18rem]">
        <form
          ref={formRef}
          className="sd-workflow-card space-y-5 p-5"
          onSubmit={(event) => {
            event.preventDefault();
            if (setupStage !== "ready") void continueSetup();
          }}
        >
          {setupStage === "music" ? (
            <div>
              <label htmlFor="session-name" className="block text-sm font-semibold">
                Session name
              </label>
              <input
                id="session-name"
                data-step-focus
                value={draft.sessionName}
                onChange={(event) => {
                  setDraft({ ...draft, sessionName: event.target.value });
                  setDirty(true);
                }}
                onBlur={() => setSessionNameTouched(true)}
                aria-describedby={sessionNameError ? "session-name-error" : undefined}
                className="mt-2 w-full rounded-xl border border-violet-200/25 bg-slate-950 px-3 py-2"
              />
              {sessionNameError ? (
                <p id="session-name-error" className="mt-2 text-sm text-amber-200">
                  {sessionNameError}
                </p>
              ) : null}
            </div>
          ) : null}

          {setupStage === "access" ? (
            <div>
              <label htmlFor="approval-mode" className="block text-sm font-semibold">
                Listener approval
              </label>
              <select
                id="approval-mode"
                data-step-focus
                value={draft.approvalMode}
                onChange={(event) => {
                  setDraft({ ...draft, approvalMode: event.target.value as ApprovalMode });
                  setDirty(true);
                }}
                className="mt-2 w-full rounded-xl border border-violet-200/25 bg-slate-950 px-3 py-2"
              >
                <option value="manual">Approve each listener</option>
                <option value="trusted_devices" disabled>
                  Approved devices only (unavailable until secure transport)
                </option>
                <option value="invite_code">Require an invite code</option>
              </select>
              <p id="approval-mode-help" className="mt-2 text-sm text-violet-100/70">
                Approved-device auto-admission requires verified identities and is unavailable on
                this plaintext LAN transport.
              </p>
            </div>
          ) : null}

          {setupStage === "access" && draft.approvalMode === "invite_code" ? (
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

          {setupStage === "access" ? (
            <label className="flex items-start gap-3 rounded-xl border border-violet-200/15 p-3">
              <input
                aria-describedby="remember-approved-devices-help"
                aria-label="Remember approved devices"
                disabled={!draft.rememberApprovedDevices}
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
                  Unavailable until encrypted transport verifies device identity. If an old profile
                  enabled this, uncheck it before continuing.
                </span>
              </span>
            </label>
          ) : null}

          {unverifiedTrust && setupStage !== "music" ? (
            <p role="alert" className="rounded-lg border border-amber-300/40 p-3 text-amber-200">
              This profile requests unverified device trust. Select manual or invite-code approval
              and turn off remembered devices before continuing.
            </p>
          ) : null}

          {setupStage === "ready" ? (
            <section aria-label="Session review" className="space-y-3">
              <h3 data-step-focus tabIndex={-1} className="text-lg font-semibold outline-none">
                Ready to host
              </h3>
              <p>{snapshot.hostDraft.sessionName}</p>
              <p>{snapshot.hostQueue.entries.length} songs queued</p>
              <p>
                {draft.approvalMode === "manual"
                  ? "Approve each listener"
                  : draft.approvalMode === "trusted_devices"
                    ? "Trusted devices"
                    : "Invite code required"}
              </p>
              {draft.approvalMode === "invite_code" ? (
                <p>Invite code: {draft.inviteCode || "Required"}</p>
              ) : null}
              {dirty ? <p role="status">Saving the latest settings…</p> : null}
            </section>
          ) : null}

          <div className="flex flex-wrap gap-3">
            {setupStage === "music" ? (
              <>
                <button
                  type="button"
                  onClick={() => void chooseSource()}
                  disabled={!canSubmit || selectingSource}
                  className="sd-workflow-control rounded-sd-control border border-sd-outline px-4 py-2 font-semibold text-sd-on-surface disabled:cursor-not-allowed disabled:opacity-40"
                >
                  {selectingSource ? "Opening audio picker…" : "Add songs"}
                </button>
                <button
                  type="submit"
                  disabled={
                    !canSubmit ||
                    snapshot.hostQueue.entries.length === 0 ||
                    !draft.sessionName.trim()
                  }
                  className="sd-workflow-control rounded-sd-control bg-sd-secondary-container px-4 py-2 font-semibold text-sd-on-secondary-container disabled:cursor-not-allowed disabled:opacity-40"
                >
                  Next: Access
                </button>
              </>
            ) : null}
            {setupStage === "access" ? (
              <>
                <button
                  type="button"
                  onClick={() => setSetupStage("music")}
                  className="sd-workflow-control rounded-sd-control border border-sd-outline px-4 py-2 font-semibold text-sd-on-surface"
                >
                  Back to music
                </button>
                <button
                  type="submit"
                  disabled={!canSubmit || unverifiedTrust}
                  className="sd-workflow-control rounded-sd-control bg-sd-secondary-container px-4 py-2 font-semibold text-sd-on-secondary-container disabled:opacity-40"
                >
                  Review session
                </button>
              </>
            ) : null}
            {setupStage === "ready" ? (
              <>
                <button
                  type="button"
                  onClick={() => setSetupStage("access")}
                  className="sd-workflow-control rounded-sd-control border border-sd-outline px-4 py-2 font-semibold text-sd-on-surface"
                >
                  Back to access
                </button>
                <button
                  type="button"
                  onClick={() => void createSession()}
                  disabled={!canCreate}
                  className="sd-workflow-control rounded-sd-control bg-sd-primary px-4 py-2 font-semibold text-sd-on-primary disabled:cursor-not-allowed disabled:opacity-40"
                >
                  {createPending ? "Starting…" : "Start hosting"}
                </button>
              </>
            ) : null}
            <button
              type="button"
              onClick={() => setShowAdvancedTuning(true)}
              className="sd-workflow-control rounded-sd-control border border-sd-outline px-4 py-2 font-semibold text-sd-on-surface"
            >
              Advanced tuning
            </button>
          </div>
        </form>

        <aside className="space-y-4" aria-label="Host setup summary">
          <SummaryCard title={`Playlist (${snapshot.hostQueue.entries.length})`}>
            {snapshot.hostQueue.entries.length === 0 ? (
              <p>No songs added. Add WAV, FLAC, or MP3 files to build the playlist.</p>
            ) : (
              <ol className="space-y-3">
                {snapshot.hostQueue.entries.map((entry, index) => (
                  <li key={entry.entryId} className="rounded-lg border border-violet-200/15 p-3">
                    <p className="font-medium">
                      {index + 1}. {entry.source.displayName}
                    </p>
                    <div className="mt-2 flex flex-wrap gap-2">
                      <button
                        type="button"
                        aria-label={`Move ${entry.source.displayName} up`}
                        disabled={pending || index === 0}
                        onClick={() => {
                          const ids = snapshot.hostQueue.entries.map((item) => item.entryId);
                          const before = ids[index - 1];
                          const current = ids[index];
                          if (before && current) {
                            ids[index - 1] = current;
                            ids[index] = before;
                          }
                          void reorderHostQueue({
                            expectedRevision: snapshot.revision,
                            expectedQueueRevision: snapshot.hostQueue.revision,
                            orderedEntryIds: ids,
                          }).catch((error: unknown) =>
                            dispatch(
                              coreActions.commandInvocationFailed(
                                toDesktopBridgeError(error, "reorder playlist"),
                              ),
                            ),
                          );
                        }}
                        className="rounded border border-violet-200/20 px-2 py-1 text-sm disabled:opacity-40"
                      >
                        Move up
                      </button>
                      <button
                        type="button"
                        aria-label={`Move ${entry.source.displayName} down`}
                        disabled={pending || index === snapshot.hostQueue.entries.length - 1}
                        onClick={() => {
                          const ids = snapshot.hostQueue.entries.map((item) => item.entryId);
                          const current = ids[index];
                          const after = ids[index + 1];
                          if (current && after) {
                            ids[index] = after;
                            ids[index + 1] = current;
                          }
                          void reorderHostQueue({
                            expectedRevision: snapshot.revision,
                            expectedQueueRevision: snapshot.hostQueue.revision,
                            orderedEntryIds: ids,
                          }).catch((error: unknown) =>
                            dispatch(
                              coreActions.commandInvocationFailed(
                                toDesktopBridgeError(error, "reorder playlist"),
                              ),
                            ),
                          );
                        }}
                        className="rounded border border-violet-200/20 px-2 py-1 text-sm disabled:opacity-40"
                      >
                        Move down
                      </button>
                      <button
                        type="button"
                        aria-label={`Remove ${entry.source.displayName}`}
                        disabled={pending}
                        onClick={() =>
                          void removeHostQueueEntry({
                            expectedRevision: snapshot.revision,
                            expectedQueueRevision: snapshot.hostQueue.revision,
                            entryId: entry.entryId,
                          }).catch((error: unknown) =>
                            dispatch(
                              coreActions.commandInvocationFailed(
                                toDesktopBridgeError(error, "remove playlist item"),
                              ),
                            ),
                          )
                        }
                        className="rounded border border-violet-200/20 px-2 py-1 text-sm disabled:opacity-40"
                      >
                        Remove
                      </button>
                    </div>
                  </li>
                ))}
              </ol>
            )}
          </SummaryCard>

          <HostNetworkReadinessSummary
            available={snapshot.capabilities.localNetworkAvailable}
            onReadinessChange={setNetworkReady}
            onConfigure={() => dispatch(uiActions.activeViewChanged("settings"))}
          />

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
            ? "Choose audio files…"
            : createPending
              ? "Starting your session…"
              : sourcePending
                ? "Adding songs…"
                : "Saving host settings…"}
        </p>
      ) : null}

      {latestError !== null ? (
        <ErrorAlert
          title="Host setup command failed"
          error={latestError}
          observedAtMs={latestError.observedAtMs}
          onDismiss={() => dispatch(coreActions.errorDismissed({ id: latestError.id }))}
        />
      ) : null}

      {!lifecycleAllowsSetup ? (
        <p role="status" className="rounded-xl border border-violet-300/25 p-3">
          Finish the active host session before changing these settings.
        </p>
      ) : null}
    </section>
  );
}

function SummaryCard({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="sd-workflow-card p-4">
      <h3 className="font-semibold text-cyan-200">{title}</h3>
      <div className="mt-2 text-sm leading-6 text-violet-100/70">{children}</div>
    </section>
  );
}
