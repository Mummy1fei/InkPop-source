use super::*;
use std::io::Cursor;

fn png() -> Vec<u8> {
    let image = Image::from_u8(640, 2, ChannelLayout::Rgb, [20, 120, 230].repeat(1280)).unwrap();
    codecs::encode(&image, Format::Png, &Default::default()).unwrap()
}

#[test]
fn recent_preview_png_is_bounded_and_correctly_colored() {
    let bytes = png();
    let image = decode(&bytes).unwrap();
    assert_eq!(image.size, [320, 1]);
    let pixel = image.pixels[0].to_array();
    for (actual, expected) in pixel.into_iter().zip([20, 120, 230, 255]) {
        assert!((actual as i16 - expected as i16).abs() <= 2);
    }
    assert!(decode(b"not an image").is_none());
    assert!(load(&mut Cursor::new(Vec::new()), MAX_ENCODED + 1, "png").is_none());
}

#[test]
fn recent_preview_channel_sample_and_embedded_profile_conversion() {
    for sample in codecs::SampleType::ALL {
        for layout in ChannelLayout::ALL {
            let values = match layout {
                ChannelLayout::Gray => vec![0.5],
                ChannelLayout::GrayA => vec![0.5, 0.5],
                ChannelLayout::Rgb => vec![0.2, 0.5, 0.8],
                ChannelLayout::Rgba => vec![0.2, 0.5, 0.8, 0.5],
                ChannelLayout::Cmyk => vec![0.1, 0.2, 0.3, 0.0],
                ChannelLayout::CmykA => vec![0.1, 0.2, 0.3, 0.0, 0.5],
            };
            let image = Image::from_normalized(1, 1, layout, sample, &values).unwrap();
            assert_eq!(display(&image, false).unwrap().size, [1, 1]);
        }
    }
    let linear = Image::from_normalized(1, 1, ChannelLayout::Rgb, codecs::SampleType::F32, &[0.25; 3])
        .unwrap()
        .with_icc(Some(Builtin::LinearSrgb.profile().to_bytes().to_vec()));
    let pixel = display(&linear, false).unwrap().pixels[0].r();
    assert!((pixel as i16 - 137).abs() <= 2, "linear profile must encode to display sRGB");
    let bad = linear.with_icc(Some(vec![0; 128]));
    assert!(display(&bad, false).is_none());
}

fn thumbnail(format: u32, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in [format, 1, 1, 4, 4, payload.len() as u32] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes.extend_from_slice(&[0, 24, 0, 1]);
    bytes.extend_from_slice(payload);
    bytes
}

