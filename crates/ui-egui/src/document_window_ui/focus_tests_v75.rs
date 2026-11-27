use super::*;
use serde_json::json;

fn fixture() -> (PhotocraftApp, egui::Context) {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    for (name, color) in [("Visible red document", "#d14735"), ("Hidden blue document", "#2364c9"), ("Visible green document", "#38a365")] {
        app.run("file.new", json!({"name":name,"width":320,"height":240})).unwrap();
        app.run("layer.setProps", json!({"name":name})).unwrap();
        app.run("edit.fill", json!({"color":color})).unwrap();
    }
    app.sync_views();
    (app, ctx)
}

fn minimize(app: &mut PhotocraftApp, ctx: &egui::Context, window: u64) {
    request_manual_minimize(ctx, ViewportId::from_hash_of(("docwin", window)), window);
    reconcile(app, ctx);
}

#[test]
fn caption_minimize_hands_focus_to_root_mru_and_stale_focused_body_cannot_reclaim_v75() {
    let (mut app, ctx) = fixture();
    app.session.set_active(2);
    assert_eq!(crate::canvas::root_document_v74::index(&app, &ctx), Some(2));
    let window = crate::view_cmds::float_window(&mut app, 1, 0);
    app.session.set_active(1);
    let revisions: Vec<_> = app.session.documents().iter().map(|state| state.revision).collect();
    minimize(&mut app, &ctx, window);
    assert_eq!(app.session.active_index(), Some(2));
    assert_eq!(app.session.active().unwrap().doc.name, "Visible green document");
    let at = pos2(250.0, 200.0);
    let mut input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0))),
        events: vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE }],
        ..Default::default()
    };
    input.viewports.entry(ViewportId::ROOT).or_default().focused = Some(true);
    let mut output = ctx.run_ui(input, |ui| super::super::body(&mut app, ui, 1, true));
    output.textures_delta.clear();
    assert_eq!(app.session.active_index(), Some(2), "stale native focus and pointer must not reclaim a minimized document");
    assert_eq!(app.session.documents().iter().map(|state| state.revision).collect::<Vec<_>>(), revisions);
    assert!(app.drag.is_none() && app.live_stroke.is_none());
}

#[test]
fn native_system_minimize_hands_off_without_native_restore_commands_v75() {
    let (mut app, ctx) = fixture();
    ctx.set_embed_viewports(false);
    let window = crate::view_cmds::float_window(&mut app, 1, 0);
    let viewport = ViewportId::from_hash_of(("docwin", window));
    app.session.set_active(1);
    ctx.data_mut(|data| data.insert_temp(native_restore_request_id(viewport), 3_u8));
    let mut input = egui::RawInput::default();
    input.viewports.entry(viewport).or_default().minimized = Some(true);
    let mut output = ctx.run_ui(input, |_| {
        ctx.show_viewport_deferred(viewport, detached_viewport_builder("Native minimize"), |_, _| {});
        keep_detached_viewport_visible(&ctx, viewport, window);
        reconcile(&mut app, &ctx);
    });
    assert_eq!(app.session.active_index(), Some(0));
    assert!(manually_minimized(&ctx, window));
    assert!(
        !output.viewport_output[&viewport]
            .commands
            .iter()
            .any(|command| { matches!(command, ViewportCommand::Minimized(false) | ViewportCommand::Visible(true) | ViewportCommand::Focus) })
    );
    output.textures_delta.clear();
}

#[test]
fn whole_application_minimize_preserves_editor_target_v75() {
    let (mut app, ctx) = fixture();
    let window = crate::view_cmds::float_window(&mut app, 1, 0);
    app.session.set_active(1);
    let mut input = egui::RawInput::default();
    input.viewports.entry(ViewportId::ROOT).or_default().minimized = Some(true);
    let mut output = ctx.run_ui(input, |_| {
        root_hidden(&app, &ctx);
        reconcile(&mut app, &ctx);
        assert!(!available(&app, &ctx, 1));
    });
    assert_eq!(app.session.active_index(), Some(1));
    assert!(!manually_minimized(&ctx, window));
    output.textures_delta.clear();
}

#[test]
fn minimize_chooses_last_visible_floating_document_by_identity_v75() {
    let (mut app, ctx) = fixture();
    let mut windows = Vec::new();
    for document in 0..3 {
        windows.push(crate::view_cmds::float_window(&mut app, document, 0));
        remember(&app, &ctx, document);
    }
    app.session.set_active(2);
    minimize(&mut app, &ctx, windows[2]);
    assert_eq!(app.session.active_index(), Some(1));
    // Close a preceding document: the focus MRU must not confuse shifted indices.
    app.run("file.close", json!({"document":0})).unwrap();
    app.sync_views();
    app.session.set_active(1);
    reconcile(&mut app, &ctx);
    assert_eq!(app.session.active_index(), Some(0));
    assert_eq!(app.session.active().unwrap().doc.name, "Hidden blue document");
}

