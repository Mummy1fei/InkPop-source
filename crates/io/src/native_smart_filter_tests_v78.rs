use super::*;
use photocraft_algo::camera_raw_color::RgbSpace;
use photocraft_algo::camera_raw_masks::{LocalAdjustment, MaskShape};
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, SmartObject, SmartSource};
use photocraft_geom::{Affine, Rect, Size};
use photocraft_psd::PsdFile;
use photocraft_psd::descriptor::{Descriptor, UnicodeString, Value as PsdValue};
use photocraft_raster::Surface;
use serde_json::json;
use std::sync::Arc;

fn smart(doc: &Document) -> &SmartObject {
    let LayerContent::Smart(smart) = &doc.layers.first().unwrap().content else { panic!("smart object became pixels") };
    smart
}

fn fixture(sample: SampleType, filters: Vec<SmartFilter>) -> Document {
    let mut source = Document::new("synthetic source", Size::new(12, 8), ColorMode::Rgb, sample);
    let fmt = PixelFormat::new(ColorMode::Rgb, sample, true);
    let area = Rect::new(0, 0, 12, 8);
    let mut pixels = Surface::new(fmt);
    pixels.write_region(area, &[0.8, 0.2, 0.1, 1.0].repeat(96));
    source.layers.push(Layer::new("source", LayerContent::Raster(pixels)));
    let bytes = crate::export(&source, "source.psd", &Default::default()).unwrap().bytes;
    let mut cache = Surface::new(fmt);
    cache.write_region(area, &[0.2, 0.4, 0.6, 1.0].repeat(96));
    let mut object = SmartObject::new(SmartSource::Embedded { file_name: "synthetic.psd".into(), bytes: Arc::new(bytes) }, Affine::IDENTITY, Some(cache));
    object.smart_filters = filters;
    let mut doc = Document::new("native filter test", Size::new(12, 8), ColorMode::Rgb, sample);
    doc.layers.push(Layer::new("editable", LayerContent::Smart(object)));
    doc
}

fn raw(params: &CameraRaw) -> SmartFilter {
    SmartFilter { command: "filter.cameraRaw".into(), params: serde_json::to_value(params).unwrap(), blend: BlendMode::Normal, opacity: 1.0, visible: true }
}

fn roundtrip(doc: &Document) -> (Document, PsdFile) {
    let result = crate::export(doc, "native.psd", &Default::default()).unwrap();
    let file = PsdFile::from_bytes(&result.bytes).unwrap();
    assert!(file.layers().first().unwrap().block(KEY).is_some(), "complete InkPop settings are written separately");
    let imported = crate::import("native.psd", &result.bytes).unwrap();
    assert!(!imported.warnings.iter().any(|warning| warning.contains("ignored InkPop")), "{:?}", imported.warnings);
    (imported.document, file)
}

#[test]
fn native_raw_psd_roundtrip_preserves_full_settings_order_flags_and_cached_pixels_at_three_depths_v78() {
    let base = CameraRaw { exposure: 0.75, contrast: 7.3, ..Default::default() };
    let mut output = base.clone();
    output.color_spaces.output = RgbSpace::AdobeRgb;
    assert!(crate::smart_map::item_for_filter(&raw(&output)).is_ok(), "output-only space must not discard a standard exposure filter");
    let mut working = base.clone();
    working.color_spaces.working = RgbSpace::AdobeRgb;
    let mut lut = base.clone();
    lut.color_lookup.enabled = true;
    lut.color_lookup.name = "Synthetic RGB grid".into();
    lut.color_lookup.size = 2;
    lut.color_lookup.values = Arc::new((0..8).flat_map(|index| [(index & 1) as f32, ((index >> 1) & 1) as f32, ((index >> 2) & 1) as f32]).collect());
    let mut local = base.clone();
    local.local_adjustments.push(LocalAdjustment {
        name: "Synthetic radial".into(),
        exposure: 1.1,
        mask: MaskShape::Radial { center: [0.5, 0.5], radius: [0.25, 0.3], feather: 0.5, rotation: 23.0 },
        ..Default::default()
    });
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for params in [&base, &output, &working, &lut, &local] {
            let mut filters = vec![raw(params), raw(&CameraRaw { exposure: -0.4, ..Default::default() })];
            filters.get_mut(1).unwrap().visible = false;
            filters.get_mut(1).unwrap().opacity = 0.35;
            filters.get_mut(1).unwrap().blend = BlendMode::Multiply;
            let mut original = fixture(sample, filters);
            let LayerContent::Smart(object) = &mut original.layers.first_mut().unwrap().content else { panic!("fixture must contain a smart object") };
            object.filters_enabled = false;
            let mut mask = photocraft_doc::LayerMask::hide_all();
            mask.enabled = false;
            mask.linked = true;
            object.filter_mask = Some(mask);
            let (back, _) = roundtrip(&original);
            assert_eq!(smart(&back).smart_filters, smart(&original).smart_filters, "{sample:?}: full parameters and application order");
            assert!(!smart(&back).filters_enabled);
            assert_eq!(smart(&back).filter_mask.as_ref().map(|mask| (mask.enabled, mask.linked)), Some((false, true)));
            let expected = smart(&original).cache.as_ref().unwrap().read_region(original.bounds());
            let actual = smart(&back).cache.as_ref().unwrap().read_region(back.bounds());
            assert_eq!(expected.len(), actual.len());
            assert!(expected.iter().zip(actual).all(|(a, b)| (a - b).abs() < 0.00004), "{sample:?}: cached appearance differs");
            let (again, _) = roundtrip(&back);
            assert_eq!(smart(&again).smart_filters, smart(&original).smart_filters);
        }
    }
}

