//! JPEG-encode the frames selection kept.

use image::codecs::jpeg::JpegEncoder;

use crate::decode::RawFrame;
use crate::select::Sampled;
use crate::{Error, Frame};

/// Encode one surviving sampled frame into its final [`Frame`].
///
/// # Errors
/// [`Error::Undecodable`] if the JPEG encoder rejects the decoded pixels (in practice
/// this should not happen: the buffer was already validated by [`crate::decode`]).
pub(crate) fn to_frame(
    index: usize,
    sampled: Sampled<RawFrame>,
    held_ms: u64,
    distance_from_previous: u32,
    jpeg_quality: u8,
) -> Result<Frame, Error> {
    let image = sampled.payload.image;
    let (width, height) = (image.width(), image.height());

    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut bytes, jpeg_quality)
        .encode_image(&image)
        .map_err(|source| Error::Undecodable(format!("jpeg encode failed: {source}")))?;

    Ok(Frame {
        index,
        ts_ms: sampled.ts_ms,
        held_ms,
        width,
        height,
        media_type: "image/jpeg",
        bytes,
        distance_from_previous,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use image::{ImageBuffer, Rgb};
    use image_hasher::{HashAlg, HasherConfig};

    use super::to_frame;
    use crate::decode::RawFrame;
    use crate::select::Sampled;

    #[test]
    fn to_frame_encodes_valid_jpeg_bytes() {
        let image: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(4, 4, |x, y| {
            Rgb([
                u8::try_from(x * 16).unwrap_or(255),
                u8::try_from(y * 16).unwrap_or(255),
                0,
            ])
        });
        let hasher = HasherConfig::new()
            .hash_alg(HashAlg::DoubleGradient)
            .hash_size(8, 8)
            .to_hasher();
        let hash = hasher.hash_image(&image);
        let sampled = Sampled {
            ts_ms: 1200,
            hash,
            payload: RawFrame { image },
        };

        let frame = to_frame(2, sampled, 500, 7, 80).expect("valid rgb buffer encodes");

        assert_eq!(frame.index, 2);
        assert_eq!(frame.ts_ms, 1200);
        assert_eq!(frame.held_ms, 500);
        assert_eq!(frame.distance_from_previous, 7);
        assert_eq!(frame.width, 4);
        assert_eq!(frame.height, 4);
        assert_eq!(frame.media_type, "image/jpeg");
        // JPEG files start with the SOI marker.
        assert_eq!(&frame.bytes[..2], &[0xFF, 0xD8]);
    }
}
