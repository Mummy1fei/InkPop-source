//! Render the native client clipping boundary over actual opaque backgrounds.
//! This detects the clipped popup shadow that a merely alpha<255 corner check missed.
use egui::{Color32, Pos2, Rect, Vec2};

const BACKGROUNDS: [Color32; 3] = [Color32::from_rgb(20, 20, 20), Color32::from_rgb(252, 252, 253), Color32::from_rgb(252, 210, 0)];

fn client_rect(theme: usize, kind: usize, background: usize, fixed: bool) -> Rect {
    Rect::from_min_size(
        Pos2::new(theme as f32 * 300.0 + if fixed { 164.0 } else { 14.0 }, (kind * 3 + background) as f32 * 100.0 + 10.0),
        Vec2::new(120.0, 80.0),
    )
}

#[test]
fn menu_corner_pixels_preserve_the_underlying_panel_in_all_themes_and_scales() {
    for scale in [1.0, 2.0] {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(Vec2::new(1500.0, 600.0))
            .with_pixels_per_point(scale)
            .wgpu_setup(crate::gpu_canvas::wgpu_setup())
            .build_ui(|ui| {
                for (theme_index, theme) in crate::theme::ThemeKind::ALL.into_iter().enumerate() {
                    crate::theme::apply(ui.ctx(), theme);
                    let style = ui.ctx().style_of(ui.ctx().theme());
                    for kind in 0..2 {
                        for (background_index, background) in BACKGROUNDS.into_iter().enumerate() {
                            let tile = Rect::from_min_size(
                                Pos2::new(theme_index as f32 * 300.0, (kind * 3 + background_index) as f32 * 100.0),
                                Vec2::new(300.0, 100.0),
                            );
                            ui.painter().rect_filled(tile, 0, background);
                            for fixed in [false, true] {
                                let frame = match (kind, fixed) {
                                    (0, false) => egui::Frame::popup(&style).inner_margin(6.0),
                                    (0, true) => crate::native_menu_ui::menu_frame(&style),
                                    (_, false) => egui::Frame::menu(&style),
                                    (_, true) => crate::native_popup_ui::menu_frame(&style),
                                };
                                let client = client_rect(theme_index, kind, background_index, fixed);
                                ui.painter().with_clip_rect(client).add(frame.paint(client - frame.total_margin()));
                            }
                        }
                    }
                }
            });
        harness.run_steps(3);
        let image = harness.render().expect("native-frame GPU render");
        let mut legacy_defects = 0;
        for (theme_index, theme) in crate::theme::ThemeKind::ALL.into_iter().enumerate() {
            if theme == crate::theme::ThemeKind::Classic {
                continue; // Classic deliberately fills a square, not a rounded hole.
            }
            for kind in 0..2 {
                for (background_index, background) in BACKGROUNDS.into_iter().enumerate() {
                    for fixed in [false, true] {
                        let r = client_rect(theme_index, kind, background_index, fixed);
                        let x0 = (r.left() * scale) as u32;
                        let y0 = (r.top() * scale) as u32;
                        let x1 = (r.right() * scale) as u32 - 1;
                        let y1 = (r.bottom() * scale) as u32 - 1;
                        for (x, y) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
                            let pixel = image.get_pixel(x, y).0;
                            let error = pixel.into_iter().zip(background.to_array()).map(|(actual, expected)| actual.abs_diff(expected)).max().unwrap_or(0);
                            if fixed {
                                assert!(error <= 2, "{theme:?} {scale}x kind={kind} bg={background_index} at=({x},{y}) {pixel:?} != {background:?}");
                            } else if error > 2 {
                                legacy_defects += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(legacy_defects > 0, "the original clipped shadow must fail this pixel oracle");
        if let Some(out) = std::env::var_os("PHOTOCRAFT_QA_MENU_PIXELS_OUT") {
            let out = std::path::PathBuf::from(out);
            std::fs::create_dir_all(&out).expect("pixel evidence directory");
            image.save(out.join(format!("native-menu-overlap-{scale}x.png"))).expect("pixel evidence");
        }
    }
}
