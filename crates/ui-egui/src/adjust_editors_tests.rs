//! UI tests for the adjustment editors: the whole app (eframe harness, CPU canvas) with an
//! adjustment layer on a new document, edited through the Properties panel with real pointer and
//! key input (issue #12: "add an adjustment layer to a new document, edit it, crash"), and the
//! Curves point interactions (issue #43).

use egui::{Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{Adjustment, LayerContent, LayerId};
use serde_json::json;

use crate::PhotocraftApp;

const ALL_KINDS: [&str; 16] = [
    "brightnessContrast",
    "levels",
    "curves",
    "exposure",
    "vibrance",
    "hueSaturation",
    "colorBalance",
    "blackWhite",
    "photoFilter",
    "channelMixer",
    "invert",
    "posterize",
    "threshold",
    "gradientMap",
    "selectiveColor",
    "colorLookup",
];

fn app_harness(kind: &str, mode: &str, depth: u32) -> Harness<'static, PhotocraftApp> {
    let (kind, mode) = (kind.to_string(), mode.to_string());
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 96, "height": 64, "mode": mode, "depth": depth})).unwrap();
        // Two tones so histograms (and Levels' Auto) have something to work with.
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 48, "height": 64})).unwrap();
        s.execute("edit.fill", json!({"color": "#6a3020"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute(&format!("layer.newAdjustmentLayer.{kind}"), json!({})).unwrap();
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        // The default Properties group gives way to Layers (#147) and scrolls taller editors;
        // these tests drive every control without scrolling, so they size it like a user would.
        app.ui.dock.heights.insert(crate::dock::Group::Properties, 560.0);
        app
    });
    h.run_steps(8);
    h
}

fn layer(h: &Harness<'_, PhotocraftApp>) -> (LayerId, Adjustment) {
    let st = h.state().session.active().unwrap();
    let id = st.active_layer.unwrap();
    match &st.doc.layer(id).unwrap().content {
        LayerContent::Adjustment(a) => (id, a.clone()),
        other => panic!("{other:?}"),
    }
}

fn drag(h: &mut Harness<'_, PhotocraftApp>, from: Pos2, to: Pos2) {
    h.hover_at(from);
    h.run_steps(1);
    h.drag_at(from);
    h.run_steps(1);
    for i in 1..=6 {
        h.hover_at(from + (to - from) * (i as f32 / 6.0));
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(3);
}

fn click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    h.drop_at(at);
    h.run_steps(3);
}

fn label_rect(h: &Harness<'_, PhotocraftApp>, text: &str) -> Option<Rect> {
    // The last match: the Properties header repeats the kind's name above its controls.
    h.query_all_by_label(text).last().map(|n| n.rect())
}

