use super::super::{Drag, ToolEvent, ViewXform, draw_drag_preview, tool_event};
use super::*;
use crate::{PhotocraftApp, state::Tool};
use egui::{Color32, Event, Modifiers, Pos2, Rect, Shape, pos2, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::json;

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":200,"height":120,"background":"transparent"})).unwrap();
    app.ui.tool = tool;
    app
}

#[test]
fn clipped_ellipse_preview_preserves_original_curve_and_includes_canvas_boundary_v80() {
    let points = (0..128)
        .map(|index| {
            let angle = index as f64 / 128.0 * std::f64::consts::TAU;
            [170.0 + 90.0 * angle.cos(), 90.0 + 70.0 * angle.sin()]
        })
        .collect();
    let clipped = clip_polygon(points, [200, 120]);
    assert!(clipped.len() > 12);
    assert!(clipped.iter().all(|point| point[0] >= 0.0 && point[0] <= 200.0 && point[1] >= 0.0 && point[1] <= 120.0));
    assert!(clipped.iter().any(|point| (point[0] - 200.0).abs() < 1e-9));
    assert!(clipped.iter().any(|point| (point[1] - 120.0).abs() < 1e-9));
    for point in clipped.iter().filter(|point| point[0] < 199.999 && point[1] < 119.999) {
        let equation = ((point[0] - 170.0) / 90.0).powi(2) + ((point[1] - 90.0) / 70.0).powi(2);
        assert!((equation - 1.0).abs() < 0.002, "clipped ellipse must not be squeezed into a smaller ellipse: {point:?}");
    }
    assert!(clipped.iter().any(|point| (point[0] - 170.0).abs() < 0.001 && (point[1] - 20.0).abs() < 0.001));
    assert!(clip_polygon(vec![[f64::NAN, 0.0], [1.0, 1.0]], [200, 120]).is_empty());
    assert!(clip_polygon(vec![[0.0, 0.0], [1.0, 1.0]], [0, 120]).is_empty());
}

fn ant_points(shape: &Shape, out: &mut Vec<Pos2>) {
    match shape {
        Shape::Path(path) if path.stroke.color == egui::epaint::ColorMode::Solid(Color32::WHITE) => out.extend(&path.points),
        Shape::LineSegment { points, stroke } if stroke.color == Color32::BLACK || stroke.color == Color32::WHITE => out.extend(points),
        Shape::Vec(shapes) => {
            for shape in shapes {
                ant_points(shape, out);
            }
        }
        _ => {}
    }
}

#[test]
fn real_marquee_preview_geometry_stays_inside_canvas_for_rotation_flip_zoom_and_shapes_v80() {
    for theme in crate::theme::ThemeKind::ALL {
        for tool in [Tool::RectMarquee, Tool::EllipseMarquee] {
            for rotation in [0.0, 23.0, 90.0] {
                for flip in [false, true] {
                    for zoom in [0.4, 1.7] {
                        let mut app = app(tool);
                        app.drag = Some(Drag::new(tool, [80.0, 20.0], vec![[80.0, 20.0, 1.0], [260.0, 160.0, 1.0]], Modifiers::NONE, false));
                        let ctx = egui::Context::default();
                        PhotocraftApp::setup_context(&ctx, theme);
                        let xf = ViewXform { rotation, flip, zoom, center: [100.0, 60.0], rect: Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0)) };
                        let input = egui::RawInput { screen_rect: Some(xf.rect), ..Default::default() };
                        let mut output = ctx.run_ui(input, |ui| draw_drag_preview(&mut app, ui.painter(), &xf));
                        let mut points = Vec::new();
                        for shape in &output.shapes {
                            ant_points(&shape.shape, &mut points);
                        }
                        assert!(!points.is_empty(), "actual drag-preview must be painted");
                        for point in points {
                            let point = xf.to_doc(point);
                            assert!(
                                point[0] >= -0.001 && point[0] <= 200.001 && point[1] >= -0.001 && point[1] <= 120.001,
                                "{theme:?}/{tool:?}/{rotation}/{flip}/{zoom}: {point:?}"
                            );
                        }
                        output.textures_delta.clear();
                    }
                }
            }
        }
    }
}

