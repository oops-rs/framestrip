//! Pure frame-selection helpers: which sampled frames survive, and for how long.
//!
//! Every function here takes ownership of (or borrows) a list of [`Sampled`] items and
//! returns a new list; none of them touch `ffmpeg`, a file, or the network, so they are
//! unit-tested with synthetic hashes in [`tests`].

use image_hasher::ImageHash;

/// The hash type used throughout selection: an 8×8 `DoubleGradient` perceptual hash.
pub(crate) type Hash = ImageHash<Box<[u8]>>;

/// One sampled frame carrying just enough to decide whether it survives, plus an
/// opaque `payload` (raw pixels in the real pipeline, `()` in tests).
#[derive(Clone, Debug)]
pub(crate) struct Sampled<P> {
    pub ts_ms: u64,
    pub hash: Hash,
    pub payload: P,
}

/// Keep the first frame, and any later frame whose hash differs from the **last kept**
/// frame by at least `threshold`. Returns the survivors and how many were dropped.
pub(crate) fn keep<P>(sampled: Vec<Sampled<P>>, threshold: u32) -> (Vec<Sampled<P>>, usize) {
    let mut kept: Vec<Sampled<P>> = Vec::with_capacity(sampled.len());
    let mut dropped = 0usize;
    for candidate in sampled {
        let is_new = match kept.last() {
            None => true,
            Some(last) => last.hash.dist(&candidate.hash) >= threshold,
        };
        if is_new {
            kept.push(candidate);
        } else {
            dropped += 1;
        }
    }
    (kept, dropped)
}

/// `held_ms` for each kept frame: the next frame's `ts_ms` minus this one's; the last
/// frame runs to `duration_ms` (0 when the duration is unknown).
pub(crate) fn hold_durations<P>(kept: &[Sampled<P>], duration_ms: Option<u64>) -> Vec<u64> {
    kept.iter()
        .enumerate()
        .map(|(index, frame)| match kept.get(index + 1) {
            Some(next) => next.ts_ms.saturating_sub(frame.ts_ms),
            None => duration_ms.map_or(0, |duration| duration.saturating_sub(frame.ts_ms)),
        })
        .collect()
}

/// Hash distance from each kept frame to its predecessor; 0 for the first frame.
pub(crate) fn distances_from_previous<P>(kept: &[Sampled<P>]) -> Vec<u32> {
    kept.iter()
        .enumerate()
        .map(|(index, frame)| match index {
            0 => 0,
            _ => kept[index - 1].hash.dist(&frame.hash),
        })
        .collect()
}

/// Drop flicker: an interior kept frame (never the first or last) whose `held_ms` is
/// under `min_hold_ms` and whose successor is within `threshold` of its predecessor is a
/// momentary blip rather than a real screen change. Repeats until a full pass finds none,
/// since removing one flicker frame can expose another (e.g. A, B, A, B, C with B's held
/// time short on both sides).
pub(crate) fn merge_flicker<P>(
    mut kept: Vec<Sampled<P>>,
    threshold: u32,
    min_hold_ms: u64,
    duration_ms: Option<u64>,
) -> (Vec<Sampled<P>>, usize) {
    let mut dropped = 0usize;
    loop {
        if kept.len() < 3 {
            break;
        }
        let holds = hold_durations(&kept, duration_ms);
        let flicker_index = (1..kept.len() - 1).find(|&index| {
            holds[index] < min_hold_ms
                && kept[index - 1].hash.dist(&kept[index + 1].hash) < threshold
        });
        match flicker_index {
            Some(index) => {
                kept.remove(index);
                dropped += 1;
            }
            None => break,
        }
    }
    (kept, dropped)
}

/// Thin down to `max_frames` by repeatedly dropping the interior kept frame (never the
/// first or the last) with the smallest distance to its current predecessor, so
/// continuous motion keeps its biggest transitions.
pub(crate) fn thin<P>(mut kept: Vec<Sampled<P>>, max_frames: usize) -> (Vec<Sampled<P>>, usize) {
    let mut dropped = 0usize;
    while kept.len() > max_frames && kept.len() > 2 {
        let distances = distances_from_previous(&kept);
        let smallest = (1..kept.len() - 1).min_by_key(|&index| distances[index]);
        match smallest {
            Some(index) => {
                kept.remove(index);
                dropped += 1;
            }
            // No interior frame left to drop (only the first and last remain); stop
            // rather than violate "never the first, never the last".
            None => break,
        }
    }
    (kept, dropped)
}

#[cfg(test)]
mod tests;
