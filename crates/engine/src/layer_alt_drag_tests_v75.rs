use super::*;
use photocraft_doc::Layer;

fn fixture(depth: u32) -> (Session, [LayerId; 3]) {
    let mut session = Session::new();
    session.execute("file.new", json!({"width":16,"height":12,"depth":depth,"background":"transparent"})).unwrap();
    let mut layers = Vec::new();
    for (name, color) in [("A", "#cc3322"), ("B", "#3366cc"), ("C", "#339966")] {
        layers.push(LayerId(session.execute("layer.new.layer", json!({"name":name})).unwrap()["layer"].as_u64().unwrap()));
        session.execute("edit.fill", json!({"color":color})).unwrap();
    }
    (session, layers.try_into().unwrap())
}

#[test]
fn layer_alt_drag_v75_copies_ordered_block_at_target_in_one_undo_without_moving_originals() {
    for depth in [8, 16, 32] {
        let (mut session, [a, b, c]) = fixture(depth);
        crate::layer_multi_cmds::set_selection(&mut session, vec![b, a], Some(a), Some(a)).unwrap();
        let before = session.active().unwrap().doc.clone();
        let selection = session.active().unwrap().selected_layers();
        let image = photocraft_compose::render(&before, before.bounds());
        let history = session.active().unwrap().history.past_len();
        let reply = session.execute("layer.moveTo", json!({"target":c.0,"position":"above","copy":true})).unwrap();
        let copies: Vec<_> = reply["layers"].as_array().unwrap().iter().map(|value| LayerId(value.as_u64().unwrap())).collect();
        assert_eq!(copies.len(), 2);
        let state = session.active().unwrap();
        assert_eq!(state.doc.layers.iter().map(|layer| layer.name.as_str()).collect::<Vec<_>>(), ["Layer 1", "A", "B", "C", "A copy", "B copy"]);
        for original in [a, b, c] {
            assert_eq!(state.doc.layer(original), before.layer(original));
        }
        for (original, copy) in [a, b].into_iter().zip(&copies) {
            assert_ne!(original, *copy);
            assert_eq!(state.doc.layer(original).unwrap().content, state.doc.layer(*copy).unwrap().content);
        }
        assert_eq!(state.selected_layers(), copies);
        assert_eq!(state.active_layer, copies.first().copied());
        assert_eq!(state.history.past_len(), history + 1);
        let after = state.doc.clone();
        assert!(session.undo());
        assert_eq!(session.active().unwrap().doc, before);
        assert_eq!(session.active().unwrap().selected_layers(), selection);
        assert_eq!(photocraft_compose::render(&session.active().unwrap().doc, before.bounds()).px, image.px);
        assert!(session.redo());
        assert_eq!(session.active().unwrap().doc, after, "redo retains exactly the same copied identities");
        assert_eq!(session.active().unwrap().selected_layers(), copies);
    }
}

#[test]
fn layer_alt_drag_v75_group_descendants_copy_once_with_fresh_ids_and_smart_content() {
    let (mut session, [a, b, c]) = fixture(32);
    session.execute("layer.select", json!({"layer":b.0})).unwrap();
    // Converting replaces the raster layer with a new Smart Object identity.
    let b = LayerId(session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap());
    crate::layer_multi_cmds::set_selection(&mut session, vec![a, b], Some(a), Some(a)).unwrap();
    let group = LayerId(session.execute("layer.new.group", json!({"name":"Source"})).unwrap()["layer"].as_u64().unwrap());
    session.execute("layer.select", json!({"layer":c.0})).unwrap();
    let destination = LayerId(session.execute("layer.new.group", json!({"name":"Destination","empty":true})).unwrap()["layer"].as_u64().unwrap());
    crate::layer_multi_cmds::set_selection(&mut session, vec![group, a], Some(a), Some(group)).unwrap();
    let before = session.active().unwrap().doc.clone();
    let reply = session.execute("layer.moveTo", json!({"layers":[group.0,a.0],"target":destination.0,"position":"into","copy":true})).unwrap();
    assert_eq!(reply["layers"].as_array().unwrap().len(), 1, "ancestor carries descendants only once");
    let copied = LayerId(reply["layers"][0].as_u64().unwrap());
    let doc = &session.active().unwrap().doc;
    assert_eq!(doc.layer(group), before.layer(group));
    assert_eq!(doc.layer(destination).unwrap().children().unwrap().iter().map(|layer| layer.id).collect::<Vec<_>>(), [copied]);
    let original = doc.layer(group).unwrap().children().unwrap();
    let children = doc.layer(copied).unwrap().children().unwrap();
    assert_eq!(children.len(), original.len());
    for (source, duplicate) in original.iter().zip(children) {
        assert_ne!(source.id, duplicate.id);
        assert_eq!(source.content, duplicate.content);
        assert_eq!(source.mask, duplicate.mask);
        assert_eq!(source.effects, duplicate.effects);
    }
    let all: Vec<_> = doc.walk().iter().map(|(_, _, layer)| layer.id).collect();
    assert_eq!(all.iter().copied().collect::<std::collections::HashSet<_>>().len(), all.len());
    assert!(session.undo());
    assert_eq!(session.active().unwrap().doc, before);
}

