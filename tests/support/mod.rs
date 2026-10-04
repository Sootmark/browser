//! The test files, read whole; those stored gzip-compressed decompressed.

use std::io::Read;

/// `tests/fixtures/{path}`, decompressed if it is gzip.
pub fn fixture(path: &str) -> Vec<u8> {
    let bytes = std::fs::read(format!(
        "{}/tests/fixtures/{path}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    if !common::gzip::is_gzip(&bytes) {
        return bytes;
    }
    let mut plain = Vec::new();
    common::gzip::Decoder::new(bytes.as_slice())
        .read_to_end(&mut plain)
        .unwrap();
    plain
}
