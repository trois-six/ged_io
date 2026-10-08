//! The writer's line guarantees, version by version: injection-proof text,
//! `@` escapes, line lengths, terminators, identifiers, header, encodings.

use ged_io::tree::{parse_tree, Tree};
use ged_io::types::custom::UserDefinedTag;
use ged_io::types::individual::Individual;
use ged_io::types::source::citation::{Citation, CitationSource};
use ged_io::types::GedcomData;
use ged_io::{
    Bom, GedcomBuilder, GedcomVersion, GedcomWriter, LineEnding, OutputEncoding, RepairKind,
    RepairPolicy, WriteError,
};

const VERSIONS: [GedcomVersion; 3] = [
    GedcomVersion::V5_5_1,
    GedcomVersion::V7_0,
    GedcomVersion::V7_1,
];

fn write(data: &GedcomData, version: GedcomVersion) -> String {
    GedcomWriter::new()
        .gedcom_version(version)
        .write_to_string(data)
        .unwrap()
}

fn read(text: &str) -> GedcomData {
    GedcomBuilder::new()
        .build_from_str(text)
        .unwrap_or_else(|e| panic!("{e}\n{text}"))
}

#[test]
fn test_text_in_pointer_fields_cannot_inject_lines() {
    // A pointer field that holds text, read with its continuation line.
    let original = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n1 SUBM X\n2 CONT 0 @I9@ INDI\n\
                    0 @I1@ INDI\n1 ALIA Y\n2 CONT 1 NAME Injected /Line/\n0 TRLR\n";
    let data = read(original);
    for version in VERSIONS {
        let written = write(&data, version);
        let back = read(&written);
        assert_eq!(back.individuals.len(), 1, "{written}");
        assert!(back.individuals[0].names.is_empty(), "{written}");
        assert_eq!(back.individuals[0].aliases, data.individuals[0].aliases);
    }
}

#[test]
fn test_model_values_cannot_inject_lines() {
    let mut data = GedcomData::default();
    let mut person = Individual {
        xref: Some("@I1@".into()),
        ..Individual::default()
    };
    person
        .aliases
        .push("Y\n0 @I666@ INDI\r1 NAME Injected /Line/".into());
    data.individuals.push(person);
    data.custom_data.push(Box::new(UserDefinedTag {
        xref: None,
        tag: "_NOTE".into(),
        value: Some("first\r\n0 TRLR".into()),
        children: Vec::new(),
    }));
    for version in VERSIONS {
        let written = write(&data, version);
        assert_eq!(
            written.lines().filter(|l| *l == "0 TRLR").count(),
            1,
            "{written}"
        );
        let back = read(&written);
        assert_eq!(back.individuals.len(), 1, "{written}");
        assert_eq!(
            back.individuals[0].aliases[0],
            "Y\n0 @I666@ INDI\n1 NAME Injected /Line/"
        );
    }
}

#[test]
fn test_headers_per_version() {
    let data = GedcomData::default();
    assert_eq!(
        write(&data, GedcomVersion::V7_0),
        "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 TRLR\n"
    );
    assert_eq!(
        write(&data, GedcomVersion::V7_1),
        "0 HEAD\n1 GEDC\n2 VERS 7.1\n0 TRLR\n"
    );
    assert_eq!(
        write(&data, GedcomVersion::V5_5_1),
        concat!(
            "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n2 FORM LINEAGE-LINKED\n1 CHAR UTF-8\n",
            "1 SOUR ged_io\n2 VERS ",
            env!("CARGO_PKG_VERSION"),
            "\n2 NAME ged_io\n1 SUBM @U1@\n0 @U1@ SUBM\n1 NAME Unknown\n0 TRLR\n"
        )
    );

    // A 5.5.1 header read from an ANSEL file: its CHAR names the input.
    let data = read(
        "0 HEAD\n1 SOUR EXAMPLE_APP\n1 SUBM @U7@\n1 GEDC\n2 VERS 5.5\n2 FORM Lineage-Linked\n\
         1 CHAR ANSEL\n2 VERS 1985\n0 @U7@ SUBM\n1 NAME Sample Submitter\n0 TRLR\n",
    );
    let written = write(&data, GedcomVersion::V5_5_1);
    assert!(written.starts_with(
        "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n2 FORM LINEAGE-LINKED\n1 CHAR UTF-8\n1 SOUR EXAMPLE_APP\n"
    ), "{written}");
    assert!(written.contains("1 SUBM @U7@\n"), "{written}");
    assert!(!written.contains("1985"), "{written}");
    let written = write(&data, GedcomVersion::V7_0);
    assert!(
        !written.contains("CHAR") && !written.contains("FORM"),
        "{written}"
    );
}

