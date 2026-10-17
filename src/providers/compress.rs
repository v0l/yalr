use flate2::{write::GzEncoder, Compression};
use std::io::Write;

const MIN_COMPRESSIBLE_BYTES: usize = 4096;

pub struct EncodedBody {
    pub bytes: Vec<u8>,
    pub gzipped: bool,
}

fn gzip(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).ok()?;
    encoder.finish().ok()
}

pub fn encode_gzip_if_worthwhile(bytes: &[u8]) -> EncodedBody {
    if bytes.len() < MIN_COMPRESSIBLE_BYTES {
        return EncodedBody { bytes: bytes.to_vec(), gzipped: false };
    }
    match gzip(bytes) {
        Some(packed) if packed.len() < bytes.len() => EncodedBody { bytes: packed, gzipped: true },
        _ => EncodedBody { bytes: bytes.to_vec(), gzipped: false },
    }
}

pub async fn encode_gzip_off_thread(bytes: Vec<u8>) -> EncodedBody {
    let fallback = bytes.clone();
    tokio::task::spawn_blocking(move || encode_gzip_if_worthwhile(&bytes))
        .await
        .unwrap_or(EncodedBody { bytes: fallback, gzipped: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn small_bodies_are_left_alone() {
        let encoded = encode_gzip_if_worthwhile(b"{\"a\":1}");
        assert!(!encoded.gzipped);
        assert_eq!(encoded.bytes, b"{\"a\":1}");
    }

    #[test]
    fn large_repetitive_bodies_shrink_and_round_trip() {
        let body = r#"{"role":"user","content":"hello world"}"#.repeat(500).into_bytes();
        let encoded = encode_gzip_if_worthwhile(&body);
        assert!(encoded.gzipped);
        assert!(encoded.bytes.len() < body.len() / 5);
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(&encoded.bytes[..]).read_to_end(&mut out).unwrap();
        assert_eq!(out, body);
    }

    #[test]
    fn incompressible_bodies_are_sent_as_is() {
        let mut x: u64 = 88172645463325252;
        let noise: Vec<u8> = (0..8192)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect();
        let encoded = encode_gzip_if_worthwhile(&noise);
        assert!(!encoded.gzipped);
        assert_eq!(encoded.bytes, noise);
    }

    #[tokio::test]
    async fn off_thread_matches_inline() {
        let body = "abc".repeat(5000).into_bytes();
        let inline = encode_gzip_if_worthwhile(&body);
        let off = encode_gzip_off_thread(body).await;
        assert_eq!(inline.gzipped, off.gzipped);
        assert_eq!(inline.bytes, off.bytes);
    }
}
