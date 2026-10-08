//! Test-side support for the conformance suite. Only `adapter` touches
//! ged_io; everything else is independent of the crate under test.

pub mod adapter;
#[rustfmt::skip]
pub mod ansel_table;
pub mod cases;
pub mod checker;
pub mod ratchet;
pub mod semantic;
#[rustfmt::skip]
pub mod spec_tables;
pub mod tree;

use std::path::{Path, PathBuf};

/// `tests/fixtures/conformance/<rel>`.
pub fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/conformance")
        .join(rel)
}

pub fn read_fixture(rel: &str) -> Vec<u8> {
    let p = fixture(rel);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}
