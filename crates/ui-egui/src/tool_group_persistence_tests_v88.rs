//! Synthetic tool-state snapshots only; never read a user's brush or preset file.

use std::sync::{Arc, Mutex};

use photocraft_engine::Session;
use serde_json::json;

use super::*;
use crate::Services;

#[derive(Default)]
struct Disk {
    text: Option<String>,
    writes: usize,
}

type SharedDisk = Arc<Mutex<Disk>>;

fn reopen(disk: &SharedDisk) -> PhotocraftApp {
    let read = disk.clone();
    let write = disk.clone();
    PhotocraftApp::new(
        Session::new(),
        Services {
            load_tool_state: Some(Box::new(move || Ok(read.lock().unwrap().text.clone()))),
            save_tool_state: Some(Box::new(move |text| {
                let mut disk = write.lock().unwrap();
                disk.text = Some(text.to_string());
                disk.writes += 1;
                Ok(())
            })),
            ..Default::default()
        },
    )
}

fn activate(app: &mut PhotocraftApp, tool: Tool) {
    crate::tool_group_memory::select(&mut app.ui, tool);
    app.sync_tool_brush();
}

fn detailed_brush() -> BrushSettings {
    let mut brush = BrushSettings {
        size: 83.0,
        pressure_size: false,
        tip: TipShape::Sampled(GrayTile::from_fn(3, 5, |x, y| (x + y * 3) as f32 / 15.0)),
        ..Default::default()
    };
    brush.dual_brush.enabled = true;
    brush.dual_brush.tip = TipShape::Sampled(GrayTile::from_fn(4, 2, |x, y| (x + y * 4) as f32 / 8.0));
    brush.dual_brush.size = 31.0;
    brush.texture.enabled = true;
    brush.texture.pattern = Pattern::Tile(GrayTile::from_fn(2, 3, |x, y| (x + y * 2) as f32 / 6.0));
    brush.texture.depth = 0.31;
    brush.mixer.wet = 0.37;
    brush.smoothing.amount = 0.23;
    brush
}

#[test]
fn v88_restart_retains_multiple_tool_groups_and_complete_brush_slots() {
    let disk = SharedDisk::default();
    let mut app = reopen(&disk);
    activate(&mut app, Tool::MixerBrush);
    let brush = detailed_brush();
    app.run("tools.setBrush", json!({"brush":brush,"summary":true})).unwrap();
    app.ui.tool_options.strength = 73.0;
    app.run("brush.presets.save", json!({"name":"v88 synthetic brush"})).unwrap();
    app.run("tools.setBrush", json!({"preset":"v88 synthetic brush","summary":true})).unwrap();
    let brush = app.session.tools.brush.clone();
    let library = app.session.tools.presets.clone();
    for tool in [Tool::PaintBucket, Tool::VerticalType, Tool::PolygonLasso, Tool::Move] {
        activate(&mut app, tool);
    }
    let expected = capture(&mut app);
    assert_eq!(expected.version, 1);
    assert_eq!(expected.tool_history.first(), Some(&Tool::Move));
    persist(&mut app, 0.0, true);
    assert_eq!(app.session.tools.presets, library, "saving group recall must not mutate the preset library");
    assert_eq!(disk.lock().unwrap().writes, 1);
    let value: Value = serde_json::from_str(disk.lock().unwrap().text.as_deref().unwrap()).unwrap();
    assert_eq!(value["version"], 1, "the additive field remains compatible with v1 state");
    assert_eq!(value["toolHistory"], json!(expected.tool_history));
    drop(app);

    let mut restored = reopen(&disk);
    assert_eq!(capture(&mut restored), expected, "group history cannot change the existing brush snapshots, owner or selections");
    for (members, remembered) in [
        (&[Tool::Gradient, Tool::PaintBucket][..], Tool::PaintBucket),
        (&[Tool::Brush, Tool::Pencil, Tool::MixerBrush][..], Tool::MixerBrush),
        (&[Tool::Type, Tool::VerticalType][..], Tool::VerticalType),
        (&[Tool::Lasso, Tool::PolygonLasso, Tool::MagneticLasso][..], Tool::PolygonLasso),
    ] {
        assert_eq!(crate::tool_group_memory::recalled(&restored.ui, members), Some(remembered));
    }
    activate(&mut restored, Tool::MixerBrush);
    assert_eq!(restored.session.tools.brush, brush, "sampled tips, dual tip, texture and mixer values survive the new field");
    assert_eq!(restored.ui.tool_options.strength, 73.0);
    assert!(
        restored
            .ui
            .tool_brush_selections
            .iter()
            .any(|(tool, selection)| *tool == Tool::MixerBrush && selection.brush.as_deref() == Some("v88 synthetic brush"))
    );
    assert!(!restored.ui.status_error, "{}", restored.ui.status);
}

