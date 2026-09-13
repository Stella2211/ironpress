import 'dart:async';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:ironpress/ironpress.dart';

import '../shared/image_card.dart';
import '../shared/test_image.dart';

class BasicCompressionScreen extends StatefulWidget {
  const BasicCompressionScreen({super.key});

  @override
  State<BasicCompressionScreen> createState() => _BasicCompressionScreenState();
}

class _BasicCompressionScreenState extends State<BasicCompressionScreen> {
  double _quality = 80;
  bool _loading = false;
  Uint8List? _original;
  CompressResult? _result;

  @override
  void initState() {
    super.initState();
    unawaited(_loadImage());
  }

  Future<void> _loadImage() async {
    final bytes = await loadTestImage();
    setState(() => _original = bytes);
    unawaited(_compress());
  }

  Future<void> _compress() async {
    if (_original == null) return;
    setState(() => _loading = true);
    try {
      final result = await Ironpress.compressBytes(
        _original!,
        quality: _quality.round(),
        format: CompressFormat.jpeg,
      );
      setState(() => _result = result);
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
      appBar: AppBar(title: const Text('Basic Compression')),
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
                      'Quality: ${_quality.round()}',
                      style: Theme.of(context).textTheme.titleMedium,
                    ),
                    Slider(
                      value: _quality,
                      min: 0,
                      max: 100,
                      divisions: 100,
                      label: _quality.round().toString(),
                      onChanged: (v) => setState(() => _quality = v),
                      onChangeEnd: (_) => _compress(),
                    ),
                  ],
                ),
              ),
            ),
            const SizedBox(height: 8),
            if (_loading) const LinearProgressIndicator(),
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
