use super::*;
use egui::{Event, Modifiers, PointerButton};
use egui_kittest::Harness;

fn harness(kind: crate::theme::ThemeKind, gpu: bool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"name":"Layer Alt drag","width":64,"height":48,"background":"transparent"})).unwrap();
    for (name, color) in [("A", "#d74430"), ("B", "#3377dd"), ("C", "#44aa66")] {
        app.run("layer.new.layer", json!({"name":name})).unwrap();
        app.run("edit.fill", json!({"color":color})).unwrap();
    }
    app.sync_views();
    let builder = Harness::builder().with_size(vec2(480.0, 760.0)).with_step_dt(1.0 / 60.0).with_max_steps(64);
    let builder = if gpu { builder.wgpu_setup(crate::gpu_canvas::wgpu_setup()) } else { builder };
    let mut h = builder.build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| layers(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, kind);
    h.state_mut().run("prefs.set", json!({"path":"interface.theme","value":kind.id()})).unwrap();
    crate::prefs_ui::load(h.state_mut());
    let ctx = h.ctx.clone();
    h.state_mut().set_theme(&ctx, kind);
    h.run_steps(6);
    h
}

fn id(h: &Harness<'_, PhotocraftApp>, name: &str) -> LayerId {
    h.state().session.active().unwrap().doc.walk().iter().find(|(_, _, layer)| layer.name == name).unwrap().2.id
}

fn row(h: &Harness<'_, PhotocraftApp>, layer: LayerId) -> crate::layer_row_ui::RowRects {
    crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|row| row.layer == layer.0).unwrap()
}

fn event(h: &mut Harness<'_, PhotocraftApp>, point: egui::Pos2, pressed: Option<bool>, alt: bool) {
    let modifiers = if alt { Modifiers::ALT } else { Modifiers::NONE };
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerMoved(point));
    if let Some(pressed) = pressed {
        h.event(Event::PointerButton { pos: point, button: PointerButton::Primary, pressed, modifiers });
    }
    h.run_steps(1);
}

fn start(h: &mut Harness<'_, PhotocraftApp>, source: LayerId, alt: bool) -> egui::Pos2 {
    let source = row(h, source);
    let from = source.name.unwrap().center();
    event(h, from, None, alt);
    event(h, from, Some(true), alt);
    event(h, from + vec2(10.0, 0.0), None, alt);
    assert!(h.ctx.data(|data| data.get_temp::<u64>(egui::Id::new("layer-drag"))).is_some(), "real row drag armed");
    from
}

fn drop_on(h: &mut Harness<'_, PhotocraftApp>, from: egui::Pos2, target: LayerId, fraction: f32, alt: bool) {
    let target = row(h, target).row;
    let to = pos2(from.x, target.top() + target.height() * fraction);
    for step in 1..=5 {
        event(h, from + (to - from) * (step as f32 / 5.0), None, alt);
    }
    event(h, to, Some(false), alt);
    h.run_steps(3);
}

#[test]
fn layer_alt_drag_v75_real_row_pointer_uses_alt_at_drop_and_preserves_ordinary_reorder() {
    for (press_alt, drop_alt) in [(false, false), (true, true), (false, true), (true, false)] {
        let mut h = harness(crate::theme::ThemeKind::StudioLight, false);
        let a = id(&h, "A");
        let c = id(&h, "C");
        let before = h.state().session.active().unwrap().doc.clone();
        let history = h.state().session.active().unwrap().history.past_len();
        let from = start(&mut h, a, press_alt);
        drop_on(&mut h, from, c, 0.12, drop_alt);
        let state = h.state().session.active().unwrap();
        assert_eq!(state.history.past_len(), history + 1, "one complete drag is one undo");
        assert_eq!(state.doc.layers.len(), before.layers.len() + usize::from(drop_alt));
        if drop_alt {
            assert_eq!(state.doc.layer(a), before.layer(a), "Alt copies must leave the original intact");
            let copy = state.doc.layers.last().unwrap();
            assert_eq!(copy.name, "A copy");
            assert_ne!(copy.id, a);
            assert_eq!(copy.content, before.layer(a).unwrap().content);
            assert_eq!(state.selected_layers(), vec![copy.id]);
        } else {
            assert_eq!(state.doc.layers.last().unwrap().id, a);
            assert!(!state.doc.layers.iter().any(|layer| layer.name.ends_with(" copy")));
        }
        assert!(h.state_mut().session.undo());
        assert_eq!(h.state().session.active().unwrap().doc, before);
    }
}

