//! Independent sampled secondary orientations must replay identically in live,
//! committed and high-depth strokes, without changing primary dynamics.
use crate::{BrushSettings, DualBrush, GrayTile, MaskMode, StrokePoint, TipShape, dynamics::DabGenerator, render::StrokeRenderer};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::Surface;

fn brush() -> BrushSettings {
    BrushSettings {
        size: 35.0,
        spacing: 0.07,
        pressure_size: false,
        color: [0.7, 0.2, 0.1, 1.0],
        dual_brush: DualBrush {
            enabled: true,
            size: 29.0,
            spacing: 0.16,
            random_rotation: true,
            tip: TipShape::Sampled(GrayTile::from_fn(13, 13, |x, y| if (x + 3 * y) % 7 < 2 { 0.8 } else { 0.01 })),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn secondary_rotations_vary_per_stamp_and_seed_without_changing_primary_or_spacing() {
    let points = [StrokePoint::new(40.0, 40.0, 0.6), StrokePoint::new(480.0, 70.0, 0.6)];
    let generate = |brush: &BrushSettings| {
        let mut g = DabGenerator::new(brush, 1.0);
        let (mut primary, mut secondary) = (Vec::new(), Vec::new());
        g.push(&points, &mut primary, &mut secondary);
        g.finish(&mut primary, &mut secondary);
        (primary, secondary)
    };
    let mut b = brush();
    let (primary, secondary) = generate(&b);
    assert_eq!((primary.clone(), secondary.clone()), generate(&b));
    let angles: std::collections::HashSet<_> = secondary.iter().map(|d| d.angle.to_bits()).collect();
    assert!(angles.len() > 50, "each stamp needs an independent orientation");
    assert!(secondary.iter().all(|d| (0.0..std::f32::consts::TAU).contains(&d.angle)));
    b.dual_brush.random_rotation = false;
    let (legacy_primary, legacy_secondary) = generate(&b);
    assert_eq!(primary, legacy_primary);
    assert_eq!(secondary.len(), legacy_secondary.len());
    for (a, b) in secondary.iter().zip(&legacy_secondary) {
        assert_eq!((a.center, a.radius, a.roundness, a.alpha, a.flip_x), (b.center, b.radius, b.roundness, b.alpha, b.flip_x));
        assert_eq!(b.angle, 0.0);
    }
    b.dual_brush.random_rotation = true;
    b.seed += 1;
    assert_ne!(secondary, generate(&b).1);
}

#[test]
fn legacy_dual_json_preserves_explicit_geometry_and_new_rotation_round_trips() {
    let legacy: DualBrush = serde_json::from_value(serde_json::json!({"enabled":true,"angle":90,"roundness":0.8})).unwrap();
    assert!(!legacy.random_rotation);
    assert_eq!((legacy.angle, legacy.roundness), (90.0, 0.8));
    let b = brush();
    let restored: BrushSettings = serde_json::from_slice(&serde_json::to_vec(&b).unwrap()).unwrap();
    assert_eq!(b, restored);
}

#[test]
fn rotated_dual_live_tail_and_commit_agree_at_all_depths() {
    let points: Vec<_> = (0..=40)
        .map(|i| {
            let t = f64::from(i) / 40.0;
            StrokePoint::new(40.0 + t * 80.0, 70.0 + (t * std::f64::consts::TAU).sin() * 20.0, 0.35 + t as f32 * 0.5)
        })
        .collect();
    let region = Rect::new(0, 0, 160, 130);
    for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        for mode in [MaskMode::Multiply, MaskMode::ColorBurn, MaskMode::Overlay] {
            let mut b = brush();
            b.opacity = 0.7;
            b.dual_brush.mode = mode;
            b.transfer.enabled = true;
            b.transfer.opacity = crate::Dynamic { minimum: 0.82, ..crate::Dynamic::controlled(crate::Control::PenPressure) };
            let mut pre = Surface::new(fmt);
            pre.fill_rect(region, &[0.2, 0.3, 0.4, 0.1]);
            let mut full = pre.clone();
            let mut oracle = StrokeRenderer::new(&b, Some(fmt), 1.0);
            oracle.push(&points);
            oracle.finish();
            oracle.composite(&pre, &mut full, None, false, true);
            assert_ne!(pre.read_region(region), full.read_region(region));
            for chunk in [1, 7, 19] {
                let mut live = pre.clone();
                let mut renderer = StrokeRenderer::new(&b, Some(fmt), 1.0);
                for packet in points.chunks(chunk) {
                    renderer.push(packet);
                    // Finishing a preview clone must not consume random states
                    // or add another orientation to the continuing renderer.
                    let mut tail = renderer.clone();
                    tail.finish();
                    let mut preview = pre.clone();
                    tail.composite(&pre, &mut preview, None, false, true);
                    renderer.composite(&pre, &mut live, None, false, false);
                }
                renderer.finish();
                renderer.composite(&pre, &mut live, None, false, false);
                assert_eq!(full.read_region(region), live.read_region(region), "{fmt:?} {mode:?} chunk={chunk}");
            }
        }
    }
}
