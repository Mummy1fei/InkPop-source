//! Colour ramps and Mixer Brush strokes for the InkPop pigment model.
//! Pass an output directory for the reference pixels and JSON measurements.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_paint::{
    BrushSettings, Control, Dynamic, Stroke, StrokePoint, Transfer,
    mixer::{MixerSettings, MixerState, apply_mixer_stroke},
    mixer_live::MixerStroke,
    wet_paint::{self, pigment_mix},
};
use photocraft_raster::Surface;
use serde_json::{Value, json};
use std::{error::Error, fs, hint::black_box, io::Write, path::Path, time::Instant};

type Rgba = [f32; 4];

const BLUE: [u8; 3] = [18, 60, 172];
const YELLOW: [u8; 3] = [251, 209, 25];
const CYAN: [u8; 3] = [18, 184, 212];
const MAGENTA: [u8; 3] = [205, 42, 143];
const OCHRE: [u8; 3] = [197, 133, 45];
const ULTRAMARINE: [u8; 3] = [61, 60, 154];
const UMBER: [u8; 3] = [109, 62, 41];
const WHITE: [u8; 3] = [245, 241, 228];
const PAIRS: [(&str, [u8; 3], [u8; 3]); 5] = [
    ("Blue + yellow", BLUE, YELLOW),
    ("Cyan + yellow", CYAN, YELLOW),
    ("Cyan + magenta", CYAN, MAGENTA),
    ("Ochre + ultramarine", OCHRE, ULTRAMARINE),
    ("Umber + off-white", UMBER, WHITE),
];
const WIDTH: i32 = 720;
const HEIGHT: i32 = 140;
const PREVIEW_BRUSH_DIAMETER: usize = 96;
const PASSES: usize = 3;
const TRIALS: usize = 7;
const KERNEL_PAIRS: usize = 32_768;
const RAMP_HEIGHT: usize = 24;

fn rgba8(rgb: [u8; 3]) -> Rgba {
    let [r, g, b] = rgb.map(|c| f32::from(c) / 255.0);
    [r, g, b, 1.]
}

fn rgb8(rgba: Rgba) -> [u8; 3] {
    let [r, g, b, _] = rgba.map(|c| (c.clamp(0., 1.) * 255.).round() as u8);
    [r, g, b]
}

fn write_ppm(path: &Path, width: usize, height: usize, rgb: &[u8]) -> std::io::Result<()> {
    let mut file = fs::File::create(path)?;
    writeln!(file, "P6\n{width} {height}\n255")?;
    file.write_all(rgb)
}

fn write_preview(path: &Path, surface: &Surface) -> std::io::Result<()> {
    let rgba = surface.read_region(Rect::new(0, 0, WIDTH, HEIGHT));
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for &[r, g, b, a] in rgba.as_chunks::<4>().0 {
        rgb.extend(rgb8([r, g, b, a]));
    }
    write_ppm(path, WIDTH as usize, HEIGHT as usize, &rgb)
}

fn swatches(out: &Path) -> Result<Vec<Value>, Box<dyn Error>> {
    let mut image = Vec::new();
    let mut midpoints = Vec::new();
    for (name, a, b) in PAIRS {
        let (a_float, b_float) = (rgba8(a), rgba8(b));
        let mut line = Vec::with_capacity(WIDTH as usize * 3);
        for x in 0..WIDTH {
            line.extend(rgb8(pigment_mix(a_float, b_float, x as f32 / (WIDTH - 1) as f32, true)));
        }
        for _ in 0..RAMP_HEIGHT {
            image.extend_from_slice(&line);
        }
        let mix = pigment_mix(a_float, b_float, 0.5, true);
        midpoints.push(json!({"name": name, "a": a, "b": b, "midpoint": rgb8(mix)}));
    }
    write_ppm(&out.join("swatches.ppm"), WIDTH as usize, RAMP_HEIGHT * PAIRS.len(), &image)?;
    Ok(midpoints)
}

fn plate(bounds: Rect, stroke_left: i32) -> Surface {
    let mut surface = Surface::new(PixelFormat::RGBA16);
    let x1 = stroke_left + 205;
    let x2 = stroke_left + 410;
    let x3 = stroke_left + 575;
    for (left, right, colour) in [(bounds.x0, x1, BLUE), (x1, x2, YELLOW), (x2, x3, MAGENTA), (x3, bounds.x1, WHITE)] {
        surface.fill_rect(Rect::new(left, bounds.y0, right, bounds.y1), &rgba8(colour));
    }
    surface
}

