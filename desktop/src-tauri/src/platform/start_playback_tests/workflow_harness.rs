//! Full desktop host workflow harness: a real `DesktopAppState` profile
//! (profile lock, database worker, core actor, platform/storage effect runners,
//! notification bridge) whose host network is the production socket
//! transport bound to this machine's real LAN address -- only mDNS and the
//! local monitor device are left out. Sources go through the production
//! dialog-inspection, staging, decode-validation, registry, and queue command
//! bodies in `host_workflow_ops`, exactly as the Tauri commands drive them.

use super::harness::{FixedInterfaceProvider, TEST_TIMEOUT, wait_for_control};
use crate::app_state::DesktopAppState;
use crate::dto::DesktopErrorDto;
use crate::host_workflow_ops;
use crate::notification_buffer::{
    DesktopNotificationBuffer, DesktopNotificationSendError, DesktopNotificationSink,
};
use crate::platform::file_picker::{
    AudioFileDialog, InspectedAudioSource, SelectedSourceRegistry, SystemAudioFileBoundary,
    select_and_inspect_files,
};
use crate::platform::identity::{DesktopIdentity, DesktopIdentityError, DesktopIdentityProvider};
use crate::platform::invitation_identity::{
    DesktopHostSigningIdentity, DesktopHostSigningIdentityError, DesktopHostSigningIdentityProvider,
};
use crate::platform::network::{DesktopHostNetworkControl, TestHostPorts};
use crate::platform::paths::DesktopProfilePaths;
use crate::platform::source_staging_control::{
    SourceStagingControl, SourceStagingProgressDto, SourceStagingProgressSink,
};
use crate::profile::ProfileId;
use crate::runtime_dto::CommandReceiptDto;
use silent_disco_core::domain::{AppRole, ApprovalMode, DeviceId, HostLifecycle};
use silent_disco_core::protocol::{ControlMessage, DeviceIdentity, JoinRequest};
use silent_disco_core::runtime::{
    AudioSourcePatch, CoreActorHandle, CoreCommand, CoreNotification, CoreSnapshot, HostDraftPatch,
    InviteCodePatch, NetworkEndpoint,
};
use silent_disco_core::transport::{
    ListenerTransportConfig, ListenerTransportNode, SystemTransportClock, TransportFactory,
    production_transport_factory,
};
use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;

pub(super) struct HostWorkflow {
    pub(super) state: DesktopAppState,
    pub(super) registry: SelectedSourceRegistry,
    pub(super) network: Arc<DesktopHostNetworkControl>,
    pub(super) handle: CoreActorHandle,
    pub(super) notifications: Arc<RecordingNotificationSink>,
    pub(super) address: Ipv4Addr,
    inputs: TempDir,
    _profile_root: TempDir,
}

