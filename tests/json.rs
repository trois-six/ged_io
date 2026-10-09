//! The dataset in JSON (feature `serde`): its shape, locked by a snapshot,
//! and every fixture read back from its JSON with the same structures.
#![cfg(feature = "serde")]

use std::path::{Path, PathBuf};

use ged_io::model::{Dataset, RecordRef};
use ged_io::GedcomStreamParser;

/// Every `.ged` file under `dir`.
fn ged_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            ged_files(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ged")) {
            out.push(p);
        }
    }
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// The JSON of `tests/fixtures/json/dataset.ged` is the snapshot
/// `dataset.json` (`JSON_SNAPSHOT_WRITE=1` rewrites it): a change of the
/// model's JSON shape shows here.
#[test]
fn json_shape_is_the_snapshot() {
    let dir = fixtures().join("json");
    let data = Dataset::from_bytes(std::fs::read(dir.join("dataset.ged")).unwrap());
    let json = serde_json::to_string_pretty(&data).unwrap() + "\n";
    let snapshot = dir.join("dataset.json");
    if std::env::var_os("JSON_SNAPSHOT_WRITE").is_some() {
        std::fs::write(&snapshot, &json).unwrap();
    }
    let expected = std::fs::read_to_string(&snapshot).unwrap();
    assert!(json == expected, "the JSON shape changed:\n{json}");
}

/// The snapshot reads back as the dataset it was written from.
#[test]
fn the_snapshot_reads_back() {
    let dir = fixtures().join("json");
    let data = Dataset::from_bytes(std::fs::read(dir.join("dataset.ged")).unwrap());
    let back: Dataset =
        serde_json::from_str(&std::fs::read_to_string(dir.join("dataset.json")).unwrap()).unwrap();
    assert_eq!(back.to_structures(), data.to_structures());
    assert_eq!(back.version(), data.version());
    assert_eq!(back.declared_version(), Some("7.0"));
    let ann = back.find_individual("@I1@").unwrap();
    assert_eq!(ann.full_name(&back).as_deref(), Some("Ann Marie Example"));
    assert_eq!(back.families_as_spouse(ann.xref).count(), 1);
    assert!(back.dangling_references().is_empty());
}

/// Every fixture's dataset reads back from its JSON with the same
/// structures, and writes the same file.
#[test]
fn every_fixture_round_trips_through_json() {
    let mut files = Vec::new();
    ged_files(&fixtures(), &mut files);
    assert!(files.len() > 50);
    for path in files {
        let data = Dataset::from_bytes(std::fs::read(&path).unwrap());
        let json = serde_json::to_string(&data).unwrap();
        let back: Dataset =
            serde_json::from_str(&json).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(
            back.to_structures() == data.to_structures(),
            "{}",
            path.display()
        );
        let writer = ged_io::GedcomWriter::new();
        assert_eq!(
            writer.write_to_string(&back).unwrap(),
            writer.write_to_string(&data).unwrap(),
            "{}",
            path.display()
        );
    }
}

/// A streamed record serializes as its record, its version and its line.
#[test]
fn streamed_records_serialize() {
    let text = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NAME Ann /Example/\n1 SEX f\n0 TRLR\n";
    let records: Vec<_> = GedcomStreamParser::new(text.as_bytes())
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(matches!(records[1].record(), RecordRef::Individual(_)));
    assert_eq!(
        serde_json::to_string(&records[1]).unwrap(),
        r#"{"version":"5.5.1","line":4,"record":{"Individual":{"xref":"@I1@","names":[{"value":"Ann /Example/"}],"sex":"Female"}}}"#
    );
}

/// What JSON cannot hold for the dataset is refused, not guessed.
#[test]
fn malformed_json_is_an_error() {
    for json in [
        r#"{"version":"6.0"}"#,
        r#"{"individuals":[{"sex":"Robot"}]}"#,
        r#"{"individuals":[{"sex":{"Other":"x"}}]}"#,
        r#"{"individuals":{"xref":"@I1@"}}"#,
        r#"{"individuals":[{"names":[{"pieces":[{"kind":"Nickname","value":1}]}]}]}"#,
    ] {
        assert!(serde_json::from_str::<Dataset>(json).is_err(), "{json}");
    }
    // Fields it does not know are skipped.
    let data: Dataset = serde_json::from_str(r#"{"version":"7.0","comment":"x"}"#).unwrap();
    assert_eq!(data.version(), ged_io::GedcomVersion::V7_0);
}
