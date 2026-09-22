//! Synthetic-hash unit tests for the pure selection helpers. No `ffmpeg` involved: hashes
//! are crafted byte patterns so distances are known in advance.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::{Hash, Sampled, distances_from_previous, hold_durations, keep, merge_flicker, thin};

/// Build a single-byte synthetic hash. Hamming distance between two of these is the
/// popcount of their XOR, i.e. 0..=8.
fn hash(byte: u8) -> Hash {
    Hash::from_bytes(&[byte]).expect("Box<[u8]> hashes accept any byte length")
}

fn sample(ts_ms: u64, byte: u8) -> Sampled<()> {
    Sampled {
        ts_ms,
        hash: hash(byte),
        payload: (),
    }
}

// ---- keep ----

#[test]
fn keep_always_keeps_the_first_frame() {
    let (kept, dropped) = keep(vec![sample(0, 0x00)], 4);
    assert_eq!(kept.len(), 1);
    assert_eq!(dropped, 0);
}

#[test]
fn keep_handles_empty_input() {
    let (kept, dropped) = keep(Vec::<Sampled<()>>::new(), 4);
    assert!(kept.is_empty());
    assert_eq!(dropped, 0);
}

#[test]
fn keep_drops_near_duplicates_of_the_last_kept_frame() {
    // 0x00, 0x00, 0x00: every later frame is identical to the last kept one (distance 0).
    let sampled = vec![sample(0, 0x00), sample(100, 0x00), sample(200, 0x00)];
    let (kept, dropped) = keep(sampled, 4);
    assert_eq!(kept.iter().map(|s| s.ts_ms).collect::<Vec<_>>(), vec![0]);
    assert_eq!(dropped, 2);
}

#[test]
fn keep_keeps_a_frame_whose_distance_meets_the_threshold() {
    // dist(0x00, 0xFF) == 8 >= 4.
    let sampled = vec![sample(0, 0x00), sample(100, 0xFF)];
    let (kept, dropped) = keep(sampled, 4);
    assert_eq!(
        kept.iter().map(|s| s.ts_ms).collect::<Vec<_>>(),
        vec![0, 100]
    );
    assert_eq!(dropped, 0);
}

#[test]
fn keep_compares_against_the_last_kept_frame_not_the_first() {
    // A(0x00) -> B(0x0F, dist 4 from A, kept) -> C(0x00, dist 4 from B, kept).
    // If `keep` mistakenly compared against the *first* frame instead of the *last
    // kept* one, C (identical to A) would be dropped.
    let sampled = vec![sample(0, 0x00), sample(100, 0x0F), sample(200, 0x00)];
    let (kept, dropped) = keep(sampled, 4);
    assert_eq!(
        kept.iter().map(|s| s.ts_ms).collect::<Vec<_>>(),
        vec![0, 100, 200]
    );
    assert_eq!(dropped, 0);
}

// ---- hold_durations ----

#[test]
fn hold_durations_spans_to_the_next_kept_frame() {
    let kept = vec![sample(0, 0x00), sample(1000, 0x11), sample(2500, 0x22)];
    let holds = hold_durations(&kept, Some(4000));
    assert_eq!(holds, vec![1000, 1500, 1500]);
}

#[test]
fn hold_durations_last_frame_runs_to_zero_when_duration_is_unknown() {
    let kept = vec![sample(0, 0x00), sample(1000, 0x11)];
    let holds = hold_durations(&kept, None);
    assert_eq!(holds, vec![1000, 0]);
}

#[test]
fn hold_durations_handles_a_single_frame() {
    let kept = vec![sample(0, 0x00)];
    assert_eq!(hold_durations(&kept, Some(5000)), vec![5000]);
    assert_eq!(hold_durations(&kept, None), vec![0]);
}

// ---- distances_from_previous ----

#[test]
fn distances_from_previous_first_frame_is_zero() {
    let kept = vec![sample(0, 0xFF)];
    assert_eq!(distances_from_previous(&kept), vec![0]);
}

#[test]
fn distances_from_previous_matches_hamming_distance() {
    let kept = vec![sample(0, 0x00), sample(1, 0x0F), sample(2, 0xFF)];
    // dist(0x00, 0x0F) = 4, dist(0x0F, 0xFF) = 4.
    assert_eq!(distances_from_previous(&kept), vec![0, 4, 4]);
}

// ---- merge_flicker ----

