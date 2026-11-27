//! Bounded clipboard image decoding. No clipboard content is read at startup or on idle frames.

use photocraft_codecs::{ChannelLayout, DecodeOptions, Image, Limits, SampleType};

pub(super) const MAX_IMAGE_PIXELS: u64 = 100_000_000;
pub(super) const MAX_IMAGE_BYTES: usize = 512 * 1024 * 1024;

#[cfg(any(target_os = "windows", test))]
#[derive(Default)]
pub(super) struct RevisionCache {
    sequence: Option<u32>,
}

#[cfg(any(target_os = "windows", test))]
impl RevisionCache {
    pub(super) fn unchanged(&self, sequence: Option<u32>) -> bool {
        sequence.is_some() && sequence == self.sequence
    }

    pub(super) fn imported(&mut self, sequence: Option<u32>, image: (u32, u32, Vec<u8>)) -> Result<photocraft_ui_egui::ClipboardRead, String> {
        let (width, height, pixels) = image;
        let expected = dimensions(width, height)?;
        if pixels.len() != expected {
            return Err("clipboard image pixel data has an invalid length".into());
        }
        // This payload satisfies the exact UI import invariants. Never record a
        // failed decode/invalid payload as successfully cached for the next paste.
        self.sequence = sequence;
        Ok(photocraft_ui_egui::ClipboardRead::Image { width, height, pixels })
    }
}

pub(super) fn dimensions(width: u32, height: u32) -> Result<usize, String> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || width > 262_144 || height > 262_144 || pixels > MAX_IMAGE_PIXELS {
        return Err("clipboard image exceeds the 100 megapixel paste limit (or has invalid dimensions)".into());
    }
    usize::try_from(pixels.checked_mul(4).ok_or("clipboard image size overflow")?).map_err(|_| "clipboard image does not fit in memory".into())
}

#[cfg(any(target_os = "windows", test))]
fn word(bytes: &[u8], at: usize) -> Result<u32, String> {
    let values: [u8; 4] = bytes.get(at..at.saturating_add(4)).and_then(|v| v.try_into().ok()).ok_or("truncated clipboard image header")?;
    Ok(u32::from_le_bytes(values))
}

#[cfg(any(target_os = "windows", test))]
fn dib_dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    let header = word(bytes, 0)?;
    if !matches!(header, 40 | 52 | 56 | 108 | 124) {
        return Err("unsupported clipboard bitmap header".into());
    }
    let width = i32::from_le_bytes(word(bytes, 4)?.to_le_bytes());
    let height = i32::from_le_bytes(word(bytes, 8)?.to_le_bytes());
    if width <= 0 || height == 0 || height == i32::MIN {
        return Err("invalid clipboard bitmap dimensions".into());
    }
    let height = height.unsigned_abs();
    dimensions(width as u32, height)?;
    Ok((width as u32, height))
}

fn options() -> DecodeOptions {
    DecodeOptions {
        limits: Limits { max_width: 262_144, max_height: 262_144, max_pixels: MAX_IMAGE_PIXELS, max_alloc: MAX_IMAGE_BYTES as u64 },
        ..Default::default()
    }
}

#[cfg(any(target_os = "windows", test))]
pub(super) fn decode_image(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    decode_image_candidate(bytes)?.ok_or_else(|| "clipboard image data cannot be decoded".into())
}

pub(super) fn decode_image_candidate(bytes: &[u8]) -> Result<Option<(u32, u32, Vec<u8>)>, String> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("clipboard image exceeds the 512 MiB decode limit".into());
    }
    let image = match photocraft_codecs::decode_with(bytes, &options()) {
        Ok(image) => image,
        Err(error @ photocraft_codecs::CodecError::LimitExceeded(_)) => return Err(error.to_string()),
        Err(_) => return Ok(None),
    };
    let (width, height) = image.dimensions();
    let pixels = into_rgba8(image)?;
    Ok(Some((width, height, pixels)))
}

fn into_rgba8(image: Image) -> Result<Vec<u8>, String> {
    let length = dimensions(image.width(), image.height())?;
    if image.layout() == ChannelLayout::Rgba && image.sample_type() == SampleType::U8 {
        return Ok(image.into_data());
    }
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(length).map_err(|_| "not enough memory to convert clipboard pixels")?;
    let bytes_per_pixel = image.layout().channels() * image.sample_type().bytes();
    // Image::to_rgba8's general conversion uses full-image f32 intermediates.
    // Convert bounded strips with exactly the same math instead of multiplying
    // a large Gray/16-bit/HDR file into several full-size float allocations.
    for source in image.data().chunks(65_536 * bytes_per_pixel) {
        let count = source.len() / bytes_per_pixel;
        let strip = Image::from_raw(count as u32, 1, image.layout(), image.sample_type(), source.to_vec()).map_err(|error| error.to_string())?;
        pixels.extend_from_slice(&strip.to_rgba8());
    }
    Ok(pixels)
}

