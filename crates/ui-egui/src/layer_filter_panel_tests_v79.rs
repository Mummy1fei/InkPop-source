//! Real compact Layers panel pointer/keyboard regressions in all five themes.
use super::*;
use crate::layer_filter_ui::{Mode, Options};
use crate::theme::ThemeKind;
use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable};

struct Panel {
    app: PhotocraftApp,
}
impl eframe::App for Panel {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| layers(&mut self.app, ui));
    }
}

fn harness(app: PhotocraftApp, theme: ThemeKind) -> Harness<'static, Panel> {
    let mut h = Harness::builder().with_size(vec2(340.0, 680.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        Panel { app }
    });
    h.run_steps(6);
    h
}

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":32,"height":24,"background":"transparent"})).unwrap();
    app
}

fn visible(h: &Harness<'_, Panel>) -> Vec<u64> {
    crate::layer_row_ui::recorded(&h.ctx).iter().map(|row| row.layer).collect()
}

#[test]
fn name_filter_real_typing_and_enable_switch_find_collapsed_children_without_document_edits_v79() {
    for theme in ThemeKind::ALL {
        let mut app = app();
        let needle = app.run("layer.new.layer", json!({"name":"Needle"})).unwrap()["layer"].as_u64().unwrap();
        let group = app.run("layer.groupLayers", json!({"name":"Folder"})).unwrap()["layer"].as_u64().unwrap();
        app.run("layer.setExpanded", json!({"layer":group,"expanded":false})).unwrap();
        app.run("layer.new.layer", json!({"name":"Unrelated"})).unwrap();
        app.ui.layer_filter_options = Options { mode: Mode::Name, ..Default::default() };
        let before = app.session.active().unwrap().doc.clone();
        let selected = app.session.active().unwrap().selected_layers();
        let history = app.session.active().unwrap().history.past_len();
        let mut h = harness(app, theme);
        h.get_by_role(Role::TextInput).click();
        h.run_steps(1);
        h.event(egui::Event::Text("needle".into()));
        h.run_steps(3);
        assert_eq!(visible(&h), [group, needle], "{theme:?}: keep ancestor row and find descendant inside collapsed folder");
        assert_eq!(h.state().app.session.active().unwrap().doc.layers, before.layers);
        assert_eq!(h.state().app.session.active().unwrap().selected_layers(), selected);
        assert_eq!(h.state().app.session.active().unwrap().history.past_len(), history);
        h.get_by_label(tl!("Enable layer filtering")).click();
        h.run_steps(3);
        assert!(!h.state().app.ui.layer_filter_options.enabled);
        assert!(!visible(&h).contains(&needle), "original collapsed group returns when filter disabled");
        assert_eq!(h.state().app.ui.layer_filter_options.query, "needle");
        assert_eq!(h.state().app.session.active().unwrap().doc.layers, before.layers);
        h.get_by_label(tl!("Enable layer filtering")).click();
        h.run_steps(3);
        assert_eq!(visible(&h), [group, needle]);
    }
}

#[test]
fn kind_icons_and_mode_dropdown_are_interactive_in_every_theme_v79() {
    for theme in ThemeKind::ALL {
        let mut app = app();
        let text = app.run("type.create", json!({"text":"Text","x":2,"y":10})).unwrap()["layer"].as_u64().unwrap();
        app.run("layer.new.layer", json!({"name":"Pixel"})).unwrap();
        let mut h = harness(app, theme);
        h.get_by_label(tl!("Filter for type layers")).click();
        h.run_steps(3);
        assert_eq!(visible(&h), [text]);
        assert_eq!(h.state().app.ui.layer_filter, ["type"]);
        h.query_all_by_role(Role::ComboBox).find(|node| node.value().as_deref() == Some(tl!("Kind"))).expect("Kind filter combobox").click();
        h.run_steps(2);
        h.get_by_label(tl!("Name")).click();
        h.run_steps(3);
        assert_eq!(h.state().app.ui.layer_filter_options.mode, Mode::Name);
        assert_eq!(h.query_all_by_role(Role::TextInput).count(), 1);
        assert_eq!(h.state().app.ui.layer_filter, ["type"], "changing criterion preserves existing kind choices");
    }
}

#[test]
fn five_lock_controls_toggle_whole_selection_and_resolve_mixed_own_bits_v79() {
    for theme in ThemeKind::ALL {
        let mut app = app();
        let a = app.run("layer.new.layer", json!({"name":"A"})).unwrap()["layer"].as_u64().unwrap();
        let b = app.run("layer.new.layer", json!({"name":"B"})).unwrap()["layer"].as_u64().unwrap();
        app.run("layer.setProps", json!({"layer":a,"locks":{"transparency":true}})).unwrap();
        app.run("layer.select", json!({"layer":a})).unwrap();
        app.run("layer.select", json!({"layer":b,"mode":"toggle"})).unwrap();
        let mut h = harness(app, theme);
        for (key, label) in [
            ("transparency", "Lock transparent pixels"),
            ("pixels", "Lock image pixels"),
            ("position", "Lock position"),
            ("artboard", "Prevent auto-nesting in and out of Artboards and Frames"),
            ("all", "Lock all"),
        ] {
            let doc = h.state().app.session.active().unwrap().doc.clone();
            let ids = [LayerId(a), LayerId(b)];
            if key == "transparency" {
                assert!(crate::layer_filter_ui::lock_state(&doc, &ids, key).mixed);
            }
            h.get_by_label(tl!(label)).click();
            h.run_steps(3);
            assert!(crate::layer_filter_ui::lock_state(&h.state().app.session.active().unwrap().doc, &ids, key).all_on, "{theme:?}/{key}");
            h.get_by_label(tl!(label)).click();
            h.run_steps(3);
            let state = crate::layer_filter_ui::lock_state(&h.state().app.session.active().unwrap().doc, &ids, key);
            assert!(!state.all_on && !state.mixed, "second click clears all own bits");
        }
    }
}

