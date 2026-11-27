//! Stateful mixer rendering. Input partitioning never resets spacing, dynamics or pickup.
//! Unselected paint stays in lazy native-channel float tiles, avoiding per-batch U8/U16
//! quantisation and repeated selection application. Only newly touched rectangles publish.
use crate::{
    BrushContext, BrushSettings, Dab, StrokePoint,
    dynamics::DabGenerator,
    mixer::{MixerSettings, MixerState},
    retouch::{Footprint, Region, mix_premultiplied, over_native},
    wet_paint::{Deposition, PigmentSpace, dry_deposition, map_sample, mix_in_space, nib_mapping, pickup_side, pigment_space, seed_pickup, transport},
};
use photocraft_color::{PixelFormat, SampleType};
use photocraft_geom::{Point, Rect};
use photocraft_raster::{Surface, from_rgba_into, to_rgba};
use std::collections::HashSet;

#[derive(Clone)]
pub struct MixerStroke {
    generator: DabGenerator,
    context: BrushContext,
    original: Surface,
    source: Surface,
    // Keep a pre-deposition contact for physical bristles and empty-substrate pickup.
    // An external all-layer composite remains owned by the engine.
    local_pickup: bool,
    pickup_snapshot: Option<Region>,
    pickup_seeded: bool,
    work: Surface,
    tiles: HashSet<(i32, i32)>,
    settings: MixerSettings,
    pub state: MixerState,
    loaded: Vec<[f32; 4]>,
    side: usize,
    bins: Vec<[f32; 4]>,
    weights: Vec<f32>,
    paints: Vec<f32>,
    deposits: Vec<f32>,
    previous: Option<Point>,
    index: usize,
    space: PigmentSpace,
    finished: bool,
    pub damage: Rect,
}

fn parallel_spectral_dab(area: Rect) -> bool {
    if u64::from(area.width()) * u64::from(area.height()) < 8192 {
        return false;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        rayon::current_num_threads() > 1
    }
    #[cfg(target_arch = "wasm32")]
    {
        false
    }
}

/// A dab freezes the sampled footprint before any bristle updates its reservoir.
/// Each cell then writes a disjoint nib, paint and opacity. Large spectral dabs
/// can share the CPU while pickup is still summarized in the same cell order.
fn mix_nib_cells(
    nib: &mut [[f32; 4]],
    paints: &mut [f32],
    deposits: &mut [f32],
    channels: usize,
    parallel: bool,
    mix: impl Fn(usize, &mut [f32; 4], &mut [f32], &mut f32) + Sync,
) {
    #[cfg(not(target_arch = "wasm32"))]
    if parallel {
        use rayon::prelude::*;
        nib.par_iter_mut()
            .zip(paints.par_chunks_exact_mut(channels))
            .zip(deposits.par_iter_mut())
            .enumerate()
            .with_min_len(1536)
            .for_each(|(i, ((nib, paint), deposit))| mix(i, nib, paint, deposit));
        return;
    }
    #[cfg(target_arch = "wasm32")]
    let _ = parallel;
    for (i, ((nib, paint), deposit)) in nib.iter_mut().zip(paints.chunks_exact_mut(channels)).zip(deposits).enumerate() {
        mix(i, nib, paint, deposit);
    }
}

/// The original region is shared across all dabs in this input batch. Empty
/// substrate has no pigment to contribute, including its hidden RGB.
fn retain_substrate(sample: &mut Region, original: &Region, alpha: Option<usize>, amount: f32) {
    let channels = sample.ch;
    let row = sample.width() * channels;
    if row == 0 {
        return;
    }
    for (i, wet_row) in sample.data.chunks_exact_mut(row).enumerate() {
        let start = original.index(sample.rect.x0, sample.rect.y0 + i as i32) * channels;
        for (wet, base) in wet_row.chunks_exact_mut(channels).zip(original.data[start..start + row].chunks_exact(channels)) {
            if let Some(a) = alpha {
                if base[a] <= 0. || wet[a] <= 0. {
                    continue;
                }
                if base[a] != wet[a] {
                    mix_premultiplied(wet, base, a, amount);
                    continue;
                }
            }
            for (v, b) in wet.iter_mut().zip(base) {
                *v += (*b - *v) * amount;
            }
        }
    }
}

