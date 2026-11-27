use crate::Session;
use photocraft_psd::descriptor::{Descriptor, Value, VersionedDescriptor};
use serde_json::json;

fn file() -> String {
    let tip = Descriptor::new("computedBrush")
        .with("Dmtr", Value::UnitFloat { unit: *b"#Pxl", value: 73.0 })
        .with("Hrdn", Value::UnitFloat { unit: *b"#Prc", value: 25.0 });
    let tool = Descriptor::new("PbTl").with("Brsh", Value::Descriptor(tip)).with("Opct", Value::Integer(42));
    descriptor_file(tool)
}

fn descriptor_file(tool: Descriptor) -> String {
    let mut body = 1u32.to_be_bytes().to_vec();
    let name: Vec<u16> = "水彩\0".encode_utf16().collect();
    body.extend((name.len() as u32).to_be_bytes());
    for c in name {
        body.extend(c.to_be_bytes());
    }
    body.extend(VersionedDescriptor::new(tool).to_bytes());
    let mut bytes = b"8BTP".to_vec();
    bytes.extend(3u32.to_be_bytes());
    bytes.extend(1u32.to_be_bytes());
    bytes.extend(b"8BIMtptp");
    bytes.extend((body.len() as u32).to_be_bytes());
    bytes.extend(body);
    photocraft_paint::tile::b64_encode(&bytes)
}

#[test]
fn tpl_smudge_selection_returns_imported_strength_without_resetting_other_options() {
    let mut s = Session::new();
    let tip = Descriptor::new("computedBrush").with("Dmtr", Value::Double(73.0));
    let tool = Descriptor::new("SmTl")
        .with("Brsh", Value::Descriptor(tip))
        .with("Prs ", Value::Integer(98))
        .with("SmdF", Value::Boolean(true))
        .with("SmdS", Value::Boolean(true));
    s.execute("tool.presets.importTpl", json!({"data":descriptor_file(tool),"group":"Smudge strength"})).unwrap();
    let selected = s.execute("tool.presets.select", json!({"preset":"水彩"})).unwrap();
    assert_eq!(selected["tool"], "smudge");
    assert_eq!(selected["options"]["toolOptions"], json!({"strength":98.0}));
    assert_eq!(s.tools.brush.size, 73.0);
    assert_eq!(s.tools.brush.flow, 1.0);
    // SmdF/SmdS behavior is still under PS validation. Do not silently change
    // finger/sampling state as part of the verified Strength import repair.
    assert_eq!(s.presets.tool_presets.last().unwrap().options["toolOptions"], json!({"strength":98.0}));
}

#[test]
fn tpl_import_replaces_its_group_and_selection_applies_settings() {
    let mut s = Session::new();
    let initial = s.presets.tool_presets.len();
    let params = json!({"data": file(), "group": "TPL Test"});
    for _ in 0..2 {
        let r = s.execute("tool.presets.importTpl", params.clone()).unwrap();
        assert_eq!(r["count"], 1);
        assert_eq!(s.presets.tool_presets.len(), initial + 1);
    }
    let selected = s.execute("tool.presets.select", json!({"preset": "水彩"})).unwrap();
    assert_eq!(selected["tool"], "brush");
    assert_eq!(s.tools.brush.size, 73.0);
    assert!((s.tools.brush.opacity - 0.42).abs() < 1e-6);
    let mut params = params;
    params["replace"] = json!(false);
    s.execute("tool.presets.importTpl", params).unwrap();
    assert_eq!(s.presets.tool_presets.len(), initial + 2);
    assert!(s.presets.tool_presets.iter().any(|p| p.name == "水彩 2"));
}

#[test]
fn invalid_tpl_leaves_library_unchanged() {
    let mut s = Session::new();
    let before = s.presets.tool_presets.clone();
    for params in [json!({}), json!({"data": "AAAA"}), json!({"data": file(), "group": ""})] {
        assert!(s.execute("tool.presets.importTpl", params).is_err());
        assert_eq!(s.presets.tool_presets, before);
    }
}

#[test]
fn replace_all_tpl_validates_before_replacing_and_does_not_keep_other_groups() {
    let mut s = Session::new();
    let before = s.presets.tool_presets.clone();
    for params in [json!({"data":"AAAA","replaceAll":true}), json!({"data":file(),"replaceAll":"yes"})] {
        assert!(s.execute("tool.presets.importTpl", params).is_err());
        assert_eq!(s.presets.tool_presets, before);
    }
    s.execute("tool.presets.importTpl", json!({"data":file(),"replaceAll":true})).unwrap();
    assert_eq!(s.presets.tool_presets.len(), 1);
    assert_eq!(s.presets.tool_presets[0].name, "水彩");
}
