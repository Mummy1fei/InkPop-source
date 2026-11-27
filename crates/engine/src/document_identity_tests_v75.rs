use super::*;
use serde_json::json;

#[test]
fn saved_file_identity_survives_undo_redo_while_layer_history_is_restored_v75() {
    for path in ["C:\\work\\地图.psd", "/work/地图.psd", "地图.psd"] {
        let mut s = Session::new();
        s.execute("file.new", json!({"name":"old.png","width":16,"height":16})).unwrap();
        s.execute("layer.new.layer", json!({"name":"Edited layer"})).unwrap();
        let layer_count = s.active().unwrap().doc.layers.len();
        let st = s.active_mut().unwrap();
        st.path = Some(path.into());
        Arc::make_mut(&mut st.doc).name = "地图.psd".into();
        st.saved_revision = st.revision;
        assert!(s.undo());
        let st = s.active().unwrap();
        assert_eq!(st.doc.name, "地图.psd");
        assert_eq!(st.path.as_deref(), Some(path));
        assert_eq!(st.doc.layers.len(), layer_count - 1);
        assert!(s.redo());
        let st = s.active().unwrap();
        assert_eq!(st.doc.name, "地图.psd");
        assert_eq!(st.path.as_deref(), Some(path));
        assert_eq!(st.doc.layers.len(), layer_count);
    }
}

#[test]
fn unsaved_document_names_still_follow_their_history_v75() {
    let mut s = Session::new();
    s.execute("file.new", json!({"name":"Unsaved original","width":16,"height":16})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    Arc::make_mut(&mut s.active_mut().unwrap().doc).name = "Current unsaved name".into();
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.name, "Unsaved original");
    assert!(s.active().unwrap().path.is_none());
}
