use super::*;
use egui_kittest::{Harness, kittest::Queryable};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":16,"height":12,"background":"transparent"})).unwrap();
    app.run("edit.fill", json!({"color":"#406080"})).unwrap();
    app
}

#[test]
fn raw_tabs_v78_pointer_navigation_exposes_independent_pages_and_keeps_settings() {
    let mut h = Harness::builder().with_size(vec2(1400.0, 1000.0)).with_max_steps(96).build_ui_state(|ui, app: &mut PhotocraftApp| show(app, ui.ctx()), app());
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
    let ctx = h.ctx.clone();
    open(h.state_mut(), &ctx).unwrap();
    let d = h.state_mut().camera_raw.as_mut().unwrap();
    d.params.exposure = 0.25;
    d.params.color_lookup.amount = 63.0;
    d.output_options.depth = 8;
    d.output_options.resolution = 225.0;
    let expected = d.params.clone();
    h.run();
    assert!(h.query_by_label(tl!("Create Output Copy")).is_none());
    assert!(h.query_by_label(tl!("Import LUT…")).is_none());
    h.get_by_label(tabs::Tab::ColorSpaces.label()).click();
    h.run();
    assert_eq!(h.state().camera_raw.as_ref().unwrap().active_tab, tabs::Tab::ColorSpaces);
    assert!(h.query_by_label(tl!("Create Output Copy")).is_some());
    assert!(h.query_by_label(tl!("Import LUT…")).is_none());
    h.get_by_label(tabs::Tab::ColorLookup.label()).click();
    h.run();
    assert_eq!(h.state().camera_raw.as_ref().unwrap().active_tab, tabs::Tab::ColorLookup);
    assert!(h.query_by_label(tl!("Import LUT…")).is_some());
    assert!(h.query_by_label(tl!("Create Output Copy")).is_none());
    h.get_by_label(tabs::Tab::Edit.label()).click();
    h.run();
    let d = h.state().camera_raw.as_ref().unwrap();
    assert_eq!(d.active_tab, tabs::Tab::Edit);
    assert_eq!(d.params, expected);
    assert_eq!(d.output_options.depth, 8);
    assert_eq!(d.output_options.resolution, 225.0);
    assert!(d.color_geometry.create.is_none(), "hidden output controls must not keep stale hit rectangles");
    assert!(h.query_by_label(tl!("Create Output Copy")).is_none());
    assert!(h.query_by_label(tl!("Import LUT…")).is_none());
}

#[test]
fn raw_tabs_v78_control_navigation_reports_five_pages_and_invalid_tab_is_atomic() {
    let mut app = app();
    let ctx = egui::Context::default();
    open(&mut app, &ctx).unwrap();
    for tab in ["colorSpaces", "colorLookup", "masks", "presets", "edit"] {
        let result = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"tab":tab}})).unwrap().unwrap();
        assert_eq!(result["tab"], tab);
        assert_eq!(result["tabs"]["buttons"].as_array().unwrap().len(), 5);
    }
    let previous = app.camera_raw.as_ref().unwrap().params.clone();
    assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"tab":"bad","set":{"exposure":1.0}}})).unwrap().is_err());
    assert_eq!(app.camera_raw.as_ref().unwrap().active_tab, tabs::Tab::Edit);
    assert_eq!(app.camera_raw.as_ref().unwrap().params, previous);
}

#[test]
fn raw_tabs_v78_collapsible_sections_paint_full_width_theme_borders() {
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = Harness::builder().with_size(vec2(600.0, 300.0)).build_ui(|ui| {
            section(ui, "Effects", false, |_| {});
            section(ui, "Curve", false, |_| {});
            section(ui, "Detail", false, |_| {});
        });
        crate::theme::apply(&h.ctx, theme);
        h.run();
        let tokens = Tokens::get(&h.ctx);
        let count = h
            .output()
            .shapes
            .iter()
            .filter(|shape| {
                matches!(&shape.shape, egui::epaint::Shape::LineSegment { points, stroke }
                if (points[1].x-points[0].x) > 500.0 && (points[1].y-points[0].y).abs() < 0.1
                    && stroke.width == 1.0 && stroke.color == tokens.field_border)
            })
            .count();
        assert_eq!(count, 3, "each section uses a full-width theme border: {theme:?}");
        assert_ne!(tokens.field_border, tokens.dock, "default theme border distinguishes the dock");
    }
}
