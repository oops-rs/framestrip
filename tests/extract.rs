//! Integration tests against real `ffmpeg`-generated fixtures.
//!
//! Every test calls [`framestrip::tooling`] first and skips (with an `eprintln!`, not a
//! failure) when `ffmpeg`/`ffprobe` are unavailable, so CI without them stays green and
//! CI with them exercises the real decode path.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use framestrip::{Error, Options, extract, tooling};

/// Returns `false` (and prints why) when `ffmpeg`/`ffprobe` are not usable here.
fn require_tooling() -> bool {
    if tooling().is_ok() {
        return true;
    }
    eprintln!("skipping: ffmpeg not available");
    false
}

/// Render a `lavfi` test source into `dir/name` with the given codec arguments.
/// Panics (this is test-fixture setup, not the code under test) if `ffmpeg` fails.
fn generate_fixture(dir: &Path, name: &str, lavfi: &str, codec_args: &[&str]) -> PathBuf {
    let out = dir.join(name);
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            lavfi,
        ])
        .args(codec_args)
        .arg(&out)
        .status()
        .expect("spawn ffmpeg to render a fixture");
    assert!(status.success(), "ffmpeg failed to render fixture {name}");
    out
}

/// Five static screens: `testsrc2` evolving at 0.25 Hz over a 20s, 30fps H.264 container.
fn static_screens_fixture(dir: &Path) -> PathBuf {
    generate_fixture(
        dir,
        "static.mp4",
        "testsrc2=size=640x360:rate=0.25:duration=20",
        &["-vf", "fps=30", "-c:v", "libx264", "-pix_fmt", "yuv420p"],
    )
}

/// Continuous motion: `testsrc2` evolving every frame over a 20s, 30fps H.264 container.
fn continuous_motion_fixture(dir: &Path) -> PathBuf {
    generate_fixture(
        dir,
        "motion.mp4",
        "testsrc2=size=640x360:rate=30:duration=20",
        &["-c:v", "libx264", "-pix_fmt", "yuv420p"],
    )
}

/// A short HEVC-encoded clip, tagged `hvc1` the way iPhone recordings are.
fn hevc_fixture(dir: &Path) -> PathBuf {
    generate_fixture(
        dir,
        "hevc.mp4",
        "testsrc2=size=640x360:rate=0.25:duration=6",
        &[
            "-vf", "fps=30", "-c:v", "libx265", "-tag:v", "hvc1", "-pix_fmt", "yuv420p",
        ],
    )
}

#[test]
fn static_screens_collapse_to_a_handful_of_frames() {
    if !require_tooling() {
        return;
    }
    let dir = tempfile::tempdir().expect("create a tempdir for the fixture");
    let video = static_screens_fixture(dir.path());

    let strip = extract(&video, &Options::default()).expect("extract the static-screens clip");

    assert!(
        (3..=8).contains(&strip.frames.len()),
        "expected roughly 5 kept frames for 5 static screens, got {}",
        strip.frames.len()
    );
    assert!(
        strip
            .frames
            .windows(2)
            .all(|pair| pair[0].ts_ms <= pair[1].ts_ms),
        "kept frame timestamps must be non-decreasing: {:?}",
        strip.frames.iter().map(|f| f.ts_ms).collect::<Vec<_>>()
    );

    let held_total: u64 = strip.frames.iter().map(|f| f.held_ms).sum();
    let duration_ms = strip
        .probe
        .duration_ms
        .expect("static fixture reports a duration");
    let delta = duration_ms.abs_diff(held_total);
    assert!(
        delta <= 1000,
        "held_ms should sum to about the clip duration: total={held_total} duration={duration_ms}"
    );
}

#[test]
fn continuous_motion_honours_max_frames_and_keeps_the_ends() {
    if !require_tooling() {
        return;
    }
    let dir = tempfile::tempdir().expect("create a tempdir for the fixture");
    let video = continuous_motion_fixture(dir.path());

    let options = Options {
        max_frames: 8,
        ..Options::default()
    };
    let strip = extract(&video, &options).expect("extract the continuous-motion clip");

    assert_eq!(
        strip.frames.len(),
        options.max_frames,
        "the cap should bind on continuous motion"
    );
    assert!(
        strip.dropped_by_cap > 0,
        "thinning should have dropped frames to reach the cap"
    );
    assert_eq!(strip.frames.first().map(|f| f.index), Some(0));
    assert_eq!(
        strip.frames.last().map(|f| f.index),
        Some(options.max_frames - 1),
        "the last sampled frame must survive thinning"
    );
}

#[test]
fn hevc_decodes() {
    if !require_tooling() {
        return;
    }
    let dir = tempfile::tempdir().expect("create a tempdir for the fixture");
    let video = hevc_fixture(dir.path());

    let strip = extract(&video, &Options::default()).expect("extract the hevc clip");

    assert!(
        strip.probe.codec.contains("hevc") || strip.probe.codec.contains("h265"),
        "expected an hevc codec name, got {:?}",
        strip.probe.codec
    );
    assert!(!strip.frames.is_empty());
}

#[test]
fn a_short_timeout_is_reported_as_timeout() {
    if !require_tooling() {
        return;
    }
    let dir = tempfile::tempdir().expect("create a tempdir for the fixture");
    let video = continuous_motion_fixture(dir.path());

    let options = Options {
        timeout: Duration::from_millis(1),
        ..Options::default()
    };
    let result = extract(&video, &options);

    assert!(
        matches!(result, Err(Error::Timeout(_))),
        "expected Error::Timeout, got {result:?}"
    );
}

#[test]
fn a_missing_file_is_undecodable() {
    if !require_tooling() {
        return;
    }
    let dir = tempfile::tempdir().expect("create a tempdir for a path that stays empty");
    let missing = dir.path().join("does-not-exist.mp4");

    let result = extract(&missing, &Options::default());

    assert!(
        matches!(result, Err(Error::Undecodable(_))),
        "expected Error::Undecodable, got {result:?}"
    );
}

#[test]
fn a_duration_ceiling_below_the_clip_length_is_too_long() {
    if !require_tooling() {
        return;
    }
    let dir = tempfile::tempdir().expect("create a tempdir for the fixture");
    let video = static_screens_fixture(dir.path());

    let options = Options {
        max_duration_ms: Some(1000),
        ..Options::default()
    };
    let result = extract(&video, &options);

    assert!(
        matches!(result, Err(Error::TooLong { .. })),
        "expected Error::TooLong, got {result:?}"
    );
    if let Err(Error::TooLong {
        duration_ms,
        max_ms,
    }) = result
    {
        assert_eq!(max_ms, 1000);
        assert!(duration_ms >= 19_000, "expected ~20s, got {duration_ms}ms");
    }
}
