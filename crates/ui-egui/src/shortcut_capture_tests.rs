//! #292: Edit › Keyboard Shortcuts records the key pressed with the modifiers, not the
//! modifier key itself (egui reports Ctrl, Shift, Alt and ⌘ as key presses too), so Ctrl+F and
//! any other non-system combination can be bound; a shortcut already in use is reported.

use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use crate::PhotocraftApp;

fn key(h: &mut Harness<'_, PhotocraftApp>, key: Key, pressed: bool, modifiers: Modifiers) {
    h.event(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
}

/// The Keyboard Shortcuts dialog with `command` selected, filtered to it.
fn dialog(ppp: f32, command: &str, filter: &str) -> (Harness<'static, PhotocraftApp>, u64) {
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 760.0)).with_pixels_per_point(ppp).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, "edit.keyboardShortcuts", json!({})).unwrap()["dialog"].as_u64().unwrap();
    let f = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
    f.insert("filter".into(), json!(filter));
    f.insert("selected".into(), json!(command));
    h.run_steps(4);
    (h, id)
}

fn field(h: &Harness<'_, PhotocraftApp>, id: u64, name: &str) -> Value {
    h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.get(name).cloned().unwrap_or(Value::Null)
}

/// Click the selected command's shortcut button, then press `modifier` and `k` like a keyboard:
/// the modifier key goes down first (its own key event), then the key with the modifier held.
fn capture(h: &mut Harness<'_, PhotocraftApp>, button: &str, modifier: Key, held: Modifiers, k: Key) {
    h.get_by_label(button).click();
    h.run_steps(2);
    key(h, modifier, true, held);
    h.run_steps(1);
    key(h, k, true, held);
    h.run_steps(1);
    key(h, k, false, held);
    key(h, modifier, false, Modifiers::NONE);
    h.run_steps(2);
}

#[test]
fn ctrl_f_is_recorded_as_ctrl_f_not_the_control_key() {
    for ppp in [1.0, 2.0] {
        let (mut h, id) = dialog(ppp, "edit.search", "search…");
        let current = crate::shortcuts::pretty("Cmd+K");
        // On Windows and Linux the Ctrl key is the command modifier.
        capture(&mut h, &current, Key::ControlLeft, Modifiers::COMMAND, Key::F);
        assert_eq!(field(&h, id, "overrides")["edit.search"], json!("Cmd+F"), "@{ppp}x");
        assert_eq!(field(&h, id, "capture"), json!(false));
        let message = field(&h, id, "message");
        assert!(!message.as_str().unwrap().contains("ontrol"), "@{ppp}x: {message}");
        // The row shows the new shortcut.
        h.get_by_label(&crate::shortcuts::pretty("Cmd+F"));
        h.render().ok();
    }
}

#[test]
fn every_modifier_key_waits_for_the_real_key() {
    for (modifier, held) in [
        (Key::ControlRight, Modifiers::COMMAND),
        (Key::ShiftLeft, Modifiers::SHIFT),
        (Key::AltRight, Modifiers::ALT),
        (Key::SuperLeft, Modifiers::COMMAND),
        (Key::ShiftRight, Modifiers::COMMAND | Modifiers::SHIFT),
    ] {
        let (mut h, id) = dialog(1.0, "edit.search", "search…");
        h.get_by_label(&crate::shortcuts::pretty("Cmd+K")).click();
        h.run_steps(2);
        key(&mut h, modifier, true, held);
        h.run_steps(2);
        assert_eq!(field(&h, id, "capture"), json!(true), "{modifier:?} alone keeps capturing");
        assert!(field(&h, id, "overrides").get("edit.search").is_none(), "{modifier:?} alone assigns nothing");
        key(&mut h, Key::J, true, held);
        h.run_steps(2);
        let want = crate::prefs_ui::shortcut_text(Key::J, held).unwrap();
        assert_eq!(field(&h, id, "overrides")["edit.search"], json!(want), "{modifier:?}");
    }
    for k in [Key::ControlLeft, Key::ShiftLeft, Key::AltLeft, Key::SuperRight] {
        assert_eq!(crate::prefs_ui::shortcut_text(k, Modifiers::COMMAND), None, "{k:?}");
    }
}