#[test]
fn test_every_line_ends_with_the_terminator() {
    let data = read("0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n0 TRLR");
    for (ending, eol) in [
        (LineEnding::Lf, "\n"),
        (LineEnding::CrLf, "\r\n"),
        (LineEnding::Cr, "\r"),
    ] {
        let written = GedcomWriter::new()
            .line_ending(ending)
            .write_to_string(&data)
            .unwrap();
        assert!(written.ends_with(&format!("0 TRLR{eol}")), "{written:?}");
        assert_eq!(written.matches(eol).count(), 5, "{written:?}");
    }
}

#[test]
fn test_at_signs_are_escaped_per_version() {
    let mut data = read("0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NOTE x\n0 TRLR");
    data.individuals[0].notes[0].value = Some("mail a@b.example\n@#DJULIAN@ and @#open".into());
    // A citation whose description looks like a pointer.
    let mut cited: Citation = read("0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 SOUR x\n0 TRLR")
        .individuals
        .remove(0)
        .source
        .remove(0);
    cited.source = CitationSource::Description("@S9@".into());
    data.individuals[0].source.push(cited);

    let v5 = write(&data, GedcomVersion::V5_5_1);
    assert!(
        v5.contains("1 NOTE mail a@@b.example\n2 CONT @#DJULIAN@ and @@#open\n"),
        "{v5}"
    );
    // A description that looks like a pointer is still text.
    assert!(v5.contains("1 SOUR @@S9@@\n"), "{v5}");
    let v7 = write(&data, GedcomVersion::V7_0);
    assert!(
        v7.contains("1 NOTE mail a@b.example\n2 CONT @@#DJULIAN@ and @#open\n"),
        "{v7}"
    );
    assert!(v7.contains("1 SOUR @@S9@\n"), "{v7}");

    for written in [v5, v7] {
        let back = read(&written);
        assert_eq!(back.individuals[0].notes, data.individuals[0].notes);
    }
}

#[test]
fn test_line_length_per_version() {
    let mut data = read("0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NOTE x\n0 TRLR");
    let note = "a reasonably long sentence about a fictitious person ".repeat(20);
    let note = note.trim_end().to_string();
    data.individuals[0].notes[0].value = Some(note.clone());

    let v7 = write(&data, GedcomVersion::V7_0);
    assert!(v7.contains(&format!("1 NOTE {note}\n")), "{v7}");
    assert!(!v7.contains("CONC"));

    for max in [255, 80] {
        let v5 = GedcomWriter::new()
            .max_line_length(max)
            .write_to_string(&data)
            .unwrap();
        assert!(v5.contains("\n2 CONC "), "{v5}");
        for line in v5.split_terminator('\n') {
            assert!(line.len() < max, "{} {line}", line.len());
            assert!(!line.ends_with(' '), "{line:?}");
        }
        assert_eq!(
            read(&v5).individuals[0].notes[0].value.as_deref(),
            Some(note.as_str())
        );
    }
}

#[test]
fn test_identifiers_are_valid_and_unique() {
    let data = read(
        "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 SEX M\n0 @I1@ INDI\n1 SEX F\n\
         0 @VOID@ INDI\n0 @i2@ INDI\n1 FAMS @f1@\n0 @f1@ FAM\n1 WIFE @i2@\n1 HUSB @VOID@\n0 TRLR",
    );
    let mut bytes = Vec::new();
    let report = GedcomWriter::new()
        .bom(Bom::Never)
        .write(&mut bytes, &data)
        .unwrap();
    let written = String::from_utf8(bytes).unwrap();
    for line in [
        "0 @I1@ INDI\n1 SEX M\n",
        "0 @I1_2@ INDI\n1 SEX F\n",
        "0 @VOID_@ INDI\n",
        "0 @I2@ INDI\n1 FAMS @F1@\n",
        "0 @F1@ FAM\n1 HUSB @VOID@\n1 WIFE @I2@\n",
    ] {
        assert!(written.contains(line), "{line:?} in\n{written}");
    }
    assert_eq!(report.repairs.len(), 4, "{:?}", report.repairs);
    assert!(report
        .repairs
        .iter()
        .all(|r| r.kind == RepairKind::Xref && r.line == 0));
    assert!(report
        .repairs
        .iter()
        .any(|r| &*r.detail == "identifier @i2@ written as @I2@"));

    let error = GedcomWriter::new()
        .on_nonconformant(RepairPolicy::Error)
        .write_to_string(&data)
        .unwrap_err();
    assert!(matches!(
        error,
        WriteError::NonConformant(ref r) if r.kind == RepairKind::Xref
    ));
}

#[test]
fn test_records_without_identifiers_get_one() {
    let mut data = GedcomData::default();
    data.individuals.push(Individual::default());
    data.individuals.push(Individual {
        xref: Some("@I1@".into()),
        ..Individual::default()
    });
    let written = write(&data, GedcomVersion::V7_0);
    assert!(written.contains("0 @I2@ INDI\n0 @I1@ INDI\n"), "{written}");
}

