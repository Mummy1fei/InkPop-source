//! Photoshop `.tpl` tool presets, implemented from observed file structure.
//! Version 3 has an `8BTP` header, a section count, `8BIM` sections and named
//! version-16 tool descriptors. Patterns and sampled tips use the existing PSD/ABR layouts.

use crate::abr::{self, AbrFile, MAX_BRUSHES, MAX_TOTAL_BYTES};
use crate::descriptor::{Descriptor, VersionedDescriptor};
use crate::error::{PsdError, Result};
use crate::patterns::parse_pattern_block;

#[derive(Debug, Clone, PartialEq)]
/// A named tool and its original Photoshop descriptor.
pub struct ToolPreset {
    /// Display name, without the stored UTF-16 terminator.
    pub name: String,
    /// Tool class and option fields.
    pub settings: Descriptor,
}

#[derive(Debug, Clone, PartialEq, Default)]
/// Parsed tool presets and the assets they reference.
pub struct TplFile {
    /// Presets in file order.
    pub presets: Vec<ToolPreset>,
    /// Sampled tips and texture patterns.
    pub assets: AbrFile,
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(len).ok_or(PsdError::LimitExceeded("tpl length"))?;
        let bytes = self.bytes.get(self.pos..end).ok_or(PsdError::UnexpectedEof { offset: self.pos, needed: len })?;
        self.pos = end;
        Ok(bytes)
    }
    fn number(&mut self) -> Result<u32> {
        let mut b = [0; 4];
        b.copy_from_slice(self.take(4)?);
        Ok(u32::from_be_bytes(b))
    }
    fn tools(&mut self) -> Result<Vec<ToolPreset>> {
        let count = self.number()? as usize;
        if count > MAX_BRUSHES {
            return Err(PsdError::LimitExceeded("tpl preset count"));
        }
        let mut tools = Vec::new();
        for _ in 0..count {
            let n = self.number()? as usize;
            if n > 4096 {
                return Err(PsdError::LimitExceeded("tpl preset name"));
            }
            let name = self.take(n * 2)?;
            let name = String::from_utf16_lossy(&name.as_chunks::<2>().0.iter().map(|b| u16::from_be_bytes(*b)).collect::<Vec<_>>());
            let (vd, used) = VersionedDescriptor::parse_prefix(self.bytes.get(self.pos..).ok_or(PsdError::invalid("tpl descriptor"))?)?;
            self.take(used)?;
            tools.push(ToolPreset { name: name.trim_end_matches('\0').to_string(), settings: vd.descriptor });
        }
        Ok(tools)
    }
}

/// Parse a version-3 tool preset file, rejecting malformed lengths and excessive counts.
pub fn parse(bytes: &[u8]) -> Result<TplFile> {
    if bytes.len() > MAX_TOTAL_BYTES {
        return Err(PsdError::LimitExceeded("tpl file size"));
    }
    let mut r = Reader { bytes, pos: 0 };
    if r.take(4)? != b"8BTP" {
        return Err(PsdError::invalid("not a Photoshop tool preset file"));
    }
    let version = r.number()?;
    if version != 3 {
        return Err(PsdError::Unsupported(format!("tool preset version {version}")));
    }
    let sections = r.number()?;
    if sections > 64 {
        return Err(PsdError::LimitExceeded("tpl section count"));
    }
    let mut file = TplFile::default();
    for _ in 0..sections {
        if r.take(4)? != b"8BIM" {
            return Err(PsdError::invalid("tpl section signature"));
        }
        let key = r.take(4)?;
        let len = r.number()? as usize;
        let data = r.take(len)?;
        match key {
            b"tptp" => file.presets.extend(Reader { bytes: data, pos: 0 }.tools()?),
            b"tpbd" => abr::tpl_samples(data, &mut file.assets)?,
            b"tppa" => file.assets.patterns.extend(parse_pattern_block(data)?),
            // Other assets are not used by painting tools; report their presence.
            b"tpsh" | b"tpst" => {}
            _ => file.assets.warnings.push(format!("tool asset section {} ignored", String::from_utf8_lossy(key))),
        }
        if file.presets.len() > MAX_BRUSHES {
            return Err(PsdError::LimitExceeded("tpl preset count"));
        }
    }
    if file.presets.is_empty() {
        return Err(PsdError::invalid("no tool presets in the file"));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::Value;

    pub fn fixture() -> Vec<u8> {
        let mut body = 1u32.to_be_bytes().to_vec();
        let name: Vec<u16> = "测试 Brush\0".encode_utf16().collect();
        body.extend_from_slice(&(name.len() as u32).to_be_bytes());
        for c in name {
            body.extend_from_slice(&c.to_be_bytes());
        }
        body.extend(VersionedDescriptor::new(Descriptor::new("PbTl").with("Opct", Value::Integer(42))).to_bytes());
        let mut file = b"8BTP".to_vec();
        file.extend_from_slice(&3u32.to_be_bytes());
        file.extend_from_slice(&1u32.to_be_bytes());
        file.extend_from_slice(b"8BIMtptp");
        file.extend_from_slice(&(body.len() as u32).to_be_bytes());
        file.extend(body);
        file
    }

    #[test]
    fn reads_named_tool_descriptor() {
        let parsed = parse(&fixture()).unwrap();
        assert_eq!(parsed.presets[0].name, "测试 Brush");
        assert!(parsed.presets[0].settings.class_id.is("PbTl"));
        assert_eq!(parsed.presets[0].settings.get("Opct"), Some(&Value::Integer(42)));
    }

    #[test]
    fn rejects_truncation_and_hostile_counts() {
        let file = fixture();
        for len in 0..file.len() {
            assert!(parse(&file[..len]).is_err(), "truncation at {len}");
        }
        let mut bad = file.clone();
        bad[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(parse(&bad).is_err());
        let mut bad = file;
        bad[24..28].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(parse(&bad).is_err());
    }
}
