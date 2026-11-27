use super::super::*;
use photocraft_color::ColorMode;
use photocraft_paint::{GrayTile, TipShape};

const FULL: Rect = Rect::new(0, 0, 128, 112);

fn periodic(x: i32, y: i32) -> [f32; 4] {
    // An 8x8 repeated tile with narrow masonry-like joints and fine detail.
    let (u, v) = (x.rem_euclid(8), y.rem_euclid(8));
    let joint = v < 2 || (u < 2 && v >= 2);
    let grain = ((u * 7 + v * 11) % 13) as f32 * 0.006;
    let value = if joint { 0.16 + grain } else { 0.6 + grain };
    [value, value * 0.8, value * 0.6, 1.0]
}

fn setup(format: PixelFormat) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":128,"height":112,"background":"transparent"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("synthetic repeated structure", |doc, _| {
        doc.mode = format.mode;
        doc.depth = format.sample;
        let mut surface = Surface::new(format);
        let mut native = Vec::new();
        for y in FULL.y0..FULL.y1 {
            for x in FULL.x0..FULL.x1 {
                native.extend(photocraft_raster::from_rgba(&format, periodic(x, y)));
            }
        }
        surface.write_region(FULL, &native);
        // Contaminate a small central area without corrupting the comparison ring.
        let stain = photocraft_raster::from_rgba(&format, [0.02, 0.01, 0.03, 1.0]);
        surface.fill_rect(Rect::new(58, 50, 70, 62), &stain);
        *doc.layer_mut(id).unwrap().surface_mut().unwrap() = surface;
        Ok(())
    })
    .unwrap();
    (s, id)
}

fn rmse(surface: &Surface) -> f32 {
    let mut squared = 0.0f32;
    let mut count = 0;
    for y in 48..64 {
        for x in 56..72 {
            let actual = surface.rgba(x, y);
            let wanted = periodic(x, y);
            for c in 0..3 {
                squared += (actual[c] - wanted[c]).powi(2);
                count += 1;
            }
        }
    }
    (squared / count as f32).sqrt()
}

#[test]
fn healing_commands_ignore_imported_tip_geometry_and_pressure_size_v82() {
    for cmd in ["paint.spotHealing", "paint.healingBrush"] {
        let (mut s, _) = setup(PixelFormat::RGBA8);
        s.tools.brush.tip = TipShape::Sampled(GrayTile::from_fn(3, 31, |_, _| 1.0));
        s.tools.brush.roundness = 0.12;
        s.tools.brush.angle = 71.0;
        s.tools.brush.texture.enabled = true;
        s.tools.brush.scattering.enabled = true;
        s.tools.brush.scattering.scatter.jitter = 4.0;
        s.tools.brush.dual_brush.enabled = true;
        s.tools.brush.shape_dynamics.enabled = true;
        s.tools.brush.shape_dynamics.size.jitter = 1.0;
        s.tools.brush.noise = true;
        s.tools.brush.pressure_size = true;
        s.tools.brush.pressure_opacity = true;
        let settings_before = s.tools.brush.clone();
        let params = json!({"points":[[64,56,0.1]],"size":24,"hardness":100,"flow":100,"spacing":10});
        let (round, _) = parse_brush(&s, &params, cmd).unwrap();
        let (low_bounds, low) = stroke_coverage(&round);
        let mut high = round.clone();
        high.points[0].pressure = 1.0;
        let (high_bounds, high_cov) = stroke_coverage(&high);
        assert_eq!(low_bounds, high_bounds, "{cmd}: diameter does not inherit painting pressure");
        assert_eq!(low, high_cov, "{cmd}: transfer does not inherit painting pressure");
        assert_eq!(round.brush.tip, TipShape::Round);
        assert_eq!(round.brush.roundness, 1.0);
        assert_eq!(round.brush.angle, 0.0);
        assert!(!round.brush.texture.enabled && !round.brush.scattering.enabled && !round.brush.dual_brush.enabled);
        let width = low_bounds.width() as usize;
        for (i, value) in low.iter().enumerate() {
            let (x, y) = (low_bounds.x0 + (i % width) as i32, low_bounds.y0 + (i / width) as i32);
            let distance = (x as f32 + 0.5 - 64.0).hypot(y as f32 + 0.5 - 56.0);
            if distance < 10.5 {
                assert!(*value > 0.99, "{cmd}: visible 24px circle has no inherited texture holes");
            }
            if distance > 12.75 {
                assert_eq!(*value, 0.0, "{cmd}: footprint does not escape the circular cursor");
            }
        }
        assert_eq!(s.tools.brush, settings_before, "normalisation never mutates the user's painting preset");
        let (smudge, _) = parse_brush(&s, &params, "paint.smudge").unwrap();
        assert_eq!(smudge.brush.tip, settings_before.tip, "Smudge still owns its imported tip");
    }
}