#[test]
fn test_deep_nesting_and_banned_characters() {
    let mut tag = UserDefinedTag {
        xref: None,
        tag: "_X".into(),
        value: Some("bell\u{7} and\u{85} more".into()),
        children: Vec::new(),
    };
    for _ in 0..300 {
        tag = UserDefinedTag {
            xref: None,
            tag: "_X".into(),
            value: None,
            children: vec![Box::new(tag)],
        };
    }
    let mut data = GedcomData::default();
    data.custom_data.push(Box::new(tag));
    for version in VERSIONS {
        let mut bytes = Vec::new();
        let report = GedcomWriter::new()
            .gedcom_version(version)
            .write(&mut bytes, &data)
            .unwrap();
        let written = String::from_utf8(bytes).unwrap();
        assert!(written.contains(" _X bell and more\n"), "{version}");
        let deepest = written
            .lines()
            .filter_map(|l| {
                l.trim_start_matches('\u{feff}')
                    .split(' ')
                    .next()?
                    .parse::<usize>()
                    .ok()
            })
            .max()
            .unwrap();
        assert!(
            deepest <= if version.is_v7() { 254 } else { 98 },
            "{deepest}"
        );
        assert!(report
            .repairs
            .iter()
            .any(|r| r.kind == RepairKind::Characters
                && r.detail.contains("2 banned character(s) left out")));
        assert!(report.repairs.iter().any(|r| r.kind == RepairKind::Level));
    }
}

#[test]
fn test_output_encodings() {
    let data = read("0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NAME Zoé /Exemple/\n0 TRLR");

    let mut ansel = Vec::new();
    GedcomWriter::new()
        .output_encoding(OutputEncoding::Ansel)
        .write(&mut ansel, &data)
        .unwrap();
    assert!(ansel.windows(12).any(|w| w == b"1 CHAR ANSEL"));
    assert!(!ansel.contains(&0xC3), "UTF-8 bytes in ANSEL output");
    let back = GedcomBuilder::new().build_from_bytes(&ansel).unwrap();
    assert_eq!(back.individuals[0].names, data.individuals[0].names);

    let mut utf16 = Vec::new();
    GedcomWriter::new()
        .output_encoding(OutputEncoding::Utf16Le)
        .write(&mut utf16, &data)
        .unwrap();
    assert!(utf16.starts_with(&[0xFF, 0xFE, b'0', 0]));
    let back = GedcomBuilder::new().build_from_bytes(&utf16).unwrap();
    assert_eq!(back.individuals[0].names, data.individuals[0].names);
    assert_eq!(
        back.header.unwrap().encoding.unwrap().value.as_deref(),
        Some("UNICODE")
    );

    let error = GedcomWriter::new()
        .output_encoding(OutputEncoding::Ascii)
        .write(Vec::new(), &data)
        .unwrap_err();
    assert!(matches!(error, WriteError::Unencodable(ref u) if u.character == 'é'));

    // 7.x is UTF-8 whatever the setting, with a byte order mark by default.
    let mut v7 = Vec::new();
    GedcomWriter::new()
        .gedcom_version(GedcomVersion::V7_0)
        .output_encoding(OutputEncoding::Ansel)
        .write(&mut v7, &data)
        .unwrap();
    assert!(v7.starts_with("\u{feff}0 HEAD\n1 GEDC\n2 VERS 7.0\n".as_bytes()));
    assert!(String::from_utf8(v7).unwrap().contains("Zoé"));
    // No mark in a string, unless asked for.
    let text = write(&data, GedcomVersion::V7_0);
    assert!(text.starts_with("0 HEAD"));
    let text = GedcomWriter::new()
        .gedcom_version(GedcomVersion::V7_0)
        .bom(Bom::Always)
        .write_to_string(&data)
        .unwrap();
    assert!(text.starts_with('\u{feff}'));
}

