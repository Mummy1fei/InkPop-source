//! Reproducible local-stroke benchmark on a 24 MP canvas; setup is outside the timed region.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_paint::{
    BrushSettings, Stroke, StrokePoint,
    mixer::{MixerSettings, MixerState, apply_mixer_stroke},
    retouch::apply_dab_stroke,
    wet_paint,
};
use photocraft_raster::Surface;
fn main() {
    if let Some(path) = std::env::args().nth(1) {
        if let Err(e) = focus(&path) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    let mut canvas = Surface::new(PixelFormat::RGBA32F);
    canvas.fill_rect(Rect::new(0, 0, 6000, 4000), &[0.8, 0.7, 0.2, 1.]);
    canvas.fill_rect(Rect::new(0, 0, 3000, 4000), &[0.1, 0.25, 0.9, 1.]);
    let stroke = Stroke {
        brush: BrushSettings { size: 96., spacing: 0.15, pressure_size: false, ..Default::default() },
        points: vec![StrokePoint::new(2850., 2000., 1.), StrokePoint::new(3150., 2000., 1.)],
    };
    let scalar = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
    for (label, multicolor, simulate_paint) in [
        ("legacy", false, false),
        ("spatial", true, false),
        ("route-C-reference", true, true),
        ("route-C-separate-fields", true, true),
        ("route-C", true, true),
    ] {
        let mut times = Vec::new();
        let mut stages = Vec::new();
        for _ in 0..3 {
            let mut surface = canvas.clone();
            let mut state = MixerState::default();
            state.load([0.9, 0.25, 0.1, 1.]);
            let m = MixerSettings { multicolor, simulate_paint, ..Default::default() };
            let mut water = Surface::new(scalar);
            let mut height = Surface::new(scalar);
            let start = std::time::Instant::now();
            let area = apply_mixer_stroke(&mut surface, None, &stroke, &m, &mut state, None, false);
            let after_mix = start.elapsed().as_secs_f64() * 1000.;
            let mut fields = 0.;
            let mut evolve = 0.;
            let mut render = 0.;
            if simulate_paint {
                if label == "route-C-separate-fields" {
                    reference_fields(&mut water, &mut height, &stroke, &m, None);
                } else {
                    wet_paint::deposit_fields(&mut water, &mut height, &stroke, &m, None);
                }
                fields = start.elapsed().as_secs_f64() * 1000. - after_mix;
                if label == "route-C-reference" {
                    reference_step(&mut surface, &mut water, &height, area, &m);
                } else {
                    wet_paint::step(&mut surface, &mut water, &mut height, area, 0.25, &m, None);
                }
                evolve = start.elapsed().as_secs_f64() * 1000. - after_mix - fields;
                let mut display = surface.clone();
                if label == "route-C-reference" {
                    reference_render(&surface, &height, &mut display, area);
                } else {
                    wet_paint::render(&surface, &height, &mut display, area);
                }
                render = start.elapsed().as_secs_f64() * 1000. - after_mix - fields - evolve;
                std::hint::black_box(display);
            }
            times.push(start.elapsed().as_secs_f64() * 1000.);
            stages.push([after_mix, fields, evolve, render]);
            std::hint::black_box(surface);
        }
        println!("{label} stages [mix,fields,diffusion,display] ms={stages:?}");
        times.sort_by(f64::total_cmp);
        println!("{label}: median_ms={:.2}, samples={times:?}, canvas=6000x4000, size=96, length=300", times[1]);
    }
}

fn focus(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let presets: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let mut canvas = Surface::new(PixelFormat::RGBA32F);
    canvas.fill_rect(Rect::new(0, 0, 6000, 4000), &[0.8, 0.7, 0.2, 1.]);
    for p in presets.as_array().ok_or("expected preset array")? {
        let name = p.get("name").and_then(serde_json::Value::as_str).unwrap_or("");
        if !["♨ 水油彩", "♨ 水油彩 粉笔 ★★★", "♨ 油彩 费欣  细腻"].contains(&name) {
            continue;
        }
        let mut brush: BrushSettings = serde_json::from_value(p.get("brush").cloned().ok_or("no brush")?)?;
        brush.size = 200.;
        let m = brush.mixer.clone();
        let stroke =
            Stroke { brush, points: vec![StrokePoint::new(2850., 2000., 0.3), StrokePoint::new(3000., 2050., 1.), StrokePoint::new(3150., 2000., 0.3)] };
        let mut times = Vec::new();
        for _ in 0..7 {
            let mut target = canvas.clone();
            let mut state = MixerState::default();
            state.load([0.9, 0.2, 0.6, 1.]);
            let start = std::time::Instant::now();
            apply_mixer_stroke(&mut target, None, &stroke, &m, &mut state, None, false);
            times.push(start.elapsed().as_secs_f64() * 1000.);
            std::hint::black_box(target);
        }
        times.sort_by(f64::total_cmp);
        println!("{name}: median_ms={:.3}, samples={times:?}, 24MP, size=200, length=300", times[3]);
    }
    Ok(())
}

// Previous per-pixel evolution/display path for a matched A/B benchmark (untagged RGB only).
fn reference_step(p: &mut Surface, w: &mut Surface, h: &Surface, r: Rect, m: &MixerSettings) {
    let before = p.clone();
    let old = w.clone();
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let moisture = old.pixel(x, y)[0].clamp(0., 1.);
            let mut spread = moisture;
            let mut color = before.rgba(x, y);
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let neighbor = before.rgba(x + dx, y + dy);
                let wet = old.pixel(x + dx, y + dy)[0];
                spread += (wet - moisture) * m.diffusion * 0.25;
                if neighbor[3] > 0. {
                    let t = moisture.min(wet) * m.diffusion * 0.25;
                    if t > 0. {
                        color = wet_paint::pigment_mix(color, neighbor, t, true);
                    }
                }
            }
            p.write_pixel(x, y, &photocraft_raster::from_rgba(&p.format(), color));
            let volume = h.pixel(x, y)[0].max(0.);
            w.write_pixel(x, y, &[spread.clamp(0., 1.) * (-0.25 * m.drying / (1. + volume)).exp()]);
        }
    }
}
fn reference_render(p: &Surface, h: &Surface, target: &mut Surface, r: Rect) {
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let gx = h.pixel(x + 1, y)[0] - h.pixel(x - 1, y)[0];
            let gy = h.pixel(x, y + 1)[0] - h.pixel(x, y - 1)[0];
            let shade = (1. + (gx + gy) * 0.35).clamp(0.6, 1.3);
            let mut rgba = p.rgba(x, y);
            for c in rgba.iter_mut().take(3) {
                *c *= shade;
            }
            target.write_pixel(x, y, &photocraft_raster::from_rgba(&target.format(), rgba));
        }
    }
}

fn reference_fields(water: &mut Surface, height: &mut Surface, stroke: &Stroke, m: &MixerSettings, selection: Option<&Surface>) -> Rect {
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
