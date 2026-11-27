//! Exercise the transition ordering, including the first stroke with no frame
//! between selecting a tool/preset and pen-down.
#![allow(clippy::unwrap_used)]

use photocraft_engine::paint::{BrushSettings, GrayTile, TipShape};
use serde_json::{Value, json};

use crate::{ControlRequest, PhotocraftApp, Services, Tool};

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), Services::default())
}

fn textured(size: f32) -> BrushSettings {
    let mut brush = BrushSettings {
        size,
        hardness: 0.17,
        spacing: 0.11,
        tip: TipShape::Sampled(GrayTile::from_fn(7, 9, |x, y| if x == 2 || (y == 5 && x < 5) { 1.0 } else { 0.0 })),
        pressure_size: false,
        ..Default::default()
    };
    brush.smoothing.amount = 0.0;
    brush.texture.enabled = true;
    brush.texture.pattern = photocraft_engine::paint::brush::Pattern::Tile(GrayTile::from_fn(3, 4, |x, y| if (x + y) % 2 == 0 { 0.6 } else { 1.0 }));
    brush.texture.scale = 0.8;
    brush.texture.depth = 0.4;
    brush.texture.each_tip = true;
    brush.dual_brush.enabled = true;
    brush.dual_brush.size = size * 1.5;
    brush.dual_brush.roundness = 0.8;
    brush.dual_brush.angle = 31.0;
    brush.dual_brush.flip = true;
    brush
}

fn set_brush(app: &mut PhotocraftApp, brush: &BrushSettings) -> BrushSettings {
    app.run("tools.setBrush", json!({"brush": brush, "summary": true})).unwrap();
    app.session.tools.brush.clone()
}

fn activate(app: &mut PhotocraftApp, tool: Tool) {
    app.ui.tool = tool;
    app.sync_tool_brush();
}

#[test]
fn latest_sampled_brush_survives_move_and_eyedropper_interludes() {
    let mut app = app();
    let brush = set_brush(&mut app, &textured(23.0));
    for intermediate in [Tool::Move, Tool::Eyedropper, Tool::Hand] {
        activate(&mut app, intermediate);
        activate(&mut app, Tool::Eraser);
        set_brush(&mut app, &BrushSettings { size: 71.0, ..Default::default() });
        activate(&mut app, intermediate);
        activate(&mut app, Tool::Brush);
        assert_eq!(app.session.tools.brush, brush, "latest sampled tip/settings must survive {intermediate:?}");
    }
}

#[test]
fn full_brush_and_smoothing_snapshots_have_the_same_owner() {
    let mut app = app();
    let mut original = textured(31.0);
    original.smoothing.amount = 0.42;
    original.smoothing.pulled_string = true;
    let original = set_brush(&mut app, &original);
    activate(&mut app, Tool::Eraser);
    let mut eraser = BrushSettings { size: 65.0, ..Default::default() };
    eraser.smoothing.amount = 0.03;
    let eraser = set_brush(&mut app, &eraser);
    for _ in 0..3 {
        app.ui.tool = Tool::Brush;
        crate::paint_mouse::sync_tool_smoothing(&mut app);
        assert_eq!(app.session.tools.brush, original);
        app.ui.tool = Tool::Eraser;
        crate::paint_mouse::sync_tool_smoothing(&mut app);
        assert_eq!(app.session.tools.brush, eraser);
    }
}

#[test]
fn editing_a_just_selected_tool_does_not_modify_the_outgoing_brush_slot() {
    let mut app = app();
    let original = set_brush(&mut app, &textured(29.0));
    app.ui.tool = Tool::Eraser;
    // A preset/size edit in the same UI pass must see the incoming tool before
    // the command applies, even when there was no intervening logic frame.
    let eraser = set_brush(&mut app, &BrushSettings { size: 83.0, ..Default::default() });
    activate(&mut app, Tool::Brush);
    assert_eq!(app.session.tools.brush, original);
    activate(&mut app, Tool::Eraser);
    assert_eq!(app.session.tools.brush, eraser);
}

