use std::borrow::Cow;
use std::io::Cursor;

use image::{DynamicImage, GenericImageView, ImageFormat, ImageReader, RgbImage};
use mozjpeg_rs::{Preset, Subsampling, TrellisConfig};
use oxipng::StripChunks;

use crate::error::CompressError;
use crate::metadata;
use crate::options::{ChromaSubsampling, CompressParams, OutputFormat};
use crate::quantize;

// ─── Fast JPEG Decoding ─────────────────────────────────────────────────────

/// Decode a JPEG using zune-jpeg (SIMD-accelerated, 2-4x faster than image crate).
/// Falls back to the image crate if zune-jpeg fails.
fn decode_jpeg_fast(data: &[u8]) -> Result<DynamicImage, CompressError> {
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;
    use zune_jpeg::JpegDecoder;

    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = JpegDecoder::new_with_options(Cursor::new(data), options);

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

    RgbImage::from_raw(width, height, pixels)
        .map(DynamicImage::ImageRgb8)
        .ok_or_else(|| CompressError::DecodeError("Pixel buffer size mismatch".into()))
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
        DynamicImage::ImageRgb8(_) | DynamicImage::ImageLuma8(_) => {
            resize_rgb8(img, new_w, new_h, &resize_opts)
        }
        _ => {
            // RGBA path: preserves alpha channel for PNG and other formats
            resize_rgba8(img, new_w, new_h, &resize_opts)
        }
    }
}

