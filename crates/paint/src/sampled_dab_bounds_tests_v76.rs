use super::*;
use crate::{Control, Dynamic, GrayTile, ShapeDynamics, Smoothing};
use photocraft_geom::Point;

fn synthetic_tip(width: u32, height: u32) -> TipShape {
    TipShape::Sampled(GrayTile::from_fn(width, height, |x, y| {
        // Includes nonzero outer edges and low alpha: bounding must not trim by a threshold.
        if (x + 3 * y) % 11 == 0 { 1.0 / 65535.0 } else { (1 + (x * 17 + y * 13) % 101) as f32 / 102.0 }
    }))
}

fn contexts(width: u32, height: u32, aliased: bool) -> (BrushContext, BrushContext) {
    let brush = BrushSettings { tip: synthetic_tip(width, height), aliased, pressure_size: false, ..Default::default() };
    let old = BrushContext::new(&brush);
    let new = StrokeRenderer::new(&brush, Some(PixelFormat::RGBA8), 1.0).ctx;
    (old, new)
}

fn assert_dab_exact(old: &BrushContext, new: &BrushContext, dab: &Dab, dual: bool) {
    let original = old.dab_rect(dab, dual);
    let bounded = new.dab_rect(dab, dual);
    assert_eq!(bounded.intersect(&original), bounded, "candidate must retain legacy clipping");
    let (mut full, mut tight) = (Vec::new(), Vec::new());
    old.rasterize(dab, dual, original, &mut full);
    new.rasterize(dab, dual, bounded, &mut tight);
    let full_width = original.width() as usize;
    let tight_width = bounded.width() as usize;
    for y in original.y0..original.y1 {
        for x in original.x0..original.x1 {
            let expected = full[(y - original.y0) as usize * full_width + (x - original.x0) as usize];
            let actual = if x >= bounded.x0 && x < bounded.x1 && y >= bounded.y0 && y < bounded.y1 {
                tight[(y - bounded.y0) as usize * tight_width + (x - bounded.x0) as usize]
            } else {
                0.0
            };
            assert_eq!(actual.to_bits(), expected.to_bits(), "outside/inside bound changed at {x},{y}: {dab:?}, dual={dual}");
        }
    }
}

#[test]
fn sampled_bounds_oracle_preserves_all_coverage_with_angle_pressure_and_projection_v76() {
    for (width, height) in [(17, 292), (292, 17), (1, 1), (3, 70), (31, 31)] {
        for aliased in [false, true] {
            let (old, new) = contexts(width, height, aliased);
            for index in 0..24 {
                let mut dab = Dab::round(Point::new(-21.375 + f64::from(index), 31.625), [0.5, 4.1, 31.7, 64.2][index as usize % 4], 0.73);
                dab.roundness = [0.0, 0.01, 0.04, 0.52, 1.0, 2.0][index as usize % 6];
                dab.angle = (index as f32 * 37.1 - 240.0).to_radians();
                dab.flip_x = index % 2 == 0;
                dab.flip_y = index % 3 == 0;
                dab.proj_scale = [1.0, 0.7, 0.05, 0.0][index as usize % 4];
                dab.proj_angle = (index as f32 * 23.0 + 11.0).to_radians();
                assert_dab_exact(&old, &new, &dab, false);
            }
        }
    }
}

#[test]
fn sampled_bounds_966px_narrow_tip_extreme_pressure_oracle_v76() {
    let (old, new) = contexts(17, 292, false);
    for (pressure, angle) in [(0.0, 0.0_f32), (0.5, 33.0), (1.0, 90.0), (0.0, 143.0), (1.0, 0.0)] {
        let mut dab = Dab::round(Point::new(12.25, -5.75), 966.2065 * (0.93 + 0.07 * pressure) / 2.0, 0.81);
        dab.roundness = 0.04 + 0.96 * pressure;
        dab.angle = angle.to_radians();
        if angle == 143.0 {
            dab.proj_scale = 0.05;
            dab.proj_angle = 72.0_f32.to_radians();
        }
        assert_dab_exact(&old, &new, &dab, false);
        let a = old.dab_rect(&dab, false);
        let b = new.dab_rect(&dab, false);
        assert!(u64::from(b.width()) * u64::from(b.height()) < u64::from(a.width()) * u64::from(a.height()) / 4);
    }
}

