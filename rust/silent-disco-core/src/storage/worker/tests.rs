use std::{
    sync::{Arc, mpsc::sync_channel},
    thread,
};

use super::{DatabaseCommand, DatabaseWorker};
use crate::{
    domain::{AppRole, DeviceId, DiagnosticRunId, SessionId, TrustState, TuningSettings},
    storage::{
        DatabaseConfig, DiagnosticExportRequest, DiagnosticQuery, DiagnosticRunSummary, SessionEnd,
        SessionOutcome, SessionStart, SessionUpdate, StorageErrorKind, StoredSettings,
        TrustedDevice, test_support::TestDatabasePath,
    },
};

#[test]
fn one_worker_thread_owns_every_connection_operation() {
    let test_path = TestDatabasePath::new("worker-ownership");
    let config = DatabaseConfig::new(test_path.path())
        .and_then(|config| config.with_queue_capacity(16))
        .expect("valid worker config");
    let worker = DatabaseWorker::start(config).expect("worker starts");
    let expected_owner = worker.initial_metadata().owner_thread_id.clone();
    let client = Arc::new(worker.client());
    let mut calls = Vec::new();
    for _ in 0..8 {
        let client = Arc::clone(&client);
        calls.push(thread::spawn(move || {
            client.metadata().map(|metadata| metadata.owner_thread_id)
        }));
    }
    for call in calls {
        let owner = call
            .join()
            .expect("client thread does not panic")
            .expect("metadata succeeds");
        assert_eq!(owner, expected_owner);
    }
    worker.stop_and_join().expect("worker closes and joins");
}

#[test]
fn full_queue_rejects_visibly_without_dropping_accepted_command() {
    let test_path = TestDatabasePath::new("worker-queue");
    let config = DatabaseConfig::new(test_path.path())
        .and_then(|config| config.with_queue_capacity(1))
        .expect("valid worker config");
    let worker = DatabaseWorker::start(config).expect("worker starts");
    let (entered_sender, entered_receiver) = sync_channel(1);
    let (release_sender, release_receiver) = sync_channel(1);
    worker
        .client
        .sender
        .send(DatabaseCommand::BlockForQueueTest {
            entered: entered_sender,
            release: release_receiver,
        })
        .expect("barrier command accepted");
    entered_receiver.recv().expect("worker entered barrier");

    let (queued_reply_sender, queued_reply_receiver) = sync_channel(1);
    worker
        .client
        .sender
        .try_send(DatabaseCommand::ReadMetadata {
            reply: queued_reply_sender,
        })
        .expect("one command fills queue");
    let overflow = worker
        .client
        .metadata()
        .expect_err("queue must reject overflow");
    assert_eq!(overflow.kind, StorageErrorKind::QueueFull);

    release_sender.send(()).expect("release worker");
    queued_reply_receiver
        .recv()
        .expect("accepted command receives reply")
        .expect("accepted command succeeds");
    worker.stop_and_join().expect("worker closes and joins");
}

#[test]
fn typed_repositories_round_trip_domain_records() {
    let test_path = TestDatabasePath::new("worker-repositories");
    let worker =
        DatabaseWorker::start(DatabaseConfig::new(test_path.path()).expect("valid worker config"))
            .expect("worker starts");
    let client = worker.client();
    assert_eq!(client.load_settings().expect("load settings"), None);

    let settings = StoredSettings {
        tuning: TuningSettings::default(),
        updated_at_ms: 100,
    };
    client.save_settings(&settings).expect("save settings");
    assert_eq!(
        client.load_settings().expect("reload settings"),
        Some(settings)
    );

    let device = sample_device();
    client
        .upsert_trusted_device(&device)
        .expect("upsert trusted device");
    assert_eq!(
        client
            .get_trusted_device(&device.device_id)
            .expect("get trusted device"),
        Some(device.clone())
    );
    assert_eq!(
        client.list_trusted_devices().expect("list trusted devices"),
        vec![device]
    );

    let session = sample_session_start();
    client.begin_session(&session).expect("begin session");
    assert!(
        client
            .update_session(&SessionUpdate {
                session_id: session.session_id.clone(),
                listener_count: 3,
            })
            .expect("update session")
    );
    assert!(
        client
            .end_session(&SessionEnd {
                session_id: session.session_id.clone(),
                ended_at_ms: 250,
                listener_count: 3,
                outcome: SessionOutcome::Completed,
                failure_code: None,
                failure_message: None,
            })
            .expect("end session")
    );

    let diagnostic = sample_diagnostic(&session.session_id);
    client
        .insert_diagnostic_run(&diagnostic)
        .expect("insert diagnostic run");
    let query = DiagnosticQuery {
        session_id: Some(session.session_id.clone()),
        limit: 10,
    };
    assert_eq!(
        client
            .query_diagnostic_runs(&query)
            .expect("query diagnostics"),
        vec![diagnostic.clone()]
    );
    let export = client
        .export_diagnostic_runs(&DiagnosticExportRequest {
            session_id: None,
            cursor: None,
            limit: 10,
        })
        .expect("export diagnostics");
    assert_eq!(export.runs, vec![diagnostic]);
    assert_eq!(export.next_cursor, None);
    worker.stop_and_join().expect("worker closes and joins");
}

