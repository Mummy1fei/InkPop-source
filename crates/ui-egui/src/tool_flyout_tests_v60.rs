use super::*;
use std::{cell::RefCell, rc::Rc};

fn row_and_bar_rects(ctx: &egui::Context, viewport: egui::ViewportId) -> (Vec<Rect>, Vec<Rect>) {
    ctx.viewport_for(viewport, |v| {
        let widgets = || v.prev_pass.widgets.layers().flat_map(|(_, widgets)| widgets.iter());
        let rows = widgets().filter(|w| w.rect.height() == 26.0 && w.rect.width() >= 180.0 && w.sense.senses_click()).map(|w| w.rect).collect();
        let bars = widgets().filter(|w| w.rect.width() < 20.0 && w.rect.height() > 26.0 && w.sense.senses_drag()).map(|w| w.rect).collect();
        (rows, bars)
    })
}

#[test]
fn native_tool_groups_fit_every_row_without_a_scrollbar_v60() {
    // A real nested child pass at the requested HWND size. Previous tests used
    // an oversized fixed 300x160 client, which hid the three-row clipping bug.
    std::thread::Builder::new()
        .name("v60-flyout-fit".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            for theme in crate::theme::ThemeKind::ALL {
                let ctx = egui::Context::default();
                PhotocraftApp::setup_context(&ctx, theme);
                ctx.set_embed_viewports(false);
                let measured = Rc::new(RefCell::new((Rect::NOTHING, Vec::new(), Vec::new())));
                let renderer_measured = measured.clone();
                let client = Rc::new(RefCell::new(Vec2::ZERO));
                let renderer_client = client.clone();
                egui::Context::set_immediate_viewport_renderer(move |ctx, mut viewport| {
                    if let Some(size) = viewport.builder.inner_size {
                        *renderer_client.borrow_mut() = size;
                    }
                    let client = Rect::from_min_size(egui::Pos2::ZERO, *renderer_client.borrow());
                    let mut input = egui::RawInput { viewport_id: viewport.ids.this, screen_rect: Some(client), ..Default::default() };
                    let info = input.viewports.entry(viewport.ids.this).or_default();
                    info.parent = Some(viewport.ids.parent);
                    info.inner_rect = Some(client);
                    let mut output = ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui));
                    let (rows, bars) = row_and_bar_rects(ctx, viewport.ids.this);
                    *renderer_measured.borrow_mut() = (client, rows, bars);
                    output.textures_delta.clear();
                });
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                for slot in TOOL_SECTIONS.iter().flat_map(|section| section.iter()).filter(|slot| slot.len() > 1) {
                    // Include groups placed at the bottom: popup placement must move
                    // the entire native host upward, not shorten the last row.
                    crate::native_popup_ui::open(&ctx, "tool-flyout", pos2(45.0, 1780.0), app.frame);
                    for _ in 0..3 {
                        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 1800.0))), ..Default::default() };
                        let mut output = ctx.run_ui(input, |ui| {
                            let t = Tokens::get(ui.ctx());
                            let size = tool_flyout_size(ui.style(), tool_flyout_width(ui, slot, &t), slot.len());
                            crate::native_popup_ui::show(&mut app, ui.ctx(), "tool-flyout", "Tools", size, |app, ui| {
                                tool_flyout_contents(app, ui, slot, slot[0], &t)
                            });
                        });
                        output.textures_delta.clear();
                        app.frame += 1;
                    }
                    let (client, rows, bars) = measured.borrow().clone();
                    assert_eq!(rows.len(), slot.len(), "{theme:?}: every tool has a row");
                    assert!(rows.iter().all(|r| client.contains_rect(*r)), "{theme:?} {slot:?}: clipped rows {rows:?} in {client:?}");
                    assert!(bars.is_empty(), "{theme:?} {slot:?}: unnecessary scrollbar {bars:?}");
                    crate::native_popup_ui::close(&ctx);
                }
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

// build_ui_state adds its own 8pt outer margin. Use the margin-free eframe
// entry point, matching the native callback rather than compensating its size.
struct FlyoutFixture {
    app: PhotocraftApp,
    theme: crate::theme::ThemeKind,
    short: bool,
    verify: bool,
    desired_size: Vec2,
}

