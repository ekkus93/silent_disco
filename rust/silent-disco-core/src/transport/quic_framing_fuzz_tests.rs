//! Hostile chunking, hostile lengths, truncation and cancellation for the v4
//! control-stream framing (remediation TODO §7), over real loopback QUIC.
//!
//! A peer controls how its bytes are split into STREAM frames, so the reader
//! must reassemble a frame from any split, refuse a bad length before
//! allocating, and never return a partial frame. Same fixed-seed generator as
//! the other fuzz tests (no nightly fuzzer in the pinned toolchain).

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use p256::ecdsa::SigningKey;
use p256::pkcs8::EncodePrivateKey;

use super::DeterministicPrng;
use super::quic_datagram::{quic_client_config, quic_server_config};
use super::quic_framing::{MAX_QUIC_CONTROL_FRAME_BYTES, QuicFrameError, read_frame};
use super::secure_control_tls::{HOST_SERVER_NAME, host_tls_identity_from_pkcs8};

const STEP: Duration = Duration::from_secs(10);

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

fn localhost() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, 0))
}

/// Both ends of one control stream on a real loopback QUIC connection: the
/// peer writes raw bytes, the host side reads with the production reader.
struct StreamPair {
    send: quinn::SendStream,
    recv: quinn::RecvStream,
    // Held so the connection stays open for the test.
    _server: quinn::Endpoint,
    _client: quinn::Endpoint,
    _connections: (quinn::Connection, quinn::Connection),
}

/// Opens a control stream. QUIC only announces a stream once it carries
/// data, so `first_bytes` (non-empty) are written before the reader accepts.
async fn stream_pair(first_bytes: &[u8]) -> StreamPair {
    assert!(!first_bytes.is_empty());
    let key = SigningKey::from_slice(&[9; 32]).expect("fixture key");
    let identity = host_tls_identity_from_pkcs8(key.to_pkcs8_der().expect("pkcs8").as_bytes())
        .expect("identity");
    let server =
        quinn::Endpoint::server(quic_server_config(&identity).expect("server"), localhost())
            .expect("server endpoint");
    let address = server.local_addr().expect("server address");
    let mut client = quinn::Endpoint::client(localhost()).expect("client endpoint");
    client.set_default_client_config(quic_client_config(&identity.spki).expect("client"));

    let accepting = server.clone();
    let host = tokio::spawn(async move {
        accepting
            .accept()
            .await
            .expect("incoming")
            .await
            .expect("host handshake")
    });
    let peer = tokio::time::timeout(
        STEP,
        client.connect(address, HOST_SERVER_NAME).expect("connect"),
    )
    .await
    .expect("handshake in time")
    .expect("peer handshake");
    let host = tokio::time::timeout(STEP, host)
        .await
        .expect("host in time")
        .expect("host task");
    let (mut send, _peer_recv) = peer.open_bi().await.expect("open stream");
    send.write_all(first_bytes).await.expect("first bytes");
    let (_host_send, recv) = tokio::time::timeout(STEP, host.accept_bi())
        .await
        .expect("stream in time")
        .expect("accept stream");
    StreamPair {
        send,
        recv,
        _server: server,
        _client: client,
        _connections: (host, peer),
    }
}

fn encode(frame: &[u8]) -> Vec<u8> {
    let mut bytes = u32::try_from(frame.len())
        .expect("length")
        .to_be_bytes()
        .to_vec();
    bytes.extend_from_slice(frame);
    bytes
}

/// A frame length that favours the edges: 1 byte, the exact maximum, and
/// lengths around the 4-byte prefix and common packet sizes.
fn frame_length(prng: &mut DeterministicPrng) -> usize {
    match prng.next_below(8) {
        0 => 1,
        1 => MAX_QUIC_CONTROL_FRAME_BYTES,
        2 => 2 + prng.next_below(6),
        3 => 1_190 + prng.next_below(40),
        _ => 1 + prng.next_below(4_096),
    }
}

/// A hostile split: single bytes, a split inside the 4-byte prefix, or
/// arbitrary runs up to 8 KiB.
fn chunk_length(prng: &mut DeterministicPrng) -> usize {
    match prng.next_below(4) {
        0 => 1,
        1 => 1 + prng.next_below(4),
        _ => 1 + prng.next_below(8_192),
    }
}

async fn read_in_time(recv: &mut quinn::RecvStream) -> Result<Vec<u8>, QuicFrameError> {
    tokio::time::timeout(STEP, read_frame(recv))
        .await
        .expect("reader settles in time")
}

