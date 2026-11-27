//! Real egui input frames: no control ui.pointer, direct tool_event, or extra rescue frame.
use super::*;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

struct FrameResult {
    uploads: HashSet<egui::TextureId>,
}

#[derive(Default)]
struct UploadedImages(HashMap<egui::TextureId, egui::ColorImage>);

impl UploadedImages {
    fn apply(&mut self, output: &egui::FullOutput) {
        for (id, updates) in &output.textures_delta.set {
            for delta in updates {
                let egui::ImageData::Color(image) = &delta.image;
                if let Some([x0, y0]) = delta.pos {
                    let current = self.0.get_mut(id).expect("partial update must have an existing allocation");
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

struct Frames {
    app: PhotocraftApp,
    ctx: egui::Context,
    view: View,
    images: UploadedImages,
    frame: u32,
}

impl Frames {
    fn new(tool: Tool, flip: bool, legacy: bool) -> Self {
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, Default::default());
        ctx.data_mut(|data| data.insert_temp(egui::Id::new("v84-legacy-paint-input-frame"), legacy));
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.background_jobs = false; // Synchronous existing small Mixer/Smudge path, no timing sleeps.
        app.run("file.new", json!({"width":240,"height":160,"background":"transparent"})).unwrap();
        app.session
            .edit("Original content fixture", |doc, active| {
                let bounds = doc.bounds();
                let layer = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
                layer.fill_rect(bounds, &[0.1, 0.3, 0.6, 1.]);
                layer.fill_rect(DRect::new(32, 24, 60, 140), &[1., 1., 1., 1.]);
                Ok(())
            })
            .unwrap();
        app.ui.tool = tool;
        app.sync_tool_brush();
        app.run(
            "tools.setBrush",
            json!({"brush":{"size":20.,"spacing":0.15,"pressureSize":true,"smoothing":{"amount":0.},
            "mixer":{"multicolor":true,"wet":0.,"load":1.,"mix":0.,"flow":1.,"loadAfterStroke":true}}}),
        )
        .unwrap();
        app.session.tools.foreground = [0.9, 0.1, 0.05, 1.];
        app.ui.tool_options.strength = 85.;
        app.ui.tool_options.sample_all_layers = false;
        app.ui.tool_options.finger_painting = false;
        app.ui.extras.rulers = false;
        app.ui.view.flip_horizontal = flip;
        app.session.edit_prefs(|prefs| prefs.tools.use_tablet_pressure = true);
        app.sync_views();
        let view = View { rotation: 0., center: [120., 80.], zoom: 1., doc_size: [240, 160], fit_pending: false };
        app.ui.views[0] = view.clone();
        Self { app, ctx, view, images: Default::default(), frame: 0 }
    }

    fn rect() -> Rect {
        Rect::from_min_size(pos2(32., 32.), vec2(400., 280.))
    }

    fn position(&self, point: [f32; 2]) -> Pos2 {
        ViewXform { rect: Self::rect(), zoom: self.view.zoom, center: self.view.center, rotation: self.view.rotation, flip: self.app.ui.view.flip_horizontal }
            .to_screen(point[0], point[1])
    }

    fn button(&self, point: [f32; 2], pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::PointerButton { pos: self.position(point), button: PointerButton::Primary, pressed, modifiers }
    }

    fn run(&mut self, mut events: Vec<egui::Event>, modifiers: egui::Modifiers, primary: bool, passive_too: bool) -> FrameResult {
        self.frame += 1;
        // The current egui input API carries modifier transitions as actual events.
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000., 380.))),
            time: Some(f64::from(self.frame) / 60.),
            focused: true,
            events,
            ..Default::default()
        };
        input.viewports.entry(egui::ViewportId::ROOT).or_default().focused = Some(true);
        let ctx = self.ctx.clone();
        let app = &mut self.app;
        let view = &mut self.view;
        app.last_canvas_rect = Self::rect();
        app.ui.views[0] = view.clone();
        let mut output = ctx.run_ui(input, |ui| {
            // Match the app's keyboard-before-canvas phase, including real Escape cancellation.
            crate::shortcuts::handle(app, ui.ctx());
            *view = canvas_view(app, ui, 0, Self::rect(), view.clone(), primary);
            if passive_too {
                let other = Self::rect().translate(vec2(480., 0.));
                let _ = canvas_view(app, ui, 0, other, view.clone(), false);
            }
        });
        self.app.ui.views[0] = self.view.clone();
        self.images.apply(&output);
        let uploads = output.textures_delta.set.keys().copied().collect();
        output.textures_delta.clear(); // The mirror consumed them; also satisfy egui's debug Drop guard.
        FrameResult { uploads }
    }