#[test]
fn merge_flicker_drops_a_brief_return_to_the_same_screen() {
    // A(0x00) at 0ms, B(0xFF) at 100ms (held only 50ms), A again at 150ms.
    // B's predecessor and successor are identical, and its hold is short: flicker.
    let kept = vec![sample(0, 0x00), sample(100, 0xFF), sample(150, 0x00)];
    let (kept, dropped) = merge_flicker(kept, 4, 250, Some(5000));
    assert_eq!(
        kept.iter().map(|s| s.ts_ms).collect::<Vec<_>>(),
        vec![0, 150]
    );
    assert_eq!(dropped, 1);
}

#[test]
fn merge_flicker_keeps_a_real_change_even_if_brief() {
    // A(0x00) -> B(0x0F) -> C(0xFF): predecessor and successor are NOT alike
    // (dist(0x00, 0xFF) = 8 >= threshold), so B is a genuine change, not flicker.
    let kept = vec![sample(0, 0x00), sample(100, 0x0F), sample(150, 0xFF)];
    let (kept, dropped) = merge_flicker(kept, 4, 250, Some(5000));
    assert_eq!(kept.len(), 3);
    assert_eq!(dropped, 0);
}

#[test]
fn merge_flicker_keeps_a_frame_held_long_enough() {
    // Same shape as the dropped case, but B is held far past `min_hold_ms`.
    let kept = vec![sample(0, 0x00), sample(100, 0xFF), sample(5000, 0x00)];
    let (kept, dropped) = merge_flicker(kept, 4, 250, Some(6000));
    assert_eq!(kept.len(), 3);
    assert_eq!(dropped, 0);
}

#[test]
fn merge_flicker_cascades_until_no_pass_finds_one() {
    // A, B, A, B, A all held ~50ms: each pass exposes a new flicker frame once its
    // neighbours become identical, down to the two protected end frames.
    let kept = vec![
        sample(0, 0x00),
        sample(50, 0xFF),
        sample(100, 0x00),
        sample(150, 0xFF),
        sample(200, 0x00),
    ];
    let (kept, dropped) = merge_flicker(kept, 4, 300, Some(1000));
    assert_eq!(
        kept.iter().map(|s| s.ts_ms).collect::<Vec<_>>(),
        vec![0, 200]
    );
    assert_eq!(dropped, 3);
}

#[test]
fn merge_flicker_never_touches_a_two_frame_list() {
    let kept = vec![sample(0, 0x00), sample(10, 0xFF)];
    let (kept, dropped) = merge_flicker(kept, 4, 10_000, Some(20));
    assert_eq!(kept.len(), 2);
    assert_eq!(dropped, 0);
}

// ---- thin ----

#[test]
fn thin_is_a_no_op_within_the_cap() {
    let kept = vec![sample(0, 0x00), sample(1, 0x11), sample(2, 0x22)];
    let (kept, dropped) = thin(kept, 5);
    assert_eq!(kept.len(), 3);
    assert_eq!(dropped, 0);
}

#[test]
fn thin_drops_the_smallest_transition_first() {
    // distances_from_previous: [0, 1, 3, 4]. The smallest interior distance (index 1,
    // dist 1) goes first, landing exactly on the cap.
    let kept = vec![
        sample(0, 0x00),
        sample(1, 0x10),
        sample(2, 0xF0),
        sample(3, 0xFF),
    ];
    let (kept, dropped) = thin(kept, 3);
    assert_eq!(
        kept.iter().map(|s| s.ts_ms).collect::<Vec<_>>(),
        vec![0, 2, 3]
    );
    assert_eq!(dropped, 1);
}

#[test]
fn thin_always_keeps_the_first_and_last_frame() {
    let kept = vec![
        sample(0, 0x00),
        sample(1, 0x10),
        sample(2, 0x20),
        sample(3, 0x30),
    ];
    let (kept, dropped) = thin(kept, 2);
    assert_eq!(kept.first().map(|s| s.ts_ms), Some(0));
    assert_eq!(kept.last().map(|s| s.ts_ms), Some(3));
    assert_eq!(dropped, 2);
}

#[test]
fn thin_stops_rather_than_drop_the_first_or_last_frame() {
    // Only 3 frames and a cap of 1: the loop can strip the single interior frame down
    // to a 2-frame list, then must stop rather than remove either protected end.
    let kept = vec![sample(0, 0x00), sample(1, 0x11), sample(2, 0x22)];
    let (kept, dropped) = thin(kept, 1);
    assert_eq!(kept.len(), 2);
    assert_eq!(kept.first().map(|s| s.ts_ms), Some(0));
    assert_eq!(kept.last().map(|s| s.ts_ms), Some(2));
    assert_eq!(dropped, 1);
}
