import 'package:flutter/material.dart';
import 'package:ironpress/ironpress.dart';

import '../shared/stat_row.dart';
import '../shared/test_image.dart';

class FormatComparisonScreen extends StatefulWidget {
  const FormatComparisonScreen({super.key});

  @override
  State<FormatComparisonScreen> createState() => _FormatComparisonScreenState();
}

class _FormatComparisonScreenState extends State<FormatComparisonScreen> {
  bool _loading = false;
  final _results = <CompressFormat, CompressResult>{};

  static const _formats = [
    CompressFormat.jpeg,
    CompressFormat.png,
    CompressFormat.webpLossy,
    CompressFormat.webpLossless,
  ];

  @override
  void initState() {
    super.initState();
    _run();
  }

  Future<void> _run() async {
    setState(() => _loading = true);
    try {
      final bytes = await loadTestImage();
      for (final fmt in _formats) {
        final result = await Ironpress.compressBytes(
          bytes,
          quality: 80,
          format: fmt,
        );
        _results[fmt] = result;
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

  String _formatName(CompressFormat fmt) {
    switch (fmt) {
      case CompressFormat.auto:
        return 'Auto';
      case CompressFormat.jpeg:
        return 'JPEG';
      case CompressFormat.png:
        return 'PNG';
      case CompressFormat.webpLossy:
        return 'WebP Lossy';
      case CompressFormat.webpLossless:
        return 'WebP Lossless';
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Format Comparison')),
      body: _loading
          ? const Center(child: CircularProgressIndicator())
          : GridView.count(
              crossAxisCount: 2,
              padding: const EdgeInsets.all(12),
              mainAxisSpacing: 12,
              crossAxisSpacing: 12,
              childAspectRatio: 0.65,
              children: [
                for (final fmt in _formats)
                  if (_results.containsKey(fmt))
                    _FormatCard(name: _formatName(fmt), result: _results[fmt]!),
              ],
            ),
    );
  }
}

class _FormatCard extends StatelessWidget {
  const _FormatCard({required this.name, required this.result});

  final String name;
  final CompressResult result;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Card(
      clipBehavior: Clip.antiAlias,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (result.data != null)
            Expanded(child: Image.memory(result.data!, fit: BoxFit.cover)),
          Padding(
            padding: const EdgeInsets.all(10),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(name, style: theme.textTheme.titleSmall),
                const SizedBox(height: 4),
                StatRow(
                  label: 'Size',
                  value: formatBytes(result.compressedSize),
                ),
                StatRow(label: 'Reduction', value: result.reductionPercent),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
