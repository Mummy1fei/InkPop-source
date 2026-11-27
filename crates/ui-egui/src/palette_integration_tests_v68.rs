//! Production shell surface/pasteboard integration for Studio Light palettes.
use crate::{
    PhotocraftApp,
    theme::{ThemeKind, Tokens},
};
use egui::{Color32, Shape, vec2};
use egui_kittest::Harness;
use photocraft_engine::prefs::palette::{ColorSlot, PaletteRgb, StudioLightColors};
use serde_json::json;

fn colors() -> StudioLightColors {
    let mut config = StudioLightColors::default();
    for (slot, rgb) in [
        (ColorSlot::TitleBar, [223, 239, 233]),
        (ColorSlot::MenuBar, [240, 231, 209]),
        (ColorSlot::Toolbar, [208, 231, 224]),
        (ColorSlot::OptionsBar, [229, 241, 236]),
        (ColorSlot::StatusBar, [197, 220, 213]),
        (ColorSlot::Card, [246, 251, 248]),
        (ColorSlot::PanelTitle, [216, 235, 226]),
        (ColorSlot::FloatingTitle, [199, 225, 212]),
        (ColorSlot::Canvas, [204, 219, 211]),
        (ColorSlot::CanvasDot, [177, 195, 185]),
        (ColorSlot::MenuBackground, [248, 250, 238]),
        (ColorSlot::MenuHover, [208, 232, 219]),
        (ColorSlot::MenuSelection, [188, 220, 204]),
        (ColorSlot::ProgressTrack, [214, 228, 221]),
        (ColorSlot::ProgressFill, [43, 128, 100]),
        (ColorSlot::SliderFill, [65, 142, 118]),
        (ColorSlot::ScrollbarTrack, [224, 236, 229]),
        (ColorSlot::ScrollbarHandle, [102, 136, 120]),
    ] {
        config.colors.insert(slot, PaletteRgb::new(rgb));
    }
    config
}
fn shell(theme: ThemeKind, custom: bool) -> Harness<'static, PhotocraftApp> {
    Harness::builder().with_size(vec2(1440.0,1000.0)).with_max_steps(64).build_eframe(move|cc|{
        PhotocraftApp::setup_context(&cc.egui_ctx,theme);
        let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());
        app.run("prefs.set",json!({"values":{"interface.theme":theme.id(),"interface.language":"zh-hans","interface.studioLightColors":if custom {colors()} else {StudioLightColors::default()}}})).unwrap();
        app.run("file.new",json!({"width":640,"height":400,"background":"#ffffff"})).unwrap();
        app
    })
}
fn fills(shape: &Shape, want: Color32) -> usize {
    match shape {
        Shape::Rect(r) if r.fill == want => 1,
        Shape::Vec(shapes) => shapes.iter().map(|s| fills(s, want)).sum(),
        _ => 0,
    }
}
#[test]
fn production_shell_paints_independent_module_surfaces_v68() {
    let mut h = shell(ThemeKind::StudioLight, true);
    h.run_steps(8);
    for slot in
        [ColorSlot::TitleBar, ColorSlot::MenuBar, ColorSlot::Toolbar, ColorSlot::OptionsBar, ColorSlot::StatusBar, ColorSlot::Card, ColorSlot::PanelTitle]
    {
        let want = crate::theme::palette::resolved_color(ThemeKind::StudioLight, &colors(), slot);
        let count: usize = h.output().shapes.iter().map(|s| fills(&s.shape, want)).sum();
        assert!(count > 0, "{slot:?} must paint a real shell surface, not merely a stored preference");
    }
}
#[test]
fn palette_canvas_color_wins_only_in_studio_light_and_reset_restores_existing_preference_v68() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("prefs.set", json!({"values":{"interface.canvasColor":"black","interface.studioLightColors":colors()}})).unwrap();
    app.ui.theme = ThemeKind::StudioLight;
    assert_eq!(crate::prefs_ui::pasteboard_color(&app), Some(Color32::from_rgb(204, 219, 211)));
    for kind in ThemeKind::ALL.into_iter().filter(|k| *k != ThemeKind::StudioLight) {
        app.ui.theme = kind;
        assert_eq!(crate::prefs_ui::pasteboard_color(&app), Some(Color32::BLACK));
    }
    app.ui.theme = ThemeKind::StudioLight;
    app.run("prefs.reset", json!({"path":"interface.studioLightColors.colors"})).unwrap();
    assert_eq!(crate::prefs_ui::pasteboard_color(&app), Some(Color32::BLACK));
}
#[test]
fn production_progress_bar_uses_independent_track_and_fill_v68() {
    let cfg = colors();
    let mut h = Harness::new_ui_state(
        move |ui, _: &mut ()| {
            crate::theme::palette::apply(ui.ctx(), ThemeKind::StudioLight, &cfg);
            let rect = egui::Rect::from_min_size(egui::pos2(20.0, 30.0), vec2(200.0, 8.0));
            crate::jobs_ui::bar(ui, rect, Some(0.5), &Tokens::get(ui.ctx()));
        },
        (),
    );
    h.run_steps(3);
    for want in [Color32::from_rgb(214, 228, 221), Color32::from_rgb(43, 128, 100)] {
        assert!(h.output().shapes.iter().any(|s| fills(&s.shape, want) > 0));
    }
}
#[test]
fn production_other_themes_ignore_saved_studio_light_surface_colors_v68() {
    for kind in ThemeKind::ALL.into_iter().filter(|k| *k != ThemeKind::StudioLight) {
        let mut h = shell(kind, true);
        h.run_steps(8);
        assert_eq!(Tokens::get(&h.ctx), Tokens::for_kind(kind));
        assert_eq!(crate::theme::palette::override_color(&h.ctx, ColorSlot::Toolbar), None);
        let count: usize = h.output().shapes.iter().map(|s| fills(&s.shape, Color32::from_rgb(208, 231, 224))).sum();
        assert_eq!(count, 0, "custom toolbar leaked to {kind:?}");
    }
}
#[test]
#[ignore = "writes production shell five-theme and customized Studio Light GPU views; root must inspect"]
fn palette_shell_gpu_visual_v68() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v68/shell-gpu");
    assert!(!out.exists());
    std::fs::create_dir_all(&out).unwrap();
    for kind in ThemeKind::ALL {
        let cfg = colors();
        let mut h = Harness::builder().with_size(vec2(1440.0, 1000.0)).with_max_steps(64).wgpu().build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, kind);
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.run("prefs.set", json!({"values":{"interface.theme":kind.id(),"interface.language":"zh-hans","interface.studioLightColors":cfg}})).unwrap();
            app.run("file.new", json!({"width":640,"height":400,"background":"#ffffff"})).unwrap();
            app
        });
        h.run_steps(8);
        h.render().unwrap().save(out.join(format!("{}-saved-palette.png", kind.id()))).unwrap();
        if kind == ThemeKind::StudioLight {
            h.state_mut().run("prefs.reset", json!({"path":"interface.studioLightColors.colors"})).unwrap();
            h.run_steps(4);
            h.render().unwrap().save(out.join("studioLight-restored.png")).unwrap();
        }
    }
}
