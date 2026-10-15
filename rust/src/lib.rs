mod compress;
mod error;
mod options;

#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::ffi::CStr;
use std::io::Read;
use std::slice;
use std::sync::{Arc, Condvar, Mutex};

use options::{CompressParams, CompressResult};

/// Cached rayon thread pools keyed by thread count — avoids rebuilding OS
/// threads on every call (~1-2ms per invocation under heavy workloads).
///
/// The cache is a small LRU: batch thread counts vary with the item count,
/// so an unbounded map would keep accumulating idle pools (and their OS
/// threads). Evicted pools shut down once no in-flight call holds them.
static THREAD_POOLS: Mutex<Vec<(usize, Arc<rayon::ThreadPool>)>> = Mutex::new(Vec::new());

/// Maximum number of cached thread pools.
const MAX_CACHED_POOLS: usize = 4;

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
    let mut pools = THREAD_POOLS.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(index) = pools.iter().position(|(n, _)| *n == requested_threads) {
        // Move to the most-recently-used position.
        let entry = pools.remove(index);
        let pool = Arc::clone(&entry.1);
        pools.push(entry);
        return pool;
    }

    if pools.len() >= MAX_CACHED_POOLS {
        pools.remove(0);
    }

    let pool = Arc::new(build_pool(requested_threads));
    pools.push((requested_threads, Arc::clone(&pool)));
    pool
}

/// Default worker count: leave two cores for the Flutter UI/raster threads.
fn default_thread_count() -> usize {
    num_cpus_safe().saturating_sub(2).max(1)
}

/// Run single-image work on the dedicated compression pool (same sizing
/// policy as batch) instead of rayon's global pool, so oxipng's internal
/// parallelism cannot occupy every core.
fn run_in_default_pool<R: Send>(op: impl FnOnce() -> R + Send) -> R {
    get_or_build_pool(default_thread_count()).install(op)
}

// ─── Batch memory budget ─────────────────────────────────────────────────────

/// Default per-call batch memory budget (`memory_budget == 0`).
const DEFAULT_BATCH_MEMORY_BUDGET: u64 = 512 * 1024 * 1024;

/// Process-wide accounting of the estimated peak memory of every batch item
/// currently running, across all concurrent `compress_batch` calls.
///
/// Each call admits its items against *its own* budget, checked against the
/// process-wide total: an item is admitted when `in-flight total + item <=
/// budget`, or when nothing is in flight at all (so an item larger than the
/// budget still runs, alone). Consequences:
/// - With equal budgets this is one shared limit for the whole process.
/// - A call with a smaller budget is more conservative: it waits while other
///   calls keep the total high, and never adds an item that would push the
///   total over its own limit.
/// - The total in flight never exceeds the largest budget among the active
///   calls (plus at most one oversized item).
///
/// Permits are only ever acquired on the batch dispatcher thread (the FFI
/// caller, never a rayon worker), so a worker never blocks on the budget and
/// permit holders always progress.
struct MemoryBudget {
    used: Mutex<u64>,
    released: Condvar,
}

struct MemoryPermit<'a> {
    budget: &'a MemoryBudget,
    weight: u64,
}

impl MemoryBudget {
    const fn new() -> Self {
        Self {
            used: Mutex::new(0),
            released: Condvar::new(),
        }
    }

    /// Block until `weight` bytes fit under `capacity` (see the type docs).
    /// Oversized requests are clamped to `capacity` so they run once nothing
    /// else is running.
    fn acquire(&self, weight: u64, capacity: u64) -> MemoryPermit<'_> {
        let capacity = capacity.max(1);
        let weight = weight.clamp(1, capacity);
        let mut used = self.used.lock().unwrap_or_else(|e| e.into_inner());
        while *used > 0 && used.saturating_add(weight) > capacity {
            used = self.released.wait(used).unwrap_or_else(|e| e.into_inner());
        }
        *used += weight;
        MemoryPermit {
            budget: self,
            weight,
        }
    }
}

impl Drop for MemoryPermit<'_> {
    fn drop(&mut self) {
        let mut used = self.budget.used.lock().unwrap_or_else(|e| e.into_inner());
        *used -= self.weight;
        drop(used);
        self.budget.released.notify_all();
    }
}

/// Process-wide in-flight accounting shared by all concurrent batch calls.
static BATCH_MEMORY: MemoryBudget = MemoryBudget::new();

