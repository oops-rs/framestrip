//! The end-to-end pipeline: probe, decode, select, encode.

use std::path::Path;

use crate::decode::{DecodeOutput, sample_frames};
use crate::probe::probe;
use crate::select::{distances_from_previous, hold_durations, merge_flicker, thin};
use crate::{Error, Options, Strip, encode};

/// Decode, sample, deduplicate, cap, and encode. Blocking; run it off any async executor.
///
/// # Errors
/// Every [`Error`] variant.
pub fn extract(path: &Path, options: &Options) -> Result<Strip, Error> {
    validate(options)?;

    let probe = probe(path)?;
    if let (Some(max_ms), Some(duration_ms)) = (options.max_duration_ms, probe.duration_ms) {
        if duration_ms > max_ms {
            return Err(Error::TooLong {
                duration_ms,
                max_ms,
            });
        }
    }

    let DecodeOutput {
        kept,
        sampled: sampled_count,
        dropped_as_duplicate: dropped_duplicates,
        dropped_by_cap: dropped_by_incremental_cap,
    } = sample_frames(path, options)?;

    let (kept, dropped_flicker) = merge_flicker(
        kept,
        options.hash_threshold,
        options.min_hold_ms,
        probe.duration_ms,
    );
    let (kept, dropped_by_final_cap) = thin(kept, options.max_frames);

    let holds = hold_durations(&kept, probe.duration_ms);
    let distances = distances_from_previous(&kept);

    let frames = kept
        .into_iter()
        .zip(holds)
        .zip(distances)
        .enumerate()
        .map(|(index, ((sampled, held_ms), distance))| {
            encode::to_frame(index, sampled, held_ms, distance)
        })
        .collect();

    Ok(Strip {
        probe,
        frames,
        sampled: sampled_count,
        dropped_as_duplicate: dropped_duplicates + dropped_flicker,
        dropped_by_cap: dropped_by_incremental_cap + dropped_by_final_cap,
    })
}

/// Reject options that cannot produce a sensible strip.
fn validate(options: &Options) -> Result<(), Error> {
    if !options.sample_fps.is_finite() || options.sample_fps <= 0.0 {
        return Err(Error::Options(format!(
            "sample_fps must be a positive, finite number, got {}",
            options.sample_fps
        )));
    }
    if options.max_frames == 0 {
        return Err(Error::Options("max_frames must be at least 1".to_owned()));
    }
    if options.max_width == 0 {
        return Err(Error::Options("max_width must be at least 1".to_owned()));
    }
    if options.jpeg_quality == 0 || options.jpeg_quality > 100 {
        return Err(Error::Options(format!(
            "jpeg_quality must be between 1 and 100, got {}",
            options.jpeg_quality
        )));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::time::Duration;

    use super::validate;
    use crate::{Error, Options};

    fn options() -> Options {
        Options::default()
    }

    #[test]
    fn validate_accepts_defaults() {
        assert!(validate(&options()).is_ok());
    }

    #[test]
    fn validate_rejects_non_positive_sample_fps() {
        let opts = Options {
            sample_fps: 0.0,
            ..options()
        };
        assert!(matches!(validate(&opts), Err(Error::Options(_))));
        let opts = Options {
            sample_fps: -1.0,
            ..options()
        };
        assert!(matches!(validate(&opts), Err(Error::Options(_))));
    }

    #[test]
    fn validate_rejects_non_finite_sample_fps() {
        let opts = Options {
            sample_fps: f32::NAN,
            ..options()
        };
        assert!(matches!(validate(&opts), Err(Error::Options(_))));
        let opts = Options {
            sample_fps: f32::INFINITY,
            ..options()
        };
        assert!(matches!(validate(&opts), Err(Error::Options(_))));
    }

    #[test]
    fn validate_rejects_zero_max_frames() {
        let opts = Options {
            max_frames: 0,
            ..options()
        };
        assert!(matches!(validate(&opts), Err(Error::Options(_))));
    }

    #[test]
    fn validate_rejects_zero_max_width() {
        let opts = Options {
            max_width: 0,
            ..options()
        };
        assert!(matches!(validate(&opts), Err(Error::Options(_))));
    }

    #[test]
    fn validate_rejects_out_of_range_jpeg_quality() {
        let opts = Options {
            jpeg_quality: 0,
            ..options()
        };
        assert!(matches!(validate(&opts), Err(Error::Options(_))));
        let opts = Options {
            jpeg_quality: 101,
            ..options()
        };
        assert!(matches!(validate(&opts), Err(Error::Options(_))));
    }

    #[test]
    fn validate_accepts_a_short_timeout() {
        // Options validation is not the timeout's job: a 1ms timeout is a valid,
        // if aggressive, configuration and surfaces as `Error::Timeout` at decode time.
        let opts = Options {
            timeout: Duration::from_millis(1),
            ..options()
        };
        assert!(validate(&opts).is_ok());
    }
}
