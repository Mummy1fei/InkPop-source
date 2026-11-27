//! Synthetic shared-picker routing; no user configuration or private presets.
#![allow(clippy::unwrap_used)]

use super::*;
use egui::{Event, Modifiers, PointerButton, Rect};
use egui_kittest::{Harness, kittest::Queryable};

struct Fixture {
    app: PhotocraftApp,
    first: [u8; 3],
    second: [u8; 3],
    rgba: [f32; 4],
    rects: [Rect; 4],
    optional_color: Option<[f32; 4]>,
    first_changes: usize,
    show_dialog: bool,
    parent: Option<u64>,
}

impl eframe::App for Fixture {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        service(&mut self.app, &ctx);
        if self.app.ui.text_edit.is_some() {
            crate::shortcuts::handle(&mut self.app, &ctx);
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(parent) = self.parent {
                if let Some(mut fields) = self.app.ui.dialogs.iter().find(|d| d.id == parent).map(|d| d.fields.clone()) {
                    crate::fill_ui::body(&self.app, ui, &mut fields);
                    if let Some(dialog) = self.app.ui.dialog_mut(parent) {
                        dialog.fields = fields;
                    }
                }
            } else {
                ui.horizontal(|ui| {
                    let first = edit_srgb(ui, &mut self.first);
                    self.rects[0] = first.rect;
                    self.first_changes += usize::from(first.changed());
                    self.rects[1] = edit_srgb(ui, &mut self.second).rect;
                    self.rects[2] = edit_rgba_unmultiplied(ui, &mut self.rgba).rect;
                    let (rect, mut response) = ui.allocate_exact_size(vec2(32.0, 18.0), egui::Sense::click());
                    self.rects[3] = rect;
                    ui.painter().rect_filled(rect, 0.0, egui::Color32::BLACK);
                    if let Some(color) = swatch(&mut response, self.optional_color.unwrap_or([0.0, 0.0, 0.0, 1.0]), false, true) {
                        self.optional_color = color;
                    }
                });
            }
        });
        if self.show_dialog {
            crate::dialogs::show(&mut self.app, &ctx);
        }
        service(&mut self.app, &ctx);
    }
}

fn harness(theme: crate::theme::ThemeKind, gpu: bool, show_dialog: bool) -> Harness<'static, Fixture> {
    let builder = Harness::builder().with_size(vec2(920.0, 700.0)).with_step_dt(1.0 / 60.0).with_max_steps(64);
    let builder = if gpu { builder.wgpu_setup(crate::gpu_canvas::wgpu_setup()) } else { builder };
    builder.build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":32,"height":32})).unwrap();
        Fixture {
            app,
            first: [51, 102, 153],
            second: [51, 102, 153],
            rgba: [0.1, 0.2, 0.3, 0.4],
            rects: [Rect::NOTHING; 4],
            optional_color: Some([0.0, 0.0, 0.0, 1.0]),
            first_changes: 0,
            show_dialog,
            parent: None,
        }
    })
}

fn click(h: &mut Harness<'static, Fixture>, index: usize) {
    h.run_steps(3);
    let pos = h.state().rects[index].center();
    h.event(Event::PointerMoved(pos));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
}

fn set_confirm(h: &mut Harness<'static, Fixture>, color: &str, alpha: Option<f32>) {
    let app = &mut h.state_mut().app;
    let id = app.ui.dialogs.last().unwrap().id;
    let d = app.ui.dialog_mut(id).unwrap();
    d.fields.insert("color".into(), json!(color));
    if let Some(alpha) = alpha {
        d.fields.insert("__alpha".into(), json!(alpha));
    }
    crate::dialogs::confirm(app, id).unwrap();
    h.run_steps(3);
}

#[test]
fn shared_picker_confirm_returns_to_exact_original_well_and_cancel_keeps_values_v74() {
    let mut h = harness(crate::theme::ThemeKind::StudioLight, false, false);
    click(&mut h, 0);
    assert_eq!(h.state().app.ui.dialogs.len(), 1);
    assert_eq!(h.state().first, [51, 102, 153], "opening does not edit the value");
    set_confirm(&mut h, "#ff8822", None);
    assert_eq!(h.state().first, [255, 136, 34]);
    assert_eq!(h.state().second, [51, 102, 153], "same-colour neighbouring widget must not consume the result");
    click(&mut h, 1);
    let app = &mut h.state_mut().app;
    let id = app.ui.dialogs.last().unwrap().id;
    app.ui.dialog_mut(id).unwrap().fields.insert("color".into(), json!("#00ff00"));
    app.ui.close_dialog(id);
    h.run_steps(3);
    assert_eq!(h.state().first, [255, 136, 34]);
    assert_eq!(h.state().second, [51, 102, 153], "Cancel must not mutate the parent value");
}

