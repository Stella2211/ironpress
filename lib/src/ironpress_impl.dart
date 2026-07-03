import 'dart:async';
import 'dart:ffi';
import 'dart:isolate';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import 'bindings.dart';
import 'native_loader.dart';
import 'options.dart';

/// Expected ABI version — must match the Rust constant.
/// Increment both sides when any #[repr(C)] struct layout changes.
/// v2: CompressParams grew pngLossy, autoOrient, preserveIcc, avifSpeed.
const int _expectedAbiVersion = 2;

/// Error code used when the batch progress isolate crashes unexpectedly.
const int _isolateCrashCode = -100;

/// Maximum value representable by native `u32` fields.
const int _maxUint32 = 0xFFFFFFFF;

const String _cancelBatchMessage = 'cancel';

/// Validates quality parameters before passing them to the native engine.
///
/// Throws [ArgumentError] if [quality] or [minQuality] are outside 0–100.
/// Callers pass the explicitly-provided values (nullable) so that only
/// values the user actually supplied are validated; preset and default
/// values are already guaranteed to be in range.
void _validateQuality({int? quality, int? minQuality}) {
  if (quality != null && (quality < 0 || quality > 100)) {
    throw ArgumentError.value(quality, 'quality', 'must be in the range 0–100');
  }
  if (minQuality != null && (minQuality < 0 || minQuality > 100)) {
    throw ArgumentError.value(
      minQuality,
      'minQuality',
      'must be in the range 0–100',
    );
  }
}

void _validateOptionalPositiveUint32(int? value, String name) {
  if (value == null) {
    return;
  }
  if (value <= 0) {
    throw ArgumentError.value(value, name, 'must be greater than 0');
  }
  if (value > _maxUint32) {
    throw ArgumentError.value(
      value,
      name,
      'must be less than or equal to $_maxUint32',
    );
  }
}

void _validateNonNegativeUint32(int value, String name) {
  if (value < 0) {
    throw ArgumentError.value(
      value,
      name,
      'must be greater than or equal to 0',
    );
  }
  if (value > _maxUint32) {
    throw ArgumentError.value(
      value,
      name,
      'must be less than or equal to $_maxUint32',
    );
  }
}

void _validatePositiveUint32(int value, String name) {
  if (value <= 0) {
    throw ArgumentError.value(value, name, 'must be greater than 0');
  }
  if (value > _maxUint32) {
    throw ArgumentError.value(
      value,
      name,
      'must be less than or equal to $_maxUint32',
    );
  }
}

void _validateResizeAndSizeParams({
  int? maxWidth,
  int? maxHeight,
  int? maxFileSize,
}) {
  _validateOptionalPositiveUint32(maxWidth, 'maxWidth');
  _validateOptionalPositiveUint32(maxHeight, 'maxHeight');
  _validateOptionalPositiveUint32(maxFileSize, 'maxFileSize');
}

void _validatePngOptions(PngOptions png) {
  final optimizationLevel = png.optimizationLevel;
  if (optimizationLevel < 0 || optimizationLevel > 6) {
    throw ArgumentError.value(
      optimizationLevel,
      'png.optimizationLevel',
      'must be in the range 0-6',
    );
  }
}

void _validateAvifOptions(AvifOptions avif) {
  final speed = avif.speed;
  if (speed < 1 || speed > 10) {
    throw ArgumentError.value(speed, 'avif.speed', 'must be in the range 1-10');
  }
}

void _validateCompressionArgs({
  int? quality,
  int? maxWidth,
  int? maxHeight,
  int? maxFileSize,
  int? minQuality,
  required PngOptions png,
  required AvifOptions avif,
}) {
  _validateQuality(quality: quality, minQuality: minQuality);
  _validateResizeAndSizeParams(
    maxWidth: maxWidth,
    maxHeight: maxHeight,
    maxFileSize: maxFileSize,
  );
  _validatePngOptions(png);
  _validateAvifOptions(avif);
}

void _validateBatchArgs({required int threadCount, required int chunkSize}) {
  _validateNonNegativeUint32(threadCount, 'threadCount');
  _validatePositiveUint32(chunkSize, 'chunkSize');
}

void _validateBenchmarkArgs({int? maxWidth, int? maxHeight}) {
  _validateOptionalPositiveUint32(maxWidth, 'maxWidth');
  _validateOptionalPositiveUint32(maxHeight, 'maxHeight');
}

/// Cached bindings per isolate (each isolate has its own top-level state).
NativeBindings? _cachedBindings;
NativeBindings _getBindings() =>
    _cachedBindings ??= NativeBindings.fromLibrary(loadNativeLibrary());

class _WorkerFailure {
  const _WorkerFailure({required this.message, this.code, this.stackTrace});

  final String message;
  final int? code;
  final String? stackTrace;

  Object toException() {
    if (code != null) {
      return CompressException(code!, message);
    }
    final suffix =
        stackTrace == null || stackTrace!.isEmpty ? '' : '\n$stackTrace';
    return StateError('$message$suffix');
  }

  Never rethrowAsException() {
    throw toException();
  }
}

class _BatchIsolateRequest {
  const _BatchIsolateRequest({
    required this.batchInputs,
    required this.eventPort,
    required this.quality,
    required this.maxWidth,
    required this.maxHeight,
    required this.maxFileSize,
    required this.minQuality,
    required this.allowResize,
    required this.format,
    required this.keepMetadata,
    required this.keepIccProfile,
    required this.autoOrient,
    required this.jpeg,
    required this.png,
    required this.avif,
    required this.threadCount,
    required this.chunkSize,
    required this.reportProgress,
    required this.enableCancellation,
  });

  final List<_TransferableBatchInputSpec> batchInputs;
  final SendPort eventPort;
  final int quality;
  final int? maxWidth;
  final int? maxHeight;
  final int? maxFileSize;
  final int minQuality;
  final bool allowResize;
  final CompressFormat format;
  final bool keepMetadata;
  final bool keepIccProfile;
  final bool autoOrient;
  final JpegOptions jpeg;
  final PngOptions png;
  final AvifOptions avif;
  final int threadCount;
  final int chunkSize;
  final bool reportProgress;
  final bool enableCancellation;
}

