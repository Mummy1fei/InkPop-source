use super::*;
use photocraft_doc::LayerContent;
use serde_json::json;

fn image_bytes() -> Vec<u8> {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width":5,"height":4,"background":"white"})).unwrap();
    photocraft_io::export(&session.active().unwrap().doc, "place.png", &Default::default()).unwrap().bytes
}

fn app_for_place(smart: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width":17,"height":13})).unwrap();
    app.session.prefs.edit(|p| p.general.always_create_smart_objects_when_placing = smart);
    app
}

fn active_is_smart(app: &PhotocraftApp) -> bool {
    let state = app.session.active().unwrap();
    matches!(state.doc.layer(state.active_layer.unwrap()).unwrap().content, LayerContent::Smart(_))
}

#[test]
fn gui_byte_placement_follows_choice_but_linked_is_always_smart_v89() {
    for smart in [false, true] {
        let mut app = app_for_place(smart);
        let result = app.place_bytes("place.png", image_bytes(), None).unwrap();
        assert_eq!(result["asSmartObject"], smart);
        assert_eq!(active_is_smart(&app), smart);
        let result = app.place_bytes("place.png", image_bytes(), Some("linked.png".into())).unwrap();
        assert_eq!(result["asSmartObject"], true);
        assert!(active_is_smart(&app));
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
struct DropImage(std::path::PathBuf, Vec<u8>);

#[cfg(not(target_arch = "wasm32"))]
impl egui::DroppedFile for DropImage {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        Ok(self.1.clone())
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_path_and_memory_drop_placement_follow_same_choice_v89() {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    for smart in [false, true] {
        for disk in [false, true] {
            let mut app = app_for_place(smart);
            let bytes = image_bytes();
            let path = if disk {
                let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!("inkpop-place-v89-{}-{serial}.png", std::process::id()));
                std::fs::write(&path, &bytes).unwrap();
                path
            } else {
                std::path::PathBuf::from("memory.png")
            };
            let document = app.session.active().unwrap().doc.id;
            app.drop_places.push_back(crate::file_open::DroppedPlacement { file: std::sync::Arc::new(DropImage(path.clone(), bytes)), document });
            app.place_next_dropped(&egui::Context::default());
            assert!(app.ui.transform.is_some(), "smart={smart},disk={disk}");
            assert_eq!(active_is_smart(&app), smart, "smart={smart},disk={disk}");
            assert!(app.drop_places.is_empty());
            crate::transform_tool::cancel(&mut app);
            if disk {
                std::fs::remove_file(&path).unwrap();
            }
        }
    }
}