#[test]
fn coherent_donor_reconstructs_repeated_structure_without_changing_known_pixels_v82() {
    let w = 112;
    let h = 96;
    let mut image = Region::new(Rect::new(0, 0, w, h), 4);
    let mut truth = Vec::new();
    let mut hole = vec![false; w as usize * h as usize];
    for y in 0..h {
        for x in 0..w {
            truth.extend(periodic(x, y));
            hole[y as usize * w as usize + x as usize] = (x - 56).pow(2) + (y - 48).pow(2) < 12 * 12;
        }
    }
    image.data = truth.clone();
    for (index, &is_hole) in hole.iter().enumerate() {
        if is_hole {
            image.data[index * 4..index * 4 + 4].copy_from_slice(&[0.02, 0.01, 0.03, 1.0]);
        }
    }
    let out = super::coherent_fill(&image, &hole, 24.0, 56).unwrap();
    assert_eq!(out, truth, "strong ring agreement preserves complete texture phase");
    for (index, &is_hole) in hole.iter().enumerate() {
        if !is_hole {
            assert_eq!(&out[index * 4..index * 4 + 4], &image.data[index * 4..index * 4 + 4]);
        }
    }
}

#[test]
fn coherent_donor_refuses_uncorrelated_ring_and_preserves_general_fill_fallback_v82() {
    let (w, h) = (80usize, 72usize);
    let mut image = Region::new(Rect::new(0, 0, w as i32, h as i32), 1);
    let mut hole = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut value = (x as u32).wrapping_mul(0x9e37_79b9) ^ (y as u32).wrapping_mul(0x85eb_ca77);
            value ^= value >> 13;
            value = value.wrapping_mul(0xc2b2_ae3d);
            image.data[y * w + x] = (value >> 8) as f32 / 16_777_215.0;
            hole[y * w + x] = (x as i32 - 40).pow(2) + (y as i32 - 36).pow(2) < 8 * 8;
        }
    }
    assert!(super::coherent_fill(&image, &hole, 16.0, 40).is_none(), "uncertain donors must not replace the general PatchMatch path");
}

#[test]
fn coherent_donor_rejects_malformed_region_without_panicking_v82() {
    let mut image = Region::new(Rect::new(0, 0, 12, 12), 4);
    let hole = vec![false; 144];
    assert!(super::coherent_fill(&image, &hole[..40], 12.0, 24).is_none());
    image.data.pop();
    assert!(super::coherent_fill(&image, &hole, 12.0, 24).is_none());
    image.data.push(0.0);
    assert!(super::coherent_fill(&image, &hole, f32::NAN, 24).is_none());
}

#[test]
fn spot_healing_repeated_structure_is_precise_and_undoable_at_rgb_depths_v82() {
    for format in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        let (mut s, id) = setup(format);
        let before = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().clone();
        let history = s.active().unwrap().history.past_len();
        s.execute("paint.spotHealing", json!({"points":[[64,56,0.2]],"size":28,"hardness":100})).unwrap();
        let output = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().clone();
        let error = rmse(&output);
        assert!(error < 0.012, "{format:?}: repeated-structure RMSE {error}");
        assert_eq!(output.read_region(Rect::new(0, 0, 40, 40)), before.read_region(Rect::new(0, 0, 40, 40)));
        assert_eq!(s.active().unwrap().history.past_len(), history + 1);
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc.layer(id).unwrap().surface().unwrap(), &before);
        assert!(s.redo());
        assert_eq!(s.active().unwrap().doc.layer(id).unwrap().surface().unwrap(), &output);
    }
}

