use super::*;
use crate::PhotocraftApp;
use crate::state::{DialogKind, UiState};
use crate::theme::ThemeKind;
use egui_kittest::{Harness, kittest::Queryable};

fn fields() -> Map<String, Value> {
    json!({"name":"Unchanged document name","width":600,"height":450,"resolution":300.0,
        "__unit":"in","__resUnit":"cm","mode":"lab","depth":16,"background":"transparent"})
    .as_object()
    .unwrap()
    .clone()
}

fn harness() -> Harness<'static, PhotocraftApp> {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, ThemeKind::StudioLight);
    h.state_mut().ui.open_dialog(DialogKind::NewDocument, fields());
    h.run_steps(4);
    h
}

#[test]
fn custom_document_preset_save_reload_apply_replace_and_delete_v71() {
    let mut h = harness();
    h.state_mut().ui.dialogs[0].fields.insert("__savePresetName".into(), json!("My print study"));
    h.run_steps(2);
    h.get_by_label("Save document preset").click();
    h.run_steps(3);
    assert_eq!(h.state().session.prefs().document_presets.len(), 1);
    assert_eq!(h.state().ui.dialogs[0].fields["__category"], "Custom");
    h.get_by_label("My print study").click();
    h.run_steps(2);
    let f = &h.state().ui.dialogs[0].fields;
    for (key, value) in fields() {
        assert_eq!(f[&key], value, "restores {key}");
    }
    assert_eq!(command_params(f)["name"], "Unchanged document name");

    let saved = h.state().session.prefs_to_json();
    let mut restored = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    restored.session.load_prefs_json(&saved).unwrap();
    let preset = restored.session.prefs().document_presets[0].clone();
    let mut fresh = UiState::new_document_fields();
    custom::apply(&mut fresh, &preset);
    for key in ["width", "height", "resolution", "__unit", "__resUnit", "mode", "depth", "background"] {
        assert_eq!(fresh[key], fields()[key], "persisted {key}");
    }
    assert_eq!(fresh["name"], "Untitled-1", "preset does not silently rename the document");
    restored.session.execute("file.new", command_params(&fresh)).unwrap();
    let doc = &restored.session.active().unwrap().doc;
    assert_eq!((doc.size.width, doc.size.height, doc.resolution_dpi), (600, 450, 300.0));

    h.state_mut().ui.dialogs[0].fields.insert("width".into(), json!(720));
    h.run_steps(2);
    h.get_by_label("Replace document preset").click();
    h.run_steps(3);
    assert_eq!(h.state().session.prefs().document_presets.len(), 1);
    assert_eq!(h.state().session.prefs().document_presets[0].width, 720);
    h.get_by_label("Delete document preset").click();
    h.run_steps(3);
    assert!(h.state().session.prefs().document_presets.is_empty());
    assert_eq!(h.state().ui.dialogs.len(), 1, "saving and deleting do not create a document or close the dialog");
    assert!(h.state().session.active().is_none());
}

#[test]
fn custom_document_presets_validate_names_and_separate_builtin_selection_v71() {
    let mut f = fields();
    assert!(custom::snapshot(&f, "   ").is_err());
    assert!(custom::snapshot(&f, "invalid\nname").is_err());
    assert!(custom::snapshot(&f, &"字".repeat(81)).is_err());
    let p = custom::snapshot(&f, "  My study  ").unwrap();
    assert_eq!(p.name, "My study");
    custom::apply(&mut f, &p);
    assert!(f.contains_key("__customPreset"));
    apply_preset(&mut f, &CATEGORIES[0].1[0]);
    assert!(!f.contains_key("__customPreset"));
    assert!(command_params(&f).as_object().unwrap().keys().all(|key| !key.starts_with("__")));
}

struct Fixture {
    app: PhotocraftApp,
}
impl eframe::App for Fixture {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let t = Tokens::get(ui.ctx());
        ui.painter().rect_filled(ui.available_rect_before_wrap(), 0.0, t.canvas);
        crate::dialogs::show(&mut self.app, ui.ctx());
    }
}

#[test]
#[ignore = "writes new-document custom preset UI screenshots for root review"]
fn new_document_presets_gpu_visual_v71() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v71/new-doc-gpu");
    assert!(!out.exists(), "preserve previous visual evidence");
    std::fs::create_dir_all(&out).unwrap();
    for kind in ThemeKind::ALL {
        let mut h = Harness::builder().with_size(vec2(1100.0, 800.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, kind);
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            let mut f = fields();
            for (name, width, height) in
                [("绘画 · 横向", 2560, 1440), ("方形 · 社交封面", 1600, 1600), ("印刷 · 竖版", 2480, 3508), ("材质练习 · 32位", 1024, 1024)]
            {
                f.insert("width".into(), json!(width));
                f.insert("height".into(), json!(height));
                let p = custom::snapshot(&f, name).unwrap();
                assert!(custom::store(&mut app, &mut f, p));
            }
            let p = app.session.prefs().document_presets[0].clone();
            custom::apply(&mut f, &p);
            f.insert("__category".into(), json!("Custom"));
            app.ui.open_dialog(DialogKind::NewDocument, f);
            Fixture { app }
        });
        let _language = crate::i18n::language_scope(crate::i18n::Lang::from_code("zh-hans").unwrap());
        h.run_steps(5);
        h.render().unwrap().save(out.join(format!("{}.png", kind.id()))).unwrap();
    }
}
