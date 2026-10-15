## Unreleased

### Swift Package Manager support

- Adds Swift Package Manager support for iOS and macOS via a shared `darwin/ironpress/Package.swift` (`sharedDarwinSource: true`). CocoaPods remains supported through `darwin/ironpress.podspec`.
- iOS and macOS now ship a single dynamic `ironpress_native.xcframework` (iOS device arm64, iOS simulator arm64 + x86_64, macOS universal) used by both integrations, replacing the static iOS xcframework and `macos/libs/libironpress.dylib`. Frameworks carry an `Info.plist` with matching minimum OS versions, `@rpath` install names, a privacy manifest, and an ad-hoc signature.
- The Dart loader opens `ironpress_native.framework/ironpress_native` on iOS and macOS, falling back to `DynamicLibrary.process()` only when the engine symbols are present there.
- `scripts/build.sh apple` builds the xcframework locally.

### Compression engine: faster target-size search, EXIF orientation, smaller decodes

These change output bytes (and sometimes `qualityUsed`, `iterations`, `width`/`height`) compared to previous releases:

- **EXIF orientation is applied.** JPEG (APP1), PNG (`eXIf`), WebP (`EXIF`) and TIFF orientation tags 2-8 are now honored: pixels are rotated/flipped into display orientation before resizing, so `maxWidth`/`maxHeight` and the reported `width`/`height` refer to the displayed image (orientations 5-8 swap width and height). With `keepMetadata`, the preserved EXIF has its Orientation reset to 1 so viewers do not rotate twice. PNG inputs with an orientation tag no longer take the lossless direct-optimization path. `probe` still reports the stored (unrotated) dimensions.
- **Target-size search (`maxFileSize`) needs fewer encodes.** The quality is found by interpolating the size curve instead of a plain binary search (12 MP JPEG: 5 encodes instead of 7; WebP: 6 instead of 14), and when even `minQuality` does not fit, the size ratio decides how many 0.75× downscale steps are clearly needed so they are taken at once (still at most four steps in total) — a 12 MP JPEG that has to shrink to 0.42× now takes 7 encodes instead of 25. Every downscale resamples the original image once instead of shrinking the previous attempt again (sharper, sometimes slightly larger output). `qualityUsed` can differ by a step or two from the old binary search because encoder sizes are not perfectly monotonic in quality; outputs that fit still never exceed `maxFileSize`. `iterations` counts every encode attempt.
- **PNG output is encoded directly by oxipng** from the pixels instead of being PNG-encoded and re-decoded first (same or smaller files, ~0.3 s less for 12 MP). Target-size PNG trials use oxipng's fastest preset and only the size that fits is encoded at the requested level and verified (12 MP PNG target-size: 13.2 s → 2.0 s). oxipng gets a 10 s trial time budget; trials already running are not interrupted.
- **Large JPEG downscales decode at reduced size.** When `maxWidth`/`maxHeight` ask for at most a quarter of the source size, the JPEG is decoded at 1/2, 1/4 or 1/8 scale (DCT scaling, keeping at least twice the target size) and the remainder is resampled as before; final dimensions are unchanged. 12 MP → 1000 px peak memory drops from 82 MB to 55 MB (→ 400 px: 66 MB → 32 MB) with output size within ~1%. Adds the `jpeg-decoder` 0.3.2 crate (MIT OR Apache-2.0) for this; CMYK/16-bit or undecodable inputs fall back to the regular decoder.
- **Grayscale JPEGs stay single-channel** through decode, resize and JPEG/PNG encoding (one third of the memory; JPEG output is a 1-component JPEG).

### Batch engine, memory budget and zero-copy results

