use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::LazyLock;
use std::sync::Mutex;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use codex_utils_string::approx_bytes_for_tokens;
use image::ColorType;
use image::DynamicImage;
use image::GenericImageView;
use image::ImageEncoder;
use image::ImageFormat;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPEncoder;
use image::imageops::FilterType;
use lru::LruCache;
use sha1::Digest;
use sha1::Sha1;

/// Maximum width or height used when resizing images before uploading.
pub const MAX_DIMENSION: u32 = 2048;

/// Maximum original-detail image patches used by the model-visible byte estimator.
pub const ORIGINAL_IMAGE_MAX_PATCHES: usize = 10_000;

const ORIGINAL_IMAGE_PATCH_SIZE: u32 = 32;
const ORIGINAL_IMAGE_ESTIMATE_CACHE_SIZE: usize = 32;

pub mod error;

pub use crate::error::ImageProcessingError;

#[derive(Debug, Clone)]
pub struct EncodedImage {
    pub bytes: Vec<u8>,
    pub mime: String,
    pub width: u32,
    pub height: u32,
}

impl EncodedImage {
    pub fn into_data_url(self) -> String {
        let encoded = BASE64_STANDARD.encode(&self.bytes);
        format!("data:{};base64,{encoded}", self.mime)
    }
}

pub fn decode_base64_image_bytes(
    payload: impl AsRef<[u8]>,
) -> Result<Vec<u8>, base64::DecodeError> {
    BASE64_STANDARD.decode(payload)
}

