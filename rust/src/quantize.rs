//! Lossy PNG palette quantization.
//!
//! Reduces RGBA images to a palette of up to 256 colors (NeuQuant) with
//! Floyd–Steinberg dithering, then writes an indexed PNG. Combined with the
//! existing oxipng pass this typically shrinks screenshots and UI graphics
//! by 60-80% — far beyond what lossless optimization alone can reach.
//!
//! NeuQuant (`color_quant`) is used instead of libimagequant because the
//! latter is GPL-licensed and this package is MIT.

use color_quant::NeuQuant;

use crate::error::CompressError;

/// NeuQuant sampling factor: 1 = every pixel (slowest/best), 30 = sparsest.
/// 10 is the conventional speed/quality balance.
const SAMPLE_FACTOR: i32 = 10;

/// Map the user-facing quality (0-100) to a palette size.
/// Monotonic, capped to the PNG palette maximum of 256 entries. The floor of
/// 16 keeps NeuQuant's network stable (it is tuned for larger palettes).
fn palette_size_for_quality(quality: u32) -> usize {
    let quality = quality.clamp(1, 100) as usize;
    (quality * 256 / 100).clamp(16, 256)
}

/// Quantize RGBA pixels to an indexed PNG with dithering.
/// Returns encoded PNG bytes (pre-oxipng).
pub fn encode_indexed_png(
    rgba: &[u8],
    width: u32,
    height: u32,
    quality: u32,
) -> Result<Vec<u8>, CompressError> {
    let pixel_count = (width as usize) * (height as usize);
    if rgba.len() != pixel_count * 4 {
        return Err(CompressError::EncodeError(
            "RGBA buffer size mismatch during quantization".into(),
        ));
    }

    let palette_size = palette_size_for_quality(quality);
    let quantizer = NeuQuant::new(SAMPLE_FACTOR, palette_size, rgba);
    let palette = quantizer.color_map_rgba();
    let indices =
        dither_floyd_steinberg(rgba, width as usize, height as usize, &quantizer, &palette);

    write_indexed_png(&indices, &palette, width, height)
}

/// Floyd–Steinberg error-diffusion dithering against the quantized palette.
/// Diffuses per-channel error to the right/below neighbours (7/16, 3/16,
/// 5/16, 1/16), which hides banding in gradients that plain nearest-color
/// mapping would produce.
fn dither_floyd_steinberg(
    rgba: &[u8],
    width: usize,
    height: usize,
    quantizer: &NeuQuant,
    palette: &[u8],
) -> Vec<u8> {
    let mut indices = vec![0u8; width * height];
    // Two rows of per-channel error accumulators (current + next).
    let mut error_rows = vec![0i16; width * 4 * 2];

    for y in 0..height {
        let (current_errors, next_errors) = error_rows.split_at_mut(width * 4);
        next_errors.fill(0);

        for x in 0..width {
            let i = y * width + x;
            let src = &rgba[i * 4..i * 4 + 4];

            let mut adjusted = [0u8; 4];
            for c in 0..4 {
                let value = src[c] as i16 + (current_errors[x * 4 + c] >> 4);
                adjusted[c] = value.clamp(0, 255) as u8;
            }

            let index = quantizer.index_of(&adjusted);
            indices[i] = index as u8;

            let chosen = &palette[index * 4..index * 4 + 4];
            for c in 0..4 {
                // Errors are stored pre-multiplied by 16 so the weights
                // (7, 3, 5, 1) stay integral.
                let error = adjusted[c] as i16 - chosen[c] as i16;
                if x + 1 < width {
                    current_errors[(x + 1) * 4 + c] += error * 7;
                }
                if x > 0 {
                    next_errors[(x - 1) * 4 + c] += error * 3;
                }
                next_errors[x * 4 + c] += error * 5;
                if x + 1 < width {
                    next_errors[(x + 1) * 4 + c] += error;
                }
            }
        }

        // Rotate: next row's accumulated errors become current.
        let (current_errors, next_errors) = error_rows.split_at_mut(width * 4);
        current_errors.copy_from_slice(next_errors);
    }

    indices
}

/// Write an 8-bit indexed PNG with PLTE and (when needed) tRNS chunks.
fn write_indexed_png(
    indices: &[u8],
    palette_rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, CompressError> {
    let entries = palette_rgba.len() / 4;
    let mut plte = Vec::with_capacity(entries * 3);
    let mut trns = Vec::with_capacity(entries);
    for entry in palette_rgba.chunks_exact(4) {
        plte.extend_from_slice(&entry[0..3]);
        trns.push(entry[3]);
    }
    // tRNS entries default to opaque; trailing 255s can be dropped entirely.
    while trns.last() == Some(&255) {
        trns.pop();
    }

    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Indexed);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_palette(plte);
        if !trns.is_empty() {
            encoder.set_trns(trns);
        }
        let mut writer = encoder
            .write_header()
            .map_err(|e| CompressError::EncodeError(format!("Indexed PNG header failed: {e}")))?;
        writer
            .write_image_data(indices)
            .map_err(|e| CompressError::EncodeError(format!("Indexed PNG encode failed: {e}")))?;
    }

    Ok(out)
}
