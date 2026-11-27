#![cfg(not(target_arch = "wasm32"))]

use super::*;
use egui::{Pos2, Rect, Shape, Vec2, vec2};

#[derive(Clone, Copy)]
enum Presentation {
    Palette,
    Direct,
    Rows,
}

#[derive(Clone, Debug)]
struct Geometry {
    available: Vec2,
    label: Rect,
    inner: Rect,
    content: Vec2,
    handles: [bool; 2],
    id: Id,
    range: Option<std::ops::Range<usize>>,
}

struct Fixture {
    ctx: Context,
    time: f64,
    axes: [bool; 2],
    hidden: bool,
    presentation: Presentation,
    floating: bool,
}

impl Fixture {
    fn new(theme: ThemeKind, axes: [bool; 2], hidden: bool, presentation: Presentation) -> Self {
        let ctx = Context::default();
        apply(&ctx, theme, &StudioLightColors::default());
        Self { ctx, time: 0.0, axes, hidden, presentation, floating: false }
    }

    fn frame(&mut self, overflow: bool) -> Geometry {
        self.time += 0.25;
        let mut measured = None;
        let axes = self.axes;
        let hidden = self.hidden;
        let presentation = self.presentation;
        let floating = self.floating;
        let mut output = self.ctx.run_ui(
            egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 320.0))), time: Some(self.time), ..Default::default() },
            |ui| {
                ui.spacing_mut().scroll.floating = floating;
                let area = egui::ScrollArea::new(axes)
                    .id_salt("stable-scrollbar-gutter-v78")
                    .max_width(240.0)
                    .max_height(160.0)
                    .auto_shrink([false, false])
                    .animated(false)
                    .scroll_bar_visibility(if hidden {
                        egui::scroll_area::ScrollBarVisibility::AlwaysHidden
                    } else {
                        egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded
                    });
                let body = |body: &mut Ui| {
                    let available = body.available_size();
                    let label =
                        body.add(egui::Label::new("A multiline settings explanation stays on the same lines when a longer list appears below it.").wrap()).rect;
                    body.allocate_space(vec2(if overflow && axes[0] { 600.0 } else { 80.0 }, if overflow && axes[1] { 700.0 } else { 8.0 }));
                    (available, label, None)
                };
                let out = match presentation {
                    Presentation::Palette => area.show_palette(ui, body),
                    Presentation::Direct => area.show(ui, body),
                    Presentation::Rows => area.show_palette_rows(ui, 20.0, if overflow { 100 } else { 2 }, |body, rows| {
                        let available = body.available_size();
                        for row in rows.clone() {
                            body.label(format!("Row {row}"));
                        }
                        (available, Rect::NOTHING, Some(rows))
                    }),
                };
                measured = Some(Geometry {
                    available: out.inner.0,
                    label: out.inner.1,
                    inner: out.inner_rect,
                    content: out.content_size,
                    handles: [false; 2],
                    id: out.id,
                    range: out.inner.2,
                });
            },
        );
        output.textures_delta.clear();
        let mut result = measured.unwrap();
        let foreground = self.ctx.global_style().visuals.widgets.inactive.fg_stroke.color;
        let scroll = self.ctx.global_style().spacing.scroll;
        fn scan(shape: &Shape, inner: Rect, foreground: Color32, scroll: egui::style::ScrollStyle, handles: &mut [bool; 2]) {
            match shape {
                Shape::Rect(rect) if rect.fill == foreground => {
                    // A large virtual list legitimately reaches the theme's 12pt
                    // minimum thumb length. Identify its actual gutter geometry
                    // and painted color instead of assuming a thumb exceeds 16pt.
                    let min_length = scroll.handle_min_length - 1.0;
                    let max_cross = scroll.bar_width + 1.0;
                    if rect.rect.top() >= inner.bottom() - 1.0
                        && rect.rect.bottom() <= inner.bottom() + scroll.allocated_width() + 1.0
                        && rect.rect.height() > 0.0
                        && rect.rect.height() <= max_cross
                        && rect.rect.width() >= min_length
                    {
                        handles[0] = true;
                    }
                    if rect.rect.left() >= inner.right() - 1.0
                        && rect.rect.right() <= inner.right() + scroll.allocated_width() + 1.0
                        && rect.rect.width() > 0.0
                        && rect.rect.width() <= max_cross
                        && rect.rect.height() >= min_length
                    {
                        handles[1] = true;
                    }
                }
                Shape::Vec(children) => {
                    for child in children {
                        scan(child, inner, foreground, scroll, handles);
                    }
                }
                _ => {}
            }
        }
        for shape in &output.shapes {
            scan(&shape.shape, result.inner, foreground, scroll, &mut result.handles);
        }
        result
    }

    fn settled(&mut self, overflow: bool) -> Geometry {
        for _ in 0..4 {
            self.frame(overflow);
        }
        self.frame(overflow)
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 0.5
}

#[test]
fn fitting_overflow_fitting_keeps_content_width_and_multiline_wrapping_in_all_themes_v78() {
    for theme in ThemeKind::ALL {
        for presentation in [Presentation::Palette, Presentation::Direct] {
            let mut fixture = Fixture::new(theme, [false, true], false, presentation);
            let fitting = fixture.settled(false);
            assert!(!fitting.handles[1], "{theme:?}: fitting content must not paint a thumb");
            assert!(fitting.available.x < 240.0, "the hidden vertical bar already has a gutter");
            assert!(close(fitting.available.y, 160.0), "a disabled horizontal axis reserves no bottom gutter");
            for overflowing in [true, false] {
                // Include the first transition frame, not only a settled animation.
                for _ in 0..5 {
                    let next = fixture.frame(overflowing);
                    assert!(close(next.available.x, fitting.available.x), "{theme:?}: content width changed: {fitting:?} -> {next:?}");
                    assert_eq!(next.label.size(), fitting.label.size(), "multiline text must not reflow");
                    assert_eq!(next.id, fitting.id);
                }
                let next = fixture.frame(overflowing);
                assert_eq!(next.handles[1], overflowing, "{theme:?}: bar visibility follows overflow, not reservation");
            }
        }
    }
}

