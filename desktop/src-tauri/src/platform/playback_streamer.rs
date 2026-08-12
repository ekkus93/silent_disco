//! Owned desktop playback pump: drains a packetizer worker and forwards
//! frames to the host transport worker, staying within a bounded send-ahead
//! horizon of the transport's current time rather than pacing one packet per
//! `packet_duration`.

include!("playback_streamer/owner.rs");
include!("playback_streamer/pump.rs");
include!("playback_streamer/tests.rs");