#[test]
fn circular_healing_footprint_keeps_runtime_native_modes_v82() {
    for mode in [ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let format = PixelFormat::new(mode, sample, true);
            let (s, _) = setup(format);
            let (stroke, _) = parse_brush(&s, &json!({"points":[[64,56]],"size":24}), "paint.spotHealing").unwrap();
            assert_eq!(stroke.brush.tip, TipShape::Round);
            let original = s.active().unwrap().doc.layers[0].surface().unwrap();
            let mut output = original.clone();
            let bounds = spot_heal_surface(&mut output, &s.active().unwrap().doc, &stroke, SpotType::ContentAware, false, None, false);
            assert!(!bounds.is_empty());
            assert_eq!(output.format(), format);
            assert_eq!(output.read_region(Rect::new(0, 0, 24, 24)), original.read_region(Rect::new(0, 0, 24, 24)));
            assert!(output.read_region(FULL).iter().all(|v| v.is_finite()));
        }
    }
}

/// Root runs this once after the unified release tests to retain a real old/new
/// ground-truth measurement. The legacy below is the frozen v81 spot path.
#[test]
#[ignore]
fn spot_healing_quality_report_v82() {
    for format in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        let (s, id) = setup(format);
        let pre = &s.active().unwrap().doc;
        let original = pre.layer(id).unwrap().surface().unwrap();
        let (stroke, _) = parse_brush(&s, &json!({"points":[[64,56]],"size":28,"hardness":100}), "paint.spotHealing").unwrap();
        let mut old = original.clone();
        legacy_spot(&mut old, pre, &stroke);
        let mut new = original.clone();
        spot_heal_surface(&mut new, pre, &stroke, SpotType::ContentAware, false, None, false);
        eprintln!("v82 periodic masonry {format:?}: damaged RMSE {:.8}, legacy RMSE {:.8}, candidate RMSE {:.8}", rmse(original), rmse(&old), rmse(&new));
        if let Ok(root) = std::env::var("INKPOP_V82_HEALING_EVIDENCE") {
            let root = std::path::PathBuf::from(root);
            std::fs::create_dir_all(&root).unwrap();
            for (name, surface) in [("damaged", original), ("legacy", &old), ("candidate", &new)] {
                let mut ppm = format!("P6\n{} {}\n255\n", FULL.width(), FULL.height()).into_bytes();
                for y in FULL.y0..FULL.y1 {
                    for x in FULL.x0..FULL.x1 {
                        let p = surface.rgba(x, y);
                        ppm.extend(p[..3].iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
                    }
                }
                std::fs::write(root.join(format!("{:?}-{name}.ppm", format.sample)), ppm).unwrap();
            }
        }
    }
}

/// A local repair on a dense 24MP document, excluding document construction.
/// This is an observational timing, not a hard machine-independent budget.
#[test]
#[ignore]
fn spot_healing_24mp_local_quality_perf_v82() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":6000,"height":4000,"background":"transparent"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("24MP synthetic repair fixture", |doc, _| {
        let surface = doc.layer_mut(id).unwrap().surface_mut().unwrap();
        surface.fill_rect(Rect::new(0, 0, 6000, 4000), &[0.5, 0.5, 0.5, 1.0]);
        let local = Rect::new(2880, 1880, 3120, 2120);
        let mut pixels = Vec::with_capacity(240 * 240 * 4);
        for y in local.y0..local.y1 {
            for x in local.x0..local.x1 {
                pixels.extend(periodic(x, y));
            }
        }
        surface.write_region(local, &pixels);
        surface.fill_rect(Rect::new(2994, 1994, 3006, 2006), &[0.02, 0.01, 0.03, 1.0]);
        Ok(())
    })
    .unwrap();
    let pre = &s.active().unwrap().doc;
    let original = pre.layer(id).unwrap().surface().unwrap();
    let (stroke, _) = parse_brush(&s, &json!({"points":[[3000,2000]],"size":28,"hardness":100}), "paint.spotHealing").unwrap();
    let mut timings = [Vec::new(), Vec::new()];
    for _ in 0..3 {
        for (index, times) in timings.iter_mut().enumerate() {
            let mut surface = original.clone();
            let start = std::time::Instant::now();
            if index == 0 {
                legacy_spot(&mut surface, pre, &stroke);
            } else {
                spot_heal_surface(&mut surface, pre, &stroke, SpotType::ContentAware, false, None, false);
            }
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(surface.rgba(100, 100), original.rgba(100, 100));
        }
    }
    for (name, times) in ["legacy", "candidate"].into_iter().zip(timings) {
        eprintln!("v82 spot-healing dense6000x4000 {name}: milliseconds {times:?}, mean {:.4}", times.iter().sum::<f64>() / times.len() as f64);
    }
}

