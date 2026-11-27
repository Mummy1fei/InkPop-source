use super::*;
use crate::theme::{ThemeKind, Tokens};
use egui_kittest::{Harness, kittest::Queryable};
use std::sync::{Arc, Mutex};

type Opened = Arc<Mutex<Vec<String>>>;

struct Fixture {
    app: PhotocraftApp,
    home: bool,
    language: crate::i18n::Lang,
    opened: Opened,
}

impl eframe::App for Fixture {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let _language = crate::i18n::language_scope(self.language);
        self.app.frame += 1;
        if self.home {
            ui.painter().rect_filled(ui.available_rect_before_wrap(), 0.0, Tokens::get(ui.ctx()).canvas);
            crate::canvas::start_screen(&mut self.app, ui);
            return;
        }
        let Some(dialog) = self.app.ui.dialogs.last().cloned() else { return };
        let mut fields = dialog.fields.clone();
        let mut footer = egui::Panel::bottom("contacts-v72-footer")
            .exact_size(super::super::footer_height(&dialog, ui.available_width()))
            .frame(egui::Frame::central_panel(ui.style()).inner_margin(16.0))
            .show(ui, |ui| ui.new_child(egui::UiBuilder::new().max_rect(ui.max_rect())))
            .inner;
        egui::CentralPanel::default().frame(egui::Frame::central_panel(ui.style()).inner_margin(16.0)).show(ui, |ui| {
            form_body_scroll(ui, &mut self.app, &dialog, &mut fields);
        });
        let (outcome, apply) = super::super::buttons(&mut self.app, &mut footer, &dialog, &fields);
        super::super::finish(&mut self.app, ui.ctx(), &dialog, fields, outcome, apply);
    }
}

fn harness(theme: ThemeKind, size: egui::Vec2, tab: &str, language: &str, gpu: bool, home: bool) -> Harness<'static, Fixture> {
    let tab = tab.to_string();
    let language = crate::i18n::Lang::from_code(language).unwrap();
    let builder = Harness::builder().with_size(size).with_max_steps(64);
    let builder = if gpu { builder.wgpu_setup(crate::gpu_canvas::wgpu_setup()) } else { builder };
    builder.build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        let opened: Opened = Default::default();
        let recorded = opened.clone();
        let mut app = PhotocraftApp::new(
            photocraft_engine::Session::new(),
            crate::Services {
                open_url: Some(Box::new(move |url| {
                    recorded.lock().unwrap().push(url.to_string());
                    Ok(())
                })),
                ..Default::default()
            },
        );
        if !home {
            app.ui.open_dialog(DialogKind::About, serde_json::Map::from_iter([("tab".into(), Value::String(tab))]));
        }
        Fixture { app, home, language, opened }
    })
}

#[test]
fn about_fixed_footer_keeps_contacts_visible_in_both_tabs_and_narrow_windows_v72() {
    for theme in ThemeKind::ALL {
        for width in [360.0, 732.0] {
            for language in ["en", "zh-hans"] {
                for tab in ["about", "credits"] {
                    let mut h = harness(theme, vec2(width, 560.0), tab, language, false, false);
                    h.run_steps(8);
                    let ok = h.get_by_label(if language == "en" { "OK" } else { "确定" }).rect();
                    assert!((ok.right() - (width - 16.0)).abs() < 1.0, "OK must remain aligned to the right edge: {ok:?}");
                    let labels = if language == "en" {
                        ["Xiaohongshu", "Weibo", crate::welcome_links::EMAIL]
                    } else {
                        ["小红书", "微博", crate::welcome_links::EMAIL]
                    };
                    for label in labels {
                        let rect = h.get_by_label(label).rect();
                        assert!(rect.left() >= 16.0 && rect.right() <= width - 15.0 && rect.bottom() < 560.0, "{theme:?}/{width}/{tab}/{label}: {rect:?}");
                        assert!(!rect.intersects(ok), "contacts and OK must not overlap");
                        assert!(rect.top() > 460.0, "contacts belong to the fixed footer, not the scrolling body");
                    }
                    assert!(h.state().opened.lock().unwrap().is_empty());
                }
            }
        }
    }
}

#[test]
fn about_footer_links_use_mock_service_and_ok_still_closes_the_dialog_v72() {
    let mut h = harness(ThemeKind::StudioLight, vec2(732.0, 560.0), "about", "en", false, false);
    h.run_steps(6);
    for (label, url) in [
        ("Xiaohongshu", crate::welcome_links::XIAOHONGSHU),
        ("Weibo", crate::welcome_links::WEIBO),
        (crate::welcome_links::EMAIL, crate::welcome_links::EMAIL_URL),
    ] {
        h.get_by_label(label).click();
        h.run_steps(3);
        assert_eq!(h.state().opened.lock().unwrap().last().map(String::as_str), Some(url));
        assert_eq!(h.state().app.ui.dialogs.len(), 1);
    }
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(h.state().app.ui.dialogs.is_empty());
    let identifier = h.state_mut().app.ui.open_dialog(DialogKind::About, serde_json::Map::from_iter([("systemInfo".into(), Value::Bool(true))]));
    let system = h.state_mut().app.ui.dialog_mut(identifier).unwrap().clone();
    assert!(!super::super::has_contacts(&system));
    assert_eq!(super::super::footer_height(&system, 360.0), 64.0);
}

#[test]
fn home_screen_has_copyright_without_contact_links_in_all_themes_v72() {
    for theme in ThemeKind::ALL {
        let mut h = harness(theme, vec2(1440.0, 900.0), "about", "en", false, true);
        h.run_steps(4);
        h.get_by_label(crate::welcome_links::COPYRIGHT);
        for label in ["Xiaohongshu", "Weibo", crate::welcome_links::EMAIL] {
            assert!(h.query_by_label(label).is_none());
        }
    }
}

#[test]
#[ignore = "writes five-theme home/About and narrow contact footer screenshots for root review"]
fn contacts_gpu_visual_v72() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v72/contacts-gpu");
    assert!(!out.exists(), "preserve previous visual evidence");
    std::fs::create_dir_all(&out).unwrap();
    for theme in ThemeKind::ALL {
        for home in [false, true] {
            let size = if home { vec2(1440.0, 900.0) } else { vec2(732.0, 560.0) };
            let mut h = harness(theme, size, "about", "zh-hans", true, home);
            h.run_steps(8);
            h.render().unwrap().save(out.join(format!("{}-{}.png", theme.id(), if home { "home" } else { "about" }))).unwrap();
        }
    }
    for (name, size, tab, language) in [
        ("credits", vec2(732.0, 560.0), "credits", "zh-hans"),
        ("narrow-en", vec2(360.0, 440.0), "about", "en"),
        ("narrow-zh", vec2(360.0, 440.0), "about", "zh-hans"),
    ] {
        let mut h = harness(ThemeKind::StudioLight, size, tab, language, true, false);
        h.run_steps(8);
        h.render().unwrap().save(out.join(format!("{name}.png"))).unwrap();
    }
}
