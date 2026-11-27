//! Regression coverage for faint dual masks and independently advancing dual dabs.

use super::*;
use crate::{DualBrush, GrayTile};
use photocraft_color::{ColorMode, SampleType};

#[test]
fn dual_color_burn_preserves_the_measured_near_white_transition() {
    // Golden pairs observed in Photoshop's exported 8-bit single sampled dabs
    // at 250 px and at 81 px with a fractional origin, outside the secondary.
    for (primary, alpha) in [(247, 0), (248, 0), (249, 36), (250, 73), (251, 109), (252, 146), (253, 182), (254, 219), (255, 255)] {
        let actual = dual_mask_combine(MaskMode::ColorBurn, primary as f32 / 255.0, 0.0);
        assert_eq!((actual * 255.0).round() as u8, alpha);
    }
    assert_eq!(dual_mask_combine(MaskMode::ColorBurn, 0.0, 1.0), 0.0);
    assert!((dual_mask_combine(MaskMode::ColorBurn, 0.75, 0.5) - 0.5).abs() < 1e-7);
    // Keep more than eight levels on float/high-depth documents and guarantee
    // finite, monotone coverage even at a black or nearly black secondary.
    for mask in [0.0, 0.0001, 0.02, 0.5, 1.0] {
        let mut previous = 0.0;
        for i in 0..=1000 {
            let value = dual_mask_combine(MaskMode::ColorBurn, i as f32 / 1000.0, mask);
            assert!(value.is_finite() && (0.0..=1.0).contains(&value) && value >= previous);
            previous = value;
        }
    }
}

#[test]
fn sampled_dual_color_burn_keeps_partial_edge_coverage_in_the_rendered_stroke() {
    let b = BrushSettings {
        size: 40.0,
        spacing: 1.0,
        pressure_size: false,
        tip: TipShape::Sampled(GrayTile::from_fn(8, 8, |_, _| 250.0 / 255.0)),
        dual_brush: DualBrush {
            enabled: true,
            mode: MaskMode::ColorBurn,
            size: 40.0,
            spacing: 1.0,
            tip: TipShape::Sampled(GrayTile::from_fn(8, 8, |_, _| 0.0)),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut r = StrokeRenderer::new(&b, Some(PixelFormat::RGBA32F), 1.0);
    r.push(&[StrokePoint::new(50.0, 50.0, 1.0), StrokePoint::new(50.125, 50.0, 1.0)]);
    assert_eq!(r.dab_count(), 1);
    assert!((r.cov.get(50, 50) - 250.0 / 255.0).abs() < 0.0001);
    let dual = r.dual.as_ref().unwrap().get(50, 50);
    assert_eq!(dual, 0.0);
    assert!((r.coverage_at(50, 50) - 2.0 / 7.0).abs() < 0.0001);
    let pre = Surface::new(PixelFormat::RGBA32F);
    let mut painted = pre.clone();
    r.composite(&pre, &mut painted, None, false, false);
    assert!((painted.rgba(50, 50)[3] - 2.0 / 7.0).abs() < 0.0001);
}

#[test]
fn dual_burn_resolves_u8_masks_but_retains_higher_depth_edge_precision() {
    let mut brush = BrushSettings::default();
    brush.dual_brush.enabled = true;
    brush.dual_brush.mode = MaskMode::ColorBurn;
    let mut counts = Vec::new();
    for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        let renderer = StrokeRenderer::new(&brush, Some(fmt), 1.0);
        let mut levels = std::collections::HashSet::new();
        for i in 0..=1024 {
            let coverage = 0.97 + 0.03 * i as f32 / 1024.0;
            let masked = renderer.ctx.stroke_mask(coverage, Some(0.0), 0, 0);
            levels.insert((masked * 65535.0).round() as u16);
        }
        counts.push(levels.len());
    }
    assert_eq!(counts[0], 8, "8-bit exported oracle has eight levels near a black secondary");
    assert!(counts[1] > 900 && counts[2] > 900, "higher-depth masks must not inherit 8-bit banding: {counts:?}");
}

#[test]
fn nonlinear_dual_edge_preview_updates_when_only_the_secondary_advances() {
    for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        let brush = BrushSettings {
            size: 40.0,
            spacing: 1.0,
            pressure_size: false,
            opacity: 0.45,
            tip: TipShape::Sampled(GrayTile::from_fn(8, 8, |_, _| 251.0 / 255.0)),
            dual_brush: DualBrush { enabled: true, size: 8.0, spacing: 0.125, mode: MaskMode::ColorBurn, ..Default::default() },
            ..Default::default()
        };
        let pre = Surface::new(fmt);
        let mut live = pre.clone();
        let mut renderer = StrokeRenderer::new(&brush, Some(fmt), 1.0);
        renderer.push(&[StrokePoint::new(50.0, 50.0, 1.0), StrokePoint::new(50.125, 50.0, 1.0)]);
        renderer.composite(&pre, &mut live, None, false, false);
        let before = live.rgba(60, 50)[3];
        assert!(before > 0.0 && before < 0.45, "a partially covered grain edge is required");
        renderer.push(&[StrokePoint::new(60.0, 50.0, 1.0)]);
        assert_eq!(renderer.dab_count(), 1);
        renderer.composite(&pre, &mut live, None, false, false);
        assert!(live.rgba(60, 50)[3] > before);
        let mut oracle = renderer.clone();
        let mut full = pre.clone();
        oracle.composite(&pre, &mut full, None, false, true);
        assert_eq!(live.read_region(Rect::new(20, 20, 80, 80)), full.read_region(Rect::new(20, 20, 80, 80)));
    }
}

