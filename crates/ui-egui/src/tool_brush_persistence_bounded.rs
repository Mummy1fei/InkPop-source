//! Bound collection growth before constructing fixed-size brush defaults.

use std::fmt;

use serde::Deserializer;
use serde::de::{Error, IgnoredAny, SeqAccess, Visitor};

use super::{MAX_SLOTS, Slot};

struct SmallString<const MAX: usize>;

impl<'de, const MAX: usize> Visitor<'de> for SmallString<MAX> {
    type Value = String;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "a string of at most {MAX} bytes without control characters")
    }

    fn visit_str<E: Error>(self, value: &str) -> Result<String, E> {
        if value.len() > MAX || value.chars().any(char::is_control) {
            return Err(E::custom(format!("tool brush string exceeds {MAX} bytes or contains control characters")));
        }
        Ok(value.to_owned())
    }
}

pub(super) fn short_string<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    deserializer.deserialize_str(SmallString::<64>)
}

pub(super) fn preset_name<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    struct OptionalName;

    impl<'de> Visitor<'de> for OptionalName {
        type Value = Option<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a bounded tool brush preset name or null")
        }

        fn visit_none<E: Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E: Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
            deserializer.deserialize_str(SmallString::<1024>).map(Some)
        }
    }

    deserializer.deserialize_option(OptionalName)
}

pub(super) fn slots<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Slot>, D::Error> {
    struct Slots;

    impl<'de> Visitor<'de> for Slots {
        type Value = Vec<Slot>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "at most {MAX_SLOTS} tool brush slots")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
            // Never trust a size hint to reserve the input's requested capacity.
            let mut slots = Vec::with_capacity(MAX_SLOTS);
            for _ in 0..MAX_SLOTS {
                match sequence.next_element()? {
                    Some(slot) => slots.push(slot),
                    None => return Ok(slots),
                }
            }
            // Do not construct a 21st BrushSettings, even if it is malformed or
            // contains enormous ignored fields. IgnoredAny streams without a tree.
            if sequence.next_element::<IgnoredAny>()?.is_some() {
                return Err(A::Error::custom(format!("too many tool brush slots (limit {MAX_SLOTS})")));
            }
            Ok(slots)
        }
    }

    deserializer.deserialize_seq(Slots)
}
