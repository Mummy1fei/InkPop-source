//! Insertion policy for newly authored layers. Copies and imports use the document's
//! metadata-preserving insertion API instead.
use photocraft_doc::{Document, LabelColor, Layer, LayerContent, LayerId};

/// Create inside an expanded active group, or above the active sibling. The new layer
/// receives its actual direct parent's organisation label, including `None`.
pub fn insert(doc: &mut Document, active: Option<LayerId>, mut layer: Layer) -> crate::Result<LayerId> {
    if let Some(id) = active
        && doc.layer(id).is_some_and(|l| matches!(&l.content, LayerContent::Group(g) if g.expanded))
        && doc.path_of(id).is_some_and(|p| p.len() > photocraft_doc::MAX_GROUP_DEPTH)
    {
        return Err(crate::EngineError::Other(format!("New layer would nest layers deeper than {} groups", photocraft_doc::MAX_GROUP_DEPTH)));
    }
    if let Some(group) = active.and_then(|id| doc.layer_mut(id)) {
        let label = group.label;
        if let LayerContent::Group(g) = &mut group.content
            && g.expanded
        {
            layer.label = label;
            let id = layer.id;
            g.children.push(layer);
            return Ok(id);
        }
    }
    Ok(insert_sibling(doc, active, layer))
}

