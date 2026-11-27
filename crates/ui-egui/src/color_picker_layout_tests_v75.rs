//! Compact production picker layout, real field input and genuine small-window scrolling.
use super::*;
use crate::theme::{ThemeKind, palette::ScrollAreaPaletteExt};
use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable};

struct Fixture {
    fields: Map<String, Value>,
    content: Rect,
    clip: Rect,
    offset: egui::Vec2,
    content_size: egui::Vec2,
}

impl eframe::App for Fixture {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        // Match the native dialog's fixed footer, 16pt frame, two-axis scroll
        // and production body. This is a body harness, not a claim about HWNDs.
        egui::Panel::bottom("picker-layout-actions-v75").exact_size(64.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(tl!("OK"));
                ui.label(tl!("Cancel"));
            });
        });
        egui::CentralPanel::default().frame(egui::Frame::central_panel(ui.style()).inner_margin(16.0)).show(ui, |ui| {
            let output = egui::ScrollArea::both().id_salt("picker-layout-v75").auto_shrink([false, false]).show_palette(ui, |ui| {
                self.clip = ui.clip_rect();
                let response = ui.scope(|ui| body(ui, &mut self.fields));
                self.content = response.response.rect;
            });
            self.offset = output.state.offset;
            self.content_size = output.content_size;
        });
    }
}

fn fixture(theme: ThemeKind, size: egui::Vec2, alpha: bool, none: bool, gpu: bool) -> Harness<'static, Fixture> {
    let builder = Harness::builder().with_size(size).with_max_steps(64).with_step_dt(1.0 / 60.0);
    let builder = if gpu { builder.wgpu_setup(crate::gpu_canvas::wgpu_setup()) } else { builder };
    builder.build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        let mut fields = Map::new();
        fields.insert("__colorPicker".into(), json!("widget"));
        fields.insert("color".into(), json!("#cc4422"));
        fields.insert("__orig".into(), json!("#336699"));
        fields.insert("__allowAlpha".into(), json!(alpha));
        fields.insert("__alpha".into(), json!(0.45));
        fields.insert("__originalAlpha".into(), json!(0.8));
        fields.insert("__allowNone".into(), json!(none));
        Fixture { fields, content: Rect::NOTHING, clip: Rect::NOTHING, offset: egui::Vec2::ZERO, content_size: egui::Vec2::ZERO }
    })
}

fn geometry(h: &Harness<'_, Fixture>, extra_rows: usize) {
    let state = h.state();
    assert!(state.content.width() <= 530.0, "compact body width {:?}", state.content);
    assert!(state.content.height() <= 264.0 + extra_rows as f32 * 30.0, "compact body height {:?}", state.content);
    assert!(state.clip.expand(0.51).contains_rect(state.content), "all controls fit without hiding overflow: {:?} clip {:?}", state.content, state.clip);
    assert_eq!(h.query_all_by_role(Role::SpinButton).count(), 13 + usize::from(state.fields["__allowAlpha"] == true));
    let hex = h.get_by_role(Role::TextInput);
    assert!(state.clip.expand(0.51).contains_rect(hex.rect()), "HEX stays visible");
    assert_eq!(hex.value().unwrap().len(), 6);
    for node in h.query_all_by_role(Role::SpinButton) {
        assert!(state.clip.expand(0.51).contains_rect(node.rect()), "component {:?} clipped", node.rect());
    }
}

#[test]
fn compact_picker_all_components_fit_five_themes_without_clipping_v75() {
    for theme in ThemeKind::ALL {
        for (alpha, none, height) in [(false, false, 368.0), (true, false, 396.0), (false, true, 396.0), (true, true, 424.0)] {
            let mut h = fixture(theme, vec2(560.0, height), alpha, none, false);
            h.run_steps(6);
            assert_eq!(Tokens::get(&h.ctx).kind, theme, "the requested theme is actually active");
            geometry(&h, usize::from(alpha) + usize::from(none));
            assert_eq!(h.state().fields["color"], json!("#cc4422"), "layout does not quantize color");
            if none {
                assert!(h.state().clip.contains_rect(h.get_by_label(tl!("No Color")).rect()));
            }
        }
    }
}

#[test]
fn compact_picker_respects_reserved_scrollbar_gutter_without_clipping_fields_v78() {
    for theme in ThemeKind::ALL {
        for width in [540.0, 560.0, 576.0] {
            let mut h = fixture(theme, vec2(width, 424.0), true, true, false);
            h.run_steps(6);
            geometry(&h, 2);
            assert!(
                h.state().content.width() <= h.state().clip.width() + 0.01,
                "production body must adapt to the actual client width at {width}pt, including any reserved gutter"
            );
            assert_eq!(h.state().offset, egui::Vec2::ZERO, "fitting controls need no initial scroll offset");
            assert_eq!(h.state().fields["color"], json!("#cc4422"));
        }
    }
}

