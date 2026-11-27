//! Production toolbar images across a held flyout. Native texture lifetime is
//! covered separately by the nested-viewport unit regression.
use egui::{Color32, Event, Modifiers, PointerButton, Rect, Vec2, vec2};
use egui_kittest::Harness;
use photocraft_ui_egui::{
    PhotocraftApp, Services, Tool, panels,
    theme::{ThemeKind, Tokens},
};

#[test]
#[ignore = "writes v59 Classic scrollbar idle/hover/drag GPU artifacts"]
fn classic_preset_scrollbar_gpu_visual_v59() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v59/classic-scrollbar-offscreen");
    assert!(!out.exists(), "preserve prior evidence");
    std::fs::create_dir(&out).unwrap();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    for i in 0..60 {
        app.run("tool.presets.new", serde_json::json!({"name":format!("Texture brush {i:02}"),"tool":"brush"})).unwrap();
    }
    let mut h = Harness::builder().with_size(vec2(340.0, 640.0)).wgpu().build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Tokens::get(ui.ctx()).card).inner_margin(8.0)).show(ui, |ui| {
                    photocraft_ui_egui::preset_panels::tool_presets_panel(app, ui);
                });
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, ThemeKind::Classic);
    h.run_steps(24);
    let bar = h
        .ctx
        .viewport(|v| {
            v.prev_pass
                .widgets
                .layers()
                .flat_map(|(_, widgets)| widgets.iter())
                .filter(|w| w.sense.senses_drag() && w.rect.width() < 20.0 && w.rect.height() > 100.0)
                .max_by(|a, b| a.rect.height().total_cmp(&b.rect.height()))
                .map(|w| w.rect)
        })
        .unwrap();
    h.render().unwrap().save(out.join("classic-idle.png")).unwrap();
    let at = egui::pos2(bar.center().x, bar.top() + 12.0);
    h.event(Event::PointerMoved(at));
    h.run_steps(3);
    h.render().unwrap().save(out.join("classic-hover.png")).unwrap();
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(2);
    h.event(Event::PointerMoved(egui::pos2(at.x, bar.center().y)));
    h.run_steps(3);
    h.render().unwrap().save(out.join("classic-drag.png")).unwrap();
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    let visuals = h.ctx.global_style().visuals.clone();
    assert_ne!(visuals.extreme_bg_color, Color32::WHITE);
    assert_ne!(visuals.widgets.inactive.fg_stroke.color, Color32::BLACK);
    assert_eq!(visuals.override_text_color, Some(Color32::BLACK));
}

#[test]
#[ignore = "writes v59 five-theme wgpu toolbar visual artifacts"]
fn held_tool_flyout_gpu_visual_v59() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v59/tool-chrome-offscreen");
    assert!(!out.exists(), "preserve prior evidence");
    std::fs::create_dir(&out).unwrap();
    for theme in ThemeKind::ALL {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
        let mut h = Harness::builder().with_size(vec2(900.0, 1400.0)).with_step_dt(1.0 / 60.0).wgpu().build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    panels::options_bar(app, ui);
                    panels::toolbar(app, ui);
                    egui::CentralPanel::default().show(ui, |ui| {
                        ui.colored_label(Color32::GRAY, "PhotoCraft tool flyout review");
                    });
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, theme);
        h.state_mut().ui.tool = Tool::Brush;
        h.run_steps(24);
        let side = if Tokens::get(&h.ctx).pro { 30.0 } else { 36.0 };
        let mut slots: Vec<Rect> = h.ctx.viewport(|v| {
            v.prev_pass
                .widgets
                .layers()
                .flat_map(|(_, widgets)| widgets.iter())
                .filter(|widget| widget.rect.size() == Vec2::splat(side) && widget.rect.left() < 100.0 && widget.sense.senses_click())
                .map(|widget| widget.rect)
                .collect()
        });
        slots.sort_by(|a, b| a.top().total_cmp(&b.top()));
        assert!(slots.len() >= 20);
        let before = h.render().unwrap();
        before.save(out.join(format!("{}-before.png", theme.id()))).unwrap();
        let at = slots[7].center(); // Brush/Pencil/Mixer group, after Healing.
        h.event(Event::PointerMoved(at));
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(24);
        for frame in 0..3 {
            h.step();
            let held = h.render().unwrap();
            held.save(out.join(format!("{}-held-{frame}.png", theme.id()))).unwrap();
            for (index, rect) in slots.iter().take(20).enumerate() {
                if index == 7 {
                    continue;
                }
                let center = rect.center();
                for dy in -6..6 {
                    for dx in -6..6 {
                        let x = (center.x.round() as i32 + dx) as u32;
                        let y = (center.y.round() as i32 + dy) as u32;
                        assert_eq!(held.get_pixel(x, y), before.get_pixel(x, y), "{theme:?} frame {frame} slot {index}: idle icon pixels changed");
                    }
                }
            }
        }
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(3);
        assert_eq!(h.state().ui.tool, Tool::Brush);
    }
}
