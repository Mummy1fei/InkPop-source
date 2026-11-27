//! Keep the native taskbar/Alt-Tab title in sync with the actual active file.
use crate::PhotocraftApp;

fn title(app: &PhotocraftApp) -> String {
    let product = photocraft_engine::build_info::CUSTOM_PRODUCT_TITLE;
    let Some(st) = app.session.active() else { return product.to_string() };
    let name = st.path.as_deref().and_then(|path| path.rsplit(['/', '\\']).next()).filter(|name| !name.is_empty()).unwrap_or(&st.doc.name);
    format!("{name}{} — {product}", if st.is_dirty() { " *" } else { "" })
}

pub(crate) fn sync(app: &PhotocraftApp, ctx: &egui::Context) {
    let key = egui::Id::new("native-active-document-title-v75");
    let title = title(app);
    if ctx.data(|data| data.get_temp::<String>(key)).as_deref() != Some(title.as_str()) {
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Title(title.clone()));
        ctx.data_mut(|data| data.insert_temp(key, title));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn taskbar_title_follows_active_file_save_dirty_undo_and_close_v75() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let product = photocraft_engine::build_info::CUSTOM_PRODUCT_TITLE;
        assert_eq!(title(&app), product);
        app.run("file.new", json!({"name":"first.png","width":16,"height":16})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        let st = app.session.active_mut().unwrap();
        st.path = Some("C:\\work\\保存后的地图.psd".into());
        st.saved_revision = st.revision;
        assert_eq!(title(&app), format!("保存后的地图.psd — {product}"));
        app.run("layer.new.layer", json!({})).unwrap();
        assert_eq!(title(&app), format!("保存后的地图.psd * — {product}"));
        app.run("edit.undo", json!({})).unwrap();
        assert!(title(&app).starts_with("保存后的地图.psd"));
        app.run("file.new", json!({"name":"second.png","width":16,"height":16})).unwrap();
        assert!(title(&app).starts_with("second.png"));
        app.session.set_active(0);
        assert!(title(&app).starts_with("保存后的地图.psd"));
        app.run("file.close", json!({})).unwrap();
        assert!(title(&app).starts_with("second.png"));
        app.run("file.close", json!({})).unwrap();
        assert_eq!(title(&app), product);
    }

    #[test]
    fn taskbar_title_is_sent_to_root_once_per_change_without_changing_child_title_v75() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        fn sent(output: &egui::FullOutput) -> Vec<String> {
            output
                .viewport_output
                .get(&egui::ViewportId::ROOT)
                .into_iter()
                .flat_map(|v| &v.commands)
                .filter_map(|c| if let egui::ViewportCommand::Title(title) = c { Some(title.clone()) } else { None })
                .collect()
        }
        let mut first = ctx.run_ui(Default::default(), |_| sync(&app, &ctx));
        assert_eq!(sent(&first), vec![title(&app)]);
        first.textures_delta.clear();
        let mut same = ctx.run_ui(Default::default(), |_| sync(&app, &ctx));
        assert!(sent(&same).is_empty());
        same.textures_delta.clear();
        app.run("file.new", json!({"name":"Active file.psd","width":16,"height":16})).unwrap();
        let mut changed = ctx.run_ui(Default::default(), |_| sync(&app, &ctx));
        assert_eq!(sent(&changed), vec![title(&app)]);
        changed.textures_delta.clear();
    }
}
