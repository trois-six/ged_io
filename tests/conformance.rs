//! GEDCOM conformance suite: fixtures, generated probes and the known-gaps
//! ratchet.
//!
//! * `cargo test --test conformance` runs the default tier (in-memory, no
//!   network).
//! * `tools/fetch-corpora.sh && cargo test --all-features --test conformance -- --ignored`
//!   runs the opt-in tier over the pinned external corpora.
//!
//! Every expected failure is listed in `tests/fixtures/conformance/known_gaps.tsv`
//! with its root cause; see `support/ratchet.rs`.

#[path = "conformance/support/mod.rs"]
mod support;

#[path = "conformance/cases.rs"]
mod cases;

#[path = "conformance/vendored.rs"]
mod vendored;

#[path = "conformance/encodings.rs"]
mod encodings;

#[path = "conformance/generated.rs"]
mod generated;

#[path = "conformance/maximal.rs"]
mod maximal;

#[path = "conformance/features.rs"]
mod features;

#[path = "conformance/corpora.rs"]
mod corpora;
