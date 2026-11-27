//! Badge size and real Layers-panel render coverage, without private brush data.
use super::*;
use crate::theme::{ThemeKind, Tokens};
use egui_kittest::Harness;

fn scene(kind: ThemeKind, ppp: f32, gpu: bool) -> Harness<'static, PhotocraftApp> {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 96, "height": 64, "name": "Badge comparison"})).unwrap();
    session.execute("layer.new.layer", json!({"name": "Smart Object / 智能对象"})).unwrap();
    session.execute("edit.fill", json!({"color": "#49a6b5"})).unwrap();
    session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let builder = Harness::builder().with_size(vec2(1100.0, 760.0)).with_pixels_per_point(ppp).with_max_steps(64);
    let builder = if gpu { builder.wgpu_setup(crate::gpu_canvas::wgpu_setup()) } else { builder };
    let mut h = builder.build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, kind);
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        app.run("prefs.set", json!({"values": {"interface.theme": kind.id(), "interface.language": "zh-hans"}})).unwrap();
        app
    });
    let ctx = h.ctx.clone();
    let (req, _) =
        crate::control::ControlRequest::new("ui.set", json!({"dockWidth": 320, "dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(8);
    assert_eq!(h.state().ui.theme, kind);
    assert_eq!(Tokens::get(&h.ctx).field, Tokens::for_kind(kind).field);
    assert_eq!(serde_json::to_value(h.state().session.prefs().interface.theme).unwrap(), json!(kind.id()));
    h
}

fn badge_rect(h: &Harness<'_, PhotocraftApp>, kind: ThemeKind) -> Rect {
    let active = h.state().session.active().unwrap().active_layer.unwrap().0;
    let row = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == active).unwrap();
    let name = row.name.unwrap();
    let field = Tokens::for_kind(kind).field;
    fn find(shape: &egui::Shape, row: Rect, field: Color32) -> Option<Rect> {
        match shape {
            egui::Shape::Rect(s) if s.fill == field && s.rect.size() == vec2(11.0, 11.0) && row.contains_rect(s.rect) => Some(s.rect),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, row, field)),
            _ => None,
        }
    }
    let badge = h.output().shapes.iter().find_map(|s| find(&s.shape, row.row, field)).expect("11-point smart-object badge drawn by the real layer thumbnail");
    assert!(badge.right() < name.left(), "badge must not obscure the layer name");
    assert!(row.thumbnail.contains_rect(badge));
    assert_eq!(badge.right_bottom(), row.thumbnail.right_bottom());
    badge
}

#[test]
fn compact_smart_badge_leaves_most_of_the_thumbnail_visible_in_every_theme() {
    for kind in ThemeKind::ALL {
        for ppp in [1.0, 2.0] {
            let h = scene(kind, ppp, false);
            assert_eq!(badge_rect(&h, kind).size(), vec2(11.0, 11.0));
        }
    }
}

#[test]
#[ignore = "writes GPU review images"]
fn smart_badge_gpu_visual_v75() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v75/badge-gpu");
    std::fs::create_dir_all(&out).unwrap();
    for kind in ThemeKind::ALL {
        let mut h = scene(kind, 1.0, true);
        badge_rect(&h, kind);
        h.render().unwrap().save(out.join(format!("{}.png", kind.id()))).unwrap();
    }
}
