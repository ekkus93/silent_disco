//! One-line QUIC connection statistics for diagnostics logs: whether audio
//! datagrams left the host, reached the listener, or were lost between.

/// Datagram frames sent and received, UDP datagrams each way, packets the
/// connection detected as lost (of its own sends), congestion events, the
/// congestion window and the smoothed round trip.
#[must_use]
pub fn stats_line(stats: &quinn::ConnectionStats) -> String {
    format!(
        "datagram_frames_tx={} datagram_frames_rx={} udp_tx={} udp_rx={} lost_packets={} \
         lost_bytes={} congestion_events={} cwnd={} rtt_ms={}",
        stats.frame_tx.datagram,
        stats.frame_rx.datagram,
        stats.udp_tx.datagrams,
        stats.udp_rx.datagrams,
        stats.path.lost_packets,
        stats.path.lost_bytes,
        stats.path.congestion_events,
        stats.path.cwnd,
        stats.path.rtt.as_millis(),
    )
}
