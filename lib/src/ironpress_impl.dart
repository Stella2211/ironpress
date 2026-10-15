import 'dart:async';
import 'dart:ffi';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import 'bindings.dart';
import 'native_loader.dart';
import 'options.dart';
import 'worker_pool.dart';

/// Expected ABI version — must match the Rust constant.
/// Increment both sides when any #[repr(C)] struct layout changes.
const int _expectedAbiVersion = 2;

/// Error code used when a batch worker isolate crashes unexpectedly.
const int _isolateCrashCode = -100;

/// Maximum value representable by native `u32` fields.
const int _maxUint32 = 0xFFFFFFFF;

/// Default native batch memory budget (must match the Rust default).
const int _defaultBatchMemoryBudget = 512 * 1024 * 1024;

/// Byte inputs whose total size is at most this fraction of the memory
/// budget are copied to native memory at once and compressed in one native
/// call; larger ones are copied and compressed chunk by chunk.
const int _bulkCopyBudgetDivisor = 4;

/// How often the caller polls the shared progress counter.
const Duration _progressPollInterval = Duration(milliseconds: 16);

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

void _validateCompressionArgs({
  int? quality,
  int? maxWidth,
  int? maxHeight,
  int? maxFileSize,
  int? minQuality,
  required PngOptions png,
}) {
  _validateQuality(quality: quality, minQuality: minQuality);
  _validateResizeAndSizeParams(
    maxWidth: maxWidth,
    maxHeight: maxHeight,
    maxFileSize: maxFileSize,
  );
  _validatePngOptions(png);
}

void _validateBatchArgs({
  required int threadCount,
  required int chunkSize,
  int? memoryBudgetBytes,
}) {
  _validateNonNegativeUint32(threadCount, 'threadCount');
  _validatePositiveUint32(chunkSize, 'chunkSize');
  if (memoryBudgetBytes != null && memoryBudgetBytes < 0) {
    throw ArgumentError.value(
      memoryBudgetBytes,
      'memoryBudgetBytes',
      'must be greater than or equal to 0',
    );
  }
}

void _validateBenchmarkArgs({int? maxWidth, int? maxHeight}) {
  _validateOptionalPositiveUint32(maxWidth, 'maxWidth');
  _validateOptionalPositiveUint32(maxHeight, 'maxHeight');
}

// ─── Native bindings (cached once per isolate) ──────────────────────────────

/// Bindings for the current isolate. Each worker isolate loads the library
/// and checks the ABI once, then reuses the bindings for its lifetime.
NativeBindings? _bindings;

/// Returns the bindings for the current isolate, loading the native library
/// and verifying its ABI version on first use.
NativeBindings _nativeBindings() {
  final cached = _bindings;
  if (cached != null) {
    return cached;
  }
  final library = loadNativeLibrary();
  // Check the ABI before resolving any other symbol: a stale library may
  // lack newer functions, which must surface as an ABI mismatch.
  final version = library
      .lookupFunction<AbiVersionNative, AbiVersionDart>('ironpress_abi_version')
      .call();
  if (version != _expectedAbiVersion) {
    throw StateError(
      'Ironpress ABI version mismatch: native=$version, '
      'expected=$_expectedAbiVersion. Rebuild the native library.',
    );
  }
  return _bindings = NativeBindings.fromLibrary(library);
}

WorkerPool get _pool => WorkerPool.shared;

// ─── Settings & transferable results ────────────────────────────────────────

/// Resolved compression settings (preset already applied). Sendable.
class _CompressSettings {
  const _CompressSettings({
    required this.quality,
    this.maxWidth,
    this.maxHeight,
    this.maxFileSize,
    required this.minQuality,
    required this.allowResize,
    required this.format,
    required this.keepMetadata,
    required this.jpeg,
    required this.png,
  });

  factory _CompressSettings.resolve({
    CompressPreset? preset,
    int? quality,
    int? maxWidth,
    int? maxHeight,
    int? maxFileSize,
    int? minQuality,
    required bool allowResize,
    required CompressFormat format,
    required bool keepMetadata,
    required JpegOptions jpeg,
    required PngOptions png,
  }) {
    return _CompressSettings(
      quality: quality ?? preset?.quality ?? 80,
      maxWidth: maxWidth ?? preset?.maxWidth,
      maxHeight: maxHeight ?? preset?.maxHeight,
      maxFileSize: maxFileSize,
      minQuality: minQuality ?? preset?.minQuality ?? 30,
      allowResize: allowResize,
      format: format,
      keepMetadata: keepMetadata,
      jpeg: jpeg,
      png: png,
    );
  }

  /// Settings used by the benchmark entry points.
  factory _CompressSettings.benchmark({int? maxWidth, int? maxHeight}) {
    return _CompressSettings(
      quality: 80,
      maxWidth: maxWidth,
      maxHeight: maxHeight,
      minQuality: 30,
      allowResize: true,
      format: CompressFormat.auto,
      keepMetadata: false,
      jpeg: const JpegOptions(),
      png: const PngOptions(),
    );
  }

