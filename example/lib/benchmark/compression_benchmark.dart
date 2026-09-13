import 'dart:isolate';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_image_compress/flutter_image_compress.dart' as fic;
import 'package:image/image.dart' as img;
import 'package:ironpress/ironpress.dart';

class BenchmarkConfig {
  const BenchmarkConfig({
    required this.quality,
    required this.batchCount,
    this.singleRuns = 5,
    this.batchRuns = 3,
  });

  final int quality;
  final int batchCount;
  final int singleRuns;
  final int batchRuns;
}

class BenchmarkProgress {
  const BenchmarkProgress({
    required this.message,
    required this.completedSteps,
    required this.totalSteps,
  });

  final String message;
  final int completedSteps;
  final int totalSteps;

  double get fraction => totalSteps == 0 ? 0 : completedSteps / totalSteps;
}

class ComparisonBenchmarkResult {
  const ComparisonBenchmarkResult({
    required this.config,
    required this.inputProbe,
    required this.packages,
    required this.ironpressSweep,
  });

  final BenchmarkConfig config;
  final ImageProbe inputProbe;
  final List<PackageBenchmarkResult> packages;
  final BenchmarkResult ironpressSweep;

  Iterable<PackageBenchmarkResult> get completedPackages =>
      packages.where((pkg) => pkg.single != null && pkg.batch != null);

  Iterable<PackageBenchmarkResult> get unavailablePackages =>
      packages.where((pkg) => pkg.statusMessage != null);
}

class PackageBenchmarkResult {
  const PackageBenchmarkResult({
    required this.id,
    required this.name,
    required this.subtitle,
    required this.usesNativeBatch,
    this.single,
    this.batch,
    this.statusMessage,
  });

  final String id;
  final String name;
  final String subtitle;
  final bool usesNativeBatch;
  final SingleBenchmarkResult? single;
  final BatchBenchmarkResult? batch;
  final String? statusMessage;
}

class SingleBenchmarkResult {
  const SingleBenchmarkResult({
    required this.outputBytes,
    required this.medianElapsedMs,
    required this.sampleElapsedMs,
  });

  final int outputBytes;
  final double medianElapsedMs;
  final List<double> sampleElapsedMs;

  double reductionRatio(int originalBytes) =>
      originalBytes > 0 ? outputBytes / originalBytes : 1.0;

  /// Bytes saved per millisecond — rewards both speed AND compression.
  double bytesSavedPerMs(int originalBytes) {
    final saved = originalBytes - outputBytes;
    return medianElapsedMs > 0 ? saved / medianElapsedMs : 0;
  }
}

class BatchBenchmarkResult {
  const BatchBenchmarkResult({
    required this.totalInputBytes,
    required this.totalOutputBytes,
    required this.successCount,
    required this.batchCount,
    required this.medianElapsedMs,
    required this.sampleElapsedMs,
  });

  final int totalInputBytes;
  final int totalOutputBytes;
  final int successCount;
  final int batchCount;
  final double medianElapsedMs;
  final List<double> sampleElapsedMs;

  double get reductionRatio =>
      totalInputBytes > 0 ? totalOutputBytes / totalInputBytes : 1.0;

  double get imagesPerSecond =>
      medianElapsedMs > 0 ? batchCount / (medianElapsedMs / 1000.0) : 0;

  double get mbPerSecond => medianElapsedMs > 0
      ? (totalInputBytes / (1024 * 1024)) / (medianElapsedMs / 1000.0)
      : 0;
}

class _SingleRunSample {
  const _SingleRunSample({required this.elapsedMs, required this.outputBytes});

  final double elapsedMs;
  final int outputBytes;
}

class _BatchRunSample {
  const _BatchRunSample({
    required this.elapsedMs,
    required this.totalOutputBytes,
    required this.successCount,
  });

  final double elapsedMs;
  final int totalOutputBytes;
  final int successCount;
}

class BatchRunOutput {
  const BatchRunOutput({
    required this.totalOutputBytes,
    required this.successCount,
  });

  final int totalOutputBytes;
  final int successCount;
}

abstract class CompressionBenchmarkAdapter {
  const CompressionBenchmarkAdapter();

  String get id;
  String get name;
  String get subtitle;
  bool get usesNativeBatch;

  String? unsupportedReason();

  Future<Uint8List> compress(Uint8List data, {required int quality});

