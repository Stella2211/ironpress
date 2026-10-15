use std::borrow::Cow;
use std::io::{BufRead, Cursor, Read, Seek, SeekFrom};
use std::time::Duration;

use image::metadata::Orientation;
use image::{DynamicImage, GenericImageView, GrayImage, ImageFormat, ImageReader, RgbImage};
use mozjpeg_rs::{Preset, Subsampling, TrellisConfig};
use oxipng::StripChunks;

use crate::error::CompressError;
use crate::options::{ChromaSubsampling, CompressParams, OutputFormat};

// ─── Fast JPEG Decoding ─────────────────────────────────────────────────────

/// Decode a JPEG using zune-jpeg (SIMD-accelerated, 2-4x faster than image crate).
/// Falls back to the image crate if zune-jpeg fails.
///
/// Grayscale JPEGs decode to `Luma8` (1 byte/pixel instead of 3), which the
/// rest of the pipeline keeps single-channel down to the JPEG/PNG encoders.
fn decode_jpeg_fast(data: &[u8]) -> Result<DynamicImage, CompressError> {
    use zune_jpeg::JpegDecoder;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = JpegDecoder::new_with_options(Cursor::new(data), options);

    let gray =
        decoder.decode_headers().is_ok() && decoder.input_colorspace() == Some(ColorSpace::Luma);
    if gray {
        decoder.set_options(options.jpeg_set_out_colorspace(ColorSpace::Luma));
    }

    let pixels = match decoder.decode() {
        Ok(p) => p,
        Err(_) => {
            // Fallback to image crate for edge cases (progressive, unusual markers)
            return image::load_from_memory_with_format(data, ImageFormat::Jpeg)
                .map_err(|e| CompressError::DecodeError(e.to_string()));
        }
    };

    let info = decoder.info().ok_or_else(|| {
        CompressError::DecodeError("Failed to get JPEG info from zune-jpeg".into())
    })?;
    let width = info.width as u32;
    let height = info.height as u32;

    let image = if gray {
        GrayImage::from_raw(width, height, pixels).map(DynamicImage::ImageLuma8)
    } else {
        RgbImage::from_raw(width, height, pixels).map(DynamicImage::ImageRgb8)
    };
    image.ok_or_else(|| CompressError::DecodeError("Pixel buffer size mismatch".into()))
}

/// How much larger than the final size a DCT-scaled decode must stay (in
/// both axes) before the remaining downscale is done with CatmullRom.
///
/// DCT scaling alone acts like a box filter: decoding straight to the final
/// size (margin 1) measured 32-35 dB PSNR against the full-resolution
/// resample on a 12 MP photo and +9% output size, while keeping a 2× margin
/// (the same idea as Pillow's `reducing_gap`) measured 46-50 dB with output
/// size within 1%, still decoding 4-64× fewer pixels.
const JPEG_SCALED_DECODE_MARGIN: u32 = 2;

/// Largest JPEG DCT scaling denominator (8, 4 or 2) whose output still
/// covers [`JPEG_SCALED_DECODE_MARGIN`]× `target_w`×`target_h` in both
/// axes, or 1 when no reduced decode applies.
pub(crate) fn jpeg_scale_denominator(width: u32, height: u32, target_w: u32, target_h: u32) -> u32 {
    let (min_w, min_h) = (
        target_w.saturating_mul(JPEG_SCALED_DECODE_MARGIN),
        target_h.saturating_mul(JPEG_SCALED_DECODE_MARGIN),
    );
    [8u32, 4, 2]
        .into_iter()
        .find(|&d| width.div_ceil(d) >= min_w && height.div_ceil(d) >= min_h)
        .unwrap_or(1)
}

/// Decode a JPEG at 1/2, 1/4 or 1/8 scale (DCT-domain scaling) when the
/// result still covers [`JPEG_SCALED_DECODE_MARGIN`]× `target_w`×`target_h`
/// (raw, pre-orientation dimensions); the caller resamples the rest. Returns `None` whenever reduced decoding does not apply or
/// fails (CMYK, 16-bit, corrupt data, ...) so callers fall back to the
/// regular full-size decode.
pub(crate) fn decode_jpeg_scaled(
    data: &[u8],
    target_w: u32,
    target_h: u32,
) -> Option<DynamicImage> {
    use jpeg_decoder::{Decoder, PixelFormat};

    let mut decoder = Decoder::new(data);
    decoder.read_info().ok()?;
    let info = decoder.info()?;
    if !matches!(info.pixel_format, PixelFormat::L8 | PixelFormat::RGB24) {
        return None;
    }

    let (full_w, full_h) = (u32::from(info.width), u32::from(info.height));
    let denominator = jpeg_scale_denominator(full_w, full_h, target_w, target_h);
    if denominator == 1 {
        return None;
    }

    let expected = (full_w.div_ceil(denominator), full_h.div_ceil(denominator));
    let (out_w, out_h) = decoder.scale(expected.0 as u16, expected.1 as u16).ok()?;
    if (u32::from(out_w), u32::from(out_h)) != expected {
        return None;
    }

    let pixels = decoder.decode().ok()?;
    let (w, h) = expected;
    match info.pixel_format {
        PixelFormat::L8 => GrayImage::from_raw(w, h, pixels).map(DynamicImage::ImageLuma8),
        _ => RgbImage::from_raw(w, h, pixels).map(DynamicImage::ImageRgb8),
    }
}

// ─── EXIF Orientation ───────────────────────────────────────────────────────

/// EXIF payload (TIFF structure) of a JPEG: the first APP1 segment that
/// carries an `Exif\0\0` header, without that header.
fn jpeg_exif_segment(data: &[u8]) -> Option<&[u8]> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }

    let mut offset = 2usize;
    while offset + 4 <= data.len() {
        if data[offset] != 0xFF {
            break;
        }

        let mut marker_offset = offset + 1;
        while marker_offset < data.len() && data[marker_offset] == 0xFF {
            marker_offset += 1;
        }
        if marker_offset >= data.len() {
            break;
        }

        let marker = data[marker_offset];
        offset = marker_offset + 1;

        if marker == 0xD9 || marker == 0xDA {
            break;
        }

        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            continue;
        }

        if offset + 2 > data.len() {
            break;
        }

        let segment_len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
        if segment_len < 2 || offset + segment_len > data.len() {
            break;
        }

        let segment = &data[offset + 2..offset + segment_len];
        if marker == 0xE1 && segment.starts_with(b"Exif\0\0") && segment.len() > 6 {
            return Some(&segment[6..]);
        }

        offset += segment_len;
    }

    None
}

/// Payload of the PNG `eXIf` chunk, if any.
fn png_exif_chunk(data: &[u8]) -> Option<&[u8]> {
    let mut offset = 8usize;
    while offset.checked_add(8)? <= data.len() {
        let chunk_len = u32::from_be_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
        let chunk_type = &data[offset + 4..offset + 8];
        let body_end = (offset + 8).checked_add(chunk_len)?;
        if chunk_type == b"eXIf" {
            return data.get(offset + 8..body_end);
        }
        if chunk_type == b"IEND" {
            break;
        }
        offset = body_end.checked_add(4)?;
    }
    None
}

/// Payload of the WebP `EXIF` chunk, if any.
fn webp_exif_chunk(data: &[u8]) -> Option<&[u8]> {
    let mut offset = 12usize;
    while offset.checked_add(8)? <= data.len() {
        let fourcc = &data[offset..offset + 4];
        let size = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().ok()?) as usize;
        let body_end = (offset + 8).checked_add(size)?;
        if fourcc == b"EXIF" {
            return data.get(offset + 8..body_end);
        }
        // Chunks are padded to an even size.
        offset = body_end.checked_add(size & 1)?;
    }
    None
}

/// EXIF orientation (1-8) declared by the input's metadata, or 1 when the
/// input has no (valid) orientation tag. Covers JPEG APP1, PNG `eXIf`,
/// WebP `EXIF` and the TIFF header itself.
pub(crate) fn read_orientation(data: &[u8]) -> u8 {
    let chunk = match input_format_hint(data) {
        Some(ImageFormat::Jpeg) => jpeg_exif_segment(data),
        Some(ImageFormat::Png) => png_exif_chunk(data),
        Some(ImageFormat::WebP) => webp_exif_chunk(data),
        Some(ImageFormat::Tiff) => Some(data),
        _ => None,
    };
    chunk
        .map(|c| c.strip_prefix(b"Exif\0\0").unwrap_or(c))
        .and_then(Orientation::from_exif_chunk)
        .map(Orientation::to_exif)
        .unwrap_or(1)
}

/// Orientations 5-8 transpose the image (width and height swap).
fn orientation_swaps_axes(orientation: u8) -> bool {
    (5..=8).contains(&orientation)
}

fn apply_orientation(mut img: DynamicImage, orientation: u8) -> DynamicImage {
    if let Some(o) = Orientation::from_exif(orientation) {
        img.apply_orientation(o);
    }
    img
}

// ─── Decode Pipeline ────────────────────────────────────────────────────────

/// Everything needed to (re)produce display-oriented pixels from the input
/// at a given size. Kept around during the target-size search so each
/// downscale resamples the original once instead of shrinking an already
/// shrunk copy (the original pixels are not retained: they are decoded
/// again, which never exceeds the first decode's peak memory).
struct DecodeSource<'a> {
    input: &'a [u8],
    detected: DetectedFormat,
    /// EXIF orientation (1-8) applied after decoding.
    orientation: u8,
    /// Raw (pre-orientation) header dimensions, when readable.
    raw_dims: Option<(u32, u32)>,
}

impl<'a> DecodeSource<'a> {
    fn new(input: &'a [u8], detected: DetectedFormat) -> Self {
        let raw_dims = input_format_hint(input)
            .and_then(|hint| read_dimensions(Cursor::new(input), hint).ok());
        Self {
            input,
            detected,
            orientation: read_orientation(input),
            raw_dims,
        }
    }

    /// Display-oriented header dimensions.
    fn oriented_dims(&self) -> Option<(u32, u32)> {
        self.raw_dims.map(|(w, h)| {
            if orientation_swaps_axes(self.orientation) {
                (h, w)
            } else {
                (w, h)
            }
        })
    }

    /// Decode with `max_width`/`max_height` applied (aspect preserved, never
    /// upscaled), in display orientation.
    fn decode_constrained(
        &self,
        max_width: u32,
        max_height: u32,
    ) -> Result<DynamicImage, CompressError> {
        match self.oriented_dims() {
            Some((w, h)) => self.decode_at(constrained_dimensions(w, h, max_width, max_height)),
            None => {
                // Header unreadable up front: decode, orient, then constrain.
                let img = decode_image_with_hint(self.input, self.detected)?;
                let img = apply_orientation(img, self.orientation);
                Ok(apply_resize_constraints(img, max_width, max_height))
            }
        }
    }

