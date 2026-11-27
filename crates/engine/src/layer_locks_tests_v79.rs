use super::*;
use photocraft_doc::{LayerMask, Locks};
use std::sync::Arc;

fn fixture(depth: u32) -> (Session, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":20,"height":16,"depth":depth,"background":"transparent"})).unwrap();
    let a = LayerId(s.execute("layer.new.layer", json!({"name":"A"})).unwrap()["layer"].as_u64().unwrap());
    let b = LayerId(s.execute("layer.new.layer", json!({"name":"B"})).unwrap()["layer"].as_u64().unwrap());
    (s, a, b)
}

fn locks(s: &Session, id: LayerId) -> Locks {
    s.active().unwrap().doc.layer(id).unwrap().locks
}

fn rgba(s: &Session, id: LayerId) -> Vec<[f32; 4]> {
    let mut values = vec![[0.0; 4]; 20 * 16];
    s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().read_rgba_into(Rect::new(0, 0, 20, 16), &mut values);
    values
}

fn assert_rejected_unchanged(s: &mut Session, command: &str, params: Value) {
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    let revision = s.active().unwrap().revision;
    assert!(s.execute(command, params).is_err(), "{command}");
    assert!(Arc::ptr_eq(&s.active().unwrap().doc, &before), "{command}: no partial edits");
    assert_eq!(s.active().unwrap().history.past_len(), history, "{command}");
    assert_eq!(s.active().unwrap().revision, revision, "{command}");
}

#[test]
fn layer_locks_v79_mixed_selection_toggle_is_uniform_and_preserves_independent_flags() {
    let (mut s, a, b) = fixture(8);
    s.execute("layer.lockLayers", json!({"layer":a.0,"position":true,"artboard":true})).unwrap();
    set_selection(&mut s, vec![a, b], Some(a), Some(a)).unwrap();
    s.execute("layer.lockLayers", json!({"toggle":"position"})).unwrap();
    assert!(locks(&s, a).position && locks(&s, b).position);
    assert!(locks(&s, a).artboard && !locks(&s, b).artboard);
    s.execute("layer.lockLayers", json!({"toggle":"position"})).unwrap();
    assert!(!locks(&s, a).position && !locks(&s, b).position);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(locks(&s, a).position && locks(&s, b).position);
    s.execute("edit.redo", json!({})).unwrap();
    assert!(!locks(&s, a).position && !locks(&s, b).position);
    s.execute("layer.lockLayers", json!({})).unwrap();
    assert!(locks(&s, a).all && locks(&s, b).all);
    s.execute("layer.lockLayers", json!({})).unwrap();
    assert!(!locks(&s, a).all && !locks(&s, b).all);
    assert!(locks(&s, a).artboard);
    let selection_before = selected(&s);
    s.execute("layer.lockLayers", json!({"layers":[b.0,a.0],"pixels":true})).unwrap();
    assert!(locks(&s, a).pixels && locks(&s, b).pixels);
    assert_eq!(selected(&s), selection_before, "explicit targets do not replace selection");
}

#[test]
fn layer_locks_v79_invalid_requests_never_turn_into_an_accidental_lock_all_toggle() {
    let (mut s, a, _) = fixture(8);
    for invalid in [
        Value::Null,
        json!({"pixels":"true"}),
        json!({"all":null}),
        json!({"all":1}),
        json!({"toggle":"unknown"}),
        json!({"toggle":true}),
        json!({"toggle":"pixels","all":true}),
        json!({"misspelled":true}),
        json!({"layers":[]}),
        json!({"layers":[u64::MAX]}),
        json!({"layer":"invalid"}),
        json!({"layer":a.0,"layers":[a.0]}),
    ] {
        assert_rejected_unchanged(&mut s, "layer.lockLayers", invalid);
    }
    let past = s.active().unwrap().history.past_len();
    let result = s.execute("layer.lockLayers", json!({"layer":a.0,"pixels":false})).unwrap();
    assert_eq!(result["changed"], 0);
    assert_eq!(s.active().unwrap().history.past_len(), past, "setting an unchanged value is not an edit");
}