/// Budget of one call: `0` selects [`DEFAULT_BATCH_MEMORY_BUDGET`].
fn resolve_memory_budget(requested: u64) -> u64 {
    if requested == 0 {
        DEFAULT_BATCH_MEMORY_BUDGET
    } else {
        requested
    }
}

/// Estimate the peak memory of one batch item from its header only.
/// Inputs that will fail before decoding (unreadable, oversized, corrupt
/// header) are weighted by their byte length only.
unsafe fn estimate_batch_input_bytes(input: &options::BatchInput, params: &CompressParams) -> u64 {
    unsafe {
        let (input_len, info) = if !input.file_path.is_null() {
            let Ok(path) = CStr::from_ptr(input.file_path).to_str() else {
                return 0;
            };
            let Ok(file) = std::fs::File::open(path) else {
                return 0;
            };
            let len = file.metadata().map(|m| m.len()).unwrap_or(0);
            if len > MAX_INPUT_SIZE as u64 {
                return 0;
            }
            (len, compress::file_header_info(file))
        } else if !input.data.is_null() && input.data_len > 0 {
            let data = slice::from_raw_parts(input.data, input.data_len);
            (data.len() as u64, compress::header_info(data))
        } else {
            return 0;
        };

        match info {
            Some(info) => compress::estimate_peak_memory(info, input_len, params),
            None => input_len,
        }
    }
}

// ─── Input file reading ──────────────────────────────────────────────────────

enum ReadInputError {
    Io(std::io::Error),
    TooLarge(u64),
}

/// Read an input file, rejecting it via its metadata length before any
/// bytes are loaded when it exceeds `MAX_INPUT_SIZE`.
fn read_input_file(path: &str) -> Result<Vec<u8>, ReadInputError> {
    let file = std::fs::File::open(path).map_err(ReadInputError::Io)?;
    let len = file.metadata().map_err(ReadInputError::Io)?.len();
    if len > MAX_INPUT_SIZE as u64 {
        return Err(ReadInputError::TooLarge(len));
    }

    let mut data = Vec::with_capacity(len as usize);
    // Bound the read in case the file grows after the metadata check.
    file.take(MAX_INPUT_SIZE as u64 + 1)
        .read_to_end(&mut data)
        .map_err(ReadInputError::Io)?;
    if data.len() > MAX_INPUT_SIZE {
        return Err(ReadInputError::TooLarge(data.len() as u64));
    }
    Ok(data)
}

/// ABI version — increment when any #[repr(C)] struct layout changes.
/// Dart checks this on first load and throws on mismatch.
const ABI_VERSION: u32 = 2;

/// Maximum allowed input file/buffer size (256 MB).
const MAX_INPUT_SIZE: usize = 256 * 1024 * 1024;

/// Return the ABI version of this native library.
/// Dart checks this on first load to detect stale binaries.
#[unsafe(no_mangle)]
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compress_file(
    input_path: *const libc::c_char,
    params: *const CompressParams,
    out: *mut CompressResult,
) {
    unsafe {
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

        let input_data = match read_input_file(path) {
            Ok(data) => data,
            Err(ReadInputError::Io(e)) => {
                *out = CompressResult::error(-3, &format!("Failed to read file: {e}"));
                return;
            }
            Err(ReadInputError::TooLarge(len)) => {
                *out = CompressResult::error(
                    -5,
                    &format!("Input file too large ({len} bytes, max {MAX_INPUT_SIZE})"),
                );
                return;
            }
        };

        let original_size = input_data.len();

        *out = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_in_default_pool(|| compress::compress_bytes(&input_data, params))
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
            Ok(Err(e)) => CompressResult::error(e.ffi_code(), &e.to_string()),
            Err(_) => CompressResult::error(-99, "Internal panic during compression"),
        };
    }
}

// ─── FFI: Compress from memory buffer ────────────────────────────────────────

/// Compress image data from a memory buffer.
///
/// # Safety
/// - `input_data` must point to a valid buffer of `input_len` bytes.
/// - `out` must point to a valid, writable CompressResult.
/// - Caller must free the result with `free_compress_result`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compress_buffer(
    input_data: *const u8,
    input_len: usize,
    params: *const CompressParams,
    out: *mut CompressResult,
) {
    unsafe {
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
            run_in_default_pool(|| compress::compress_bytes(data, params))
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
            Ok(Err(e)) => CompressResult::error(e.ffi_code(), &e.to_string()),
            Err(_) => CompressResult::error(-99, "Internal panic during compression"),
        };
    }
}