#[test]
fn canvas_pointer_commit_uses_existing_bounded_mask_without_squeezing_ellipse_v80() {
    for tool in [Tool::RectMarquee, Tool::EllipseMarquee] {
        let mut app = app(tool);
        for event in
            [ToolEvent::Down { x: 80.0, y: 20.0, pressure: 1.0 }, ToolEvent::Move { x: 260.0, y: 160.0, pressure: 1.0 }, ToolEvent::Up { x: 260.0, y: 160.0 }]
        {
            tool_event(&mut app, event, Modifiers::NONE);
        }
        let state = app.session.active().unwrap();
        let selection = state.doc.selection.as_ref().unwrap();
        let bounds = selection.content_bounds();
        assert!(bounds.x0 >= 0 && bounds.y0 >= 0 && bounds.x1 <= 200 && bounds.y1 <= 120);
        assert!(selection.sample_channel(-1, 50, 0) == 0.0 && selection.sample_channel(201, 50, 0) == 0.0);
        if tool == Tool::EllipseMarquee {
            assert!(selection.sample_channel(100, 40, 0) < 0.1, "retain original ellipse cut by canvas, rather than a compressed ellipse");
            assert!(selection.sample_channel(170, 80, 0) > 0.9);
        } else {
            assert_eq!(bounds, photocraft_geom::Rect::new(80, 20, 200, 120));
            assert!(selection.sample_channel(100, 40, 0) > 0.9);
        }
    }
}

#[test]
fn size_readout_stops_at_canvas_for_normal_ratio_size_and_centered_drags_v80() {
    use crate::canvas::{marquee_corners, marquee_readout};
    let mut options = crate::state::ToolOptions::default();
    let mut drag = Drag::new(Tool::RectMarquee, [80.0, 20.0], Vec::new(), Modifiers::NONE, false);
    assert_eq!(marquee_readout(clipped_readout(marquee_corners(&options, &drag, [260.0, 160.0]), [200, 120])), ["120 px", "100 px"]);
    for (style, width, height, alt) in [("fixedRatio", 2.0, 1.0, false), ("fixedSize", 300.0, 200.0, false), ("normal", 1.0, 1.0, true)] {
        options.marquee_style = style.into();
        options.marquee_width = width;
        options.marquee_height = height;
        drag.live.alt = alt;
        let area = clipped_readout(marquee_corners(&options, &drag, [260.0, 160.0]), [200, 120]);
        assert!(area[0] >= 0.0 && area[1] >= 0.0 && area[2] <= 200.0 && area[3] <= 120.0);
    }
}

fn toolbar(theme: crate::theme::ThemeKind, tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut app = app(tool);
    app.run("select.rect", json!({"x":10,"y":10,"width":30,"height":20})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1440.0, 500.0)).with_max_steps(64).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    h.run_steps(5);
    h
}

/// opt_label adds the Pro-theme colon after translating the field label.
/// Keep querying the precise visible string, rather than a fuzzy substring.
fn option_label(theme: crate::theme::ThemeKind, key: &str) -> String {
    let translated = crate::i18n::t(key);
    let label = if crate::theme::Tokens::for_kind(theme).pro && !translated.ends_with(':') { format!("{translated}:") } else { translated.to_string() };
    crate::i18n::t(&label).to_string()
}

