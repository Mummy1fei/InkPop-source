//! Compact Color panel and persisted display modes. Tool colours remain encoded sRGB.
use crate::{
    PhotocraftApp,
    color_picker_ui::{hsv_to_rgb, rgb_to_hsv},
    theme::Tokens,
    widgets,
};
use egui::{Color32, Sense, Stroke, pos2, vec2};
use photocraft_color::convert;
use serde::{Deserialize, Serialize};

#[path = "color_swatches_ui.rs"]
pub mod swatches;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    HueCube,
    BrightnessCube,
    #[default]
    Wheel,
    Gray,
    Rgb,
    Hsb,
    Cmyk,
    Lab,
    Web,
}
impl Mode {
    const ALL: [(Self, &'static str); 9] = [
        (Self::HueCube, "Hue Cube"),
        (Self::BrightnessCube, "Brightness Cube"),
        (Self::Wheel, "Color Wheel"),
        (Self::Gray, "Grayscale Slider"),
        (Self::Rgb, "RGB Sliders"),
        (Self::Hsb, "HSB Sliders"),
        (Self::Cmyk, "CMYK Sliders"),
        (Self::Lab, "Lab Sliders"),
        (Self::Web, "Web Color Sliders"),
    ];
}
pub(crate) fn menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    for (mode, label) in Mode::ALL {
        if ui.selectable_label(app.ui.color_panel.mode == mode, tl!(label)).clicked() {
            app.ui.color_panel.mode = mode;
            ui.close();
        }
    }
    ui.separator();
    let c = if app.ui.color_panel.background { app.session.tools.background } else { app.session.tools.foreground };
    let [r, g, b] = crate::panels::srgb_bytes(c);
    let hex = format!("#{r:02X}{g:02X}{b:02X}");
    if ui.button(tl!("Copy Color as HTML")).clicked() {
        ui.ctx().copy_text(format!("<span style=\"color: {hex}\">{hex}</span>"));
        ui.close();
    }
    if ui.button(tl!("Copy Color's Hex Code")).clicked() {
        ui.ctx().copy_text(hex);
        ui.close();
    }
}
fn bytes(rgb: [f32; 3]) -> Color32 {
    let [r, g, b] = rgb.map(|x| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
    Color32::from_rgb(r, g, b)
}
fn row(ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, unit: &str, stops: &[Color32]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let short = match label {
            "Hue" => "H",
            "Saturation" => "S",
            "Brightness" => "B",
            _ => label,
        };
        ui.label(egui::RichText::new(short).color(Tokens::get(ui.ctx()).text_dim));
        let width = (ui.available_width() - 66.0).max(60.0);
        ui.allocate_ui_with_layout(vec2(width, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let r = widgets::slider(ui, value, range.clone(), Some(stops));
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, label));
            changed |= r.changed();
        });
        changed |= widgets::value_field(ui, value, range, unit, 58.0).changed();
    });
    changed
}
fn hsb(ui: &mut egui::Ui, hsv: &mut [f32; 3]) -> bool {
    let mut changed = row(ui, "Hue", &mut hsv[0], 0.0..=360.0, "°", &widgets::hue_stops());
    let sat = [bytes(hsv_to_rgb(hsv[0], 0.0, hsv[2].max(0.2))), bytes(hsv_to_rgb(hsv[0], 1.0, hsv[2].max(0.2)))];
    let mut s = hsv[1] * 100.0;
    changed |= row(ui, "Saturation", &mut s, 0.0..=100.0, "%", &sat);
    hsv[1] = s / 100.0;
    let mut v = hsv[2] * 100.0;
    changed |= row(ui, "Brightness", &mut v, 0.0..=100.0, "%", &[Color32::BLACK, bytes(hsv_to_rgb(hsv[0], hsv[1], 1.0))]);
    hsv[2] = v / 100.0;
    changed
}
fn component_rgb(mode: Mode, v: [f32; 4]) -> [f32; 3] {
    match mode {
        Mode::Gray => [1.0 - v[0] / 100.0; 3],
        Mode::Cmyk => convert::cmyk_to_rgb(v.map(|x| x / 100.0)),
        Mode::Lab => convert::lab_to_srgb([v[0], v[1], v[2]]),
        _ => [v[0] / 255.0, v[1] / 255.0, v[2] / 255.0],
    }
}
fn channels(ui: &mut egui::Ui, mode: Mode, rgb: &mut [f32; 3]) -> bool {
    type ChannelSpec = (&'static str, f32, f32, &'static str);
    let (mut values, specs): ([f32; 4], &[ChannelSpec]) = match mode {
        Mode::Gray => ([100.0 * (1.0 - convert::rgb_to_gray(*rgb)), 0.0, 0.0, 0.0], &[("K", 0.0, 100.0, "%")]),
        Mode::Cmyk => {
            (convert::rgb_to_cmyk(*rgb).map(|x| x * 100.0), &[("C", 0.0, 100.0, "%"), ("M", 0.0, 100.0, "%"), ("Y", 0.0, 100.0, "%"), ("K", 0.0, 100.0, "%")])
        }
        Mode::Lab => {
            let [l, a, b] = convert::srgb_to_lab(*rgb);
            ([l, a, b, 0.0], &[("L", 0.0, 100.0, ""), ("a", -128.0, 127.0, ""), ("b", -128.0, 127.0, "")])
        }
        _ => ([rgb[0] * 255.0, rgb[1] * 255.0, rgb[2] * 255.0, 0.0], &[("R", 0.0, 255.0, ""), ("G", 0.0, 255.0, ""), ("B", 0.0, 255.0, "")]),
    };
    let mut changed = false;
    for (i, &(label, low, high, unit)) in specs.iter().enumerate() {
        let mut stops = Vec::with_capacity(9);
        for n in 0..=8 {
            let mut sample = values;
            if let Some(x) = sample.get_mut(i) {
                *x = low + (high - low) * n as f32 / 8.0;
            }
            stops.push(bytes(component_rgb(mode, sample)));
        }
        if let Some(value) = values.get_mut(i) {
            let edit = row(ui, label, value, low..=high, unit, &stops);
            if edit && mode == Mode::Web {
                *value = (*value / 51.0).round() * 51.0;
            }
            changed |= edit;
        }
    }
    if changed {
        *rgb = component_rgb(mode, values);
    }
    changed
}
fn cube(ui: &mut egui::Ui, hsv: &mut [f32; 3], hue_cube: bool) -> bool {
    let size = vec2((ui.available_width() - 25.0).max(100.0), ui.available_height().clamp(120.0, 240.0));
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let mut changed = false;
    if (response.clicked() || response.dragged())
        && let Some(p) = response.interact_pointer_pos()
    {
        let x = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let y = ((p.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        if hue_cube {
            hsv[1] = x;
            hsv[2] = 1.0 - y;
        } else {
            hsv[0] = x * 360.0;
            hsv[1] = 1.0 - y;
        }
        changed = true;
    }
    let color = |x: f32, y: f32| if hue_cube { bytes(hsv_to_rgb(hsv[0], x, 1.0 - y)) } else { bytes(hsv_to_rgb(x * 360.0, 1.0 - y, hsv[2])) };
    let mut mesh = egui::Mesh::default();
    for n in 0..36 {
        let x = n as f32 / 36.0;
        let x2 = (n + 1) as f32 / 36.0;
        let base = mesh.vertices.len() as u32;
        for (x, y) in [(x, 0.0), (x2, 0.0), (x2, 1.0), (x, 1.0)] {
            mesh.colored_vertex(pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height()), color(x, y));
        }
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    ui.painter().add(mesh);
    let p = pos2(
        rect.left() + if hue_cube { hsv[1] } else { hsv[0] / 360.0 } * rect.width(),
        rect.top() + (1.0 - if hue_cube { hsv[2] } else { hsv[1] }) * rect.height(),
    );
    ui.painter().circle_stroke(p, 4.0, Stroke::new(2.5, Color32::BLACK));
    ui.painter().circle_stroke(p, 4.0, Stroke::new(1.0, Color32::WHITE));
    changed
}
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let space = app.session.active().and_then(|st| convert::CmykSpace::for_profile(st.doc.icc_profile.as_ref()));
    convert::with_cmyk_space(space.as_ref(), || show_in_space(app, ui));
}
fn show_in_space(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let background = app.ui.color_panel.background;
    let color = if background { app.session.tools.background } else { app.session.tools.foreground };
    let mut rgb = [color[0], color[1], color[2]];
    let key = egui::Id::new(("color-panel-hsv", background));
    let stored: Option<([f32; 3], [f32; 3])> = ui.data(|d| d.get_temp(key));
    let mut hsv = stored.filter(|(_, c)| *c == rgb).map_or_else(|| rgb_to_hsv(rgb), |(h, _)| h);
    let mode = app.ui.color_panel.mode;
    let mut changed = false;
    ui.horizontal_top(|ui| {
        let (chips, _) = ui.allocate_exact_size(vec2(38.0, 40.0), Sense::hover());
        crate::panels::field_chips(app, ui, chips);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if matches!(mode, Mode::Wheel | Mode::HueCube | Mode::BrightnessCube | Mode::Hsb) {
                changed |= hsb(ui, &mut hsv);
                if changed {
                    rgb = hsv_to_rgb(hsv[0], hsv[1], hsv[2]);
                }
            } else {
                changed |= channels(ui, mode, &mut rgb);
                if changed {
                    hsv = rgb_to_hsv(rgb);
                }
            }
        });
    });
    ui.add_space(4.0);
    let visual_changed = match mode {
        Mode::Wheel => {
            let side = ui.available_width().clamp(120.0, 240.0);
            let offset = ((ui.available_width() - side) / 2.0).max(0.0);
            let mut wheel = egui::ecolor::Hsva::new(hsv[0] / 360.0, hsv[1], hsv[2], 1.0);
            let mut edit = false;
            ui.horizontal(|ui| {
                ui.add_space(offset);
                edit = crate::color_wheel_ui::show(ui, &mut wheel);
            });
            if edit {
                hsv = [wheel.h * 360.0, wheel.s, wheel.v];
            }
            edit
        }
        Mode::HueCube | Mode::BrightnessCube => {
            let mut edit = false;
            ui.horizontal(|ui| {
                edit = cube(ui, &mut hsv, mode == Mode::HueCube);
                let mut channel = if mode == Mode::HueCube { hsv[0] } else { hsv[2] * 100.0 };
                edit |= ui.add(egui::Slider::new(&mut channel, 0.0..=if mode == Mode::HueCube { 360.0 } else { 100.0 }).vertical().show_value(false)).changed();
                if mode == Mode::HueCube {
                    hsv[0] = channel;
                } else {
                    hsv[2] = channel / 100.0;
                }
            });
            edit
        }
        _ => false,
    };
    if visual_changed {
        rgb = hsv_to_rgb(hsv[0], hsv[1], hsv[2]);
        changed = true;
    }
    if changed {
        let value = [rgb[0], rgb[1], rgb[2], color[3]];
        if let Err(e) = app.run("tools.setColors", if background { serde_json::json!({"background": value}) } else { serde_json::json!({"foreground": value}) })
        {
            app.ui.status = e;
        } else {
            ui.data_mut(|d| d.insert_temp(key, (hsv, rgb)));
            if !background {
                crate::type_tool::foreground_changed(app);
            }
        }
    }
    let [r, g, b] = crate::panels::srgb_bytes([rgb[0], rgb[1], rgb[2], 1.0]);
    ui.label(egui::RichText::new(format!("#{r:02X}{g:02X}{b:02X}")).small().color(Tokens::get(ui.ctx()).text_faint));
    swatches::show(app, ui);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    #[test]
    fn modes_are_idle_safe_and_rgb_slider_edits_the_selected_background() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("tools.setColors", serde_json::json!({"foreground":"#d8452e","background":"#204060"})).unwrap();
        let fg = app.session.tools.foreground;
        let bg = app.session.tools.background;
        app.ui.color_panel.background = true;
        let mut h = egui_kittest::Harness::builder().with_size(vec2(320., 420.)).build_ui_state(|ui, app: &mut PhotocraftApp| show(app, ui), app);
        for (mode, _) in Mode::ALL {
            h.state_mut().ui.color_panel.mode = mode;
            h.run_steps(3);
            assert_eq!(h.state().session.tools.foreground, fg);
            assert_eq!(h.state().session.tools.background, bg);
        }
        h.state_mut().ui.color_panel.mode = Mode::Rgb;
        h.run_steps(2);
        h.query_all_by_label("R").find(|n| n.rect().width() > 60.0).unwrap().click();
        h.run_steps(2);
        assert_eq!(h.state().session.tools.foreground, fg);
        assert_ne!(h.state().session.tools.background, bg);
        h.state_mut().ui.color_panel.mode = Mode::Wheel;
        h.run_steps(2);
        let slider = h.get_by_label("Hue").rect();
        let wheel = h.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new("color-wheel-rect"))).unwrap();
        assert!(slider.bottom() < wheel.top());
    }
    #[test]
    fn display_modes_persist_and_old_state_defaults_to_wheel() {
        let old: crate::state::ColorPanelState = serde_json::from_str("{\"background\":true}").unwrap();
        assert_eq!(old.mode, Mode::Wheel);
        for (mode, _) in Mode::ALL {
            let state = crate::state::ColorPanelState { mode, background: true };
            assert_eq!(serde_json::from_value::<crate::state::ColorPanelState>(serde_json::to_value(state).unwrap()).unwrap(), state);
        }
    }
    #[test]
    fn gray_rgb_lab_and_cmyk_channels_produce_valid_colors() {
        assert_eq!(component_rgb(Mode::Gray, [0.0; 4]), [1.0; 3]);
        assert_eq!(component_rgb(Mode::Gray, [100.0; 4]), [0.0; 3]);
        assert_eq!(component_rgb(Mode::Rgb, [255.0, 0.0, 51.0, 0.0]), [1.0, 0.0, 0.2]);
        let rgb = [0.7, 0.2, 0.5];
        let [l, a, b] = convert::srgb_to_lab(rgb);
        let restored = component_rgb(Mode::Lab, [l, a, b, 0.0]);
        for (x, y) in rgb.into_iter().zip(restored) {
            assert!((x - y).abs() < 0.001);
        }
        assert!(component_rgb(Mode::Cmyk, [20.0, 80.0, 40.0, 10.0]).into_iter().all(|v| v.is_finite() && (0.0..=1.0).contains(&v)));
    }
}
