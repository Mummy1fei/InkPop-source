#![cfg(not(target_arch = "wasm32"))]

use super::*;
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, Shape, TouchPhase, Vec2, pos2, vec2};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    ReferenceLegacyOnTouch,
    DependencyDefault,
    Normal,
    Rows,
}

struct Fixture {
    ctx: Context,
    mode: Mode,
    time: f64,
    offset: Vec2,
    inner: Rect,
    shapes: Vec<egui::epaint::ClippedShape>,
}

impl Fixture {
    fn new(mode: Mode, theme: ThemeKind, custom: bool) -> Self {
        let ctx = Context::default();
        let colors = if custom {
            StudioLightColors {
                colors: [(ColorSlot::ScrollbarHandle, PaletteRgb::new([47, 99, 151])), (ColorSlot::ScrollbarTrack, PaletteRgb::new([215, 225, 235]))].into(),
                ..Default::default()
            }
        } else {
            StudioLightColors::default()
        };
        apply(&ctx, theme, &colors);
        let mut fixture = Self { ctx, mode, time: 0.0, offset: Vec2::ZERO, inner: Rect::NOTHING, shapes: Vec::new() };
        for _ in 0..4 {
            fixture.step(Vec::new());
        }
        fixture
    }

    fn step(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let mut metrics = (Vec2::ZERO, Rect::NOTHING);
        let mode = self.mode;
        let mut output = self.ctx.run_ui(
            egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(340.0, 260.0))), events, time: Some(self.time), ..Default::default() },
            |ui| {
                ui.spacing_mut().scroll.floating = false;
                ui.spacing_mut().scroll.bar_width = 12.0;
                let area = egui::ScrollArea::vertical()
                    .id_salt("desktop-scroll-interaction-v70")
                    .max_height(160.0)
                    .max_width(240.0)
                    .auto_shrink([false, false])
                    .animated(false)
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible);
                let out = match mode {
                    Mode::ReferenceLegacyOnTouch => area
                        .scroll_source(egui::containers::scroll_area::ScrollSource {
                            scroll_bar: true,
                            drag: egui::containers::scroll_area::DragScroll::OnTouch,
                            mouse_wheel: true,
                        })
                        .show(ui, |body| {
                            body.allocate_space(vec2(200.0, 800.0));
                            7
                        }),
                    Mode::DependencyDefault => area.show(ui, |body| {
                        body.allocate_space(vec2(200.0, 800.0));
                        7
                    }),
                    // ALL deliberately requests drag too: project wrappers must
                    // enforce the desktop policy even for that existing caller.
                    Mode::Normal => area.scroll_source(egui::containers::scroll_area::ScrollSource::ALL).show_palette(ui, |body| {
                        body.allocate_space(vec2(200.0, 800.0));
                        7
                    }),
                    Mode::Rows => area.show_palette_rows(ui, 20.0, 80, |body, range| {
                        for _ in range {
                            body.allocate_space(vec2(200.0, 20.0));
                        }
                        7
                    }),
                };
                assert_eq!(out.inner, 7, "the helper preserves the closure return value");
                metrics = (out.state.offset, out.inner_rect);
            },
        );
        output.textures_delta.clear();
        self.shapes = output.shapes;
        (self.offset, self.inner) = metrics;
    }

    fn detect_touch_device(&mut self) {
        let outside = pos2(-50.0, -50.0);
        self.step(vec![touch(TouchPhase::Start, outside)]);
        self.step(vec![touch(TouchPhase::End, outside)]);
        assert!(self.ctx.input(|input| input.has_touch_screen()));
    }

    fn drag_content(&mut self, contact: bool) {
        let start = self.inner.center();
        let end = start - vec2(0.0, 60.0);
        self.step(vec![Event::PointerMoved(start)]);
        let mut down = vec![pointer(start, true)];
        if contact {
            down.push(touch(TouchPhase::Start, start));
        }
        self.step(down);
        for i in 1..=4 {
            let p = start.lerp(end, i as f32 / 4.0);
            let mut events = vec![Event::PointerMoved(p)];
            if contact {
                events.push(touch(TouchPhase::Move, p));
            }
            self.step(events);
        }
        let mut up = vec![pointer(end, false)];
        if contact {
            up.push(touch(TouchPhase::End, end));
        }
        self.step(up);
        for _ in 0..5 {
            self.step(Vec::new());
        }
    }

    fn handle(&self) -> Rect {
        fn find(shape: &Shape, inner: Rect, color: Color32) -> Option<Rect> {
            match shape {
                Shape::Rect(r) if r.fill == color && r.rect.left() >= inner.right() - 1.0 && r.rect.width() <= 14.0 => Some(r.rect),
                Shape::Vec(children) => children.iter().find_map(|shape| find(shape, inner, color)),
                _ => None,
            }
        }
        let color = if self.mode == Mode::DependencyDefault {
            self.ctx.global_style().visuals.widgets.inactive.fg_stroke.color
        } else {
            Color32::from_rgb(47, 99, 151)
        };
        self.shapes.iter().find_map(|shape| find(&shape.shape, self.inner, color)).expect("an actual scrollbar handle is painted")
    }
}

