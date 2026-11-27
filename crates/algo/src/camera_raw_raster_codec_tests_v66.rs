use super::*;
use serde::Deserialize;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Stored {
    #[serde(with = "super")]
    values: Vec<u8>,
}

fn compressed_text(values: &[u8]) -> String {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(values).unwrap();
    format!("{PREFIX}{}", STANDARD.encode(encoder.finish().unwrap()))
}

fn deserialize_text(text: String) -> Result<Stored, serde_json::Error> {
    serde_json::from_value(serde_json::json!({"values":text}))
}

#[test]
fn small_array_representation_and_existing_large_array_remain_compatible() {
    let tiny = Stored { values: vec![0, 1, 2, 255] };
    assert_eq!(serde_json::to_string(&tiny).unwrap(), r#"{"values":[0,1,2,255]}"#);
    let threshold = Stored { values: vec![11; ARRAY_THRESHOLD] };
    assert!(serde_json::to_value(&threshold).unwrap()["values"].is_array());
    let legacy = vec![37u8; 16_384];
    let parsed: Stored = serde_json::from_value(serde_json::json!({"values":legacy})).unwrap();
    assert_eq!(parsed.values, vec![37; 16_384]);
    let compressed = Stored { values: vec![11; ARRAY_THRESHOLD + 1] };
    assert!(serde_json::to_value(&compressed).unwrap()["values"].as_str().unwrap().starts_with(PREFIX));
}

#[test]
fn compressed_probabilities_roundtrip_every_byte_and_exact_per_shape_boundary() {
    for length in [ARRAY_THRESHOLD + 1, 1024 * 683, MAX_DECODED] {
        let values = (0..length).map(|i| ((i * 29) ^ (i / 113)) as u8).collect::<Vec<_>>();
        let stored = Stored { values };
        let bytes = serde_json::to_vec(&stored).unwrap();
        let decoded: Stored = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded, stored);
    }
}

#[test]
fn incompressible_bytes_are_lossless_without_relaxing_the_decoded_budget() {
    let mut rng = 0x1234_5678u32;
    let values = (0..MAX_DECODED)
        .map(|_| {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            rng as u8
        })
        .collect::<Vec<_>>();
    let stored = Stored { values };
    let bytes = serde_json::to_vec(&stored).unwrap();
    assert_eq!(serde_json::from_slice::<Stored>(&bytes).unwrap(), stored);
    assert!(serde_json::to_vec(&Stored { values: vec![0; MAX_DECODED + 1] }).is_err());
}

#[test]
fn bad_types_unknown_versions_and_malformed_base64_fail_gracefully() {
    for value in [
        serde_json::Value::Null,
        serde_json::json!(42),
        serde_json::json!({}),
        serde_json::json!([256]),
        serde_json::json!([-1]),
        serde_json::json!([1.5]),
        serde_json::json!(["1"]),
    ] {
        assert!(serde_json::from_value::<Stored>(serde_json::json!({"values":value})).is_err());
    }
    for text in ["", "zlib64-v0:eA==", "zlib64-v2:eA==", "zlib64-v1:", "zlib64-v1:@@@@", "zlib64-v1:AAAA", "zlib64-v1:eA"] {
        assert!(deserialize_text(text.into()).is_err(), "{text}");
    }
}

#[test]
fn truncated_stream_and_bad_checksum_are_rejected_instead_of_partial_success() {
    let text = compressed_text(&vec![127; 10_000]);
    let binary = STANDARD.decode(text.strip_prefix(PREFIX).unwrap()).unwrap();
    for missing in [1, 2, 4, 8] {
        let truncated = &binary[..binary.len() - missing];
        assert!(deserialize_text(format!("{PREFIX}{}", STANDARD.encode(truncated))).is_err());
    }
    let mut corrupt = binary;
    let last = corrupt.len() - 1;
    corrupt[last] ^= 1;
    assert!(deserialize_text(format!("{PREFIX}{}", STANDARD.encode(corrupt))).is_err());
}

#[test]
fn appended_garbage_or_a_second_valid_stream_is_rejected() {
    let text = compressed_text(&vec![31; 5000]);
    let binary = STANDARD.decode(text.strip_prefix(PREFIX).unwrap()).unwrap();
    for suffix in [vec![0], vec![255, 0, 255], binary.clone()] {
        let mut appended = binary.clone();
        appended.extend(suffix);
        assert!(deserialize_text(format!("{PREFIX}{}", STANDARD.encode(appended))).is_err());
    }
}

#[test]
fn oversized_legacy_arrays_strings_and_zlib_bombs_are_bounded() {
    assert!(serde_json::from_value::<Stored>(serde_json::json!({"values":vec![0u8; MAX_DECODED + 1]})).is_err());
    assert!(deserialize_text(format!("{PREFIX}{}", "A".repeat(MAX_ENCODED + 1))).is_err());
    let bomb = compressed_text(&vec![1u8; MAX_DECODED + 1]);
    assert!(bomb.len() < 16_384, "exercise a small compressed input with oversized decoded output");
    assert!(deserialize_text(bomb).is_err());
}

#[test]
fn generated_reference_masks_stay_exact_and_compact_in_a_full_shape_roundtrip() {
    let mask = crate::camera_raw_masks::MaskShape::Raster {
        width: 1024,
        height: 683,
        values: (0..1024 * 683).map(|i| if i % 1024 < 512 { 255 } else { 0 }).collect(),
    };
    let encoded = serde_json::to_vec(&mask).unwrap();
    assert!(encoded.len() < 32_768);
    let decoded: crate::camera_raw_masks::MaskShape = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, mask);
}
