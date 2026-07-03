<p align="center">
  <img src="https://raw.githubusercontent.com/rickyirfandi/ironpress/master/asset/ironpress.png" alt="ironpress logo" width="200"/>
</p>

<h1 align="center">ironpress</h1>

<p align="center">
  <strong>High-performance Rust-powered image compression for Flutter.</strong>
</p>

<p align="center">
  <a href="https://pub.dev/packages/ironpress"><img src="https://img.shields.io/pub/v/ironpress.svg" alt="pub package"></a>
  <a href="https://pub.dev/packages/ironpress/score"><img src="https://img.shields.io/pub/likes/ironpress" alt="pub likes"></a>
  <a href="https://pub.dev/packages/ironpress/score"><img src="https://img.shields.io/pub/points/ironpress" alt="pub points"></a>
  <a href="https://github.com/rickyirfandi/ironpress/actions/workflows/test.yml"><img src="https://github.com/rickyirfandi/ironpress/actions/workflows/test.yml/badge.svg" alt="tests"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="license"></a>
  <img src="https://img.shields.io/badge/platforms-android%20%7C%20iOS%20%7C%20windows%20%7C%20macOS%20%7C%20linux-blue" alt="platforms">
</p>

---

ironpress compresses JPEG, PNG, WebP, and AVIF images using mozjpeg, oxipng, libwebp, and rav1e, compiled as native Rust libraries. mozjpeg and oxipng are **state-of-the-art compression engines** trusted by major CDNs and tech companies. Single-image operations run in one native call; batch work is orchestrated chunk-by-chunk in Dart and each chunk is processed natively, which keeps progress and cancellation deterministic. Accepts JPEG, PNG, WebP, GIF, BMP, and TIFF as input.

## Features

- **mozjpeg JPEG compression** with trellis quantization (25-35% smaller than standard encoders)
- **oxipng PNG optimization**, lossless and multithreaded
- **WebP lossy and lossless** encoding
- **AVIF output** via rav1e — typically ~50% smaller than JPEG at equivalent visual quality
- **Lossy PNG quantization** (up to 256 colors, Floyd–Steinberg dithered) — 60-80% smaller screenshots and UI graphics
- **EXIF auto-orientation** — pixels are physically rotated to match the orientation tag, so portrait photos never come out sideways
- **ICC color profile preservation** for JPEG and PNG output — no color shifts on wide-gamut (Display P3) photos
- **Target file size** in a single FFI call (binary search runs entirely in Rust, zero round-trips)
- **Parallel batch compression** via Rayon with work-stealing across all cores
- **Byte-identical output** on Android, iOS, and Windows
- **Deterministic progress callbacks** and **cancellation tokens** for batch operations
- **Quality presets** (low, medium, high) for common use cases
- **Image probe** reads dimensions, format, and EXIF presence without decoding pixels
- **Quality benchmark sweep** to find optimal compression settings
- **EXIF metadata preservation** for JPEG-to-JPEG output
- **Panic-safe batch processing** (one corrupt image never kills the batch)

## What's New in 0.3.0

| Feature | How to use | Cost / trade-off |
|---|---|---|
| AVIF output | `format: CompressFormat.avif` | ~39-47% smaller than JPEG, but 2-10× slower to encode depending on `AvifOptions.speed` |
| Lossy PNG | `png: PngOptions(lossy: true)` | Up to ~3× smaller PNGs, ~1.7× slower than lossless-only; ≤256 colors |
| EXIF auto-orientation | on by default (`autoOrient: false` to opt out) | Free when no orientation tag; ~2% when a rotation is applied |
| ICC profile preservation | on by default (`keepIccProfile: false` to opt out) | Output grows by the profile size (typically a few KB); no measurable time cost |
| HEIC detection | automatic | HEIC input fails with a clear, actionable error instead of a generic one |

Auto-orientation and ICC preservation default to **on** because they fix correctness
bugs (sideways photos, color shifts on wide-gamut displays). Pass the opt-out flags if
you need byte-compatible output with 0.2.x.

## Platform Support

| Platform | Architectures | Status |
|---|---|---|
| Android | arm64, armv7, x86_64, x86 | Prebuilt |
| iOS | arm64 (device + simulator) | Prebuilt |
| Windows | x86_64 | Prebuilt |
| macOS | arm64, x86_64 (universal) | Prebuilt |
| Linux | x86_64 | Prebuilt |