#[test]
fn diagnostic_export_is_bounded_and_cursor_paginated() {
    let test_path = TestDatabasePath::new("worker-export-pagination");
    let worker =
        DatabaseWorker::start(DatabaseConfig::new(test_path.path()).expect("valid worker config"))
            .expect("worker starts");
    let client = worker.client();
    let session = sample_session_start();
    client.begin_session(&session).expect("begin session");

    for (suffix, started_at_ms) in [("a", 110), ("b", 120), ("c", 130)] {
        client
            .insert_diagnostic_run(&DiagnosticRunSummary {
                run_id: DiagnosticRunId::new(format!("diagnostic-{suffix}"))
                    .expect("valid diagnostic identifier"),
                session_id: Some(session.session_id.clone()),
                started_at_ms,
                ended_at_ms: Some(started_at_ms + 1),
                summary_json: format!(r#"{{"run":"{suffix}"}}"#),
            })
            .expect("insert diagnostic run");
    }

    let first = client
        .export_diagnostic_runs(&DiagnosticExportRequest {
            session_id: Some(session.session_id.clone()),
            cursor: None,
            limit: 2,
        })
        .expect("first export page");
    assert_eq!(first.runs.len(), 2);
    let cursor = first.next_cursor.expect("more rows remain");

    let second = client
        .export_diagnostic_runs(&DiagnosticExportRequest {
            session_id: Some(session.session_id),
            cursor: Some(cursor),
            limit: 2,
        })
        .expect("second export page");
    assert_eq!(second.runs.len(), 1);
    assert_eq!(second.next_cursor, None);
    worker.stop_and_join().expect("worker closes and joins");
}

#[test]
fn recent_session_history_is_bounded_and_deterministically_ordered() {
    let test_path = TestDatabasePath::new("worker-recent-sessions");
    let worker =
        DatabaseWorker::start(DatabaseConfig::new(test_path.path()).expect("valid worker config"))
            .expect("worker starts");
    let client = worker.client();

    for (session_id, started_at_ms) in [("session-b", 300), ("session-a", 300), ("session-c", 200)]
    {
        client
            .begin_session(&SessionStart {
                session_id: SessionId::new(session_id).expect("valid session identifier"),
                role: AppRole::Host,
                session_name: format!("Session {session_id}"),
                started_at_ms,
            })
            .expect("begin session");
    }

    let recent = client
        .list_recent_sessions(2)
        .expect("bounded recent history query");
    assert_eq!(
        recent
            .iter()
            .map(|history| history.session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["session-a", "session-b"]
    );

    let invalid = client
        .list_recent_sessions(0)
        .expect_err("zero history limit is rejected");
    assert_eq!(invalid.kind, StorageErrorKind::InvalidConfiguration);
    worker.stop_and_join().expect("worker closes and joins");
}

#[test]
fn duplicate_session_maps_to_constraint_violation() {
    let test_path = TestDatabasePath::new("worker-constraint");
    let worker =
        DatabaseWorker::start(DatabaseConfig::new(test_path.path()).expect("valid worker config"))
            .expect("worker starts");
    let client = worker.client();
    let session = sample_session_start();
    client
        .begin_session(&session)
        .expect("first insert succeeds");
    let error = client
        .begin_session(&session)
        .expect_err("duplicate primary key must fail");
    assert_eq!(error.kind, StorageErrorKind::Constraint);
    worker.stop_and_join().expect("worker closes and joins");
}

#[test]
fn queued_write_order_is_serialized_by_the_worker() {
    let test_path = TestDatabasePath::new("worker-ordering");
    let config = DatabaseConfig::new(test_path.path())
        .and_then(|config| config.with_queue_capacity(4))
        .expect("valid worker config");
    let worker = DatabaseWorker::start(config).expect("worker starts");
    let (entered_sender, entered_receiver) = sync_channel(1);
    let (release_sender, release_receiver) = sync_channel(1);
    worker
        .client
        .sender
        .send(DatabaseCommand::BlockForQueueTest {
            entered: entered_sender,
            release: release_receiver,
        })
        .expect("barrier command accepted");
    entered_receiver.recv().expect("worker entered barrier");

    let first = StoredSettings {
        tuning: TuningSettings {
            scan_window_ms: 4_000,
            ..TuningSettings::default()
        },
        updated_at_ms: 1,
    };
    let second = StoredSettings {
        tuning: TuningSettings {
            scan_window_ms: 5_000,
            ..TuningSettings::default()
        },
        updated_at_ms: 2,
    };
    let (first_sender, first_receiver) = sync_channel(1);
    let (second_sender, second_receiver) = sync_channel(1);
    let (load_sender, load_receiver) = sync_channel(1);
    worker
        .client
        .sender
        .send(DatabaseCommand::SaveSettings {
            settings: first,
            reply: first_sender,
        })
        .expect("first save queued");
    worker
        .client
        .sender
        .send(DatabaseCommand::SaveSettings {
            settings: second.clone(),
            reply: second_sender,
        })
        .expect("second save queued");
    worker
        .client
        .sender
        .send(DatabaseCommand::LoadSettings { reply: load_sender })
        .expect("load queued");
    release_sender.send(()).expect("release worker");

    first_receiver
        .recv()
        .expect("first reply received")
        .expect("first save succeeds");
    second_receiver
        .recv()
        .expect("second reply received")
        .expect("second save succeeds");
    assert_eq!(
        load_receiver
            .recv()
            .expect("load reply received")
            .expect("load succeeds"),
        Some(second)
    );
    worker.stop_and_join().expect("worker closes and joins");
}

#[test]
fn explicit_stop_then_join_rejects_cloned_clients_deterministically() {
    let test_path = TestDatabasePath::new("worker-lifecycle");
    let config = DatabaseConfig::new(test_path.path()).expect("valid worker config");
    let mut worker = DatabaseWorker::start(config).expect("worker starts");
    let client = worker.client();

    worker.stop().expect("stop checkpoints and closes");
    let error = client
        .metadata()
        .expect_err("stopped worker rejects requests");
    assert_eq!(error.kind, StorageErrorKind::WorkerStopped);
    worker.join().expect("join succeeds");
}

fn sample_device() -> TrustedDevice {
    TrustedDevice {
        device_id: DeviceId::new("listener-東京").expect("valid device identifier"),
        display_name: "Zoë 🎧 東京".into(),
        public_key: Some(vec![0, 1, 2, 0xff, 0]),
        private_key_ref: Some("keystore:listener-1".into()),
        trust_state: TrustState::Trusted,
        first_seen_ms: 10,
        last_seen_ms: 20,
        updated_at_ms: 30,
    }
}

fn sample_session_start() -> SessionStart {
    SessionStart {
        session_id: SessionId::new("session-1").expect("valid session identifier"),
        role: AppRole::Host,
        session_name: "Noche silenciosa 東京".into(),
        started_at_ms: 100,
    }
}

fn sample_diagnostic(session_id: &SessionId) -> DiagnosticRunSummary {
    DiagnosticRunSummary {
        run_id: DiagnosticRunId::new("diagnostic-1").expect("valid diagnostic identifier"),
        session_id: Some(session_id.clone()),
        started_at_ms: 110,
        ended_at_ms: Some(240),
        summary_json: r#"{"listeners":3,"quality":"good"}"#.into(),
    }
}
