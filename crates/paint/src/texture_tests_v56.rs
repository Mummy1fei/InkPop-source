//! Synthetic regressions for the independently measured texture transform.
use super::*;
use crate::{GrayTile, Texture};

#[test]
fn height_depth_uses_measured_byte_control_and_negative_paper_relief() {
    // D10/19/25/50/75/100 also checked against original RGBA16 PS exports.
    for (depth, byte) in [(0.0, 0.0), (0.10, 26.0), (0.19, 48.0), (0.25, 64.0), (0.50, 128.0), (0.75, 191.0), (1.0, 255.0)] {
        let coverage = 0.04;
        let expected = (12.0_f32 * byte / 255.0 * coverage - 0.1).clamp(0.0, 1.0);
        assert!((texture_mask_combine(MaskMode::Height, coverage, 0.1, depth) - expected).abs() < 1e-6);
        assert_eq!(texture_mask_combine(MaskMode::Height, 0.0, 0.0, depth), 0.0);
    }
    assert!(texture_mask_combine(MaskMode::Height, 0.4, 0.3, 0.19) > texture_mask_combine(MaskMode::Height, 0.4, 0.7, 0.19));
    // Do not invert Multiply/Overlay or alter independently calibrated Dual.
    for mode in [MaskMode::Multiply, MaskMode::Overlay, MaskMode::ColorBurn] {
        assert_eq!(texture_mask_combine(mode, 0.4, 0.7, 0.19), mask_combine(mode, 0.4, 0.7, 0.19));
    }
    assert_eq!((dual_mask_combine(MaskMode::ColorBurn, 249.0 / 255.0, 0.0) * 255.0).round(), 36.0);
}

#[test]
fn texture_tone_maps_control_units_and_inverts_after_clipping() {
    for (brightness, contrast, gain) in [(73.0, 51.0, 1.0 / 0.49), (-73.0, 0.0, 1.0), (0.0, -25.0, 0.75)] {
        let mut texture = Texture { brightness: brightness / 150.0, contrast: contrast / if contrast >= 0.0 { 100.0 } else { 50.0 }, ..Default::default() };
        for sample in [0.0, 0.13, 0.5, 0.79, 1.0] {
            let expected = ((sample + brightness / 255.0 - 0.5) * gain + 0.5).clamp(0.0_f32, 1.0);
            assert!((TextureTone::new(&texture).apply(sample) - expected).abs() < 1e-6);
            texture.invert = true;
            assert!((TextureTone::new(&texture).apply(sample) - (1.0 - expected)).abs() < 1e-6);
            texture.invert = false;
        }
    }
    for contrast in [-1.0, 1.0, f32::NAN, f32::INFINITY] {
        let tone = TextureTone::new(&Texture { contrast, brightness: f32::NAN, ..Default::default() });
        assert!(tone.apply(0.2).is_finite());
    }
}

#[test]
fn sample_raw_paper_before_tone_and_saturation() {
    let brush = BrushSettings {
        texture: Texture {
            enabled: true,
            pattern: Pattern::Tile(GrayTile { width: 2, height: 1, data: vec![0, 65535].into() }),
            scale: 2.0,
            brightness: 73.0 / 150.0,
            contrast: 0.51,
            ..Default::default()
        },
        ..Default::default()
    };
    // At x=1, raw repeat sampling is 25% white; contrast must follow sampling.
    let context = BrushContext::new(&brush);
    let expected = TextureTone::new(&brush.texture).apply(0.25);
    assert!((context.texture_at(1, 0) - expected).abs() < 1e-6);
    let premature = TextureTone::new(&brush.texture).apply(0.0) * 0.75 + TextureTone::new(&brush.texture).apply(1.0) * 0.25;
    assert!((expected - premature).abs() > 0.1);
    assert_eq!(context.texture_at(1, 0), context.texture_at(5, 0));
}
