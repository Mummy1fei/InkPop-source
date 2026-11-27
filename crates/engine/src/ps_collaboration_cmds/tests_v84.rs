use super::*;
use photocraft_color::{BlendMode, ColorMode, SampleType};
use photocraft_doc::{Group, LabelColor, LayerMask};
use photocraft_geom::{Affine, Rect, Size};
use photocraft_raster::Surface;
use std::sync::Arc;

fn raw(imported: bool) -> SmartFilter {
    let local = SmartFilter { command: "filter.cameraRaw".into(), params: json!({"exposure":0.5}), blend: BlendMode::Normal, opacity: 0.7, visible: true };
    if imported {
        let item = photocraft_io::smart_map::item_for_filter(&local).unwrap();
        let imported = photocraft_io::smart_map::filter_from_item(&item);
        assert_eq!(imported.command, "filter.cameraRaw");
        assert!(imported.params.get(RAW_TEMPLATE_KEY).is_some());
        imported
    } else {
        local
    }
}

fn fixture(mode: ColorMode, depth: SampleType, imported: bool) -> (Document, LayerId) {
    let mut doc = Document::new("Collaboration original", Size::new(12, 9), mode, depth);
    let mut cache = Surface::new(doc.pixel_format());
    let pixel = photocraft_raster::from_rgba(&cache.format(), [0.8, 0.2, 0.4, 0.7]);
    cache.fill_rect(Rect::new(-3, -2, 14, 11), &pixel);
    let source = Document::new("Embedded source", Size::new(12, 9), mode, depth);
    let mut smart = SmartObject::new(
        SmartSource::Embedded { file_name: "source.pcraft".into(), bytes: Arc::new(crate::smart_cmds::encode_source(&source).unwrap()) },
        Affine::IDENTITY,
        Some(cache),
    );
    smart.smart_filters = vec![
        raw(imported),
        SmartFilter { command: "filter.blur.gaussianBlur".into(), params: json!({"radius":1}), blend: BlendMode::Multiply, opacity: 0.4, visible: false },
    ];
    smart.filter_mask = Some(LayerMask::reveal_all());
    let mut layer = Layer::new("Raw layer", LayerContent::Smart(smart));
    layer.opacity = 0.6;
    layer.fill_opacity = 0.75;
    layer.blend = BlendMode::Screen;
    layer.mask = Some(LayerMask::reveal_all());
    layer.clipped = true;
    layer.label = LabelColor::Red;
    layer.link_group = Some(42);
    layer.excluded_channels = 1;
    for key in [*b"SoLd", *b"PlLd", *b"SoLE", *b"InSF", *b"FEid", *b"FXid"] {
        layer.psd_blocks.push((key, Arc::new(vec![1, 2, 3])));
    }
    layer.psd_blocks.push((*b"test", Arc::new(vec![6, 7, 8])));
    let id = layer.id;
    let mut base = Surface::new(doc.pixel_format());
    let base_pixel = photocraft_raster::from_rgba(&base.format(), [0.15, 0.35, 0.6, 1.0]);
    base.fill_rect(doc.bounds(), &base_pixel);
    let base = Layer::new("Clipping base", LayerContent::Raster(base));
    doc.layers.push(Layer::new("Nested group", LayerContent::Group(Group { children: vec![base, layer], expanded: true, artboard: None })));
    (doc, id)
}

fn session(imported: bool) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":12,"height":9,"background":"transparent"})).unwrap();
    s.execute("edit.fill", json!({"color":"#305080"})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("Synthetic Raw cache", |doc, _| {
        let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!() };
        sm.smart_filters = vec![raw(imported)];
        sm.cache.as_mut().unwrap().fill_rect(Rect::new(0, 0, 12, 9), &[0.8, 0.2, 0.4, 0.7]);
        Ok(())
    })
    .unwrap();
    (s, id)
}

