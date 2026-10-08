//! Writer output re-parses and follows the line grammar: for any input,
//! the typed model and the lossless tree read from it are written in each
//! version without panicking, every written line passes the line rules of
//! the version, and the output reads back with the same records.
#![no_main]

use ged_io::tree::{parse_tree, Tree};
use ged_io::{GedcomBuilder, GedcomVersion, GedcomWriter, OutputEncoding};
use libfuzzer_sys::fuzz_target;
use std::collections::HashSet;

const VERSIONS: [GedcomVersion; 3] = [
    GedcomVersion::V5_5_1,
    GedcomVersion::V7_0,
    GedcomVersion::V7_1,
];

/// Asserts the line rules the emitter guarantees.
fn check_lines(out: &str, version: GedcomVersion) {
    let rules = version.rules();
    assert!(out.ends_with('\n'), "no final terminator");
    let mut previous: Option<usize> = None;
    let mut xrefs = HashSet::new();
    for line in out.split_terminator('\n') {
        assert!(!line.contains('\r'), "CR inside a line: {line:?}");
        assert!(!line.chars().any(|c| rules.is_banned(c)), "banned: {line:?}");
        if let Some(max) = rules.max_line_length() {
            assert!(line.len() < max, "too long: {} {line:?}", line.len());
        }
        let mut parts = line.splitn(2, ' ');
        let level: usize = parts.next().unwrap().parse().expect("a level");
        let rest = parts.next().expect("a tag");
        match previous {
            None => assert_eq!(level, 0),
            Some(p) => assert!(level <= p + 1, "level jump: {line:?}"),
        }
        previous = Some(level);
        let (xref, rest) = match rest.strip_prefix('@') {
            Some(r) => {
                let end = r.find("@ ").expect("an xref") + 1;
                (Some(&rest[..=end]), &r[end + 1..])
            }
            None => (None, rest),
        };
        if let Some(x) = xref {
            assert_eq!(level, 0, "xref on a substructure: {line:?}");
            assert!(rules.is_valid_xref(x), "xref: {line:?}");
            assert!(xrefs.insert(x.to_string()), "duplicate xref: {line:?}");
        }
        let (tag, payload) = match rest.split_once(' ') {
            Some((t, p)) => (t, Some(p)),
            None => (rest, None),
        };
        assert!(rules.is_valid_tag(tag), "tag: {line:?}");
        assert!(!(tag == "CONC" && !rules.uses_conc()), "CONC: {line:?}");
        if let Some(p) = payload {
            assert!(!p.is_empty(), "trailing delimiter: {line:?}");
            if !rules.doubles_every_at_sign() && p.starts_with('@') && !p.starts_with("@@") {
                assert!(p.ends_with('@') && !p[1..p.len() - 1].contains('@'), "escape: {line:?}");
            }
        }
    }
    assert!(out.ends_with("0 TRLR\n"), "no trailer");
}

/// The number of level-0 lines.
fn records(out: &str) -> usize {
    out.split_terminator('\n').filter(|l| l.starts_with("0 ")).count()
}

fuzz_target!(|data: &[u8]| {
    // The typed model.
    if let Ok(model) = GedcomBuilder::new().build_from_bytes(data) {
        for version in VERSIONS {
            let writer = GedcomWriter::new().gedcom_version(version);
            let out = writer.write_to_string(&model).expect("repairs, never fails");
            check_lines(&out, version);
            let back = GedcomBuilder::new()
                .build_from_str(&out)
                .unwrap_or_else(|e| panic!("{e}\n{out}"));
            assert_eq!(back.individuals.len(), model.individuals.len());
            assert_eq!(back.families.len(), model.families.len());
            assert_eq!(back.sources.len(), model.sources.len());
            assert_eq!(back.repositories.len(), model.repositories.len());
            assert_eq!(back.multimedia.len(), model.multimedia.len());
            assert_eq!(back.shared_notes.len(), model.shared_notes.len());
            assert_eq!(back.custom_data.len(), model.custom_data.len());
            for encoding in [
                OutputEncoding::Ansel,
                OutputEncoding::Ascii,
                OutputEncoding::Utf16Le,
            ] {
                let _ = writer
                    .clone()
                    .output_encoding(encoding)
                    .write(std::io::sink(), &model);
            }
        }
    }

    // The lossless tree.
    let tree = Tree::from_bytes(data);
    let mut kept = 0;
    let mut head = false;
    for record in tree.records() {
        let empty_trailer = record.tag() == "TRLR"
            && record.payload() == ged_io::tree::PayloadRef::None
            && record.substructures().next().is_none();
        if record.tag() == "HEAD" && !head {
            head = true;
        } else if !empty_trailer {
            kept += 1;
        }
    }
    for version in VERSIONS {
        let mut bytes = Vec::new();
        GedcomWriter::new()
            .gedcom_version(version)
            .bom(ged_io::Bom::Never)
            .write_tree(&mut bytes, &tree)
            .expect("repairs, never fails");
        let out = String::from_utf8(bytes).expect("UTF-8");
        check_lines(&out, version);
        let written = records(&out) - 2; // HEAD and TRLR
        assert!(
            written == kept || (version.rules().max_line_length().is_some() && written == kept + 1),
            "{written} records written for {kept}"
        );
        assert_eq!(parse_tree(&out).records().count(), records(&out));
    }
});