    fn warm(&mut self) {
        self.run(vec![], egui::Modifiers::NONE, true, false);
        self.run(vec![egui::Event::PointerMoved(self.position([40., 80.]))], egui::Modifiers::NONE, true, false);
    }

    fn canvas_texture(&self) -> egui::TextureId {
        let id = self.app.session.active().unwrap().doc.id;
        self.app.canvases[&(id, 0)].texture.as_ref().unwrap().id()
    }

    fn desired(&mut self) -> (Arc<Document>, u64, egui::ColorImage) {
        let (doc, key) = display_doc(&mut self.app, 0);
        let (display, display_key) = canvas_display(&self.app, &doc, None);
        let pixels = display_image(display.as_deref(), &photocraft_compose::render(&doc, doc.bounds()));
        (doc, key ^ display_key, pixels)
    }

    fn current(&mut self, expected: bool, phase: &str) {
        let (_, key, pixels) = self.desired();
        let state = self.app.session.active().unwrap();
        let cache = &self.app.canvases[&(state.doc.id, 0)];
        let fresh = cache.tex_revision == state.revision && cache.tex_preview_key == key;
        assert_eq!(fresh, expected, "{phase}: {:?}, flip={}", self.app.ui.tool, self.app.ui.view.flip_horizontal);
        if expected {
            assert!(self.images.0[&self.canvas_texture()] == pixels, "{phase}: upload must contain this frame's actual composited pixels");
        }
    }

    fn paint_count(&self) -> usize {
        self.app.session.journal.iter().filter(|(id, _)| matches!(id.as_str(), "paint.stroke" | "paint.pencil" | "paint.mixerBrush" | "paint.smudge")).count()
    }
}

fn drag_frames(tool: Tool, flip: bool, legacy: bool) {
    let mut f = Frames::new(tool, flip, legacy);
    f.warm();
    let before = f.app.session.active().unwrap().doc.clone();
    let revision = f.app.session.active().unwrap().revision;
    let history = f.app.session.active().unwrap().history.past_len();
    let original = f.images.0[&f.canvas_texture()].clone();
    f.run(vec![f.button([40., 80.], true, egui::Modifiers::NONE)], egui::Modifiers::NONE, true, false);
    let before_move = f.desired().2;
    // egui starts a drag after its real movement threshold, rather than on an artificial Down.
    f.run(vec![egui::Event::PointerMoved(f.position([100., 80.]))], egui::Modifiers::NONE, true, false);
    assert!(f.app.drag.as_ref().is_some_and(|drag| drag.tool == tool && drag.points.len() >= 2));
    assert!(Arc::ptr_eq(&before, &f.app.session.active().unwrap().doc));
    let (_, key, desired) = f.desired();
    assert_ne!(key, 0, "a real live preview was produced");
    assert!(desired != original, "{tool:?}: fixture must paint visible pixels, not two blank images");
    f.current(!legacy, "first actual drag frame");
    if legacy {
        assert!(f.images.0[&f.canvas_texture()] == before_move, "legacy counterexample presents the state before this frame's move");
    }
    f.run(vec![egui::Event::PointerMoved(f.position([165., 95.]))], egui::Modifiers::NONE, true, false);
    f.current(!legacy, "second actual move frame");
    assert_eq!(f.app.session.active().unwrap().revision, revision);
    assert_eq!(f.app.session.active().unwrap().history.past_len(), history);
    assert_eq!(f.paint_count(), 0, "previews must not create document commands");
    let last = [205., 105.];
    f.run(vec![egui::Event::PointerMoved(f.position(last)), f.button(last, false, egui::Modifiers::NONE)], egui::Modifiers::NONE, true, false);
    assert!(f.app.drag.is_none());
    assert_eq!(f.app.session.active().unwrap().revision, revision + 1);
    assert_eq!(f.app.session.active().unwrap().history.past_len(), history + 1);
    assert_eq!(f.paint_count(), 1, "one physical gesture creates exactly one paint command");
    f.current(!legacy, "actual release frame, with a new endpoint");
}