Future<void> _batchWorkerMain(_BatchIsolateRequest request) async {
  try {
    final result = await Ironpress._compressBatchTransferAsync(
      request.batchInputs
          .map((input) => input.materialize())
          .toList(growable: false),
      quality: request.quality,
      maxWidth: request.maxWidth,
      maxHeight: request.maxHeight,
      maxFileSize: request.maxFileSize,
      minQuality: request.minQuality,
      allowResize: request.allowResize,
      format: request.format,
      keepMetadata: request.keepMetadata,
      keepIccProfile: request.keepIccProfile,
      autoOrient: request.autoOrient,
      jpeg: request.jpeg,
      png: request.png,
      avif: request.avif,
      threadCount: request.threadCount,
      chunkSize: request.chunkSize,
      progressSendPort: request.reportProgress ? request.eventPort : null,
      controlInitPort: request.enableCancellation ? request.eventPort : null,
    );
    request.eventPort.send(result);
  } on CompressException catch (error, stackTrace) {
    request.eventPort.send(
      _WorkerFailure(
        message: error.message,
        code: error.code,
        stackTrace: stackTrace.toString(),
      ),
    );
  } catch (error, stackTrace) {
    request.eventPort.send(
      _WorkerFailure(
        message: error.toString(),
        stackTrace: stackTrace.toString(),
      ),
    );
  }
}

class _TransferableBatchInputSpec {
  const _TransferableBatchInputSpec({this.path, this.data, this.outputPath});

  final String? path;
  final TransferableTypedData? data;
  final String? outputPath;

  _BatchInputSpec materialize() {
    return _BatchInputSpec(
      path: path,
      data: data?.materialize().asUint8List(),
      outputPath: outputPath,
    );
  }
}

class _TransferableCompressResult {
  const _TransferableCompressResult({
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

  final TransferableTypedData? data;
  final int originalSize;
  final int compressedSize;
  final int width;
  final int height;
  final int qualityUsed;
  final int iterations;
  final bool resizedToFit;
  final int? errorCode;
  final String? errorMessage;

  CompressResult materialize() {
    return CompressResult(
      data: data?.materialize().asUint8List(),
      originalSize: originalSize,
      compressedSize: compressedSize,
      width: width,
      height: height,
      qualityUsed: qualityUsed,
      iterations: iterations,
      resizedToFit: resizedToFit,
      errorCode: errorCode,
      errorMessage: errorMessage,
    );
  }
}

class _TransferableBatchCompressResult {
  const _TransferableBatchCompressResult({
    required this.results,
    required this.elapsedMs,
  });

  final List<_TransferableCompressResult> results;
  final int elapsedMs;

  BatchCompressResult materialize() {
    return BatchCompressResult(
      results: results.map((result) => result.materialize()).toList(),
      elapsedMs: elapsedMs,
    );
  }
}

/// High-performance image compression powered by Rust.
///
/// Uses mozjpeg (trellis quantization) for JPEG and oxipng for PNG —
/// delivering consistent, high-quality results across all platforms.
///
/// All operations run on a background isolate to keep the UI responsive.
///
/// ```dart
/// // Simple usage
/// final result = await Ironpress.compressFile(
///   'photo.jpg',
///   quality: 80,
/// );
///
/// // Target file size
/// final result = await Ironpress.compressFile(
///   'photo.jpg',
///   maxFileSize: 200 * 1024, // 200 KB
/// );
/// ```
class Ironpress {
  // Prevent instantiation
  Ironpress._();

  static NativeBindings? _bindings;

  /// Lazily initialize native bindings with ABI version check.
  static NativeBindings get _b {
    if (_bindings == null) {
      _bindings = _getBindings();
      final version = _bindings!.abiVersion();
      if (version != _expectedAbiVersion) {
        _bindings = null;
        throw StateError(
          'Ironpress ABI version mismatch: native=$version, '
          'expected=$_expectedAbiVersion. Rebuild the native library.',
        );
      }
    }
    return _bindings!;
  }

  // ─── Public API ──────────────────────────────────────────────────────

  /// Compress an image file and return the result as bytes.
  ///
  /// [path] — Absolute path to the input image (JPEG, PNG, WebP, GIF, BMP,
  /// or TIFF).
  ///
  /// [preset] — Optional quality preset ([CompressPreset.low],
  /// [CompressPreset.medium], [CompressPreset.high]). Explicit parameters
  /// always take priority over the preset.
  ///
  /// [quality] — JPEG quality 0-100. Default: 80. Ignored for PNG output.
  ///
  /// [maxWidth] / [maxHeight] — Resize constraints. Image is scaled down
  /// to fit within these bounds while preserving aspect ratio.
  ///
  /// [maxFileSize] — Target maximum output size in bytes. When set,
  /// the engine performs a binary search to find the highest quality
  /// that fits. This is the killer feature — one FFI call, no round-trips.
  ///
  /// [minQuality] — Quality floor for the binary search (default: 30).
  ///
  /// [allowResize] — If quality alone can't hit [maxFileSize], allow
  /// automatic downscaling. Default: `true`.
  ///
  /// [format] — Output format. Default: [CompressFormat.auto] (same as input).
  ///
  /// [keepMetadata] — Preserve JPEG EXIF metadata when compressing JPEG to
  /// JPEG. Other paths silently drop metadata.
  ///
  /// [keepIccProfile] — Preserve the input's ICC color profile in JPEG and
  /// PNG output (from JPEG, PNG, or WebP input). Prevents color shifts on
  /// wide-gamut (Display P3) photos. Default: `true`.
  ///
  /// [autoOrient] — Physically rotate pixels to match the EXIF orientation
  /// tag, so photos shot in portrait never come out sideways. When combined
  /// with [keepMetadata], the preserved EXIF orientation tag is reset to
  /// upright. Default: `true`.
  ///
  /// [jpeg] — Advanced JPEG options (progressive, trellis, chroma).
  ///
  /// [png] — Advanced PNG options (optimization level, lossy quantization).
  ///
  /// [avif] — Advanced AVIF options (encoder speed). Only used when [format]
  /// is [CompressFormat.avif].
  ///
  /// Throws [ArgumentError] if [path] is empty.
  /// Throws [CompressException] if the native engine reports an error.
  ///
  /// ```dart
  /// // One-liner with a preset
  /// final result = await Ironpress.compressFile(
  ///   '/path/to/photo.jpg',
  ///   preset: CompressPreset.medium,
  /// );
  /// print(result); // CompressResult(4.2 MB → 380 KB [91%], 1920x1440, q80)
  ///
  /// // Target file size — binary search in Rust, single FFI call
  /// final result = await Ironpress.compressFile(
  ///   '/path/to/photo.jpg',
  ///   maxFileSize: 200 * 1024, // 200 KB
  ///   maxWidth: 1920,
  /// );
  /// ```
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
  }) async {
    if (path.isEmpty) {
      throw ArgumentError.value(path, 'path', 'must not be empty');
    }
    _validateCompressionArgs(
      quality: quality,
      maxWidth: maxWidth,
      maxHeight: maxHeight,
      maxFileSize: maxFileSize,
      minQuality: minQuality,
      png: png,
      avif: avif,
    );
    final effectiveQuality = quality ?? preset?.quality ?? 80;
    final effectiveMaxWidth = maxWidth ?? preset?.maxWidth;
    final effectiveMaxHeight = maxHeight ?? preset?.maxHeight;
    final effectiveMinQuality = minQuality ?? preset?.minQuality ?? 30;
    final result = await Isolate.run(() {
      return _compressFileTransferSync(
        path,
        quality: effectiveQuality,
        maxWidth: effectiveMaxWidth,
        maxHeight: effectiveMaxHeight,
        maxFileSize: maxFileSize,
        minQuality: effectiveMinQuality,
        allowResize: allowResize,
        format: format,
        keepMetadata: keepMetadata,
        keepIccProfile: keepIccProfile,
        autoOrient: autoOrient,
        jpeg: jpeg,
        png: png,
        avif: avif,
      );
    });
    return result.materialize();
  }

