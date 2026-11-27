//! Real CPU canvas-cache updates, reconstructed from egui texture deltas and compared to
//! an independent full-composite oracle. No physical input, native GPU or timing claim.
use super::*;
use egui::{ColorImage, Event, Key, Modifiers, TextureId};
use serde_json::json;
use std::collections::HashMap;

#[derive(Default)]
struct Uploaded(HashMap<TextureId, ColorImage>);

impl Uploaded {
    fn apply(&mut self, output: &egui::FullOutput) {
        for (id, updates) in &output.textures_delta.set {
            for delta in updates {
                let egui::ImageData::Color(image) = &delta.image;
                if let Some([x0, y0]) = delta.pos {
                    let current = self.0.get_mut(id).expect("partial cache upload needs prior allocation");
                    assert!(x0 + image.size[0] <= current.size[0] && y0 + image.size[1] <= current.size[1]);
                    for y in 0..image.size[1] {
                        for x in 0..image.size[0] {
                            current.pixels[(y0 + y) * current.size[0] + x0 + x] = image.pixels[y * image.size[0] + x];
                        }
                    }
                } else {
                    self.0.insert(*id, image.as_ref().clone());
                }
            }
        }
        for id in &output.textures_delta.free {
            self.0.remove(id);
        }
    }
}

fn placed(smart: bool) -> (PhotocraftApp, egui::Context) {
    placed_at(smart, 8, "rgb")
}

fn placed_at(smart: bool, depth: u32, mode: &str) -> (PhotocraftApp, egui::Context) {
    let mut source = photocraft_engine::Session::new();
    source.execute("file.new", json!({"width":61,"height":37,"background":"#d84925"})).unwrap();
    source
        .edit("Asymmetric source fixture", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(5, 3, 17, 29), &[0.1, 0.35, 0.9, 1.0]);
            Ok(())
        })
        .unwrap();
    let png = photocraft_io::export(&source.active().unwrap().doc, "source.png", &Default::default()).unwrap().bytes;
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":320,"height":240,"background":"#c7d0d8","depth":depth,"mode":mode})).unwrap();
    app.session.prefs.edit(|prefs| prefs.general.always_create_smart_objects_when_placing = smart);
    app.place_bytes("source.png", png, None).unwrap();
    app.ui.tool = crate::state::Tool::Move;
    app.ui.extras.snap = false;
    (app, ctx)
}

#[test]
fn partial_cache_matches_full_oracle_across_native_depths_and_gray_v91() {
    for mode in ["rgb", "gray"] {
        for depth in [8, 16, 32] {
            for smart in [false, true] {
                let (mut app, ctx) = placed_at(smart, depth, mode);
                crate::transform_tool::begin_placed(&mut app, &ctx).unwrap();
                let mut images = Uploaded::default();
                let (id, _) = paint(&mut app, &ctx, &mut images, None, None);
                change_quad(&mut app, 3);
                let (_, partial) = paint(&mut app, &ctx, &mut images, None, Some(Key::Enter));
                assert!(partial, "mode={mode} depth={depth} smart={smart}");
                assert_eq!(images.0[&id].pixels, oracle(&app, None).pixels, "mode={mode} depth={depth} smart={smart}");
            }
        }
    }
}

fn paint(app: &mut PhotocraftApp, ctx: &egui::Context, images: &mut Uploaded, output: Option<u32>, key: Option<Key>) -> (TextureId, bool) {
    let events = key.into_iter().map(|key| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }).collect();
    let mut id = None;
    let mut frame = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
        crate::shortcuts::handle(app, ui.ctx());
        id = Some(crate::canvas::ensure_texture(app, ui.ctx(), 0, output).unwrap().0);
    });
    let id = id.unwrap();
    let partial = frame.textures_delta.set.get(&id).is_some_and(|updates| updates.iter().any(|update| update.pos.is_some()));
    images.apply(&frame);
    frame.textures_delta.clear();
    (id, partial)
}