#[test]
fn dual_only_motion_updates_incremental_pixels_at_every_depth_without_reaccumulating() {
    let region = Rect::new(0, 0, 100, 100);
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let fmt = PixelFormat::new(ColorMode::Rgb, sample, true);
        for opacity in [1.0, 0.45] {
            for selected in [false, true] {
                let b = BrushSettings {
                    size: 40.0,
                    spacing: 1.0,
                    pressure_size: false,
                    opacity,
                    color: [0.8, 0.2, 0.1, 1.0],
                    dual_brush: DualBrush { enabled: true, size: 8.0, spacing: 0.125, mode: MaskMode::Multiply, ..Default::default() },
                    ..Default::default()
                };
                let mut pre = Surface::new(fmt);
                pre.fill_rect(region, &[0.1, 0.25, 0.4, 0.2]);
                let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
                selection.fill_rect(Rect::new(35, 35, 68, 68), &[0.375]);
                let selection = selected.then_some(&selection);
                let mut live = pre.clone();
                let mut r = StrokeRenderer::new(&b, Some(fmt), 1.0);
                // Establish the initial direction so PathWalker emits its pending
                // first dab. This short motion cannot emit a second primary/dual.
                r.push(&[StrokePoint::new(50.0, 50.0, 1.0), StrokePoint::new(50.125, 50.0, 1.0)]);
                assert_eq!(r.dab_count(), 1);
                assert!(r.coverage_at(50, 50) > 0.0);
                assert_eq!(r.coverage_at(60, 50), 0.0);
                r.composite(&pre, &mut live, selection, false, false);
                assert!(r.cov.dirty.is_empty());
                assert_eq!(live.pixel(60, 50), pre.pixel(60, 50));

                r.push(&[StrokePoint::new(60.0, 50.0, 1.0)]);
                assert_eq!(r.dab_count(), 1, "the primary must remain unchanged");
                assert!(r.dual.as_ref().is_some_and(|d| !d.dirty.is_empty()));
                assert!(r.coverage_at(60, 50) > 0.0, "only new dual coverage reveals this existing primary area");
                let mut oracle = r.clone();
                let mut full = pre.clone();
                oracle.composite(&pre, &mut full, selection, false, true);
                let damage = r.composite(&pre, &mut live, selection, false, false);
                assert!(!damage.is_empty(), "{fmt:?}, opacity={opacity}, selected={selected}");
                assert_ne!(live.pixel(60, 50), pre.pixel(60, 50));
                assert_eq!(live.read_region(region), full.read_region(region), "{fmt:?}, opacity={opacity}, selected={selected}");

                // Neither an idle composite nor a forced repaint may build paint
                // onto the already composited pixels (opacity/selection apply once).
                assert!(r.composite(&pre, &mut live, selection, false, false).is_empty());
                r.composite(&pre, &mut live, selection, false, true);
                r.composite(&pre, &mut live, selection, false, true);
                assert_eq!(live.read_region(region), full.read_region(region), "repaint accumulated coverage: {fmt:?}");
                if selected {
                    assert_eq!(live.pixel(50, 34), pre.pixel(50, 34));
                }
            }
        }
    }
}