fn fallback_fixture(format: PixelFormat, case: &str, width: i32, height: i32) -> (Session, LayerId, Stroke) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":width,"height":height,"background":"transparent"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    let (cx, cy) = (width / 2, height / 2);
    s.edit("non-repeated repair fixture", |doc, _| {
        doc.mode = format.mode;
        doc.depth = format.sample;
        let mut surface = Surface::new(format);
        surface.fill_rect(Rect::new(0, 0, width, height), &photocraft_raster::from_rgba(&format, [0.5, 0.5, 0.5, 1.0]));
        let local = Rect::new(cx - 120, cy - 120, cx + 120, cy + 120).intersect(&doc.bounds());
        let mut pixels = Vec::new();
        for y in local.y0..local.y1 {
            for x in local.x0..local.x1 {
                let color = if case == "gradient" {
                    let u = (x - cx + 120) as f32 / 240.0;
                    let v = (y - cy + 120) as f32 / 240.0;
                    [0.15 + 0.6 * u, 0.2 + 0.5 * v, 0.12 + 0.3 * u + 0.25 * v, 1.0]
                } else {
                    let mut value = (x as u32).wrapping_mul(0x9e37_79b9) ^ (y as u32).wrapping_mul(0x85eb_ca77);
                    value ^= value >> 13;
                    value = value.wrapping_mul(0xc2b2_ae3d);
                    let v = (value >> 8) as f32 / 16_777_215.0;
                    [0.1 + 0.75 * v, 0.2 + 0.5 * v, 0.25 + 0.4 * v, 1.0]
                };
                pixels.extend(photocraft_raster::from_rgba(&format, color));
            }
        }
        surface.write_region(local, &pixels);
        surface.fill_rect(Rect::new(cx - 6, cy - 6, cx + 6, cy + 6), &photocraft_raster::from_rgba(&format, [0.02, 0.01, 0.03, 1.0]));
        *doc.layer_mut(id).unwrap().surface_mut().unwrap() = surface;
        Ok(())
    })
    .unwrap();
    let (stroke, _) = parse_brush(&s, &json!({"points":[[cx,cy]],"size":28,"hardness":100}), "paint.spotHealing").unwrap();
    (s, id, stroke)
}

fn assert_probe_rejects(surface: &Surface, pre: &Document, stroke: &Stroke, case: &str) {
    let (bounds, cov) = stroke_coverage(stroke);
    let size = stroke.brush.size;
    let margin = (size.max(8.0) + size.clamp(8.0, 128.0)).ceil() as i32 + 8;
    let rect = bounds.inflate(margin).intersect(&pre.bounds());
    let image = Region::read(surface, rect);
    let (w, h) = (image.width(), image.height());
    let mut hole = vec![false; w * h];
    let bw = bounds.width() as usize;
    for (i, c) in cov.iter().enumerate() {
        let (x, y) = (bounds.x0 + (i % bw) as i32, bounds.y0 + (i / bw) as i32);
        if rect.contains(x, y) {
            hole[image.index(x, y)] = *c > 0.0;
        }
    }
    let domain = dilate(w, h, &hole, 2);
    assert!(super::coherent_fill(&image, &domain, size, margin).is_none(), "{case}: measurement must exercise fallback rather than the coherent success path");
}

