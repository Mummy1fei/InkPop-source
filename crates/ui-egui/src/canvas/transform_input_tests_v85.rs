//! Actual canvas response routing: a transform must own its drag before base-tool navigation.
//! These frames never call tool_event, transform_tool::pointer, or ui.pointer directly.
use super::*;
use egui::{Event, Key, Modifiers};
use std::sync::Arc;

struct Frames {
    app: PhotocraftApp,
    ctx: egui::Context,
    view: View,
    frame: u32,
}

impl Frames {
    fn new(tool: Tool, rotation: f32, flip: bool) -> Self {
        Self::with_legacy(tool, rotation, flip, false)
    }

    fn with_legacy(tool: Tool, rotation: f32, flip: bool, legacy: bool) -> Self {
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, Default::default());
        ctx.data_mut(|data| data.insert_temp(egui::Id::new("v85-legacy-transform-input"), legacy));
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.background_jobs = false;
        app.run("file.new", json!({"width":240,"height":180,"background":"transparent"})).unwrap();
        app.session
            .edit("Original transform subject", |doc, active| {
                let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
                surface.fill_rect(DRect::new(40, 40, 140, 120), &[0.8, 0.2, 0.1, 1.]);
                surface.fill_rect(DRect::new(48, 48, 68, 100), &[0.1, 0.3, 0.8, 1.]);
                Ok(())
            })
            .unwrap();
        app.ui.tool = tool;
        app.ui.extras.rulers = false;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        app.ui.view.flip_horizontal = flip;
        app.sync_views();
        let view = View { zoom: 1., center: [120., 90.], rotation, doc_size: [240, 180], fit_pending: false };
        app.ui.views[0] = view.clone();
        let mut frames = Self { app, ctx, view, frame: 0 };
        frames.run(vec![], Modifiers::NONE);
        let ctx = frames.ctx.clone();
        crate::menus::invoke(&mut frames.app, &ctx, "edit.freeTransform", json!({})).unwrap();
        assert_eq!(frames.app.ui.transform.as_ref().unwrap().rect, [40., 40., 140., 120.]);
        frames.run(vec![], Modifiers::NONE);
        frames
    }

    fn rect() -> Rect {
        Rect::from_min_size(pos2(32., 32.), vec2(440., 320.))
    }

    fn screen(&self, point: [f64; 2]) -> Pos2 {
        ViewXform { rect: Self::rect(), zoom: self.view.zoom, center: self.view.center, rotation: self.view.rotation, flip: self.app.ui.view.flip_horizontal }
            .to_screen(point[0] as f32, point[1] as f32)
    }

    fn button(point: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
        Event::PointerButton { pos: point, button: PointerButton::Primary, pressed, modifiers }
    }

    fn key(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }
    }

    fn run(&mut self, mut events: Vec<Event>, modifiers: Modifiers) {
        self.frame += 1;
        events.insert(0, Event::ModifiersChanged(modifiers));
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(520., 400.))),
            time: Some(f64::from(self.frame) / 60.),
            focused: true,
            events,
            ..Default::default()
        };
        input.viewports.entry(egui::ViewportId::ROOT).or_default().focused = Some(true);
        let app = &mut self.app;
        let view = &mut self.view;
        app.last_canvas_rect = Self::rect();
        app.ui.views[0] = view.clone();
        let mut output = self.ctx.run_ui(input, |ui| {
            crate::transform_tool::track_steps(app, ui.ctx());
            crate::shortcuts::handle(app, ui.ctx());
            *view = canvas_view(app, ui, 0, Self::rect(), view.clone(), true);
        });
        output.textures_delta.clear();
        self.app.ui.views[0] = self.view.clone();
    }

    fn drag(&mut self, from: [f64; 2], to: [f64; 2]) {
        let a = self.screen(from);
        let b = self.screen(to);
        self.run(vec![Event::PointerMoved(a)], Modifiers::NONE);
        self.run(vec![Self::button(a, true, Modifiers::NONE)], Modifiers::NONE);
        self.run(vec![Event::PointerMoved(b)], Modifiers::NONE);
        self.run(vec![Self::button(b, false, Modifiers::NONE)], Modifiers::NONE);
    }

    fn quad(&self) -> [[f64; 2]; 4] {
        self.app.ui.transform.as_ref().unwrap().quad
    }
}

