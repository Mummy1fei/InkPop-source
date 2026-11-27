//! Bitwise Normal sampled-reservoir oracle. Wire as a child of retouch in cfg(test).
//! Scalar traversal intentionally does not reuse matching_rows or SampledKernel.
use super::*;
use crate::{BrushContext, BrushSettings, Dab, GrayTile, TipShape};
use photocraft_color::{ColorMode, SampleType};
use photocraft_geom::Point;

// Independent preoptimization scalar-model oracle reconstructed from the old
// recurrence; not a verbatim source snapshot. Historical native GUI outputs are
// the separate RGBA8 golden. No row traversal or production mode helper is reused.
fn scalar_normal(sm: &mut Smudge, work: &mut Region, fp: &Footprint) {
    if !fp.dab.alpha.is_finite() || !fp.dab.opacity.is_finite() || fp.dab.alpha <= 0.0 || fp.dab.opacity <= 0.0 {
        return;
    }
    let center = (fp.dab.center.x, fp.dab.center.y);
    if !center.0.is_finite() || !center.1.is_finite() || !fp.cov.iter().any(|c| c.is_finite() && *c > 0.0) || sm.last_center == Some(center) {
        return;
    }
    let (r, n) = (sm.radius, work.ch);
    let Some(side) = r.checked_mul(2).and_then(|v| v.checked_add(1)).and_then(|v| usize::try_from(v).ok()).filter(|v| *v > 0) else { return };
    let Some(len) = side.checked_mul(side).and_then(|v| v.checked_mul(n)) else { return };
    if n == 0 || n > 16 || len > 134_217_728 || work.rect.is_empty() {
        return;
    }
    let (cx, cy) = (center.0.floor() as i32, center.1.floor() as i32);
    let alpha = sm.alpha.filter(|a| *a < n);
    if sm.carry.is_none() {
        let mut carry = vec![0.0; len];
        for q in 0..side * side {
            let y = cy.saturating_add((q / side) as i32).saturating_sub(r);
            let x = cx.saturating_add((q % side) as i32).saturating_sub(r);
            if !work.rect.contains(x, y) {
                continue;
            }
            let q = q * n;
            let old = work.px(x, y);
            let a = alpha.map_or(1.0, |a| old[a]);
            for (c, value) in old.iter().enumerate() {
                carry[q + c] = if alpha.is_some() && alpha != Some(c) { *value * a } else { *value };
            }
        }
        sm.carry = Some(carry);
        sm.last_center = Some(center);
        return;
    }
    let Some(carry) = sm.carry.as_mut().filter(|v| v.len() == len) else { return };
    let strength = (sm.strength * fp.dab.opacity).clamp(0.0, 1.0);
    let pickup = 1.0 - strength * fp.dab.alpha.clamp(0.0, 1.0);
    for q in 0..side * side {
        let y = cy.saturating_add((q / side) as i32).saturating_sub(r);
        let x = cx.saturating_add((q % side) as i32).saturating_sub(r);
        if !work.rect.contains(x, y) {
            continue;
        }
        let coverage = if fp.rect.contains(x, y) { fp.at(x, y).clamp(0.0, 1.0) } else { 0.0 };
        let k = coverage * strength;
        let q = q * n;
        let held = &mut carry[q..q + n];
        let pixel = work.px_mut(x, y);
        let old_alpha = alpha.map_or(1.0, |a| pixel[a]);
        let held_alpha = alpha.map_or(1.0, |a| held[a]);
        let out_alpha = old_alpha + (held_alpha - old_alpha) * k;
        for (c, (value, carried)) in pixel.iter_mut().zip(held).enumerate() {
            let old = if alpha.is_some() && alpha != Some(c) { *value * old_alpha } else { *value };
            let paint = *carried;
            let deposited = if k == 1.0 { paint } else { old + (paint - old) * k };
            let next = *carried + (old - *carried) * pickup;
            if k > 0.0 && !(old == deposited && old_alpha == out_alpha) {
                *value = if alpha.is_none() || alpha == Some(c) {
                    deposited
                } else if out_alpha > 0.0 {
                    deposited / out_alpha
                } else {
                    0.0
                };
            }
            *carried = next;
        }
    }
    sm.last_center = Some(center);
}

