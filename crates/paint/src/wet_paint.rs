//! Spatial nib pickup and an opt-in, bounded 2D wet paint model.
//! Pigment colour, water and thickness are separate: shading never contaminates pickup.
use crate::{
    Stroke,
    mixer::{MixerSettings, MixerState},
    retouch::{Footprint, apply_dab_stroke, over_native},
};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba_into, to_rgba};

#[path = "pigment.rs"]
mod pigment;

// Match canvas detail for ordinary tips; bound memory for very large tips.
pub fn pickup_side(size: f32) -> usize {
    if size.is_finite() { size.ceil().clamp(64., 512.) as usize } else { 64 }
}
pub(crate) fn map_sample(map: &[[f32; 4]], side: usize, x: usize, y: usize) -> [f32; 4] {
    let old = (map.len() as f64).sqrt() as usize;
    if old == 0 || old * old != map.len() {
        return [0.; 4];
    }
    if old == side {
        return map.get(y * side + x).copied().unwrap_or([0.; 4]);
    }
    // Resizing paint requires premultiplied interpolation: hidden RGB in an
    // empty cell must not tint its opaque neighbours.
    let u = ((x as f32 + 0.5) * old as f32 / side as f32 - 0.5).clamp(0., (old - 1) as f32);
    let v = ((y as f32 + 0.5) * old as f32 / side as f32 - 0.5).clamp(0., (old - 1) as f32);
    let (ix, iy) = (u.floor() as usize, v.floor() as usize);
    let (fx, fy) = (u - ix as f32, v - iy as f32);
    let mut out = [0.; 4];
    for (xx, yy, weight) in [
        (ix, iy, (1. - fx) * (1. - fy)),
        ((ix + 1).min(old - 1), iy, fx * (1. - fy)),
        (ix, (iy + 1).min(old - 1), (1. - fx) * fy),
        ((ix + 1).min(old - 1), (iy + 1).min(old - 1), fx * fy),
    ] {
        if let Some(px) = map.get(yy * old + xx) {
            let a = px[3].clamp(0., 1.) * weight;
            for c in 0..3 {
                out[c] += px[c] * a;
            }
            out[3] += a;
        }
    }
    if out[3] > 1e-6 {
        for c in 0..3 {
            out[c] /= out[3];
        }
    }
    out
}
pub(crate) type PigmentSpace = Option<(std::sync::Arc<photocraft_cms::Transform>, std::sync::Arc<photocraft_cms::Transform>)>;
pub(crate) fn pigment_space(m: &MixerSettings) -> PigmentSpace {
    use photocraft_cms::{
        Builtin, Profile,
        transform::{TransformOptions, cached},
    };
    let src = Profile::parse(m.pigment_profile.as_ref()?).ok()?;
    let dst = Builtin::Srgb.profile();
    if src.content_hash() == dst.content_hash() {
        return None;
    }
    Some((cached(&src, dst, TransformOptions::default()).ok()?, cached(dst, &src, TransformOptions::default()).ok()?))
}
pub(crate) fn mix_in_space(a: [f32; 4], b: [f32; 4], t: f32, pigments: bool, space: &PigmentSpace) -> [f32; 4] {
    if t <= 0.0 || a == b {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let Some((to, from)) = space.as_ref().filter(|_| pigments) else { return pigment_mix(a, b, t, pigments) };
    let mut ar = [0.; 3];
    let mut br = [0.; 3];
    to.eval(&a[..3], &mut ar);
    to.eval(&b[..3], &mut br);
    let c = pigment_mix([ar[0], ar[1], ar[2], a[3]], [br[0], br[1], br[2], b[3]], t, true);
    let mut rgb = [0.; 3];
    from.eval(&c[..3], &mut rgb);
    [rgb[0], rgb[1], rgb[2], c[3]]
}

/// Mix straight RGBA channels, optionally modelling physical pigment in sRGB.
pub fn pigment_mix(a: [f32; 4], b: [f32; 4], t: f32, pigments: bool) -> [f32; 4] {
    if !pigments {
        let t = t.clamp(0.0, 1.0);
        return std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t);
    }
    if t.is_nan() || t <= 0. || a == b {
        return a;
    }
    if t >= 1. {
        return b;
    }
    let aa = [a[0].clamp(0., 1.), a[1].clamp(0., 1.), a[2].clamp(0., 1.)];
    let bb = [b[0].clamp(0., 1.), b[1].clamp(0., 1.), b[2].clamp(0., 1.)];
    let mut rgb = pigment::mix_srgb(aa, bb, t);
    for (i, c) in rgb.iter_mut().enumerate() {
        *c += (a[i] - aa[i]) * (1. - t) + (b[i] - bb[i]) * t;
    }
    [rgb[0], rgb[1], rgb[2], a[3] + (b[3] - a[3]) * t]
}

pub(crate) fn nib_mapping(fp: &Footprint, side: usize) -> impl Fn(i32, i32) -> usize + use<> {
    let d = &fp.dab;
    let (s, c) = d.angle.sin_cos();
    let radius = d.radius.max(0.01);
    let (cx, cy, roundness, flip_x, flip_y) = (d.center.x as f32, d.center.y as f32, d.roundness.max(0.01), d.flip_x, d.flip_y);
    move |x, y| {
        let dx = x as f32 + 0.5 - cx;
        let dy = y as f32 + 0.5 - cy;
        let mut u = (dx * c - dy * s) / (radius * 2.0);
        let mut v = (dx * s + dy * c) / (radius * 2.0 * roundness);
        if flip_x {
            u = -u;
        }
        if flip_y {
            v = -v;
        }
        let ix = ((u + 0.5) * side as f32).clamp(0.0, (side - 1) as f32) as usize;
        let iy = ((v + 0.5) * side as f32).clamp(0.0, (side - 1) as f32) as usize;
        iy * side + ix
    }
}

