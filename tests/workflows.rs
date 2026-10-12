//! The nightly workflow fuzzes every target of `fuzz/`, each seeded with
//! the fixtures.

use std::path::Path;

/// The `[[bin]]` names of `fuzz/Cargo.toml`.
fn fuzz_targets() -> Vec<String> {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/Cargo.toml"))
            .expect("fuzz/Cargo.toml");
    let mut out = Vec::new();
    let mut in_bin = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_bin = line == "[[bin]]";
        } else if let (true, Some(name)) = (in_bin, line.strip_prefix("name = ")) {
            out.push(name.trim_matches('"').to_string());
        }
    }
    out
}

#[test]
fn the_nightly_workflow_fuzzes_every_target() {
    let workflow = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/nightly.yml"),
    )
    .expect("nightly.yml");
    let seeded = workflow
        .lines()
        .find_map(|l| l.trim().strip_prefix("for target in "))
        .and_then(|l| l.strip_suffix("; do"))
        .expect("the seeding loop");
    let targets = fuzz_targets();
    assert!(targets.len() >= 7, "{targets:?}");
    for t in &targets {
        assert!(
            workflow.contains(&format!("cargo fuzz run {t} ")),
            "the nightly workflow does not fuzz `{t}`"
        );
        assert!(
            seeded.split(' ').any(|s| s == t),
            "the nightly workflow does not seed `{t}`"
        );
    }
}