#[test]
fn test_trees_are_written_under_the_line_rules() {
    let tree = Tree::parse(
        "0 HEAD\n1 GEDC\n2 VERS 7.0\n2 FORM LINEAGE-LINKED\n1 CHAR UTF-8\n\
         0 @i1@ INDI\n1 _lower text\n1 NOTE @@x\n2 CONT y\n1 FAMC @f1@\n2 _X @i1@\n\
         0 @f1@ FAM\n1 CHIL @i1@\n0 TRLR\n",
    );
    let mut bytes = Vec::new();
    let report = GedcomWriter::new()
        .bom(Bom::Never)
        .write_tree(&mut bytes, &tree)
        .unwrap();
    let written = String::from_utf8(bytes).unwrap();
    assert_eq!(
        written,
        "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 _LOWER text\n1 NOTE @@x\n2 CONT y\n\
         1 FAMC @F1@\n2 _X @I1@\n0 @F1@ FAM\n1 CHIL @I1@\n0 TRLR\n"
    );
    assert_eq!(report.repairs.len(), 3, "{:?}", report.repairs);

    // The same records, owned, written as 5.5.1.
    let records = tree.to_structures();
    let mut bytes = Vec::new();
    GedcomWriter::new()
        .gedcom_version(GedcomVersion::V5_5_1)
        .write_structures(&mut bytes, &records)
        .unwrap();
    let written = String::from_utf8(bytes).unwrap();
    assert!(
        written.contains("0 @i1@ INDI\n1 _lower text\n"),
        "{written}"
    );
    assert!(written.contains("1 SUBM @U1@\n"), "{written}");
    assert_eq!(parse_tree(&written).records().count(), 5, "{written}");
}

#[test]
fn test_extension_tags_under_record_text_read_back() {
    // An extension tag right below the text of a note record, as the
    // writer makes from an unknown tag there (found by fuzzing).
    let original = "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @N1@ SNOTE A shared note\n1 CONT more\n\
                    1 _X kept\n0 @N2@ SNOTE Another\n1 Sou\u{b5}rce kept too\n0 TRLR\n";
    let data = read(original);
    for version in VERSIONS {
        let written = write(&data, version);
        let back = read(&written);
        assert_eq!(back.shared_notes.len(), 2, "{written}");
        assert_eq!(back.shared_notes[0].text, "A shared note\nmore");
    }
}

#[test]
fn test_long_tags_fit_the_551_line() {
    let mut data = GedcomData::default();
    data.custom_data.push(Box::new(UserDefinedTag {
        xref: Some("@X1@".into()),
        tag: "-".repeat(300),
        value: Some("value".into()),
        children: Vec::new(),
    }));
    let written = write(&data, GedcomVersion::V5_5_1);
    let line = written.lines().find(|l| l.starts_with("0 @X1@")).unwrap();
    assert_eq!(line, format!("0 @X1@ _{} value", "_".repeat(30)));
}

#[test]
fn test_valid_551_identifiers_are_kept_as_they_are() {
    // 5.5.1 identifiers may hold spaces, `!` and `:` (p. 13) and start with
    // `_`: valid ones are neither rewritten nor taken as text.
    let original = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I 1@ INDI\n1 FAMS @F:1@\n1 ASSO @_I2!1@\n\
                    2 RELA godparent\n0 @F:1@ FAM\n1 HUSB @I 1@\n0 @_I2@ INDI\n0 TRLR\n";
    let mut bytes = Vec::new();
    let report = GedcomWriter::new()
        .write_tree(&mut bytes, &Tree::parse(original))
        .unwrap();
    let written = String::from_utf8(bytes).unwrap();
    for line in [
        "0 @I 1@ INDI\n1 FAMS @F:1@\n1 ASSO @_I2!1@\n",
        "0 @F:1@ FAM\n1 HUSB @I 1@\n",
        "0 @_I2@ INDI\n",
    ] {
        assert!(written.contains(line), "{line:?} in\n{written}");
    }
    assert!(report.repairs.is_empty(), "{:?}", report.repairs);

    // The typed model reads no space in an identifier; the other forms
    // are kept the same way.
    let data = read(&original.replace("I 1", "I1"));
    let v5 = write(&data, GedcomVersion::V5_5_1);
    assert!(v5.contains("0 @I1@ INDI\n1 FAMS @F:1@\n"), "{v5}");
    assert!(
        v5.contains("0 @_I2@ INDI\n0 @F:1@ FAM\n1 HUSB @I1@\n"),
        "{v5}"
    );
    // 7.0 has none of them.
    let v7 = write(&data, GedcomVersion::V7_0);
    assert!(v7.contains("0 @I1@ INDI\n1 FAMS @F_1@\n"), "{v7}");
    assert!(v7.contains("0 @F_1@ FAM\n"), "{v7}");
}

#[test]
fn test_tabs_are_spaces_in_551_and_kept_in_7() {
    let mut data = read("0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NOTE x\n0 TRLR");
    data.individuals[0].notes[0].value = Some("a\tb".into());
    let mut bytes = Vec::new();
    let report = GedcomWriter::new().write(&mut bytes, &data).unwrap();
    assert!(String::from_utf8(bytes).unwrap().contains("1 NOTE a b\n"));
    assert_eq!(report.repairs.len(), 1);
    assert_eq!(report.repairs[0].kind, RepairKind::Characters);
    assert!(write(&data, GedcomVersion::V7_0).contains("1 NOTE a\tb\n"));
}
