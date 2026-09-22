//! `ffprobe`-based inspection of a video, without decoding any frames.

use std::path::Path;
use std::process::{Command, Stdio};

use serde::Deserialize;

use crate::Error;
use crate::tooling::tooling;

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

/// Probe a video without decoding it.
///
/// # Errors
/// [`Error::ToolingUnavailable`], [`Error::Undecodable`], [`Error::Io`].
pub fn probe(path: &Path) -> Result<Probe, Error> {
    let tools = tooling()?;

    let output = Command::new(&tools.ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "format=duration,format_name:stream=codec_name,width,height,avg_frame_rate",
            "-of",
            "json",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(Error::Io)?;

    if !output.status.success() {
        return Err(Error::Undecodable(undecodable_message(
            &output.stderr,
            output.status,
        )));
    }

    let parsed: ProbeJson = serde_json::from_slice(&output.stdout).map_err(|source| {
        Error::Undecodable(format!("ffprobe produced unparsable json: {source}"))
    })?;

    let stream = parsed.streams.first();
    let width = stream.and_then(|s| s.width).unwrap_or(0);
    let height = stream.and_then(|s| s.height).unwrap_or(0);
    if width == 0 || height == 0 {
        return Err(Error::Undecodable(
            "ffprobe found no video stream".to_owned(),
        ));
    }

    let codec = stream
        .and_then(|s| s.codec_name.clone())
        .unwrap_or_default();
    let avg_fps = stream
        .and_then(|s| s.avg_frame_rate.as_deref())
        .and_then(parse_rational);

    let container = parsed
        .format
        .as_ref()
        .and_then(|f| f.format_name.clone())
        .unwrap_or_default();
    let duration_ms = parsed
        .format
        .as_ref()
        .and_then(|f| f.duration.as_deref())
        .and_then(|d| d.parse::<f64>().ok())
        .map(duration_seconds_to_ms);

    Ok(Probe {
        duration_ms,
        width,
        height,
        codec,
        container,
        avg_fps,
    })
}

/// A missing file or a container `ffprobe` cannot parse both surface as a non-zero exit;
/// this crate treats that case as [`Error::Undecodable`], never [`Error::Io`], since the
/// `ffprobe` process itself ran successfully and simply rejected the input.
fn undecodable_message(stderr: &[u8], status: std::process::ExitStatus) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let stderr = stderr.trim();
    if stderr.is_empty() {
        format!("ffprobe exited with {status}")
    } else {
        stderr.to_owned()
    }
}

/// ffprobe reports `duration` in fractional seconds; convert to whole milliseconds,
/// clamping away any (unexpected) negative value before the cast.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn duration_seconds_to_ms(seconds: f64) -> u64 {
    (seconds * 1000.0).round().max(0.0) as u64
}

fn parse_rational(raw: &str) -> Option<f32> {
    let (num, den) = raw.split_once('/')?;
    let num: f32 = num.parse().ok()?;
    let den: f32 = den.parse().ok()?;
    if den == 0.0 { None } else { Some(num / den) }
}

#[derive(Debug, Deserialize)]
struct ProbeJson {
    #[serde(default)]
    streams: Vec<StreamJson>,
    #[serde(default)]
    format: Option<FormatJson>,
}

#[derive(Debug, Deserialize)]
struct StreamJson {
    #[serde(default)]
    codec_name: Option<String>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    avg_frame_rate: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FormatJson {
    #[serde(default)]
    format_name: Option<String>,
    #[serde(default)]
    duration: Option<String>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_rational_handles_fraction() {
        assert_eq!(parse_rational("30/1"), Some(30.0));
        assert_eq!(parse_rational("30000/1001"), Some(30000.0 / 1001.0));
    }

    #[test]
    fn parse_rational_rejects_zero_denominator() {
        assert_eq!(parse_rational("0/0"), None);
    }

    #[test]
    fn parse_rational_rejects_garbage() {
        assert_eq!(parse_rational("not-a-fraction"), None);
        assert_eq!(parse_rational("30"), None);
    }

    #[test]
    fn probe_json_defaults_missing_fields() {
        let parsed: ProbeJson = serde_json::from_str("{}").expect("empty object parses");
        assert!(parsed.streams.is_empty());
        assert!(parsed.format.is_none());
    }
}
