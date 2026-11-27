//! Actual proxy-worker publication, independent of rendering/texture magnification.
use super::*;
use serde_json::json;

fn held_input(time: f64, press: bool) -> egui::RawInput {
    egui::RawInput {
        time: Some(time),
        events: if press {
            vec![egui::Event::PointerButton { pos: pos2(-10.0, -10.0), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE }]
        } else {
            Vec::new()
        },
        ..Default::default()
    }
}

fn await_publication(dialog: &mut CameraRawDialog, ctx: &egui::Context) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while dialog.preview_revision != dialog.requested_revision || dialog.proxy_pending.is_some() || dialog.proxy_requested {
        assert!(std::time::Instant::now() < deadline, "worker must publish while held without a release event");
        let time = ctx.input(|input| input.time) + 0.01;
        let mut output = ctx.run_ui(held_input(time, false), |ui| dialog.poll_render(ui.ctx()));
        output.textures_delta.clear();
        assert!(ctx.input(|input| input.pointer.any_down()), "publication must occur before the held gesture ends");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn held_worker_keeps_fine_display_pixels_in_spatial_and_pointwise_revisions_at_one_and_two_dpi() {
    for (width, height) in [(1046, 698), (2092, 1395)] {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":64,"height":48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color":"#506080"})).unwrap();
        open(&mut app, &ctx).unwrap();
        let d = app.camera_raw.as_mut().unwrap();
        d.pw = width;
        d.ph = height;
        d.full_w = width;
        d.full_h = height;
        d.proxy = (0..width * height)
            .map(|i| {
                let v = if i % width % 2 == 0 { 0.15 } else { 0.65 };
                [v, v * 0.5, v * 0.25, 0.75]
            })
            .collect::<Vec<_>>()
            .into();
        d.processed = d.proxy.clone();
        d.live_source = live::Source::new(d.proxy.clone(), width, height);
        d.selection = Some((0..width * height).map(|i| if i % width < width / 2 { 0.0 } else { 0.5 }).collect());
        for step in 0..2 {
            d.params = CameraRaw { exposure: 0.1 + step as f32 * 0.1, clarity: 5.0, ..Default::default() };
            let time = ctx.input(|input| input.time) + 0.01;
            let mut output = ctx.run_ui(held_input(time, step == 0), |ui| d.queue_render(ui.ctx()));
            output.textures_delta.clear();
            await_publication(d, &ctx);
            assert!(d.proxy_reduced, "spatial work remains reduced while held");
            let mut low = d.live_source.pixels.to_vec();
            let mut params = d.params.clone();
            params.pixel_scale = d.live_source.width as f32 / width as f32;
            photocraft_algo::camera_raw::develop(&mut low, d.live_source.width, d.live_source.height, &params, d.float);
            let mut expected = d.live_source.expand_preserving_detail(&low, &d.proxy, width, height);
            for ((pixel, original), coverage) in expected.iter_mut().zip(d.proxy.iter()).zip(d.selection.as_ref().unwrap().iter()) {
                for value in &mut pixel[..3] {
                    *value = value.clamp(0.0, 1.0);
                }
                for (value, before) in pixel.iter_mut().zip(original) {
                    *value = *before + (*value - *before) * coverage;
                }
            }
            assert_eq!(d.processed.as_ref(), expected.as_slice(), "the published frame itself must retain detail");
            let fine_difference = d.processed.chunks(width).map(|row| (row[width - 2][0] - row[width - 3][0]).abs()).sum::<f32>() / height as f32;
            assert!(fine_difference > 0.4, "fine stripes must survive the actual spatial worker publication");
            assert_eq!(d.after_histogram, RgbHistogram::from_rgba(&expected));
        }
        // A pointwise change now processes the complete original display grid even
        // before release, rather than upscaling a second, quarter-area image.
        d.selection = None;
        d.params = CameraRaw { exposure: 0.25, contrast: 5.0, ..Default::default() };
        d.queue_render(&ctx);
        await_publication(d, &ctx);
        assert!(!d.proxy_reduced);
        let mut expected = d.proxy.to_vec();
        photocraft_algo::camera_raw::develop(&mut expected, width, height, &d.params, d.float);
        assert_eq!(d.processed.as_ref(), expected.as_slice());
        assert_eq!(d.rendered_params, d.params);
    }
}

