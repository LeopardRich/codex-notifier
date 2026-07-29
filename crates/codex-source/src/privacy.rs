//! Shared one-way identifiers for privacy-safe source correlation.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

pub(crate) fn hash_source_id(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let mut output = String::with_capacity(71);
    output.push_str("sha256:");
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}