fn stroke(left: i32, top: i32, pass: usize, brush: BrushSettings) -> Stroke {
    let path = [
        (56., 0., 0.22),
        (137., -9., 0.48),
        (218., -11., 0.79),
        (299., -6., 1.),
        (380., 2., 0.68),
        (461., 10., 0.4),
        (542., 9., 0.93),
        (625., 4., 0.74),
        (666., 0., 0.26),
    ];
    let samples = path.into_iter().zip(path.into_iter().skip(1)).flat_map(|((x, y, p), (nx, ny, np))| {
        (0..4).map(move |step| {
            let t = f64::from(step) / 4.;
            (x + (nx - x) * t, y + (ny - y) * t, p + (np - p) * t as f32)
        })
    });
    let mut points: Vec<_> = samples
        .chain(path.last().copied())
        .map(|(x, y, p)| StrokePoint::new(f64::from(left) + x, f64::from(top + HEIGHT / 2) + y + (pass as f64 - 1.) * 6., p))
        .collect();
    if pass == 1 {
        points.reverse();
    }
    for (i, point) in points.iter_mut().enumerate() {
        point.time = (i * 8) as f64;
    }
    Stroke { brush, points }
}

#[derive(Clone, Copy)]
enum Scenario<'a> {
    LiveWet,
    Simulated,
    DryControl,
    ActualPreset(&'a BrushSettings),
}

impl Scenario<'_> {
    fn name(self) -> &'static str {
        match self {
            Self::LiveWet => "live-wet",
            Self::Simulated => "simulated",
            Self::DryControl => "dry-control",
            Self::ActualPreset(_) => "actual-preset",
        }
    }

    fn brush(self, diameter: usize) -> BrushSettings {
        if let Self::ActualPreset(preset) = self {
            return BrushSettings { size: diameter as f32, ..preset.clone() };
        }
        BrushSettings {
            size: diameter as f32,
            pressure_size: true,
            hardness: 0.55,
            spacing: 0.18,
            transfer: Transfer { enabled: true, wetness: Dynamic::controlled(Control::PenPressure), ..Default::default() },
            ..Default::default()
        }
    }

    fn settings(self) -> MixerSettings {
        if let Self::ActualPreset(preset) = self {
            return preset.mixer.clone();
        }
        MixerSettings {
            // Core conditions from the bundled water/oil preset regression,
            // with a synthetic computed nib and no imported preset data.
            wet: if matches!(self, Self::DryControl) { 0. } else { 1. },
            load: 1.,
            mix: 0.,
            flow: 1.,
            clean_after_stroke: false,
            load_after_stroke: false,
            simulate_paint: matches!(self, Self::Simulated),
            ..Default::default()
        }
    }
}

fn stroke_series(
    scenario: Scenario<'_>,
    original: &Surface,
    bounds: Rect,
    origin: (i32, i32),
    diameter: usize,
    previews: Option<&Path>,
) -> Result<(Vec<f64>, Vec<f64>), Box<dyn Error>> {
    let settings = scenario.settings();
    let mut surface = original.clone();
    let mut pigment = original.clone();
    let scalar = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
    let mut water = Surface::new(scalar);
    let mut thickness = Surface::new(scalar);
    let mut state = MixerState::default();
    state.load(rgba8(YELLOW));
    let mut stroke_ms = Vec::new();
    let mut live_batch_ms = Vec::new();
    for pass in 0..PASSES {
        let stroke = stroke(origin.0, origin.1, pass, scenario.brush(diameter));
        let start = Instant::now();
        match scenario {
            Scenario::LiveWet | Scenario::ActualPreset(_) => {
                let mut live = MixerStroke::new(&surface, None, &stroke.brush, &settings, state);
                for point in &stroke.points {
                    let batch_start = Instant::now();
                    live.push(std::slice::from_ref(point), &mut surface, None, false);
                    live_batch_ms.push(batch_start.elapsed().as_secs_f64() * 1000.);
                }
                live.finish(&mut surface, None, false);
                state = live.state;
            }
            Scenario::Simulated => {
                let area = apply_mixer_stroke(&mut pigment, None, &stroke, &settings, &mut state, None, false).intersect(&bounds);
                wet_paint::deposit_fields(&mut water, &mut thickness, &stroke, &settings, None);
                wet_paint::step(&mut pigment, &mut water, &mut thickness, area, 0.25, &settings, None);
                wet_paint::render(&pigment, &thickness, &mut surface, area);
            }
            Scenario::DryControl => {
                apply_mixer_stroke(&mut surface, None, &stroke, &settings, &mut state, None, false);
            }
        }
        if settings.clean_after_stroke {
            state.clean();
        }
        if settings.load_after_stroke {
            state.load(rgba8(YELLOW));
        }
        stroke_ms.push(start.elapsed().as_secs_f64() * 1000.);
        if let Some(path) = previews.filter(|_| pass == 0 || pass + 1 == PASSES) {
            write_preview(&path.join(format!("{}-pass-{}.ppm", scenario.name(), pass + 1)), &surface)?;
        }
    }
    black_box((surface, state));
    Ok((stroke_ms, live_batch_ms))
}