#[test]
fn sampled_bounds_dual_uses_its_own_tip_and_keeps_sequential_footprints_legacy_v76() {
    let mut brush = BrushSettings { tip: synthetic_tip(292, 17), pressure_size: false, ..Default::default() };
    brush.dual_brush.enabled = true;
    brush.dual_brush.tip = synthetic_tip(17, 292);
    let old = BrushContext::new(&brush);
    let new = StrokeRenderer::new(&brush, Some(PixelFormat::RGBA8), 1.0).ctx;
    let mut dab = Dab::round(Point::new(-9.25, 18.75), 71.0, 0.5);
    dab.roundness = 0.04;
    dab.angle = 47.0_f32.to_radians();
    dab.proj_scale = 0.05;
    dab.proj_angle = 19.0_f32.to_radians();
    for dual in [false, true] {
        assert_dab_exact(&old, &new, &dab, dual);
    }
    let expected_radius = (dab.radius * std::f32::consts::SQRT_2).ceil() as i32 + 1;
    let expected = Rect::new(-10 - expected_radius, 18 - expected_radius, -10 + expected_radius + 1, 18 + expected_radius + 1);
    assert_eq!(old.dab_rect(&dab, false), expected);
    assert_eq!(old.footprint(&dab, 0).rect, expected, "mixer/smudge transport retains historical footprint");
}

#[test]
fn sampled_bounds_preserves_round_and_falls_back_for_extreme_coordinates_v76() {
    let brush = BrushSettings::default();
    let old = BrushContext::new(&brush);
    let new = StrokeRenderer::new(&brush, Some(PixelFormat::RGBA8), 1.0).ctx;
    let mut dab = Dab::round(Point::new(22.25, -23.5), 31.75, 0.7);
    for roundness in [0.01, 0.5, 1.0] {
        dab.roundness = roundness;
        assert_eq!(old.dab_rect(&dab, false), new.dab_rect(&dab, false));
    }
    let (old, new) = contexts(17, 292, false);
    for (radius, center) in [(-1.0, 0.0), (0.0, 0.0), (32.0, 2_000_000.0), (6000.0, 0.0)] {
        dab.radius = radius;
        dab.center = Point::new(center, center);
        assert_eq!(old.dab_rect(&dab, false), new.dab_rect(&dab, false));
    }
}

fn render_reference(
    brush: &BrushSettings,
    pre: &Surface,
    points: &[StrokePoint],
    old_bounds: bool,
    selection: Option<&Surface>,
    lock: bool,
) -> (Surface, usize, Rect) {
    let mut renderer = StrokeRenderer::new(brush, Some(pre.format()), 1.0);
    renderer.ctx.tight_sampled_bounds = !old_bounds;
    let mut target = pre.clone();
    for points in points.chunks(3) {
        renderer.push(points);
        renderer.composite(pre, &mut target, selection, lock, false);
    }
    renderer.finish();
    let damage = renderer.composite(pre, &mut target, selection, lock, true);
    (target, renderer.dab_count(), damage)
}