fn brush() -> BrushSettings {
    let mut b = BrushSettings {
        size: 8.0,
        pressure_size: false,
        pressure_opacity: false,
        tip: TipShape::Sampled(GrayTile::from_fn(8, 4, |x, y| {
            if x % 3 == 0 {
                0.0
            } else if y == 1 {
                0.25
            } else {
                1.0
            }
        })),
        ..Default::default()
    };
    b.shape_dynamics.enabled = true;
    b.shape_dynamics.angle.jitter = 1.0;
    b
}

fn formats() -> [PixelFormat; 10] {
    [
        PixelFormat::RGBA8,
        PixelFormat::RGBA16,
        PixelFormat::RGBA32F,
        PixelFormat::GRAYA8,
        PixelFormat::GRAY8,
        PixelFormat::CMYKA8,
        PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F32, alpha: false },
        PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F32, alpha: true },
        PixelFormat { mode: ColorMode::Cmyk, sample: SampleType::F32, alpha: false },
        PixelFormat { mode: ColorMode::Cmyk, sample: SampleType::F32, alpha: true },
    ]
}

fn varied_region(format: PixelFormat, rect: Rect, opaque: bool) -> Region {
    let n = format.channels();
    let alpha = alpha_index(&format);
    let palette = [0.0, -0.0, 0.125, 1.0, 0.8, -2.0, 100.0, 1e-30];
    let alphas = [1.0, 0.0, 1e-7, 0.5, 1.0, -0.0, 1e-30];
    let mut values = vec![0.0; rect.width() as usize * rect.height() as usize * n];
    for (i, pixel) in values.chunks_exact_mut(n).enumerate() {
        for (c, value) in pixel.iter_mut().enumerate() {
            *value = palette[(i * 3 + c) % palette.len()];
        }
        if let Some(a) = alpha {
            pixel[a] = if opaque { 1.0 } else { alphas[i % alphas.len()] };
        }
    }
    let mut surface = Surface::new(format);
    surface.write_region(rect, &values);
    Region::read(&surface, rect)
}

fn assert_bits(a: &[f32], b: &[f32], context: &str) {
    assert_eq!(a.len(), b.len(), "{context}");
    for (i, (a, b)) in a.iter().zip(b).enumerate() {
        assert_eq!(a.to_bits(), b.to_bits(), "{context}: channel {i}, {a:?} != {b:?}");
    }
}

fn compare_contacts(format: PixelFormat, strength: f32, initial: Region, contacts: &[Footprint]) {
    let b = brush();
    let mut fast = Smudge::new(strength, None, b.size).with_brush(&b, format);
    let mut scalar = fast.clone();
    let mut a = initial.clone();
    let mut b = initial;
    for (i, fp) in contacts.iter().enumerate() {
        fast.dab(&mut a, fp);
        scalar_normal(&mut scalar, &mut b, fp);
        let context = format!("{format:?} strength {strength} contact {i}");
        assert_bits(&a.data, &b.data, &context);
        match (&fast.carry, &scalar.carry) {
            (Some(a), Some(b)) => assert_bits(a, b, &context),
            (a, b) => assert_eq!(a.is_some(), b.is_some(), "{context}"),
        }
        assert_eq!(fast.last_center.map(|(x, y)| (x.to_bits(), y.to_bits())), scalar.last_center.map(|(x, y)| (x.to_bits(), y.to_bits())), "{context}");
    }
}

#[test]
fn sampled_rows_match_scalar_every_contact_across_native_formats_and_sparse_dynamics() {
    let b = brush();
    let ctx = BrushContext::new(&b);
    let mut contacts = Vec::new();
    for (i, x) in [-4.0, 0.0, 3.25, 5.0, 5.0, 8.0, 12.5, 17.0, 24.0, 32.0, 40.0].into_iter().enumerate() {
        let mut dab = Dab::round(Point::new(x, 10.0 + (i % 3) as f64), [4.0, 1.25, 2.5][i % 3], [1.0, 0.25, 0.0, 0.75][i % 4]);
        dab.angle = i as f32 * 0.37;
        dab.opacity = [1.0, 0.5, 0.75, 0.0, 1.0][i % 5];
        let mut fp = ctx.footprint(&dab, i);
        if i == 5 {
            fp.cov.fill(0.0);
        }
        contacts.push(fp.clone());
        // Exact repeated active contact: neither recharges nor deposits.
        if i == 0 || i == 6 {
            contacts.push(fp);
        }
    }
    // Inactive contacts before first seeding and after a real dab.
    let mut inactive = contacts[0].clone();
    inactive.dab.alpha = 0.0;
    contacts.insert(0, inactive);
    for format in formats() {
        for opaque in [false, true] {
            for strength in [0.0, 0.5, 0.65, 1.0] {
                compare_contacts(format, strength, varied_region(format, Rect::new(0, 0, 36, 24), opaque), &contacts);
            }
        }
    }
}

