import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:ironpress/ironpress.dart';

Future<void> _expectNoArgumentError(Future<Object?> Function() action) async {
  try {
    await action();
  } on ArgumentError catch (error, stackTrace) {
    fail('Unexpected ArgumentError: $error\n$stackTrace');
  } catch (_) {
    // Any later native or loader failure is fine for validation-only tests.
  }
}

void main() {
  group('Input validation', () {
    test('compressFile throws on empty path', () {
      expect(() => Ironpress.compressFile(''), throwsA(isA<ArgumentError>()));
    });

    test('compressFileToFile throws on empty inputPath', () {
      expect(
        () => Ironpress.compressFileToFile('', '/out.jpg'),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('compressFileToFile throws on empty outputPath', () {
      expect(
        () => Ironpress.compressFileToFile('/in.jpg', ''),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('compressBytes throws on empty data', () {
      expect(
        () => Ironpress.compressBytes(Uint8List(0)),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('probeFile throws on empty path', () {
      expect(() => Ironpress.probeFile(''), throwsA(isA<ArgumentError>()));
    });

    test('probeBytes throws on empty data', () {
      expect(
        () => Ironpress.probeBytes(Uint8List(0)),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('benchmarkFile throws on empty path', () {
      expect(() => Ironpress.benchmarkFile(''), throwsA(isA<ArgumentError>()));
    });

    test('benchmarkBytes throws on empty data', () {
      expect(
        () => Ironpress.benchmarkBytes(Uint8List(0)),
        throwsA(isA<ArgumentError>()),
      );
    });
  });

  group('Numeric argument validation', () {
    test('compressBatch throws on negative threadCount', () {
      expect(
        () => Ironpress.compressBatch([
          CompressInput(data: Uint8List(1)),
        ], threadCount: -1),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('compressBatch throws on zero chunkSize', () {
      expect(
        () => Ironpress.compressBatch([
          CompressInput(data: Uint8List(1)),
        ], chunkSize: 0),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('compressBatch throws on negative chunkSize', () {
      expect(
        () => Ironpress.compressBatch([
          CompressInput(data: Uint8List(1)),
        ], chunkSize: -1),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('compressBytes throws on zero maxWidth before native call', () {
      expect(
        () => Ironpress.compressBytes(Uint8List(1), maxWidth: 0),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('compressFile throws on negative maxWidth before native call', () {
      expect(
        () => Ironpress.compressFile('/missing.jpg', maxWidth: -1),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('compressBytes throws on zero maxHeight before native call', () {
      expect(
        () => Ironpress.compressBytes(Uint8List(1), maxHeight: 0),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('compressFile throws on negative maxHeight before native call', () {
      expect(
        () => Ironpress.compressFile('/missing.jpg', maxHeight: -1),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('compressBytes throws on zero maxFileSize before native call', () {
      expect(
        () => Ironpress.compressBytes(Uint8List(1), maxFileSize: 0),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('compressFile throws on negative maxFileSize before native call', () {
      expect(
        () => Ironpress.compressFile('/missing.jpg', maxFileSize: -1),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('compressBytes throws on values larger than native u32', () {
      expect(
        () => Ironpress.compressBytes(Uint8List(1), maxWidth: 0x100000000),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('benchmarkFile validates resize arguments before native call', () {
      expect(
        () => Ironpress.benchmarkFile('/missing.jpg', maxWidth: 0),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('benchmarkBytes validates resize arguments before native call', () {
      expect(
        () => Ironpress.benchmarkBytes(Uint8List(1), maxHeight: -1),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('compressBytes validates png optimization before native call', () {
      expect(
        () => Ironpress.compressBytes(
          Uint8List(1),
          png: const PngOptions(optimizationLevel: 7),
        ),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('compressBytes validates avif speed before native call', () {
      expect(
        () => Ironpress.compressBytes(
          Uint8List(1),
          avif: const AvifOptions(speed: 0),
        ),
        throwsA(isA<ArgumentError>()),
      );
      expect(
        () => Ironpress.compressBytes(
          Uint8List(1),
          avif: const AvifOptions(speed: 11),
        ),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('compressFile validates avif speed before native call', () {
      expect(
        () => Ironpress.compressFile(
          '/photo.jpg',
          avif: const AvifOptions(speed: 42),
        ),
        throwsA(isA<ArgumentError>()),
      );
    });
  });

  group('Quality range validation', () {
    test('compressFile throws on quality > 100', () {
      expect(
        () => Ironpress.compressFile('/photo.jpg', quality: 101),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('compressFile throws on quality < 0', () {
      expect(
        () => Ironpress.compressFile('/photo.jpg', quality: -1),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('compressBytes throws on quality > 100', () {
      expect(
        () => Ironpress.compressBytes(Uint8List(1), quality: 150),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('compressFile throws on minQuality > 100', () {
      expect(
        () => Ironpress.compressFile('/photo.jpg', minQuality: 101),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('compressBatch throws on out-of-range quality', () {
      expect(
        () => Ironpress.compressBatch([
          CompressInput(data: Uint8List(1)),
        ], quality: 200),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('compressFile accepts boundary values 0 and 100', () async {
      await _expectNoArgumentError(
        () => Ironpress.compressFile('/photo.jpg', quality: 0),
      );
      await _expectNoArgumentError(
        () => Ironpress.compressFile('/photo.jpg', quality: 100),
      );
    });
  });

  group('CompressException', () {
    test('creation and toString includes code and message', () {
      const ex = CompressException(-3, 'File not found');
      expect(ex.code, -3);
      expect(ex.message, 'File not found');
      expect(ex.toString(), contains('-3'));
      expect(ex.toString(), contains('File not found'));
    });

    test('hint is non-empty for known error codes', () {
      const codes = [-1, -2, -3, -4, -5, -10, -99, -100];
      for (final code in codes) {
        final ex = CompressException(code, 'msg');
        expect(ex.hint, isNotEmpty, reason: 'code $code should have a hint');
      }
    });

    test('hint is empty for unknown error codes', () {
      const ex = CompressException(-999, 'msg');
      expect(ex.hint, isEmpty);
    });

    test('toString includes hint when known code', () {
      const ex = CompressException(-3, 'File not found');
      expect(ex.toString(), contains('Hint:'));
    });

    test('toString does not include hint for unknown code', () {
      const ex = CompressException(-999, 'msg');
      expect(ex.toString(), isNot(contains('Hint:')));
    });
  });

  group('Batch empty input', () {
    test('compressBatch returns empty result for empty list', () async {
      final result = await Ironpress.compressBatch([]);
      expect(result.results, isEmpty);
      expect(result.elapsedMs, 0);
    });
  });
}
