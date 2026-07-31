from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    source = target.read_text()
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"UI patch anchor count {count}: {path}: {old[:100]!r}")
    target.write_text(source.replace(old, new))


screen = "desktop/src/screens/HostSetupScreen.tsx"
replace_once(
    screen,
    'import type { UpdateHostDraftRequest } from "../core/generated/desktop-bindings";\n',
    'import type { UpdateHostDraftRequest } from "../core/generated/desktop-bindings";\nimport { HostNetworkPolicyCard } from "./HostNetworkPolicyCard";\n',
)
replace_once(
    screen,
    "  const [selectingSource, setSelectingSource] = useState(false);\n",
    "  const [selectingSource, setSelectingSource] = useState(false);\n  const [networkReady, setNetworkReady] = useState(false);\n",
)
replace_once(
    screen,
    "  const canCreate = canSubmit && !dirty && snapshot.canCreateHostSession;\n",
    "  const canCreate = canSubmit && !dirty && snapshot.canCreateHostSession && networkReady;\n",
)
replace_once(
    screen,
    '''          <SummaryCard title="Network interface policy">
            <p>
              {snapshot.capabilities.localNetworkAvailable
                ? "Automatic private-LAN selection; explicit interface policy arrives in Block 21."
                : "Local-network capability has not been confirmed by the platform runner."}
            </p>
          </SummaryCard>
''',
    '''          <HostNetworkPolicyCard
            available={snapshot.capabilities.localNetworkAvailable}
            disabled={!lifecycleAllowsSetup || pending}
            onReadinessChange={setNetworkReady}
          />
''',
)

test = "desktop/src/screens/HostSetupScreen.test.tsx"
replace_once(
    test,
    '''const { selectAudioSource, selectHostRole, updateHostDraft, createHostSession } = vi.hoisted(
  () => ({
    selectAudioSource: vi.fn(),
    selectHostRole: vi.fn(),
    updateHostDraft: vi.fn(),
    createHostSession: vi.fn(),
  }),
);
''',
    '''const {
  selectAudioSource,
  selectHostRole,
  updateHostDraft,
  createHostSession,
  getHostNetworkState,
  setHostNetworkPreference,
} = vi.hoisted(() => ({
  selectAudioSource: vi.fn(),
  selectHostRole: vi.fn(),
  updateHostDraft: vi.fn(),
  createHostSession: vi.fn(),
  getHostNetworkState: vi.fn(),
  setHostNetworkPreference: vi.fn(),
}));
''',
)
replace_once(
    test,
    "  return { ...actual, selectHostRole, updateHostDraft, createHostSession };\n",
    '''  return {
    ...actual,
    selectHostRole,
    updateHostDraft,
    createHostSession,
    getHostNetworkState,
    setHostNetworkPreference,
  };
''',
)
insert_anchor = "function renderScreen(initial = snapshot()) {\n"
network_helper = '''function networkSnapshot() {
  const selected = {
    interfaceName: "enp1s0",
    interfaceIndex: 2,
    address: "192.168.1.20",
    prefixLength: 24,
    classification: "private_lan" as const,
    isDefaultRoute: true,
    isActive: true,
    isPhysical: true,
    selectable: true,
    rejectionReason: null,
  };
  return {
    preference: { mode: "automatic", interfaceName: null, address: null },
    candidates: [selected],
    automaticSelection: selected,
    resolvedSelection: selected,
    requiresExplicitSelection: false,
    selectionError: null,
    activeBinding: null,
    activeBindingValid: false,
    interfaceChange: null,
  };
}

'''
replace_once(test, insert_anchor, network_helper + insert_anchor)
replace_once(
    test,
    '''  createHostSession.mockResolvedValue({ operationId: "create-1", acceptedAtRevision: "4" });
});
''',
    '''  createHostSession.mockResolvedValue({ operationId: "create-1", acceptedAtRevision: "4" });
  getHostNetworkState.mockResolvedValue(networkSnapshot());
  setHostNetworkPreference.mockResolvedValue(networkSnapshot());
});
''',
)
replace_once(
    test,
    '''    const store = renderScreen();
    fireEvent.click(screen.getByRole("button", { name: "Create session" }));
    await waitFor(() => expect(createHostSession).toHaveBeenCalledWith("4"));
''',
    '''    const store = renderScreen();
    const create = screen.getByRole("button", { name: "Create session" });
    await waitFor(() => expect(create).toBeEnabled());
    fireEvent.click(create);
    await waitFor(() => expect(createHostSession).toHaveBeenCalledWith("4"));
''',
)
replace_once(
    test,
    '''    const store = renderScreen();
    fireEvent.click(screen.getByRole("button", { name: "Create session" }));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("stale revision"));
''',
    '''    const store = renderScreen();
    const create = screen.getByRole("button", { name: "Create session" });
    await waitFor(() => expect(create).toBeEnabled());
    fireEvent.click(create);
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("stale revision"));
''',
)