#[test]
fn spot_healing_no_match_fallback_preserves_legacy_pixels_v82() {
    for format in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        for case in ["hashed", "gradient"] {
            let (s, id, stroke) = fallback_fixture(format, case, 256, 240);
            let pre = &s.active().unwrap().doc;
            let original = pre.layer(id).unwrap().surface().unwrap();
            assert_probe_rejects(original, pre, &stroke, case);
            let mut old = original.clone();
            legacy_spot(&mut old, pre, &stroke);
            let mut new = original.clone();
            spot_heal_surface(&mut new, pre, &stroke, SpotType::ContentAware, false, None, false);
            assert_eq!(new, old, "{case} {format:?}: a failed extended probe leaves the entire legacy fallback result exact");
        }
    }
}

/// Excludes document construction and the assertion probe; both timed paths
/// include their own work. No fixed-speed assertion across different machines.
#[test]
#[ignore]
fn spot_healing_24mp_no_match_perf_v82() {
    for case in ["hashed", "gradient"] {
        let (s, id, stroke) = fallback_fixture(PixelFormat::RGBA8, case, 6000, 4000);
        let pre = &s.active().unwrap().doc;
        let original = pre.layer(id).unwrap().surface().unwrap();
        assert_probe_rejects(original, pre, &stroke, case);
        let mut timings = [Vec::new(), Vec::new()];
        for _ in 0..3 {
            for (index, times) in timings.iter_mut().enumerate() {
                let mut surface = original.clone();
                let start = std::time::Instant::now();
                if index == 0 {
                    legacy_spot(&mut surface, pre, &stroke);
                } else {
                    spot_heal_surface(&mut surface, pre, &stroke, SpotType::ContentAware, false, None, false);
                }
                times.push(start.elapsed().as_secs_f64() * 1000.0);
                assert_eq!(surface.rgba(100, 100), original.rgba(100, 100));
            }
        }
        for (name, times) in ["legacy", "candidate"].into_iter().zip(timings) {
            eprintln!(
                "v82 spot-healing fallback {case} dense6000x4000 {name}: milliseconds {times:?}, mean {:.4}",
                times.iter().sum::<f64>() / times.len() as f64
            );
        }
    }
}

fn legacy_spot(surface: &mut Surface, pre: &Document, stroke: &Stroke) {
    let (bounds, cov) = stroke_coverage(stroke);
    let margin = stroke.brush.size.max(8.0).ceil() as i32 + 8;
    let rect = bounds.inflate(margin).intersect(&pre.bounds());
    let image = Region::read(surface, rect);
    let (w, h, ch) = (image.width(), image.height(), image.ch);
    let mut coverage = vec![0.0; w * h];
    let bw = bounds.width() as usize;
    for (i, c) in cov.iter().enumerate() {
        let (x, y) = (bounds.x0 + (i % bw) as i32, bounds.y0 + (i / bw) as i32);
        if rect.contains(x, y) {
            coverage[image.index(x, y)] = *c;
        }
    }
    let hole: Vec<_> = coverage.iter().map(|c| *c > 0.0).collect();
    let domain = dilate(w, h, &hole, 2);
    let mask = dilate(w, h, &hole, 1);
    let filled =
        inpaint::complete(w, h, ch, &image.data, &domain, &CompleteParams::default()).unwrap_or_else(|| poisson::membrane_fill(w, h, ch, &image.data, &domain));
    let source = Region { rect, ch, data: filled };
    let healed = heal_region(&surface.format(), &source, &image, &mask);
    let dmg = bounds.intersect(&rect);
    let weights: Vec<_> = (dmg.y0..dmg.y1).flat_map(|y| (dmg.x0..dmg.x1).map(move |x| (x, y))).map(|(x, y)| coverage[image.index(x, y)]).collect();
    apply_coverage(surface, dmg, &weights, stroke.brush.opacity, None, false, &healed, BlendMode::Normal);
}