#[cfg(any(target_os = "windows", test))]
fn decode_dib(mut bytes: Vec<u8>) -> Result<(u32, u32, Vec<u8>), String> {
    let (width, height) = dib_dimensions(&bytes)?;
    let header = usize::try_from(word(&bytes, 0)?).map_err(|_| "invalid clipboard bitmap header")?;
    if bytes.len() < header || bytes.len() > MAX_IMAGE_BYTES {
        return Err("invalid clipboard bitmap length".into());
    }
    let bits = bytes.get(14..16).and_then(|v| <[u8; 2]>::try_from(v).ok()).map(u16::from_le_bytes).ok_or("truncated clipboard bitmap depth")?;
    let compression = word(&bytes, 16)?;
    // Chromium uses BI_RGB with a valid alpha mask. Respect its alpha, as arboard did.
    if header >= 56 && bits == 32 && compression == 0 && word(&bytes, 52)? == 0xff00_0000 {
        bytes.get_mut(16..20).ok_or("invalid bitmap compression")?.copy_from_slice(&3u32.to_le_bytes());
        if word(&bytes, 40)? == 0 && word(&bytes, 44)? == 0 && word(&bytes, 48)? == 0 {
            for (at, mask) in [(40, 0x00ff_0000u32), (44, 0x0000_ff00), (48, 0x0000_00ff)] {
                bytes.get_mut(at..at + 4).ok_or("invalid bitmap mask")?.copy_from_slice(&mask.to_le_bytes());
            }
        }
    }
    let colors = word(&bytes, 32)?;
    let palette = if colors != 0 {
        u64::from(colors)
    } else if bits <= 8 {
        1u64 << bits
    } else {
        0
    };
    let masks = if header == 40 {
        match compression {
            3 => 12,
            6 => 16,
            _ => 0,
        }
    } else {
        0
    };
    let offset = 14u64 + header as u64 + masks + palette.saturating_mul(4);
    if offset > bytes.len() as u64 + 14 {
        return Err("clipboard bitmap palette is truncated".into());
    }
    let total = bytes.len().checked_add(14).ok_or("clipboard bitmap size overflow")?;
    let total_u32 = u32::try_from(total).map_err(|_| "clipboard bitmap is too large")?;
    bytes.try_reserve_exact(14).map_err(|_| "not enough memory to decode clipboard bitmap")?;
    let old_len = bytes.len();
    bytes.resize(total, 0);
    bytes.copy_within(0..old_len, 14);
    let file_header = bytes.get_mut(..14).ok_or("invalid clipboard bitmap")?;
    file_header.fill(0);
    file_header.get_mut(..2).ok_or("invalid bitmap signature")?.copy_from_slice(b"BM");
    file_header.get_mut(2..6).ok_or("invalid bitmap size")?.copy_from_slice(&total_u32.to_le_bytes());
    file_header.get_mut(10..14).ok_or("invalid bitmap offset")?.copy_from_slice(&(offset as u32).to_le_bytes());
    let decoded = decode_image(&bytes)?;
    if (decoded.0, decoded.1) != (width, height) {
        return Err("clipboard bitmap dimensions changed during decoding".into());
    }
    Ok(decoded)
}

