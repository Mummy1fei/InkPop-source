//! Labels and explicit hidden-state icons through the real Layers panel drawing path.

use egui::{Color32, Event, Modifiers, PointerButton, Rect, Shape, vec2};
use egui_kittest::Harness;
use photocraft_doc::{LabelColor, LayerId};
use serde_json::json;

use crate::PhotocraftApp;
use crate::theme::{ThemeKind, Tokens};

fn session() -> (photocraft_engine::Session, u64, u64, u64) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width":64,"height":48})).unwrap();
    let hidden = s.execute("layer.new.layer", json!({"name":"Hidden child"})).unwrap()["layer"].as_u64().unwrap();
    let shown = s.execute("layer.new.layer", json!({"name":"Visible child"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.select", json!({"layer":hidden,"mode":"toggle"})).unwrap();
    let group = s.execute("layer.new.group", json!({"name":"Colored group"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setLabel", json!({"color":"red"})).unwrap();
    s.execute("layer.setProps", json!({"layer":hidden,"visible":false})).unwrap();
    s.execute("layer.setExpanded", json!({"layer":group,"expanded":true})).unwrap();
    (s, hidden, shown, group)
}

fn paints_label(shape: &Shape, rect: Rect, color: Color32) -> bool {
    match shape {
        Shape::Rect(r) => r.rect == rect && r.fill == color,
        Shape::Vec(shapes) => shapes.iter().any(|s| paints_label(s, rect, color)),
        _ => false,
    }
}

fn paints_icon(shape: &Shape, eye: Rect, tint: Color32) -> bool {
    match shape {
        Shape::Rect(r) => eye.contains_rect(r.rect) && r.brush.is_some() && r.fill == tint,
        Shape::Mesh(m) => eye.contains_rect(m.calc_bounds()) && !m.vertices.is_empty() && m.vertices.iter().all(|v| v.color == tint),
        Shape::Vec(shapes) => shapes.iter().any(|s| paints_icon(s, eye, tint)),
        _ => false,
    }
}

#[test]
fn hidden_icons_and_wide_labels_are_painted_and_toggle_in_all_themes() {
    for kind in ThemeKind::ALL {
        for ppp in [1.0, 2.0] {
            let (session, hidden, shown, group) = session();
            let mut h = Harness::builder().with_size(vec2(1440.0, 1000.0)).with_pixels_per_point(ppp).with_max_steps(64).build_eframe(move |cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                PhotocraftApp::new(session, crate::Services::default())
            });
            let ctx = h.ctx.clone();
            let (req, _rx) = crate::control::ControlRequest::new(
                "ui.set",
                json!({"theme":kind.id(),"dockWidth":290,"dock":{"collapsed":["color","properties","history","navigator"]}}),
            );
            crate::control::handle(h.state_mut(), &ctx, &req);
            h.run_steps(8);
            let t = Tokens::for_kind(kind);
            let rows = super::recorded(&h.ctx);
            for (id, icon) in [(hidden, "eye-off"), (shown, "eye"), (group, "eye")] {
                let row = rows.iter().find(|r| r.layer == id).unwrap();
                assert_eq!(row.visibility_icon, icon, "{kind:?}@{ppp}: visible and hidden states differ");
                let label = row.label.expect("colored gutter");
                assert!(label.width() >= 29.0, "{kind:?}: broad color block, not a four-point stripe");
                assert!(label.contains_rect(row.eye), "{kind:?}: the whole eye lives on the color block");
                assert!(label.right() < row.name.unwrap().left(), "{kind:?}: label leaves thumbnail/name space");
                assert!(label.top() > row.row.top() && label.left() > row.row.left(), "selection outline remains clear");
                assert!(h.output().shapes.iter().any(|s| paints_label(&s.shape, label, super::label_color(LabelColor::Red).unwrap())));
                assert!(
                    h.output().shapes.iter().any(|s| paints_icon(&s.shape, row.eye, super::visibility_tint(LabelColor::Red, &t))),
                    "{kind:?}: actual icon texture must be drawn, including hidden layers"
                );
            }
            let hidden_row = rows.iter().find(|r| r.layer == hidden).unwrap();
            let group_row = rows.iter().find(|r| r.layer == group).unwrap();
            assert!(hidden_row.label.unwrap().width() > group_row.label.unwrap().width(), "nested indentation joins the color gutter");
            let p = hidden_row.eye.center();
            h.hover_at(p);
            h.run_steps(1);
            for pressed in [true, false] {
                h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
                h.run_steps(1);
            }
            h.run_steps(3);
            assert!(h.state().session.active().unwrap().doc.layer(LayerId(hidden)).unwrap().visible);
            assert_eq!(super::recorded(&h.ctx).iter().find(|r| r.layer == hidden).unwrap().visibility_icon, "eye");
            assert_eq!(h.state().session.active().unwrap().active_layer, Some(LayerId(group)), "eye does not change layer selection");
        }
    }
}

fn luminance(c: Color32) -> f32 {
    let linear = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    linear(c.r()) * 0.2126 + linear(c.g()) * 0.7152 + linear(c.b()) * 0.0722
}

#[test]
fn colored_gutter_icons_keep_three_to_one_contrast_in_every_theme() {
    for kind in ThemeKind::ALL {
        let t = Tokens::for_kind(kind);
        for label in [LabelColor::Red, LabelColor::Orange, LabelColor::Yellow, LabelColor::Green, LabelColor::Blue, LabelColor::Violet, LabelColor::Gray] {
            let bg = luminance(super::label_color(label).unwrap());
            let fg = luminance(super::visibility_tint(label, &t));
            let ratio = (bg.max(fg) + 0.05) / (bg.min(fg) + 0.05);
            assert!(ratio >= 3.0, "{kind:?} {label:?}: {ratio}");
        }
    }
}

fn folder_scene(kind: ThemeKind, ppp: f32, gpu: bool) -> (Harness<'static, PhotocraftApp>, u64, u64) {
    use photocraft_engine::prefs::palette::{StudioLightColors, builtin::BUILTIN_PALETTES};
    let (mut s, _, shown, group) = session();
    s.execute("layer.layerMask.revealAll", json!({"layer":group})).unwrap();
    let builder = Harness::builder().with_size(vec2(1200.0, 800.0)).with_pixels_per_point(ppp).with_max_steps(64);
    let builder = if gpu { builder.wgpu_setup(crate::gpu_canvas::wgpu_setup()) } else { builder };
    let mut h = builder.build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, kind);
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        let colors = if kind == ThemeKind::StudioLight {
            StudioLightColors { colors: BUILTIN_PALETTES[0].colors(), presets: Vec::new() }
        } else {
            StudioLightColors::default()
        };
        app.run("prefs.set", json!({"values":{"interface.theme":kind.id(),"interface.language":"en","interface.studioLightColors":colors}})).unwrap();
        app.ui.mask_target = false;
        app.ui.vector_mask_target = false;
        app
    });
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::control::ControlRequest::new(
        "ui.set",
        json!({"theme":kind.id(),"dockWidth":320,"dock":{"collapsed":["color","properties","history","navigator"]}}),
    );
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(8);
    (h, group, shown)
}

fn any_shape(shape: &Shape, predicate: &impl Fn(&Shape) -> bool) -> bool {
    predicate(shape) || matches!(shape, Shape::Vec(children) if children.iter().any(|child| any_shape(child, predicate)))
}

fn thumbnail_rect(row: &super::RowRects, group: bool, depth: usize, pro: bool) -> Rect {
    let size = if pro { 24.0 } else { 34.0 };
    // This fixture uses a wide panel: the 14-point nesting and full thumbnail
    // sizes fit without compression. The folder owns the disclosure slot too.
    let x = row.row.left() + 6.0 + 28.0 + depth as f32 * 14.0 + if group { crate::layer_tree_ui::TRIANGLE_W } else { 0.0 };
    Rect::from_min_size(egui::pos2(x, row.row.center().y - size / 2.0), vec2(size, size))
}

fn has_thumbnail_border(h: &Harness<'_, PhotocraftApp>, rect: Rect) -> bool {
    h.output().shapes.iter().any(|s| any_shape(&s.shape, &|s| matches!(s, Shape::Rect(r) if r.rect == rect && r.stroke.width > 0.0)))
}

fn has_target_brackets(h: &Harness<'_, PhotocraftApp>, rect: Rect) -> bool {
    let r = rect.expand(3.0);
    [(r.left_top(), 1.0, 1.0), (r.right_top(), -1.0, 1.0), (r.right_bottom(), -1.0, -1.0), (r.left_bottom(), 1.0, -1.0)].into_iter().all(|(corner, dx, dy)| {
        [vec2(6.0 * dx, 0.0), vec2(0.0, 6.0 * dy)].into_iter().all(|delta| {
            h.output().shapes.iter().any(|s| {
                any_shape(&s.shape, &|s| matches!(s, Shape::LineSegment { points, stroke } if *points == [corner, corner + delta] && stroke.width == 1.5))
            })
        })
    })
}

fn assert_folder_only_glyph(h: &Harness<'_, PhotocraftApp>, group: u64, raster: u64) -> (Rect, Rect) {
    let rows = super::recorded(&h.ctx);
    let group_row = rows.iter().find(|r| r.layer == group).unwrap();
    let raster_row = rows.iter().find(|r| r.layer == raster).unwrap();
    let t = Tokens::get(&h.ctx);
    let folder = thumbnail_rect(group_row, true, 0, t.pro);
    let raster = thumbnail_rect(raster_row, false, 1, t.pro);
    assert!(!has_thumbnail_border(h, folder), "folder glyph must not have a thumbnail frame");
    assert!(!has_target_brackets(h, folder), "selected folder must not have pixel target brackets");
    assert!(
        !h.output().shapes.iter().any(|s| any_shape(&s.shape, &|s| matches!(s, Shape::Rect(r) if r.rect == folder && r.fill != Color32::TRANSPARENT))),
        "folder must not have the old filled tile"
    );
    assert!(paints_icon_in_output(h, Rect::from_center_size(folder.center(), vec2(18.0, 18.0)), t.icon), "the folder glyph remains painted");
    assert!(has_thumbnail_border(h, raster), "ordinary raster thumbnail keeps its frame");
    if !t.pro && h.state().session.active().unwrap().active_layer == Some(LayerId(group)) {
        assert!(has_thumbnail_border(h, group_row.row), "the selected group's whole-row outline remains");
    }
    let label = group_row.label.unwrap();
    assert!(h.output().shapes.iter().any(|s| paints_label(&s.shape, label, super::label_color(LabelColor::Red).unwrap())));
    let mask =
        crate::mask_thumbs_ui::recorded(&h.ctx, group).unwrap().0.into_iter().find(|(kind, _)| *kind == crate::mask_thumbs_ui::MaskKind::Pixel).unwrap().1;
    assert!(has_thumbnail_border(h, mask), "actual group mask keeps its thumbnail frame");
    (raster, mask)
}

fn paints_icon_in_output(h: &Harness<'_, PhotocraftApp>, rect: Rect, tint: Color32) -> bool {
    h.output().shapes.iter().any(|s| paints_icon(&s.shape, rect.expand(0.51), tint))
}

#[test]
fn folder_glyph_has_no_tile_or_frame_while_raster_and_mask_targets_keep_theirs_v70() {
    for kind in ThemeKind::ALL {
        for ppp in [1.0, 2.0] {
            let (mut h, group, raster) = folder_scene(kind, ppp, false);
            let (_, mask) = assert_folder_only_glyph(&h, group, raster);
            h.state_mut().ui.mask_target = true;
            h.run_steps(3);
            assert!(has_target_brackets(&h, mask), "actual group mask keeps target brackets");
            h.state_mut().run("layer.select", json!({"layer":raster})).unwrap();
            h.state_mut().ui.mask_target = false;
            h.run_steps(3);
            let (raster_rect, _) = assert_folder_only_glyph(&h, group, raster);
            assert!(has_target_brackets(&h, raster_rect), "ordinary raster target brackets remain");
        }
    }
}

#[test]
#[ignore = "writes five-theme actual Layers panel GPU captures for root visual review"]
fn folder_icon_gpu_visual_v70() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v70/folder-icon-gpu");
    assert!(!out.exists(), "preserve previous visual evidence");
    std::fs::create_dir_all(&out).unwrap();
    for kind in ThemeKind::ALL {
        let (mut h, group, raster) = folder_scene(kind, 1.0, true);
        h.run_steps(2);
        assert_folder_only_glyph(&h, group, raster);
        h.render().unwrap().save(out.join(format!("{}.png", kind.id()))).unwrap();
    }
}
