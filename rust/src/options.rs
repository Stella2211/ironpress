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
}

impl OutputFormat {
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Jpeg,
            2 => Self::Png,
            3 => Self::WebpLossless,
            4 => Self::WebpLossy,
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
    /// 1 = use `png_optimization_level` as-is on every image size; 0
    /// (default) = levels above 2 are lowered automatically for large
    /// images (see `compress::effective_png_level`).
    pub png_level_unbounded: u32,
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
            png_level_unbounded: 0,
        }
    }
}

/// Compression result returned to Dart via FFI.
#[repr(C)]
pub struct CompressResult {
    /// Pointer to compressed data, owned by `buffer`. Null when there is no
    /// in-memory output.
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
    /// Number of encode attempts (every full encode, including the trial
    /// encodes of the target-size search). 1 when `max_file_size` is 0.
    pub iterations: u32,
    /// Whether the image was auto-resized to meet max_file_size. 1 = yes.
    pub resized_to_fit: u32,
    /// Error code: 0 = success, non-zero = error.
    pub error_code: i32,
    /// Error message (null-terminated C string). Null if no error.
    pub error_message: *mut libc::c_char,
    /// Opaque owner of `data` (null when `data` is null). The caller either
    /// leaves it in place and calls `free_compress_result`, or takes it over
    /// (setting this field to null) and later releases it with
    /// `ironpress_free_buffer` — e.g. as a Dart `NativeFinalizer` token.
    pub buffer: *mut libc::c_void,
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
        let (data_ptr, data_len, buffer) = OutputBuffer::into_raw(data);
        Self {
            data: data_ptr,
            data_len,
            original_size,
            width,
            height,
            quality_used,
            iterations,
            resized_to_fit: if resized_to_fit { 1 } else { 0 },
            error_code: 0,
            error_message: std::ptr::null_mut(),
            buffer,
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
            buffer: std::ptr::null_mut(),
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
            buffer: std::ptr::null_mut(),
        }
    }

    /// Release the owned allocations (output buffer and error message) and
    /// null the pointers. Safe to call more than once.
    ///
    /// # Safety
    /// `buffer` and `error_message` must be null or owned by this result.
    pub unsafe fn release(&mut self) {
        unsafe {
            OutputBuffer::free(self.buffer);
            if !self.error_message.is_null() {
                drop(std::ffi::CString::from_raw(self.error_message));
            }
        }
        self.buffer = std::ptr::null_mut();
        self.data = std::ptr::null_mut();
        self.data_len = 0;
        self.error_message = std::ptr::null_mut();
    }
}

/// Owner of an output buffer handed to the caller without copying or
/// reallocating it: the encoder's `Vec` is boxed as-is (its spare capacity
/// included), so no `into_boxed_slice` shrink/realloc happens.
pub struct OutputBuffer;

impl OutputBuffer {
    /// Returns `(data, len, handle)`; all null/0 for an empty vector.
    pub fn into_raw(data: Vec<u8>) -> (*mut u8, usize, *mut libc::c_void) {
        if data.is_empty() {
            return (std::ptr::null_mut(), 0, std::ptr::null_mut());
        }
        let mut boxed = Box::new(data);
        let ptr = boxed.as_mut_ptr();
        let len = boxed.len();
        (ptr, len, Box::into_raw(boxed) as *mut libc::c_void)
    }

    /// Free a handle returned by [`OutputBuffer::into_raw`]. Null is a no-op.
    ///
    /// # Safety
    /// `handle` must be null or a live handle from `into_raw`, freed once.
    pub unsafe fn free(handle: *mut libc::c_void) {
        if !handle.is_null() {
            drop(unsafe { Box::from_raw(handle as *mut Vec<u8>) });
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
    /// Number of results in the array: every input, or — when the batch was
    /// cancelled through its [`BatchControl`] — the inputs started before
    /// the cancellation (always a prefix of the inputs).
    pub count: usize,
    /// Total wall-clock time in milliseconds for the entire batch.
    pub elapsed_ms: u64,
}

/// Shared progress/cancellation block of a batch call, owned by the caller
/// and readable/writable from other threads while `compress_batch` runs.
///
/// Both fields are 32-bit words so that plain aligned loads/stores from the
/// caller (e.g. Dart `Pointer<Uint32>` on 32-bit ARM) are single-copy atomic.
#[repr(C)]
pub struct BatchControl {
    /// Number of finished items (successful or failed). Incremented by the
    /// engine; it is never reset, so one block can span several calls.
    pub completed: std::sync::atomic::AtomicU32,
    /// Set to non-zero by the caller to cancel: items that have not started
    /// yet are skipped, running items finish.
    pub cancelled: std::sync::atomic::AtomicU32,
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
    /// EXIF orientation 1-8 (1 when absent). `width`/`height` are the stored
    /// dimensions; the display orientation swaps them for 5-8.
    pub orientation: u32,
    /// Error code: 0 = success.
    pub error_code: i32,
    /// Error message (null-terminated C string). Null if no error.
    pub error_message: *mut libc::c_char,
}

impl ProbeResult {
    pub fn success(
        width: u32,
        height: u32,
        format: u32,
        file_size: usize,
        has_exif: bool,
        orientation: u8,
    ) -> Self {
        Self {
            width,
            height,
            format,
            file_size,
            has_exif: if has_exif { 1 } else { 0 },
            orientation: u32::from(orientation),
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
            orientation: 0,
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
