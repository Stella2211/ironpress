import 'dart:async';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:ironpress/ironpress.dart';

import '../shared/image_card.dart';
import '../shared/stat_row.dart';
import '../shared/test_image.dart';

class TargetSizeScreen extends StatefulWidget {
  const TargetSizeScreen({super.key});

  @override
  State<TargetSizeScreen> createState() => _TargetSizeScreenState();
}

class _TargetSizeScreenState extends State<TargetSizeScreen> {
  double _targetKB = 100;
  double _minQuality = 30;
  bool _allowResize = true;
  bool _loading = false;
  int _requestId = 0;
  Uint8List? _original;
  CompressResult? _result;

  @override
  void initState() {
    super.initState();
    unawaited(_loadAndCompress());
  }

  Future<void> _loadAndCompress() async {
    final bytes = await loadTestImage();
    setState(() => _original = bytes);
    unawaited(_compress());
  }

  Future<void> _compress() async {
    if (_original == null) return;
    final requestId = ++_requestId;
    setState(() => _loading = true);
    try {
      final result = await Ironpress.compressBytes(
        _original!,
        format: CompressFormat.jpeg,
        maxFileSize: (_targetKB * 1024).round(),
        minQuality: _minQuality.round(),
        allowResize: _allowResize,
      );
      if (!mounted || requestId != _requestId) {
        return;
      }
      setState(() => _result = result);
    } catch (e) {
      if (mounted && requestId == _requestId) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text('Error: $e')));
      }
    } finally {
      if (mounted && requestId == _requestId) {
        setState(() => _loading = false);
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Target File Size')),
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
                      'Target: ${_targetKB.round()} KB',
                      style: Theme.of(context).textTheme.titleMedium,
                    ),
                    Slider(
                      value: _targetKB,
                      min: 10,
                      max: 500,
                      divisions: 49,
                      label: '${_targetKB.round()} KB',
                      onChanged: (v) => setState(() => _targetKB = v),
                      onChangeEnd: (_) => _compress(),
                    ),
                    const SizedBox(height: 8),
                    Text(
                      'Min quality: ${_minQuality.round()}',
                      style: Theme.of(context).textTheme.bodyMedium,
                    ),
                    Slider(
                      value: _minQuality,
                      min: 0,
                      max: 100,
                      divisions: 100,
                      label: _minQuality.round().toString(),
                      onChanged: (v) => setState(() => _minQuality = v),
                      onChangeEnd: (_) => _compress(),
                    ),
                    SwitchListTile(
                      title: const Text('Allow resize'),
                      subtitle: const Text(
                        'Auto-downscale if quality alone cannot meet target',
                      ),
                      value: _allowResize,
                      onChanged: (v) {
                        setState(() => _allowResize = v);
                        _compress();
                      },
                      contentPadding: EdgeInsets.zero,
                    ),
                  ],
                ),
              ),
            ),
            const SizedBox(height: 8),
            if (_loading) const LinearProgressIndicator(),
            if (_result != null) ...[
              const SizedBox(height: 8),
              Card(
                child: Padding(
                  padding: const EdgeInsets.all(12),
                  child: Column(
                    children: [
                      StatRow(
                        icon: Icons.loop,
                        label: 'Binary search iterations',
                        value: '${_result!.iterations}',
                      ),
                      StatRow(
                        icon: Icons.aspect_ratio,
                        label: 'Auto-resized',
                        value: _result!.resizedToFit ? 'Yes' : 'No',
                      ),
                      StatRow(
                        icon: Icons.check_circle_outline,
                        label: 'Met target',
                        value:
                            _result!.compressedSize <=
                                (_targetKB * 1024).round()
                            ? 'Yes'
                            : 'No',
                      ),
                    ],
                  ),
                ),
              ),
            ],
            if (_original != null && _result != null && _result!.data != null)
              BeforeAfterCard(
                original: _original!,
                compressed: _result!.data!,
                result: _result!,
              ),
          ],
        ),
      ),
    );
  }
}