fn assert_quad(actual: [[f64; 2]; 4], expected: [[f64; 2]; 4]) {
    for (a, b) in actual.into_iter().zip(expected) {
        assert!((a[0] - b[0]).abs() < 0.002 && (a[1] - b[1]).abs() < 0.002, "{actual:?} != {expected:?}");
    }
}

fn assert_camera(frames: &Frames, before: &View) {
    assert_eq!(frames.view.zoom, before.zoom, "a layer handle must not scrub-zoom the canvas");
    assert_eq!(frames.view.center, before.center, "a layer handle must not pan the canvas");
    assert_eq!(frames.view.rotation, before.rotation, "a layer handle must not rotate the camera");
    assert_eq!(frames.app.session.active().unwrap().doc.size, photocraft_geom::Size::new(240, 180));
}

#[test]
fn free_transform_all_corners_scale_the_layer_with_navigation_and_selection_tools_selected() {
    let original = [[40., 40.], [140., 40.], [140., 120.], [40., 120.]];
    for tool in [Tool::Zoom, Tool::Hand, Tool::RotateView, Tool::Lasso, Tool::PolygonLasso, Tool::Brush, Tool::Move] {
        for (rotation, flip) in [(0., false), (23., true)] {
            for index in 0..4 {
                let mut f = Frames::new(tool, rotation, flip);
                let view = f.view.clone();
                let doc = f.app.session.active().unwrap().doc.clone();
                let history = f.app.session.active().unwrap().history.past_len();
                let anchor = original[(index + 2) % 4];
                let expected = original.map(|p| [anchor[0] + (p[0] - anchor[0]) * 1.2, anchor[1] + (p[1] - anchor[1]) * 1.2]);
                f.drag(original[index], expected[index]);
                assert_quad(f.quad(), expected);
                assert_camera(&f, &view);
                assert_eq!(f.app.ui.tool, tool, "temporary routing must not commit by changing the selected tool");
                assert!(Arc::ptr_eq(&doc, &f.app.session.active().unwrap().doc), "transform preview cannot edit source pixels");
                assert_eq!(f.app.session.active().unwrap().history.past_len(), history);
                // A plain hover after release must not keep transforming or begin navigation.
                f.run(vec![Event::PointerMoved(f.screen([210., 150.]))], Modifiers::NONE);
                assert_quad(f.quad(), expected);
                assert_camera(&f, &view);
                f.run(vec![Frames::key(Key::Enter, true, Modifiers::NONE)], Modifiers::NONE);
                assert!(f.app.ui.transform.is_none() && f.app.transform_preview.is_none());
                assert_eq!(f.app.session.active().unwrap().history.past_len(), history + 1);
                assert_camera(&f, &view);
                assert!(*f.app.session.active().unwrap().doc != *doc);
                f.app.run("edit.undo", json!({})).unwrap();
                assert!(*f.app.session.active().unwrap().doc == *doc, "one undo restores the entire original layer");
            }
        }
    }
}

#[test]
fn free_transform_edges_and_outside_rotation_do_not_navigate_the_camera() {
    for tool in [Tool::Zoom, Tool::Hand, Tool::RotateView, Tool::Lasso, Tool::Brush, Tool::Move] {
        let mut f = Frames::new(tool, 0., false);
        let view = f.view.clone();
        f.drag([90., 40.], [90., 24.]);
        assert_quad(f.quad(), [[40., 24.], [140., 24.], [140., 120.], [40., 120.]]);
        assert_camera(&f, &view);
        // Rotation is tested on a fresh box so expected coordinates are independent.
        let mut f = Frames::new(tool, 0., false);
        f.drag([185., 80.], [90., 175.]);
        assert_quad(f.quad(), [[130., 30.], [130., 130.], [50., 130.], [50., 30.]]);
        assert_camera(&f, &view);
    }
}

