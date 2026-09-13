import 'dart:io';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:ironpress/ironpress.dart';

Future<void> main(List<String> args) async {
  final config = _BenchmarkConfig.parse(args);

  try {
    // Force early native load so failures are obvious before setup work.
    stdout.writeln('ironpress native ${Ironpress.nativeVersion}');
  } on Object catch (error) {
    stderr.writeln('Failed to load ironpress native library: $error');
    exitCode = 1;
    return;
  }

  final corpus = await _prepareCorpus(config);
  if (corpus.isEmpty) {
    stderr.writeln('No benchmark corpus entries were found.');
    exitCode = 1;
    return;
  }

  final summaries = <_WorkloadSummary>[
    await _measureWorkload(
      'compressFile',
      config,
      corpus,
      () => _runCompressFile(corpus, config),
    ),
    await _measureWorkload(
      'compressBytes',
      config,
      corpus,
      () => _runCompressBytes(corpus, config),
    ),
    await _measureWorkload(
      'compressBatch(files)',
      config,
      corpus,
      () => _runCompressBatchFiles(corpus, config),
    ),
    await _measureWorkload(
      'compressBatch(bytes)',
      config,
      corpus,
      () => _runCompressBatchBytes(corpus, config),
    ),
  ];

  stdout.writeln(_buildReport(config, corpus, summaries));
}

class _BenchmarkConfig {
  const _BenchmarkConfig({
    required this.warmupRuns,
    required this.measureRuns,
    required this.batchSize,
    required this.chunkSize,
    required this.threadCount,
    required this.quality,
    this.corpusDir,
  });

  final int warmupRuns;
  final int measureRuns;
  final int batchSize;
  final int chunkSize;
  final int threadCount;
  final int quality;
  final String? corpusDir;

  static _BenchmarkConfig parse(List<String> args) {
    int parsePositive(String name, String value) {
      final parsed = int.tryParse(value);
      if (parsed == null || parsed <= 0) {
        throw ArgumentError.value(value, name, 'must be a positive integer');
      }
      return parsed;
    }

    int parseNonNegative(String name, String value) {
      final parsed = int.tryParse(value);
      if (parsed == null || parsed < 0) {
        throw ArgumentError.value(
          value,
          name,
          'must be a non-negative integer',
        );
      }
      return parsed;
    }

    var warmupRuns = 2;
    var measureRuns = 5;
    var batchSize = 24;
    var chunkSize = 8;
    var threadCount = 0;
    var quality = 80;
    String? corpusDir;

    for (final arg in args) {
      if (!arg.startsWith('--') || !arg.contains('=')) {
        throw ArgumentError(
          'Expected arguments in --name=value form. '
          'Supported flags: --warmup, --runs, --batch-size, '
          '--chunk-size, --thread-count, --quality, --corpus-dir.',
        );
      }

      final separator = arg.indexOf('=');
      final name = arg.substring(2, separator);
      final value = arg.substring(separator + 1);
      switch (name) {
        case 'warmup':
          warmupRuns = parseNonNegative(name, value);
        case 'runs':
          measureRuns = parsePositive(name, value);
        case 'batch-size':
          batchSize = parsePositive(name, value);
        case 'chunk-size':
          chunkSize = parsePositive(name, value);
        case 'thread-count':
          threadCount = parseNonNegative(name, value);
        case 'quality':
          quality = parsePositive(name, value).clamp(1, 100);
        case 'corpus-dir':
          corpusDir = value;
        default:
          throw ArgumentError('Unsupported benchmark argument "--$name".');
      }
    }

    return _BenchmarkConfig(
      warmupRuns: warmupRuns,
      measureRuns: measureRuns,
      batchSize: batchSize,
      chunkSize: chunkSize,
      threadCount: threadCount,
      quality: quality,
      corpusDir: corpusDir,
    );
  }
}

class _CorpusEntry {
  const _CorpusEntry({
    required this.name,
    required this.file,
    required this.bytes,
  });

  final String name;
  final File file;
  final Uint8List bytes;
}

class _RunSample {
  const _RunSample({
    required this.elapsed,
    required this.itemCount,
    required this.outputBytes,
    required this.rssAfterBytes,
    required this.rssDeltaBytes,
  });

  final Duration elapsed;
  final int itemCount;
  final int outputBytes;
  final int rssAfterBytes;
  final int rssDeltaBytes;
}

