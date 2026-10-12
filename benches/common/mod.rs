//! Inputs shared by the benchmarks: the fixtures and a generated dataset.

use std::fs;

#[path = "../../tools/perf/src/generate.rs"]
mod generate;

pub use generate::generated;

/// The fixtures and a generated dataset of 20,000 individuals, by name.
pub fn inputs() -> Vec<(String, String)> {
    let mut inputs: Vec<(String, String)> = [
        ("simple", "tests/fixtures/simple.ged"),
        ("sample", "tests/fixtures/sample.ged"),
        ("maximal551", "tests/fixtures/conformance/maximal551.ged"),
        ("washington", "tests/fixtures/washington.ged"),
    ]
    .iter()
    .filter_map(|(name, path)| Some(((*name).to_string(), fs::read_to_string(path).ok()?)))
    .collect();
    inputs.push(("generated-20k".to_string(), generated(20_000)));
    inputs
}
