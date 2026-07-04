mod compress;
mod error;
mod options;

#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::CStr;
use std::slice;
use std::sync::{Arc, Mutex, OnceLock};

use options::{CompressParams, CompressResult};

/// Cached rayon thread pool — avoids rebuilding OS threads on every batch call.
/// Under high-intensity workloads this saves ~1-2ms per batch invocation.
/// The pool is sized to `available_cpus - 2` on first use and reused thereafter.
static THREAD_POOLS: OnceLock<Mutex<HashMap<usize, Arc<rayon::ThreadPool>>>> = OnceLock::new();

fn build_pool(requested_threads: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new()
        .num_threads(requested_threads)
        .thread_name(|i| format!("img-compress-{i}"))
        .stack_size(4 * 1024 * 1024)
        .build()
        .unwrap_or_else(|_| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(1)
                .build()
                .expect("Failed to create even a single-threaded rayon pool")
        })
}

fn get_or_build_pool(requested_threads: usize) -> Arc<rayon::ThreadPool> {
    let pools = THREAD_POOLS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut pools = pools.lock().expect("thread pool cache poisoned");

    if let Some(pool) = pools.get(&requested_threads) {
        return Arc::clone(pool);
    }

    let pool = Arc::new(build_pool(requested_threads));
    pools.insert(requested_threads, Arc::clone(&pool));
    pool
}

/// ABI version — increment when any #[repr(C)] struct layout changes.
/// Dart checks this on first load and throws on mismatch.
const ABI_VERSION: u32 = 1;

/// Maximum allowed input file/buffer size (256 MB).
const MAX_INPUT_SIZE: usize = 256 * 1024 * 1024;

/// Return the ABI version of this native library.
/// Dart checks this on first load to detect stale binaries.
#[no_mangle]
pub extern "C" fn ironpress_abi_version() -> u32 {
    ABI_VERSION
}

// ─── FFI: Compress from file path ────────────────────────────────────────────

/// Compress an image file at the given path.
///
/// # Safety
/// - `input_path` must be a valid null-terminated UTF-8 C string.
/// - `params` must point to a valid CompressParams.
/// - `out` must point to a valid, writable CompressResult.
/// - Caller must free the result with `free_compress_result`.
#[no_mangle]
pub unsafe extern "C" fn compress_file(
    input_path: *const libc::c_char,
    params: *const CompressParams,
    out: *mut CompressResult,
) {
    if out.is_null() {
        return;
    }

    if input_path.is_null() || params.is_null() {
        *out = CompressResult::error(-1, "Null pointer argument");
        return;
    }

    let path = match CStr::from_ptr(input_path).to_str() {
        Ok(s) => s,
        Err(_) => {
            *out = CompressResult::error(-2, "Invalid UTF-8 in path");
            return;
        }
    };

    let params = &*params;

    let input_data = match std::fs::read(path) {
        Ok(data) => data,
        Err(e) => {
            *out = CompressResult::error(-3, &format!("Failed to read file: {e}"));
            return;
        }
    };

    if input_data.len() > MAX_INPUT_SIZE {
        *out = CompressResult::error(
            -5,
            &format!(
                "Input file too large ({} bytes, max {})",
                input_data.len(),
                MAX_INPUT_SIZE
            ),
        );
        return;
    }

    let original_size = input_data.len();

    *out = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        compress::compress_bytes(&input_data, params)
    })) {
        Ok(Ok(result)) => CompressResult::success(
            result.data,
            original_size,
            result.width,
            result.height,
            result.quality_used,
            result.iterations,
            result.resized_to_fit,
        ),
        Ok(Err(e)) => CompressResult::error(-10, &e.to_string()),
        Err(_) => CompressResult::error(-99, "Internal panic during compression"),
    };
}

// ─── FFI: Compress from memory buffer ────────────────────────────────────────

