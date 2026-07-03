/// Integration tests that exercise the real native compression library.
///
/// These tests require the Rust library to be compiled and available at
/// runtime (libironpress.so on Linux, ironpress.dll on Windows, etc.).
///
/// In CI, build the native library first and set LD_LIBRARY_PATH:
///   cd rust && cargo build --release
///   LD_LIBRARY_PATH=rust/target/release flutter test test/integration_test.dart
///
/// Tests skip automatically when the native library is not found.
library;

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:ironpress/ironpress.dart';

// ─── Fixtures ────────────────────────────────────────────────────────────────

/// Minimal valid JPEG — 1×1 white pixel.
///
/// Same fixture used in the example app. Enough for all codec paths.
Uint8List _syntheticJpeg() {
  return Uint8List.fromList([
    0xFF,
    0xD8,
    0xFF,
    0xE0,
    0x00,
    0x10,
    0x4A,
    0x46,
    0x49,
    0x46,
    0x00,
    0x01,
    0x01,
    0x00,
    0x00,
    0x01,
    0x00,
    0x01,
    0x00,
    0x00,
    0xFF,
    0xDB,
    0x00,
    0x43,
    0x00,
    0x08,
    0x06,
    0x06,
    0x07,
    0x06,
    0x05,
    0x08,
    0x07,
    0x07,
    0x07,
    0x09,
    0x09,
    0x08,
    0x0A,
    0x0C,
    0x14,
    0x0D,
    0x0C,
    0x0B,
    0x0B,
    0x0C,
    0x19,
    0x12,
    0x13,
    0x0F,
    0x14,
    0x1D,
    0x1A,
    0x1F,
    0x1E,
    0x1D,
    0x1A,
    0x1C,
    0x1C,
    0x20,
    0x24,
    0x2E,
    0x27,
    0x20,
    0x22,
    0x2C,
    0x23,
    0x1C,
    0x1C,
    0x28,
    0x37,
    0x29,
    0x2C,
    0x30,
    0x31,
    0x34,
    0x34,
    0x34,
    0x1F,
    0x27,
    0x39,
    0x3D,
    0x38,
    0x32,
    0x3C,
    0x2E,
    0x33,
    0x34,
    0x32,
    0xFF,
    0xC0,
    0x00,
    0x0B,
    0x08,
    0x00,
    0x01,
    0x00,
    0x01,
    0x01,
    0x01,
    0x11,
    0x00,
    0xFF,
    0xC4,
    0x00,
    0x1F,
    0x00,
    0x00,
    0x01,
    0x05,
    0x01,
    0x01,
    0x01,
    0x01,
    0x01,
    0x01,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0x01,
    0x02,
    0x03,
    0x04,
    0x05,
    0x06,
    0x07,
    0x08,
    0x09,
    0x0A,
    0x0B,
    0xFF,
    0xC4,
    0x00,
    0xB5,
    0x10,
    0x00,
    0x02,
    0x01,
    0x03,
    0x03,
    0x02,
    0x04,
    0x03,
    0x05,
    0x05,
    0x04,
    0x04,
    0x00,
    0x00,
    0x01,
    0x7D,
    0x01,
    0x02,
    0x03,
    0x00,
    0x04,
    0x11,
    0x05,
    0x12,
    0x21,
    0x31,
    0x41,
    0x06,
    0x13,
    0x51,
    0x61,
    0x07,
    0x22,
    0x71,
    0x14,
    0x32,
    0x81,
    0x91,
    0xA1,
    0x08,
    0x23,
    0x42,
    0xB1,
    0xC1,
    0x15,
    0x52,
    0xD1,
    0xF0,
    0x24,
    0x33,
    0x62,
    0x72,
    0x82,
    0x09,
    0x0A,
    0x16,
    0x17,
    0x18,
    0x19,
    0x1A,
    0x25,
    0x26,
    0x27,
    0x28,
    0x29,
    0x2A,
    0x34,
    0x35,
    0x36,
    0x37,
    0x38,
    0x39,
    0x3A,
    0x43,
    0x44,
    0x45,
    0x46,
    0x47,
    0x48,
    0x49,
    0x4A,
    0x53,
    0x54,
    0x55,
    0x56,
    0x57,
    0x58,
    0x59,
    0x5A,
    0x63,
    0x64,
    0x65,
    0x66,
    0x67,
    0x68,
    0x69,
    0x6A,
    0x73,
    0x74,
    0x75,
    0x76,
    0x77,
    0x78,
    0x79,
    0x7A,
    0x83,
    0x84,
    0x85,
    0x86,
    0x87,
    0x88,
    0x89,
    0x8A,
    0x92,
    0x93,
    0x94,
    0x95,
    0x96,
    0x97,
    0x98,
    0x99,
    0x9A,
    0xA2,
    0xA3,
    0xA4,
    0xA5,
    0xA6,
    0xA7,
    0xA8,
    0xA9,
    0xAA,
    0xB2,
    0xB3,
    0xB4,
    0xB5,
    0xB6,
    0xB7,
    0xB8,
    0xB9,
    0xBA,
    0xC2,
    0xC3,
    0xC4,
    0xC5,
    0xC6,
    0xC7,
    0xC8,
    0xC9,
    0xCA,
    0xD2,
    0xD3,
    0xD4,
    0xD5,
    0xD6,
    0xD7,
    0xD8,
    0xD9,
    0xDA,
    0xE1,
    0xE2,
    0xE3,
    0xE4,
    0xE5,
    0xE6,
    0xE7,
    0xE8,
    0xE9,
    0xEA,
    0xF1,
    0xF2,
    0xF3,
    0xF4,
    0xF5,
    0xF6,
    0xF7,
    0xF8,
    0xF9,
    0xFA,
    0xFF,
    0xDA,
    0x00,
    0x08,
    0x01,
    0x01,
    0x00,
    0x00,
    0x3F,
    0x00,
    0x7B,
    0x94,
    0x11,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0xFF,
    0xD9,
  ]);
}