#[test]
fn all_five_themes_show_real_marquee_top_bar_modes_and_disabled_rect_aa_v80() {
    for theme in crate::theme::ThemeKind::ALL {
        for tool in [Tool::RectMarquee, Tool::EllipseMarquee] {
            let mut h = toolbar(theme, tool);
            for label in [
                option_label(theme, "Feather"),
                tl!("Anti-alias").to_string(),
                option_label(theme, "Style"),
                option_label(theme, "Width"),
                option_label(theme, "Height"),
                tl!("Select and Mask…").to_string(),
            ] {
                let rect = h.get_by_label(&label).rect();
                assert!(rect.left() >= 0.0 && rect.right() <= 1440.0 && rect.bottom() <= 65.0, "{theme:?}/{tool:?}: {label}");
            }
            assert_eq!(h.get_by_label(tl!("Anti-alias")).accesskit_node().is_disabled(), tool == Tool::RectMarquee);
            let labels = [
                tl!("New selection").to_string(),
                crate::i18n::fmt(tl!("Add to selection  ({key})"), &[("key", &crate::shortcuts::pretty("Shift"))]),
                crate::i18n::fmt(tl!("Subtract from selection  ({key})"), &[("key", &crate::shortcuts::pretty("Alt"))]),
                crate::i18n::fmt(tl!("Intersect with selection  ({key})"), &[("key", &crate::shortcuts::pretty("Shift+Alt"))]),
            ];
            for (index, label) in labels.iter().enumerate() {
                h.get_by_label(label).click();
                h.run_steps(2);
                assert_eq!(h.state().ui.selection_mode, index as u8);
            }
        }
    }
}

fn type_number(h: &mut Harness<'_, PhotocraftApp>, index: usize, value: &str) {
    h.query_all_by_role(egui::accesskit::Role::SpinButton).nth(index).unwrap().click();
    h.run_steps(2);
    h.event(Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND });
    h.event(Event::Text(value.into()));
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
}

#[test]
fn marquee_style_dimensions_feather_swap_aa_and_select_mask_use_live_controls_v80() {
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = toolbar(theme, Tool::EllipseMarquee);
        type_number(&mut h, 0, "4");
        assert_eq!(h.state().ui.tool_options.feather, 4.0);
        let aa = h.state().ui.tool_options.anti_alias;
        h.get_by_label(tl!("Anti-alias")).click();
        h.run_steps(2);
        assert_ne!(h.state().ui.tool_options.anti_alias, aa);
        h.get_by_role(egui::accesskit::Role::ComboBox).click();
        h.run_steps(2);
        h.get_by_label(tl!("Fixed Ratio")).click();
        h.run_steps(2);
        assert_eq!(h.state().ui.tool_options.marquee_style, "fixedRatio");
        type_number(&mut h, 1, "2");
        type_number(&mut h, 2, "1");
        let width = h.query_all_by_role(egui::accesskit::Role::SpinButton).nth(1).unwrap().rect();
        let height_label = h.get_by_label(&option_label(theme, "Height")).rect();
        let swap = pos2((width.right() + height_label.left()) * 0.5, width.center().y);
        h.event(Event::PointerMoved(swap));
        h.run_steps(1);
        h.event(Event::PointerButton { pos: swap, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        h.event(Event::PointerButton { pos: swap, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
        assert_eq!([h.state().ui.tool_options.marquee_width, h.state().ui.tool_options.marquee_height], [1.0, 2.0]);
        h.get_by_role(egui::accesskit::Role::ComboBox).click();
        h.run_steps(2);
        h.get_by_label(tl!("Fixed Size")).click();
        h.run_steps(2);
        assert_eq!([h.state().ui.tool_options.marquee_width, h.state().ui.tool_options.marquee_height], [64.0, 64.0]);
        type_number(&mut h, 1, "80");
        type_number(&mut h, 2, "40");
        assert_eq!([h.state().ui.tool_options.marquee_width, h.state().ui.tool_options.marquee_height], [80.0, 40.0]);
        h.get_by_label(tl!("Select and Mask…")).click();
        h.run_steps(2);
        assert!(h.state().ui.dialogs.iter().any(|dialog| dialog.fields["__command"] == "select.refineEdge"));
    }
}
