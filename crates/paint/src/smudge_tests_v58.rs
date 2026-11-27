//! Synthetic sampled-tip integration guards. Parent wires this module after the kernel lands.
use super::*;
use crate::{BrushSettings, GrayTile, StrokePoint, TipShape};
use photocraft_geom::Point;

fn sampled() -> BrushSettings {
    BrushSettings {
        size: 8.0,
        tip: TipShape::Sampled(GrayTile::from_fn(8, 4, |x, y| {
            if x % 3 == 0 {
                0.0
            } else if y == 1 {
                0.25
            } else {
                1.0
            }
        })),
        pressure_size: false,
        pressure_opacity: false,
        ..Default::default()
    }
}

fn state(brush: &BrushSettings, strength: f32, fmt: PixelFormat) -> Smudge {
    Smudge::new(strength, None, brush.size).with_brush(brush, fmt)
}

// Coverage is already multiplied by flow, exactly as BrushContext::footprint promises.
fn contact(x: i32, offset: i32, tip_alpha: f32, flow: f32, opacity: f32) -> Footprint {
    let mut dab = crate::Dab::round(Point::new(f64::from(x), 12.0), 4.0, flow);
    dab.opacity = opacity;
    Footprint { dab, index: 0, rect: Rect::new(x + offset, 12, x + offset + 1, 13), cov: vec![tip_alpha * flow] }
}

fn three_patches(fmt: PixelFormat) -> Region {
    let mut surface = Surface::new(fmt);
    let n = fmt.channels();
    for (left, value) in [(0, 1.0), (16, 0.5), (32, 0.0)] {
        let mut color = vec![value; n];
        if let Some(a) = alpha_index(&fmt) {
            color[a] = 1.0;
        }
        surface.fill_rect(Rect::new(left, 0, left + 16, 24), &color);
    }
    Region::read(&surface, Rect::new(0, 0, 48, 24))
}

fn close(a: f32, b: f32) {
    assert!((a - b).abs() <= 2e-6, "actual {a}, expected {b}");
}

#[test]
fn sparse_contacts_use_prewrite_full_square_pickup_with_flow_only_once_in_deposit() {
    let brush = sampled();
    for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::GRAYA8, PixelFormat::CMYKA8] {
        for tip_alpha in [0.125, 0.5, 1.0] {
            for (flow, opacity) in [(1.0, 1.0), (0.5, 1.0), (1.0, 0.5), (0.5, 0.5)] {
                let mut work = three_patches(fmt);
                let initial = work.clone();
                let first = initial.px(8, 12).to_vec();
                let middle = initial.px(24, 12).to_vec();
                let final_old = initial.px(42, 12).to_vec();
                let mut sm = state(&brush, 0.5, fmt);
                sm.dab(&mut work, &contact(8, 0, tip_alpha, flow, opacity));
                assert_eq!(work, initial, "single first contact must be a bit-exact pickup: {fmt:?}");
                sm.dab(&mut work, &contact(24, 0, tip_alpha, flow, opacity));
                let deposit = tip_alpha * flow * 0.5 * opacity;
                for c in 0..work.ch {
                    close(work.px(24, 12)[c], middle[c] + (first[c] - middle[c]) * deposit);
                }
                // +2 was outside the second dab's positive coverage: read it, never paint it.
                assert_eq!(work.px(26, 12), initial.px(26, 12));
                sm.dab(&mut work, &contact(40, 2, tip_alpha, flow, opacity));
                let take = 1.0 - 0.5 * opacity * flow;
                for c in 0..work.ch {
                    let held = first[c] + (middle[c] - first[c]) * take;
                    close(work.px(42, 12)[c], final_old[c] + (held - final_old[c]) * deposit);
                }
                // Also test the contacted reservoir slot: using the just-deposited pixel
                // for pickup would retain too much of the first patch here.
                let mut contacted = initial.clone();
                let mut sm = state(&brush, 0.5, fmt);
                for x in [8, 24, 40] {
                    sm.dab(&mut contacted, &contact(x, 0, tip_alpha, flow, opacity));
                }
                for c in 0..work.ch {
                    let held = first[c] + (middle[c] - first[c]) * take;
                    close(contacted.px(40, 12)[c], final_old[c] + (held - final_old[c]) * deposit);
                }
                let mut committed = Surface::new(fmt);
                work.write(&mut committed);
                let tolerance = if fmt == PixelFormat::RGBA32F {
                    2e-6
                } else if fmt == PixelFormat::RGBA16 {
                    1.0 / 65535.0
                } else {
                    1.0 / 255.0
                };
                for (actual, expected) in committed.read_region(work.rect).iter().zip(&work.data) {
                    assert!((actual - expected).abs() <= tolerance, "native depth commit: {fmt:?}");
                }
            }
        }
    }
}