#[test]
fn sampled_bounds_live_tail_restoration_and_dirty_region_match_legacy_v76() {
    let brush = BrushSettings {
        size: 97.3,
        spacing: 0.57,
        tip: synthetic_tip(17, 292),
        pressure_size: false,
        angle: 33.0,
        smoothing: Smoothing { amount: 0.4, catch_up_on_end: true, ..Default::default() },
        shape_dynamics: ShapeDynamics {
            enabled: true,
            size: Dynamic { control: Control::PenPressure, minimum: 0.93, ..Default::default() },
            roundness: Dynamic { control: Control::PenPressure, minimum: 0.04, ..Default::default() },
            brush_projection: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let pre = Surface::new(PixelFormat::RGBA32F);
    let mut old = StrokeRenderer::new(&brush, Some(pre.format()), 1.0);
    old.ctx.tight_sampled_bounds = false;
    let mut new = StrokeRenderer::new(&brush, Some(pre.format()), 1.0);
    let (mut target_old, mut target_new) = (pre.clone(), pre.clone());
    let (mut tail_old, mut tail_new) = (Rect::EMPTY, Rect::EMPTY);
    for index in 0..12 {
        let mut point = StrokePoint::new(-12.5 + f64::from(index) * 6.2, -22.0 + (f64::from(index) * 0.8).cos() * 13.0, index as f32 / 11.0);
        point.tilt_x = 65.0;
        point.tilt_y = 19.0;
        point.time = f64::from(index) * 8.0;
        let before = target_new.clone();
        let mut candidate_damage = Rect::EMPTY;
        for (renderer, target, tail, candidate) in [(&mut old, &mut target_old, &mut tail_old, false), (&mut new, &mut target_new, &mut tail_new, true)] {
            renderer.push(&[point]);
            let mut damage = *tail;
            if !tail.is_empty() {
                target.write_region(*tail, &pre.read_region(*tail));
                renderer.mark_dirty(*tail);
            }
            damage = damage.union(&renderer.composite(&pre, target, None, false, false));
            *tail = Rect::EMPTY;
            if let Some(mut preview) = renderer.tail_preview() {
                *tail = preview.composite(&pre, target, None, false, false);
                damage = damage.union(tail);
            }
            if candidate {
                candidate_damage = damage;
            }
        }
        let region = target_old.tile_bounds().union(&target_new.tile_bounds());
        assert_eq!(target_new.to_interleaved(region), target_old.to_interleaved(region), "live preview step {index}");
        let after = target_new.read_region(region);
        let prior = before.read_region(region);
        let (after_pixels, after_tail) = after.as_chunks::<4>();
        let (prior_pixels, prior_tail) = prior.as_chunks::<4>();
        assert!(after_tail.is_empty() && prior_tail.is_empty());
        for (offset, (a, b)) in after_pixels.iter().zip(prior_pixels.iter()).enumerate() {
            if a != b {
                let x = region.x0 + (offset % region.width() as usize) as i32;
                let y = region.y0 + (offset / region.width() as usize) as i32;
                assert!(x >= candidate_damage.x0 && x < candidate_damage.x1 && y >= candidate_damage.y0 && y < candidate_damage.y1);
            }
        }
    }
}

#[test]
fn sampled_bounds_paint_erase_and_dual_match_legacy_pixels_at_all_depths_v76() {
    let brush = BrushSettings {
        size: 89.3,
        spacing: 0.57,
        tip: synthetic_tip(17, 292),
        roundness: 1.0,
        color: [0.2, 0.7, 0.4, 0.8],
        opacity: 0.63,
        seed: 739,
        pressure_size: false,
        shape_dynamics: ShapeDynamics {
            enabled: true,
            size: Dynamic { control: Control::PenPressure, minimum: 0.93, ..Default::default() },
            roundness: Dynamic { control: Control::PenPressure, minimum: 0.04, ..Default::default() },
            angle: Dynamic { control: Control::Direction, jitter: 0.12, ..Default::default() },
            flip_x_jitter: true,
            ..Default::default()
        },
        smoothing: Smoothing { amount: 0.0, ..Default::default() },
        ..Default::default()
    };
    let points: Vec<_> = (0..13).map(|i| StrokePoint::new(-40.0 + f64::from(i) * 7.1, -11.0 + (f64::from(i) * 0.7).sin() * 12.0, i as f32 / 12.0)).collect();
    for format in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::CMYKA8] {
        for (erase, lock, dual, aliased) in [(false, false, false, false), (true, false, false, false), (false, true, true, false), (false, false, false, true)]
        {
            let mut current = brush.clone();
            current.erase = erase;
            current.aliased = aliased;
            current.dual_brush.enabled = dual;
            current.dual_brush.tip = synthetic_tip(53, 3);
            current.dual_brush.size = 71.0;
            current.dual_brush.roundness = 0.17;
            let mut pre = Surface::new(format);
            pre.fill_rect(Rect::new(-150, -150, 150, 150), &vec![0.37; format.channels()]);
            let mut selection = Surface::new(PixelFormat::GRAY8);
            selection.fill_rect(Rect::new(-70, -80, 80, 90), &[0.5]);
            let (expected, expected_dabs, _) = render_reference(&current, &pre, &points, true, Some(&selection), lock);
            let (actual, actual_dabs, damage) = render_reference(&current, &pre, &points, false, Some(&selection), lock);
            assert_eq!(actual_dabs, expected_dabs, "spacing/generator must not change");
            let region = expected.tile_bounds().union(&actual.tile_bounds());
            assert_eq!(actual.to_interleaved(region), expected.to_interleaved(region), "{format:?} erase={erase} lock={lock} dual={dual} aliased={aliased}");
            // Every changed pixel remains inside reported final damage.
            let channels = format.channels();
            let after = actual.read_region(region);
            let before = pre.read_region(region);
            for (index, (a, b)) in after.chunks_exact(channels).zip(before.chunks_exact(channels)).enumerate() {
                if a != b {
                    let x = region.x0 + (index % region.width() as usize) as i32;
                    let y = region.y0 + (index / region.width() as usize) as i32;
                    assert!(x >= damage.x0 && x < damage.x1 && y >= damage.y0 && y < damage.y1);
                }
            }
        }
    }
}