#[test]
fn frames_split_into_hostile_chunks_reassemble_exactly_in_order() {
    runtime().block_on(async {
        for round in 0..6_u64 {
            let mut prng = DeterministicPrng::new(0x5D15_C0F0 ^ round);
            let frames: Vec<Vec<u8>> = (0..24)
                .map(|_| {
                    let length = frame_length(&mut prng);
                    (0..length)
                        .map(|_| prng.next_u64().to_le_bytes()[0])
                        .collect()
                })
                .collect();
            let wire: Vec<u8> = frames.iter().flat_map(|frame| encode(frame)).collect();

            // Split into hostile chunks; the first chunk opens the stream.
            let mut chunks = Vec::new();
            let mut offset = 0;
            while offset < wire.len() {
                let end = (offset + chunk_length(&mut prng)).min(wire.len());
                chunks.push(wire[offset..end].to_vec());
                offset = end;
            }
            let StreamPair {
                mut send,
                mut recv,
                _server,
                _client,
                _connections,
            } = stream_pair(&chunks[0]).await;
            let writer = tokio::spawn(async move {
                for chunk in &chunks[1..] {
                    send.write_all(chunk).await.expect("chunk");
                    tokio::task::yield_now().await;
                }
                send.finish().expect("finish");
                send
            });

            for (index, expected) in frames.iter().enumerate() {
                let frame = read_in_time(&mut recv)
                    .await
                    .unwrap_or_else(|error| panic!("round {round} frame {index}: {error}"));
                assert_eq!(&frame, expected, "round {round} frame {index} differs");
            }
            assert_eq!(read_in_time(&mut recv).await, Err(QuicFrameError::Finished));
            let _send = writer.await.expect("writer");
        }
    });
}

#[test]
fn hostile_length_prefixes_are_refused_without_waiting_for_a_body() {
    runtime().block_on(async {
        let too_long = u32::try_from(MAX_QUIC_CONTROL_FRAME_BYTES + 1).expect("length");
        for length in [0_u32, too_long, 0x0100_0000, u32::MAX] {
            // Only the prefix is sent and the stream stays open: a reader that
            // trusted the length would wait for (or allocate) the body.
            let StreamPair {
                mut recv,
                send: _send,
                _server,
                _client,
                _connections,
            } = stream_pair(&length.to_be_bytes()).await;
            assert_eq!(
                read_in_time(&mut recv).await,
                Err(QuicFrameError::Oversized(
                    usize::try_from(length).expect("usize")
                )),
                "length {length}"
            );
        }

        // The exact maximum is a valid frame.
        let largest = vec![0xA5_u8; MAX_QUIC_CONTROL_FRAME_BYTES];
        let StreamPair {
            mut send,
            mut recv,
            _server,
            _client,
            _connections,
        } = stream_pair(&encode(&largest)[..1]).await;
        let rest = encode(&largest)[1..].to_vec();
        let writer = tokio::spawn(async move {
            send.write_all(&rest).await.expect("rest");
            send
        });
        assert_eq!(read_in_time(&mut recv).await, Ok(largest));
        let _send = writer.await.expect("writer");
    });
}

#[test]
fn a_stream_cut_off_mid_frame_is_an_error_never_a_partial_frame() {
    runtime().block_on(async {
        let frame = encode(&[7_u8; 600]);
        // Cut inside the prefix, right after it, and inside the body.
        for cut in [1_usize, 3, 4, 5, 300, frame.len() - 1] {
            let StreamPair {
                mut send,
                mut recv,
                _server,
                _client,
                _connections,
            } = stream_pair(&frame[..cut]).await;
            send.finish().expect("finish");
            let outcome = read_in_time(&mut recv).await;
            assert!(
                matches!(outcome, Err(QuicFrameError::Stream(_))),
                "cut at {cut}: {outcome:?}"
            );
        }

        // A reset mid-body is an error too.
        let StreamPair {
            mut send,
            mut recv,
            _server,
            _client,
            _connections,
        } = stream_pair(&frame[..300]).await;
        send.reset(0_u32.into()).expect("reset");
        let outcome = read_in_time(&mut recv).await;
        assert!(
            matches!(outcome, Err(QuicFrameError::Stream(_))),
            "{outcome:?}"
        );

        // Finishing exactly at a boundary is a clean end after the frame.
        let StreamPair {
            mut send,
            mut recv,
            _server,
            _client,
            _connections,
        } = stream_pair(&frame).await;
        send.finish().expect("finish");
        assert_eq!(read_in_time(&mut recv).await, Ok(vec![7_u8; 600]));
        assert_eq!(read_in_time(&mut recv).await, Err(QuicFrameError::Finished));
    });
}

/// `read_frame` is cancel-safe only between frames: a reader dropped while
/// waiting for the next frame's first byte loses nothing. The workers rely on
/// this and never cancel a read mid-frame (each runs in its own task that is
/// only ever stopped as a whole).
#[test]
fn dropping_a_read_that_is_waiting_between_frames_loses_nothing() {
    runtime().block_on(async {
        let mut prng = DeterministicPrng::new(0xCA9C_E11E);
        let first = encode(&[1_u8; 40]);
        let StreamPair {
            mut send,
            mut recv,
            _server,
            _client,
            _connections,
        } = stream_pair(&first).await;
        assert_eq!(read_in_time(&mut recv).await, Ok(vec![1_u8; 40]));

        for index in 2..=12_u8 {
            // Nothing is sent yet: the read times out and its future is dropped.
            let waited =
                tokio::time::timeout(Duration::from_millis(20), read_frame(&mut recv)).await;
            assert!(
                waited.is_err(),
                "frame {index}: a read completed with nothing sent"
            );

            let length = 1 + prng.next_below(2_000);
            let body = vec![index; length];
            let wire = encode(&body);
            let split = 1 + prng.next_below(wire.len() - 1);
            send.write_all(&wire[..split]).await.expect("first part");
            tokio::task::yield_now().await;
            send.write_all(&wire[split..]).await.expect("second part");
            assert_eq!(read_in_time(&mut recv).await, Ok(body), "frame {index}");
        }
    });
}