#[test]
fn changed_standard_filter_placement_or_source_invalidates_native_supplement_v78() {
    let mut params = CameraRaw { exposure: 0.75, ..Default::default() };
    params.color_spaces.output = RgbSpace::AdobeRgb;
    let original = fixture(SampleType::U8, vec![raw(&params)]);
    let (_, file) = roundtrip(&original);
    for edit in ["filter", "placement", "source"] {
        let mut edited = file.clone();
        if edit == "source" {
            let block = edited.global_blocks.iter_mut().find(|block| &block.key == b"lnk2").unwrap();
            let mut files = crate::linked::parse_linked_files(&block.data);
            *files.first_mut().unwrap().bytes.last_mut().unwrap() ^= 1;
            block.data = crate::linked::encode_linked_file(files.first().unwrap());
        } else {
            let block = edited.layer_info.as_mut().unwrap().layers.first_mut().unwrap().blocks.iter_mut().find(|block| &block.key == b"SoLd").unwrap();
            let mut placed = crate::smart_map::parse_sold(&block.data).unwrap();
            if edit == "filter" {
                let mut stack = placed.stack.clone().unwrap();
                stack.filters.first_mut().unwrap().params["exposure"] = json!(1.25);
                super::super::set(&mut placed.descriptor, "filterFX", PsdValue::Descriptor(crate::smart_map::filter_fx(&stack, "synthetic", &mut Vec::new())));
            } else {
                placed.descriptor.items.iter_mut().find(|(key, _)| key.is("Trnf")).unwrap().1 = PsdValue::List(vec![PsdValue::Double(0.0); 8]);
            }
            let mut bytes = b"soLD".to_vec();
            bytes.extend_from_slice(&4u32.to_be_bytes());
            bytes.extend_from_slice(&photocraft_psd::descriptor::VersionedDescriptor::new(placed.descriptor).to_bytes());
            block.data = bytes;
        }
        let imported = crate::import("externally-edited.psd", &edited.to_bytes().unwrap()).unwrap();
        assert!(imported.warnings.iter().any(|warning| warning.contains("ignored InkPop") && warning.contains("changed")), "{edit}: {:?}", imported.warnings);
        assert_eq!(
            smart(&imported.document).smart_filters.first().unwrap().params["colorSpaces"]["output"],
            "document",
            "stale native output choice was not restored"
        );
        if edit == "filter" {
            assert_eq!(smart(&imported.document).smart_filters.first().unwrap().params["exposure"], 1.25);
        }
    }
}

#[test]
fn malformed_oversized_and_invalid_native_payloads_warn_and_keep_standard_psd_filters_v78() {
    let original = fixture(SampleType::U8, vec![raw(&CameraRaw { exposure: 0.75, ..Default::default() })]);
    let (_, file) = roundtrip(&original);
    let saved = file.layers().first().unwrap().block(KEY).unwrap().data.clone();
    let mut unknown_version: Value = serde_json::from_slice(&saved).unwrap();
    unknown_version[0] = json!(99);
    let mut invalid_lut: Value = serde_json::from_slice(&saved).unwrap();
    invalid_lut[5][0]["params"]["colorLookup"]["size"] = json!(usize::MAX);
    let mut too_many_filters: Value = serde_json::from_slice(&saved).unwrap();
    too_many_filters[5] = json!(vec![too_many_filters[5][0].clone(); MAX_FILTERS + 1]);
    let mut invalid_masks: Value = serde_json::from_slice(&saved).unwrap();
    invalid_masks[5][0]["params"]["localAdjustments"] = json!([{"mask": {"kind": "radial", "center": [0.5, 0.5], "radius": [-1.0, 0.2], "feather": 0.5}}]);
    let mut invalid_command: Value = serde_json::from_slice(&saved).unwrap();
    invalid_command[5][0]["command"] = json!("file.save");
    for bad in [
        b"{".to_vec(),
        serde_json::to_vec(&unknown_version).unwrap(),
        serde_json::to_vec(&invalid_lut).unwrap(),
        serde_json::to_vec(&too_many_filters).unwrap(),
        serde_json::to_vec(&invalid_masks).unwrap(),
        serde_json::to_vec(&invalid_command).unwrap(),
        vec![b' '; MAX_BYTES + 1],
    ] {
        let mut damaged = file.clone();
        damaged.layer_info.as_mut().unwrap().layers.first_mut().unwrap().blocks.iter_mut().find(|block| &block.key == KEY).unwrap().data = bad;
        let imported = crate::import("damaged-private-data.psd", &damaged.to_bytes().unwrap()).unwrap();
        assert!(imported.warnings.iter().any(|warning| warning.contains("ignored InkPop")));
        assert_eq!(smart(&imported.document).smart_filters.first().unwrap().params["exposure"], 0.75);
    }
}