/// Compress image data from a memory buffer.
///
/// # Safety
/// - `input_data` must point to a valid buffer of `input_len` bytes.
/// - `out` must point to a valid, writable CompressResult.
/// - Caller must free the result with `free_compress_result`.
#[no_mangle]
pub unsafe extern "C" fn compress_buffer(
    input_data: *const u8,
    input_len: usize,
    params: *const CompressParams,
    out: *mut CompressResult,
) {
    if out.is_null() {
        return;
    }

    if input_data.is_null() || params.is_null() {
        *out = CompressResult::error(-1, "Null pointer argument");
        return;
    }

    if input_len == 0 {
        *out = CompressResult::error(-2, "Empty input buffer");
        return;
    }

    if input_len > MAX_INPUT_SIZE {
        *out = CompressResult::error(
            -5,
            &format!("Input buffer too large ({input_len} bytes, max {MAX_INPUT_SIZE})"),
        );
        return;
    }

    let data = slice::from_raw_parts(input_data, input_len);
    let params = &*params;

    *out = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        compress::compress_bytes(data, params)
    })) {
        Ok(Ok(result)) => CompressResult::success(
            result.data,
            input_len,
            result.width,
            result.height,
            result.quality_used,
            result.iterations,
            result.resized_to_fit,
        ),
        Ok(Err(e)) => CompressResult::error(-10, &e.to_string()),
        Err(_) => CompressResult::error(-99, "Internal panic during compression"),
    };
}

// ─── FFI: Compress file and write to output path ─────────────────────────────

/// Compress an image file and write the result to the output path.
///
/// # Safety
/// - Both paths must be valid null-terminated UTF-8 C strings.
/// - `out` must point to a valid, writable CompressResult.
/// - Caller must free the result with `free_compress_result`.
#[no_mangle]
pub unsafe extern "C" fn compress_file_to_file(
    input_path: *const libc::c_char,
    output_path: *const libc::c_char,
    params: *const CompressParams,
    out: *mut CompressResult,
) {
    if out.is_null() {
        return;
    }

    if input_path.is_null() || output_path.is_null() || params.is_null() {
        *out = CompressResult::error(-1, "Null pointer argument");
        return;
    }

    let in_path = match CStr::from_ptr(input_path).to_str() {
        Ok(s) => s,
        Err(_) => {
            *out = CompressResult::error(-2, "Invalid UTF-8 in input path");
            return;
        }
    };

    let out_path = match CStr::from_ptr(output_path).to_str() {
        Ok(s) => s,
        Err(_) => {
            *out = CompressResult::error(-2, "Invalid UTF-8 in output path");
            return;
        }
    };

    let params = &*params;

    let input_data = match std::fs::read(in_path) {
        Ok(data) => data,
        Err(e) => {
            *out = CompressResult::error(-3, &format!("Failed to read file: {e}"));
            return;
        }
    };

    if input_data.len() > MAX_INPUT_SIZE {
        *out = CompressResult::error(
            -5,
            &format!(
                "Input file too large ({} bytes, max {})",
                input_data.len(),
                MAX_INPUT_SIZE
            ),
        );
        return;
    }

    let original_size = input_data.len();

    *out = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        compress::compress_bytes(&input_data, params)
    })) {
        Ok(Ok(result)) => {
            let compressed_size = result.data.len();
            if let Err(e) = std::fs::write(out_path, &result.data) {
                CompressResult::error(-4, &format!("Failed to write output: {e}"))
            } else {
                CompressResult::success_without_data(
                    compressed_size,
                    original_size,
                    result.width,
                    result.height,
                    result.quality_used,
                    result.iterations,
                    result.resized_to_fit,
                )
            }
        }
        Ok(Err(e)) => CompressResult::error(-10, &e.to_string()),
        Err(_) => CompressResult::error(-99, "Internal panic during compression"),
    };
}

// ─── FFI: Memory management ──────────────────────────────────────────────────

/// Free a CompressResult's inner allocations (data buffer and error message).
///
/// # Safety
/// Must only be called once per result. The CompressResult struct itself
/// is NOT freed — caller is responsible for freeing the outer allocation.
#[no_mangle]
pub unsafe extern "C" fn free_compress_result(result: *mut CompressResult) {
    if result.is_null() {
        return;
    }

    let r = &mut *result;

    // Free the data buffer
    if !r.data.is_null() && r.data_len > 0 {
        let _ = Box::from_raw(std::ptr::slice_from_raw_parts_mut(r.data, r.data_len));
    }
    r.data = std::ptr::null_mut();
    r.data_len = 0;

    // Free the error message
    if !r.error_message.is_null() {
        let _ = std::ffi::CString::from_raw(r.error_message);
    }
    r.error_message = std::ptr::null_mut();
}

// ─── FFI: Batch compression with rayon ───────────────────────────────────────