/// New containers around existing layers retain their sibling insertion position.
/// Existing children retain their own labels; only the newly created container inherits.
pub(crate) fn insert_sibling(doc: &mut Document, above: Option<LayerId>, mut layer: Layer) -> LayerId {
    layer.label = above
        .and_then(|id| doc.path_of(id))
        .and_then(|path| path.split_last().and_then(|(_, parent)| doc.layer_at(parent)).map(|parent| parent.label))
        .unwrap_or(LabelColor::None);
    doc.insert_above(above, layer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;
    use serde_json::json;

    fn setup(depth: u32) -> (Session, LayerId, LayerId, LayerId) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":16,"height":16,"depth":depth})).unwrap();
        let ids = s
            .edit("Fixture", |doc, active| {
                let mut child = Layer::raster("Existing", doc.pixel_format());
                child.label = LabelColor::Blue;
                let child_id = child.id;
                let mut inner = Layer::group("Inner", vec![child]);
                inner.label = LabelColor::Red;
                let inner_id = inner.id;
                let mut outer = Layer::group("Outer", vec![inner]);
                outer.label = LabelColor::Green;
                let outer_id = outer.id;
                doc.layers.push(outer);
                *active = Some(child_id);
                Ok((outer_id, inner_id, child_id))
            })
            .unwrap();
        (s, ids.0, ids.1, ids.2)
    }

    #[test]
    fn authored_layer_kinds_inherit_direct_parent_at_all_depths_with_single_step_undo() {
        for depth in [8, 16, 32] {
            let cases = [
                ("layer.new.layer", json!({})),
                ("layer.new.group", json!({"empty":true})),
                ("layer.newFillLayer.solidColor", json!({"color":"#cc8020"})),
                ("layer.newFillLayer.gradient", json!({})),
                ("layer.newFillLayer.pattern", json!({"pattern":"Checkerboard"})),
                ("layer.newAdjustmentLayer.invert", json!({})),
                ("gradient.fill.create", json!({"from":[0,0],"to":[16,16]})),
                ("type.create", json!({"text":"Label","x":0,"y":0})),
                ("shape.create", json!({"kind":"rect","rect":[0,0,4,4]})),
            ];
            for (cmd, params) in cases {
                let (mut s, _, inner, existing) = setup(depth);
                let past = s.active().unwrap().history.past_len();
                let result = s.execute(cmd, params).unwrap();
                let id = LayerId(result["layer"].as_u64().unwrap());
                let d = &s.active().unwrap().doc;
                assert_eq!(d.layer(id).unwrap().label, LabelColor::Red, "{cmd}, {depth}");
                assert!(d.layer(inner).unwrap().children().unwrap().iter().any(|l| l.id == id));
                assert_eq!(d.layer(existing).unwrap().label, LabelColor::Blue);
                assert_eq!(s.active().unwrap().history.past_len(), past + 1);
                assert!(s.undo());
                assert!(s.active().unwrap().doc.layer(id).is_none());
                assert!(s.redo());
                assert_eq!(s.active().unwrap().doc.layer(id).unwrap().label, LabelColor::Red);
            }
        }
    }

    #[test]
    fn active_expanded_folder_inserts_inside_but_collapsed_folder_inserts_beside() {
        let (mut s, outer, inner, _) = setup(8);
        s.select_layer(inner).unwrap();
        let id = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
        let d = &s.active().unwrap().doc;
        assert_eq!(d.layer(id).unwrap().label, LabelColor::Red);
        assert!(d.layer(inner).unwrap().children().unwrap().iter().any(|l| l.id == id));
        s.edit("Collapse", |doc, active| {
            if let LayerContent::Group(g) = &mut doc.layer_mut(inner).unwrap().content {
                g.expanded = false;
            }
            *active = Some(inner);
            Ok(())
        })
        .unwrap();
        let id = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
        let d = &s.active().unwrap().doc;
        assert_eq!(d.layer(id).unwrap().label, LabelColor::Green);
        assert!(d.layer(outer).unwrap().children().unwrap().iter().any(|l| l.id == id));
    }

    #[test]
    fn unlabeled_direct_parent_does_not_inherit_grandparent_or_active_sibling_color() {
        let (mut s, _, inner, _) = setup(16);
        s.edit("Clear parent only", |doc, _| {
            doc.layer_mut(inner).unwrap().label = LabelColor::None;
            Ok(())
        })
        .unwrap();
        let pixels = photocraft_compose::flatten(&s.active().unwrap().doc).px;
        let id = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
        assert_eq!(s.active().unwrap().doc.layer(id).unwrap().label, LabelColor::None);
        assert_eq!(photocraft_compose::flatten(&s.active().unwrap().doc).px, pixels);
        let root = s.active().unwrap().doc.layers[0].id;
        s.select_layer(root).unwrap();
        let id = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
        assert_eq!(s.active().unwrap().doc.layer(id).unwrap().label, LabelColor::None);
    }

    #[test]
    fn duplicated_and_imported_layers_keep_their_own_labels() {
        let (mut s, _, _, child) = setup(32);
        let result = s.execute("layer.duplicate", json!({"layer":child.0})).unwrap();
        let id = LayerId(result["layer"].as_u64().unwrap());
        assert_eq!(s.active().unwrap().doc.layer(id).unwrap().label, LabelColor::Blue);
        s.edit("Import fixture", |doc, _| {
            for label in [LabelColor::Yellow, LabelColor::None] {
                let mut imported = Layer::raster("Imported", doc.pixel_format());
                imported.label = label;
                let id = doc.insert_above(Some(child), imported);
                assert_eq!(doc.layer(id).unwrap().label, label);
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn creating_inside_deepest_folder_keeps_group_depth_limit_atomic() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":4,"height":4})).unwrap();
        let group = s
            .edit("Deep fixture", |doc, active| {
                let mut nested = Layer::group("Deepest", vec![]);
                nested.label = LabelColor::Violet;
                let id = nested.id;
                for _ in 1..photocraft_doc::MAX_GROUP_DEPTH {
                    nested = Layer::group("Outer", vec![nested]);
                }
                doc.layers.push(nested);
                *active = Some(id);
                Ok(id)
            })
            .unwrap();
        // The empty folder is at depth MAX - 1: one child at depth MAX is valid.
        let deepest = LayerId(s.execute("layer.new.group", json!({"empty":true})).unwrap()["layer"].as_u64().unwrap());
        assert_eq!(s.active().unwrap().doc.max_group_depth(), photocraft_doc::MAX_GROUP_DEPTH);
        let past = s.active().unwrap().history.past_len();
        let count = s.active().unwrap().doc.layer_count();
        assert!(s.execute("layer.new.group", json!({"empty":true})).is_err());
        assert!(s.execute("layer.new.layer", json!({})).is_err());
        assert_eq!(s.active().unwrap().history.past_len(), past);
        assert_eq!(s.active().unwrap().doc.layer_count(), count);
        assert_eq!(s.active().unwrap().active_layer, Some(deepest));
        s.select_layer(group).unwrap();
        let id = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
        assert_eq!(s.active().unwrap().doc.layer(id).unwrap().label, LabelColor::Violet);
        assert_eq!(s.active().unwrap().doc.max_group_depth(), photocraft_doc::MAX_GROUP_DEPTH);
    }
}