#[test]
fn explicit_space_navigation_temporarily_keeps_the_transform_box_unchanged() {
    for zoom in [false, true] {
        let mut f = Frames::new(Tool::Zoom, 0., false);
        let original = f.quad();
        let before = f.view.clone();
        let modifiers = if zoom { Modifiers { ctrl: true, command: true, ..Default::default() } } else { Modifiers::NONE };
        f.run(vec![Frames::key(Key::Space, true, modifiers)], modifiers);
        let a = f.screen([140., 40.]);
        let b = a + vec2(30., 20.);
        f.run(vec![Event::PointerMoved(a), Frames::button(a, true, modifiers)], modifiers);
        f.run(vec![Event::PointerMoved(b)], modifiers);
        assert_quad(f.quad(), original);
        if zoom {
            assert!(f.view.zoom > before.zoom);
        } else {
            assert_ne!(f.view.center, before.center);
        }
        f.run(vec![Frames::button(b, false, modifiers)], modifiers);
        f.run(vec![Frames::key(Key::Space, false, Modifiers::NONE)], Modifiers::NONE);
        let navigated = f.view.clone();
        f.drag([140., 120.], [160., 136.]);
        assert_quad(f.quad(), [[40., 40.], [160., 40.], [160., 136.], [40., 136.]]);
        assert_camera(&f, &navigated);
        assert_eq!(f.app.ui.tool, Tool::Zoom);
    }
}

#[test]
fn pressing_navigation_keys_mid_handle_drag_cannot_steal_its_release() {
    for modifiers in [Modifiers::NONE, Modifiers { ctrl: true, command: true, ..Default::default() }] {
        let mut f = Frames::new(Tool::Zoom, 0., false);
        let view = f.view.clone();
        let a = f.screen([140., 120.]);
        let b = f.screen([150., 128.]);
        let c = f.screen([160., 136.]);
        f.run(vec![Event::PointerMoved(a)], Modifiers::NONE);
        f.run(vec![Frames::button(a, true, Modifiers::NONE)], Modifiers::NONE);
        f.run(vec![Event::PointerMoved(b)], Modifiers::NONE);
        f.run(vec![Frames::key(Key::Space, true, modifiers), Event::PointerMoved(c)], modifiers);
        // Ownership stays with Free Transform, including its existing Ctrl-corner
        // distortion semantics. Space alone keeps proportional scaling instead.
        let expected =
            if modifiers.command { [[40., 40.], [140., 40.], [160., 136.], [40., 120.]] } else { [[40., 40.], [160., 40.], [160., 136.], [40., 136.]] };
        assert_quad(f.quad(), expected);
        assert_camera(&f, &view);
        f.run(vec![Frames::button(c, false, modifiers)], modifiers);
        f.run(vec![Frames::key(Key::Space, false, Modifiers::NONE)], Modifiers::NONE);
        assert_quad(f.quad(), expected);
        f.run(vec![Event::PointerMoved(f.screen([210., 150.]))], Modifiers::NONE);
        assert_quad(f.quad(), expected);
        assert_camera(&f, &view);
    }
}

#[test]
fn identical_real_corner_drag_reproduces_the_legacy_camera_navigation_counterexample() {
    for tool in [Tool::Zoom, Tool::Hand] {
        let mut old = Frames::with_legacy(tool, 0., false, true);
        let original = old.quad();
        let camera = old.view.clone();
        old.drag([140., 120.], [160., 136.]);
        assert_quad(old.quad(), original);
        if tool == Tool::Zoom {
            assert!(old.view.zoom > camera.zoom, "the legacy same corner drag wrongly scrub-zooms the view");
        } else {
            assert_ne!(old.view.center, camera.center, "the legacy same corner drag wrongly pans the view");
        }
        let mut candidate = Frames::with_legacy(tool, 0., false, false);
        let camera = candidate.view.clone();
        candidate.drag([140., 120.], [160., 136.]);
        assert_quad(candidate.quad(), [[40., 40.], [160., 40.], [160., 136.], [40., 136.]]);
        assert_camera(&candidate, &camera);
    }
}