All platforms ship with precompiled native libraries. No Rust toolchain is required for normal package consumers. Web is not supported (dart:ffi is unavailable on Flutter Web).

For desktop Flutter apps, the packaged native library is expected to be bundled automatically by the plugin. When running tests or examples directly from this package checkout, ironpress also probes the repo `windows/libs`, `linux/libs`, and `macos/libs` directories before failing.

## Getting Started

### Installation

```yaml
dependencies:
  ironpress: ^0.3.0
```

### Basic Usage

```dart
import 'package:ironpress/ironpress.dart';

final result = await Ironpress.compressFile(
  'photo.jpg',
  quality: 80,
);
print(result);
// CompressResult(4.2 MB -> 380.0 KB [91.0%], 4000x3000, q80, 1iter)
```

No ProGuard or R8 rules required. ironpress loads native libraries via dart:ffi with no Java or Kotlin code.

## Usage

### Target File Size

Pass `maxFileSize` and the engine handles the entire binary search in Rust with no round-trips to Dart.

```dart
final result = await Ironpress.compressFile(
  'photo.jpg',
  maxFileSize: 200 * 1024, // 200 KB
);
print('Quality: ${result.qualityUsed}, Iterations: ${result.iterations}');
```

### Batch Compression

Compress entire galleries in parallel across all available cores. Progress callbacks and cancellation are built in.

Progress is reported at chunk boundaries, so smaller `chunkSize` values give finer-grained updates. The callback is monotonic and emits the final `(total, total)` update exactly once. Cancellation works with or without `onProgress`, is observed between chunks, and preserves results for work that already completed.

```dart
final token = CancellationToken();

final batch = await Ironpress.compressBatch(
  photos.map((p) => CompressInput(path: p)).toList(),
  maxFileSize: 300 * 1024,
  maxWidth: 1920,
  threadCount: 4,
  onProgress: (done, total) => setState(() => progress = done / total),
  cancellationToken: token,
);

print(batch);
// BatchCompressResult(200 images, 6823ms, 29.3 img/s, 4.1 MB/s, 91.0% avg reduction)
```

### Quality Presets

Built-in presets for common use cases.

```dart
final result = await Ironpress.compressFile(
  'photo.jpg',
  preset: CompressPreset.medium, // q80, max 1920px
);
```

## Benchmarks

For repeatable local measurements, run the manual benchmark harness from the repo root:

```powershell
$env:IRONPRESS_BENCH_WARMUP='2'
$env:IRONPRESS_BENCH_RUNS='5'
$env:IRONPRESS_BENCH_BATCH_SIZE='24'
$env:IRONPRESS_BENCH_CHUNK_SIZE='8'
flutter test scripts\perf_benchmark_test.dart --reporter expanded
```

Optional environment variables:

- `IRONPRESS_BENCH_CORPUS_DIR`: absolute path to a real image corpus directory
- `IRONPRESS_BENCH_THREAD_COUNT`: override native batch threads
- `IRONPRESS_BENCH_QUALITY`: override the benchmark quality (default `80`)

The command prints p50/p95 latency, throughput, average output size, and observed RSS deltas for `compressFile`, `compressBytes`, `compressBatch(files)`, and `compressBatch(bytes)`.

Measured on a 2048x1536 JPEG (250 KB) at quality 80, JPEG output, no resize, no metadata. Median of 5 runs after 2 warm-ups.

### Single Image

| Package | Output | Reduction | Time | Efficiency |
|---|---|---|---|---|
| **ironpress** | 55.8 KB | 77.7% | 125 ms | 1.6 KB/ms |
| **ironpress** (fast) | 96.7 KB | 61.3% | 30.2 ms | 5.1 KB/ms |
| **flutter_image_compress** | 87.4 KB | 65.0% | 37.6 ms | 4.3 KB/ms |
| **image** (pure Dart) | 150.5 KB | 39.8% | 470 ms | 0.2 KB/ms |

### Batch (20 images)

| Package | Mode | Total output | Reduction | Time | Throughput |
|---|---|---|---|---|---|
| **ironpress** | Native batch | 1.1 MB | 77.7% | 1248 ms | 16.0 img/s |
| **ironpress** (fast) | Native batch | 1.9 MB | 61.3% | 315 ms | 63.5 img/s |
| **flutter_image_compress** | Sequential | 1.7 MB | 65.0% | 714 ms | 28.0 img/s |
| **image** (pure Dart) | Sequential | 2.9 MB | 39.8% | 9281 ms | 2.2 img/s |

