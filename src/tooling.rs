//! Locate and verify the system `ffmpeg`/`ffprobe` binaries.
//!
//! The check runs once per process (`ffmpeg -version` / `ffprobe -version`) and the
//! result is cached; every later call to [`tooling`] returns the cached outcome.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use crate::Error;

/// The resolved decoder binaries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tooling {
    /// Path of the `ffmpeg` binary.
    pub ffmpeg: PathBuf,
    /// Path of the `ffprobe` binary.
    pub ffprobe: PathBuf,
    /// First line of `ffmpeg -version`.
    pub version: String,
}

static TOOLING: OnceLock<Result<Tooling, String>> = OnceLock::new();

/// Locate and verify `ffmpeg`/`ffprobe` once; later calls return the cached result.
///
/// # Errors
/// [`Error::ToolingUnavailable`] when either binary is missing or does not run.
pub fn tooling() -> Result<Tooling, Error> {
    TOOLING
        .get_or_init(probe_tooling)
        .clone()
        .map_err(Error::ToolingUnavailable)
}

fn probe_tooling() -> Result<Tooling, String> {
    let ffmpeg = PathBuf::from("ffmpeg");
    let ffprobe = PathBuf::from("ffprobe");

    let ffmpeg_version = run_version(&ffmpeg).map_err(|source| format!("ffmpeg: {source}"))?;
    run_version(&ffprobe).map_err(|source| format!("ffprobe: {source}"))?;

    let version = ffmpeg_version.lines().next().unwrap_or_default().to_owned();
    Ok(Tooling {
        ffmpeg,
        ffprobe,
        version,
    })
}

/// Run `<bin> -version` and return its stdout, or a message explaining why it could not.
fn run_version(bin: &Path) -> Result<String, String> {
    let output = Command::new(bin)
        .arg("-version")
        .stdin(Stdio::null())
        .output()
        .map_err(|source| source.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("exited with {}: {}", output.status, stderr.trim()));
    }

    String::from_utf8(output.stdout).map_err(|source| source.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn tooling_is_cached_across_calls() {
        // Whatever the outcome on this machine, two calls must agree: the
        // cache, not a fresh probe, answers the second call.
        let first = tooling();
        let second = tooling();
        assert_eq!(first.is_ok(), second.is_ok());
        if let (Ok(a), Ok(b)) = (first, second) {
            assert_eq!(a, b);
        }
    }
}
