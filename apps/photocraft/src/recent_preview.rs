//! Bounded, best-effort home-screen previews. Never imports a document or scans user folders.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::SystemTime;

use egui::{Color32, ColorImage};
use photocraft_cms::{Builtin, ColorSpace, Intent, Profile, Transform};
use photocraft_codecs::{self as codecs, ChannelLayout, DecodeOptions, Format, Image, Limits};
use photocraft_ui_egui::RecentThumbnailFn;

mod merged;

const MAX_ENCODED: u64 = 32 << 20;
const MAX_PREVIEW: u64 = 4 << 20;
const MAX_SIDE: u32 = 320;
const CACHE_ENTRIES: usize = 64;
const LIMITS: Limits = Limits { max_width: 16384, max_height: 16384, max_pixels: 40_000_000, max_alloc: 256 << 20 };

struct Request {
    path: PathBuf,
    reply: Sender<Option<ColorImage>>,
}

/// One worker per service, eight queued requests at most. Calling this closure performs no I/O
/// and never waits for queue space. A busy/unavailable service immediately supplies a placeholder.
pub(crate) fn service() -> RecentThumbnailFn {
    let (tx, rx) = mpsc::sync_channel::<Request>(8);
    let started = std::thread::Builder::new().name("recent-previews".into()).spawn(move || {
        let mut cache = Cache::default();
        while let Ok(request) = rx.recv() {
            let image = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cache.load(&request.path))).ok().flatten();
            let _ = request.reply.send(image);
        }
    });
    // Dropping the JoinHandle detaches the worker; dropping the service closes its bounded queue.
    let available = started.is_ok();
    Box::new(move |path: &str| -> Receiver<Option<ColorImage>> {
        let (reply, result) = mpsc::channel();
        let request = Request { path: PathBuf::from(path), reply };
        if available {
            if let Err(error) = tx.try_send(request) {
                let (mpsc::TrySendError::Full(request) | mpsc::TrySendError::Disconnected(request)) = error;
                let _ = request.reply.send(None);
            }
        } else {
            let _ = request.reply.send(None);
        }
        result
    })
}

#[derive(Clone, PartialEq, Eq)]
struct Key {
    path: PathBuf,
    modified: SystemTime,
    len: u64,
}

#[derive(Default)]
struct Cache(VecDeque<(Key, Option<ColorImage>)>);

impl Cache {
    fn load(&mut self, path: &Path) -> Option<ColorImage> {
        let mut file = File::open(path).ok()?;
        let meta = file.metadata().ok()?;
        if !meta.is_file() {
            return None;
        }
        let key = Key { path: path.to_owned(), modified: meta.modified().ok()?, len: meta.len() };
        if let Some(index) = self.0.iter().position(|(cached, _)| *cached == key) {
            let entry = self.0.remove(index)?;
            let image = entry.1.clone();
            self.0.push_back(entry);
            return image;
        }
        // Do not retain stale versions of a path in addition to the current version.
        self.0.retain(|(old, _)| old.path != key.path);
        let image = load(&mut file, key.len, path.extension().and_then(|ext| ext.to_str()).unwrap_or(""));
        self.remember(key, image.clone());
        image
    }

    fn remember(&mut self, key: Key, image: Option<ColorImage>) {
        self.0.push_back((key, image));
        while self.0.len() > CACHE_ENTRIES {
            self.0.pop_front();
        }
    }
}

fn read_at(reader: &mut (impl Read + Seek), offset: u64, count: usize) -> Option<Vec<u8>> {
    reader.seek(SeekFrom::Start(offset)).ok()?;
    let mut bytes = vec![0; count];
    reader.read_exact(&mut bytes).ok()?;
    Some(bytes)
}

fn load(reader: &mut (impl Read + Seek), len: u64, extension: &str) -> Option<ColorImage> {
    match extension.to_ascii_lowercase().as_str() {
        "psd" | "psb" | "psdt" => psd_preview(reader, len).or_else(|| merged::preview(reader, len)),
        "pcraft" => decode(&zip_thumbnail(reader, len)?),
        _ if len <= MAX_ENCODED => decode(&read_at(reader, 0, usize::try_from(len).ok()?)?),
        _ => None,
    }
}

fn decode(bytes: &[u8]) -> Option<ColorImage> {
    if bytes.len() as u64 > MAX_ENCODED {
        return None;
    }
    let image = codecs::decode_with(bytes, &DecodeOptions { limits: LIMITS, keep_orientation: false }).ok()?;
    display(&image, matches!(codecs::detect(bytes), Some(Format::OpenExr | Format::Hdr)))
}

