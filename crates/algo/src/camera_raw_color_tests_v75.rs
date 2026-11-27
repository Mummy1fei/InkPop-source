use super::*;
use crate::camera_raw::{CameraRaw, develop};
use crate::camera_raw_lut::ColorLookup;

fn identity_lookup() -> ColorLookup {
    let mut values = Vec::new();
    for b in 0..2 {
        for g in 0..2 {
            for r in 0..2 {
                values.extend([r as f32, g as f32, b as f32]);
            }
        }
    }
    ColorLookup { enabled: true, size: 2, values: Arc::new(values), ..Default::default() }
}

#[test]
fn raw_color_v75_working_pipeline_matches_independent_cms_including_masks() {
    for source in [Builtin::Srgb, Builtin::SGray] {
        let spaces = ColorSpaces { working: RgbSpace::ProPhotoRgb, source_profile: source.id().into(), ..Default::default() };
        let params = CameraRaw {
            contrast: 22.0,
            saturation: 17.0,
            local_adjustments: vec![crate::camera_raw_masks::LocalAdjustment { exposure: 0.4, ..Default::default() }],
            color_spaces: spaces,
            ..Default::default()
        };
        let mut actual = vec![[0.2, 0.2, 0.2, 0.73]; 48];
        if source == Builtin::Srgb {
            actual.fill([0.2, 0.4, 0.7, 0.73]);
        }
        let mut expected = actual.clone();
        let working = Builtin::ProPhotoCompat.profile();
        let forward = photocraft_cms::cached(source.profile(), working, Default::default()).unwrap();
        let backward = photocraft_cms::cached(working, source.profile(), Default::default()).unwrap();
        for pixel in &mut expected {
            let mut out = [0.; 3];
            forward.eval(&pixel[..forward.inputs()], &mut out);
            pixel[..3].copy_from_slice(&out);
        }
        develop(&mut expected, 8, 6, &CameraRaw { color_spaces: Default::default(), ..params.clone() }, true);
        for pixel in &mut expected {
            let mut out = [0.; 3];
            backward.eval(&pixel[..3], &mut out);
            if backward.outputs() == 1 {
                out = [out[0]; 3];
            }
            pixel[..3].copy_from_slice(&out);
        }
        develop(&mut actual, 8, 6, &params, true);
        for (a, b) in actual.iter().flatten().zip(expected.iter().flatten()) {
            assert!((a - b).abs() < 2e-6, "{source:?}: {a} {b}");
        }
        assert_eq!(actual[0][3], 0.73);
    }
}

#[test]
fn raw_color_v75_hdr_roundtrip_and_profile_gamma_exposure_are_real() {
    for working in [RgbSpace::Srgb, RgbSpace::DisplayP3, RgbSpace::AdobeRgb, RgbSpace::ProPhotoRgb] {
        let params = CameraRaw {
            color_lookup: identity_lookup(),
            color_spaces: ColorSpaces { working, source_profile: "srgb".into(), ..Default::default() },
            ..Default::default()
        };
        let original = [1.8, 2.1, 1.4, 0.53];
        let mut pixels = vec![original];
        develop(&mut pixels, 1, 1, &params, true);
        for (a, b) in pixels[0].iter().zip(original) {
            assert!((a - b).abs() < 1e-4, "{working:?}: {pixels:?}");
        }
        let mut exposed = vec![[0.2, 0.4, 0.7, 0.53]];
        let params = CameraRaw { exposure: 0.5, ..params };
        develop(&mut exposed, 1, 1, &params, true);
        assert!(exposed[0][0] > 0.2 && exposed[0][2] > 0.7);
        assert_eq!(exposed[0][3], 0.53);
    }
}

#[test]
fn raw_color_v75_profile_context_is_exact_bounded_and_rejects_invalid_channels() {
    let mut custom = Builtin::Srgb.profile().clone();
    custom.description = "Synthetic gamma 1.9".into();
    custom.trc = Some(std::array::from_fn(|_| photocraft_cms::Curve::Gamma(1.9)));
    let custom = custom.with_encoded_bytes();
    let encoded = encode_source(&custom).unwrap();
    assert!(encoded.starts_with("icc:"));
    let spaces = ColorSpaces { working: RgbSpace::DisplayP3, output: RgbSpace::AdobeRgb, source_profile: encoded };
    let decoded: ColorSpaces = serde_json::from_slice(&serde_json::to_vec(&spaces).unwrap()).unwrap();
    assert_eq!(decoded.source().unwrap().to_bytes(), custom.to_bytes());
    assert!(decoded.prepared(true).unwrap().is_some());
    assert!(ColorSpaces { source_profile: "coated-cmyk".into(), ..spaces.clone() }.validate().is_err());
    assert!(ColorSpaces { source_profile: "icc:bad!".into(), ..spaces }.validate().is_err());
    assert!(serde_json::from_value::<ColorSpaces>(serde_json::json!({"sourceProfile":"x".repeat(MAX_PROFILE_TEXT + 1)})).is_err());
}

#[test]
fn raw_color_v75_sampled_rgb_and_gray_curves_reject_hdr_without_clipping_pixels() {
    for gray in [false, true] {
        let mut profile = if gray { Builtin::SGray.profile().clone() } else { Builtin::Srgb.profile().clone() };
        let table = photocraft_cms::Curve::Table(vec![0.0, 0.214, 1.0]);
        if gray {
            profile.gray_trc = Some(table);
        } else {
            profile.trc = Some(std::array::from_fn(|_| table.clone()));
        }
        let profile = profile.with_encoded_bytes();
        let source_profile = encode_source(&profile).unwrap();
        let spaces = ColorSpaces { working: RgbSpace::AdobeRgb, source_profile, ..Default::default() };
        assert!(spaces.validate_depth(false).is_ok(), "integer documents retain table TRC support");
        assert_eq!(spaces.validate_depth(true).unwrap_err(), HDR_TABLE_ERROR);
        assert_eq!(validate_float_profile(&profile).unwrap_err(), HDR_TABLE_ERROR);
        let before = vec![[1.8, 1.8, 1.8, 0.61]];
        let mut pixels = before.clone();
        let params = CameraRaw { color_lookup: identity_lookup(), color_spaces: spaces, ..Default::default() };
        develop(&mut pixels, 1, 1, &params, true);
        assert_eq!(pixels, before, "invalid direct calls cannot clip HDR pixels");
        let unchecked = photocraft_cms::cached(&profile, Builtin::AdobeRgbCompat.profile(), Default::default()).unwrap();
        let mut old_result = [0.; 3];
        unchecked.eval(&before[0][..unchecked.inputs()], &mut old_result);
        assert!(old_result.iter().all(|v| *v < 1.1), "fixture must expose the table clamp: {old_result:?}");
    }
}
