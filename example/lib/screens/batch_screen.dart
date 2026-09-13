import 'package:flutter/material.dart';
import 'package:ironpress/ironpress.dart';

import '../shared/stat_row.dart';
import '../shared/test_image.dart';

class BatchScreen extends StatefulWidget {
  const BatchScreen({super.key});

  @override
  State<BatchScreen> createState() => _BatchScreenState();
}

class _BatchScreenState extends State<BatchScreen> {
  double _count = 5;
  bool _loading = false;
  double _progress = 0;
  BatchCompressResult? _batch;
  CancellationToken? _token;

  Future<void> _run() async {
    setState(() {
      _loading = true;
      _progress = 0;
      _batch = null;
    });
    try {
      final bytes = await loadTestImage();
      final inputs = List.generate(
        _count.round(),
        (_) => CompressInput(data: bytes),
      );

      _token = CancellationToken();

      final batch = await Ironpress.compressBatch(
        inputs,
        quality: 80,
        cancellationToken: _token,
        onProgress: (done, total) {
          if (mounted) {
            setState(() => _progress = done / total);
          }
        },
      );
      setState(() => _batch = batch);
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text('Error: $e')));
      }
    } finally {
      setState(() {
        _loading = false;
        _token = null;
      });
    }
  }

  void _cancel() {
    _token?.cancel();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Batch Processing')),
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
                      'Images: ${_count.round()}',
                      style: Theme.of(context).textTheme.titleMedium,
                    ),
                    Slider(
                      value: _count,
                      min: 1,
                      max: 20,
                      divisions: 19,
                      label: _count.round().toString(),
                      onChanged: _loading
                          ? null
                          : (v) => setState(() => _count = v),
                    ),
                    const SizedBox(height: 8),
                    Row(
                      children: [
                        Expanded(
                          child: FilledButton.icon(
                            onPressed: _loading ? null : _run,
                            icon: const Icon(Icons.play_arrow),
                            label: const Text('Start Batch'),
                          ),
                        ),
                        if (_loading) ...[
                          const SizedBox(width: 8),
                          FilledButton.tonalIcon(
                            onPressed: _cancel,
                            icon: const Icon(Icons.stop),
                            label: const Text('Cancel'),
                          ),
                        ],
                      ],
                    ),
                  ],
                ),
              ),
            ),
            if (_loading) ...[
              const SizedBox(height: 16),
              LinearProgressIndicator(value: _progress),
              const SizedBox(height: 8),
              Text(
                '${(_progress * 100).round()}%',
                textAlign: TextAlign.center,
              ),
            ],
            if (_batch != null) ...[
              const SizedBox(height: 16),
              Card(
                child: Padding(
                  padding: const EdgeInsets.all(16),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        'Results',
                        style: Theme.of(context).textTheme.titleMedium,
                      ),
                      const SizedBox(height: 8),
                      StatRow(
                        icon: Icons.timer,
                        label: 'Total time',
                        value: '${_batch!.elapsedMs} ms',
                      ),
                      StatRow(
                        icon: Icons.speed,
                        label: 'Throughput',
                        value:
                            '${_batch!.imagesPerSecond.toStringAsFixed(1)} img/s',
                      ),
                      StatRow(
                        icon: Icons.data_usage,
                        label: 'MB/s',
                        value: _batch!.mbPerSecond.toStringAsFixed(1),
                      ),
                      StatRow(
                        icon: Icons.compress,
                        label: 'Avg reduction',
                        value:
                            '${((1.0 - _batch!.averageRatio) * 100).toStringAsFixed(1)}%',
                      ),
                      StatRow(
                        icon: Icons.check_circle,
                        label: 'Successful',
                        value:
                            '${_batch!.successfulCount} / ${_batch!.results.length}',
                      ),
                      if (_batch!.hasFailures)
                        StatRow(
                          icon: Icons.error,
                          label: 'Failed',
                          value: '${_batch!.failedCount}',
                        ),
                    ],
                  ),
                ),
              ),
              const SizedBox(height: 8),
              ...List.generate(_batch!.results.length, (i) {
                final r = _batch!.results[i];
                return ListTile(
                  leading: CircleAvatar(child: Text('${i + 1}')),
                  title: Text(
                    '${formatBytes(r.originalSize)} -> ${formatBytes(r.compressedSize)}',
                  ),
                  subtitle: Text('q${r.qualityUsed} | ${r.reductionPercent}'),
                  trailing: Icon(
                    r.isSuccess ? Icons.check_circle : Icons.error,
                    color: r.isSuccess ? Colors.green : Colors.red,
                  ),
                );
              }),
            ],
          ],
        ),
      ),
    );
  }
}