    /// Decode and resize to exactly `target` (display-oriented), or keep
    /// the full size when `target` is `None`. JPEGs use DCT-scaled decoding
    /// when the target is at most a quarter of the source size (see
    /// [`JPEG_SCALED_DECODE_MARGIN`]); the remainder is resampled as before,
    /// so the final dimensions do not change.
    fn decode_at(&self, target: Option<(u32, u32)>) -> Result<DynamicImage, CompressError> {
        let swaps = orientation_swaps_axes(self.orientation);
        let raw_target = target.map(|(w, h)| if swaps { (h, w) } else { (w, h) });

        let scaled = match raw_target {
            Some((tw, th))
                if self.detected == DetectedFormat::Jpeg && is_native_jpeg(self.input) =>
            {
                decode_jpeg_scaled(self.input, tw, th)
            }
            _ => None,
        };
        let img = match scaled {
            Some(img) => img,
            None => decode_image_with_hint(self.input, self.detected)?,
        };

        // Resize before orienting: the rotation then touches the smaller
        // buffer (rotations by 90° allocate a full copy).
        let img = match raw_target {
            Some((tw, th)) => resize_owned(img, tw, th),
            None => img,
        };
        Ok(apply_orientation(img, self.orientation))
    }
}

// ─── Fast Resizing ──────────────────────────────────────────────────────────

/// Resize using fast_image_resize (SIMD-accelerated, ~5x faster than image crate Lanczos3).
/// Uses CatmullRom (bicubic) which is visually close to Lanczos3 but much faster.
///
/// Preserves the pixel format: RGB images stay RGB, RGBA images stay RGBA.
/// Falls back to image crate resize if fast_image_resize fails.
fn resize_fast(img: &DynamicImage, new_w: u32, new_h: u32) -> DynamicImage {
    use fast_image_resize as fir;

    // Guard against zero dimensions — fast_image_resize would panic.
    let new_w = new_w.max(1);
    let new_h = new_h.max(1);

    let resize_opts = fir::ResizeOptions::new()
        .resize_alg(fir::ResizeAlg::Convolution(fir::FilterType::CatmullRom));

    // Try RGBA path first (preserves alpha for PNG), fall back to RGB.
    match img {
        DynamicImage::ImageLuma8(gray) => resize_luma8(gray, new_w, new_h, &resize_opts),
        DynamicImage::ImageRgb8(_) => resize_rgb8(img, new_w, new_h, &resize_opts),
        _ => {
            // RGBA path: preserves alpha channel for PNG and other formats
            resize_rgba8(img, new_w, new_h, &resize_opts)
        }
    }
}

/// Resize to exactly `new_w`×`new_h`, consuming the source so its buffer is
/// freed as soon as the resized copy exists. No-op when already that size.
fn resize_owned(img: DynamicImage, new_w: u32, new_h: u32) -> DynamicImage {
    if img.dimensions() == (new_w, new_h) {
        img
    } else {
        resize_fast(&img, new_w, new_h)
    }
}

/// Resize as Luma8 (1 channel). Used for grayscale JPEG inputs.
fn resize_luma8(
    img: &image::GrayImage,
    new_w: u32,
    new_h: u32,
    opts: &fast_image_resize::ResizeOptions,
) -> DynamicImage {
    use fast_image_resize as fir;

    let fallback = || {
        DynamicImage::ImageLuma8(image::imageops::resize(
            img,
            new_w,
            new_h,
            image::imageops::FilterType::CatmullRom,
        ))
    };

    let Ok(src) =
        fir::images::ImageRef::new(img.width(), img.height(), img.as_raw(), fir::PixelType::U8)
    else {
        return fallback();
    };
    let mut dst = fir::images::Image::new(new_w, new_h, fir::PixelType::U8);
    if fir::Resizer::new().resize(&src, &mut dst, opts).is_err() {
        return fallback();
    }
    GrayImage::from_raw(new_w, new_h, dst.into_vec())
        .map(DynamicImage::ImageLuma8)
        .unwrap_or_else(fallback)
}

/// Resize as RGB8 (3 channels). Used for JPEG inputs.
fn resize_rgb8(
    img: &DynamicImage,
    new_w: u32,
    new_h: u32,
    opts: &fast_image_resize::ResizeOptions,
) -> DynamicImage {
    use fast_image_resize as fir;

    // Borrow the pixel buffer when the image is already RGB8 (the common
    // JPEG case); only other layouts (e.g. Luma8) need a converted copy.
    let converted;
    let (src_w, src_h, src_buf) = match img {
        DynamicImage::ImageRgb8(rgb) => (rgb.width(), rgb.height(), rgb.as_raw().as_slice()),
        other => {
            converted = other.to_rgb8();
            (
                converted.width(),
                converted.height(),
                converted.as_raw().as_slice(),
            )
        }
    };

    let src = match fir::images::ImageRef::new(src_w, src_h, src_buf, fir::PixelType::U8x3) {
        Ok(s) => s,
        Err(_) => return fallback_resize(img, new_w, new_h),
    };

    let mut dst = fir::images::Image::new(new_w, new_h, fir::PixelType::U8x3);
    let mut resizer = fir::Resizer::new();

    if resizer.resize(&src, &mut dst, opts).is_err() {
        return fallback_resize(img, new_w, new_h);
    }

    RgbImage::from_raw(new_w, new_h, dst.into_vec())
        .map(DynamicImage::ImageRgb8)
        .unwrap_or_else(|| fallback_resize(img, new_w, new_h))
}

/// Resize as RGBA8 (4 channels). Used for PNG and WebP inputs that may have alpha.
fn resize_rgba8(
    img: &DynamicImage,
    new_w: u32,
    new_h: u32,
    opts: &fast_image_resize::ResizeOptions,
) -> DynamicImage {
    use fast_image_resize as fir;
    use image::RgbaImage;

    // Borrow the pixel buffer when the image is already RGBA8; only other
    // layouts (LumaA8, 16-bit, ...) need a converted copy.
    let converted;
    let (src_w, src_h, src_buf) = match img {
        DynamicImage::ImageRgba8(rgba) => (rgba.width(), rgba.height(), rgba.as_raw().as_slice()),
        other => {
            converted = other.to_rgba8();
            (
                converted.width(),
                converted.height(),
                converted.as_raw().as_slice(),
            )
        }
    };

    let src = match fir::images::ImageRef::new(src_w, src_h, src_buf, fir::PixelType::U8x4) {
        Ok(s) => s,
        Err(_) => return fallback_resize(img, new_w, new_h),
    };

    let mut dst = fir::images::Image::new(new_w, new_h, fir::PixelType::U8x4);
    let mut resizer = fir::Resizer::new();

    if resizer.resize(&src, &mut dst, opts).is_err() {
        return fallback_resize(img, new_w, new_h);
    }

    RgbaImage::from_raw(new_w, new_h, dst.into_vec())
        .map(DynamicImage::ImageRgba8)
        .unwrap_or_else(|| fallback_resize(img, new_w, new_h))
}

/// Fallback to image crate resize. Slower but always correct.
fn fallback_resize(img: &DynamicImage, new_w: u32, new_h: u32) -> DynamicImage {
    img.resize_exact(new_w, new_h, image::imageops::FilterType::CatmullRom)
}

/// Maximum number of ×[`RESIZE_SCALE_FACTOR`] downscale steps (overall
/// at most 0.75⁴ ≈ 0.32× per side) before the target-size search returns its
/// best effort at the floor quality.
const MAX_RESIZE_CYCLES: u32 = 4;
/// Per-step downscale factor when auto-resizing to meet the size target.
const RESIZE_SCALE_FACTOR: f64 = 0.75;
/// A size prediction this many times the target is considered clearly
/// unreachable: used to take several downscale steps at once and to skip a
/// hopeless initial-quality attempt after a downscale. Generous because
/// downscaling can shrink noisy images faster than their area.
const CLEARLY_TOO_LARGE_FACTOR: f64 = 2.0;
/// Encodes spent on the quality search in one resize cycle, excluding the
/// initial-quality attempt (bounds the worst case; interpolation usually
/// converges in 2-5).
const MAX_QUALITY_SEARCH_ENCODES: u32 = 8;
/// Assumed slope of ln(size) over [`quality_scale_log`] until two samples
/// exist (measured 1.1-1.9 for mozjpeg and libwebp on photos).
const DEFAULT_LOG_SIZE_SLOPE: f64 = 1.4;

/// Time budget for oxipng's optimization trials per encode. oxipng stops
/// starting new trials once it is exceeded and keeps the best result so far.
/// 10 s: the default level 2 needs ~6 s for a worst-case (noise-heavy)
/// 12 MP image on an Apple M-series machine, so default-level output stays
/// fully optimized there, while levels 4-6 (12-35+ s for the same image,
/// several times longer on phones) get a hard cost ceiling.
pub(crate) const PNG_OPTIMIZE_TIMEOUT: Duration = Duration::from_secs(10);
/// oxipng preset used for target-size trial encodes while searching for a
/// size that fits (~7x faster than level 2, ~10% larger on photos). The
/// accepted size is always re-encoded/verified at the requested level.
const PNG_TRIAL_LEVEL: u8 = 0;
/// A trial-preset result within this factor of the target is re-encoded at
/// the requested level to check whether that one fits.
const PNG_TRIAL_MARGIN: f64 = 1.15;

/// Check if the raw bytes are actually a JPEG file (not GIF/BMP/TIFF mapped to Jpeg output).
fn is_native_jpeg(data: &[u8]) -> bool {
    data.len() >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF
}

fn input_format_hint(data: &[u8]) -> Option<ImageFormat> {
    if is_native_jpeg(data) {
        return Some(ImageFormat::Jpeg);
    }

    if data.len() >= 4 && data[0] == 0x89 && data[1] == 0x50 && data[2] == 0x4E && data[3] == 0x47 {
        return Some(ImageFormat::Png);
    }

    if data.len() >= 16 && data[0..4] == *b"RIFF" && data[8..12] == *b"WEBP" {
        return Some(ImageFormat::WebP);
    }

    if data.len() >= 6 && &data[0..3] == b"GIF" {
        return Some(ImageFormat::Gif);
    }

    if data.len() >= 2 && data[0] == 0x42 && data[1] == 0x4D {
        return Some(ImageFormat::Bmp);
    }

    if data.len() >= 4
        && ((data[0] == 0x49 && data[1] == 0x49) || (data[0] == 0x4D && data[1] == 0x4D))
    {
        return Some(ImageFormat::Tiff);
    }

    None
}

