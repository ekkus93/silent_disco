//! Playlist packetizer constructors and optional synchronous boundary gate.
use super::*;

impl StreamingPacketizeHandle {
    /// Starts one continuous stream across a bounded sequence of tracks.
    /// The next source is opened while the current decoder is running, so at
    /// most two decoder queues exist at any time and its first PCM chunk is
    /// ready before the boundary whenever decoding keeps pace.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the first decoder format/configuration or
    /// current track metadata is invalid, or when the worker cannot start.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_playlist(
        decoder: StreamingDecodeHandle,
        current_entry_id: impl Into<String>,
        current_display_name: impl Into<String>,
        upcoming: Vec<StreamingPlaylistTrackFactory>,
        session_id: SessionId,
        stream_id: StreamId,
        host_start_time_ms: MonotonicMillis,
        config: StreamingPacketizeConfig,
    ) -> Result<Self, PacketizerWorkerError> {
        Self::spawn_playlist_inner(
            decoder,
            current_entry_id,
            current_display_name,
            upcoming,
            session_id,
            stream_id,
            host_start_time_ms,
            config,
            None,
        )
    }

    /// Starts a playlist whose producer must synchronously reserve each next
    /// track with the authoritative actor *before* emitting its boundary or
    /// successor audio. The callback must not wait for a frame consumer.
    ///
    /// # Errors
    /// Rejects invalid sources, a failed worker spawn, or packetizer setup.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_playlist_reserved(
        decoder: StreamingDecodeHandle,
        current_entry_id: impl Into<String>,
        current_display_name: impl Into<String>,
        upcoming: Vec<StreamingPlaylistTrackFactory>,
        session_id: SessionId,
        stream_id: StreamId,
        host_start_time_ms: MonotonicMillis,
        config: StreamingPacketizeConfig,
        reserve: Arc<
            dyn Fn(&TrackBoundary, Arc<AtomicBool>) -> Result<(), PacketizerWorkerError>
                + Send
                + Sync,
        >,
    ) -> Result<Self, PacketizerWorkerError> {
        Self::spawn_playlist_inner(
            decoder,
            current_entry_id,
            current_display_name,
            upcoming,
            session_id,
            stream_id,
            host_start_time_ms,
            config,
            Some(reserve),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_playlist_inner(
        decoder: StreamingDecodeHandle,
        current_entry_id: impl Into<String>,
        current_display_name: impl Into<String>,
        upcoming: Vec<StreamingPlaylistTrackFactory>,
        session_id: SessionId,
        stream_id: StreamId,
        host_start_time_ms: MonotonicMillis,
        config: StreamingPacketizeConfig,
        boundary_reserver: Option<BoundaryReserver>,
    ) -> Result<Self, PacketizerWorkerError> {
        let current_entry_id = current_entry_id.into();
        let current_display_name = current_display_name.into();
        validate_track_identity(&current_entry_id, &current_display_name)?;
        validate_playlist_entries(Some(&current_entry_id), &upcoming)?;
        let format = decoder.stream_info().format;
        let packetizer = Packetizer::new(
            session_id,
            stream_id,
            format,
            config.packet_duration_ms,
            host_start_time_ms,
        )?;
        let mut stream_start = packetizer.stream_start_message();
        if let ProtocolFrame::Control(ControlMessage::StreamStart(start)) = &mut stream_start {
            start.current_entry_id = Some(current_entry_id.clone());
            start.current_display_name = Some(current_display_name.clone());
        }
        let (sender, receiver) = sync_channel(config.queue_capacity);
        let cancellation = Arc::new(AtomicBool::new(false));
        let statistics = Arc::new(SharedStatistics::default());
        let worker_cancellation = Arc::clone(&cancellation);
        let worker_statistics = Arc::clone(&statistics);
        let playlist_updater = StreamingPlaylistUpdater {
            pending: Arc::new(Mutex::new(PlaylistUpdateState {
                current_entry_id: current_entry_id.clone(),
                pending: None,
                terminal: false,
            })),
        };
        let worker_updates = Arc::clone(&playlist_updater.pending);
        let join = thread::Builder::new()
            .name("silent-disco-streaming-playlist-packetizer".to_owned())
            .spawn(move || {
                run_playlist_packetizer_worker(
                    decoder,
                    current_entry_id,
                    current_display_name,
                    upcoming,
                    &worker_updates,
                    packetizer,
                    &sender,
                    config,
                    &worker_cancellation,
                    &worker_statistics,
                    boundary_reserver.as_ref(),
                )
            })
            .map_err(|error| {
                PacketizerWorkerError::new(
                    PacketizerWorkerErrorKind::WorkerPanicked,
                    format!("could not start the playlist packetizer worker: {error}"),
                )
            })?;
        Ok(Self {
            receiver,
            cancellation,
            statistics,
            config,
            stream_start,
            playlist_updater: Some(playlist_updater),
            join: Some(join),
        })
    }



}
