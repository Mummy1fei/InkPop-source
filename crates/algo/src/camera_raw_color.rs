//! Explicit working RGB space with bounded, replayable document profile context.
use base64::Engine as _;
use photocraft_cms::{Builtin, ColorSpace, Profile, Transform};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const MAX_PROFILE_BYTES: usize = 256 * 1024;
const MAX_PROFILE_TEXT: usize = MAX_PROFILE_BYTES * 4 / 3 + 32;
pub const HDR_TABLE_ERROR: &str =
    "Table TRC profiles cannot preserve 32-bit HDR. Convert the source document to an analytic RGB profile before changing color spaces.";

/// The CMS table evaluator is defined only on [0,1]; never silently use it for HDR.
pub fn validate_float_profile(profile: &Profile) -> Result<(), String> {
    let sampled_rgb = profile.trc.as_ref().is_some_and(|curves| curves.iter().any(|curve| matches!(curve, photocraft_cms::Curve::Table(_))));
    let sampled_gray = matches!(profile.gray_trc, Some(photocraft_cms::Curve::Table(_)));
    if sampled_rgb || sampled_gray {
        return Err(HDR_TABLE_ERROR.into());
    }
    if !profile.is_matrix_shaper() {
        return Err("HDR color conversion requires a matrix/TRC source profile".into());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RgbSpace {
    #[default]
    Document,
    Srgb,
    DisplayP3,
    AdobeRgb,
    ProPhotoRgb,
}
impl RgbSpace {
    pub fn profile(self) -> Option<&'static Profile> {
        Some(match self {
            Self::Document => return None,
            Self::Srgb => Builtin::Srgb.profile(),
            Self::DisplayP3 => Builtin::DisplayP3.profile(),
            Self::AdobeRgb => Builtin::AdobeRgbCompat.profile(),
            Self::ProPhotoRgb => Builtin::ProPhotoCompat.profile(),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ColorSpaces {
    pub working: RgbSpace,
    pub output: RgbSpace,
    /// Empty is the legacy sRGB context. A built-in id or bounded `icc:<base64>`.
    #[serde(deserialize_with = "deserialize_source")]
    pub source_profile: String,
}

fn deserialize_source<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    use serde::de::Error as _;
    let text = String::deserialize(d)?;
    if text.len() > MAX_PROFILE_TEXT {
        return Err(D::Error::custom("Camera Raw source profile exceeds 256 KiB"));
    }
    Ok(text)
}

pub fn encode_source(profile: &Profile) -> Result<String, String> {
    if !matches!(profile.color_space, ColorSpace::Rgb | ColorSpace::Gray) || !profile.is_matrix_shaper() {
        return Err("Camera Raw working spaces require an RGB matrix/TRC or Gray TRC source profile".into());
    }
    for builtin in [
        Builtin::Srgb,
        Builtin::DisplayP3,
        Builtin::AdobeRgbCompat,
        Builtin::ProPhotoCompat,
        Builtin::LinearSrgb,
        Builtin::Rec2020,
        Builtin::SGray,
        Builtin::GrayGamma22,
    ] {
        if profile.content_hash() == builtin.profile().content_hash() {
            return Ok(builtin.id().to_string());
        }
    }
    let bytes = profile.to_bytes();
    if bytes.len() > MAX_PROFILE_BYTES {
        return Err("Camera Raw source profile exceeds 256 KiB".into());
    }
    Ok(format!("icc:{}", base64::engine::general_purpose::STANDARD.encode(bytes.as_slice())))
}

impl ColorSpaces {
    pub fn source(&self) -> Result<Arc<Profile>, String> {
        if self.source_profile.len() > MAX_PROFILE_TEXT {
            return Err("Camera Raw source profile exceeds 256 KiB".into());
        }
        let profile = if let Some(encoded) = self.source_profile.strip_prefix("icc:") {
            let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|_| "Invalid Camera Raw source profile encoding")?;
            if bytes.len() > MAX_PROFILE_BYTES {
                return Err("Camera Raw source profile exceeds 256 KiB".into());
            }
            Arc::new(Profile::parse(&bytes).map_err(|e| e.to_string())?)
        } else if self.source_profile.is_empty() {
            Arc::new(Builtin::Srgb.profile().clone())
        } else {
            Arc::new(Builtin::from_id(&self.source_profile).ok_or("Unknown Camera Raw source profile")?.profile().clone())
        };
        if !matches!(profile.color_space, ColorSpace::Rgb | ColorSpace::Gray) || !profile.is_matrix_shaper() {
            return Err("Camera Raw working spaces require an RGB matrix/TRC or Gray TRC source profile".into());
        }
        Ok(profile)
    }
    pub fn validate(&self) -> Result<(), String> {
        // Legacy/default operation never needs to parse or transform the document profile.
        if self.working == RgbSpace::Document && self.source_profile.is_empty() {
            return Ok(());
        }
        self.prepared(false).map(|_| ())
    }
    pub fn validate_depth(&self, float: bool) -> Result<(), String> {
        self.prepared(float).map(|_| ())
    }
    pub fn prepared(&self, float: bool) -> Result<Option<WorkingTransform>, String> {
        if self.working == RgbSpace::Document && self.source_profile.is_empty() {
            return Ok(None);
        }
        let source = self.source()?;
        let Some(working) = self.working.profile() else {
            return Ok(None);
        };
        if float {
            validate_float_profile(&source)?;
            validate_float_profile(working)?;
        }
        let to_work = photocraft_cms::cached(&source, working, Default::default()).map_err(|e| e.to_string())?;
        let to_source = photocraft_cms::cached(working, &source, Default::default()).map_err(|e| e.to_string())?;
        if !matches!(to_work.inputs(), 1 | 3) || to_work.outputs() != 3 || to_source.inputs() != 3 || !matches!(to_source.outputs(), 1 | 3) {
            return Err("Invalid Camera Raw working transform channels".into());
        }
        Ok(Some(WorkingTransform { to_work, to_source }))
    }
    pub fn working_curves(&self) -> Option<&'static [photocraft_cms::Curve; 3]> {
        self.working.profile()?.trc.as_ref()
    }
}

pub struct WorkingTransform {
    to_work: Arc<Transform>,
    to_source: Arc<Transform>,
}

impl WorkingTransform {
    pub(crate) fn to_working(&self, px: &mut [[f32; 4]], width: usize) {
        crate::photo_util::par_rows(px, width, 1, |_, row| {
            for pixel in row {
                let mut out = [0.0; 3];
                self.to_work.eval(&pixel[..self.to_work.inputs()], &mut out);
                pixel[..3].copy_from_slice(&out);
            }
        });
    }
    pub(crate) fn back_to_source(&self, px: &mut [[f32; 4]], width: usize, float: bool) {
        crate::photo_util::par_rows(px, width, 1, |_, row| {
            for pixel in row {
                let mut out = [0.0; 3];
                self.to_source.eval(&pixel[..3], &mut out);
                if self.to_source.outputs() == 1 {
                    out = [out[0]; 3];
                }
                if !float {
                    out = out.map(|v| v.clamp(0.0, 1.0));
                }
                pixel[..3].copy_from_slice(&out);
            }
        });
    }
}

#[cfg(test)]
#[path = "camera_raw_color_tests_v75.rs"]
mod tests;