#[test]
#[ignore = "release-only CPU quality/performance comparison; main task runs serially"]
fn camera_raw_quality_cpu_benchmark_v57() {
    let median = |values: &mut Vec<f64>| {
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    };
    for (width, height) in [(1046, 698), (2092, 1395)] {
        let original: std::sync::Arc<[[f32; 4]]> = (0..width * height)
            .map(|i| {
                let x = i % width;
                let y = i / width;
                let fine = if (x + y) % 2 == 0 { 0.1 } else { -0.1 };
                let base = 0.2 + x as f32 / width as f32 * 0.4;
                [base + fine, base * 0.8 + fine, base * 0.5 + fine, 0.75]
            })
            .collect::<Vec<_>>()
            .into();
        let reduced = live::Source::new(original.clone(), width, height);
        for (name, params) in [
            ("pointwise", CameraRaw { exposure: 0.5, temperature: 15.0, contrast: 20.0, ..Default::default() }),
            ("spatial", CameraRaw { exposure: 0.5, clarity: 25.0, texture: 20.0, highlights: -20.0, shadows: 20.0, dehaze: 10.0, ..Default::default() }),
        ] {
            let process = |new: bool| {
                let start = std::time::Instant::now();
                let use_reduced = !new || live::should_reduce(&params);
                let (work_width, work_height) = if use_reduced { (reduced.width, reduced.height) } else { (width, height) };
                let mut pixels = if use_reduced { reduced.pixels.to_vec() } else { original.to_vec() };
                let mut p = params.clone();
                p.pixel_scale = work_width as f32 / 6000.0;
                photocraft_algo::camera_raw::develop(&mut pixels, work_width, work_height, &p, false);
                if use_reduced {
                    pixels = if new { reduced.expand_preserving_detail(&pixels, &original, width, height) } else { reduced.expand(&pixels, width, height) };
                    for (pixel, before) in pixels.iter_mut().zip(original.iter()) {
                        pixel[3] = before[3];
                        for value in &mut pixel[..3] {
                            *value = value.clamp(0.0, 1.0);
                        }
                    }
                }
                let histogram = frame::histogram(&pixels);
                let image = CameraRawDialog::image(&pixels, width, height);
                std::hint::black_box((&histogram, &image));
                (start.elapsed().as_secs_f64() * 1000.0, pixels)
            };
            let mut exact = original.to_vec();
            let mut p = params.clone();
            p.pixel_scale = width as f32 / 6000.0;
            photocraft_algo::camera_raw::develop(&mut exact, width, height, &p, false);
            let (_, old_pixels) = process(false);
            let (_, new_pixels) = process(true);
            if name == "pointwise" {
                assert_eq!(new_pixels, exact);
            }
            let rmse = |pixels: &[[f32; 4]]| {
                (pixels.iter().zip(&exact).map(|(a, b)| (0..3).map(|c| ((a[c] - b[c]) as f64).powi(2)).sum::<f64>()).sum::<f64>() / (pixels.len() * 3) as f64)
                    .sqrt()
            };
            let contrast = |pixels: &[[f32; 4]]| {
                pixels.chunks(width).map(|row| row.windows(2).map(|w| (w[0][0] - w[1][0]).abs() as f64).sum::<f64>()).sum::<f64>()
                    / (height * (width - 1)) as f64
            };
            let mut old_ms = Vec::new();
            let mut new_ms = Vec::new();
            for repeat in 0..6 {
                for new in if repeat % 2 == 0 { [false, true] } else { [true, false] } {
                    let (ms, pixels) = process(new);
                    std::hint::black_box(pixels);
                    if new {
                        new_ms.push(ms);
                    } else {
                        old_ms.push(ms);
                    }
                }
            }
            let old_median = median(&mut old_ms);
            let new_median = median(&mut new_ms);
            println!(
                "CAMERA_RAW_V57_QUALITY_CPU {}",
                json!({"display":[width,height],"sourceSizeAssumed":[6000,4000],"case":name,"oldMedianMs":old_median,"newMedianMs":new_median,"oldSamplesMs":old_ms,"newSamplesMs":new_ms,"oldRmse":rmse(&old_pixels),"newRmse":rmse(&new_pixels),"exactNeighborContrast":contrast(&exact),"oldNeighborContrast":contrast(&old_pixels),"newNeighborContrast":contrast(&new_pixels),"excludes":"24MP source creation, UI upload/presentation, native detail and physical pen latency"})
            );
        }
    }
}
