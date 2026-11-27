use super::*;
use photocraft_doc::{Effect, LayerMask, SmartObject};
use serde_json::json;
use std::sync::Arc;

fn base_document() -> photocraft_doc::Document {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width":32,"height":24,"background":"transparent"})).unwrap();
    (*session.active().unwrap().doc).clone()
}

fn pixel(doc: &photocraft_doc::Document, name: &str) -> Layer {
    Layer::new(name, doc.layers[0].content.clone())
}

#[test]
fn filters_use_real_layer_metadata_and_do_not_modify_document_v79() {
    let mut doc = base_document();
    let mut paint = pixel(&doc, "Paint SKY 天空");
    paint.visible = false;
    paint.label = LabelColor::Blue;
    paint.blend = BlendMode::Multiply;
    paint.clipped = true;
    paint.locks.position = true;
    paint.mask = Some(LayerMask::reveal_all());
    paint.effects.items.push(Effect::default_drop_shadow());
    let chosen = [paint.id];
    doc.layers.push(paint.clone());
    let before = doc.layers.clone();
    for (mode, value, query, want) in [
        (Mode::Kind, "any", "", true),
        (Mode::Name, "any", "sky 天", true),
        (Mode::Name, "any", "unrelated", false),
        (Mode::Effect, "Drop Shadow", "", true),
        (Mode::Effect, "none", "", false),
        (Mode::Blend, "Multiply", "", true),
        (Mode::Blend, "Normal", "", false),
        (Mode::Attribute, "hidden", "", true),
        (Mode::Attribute, "visible", "", false),
        (Mode::Attribute, "locked", "", true),
        (Mode::Attribute, "clipped", "", true),
        (Mode::Attribute, "mask", "", true),
        (Mode::Attribute, "vectorMask", "", false),
        (Mode::Color, "blue", "", true),
        (Mode::Color, "red", "", false),
        (Mode::Selected, "any", "", true),
        (Mode::Artboard, "any", "", false),
        (Mode::SmartObject, "any", "", false),
    ] {
        let options = Options { mode, value: value.into(), query: query.into(), ..Default::default() };
        assert_eq!(options.matches(&paint, &["pixel".into()], &chosen), want, "{mode:?}/{value}/{query}");
    }
    assert_eq!(doc.layers, before, "metadata lookup never changes layer visibility/locks/selection or effects");
    let mut options = Options { mode: Mode::Name, query: "not present".into(), ..Default::default() };
    options.enabled = false;
    assert!(options.matches(&paint, &["type".into()], &[]));
    assert!(!options.active(&["type".into()]), "disable preserves filters without applying them");
}

#[test]
fn smart_object_source_and_filter_predicates_do_not_inspect_embedded_pixels_v79() {
    for linked in [false, true] {
        let source = if linked {
            SmartSource::Linked { path: "not-opened-for-panel-filter.psd".into() }
        } else {
            SmartSource::Embedded { file_name: "not-decoded.psd".into(), bytes: Arc::new(vec![0, 1, 2]) }
        };
        let layer = Layer::new("Smart", LayerContent::Smart(SmartObject::new(source, Default::default(), None)));
        for (value, want) in [("any", true), ("embedded", !linked), ("linked", linked), ("filters", false)] {
            assert_eq!(Options { mode: Mode::SmartObject, value: value.into(), ..Default::default() }.matches(&layer, &[], &[]), want);
        }
        assert!(Options::default().matches(&layer, &["smart".into()], &[]));
    }
    let document = base_document();
    let fill = Layer::new("Fill", LayerContent::Fill(photocraft_doc::Fill::Solid(photocraft_color::Color::BLACK)));
    assert_eq!(kind_of(&fill), "adjustment", "retain the existing five-kind filter contract");
    let group = Layer::group("Folder", vec![pixel(&document, "Child")]);
    assert!(!Options::default().matches(&group, &["pixel".into()], &[]));
}

#[test]
fn five_kind_icons_correspond_to_the_actual_content_variants_v79() {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width":32,"height":24,"background":"transparent"})).unwrap();
    for (command, params, expected) in [
        ("layer.new.layer", json!({}), "pixel"),
        ("layer.newAdjustmentLayer.curves", json!({}), "adjustment"),
        ("type.create", json!({"text":"Test","x":2,"y":12}), "type"),
        ("shape.create", json!({"kind":"rect","rect":[2,2,8,8],"fill":"#ff0000"}), "shape"),
    ] {
        session.execute(command, params).unwrap();
        let active = session.active().unwrap();
        let layer = active.doc.layer(active.active_layer.unwrap()).unwrap();
        assert_eq!(kind_of(layer), expected);
        assert!(Options::default().matches(layer, &[expected.into()], &[]));
        for other in ["pixel", "adjustment", "type", "shape", "smart"].into_iter().filter(|kind| *kind != expected) {
            assert!(!Options::default().matches(layer, &[other.into()], &[]));
        }
    }
    let mut artboard = Layer::group("Board", Vec::new());
    if let LayerContent::Group(group) = &mut artboard.content {
        group.artboard = Some(photocraft_doc::Artboard::new(photocraft_geom::Rect::new(0, 0, 32, 24)));
    }
    assert!(Options { mode: Mode::Artboard, ..Default::default() }.matches(&artboard, &[], &[]));
}

