//! Container-level metadata handling: EXIF orientation and ICC color profiles.
//!
//! Everything here works on encoded container bytes (JPEG segments, PNG
//! chunks, WebP RIFF chunks) without decoding pixels, plus a small TIFF/IFD
//! parser for the EXIF orientation tag.

use image::DynamicImage;

// ─── JPEG segment walking ────────────────────────────────────────────────────

/// Iterate JPEG marker segments, calling `visit(marker, payload)` for each
/// segment that carries a length field. Stops at SOS/EOI. `payload` excludes
/// the 2 length bytes. Returning `false` from `visit` stops the walk.
fn walk_jpeg_segments(data: &[u8], mut visit: impl FnMut(u8, &[u8]) -> bool) {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return;
    }

    let mut offset = 2usize;
    while offset + 4 <= data.len() {
        if data[offset] != 0xFF {
            break;
        }

        // Skip padding 0xFF bytes
        let mut marker_offset = offset + 1;
        while marker_offset < data.len() && data[marker_offset] == 0xFF {
            marker_offset += 1;
        }
        if marker_offset >= data.len() {
            break;
        }

        let marker = data[marker_offset];
        offset = marker_offset + 1;

        // SOS or EOI — no more metadata markers
        if marker == 0xDA || marker == 0xD9 {
            break;
        }

        // Standalone markers (no length field)
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            continue;
        }

        if offset + 2 > data.len() {
            break;
        }
        let segment_len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
        if segment_len < 2 || offset + segment_len > data.len() {
            break;
        }

        if !visit(marker, &data[offset + 2..offset + segment_len]) {
            return;
        }

        offset += segment_len;
    }
}

/// Find the EXIF payload (TIFF structure, without the "Exif\0\0" prefix)
/// in a JPEG's APP1 segment.
pub fn jpeg_extract_exif(data: &[u8]) -> Option<Vec<u8>> {
    let mut exif: Option<Vec<u8>> = None;
    walk_jpeg_segments(data, |marker, payload| {
        if marker == 0xE1 && payload.starts_with(b"Exif\0\0") && payload.len() > 6 {
            exif = Some(payload[6..].to_vec());
            return false;
        }
        true
    });
    exif
}

/// Whether a JPEG carries an EXIF APP1 segment.
pub fn jpeg_has_exif(data: &[u8]) -> bool {
    let mut found = false;
    walk_jpeg_segments(data, |marker, payload| {
        if marker == 0xE1 && payload.starts_with(b"Exif\0\0") {
            found = true;
            return false;
        }
        true
    });
    found
}

/// ICC profiles in JPEG are split across APP2 segments, each prefixed with
/// "ICC_PROFILE\0" + sequence number (1-based) + total count.
const ICC_JPEG_HEADER: &[u8] = b"ICC_PROFILE\0";

/// Extract and reassemble a (possibly multi-segment) ICC profile from a JPEG.
pub fn jpeg_extract_icc(data: &[u8]) -> Option<Vec<u8>> {
    let mut parts: Vec<(u8, Vec<u8>)> = Vec::new();
    walk_jpeg_segments(data, |marker, payload| {
        if marker == 0xE2
            && payload.len() > ICC_JPEG_HEADER.len() + 2
            && payload.starts_with(ICC_JPEG_HEADER)
        {
            let seq = payload[ICC_JPEG_HEADER.len()];
            let chunk = payload[ICC_JPEG_HEADER.len() + 2..].to_vec();
            parts.push((seq, chunk));
        }
        true
    });

    if parts.is_empty() {
        return None;
    }
    parts.sort_by_key(|(seq, _)| *seq);
    let mut profile = Vec::with_capacity(parts.iter().map(|(_, c)| c.len()).sum());
    for (_, chunk) in parts {
        profile.extend_from_slice(&chunk);
    }
    Some(profile)
}

// ─── PNG chunk walking ───────────────────────────────────────────────────────

/// Iterate PNG chunks, calling `visit(type, data)` for each. Stops at IEND
/// or when `visit` returns `false`.
fn walk_png_chunks(data: &[u8], mut visit: impl FnMut(&[u8; 4], &[u8]) -> bool) {
    if data.len() < 12 {
        return;
    }

    // Skip PNG signature (8 bytes)
    let mut offset = 8usize;
    while offset + 8 <= data.len() {
        let chunk_len = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        let chunk_type: &[u8; 4] = data[offset + 4..offset + 8].try_into().unwrap();

        if offset + 12 + chunk_len > data.len() {
            break;
        }

        if !visit(chunk_type, &data[offset + 8..offset + 8 + chunk_len]) {
            return;
        }

        if chunk_type == b"IEND" {
            break;
        }

        offset += 12 + chunk_len;
    }
}