#[test]
fn compact_picker_keyboard_hex_rgb_hsb_lab_and_cmyk_still_use_production_conversions_v75() {
    for (field, typed) in [(0, "120"), (3, "85"), (6, "60"), (7, "30"), (9, "40")] {
        let mut h = fixture(ThemeKind::StudioLight, vec2(560.0, 368.0), false, false, false);
        h.run_steps(4);
        let rgb = parse_hex("#cc4422").unwrap();
        let mut hsv = rgb_to_hsv(rgb);
        let mut lab = photocraft_color::convert::srgb_to_lab(rgb);
        let mut cmyk = photocraft_color::convert::rgb_to_cmyk(rgb);
        let expected = match field {
            0 => {
                hsv[0] = 120.0;
                hsv_to_rgb(hsv[0], hsv[1], hsv[2])
            }
            3 => [85.0 / 255.0, rgb[1], rgb[2]],
            6 => {
                lab[0] = 60.0;
                photocraft_color::convert::lab_to_srgb(lab)
            }
            7 => {
                lab[1] = 30.0;
                photocraft_color::convert::lab_to_srgb(lab)
            }
            _ => {
                cmyk[0] = 0.4;
                photocraft_color::convert::cmyk_to_rgb(cmyk)
            }
        };
        h.query_all_by_role(Role::SpinButton).nth(field).unwrap().click();
        h.run_steps(1);
        for ch in typed.chars() {
            h.event(egui::Event::Text(ch.to_string()));
            h.run_steps(1);
        }
        h.key_press(egui::Key::Tab);
        h.run_steps(2);
        assert_eq!(h.state().fields["color"], json!(hex(expected)), "field {field}");
    }
    let mut h = fixture(ThemeKind::Classic, vec2(560.0, 368.0), false, false, false);
    h.run_steps(4);
    h.get_by_role(Role::TextInput).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    for ch in "12ab".chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("12ab"));
    assert_eq!(h.state().fields["color"], json!("#cc4422"));
    for ch in "ef".chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    assert_eq!(h.state().fields["color"], json!("#12abef"));
}

#[test]
fn compact_picker_current_sample_restores_color_and_alpha_and_no_color_remains_available_v75() {
    let mut h = fixture(ThemeKind::StudioLight, vec2(560.0, 424.0), true, true, false);
    h.run_steps(4);
    h.get_by_role_and_label(Role::Button, tl!("current")).click();
    h.run_steps(3);
    assert_eq!(h.state().fields["color"], json!("#336699"));
    assert!((h.state().fields["__alpha"].as_f64().unwrap() - 0.8).abs() < 1e-5);
    h.get_by_label(tl!("Only Web Colors")).click();
    h.run_steps(3);
    assert_eq!(h.state().fields["__webOnly"], true);
    h.get_by_label(tl!("No Color")).click();
    h.run_steps(3);
    assert_eq!(h.state().fields["__none"], true);
}

fn scroll_to_end(h: &mut Harness<'_, Fixture>) {
    h.hover_at(egui::pos2(180.0, 100.0));
    h.run_steps(1);
    for _ in 0..5 {
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(-800.0, -800.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(8);
    }
    // Egui keeps a smoothed wheel delta after the raw event. Finish that
    // gesture before issuing a separate accessibility ScrollIntoView action,
    // otherwise later wheel frames legitimately scroll away from its target.
    h.run_steps(32);
}

#[test]
fn narrow_picker_preserves_genuine_scrolling_to_hex_and_no_color_v75() {
    let mut h = fixture(ThemeKind::StudioLight, vec2(380.0, 240.0), true, true, false);
    h.run_steps(6);
    assert!(h.state().content_size.x > h.state().clip.width() && h.state().content_size.y > h.state().clip.height());
    scroll_to_end(&mut h);
    assert!(h.state().offset.x > 50.0 && h.state().offset.y > 50.0, "real wheel input scrolls both axes");
    h.get_by_label(tl!("No Color")).scroll_to_me();
    h.run_steps(8);
    assert!(
        h.state().clip.expand(0.51).contains_rect(h.get_by_label(tl!("No Color")).rect()),
        "last action remains reachable: offset {:?}, clip {:?}, button {:?}",
        h.state().offset,
        h.state().clip,
        h.get_by_label(tl!("No Color")).rect()
    );
    // HEX is above the optional rows: scroll it explicitly into view, then use it.
    h.get_by_role(Role::TextInput).scroll_to_me();
    h.run_steps(8);
    assert!(h.state().clip.expand(0.51).contains_rect(h.get_by_role(Role::TextInput).rect()));
}

#[test]
#[ignore = "writes actual compact color-picker body screenshots; root must visually inspect"]
fn compact_color_picker_gpu_visual_v75() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v75/color-picker-layout-gpu");
    assert!(!out.exists(), "preserve previous actual screenshots");
    std::fs::create_dir_all(&out).unwrap();
    let mut manifest = Vec::new();
    for theme in ThemeKind::ALL {
        let mut h = fixture(theme, vec2(560.0, 368.0), false, false, true);
        h.run_steps(6);
        geometry(&h, 0);
        assert_eq!(Tokens::get(&h.ctx).kind, theme);
        let filename = format!("{}-compact.png", theme.id());
        h.render().unwrap().save(out.join(&filename)).unwrap();
        manifest.push(json!({"file":filename,"theme":theme.id(),"actualTokenTheme":Tokens::get(&h.ctx).kind.id(),
            "body":[h.state().content.width(),h.state().content.height()],"window":[560,368]}));
    }
    let mut h = fixture(ThemeKind::StudioLight, vec2(560.0, 424.0), true, true, true);
    h.run_steps(6);
    geometry(&h, 2);
    h.render().unwrap().save(out.join("alpha-no-color.png")).unwrap();
    let mut h = fixture(ThemeKind::StudioLight, vec2(380.0, 240.0), true, true, true);
    h.run_steps(6);
    h.render().unwrap().save(out.join("narrow-before-scroll.png")).unwrap();
    scroll_to_end(&mut h);
    h.render().unwrap().save(out.join("narrow-after-scroll.png")).unwrap();
    std::fs::write(out.join("manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
}