#[test]
fn gutter_reservation_respects_axes_and_explicit_hidden_bars_v78() {
    for theme in ThemeKind::ALL {
        for axes in [[true, false], [false, true], [true, true], [false, false]] {
            for hidden in [false, true] {
                let mut fixture = Fixture::new(theme, axes, hidden, Presentation::Palette);
                let fit = fixture.settled(false);
                let width_reserved = 240.0 - fit.available.x;
                let height_reserved = 160.0 - fit.available.y;
                assert_eq!(width_reserved > 0.5, axes[1] && !hidden, "{theme:?}/{axes:?}: vertical gutter");
                assert_eq!(height_reserved > 0.5, axes[0] && !hidden, "{theme:?}/{axes:?}: horizontal gutter");
                if hidden {
                    let over = fixture.settled(true);
                    assert_eq!(over.handles, [false, false]);
                } else if axes[0] || axes[1] {
                    let over = fixture.settled(true);
                    assert_eq!(over.available, fit.available, "reservation is independent of overflow");
                    assert_eq!(over.handles, axes);
                }
            }
        }
    }
}

#[test]
fn virtual_rows_keep_the_same_viewport_and_ids_when_scrollbar_is_needed_v78() {
    for theme in ThemeKind::ALL {
        let mut fixture = Fixture::new(theme, [false, true], false, Presentation::Rows);
        let fitting = fixture.settled(false);
        let overflow = fixture.settled(true);
        let fitting_again = fixture.settled(false);
        assert_eq!(fitting.inner, overflow.inner);
        assert_eq!(fitting.inner, fitting_again.inner);
        assert_eq!(fitting.available.x, overflow.available.x);
        assert_eq!(fitting.id, overflow.id);
        assert_eq!(fitting.range, Some(0..2));
        assert!(overflow.range.as_ref().is_some_and(|range| range.start == 0 && range.end < 100));
        assert!(overflow.content.y > overflow.inner.height());
        assert!(
            !fitting.handles[1] && overflow.handles[1] && !fitting_again.handles[1],
            "{theme:?}: actual painted thumb must follow overflow; fitting={fitting:?}, overflow={overflow:?}, fitting_again={fitting_again:?}"
        );
    }
}

#[test]
fn menus_keep_reserved_gutter_and_stable_submenu_adjacency_v78() {
    for theme in ThemeKind::ALL {
        let mut fixture = Fixture::new(theme, [false, true], false, Presentation::Palette);
        let fitting = fixture.settled(false);
        let reserved = fixture.ctx.global_style().spacing.scroll.allocated_width();
        assert!(close(fitting.available.x, 240.0 - reserved));
        for overflow in [true, false] {
            let next = fixture.settled(overflow);
            assert_eq!(next.available.x, fitting.available.x);
            assert_eq!(next.label.size(), fitting.label.size());
        }
    }
}

#[test]
fn collapsed_scroll_regions_keep_finite_nonnegative_geometry_and_clips_v78() {
    for theme in ThemeKind::ALL {
        for axes in [[false, true], [true, false], [true, true]] {
            for size in [Vec2::ZERO, vec2(0.0, 4.0), vec2(4.0, 0.0), vec2(4.0, 4.0)] {
                let ctx = Context::default();
                apply(&ctx, theme, &StudioLightColors::default());
                let bounds = Rect::from_min_size(egui::pos2(20.0, 20.0), size);
                let mut metrics = None;
                let mut output =
                    ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0))), ..Default::default() }, |ui| {
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bounds));
                        child.set_clip_rect(bounds);
                        let out = egui::ScrollArea::new(axes)
                            .id_salt("collapsed-scrollbar-gutter-v78")
                            .max_width(size.x)
                            .max_height(size.y)
                            .auto_shrink([false, false])
                            .show_palette(&mut child, |body| {
                                let initial = body.max_rect();
                                let clip = body.clip_rect();
                                let (_, allocated) = body.allocate_space(vec2(16.0, 16.0));
                                body.painter().rect_filled(allocated, 0.0, Color32::MAGENTA);
                                (initial, clip, allocated)
                            });
                        metrics = Some((out.inner_rect, out.inner));
                    });
                output.textures_delta.clear();
                let (inner, (initial, clip, allocated)) = metrics.unwrap();
                for rect in [inner, initial, clip, allocated] {
                    assert!(rect.min.is_finite() && rect.max.is_finite(), "{theme:?}/{axes:?}/{size:?}: {rect:?}");
                    assert!(rect.width() >= 0.0 && rect.height() >= 0.0, "{theme:?}/{axes:?}/{size:?}: {rect:?}");
                }
                for (axis, enabled) in axes.into_iter().enumerate() {
                    if enabled {
                        assert!(inner.size()[axis] >= 64.0, "the existing scroll-axis minimum remains intact");
                    }
                }
                assert!(clip.min.x >= bounds.min.x && clip.min.y >= bounds.min.y);
                assert!(clip.max.x <= bounds.max.x && clip.max.y <= bounds.max.y);
                assert!(
                    output.shapes.iter().any(|shape| { matches!(&shape.shape, Shape::Rect(rect) if rect.fill == Color32::MAGENTA) && shape.clip_rect == clip }),
                    "actual content painting uses the bounded nonnegative clip"
                );
            }
        }
    }
}
