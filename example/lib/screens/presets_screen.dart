import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:ironpress/ironpress.dart';

import '../shared/stat_row.dart';
import '../shared/test_image.dart';

class PresetsScreen extends StatefulWidget {
  const PresetsScreen({super.key});

  @override
  State<PresetsScreen> createState() => _PresetsScreenState();
}

class _PresetsScreenState extends State<PresetsScreen> {
  bool _loading = false;
  final _results = <String, CompressResult>{};
  Uint8List? _original;

  @override
  void initState() {
    super.initState();
    _run();
  }

  Future<void> _run() async {
    setState(() => _loading = true);
    try {
      final bytes = await loadTestImage();
      _original = bytes;

      final presets = {
        'Low': CompressPreset.low,
        'Medium': CompressPreset.medium,
        'High': CompressPreset.high,
      };

      for (final entry in presets.entries) {
        final result = await Ironpress.compressBytes(
          bytes,
          preset: entry.value,
          format: CompressFormat.jpeg,
        );
        _results[entry.key] = result;
      }
      setState(() {});
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text('Error: $e')));
      }
    } finally {
      setState(() => _loading = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Quality Presets')),
      body: _loading
          ? const Center(child: CircularProgressIndicator())
          : ListView(
              padding: const EdgeInsets.all(16),
              children: [
                for (final entry in _results.entries)
                  _PresetCard(
                    name: entry.key,
                    result: entry.value,
                    original: _original,
                  ),
              ],
            ),
    );
  }
}

class _PresetCard extends StatelessWidget {
  const _PresetCard({
    required this.name,
    required this.result,
    required this.original,
  });

  final String name;
  final CompressResult result;
  final Uint8List? original;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Card(
      clipBehavior: Clip.antiAlias,
      margin: const EdgeInsets.only(bottom: 12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (result.data != null)
            SizedBox(
              height: 160,
              child: Image.memory(result.data!, fit: BoxFit.cover),
            ),
          Padding(
            padding: const EdgeInsets.all(12),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text('$name Preset', style: theme.textTheme.titleMedium),
                const SizedBox(height: 8),
                StatRow(
                  icon: Icons.storage,
                  label: 'Size',
                  value:
                      '${formatBytes(result.originalSize)} -> ${formatBytes(result.compressedSize)}',
                ),
                StatRow(
                  icon: Icons.compress,
                  label: 'Reduction',
                  value: result.reductionPercent,
                ),
                StatRow(
                  icon: Icons.tune,
                  label: 'Quality',
                  value: 'q${result.qualityUsed}',
                ),
                StatRow(
                  icon: Icons.photo_size_select_large,
                  label: 'Output size',
                  value: '${result.width} x ${result.height}',
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
