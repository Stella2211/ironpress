/// Output format for compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum OutputFormat {
    /// Keep the same format as input
    Auto = 0,
    /// JPEG output
    Jpeg = 1,
    /// PNG output
    Png = 2,
    /// WebP output (lossless — smaller than PNG for graphics/screenshots)
    WebpLossless = 3,
    /// WebP output (lossy — often more useful than lossless for photos)
    WebpLossy = 4,
    /// AVIF output (lossy — ~50% smaller than JPEG at equivalent quality,
    /// but much slower to encode)
    Avif = 5,
}

impl OutputFormat {
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Jpeg,
            2 => Self::Png,
            3 => Self::WebpLossless,
            4 => Self::WebpLossy,
            5 => Self::Avif,
            _ => Self::Auto,
        }
    }
}

/// Chroma subsampling mode for JPEG encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ChromaSubsampling {
    /// 4:2:0 — best compression, slight color blurring (default)
    Yuv420 = 0,
    /// 4:2:2 — balanced
    Yuv422 = 1,
    /// 4:4:4 — no chroma loss, larger files
    Yuv444 = 2,
}

impl ChromaSubsampling {
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Yuv422,
            2 => Self::Yuv444,
            _ => Self::Yuv420,
        }
    }
}

/// Full compression parameters, passed from Dart via FFI.
#[repr(C)]
pub struct CompressParams {
    /// JPEG quality 0-100 (default 80). Ignored for PNG.
    pub quality: u32,
    /// Maximum width constraint. 0 = no constraint.
    pub max_width: u32,
    /// Maximum height constraint. 0 = no constraint.
    pub max_height: u32,
    /// Target maximum file size in bytes. 0 = disabled.
    pub max_file_size: u32,
    /// Minimum quality floor for max_file_size binary search (default 30).
    pub min_quality: u32,
    /// Allow automatic downscale if quality alone can't hit max_file_size target.
    /// 1 = true, 0 = false.
    pub allow_resize: u32,
    /// Output format: 0=auto, 1=jpeg, 2=png.
    pub format: u32,
    /// Keep EXIF/metadata. 1 = true, 0 = false.
    pub keep_metadata: u32,
    /// JPEG progressive encoding. 1 = true (default), 0 = false.
    pub jpeg_progressive: u32,
    /// Chroma subsampling: 0=4:2:0, 1=4:2:2, 2=4:4:4.
    pub jpeg_chroma_subsampling: u32,
    /// JPEG trellis quantization (mozjpeg feature). 1 = true (default), 0 = false.
    pub jpeg_trellis: u32,
    /// PNG optimization level 0-6 (default 2).
    pub png_optimization_level: u32,
    /// Lossy PNG output: quantize to a palette (up to 256 colors) with
    /// Floyd–Steinberg dithering before optimization. 1 = true, 0 = false (default).
    pub png_lossy: u32,
    /// Physically rotate pixels to match the EXIF orientation tag.
    /// 1 = true (default), 0 = false.
    pub auto_orient: u32,
    /// Preserve the input's ICC color profile in JPEG and PNG output.
    /// 1 = true (default), 0 = false.
    pub preserve_icc: u32,
    /// AVIF encoder speed 1-10. 1 = slowest/smallest, 10 = fastest (default 6).
    pub avif_speed: u32,
}

impl Default for CompressParams {
    fn default() -> Self {
        Self {
            quality: 80,
            max_width: 0,
            max_height: 0,
            max_file_size: 0,
            min_quality: 30,
            allow_resize: 1,
            format: 0,
            keep_metadata: 0,
            jpeg_progressive: 1,
            jpeg_chroma_subsampling: 0,
            jpeg_trellis: 1,
            png_optimization_level: 2,
            png_lossy: 0,
            auto_orient: 1,
            preserve_icc: 1,
            avif_speed: 6,
        }
    }
}

/// Compression result returned to Dart via FFI.
#[repr(C)]
pub struct CompressResult {
    /// Pointer to compressed data. Caller must free with `free_compress_result`.
    pub data: *mut u8,
    /// Length of compressed data in bytes.
    pub data_len: usize,
    /// Original input size in bytes.
    pub original_size: usize,
    /// Final image width.
    pub width: u32,
    /// Final image height.
    pub height: u32,
    /// Actual quality used (may differ from requested if max_file_size was set).
    pub quality_used: u32,
    /// Number of compression iterations attempted (1 if no max_file_size).
    pub iterations: u32,
    /// Whether the image was auto-resized to meet max_file_size. 1 = yes.
    pub resized_to_fit: u32,
    /// Error code: 0 = success, non-zero = error.
    pub error_code: i32,
    /// Error message (null-terminated C string). Null if no error.
    pub error_message: *mut libc::c_char,
}

// Safety: CompressResult only contains raw pointers that are either null
// or exclusively owned by this result. We need Send for rayon parallelism.
unsafe impl Send for CompressResult {}