fn decode_image_with_hint(
    data: &[u8],
    detected: DetectedFormat,
) -> Result<DynamicImage, CompressError> {
    if detected == DetectedFormat::Jpeg && is_native_jpeg(data) {
        return decode_jpeg_fast(data);
    }

    if let Some(format_hint) = input_format_hint(data) {
        return image::load_from_memory_with_format(data, format_hint)
            .map_err(|e| CompressError::DecodeError(e.to_string()));
    }

    image::load_from_memory(data).map_err(|e| CompressError::DecodeError(e.to_string()))
}

fn is_apng(data: &[u8]) -> bool {
    if data.len() < 12 {
        return false;
    }

    let mut offset = 8usize;
    while offset + 8 <= data.len() {
        let chunk_len = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        let chunk_type = &data[offset + 4..offset + 8];

        if chunk_type == b"acTL" {
            return true;
        }

        if chunk_type == b"IEND" || offset + 12 + chunk_len > data.len() {
            break;
        }

        offset += 12 + chunk_len;
    }

    false
}

fn can_direct_optimize_png(
    input: &[u8],
    original_format: DetectedFormat,
    output_format: DetectedFormat,
    params: &CompressParams,
) -> bool {
    original_format == DetectedFormat::Png
        && output_format == DetectedFormat::Png
        && params.max_width == 0
        && params.max_height == 0
        && params.max_file_size == 0
        && !is_apng(input)
        // A rotated/flipped PNG must be decoded so the orientation is applied.
        && read_orientation(input) == 1
}

fn optimize_png_direct(
    input: &[u8],
    params: &CompressParams,
) -> Result<EngineResult, CompressError> {
    let probe = probe_bytes(input)?;
    let mut options = png_options(png_level(params, probe.width, probe.height));
    options.strip = StripChunks::All;

    let optimized = oxipng::optimize_from_memory(input, &options)
        .map_err(|e| CompressError::EncodeError(format!("PNG optimization failed: {e}")))?;

    Ok(EngineResult {
        data: optimized,
        width: probe.width,
        height: probe.height,
        quality_used: params.quality.min(100),
        iterations: 1,
        resized_to_fit: false,
    })
}

/// Detected input format (also used as output format selector).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectedFormat {
    Jpeg,
    Png,
    WebpLossless,
    WebpLossy,
}

/// Pixel buffer in the layout the encoder needs, prepared once so that
/// quality search iterations don't repeat `to_rgb8()`/`to_rgba8()` (each
/// conversion of a 12MP image allocates ~36-48 MB).
///
/// When the decoded image already has the required layout the buffer is
/// borrowed instead of copied. PNG output encodes straight from the
/// `DynamicImage`, so no buffer is prepared at all.
enum PreparedPixels<'a> {
    /// Single-channel 8-bit (grayscale JPEG output).
    Gray {
        data: Cow<'a, [u8]>,
        width: u32,
        height: u32,
    },
    Rgb {
        data: Cow<'a, [u8]>,
        width: u32,
        height: u32,
    },
    Rgba {
        data: Cow<'a, [u8]>,
        width: u32,
        height: u32,
    },
    /// No pre-converted buffer (PNG output encodes from the DynamicImage).
    Unused { width: u32, height: u32 },
}

impl PreparedPixels<'_> {
    fn dimensions(&self) -> (u32, u32) {
        match self {
            Self::Gray { width, height, .. }
            | Self::Rgb { width, height, .. }
            | Self::Rgba { width, height, .. }
            | Self::Unused { width, height } => (*width, *height),
        }
    }
}

fn prepared_rgb(img: &DynamicImage) -> PreparedPixels<'_> {
    match img {
        DynamicImage::ImageRgb8(rgb) => {
            let (width, height) = rgb.dimensions();
            PreparedPixels::Rgb {
                data: Cow::Borrowed(rgb.as_raw().as_slice()),
                width,
                height,
            }
        }
        other => {
            let rgb = other.to_rgb8();
            let (width, height) = rgb.dimensions();
            PreparedPixels::Rgb {
                data: Cow::Owned(rgb.into_raw()),
                width,
                height,
            }
        }
    }
}

fn prepared_rgba(img: &DynamicImage) -> PreparedPixels<'_> {
    match img {
        DynamicImage::ImageRgba8(rgba) => {
            let (width, height) = rgba.dimensions();
            PreparedPixels::Rgba {
                data: Cow::Borrowed(rgba.as_raw().as_slice()),
                width,
                height,
            }
        }
        other => {
            let rgba = other.to_rgba8();
            let (width, height) = rgba.dimensions();
            PreparedPixels::Rgba {
                data: Cow::Owned(rgba.into_raw()),
                width,
                height,
            }
        }
    }
}

fn prepare_pixels(img: &DynamicImage, format: DetectedFormat) -> PreparedPixels<'_> {
    match format {
        DetectedFormat::Jpeg => match img {
            DynamicImage::ImageLuma8(gray) => {
                let (width, height) = gray.dimensions();
                PreparedPixels::Gray {
                    data: Cow::Borrowed(gray.as_raw().as_slice()),
                    width,
                    height,
                }
            }
            _ => prepared_rgb(img),
        },
        DetectedFormat::WebpLossless | DetectedFormat::WebpLossy => match img {
            DynamicImage::ImageRgb8(_) => prepared_rgb(img),
            _ => prepared_rgba(img),
        },
        DetectedFormat::Png => {
            let (width, height) = img.dimensions();
            PreparedPixels::Unused { width, height }
        }
    }
}

/// Internal result from the compression engine.
pub struct EngineResult {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub quality_used: u32,
    pub iterations: u32,
    pub resized_to_fit: bool,
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Compress raw image bytes with the given parameters.
pub fn compress_bytes(
    input: &[u8],
    params: &CompressParams,
) -> Result<EngineResult, CompressError> {
    let original_format = detect_format(input)?;
    let output_format = resolve_output_format(params, original_format);

    // Reject decompression bombs before any pixel buffer is allocated
    // (this also covers oxipng's full decode in the direct PNG path).
    check_pixel_limit(input)?;

    if can_direct_optimize_png(input, original_format, output_format, params) {
        return optimize_png_direct(input, params);
    }

    let source = DecodeSource::new(input, original_format);
    let jpeg_exif = resolve_jpeg_exif_payload(input, original_format, output_format, params)?;

    // Decode (DCT-scaled for large JPEG downscales), apply the EXIF
    // orientation and the explicit resize constraints.
    let img = source.decode_constrained(params.max_width, params.max_height)?;

    if params.max_file_size > 0 {
        compress_to_target_size(img, &source, params, output_format, jpeg_exif.as_deref())
    } else {
        let quality = params.quality.min(100) as u8;
        let (w, h) = img.dimensions();
        let encoded = match output_format {
            // Hand the pixel buffer to oxipng without copying it.
            DetectedFormat::Png => encode_png_owned(img, png_level(params, w, h))?,
            _ => encode_image(&img, quality, output_format, params, jpeg_exif.as_deref())?,
        };
        Ok(EngineResult {
            data: encoded,
            width: w,
            height: h,
            quality_used: quality as u32,
            iterations: 1,
            resized_to_fit: false,
        })
    }
}

// ─── Target Size Search ──────────────────────────────────────────────────────

/// Encoder settings shared by every attempt of one target-size search.
struct EncodeCtx<'a> {
    params: &'a CompressParams,
    format: DetectedFormat,
    jpeg_exif: Option<&'a [u8]>,
}

impl EncodeCtx<'_> {
    fn encode(
        &self,
        prepared: &PreparedPixels<'_>,
        img: &DynamicImage,
        quality: u8,
    ) -> Result<Vec<u8>, CompressError> {
        encode_image_prepared(
            prepared,
            img,
            quality,
            self.format,
            self.params,
            self.jpeg_exif,
        )
    }
}

/// What one resize cycle learned when nothing fit.
struct CycleMiss {
    /// Output at the floor quality with the requested settings, when one
    /// was produced (reused as the best-effort result instead of encoding
    /// it again).
    floor_data: Option<Vec<u8>>,
    /// Size at the floor quality (or of the fast PNG trial encode); drives
    /// the downscale decision.
    floor_size: usize,
    /// Size at the initial quality (measured, or predicted when skipped).
    initial_size: Option<usize>,
    /// Observed slope of ln(size) over the quality scale (see
    /// [`quality_scale_log`]).
    slope: f64,
}

enum CycleOutcome {
    Fit { data: Vec<u8>, quality: u8 },
    Miss(CycleMiss),
}

/// Measurements carried from one resize cycle into the next.
struct CycleStats {
    pixels: u64,
    initial_size: Option<usize>,
    slope: f64,
}

/// Find the highest quality whose output fits in `params.max_file_size`,
/// downscaling when even the minimum quality does not fit.
///
/// Within a cycle the quality is found by interpolating ln(size) over the
/// JPEG quantizer scale (secant / regula falsi with a bisection safeguard),
/// which usually converges in 3-5 encodes instead of ~7-10 for a plain
/// binary search. When nothing fits, the size ratio decides how many of the
/// fixed 0.75 downscale steps are clearly needed, and they are taken at once
/// (each still counts against [`MAX_RESIZE_CYCLES`]); every downscale
/// resamples the original input once.
fn compress_to_target_size(
    mut img: DynamicImage,
    source: &DecodeSource<'_>,
    params: &CompressParams,
    output_format: DetectedFormat,
    jpeg_exif: Option<&[u8]>,
) -> Result<EngineResult, CompressError> {
    let ctx = EncodeCtx {
        params,
        format: output_format,
        jpeg_exif,
    };
    let target = params.max_file_size as usize;
    let initial_q = params.quality.min(100) as u8;
    let allow_resize = params.allow_resize != 0;
    let quality_search = supports_quality_search(output_format);
    // Lowest quality the search may use; also the best-effort quality.
    let floor_q = if quality_search {
        (params.min_quality.min(100) as u8).min(initial_q)
    } else {
        initial_q
    };
    // Bytes that do not scale with the pixel count (preserved EXIF).
    let overhead = jpeg_exif.map_or(0, <[u8]>::len);

    let mut iterations: u32 = 0;
    let mut resized = false;
    let mut resize_cycles: u32 = 0;
    let mut history: Option<CycleStats> = None;

    loop {
        let prepared = prepare_pixels(&img, output_format);
        let (w, h) = prepared.dimensions();
        let pixels = w as u64 * h as u64;
        let can_resize = allow_resize && resize_cycles < MAX_RESIZE_CYCLES;

        let outcome = if quality_search {
            search_quality_cycle(
                &ctx,
                &prepared,
                &img,
                initial_q,
                floor_q,
                target,
                history.as_ref().map(|prev| (prev, pixels)),
                &mut iterations,
            )?
        } else {
            fixed_quality_cycle(
                &ctx,
                &prepared,
                &img,
                initial_q,
                target,
                can_resize,
                &mut iterations,
            )?
        };

        let finish = |data: Vec<u8>, quality: u8, iterations: u32| {
            Ok(EngineResult {
                data,
                width: w,
                height: h,
                quality_used: quality as u32,
                iterations,
                resized_to_fit: resized,
            })
        };
        let miss = match outcome {
            CycleOutcome::Fit { data, quality } => return finish(data, quality, iterations),
            CycleOutcome::Miss(miss) => miss,
        };

        let steps = if can_resize {
            resize_steps(
                target.saturating_sub(overhead),
                miss.floor_size.saturating_sub(overhead),
                MAX_RESIZE_CYCLES - resize_cycles,
            )
        } else {
            0
        };
        let (new_w, new_h) = (0..steps).fold((w, h), |(cw, ch), _| {
            (resize_step_dimension(cw), resize_step_dimension(ch))
        });

        if steps == 0 || (new_w, new_h) == (w, h) {
            // Downscaling not allowed, exhausted or impossible: best effort
            // at the floor quality.
            let data = match miss.floor_data {
                Some(data) => data,
                None => {
                    iterations += 1;
                    ctx.encode(&prepared, &img, floor_q)?
                }
            };
            return finish(data, floor_q, iterations);
        }

        history = Some(CycleStats {
            pixels,
            initial_size: miss.initial_size,
            slope: miss.slope,
        });

        // Free this cycle's buffers before decoding again, then resample the
        // original input once to the new size (no blur accumulation).
        drop(prepared);
        drop(img);
        img = source.decode_at(Some((new_w, new_h)))?;
        resized = true;
        resize_cycles += steps;
    }
}

