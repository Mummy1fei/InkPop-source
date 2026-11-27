use super::*;
use crate::state::Tool;
use egui::{Event, Key, Modifiers, RawInput};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":400,"height":300})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Type;
    app
}

fn input(app: &mut PhotocraftApp, events: Vec<Event>) {
    let ctx = egui::Context::default();
    ctx.run_ui(RawInput { events, ..Default::default() }, |ui| {
        handle_keys(app, ui.ctx());
    })
    .textures_delta
    .clear();
}

#[test]
fn type_placeholder_preference_controls_new_point_box_and_vertical_text_v78() {
    for tool in [Tool::Type, Tool::VerticalType] {
        for enabled in [false, true] {
            for end in [[50.0, 100.0], [250.0, 200.0]] {
                let mut app = app();
                app.ui.tool = tool;
                app.run("prefs.set", json!({"path":"type.fillNewTypeLayersWithPlaceholder","value":enabled})).unwrap();
                pointer_up(&mut app, [50.0, 100.0], end);
                let ed = app.ui.text_edit.clone().unwrap();
                assert_eq!(current_text(&app, LayerId(ed.layer)).unwrap(), if enabled { "Lorem ipsum" } else { "" });
                assert_eq!((ed.anchor, ed.caret), (0, if enabled { 11 } else { 0 }));
                input(&mut app, vec![Event::Text("新".into())]);
                assert_eq!(current_text(&app, LayerId(ed.layer)).unwrap(), "新", "placeholder must be replaced, not appended to");
                commit(&mut app);
                app.run("edit.undo", json!({})).unwrap();
                assert!(app.session.active().unwrap().doc.layer(LayerId(ed.layer)).is_none());
            }
        }
    }
}

#[test]
fn escape_type_preference_commits_or_cancels_the_same_edit_v78() {
    for commit_on_escape in [true, false] {
        let mut app = app();
        app.run("prefs.set", json!({"path":"type.useEscToCommit","value":commit_on_escape})).unwrap();
        pointer_up(&mut app, [50.0, 100.0], [50.0, 100.0]);
        input(&mut app, vec![Event::Text("Keep or cancel".into())]);
        let layer = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
        input(&mut app, vec![Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
        assert!(app.ui.text_edit.is_none());
        if commit_on_escape {
            assert_eq!(current_text(&app, layer).unwrap(), "Keep or cancel");
        } else {
            assert!(app.session.active().unwrap().doc.layer(layer).is_none(), "Escape cancellation must undo the new typing session");
        }
    }
}
