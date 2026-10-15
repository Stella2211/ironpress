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
import 'package:ironpress/src/worker_pool.dart';

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

/// CRC-32 (PNG chunk checksum).
int _crc32(List<int> bytes) {
  var crc = 0xFFFFFFFF;
  for (final b in bytes) {
    crc ^= b;
    for (var i = 0; i < 8; i++) {
      crc = (crc & 1) != 0 ? (crc >> 1) ^ 0xEDB88320 : crc >> 1;
    }
  }
  return crc ^ 0xFFFFFFFF;
}

List<int> _be32(int v) => [
  (v >> 24) & 0xFF,
  (v >> 16) & 0xFF,
  (v >> 8) & 0xFF,
  v & 0xFF,
];

List<int> _pngChunk(String type, List<int> data) {
  final body = [...type.codeUnits, ...data];
  return [..._be32(data.length), ...body, ..._be32(_crc32(body))];
}

/// Little-endian TIFF/EXIF block holding only an Orientation tag.
List<int> _exifOrientation(int orientation) => [
  0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00, // II*, IFD0 at 8
  0x01, 0x00, // one entry
  0x12, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, // 0x0112 SHORT x1
  orientation, 0x00, 0x00, 0x00, // value
  0x00, 0x00, 0x00, 0x00, // no next IFD
];

/// 4×2 RGB PNG whose eXIf chunk declares [orientation].
Uint8List _orientedPng(int orientation) {
  const width = 4, height = 2;
  final raw = <int>[];
  for (var y = 0; y < height; y++) {
    raw.add(0); // filter: none
    for (var x = 0; x < width; x++) {
      raw.addAll([x * 60, y * 120, 200]);
    }
  }
  return Uint8List.fromList([
    0x89,
    0x50,
    0x4E,
    0x47,
    0x0D,
    0x0A,
    0x1A,
    0x0A,
    ..._pngChunk('IHDR', [..._be32(width), ..._be32(height), 8, 2, 0, 0, 0]),
    ..._pngChunk('eXIf', _exifOrientation(orientation)),
    ..._pngChunk('IDAT', ZLibCodec().encode(raw)),
    ..._pngChunk('IEND', const []),
  ]);
}

/// Insert an APP1/Exif segment declaring [orientation] after the SOI.
Uint8List _withJpegOrientation(Uint8List jpeg, int orientation) {
  final tiff = _exifOrientation(orientation);
  final length = 2 + 6 + tiff.length;
  return Uint8List.fromList([
    ...jpeg.sublist(0, 2),
    0xFF,
    0xE1,
    (length >> 8) & 0xFF,
    length & 0xFF,
    ...'Exif\x00\x00'.codeUnits,
    ...tiff,
    ...jpeg.sublist(2),
  ]);
}

/// [size]×[size] RGB PNG of pseudo-random noise: slow enough to encode
/// (tens of ms as PNG output) for progress/cancellation tests, and with a
/// large PNG output for buffer-lifetime tests.
Uint8List _noisyPng(int size, {int seed = 1}) {
  var state = seed;
  final raw = <int>[];
  for (var y = 0; y < size; y++) {
    raw.add(0); // filter: none
    for (var x = 0; x < size * 3; x++) {
      state = (state * 1103515245 + 12345) & 0x7FFFFFFF;
      raw.add((state >> 16) & 0xFF);
    }
  }
  return Uint8List.fromList([
    0x89,
    0x50,
    0x4E,
    0x47,
    0x0D,
    0x0A,
    0x1A,
    0x0A,
    ..._pngChunk('IHDR', [..._be32(size), ..._be32(size), 8, 2, 0, 0, 0]),
    ..._pngChunk('IDAT', ZLibCodec(level: 1).encode(raw)),
    ..._pngChunk('IEND', const []),
  ]);
}

// ─── Helper ──────────────────────────────────────────────────────────────────

const bool _nativeRequired = bool.fromEnvironment('IRONPRESS_NATIVE_REQUIRED');

bool _isNativeLibraryUnavailable(StateError error) {
  return error.toString().contains(
    'ironpress: failed to load the native library',
  );
}

void _skipIfNativeLibraryUnavailable(StateError error, StackTrace stackTrace) {
  if (_nativeRequired || !_isNativeLibraryUnavailable(error)) {
    Error.throwWithStackTrace(error, stackTrace);
  }
  markTestSkipped('Native library not available: $error');
}