#[test]
fn all_minimized_retains_last_target_for_explicit_restore_without_canvas_input_v75() {
    let (mut app, ctx) = fixture();
    let windows: Vec<_> = (0..3).map(|index| crate::view_cmds::float_window(&mut app, index, 0)).collect();
    for window in windows {
        minimize(&mut app, &ctx, window);
    }
    assert_eq!(app.session.active_index(), Some(2));
    assert!(!available(&app, &ctx, 2));
    let window = app.ui.windows.iter().find(|window| window.document == 2).unwrap().id;
    restore_manual_from_menu(&ctx, window);
    assert!(available(&app, &ctx, 2));
}

#[test]
fn modal_dialog_defers_focus_handoff_until_its_document_transaction_finishes_v75() {
    let (mut app, ctx) = fixture();
    let window = crate::view_cmds::float_window(&mut app, 1, 0);
    app.session.set_active(1);
    let dialog = app.ui.open_dialog(crate::state::DialogKind::About, Default::default());
    minimize(&mut app, &ctx, window);
    assert_eq!(app.session.active_index(), Some(1));
    app.ui.close_dialog(dialog);
    reconcile(&mut app, &ctx);
    assert_eq!(app.session.active_index(), Some(0));
}

#[test]
fn native_minimize_finishes_interrupted_stroke_in_original_document_v75() {
    let (mut app, ctx) = fixture();
    let window = crate::view_cmds::float_window(&mut app, 1, 0);
    app.session.set_active(1);
    app.ui.tool = crate::state::Tool::Brush;
    app.sync_tool_brush();
    app.run("tools.setBrush", json!({"brush":{"size":12,"hardness":1.0,"spacing":0.1}})).unwrap();
    app.drag =
        Some(crate::canvas::Drag::new(crate::state::Tool::Brush, [40.0, 40.0], vec![[40.0, 40.0, 1.0], [100.0, 80.0, 1.0]], egui::Modifiers::NONE, false));
    let revisions: Vec<_> = app.session.documents().iter().map(|state| state.revision).collect();
    minimize(&mut app, &ctx, window);
    assert!(app.drag.is_none() && app.live_stroke.is_none(), "a lost release must not leave editing stuck");
    assert_eq!(app.session.active_index(), Some(0));
    assert_eq!(app.session.documents()[0].revision, revisions[0]);
    assert!(app.session.documents()[1].revision > revisions[1]);
    assert_eq!(app.session.documents()[2].revision, revisions[2]);
}

#[test]
#[ignore = "writes five-theme ROOT canvas and production Layers after child minimization"]
fn minimized_document_layers_gpu_visual_v75() {
    struct Canvas(PhotocraftApp);
    impl eframe::App for Canvas {
        fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
            reconcile(&mut self.0, ui.ctx());
            egui::Panel::right("layers-focus-v75").exact_size(300.0).show(ui, |ui| crate::panels::dock_body(&mut self.0, ui, crate::dock::Group::Layers, 0));
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(&mut self.0, ui));
        }
    }
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v75/minimized-document-gpu");
    assert!(!out.exists(), "preserve earlier candidates");
    std::fs::create_dir_all(&out).unwrap();
    let mut manifest = Vec::new();
    for kind in crate::theme::ThemeKind::ALL {
        let mut h =
            egui_kittest::Harness::builder().with_size(vec2(1100.0, 760.0)).with_max_steps(64).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(
                move |cc| {
                    let (mut app, _) = fixture();
                    PhotocraftApp::setup_context(&cc.egui_ctx, kind);
                    app.run("prefs.set", json!({"path":"interface.theme","value":kind.id()})).unwrap();
                    crate::prefs_ui::load(&mut app);
                    app.set_theme(&cc.egui_ctx, kind);
                    app.session.set_active(0);
                    app.ui.extras.rulers = false;
                    Canvas(app)
                },
            );
        h.run_steps(5);
        let window = crate::view_cmds::float_window(&mut h.state_mut().0, 1, 0);
        h.state_mut().0.session.set_active(1);
        h.run_steps(3);
        let ctx = h.ctx.clone();
        minimize(&mut h.state_mut().0, &ctx, window);
        h.run_steps(4);
        assert_eq!(h.state().0.session.active_index(), Some(0));
        assert_eq!(h.state().0.ui.theme, kind);
        assert_eq!(crate::theme::Tokens::get(&h.ctx).kind, kind);
        assert_eq!(h.state().0.session.prefs_value()["interface"]["theme"], json!(kind.id()));
        let file = format!("{}-root-and-layers-after-minimize.png", kind.id());
        h.render().unwrap().save(out.join(&file)).unwrap();
        manifest.push(json!({"file":file,"requestedTheme":kind.id(),"actualTheme":h.state().0.ui.theme.id(),"activeDocument":h.state().0.session.active().unwrap().doc.name}));
    }
    std::fs::write(out.join("theme-manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
}