> **Note:** ironpress uses mozjpeg with trellis quantization (optimizes for size). flutter_image_compress uses platform libjpeg-turbo (optimizes for speed). The "fast" entry disables trellis for a direct speed comparison — it beats libjpeg-turbo on both speed and size.

## Advanced

### JPEG Options

```dart
final result = await Ironpress.compressFile(
  'photo.jpg',
  quality: 85,
  jpeg: const JpegOptions(
    progressive: true,
    trellis: true,
    chromaSubsampling: ChromaSubsampling.yuv420,
  ),
);
```

### AVIF Output

```dart
final result = await Ironpress.compressFile(
  'photo.jpg',
  quality: 60, // AVIF stays sharp at lower quality than JPEG
  format: CompressFormat.avif,
  avif: const AvifOptions(speed: 6), // 1 = smallest/slowest, 10 = fastest
);
```

AVIF encoding is significantly slower than JPEG or WebP. Measured on a 2048×1536
photo (same machine, relative to JPEG q80 with trellis):

| Output | Encode time vs JPEG | Size vs JPEG q80 |
|---|---|---|
| AVIF q60, speed 10 | ~2.3× slower | ~39% smaller |
| AVIF q60, speed 6 (default) | ~10× slower | ~47% smaller |

Use `AvifOptions(speed: 8-10)` for interactive flows and lower speeds for
background/archival work. Combining AVIF with `maxFileSize` multiplies the cost by up
to 10 binary-search iterations — prefer high speed settings there. AVIF is output-only;
AVIF files are not accepted as input.

### Lossy PNG

```dart
final result = await Ironpress.compressFile(
  'screenshot.png',
  quality: 80, // controls palette size
  png: const PngOptions(lossy: true),
);
```

Quantizes to a dithered palette of up to 256 colors before oxipng optimization —
typically 60-80% smaller on screenshots and UI graphics. On a photo-like 1200×900 PNG
this measured ~3.2× smaller output for ~1.7× the encode time of the lossless path.

Two caveats: output is limited to 256 colors (dithering hides most banding, but this
is not for archival masters), and on *simple* flat graphics that already compress
losslessly to a few KB, lossless mode can win — the dithering noise works against
PNG's filters there. Photos with fine gradients are better served by JPEG, WebP, or
AVIF output.

### Orientation & Color Profiles

Both are on by default because they are correctness fixes, not optimizations:

- `autoOrient: true` physically rotates pixels to match the EXIF orientation tag
  (from JPEG APP1, PNG eXIf, or WebP EXIF chunks). Without it, compressed portrait
  photos display sideways in anything that ignores the tag. When combined with
  `keepMetadata: true`, the preserved orientation tag is reset to upright.
- `keepIccProfile: true` carries the input's ICC color profile (JPEG, PNG, or WebP
  input) into JPEG and PNG output. Without it, wide-gamut photos (iPhone Display P3)
  visibly shift color after compression. WebP and AVIF output does not support ICC
  embedding, so the flag has no effect for those formats.

Overhead is negligible: when the input carries no EXIF/ICC the checks are header-only
scans (within measurement noise), and when a rotation + profile embed actually happens
the total cost measured ~2% on a 3 MP photo. Output grows by roughly the embedded
profile size (sRGB ≈ 3 KB, Display P3 ≈ 0.5 KB).

### Metadata Handling

`keepMetadata: true` preserves EXIF data for JPEG-to-JPEG output. When converting to PNG or WebP, metadata is silently dropped. The flag is always safe to pass.

### HEIC Input

HEIC/HEIF files (default iPhone camera format) are **not supported** and fail with a
clear error. Decoding HEVC requires the patent-encumbered libheif C stack, which has no
pure-Rust implementation and would break ironpress's zero-toolchain prebuilt binaries.
Request JPEG from your image picker instead — `image_picker` on iOS already transcodes
HEIC to JPEG by default.

### Desktop Loading

In packaged Flutter desktop apps, the native library should be bundled automatically by the plugin and loaded by name:

- Windows: `ironpress.dll`
- Linux: `libironpress.so`
- macOS: `libironpress.dylib`

