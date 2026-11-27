//! Exact shared-service behavior, independent of whichever model Auto selects.
use super::*;
use photocraft_geom::Rect;

fn fixture(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":80,"height":64,"depth":depth})).unwrap();
    s.edit("fixture", |doc, _| {
        let surface = doc.layers[0].surface_mut().unwrap();
        let channels = surface.channels();
        let mut values = Vec::with_capacity(80 * 64 * channels);
        for y in 0..64 {
            for x in 0..80 {
                let (dx, dy) = (x as f32 - 40.0, y as f32 - 32.0);
                let color: [f32; 3] = if dx * dx + dy * dy < 22.0 * 22.0 { [0.9, 0.1, 0.15] } else { [0.15, 0.3, 0.8] };
                values.extend_from_slice(&color[..channels.min(3)]);
                if channels == 4 {
                    values.push(1.0);
                }
            }
        }
        surface.write_region(Rect::new(0, 0, 80, 64), &values);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn mask_all_objects_equals_menu_subject_for_the_same_source_and_is_one_undo_v93() {
    for depth in [8, 16, 32] {
        let mut s = fixture(depth);
        let id = s.active().unwrap().active_layer.unwrap();
        let bounds = s.active().unwrap().doc.bounds();
        let pixels = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().read_region(bounds);
        let selected = s.execute("select.subject", json!({"sampleAllLayers":false})).unwrap();
        let expected = s.active().unwrap().doc.selection.as_ref().unwrap().read_region(bounds);
        s.execute("edit.undo", json!({})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let steps = s.active().unwrap().history.past_len();
        let result = s.execute("layer.maskAllObjects", json!({"layer":id.0})).unwrap();
        assert_eq!(result["backend"], selected["backend"]);
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
        let layer = s.active().unwrap().doc.layer(id).unwrap();
        assert_eq!(layer.mask.as_ref().unwrap().surface.read_region(bounds), expected, "depth {depth}");
        assert_eq!(layer.surface().unwrap().read_region(bounds), pixels);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, *before);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.layer(id).unwrap().mask.as_ref().unwrap().surface.read_region(bounds), expected);
    }
}

#[test]
fn remove_background_without_extra_refine_equals_shared_subject_service_v93() {
    let mut s = fixture(16);
    let id = s.active().unwrap().active_layer.unwrap();
    let bounds = s.active().unwrap().doc.bounds();
    let (region, backend, _) = recognize_subject_region(
        &s.active().unwrap().doc,
        Some(id),
        false,
        s.prefs().performance.subject_recognition_quality.backend(),
        &crate::jobs::JobCtx::new(),
    )
    .unwrap();
    let expected = photocraft_algo::matting::region_surface(&region).read_region(bounds);
    let result = s.execute(CMD, json!({"refine":false})).unwrap();
    assert_eq!(result["backend"], backend);
    assert_eq!(s.active().unwrap().doc.layer(id).unwrap().mask.as_ref().unwrap().surface.read_region(bounds), expected);
}

#[test]
fn shared_subject_layer_source_does_not_inherit_a_visible_lower_layers_subject_v93() {
    let mut s = fixture(8);
    s.execute("layer.new.layer", json!({"name":"Empty target"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    let before = s.active().unwrap().doc.clone();
    assert!(s.execute("layer.maskAllObjects", json!({})).is_err());
    assert_eq!(*s.active().unwrap().doc, *before);
    assert!(s.execute(CMD, json!({"sampleAllLayers":false})).is_err());
    assert_eq!(*s.active().unwrap().doc, *before);
    s.execute(CMD, json!({"sampleAllLayers":true,"refine":false})).unwrap();
    assert!(s.active().unwrap().doc.layer(id).unwrap().mask.is_some());
}

#[test]
fn shared_subject_region_cancellation_leaves_input_unchanged_v93() {
    let s = fixture(32);
    let original = s.active().unwrap().doc.clone();
    let ctx = crate::jobs::JobCtx::new();
    ctx.cancel();
    assert!(matches!(
        recognize_subject_region(&original, s.active().unwrap().active_layer, true, s.prefs().performance.subject_recognition_quality.backend(), &ctx),
        Err(EngineError::Cancelled)
    ));
    assert_eq!(*s.active().unwrap().doc, *original);
}
