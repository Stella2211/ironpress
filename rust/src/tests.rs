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

    // ─── EXIF Orientation ────────────────────────────────────────────

    /// Minimal valid little-endian TIFF payload with a single IFD0 entry:
    /// the orientation tag (0x0112, SHORT, count 1).
    fn exif_tiff_with_orientation(orientation: u16) -> Vec<u8> {
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II");
        tiff.extend_from_slice(&42u16.to_le_bytes());
        tiff.extend_from_slice(&8u32.to_le_bytes()); // IFD0 offset
        tiff.extend_from_slice(&1u16.to_le_bytes()); // entry count
        tiff.extend_from_slice(&0x0112u16.to_le_bytes()); // orientation tag
        tiff.extend_from_slice(&3u16.to_le_bytes()); // type SHORT
        tiff.extend_from_slice(&1u32.to_le_bytes()); // count
        tiff.extend_from_slice(&orientation.to_le_bytes());
        tiff.extend_from_slice(&[0u8, 0]); // value field padding
        tiff.extend_from_slice(&0u32.to_le_bytes()); // next IFD offset
        tiff
    }

    /// Gradient JPEG with a real EXIF orientation tag injected as APP1.
    fn jpeg_with_orientation(width: u32, height: u32, orientation: u16) -> Vec<u8> {
        let base = test_image_jpeg(width, height);
        let mut payload = Vec::from(&b"Exif\0\0"[..]);
        payload.extend_from_slice(&exif_tiff_with_orientation(orientation));

        let mut out = Vec::new();
        out.extend_from_slice(&base[..2]); // SOI
        out.push(0xFF);
        out.push(0xE1);
        out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&payload);
        out.extend_from_slice(&base[2..]);
        out
    }

    #[test]
    fn exif_orientation_parses_both_byte_orders() {
        let le = exif_tiff_with_orientation(6);
        assert_eq!(crate::metadata::exif_orientation(&le), Some(6));

        // Big-endian variant
        let mut be = Vec::new();
        be.extend_from_slice(b"MM");
        be.extend_from_slice(&42u16.to_be_bytes());
        be.extend_from_slice(&8u32.to_be_bytes());
        be.extend_from_slice(&1u16.to_be_bytes());
        be.extend_from_slice(&0x0112u16.to_be_bytes());
        be.extend_from_slice(&3u16.to_be_bytes());
        be.extend_from_slice(&1u32.to_be_bytes());
        be.extend_from_slice(&8u16.to_be_bytes());
        be.extend_from_slice(&[0u8, 0]);
        be.extend_from_slice(&0u32.to_be_bytes());
        assert_eq!(crate::metadata::exif_orientation(&be), Some(8));
    }

    #[test]
    fn auto_orient_rotates_pixels() {
        // Orientation 6 = rotate 90° CW, so a 64x32 image becomes 32x64.
        let input = jpeg_with_orientation(64, 32, 6);
        let params = CompressParams::default();
        let result = compress_bytes(&input, &params).unwrap();
        assert_eq!((result.width, result.height), (32, 64));
    }

    #[test]
    fn auto_orient_disabled_keeps_stored_dimensions() {
        let input = jpeg_with_orientation(64, 32, 6);
        let params = CompressParams {
            auto_orient: 0,
            ..Default::default()
        };
        let result = compress_bytes(&input, &params).unwrap();
        assert_eq!((result.width, result.height), (64, 32));
    }

    #[test]
    fn auto_orient_patches_preserved_exif_to_upright() {
        let input = jpeg_with_orientation(64, 32, 6);
        let params = CompressParams {
            keep_metadata: 1,
            ..Default::default()
        };
        let result = compress_bytes(&input, &params).unwrap();

        let exif = crate::metadata::jpeg_extract_exif(&result.data)
            .expect("EXIF should be preserved in JPEG output");
        assert_eq!(crate::metadata::exif_orientation(&exif), Some(1));
    }

    #[test]
    fn upright_orientation_skips_rotation() {
        let input = jpeg_with_orientation(64, 32, 1);
        let params = CompressParams::default();
        let result = compress_bytes(&input, &params).unwrap();
        assert_eq!((result.width, result.height), (64, 32));
    }

    // ─── ICC Profile Preservation ────────────────────────────────────

    /// A fake-but-plausible ICC profile payload. Contents don't matter for
    /// container round-trip tests, only faithful byte preservation.
    fn fake_icc_profile() -> Vec<u8> {
        let mut profile = Vec::from(&b"acspAPPL-test-profile"[..]);
        profile.extend((0..200u16).map(|i| (i % 251) as u8));
        profile
    }

    /// Inject an ICC profile into a JPEG as a single APP2 segment.
    fn jpeg_with_icc(profile: &[u8]) -> Vec<u8> {
        let base = test_image_jpeg(64, 64);
        let mut payload = Vec::from(&b"ICC_PROFILE\0"[..]);
        payload.push(1); // sequence number
        payload.push(1); // total count
        payload.extend_from_slice(profile);

        let mut out = Vec::new();
        out.extend_from_slice(&base[..2]);
        out.push(0xFF);
        out.push(0xE2);
        out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&payload);
        out.extend_from_slice(&base[2..]);
        out
    }

    #[test]
    fn icc_profile_roundtrips_jpeg_to_jpeg() {
        let profile = fake_icc_profile();
        let input = jpeg_with_icc(&profile);
        assert_eq!(
            crate::metadata::jpeg_extract_icc(&input).as_deref(),
            Some(profile.as_slice())
        );

        let result = compress_bytes(&input, &CompressParams::default()).unwrap();
        assert_eq!(
            crate::metadata::jpeg_extract_icc(&result.data).as_deref(),
            Some(profile.as_slice())
        );
    }

    #[test]
    fn icc_profile_disabled_is_dropped() {
        let profile = fake_icc_profile();
        let input = jpeg_with_icc(&profile);
        let params = CompressParams {
            preserve_icc: 0,
            ..Default::default()
        };
        let result = compress_bytes(&input, &params).unwrap();
        assert_eq!(crate::metadata::jpeg_extract_icc(&result.data), None);
    }

    #[test]
    fn icc_profile_roundtrips_png_direct_path() {
        let profile = fake_icc_profile();
        let input = crate::metadata::png_insert_iccp(test_image_png(64, 64), &profile);
        assert_eq!(
            crate::metadata::png_extract_icc(&input).as_deref(),
            Some(profile.as_slice())
        );

        // No resize + PNG→PNG takes the direct oxipng path.
        let result = compress_bytes(&input, &CompressParams::default()).unwrap();
        assert_eq!(
            crate::metadata::png_extract_icc(&result.data).as_deref(),
            Some(profile.as_slice())
        );
    }

    #[test]
    fn icc_profile_roundtrips_png_reencode_path() {
        let profile = fake_icc_profile();
        let input = crate::metadata::png_insert_iccp(test_image_png(64, 64), &profile);

        // Resize forces the full decode/re-encode path.
        let params = CompressParams {
            max_width: 32,
            ..Default::default()
        };
        let result = compress_bytes(&input, &params).unwrap();
        assert_eq!(result.width, 32);
        assert_eq!(
            crate::metadata::png_extract_icc(&result.data).as_deref(),
            Some(profile.as_slice())
        );
    }

    #[test]
    fn icc_profile_carries_from_jpeg_to_png() {
        let profile = fake_icc_profile();
        let input = jpeg_with_icc(&profile);
        let params = CompressParams {
            format: 2, // PNG output
            ..Default::default()
        };
        let result = compress_bytes(&input, &params).unwrap();
        assert_eq!(
            crate::metadata::png_extract_icc(&result.data).as_deref(),
            Some(profile.as_slice())
        );
    }

    // ─── Lossy PNG ───────────────────────────────────────────────────

    /// Photo-like PNG: deterministic pseudo-random noise. Nearly
    /// incompressible losslessly, which is where quantization shines.
    /// (Smooth synthetic gradients compress better losslessly, since
    /// dithering noise defeats PNG filtering.)
    fn noisy_png(width: u32, height: u32) -> Vec<u8> {
        let mut state = 0x12345678u32;
        let mut next = move || {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (state >> 24) as u8
        };
        let mut img = image::RgbaImage::new(width, height);
        for y in 0..height {
            for x in 0..width {
                img.put_pixel(x, y, image::Rgba([next(), next(), next(), 255]));
            }
        }
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        buf
    }

    #[test]
    fn lossy_png_is_smaller_and_decodable() {
        let input = noisy_png(256, 256);

        let lossless = compress_bytes(&input, &CompressParams::default()).unwrap();
        let lossy_params = CompressParams {
            png_lossy: 1,
            ..Default::default()
        };
        let lossy = compress_bytes(&input, &lossy_params).unwrap();

        assert!(
            lossy.data.len() < lossless.data.len(),
            "lossy ({}) should be smaller than lossless ({})",
            lossy.data.len(),
            lossless.data.len()
        );

        let decoded = image::load_from_memory(&lossy.data).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (256, 256));
    }

    #[test]
    fn lossy_png_preserves_transparency() {
        let mut img = image::RgbaImage::new(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                let alpha = if x < 32 { 0 } else { 255 };
                img.put_pixel(x, y, image::Rgba([200, 50, 50, alpha]));
            }
        }
        let mut input = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut input),
                image::ImageFormat::Png,
            )
            .unwrap();

        let params = CompressParams {
            png_lossy: 1,
            ..Default::default()
        };
        let result = compress_bytes(&input, &params).unwrap();

        let decoded = image::load_from_memory(&result.data).unwrap().to_rgba8();
        assert_eq!(
            decoded.get_pixel(0, 0)[3],
            0,
            "transparent side stays transparent"
        );
        assert_eq!(decoded.get_pixel(63, 0)[3], 255, "opaque side stays opaque");
    }

    #[test]
    fn lossy_png_supports_target_size_search() {
        let input = test_image_png(256, 256);
        let params = CompressParams {
            png_lossy: 1,
            max_file_size: 4 * 1024,
            allow_resize: 0,
            min_quality: 5,
            ..Default::default()
        };
        let result = compress_bytes(&input, &params).unwrap();
        assert!(
            result.iterations > 1,
            "quality search should run for lossy PNG"
        );
    }

    // ─── AVIF Output ─────────────────────────────────────────────────

    fn avif_params() -> CompressParams {
        CompressParams {
            format: 5,      // AVIF
            avif_speed: 10, // fastest — keep tests quick
            quality: 60,
            ..Default::default()
        }
    }

    #[test]
    fn avif_output_has_valid_container() {
        let input = test_image_jpeg(128, 128);
        let result = compress_bytes(&input, &avif_params()).unwrap();
        assert!(result.data.len() > 12);
        assert_eq!(&result.data[4..12], b"ftypavif");
    }

    #[test]
    fn avif_encodes_alpha_from_png() {
        let input = test_image_png(64, 64);
        let result = compress_bytes(&input, &avif_params()).unwrap();
        assert_eq!(&result.data[4..12], b"ftypavif");
        assert_eq!((result.width, result.height), (64, 64));
    }

    #[test]
    fn avif_supports_target_size_search() {
        let input = test_image_jpeg(256, 256);
        let mut params = avif_params();
        params.quality = 90;
        params.max_file_size = 2 * 1024;
        params.allow_resize = 0;
        params.min_quality = 5;
        let result = compress_bytes(&input, &params).unwrap();
        assert!(result.iterations > 1, "quality search should run for AVIF");
        assert_eq!(&result.data[4..12], b"ftypavif");
    }

    // ─── HEIC / AVIF Input Detection ─────────────────────────────────

    #[test]
    fn heic_input_reports_actionable_error() {
        let mut data = vec![0, 0, 0, 24];
        data.extend_from_slice(b"ftypheic");
        data.extend_from_slice(&[0u8; 32]);

        let err = detect_format(&data).unwrap_err();
        assert!(err.to_string().contains("HEIC"), "got: {err}");
    }

    #[test]
    fn avif_input_reports_output_only_error() {
        let mut data = vec![0, 0, 0, 24];
        data.extend_from_slice(b"ftypavif");
        data.extend_from_slice(&[0u8; 32]);

        let err = detect_format(&data).unwrap_err();
        assert!(err.to_string().contains("AVIF"), "got: {err}");
    }
}