/// Returns true if the native library loaded successfully.
bool _nativeLibraryAvailable() {
  try {
    Ironpress.nativeVersion;
    return true;
  } on StateError catch (error, stackTrace) {
    _skipIfNativeLibraryUnavailable(error, stackTrace);
    return false;
  }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

Future<T?> _runWithNative<T>(Future<T> Function() action) async {
  try {
    return await action();
  } on StateError catch (error, stackTrace) {
    _skipIfNativeLibraryUnavailable(error, stackTrace);
    return null;
  }
}

void main() {
  group('Integration — compressBytes (JPEG)', () {
    test('produces non-empty output with valid stats', () async {
      CompressResult result;
      try {
        result = await Ironpress.compressBytes(_syntheticJpeg(), quality: 80);
      } on StateError catch (error, stackTrace) {
        // Native library not built for this environment — skip.
        // To run: cd rust && cargo build --release
        //         LD_LIBRARY_PATH=rust/target/release flutter test
        _skipIfNativeLibraryUnavailable(error, stackTrace);
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
      } on StateError catch (error, stackTrace) {
        _skipIfNativeLibraryUnavailable(error, stackTrace);
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
      } on StateError catch (error, stackTrace) {
        _skipIfNativeLibraryUnavailable(error, stackTrace);
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
      } on StateError catch (error, stackTrace) {
        _skipIfNativeLibraryUnavailable(error, stackTrace);
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
      } on StateError catch (error, stackTrace) {
        _skipIfNativeLibraryUnavailable(error, stackTrace);
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
      } on StateError catch (error, stackTrace) {
        _skipIfNativeLibraryUnavailable(error, stackTrace);
        return;
      }

      expect(result.isSuccess, isTrue);
      expect(result.data![0], 0x89); // PNG signature
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
      } on StateError catch (error, stackTrace) {
        _skipIfNativeLibraryUnavailable(error, stackTrace);
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
      } on StateError catch (error, stackTrace) {
        _skipIfNativeLibraryUnavailable(error, stackTrace);
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
      } on StateError catch (error, stackTrace) {
        _skipIfNativeLibraryUnavailable(error, stackTrace);
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
      } on StateError catch (error, stackTrace) {
        _skipIfNativeLibraryUnavailable(error, stackTrace);
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
      } on StateError catch (error, stackTrace) {
        _skipIfNativeLibraryUnavailable(error, stackTrace);
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
      } on StateError catch (error, stackTrace) {
        _skipIfNativeLibraryUnavailable(error, stackTrace);
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
      } on StateError catch (error, stackTrace) {
        _skipIfNativeLibraryUnavailable(error, stackTrace);
        return;
      }

      expect(batch.results.length, 3);
      // Progress callback should have been called at least once.
      expect(progressUpdates.length, greaterThanOrEqualTo(1));
    });
  });

  group('Integration — native version', () {
    test('ABI mismatch is not treated as an unavailable library', () {
      expect(
        () => _skipIfNativeLibraryUnavailable(
          StateError('Ironpress ABI version mismatch: native=0, expected=1.'),
          StackTrace.current,
        ),
        throwsA(isA<StateError>()),
      );
    });

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
        // Items must outlast the ~16 ms progress poll for the cancel issued
        // from the callback to land while the batch is still running.
        const total = 20;
        final input = _noisyPng(384);
        final token = CancellationToken();
        final batch = await _runWithNative(
          () => Ironpress.compressBatch(
            List.generate(total, (_) => CompressInput(data: input)),
            format: CompressFormat.png,
            threadCount: 1,
            cancellationToken: token,
            onProgress: (completed, _) {
              if (completed >= 1) {
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
        expect(batch.results.every((r) => r.isSuccess), isTrue);
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
          } on StateError catch (error, stackTrace) {
            _skipIfNativeLibraryUnavailable(error, stackTrace);
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

    group('Integration - EXIF orientation', () {
      test('orientation is applied and reset in kept EXIF', () async {
        final png = _orientedPng(6);
        final rotated = await _runWithNative(
          () => Ironpress.compressBytes(png, format: CompressFormat.png),
        );
        if (rotated == null) {
          return;
        }

        // Probe reports stored dimensions; the output is display-oriented.
        final probe = await Ironpress.probeBytes(png);
        expect([probe.width, probe.height], [4, 2]);
        expect([rotated.width, rotated.height], [2, 4]);

        final jpeg = await Ironpress.compressBytes(
          _orientedPng(1),
          format: CompressFormat.jpeg,
          quality: 90,
        );
        final kept = await Ironpress.compressBytes(
          _withJpegOrientation(jpeg.data!, 8),
          format: CompressFormat.jpeg,
          keepMetadata: true,
        );
        expect([kept.width, kept.height], [2, 4]);
        expect((await Ironpress.probeBytes(kept.data!)).hasExif, isTrue);

        // Re-compressing the output must not rotate it again.
        final again = await Ironpress.compressBytes(
          kept.data!,
          format: CompressFormat.jpeg,
          keepMetadata: true,
        );
        expect([again.width, again.height], [2, 4]);
      });
    });

    test('nativeVersion matches the packaged native release', () {
      if (!_nativeLibraryAvailable()) {
        return;
      }

      final version = Ironpress.nativeVersion;
      expect(version, '0.3.0');
    });
  });

  group('Integration - worker pool', () {
    late Directory tempDir;

    setUp(() {
      tempDir = Directory.systemTemp.createTempSync('ironpress_pool_');
    });

    tearDown(() {
      tempDir.deleteSync(recursive: true);
    });

    test('concurrent single calls all succeed with ordered results', () async {
      final input = File('${tempDir.path}/input.jpg')
        ..writeAsBytesSync(_syntheticJpeg());
      final results = await _runWithNative(
        () => Future.wait<Object>([
          for (var i = 0; i < 12; i++) ...[
            Ironpress.compressBytes(_syntheticJpeg(), quality: 50 + i),
            Ironpress.compressFile(input.path, quality: 50 + i),
            Ironpress.probeBytes(_syntheticPng()),
          ],
        ]),
      );
      if (results == null) {
        return;
      }

      expect(results.length, 36);
      for (var i = 0; i < results.length; i++) {
        final result = results[i];
        if (i % 3 == 2) {
          expect((result as ImageProbe).format, ImageFormat.png);
        } else {
          expect((result as CompressResult).isSuccess, isTrue);
          expect(result.data, isNotNull);
        }
      }
    });

    test('native errors keep their CompressException type', () async {
      if (!_nativeLibraryAvailable()) {
        return;
      }
      await expectLater(
        Ironpress.compressBytes(Uint8List.fromList([1, 2, 3, 4])),
        throwsA(isA<CompressException>()),
      );
      await expectLater(
        Ironpress.probeFile('${tempDir.path}/missing.jpg'),
        throwsA(isA<CompressException>()),
      );
      await expectLater(
        Ironpress.benchmarkBytes(Uint8List.fromList([9, 9, 9])),
        throwsA(isA<CompressException>()),
      );
      // Workers keep serving requests after errors.
      final result = await Ironpress.compressBytes(_syntheticJpeg());
      expect(result.isSuccess, isTrue);
    });

    test('path-only batch without events runs as one native call', () async {
      final input = File('${tempDir.path}/input.jpg')
        ..writeAsBytesSync(_syntheticJpeg());
      final batch = await _runWithNative(
        () => Ironpress.compressBatch(
          [
            for (var i = 0; i < 9; i++)
              CompressInput(
                path: i == 4 ? '${tempDir.path}/missing.jpg' : input.path,
              ),
          ],
          quality: 80,
          chunkSize: 2,
        ),
      );
      if (batch == null) {
        return;
      }

      expect(batch.results.length, 9);
      for (var i = 0; i < 9; i++) {
        expect(batch.results[i].isSuccess, i != 4, reason: 'item $i');
      }
    });

    for (final chunked in [false, true]) {
      test(
        'byte inputs keep order (${chunked ? 'chunked' : 'bulk'})',
        () async {
          final input = File('${tempDir.path}/input.png')
            ..writeAsBytesSync(_syntheticPng());
          final progress = <int>[];
          final batch = await _runWithNative(
            () => Ironpress.compressBatch(
              [
                for (var i = 0; i < 10; i++)
                  switch (i % 4) {
                    0 => CompressInput(data: _syntheticJpeg()),
                    1 => CompressInput(path: input.path),
                    2 => CompressInput(data: Uint8List.fromList([1, 2, 3])),
                    _ => CompressInput(data: _syntheticPng()),
                  },
              ],
              format: CompressFormat.png,
              chunkSize: 3,
              // A 1-byte budget forces the chunk-by-chunk copy of byte inputs
              // (and runs the images one at a time).
              memoryBudgetBytes: chunked ? 1 : null,
              onProgress: (completed, total) {
                expect(total, 10);
                progress.add(completed);
              },
            ),
          );
          if (batch == null) {
            return;
          }

          expect(progress, isNotEmpty);
          for (var i = 1; i < progress.length; i++) {
            expect(progress[i], greaterThan(progress[i - 1]));
          }
          expect(progress.last, 10);
          expect(batch.results.length, 10);
          for (var i = 0; i < 10; i++) {
            expect(batch.results[i].isSuccess, i % 4 != 2, reason: 'item $i');
          }
        },
      );
    }

    test('streamed byte inputs without events complete', () async {
      final batch = await _runWithNative(
        () => Ironpress.compressBatch([
          for (var i = 0; i < 7; i++) CompressInput(data: _syntheticJpeg()),
          CompressInput(data: Uint8List(0)),
        ], chunkSize: 2),
      );
      if (batch == null) {
        return;
      }

      expect(batch.results.length, 8);
      expect(batch.results.take(7).every((r) => r.isSuccess), isTrue);
      expect(batch.results.last.isSuccess, isFalse);
    });

    test('cancelling a queued batch yields no results', () async {
      if (!_nativeLibraryAvailable()) {
        return;
      }
      final previous = WorkerPool.shared;
      WorkerPool.shared = WorkerPool(maxWorkers: 1);
      try {
        final blocker = Ironpress.benchmarkBytes(_syntheticJpeg());
        final token = CancellationToken();
        final future = Ironpress.compressBatch(
          List.generate(4, (_) => CompressInput(data: _syntheticJpeg())),
          chunkSize: 1,
          cancellationToken: token,
        );
        token.cancel();
        final batch = await future;
        await blocker;
        expect(batch.results, isEmpty);
      } finally {
        WorkerPool.shared = previous;
      }
    });

    test('single-call path batch reports live progress and cancels', () async {
      final input = File('${tempDir.path}/noise.png')
        ..writeAsBytesSync(_noisyPng(384));
      const total = 16;
      final progress = <int>[];
      final token = CancellationToken();
      final batch = await _runWithNative(
        () => Ironpress.compressBatch(
          List.generate(total, (_) => CompressInput(path: input.path)),
          format: CompressFormat.png,
          threadCount: 1,
          // chunkSize no longer splits path batches: progress and
          // cancellation work inside the single native call.
          chunkSize: 1,
          cancellationToken: token,
          onProgress: (completed, _) {
            progress.add(completed);
            if (completed >= 2) {
              token.cancel();
            }
          },
        ),
      );
      if (batch == null) {
        return;
      }

      expect(batch.results.length, inInclusiveRange(2, total - 1));
      expect(batch.results.every((r) => r.isSuccess), isTrue);
      for (var i = 1; i < progress.length; i++) {
        expect(progress[i], greaterThan(progress[i - 1]));
      }
      expect(progress.last, batch.results.length);
    });

    test('progress ends at total without cancellation', () async {
      final input = File('${tempDir.path}/noise.png')
        ..writeAsBytesSync(_noisyPng(256));
      final progress = <int>[];
      final batch = await _runWithNative(
        () => Ironpress.compressBatch(
          List.generate(6, (_) => CompressInput(path: input.path)),
          format: CompressFormat.png,
          threadCount: 2,
          onProgress: (completed, total) {
            expect(total, 6);
            progress.add(completed);
          },
        ),
      );
      if (batch == null) {
        return;
      }

      expect(batch.results.length, 6);
      expect(progress.last, 6);
      expect(progress.where((p) => p == 6).length, 1);
    });

    test('memoryBudgetBytes is accepted and does not change results', () async {
      final inputs = [
        for (var i = 0; i < 4; i++) CompressInput(data: _noisyPng(64, seed: i)),
      ];
      final unbounded = await _runWithNative(
        () => Ironpress.compressBatch(inputs, format: CompressFormat.png),
      );
      if (unbounded == null) {
        return;
      }
      for (final budget in [1, 0, 64 * 1024 * 1024]) {
        final batch = await Ironpress.compressBatch(
          inputs,
          format: CompressFormat.png,
          memoryBudgetBytes: budget,
        );
        expect(batch.results.length, 4);
        for (var i = 0; i < 4; i++) {
          expect(batch.results[i].data, unbounded.results[i].data);
        }
      }
    });

    test('zero-copy outputs match the file output byte for byte', () async {
      final png = _noisyPng(96);
      final outPath = '${tempDir.path}/out.png';
      final inPath = '${tempDir.path}/in.png';
      File(inPath).writeAsBytesSync(png);
      final bytes = await _runWithNative(
        () => Ironpress.compressBytes(png, format: CompressFormat.png),
      );
      if (bytes == null) {
        return;
      }
      await Ironpress.compressFileToFile(
        inPath,
        outPath,
        format: CompressFormat.png,
      );
      final expected = File(outPath).readAsBytesSync();
      expect(bytes.data, expected);
      expect(bytes.compressedSize, expected.length);

      final fromFile = await Ironpress.compressFile(
        inPath,
        format: CompressFormat.png,
      );
      expect(fromFile.data, expected);

      final batch = await Ironpress.compressBatch([
        CompressInput(data: png),
        CompressInput(path: inPath),
      ], format: CompressFormat.png);
      expect(batch.results[0].data, expected);
      expect(batch.results[1].data, expected);
      // The external list is a normal, readable Uint8List.
      expect(batch.results[0].data!.sublist(1, 4), 'PNG'.codeUnits);
    });

    test('released outputs do not accumulate native memory', () async {
      if (!_nativeLibraryAvailable()) {
        return;
      }
      // ~0.8 MB of PNG output per call; 400 calls would leak ~300 MB.
      final png = _noisyPng(512);
      Future<void> churn(int rounds) async {
        for (var i = 0; i < rounds; i++) {
          final result = await Ironpress.compressBytes(
            png,
            format: CompressFormat.png,
            png: const PngOptions(optimizationLevel: 0),
          );
          expect(result.compressedSize, greaterThan(500 * 1024));
        }
      }

      await churn(40); // warm up allocator, workers and caches
      final before = ProcessInfo.currentRss;
      await churn(400);
      final growth = ProcessInfo.currentRss - before;
      expect(
        growth,
        lessThan(100 * 1024 * 1024),
        reason: 'RSS grew by ${growth ~/ (1024 * 1024)} MB',
      );
    });

    test('probe reports the EXIF orientation', () async {
      final png6 = _orientedPng(6);
      final probe = await _runWithNative(() => Ironpress.probeBytes(png6));
      if (probe == null) {
        return;
      }
      expect(probe.orientation, 6);
      expect([probe.width, probe.height], [4, 2]); // stored dimensions

      final file = File('${tempDir.path}/o6.png')..writeAsBytesSync(png6);
      expect((await Ironpress.probeFile(file.path)).orientation, 6);

      final jpeg = await Ironpress.compressBytes(
        _orientedPng(1),
        format: CompressFormat.jpeg,
      );
      final jpeg8 = _withJpegOrientation(jpeg.data!, 8);
      expect((await Ironpress.probeBytes(jpeg8)).orientation, 8);
      expect((await Ironpress.probeBytes(_syntheticJpeg())).orientation, 1);
      expect((await Ironpress.probeBytes(_syntheticPng())).orientation, 1);
    });

    test('idle workers exit and are recreated on the next call', () async {
      if (!_nativeLibraryAvailable()) {
        return;
      }
      final previous = WorkerPool.shared;
      final pool = WorkerPool(idleTimeout: const Duration(milliseconds: 100));
      WorkerPool.shared = pool;
      try {
        await Ironpress.compressBytes(_syntheticJpeg());
        expect(pool.debugWorkerCount, 1);

        final deadline = DateTime.now().add(const Duration(seconds: 10));
        while (pool.debugWorkerCount > 0 && DateTime.now().isBefore(deadline)) {
          await Future<void>.delayed(const Duration(milliseconds: 20));
        }
        expect(pool.debugWorkerCount, 0);

        final result = await Ironpress.compressBytes(_syntheticJpeg());
        expect(result.isSuccess, isTrue);
        expect(pool.debugSpawnCount, 2);
      } finally {
        WorkerPool.shared = previous;
      }
    });
  });
}