- **Native ABI version 2.** `CompressResult` gains an output-buffer handle, `ProbeResult` an `orientation` field, `CompressParams` a `png_level_unbounded` flag, `BatchResult` drops its reserved `completed` pointer, and `compress_batch` now takes `(inputs, count, params, thread_count, memory_budget, control, out)` with a caller-owned `BatchControl` block. New export: `ironpress_free_buffer`. The Dart side checks the ABI version before resolving any symbol, so a stale native library reports an ABI mismatch.
- **Batches run in a single native call, also with `onProgress` / `cancellationToken`.** Progress is a completed-items counter in shared native memory that the calling isolate polls (about every 16 ms); cancellation is a flag the engine checks before starting each image. No core idles at chunk boundaries any more. `onProgress` is called more often and with arbitrary (not chunk-aligned) values; it stays monotonic, never repeats a value, and still reports the final count exactly once.
- **Cancellation skips images that have not started** (running ones finish) instead of finishing the current chunk; the result is the processed prefix of the inputs. A batch cancelled while it is still waiting for a free worker returns an empty result (0 items).
- **Batches respect a memory budget, configurable and 512 MB by default:** `compressBatch(memoryBudgetBytes: ...)`, `null`/0 = default. Images start in order while the estimated peak memory of all running batch images (process-wide, across concurrent calls) plus the next one fits the call's budget; one oversized image still runs alone. The estimate now accounts for EXIF rotation and reduced-size JPEG decoding, so downscaling batches run with more concurrency.
- **In-memory (`data`) batch inputs:** up to a quarter of the budget (128 MB by default) they are copied to native memory once, when the batch starts; larger byte batches are copied and compressed `chunkSize` items at a time, so their bytes are read when each chunk is processed rather than up front. Keep the input buffers unchanged until the future completes. `chunkSize` no longer affects path inputs.
- **Batch work runs on a pool of long-lived worker isolates** that load the native library once and exit after 10 s idle.
- **Output bytes are not copied.** The encoder's buffer is handed to Dart as-is (no shrink/realloc in Rust, no copy in the worker or across isolates): `CompressResult.data` is an external `Uint8List` backed by native memory, released by a native finalizer when it is garbage collected (its size is reported to the GC). Results that never reach the caller (worker crash, failed send) are freed as well.
- **PNG levels 3-6 are capped for large images** (new `PngOptions.limitLevelForLargeImages`, default `true`): above 3.75 MP at most level 4, above 12 MP at most level 2; levels 0-2 are unchanged. oxipng's time budget cannot stop a deflate trial that is already running, so a 12 MP level-6 encode took ~34 s (level 4: ~10.6 s) on an M-series Mac. Set it to `false` for the previous behavior.
- **`ImageProbe.orientation`** reports the EXIF orientation (1-8, 1 when absent). `width`/`height` stay the stored dimensions; swap them for orientations 5-8 to get the displayed size.
- **Error code `-5`** now also covers images over 100 MP (rejected from the header before decoding), in addition to inputs over 256 MB.
- `CompressResult.iterations` is documented as the number of encoder runs (including target-size trial encodes).

### Breaking changes

- Minimum deployment targets are now iOS 15.0 and macOS 12.0, matching Flutter 3.47. Apps already on Flutter 3.47 are unaffected.
- The iOS/macOS native layout moved from `ios/` and `macos/` to `darwin/`. Run `flutter clean` (or `pod install`) after upgrading so stale CocoaPods integration is refreshed.

### Toolchain and dependency upgrades

- Builds packaged native libraries with Rust 1.99.0 and Android NDK r30 (30.0.16248370).
- Updates libc to 0.2.190 and refreshes the Rust lockfile (cc 1.8.0, zlib-rs 0.6.8, libdeflater 1.26.1, zerocopy 0.8.63, and others).
- Refreshes the Android Gradle Plugin to 9.4.1 and CI Flutter to 3.47.7.
- Updates example dependencies to image 4.12.0 and flutter_image_compress 2.5.1.
- Rebuilds all packaged native libraries from the updated lockfile. No public Dart API changes.

---

## 0.3.0

### Toolchain and dependency upgrades

- Requires Dart 3.13.0 or later and Flutter 3.47.0 or later.
- Updates the Rust crate to edition 2024 and publishes native version 0.3.0.
- Updates the Rust image stack to image 0.25.10, mozjpeg-rs 0.9.2, oxipng 10.2.1, fast_image_resize 6.1.0, zune-jpeg 0.5.15, rayon 1.12.0, and libc 0.2.189.
- Refreshes the Android Gradle Plugin to 9.4.0 and Android compile SDK to API 37.
- Rebuilds all packaged native libraries from the updated lockfile; macOS install names and iOS xcframework slice names remain relocatable and consistent.

### Breaking changes

- Minimum Dart and Flutter versions have increased to Dart 3.13.0 and Flutter 3.47.0.
- Native version metadata and CocoaPods specs now report 0.3.0.

---

## 0.2.0

### Performance

- **Targeted isolate offload** — single-item APIs use per-call `Isolate.run()` for failure isolation, while batch work uses an ephemeral event-driven isolate with `TransferableTypedData` to keep cross-isolate overhead low
- **Direct PNG optimization** — PNG-to-PNG compression without resize now skips the full decode/re-encode cycle and goes straight through oxipng, significantly faster for lossless workflows
- **Format-hinted decoding** — image decoding uses explicit format hints instead of `with_guessed_format()`, removing a redundant header scan on every call
- **Zero-copy batch input** — Rust batch path uses `Cow<[u8]>` for memory-buffer inputs, avoiding a needless copy
- **`TransferableTypedData`** — cross-isolate data transfer avoids copying bytes through the message port
- **Smarter thread allocation** — Rayon thread count is capped to the batch size, preventing over-allocation for small batches
- **Keyed thread pool cache** — Rust thread pools are cached by requested thread count instead of a single static pool, so different `threadCount` values reuse their own pool without rebuilding OS threads

### Bug fixes

