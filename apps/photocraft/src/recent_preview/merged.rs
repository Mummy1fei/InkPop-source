//! Sparse RAW/RLE composite fallback for files without a Photoshop thumbnail. Layer records
//! and pixels are sought over. Extra alpha/spot channels are not mistaken for transparency:
//! the stored PSD composite is displayed with its standard white matte intact.

use super::*;

pub(super) fn preview(reader: &mut (impl Read + Seek), len: u64) -> Option<ColorImage> {
    let header = read_at(reader, 0, 30)?;
    if &header[..4] != b"8BPS" {
        return None;
    }
    let version = u16::from_be_bytes([header[4], header[5]]);
    if !matches!(version, 1 | 2) {
        return None;
    }
    let channels = u16::from_be_bytes([header[12], header[13]]) as usize;
    let (height, width) = (be32(&header, 14)? as usize, be32(&header, 18)? as usize);
    let depth = u16::from_be_bytes([header[22], header[23]]);
    let layout = match u16::from_be_bytes([header[24], header[25]]) {
        1 => ChannelLayout::Gray,
        3 => ChannelLayout::Rgb,
        4 => ChannelLayout::Cmyk,
        _ => return None,
    };
    if !matches!(depth, 8 | 16 | 32)
        || width == 0
        || height == 0
        || width > LIMITS.max_width as usize
        || height > LIMITS.max_height as usize
        || channels < layout.channels()
        || channels > 56
    {
        return None;
    }
    let resource_at = 30u64.checked_add(be32(&header, 26)? as u64)?;
    let resource_len = be32(&read_at(reader, resource_at, 4)?, 0)? as u64;
    let resource_end = resource_at.checked_add(4)?.checked_add(resource_len)?;
    if resource_len > 64 << 20 || resource_end > len {
        return None;
    }
    let icc = profile(reader, resource_at + 4, resource_end)?;
    let section_width = if version == 2 { 8 } else { 4 };
    let section = read_at(reader, resource_end, section_width)?;
    let layer_len = if version == 2 { u64::from_be_bytes(section.as_slice().try_into().ok()?) } else { be32(&section, 0)? as u64 };
    let composite_at = resource_end.checked_add(section_width as u64)?.checked_add(layer_len)?;
    let compression = read_at(reader, composite_at, 2)?;
    let compression = u16::from_be_bytes([compression[0], compression[1]]);
    if !matches!(compression, 0 | 1) {
        return None;
    }
    let row_bytes = width.checked_mul(depth as usize / 8)?;
    let rows = channels.checked_mul(height)?;
    let mut data_at = composite_at.checked_add(2)?;
    let offsets = if compression == 1 {
        let count_width = if version == 2 { 4 } else { 2 };
        let table_len = rows.checked_mul(count_width)?;
        if table_len > 4 << 20 || data_at.checked_add(table_len as u64)? > len {
            return None;
        }
        let table = read_at(reader, data_at, table_len)?;
        data_at += table_len as u64;
        let mut offsets = Vec::with_capacity(rows + 1);
        offsets.push(0u64);
        let mut total = 0u64;
        for count in table.chunks_exact(count_width) {
            let count = if version == 2 { be32(count, 0)? as u64 } else { u16::from_be_bytes([count[0], count[1]]) as u64 };
            total = total.checked_add(count)?;
            offsets.push(total);
        }
        if data_at.checked_add(total)? > len {
            return None;
        }
        Some(offsets)
    } else {
        if data_at.checked_add(rows.checked_mul(row_bytes)? as u64)? > len {
            return None;
        }
        None
    };
    let scale = (MAX_SIDE as f64 / width.max(height) as f64).min(1.0);
    let (out_w, out_h) = (((width as f64 * scale).round() as usize).max(1), ((height as f64 * scale).round() as usize).max(1));
    let mut values = vec![0.0f32; out_w * out_h * layout.channels()];
    let mut read_bytes = 0u64;
    for channel in 0..layout.channels() {
        for y in 0..out_h {
            let sy = ((2 * y + 1) * height / (2 * out_h)).min(height - 1);
            let row_index = channel * height + sy;
            let (offset, packed) = match offsets.as_ref() {
                Some(offsets) => (*offsets.get(row_index)?, offsets.get(row_index + 1)?.checked_sub(*offsets.get(row_index)?)?),
                None => ((row_index * row_bytes) as u64, row_bytes as u64),
            };
            // Bound per-row allocation and aggregate physical reads even for hostile PackBits.
            read_bytes = read_bytes.checked_add(packed)?;
            if packed > (row_bytes * 2 + 1024) as u64 || read_bytes > 128 << 20 {
                return None;
            }
            let encoded = read_at(reader, data_at.checked_add(offset)?, packed as usize)?;
            let row = if compression == 1 { photocraft_psd::compression::packbits::decode(&encoded, row_bytes).ok()? } else { encoded };
            for x in 0..out_w {
                let sx = ((2 * x + 1) * width / (2 * out_w)).min(width - 1);
                let value = sample(&row, sx, depth)?;
                values[(y * out_w + x) * layout.channels() + channel] = if layout.is_cmyk() { 1.0 - value } else { value };
            }
        }
    }
    let image = Image::from_normalized(out_w as u32, out_h as u32, layout, codecs::SampleType::F32, &values).ok()?.with_icc(icc);
    display(&image, false)
}

fn profile(reader: &mut (impl Read + Seek), mut at: u64, end: u64) -> Option<Option<Vec<u8>>> {
    let mut profile = None;
    for _ in 0..4096 {
        if at == end {
            return Some(profile);
        }
        if end.checked_sub(at)? < 7 {
            return None;
        }
        let header = read_at(reader, at, 7)?;
        if &header[..4] != b"8BIM" {
            return None;
        }
        let name_len = header[6] as u64 + 1;
        let size_at = at.checked_add(6 + name_len + name_len % 2)?;
        if size_at.checked_add(4)? > end {
            return None;
        }
        let size = be32(&read_at(reader, size_at, 4)?, 0)? as u64;
        let data_at = size_at + 4;
        at = data_at.checked_add(size)?.checked_add(size % 2)?;
        if at > end {
            return None;
        }
        if u16::from_be_bytes([header[4], header[5]]) == 1039 {
            if size > MAX_PREVIEW {
                return None;
            }
            profile = Some(read_at(reader, data_at, size as usize)?);
        }
    }
    None
}

fn sample(row: &[u8], x: usize, depth: u16) -> Option<f32> {
    let at = x.checked_mul(depth as usize / 8)?;
    Some(match depth {
        8 => *row.get(at)? as f32 / 255.0,
        16 => u16::from_be_bytes(row.get(at..at + 2)?.try_into().ok()?) as f32 / 65535.0,
        32 => f32::from_be_bytes(row.get(at..at + 4)?.try_into().ok()?),
        _ => return None,
    })
}