#[test]
fn layer_locks_v79_pixel_specific_controls_skip_non_raster_types_without_replacing_selection() {
    let (mut s, a, _) = fixture(8);
    let group = LayerId(s.execute("layer.new.group", json!({"name":"G","empty":true})).unwrap()["layer"].as_u64().unwrap());
    set_selection(&mut s, vec![a, group], Some(group), Some(group)).unwrap();
    let selection_before = selected(&s);
    s.execute("layer.lockLayers", json!({"toggle":"pixels"})).unwrap();
    assert!(locks(&s, a).pixels);
    assert!(!locks(&s, group).pixels);
    assert_eq!(selected(&s), selection_before);
    assert_rejected_unchanged(&mut s, "layer.lockLayers", json!({"layer":group.0,"toggle":"transparency"}));
    s.execute("layer.lockLayers", json!({"toggle":"position"})).unwrap();
    assert!(locks(&s, a).position && locks(&s, group).position);
}

#[test]
fn layer_locks_v79_transparency_keeps_alpha_but_pixel_lock_blocks_paint_not_movement_at_three_depths() {
    for depth in [8, 16, 32] {
        let (mut s, a, _) = fixture(depth);
        s.select_layer(a).unwrap();
        s.edit("Synthetic translucent paint", |doc, _| {
            let surf = doc.layer_mut(a).unwrap().surface_mut().unwrap();
            let pixel = photocraft_raster::from_rgba(&surf.format(), [0.2, 0.4, 0.7, 0.5]);
            surf.fill_rect(Rect::new(4, 4, 12, 12), &pixel);
            Ok(())
        })
        .unwrap();
        let alpha: Vec<_> = rgba(&s, a).iter().map(|p| p[3]).collect();
        s.execute("layer.lockLayers", json!({"transparency":true})).unwrap();
        s.execute("paint.stroke", json!({"points":[[0,8],[19,8]],"size":8,"color":"#ff0000"})).unwrap();
        let after = rgba(&s, a);
        assert_eq!(after.iter().map(|p| p[3]).collect::<Vec<_>>(), alpha, "{depth}: alpha is protected");
        assert!(after[8 * 20 + 8][0] > 0.5, "{depth}: opaque content can be painted");
        s.execute("layer.lockLayers", json!({"pixels":true})).unwrap();
        assert_rejected_unchanged(&mut s, "paint.stroke", json!({"points":[[8,8]],"size":4}));
        s.execute("layer.translate", json!({"dx":1,"dy":1})).unwrap();
    }
}

#[test]
fn layer_locks_v79_position_protects_geometry_while_all_also_protects_masks() {
    let (mut s, a, _) = fixture(8);
    s.select_layer(a).unwrap();
    s.edit("Synthetic mask", |doc, _| {
        doc.layer_mut(a).unwrap().mask = Some(LayerMask::reveal_all());
        Ok(())
    })
    .unwrap();
    s.execute("layer.lockLayers", json!({"position":true})).unwrap();
    s.execute("paint.stroke", json!({"points":[[8,8]],"size":4})).unwrap();
    assert_rejected_unchanged(&mut s, "layer.translate", json!({"dx":1}));
    assert_rejected_unchanged(&mut s, "edit.transform", json!({"matrix":[1,0,0,1,2,0]}));
    s.execute("layer.lockLayers", json!({"position":false,"pixels":true})).unwrap();
    s.execute("paint.stroke", json!({"target":"mask","points":[[8,8]],"size":4,"color":"#000000"})).unwrap();
    s.execute("layer.lockLayers", json!({"all":true})).unwrap();
    assert_rejected_unchanged(&mut s, "paint.stroke", json!({"target":"mask","points":[[4,4]],"size":4}));
}

