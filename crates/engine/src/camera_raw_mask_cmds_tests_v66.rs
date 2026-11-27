use super::*;
use serde_json::json;

#[test]
fn draft_request_and_result_use_the_same_aggregate_decode_budget_as_camera_raw() {
    let group =
        serde_json::to_value(LocalAdjustment { mask: MaskShape::Raster { width: 1024, height: 1024, values: vec![0; 1024 * 1024] }, ..Default::default() })
            .unwrap();
    let oversized = vec![group.clone(); 17];
    let mut session = Session::new();
    assert!(run(&mut session, &json!({"items":oversized,"operation":{"kind":"deleteAll"}})).is_err());
    assert!(serde_json::from_value::<EditResult>(json!({"items":vec![group; 17],"oldToNew":[],"preferredSelection":null})).is_err());
    assert!(serde_json::from_value::<Request>(json!({"items":vec![json!({});65],"operation":{"kind":"deleteAll"}})).is_err());
}

#[test]
fn legal_reference_resolution_draft_roundtrips_through_the_bounded_parser() {
    let items = (0..3)
        .map(|i| LocalAdjustment {
            name: format!("Generated {i}"),
            mask: MaskShape::Raster { width: 1024, height: 683, values: vec![i as u8 * 127; 1024 * 683] },
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let mut session = Session::new();
    let result = run(&mut session, &json!({"items":items,"operation":{"kind":"rename","index":1,"name":"Renamed"}})).unwrap();
    let result: EditResult = serde_json::from_value(result).unwrap();
    assert_eq!(result.items.len(), 3);
    assert_eq!(result.items[1].name, "Renamed");
    assert_eq!(result.items[0], items[0]);
    assert_eq!(result.items[2], items[2]);
    assert_eq!(result.items[1].mask, items[1].mask);
}
