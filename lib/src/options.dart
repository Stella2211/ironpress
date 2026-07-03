import 'dart:typed_data';

import 'package:flutter/foundation.dart' show immutable;

// ─── Shared utility ──────────────────────────────────────────────────────────

String formatBytes(int bytes) {
  if (bytes < 1024) return '$bytes B';
  if (bytes < 1024 * 1024) return '${(bytes / 1024).toStringAsFixed(1)} KB';
  return '${(bytes / (1024 * 1024)).toStringAsFixed(1)} MB';
}

// ─── Enums ───────────────────────────────────────────────────────────────────

/// Output image format.
enum CompressFormat {
  /// Keep the same format as input (default).
  auto(0),

  /// JPEG output — lossy, best for photos.
  jpeg(1),

  /// PNG output — lossless.
  png(2),

  /// WebP lossless — pure Rust, no C deps.
  /// Typically 25-35% smaller than PNG for graphics/screenshots.
  /// For photos, JPEG via mozjpeg is still smaller.
  webpLossless(3),

  /// WebP lossy — quality-based, often more useful than lossless for photos.
  /// Typically smaller than JPEG at equivalent visual quality.
  webpLossy(4),

  /// AVIF — modern format, typically ~50% smaller than JPEG at equivalent
  /// visual quality. Encoding is significantly slower than JPEG/WebP;
  /// tune with [AvifOptions.speed]. Output only — AVIF input is not decoded.
  avif(5);

  const CompressFormat(this.value);
  final int value;
}

/// JPEG chroma subsampling mode.
enum ChromaSubsampling {
  /// 4:2:0 — best compression, slight color loss (default, recommended).
  yuv420(0),

  /// 4:2:2 — balanced.
  yuv422(1),

  /// 4:4:4 — no chroma loss, larger files.
  yuv444(2);

  const ChromaSubsampling(this.value);
  final int value;
}

// ─── Options ─────────────────────────────────────────────────────────────────

/// Advanced JPEG encoding options.
///
/// These expose mozjpeg's powerful features that most Flutter packages
/// don't offer. All have sensible defaults.
@immutable
class JpegOptions {
  const JpegOptions({
    this.progressive = true,
    this.chromaSubsampling = ChromaSubsampling.yuv420,
    this.trellis = true,
  });

  /// Enable progressive JPEG encoding. Produces smaller files for web.
  /// Default: `true`.
  final bool progressive;

  /// Chroma subsampling mode. Default: [ChromaSubsampling.yuv420].
  final ChromaSubsampling chromaSubsampling;

  /// Enable trellis quantization — mozjpeg's killer feature.
  /// Produces measurably smaller files at the same quality.
  /// Default: `true`.
  final bool trellis;
}

/// Advanced PNG optimization options.
@immutable
class PngOptions {
  const PngOptions({this.optimizationLevel = 2, this.lossy = false});

  /// Optimization level 0-6. Higher = slower but smaller output.
  ///
  /// - 0: No optimization (fastest)
  /// - 2: Good balance (default)
  /// - 6: Maximum compression (slowest)
  final int optimizationLevel;

  /// Enable lossy palette quantization (up to 256 colors, Floyd–Steinberg
  /// dithered) before optimization. Typically shrinks screenshots and UI
  /// graphics by 60-80% versus lossless-only optimization.
  ///
  /// The main `quality` parameter controls the palette size: higher quality
  /// keeps more colors. Default: `false` (lossless).
  final bool lossy;
}

/// Advanced AVIF encoding options.
@immutable
class AvifOptions {
  const AvifOptions({this.speed = 6});

  /// Encoder speed 1-10. Lower is slower but compresses better.
  ///
  /// - 1-3: Very slow, best compression (archival)
  /// - 6: Good balance (default)
  /// - 10: Fastest, larger output
  final int speed;
}

// ─── Quality Presets ─────────────────────────────────────────────────────────