/// Returns the base64 payload for inline image data URLs that are eligible for image-cost
/// estimation.
pub fn base64_image_data_url_payload(url: &str) -> Option<&str> {
    if !url
        .get(.."data:".len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:"))
    {
        return None;
    }
    let comma_index = url.find(',')?;
    let metadata = &url[..comma_index];
    let payload = &url[comma_index + 1..];

    let metadata_without_scheme = &metadata["data:".len()..];
    let mut metadata_parts = metadata_without_scheme.split(';');
    let mime_type = metadata_parts.next().unwrap_or_default();
    let has_base64_marker = metadata_parts.any(|part| part.eq_ignore_ascii_case("base64"));
    if !mime_type
        .get(.."image/".len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("image/"))
    {
        return None;
    }
    if !has_base64_marker {
        return None;
    }
    Some(payload)
}

/// Estimates model-visible bytes for a `detail: "original"` inline image data URL.
///
/// Returns `None` when the URL is not a supported base64 image data URL or when image dimensions
/// cannot be decoded.
pub fn estimate_original_image_data_url_bytes(image_url: &str) -> Option<i64> {
    let key = sha1_digest(image_url.as_bytes());
    ORIGINAL_IMAGE_ESTIMATE_CACHE.get_or_insert_with(key, || {
        let payload = match base64_image_data_url_payload(image_url) {
            Some(payload) => payload,
            None => {
                tracing::trace!("skipping original-detail estimate for non-base64 image data URL");
                return None;
            }
        };
        let bytes = match decode_base64_image_bytes(payload) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::trace!("failed to decode original-detail image payload: {error}");
                return None;
            }
        };
        let (width, height) = match dimensions_from_memory(&bytes) {
            Ok(dimensions) => dimensions,
            Err(error) => {
                tracing::trace!("failed to decode original-detail image bytes: {error}");
                return None;
            }
        };
        let width = i64::from(width);
        let height = i64::from(height);
        let patch_size = i64::from(ORIGINAL_IMAGE_PATCH_SIZE);
        let patches_wide = width.saturating_add(patch_size.saturating_sub(1)) / patch_size;
        let patches_high = height.saturating_add(patch_size.saturating_sub(1)) / patch_size;
        let patch_count = patches_wide.saturating_mul(patches_high);
        let patch_count = usize::try_from(patch_count).unwrap_or(usize::MAX);
        let patch_count = patch_count.min(ORIGINAL_IMAGE_MAX_PATCHES);
        Some(i64::try_from(approx_bytes_for_tokens(patch_count)).unwrap_or(i64::MAX))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PromptImageMode {
    ResizeToFit,
    Original,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ImageCacheKey {
    digest: [u8; 20],
    mode: PromptImageMode,
}

struct ImageLruCache<K, V> {
    inner: Mutex<LruCache<K, V>>,
}

impl<K, V> ImageLruCache<K, V>
where
    K: Eq + std::hash::Hash,
{
    fn new(capacity: NonZeroUsize) -> Self {
        Self {
            inner: Mutex::new(LruCache::new(capacity)),
        }
    }

    fn get_or_insert_with(&self, key: K, value: impl FnOnce() -> V) -> V
    where
        V: Clone,
    {
        let Ok(mut guard) = self.inner.lock() else {
            return value();
        };
        if let Some(cached) = guard.get(&key) {
            return cached.clone();
        }
        let value = value();
        guard.put(key, value.clone());
        value
    }

    fn get_or_try_insert_with<E>(
        &self,
        key: K,
        value: impl FnOnce() -> Result<V, E>,
    ) -> Result<V, E>
    where
        V: Clone,
    {
        let Ok(mut guard) = self.inner.lock() else {
            return value();
        };
        if let Some(cached) = guard.get(&key) {
            return Ok(cached.clone());
        }
        let value = value()?;
        guard.put(key, value.clone());
        Ok(value)
    }

    #[cfg(test)]
    fn clear(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.clear();
        }
    }
}

static IMAGE_CACHE: LazyLock<ImageLruCache<ImageCacheKey, EncodedImage>> =
    LazyLock::new(|| ImageLruCache::new(NonZeroUsize::new(32).unwrap_or(NonZeroUsize::MIN)));

static ORIGINAL_IMAGE_ESTIMATE_CACHE: LazyLock<ImageLruCache<[u8; 20], Option<i64>>> =
    LazyLock::new(|| {
        ImageLruCache::new(
            NonZeroUsize::new(ORIGINAL_IMAGE_ESTIMATE_CACHE_SIZE).unwrap_or(NonZeroUsize::MIN),
        )
    });

pub fn load_for_prompt_bytes(
    path: &Path,
    file_bytes: Vec<u8>,
    mode: PromptImageMode,
) -> Result<EncodedImage, ImageProcessingError> {
    let path_buf = path.to_path_buf();

    let key = ImageCacheKey {
        digest: sha1_digest(&file_bytes),
        mode,
    };

    IMAGE_CACHE.get_or_try_insert_with(key, move || {
        let format = match image::guess_format(&file_bytes) {
            Ok(ImageFormat::Png) => Some(ImageFormat::Png),
            Ok(ImageFormat::Jpeg) => Some(ImageFormat::Jpeg),
            Ok(ImageFormat::Gif) => Some(ImageFormat::Gif),
            Ok(ImageFormat::WebP) => Some(ImageFormat::WebP),
            _ => None,
        };

        let dynamic = image::load_from_memory(&file_bytes)
            .map_err(|source| ImageProcessingError::decode_error(&path_buf, source))?;

        let (width, height) = dynamic.dimensions();

        let encoded = if mode == PromptImageMode::Original
            || (width <= MAX_DIMENSION && height <= MAX_DIMENSION)
        {
            if let Some(format) = format.filter(|format| can_preserve_source_bytes(*format)) {
                let mime = format_to_mime(format);
                EncodedImage {
                    bytes: file_bytes,
                    mime,
                    width,
                    height,
                }
            } else {
                let (bytes, output_format) = encode_image(&dynamic, ImageFormat::Png)?;
                let mime = format_to_mime(output_format);
                EncodedImage {
                    bytes,
                    mime,
                    width,
                    height,
                }
            }
        } else {
            let resized = dynamic.resize(MAX_DIMENSION, MAX_DIMENSION, FilterType::Triangle);
            let target_format = format
                .filter(|format| can_preserve_source_bytes(*format))
                .unwrap_or(ImageFormat::Png);
            let (bytes, output_format) = encode_image(&resized, target_format)?;
            let mime = format_to_mime(output_format);
            EncodedImage {
                bytes,
                mime,
                width: resized.width(),
                height: resized.height(),
            }
        };

        Ok(encoded)
    })
}

pub fn dimensions_from_memory(bytes: &[u8]) -> Result<(u32, u32), ImageProcessingError> {
    let dynamic = image::load_from_memory(bytes)
        .map_err(|source| ImageProcessingError::DecodeMemory { source })?;
    Ok(dynamic.dimensions())
}

fn sha1_digest(bytes: &[u8]) -> [u8; 20] {
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    let result = hasher.finalize();
    let mut out = [0; 20];
    out.copy_from_slice(&result);
    out
}

fn can_preserve_source_bytes(format: ImageFormat) -> bool {
    // Public API docs explicitly call out non-animated GIF support only.
    // Preserve byte-for-byte only for formats we can safely pass through.
    matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    )
}

fn encode_image(
    image: &DynamicImage,
    preferred_format: ImageFormat,
) -> Result<(Vec<u8>, ImageFormat), ImageProcessingError> {
    let target_format = match preferred_format {
        ImageFormat::Jpeg => ImageFormat::Jpeg,
        ImageFormat::WebP => ImageFormat::WebP,
        _ => ImageFormat::Png,
    };

    let mut buffer = Vec::new();

    match target_format {
        ImageFormat::Png => {
            let rgba = image.to_rgba8();
            let encoder = PngEncoder::new(&mut buffer);
            encoder
                .write_image(
                    rgba.as_raw(),
                    image.width(),
                    image.height(),
                    ColorType::Rgba8.into(),
                )
                .map_err(|source| ImageProcessingError::Encode {
                    format: target_format,
                    source,
                })?;
        }
        ImageFormat::Jpeg => {
            let mut encoder = JpegEncoder::new_with_quality(&mut buffer, 85);
            encoder
                .encode_image(image)
                .map_err(|source| ImageProcessingError::Encode {
                    format: target_format,
                    source,
                })?;
        }
        ImageFormat::WebP => {
            let rgba = image.to_rgba8();
            let encoder = WebPEncoder::new_lossless(&mut buffer);
            encoder
                .write_image(
                    rgba.as_raw(),
                    image.width(),
                    image.height(),
                    ColorType::Rgba8.into(),
                )
                .map_err(|source| ImageProcessingError::Encode {
                    format: target_format,
                    source,
                })?;
        }
        _ => unreachable!("unsupported target_format should have been handled earlier"),
    }

    Ok((buffer, target_format))
}

fn format_to_mime(format: ImageFormat) -> String {
    match format {
        ImageFormat::Jpeg => "image/jpeg".to_string(),
        ImageFormat::Gif => "image/gif".to_string(),
        ImageFormat::WebP => "image/webp".to_string(),
        _ => "image/png".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use image::GenericImageView;
    use image::ImageBuffer;
    use image::Rgba;

    fn image_bytes(image: &ImageBuffer<Rgba<u8>, Vec<u8>>, format: ImageFormat) -> Vec<u8> {
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image.clone())
            .write_to(&mut encoded, format)
            .expect("encode image to bytes");
        encoded.into_inner()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn returns_original_image_when_within_bounds() {
        for (format, mime) in [
            (ImageFormat::Png, "image/png"),
            (ImageFormat::WebP, "image/webp"),
        ] {
            let image = ImageBuffer::from_pixel(64, 32, Rgba([10u8, 20, 30, 255]));
            let original_bytes = image_bytes(&image, format);

            let encoded = load_for_prompt_bytes(
                Path::new("in-memory-image"),
                original_bytes.clone(),
                PromptImageMode::ResizeToFit,
            )
            .expect("process image");

            assert_eq!(encoded.width, 64);
            assert_eq!(encoded.height, 32);
            assert_eq!(encoded.mime, mime);
            assert_eq!(encoded.bytes, original_bytes);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn downscales_large_image() {
        for (format, mime) in [
            (ImageFormat::Png, "image/png"),
            (ImageFormat::WebP, "image/webp"),
        ] {
            let image = ImageBuffer::from_pixel(4096, 2048, Rgba([200u8, 10, 10, 255]));
            let original_bytes = image_bytes(&image, format);

            let processed = load_for_prompt_bytes(
                Path::new("in-memory-image"),
                original_bytes,
                PromptImageMode::ResizeToFit,
            )
            .expect("process image");

            assert!(processed.width <= MAX_DIMENSION);
            assert!(processed.height <= MAX_DIMENSION);
            assert_eq!(processed.mime, mime);

            let detected_format =
                image::guess_format(&processed.bytes).expect("detect resized output format");
            assert_eq!(detected_format, format);

            let loaded = image::load_from_memory(&processed.bytes)
                .expect("read resized bytes back into image");
            assert_eq!(loaded.dimensions(), (processed.width, processed.height));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn downscales_tall_image_to_fit_square_bounds() {
        let image = ImageBuffer::from_pixel(1024, 4096, Rgba([200u8, 10, 10, 255]));
        let original_bytes = image_bytes(&image, ImageFormat::Png);

        let processed = load_for_prompt_bytes(
            Path::new("in-memory-image"),
            original_bytes,
            PromptImageMode::ResizeToFit,
        )
        .expect("process image");

        assert_eq!(processed.width, 512);
        assert_eq!(processed.height, MAX_DIMENSION);
        assert_eq!(processed.mime, "image/png");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn preserves_large_image_in_original_mode() {
        let image = ImageBuffer::from_pixel(4096, 2048, Rgba([180u8, 30, 30, 255]));
        let original_bytes = image_bytes(&image, ImageFormat::Png);

        let processed = load_for_prompt_bytes(
            Path::new("in-memory-image"),
            original_bytes.clone(),
            PromptImageMode::Original,
        )
        .expect("process image");

        assert_eq!(processed.width, 4096);
        assert_eq!(processed.height, 2048);
        assert_eq!(processed.mime, "image/png");
        assert_eq!(processed.bytes, original_bytes);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fails_cleanly_for_invalid_images() {
        let err = load_for_prompt_bytes(
            Path::new("in-memory-image"),
            b"not an image".to_vec(),
            PromptImageMode::ResizeToFit,
        )
        .expect_err("invalid image should fail");
        assert!(matches!(
            err,
            ImageProcessingError::Decode { .. }
                | ImageProcessingError::UnsupportedImageFormat { .. }
        ));
    }

    #[test]
    fn returns_dimensions_from_memory() {
        let image = ImageBuffer::from_pixel(17, 23, Rgba([10u8, 20, 30, 255]));
        let bytes = image_bytes(&image, ImageFormat::Png);

        let dimensions = dimensions_from_memory(&bytes).expect("decode dimensions");

        assert_eq!(dimensions, (17, 23));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reprocesses_updated_file_contents() {
        {
            IMAGE_CACHE.clear();
        }

        let first_image = ImageBuffer::from_pixel(32, 16, Rgba([20u8, 120, 220, 255]));
        let first_bytes = image_bytes(&first_image, ImageFormat::Png);

        let first = load_for_prompt_bytes(
            Path::new("in-memory-image"),
            first_bytes,
            PromptImageMode::ResizeToFit,
        )
        .expect("process first image");

        let second_image = ImageBuffer::from_pixel(96, 48, Rgba([50u8, 60, 70, 255]));
        let second_bytes = image_bytes(&second_image, ImageFormat::Png);

        let second = load_for_prompt_bytes(
            Path::new("in-memory-image"),
            second_bytes,
            PromptImageMode::ResizeToFit,
        )
        .expect("process updated image");

        assert_eq!(first.width, 32);
        assert_eq!(first.height, 16);
        assert_eq!(second.width, 96);
        assert_eq!(second.height, 48);
        assert_ne!(second.bytes, first.bytes);
    }
}