#[cfg(target_os = "windows")]
pub(super) fn read_windows_image() -> Result<Option<(u32, u32, Vec<u8>)>, String> {
    // The same clipboard lock covers preflight and reading, avoiding a format/size race.
    let _open = clipboard_win::Clipboard::new_attempts(5).map_err(|error| format!("clipboard is busy: {error}"))?;
    let png = clipboard_win::register_format("PNG").map(|value| value.get());
    let (format, is_png) = if let Some(format) = png.filter(|format| clipboard_win::is_format_avail(*format)) {
        (format, true)
    } else if clipboard_win::is_format_avail(17) {
        (17, false) // CF_DIBV5
    } else if clipboard_win::is_format_avail(8) {
        (8, false) // CF_DIB
    } else {
        return Ok(None);
    };
    let length = clipboard_win::size(format).map(|value| value.get()).ok_or("clipboard image has no readable data")?;
    if length > MAX_IMAGE_BYTES {
        return Err("clipboard image exceeds the 512 MiB decode limit".into());
    }
    let mut head = [0u8; 124];
    let read = clipboard_win::raw::get(format, &mut head).map_err(|error| error.to_string())?;
    let header = head.get(..read).ok_or("invalid clipboard image header")?;
    if is_png {
        if !header.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err("invalid clipboard PNG header".into());
        }
        let width = header.get(16..20).and_then(|v| <[u8; 4]>::try_from(v).ok()).map(u32::from_be_bytes).ok_or("truncated clipboard PNG width")?;
        let height = header.get(20..24).and_then(|v| <[u8; 4]>::try_from(v).ok()).map(u32::from_be_bytes).ok_or("truncated clipboard PNG height")?;
        dimensions(width, height)?;
    } else {
        dib_dimensions(header)?;
    }
    let mut bytes = Vec::new();
    let capacity = length.checked_add(if is_png { 0 } else { 14 }).ok_or("clipboard image size overflow")?;
    bytes.try_reserve_exact(capacity).map_err(|_| "not enough memory to read clipboard image")?;
    bytes.resize(length, 0);
    let read = clipboard_win::raw::get(format, &mut bytes).map_err(|error| error.to_string())?;
    if read != length {
        return Err("clipboard image data is truncated".into());
    }
    // Decoding may take appreciable time. Release the system clipboard as soon
    // as our bounded copy is complete, before decompression or color conversion.
    drop(_open);
    if is_png { decode_image(&bytes).map(Some) } else { decode_dib(bytes).map(Some) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_clipboard_dimensions_rejected_without_pixels_v75() {
        assert!(dimensions(u32::MAX, u32::MAX).is_err());
        assert!(dimensions(10_001, 10_000).is_err());
        assert!(dimensions(0, 5).is_err());
        assert_eq!(dimensions(6000, 4000).unwrap(), 96_000_000);
    }

    #[test]
    fn clipboard_revision_cache_only_commits_valid_pixels_v75() {
        let mut cache = RevisionCache::default();
        assert!(!cache.unchanged(Some(1)));
        assert!(cache.imported(Some(1), (2, 2, vec![0; 4])).is_err());
        assert!(!cache.unchanged(Some(1)), "rejected payload must be retried");
        cache.imported(Some(2), (1, 1, vec![255, 0, 0, 255])).unwrap();
        assert!(cache.unchanged(Some(2)));
        assert!(cache.imported(Some(3), (u32::MAX, u32::MAX, vec![])).is_err());
        assert!(!cache.unchanged(Some(3)));
        assert!(cache.unchanged(Some(2)), "a failed new revision does not claim success");
        assert!(!cache.unchanged(None));
    }

    #[test]
    fn clipboard_uncacheable_import_clears_old_bitmap_revision_v75() {
        let mut cache = RevisionCache::default();
        cache.imported(Some(21), (1, 1, vec![255, 0, 0, 255])).unwrap();
        assert!(cache.unchanged(Some(21)));
        cache.imported(None, (1, 1, vec![0, 0, 255, 255])).unwrap();
        assert!(!cache.unchanged(Some(21)));
        assert!(!cache.unchanged(None));
        cache.imported(Some(22), (1, 1, vec![0, 255, 0, 255])).unwrap();
        assert!(cache.unchanged(Some(22)), "immutable bitmap clipboard caching still works");
    }

    #[test]
    fn clipboard_rgba_conversion_owns_existing_pixels_and_matches_formats_v75() {
        let data = vec![23u8; 16];
        let pointer = data.as_ptr();
        let image = Image::from_u8(2, 2, ChannelLayout::Rgba, data).unwrap();
        let output = into_rgba8(image).unwrap();
        assert_eq!(output.as_ptr(), pointer, "RGBA8 consumes the decoded buffer without a second full-image copy");
        for layout in ChannelLayout::ALL {
            for sample in [SampleType::U8, SampleType::U16, SampleType::F16, SampleType::F32] {
                let values: Vec<f32> = (0..51 * layout.channels()).map(|index| (index % 13) as f32 / 12.0).collect();
                let image = Image::from_normalized(17, 3, layout, sample, &values).unwrap();
                let expected = image.to_rgba8();
                assert_eq!(into_rgba8(image).unwrap(), expected, "{layout:?}/{sample:?}");
            }
        }
    }

    #[test]
    fn clipboard_dib_header_bomb_rejected_before_allocating_v75() {
        let mut header = vec![0; 124];
        header[..4].copy_from_slice(&124u32.to_le_bytes());
        header[4..8].copy_from_slice(&i32::MAX.to_le_bytes());
        header[8..12].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(decode_dib(header).unwrap_err().contains("100 megapixel"));
        assert!(decode_dib(vec![0; 12]).is_err());
    }

    #[test]
    fn clipboard_dib_preserves_top_down_alpha_v75() {
        let mut dib = vec![0; 124 + 8];
        dib[..4].copy_from_slice(&124u32.to_le_bytes());
        dib[4..8].copy_from_slice(&2i32.to_le_bytes());
        dib[8..12].copy_from_slice(&(-1i32).to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        dib[52..56].copy_from_slice(&0xff00_0000u32.to_le_bytes());
        dib[124..].copy_from_slice(&[0, 0, 255, 128, 255, 0, 0, 255]);
        let (width, height, pixels) = decode_dib(dib).unwrap();
        assert_eq!((width, height), (2, 1));
        assert_eq!(pixels, [255, 0, 0, 128, 0, 0, 255, 255]);
    }
}
