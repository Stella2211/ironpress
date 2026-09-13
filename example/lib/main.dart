import 'package:flutter/material.dart';
import 'package:ironpress/ironpress.dart';

import 'screens/advanced_options_screen.dart';
import 'screens/basic_compression_screen.dart';
import 'screens/batch_screen.dart';
import 'screens/benchmark_screen.dart';
import 'screens/file_io_screen.dart';
import 'screens/format_comparison_screen.dart';
import 'screens/presets_screen.dart';
import 'screens/probe_screen.dart';
import 'screens/target_size_screen.dart';

void main() => runApp(const ExampleApp());

class ExampleApp extends StatelessWidget {
  const ExampleApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'ironpress Example',
      theme: ThemeData(useMaterial3: true, colorSchemeSeed: Colors.indigo),
      home: const HomeScreen(),
    );
  }
}

class _Feature {
  const _Feature({
    required this.title,
    required this.subtitle,
    required this.icon,
    required this.builder,
  });

  final String title;
  final String subtitle;
  final IconData icon;
  final WidgetBuilder builder;
}

final _features = <_Feature>[
  _Feature(
    title: 'Basic Compression',
    subtitle: 'Quality slider with before/after preview',
    icon: Icons.compress,
    builder: (_) => const BasicCompressionScreen(),
  ),
  _Feature(
    title: 'Quality Presets',
    subtitle: 'Low, medium, high side-by-side',
    icon: Icons.tune,
    builder: (_) => const PresetsScreen(),
  ),
  _Feature(
    title: 'Target File Size',
    subtitle: 'Binary search with maxFileSize',
    icon: Icons.straighten,
    builder: (_) => const TargetSizeScreen(),
  ),
  _Feature(
    title: 'Format Comparison',
    subtitle: 'JPEG vs PNG vs WebP',
    icon: Icons.compare,
    builder: (_) => const FormatComparisonScreen(),
  ),
  _Feature(
    title: 'Batch Processing',
    subtitle: 'Progress bar and cancellation',
    icon: Icons.burst_mode,
    builder: (_) => const BatchScreen(),
  ),
  _Feature(
    title: 'Probe Metadata',
    subtitle: 'Read image info without decoding',
    icon: Icons.info_outline,
    builder: (_) => const ProbeScreen(),
  ),
  _Feature(
    title: 'Benchmark',
    subtitle: 'Compare ironpress with popular packages',
    icon: Icons.speed,
    builder: (_) => const BenchmarkScreen(),
  ),
  _Feature(
    title: 'Advanced Options',
    subtitle: 'JpegOptions, PngOptions, ChromaSubsampling',
    icon: Icons.settings,
    builder: (_) => const AdvancedOptionsScreen(),
  ),
  _Feature(
    title: 'File I/O',
    subtitle: 'compressFile and compressFileToFile',
    icon: Icons.folder_open,
    builder: (_) => const FileIoScreen(),
  ),
];

class HomeScreen extends StatelessWidget {
  const HomeScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    String? version;
    try {
      version = Ironpress.nativeVersion;
    } catch (_) {}

    return Scaffold(
      appBar: AppBar(
        title: const Text('ironpress'),
        centerTitle: false,
        bottom: version != null
            ? PreferredSize(
                preferredSize: const Size.fromHeight(20),
                child: Padding(
                  padding: const EdgeInsets.only(left: 16, bottom: 8),
                  child: Align(
                    alignment: Alignment.centerLeft,
                    child: Text(
                      'Native library $version',
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: theme.colorScheme.onSurfaceVariant,
                      ),
                    ),
                  ),
                ),
              )
            : null,
      ),
      body: ListView.separated(
        padding: const EdgeInsets.symmetric(vertical: 8),
        itemCount: _features.length,
        separatorBuilder: (_, _) => const SizedBox(height: 2),
        itemBuilder: (context, index) {
          final f = _features[index];
          return ListTile(
            leading: CircleAvatar(
              backgroundColor: theme.colorScheme.primaryContainer,
              child: Icon(f.icon, color: theme.colorScheme.onPrimaryContainer),
            ),
            title: Text(f.title),
            subtitle: Text(f.subtitle),
            trailing: const Icon(Icons.chevron_right),
            onTap: () =>
                Navigator.push(context, MaterialPageRoute(builder: f.builder)),
          );
        },
      ),
    );
  }
}
