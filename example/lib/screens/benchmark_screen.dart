import 'package:flutter/material.dart';
import 'package:ironpress/ironpress.dart';

import '../benchmark/compression_benchmark.dart';
import '../shared/stat_row.dart';
import '../shared/test_image.dart';

class BenchmarkScreen extends StatefulWidget {
  const BenchmarkScreen({super.key});

  @override
  State<BenchmarkScreen> createState() => _BenchmarkScreenState();
}

class _BenchmarkScreenState extends State<BenchmarkScreen> {
  static const _singleRuns = 5;
  static const _batchRuns = 3;

  double _quality = 80;
  double _batchCount = 8;
  bool _loading = false;
  double _progress = 0;
  String _status = 'Ready to run';
  ComparisonBenchmarkResult? _result;

  Future<void> _run() async {
    setState(() {
      _loading = true;
      _progress = 0;
      _status = 'Loading sample image';
      _result = null;
    });

    try {
      final bytes = await loadTestImage();
      final result = await runCompressionBenchmark(
        bytes,
        config: BenchmarkConfig(
          quality: _quality.round(),
          batchCount: _batchCount.round(),
          singleRuns: _singleRuns,
          batchRuns: _batchRuns,
        ),
        onProgress: (progress) {
          if (!mounted) return;
          setState(() {
            _progress = progress.fraction;
            _status = progress.message;
          });
        },
      );

      if (!mounted) return;
      setState(() {
        _result = result;
        _progress = 1;
        _status = 'Completed';
      });
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text('Error: $error')));
      }
    } finally {
      if (mounted) {
        setState(() => _loading = false);
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final result = _result;
    final completedPackages = result?.completedPackages.toList() ?? const [];
    final unavailablePackages =
        result?.unavailablePackages.toList() ?? const [];

    return Scaffold(
      appBar: AppBar(title: const Text('Benchmark')),
      body: SingleChildScrollView(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Card(
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      'Fair Comparison Setup',
                      style: theme.textTheme.titleMedium,
                    ),
                    const SizedBox(height: 8),
                    Text(
                      'Same input bytes, same JPEG target, same nominal quality, no resize, no metadata retention.',
                      style: theme.textTheme.bodyMedium,
                    ),
                    const SizedBox(height: 12),
                    Text(
                      'Quality: ${_quality.round()}',
                      style: theme.textTheme.titleSmall,
                    ),
                    Slider(
                      value: _quality,
                      min: 40,
                      max: 95,
                      divisions: 55,
                      label: _quality.round().toString(),
                      onChanged: _loading
                          ? null
                          : (value) => setState(() => _quality = value),
                    ),
                    Text(
                      'Batch images: ${_batchCount.round()}',
                      style: theme.textTheme.titleSmall,
                    ),
                    Slider(
                      value: _batchCount,
                      min: 4,
                      max: 20,
                      divisions: 16,
                      label: _batchCount.round().toString(),
                      onChanged: _loading
                          ? null
                          : (value) => setState(() => _batchCount = value),
                    ),
                    const SizedBox(height: 8),
                    Row(
                      children: [
                        Expanded(
                          child: FilledButton.icon(
                            onPressed: _loading ? null : _run,
                            icon: const Icon(Icons.speed),
                            label: const Text('Run Benchmark'),
                          ),
                        ),
                      ],
                    ),
                    if (!_loading && result == null) ...[
                      const SizedBox(height: 12),
                      Text(
                        'The benchmark is intentionally manual because it is compute-heavy, especially on Android emulators.',
                        style: theme.textTheme.bodySmall?.copyWith(
                          color: theme.colorScheme.onSurfaceVariant,
                        ),
                      ),
                    ],
                  ],
                ),
              ),
            ),
            const SizedBox(height: 12),
            Card(
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text('Methodology', style: theme.textTheme.titleMedium),
                    const SizedBox(height: 8),
                    StatRow(
                      icon: Icons.image,
                      label: 'Output format',
                      value: 'JPEG q${_quality.round()}',
                    ),
                    const StatRow(
                      icon: Icons.aspect_ratio,
                      label: 'Resize policy',
                      value: 'Disabled for all packages',
                    ),
                    const StatRow(
                      icon: Icons.photo,
                      label: 'Metadata',
                      value: 'Removed for all packages',
                    ),
                    const StatRow(
                      icon: Icons.filter_1,
                      label: 'Single benchmark',
                      value: '2 warm-up + 5 measured runs',
                    ),
                    StatRow(
                      icon: Icons.filter_2,
                      label: 'Batch benchmark',
                      value:
                          '${_batchCount.round()} inputs, 2 warm-up + 3 measured runs',
                    ),
                    const StatRow(
                      icon: Icons.info_outline,
                      label: 'ironpress codec',
                      value: 'mozjpeg (optimizes for size)',
                    ),
                    const StatRow(
                      icon: Icons.info_outline,
                      label: 'flutter_image_compress codec',
                      value: 'Platform libjpeg-turbo (optimizes for speed)',
                    ),
                    const SizedBox(height: 8),
                    Text(
                      'Quality numbers are matched, but encoders differ. '
                      'ironpress uses mozjpeg with trellis quantization (smaller output, slower). '
                      'flutter_image_compress uses platform libjpeg-turbo (larger output, faster). '
                      'The "ironpress (fast)" entry disables trellis for a direct speed comparison.',
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: theme.colorScheme.onSurfaceVariant,
                      ),
                    ),
                  ],
                ),
              ),
            ),
            if (_loading) ...[
              const SizedBox(height: 12),
              Card(
                child: Padding(
                  padding: const EdgeInsets.all(16),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text('Running', style: theme.textTheme.titleMedium),
                      const SizedBox(height: 12),
                      LinearProgressIndicator(value: _progress),
                      const SizedBox(height: 8),
                      Text(_status, style: theme.textTheme.bodyMedium),
                    ],
                  ),
                ),
              ),
            ],
            if (result != null) ...[
              const SizedBox(height: 12),
              Card(
                child: Padding(
                  padding: const EdgeInsets.all(16),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text('Input Image', style: theme.textTheme.titleMedium),
                      const SizedBox(height: 8),
                      StatRow(
                        icon: Icons.photo_size_select_large,
                        label: 'Dimensions',
                        value:
                            '${result.inputProbe.width} x ${result.inputProbe.height}',
                      ),
                      StatRow(
                        icon: Icons.storage,
                        label: 'Original size',
                        value: formatBytes(result.inputProbe.fileSize),
                      ),
                      StatRow(
                        icon: Icons.category,
                        label: 'Detected format',
                        value: result.inputProbe.format.name.toUpperCase(),
                      ),
                    ],
                  ),
                ),
              ),
              if (completedPackages.isNotEmpty) ...[
                const SizedBox(height: 12),
                _HighlightsCard(
                  originalBytes: result.inputProbe.fileSize,
                  packages: completedPackages,
                ),
                const SizedBox(height: 16),
                Text(
                  'Single Image Comparison',
                  style: theme.textTheme.titleMedium,
                ),
                const SizedBox(height: 8),
                Card(
                  child: SingleChildScrollView(
                    scrollDirection: Axis.horizontal,
                    child: DataTable(
                      columns: const [
                        DataColumn(label: Text('Package')),
                        DataColumn(label: Text('Output')),
                        DataColumn(label: Text('Reduction')),
                        DataColumn(label: Text('Median time')),
                        DataColumn(label: Text('Efficiency')),
                      ],
                      rows: [
                        for (final package in completedPackages)
                          DataRow(
                            color: _rowColor(
                              theme,
                              package ==
                                  _fastestSinglePackage(completedPackages),
                            ),
                            cells: [
                              DataCell(_PackageLabel(package: package)),
                              DataCell(
                                Text(formatBytes(package.single!.outputBytes)),
                              ),
                              DataCell(
                                Text(
                                  _formatReduction(
                                    package.single!.reductionRatio(
                                      result.inputProbe.fileSize,
                                    ),
                                  ),
                                ),
                              ),
                              DataCell(
                                Text(
                                  '${_formatMs(package.single!.medianElapsedMs)} ms',
                                ),
                              ),
                              DataCell(
                                Text(
                                  '${(package.single!.bytesSavedPerMs(result.inputProbe.fileSize) / 1024).toStringAsFixed(1)} KB/ms',
                                ),
                              ),
                            ],
                          ),
                      ],
                    ),
                  ),
                ),
                const SizedBox(height: 16),
                Text(
                  'Batch Processing Comparison',
                  style: theme.textTheme.titleMedium,
                ),
                const SizedBox(height: 8),
                Card(
                  child: SingleChildScrollView(
                    scrollDirection: Axis.horizontal,
                    child: DataTable(
                      columns: const [
                        DataColumn(label: Text('Package')),
                        DataColumn(label: Text('Mode')),
                        DataColumn(label: Text('Output')),
                        DataColumn(label: Text('Reduction')),
                        DataColumn(label: Text('Median time')),
                        DataColumn(label: Text('Throughput')),
                      ],
                      rows: [
                        for (final package in completedPackages)
                          DataRow(
                            color: _rowColor(
                              theme,
                              package ==
                                  _fastestBatchPackage(completedPackages),
                            ),
                            cells: [
                              DataCell(_PackageLabel(package: package)),
                              DataCell(
                                Text(
                                  package.usesNativeBatch
                                      ? 'Native batch'
                                      : 'Sequential loop',
                                ),
                              ),
                              DataCell(
                                Text(
                                  formatBytes(package.batch!.totalOutputBytes),
                                ),
                              ),
                              DataCell(
                                Text(
                                  _formatReduction(
                                    package.batch!.reductionRatio,
                                  ),
                                ),
                              ),
                              DataCell(
                                Text(
                                  '${_formatMs(package.batch!.medianElapsedMs)} ms',
                                ),
                              ),
                              DataCell(
                                Text(
                                  '${package.batch!.imagesPerSecond.toStringAsFixed(1)} img/s',
                                ),
                              ),
                            ],
                          ),
                      ],
                    ),
                  ),
                ),
              ],
              if (unavailablePackages.isNotEmpty) ...[
                const SizedBox(height: 16),
                Card(
                  child: Padding(
                    padding: const EdgeInsets.all(16),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          'Platform Notes',
                          style: theme.textTheme.titleMedium,
                        ),
                        const SizedBox(height: 8),
                        for (final package in unavailablePackages)
                          Padding(
                            padding: const EdgeInsets.only(bottom: 8),
                            child: Text(
                              '${package.name}: ${package.statusMessage}',
                              style: theme.textTheme.bodyMedium,
                            ),
                          ),
                      ],
                    ),
                  ),
                ),
              ],
              const SizedBox(height: 16),
              Text(
                'ironpress Quality Sweep',
                style: theme.textTheme.titleMedium,
              ),
              const SizedBox(height: 8),
              Card(
                child: Padding(
                  padding: const EdgeInsets.all(16),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      StatRow(
                        icon: Icons.star,
                        label: 'Recommended quality',
                        value: 'q${result.ironpressSweep.recommendedQuality}',
                      ),
                      const SizedBox(height: 8),
                      for (final entry in result.ironpressSweep.entries)
                        _BarRow(
                          entry: entry,
                          maxSize: result.ironpressSweep.originalSize,
                          isRecommended:
                              entry.quality ==
                              result.ironpressSweep.recommendedQuality,
                        ),
                    ],
                  ),
                ),
              ),
              const SizedBox(height: 8),
              Card(
                child: SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  child: DataTable(
                    columns: const [
                      DataColumn(label: Text('Quality')),
                      DataColumn(label: Text('Size')),
                      DataColumn(label: Text('Reduction')),
                      DataColumn(label: Text('Time')),
                    ],
                    rows: [
                      for (final entry in result.ironpressSweep.entries)
                        DataRow(
                          color:
                              entry.quality ==
                                  result.ironpressSweep.recommendedQuality
                              ? WidgetStateProperty.all(
                                  theme.colorScheme.primaryContainer.withAlpha(
                                    80,
                                  ),
                                )
                              : null,
                          cells: [
                            DataCell(Text('q${entry.quality}')),
                            DataCell(Text(entry.sizeFormatted)),
                            DataCell(Text(entry.reductionPercent)),
                            DataCell(Text('${entry.encodeMs} ms')),
                          ],
                        ),
                    ],
                  ),
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

class _HighlightsCard extends StatelessWidget {
  const _HighlightsCard({required this.originalBytes, required this.packages});

  final int originalBytes;
  final List<PackageBenchmarkResult> packages;

  @override
  Widget build(BuildContext context) {
    final fastestSingle = _fastestSinglePackage(packages);
    final smallestSingle = _smallestSinglePackage(packages);
    final fastestBatch = _fastestBatchPackage(packages);
    final mostEfficient = _mostEfficientPackage(packages, originalBytes);

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Highlights', style: Theme.of(context).textTheme.titleMedium),
            const SizedBox(height: 8),
            if (fastestSingle != null)
              StatRow(
                icon: Icons.flash_on,
                label: 'Fastest single encode',
                value:
                    '${fastestSingle.name} (${_formatMs(fastestSingle.single!.medianElapsedMs)} ms)',
              ),
            if (smallestSingle != null)
              StatRow(
                icon: Icons.compress,
                label: 'Smallest single output',
                value:
                    '${smallestSingle.name} (${formatBytes(smallestSingle.single!.outputBytes)}, ${_formatReduction(smallestSingle.single!.reductionRatio(originalBytes))})',
              ),
            if (mostEfficient != null)
              StatRow(
                icon: Icons.balance,
                label: 'Best efficiency (KB saved/ms)',
                value:
                    '${mostEfficient.name} (${(mostEfficient.single!.bytesSavedPerMs(originalBytes) / 1024).toStringAsFixed(1)} KB/ms)',
              ),
            if (fastestBatch != null)
              StatRow(
                icon: Icons.burst_mode,
                label: 'Fastest batch throughput',
                value:
                    '${fastestBatch.name} (${fastestBatch.batch!.imagesPerSecond.toStringAsFixed(1)} img/s)',
              ),
            if (fastestSingle != null &&
                smallestSingle != null &&
                fastestSingle.id != smallestSingle.id)
              StatRow(
                icon: Icons.savings,
                label: 'Size difference',
                value:
                    '${smallestSingle.name} saves ${formatBytes(fastestSingle.single!.outputBytes - smallestSingle.single!.outputBytes)} vs ${fastestSingle.name}',
              ),
          ],
        ),
      ),
    );
  }
}

