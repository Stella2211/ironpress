use std::borrow::Cow;
use std::io::Cursor;

use image::{DynamicImage, GenericImageView, ImageFormat, ImageReader, RgbImage};
use mozjpeg_rs::{Preset, Subsampling, TrellisConfig};
use oxipng::StripChunks;

use crate::error::CompressError;
use crate::options::{ChromaSubsampling, CompressParams, OutputFormat};

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
        DetectedFormat::WebpLossless | DetectedFormat::WebpLossy => match img {
            DynamicImage::ImageRgb8(_) => prepared_rgb(img),
            _ => prepared_rgba(img),
        },
        DetectedFormat::Png => prepared_rgba(img),
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

    if can_direct_optimize_png(input, original_format, output_format, params) {
        return optimize_png_direct(input, params);
    }

    let img = decode_image_with_hint(input, original_format)?;
    let jpeg_exif = resolve_jpeg_exif_payload(input, original_format, output_format, params)?;

    // Apply explicit resize constraints first
    let img = apply_resize_constraints(&img, params.max_width, params.max_height);

    if params.max_file_size > 0 {
        compress_to_target_size(&img, params, output_format, jpeg_exif.as_deref())
    } else {
        let quality = params.quality.min(100) as u8;
        let encoded = encode_image(&img, quality, output_format, params, jpeg_exif.as_deref())?;
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
    jpeg_exif: Option<&[u8]>,
) -> Result<EngineResult, CompressError> {
    let target = params.max_file_size as usize;
    let min_q = params.min_quality.min(100) as u8;
    let initial_q = params.quality.min(100) as u8;
    let allow_resize = params.allow_resize != 0;
    let quality_search = supports_quality_search(output_format);

    let mut img = Cow::Borrowed(original_img);
    let mut total_iterations: u32 = 0;
    let mut resized = false;

    // Pre-convert pixel data once — avoids redundant to_rgb8()/to_rgba8()
    // on every binary search iteration (up to 10× per resize cycle).
    let mut prepared = prepare_pixels(&img, output_format);

    for _resize_cycle in 0..MAX_RESIZE_CYCLES {
        // First, try at the requested quality — often it's already small enough
        let initial_encoded =
            encode_image_prepared(&prepared, &img, initial_q, output_format, params, jpeg_exif)?;
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
                jpeg_exif,
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
                let fallback = encode_image_prepared(
                    &prepared,
                    &img,
                    min_q,
                    output_format,
                    params,
                    jpeg_exif,
                )?;
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
    let fallback = encode_image_prepared(
        &prepared,
        &img,
        fallback_q,
        output_format,
        params,
        jpeg_exif,
    )?;
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

fn supports_quality_search(format: DetectedFormat) -> bool {
    matches!(format, DetectedFormat::Jpeg | DetectedFormat::WebpLossy)
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
    jpeg_exif: Option<&[u8]>,
) -> Result<SearchResult, CompressError> {
    let mut lo = lo_init;
    let mut hi = hi_init;
    let mut best: Option<(Vec<u8>, u8)> = None;
    let mut iterations: u32 = 0;

    while lo <= hi && iterations < MAX_BINARY_SEARCH_ITERATIONS {
        let mid = lo + (hi - lo) / 2;
        let encoded = encode_image_prepared(prepared, img, mid, output_format, params, jpeg_exif)?;
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
    jpeg_exif: Option<&[u8]>,
) -> Result<Vec<u8>, CompressError> {
    match format {
        DetectedFormat::Jpeg => encode_jpeg(img, quality, params, jpeg_exif),
        DetectedFormat::Png => encode_png(img, params),
        DetectedFormat::WebpLossless => encode_webp_lossless(img, params),
        DetectedFormat::WebpLossy => encode_webp_lossy(img, quality),
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
    jpeg_exif: Option<&[u8]>,
) -> Result<Vec<u8>, CompressError> {
    match format {
        DetectedFormat::Jpeg => {
            if let PreparedPixels::Rgb {
                data,
                width,
                height,
            } = prepared
            {
                encode_jpeg_raw(data, *width, *height, quality, params, jpeg_exif)
            } else {
                encode_jpeg(img, quality, params, jpeg_exif)
            }
        }
        DetectedFormat::Png => encode_png(img, params),
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
    }
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

    encode_jpeg_raw(pixels, width, height, quality, params, jpeg_exif)
}

/// JPEG encoding from pre-converted RGB8 pixel data.
fn encode_jpeg_raw(
    pixels: &[u8],
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

    let data = encoder
        .encode_rgb(pixels, width, height)
        .map_err(|e| CompressError::EncodeError(format!("JPEG encode failed: {e}")))?;

    Ok(data)
}

fn encode_png(img: &DynamicImage, params: &CompressParams) -> Result<Vec<u8>, CompressError> {
    // Pre-allocate based on image size to reduce reallocation during encoding.
    let (w, h) = img.dimensions();
    let estimated = (w as usize * h as usize * 4) / 2;
    let mut buf = Vec::with_capacity(estimated.min(64 * 1024 * 1024));
    let mut cursor = Cursor::new(&mut buf);
    img.write_to(&mut cursor, ImageFormat::Png)
        .map_err(|e| CompressError::EncodeError(format!("PNG encode failed: {e}")))?;

    // Then optimize with oxipng
    let opt_level = oxipng::Options::from_preset(params.png_optimization_level.min(6) as u8);

    let optimized = oxipng::optimize_from_memory(&buf, &opt_level)
        .map_err(|e| CompressError::EncodeError(format!("PNG optimization failed: {e}")))?;

    Ok(optimized)
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

/// Map a `webpx` encode error to our error type, discarding the source-location
/// trace (`At<Error>`) and keeping the plain, `Display`-able `webpx::Error`.
fn map_webpx_error(err: webpx::Result<Vec<u8>>) -> Result<Vec<u8>, CompressError> {
    err.map_err(|e| CompressError::EncodeError(format!("WebP encode failed: {}", e.into_inner())))
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

    // Simple EXIF detection: search for EXIF marker in JPEG (APP1 segment with "Exif\0\0")
    let has_exif = match format {
        DetectedFormat::Jpeg => detect_exif_jpeg(data),
        DetectedFormat::Png => detect_exif_png(data),
        DetectedFormat::WebpLossless | DetectedFormat::WebpLossy => false,
    };

    Ok(ProbeInfo {
        width,
        height,
        format,
        file_size: data.len(),
        has_exif,
    })
}

/// Detect EXIF in JPEG by parsing marker segments using length fields.
/// Only walks marker segments (not compressed data) to avoid false positives.
fn detect_exif_jpeg(data: &[u8]) -> bool {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return false;
    }

    let mut offset = 2usize;
    while offset + 4 <= data.len() {
        if data[offset] != 0xFF {
            break;
        }

        // Skip padding 0xFF bytes
        let mut marker_offset = offset + 1;
        while marker_offset < data.len() && data[marker_offset] == 0xFF {
            marker_offset += 1;
        }
        if marker_offset >= data.len() {
            break;
        }

        let marker = data[marker_offset];
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
        if offset + 2 > data.len() {
            break;
        }
        let segment_len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
        if segment_len < 2 || offset + segment_len > data.len() {
            break;
        }

        // APP1 marker with Exif header?
        if marker == 0xE1 && segment_len >= 8 {
            let payload = &data[offset + 2..offset + segment_len];
            if payload.starts_with(b"Exif\0\0") {
                return true;
            }
        }

        offset += segment_len;
    }

    false
}

/// Detect EXIF in PNG by walking chunk structure (length + type pairs).
/// Avoids false positives from scanning compressed data.
fn detect_exif_png(data: &[u8]) -> bool {
    if data.len() < 12 {
        return false;
    }

    // Skip PNG signature (8 bytes)
    let mut offset = 8usize;
    while offset + 8 <= data.len() {
        let chunk_len = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        let chunk_type = &data[offset + 4..offset + 8];

        if chunk_type == b"eXIf" {
            return true;
        }

        // IEND marks end of PNG
        if chunk_type == b"IEND" {
            break;
        }

        // Next chunk: length(4) + type(4) + data(chunk_len) + CRC(4)
        offset += 12 + chunk_len;
    }

    false
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
    let img = decode_image_with_hint(data, format)?;

    // Apply resize constraints so benchmark matches real output
    let img = apply_resize_constraints(&img, params.max_width, params.max_height);
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

    // Pre-convert pixels once for the entire sweep (avoids 11× redundant conversions).
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

fn extract_jpeg_exif_payload(data: &[u8]) -> Option<Vec<u8>> {
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
            return Some(segment[6..].to_vec());
        }

        offset += segment_len;
    }

    None
}
