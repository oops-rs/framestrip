//! JPEG-encode decoded pixels, and assemble the public [`Frame`] from an already-encoded
//! kept frame.

use image::codecs::jpeg::JpegEncoder;
use image::{ImageBuffer, Rgb};

use crate::decode::EncodedFrame;
use crate::select::Sampled;
use crate::{Error, Frame};

/// JPEG-encode one decoded frame's pixels at `jpeg_quality`. Called by
/// [`crate::decode`] the moment a frame is kept, so its raw pixels never outlive this
/// call.
///
/// # Errors
/// [`Error::Undecodable`] if the encoder rejects the pixels (in practice this should
/// not happen: the buffer was already validated when it was decoded).
pub(crate) fn encode_jpeg(
    image: &ImageBuffer<Rgb<u8>, Vec<u8>>,
    jpeg_quality: u8,
) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut bytes, jpeg_quality)
        .encode_image(image)
        .map_err(|source| Error::Undecodable(format!("jpeg encode failed: {source}")))?;
    Ok(bytes)
}

/// Assemble the final, public [`Frame`] from an already-encoded kept frame plus the
/// timing and distance metadata the selection pass computed for it.
pub(crate) fn to_frame(
    index: usize,
    sampled: Sampled<EncodedFrame>,
    held_ms: u64,
    distance_from_previous: u32,
) -> Frame {
    let EncodedFrame {
        bytes,
        width,
        height,
    } = sampled.payload;
    Frame {
        index,
        ts_ms: sampled.ts_ms,
        held_ms,
        width,
        height,
        media_type: "image/jpeg",
        bytes,
        distance_from_previous,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use image::{ImageBuffer, Rgb};

    use super::{encode_jpeg, to_frame};
    use crate::decode::EncodedFrame;
    use crate::select::{Hash, Sampled};

    #[test]
    fn encode_jpeg_produces_valid_jpeg_bytes() {
        let image: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(4, 4, |x, y| {
            Rgb([
                u8::try_from(x * 16).unwrap_or(255),
                u8::try_from(y * 16).unwrap_or(255),
                0,
            ])
        });

        let bytes = encode_jpeg(&image, 80).expect("valid rgb buffer encodes");

        // JPEG files start with the SOI marker.
        assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
    }

    #[test]
    fn to_frame_assembles_from_an_already_encoded_frame() {
        let hash = Hash::from_bytes(&[0]).expect("Box<[u8]> hashes accept any byte length");
        let sampled = Sampled {
            ts_ms: 1200,
            hash,
            payload: EncodedFrame {
                bytes: vec![1, 2, 3],
                width: 2,
                height: 2,
            },
        };

        let frame = to_frame(2, sampled, 500, 7);

        assert_eq!(frame.index, 2);
        assert_eq!(frame.ts_ms, 1200);
        assert_eq!(frame.held_ms, 500);
        assert_eq!(frame.distance_from_previous, 7);
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 2);
        assert_eq!(frame.media_type, "image/jpeg");
        assert_eq!(frame.bytes, vec![1, 2, 3]);
    }
}
