# ironpress API Reference

Complete API documentation for the `ironpress` Flutter plugin.

> For quick-start examples, see the main [README](README.md).
>
> Loading note: packaged Flutter apps load the bundled native library by name (`ironpress_native.framework/ironpress_native` on iOS and macOS, embedded via Swift Package Manager or CocoaPods). When running directly from this package checkout, ironpress also probes the repo `windows/libs`, `linux/libs`, and `darwin/ironpress/Frameworks` directories.

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
  JpegOptions jpeg = const JpegOptions(),
  PngOptions png = const PngOptions(),
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
| `jpeg` | `JpegOptions` | `JpegOptions()` | Advanced JPEG options. |
| `png` | `PngOptions` | `PngOptions()` | Advanced PNG options. |

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

The whole batch normally runs in a single native call, so no thread idles at chunk boundaries — also with `onProgress` and `cancellationToken`. Progress and cancellation go through a small shared native block: the engine increments a completed-items counter that the calling isolate polls (about every 16 ms), and cancelling sets a flag the engine checks before starting each image. Images that have not started are skipped; running images finish and their results are returned.

In-memory (`data`) inputs are copied to native memory: all at once when the batch starts if their total size is at most a quarter of the memory budget (128 MB by default), otherwise `chunkSize` items at a time (plus one chunk of prefetch), with one native call per chunk. Do not modify the input buffers until the returned future completes.

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
  JpegOptions jpeg = const JpegOptions(),
  PngOptions png = const PngOptions(),
  int threadCount = 0,
  int chunkSize = 8,
  int? memoryBudgetBytes,
  void Function(int completed, int total)? onProgress,
  CancellationToken? cancellationToken,
})
```

**Additional parameters:**

| Parameter | Type | Default | Description |
|---|---|---|---|
| `threadCount` | `int` | `0` | Number of Rayon threads. `0` = auto (cores − 2, leaving room for Flutter UI). |
| `chunkSize` | `int` | `8` | Only used when the in-memory (`data`) inputs total more than a quarter of the memory budget: they are then copied to native memory and compressed this many at a time, bounding the extra native memory to about two chunks of input bytes. Path inputs and smaller byte batches run in one native call and ignore it. Concurrency and decode memory are bounded by `threadCount` and `memoryBudgetBytes` instead. |
| `memoryBudgetBytes` | `int?` | `null` (512 MB) | Budget for the estimated peak memory of the images being compressed at once. `null` or `0` = default (512 MB). Images start in order while the estimate of all running batch images plus the next one fits; a single image larger than the budget still runs, alone. See *Memory budget* below. |
| `onProgress` | `Function?` | `null` | Called on the calling isolate with `(completed, total)`. Safe to call `setState` directly. |
| `cancellationToken` | `CancellationToken?` | `null` | Stops starting new images. Returns the results of the images that were processed (a prefix of `inputs`, in order). |

Validation note: `threadCount` must be `>= 0`, `chunkSize` must be `> 0`, `memoryBudgetBytes` must be `>= 0` (or `null`), and numeric resize/size arguments must be positive and fit the native `u32` ABI.

Behavior note: `completed` counts finished images (successful or failed). `onProgress` is called whenever the polled count has increased — several images may finish between two calls, so not every value is reported — and never twice with the same value; it is monotonic, and the last call reports the final result count (`(total, total)` exactly once unless cancelled). `CancellationToken.cancel()` takes effect immediately: images not yet started are skipped. Cancelling before the batch has started (e.g. while it waits for a free worker) returns an empty result.

**Memory budget:** each image's peak memory is estimated from its header only — the decoded size (after EXIF rotation and, for JPEG downscales, the reduced DCT-scaled decode size) plus the encoder's working set. The accounting is process-wide: an image is admitted when the estimates of *all* running batch images, from every concurrent `compressBatch` call, plus the new image fit that call's own budget. With equal budgets, concurrent batches therefore share one limit; a batch with a smaller budget waits while others keep usage high, and total usage never exceeds the largest active budget (plus at most one oversized image).

**Safety guarantees:**
- Each item is panic-safe: a corrupt image produces an error result, not a process crash.
- `memoryBudgetBytes` bounds the estimated decode/encode memory of concurrently running images (default 512 MB).
- `threadCount` defaults to `cores − 2`, keeping the UI responsive.
- Output bytes are handed over without copying: `CompressResult.data` is backed by native memory that is freed when the list is garbage collected.

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
print(Ironpress.nativeVersion); // "0.3.0"
```

---

## Options

### CompressFormat

Output image format.

```dart
enum CompressFormat {
  auto,         // Keep same format as input (default)
  jpeg,         // JPEG — lossy, best for photos
  png,          // PNG — lossless
  webpLossless, // WebP lossless — typically 25-35% smaller than PNG
  webpLossy,    // WebP lossy — often smaller than JPEG at same quality
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
const PngOptions({
  int optimizationLevel = 2,
  bool limitLevelForLargeImages = true,
})
```

| Field | Type | Default | Description |
|---|---|---|---|
| `optimizationLevel` | `int` | `2` | Optimization level 0–6. `0` = no optimization (fastest), `2` = good balance, `6` = maximum compression (slowest). |
| `limitLevelForLargeImages` | `bool` | `true` | Lower levels above 2 for large images (size after any resize): up to 3.75 MP the requested level is used, above 3.75 MP at most level 4, above 12 MP at most level 2. Levels 0–2 are never changed. Reference (worst-case 12 MP image, Apple M-series): level 2 ≈ 6 s, level 4 ≈ 10.6 s, level 6 ≈ 34 s; phones are several times slower. `false` = always use `optimizationLevel` as given. |

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
| `iterations` | `int` | Number of encode attempts (encoder runs). `1` without `maxFileSize`; with `maxFileSize`, every trial encode of the target-size search, including fast PNG trial encodes. |
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
| `width` | `int` | Image width in pixels, as stored (before EXIF orientation). |
| `height` | `int` | Image height in pixels, as stored (before EXIF orientation). |
| `format` | `ImageFormat` | Detected format (`jpeg`, `png`, `webp`). |
| `fileSize` | `int` | File size in bytes. |
| `hasExif` | `bool` | Whether EXIF metadata is present. |
| `orientation` | `int` | EXIF orientation 1–8 (`1` when absent). Compression applies it, so the displayed/compressed image has `width` and `height` swapped when `orientation` is 5–8. |

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

Token for cancelling a batch compression. Cancelling stops the batch from starting new images; images already being compressed finish and their results are returned.

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
| `-5` | Input too large: file/buffer exceeds 256 MB, or the image exceeds 100 MP (checked from the header, before decoding) |
| `-10` | Compression engine error (decode failure, unsupported format) |
| `-99` | Internal panic during batch item (OOM or corrupt image — other items unaffected) |
| `-100` | Batch isolate crashed unexpectedly |