#[test]
fn v88_old_v1_without_history_loads_its_brushes_and_seeds_only_the_selected_tool() {
    let disk = SharedDisk::default();
    let mut app = reopen(&disk);
    activate(&mut app, Tool::MixerBrush);
    let brush = detailed_brush();
    app.run("tools.setBrush", json!({"brush":brush,"summary":true})).unwrap();
    let brush = app.session.tools.brush.clone();
    activate(&mut app, Tool::PaintBucket);
    let expected = capture(&mut app);
    let mut legacy = serde_json::to_value(&expected).unwrap();
    legacy.as_object_mut().unwrap().remove("toolHistory");
    disk.lock().unwrap().text = Some(legacy.to_string());

    let mut restored = reopen(&disk);
    let actual = capture(&mut restored);
    assert_eq!(actual.version, 1);
    assert_eq!(actual.selected_tool, Tool::PaintBucket);
    assert_eq!(actual.owner, expected.owner);
    assert_eq!(actual.slots, expected.slots, "missing recall data must not default or replace stored brush settings");
    assert_eq!(actual.tool_history, [Tool::PaintBucket], "legacy files cannot invent unrecorded group choices");
    assert!(!restored.ui.status_error);
    activate(&mut restored, Tool::MixerBrush);
    assert_eq!(restored.session.tools.brush, brush);
}

#[test]
fn v88_history_decode_rejects_duplicates_unknown_tools_and_oversized_lists() {
    let base = json!({"version":1,"selectedTool":"Move","owner":null,"slots":[]});
    let mut valid = base.clone();
    valid["toolHistory"] = json!(Tool::ALL.as_slice());
    assert!(decode(&valid.to_string()).is_ok(), "all fifty distinct known tools fit the bounded recall list");

    let mut duplicate = base.clone();
    duplicate["toolHistory"] = json!(["PaintBucket", "Move", "PaintBucket"]);
    assert!(decode(&duplicate.to_string()).is_err());
    let mut unknown = base.clone();
    unknown["toolHistory"] = json!(["NotAnInkPopTool"]);
    assert!(decode(&unknown.to_string()).is_err());
    let mut oversized = valid;
    oversized["toolHistory"].as_array_mut().unwrap().push(json!("NotAnInkPopTool"));
    assert!(decode(&oversized.to_string()).is_err());

    let mut app = reopen(&SharedDisk::default());
    let mut snapshot = capture(&mut app);
    snapshot.tool_history = vec![Tool::PaintBucket, Tool::PaintBucket];
    assert!(validate(&snapshot).is_err(), "programmatic invalid state is rejected before the writer too");
    snapshot.tool_history = vec![Tool::Move; Tool::ALL.len() + 1];
    assert!(validate(&snapshot).is_err());
}

#[test]
fn v88_bad_history_keeps_the_previous_file_until_a_real_user_change() {
    let disk = SharedDisk::default();
    let original = json!({"version":1,"selectedTool":"Move","owner":null,"slots":[],"toolHistory":["Move","Move"]}).to_string();
    disk.lock().unwrap().text = Some(original.clone());
    let mut app = reopen(&disk);
    assert!(app.ui.status_error, "a rejected recall list is reported rather than partly loaded");
    persist(&mut app, 0.0, true);
    assert_eq!(disk.lock().unwrap().writes, 0, "opening and closing must not replace corrupt original state");
    assert_eq!(disk.lock().unwrap().text.as_deref(), Some(original.as_str()));
    activate(&mut app, Tool::PaintBucket);
    persist(&mut app, 1.0, true);
    assert_eq!(disk.lock().unwrap().writes, 1);
    assert!(decode(disk.lock().unwrap().text.as_deref().unwrap()).is_ok());
}

#[test]
fn v88_group_changes_use_the_existing_debounce_instead_of_writing_each_frame() {
    let disk = SharedDisk::default();
    let mut app = reopen(&disk);
    activate(&mut app, Tool::PaintBucket);
    assert!(persist(&mut app, 0.0, false).is_some());
    activate(&mut app, Tool::VerticalType);
    assert!(persist(&mut app, 0.5, false).is_some());
    assert_eq!(disk.lock().unwrap().writes, 0);
    assert!(persist(&mut app, 1.51, false).is_none());
    assert_eq!(disk.lock().unwrap().writes, 1);
    for step in 0..20 {
        assert!(persist(&mut app, 2.0 + step as f64 * 0.1, false).is_none());
    }
    assert_eq!(disk.lock().unwrap().writes, 1, "stable tool recall must not trigger repeated state writes");
}