fn oracle(app: &PhotocraftApp, output: Option<u32>) -> ColorImage {
    let doc = &app.session.active().unwrap().doc;
    let full = photocraft_compose::render(doc, doc.bounds());
    let rgba = match app.session.color.canvas_display_for(doc, output.or(app.session.color.main_display)) {
        Ok(display) if !display.is_identity() => display.to_rgba8(&full),
        _ => full.to_rgba8(),
    };
    ColorImage::from_rgba_unmultiplied([rgba.width as usize, rgba.height as usize], &rgba.pixels)
}

fn change_quad(app: &mut PhotocraftApp, kind: u8) {
    let transform = app.ui.transform.as_mut().unwrap();
    let [x0, y0, _, _] = transform.rect;
    let [cx, cy] = transform.pivot;
    let angle = 20.0f64.to_radians();
    for point in &mut transform.quad {
        let [x, y] = *point;
        *point = match kind {
            0 => [x, y],
            1 => [x + 17.0, y + 13.0],
            2 => [x0 + (x - x0) * 1.25, y0 + (y - y0) * 1.25],
            3 => [cx + (x - cx) * angle.cos() - (y - cy) * angle.sin(), cy + (x - cx) * angle.sin() + (y - cy) * angle.cos()],
            _ => [x - 145.0, y - 108.0],
        };
    }
}

#[test]
fn actual_cache_partial_upload_matches_full_canvas_for_pixel_and_smart_placement_v91() {
    for smart in [false, true] {
        for kind in 0..=4 {
            let (mut app, ctx) = placed(smart);
            crate::transform_tool::begin_placed(&mut app, &ctx).unwrap();
            assert!(app.transform_preview.as_ref().unwrap().damage_origin.is_some());
            let mut images = Uploaded::default();
            let (id, _) = paint(&mut app, &ctx, &mut images, None, None);
            let backdrop = images.0[&id].clone();
            let before = app.session.active().unwrap().revision;
            change_quad(&mut app, kind);
            let (final_id, partial) = paint(&mut app, &ctx, &mut images, None, Some(Key::Enter));
            assert!(app.ui.transform.is_none() && app.transform_preview.is_none());
            assert_eq!(app.session.active().unwrap().revision, before + u64::from(kind != 0));
            assert_eq!(final_id, id);
            assert!(partial, "must update the actual existing texture, smart={smart} kind={kind}");
            assert_eq!(app.perf.last_refresh, "rect");
            assert!(app.perf.last_refresh_px > 0 && app.perf.last_refresh_px < 320 * 240);
            let expected = oracle(&app, None);
            assert_eq!(images.0[&id].pixels, expected.pixels, "full pixel oracle, smart={smart} kind={kind}");
            assert_ne!(backdrop.pixels, expected.pixels, "untouched confirmation must not leave the layer hidden");
            let dirty = app.transform_damage.as_ref().unwrap().damage;
            for y in 0..240usize {
                for x in 0..320usize {
                    if (x as i32) < dirty.x0 || (y as i32) < dirty.y0 || x as i32 >= dirty.x1 || y as i32 >= dirty.y1 {
                        assert_eq!(backdrop.pixels[y * 320 + x], images.0[&id].pixels[y * 320 + x], "outside ROI must be retained");
                    }
                }
            }
        }
    }
}

#[test]
fn unchanged_revision_transition_updates_each_real_output_cache_independently_v91() {
    let (mut app, ctx) = placed(false);
    crate::transform_tool::begin_placed(&mut app, &ctx).unwrap();
    let mut images = Uploaded::default();
    let (main, _) = paint(&mut app, &ctx, &mut images, None, None);
    let (other, _) = paint(&mut app, &ctx, &mut images, Some(42), None);
    assert_ne!(main, other);
    let revision = app.session.active().unwrap().revision;
    let (_, first_partial) = paint(&mut app, &ctx, &mut images, None, Some(Key::Enter));
    let (_, other_partial) = paint(&mut app, &ctx, &mut images, Some(42), None);
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert!(first_partial && other_partial);
    assert_eq!(images.0[&main].pixels, oracle(&app, None).pixels);
    assert_eq!(images.0[&other].pixels, oracle(&app, Some(42)).pixels);
}