#[test]
fn confirming_original_color_is_not_an_edit_and_does_not_quantize_float_values_v74() {
    let mut h = harness(crate::theme::ThemeKind::Pro, false, false);
    click(&mut h, 0);
    set_confirm(&mut h, "#336699", None);
    assert_eq!(h.state().first_changes, 0);
    let original = h.state().rgba;
    click(&mut h, 2);
    let color = h.state().app.ui.dialogs.last().unwrap().fields["color"].as_str().unwrap().to_string();
    set_confirm(&mut h, &color, None);
    assert_eq!(h.state().rgba, original, "opening and OK preserves exact float RGB/alpha");
}

#[test]
fn shared_picker_preserves_alpha_and_linear_rgb_contract_v74() {
    let mut h = harness(crate::theme::ThemeKind::Pro, false, false);
    click(&mut h, 2);
    let fields = &h.state().app.ui.dialogs.last().unwrap().fields;
    assert_eq!(fields["__allowAlpha"], json!(true));
    assert!((fields["__alpha"].as_f64().unwrap() - 0.4).abs() < 1e-5);
    set_confirm(&mut h, "#8acc22", Some(0.63));
    let rgba = h.state().rgba;
    for (got, want) in rgba[..3].iter().zip([138.0 / 255.0, 204.0 / 255.0, 34.0 / 255.0].map(photocraft_color::convert::srgb_to_linear)) {
        assert!((*got - want).abs() < 1e-5);
    }
    assert!((rgba[3] - 0.63).abs() < 1e-6);
}

#[test]
fn picker_rejects_changed_target_or_invalid_color_without_closing_v74() {
    let mut h = harness(crate::theme::ThemeKind::Classic, false, false);
    click(&mut h, 0);
    let app = &mut h.state_mut().app;
    let id = app.ui.dialogs.last().unwrap().id;
    app.ui.dialog_mut(id).unwrap().fields.insert("color".into(), json!("not a color"));
    assert!(crate::dialogs::confirm(app, id).is_err());
    assert!(app.ui.dialog_mut(id).is_some(), "invalid input stays editable");
    app.ui.dialog_mut(id).unwrap().fields.insert("color".into(), json!("#ff0000"));
    app.run("layer.new.layer", json!({})).unwrap();
    assert!(crate::dialogs::confirm(app, id).is_err(), "cannot retarget a different layer");
    assert!(app.ui.color_picker_results.is_empty());
    app.ui.close_dialog(id);
    h.run_steps(3);
    assert_eq!(h.state().first, [51, 102, 153]);
}

#[test]
fn shared_picker_keeps_no_color_action_and_never_serializes_pending_values_v74() {
    let mut h = harness(crate::theme::ThemeKind::Pro, false, false);
    click(&mut h, 3);
    let app = &mut h.state_mut().app;
    let id = app.ui.dialogs.last().unwrap().id;
    assert_eq!(app.ui.dialog_mut(id).unwrap().fields["__allowNone"], json!(true));
    app.ui.dialog_mut(id).unwrap().fields.insert("__none".into(), json!(true));
    crate::dialogs::confirm(app, id).unwrap();
    assert_eq!(app.ui.color_picker_results.len(), 1);
    let serialized = serde_json::to_value(&app.ui).unwrap();
    assert!(serialized.get("color_picker_results").is_none());
    let restored: crate::state::UiState = serde_json::from_value(serialized).unwrap();
    assert!(restored.color_picker_results.is_empty());
    h.run_steps(3);
    assert!(h.state().optional_color.is_none(), "No Color must reach the original shape swatch");
}

#[test]
fn text_picker_captures_selection_and_keeps_edit_session_v74() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":300,"height":100})).unwrap();
    app.run("type.create", json!({"text":"ABCD","x":10,"y":30})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    app.ui.text_edit = Some(crate::state::TextEdit {
        layer: layer.0,
        caret: 3,
        anchor: 1,
        session: "picker-type-session".into(),
        created: false,
        dragging: false,
        resize: None,
        preedit: None,
    });
    let id = open_text(&mut app);
    let fields = app.ui.dialog_mut(id).unwrap().fields.clone();
    assert_eq!(fields["__params"]["layer"], json!(layer.0));
    assert_eq!(fields["__params"]["range"], json!([1, 3]));
    assert_eq!(fields["__params"]["coalesce"], json!("picker-type-session"));
    app.ui.dialog_mut(id).unwrap().fields.insert("color".into(), json!("#cc2299"));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(app.ui.text_edit.as_ref().unwrap().session, "picker-type-session");
    let text = match &app.session.active().unwrap().doc.layer(layer).unwrap().content {
        photocraft_doc::LayerContent::Text(t) => t,
        _ => panic!("text layer"),
    };
    assert_eq!(text.text, "ABCD");
    let runs = text.char_runs();
    assert!(runs.iter().any(|run| hex(run.style.color.to_rgb()) == "#cc2299"));
}