class _WorkloadSummary {
  const _WorkloadSummary({
    required this.name,
    required this.p50,
    required this.p95,
    required this.itemsPerSecond,
    required this.avgOutputBytesPerItem,
    required this.maxObservedRssBytes,
    required this.maxObservedRssDeltaBytes,
  });

  final String name;
  final Duration p50;
  final Duration p95;
  final double itemsPerSecond;
  final double avgOutputBytesPerItem;
  final int maxObservedRssBytes;
  final int maxObservedRssDeltaBytes;
}

Future<List<_CorpusEntry>> _prepareCorpus(_BenchmarkConfig config) async {
  if (config.corpusDir != null) {
    return _loadUserCorpus(Directory(config.corpusDir!));
  }
  return _buildDefaultCorpus();
}

Future<List<_CorpusEntry>> _loadUserCorpus(Directory dir) async {
  if (!dir.existsSync()) {
    throw ArgumentError.value(
      dir.path,
      'corpusDir',
      'directory does not exist',
    );
  }

  const extensions = <String>{
    '.jpg',
    '.jpeg',
    '.png',
    '.webp',
    '.gif',
    '.bmp',
    '.tif',
    '.tiff',
  };

  final files =
      dir
          .listSync(recursive: true)
          .whereType<File>()
          .where((file) => extensions.contains(_extension(file.path)))
          .toList()
        ..sort((a, b) => a.path.compareTo(b.path));

  final entries = <_CorpusEntry>[];
  for (final file in files) {
    entries.add(
      _CorpusEntry(
        name: file.uri.pathSegments.last,
        file: file,
        bytes: await file.readAsBytes(),
      ),
    );
  }
  return entries;
}

Future<List<_CorpusEntry>> _buildDefaultCorpus() async {
  final tempDir = await Directory.systemTemp.createTemp('ironpress_bench_');
  final sourcePng = File(
    '${Directory.current.path}${Platform.pathSeparator}asset'
    '${Platform.pathSeparator}ironpress.png',
  );
  if (!sourcePng.existsSync()) {
    throw StateError(
      'Default corpus source image is missing at ${sourcePng.path}. '
      'Run from the repository root or pass --corpus-dir=...',
    );
  }

  Future<_CorpusEntry> writeEntry(String name, Uint8List bytes) async {
    final file = File('${tempDir.path}${Platform.pathSeparator}$name');
    await file.writeAsBytes(bytes, flush: true);
    return _CorpusEntry(name: name, file: file, bytes: bytes);
  }

  final sourcePngBytes = await sourcePng.readAsBytes();
  final generatedJpeg = (await Ironpress.compressBytes(
    sourcePngBytes,
    format: CompressFormat.jpeg,
    quality: 90,
    keepMetadata: false,
  )).data!;
  final generatedWebpLossy = (await Ironpress.compressBytes(
    sourcePngBytes,
    format: CompressFormat.webpLossy,
    quality: 80,
  )).data!;
  final generatedWebpLossless = (await Ironpress.compressBytes(
    sourcePngBytes,
    format: CompressFormat.webpLossless,
  )).data!;
  final generatedThumbJpeg = (await Ironpress.compressBytes(
    sourcePngBytes,
    format: CompressFormat.jpeg,
    quality: 82,
    maxWidth: 48,
    maxHeight: 48,
  )).data!;
  final generatedThumbWebp = (await Ironpress.compressBytes(
    sourcePngBytes,
    format: CompressFormat.webpLossy,
    quality: 82,
    maxWidth: 48,
    maxHeight: 48,
  )).data!;

  return <_CorpusEntry>[
    await writeEntry('large_png_alpha.png', sourcePngBytes),
    await writeEntry('generated_photo.jpeg', generatedJpeg),
    await writeEntry('generated_photo_lossy.webp', generatedWebpLossy),
    await writeEntry('generated_photo_lossless.webp', generatedWebpLossless),
    await writeEntry('generated_thumb.jpeg', generatedThumbJpeg),
    await writeEntry('generated_thumb_lossy.webp', generatedThumbWebp),
  ];
}