#[test]
fn layer_alt_drag_v75_escape_focus_loss_and_drop_outside_cancel_without_mutation() {
    for cancel in ["escape", "focus", "outside"] {
        let mut h = harness(crate::theme::ThemeKind::ProMedium, false);
        let a = id(&h, "A");
        let c = id(&h, "C");
        let before = h.state().session.active().unwrap().doc.clone();
        let history = h.state().session.active().unwrap().history.past_len();
        let from = start(&mut h, a, true);
        if cancel == "escape" {
            h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::ALT });
            h.run_steps(1);
            drop_on(&mut h, from, c, 0.12, true);
        } else if cancel == "focus" {
            h.event(Event::WindowFocused(false));
            h.run_steps(1);
            assert!(h.ctx.data(|data| data.get_temp::<u64>(egui::Id::new("layer-drag"))).is_none());
            h.event(Event::WindowFocused(true));
            h.run_steps(1);
            drop_on(&mut h, from, c, 0.12, true);
        } else {
            event(&mut h, pos2(800.0, 500.0), Some(false), true);
            h.run_steps(3);
        }
        assert_eq!(h.state().session.active().unwrap().doc, before, "{cancel}");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
        assert!(h.ctx.data(|data| data.get_temp::<u64>(egui::Id::new("layer-drag"))).is_none());
        assert!(crate::layer_transfer::pointer_if_armed(h.state(), &h.ctx).is_none(), "a cancelled panel drag cannot later copy to another document");
    }
}

#[test]
fn layer_alt_drag_v75_real_multi_row_copy_into_folder_preserves_sources_and_order() {
    let mut h = harness(crate::theme::ThemeKind::Classic, false);
    let a = id(&h, "A");
    let b = id(&h, "B");
    h.state_mut().run("layer.select", json!({"layer":a.0})).unwrap();
    h.state_mut().run("layer.select", json!({"layer":b.0,"mode":"add"})).unwrap();
    let folder = LayerId(h.state_mut().run("layer.new.group", json!({"name":"Target folder","empty":true})).unwrap()["layer"].as_u64().unwrap());
    h.state_mut().run("layer.select", json!({"layer":a.0})).unwrap();
    h.state_mut().run("layer.select", json!({"layer":b.0,"mode":"add"})).unwrap();
    h.run_steps(3);
    let before = h.state().session.active().unwrap().doc.clone();
    let from = start(&mut h, a, true);
    drop_on(&mut h, from, folder, 0.5, true);
    let state = h.state().session.active().unwrap();
    for source in [a, b] {
        assert_eq!(state.doc.layer(source), before.layer(source));
    }
    let children = state.doc.layer(folder).unwrap().children().unwrap();
    assert_eq!(children.iter().map(|layer| layer.name.as_str()).collect::<Vec<_>>(), ["A copy", "B copy"]);
    assert_eq!(state.selected_layers(), children.iter().map(|layer| layer.id).collect::<Vec<_>>());
    assert!(h.state_mut().session.undo());
    assert_eq!(h.state().session.active().unwrap().doc, before);
}

#[test]
#[ignore = "writes five themes of actual Alt row dragging and copied Layers panel"]
fn layer_alt_drag_gpu_visual_v75() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v75/layer-alt-drag-gpu");
    assert!(!out.exists(), "preserve earlier visual candidates");
    std::fs::create_dir_all(&out).unwrap();
    let mut manifest = Vec::new();
    for kind in crate::theme::ThemeKind::ALL {
        let mut h = harness(kind, true);
        let a = id(&h, "A");
        let c = id(&h, "C");
        let from = start(&mut h, a, true);
        let target = row(&h, c).row;
        let to = pos2(from.x, target.top() + target.height() * 0.12);
        event(&mut h, to, None, true);
        h.run_steps(2);
        for phase in ["drag", "copied"] {
            if phase == "copied" {
                event(&mut h, to, Some(false), true);
                h.run_steps(3);
            }
            assert_eq!(h.state().ui.theme, kind);
            assert_eq!(Tokens::get(&h.ctx).kind, kind);
            assert_eq!(h.state().session.prefs_value()["interface"]["theme"], json!(kind.id()));
            let file = format!("{}-{phase}.png", kind.id());
            h.render().unwrap().save(out.join(&file)).unwrap();
            manifest.push(json!({"file":file,"phase":phase,"requestedTheme":kind.id(),"actualTheme":h.state().ui.theme.id(),"tokenTheme":Tokens::get(&h.ctx).kind.id(),"preferenceTheme":h.state().session.prefs_value()["interface"]["theme"]}));
        }
    }
    std::fs::write(out.join("theme-manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
}
