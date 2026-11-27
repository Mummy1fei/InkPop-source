use super::*;
use photocraft_color::SampleType;
use photocraft_doc::SmartFilter;

fn fixture(depth: u32, opaque: bool) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":16,"height":12,"depth":depth,"background":"transparent"})).unwrap();
    s.execute("edit.fill", json!({"color":"#305080"})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    if opaque {
        s.edit("Synthetic imported smart-filter appearance", |doc, active| {
            let sm = smart_mut(doc, active.ok_or(EngineError::NoDocument)?)?;
            sm.smart_filters.push(SmartFilter {
                command: UNSUPPORTED_FILTER.into(),
                params: json!({"name":"Camera Raw Filter","filterID":2783,"psd":"000102fe"}),
                blend: BlendMode::Normal,
                opacity: 1.0,
                visible: true,
            });
            sm.filter_mask = Some(LayerMask::reveal_all());
            sm.psd_raw = Some(Arc::new(vec![4, 3, 2, 1]));
            let cache = sm.cache.as_mut().ok_or_else(|| other("missing synthetic cache"))?;
            let pixel = photocraft_raster::from_rgba(&cache.format(), [0.8, 0.2, 0.1, 0.75]);
            cache.fill_rect(Rect::new(0, 0, 16, 12), &pixel);
            Ok(())
        })
        .unwrap();
    }
    s
}

fn current(s: &Session) -> &SmartObject {
    let st = s.active().unwrap();
    smart(&st.doc, st.active_layer.unwrap()).unwrap()
}

#[test]
fn opaque_cache_v78_filter_mutations_cannot_overwrite_preserved_appearance_or_history() {
    for depth in [8, 16, 32] {
        let mut s = fixture(depth, true);
        let original = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.past_len();
        let revision = s.active().unwrap().revision;
        let dirty = s.active().unwrap().is_dirty();
        for (command, params) in [
            ("layer.smartFilter.setVisible", json!({"index":0,"visible":false})),
            ("layer.smartFilter.disableSmartFilters", json!({"enabled":false})),
            ("layer.smartFilter.delete", json!({"index":0})),
            ("layer.smartFilter.clearSmartFilters", json!({})),
            ("layer.smartFilter.move", json!({"index":0,"to":0})),
            ("layer.smartFilter.blendingOptions", json!({"index":0,"opacity":0.5})),
            ("layer.smartFilter.setParams", json!({"index":0,"params":{"exposure":1.0}})),
            ("layer.smartFilter.disableFilterMask", json!({"enabled":false})),
            ("layer.smartFilter.deleteFilterMask", json!({})),
            ("filter.blur.gaussianBlur", json!({"radius":1.0})),
            ("layer.smartObjects.relinkToFile", json!({"path":"synthetic-unread-source.png"})),
        ] {
            let error = s.execute(command, params).unwrap_err().to_string();
            assert!(error.contains("preserved smart filter"), "{command}: {error}");
            let st = s.active().unwrap();
            assert!(Arc::ptr_eq(&st.doc, &original), "{depth}/{command}: source, cache, unknown bytes and params unchanged");
            assert_eq!(st.history.past_len(), history, "{command}: no failed-edit history");
            assert_eq!(st.revision, revision);
            assert_eq!(st.is_dirty(), dirty);
        }
    }
}

#[test]
fn opaque_cache_v78_central_render_and_refresh_never_publish_partial_stack_pixels() {
    let s = fixture(16, true);
    let st = s.active().unwrap();
    let sm = current(&s);
    let cache = sm.cache.as_ref().unwrap().clone();
    assert!(apply_smart_filters(&cache, sm, st.doc.bounds()).is_err());
    assert!(render(&st.doc, sm).is_err());
    let mut layer = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
    let before = layer.clone();
    assert!(refresh_layer(&st.doc, &mut layer).is_err());
    assert_eq!(layer, before);
    // A recognized command that is not an executable pixel filter must also fail,
    // rather than silently becoming a pass-through and replacing the cache.
    let mut invalid = sm.clone();
    invalid.smart_filters[0].command = "layer.new.layer".into();
    assert!(apply_smart_filters(&cache, &invalid, st.doc.bounds()).is_err());
}

#[test]
fn opaque_cache_v78_hidden_unknown_filter_can_stay_hidden_but_cannot_be_enabled() {
    let mut s = fixture(8, false);
    s.edit("Synthetic hidden preserved filter", |doc, active| {
        smart_mut(doc, active.ok_or(EngineError::NoDocument)?)?.smart_filters.push(SmartFilter {
            command: UNSUPPORTED_FILTER.into(),
            params: json!({"psd":"0123"}),
            blend: BlendMode::Normal,
            opacity: 1.0,
            visible: false,
        });
        Ok(())
    })
    .unwrap();
    let original = s.active().unwrap().doc.clone();
    assert!(s.execute("layer.smartFilter.setVisible", json!({"visible":true})).is_err());
    assert!(Arc::ptr_eq(&s.active().unwrap().doc, &original));
    // Hidden opaque entries do not block an otherwise replayable known stack.
    s.execute("filter.blur.gaussianBlur", json!({"radius":1.0})).unwrap();
    assert!(!current(&s).smart_filters[0].visible);
    assert_eq!(current(&s).smart_filters[0].params, json!({"psd":"0123"}));
}

