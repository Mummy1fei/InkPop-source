//! Synthetic snapshots only; no real user state or private presets are read.
#![allow(clippy::unwrap_used)]

use std::sync::{Arc, Mutex};

use photocraft_engine::{
    Session,
    preset_store::{MemBackend, open},
};
use serde_json::json;

use super::*;
use crate::Services;

#[derive(Default)]
struct Disk {
    text: Option<String>,
    writes: usize,
    fail: bool,
}

type SharedDisk = Arc<Mutex<Disk>>;

fn services(disk: &SharedDisk) -> Services {
    let read = disk.clone();
    let write = disk.clone();
    Services {
        load_tool_state: Some(Box::new(move || Ok(read.lock().unwrap().text.clone()))),
        save_tool_state: Some(Box::new(move |text| {
            let mut disk = write.lock().unwrap();
            disk.writes += 1;
            if disk.fail {
                return Err("synthetic disk failure".into());
            }
            disk.text = Some(text.into());
            Ok(())
        })),
        ..Default::default()
    }
}

fn reopen(disk: &SharedDisk) -> PhotocraftApp {
    PhotocraftApp::new(Session::new(), services(disk))
}

fn activate(app: &mut PhotocraftApp, tool: Tool) {
    app.ui.tool = tool;
    app.sync_tool_brush();
}

fn textured(size: f32, seed: u32) -> BrushSettings {
    let tile = |w, h| GrayTile::from_fn(w, h, |x, y| ((x * 137 + y * 173 + seed) % 65536) as f32 / 65535.0);
    let mut brush = BrushSettings {
        size,
        spacing: 0.13 + seed as f32 * 0.01,
        hardness: 0.23,
        tip: TipShape::Sampled(tile(5, 9)),
        pressure_size: false,
        ..Default::default()
    };
    brush.texture.enabled = true;
    brush.texture.pattern = Pattern::Tile(tile(3, 7));
    brush.texture.scale = 0.7;
    brush.texture.depth = 0.31;
    brush.texture.each_tip = true;
    brush.dual_brush.enabled = true;
    brush.dual_brush.tip = TipShape::Sampled(tile(4, 6));
    brush.dual_brush.size = size * 1.3;
    brush.dual_brush.angle = 23.0;
    brush.mixer.wet = seed as f32 * 0.11;
    brush.mixer.load = 0.23;
    brush.mixer.mix = 0.71;
    brush.mixer.multicolor = false;
    brush.smoothing.amount = 0.37;
    brush
}

fn set_brush(app: &mut PhotocraftApp, brush: &BrushSettings) {
    app.run("tools.setBrush", json!({"brush": brush, "summary": true})).unwrap();
}

fn exit_save(app: &mut PhotocraftApp) {
    struct Storage;
    impl eframe::Storage for Storage {
        fn get_string(&self, _key: &str) -> Option<String> {
            None
        }
        fn set_string(&mut self, _key: &str, _value: String) {}
        fn remove_string(&mut self, _key: &str) {}
        fn flush(&mut self) {}
    }
    eframe::App::save(app, &mut Storage);
}

#[test]
fn v73_restart_keeps_three_complete_brushes_and_latest_unswitched_edit() {
    let disk = SharedDisk::default();
    let mut app = reopen(&disk);
    let mut expected = Vec::new();
    for (i, tool) in [Tool::Brush, Tool::MixerBrush, Tool::Smudge].into_iter().enumerate() {
        activate(&mut app, tool);
        set_brush(&mut app, &textured(31.0 + i as f32 * 19.0, i as u32 + 1));
        app.ui.tool_options.strength = 23.0 + i as f32 * 17.0;
        app.ui.tool_options.sample_all_layers = i != 0;
        app.ui.tool_options.finger_painting = i == 2;
        app.ui.tool_options.protect_detail = i == 1;
        let name = format!("remember-{i}");
        app.run("brush.presets.save", json!({"name":name})).unwrap();
        app.run("tools.setBrush", json!({"preset":name,"summary":true})).unwrap();
        expected.push((tool, app.session.tools.brush.clone(), PaintingOptions::capture(&app.ui.tool_options)));
    }
    // No tool switch or tick before the real eframe exit hook.
    app.session.tools.brush.size = 113.0;
    app.ui.tool_options.strength = 87.0;
    let last = expected.last_mut().unwrap();
    last.1 = app.session.tools.brush.clone();
    last.2 = PaintingOptions::capture(&app.ui.tool_options);
    exit_save(&mut app);
    assert_eq!(disk.lock().unwrap().writes, 1);
    drop(app);
    let mut restored = reopen(&disk);
    assert_eq!(restored.ui.tool, Tool::Smudge);
    for (tool, brush, options) in expected {
        for interlude in [Tool::Move, Tool::Eyedropper] {
            activate(&mut restored, interlude);
            activate(&mut restored, tool);
            assert_eq!(restored.session.tools.brush, brush, "complete brush for {tool:?}");
            assert_eq!(PaintingOptions::capture(&restored.ui.tool_options), options);
        }
        assert!(restored.ui.tool_brush_selections.iter().any(|(t, s)| *t == tool && s.brush.is_some()));
    }
    assert!(!restored.ui.status_error, "{}", restored.ui.status);
}