/// Seed a clean pickup map by inverse mapping, once at contact. Forward pixel
/// binning leaves holes whenever pressure makes the footprint smaller than its
/// reservoir grid. Activating those holes later (pressure/rotation changes)
/// used to replace carried paint abruptly with a fresh canvas sample.
/// Existing carried paint is retained; interpolation is alpha weighted.
pub(crate) fn seed_pickup(nib: &mut [[f32; 4]], side: usize, fp: &Footprint, source: &crate::retouch::Region, fmt: &photocraft_color::PixelFormat) {
    let d = &fp.dab;
    let (sin, cos) = d.angle.sin_cos();
    let diameter = d.radius.max(0.01) * 2.;
    for (i, cell) in nib.iter_mut().enumerate() {
        if cell[3] > 0. {
            continue;
        }
        let mut u = ((i % side) as f32 + 0.5) / side as f32 - 0.5;
        let mut v = ((i / side) as f32 + 0.5) / side as f32 - 0.5;
        if d.flip_x {
            u = -u;
        }
        if d.flip_y {
            v = -v;
        }
        let x = d.center.x as f32 + diameter * (u * cos + v * d.roundness.max(0.01) * sin) - 0.5;
        let y = d.center.y as f32 + diameter * (-u * sin + v * d.roundness.max(0.01) * cos) - 0.5;
        let (ix, iy) = (x.floor() as i32, y.floor() as i32);
        let (fx, fy) = (x - ix as f32, y - iy as f32);
        let mut color = [0.; 4];
        let mut contact_weight = 0.;
        for (sx, sy, weight) in [
            (ix, iy, (1. - fx) * (1. - fy)),
            (ix.saturating_add(1), iy, fx * (1. - fy)),
            (ix, iy.saturating_add(1), (1. - fx) * fy),
            (ix.saturating_add(1), iy.saturating_add(1), fx * fy),
        ] {
            if weight <= 0.
                || sx < source.rect.x0.max(fp.rect.x0)
                || sx >= source.rect.x1.min(fp.rect.x1)
                || sy < source.rect.y0.max(fp.rect.y0)
                || sy >= source.rect.y1.min(fp.rect.y1)
            {
                continue;
            }
            let pixel = to_rgba(fmt, source.px(sx, sy));
            let contact = weight * fp.at(sx, sy);
            let alpha = pixel[3].clamp(0., 1.) * contact;
            for channel in 0..3 {
                color[channel] += pixel[channel] * alpha;
            }
            color[3] += alpha;
            contact_weight += contact;
        }
        if color[3] > 1e-6 {
            for channel in 0..3 {
                color[channel] /= color[3];
            }
            color[3] = (color[3] / contact_weight.max(1e-6)).clamp(0., 1.);
            *cell = color;
        }
    }
}

/// Loaded dry paint accumulates once per dab. Explicit Photoshop Mixer oracles
/// distinguish dense 1% spacing from 25%; normalising this dose by travel made
/// dense dry strokes incorrectly faint. Load consumption remains distance-based.
pub(crate) fn dry_deposition(flow: f32, coverage: f32) -> f32 {
    flow.clamp(0., 1.) * coverage.clamp(0., 1.)
}

/// Optical transfer for existing wet pickup, integrated over travel and final
/// contact coverage. Dry paint and an empty-pickup loaded fallback do not use it.
/// A small interpolated table keeps exponentiation out of the wet pixel loop.
pub(crate) struct Deposition {
    table: [f32; 257],
}

impl Deposition {
    pub(crate) fn new(flow: f32, travel: f32) -> Self {
        let flow = flow.clamp(0., 1.);
        let mut table = [0.; 257];
        for (i, value) in table.iter_mut().enumerate() {
            // Concentrate samples near opacity one, where fractional powers
            // curve sharply. This retains the 1/1024 tail resolution with a
            // quarter of the exponentiations of a uniform table.
            let coverage = if i <= 192 { i as f32 / 204.8 } else { 0.9375 + (i - 192) as f32 / 1024. };
            *value = if travel < 0. { coverage * flow } else { 1. - (1. - coverage * flow.min(0.999)).powf(travel) };
        }
        Self { table }
    }

    pub(crate) fn at(&self, coverage: f32) -> f32 {
        let coverage = coverage.clamp(0., 1.);
        let position = if coverage <= 0.9375 { coverage * 204.8 } else { 192. + (coverage - 0.9375) * 1024. };
        let i = position as usize;
        let a = self.table.get(i).copied().unwrap_or(0.);
        let b = self.table.get(i + 1).copied().unwrap_or(a);
        a + (b - a) * (position - i as f32)
    }
}

/// Return distance, optical travel and reservoir drain. Negative travel marks
/// the initial tap, whose alpha remains exactly toolbar Flow × contact coverage.
pub(crate) fn transport(fp: &Footprint, previous: &mut Option<photocraft_geom::Point>, load: f32) -> (f32, f32, f32) {
    let first = previous.is_none();
    let p = fp.dab.center;
    let diameter = (fp.dab.radius * 2.).max(0.01);
    let distance = previous.map_or(diameter * 0.15, |q| (p.x - q.x).hypot(p.y - q.y) as f32);
    *previous = Some(p);
    let travel = distance / diameter;
    let deposit = if first { -1. } else { travel * 4. };
    let drain = (1. - 0.08 * (1. - load.clamp(0., 1.))).powf(if first { 1. } else { travel * 10. });
    (distance, deposit, drain)
}