#[test]
fn opaque_cache_v78_cache_only_translation_and_rasterization_remain_available() {
    for depth in [8, 16, 32] {
        let mut s = fixture(depth, true);
        let cache = current(&s).cache.as_ref().unwrap().clone();
        let params = current(&s).smart_filters.clone();
        s.execute("layer.translate", json!({"dx":3,"dy":-2})).unwrap();
        assert_eq!(current(&s).cache.as_ref().unwrap().rgba(5, 1), cache.rgba(2, 3));
        assert_eq!(current(&s).smart_filters, params);
        s.execute("layer.smartObjects.rasterize", json!({})).unwrap();
        let layer = s.active().unwrap().doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap();
        assert!(matches!(layer.content, LayerContent::Raster(_)));
        let actual = layer.surface().unwrap().rgba(5, 1);
        let expected = cache.rgba(2, 3);
        if depth == 32 {
            // Rasterization composites premultiplied float color once; division
            // back to straight RGB can differ by one float ULP.
            assert!(actual.iter().zip(expected).all(|(a, b)| (*a - b).abs() <= 2.0 * f32::EPSILON));
        } else {
            assert_eq!(actual, expected);
        }
    }
}

#[test]
fn opaque_cache_v78_known_stacks_still_toggle_losslessly_at_all_depths() {
    for (depth, sample) in [(8, SampleType::U8), (16, SampleType::U16), (32, SampleType::F32)] {
        let mut s = fixture(depth, false);
        let source = current(&s).cache.clone().unwrap();
        s.execute("filter.other.offset", json!({"horizontal":3,"vertical":0,"undefinedAreas":"transparent"})).unwrap();
        let filtered = current(&s).cache.clone().unwrap();
        assert_eq!(filtered.format().sample, sample);
        for command in ["layer.smartFilter.setVisible", "layer.smartFilter.disableSmartFilters"] {
            s.execute(command, json!({})).unwrap();
            assert_eq!(current(&s).cache.as_ref().unwrap(), &source);
            s.execute(command, json!({})).unwrap();
            assert_eq!(current(&s).cache.as_ref().unwrap(), &filtered);
        }
    }
}

#[test]
fn opaque_cache_v78_edit_contents_failure_preserves_parent_and_child_unsaved_state() {
    // The child was legitimately opened before another edit/import introduced
    // an opaque parent filter (the same condition as an older saved session).
    let mut s = fixture(16, false);
    let parent_layer = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.smartObjects.editContents", json!({})).unwrap();
    let child = s.active_index().unwrap();
    s.set_active(0);
    s.edit("Synthetic concurrent preserved filter", |doc, _| {
        smart_mut(doc, parent_layer)?.smart_filters.push(SmartFilter {
            command: UNSUPPORTED_FILTER.into(),
            params: json!({"psd":"0123"}),
            blend: BlendMode::Normal,
            opacity: 1.0,
            visible: true,
        });
        Ok(())
    })
    .unwrap();
    let parent = s.active().unwrap().doc.clone();
    let parent_history = s.active().unwrap().history.past_len();
    s.set_active(child);
    s.execute("edit.fill", json!({"color":"#40c080"})).unwrap();
    let child_doc = s.active().unwrap().doc.clone();
    let saved = s.active().unwrap().saved_revision;
    assert!(s.execute("layer.smartObjects.saveContents", json!({})).is_err());
    assert_eq!(s.active_index(), Some(child));
    assert!(Arc::ptr_eq(&s.documents()[0].doc, &parent));
    assert_eq!(s.documents()[0].history.past_len(), parent_history);
    assert!(Arc::ptr_eq(&s.active().unwrap().doc, &child_doc));
    assert_eq!(s.active().unwrap().saved_revision, saved);
    assert!(s.active().unwrap().is_dirty());
    for (command, params) in [("file.close", json!({})), ("file.closeAll", json!({})), ("file.closeOthers", json!({"document":0}))] {
        let error = s.execute(command, params).unwrap_err().to_string();
        assert!(error.contains("preserved smart filter"), "{command}: {error}");
        assert_eq!(s.documents().len(), 2);
        assert_eq!(s.active_index(), Some(child));
        assert_eq!(s.smart_links.len(), 1);
        assert!(Arc::ptr_eq(&s.documents()[0].doc, &parent));
        assert!(Arc::ptr_eq(&s.active().unwrap().doc, &child_doc));
        assert!(s.active().unwrap().is_dirty());
    }
    assert!(s.close(child).is_none(), "legacy close API also retains an uncommittable child");
    assert_eq!(s.documents().len(), 2);
}

#[test]
fn opaque_cache_v100_edit_contents_opens_an_independent_copy() {
    let mut s = fixture(8, true);
    let original = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    let revision = s.active().unwrap().revision;
    let result = s.execute("layer.smartObjects.editContents", json!({})).unwrap();
    assert_eq!(result["independentCopy"], true);
    assert_eq!(s.documents().len(), 2);
    assert_eq!(s.active_index(), Some(1));
    assert!(s.active().unwrap().path.is_none());
    assert!(s.smart_links.is_empty());
    s.execute("layer.new.layer", json!({"name": "Only in the copy"})).unwrap();
    assert!(!commit_child(&mut s, 1).unwrap());
    s.try_close(1).unwrap().unwrap();
    let st = s.active().unwrap();
    assert!(Arc::ptr_eq(&st.doc, &original));
    assert_eq!(st.history.past_len(), history);
    assert_eq!(st.revision, revision);

    // A replayable known stack still opens its layered source normally.
    let mut known = fixture(8, false);
    known.execute("filter.blur.gaussianBlur", json!({"radius":1.0})).unwrap();
    known.execute("layer.smartObjects.editContents", json!({})).unwrap();
    assert_eq!(known.documents().len(), 2);
    assert_eq!(known.smart_links.len(), 1);
}