#[test]
fn layer_alt_drag_v75_relative_self_copy_and_invalid_destinations_are_atomic() {
    let (mut session, [a, b, c]) = fixture(8);
    let copy = session.execute("layer.moveTo", json!({"layer":a.0,"target":a.0,"position":"below","copy":true})).unwrap()["layers"][0].as_u64().unwrap();
    assert_eq!(
        session.active().unwrap().doc.layers.iter().map(|layer| layer.id).collect::<Vec<_>>(),
        [session.active().unwrap().doc.layers[0].id, LayerId(copy), a, b, c]
    );
    assert!(session.undo());
    session.execute("layer.select", json!({"layer":a.0})).unwrap();
    let group = session.execute("layer.groupLayers", json!({"layer":a.0,"name":"G"})).unwrap()["layer"].as_u64().unwrap();
    let before = session.active().unwrap().doc.clone();
    let history = session.active().unwrap().history.past_len();
    for params in [
        json!({"layer":group,"target":a.0,"position":"above","copy":true}),
        json!({"layer":group,"target":group,"position":"into","copy":true}),
        json!({"layer":b.0,"target":c.0,"position":"into","copy":true}),
        json!({"layers":[b.0,999999],"target":c.0,"copy":true}),
        json!({"layer":b.0,"target":c.0,"copy":"yes"}),
    ] {
        assert!(session.execute("layer.moveTo", params.clone()).is_err(), "{params}");
        assert_eq!(session.active().unwrap().doc, before);
        assert_eq!(session.active().unwrap().history.past_len(), history);
    }
}

#[test]
fn layer_alt_drag_v75_depth_failure_preserves_document_selection_and_history() {
    let (mut session, [a, _, _]) = fixture(8);
    let (source, target) = session
        .edit("bounded nested fixture", |doc, active| {
            let source = Layer::group("Source subtree", vec![Layer::raster("Child", doc.pixel_format())]);
            let source_id = source.id;
            let mut chain = Layer::group("Deep destination", Vec::new());
            let target = chain.id;
            for _ in 1..photocraft_doc::MAX_GROUP_DEPTH {
                chain = Layer::group("Ancestor", vec![chain]);
            }
            doc.layers.extend([source, chain]);
            *active = Some(a);
            crate::layer_multi_cmds::check_group_depth(doc, "fixture")?;
            Ok((source_id, target))
        })
        .unwrap();
    let before = session.active().unwrap().doc.clone();
    let selected = session.active().unwrap().selected_layers();
    let history = session.active().unwrap().history.past_len();
    assert!(session.execute("layer.moveTo", json!({"layer":source.0,"target":target.0,"position":"into","copy":true})).is_err());
    assert_eq!(session.active().unwrap().doc, before);
    assert_eq!(session.active().unwrap().selected_layers(), selected);
    assert_eq!(session.active().unwrap().history.past_len(), history);
}