#[test]
fn sampled_rows_match_scalar_with_mixed_nan_zero_coverage_and_native_signed_zeros() {
    let rect = Rect::new(-8, -8, 16, 16);
    let contacts: Vec<_> = [0.0, 1.0, 2.0, 2.0, 3.0]
        .into_iter()
        .enumerate()
        .map(|(i, x)| {
            let mut dab = Dab::round(Point::new(x, 0.0), 1.0, 1.0);
            dab.opacity = if i == 2 { 0.5 } else { 1.0 };
            Footprint { dab, index: i, rect: Rect::new(x as i32, 0, x as i32 + 4, 1), cov: vec![1.0, f32::NAN, -0.0, 0.25] }
        })
        .collect();
    for format in formats() {
        for strength in [0.0, 0.65, 1.0] {
            compare_contacts(format, strength, varied_region(format, rect, true), &contacts);
        }
    }
}

#[test]
fn large_independent_rows_match_scalar_bits_at_every_contact() {
    let mut settings = brush();
    settings.size = 300.0;
    settings.shape_dynamics.enabled = true;
    settings.shape_dynamics.angle.jitter = 1.0;
    let ctx = BrushContext::new(&settings);
    for format in formats() {
        for opaque in [false, true] {
            let mut fast = Smudge::new(0.65, None, settings.size).with_brush(&settings, format);
            let mut scalar = fast.clone();
            let mut a = varied_region(format, Rect::new(0, 0, 620, 420), opaque);
            let mut b = a.clone();
            for (i, x) in [160.0, 230.25, 320.5].into_iter().enumerate() {
                let mut dab = Dab::round(Point::new(x, 170.0 + i as f64 * 11.25), [120.0, 150.0, 90.0][i], [1.0, 0.5, 0.75][i]);
                dab.opacity = [1.0, 0.7, 0.5][i];
                dab.angle = i as f32 * 0.6;
                let fp = ctx.footprint(&dab, i);
                fast.dab(&mut a, &fp);
                scalar_normal(&mut scalar, &mut b, &fp);
                let context = format!("large {format:?} opaque {opaque} contact {i}");
                assert_bits(&a.data, &b.data, &context);
                assert_bits(fast.carry.as_ref().unwrap(), scalar.carry.as_ref().unwrap(), &context);
            }
        }
    }
}

#[test]
fn sampled_rows_match_scalar_at_saturated_xy_with_repeated_pixel_aliases() {
    for high_x in [false, true] {
        for high_y in [false, true] {
            let x0 = if high_x { i32::MAX - 12 } else { i32::MIN };
            let y0 = if high_y { i32::MAX - 12 } else { i32::MIN };
            let rect = Rect::new(x0, y0, x0 + 12, y0 + 12);
            let x = if high_x { f64::from(i32::MAX) } else { f64::from(i32::MIN) };
            let y = if high_y { f64::from(i32::MAX) } else { f64::from(i32::MIN) };
            let contacts: Vec<_> = [(x, y), (x + 0.25, y + 0.25), (x, y), (x - 0.25, y - 0.25)]
                .into_iter()
                .enumerate()
                .map(|(i, (x, y))| {
                    let mut dab = Dab::round(Point::new(x, y), 1.0, [1.0, 0.5][i % 2]);
                    dab.opacity = [0.75, 1.0][i % 2];
                    Footprint { dab, index: i, rect, cov: (0..144).map(|p| if p % 3 == 0 { 0.0 } else { 0.5 }).collect() }
                })
                .collect();
            for format in formats() {
                for strength in [0.5, 1.0] {
                    let mut initial = varied_region(format, Rect::new(0, 0, 12, 12), false);
                    // Relocate the already-quantized small buffer, avoiding unrelated
                    // Surface tile-coordinate arithmetic at i32 extrema.
                    initial.rect = rect;
                    compare_contacts(format, strength, initial, &contacts);
                }
            }
        }
    }
}
