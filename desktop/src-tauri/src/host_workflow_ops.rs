//! Production bodies of the desktop host queue/session Tauri commands.
//!
//! The `#[tauri::command]` wrappers in `host_commands.rs` only extract Tauri
//! state (`AppHandle`, `State`) and delegate here, so the exact command path
//! -- staging, validation, registry publication, revision-aware actor
//! submission, active-packetizer retargeting, and session-end source
//! release -- is exercised by integration tests without a Tauri runtime.

use crate::app_state::DesktopAppState;
use crate::dto::DesktopErrorDto;
use crate::host_commands::parse_snapshot_revision;
use crate::platform::audio_decode::validate_staged_audio_source;
use crate::platform::file_picker::{
    InspectedAudioSource, SelectedSourceRegistry, named_source_error,
};
use crate::platform::source_staging::{StagingResult, stage_audio_source};
use crate::platform::source_staging_control::{SourceStagingOperation, SourceStagingProgressSink};
use crate::runtime_dto::{
    AppendHostQueueRequest, CommandReceiptDto, HostQueueClearRequest, HostQueueEntryCommandRequest,
    ReorderHostQueueRequest, RevisionCommandRequest,
};
use silent_disco_core::runtime::{AudioSourceDescriptor, CoreCommand, QueueEntry};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_QUEUE_ENTRY_ID: AtomicU64 = AtomicU64::new(1);

/// Stages and fully decode-validates every selected source before any of them
/// becomes visible to the registry or the actor. The first failure aborts the
/// whole selection with an item-specific error, so a bad file can never
/// partially extend the queue.
pub(crate) fn stage_and_validate_sources(
    inspected: &[InspectedAudioSource],
    sources_directory: &Path,
    operation: &SourceStagingOperation,
    progress: &dyn SourceStagingProgressSink,
) -> Result<Vec<StagingResult>, DesktopErrorDto> {
    inspected
        .iter()
        .map(|source| {
            let staged = stage_audio_source(source, sources_directory, operation, progress)
                .map_err(|error| named_source_error(&error, &source.descriptor().display_name))?;
            validate_staged_audio_source(&staged.source, operation)
                .map_err(|error| named_source_error(&error, &source.descriptor().display_name))?;
            Ok(staged)
        })
        .collect()
}

/// Registers already-validated staged sources and appends them as one queue
/// mutation, then retargets an active packetizer so live additions are
/// prebuffered like any other queue edit.
///
/// Revisions are read from the authoritative snapshot here, after staging,
/// not before the file dialog opened: playback position, listener sync, and
/// track boundaries advance the snapshot revision continuously during a live
/// session, so a revision captured before a multi-second pick/copy/decode
/// would reject every live "Add songs". Append only extends the queue tail,
/// so it carries no stale-position hazard.
pub(crate) fn append_staged_sources(
    state: &DesktopAppState,
    registry: &SelectedSourceRegistry,
    staged: Vec<StagingResult>,
) -> Result<CommandReceiptDto, DesktopErrorDto> {
    let mut entries = Vec::with_capacity(staged.len());
    let mut newly_registered = Vec::new();
    for StagingResult { source, .. } in staged {
        let descriptor = source.descriptor().clone();
        if registry.register(source)?.is_none() {
            newly_registered.push(descriptor.source_id.clone());
        }
        let sequence = NEXT_QUEUE_ENTRY_ID.fetch_add(1, Ordering::Relaxed);
        entries.push(
            QueueEntry::new(format!("queue-entry-{sequence}"), descriptor)
                .map_err(|error| queue_validation_error(&error.to_string()))?,
        );
    }

    let current = state.current_snapshot()?;
    let receipt = parse_queue_revision(&current.host_queue.revision).and_then(|queue_revision| {
        state.submit_core_command(
            parse_snapshot_revision(&current.revision)?,
            CoreCommand::AppendHostQueue {
                expected_queue_revision: queue_revision,
                entries,
            },
        )
    });
    match receipt {
        Ok(receipt) => {
            retarget_after_queue_change(state, registry, &current.host_queue.revision)?;
            Ok(receipt)
        }
        Err(error) => {
            for source_id in newly_registered {
                registry.remove(&source_id)?;
            }
            Err(error)
        }
    }
}