/// Number of [`RESIZE_SCALE_FACTOR`] steps to take at once (1 up to
/// `remaining`): further steps are only added while the floor-quality size,
/// scaled by the area ratio, would still be more than
/// [`CLEARLY_TOO_LARGE_FACTOR`]× the budget. The generous margin keeps the
/// jump from skipping a step that might have fit (downscaling can shrink
/// noisy images faster than their area), so the dimensions match what the
/// step-by-step search would have reached, with fewer encodes.
fn resize_steps(budget: usize, floor_size: usize, remaining: u32) -> u32 {
    let area_step = RESIZE_SCALE_FACTOR * RESIZE_SCALE_FACTOR;
    let limit = budget.max(1) as f64 * CLEARLY_TOO_LARGE_FACTOR;
    let mut predicted = floor_size as f64 * area_step;
    let mut steps = 1;
    while steps < remaining && predicted > limit {
        steps += 1;
        predicted *= area_step;
    }
    steps.min(remaining)
}

/// One downscale step of a side length (as before: ×0.75, at least 16 px,
/// never upscaling).
fn resize_step_dimension(current: u32) -> u32 {
    (((current as f64) * RESIZE_SCALE_FACTOR).max(16.0) as u32).min(current)
}

/// One cycle for formats without a quality knob (PNG, lossless WebP).
///
/// While further downscaling is possible, PNG trial encodes use the fast
/// [`PNG_TRIAL_LEVEL`] preset; only a size that (nearly) fits is encoded at
/// the requested level, and a returned fit is always a verified size.
fn fixed_quality_cycle(
    ctx: &EncodeCtx<'_>,
    prepared: &PreparedPixels<'_>,
    img: &DynamicImage,
    quality: u8,
    target: usize,
    can_resize: bool,
    iterations: &mut u32,
) -> Result<CycleOutcome, CompressError> {
    let miss = |data: Vec<u8>| {
        CycleOutcome::Miss(CycleMiss {
            floor_size: data.len(),
            initial_size: Some(data.len()),
            floor_data: Some(data),
            slope: DEFAULT_LOG_SIZE_SLOPE,
        })
    };

    let (img_w, img_h) = img.dimensions();
    let level = png_level(ctx.params, img_w, img_h);
    if ctx.format == DetectedFormat::Png && can_resize && level > PNG_TRIAL_LEVEL {
        let trial = encode_png(img, PNG_TRIAL_LEVEL)?;
        *iterations += 1;
        if trial.len() as f64 > target as f64 * PNG_TRIAL_MARGIN {
            return Ok(CycleOutcome::Miss(CycleMiss {
                floor_data: None,
                floor_size: trial.len(),
                initial_size: Some(trial.len()),
                slope: DEFAULT_LOG_SIZE_SLOPE,
            }));
        }

        let full = encode_png(img, level)?;
        *iterations += 1;
        // Verify the final encode; keep the (fitting) trial output if the
        // requested level happened to come out larger.
        let data = if full.len() <= target {
            full
        } else if trial.len() <= target {
            trial
        } else {
            return Ok(miss(full));
        };
        return Ok(CycleOutcome::Fit { data, quality });
    }

    let data = ctx.encode(prepared, img, quality)?;
    *iterations += 1;
    if data.len() <= target {
        return Ok(CycleOutcome::Fit { data, quality });
    }
    Ok(miss(data))
}

/// A measured (or predicted) output size at one quality.
#[derive(Clone, Copy)]
struct Sample {
    quality: u8,
    size: usize,
}

/// One cycle of the quality search (JPEG, lossy WebP): the highest quality
/// in `floor_q..=initial_q` whose output fits, or a miss carrying the floor
/// output.
///
/// `previous` carries the last cycle's statistics and this cycle's pixel
/// count after a downscale; it is used to skip a clearly hopeless
/// initial-quality attempt and to seed the interpolation slope.
#[allow(clippy::too_many_arguments)]
fn search_quality_cycle(
    ctx: &EncodeCtx<'_>,
    prepared: &PreparedPixels<'_>,
    img: &DynamicImage,
    initial_q: u8,
    floor_q: u8,
    target: usize,
    previous: Option<(&CycleStats, u64)>,
    iterations: &mut u32,
) -> Result<CycleOutcome, CompressError> {
    let mut encode = |quality: u8| {
        *iterations += 1;
        ctx.encode(prepared, img, quality)
    };

    // Size at the initial quality predicted from the previous cycle by the
    // area ratio.
    let predicted_initial = previous.and_then(|(prev, pixels)| {
        prev.initial_size
            .map(|size| size as f64 * pixels as f64 / prev.pixels.max(1) as f64)
    });
    let mut slope = previous.map_or(DEFAULT_LOG_SIZE_SLOPE, |(prev, _)| prev.slope);

    // Lowest quality known (or confidently predicted) not to fit.
    let mut hi = match predicted_initial {
        Some(p) if p > target as f64 * CLEARLY_TOO_LARGE_FACTOR && initial_q > floor_q => Sample {
            quality: initial_q,
            size: p as usize,
        },
        _ => {
            let data = encode(initial_q)?;
            if data.len() <= target {
                return Ok(CycleOutcome::Fit {
                    data,
                    quality: initial_q,
                });
            }
            if initial_q <= floor_q {
                return Ok(CycleOutcome::Miss(CycleMiss {
                    floor_size: data.len(),
                    initial_size: Some(data.len()),
                    floor_data: Some(data),
                    slope,
                }));
            }
            Sample {
                quality: initial_q,
                size: data.len(),
            }
        }
    };
    let initial_size = hi.size;

    // Highest quality known to fit, with its output.
    let mut lo: Option<(Sample, Vec<u8>)> = None;
    let mut floor_data: Option<Vec<u8>> = None;
    let mut encodes = 0u32;
    // Illinois weights: when the same bracket end survives twice in a row,
    // its residual is halved so the interpolation stops creeping from one
    // side (the size curve is not exactly linear).
    let mut last_fit: Option<bool> = None;
    let mut fit_weight = 1.0f64;
    let mut miss_weight = 1.0f64;

    while encodes < MAX_QUALITY_SEARCH_ENCODES {
        // The floor itself missed: nothing in range fits.
        if hi.quality <= floor_q {
            break;
        }
        let (lower, upper) = match &lo {
            Some((fit, _)) => (fit.quality + 1, hi.quality - 1),
            None => (floor_q, hi.quality - 1),
        };
        // Adjacent bracket: `lo` is the answer.
        if lower > upper {
            break;
        }

        let guess = match &lo {
            Some((fit, _)) => interpolate_quality(*fit, fit_weight, hi, miss_weight, target),
            None => extrapolate_quality(hi, target, slope),
        };
        let quality = guess.clamp(lower, upper);

        let data = encode(quality)?;
        encodes += 1;
        let fits = data.len() <= target;
        let sample = Sample {
            quality,
            size: data.len(),
        };
        if fits {
            fit_weight = 1.0;
            if last_fit == Some(true) {
                miss_weight *= 0.5;
            }
            lo = Some((sample, data));
        } else {
            miss_weight = 1.0;
            if last_fit == Some(false) {
                fit_weight *= 0.5;
            }
            if lo.is_none() {
                let observed = log_size_slope(sample, hi);
                if observed.is_finite() && observed > 0.05 {
                    slope = observed;
                }
            }
            if quality == floor_q {
                floor_data = Some(data);
            }
            hi = sample;
        }
        last_fit = Some(fits);
    }

    if let Some((fit, data)) = lo {
        return Ok(CycleOutcome::Fit {
            data,
            quality: fit.quality,
        });
    }

    // Nothing fit within the budget: the floor quality decides.
    let floor = match floor_data {
        Some(data) => data,
        None => {
            let data = encode(floor_q)?;
            if data.len() <= target {
                return Ok(CycleOutcome::Fit {
                    data,
                    quality: floor_q,
                });
            }
            data
        }
    };
    Ok(CycleOutcome::Miss(CycleMiss {
        floor_size: floor.len(),
        floor_data: Some(floor),
        initial_size: Some(initial_size),
        slope,
    }))
}

/// ln of the IJG quantizer scale (in percent) used for `quality`
/// (`5000/q` below 50, `200-2q` above). Output size is close to linear in
/// this value for both mozjpeg and libwebp, unlike in the raw quality, whose
/// size curve is much steeper near the top.
fn quality_scale_log(quality: f64) -> f64 {
    let q = quality.clamp(1.0, 100.0);
    let scale = if q < 50.0 {
        5000.0 / q
    } else {
        200.0 - 2.0 * q
    };
    scale.max(1.0).ln()
}

/// Inverse of [`quality_scale_log`] (continuous quality).
fn quality_from_scale_log(x: f64) -> f64 {
    let scale = x.exp();
    let q = if scale >= 100.0 {
        5000.0 / scale
    } else {
        (200.0 - scale) / 2.0
    };
    q.clamp(0.0, 100.0)
}