#[test]
fn v73_nonpainting_options_do_not_overwrite_smudge_and_nonpainting_start_keeps_owner() {
    let disk = SharedDisk::default();
    let mut app = reopen(&disk);
    activate(&mut app, Tool::Smudge);
    set_brush(&mut app, &textured(55.0, 2));
    app.ui.tool_options.sample_all_layers = true;
    app.ui.tool_options.strength = 81.0;
    app.ui.tool_options.finger_painting = true;
    let brush = app.session.tools.brush.clone();
    let options = PaintingOptions::capture(&app.ui.tool_options);
    activate(&mut app, Tool::Move);
    app.ui.tool_options.sample_all_layers = false;
    app.ui.tool_options.strength = 9.0;
    exit_save(&mut app);
    let mut restored = reopen(&disk);
    assert_eq!(restored.ui.tool, Tool::Move);
    activate(&mut restored, Tool::Smudge);
    assert_eq!(restored.session.tools.brush, brush);
    assert_eq!(PaintingOptions::capture(&restored.ui.tool_options), options);
}

#[test]
fn v73_late_preset_library_cannot_replace_restored_snapshot_or_require_preset_presence() {
    let disk = SharedDisk::default();
    let mem = MemBackend::default();
    let mut library = Session::new();
    assert!(library.attach_preset_store(open(Box::new(mem.clone()))).is_empty());
    library.execute("brush.presets.save", json!({"name":"Late library", "brush":{"size":19.0}})).unwrap();
    let mut app = reopen(&disk);
    app.session.tools.presets = library.tools.presets.clone();
    app.run("tools.setBrush", json!({"preset":"Late library", "summary":true})).unwrap();
    set_brush(&mut app, &textured(137.0, 3));
    let brush = app.session.tools.brush.clone();
    app.run("tool.presets.new", json!({"name":"Deleted tool preset", "tool":"brush", "options":{"brush":brush}})).unwrap();
    app.run("tool.presets.select", json!({"preset":"Deleted tool preset"})).unwrap();
    // Both names can disappear; the self-contained brush must still work.
    exit_save(&mut app);
    let (tx, rx) = std::sync::mpsc::channel();
    let mut svc = services(&disk);
    svc.preset_store = Some(rx);
    let mut restored = PhotocraftApp::new(Session::new(), svc);
    assert_eq!(restored.session.tools.brush, brush);
    assert!(!restored.ui.presets_ui.selected.contains_key("toolPresets"));
    let ctx = egui::Context::default();
    crate::prefs_ui::tick(&mut restored, &ctx);
    tx.send(open(Box::new(mem))).unwrap();
    crate::prefs_ui::tick(&mut restored, &ctx);
    assert!(restored.services.preset_store.is_none());
    assert!(restored.session.tools.presets.iter().any(|p| p.name == "Late library"));
    assert_eq!(restored.session.tools.brush, brush);
    activate(&mut restored, Tool::Move);
    activate(&mut restored, Tool::Brush);
    assert_eq!(restored.session.tools.brush, brush);
}

