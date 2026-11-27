//! Lossless, backwards-readable storage for generated Camera Raw raster masks.
//! Small fields keep the original JSON byte array; larger fields use one bounded
//! checksummed zlib stream. This does not change mask coverage or transport caps.
use std::fmt;
use std::io::{self, Read, Write};

use base64::{Engine, engine::general_purpose::STANDARD};
use flate2::{Compression, Decompress, FlushDecompress, Status, write::ZlibEncoder};
use serde::de::{Error as _, SeqAccess, Visitor};
use serde::{Deserializer, Serialize, Serializer};

const ARRAY_THRESHOLD: usize = 4096;
const PREFIX: &str = "zlib64-v1:";
const MAX_DECODED: usize = super::MAX_RASTER_SIDE as usize * super::MAX_RASTER_SIDE as usize;
// Above the maximum expansion overhead of a valid encoder for a 1 MiB input,
// while still bounding base64 and compressed-input allocations independently.
const MAX_COMPRESSED: usize = MAX_DECODED + 64 * 1024;
const MAX_ENCODED: usize = MAX_COMPRESSED.div_ceil(3) * 4;

pub(super) fn serialize<S: Serializer>(values: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    if values.len() > MAX_DECODED {
        return Err(serde::ser::Error::custom("Raster values exceed the per-shape storage budget"));
    }
    if values.len() <= ARRAY_THRESHOLD {
        return values.serialize(serializer);
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(values).map_err(serde::ser::Error::custom)?;
    let compressed = encoder.finish().map_err(serde::ser::Error::custom)?;
    if compressed.len() > MAX_COMPRESSED {
        return Err(serde::ser::Error::custom("Compressed Raster values exceed the storage budget"));
    }
    serializer.serialize_str(&format!("{PREFIX}{}", STANDARD.encode(compressed)))
}

pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    struct ValuesVisitor;
    impl<'de> Visitor<'de> for ValuesVisitor {
        type Value = Vec<u8>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a bounded byte array or zlib64-v1 encoded Raster values")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
            let hint = sequence.size_hint().unwrap_or(0);
            if hint > MAX_DECODED {
                return Err(A::Error::custom("Raster array exceeds the per-shape storage budget"));
            }
            let mut values = Vec::with_capacity(hint.min(ARRAY_THRESHOLD));
            while let Some(value) = sequence.next_element::<u8>()? {
                if values.len() >= MAX_DECODED {
                    return Err(A::Error::custom("Raster array exceeds the per-shape storage budget"));
                }
                values.push(value);
            }
            Ok(values)
        }

        fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Self::Value, E> {
            decode(text).map_err(E::custom)
        }
    }
    deserializer.deserialize_any(ValuesVisitor)
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Unlike a plain Read decoder, this adapter requires Status::StreamEnd. EOF
/// before the checksum is an error, not a successful partial raster. StreamEnd
/// verifies Adler-32; total_in rejects both garbage and another appended stream.
struct CompleteZlibReader<'a> {
    bytes: &'a [u8],
    decoder: Decompress,
    finished: bool,
}

impl Read for CompleteZlibReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() || self.finished {
            return Ok(0);
        }
        loop {
            let before_in = self.decoder.total_in();
            let before_out = self.decoder.total_out();
            let consumed = usize::try_from(before_in).map_err(|_| invalid("Invalid Raster stream offset"))?;
            let remaining = self.bytes.get(consumed..).ok_or_else(|| invalid("Invalid Raster stream offset"))?;
            let status = self.decoder.decompress(remaining, output, FlushDecompress::None).map_err(|e| invalid(format!("Invalid Raster zlib stream: {e}")))?;
            let produced = usize::try_from(self.decoder.total_out() - before_out).map_err(|_| invalid("Invalid Raster output count"))?;
            if status == Status::StreamEnd {
                self.finished = true;
                if self.decoder.total_in() != self.bytes.len() as u64 {
                    return Err(invalid("Raster zlib stream contains trailing data"));
                }
                return Ok(produced);
            }
            if produced != 0 {
                return Ok(produced);
            }
            if self.decoder.total_in() == before_in {
                return Err(invalid("Truncated or stalled Raster zlib stream"));
            }
        }
    }
}

fn decode(text: &str) -> io::Result<Vec<u8>> {
    let encoded = text.strip_prefix(PREFIX).ok_or_else(|| invalid("Unsupported Raster values encoding version"))?;
    if encoded.len() > MAX_ENCODED {
        return Err(invalid("Encoded Raster values exceed the storage budget"));
    }
    let compressed = STANDARD.decode(encoded).map_err(|e| invalid(format!("Invalid Raster base64: {e}")))?;
    if compressed.len() > MAX_COMPRESSED {
        return Err(invalid("Compressed Raster values exceed the storage budget"));
    }
    let mut reader = CompleteZlibReader { bytes: &compressed, decoder: Decompress::new(true), finished: false };
    let mut values = Vec::new();
    reader.by_ref().take(MAX_DECODED as u64 + 1).read_to_end(&mut values)?;
    if values.len() > MAX_DECODED {
        return Err(invalid("Decoded Raster values exceed the per-shape storage budget"));
    }
    if !reader.finished {
        return Err(invalid("Raster zlib stream did not finish"));
    }
    Ok(values)
}

#[cfg(test)]
#[path = "camera_raw_raster_codec_tests_v66.rs"]
mod tests;