/// Slope of ln(size) against the quantizer scale between two samples
/// (positive: size falls as the scale grows).
fn log_size_slope(low_q: Sample, high_q: Sample) -> f64 {
    let dy = (high_q.size.max(1) as f64).ln() - (low_q.size.max(1) as f64).ln();
    let dx = quality_scale_log(low_q.quality as f64) - quality_scale_log(high_q.quality as f64);
    dy / dx
}

/// Quality at which ln(size/target), interpolated linearly over the
/// quantizer scale between `fit` and `miss` (residuals scaled by the
/// Illinois weights), reaches zero; rounded towards `fit`.
fn interpolate_quality(
    fit: Sample,
    fit_weight: f64,
    miss: Sample,
    miss_weight: f64,
    target: usize,
) -> u8 {
    let target = target.max(1) as f64;
    let y_fit = (fit.size.max(1) as f64 / target).ln() * fit_weight;
    let y_miss = (miss.size.max(1) as f64 / target).ln() * miss_weight;
    let x_fit = quality_scale_log(fit.quality as f64);
    let x_miss = quality_scale_log(miss.quality as f64);
    let span = y_miss - y_fit;
    if !span.is_finite() || span <= 0.0 {
        return fit.quality + (miss.quality - fit.quality) / 2;
    }
    let x = x_fit + (x_miss - x_fit) * (-y_fit / span);
    quality_from_scale_log(x).floor() as u8
}

/// Quality expected to reach the target from a single too-large sample,
/// assuming ln(size) falls by `slope` per unit of [`quality_scale_log`].
fn extrapolate_quality(miss: Sample, target: usize, slope: f64) -> u8 {
    let excess = (miss.size.max(1) as f64 / target.max(1) as f64).ln();
    let x = quality_scale_log(miss.quality as f64) + excess / slope;
    quality_from_scale_log(x).floor() as u8
}

fn supports_quality_search(format: DetectedFormat) -> bool {
    matches!(format, DetectedFormat::Jpeg | DetectedFormat::WebpLossy)
}

// ─── Encoding ────────────────────────────────────────────────────────────────

/// Encode using a DynamicImage — used for single-shot paths where pixel
/// conversion overhead is negligible (called once, not in a loop).
fn encode_image(
    img: &DynamicImage,
    quality: u8,
    format: DetectedFormat,
    params: &CompressParams,
    jpeg_exif: Option<&[u8]>,
) -> Result<Vec<u8>, CompressError> {
    match format {
        DetectedFormat::Jpeg => encode_jpeg(img, quality, params, jpeg_exif),
        DetectedFormat::Png => {
            let (w, h) = img.dimensions();
            encode_png(img, png_level(params, w, h))
        }
        DetectedFormat::WebpLossless => encode_webp_lossless(img, params),
        DetectedFormat::WebpLossy => encode_webp_lossy(img, quality),
    }
}

/// Encode using pre-converted pixel data — used in quality search loops
/// to avoid redundant to_rgb8()/to_rgba8() conversions per iteration.
/// PNG always encodes from the DynamicImage (it has no prepared buffer).
fn encode_image_prepared(
    prepared: &PreparedPixels<'_>,
    img: &DynamicImage,
    quality: u8,
    format: DetectedFormat,
    params: &CompressParams,
    jpeg_exif: Option<&[u8]>,
) -> Result<Vec<u8>, CompressError> {
    match format {
        DetectedFormat::Jpeg => match prepared {
            PreparedPixels::Gray {
                data,
                width,
                height,
            } => encode_jpeg_raw(
                JpegPixels::Gray(data.as_ref()),
                *width,
                *height,
                quality,
                params,
                jpeg_exif,
            ),
            PreparedPixels::Rgb {
                data,
                width,
                height,
            } => encode_jpeg_raw(
                JpegPixels::Rgb(data.as_ref()),
                *width,
                *height,
                quality,
                params,
                jpeg_exif,
            ),
            _ => encode_jpeg(img, quality, params, jpeg_exif),
        },
        DetectedFormat::Png => {
            let (w, h) = img.dimensions();
            encode_png(img, png_level(params, w, h))
        }
        DetectedFormat::WebpLossless => match prepared {
            PreparedPixels::Rgb {
                data,
                width,
                height,
            } => encode_webp_lossless_rgb_raw(data.as_ref(), *width, *height),
            PreparedPixels::Rgba {
                data,
                width,
                height,
            } => encode_webp_lossless_rgba_raw(data.as_ref(), *width, *height),
            _ => encode_webp_lossless(img, params),
        },
        DetectedFormat::WebpLossy => match prepared {
            PreparedPixels::Rgb {
                data,
                width,
                height,
            } => encode_webp_lossy_rgb_raw(data.as_ref(), *width, *height, quality),
            PreparedPixels::Rgba {
                data,
                width,
                height,
            } => encode_webp_lossy_rgba_raw(data.as_ref(), *width, *height, quality),
            _ => encode_webp_lossy(img, quality),
        },
    }
}

/// Pixel data accepted by the JPEG encoder.
#[derive(Clone, Copy)]
enum JpegPixels<'a> {
    /// 1 byte per pixel; encoded as a single-component (grayscale) JPEG.
    Gray(&'a [u8]),
    /// 3 bytes per pixel.
    Rgb(&'a [u8]),
}

fn encode_jpeg(
    img: &DynamicImage,
    quality: u8,
    params: &CompressParams,
    jpeg_exif: Option<&[u8]>,
) -> Result<Vec<u8>, CompressError> {
    // Avoid redundant allocation when image is already RGB8 (common after decode_jpeg_fast).
    // For a 4K image this saves ~36 MB of heap allocation per encode call.
    let owned;
    let (pixels, width, height) = match img {
        DynamicImage::ImageLuma8(gray) => {
            let (w, h) = gray.dimensions();
            (JpegPixels::Gray(gray.as_raw().as_slice()), w, h)
        }
        DynamicImage::ImageRgb8(rgb) => {
            let (w, h) = rgb.dimensions();
            (JpegPixels::Rgb(rgb.as_raw().as_slice()), w, h)
        }
        other => {
            owned = other.to_rgb8();
            let (w, h) = owned.dimensions();
            (JpegPixels::Rgb(owned.as_raw().as_slice()), w, h)
        }
    };

    encode_jpeg_raw(pixels, width, height, quality, params, jpeg_exif)
}

/// JPEG encoding from pre-converted RGB8 or Gray8 pixel data.
fn encode_jpeg_raw(
    pixels: JpegPixels<'_>,
    width: u32,
    height: u32,
    quality: u8,
    params: &CompressParams,
    jpeg_exif: Option<&[u8]>,
) -> Result<Vec<u8>, CompressError> {
    let use_trellis = params.jpeg_trellis != 0;
    let progressive = params.jpeg_progressive != 0;

    let preset = match (progressive, use_trellis) {
        (true, true) => Preset::ProgressiveBalanced,
        (true, false) => Preset::ProgressiveBalanced,
        (false, true) => Preset::BaselineBalanced,
        (false, false) => Preset::BaselineFastest,
    };

    let chroma = match ChromaSubsampling::from_u32(params.jpeg_chroma_subsampling) {
        ChromaSubsampling::Yuv420 => Subsampling::S420,
        ChromaSubsampling::Yuv422 => Subsampling::S422,
        ChromaSubsampling::Yuv444 => Subsampling::S444,
    };

    let trellis = if use_trellis {
        TrellisConfig::default()
    } else {
        TrellisConfig::disabled()
    };

    let mut encoder = mozjpeg_rs::Encoder::new(preset)
        .quality(quality)
        .progressive(progressive)
        .subsampling(chroma)
        .trellis(trellis);

    if let Some(exif) = jpeg_exif {
        encoder = encoder.exif_data(exif.to_vec());
    }

    let data = match pixels {
        JpegPixels::Gray(gray) => encoder.encode_gray(gray, width, height),
        JpegPixels::Rgb(rgb) => encoder.encode_rgb(rgb, width, height),
    }
    .map_err(|e| CompressError::EncodeError(format!("JPEG encode failed: {e}")))?;

    Ok(data)
}

// ─── PNG Encoding ────────────────────────────────────────────────────────────

/// Largest image (pixels) still encoded at a requested PNG level 5-6.
///
/// Worst-case (noise-heavy) 12 MP measurements on an Apple M-series machine:
/// level 2 ≈ 6 s, level 4 ≈ 10.6 s, level 6 ≈ 34 s (phones are several times
/// slower). oxipng's timeout cannot interrupt a deflate trial that is already
/// running, so it does not bound these costs; capping the level does. 3.75 MP
/// keeps level 6 at about the cost of level 4 at 12 MP (12 MP × 10.6 / 34).
pub(crate) const PNG_HIGH_LEVEL_MAX_PIXELS: u64 = 3_750_000;
/// Largest image (pixels) still encoded at a requested PNG level 3-4
/// (level 4 at 12 MP ≈ 10.6 s, see above). Larger images use level 2.
pub(crate) const PNG_MID_LEVEL_MAX_PIXELS: u64 = 12_000_000;
/// Levels up to this one (the default) are never lowered.
const PNG_ALWAYS_ALLOWED_LEVEL: u8 = 2;

/// oxipng preset (0-6) used for a `width`×`height` image: the requested
/// level, lowered for large images unless `png_level_unbounded` is set
/// (see [`PNG_HIGH_LEVEL_MAX_PIXELS`] / [`PNG_MID_LEVEL_MAX_PIXELS`]).
pub(crate) fn png_level(params: &CompressParams, width: u32, height: u32) -> u8 {
    let requested = params.png_optimization_level.min(6) as u8;
    if params.png_level_unbounded != 0 || requested <= PNG_ALWAYS_ALLOWED_LEVEL {
        return requested;
    }
    let pixels = width as u64 * height as u64;
    let cap = if pixels > PNG_MID_LEVEL_MAX_PIXELS {
        PNG_ALWAYS_ALLOWED_LEVEL
    } else if pixels > PNG_HIGH_LEVEL_MAX_PIXELS {
        4
    } else {
        6
    };
    requested.min(cap)
}

/// oxipng options for `level`, bounded by [`PNG_OPTIMIZE_TIMEOUT`].
pub(crate) fn png_options(level: u8) -> oxipng::Options {
    let mut options = oxipng::Options::from_preset(level);
    options.timeout = Some(PNG_OPTIMIZE_TIMEOUT);
    options
}

/// Split a decoded image into the PNG color type, bit depth and raw
/// (big-endian for 16-bit) sample bytes oxipng expects.
fn png_raw_parts(img: DynamicImage) -> (oxipng::ColorType, oxipng::BitDepth, Vec<u8>) {
    use oxipng::{BitDepth, ColorType};

    fn be_bytes(samples: Vec<u16>) -> Vec<u8> {
        samples.into_iter().flat_map(u16::to_be_bytes).collect()
    }

    let gray = ColorType::Grayscale {
        transparent_shade: None,
    };
    let rgb = ColorType::RGB {
        transparent_color: None,
    };
    match img {
        DynamicImage::ImageLuma8(b) => (gray, BitDepth::Eight, b.into_raw()),
        DynamicImage::ImageLumaA8(b) => (ColorType::GrayscaleAlpha, BitDepth::Eight, b.into_raw()),
        DynamicImage::ImageRgb8(b) => (rgb, BitDepth::Eight, b.into_raw()),
        DynamicImage::ImageRgba8(b) => (ColorType::RGBA, BitDepth::Eight, b.into_raw()),
        DynamicImage::ImageLuma16(b) => (gray, BitDepth::Sixteen, be_bytes(b.into_raw())),
        DynamicImage::ImageLumaA16(b) => (
            ColorType::GrayscaleAlpha,
            BitDepth::Sixteen,
            be_bytes(b.into_raw()),
        ),
        DynamicImage::ImageRgb16(b) => (rgb, BitDepth::Sixteen, be_bytes(b.into_raw())),
        DynamicImage::ImageRgba16(b) => {
            (ColorType::RGBA, BitDepth::Sixteen, be_bytes(b.into_raw()))
        }
        // Float images (no PNG representation): 8-bit, keeping alpha if any.
        other if other.color().has_alpha() => (
            ColorType::RGBA,
            BitDepth::Eight,
            other.to_rgba8().into_raw(),
        ),
        other => (rgb, BitDepth::Eight, other.to_rgb8().into_raw()),
    }
}

/// Encode an optimized PNG straight from the pixels with oxipng (no
/// intermediate PNG encode + re-decode). Consumes the image so its buffer
/// moves into oxipng without a copy.
fn encode_png_owned(img: DynamicImage, level: u8) -> Result<Vec<u8>, CompressError> {
    let (width, height) = img.dimensions();
    let (color, depth, data) = png_raw_parts(img);
    let raw = oxipng::RawImage::new(width, height, color, depth, data)
        .map_err(|e| CompressError::EncodeError(format!("PNG encode failed: {e}")))?;

    match raw.create_optimized_png(&png_options(level)) {
        Ok(png) => Ok(png),
        // The timeout expired before any trial completed: produce a valid
        // PNG with the fastest preset and no deadline.
        Err(_) => raw
            .create_optimized_png(&oxipng::Options::from_preset(0))
            .map_err(|e| CompressError::EncodeError(format!("PNG optimization failed: {e}"))),
    }
}

pub(crate) fn encode_png(img: &DynamicImage, level: u8) -> Result<Vec<u8>, CompressError> {
    encode_png_owned(img.clone(), level)
}

// ─── Utilities ───────────────────────────────────────────────────────────────

pub(crate) fn detect_format(data: &[u8]) -> Result<DetectedFormat, CompressError> {
    if data.len() < 4 {
        return Err(CompressError::DecodeError("Input too small".into()));
    }

    // JPEG: starts with FF D8 FF
    if data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        return Ok(DetectedFormat::Jpeg);
    }

    // PNG: starts with 89 50 4E 47 (‰PNG)
    if data[0] == 0x89 && data[1] == 0x50 && data[2] == 0x4E && data[3] == 0x47 {
        return Ok(DetectedFormat::Png);
    }

    // WebP: starts with RIFF....WEBP — check VP8 chunk to distinguish lossy/lossless
    if data.len() >= 16 && data[0..4] == *b"RIFF" && data[8..12] == *b"WEBP" {
        // VP8L = lossless, VP8 (lossy) or VP8X (extended) = lossy
        if data.len() >= 16 && &data[12..16] == b"VP8L" {
            return Ok(DetectedFormat::WebpLossless);
        }
        return Ok(DetectedFormat::WebpLossy);
    }

    // GIF: starts with GIF87a or GIF89a
    if data.len() >= 6 && &data[0..3] == b"GIF" {
        // GIF input accepted — will be decoded by image crate and re-encoded
        // as JPEG (auto format defaults to JPEG for non-native formats)
        return Ok(DetectedFormat::Jpeg);
    }

    // BMP: starts with BM
    if data[0] == 0x42 && data[1] == 0x4D {
        return Ok(DetectedFormat::Jpeg);
    }

    // TIFF: starts with II (little-endian) or MM (big-endian)
    if (data[0] == 0x49 && data[1] == 0x49) || (data[0] == 0x4D && data[1] == 0x4D) {
        return Ok(DetectedFormat::Jpeg);
    }

    Err(CompressError::UnsupportedFormat(
        "Unsupported format. Supported inputs: JPEG, PNG, WebP, GIF, BMP, TIFF".into(),
    ))
}