impl HostWorkflow {
    /// Opens a production profile and selects the host role with a manual-
    /// approval draft. Returns `None` when this machine has no LAN address the
    /// production bind policy would accept (the test then skips, like the rest
    /// of this suite).
    pub(super) fn open() -> Option<Self> {
        let (interface_name, interface_index, address) =
            super::harness::real_private_lan_address()?;
        let network = Arc::new(DesktopHostNetworkControl::with_components(
            Arc::new(FixedInterfaceProvider::new(
                interface_name,
                interface_index,
                address,
            )),
            Arc::new(production_transport_factory()),
            TestHostPorts::default(),
        ));
        let profile_root = TempDir::new().expect("profile root");
        let profile_id = ProfileId::parse("main").expect("profile id");
        let paths =
            DesktopProfilePaths::from_trusted_app_local_data_root(profile_root.path(), &profile_id)
                .expect("profile paths");
        let state = DesktopAppState::new();
        let buffer = Arc::new(DesktopNotificationBuffer::new());
        state
            .open_profile_sync_with_network(
                &paths,
                profile_id,
                &FixedIdentity,
                &FixedSigningIdentity,
                Arc::clone(&buffer),
                Arc::clone(&network),
            )
            .expect("open production profile");
        let notifications = Arc::new(RecordingNotificationSink::default());
        buffer
            .attach_sink(Arc::clone(&notifications) as Arc<dyn DesktopNotificationSink>)
            .expect("attach notification sink");
        let handle = state.ready_actor_handle().expect("actor handle");
        let workflow = Self {
            state,
            registry: SelectedSourceRegistry::new(),
            network,
            handle,
            notifications,
            address,
            inputs: TempDir::new().expect("input directory"),
            _profile_root: profile_root,
        };
        workflow.submit(CoreCommand::SelectRole {
            role: AppRole::Host,
        });
        workflow.wait(|snapshot| snapshot.selected_role == Some(AppRole::Host));
        workflow.submit(CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: Some("Workflow host".to_owned()),
            approval_mode: Some(ApprovalMode::Manual),
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Unchanged,
            remember_approved_devices: Some(false),
        }));
        workflow.wait(|snapshot| snapshot.host_draft.session_name == "Workflow host");
        Some(workflow)
    }

    /// Writes user files outside the profile and inspects them through the
    /// production multi-select inspection path.
    pub(super) fn pick(&self, files: &[(&str, Vec<u8>)]) -> Vec<InspectedAudioSource> {
        let paths = files
            .iter()
            .map(|(name, bytes)| {
                let path = self.inputs.path().join(name);
                fs::write(&path, bytes).expect("write user file");
                path
            })
            .collect();
        select_and_inspect_files(&FixedDialog(paths), &SystemAudioFileBoundary)
            .expect("inspect selected files")
            .expect("selection was not cancelled")
    }

    /// The production "Add songs" body after the dialog: stage and validate
    /// every file, then register and append them as one queue command.
    pub(super) fn add_songs(
        &self,
        inspected: &[InspectedAudioSource],
    ) -> Result<CommandReceiptDto, DesktopErrorDto> {
        let control = SourceStagingControl::new();
        let operation = control.begin()?;
        let sources = self.state.source_staging_directory()?;
        let staged = host_workflow_ops::stage_and_validate_sources(
            inspected,
            &sources,
            &operation,
            &DiscardProgress,
        )?;
        drop(operation);
        host_workflow_ops::append_staged_sources(&self.state, &self.registry, staged)
    }

    pub(super) fn submit(&self, command: CoreCommand) -> CommandReceiptDto {
        let revision = self.snapshot().revision;
        self.state
            .submit_core_command(revision, command)
            .expect("submit core command")
    }

    pub(super) fn snapshot(&self) -> CoreSnapshot {
        self.handle.current_snapshot().expect("current snapshot")
    }

    pub(super) fn wait(&self, predicate: impl Fn(&CoreSnapshot) -> bool) -> CoreSnapshot {
        super::harness::wait_snapshot(&self.handle, predicate)
    }

    /// Creates the session through the real platform effect runner, which
    /// binds the production socket transport on the injected interface.
    pub(super) fn create_session(&self) -> NetworkEndpoint {
        self.submit(CoreCommand::CreateHostSession);
        let deadline = Instant::now() + TEST_TIMEOUT;
        loop {
            if let Some(active) = self.network.active_host_session().expect("network state") {
                // Binding the socket precedes the actor's AdvertisingStarted
                // acknowledgment. A listener joining before that transition
                // is rejected as host admission unavailable, even though the
                // socket already answers Hello. Wait for authoritative
                // readiness before exercising the listener workflow.
                self.wait(|snapshot| {
                    matches!(
                        snapshot.host_lifecycle,
                        HostLifecycle::WaitingForListeners | HostLifecycle::Ready
                    )
                });
                return active.endpoint;
            }
            assert!(Instant::now() < deadline, "host transport never started");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Joins one loopback-routed listener and approves it through the real
    /// command -> transport-effect dispatch the effect runner performs.
    pub(super) fn join_listener(
        &self,
        endpoint: NetworkEndpoint,
    ) -> Box<dyn ListenerTransportNode> {
        let session_id = self
            .network
            .active_host_session()
            .expect("network state")
            .expect("active session")
            .advertisement
            .session_id;
        let listener_id = DeviceId::new("workflow-listener").expect("listener id");
        let mut listener = production_transport_factory()
            .connect_listener(
                ListenerTransportConfig {
                    local_address: IpAddr::V4(self.address),
                    ..ListenerTransportConfig::loopback(
                        session_id.clone(),
                        listener_id.clone(),
                        endpoint,
                    )
                },
                Arc::new(SystemTransportClock::default()),
            )
            .expect("listener connects");
        let routes = listener.local_routes();
        listener
            .send_control(&ControlMessage::JoinRequest(JoinRequest {
                session_id,
                device: DeviceIdentity {
                    device_id: listener_id,
                    display_name: "Workflow listener".to_owned(),
                },
                invite_code: None,
                sync_port: routes.synchronization.port(),
                audio_port: routes.audio.port(),
            }))
            .expect("send join request");
        wait_for_control(&mut *listener, |message| {
            matches!(message, ControlMessage::Hello(_))
        });
        let joined = self.wait(|snapshot| !snapshot.pending_join_requests.is_empty());
        self.state
            .submit_core_command(
                joined.revision,
                CoreCommand::ApproveJoin {
                    request_id: joined.pending_join_requests[0].request_id.clone(),
                    remember_for_future: false,
                },
            )
            .expect("approve join");
        wait_for_control(&mut *listener, |message| {
            matches!(message, ControlMessage::JoinApproval(_))
        });
        self.wait(|snapshot| !snapshot.listeners.is_empty());
        listener
    }

    pub(super) fn queue_ids(&self) -> Vec<String> {
        self.snapshot()
            .host_queue
            .entries()
            .iter()
            .map(|entry| entry.entry_id.clone())
            .collect()
    }

    pub(super) fn revisions(&self) -> (String, String) {
        let snapshot = self.snapshot();
        (
            snapshot.revision.get().to_string(),
            snapshot.host_queue.revision().to_string(),
        )
    }

    pub(super) fn close(self) {
        self.state.close_sync().expect("close profile");
    }
}

/// Records every notification the frontend channel would receive.
#[derive(Default)]
pub(super) struct RecordingNotificationSink {
    received: Mutex<Vec<CoreNotification>>,
}

impl RecordingNotificationSink {
    /// Waits for a core error notification whose message contains `needle`.
    pub(super) fn wait_for_error(&self, needle: &str) -> String {
        let deadline = Instant::now() + TEST_TIMEOUT;
        loop {
            let found = self
                .received
                .lock()
                .expect("notification lock")
                .iter()
                .find_map(|notification| match notification {
                    CoreNotification::Error(error) if error.message.contains(needle) => {
                        Some(error.message.clone())
                    }
                    _ => None,
                });
            if let Some(message) = found {
                return message;
            }
            assert!(
                Instant::now() < deadline,
                "no frontend error notification mentioned {needle:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl DesktopNotificationSink for RecordingNotificationSink {
    fn send(&self, notification: CoreNotification) -> Result<(), DesktopNotificationSendError> {
        self.received
            .lock()
            .expect("notification lock")
            .push(notification);
        Ok(())
    }
}

struct FixedDialog(Vec<PathBuf>);

impl AudioFileDialog for FixedDialog {
    fn pick_file(&self) -> Result<Option<PathBuf>, DesktopErrorDto> {
        Ok(self.0.first().cloned())
    }

    fn pick_files(&self) -> Result<Option<Vec<PathBuf>>, DesktopErrorDto> {
        Ok(Some(self.0.clone()))
    }
}

pub(super) struct DiscardProgress;

impl SourceStagingProgressSink for DiscardProgress {
    fn emit(&self, _progress: SourceStagingProgressDto) -> Result<(), DesktopErrorDto> {
        Ok(())
    }
}

struct FixedIdentity;

impl DesktopIdentityProvider for FixedIdentity {
    fn load_or_create(
        &self,
        _profile_id: &ProfileId,
    ) -> Result<DesktopIdentity, DesktopIdentityError> {
        DesktopIdentity::from_secret(&[21; 32])
    }
}

struct FixedSigningIdentity;

impl DesktopHostSigningIdentityProvider for FixedSigningIdentity {
    fn load_or_create(
        &self,
        _profile_id: &ProfileId,
    ) -> Result<DesktopHostSigningIdentity, DesktopHostSigningIdentityError> {
        Ok(crate::platform::invitation_identity::tests_support::fixed_identity_for_tests(21))
    }
}
