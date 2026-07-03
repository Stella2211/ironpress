import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:ironpress/ironpress.dart';

void main() {
  group('CompressResult', () {
    test('calculates ratio correctly', () {
      const result = CompressResult(
        data: null,
        originalSize: 1000000,
        compressedSize: 350000,
        width: 1920,
        height: 1080,
        qualityUsed: 80,
        iterations: 1,
        resizedToFit: false,
      );

      expect(result.ratio, closeTo(0.35, 0.001));
      expect(result.reductionPercent, '65.0%');
    });

    test('handles zero original size', () {
      const result = CompressResult(
        data: null,
        originalSize: 0,
        compressedSize: 0,
        width: 0,
        height: 0,
        qualityUsed: 0,
        iterations: 0,
        resizedToFit: false,
      );

      expect(result.ratio, 1.0);
    });

    test('toString produces readable output', () {
      const result = CompressResult(
        data: null,
        originalSize: 4200000,
        compressedSize: 380000,
        width: 4000,
        height: 3000,
        qualityUsed: 80,
        iterations: 1,
        resizedToFit: false,
      );

      final str = result.toString();
      expect(str, contains('4.0 MB'));
      expect(str, contains('371.1 KB'));
      expect(str, contains('q80'));
      expect(str, contains('4000x3000'));
    });

    test('toString includes auto-resized flag', () {
      const result = CompressResult(
        data: null,
        originalSize: 1000000,
        compressedSize: 200000,
        width: 1440,
        height: 1080,
        qualityUsed: 62,
        iterations: 4,
        resizedToFit: true,
      );

      expect(result.toString(), contains('auto-resized'));
      expect(result.toString(), contains('4iter'));
    });

    test('toString includes failure details for batch item errors', () {
      const result = CompressResult(
        data: null,
        originalSize: 1000000,
        compressedSize: 0,
        width: 0,
        height: 0,
        qualityUsed: 0,
        iterations: 0,
        resizedToFit: false,
        errorCode: -10,
        errorMessage: 'decode failed',
      );

      expect(result.isSuccess, isFalse);
      expect(result.toString(), contains('decode failed'));
      expect(result.toString(), contains('(-10)'));
    });

    test('isFileOutput is true for file-to-file success', () {
      const result = CompressResult(
        data: null,
        originalSize: 1000000,
        compressedSize: 200000,
        width: 1920,
        height: 1080,
        qualityUsed: 80,
        iterations: 1,
        resizedToFit: false,
      );

      expect(result.isFileOutput, isTrue);
      expect(result.isSuccess, isTrue);
    });

    test('isFileOutput is false when data is present', () {
      final result = CompressResult(
        data: Uint8List(100),
        originalSize: 1000000,
        compressedSize: 100,
        width: 1920,
        height: 1080,
        qualityUsed: 80,
        iterations: 1,
        resizedToFit: false,
      );

      expect(result.isFileOutput, isFalse);
    });

    test('isFileOutput is false for error results', () {
      const result = CompressResult(
        data: null,
        originalSize: 0,
        compressedSize: 0,
        width: 0,
        height: 0,
        qualityUsed: 0,
        iterations: 0,
        resizedToFit: false,
        errorCode: -10,
        errorMessage: 'failed',
      );

      expect(result.isFileOutput, isFalse);
      expect(result.isSuccess, isFalse);
    });
  });

  group('ChromaSubsampling', () {
    test('enum values match Rust constants', () {
      expect(ChromaSubsampling.yuv420.value, 0);
      expect(ChromaSubsampling.yuv422.value, 1);
      expect(ChromaSubsampling.yuv444.value, 2);
    });
  });

  group('JpegOptions', () {
    test('defaults are production-ready', () {
      const opts = JpegOptions();
      expect(opts.progressive, true);
      expect(opts.trellis, true);
      expect(opts.chromaSubsampling, ChromaSubsampling.yuv420);
    });
  });

  group('PngOptions', () {
    test('default optimization level is balanced', () {
      const opts = PngOptions();
      expect(opts.optimizationLevel, 2);
    });

    test('lossy quantization is opt-in', () {
      expect(const PngOptions().lossy, false);
      expect(const PngOptions(lossy: true).lossy, true);
    });
  });

  group('AvifOptions', () {
    test('default speed is balanced', () {
      expect(const AvifOptions().speed, 6);
    });
  });

  group('CompressInput', () {
    test('accepts path', () {
      const input = CompressInput(path: '/tmp/photo.jpg');
      expect(input.path, '/tmp/photo.jpg');
      expect(input.data, isNull);
    });

    test('accepts data', () {
      final input = CompressInput(data: Uint8List(100));
      expect(input.path, isNull);
      expect(input.data, isNotNull);
      expect(input.data!.length, 100);
    });
  });

  group('CompressFormat', () {
    test('enum values match Rust constants', () {
      expect(CompressFormat.auto.value, 0);
      expect(CompressFormat.jpeg.value, 1);
      expect(CompressFormat.png.value, 2);
      expect(CompressFormat.webpLossless.value, 3);
      expect(CompressFormat.webpLossy.value, 4);
      expect(CompressFormat.avif.value, 5);
    });
  });

  group('ImageFormat', () {
    test('fromValue round-trips', () {
      expect(ImageFormat.fromValue(1), ImageFormat.jpeg);
      expect(ImageFormat.fromValue(2), ImageFormat.png);
      expect(ImageFormat.fromValue(3), ImageFormat.webp);
      expect(ImageFormat.fromValue(5), ImageFormat.avif);
    });

    test('name returns readable string', () {
      expect(ImageFormat.jpeg.name, 'JPEG');
      expect(ImageFormat.png.name, 'PNG');
      expect(ImageFormat.webp.name, 'WebP');
      expect(ImageFormat.avif.name, 'AVIF');
    });

    test('unknown value throws instead of silently defaulting', () {
      expect(() => ImageFormat.fromValue(99), throwsA(isA<StateError>()));
      expect(() => ImageFormat.fromValue(0), throwsA(isA<StateError>()));
    });
  });

  group('ImageProbe', () {
    test('formats megapixels correctly', () {
      const info = ImageProbe(
        width: 4000,
        height: 3000,
        format: ImageFormat.jpeg,
        fileSize: 4200000,
        hasExif: true,
      );

      expect(info.megapixels, closeTo(12.0, 0.01));
      expect(info.pixelCount, 12000000);
      expect(info.toString(), contains('4000x3000'));
      expect(info.toString(), contains('JPEG'));
      expect(info.toString(), contains('4.0 MB'));
      expect(info.toString(), contains('EXIF'));
    });

    test('no EXIF shows cleanly', () {
      const info = ImageProbe(
        width: 800,
        height: 600,
        format: ImageFormat.png,
        fileSize: 50000,
        hasExif: false,
      );

      expect(info.toString(), isNot(contains('EXIF')));
      expect(info.toString(), contains('PNG'));
    });
  });

  group('BenchmarkEntry', () {
    test('formats output correctly', () {
      const entry = BenchmarkEntry(
        quality: 80,
        sizeBytes: 380000,
        ratio: 0.09,
        encodeMs: 38,
      );

      expect(entry.sizeFormatted, '371.1 KB');
      expect(entry.reductionPercent, '91.0%');
      expect(entry.toString(), contains('q80'));
      expect(entry.toString(), contains('38ms'));
    });
  });

  group('BenchmarkResult', () {
    test('toString produces readable table', () {
      const result = BenchmarkResult(
        originalSize: 4200000,
        width: 4000,
        height: 3000,
        format: ImageFormat.jpeg,
        entries: [
          BenchmarkEntry(
            quality: 95,
            sizeBytes: 1800000,
            ratio: 0.43,
            encodeMs: 45,
          ),
          BenchmarkEntry(
            quality: 80,
            sizeBytes: 380000,
            ratio: 0.09,
            encodeMs: 38,
          ),
          BenchmarkEntry(
            quality: 60,
            sizeBytes: 180000,
            ratio: 0.04,
            encodeMs: 32,
          ),
        ],
        recommendedQuality: 80,
      );

      final str = result.toString();
      expect(str, contains('recommended: q80'));
      expect(str, contains('← recommended'));
      expect(str, contains('q95'));
      expect(str, contains('q60'));
    });
  });

  group('BatchCompressResult', () {
    test('calculates aggregate stats', () {
      const result = BatchCompressResult(
        results: [
          CompressResult(
            originalSize: 1000000,
            compressedSize: 100000,
            width: 1920,
            height: 1080,
            qualityUsed: 80,
            iterations: 1,
            resizedToFit: false,
          ),
          CompressResult(
            originalSize: 2000000,
            compressedSize: 200000,
            width: 1920,
            height: 1080,
            qualityUsed: 80,
            iterations: 1,
            resizedToFit: false,
          ),
        ],
        elapsedMs: 1000,
      );

      expect(result.totalOriginalSize, 3000000);
      expect(result.totalCompressedSize, 300000);
      expect(result.averageRatio, closeTo(0.1, 0.001));
      expect(result.imagesPerSecond, closeTo(2.0, 0.01));
    });

    test('tracks failed items', () {
      const result = BatchCompressResult(
        results: [
          CompressResult(
            originalSize: 100,
            compressedSize: 50,
            width: 10,
            height: 10,
            qualityUsed: 80,
            iterations: 1,
            resizedToFit: false,
          ),
          CompressResult(
            originalSize: 100,
            compressedSize: 0,
            width: 0,
            height: 0,
            qualityUsed: 0,
            iterations: 0,
            resizedToFit: false,
            errorCode: -4,
            errorMessage: 'write failed',
          ),
        ],
        elapsedMs: 1000,
      );

      expect(result.successfulCount, 1);
      expect(result.failedCount, 1);
      expect(result.toString(), contains('1 failed'));
    });
  });

  group('CancellationToken', () {
    test('starts not cancelled', () {
      final token = CancellationToken();
      expect(token.isCancelled, isFalse);
    });

    test('cancel sets flag', () {
      final token = CancellationToken();
      token.cancel();
      expect(token.isCancelled, isTrue);
    });

    test('reset clears flag', () {
      final token = CancellationToken();
      token.cancel();
      expect(token.isCancelled, isTrue);
      token.reset();
      expect(token.isCancelled, isFalse);
    });

    test('listeners run once on cancel', () {
      final token = CancellationToken();
      var notifications = 0;
      token.addListener(() {
        notifications++;
      });

      token.cancel();
      token.cancel();

      expect(notifications, 1);
    });

    test('listener disposer unsubscribes callback', () {
      final token = CancellationToken();
      var notifications = 0;
      final dispose = token.addListener(() {
        notifications++;
      });

      dispose();
      token.cancel();

      expect(notifications, 0);
    });
  });

  group('CompressException', () {
    test('toString includes code and message', () {
      const ex = CompressException(-10, 'decode failed');
      expect(ex.toString(), contains('CompressException(-10): decode failed'));
      expect(ex.toString(), contains('Hint:'));
      expect(ex.code, -10);
      expect(ex.message, 'decode failed');
    });
  });
}
