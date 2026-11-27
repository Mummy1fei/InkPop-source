//! Atomic batch management of brush and tool libraries.
use super::*;
pub fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "presets.deleteBatch";
    let kind = req_str(p, "kind", CMD)?;
    let names: Vec<String> =
        serde_json::from_value(p.get("names").cloned().unwrap_or(Value::Null)).map_err(|_| bad(CMD, "names must be an array of strings"))?;
    if names.is_empty() || names.len() > 10000 {
        return Err(bad(CMD, "select 1..10000 presets"));
    }
    let names: std::collections::HashSet<String> = names.into_iter().collect();
    let known: std::collections::HashSet<&str> = match kind {
        "brushes" => s.tools.presets.iter().map(|p| p.name.as_str()).collect(),
        "tools" => s.presets.tool_presets.iter().map(|p| p.name.as_str()).collect(),
        _ => return Err(bad(CMD, "kind must be brushes or tools")),
    };
    if names.iter().any(|n| !known.contains(n.as_str())) {
        return Err(bad(CMD, "a selected preset no longer exists; refresh the list"));
    }
    if kind == "brushes" {
        s.tools.presets.retain(|p| !names.contains(&p.name));
        s.brush_presets_changed();
    } else {
        s.presets.tool_presets.retain(|p| !names.contains(&p.name));
        s.presets_changed();
    }
    Ok(json!({"deleted":names.len(),"kind":kind}))
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "presets.deleteBatch",
        label: "Delete Selected Presets",
        menu: &[],
        shortcut: None,
        params: r#"{"kind":"brushes"|"tools","names":[name,...]} validates the entire selection before deletion"#,
        enabled: |_| Ok(()),
        run: delete,
        journal: true,
    }]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batches_validate_before_mutation_and_keep_other_library() {
        let mut s = Session::new();
        let brushes = s.tools.presets.clone();
        let tools = s.presets.tool_presets.clone();
        assert!(s.execute("presets.deleteBatch", json!({"kind":"brushes","names":[brushes[0].name,"missing"]})).is_err());
        assert_eq!(s.tools.presets, brushes);
        s.execute("presets.deleteBatch", json!({"kind":"brushes","names":[brushes[0].name,brushes[1].name]})).unwrap();
        assert_eq!(s.tools.presets.len(), brushes.len() - 2);
        assert_eq!(s.presets.tool_presets, tools);
        s.execute("presets.deleteBatch", json!({"kind":"tools","names":[tools[0].name,tools[1].name]})).unwrap();
        assert_eq!(s.presets.tool_presets.len(), tools.len() - 2);
        for p in [json!({}), json!({"kind":"tools","names":[]}), json!({"kind":"bad","names":["x"]})] {
            assert!(s.execute("presets.deleteBatch", p).is_err());
        }
    }
}