#[test]
fn genuine_unknown_photoshop_descriptor_remains_opaque_and_byte_exact_v78() {
    let neutral = SmartFilter { command: String::new(), params: Value::Null, blend: BlendMode::Normal, opacity: 1.0, visible: true };
    let item = Descriptor::new("filterFX")
        .with("Nm  ", PsdValue::Text(UnicodeString::new_nul("Unknown future Photoshop filter")))
        .with("enab", PsdValue::Boolean(true))
        .with("blendOptions", super::super::blend_options(&neutral))
        .with("filterID", PsdValue::Integer(99999))
        .with("Fltr", PsdValue::Descriptor(Descriptor::new("FutureFilter").with("unmodelled", PsdValue::RawData(vec![0, 128, 255, 17]))));
    let opaque = crate::smart_map::filter_from_item(&item);
    assert_eq!(opaque.command, crate::smart_map::UNSUPPORTED_FILTER);
    assert_eq!(crate::smart_map::item_for_filter(&opaque).unwrap().to_bytes(), item.to_bytes());
    let (back, first) = roundtrip(&fixture(SampleType::U16, vec![opaque.clone()]));
    assert_eq!(smart(&back).smart_filters, vec![opaque]);
    let (_, second) = roundtrip(&back);
    assert_eq!(first.layers().first().unwrap().block(b"SoLd").unwrap().data, second.layers().first().unwrap().block(b"SoLd").unwrap().data);
}

#[test]
fn native_payload_and_repeated_psd_roundtrips_keep_nontrivial_exposure_bits_and_template_exact_v78() {
    // This valid f32 exposure has a longer exact f64 decimal representation.
    // serde_json's optional fast parser used to round it one f64 ULP downward.
    // The fixture is synthetic and contains no source document data or paths.
    let exposure = 0.18707036972045898_f64;
    let mut filter = raw(&CameraRaw::default());
    filter.params["exposure"] = json!(exposure);
    let mut item = crate::smart_map::item_for_filter(&filter).unwrap();
    super::super::set(&mut item, "filterID", PsdValue::Integer(2621));
    let original = crate::smart_map::filter_from_item(&item);
    assert_eq!(original.command, "filter.cameraRaw");
    assert_eq!(original.params["exposure"].as_f64().unwrap().to_bits(), exposure.to_bits());
    let stack = FilterStack { filters: vec![original.clone()], ..Default::default() };
    let source_digest = *blake3::hash(b"synthetic source binding").as_bytes();
    let encoded = encode(&stack, b"synthetic standard placed block", &source_digest).unwrap();
    let decoded = decode(&encoded, b"synthetic standard placed block", &source_digest).unwrap();
    assert_eq!(decoded, stack, "native JSON must preserve complete parameters exactly");
    assert_eq!(decoded.filters[0].params["exposure"].as_f64().unwrap().to_bits(), exposure.to_bits());
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut document = fixture(sample, vec![original.clone()]);
        for pass in 0..3 {
            let (back, _) = roundtrip(&document);
            let filters = &smart(&back).smart_filters;
            assert_eq!(filters.as_slice(), std::slice::from_ref(&original), "{sample:?}, pass {pass}: full JSON/template mismatch");
            assert_eq!(filters[0].params["exposure"].as_f64().unwrap().to_bits(), exposure.to_bits());
            assert_eq!(crate::smart_map::item_for_filter(&filters[0]).unwrap(), item, "standard descriptor and legacy id stay exact");
            document = back;
        }
    }
}