fn kernel_ns() -> Vec<f64> {
    // A reproducible sRGB sequence with varied blend weights. Prepare
    // inputs outside timing and use the output to keep the kernel call alive.
    let mut seed = 0x6a09_e667u32;
    let mut next = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / 16_777_215.
    };
    let input: Vec<_> = (0..KERNEL_PAIRS).map(|_| ([next(), next(), next(), 1.], [next(), next(), next(), 1.], next())).collect();
    let mut timings = Vec::new();
    for trial in 0..=TRIALS {
        let start = Instant::now();
        for &(a, b, t) in black_box(&input) {
            black_box(pigment_mix(a, b, t, true));
        }
        if trial > 0 {
            timings.push(start.elapsed().as_secs_f64() * 1e9 / KERNEL_PAIRS as f64);
        }
    }
    timings
}

fn percentile(values: &[f64], percent: usize) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = (values.len() * percent).div_ceil(100).saturating_sub(1);
    sorted.get(rank).copied().unwrap_or_default()
}

fn actual_water_oil_preset(path: &Path) -> Result<BrushSettings, Box<dyn Error>> {
    let collection: Value = serde_json::from_slice(&fs::read(path)?)?;
    let data = collection
        .as_array()
        .and_then(|presets| presets.iter().find(|preset| preset.get("name").and_then(Value::as_str) == Some("♨ 水油彩")))
        .and_then(|preset| preset.get("brush"))
        .ok_or("could not find the water/oil brush in the supplied collection")?;
    let brush: BrushSettings = serde_json::from_value(data.clone())?;
    if brush.mixer.wet <= 0. || !brush.mixer.multicolor || brush.mixer.simulate_paint {
        return Err("expected the legacy wet multicolour water/oil preset with simulation off".into());
    }
    Ok(brush)
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("usage: pigment_compare <output directory> [local wet preset collection]")?;
    let actual_preset = args.next().map(|path| actual_water_oil_preset(Path::new(&path))).transpose()?;
    let out = Path::new(&path);
    fs::create_dir_all(out)?;
    let midpoints = swatches(out)?;
    let preview_bounds = Rect::new(0, 0, WIDTH, HEIGHT);
    let preview = plate(preview_bounds, 0);
    write_preview(&out.join("input.ppm"), &preview)?;
    let bounds = Rect::new(0, 0, 6000, 4000);
    let origin = ((6000 - WIDTH) / 2, (4000 - HEIGHT) / 2);
    let benchmark = plate(bounds, origin.0);
    let mut strokes = Vec::new();
    let scenarios = [Scenario::LiveWet, Scenario::Simulated, Scenario::DryControl].into_iter().chain(actual_preset.as_ref().map(Scenario::ActualPreset));
    for scenario in scenarios {
        // The small visual fixture also warms this path before the timed trials.
        stroke_series(scenario, &preview, preview_bounds, (0, 0), PREVIEW_BRUSH_DIAMETER, Some(out))?;
        for diameter in [PREVIEW_BRUSH_DIAMETER, 200] {
            let mut series = Vec::new();
            let mut batches = Vec::new();
            for _ in 0..TRIALS {
                let (stroke_ms, live_batch_ms) = stroke_series(scenario, &benchmark, bounds, origin, diameter, None)?;
                series.push(stroke_ms.iter().sum::<f64>());
                batches.extend(live_batch_ms);
            }
            strokes.push(json!({
                "scenario": scenario.name(),
                "brush_diameter": diameter,
                "series_ms": series,
                "series_median_ms": percentile(&series, 50),
                "live_batch_ms": batches,
                "live_batch_median_ms": (!batches.is_empty()).then(|| percentile(&batches, 50)),
                "live_batch_p95_ms": (!batches.is_empty()).then(|| percentile(&batches, 95)),
            }));
        }
    }
    let kernel = kernel_ns();
    let result = json!({
        "model": "Kubelka–Munk",
        "canvas": [bounds.width(), bounds.height()],
        "pixel_format": "RGBA16",
        "preview_brush_diameter": PREVIEW_BRUSH_DIAMETER,
        "pressure_range": [0.22, 1.],
        "passes": PASSES,
        "trials": TRIALS,
        "kernel_ns": kernel,
        "kernel_median_ns": percentile(&kernel, 50),
        "kernel_pairs_per_trial": KERNEL_PAIRS,
        "midpoints": midpoints,
        "strokes": strokes,
    });
    let report = serde_json::to_string_pretty(&result)?;
    fs::write(out.join("measurements.json"), &report)?;
    println!("{report}");
    Ok(())
}
