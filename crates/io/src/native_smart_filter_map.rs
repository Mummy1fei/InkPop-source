//! InkPop's private, versioned smart-filter settings, separate from Photoshop's descriptors.
//! The binding covers actual embedded source bytes and the complete standard placed block.
//! A real external source/placement/filter edit invalidates this supplementary metadata.

use photocraft_algo::camera_raw::CameraRaw;
use photocraft_doc::SmartFilter;
use serde_json::Value;
use std::io::Write;

use super::FilterStack;

pub(crate) const KEY: &[u8; 4] = b"InSF";
const VERSION: u32 = 1;
const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_FILTERS: usize = 256;
const MAX_JSON_TOKENS: usize = 1_000_000;

fn binding(sold: &[u8], source: &[u8; 32]) -> String {
    let mut hash = blake3::Hasher::new();
    hash.update(b"InkPop PSD smart filters v1\0");
    hash.update(source);
    hash.update(&(sold.len() as u64).to_be_bytes());
    hash.update(sold);
    hash.finalize().to_hex().to_string()
}

/// Find only the requested embedded source without cloning unrelated large images.
pub(crate) fn linked_source_digest(data: &[u8], uuid: &str) -> Option<[u8; 32]> {
    fn take<'a>(data: &'a [u8], position: &mut usize, length: usize) -> Option<&'a [u8]> {
        let end = position.checked_add(length)?;
        let result = data.get(*position..end)?;
        *position = end;
        Some(result)
    }
    fn u32_at(data: &[u8], position: &mut usize) -> Option<u32> {
        Some(u32::from_be_bytes(take(data, position, 4)?.try_into().ok()?))
    }
    fn u64_at(data: &[u8], position: &mut usize) -> Option<u64> {
        Some(u64::from_be_bytes(take(data, position, 8)?.try_into().ok()?))
    }
    let mut position = 0;
    while position < data.len() {
        let length = usize::try_from(u64_at(data, &mut position)?).ok()?;
        let item = take(data, &mut position, length)?;
        position = position.checked_add(3)? & !3;
        let mut cursor = 0;
        let kind = take(item, &mut cursor, 4)?;
        take(item, &mut cursor, 4)?;
        let uuid_length = usize::from(*take(item, &mut cursor, 1)?.first()?);
        let id = take(item, &mut cursor, uuid_length)?;
        if kind != b"liFD" || id != uuid.as_bytes() {
            continue;
        }
        let name_units = usize::try_from(u32_at(item, &mut cursor)?).ok()?;
        take(item, &mut cursor, name_units.checked_mul(2)?)?;
        take(item, &mut cursor, 8)?;
        let source_length = usize::try_from(u64_at(item, &mut cursor)?).ok()?;
        let descriptor = *take(item, &mut cursor, 1)?.first()?;
        if descriptor != 0 {
            let (_, used) = photocraft_psd::descriptor::VersionedDescriptor::parse_prefix(item.get(cursor..)?).ok()?;
            cursor = cursor.checked_add(used)?;
        }
        return Some(*blake3::hash(take(item, &mut cursor, source_length)?).as_bytes());
    }
    None
}

