//! Decode a video and, for every frame worth keeping, hash and JPEG-encode it on the
//! spot through `ffmpeg-sidecar`.
//!
//! No raw pixel buffer outlives the decode iteration that produced it: a sampled frame
//! is either dropped as a near-duplicate (its pixels never leave this loop) or encoded
//! immediately into an [`EncodedFrame`], and the in-flight kept list is itself
//! periodically thinned (see [`select::thin_incremental`]) so a long, high-motion clip
//! cannot accumulate thousands of encoded frames either.
//!
//! Deliberately does **not** pass `-hide_banner`/`-loglevel error`: `ffmpeg-sidecar`'s
//! log parser depends on the `level+info` prefix it sets in [`FfmpegCommand::new_with_path`],
//! and overriding `-loglevel` on the command line replaces that value wholesale, breaking
//! metadata parsing. This mirrors the working spike in the proposal appendix.

use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use ffmpeg_sidecar::child::FfmpegChild;
use ffmpeg_sidecar::command::FfmpegCommand;
use ffmpeg_sidecar::event::{FfmpegEvent, LogLevel};
use image::{ImageBuffer, Rgb};
use image_hasher::{HashAlg, HasherConfig};

use crate::select::{Dedupe, Sampled, thin_incremental};
use crate::tooling::tooling;
use crate::{Error, Options, encode};

/// Multiplier applied to `Options::max_frames` for the incremental in-flight cap: kept
/// frames accumulate at most this many times the final target before being thinned back
/// down mid-decode, bounding memory on a long or high-motion clip without thinning so
/// aggressively early on that the final pass loses good candidates.
const INCREMENTAL_CAP_MULTIPLIER: usize = 8;