/// Compress multiple images in parallel using rayon's work-stealing thread pool.
///
/// # Safety
/// - `inputs` must point to a valid array of `count` BatchInput structs.
/// - Each BatchInput's file_path/data pointers must be valid for the call duration.
/// - `params` must point to a valid CompressParams.
/// - `out` must point to a valid, writable BatchResult.
/// - Caller must free the result with `free_batch_result`.
#[no_mangle]
pub unsafe extern "C" fn compress_batch(
    inputs: *const options::BatchInput,
    count: usize,
    params: *const CompressParams,
    thread_count: u32,
    _chunk_size: u32,
    out: *mut options::BatchResult,
) {
    if out.is_null() {
        return;
    }

    if inputs.is_null() || params.is_null() || count == 0 {
        *out = options::BatchResult {
            results: std::ptr::null_mut(),
            count: 0,
            elapsed_ms: 0,
            completed: std::ptr::null_mut(),
        };
        return;
    }

    // Wrap entire batch operation in catch_unwind to prevent panics
    // from crossing the FFI boundary (undefined behavior).
    let batch_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        use rayon::prelude::*;
        use std::time::Instant;

        let params = &*params;
        let inputs_slice = slice::from_raw_parts(inputs, count);

        // ── Thread count: safe default leaves room for Flutter UI ──
        let available = num_cpus_safe();
        let num_threads = if thread_count > 0 {
            (thread_count as usize).min(available).min(count.max(1))
        } else {
            available.saturating_sub(2).max(1).min(count.max(1))
        };

        // ── Chunk size: bounds peak memory ──
        // Dart already chunks batch work when it needs chunk-boundary
        // cancellation or progress semantics. Native batch parallelizes the
        // provided slice directly to avoid a second scheduling layer.

        // ── Atomic progress counter (Dart can poll this) ──
        // Measure native wall-clock time for the provided slice.
        let start = Instant::now();

        // Reuse cached thread pool — avoids OS thread creation overhead on every call.
        let pool = get_or_build_pool(num_threads);

        // ── Process in chunks to bound memory ──
        let all_results: Vec<CompressResult> = pool.install(|| {
            inputs_slice
                .par_iter()
                .map(|input| {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        process_batch_input(input, params)
                    }));

                    match result {
                        Ok(r) => r,
                        Err(_) => CompressResult::error(
                            -99,
                            "Internal panic during compression (possible OOM or corrupt image)",
                        ),
                    }
                })
                .collect()
        });

        let elapsed_ms = start.elapsed().as_millis() as u64;

        // Move results into heap-allocated array for FFI
        let mut boxed_results = all_results.into_boxed_slice();
        let ptr = boxed_results.as_mut_ptr();
        let len = boxed_results.len();
        std::mem::forget(boxed_results);

        options::BatchResult {
            results: ptr,
            count: len,
            elapsed_ms,
            completed: std::ptr::null_mut(),
        }
    }));

    *out = match batch_result {
        Ok(result) => result,
        Err(_) => options::BatchResult {
            results: std::ptr::null_mut(),
            count: 0,
            elapsed_ms: 0,
            completed: std::ptr::null_mut(),
        },
    };
}

/// Process a single batch input item. Called from rayon worker threads.
unsafe fn process_batch_input(
    input: &options::BatchInput,
    params: &CompressParams,
) -> CompressResult {
    // Read input data — file path or memory buffer
    let input_data: Cow<'_, [u8]> = if !input.file_path.is_null() {
        let path = match CStr::from_ptr(input.file_path).to_str() {
            Ok(s) => s,
            Err(_) => return CompressResult::error(-2, "Invalid UTF-8 in file path"),
        };
        match std::fs::read(path) {
            Ok(data) => Cow::Owned(data),
            Err(e) => {
                return CompressResult::error(-3, &format!("Failed to read: {e}"));
            }
        }
    } else if !input.data.is_null() && input.data_len > 0 {
        Cow::Borrowed(slice::from_raw_parts(input.data, input.data_len))
    } else {
        return CompressResult::error(-1, "BatchInput has no file_path or data");
    };
    let original_size = input_data.len();

    if original_size > MAX_INPUT_SIZE {
        return CompressResult::error(
            -5,
            &format!("Input too large ({original_size} bytes, max {MAX_INPUT_SIZE})"),
        );
    }

    // Compress
    let compress_result = match compress::compress_bytes(input_data.as_ref(), params) {
        Ok(r) => r,
        Err(e) => return CompressResult::error(-10, &e.to_string()),
    };

    // Write to output file if path provided
    if !input.output_path.is_null() {
        let out_path = match CStr::from_ptr(input.output_path).to_str() {
            Ok(s) => s,
            Err(_) => return CompressResult::error(-2, "Invalid UTF-8 in output path"),
        };
        let compressed_size = compress_result.data.len();
        if let Err(e) = std::fs::write(out_path, &compress_result.data) {
            return CompressResult::error(-4, &format!("Failed to write: {e}"));
        }
        CompressResult::success_without_data(
            compressed_size,
            original_size,
            compress_result.width,
            compress_result.height,
            compress_result.quality_used,
            compress_result.iterations,
            compress_result.resized_to_fit,
        )
    } else {
        CompressResult::success(
            compress_result.data,
            original_size,
            compress_result.width,
            compress_result.height,
            compress_result.quality_used,
            compress_result.iterations,
            compress_result.resized_to_fit,
        )
    }
}