// ─── FFI: Compress file and write to output path ─────────────────────────────

/// Compress an image file and write the result to the output path.
///
/// # Safety
/// - Both paths must be valid null-terminated UTF-8 C strings.
/// - `out` must point to a valid, writable CompressResult.
/// - Caller must free the result with `free_compress_result`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compress_file_to_file(
    input_path: *const libc::c_char,
    output_path: *const libc::c_char,
    params: *const CompressParams,
    out: *mut CompressResult,
) {
    unsafe {
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

        let input_data = match read_input_file(in_path) {
            Ok(data) => data,
            Err(ReadInputError::Io(e)) => {
                *out = CompressResult::error(-3, &format!("Failed to read file: {e}"));
                return;
            }
            Err(ReadInputError::TooLarge(len)) => {
                *out = CompressResult::error(
                    -5,
                    &format!("Input file too large ({len} bytes, max {MAX_INPUT_SIZE})"),
                );
                return;
            }
        };

        let original_size = input_data.len();

        *out = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_in_default_pool(|| compress::compress_bytes(&input_data, params))
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
            Ok(Err(e)) => CompressResult::error(e.ffi_code(), &e.to_string()),
            Err(_) => CompressResult::error(-99, "Internal panic during compression"),
        };
    }
}

// ─── FFI: Memory management ──────────────────────────────────────────────────

/// Free a CompressResult's inner allocations (data buffer and error message).
///
/// # Safety
/// Must only be called once per result. The CompressResult struct itself
/// is NOT freed — caller is responsible for freeing the outer allocation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_compress_result(result: *mut CompressResult) {
    unsafe {
        if let Some(r) = result.as_mut() {
            r.release();
        }
    }
}

/// Free an output buffer taken over from a `CompressResult` (its `buffer`
/// field). Signature-compatible with a Dart `NativeFinalizer` callback
/// (`void (*)(void*)`), so it can be attached directly to an external
/// `Uint8List` with `Pointer.asTypedList(finalizer:, token:)`.
///
/// # Safety
/// `buffer` must be null or a handle taken from a result and not yet freed;
/// the handle must not be freed twice (by this function or by
/// `free_compress_result`/`free_batch_result`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ironpress_free_buffer(buffer: *mut libc::c_void) {
    unsafe { options::OutputBuffer::free(buffer) }
}

// ─── FFI: Batch compression with rayon ───────────────────────────────────────