/// Minimal valid 1×1 PNG (white pixel, lossless).
Uint8List _syntheticPng() {
  return Uint8List.fromList([
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
    0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR length + type
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // width=1, height=1
    0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, // 8-bit RGB
    0xDE, // IHDR CRC
    0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, // IDAT length=12 + type
    0x78, 0x9C, 0x63, 0xF8, 0xFF, 0xFF, 0x3F, 0x00, // zlib compressed pixel
    0x05, 0xFE, 0x02, 0xFE, // zlib continued
    0x0D, 0xEF, 0x46, 0xB8, // IDAT CRC
    0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, // IEND length=0 + type
    0xAE, 0x42, 0x60, 0x82, // IEND CRC
  ]);
}

Uint8List _jpegWithExif() {
  final base = _syntheticJpeg();
  return Uint8List.fromList([
    ...base.sublist(0, 2),
    0xFF,
    0xE1,
    0x00,
    0x0A,
    ...'Exif\x00\x00'.codeUnits,
    0x00,
    0x00,
    ...base.sublist(2),
  ]);
}

// ─── Helper ──────────────────────────────────────────────────────────────────

/// Returns true if the native library loaded successfully.
bool _nativeLibraryAvailable() {
  try {
    Ironpress.nativeVersion;
    return true;
  } catch (_) {
    return false;
  }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

Future<T?> _runWithNative<T>(Future<T> Function() action) async {
  try {
    return await action();
  } on StateError catch (error) {
    markTestSkipped('Native library not available: $error');
    return null;
  }
}

void main() {
  group('Integration — compressBytes (JPEG)', () {
    test('produces non-empty output with valid stats', () async {
      CompressResult result;
      try {
        result = await Ironpress.compressBytes(_syntheticJpeg(), quality: 80);
      } on StateError catch (e) {
        // Native library not built for this environment — skip.
        // To run: cd rust && cargo build --release
        //         LD_LIBRARY_PATH=rust/target/release flutter test
        markTestSkipped('Native library not available: $e');
        return;
      }

      expect(result.isSuccess, isTrue);
      expect(result.data, isNotNull);
      expect(result.data!.isNotEmpty, isTrue);
      expect(result.compressedSize, greaterThan(0));
      expect(result.originalSize, greaterThan(0));
      expect(result.qualityUsed, inInclusiveRange(0, 100));
    });

    test('width and height match the 1×1 input', () async {
      CompressResult result;
      try {
        result = await Ironpress.compressBytes(_syntheticJpeg(), quality: 80);
      } on StateError catch (_) {
        markTestSkipped('Native library not available');
        return;
      }

      expect(result.width, 1);
      expect(result.height, 1);
    });

    test('output starts with JPEG magic bytes (FF D8 FF)', () async {
      CompressResult result;
      try {
        result = await Ironpress.compressBytes(
          _syntheticJpeg(),
          quality: 80,
          format: CompressFormat.jpeg,
        );
      } on StateError catch (_) {
        markTestSkipped('Native library not available');
        return;
      }

      final data = result.data!;
      expect(data[0], 0xFF);
      expect(data[1], 0xD8);
      expect(data[2], 0xFF);
    });
  });

  group('Integration — compressBytes (PNG)', () {
    test('PNG input re-encodes successfully', () async {
      CompressResult result;
      try {
        result = await Ironpress.compressBytes(
          _syntheticPng(),
          format: CompressFormat.png,
        );
      } on StateError catch (_) {
        markTestSkipped('Native library not available');
        return;
      }

      expect(result.isSuccess, isTrue);
      expect(result.data, isNotNull);
      expect(result.data!.isNotEmpty, isTrue);
      // PNG signature: 89 50 4E 47
      final data = result.data!;
      expect(data[0], 0x89);
      expect(data[1], 0x50);
      expect(data[2], 0x4E);
      expect(data[3], 0x47);
    });
  });

  group('Integration — format conversion', () {
    test('JPEG input → WebP lossy output', () async {
      CompressResult result;
      try {
        result = await Ironpress.compressBytes(
          _syntheticJpeg(),
          quality: 80,
          format: CompressFormat.webpLossy,
        );
      } on StateError catch (_) {
        markTestSkipped('Native library not available');
        return;
      }

      expect(result.isSuccess, isTrue);
      expect(result.data, isNotNull);
      // WebP header: RIFF....WEBP
      final data = result.data!;
      expect(data[0], 0x52); // R
      expect(data[1], 0x49); // I
      expect(data[2], 0x46); // F
      expect(data[3], 0x46); // F
    });

    test('JPEG input → PNG output', () async {
      CompressResult result;
      try {
        result = await Ironpress.compressBytes(
          _syntheticJpeg(),
          format: CompressFormat.png,
        );
      } on StateError catch (_) {
        markTestSkipped('Native library not available');
        return;
      }

      expect(result.isSuccess, isTrue);
      expect(result.data![0], 0x89); // PNG signature
    });

    test('JPEG input → AVIF output', () async {
      CompressResult result;
      try {
        result = await Ironpress.compressBytes(
          _syntheticJpeg(),
          quality: 60,
          format: CompressFormat.avif,
          avif: const AvifOptions(speed: 10),
        );
      } on StateError catch (_) {
        markTestSkipped('Native library not available');
        return;
      }

      expect(result.isSuccess, isTrue);
      final data = result.data!;
      // ISO-BMFF container: "ftypavif" at offset 4
      expect(String.fromCharCodes(data.sublist(4, 12)), 'ftypavif');
    });
  });

  group('Integration — lossy PNG', () {
    test('lossy PNG output is a valid PNG', () async {
      CompressResult result;
      try {
        result = await Ironpress.compressBytes(
          _syntheticPng(),
          format: CompressFormat.png,
          png: const PngOptions(lossy: true),
        );
      } on StateError catch (_) {
        markTestSkipped('Native library not available');
        return;
      }

      expect(result.isSuccess, isTrue);
      expect(result.data![0], 0x89); // PNG signature
      expect(result.width, greaterThan(0));
    });
  });

  group('Integration — target file size', () {
    test('maxFileSize produces output within the target', () async {
      const targetBytes = 50 * 1024; // 50 KB — well above 1×1 pixel
      CompressResult result;
      try {
        result = await Ironpress.compressBytes(
          _syntheticJpeg(),
          maxFileSize: targetBytes,
        );
      } on StateError catch (_) {
        markTestSkipped('Native library not available');
        return;
      }

      expect(result.isSuccess, isTrue);
      expect(result.compressedSize, lessThanOrEqualTo(targetBytes));
    });
  });

  group('Integration — probe', () {
    test('probeBytes reads correct dimensions from JPEG', () async {
      ImageProbe probe;
      try {
        probe = await Ironpress.probeBytes(_syntheticJpeg());
      } on StateError catch (_) {
        markTestSkipped('Native library not available');
        return;
      }

      expect(probe.width, 1);
      expect(probe.height, 1);
      expect(probe.format, ImageFormat.jpeg);
    });

    test('probeBytes reads correct dimensions from PNG', () async {
      ImageProbe probe;
      try {
        probe = await Ironpress.probeBytes(_syntheticPng());
      } on StateError catch (_) {
        markTestSkipped('Native library not available');
        return;
      }

      expect(probe.width, 1);
      expect(probe.height, 1);
      expect(probe.format, ImageFormat.png);
    });
  });

  group('Integration — compressFile', () {
    late Directory tempDir;

    setUp(() {
      tempDir = Directory.systemTemp.createTempSync('ironpress_test_');
    });

    tearDown(() {
      tempDir.deleteSync(recursive: true);
    });

    test('compresses a JPEG file and returns valid bytes', () async {
      final inputFile = File('${tempDir.path}/input.jpg')
        ..writeAsBytesSync(_syntheticJpeg());

      CompressResult result;
      try {
        result = await Ironpress.compressFile(inputFile.path, quality: 80);
      } on StateError catch (_) {
        markTestSkipped('Native library not available');
        return;
      }

      expect(result.isSuccess, isTrue);
      expect(result.data, isNotNull);
      expect(result.compressedSize, greaterThan(0));
    });

    test('compressFileToFile writes output to disk', () async {
      final inputFile = File('${tempDir.path}/input.jpg')
        ..writeAsBytesSync(_syntheticJpeg());
      final outputPath = '${tempDir.path}/output.jpg';

      CompressResult result;
      try {
        result = await Ironpress.compressFileToFile(
          inputFile.path,
          outputPath,
          quality: 80,
        );
      } on StateError catch (_) {
        markTestSkipped('Native library not available');
        return;
      }

      expect(result.isSuccess, isTrue);
      expect(result.isFileOutput, isTrue);
      expect(result.data, isNull); // file-to-file: no bytes returned
      final outputFile = File(outputPath);
      expect(outputFile.existsSync(), isTrue);
      expect(outputFile.lengthSync(), greaterThan(0));
    });
  });

  group('Integration — batch compression', () {
    test('compressBatch processes multiple items', () async {
      final inputs = [
        CompressInput(data: _syntheticJpeg()),
        CompressInput(data: _syntheticJpeg()),
        CompressInput(data: _syntheticJpeg()),
      ];

      BatchCompressResult batch;
      try {
        batch = await Ironpress.compressBatch(inputs, quality: 80);
      } on StateError catch (_) {
        markTestSkipped('Native library not available');
        return;
      }

      expect(batch.results.length, 3);
      for (final result in batch.results) {
        expect(result.isSuccess, isTrue);
        expect(result.compressedSize, greaterThan(0));
      }
      expect(batch.successfulCount, 3);
      expect(batch.failedCount, 0);
    });

    test('compressBatch reports progress via callback', () async {
      final inputs = List.generate(
        3,
        (_) => CompressInput(data: _syntheticJpeg()),
      );
      final progressUpdates = <int>[];

      BatchCompressResult batch;
      try {
        batch = await Ironpress.compressBatch(
          inputs,
          quality: 80,
          onProgress: (completed, total) {
            progressUpdates.add(completed);
          },
        );
      } on StateError catch (_) {
        markTestSkipped('Native library not available');
        return;
      }

      expect(batch.results.length, 3);
      // Progress callback should have been called at least once.
      expect(progressUpdates.length, greaterThanOrEqualTo(1));
    });
  });

  group('Integration — native version', () {
    group('Integration - batch contracts', () {
      test(
        'progress is monotonic and final completion is emitted once',
        () async {
          final progressUpdates = <MapEntry<int, int>>[];
          final batch = await _runWithNative(
            () => Ironpress.compressBatch(
              List.generate(4, (_) => CompressInput(data: _syntheticJpeg())),
              quality: 80,
              chunkSize: 1,
              onProgress: (completed, total) {
                progressUpdates.add(MapEntry(completed, total));
              },
            ),
          );
          if (batch == null) {
            return;
          }

          expect(batch.results.length, 4);
          expect(progressUpdates, isNotEmpty);

          var previous = 0;
          for (final update in progressUpdates) {
            expect(update.key, greaterThanOrEqualTo(previous));
            expect(update.value, 4);
            previous = update.key;
          }

          expect(progressUpdates.where((update) => update.key == 4).length, 1);
        },
      );

      test('cancellation with progress preserves partial results', () async {
        const total = 20;
        final token = CancellationToken();
        final batch = await _runWithNative(
          () => Ironpress.compressBatch(
            List.generate(total, (_) => CompressInput(data: _syntheticJpeg())),
            quality: 80,
            chunkSize: 1,
            cancellationToken: token,
            onProgress: (completed, _) {
              if (completed == 1) {
                token.cancel();
              }
            },
          ),
        );
        if (batch == null) {
          return;
        }

        expect(batch.results.length, greaterThan(0));
        expect(batch.results.length, lessThan(total));
      });

      test(
        'cancellation without progress stops before full completion',
        () async {
          const total = 50;
          final token = CancellationToken();
          final future = Ironpress.compressBatch(
            List.generate(total, (_) => CompressInput(data: _syntheticJpeg())),
            quality: 80,
            chunkSize: 1,
            cancellationToken: token,
          );
          token.cancel();

          BatchCompressResult batch;
          try {
            batch = await future;
          } on StateError catch (error) {
            markTestSkipped('Native library not available: $error');
            return;
          }

          expect(batch.results.length, lessThan(total));
        },
      );

      test('mixed success and failure results are preserved', () async {
        final batch = await _runWithNative(
          () => Ironpress.compressBatch(
            [
              CompressInput(data: _syntheticJpeg()),
              CompressInput(data: Uint8List.fromList([1, 2, 3, 4])),
              CompressInput(data: _syntheticPng()),
            ],
            quality: 80,
            chunkSize: 1,
          ),
        );
        if (batch == null) {
          return;
        }

        expect(batch.results.length, 3);
        expect(batch.results[0].isSuccess, isTrue);
        expect(batch.results[1].isSuccess, isFalse);
        expect(batch.results[1].errorCode, isNotNull);
        expect(batch.results[2].isSuccess, isTrue);
      });
    });

    group('Integration - batch output paths', () {
      late Directory tempDir;

      setUp(() {
        tempDir = Directory.systemTemp.createTempSync('ironpress_batch_');
      });

      tearDown(() {
        tempDir.deleteSync(recursive: true);
      });

      test('per-item outputPath writes to disk and preserves order', () async {
        final outputPath = '${tempDir.path}/batch-output.jpg';
        final batch = await _runWithNative(
          () => Ironpress.compressBatch(
            [
              CompressInput(data: _syntheticJpeg(), outputPath: outputPath),
              CompressInput(data: _syntheticJpeg()),
            ],
            quality: 80,
            chunkSize: 1,
          ),
        );
        if (batch == null) {
          return;
        }

        expect(batch.results.length, 2);
        expect(batch.results[0].isFileOutput, isTrue);
        expect(batch.results[0].data, isNull);
        expect(File(outputPath).existsSync(), isTrue);
        expect(File(outputPath).lengthSync(), greaterThan(0));
        expect(batch.results[1].data, isNotNull);
      });
    });

    group('Integration - benchmark', () {
      late Directory tempDir;

      setUp(() {
        tempDir = Directory.systemTemp.createTempSync('ironpress_benchmark_');
      });

      tearDown(() {
        tempDir.deleteSync(recursive: true);
      });

      test('benchmarkBytes returns sweep entries', () async {
        final result = await _runWithNative(
          () => Ironpress.benchmarkBytes(_syntheticJpeg()),
        );
        if (result == null) {
          return;
        }

        expect(result.entries, isNotEmpty);
        expect(result.recommendedQuality, inInclusiveRange(0, 100));
      });

      test('benchmarkFile reads a real file path', () async {
        final inputFile = File('${tempDir.path}/input.jpg')
          ..writeAsBytesSync(_syntheticJpeg());
        final result = await _runWithNative(
          () => Ironpress.benchmarkFile(inputFile.path),
        );
        if (result == null) {
          return;
        }

        expect(result.entries, isNotEmpty);
        expect(result.format, ImageFormat.jpeg);
      });
    });

    group('Integration - keepMetadata', () {
      test('JPEG metadata is preserved only for JPEG output', () async {
        final preserved = await _runWithNative(
          () => Ironpress.compressBytes(
            _jpegWithExif(),
            quality: 80,
            format: CompressFormat.jpeg,
            keepMetadata: true,
          ),
        );
        if (preserved == null) {
          return;
        }

        final preservedProbe = await Ironpress.probeBytes(preserved.data!);
        expect(preservedProbe.hasExif, isTrue);

        final dropped = await Ironpress.compressBytes(
          _jpegWithExif(),
          quality: 80,
          format: CompressFormat.jpeg,
          keepMetadata: false,
        );
        final droppedProbe = await Ironpress.probeBytes(dropped.data!);
        expect(droppedProbe.hasExif, isFalse);

        final pngOutput = await Ironpress.compressBytes(
          _jpegWithExif(),
          format: CompressFormat.png,
          keepMetadata: true,
        );
        final pngProbe = await Ironpress.probeBytes(pngOutput.data!);
        expect(pngProbe.format, ImageFormat.png);
        expect(pngProbe.hasExif, isFalse);
      });
    });

    test('nativeVersion returns a non-empty string', () {
      if (!_nativeLibraryAvailable()) {
        markTestSkipped('Native library not available');
        return;
      }

      final version = Ironpress.nativeVersion;
      expect(version, isNotEmpty);
    });
  });
}