impl MixerStroke {
    pub fn new(original: &Surface, sample: Option<&Surface>, brush: &BrushSettings, settings: &MixerSettings, mut state: MixerState) -> Self {
        let mut brush = brush.clone();
        brush.opacity = 1.;
        brush.flow = 1.;
        let side = pickup_side(brush.size);
        let cells = side * side;
        let loaded: Vec<[f32; 4]> = if state.loaded_nib.is_empty() {
            vec![state.reservoir.unwrap_or([0.; 4]); cells]
        } else {
            (0..cells).map(|i| map_sample(&state.loaded_nib, side, i % side, i / side)).collect()
        };
        if settings.wet > 0. && state.nib.len() != cells {
            let old = std::mem::take(&mut state.nib);
            state.nib = (0..cells).map(|i| map_sample(&old, side, i % side, i / side)).collect();
        }
        state.amount = if state.reservoir.is_some() || !state.loaded_nib.is_empty() { state.amount.clamp(0., 1.) } else { 0. };
        let fmt = original.format();
        let mut paints = vec![0.; cells * fmt.channels()];
        if settings.wet <= 0. {
            for (color, native) in loaded.iter().zip(paints.chunks_mut(fmt.channels())) {
                from_rgba_into(&fmt, [color[0], color[1], color[2], 1.], native);
            }
        }
        Self {
            generator: DabGenerator::new(&brush, 1.),
            context: BrushContext::new(&brush),
            original: original.clone(),
            source: sample.unwrap_or(original).clone(),
            local_pickup: sample.is_none(),
            pickup_snapshot: None,
            pickup_seeded: false,
            work: Surface::new(PixelFormat::new(fmt.mode, SampleType::F32, fmt.alpha)),
            tiles: HashSet::new(),
            settings: settings.clone(),
            state,
            loaded,
            side,
            bins: vec![[0.; 4]; cells],
            weights: vec![0.; cells],
            paints,
            deposits: vec![0.; cells],
            previous: None,
            index: 0,
            space: if settings.simulate_paint { pigment_space(settings) } else { None },
            finished: false,
            damage: Rect::EMPTY,
        }
    }

    /// Bounds needed from a pre-stroke composite; dab generation is cheap and deterministic.
    pub fn sample_rects(&self, points: &[StrokePoint], finish: bool) -> Vec<Rect> {
        let mut generator = self.generator.clone();
        let mut dabs = Vec::new();
        generator.push(points, &mut dabs, &mut Vec::new());
        if finish {
            generator.finish(&mut dabs, &mut Vec::new());
        }
        dabs.iter().map(|d| self.context.dab_rect(d, false)).collect()
    }
    pub fn set_sample_region(&mut self, bounds: Rect, pixels: &[f32]) {
        self.source.write_region(bounds, pixels);
    }
    /// Whether release would draw anything. Probing the small generator avoids
    /// cloning the entire paint reservoir merely to discover an empty tail.
    pub fn has_pending_tail(&self) -> bool {
        if self.finished {
            return false;
        }
        let mut generator = self.generator.clone();
        let mut dabs = Vec::new();
        generator.finish(&mut dabs, &mut Vec::new());
        !dabs.is_empty()
    }
    pub fn push(&mut self, points: &[StrokePoint], target: &mut Surface, selection: Option<&Surface>, lock: bool) -> Rect {
        if self.finished {
            return Rect::EMPTY;
        }
        let mut dabs = Vec::new();
        self.generator.push(points, &mut dabs, &mut Vec::new());
        self.render(&dabs, target, selection, lock)
    }

    pub fn finish(&mut self, target: &mut Surface, selection: Option<&Surface>, lock: bool) -> Rect {
        if self.finished {
            return Rect::EMPTY;
        }
        self.finished = true;
        let mut dabs = Vec::new();
        self.generator.finish(&mut dabs, &mut Vec::new());
        self.render(&dabs, target, selection, lock)
    }

