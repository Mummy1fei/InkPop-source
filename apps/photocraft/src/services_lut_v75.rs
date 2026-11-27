//! Native LUT imports are bounded before allocating/reading the file's contents.
use std::io::Read;
use std::path::Path;

pub(super) const EXTENSIONS: &[&str] = &["cube", "3dl", "look"];
const MAX_BYTES: usize = 64 * 1024 * 1024;

pub(super) fn read(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("LUT input must be a regular file".into());
    }
    if metadata.len() > MAX_BYTES as u64 {
        return Err("LUT file exceeds 64 MiB".into());
    }
    read_bounded(file, MAX_BYTES)
}

fn read_bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader.take(limit.saturating_add(1) as u64).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() > limit {
        return Err("LUT file exceeds 64 MiB".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lut_reader_v75_rejects_growing_streams_and_preserves_exact_bytes() {
        assert_eq!(EXTENSIONS, &["cube", "3dl", "look"]);
        assert_eq!(read_bounded(&b"LUT_3D_SIZE 2\n"[..], 64).unwrap(), b"LUT_3D_SIZE 2\n");
        assert_eq!(read_bounded(&b"1234"[..], 4).unwrap(), b"1234");
        assert!(read_bounded(std::io::repeat(1), 4).is_err());
    }

    #[test]
    fn lut_reader_v75_rejects_oversized_file_before_reading() {
        let path = std::env::temp_dir().join(format!("inkpop-v75-lut-budget-{}.cube", std::process::id()));
        let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
        file.set_len(MAX_BYTES as u64 + 1).unwrap();
        drop(file);
        let result = read(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.unwrap_err(), "LUT file exceeds 64 MiB");
        assert!(read(&path).is_err());
    }
}