#[test]
fn a_shortcut_in_use_is_reported_and_moved_on_ok() {
    // ⌘J is Layer › New › Layer via Copy: giving it to Search warns, and OK moves it.
    let (mut h, id) = dialog(1.0, "edit.search", "search…");
    capture(&mut h, &crate::shortcuts::pretty("Cmd+K"), Key::ControlLeft, Modifiers::COMMAND, Key::J);
    let message = field(&h, id, "message");
    assert!(message.as_str().unwrap().contains("already in use"), "{message}");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    h.run_steps(2);
    let app = h.state();
    assert_eq!(app.session.prefs().shortcut("edit.search", Some("Cmd+K")), Some("Cmd+J"));
    let bound: Vec<String> =
        crate::shortcut_dispatch::bindings(app).into_iter().filter(|(_, sc)| *sc == crate::shortcuts::parse("Cmd+J").unwrap()).map(|(id, _)| id).collect();
    assert_eq!(bound, ["edit.search"], "one owner for ⌘J");
}

#[test]
fn use_default_ends_capture_and_survives_confirmation() {
    for ppp in [1.0, 2.0] {
        let (mut h, id) = dialog(ppp, "edit.search", "search…");
        capture(&mut h, &crate::shortcuts::pretty("Cmd+K"), Key::ControlLeft, Modifiers::COMMAND, Key::F);
        h.get_by_label(&crate::shortcuts::pretty("Cmd+F")).click();
        h.run_steps(2);
        assert_eq!(field(&h, id, "capture"), json!(true));
        h.get_by_label("Use Default").click();
        h.run_steps(2);
        assert_eq!(field(&h, id, "capture"), json!(false), "@{ppp}x");
        assert!(field(&h, id, "overrides").get("edit.search").is_none());
        h.get_by_label(&crate::shortcuts::pretty("Cmd+K"));
        // The next unrelated key must not silently replace the restored default.
        key(&mut h, Key::G, true, Modifiers::NONE);
        key(&mut h, Key::G, false, Modifiers::NONE);
        h.run_steps(2);
        assert!(field(&h, id, "overrides").get("edit.search").is_none());
        h.get_by_label("OK").click();
        h.run_steps(2);
        assert_eq!(crate::shortcuts::shortcut_label(h.state(), "edit.search"), Some(crate::shortcuts::pretty("Cmd+K")));
        assert!(h.state().session.prefs().shortcuts.is_empty());
    }
}

#[test]
fn restoring_a_default_reclaims_it_from_another_override() {
    let (mut h, id) = dialog(1.0, "edit.search", "search…");
    let f = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
    f.insert("overrides".into(), json!({"edit.search": "Cmd+F", "edit.copy": "Cmd+K", "file.new": "Cmd+Alt+N"}));
    f.insert("capture".into(), json!(true));
    h.run_steps(2);
    h.get_by_label("Use Default").click();
    h.run_steps(2);
    assert_eq!(field(&h, id, "overrides")["edit.copy"], json!(""));
    assert_eq!(field(&h, id, "overrides")["file.new"], json!("Cmd+Alt+N"));
    h.get_by_label("OK").click();
    h.run_steps(2);
    let owners: Vec<_> = crate::shortcut_dispatch::bindings(h.state())
        .into_iter()
        .filter(|(_, sc)| *sc == crate::shortcuts::parse("Cmd+K").unwrap())
        .map(|(id, _)| id)
        .collect();
    assert_eq!(owners, ["edit.search"]);
    assert_eq!(crate::shortcuts::shortcut_label(h.state(), "file.new"), Some(crate::shortcuts::pretty("Cmd+Alt+N")));
}

#[test]
fn reset_and_delete_actions_end_capture_even_without_a_default() {
    for action in ["Use Default", "Delete Shortcut", "Reset All to Defaults"] {
        // No built-in shortcut: restoration must show an empty binding, not “Press keys…”.
        let (mut h, id) = dialog(1.0, "edit.puppetWarp", "puppet warp");
        assert_eq!(crate::shortcuts::default_shortcut("edit.puppetWarp"), None);
        let f = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
        f.insert("overrides".into(), json!({"edit.puppetWarp": "Cmd+Alt+F", "edit.copy": "Cmd+Alt+C"}));
        f.insert("capture".into(), json!(true));
        h.run_steps(2);
        h.get_by_label(action).click();
        h.run_steps(2);
        assert_eq!(field(&h, id, "capture"), json!(false), "{action}");
        key(&mut h, Key::J, true, Modifiers::NONE);
        key(&mut h, Key::J, false, Modifiers::NONE);
        h.run_steps(2);
        let overrides = field(&h, id, "overrides");
        if action == "Delete Shortcut" {
            assert_eq!(overrides["edit.puppetWarp"], json!(""));
        } else {
            assert!(overrides.get("edit.puppetWarp").is_none());
        }
        if action == "Reset All to Defaults" {
            assert_eq!(overrides, json!({}));
            h.get_by_label("OK").click();
            h.run_steps(2);
            assert!(h.state().session.prefs().shortcuts.is_empty());
            assert_eq!(crate::shortcuts::shortcut_label(h.state(), "edit.copy"), Some(crate::shortcuts::pretty("Cmd+C")));
        }
    }
}

