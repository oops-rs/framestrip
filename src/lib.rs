//! Turn a video into a short, deduplicated, timestamped frame strip.
//!
//! Decodes through the system `ffmpeg` binary (`ffmpeg-sidecar`), samples at a
//! fixed rate, perceptually hashes each sampled frame, keeps a frame only when
//! it differs from the last kept one, thins to a hard cap by dropping the
//! smallest transitions, and returns each kept frame as JPEG with its source
//! timestamp and hold duration.
//!
//! API stub — see the spec (nous `docs/spec/2026-09-22-video-evidence.md` §2).

use std::path::Path;
use std::time::Duration;

/// Extraction parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// Frames sampled per second of source before deduplication.
    pub sample_fps: f32,
    /// Width of emitted frames; height follows the aspect ratio (even).
    pub max_width: u32,
    /// Minimum Hamming distance (8×8 DoubleGradient hash) for a frame to count as new.
    pub hash_threshold: u32,
    /// Hard cap on emitted frames; the smallest transitions are dropped first.
    pub max_frames: usize,
    /// A kept frame held shorter than this between near-identical neighbours is flicker.
    pub min_hold_ms: u64,
    /// JPEG quality of emitted frames (1–100).
    pub jpeg_quality: u8,
    /// Wall-clock bound for the decode child process.
    pub timeout: Duration,
    /// Reject inputs whose probed duration exceeds this.
    pub max_duration_ms: Option<u64>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            sample_fps: 4.0,
            max_width: 640,
            hash_threshold: 4,
            max_frames: 24,
            min_hold_ms: 250,
            jpeg_quality: 80,
            timeout: Duration::from_secs(60),
            max_duration_ms: Some(600_000),
        }
    }
}

/// What `ffprobe` reported about the input.
#[derive(Clone, Debug, PartialEq)]
pub struct Probe {
    /// Container duration, when known.
    pub duration_ms: Option<u64>,
    /// Source width in pixels.
    pub width: u32,
    /// Source height in pixels.
    pub height: u32,
    /// Video codec name, e.g. `h264` or `hevc`.
    pub codec: String,
    /// ffprobe `format_name`, e.g. `mov,mp4,m4a,3gp,3g2,mj2`.
    pub container: String,
    /// Average source frame rate, when known.
    pub avg_fps: Option<f32>,
}

/// One kept frame.
#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    /// Position in the strip, 0-based.
    pub index: usize,
    /// Source timestamp of the sampled frame.
    pub ts_ms: u64,
    /// How long this screen stayed on: next kept `ts_ms` minus this one; last frame runs to the end.
    pub held_ms: u64,
    /// Emitted width.
    pub width: u32,
    /// Emitted height.
    pub height: u32,
    /// Always `image/jpeg`.
    pub media_type: &'static str,
    /// JPEG bytes.
    pub bytes: Vec<u8>,
    /// Hash distance to the previous kept frame; 0 for the first.
    pub distance_from_previous: u32,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame")
            .field("index", &self.index)
            .field("ts_ms", &self.ts_ms)
            .field("held_ms", &self.held_ms)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("byte_size", &self.bytes.len())
            .field("distance_from_previous", &self.distance_from_previous)
            .finish()
    }
}

/// The extraction result.
#[derive(Clone, Debug, PartialEq)]
pub struct Strip {
    /// What the input looked like.
    pub probe: Probe,
    /// Kept frames in time order.
    pub frames: Vec<Frame>,
    /// Frames decoded at `sample_fps` before selection.
    pub sampled: usize,
    /// Sampled frames dropped as near-duplicates of the last kept frame.
    pub dropped_as_duplicate: usize,
    /// Kept frames dropped to honour `max_frames`.
    pub dropped_by_cap: usize,
}

/// Why extraction did not produce a strip.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// `ffmpeg` or `ffprobe` is missing from `PATH` or cannot run.
    #[error("video tooling unavailable: {0}")]
    ToolingUnavailable(String),
    /// The input is not a video ffmpeg can read.
    #[error("undecodable video: {0}")]
    Undecodable(String),
    /// The probed duration exceeds `Options::max_duration_ms`.
    #[error("video is {duration_ms} ms long, above the {max_ms} ms limit")]
    TooLong {
        /// Probed duration.
        duration_ms: u64,
        /// Configured limit.
        max_ms: u64,
    },
    /// The decode child outlived `Options::timeout`.
    #[error("video decode exceeded {0:?}")]
    Timeout(Duration),
    /// A file or pipe error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The options are unusable (non-positive rate, zero cap, …).
    #[error("invalid options: {0}")]
    Options(String),
}

/// The resolved decoder binaries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tooling {
    /// Path of the `ffmpeg` binary.
    pub ffmpeg: std::path::PathBuf,
    /// Path of the `ffprobe` binary.
    pub ffprobe: std::path::PathBuf,
    /// First line of `ffmpeg -version`.
    pub version: String,
}

/// Locate and verify `ffmpeg`/`ffprobe` once; later calls return the cached result.
///
/// # Errors
/// [`Error::ToolingUnavailable`] when either binary is missing or does not run.
pub fn tooling() -> Result<Tooling, Error> {
    Err(Error::ToolingUnavailable("not implemented".to_owned()))
}

/// Probe a video without decoding it.
///
/// # Errors
/// [`Error::ToolingUnavailable`], [`Error::Undecodable`], [`Error::Io`].
pub fn probe(path: &Path) -> Result<Probe, Error> {
    let _ = path;
    Err(Error::ToolingUnavailable("not implemented".to_owned()))
}

/// Decode, sample, deduplicate, cap, and encode. Blocking; run it off any async executor.
///
/// # Errors
/// Every [`Error`] variant.
pub fn extract(path: &Path, options: &Options) -> Result<Strip, Error> {
    let _ = (path, options);
    Err(Error::ToolingUnavailable("not implemented".to_owned()))
}