#[test]
fn unpainted_preview_and_new_output_keep_full_refresh_v91() {
    let (mut app, ctx) = placed(false);
    let mut images = Uploaded::default();
    paint(&mut app, &ctx, &mut images, None, None);
    crate::transform_tool::begin_placed(&mut app, &ctx).unwrap();
    let (id, partial) = paint(&mut app, &ctx, &mut images, None, Some(Key::Enter));
    assert!(!partial);
    assert_eq!(app.perf.last_refresh, "full");
    assert_eq!(images.0[&id].pixels, oracle(&app, None).pixels);
    let (other, partial) = paint(&mut app, &ctx, &mut images, Some(99), None);
    assert!(!partial);
    assert_eq!(app.perf.last_refresh, "full");
    assert_eq!(images.0[&other].pixels, oracle(&app, Some(99)).pixels);
}

#[test]
fn wrong_revision_session_key_snapshot_and_document_do_not_reuse_transition_v91() {
    let (mut app, ctx) = placed(false);
    crate::transform_tool::begin_placed(&mut app, &ctx).unwrap();
    let mut images = Uploaded::default();
    paint(&mut app, &ctx, &mut images, None, None);
    change_quad(&mut app, 1);
    crate::transform_tool::commit(&mut app);
    let transition = app.transform_damage.as_ref().unwrap();
    let seen = (transition.before_revision, transition.preview_key);
    let now = (transition.after_revision, 0);
    assert!(damage(&app, 0, seen, now, 0).is_some());
    assert!(damage(&app, 0, (seen.0 + 1, seen.1), now, 0).is_none());
    assert!(damage(&app, 0, (seen.0, seen.1 + 1), now, 0).is_none());
    assert!(damage(&app, 0, seen, (now.0 + 1, 0), 0).is_none());
    assert!(damage(&app, 0, seen, (now.0, 1), 0).is_none());
    assert!(damage(&app, 0, seen, now, 1).is_none());
    assert!(damage(&app, 1, seen, now, 0).is_none());
    app.transform_damage.as_mut().unwrap().session += 1;
    assert!(damage(&app, 0, seen, now, 0).is_none());
    app.transform_damage.as_mut().unwrap().session -= 1;
    // Preserved revision/ID with a different document Arc must still invalidate the token.
    let state = app.session.active_mut().unwrap();
    state.doc = Arc::new(state.doc.as_ref().clone());
    assert!(damage(&app, 0, seen, now, 0).is_none());
    let (_, partial) = paint(&mut app, &ctx, &mut images, None, None);
    assert!(!partial);
    assert_eq!(app.perf.last_refresh, "full");
}

#[test]
fn dependency_and_edit_conflict_guards_keep_full_path_v91() {
    for case in 0..7 {
        let (mut app, _) = placed(false);
        let state = app.session.active_mut().unwrap();
        let id = state.active_layer.unwrap();
        let revision = state.revision;
        let doc = Arc::make_mut(&mut state.doc);
        if case == 6 {
            doc.layers.push(photocraft_doc::Layer::new("overlying", LayerContent::Adjustment(photocraft_doc::Adjustment::Invert)));
        } else {
            let layer = doc.layer_mut(id).unwrap();
            match case {
                0 => layer.mask = Some(photocraft_doc::LayerMask::reveal_all()),
                1 => layer.clipped = true,
                2 => layer.blend = photocraft_color::BlendMode::Multiply,
                3 => layer.opacity = 0.4,
                4 => layer.fill_opacity = 0.6,
                _ => layer.link_group = Some(9),
            }
        }
        assert!(Origin::capture(&state.doc, revision, id).is_none(), "case={case}");
    }
    let (mut app, ctx) = placed(false);
    crate::transform_tool::begin_placed(&mut app, &ctx).unwrap();
    let mut images = Uploaded::default();
    paint(&mut app, &ctx, &mut images, None, None);
    app.session.execute("layer.new.layer", json!({})).unwrap();
    change_quad(&mut app, 1);
    crate::transform_tool::commit(&mut app);
    assert!(app.transform_damage.is_none(), "concurrent document edit invalidates source identity/revision");
    let (_, partial) = paint(&mut app, &ctx, &mut images, None, None);
    assert!(!partial);
    assert_eq!(app.perf.last_refresh, "full");
}

