//! Real Camera Raw proxy-update timing on a 36 MP document (CPU preparation, not GPU presentation).
use photocraft_ui_egui::{PhotocraftApp, camera_raw_ui};
use serde_json::{Value, json};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let quick = args.iter().any(|a| a == "--quick");
    let (width, height, reps) = if quick { (6000, 4000, 12) } else { (7360, 4912, 40) };
    if args.iter().any(|a| a == "--region" || a == "--local-detail") {
        let report = region_benchmark(width, height, if quick { 3 } else { 5 }, args.iter().any(|a| a == "--local-detail"))?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        if let Some(path) = args.iter().position(|a| a == "--json").and_then(|i| args.get(i + 1)) {
            std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
        }
        return Ok(());
    }
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    // Match the actual preview window before opening, rather than egui's default 10000 pt
    // offscreen area which unintentionally exercised the maximum 4 MP proxy.
    let high_dpi = args.iter().any(|a| a == "--high-dpi");
    let mut input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, if high_dpi { egui::vec2(1440.0, 900.0) } else { egui::vec2(1200.0, 800.0) })),
        ..Default::default()
    };
    if high_dpi && let Some(viewport) = input.viewports.get_mut(&egui::ViewportId::ROOT) {
        viewport.native_pixels_per_point = Some(2.0);
    }
    let mut output = ctx.run_ui(input, |_| {});
    output.textures_delta.clear();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    // A full-resolution synthetic image with dense hue/saturation coverage and exact clipped
    // endpoints. Setup/import is outside the interactive measurements.
    let mut bytes = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            bytes.extend([(x % 256) as u8, (y % 256) as u8, ((x ^ y) % 256) as u8]);
        }
    }
    let image =
        photocraft_codecs::Image::from_raw(width as u32, height as u32, photocraft_codecs::ChannelLayout::Rgb, photocraft_codecs::SampleType::U8, bytes)?;
    let encoded = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default())?;
    app.session.open_document(photocraft_io::import("scope.png", &encoded)?.document, None);
    camera_raw_ui::menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).ok_or("no dialog entry point")??;
    if args.iter().any(|a| a == "--live-drag") {
        let report = live_drag_benchmark(&mut app, &ctx, width, height, if quick { 3.0 } else { 6.0 })?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        if let Some(path) = args.iter().position(|a| a == "--json").and_then(|i| args.get(i + 1)) {
            std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
        }
        return Ok(());
    }
    if args.iter().any(|a| a == "--async") {
        let settings = |i: usize| {
            json!({"exposure":i as f32 / 24.0, "contrast":i as f32,
            "temperature":-(i as f32), "texture":25.0,"clarity":15.0,"noiseLuminance":20.0})
        };
        let mut blocking = Vec::new();
        for i in 0..reps {
            let start = std::time::Instant::now();
            camera_raw_ui::menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"set":settings(i)}})).ok_or("no dialog")??;
            blocking.push(start.elapsed().as_secs_f64() * 1000.);
            let mut output = ctx.run_ui(Default::default(), |_| {});
            output.textures_delta.clear();
        }
        let expected = camera_raw_ui::menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{}})).ok_or("no dialog")??;
        let mut dispatch = Vec::new();
        let start = std::time::Instant::now();
        for i in 0..reps {
            let tick = std::time::Instant::now();
            camera_raw_ui::menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"set":settings(i),"async":true}})).ok_or("no dialog")??;
            dispatch.push(tick.elapsed().as_secs_f64() * 1000.);
        }
        let released = std::time::Instant::now();
        let mut frames = Vec::new();
        let final_state = loop {
            let tick = std::time::Instant::now();
            let mut output = ctx.run_ui(
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000., 700.))), ..Default::default() },
                |ui| camera_raw_ui::show(&mut app, ui.ctx()),
            );
            std::hint::black_box(ctx.tessellate(output.shapes, output.pixels_per_point));
            output.textures_delta.clear();
            frames.push(tick.elapsed().as_secs_f64() * 1000.);
            let state = camera_raw_ui::menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{}})).ok_or("no dialog")??;
            if state["previewPending"] == false {
                break state;
            }
            if released.elapsed().as_secs() > 30 {
                return Err("preview did not settle without input".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        if final_state["histogram"] != expected["histogram"] || final_state["previewRevision"] != final_state["requestedRevision"] {
            return Err("latest preview differs from synchronous reference".into());
        }
        let report = json!({"source":[width,height],"proxy":final_state["proxy"],"edits":reps,
            "sync_dispatch_ms":blocking,"async_dispatch_ms":dispatch,"eventless_frame_ms":frames,
            "release_to_latest_ms":released.elapsed().as_secs_f64()*1000.,
            "burst_total_ms":start.elapsed().as_secs_f64()*1000.,
            "latest_worker_ms":final_state["renderMs"],"histogram_matches_reference":true});
        println!("{}", serde_json::to_string_pretty(&report)?);
        if let Some(path) = args.iter().position(|a| a == "--json").and_then(|i| args.get(i + 1)) {
            std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
        }
        return Ok(());
    }
    let mut samples = Vec::new();
    let mut histogram_samples = Vec::new();
    for i in 0..reps + 3 {
        let r = camera_raw_ui::menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": if i % 2 == 0 { 0.25 } else { 0.75 }}}}))
            .ok_or("no dialog entry point")??;
        if i >= 3 {
            samples.push(r.get("renderMs").and_then(Value::as_f64).ok_or("no render timing")?);
            if let Some(ms) = r.get("histogramMs").and_then(Value::as_f64) {
                histogram_samples.push(ms);
            }
        }
        // Drain texture deltas just as a real frame does, rather than retaining every upload.
        let mut output = ctx.run_ui(Default::default(), |_| {});
        output.textures_delta.clear();
    }
    // Measure the complete native dialog CPU path with every SDR scope feature enabled:
    // filter preparation + ICC vectorscope + clipping texture + nine Lab probes + pixel-hover overlays + UI mesh.
    // A full-coverage selection exercises the mask blend and selected-region scope without
    // reducing the amount of analysed colour data. Selection setup is outside the timer.
    app.run("select.rect", json!({"x":0,"y":0,"width":width,"height":height}))?;
    camera_raw_ui::menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).ok_or("no dialog entry point")??;
    camera_raw_ui::menu(
        &mut app,
        &ctx,
        "filter.cameraRaw",
        &json!({"ui":{"scope":{
            "vectorscope":true,"selectedRegion":true,"lab":true,"shadows":true,"highlights":true,
            "samplers":(0..9).map(|i| [i as f32/9.0,0.5]).collect::<Vec<_>>()
        }}}),
    )
    .ok_or("no dialog entry point")??;
    let mut interactive_samples = Vec::new();
    let mut scope_samples = Vec::new();
    let mut idle_samples = Vec::new();
    let input = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
        events: vec![egui::Event::PointerMoved(egui::pos2(430.0, 400.0))],
        ..Default::default()
    };
    for i in 0..reps + 3 {
        let start = std::time::Instant::now();
        let r = camera_raw_ui::menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"set":{"exposure":if i%2==0 {0.25}else{0.75}},"scope":{}}}))
            .ok_or("no dialog entry point")??;
        let mut output = ctx.run_ui(input(), |ui| camera_raw_ui::show(&mut app, ui.ctx()));
        std::hint::black_box(ctx.tessellate(output.shapes, output.pixels_per_point));
        output.textures_delta.clear();
        if i >= 3 {
            interactive_samples.push(start.elapsed().as_secs_f64() * 1000.0);
            scope_samples.push(r.get("scopeMs").and_then(Value::as_f64).ok_or("no scope timing")?);
        }
        let start = std::time::Instant::now();
        let mut output = ctx.run_ui(input(), |ui| camera_raw_ui::show(&mut app, ui.ctx()));
        std::hint::black_box(ctx.tessellate(output.shapes, output.pixels_per_point));
        output.textures_delta.clear();
        if i >= 3 {
            idle_samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        if i == 3 {
            let state = camera_raw_ui::menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{}})).ok_or("no dialog entry point")??;
            if state["hoverSample"]["hueSaturation"].is_null() {
                return Err("benchmark must render the RGB badge and vectorscope pixel target".into());
            }
        }
    }
    // Dense RGB curves exercise the shared widget, including its outlines and egui CPU
    // tessellation. This is measured separately from pixel processing and GPU presentation.
    let pixels: Vec<_> = (0..8192)
        .map(|i| {
            let x = (i % 128) as f32;
            let y = (i / 128) as f32;
            [0.5 + 0.4 * (x * 0.07).sin(), 0.5 + 0.4 * (y * 0.13).cos(), (x + y) / 192.0, 1.0]
        })
        .collect();
    let histogram = photocraft_algo::histogram::RgbHistogram::from_rgba(&pixels);
    let mut draw_samples = Vec::new();
    for i in 0..reps + 3 {
        let start = std::time::Instant::now();
        let mut output = ctx.run_ui(
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(330.0, 160.0))), ..Default::default() },
            |ui| {
                photocraft_ui_egui::widgets::rgb_histogram(ui, &histogram, 110.0);
            },
        );
        let _meshes = ctx.tessellate(output.shapes, output.pixels_per_point);
        output.textures_delta.clear();
        if i >= 3 {
            draw_samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let mut sorted = samples.clone();
    sorted.sort_by(f64::total_cmp);
    let p50 = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
    let p95 = sorted.get(sorted.len().saturating_sub(1) * 95 / 100).copied().unwrap_or(0.0);
    let max = sorted.last().copied().unwrap_or(0.0);
    let report = json!({"width": width, "height": height, "rows": [{"name": "exposure proxy update", "median_ms": p50, "p95_ms": p95, "max_ms": max, "samples_ms": samples}, {"name": "RGB histogram", "samples_ms": histogram_samples}, {"name": "RGB histogram paint + tessellation", "samples_ms": draw_samples}, {"name":"interactive scope update", "samples_ms":interactive_samples}, {"name":"vectorscope ICC analysis", "samples_ms":scope_samples}, {"name":"cached scope frame", "samples_ms":idle_samples}]});
    println!("{}", serde_json::to_string_pretty(&report)?);
    if let Some(path) = args.iter().position(|a| a == "--json").and_then(|i| args.get(i + 1)) {
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    Ok(())
}

/// Sustained global edits while Primary stays held. This measures publication during the
/// gesture, rather than only timing the final frame after a burst of requests stops.
fn live_drag_benchmark(app: &mut PhotocraftApp, ctx: &egui::Context, width: usize, height: usize, seconds: f64) -> Result<Value, Box<dyn std::error::Error>> {
    let mut phases = Vec::new();
    let screen = ctx.content_rect();
    let pointer = egui::pos2(-10.0, -10.0);
    for zoom in [None, Some(1.0)] {
        app.ui.camera_raw_preview.zoom = zoom;
        let origin = ctx.input(|input| input.time);
        let started = std::time::Instant::now();
        let mut next_edit = 0.0;
        let mut edits = 0usize;
        let mut down = true;
        let mut records = Vec::new();
        let mut last_displayed = None;
        let mut frames = Vec::new();
        while started.elapsed().as_secs_f64() < seconds {
            let elapsed = started.elapsed().as_secs_f64();
            if elapsed >= next_edit {
                camera_raw_ui::menu(
                    app,
                    ctx,
                    "filter.cameraRaw",
                    &json!({"ui":{"async":true,"set":{
                        "clarity":10.0+(edits%40) as f32,"texture":25.0,"highlights":-20.0,
                        "exposure":(edits%40) as f32*0.02,"dehaze":10.0
                    }}}),
                )
                .ok_or("no dialog")??;
                edits += 1;
                next_edit = elapsed + 0.03;
            }
            let events = if down {
                vec![
                    egui::Event::PointerMoved(pointer),
                    egui::Event::PointerButton { pos: pointer, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                ]
            } else {
                Vec::new()
            };
            down = false;
            let tick = std::time::Instant::now();
            let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(origin + elapsed), events, ..Default::default() }, |ui| {
                camera_raw_ui::show(app, ui.ctx())
            });
            std::hint::black_box(ctx.tessellate(output.shapes, output.pixels_per_point));
            output.textures_delta.clear();
            frames.push(tick.elapsed().as_secs_f64() * 1000.0);
            if !ctx.input(|input| input.pointer.any_down()) {
                return Err("benchmark lost its held pointer".into());
            }
            let state = camera_raw_ui::menu(app, ctx, "filter.cameraRaw", &json!({"ui":{}})).ok_or("no dialog")??;
            let displayed = if zoom.is_some() { state["detailDisplayedRevision"].as_u64() } else { state["previewRevision"].as_u64() };
            if displayed != last_displayed {
                records.push(json!({"elapsedMs":started.elapsed().as_secs_f64()*1000.0,"requested":state["requestedRevision"],
                    "proxy":state["previewRevision"],"displayed":displayed,"approximate":state["detailApproximate"],
                    "proxyMs":state["renderMs"],"liveDetailMs":state["interactiveDetailMs"],
                    "proxyReduced":state["proxyReduced"],"interactionProxy":state["interactionProxy"]}));
                last_displayed = displayed;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let published = records.iter().filter(|record| !record["displayed"].is_null()).count();
        if published < 3 {
            return Err(format!("{zoom:?} published only {published} visible revisions while held").into());
        }
        if zoom.is_some() && records.iter().filter(|record| record["approximate"] == true).count() < 3 {
            return Err("zoomed global preview did not publish live detail while held".into());
        }
        let released = std::time::Instant::now();
        let mut up = true;
        let final_state = loop {
            if released.elapsed().as_secs() > 45 {
                return Err("exact result failed to settle after release".into());
            }
            let events = if up {
                vec![egui::Event::PointerButton { pos: pointer, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }]
            } else {
                Vec::new()
            };
            up = false;
            let mut output = ctx.run_ui(
                egui::RawInput { screen_rect: Some(screen), time: Some(origin + started.elapsed().as_secs_f64()), events, ..Default::default() },
                |ui| camera_raw_ui::show(app, ui.ctx()),
            );
            std::hint::black_box(ctx.tessellate(output.shapes, output.pixels_per_point));
            output.textures_delta.clear();
            let state = camera_raw_ui::menu(app, ctx, "filter.cameraRaw", &json!({"ui":{}})).ok_or("no dialog")??;
            if state["previewPending"] == false
                && (zoom.is_none()
                    || state["detailDisplayedRevision"] == state["requestedRevision"] && state["detailApproximate"] == false && state["detailPending"] == false)
            {
                break state;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        frames.sort_by(f64::total_cmp);
        phases.push(json!({"zoom":zoom,"heldSeconds":seconds,"edits":edits,"publishedWhileHeld":published,"records":records,
            "cpuFrameMedianMs":frames.get(frames.len()/2),"cpuFrameP95Ms":frames.get(frames.len().saturating_sub(1)*95/100),"cpuFrameMaxMs":frames.last(),
            "releaseToSettledMs":released.elapsed().as_secs_f64()*1000.0,"final":final_state}));
    }
    Ok(
        json!({"source":[width,height],"pixelsPerPoint":ctx.pixels_per_point(),"screen":[screen.width(),screen.height()],"phases":phases,"measurement":"CPU UI publication while pointer is continuously held; source-frequency reconstruction is approximate, exact engine refinement follows release; not GPU/input latency"}),
    )
}

/// The pointwise visible-region optimization and the former complete-domain path use the
/// same engine in the same process. This measures CPU work, not GPU/input latency.
fn region_benchmark(width: usize, height: usize, reps: usize, spatial: bool) -> Result<Value, Box<dyn std::error::Error>> {
    use photocraft_algo::camera_raw::CameraRaw;
    use photocraft_color::PixelFormat;
    use photocraft_geom::Rect;
    use photocraft_raster::Surface;
    let area = Rect::new(0, 0, width as i32, height as i32);
    let crop = Rect::new(width as i32 / 2 - 400, height as i32 / 2 - 300, width as i32 / 2 + 400, height as i32 / 2 + 300);
    let mut source = Surface::new(PixelFormat::RGBA8);
    let mut row = vec![0.0; width * 4];
    for y in 0..height {
        for (x, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            pixel.copy_from_slice(&[(x % 256) as f32 / 255.0, (y % 256) as f32 / 255.0, ((x ^ y) % 256) as f32 / 255.0, 1.0]);
        }
        source.write_region(Rect::new(0, y as i32, width as i32, y as i32 + 1), &row);
    }
    let mut full_ms = Vec::new();
    let mut region_ms = Vec::new();
    let mut max_error = 0.0f32;
    let mut different_samples = 0usize;
    let mut work_size = [800, 600];
    for i in 0..reps {
        let mut params = CameraRaw { exposure: 0.5 + i as f32 * 0.1, contrast: 20.0, temperature: -15.0, saturation: 10.0, ..Default::default() };
        if spatial {
            params.noise_luminance = 70.0;
            params.noise_color = 80.0;
            params.sharpen_amount = 100.0 + 10.0 * i as f32;
            params.sharpen_radius = 3.0;
            params.sharpen_masking = 70.0;
        }
        let halo = photocraft_algo::camera_raw::preview_halo(&params).ok_or("benchmark settings need global domain")? as i32;
        let work = Rect::new(crop.x0 - halo, crop.y0 - halo, crop.x1 + halo, crop.y1 + halo).intersect(&area);
        work_size = [work.width(), work.height()];
        let start = std::time::Instant::now();
        let full = photocraft_engine::lens_cmds::camera_raw_surface(&source, area, &params);
        full_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        let start = std::time::Instant::now();
        let region = photocraft_engine::lens_cmds::camera_raw_surface(&source, work, &params);
        region_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        for (a, b) in full.read_region(crop).iter().zip(region.read_region(crop)) {
            max_error = max_error.max((a - b).abs());
            different_samples += usize::from(*a != b);
        }
        if max_error > if spatial { 1.0 / 255.0 + 1e-7 } else { 0.0 } {
            return Err(format!("visible-region pixels differ beyond depth rounding: {max_error}").into());
        }
    }
    Ok(json!({"source":[width,height],"visible":[800,600],"work_size":work_size,
        "settings":if spatial { "exposure, contrast, temperature, saturation, luminance/color denoise, sharpen + masking" } else { "exposure, contrast, temperature, saturation" },
        "full_engine_ms":full_ms,"visible_region_engine_ms":region_ms,"exact_pixel_match":different_samples == 0,
        "max_sample_error":max_error,"different_samples":different_samples,"measurement":"Same-process full-domain vs halo crop CPU develop; same source pixels and kernels; no GPU/input latency"}))
}
