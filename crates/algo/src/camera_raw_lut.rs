//! Replayable creative LUT settings. Tables are embedded, never file paths.
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::sync::Arc;

pub const MAX_SIDE: usize = 33;
const MAX_VALUES: usize = MAX_SIDE * MAX_SIDE * MAX_SIDE * 3;
const MAX_BYTES: usize = MAX_VALUES * 4;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ColorLookup {
    pub name: String,
    pub enabled: bool,
    pub amount: f32,
    pub size: usize,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    pub tetrahedral: bool,
    #[serde(with = "table_codec")]
    pub values: Arc<Vec<f32>>,
}
impl Default for ColorLookup {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: false,
            amount: 100.0,
            size: 0,
            domain_min: [0.0; 3],
            domain_max: [1.0; 3],
            tetrahedral: true,
            values: Arc::new(Vec::new()),
        }
    }
}
impl ColorLookup {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.len() > 256 || self.name.chars().any(char::is_control) {
            return Err("LUT name exceeds 256 bytes or contains control characters".into());
        }
        if !self.amount.is_finite() || !(0.0..=100.0).contains(&self.amount) {
            return Err("LUT amount must be between 0 and 100".into());
        }
        for (min, max) in self.domain_min.iter().zip(self.domain_max) {
            if !min.is_finite() || !max.is_finite() || !(max - min).is_finite() || max - min < 1e-6 {
                return Err("LUT input domain must have finite increasing bounds".into());
            }
        }
        if self.size == 0 && self.values.is_empty() && !self.enabled {
            return Ok(());
        }
        if !(2..=MAX_SIDE).contains(&self.size) || self.values.len() != self.size.saturating_pow(3).saturating_mul(3) {
            return Err("LUT table must contain a complete 2–33 cubed RGB grid".into());
        }
        if self.values.iter().any(|v| !v.is_finite()) {
            return Err("LUT contains non-finite values".into());
        }
        Ok(())
    }
    pub fn active(&self) -> bool {
        self.enabled
            && self.amount.is_finite()
            && self.amount > 0.0
            && (2..=MAX_SIDE).contains(&self.size)
            && self.values.len() == self.size.saturating_pow(3).saturating_mul(3)
            && self.domain_min.iter().zip(self.domain_max).all(|(min, max)| min.is_finite() && max.is_finite() && (max - min).is_finite() && max - min >= 1e-6)
    }
    /// Preserve out-of-domain residuals rather than clipping HDR data to the cube.
    pub fn rgb(&self, rgb: [f32; 3]) -> [f32; 3] {
        if !self.active() {
            return rgb;
        }
        let input = std::array::from_fn(|k| (rgb[k] - self.domain_min[k]) / (self.domain_max[k] - self.domain_min[k]));
        let sampled = sample(&self.values, self.size, input, self.tetrahedral);
        std::array::from_fn(|k| {
            let residual = rgb[k] - rgb[k].clamp(self.domain_min[k], self.domain_max[k]);
            rgb[k] + self.amount / 100.0 * (sampled[k] + residual - rgb[k])
        })
    }
}
/// Bounded interpolation, red fastest. Invalid tables are an identity.
pub fn sample(table: &[f32], side: usize, rgb: [f32; 3], tetrahedral: bool) -> [f32; 3] {
    if !(2..=129).contains(&side) || table.len() != side.saturating_pow(3).saturating_mul(3) || rgb.iter().any(|v| !v.is_finite()) {
        return rgb;
    }
    let pos = rgb.map(|v| v.clamp(0.0, 1.0) * (side - 1) as f32);
    let base = pos.map(|v| (v.floor() as usize).min(side - 2));
    let frac: [f32; 3] = std::array::from_fn(|k| pos[k] - base[k] as f32);
    let at = |offset: [usize; 3]| {
        let index = (((base[2] + offset[2]) * side + base[1] + offset[1]) * side + base[0] + offset[0]) * 3;
        std::array::from_fn::<_, 3, _>(|k| table.get(index + k).copied().unwrap_or(0.0))
    };
    if tetrahedral {
        let mut order = [0usize, 1, 2];
        order.sort_by(|a, b| frac[*b].total_cmp(&frac[*a]));
        let mut first = [0; 3];
        first[order[0]] = 1;
        let mut second = first;
        second[order[1]] = 1;
        let vertices = [at([0; 3]), at(first), at(second), at([1; 3])];
        let weights = [1.0 - frac[order[0]], frac[order[0]] - frac[order[1]], frac[order[1]] - frac[order[2]], frac[order[2]]];
        std::array::from_fn(|k| vertices.iter().zip(weights).map(|(v, w)| v[k] * w).sum())
    } else {
        let mut out = [0.0; 3];
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    let offset = [r, g, b];
                    let weight: f32 = offset.iter().zip(frac).map(|(o, f)| if *o == 0 { 1.0 - f } else { f }).product();
                    for (dest, value) in out.iter_mut().zip(at(offset)) {
                        *dest += value * weight;
                    }
                }
            }
        }
        out
    }
}
mod table_codec {
    use super::*;
    use serde::{Deserializer, Serializer, de::Error as _, ser::Error as _};
    pub fn serialize<S: Serializer>(values: &Arc<Vec<f32>>, serializer: S) -> Result<S::Ok, S::Error> {
        if values.len() > MAX_VALUES {
            return Err(S::Error::custom("LUT table exceeds its decoded budget"));
        }
        if values.is_empty() {
            return serializer.serialize_str("");
        }
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        for value in values.iter() {
            encoder.write_all(&value.to_le_bytes()).map_err(S::Error::custom)?;
        }
        let bytes = encoder.finish().map_err(S::Error::custom)?;
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Arc<Vec<f32>>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        if encoded.len() > MAX_BYTES.saturating_mul(2) {
            return Err(D::Error::custom("Encoded LUT exceeds its storage budget"));
        }
        if encoded.is_empty() {
            return Ok(Arc::new(Vec::new()));
        }
        let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).map_err(D::Error::custom)?;
        let mut decoded = Vec::new();
        flate2::read::ZlibDecoder::new(bytes.as_slice()).take((MAX_BYTES + 1) as u64).read_to_end(&mut decoded).map_err(D::Error::custom)?;
        if decoded.len() > MAX_BYTES || decoded.len() % 4 != 0 {
            return Err(D::Error::custom("Decoded LUT exceeds its budget or has incomplete values"));
        }
        let values: Vec<f32> = decoded.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
        if values.iter().any(|v| !v.is_finite()) {
            return Err(D::Error::custom("LUT contains non-finite values"));
        }
        Ok(Arc::new(values))
    }
}
#[cfg(test)]
#[path = "camera_raw_lut_tests_v75.rs"]
mod tests;