#[test]
fn collaboration_v84_cache_bake_preserves_all_outer_properties_and_depth_models() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in SampleType::ALL {
            let (doc, id) = fixture(mode, depth, false);
            let before = doc.clone();
            let (copy, report) = prepare(&doc).unwrap();
            assert_eq!(doc, before);
            assert_eq!(report.raw_filter_count, 1);
            assert_eq!(report.baked_layer_count, 1);
            assert_eq!(report.layers[0].layer, id.0);
            let old = doc.layer(id).unwrap();
            let LayerContent::Smart(sm) = &old.content else { panic!() };
            let mut expected = old.clone();
            expected.content = LayerContent::Raster(sm.cache.clone().unwrap());
            expected.psd_blocks.retain(|(key, _)| key == b"test");
            assert_eq!(copy.layer(id).unwrap(), &expected);
            assert_eq!(photocraft_compose::flatten(&copy), photocraft_compose::flatten(&doc));
        }
    }
}

#[test]
fn collaboration_v84_missing_cache_fails_even_for_disabled_or_hidden_raw() {
    let (mut doc, id) = fixture(ColorMode::Rgb, SampleType::U16, false);
    let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!() };
    sm.cache = None;
    sm.smart_filters[0].visible = false;
    sm.filters_enabled = false;
    assert!(analyze(&doc).unwrap().warnings.iter().any(|w| w.contains("no rendered cache")));
    assert!(prepare(&doc).unwrap_err().to_string().contains("missing displayed"));
}

#[test]
fn collaboration_v84_encoded_copy_is_raster_and_native_keeps_recipe() {
    for depth in SampleType::ALL {
        let (doc, id) = fixture(ColorMode::Rgb, depth, false);
        let before = doc.clone();
        let encoded = encode_copy(&doc, "new.psd", true).unwrap();
        assert_eq!(doc, before);
        let native = photocraft_format::load_from_bytes(encoded.master.as_ref().unwrap()).unwrap();
        assert_eq!(native.layer(id).unwrap(), doc.layer(id).unwrap());
        let psd = photocraft_io::import("copy.psd", &encoded.psd).unwrap().document;
        let copied = psd.walk().into_iter().map(|(_, _, l)| l).find(|l| l.name == "Raw layer").unwrap();
        assert!(matches!(copied.content, LayerContent::Raster(_)));
        assert!(!copied.psd_blocks.iter().any(|(key, _)| matches!(key, b"SoLd" | b"SoLE" | b"PlLd" | b"InSF" | b"FEid" | b"FXid")));
        assert_eq!(encoded.report.baked_layer_count, 1);
        assert!(encode_copy(&doc, "new.psb", false).unwrap().master.is_none());
    }
}

#[test]
fn collaboration_v84_raw_linked_cache_export_never_opens_source() {
    let (mut doc, id) = fixture(ColorMode::Rgb, SampleType::U8, false);
    let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!() };
    sm.source = SmartSource::Linked { path: "Z:/does-not-exist/never-read-private-file.png".into() };
    let encoded = encode_copy(&doc, "copy.psd", true).unwrap();
    assert!(encoded.warnings.iter().any(|w| w.contains("not fully self-contained")));
    let native = photocraft_format::load_from_bytes(encoded.master.as_ref().unwrap()).unwrap();
    let LayerContent::Smart(sm) = &native.layer(id).unwrap().content else { panic!() };
    assert!(matches!(sm.source, SmartSource::Linked { .. }));
    let mut other = doc.layer(id).unwrap().clone();
    other.id = LayerId::fresh();
    let LayerContent::Smart(sm) = &mut other.content else { panic!() };
    sm.smart_filters.clear();
    sm.psd_raw = None;
    other.psd_blocks.clear();
    doc.layers.push(other);
    assert!(encode_copy(&doc, "copy.psd", true).unwrap_err().to_string().contains("external linked source"));
}

#[test]
fn collaboration_v84_opaque_raw_bakes_and_psd_writer_prunes_orphan_source_and_filter_globals() {
    for filter_id in [2621, 2783] {
        opaque_copy_case(filter_id);
    }
    for filter_id in [-1, 0, 2620, 2622, 2782, 2784, 99999] {
        let filter = SmartFilter {
            command: photocraft_io::smart_map::UNSUPPORTED_FILTER.into(),
            params: json!({"filterID":filter_id}),
            blend: BlendMode::Normal,
            opacity: 1.0,
            visible: true,
        };
        assert!(!is_raw_filter(&filter), "filter id {filter_id} is not a verified Camera Raw identifier");
    }
}