    fn render(&mut self, dabs: &[Dab], target: &mut Surface, selection: Option<&Surface>, lock: bool) -> Rect {
        let bounds = dabs.iter().fold(Rect::EMPTY, |r, d| r.union(&self.context.dab_rect(d, false)));
        if bounds.is_empty() {
            return bounds;
        }
        // Widely separated points must not allocate their enclosing empty rectangle.
        if dabs.len() > 1 && u64::from(bounds.width()) * u64::from(bounds.height()) > 1_048_576 {
            let (left, right) = dabs.split_at(dabs.len() / 2);
            let a = self.render(left, target, selection, lock);
            return a.union(&self.render(right, target, selection, lock));
        }
        // Initialise only new tiles from the original, and retain full float precision thereafter.
        const TILE: i32 = 256;
        for ty in bounds.y0.div_euclid(TILE)..=(bounds.y1 - 1).div_euclid(TILE) {
            for tx in bounds.x0.div_euclid(TILE)..=(bounds.x1 - 1).div_euclid(TILE) {
                if self.tiles.insert((tx, ty)) {
                    let r = Rect::new(tx * TILE, ty * TILE, (tx + 1) * TILE, (ty + 1) * TILE);
                    self.work.write_region(r, &self.original.read_region(r));
                }
            }
        }
        let mut work = Region::read(&self.work, bounds);
        let mut output = Region::read(&self.original, bounds);
        let fmt = self.original.format();
        let alpha = fmt.alpha.then(|| fmt.mode.color_channels());
        let source = (self.settings.wet > 0. && !self.local_pickup).then(|| Region::read(&self.source, bounds));
        for dab in dabs {
            let footprint = self.context.footprint(dab, self.index);
            self.index += 1;
            let snapshot = (self.settings.wet > 0. && self.local_pickup).then(|| {
                let mut sample = self.snapshot_at_contact(&work, footprint.rect, selection, lock);
                let wet = self.settings.wet.clamp(0., 1.) * dab.wet.clamp(0., 1.);
                if !self.settings.simulate_paint && wet > 0. && wet < 1. {
                    // Thin wet films pick up both the fresh visible paint and some of
                    // the original substrate. This limits drift toward pure loaded paint
                    // on long strokes; at full wetness only the mobile film is sampled.
                    let dry = 1. - wet;
                    let mix = self.settings.mix.clamp(0., 1.) * dab.mix.clamp(0., 1.);
                    let substrate = 0.45 * dry * dry * dry * (1. + 0.25 * mix);
                    retain_substrate(&mut sample, &output, alpha, substrate);
                }
                sample
            });
            self.dab(&mut work, &footprint, snapshot.as_ref().or(source.as_ref()), lock);
            self.pickup_snapshot = snapshot;
        }
        work.write(&mut self.work);
        for y in bounds.y0..bounds.y1 {
            for x in bounds.x0..bounds.x1 {
                let k = selection.map_or(1., |s| s.sample_channel(x, y, 0));
                if k <= 0. {
                    continue;
                }
                let dst = output.px_mut(x, y);
                let src = work.px(x, y);
                if let Some(a) = alpha.filter(|_| !lock) {
                    mix_premultiplied(dst, src, a, k);
                } else {
                    for (i, (d, s)) in dst.iter_mut().zip(src).enumerate() {
                        if !lock || alpha != Some(i) {
                            *d += (*s - *d) * k;
                        }
                    }
                }
            }
        }
        output.write(target);
        self.damage = self.damage.union(&bounds);
        bounds
    }

    /// Ordinary bristles re-sample the visible wet film at each dab. Physical
    /// bristles retain first-contact colour until leaving the contact patch.
    /// A fully empty contact cannot supply pigment back to the loaded bristle.
    fn snapshot_at_contact(&self, work: &Region, bounds: Rect, selection: Option<&Surface>, lock: bool) -> Region {
        let mut snapshot = work.crop(bounds);
        let fmt = self.original.format();
        let alpha = fmt.alpha.then(|| fmt.mode.color_channels());
        let overlap = self.pickup_snapshot.as_ref().map_or(Rect::EMPTY, |old| old.rect.intersect(&snapshot.rect));
        if let Some(old) = &self.pickup_snapshot {
            let length = overlap.width() as usize * snapshot.ch;
            for y in overlap.y0..overlap.y1 {
                let src = old.index(overlap.x0, y) * old.ch;
                let dst = snapshot.index(overlap.x0, y) * snapshot.ch;
                if let (Some(source), Some(target)) = (old.data.get(src..src + length), snapshot.data.get_mut(dst..dst + length)) {
                    if self.settings.simulate_paint {
                        target.copy_from_slice(source);
                    } else if let Some(a) = alpha {
                        for (before, visible) in source.chunks_exact(old.ch).zip(target.chunks_exact_mut(snapshot.ch)) {
                            if before[a] <= 0. {
                                visible.copy_from_slice(before);
                            }
                        }
                    }
                }
            }
        }
        if let Some(selection) = selection {
            for y in snapshot.rect.y0..snapshot.rect.y1 {
                for x in snapshot.rect.x0..snapshot.rect.x1 {
                    if self.settings.simulate_paint && x >= overlap.x0 && x < overlap.x1 && y >= overlap.y0 && y < overlap.y1 {
                        continue;
                    }
                    // `work` is deliberately unmasked so feathering is applied
                    // once on publication. Pickup must see the published paint.
                    let k = selection.sample_channel(x, y, 0).clamp(0., 1.);
                    if k < 1. {
                        let mut visible = self.original.pixel(x, y);
                        let painted = snapshot.px(x, y);
                        if let Some(a) = alpha.filter(|_| !lock) {
                            mix_premultiplied(&mut visible, painted, a, k);
                        } else {
                            for (i, (d, s)) in visible.iter_mut().zip(painted).enumerate() {
                                if !lock || alpha != Some(i) {
                                    *d += (*s - *d) * k;
                                }
                            }
                        }
                        snapshot.px_mut(x, y).copy_from_slice(&visible);
                    }
                }
            }
        }
        snapshot
    }