#[test]
fn filtered_or_unavailable_smart_cache_cannot_arm_partial_transition_v91() {
    for case in 0..5 {
        let (mut app, _) = placed(true);
        let state = app.session.active_mut().unwrap();
        let id = state.active_layer.unwrap();
        let revision = state.revision;
        let doc = Arc::make_mut(&mut state.doc);
        let LayerContent::Smart(smart) = &mut doc.layer_mut(id).unwrap().content else { panic!("expected placed smart object") };
        match case {
            0 => smart.smart_filters.push(photocraft_doc::SmartFilter {
                command: "filter.blur.gaussianBlur".into(),
                params: json!({"radius":1}),
                blend: photocraft_color::BlendMode::Normal,
                opacity: 1.0,
                visible: true,
            }),
            1 => smart.filter_mask = Some(photocraft_doc::LayerMask::reveal_all()),
            2 => smart.perspective = Some([1., 0., 0., 0., 1., 0., 0., 0., 1.]),
            3 => smart.cache = None,
            _ => smart.psd_raw = Some(Arc::new(vec![])),
        }
        assert!(Origin::capture(&state.doc, revision, id).is_none(), "smart case={case}");
    }
}

#[test]
fn cancel_failed_transform_and_undo_redo_do_not_leave_a_partial_token_v91() {
    let (mut app, ctx) = placed(false);
    crate::transform_tool::begin_placed(&mut app, &ctx).unwrap();
    let mut images = Uploaded::default();
    paint(&mut app, &ctx, &mut images, None, None);
    crate::transform_tool::cancel(&mut app);
    assert!(app.transform_damage.is_none());
    let (_, partial) = paint(&mut app, &ctx, &mut images, None, None);
    assert!(!partial);
    let (mut app, ctx) = placed(false);
    let state = app.session.active_mut().unwrap();
    let id = state.active_layer.unwrap();
    Arc::make_mut(&mut state.doc).layer_mut(id).unwrap().locks.position = true;
    crate::transform_tool::begin_placed(&mut app, &ctx).unwrap();
    let mut images = Uploaded::default();
    paint(&mut app, &ctx, &mut images, None, None);
    change_quad(&mut app, 1);
    crate::transform_tool::commit(&mut app);
    assert!(app.transform_damage.is_none());
    assert!(app.ui.status.contains("locked"));
    let (_, partial) = paint(&mut app, &ctx, &mut images, None, None);
    assert!(!partial);
    let (mut app, ctx) = placed(false);
    crate::transform_tool::begin_placed(&mut app, &ctx).unwrap();
    let mut images = Uploaded::default();
    paint(&mut app, &ctx, &mut images, None, None);
    paint(&mut app, &ctx, &mut images, None, Some(Key::Enter));
    assert!(app.transform_damage.is_some());
    for command in ["edit.undo", "edit.redo"] {
        app.run(command, json!({})).unwrap();
        let (id, partial) = paint(&mut app, &ctx, &mut images, None, None);
        assert!(!partial);
        assert_eq!(app.perf.last_refresh, "full");
        assert_eq!(images.0[&id].pixels, oracle(&app, None).pixels);
    }
}