pub fn apply_spatial_mixer(
    target: &mut Surface,
    sample: Option<&Surface>,
    stroke: &Stroke,
    m: &MixerSettings,
    state: &mut MixerState,
    selection: Option<&Surface>,
    lock_transparency: bool,
) -> Rect {
    if m.wet <= 0. {
        return apply_loaded_mixer(target, stroke, m, state, selection, lock_transparency);
    }
    // One stateful wet renderer for commands and live preview. Besides avoiding
    // divergent pickup algorithms, this keeps revisited paint and float precision
    // independent of how input points are divided into batches.
    let mut live = crate::mixer_live::MixerStroke::new(target, sample, &stroke.brush, m, std::mem::take(state));
    let damage = live.push(&stroke.points, target, selection, lock_transparency);
    let damage = damage.union(&live.finish(target, selection, lock_transparency));
    *state = live.state;
    damage
}

// Wet=0 disables canvas pickup and Mix. Keep the real tip/texture/dual masks,
// but avoid a source snapshot, pickup bins and colour mixing for each dab.
fn apply_loaded_mixer(target: &mut Surface, stroke: &Stroke, m: &MixerSettings, state: &mut MixerState, selection: Option<&Surface>, lock: bool) -> Rect {
    let fmt = target.format();
    let side = pickup_side(stroke.brush.size);
    let paint: Vec<_> = if state.loaded_nib.is_empty() {
        state.reservoir.into_iter().collect()
    } else {
        (0..side * side).map(|i| map_sample(&state.loaded_nib, side, i % side, i / side)).collect()
    };
    let mut native = Vec::with_capacity(paint.len() * fmt.channels());
    let mut alpha = Vec::with_capacity(paint.len());
    for mut px in paint {
        alpha.push(px[3].clamp(0., 1.));
        px[3] = 1.;
        let mut out = [0.; 5];
        let n = from_rgba_into(&fmt, px, &mut out);
        native.extend_from_slice(&out[..n]);
    }
    let mut amount = if alpha.is_empty() { 0. } else { state.amount.clamp(0., 1.) };
    let mut st = stroke.clone();
    st.brush.opacity = 1.;
    st.brush.flow = 1.;
    let mut previous = None;
    let damage = apply_dab_stroke(target, &st, selection, lock, 0, |work, fp| {
        let (_, _, drain) = transport(fp, &mut previous, m.load);
        let index = nib_mapping(fp, side);
        let r = fp.rect.intersect(&work.rect);
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let coverage = fp.at(x, y);
                if coverage <= 0. {
                    continue;
                }
                let i = if alpha.len() == 1 { 0 } else { index(x, y) };
                if let (Some(a), Some(color)) = (alpha.get(i), native.get(i * fmt.channels()..(i + 1) * fmt.channels())) {
                    over_native(&fmt, work.px_mut(x, y), color, dry_deposition(m.flow, coverage * fp.dab.opacity.clamp(0., 1.) * a * amount), lock);
                }
            }
        }
        if !state.canvas_loaded {
            amount *= drain;
        }
    });
    state.amount = amount;
    damage
}

/// Deposit water and paint volume through the same tip, dual brush, texture and selection.
pub fn deposit_fields(water: &mut Surface, height: &mut Surface, stroke: &Stroke, m: &MixerSettings, selection: Option<&Surface>) -> Rect {
    if water.channels() != 1 || height.channels() != 1 {
        return deposit_fields_separate(water, height, stroke, m, selection);
    }
    let ds = crate::dabs(stroke);
    if ds.is_empty() {
        return Rect::EMPTY;
    }
    let ctx = crate::BrushContext::new(&stroke.brush);
    let bounds = ds.iter().fold(Rect::EMPTY, |r, d| r.union(&ctx.dab_rect(d, false)));
    let original_water = water.read_region(bounds);
    let original_height = height.read_region(bounds);
    let mut w = original_water.clone();
    let mut h = original_height.clone();
    let width = bounds.width() as usize;
    let wet = m.wet.clamp(0., 1.);
    let thickness = m.thickness.clamp(0., 1.);
    for (i, d) in ds.iter().enumerate() {
        let fp = ctx.footprint(d, i);
        let r = fp.rect.intersect(&bounds);
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let k = fp.at(x, y);
                let index = (y - bounds.y0) as usize * width + (x - bounds.x0) as usize;
                if let Some(v) = w.get_mut(index) {
                    *v = (*v + k * wet * (1. - *v)).clamp(0., 1.);
                }
                if let Some(v) = h.get_mut(index) {
                    *v = (*v + k * thickness * 0.1).min(4.);
                }
            }
        }
    }
    let opacity = stroke.brush.opacity.clamp(0., 1.);
    for y in bounds.y0..bounds.y1 {
        for x in bounds.x0..bounds.x1 {
            let k = opacity * selection.map_or(1., |s| s.sample_channel(x, y, 0));
            let i = (y - bounds.y0) as usize * width + (x - bounds.x0) as usize;
            for (output, original) in [(&mut w, &original_water), (&mut h, &original_height)] {
                if let (Some(v), Some(&o)) = (output.get_mut(i), original.get(i)) {
                    *v = o + (*v - o) * k.max(0.);
                }
            }
        }
    }
    water.write_region(bounds, &w);
    height.write_region(bounds, &h);
    bounds
}