/// Sample by declared channels/depth, convert the embedded profile to sRGB, then area-sample
/// in premultiplied linear light. Work and output allocations are bounded independently of input.
fn display(image: &Image, linear: bool) -> Option<ColorImage> {
    let layout = image.layout();
    let expected = if layout.is_cmyk() {
        ColorSpace::Cmyk
    } else if layout.is_gray() {
        ColorSpace::Gray
    } else {
        ColorSpace::Rgb
    };
    let fallback = if layout.is_cmyk() {
        Builtin::CoatedCmyk
    } else if layout.is_gray() {
        Builtin::SGray
    } else if linear {
        Builtin::LinearSrgb
    } else {
        Builtin::Srgb
    };
    let profile = match image.icc.as_deref() {
        Some(bytes) => {
            if bytes.len() as u64 > MAX_PREVIEW {
                return None;
            }
            let profile = Profile::parse(bytes).ok()?;
            if profile.color_space != expected {
                return None;
            }
            profile
        }
        None => fallback.profile().clone(),
    };
    let transform = Transform::new(&profile, Builtin::Srgb.profile(), Intent::RelativeColorimetric, true).ok()?;
    let (width, height) = image.dimensions();
    LIMITS.check(width, height, layout, image.sample_type()).ok()?;
    let scale = (MAX_SIDE as f64 / width.max(height) as f64).min(1.0);
    let (out_w, out_h) = (((width as f64 * scale).round() as u32).max(1), ((height as f64 * scale).round() as u32).max(1));
    let mut pixels = Vec::with_capacity((out_w * out_h) as usize);
    for y in 0..out_h {
        let (y0, y1) = (y * height / out_h, ((y + 1) * height / out_h).max(y * height / out_h + 1));
        for x in 0..out_w {
            let (x0, x1) = (x * width / out_w, ((x + 1) * width / out_w).max(x * width / out_w + 1));
            let mut sum = [0.0f32; 4];
            let (samples_x, samples_y) = ((x1 - x0).min(4), (y1 - y0).min(4));
            for iy in 0..samples_y {
                let sy = y0 + ((2 * iy + 1) * (y1 - y0) / (2 * samples_y)).min(y1 - y0 - 1);
                for ix in 0..samples_x {
                    let sx = x0 + ((2 * ix + 1) * (x1 - x0) / (2 * samples_x)).min(x1 - x0 - 1);
                    let mut input = [0.0; 4];
                    for (c, value) in input[..layout.color_channels()].iter_mut().enumerate() {
                        *value = finite(image.get(sx, sy, c));
                    }
                    let mut rgb = [0.0; 3];
                    transform.eval_fast(&input[..layout.color_channels()], &mut rgb);
                    let alpha = if layout.has_alpha() { finite(image.get(sx, sy, layout.channels() - 1)).clamp(0.0, 1.0) } else { 1.0 };
                    for (sum, color) in sum[..3].iter_mut().zip(rgb) {
                        *sum += photocraft_color::convert::srgb_to_linear(finite(color).clamp(0.0, 1.0)) * alpha;
                    }
                    sum[3] += alpha;
                }
            }
            let alpha = sum[3] / (samples_x * samples_y) as f32;
            let mut rgba = [0u8; 4];
            if sum[3] > 0.0 {
                for (out, color) in rgba[..3].iter_mut().zip(&sum[..3]) {
                    *out = byte(photocraft_color::convert::linear_to_srgb(*color / sum[3]));
                }
            }
            rgba[3] = byte(alpha);
            pixels.push(Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]));
        }
    }
    Some(ColorImage::new([out_w as usize, out_h as usize], pixels))
}

fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

fn byte(value: f32) -> u8 {
    (finite(value).clamp(0.0, 1.0) * 255.0).round() as u8
}

fn be32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?))
}

/// PSD and PSB share the header/color-data/image-resource framing. Large sections are sought
/// over, never imported. Resource count and thumbnail payload are independently bounded.
fn psd_preview(reader: &mut (impl Read + Seek), len: u64) -> Option<ColorImage> {
    let head = read_at(reader, 0, 30)?;
    if &head[..4] != b"8BPS" || !matches!(u16::from_be_bytes([head[4], head[5]]), 1 | 2) {
        return None;
    }
    let section = 30u64.checked_add(be32(&head, 26)? as u64)?;
    let size = be32(&read_at(reader, section, 4)?, 0)? as u64;
    let mut at = section.checked_add(4)?;
    let end = at.checked_add(size)?;
    if end > len || size > 64 << 20 {
        return None;
    }
    let mut legacy = None;
    for _ in 0..4096 {
        if at == end {
            return legacy;
        }
        if end.checked_sub(at)? < 7 {
            return None;
        }
        let header = read_at(reader, at, 7)?;
        if &header[..4] != b"8BIM" {
            return None;
        }
        let id = u16::from_be_bytes([header[4], header[5]]);
        let name_len = header[6] as u64 + 1;
        let size_at = at.checked_add(6 + name_len + name_len % 2)?;
        if size_at.checked_add(4)? > end {
            return None;
        }
        let count = be32(&read_at(reader, size_at, 4)?, 0)? as u64;
        let data_at = size_at + 4;
        let next = data_at.checked_add(count)?.checked_add(count % 2)?;
        if next > end {
            return None;
        }
        if matches!(id, 1033 | 1036) && count <= MAX_PREVIEW {
            let bytes = read_at(reader, data_at, count as usize)?;
            if let Some(image) = psd_thumbnail(&bytes, id == 1033) {
                if id == 1036 {
                    return Some(image);
                }
                legacy = Some(image);
            }
        }
        at = next;
    }
    None
}