fn opaque_copy_case(filter_id: i32) {
    use photocraft_psd::filter_effects::{FilterEffects, FilterEffectsItem};
    let (mut doc, id) = fixture(ColorMode::Rgb, SampleType::U8, false);
    let mut item = photocraft_io::smart_map::item_for_filter(&raw(false)).unwrap();
    for (key, value) in &mut item.items {
        if key.is("filterID") {
            *value = photocraft_psd::descriptor::Value::Integer(filter_id);
        }
    }
    let hex: String = item.to_bytes().iter().map(|byte| format!("{byte:02x}")).collect();
    let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!() };
    sm.smart_filters = vec![SmartFilter {
        command: photocraft_io::smart_map::UNSUPPORTED_FILTER.into(),
        params: json!({"name":"Camera Raw Filter","filterID":filter_id,"psd":hex}),
        blend: BlendMode::Normal,
        opacity: 1.0,
        visible: true,
    }];
    let source = photocraft_io::linked::LinkedFile { uuid: "raw-source-orphan-v84".into(), file_name: "source.pcraft".into(), bytes: vec![1, 2, 3] };
    let linked = photocraft_io::linked::encode_linked_file(&source);
    doc.metadata.psd_global_blocks.push((*b"8BIM", *b"lnk2", Arc::new(linked)));
    let fx = FilterEffects {
        version: 3,
        items: vec![FilterEffectsItem {
            id: "raw-cache-orphan-v84".into(),
            version: 1,
            rect: photocraft_psd::layer::Rect { top: 0, left: 0, bottom: 1, right: 1 },
            depth: 8,
            max_channels: 0,
            slots: vec![None, None],
            mask: None,
        }],
    };
    doc.metadata.psd_global_blocks.push((*b"8BIM", *b"FEid", Arc::new(fx.to_bytes())));
    let report = analyze(&doc).unwrap();
    assert_eq!(report.raw_filter_count, 1);
    assert!(!report.layers[0].requires_recompute_consent);
    assert!(!report.layers[0].filters[0].standard_parameters);
    assert!(matches!(prepare(&doc).unwrap().0.layer(id).unwrap().content, LayerContent::Raster(_)));
    let copy = encode_copy(&doc, "opaque.psd", false).unwrap();
    let reopened = photocraft_io::import("opaque.psd", &copy.psd).unwrap().document;
    assert!(!reopened.metadata.psd_global_blocks.iter().any(|(_, key, _)| matches!(key, b"lnk2" | b"FEid" | b"FXid")));
    assert!(!reopened.walk().into_iter().any(|(_, _, layer)| matches!(layer.content, LayerContent::Smart(_))));
}

#[test]
fn collaboration_v84_export_params_are_strict_and_default_companion_is_named() {
    assert_eq!(export_paths(&json!({"path":"folder/画.psd"})).unwrap(), ("folder/画.psd".into(), Some("folder/画.inkpop.pcraft".into())));
    assert_eq!(export_paths(&json!({"path":"copy.PSB","masterPath":false})).unwrap().1, None);
    assert_eq!(export_paths(&json!({"path":"copy.psd","masterPath":null})).unwrap().1, None);
    for params in [
        json!({}),
        json!({"path":12}),
        json!({"path":""}),
        json!({"path":"a.png"}),
        json!({"path":"a.psd\0"}),
        json!({"path":"a.psd","masterPath":true}),
        json!({"path":"a.psd","masterPath":"b.psd"}),
    ] {
        assert!(export_paths(&params).is_err(), "{params}");
    }
}