  final int quality;
  final int? maxWidth;
  final int? maxHeight;
  final int? maxFileSize;
  final int minQuality;
  final bool allowResize;
  final CompressFormat format;
  final bool keepMetadata;
  final JpegOptions jpeg;
  final PngOptions png;

  Pointer<NativeCompressParams> toNative(Allocator allocator) {
    final p = allocator<NativeCompressParams>();
    p.ref
      ..quality = quality.clamp(0, 100)
      ..maxWidth = maxWidth ?? 0
      ..maxHeight = maxHeight ?? 0
      ..maxFileSize = maxFileSize ?? 0
      ..minQuality = minQuality.clamp(0, 100)
      ..allowResize = allowResize ? 1 : 0
      ..format = format.value
      ..keepMetadata = keepMetadata ? 1 : 0
      ..jpegProgressive = jpeg.progressive ? 1 : 0
      ..jpegChromaSubsampling = jpeg.chromaSubsampling.value
      ..jpegTrellis = jpeg.trellis ? 1 : 0
      ..pngOptimizationLevel = png.optimizationLevel.clamp(0, 6)
      ..pngLevelUnbounded = png.limitLevelForLargeImages ? 0 : 1;
    return p;
  }
}

/// A native output buffer handed from a worker to the caller isolate by
/// address (no copy). Exactly one owner releases it: [materialize] attaches
/// `ironpress_free_buffer` as a finalizer to the returned external
/// [Uint8List]; [free] releases it directly.
class _NativeOutput {
  const _NativeOutput(this.handle, this.address, this.length, this.freeFn);

  /// Opaque owner handle (`CompressResult.buffer`).
  final int handle;

  /// Address of the first output byte.
  final int address;
  final int length;

  /// Address of `ironpress_free_buffer` (process-wide).
  final int freeFn;

  Uint8List materialize() {
    return Pointer<Uint8>.fromAddress(address).asTypedList(
      length,
      finalizer: Pointer<NativeFinalizerFunction>.fromAddress(freeFn),
      token: Pointer<Void>.fromAddress(handle),
    );
  }

  void free() {
    Pointer<NativeFunction<FreeBufferNative>>.fromAddress(freeFn)
        .asFunction<FreeBufferDart>()(Pointer<Void>.fromAddress(handle));
  }
}