#[test]
fn drag_in_filtered_rows_uses_real_target_layer_id_and_preserves_omitted_layers_v79() {
    let mut app = app();
    let a = app.run("layer.new.layer", json!({"name":"Match A"})).unwrap()["layer"].as_u64().unwrap();
    let hidden = app.run("layer.new.layer", json!({"name":"Omitted middle"})).unwrap()["layer"].as_u64().unwrap();
    let b = app.run("layer.new.layer", json!({"name":"Match B"})).unwrap()["layer"].as_u64().unwrap();
    let outside = app.run("layer.new.layer", json!({"name":"Omitted top"})).unwrap()["layer"].as_u64().unwrap();
    app.run("layer.select", json!({"layer":a})).unwrap();
    app.ui.layer_filter_options = Options { mode: Mode::Name, query: "Match".into(), ..Default::default() };
    let before = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    let mut h = harness(app, ThemeKind::StudioLight);
    assert_eq!(visible(&h), [b, a]);
    let rows = crate::layer_row_ui::recorded(&h.ctx);
    let source = rows.iter().find(|row| row.layer == a).unwrap().name.unwrap().center();
    let target_row = rows.iter().find(|row| row.layer == b).unwrap().row;
    let target = pos2(source.x, target_row.top() + target_row.height() * 0.12);
    let event = |h: &mut Harness<'_, Panel>, p, pressed: Option<bool>| {
        h.event(egui::Event::PointerMoved(p));
        if let Some(pressed) = pressed {
            h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        }
        h.run_steps(1);
    };
    event(&mut h, source, None);
    event(&mut h, source, Some(true));
    event(&mut h, source + vec2(10.0, 0.0), None);
    assert!(h.ctx.data(|data| data.get_temp::<u64>(egui::Id::new("layer-drag"))).is_some());
    for step in 1..=5 {
        event(&mut h, source + (target - source) * (step as f32 / 5.0), None);
    }
    event(&mut h, target, Some(false));
    h.run_steps(3);
    let state = h.state().app.session.active().unwrap();
    assert_eq!(state.history.past_len(), history + 1);
    let order: Vec<_> = state.doc.layers.iter().map(|layer| layer.id.0).collect();
    assert_eq!(order, [before.layers[0].id.0, hidden, b, a, outside], "drop anchors against B's actual id, not filtered index zero");
    assert_eq!(state.doc.layer(LayerId(hidden)), before.layer(LayerId(hidden)));
    assert_eq!(state.doc.layer(LayerId(outside)), before.layer(LayerId(outside)));
    assert!(h.state_mut().app.session.undo());
    assert_eq!(h.state().app.session.active().unwrap().doc, before);
}

#[test]
fn artboard_filter_fallback_restores_real_folder_disclosure_interaction_v79() {
    for theme in ThemeKind::ALL {
        let mut app = app();
        let outside = app.session.active().unwrap().doc.layers[0].id.0;
        let child = app.run("layer.new.layer", json!({"name":"Board child"})).unwrap()["layer"].as_u64().unwrap();
        let board = app.run("layer.groupLayers", json!({"name":"Board"})).unwrap()["layer"].as_u64().unwrap();
        {
            let state = app.session.active_mut().unwrap();
            let document = std::sync::Arc::make_mut(&mut state.doc);
            let layer = document.layer_mut(LayerId(board)).unwrap();
            if let photocraft_doc::LayerContent::Group(group) = &mut layer.content {
                group.artboard = Some(photocraft_doc::Artboard::new(photocraft_geom::Rect::new(0, 0, 32, 24)));
            }
        }
        app.run("layer.select", json!({"layer":outside})).unwrap();
        app.run("layer.setExpanded", json!({"layer":board,"expanded":false})).unwrap();
        app.ui.layer_filter_options = Options { mode: Mode::Artboard, ..Default::default() };
        let mut h = harness(app, theme);
        assert!(!visible(&h).contains(&child));
        h.get_by_label(&format!("{} group Board", tl!("Expand"))).click();
        h.run_steps(3);
        assert!(visible(&h).contains(&child), "fallback is an ordinary interactive folder, not a forced-open filtered context");
        h.state_mut().app.run("layer.select", json!({"layer":child})).unwrap();
        h.run_steps(3);
        assert_eq!(visible(&h), [board, child]);
        let state = h.state().app.session.active().unwrap();
        let matches = crate::layer_filter_ui::artboard_match_ids(&state.doc, &state.selected_layers()).unwrap();
        assert!(matches.contains(&LayerId(child)), "board contents are actual matches, not dimmed ancestor context");
    }
}
