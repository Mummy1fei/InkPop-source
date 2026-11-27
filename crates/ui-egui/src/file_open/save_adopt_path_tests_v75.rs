use std::{cell::RefCell, rc::Rc};

use egui_kittest::{Harness, kittest::Queryable};
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::Document;
use photocraft_geom::Size;
use serde_json::{Value, json};

use crate::{PhotocraftApp, Services, menus, theme::ThemeKind};

type Writes = Rc<RefCell<Vec<(String, Vec<u8>)>>>;
type Picks = Rc<RefCell<Vec<String>>>;

fn app(target: Option<&str>) -> (PhotocraftApp, Picks, Writes) {
    let picks: Picks = Rc::default();
    let writes: Writes = Rc::default();
    let seen = picks.clone();
    let written = writes.clone();
    let target = target.map(str::to_string);
    let services = Services {
        pick_save: Some(Box::new(move |suggested| {
            seen.borrow_mut().push(suggested.to_string());
            target.clone()
        })),
        export: Some(Box::new(|doc, path, _| {
            let out = photocraft_io::export(doc, path, &Default::default()).map_err(|e| e.to_string())?;
            Ok((out.bytes, out.warnings))
        })),
        write: Some(Box::new(move |path, bytes| {
            written.borrow_mut().push((path.to_string(), bytes.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let doc = Document::with_background("source-image.png", Size::new(8, 6), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    app.session.add_document(doc, Some("D:/synthetic/source-image.png".into()));
    app.run("layer.new.layer", json!({"name":"separate paint"})).unwrap();
    (app, picks, writes)
}

fn invoke(app: &mut PhotocraftApp, command: &str) -> Value {
    menus::invoke(app, &egui::Context::default(), command, json!({})).unwrap()
}

fn identity(app: &PhotocraftApp) -> (u64, String, Option<String>, u64, u64, usize) {
    let st = app.session.active().unwrap();
    (st.doc.id.0, st.doc.name.clone(), st.path.clone(), st.revision, st.saved_revision, st.history.past_len())
}

#[test]
fn normal_save_and_save_as_adopt_real_psd_path_name_clean_revision_and_reopen_layers_v75() {
    for command in ["file.save", "file.saveAs"] {
        let target = "D:/synthetic/示例画布.psd";
        let (mut app, picks, writes) = app(Some(target));
        let before = identity(&app);
        assert!(app.session.active().unwrap().is_dirty());
        assert_eq!(invoke(&mut app, command)["path"], target);
        let after = identity(&app);
        assert_eq!(after.0, before.0, "Save As retains document identity");
        assert_eq!(after.1, "示例画布.psd");
        assert_eq!(after.2.as_deref(), Some(target));
        assert_eq!((after.3, after.5), (before.3, before.5), "adopting a path does not add a history step/revision");
        assert_eq!(after.3, after.4);
        assert!(!app.session.active().unwrap().is_dirty());
        assert_eq!(app.ui.recent_files.first().map(String::as_str), Some(target));
        assert_eq!(*picks.borrow(), ["D:/synthetic/source-image.psd"], "v74 layered default remains PSD");
        let written = writes.borrow();
        assert_eq!(written.len(), 1);
        assert!(photocraft_io::is_psd(&written[0].1));
        let reopened = photocraft_io::import(target, &written[0].1).unwrap().document;
        assert_eq!(reopened.layer_count(), 2, "both layers survive actual PSD encoding");
        let mut opened = PhotocraftApp::new(
            photocraft_engine::Session::new(),
            Services {
                import: Some(Box::new(|name, bytes| {
                    let imported = photocraft_io::import(name, bytes).map_err(|error| error.to_string())?;
                    Ok((imported.document, imported.warnings))
                })),
                ..Default::default()
            },
        );
        opened.open_file(target, &written[0].1).unwrap();
        assert_eq!(opened.session.active().unwrap().path.as_deref(), Some(target));
        assert_eq!(opened.session.active().unwrap().doc.name, "示例画布.psd");
        assert_eq!(opened.session.active().unwrap().doc.layer_count(), 2, "UI reopening retains both encoded layers");
    }
}

#[test]
fn second_ctrl_s_writes_adopted_psd_without_reopening_picker_v75() {
    let target = "D:/synthetic/confirmed.psd";
    let (mut app, picks, writes) = app(Some(target));
    invoke(&mut app, "file.saveAs");
    app.run("layer.new.layer", json!({"name":"later edit"})).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(700.0, 400.0))
        .with_max_steps(64)
        .build_ui_state(|ui, app: &mut PhotocraftApp| crate::shortcuts::handle(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, ThemeKind::StudioLight);
    h.run_steps(3);
    let modifiers = egui::Modifiers { command: true, ctrl: !cfg!(target_os = "macos"), mac_cmd: cfg!(target_os = "macos"), ..Default::default() };
    h.key_press_modifiers(modifiers, egui::Key::S);
    h.run_steps(3);
    assert_eq!(picks.borrow().len(), 1, "second actual Save shortcut never asks for another path");
    assert_eq!(writes.borrow().len(), 2, "second Save wrote again");
    assert!(writes.borrow().iter().all(|(path, _)| path == target));
    assert_eq!(h.state().session.active().unwrap().doc.name, "confirmed.psd");
    assert!(!h.state().session.active().unwrap().is_dirty());
    let reopened = photocraft_io::import(target, &writes.borrow()[1].1).unwrap().document;
    assert_eq!(reopened.layer_count(), 3, "the second save contains the new edit");
}

#[test]
fn saved_pcraft_embeds_destination_name_in_bytes_for_both_writers_v75() {
    for automation in [false, true] {
        let (mut app, _, writes) = app(Some("D:/synthetic/renamed-art.pcraft"));
        let auto_written: Writes = Rc::default();
        let captured = auto_written.clone();
        app.services.automation_write = Some(Box::new(move |path, bytes| {
            captured.borrow_mut().push((path.to_string(), bytes.to_vec()));
            Ok(())
        }));
        if automation {
            app.save_automation(Some("D:/synthetic/renamed-art.pcraft".into())).unwrap();
        } else {
            invoke(&mut app, "file.saveAs");
        }
        let bytes = if automation { auto_written.borrow()[0].1.clone() } else { writes.borrow()[0].1.clone() };
        // Native import preserves the bundle's internal document name. An
        // unrelated input name ensures this does not pass through a UI rename.
        let reopened = photocraft_io::import("unrelated-input.pcraft", &bytes).unwrap().document;
        assert_eq!(reopened.name, "renamed-art.pcraft");
        assert_eq!(reopened.layer_count(), 2);
        assert_eq!(app.session.active().unwrap().doc.name, "renamed-art.pcraft");
    }
}

#[test]
fn destination_named_export_snapshot_never_mutates_live_identity_on_encode_error_v75() {
    let (mut app, _, _) = app(Some("D:/synthetic/new-name.psd"));
    let seen: Rc<RefCell<Option<String>>> = Rc::default();
    let captured = seen.clone();
    app.services.export = Some(Box::new(move |doc, _, _| {
        *captured.borrow_mut() = Some(doc.name.clone());
        Err("synthetic encoder failure after inspecting snapshot".into())
    }));
    let original = identity(&app);
    assert!(menus::invoke(&mut app, &egui::Context::default(), "file.saveAs", json!({})).is_err());
    assert_eq!(seen.borrow().as_deref(), Some("new-name.psd"), "encoder sees destination metadata");
    assert_eq!(identity(&app), original, "live name/path/revision/history stay unchanged on failure");
}

#[test]
fn cancelled_encode_failed_or_write_failed_save_preserves_all_document_identity_v75() {
    for failure in ["cancel", "encode", "write"] {
        let (mut app, _, writes) = app(if failure == "cancel" { None } else { Some("D:/synthetic/failed.psd") });
        if failure == "encode" {
            app.services.export = Some(Box::new(|_, _, _| Err("synthetic encoder failure".into())));
        }
        if failure == "write" {
            app.services.write = Some(Box::new(|_, _| Err("synthetic disk write failure".into())));
        }
        let before = identity(&app);
        let recent = app.ui.recent_files.clone();
        assert!(menus::invoke(&mut app, &egui::Context::default(), "file.saveAs", json!({})).is_err());
        assert_eq!(identity(&app), before, "{failure} must not rename/retarget/mark saved or add history");
        assert_eq!(app.ui.recent_files, recent);
        assert!(writes.borrow().is_empty());
    }
}

#[test]
fn export_and_save_a_copy_keep_original_path_name_revision_and_history_v75() {
    let (mut app, _, writes) = app(Some("D:/synthetic/export.png"));
    let before = identity(&app);
    let id = crate::export_dialog::open(&mut app).unwrap();
    let fields = app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
    crate::export_dialog::confirm(&mut app, &fields).unwrap();
    assert_eq!(identity(&app), before);
    assert_eq!(writes.borrow().len(), 1);
    assert!(writes.borrow()[0].1.starts_with(b"\x89PNG\r\n\x1a\n"));
    let directory = std::env::temp_dir().join(format!(
        "inkpop-save-copy-v75-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("copy.psd");
    app.run("file.saveACopy", json!({"path":path.to_string_lossy()})).unwrap();
    assert_eq!(identity(&app), before, "explicit copy does not adopt its destination");
    assert_eq!(photocraft_io::import("copy.psd", &std::fs::read(&path).unwrap()).unwrap().document.layer_count(), 2);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

#[test]
fn capability_scoped_automation_save_also_adopts_only_after_success_v75() {
    let (mut app, picks, _) = app(None);
    let written: Writes = Rc::default();
    let captured = written.clone();
    app.services.automation_write = Some(Box::new(move |path, bytes| {
        captured.borrow_mut().push((path.to_string(), bytes.to_vec()));
        Ok(())
    }));
    app.save_automation(Some("D:/synthetic/automation.psd".into())).unwrap();
    assert!(picks.borrow().is_empty(), "automation never opens ambient file dialogs");
    assert_eq!(app.session.active().unwrap().doc.name, "automation.psd");
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("D:/synthetic/automation.psd"));
    assert!(!app.session.active().unwrap().is_dirty());
    assert!(photocraft_io::is_psd(&written.borrow()[0].1));
    app.run("layer.new.layer", json!({})).unwrap();
    let before = identity(&app);
    app.services.automation_write = Some(Box::new(|_, _| Err("synthetic denied writer".into())));
    assert!(app.save_automation(Some("D:/synthetic/rejected.psd".into())).is_err());
    assert_eq!(identity(&app), before);
}

#[test]
fn save_metadata_does_not_become_an_undo_step_or_restore_an_old_png_filename_v75() {
    let (mut app, _, _) = app(Some("D:/synthetic/history.psd"));
    invoke(&mut app, "file.saveAs");
    let id = app.session.active().unwrap().doc.id;
    app.run("edit.undo", json!({})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.id, id);
    assert_eq!(st.doc.name, "history.psd", "Undo changes content, not the adopted filename");
    assert_eq!(st.path.as_deref(), Some("D:/synthetic/history.psd"));
    assert_eq!(st.doc.layer_count(), 1, "Undo still restores the actual earlier contents");
    assert!(st.is_dirty());
    app.run("edit.redo", json!({})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.name, "history.psd");
    assert_eq!(st.doc.layer_count(), 2);
    assert_eq!(st.path.as_deref(), Some("D:/synthetic/history.psd"));
}

#[test]
fn save_adoption_refuses_another_document_and_only_cleans_the_encoded_revision_v75() {
    let (mut app, _, _) = app(None);
    let original = identity(&app);
    let id = app.session.active().unwrap().doc.id;
    app.run("layer.new.layer", json!({})).unwrap();
    app.adopt_saved_path(id, original.3, "C:\\art.v1\\晚到保存.psd").unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.name, "晚到保存.psd");
    assert_eq!(st.saved_revision, original.3);
    assert!(st.is_dirty(), "an edit newer than the encoded revision remains unsaved");
    app.run("file.new", json!({"width":2,"height":2})).unwrap();
    let second = identity(&app);
    assert!(app.adopt_saved_path(id, original.3, "wrong.psd").is_err());
    assert_eq!(identity(&app), second, "a stale completion cannot retarget another document");
}

#[test]
#[ignore = "writes actual shell tabs after PSD save; root must inspect the original PNG"]
fn saved_document_identity_gpu_visual_v75() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v75/save-identity-gpu");
    assert!(!out.exists(), "preserve prior evidence");
    std::fs::create_dir_all(&out).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_max_steps(64).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::StudioLight);
        let (mut app, _, _) = app(Some("D:/synthetic/示例画布.psd"));
        app.run("prefs.set", json!({"path":"interface.theme","value":"studioLight"})).unwrap();
        crate::prefs_ui::load(&mut app);
        app.set_theme(&cc.egui_ctx, ThemeKind::StudioLight);
        app.custom_titlebar = true;
        invoke(&mut app, "file.saveAs");
        app
    });
    h.run_steps(6);
    assert_eq!(h.state().session.active().unwrap().doc.name, "示例画布.psd");
    assert!(!h.state().session.active().unwrap().is_dirty());
    assert!(h.query_by_label_contains("source-image.png").is_none(), "old PNG identity is gone from the tab");
    h.render().unwrap().save(out.join("saved-psd-clean-tab.png")).unwrap();
}
