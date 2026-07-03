import 'dart:ffi';
import 'package:ffi/ffi.dart';

// ─── Native struct mirrors ───────────────────────────────────────────────────

/// Mirrors Rust's CompressParams #[repr(C)]
final class NativeCompressParams extends Struct {
  @Uint32()
  external int quality;
  @Uint32()
  external int maxWidth;
  @Uint32()
  external int maxHeight;
  @Uint32()
  external int maxFileSize;
  @Uint32()
  external int minQuality;
  @Uint32()
  external int allowResize;
  @Uint32()
  external int format;
  @Uint32()
  external int keepMetadata;
  @Uint32()
  external int jpegProgressive;
  @Uint32()
  external int jpegChromaSubsampling;
  @Uint32()
  external int jpegTrellis;
  @Uint32()
  external int pngOptimizationLevel;
  @Uint32()
  external int pngLossy;
  @Uint32()
  external int autoOrient;
  @Uint32()
  external int preserveIcc;
  @Uint32()
  external int avifSpeed;
}

/// Mirrors Rust's CompressResult #[repr(C)]
final class NativeCompressResult extends Struct {
  external Pointer<Uint8> data;
  @Size()
  external int dataLen;
  @Size()
  external int originalSize;
  @Uint32()
  external int width;
  @Uint32()
  external int height;
  @Uint32()
  external int qualityUsed;
  @Uint32()
  external int iterations;
  @Uint32()
  external int resizedToFit;
  @Int32()
  external int errorCode;
  external Pointer<Utf8> errorMessage;
}

/// Mirrors Rust's BatchInput #[repr(C)]
final class NativeBatchInput extends Struct {
  external Pointer<Utf8> filePath;
  external Pointer<Uint8> data;
  @Size()
  external int dataLen;
  external Pointer<Utf8> outputPath;
}

/// Mirrors Rust's BatchResult #[repr(C)]
final class NativeBatchResult extends Struct {
  external Pointer<NativeCompressResult> results;
  @Size()
  external int count;
  @Uint64()
  external int elapsedMs;
  // Reserved for ABI compatibility with prebuilt native libraries.
  external Pointer<Uint32> completed;
}

// ─── Native function typedefs ────────────────────────────────────────────────

// compress_file(path, params, out) -> void
typedef CompressFileNative =
    Void Function(
      Pointer<Utf8> inputPath,
      Pointer<NativeCompressParams> params,
      Pointer<NativeCompressResult> out,
    );
typedef CompressFileDart =
    void Function(
      Pointer<Utf8> inputPath,
      Pointer<NativeCompressParams> params,
      Pointer<NativeCompressResult> out,
    );

// compress_buffer(data, len, params, out) -> void
typedef CompressBufferNative =
    Void Function(
      Pointer<Uint8> inputData,
      Size inputLen,
      Pointer<NativeCompressParams> params,
      Pointer<NativeCompressResult> out,
    );
typedef CompressBufferDart =
    void Function(
      Pointer<Uint8> inputData,
      int inputLen,
      Pointer<NativeCompressParams> params,
      Pointer<NativeCompressResult> out,
    );

// compress_file_to_file(input, output, params, out) -> void
typedef CompressFileToFileNative =
    Void Function(
      Pointer<Utf8> inputPath,
      Pointer<Utf8> outputPath,
      Pointer<NativeCompressParams> params,
      Pointer<NativeCompressResult> out,
    );
typedef CompressFileToFileDart =
    void Function(
      Pointer<Utf8> inputPath,
      Pointer<Utf8> outputPath,
      Pointer<NativeCompressParams> params,
      Pointer<NativeCompressResult> out,
    );

// compress_batch(inputs, count, params, thread_count, chunk_size, out) -> void
typedef CompressBatchNative =
    Void Function(
      Pointer<NativeBatchInput> inputs,
      Size count,
      Pointer<NativeCompressParams> params,
      Uint32 threadCount,
      Uint32 chunkSize,
      Pointer<NativeBatchResult> out,
    );
typedef CompressBatchDart =
    void Function(
      Pointer<NativeBatchInput> inputs,
      int count,
      Pointer<NativeCompressParams> params,
      int threadCount,
      int chunkSize,
      Pointer<NativeBatchResult> out,
    );

// free_compress_result(result)
typedef FreeCompressResultNative =
    Void Function(Pointer<NativeCompressResult> result);
typedef FreeCompressResultDart =
    void Function(Pointer<NativeCompressResult> result);

// free_batch_result(result)
typedef FreeBatchResultNative =
    Void Function(Pointer<NativeBatchResult> result);
typedef FreeBatchResultDart = void Function(Pointer<NativeBatchResult> result);

// ─── Probe ───────────────────────────────────────────────────────────────────

/// Mirrors Rust's ProbeResult #[repr(C)]
final class NativeProbeResult extends Struct {
  @Uint32()
  external int width;
  @Uint32()
  external int height;
  @Uint32()
  external int format;
  @Size()
  external int fileSize;
  @Uint32()
  external int hasExif;
  @Int32()
  external int errorCode;
  external Pointer<Utf8> errorMessage;
}

// probe_file(path, out) -> void
typedef ProbeFileNative =
    Void Function(Pointer<Utf8> path, Pointer<NativeProbeResult> out);
typedef ProbeFileDart =
    void Function(Pointer<Utf8> path, Pointer<NativeProbeResult> out);

// probe_buffer(data, len, out) -> void
typedef ProbeBufferNative =
    Void Function(
      Pointer<Uint8> data,
      Size len,
      Pointer<NativeProbeResult> out,
    );
