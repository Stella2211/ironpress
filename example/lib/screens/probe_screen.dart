import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:ironpress/ironpress.dart';

import '../shared/stat_row.dart';
import '../shared/test_image.dart';

class ProbeScreen extends StatefulWidget {
  const ProbeScreen({super.key});

  @override
  State<ProbeScreen> createState() => _ProbeScreenState();
}

class _ProbeScreenState extends State<ProbeScreen> {
  bool _loading = false;
  Uint8List? _imageBytes;
  ImageProbe? _probe;

  @override
  void initState() {
    super.initState();
    _run();
  }

  Future<void> _run() async {
    setState(() => _loading = true);
    try {
      final bytes = await loadTestImage();
      final probe = await Ironpress.probeBytes(bytes);
      setState(() {
        _imageBytes = bytes;
        _probe = probe;
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
    return Scaffold(
      appBar: AppBar(title: const Text('Probe Metadata')),
      body: _loading
          ? const Center(child: CircularProgressIndicator())
          : _imageBytes == null
          ? const Center(child: Text('No image loaded'))
          : SingleChildScrollView(
              padding: const EdgeInsets.all(16),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Card(
                    clipBehavior: Clip.antiAlias,
                    child: Stack(
                      children: [
                        Image.memory(
                          _imageBytes!,
                          fit: BoxFit.cover,
                          width: double.infinity,
                          height: 250,
                        ),
                        if (_probe != null)
                          Positioned(
                            right: 12,
                            bottom: 12,
                            child: Container(
                              padding: const EdgeInsets.symmetric(
                                horizontal: 10,
                                vertical: 6,
                              ),
                              decoration: BoxDecoration(
                                color: Colors.black54,
                                borderRadius: BorderRadius.circular(6),
                              ),
                              child: Text(
                                '${_probe!.width} x ${_probe!.height}',
                                style: const TextStyle(
                                  color: Colors.white,
                                  fontWeight: FontWeight.bold,
                                ),
                              ),
                            ),
                          ),
                      ],
                    ),
                  ),
                  const SizedBox(height: 16),
                  if (_probe != null)
                    Card(
                      child: Padding(
                        padding: const EdgeInsets.all(16),
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            Text(
                              'Image Metadata',
                              style: Theme.of(context).textTheme.titleMedium,
                            ),
                            const SizedBox(height: 12),
                            StatRow(
                              icon: Icons.image,
                              label: 'Format',
                              value: _probe!.format.name,
                            ),
                            StatRow(
                              icon: Icons.photo_size_select_large,
                              label: 'Dimensions',
                              value: '${_probe!.width} x ${_probe!.height}',
                            ),
                            StatRow(
                              icon: Icons.camera,
                              label: 'Megapixels',
                              value:
                                  '${_probe!.megapixels.toStringAsFixed(2)} MP',
                            ),
                            StatRow(
                              icon: Icons.storage,
                              label: 'File size',
                              value: formatBytes(_probe!.fileSize),
                            ),
                            StatRow(
                              icon: Icons.info_outline,
                              label: 'Has EXIF',
                              value: _probe!.hasExif ? 'Yes' : 'No',
                            ),
                            StatRow(
                              icon: Icons.grid_4x4,
                              label: 'Pixel count',
                              value: '${_probe!.pixelCount}',
                            ),
                          ],
                        ),
                      ),
                    ),
                ],
              ),
            ),
    );
  }
}