Future<_WorkloadSummary> _measureWorkload(
  String name,
  _BenchmarkConfig config,
  List<_CorpusEntry> corpus,
  Future<_RunSample> Function() runner,
) async {
  for (var i = 0; i < config.warmupRuns; i++) {
    await runner();
  }

  final measured = <_RunSample>[];
  for (var i = 0; i < config.measureRuns; i++) {
    measured.add(await runner());
  }

  final elapsedUs =
      measured.map((sample) => sample.elapsed.inMicroseconds).toList()..sort();
  final p50Us = _percentile(elapsedUs, 0.5);
  final p95Us = _percentile(elapsedUs, 0.95);
  final avgItems =
      measured.map((sample) => sample.itemCount).reduce((a, b) => a + b) /
      measured.length;
  final avgOutputBytes =
      measured.map((sample) => sample.outputBytes).reduce((a, b) => a + b) /
      measured.length;
  final maxObservedRssBytes = measured
      .map((sample) => sample.rssAfterBytes)
      .reduce(math.max);
  final maxObservedRssDeltaBytes = measured
      .map((sample) => sample.rssDeltaBytes)
      .reduce(math.max);

  return _WorkloadSummary(
    name: name,
    p50: Duration(microseconds: p50Us),
    p95: Duration(microseconds: p95Us),
    itemsPerSecond: avgItems / (p50Us / Duration.microsecondsPerSecond),
    avgOutputBytesPerItem: avgOutputBytes / avgItems,
    maxObservedRssBytes: maxObservedRssBytes,
    maxObservedRssDeltaBytes: maxObservedRssDeltaBytes,
  );
}

Future<_RunSample> _runCompressFile(
  List<_CorpusEntry> corpus,
  _BenchmarkConfig config,
) async {
  final rssBefore = ProcessInfo.currentRss;
  final stopwatch = Stopwatch()..start();
  var outputBytes = 0;
  for (final entry in corpus) {
    final result = await Ironpress.compressFile(
      entry.file.path,
      quality: config.quality,
    );
    outputBytes += result.compressedSize;
  }
  stopwatch.stop();
  final rssAfter = ProcessInfo.currentRss;
  return _RunSample(
    elapsed: stopwatch.elapsed,
    itemCount: corpus.length,
    outputBytes: outputBytes,
    rssAfterBytes: rssAfter,
    rssDeltaBytes: math.max(0, rssAfter - rssBefore),
  );
}

Future<_RunSample> _runCompressBytes(
  List<_CorpusEntry> corpus,
  _BenchmarkConfig config,
) async {
  final rssBefore = ProcessInfo.currentRss;
  final stopwatch = Stopwatch()..start();
  var outputBytes = 0;
  for (final entry in corpus) {
    final result = await Ironpress.compressBytes(
      entry.bytes,
      quality: config.quality,
    );
    outputBytes += result.compressedSize;
  }
  stopwatch.stop();
  final rssAfter = ProcessInfo.currentRss;
  return _RunSample(
    elapsed: stopwatch.elapsed,
    itemCount: corpus.length,
    outputBytes: outputBytes,
    rssAfterBytes: rssAfter,
    rssDeltaBytes: math.max(0, rssAfter - rssBefore),
  );
}

Future<_RunSample> _runCompressBatchFiles(
  List<_CorpusEntry> corpus,
  _BenchmarkConfig config,
) async {
  final inputs = List<CompressInput>.generate(
    config.batchSize,
    (index) => CompressInput(path: corpus[index % corpus.length].file.path),
  );
  final rssBefore = ProcessInfo.currentRss;
  final stopwatch = Stopwatch()..start();
  final batch = await Ironpress.compressBatch(
    inputs,
    quality: config.quality,
    chunkSize: config.chunkSize,
    threadCount: config.threadCount,
  );
  stopwatch.stop();
  final rssAfter = ProcessInfo.currentRss;
  return _RunSample(
    elapsed: stopwatch.elapsed,
    itemCount: inputs.length,
    outputBytes: _sumCompressedBytes(batch),
    rssAfterBytes: rssAfter,
    rssDeltaBytes: math.max(0, rssAfter - rssBefore),
  );
}

Future<_RunSample> _runCompressBatchBytes(
  List<_CorpusEntry> corpus,
  _BenchmarkConfig config,
) async {
  final inputs = List<CompressInput>.generate(
    config.batchSize,
    (index) => CompressInput(data: corpus[index % corpus.length].bytes),
  );
  final rssBefore = ProcessInfo.currentRss;
  final stopwatch = Stopwatch()..start();
  final batch = await Ironpress.compressBatch(
    inputs,
    quality: config.quality,
    chunkSize: config.chunkSize,
    threadCount: config.threadCount,
  );
  stopwatch.stop();
  final rssAfter = ProcessInfo.currentRss;
  return _RunSample(
    elapsed: stopwatch.elapsed,
    itemCount: inputs.length,
    outputBytes: _sumCompressedBytes(batch),
    rssAfterBytes: rssAfter,
    rssDeltaBytes: math.max(0, rssAfter - rssBefore),
  );
}

