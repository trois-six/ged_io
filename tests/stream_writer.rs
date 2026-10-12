//! The streaming writer writes what the in-memory writer writes, record by
//! record, on every fixture where knowing the records to come changes
//! nothing; elsewhere, only in the ways its documentation lists. Its output
//! conforms, but for the pointers it cannot check.

use ged_io::model::Header;
use ged_io::spec::{validate_text, DeviationKind};
use ged_io::{
    Bom, Dataset, GedcomStreamWriter, GedcomVersion, GedcomWriter, RepairKind, RepairPolicy,
    WriteReport, WriterConfig,
};
use std::path::{Path, PathBuf};

fn ged_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            ged_files(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ged")) {
            out.push(p);
        }
    }
}

/// `data` written by the streaming writer, its records in the order the
/// in-memory writer writes them.
fn streamed(data: &Dataset, config: WriterConfig) -> (String, WriteReport) {
    let default = Header::default();
    let header = data.header.as_ref().unwrap_or(&default);
    let mut w = GedcomStreamWriter::new(Vec::new(), config, header, data).unwrap();
    for r in data.records().skip(usize::from(data.header.is_some())) {
        w.write_record(r, data).unwrap();
    }
    let (bytes, report) = w.finish().unwrap();
    (String::from_utf8(bytes).unwrap(), report)
}

/// Why the outputs may differ, from the repairs: one of the cases the
/// streaming writer documents.
fn documented(in_memory: &WriteReport, stream: &WriteReport, stream_text: &str) -> bool {
    let renamed = |r: &WriteReport| r.repairs.iter().any(|r| r.kind == RepairKind::Xref);
    let pointer = in_memory
        .repairs
        .iter()
        .any(|r| r.kind == RepairKind::Pointer);
    let relocated = in_memory
        .repairs
        .iter()
        .any(|r| r.detail.contains("(declared as "));
    let large = in_memory
        .repairs
        .iter()
        .any(|r| r.kind == RepairKind::RecordSize);
    let stub = stream_text.contains("0 @U1@ SUBM\n1 NAME Unknown\n");
    renamed(in_memory) || renamed(stream) || pointer || relocated || large || stub
}

#[test]
fn streaming_writes_as_the_in_memory_writer() {
    let mut files = Vec::new();
    ged_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        &mut files,
    );
    files.sort();
    let (mut same, mut different) = (0, 0);
    for path in files {
        let name = path.display();
        let data = Dataset::from_bytes(std::fs::read(&path).unwrap());
        for version in [
            GedcomVersion::V5_5_1,
            GedcomVersion::V7_0,
            GedcomVersion::V7_1,
        ] {
            let config = WriterConfig {
                version: Some(version),
                bom: Bom::Never,
                ..WriterConfig::default()
            };
            let mut bytes = Vec::new();
            let report = GedcomWriter::with_config(config.clone())
                .write(&mut bytes, &data)
                .unwrap();
            let in_memory = String::from_utf8(bytes).unwrap();
            let (stream, stream_report) = streamed(&data, config);
            if stream == in_memory {
                // The same repairs; renames come first in memory, where
                // the record is in a stream.
                let sorted = |r: &WriteReport| {
                    let mut d: Vec<String> = r.repairs.iter().map(ToString::to_string).collect();
                    d.sort();
                    d
                };
                assert_eq!(
                    sorted(&stream_report),
                    sorted(&report),
                    "{name} as {version}"
                );
                same += 1;
            } else {
                assert!(
                    documented(&report, &stream_report, &stream),
                    "{name} as {version}:\n{in_memory}\n---\n{stream}"
                );
                different += 1;
            }
            // Every line and structure conforms; a pointer may name no
            // record, or one of another type, and a 5.5.1 record without
            // notes to move may stay over 32K, as in memory.
            let left: Vec<_> = validate_text(&stream)
                .into_iter()
                .filter(|d| {
                    !matches!(
                        d.kind,
                        DeviationKind::PointerTarget
                            | DeviationKind::DanglingPointer
                            | DeviationKind::Xref
                            | DeviationKind::RecordSize
                    )
                })
                .collect();
            assert!(left.is_empty(), "{name} as {version}: {left:?}\n{stream}");
        }
    }
    assert!(same > 150 && different < 25, "{same} {different}");
}