#[test]
fn collaboration_v84_imported_raw_replay_is_atomic_until_explicit_undoable_acceptance() {
    let (mut s, id) = session(true);
    for hidden in [false, true] {
        if hidden {
            s.edit("Synthetic disabled Raw", |doc, _| {
                let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!() };
                sm.filters_enabled = false;
                sm.smart_filters[0].visible = false;
                Ok(())
            })
            .unwrap();
        }
        let before = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        let history = s.active().unwrap().history.past_len();
        assert!(requires_raw_recompute_consent(&before, id));
        for (command, params) in [
            ("layer.smartFilter.setVisible", json!({"visible":!hidden})),
            ("layer.smartFilter.disableSmartFilters", json!({"enabled":true})),
            ("layer.smartFilter.delete", json!({})),
            ("layer.smartFilter.clearSmartFilters", json!({})),
            ("layer.smartFilter.setParams", json!({"params":{"exposure":1}})),
            ("layer.smartObjects.relinkToFile", json!({"path":"synthetic-no-read-source.png"})),
        ] {
            let error = s.execute(command, params).unwrap_err().to_string();
            assert!(error.contains(RAW_RECOMPUTE_CONSENT_ERROR), "{command}: {error}");
            assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
            assert_eq!(revision, s.active().unwrap().revision);
            assert_eq!(history, s.active().unwrap().history.past_len());
        }
        let LayerContent::Smart(sm) = &before.layer(id).unwrap().content else { panic!() };
        assert!(crate::smart_cmds::render_below_filter(&before, sm, 0).is_err());
    }
    let before = s.active().unwrap().doc.clone();
    let LayerContent::Smart(old) = &before.layer(id).unwrap().content else { panic!() };
    let cache = old.cache.clone();
    assert_eq!(accept(&mut s, &json!({"layer":id.0})).unwrap()["accepted"], 1);
    assert!(!requires_raw_recompute_consent(&s.active().unwrap().doc, id));
    let LayerContent::Smart(sm) = &s.active().unwrap().doc.layer(id).unwrap().content else { panic!() };
    assert_eq!(sm.cache, cache);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc, before);
    accept(&mut s, &json!({"layer":id.0})).unwrap();
    s.execute("layer.smartFilter.setParams", json!({"params":{"exposure":0.1}})).unwrap();
}

#[test]
fn collaboration_v84_reserved_metadata_cannot_be_removed_or_forged_by_set_params() {
    let (mut s, id) = session(true);
    accept(&mut s, &json!({"layer":id.0})).unwrap();
    let before = s.active().unwrap().doc.clone();
    for params in [Value::Null, json!(12), json!({"__cameraRawPsd":null}), json!({"__inkpopRawRecomputeAccepted":false})] {
        assert!(s.execute("layer.smartFilter.setParams", json!({"params":params})).is_err());
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    }
    let local = raw(false);
    let mut accepted = local.clone();
    accepted.params.as_object_mut().unwrap().insert(RAW_ACCEPTED_KEY.into(), json!(true));
    assert_eq!(photocraft_io::smart_map::item_for_filter(&local).unwrap(), photocraft_io::smart_map::item_for_filter(&accepted).unwrap());
    assert!(validate_raw_params_update(&local, &json!({"__inkpopRawRecomputeAccepted":true})).is_err());
    assert!(validate_raw_params_update(&local, &json!({"__cameraRawPsd":"00"})).is_err());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn collaboration_v84_native_new_only_writer_does_not_overwrite_or_leave_partial_pair() {
    let suffix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("inkpop-collaboration-v84-{}-{suffix}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let first = dir.join("copy.psd");
    let second = dir.join("copy.inkpop.pcraft");
    let first = first.to_str().unwrap();
    let second = second.to_str().unwrap();
    std::fs::write(second, b"existing master").unwrap();
    assert!(write_new_files(&[(first, b"new psd"), (second, b"new master")]).is_err());
    assert!(!Path::new(first).exists());
    assert_eq!(std::fs::read(second).unwrap(), b"existing master");
    assert!(validate_export_targets(first, Some(second), None).is_err());
    std::fs::remove_file(second).unwrap();
    validate_export_targets(first, Some(second), None).unwrap();
    write_new_files(&[(first, b"new psd"), (second, b"new master")]).unwrap();
    assert!(write_new_files(&[(first, b"overwrite")]).is_err());
    assert_eq!(std::fs::read(first).unwrap(), b"new psd");
    std::fs::remove_file(first).unwrap();
    std::fs::remove_file(second).unwrap();
    std::fs::remove_dir(dir).unwrap();
}