struct BoundedBytes(Vec<u8>);
impl Write for BoundedBytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().checked_add(bytes.len()).is_none_or(|length| length > MAX_BYTES) {
            return Err(std::io::Error::other("InkPop smart-filter payload exceeds 32 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Reject excessive JSON structure before serde allocates its object/array nodes.
fn preflight(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_BYTES {
        return Err("payload exceeds 32 MiB".into());
    }
    let (mut quoted, mut escaped, mut depth, mut tokens) = (false, false, 0usize, 0usize);
    for byte in bytes {
        if quoted {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                quoted = false;
            }
        } else {
            match byte {
                b'"' => quoted = true,
                b'[' | b'{' => {
                    depth += 1;
                    tokens += 1;
                    if depth > 64 {
                        return Err("payload nesting exceeds 64 levels".into());
                    }
                }
                b']' | b'}' => depth = depth.saturating_sub(1),
                b',' | b':' => tokens += 1,
                _ => {}
            }
            if tokens > MAX_JSON_TOKENS {
                return Err("payload has too many JSON values".into());
            }
        }
    }
    Ok(())
}

fn validate_filter(filter: &SmartFilter) -> Result<(), String> {
    if !(filter.command.starts_with("filter.") || filter.command.starts_with("image.adjustments.") || filter.command == super::UNSUPPORTED_FILTER)
        || filter.command.len() > 256
        || filter.command.chars().any(char::is_control)
        || !filter.opacity.is_finite()
        || !(0.0..=1.0).contains(&filter.opacity)
    {
        return Err("invalid filter identity or opacity".into());
    }
    if filter.command == "filter.cameraRaw" {
        for key in ["pointCurve", "pointCurveRed", "pointCurveGreen", "pointCurveBlue"] {
            if filter.params.get(key).and_then(Value::as_array).is_some_and(|points| points.len() > photocraft_algo::camera_raw::MAX_CURVE_POINTS) {
                return Err("Camera Raw point curve exceeds its point limit".into());
            }
        }
        let typed: CameraRaw = serde_json::from_value(filter.params.clone()).map_err(|error| error.to_string())?;
        typed.validate()?;
    }
    Ok(())
}

pub(crate) fn encode(stack: &FilterStack, sold: &[u8], source: &[u8; 32]) -> Result<Vec<u8>, String> {
    if stack.filters.len() > MAX_FILTERS {
        return Err("more than 256 smart filters".into());
    }
    for filter in &stack.filters {
        validate_filter(filter)?;
    }
    let mut bytes = BoundedBytes(Vec::new());
    // A fixed tuple avoids a second cloned JSON tree just to serialize the stack.
    serde_json::to_writer(&mut bytes, &(VERSION, binding(sold, source), stack.enabled, stack.mask_enabled, stack.mask_linked, &stack.filters))
        .map_err(|error| error.to_string())?;
    preflight(&bytes.0)?;
    Ok(bytes.0)
}

pub(crate) fn decode(bytes: &[u8], sold: &[u8], source: &[u8; 32]) -> Result<FilterStack, String> {
    preflight(bytes)?;
    // PSD tagged blocks can expose their even-length alignment byte on read.
    // JSON itself cannot contain a literal NUL: accept exactly one terminal
    // alignment byte, retaining strict rejection of other trailing data.
    let bytes = if bytes.len().is_multiple_of(2) && bytes.last() == Some(&0) {
        bytes.get(..bytes.len().saturating_sub(1)).ok_or("invalid payload padding")?
    } else {
        bytes
    };
    let value: Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let values = value.as_array().filter(|array| array.len() == 6).ok_or("invalid payload structure")?;
    if values.first().and_then(Value::as_u64) != Some(u64::from(VERSION)) {
        return Err("unknown payload version".into());
    }
    if values.get(1).and_then(Value::as_str) != Some(binding(sold, source).as_str()) {
        return Err("source, placement or Photoshop filters changed; ignoring stale InkPop settings".into());
    }
    let flag = |index| values.get(index).and_then(Value::as_bool).ok_or_else(|| "invalid filter-stack flags".to_string());
    let filters =
        values.get(5).and_then(Value::as_array).filter(|filters| filters.len() <= MAX_FILTERS).ok_or("more than 256 filters or invalid filter list")?;
    let mut restored = Vec::with_capacity(filters.len());
    for filter in filters {
        let filter: SmartFilter = serde_json::from_value(filter.clone()).map_err(|error| error.to_string())?;
        validate_filter(&filter)?;
        restored.push(filter);
    }
    Ok(FilterStack { enabled: flag(2)?, mask_enabled: flag(3)?, mask_linked: flag(4)?, filters: restored })
}

#[cfg(test)]
#[path = "native_smart_filter_tests_v78.rs"]
mod tests;