  /// Compress an image file and write the result to [outputPath].
  ///
  /// Returns a [CompressResult] with stats but `data` will be `null`
  /// since the output was written to disk.
  ///
  /// Accepts the same parameters as [compressFile] including [preset].
  ///
  /// Throws [ArgumentError] if [inputPath] or [outputPath] is empty.
  /// Throws [CompressException] if the native engine reports an error.
  ///
  /// ```dart
  /// final result = await Ironpress.compressFileToFile(
  ///   '/input/photo.jpg',
  ///   '/output/photo_compressed.jpg',
  ///   preset: CompressPreset.medium,
  /// );
  /// assert(result.isFileOutput); // true — no bytes in memory
  /// ```
  static Future<CompressResult> compressFileToFile(
    String inputPath,
    String outputPath, {
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
  }) async {
    if (inputPath.isEmpty) {
      throw ArgumentError.value(inputPath, 'inputPath', 'must not be empty');
    }
    if (outputPath.isEmpty) {
      throw ArgumentError.value(outputPath, 'outputPath', 'must not be empty');
    }
    _validateCompressionArgs(
      quality: quality,
      maxWidth: maxWidth,
      maxHeight: maxHeight,
      maxFileSize: maxFileSize,
      minQuality: minQuality,
      png: png,
      avif: avif,
    );
    final effectiveQuality = quality ?? preset?.quality ?? 80;
    final effectiveMaxWidth = maxWidth ?? preset?.maxWidth;
    final effectiveMaxHeight = maxHeight ?? preset?.maxHeight;
    final effectiveMinQuality = minQuality ?? preset?.minQuality ?? 30;
    final result = await Isolate.run(() {
      return _compressFileToFileTransferSync(
        inputPath,
        outputPath,
        quality: effectiveQuality,
        maxWidth: effectiveMaxWidth,
        maxHeight: effectiveMaxHeight,
        maxFileSize: maxFileSize,
        minQuality: effectiveMinQuality,
        allowResize: allowResize,
        format: format,
        keepMetadata: keepMetadata,
        keepIccProfile: keepIccProfile,
        autoOrient: autoOrient,
        jpeg: jpeg,
        png: png,
        avif: avif,
      );
    });
    return result.materialize();
  }

  /// Compress raw image bytes in memory and return the result as bytes.
  ///
  /// Accepts JPEG, PNG, WebP, GIF, BMP, or TIFF input.
  /// Accepts the same parameters as [compressFile] including [preset].
  ///
  /// Throws [ArgumentError] if [data] is empty.
  /// Throws [CompressException] if the native engine reports an error.
  ///
  /// ```dart
  /// final bytes = await File('photo.jpg').readAsBytes();
  ///
  /// // Compress with a preset
  /// final result = await Ironpress.compressBytes(
  ///   bytes,
  ///   preset: CompressPreset.medium,
  /// );
  ///
  /// // Convert to WebP
  /// final webpResult = await Ironpress.compressBytes(
  ///   bytes,
  ///   quality: 80,
  ///   format: CompressFormat.webpLossy,
  /// );
  /// ```
  static Future<CompressResult> compressBytes(
    Uint8List data, {
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
  }) async {
    if (data.isEmpty) {
      throw ArgumentError.value(data, 'data', 'must not be empty');
    }
    _validateCompressionArgs(
      quality: quality,
      maxWidth: maxWidth,
      maxHeight: maxHeight,
      maxFileSize: maxFileSize,
      minQuality: minQuality,
      png: png,
      avif: avif,
    );
    final effectiveQuality = quality ?? preset?.quality ?? 80;
    final effectiveMaxWidth = maxWidth ?? preset?.maxWidth;
    final effectiveMaxHeight = maxHeight ?? preset?.maxHeight;
    final effectiveMinQuality = minQuality ?? preset?.minQuality ?? 30;
    final transferData = TransferableTypedData.fromList([data]);
    final result = await Isolate.run(() {
      return _compressBytesTransferSync(
        transferData.materialize().asUint8List(),
        quality: effectiveQuality,
        maxWidth: effectiveMaxWidth,
        maxHeight: effectiveMaxHeight,
        maxFileSize: maxFileSize,
        minQuality: effectiveMinQuality,
        allowResize: allowResize,
        format: format,
        keepMetadata: keepMetadata,
        keepIccProfile: keepIccProfile,
        autoOrient: autoOrient,
        jpeg: jpeg,
        png: png,
        avif: avif,
      );
    });
    return result.materialize();
  }