#[test]
fn private_json_accepts_only_psd_single_byte_alignment_v78() {
    let stack = FilterStack { filters: vec![raw(&CameraRaw::default())], ..Default::default() };
    let sold = b"synthetic placed binding";
    let source = [7; 32];
    let bytes = encode(&stack, sold, &source).unwrap();
    assert_eq!(decode(&bytes, sold, &source).unwrap(), stack);
    let mut padded = bytes.clone();
    if !padded.len().is_multiple_of(2) {
        padded.push(0);
        assert_eq!(decode(&padded, sold, &source).unwrap(), stack);
    }
    let mut double = bytes.clone();
    double.extend([0, 0]);
    assert!(decode(&double, sold, &source).is_err());
    let mut garbage = bytes;
    garbage.extend(b"garbage");
    assert!(decode(&garbage, sold, &source).is_err());
}

#[test]
fn native_stale_payload_v78_encoding_failure_cannot_reuse_an_older_bound_payload() {
    // Non-default working space cannot be represented by the standard filter
    // descriptor. The old and new standard stacks therefore both remain empty:
    // a stale supplement would still pass the source/SoLd binding check.
    let mut params = CameraRaw { exposure: 0.75, ..Default::default() };
    params.color_spaces.working = RgbSpace::AdobeRgb;
    let (mut imported, old_file) = roundtrip(&fixture(SampleType::U8, vec![raw(&params)]));
    assert!(imported.layers[0].psd_blocks.iter().any(|(key, _)| key == KEY), "exercise a real imported old payload");
    let old_sold = old_file.layers()[0].block(b"SoLd").unwrap().data.clone();
    let bounds = imported.bounds();
    let LayerContent::Smart(object) = &mut imported.layers[0].content else { panic!("synthetic smart layer") };
    object.smart_filters = vec![object.smart_filters[0].clone(); MAX_FILTERS + 1];
    let expected_cache = object.cache.as_ref().unwrap().read_region(bounds);
    let exported = crate::export(&imported, "native-payload-limit.psd", &Default::default()).unwrap();
    assert!(exported.warnings.iter().any(|warning| warning.contains("settings were not retained") && warning.contains("256")), "{:?}", exported.warnings);
    let file = PsdFile::from_bytes(&exported.bytes).unwrap();
    assert_eq!(file.layers()[0].block(b"SoLd").unwrap().data, old_sold, "the pre-existing payload would still bind to this unchanged standard block");
    assert!(file.layers()[0].block(KEY).is_none(), "failed encoding must not copy the old trusted supplement");
    let back = crate::import("native-payload-limit.psd", &exported.bytes).unwrap();
    assert!(smart(&back.document).smart_filters.is_empty(), "old parameters were not silently restored");
    assert_eq!(smart(&back.document).cache.as_ref().unwrap().read_region(back.document.bounds()), expected_cache);
}

#[test]
fn native_stale_payload_v78_empty_stack_raster_and_unavailable_source_drop_old_supplements() {
    for path in ["empty", "raster", "unavailable-source"] {
        let mut params = CameraRaw { exposure: 0.75, ..Default::default() };
        params.color_spaces.working = RgbSpace::AdobeRgb;
        let (mut imported, _) = roundtrip(&fixture(SampleType::U8, vec![raw(&params)]));
        let bounds = imported.bounds();
        let layer = &mut imported.layers[0];
        assert!(layer.psd_blocks.iter().any(|(key, _)| key == KEY));
        let LayerContent::Smart(object) = &mut layer.content else { panic!("synthetic smart layer") };
        let cache = object.cache.clone().unwrap();
        if path == "empty" {
            object.smart_filters.clear();
        } else if path == "raster" {
            layer.content = LayerContent::Raster(cache.clone());
            // Model ordinary rasterization while leaving the newly introduced
            // private supplement in the legacy unknown-tag list deliberately.
            layer.psd_blocks.retain(|(key, _)| !matches!(key, b"SoLd" | b"SoLE" | b"PlLd"));
        } else {
            // A NUL cannot name a real file: exercise source failure without
            // reading any user/project source from the filesystem.
            object.source = SmartSource::Linked { path: "\0".into() };
        }
        let exported = crate::export(&imported, "native-payload-cleared.psd", &Default::default()).unwrap();
        let file = PsdFile::from_bytes(&exported.bytes).unwrap();
        assert!(file.layers()[0].block(KEY).is_none(), "{path}: old supplement is never copied through");
        if path == "unavailable-source" {
            assert!(exported.warnings.iter().any(|warning| warning.contains("smart object written as pixels")));
        }
        let back = crate::import("native-payload-cleared.psd", &exported.bytes).unwrap();
        if path == "empty" {
            assert!(smart(&back.document).smart_filters.is_empty());
        } else {
            assert!(matches!(back.document.layers[0].content, LayerContent::Raster(_)), "{path}");
        }
        assert_eq!(back.document.layers[0].surface().unwrap().read_region(bounds), cache.read_region(bounds), "{path}: appearance pixels survive");
    }
}