fn psd(version: u16, resources: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = b"8BPS".to_vec();
    bytes.extend_from_slice(&version.to_be_bytes());
    bytes.resize(26, 0);
    bytes.extend_from_slice(&0u32.to_be_bytes());
    let mut blocks = Vec::new();
    for (id, data) in resources {
        blocks.extend_from_slice(b"8BIM");
        blocks.extend_from_slice(&id.to_be_bytes());
        blocks.extend_from_slice(&[0, 0]);
        blocks.extend_from_slice(&(data.len() as u32).to_be_bytes());
        blocks.extend_from_slice(data);
        if data.len() % 2 != 0 {
            blocks.push(0);
        }
    }
    bytes.extend_from_slice(&(blocks.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&blocks);
    bytes
}

#[test]
fn recent_preview_psd_psb_raw_jpeg_and_legacy_order() {
    for version in [1, 2] {
        for (id, raw) in [(1036, vec![240, 20, 10, 0]), (1033, vec![10, 20, 240, 0])] {
            let bytes = psd(version, &[(id, thumbnail(0, &raw))]);
            let image = psd_preview(&mut Cursor::new(&bytes), bytes.len() as u64).unwrap();
            assert!(image.pixels[0].r() > 230 && image.pixels[0].b() < 20);
        }
    }
    let rgb = Image::from_u8(1, 1, ChannelLayout::Rgb, vec![240, 20, 10]).unwrap();
    let jpeg = codecs::encode(&rgb, Format::Jpeg, &Default::default()).unwrap();
    let bytes = psd(1, &[(1036, thumbnail(1, &jpeg))]);
    assert!(psd_preview(&mut Cursor::new(&bytes), bytes.len() as u64).unwrap().pixels[0].r() > 220);
    for end in 0..bytes.len() {
        assert!(psd_preview(&mut Cursor::new(&bytes[..end]), end as u64).is_none());
    }
}

#[test]
fn recent_preview_psd_only_seeks_resources_and_rejects_huge_thumbnail() {
    let bytes = psd(1, &[(1036, thumbnail(0, &[20, 30, 40, 0]))]);
    let resource_end = bytes.len() as u64;
    // A multi-gigabyte document needs no layer data to read this embedded preview.
    assert!(psd_preview(&mut Cursor::new(&bytes), 8 << 30).is_some());
    let mut bad = bytes.clone();
    let width_at = 34 + 12 + 4;
    bad[width_at..width_at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(psd_preview(&mut Cursor::new(&bad), resource_end).is_none());
}

#[test]
fn recent_preview_own_psd_psb_saves_without_thumbnail_use_sparse_merged_raw_or_rle() {
    use photocraft_color::{Color, ColorMode, SampleType};
    use photocraft_doc::{Document, Size};
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk] {
        for depth in SampleType::ALL {
            let document = Document::with_background("synthetic recent", Size::new(8, 5), mode, depth, Color::rgb(0.18, 0.47, 0.82));
            let expected = photocraft_compose::flatten(&document).px[0];
            for extension in ["psd", "psb"] {
                let bytes = photocraft_io::export(&document, extension, &Default::default()).unwrap().bytes;
                assert!(psd_preview(&mut Cursor::new(&bytes), bytes.len() as u64).is_none(), "fixture must exercise missing embedded preview");
                let preview = load(&mut Cursor::new(&bytes), bytes.len() as u64, extension).unwrap();
                assert_eq!(preview.size, [8, 5]);
                for (actual, expected) in preview.pixels[0].to_array().into_iter().zip(expected) {
                    assert!((actual as i16 - byte(expected) as i16).abs() <= 5, "{mode:?}/{depth:?}/{extension}");
                }
                let truncated = &bytes[..bytes.len() - 1];
                assert!(load(&mut Cursor::new(truncated), truncated.len() as u64, extension).is_none());
            }
        }
    }
}

#[test]
fn recent_preview_merged_honors_icc_and_keeps_white_matte_without_guessing_extra_alpha() {
    use photocraft_color::{Color, ColorMode, SampleType};
    use photocraft_doc::{Document, Size};
    use std::sync::Arc;
    let mut document = Document::with_background("synthetic ICC", Size::new(2, 2), ColorMode::Rgb, SampleType::F32, Color::rgb(0.25, 0.25, 0.25));
    document.icc_profile = Some(Arc::new(Builtin::LinearSrgb.profile().to_bytes().to_vec()));
    let bytes = photocraft_io::export(&document, "psd", &Default::default()).unwrap().bytes;
    let preview = load(&mut Cursor::new(&bytes), bytes.len() as u64, "psd").unwrap();
    assert!((preview.pixels[0].r() as i16 - 137).abs() <= 2);

    let document = Document::with_background("synthetic matte", Size::new(2, 2), ColorMode::Rgb, SampleType::U8, Color::rgba(1.0, 0.0, 0.0, 0.5));
    let bytes = photocraft_io::export(&document, "psb", &Default::default()).unwrap().bytes;
    let preview = load(&mut Cursor::new(&bytes), bytes.len() as u64, "psb").unwrap();
    assert_eq!(preview.pixels[0].a(), 255, "stored white-matted composite remains opaque");
    assert!(preview.pixels[0].r() > 245 && preview.pixels[0].g() > 110 && preview.pixels[0].b() > 110);
    let mut no_zip_fallback = bytes.clone();
    // Locate the merged compression after the PSB resource + layer/mask sections.
    let resources = 30 + be32(&bytes, 26).unwrap() as usize;
    let layers = resources + 4 + be32(&bytes, resources).unwrap() as usize;
    let composite = layers + 8 + u64::from_be_bytes(bytes[layers..layers + 8].try_into().unwrap()) as usize;
    no_zip_fallback[composite..composite + 2].copy_from_slice(&2u16.to_be_bytes());
    assert!(load(&mut Cursor::new(&no_zip_fallback), no_zip_fallback.len() as u64, "psb").is_none());
}

fn zip(data: &[u8], deflate: bool) -> Vec<u8> {
    use std::io::Write;
    let encoded = if deflate {
        let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    } else {
        data.to_vec()
    };
    let method = if deflate { 8u16 } else { 0u16 };
    let mut bytes = vec![0; 30];
    bytes[..4].copy_from_slice(b"PK\x03\x04");
    bytes[8..10].copy_from_slice(&method.to_le_bytes());
    bytes[14..18].copy_from_slice(&crc32(data).to_le_bytes());
    bytes[18..22].copy_from_slice(&(encoded.len() as u32).to_le_bytes());
    bytes[22..26].copy_from_slice(&(data.len() as u32).to_le_bytes());
    bytes[26..28].copy_from_slice(&9u16.to_le_bytes());
    bytes.extend_from_slice(b"thumb.png");
    bytes.extend_from_slice(&encoded);
    let central_at = bytes.len() as u32;
    let mut central = vec![0; 46];
    central[..4].copy_from_slice(b"PK\x01\x02");
    central[10..12].copy_from_slice(&method.to_le_bytes());
    central[16..20].copy_from_slice(&crc32(data).to_le_bytes());
    central[20..24].copy_from_slice(&(encoded.len() as u32).to_le_bytes());
    central[24..28].copy_from_slice(&(data.len() as u32).to_le_bytes());
    central[28..30].copy_from_slice(&9u16.to_le_bytes());
    central.extend_from_slice(b"thumb.png");
    bytes.extend_from_slice(&central);
    let mut end = vec![0; 22];
    end[..4].copy_from_slice(b"PK\x05\x06");
    end[8..10].copy_from_slice(&1u16.to_le_bytes());
    end[10..12].copy_from_slice(&1u16.to_le_bytes());
    end[12..16].copy_from_slice(&(central.len() as u32).to_le_bytes());
    end[16..20].copy_from_slice(&central_at.to_le_bytes());
    bytes.extend_from_slice(&end);
    bytes
}

#[test]
fn recent_preview_pcraft_reads_only_thumbnail_and_checks_zip_bounds_crc_and_inflation() {
    for deflate in [false, true] {
        let bytes = zip(&png(), deflate);
        let thumbnail = zip_thumbnail(&mut Cursor::new(&bytes), bytes.len() as u64).unwrap();
        assert!(decode(&thumbnail).is_some());
        for end in 0..bytes.len() {
            assert!(zip_thumbnail(&mut Cursor::new(&bytes[..end]), end as u64).is_none());
        }
        let mut corrupt = bytes.clone();
        corrupt[39] ^= 1;
        assert!(zip_thumbnail(&mut Cursor::new(&corrupt), corrupt.len() as u64).is_none());
    }
    let mut bomb = zip(&vec![0; 16_384], true);
    let central = bomb.len() - 22 - 55;
    bomb[central + 24..central + 28].copy_from_slice(&1u32.to_le_bytes());
    assert!(zip_thumbnail(&mut Cursor::new(&bomb), bomb.len() as u64).is_none());
    let mut cache = Cache::default();
    for index in 0..65 {
        cache.remember(Key { path: PathBuf::from(index.to_string()), modified: SystemTime::UNIX_EPOCH, len: 1 }, None);
    }
    assert_eq!(cache.0.len(), CACHE_ENTRIES);
    assert_eq!(cache.0.front().unwrap().0.path, PathBuf::from("1"));
}

#[test]
fn recent_preview_service_only_reads_synthetic_file_and_invalidates_cache() {
    let path = std::env::temp_dir().join(format!(
        "inkpop-recent-preview-{}-{}.png",
        std::process::id(),
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::write(&path, png()).unwrap();
    let request = service();
    assert_eq!(request(path.to_str().unwrap()).recv_timeout(std::time::Duration::from_secs(5)).unwrap().unwrap().size, [320, 1]);
    let mut cache = Cache::default();
    assert!(cache.load(&path).is_some());
    std::fs::write(&path, b"invalid replacement").unwrap();
    assert!(cache.load(&path).is_none());
    assert_eq!(cache.0.len(), 1);
    std::fs::remove_file(path).unwrap();
}