When running this package directly from a checkout, ironpress also probes the repo `windows/libs`, `linux/libs`, and `macos/libs` directories. If you are rebuilding the native code locally, update those packaged desktop libraries or point the platform loader (`PATH`, `LD_LIBRARY_PATH`, or `DYLD_LIBRARY_PATH`) at your rebuilt output.

### Diagnostics

```dart
// Read image metadata without decoding pixels
final probe = await Ironpress.probeFile('photo.jpg');
print(probe); // ImageProbe(3264x2448, JPEG, 4.2 MB, 7.99 MP)

// Find the optimal quality setting for your image
final bench = await Ironpress.benchmarkFile('photo.jpg');
print(bench.recommendedQuality); // e.g., 82
for (final entry in bench.entries) {
  print(entry); // q95: 520 KB (12.4%, 45ms)
}
```

## Size Impact

| Platform | Native library |
|---|---|
| Android arm64 | 2 MB |
| Android armv7 | 1 MB |
| Android x86 | 3 MB |
| Android x86_64 | 3 MB |
| iOS arm64 (device) | 9 MB |
| iOS simulator (arm64 + x86_64) | 18 MB |
| Linux x86_64 | 3 MB |
| macOS universal (arm64 + x86_64) | 5 MB |
| Windows x86_64 | 3 MB |
| **pub.dev package (all platforms)** | **19 MB** |

Android App Bundles automatically include only the ABI matching the user's device (~2 MB for arm64).

## API Reference

Full API documentation is available in [API.md](https://github.com/rickyirfandi/ironpress/blob/master/API.md) and on [pub.dev](https://pub.dev/documentation/ironpress/latest/).

| Method | Description |
|---|---|
| `Ironpress.compressFile()` | Compress a file, return bytes + stats |
| `Ironpress.compressFileToFile()` | Compress file to file on disk (no memory copy) |
| `Ironpress.compressBytes()` | Compress in-memory `Uint8List` |
| `Ironpress.compressBatch()` | Parallel batch compression with progress + cancellation |
| `Ironpress.probeFile()` / `probeBytes()` | Read image metadata without decoding pixels |
| `Ironpress.benchmarkFile()` / `benchmarkBytes()` | Quality sweep to find optimal settings |
| `Ironpress.nativeVersion` | Verify loaded native library version |

## Migrating from flutter_image_compress

```dart
// Before
final result = await FlutterImageCompress.compressWithFile(
  file.path,
  minWidth: 1920,
  minHeight: 1080,
  quality: 80,
);

// After
final result = await Ironpress.compressFile(
  file.path,
  maxWidth: 1920,
  maxHeight: 1080,
  quality: 80,
);
final bytes = result.data; // Same Uint8List
```

## Building from Source

All platforms ship with prebuilt binaries. You only need the Rust toolchain if you want to recompile the native libraries yourself.

```bash
# Prerequisites
cargo install cargo-ndk
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android

# Build Android
cd rust
cargo ndk --target aarch64-linux-android --platform 21 build --release
cargo ndk --target armv7-linux-androideabi --platform 21 build --release
cargo ndk --target x86_64-linux-android --platform 21 build --release

# Build Windows
cargo build --release
```

<details>
<summary><strong>Error Codes</strong></summary>

| Code | Meaning |
|---|---|
| `-1` | Null pointer or missing input (no file path or data) |
| `-2` | Invalid UTF-8 in path or empty input buffer |
| `-3` | Failed to read input file |
| `-4` | Failed to write output file |
| `-5` | Input too large (exceeds 256 MB limit) |
| `-10` | Compression engine error (decode failure, unsupported format) |
| `-99` | Internal panic during batch item (OOM or corrupt image, other items unaffected) |
| `-100` | Batch isolate crashed unexpectedly |

</details>

## Contributing

Contributions are welcome. Please open an issue before submitting a pull request.

## License

MIT License — Copyright (c) 2026 [Ricky Irfandi](https://github.com/rickyirfandi). See [LICENSE](LICENSE) for details.

Rust compression engines: [mozjpeg-rs](https://crates.io/crates/mozjpeg-rs) (BSD-3), [oxipng](https://crates.io/crates/oxipng) (MIT), [ravif](https://crates.io/crates/ravif) (BSD-3), [color_quant](https://crates.io/crates/color_quant) (MIT).