#[test]
fn tool_preset_replaces_target_slot_atomically_without_poisoning_source() {
    let mut app = app();
    let outgoing = set_brush(&mut app, &textured(19.0));
    activate(&mut app, Tool::Smudge);
    set_brush(&mut app, &BrushSettings { size: 51.0, ..Default::default() });
    activate(&mut app, Tool::Brush);
    let incoming = textured(47.0);
    let name = app
        .run("tool.presets.new", json!({"name":"Switch target", "tool":"smudge", "options":{"brush":incoming, "toolOptions":{"strength":73.0}}}))
        .unwrap()["name"]
        .as_str()
        .unwrap()
        .to_owned();
    crate::preset_panels::select_tool_preset(&mut app, &name);
    assert_eq!(app.ui.tool, Tool::Smudge);
    assert_eq!(app.ui.tool_options.strength, 73.0);
    assert!(matches!(app.session.tools.brush.tip, TipShape::Sampled(_)));
    assert_eq!(app.session.tools.brush.size, 47.0);
    let applied = app.session.tools.brush.clone();
    for _ in 0..4 {
        app.sync_tool_brush();
        crate::paint_mouse::sync_tool_smoothing(&mut app);
        assert_eq!(app.session.tools.brush, applied, "a later frame must not restore stale round-tip settings");
    }
    activate(&mut app, Tool::Brush);
    assert_eq!(app.session.tools.brush, outgoing);
    activate(&mut app, Tool::Smudge);
    assert_eq!(app.session.tools.brush, applied);
}

#[test]
fn failed_tool_preset_does_not_switch_or_replace_the_brush() {
    let mut app = app();
    let original = set_brush(&mut app, &textured(21.0));
    assert!(app.run("tool.presets.select", json!({"preset":"missing v59 test preset"})).is_err());
    app.sync_tool_brush();
    assert_eq!(app.ui.tool, Tool::Brush);
    assert_eq!(app.session.tools.brush, original);
}

#[test]
fn queued_control_tool_changes_restore_brush_before_reply_and_next_request() {
    let mut app = app();
    let tools = [Tool::Brush, Tool::Eraser, Tool::Pencil, Tool::MixerBrush, Tool::Smudge, Tool::QuickSelection];
    let mut expected = Vec::new();
    for (index, tool) in tools.into_iter().enumerate() {
        activate(&mut app, tool);
        expected.push((tool, set_brush(&mut app, &textured(17.0 + index as f32))));
    }
    let (tx, rx) = std::sync::mpsc::channel();
    app.control_rx = Some(rx);
    let ctx = egui::Context::default();
    for (tool, brush) in expected {
        let (request, reply) = ControlRequest::new("ui.set", json!({"tool":format!("{tool:?}")}));
        tx.send(request).unwrap();
        app.drain_control(&ctx);
        assert_eq!(reply.try_recv().unwrap()["ok"], true);
        assert_eq!(app.session.tools.brush, brush, "{tool:?} is restored before the next control request");
    }
}

fn stroke(app: &mut PhotocraftApp) -> Vec<[f32; 4]> {
    use crate::canvas::{ToolEvent, tool_event};
    for event in [ToolEvent::Down { x: 12.0, y: 25.0, pressure: 1.0 }, ToolEvent::Move { x: 36.0, y: 25.0, pressure: 1.0 }, ToolEvent::Up { x: 48.0, y: 25.0 }]
    {
        tool_event(app, event, egui::Modifiers::NONE);
    }
    let surface = app.session.active().unwrap().doc.layers.last().unwrap().surface().unwrap();
    (0..64).flat_map(|y| (0..64).map(move |x| surface.rgba(x, y))).collect()
}

#[test]
fn first_pen_down_after_switch_uses_sampled_tip_instead_of_previous_hard_round() {
    let mut reference = app();
    let mut switched = app();
    for app in [&mut reference, &mut switched] {
        app.run("file.new", json!({"width":64,"height":64})).unwrap();
        app.run("layer.new.layer", Value::Null).unwrap();
        set_brush(app, &textured(15.0));
    }
    let expected = stroke(&mut reference);
    assert!(expected.iter().any(|pixel| pixel[3] > 0.0));
    activate(&mut switched, Tool::Eraser);
    set_brush(&mut switched, &BrushSettings { size: 3.0, hardness: 1.0, ..Default::default() });
    switched.ui.tool = Tool::Brush;
    // Deliberately no sync/frame/command between this assignment and pen-down.
    assert_eq!(stroke(&mut switched), expected, "the very first committed stroke must match the selected sampled brush");
}