/// Appends already-staged sources as one revision-aware queue mutation.
pub(crate) fn append_host_queue(
    state: &DesktopAppState,
    registry: &SelectedSourceRegistry,
    request: AppendHostQueueRequest,
) -> Result<CommandReceiptDto, DesktopErrorDto> {
    let revision = parse_snapshot_revision(&request.expected_revision)?;
    let queue_revision = parse_queue_revision(&request.expected_queue_revision)?;
    let mut entries = Vec::with_capacity(request.entries.len());
    for item in request.entries {
        let descriptor = AudioSourceDescriptor::new(
            item.source.source_id,
            item.source.display_name,
            item.source
                .byte_length
                .as_deref()
                .map(parse_queue_revision)
                .transpose()?,
            item.source
                .duration_ms
                .as_deref()
                .map(parse_queue_revision)
                .transpose()?,
        )
        .map_err(|error| queue_validation_error(&error.to_string()))?;
        let staged = registry.resolve(&descriptor.source_id)?.ok_or_else(|| {
            queue_validation_error("queue source is not staged in this desktop profile")
        })?;
        if staged.descriptor() != &descriptor {
            return Err(queue_validation_error(
                "staged queue source metadata does not match the request",
            ));
        }
        entries.push(
            QueueEntry::new(item.entry_id, descriptor)
                .map_err(|error| queue_validation_error(&error.to_string()))?,
        );
    }
    let receipt = state.submit_core_command(
        revision,
        CoreCommand::AppendHostQueue {
            expected_queue_revision: queue_revision,
            entries,
        },
    )?;
    retarget_after_queue_change(state, registry, &request.expected_queue_revision)?;
    Ok(receipt)
}

pub(crate) fn reorder_host_queue(
    state: &DesktopAppState,
    registry: &SelectedSourceRegistry,
    request: ReorderHostQueueRequest,
) -> Result<CommandReceiptDto, DesktopErrorDto> {
    let receipt = state.submit_core_command(
        parse_snapshot_revision(&request.expected_revision)?,
        CoreCommand::ReorderHostQueue {
            expected_queue_revision: parse_queue_revision(&request.expected_queue_revision)?,
            ordered_entry_ids: request.ordered_entry_ids,
        },
    )?;
    retarget_after_queue_change(state, registry, &request.expected_queue_revision)?;
    Ok(receipt)
}

pub(crate) fn remove_host_queue_entry(
    state: &DesktopAppState,
    registry: &SelectedSourceRegistry,
    request: HostQueueEntryCommandRequest,
) -> Result<CommandReceiptDto, DesktopErrorDto> {
    let receipt = state.submit_core_command(
        parse_snapshot_revision(&request.expected_revision)?,
        CoreCommand::RemoveHostQueueEntry {
            expected_queue_revision: parse_queue_revision(&request.expected_queue_revision)?,
            entry_id: request.entry_id,
        },
    )?;
    retarget_after_queue_change(state, registry, &request.expected_queue_revision)?;
    Ok(receipt)
}

pub(crate) fn clear_host_queue(
    state: &DesktopAppState,
    registry: &SelectedSourceRegistry,
    request: &HostQueueClearRequest,
) -> Result<CommandReceiptDto, DesktopErrorDto> {
    let receipt = state.submit_core_command(
        parse_snapshot_revision(&request.expected_revision)?,
        CoreCommand::ClearHostQueue {
            expected_queue_revision: parse_queue_revision(&request.expected_queue_revision)?,
        },
    )?;
    retarget_after_queue_change(state, registry, &request.expected_queue_revision)?;
    Ok(receipt)
}

/// Requests a revision-aware host-session shutdown and releases every staged
/// source the session's queue referenced.
pub(crate) fn end_host_session(
    state: &DesktopAppState,
    registry: &SelectedSourceRegistry,
    request: &RevisionCommandRequest,
) -> Result<CommandReceiptDto, DesktopErrorDto> {
    let receipt = state.submit_core_command(
        parse_snapshot_revision(&request.expected_revision)?,
        CoreCommand::EndHostSession,
    )?;
    registry.clear()?;
    Ok(receipt)
}

fn retarget_after_queue_change(
    state: &DesktopAppState,
    registry: &SelectedSourceRegistry,
    previous_queue_revision: &str,
) -> Result<(), DesktopErrorDto> {
    if let Some(snapshot) = state.wait_for_queue_revision_change(previous_queue_revision)? {
        state.retarget_active_host_queue(&snapshot, registry)?;
    }
    Ok(())
}

pub(crate) fn parse_queue_revision(value: &str) -> Result<u64, DesktopErrorDto> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(queue_validation_error(
            "expected queue revision must be a canonical unsigned decimal string",
        ));
    }
    value.parse::<u64>().map_err(|error| {
        queue_validation_error(&format!("queue revision is out of range: {error}"))
    })
}

pub(crate) fn queue_validation_error(message: &str) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.host_queue.invalid_request",
        "validation",
        "error",
        false,
        message,
    )
}