fn psd_thumbnail(bytes: &[u8], bgr: bool) -> Option<ColorImage> {
    let (format, width, height, row) = (be32(bytes, 0)?, be32(bytes, 4)?, be32(bytes, 8)?, be32(bytes, 12)? as usize);
    LIMITS.check(width, height, ChannelLayout::Rgb, codecs::SampleType::U8).ok()?;
    if bytes.get(24..28)? != [0, 24, 0, 1] {
        return None;
    }
    if format == 1 {
        let payload = bytes.get(28..28usize.checked_add(be32(bytes, 20)? as usize)?)?;
        if codecs::detect(payload) != Some(Format::Jpeg) {
            return None;
        }
        let mut image = codecs::decode_with(payload, &DecodeOptions { limits: LIMITS, keep_orientation: false }).ok()?;
        if image.dimensions() != (width, height) {
            return None;
        }
        if bgr {
            image = swap_red_blue(image)?;
        }
        return display(&image, false);
    }
    if format != 0 || row < width as usize * 3 || row.checked_mul(height as usize)? > MAX_PREVIEW as usize {
        return None;
    }
    let raw = bytes.get(28..28usize.checked_add(row.checked_mul(height as usize)?)?)?;
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 3);
    for line in raw.chunks_exact(row) {
        pixels.extend_from_slice(&line[..width as usize * 3]);
    }
    let image = Image::from_u8(width, height, ChannelLayout::Rgb, pixels).ok()?;
    display(&if bgr { swap_red_blue(image)? } else { image }, false)
}

fn swap_red_blue(image: Image) -> Option<Image> {
    let mut pixels = image.to_rgba8();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Image::from_u8(image.width(), image.height(), ChannelLayout::Rgba, pixels).ok()
}

fn le16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn le32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// Native saves STORE their already-compressed thumb.png; DEFLATE is accepted for re-zipped
/// archives. Only the bounded central directory and this entry are read. ZIP64/encryption fail.
fn zip_thumbnail(reader: &mut (impl Read + Seek), len: u64) -> Option<Vec<u8>> {
    let tail_len = len.min(65_557) as usize;
    let tail = read_at(reader, len - tail_len as u64, tail_len)?;
    let eocd = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&at| tail.get(at..at + 4) == Some(b"PK\x05\x06") && le16(&tail, at + 20).is_some_and(|comment| at + 22 + comment as usize == tail.len()))?;
    if le16(&tail, eocd + 4)? != 0 || le16(&tail, eocd + 6)? != 0 || le16(&tail, eocd + 8)? != le16(&tail, eocd + 10)? {
        return None;
    }
    let count = le16(&tail, eocd + 10)? as usize;
    let directory_len = le32(&tail, eocd + 12)? as u64;
    let directory_at = le32(&tail, eocd + 16)? as u64;
    if count > 8192 || directory_len > 2 << 20 || directory_at.checked_add(directory_len)? > len - tail_len as u64 + eocd as u64 {
        return None;
    }
    let directory = read_at(reader, directory_at, directory_len as usize)?;
    let mut at = 0usize;
    for _ in 0..count {
        if directory.get(at..at + 4)? != b"PK\x01\x02" {
            return None;
        }
        let name_len = le16(&directory, at + 28)? as usize;
        let next = at.checked_add(46 + name_len + le16(&directory, at + 30)? as usize + le16(&directory, at + 32)? as usize)?;
        let name = directory.get(at + 46..at + 46 + name_len)?;
        if name == b"thumb.png" {
            let size = le32(&directory, at + 24)? as u64;
            let packed = le32(&directory, at + 20)? as u64;
            let method = le16(&directory, at + 10)?;
            if le16(&directory, at + 8)? & 1 != 0 || !matches!(method, 0 | 8) || size > MAX_PREVIEW || packed > MAX_PREVIEW || (method == 0 && packed != size) {
                return None;
            }
            let local = le32(&directory, at + 42)? as u64;
            let header = read_at(reader, local, 30)?;
            if header.get(..4)? != b"PK\x03\x04" || le16(&header, 8)? != method || le16(&header, 6)? & 1 != 0 || le16(&header, 26)? != 9 {
                return None;
            }
            let data_at = local.checked_add(30 + le16(&header, 26)? as u64 + le16(&header, 28)? as u64)?;
            if data_at.checked_add(packed)? > directory_at || read_at(reader, local + 30, 9)? != b"thumb.png" {
                return None;
            }
            let encoded = read_at(reader, data_at, packed as usize)?;
            let bytes = if method == 8 {
                let mut bytes = Vec::new();
                flate2::read::DeflateDecoder::new(encoded.as_slice()).take(size + 1).read_to_end(&mut bytes).ok()?;
                bytes
            } else {
                encoded
            };
            if bytes.len() as u64 != size {
                return None;
            }
            return (crc32(&bytes) == le32(&directory, at + 16)?).then_some(bytes);
        }
        at = next;
    }
    None
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

#[cfg(test)]
mod tests;