    fn dab(&mut self, work: &mut Region, fp: &Footprint, source: Option<&Region>, lock: bool) {
        let fmt = self.original.format();
        let channels = fmt.channels();
        let m = &self.settings;
        let mapping = nib_mapping(fp, self.side);
        let area = fp.rect.intersect(&work.rect);
        let (distance, travel, drain) = transport(fp, &mut self.previous, m.load);
        if m.wet <= 0. {
            for y in area.y0..area.y1 {
                for x in area.x0..area.x1 {
                    let coverage = fp.at(x, y);
                    if coverage <= 0. {
                        continue;
                    }
                    let i = mapping(x, y);
                    if let (Some(color), Some(native)) = (self.loaded.get(i), self.paints.get(i * channels..(i + 1) * channels)) {
                        over_native(
                            &fmt,
                            work.px_mut(x, y),
                            native,
                            dry_deposition(m.flow, coverage * fp.dab.opacity.clamp(0., 1.) * color[3].clamp(0., 1.) * self.state.amount),
                            lock,
                        );
                    }
                }
            }
        } else if let Some(source) = source {
            let deposition = Deposition::new(m.flow, travel);
            self.bins.fill([0.; 4]);
            self.weights.fill(0.);
            let wet = m.wet.clamp(0., 1.) * fp.dab.wet.clamp(0., 1.);
            if !self.pickup_seeded && wet > 0. && fp.cov.iter().any(|coverage| *coverage > 0.) {
                seed_pickup(&mut self.state.nib, self.side, fp, source, &self.source.format());
                self.pickup_seeded = true;
            }
            let mix = if wet > 0. { m.mix.clamp(0., 1.) * fp.dab.mix.clamp(0., 1.) } else { 0. };
            let (film, carry_diameters) = if m.simulate_paint {
                (wet * (1. - mix) * 0.5, 0.25 + 4. * wet)
            } else {
                // The well blend already includes Mix. Wet controls the remaining
                // contact film; longer steps expose more fresh reservoir paint.
                let step_diameters = travel.max(0.) * 0.25;
                let saturated_wet = wet / (wet + 0.25 * (1. - wet));
                (saturated_wet / (1.2 + 0.85 * step_diameters), 0.60 - 0.15 * mix + 0.20 * (1. - wet))
            };
            let exchange = 1. - (-distance / (fp.dab.radius * 2. * carry_diameters).max(0.01)).exp();
            for y in area.y0..area.y1 {
                for x in area.x0..area.x1 {
                    let coverage = fp.at(x, y);
                    if coverage <= 0. {
                        continue;
                    }
                    let index = mapping(x, y);
                    if let (Some(bin), Some(weight)) = (self.bins.get_mut(index), self.weights.get_mut(index)) {
                        if wet > 0. {
                            let px = to_rgba(&self.source.format(), source.px(x, y));
                            let a = px[3].clamp(0., 1.) * coverage;
                            for (v, c) in bin.iter_mut().take(3).zip(px) {
                                *v += c * a;
                            }
                            bin[3] += a;
                        }
                        *weight += coverage;
                    }
                }
            }
            let pigment_mixing = m.simulate_paint;
            // Away from a pigmented substrate, loaded paint gradually replaces the
            // colour carried by the mobile bristles. An unloaded well cannot refill it.
            let local_wet_film = !pigment_mixing && self.local_pickup;
            let refill = if local_wet_film && self.state.amount > 0. && mix < 1. {
                (1. - (-distance / (fp.dab.radius * 2. * 2.8).max(0.01)).exp()) * (1. - mix) * self.state.amount
            } else {
                0.
            };
            let parallel = pigment_mixing && parallel_spectral_dab(area);
            mix_nib_cells(&mut self.state.nib, &mut self.paints, &mut self.deposits, channels, parallel, |i, nib, native, deposit| {
                let weight = self.weights.get(i).copied().unwrap_or(0.);
                if weight <= 0. {
                    return;
                }
                let loaded = self.loaded.get(i).copied().unwrap_or([0.; 4]);
                if wet > 0. {
                    let bin = self.bins.get(i).copied().unwrap_or([0.; 4]);
                    let canvas = if bin[3] > 1e-6 { [bin[0] / bin[3], bin[1] / bin[3], bin[2] / bin[3], (bin[3] / weight).clamp(0., 1.)] } else { [0.; 4] };
                    if canvas[3] > 0. {
                        *nib = if nib[3] <= 0. { canvas } else { mix_in_space(*nib, canvas, exchange, pigment_mixing, &self.space) };
                    } else if refill > 0. && nib[3] > 0. && loaded[3] > 0. {
                        let color_exchange = refill * loaded[3].clamp(0., 1.);
                        for c in 0..3 {
                            nib[c] += (loaded[c] - nib[c]) * color_exchange;
                        }
                    }
                }
                // Mix=100 selects canvas pigment only when that cell actually
                // carries some. With no pickup, Photoshop retains loaded paint;
                // positive-alpha pickup keeps the existing wet weights unchanged.
                let has_pickup = nib[3] > 0.;
                let wr = self.state.amount * (if has_pickup { 1. - mix } else { 1. }) * loaded[3];
                let wp = if has_pickup { mix * nib[3] } else { 0. };
                let total = wr + wp;
                let mut paint = if total > 1e-6 { mix_in_space(loaded, *nib, wp / total, pigment_mixing, &self.space) } else { [0.; 4] };
                let opacity = nib[3].clamp(0., 1.);
                // A translucent pigment layer can colour a wet film strongly.
                // The original pickup alpha still controls deposited mass above.
                let coverage = if local_wet_film && opacity > 0. && opacity < 1. { opacity * 1.3 / (opacity + 0.3) } else { opacity };
                paint = mix_in_space(paint, *nib, film * coverage, pigment_mixing, &self.space);
                paint[3] = 1.;
                from_rgba_into(&fmt, paint, native);
                *deposit = total.min(1.);
            });
            let mut summary = [0.; 4];
            let mut summary_weight = 0.;
            for (nib, &weight) in self.state.nib.iter().zip(&self.weights) {
                if weight > 0. && nib[3] > 0. {
                    for (v, c) in summary.iter_mut().zip(*nib) {
                        *v += c * weight;
                    }
                    summary_weight += weight;
                }
            }
            self.state.pickup = (summary_weight > 0.).then(|| summary.map(|v| v / summary_weight));
            for y in area.y0..area.y1 {
                for x in area.x0..area.x1 {
                    let coverage = fp.at(x, y);
                    if coverage <= 0. {
                        continue;
                    }
                    let i = mapping(x, y);
                    let contact = coverage * fp.dab.opacity.clamp(0., 1.) * self.deposits.get(i).copied().unwrap_or(0.);
                    let k = if self.state.nib.get(i).is_some_and(|nib| nib[3] > 0.) { deposition.at(contact) } else { dry_deposition(m.flow, contact) };
                    if k > 0.
                        && let Some(paint) = self.paints.get(i * channels..(i + 1) * channels)
                    {
                        over_native(&fmt, work.px_mut(x, y), paint, k, lock);
                    }
                }
            }
        }
        if !self.state.canvas_loaded {
            self.state.amount *= drain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Stroke, wet_paint::apply_spatial_mixer};
    use photocraft_color::ColorMode;

    #[test]
    fn large_spectral_dabs_share_only_independent_cells() {
        let small = Rect::new(0, 0, 127, 64);
        let large = Rect::new(0, 0, 128, 64);
        assert!(!parallel_spectral_dab(small));
        #[cfg(target_arch = "wasm32")]
        assert!(!parallel_spectral_dab(large));
        #[cfg(not(target_arch = "wasm32"))]
        {
            let run = |workers| {
                let pool = rayon::ThreadPoolBuilder::new().num_threads(workers).build().unwrap();
                pool.install(|| {
                    let parallel = parallel_spectral_dab(large);
                    assert_eq!(parallel, workers > 1);
                    // Five native channels exercise independently strided CMYK + alpha writes.
                    let mut nib = vec![[0.2, 0.4, 0.8, 1.]; 8192];
                    let mut paints = vec![0.; nib.len() * 5];
                    let mut deposits = vec![0.; nib.len()];
                    mix_nib_cells(&mut nib, &mut paints, &mut deposits, 5, parallel, |i, cell, paint, deposit| {
                        let ratio = (i % 101) as f32 * 0.01;
                        *cell = crate::wet_paint::pigment_mix(*cell, [0.9, 0.75, 0.1, 1.], ratio, true);
                        from_rgba_into(&PixelFormat::new(ColorMode::Cmyk, SampleType::F32, true), *cell, paint);
                        *deposit = ratio;
                    });
                    (nib, paints, deposits)
                })
            };
            assert_eq!(run(1), run(3));
        }
    }

    #[test]
    fn tail_probe_is_non_mutating_and_repeated_finish_does_not_deposit() {
        for smoothing in [0., 0.5] {
            let mut original = Surface::new(PixelFormat::RGBA32F);
            original.fill_rect(Rect::new(0, 0, 100, 100), &[0., 0., 1., 1.]);
            let mut brush = BrushSettings { size: 20., pressure_size: false, ..Default::default() };
            brush.smoothing.amount = smoothing;
            let mut state = MixerState::default();
            state.load([1., 0., 0., 1.]);
            let mut live = MixerStroke::new(&original, None, &brush, &MixerSettings::default(), state);
            let mut target = original;
            live.push(&[StrokePoint::new(20., 20., 1.), StrokePoint::new(65., 55., 0.2)], &mut target, None, false);
            let expected = live.has_pending_tail();
            assert_eq!(expected, live.has_pending_tail());
            let damage = live.finish(&mut target, None, false);
            assert_eq!(expected, !damage.is_empty());
            let before = target.read_region(Rect::new(0, 0, 100, 100));
            assert!(!live.has_pending_tail());
            assert!(live.finish(&mut target, None, false).is_empty());
            assert_eq!(before, target.read_region(Rect::new(0, 0, 100, 100)));
        }
    }
    #[test]
    fn partitions_match_reference_across_depths_modes_selection_and_lock() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                for wet in [0., 0.5, 1.] {
                    for lock in [false, true] {
                        let fmt = PixelFormat::new(mode, depth, true);
                        let area = Rect::new(-25, -25, 105, 75);
                        let mut original = Surface::new(fmt);
                        original.write_region(area, &vec![0.4; area.width() as usize * area.height() as usize * fmt.channels()]);
                        let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
                        selection.write_region(area, &vec![0.37; area.width() as usize * area.height() as usize]);
                        let mut brush = BrushSettings { size: 12., spacing: 0.13, ..Default::default() };
                        brush.smoothing.amount = 0.2;
                        brush.tip = crate::TipShape::Sampled(crate::GrayTile::from_fn(8, 8, |x, y| ((x + y) % 3) as f32 / 2.));
                        let stroke = Stroke {
                            brush,
                            points: (0..9).map(|i| StrokePoint::new(i as f64 * 8., 20. + (i as f64).sin() * 10., 0.2 + i as f32 * 0.08)).collect(),
                        };
                        let m = MixerSettings { wet, load: 0.8, mix: 0.6, flow: 0.43, ..Default::default() };
                        let mut state = MixerState::default();
                        state.load([0.8, 0.2, 0.1, 0.7]);
                        state.loaded_nib = (0..16).map(|i| [i as f32 / 16., 0.2, 0.8, 0.7]).collect();
                        let mut reference = original.clone();
                        let mut expected = state.clone();
                        apply_spatial_mixer(&mut reference, None, &stroke, &m, &mut expected, Some(&selection), lock);
                        for chunk in [1, 3, 9] {
                            let mut actual = original.clone();
                            let mut live = MixerStroke::new(&original, None, &stroke.brush, &m, state.clone());
                            for pts in stroke.points.chunks(chunk) {
                                live.push(pts, &mut actual, Some(&selection), lock);
                            }
                            live.finish(&mut actual, Some(&selection), lock);
                            let error = reference.read_region(area).iter().zip(actual.read_region(area)).map(|(a, b)| (a - b).abs()).fold(0., f32::max);
                            assert!(error < 1e-6, "{mode:?} {depth:?} wet={wet} lock={lock} chunk={chunk}: {error}");
                            assert_eq!(expected, live.state);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "mixer_live_tests_v18.rs"]
mod tests_v18;

#[cfg(test)]
#[path = "mixer_material_tests_v56.rs"]
mod tests_v56;