#[test]
fn exposure_editor_keeps_hundredths_when_dragged_and_typed() {
    use egui::accesskit::Role;

    let mut h = Harness::builder().with_size(vec2(440.0, 240.0)).build_ui_state(
        |ui, (values, rect): &mut (serde_json::Value, Rect)| {
            *rect = ui.available_rect_before_wrap();
            let cx = super::EditorCx { mem: ui.id().with("exposure-precision"), hist: None, gray: false, swatches: [[0.0; 3]; 2] };
            super::editor(ui, "exposure", values, &cx);
        },
        (json!({"exposure": 3.0, "offset": -0.01, "gamma": 1.03}), Rect::NOTHING),
    );
    h.run_steps(3);
    h.query_all_by_role(Role::SpinButton).find(|node| node.value().as_deref() == Some("3.00")).expect("fixed exposure decimals").click();
    h.run_steps(1);
    h.event(egui::Event::Text("-0.07".into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!((h.state().0["exposure"].as_f64().unwrap() + 0.07).abs() < 1e-6);

    let rect = h.state().1;
    let y = h.get_by_label("Exposure").rect().bottom() + 14.0;
    let from = Pos2::new(rect.center().x, y);
    let to = Pos2::new(rect.left() + 7.0 + (rect.width() - 14.0) * (21.23 / 40.0), y);
    h.hover_at(from);
    h.run_steps(1);
    h.drag_at(from);
    h.run_steps(1);
    h.hover_at(to);
    h.run_steps(1);
    h.drop_at(to);
    h.run_steps(3);
    assert!((h.state().0["exposure"].as_f64().unwrap() - 1.23).abs() < 1e-6, "{}", h.state().0);
    assert!(h.query_all_by_role(Role::SpinButton).any(|node| node.value().as_deref() == Some("1.23")));
    assert_eq!(h.state().0["offset"], json!(-0.01));
    assert_eq!(h.state().0["gamma"], json!(1.03));
}

fn curves_graph(h: &Harness<'_, PhotocraftApp>, id: LayerId) -> Rect {
    h.ctx.data(|d| d.get_temp::<Rect>(egui::Id::new(("adjust-layer", id.0)).with("curves-graph"))).expect("curves graph drawn")
}

/// Issue #12: every adjustment kind, on a new document at 8/16/32 bits and in RGB, Grayscale,
/// CMYK and Lab, opens its Properties editor and takes an edit through real input without
/// panicking; slider kinds commit exactly one history step per gesture.
#[test]
fn every_adjustment_layer_edits_through_properties() {
    for (mode, depth) in [("rgb", 8), ("rgb", 16), ("rgb", 32), ("gray", 8), ("cmyk", 8), ("lab", 16)] {
        for kind in ALL_KINDS {
            let mut h = app_harness(kind, mode, depth);
            let (id, before) = layer(&h);
            let steps = h.state().session.active().unwrap().history.past_len();
            // A probe label per editor; the control sits just below it.
            let probe = match kind {
                "brightnessContrast" => Some("Brightness"),
                "exposure" => Some("Exposure"),
                "vibrance" => Some("Vibrance"),
                "hueSaturation" => Some("Hue"),
                "colorBalance" => Some("Cyan  ·  Red"),
                "blackWhite" => Some("Reds"),
                "photoFilter" => Some("Density"),
                "channelMixer" => Some("Constant"),
                "posterize" => Some("Levels"),
                "threshold" => Some("Threshold Level"),
                "selectiveColor" => Some("Cyan"),
                _ => None,
            };
            match kind {
                "curves" => {
                    let g = curves_graph(&h, id);
                    drag(&mut h, g.center(), g.center() + vec2(0.0, -40.0));
                }
                "levels" => {
                    // The output black handle sits below the output gradient bar.
                    let r = label_rect(&h, "Output Levels:").unwrap_or_else(|| panic!("{kind} {mode}: Output Levels"));
                    let y = r.bottom() + 17.0;
                    drag(&mut h, Pos2::new(r.left() + 6.0, y), Pos2::new(r.left() + 60.0, y));
                }
                "gradientMap" => {
                    let r = label_rect(&h, "Reverse").unwrap_or_else(|| panic!("{kind} {mode}: Reverse"));
                    click(&mut h, r.center());
                }
                _ => {
                    if let Some(p) = probe {
                        let r = label_rect(&h, p).unwrap_or_else(|| panic!("{kind} {mode}: no `{p}` label"));
                        let y = r.bottom() + 14.0;
                        drag(&mut h, Pos2::new(r.left() + 40.0, y), Pos2::new(r.left() + 120.0, y));
                    }
                }
            }
            h.run_steps(4);
            let (_, after) = layer(&h);
            assert!(h.state().live_adjust.is_none(), "{kind} {mode}: preview ended with the gesture");
            if !matches!(kind, "invert" | "colorLookup") {
                assert_ne!(after, before, "{kind} {mode}@{depth}: the edit reached the layer");
                assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "{kind} {mode}: one history step");
            }
        }
    }
}

fn curve_of(a: &Adjustment) -> Vec<[f32; 2]> {
    match a {
        Adjustment::Curves { master, .. } => master.iter().map(|p| [(p.input * 255.0).round(), (p.output * 255.0).round()]).collect(),
        other => panic!("{other:?}"),
    }
}

/// Issue #43: pressing on a point selects it (no new point), Delete/Backspace and ⌘/Ctrl-click
/// remove points, dragging off the graph removes one, endpoints stay.
#[test]
fn curves_points_are_added_selected_and_deleted() {
    let mut h = app_harness("curves", "rgb", 8);
    let (id, _) = layer(&h);
    let g = curves_graph(&h, id);
    let at = |v: [f32; 2]| Pos2::new(g.left() + v[0] / 255.0 * g.width(), g.bottom() - v[1] / 255.0 * g.height());
    // Add a point.
    click(&mut h, at([128.0, 128.0]));
    assert_eq!(curve_of(&layer(&h).1).len(), 3);
    // Press 4 px off it and drag: moves that point, adds none.
    drag(&mut h, at([128.0, 128.0]) + vec2(4.0, 3.0), at([128.0, 180.0]));
    let c = curve_of(&layer(&h).1);
    assert_eq!(c.len(), 3, "{c:?}");
    assert!(c[1][1] > 165.0, "{c:?}");
    // Delete key removes the selected point (the graph has focus).
    h.key_press(egui::Key::Delete);
    h.run_steps(3);
    assert_eq!(curve_of(&layer(&h).1).len(), 2);
    // Backspace too.
    click(&mut h, at([64.0, 100.0]));
    assert_eq!(curve_of(&layer(&h).1).len(), 3);
    h.key_press(egui::Key::Backspace);
    h.run_steps(3);
    assert_eq!(curve_of(&layer(&h).1).len(), 2);
    // ⌘/Ctrl-click removes.
    click(&mut h, at([190.0, 120.0]));
    assert_eq!(curve_of(&layer(&h).1).len(), 3);
    let p = at([190.0, 120.0]);
    h.event_modifiers(egui::Event::PointerMoved(p), egui::Modifiers::COMMAND);
    h.event_modifiers(
        egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::COMMAND },
        egui::Modifiers::COMMAND,
    );
    h.run_steps(1);
    h.event_modifiers(
        egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::COMMAND },
        egui::Modifiers::COMMAND,
    );
    h.run_steps(3);
    assert_eq!(curve_of(&layer(&h).1).len(), 2);
    // Dragging a point off the graph removes it; endpoints survive both.
    click(&mut h, at([100.0, 60.0]));
    drag(&mut h, at([100.0, 60.0]), g.right_center() + vec2(80.0, 0.0));
    assert_eq!(curve_of(&layer(&h).1).len(), 2);
    drag(&mut h, at([0.0, 0.0]), g.left_top() + vec2(-80.0, -80.0));
    click(&mut h, at([0.0, 0.0]));
    h.key_press(egui::Key::Delete);
    h.run_steps(3);
    assert_eq!(curve_of(&layer(&h).1).len(), 2, "endpoints can't be deleted");
}

