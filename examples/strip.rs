//! Print a frame-strip summary for a video.
//!
//! ```text
//! cargo run --example strip -- <video> [sample_fps] [hash_threshold]
//! ```

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use keyframe::{Options, extract};

fn main() -> ExitCode {
    let mut args = env::args().skip(1);

    let Some(path) = args.next() else {
        eprintln!("usage: strip <video> [sample_fps] [hash_threshold]");
        return ExitCode::FAILURE;
    };
    let path = PathBuf::from(path);

    let mut options = Options::default();
    if let Some(raw) = args.next() {
        let Ok(sample_fps) = raw.parse::<f32>() else {
            eprintln!("invalid sample_fps: {raw}");
            return ExitCode::FAILURE;
        };
        options.sample_fps = sample_fps;
    }
    if let Some(raw) = args.next() {
        let Ok(hash_threshold) = raw.parse::<u32>() else {
            eprintln!("invalid hash_threshold: {raw}");
            return ExitCode::FAILURE;
        };
        options.hash_threshold = hash_threshold;
    }

    let started = Instant::now();
    let strip = match extract(&path, &options) {
        Ok(strip) => strip,
        Err(error) => {
            eprintln!("extract failed: {error}");
            return ExitCode::FAILURE;
        }
    };
    let elapsed = started.elapsed();

    println!(
        "{}: {}x{} {} ({}), duration={:?}",
        path.display(),
        strip.probe.width,
        strip.probe.height,
        strip.probe.codec,
        strip.probe.container,
        strip.probe.duration_ms.map(Duration::from_millis),
    );
    println!(
        "sampled={} kept={} dropped_as_duplicate={} dropped_by_cap={} wall={elapsed:?}",
        strip.sampled,
        strip.frames.len(),
        strip.dropped_as_duplicate,
        strip.dropped_by_cap,
    );
    for frame in &strip.frames {
        println!(
            "  [{:>3}] t={:>7}ms held={:>7}ms dist={:>3} {}x{} {:>6} bytes",
            frame.index,
            frame.ts_ms,
            frame.held_ms,
            frame.distance_from_previous,
            frame.width,
            frame.height,
            frame.bytes.len(),
        );
    }

    ExitCode::SUCCESS
}