impl eframe::App for FlyoutFixture {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
            return;
        }
        let t = Tokens::get(ui.ctx());
        let slot = [Tool::Eraser, Tool::BackgroundEraser, Tool::MagicEraser];
        // Compute from the actual rendered viewport style, after setup has run.
        self.desired_size = tool_flyout_size(ui.style(), 245.0, slot.len());
        egui::CentralPanel::default().frame(crate::native_popup_ui::menu_frame(ui.style())).show(ui, |ui| {
            let scroll = egui::ScrollArea::vertical()
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let selected = self.app.ui.tool;
                    tool_flyout_contents(&mut self.app, ui, &slot, selected, &t)
                });
            if self.verify {
                if self.short {
                    assert!(scroll.content_size.y > scroll.inner_rect.height());
                } else {
                    assert!(
                        scroll.content_size.y <= scroll.inner_rect.height() + 0.1,
                        "{:?}: content {:?}, client {:?}",
                        self.theme,
                        scroll.content_size,
                        scroll.inner_rect
                    );
                }
            }
        });
    }
}

#[test]
#[ignore = "writes v60 five-theme exact-client tool flyout GPU artifacts"]
fn tool_flyout_exact_client_gpu_visual_v60() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v60/flyout-gpu");
    assert!(!out.exists(), "preserve previous evidence");
    std::fs::create_dir_all(&out).unwrap();
    for theme in crate::theme::ThemeKind::ALL {
        for short in [false, true] {
            let mut h = egui_kittest::Harness::builder().with_size(vec2(350.0, 200.0)).wgpu().build_eframe(move |cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, theme);
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                app.ui.tool = Tool::Eraser;
                FlyoutFixture { app, theme, short, verify: false, desired_size: Vec2::ZERO }
            });
            h.run_steps(4);
            let size = h.state().desired_size;
            assert!(size.y > 78.0);
            // Native popup placement has an 80pt minimum. A smaller synthetic
            // client would violate ScrollArea's own 64pt inner minimum after
            // subtracting the frame, making the last row unreachable by design.
            h.set_size(if short { vec2(size.x, 80.0) } else { size });
            h.state_mut().verify = true;
            h.run_steps(20);
            let (rows, bars) = row_and_bar_rects(&h.ctx, egui::ViewportId::ROOT);
            assert_eq!(rows.len(), 3);
            if short {
                assert!(!bars.is_empty(), "genuinely constrained menu retains scrolling");
            } else {
                assert!(bars.is_empty(), "all three tools fit without a scrollbar");
            }
            h.render().unwrap().save(out.join(format!("{}-{}.png", theme.id(), if short { "constrained" } else { "three-tools" }))).unwrap();
            if short {
                // The last item must remain reachable when the screen really is
                // too short. Exercise wheel input rather than hiding its scrollbar.
                h.event(egui::Event::PointerMoved(pos2(100.0, 32.0)));
                h.event(egui::Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, -160.0),
                    modifiers: egui::Modifiers::NONE,
                });
                let mut previous = None;
                let mut stable = 0;
                for _ in 0..120 {
                    h.step();
                    let (mut rows, _) = row_and_bar_rects(&h.ctx, egui::ViewportId::ROOT);
                    rows.sort_by(|a, b| a.top().total_cmp(&b.top()));
                    let last = *rows.last().unwrap();
                    if previous == Some(last) && h.ctx.content_rect().contains_rect(last) {
                        stable += 1;
                    } else {
                        stable = 0;
                    }
                    previous = Some(last);
                    if stable >= 3 {
                        break;
                    }
                }
                h.render().unwrap().save(out.join(format!("{}-constrained-scrolled.png", theme.id()))).unwrap();
                assert!(
                    stable >= 3,
                    "{theme:?}, short={short}: scrolling did not converge to a fully visible third row; last={previous:?}, client={:?}",
                    h.ctx.content_rect()
                );
            }
            let (mut rows, _) = row_and_bar_rects(&h.ctx, egui::ViewportId::ROOT);
            rows.sort_by(|a, b| a.top().total_cmp(&b.top()));
            let last = *rows.last().unwrap();
            assert!(
                h.ctx.content_rect().contains_rect(last),
                "{theme:?}, short={short}: third row {last:?} must be fully inside client {:?} before clicking",
                h.ctx.content_rect()
            );
            let at = last.center();
            h.event(egui::Event::PointerMoved(at));
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
            h.run_steps(2);
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
            h.run_steps(2);
            assert_eq!(h.state().app.ui.tool, Tool::MagicEraser, "{:?}: third tool is selectable", theme);
        }
    }
}
