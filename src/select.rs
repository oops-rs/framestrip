//! Pure, `ffmpeg`-free frame-selection logic: which sampled frames survive a stream, and
//! for how long.
//!
//! [`Dedupe`] is driven one hash at a time (by [`crate::decode`], as frames are decoded,
//! so no raw pixel buffer outlives the frame that produced it) and is unit-tested the
//! same way. Every other item here takes ownership of (or borrows) a list of [`Sampled`]
//! items and returns a new list; none of them touch `ffmpeg`, a file, or the network, so
//! they are unit-tested with synthetic hashes in [`tests`].

use image_hasher::ImageHash;

/// The hash type used throughout selection: an 8×8 `DoubleGradient` perceptual hash.
pub(crate) type Hash = ImageHash<Box<[u8]>>;

/// One sampled frame carrying just enough to decide whether it survives, plus an
/// opaque `payload` (an encoded JPEG plus its dimensions in the real pipeline, `()` in
/// tests).
#[derive(Clone, Debug)]
pub(crate) struct Sampled<P> {
    pub ts_ms: u64,
    pub hash: Hash,
    pub payload: P,
}

/// Streaming near-duplicate filter: keeps the first hash it sees, and any later one
/// that differs from the last **kept** hash by at least `threshold`. Holding only the
/// last kept hash (not the whole history) is what lets [`crate::decode`] decide
/// keep-or-drop one frame at a time, before it has to hold more than one frame's pixels
/// in memory.
pub(crate) struct Dedupe {
    threshold: u32,
    last_kept: Option<Hash>,
}

impl Dedupe {
    pub(crate) fn new(threshold: u32) -> Self {
        Self {
            threshold,
            last_kept: None,
        }
    }

    /// Returns `true` when `hash` should be kept, and remembers it as the new "last
    /// kept" hash that later candidates are compared against.
    pub(crate) fn consider(&mut self, hash: &Hash) -> bool {
        let is_new = match &self.last_kept {
            None => true,
            Some(last) => last.dist(hash) >= self.threshold,
        };
        if is_new {
            self.last_kept = Some(hash.clone());
        }
        is_new
    }
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

/// If `kept` has grown past `cap`, thin it back down right away using the same rule as
/// the final pass (never the first frame, never the current last). Called by
/// [`crate::decode`] after every frame it keeps, so a long clip cannot accumulate
/// encoded frames without bound while it is still being decoded. Returns the
/// possibly-thinned list and how many frames this call dropped (always 0 below `cap`).
pub(crate) fn thin_incremental<P>(kept: Vec<Sampled<P>>, cap: usize) -> (Vec<Sampled<P>>, usize) {
    if kept.len() > cap {
        thin(kept, cap)
    } else {
        (kept, 0)
    }
}

#[cfg(test)]
mod tests;
