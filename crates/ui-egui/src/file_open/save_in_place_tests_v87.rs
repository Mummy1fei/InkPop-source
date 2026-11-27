//! Regression of the actual File > Save route, with real PNG/JPEG/PSD bytes.
//! The platform picker/writer are captured, so no user files can be overwritten.

use std::{cell::RefCell, rc::Rc, sync::Arc};

use egui_kittest::Harness;
use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, LayerMask, TextLayer};
use photocraft_geom::Size;
use serde_json::{Value, json};

use crate::{PhotocraftApp, Services, menus};

#[derive(Default)]
struct Trace {
    picks: Vec<String>,
    writes: Vec<(String, Vec<u8>)>,
    qualities: Vec<Option<u8>>,
}

type Recorded = Rc<RefCell<Trace>>;

fn plain() -> Document {
    Document::with_background("original.jpg", Size::new(12, 8), ColorMode::Rgb, SampleType::U8, Color::WHITE)
}

fn app(doc: Document, path: Option<&str>, picked: Option<&str>) -> (PhotocraftApp, Recorded) {
    let trace: Recorded = Rc::default();
    let picks = trace.clone();
    let writes = trace.clone();
    let qualities = trace.clone();
    let picked = picked.map(str::to_string);
    let services = Services {
        pick_save: Some(Box::new(move |suggested| {
            picks.borrow_mut().picks.push(suggested.to_string());
            picked.clone()
        })),
        export: Some(Box::new(move |doc, path, settings| {
            qualities.borrow_mut().qualities.push(settings.jpeg_quality);
            let mut options = photocraft_io::ExportOptions::default();
            if let Some(quality) = settings.jpeg_quality {
                options.encode.jpeg_quality = quality;
            }
            let encoded = photocraft_io::export(doc, path, &options).map_err(|error| error.to_string())?;
            Ok((encoded.bytes, encoded.warnings))
        })),
        write: Some(Box::new(move |path, bytes| {
            writes.borrow_mut().writes.push((path.to_string(), bytes.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.session.add_document(doc, path.map(str::to_string));
    (app, trace)
}

fn imported(path: &str, picked: Option<&str>) -> (PhotocraftApp, Recorded) {
    let encoded = photocraft_io::export(&plain(), path, &Default::default()).unwrap();
    let doc = photocraft_io::import(path, &encoded.bytes).unwrap().document;
    app(doc, Some(path), picked)
}

fn invoke(app: &mut PhotocraftApp, command: &str) -> Result<Value, String> {
    menus::invoke(app, &egui::Context::default(), command, json!({}))
}

fn paint(app: &mut PhotocraftApp, color: &str) {
    app.run("edit.fill", json!({"color":color})).unwrap();
    assert!(app.session.active().unwrap().is_dirty());
}

fn identity(app: &PhotocraftApp) -> (u64, String, Option<String>, u64, u64, usize) {
    let st = app.session.active().unwrap();
    (st.doc.id.0, st.doc.name.clone(), st.path.clone(), st.revision, st.saved_revision, st.history.past_len())
}

#[test]
fn opened_single_png_and_jpeg_save_each_edit_to_the_original_without_a_picker_v87() {
    for path in ["D:/synthetic/原图.PNG", "D:/synthetic/原图.JPG", "D:/synthetic/原图.JPEG"] {
        // A cancelled picker makes the pre-fix route fail, rather than silently
        // accepting a second choice which happens to be the same path.
        let (mut app, trace) = imported(path, None);
        for (step, (color, expected)) in [("#ff0000", [1.0, 0.0, 0.0]), ("#0000ff", [0.0, 0.0, 1.0])].into_iter().enumerate() {
            paint(&mut app, color);
            let before = identity(&app);
            let response = invoke(&mut app, "file.save").unwrap();
            if let Some(prompt) = app.jpeg_options.take() {
                assert_eq!(response["pending"], true);
                assert_eq!(prompt.path, path);
                assert_eq!(app.complete_jpeg_save(prompt).unwrap().0, path);
            } else {
                assert_eq!(response["path"], path);
            }
            let st = app.session.active().unwrap();
            assert_eq!(st.path.as_deref(), Some(path));
            assert_eq!((st.revision, st.saved_revision), (before.3, before.3));
            assert!(!st.is_dirty());
            let got = trace.borrow();
            assert!(got.picks.is_empty(), "ordinary Save must not ask for an established flat path");
            assert_eq!(got.writes.len(), step + 1);
            assert_eq!(got.writes[step].0, path);
            let reopened = photocraft_io::import(path, &got.writes[step].1).unwrap().document;
            assert_eq!(reopened.layer_count(), 1);
            let px = reopened.layers[0].surface().unwrap().pixel(6, 4);
            for (actual, expected) in px.iter().take(3).zip(expected) {
                // Uniform JPEG primaries tolerate codec quantisation; PNG also
                // goes through the real encoder/decoder rather than a stub.
                assert!((*actual - expected).abs() < 0.02, "{path}: saved edit {step} has {px:?}");
            }
        }
    }
}

#[test]
fn real_ctrl_s_saves_each_single_png_and_jpeg_edit_without_reopening_the_picker_v87() {
    for path in ["D:/synthetic/shortcut.png", "D:/synthetic/shortcut.jpg"] {
        let (app, trace) = imported(path, None);
        let mut h = Harness::builder()
            .with_size(egui::vec2(700.0, 400.0))
            .with_max_steps(64)
            .build_ui_state(|ui, app: &mut PhotocraftApp| crate::shortcuts::handle(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::StudioLight);
        h.run_steps(3);
        let modifiers = egui::Modifiers { command: true, ctrl: !cfg!(target_os = "macos"), mac_cmd: cfg!(target_os = "macos"), ..Default::default() };
        for (step, (color, expected)) in [("#ff0000", [1.0, 0.0, 0.0]), ("#0000ff", [0.0, 0.0, 1.0])].into_iter().enumerate() {
            paint(h.state_mut(), color);
            let before = identity(h.state());
            h.key_press_modifiers(modifiers, egui::Key::S);
            h.run_steps(3);
            if let Some(prompt) = h.state_mut().jpeg_options.take() {
                assert_eq!(step, 0, "quality is asked once per editing session");
                assert_eq!(prompt.path, path);
                h.state_mut().complete_jpeg_save(prompt).unwrap();
            }
            let state = h.state().session.active().unwrap();
            assert_eq!(state.path.as_deref(), Some(path));
            assert_eq!((state.revision, state.saved_revision), (before.3, before.3), "the actual shortcut marks only this written revision saved");
            assert_eq!(state.history.past_len(), before.5, "saving does not create an edit");
            assert!(!state.is_dirty());
            let got = trace.borrow();
            assert!(got.picks.is_empty());
            assert_eq!(got.writes.len(), step + 1, "each actual shortcut writes its new edit exactly once");
            assert_eq!(got.writes[step].0, path);
            if step != 0 {
                assert_ne!(got.writes[step].1, got.writes[step - 1].1, "the second shortcut must encode the second edit");
            }
            let reopened = photocraft_io::import(path, &got.writes[step].1).unwrap().document;
            let px = reopened.layers[0].surface().unwrap().pixel(6, 4);
            for (actual, expected) in px.iter().take(3).zip(expected) {
                assert!((*actual - expected).abs() < 0.02, "{path}: shortcut {step} wrote {px:?}");
            }
        }
    }
}

#[test]
fn save_as_still_asks_once_and_the_next_ordinary_save_uses_its_new_path_v87() {
    for (source, destination) in [("D:/synthetic/original.png", "D:/synthetic/renamed.png"), ("D:/synthetic/original.jpg", "D:/synthetic/renamed.jpg")] {
        let (mut app, trace) = imported(source, Some(destination));
        paint(&mut app, "#ff0000");
        let response = invoke(&mut app, "file.saveAs").unwrap();
        if destination.ends_with(".jpg") {
            assert_eq!(response["pending"], true);
            let mut prompt = app.jpeg_options.take().unwrap();
            prompt.quality = 37;
            assert_eq!(app.complete_jpeg_save(prompt).unwrap().0, destination);
        } else {
            assert_eq!(response["path"], destination);
        }
        assert_eq!(trace.borrow().picks, [source]);
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some(destination));
        assert_eq!(trace.borrow().writes[0].0, destination);
        paint(&mut app, "#0000ff");
        assert_eq!(invoke(&mut app, "file.save").unwrap()["path"], destination);
        let got = trace.borrow();
        assert_eq!(got.picks, [source], "the second Save does not reopen Save As");
        assert_eq!(got.writes.len(), 2);
        assert!(got.writes.iter().all(|(path, _)| path == destination));
        if destination.ends_with(".jpg") {
            assert_eq!(got.qualities, [Some(37), Some(37)], "a normal Save reuses the chosen JPEG quality");
        }
    }
}

#[test]
fn saving_a_layered_psd_as_jpeg_keeps_its_identity_and_ctrl_s_updates_the_psd() {
    let master = "D:/synthetic/原始稿.PSD";
    let jpeg = "D:/synthetic/分享稿.JPEG";
    let mut doc = plain();
    doc.layers.push(Layer::raster("Editable artwork", PixelFormat::RGBA8));
    let encoded = photocraft_io::export(&doc, master, &Default::default()).unwrap();
    let original = photocraft_io::import("原始稿.PSD", &encoded.bytes).unwrap().document;
    let (mut app, trace) = app(original, Some(master), Some(jpeg));
    paint(&mut app, "#ff0000");
    let before = identity(&app);
    let response = invoke(&mut app, "file.saveAs").unwrap();
    assert_eq!(response["pending"], true);
    assert!(trace.borrow().writes.is_empty(), "quality is chosen before writing");
    let mut prompt = app.jpeg_options.take().unwrap();
    assert!(prompt.as_copy);
    prompt.quality = 95;
    app.complete_jpeg_save(prompt).unwrap();
    assert_eq!(identity(&app), before);
    assert!(app.session.active().unwrap().is_dirty());
    assert!(!app.ui.recent_files.iter().any(|path| path == jpeg), "copy does not become the current document");
    paint(&mut app, "#0000ff");
    let mut h = Harness::builder()
        .with_size(egui::vec2(700.0, 400.0))
        .with_max_steps(64)
        .build_ui_state(|ui, app: &mut PhotocraftApp| crate::shortcuts::handle(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::StudioLight);
    h.run_steps(3);
    let modifiers = egui::Modifiers { command: true, ctrl: !cfg!(target_os = "macos"), mac_cmd: cfg!(target_os = "macos"), ..Default::default() };
    h.key_press_modifiers(modifiers, egui::Key::S);
    h.run_steps(3);
    let got = trace.borrow();
    assert_eq!(got.picks, [master], "only Save As asks for a destination");
    assert_eq!(got.qualities, [Some(95), None]);
    assert_eq!(got.writes.iter().map(|(path, _)| path.as_str()).collect::<Vec<_>>(), [jpeg, master]);
    assert!(got.writes[0].1.starts_with(&[0xff, 0xd8]));
    let reopened = photocraft_io::import(master, &got.writes[1].1).unwrap().document;
    assert_eq!(reopened.layer_count(), 2);
    assert_eq!(reopened.layers[1].surface().unwrap().rgba(6, 4), [0.0, 0.0, 1.0, 1.0]);
    let after = identity(h.state());
    assert_eq!((after.0, after.1, after.2), (before.0, before.1, before.2));
    assert_eq!(after.3, after.4);
}

#[test]
fn cancelling_save_as_keeps_the_existing_flat_document_dirty_and_unchanged_v87() {
    let (mut app, trace) = imported("D:/synthetic/original.jpg", None);
    paint(&mut app, "#ff0000");
    let before = identity(&app);
    let snapshot = app.session.active().unwrap().doc.clone();
    assert_eq!(invoke(&mut app, "file.saveAs").unwrap_err(), "cancelled");
    assert_eq!(identity(&app), before);
    assert!(Arc::ptr_eq(&snapshot, &app.session.active().unwrap().doc));
    assert!(app.session.active().unwrap().is_dirty());
    assert_eq!(trace.borrow().picks, ["D:/synthetic/original.jpg"]);
    assert!(trace.borrow().writes.is_empty());
}

#[test]
fn flat_paths_do_not_silently_overwrite_editable_layers_or_read_only_formats_v87() {
    for kind in ["new", "multiple", "text", "mask", "effect", "heic", "unknown"] {
        let mut doc = plain();
        match kind {
            "multiple" => doc.layers.push(Layer::raster("separate paint", PixelFormat::RGBA8)),
            "text" => doc.layers = vec![Layer::new("editable type", LayerContent::Text(TextLayer::default()))],
            "mask" => {
                let mut mask = LayerMask::reveal_all();
                mask.enabled = false;
                doc.layers[0].mask = Some(mask);
            }
            _ => {}
        }
        let source = match kind {
            "new" => None,
            "heic" => Some("D:/synthetic/original.HEIC"),
            "unknown" => Some("D:/synthetic/original.unknown"),
            _ => Some("D:/synthetic/original.jpg"),
        };
        let (mut app, trace) = app(doc, source, None);
        if kind == "effect" {
            app.run("layer.layerStyle.stroke", json!({"size":3})).unwrap();
        }
        app.run("layer.renameLayer", json!({"name":"Edited layer"})).unwrap();
        let before = identity(&app);
        assert!(app.session.active().unwrap().is_dirty());
        assert_eq!(invoke(&mut app, "file.save").unwrap_err(), "cancelled", "{kind}");
        let expected = if kind == "new" { "original.psd" } else { "D:/synthetic/original.psd" };
        assert_eq!(trace.borrow().picks, [expected], "{kind}: offer a layered destination");
        assert!(trace.borrow().writes.is_empty(), "{kind}: no original bytes overwritten");
        assert_eq!(identity(&app), before, "{kind}: cancelled Save never adopts or marks clean");
    }
}

#[test]
fn off_canvas_single_layer_pixels_require_a_layered_save_instead_of_truncation_v87() {
    for path in ["D:/synthetic/original.png", "D:/synthetic/original.jpg"] {
        for (x, y) in [(-1, 4), (12, 4)] {
            let mut doc = plain();
            doc.layers[0].surface_mut().unwrap().write_pixel(x, y, &[1.0, 0.0, 0.0, 1.0]);
            let (mut app, trace) = app(doc, Some(path), None);
            app.run("layer.renameLayer", json!({"name":"Paint extends beyond canvas"})).unwrap();
            let before = identity(&app);
            let snapshot = app.session.active().unwrap().doc.clone();
            assert_eq!(invoke(&mut app, "file.save").unwrap_err(), "cancelled");
            assert_eq!(trace.borrow().picks, ["D:/synthetic/original.psd"], "a one-layer flat save would lose pixel {x},{y}");
            assert!(trace.borrow().writes.is_empty(), "the original flat file must not be overwritten");
            assert_eq!(identity(&app), before);
            assert!(Arc::ptr_eq(&snapshot, &app.session.active().unwrap().doc));
            assert_eq!(app.session.active().unwrap().doc.layers[0].surface().unwrap().pixel(x, y), [1.0, 0.0, 0.0, 1.0]);
        }
    }
}

#[test]
fn failed_in_place_export_or_write_does_not_adopt_a_path_or_mark_saved_v87() {
    for failure in ["encode", "write"] {
        let (mut app, trace) = imported("D:/synthetic/original.jpg", None);
        paint(&mut app, "#ff0000");
        if failure == "encode" {
            app.services.export = Some(Box::new(|_, _, _| Err("v87 synthetic encoder failure".into())));
        } else {
            app.services.write = Some(Box::new(|_, _| Err("v87 synthetic disk write failure".into())));
        }
        let before = identity(&app);
        let snapshot = app.session.active().unwrap().doc.clone();
        assert_eq!(invoke(&mut app, "file.save").unwrap()["pending"], true);
        let prompt = app.jpeg_options.take().unwrap();
        let error = app.complete_jpeg_save(prompt).unwrap_err();
        assert!(error.contains(if failure == "encode" { "encoder failure" } else { "disk write failure" }));
        assert_eq!(identity(&app), before);
        assert!(Arc::ptr_eq(&snapshot, &app.session.active().unwrap().doc));
        assert!(app.session.active().unwrap().is_dirty());
        assert!(trace.borrow().picks.is_empty());
        assert!(trace.borrow().writes.is_empty());
    }
}

fn close_prompt(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(800.0, 600.0)).with_max_steps(64).build_ui_state(
        |ui, app| {
            if ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::discard_ui::show(app, ui.ctx());
                if let Some(crate::jpeg_options_ui::Answer::Save(prompt)) = crate::jpeg_options_ui::show(app, ui.ctx())
                    && let Err(error) = app.complete_jpeg_save(prompt)
                {
                    crate::notices::error(app, error);
                }
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::StudioLight);
    h.ctx.set_os(egui::os::OperatingSystem::Windows);
    assert!(crate::discard_ui::intercept(h.state_mut(), "file.close", &json!({})));
    h.run_steps(3);
    h
}

#[test]
fn close_save_of_an_edited_jpeg_writes_in_place_before_closing_v87() {
    let path = "D:/synthetic/original.jpg";
    let (mut app, trace) = imported(path, None);
    paint(&mut app, "#ff0000");
    let mut h = close_prompt(app);
    h.key_press(egui::Key::Y);
    h.run_steps(3);
    assert_eq!(h.state().jpeg_options.as_ref().unwrap().path, path);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().session.documents().is_empty(), "successful close-save advances the pending close");
    assert!(h.state().discard.is_none());
    let got = trace.borrow();
    assert!(got.picks.is_empty(), "a close-save must not turn into Save As for this JPEG");
    assert_eq!(got.writes.len(), 1);
    assert_eq!(got.writes[0].0, path);
    let reopened = photocraft_io::import(path, &got.writes[0].1).unwrap().document;
    let pixel = reopened.layers[0].surface().unwrap().pixel(6, 4);
    assert!(pixel[0] > 0.98 && pixel[1] < 0.02 && pixel[2] < 0.02, "the saved bytes contain the pending edit");
}

#[test]
fn close_save_failure_keeps_the_dirty_document_and_prompt_open_v87() {
    let (mut app, trace) = imported("D:/synthetic/original.jpg", None);
    paint(&mut app, "#ff0000");
    app.services.write = Some(Box::new(|_, _| Err("v87 synthetic disk write failure".into())));
    let before = identity(&app);
    let mut h = close_prompt(app);
    h.key_press(egui::Key::Y);
    h.run_steps(3);
    assert!(h.state().jpeg_options.is_some());
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().session.documents().len(), 1);
    assert_eq!(identity(h.state()), before);
    assert!(h.state().session.active().unwrap().is_dirty());
    assert!(h.state().discard.is_some());
    assert!(h.state().ui.status.contains("v87 synthetic disk write failure"));
    assert!(trace.borrow().picks.is_empty());
    assert!(trace.borrow().writes.is_empty());
}