fn resolve_output_format(params: &CompressParams, detected: DetectedFormat) -> DetectedFormat {
    match OutputFormat::from_u32(params.format) {
        OutputFormat::Auto => detected,
        OutputFormat::Jpeg => DetectedFormat::Jpeg,
        OutputFormat::Png => DetectedFormat::Png,
        OutputFormat::WebpLossless => DetectedFormat::WebpLossless,
        OutputFormat::WebpLossy => DetectedFormat::WebpLossy,
    }
}

/// Downscale to fit `max_width`/`max_height`. Takes ownership so that the
/// original full-resolution image is dropped as soon as the resized copy
/// exists, instead of living until the end of the compression call.
fn apply_resize_constraints(img: DynamicImage, max_width: u32, max_height: u32) -> DynamicImage {
    let (orig_w, orig_h) = img.dimensions();
    match constrained_dimensions(orig_w, orig_h, max_width, max_height) {
        Some((new_w, new_h)) => resize_fast(&img, new_w, new_h),
        None => img,
    }
}

/// Target size for `max_width`/`max_height` (aspect preserved, never
/// upscaled), or `None` when no resize is needed.
pub(crate) fn constrained_dimensions(
    orig_w: u32,
    orig_h: u32,
    max_width: u32,
    max_height: u32,
) -> Option<(u32, u32)> {
    if max_width == 0 && max_height == 0 {
        return None;
    }

    let target_w = if max_width > 0 { max_width } else { orig_w };
    let target_h = if max_height > 0 { max_height } else { orig_h };

    if orig_w <= target_w && orig_h <= target_h {
        return None;
    }

    let scale_w = target_w as f64 / orig_w as f64;
    let scale_h = target_h as f64 / orig_h as f64;
    let scale = scale_w.min(scale_h);

    let new_w = ((orig_w as f64) * scale).round() as u32;
    let new_h = ((orig_h as f64) * scale).round() as u32;

    // resize_fast already clamps to min 1, but be explicit here too
    Some((new_w.max(1), new_h.max(1)))
}

// ─── WebP Encoding ───────────────────────────────────────────────────────────

fn encode_webp_lossless(
    img: &DynamicImage,
    _params: &CompressParams,
) -> Result<Vec<u8>, CompressError> {
    match img {
        DynamicImage::ImageRgb8(rgb) => {
            let (w, h) = rgb.dimensions();
            encode_webp_lossless_rgb_raw(rgb.as_raw(), w, h)
        }
        other => {
            let rgba = other.to_rgba8();
            let (w, h) = rgba.dimensions();
            encode_webp_lossless_rgba_raw(rgba.as_raw(), w, h)
        }
    }
}

