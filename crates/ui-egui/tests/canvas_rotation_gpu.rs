//! Exercise the actual tiled GPU callback, independently of the shell and input handling.
use photocraft_ui_egui::gpu_canvas::{GpuCanvas, ViewParams};

struct RotationCanvas {
    gpu: GpuCanvas,
    params: ViewParams,
}

impl eframe::App for RotationCanvas {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0, egui::Color32::from_gray(48));
        GpuCanvas::paint(ui.painter(), rect, self.params);
    }
}

#[test]
fn rotated_tiled_canvas_preserves_pixels_checker_and_reset() {
    let adapter_ready = std::sync::atomic::AtomicBool::new(false);
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // Exercise the application's adapter/backend selection. Harness::wgpu() instead
        // deliberately prefers a software rasterizer, which is not the native app's path.
        egui_kittest::Harness::builder()
            .with_size(egui::vec2(256.0, 256.0))
            .with_pixels_per_point(1.0)
            .wgpu_setup(photocraft_ui_egui::gpu_canvas::wgpu_setup())
            .build_eframe(|cc| {
                let rs = cc.wgpu_render_state.as_ref().expect("wgpu renderer");
                adapter_ready.store(true, std::sync::atomic::Ordering::Relaxed);
                let gpu = GpuCanvas::with_tile(rs, Some(64));
                eprintln!("canvas rotation test adapter: {:?}", gpu.adapter_info());
                let mut rgba = Vec::with_capacity(128 * 128 * 4);
                for y in 0..128 {
                    for x in 0..128 {
                        let color = match (x < 64, y < 64) {
                            (true, true) => [240, 32, 16, 255],
                            (false, true) => [32, 224, 48, 255],
                            (true, false) => [16, 48, 240, 255],
                            (false, false) => [0, 0, 0, 0],
                        };
                        rgba.extend_from_slice(&color);
                    }
                }
                gpu.upload_full(1, [128, 128], &rgba);
                RotationCanvas {
                    gpu,
                    params: ViewParams {
                        doc: 1,
                        doc_size: [128, 128],
                        zoom: 1.0,
                        center: [64.0, 64.0],
                        rotation: 0.0,
                        shadow: true,
                        pixel_grid: true,
                        view_key: 1,
                        display: 0,
                        output: 0,
                        hdr: None,
                    },
                }
            })
    }));
    let mut harness = match built {
        Ok(harness) => harness,
        Err(error) => {
            if adapter_ready.load(std::sync::atomic::Ordering::Relaxed) {
                std::panic::resume_unwind(error);
            }
            eprintln!("skipping: no GPU adapter");
            return;
        }
    };
    harness.run_steps(2);
    let original = harness.render().expect("unrotated render");
    for degrees in [0.0_f32, 23.0, 90.0, -90.0, 179.0, 0.0] {
        let rotation = degrees.to_radians();
        harness.state_mut().params.rotation = degrees;
        harness.run_steps(2);
        let image = harness.render().expect("rotated render");
        assert!(harness.state().gpu.fault().is_none(), "GPU validation/device fault");
        let (s, c) = rotation.sin_cos();
        let mut samples = 0;
        for y in (8..248).step_by(5) {
            for x in (8..248).step_by(5) {
                let dx = x as f32 + 0.5 - 128.0;
                let dy = y as f32 + 0.5 - 128.0;
                let u = c * dx + s * dy + 64.0;
                let v = -s * dx + c * dy + 64.0;
                // Avoid interpolation along tile/document boundaries. The sample locations are
                // computed from the independent inverse transform, never from shader output.
                if !(3.0..125.0).contains(&u) || !(3.0..125.0).contains(&v) || (u - 64.0).abs() < 3.0 || (v - 64.0).abs() < 3.0 {
                    continue;
                }
                let expected = match (u < 64.0, v < 64.0) {
                    (true, true) => [240_u8, 32, 16],
                    (false, true) => [32, 224, 48],
                    (true, false) => [16, 48, 240],
                    (false, false) => {
                        if (x / 8 + y / 8) % 2 == 0 {
                            [255; 3]
                        } else {
                            [204; 3]
                        }
                    }
                };
                let pixel = image.get_pixel(x, y);
                assert!(
                    (0..3).all(|channel| pixel[channel].abs_diff(expected[channel]) <= 2),
                    "{degrees} degrees at ({x},{y}), doc ({u},{v}): {pixel:?} != {expected:?}"
                );
                samples += 1;
            }
        }
        assert!(samples > 300, "sufficient samples across all four tiles");
        if degrees == 0.0 {
            assert_eq!(original.as_raw(), image.as_raw(), "reset must restore every rendered pixel, including shadow");
        }
    }
}