  /// Batch compress multiple images using Rust's rayon thread pool.
  ///
  /// **Performance:** Single FFI call → rayon splits across CPU cores with
  /// zero-copy shared params. For 200 inspection photos, this is 3-5x faster
  /// than sequential Dart isolates.
  ///
  /// **Safety:** Designed to never hang, OOM, or crash your app:
  /// - [chunkSize] limits how many images are decoded simultaneously
  ///   (default 8 ≈ ~288 MB peak for 4K photos). Between chunks, all
  ///   decoded pixel buffers are freed.
  /// - [threadCount] defaults to `cores - 2`, leaving room for Flutter
  ///   UI thread and Dart isolate. Your app stays responsive.
  /// - Each item is panic-safe: a corrupt JPEG or OOM on one image
  ///   produces an error result, not a process crash.
  ///
  /// **Progress:** Pass [onProgress] to get live updates. Called on the
  /// main thread with `(completed, total)` — safe to call `setState` directly.
  ///
  /// **Cancellation:** Pass a [CancellationToken] to cancel between chunks.
  /// Returns partial results for already-completed chunks.
  ///
  /// ```dart
  /// final token = CancellationToken();
  ///
  /// final result = await Ironpress.compressBatch(
  ///   photos.map((p) => CompressInput(path: p)).toList(),
  ///   preset: CompressPreset.medium,
  ///   maxFileSize: 300 * 1024,
  ///   cancellationToken: token,
  ///   onProgress: (done, total) {
  ///     setState(() => _progress = done / total);
  ///   },
  /// );
  /// print(result); // 200 images, 6823ms, 29.3 img/s
  /// ```
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
  }) async {
    _validateCompressionArgs(
      quality: quality,
      maxWidth: maxWidth,
      maxHeight: maxHeight,
      maxFileSize: maxFileSize,
      minQuality: minQuality,
      png: png,
      avif: avif,
    );
    _validateBatchArgs(threadCount: threadCount, chunkSize: chunkSize);
    final effectiveQuality = quality ?? preset?.quality ?? 80;
    final effectiveMaxWidth = maxWidth ?? preset?.maxWidth;
    final effectiveMaxHeight = maxHeight ?? preset?.maxHeight;
    final effectiveMinQuality = minQuality ?? preset?.minQuality ?? 30;
    final total = inputs.length;
    if (total == 0) {
      return const BatchCompressResult(results: [], elapsedMs: 0);
    }
    if (cancellationToken?.isCancelled ?? false) {
      return const BatchCompressResult(results: [], elapsedMs: 0);
    }

    final transferableInputs = inputs
        .map(
          (i) => _BatchInputSpec(
            path: i.path,
            data: i.data,
            outputPath: i.outputPath,
          ),
        )
        .map(
          (input) => _TransferableBatchInputSpec(
            path: input.path,
            data:
                input.data == null
                    ? null
                    : TransferableTypedData.fromList([input.data!]),
            outputPath: input.outputPath,
          ),
        )
        .toList(growable: false);

    if (onProgress != null || cancellationToken != null) {
      return _compressBatchWithEvents(
        transferableInputs,
        total: total,
        quality: effectiveQuality,
        maxWidth: effectiveMaxWidth,
        maxHeight: effectiveMaxHeight,
        maxFileSize: maxFileSize,
        minQuality: effectiveMinQuality,
        allowResize: allowResize,
        format: format,
        keepMetadata: keepMetadata,
        keepIccProfile: keepIccProfile,
        autoOrient: autoOrient,
        jpeg: jpeg,
        png: png,
        avif: avif,
        threadCount: threadCount,
        chunkSize: chunkSize,
        onProgress: onProgress,
        cancellationToken: cancellationToken,
      );
    }

    final result = await Isolate.run(() {
      return _compressBatchTransferAsync(
        transferableInputs
            .map((input) => input.materialize())
            .toList(growable: false),
        quality: effectiveQuality,
        maxWidth: effectiveMaxWidth,
        maxHeight: effectiveMaxHeight,
        maxFileSize: maxFileSize,
        minQuality: effectiveMinQuality,
        allowResize: allowResize,
        format: format,
        keepMetadata: keepMetadata,
        keepIccProfile: keepIccProfile,
        autoOrient: autoOrient,
        jpeg: jpeg,
        png: png,
        avif: avif,
        threadCount: threadCount,
        chunkSize: chunkSize,
      );
    });
    return result.materialize();
  }

  // ─── Probe: Quick Metadata ───────────────────────────────────────────

  /// Read image metadata from a file without decoding pixel data.
  ///
  /// Much faster than compression — useful for validating images before
  /// upload or checking resolution before deciding whether to resize.
  ///
  /// Throws [ArgumentError] if [path] is empty.
  /// Throws [CompressException] if the file cannot be read or parsed.
  ///
  /// ```dart
  /// final info = await Ironpress.probeFile('/path/to/photo.jpg');
  /// print(info); // ImageProbe(4000x3000, JPEG, 4.2 MB, 12.0MP, EXIF)
  ///
  /// if (info.megapixels > 12) {
  ///   // Only compress if large enough to benefit
  ///   await Ironpress.compressFile(path, preset: CompressPreset.medium);
  /// }
  /// ```
  // Internal: batch work with progress/cancellation needs one ordered event
  // channel so progress and the terminal result cannot race each other.
  static Future<BatchCompressResult> _compressBatchWithEvents(
    List<_TransferableBatchInputSpec> batchInputs, {
    required int total,
    required int quality,
    int? maxWidth,
    int? maxHeight,
    int? maxFileSize,
    required int minQuality,
    required bool allowResize,
    required CompressFormat format,
    required bool keepMetadata,
    required bool keepIccProfile,
    required bool autoOrient,
    required JpegOptions jpeg,
    required PngOptions png,
    required AvifOptions avif,
    required int threadCount,
    required int chunkSize,
    void Function(int completed, int total)? onProgress,
    CancellationToken? cancellationToken,
  }) async {
    final eventPort = ReceivePort();
    final errorPort = ReceivePort();
    final resultCompleter = Completer<BatchCompressResult>();
    StreamSubscription? eventSubscription;
    StreamSubscription? errorSubscription;
    SendPort? cancelSendPort;
    var cancelSignalSent = false;
    var lastCompleted = 0;

    void completeError(Object error, [StackTrace? stackTrace]) {
      if (!resultCompleter.isCompleted) {
        resultCompleter.completeError(error, stackTrace);
      }
    }

    void sendCancelSignal() {
      if (cancelSignalSent || cancelSendPort == null) {
        return;
      }
      cancelSignalSent = true;
      cancelSendPort!.send(_cancelBatchMessage);
    }

    final removeCancelListener = cancellationToken?.addListener(
      sendCancelSignal,
    );

    try {
      eventSubscription = eventPort.listen((message) {
        if (message is SendPort) {
          cancelSendPort = message;
          if (cancellationToken?.isCancelled ?? false) {
            sendCancelSignal();
          }
          return;
        }

        if (message is int) {
          lastCompleted = message;
          onProgress?.call(message, total);
          return;
        }

        if (message is _TransferableBatchCompressResult) {
          final result = message.materialize();
          final finalCompleted = result.results.length;
          if (onProgress != null && lastCompleted != finalCompleted) {
            lastCompleted = finalCompleted;
            onProgress(finalCompleted, total);
          }
          if (!resultCompleter.isCompleted) {
            resultCompleter.complete(result);
          }
          return;
        }

        if (message is _WorkerFailure) {
          completeError(message.toException());
        }
      });

      errorSubscription = errorPort.listen((message) {
        final details =
            message is List && message.isNotEmpty
                ? message.first.toString()
                : 'unknown isolate error';
        completeError(
          CompressException(
            _isolateCrashCode,
            'Batch compression isolate crashed: $details',
          ),
        );
      });

      await Isolate.spawn<_BatchIsolateRequest>(
        _batchWorkerMain,
        _BatchIsolateRequest(
          batchInputs: batchInputs,
          eventPort: eventPort.sendPort,
          quality: quality,
          maxWidth: maxWidth,
          maxHeight: maxHeight,
          maxFileSize: maxFileSize,
          minQuality: minQuality,
          allowResize: allowResize,
          format: format,
          keepMetadata: keepMetadata,
          keepIccProfile: keepIccProfile,
          autoOrient: autoOrient,
          jpeg: jpeg,
          png: png,
          avif: avif,
          threadCount: threadCount,
          chunkSize: chunkSize,
          reportProgress: onProgress != null,
          enableCancellation: cancellationToken != null,
        ),
        onError: errorPort.sendPort,
        errorsAreFatal: true,
      );

      return await resultCompleter.future;
    } finally {
      removeCancelListener?.call();
      await eventSubscription?.cancel();
      await errorSubscription?.cancel();
      eventPort.close();
      errorPort.close();
    }
  }

  /// Read image metadata from a file without decoding pixel data.
  ///
  /// Much faster than compression â€” useful for validating images before
  /// upload or checking resolution before deciding whether to resize.
  ///
  /// Throws [ArgumentError] if [path] is empty.
  /// Throws [CompressException] if the file cannot be read or parsed.
  ///
  /// ```dart
  /// final info = await Ironpress.probeFile('/path/to/photo.jpg');
  /// print(info); // ImageProbe(4000x3000, JPEG, 4.2 MB, 12.0MP, EXIF)
  ///
  /// if (info.megapixels > 12) {
  ///   // Only compress if large enough to benefit
  ///   await Ironpress.compressFile(path, preset: CompressPreset.medium);
  /// }
  /// ```
  static Future<ImageProbe> probeFile(String path) async {
    if (path.isEmpty) {
      throw ArgumentError.value(path, 'path', 'must not be empty');
    }
    return Isolate.run(() => _probeFileSync(path));
  }

  /// Deprecated. Use [probeFile] instead.
  @Deprecated('Use probeFile instead')
  static Future<ImageProbe> probe(String path) => probeFile(path);

  /// Read image metadata from bytes without decoding pixel data.
  ///
  /// Throws [ArgumentError] if [data] is empty.
  /// Throws [CompressException] if the data cannot be parsed.
  static Future<ImageProbe> probeBytes(Uint8List data) async {
    if (data.isEmpty) {
      throw ArgumentError.value(data, 'data', 'must not be empty');
    }
    final transferData = TransferableTypedData.fromList([data]);
    return Isolate.run(
      () => _probeBytesSync(transferData.materialize().asUint8List()),
    );
  }

  // ─── Benchmark: Quality Sweep ────────────────────────────────────────

  /// Run a quality sweep on a file: encode at multiple quality levels and
  /// measure output size and speed for each level.
  ///
  /// Useful for picking the right quality for your use case before committing
  /// to a value. The result includes a `recommendedQuality` field with the
  /// best size/quality trade-off.
  ///
  /// Throws [ArgumentError] if [path] is empty.
  /// Throws [CompressException] if the file cannot be read or parsed.
  ///
  /// ```dart
  /// final bench = await Ironpress.benchmarkFile('/path/to/photo.jpg');
  /// print(bench);
  /// // Benchmark(4.2 MB, 4000x3000, JPEG, recommended: q78)
  /// //   q95: 1.2 MB (70.5%, 210ms)
  /// //   q85: 620 KB (85.2%, 180ms)
  /// //   q78: 410 KB (90.2%, 165ms)  ← recommended
  /// //   q65: 280 KB (93.3%, 155ms)
  /// //   ...
  /// ```
  static Future<BenchmarkResult> benchmarkFile(
    String path, {
    int? maxWidth,
    int? maxHeight,
  }) async {
    if (path.isEmpty) {
      throw ArgumentError.value(path, 'path', 'must not be empty');
    }
    _validateBenchmarkArgs(maxWidth: maxWidth, maxHeight: maxHeight);
    return Isolate.run(
      () => _benchmarkFileSync(path, maxWidth: maxWidth, maxHeight: maxHeight),
    );
  }

  /// Deprecated. Use [benchmarkFile] instead.
  @Deprecated('Use benchmarkFile instead')
  static Future<BenchmarkResult> benchmark(
    String path, {
    int? maxWidth,
    int? maxHeight,
  }) => benchmarkFile(path, maxWidth: maxWidth, maxHeight: maxHeight);

  /// Run a quality sweep on raw image bytes.
  ///
  /// Throws [ArgumentError] if [data] is empty.
  static Future<BenchmarkResult> benchmarkBytes(
    Uint8List data, {
    int? maxWidth,
    int? maxHeight,
  }) async {
    if (data.isEmpty) {
      throw ArgumentError.value(data, 'data', 'must not be empty');
    }
    _validateBenchmarkArgs(maxWidth: maxWidth, maxHeight: maxHeight);
    final transferData = TransferableTypedData.fromList([data]);
    return Isolate.run(
      () => _benchmarkBytesSync(
        transferData.materialize().asUint8List(),
        maxWidth: maxWidth,
        maxHeight: maxHeight,
      ),
    );
  }

  /// Return the native library version string.
  static String get nativeVersion {
    final ptr = _b.version();
    return ptr.toDartString();
  }

  // ─── Internal: Sync FFI calls (run on isolates) ──────────────────────

  static Pointer<Uint8> _copyBytesToNative(Uint8List data) {
    final nativeData = malloc<Uint8>(data.length);
    nativeData.asTypedList(data.length).setAll(0, data);
    return nativeData;
  }

  static _TransferableCompressResult _compressFileTransferSync(
    String path, {
    required int quality,
    int? maxWidth,
    int? maxHeight,
    int? maxFileSize,
    required int minQuality,
    required bool allowResize,
    required CompressFormat format,
    required bool keepMetadata,
    required bool keepIccProfile,
    required bool autoOrient,
    required JpegOptions jpeg,
    required PngOptions png,
    required AvifOptions avif,
  }) {
    final bindings = Ironpress._b;
    final pathPtr = path.toNativeUtf8();
    final paramsPtr = _buildParams(
      quality: quality,
      maxWidth: maxWidth,
      maxHeight: maxHeight,
      maxFileSize: maxFileSize,
      minQuality: minQuality,
      allowResize: allowResize,
      format: format,
      keepMetadata: keepMetadata,
      keepIccProfile: keepIccProfile,
      autoOrient: autoOrient,
      jpeg: jpeg,
      png: png,
      avif: avif,
    );
    final outPtr = malloc<NativeCompressResult>();

    try {
      bindings.compressFile(pathPtr, paramsPtr, outPtr);
      return _convertResultTransfer(outPtr, bindings);
    } finally {
      calloc.free(pathPtr);
      calloc.free(paramsPtr);
    }
  }

  static _TransferableCompressResult _compressFileToFileTransferSync(
    String inputPath,
    String outputPath, {
    required int quality,
    int? maxWidth,
    int? maxHeight,
    int? maxFileSize,
    required int minQuality,
    required bool allowResize,
    required CompressFormat format,
    required bool keepMetadata,
    required bool keepIccProfile,
    required bool autoOrient,
    required JpegOptions jpeg,
    required PngOptions png,
    required AvifOptions avif,
  }) {
    final bindings = Ironpress._b;
    final inputPtr = inputPath.toNativeUtf8();
    final outputPtr = outputPath.toNativeUtf8();
    final paramsPtr = _buildParams(
      quality: quality,
      maxWidth: maxWidth,
      maxHeight: maxHeight,
      maxFileSize: maxFileSize,
      minQuality: minQuality,
      allowResize: allowResize,
      format: format,
      keepMetadata: keepMetadata,
      keepIccProfile: keepIccProfile,
      autoOrient: autoOrient,
      jpeg: jpeg,
      png: png,
      avif: avif,
    );
    final outPtr = malloc<NativeCompressResult>();

    try {
      bindings.compressFileToFile(inputPtr, outputPtr, paramsPtr, outPtr);
      return _convertResultTransfer(outPtr, bindings);
    } finally {
      calloc.free(inputPtr);
      calloc.free(outputPtr);
      calloc.free(paramsPtr);
    }
  }

  static _TransferableCompressResult _compressBytesTransferSync(
    Uint8List data, {
    required int quality,
    int? maxWidth,
    int? maxHeight,
    int? maxFileSize,
    required int minQuality,
    required bool allowResize,
    required CompressFormat format,
    required bool keepMetadata,
    required bool keepIccProfile,
    required bool autoOrient,
    required JpegOptions jpeg,
    required PngOptions png,
    required AvifOptions avif,
  }) {
    final bindings = Ironpress._b;
    final nativeData = _copyBytesToNative(data);
    final paramsPtr = _buildParams(
      quality: quality,
      maxWidth: maxWidth,
      maxHeight: maxHeight,
      maxFileSize: maxFileSize,
      minQuality: minQuality,
      allowResize: allowResize,
      format: format,
      keepMetadata: keepMetadata,
      keepIccProfile: keepIccProfile,
      autoOrient: autoOrient,
      jpeg: jpeg,
      png: png,
      avif: avif,
    );
    final outPtr = malloc<NativeCompressResult>();

    try {
      bindings.compressBuffer(nativeData, data.length, paramsPtr, outPtr);
      return _convertResultTransfer(outPtr, bindings);
    } finally {
      malloc.free(nativeData);
      calloc.free(paramsPtr);
    }
  }

  static ImageProbe _probeFileSync(String path) {
    final bindings = Ironpress._b;
    final pathPtr = path.toNativeUtf8();
    final outPtr = malloc<NativeProbeResult>();

    try {
      bindings.probeFile(pathPtr, outPtr);
      return _convertProbeResult(outPtr, bindings);
    } finally {
      calloc.free(pathPtr);
    }
  }

  static ImageProbe _probeBytesSync(Uint8List data) {
    final bindings = Ironpress._b;
    final nativeData = _copyBytesToNative(data);
    final outPtr = malloc<NativeProbeResult>();

    try {
      bindings.probeBuffer(nativeData, data.length, outPtr);
      return _convertProbeResult(outPtr, bindings);
    } finally {
      malloc.free(nativeData);
    }
  }

  // ─── Benchmark sync helpers ────────────────────────────────────────────

  static BenchmarkResult _benchmarkFileSync(
    String path, {
    int? maxWidth,
    int? maxHeight,
  }) {
    final bindings = Ironpress._b;
    final pathPtr = path.toNativeUtf8();
    final paramsPtr = _buildParams(
      quality: 80,
      maxWidth: maxWidth,
      maxHeight: maxHeight,
      format: CompressFormat.auto,
      keepMetadata: false,
      keepIccProfile: true,
      autoOrient: true,
      jpeg: const JpegOptions(),
      png: const PngOptions(),
      avif: const AvifOptions(),
      minQuality: 30,
      allowResize: true,
    );
    final outPtr = malloc<NativeBenchmarkResult>();

    try {
      bindings.benchmarkFile(pathPtr, paramsPtr, outPtr);
      return _convertBenchmarkResult(outPtr, bindings);
    } finally {
      calloc.free(pathPtr);
      calloc.free(paramsPtr);
    }
  }

  static BenchmarkResult _benchmarkBytesSync(
    Uint8List data, {
    int? maxWidth,
    int? maxHeight,
  }) {
    final bindings = Ironpress._b;
    final nativeData = _copyBytesToNative(data);
    final paramsPtr = _buildParams(
      quality: 80,
      maxWidth: maxWidth,
      maxHeight: maxHeight,
      format: CompressFormat.auto,
      keepMetadata: false,
      keepIccProfile: true,
      autoOrient: true,
      jpeg: const JpegOptions(),
      png: const PngOptions(),
      avif: const AvifOptions(),
      minQuality: 30,
      allowResize: true,
    );
    final outPtr = malloc<NativeBenchmarkResult>();

    try {
      bindings.benchmarkBuffer(nativeData, data.length, paramsPtr, outPtr);
      return _convertBenchmarkResult(outPtr, bindings);
    } finally {
      malloc.free(nativeData);
      calloc.free(paramsPtr);
    }
  }

  // ─── Helpers ─────────────────────────────────────────────────────────

  static Pointer<NativeCompressParams> _buildParams({
    required int quality,
    int? maxWidth,
    int? maxHeight,
    int? maxFileSize,
    required int minQuality,
    required bool allowResize,
    required CompressFormat format,
    required bool keepMetadata,
    required bool keepIccProfile,
    required bool autoOrient,
    required JpegOptions jpeg,
    required PngOptions png,
    required AvifOptions avif,
  }) {
    final p = calloc<NativeCompressParams>();
    p.ref.quality = quality.clamp(0, 100);
    p.ref.maxWidth = maxWidth ?? 0;
    p.ref.maxHeight = maxHeight ?? 0;
    p.ref.maxFileSize = maxFileSize ?? 0;
    p.ref.minQuality = minQuality.clamp(0, 100);
    p.ref.allowResize = allowResize ? 1 : 0;
    p.ref.format = format.value;
    p.ref.keepMetadata = keepMetadata ? 1 : 0;
    p.ref.jpegProgressive = jpeg.progressive ? 1 : 0;
    p.ref.jpegChromaSubsampling = jpeg.chromaSubsampling.value;
    p.ref.jpegTrellis = jpeg.trellis ? 1 : 0;
    p.ref.pngOptimizationLevel = png.optimizationLevel.clamp(0, 6);
    p.ref.pngLossy = png.lossy ? 1 : 0;
    p.ref.autoOrient = autoOrient ? 1 : 0;
    p.ref.preserveIcc = keepIccProfile ? 1 : 0;
    p.ref.avifSpeed = avif.speed.clamp(1, 10);
    return p;
  }

  static _TransferableCompressResult _convertResultTransfer(
    Pointer<NativeCompressResult> outPtr,
    NativeBindings bindings,
  ) {
    try {
      final ref = outPtr.ref;

      if (ref.errorCode != 0) {
        final msg =
            ref.errorMessage != nullptr
                ? ref.errorMessage.toDartString()
                : 'Unknown error (code ${ref.errorCode})';
        throw CompressException(ref.errorCode, msg);
      }

      TransferableTypedData? transferData;
      if (ref.data != nullptr && ref.dataLen > 0) {
        transferData = TransferableTypedData.fromList([
          ref.data.cast<Uint8>().asTypedList(ref.dataLen),
        ]);
      }

      return _TransferableCompressResult(
        data: transferData,
        originalSize: ref.originalSize,
        compressedSize: ref.dataLen,
        width: ref.width,
        height: ref.height,
        qualityUsed: ref.qualityUsed,
        iterations: ref.iterations,
        resizedToFit: ref.resizedToFit != 0,
      );
    } finally {
      bindings.freeCompressResult(outPtr);
      malloc.free(outPtr);
    }
  }

  static ImageProbe _convertProbeResult(
    Pointer<NativeProbeResult> outPtr,
    NativeBindings bindings,
  ) {
    try {
      final ref = outPtr.ref;

      if (ref.errorCode != 0) {
        final msg =
            ref.errorMessage != nullptr
                ? ref.errorMessage.toDartString()
                : 'Probe failed (code ${ref.errorCode})';
        throw CompressException(ref.errorCode, msg);
      }

      return ImageProbe(
        width: ref.width,
        height: ref.height,
        format: ImageFormat.fromValue(ref.format),
        fileSize: ref.fileSize,
        hasExif: ref.hasExif != 0,
      );
    } finally {
      bindings.freeProbeResult(outPtr);
      malloc.free(outPtr);
    }
  }

  /// Convert a native BenchmarkResult to Dart. Frees native memory.
  static BenchmarkResult _convertBenchmarkResult(
    Pointer<NativeBenchmarkResult> outPtr,
    NativeBindings bindings,
  ) {
    try {
      final ref = outPtr.ref;

      if (ref.errorCode != 0) {
        final msg =
            ref.errorMessage != nullptr
                ? ref.errorMessage.toDartString()
                : 'Benchmark failed (code ${ref.errorCode})';
        throw CompressException(ref.errorCode, msg);
      }

      final entries = <BenchmarkEntry>[];
      for (var i = 0; i < ref.entryCount; i++) {
        final e = ref.entries[i];
        entries.add(
          BenchmarkEntry(
            quality: e.quality,
            sizeBytes: e.sizeBytes,
            ratio: e.ratio,
            encodeMs: e.encodeMs,
          ),
        );
      }

      return BenchmarkResult(
        originalSize: ref.originalSize,
        width: ref.width,
        height: ref.height,
        format: ImageFormat.fromValue(ref.format),
        entries: entries,
        recommendedQuality: ref.recommendedQuality,
      );
    } finally {
      bindings.freeBenchmarkResult(outPtr);
      malloc.free(outPtr);
    }
  }

  static Future<_TransferableBatchCompressResult> _compressBatchTransferAsync(
    List<_BatchInputSpec> inputs, {
    required int quality,
    int? maxWidth,
    int? maxHeight,
    int? maxFileSize,
    required int minQuality,
    required bool allowResize,
    required CompressFormat format,
    required bool keepMetadata,
    required bool keepIccProfile,
    required bool autoOrient,
    required JpegOptions jpeg,
    required PngOptions png,
    required AvifOptions avif,
    required int threadCount,
    required int chunkSize,
    SendPort? progressSendPort,
    SendPort? controlInitPort,
  }) async {
    final bindings = Ironpress._b;
    final stopwatch = Stopwatch()..start();
    final paramsPtr = _buildParams(
      quality: quality,
      maxWidth: maxWidth,
      maxHeight: maxHeight,
      maxFileSize: maxFileSize,
      minQuality: minQuality,
      allowResize: allowResize,
      format: format,
      keepMetadata: keepMetadata,
      keepIccProfile: keepIccProfile,
      autoOrient: autoOrient,
      jpeg: jpeg,
      png: png,
      avif: avif,
    );

    final allResults = <_TransferableCompressResult>[];
    ReceivePort? controlPort;
    StreamSubscription? controlSubscription;
    var cancelled = false;
    var completed = 0;

    if (controlInitPort != null) {
      controlPort = ReceivePort();
      controlSubscription = controlPort.listen((message) {
        if (message == _cancelBatchMessage) {
          cancelled = true;
        }
      });
      controlInitPort.send(controlPort.sendPort);
    }

    try {
      for (var start = 0; start < inputs.length; start += chunkSize) {
        if (cancelled) {
          break;
        }

        final end = (start + chunkSize).clamp(0, inputs.length);
        final chunkInputs = inputs.sublist(start, end);
        allResults.addAll(
          _compressBatchChunkTransferSync(
            chunkInputs,
            bindings,
            paramsPtr,
            threadCount: threadCount,
            chunkSize: chunkSize,
          ),
        );

        completed += chunkInputs.length;
        progressSendPort?.send(completed);

        if (end < inputs.length &&
            (progressSendPort != null || controlInitPort != null)) {
          await Future<void>.delayed(Duration.zero);
        }
      }
    } finally {
      await controlSubscription?.cancel();
      controlPort?.close();
      calloc.free(paramsPtr);
    }

    stopwatch.stop();
    return _TransferableBatchCompressResult(
      results: allResults,
      elapsedMs: stopwatch.elapsedMilliseconds,
    );
  }

  static List<_TransferableCompressResult> _compressBatchChunkTransferSync(
    List<_BatchInputSpec> chunkInputs,
    NativeBindings bindings,
    Pointer<NativeCompressParams> paramsPtr, {
    required int threadCount,
    required int chunkSize,
  }) {
    final nativeInputs = calloc<NativeBatchInput>(chunkInputs.length);
    final disposers = <void Function()>[];

    try {
      for (var i = 0; i < chunkInputs.length; i++) {
        final spec = chunkInputs[i];
        final ref = nativeInputs[i];

        if (spec.path != null) {
          final pathPtr = spec.path!.toNativeUtf8();
          disposers.add(() => calloc.free(pathPtr));
          ref.filePath = pathPtr;
          ref.data = nullptr;
          ref.dataLen = 0;
        } else if (spec.data != null) {
          final dataPtr = _copyBytesToNative(spec.data!);
          disposers.add(() => malloc.free(dataPtr));
          ref.filePath = nullptr;
          ref.data = dataPtr;
          ref.dataLen = spec.data!.length;
        } else {
          ref.filePath = nullptr;
          ref.data = nullptr;
          ref.dataLen = 0;
        }

        if (spec.outputPath != null) {
          final outputPathPtr = spec.outputPath!.toNativeUtf8();
          disposers.add(() => calloc.free(outputPathPtr));
          ref.outputPath = outputPathPtr;
        } else {
          ref.outputPath = nullptr;
        }
      }

      final batchOutPtr = malloc<NativeBatchResult>();

      try {
        bindings.compressBatch(
          nativeInputs,
          chunkInputs.length,
          paramsPtr,
          threadCount,
          chunkSize,
          batchOutPtr,
        );

        final results = <_TransferableCompressResult>[];
        final batchRef = batchOutPtr.ref;

        for (var i = 0; i < batchRef.count; i++) {
          final native = batchRef.results[i];
          if (native.errorCode != 0) {
            final msg =
                native.errorMessage != nullptr
                    ? native.errorMessage.toDartString()
                    : 'Error (code ${native.errorCode})';
            results.add(
              _TransferableCompressResult(
                originalSize: native.originalSize,
                compressedSize: 0,
                width: 0,
                height: 0,
                qualityUsed: 0,
                iterations: 0,
                resizedToFit: false,
                errorCode: native.errorCode,
                errorMessage: msg,
              ),
            );
            continue;
          }

          TransferableTypedData? transferData;
          if (native.data != nullptr && native.dataLen > 0) {
            transferData = TransferableTypedData.fromList([
              native.data.cast<Uint8>().asTypedList(native.dataLen),
            ]);
          }

          results.add(
            _TransferableCompressResult(
              data: transferData,
              originalSize: native.originalSize,
              compressedSize: native.dataLen,
              width: native.width,
              height: native.height,
              qualityUsed: native.qualityUsed,
              iterations: native.iterations,
              resizedToFit: native.resizedToFit != 0,
            ),
          );
        }

        bindings.freeBatchResult(batchOutPtr);
        return results;
      } finally {
        malloc.free(batchOutPtr);
      }
    } finally {
      for (final dispose in disposers) {
        dispose();
      }
      calloc.free(nativeInputs);
    }
  }
}