#[test]
fn matching_descendants_keep_closed_group_ancestors_in_stable_display_order_v79() {
    let mut doc = base_document();
    let child = pixel(&doc, "needle");
    let child_id = child.id;
    let sibling = pixel(&doc, "irrelevant");
    let mut inner = Layer::group("Inner", vec![child, sibling]);
    if let LayerContent::Group(group) = &mut inner.content {
        group.expanded = false;
    }
    let inner_id = inner.id;
    let mut outer = Layer::group("Outer", vec![inner]);
    if let LayerContent::Group(group) = &mut outer.content {
        group.expanded = false;
    }
    let outer_id = outer.id;
    doc.layers.push(outer);
    let before = doc.layers.clone();
    let rows = crate::layer_tree_ui::filtered_rows(&doc, |layer| layer.name == "needle");
    assert_eq!(rows.iter().map(|(depth, layer)| (*depth, layer.id)).collect::<Vec<_>>(), [(0, outer_id), (1, inner_id), (2, child_id)]);
    assert_eq!(doc.layers, before, "filtering does not expand folders in stored document data");
    assert_eq!(crate::layer_tree_ui::display_rows(&doc, false).len(), 2, "disable immediately restores original closed folders");
    assert!(crate::layer_tree_ui::filtered_rows(&doc, |_| false).is_empty());
}

#[test]
fn own_lock_mixed_state_and_type_support_match_engine_selection_contract_v79() {
    let mut doc = base_document();
    let mut a = pixel(&doc, "A");
    a.locks.transparency = true;
    let b = pixel(&doc, "B");
    let ids = [a.id, b.id];
    doc.layers = vec![a, b];
    assert_eq!(lock_state(&doc, &ids, "transparency"), LockState { applicable: 2, all_on: false, mixed: true, inherited: false });
    let mut group = Layer::group("Locked parent", std::mem::take(&mut doc.layers));
    group.locks.position = true;
    let group_id = group.id;
    doc.layers.push(group);
    let state = lock_state(&doc, &ids, "position");
    assert_eq!(state, LockState { applicable: 2, all_on: false, mixed: false, inherited: true }, "inherited lock is not copied into child's own checkbox");
    assert_eq!(lock_state(&doc, &[group_id], "pixels").applicable, 0);
    assert_eq!(lock_state(&doc, &[group_id], "transparency").applicable, 0);
    assert_eq!(lock_state(&doc, &[group_id], "artboard").applicable, 1);
}

#[test]
fn filter_options_round_trip_and_old_missing_fields_keep_kind_filter_enabled_v79() {
    assert_eq!(serde_json::from_value::<Options>(json!({})).unwrap(), Options::default());
    let options = Options { enabled: false, mode: Mode::Name, query: "中文 Layer".into(), value: "none".into() };
    assert_eq!(serde_json::from_value::<Options>(serde_json::to_value(&options).unwrap()).unwrap(), options);
}

#[test]
fn vector_pixel_locks_are_selected_but_not_editable_in_the_layers_bar_v79() {
    let mut doc = base_document();
    let text = Layer::new("Type", LayerContent::Text(photocraft_doc::TextLayer::default()));
    let text_id = text.id;
    let raster = pixel(&doc, "Pixels");
    let raster_id = raster.id;
    doc.layers = vec![text, raster];
    for key in ["pixels", "transparency"] {
        let fixed = lock_state(&doc, &[text_id], key);
        assert_eq!(fixed.applicable, 0, "vector pixel locks cannot be changed");
        assert!(fixed.all_on && !fixed.mixed, "fixed vector properties still display as selected");
        let mixed = lock_state(&doc, &[text_id, raster_id], key);
        assert_eq!(mixed.applicable, 1);
        assert!(mixed.mixed && !mixed.all_on, "a raster layer can still be changed in a mixed selection");
    }
}

#[test]
fn artboard_filter_shows_selected_board_contents_and_releases_outside_selection_v79() {
    let mut doc = base_document();
    let outside = doc.layers[0].id;
    let child = pixel(&doc, "Inside board");
    let child_id = child.id;
    let mut board = Layer::group("Board", vec![child]);
    if let LayerContent::Group(group) = &mut board.content {
        group.artboard = Some(photocraft_doc::Artboard::new(photocraft_geom::Rect::new(0, 0, 32, 24)));
        group.expanded = false;
    }
    let board_id = board.id;
    doc.layers.push(board);
    let options = Options { mode: Mode::Artboard, ..Default::default() };
    for selection in [vec![board_id], vec![child_id]] {
        let ids: Vec<_> = rows(&doc, &options, &[], &selection).into_iter().map(|(_, layer)| layer.id).collect();
        assert_eq!(ids, vec![board_id, child_id]);
    }
    for selection in [vec![], vec![outside], vec![outside, child_id]] {
        let ids: Vec<_> = rows(&doc, &options, &[], &selection).into_iter().map(|(_, layer)| layer.id).collect();
        assert_eq!(ids, vec![board_id, outside], "restores unfiltered disclosure state");
    }
    assert!(matches!(&doc.layer(board_id).unwrap().content, LayerContent::Group(group) if !group.expanded));
}