  Future<BatchRunOutput> compressBatch(
    List<Uint8List> inputs, {
    required int quality,
  });
}

class IronpressBenchmarkAdapter extends CompressionBenchmarkAdapter {
  const IronpressBenchmarkAdapter();

  @override
  String get id => 'ironpress';

  @override
  String get name => 'ironpress';

  @override
  String get subtitle => 'Rust/mozjpeg, trellis on — size-optimized';

  @override
  bool get usesNativeBatch => true;

  @override
  String? unsupportedReason() => null;

  @override
  Future<Uint8List> compress(Uint8List data, {required int quality}) async {
    final result = await Ironpress.compressBytes(
      data,
      quality: quality,
      allowResize: false,
      format: CompressFormat.jpeg,
      keepMetadata: false,
    );
    final bytes = result.data;
    if (bytes == null) {
      throw StateError('ironpress returned no in-memory output bytes.');
    }
    return bytes;
  }

  @override
  Future<BatchRunOutput> compressBatch(
    List<Uint8List> inputs, {
    required int quality,
  }) async {
    final result = await Ironpress.compressBatch(
      inputs.map((bytes) => CompressInput(data: bytes)).toList(),
      quality: quality,
      allowResize: false,
      format: CompressFormat.jpeg,
      keepMetadata: false,
    );
    return BatchRunOutput(
      totalOutputBytes: result.totalCompressedSize,
      successCount: result.successfulCount,
    );
  }
}

class IronpressFastBenchmarkAdapter extends CompressionBenchmarkAdapter {
  const IronpressFastBenchmarkAdapter();

  @override
  String get id => 'ironpress_fast';

  @override
  String get name => 'ironpress (fast)';

  @override
  String get subtitle => 'Rust codecs, trellis off — speed-optimized';

  @override
  bool get usesNativeBatch => true;

  @override
  String? unsupportedReason() => null;

  @override
  Future<Uint8List> compress(Uint8List data, {required int quality}) async {
    final result = await Ironpress.compressBytes(
      data,
      quality: quality,
      allowResize: false,
      format: CompressFormat.jpeg,
      keepMetadata: false,
      jpeg: const JpegOptions(trellis: false, progressive: false),
    );
    final bytes = result.data;
    if (bytes == null) {
      throw StateError('ironpress returned no in-memory output bytes.');
    }
    return bytes;
  }

  @override
  Future<BatchRunOutput> compressBatch(
    List<Uint8List> inputs, {
    required int quality,
  }) async {
    final result = await Ironpress.compressBatch(
      inputs.map((bytes) => CompressInput(data: bytes)).toList(),
      quality: quality,
      allowResize: false,
      format: CompressFormat.jpeg,
      keepMetadata: false,
      jpeg: const JpegOptions(trellis: false, progressive: false),
    );
    return BatchRunOutput(
      totalOutputBytes: result.totalCompressedSize,
      successCount: result.successfulCount,
    );
  }
}

class FlutterImageCompressBenchmarkAdapter extends CompressionBenchmarkAdapter {
  const FlutterImageCompressBenchmarkAdapter();

  @override
  String get id => 'flutter_image_compress';

  @override
  String get name => 'flutter_image_compress';

  @override
  String get subtitle => 'Popular native plugin';

  @override
  bool get usesNativeBatch => false;

  @override
  String? unsupportedReason() {
    if (kIsWeb) {
      return 'Skipped on web in this example because the plugin needs extra runtime setup.';
    }
    switch (defaultTargetPlatform) {
      case TargetPlatform.android:
      case TargetPlatform.iOS:
      case TargetPlatform.macOS:
        return null;
      case TargetPlatform.fuchsia:
      case TargetPlatform.linux:
      case TargetPlatform.windows:
        return 'Not supported on ${_platformName(defaultTargetPlatform)}.';
    }
  }

  @override
  Future<Uint8List> compress(Uint8List data, {required int quality}) async {
    final output = await fic.FlutterImageCompress.compressWithList(
      data,
      quality: quality,
      format: fic.CompressFormat.jpeg,
      keepExif: false,
    );
    return Uint8List.fromList(output);
  }

  @override
  Future<BatchRunOutput> compressBatch(
    List<Uint8List> inputs, {
    required int quality,
  }) async {
    var totalOutputBytes = 0;
    var successCount = 0;

    for (final input in inputs) {
      final output = await compress(input, quality: quality);
      totalOutputBytes += output.length;
      successCount += 1;
    }

    return BatchRunOutput(
      totalOutputBytes: totalOutputBytes,
      successCount: successCount,
    );
  }
}