/// Built-in quality presets for common use cases.
///
/// Pass to [Ironpress.compressFile], [Ironpress.compressBytes], or
/// [Ironpress.compressBatch] via the `preset` parameter. Individual
/// parameters always take priority over the preset value.
///
/// ```dart
/// // One-liner with sensible defaults
/// final result = await Ironpress.compressFile(
///   'photo.jpg',
///   preset: CompressPreset.medium,
/// );
///
/// // Override one field while keeping the rest from the preset
/// final result = await Ironpress.compressFile(
///   'photo.jpg',
///   preset: CompressPreset.medium,
///   maxWidth: 1280, // override just this
/// );
/// ```
@immutable
class CompressPreset {
  const CompressPreset._({
    required this.quality,
    required this.minQuality,
    this.maxWidth,
    this.maxHeight,
  });

  /// Social media / messaging. Small file size, fast upload.
  ///
  /// - JPEG quality: 65
  /// - Max dimension: 1280 px (long edge)
  /// - Min quality floor: 25 (aggressive target-size search)
  static const low = CompressPreset._(
    quality: 65,
    minQuality: 25,
    maxWidth: 1280,
    maxHeight: 1280,
  );

  /// General uploads / in-app photos. Good balance of size and quality.
  ///
  /// - JPEG quality: 80
  /// - Max dimension: 1920 px (long edge)
  /// - Min quality floor: 35
  static const medium = CompressPreset._(
    quality: 80,
    minQuality: 35,
    maxWidth: 1920,
    maxHeight: 1920,
  );

  /// High-quality archives or professional use. Minimal compression.
  ///
  /// - JPEG quality: 92
  /// - No resize applied
  /// - Min quality floor: 70
  static const high = CompressPreset._(quality: 92, minQuality: 70);

  /// JPEG quality (0–100) for this preset.
  final int quality;

  /// Minimum quality floor used during binary-search file size targeting.
  final int minQuality;

  /// Maximum width in pixels, or `null` for no constraint.
  final int? maxWidth;

  /// Maximum height in pixels, or `null` for no constraint.
  final int? maxHeight;
}

// ─── Batch Input ─────────────────────────────────────────────────────────────

/// Input descriptor for batch compression.
class CompressInput {
  const CompressInput({this.path, this.data, this.outputPath})
    : assert(
        (path != null) != (data != null),
        'Exactly one of path or data must be provided',
      );

  /// File path to compress (mutually exclusive with [data]).
  final String? path;

  /// Raw image bytes to compress (mutually exclusive with [path]).
  final Uint8List? data;

  /// Output path. If null, result is returned as bytes.
  final String? outputPath;
}

// ─── Cancellation ────────────────────────────────────────────────────────────

/// Token for cancelling batch compression between chunks.
///
/// ```dart
/// final token = CancellationToken();
///
/// // Start batch
/// final future = Ironpress.compressBatch(
///   inputs,
///   cancellationToken: token,
/// );
///
/// // Cancel after 5 seconds
/// Future.delayed(Duration(seconds: 5), () => token.cancel());
///
/// final result = await future; // Contains partial results
/// ```
class CancellationToken {
  bool _cancelled = false;
  final Set<void Function()> _listeners = <void Function()>{};

  /// Whether cancellation has been requested.
  bool get isCancelled => _cancelled;

  /// Request cancellation. The batch will stop after the current chunk
  /// completes and return partial results.
  void cancel() {
    if (_cancelled) {
      return;
    }
    _cancelled = true;
    for (final listener in List<void Function()>.from(_listeners)) {
      listener();
    }
  }

  /// Reset the token for reuse.
  void reset() => _cancelled = false;

  /// Registers a callback that runs when [cancel] is invoked.
  ///
  /// Returns a disposer that removes the listener.
  void Function() addListener(void Function() listener) {
    _listeners.add(listener);
    return () {
      _listeners.remove(listener);
    };
  }
}

// ─── Result ──────────────────────────────────────────────────────────────────