/// One kept frame's encoded pixels: a JPEG plus the dimensions it was encoded at.
pub(crate) struct EncodedFrame {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// What decoding produced: the kept, already-encoded frames plus the counts needed for
/// [`crate::Strip`].
pub(crate) struct DecodeOutput {
    pub kept: Vec<Sampled<EncodedFrame>>,
    pub sampled: usize,
    pub dropped_as_duplicate: usize,
    pub dropped_by_cap: usize,
}

/// Decode `path` at `options.sample_fps`, keeping and JPEG-encoding a frame the moment
/// it is deemed new, and enforcing `options.timeout` with a watchdog thread that kills
/// the `ffmpeg` child.
///
/// # Errors
/// [`Error::ToolingUnavailable`], [`Error::Io`], [`Error::Undecodable`], [`Error::Timeout`].
pub(crate) fn sample_frames(path: &Path, options: &Options) -> Result<DecodeOutput, Error> {
    let tools = tooling()?;
    let hasher = HasherConfig::new()
        .hash_alg(HashAlg::DoubleGradient)
        .hash_size(8, 8)
        .to_hasher();

    let filter = format!("fps={},scale={}:-2", options.sample_fps, options.max_width);
    let mut command = FfmpegCommand::new_with_path(&tools.ffmpeg);
    command
        .input(path.to_string_lossy())
        .args(["-vf", &filter])
        .rawvideo();

    let mut child = command.spawn().map_err(Error::Io)?;
    let events = child
        .iter()
        .map_err(|source| Error::Undecodable(source.to_string()))?;
    let child = Arc::new(Mutex::new(child));

    let watchdog = spawn_watchdog(Arc::clone(&child), options.timeout);

    let incremental_cap = options
        .max_frames
        .saturating_mul(INCREMENTAL_CAP_MULTIPLIER);
    let mut dedupe = Dedupe::new(options.hash_threshold);
    let mut kept: Vec<Sampled<EncodedFrame>> = Vec::new();
    let mut sampled_count = 0usize;
    let mut dropped_as_duplicate = 0usize;
    let mut dropped_by_cap = 0usize;
    let mut last_error: Option<String> = None;
    let mut fatal: Option<Error> = None;

    for event in events {
        match event {
            FfmpegEvent::OutputFrame(frame) => {
                match ImageBuffer::<Rgb<u8>, _>::from_raw(frame.width, frame.height, frame.data) {
                    Some(image) => {
                        sampled_count += 1;
                        let hash = hasher.hash_image(&image);
                        if dedupe.consider(&hash) {
                            match encode::encode_jpeg(&image, options.jpeg_quality) {
                                Ok(bytes) => {
                                    let (width, height) = image.dimensions();
                                    kept.push(Sampled {
                                        ts_ms: timestamp_ms(frame.timestamp),
                                        hash,
                                        payload: EncodedFrame {
                                            bytes,
                                            width,
                                            height,
                                        },
                                    });
                                    let (thinned, cap_dropped) =
                                        thin_incremental(kept, incremental_cap);
                                    kept = thinned;
                                    dropped_by_cap += cap_dropped;
                                }
                                Err(source) => fatal = Some(source),
                            }
                        } else {
                            dropped_as_duplicate += 1;
                        }
                    }
                    None => {
                        last_error = Some(
                            "ffmpeg emitted a frame whose buffer did not match its dimensions"
                                .to_owned(),
                        );
                    }
                }
            }
            FfmpegEvent::Log(LogLevel::Error | LogLevel::Fatal, message)
            | FfmpegEvent::Error(message) => {
                last_error = Some(message);
            }
            FfmpegEvent::Done => break,
            _ => {}
        }
        if fatal.is_some() {
            break;
        }
    }

    // Whatever ended the loop, stop the process rather than risk it blocking on a
    // stdout pipe nobody is draining anymore (we may have `break`ed out early on a
    // fatal encode error). A kill on an already-exited process is a harmless no-op, so
    // its result is intentionally ignored.
    let _ = lock(&child).kill();

    // Dropping the sender lets the watchdog's `recv_timeout` return immediately
    // (as a disconnect, not a timeout) instead of waiting out the full duration.
    drop(watchdog.done_tx);
    let timed_out = watchdog.handle.join().unwrap_or_default();

    // Always reap: an un-awaited `FfmpegChild` is otherwise left as a zombie, on every
    // exit path (timeout, a fatal encode error, or a clean finish).
    let wait_result = lock(&child).wait();

    if timed_out {
        return Err(Error::Timeout(options.timeout));
    }
    if let Some(error) = fatal {
        return Err(error);
    }
    // A `wait()` failure only matters once every more specific error has had its say;
    // masking a real timeout or encode failure with a secondary reap hiccup would be
    // less useful, not more honest.
    wait_result.map_err(Error::Io)?;

    if kept.is_empty() {
        return Err(Error::Undecodable(
            last_error.unwrap_or_else(|| "ffmpeg produced no frames".to_owned()),
        ));
    }

    Ok(DecodeOutput {
        kept,
        sampled: sampled_count,
        dropped_as_duplicate,
        dropped_by_cap,
    })
}

struct Watchdog {
    handle: thread::JoinHandle<bool>,
    done_tx: mpsc::Sender<()>,
}

/// Kill `child` if it outlives `timeout`. Returns immediately, without waiting, once the
/// caller drops the returned sender.
fn spawn_watchdog(child: Arc<Mutex<FfmpegChild>>, timeout: Duration) -> Watchdog {
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let handle = thread::spawn(move || {
        let timed_out = matches!(
            done_rx.recv_timeout(timeout),
            Err(RecvTimeoutError::Timeout)
        );
        if timed_out {
            let _ = lock(&child).kill();
        }
        timed_out
    });
    Watchdog { handle, done_tx }
}

/// Lock `child`, recovering the guard even if a prior holder panicked.
fn lock(child: &Arc<Mutex<FfmpegChild>>) -> MutexGuard<'_, FfmpegChild> {
    match child.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// `ffmpeg-sidecar` reports frame timestamps in seconds as `f32`; convert to whole
/// milliseconds, clamping away negative jitter from the decoder.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn timestamp_ms(timestamp_s: f32) -> u64 {
    let millis = f64::from(timestamp_s).max(0.0) * 1000.0;
    millis.round() as u64
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::timestamp_ms;

    #[test]
    fn timestamp_ms_rounds_and_clamps() {
        assert_eq!(timestamp_ms(0.0), 0);
        assert_eq!(timestamp_ms(1.2345), 1235);
        assert_eq!(timestamp_ms(-0.5), 0);
    }
}