#[test]
fn restoring_tool_defaults_preserves_shared_groups_and_cancel_is_transactional() {
    let (mut h, id) = dialog(1.0, "tools.select.Brush", "tools.select.Brush");
    let before = h.state().session.prefs().to_json();
    let f = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
    f.insert("overrides".into(), json!({"tools.select.Brush": "G", "tools.select.Pencil": "B"}));
    f.insert("capture".into(), json!(true));
    h.run_steps(2);
    h.get_by_label("Use Default").click();
    h.run_steps(2);
    assert_eq!(field(&h, id, "overrides"), json!({}), "redundant shared-key override must not disable the restored brush on OK");
    let fields = h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert_eq!(h.state().session.prefs().to_json(), before, "Cancel keeps saved preferences");
    // Confirm the same draft separately to verify both tools still belong to the B group.
    super::confirm(h.state_mut(), &fields).unwrap();
    for tool in ["tools.select.Brush", "tools.select.Pencil"] {
        assert_eq!(crate::shortcuts::shortcut_label(h.state(), tool).as_deref(), Some("B"));
    }
}

#[test]
#[ignore = "writes v61 GPU screenshots of shortcut modification, capture, and restored default"]
fn shortcut_default_gpu_visual_v61() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v61");
    std::fs::create_dir_all(&out).unwrap();
    for theme in [crate::theme::ThemeKind::ProMedium, crate::theme::ThemeKind::StudioLight] {
        let mut h = Harness::builder().with_size(egui::vec2(1100.0, 760.0)).wgpu().with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, theme);
            let mut session = photocraft_engine::Session::new();
            session.execute("prefs.set", json!({"path":"interface.theme","value":theme.id()})).unwrap();
            PhotocraftApp::new(session, crate::Services::default())
        });
        assert_eq!(h.state().ui.theme, theme, "the app must not replace the requested screenshot theme with its default");
        let ctx = h.ctx.clone();
        let id = crate::menus::invoke(h.state_mut(), &ctx, "edit.keyboardShortcuts", json!({})).unwrap()["dialog"].as_u64().unwrap();
        let f = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
        f.insert("filter".into(), json!("content-aware scale"));
        f.insert("selected".into(), json!("edit.contentAwareScale"));
        h.run_steps(4);
        capture(&mut h, &crate::shortcuts::pretty("Cmd+Alt+Shift+C"), Key::ControlLeft, Modifiers::COMMAND, Key::F);
        let save = |h: &mut Harness<'_, PhotocraftApp>, phase: &str| {
            let path = out.join(format!("shortcut-default-{}-{phase}.png", theme.id()));
            assert!(!path.exists(), "preserve previous evidence: {}", path.display());
            h.render().unwrap().save(path).unwrap();
        };
        save(&mut h, "modified");
        h.get_by_label(&crate::shortcuts::pretty("Cmd+F")).click();
        h.run_steps(3);
        assert_eq!(field(&h, id, "capture"), json!(true));
        save(&mut h, "capture");
        h.get_by_label("Use Default").click();
        h.run_steps(3);
        assert_eq!(field(&h, id, "capture"), json!(false));
        assert!(field(&h, id, "overrides").get("edit.contentAwareScale").is_none());
        h.get_by_label(&crate::shortcuts::pretty("Cmd+Alt+Shift+C"));
        save(&mut h, "restored");
        h.get_by_label("OK").click();
        h.run_steps(2);
        assert_eq!(crate::shortcuts::shortcut_label(h.state(), "edit.contentAwareScale"), Some(crate::shortcuts::pretty("Cmd+Alt+Shift+C")));
    }
}