fn encode_webp_lossless_rgb_raw(
    pixels: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, CompressError> {
    let mut buf = Vec::new();
    let encoder = image::codecs::webp::WebPEncoder::new_lossless(&mut buf);
    encoder
        .encode(pixels, width, height, image::ExtendedColorType::Rgb8)
        .map_err(|e| CompressError::EncodeError(format!("WebP encode failed: {e}")))?;
    Ok(buf)
}

fn encode_webp_lossless_rgba_raw(
    pixels: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, CompressError> {
    let mut buf = Vec::new();
    let encoder = image::codecs::webp::WebPEncoder::new_lossless(&mut buf);
    encoder
        .encode(pixels, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|e| CompressError::EncodeError(format!("WebP encode failed: {e}")))?;
    Ok(buf)
}

/// Encode image as lossy WebP using the `webpx` crate (libwebp bindings).
/// Uses RGB path when possible to avoid unnecessary RGBA conversion.
fn encode_webp_lossy(img: &DynamicImage, quality: u8) -> Result<Vec<u8>, CompressError> {
    match img {
        DynamicImage::ImageRgb8(rgb) => {
            encode_webp_lossy_rgb_raw(rgb.as_raw(), rgb.width(), rgb.height(), quality)
        }
        other => {
            let rgba = other.to_rgba8();
            encode_webp_lossy_rgba_raw(rgba.as_raw(), rgba.width(), rgba.height(), quality)
        }
    }
}

/// Map a `webpx` encode error to our error type. `webpx::Result` wraps the
/// error in `At<Error>` for source-location tracing; we only need the plain,
/// `Display`-able `webpx::Error`, obtained via the non-consuming `error()`
/// accessor (`into_inner()` is deprecated).
fn map_webpx_error(err: webpx::Result<Vec<u8>>) -> Result<Vec<u8>, CompressError> {
    err.map_err(|e| CompressError::EncodeError(format!("WebP encode failed: {}", e.error())))
}

fn encode_webp_lossy_rgb_raw(
    pixels: &[u8],
    width: u32,
    height: u32,
    quality: u8,
) -> Result<Vec<u8>, CompressError> {
    map_webpx_error(
        webpx::Encoder::new_rgb(pixels, width, height)
            .quality(quality as f32)
            .encode(webpx::Unstoppable),
    )
}

fn encode_webp_lossy_rgba_raw(
    pixels: &[u8],
    width: u32,
    height: u32,
    quality: u8,
) -> Result<Vec<u8>, CompressError> {
    map_webpx_error(
        webpx::Encoder::new_rgba(pixels, width, height)
            .quality(quality as f32)
            .encode(webpx::Unstoppable),
    )
}

// ─── Probe: Quick Metadata Without Full Decode ───────────────────────────────

/// Information extracted from image header without decoding pixel data.
pub struct ProbeInfo {
    pub width: u32,
    pub height: u32,
    pub format: DetectedFormat,
    pub file_size: usize,
    pub has_exif: bool,
    /// EXIF orientation 1-8 (1 when absent or invalid).
    pub orientation: u8,
}

/// Read image metadata from bytes without decoding the full image.
/// Only reads headers — very fast even on large files.
pub fn probe_bytes(data: &[u8]) -> Result<ProbeInfo, CompressError> {
    probe_source(&mut SliceSource(data))
}

/// Read image metadata from an open file without loading it into memory.
/// Only the header bytes (and, for JPEG/PNG, the marker/chunk headers used
/// for EXIF detection) are read. Produces the same result as
/// [`probe_bytes`] on the file's full contents.
pub fn probe_file(file: std::fs::File) -> Result<ProbeInfo, CompressError> {
    let len = file.metadata()?.len();
    let len = usize::try_from(len).map_err(|_| {
        CompressError::InvalidParameter(format!("File too large to probe ({len} bytes)"))
    })?;
    probe_source(&mut FileSource {
        reader: std::io::BufReader::new(file),
        pos: None,
        len,
    })
}

fn probe_source<S: ByteSource>(src: &mut S) -> Result<ProbeInfo, CompressError> {
    let len = src.len();

    // Every signature check only looks at the first 16 bytes.
    let mut header_buf = [0u8; 16];
    let header = &mut header_buf[..len.min(16)];
    src.read_at(0, header)?;
    let header = &*header;

    let format = detect_format(header)?;
    let format_hint = input_format_hint(header)
        .ok_or_else(|| CompressError::DecodeError("Failed to detect image header".into()))?;

    let (width, height) = src
        .dimensions(format_hint)
        .map_err(|e| CompressError::DecodeError(format!("Failed to read dimensions: {e}")))?;

    let has_exif = match format {
        DetectedFormat::Jpeg => find_exif_jpeg(src)?.is_some(),
        DetectedFormat::Png => find_exif_png(src)?.is_some(),
        DetectedFormat::WebpLossless | DetectedFormat::WebpLossy => false,
    };
    let orientation = orientation_from_source(src, format_hint);

    Ok(ProbeInfo {
        width,
        height,
        format,
        file_size: len,
        has_exif,
        orientation,
    })
}

/// Maximum number of pixels (width × height) accepted for decoding.
/// 100 MP is ~400 MB as RGBA8 — anything larger is almost certainly a
/// decompression bomb on a mobile device.
pub(crate) const MAX_PIXELS: u64 = 100_000_000;

/// Read image dimensions from the header only.
///
/// JPEG goes through zune-jpeg directly with the same options the `image`
/// crate uses for `into_dimensions()` (identical result), because the
/// `image` JPEG decoder buffers the entire input before parsing headers.
fn read_dimensions<R: BufRead + Seek>(
    reader: R,
    format: ImageFormat,
) -> Result<(u32, u32), String> {
    if format == ImageFormat::Jpeg {
        use zune_jpeg::zune_core::options::DecoderOptions;

        let options = DecoderOptions::default()
            .set_strict_mode(false)
            .set_max_width(usize::MAX)
            .set_max_height(usize::MAX);
        let mut decoder = zune_jpeg::JpegDecoder::new_with_options(reader, options);
        decoder.decode_headers().map_err(|e| e.to_string())?;
        let (w, h) = decoder
            .dimensions()
            .ok_or_else(|| "JPEG headers decoded without dimensions".to_string())?;
        return Ok((w as u32, h as u32));
    }

    ImageReader::with_format(reader, format)
        .into_dimensions()
        .map_err(|e| e.to_string())
}

pub(crate) fn ensure_pixel_limit(width: u32, height: u32) -> Result<(), CompressError> {
    let pixels = width as u64 * height as u64;
    if pixels > MAX_PIXELS {
        return Err(CompressError::ImageTooLarge(format!(
            "{width}x{height} ({pixels} pixels) exceeds the {MAX_PIXELS} pixel limit"
        )));
    }
    Ok(())
}

/// Format and dimensions read from an image header (no pixel decoding).
#[derive(Debug, Clone, Copy)]
pub(crate) struct HeaderInfo {
    pub format: DetectedFormat,
    /// Raw (stored, pre-orientation) dimensions.
    pub width: u32,
    pub height: u32,
    /// EXIF orientation 1-8 (1 when absent).
    pub orientation: u8,
    /// A real JPEG (eligible for DCT-scaled decoding), as opposed to the
    /// GIF/BMP/TIFF inputs that `format` also reports as JPEG.
    pub native_jpeg: bool,
}

fn header_info_from_source<S: ByteSource>(src: &mut S) -> Option<HeaderInfo> {
    let mut header_buf = [0u8; 16];
    let header = &mut header_buf[..src.len().min(16)];
    src.read_at(0, header).ok()?;
    let format = detect_format(header).ok()?;
    let hint = input_format_hint(header)?;
    let (width, height) = src.dimensions(hint).ok()?;
    Some(HeaderInfo {
        format,
        width,
        height,
        orientation: orientation_from_source(src, hint),
        native_jpeg: hint == ImageFormat::Jpeg,
    })
}

/// Header-only format/dimensions of an in-memory image, if readable.
pub(crate) fn header_info(data: &[u8]) -> Option<HeaderInfo> {
    header_info_from_source(&mut SliceSource(data))
}

/// Header-only format/dimensions of an image file, if readable. Reads only
/// the header bytes, never the whole file.
pub(crate) fn file_header_info(file: std::fs::File) -> Option<HeaderInfo> {
    let len = usize::try_from(file.metadata().ok()?.len()).ok()?;
    header_info_from_source(&mut FileSource {
        reader: std::io::BufReader::new(file),
        pos: None,
        len,
    })
}

/// Reject images whose header declares more than [`MAX_PIXELS`] pixels,
/// before any pixel buffer is allocated. Unreadable headers are left to the
/// decoder so that error reporting stays unchanged.
fn check_pixel_limit(data: &[u8]) -> Result<(), CompressError> {
    let Some(hint) = input_format_hint(data) else {
        return Ok(());
    };
    match read_dimensions(Cursor::new(data), hint) {
        Ok((w, h)) => ensure_pixel_limit(w, h),
        Err(_) => Ok(()),
    }
}

/// Estimated decode working set per input pixel (decoded RGB/RGBA buffer).
const DECODE_BYTES_PER_PIXEL: u64 = 4;
/// Estimated encode working set per output pixel for JPEG/WebP
/// (converted buffer + mozjpeg/libwebp internal planes and coefficients).
const LOSSY_ENCODE_BYTES_PER_PIXEL: u64 = 16;
/// Estimated encode working set per output pixel for PNG (raw encode +
/// oxipng's parallel filter/deflate trials).
const PNG_ENCODE_BYTES_PER_PIXEL: u64 = 40;

/// Rough peak-memory estimate for compressing one image, from header data
/// only. Calibrated against measured peaks of 12 MP JPEG/PNG compressions
/// (e.g. 12 MP JPEG → JPEG ≈ 210 MB, → 1920 px wide ≈ 90 MB).
///
/// Mirrors the decode pipeline: the resize bounds apply to the
/// display-oriented size (EXIF orientation 5-8 swaps the axes), and a JPEG
/// downscale decodes only the DCT-scaled size picked by
/// [`jpeg_scale_denominator`] (at least [`JPEG_SCALED_DECODE_MARGIN`]× the
/// target) instead of the full image. CMYK/16-bit JPEGs, which fall back to
/// a full decode, are rare enough to accept the under-estimate.
pub(crate) fn estimate_peak_memory(
    info: HeaderInfo,
    input_len: u64,
    params: &CompressParams,
) -> u64 {
    let pixels = info.width as u64 * info.height as u64;
    if pixels > MAX_PIXELS {
        // Rejected before decoding.
        return input_len;
    }

    let swaps = orientation_swaps_axes(info.orientation);
    let (disp_w, disp_h) = if swaps {
        (info.height, info.width)
    } else {
        (info.width, info.height)
    };
    let target = constrained_dimensions(disp_w, disp_h, params.max_width, params.max_height);
    let (out_w, out_h) = target.unwrap_or((disp_w, disp_h));
    let out_pixels = out_w as u64 * out_h as u64;

    let output_format = resolve_output_format(params, info.format);
    let direct_png = info.format == DetectedFormat::Png
        && output_format == DetectedFormat::Png
        && target.is_none()
        && params.max_file_size == 0;
    let decoded_pixels = match target {
        Some((tw, th)) if info.native_jpeg && info.format == DetectedFormat::Jpeg => {
            let (raw_tw, raw_th) = if swaps { (th, tw) } else { (tw, th) };
            let d = jpeg_scale_denominator(info.width, info.height, raw_tw, raw_th) as u64;
            (info.width as u64).div_ceil(d) * (info.height as u64).div_ceil(d)
        }
        _ => pixels,
    };
    let encode_bpp = match output_format {
        DetectedFormat::Png => PNG_ENCODE_BYTES_PER_PIXEL,
        _ => LOSSY_ENCODE_BYTES_PER_PIXEL,
    };
    // The direct PNG path (no resize) hands the file to oxipng, whose own
    // decode is covered by the encode working set.
    let decode_bytes = if direct_png {
        0
    } else {
        decoded_pixels.saturating_mul(DECODE_BYTES_PER_PIXEL)
    };

    input_len
        .saturating_add(decode_bytes)
        .saturating_add(out_pixels.saturating_mul(encode_bpp))
}

/// Random-access byte source for the header walkers, so the same code
/// probes in-memory buffers and files (without reading the whole file).
trait ByteSource {
    fn len(&self) -> usize;
    fn read_at(&mut self, offset: usize, buf: &mut [u8]) -> std::io::Result<()>;
    fn dimensions(&mut self, format: ImageFormat) -> Result<(u32, u32), String>;

    fn byte(&mut self, offset: usize) -> std::io::Result<u8> {
        let mut b = [0u8; 1];
        self.read_at(offset, &mut b)?;
        Ok(b[0])
    }
}

struct SliceSource<'a>(&'a [u8]);

impl ByteSource for SliceSource<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn read_at(&mut self, offset: usize, buf: &mut [u8]) -> std::io::Result<()> {
        let src = offset
            .checked_add(buf.len())
            .and_then(|end| self.0.get(offset..end))
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::UnexpectedEof))?;
        buf.copy_from_slice(src);
        Ok(())
    }

    fn dimensions(&mut self, format: ImageFormat) -> Result<(u32, u32), String> {
        read_dimensions(Cursor::new(self.0), format)
    }
}

struct FileSource {
    reader: std::io::BufReader<std::fs::File>,
    /// Current stream position, if known (enables buffer-preserving seeks).
    pos: Option<u64>,
    len: usize,
}

impl ByteSource for FileSource {
    fn len(&self) -> usize {
        self.len
    }

    fn read_at(&mut self, offset: usize, buf: &mut [u8]) -> std::io::Result<()> {
        let offset = offset as u64;
        match self.pos {
            Some(pos) if pos == offset => {}
            Some(pos) => self.reader.seek_relative(offset as i64 - pos as i64)?,
            None => {
                self.reader.seek(SeekFrom::Start(offset))?;
            }
        }
        self.pos = None;
        self.reader.read_exact(buf)?;
        self.pos = Some(offset + buf.len() as u64);
        Ok(())
    }