/// Find a PNG chunk's data by type.
fn png_find_chunk(data: &[u8], wanted: &[u8; 4]) -> Option<Vec<u8>> {
    let mut found: Option<Vec<u8>> = None;
    walk_png_chunks(data, |chunk_type, chunk_data| {
        if chunk_type == wanted {
            found = Some(chunk_data.to_vec());
            return false;
        }
        true
    });
    found
}

/// Whether a PNG has an eXIf chunk.
pub fn png_has_exif(data: &[u8]) -> bool {
    let mut found = false;
    walk_png_chunks(data, |chunk_type, _| {
        if chunk_type == b"eXIf" {
            found = true;
            return false;
        }
        true
    });
    found
}

/// Extract the EXIF payload (raw TIFF structure) from a PNG eXIf chunk.
pub fn png_extract_exif(data: &[u8]) -> Option<Vec<u8>> {
    png_find_chunk(data, b"eXIf")
}

/// Extract and decompress the ICC profile from a PNG iCCP chunk.
pub fn png_extract_icc(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;

    let chunk = png_find_chunk(data, b"iCCP")?;
    // iCCP layout: profile name (1-79 bytes, latin-1) + null + compression
    // method (must be 0 = zlib) + compressed profile.
    let null_pos = chunk.iter().position(|&b| b == 0)?;
    if null_pos + 2 > chunk.len() || chunk[null_pos + 1] != 0 {
        return None;
    }
    let compressed = &chunk[null_pos + 2..];

    let mut profile = Vec::new();
    let mut decoder = flate2::read::ZlibDecoder::new(compressed);
    decoder.read_to_end(&mut profile).ok()?;
    if profile.is_empty() {
        None
    } else {
        Some(profile)
    }
}

/// Insert an iCCP chunk (zlib-compressed ICC profile) immediately after IHDR.
/// Assumes `png_data` currently has no iCCP chunk, which holds for everything
/// this crate emits (the image encoder writes none and oxipng strips all).
pub fn png_insert_iccp(png_data: Vec<u8>, icc: &[u8]) -> Vec<u8> {
    use std::io::Write;

    if png_data.len() < 33 || &png_data[0..8] != b"\x89PNG\r\n\x1a\n" {
        return png_data;
    }

    // IHDR is required to be the first chunk; compute its end dynamically.
    let ihdr_len =
        u32::from_be_bytes([png_data[8], png_data[9], png_data[10], png_data[11]]) as usize;
    let insert_at = 8 + 12 + ihdr_len;
    if insert_at > png_data.len() {
        return png_data;
    }

    let mut compressed = Vec::new();
    {
        let mut encoder =
            flate2::write::ZlibEncoder::new(&mut compressed, flate2::Compression::best());
        if encoder.write_all(icc).is_err() || encoder.finish().is_err() {
            return png_data;
        }
    }

    // Chunk data: profile name + null + compression method 0 + compressed profile
    let mut chunk_data = Vec::with_capacity(13 + 2 + compressed.len());
    chunk_data.extend_from_slice(b"ICC profile\0\0");
    chunk_data.extend_from_slice(&compressed);

    let mut hasher = crc32fast::Hasher::new();
    hasher.update(b"iCCP");
    hasher.update(&chunk_data);
    let crc = hasher.finalize();

    let mut out = Vec::with_capacity(png_data.len() + chunk_data.len() + 12);
    out.extend_from_slice(&png_data[..insert_at]);
    out.extend_from_slice(&(chunk_data.len() as u32).to_be_bytes());
    out.extend_from_slice(b"iCCP");
    out.extend_from_slice(&chunk_data);
    out.extend_from_slice(&crc.to_be_bytes());
    out.extend_from_slice(&png_data[insert_at..]);
    out
}

// ─── WebP RIFF chunk walking ─────────────────────────────────────────────────

/// Find a chunk in a WebP RIFF container by FourCC.
fn webp_find_chunk(data: &[u8], wanted: &[u8; 4]) -> Option<Vec<u8>> {
    if data.len() < 16 || &data[0..4] != b"RIFF" || &data[8..12] != b"WEBP" {
        return None;
    }

    let mut offset = 12usize;
    while offset + 8 <= data.len() {
        let fourcc = &data[offset..offset + 4];
        let chunk_len = u32::from_le_bytes([
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]) as usize;

        if offset + 8 + chunk_len > data.len() {
            break;
        }

        if fourcc == wanted {
            return Some(data[offset + 8..offset + 8 + chunk_len].to_vec());
        }

        // Chunks are padded to even sizes
        offset += 8 + chunk_len + (chunk_len & 1);
    }

    None
}