#[test]
fn layer_locks_v79_all_locked_groups_protect_descendants_and_structural_operations_atomically() {
    let (mut s, a, b) = fixture(8);
    s.select_layer(a).unwrap();
    let group = LayerId(s.execute("layer.new.groupFromLayers", json!({"name":"G"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.lockLayers", json!({"layer":group.0,"all":true})).unwrap();
    s.select_layer(a).unwrap();
    s.execute("layer.lockLayers", json!({"all":false})).unwrap();
    assert!(s.active().unwrap().doc.effective_locks(a).all, "a child cannot clear its ancestor's lock");
    for (command, params) in [
        ("paint.stroke", json!({"points":[[5,5]],"size":3})),
        ("layer.translate", json!({"dx":2})),
        ("layer.delete", json!({"layer":a.0})),
        ("layer.moveTo", json!({"layer":a.0,"target":b.0,"position":"above"})),
        ("layer.arrange.bringToFront", json!({"layer":a.0})),
        ("layer.new.groupFromLayers", json!({})),
    ] {
        assert_rejected_unchanged(&mut s, command, params);
    }
    s.execute("layer.lockLayers", json!({"layer":group.0,"all":false})).unwrap();
    s.execute("layer.translate", json!({"dx":2})).unwrap();
}

#[test]
fn layer_locks_v79_locked_children_cannot_be_changed_through_moving_or_deleting_their_group() {
    let (mut s, a, _) = fixture(8);
    s.select_layer(a).unwrap();
    let group = LayerId(s.execute("layer.new.groupFromLayers", json!({"name":"G"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.lockLayers", json!({"layer":a.0,"position":true})).unwrap();
    s.select_layer(group).unwrap();
    assert_rejected_unchanged(&mut s, "layer.translate", json!({"dx":3}));
    assert!(moved(&s.active().unwrap().doc, &[group], 3, 0).is_err(), "live preview follows commit protection");
    s.execute("layer.lockLayers", json!({"layer":a.0,"position":false,"all":true})).unwrap();
    assert_rejected_unchanged(&mut s, "layer.delete", json!({"layer":group.0}));
}

#[test]
fn layer_locks_v79_legacy_set_props_rejects_invalid_lock_values_without_partial_property_edits() {
    let (mut s, a, _) = fixture(8);
    for invalid in [
        json!({"locked":"true"}),
        json!({"locked":null}),
        json!({"locks":true}),
        json!({"locks":{"pixels":"true"}}),
        json!({"locks":{"all":null}}),
        json!({"locks":{"bogus":true}}),
    ] {
        let mut request = invalid;
        request["layer"] = json!(a.0);
        request["name"] = json!("Must not be partially renamed");
        assert_rejected_unchanged(&mut s, "layer.setProps", request);
    }
}

fn artboards() -> (Session, LayerId, LayerId, LayerId) {
    let (mut s, a, _) = fixture(8);
    s.select_layer(a).unwrap();
    s.edit("Synthetic artboard content", |doc, _| {
        let surface = doc.layer_mut(a).unwrap().surface_mut().unwrap();
        let pixel = photocraft_raster::from_rgba(&surface.format(), [0.8, 0.3, 0.1, 1.0]);
        surface.fill_rect(Rect::new(5, 5, 15, 15), &pixel);
        Ok(())
    })
    .unwrap();
    let first = LayerId(s.execute("layer.new.artboard", json!({"name":"Left","rect":[0,0,40,40]})).unwrap()["layer"].as_u64().unwrap());
    let second = LayerId(s.execute("layer.new.artboard", json!({"name":"Right","rect":[60,0,40,40]})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.moveTo", json!({"layer":a.0,"target":first.0,"position":"into"})).unwrap();
    s.select_layer(a).unwrap();
    (s, a, first, second)
}

#[test]
fn layer_locks_v79_artboard_auto_membership_matches_preview_commit_undo_and_leaving_boards() {
    let (mut s, a, first, second) = artboards();
    let before = s.active().unwrap().doc.clone();
    let preview = moved(&before, &[a], 60, 0).unwrap();
    assert_eq!(preview.artboard_of(a), Some(second));
    s.execute("layer.translate", json!({"dx":60})).unwrap();
    assert_eq!(s.active().unwrap().doc.artboard_of(a), Some(second));
    assert_eq!(preview.layer(a).unwrap().surface(), s.active().unwrap().doc.layer(a).unwrap().surface());
    assert_eq!(s.active().unwrap().selected_layers(), vec![a]);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.artboard_of(a), Some(first));
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.artboard_of(a), Some(second));
    s.execute("layer.translate", json!({"dx":60})).unwrap();
    assert_eq!(s.active().unwrap().doc.artboard_of(a), None, "leaving every board moves to the root");
    assert_eq!(s.active().unwrap().doc.path_of(a).unwrap().len(), 1);
    assert_eq!(s.active().unwrap().doc.layer(a).unwrap().surface().unwrap().content_bounds(), Rect::new(125, 5, 135, 15));
}

#[test]
fn layer_locks_v79_prevent_auto_nesting_restricts_membership_without_locking_geometry_or_manual_moves() {
    for which in ["layer", "source-board", "target-board", "target-all"] {
        let (mut s, a, first, second) = artboards();
        let locked = match which {
            "source-board" => first,
            "target-board" | "target-all" => second,
            _ => a,
        };
        let key = if which == "target-all" { "all" } else { "artboard" };
        let mut request = json!({"layer":locked.0});
        request[key] = json!(true);
        s.execute("layer.lockLayers", request).unwrap();
        let preview = moved(&s.active().unwrap().doc, &[a], 60, 0).unwrap();
        assert_eq!(preview.artboard_of(a), Some(first), "{which}: preview must honor membership lock");
        s.execute("layer.translate", json!({"dx":60})).unwrap();
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.artboard_of(a), Some(first), "{which}: membership retained");
        assert_eq!(doc.layer(a).unwrap().surface().unwrap().content_bounds(), Rect::new(65, 5, 75, 15), "{which}: geometry still moves");
        if which != "target-all" {
            s.execute("layer.moveTo", json!({"layer":a.0,"target":second.0,"position":"into"})).unwrap();
            assert_eq!(s.active().unwrap().doc.artboard_of(a), Some(second), "{which}: explicit user nesting remains available");
        }
    }
}

#[test]
fn layer_locks_v79_artboard_lock_on_a_child_cannot_be_bypassed_by_moving_its_group() {
    let (mut s, a, first, second) = artboards();
    let group = LayerId(s.execute("layer.new.groupFromLayers", json!({"name":"Grouped content"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.lockLayers", json!({"layer":a.0,"artboard":true})).unwrap();
    s.select_layer(group).unwrap();
    s.execute("layer.translate", json!({"dx":60})).unwrap();
    assert_eq!(s.active().unwrap().doc.artboard_of(group), Some(first));
    assert_eq!(s.active().unwrap().doc.artboard_of(a), Some(first));
    assert_eq!(s.active().unwrap().doc.layer(a).unwrap().surface().unwrap().content_bounds(), Rect::new(65, 5, 75, 15));
    // Artboards themselves are never auto-nested into an overlapping board.
    s.execute("layer.lockLayers", json!({"layer":second.0,"artboard":true})).unwrap();
    s.execute("layer.translate", json!({"layer":second.0,"dx":-60})).unwrap();
    assert_eq!(s.active().unwrap().doc.path_of(second).unwrap().len(), 1);
    assert_eq!(s.active().unwrap().doc.layer(second).unwrap().artboard().unwrap().rect, Rect::new(0, 0, 40, 40));
}

#[test]
fn layer_locks_v79_artboard_position_lock_leaves_contents_editable_and_moveable() {
    let (mut s, a, first, _) = artboards();
    s.execute("layer.lockLayers", json!({"layer":first.0,"position":true})).unwrap();
    assert_rejected_unchanged(&mut s, "layer.translate", json!({"layer":first.0,"dx":1}));
    let preview = moved(&s.active().unwrap().doc, &[a], 2, 0).unwrap();
    assert_eq!(preview.layer(a).unwrap().surface().unwrap().content_bounds(), Rect::new(7, 5, 17, 15));
    s.execute("layer.translate", json!({"layer":a.0,"dx":2})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer(a).unwrap().surface().unwrap().content_bounds(), Rect::new(7, 5, 17, 15));
    s.select_layer(first).unwrap();
    let added = LayerId(s.execute("layer.new.layer", json!({"name":"Added to position-locked board"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.moveTo", json!({"layer":added.0,"target":first.0,"position":"into"})).unwrap();
    assert_eq!(s.active().unwrap().doc.artboard_of(added), Some(first));
    s.execute("layer.delete", json!({"layer":added.0})).unwrap();
    s.execute("layer.delete", json!({"layer":a.0})).unwrap();
    assert!(s.active().unwrap().doc.layer(first).is_some());
    assert!(s.active().unwrap().doc.layer(a).is_none());
}

#[test]
fn layer_locks_v79_type_pixel_locks_are_fixed_selected_while_raster_controls_remain_editable() {
    let text = Layer::new("Type", LayerContent::Text(photocraft_doc::TextLayer::default()));
    for key in ["pixels", "transparency"] {
        assert!(!supports_lock(&text, key));
        assert!(fixed_lock_selected(&text, key));
    }
    assert!(!fixed_lock_selected(&text, "position"));
    let (s, a, _) = fixture(8);
    let raster = s.active().unwrap().doc.layer(a).unwrap();
    assert!(supports_lock(raster, "pixels"));
    assert!(!fixed_lock_selected(raster, "pixels"));
}