typedef ProbeBufferDart =
    void Function(Pointer<Uint8> data, int len, Pointer<NativeProbeResult> out);

typedef FreeProbeResultNative = Void Function(Pointer<NativeProbeResult> r);
typedef FreeProbeResultDart = void Function(Pointer<NativeProbeResult> r);

// ─── Benchmark ───────────────────────────────────────────────────────────────

/// Mirrors Rust's BenchmarkEntry #[repr(C)]
final class NativeBenchmarkEntry extends Struct {
  @Uint32()
  external int quality;
  @Size()
  external int sizeBytes;
  @Float()
  external double ratio;
  @Uint32()
  external int encodeMs;
}

/// Mirrors Rust's BenchmarkResult #[repr(C)]
final class NativeBenchmarkResult extends Struct {
  @Size()
  external int originalSize;
  @Uint32()
  external int width;
  @Uint32()
  external int height;
  @Uint32()
  external int format;
  external Pointer<NativeBenchmarkEntry> entries;
  @Size()
  external int entryCount;
  @Uint32()
  external int recommendedQuality;
  @Int32()
  external int errorCode;
  external Pointer<Utf8> errorMessage;
}

// benchmark_file(path, params, out) -> void
typedef BenchmarkFileNative =
    Void Function(
      Pointer<Utf8> path,
      Pointer<NativeCompressParams> params,
      Pointer<NativeBenchmarkResult> out,
    );
typedef BenchmarkFileDart =
    void Function(
      Pointer<Utf8> path,
      Pointer<NativeCompressParams> params,
      Pointer<NativeBenchmarkResult> out,
    );

// benchmark_buffer(data, len, params, out) -> void
typedef BenchmarkBufferNative =
    Void Function(
      Pointer<Uint8> data,
      Size len,
      Pointer<NativeCompressParams> params,
      Pointer<NativeBenchmarkResult> out,
    );
typedef BenchmarkBufferDart =
    void Function(
      Pointer<Uint8> data,
      int len,
      Pointer<NativeCompressParams> params,
      Pointer<NativeBenchmarkResult> out,
    );

typedef FreeBenchmarkResultNative =
    Void Function(Pointer<NativeBenchmarkResult> r);
typedef FreeBenchmarkResultDart =
    void Function(Pointer<NativeBenchmarkResult> r);

// ironpress_version() -> char*
typedef VersionNative = Pointer<Utf8> Function();
typedef VersionDart = Pointer<Utf8> Function();

// ironpress_abi_version() -> u32
typedef AbiVersionNative = Uint32 Function();
typedef AbiVersionDart = int Function();

// ─── Binding holder ──────────────────────────────────────────────────────────

class NativeBindings {
  NativeBindings._(
    this.compressFile,
    this.compressBuffer,
    this.compressFileToFile,
    this.compressBatch,
    this.freeCompressResult,
    this.freeBatchResult,
    this.probeFile,
    this.probeBuffer,
    this.freeProbeResult,
    this.benchmarkFile,
    this.benchmarkBuffer,
    this.freeBenchmarkResult,
    this.version,
    this.abiVersion,
  );

  factory NativeBindings.fromLibrary(DynamicLibrary lib) {
    return NativeBindings._(
      lib.lookupFunction<CompressFileNative, CompressFileDart>('compress_file'),
      lib.lookupFunction<CompressBufferNative, CompressBufferDart>(
        'compress_buffer',
      ),
      lib.lookupFunction<CompressFileToFileNative, CompressFileToFileDart>(
        'compress_file_to_file',
      ),
      lib.lookupFunction<CompressBatchNative, CompressBatchDart>(
        'compress_batch',
      ),
      lib.lookupFunction<FreeCompressResultNative, FreeCompressResultDart>(
        'free_compress_result',
      ),
      lib.lookupFunction<FreeBatchResultNative, FreeBatchResultDart>(
        'free_batch_result',
      ),
      lib.lookupFunction<ProbeFileNative, ProbeFileDart>('probe_file'),
      lib.lookupFunction<ProbeBufferNative, ProbeBufferDart>('probe_buffer'),
      lib.lookupFunction<FreeProbeResultNative, FreeProbeResultDart>(
        'free_probe_result',
      ),
      lib.lookupFunction<BenchmarkFileNative, BenchmarkFileDart>(
        'benchmark_file',
      ),
      lib.lookupFunction<BenchmarkBufferNative, BenchmarkBufferDart>(
        'benchmark_buffer',
      ),
      lib.lookupFunction<FreeBenchmarkResultNative, FreeBenchmarkResultDart>(
        'free_benchmark_result',
      ),
      lib.lookupFunction<VersionNative, VersionDart>('ironpress_version'),
      lib.lookupFunction<AbiVersionNative, AbiVersionDart>(
        'ironpress_abi_version',
      ),
    );
  }

  final CompressFileDart compressFile;
  final CompressBufferDart compressBuffer;
  final CompressFileToFileDart compressFileToFile;
  final CompressBatchDart compressBatch;
  final FreeCompressResultDart freeCompressResult;
  final FreeBatchResultDart freeBatchResult;
  final ProbeFileDart probeFile;
  final ProbeBufferDart probeBuffer;
  final FreeProbeResultDart freeProbeResult;
  final BenchmarkFileDart benchmarkFile;
  final BenchmarkBufferDart benchmarkBuffer;
  final FreeBenchmarkResultDart freeBenchmarkResult;
  final VersionDart version;
  final AbiVersionDart abiVersion;
}
