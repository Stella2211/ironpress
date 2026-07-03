// High-performance image compression powered by Rust.
//
// Uses mozjpeg (trellis quantization) for JPEG and oxipng for PNG.

export 'src/ironpress_impl.dart' show Ironpress, CompressException;
export 'src/options.dart'
    show
        CompressFormat,
        ChromaSubsampling,
        CompressPreset,
        JpegOptions,
        PngOptions,
        AvifOptions,
        CompressInput,
        CancellationToken,
        CompressResult,
        BatchCompressResult,
        ImageFormat,
        ImageProbe,
        BenchmarkEntry,
        BenchmarkResult;
