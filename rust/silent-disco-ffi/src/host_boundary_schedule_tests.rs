//! The combined Android/desktop host boundary scenario (gapless TODO §5).
//!
//! The Android stream loop reaches the schedule through this `UniFFI` object;
//! the desktop pump builds the core schedule at the canonical rate. One script
//! of inputs drives both, and every step must give the same boundaries back.

use silent_disco_core::audio::{
    CANONICAL_SAMPLE_RATE_HZ, HostBoundarySchedule, PendingHostBoundary,
};

use super::{FfiHostBoundary, FfiHostBoundarySchedule};

const START_MS: u64 = 1_000_000;

enum Step {
    Schedule(&'static str, u64),
    /// Ask for due boundaries at a host time with a total pause offset.
    Poll {
        now_ms: u64,
        pause_offset_ms: u64,
    },
    WithdrawFrom(u64),
}

/// Three tracks, a pause before the first boundary, a boundary scheduled
/// after the pause, a second pause, overdue boundaries collected together,
/// and a failure that withdraws a boundary at the end of the sent audio.
fn scenario() -> Vec<Step> {
    use Step::{Poll, Schedule, WithdrawFrom};
    vec![
        Schedule("track-2", 240_241), // 5 005.020 ms: not on a packet edge
        Poll {
            now_ms: START_MS + 5_004,
            pause_offset_ms: 0,
        },
        // A 1 500 ms pause: the boundary moves with the audio.
        Poll {
            now_ms: START_MS + 5_005,
            pause_offset_ms: 1_500,
        },
        Poll {
            now_ms: START_MS + 6_504,
            pause_offset_ms: 1_500,
        },
        Poll {
            now_ms: START_MS + 6_505,
            pause_offset_ms: 1_500,
        },
        Schedule("track-3", 480_000),
        Schedule("track-4", 528_000),
        // A second pause of 250 ms.
        Poll {
            now_ms: START_MS + 11_499,
            pause_offset_ms: 1_750,
        },
        // Late wake-up: both are overdue and come back together, in order.
        Poll {
            now_ms: START_MS + 13_000,
            pause_offset_ms: 1_750,
        },
        Schedule("track-5", 576_000),
        Schedule("track-6", 624_000),
        // The stream fails with audio sent up to sample 576 000.
        WithdrawFrom(576_000),
        Poll {
            now_ms: u64::MAX,
            pause_offset_ms: 1_750,
        },
    ]
}

fn pending(entry_id: &str, first_sample_index: u64) -> PendingHostBoundary {
    PendingHostBoundary {
        entry_id: entry_id.to_owned(),
        first_sample_index,
        display_name: format!("{entry_id} title"),
    }
}

fn names(boundaries: Vec<FfiHostBoundary>) -> Vec<String> {
    boundaries
        .into_iter()
        .map(|boundary| boundary.entry_id)
        .collect()
}

fn core_names(boundaries: Vec<PendingHostBoundary>) -> Vec<String> {
    boundaries
        .into_iter()
        .map(|boundary| boundary.entry_id)
        .collect()
}

#[test]
fn the_android_and_desktop_hosts_apply_the_same_boundaries_at_the_same_times() {
    let android =
        FfiHostBoundarySchedule::new(START_MS, CANONICAL_SAMPLE_RATE_HZ).expect("android");
    let mut desktop = HostBoundarySchedule::at_canonical_rate(START_MS);
    let mut android_log = Vec::new();
    let mut desktop_log = Vec::new();
    for step in scenario() {
        match step {
            Step::Schedule(entry_id, sample) => {
                android
                    .schedule(entry_id.to_owned(), sample, format!("{entry_id} title"))
                    .expect("android schedule");
                desktop
                    .schedule(pending(entry_id, sample))
                    .expect("desktop schedule");
            }
            Step::Poll {
                now_ms,
                pause_offset_ms,
            } => {
                assert_eq!(
                    android.next_due_at_ms(pause_offset_ms).expect("next"),
                    desktop.next_due_at_ms(pause_offset_ms),
                    "next deadline at {now_ms}"
                );
                android_log.push(names(
                    android.take_due(now_ms, pause_offset_ms).expect("due"),
                ));
                desktop_log.push(core_names(desktop.take_due(now_ms, pause_offset_ms)));
            }
            Step::WithdrawFrom(sample) => {
                android_log.push(names(android.withdraw_from(sample).expect("withdraw")));
                desktop_log.push(core_names(desktop.withdraw_from(sample)));
            }
        }
    }
    assert_eq!(android_log, desktop_log);
    let expected: Vec<Vec<&str>> = vec![
        vec![],                     // 1 ms before track 2
        vec![],                     // the pause moved it
        vec![],                     // 1 ms before the moved deadline
        vec!["track-2"],            // 5 005 + 1 500
        vec![],                     // track 3 due at 10 000 + 1 750
        vec!["track-3", "track-4"], // overdue together, in order
        vec!["track-5", "track-6"], // withdrawn after the failure
        vec![],                     // and never applied
    ];
    assert_eq!(android_log, expected);
}

#[test]
fn the_android_object_refuses_what_the_core_refuses() {
    assert!(matches!(
        FfiHostBoundarySchedule::new(START_MS, 0),
        Err(crate::host_playlist::FfiHostPlaylistError::InvalidConfiguration(_))
    ));
    let schedule = FfiHostBoundarySchedule::new(START_MS, 48_000).expect("schedule");
    schedule
        .schedule("a".to_owned(), 960, "A".to_owned())
        .expect("a");
    let refused = schedule.schedule("b".to_owned(), 960, "B".to_owned());
    assert!(
        matches!(&refused, Err(crate::host_playlist::FfiHostPlaylistError::InvalidConfiguration(message)) if message.contains("does not follow")),
        "{refused:?}"
    );
    assert_eq!(
        schedule.pending().expect("pending"),
        vec![FfiHostBoundary {
            entry_id: "a".to_owned(),
            first_sample_index: 960,
            display_name: "A".to_owned(),
        }]
    );
    schedule.clear().expect("clear");
    assert!(schedule.pending().expect("pending").is_empty());
    assert_eq!(schedule.next_due_at_ms(0).expect("next"), None);
}