// Reference/fallback for unusual non-scalar surfaces; the engine's fields are scalar.
fn deposit_fields_separate(water: &mut Surface, height: &mut Surface, stroke: &Stroke, m: &MixerSettings, selection: Option<&Surface>) -> Rect {
    let damage = apply_dab_stroke(water, stroke, selection, false, 0, |work, fp| {
        let r = fp.rect.intersect(&work.rect);
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let k = fp.at(x, y);
                if let Some(v) = work.px_mut(x, y).first_mut() {
                    *v = (*v + k * m.wet.clamp(0.0, 1.0) * (1.0 - *v)).clamp(0.0, 1.0);
                }
            }
        }
    });
    apply_dab_stroke(height, stroke, selection, false, 0, |work, fp| {
        let r = fp.rect.intersect(&work.rect);
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let k = fp.at(x, y);
                if let Some(v) = work.px_mut(x, y).first_mut() {
                    *v = (*v + k * m.thickness.clamp(0.0, 1.0) * 0.1).min(4.0);
                }
            }
        }
    });
    damage
}

/// Stable four-neighbour diffusion plus evaporation. Work is capped by the caller's ROI.
pub fn step(pigment: &mut Surface, water: &mut Surface, height: &mut Surface, area: Rect, seconds: f32, m: &MixerSettings, selection: Option<&Surface>) {
    if area.is_empty() {
        return;
    }
    let before = pigment.clone();
    let space = pigment_space(m);
    let old_water = water.clone();
    let old_height = height.clone();
    let dt = seconds.clamp(0.0, 0.25);
    // Batch tile reads and writes one scanline at a time, retaining a frozen neighbor field.
    // Memory is bounded independently of canvas height; exceptionally wide rows use chunks.
    let fmt = pigment.format();
    for y in area.y0..area.y1 {
        let mut left = area.x0;
        while left < area.x1 {
            let right = left.saturating_add(8192).min(area.x1);
            let row = Rect::new(left, y, right, y.saturating_add(1));
            let halo = row.inflate(1);
            let width = halo.width() as usize;
            let mut rgba = vec![[0.; 4]; width * 3];
            before.read_rgba_into(halo, &mut rgba);
            let waters = old_water.read_region(halo);
            let heights = old_height.read_region(row);
            let masks = selection.map(|s| s.read_region(row));
            let mut output = Vec::with_capacity(row.width() as usize * fmt.channels());
            let mut new_water = Vec::with_capacity(row.width() as usize);
            for i in 0..row.width() as usize {
                let center = width + i + 1;
                let mask = masks.as_ref().and_then(|v| v.get(i)).copied().unwrap_or(1.);
                let moisture = waters.get(center).copied().unwrap_or(0.).clamp(0., 1.);
                let mut spread = moisture;
                let mut color = rgba.get(center).copied().unwrap_or([0.; 4]);
                if mask > 0. {
                    for index in [center + 1, center - 1, center + width, center - width] {
                        let neighbor = rgba.get(index).copied().unwrap_or([0.; 4]);
                        let wet = waters.get(index).copied().unwrap_or(0.);
                        let rate = m.diffusion.clamp(0., 1.) * dt * mask;
                        spread += (wet - moisture) * rate;
                        if neighbor[3] > 0. {
                            color = mix_in_space(color, neighbor, moisture.min(wet) * rate, true, &space);
                        }
                    }
                }
                let mut native = [0.; 5];
                let channels = from_rgba_into(&fmt, color, &mut native);
                output.extend_from_slice(&native[..channels]);
                let volume = heights.get(i).copied().unwrap_or(0.).max(0.);
                new_water.push(if mask > 0. { spread.clamp(0., 1.) * (-seconds.max(0.) * m.drying.clamp(0., 1.) / (1. + volume)).exp() } else { moisture });
            }
            pigment.write_region(row, &output);
            water.write_region(row, &new_water);
            left = right;
        }
    }
}