impl CompressResult {
    pub fn success(
        data: Vec<u8>,
        original_size: usize,
        width: u32,
        height: u32,
        quality_used: u32,
        iterations: u32,
        resized_to_fit: bool,
    ) -> Self {
        let data_len = data.len();
        let boxed = data.into_boxed_slice();
        let ptr = Box::into_raw(boxed) as *mut u8;
        Self {
            data: ptr,
            data_len,
            original_size,
            width,
            height,
            quality_used,
            iterations,
            resized_to_fit: if resized_to_fit { 1 } else { 0 },
            error_code: 0,
            error_message: std::ptr::null_mut(),
        }
    }

    pub fn error(code: i32, message: &str) -> Self {
        let c_msg = std::ffi::CString::new(message).unwrap_or_default();
        Self {
            data: std::ptr::null_mut(),
            data_len: 0,
            original_size: 0,
            width: 0,
            height: 0,
            quality_used: 0,
            iterations: 0,
            resized_to_fit: 0,
            error_code: code,
            error_message: c_msg.into_raw(),
        }
    }

    pub fn success_without_data(
        compressed_size: usize,
        original_size: usize,
        width: u32,
        height: u32,
        quality_used: u32,
        iterations: u32,
        resized_to_fit: bool,
    ) -> Self {
        Self {
            data: std::ptr::null_mut(),
            data_len: compressed_size,
            original_size,
            width,
            height,
            quality_used,
            iterations,
            resized_to_fit: if resized_to_fit { 1 } else { 0 },
            error_code: 0,
            error_message: std::ptr::null_mut(),
        }
    }
}

// ─── Batch Types ─────────────────────────────────────────────────────────────

/// Single input for batch compression, passed from Dart via FFI.
///
/// Exactly one of `file_path` or `data` must be non-null.
#[repr(C)]
pub struct BatchInput {
    /// Path to input file (null-terminated C string). Null if using `data`.
    pub file_path: *const libc::c_char,
    /// Pointer to input image bytes. Null if using `file_path`.
    pub data: *const u8,
    /// Length of `data` in bytes. 0 if using `file_path`.
    pub data_len: usize,
    /// Path to write output (null-terminated C string).
    /// Null to return compressed bytes in the result.
    pub output_path: *const libc::c_char,
}

// Safety: BatchInput contains only raw pointers for read-only access.
// The underlying data is owned by Dart and guaranteed valid for the
// duration of the FFI call.
unsafe impl Send for BatchInput {}
unsafe impl Sync for BatchInput {}

/// Result of a batch compression operation.
#[repr(C)]
pub struct BatchResult {
    /// Pointer to array of CompressResult. Caller must free with `free_batch_result`.
    pub results: *mut CompressResult,
    /// Number of results in the array.
    pub count: usize,
    /// Total wall-clock time in milliseconds for the entire batch.
    pub elapsed_ms: u64,
    /// Reserved for ABI compatibility with previously shipped binaries.
    /// Always null in current builds.
    pub completed: *mut u32,
}

// ─── Probe Types ─────────────────────────────────────────────────────────────

/// Quick metadata about an image without full decoding.
#[repr(C)]
pub struct ProbeResult {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Detected format: 1=jpeg, 2=png, 3=webp.
    pub format: u32,
    /// File size in bytes.
    pub file_size: usize,
    /// Whether EXIF metadata is present. 1 = yes.
    pub has_exif: u32,
    /// Error code: 0 = success.
    pub error_code: i32,
    /// Error message (null-terminated C string). Null if no error.
    pub error_message: *mut libc::c_char,
}

impl ProbeResult {
    pub fn success(width: u32, height: u32, format: u32, file_size: usize, has_exif: bool) -> Self {
        Self {
            width,
            height,
            format,
            file_size,
            has_exif: if has_exif { 1 } else { 0 },
            error_code: 0,
            error_message: std::ptr::null_mut(),
        }
    }

    pub fn error(code: i32, message: &str) -> Self {
        let c_msg = std::ffi::CString::new(message).unwrap_or_default();
        Self {
            width: 0,
            height: 0,
            format: 0,
            file_size: 0,
            has_exif: 0,
            error_code: code,
            error_message: c_msg.into_raw(),
        }
    }
}

// ─── Benchmark Types ─────────────────────────────────────────────────────────

/// Single quality point in a benchmark sweep.
#[repr(C)]
pub struct BenchmarkEntry {
    /// Quality level used (0-100).
    pub quality: u32,
    /// Compressed output size in bytes.
    pub size_bytes: usize,
    /// Compression ratio (size / original, e.g. 0.35).
    pub ratio: f32,
    /// Encoding time in milliseconds.
    pub encode_ms: u32,
}

/// Result of a benchmark sweep across multiple quality levels.
#[repr(C)]
pub struct BenchmarkResult {
    /// Original file size in bytes.
    pub original_size: usize,
    /// Original width.
    pub width: u32,
    /// Original height.
    pub height: u32,
    /// Detected format: 1=jpeg, 2=png, 3=webp.
    pub format: u32,
    /// Pointer to array of BenchmarkEntry.
    pub entries: *mut BenchmarkEntry,
    /// Number of entries.
    pub entry_count: usize,
    /// Recommended quality for web (best size/quality trade-off).
    pub recommended_quality: u32,
    /// Error code: 0 = success.
    pub error_code: i32,
    /// Error message.
    pub error_message: *mut libc::c_char,
}