- **`ImageFormat.fromValue` no longer silently defaults to JPEG** — unknown native format values now throw `StateError` instead of returning a wrong format, preventing subtle data corruption
- **Batch cancellation reworked** — cancellation now propagates reliably from Dart through the worker isolate via `SendPort`, fixing a race where the native `AtomicU32` progress pointer could be freed while still in use
- **`probe_bytes` header detection fixed** — uses `ImageReader::with_format` with an explicit format hint instead of `with_guessed_format`, which could fail on certain PNG and WebP variants

### Robustness

- **Input validation hardened** — `maxWidth`, `maxHeight`, `maxFileSize`, `threadCount`, `chunkSize`, and `png.optimizationLevel` are now validated before crossing the FFI boundary, with clear `ArgumentError` messages including the parameter name and allowed range
- **Native uint32 overflow protection** — all numeric parameters are checked against the `u32` ceiling before being passed to native code
- **Batch pre-cancellation check** — `compressBatch` returns empty results immediately if the token is already cancelled
- **APNG detection** — PNG files containing `acTL` chunks (animated PNG) are routed through the full decode path instead of direct oxipng optimization, which would silently drop animation frames

### Native loader

- **Multi-candidate library loading** — desktop platforms now probe multiple locations (bundled name, repo `libs/` directories up to 3 parent levels) before failing, with clear per-candidate error messages showing exactly which paths were tried and why each failed

### Breaking changes

- `ImageFormat.fromValue` throws `StateError` on unknown values instead of returning `ImageFormat.jpeg`
- `BatchCompressResult.elapsedMs` is now measured on the Dart side for the full batch operation, not inside Rust (excludes FFI overhead was misleading for chunked batches)
- `CancellationToken.addListener` is a new public method (non-breaking for consumers, but notable for subclasses)

### Tests

- Batch event-order regression test
- Batch contract tests: monotonic progress, cancellation with/without progress, mixed success/failure, output paths
- Benchmark integration tests for `benchmarkBytes` and `benchmarkFile`
- `keepMetadata` round-trip test (preserve EXIF for JPEG, drop for PNG)
- Numeric argument validation tests for all boundary conditions
- `CancellationToken` listener and disposer tests

---

## 0.1.0

**Initial release.**

### Compression engines
- JPEG compression via [mozjpeg-rs](https://crates.io/crates/mozjpeg-rs) — pure Rust, trellis quantization, progressive encoding
- PNG optimization via [oxipng](https://crates.io/crates/oxipng) — pure Rust, lossless, multithreaded
- WebP support (lossy + lossless) via the `webp` crate

### API
- `Ironpress.compressFile` — compress a file path, return bytes + stats
- `Ironpress.compressFileToFile` — compress file-to-file with no in-memory byte copy
- `Ironpress.compressBytes` — compress an in-memory `Uint8List`
- `Ironpress.compressBatch` — parallel batch compression with progress callbacks and cancellation support
- `Ironpress.probeFile` / `Ironpress.probeBytes` — read image dimensions and format without decoding pixels
- `Ironpress.benchmarkFile` / `Ironpress.benchmarkBytes` — sweep quality levels to generate a compression/size curve
- `Ironpress.nativeVersion` — verify the loaded native library version

### Robustness
- `ReceivePort`s in `compressBatch` with progress callbacks are always closed in a `finally` block
- `compressFile`, `compressFileToFile`, `compressBytes`, and `compressBatch` throw `ArgumentError` immediately when `quality` or `minQuality` is outside 0–100
- `JpegOptions`, `PngOptions`, and `CompressPreset` are annotated `@immutable`

### Example app
- Comprehensive example with 9 demo screens covering 100% of the public API
- Visual before/after comparisons, interactive sliders, bar charts, and progress indicators
- Screens: basic compression, quality presets, target file size, format comparison, batch processing, probe metadata, benchmark, advanced options (JPEG/PNG), file I/O

### Key features
- **Binary-search target file size** (`maxFileSize`): the engine loops entirely in Rust — single FFI call, no round-trips
- **Auto-resize fallback**: if quality alone can't reach `maxFileSize`, image is downscaled and retried
- **Aspect-ratio-preserving resize** via `maxWidth` / `maxHeight`
- **Batch panic safety**: one corrupt image never crashes the batch; other items continue normally
- **ABI version checking**: prevents stale native library mismatches from causing silent bugs
- **EXIF metadata preservation** for JPEG→JPEG (`keepMetadata: true`)
- **Format conversion**: auto-detect input, choose output format (JPEG / PNG / WebP lossy / WebP lossless)

### Platform support
- Android: `arm64-v8a`, `armeabi-v7a`, `x86`, `x86_64` (precompiled `.so`)
- iOS: device + simulator (precompiled `xcframework`)
- Windows: `x86_64` (precompiled `.dll`)
- Linux: `x86_64` (precompiled `.so`)
- macOS: `arm64` + `x86_64` universal (precompiled `.dylib`)
- Web: not supported (`dart:ffi` is unavailable on Flutter Web)
