#[cfg(test)]
mod tests {
    use crate::compress::*;
    use crate::options::*;
    use std::sync::Arc;

    /// Create a minimal valid JPEG for testing.
    fn minimal_jpeg() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(64, 64, image::Rgb([255, 0, 0]));
        let dynamic = image::DynamicImage::ImageRgb8(img);
        let mut buf = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut buf);
        dynamic
            .write_to(&mut cursor, image::ImageFormat::Jpeg)
            .unwrap();
        buf
    }

    /// Create a minimal valid PNG for testing.
    fn minimal_png() -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(64, 64, image::Rgba([0, 0, 255, 255]));
        let dynamic = image::DynamicImage::ImageRgba8(img);
        let mut buf = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut buf);
        dynamic
            .write_to(&mut cursor, image::ImageFormat::Png)
            .unwrap();
        buf
    }

    /// Create a larger image for more realistic testing.
    fn test_image_jpeg(width: u32, height: u32) -> Vec<u8> {
        let mut img = image::RgbImage::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let r = ((x * 255) / width.max(1)) as u8;
                let g = ((y * 255) / height.max(1)) as u8;
                let b = (((x + y) * 128) / (width + height).max(1)) as u8;
                img.put_pixel(x, y, image::Rgb([r, g, b]));
            }
        }
        let dynamic = image::DynamicImage::ImageRgb8(img);
        let mut buf = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut buf);
        dynamic
            .write_to(&mut cursor, image::ImageFormat::Jpeg)
            .unwrap();
        buf
    }

    fn test_image_png(width: u32, height: u32) -> Vec<u8> {
        let mut img = image::RgbaImage::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let r = ((x * 255) / width.max(1)) as u8;
                let g = ((y * 255) / height.max(1)) as u8;
                let b = (((x + y) * 128) / (width + height).max(1)) as u8;
                img.put_pixel(x, y, image::Rgba([r, g, b, 255]));
            }
        }
        let dynamic = image::DynamicImage::ImageRgba8(img);
        let mut buf = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut buf);
        dynamic
            .write_to(&mut cursor, image::ImageFormat::Png)
            .unwrap();
        buf
    }

    /// Build a JPEG with an APP1/Exif segment for EXIF detection tests.
    fn jpeg_with_exif() -> Vec<u8> {
        let base = minimal_jpeg();
        // Inject a fake APP1/Exif segment right after SOI (FF D8)
        let mut out = Vec::new();
        out.extend_from_slice(&base[..2]); // SOI: FF D8
        // APP1 marker: FF E1
        out.push(0xFF);
        out.push(0xE1);
        // Segment length (includes length field itself): 2 + 6 (Exif\0\0) + 2 (dummy) = 10
        out.push(0x00);
        out.push(0x0A);
        out.extend_from_slice(b"Exif\0\0");
        out.push(0x00);
        out.push(0x00); // dummy TIFF data
        out.extend_from_slice(&base[2..]); // rest of JPEG
        out
    }

    /// Build a PNG with an eXIf chunk for EXIF detection tests.
    fn png_with_exif() -> Vec<u8> {
        let base = minimal_png();
        // Insert an eXIf chunk before IEND
        // Find IEND position (last 12 bytes of a valid PNG: len+type+crc)
        let iend_pos = base.len() - 12;

        let mut out = Vec::new();
        out.extend_from_slice(&base[..iend_pos]);

        // eXIf chunk with 4 bytes of dummy data
        let chunk_data = [0u8; 4];
        let chunk_len = (chunk_data.len() as u32).to_be_bytes();
        out.extend_from_slice(&chunk_len);
        out.extend_from_slice(b"eXIf");
        out.extend_from_slice(&chunk_data);
        // CRC (simplified — not valid but sufficient for detection test)
        out.extend_from_slice(&[0u8; 4]);

        out.extend_from_slice(&base[iend_pos..]); // IEND
        out
    }

    // ─── Format Detection ────────────────────────────────────────────

    #[test]
    fn detect_jpeg_format() {
        let data = minimal_jpeg();
        let fmt = detect_format(&data).unwrap();
        assert_eq!(fmt, DetectedFormat::Jpeg);
    }

    #[test]
    fn detect_png_format() {
        let data = minimal_png();
        let fmt = detect_format(&data).unwrap();
        assert_eq!(fmt, DetectedFormat::Png);
    }

    #[test]
    fn detect_invalid_format() {
        let data = vec![0u8; 100];
        assert!(detect_format(&data).is_err());
    }

    #[test]
    fn detect_too_small() {
        let data = vec![0u8; 3];
        assert!(detect_format(&data).is_err());
    }

    // ─── Basic Compression ───────────────────────────────────────────

    #[test]
    fn compress_jpeg_basic() {
        let input = minimal_jpeg();
        let params = CompressParams::default();
        let result = compress_bytes(&input, &params).unwrap();

        assert!(!result.data.is_empty());
        assert_eq!(result.width, 64);
        assert_eq!(result.height, 64);
        assert_eq!(result.quality_used, 80);
        assert_eq!(result.iterations, 1);
        assert!(!result.resized_to_fit);
    }

    #[test]
    fn compress_png_basic() {
        let input = minimal_png();
        let params = CompressParams::default();
        let result = compress_bytes(&input, &params).unwrap();

        assert!(!result.data.is_empty());
        assert_eq!(result.width, 64);
        assert_eq!(result.height, 64);
    }

    #[test]
    fn compress_different_qualities() {
        let input = test_image_jpeg(256, 256);

        let low_q = {
            let mut p = CompressParams::default();
            p.quality = 20;
            compress_bytes(&input, &p).unwrap()
        };

        let high_q = {
            let mut p = CompressParams::default();
            p.quality = 95;
            compress_bytes(&input, &p).unwrap()
        };

        assert!(
            low_q.data.len() < high_q.data.len(),
            "q20 ({}) should be smaller than q95 ({})",
            low_q.data.len(),
            high_q.data.len()
        );
    }

    // ─── JPEG Preset Branches ────────────────────────────────────────

    #[test]
    fn jpeg_progressive_trellis() {
        let input = minimal_jpeg();
        let mut params = CompressParams::default();
        params.jpeg_progressive = 1;
        params.jpeg_trellis = 1;
        let result = compress_bytes(&input, &params).unwrap();
        assert!(!result.data.is_empty());
    }

    #[test]
    fn jpeg_progressive_no_trellis() {
        let input = minimal_jpeg();
        let mut params = CompressParams::default();
        params.jpeg_progressive = 1;
        params.jpeg_trellis = 0;
        let result = compress_bytes(&input, &params).unwrap();
        assert!(!result.data.is_empty());
    }

    #[test]
    fn jpeg_baseline_trellis() {
        let input = minimal_jpeg();
        let mut params = CompressParams::default();
        params.jpeg_progressive = 0;
        params.jpeg_trellis = 1;
        let result = compress_bytes(&input, &params).unwrap();
        assert!(!result.data.is_empty());
    }

    #[test]
    fn jpeg_baseline_no_trellis() {
        let input = minimal_jpeg();
        let mut params = CompressParams::default();
        params.jpeg_progressive = 0;
        params.jpeg_trellis = 0;
        let result = compress_bytes(&input, &params).unwrap();
        assert!(!result.data.is_empty());
    }

    // ─── Resize ──────────────────────────────────────────────────────

    #[test]
    fn resize_constrains_width() {
        let input = test_image_jpeg(400, 300);
        let mut params = CompressParams::default();
        params.max_width = 200;

        let result = compress_bytes(&input, &params).unwrap();

        assert!(
            result.width <= 200,
            "Width {} exceeds max 200",
            result.width
        );
        assert_eq!(result.height, 150);
    }

    #[test]
    fn resize_constrains_height() {
        let input = test_image_jpeg(400, 300);
        let mut params = CompressParams::default();
        params.max_height = 150;

        let result = compress_bytes(&input, &params).unwrap();

        assert!(
            result.height <= 150,
            "Height {} exceeds max 150",
            result.height
        );
        assert_eq!(result.width, 200);
    }

    #[test]
    fn resize_no_upscale() {
        let input = test_image_jpeg(100, 100);
        let mut params = CompressParams::default();
        params.max_width = 500;
        params.max_height = 500;

        let result = compress_bytes(&input, &params).unwrap();

        assert_eq!(result.width, 100);
        assert_eq!(result.height, 100);
    }

    // ─── Target File Size ────────────────────────────────────────────

    #[test]
    fn target_size_achievable() {
        let input = test_image_jpeg(512, 512);
        let target = input.len() / 2;

        let mut params = CompressParams::default();
        params.max_file_size = target as u32;
        params.min_quality = 10;

        let result = compress_bytes(&input, &params).unwrap();

        assert!(
            result.data.len() <= target,
            "Output {} exceeds target {}",
            result.data.len(),
            target
        );
        assert!(
            result.iterations > 1,
            "Should have needed multiple iterations"
        );
    }

    #[test]
    fn target_size_with_auto_resize() {
        let input = test_image_jpeg(512, 512);
        let target: usize = 1024;

        let mut params = CompressParams::default();
        params.max_file_size = target as u32;
        params.min_quality = 10;
        params.allow_resize = 1;

        let result = compress_bytes(&input, &params).unwrap();

        assert!(
            result.data.len() <= target || result.resized_to_fit,
            "Should either hit target or have resized"
        );
    }

    #[test]
    fn target_size_without_resize_returns_best_effort() {
        let input = test_image_jpeg(512, 512);
        let target: usize = 100;

        let mut params = CompressParams::default();
        params.max_file_size = target as u32;
        params.min_quality = 10;
        params.allow_resize = 0;

        let result = compress_bytes(&input, &params).unwrap();

        assert_eq!(result.quality_used, 10);
        assert!(!result.resized_to_fit);
    }

    #[test]
    fn target_size_png_skips_quality_binary_search() {
        let input = test_image_png(512, 512);
        let target: usize = 2048;

        let mut params = CompressParams::default();
        params.max_file_size = target as u32;
        params.min_quality = 10;
        params.allow_resize = 0;

        let result = compress_bytes(&input, &params).unwrap();

        assert!(!result.data.is_empty());
        assert!(
            result.iterations <= 1,
            "PNG target-size should not binary search, got {} iterations",
            result.iterations
        );
    }

    // ─── Format Conversion ───────────────────────────────────────────

    #[test]
    fn jpeg_to_png_conversion() {
        let input = minimal_jpeg();
        let mut params = CompressParams::default();
        params.format = 2;

        let result = compress_bytes(&input, &params).unwrap();

        assert_eq!(&result.data[0..4], &[0x89, 0x50, 0x4E, 0x47]);
    }

    #[test]
    fn png_to_jpeg_conversion() {
        let input = minimal_png();
        let mut params = CompressParams::default();
        params.format = 1;

        let result = compress_bytes(&input, &params).unwrap();

        assert_eq!(&result.data[0..3], &[0xFF, 0xD8, 0xFF]);
    }

    #[test]
    fn keep_metadata_on_png_silently_skips() {
        let input = minimal_png();
        let mut params = CompressParams::default();
        params.keep_metadata = 1;

        // keepMetadata on non-JPEG should succeed (metadata silently dropped)
        let result = compress_bytes(&input, &params).unwrap();
        assert!(!result.data.is_empty());
    }

    #[test]
    fn keep_metadata_on_jpeg_without_exif_still_succeeds() {
        let input = minimal_jpeg();
        let mut params = CompressParams::default();
        params.keep_metadata = 1;

        let result = compress_bytes(&input, &params).unwrap();
        assert!(!result.data.is_empty());
    }

    // ─── Edge Cases ──────────────────────────────────────────────────

    #[test]
    fn empty_input_returns_error() {
        let params = CompressParams::default();
        assert!(compress_bytes(&[], &params).is_err());
    }

    #[test]
    fn garbage_input_returns_error() {
        let params = CompressParams::default();
        assert!(compress_bytes(&[0xFF, 0xD8, 0xFF, 0x00, 0x00], &params).is_err());
    }

    #[test]
    fn quality_zero_still_produces_output() {
        let input = minimal_jpeg();
        let mut params = CompressParams::default();
        params.quality = 0;

        let result = compress_bytes(&input, &params).unwrap();
        assert!(!result.data.is_empty());
    }

    #[test]
    fn quality_hundred_produces_output() {
        let input = minimal_jpeg();
        let mut params = CompressParams::default();
        params.quality = 100;

        let result = compress_bytes(&input, &params).unwrap();
        assert!(!result.data.is_empty());
    }

    // ─── WebP Encoding ───────────────────────────────────────────────

    #[test]
    fn jpeg_to_webp_conversion() {
        let input = minimal_jpeg();
        let mut params = CompressParams::default();
        params.format = 3;

        let result = compress_bytes(&input, &params).unwrap();

        assert!(result.data.len() > 12);
        assert_eq!(&result.data[0..4], b"RIFF");
        assert_eq!(&result.data[8..12], b"WEBP");
    }

    #[test]
    fn png_to_webp_conversion() {
        let input = minimal_png();
        let mut params = CompressParams::default();
        params.format = 3;

        let result = compress_bytes(&input, &params).unwrap();
        assert_eq!(&result.data[0..4], b"RIFF");
        assert_eq!(&result.data[8..12], b"WEBP");
    }

    // ─── EXIF Detection ─────────────────────────────────────────────

    #[test]
    fn detect_exif_in_jpeg_with_exif() {
        let data = jpeg_with_exif();
        let info = probe_bytes(&data).unwrap();
        assert!(info.has_exif, "Should detect injected EXIF in JPEG");
    }

    #[test]
    fn detect_no_exif_in_plain_jpeg() {
        let data = minimal_jpeg();
        let info = probe_bytes(&data).unwrap();
        assert!(!info.has_exif, "Plain JPEG should not have EXIF");
    }

    #[test]
    fn detect_exif_in_png_with_exif_chunk() {
        let data = png_with_exif();
        let info = probe_bytes(&data).unwrap();
        assert!(info.has_exif, "Should detect injected eXIf chunk in PNG");
    }

    #[test]
    fn detect_no_exif_in_plain_png() {
        let data = minimal_png();
        let info = probe_bytes(&data).unwrap();
        assert!(!info.has_exif, "Plain PNG should not have EXIF");
    }

    #[test]
    fn exif_preserved_in_jpeg_roundtrip() {
        let data = jpeg_with_exif();
        let mut params = CompressParams::default();
        params.keep_metadata = 1;

        let result = compress_bytes(&data, &params).unwrap();
        let probe = probe_bytes(&result.data).unwrap();
        assert!(
            probe.has_exif,
            "EXIF should survive JPEG roundtrip with keep_metadata"
        );
    }

    // ─── PNG Optimization Levels ─────────────────────────────────────

    #[test]
    fn png_optimization_level_0() {
        let input = minimal_png();
        let mut params = CompressParams::default();
        params.png_optimization_level = 0;
        let result = compress_bytes(&input, &params).unwrap();
        assert!(!result.data.is_empty());
    }

    #[test]
    fn png_optimization_level_6() {
        let input = minimal_png();
        let mut params = CompressParams::default();
        params.png_optimization_level = 6;
        let result = compress_bytes(&input, &params).unwrap();
        assert!(!result.data.is_empty());
    }

    #[test]
    fn png_optimization_clamped_above_6() {
        let input = minimal_png();
        let mut params = CompressParams::default();
        params.png_optimization_level = 99;
        // Should not panic — clamped to 6
        let result = compress_bytes(&input, &params).unwrap();
        assert!(!result.data.is_empty());
    }

    // ─── Probe ───────────────────────────────────────────────────────

    #[test]
    fn probe_jpeg() {
        let input = test_image_jpeg(400, 300);
        let info = probe_bytes(&input).unwrap();

        assert_eq!(info.width, 400);
        assert_eq!(info.height, 300);
        assert_eq!(info.format, DetectedFormat::Jpeg);
        assert_eq!(info.file_size, input.len());
    }

    #[test]
    fn probe_png() {
        let input = minimal_png();
        let info = probe_bytes(&input).unwrap();

        assert_eq!(info.width, 64);
        assert_eq!(info.height, 64);
        assert_eq!(info.format, DetectedFormat::Png);
    }

    #[test]
    fn probe_empty_returns_error() {
        assert!(probe_bytes(&[]).is_err());
    }

    #[test]
    fn probe_garbage_returns_error() {
        assert!(probe_bytes(&[0x00; 100]).is_err());
    }

    // ─── Benchmark ───────────────────────────────────────────────────

    #[test]
    fn benchmark_jpeg_produces_entries() {
        let input = test_image_jpeg(256, 256);
        let params = CompressParams::default();

        let result = benchmark_bytes(&input, &params).unwrap();

        assert_eq!(result.width, 256);
        assert_eq!(result.height, 256);
        assert_eq!(result.format, DetectedFormat::Jpeg);
        assert_eq!(result.original_size, input.len());

        assert!(
            result.entries.len() >= 5,
            "Got {} entries",
            result.entries.len()
        );
        assert!(result.entries[0].quality > result.entries.last().unwrap().quality);
        assert!(result.entries[0].size_bytes >= result.entries.last().unwrap().size_bytes);
        assert!(result.recommended_quality >= 20 && result.recommended_quality <= 95);
    }

    #[test]
    fn benchmark_png_produces_single_entry() {
        let input = minimal_png();
        let params = CompressParams::default();

        let result = benchmark_bytes(&input, &params).unwrap();
        assert_eq!(result.entries.len(), 1);
    }

    #[test]
    fn benchmark_with_resize() {
        let input = test_image_jpeg(512, 512);
        let mut params = CompressParams::default();
        params.max_width = 128;

        let result = benchmark_bytes(&input, &params).unwrap();

        assert_eq!(result.width, 128);
        assert_eq!(result.height, 128);
    }

    // ─── find_recommended_quality edge cases ─────────────────────────

    #[test]
    fn recommended_quality_single_entry() {
        let entries = vec![crate::compress::BenchmarkEntry {
            quality: 80,
            size_bytes: 50000,
            ratio: 0.5,
            encode_ms: 10,
        }];
        let rec = find_recommended_quality(&entries, 100000);
        assert_eq!(rec, 80);
    }

    #[test]
    fn recommended_quality_empty_entries() {
        let entries: Vec<crate::compress::BenchmarkEntry> = vec![];
        let rec = find_recommended_quality(&entries, 100000);
        assert_eq!(rec, 80); // fallback
    }

    // ─── PreparedPixels / Binary Search Optimization ────────────────

    #[test]
    fn target_size_many_iterations_produces_valid_output() {
        // Force a very small target on a larger image to exercise many
        // binary search iterations + potential resize cycles.
        let input = test_image_jpeg(512, 512);
        let mut params = CompressParams::default();
        params.max_file_size = 2048; // very tight target
        params.min_quality = 5;
        params.allow_resize = 1;

        let result = compress_bytes(&input, &params).unwrap();

        assert!(!result.data.is_empty(), "Should produce output");
        assert!(
            result.iterations > 2,
            "Should need multiple iterations, got {}",
            result.iterations
        );
        // Verify output is valid JPEG
        assert_eq!(&result.data[0..3], &[0xFF, 0xD8, 0xFF]);
    }

    #[test]
    fn target_size_webp_lossy_uses_prepared_pixels() {
        let input = minimal_jpeg();
        let mut params = CompressParams::default();
        params.format = 4; // WebP lossy
        params.max_file_size = 500; // tight target
        params.min_quality = 10;
        params.allow_resize = 1;

        let result = compress_bytes(&input, &params).unwrap();

        assert!(!result.data.is_empty());
        assert_eq!(&result.data[0..4], b"RIFF");
    }

    // ─── ABI Version ────────────────────────────────────────────────

    #[test]
    fn thread_pool_cache_is_keyed_by_effective_thread_count() {
        let _guard = POOL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let available = crate::num_cpus_safe();
        if available < 2 {
            return;
        }

        let pool_one = crate::get_or_build_pool(1);
        let pool_one_again = crate::get_or_build_pool(1);
        let pool_two = crate::get_or_build_pool(2.min(available));

        assert!(Arc::ptr_eq(&pool_one, &pool_one_again));
        assert!(!Arc::ptr_eq(&pool_one, &pool_two));
        assert_eq!(pool_one.current_num_threads(), 1);
        assert_eq!(pool_two.current_num_threads(), 2.min(available));
    }

    #[test]
    fn abi_version_is_nonzero() {
        assert!(crate::ABI_VERSION > 0);
    }

    // ─── Header-only probe / limits ─────────────────────────────────

    fn write_temp_file(name: &str, data: &[u8]) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("ironpress-test-{}-{name}", std::process::id()));
        std::fs::write(&path, data).unwrap();
        path
    }

    fn webp_lossless_sample() -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(40, 30, image::Rgba([10, 20, 30, 128]));
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut buf),
                image::ImageFormat::WebP,
            )
            .unwrap();
        buf
    }

    /// JPEG whose SOF0 header claims 20000x20000 (400 MP) pixels.
    fn jpeg_with_huge_header() -> Vec<u8> {
        let mut data = minimal_jpeg();
        let sof = data
            .windows(2)
            .position(|w| w == [0xFF, 0xC0])
            .expect("baseline SOF0 marker");
        // FF C0 | len(2) | precision(1) | height(2) | width(2)
        data[sof + 5..sof + 7].copy_from_slice(&20000u16.to_be_bytes());
        data[sof + 7..sof + 9].copy_from_slice(&20000u16.to_be_bytes());
        data
    }

    #[test]
    fn probe_file_matches_probe_bytes() {
        let samples: Vec<(&str, Vec<u8>)> = vec![
            ("plain.jpg", test_image_jpeg(123, 45)),
            ("exif.jpg", jpeg_with_exif()),
            ("plain.png", minimal_png()),
            ("exif.png", png_with_exif()),
            ("lossless.webp", webp_lossless_sample()),
        ];

        for (name, data) in samples {
            let path = write_temp_file(name, &data);
            let from_file = probe_file(std::fs::File::open(&path).unwrap()).unwrap();
            let from_bytes = probe_bytes(&data).unwrap();
            std::fs::remove_file(&path).ok();

            assert_eq!(from_file.width, from_bytes.width, "{name}");
            assert_eq!(from_file.height, from_bytes.height, "{name}");
            assert_eq!(from_file.format, from_bytes.format, "{name}");
            assert_eq!(from_file.file_size, from_bytes.file_size, "{name}");
            assert_eq!(from_file.has_exif, from_bytes.has_exif, "{name}");
        }
    }

    #[test]
    fn probe_file_rejects_garbage() {
        let path = write_temp_file("garbage.bin", &[0u8; 64]);
        let result = probe_file(std::fs::File::open(&path).unwrap());
        std::fs::remove_file(&path).ok();
        assert!(result.is_err());
    }

    #[test]
    fn pixel_limit_rejects_oversized_header_before_decode() {
        let data = jpeg_with_huge_header();
        let info = probe_bytes(&data).unwrap();
        assert_eq!((info.width, info.height), (20000, 20000));

        let err = compress_bytes(&data, &CompressParams::default())
            .err()
            .expect("oversized image must be rejected");
        assert_eq!(err.ffi_code(), -5, "{err}");

        let err = benchmark_bytes(&data, &CompressParams::default())
            .err()
            .expect("oversized image must be rejected");
        assert_eq!(err.ffi_code(), -5, "{err}");
    }

    #[test]
    fn compress_file_rejects_oversized_file_before_reading() {
        // Sparse file: metadata says > 256 MB without writing the bytes.
        let path = write_temp_file("huge.jpg", &[]);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(256 * 1024 * 1024 + 1)
            .unwrap();
        let c_path = std::ffi::CString::new(path.to_str().unwrap()).unwrap();

        let params = CompressParams::default();
        let mut out = std::mem::MaybeUninit::<CompressResult>::uninit();
        unsafe {
            crate::compress_file(c_path.as_ptr(), &params, out.as_mut_ptr());
            let mut out = out.assume_init();
            assert_eq!(out.error_code, -5);
            crate::free_compress_result(&mut out);
        }
        std::fs::remove_file(&path).ok();
    }

    // ─── Batch ───────────────────────────────────────────────────────

    #[test]
    fn batch_preserves_input_order() {
        let inputs_data: Vec<Vec<u8>> = vec![
            test_image_jpeg(64, 32),
            minimal_png(),
            test_image_jpeg(100, 50),
            vec![0u8; 16], // invalid
            test_image_jpeg(32, 64),
        ];
        let expected = [
            Some((64, 32)),
            Some((64, 64)),
            Some((100, 50)),
            None,
            Some((32, 64)),
        ];

        let inputs: Vec<BatchInput> = inputs_data
            .iter()
            .map(|d| BatchInput {
                file_path: std::ptr::null(),
                data: d.as_ptr(),
                data_len: d.len(),
                output_path: std::ptr::null(),
            })
            .collect();
        let params = CompressParams::default();
        let mut out = std::mem::MaybeUninit::<BatchResult>::uninit();

        unsafe {
            crate::compress_batch(
                inputs.as_ptr(),
                inputs.len(),
                &params,
                0,
                0,
                std::ptr::null(),
                out.as_mut_ptr(),
            );
            let mut out = out.assume_init();
            assert_eq!(out.count, inputs.len());
            for (i, exp) in expected.iter().enumerate() {
                let r = &*out.results.add(i);
                match exp {
                    Some((w, h)) => {
                        assert_eq!(r.error_code, 0, "item {i}");
                        assert_eq!((r.width, r.height), (*w, *h), "item {i}");
                    }
                    None => assert_ne!(r.error_code, 0, "item {i}"),
                }
            }
            crate::free_batch_result(&mut out);
        }
    }

    #[test]
    fn memory_budget_limits_concurrency_and_admits_oversized_items() {
        use std::sync::mpsc;
        use std::time::Duration;

        let budget: &'static crate::MemoryBudget = Box::leak(Box::new(crate::MemoryBudget::new()));

        // Oversized request is clamped and admitted when the budget is idle.
        drop(budget.acquire(1_000, 100));

        let first = budget.acquire(60, 100);
        let (tx, rx) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let _second = budget.acquire(60, 100);
            tx.send(()).unwrap();
        });

        // 60 + 60 > 100: the second acquire must wait for the first permit.
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        drop(first);
        rx.recv_timeout(Duration::from_secs(5))
            .expect("waiter admitted after release");
        waiter.join().unwrap();
        assert_eq!(*budget.used.lock().unwrap(), 0);
    }

    #[test]
    fn thread_pool_cache_is_bounded() {
        let _guard = POOL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for n in 1..=crate::MAX_CACHED_POOLS + 2 {
            crate::get_or_build_pool(n);
        }
        assert!(crate::THREAD_POOLS.lock().unwrap().len() <= crate::MAX_CACHED_POOLS);
    }

    // ─── Phase 2: target-size search, PNG, scaled decode, orientation ──

    /// Deterministic textured RGB image (gradient + pseudo-random detail)
    /// whose JPEG/WebP size varies smoothly with quality.
    fn textured_rgb(width: u32, height: u32, seed: u32) -> image::RgbImage {
        let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
        image::RgbImage::from_fn(width, height, |x, y| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (state >> 24) as i32 - 128;
            let base = |v: u32, m: u32| ((v * 255) / m.max(1)) as i32;
            let px = |b: i32| (b + noise / 3).clamp(0, 255) as u8;
            image::Rgb([
                px(base(x, width)),
                px(base(y, height)),
                px((base(x + y, width + height) + (((x / 7) ^ (y / 5)) & 1) as i32 * 60).min(255)),
            ])
        })
    }

    fn encode_with(img: &image::DynamicImage, format: image::ImageFormat) -> Vec<u8> {
        let mut buf = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut buf), format)
            .unwrap();
        buf
    }

    fn textured_jpeg(width: u32, height: u32) -> Vec<u8> {
        let img = image::DynamicImage::ImageRgb8(textured_rgb(width, height, 7));
        let mut buf = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 95)
            .encode_image(&img)
            .unwrap();
        buf
    }

    /// The pre-phase-2 algorithm as a reference: initial quality, then a
    /// plain binary search over [min_q, initial_q - 1].
    fn reference_binary_search(input: &[u8], base: &CompressParams, target: usize) -> Option<u32> {
        let size_at = |q: u32| {
            let mut p = CompressParams { ..*base };
            p.quality = q;
            p.max_file_size = 0;
            compress_bytes(input, &p).unwrap().data.len()
        };
        if size_at(base.quality) <= target {
            return Some(base.quality);
        }
        let (mut lo, mut hi) = (base.min_quality as i64, base.quality as i64 - 1);
        let mut best = None;
        let mut steps = 0;
        while lo <= hi && steps < 10 {
            let mid = lo + (hi - lo) / 2;
            steps += 1;
            if size_at(mid as u32) <= target {
                best = Some(mid as u32);
                lo = mid + 1;
            } else {
                hi = mid - 1;
            }
        }
        best
    }

    #[test]
    fn target_size_search_matches_binary_search_quality() {
        let input = textured_jpeg(320, 240);
        for format in [1u32, 4] {
            let mut base = CompressParams::default();
            base.format = format;
            base.min_quality = 10;
            base.allow_resize = 0;
            let full = {
                let mut p = CompressParams { ..base };
                p.quality = 80;
                compress_bytes(&input, &p).unwrap().data.len()
            };
            for fraction in [0.25f64, 0.4, 0.55, 0.7, 0.9] {
                let target = (full as f64 * fraction) as usize;
                let mut params = CompressParams { ..base };
                params.max_file_size = target as u32;
                let result = compress_bytes(&input, &params).unwrap();
                let reference = reference_binary_search(&input, &base, target);

                match reference {
                    Some(ref_q) => {
                        assert!(
                            result.data.len() <= target,
                            "format {format} target {target}: {} bytes exceeds target",
                            result.data.len()
                        );
                        assert!(
                            result.quality_used + 3 >= ref_q,
                            "format {format} target {target}: q{} vs binary search q{ref_q}",
                            result.quality_used
                        );
                    }
                    None => assert_eq!(result.quality_used, 10),
                }
                assert!(
                    result.iterations <= 1 + 8 + 1,
                    "format {format}: {} encodes in one cycle",
                    result.iterations
                );
            }
        }
    }

    #[test]
    fn target_size_resize_never_exceeds_target_and_uses_fixed_steps() {
        let input = textured_jpeg(640, 480);
        for format in [1u32, 4] {
            for target in [6_000u32, 15_000, 40_000] {
                let mut params = CompressParams::default();
                params.format = format;
                params.max_file_size = target;
                params.min_quality = 30;
                params.allow_resize = 1;
                let result = compress_bytes(&input, &params).unwrap();

                assert!(
                    result.data.len() <= target as usize,
                    "format {format}: {} > {target} ({}x{})",
                    result.data.len(),
                    result.width,
                    result.height
                );
                // Dimensions follow the ×0.75 step sequence (at most 4 steps).
                let mut dims = vec![(640u32, 480u32)];
                for _ in 0..4 {
                    let (w, h) = *dims.last().unwrap();
                    dims.push(((w as f64 * 0.75) as u32, (h as f64 * 0.75) as u32));
                }
                assert!(
                    dims.contains(&(result.width, result.height)),
                    "unexpected dims {}x{}",
                    result.width,
                    result.height
                );
                assert_eq!(
                    result.resized_to_fit,
                    (result.width, result.height) != (640, 480)
                );
            }
        }
    }

    #[test]
    fn target_size_png_with_resize_fits_and_is_verified() {
        let img = image::DynamicImage::ImageRgb8(textured_rgb(400, 300, 3));
        let input = encode_with(&img, image::ImageFormat::Png);
        let full = {
            let mut p = CompressParams::default();
            p.max_width = 400; // force the decode path (no direct optimize)
            p.max_height = 300;
            compress_bytes(&input, &p).unwrap().data.len()
        };
        let mut params = CompressParams::default();
        params.max_file_size = (full / 3) as u32;
        params.allow_resize = 1;
        let result = compress_bytes(&input, &params).unwrap();
        assert!(result.data.len() <= params.max_file_size as usize);
        assert!(result.resized_to_fit);
        assert_eq!(&result.data[1..4], b"PNG");
    }

    /// Reference for the old PNG path: image-crate encode, then oxipng.
    fn png_via_reencode(img: &image::DynamicImage, level: u8) -> Vec<u8> {
        let png = encode_with(img, image::ImageFormat::Png);
        oxipng::optimize_from_memory(&png, &oxipng::Options::from_preset(level)).unwrap()
    }

    #[test]
    fn png_raw_encode_is_never_larger_than_reencode_path() {
        let rgb = textured_rgb(160, 120, 11);
        let images = [
            image::DynamicImage::ImageRgb8(rgb.clone()),
            image::DynamicImage::ImageRgba8(image::DynamicImage::ImageRgb8(rgb.clone()).to_rgba8()),
            image::DynamicImage::ImageLuma8(image::DynamicImage::ImageRgb8(rgb.clone()).to_luma8()),
            image::DynamicImage::ImageRgba16(image::DynamicImage::ImageRgb8(rgb).to_rgba16()),
            image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(96, 64, |x, y| {
                image::Rgba([(x * 2) as u8, (y * 3) as u8, 128, ((x + y) * 2) as u8])
            })),
            image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                64,
                64,
                image::Rgb([9, 9, 9]),
            )),
        ];
        for (i, img) in images.iter().enumerate() {
            for level in [0u8, 2, 4] {
                let new = encode_png(img, level).unwrap();
                let old = png_via_reencode(img, level);
                assert!(
                    new.len() <= old.len(),
                    "image {i} level {level}: raw {} > re-encode {}",
                    new.len(),
                    old.len()
                );
                let decoded = image::load_from_memory(&new).unwrap();
                assert_eq!(decoded.to_rgba16(), img.to_rgba16(), "image {i}: lossless");
            }
        }
    }

    #[test]
    fn png_options_are_time_bounded() {
        for level in 0..=6 {
            assert_eq!(png_options(level).timeout, Some(PNG_OPTIMIZE_TIMEOUT));
        }
    }

    #[test]
    fn jpeg_scale_denominator_picks_largest_covering_scale() {
        // The DCT-scaled size must cover twice the target in both axes.
        assert_eq!(jpeg_scale_denominator(4000, 3000, 1000, 750), 2);
        assert_eq!(jpeg_scale_denominator(4000, 3000, 500, 375), 4);
        assert_eq!(jpeg_scale_denominator(4000, 3000, 501, 375), 2);
        assert_eq!(jpeg_scale_denominator(4000, 3000, 250, 187), 8);
        assert_eq!(jpeg_scale_denominator(4000, 3000, 250, 188), 4);
        assert_eq!(jpeg_scale_denominator(4000, 3000, 1001, 750), 1);
        assert_eq!(jpeg_scale_denominator(4000, 3000, 1920, 1440), 1);
        assert_eq!(jpeg_scale_denominator(4001, 2999, 500, 375), 4);
        assert_eq!(jpeg_scale_denominator(33, 17, 2, 1), 8);
    }

    #[test]
    fn scaled_jpeg_decode_keeps_final_dimensions() {
        let input = textured_jpeg(1001, 753);
        for (max_w, max_h) in [
            (500, 0),
            (501, 0),
            (250, 0),
            (333, 0),
            (0, 100),
            (125, 125),
            (700, 0),
        ] {
            let mut params = CompressParams::default();
            params.max_width = max_w;
            params.max_height = max_h;
            let result = compress_bytes(&input, &params).unwrap();
            let expected = constrained_dimensions(1001, 753, max_w, max_h).unwrap();
            assert_eq!(
                (result.width, result.height),
                expected,
                "max {max_w}x{max_h}"
            );
            let decoded = image::load_from_memory(&result.data).unwrap();
            assert_eq!((decoded.width(), decoded.height()), expected);
        }
    }

    #[test]
    fn scaled_jpeg_decode_is_close_to_full_decode() {
        let input = textured_jpeg(800, 600);
        let scaled = decode_jpeg_scaled(&input, 100, 75).expect("1/4 scaled decode");
        assert_eq!((scaled.width(), scaled.height()), (200, 150));
        let full = image::load_from_memory(&input).unwrap().resize_exact(
            200,
            150,
            image::imageops::FilterType::Triangle,
        );
        let a = scaled.to_rgb8();
        let b = full.to_rgb8();
        let diff: u64 = a
            .as_raw()
            .iter()
            .zip(b.as_raw())
            .map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs() as u64)
            .sum();
        let mean = diff as f64 / a.as_raw().len() as f64;
        assert!(mean < 12.0, "mean abs diff {mean}");
    }

    #[test]
    fn scaled_jpeg_decode_handles_progressive_gray_and_falls_back_on_garbage() {
        let rgb = image::DynamicImage::ImageRgb8(textured_rgb(320, 240, 5));
        let progressive = compress_bytes(&encode_with(&rgb, image::ImageFormat::Png), &{
            let mut p = CompressParams::default();
            p.format = 1;
            p.jpeg_progressive = 1;
            p
        })
        .unwrap()
        .data;
        let img = decode_jpeg_scaled(&progressive, 40, 30).expect("progressive");
        assert_eq!((img.width(), img.height()), (80, 60));

        let gray = encode_with(
            &image::DynamicImage::ImageLuma8(rgb.to_luma8()),
            image::ImageFormat::Jpeg,
        );
        let img = decode_jpeg_scaled(&gray, 80, 60).expect("grayscale");
        assert!(matches!(img, image::DynamicImage::ImageLuma8(_)));
        assert_eq!((img.width(), img.height()), (160, 120));

        // No reduction possible / corrupt data → None (caller falls back).
        assert!(decode_jpeg_scaled(&progressive, 100, 75).is_none());
        let mut corrupt = progressive.clone();
        corrupt.truncate(200);
        assert!(decode_jpeg_scaled(&corrupt, 40, 30).is_none());
    }

    /// Minimal little-endian TIFF/EXIF block with only an Orientation tag.
    fn exif_with_orientation(orientation: u16) -> Vec<u8> {
        let mut t = b"II*\0".to_vec();
        t.extend_from_slice(&8u32.to_le_bytes()); // IFD0 offset
        t.extend_from_slice(&1u16.to_le_bytes()); // one entry
        t.extend_from_slice(&0x0112u16.to_le_bytes()); // Orientation
        t.extend_from_slice(&3u16.to_le_bytes()); // SHORT
        t.extend_from_slice(&1u32.to_le_bytes()); // count
        t.extend_from_slice(&orientation.to_le_bytes());
        t.extend_from_slice(&[0, 0]);
        t.extend_from_slice(&0u32.to_le_bytes()); // no next IFD
        t
    }

    fn jpeg_with_exif_payload(jpeg: &[u8], tiff: &[u8]) -> Vec<u8> {
        let mut out = jpeg[..2].to_vec();
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&((2 + 6 + tiff.len()) as u16).to_be_bytes());
        out.extend_from_slice(b"Exif\0\0");
        out.extend_from_slice(tiff);
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &b in bytes {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    fn png_with_exif_payload(png: &[u8], tiff: &[u8]) -> Vec<u8> {
        // Insert eXIf right after IHDR (8-byte signature + 25-byte IHDR chunk).
        let at = 8 + 25;
        let mut chunk = (tiff.len() as u32).to_be_bytes().to_vec();
        let mut body = b"eXIf".to_vec();
        body.extend_from_slice(tiff);
        chunk.extend_from_slice(&body);
        chunk.extend_from_slice(&crc32(&body).to_be_bytes());
        let mut out = png[..at].to_vec();
        out.extend_from_slice(&chunk);
        out.extend_from_slice(&png[at..]);
        out
    }

    /// 6×4 image with a distinct color per pixel (lossless round trips).
    fn orientation_probe_image() -> image::RgbImage {
        image::RgbImage::from_fn(6, 4, |x, y| {
            image::Rgb([(x * 40) as u8, (y * 60) as u8, 200])
        })
    }

    #[test]
    fn png_orientation_all_eight_values_are_applied() {
        let src = orientation_probe_image();
        let png = encode_with(
            &image::DynamicImage::ImageRgb8(src.clone()),
            image::ImageFormat::Png,
        );
        for orientation in 1u16..=8 {
            let input = png_with_exif_payload(&png, &exif_with_orientation(orientation));
            assert_eq!(read_orientation(&input), orientation as u8);
            // Probe keeps reporting the stored (raw) dimensions.
            let probe = probe_bytes(&input).unwrap();
            assert_eq!((probe.width, probe.height), (6, 4));

            let result = compress_bytes(&input, &CompressParams::default()).unwrap();
            let mut expected = image::DynamicImage::ImageRgb8(src.clone());
            expected.apply_orientation(
                image::metadata::Orientation::from_exif(orientation as u8).unwrap(),
            );
            assert_eq!(
                (result.width, result.height),
                (expected.width(), expected.height())
            );
            let decoded = image::load_from_memory(&result.data).unwrap();
            assert_eq!(
                decoded.to_rgb8(),
                expected.to_rgb8(),
                "orientation {orientation}"
            );
        }
    }

    #[test]
    fn jpeg_orientation_all_eight_values_swap_dims_and_move_corners() {
        // Left half red, right half blue; top rows get a green stripe.
        let src = image::RgbImage::from_fn(64, 32, |x, y| {
            let g = if y < 8 { 255 } else { 0 };
            if x < 32 {
                image::Rgb([255, g, 0])
            } else {
                image::Rgb([0, g, 255])
            }
        });
        let jpeg = encode_with(
            &image::DynamicImage::ImageRgb8(src.clone()),
            image::ImageFormat::Jpeg,
        );
        for orientation in 1u16..=8 {
            let input = jpeg_with_exif_payload(&jpeg, &exif_with_orientation(orientation));
            let mut params = CompressParams::default();
            params.quality = 95;
            params.max_width = 32; // also exercises resize-before-orient
            let result = compress_bytes(&input, &params).unwrap();

            let mut expected = image::DynamicImage::ImageRgb8(src.clone());
            expected.apply_orientation(
                image::metadata::Orientation::from_exif(orientation as u8).unwrap(),
            );
            let (ew, eh) = constrained_dimensions(expected.width(), expected.height(), 32, 0)
                .unwrap_or((expected.width(), expected.height()));
            assert_eq!(
                (result.width, result.height),
                (ew, eh),
                "orientation {orientation}"
            );
            let expected = expected.resize_exact(ew, eh, image::imageops::FilterType::Triangle);

            let decoded = image::load_from_memory(&result.data).unwrap().to_rgb8();
            for (x, y) in [(1, 1), (ew - 2, 1), (1, eh - 2), (ew - 2, eh - 2)] {
                let a = decoded.get_pixel(x, y).0;
                let b = expected.to_rgb8().get_pixel(x, y).0;
                let close = a
                    .iter()
                    .zip(b)
                    .all(|(p, q)| (*p as i32 - q as i32).abs() < 60);
                assert!(
                    close,
                    "orientation {orientation} at ({x},{y}): {a:?} vs {b:?}"
                );
            }
        }
    }

    #[test]
    fn webp_and_tiff_orientation_is_read() {
        let tiff = exif_with_orientation(6);
        let mut webp = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
        webp.extend_from_slice(&10u32.to_le_bytes());
        webp.extend_from_slice(&[0x08, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        webp.extend_from_slice(b"EXIF");
        webp.extend_from_slice(&(tiff.len() as u32).to_le_bytes());
        webp.extend_from_slice(&tiff);
        assert_eq!(read_orientation(&webp), 6);
        assert_eq!(read_orientation(&tiff), 6);
        assert_eq!(read_orientation(&minimal_jpeg()), 1);
        assert_eq!(read_orientation(&jpeg_with_exif()), 1); // malformed TIFF → ignored
    }

    #[test]
    fn kept_exif_has_orientation_reset_to_one() {
        let src = image::DynamicImage::ImageRgb8(textured_rgb(48, 32, 1));
        let jpeg = encode_with(&src, image::ImageFormat::Jpeg);
        let input = jpeg_with_exif_payload(&jpeg, &exif_with_orientation(6));

        let mut params = CompressParams::default();
        params.keep_metadata = 1;
        let result = compress_bytes(&input, &params).unwrap();
        assert_eq!((result.width, result.height), (32, 48));
        assert!(probe_bytes(&result.data).unwrap().has_exif);
        assert_eq!(
            read_orientation(&result.data),
            1,
            "orientation must be reset"
        );
        // The output is displayed as-is: no second rotation by viewers.
        let decoded = image::load_from_memory(&result.data).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (32, 48));

        params.keep_metadata = 0;
        let stripped = compress_bytes(&input, &params).unwrap();
        assert!(!probe_bytes(&stripped.data).unwrap().has_exif);
        assert_eq!((stripped.width, stripped.height), (32, 48));
    }

    /// Number of components in the first SOF segment of a JPEG.
    fn jpeg_components(data: &[u8]) -> u8 {
        let mut i = 2;
        while i + 4 < data.len() {
            let marker = data[i + 1];
            let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
            if (0xC0..=0xC2).contains(&marker) {
                return data[i + 9];
            }
            i += 2 + len;
        }
        0
    }

    #[test]
    fn grayscale_jpeg_stays_single_channel() {
        let gray = image::DynamicImage::ImageLuma8(
            image::DynamicImage::ImageRgb8(textured_rgb(300, 200, 9)).to_luma8(),
        );
        let input = encode_with(&gray, image::ImageFormat::Jpeg);
        assert_eq!(jpeg_components(&input), 1);

        for (max_w, target) in [(0u32, 0u32), (100, 0), (0, 4000)] {
            let mut params = CompressParams::default();
            params.max_width = max_w;
            params.max_file_size = target;
            let result = compress_bytes(&input, &params).unwrap();
            assert_eq!(
                jpeg_components(&result.data),
                1,
                "max_w {max_w} target {target}"
            );
            let decoded = image::load_from_memory(&result.data).unwrap();
            assert!(matches!(decoded, image::DynamicImage::ImageLuma8(_)));
        }

        // Gray → PNG / WebP still work.
        for format in [2u32, 3, 4] {
            let mut params = CompressParams::default();
            params.format = format;
            let result = compress_bytes(&input, &params).unwrap();
            let decoded = image::load_from_memory(&result.data).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (300, 200));
        }
    }

    static POOL_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    // ─── Phase 3: batch control, budgets, buffers, probe orientation ──

    fn data_inputs(data: &[Vec<u8>]) -> Vec<BatchInput> {
        data.iter()
            .map(|d| BatchInput {
                file_path: std::ptr::null(),
                data: d.as_ptr(),
                data_len: d.len(),
                output_path: std::ptr::null(),
            })
            .collect()
    }

    fn run_batch(
        inputs: &[BatchInput],
        params: &CompressParams,
        threads: u32,
        control: Option<&BatchControl>,
    ) -> BatchResult {
        let mut out = std::mem::MaybeUninit::<BatchResult>::uninit();
        unsafe {
            crate::compress_batch(
                inputs.as_ptr(),
                inputs.len(),
                params,
                threads,
                0,
                control.map_or(std::ptr::null(), |c| c as *const BatchControl),
                out.as_mut_ptr(),
            );
            out.assume_init()
        }
    }

    fn new_control() -> BatchControl {
        BatchControl {
            completed: std::sync::atomic::AtomicU32::new(0),
            cancelled: std::sync::atomic::AtomicU32::new(0),
        }
    }

    #[test]
    fn batch_control_counts_completed_items_across_calls() {
        use std::sync::atomic::Ordering;
        let data: Vec<Vec<u8>> = (0..5).map(|i| test_image_jpeg(40 + i, 30)).collect();
        let inputs = data_inputs(&data);
        let params = CompressParams::default();
        let control = new_control();

        let mut out = run_batch(&inputs, &params, 0, Some(&control));
        assert_eq!(out.count, 5);
        assert_eq!(control.completed.load(Ordering::Acquire), 5);
        unsafe { crate::free_batch_result(&mut out) };

        // The counter is cumulative, so one block can span chunked calls.
        let mut out = run_batch(&inputs[..2], &params, 0, Some(&control));
        assert_eq!(out.count, 2);
        assert_eq!(control.completed.load(Ordering::Acquire), 7);
        unsafe { crate::free_batch_result(&mut out) };
    }

    #[test]
    fn batch_cancelled_before_start_returns_no_results() {
        use std::sync::atomic::Ordering;
        let data = vec![minimal_jpeg(), minimal_jpeg()];
        let inputs = data_inputs(&data);
        let control = new_control();
        control.cancelled.store(1, Ordering::Release);

        let mut out = run_batch(&inputs, &CompressParams::default(), 0, Some(&control));
        assert_eq!(out.count, 0);
        assert!(out.results.is_null());
        assert_eq!(control.completed.load(Ordering::Acquire), 0);
        unsafe { crate::free_batch_result(&mut out) };
    }

    #[test]
    fn batch_cancel_mid_run_skips_unstarted_items_and_keeps_a_prefix() {
        use std::sync::atomic::Ordering;
        // Slow-ish items on one thread: cancel as soon as the first finishes.
        let data: Vec<Vec<u8>> = (0..12).map(|_| textured_jpeg(640, 480)).collect();
        let inputs = data_inputs(&data);
        let params = CompressParams {
            format: 2, // PNG output: tens of ms per item
            ..CompressParams::default()
        };
        let control = new_control();

        let mut out = std::thread::scope(|scope| {
            scope.spawn(|| {
                while control.completed.load(Ordering::Acquire) == 0 {
                    std::thread::yield_now();
                }
                control.cancelled.store(1, Ordering::Release);
            });
            run_batch(&inputs, &params, 1, Some(&control))
        });

        assert!(out.count >= 1, "items finished before the cancel are kept");
        assert!(out.count < inputs.len(), "unstarted items are skipped");
        assert_eq!(
            control.completed.load(Ordering::Acquire) as usize,
            out.count
        );
        for i in 0..out.count {
            let r = unsafe { &*out.results.add(i) };
            assert_eq!(r.error_code, 0, "item {i}");
            assert!(!r.data.is_null());
        }
        unsafe { crate::free_batch_result(&mut out) };
    }

    #[test]
    fn memory_budget_is_checked_per_call_against_the_shared_total() {
        use std::sync::mpsc;
        use std::time::Duration;

        let budget: &'static crate::MemoryBudget = Box::leak(Box::new(crate::MemoryBudget::new()));

        // A large-budget call admits more work while a small one is running...
        let small = budget.acquire(60, 100);
        let large = budget.acquire(200, 1_000);
        assert_eq!(*budget.used.lock().unwrap(), 260);

        // ...but a small-budget call waits until the total fits its budget.
        let (tx, rx) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let _permit = budget.acquire(30, 100);
            tx.send(()).unwrap();
        });
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        drop(large);
        rx.recv_timeout(Duration::from_secs(5))
            .expect("admitted once the total fits its own budget");
        waiter.join().unwrap();
        drop(small);
        assert_eq!(*budget.used.lock().unwrap(), 0);

        assert_eq!(crate::resolve_memory_budget(0), 512 * 1024 * 1024);
        assert_eq!(crate::resolve_memory_budget(1234), 1234);
    }

    #[test]
    fn output_buffer_can_be_taken_over_and_freed_separately() {
        let data = minimal_jpeg();
        let params = CompressParams::default();
        let mut out = std::mem::MaybeUninit::<CompressResult>::uninit();
        unsafe {
            crate::compress_buffer(data.as_ptr(), data.len(), &params, out.as_mut_ptr());
            let mut out = out.assume_init();
            assert_eq!(out.error_code, 0);
            assert!(!out.buffer.is_null());
            let owned = &*(out.buffer as *const Vec<u8>);
            assert_eq!(owned.as_ptr(), out.data as *const u8);
            assert_eq!(owned.len(), out.data_len);
            let bytes = std::slice::from_raw_parts(out.data, out.data_len).to_vec();
            assert_eq!(&bytes[..3], &[0xFF, 0xD8, 0xFF]);

            // Take ownership, then free the result (must not touch the buffer).
            let handle = out.buffer;
            out.buffer = std::ptr::null_mut();
            crate::free_compress_result(&mut out);
            assert!(out.data.is_null());
            crate::ironpress_free_buffer(handle);
            crate::ironpress_free_buffer(std::ptr::null_mut());
        }

        // Errors carry no buffer.
        let err = CompressResult::error(-1, "x");
        assert!(err.buffer.is_null() && err.data.is_null());
    }

    #[test]
    fn probe_reports_orientation_for_files_and_bytes() {
        let tiff = exif_with_orientation(6);
        let png = png_with_exif_payload(&minimal_png(), &exif_with_orientation(8));
        let mut webp = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
        webp.extend_from_slice(&10u32.to_le_bytes());
        webp.extend_from_slice(&[0x08, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        webp.extend_from_slice(b"EXIF");
        webp.extend_from_slice(&(tiff.len() as u32).to_le_bytes());
        webp.extend_from_slice(&tiff);
        let samples: Vec<(&str, Vec<u8>, u8)> = vec![
            ("o6.jpg", jpeg_with_exif_payload(&minimal_jpeg(), &tiff), 6),
            ("o8.png", png, 8),
            ("plain.jpg", minimal_jpeg(), 1),
            ("bad-exif.jpg", jpeg_with_exif(), 1),
            ("plain.png", minimal_png(), 1),
            ("lossless.webp", webp_lossless_sample(), 1),
        ];

        for (name, data, expected) in samples {
            assert_eq!(read_orientation(&data), expected, "{name}");
            let from_bytes = probe_bytes(&data).unwrap();
            assert_eq!(from_bytes.orientation, expected, "{name}");
            let path = write_temp_file(name, &data);
            let from_file = probe_file(std::fs::File::open(&path).unwrap()).unwrap();
            std::fs::remove_file(&path).ok();
            assert_eq!(from_file.orientation, expected, "{name}");
            // Stored (not display) dimensions are reported.
            assert_eq!(
                (from_file.width, from_file.height),
                (from_bytes.width, from_bytes.height)
            );
        }

        // The orientation reader handles a WebP EXIF chunk (dimensions of
        // this synthetic header are not readable, so only check the walker).
        assert_eq!(slice_orientation(&webp), 6);
        assert_eq!(slice_orientation(&tiff), 6);

        let ffi = crate::probe_bytes_impl(&jpeg_with_exif_payload(&minimal_jpeg(), &tiff));
        assert_eq!(ffi.orientation, 6);
    }

    #[test]
    fn peak_estimate_accounts_for_scaled_decode_and_orientation() {
        let info = |width, height, orientation| HeaderInfo {
            format: DetectedFormat::Jpeg,
            width,
            height,
            orientation,
            native_jpeg: true,
        };
        let full = CompressParams::default();
        let small = CompressParams {
            max_width: 1000,
            max_height: 1000,
            ..CompressParams::default()
        };

        let full_est = estimate_peak_memory(info(4000, 3000, 1), 0, &full);
        let small_est = estimate_peak_memory(info(4000, 3000, 1), 0, &small);
        // 1000x750 target → 1/2-scale decode (2000x1500) instead of 12 MP.
        assert_eq!(small_est, 2000 * 1500 * 4 + 1000 * 750 * 16);
        assert!(full_est > 4 * small_est);

        // Orientation 6: the bounds apply to the rotated 3000x4000 image.
        let rotated = CompressParams {
            max_width: 1000,
            max_height: 0,
            ..CompressParams::default()
        };
        let upright = estimate_peak_memory(info(4000, 3000, 1), 0, &rotated);
        let sideways = estimate_peak_memory(info(4000, 3000, 6), 0, &rotated);
        // Upright: 1000x750 output from a 1/2-scale decode. Sideways: the
        // display size is 3000x4000, so the output is 1000x1333 and the
        // 2x-margin rule needs the full-size decode.
        assert_eq!(upright, 2000 * 1500 * 4 + 1000 * 750 * 16);
        assert_eq!(sideways, 4000 * 3000 * 4 + 1000 * 1333 * 16);

        // Non-JPEG sources never use the scaled decode.
        let gif = HeaderInfo {
            native_jpeg: false,
            ..info(4000, 3000, 1)
        };
        assert_eq!(
            estimate_peak_memory(gif, 0, &small),
            4000 * 3000 * 4 + 1000 * 750 * 16
        );
    }

    #[test]
    fn png_level_is_capped_for_large_images_unless_unbounded() {
        let params = |level, unbounded| CompressParams {
            png_optimization_level: level,
            png_level_unbounded: unbounded,
            ..CompressParams::default()
        };
        // Default and lower levels are never touched.
        assert_eq!(png_level(&params(2, 0), 8000, 6000), 2);
        assert_eq!(png_level(&params(0, 0), 8000, 6000), 0);
        // Small images keep the requested level.
        assert_eq!(png_level(&params(6, 0), 1920, 1080), 6);
        // 3.75-12 MP: at most level 4.
        assert_eq!(png_level(&params(6, 0), 4000, 3000), 4);
        assert_eq!(png_level(&params(3, 0), 4000, 3000), 3);
        // Above 12 MP: level 2.
        assert_eq!(png_level(&params(4, 0), 4001, 3000), 2);
        // Opt-out keeps the requested level.
        assert_eq!(png_level(&params(6, 1), 8000, 6000), 6);
        assert_eq!(png_level(&params(9, 1), 10, 10), 6);
    }
}