/// Compression result with stats.
///
/// This provides transparency into what the compression engine actually did,
/// which is invaluable for logging and debugging.
class CompressResult {
  const CompressResult({
    this.data,
    required this.originalSize,
    required this.compressedSize,
    required this.width,
    required this.height,
    required this.qualityUsed,
    required this.iterations,
    required this.resizedToFit,
    this.errorCode,
    this.errorMessage,
  });

  /// Compressed image data. Null when output was written to a file
  /// via [Ironpress.compressFileToFile].
  final Uint8List? data;

  /// Original input size in bytes.
  final int originalSize;

  /// Compressed output size in bytes.
  final int compressedSize;

  /// Final image width in pixels.
  final int width;

  /// Final image height in pixels.
  final int height;

  /// Actual quality used by the encoder.
  /// May differ from requested if [maxFileSize] was set.
  final int qualityUsed;

  /// Number of compression iterations (1 if no [maxFileSize]).
  final int iterations;

  /// Whether the image was auto-resized to meet the file size target.
  final bool resizedToFit;

  /// Native error code for batch items that failed.
  /// Null for successful results.
  final int? errorCode;

  /// Native error message for batch items that failed.
  /// Null for successful results.
  final String? errorMessage;

  /// Compression ratio (e.g., 0.35 means 65% reduction).
  double get ratio => originalSize > 0 ? compressedSize / originalSize : 1.0;

  /// Human-readable reduction percentage (e.g., "65.2%").
  String get reductionPercent => '${((1.0 - ratio) * 100).toStringAsFixed(1)}%';

  /// Whether the compression completed successfully.
  bool get isSuccess => errorCode == null;

  /// Whether the result was a file-to-file operation (output written to disk,
  /// no in-memory data returned).
  bool get isFileOutput => data == null && isSuccess;

  @override
  String toString() {
    if (!isSuccess) {
      final code = errorCode != null ? '($errorCode)' : '';
      return 'CompressResult$code: ${errorMessage ?? 'unknown error'}';
    }

    return 'CompressResult('
        '${formatBytes(originalSize)} → ${formatBytes(compressedSize)} '
        '[$reductionPercent], '
        '${width}x$height, '
        'q$qualityUsed, '
        '${iterations}iter'
        '${resizedToFit ? ', auto-resized' : ''}'
        ')';
  }
}

/// Result of a batch compression operation.
///
/// Contains individual results for each input plus aggregate stats.
class BatchCompressResult {
  const BatchCompressResult({required this.results, required this.elapsedMs});

  /// Individual compression results, one per input.
  final List<CompressResult> results;

  /// Total wall-clock time for the entire batch (milliseconds).
  ///
  /// This is measured on the Dart side for the full batch operation.
  final int elapsedMs;

  /// Number of successful compression results.
  int get successfulCount => results.where((r) => r.isSuccess).length;

  /// Number of failed compression results.
  int get failedCount => results.length - successfulCount;

  /// Whether any item failed during compression.
  bool get hasFailures => failedCount > 0;

  /// Total original size of all inputs combined.
  int get totalOriginalSize =>
      results.fold(0, (sum, r) => sum + r.originalSize);

  /// Total compressed size of all outputs combined.
  int get totalCompressedSize =>
      results.fold(0, (sum, r) => sum + r.compressedSize);

  /// Average compression ratio across all results.
  double get averageRatio =>
      totalOriginalSize > 0 ? totalCompressedSize / totalOriginalSize : 1.0;

  /// Throughput in images per second.
  double get imagesPerSecond =>
      elapsedMs > 0 ? results.length / (elapsedMs / 1000.0) : 0;

  /// Throughput in MB/s of input data processed.
  double get mbPerSecond =>
      elapsedMs > 0
          ? (totalOriginalSize / (1024 * 1024)) / (elapsedMs / 1000.0)
          : 0;

