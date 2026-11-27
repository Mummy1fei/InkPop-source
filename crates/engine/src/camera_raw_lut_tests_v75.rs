use super::*;
use photocraft_algo::camera_raw_lut::ColorLookup;
use std::sync::Arc;

#[test]
fn raw_lut_v75_depth_selection_smart_replay_and_preset_roundtrip() {
    let mut values = Vec::new();
    for b in 0..2 {
        for g in 0..2 {
            for r in 0..2 {
                values.extend([r, g, b].map(|v| 1.0 - v as f32));
            }
        }
    }
    let lut = ColorLookup { name: "Invert.cube".into(), enabled: true, size: 2, values: Arc::new(values), ..Default::default() };
    for depth in [8, 16, 32] {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":12,"height":8,"depth":depth})).unwrap();
        session.execute("layer.new.layer", json!({})).unwrap();
        session.execute("edit.fill", json!({"color":"#204080"})).unwrap();
        let get = |s: &Session, x| {
            let d = s.active().unwrap();
            d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, 0)
        };
        let original = get(&session, 0);
        session.execute("select.rect", json!({"x":0,"y":0,"width":6,"height":8})).unwrap();
        session.execute(RAW, json!({"colorLookup":lut})).unwrap();
        let processed = get(&session, 0);
        for k in 0..3 {
            assert!((processed[k] - (1.0 - original[k])).abs() < 0.005);
        }
        assert_eq!(get(&session, 10), original);
        assert_eq!(processed[3], original[3]);
        session.undo();
        session.execute("select.deselect", json!({})).unwrap();
        session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        session.execute(RAW, json!({"colorLookup":lut})).unwrap();
        let p = get(&session, 0);
        for k in 0..3 {
            assert!((p[k] - processed[k]).abs() < 0.005);
        }
        let params = CameraRaw { color_lookup: lut.clone(), ..Default::default() };
        session.execute("cameraRaw.presets.save", json!({"name":"LUT recipe","params":params})).unwrap();
        let value = session.execute("cameraRaw.presets.get", json!({"name":"LUT recipe"})).unwrap();
        let preset: crate::camera_raw_preset_cmds::RawPreset = serde_json::from_value(value).unwrap();
        let export = preset.export_value().unwrap();
        assert!(serde_json::to_vec(&export).unwrap().len() < crate::camera_raw_preset_cmds::MAX_PRESET_BYTES);
        assert_eq!(preset.params.color_lookup, lut);
        assert!(session.execute(RAW, json!({"colorLookup":{"size":2,"enabled":true,"values":""}})).is_err());
    }
}

#[test]
fn raw_lut_v75_full_grid_preset_fits_transport_and_roundtrips_with_mask() {
    let mut random = 0x12345678u32;
    let values: Vec<f32> = (0..33usize.pow(3) * 3)
        .map(|_| {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            f32::from_bits(0x3e800000 | (random & 0x007fffff))
        })
        .collect();
    let params = CameraRaw {
        color_lookup: ColorLookup { name: "Full grid.cube".into(), size: 33, enabled: true, values: Arc::new(values), ..Default::default() },
        local_adjustments: vec![photocraft_algo::camera_raw_masks::LocalAdjustment { exposure: 0.2, ..Default::default() }],
        ..Default::default()
    };
    let preset = crate::camera_raw_preset_cmds::RawPreset { name: "Full LUT and mask".into(), params: params.clone() };
    let value = preset.export_value().unwrap();
    let rpc = json!({"id":1,"method":"ui.menu.invoke","params":{"id":"filter.cameraRaw","params":{"ui":{"preset":{"action":"import","data":value}}}}});
    assert!(serde_json::to_vec(&rpc).unwrap().len() < 1_048_576);
    let mut s = Session::new();
    s.execute("cameraRaw.presets.save", json!({"name":preset.name,"params":preset.params})).unwrap();
    let loaded = s.execute("cameraRaw.presets.get", json!({"name":"Full LUT and mask"})).unwrap();
    let loaded: crate::camera_raw_preset_cmds::RawPreset = serde_json::from_value(loaded).unwrap();
    assert_eq!(loaded.params, params);
}