/// Capture and history are shared with Camera Raw, but Properties commits on release.
#[test]
fn curves_fast_release_commits_one_edit_and_undo_restores_the_curve() {
    let mut h = app_harness("curves", "rgb", 8);
    let (id, _) = layer(&h);
    let graph = curves_graph(&h, id);
    click(&mut h, graph.center());
    let (_, before) = layer(&h);
    let steps = h.state().session.active().unwrap().history.past_len();
    let at = graph.center() + vec2(4.0, 3.0);
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    // Only the release frame supplies the destination, as a very fast physical gesture can.
    h.drop_at(at + vec2(40.0, -32.0));
    h.run_steps(3);
    let after = curve_of(&layer(&h).1);
    assert_eq!(after.len(), 3);
    assert!(after[1][0] > 150.0 && after[1][1] > 150.0, "{after:?}");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
    assert!(h.state().live_adjust.is_none());
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(layer(&h).1, before);
}

/// The floating Properties content uses its tabs and identity row without stacking section
/// margins, and the smaller plot still commits edits and supports undo at minimum host width.
#[test]
fn compact_properties_curves_fit_below_the_header_and_remain_editable_in_every_theme() {
    for theme in crate::theme::ThemeKind::ALL {
        for width in [250.0, 320.0] {
            let mut session = photocraft_engine::Session::new();
            session.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
            session.execute("layer.newAdjustmentLayer.curves", json!({})).unwrap();
            let app = PhotocraftApp::new(session, Default::default());
            let mut h = Harness::builder().with_size(vec2(width + 20.0, 650.0)).with_max_steps(64).build_ui_state(
                move |ui, app: &mut PhotocraftApp| {
                    if ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                        ui.set_width(width);
                        let mut selected = 0;
                        let card = crate::widgets::card_ex_with_compact_body(
                            ui,
                            "compact-properties",
                            &["Properties", "Adjustments"],
                            &mut selected,
                            false,
                            true,
                            |ui, _| {
                                crate::panels::dock_body(app, ui, crate::dock::Group::Properties, 0);
                            },
                        );
                        ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new("compact-properties-tab"), card.tabs[0].1));
                    }
                },
                app,
            );
            PhotocraftApp::setup_context(&h.ctx, theme);
            h.run_steps(4);
            let (id, before) = layer(&h);
            let name = &h.state().session.active().unwrap().doc.layer(id).unwrap().name;
            let header = h.get_by_value(&format!("{name} — Curves")).rect();
            let channel = h.get_by_label("Channel:").rect();
            let tab = h.ctx.data(|data| data.get_temp::<Rect>(egui::Id::new("compact-properties-tab"))).unwrap();
            assert!(header.top() - tab.bottom() <= 12.0, "{theme:?} {width}: tab {tab:?}, header {header:?}");
            assert!(channel.top() - header.bottom() <= 12.0, "{theme:?} {width}: header {header:?}, channel {channel:?}");
            let graph = curves_graph(&h, id);
            let old_side = header.width().clamp(120.0, 300.0) - 24.0;
            assert!(graph.area() <= old_side * old_side * 0.82, "{theme:?} {width}: plot was not made smaller: {graph:?}");
            assert!(graph.width() >= 84.0 && graph.is_finite(), "{graph:?}");
            assert!(graph.left() >= header.left() && graph.right() <= header.right(), "{graph:?}, {header:?}");
            let steps = h.state().session.active().unwrap().history.past_len();
            drag(&mut h, graph.center(), graph.center() + vec2(0.0, -24.0));
            let edited = curve_of(&layer(&h).1);
            assert_eq!(edited.len(), 3, "{theme:?} {width}: {edited:?}");
            assert!(edited[1][1] > 140.0, "{theme:?} {width}: {edited:?}");
            assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
            assert!(h.state().live_adjust.is_none());
            h.state_mut().run("edit.undo", json!({})).unwrap();
            assert_eq!(layer(&h).1, before);
        }
    }
}