#[test]
fn zero_active_contacts_cannot_seed_or_recharge_sampled_reservoir() {
    let brush = sampled();
    for inactive in [contact(8, 0, 1.0, 0.0, 1.0), contact(8, 0, 1.0, 1.0, 0.0), contact(8, 0, 0.0, 1.0, 1.0)] {
        let mut work = three_patches(PixelFormat::RGBA32F);
        let before = work.clone();
        let mut sm = state(&brush, 0.5, PixelFormat::RGBA32F);
        sm.dab(&mut work, &inactive);
        assert_eq!(work, before);
        sm.dab(&mut work, &contact(24, 0, 1.0, 1.0, 1.0));
        assert_eq!(work, before, "first active contact must initialize from green, not the skipped white patch");
        sm.dab(&mut work, &inactive);
        sm.dab(&mut work, &contact(40, 0, 1.0, 1.0, 1.0));
        close(work.px(40, 12)[0], 0.25);
    }
}

#[test]
fn sampled_uniform_hdr_tiny_alpha_and_first_contact_preserve_bits() {
    let brush = sampled();
    for color in [[100.0, -2.0, 1.0, 1e-7], [0.7, -0.2, 4.0, 0.0], [0.5, 0.25, 0.125, 1.0]] {
        let mut surface = Surface::new(PixelFormat::RGBA32F);
        surface.fill_rect(Rect::new(0, 0, 48, 24), &color);
        let mut work = Region::read(&surface, Rect::new(0, 0, 48, 24));
        let before: Vec<_> = work.data.iter().map(|v| v.to_bits()).collect();
        let mut sm = state(&brush, 0.65, PixelFormat::RGBA32F);
        for x in [8, 16, 24, 32, 40] {
            sm.dab(&mut work, &contact(x, 0, 0.25, 0.5, 0.75));
            assert_eq!(work.data.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), before);
        }
    }
}

#[test]
fn sampled_strength_99_to_100_has_no_advection_branch_jump() {
    let brush = sampled();
    let run = |strength| {
        let mut work = three_patches(PixelFormat::RGBA32F);
        let mut sm = state(&brush, strength, PixelFormat::RGBA32F);
        for x in [8, 24, 40] {
            sm.dab(&mut work, &contact(x, 0, 0.5, 1.0, 1.0));
        }
        work
    };
    let near = run(0.99);
    let full = run(1.0);
    close(full.px(40, 12)[0], 0.5);
    close(near.px(40, 12)[0], (0.99 + 0.01 * 0.5) * 0.495);
    assert!(near.data.iter().zip(&full.data).all(|(a, b)| (a - b).abs() < 0.02));
}

#[test]
fn real_sampled_stroke_halo_selection_and_transparency_lock_do_not_expand_writes() {
    let area = Rect::new(-24, -24, 80, 48);
    let mut brush = sampled();
    brush.shape_dynamics.enabled = true;
    brush.shape_dynamics.size.control = crate::Control::PenPressure;
    let stroke = Stroke { brush, points: vec![StrokePoint::new(0.0, 12.0, 0.25), StrokePoint::new(44.0, 12.0, 0.25)] };
    for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::GRAYA8, PixelFormat::CMYKA8] {
        let mut source = Surface::new(fmt);
        source.fill_rect(area, &photocraft_raster::from_rgba(&fmt, [0.0, 0.0, 1.0, 0.5]));
        source.fill_rect(Rect::new(-24, -24, 16, 48), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
        let mut selection = Surface::new(PixelFormat::GRAY8);
        selection.fill_rect(Rect::new(0, 0, 32, 24), &[1.0]);
        let mut output = source.clone();
        let mut sm = state(&stroke.brush, 0.65, fmt);
        let mut halo_calls = 0;
        apply_smudge_stroke(&mut output, &stroke, Some(&selection), true, |work, fp| {
            let (cx, cy) = (fp.dab.center.x.floor() as i32, fp.dab.center.y.floor() as i32);
            assert!(work.rect.contains(cx - sm.radius, cy - sm.radius));
            assert!(work.rect.contains(cx + sm.radius, cy + sm.radius), "full reservoir must fit despite a pressure-shrunken footprint");
            halo_calls += 1;
            sm.dab(work, fp);
        });
        assert!(halo_calls > 1);
        let a = alpha_index(&fmt).unwrap();
        let before = source.read_region(area);
        let after = output.read_region(area);
        for y in area.y0..area.y1 {
            for x in area.x0..area.x1 {
                let i = ((y - area.y0) as usize * area.width() as usize + (x - area.x0) as usize) * fmt.channels();
                assert_eq!(after[i + a], before[i + a], "alpha lock: {fmt:?}");
                if selection.sample_channel(x, y, 0) == 0.0 {
                    assert_eq!(&after[i..i + fmt.channels()], &before[i..i + fmt.channels()], "selection cannot expand to pickup halo");
                }
            }
        }
    }
}

