use super::*;
use egui_kittest::{Harness, kittest::Queryable};

struct Fixture {
    geometry: (egui::Rect, f32, f32),
}
impl Default for Fixture {
    fn default() -> Self {
        Self { geometry: (egui::Rect::NOTHING, 0.0, 0.0) }
    }
}

impl eframe::App for Fixture {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| self.geometry = content(ui).0);
    }
}

struct AboutFixture(crate::PhotocraftApp);
impl eframe::App for AboutFixture {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        crate::dialogs::show(&mut self.0, ui.ctx());
    }
}

fn about_harness(theme: crate::theme::ThemeKind, services: crate::Services) -> Harness<'static, AboutFixture> {
    Harness::builder().with_size(vec2(1100.0, 820.0)).with_max_steps(64).build_eframe(move |cc| {
        crate::PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.ui.open_dialog(crate::state::DialogKind::About, serde_json::Map::new());
        AboutFixture(app)
    })
}

fn harness(theme: crate::theme::ThemeKind) -> Harness<'static, Fixture> {
    let mut harness = Harness::builder()
        .with_size(vec2(740.0, 560.0))
        .with_max_steps(64)
        .build_ui_state(|ui, state: &mut Fixture| state.geometry = content(ui).0, Fixture::default());
    crate::PhotocraftApp::setup_context(&harness.ctx, theme);
    harness.run_steps(4);
    harness
}

#[test]
fn actual_components_preserve_license_boundaries_and_do_not_advertise_research_models() {
    let entries = GROUPS.iter().flat_map(|g| g.entries).collect::<Vec<_>>();
    for name in ["U²-Net / U2NETP", "SkySeg U2NETP", "MobileSAM / TinyViT", "Candle", "wgpu", "egui / eframe", "photocraft-cms"] {
        assert!(entries.iter().any(|entry| entry.name == name), "missing runtime component {name}");
    }
    assert!(!entries.iter().any(|entry| entry.name.contains("BiRefNet") || entry.name.contains("SAM 2")));
    assert!(entries.iter().filter(|entry| entry.license == OWN_IMPLEMENTATION).count() >= 6);
    assert!(entries.iter().all(|entry| !entry.author.is_empty() && entry.source.starts_with("https://")));
    assert!(ATTRIBUTION.contains("LICENSE-MobileSAM"));
}

#[test]
fn five_theme_index_is_bounded_scrollable_and_reaches_license_text() {
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = harness(theme);
        assert!(h.query_by_label("算法、模型与第三方组件").is_some());
        assert!(h.query_by_label("U²-Net / U2NETP").is_some());
        let (viewport, content_height, offset) = h.state().geometry;
        assert!(viewport.is_finite() && viewport.is_positive());
        assert!(viewport.width() <= 740.0 && viewport.height() <= 340.0);
        assert!(content_height > viewport.height() * 2.0 && offset == 0.0);
        h.get_by_label("第三方素材与来源索引（ATTRIBUTION.md）").scroll_to_me();
        h.run_steps(5);
        assert!(h.state().geometry.2 > 0.0, "{theme:?}: the source list must actually scroll");
        h.get_by_label("第三方素材与来源索引（ATTRIBUTION.md）").click();
        h.run_steps(4);
        assert!(h.query_by_label_contains("随源码编译的原文索引").is_some());
    }
}

#[test]
fn wide_index_shows_every_component_and_license_without_scrolling_v73() {
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = Harness::builder()
            .with_size(vec2(1040.0, 860.0))
            .with_max_steps(64)
            .build_ui_state(|ui, state: &mut Fixture| state.geometry = content(ui).0, Fixture::default());
        crate::PhotocraftApp::setup_context(&h.ctx, theme);
        h.run_steps(6);
        let (viewport, height, offset) = h.state().geometry;
        assert!(height <= viewport.height() + 1.0, "{theme:?}: full index should fit: {height} vs {viewport:?}");
        assert_eq!(offset, 0.0);
        for item in GROUPS.iter().flat_map(|group| group.entries) {
            let rect = h.get_by_label(item.name).rect();
            assert!(viewport.expand(1.0).contains_rect(rect), "{theme:?}: component must be fully visible: {} {rect:?}", item.name);
            h.get_by_label(&format!("来源：{}", item.name));
        }
        assert!(viewport.expand(1.0).contains_rect(h.get_by_label("第三方素材与来源索引（ATTRIBUTION.md）").rect()));
    }
}

#[test]
fn about_main_tab_keeps_identity_and_shows_new_index() {
    let mut h = about_harness(crate::theme::ThemeKind::ProMedium, crate::Services::default());
    h.run_steps(4);
    for label in ["InkPop", "基于开源软件PhotoCraft 版本0.5开发", "开发者：Mummy-fei", "photocraft0.5贡献者与模型", "算法、模型与第三方组件", "U²-Net / U2NETP"]
    {
        assert!(h.query_by_label(label).is_some(), "About must retain {label}");
    }
}

#[test]
fn about_sources_dispatch_through_the_platform_service() {
    use std::sync::{Arc, Mutex};
    let opened = Arc::new(Mutex::new(Vec::<String>::new()));
    let recording = opened.clone();
    let services = crate::Services {
        open_url: Some(Box::new(move |url| {
            recording.lock().unwrap().push(url.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut h = about_harness(crate::theme::ThemeKind::StudioLight, services);
    h.run_steps(4);
    let mut expected = Vec::new();
    for entry in GROUPS.iter().flat_map(|group| group.entries) {
        let label = format!("来源：{}", entry.name);
        h.get_by_label(&label).scroll_to_me();
        h.run_steps(3);
        h.get_by_label(&label).click();
        h.run_steps(3);
        expected.push(entry.source.to_string());
        assert_eq!(*opened.lock().unwrap(), expected, "one platform browser call for {}", entry.name);
    }
    let header = "第三方素材与来源索引（ATTRIBUTION.md）";
    let link = "来源：ATTRIBUTION 上游索引";
    h.get_by_label(header).scroll_to_me();
    h.run_steps(3);
    h.get_by_label(header).click();
    h.run_steps(4);
    h.get_by_label(link).scroll_to_me();
    h.run_steps(3);
    h.get_by_label(link).click();
    h.run_steps(3);
    expected.push("https://github.com/storytold/photocraft/blob/main/ATTRIBUTION.md".to_string());
    assert_eq!(*opened.lock().unwrap(), expected);
}

#[test]
#[ignore = "exports v66 five-theme About/source index PNGs; root must visually inspect"]
fn about_open_source_gpu_visual_v66() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v66/about-open-source-gpu");
    assert!(!out.exists(), "keep previous visual evidence");
    std::fs::create_dir_all(&out).unwrap();
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = Harness::builder().with_size(vec2(1100.0, 820.0)).with_max_steps(64).wgpu().build_eframe(move |cc| {
            crate::PhotocraftApp::setup_context(&cc.egui_ctx, theme);
            let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.ui.open_dialog(crate::state::DialogKind::About, serde_json::Map::new());
            app
        });
        h.run_steps(8);
        h.render().unwrap().save(out.join(format!("{}-home.png", theme.id()))).unwrap();
        h.get_by_label("第三方素材与来源索引（ATTRIBUTION.md）").scroll_to_me();
        h.run_steps(5);
        h.render().unwrap().save(out.join(format!("{}-sources-bottom.png", theme.id()))).unwrap();
    }
}
