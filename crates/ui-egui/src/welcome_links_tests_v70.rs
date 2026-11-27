use super::*;
use egui_kittest::{Harness, kittest::Queryable};
use std::sync::{Arc, Mutex};

type Opened = Arc<Mutex<Vec<String>>>;

fn harness(width: f32, show_contacts: bool) -> (Harness<'static, PhotocraftApp>, Opened) {
    let opened: Opened = Default::default();
    let recorded = opened.clone();
    let mut initialized = false;
    let mut harness = Harness::builder().with_size(egui::vec2(width, 200.0)).build_ui_state(
        move |ui, app| {
            let _language = crate::i18n::language_scope(crate::i18n::Lang::EN);
            if !initialized {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::StudioLight);
                initialized = true;
            }
            if show_contacts {
                contacts(app, ui);
            } else {
                show(ui);
            }
        },
        PhotocraftApp::new(
            photocraft_engine::Session::new(),
            crate::Services {
                open_url: Some(Box::new(move |url| {
                    recorded.lock().unwrap().push(url.to_string());
                    Ok(())
                })),
                ..Default::default()
            },
        ),
    );
    harness.run_steps(5);
    (harness, opened)
}

#[test]
fn about_contacts_open_only_clicked_canonical_urls_through_mock_service_v72() {
    let (mut harness, opened) = harness(500.0, true);
    harness.get_by_label(EMAIL);
    assert!(opened.lock().unwrap().is_empty(), "rendering must not launch a browser or mail composer");
    for (label, url) in [("Xiaohongshu", XIAOHONGSHU), ("Weibo", WEIBO), (EMAIL, EMAIL_URL)] {
        harness.get_by_label(label).click();
        harness.run_steps(3);
        assert_eq!(opened.lock().unwrap().last().map(String::as_str), Some(url));
        assert_eq!(harness.state().ui.status, format!("Opened {url}"));
    }
    assert_eq!(opened.lock().unwrap().as_slice(), [XIAOHONGSHU, WEIBO, EMAIL_URL]);
    for url in [XIAOHONGSHU, WEIBO, EMAIL_URL] {
        assert!(!url.contains('?') && !url.contains("xsec_") && !url.contains("token"));
    }
}

#[test]
fn welcome_footer_keeps_copyright_and_removes_contact_links_v72() {
    for width in [280.0, 500.0] {
        let (harness, opened) = harness(width, false);
        let rect = harness.get_by_label(COPYRIGHT).rect();
        assert!(rect.left() >= 0.0 && rect.right() <= width + 0.5, "copyright must fit the {width}-point footer");
        for label in ["Xiaohongshu", "Weibo", EMAIL, "Join us on Discord", "PhotoCraft website", "GitHub", "ArtCraft"] {
            assert!(harness.query_by_label(label).is_none());
        }
        assert!(opened.lock().unwrap().is_empty());
    }
}
