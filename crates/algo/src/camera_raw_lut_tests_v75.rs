use super::*;
use crate::camera_raw::{CameraRaw, develop, preview_halo};
use serde_json::json;

fn lookup(invert: bool, side: usize) -> ColorLookup {
    let mut values = Vec::new();
    for b in 0..side {
        for g in 0..side {
            for r in 0..side {
                values.extend([r, g, b].map(|v| {
                    let x = v as f32 / (side - 1) as f32;
                    if invert { 1.0 - x } else { x }
                }));
            }
        }
    }
    ColorLookup { enabled: true, size: side, values: Arc::new(values), ..Default::default() }
}

#[test]
fn raw_lut_v75_interpolation_strength_alpha_and_hdr() {
    for tetrahedral in [false, true] {
        let identity = ColorLookup { tetrahedral, ..lookup(false, 5) };
        for rgb in [[0.23, 0.61, 0.88], [2.0, -0.2, 0.5], [0.0, 1.0, 0.0]] {
            for (a, b) in identity.rgb(rgb).into_iter().zip(rgb) {
                assert!((a - b).abs() < 2e-6);
            }
        }
        let lut = ColorLookup { tetrahedral, amount: 50.0, ..lookup(true, 5) };
        let p = CameraRaw { color_lookup: lut, ..Default::default() };
        assert_eq!(preview_halo(&p), Some(0));
        let mut px = vec![[0.1, 0.3, 0.9, 0.27]];
        develop(&mut px, 1, 1, &p, true);
        for c in px[0].iter().take(3) {
            assert!((c - 0.5).abs() < 2e-6);
        }
        assert_eq!(px[0][3], 0.27);
        let mut disabled = p;
        disabled.color_lookup.enabled = false;
        assert!(disabled.is_identity());
        disabled.color_lookup.enabled = true;
        disabled.color_lookup.amount = 0.0;
        assert!(disabled.is_identity());
    }
}

#[test]
fn raw_lut_v75_codec_preserves_f32_and_bounds_expansion() {
    let mut lut = lookup(true, MAX_SIDE);
    // Poorly compressible, finite float32 values exercise the practical worst-case envelope.
    let mut random = 0x12345678u32;
    let values = Arc::make_mut(&mut lut.values);
    for v in values {
        random ^= random << 13;
        random ^= random >> 17;
        random ^= random << 5;
        *v = f32::from_bits(0x3e800000 | (random & 0x007fffff));
    }
    let encoded = serde_json::to_vec(&lut).unwrap();
    assert!(encoded.len() < 600_000, "{}", encoded.len());
    let decoded: ColorLookup = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, lut);
    let mut value = json!(lut);
    value["values"] = json!("!not-base64!");
    assert!(serde_json::from_value::<ColorLookup>(value.clone()).is_err());
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&vec![0; MAX_BYTES * 3]).unwrap();
    value["values"] = json!(base64::engine::general_purpose::STANDARD.encode(encoder.finish().unwrap()));
    assert!(serde_json::from_value::<ColorLookup>(value.clone()).is_err());
    value["values"] = json!("x".repeat(MAX_BYTES * 2 + 1));
    assert!(serde_json::from_value::<ColorLookup>(value).is_err());
}

#[test]
fn raw_lut_v75_domain_validation_and_local_masks_order() {
    let mut lut = lookup(true, 2);
    lut.domain_min = [0.0; 3];
    lut.domain_max = [2.0; 3];
    assert_eq!(lut.rgb([1.0; 3]), [0.5; 3]);
    let mut invalid = lut.clone();
    invalid.domain_max[0] = -1.0;
    assert!(invalid.validate().is_err());
    assert_eq!(invalid.rgb([0.2; 3]), [0.2; 3]);
    let group = crate::camera_raw_masks::LocalAdjustment { exposure: 0.4, ..Default::default() };
    let p = CameraRaw { color_lookup: lookup(true, 2), local_adjustments: vec![group.clone()], ..Default::default() };
    let mut actual = vec![[0.2, 0.3, 0.4, 0.7]; 16];
    let mut expected = actual.clone();
    develop(&mut actual, 4, 4, &p, true);
    develop(&mut expected, 4, 4, &CameraRaw { local_adjustments: Vec::new(), ..p.clone() }, true);
    crate::camera_raw_masks::apply_scaled(&mut expected, 4, 4, &[group], true, 1.0, 0);
    assert_eq!(actual, expected);
    assert_eq!(serde_json::from_value::<CameraRaw>(json!({})).unwrap(), CameraRaw::default());
}