/// Lightweight spec for crossing isolate boundary (no FFI pointers).
class _BatchInputSpec {
  const _BatchInputSpec({this.path, this.data, this.outputPath});

  final String? path;
  final Uint8List? data;
  final String? outputPath;
}

/// Exception thrown when native compression fails.
class CompressException implements Exception {
  const CompressException(this.code, this.message);

  /// Native error code from the Rust engine.
  ///
  /// | Code | Meaning |
  /// |------|---------|
  /// | -1   | Null pointer or missing input |
  /// | -2   | Invalid UTF-8 path or empty buffer |
  /// | -3   | Failed to read input file |
  /// | -4   | Failed to write output file |
  /// | -5   | Input exceeds 256 MB limit |
  /// | -10  | Compression engine error (unsupported format, corrupt data) |
  /// | -99  | Internal panic on a batch item |
  /// | -100 | Batch isolate crashed unexpectedly |
  final int code;

  /// Human-readable error message from the Rust engine.
  final String message;

  /// Actionable hint based on the error code.
  String get hint {
    switch (code) {
      case -1:
        return 'Ensure a non-empty file path or byte buffer was provided.';
      case -2:
        return 'Check that the file path uses valid UTF-8 characters and the buffer is not empty.';
      case -3:
        return 'Verify the input file exists and the app has read permission.';
      case -4:
        return 'Verify the output directory exists and the app has write permission.';
      case -5:
        return 'Input exceeds the 256 MB limit. Reduce input size or split into smaller files.';
      case -10:
        return 'Ensure the input is a valid JPEG, PNG, or WebP file. '
            'GIF, BMP, and TIFF are accepted as input but must be intact.';
      case -99:
        return 'The image may be corrupt or the device ran out of memory for this item. '
            'Other batch items are unaffected.';
      case -100:
        return 'Internal error. Please file a bug at https://github.com/rickyirfandi/ironpress/issues.';
      default:
        return '';
    }
  }

  @override
  String toString() {
    final h = hint;
    return h.isEmpty
        ? 'CompressException($code): $message'
        : 'CompressException($code): $message\nHint: $h';
  }
}