int _sumCompressedBytes(BatchCompressResult batch) {
  return batch.results
      .where((result) => result.isSuccess)
      .map((result) => result.compressedSize)
      .fold<int>(0, (sum, size) => sum + size);
}

String _buildReport(
  _BenchmarkConfig config,
  List<_CorpusEntry> corpus,
  List<_WorkloadSummary> summaries,
) {
  final totalInputBytes = corpus.fold<int>(
    0,
    (sum, entry) => sum + entry.bytes.length,
  );
  final buffer = StringBuffer()
    ..writeln()
    ..writeln('# ironpress performance benchmark')
    ..writeln()
    ..writeln(
      'Config: warmup=${config.warmupRuns}, runs=${config.measureRuns}, '
      'batchSize=${config.batchSize}, chunkSize=${config.chunkSize}, '
      'threadCount=${config.threadCount}, quality=${config.quality}',
    )
    ..writeln()
    ..writeln(
      'Corpus: ${corpus.length} items, total input ${_formatBytes(totalInputBytes)}',
    );

  if (config.corpusDir == null) {
    buffer
      ..writeln(
        'Using the default repo corpus: the package logo PNG, generated JPEG/WebP '
        'derivatives, and generated thumbnail variants for small-call overhead.',
      )
      ..writeln(
        'Pass `--corpus-dir=/abs/path/to/images` to benchmark a production corpus.',
      );
  } else {
    buffer.writeln('Using corpus directory `${config.corpusDir}`.');
  }

  buffer
    ..writeln()
    ..writeln(
      '| Workload | p50 | p95 | Throughput | Avg output/item | Max RSS | Max RSS delta |',
    )
    ..writeln('|---|---:|---:|---:|---:|---:|---:|');

  for (final summary in summaries) {
    buffer.writeln(
      '| ${summary.name} '
      '| ${_formatDuration(summary.p50)} '
      '| ${_formatDuration(summary.p95)} '
      '| ${summary.itemsPerSecond.toStringAsFixed(1)} img/s '
      '| ${_formatBytes(summary.avgOutputBytesPerItem.round())} '
      '| ${_formatBytes(summary.maxObservedRssBytes)} '
      '| ${_formatBytes(summary.maxObservedRssDeltaBytes)} |',
    );
  }

  buffer
    ..writeln()
    ..writeln('Corpus entries:')
    ..writeln();
  for (final entry in corpus) {
    buffer.writeln('- ${entry.name}: ${_formatBytes(entry.bytes.length)}');
  }

  return buffer.toString();
}

String _formatDuration(Duration duration) {
  if (duration.inMilliseconds >= 1000) {
    return '${(duration.inMicroseconds / 1000000).toStringAsFixed(2)} s';
  }
  if (duration.inMilliseconds > 0) {
    return '${(duration.inMicroseconds / 1000).toStringAsFixed(1)} ms';
  }
  return '${duration.inMicroseconds} us';
}

String _formatBytes(num bytes) {
  if (bytes < 1024) {
    return '${bytes.toStringAsFixed(0)} B';
  }
  if (bytes < 1024 * 1024) {
    return '${(bytes / 1024).toStringAsFixed(1)} KB';
  }
  return '${(bytes / (1024 * 1024)).toStringAsFixed(1)} MB';
}

String _extension(String path) {
  final separator = math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
  final dot = path.lastIndexOf('.');
  if (dot <= separator) {
    return '';
  }
  return path.substring(dot).toLowerCase();
}

int _percentile(List<int> sortedValues, double quantile) {
  if (sortedValues.isEmpty) {
    return 0;
  }
  if (sortedValues.length == 1) {
    return sortedValues.single;
  }
  final position = (sortedValues.length - 1) * quantile;
  final lower = position.floor();
  final upper = position.ceil();
  if (lower == upper) {
    return sortedValues[lower];
  }
  final fraction = position - lower;
  return (sortedValues[lower] +
          ((sortedValues[upper] - sortedValues[lower]) * fraction))
      .round();
}
