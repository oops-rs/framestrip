//! Decode a video into hashed, timestamped raw frames through `ffmpeg-sidecar`.
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

use crate::select::Sampled;
use crate::tooling::tooling;
use crate::{Error, Options};

/// A sampled frame's raw decoded pixels, kept only until it is either dropped as a
/// duplicate or JPEG-encoded into the final [`crate::Frame`].
pub(crate) struct RawFrame {
    pub image: ImageBuffer<Rgb<u8>, Vec<u8>>,
}

/// Decode `path` at `options.sample_fps`, hashing every sampled frame. Enforces
/// `options.timeout` with a watchdog thread that kills the `ffmpeg` child.
///
/// # Errors
/// [`Error::ToolingUnavailable`], [`Error::Io`], [`Error::Undecodable`], [`Error::Timeout`].
pub(crate) fn sample_frames(
    path: &Path,
    options: &Options,
) -> Result<Vec<Sampled<RawFrame>>, Error> {
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
    let done_tx = watchdog.done_tx;

    let mut sampled: Vec<Sampled<RawFrame>> = Vec::new();
    let mut last_error: Option<String> = None;
    for event in events {
        match event {
            FfmpegEvent::OutputFrame(frame) => {
                match ImageBuffer::<Rgb<u8>, _>::from_raw(frame.width, frame.height, frame.data) {
                    Some(image) => {
                        let hash = hasher.hash_image(&image);
                        sampled.push(Sampled {
                            ts_ms: timestamp_ms(frame.timestamp),
                            hash,
                            payload: RawFrame { image },
                        });
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
    }
    // Dropping the sender lets the watchdog's `recv_timeout` return immediately
    // (as a disconnect, not a timeout) instead of waiting out the full duration.
    drop(done_tx);

    let timed_out = watchdog.handle.join().unwrap_or_default();
    if timed_out {
        return Err(Error::Timeout(options.timeout));
    }

    // Reap the process; an un-awaited `FfmpegChild` is otherwise left as a zombie.
    lock(&child).wait().map_err(Error::Io)?;

    if sampled.is_empty() {
        return Err(Error::Undecodable(
            last_error.unwrap_or_else(|| "ffmpeg produced no frames".to_owned()),
        ));
    }

    Ok(sampled)
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
