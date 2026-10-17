use std::io::Read;

const ZSTD_LEVEL: i32 = 9;
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

pub fn pack(bytes: &[u8]) -> Option<Vec<u8>> {
    zstd::bulk::compress(bytes, ZSTD_LEVEL).ok()
}

pub fn unpack(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.starts_with(&ZSTD_MAGIC) {
        return zstd::stream::decode_all(bytes).ok();
    }
    if bytes.starts_with(&GZIP_MAGIC) {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(bytes).read_to_end(&mut out).ok()?;
        return Some(out);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn legacy_gzip(data: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn packed_bodies_round_trip_and_shrink() {
        let data = br#"{"role":"user","content":"hello hello"}"#.repeat(500);
        let packed = pack(&data).unwrap();
        assert!(packed.starts_with(&ZSTD_MAGIC));
        assert!(packed.len() < data.len() / 10);
        assert_eq!(unpack(&packed).unwrap(), data);
    }

    #[test]
    fn rows_stored_as_gzip_before_the_switch_still_decode() {
        let data = b"an older request body".repeat(40);
        assert_eq!(unpack(&legacy_gzip(&data)).unwrap(), data);
    }

    #[test]
    fn empty_and_unknown_inputs_are_not_decoded() {
        assert_eq!(unpack(&pack(b"").unwrap()).unwrap(), b"");
        assert_eq!(unpack(b"plain text, not packed"), None);
        assert_eq!(unpack(b""), None);
    }

    #[test]
    fn zstd_beats_fast_gzip_on_json() {
        let data = (0..400)
            .map(|i| format!(r#"{{"id":{i},"role":"assistant","content":"step {i} of the plan"}}"#))
            .collect::<Vec<_>>()
            .join(",")
            .into_bytes();
        assert!(pack(&data).unwrap().len() < legacy_gzip(&data).len());
    }
}
