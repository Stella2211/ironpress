import 'dart:io';

import 'package:flutter/material.dart';
import 'package:ironpress/ironpress.dart';

import '../shared/stat_row.dart';
import '../shared/test_image.dart';

class FileIoScreen extends StatefulWidget {
  const FileIoScreen({super.key});

  @override
  State<FileIoScreen> createState() => _FileIoScreenState();
}

class _FileIoScreenState extends State<FileIoScreen> {
  bool _loading = false;
  CompressResult? _compressFileResult;
  CompressResult? _fileToFileResult;
  String? _tempInputPath;
  String? _tempOutputPath;

  Future<void> _runCompressFile() async {
    setState(() {
      _loading = true;
      _compressFileResult = null;
    });
    try {
      // Write test image to temp file
      final bytes = await loadTestImage();
      final tempDir = Directory.systemTemp;
      final inputFile = File('${tempDir.path}/ironpress_test_input.png');
      await inputFile.writeAsBytes(bytes);
      _tempInputPath = inputFile.path;

      // compressFile: reads from disk, returns bytes in memory
      final result = await Ironpress.compressFile(
        inputFile.path,
        quality: 80,
        preset: CompressPreset.medium,
      );
      setState(() => _compressFileResult = result);
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text('Error: $e')));
      }
    } finally {
      setState(() => _loading = false);
    }
  }

  Future<void> _runFileToFile() async {
    setState(() {
      _loading = true;
      _fileToFileResult = null;
    });
    try {
      final bytes = await loadTestImage();
      final tempDir = Directory.systemTemp;
      final inputFile = File('${tempDir.path}/ironpress_f2f_input.png');
      await inputFile.writeAsBytes(bytes);
      _tempInputPath = inputFile.path;

      final outputPath = '${tempDir.path}/ironpress_f2f_output.jpg';
      _tempOutputPath = outputPath;

      // compressFileToFile: reads from disk, writes to disk, no bytes in memory
      final result = await Ironpress.compressFileToFile(
        inputFile.path,
        outputPath,
        quality: 80,
      );
      setState(() => _fileToFileResult = result);
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
      appBar: AppBar(title: const Text('File I/O')),
      body: SingleChildScrollView(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            // compressFile
            Card(
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text('compressFile()', style: theme.textTheme.titleMedium),
                    const SizedBox(height: 4),
                    Text(
                      'Reads from disk, returns compressed bytes in memory.',
                      style: theme.textTheme.bodySmall,
                    ),
                    const SizedBox(height: 12),
                    FilledButton.icon(
                      onPressed: _loading ? null : _runCompressFile,
                      icon: const Icon(Icons.file_open),
                      label: const Text('Run compressFile'),
                    ),
                    if (_compressFileResult != null) ...[
                      const SizedBox(height: 12),
                      StatRow(
                        icon: Icons.storage,
                        label: 'Original',
                        value: formatBytes(_compressFileResult!.originalSize),
                      ),
                      StatRow(
                        icon: Icons.compress,
                        label: 'Compressed',
                        value: formatBytes(_compressFileResult!.compressedSize),
                      ),
                      StatRow(
                        icon: Icons.percent,
                        label: 'Reduction',
                        value: _compressFileResult!.reductionPercent,
                      ),
                      StatRow(
                        icon: Icons.memory,
                        label: 'Has data in memory',
                        value: _compressFileResult!.data != null ? 'Yes' : 'No',
                      ),
                      StatRow(
                        icon: Icons.check,
                        label: 'isFileOutput',
                        value: '${_compressFileResult!.isFileOutput}',
                      ),
                      if (_tempInputPath != null)
                        Padding(
                          padding: const EdgeInsets.only(top: 8),
                          child: Text(
                            'Input: $_tempInputPath',
                            style: theme.textTheme.bodySmall,
                          ),
                        ),
                    ],
                  ],
                ),
              ),
            ),
            const SizedBox(height: 16),
            // compressFileToFile
            Card(
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      'compressFileToFile()',
                      style: theme.textTheme.titleMedium,
                    ),
                    const SizedBox(height: 4),
                    Text(
                      'Reads from disk, writes compressed output to disk. '
                      'No bytes in memory — ideal for large files.',
                      style: theme.textTheme.bodySmall,
                    ),
                    const SizedBox(height: 12),
                    FilledButton.icon(
                      onPressed: _loading ? null : _runFileToFile,
                      icon: const Icon(Icons.save),
                      label: const Text('Run compressFileToFile'),
                    ),
                    if (_fileToFileResult != null) ...[
                      const SizedBox(height: 12),
                      StatRow(
                        icon: Icons.storage,
                        label: 'Original',
                        value: formatBytes(_fileToFileResult!.originalSize),
                      ),
                      StatRow(
                        icon: Icons.compress,
                        label: 'Compressed',
                        value: formatBytes(_fileToFileResult!.compressedSize),
                      ),
                      StatRow(
                        icon: Icons.percent,
                        label: 'Reduction',
                        value: _fileToFileResult!.reductionPercent,
                      ),
                      StatRow(
                        icon: Icons.memory,
                        label: 'Has data in memory',
                        value: _fileToFileResult!.data != null ? 'Yes' : 'No',
                      ),
                      StatRow(
                        icon: Icons.check,
                        label: 'isFileOutput',
                        value: '${_fileToFileResult!.isFileOutput}',
                      ),
                      if (_tempInputPath != null)
                        Padding(
                          padding: const EdgeInsets.only(top: 8),
                          child: Text(
                            'Input: $_tempInputPath',
                            style: theme.textTheme.bodySmall,
                          ),
                        ),
                      if (_tempOutputPath != null)
                        Padding(
                          padding: const EdgeInsets.only(top: 4),
                          child: Text(
                            'Output: $_tempOutputPath',
                            style: theme.textTheme.bodySmall,
                          ),
                        ),
                    ],
                  ],
                ),
              ),
            ),
            if (_loading) ...[
              const SizedBox(height: 16),
              const LinearProgressIndicator(),
            ],
          ],
        ),
      ),
    );
  }
}