/// Resize as RGB8 (3 channels). Used for JPEG and grayscale inputs.
fn resize_rgb8(
    img: &DynamicImage,
    new_w: u32,
    new_h: u32,
    opts: &fast_image_resize::ResizeOptions,
) -> DynamicImage {
    use fast_image_resize as fir;

    let rgb = img.to_rgb8();
    let (src_w, src_h) = rgb.dimensions();

    let src =
        match fir::images::Image::from_vec_u8(src_w, src_h, rgb.into_raw(), fir::PixelType::U8x3) {
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

    let rgba = img.to_rgba8();
    let (src_w, src_h) = rgba.dimensions();

    let src = match fir::images::Image::from_vec_u8(
        src_w,
        src_h,
        rgba.into_raw(),
        fir::PixelType::U8x4,
    ) {
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

/// Maximum binary search iterations to prevent runaway loops.
const MAX_BINARY_SEARCH_ITERATIONS: u32 = 10;
/// Maximum resize-and-retry cycles.
const MAX_RESIZE_CYCLES: u32 = 4;
/// Scale factor applied when auto-resizing to meet file size target.
const RESIZE_SCALE_FACTOR: f64 = 0.75;

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
        && params.png_lossy == 0
        && !is_apng(input)
}

fn optimize_png_direct(
    input: &[u8],
    params: &CompressParams,
) -> Result<EngineResult, CompressError> {
    let probe = probe_bytes(input)?;
    let mut options = oxipng::Options::from_preset(params.png_optimization_level.min(6) as u8);
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
    /// Output-only: AVIF input decoding is not supported.
    Avif,
}

/// Pre-converted pixel buffer to avoid redundant `to_rgb8()`/`to_rgba8()`
/// calls during binary search iterations. For a 12MP image each conversion
/// allocates ~36 MB, so converting once instead of 10× eliminates ~360 MB
/// of allocation churn per compression call.
enum PreparedPixels {
    Rgb {
        data: Vec<u8>,
        width: u32,
        height: u32,
    },
    Rgba {
        data: Vec<u8>,
        width: u32,
        height: u32,
    },
}

impl PreparedPixels {
    fn dimensions(&self) -> (u32, u32) {
        match self {
            Self::Rgb { width, height, .. } | Self::Rgba { width, height, .. } => (*width, *height),
        }
    }
}

fn prepared_rgb(img: &DynamicImage) -> PreparedPixels {
    match img {
        DynamicImage::ImageRgb8(rgb) => {
            let (width, height) = rgb.dimensions();
            PreparedPixels::Rgb {
                data: rgb.as_raw().clone(),
                width,
                height,
            }
        }
        other => {
            let rgb = other.to_rgb8();
            let (width, height) = rgb.dimensions();
            PreparedPixels::Rgb {
                data: rgb.into_raw(),
                width,
                height,
            }
        }
    }
}

fn prepared_rgba(img: &DynamicImage) -> PreparedPixels {
    match img {
        DynamicImage::ImageRgba8(rgba) => {
            let (width, height) = rgba.dimensions();
            PreparedPixels::Rgba {
                data: rgba.as_raw().clone(),
                width,
                height,
            }
        }
        other => {
            let rgba = other.to_rgba8();
            let (width, height) = rgba.dimensions();
            PreparedPixels::Rgba {
                data: rgba.into_raw(),
                width,
                height,
            }
        }
    }
}

fn prepare_pixels(img: &DynamicImage, format: DetectedFormat) -> PreparedPixels {
    match format {
        DetectedFormat::Jpeg => prepared_rgb(img),
        DetectedFormat::WebpLossless | DetectedFormat::WebpLossy | DetectedFormat::Avif => {
            match img {
                DynamicImage::ImageRgb8(_) => prepared_rgb(img),
                _ => prepared_rgba(img),
            }
        }
        DetectedFormat::Png => prepared_rgba(img),
    }
}

/// Metadata carried from the input container into the encoded output.
#[derive(Default)]
pub struct MetaPayload {
    /// Raw TIFF EXIF payload for JPEG output. When pixels were physically
    /// rotated, the orientation tag inside has already been reset to upright.
    exif: Option<Vec<u8>>,
    /// ICC color profile, embedded into JPEG and PNG output.
    icc: Option<Vec<u8>>,
}

fn is_png_bytes(data: &[u8]) -> bool {
    data.len() >= 8 && data.starts_with(b"\x89PNG\r\n\x1a\n")
}

fn is_webp_bytes(data: &[u8]) -> bool {
    data.len() >= 16 && data[0..4] == *b"RIFF" && data[8..12] == *b"WEBP"
}

/// Pull the raw EXIF (TIFF) payload out of whichever container the input is.
fn extract_input_exif(input: &[u8]) -> Option<Vec<u8>> {
    if is_native_jpeg(input) {
        metadata::jpeg_extract_exif(input)
    } else if is_png_bytes(input) {
        metadata::png_extract_exif(input)
    } else if is_webp_bytes(input) {
        metadata::webp_extract_exif(input)
    } else {
        None
    }
}

/// Pull the ICC color profile out of whichever container the input is.
fn extract_input_icc(input: &[u8]) -> Option<Vec<u8>> {
    if is_native_jpeg(input) {
        metadata::jpeg_extract_icc(input)
    } else if is_png_bytes(input) {
        metadata::png_extract_icc(input)
    } else if is_webp_bytes(input) {
        metadata::webp_extract_icc(input)
    } else {
        None
    }
}

/// EXIF orientation of the input (JPEG APP1, PNG eXIf, or WebP EXIF chunk).
fn extract_input_orientation(input: &[u8]) -> Option<u16> {
    let tiff = extract_input_exif(input)?;
    metadata::exif_orientation(&tiff)
}

fn build_meta(
    input: &[u8],
    detected: DetectedFormat,
    output_format: DetectedFormat,
    params: &CompressParams,
    rotated: bool,
) -> MetaPayload {
    let mut meta = MetaPayload::default();

    // Full EXIF preservation is only supported for JPEG→JPEG; for all other
    // format combinations metadata is silently dropped.
    if params.keep_metadata != 0
        && detected == DetectedFormat::Jpeg
        && output_format == DetectedFormat::Jpeg
        && is_native_jpeg(input)
    {
        let mut exif = metadata::jpeg_extract_exif(input);
        if rotated {
            if let Some(tiff) = exif.as_mut() {
                metadata::patch_exif_orientation_to_upright(tiff);
            }
        }
        meta.exif = exif;
    }

    // ICC profiles ride along independently of keep_metadata — dropping them
    // visibly shifts colors on wide-gamut (Display P3) photos.
    if params.preserve_icc != 0
        && matches!(output_format, DetectedFormat::Jpeg | DetectedFormat::Png)
    {
        meta.icc = extract_input_icc(input);
    }

    meta
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

    // Orientation values > 1 mean the stored pixels need rotating.
    let orientation = if params.auto_orient != 0 {
        extract_input_orientation(input).filter(|&o| o > 1)
    } else {
        None
    };

    // The direct oxipng path never decodes pixels, so it can't rotate them.
    if orientation.is_none()
        && can_direct_optimize_png(input, original_format, output_format, params)
    {
        let mut result = optimize_png_direct(input, params)?;
        if params.preserve_icc != 0 {
            if let Some(icc) = metadata::png_extract_icc(input) {
                result.data = metadata::png_insert_iccp(result.data, &icc);
            }
        }
        return Ok(result);
    }

    let img = decode_image_with_hint(input, original_format)?;
    let img = match orientation {
        Some(o) => metadata::apply_orientation(img, o),
        None => img,
    };
    let meta = build_meta(
        input,
        original_format,
        output_format,
        params,
        orientation.is_some(),
    );

    // Apply explicit resize constraints first
    let img = apply_resize_constraints(&img, params.max_width, params.max_height);

    if params.max_file_size > 0 {
        compress_to_target_size(&img, params, output_format, &meta)
    } else {
        let quality = params.quality.min(100) as u8;
        let encoded = encode_image(&img, quality, output_format, params, &meta)?;
        let (w, h) = img.dimensions();
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

// ─── Target Size Binary Search ───────────────────────────────────────────────

fn compress_to_target_size(
    original_img: &DynamicImage,
    params: &CompressParams,
    output_format: DetectedFormat,
    meta: &MetaPayload,
) -> Result<EngineResult, CompressError> {
    let target = params.max_file_size as usize;
    let min_q = params.min_quality.min(100) as u8;
    let initial_q = params.quality.min(100) as u8;
    let allow_resize = params.allow_resize != 0;
    let quality_search = supports_quality_search(output_format, params);

    let mut img = Cow::Borrowed(original_img);
    let mut total_iterations: u32 = 0;
    let mut resized = false;

    // Pre-convert pixel data once — avoids redundant to_rgb8()/to_rgba8()
    // on every binary search iteration (up to 10× per resize cycle).
    let mut prepared = prepare_pixels(&img, output_format);

    for _resize_cycle in 0..MAX_RESIZE_CYCLES {
        // First, try at the requested quality — often it's already small enough
        let initial_encoded =
            encode_image_prepared(&prepared, &img, initial_q, output_format, params, meta)?;
        total_iterations += 1;

        if initial_encoded.len() <= target {
            let (w, h) = prepared.dimensions();
            return Ok(EngineResult {
                data: initial_encoded,
                width: w,
                height: h,
                quality_used: initial_q as u32,
                iterations: total_iterations,
                resized_to_fit: resized,
            });
        }

        if !quality_search {
            if !allow_resize {
                let (w, h) = prepared.dimensions();
                return Ok(EngineResult {
                    data: initial_encoded,
                    width: w,
                    height: h,
                    quality_used: initial_q as u32,
                    iterations: total_iterations,
                    resized_to_fit: resized,
                });
            }
        } else {
            // Binary search: find highest quality that fits under target
            let search_result = binary_search_quality(
                &prepared,
                &img,
                min_q,
                initial_q.saturating_sub(1),
                target,
                output_format,
                params,
                meta,
            )?;
            total_iterations += search_result.iterations;

            if let Some((best_data, best_q)) = search_result.best {
                let (w, h) = prepared.dimensions();
                return Ok(EngineResult {
                    data: best_data,
                    width: w,
                    height: h,
                    quality_used: best_q as u32,
                    iterations: total_iterations,
                    resized_to_fit: resized,
                });
            }

            // Even min_quality didn't fit — try resize if allowed
            if !allow_resize {
                // Encode at min quality as best effort
                let fallback =
                    encode_image_prepared(&prepared, &img, min_q, output_format, params, meta)?;
                total_iterations += 1;
                let (w, h) = prepared.dimensions();
                return Ok(EngineResult {
                    data: fallback,
                    width: w,
                    height: h,
                    quality_used: min_q as u32,
                    iterations: total_iterations,
                    resized_to_fit: false,
                });
            }
        }

        // Downscale and retry
        let (cur_w, cur_h) = img.dimensions();
        let new_w = ((cur_w as f64) * RESIZE_SCALE_FACTOR).max(16.0) as u32;
        let new_h = ((cur_h as f64) * RESIZE_SCALE_FACTOR).max(16.0) as u32;

        if new_w == cur_w && new_h == cur_h {
            // Can't shrink further
            break;
        }

        img = Cow::Owned(resize_fast(&img, new_w, new_h));
        resized = true;
        // Re-prepare pixels from resized image
        prepared = prepare_pixels(&img, output_format);
    }

    let fallback_q = if quality_search { min_q } else { initial_q };
    let fallback = encode_image_prepared(&prepared, &img, fallback_q, output_format, params, meta)?;
    total_iterations += 1;
    let (w, h) = prepared.dimensions();
    Ok(EngineResult {
        data: fallback,
        width: w,
        height: h,
        quality_used: fallback_q as u32,
        iterations: total_iterations,
        resized_to_fit: resized,
    })
}

fn supports_quality_search(format: DetectedFormat, params: &CompressParams) -> bool {
    match format {
        DetectedFormat::Jpeg | DetectedFormat::WebpLossy | DetectedFormat::Avif => true,
        // Lossy PNG maps quality to palette size, so searching works.
        DetectedFormat::Png => params.png_lossy != 0,
        DetectedFormat::WebpLossless => false,
    }
}

struct SearchResult {
    best: Option<(Vec<u8>, u8)>,
    iterations: u32,
}

#[allow(clippy::too_many_arguments)]
fn binary_search_quality(
    prepared: &PreparedPixels,
    img: &DynamicImage,
    lo_init: u8,
    hi_init: u8,
    target: usize,
    output_format: DetectedFormat,
    params: &CompressParams,
    meta: &MetaPayload,
) -> Result<SearchResult, CompressError> {
    let mut lo = lo_init;
    let mut hi = hi_init;
    let mut best: Option<(Vec<u8>, u8)> = None;
    let mut iterations: u32 = 0;

    while lo <= hi && iterations < MAX_BINARY_SEARCH_ITERATIONS {
        let mid = lo + (hi - lo) / 2;
        let encoded = encode_image_prepared(prepared, img, mid, output_format, params, meta)?;
        iterations += 1;

        if encoded.len() <= target {
            // Fits! Try higher quality
            best = Some((encoded, mid));
            if mid == hi {
                break;
            }
            lo = mid + 1;
        } else {
            // Too big — try lower quality
            if mid == lo {
                break;
            }
            hi = mid - 1;
        }
    }

    Ok(SearchResult { best, iterations })
}

// ─── Encoding ────────────────────────────────────────────────────────────────

/// Encode using a DynamicImage — used for single-shot paths where pixel
/// conversion overhead is negligible (called once, not in a loop).
fn encode_image(
    img: &DynamicImage,
    quality: u8,
    format: DetectedFormat,
    params: &CompressParams,
    meta: &MetaPayload,
) -> Result<Vec<u8>, CompressError> {
    match format {
        DetectedFormat::Jpeg => encode_jpeg(img, quality, params, meta),
        DetectedFormat::Png => encode_png(img, quality, params, meta),
        DetectedFormat::WebpLossless => encode_webp_lossless(img, params),
        DetectedFormat::WebpLossy => encode_webp_lossy(img, quality),
        DetectedFormat::Avif => encode_avif(img, quality, params),
    }
}

/// Encode using pre-converted pixel data — used in binary search loops
/// to avoid redundant to_rgb8()/to_rgba8() conversions per iteration.
/// Falls back to DynamicImage for PNG (which doesn't participate in
/// quality-based binary search in practice).
fn encode_image_prepared(
    prepared: &PreparedPixels,
    img: &DynamicImage,
    quality: u8,
    format: DetectedFormat,
    params: &CompressParams,
    meta: &MetaPayload,
) -> Result<Vec<u8>, CompressError> {
    match format {
        DetectedFormat::Jpeg => {
            if let PreparedPixels::Rgb {
                data,
                width,
                height,
            } = prepared
            {
                encode_jpeg_raw(data, *width, *height, quality, params, meta)
            } else {
                encode_jpeg(img, quality, params, meta)
            }
        }
        DetectedFormat::Png => match prepared {
            PreparedPixels::Rgba {
                data,
                width,
                height,
            } if params.png_lossy != 0 => {
                encode_png_lossy_raw(data, *width, *height, quality, params, meta)
            }
            _ => encode_png(img, quality, params, meta),
        },
        DetectedFormat::WebpLossless => match prepared {
            PreparedPixels::Rgb {
                data,
                width,
                height,
            } => encode_webp_lossless_rgb_raw(data, *width, *height),
            PreparedPixels::Rgba {
                data,
                width,
                height,
            } => encode_webp_lossless_rgba_raw(data, *width, *height),
        },
        DetectedFormat::WebpLossy => match prepared {
            PreparedPixels::Rgb {
                data,
                width,
                height,
            } => encode_webp_lossy_rgb_raw(data, *width, *height, quality),
            PreparedPixels::Rgba {
                data,
                width,
                height,
            } => encode_webp_lossy_rgba_raw(data, *width, *height, quality),
        },
        DetectedFormat::Avif => match prepared {
            PreparedPixels::Rgb {
                data,
                width,
                height,
            } => encode_avif_rgb_raw(data, *width, *height, quality, params),
            PreparedPixels::Rgba {
                data,
                width,
                height,
            } => encode_avif_rgba_raw(data, *width, *height, quality, params),
        },
    }
}

fn encode_jpeg(
    img: &DynamicImage,
    quality: u8,
    params: &CompressParams,
    meta: &MetaPayload,
) -> Result<Vec<u8>, CompressError> {
    // Avoid redundant allocation when image is already RGB8 (common after decode_jpeg_fast).
    // For a 4K image this saves ~36 MB of heap allocation per encode call.
    let owned;
    let (pixels, width, height) = match img {
        DynamicImage::ImageRgb8(rgb) => {
            let (w, h) = rgb.dimensions();
            (rgb.as_raw().as_slice(), w, h)
        }
        other => {
            owned = other.to_rgb8();
            let (w, h) = owned.dimensions();
            (owned.as_raw().as_slice(), w, h)
        }
    };

    encode_jpeg_raw(pixels, width, height, quality, params, meta)
}

/// JPEG encoding from pre-converted RGB8 pixel data.
fn encode_jpeg_raw(
    pixels: &[u8],
    width: u32,
    height: u32,
    quality: u8,
    params: &CompressParams,
    meta: &MetaPayload,
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

    if let Some(exif) = &meta.exif {
        encoder = encoder.exif_data(exif.clone());
    }
    if let Some(icc) = &meta.icc {
        encoder = encoder.icc_profile(icc.clone());
    }

    let data = encoder
        .encode_rgb(pixels, width, height)
        .map_err(|e| CompressError::EncodeError(format!("JPEG encode failed: {e}")))?;

    Ok(data)
}

fn encode_png(
    img: &DynamicImage,
    quality: u8,
    params: &CompressParams,
    meta: &MetaPayload,
) -> Result<Vec<u8>, CompressError> {
    if params.png_lossy != 0 {
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        return encode_png_lossy_raw(rgba.as_raw(), w, h, quality, params, meta);
    }

    // Pre-allocate based on image size to reduce reallocation during encoding.
    let (w, h) = img.dimensions();
    let estimated = (w as usize * h as usize * 4) / 2;
    let mut buf = Vec::with_capacity(estimated.min(64 * 1024 * 1024));
    let mut cursor = Cursor::new(&mut buf);
    img.write_to(&mut cursor, ImageFormat::Png)
        .map_err(|e| CompressError::EncodeError(format!("PNG encode failed: {e}")))?;

    optimize_and_finish_png(&buf, params, meta)
}

/// Lossy PNG from pre-converted RGBA8 pixel data: palette quantization with
/// dithering, then the regular oxipng pass.
fn encode_png_lossy_raw(
    pixels: &[u8],
    width: u32,
    height: u32,
    quality: u8,
    params: &CompressParams,
    meta: &MetaPayload,
) -> Result<Vec<u8>, CompressError> {
    let indexed = quantize::encode_indexed_png(pixels, width, height, quality as u32)?;
    optimize_and_finish_png(&indexed, params, meta)
}

/// Shared oxipng pass + ICC re-insertion for all PNG output paths.
fn optimize_and_finish_png(
    encoded: &[u8],
    params: &CompressParams,
    meta: &MetaPayload,
) -> Result<Vec<u8>, CompressError> {
    let opt_level = oxipng::Options::from_preset(params.png_optimization_level.min(6) as u8);

    let optimized = oxipng::optimize_from_memory(encoded, &opt_level)
        .map_err(|e| CompressError::EncodeError(format!("PNG optimization failed: {e}")))?;

    Ok(match &meta.icc {
        Some(icc) => metadata::png_insert_iccp(optimized, icc),
        None => optimized,
    })
}

// ─── AVIF Encoding ───────────────────────────────────────────────────────────

fn avif_encoder(quality: u8, params: &CompressParams) -> ravif::Encoder {
    ravif::Encoder::new()
        .with_quality(quality.clamp(1, 100) as f32)
        .with_speed(params.avif_speed.clamp(1, 10) as u8)
}

fn encode_avif(
    img: &DynamicImage,
    quality: u8,
    params: &CompressParams,
) -> Result<Vec<u8>, CompressError> {
    match img {
        DynamicImage::ImageRgb8(rgb) => {
            encode_avif_rgb_raw(rgb.as_raw(), rgb.width(), rgb.height(), quality, params)
        }
        other => {
            let rgba = other.to_rgba8();
            encode_avif_rgba_raw(rgba.as_raw(), rgba.width(), rgba.height(), quality, params)
        }
    }
}

fn encode_avif_rgb_raw(
    pixels: &[u8],
    width: u32,
    height: u32,
    quality: u8,
    params: &CompressParams,
) -> Result<Vec<u8>, CompressError> {
    use rgb::FromSlice;

    let img = ravif::Img::new(pixels.as_rgb(), width as usize, height as usize);
    let encoded = avif_encoder(quality, params)
        .encode_rgb(img)
        .map_err(|e| CompressError::EncodeError(format!("AVIF encode failed: {e}")))?;
    Ok(encoded.avif_file)
}

fn encode_avif_rgba_raw(
    pixels: &[u8],
    width: u32,
    height: u32,
    quality: u8,
    params: &CompressParams,
) -> Result<Vec<u8>, CompressError> {
    use rgb::FromSlice;

    let img = ravif::Img::new(pixels.as_rgba(), width as usize, height as usize);
    let encoded = avif_encoder(quality, params)
        .encode_rgba(img)
        .map_err(|e| CompressError::EncodeError(format!("AVIF encode failed: {e}")))?;
    Ok(encoded.avif_file)
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

    // ISO-BMFF (HEIC/HEIF/AVIF): "ftyp" box at offset 4. Recognized so the
    // error can say what the file is and what to do about it.
    if data.len() >= 12 && &data[4..8] == b"ftyp" {
        let brand = &data[8..12];
        if matches!(brand, b"avif" | b"avis") {
            return Err(CompressError::UnsupportedFormat(
                "AVIF input is not supported; ironpress can write AVIF but not read it".into(),
            ));
        }
        if matches!(
            brand,
            b"heic"
                | b"heix"
                | b"hevc"
                | b"hevx"
                | b"heim"
                | b"heis"
                | b"hevm"
                | b"hevs"
                | b"mif1"
                | b"msf1"
        ) {
            return Err(CompressError::UnsupportedFormat(
                "HEIC/HEIF input is not supported (HEVC decoding requires the \
                 patent-encumbered libheif C stack). Request JPEG from the platform \
                 image picker instead — e.g. image_picker on iOS already transcodes \
                 HEIC to JPEG"
                    .into(),
            ));
        }
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
        OutputFormat::Avif => DetectedFormat::Avif,
    }
}

fn apply_resize_constraints<'a>(
    img: &'a DynamicImage,
    max_width: u32,
    max_height: u32,
) -> Cow<'a, DynamicImage> {
    if max_width == 0 && max_height == 0 {
        return Cow::Borrowed(img);
    }

    let (orig_w, orig_h) = img.dimensions();
    let target_w = if max_width > 0 { max_width } else { orig_w };
    let target_h = if max_height > 0 { max_height } else { orig_h };

    if orig_w <= target_w && orig_h <= target_h {
        return Cow::Borrowed(img);
    }

    let scale_w = target_w as f64 / orig_w as f64;
    let scale_h = target_h as f64 / orig_h as f64;
    let scale = scale_w.min(scale_h);

    let new_w = ((orig_w as f64) * scale).round() as u32;
    let new_h = ((orig_h as f64) * scale).round() as u32;

    // resize_fast already clamps to min 1, but be explicit here too
    Cow::Owned(resize_fast(img, new_w.max(1), new_h.max(1)))
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

/// Encode image as lossy WebP using the `webp` crate.
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

fn encode_webp_lossy_rgb_raw(
    pixels: &[u8],
    width: u32,
    height: u32,
    quality: u8,
) -> Result<Vec<u8>, CompressError> {
    let encoder = webp::Encoder::from_rgb(pixels, width, height);
    let mem = encoder.encode(quality as f32);
    Ok(mem.to_vec())
}

fn encode_webp_lossy_rgba_raw(
    pixels: &[u8],
    width: u32,
    height: u32,
    quality: u8,
) -> Result<Vec<u8>, CompressError> {
    let encoder = webp::Encoder::from_rgba(pixels, width, height);
    let mem = encoder.encode(quality as f32);
    Ok(mem.to_vec())
}

// ─── Probe: Quick Metadata Without Full Decode ───────────────────────────────

/// Information extracted from image header without decoding pixel data.
pub struct ProbeInfo {
    pub width: u32,
    pub height: u32,
    pub format: DetectedFormat,
    pub file_size: usize,
    pub has_exif: bool,
}

/// Read image metadata from bytes without decoding the full image.
/// Only reads headers — very fast even on large files.
pub fn probe_bytes(data: &[u8]) -> Result<ProbeInfo, CompressError> {
    let format = detect_format(data)?;

    let format_hint = input_format_hint(data)
        .ok_or_else(|| CompressError::DecodeError("Failed to detect image header".into()))?;
    let reader = ImageReader::with_format(Cursor::new(data), format_hint);

    let (width, height) = reader
        .into_dimensions()
        .map_err(|e| CompressError::DecodeError(format!("Failed to read dimensions: {e}")))?;

    // Structured EXIF detection per container (marker segments / chunks only,
    // never compressed data, to avoid false positives).
    let has_exif = match format {
        DetectedFormat::Jpeg => metadata::jpeg_has_exif(data),
        DetectedFormat::Png => metadata::png_has_exif(data),
        DetectedFormat::WebpLossless | DetectedFormat::WebpLossy => metadata::webp_has_exif(data),
        DetectedFormat::Avif => false,
    };

    Ok(ProbeInfo {
        width,
        height,
        format,
        file_size: data.len(),
        has_exif,
    })
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
/// Only meaningful for quality-driven formats (JPEG, lossy WebP, AVIF,
/// lossy PNG). For lossless output, returns a single entry.
pub fn benchmark_bytes(
    data: &[u8],
    params: &CompressParams,
) -> Result<BenchmarkInfo, CompressError> {
    let format = detect_format(data)?;
    let img = decode_image_with_hint(data, format)?;

    // Match the real compression pipeline: orient, then resize.
    let img = if params.auto_orient != 0 {
        match extract_input_orientation(data).filter(|&o| o > 1) {
            Some(o) => metadata::apply_orientation(img, o),
            None => img,
        }
    } else {
        img
    };

    // Apply resize constraints so benchmark matches real output
    let img = apply_resize_constraints(&img, params.max_width, params.max_height);
    let (width, height) = img.dimensions();
    let original_size = data.len();

    let output_format = resolve_output_format(params, format);

    let quality_levels: Vec<u32> = if supports_quality_search(output_format, params) {
        vec![95, 90, 85, 80, 75, 70, 60, 50, 40, 30, 20]
    } else {
        // Lossless output: quality doesn't apply, just benchmark once
        vec![0]
    };

    // Pre-convert pixels once for the entire sweep (avoids 11× redundant conversions).
    let prepared = prepare_pixels(&img, output_format);
    let mut entries = Vec::with_capacity(quality_levels.len());
    let meta = MetaPayload::default();

    for &q in &quality_levels {
        let start = std::time::Instant::now();
        let encoded =
            encode_image_prepared(&prepared, &img, q as u8, output_format, params, &meta)?;
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
