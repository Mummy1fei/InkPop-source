use super::*;

#[test]
fn long_document_titles_are_bounded_and_tabs_remain_clickable_in_every_theme_v95() {
    for theme in crate::theme::ThemeKind::ALL {
        let name = format!("中文文件名_{}.jpg", "LongDownloadName".repeat(30));
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for title in [&name, "Second.png", "Third.psd"] {
            app.run("file.new", json!({"width":8,"height":8,"name":title})).unwrap();
        }
        app.session.set_active(0);
        app.run("layer.new.layer", json!({})).unwrap();
        app.session.set_active(2);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 60.0)).build_ui_state(
            |ui, (app, geometry, ready): &mut (PhotocraftApp, Vec<Rect>, bool)| {
                if *ready {
                    *geometry = tabs(app, ui).tabs;
                }
            },
            (app, Vec::new(), false),
        );
        PhotocraftApp::setup_context(&h.ctx, theme);
        h.state_mut().2 = true;
        h.run_steps(3);
        let geometry = &h.state().1;
        assert_eq!(geometry.len(), 3);
        assert!(geometry.iter().all(|r| r.width() <= DOCUMENT_TAB_MAX_WIDTH + 1.0), "{theme:?}: {geometry:?}");
        assert!(geometry.last().unwrap().right() < 1000.0, "one long name must not push its neighbours offscreen");
        assert!(geometry.windows(2).all(|pair| pair[0].right() <= pair[1].left() + 0.1));
        assert!(
            h.output().shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text)
                if text.galley.text().trim() == "*"
                    && geometry[0].contains_rect(Rect::from_min_size(text.pos, text.galley.size())))
            }),
            "{theme:?}: the dirty marker must survive filename truncation"
        );
        let second = geometry[1].center();
        h.event(egui::Event::PointerMoved(second));
        h.event(egui::Event::PointerButton { pos: second, button: PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.step();
        h.event(egui::Event::PointerButton { pos: second, button: PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.step();
        assert_eq!(h.state().0.session.active_index(), Some(1), "{theme:?}: second tab remains selectable");
        assert_eq!(h.state().0.session.documents()[0].doc.name, name, "only presentation changes the name");
    }
}