    fn dimensions(&mut self, format: ImageFormat) -> Result<(u32, u32), String> {
        self.pos = None;
        self.reader
            .seek(SeekFrom::Start(0))
            .map_err(|e| e.to_string())?;
        read_dimensions(&mut self.reader, format)
    }
}

/// Locate the EXIF APP1 segment of a JPEG by walking marker segments using
/// their length fields (never the compressed data, to avoid false
/// positives). Returns the offset and length of the EXIF payload after the
/// `Exif\0\0` identifier.
fn find_exif_jpeg<S: ByteSource>(src: &mut S) -> std::io::Result<Option<(usize, usize)>> {
    let len = src.len();
    if len < 4 || src.byte(0)? != 0xFF || src.byte(1)? != 0xD8 {
        return Ok(None);
    }

    let mut offset = 2usize;
    while offset + 4 <= len {
        if src.byte(offset)? != 0xFF {
            break;
        }

        // Skip padding 0xFF bytes
        let mut marker_offset = offset + 1;
        while marker_offset < len && src.byte(marker_offset)? == 0xFF {
            marker_offset += 1;
        }
        if marker_offset >= len {
            break;
        }

        let marker = src.byte(marker_offset)?;
        offset = marker_offset + 1;

        // SOS or EOI — no more metadata markers
        if marker == 0xDA || marker == 0xD9 {
            break;
        }

        // Standalone markers (no length field)
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            continue;
        }

        // Read segment length
        if offset + 2 > len {
            break;
        }
        let mut len_bytes = [0u8; 2];
        src.read_at(offset, &mut len_bytes)?;
        let segment_len = u16::from_be_bytes(len_bytes) as usize;
        if segment_len < 2 || offset + segment_len > len {
            break;
        }

        // APP1 marker with Exif header?
        if marker == 0xE1 && segment_len >= 8 {
            let mut ident = [0u8; 6];
            src.read_at(offset + 2, &mut ident)?;
            if &ident == b"Exif\0\0" {
                return Ok(Some((offset + 8, segment_len - 8)));
            }
        }

        offset += segment_len;
    }

    Ok(None)
}

/// Locate the `eXIf` chunk of a PNG by walking the chunk structure (length +
/// type pairs). Returns the chunk body's offset and declared length.
fn find_exif_png<S: ByteSource>(src: &mut S) -> std::io::Result<Option<(usize, usize)>> {
    let len = src.len();
    if len < 12 {
        return Ok(None);
    }

    // Skip PNG signature (8 bytes)
    let mut offset = 8usize;
    while offset + 8 <= len {
        let mut chunk_header = [0u8; 8];
        src.read_at(offset, &mut chunk_header)?;
        let chunk_len = u32::from_be_bytes([
            chunk_header[0],
            chunk_header[1],
            chunk_header[2],
            chunk_header[3],
        ]) as usize;
        let chunk_type = &chunk_header[4..8];

        if chunk_type == b"eXIf" {
            return Ok(Some((offset + 8, chunk_len)));
        }

        // IEND marks end of PNG
        if chunk_type == b"IEND" {
            break;
        }

        // Next chunk: length(4) + type(4) + data(chunk_len) + CRC(4)
        match offset
            .checked_add(12)
            .and_then(|o| o.checked_add(chunk_len))
        {
            Some(next) => offset = next,
            None => break,
        }
    }

    Ok(None)
}

/// Locate the `EXIF` chunk of a WebP (RIFF chunk walk).
fn find_exif_webp<S: ByteSource>(src: &mut S) -> std::io::Result<Option<(usize, usize)>> {
    let len = src.len();
    let mut offset = 12usize;
    while offset.checked_add(8).is_some_and(|end| end <= len) {
        let mut chunk_header = [0u8; 8];
        src.read_at(offset, &mut chunk_header)?;
        let size = u32::from_le_bytes([
            chunk_header[4],
            chunk_header[5],
            chunk_header[6],
            chunk_header[7],
        ]) as usize;
        if &chunk_header[0..4] == b"EXIF" {
            return Ok(Some((offset + 8, size)));
        }
        // Chunks are padded to an even size.
        match (offset + 8)
            .checked_add(size)
            .and_then(|end| end.checked_add(size & 1))
        {
            Some(next) => offset = next,
            None => break,
        }
    }
    Ok(None)
}

/// [`orientation_from_source`] over an in-memory buffer (tests only).
#[cfg(test)]
pub(crate) fn slice_orientation(data: &[u8]) -> u8 {
    match input_format_hint(data) {
        Some(hint) => orientation_from_source(&mut SliceSource(data), hint),
        None => 1,
    }
}

/// Largest EXIF block read to find the orientation (a JPEG APP1 segment is
/// at most 64 KB; larger PNG/WebP/TIFF metadata is truncated, which only
/// matters if IFD0 lies beyond it).
const MAX_EXIF_READ: usize = 1024 * 1024;

/// EXIF orientation (1-8) read through a [`ByteSource`] without loading the
/// image; same result as [`read_orientation`] on the full contents for
/// metadata within [`MAX_EXIF_READ`]. Read errors yield 1.
fn orientation_from_source<S: ByteSource>(src: &mut S, hint: ImageFormat) -> u8 {
    let location = match hint {
        ImageFormat::Jpeg => find_exif_jpeg(src),
        ImageFormat::Png => find_exif_png(src),
        ImageFormat::WebP => find_exif_webp(src),
        ImageFormat::Tiff => Ok(Some((0, src.len()))),
        _ => Ok(None),
    };
    let Ok(Some((offset, len))) = location else {
        return 1;
    };
    let available = src.len().saturating_sub(offset);
    let len = len.min(available).min(MAX_EXIF_READ);
    if len == 0 {
        return 1;
    }
    let mut chunk = vec![0u8; len];
    if src.read_at(offset, &mut chunk).is_err() {
        return 1;
    }
    let chunk = chunk.strip_prefix(b"Exif\0\0").unwrap_or(&chunk);
    Orientation::from_exif_chunk(chunk)
        .map(Orientation::to_exif)
        .unwrap_or(1)
}

// ─── Benchmark: Quality Sweep ────────────────────────────────────────────────

/// Single data point from a benchmark sweep.
pub struct BenchmarkEntry {
    pub quality: u32,
    pub size_bytes: usize,
    pub ratio: f32,
    pub encode_ms: u32,
}

/// Full benchmark result.
pub struct BenchmarkInfo {
    pub original_size: usize,
    pub width: u32,
    pub height: u32,
    pub format: DetectedFormat,
    pub entries: Vec<BenchmarkEntry>,
    pub recommended_quality: u32,
}

/// Run a quality sweep: encode the image at multiple quality levels
/// and measure size + speed for each. Returns a table that developers
/// can use to choose the optimal quality for their use case.
///
/// Only meaningful for JPEG (quality affects output size).
/// For PNG, returns a single entry since quality doesn't apply.
pub fn benchmark_bytes(
    data: &[u8],
    params: &CompressParams,
) -> Result<BenchmarkInfo, CompressError> {
    let format = detect_format(data)?;
    check_pixel_limit(data)?;

    // Same decode pipeline as compress_bytes (orientation, scaled JPEG
    // decode, resize constraints) so the benchmark matches real output.
    let img =
        DecodeSource::new(data, format).decode_constrained(params.max_width, params.max_height)?;
    let (width, height) = img.dimensions();
    let original_size = data.len();

    let output_format = resolve_output_format(params, format);

    let quality_levels: Vec<u32> = match output_format {
        DetectedFormat::Jpeg | DetectedFormat::WebpLossy => {
            vec![95, 90, 85, 80, 75, 70, 60, 50, 40, 30, 20]
        }
        // PNG/WebP lossless: quality doesn't apply, just benchmark once
        _ => vec![0],
    };

    // Pre-convert pixels once for the entire sweep (avoids 11× redundant
    // conversions). Borrowed when the layout already matches; PNG skips it.
    let prepared = prepare_pixels(&img, output_format);
    let mut entries = Vec::with_capacity(quality_levels.len());

    for &q in &quality_levels {
        let start = std::time::Instant::now();
        let encoded = encode_image_prepared(&prepared, &img, q as u8, output_format, params, None)?;
        let elapsed = start.elapsed().as_millis() as u32;

        entries.push(BenchmarkEntry {
            quality: q,
            size_bytes: encoded.len(),
            ratio: encoded.len() as f32 / original_size as f32,
            encode_ms: elapsed,
        });
    }

    // Find recommended quality: the "knee" of the curve.
    // Highest quality where size reduction per quality step is still > 2%.
    let recommended = find_recommended_quality(&entries, original_size);

    Ok(BenchmarkInfo {
        original_size,
        width,
        height,
        format,
        entries,
        recommended_quality: recommended,
    })
}

/// Find the "sweet spot" quality — where you get diminishing returns
/// from reducing quality further. Uses the elbow method.
pub(crate) fn find_recommended_quality(entries: &[BenchmarkEntry], original_size: usize) -> u32 {
    if entries.len() < 2 {
        return entries.first().map(|e| e.quality).unwrap_or(80);
    }

    // Score each quality level: ratio of quality to file size reduction.
    // We want the highest quality where we still get meaningful size savings.
    let mut best_q = entries[0].quality;
    let mut best_score = 0.0f32;

    for entry in entries {
        let reduction = 1.0 - (entry.size_bytes as f32 / original_size as f32);
        let quality_normalized = entry.quality as f32 / 100.0;

        // Score favors high quality + good reduction
        // Geometric mean ensures both matter
        let score = (quality_normalized * reduction).sqrt();

        if score > best_score {
            best_score = score;
            best_q = entry.quality;
        }
    }

    best_q
}

fn resolve_jpeg_exif_payload(
    input: &[u8],
    detected: DetectedFormat,
    output_format: DetectedFormat,
    params: &CompressParams,
) -> Result<Option<Vec<u8>>, CompressError> {
    if params.keep_metadata == 0 {
        return Ok(None);
    }

    match (detected, output_format) {
        (DetectedFormat::Jpeg, DetectedFormat::Jpeg) => Ok(extract_jpeg_exif_payload(input)),
        // Metadata preservation is only supported for JPEG→JPEG; for all other
        // format combinations silently skip rather than failing the operation.
        _ => Ok(None),
    }
}

/// EXIF payload to carry over into the output JPEG. The pixels are already
/// rotated/flipped into display orientation, so the Orientation tag is reset
/// to 1 (top-left); otherwise viewers would apply it a second time.
fn extract_jpeg_exif_payload(data: &[u8]) -> Option<Vec<u8>> {
    let mut payload = jpeg_exif_segment(data)?.to_vec();
    let _ = Orientation::remove_from_exif_chunk(&mut payload);
    Some(payload)
}