#[test]
fn picker_hex_and_ime_input_never_enters_the_canvas_text_v74() {
    let mut h = harness(crate::theme::ThemeKind::StudioLight, false, true);
    let app = &mut h.state_mut().app;
    app.run("type.create", json!({"text":"ABCD","x":5,"y":20})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    app.ui.tool = crate::state::Tool::Type;
    app.ui.text_edit = Some(crate::state::TextEdit {
        layer: layer.0,
        caret: 3,
        anchor: 1,
        session: "hex-isolation-session".into(),
        created: false,
        dragging: false,
        resize: None,
        preedit: None,
    });
    let id = open_text(app);
    h.run_steps(4);
    h.get_by_role(egui::accesskit::Role::TextInput).click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.event(Event::Ime(egui::ImeEvent::Preedit { text: "文".into(), active_range_chars: Some(0..1) }));
    h.run_steps(1);
    h.event(Event::Ime(egui::ImeEvent::Commit("字".into())));
    h.run_steps(2);
    assert!(h.state().app.ui.text_edit.as_ref().unwrap().preedit.is_none(), "picker IME must not enter canvas composition");
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.event(Event::Text("00ffcc".into()));
    h.run_steps(3);
    let app = &h.state().app;
    assert_eq!(app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields["color"], json!("#00ffcc"));
    let text = match &app.session.active().unwrap().doc.layer(layer).unwrap().content {
        photocraft_doc::LayerContent::Text(t) => t,
        _ => panic!("text layer"),
    };
    assert_eq!(text.text, "ABCD", "all keyboard/IME input belongs to the picker field");
    assert_eq!(app.ui.text_edit.as_ref().unwrap().session, "hex-isolation-session");
    crate::dialogs::confirm(&mut h.state_mut().app, id).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().app.ui.text_edit.as_ref().unwrap().session, "hex-isolation-session");
}

#[test]
fn nested_fill_picker_changes_parent_only_after_confirm_v74() {
    let mut h = harness(crate::theme::ThemeKind::Studio, false, false);
    let parent = crate::fill_ui::open(&mut h.state_mut().app);
    h.state_mut().app.ui.dialog_mut(parent).unwrap().fields.insert("contents".into(), json!("color"));
    h.state_mut().parent = Some(parent);
    h.run_steps(4);
    h.get_by_label(tl!("Color")).click();
    h.run_steps(4);
    assert_eq!(h.state().app.ui.dialogs.len(), 2, "picker is stacked above the still-live Fill dialog");
    assert_ne!(h.state().app.ui.dialogs.last().unwrap().id, parent);
    assert_eq!(h.state().app.ui.dialogs.iter().find(|d| d.id == parent).unwrap().fields["color"], json!("#000000"));
    set_confirm(&mut h, "#dd2266", None);
    assert_eq!(h.state().app.ui.dialogs.len(), 1);
    let fields = &h.state().app.ui.dialogs.iter().find(|d| d.id == parent).unwrap().fields;
    assert_eq!(fields["color"], json!("#dd2266"));
    assert_eq!(fields["contents"], json!("color"));
}

#[test]
#[ignore = "writes shared independent picker theme screenshots for root review"]
fn shared_color_picker_gpu_visual_v74() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v74/color-gpu");
    assert!(!out.exists(), "preserve previous screenshots");
    std::fs::create_dir_all(&out).unwrap();
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = harness(theme, true, true);
        click(&mut h, 0);
        assert!(h.state().app.ui.dialogs.last().is_some_and(|d| owns(&d.fields)));
        h.run_steps(5);
        h.render().unwrap().save(out.join(format!("{}-picker.png", theme.id()))).unwrap();
    }
    let mut h = harness(crate::theme::ThemeKind::StudioLight, true, true);
    click(&mut h, 2);
    h.run_steps(5);
    h.render().unwrap().save(out.join("alpha-picker.png")).unwrap();
}