#[test]
fn computed_round_still_matches_the_existing_advection_path() {
    let brush = BrushSettings { size: 8.0, pressure_size: false, pressure_opacity: false, ..Default::default() };
    let ctx = crate::BrushContext::new(&brush);
    let mut routed = three_patches(PixelFormat::RGBA32F);
    let mut direct = routed.clone();
    let mut sm = state(&brush, 0.65, PixelFormat::RGBA32F);
    let mut old = state(&brush, 0.65, PixelFormat::RGBA32F);
    for x in 8..=40 {
        let mut dab = crate::Dab::round(Point::new(f64::from(x) + 0.25, 12.5), 4.0, 0.75);
        dab.opacity = 0.8;
        let fp = ctx.footprint(&dab, x as usize);
        sm.dab(&mut routed, &fp);
        old.advect(&mut direct, &fp);
    }
    assert_eq!(routed, direct, "sampled-kernel work must not change computed-round semantics");
}

#[test]
fn sampled_darken_deposition_respects_transparency_and_reads_prewrite_pickup() {
    for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        let mut brush = sampled();
        brush.mode = BlendMode::Darken;
        for (backdrop, expected) in [([0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 0.5, 1.0]), ([0.0; 4], [1.0, 0.0, 0.0, 0.5])] {
            let mut surface = Surface::new(fmt);
            surface.fill_rect(Rect::new(0, 0, 16, 24), &[1.0, 0.0, 0.0, 1.0]);
            surface.fill_rect(Rect::new(16, 0, 48, 24), &backdrop);
            let mut work = Region::read(&surface, Rect::new(0, 0, 48, 24));
            let mut sm = state(&brush, 0.5, fmt).with_format(fmt);
            sm.dab(&mut work, &contact(8, 0, 1.0, 1.0, 1.0));
            sm.dab(&mut work, &contact(24, 0, 1.0, 1.0, 1.0));
            for (actual, want) in work.px(24, 12).iter().zip(expected) {
                close(*actual, want);
            }
            // The carried patch picks up original blue, not the darkened blue.
            if backdrop[3] == 1.0 {
                let q = ((sm.radius as usize * (2 * sm.radius + 1) as usize) + sm.radius as usize) * 4;
                let carry = sm.carry.as_ref().unwrap();
                assert_eq!(&carry[q..q + 4], &[0.5, 0.0, 0.5, 1.0]);
            }
        }
    }
}

#[test]
fn sampled_hdr_darken_and_full_replacement_keep_small_values_after_large_carry() {
    for mode in [BlendMode::Normal, BlendMode::Darken] {
        let mut brush = sampled();
        brush.mode = mode;
        let mut surface = Surface::new(PixelFormat::RGBA32F);
        surface.fill_rect(Rect::new(0, 0, 16, 24), &[1e20, 1e20, 1e20, 1.0]);
        surface.fill_rect(Rect::new(16, 0, 48, 24), &[0.5, 0.5, 0.5, 1.0]);
        let mut work = Region::read(&surface, Rect::new(0, 0, 48, 24));
        let mut sm = state(&brush, 1.0, PixelFormat::RGBA32F).with_format(PixelFormat::RGBA32F);
        for x in [8, 24] {
            sm.dab(&mut work, &contact(x, 0, 1.0, 1.0, 1.0));
        }
        assert_eq!(work.px(24, 12), if mode == BlendMode::Normal { &[1e20, 1e20, 1e20, 1.0] } else { &[0.5, 0.5, 0.5, 1.0] });
    }
    let brush = sampled();
    let mut surface = Surface::new(PixelFormat::RGBA32F);
    surface.fill_rect(Rect::new(0, 0, 16, 24), &[0.5, 0.5, 0.5, 1.0]);
    surface.fill_rect(Rect::new(16, 0, 48, 24), &[1e20, 1e20, 1e20, 1.0]);
    let mut work = Region::read(&surface, Rect::new(0, 0, 48, 24));
    let mut sm = state(&brush, 1.0, PixelFormat::RGBA32F);
    for x in [8, 24] {
        sm.dab(&mut work, &contact(x, 0, 1.0, 1.0, 1.0));
    }
    assert_eq!(work.px(24, 12), &[0.5, 0.5, 0.5, 1.0]);
}