/// Height-derived lighting is applied only to the display surface; pigment remains unshaded.
pub fn render(pigment: &Surface, height: &Surface, target: &mut Surface, area: Rect) {
    render_masked(pigment, height, target, area, None, false);
}
pub fn render_masked(pigment: &Surface, height: &Surface, target: &mut Surface, area: Rect, selection: Option<&Surface>, lock_transparency: bool) {
    let fmt = target.format();
    for y in area.y0..area.y1 {
        let mut left = area.x0;
        while left < area.x1 {
            let right = left.saturating_add(8192).min(area.x1);
            let row = Rect::new(left, y, right, y.saturating_add(1));
            let halo = row.inflate(1);
            let width = halo.width() as usize;
            let heights = height.read_region(halo);
            let mut colors = vec![[0.; 4]; row.width() as usize];
            pigment.read_rgba_into(row, &mut colors);
            let masks = selection.map(|s| s.read_region(row));
            let mut native = target.read_region(row);
            for (i, px) in native.chunks_mut(fmt.channels()).enumerate() {
                let mask = masks.as_ref().and_then(|v| v.get(i)).copied().unwrap_or(1.).clamp(0., 1.);
                if mask <= 0. {
                    continue;
                }
                let center = width + i + 1;
                let h = |index| heights.get(index).copied().unwrap_or(0.);
                let gx = h(center + 1) - h(center - 1);
                let gy = h(center + width) - h(center - width);
                let shade = 1. + ((1. + (gx + gy) * 0.35).clamp(0.6, 1.3) - 1.) * mask;
                let mut rgba = colors.get(i).copied().unwrap_or([0.; 4]);
                for c in rgba.iter_mut().take(3) {
                    *c *= shade;
                }
                if lock_transparency {
                    rgba[3] = to_rgba(&fmt, px)[3];
                }
                from_rgba_into(&fmt, rgba, px);
            }
            target.write_region(row, &native);
            left = right;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BrushSettings, StrokePoint};
    use photocraft_color::{ColorMode, PixelFormat, SampleType};

    #[test]
    fn wet_optical_transfer_conserves_soft_contact_dose_across_stamp_counts() {
        for flow in [0.3, 1.] {
            for coverage in [0.05, 0.2, 0.47, 0.8, 1.] {
                let once = Deposition::new(flow, 4.).at(coverage);
                for count in [10, 100] {
                    let step = Deposition::new(flow, 4. / count as f32).at(coverage);
                    let repeated = 1. - (1. - step).powi(count);
                    assert!((once - repeated).abs() < 0.001, "flow={flow} contact={coverage}: {once} / {repeated}");
                }
                assert_eq!(Deposition::new(flow, 0.).at(coverage), 0.);
                assert!((Deposition::new(flow, -1.).at(coverage) - flow * coverage).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn initial_low_pressure_pickup_has_no_interior_holes() {
        let mut target = Surface::new(PixelFormat::RGBA32F);
        target.fill_rect(Rect::new(0, 0, 100, 100), &[0.2, 0.4, 0.8, 1.]);
        let stroke = Stroke { brush: BrushSettings { size: 80., pressure_size: true, ..Default::default() }, points: vec![StrokePoint::new(50., 50., 0.3)] };
        let mut state = MixerState::default();
        apply_spatial_mixer(&mut target, None, &stroke, &MixerSettings { wet: 1., mix: 1., ..Default::default() }, &mut state, None, false);
        let side = pickup_side(80.);
        for y in side / 4..side * 3 / 4 {
            for x in side / 4..side * 3 / 4 {
                let color = state.nib[y * side + x];
                assert!(color[3] > 0.999, "empty carried-color cell at {x},{y}: {color:?}");
                assert!((color[2] - 0.8).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn pickup_seed_does_not_load_color_under_uncontacted_tip_holes() {
        let mut surface = Surface::new(PixelFormat::RGBA32F);
        surface.fill_rect(Rect::new(0, 0, 40, 100), &[1., 0., 0., 1.]);
        surface.fill_rect(Rect::new(40, 0, 100, 100), &[0., 0., 1., 1.]);
        let stroke = Stroke {
            brush: BrushSettings {
                size: 40.,
                pressure_size: false,
                pressure_opacity: false,
                tip: crate::TipShape::Sampled(crate::GrayTile::from_fn(16, 16, |x, _| if x < 8 { 0. } else { 1. })),
                ..Default::default()
            },
            points: vec![StrokePoint::new(50., 50., 1.)],
        };
        let dabs = crate::dabs(&stroke);
        let context = crate::BrushContext::new(&stroke.brush);
        let fp = context.footprint(&dabs[0], 0);
        let source = crate::retouch::Region::read(&surface, fp.rect);
        let mut nib = vec![[0.; 4]; 64 * 64];
        seed_pickup(&mut nib, 64, &fp, &source, &surface.format());
        assert_eq!(nib[32 * 64 + 12], [0.; 4]);
        assert!(nib[32 * 64 + 52][2] > 0.999);
        assert!(nib.iter().filter(|c| c[3] > 0.).all(|c| c[0] < 1e-6), "uncontacted red paint entered the reservoir");
    }

    #[test]
    fn dry_soft_contact_accumulates_more_at_dense_spacing_like_ps() {
        let render = |spacing| {
            let mut target = Surface::new(PixelFormat::RGBA32F);
            let brush = BrushSettings {
                size: 40.,
                spacing,
                tip: crate::TipShape::Sampled(crate::GrayTile::from_fn(16, 16, |_, _| 0.2)),
                pressure_size: false,
                pressure_opacity: false,
                ..Default::default()
            };
            let stroke = Stroke { brush, points: vec![StrokePoint::new(40., 40., 1.), StrokePoint::new(260., 40., 1.)] };
            let mut state = MixerState::default();
            state.load([1., 0., 0., 1.]);
            apply_spatial_mixer(&mut target, None, &stroke, &MixerSettings { wet: 0., flow: 1., load: 1., ..Default::default() }, &mut state, None, false);
            target.rgba(140, 40)[3]
        };
        let (dense, sparse) = (render(0.01), render(0.25));
        assert!(dense > 0.999, "dense dry contact must build up: {dense}");
        assert!(sparse > 0.5 && sparse < 0.7, "four sparse .2 contacts: {sparse}");
        assert!(dense - sparse > 0.3, "the measured density effect disappeared: {dense}/{sparse}");
    }

    #[test]
    fn loaded_map_resize_interpolates_alpha_without_hidden_color() {
        let map = [[1., 0., 0., 1.], [0., 1., 0., 0.], [1., 0., 0., 1.], [0., 1., 0., 0.]];
        let px = map_sample(&map, 4, 1, 1);
        assert!((px[3] - 0.75).abs() < 1e-6);
        assert_eq!(px[..3], [1., 0., 0.]);
        assert_eq!(map_sample(&map, 2, 0, 0), map[0]);
    }

    #[test]
    fn wet_zero_mix_exchanges_color_with_equal_alpha_canvases() {
        let mut brush = BrushSettings { size: 32., angle: 70., roundness: 0.6, pressure_size: true, spacing: 0.07, ..Default::default() };
        brush.texture.enabled = true;
        brush.texture.each_tip = true;
        brush.texture.depth = 0.3;
        let stroke = Stroke { brush, points: vec![StrokePoint::new(25., 25., 0.3), StrokePoint::new(60., 40., 1.)] };
        let mut initial = Surface::new(PixelFormat::RGBA32F);
        initial.fill_rect(Rect::new(0, 0, 80, 80), &[0.1, 0.2, 0.3, 0.7]);
        let mut a = initial.clone();
        let mut b = initial;
        b.fill_rect(Rect::new(0, 0, 80, 80), &[0.7, 0.2, 0.1, 0.7]);
        let mut state = MixerState {
            amount: 1.,
            canvas_loaded: true,
            loaded_nib: (0..64 * 64).map(|i| if i % 64 < 32 { [1., 0., 0., 0.5] } else { [0., 0., 1., 1.] }).collect(),
            ..Default::default()
        };
        let mut reference = state.clone();
        // The dry density rule is now intentionally different. Preserve the wet
        // exchange guard using equal-alpha canvases with different colours.
        let m = MixerSettings { wet: 1., mix: 0., flow: 0.6, ..Default::default() };
        apply_spatial_mixer(&mut a, None, &stroke, &m, &mut state, None, false);
        apply_spatial_mixer(&mut b, None, &stroke, &m, &mut reference, None, false);
        let mut color_difference = 0.;
        for y in 0..80 {
            for x in 0..80 {
                let (a, b) = (a.rgba(x, y), b.rgba(x, y));
                assert!((a[3] - b[3]).abs() < 1e-6);
                // Remove the known backdrop contribution. Otherwise two
                // different untouched canvases would trivially pass this test.
                let k = (a[3] - 0.7) / 0.3;
                if k > 0.0001 {
                    let paint_a = (a[0] * a[3] - 0.1 * 0.7 * (1. - k)) / k;
                    let paint_b = (b[0] * b[3] - 0.7 * 0.7 * (1. - k)) / k;
                    color_difference += (paint_a - paint_b).abs();
                }
            }
        }
        assert!(color_difference > 1.);
    }

    #[test]
    fn dry_density_accumulates_paint_but_load_drain_remains_distance_based() {
        let render = |spacing| {
            let mut s = Surface::new(PixelFormat::RGBA32F);
            s.fill_rect(Rect::new(0, 0, 300, 80), &[0., 0., 1., 1.]);
            let stroke = Stroke {
                brush: BrushSettings { size: 40., spacing, hardness: 1., pressure_size: false, pressure_opacity: false, ..Default::default() },
                points: vec![StrokePoint::new(40., 40., 1.), StrokePoint::new(260., 40., 1.)],
            };
            let mut state = MixerState::default();
            state.load([1., 0., 0., 1.]);
            apply_spatial_mixer(&mut s, None, &stroke, &MixerSettings { wet: 0., flow: 0.03, load: 0.8, ..Default::default() }, &mut state, None, false);
            (s.rgba(140, 40), state.amount)
        };
        let (dense, a) = render(0.01);
        let (wide, b) = render(0.25);
        assert!(dense[0] > 0.7, "dense dry flow must accumulate: {dense:?}");
        assert!(wide[0] < 0.2 && dense[0] - wide[0] > 0.5, "density gain lost: {dense:?} {wide:?}");
        assert!((a - b).abs() < 0.005, "{a} {b}");
    }

    #[test]
    fn dry_mixer_preserves_alpha_flow_selection_and_transparency_lock() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let fmt = PixelFormat::new(ColorMode::Rgb, depth, true);
            let mut target = Surface::new(fmt);
            let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
            selection.fill_rect(Rect::new(0, 0, 40, 40), &[0.5]);
            let stroke = Stroke {
                brush: BrushSettings { size: 20., pressure_size: false, flow: 0.5, ..Default::default() },
                points: vec![StrokePoint::new(20., 20., 1.)],
            };
            let m = MixerSettings { wet: 0., mix: 1., flow: 0.5, ..Default::default() };
            let mut state = MixerState::default();
            state.load([1., 0., 0., 0.5]);
            apply_spatial_mixer(&mut target, None, &stroke, &m, &mut state, Some(&selection), false);
            assert!((target.rgba(20, 20)[3] - 0.125).abs() < 0.005);
            assert!(state.nib.is_empty());
            let alpha = target.rgba(20, 20)[3];
            apply_spatial_mixer(&mut target, None, &stroke, &m, &mut state, None, true);
            assert_eq!(target.rgba(20, 20)[3], alpha);
        }
    }

    #[test]
    fn canvas_only_mix_has_no_loaded_color_even_at_low_wetness() {
        for sample_type in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut target = Surface::new(PixelFormat::new(ColorMode::Rgb, sample_type, true));
            target.fill_rect(Rect::new(0, 0, 80, 40), &[0., 0., 1., 1.]);
            let stroke = Stroke {
                brush: BrushSettings { size: 20., pressure_size: false, ..Default::default() },
                points: vec![StrokePoint::new(20., 20., 1.), StrokePoint::new(60., 20., 1.)],
            };
            let mut state = MixerState::default();
            state.load([1., 0., 0., 1.]);
            let m = MixerSettings { wet: 0.3, mix: 1., flow: 1., ..Default::default() };
            apply_spatial_mixer(&mut target, None, &stroke, &m, &mut state, None, false);
            let c = target.rgba(40, 20);
            assert!(c[0] < 0.001 && c[2] > 0.999, "{c:?}");
        }
    }

    #[test]
    fn loaded_alpha_and_toolbar_flow_are_applied_once() {
        let mut target = Surface::new(PixelFormat::RGBA32F);
        let stroke = Stroke {
            brush: BrushSettings { size: 20., flow: 0.5, pressure_size: false, pressure_opacity: false, hardness: 1., ..Default::default() },
            points: vec![StrokePoint::new(20., 20., 1.)],
        };
        let mut state = MixerState::default();
        state.load([1., 0., 0., 0.5]);
        apply_spatial_mixer(&mut target, None, &stroke, &MixerSettings { wet: 0., mix: 0., flow: 0.5, ..Default::default() }, &mut state, None, false);
        let c = target.rgba(20, 20);
        assert!((c[3] - 0.25).abs() < 0.001 && c[0] > 0.999, "{c:?}");
    }

    #[test]
    fn rotated_pickup_ignores_rgb_hidden_under_transparent_pixels() {
        for sample_type in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut target = Surface::new(PixelFormat::new(ColorMode::Rgb, sample_type, true));
            for x in 0..100 {
                let c = if x % 2 == 0 { [1., 0., 0., 0.5] } else { [0., 1., 0., 0.] };
                target.fill_rect(Rect::new(x, 0, x + 1, 100), &c);
            }
            let stroke = Stroke {
                brush: BrushSettings { size: 80., angle: std::f32::consts::FRAC_PI_4, pressure_size: false, ..Default::default() },
                points: vec![StrokePoint::new(50., 50., 1.)],
            };
            let mut state = MixerState::default();
            apply_spatial_mixer(&mut target, None, &stroke, &MixerSettings { wet: 1., mix: 1., ..Default::default() }, &mut state, None, false);
            let colors: Vec<_> = state.nib.iter().filter(|c| c[3] > 0.01).collect();
            assert!(!colors.is_empty());
            assert!(colors.iter().all(|c| c[0] > 0.999 && c[1] < 0.001));
        }
    }

    #[test]
    fn adaptive_pickup_keeps_two_pixel_stripes_in_a_large_tip() {
        let mut target = Surface::new(PixelFormat::RGBA32F);
        for y in 0..256 {
            let c = if y % 4 < 2 { [1., 0., 0., 1.] } else { [0., 0., 1., 1.] };
            target.fill_rect(Rect::new(0, y, 260, y + 1), &c);
        }
        let stroke = Stroke {
            brush: BrushSettings { size: 256., pressure_size: false, pressure_opacity: false, hardness: 1., ..Default::default() },
            points: vec![StrokePoint::new(128., 128., 1.)],
        };
        let mut state = MixerState::default();
        apply_spatial_mixer(&mut target, None, &stroke, &MixerSettings { wet: 1., mix: 1., ..Default::default() }, &mut state, None, false);
        assert_eq!(state.nib.len(), 256 * 256);
        for y in [120, 122] {
            let i = y * 256 + 128;
            let c = state.nib[i];
            assert!((c[0] - c[2]).abs() > 0.99, "{c:?}");
        }
        assert_eq!(pickup_side(5000.), 512);
        assert_eq!(pickup_side(f32::NAN), 64);
    }

    #[test]
    fn fused_fields_match_separate_passes_with_texture_dual_opacity_selection_and_depth() {
        use crate::{GrayTile, Pattern};
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let fmt = PixelFormat::new(ColorMode::Grayscale, depth, false);
            let mut water = Surface::new(fmt);
            let mut height = Surface::new(fmt);
            water.fill_rect(Rect::new(-10, -10, 100, 80), &[0.2]);
            height.fill_rect(Rect::new(-10, -10, 100, 80), &[0.3]);
            let mut oldw = water.clone();
            let mut oldh = height.clone();
            let mut b = BrushSettings { size: 24., opacity: 0.6, spacing: 0.07, pressure_size: false, ..Default::default() };
            b.texture.enabled = true;
            b.texture.pattern = Pattern::Tile(GrayTile::from_fn(7, 5, |x, y| if (x + y) % 2 == 0 { 0.2 } else { 0.8 }));
            b.dual_brush.enabled = true;
            b.dual_brush.size = 16.;
            b.dual_brush.spacing = 0.1;
            let stroke = Stroke { brush: b, points: vec![StrokePoint::new(12., 20., 1.), StrokePoint::new(68., 40., 0.7)] };
            let m = MixerSettings::default();
            let mut sel = Surface::new(fmt);
            sel.fill_rect(Rect::new(30, 0, 80, 70), &[0.4]);
            let a = deposit_fields(&mut water, &mut height, &stroke, &m, Some(&sel));
            let old = deposit_fields_separate(&mut oldw, &mut oldh, &stroke, &m, Some(&sel));
            assert_eq!(a, old);
            assert_eq!(water.read_region(a), oldw.read_region(a));
            assert_eq!(height.read_region(a), oldh.read_region(a));
        }
    }
    #[test]
    fn height_render_respects_selection_and_locked_alpha() {
        let area = Rect::new(0, 0, 8, 8);
        let mut pigment = Surface::new(PixelFormat::RGBA32F);
        pigment.fill_rect(area, &[0.8, 0.3, 0.1, 1.]);
        let mut height = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        height.fill_rect(Rect::new(0, 0, 4, 8), &[1.]);
        let mut selection = Surface::new(height.format());
        selection.fill_rect(Rect::new(0, 0, 4, 8), &[1.]);
        let mut target = Surface::new(PixelFormat::RGBA32F);
        target.fill_rect(area, &[0.1, 0.2, 0.3, 0.25]);
        render_masked(&pigment, &height, &mut target, area, Some(&selection), true);
        assert_eq!(target.rgba(5, 3), [0.1, 0.2, 0.3, 0.25]);
        assert_eq!(target.rgba(2, 3)[3], 0.25);
        assert!(target.rgba(2, 3)[0] > 0.7);
    }
    #[test]
    fn pigment_mixing_uses_working_profile_and_retains_hdr_endpoints() {
        let m = MixerSettings { pigment_profile: Some(photocraft_cms::Builtin::DisplayP3.profile().to_bytes()), ..Default::default() };
        let space = pigment_space(&m);
        assert!(space.is_some());
        let a = [0.1, 0.25, 0.8, 1.];
        let b = [0.9, 0.75, 0.1, 0.5];
        let managed = mix_in_space(a, b, 0.5, true, &space);
        let untagged = pigment_mix(a, b, 0.5, true);
        assert!(managed.iter().zip(untagged).take(3).any(|(a, b)| (a - b).abs() > 0.005));
        assert!((managed[3] - 0.75).abs() < 1e-6);
        let hdr = [2., -0.1, 0.5, 1.];
        assert_eq!(mix_in_space(hdr, b, 0., true, &space), hdr);
        assert_eq!(mix_in_space(a, hdr, 1., true, &space), hdr);
    }

    #[test]
    fn kubelka_munk_retains_endpoints_alpha_and_hdr_excursions() {
        let a = [-0.4, 0.2, 2.5, 0.25];
        let b = [-0.2, 0.8, 1.8, 0.75];
        assert_eq!(pigment_mix(a, b, 0.0, true), a);
        assert_eq!(pigment_mix(a, b, 1.0, true), b);
        assert_eq!(pigment_mix(a, a, 0.5, true), a);
        assert_eq!(pigment_mix(a, b, f32::NAN, true), a);
        let midpoint = pigment_mix(a, b, 0.5, true);
        assert!(midpoint.iter().all(|v| v.is_finite()), "{midpoint:?}");
        assert!(midpoint[0] < 0. && midpoint[2] > 1., "{midpoint:?}");
        assert_eq!(midpoint[3], 0.5);
    }

    #[test]
    fn kubelka_munk_mixes_blue_and_yellow_with_a_green_midpoint() {
        let blue = [0., 33. / 255., 133. / 255., 1.];
        let yellow = [252. / 255., 211. / 255., 0., 1.];
        let midpoint = pigment_mix(blue, yellow, 0.5, true);
        assert!(midpoint[1] > midpoint[0] && midpoint[1] > midpoint[2], "{midpoint:?}");
        for t in [0.01, 0.15, 0.35, 0.5, 0.8, 0.99] {
            let forward = pigment_mix(blue, yellow, t, true);
            let reverse = pigment_mix(yellow, blue, 1. - t, true);
            assert!(forward.into_iter().zip(reverse).all(|(a, b)| (a - b).abs() < 1e-5), "{t}: {forward:?} vs {reverse:?}");
        }
    }

    #[test]
    fn spatial_nib_retains_two_stripes_at_multiple_depths() {
        for sample_type in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, sample_type, true));
            s.fill_rect(Rect::new(0, 0, 35, 10), &[1.0, 0.0, 0.0, 1.0]);
            s.fill_rect(Rect::new(0, 10, 35, 20), &[0.0, 0.0, 1.0, 1.0]);
            s.fill_rect(Rect::new(35, 0, 100, 20), &[1.0, 1.0, 1.0, 1.0]);
            let stroke = Stroke {
                brush: BrushSettings { size: 18.0, hardness: 1.0, spacing: 0.08, pressure_size: false, pressure_opacity: false, ..Default::default() },
                points: vec![StrokePoint::new(15.0, 10.0, 1.0), StrokePoint::new(70.0, 10.0, 1.0)],
            };
            let m = MixerSettings { wet: 1.0, mix: 1.0, clean_after_stroke: false, load_after_stroke: false, ..Default::default() };
            let mut state = MixerState::default();
            apply_spatial_mixer(&mut s, None, &stroke, &m, &mut state, None, false);
            let top = s.rgba(40, 6);
            let bottom = s.rgba(40, 14);
            assert!(top[0] > top[2] + 0.1, "top {top:?}");
            assert!(bottom[2] > bottom[0] + 0.1, "bottom {bottom:?}");
            assert_eq!(state.nib.len(), pickup_side(stroke.brush.size).pow(2));
        }
    }
    #[test]
    fn pigment_mixing_and_drying_preserve_fields_and_selection() {
        let blue = [0.0, 33.0 / 255.0, 133.0 / 255.0, 1.0];
        let yellow = [252.0 / 255.0, 211.0 / 255.0, 0.0, 1.0];
        let green = pigment_mix(blue, yellow, 0.5, true);
        assert!(green[1] > green[0] && green[1] > green[2], "{green:?}");
        let scalar = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
        let mut water = Surface::new(scalar);
        let mut height = Surface::new(scalar);
        let mut pigment = Surface::new(PixelFormat::RGBA32F);
        let area = Rect::new(0, 0, 8, 8);
        water.fill_rect(area, &[1.0]);
        height.fill_rect(area, &[0.5]);
        pigment.fill_rect(area, &[1.0, 0.0, 0.0, 1.0]);
        let mut selection = Surface::new(scalar);
        selection.fill_rect(Rect::new(0, 0, 4, 8), &[1.0]);
        let m = MixerSettings { drying: 1.0, ..Default::default() };
        step(&mut pigment, &mut water, &mut height, area, 60.0, &m, Some(&selection));
        assert!(water.pixel(1, 1)[0] < 0.01);
        assert_eq!(water.pixel(6, 1)[0], 1.0);
        assert_eq!(height.pixel(1, 1)[0], 0.5);
    }
}