#[test]
fn v73_brush_name_and_tool_preset_options_follow_successful_selection() {
    let disk = SharedDisk::default();
    let mut app = reopen(&disk);
    app.run("brush.presets.save", json!({"name":"Remember brush", "brush":textured(43.0, 1)})).unwrap();
    app.run("tools.setBrush", json!({"preset":"Remember brush", "summary":true})).unwrap();
    let name = app.ui.tool_brush_selections.iter().find(|(t, _)| *t == Tool::Brush).unwrap().1.clone();
    assert_eq!(name.brush.as_deref(), Some("Remember brush"));
    assert!(app.run("tools.setBrush", json!({"preset":"missing synthetic"})).is_err());
    assert_eq!(app.ui.tool_brush_selections.iter().find(|(t, _)| *t == Tool::Brush).unwrap().1, name);
    app.run(
        "tool.presets.new",
        json!({"name":"Remember smudge", "tool":"smudge", "options":{"brush":textured(67.0, 2), "toolOptions":{"strength":72.0,"finger_painting":true}}}),
    )
    .unwrap();
    app.run("tool.presets.select", json!({"preset":"Remember smudge"})).unwrap();
    assert_eq!(app.ui.tool, Tool::Smudge);
    exit_save(&mut app);
    let mut restored = reopen(&disk);
    assert_eq!(restored.session.tools.brush.size, 67.0);
    assert_eq!(restored.ui.tool_options.strength, 72.0);
    assert!(restored.ui.tool_options.finger_painting);
    activate(&mut restored, Tool::Brush);
    assert_eq!(restored.session.tools.brush.size, 43.0);
    assert_eq!(restored.ui.tool_brush_selections.iter().find(|(t, _)| *t == Tool::Brush).unwrap().1, name);
}

#[test]
fn v73_bad_state_preserves_original_and_loads_ordinary_preferences() {
    let unknown_version = json!({"version":999,"selectedTool":Tool::Brush,"owner":Tool::Brush,"slots":[]}).to_string();
    let error = decode(&unknown_version).unwrap_err();
    assert!(error.contains("unsupported or oversized tool brush state"), "{error}");
    for text in ["{broken", unknown_version.as_str()] {
        let disk = Arc::new(Mutex::new(Disk { text: Some(text.into()), ..Default::default() }));
        let mut preferences = Session::new();
        preferences.execute("prefs.set", json!({"path":"performance.historyStates", "value":17})).unwrap();
        let prefs = serde_json::to_string(&preferences.prefs_value()).unwrap();
        let mut svc = services(&disk);
        svc.load_prefs = Some(Box::new(move || Some(prefs.clone())));
        let mut app = PhotocraftApp::new(Session::new(), svc);
        assert_eq!(app.session.prefs().performance.history_states, 17);
        assert!(app.ui.status_error);
        exit_save(&mut app);
        assert_eq!(disk.lock().unwrap().writes, 0);
        assert_eq!(disk.lock().unwrap().text.as_deref(), Some(text));
        set_brush(&mut app, &textured(51.0, 2));
        exit_save(&mut app);
        assert_eq!(disk.lock().unwrap().writes, 1);
        assert!(decode(disk.lock().unwrap().text.as_deref().unwrap()).is_ok());
    }
}

#[test]
fn v73_snapshot_rejects_duplicate_owner_nonfinite_and_invalid_bitmaps() {
    let mut app = reopen(&SharedDisk::default());
    set_brush(&mut app, &textured(31.0, 1));
    let snapshot = capture(&mut app);
    assert!(validate(&snapshot).is_ok());
    assert!(decode(&serde_json::to_string(&snapshot).unwrap()).is_ok());
    let mut invalid = snapshot.clone();
    invalid.slots.push(invalid.slots[0].clone());
    assert!(validate(&invalid).is_err());
    invalid = snapshot.clone();
    invalid.owner = Some(Tool::MixerBrush);
    assert!(validate(&invalid).is_err());
    invalid = snapshot.clone();
    invalid.slots[0].brush.size = 0.0;
    assert!(validate(&invalid).is_err());
    invalid = snapshot.clone();
    invalid.slots[0].brush.texture.depth = f32::NAN;
    assert!(validate(&invalid).is_err());
    invalid = snapshot.clone();
    invalid.slots[0].brush.tip = TipShape::Sampled(GrayTile { width: 16385, height: 1, data: Arc::from([0u16]) });
    assert!(validate(&invalid).is_err());
    invalid = snapshot.clone();
    invalid.slots[0].options.strength = f32::INFINITY;
    assert!(validate(&invalid).is_err());
    invalid = snapshot;
    invalid.slots[0].tool = Tool::Move;
    assert!(validate(&invalid).is_err());
}