/// Portable CPU count that never panics.
fn num_cpus_safe() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

/// Free a BatchResult and all its contained CompressResults.
///
/// # Safety
/// Must only be called once per batch result.
#[no_mangle]
pub unsafe extern "C" fn free_batch_result(result: *mut options::BatchResult) {
    if result.is_null() {
        return;
    }

    let batch = &mut *result;

    if !batch.results.is_null() && batch.count > 0 {
        for i in 0..batch.count {
            let r = &mut *batch.results.add(i);

            if !r.data.is_null() && r.data_len > 0 {
                let _ = Box::from_raw(std::ptr::slice_from_raw_parts_mut(r.data, r.data_len));
                r.data = std::ptr::null_mut();
            }

            if !r.error_message.is_null() {
                let _ = std::ffi::CString::from_raw(r.error_message);
                r.error_message = std::ptr::null_mut();
            }
        }

        let _ = Box::from_raw(std::ptr::slice_from_raw_parts_mut(
            batch.results,
            batch.count,
        ));
        batch.results = std::ptr::null_mut();
    }

    batch.completed = std::ptr::null_mut();
    batch.count = 0;
}

// ─── FFI: Probe (quick metadata) ─────────────────────────────────────────────

/// Read image metadata without decoding — dimensions, format, EXIF presence.
///
/// # Safety
/// - `input_path` must be a valid null-terminated UTF-8 C string.
/// - `out` must point to a valid, writable ProbeResult.
#[no_mangle]
pub unsafe extern "C" fn probe_file(
    input_path: *const libc::c_char,
    out: *mut options::ProbeResult,
) {
    if out.is_null() {
        return;
    }

    if input_path.is_null() {
        *out = options::ProbeResult::error(-1, "Null pointer argument");
        return;
    }

    let path = match CStr::from_ptr(input_path).to_str() {
        Ok(s) => s,
        Err(_) => {
            *out = options::ProbeResult::error(-2, "Invalid UTF-8 in path");
            return;
        }
    };

    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            *out = options::ProbeResult::error(-3, &format!("Failed to read: {e}"));
            return;
        }
    };

    *out = probe_bytes_impl(&data);
}

/// Read image metadata from a memory buffer without decoding.
///
/// # Safety
/// - `input_data` must point to a valid buffer of `input_len` bytes.
/// - `out` must point to a valid, writable ProbeResult.
#[no_mangle]
pub unsafe extern "C" fn probe_buffer(
    input_data: *const u8,
    input_len: usize,
    out: *mut options::ProbeResult,
) {
    if out.is_null() {
        return;
    }

    if input_data.is_null() || input_len == 0 {
        *out = options::ProbeResult::error(-1, "Null or empty input");
        return;
    }

    let data = slice::from_raw_parts(input_data, input_len);
    *out = probe_bytes_impl(data);
}

fn probe_bytes_impl(data: &[u8]) -> options::ProbeResult {
    match compress::probe_bytes(data) {
        Ok(info) => {
            let fmt = match info.format {
                compress::DetectedFormat::Jpeg => 1u32,
                compress::DetectedFormat::Png => 2u32,
                compress::DetectedFormat::WebpLossless => 3u32,
                compress::DetectedFormat::WebpLossy => 4u32,
            };
            options::ProbeResult::success(
                info.width,
                info.height,
                fmt,
                info.file_size,
                info.has_exif,
            )
        }
        Err(e) => options::ProbeResult::error(-10, &e.to_string()),
    }
}

/// Free a ProbeResult's error message if present.
///
/// # Safety
/// `result` must be a valid pointer to a ProbeResult, or null.
#[no_mangle]
pub unsafe extern "C" fn free_probe_result(result: *mut options::ProbeResult) {
    if result.is_null() {
        return;
    }
    let r = &mut *result;
    if !r.error_message.is_null() {
        let _ = std::ffi::CString::from_raw(r.error_message);
        r.error_message = std::ptr::null_mut();
    }
}

// ─── FFI: Benchmark (quality sweep) ─────────────────────────────────────────

