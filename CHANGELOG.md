## 0.3.0

### New features

- **AVIF output** (`CompressFormat.avif`) — encoded with rav1e via `ravif`, typically ~50% smaller than JPEG at equivalent visual quality. Integrates with target-size binary search, batch compression, and benchmarks. Tune with `AvifOptions(speed: 1-10)` (default 6). Output-only: AVIF input is not decoded.
- **Lossy PNG quantization** (`PngOptions(lossy: true)`) — quantizes to a palette of up to 256 colors (NeuQuant) with Floyd–Steinberg dithering before the oxipng pass. Typically 60-80% smaller on screenshots and UI graphics. The main `quality` parameter maps to palette size, which also enables target-size quality search for lossy PNG.
- **EXIF auto-orientation** (`autoOrient`, default `true`) — pixels are physically rotated to match the EXIF orientation tag (JPEG APP1, PNG eXIf, and WebP EXIF chunks), so portrait photos never come out sideways. When combined with `keepMetadata: true`, the preserved orientation tag is reset to upright so the output is never double-rotated.
- **ICC color profile preservation** (`keepIccProfile`, default `true`) — the input's ICC profile (JPEG APP2, PNG iCCP, or WebP ICCP) is carried into JPEG and PNG output, fixing color shifts on wide-gamut (Display P3) photos. Applies to both the full encode path and the direct oxipng fast path.
- **HEIC/HEIF input detection** — HEIC files now fail with a clear, actionable error message instead of a generic "unsupported format". Full HEIC decoding is intentionally not shipped: HEVC decoding requires the patent-encumbered libheif C stack, which has no pure-Rust implementation and would break the zero-toolchain prebuilt binary model.

### Improvements

- `probeFile`/`probeBytes` now detect EXIF in WebP files (previously always reported `false` for WebP)
- Benchmarks now sweep quality for lossy PNG and AVIF output, and apply EXIF orientation so results match real compression output
- `ImageFormat` gained an `avif` value for future-proofing

### Behavior changes

- Compressing a rotated (EXIF orientation > 1) image now produces upright pixels by default; pass `autoOrient: false` for the old behavior. Output width/height reflect the rotated image.
- JPEG/PNG output now retains the input's ICC profile by default; pass `keepIccProfile: false` for the old behavior. Outputs with profiles are slightly larger but color-correct.
- PNG inputs carrying an EXIF orientation tag (> 1) no longer take the direct oxipng fast path when `autoOrient` is enabled, since rotation requires decoding.
- Native ABI version bumped to 2 (`CompressParams` gained `png_lossy`, `auto_orient`, `preserve_icc`, `avif_speed`). Prebuilt binaries and Dart are versioned together; stale native libraries fail fast with an ABI mismatch error.

### Tests

- EXIF orientation: TIFF parsing (both byte orders), rotation, tag patching, opt-out
- ICC: JPEG→JPEG, PNG direct path, PNG re-encode path, JPEG→PNG carry-over, opt-out
- Lossy PNG: size reduction on photographic content, transparency preservation, target-size search
- AVIF: container validity, alpha input, target-size search
- HEIC/AVIF input detection errors

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