fn pointer(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, pressed, button: PointerButton::Primary, modifiers: Modifiers::NONE }
}

fn touch(phase: TouchPhase, pos: Pos2) -> Event {
    Event::Touch { device_id: egui::TouchDeviceId(70), id: egui::TouchId(1), phase, pos, force: Some(0.65) }
}

#[test]
fn content_pointer_drags_do_not_scroll_after_a_touch_capable_device_was_seen_v70() {
    // A positive reference proves these events exercise the old default path,
    // rather than passing because a synthetic drag was never recognised.
    for contact in [false, true] {
        let mut old = Fixture::new(Mode::ReferenceLegacyOnTouch, ThemeKind::StudioLight, false);
        old.detect_touch_device();
        old.drag_content(contact);
        assert!(old.offset.y > 20.0, "the same pointer/contact events scroll the old OnTouch default");
        for theme in ThemeKind::ALL {
            for mode in [Mode::DependencyDefault, Mode::Normal, Mode::Rows] {
                let mut fixed = Fixture::new(mode, theme, theme == ThemeKind::StudioLight);
                if !contact {
                    fixed.drag_content(false);
                    assert_eq!(fixed.offset, Vec2::ZERO, "mouse content drags are disabled before any touch event too");
                }
                fixed.detect_touch_device();
                fixed.drag_content(contact);
                assert_eq!(fixed.offset, Vec2::ZERO, "{theme:?}: content drags must not scroll or create kinetic motion");
            }
        }
    }
}

#[test]
fn scrollbar_thumbs_and_wheel_still_scroll_normal_and_virtual_lists_v70() {
    for mode in [Mode::DependencyDefault, Mode::Normal, Mode::Rows] {
        let mut wheel = Fixture::new(mode, ThemeKind::StudioLight, true);
        let hover = wheel.inner.center();
        wheel.step(vec![Event::PointerMoved(hover)]);
        wheel.step(vec![Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: vec2(0.0, -80.0), phase: TouchPhase::Move, modifiers: Modifiers::NONE }]);
        for _ in 0..8 {
            wheel.step(Vec::new());
        }
        assert!(wheel.offset.y > 10.0, "wheel scrolling remains enabled");

        let mut thumb = Fixture::new(mode, ThemeKind::StudioLight, true);
        let start = thumb.handle().center();
        thumb.step(vec![Event::PointerMoved(start)]);
        thumb.step(vec![pointer(start, true)]);
        thumb.step(vec![Event::PointerMoved(start + vec2(0.0, 65.0))]);
        thumb.step(vec![pointer(start + vec2(0.0, 65.0), false)]);
        assert!(thumb.offset.y > 100.0, "dragging the actual scrollbar handle remains enabled");
    }
}

#[test]
fn desktop_scroll_wrappers_preserve_virtual_row_ranges_ids_and_geometry_v70() {
    fn draw(wrapped: bool) -> (Id, Id, std::ops::Range<usize>, Rect, Vec2) {
        let ctx = Context::default();
        let mut result = None;
        for frame in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(320.0, 260.0))),
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                },
                |ui| {
                    let area = egui::ScrollArea::vertical()
                        .id_salt("desktop-virtual-id-v70")
                        .max_height(120.0)
                        .vertical_scroll_offset(178.0)
                        .scroll_source(desktop_scroll_source());
                    let body = |body: &mut Ui, range: std::ops::Range<usize>| {
                        for _ in range.clone() {
                            body.allocate_space(vec2(160.0, 20.0));
                        }
                        (body.id(), range)
                    };
                    let out = if wrapped { area.show_palette_rows(ui, 20.0, 100, body) } else { area.show_rows(ui, 20.0, 100, body) };
                    result = Some((out.id, out.inner.0, out.inner.1, out.inner_rect, out.content_size));
                },
            );
            output.textures_delta.clear();
        }
        result.unwrap()
    }
    assert_eq!(draw(false), draw(true));
}