#[test]
fn real_input_frames_upload_brush_pencil_eraser_mixer_and_smudge_without_an_extra_frame() {
    for tool in [Tool::Brush, Tool::Pencil, Tool::Eraser, Tool::MixerBrush, Tool::Smudge] {
        for flip in [false, true] {
            drag_frames(tool, flip, false);
        }
    }
}

#[test]
fn the_same_real_pointer_frames_reproduce_the_legacy_one_frame_delay() {
    for tool in [Tool::Brush, Tool::Pencil, Tool::Eraser, Tool::MixerBrush, Tool::Smudge] {
        for flip in [false, true] {
            drag_frames(tool, flip, true);
        }
    }
}

#[test]
fn pen_touch_pressure_and_mouse_mapping_refresh_the_current_input_frame() {
    let mut f = Frames::new(Tool::Brush, false, false);
    f.warm();
    let touch = |point, phase, force| egui::Event::Touch { device_id: egui::TouchDeviceId(3), id: egui::TouchId(7), phase, pos: point, force: Some(force) };
    let a = f.position([40., 80.]);
    let b = f.position([100., 80.]);
    let c = f.position([165., 95.]);
    // Match egui-winit::on_touch. A press frame cannot yet be decidedly dragging,
    // even when a driver also batches moves into it; recognition happens later.
    f.run(
        vec![touch(a, egui::TouchPhase::Start, 0.2), egui::Event::PointerMoved(a), f.button([40., 80.], true, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
        true,
        false,
    );
    assert!(f.app.stylus.feed.get().is_none(), "exercise Windows Touch pressure, not an injected native feed");
    assert!((f.app.stylus.pressure() - 0.2).abs() < 1e-5, "the initial contact arrives in the press frame");
    f.run(
        vec![touch(b, egui::TouchPhase::Move, 0.5), egui::Event::PointerMoved(b), touch(c, egui::TouchPhase::Move, 0.8), egui::Event::PointerMoved(c)],
        egui::Modifiers::NONE,
        true,
        false,
    );
    let drag = f.app.drag.as_ref().expect("emulated primary pointer must begin a real pen drag");
    assert!((drag.points[0][2] - 0.2).abs() < 1e-5, "initial contact pressure must survive egui's delayed drag recognition");
    assert!(drag.points.iter().any(|point| (point[2] - 0.5).abs() < 1e-5), "keep the intermediate pressure, not just the last sample");
    assert!((drag.points.last().unwrap()[2] - 0.8).abs() < 1e-5);
    f.current(true, "pen Touch/Pointer same frame");
    f.run(
        vec![touch(c, egui::TouchPhase::End, 0.8), f.button([165., 95.], false, egui::Modifiers::NONE), egui::Event::PointerGone],
        egui::Modifiers::NONE,
        true,
        false,
    );
    assert_eq!(f.paint_count(), 1);
    f.current(true, "pen release same frame");
}

#[test]
fn mask_paint_and_committed_alpha_overlay_upload_in_the_actual_input_frame() {
    let mut mask = Frames::new(Tool::Brush, true, false);
    let id = mask.app.session.active().unwrap().active_layer.unwrap();
    mask.app
        .session
        .edit("Hidden mask fixture", |doc, _| {
            doc.layer_mut(id).unwrap().mask = Some(photocraft_doc::LayerMask::hide_all());
            Ok(())
        })
        .unwrap();
    mask.app.ui.mask_target = true;
    mask.app.session.tools.foreground = [1.; 4];
    mask.warm();
    mask.run(vec![mask.button([40., 80.], true, egui::Modifiers::NONE)], egui::Modifiers::NONE, true, false);
    mask.run(vec![egui::Event::PointerMoved(mask.position([100., 80.]))], egui::Modifiers::NONE, true, false);
    mask.current(true, "layer mask live pixels");
    mask.run(vec![mask.button([100., 80.], false, egui::Modifiers::NONE)], egui::Modifiers::NONE, true, false);
    mask.current(true, "layer mask release pixels");

    let mut alpha = Frames::new(Tool::Brush, false, false);
    alpha
        .app
        .session
        .edit("Alpha fixture", |doc, _| {
            doc.channels
                .push(photocraft_doc::AlphaChannel::new("Alpha", photocraft_raster::Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[1.])));
            Ok(())
        })
        .unwrap();
    alpha.app.session.active_mut().unwrap().channel_view.target = photocraft_engine::channel_cmds::ChannelTarget::Alpha(0);
    alpha.app.session.active_mut().unwrap().channel_view.set_alpha_visible(0, true);
    alpha.app.session.tools.foreground = [0., 0., 0., 1.];
    alpha.warm();
    let canvas = alpha.canvas_texture();
    let overlay = *alpha.images.0.iter().find(|(id, img)| **id != canvas && img.size == [240, 160]).expect("visible alpha overlay texture").0;
    let old = alpha.images.0[&overlay].clone();
    alpha.run(vec![alpha.button([40., 80.], true, egui::Modifiers::NONE)], egui::Modifiers::NONE, true, false);
    alpha.run(vec![egui::Event::PointerMoved(alpha.position([100., 80.]))], egui::Modifiers::NONE, true, false);
    // Existing alpha overlays are committed-document views, not a new live-alpha architecture.
    let output = alpha.run(vec![alpha.button([100., 80.], false, egui::Modifiers::NONE)], egui::Modifiers::NONE, true, false);
    let state = alpha.app.session.active().unwrap();
    let expected = crate::channel_view::render(&state.doc, &state.channel_view, state.doc.bounds(), 1, false).unwrap();
    assert!(expected != old.pixels, "fixture must change alpha overlay pixels");
    assert!(output.uploads.contains(&overlay), "release must upload the existing overlay in this frame");
    assert_eq!(alpha.images.0[&overlay].pixels, expected);
    alpha.current(true, "alpha release base texture state");
}

#[test]
fn clicks_passive_views_hover_and_temporary_eyedropper_never_double_paint() {
    let mut f = Frames::new(Tool::Brush, false, false);
    f.warm();
    let history = f.app.session.active().unwrap().history.past_len();
    let out = f.run(vec![egui::Event::PointerMoved(f.position([55., 85.]))], egui::Modifiers::NONE, true, false);
    assert!(!out.uploads.contains(&f.canvas_texture()), "hover alone does not rebuild the image");
    let at = [70., 70.];
    f.run(
        vec![egui::Event::PointerMoved(f.position(at)), f.button(at, true, egui::Modifiers::NONE), f.button(at, false, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
        true,
        true,
    );
    assert_eq!(f.paint_count(), 1, "a second passive view cannot paint the physical click again");
    assert_eq!(f.app.session.active().unwrap().history.past_len(), history + 1);
    f.current(true, "click commit, same frame");
    let before = f.app.session.active().unwrap().doc.clone();
    let paints = f.paint_count();
    let alt = egui::Modifiers { alt: true, ..Default::default() };
    f.run(vec![egui::Event::PointerMoved(f.position([40., 80.])), f.button([40., 80.], true, alt)], alt, true, false);
    f.run(vec![egui::Event::PointerMoved(f.position([100., 80.]))], alt, true, false);
    f.run(vec![f.button([100., 80.], false, alt)], alt, true, false);
    assert!(Arc::ptr_eq(&before, &f.app.session.active().unwrap().doc));
    assert_eq!(f.paint_count(), paints, "temporary Alt sampler never paints");
    f.run(vec![egui::Event::PointerMoved(f.position([40., 80.])), f.button([40., 80.], true, egui::Modifiers::NONE)], egui::Modifiers::NONE, false, false);
    f.run(vec![egui::Event::PointerMoved(f.position([100., 80.]))], egui::Modifiers::NONE, false, false);
    f.run(vec![f.button([100., 80.], false, egui::Modifiers::NONE)], egui::Modifiers::NONE, false, false);
    assert_eq!(f.paint_count(), paints, "a nonprimary canvas never consumes a paint gesture");
    assert!(Arc::ptr_eq(&before, &f.app.session.active().unwrap().doc));
}

#[test]
fn escape_cancels_a_real_selected_pixel_move_without_committing_a_hidden_paint() {
    let mut f = Frames::new(Tool::Move, false, false);
    f.app.run("select.rect", json!({"x":30,"y":30,"width":60,"height":90})).unwrap();
    f.app.ui.tool_options.move_auto_select = false;
    f.warm();
    let before = f.app.session.active().unwrap().doc.clone();
    let history = f.app.session.active().unwrap().history.past_len();
    f.run(vec![f.button([40., 80.], true, egui::Modifiers::NONE)], egui::Modifiers::NONE, true, false);
    f.run(vec![egui::Event::PointerMoved(f.position([100., 80.]))], egui::Modifiers::NONE, true, false);
    assert!(f.app.drag.as_ref().is_some_and(|drag| drag.tool == Tool::Move && drag.sel_move == Some(true)));
    f.run(
        vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
        egui::Modifiers::NONE,
        true,
        false,
    );
    assert!(f.app.drag.is_none());
    assert!(photocraft_engine::float_cmds::floating(f.app.session.active().unwrap()).is_none());
    assert!(f.app.session.active().unwrap().doc == before);
    assert_eq!(f.app.session.active().unwrap().history.past_len(), history);
    assert_eq!(f.paint_count(), 0);
    f.current(true, "Escape restores the committed source in the same frame");
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
#[ignore = "exports the actual input frame's reconstructed canvas texture; set PHOTOCRAFT_INPUT_FRAME_OUT to a new directory"]
fn export_actual_input_frame_images() {
    let out = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_INPUT_FRAME_OUT").expect("set PHOTOCRAFT_INPUT_FRAME_OUT"));
    assert!(!out.exists(), "preserve prior frame evidence; choose a new directory");
    std::fs::create_dir_all(&out).unwrap();
    let mut records = Vec::new();
    for legacy in [true, false] {
        let mut f = Frames::new(Tool::Brush, false, legacy);
        f.warm();
        let original = f.images.0[&f.canvas_texture()].clone();
        f.run(vec![f.button([40., 80.], true, egui::Modifiers::NONE)], egui::Modifiers::NONE, true, false);
        f.run(vec![egui::Event::PointerMoved(f.position([100., 80.]))], egui::Modifiers::NONE, true, false);
        f.current(!legacy, "exported actual input frame");
        let (_, wanted, pixels) = f.desired();
        let cache = &f.app.canvases[&(f.app.session.active().unwrap().doc.id, 0)];
        let image = &f.images.0[&f.canvas_texture()];
        let rgba: Vec<_> = image.pixels.iter().flat_map(Color32::to_srgba_unmultiplied).collect();
        let encoded =
            photocraft_codecs::Image::from_u8(image.size[0] as u32, image.size[1] as u32, photocraft_codecs::ChannelLayout::Rgba, rgba.clone()).unwrap();
        let png = photocraft_codecs::encode(&encoded, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        let name = if legacy { "legacy-move-frame.png" } else { "candidate-move-frame.png" };
        std::fs::write(out.join(name), &png).unwrap();
        let decoded = photocraft_codecs::decode(&png).unwrap();
        assert_eq!(decoded.to_rgba8(), rgba, "export must preserve the exact uploaded frame pixels");
        records.push(json!({"legacy":legacy,"path":name,"frame":f.frame,"physicalHardwareMeasured":false,
            "source":"RawInput frame ImageDelta mirror, captured before any next frame",
            "revision":f.app.session.active().unwrap().revision,"textureRevision":cache.tex_revision,
            "wantedPreviewKey":wanted,"texturePreviewKey":cache.tex_preview_key,
            "changedPixelsVsBefore":image.pixels.iter().zip(&original.pixels).filter(|(a,b)|a!=b).count(),
            "wrongPixelsVsCurrentDocument":image.pixels.iter().zip(&pixels.pixels).filter(|(a,b)|a!=b).count()}));
    }
    std::fs::write(out.join("summary.json"), serde_json::to_vec_pretty(&records).unwrap()).unwrap();
}