/// Compress multiple images in parallel on a dedicated rayon thread pool,
/// in a single call (no chunking needed for progress or cancellation).
///
/// Items are admitted in order against the memory budget (estimated from
/// each image's header; see [`MemoryBudget`] for how concurrent calls
/// share it), so large images run with less concurrency instead of all
/// being decoded at once.
///
/// `memory_budget` is the call's budget in bytes (0 = 512 MB default).
///
/// `control` (nullable) is a caller-owned [`options::BatchControl`]: the
/// engine increments `completed` as items finish and stops starting new
/// items once `cancelled` is non-zero. The result then holds only the items
/// started before the cancellation, which is always a prefix of `inputs`.
///
/// # Safety
/// - `inputs` must point to a valid array of `count` BatchInput structs.
/// - Each BatchInput's file_path/data pointers must be valid for the call duration.
/// - `params` must point to a valid CompressParams.
/// - `control` must be null or valid (and 4-byte aligned) for the call duration.
/// - `out` must point to a valid, writable BatchResult.
/// - Caller must free the result with `free_batch_result`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compress_batch(
    inputs: *const options::BatchInput,
    count: usize,
    params: *const CompressParams,
    thread_count: u32,
    memory_budget: u64,
    control: *const options::BatchControl,
    out: *mut options::BatchResult,
) {
    unsafe {
        if out.is_null() {
            return;
        }

        let empty = options::BatchResult {
            results: std::ptr::null_mut(),
            count: 0,
            elapsed_ms: 0,
        };

        if inputs.is_null() || params.is_null() || count == 0 {
            *out = empty;
            return;
        }

        let control = control.as_ref();

        // Wrap entire batch operation in catch_unwind to prevent panics
        // from crossing the FFI boundary (undefined behavior).
        let batch_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            use std::sync::atomic::Ordering;
            use std::time::Instant;

            let params = &*params;
            let inputs_slice = slice::from_raw_parts(inputs, count);
            let budget = resolve_memory_budget(memory_budget);
            let is_cancelled = || control.is_some_and(|c| c.cancelled.load(Ordering::Acquire) != 0);

            // ── Thread count: safe default leaves room for Flutter UI ──
            let available = num_cpus_safe();
            let num_threads = if thread_count > 0 {
                (thread_count as usize).min(available).min(count.max(1))
            } else {
                default_thread_count().min(count.max(1))
            };

            let start = Instant::now();

            // Reuse cached thread pool — avoids OS thread creation overhead on every call.
            let pool = get_or_build_pool(num_threads);

            // ── Memory-bounded dispatch ──
            // The calling thread (never a pool worker) admits items in order
            // against the memory budget and spawns one task per admitted
            // item. Workers never wait on the budget, so a permit holder can
            // always finish (even when oxipng's nested parallelism steals
            // other batch jobs) and release its permit — no deadlock. The
            // scope returns once every spawned task has completed.
            //
            // Cancellation: a task claims the next input index (in order)
            // only while the batch is not cancelled, under one lock. Items
            // queued but not yet claimed are skipped, and the processed items
            // are always the prefix `0..claimed`. The dispatcher also stops
            // admitting items once cancelled.
            let slots: Vec<Mutex<Option<CompressResult>>> =
                inputs_slice.iter().map(|_| Mutex::new(None)).collect();
            let permits: Vec<Mutex<Option<MemoryPermit<'_>>>> =
                inputs_slice.iter().map(|_| Mutex::new(None)).collect();
            let claimed = Mutex::new(0usize);

            pool.in_place_scope(|scope| {
                for (input, permit_slot) in inputs_slice.iter().zip(&permits) {
                    if is_cancelled() {
                        break;
                    }
                    let weight = estimate_batch_input_bytes(input, params);
                    let permit = BATCH_MEMORY.acquire(weight, budget);
                    if is_cancelled() {
                        break;
                    }
                    *permit_slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(permit);
                    let (slots, permits, claimed) = (&slots, &permits, &claimed);
                    scope.spawn(move |_| {
                        // Claims never outnumber spawned tasks, so the
                        // claimed item's permit is already in its slot.
                        let index = {
                            let mut next = claimed.lock().unwrap_or_else(|e| e.into_inner());
                            if is_cancelled() {
                                return;
                            }
                            *next += 1;
                            *next - 1
                        };
                        let permit = permits[index]
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .take();
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            process_batch_input(&inputs_slice[index], params)
                        }))
                        .unwrap_or_else(|_| {
                            CompressResult::error(
                                -99,
                                "Internal panic during compression (possible OOM or corrupt image)",
                            )
                        });
                        drop(permit);
                        *slots[index].lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
                        if let Some(control) = control {
                            control.completed.fetch_add(1, Ordering::Release);
                        }
                    });
                }
            });
            // Release the permits of admitted-but-skipped items.
            drop(permits);
            let started = claimed.into_inner().unwrap_or_else(|e| e.into_inner());

            let all_results: Vec<CompressResult> = slots
                .into_iter()
                .take(started)
                .map(|slot| {
                    slot.into_inner()
                        .unwrap_or_else(|e| e.into_inner())
                        .unwrap_or_else(|| {
                            CompressResult::error(-99, "Batch item produced no result")
                        })
                })
                .collect();

            let elapsed_ms = start.elapsed().as_millis() as u64;

            // Move results into heap-allocated array for FFI
            let len = all_results.len();
            let ptr = if len == 0 {
                std::ptr::null_mut()
            } else {
                Box::into_raw(all_results.into_boxed_slice()) as *mut CompressResult
            };

            options::BatchResult {
                results: ptr,
                count: len,
                elapsed_ms,
            }
        }));

        *out = batch_result.unwrap_or(empty);
    }
}