struct ComboFixture {
    ctx: Context,
    time: f64,
    button: Rect,
    blank_header: Option<Rect>,
    first_row: Option<Rect>,
}

impl ComboFixture {
    fn new() -> Self {
        let mut fixture = Self { ctx: Context::default(), time: 0.0, button: Rect::NOTHING, blank_header: None, first_row: None };
        for _ in 0..3 {
            fixture.step(Vec::new());
        }
        let p = fixture.button.center();
        fixture.step(vec![Event::PointerMoved(p)]);
        fixture.step(vec![pointer(p, true)]);
        fixture.step(vec![pointer(p, false)]);
        for _ in 0..3 {
            fixture.step(Vec::new());
        }
        assert!(fixture.blank_header.is_some() && fixture.first_row.is_some(), "the real long ComboBox popup must be open");
        fixture
    }

    fn step(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let mut header = None;
        let mut first_row = None;
        let mut button = Rect::NOTHING;
        let mut output = self.ctx.run_ui(
            egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 360.0))), time: Some(self.time), events, ..Default::default() },
            |ui| {
                let out =
                    egui::ComboBox::from_id_salt("dependency-combo-scroll-v70").selected_text("Choose an item").width(240.0).height(150.0).show_ui(ui, |ui| {
                        // A non-interactive heading exercises content background drag
                        // without accidentally choosing a row and closing the popup.
                        header = Some(ui.allocate_exact_size(vec2(220.0, 48.0), egui::Sense::hover()).0);
                        for index in 0..80 {
                            let row = ui.selectable_label(false, format!("List item {index}"));
                            if index == 0 {
                                first_row = Some(row.rect);
                            }
                        }
                    });
                button = out.response.rect;
            },
        );
        output.textures_delta.clear();
        self.button = button;
        self.blank_header = header;
        self.first_row = first_row;
    }
}

#[test]
fn real_long_combobox_dependency_popup_rejects_content_drag_and_keeps_wheel_v70() {
    for contact in [false, true] {
        let mut fixture = ComboFixture::new();
        fixture.step(vec![touch(TouchPhase::Start, pos2(-50.0, -50.0))]);
        fixture.step(vec![touch(TouchPhase::End, pos2(-50.0, -50.0))]);
        assert!(fixture.ctx.input(|input| input.has_touch_screen()));
        let original = fixture.first_row.expect("the actual ComboBox list is open");
        let start = fixture.blank_header.unwrap().center() + vec2(0.0, 6.0);
        let end = start - vec2(0.0, 25.0);
        fixture.step(vec![Event::PointerMoved(start)]);
        let mut down = vec![pointer(start, true)];
        if contact {
            down.push(touch(TouchPhase::Start, start));
        }
        fixture.step(down);
        for i in 1..=4 {
            let p = start.lerp(end, i as f32 / 4.0);
            let mut moved = vec![Event::PointerMoved(p)];
            if contact {
                moved.push(touch(TouchPhase::Move, p));
            }
            fixture.step(moved);
        }
        let mut up = vec![pointer(end, false)];
        if contact {
            up.push(touch(TouchPhase::End, end));
        }
        fixture.step(up);
        for _ in 0..5 {
            fixture.step(Vec::new());
        }
        assert_eq!(fixture.first_row, Some(original), "the dependency's internal ScrollArea does not pan or gain kinetic motion");
        fixture.step(vec![Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -80.0),
            phase: TouchPhase::Move,
            modifiers: Modifiers::NONE,
        }]);
        for _ in 0..8 {
            fixture.step(Vec::new());
        }
        let scrolled = fixture.first_row.expect("scrolling the real menu keeps it open");
        assert!(scrolled.top() < original.top() - 10.0, "the actual dependency popup remains wheel-scrollable");
    }
}
