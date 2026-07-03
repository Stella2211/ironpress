# ironpress API Reference

Complete API documentation for the `ironpress` Flutter plugin.

> For quick-start examples, see the main [README](README.md).
>
> Desktop note: packaged Flutter desktop apps should load the bundled native library by name. When running directly from this package checkout, ironpress also probes the repo `windows/libs`, `linux/libs`, and `macos/libs` directories.

---

## Table of Contents

- [Ironpress](#ironpress)
  - [compressFile](#ironpresscompressfile)
  - [compressFileToFile](#ironpresscompressfiletofile)
  - [compressBytes](#ironpresscompressbytes)
  - [compressBatch](#ironpresscompressbatch)
  - [probeFile](#ironpressprobefile)
  - [probeBytes](#ironpressprobebytes)
  - [benchmarkFile](#ironpressbenchmarkfile)
  - [benchmarkBytes](#ironpressbenchmarkbytes)
  - [nativeVersion](#ironpressnativeversion)
- [Options](#options)
  - [CompressFormat](#compressformat)
  - [JpegOptions](#jpegoptions)
  - [PngOptions](#pngoptions)
  - [AvifOptions](#avifoptions)
  - [ChromaSubsampling](#chromasubsampling)
  - [CompressPreset](#compresspreset)
- [Input & Output](#input--output)
  - [CompressInput](#compressinput)
  - [CompressResult](#compressresult)
  - [BatchCompressResult](#batchcompressresult)
  - [ImageProbe](#imageprobe)
  - [BenchmarkResult](#benchmarkresult)
  - [BenchmarkEntry](#benchmarkentry)
  - [CancellationToken](#cancellationtoken)
- [Exceptions](#exceptions)
  - [CompressException](#compressexception)
  - [Error Codes](#error-codes)

---

## Ironpress

All methods are static. The class cannot be instantiated.

```dart
import 'package:ironpress/ironpress.dart';
```

---

### `Ironpress.compressFile`

Compress an image file and return the result as bytes.

```dart
static Future<CompressResult> compressFile(
  String path, {
  CompressPreset? preset,
  int? quality,
  int? maxWidth,
  int? maxHeight,
  int? maxFileSize,
  int? minQuality,
  bool allowResize = true,
  CompressFormat format = CompressFormat.auto,
  bool keepMetadata = false,
  bool keepIccProfile = true,
  bool autoOrient = true,
  JpegOptions jpeg = const JpegOptions(),
  PngOptions png = const PngOptions(),
  AvifOptions avif = const AvifOptions(),
})
```

**Parameters:**

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `String` | required | Absolute path to the input image. Accepts JPEG, PNG, WebP, GIF, BMP, TIFF. |
| `preset` | `CompressPreset?` | `null` | Quality preset. Explicit parameters always override preset values. |
| `quality` | `int?` | `80` | JPEG/WebP quality 0–100. Ignored for PNG output. |
| `maxWidth` | `int?` | `null` | Maximum output width. Image is scaled down preserving aspect ratio. |
| `maxHeight` | `int?` | `null` | Maximum output height. Image is scaled down preserving aspect ratio. |
| `maxFileSize` | `int?` | `null` | Target maximum output size in bytes. Triggers binary-search in Rust — single FFI call, no round-trips. |
| `minQuality` | `int?` | `30` | Quality floor for the binary search when `maxFileSize` is set. |
| `allowResize` | `bool` | `true` | If quality alone can't reach `maxFileSize`, allow automatic downscaling. |
| `format` | `CompressFormat` | `auto` | Output format. |
| `keepMetadata` | `bool` | `false` | Preserve JPEG EXIF metadata (JPEG→JPEG only, silently ignored otherwise). |
| `keepIccProfile` | `bool` | `true` | Preserve the input's ICC color profile in JPEG/PNG output. Prevents color shifts on wide-gamut (Display P3) photos. |
| `autoOrient` | `bool` | `true` | Physically rotate pixels to match the EXIF orientation tag so portrait photos never come out sideways. With `keepMetadata`, the preserved orientation tag is reset to upright. |
| `jpeg` | `JpegOptions` | `JpegOptions()` | Advanced JPEG options. |
| `png` | `PngOptions` | `PngOptions()` | Advanced PNG options (including lossy quantization). |
| `avif` | `AvifOptions` | `AvifOptions()` | Advanced AVIF options. Used when `format` is `CompressFormat.avif`. |

**Returns:** `Future<CompressResult>` with compressed bytes and stats.

**Throws:**
- `ArgumentError` if `path` is empty or `quality`/`minQuality` is outside 0–100.
- `CompressException` if the native engine reports an error.

**Example:**

```dart
// Simple compression
final result = await Ironpress.compressFile(
  '/path/to/photo.jpg',
  quality: 80,
);
print(result);
// CompressResult(4.2 MB → 380 KB [91%], 1920x1440, q80, 1iter)

// Target file size — binary search runs entirely in Rust
final result = await Ironpress.compressFile(
  '/path/to/photo.jpg',
  maxFileSize: 200 * 1024, // 200 KB
  maxWidth: 1920,
);
```

---

### `Ironpress.compressFileToFile`

Compress an image file and write the result directly to disk.

```dart
static Future<CompressResult> compressFileToFile(
  String inputPath,
  String outputPath, {
  // Same parameters as compressFile
})
```

**Returns:** `Future<CompressResult>` with stats but `data` is `null` (output written to disk).

**Throws:**
- `ArgumentError` if `inputPath` or `outputPath` is empty.
- `CompressException` if the native engine reports an error.

**Example:**

```dart
final result = await Ironpress.compressFileToFile(
  '/input/photo.jpg',
  '/output/photo_compressed.jpg',
  preset: CompressPreset.medium,
);
assert(result.isFileOutput); // true — no bytes in memory
print('Saved ${result.compressedSize} bytes to disk');
```

---

### `Ironpress.compressBytes`

Compress raw image bytes in memory.

```dart
static Future<CompressResult> compressBytes(
  Uint8List data, {
  // Same parameters as compressFile
})
```

Accepts JPEG, PNG, WebP, GIF, BMP, or TIFF input bytes.

**Throws:**
- `ArgumentError` if `data` is empty.
- `CompressException` if the native engine reports an error.

**Example:**

```dart
final bytes = await File('photo.jpg').readAsBytes();

final result = await Ironpress.compressBytes(
  bytes,
  quality: 80,
);

// Convert to WebP
final webpResult = await Ironpress.compressBytes(
  bytes,
  format: CompressFormat.webpLossy,
  quality: 80,
);
```

---

### `Ironpress.compressBatch`

Batch compress multiple images using Rust's rayon thread pool.

Batch work is orchestrated chunk-by-chunk. Each chunk is processed natively, progress is reported after each completed chunk, and cancellation is observed before the next chunk starts. Already-completed results are preserved when cancellation is requested.

```dart
static Future<BatchCompressResult> compressBatch(
  List<CompressInput> inputs, {
  CompressPreset? preset,
  int? quality,
  int? maxWidth,
  int? maxHeight,
  int? maxFileSize,
  int? minQuality,
  bool allowResize = true,
  CompressFormat format = CompressFormat.auto,
  bool keepMetadata = false,
  bool keepIccProfile = true,
  bool autoOrient = true,
  JpegOptions jpeg = const JpegOptions(),
  PngOptions png = const PngOptions(),
  AvifOptions avif = const AvifOptions(),
  int threadCount = 0,
  int chunkSize = 8,
  void Function(int completed, int total)? onProgress,
  CancellationToken? cancellationToken,
})
```

**Additional parameters:**

| Parameter | Type | Default | Description |
|---|---|---|---|
| `threadCount` | `int` | `0` | Number of Rayon threads. `0` = auto (cores − 2, leaving room for Flutter UI). |
| `chunkSize` | `int` | `8` | Images decoded simultaneously per chunk. Controls peak memory (~36 MB per 4K image). Between chunks, pixel buffers are freed. |
| `onProgress` | `Function?` | `null` | Called on the main thread with `(completed, total)`. Safe to call `setState` directly. |
| `cancellationToken` | `CancellationToken?` | `null` | Cancel between chunks. Returns partial results for completed chunks. |

Validation note: `threadCount` must be `>= 0`, `chunkSize` must be `> 0`, and numeric resize/size arguments must be positive and fit the native `u32` ABI.

Behavior note: `onProgress` fires after each completed chunk and emits the final `(total, total)` callback exactly once. `CancellationToken.cancel()` stops scheduling later chunks after the current chunk finishes.

**Safety guarantees:**
- Each item is panic-safe: a corrupt image produces an error result, not a process crash.
- `chunkSize` limits peak memory (default 8 ≈ ~288 MB for 4K photos).
- `threadCount` defaults to `cores − 2`, keeping the UI responsive.

**Example:**

```dart
final token = CancellationToken();

final result = await Ironpress.compressBatch(
  photos.map((p) => CompressInput(path: p)).toList(),
  preset: CompressPreset.medium,
  maxFileSize: 300 * 1024,
  threadCount: 4,
  onProgress: (done, total) {
    setState(() => _progress = done / total);
  },
  cancellationToken: token,
);

print(result);
// BatchCompressResult(200 images, 6823ms, 29.3 img/s, 4.1 MB/s, 91.0% avg reduction)
```

---

### `Ironpress.probeFile`

Read image dimensions, format, and EXIF presence from a file without decoding pixels.

```dart
static Future<ImageProbe> probeFile(String path)
```

**Returns:** `Future<ImageProbe>` with metadata.

**Throws:**
- `ArgumentError` if `path` is empty.
- `CompressException` if the file cannot be read or parsed.

**Example:**

```dart
final info = await Ironpress.probeFile('/path/to/photo.jpg');
print(info); // ImageProbe(4000x3000, JPEG, 4.2 MB, 12.0MP, EXIF)

if (info.megapixels > 12) {
  await Ironpress.compressFile(path, preset: CompressPreset.medium);
}
```

---

### `Ironpress.probeBytes`

Read image metadata from bytes without decoding pixel data.

```dart
static Future<ImageProbe> probeBytes(Uint8List data)
```

**Throws:**
- `ArgumentError` if `data` is empty.
- `CompressException` if the data cannot be parsed.

---

### `Ironpress.benchmarkFile`

Run a quality sweep: encode at multiple quality levels and measure output size and speed.

```dart
static Future<BenchmarkResult> benchmarkFile(
  String path, {
  int? maxWidth,
  int? maxHeight,
})
```

Returns a `BenchmarkResult` with entries for each quality level and a `recommendedQuality` field with the best size/quality trade-off.

**Throws:**
- `ArgumentError` if `path` is empty.
- `CompressException` if the file cannot be read or parsed.

**Example:**

```dart
final bench = await Ironpress.benchmarkFile('/path/to/photo.jpg');
print(bench.recommendedQuality); // e.g., 78

for (final entry in bench.entries) {
  print(entry); // q95: 1.2 MB (70.5%, 210ms)
}
```

---

### `Ironpress.benchmarkBytes`

Run a quality sweep on raw image bytes.

```dart
static Future<BenchmarkResult> benchmarkBytes(
  Uint8List data, {
  int? maxWidth,
  int? maxHeight,
})
```

**Throws:**
- `ArgumentError` if `data` is empty.

---

### `Ironpress.nativeVersion`

Return the native library version string.

```dart
static String get nativeVersion
```

**Example:**

```dart
print(Ironpress.nativeVersion); // "0.1.0"
```

---

## Options

### CompressFormat

Output image format.

```dart
enum CompressFormat {
  auto,         // Keep same format as input (default)
  jpeg,         // JPEG — lossy, best for photos
  png,          // PNG — lossless (or lossy with PngOptions.lossy)
  webpLossless, // WebP lossless — typically 25-35% smaller than PNG
  webpLossy,    // WebP lossy — often smaller than JPEG at same quality
  avif,         // AVIF — ~50% smaller than JPEG, slower to encode. Output only.
}
```

---

### JpegOptions

Advanced JPEG encoding options. All have sensible defaults.

```dart
const JpegOptions({
  bool progressive = true,
  ChromaSubsampling chromaSubsampling = ChromaSubsampling.yuv420,
  bool trellis = true,
})
```

| Field | Type | Default | Description |
|---|---|---|---|
| `progressive` | `bool` | `true` | Progressive JPEG encoding. Produces smaller files. |
| `chromaSubsampling` | `ChromaSubsampling` | `yuv420` | Chroma subsampling mode. |
| `trellis` | `bool` | `true` | Trellis quantization — mozjpeg's killer feature. Produces measurably smaller files at the same visual quality. Set to `false` for faster encoding at the cost of larger output. |

---

### PngOptions

Advanced PNG optimization options.

```dart
const PngOptions({int optimizationLevel = 2, bool lossy = false})
```

| Field | Type | Default | Description |
|---|---|---|---|
| `optimizationLevel` | `int` | `2` | Optimization level 0–6. `0` = no optimization (fastest), `2` = good balance, `6` = maximum compression (slowest). |
| `lossy` | `bool` | `false` | Quantize to a palette of up to 256 colors with Floyd–Steinberg dithering before optimization. Typically 60-80% smaller on screenshots and UI graphics. The main `quality` parameter controls the palette size. |

---

### AvifOptions

Advanced AVIF encoding options. Only used when `format` is `CompressFormat.avif`.

```dart
const AvifOptions({int speed = 6})
```

| Field | Type | Default | Description |
|---|---|---|---|
| `speed` | `int` | `6` | Encoder speed 1–10. `1-3` = very slow, best compression (archival), `6` = good balance, `10` = fastest, larger output. |

AVIF encoding is significantly slower than JPEG or WebP — expect seconds rather than
milliseconds for large photos at low speed settings. AVIF is an **output-only** format:
AVIF files are not accepted as input.

---

### ChromaSubsampling

JPEG chroma subsampling mode.

```dart
enum ChromaSubsampling {
  yuv420, // 4:2:0 — best compression, slight color loss (default)
  yuv422, // 4:2:2 — balanced
  yuv444, // 4:4:4 — no chroma loss, larger files
}
```

---

### CompressPreset

Built-in quality presets. Explicit parameters always override preset values.

| Preset | Quality | Min Quality | Max Dimension | Use Case |
|---|---|---|---|---|
| `CompressPreset.low` | 65 | 25 | 1280 px | Social media, messaging |
| `CompressPreset.medium` | 80 | 35 | 1920 px | General uploads, in-app photos |
| `CompressPreset.high` | 92 | 70 | No resize | Archives, professional use |

**Example:**

```dart
// Use preset as-is
final result = await Ironpress.compressFile(
  'photo.jpg',
  preset: CompressPreset.medium,
);

// Override one field
final result = await Ironpress.compressFile(
  'photo.jpg',
  preset: CompressPreset.medium,
  maxWidth: 1280, // override just this
);
```

---

## Input & Output

### CompressInput

Input descriptor for batch compression. Exactly one of `path` or `data` must be provided.

```dart
const CompressInput({
  String? path,        // File path (mutually exclusive with data)
  Uint8List? data,     // Raw bytes (mutually exclusive with path)
  String? outputPath,  // Output path. If null, result returned as bytes.
})
```

---

### CompressResult

Compression result with stats.

**Fields:**

| Field | Type | Description |
|---|---|---|
| `data` | `Uint8List?` | Compressed bytes. `null` when output was written to file via `compressFileToFile`. |
| `originalSize` | `int` | Original input size in bytes. |
| `compressedSize` | `int` | Compressed output size in bytes. |
| `width` | `int` | Final image width in pixels. |
| `height` | `int` | Final image height in pixels. |
| `qualityUsed` | `int` | Actual quality used. May differ from requested if `maxFileSize` was set. |
| `iterations` | `int` | Number of compression iterations (1 if no `maxFileSize`). |
| `resizedToFit` | `bool` | Whether the image was auto-resized to meet the file size target. |
| `errorCode` | `int?` | Error code for failed batch items. `null` for success. |
| `errorMessage` | `String?` | Error message for failed batch items. `null` for success. |

**Computed properties:**

| Property | Type | Description |
|---|---|---|
| `ratio` | `double` | Compression ratio (e.g., 0.35 means 65% reduction). |
| `reductionPercent` | `String` | Human-readable reduction (e.g., "65.2%"). |
| `isSuccess` | `bool` | Whether compression completed successfully. |
| `isFileOutput` | `bool` | Whether this was a file-to-file operation. |

**toString example:**

```
CompressResult(4.2 MB → 380.0 KB [91.0%], 4000x3000, q80, 1iter)
```

---

### BatchCompressResult

Aggregate result of a batch compression operation.

**Fields:**

| Field | Type | Description |
|---|---|---|
| `results` | `List<CompressResult>` | Individual results, one per input. |
| `elapsedMs` | `int` | Total wall-clock time for the full batch operation (measured on the Dart side). |

**Computed properties:**

| Property | Type | Description |
|---|---|---|
| `successfulCount` | `int` | Number of successful results. |
| `failedCount` | `int` | Number of failed results. |
| `hasFailures` | `bool` | Whether any item failed. |
| `totalOriginalSize` | `int` | Total input size of all items. |
| `totalCompressedSize` | `int` | Total output size of all items. |
| `averageRatio` | `double` | Average compression ratio. |
| `imagesPerSecond` | `double` | Throughput in images/s. |
| `mbPerSecond` | `double` | Throughput in MB/s of input data. |

---

### ImageProbe

Quick image metadata from file headers — no pixel decoding.

**Fields:**

| Field | Type | Description |
|---|---|---|
| `width` | `int` | Image width in pixels. |
| `height` | `int` | Image height in pixels. |
| `format` | `ImageFormat` | Detected format (`jpeg`, `png`, `webp`). |
| `fileSize` | `int` | File size in bytes. |
| `hasExif` | `bool` | Whether EXIF metadata is present. |

**Computed properties:**

| Property | Type | Description |
|---|---|---|
| `pixelCount` | `int` | Total pixel count. |
| `megapixels` | `double` | Megapixels (e.g., 12.0 for 4000x3000). |

**toString example:**

```
ImageProbe(4000x3000, JPEG, 4.2 MB, 12.0MP, EXIF)
```

---

### BenchmarkResult

Quality sweep result across multiple levels.

**Fields:**

| Field | Type | Description |
|---|---|---|
| `originalSize` | `int` | Original file size in bytes. |
| `width` | `int` | Image width. |
| `height` | `int` | Image height. |
| `format` | `ImageFormat` | Detected format. |
| `entries` | `List<BenchmarkEntry>` | Quality sweep entries, highest quality first. |
| `recommendedQuality` | `int` | Best size/quality trade-off. |

---

### BenchmarkEntry

Single quality point from a benchmark sweep.

**Fields:**

| Field | Type | Description |
|---|---|---|
| `quality` | `int` | Quality level (0–100). |
| `sizeBytes` | `int` | Compressed output size in bytes. |
| `ratio` | `double` | Compression ratio. |
| `encodeMs` | `int` | Encoding time in milliseconds. |

**Computed properties:**

| Property | Type | Description |
|---|---|---|
| `sizeFormatted` | `String` | Human-readable size. |
| `reductionPercent` | `String` | Human-readable reduction. |

**toString example:**

```
q85: 620 KB (85.2%, 180ms)
```

---

### CancellationToken

Token for cancelling batch compression between chunks.

```dart
class CancellationToken {
  bool get isCancelled;              // Whether cancellation was requested
  void cancel();                     // Request cancellation
  void reset();                      // Reset for reuse
  void Function() addListener(       // Register a callback; returns a disposer
    void Function() listener,
  );
}
```

**Example:**

```dart
final token = CancellationToken();

final future = Ironpress.compressBatch(
  inputs,
  cancellationToken: token,
);

// Cancel after 5 seconds
Future.delayed(Duration(seconds: 5), () => token.cancel());

final result = await future; // Contains partial results
```

---

## Exceptions

### CompressException

Thrown when the native Rust engine reports an error.

```dart
class CompressException implements Exception {
  final int code;      // Native error code
  final String message; // Human-readable message
}
```

### Error Codes

| Code | Meaning |
|---|---|
| `-1` | Null pointer or missing input (no file path or data) |
| `-2` | Invalid UTF-8 in path or empty input buffer |
| `-3` | Failed to read input file |
| `-4` | Failed to write output file |
| `-5` | Input too large (exceeds 256 MB limit) |
| `-10` | Compression engine error (decode failure, unsupported format) |
| `-99` | Internal panic during batch item (OOM or corrupt image — other items unaffected) |
| `-100` | Batch isolate crashed unexpectedly |