  @override
  String toString() =>
      'BatchCompressResult('
      '${results.length} images, '
      '${elapsedMs}ms, '
      '${imagesPerSecond.toStringAsFixed(1)} img/s, '
      '${mbPerSecond.toStringAsFixed(1)} MB/s, '
      '${((1.0 - averageRatio) * 100).toStringAsFixed(1)}% avg reduction'
      '${hasFailures ? ', $failedCount failed' : ''}'
      ')';
}

// ─── Probe Result ────────────────────────────────────────────────────────────

/// Detected image format from file header.
enum ImageFormat {
  jpeg(1),
  png(2),
  webp(3),
  avif(5);

  const ImageFormat(this.value);
  final int value;

  static ImageFormat fromValue(int v) {
    switch (v) {
      case 1:
        return jpeg;
      case 2:
        return png;
      case 3:
        return webp;
      case 4:
        return webp;
      case 5:
        return avif;
      default:
        throw StateError('Unknown native image format value: $v');
    }
  }

  String get name {
    switch (this) {
      case jpeg:
        return 'JPEG';
      case png:
        return 'PNG';
      case webp:
        return 'WebP';
      case avif:
        return 'AVIF';
    }
  }
}

/// Quick image metadata read from file headers — no pixel decoding.
class ImageProbe {
  const ImageProbe({
    required this.width,
    required this.height,
    required this.format,
    required this.fileSize,
    required this.hasExif,
  });

  /// Image width in pixels.
  final int width;

  /// Image height in pixels.
  final int height;

  /// Detected image format.
  final ImageFormat format;

  /// File size in bytes.
  final int fileSize;

  /// Whether EXIF metadata is present.
  final bool hasExif;

  /// Total pixel count.
  int get pixelCount => width * height;

  /// Megapixels (e.g., 12.0 for 4000x3000).
  double get megapixels => pixelCount / 1000000.0;

  @override
  String toString() =>
      'ImageProbe(${width}x$height, ${format.name}, '
      '${formatBytes(fileSize)}, '
      '${megapixels.toStringAsFixed(1)}MP'
      '${hasExif ? ', EXIF' : ''})';
}

// ─── Benchmark Result ────────────────────────────────────────────────────────

/// Single quality point from a benchmark sweep.
class BenchmarkEntry {
  const BenchmarkEntry({
    required this.quality,
    required this.sizeBytes,
    required this.ratio,
    required this.encodeMs,
  });

  /// Quality level (0-100). 0 for PNG/WebP.
  final int quality;

  /// Compressed output size in bytes.
  final int sizeBytes;

  /// Compression ratio (e.g., 0.35 = 65% reduction).
  final double ratio;

  /// Encoding time in milliseconds.
  final int encodeMs;

  /// Human-readable size.
  String get sizeFormatted => formatBytes(sizeBytes);

  /// Human-readable reduction.
  String get reductionPercent => '${((1.0 - ratio) * 100).toStringAsFixed(1)}%';

  @override
  String toString() =>
      'q$quality: $sizeFormatted ($reductionPercent, ${encodeMs}ms)';
}

/// Full benchmark result — quality sweep across multiple levels.
class BenchmarkResult {
  const BenchmarkResult({
    required this.originalSize,
    required this.width,
    required this.height,
    required this.format,
    required this.entries,
    required this.recommendedQuality,
  });

  /// Original file size in bytes.
  final int originalSize;

  /// Image width.
  final int width;

  /// Image height.
  final int height;

  /// Detected format.
  final ImageFormat format;

  /// Quality sweep entries, highest quality first.
  final List<BenchmarkEntry> entries;

  /// Recommended quality — best size/quality trade-off.
  final int recommendedQuality;

  @override
  String toString() {
    final buf = StringBuffer();
    buf.writeln(
      'Benchmark(${formatBytes(originalSize)}, ${width}x$height, ${format.name}, '
      'recommended: q$recommendedQuality)',
    );
    for (final entry in entries) {
      final marker =
          entry.quality == recommendedQuality ? '  ← recommended' : '';
      buf.writeln('  $entry$marker');
    }
    return buf.toString().trimRight();
  }
}