#[test]
fn v73_debounce_and_retry_avoid_frame_writes_and_flush_last_edit_on_exit() {
    let disk = SharedDisk::default();
    let mut app = reopen(&disk);
    for step in 0..10 {
        app.session.tools.brush.size = 30.0 + step as f32;
        assert!(persist(&mut app, step as f64 * 0.1, false).is_some());
    }
    assert_eq!(disk.lock().unwrap().writes, 0);
    assert!(persist(&mut app, 1.91, false).is_none());
    assert_eq!(disk.lock().unwrap().writes, 1);
    assert!(persist(&mut app, 3.0, false).is_none());
    assert_eq!(disk.lock().unwrap().writes, 1);
    disk.lock().unwrap().fail = true;
    app.session.tools.brush.size = 91.0;
    persist(&mut app, 4.0, false);
    assert_eq!(persist(&mut app, 5.1, false), Some(5.0));
    assert_eq!(disk.lock().unwrap().writes, 2);
    persist(&mut app, 7.0, false);
    assert_eq!(disk.lock().unwrap().writes, 2);
    disk.lock().unwrap().fail = false;
    app.session.tools.brush.size = 93.0;
    exit_save(&mut app);
    assert_eq!(disk.lock().unwrap().writes, 3);
    assert_eq!(reopen(&disk).session.tools.brush.size, 93.0);
}

#[test]
fn v73_aggregate_bitmap_limit_rejects_before_base64_and_preserves_previous_state() {
    let disk = SharedDisk::default();
    let mut app = reopen(&disk);
    set_brush(&mut app, &textured(37.0, 1));
    exit_save(&mut app);
    let previous = disk.lock().unwrap().text.clone();
    // One 32 MiB allocation shared by three fields. The state would need over
    // 128 MiB after base64, so validation must reject before serializing it.
    let tile = GrayTile { width: 4096, height: 4096, data: vec![0u16; 1 << 24].into() };
    app.session.tools.brush.tip = TipShape::Sampled(tile.clone());
    app.session.tools.brush.dual_brush.tip = TipShape::Sampled(tile.clone());
    app.session.tools.brush.texture.pattern = Pattern::Tile(tile);
    exit_save(&mut app);
    assert!(app.ui.status_error);
    assert!(app.ui.status.contains("128 MiB"));
    assert_eq!(disk.lock().unwrap().writes, 1);
    assert_eq!(disk.lock().unwrap().text, previous);
}

#[test]
fn v73_decode_bounds_slot_growth_before_constructing_extra_brushes() {
    let mut slots = vec![json!({"tool":Tool::Brush,"brush":{}}); MAX_SLOTS];
    // The element past the limit is not deserialized as BrushSettings at all.
    slots.push(json!({"tool":Tool::Brush,"brush":{"size":"not a brush size"}}));
    let text = json!({"version":1,"selectedTool":Tool::Brush,"owner":Tool::Brush,"slots":slots}).to_string();
    let error = decode(&text).unwrap_err();
    assert!(error.contains("too many tool brush slots"), "{error}");
}

#[test]
fn v73_decode_bounds_preset_and_option_strings_before_copying_into_state() {
    let mut app = reopen(&SharedDisk::default());
    let value = serde_json::to_value(capture(&mut app)).unwrap();
    for (path, length, message) in [
        ("/slots/0/selection/brush", 1025, "exceeds 1024 bytes"),
        ("/slots/0/selection/tool", 1025, "exceeds 1024 bytes"),
        ("/slots/0/options/cloneSample", 65, "exceeds 64 bytes"),
        ("/slots/0/options/bgSampling", 65, "exceeds 64 bytes"),
        ("/slots/0/options/bgLimits", 65, "exceeds 64 bytes"),
        ("/slots/0/options/toneRange", 65, "exceeds 64 bytes"),
        ("/slots/0/options/spongeMode", 65, "exceeds 64 bytes"),
    ] {
        let mut invalid = value.clone();
        *invalid.pointer_mut(path).unwrap() = json!("x".repeat(length));
        assert!(decode(&invalid.to_string()).unwrap_err().contains(message), "{path}");
    }
    assert!(decode(&value.to_string()).is_ok(), "null selections and defaults remain compatible");
}