class _PackageLabel extends StatelessWidget {
  const _PackageLabel({required this.package});

  final PackageBenchmarkResult package;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          package.name,
          style: theme.textTheme.bodyMedium?.copyWith(
            fontWeight: FontWeight.w600,
          ),
        ),
        Text(
          package.subtitle,
          style: theme.textTheme.bodySmall?.copyWith(
            color: theme.colorScheme.onSurfaceVariant,
          ),
        ),
      ],
    );
  }
}

class _BarRow extends StatelessWidget {
  const _BarRow({
    required this.entry,
    required this.maxSize,
    required this.isRecommended,
  });

  final BenchmarkEntry entry;
  final int maxSize;
  final bool isRecommended;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final fraction = maxSize > 0 ? entry.sizeBytes / maxSize : 0.0;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Row(
        children: [
          SizedBox(
            width: 32,
            child: Text(
              'q${entry.quality}',
              style: TextStyle(
                fontSize: 11,
                fontWeight: isRecommended ? FontWeight.bold : FontWeight.normal,
                color: isRecommended ? theme.colorScheme.primary : null,
              ),
            ),
          ),
          const SizedBox(width: 4),
          Expanded(
            child: Stack(
              children: [
                Container(
                  height: 18,
                  decoration: BoxDecoration(
                    color: theme.colorScheme.surfaceContainerHighest,
                    borderRadius: BorderRadius.circular(3),
                  ),
                ),
                FractionallySizedBox(
                  widthFactor: fraction.clamp(0.0, 1.0),
                  child: Container(
                    height: 18,
                    decoration: BoxDecoration(
                      color: isRecommended
                          ? theme.colorScheme.primary
                          : theme.colorScheme.primaryContainer,
                      borderRadius: BorderRadius.circular(3),
                    ),
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(width: 8),
          SizedBox(
            width: 56,
            child: Text(
              entry.sizeFormatted,
              style: const TextStyle(fontSize: 11),
              textAlign: TextAlign.right,
            ),
          ),
        ],
      ),
    );
  }
}

PackageBenchmarkResult? _mostEfficientPackage(
  List<PackageBenchmarkResult> packages,
  int originalBytes,
) {
  if (packages.isEmpty) return null;
  return packages.reduce(
    (best, next) =>
        next.single!.bytesSavedPerMs(originalBytes) >
            best.single!.bytesSavedPerMs(originalBytes)
        ? next
        : best,
  );
}

PackageBenchmarkResult? _fastestSinglePackage(
  List<PackageBenchmarkResult> packages,
) {
  if (packages.isEmpty) return null;
  return packages.reduce(
    (best, next) => next.single!.medianElapsedMs < best.single!.medianElapsedMs
        ? next
        : best,
  );
}

PackageBenchmarkResult? _smallestSinglePackage(
  List<PackageBenchmarkResult> packages,
) {
  if (packages.isEmpty) return null;
  return packages.reduce(
    (best, next) =>
        next.single!.outputBytes < best.single!.outputBytes ? next : best,
  );
}

PackageBenchmarkResult? _fastestBatchPackage(
  List<PackageBenchmarkResult> packages,
) {
  if (packages.isEmpty) return null;
  return packages.reduce(
    (best, next) =>
        next.batch!.imagesPerSecond > best.batch!.imagesPerSecond ? next : best,
  );
}

String _formatReduction(double ratio) =>
    '${((1.0 - ratio) * 100).toStringAsFixed(1)}%';

String _formatMs(double elapsedMs) {
  if (elapsedMs >= 100) {
    return elapsedMs.toStringAsFixed(0);
  }
  if (elapsedMs >= 10) {
    return elapsedMs.toStringAsFixed(1);
  }
  return elapsedMs.toStringAsFixed(2);
}

WidgetStateProperty<Color?>? _rowColor(ThemeData theme, bool highlight) {
  if (!highlight) return null;
  return WidgetStateProperty.all(
    theme.colorScheme.primaryContainer.withAlpha(80),
  );
}
