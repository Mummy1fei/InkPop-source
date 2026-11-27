//! ROOT retains a docked view even when a floating document owns the editor focus.
use crate::PhotocraftApp;

fn detached(app: &PhotocraftApp, index: usize) -> bool {
    app.ui.windows.iter().any(|window| window.open && window.contains_document(index))
}

/// Document ids survive close/reindex. The bounded MRU is pure view state.
pub(crate) fn index(app: &PhotocraftApp, ctx: &egui::Context) -> Option<usize> {
    let active = app.session.active_index()?;
    let documents = app.session.documents();
    let key = egui::Id::new("root-docked-document-history-v74");
    let mut history = ctx.data(|data| data.get_temp::<Vec<u64>>(key)).unwrap_or_default();
    if !detached(app, active) {
        let id = documents.get(active)?.doc.id.0;
        if history.last() != Some(&id) {
            history.retain(|old| *old != id);
            history.push(id);
            if history.len() > 32 {
                history.remove(0);
            }
            ctx.data_mut(|data| data.insert_temp(key, history));
        }
        return Some(active);
    }
    for id in history.into_iter().rev() {
        if let Some(index) = documents.iter().position(|state| state.doc.id.0 == id)
            && !detached(app, index)
        {
            return Some(index);
        }
    }
    // A control-created detached window can precede ROOT's first frame.
    (0..active).rev().find(|index| !detached(app, *index)).or_else(|| (active + 1..documents.len()).find(|index| !detached(app, *index)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        for color in ["#dd3311", "#2255dd", "#33bb66"] {
            app.run("file.new", json!({"width":120,"height":90})).unwrap();
            app.run("edit.fill", json!({"color":color})).unwrap();
        }
        app.sync_views();
        app
    }

    #[test]
    fn root_docked_view_keeps_mru_without_stealing_floating_focus_v74() {
        let mut app = fixture();
        let ctx = egui::Context::default();
        app.session.set_active(0);
        assert_eq!(index(&app, &ctx), Some(0));
        app.session.set_active(2);
        assert_eq!(index(&app, &ctx), Some(2));
        crate::view_cmds::float_window(&mut app, 2, 0);
        assert_eq!(index(&app, &ctx), Some(0), "last ROOT document remains painted");
        assert_eq!(app.session.active_index(), Some(2), "floating document still owns editing");
        crate::view_cmds::float_window(&mut app, 0, 0);
        assert_eq!(index(&app, &ctx), Some(1));
        crate::view_cmds::float_window(&mut app, 1, 0);
        assert_eq!(index(&app, &ctx), None, "all detached means no duplicate ROOT canvas");
        app.ui.windows.retain(|window| window.document != 0);
        assert_eq!(index(&app, &ctx), Some(0), "redocking immediately repopulates ROOT");
    }

    #[test]
    fn root_docked_view_works_before_first_frame_and_after_close_reindex_v74() {
        let mut app = fixture();
        let ctx = egui::Context::default();
        crate::view_cmds::float_window(&mut app, 2, 0);
        assert_eq!(index(&app, &ctx), Some(1));
        app.session.set_active(1);
        assert_eq!(index(&app, &ctx), Some(1));
        app.session.set_active(0);
        app.run("file.close", json!({"document":0})).unwrap();
        app.sync_views();
        app.session.set_active(1);
        assert_eq!(index(&app, &ctx), Some(0), "MRU is resolved by document id, not stale index");
    }

    #[test]
    #[ignore = "writes ROOT background with another document detached for visual review"]
    fn root_docked_background_gpu_visual_v74() {
        struct Canvas(PhotocraftApp);
        impl eframe::App for Canvas {
            fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(&mut self.0, ui));
            }
        }
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v74/root-background-gpu");
        assert!(!out.exists(), "preserve previous evidence");
        std::fs::create_dir_all(&out).unwrap();
        for theme in crate::theme::ThemeKind::ALL {
            let mut h = egui_kittest::Harness::builder()
                .with_size(egui::vec2(1000.0, 700.0))
                .with_max_steps(64)
                .wgpu_setup(crate::gpu_canvas::wgpu_setup())
                .build_eframe(move |cc| {
                    PhotocraftApp::setup_context(&cc.egui_ctx, theme);
                    let mut app = fixture();
                    app.session.set_active(0);
                    app.ui.extras.rulers = false;
                    Canvas(app)
                });
            h.run_steps(5);
            crate::view_cmds::float_window(&mut h.state_mut().0, 2, 0);
            h.state_mut().0.session.set_active(2);
            h.run_steps(5);
            assert_eq!(h.state().0.session.active_index(), Some(2));
            assert_eq!(index(&h.state().0, &h.ctx), Some(0));
            h.render().unwrap().save(out.join(format!("{}-docked-behind-floating.png", theme.id()))).unwrap();
        }
    }
}
