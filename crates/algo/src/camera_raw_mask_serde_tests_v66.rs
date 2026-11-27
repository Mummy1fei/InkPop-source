use super::*;
use serde::de::{DeserializeSeed, IntoDeserializer};
use serde_json::{Value, json};
use std::cell::Cell;

struct CountingSequence<'a> {
    values: std::vec::IntoIter<Value>,
    read: &'a Cell<usize>,
    hint: Option<usize>,
}
impl<'de> SeqAccess<'de> for CountingSequence<'_> {
    type Error = serde_json::Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error> {
        match self.values.next() {
            Some(value) => {
                self.read.set(self.read.get() + 1);
                seed.deserialize(value.into_deserializer()).map(Some)
            }
            None => Ok(None),
        }
    }
    fn size_hint(&self) -> Option<usize> {
        self.hint
    }
}

fn group_with_full_raster() -> Value {
    serde_json::to_value(LocalAdjustment { mask: MaskShape::Raster { width: 1024, height: 1024, values: vec![0; 1024 * 1024] }, ..Default::default() }).unwrap()
}

#[test]
fn oversize_known_group_or_component_count_rejects_before_parsing_any_element() {
    for (limit, components) in [(MAX_MASKS, false), (MAX_COMPONENTS, true)] {
        let read = Cell::new(0);
        let seq = CountingSequence { values: vec![json!({}); limit + 1].into_iter(), read: &read, hint: Some(limit + 1) };
        let deserializer = serde::de::value::SeqAccessDeserializer::new(seq);
        if components {
            assert!(deserialize_components(deserializer).is_err());
        } else {
            assert!(deserialize_items(deserializer).is_err());
        }
        assert_eq!(read.get(), 0);
    }
}

#[test]
fn unknown_length_group_stream_stops_at_first_aggregate_overflow() {
    let value = group_with_full_raster();
    let read = Cell::new(0);
    let seq = CountingSequence { values: vec![value; 64].into_iter(), read: &read, hint: None };
    assert!(deserialize_items(serde::de::value::SeqAccessDeserializer::new(seq)).is_err());
    assert_eq!(read.get(), 17, "the parser must not expand all 64 independently valid 1 MiB fields");
}

#[test]
fn unknown_length_components_stop_at_first_aggregate_overflow() {
    let group = group_with_full_raster();
    let component = json!({"name":"Generated", "mask":group["mask"]});
    let read = Cell::new(0);
    let seq = CountingSequence { values: vec![component; MAX_COMPONENTS].into_iter(), read: &read, hint: None };
    assert!(deserialize_components(serde::de::value::SeqAccessDeserializer::new(seq)).is_err());
    assert_eq!(read.get(), 17);
}

#[test]
fn component_and_camera_raw_defaults_and_legal_arrays_stay_compatible() {
    let group: LocalAdjustment = serde_json::from_value(json!({})).unwrap();
    assert!(group.components.is_empty());
    let params: crate::camera_raw::CameraRaw = serde_json::from_value(json!({})).unwrap();
    assert!(params.local_adjustments.is_empty());
    let params: crate::camera_raw::CameraRaw =
        serde_json::from_value(json!({"localAdjustments":[{"mask":{"kind":"raster","width":2,"height":1,"values":[0,255]},"components":[]}]})).unwrap();
    let bytes = serde_json::to_vec(&params).unwrap();
    let loaded: crate::camera_raw::CameraRaw = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(loaded, params);
    assert!(params.validate().is_ok());
    for invalid in [Value::Null, json!({}), json!("not masks")] {
        assert!(serde_json::from_value::<crate::camera_raw::CameraRaw>(json!({"localAdjustments":invalid})).is_err());
    }
}

#[test]
fn camera_raw_rejects_compressed_aggregate_overflow_while_exact_boundary_roundtrips() {
    let group = group_with_full_raster();
    let legal = json!({"localAdjustments":vec![group.clone(); 16]});
    let params: crate::camera_raw::CameraRaw = serde_json::from_value(legal).unwrap();
    assert!(params.validate().is_ok());
    let bytes = serde_json::to_vec(&params).unwrap();
    assert!(bytes.len() < 1_048_576, "the adversarial expansion fits the public request budget");
    let loaded: crate::camera_raw::CameraRaw = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(loaded, params);
    let excessive = json!({"localAdjustments":vec![group; 17]});
    assert!(serde_json::from_value::<crate::camera_raw::CameraRaw>(excessive).is_err());
}

#[test]
fn local_group_component_expansion_is_bounded_before_later_validation() {
    let group = group_with_full_raster();
    let component = json!({"name":"Generated", "mask":group["mask"]});
    let excessive = json!({"components":vec![component; MAX_COMPONENTS]});
    assert!(serde_json::from_value::<LocalAdjustment>(excessive).is_err());
}

#[test]
fn unknown_length_count_limit_probes_without_deserializing_an_extra_item() {
    let read = Cell::new(0);
    let mut values = vec![json!({}); MAX_MASKS];
    values.push(json!({"mask":{"kind":"raster","width":1024,"height":1024,"values":"zlib64-v1:not even base64"}}));
    let seq = CountingSequence { values: values.into_iter(), read: &read, hint: None };
    let error = deserialize_items(serde::de::value::SeqAccessDeserializer::new(seq)).unwrap_err();
    assert!(error.to_string().contains("Too many local masks"), "the extra item must be ignored, not inflated: {error}");
    assert_eq!(read.get(), MAX_MASKS + 1);
}
