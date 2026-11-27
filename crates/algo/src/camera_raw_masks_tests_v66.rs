use super::*;

#[test]
fn multiple_reference_resolution_automatic_masks_fit_without_reducing_edges() {
    let groups = (0..3)
        .map(|i| LocalAdjustment {
            name: format!("Automatic mask {i}"),
            mask: MaskShape::Raster { width: 1024, height: 683, values: vec![127; 1024 * 683] },
            ..Default::default()
        })
        .collect::<Vec<_>>();
    assert!(validate(&groups).is_ok(), "subject, sky and background must coexist at reference resolution");
    let encoded = serde_json::to_value(&groups).unwrap();
    let loaded: Vec<LocalAdjustment> = serde_json::from_value(encoded).unwrap();
    assert_eq!(loaded, groups);
    assert!(validate(&loaded).is_ok());
}

#[test]
fn per_shape_and_aggregate_raster_budgets_are_enforced_across_groups() {
    let raster = || MaskShape::Raster { width: 1024, height: 1024, values: vec![0; 1024 * 1024] };
    let slots = MAX_RASTER_PIXELS / (1024 * 1024);
    let mut groups = (0..slots).map(|_| LocalAdjustment { mask: raster(), ..Default::default() }).collect::<Vec<_>>();
    assert!(validate(&groups).is_ok());
    groups.push(LocalAdjustment { mask: MaskShape::Raster { width: 1, height: 1, values: vec![1] }, ..Default::default() });
    assert!(validate(&groups).is_err(), "an additional byte cannot bypass the shared storage budget");
    for shape in [
        MaskShape::Raster { width: 1025, height: 1, values: vec![0; 1025] },
        MaskShape::Raster { width: 1, height: 1025, values: vec![0; 1025] },
        MaskShape::Raster { width: 1024, height: 1024, values: vec![0; 12] },
    ] {
        assert!(validate(&[LocalAdjustment { mask: shape, ..Default::default() }]).is_err());
    }
}
