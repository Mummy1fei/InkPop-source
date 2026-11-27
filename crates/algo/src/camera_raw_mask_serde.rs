//! Stream collection deserialization stops expansion as soon as the aggregate
//! Raster budget is exceeded. Per-field zlib limits alone cannot bound a list
//! containing many independently valid compressed fields.
use std::fmt;
use std::marker::PhantomData;

use serde::de::{Error as _, IgnoredAny, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use super::{LocalAdjustment, MAX_COMPONENTS, MAX_MASKS, MAX_RASTER_PIXELS, MaskComponent, MaskShape};

fn shape_bytes(shape: &MaskShape) -> usize {
    match shape {
        MaskShape::Raster { values, .. } => values.len(),
        _ => 0,
    }
}

fn group_bytes(group: &LocalAdjustment) -> usize {
    group.components.iter().fold(shape_bytes(&group.mask), |total, component| total.saturating_add(shape_bytes(&component.mask)))
}

/// Shared entry for CameraRaw.local_adjustments and engine draft requests/results.
/// Missing fields still use their containing struct's existing serde defaults.
pub fn deserialize_items<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<LocalAdjustment>, D::Error> {
    bounded(deserializer, MAX_MASKS, "local masks", group_bytes)
}

pub(super) fn deserialize_components<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<MaskComponent>, D::Error> {
    bounded(deserializer, MAX_COMPONENTS, "mask components", |component: &MaskComponent| shape_bytes(&component.mask))
}

fn bounded<'de, D, T>(deserializer: D, count_limit: usize, label: &'static str, bytes: fn(&T) -> usize) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct BoundedVisitor<T> {
        count_limit: usize,
        label: &'static str,
        bytes: fn(&T) -> usize,
        marker: PhantomData<T>,
    }
    impl<'de, T: Deserialize<'de>> Visitor<'de> for BoundedVisitor<T> {
        type Value = Vec<T>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "a bounded sequence of {}", self.label)
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
            let hint = sequence.size_hint().unwrap_or(0);
            if hint > self.count_limit {
                return Err(A::Error::custom(format!("Too many {} (maximum {})", self.label, self.count_limit)));
            }
            let mut values = Vec::with_capacity(hint.min(self.count_limit));
            let mut raster_bytes = 0usize;
            loop {
                if values.len() == self.count_limit {
                    // Probe only for existence; do not inflate another Raster
                    // merely to learn that the sequence has too many entries.
                    if sequence.next_element::<IgnoredAny>()?.is_some() {
                        return Err(A::Error::custom(format!("Too many {} (maximum {})", self.label, self.count_limit)));
                    }
                    break;
                }
                let Some(value) = sequence.next_element::<T>()? else { break };
                let expanded = (self.bytes)(&value);
                if expanded > MAX_RASTER_PIXELS.saturating_sub(raster_bytes) {
                    return Err(A::Error::custom("Aggregate Raster storage budget exceeded during deserialization"));
                }
                raster_bytes += expanded;
                values.push(value);
            }
            Ok(values)
        }
    }
    deserializer.deserialize_seq(BoundedVisitor { count_limit, label, bytes, marker: PhantomData })
}

#[cfg(test)]
#[path = "camera_raw_mask_serde_tests_v66.rs"]
mod tests;