class _TransferableCompressResult implements WorkerDisposable {
  const _TransferableCompressResult({
    this.output,
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

  final _NativeOutput? output;
  final int originalSize;
  final int compressedSize;
  final int width;
  final int height;
  final int qualityUsed;
  final int iterations;
  final bool resizedToFit;
  final int? errorCode;
  final String? errorMessage;

  /// Builds the public result. Must be called at most once, on the caller
  /// isolate: it takes ownership of the native output buffer.
  CompressResult materialize() {
    return CompressResult(
      data: output?.materialize(),
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

  @override
  void disposeUndelivered() => output?.free();
}

class _TransferableBatchCompressResult implements WorkerDisposable {
  const _TransferableBatchCompressResult({
    required this.results,
    required this.elapsedMs,
  });

  final List<_TransferableCompressResult> results;
  final int elapsedMs;

  /// See [_TransferableCompressResult.materialize].
  BatchCompressResult materialize() {
    return BatchCompressResult(
      results: results.map((result) => result.materialize()).toList(),
      elapsedMs: elapsedMs,
    );
  }

  @override
  void disposeUndelivered() {
    for (final result in results) {
      result.disposeUndelivered();
    }
  }
}

/// Worker-side guard for native outputs held across asynchronous gaps
/// (between batch chunks): if the worker isolate dies or the task fails
/// before the outputs are handed over, the attached [NativeFinalizer]
/// releases them.
final class _OutputGuard implements Finalizable {
  _OutputGuard(this.output, this._finalizer) {
    _finalizer.attach(
      this,
      Pointer<Void>.fromAddress(output.handle),
      detach: this,
      externalSize: output.length,
    );
  }

  final _NativeOutput output;
  final NativeFinalizer _finalizer;

  /// Hands the output over to the caller (no automatic release anymore).
  _NativeOutput release() {
    _finalizer.detach(this);
    return output;
  }

  /// Releases the output now.
  void free() => release().free();
}

/// Per-isolate finalizer releasing native outputs.
NativeFinalizer? _outputFinalizer;

NativeFinalizer _outputFinalizerFor(NativeBindings bindings) {
  return _outputFinalizer ??= NativeFinalizer(
    bindings.freeBufferPointer.cast(),
  );
}

// ─── Worker tasks (run on pooled worker isolates) ───────────────────────────

class _CompressFileTask extends WorkerTask<_TransferableCompressResult> {
  const _CompressFileTask(this.path, this.settings);

  final String path;
  final _CompressSettings settings;

  @override
  _TransferableCompressResult run(WorkerTaskContext context) {
    final bindings = _nativeBindings();
    return using((arena) {
      final out = arena<NativeCompressResult>();
      bindings.compressFile(
        path.toNativeUtf8(allocator: arena),
        settings.toNative(arena),
        out,
      );
      return _convertCompressResult(out, bindings);
    });
  }
}

class _CompressFileToFileTask extends WorkerTask<_TransferableCompressResult> {
  const _CompressFileToFileTask(this.inputPath, this.outputPath, this.settings);

  final String inputPath;
  final String outputPath;
  final _CompressSettings settings;

  @override
  _TransferableCompressResult run(WorkerTaskContext context) {
    final bindings = _nativeBindings();
    return using((arena) {
      final out = arena<NativeCompressResult>();
      bindings.compressFileToFile(
        inputPath.toNativeUtf8(allocator: arena),
        outputPath.toNativeUtf8(allocator: arena),
        settings.toNative(arena),
        out,
      );
      return _convertCompressResult(out, bindings);
    });
  }
}

/// Compresses bytes the caller already copied into native memory.
///
/// The buffer at [address] is owned (allocated and freed) by the calling
/// isolate, which keeps it alive until this task has finished or its worker
/// has exited.
class _CompressBufferTask extends WorkerTask<_TransferableCompressResult> {
  const _CompressBufferTask(this.address, this.length, this.settings);

  final int address;
  final int length;
  final _CompressSettings settings;

  @override
  _TransferableCompressResult run(WorkerTaskContext context) {
    final bindings = _nativeBindings();
    return using((arena) {
      final out = arena<NativeCompressResult>();
      bindings.compressBuffer(
        Pointer<Uint8>.fromAddress(address),
        length,
        settings.toNative(arena),
        out,
      );
      return _convertCompressResult(out, bindings);
    });
  }
}

class _ProbeFileTask extends WorkerTask<ImageProbe> {
  const _ProbeFileTask(this.path);

  final String path;

  @override
  ImageProbe run(WorkerTaskContext context) {
    final bindings = _nativeBindings();
    return using((arena) {
      final out = arena<NativeProbeResult>();
      bindings.probeFile(path.toNativeUtf8(allocator: arena), out);
      return _convertProbeResult(out, bindings);
    });
  }
}

/// See [_CompressBufferTask] for buffer ownership.
class _ProbeBufferTask extends WorkerTask<ImageProbe> {
  const _ProbeBufferTask(this.address, this.length);

  final int address;
  final int length;

  @override
  ImageProbe run(WorkerTaskContext context) {
    final bindings = _nativeBindings();
    return using((arena) {
      final out = arena<NativeProbeResult>();
      bindings.probeBuffer(Pointer<Uint8>.fromAddress(address), length, out);
      return _convertProbeResult(out, bindings);
    });
  }
}

class _BenchmarkFileTask extends WorkerTask<BenchmarkResult> {
  const _BenchmarkFileTask(this.path, this.settings);

  final String path;
  final _CompressSettings settings;

  @override
  BenchmarkResult run(WorkerTaskContext context) {
    final bindings = _nativeBindings();
    return using((arena) {
      final out = arena<NativeBenchmarkResult>();
      bindings.benchmarkFile(
        path.toNativeUtf8(allocator: arena),
        settings.toNative(arena),
        out,
      );
      return _convertBenchmarkResult(out, bindings);
    });
  }
}

/// See [_CompressBufferTask] for buffer ownership.
class _BenchmarkBufferTask extends WorkerTask<BenchmarkResult> {
  const _BenchmarkBufferTask(this.address, this.length, this.settings);

  final int address;
  final int length;
  final _CompressSettings settings;

  @override
  BenchmarkResult run(WorkerTaskContext context) {
    final bindings = _nativeBindings();
    return using((arena) {
      final out = arena<NativeBenchmarkResult>();
      bindings.benchmarkBuffer(
        Pointer<Uint8>.fromAddress(address),
        length,
        settings.toNative(arena),
        out,
      );
      return _convertBenchmarkResult(out, bindings);
    });
  }
}

// ─── Batch protocol ─────────────────────────────────────────────────────────

/// Batch item description sent to the worker. Byte inputs are not included:
/// the caller copies them into native memory on request.
class _BatchItem {
  const _BatchItem({this.path, this.dataLength, this.outputPath});

  final String? path;

  /// Length of the in-memory input, or null for path inputs.
  final int? dataLength;
  final String? outputPath;
}

/// Worker → caller: copy the byte inputs of chunk [index] to native memory.
class _BatchChunkRequest {
  const _BatchChunkRequest(this.index);

  final int index;
}

/// Caller → worker: native addresses for the items of chunk [index]
/// (0 for path inputs or empty buffers), or an [error] if copying failed.
class _BatchChunkData {
  const _BatchChunkData(this.index, this.addresses, [this.error]);

  final int index;
  final List<int>? addresses;
  final String? error;
}

/// Worker → caller: chunk [index] finished, its input buffers can be freed.
class _BatchChunkDone {
  const _BatchChunkDone(this.index);

  final int index;
}

class _BatchTask extends WorkerTask<_TransferableBatchCompressResult> {
  const _BatchTask({
    required this.items,
    required this.settings,
    required this.threadCount,
    required this.chunkSize,
    required this.memoryBudget,
    required this.streamsData,
    required this.controlAddress,
  });

  final List<_BatchItem> items;
  final _CompressSettings settings;
  final int threadCount;

  /// Items per native call. Equal to the item count (one native call) unless
  /// large byte inputs must be copied to native memory chunk by chunk.
  final int chunkSize;

  /// Native memory budget in bytes (0 = default).
  final int memoryBudget;

  /// Whether some items are in-memory inputs that must be requested from
  /// the caller (all at once when [chunkSize] covers every item).
  final bool streamsData;

  /// Address of the caller-owned [NativeBatchControl] (progress counter and
  /// cancellation flag). The caller keeps it alive until this task is done.
  final int controlAddress;

  @override
  Future<_TransferableBatchCompressResult> run(
    WorkerTaskContext context,
  ) async {
    final bindings = _nativeBindings();
    final finalizer = _outputFinalizerFor(bindings);
    final control = Pointer<NativeBatchControl>.fromAddress(controlAddress);
    bool cancelled() => control.ref.cancelled != 0;

    final stopwatch = Stopwatch()..start();
    final results = <_TransferableCompressResult>[];
    final guards = <_OutputGuard?>[];
    final chunkWaiters = <int, Completer<_BatchChunkData>>{};

    context.onMessage = (message) {
      if (message is _BatchChunkData) {
        chunkWaiters.remove(message.index)?.complete(message);
      }
    };

    final paramsPtr = settings.toNative(calloc);
    try {
      final total = items.length;
      final chunkCount = (total + chunkSize - 1) ~/ chunkSize;

      Future<_BatchChunkData>? requestChunk(int index) {
        if (!streamsData) {
          return null;
        }
        final waiter = Completer<_BatchChunkData>();
        chunkWaiters[index] = waiter;
        context.send(_BatchChunkRequest(index));
        return waiter.future;
      }

      var pending = requestChunk(0);
      for (var index = 0; index < chunkCount; index++) {
        if (cancelled()) {
          break;
        }
        final chunkData = pending == null ? null : await pending;
        if (cancelled()) {
          break;
        }
        if (chunkData != null && chunkData.addresses == null) {
          throw StateError(
            chunkData.error ?? 'Failed to copy batch input to native memory',
          );
        }

        final start = index * chunkSize;
        final end = math.min(start + chunkSize, total);
        // Prefetch: the caller copies the next chunk while this one runs.
        pending = index + 1 < chunkCount ? requestChunk(index + 1) : null;

        final chunkResults = _compressBatchChunk(
          bindings,
          items.sublist(start, end),
          chunkData?.addresses,
          paramsPtr,
          threadCount: threadCount,
          memoryBudget: memoryBudget,
          control: control,
        );
        for (final result in chunkResults) {
          results.add(result);
          final output = result.output;
          guards.add(output == null ? null : _OutputGuard(output, finalizer));
        }

        if (streamsData) {
          context.send(_BatchChunkDone(index));
        }
        if (chunkResults.length < end - start) {
          // Cancelled during the native call: the rest was skipped.
          break;
        }
      }
    } catch (_) {
      for (final guard in guards) {
        guard?.free();
      }
      rethrow;
    } finally {
      calloc.free(paramsPtr);
    }

    // Hand every output over to the caller.
    for (final guard in guards) {
      guard?.release();
    }
    stopwatch.stop();
    return _TransferableBatchCompressResult(
      results: results,
      elapsedMs: stopwatch.elapsedMilliseconds,
    );
  }
}

/// Main-isolate owner of the native input buffers of a streaming batch.
class _BatchInputBuffers {
  _BatchInputBuffers(this.inputs, this.chunkSize);

  final List<CompressInput> inputs;
  final int chunkSize;
  final Map<int, List<Pointer<Uint8>>> _live = <int, List<Pointer<Uint8>>>{};

  /// Copies the byte inputs of chunk [index] into native memory and returns
  /// their addresses (0 for path inputs and empty buffers).
  List<int> allocate(int index) {
    free(index);
    final start = index * chunkSize;
    final end = math.min(start + chunkSize, inputs.length);
    final pointers = <Pointer<Uint8>>[];
    final addresses = List<int>.filled(end - start, 0);
    try {
      for (var i = start; i < end; i++) {
        final data = inputs[i].data;
        if (data == null || data.isEmpty) {
          continue;
        }
        final pointer = _copyToNative(data);
        pointers.add(pointer);
        addresses[i - start] = pointer.address;
      }
    } catch (_) {
      pointers.forEach(malloc.free);
      rethrow;
    }
    _live[index] = pointers;
    return addresses;
  }

  void free(int index) {
    _live.remove(index)?.forEach(malloc.free);
  }

  void freeAll() {
    for (final pointers in _live.values) {
      pointers.forEach(malloc.free);
    }
    _live.clear();
  }
}

// ─── Native helpers (worker side) ───────────────────────────────────────────

Pointer<Uint8> _copyToNative(Uint8List data) {
  final pointer = malloc<Uint8>(data.length);
  pointer.asTypedList(data.length).setAll(0, data);
  return pointer;
}

/// Copies [data] into native memory owned by the calling isolate, runs
/// [createTask] on a worker with its address, and frees the copy once the
/// worker is done with it — whether the task succeeded, threw, or the worker
/// died. This avoids an intermediate `TransferableTypedData` copy.
Future<T> _runWithNativeCopy<T>(
  Uint8List data,
  WorkerTask<T> Function(int address, int length) createTask,
) async {
  final pointer = _copyToNative(data);
  try {
    return await _pool.run(createTask(pointer.address, data.length));
  } finally {
    malloc.free(pointer);
  }
}

String _nativeErrorMessage(Pointer<Utf8> message, String fallback) {
  return message != nullptr ? message.toDartString() : fallback;
}

/// Converts a native compress result and releases its native payload. The
/// result struct itself belongs to the caller's allocator.
_TransferableCompressResult _convertCompressResult(
  Pointer<NativeCompressResult> outPtr,
  NativeBindings bindings,
) {
  try {
    final ref = outPtr.ref;
    if (ref.errorCode != 0) {
      throw CompressException(
        ref.errorCode,
        _nativeErrorMessage(
          ref.errorMessage,
          'Unknown error (code ${ref.errorCode})',
        ),
      );
    }
    return _successResult(ref, bindings);
  } finally {
    bindings.freeCompressResult(outPtr);
  }
}

/// Converts a successful native result, taking over its output buffer (the
/// `buffer` field is cleared so that freeing the result leaves it alone).
_TransferableCompressResult _successResult(
  NativeCompressResult ref,
  NativeBindings bindings,
) {
  _NativeOutput? output;
  if (ref.buffer != nullptr && ref.data != nullptr && ref.dataLen > 0) {
    output = _NativeOutput(
      ref.buffer.address,
      ref.data.address,
      ref.dataLen,
      bindings.freeBufferPointer.address,
    );
    ref.buffer = nullptr;
  }
  return _TransferableCompressResult(
    output: output,
    originalSize: ref.originalSize,
    compressedSize: ref.dataLen,
    width: ref.width,
    height: ref.height,
    qualityUsed: ref.qualityUsed,
    iterations: ref.iterations,
    resizedToFit: ref.resizedToFit != 0,
  );
}

ImageProbe _convertProbeResult(
  Pointer<NativeProbeResult> outPtr,
  NativeBindings bindings,
) {
  try {
    final ref = outPtr.ref;
    if (ref.errorCode != 0) {
      throw CompressException(
        ref.errorCode,
        _nativeErrorMessage(
          ref.errorMessage,
          'Probe failed (code ${ref.errorCode})',
        ),
      );
    }
    return ImageProbe(
      width: ref.width,
      height: ref.height,
      format: ImageFormat.fromValue(ref.format),
      fileSize: ref.fileSize,
      hasExif: ref.hasExif != 0,
      orientation: ref.orientation == 0 ? 1 : ref.orientation,
    );
  } finally {
    bindings.freeProbeResult(outPtr);
  }
}

BenchmarkResult _convertBenchmarkResult(
  Pointer<NativeBenchmarkResult> outPtr,
  NativeBindings bindings,
) {
  try {
    final ref = outPtr.ref;
    if (ref.errorCode != 0) {
      throw CompressException(
        ref.errorCode,
        _nativeErrorMessage(
          ref.errorMessage,
          'Benchmark failed (code ${ref.errorCode})',
        ),
      );
    }
    return BenchmarkResult(
      originalSize: ref.originalSize,
      width: ref.width,
      height: ref.height,
      format: ImageFormat.fromValue(ref.format),
      entries: [
        for (var i = 0; i < ref.entryCount; i++)
          BenchmarkEntry(
            quality: ref.entries[i].quality,
            sizeBytes: ref.entries[i].sizeBytes,
            ratio: ref.entries[i].ratio,
            encodeMs: ref.entries[i].encodeMs,
          ),
      ],
      recommendedQuality: ref.recommendedQuality,
    );
  } finally {
    bindings.freeBenchmarkResult(outPtr);
  }
}

/// Runs one native `compress_batch` call over [items].
///
/// [addresses] holds the native buffers of in-memory items (owned by the
/// caller isolate); it may be null when every item is a path input. Returns
/// fewer results than [items] when [control] was cancelled during the call
/// (the results are then the leading items that were started).
List<_TransferableCompressResult> _compressBatchChunk(
  NativeBindings bindings,
  List<_BatchItem> items,
  List<int>? addresses,
  Pointer<NativeCompressParams> paramsPtr, {
  required int threadCount,
  required int memoryBudget,
  required Pointer<NativeBatchControl> control,
}) {
  return using((arena) {
    final nativeInputs = arena<NativeBatchInput>(items.length);
    for (var i = 0; i < items.length; i++) {
      final item = items[i];
      final ref = nativeInputs[i];
      final path = item.path;
      final dataLength = item.dataLength;
      final outputPath = item.outputPath;
      ref.filePath = path == null
          ? nullptr
          : path.toNativeUtf8(allocator: arena);
      if (path == null && dataLength != null && addresses != null) {
        ref.data = Pointer<Uint8>.fromAddress(addresses[i]);
        ref.dataLen = addresses[i] == 0 ? 0 : dataLength;
      } else {
        ref.data = nullptr;
        ref.dataLen = 0;
      }
      ref.outputPath = outputPath == null
          ? nullptr
          : outputPath.toNativeUtf8(allocator: arena);
    }

    final batchOutPtr = arena<NativeBatchResult>();
    final results = <_TransferableCompressResult>[];
    try {
      bindings.compressBatch(
        nativeInputs,
        items.length,
        paramsPtr,
        threadCount,
        memoryBudget,
        control,
        batchOutPtr,
      );

      final batchRef = batchOutPtr.ref;
      for (var i = 0; i < batchRef.count; i++) {
        final native = batchRef.results[i];
        if (native.errorCode != 0) {
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
              errorMessage: _nativeErrorMessage(
                native.errorMessage,
                'Error (code ${native.errorCode})',
              ),
            ),
          );
        } else {
          results.add(_successResult(native, bindings));
        }
      }
      return results;
    } catch (_) {
      // Outputs already taken over would no longer be freed with the batch.
      for (final result in results) {
        result.disposeUndelivered();
      }
      rethrow;
    } finally {
      // Always release the native results (outputs taken over are skipped).
      bindings.freeBatchResult(batchOutPtr);
    }
  });
}

/// Maps a failure of a batch that reports progress or supports cancellation
/// to the exception type that path has always surfaced.
Object _eventBatchError(Object error, StackTrace stackTrace) {
  if (error is CompressException) {
    return error;
  }
  final trace = stackTrace.toString();
  return StateError(trace.isEmpty ? '$error' : '$error\n$trace');
}

/// High-performance image compression powered by Rust.
///
/// Uses mozjpeg (trellis quantization) for JPEG and oxipng for PNG —
/// delivering consistent, high-quality results across all platforms.
///
/// All operations run on a small pool of long-lived background isolates to
/// keep the UI responsive. Workers load the native library once and exit
/// after a few seconds without work.
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
  /// to fit within these bounds while preserving aspect ratio. The EXIF
  /// orientation is applied first, so the bounds refer to the image as
  /// displayed.
  ///
  /// [maxFileSize] — Target maximum output size in bytes. When set,
  /// the engine searches for the highest quality that fits (size-model
  /// interpolation, typically 3-5 encodes). This is the killer feature —
  /// one FFI call, no round-trips.
  ///
  /// [minQuality] — Quality floor for the quality search (default: 30).
  ///
  /// [allowResize] — If quality alone can't hit [maxFileSize], allow
  /// automatic downscaling. Default: `true`.
  ///
  /// [format] — Output format. Default: [CompressFormat.auto] (same as input).
  ///
  /// [keepMetadata] — Preserve JPEG EXIF metadata when compressing JPEG to
  /// JPEG. The pixels are always rotated into display orientation, so the
  /// kept EXIF Orientation tag is reset to 1. Other paths silently drop
  /// metadata.
  ///
  /// [jpeg] — Advanced JPEG options (progressive, trellis, chroma).
  ///
  /// [png] — Advanced PNG options (optimization level).
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
  /// // Target file size — quality search in Rust, single FFI call
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
    JpegOptions jpeg = const JpegOptions(),
    PngOptions png = const PngOptions(),
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
    );
    final settings = _CompressSettings.resolve(
      preset: preset,
      quality: quality,
      maxWidth: maxWidth,
      maxHeight: maxHeight,
      maxFileSize: maxFileSize,
      minQuality: minQuality,
      allowResize: allowResize,
      format: format,
      keepMetadata: keepMetadata,
      jpeg: jpeg,
      png: png,
    );
    final result = await _pool.run(_CompressFileTask(path, settings));
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
    JpegOptions jpeg = const JpegOptions(),
    PngOptions png = const PngOptions(),
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
    );
    final settings = _CompressSettings.resolve(
      preset: preset,
      quality: quality,
      maxWidth: maxWidth,
      maxHeight: maxHeight,
      maxFileSize: maxFileSize,
      minQuality: minQuality,
      allowResize: allowResize,
      format: format,
      keepMetadata: keepMetadata,
      jpeg: jpeg,
      png: png,
    );
    final result = await _pool.run(
      _CompressFileToFileTask(inputPath, outputPath, settings),
    );
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
    JpegOptions jpeg = const JpegOptions(),
    PngOptions png = const PngOptions(),
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
    );
    final settings = _CompressSettings.resolve(
      preset: preset,
      quality: quality,
      maxWidth: maxWidth,
      maxHeight: maxHeight,
      maxFileSize: maxFileSize,
      minQuality: minQuality,
      allowResize: allowResize,
      format: format,
      keepMetadata: keepMetadata,
      jpeg: jpeg,
      png: png,
    );
    final result = await _runWithNativeCopy(
      data,
      (address, length) => _CompressBufferTask(address, length, settings),
    );
    return result.materialize();
  }

  /// Batch compress multiple images using Rust's rayon thread pool.
  ///
  /// **Performance:** rayon splits the work across CPU cores with
  /// zero-copy shared params. For 200 inspection photos, this is 3-5x faster
  /// than sequential Dart isolates.
  ///
  /// **Scheduling:** the batch is normally handed to the native engine in a
  /// single call, so no thread idles waiting for a slow image before the
  /// next ones can start — also with [onProgress] and [cancellationToken],
  /// which work through a small shared native block (a completed-items
  /// counter polled by this isolate and a cancellation flag) instead of
  /// splitting the batch.
  ///
  /// In-memory (`data`) inputs must be copied to native memory. When their
  /// total size is at most a quarter of the memory budget (128 MB by
  /// default) they are copied at once when the batch starts; larger byte
  /// batches are copied and compressed [chunkSize] items at a time (default
  /// 8, plus one chunk of prefetch), which bounds the extra native memory to
  /// about two chunks of input bytes. Path inputs never need a copy and
  /// ignore [chunkSize]. In-memory inputs are read lazily: do not modify
  /// their buffers until the returned future completes.
  ///
  /// **Memory budget:** images are started in order only while the sum of
  /// their estimated peak memory (from the image header: decode size, which
  /// accounts for EXIF rotation and reduced-size JPEG decoding, plus the
  /// encoder working set) stays within [memoryBudgetBytes] — `null` or 0
  /// selects the default of 512 MB. Large images therefore run with less
  /// concurrency instead of all being decoded at once; a single image larger
  /// than the budget still runs, alone. The accounting is process-wide:
  /// each batch admits an image only while the estimated memory of *all*
  /// running batch images (from every concurrent call) plus the new image
  /// fits its own budget, so with equal budgets concurrent batches share one
  /// limit, and a batch with a smaller budget waits for the others to drain.
  ///
  /// **Safety:** Designed to never hang, OOM, or crash your app:
  /// - [threadCount] defaults to `cores - 2`, leaving room for Flutter
  ///   UI thread and Dart isolate. Your app stays responsive. It also bounds
  ///   how many images are decoded simultaneously.
  /// - Each item is panic-safe: a corrupt JPEG or OOM on one image
  ///   produces an error result, not a process crash.
  ///
  /// **Progress:** Pass [onProgress] to get live updates. Called on the
  /// calling isolate with `(completed, total)` — safe to call `setState`
  /// directly. `completed` counts finished images (successful or failed),
  /// increases monotonically (polled about every 16 ms, so several images
  /// may complete between two calls) and is reported at most once per
  /// value; the last call reports the final number of results (`total`
  /// unless cancelled).
  ///
  /// **Cancellation:** Pass a [CancellationToken] to cancel. Images that
  /// have not started yet are skipped; images already being compressed
  /// finish. The returned result holds the images that were processed — the
  /// leading part of [inputs], in order. Cancelling before the batch starts
  /// returns an empty result.
  ///
  /// **Output bytes** (`CompressResult.data`) are native memory owned by the
  /// returned list and released when it is garbage collected (no copy is
  /// made on the way back from the native engine).
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
    JpegOptions jpeg = const JpegOptions(),
    PngOptions png = const PngOptions(),
    int threadCount = 0,
    int chunkSize = 8,
    int? memoryBudgetBytes,
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
    );
    _validateBatchArgs(
      threadCount: threadCount,
      chunkSize: chunkSize,
      memoryBudgetBytes: memoryBudgetBytes,
    );
    final total = inputs.length;
    if (total == 0) {
      return const BatchCompressResult(results: [], elapsedMs: 0);
    }
    if (cancellationToken?.isCancelled ?? false) {
      return const BatchCompressResult(results: [], elapsedMs: 0);
    }

    final settings = _CompressSettings.resolve(
      preset: preset,
      quality: quality,
      maxWidth: maxWidth,
      maxHeight: maxHeight,
      maxFileSize: maxFileSize,
      minQuality: minQuality,
      allowResize: allowResize,
      format: format,
      keepMetadata: keepMetadata,
      jpeg: jpeg,
      png: png,
    );
    // Snapshot the inputs: the caller may mutate its list while we run.
    final snapshot = List<CompressInput>.of(inputs, growable: false);
    final items = [
      for (final input in snapshot)
        _BatchItem(
          path: input.path,
          dataLength: input.path == null ? input.data?.length : null,
          outputPath: input.outputPath,
        ),
    ];
    final streamsData = items.any((item) => item.dataLength != null);
    final dataBytes = items.fold<int>(
      0,
      (sum, item) => sum + (item.dataLength ?? 0),
    );
    final budget = (memoryBudgetBytes ?? 0) == 0
        ? _defaultBatchMemoryBudget
        : memoryBudgetBytes!;
    final nativeChunkSize =
        !streamsData || dataBytes <= budget ~/ _bulkCopyBudgetDivisor
        ? total
        : chunkSize;
    final hasEvents = onProgress != null || cancellationToken != null;
    final buffers = _BatchInputBuffers(snapshot, nativeChunkSize);
    // Shared with the native call: progress counter + cancellation flag.
    final control = calloc<NativeBatchControl>();
    var lastCompleted = 0;

    void reportProgress(int completed) {
      if (onProgress != null && completed > lastCompleted) {
        lastCompleted = completed;
        onProgress(completed, total);
      }
    }

    final job = _pool.submit(
      _BatchTask(
        items: items,
        settings: settings,
        threadCount: threadCount,
        chunkSize: nativeChunkSize,
        memoryBudget: memoryBudgetBytes ?? 0,
        streamsData: streamsData,
        controlAddress: control.address,
      ),
      onEvent: (event, reply) {
        if (event is _BatchChunkRequest) {
          try {
            reply(_BatchChunkData(event.index, buffers.allocate(event.index)));
          } catch (error) {
            reply(_BatchChunkData(event.index, null, '$error'));
          }
        } else if (event is _BatchChunkDone) {
          buffers.free(event.index);
          reportProgress(control.ref.completed);
        }
      },
      crashError: hasEvents
          ? (details) => CompressException(
              _isolateCrashCode,
              'Batch compression isolate crashed: $details',
            )
          : null,
    );
    final removeCancelListener = cancellationToken?.addListener(() {
      control.ref.cancelled = 1;
    });
    final poller = onProgress == null
        ? null
        : Timer.periodic(
            _progressPollInterval,
            (_) => reportProgress(control.ref.completed),
          );

    try {
      final result = (await job.result).materialize();
      poller?.cancel();
      reportProgress(result.results.length);
      return result;
    } catch (error, stackTrace) {
      if (!hasEvents) {
        rethrow;
      }
      Error.throwWithStackTrace(
        _eventBatchError(error, stackTrace),
        stackTrace,
      );
    } finally {
      poller?.cancel();
      removeCancelListener?.call();
      // The worker has finished (or died), so no native call can still be
      // reading these buffers or the control block.
      buffers.freeAll();
      calloc.free(control);
    }
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
  static Future<ImageProbe> probeFile(String path) async {
    if (path.isEmpty) {
      throw ArgumentError.value(path, 'path', 'must not be empty');
    }
    return _pool.run(_ProbeFileTask(path));
  }

  /// Read image metadata from bytes without decoding pixel data.
  ///
  /// Throws [ArgumentError] if [data] is empty.
  /// Throws [CompressException] if the data cannot be parsed.
  static Future<ImageProbe> probeBytes(Uint8List data) async {
    if (data.isEmpty) {
      throw ArgumentError.value(data, 'data', 'must not be empty');
    }
    return _runWithNativeCopy(data, _ProbeBufferTask.new);
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
    return _pool.run(
      _BenchmarkFileTask(
        path,
        _CompressSettings.benchmark(maxWidth: maxWidth, maxHeight: maxHeight),
      ),
    );
  }

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
    final settings = _CompressSettings.benchmark(
      maxWidth: maxWidth,
      maxHeight: maxHeight,
    );
    return _runWithNativeCopy(
      data,
      (address, length) => _BenchmarkBufferTask(address, length, settings),
    );
  }

  /// Return the native library version string.
  static String get nativeVersion => _nativeBindings().version().toDartString();
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
  /// | -5   | Input exceeds the 256 MB size limit or the 100 MP pixel limit |
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
        return 'Input exceeds the 256 MB size limit or the 100 MP pixel limit. '
            'Reduce the input size or resolution, or split it into smaller files.';
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