class ImagePackageBenchmarkAdapter extends CompressionBenchmarkAdapter {
  const ImagePackageBenchmarkAdapter();

  @override
  String get id => 'image';

  @override
  String get name => 'image';

  @override
  String get subtitle => 'Popular pure-Dart library';

  @override
  bool get usesNativeBatch => false;

  @override
  String? unsupportedReason() => null;

  @override
  Future<Uint8List> compress(Uint8List data, {required int quality}) {
    return Isolate.run(() {
      final decoded = img.decodeImage(data);
      if (decoded == null) {
        throw const FormatException('image could not decode the sample bytes.');
      }
      return Uint8List.fromList(img.encodeJpg(decoded, quality: quality));
    });
  }

  @override
  Future<BatchRunOutput> compressBatch(
    List<Uint8List> inputs, {
    required int quality,
  }) {
    return Isolate.run(() {
      var totalOutputBytes = 0;
      var successCount = 0;

      for (final input in inputs) {
        final decoded = img.decodeImage(input);
        if (decoded == null) {
          continue;
        }
        totalOutputBytes += img.encodeJpg(decoded, quality: quality).length;
        successCount += 1;
      }

      return BatchRunOutput(
        totalOutputBytes: totalOutputBytes,
        successCount: successCount,
      );
    });
  }
}

Future<ComparisonBenchmarkResult> runCompressionBenchmark(
  Uint8List input, {
  required BenchmarkConfig config,
  void Function(BenchmarkProgress progress)? onProgress,
}) async {
  if (input.isEmpty) {
    throw ArgumentError.value(input, 'input', 'must not be empty');
  }

  final adapters = <CompressionBenchmarkAdapter>[
    const IronpressBenchmarkAdapter(),
    const IronpressFastBenchmarkAdapter(),
    const FlutterImageCompressBenchmarkAdapter(),
    const ImagePackageBenchmarkAdapter(),
  ];

  final availableAdapters = adapters
      .where((adapter) => adapter.unsupportedReason() == null)
      .toList();
  final totalSteps =
      availableAdapters.length *
          (2 + config.singleRuns + 2 + config.batchRuns) +
      1;
  var completedSteps = 0;

  void tick(String message) {
    completedSteps += 1;
    onProgress?.call(
      BenchmarkProgress(
        message: message,
        completedSteps: completedSteps,
        totalSteps: totalSteps,
      ),
    );
  }

  final inputProbe = await Ironpress.probeBytes(input);
  final batchInputs = List<Uint8List>.generate(
    config.batchCount,
    (_) => input,
    growable: false,
  );
  final totalInputBytes = input.length * config.batchCount;

  final packageResults = <PackageBenchmarkResult>[];
  for (final adapter in adapters) {
    final unsupportedReason = adapter.unsupportedReason();
    if (unsupportedReason != null) {
      packageResults.add(
        PackageBenchmarkResult(
          id: adapter.id,
          name: adapter.name,
          subtitle: adapter.subtitle,
          usesNativeBatch: adapter.usesNativeBatch,
          statusMessage: unsupportedReason,
        ),
      );
      continue;
    }

    packageResults.add(
      await _runAdapterBenchmark(
        adapter,
        input: input,
        batchInputs: batchInputs,
        totalInputBytes: totalInputBytes,
        config: config,
        tick: tick,
      ),
    );
  }

  final sweep = await Ironpress.benchmarkBytes(input);
  tick('Completed ironpress quality sweep');

  return ComparisonBenchmarkResult(
    config: config,
    inputProbe: inputProbe,
    packages: packageResults,
    ironpressSweep: sweep,
  );
}

