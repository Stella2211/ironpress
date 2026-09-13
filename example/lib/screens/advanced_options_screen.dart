import 'dart:async';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:ironpress/ironpress.dart';

import '../shared/image_card.dart';
import '../shared/test_image.dart';

class AdvancedOptionsScreen extends StatefulWidget {
  const AdvancedOptionsScreen({super.key});

  @override
  State<AdvancedOptionsScreen> createState() => _AdvancedOptionsScreenState();
}

class _AdvancedOptionsScreenState extends State<AdvancedOptionsScreen> {
  // JPEG options
  bool _progressive = true;
  bool _trellis = true;
  ChromaSubsampling _chroma = ChromaSubsampling.yuv420;

  // PNG options
  double _pngOptLevel = 2;

  bool _loading = false;
  Uint8List? _original;
  CompressResult? _jpegResult;
  CompressResult? _pngResult;

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
    setState(() => _loading = true);
    try {
      final jpegResult = await Ironpress.compressBytes(
        _original!,
        quality: 80,
        format: CompressFormat.jpeg,
        jpeg: JpegOptions(
          progressive: _progressive,
          trellis: _trellis,
          chromaSubsampling: _chroma,
        ),
      );
      final pngResult = await Ironpress.compressBytes(
        _original!,
        format: CompressFormat.png,
        png: PngOptions(optimizationLevel: _pngOptLevel.round()),
      );
      setState(() {
        _jpegResult = jpegResult;
        _pngResult = pngResult;
      });
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
    final theme = Theme.of(context);
    return Scaffold(
      appBar: AppBar(title: const Text('Advanced Options')),
      body: SingleChildScrollView(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            // JPEG section
            Card(
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text('JPEG Options', style: theme.textTheme.titleMedium),
                    const SizedBox(height: 8),
                    SwitchListTile(
                      title: const Text('Progressive'),
                      subtitle: const Text(
                        'Smaller files, progressive loading',
                      ),
                      value: _progressive,
                      onChanged: (v) {
                        setState(() => _progressive = v);
                        _compress();
                      },
                      contentPadding: EdgeInsets.zero,
                    ),
                    SwitchListTile(
                      title: const Text('Trellis quantization'),
                      subtitle: const Text(
                        'mozjpeg\'s killer feature — smaller files at same quality',
                      ),
                      value: _trellis,
                      onChanged: (v) {
                        setState(() => _trellis = v);
                        _compress();
                      },
                      contentPadding: EdgeInsets.zero,
                    ),
                    const SizedBox(height: 8),
                    Text(
                      'Chroma subsampling',
                      style: theme.textTheme.bodyMedium,
                    ),
                    const SizedBox(height: 4),
                    SegmentedButton<ChromaSubsampling>(
                      segments: const [
                        ButtonSegment(
                          value: ChromaSubsampling.yuv420,
                          label: Text('4:2:0'),
                        ),
                        ButtonSegment(
                          value: ChromaSubsampling.yuv422,
                          label: Text('4:2:2'),
                        ),
                        ButtonSegment(
                          value: ChromaSubsampling.yuv444,
                          label: Text('4:4:4'),
                        ),
                      ],
                      selected: {_chroma},
                      onSelectionChanged: (v) {
                        setState(() => _chroma = v.first);
                        _compress();
                      },
                    ),
                  ],
                ),
              ),
            ),
            if (_loading) const LinearProgressIndicator(),
            if (_original != null &&
                _jpegResult != null &&
                _jpegResult!.data != null)
              BeforeAfterCard(
                original: _original!,
                compressed: _jpegResult!.data!,
                result: _jpegResult!,
              ),
            const SizedBox(height: 16),
            // PNG section
            Card(
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text('PNG Options', style: theme.textTheme.titleMedium),
                    const SizedBox(height: 8),
                    Text(
                      'Optimization level: ${_pngOptLevel.round()} (0=fast, 6=max)',
                    ),
                    Slider(
                      value: _pngOptLevel,
                      min: 0,
                      max: 6,
                      divisions: 6,
                      label: _pngOptLevel.round().toString(),
                      onChanged: (v) => setState(() => _pngOptLevel = v),
                      onChangeEnd: (_) => _compress(),
                    ),
                  ],
                ),
              ),
            ),
            if (_original != null &&
                _pngResult != null &&
                _pngResult!.data != null)
              BeforeAfterCard(
                original: _original!,
                compressed: _pngResult!.data!,
                result: _pngResult!,
              ),
          ],
        ),
      ),
    );
  }
}