/// Process a single batch input item. Called from rayon worker threads.
/// Must not block on the batch memory budget (see `compress_batch`).
unsafe fn process_batch_input(
    input: &options::BatchInput,
    params: &CompressParams,
) -> CompressResult {
    unsafe {
        // Read input data — file path or memory buffer
        let input_data: Cow<'_, [u8]> = if !input.file_path.is_null() {
            let path = match CStr::from_ptr(input.file_path).to_str() {
                Ok(s) => s,
                Err(_) => return CompressResult::error(-2, "Invalid UTF-8 in file path"),
            };
            match read_input_file(path) {
                Ok(data) => Cow::Owned(data),
                Err(ReadInputError::Io(e)) => {
                    return CompressResult::error(-3, &format!("Failed to read: {e}"));
                }
                Err(ReadInputError::TooLarge(len)) => {
                    return CompressResult::error(
                        -5,
                        &format!("Input too large ({len} bytes, max {MAX_INPUT_SIZE})"),
                    );
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
            Err(e) => return CompressResult::error(e.ffi_code(), &e.to_string()),
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_batch_result(result: *mut options::BatchResult) {
    unsafe {
        let Some(batch) = result.as_mut() else {
            return;
        };

        if !batch.results.is_null() && batch.count > 0 {
            let mut results = Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                batch.results,
                batch.count,
            ));
            for r in results.iter_mut() {
                r.release();
            }
        }

        batch.results = std::ptr::null_mut();
        batch.count = 0;
    }
}

// ─── FFI: Probe (quick metadata) ─────────────────────────────────────────────

/// Read image metadata without decoding — dimensions, format, EXIF presence.
///
/// # Safety
/// - `input_path` must be a valid null-terminated UTF-8 C string.
/// - `out` must point to a valid, writable ProbeResult.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn probe_file(
    input_path: *const libc::c_char,
    out: *mut options::ProbeResult,
) {
    unsafe {
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

        // Header-only probe: never loads the whole file into memory.
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                *out = options::ProbeResult::error(-3, &format!("Failed to read: {e}"));
                return;
            }
        };

        *out = probe_result_to_ffi(compress::probe_file(file));
    }
}

/// Read image metadata from a memory buffer without decoding.
///
/// # Safety
/// - `input_data` must point to a valid buffer of `input_len` bytes.
/// - `out` must point to a valid, writable ProbeResult.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn probe_buffer(
    input_data: *const u8,
    input_len: usize,
    out: *mut options::ProbeResult,
) {
    unsafe {
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
}

fn probe_bytes_impl(data: &[u8]) -> options::ProbeResult {
    probe_result_to_ffi(compress::probe_bytes(data))
}

fn probe_result_to_ffi(
    result: Result<compress::ProbeInfo, error::CompressError>,
) -> options::ProbeResult {
    match result {
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
                info.orientation,
            )
        }
        Err(e) => options::ProbeResult::error(-10, &e.to_string()),
    }
}

/// Free a ProbeResult's error message if present.
///
/// # Safety
/// `result` must be a valid pointer to a ProbeResult, or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_probe_result(result: *mut options::ProbeResult) {
    unsafe {
        if result.is_null() {
            return;
        }
        let r = &mut *result;
        if !r.error_message.is_null() {
            let _ = std::ffi::CString::from_raw(r.error_message);
            r.error_message = std::ptr::null_mut();
        }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn benchmark_file(
    input_path: *const libc::c_char,
    params: *const options::CompressParams,
    out: *mut options::BenchmarkResult,
) {
    unsafe {
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

        let data = match read_input_file(path) {
            Ok(d) => d,
            Err(ReadInputError::Io(e)) => {
                *out = benchmark_error(-3, &format!("Failed to read: {e}"));
                return;
            }
            Err(ReadInputError::TooLarge(len)) => {
                *out = benchmark_error(
                    -5,
                    &format!("Input file too large ({len} bytes, max {MAX_INPUT_SIZE})"),
                );
                return;
            }
        };

        let params = &*params;
        *out = benchmark_bytes_impl(&data, params);
    }
}

/// Run a quality sweep on a memory buffer.
///
/// # Safety
/// - `input_data` must point to a valid buffer of `input_len` bytes.
/// - `out` must point to a valid, writable BenchmarkResult.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn benchmark_buffer(
    input_data: *const u8,
    input_len: usize,
    params: *const options::CompressParams,
    out: *mut options::BenchmarkResult,
) {
    unsafe {
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
}

fn benchmark_bytes_impl(data: &[u8], params: &options::CompressParams) -> options::BenchmarkResult {
    match run_in_default_pool(|| compress::benchmark_bytes(data, params)) {
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
        Err(e) => benchmark_error(e.ffi_code(), &e.to_string()),
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_benchmark_result(result: *mut options::BenchmarkResult) {
    unsafe {
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
}

// ─── FFI: Version info ───────────────────────────────────────────────────────

/// Return the library version as a null-terminated C string.
/// The returned pointer is static and must NOT be freed.
#[unsafe(no_mangle)]
pub extern "C" fn ironpress_version() -> *const libc::c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const libc::c_char
}