Future<PackageBenchmarkResult> _runAdapterBenchmark(
  CompressionBenchmarkAdapter adapter, {
  required Uint8List input,
  required List<Uint8List> batchInputs,
  required int totalInputBytes,
  required BenchmarkConfig config,
  required void Function(String message) tick,
}) async {
  try {
    tick('Warm up ${adapter.name} single (1/2)');
    await adapter.compress(input, quality: config.quality);
    tick('Warm up ${adapter.name} single (2/2)');
    await adapter.compress(input, quality: config.quality);

    final singleSamples = <_SingleRunSample>[];
    for (var i = 0; i < config.singleRuns; i++) {
      final watch = Stopwatch()..start();
      final output = await adapter.compress(input, quality: config.quality);
      watch.stop();
      singleSamples.add(
        _SingleRunSample(
          elapsedMs: watch.elapsedMicroseconds / 1000.0,
          outputBytes: output.length,
        ),
      );
      tick('Measured ${adapter.name} single run ${i + 1}/${config.singleRuns}');
    }

    tick('Warm up ${adapter.name} batch (1/2)');
    await adapter.compressBatch(batchInputs, quality: config.quality);
    tick('Warm up ${adapter.name} batch (2/2)');
    await adapter.compressBatch(batchInputs, quality: config.quality);

    final batchSamples = <_BatchRunSample>[];
    for (var i = 0; i < config.batchRuns; i++) {
      final watch = Stopwatch()..start();
      final output = await adapter.compressBatch(
        batchInputs,
        quality: config.quality,
      );
      watch.stop();
      batchSamples.add(
        _BatchRunSample(
          elapsedMs: watch.elapsedMicroseconds / 1000.0,
          totalOutputBytes: output.totalOutputBytes,
          successCount: output.successCount,
        ),
      );
      tick('Measured ${adapter.name} batch run ${i + 1}/${config.batchRuns}');
    }

    return PackageBenchmarkResult(
      id: adapter.id,
      name: adapter.name,
      subtitle: adapter.subtitle,
      usesNativeBatch: adapter.usesNativeBatch,
      single: SingleBenchmarkResult(
        outputBytes: _medianInt(
          singleSamples.map((sample) => sample.outputBytes),
        ),
        medianElapsedMs: _medianDouble(
          singleSamples.map((sample) => sample.elapsedMs),
        ),
        sampleElapsedMs: singleSamples
            .map((sample) => sample.elapsedMs)
            .toList(growable: false),
      ),
      batch: BatchBenchmarkResult(
        totalInputBytes: totalInputBytes,
        totalOutputBytes: _medianInt(
          batchSamples.map((sample) => sample.totalOutputBytes),
        ),
        successCount: _medianInt(
          batchSamples.map((sample) => sample.successCount),
        ),
        batchCount: config.batchCount,
        medianElapsedMs: _medianDouble(
          batchSamples.map((sample) => sample.elapsedMs),
        ),
        sampleElapsedMs: batchSamples
            .map((sample) => sample.elapsedMs)
            .toList(growable: false),
      ),
    );
  } on UnsupportedError catch (error) {
    return PackageBenchmarkResult(
      id: adapter.id,
      name: adapter.name,
      subtitle: adapter.subtitle,
      usesNativeBatch: adapter.usesNativeBatch,
      statusMessage: error.message ?? error.toString(),
    );
  } on MissingPluginException catch (error) {
    return PackageBenchmarkResult(
      id: adapter.id,
      name: adapter.name,
      subtitle: adapter.subtitle,
      usesNativeBatch: adapter.usesNativeBatch,
      statusMessage: 'Plugin unavailable: $error',
    );
  } catch (error) {
    return PackageBenchmarkResult(
      id: adapter.id,
      name: adapter.name,
      subtitle: adapter.subtitle,
      usesNativeBatch: adapter.usesNativeBatch,
      statusMessage: 'Benchmark failed: $error',
    );
  }
}

double _medianDouble(Iterable<double> values) {
  final sorted = values.toList()..sort();
  if (sorted.isEmpty) return 0;
  final middle = sorted.length ~/ 2;
  if (sorted.length.isOdd) {
    return sorted[middle];
  }
  return (sorted[middle - 1] + sorted[middle]) / 2;
}

int _medianInt(Iterable<int> values) {
  final sorted = values.toList()..sort();
  if (sorted.isEmpty) return 0;
  final middle = sorted.length ~/ 2;
  if (sorted.length.isOdd) {
    return sorted[middle];
  }
  return ((sorted[middle - 1] + sorted[middle]) / 2).round();
}

String _platformName(TargetPlatform platform) {
  switch (platform) {
    case TargetPlatform.android:
      return 'Android';
    case TargetPlatform.fuchsia:
      return 'Fuchsia';
    case TargetPlatform.iOS:
      return 'iOS';
    case TargetPlatform.linux:
      return 'Linux';
    case TargetPlatform.macOS:
      return 'macOS';
    case TargetPlatform.windows:
      return 'Windows';
  }
}