/// Run a quality sweep on a file.
///
/// # Safety
/// - `input_path` must be a valid null-terminated UTF-8 C string.
/// - `params` must be a valid pointer.
/// - `out` must point to a valid, writable BenchmarkResult.
/// - Caller must free with `free_benchmark_result`.
#[no_mangle]
pub unsafe extern "C" fn benchmark_file(
    input_path: *const libc::c_char,
    params: *const options::CompressParams,
    out: *mut options::BenchmarkResult,
) {
    if out.is_null() {
        return;
    }

    if input_path.is_null() || params.is_null() {
        *out = benchmark_error(-1, "Null pointer argument");
        return;
    }

    let path = match CStr::from_ptr(input_path).to_str() {
        Ok(s) => s,
        Err(_) => {
            *out = benchmark_error(-2, "Invalid UTF-8 in path");
            return;
        }
    };

    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            *out = benchmark_error(-3, &format!("Failed to read: {e}"));
            return;
        }
    };

    let params = &*params;
    *out = benchmark_bytes_impl(&data, params);
}

/// Run a quality sweep on a memory buffer.
///
/// # Safety
/// - `input_data` must point to a valid buffer of `input_len` bytes.
/// - `out` must point to a valid, writable BenchmarkResult.
#[no_mangle]
pub unsafe extern "C" fn benchmark_buffer(
    input_data: *const u8,
    input_len: usize,
    params: *const options::CompressParams,
    out: *mut options::BenchmarkResult,
) {
    if out.is_null() {
        return;
    }

    if input_data.is_null() || params.is_null() || input_len == 0 {
        *out = benchmark_error(-1, "Null or empty input");
        return;
    }

    let data = slice::from_raw_parts(input_data, input_len);
    let params = &*params;
    *out = benchmark_bytes_impl(data, params);
}

fn benchmark_bytes_impl(data: &[u8], params: &options::CompressParams) -> options::BenchmarkResult {
    match compress::benchmark_bytes(data, params) {
        Ok(info) => {
            let fmt = match info.format {
                compress::DetectedFormat::Jpeg => 1u32,
                compress::DetectedFormat::Png => 2u32,
                compress::DetectedFormat::WebpLossless => 3u32,
                compress::DetectedFormat::WebpLossy => 4u32,
            };

            let mut ffi_entries: Vec<options::BenchmarkEntry> = info
                .entries
                .into_iter()
                .map(|e| options::BenchmarkEntry {
                    quality: e.quality,
                    size_bytes: e.size_bytes,
                    ratio: e.ratio,
                    encode_ms: e.encode_ms,
                })
                .collect();

            let entries_ptr = ffi_entries.as_mut_ptr();
            let entry_count = ffi_entries.len();
            std::mem::forget(ffi_entries);

            options::BenchmarkResult {
                original_size: info.original_size,
                width: info.width,
                height: info.height,
                format: fmt,
                entries: entries_ptr,
                entry_count,
                recommended_quality: info.recommended_quality,
                error_code: 0,
                error_message: std::ptr::null_mut(),
            }
        }
        Err(e) => benchmark_error(-10, &e.to_string()),
    }
}

fn benchmark_error(code: i32, message: &str) -> options::BenchmarkResult {
    let c_msg = std::ffi::CString::new(message).unwrap_or_default();
    options::BenchmarkResult {
        original_size: 0,
        width: 0,
        height: 0,
        format: 0,
        entries: std::ptr::null_mut(),
        entry_count: 0,
        recommended_quality: 0,
        error_code: code,
        error_message: c_msg.into_raw(),
    }
}

/// Free a BenchmarkResult.
///
/// # Safety
/// `result` must be a valid pointer to a BenchmarkResult, or null.
#[no_mangle]
pub unsafe extern "C" fn free_benchmark_result(result: *mut options::BenchmarkResult) {
    if result.is_null() {
        return;
    }
    let r = &mut *result;
    if !r.entries.is_null() && r.entry_count > 0 {
        let _ = Box::from_raw(std::ptr::slice_from_raw_parts_mut(r.entries, r.entry_count));
        r.entries = std::ptr::null_mut();
    }
    if !r.error_message.is_null() {
        let _ = std::ffi::CString::from_raw(r.error_message);
        r.error_message = std::ptr::null_mut();
    }
}

// ─── FFI: Version info ───────────────────────────────────────────────────────

/// Return the library version as a null-terminated C string.
/// The returned pointer is static and must NOT be freed.
#[no_mangle]
pub extern "C" fn ironpress_version() -> *const libc::c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const libc::c_char
}