/// Under the strict policy, the first repair is an error, as soon as the
/// record that needs it is written.
#[test]
fn strict_streaming_fails_on_the_first_repair() {
    let data = Dataset::parse(
        "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 SEX M\n0 @I2@ INDI\n1 SEX X\n1 SEX F\n0 TRLR\n",
    );
    let config = WriterConfig {
        on_nonconformant: RepairPolicy::Error,
        ..WriterConfig::default()
    };
    let header = data.header.as_ref().unwrap();
    let mut w = GedcomStreamWriter::new(Vec::new(), config, header, &data).unwrap();
    let mut records = data.records().skip(1);
    w.write_record(records.next().unwrap(), &data).unwrap();
    assert!(w.write_record(records.next().unwrap(), &data).is_err());
}

/// Identifiers stay unique and valid, record by record: a repeated one is
/// renamed, an invalid one written in its valid form, and pointers to it
/// follow; a record without one gets the first free one.
#[test]
fn identifiers_are_kept_unique_record_by_record() {
    let data = Dataset::parse(
        "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @i1@ INDI\n1 FAMS @F1@\n0 @F1@ FAM\n1 HUSB @i1@\n0 @F1@ FAM\n0 INDI\n0 TRLR\n",
    );
    let (text, report) = streamed(&data, WriterConfig::default());
    assert!(text.contains("0 @I1@ INDI\n"), "{text}");
    assert!(text.contains("1 HUSB @I1@\n"), "{text}");
    assert!(text.contains("0 @F1_2@ FAM\n"), "{text}");
    assert!(text.contains("0 @I2@ INDI\n"), "{text}");
    assert_eq!(
        report
            .repairs
            .iter()
            .filter(|r| r.kind == RepairKind::Xref)
            .count(),
        2,
        "{report:?}"
    );
}

/// Streaming is idempotent: what it wrote, read back and streamed again,
/// is the same bytes.
#[test]
fn streaming_is_idempotent() {
    let mut files = Vec::new();
    ged_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        &mut files,
    );
    files.sort();
    for path in files {
        let name = path.display();
        let data = Dataset::from_bytes(std::fs::read(&path).unwrap());
        for version in [
            GedcomVersion::V5_5_1,
            GedcomVersion::V7_0,
            GedcomVersion::V7_1,
        ] {
            let config = WriterConfig {
                version: Some(version),
                bom: Bom::Never,
                ..WriterConfig::default()
            };
            let (once, _) = streamed(&data, config.clone());
            let (twice, _) = streamed(&Dataset::parse(once.as_str()), config);
            assert!(once == twice, "{name} as {version}:\n{once}\n---\n{twice}");
        }
    }
}

/// A 5.5.1 header whose `SUBM` the repair makes an extension still points
/// to a submitter, the stub (fuzz case).
#[test]
fn a_repaired_header_still_has_a_submitter() {
    let data = Dataset::from_bytes(&b"0 HEAD\n1 SUBM @\x001@\n0 TRLR\n"[..]);
    let config = WriterConfig {
        version: Some(GedcomVersion::V5_5_1),
        ..WriterConfig::default()
    };
    let (once, _) = streamed(&data, config.clone());
    assert!(once.contains("\n1 SUBM @U1@\n"), "{once}");
    assert!(once.contains("\n0 @U1@ SUBM\n1 NAME Unknown\n"), "{once}");
    let (twice, _) = streamed(&Dataset::parse(once.as_str()), config);
    assert_eq!(once, twice);
}
