//! Atomic Layers-panel reordering. A selected group carries its descendants only once.

use photocraft_doc::LayerId;
use serde_json::{Value, json};

use crate::{EngineError, Result, Session};

fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: "layer.moveTo".into(), msg: message.into() }
}

pub(crate) fn move_to(s: &mut Session, p: &Value) -> Result<Value> {
    let copy = match p.get("copy") {
        None => false,
        Some(value) => value.as_bool().ok_or_else(|| bad("`copy` must be a boolean"))?,
    };
    let target = LayerId(p.get("target").and_then(Value::as_u64).ok_or_else(|| bad("missing `target`"))?);
    let position = match p.get("position") {
        None => "above",
        Some(v) => v.as_str().ok_or_else(|| bad("`position` must be a string"))?,
    };
    if !matches!(position, "above" | "below" | "into") {
        return Err(bad("`position` must be above, below or into"));
    }
    if p.get("layer").is_some() && p.get("layers").is_some() {
        return Err(bad("use either `layer` or `layers`"));
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let selection = if let Some(value) = p.get("layers") {
        let values = value.as_array().ok_or_else(|| bad("`layers` must be an array"))?;
        if values.is_empty() || values.len() > st.doc.walk().len() {
            return Err(bad("`layers` must contain existing layer ids"));
        }
        values.iter().map(|v| v.as_u64().map(LayerId).ok_or_else(|| bad("invalid layer id"))).collect::<Result<Vec<_>>>()?
    } else if let Some(value) = p.get("layer") {
        vec![LayerId(value.as_u64().ok_or_else(|| bad("invalid layer id"))?)]
    } else {
        st.selected_layers()
    };
    if selection.is_empty() {
        return Err(bad("no layers selected"));
    }
    for id in selection.iter().chain(std::iter::once(&target)) {
        if st.doc.layer(*id).is_none() {
            return Err(EngineError::NoLayer(*id));
        }
    }
    let roots = crate::layer_multi_cmds::top_level(&st.doc, &selection);
    if roots.contains(&target) && !copy {
        return Ok(Value::Null);
    }
    if roots.contains(&target) && position == "into" {
        return Err(bad("can't copy a group into itself"));
    }
    let target_path = st.doc.path_of(target).ok_or(EngineError::NoLayer(target))?;
    for root in &roots {
        let path = st.doc.path_of(*root).ok_or(EngineError::NoLayer(*root))?;
        if target_path.starts_with(&path) && target != *root {
            return Err(bad("can't move a group into itself or its descendants"));
        }
    }
    if position == "into" && !st.doc.layer(target).is_some_and(|l| l.is_group()) {
        return Err(bad("target is not a group"));
    }
    if !copy {
        crate::layer_multi_cmds::ensure_structure_unlocked(&st.doc, &roots)?;
    }
    // Inserting into a locked destination is a structural change to that group.
    // A locked reference sibling, by itself, does not lock its whole container.
    let destination =
        if position == "into" { Some(target) } else { target_path.split_last().and_then(|(_, parent)| st.doc.layer_at(parent)).map(|layer| layer.id) };
    if let Some(parent) = destination
        && st.doc.effective_locks(parent).all
    {
        return Err(EngineError::Other("the destination group is locked".into()));
    }
    let mut active = st.active_layer.filter(|id| selection.contains(id)).or_else(|| selection.last().copied());
    // Prepare and validate the complete tree before touching history, including the depth cap.
    // A drag's undo should restore the selected block as it stood just before the drag, even
    // if the last edit originally targeted a different layer.
    let mut reordered = (*st.doc).clone();
    let mut result_layers = roots.clone();
    {
        let doc = &mut reordered;
        let mut moved = Vec::with_capacity(roots.len());
        if copy {
            result_layers.clear();
            let old_active = active;
            active = None;
            for id in &roots {
                let source = doc.layer(*id).ok_or(EngineError::NoLayer(*id))?;
                let background = source.name == "Background" && source.locks.transparency && doc.layers.first().is_some_and(|layer| layer.id == *id);
                let mut duplicate = source.duplicate();
                duplicate.name = format!("{} copy", duplicate.name);
                if background {
                    duplicate.locks = Default::default();
                }
                if old_active == Some(*id) {
                    active = Some(duplicate.id);
                }
                result_layers.push(duplicate.id);
                moved.push(duplicate);
            }
            active = active.or_else(|| result_layers.last().copied());
        } else {
            for id in &roots {
                moved.push(doc.remove(*id).ok_or(EngineError::NoLayer(*id))?);
            }
        }
        // Resolve the destination after removal: source rows may precede it in the same stack.
        let path = doc.path_of(target).ok_or(EngineError::NoLayer(target))?;
        if position == "into" {
            let children = doc.layer_at_mut(&path).and_then(|l| l.children_mut()).ok_or_else(|| bad("target is not a group"))?;
            children.extend(moved);
        } else {
            let (&last, parent) = path.split_last().ok_or(EngineError::NoLayer(target))?;
            let siblings = if parent.is_empty() {
                &mut doc.layers
            } else {
                doc.layer_at_mut(parent).and_then(|l| l.children_mut()).ok_or_else(|| bad("target's parent is not a group"))?
            };
            let at = if position == "below" { last } else { last.saturating_add(1) }.min(siblings.len());
            siblings.splice(at..at, moved);
        }
        crate::layer_multi_cmds::check_group_depth(doc, "Reorder Layers")?;
    }
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let before_target = st.layer_target();
    st.history.set_current_layers(before_target);
    s.edit(if copy { "Duplicate Layers" } else { "Reorder Layers" }, |doc, active_layer| {
        *doc = reordered;
        *active_layer = active;
        Ok(())
    })?;
    crate::layer_multi_cmds::reselect(s, if copy { result_layers.clone() } else { selection }, active);
    Ok(json!({"layers": result_layers.iter().map(|id| id.0).collect::<Vec<_>>()}))
}

#[cfg(test)]
#[path = "layer_alt_drag_tests_v75.rs"]
mod alt_drag_tests_v75;

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Session, [LayerId; 4]) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        let mut ids = Vec::new();
        for name in ["A", "B", "C", "D"] {
            ids.push(LayerId(s.execute("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap()));
        }
        (s, ids.try_into().unwrap())
    }

    fn order(s: &Session) -> Vec<String> {
        s.active().unwrap().doc.layers.iter().map(|l| l.name.clone()).collect()
    }

    #[test]
    fn selected_rows_move_as_one_ordered_block_and_undo_restores_selection() {
        let (mut s, [a, b, c, d]) = fixture();
        crate::layer_multi_cmds::set_selection(&mut s, vec![c, a], Some(a), Some(a)).unwrap();
        let before = order(&s);
        let selected = s.active().unwrap().selected_layers();
        let past = s.active().unwrap().history.past_len();
        s.execute("layer.moveTo", json!({"target": d.0, "position": "above"})).unwrap();
        assert_eq!(order(&s), ["Background", "B", "D", "A", "C"]);
        assert_eq!(s.active().unwrap().selected_layers(), vec![a, c]);
        assert_eq!(s.active().unwrap().active_layer, Some(a));
        assert_eq!(s.active().unwrap().history.past_len(), past + 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(order(&s), before);
        assert_eq!(s.active().unwrap().selected_layers(), selected);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(order(&s), ["Background", "B", "D", "A", "C"]);
        s.execute("layer.moveTo", json!({"target": b.0, "position": "below"})).unwrap();
        assert_eq!(order(&s), ["Background", "A", "C", "B", "D"]);
    }

    #[test]
    fn cross_parent_rows_nest_together_and_group_descendants_are_not_moved_twice() {
        let (mut s, [a, b, c, d]) = fixture();
        let group = LayerId(s.execute("layer.new.group", json!({"name": "G", "empty": true})).unwrap()["layer"].as_u64().unwrap());
        s.execute("layer.moveTo", json!({"layers": [b.0, a.0], "target": group.0, "position": "into"})).unwrap();
        let children = |s: &Session| s.active().unwrap().doc.layer(group).unwrap().children().unwrap().iter().map(|l| l.id).collect::<Vec<_>>();
        assert_eq!(children(&s), vec![a, b]);
        s.execute("layer.moveTo", json!({"layers": [c.0, b.0], "target": d.0, "position": "below"})).unwrap();
        assert_eq!(children(&s), vec![a]);
        assert_eq!(order(&s), ["Background", "C", "B", "D", "G"]);
        // Doc traversal places C below G's child B, regardless of supplied array order.
        s.execute("layer.moveTo", json!({"layers": [a.0, group.0], "target": c.0, "position": "below"})).unwrap();
        assert_eq!(children(&s), vec![a]);
        assert_eq!(order(&s), ["Background", "G", "C", "B", "D"]);
    }

    #[test]
    fn invalid_multi_moves_and_cycles_leave_document_and_history_untouched() {
        let (mut s, [a, b, _, d]) = fixture();
        let group = s.execute("layer.new.group", json!({"name": "G", "empty": true})).unwrap()["layer"].as_u64().unwrap();
        s.execute("layer.moveTo", json!({"layer": a.0, "target": group, "position": "into"})).unwrap();
        let before = s.execute("document.inspect", json!({})).unwrap();
        let past = s.active().unwrap().history.past_len();
        for params in [
            json!({"layers": [b.0, group], "target": a.0}),
            json!({"layers": [b.0, 999999], "target": d.0}),
            json!({"layers": [b.0], "target": d.0, "position": "into"}),
            json!({"layers": [], "target": d.0}),
            json!({"layers": "wrong", "target": d.0}),
            json!({"layers": [b.0], "target": d.0, "position": "invalid"}),
            json!({"layer": b.0, "layers": [b.0], "target": d.0}),
        ] {
            assert!(s.execute("layer.moveTo", params.clone()).is_err(), "{params}");
            assert_eq!(s.execute("document.inspect", json!({})).unwrap(), before);
            assert_eq!(s.active().unwrap().history.past_len(), past);
        }
    }

    #[test]
    fn multi_move_preserves_raster_data_at_every_depth() {
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 8, "height": 8, "depth": depth})).unwrap();
            let target = s.active().unwrap().active_layer.unwrap();
            let mut ids = Vec::new();
            for color in ["#ff3300", "#0088ff"] {
                let id = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
                s.execute("edit.fill", json!({"color": color})).unwrap();
                ids.push(id);
            }
            let before = ids.iter().map(|id| s.active().unwrap().doc.layer(*id).unwrap().surface().unwrap().clone()).collect::<Vec<_>>();
            s.execute("layer.moveTo", json!({"layers": ids.iter().map(|id| id.0).collect::<Vec<_>>(), "target": target.0, "position": "below"})).unwrap();
            for (id, old) in ids.iter().zip(before) {
                let new = s.active().unwrap().doc.layer(*id).unwrap().surface().unwrap();
                assert_eq!(&old, new, "{depth}-bit pixels unchanged");
            }
        }
    }
}
