//! Layer organisation labels. Labels change metadata, never pixel colours.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::LabelColor;
use serde_json::{Value, json};

pub fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let color = p.get("color").and_then(Value::as_str).ok_or_else(|| EngineError::Other("color must be a layer label name".into()))?;
    let label = match color.to_ascii_lowercase().as_str() {
        "none" => LabelColor::None,
        "red" => LabelColor::Red,
        "orange" => LabelColor::Orange,
        "yellow" => LabelColor::Yellow,
        "green" => LabelColor::Green,
        "blue" => LabelColor::Blue,
        "violet" | "purple" => LabelColor::Violet,
        "gray" | "grey" => LabelColor::Gray,
        _ => return Err(EngineError::Other("unknown layer label color".into())),
    };
    let ids = crate::layer_multi_cmds::selected(s);
    if ids.is_empty() {
        return Err(EngineError::NoDocument);
    }
    let changed = s.edit("Layer Color", |doc, _| {
        // Walk once, carrying the selected-group state to every descendant. Avoid recursive
        // calls and repeated whole-tree lookups, including for groups selected with children.
        let selected: std::collections::HashSet<_> = ids.iter().copied().collect();
        let mut found = std::collections::HashSet::new();
        let mut changed = Vec::new();
        let mut stack: Vec<_> = doc.layers.iter_mut().rev().map(|layer| (layer, false, 0)).collect();
        while let Some((layer, inherited, depth)) = stack.pop() {
            if depth > photocraft_doc::MAX_GROUP_DEPTH {
                return Err(EngineError::Other("layer tree exceeds the group nesting limit".into()));
            }
            let direct = selected.contains(&layer.id);
            if direct {
                found.insert(layer.id);
            }
            let apply = direct || inherited;
            if apply {
                layer.label = label;
                changed.push(layer.id);
            }
            if let Some(children) = layer.children_mut() {
                stack.extend(children.iter_mut().rev().map(|child| (child, apply, depth + 1)));
            }
        }
        if let Some(missing) = ids.iter().find(|id| !found.contains(id)) {
            return Err(EngineError::NoLayer(*missing));
        }
        Ok(changed)
    })?;
    Ok(json!({"color": color, "layers": changed.iter().map(|id| id.0).collect::<Vec<_>>()}))
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "layer.setLabel",
        label: "Layer Color",
        menu: &[],
        shortcut: None,
        params: r#"{"color":"none|red|orange|yellow|green|blue|violet|gray"}; selected layers and all descendants of selected groups"#,
        enabled: |s| if crate::layer_multi_cmds::selected(s).is_empty() { Err("no selected layers".into()) } else { Ok(()) },
        run,
        journal: true,
    }]
}
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::LayerId;

    #[test]
    fn group_label_reaches_nested_contents_and_undo_restores_individual_colors_without_pixel_changes() {
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":8,"height":8,"depth":depth})).unwrap();
            let bg = s.active().unwrap().active_layer.unwrap();
            let a = LayerId(s.execute("layer.new.layer", json!({"name":"A"})).unwrap()["layer"].as_u64().unwrap());
            s.execute("edit.fill", json!({"color":"#ed8020"})).unwrap();
            s.execute("layer.setLabel", json!({"color":"blue"})).unwrap();
            let inner = LayerId(s.execute("layer.groupLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
            let b = LayerId(s.execute("layer.new.layer", json!({"name":"B"})).unwrap()["layer"].as_u64().unwrap());
            s.execute("layer.setLabel", json!({"color":"yellow"})).unwrap();
            s.execute("layer.select", json!({"layer":inner.0,"mode":"add"})).unwrap();
            let outer = LayerId(s.execute("layer.groupLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
            s.execute("layer.select", json!({"layer":a.0,"mode":"add"})).unwrap();
            let pixels = photocraft_compose::flatten(&s.active().unwrap().doc).px;
            let past = s.active().unwrap().history.past_len();
            let result = s.execute("layer.setLabel", json!({"color":"red"})).unwrap();
            assert_eq!(result["layers"].as_array().unwrap().len(), 4, "selected descendant is only changed once");
            let doc = &s.active().unwrap().doc;
            for id in [outer, inner, a, b] {
                assert_eq!(doc.layer(id).unwrap().label, LabelColor::Red);
            }
            assert_eq!(doc.layer(bg).unwrap().label, LabelColor::None);
            assert_eq!(photocraft_compose::flatten(doc).px, pixels);
            assert_eq!(s.active().unwrap().history.past_len(), past + 1);
            s.undo();
            let doc = &s.active().unwrap().doc;
            assert_eq!(doc.layer(a).unwrap().label, LabelColor::Blue);
            assert_eq!(doc.layer(b).unwrap().label, LabelColor::Yellow);
            assert_eq!(doc.layer(inner).unwrap().label, LabelColor::None);
            assert_eq!(doc.layer(outer).unwrap().label, LabelColor::None);
            s.redo();
            s.execute("layer.select", json!({"layer":outer.0})).unwrap();
            s.execute("layer.setLabel", json!({"color":"none"})).unwrap();
            assert!(s.active().unwrap().doc.walk().iter().all(|(_, _, l)| l.label == LabelColor::None));
            assert_eq!(photocraft_compose::flatten(&s.active().unwrap().doc).px, pixels);
        }
    }

    #[test]
    fn labels_are_undoable_and_invalid_names_leave_metadata_unchanged() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":8,"height":8})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.setLabel", json!({"color":"red"})).unwrap();
        assert_eq!(s.active().unwrap().doc.layer(id).unwrap().label, LabelColor::Red);
        assert!(s.execute("layer.setLabel", json!({"color":"bad"})).is_err());
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.layer(id).unwrap().label, LabelColor::None);
        assert!(Session::new().execute("layer.setLabel", json!({"color":"red"})).is_err());
    }
}