/// Extract the ICC profile from a WebP ICCP chunk.
pub fn webp_extract_icc(data: &[u8]) -> Option<Vec<u8>> {
    webp_find_chunk(data, b"ICCP")
}

/// Extract the EXIF payload from a WebP EXIF chunk.
/// Some encoders include the "Exif\0\0" prefix; strip it if present.
pub fn webp_extract_exif(data: &[u8]) -> Option<Vec<u8>> {
    let chunk = webp_find_chunk(data, b"EXIF")?;
    if chunk.starts_with(b"Exif\0\0") {
        if chunk.len() > 6 {
            Some(chunk[6..].to_vec())
        } else {
            None
        }
    } else {
        Some(chunk)
    }
}

/// Whether a WebP carries an EXIF chunk.
pub fn webp_has_exif(data: &[u8]) -> bool {
    webp_find_chunk(data, b"EXIF").is_some()
}

// ─── TIFF/IFD parsing for the EXIF orientation tag ───────────────────────────

const ORIENTATION_TAG: u16 = 0x0112;

/// Locate the EXIF orientation tag in a raw TIFF payload.
/// Returns (value_offset, big_endian) of the tag's 2-byte SHORT value.
fn find_orientation_value(tiff: &[u8]) -> Option<(usize, bool)> {
    if tiff.len() < 8 {
        return None;
    }

    let big_endian = match &tiff[0..2] {
        b"II" => false,
        b"MM" => true,
        _ => return None,
    };

    let read_u16 = |bytes: &[u8]| -> u16 {
        let pair = [bytes[0], bytes[1]];
        if big_endian {
            u16::from_be_bytes(pair)
        } else {
            u16::from_le_bytes(pair)
        }
    };
    let read_u32 = |bytes: &[u8]| -> u32 {
        let quad = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if big_endian {
            u32::from_be_bytes(quad)
        } else {
            u32::from_le_bytes(quad)
        }
    };

    if read_u16(&tiff[2..4]) != 42 {
        return None;
    }

    let ifd0 = read_u32(&tiff[4..8]) as usize;
    if ifd0 + 2 > tiff.len() {
        return None;
    }

    let entry_count = read_u16(&tiff[ifd0..ifd0 + 2]) as usize;
    for i in 0..entry_count {
        let entry = ifd0 + 2 + i * 12;
        if entry + 12 > tiff.len() {
            return None;
        }
        let tag = read_u16(&tiff[entry..entry + 2]);
        if tag != ORIENTATION_TAG {
            continue;
        }
        // Type must be SHORT (3) with count 1; value lives inline.
        let field_type = read_u16(&tiff[entry + 2..entry + 4]);
        let count = read_u32(&tiff[entry + 4..entry + 8]);
        if field_type != 3 || count != 1 {
            return None;
        }
        return Some((entry + 8, big_endian));
    }

    None
}

/// Read the EXIF orientation (1-8) from a raw TIFF payload.
pub fn exif_orientation(tiff: &[u8]) -> Option<u16> {
    let (offset, big_endian) = find_orientation_value(tiff)?;
    let pair = [tiff[offset], tiff[offset + 1]];
    let value = if big_endian {
        u16::from_be_bytes(pair)
    } else {
        u16::from_le_bytes(pair)
    };
    (1..=8).contains(&value).then_some(value)
}

/// Rewrite the orientation tag to 1 ("upright") in a raw TIFF payload.
/// Called after pixels have been physically rotated, so a preserved EXIF
/// block no longer claims the image needs rotation.
pub fn patch_exif_orientation_to_upright(tiff: &mut [u8]) {
    if let Some((offset, big_endian)) = find_orientation_value(tiff) {
        let bytes = if big_endian {
            1u16.to_be_bytes()
        } else {
            1u16.to_le_bytes()
        };
        tiff[offset] = bytes[0];
        tiff[offset + 1] = bytes[1];
    }
}

// ─── Orientation application ─────────────────────────────────────────────────

/// Physically transform pixels to undo an EXIF orientation (1-8).
pub fn apply_orientation(img: DynamicImage, orientation: u16) -> DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}
