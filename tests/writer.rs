//! The writer's line guarantees, version by version: headers, text that
//! cannot inject lines, `@` escapes, line lengths (whole lines, terminator
//! included), line terminators, byte order marks, output encodings,
//! identifiers, levels and banned characters, and the repairs it reports.
//! Every name and value is fictitious.

use ged_io::model::{
    Citation, CitationSource, Individual, Node, Note, NoteContent, Record, SharedNote, Source,
    Value,
};
use ged_io::model::{Text, XrefId};
use ged_io::tree::{parse_tree, Tree};
use ged_io::{
    Bom, Dataset, GedcomBuilder, GedcomVersion, GedcomWriter, LineEnding, OutputEncoding,
    RepairKind, RepairPolicy, WriteError,
};

const VERSIONS: [GedcomVersion; 3] = [
    GedcomVersion::V5_5_1,
    GedcomVersion::V7_0,
    GedcomVersion::V7_1,
];

fn write(data: &Dataset, version: GedcomVersion) -> String {
    GedcomWriter::new()
        .gedcom_version(version)
        .write_to_string(data)
        .unwrap()
}

fn read(text: &str) -> Dataset {
    GedcomBuilder::new()
        .build_from_str(text)
        .unwrap_or_else(|e| panic!("{e}\n{text}"))
}

/// The text of a note.
fn note_text(data: &Dataset, note: &Note) -> String {
    match &note.content {
        NoteContent::Text(text) => text.to_str(data).into_owned(),
        NoteContent::Shared(_) => panic!("a shared note"),
    }
}

/// An untyped structure with a text payload.
fn node(data: &mut Dataset, tag: &str, text: Option<&str>) -> Node {
    let mut node = Node::new(data.store_mut().intern_tag(tag));
    if let Some(text) = text {
        node.payload = Value::Text(Text::new(text));
    }
    node
}

fn xref(data: &mut Dataset, xref: &str) -> Option<XrefId> {
    data.store_mut().intern_xref(xref)
}

// Headers

#[test]
fn headers_per_version() {
    let data = Dataset::default();
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
    assert!(
        written.starts_with(
            "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n2 FORM LINEAGE-LINKED\n1 CHAR UTF-8\n1 SOUR EXAMPLE_APP\n"
        ),
        "{written}"
    );
    assert!(written.contains("1 SUBM @U7@\n"), "{written}");
    assert!(!written.contains("1985"), "{written}");
    let written = write(&data, GedcomVersion::V7_0);
    assert!(
        !written.contains("CHAR") && !written.contains("FORM"),
        "{written}"
    );
}

// Text cannot inject lines

#[test]
fn text_in_pointer_fields_cannot_inject_lines() {
    // Pointer fields that hold text, read with their continuation lines:
    // 5.5.1 keeps the text as an extension, 7.x as the pointer's phrase.
    let original = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n1 SUBM X\n2 CONT 0 @I9@ INDI\n\
                    0 @I1@ INDI\n1 ALIA Y\n2 CONT 1 NAME Injected /Line/\n0 TRLR\n";
    let data = read(original);
    let alias = |data: &Dataset| {
        let records = data.to_structures();
        let person = records.iter().find(|s| s.tag.as_str() == "INDI")?;
        let alias = person.first("_ALIA").or_else(|| person.first("ALIA"))?;
        alias
            .text()
            .or_else(|| alias.first("PHRASE")?.text())
            .map(str::to_string)
    };
    assert_eq!(alias(&data).as_deref(), Some("Y\n1 NAME Injected /Line/"));
    for version in VERSIONS {
        let written = write(&data, version);
        assert!(!written.contains("\n0 @I9@"), "{written}");
        let back = read(&written);
        assert_eq!(back.individuals().len(), 1, "{written}");
        assert!(back.individuals()[0].names.is_empty(), "{written}");
        assert_eq!(alias(&back), alias(&data), "{written}");
    }
}

#[test]
fn model_values_cannot_inject_lines() {
    let mut data = Dataset::default();
    let mut person = Individual {
        xref: xref(&mut data, "@I1@"),
        ..Individual::default()
    };
    person
        .notes
        .push(Note::text("Y\n0 @I666@ INDI\r1 NAME Injected /Line/"));
    data.push(Record::Individual(Box::new(person)));
    let extension = node(&mut data, "_NOTE", Some("first\r\n0 TRLR"));
    data.push(Record::Other(Box::new(extension)));
    for version in VERSIONS {
        let written = write(&data, version);
        assert_eq!(
            written.lines().filter(|l| *l == "0 TRLR").count(),
            1,
            "{written}"
        );
        let back = read(&written);
        assert_eq!(back.individuals().len(), 1, "{written}");
        assert_eq!(
            note_text(&back, &back.individuals()[0].notes[0]),
            "Y\n0 @I666@ INDI\n1 NAME Injected /Line/"
        );
    }
}

// Escapes, lengths, terminators

#[test]
fn at_signs_are_escaped_per_version() {
    let mut data = read("0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NOTE x\n0 TRLR");
    let note = "mail a@b.example\n@#DJULIAN@ and @#open";
    data.get_mut_at::<Individual>(0).unwrap().notes[0] = Note::text(note);
    // A citation whose description looks like a pointer.
    data.get_mut_at::<Individual>(0)
        .unwrap()
        .citations
        .push(Citation {
            source: CitationSource::Description(Text::new("@S9@")),
            ..Citation::default()
        });

    let v5 = write(&data, GedcomVersion::V5_5_1);
    assert!(
        v5.contains("1 NOTE mail a@@b.example\n2 CONT @#DJULIAN@ and @@#open\n"),
        "{v5}"
    );
    // A description that looks like a pointer is still text.
    assert!(v5.contains("1 SOUR @@S9@@\n"), "{v5}");
    // 7.x data: a leading `@` doubled, an escape is text.
    let mut data7 = read("0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 NOTE x\n0 TRLR");
    data7.get_mut_at::<Individual>(0).unwrap().notes[0] = Note::text(note);
    let v7 = write(&data7, GedcomVersion::V7_0);
    assert!(
        v7.contains("1 NOTE mail a@b.example\n2 CONT @@#DJULIAN@ and @#open\n"),
        "{v7}"
    );
    for written in [v5, v7] {
        let back = read(&written);
        assert_eq!(note_text(&back, &back.individuals()[0].notes[0]), note);
    }

    // The 5.5.1 data converted: 7.x has no escapes, a calendar's is its
    // name; a source description is the note of a source record.
    let converted = write(&data, GedcomVersion::V7_0);
    assert!(
        converted.contains("1 NOTE mail a@b.example\n2 CONT JULIAN and @#open\n"),
        "{converted}"
    );
    assert!(converted.contains("1 SOUR @S1@\n"), "{converted}");
    assert!(
        converted.contains("0 @S1@ SOUR\n1 NOTE @@S9@\n"),
        "{converted}"
    );
}

#[test]
fn line_length_per_version() {
    let mut data = read("0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NOTE x\n0 TRLR");
    let note = "a reasonably long sentence about a fictitious person ".repeat(20);
    let note = note.trim_end().to_string();
    data.get_mut_at::<Individual>(0).unwrap().notes[0] = Note::text(note.as_str());

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
        let back = read(&v5);
        assert_eq!(note_text(&back, &back.individuals()[0].notes[0]), note);
    }
}

/// Words of every length, so that the split points fall in many places.
fn long_text(len: usize) -> String {
    let mut text = String::new();
    let mut word = 1;
    while text.len() < len {
        text.push_str(&"x".repeat(word));
        text.push(' ');
        word = word % 11 + 1;
    }
    text.truncate(len);
    text.trim_end().to_string()
}

/// `max_line_length` caps a whole written line — level, xref, tag, value,
/// delimiters and terminator — as GEDCOM 5.5.1 caps it at 255 characters.
#[test]
fn whole_line_length_includes_level_xref_tag_and_terminator() {
    let source = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NAME Ann /Example/\n\
                  1 NOTE x\n1 BIRT\n2 SOUR @S1@\n0 @S1@ SOUR\n1 TITL x\n\
                  0 @N123456789@ NOTE x\n0 TRLR";
    let mut data = read(source);
    let note = long_text(700);
    // A `CONC` line whose value starts with `@`, which is written as `@@`.
    let at_sign = format!("{}@{}", "a".repeat(247), "b".repeat(300));
    let multi_line = format!("{}\n{}", long_text(400), long_text(300));
    let page = long_text(500);
    // A record line carries its xref: `0 @N123456789@ NOTE …`.
    let shared_note = long_text(600);
    data.get_mut_at::<Individual>(0).unwrap().notes[0] = Note::text(note.as_str());
    data.get_mut_at::<Individual>(0)
        .unwrap()
        .notes
        .push(Note::text(at_sign.as_str()));
    data.get_mut_at::<Source>(0).unwrap().title = Some(Text::new(multi_line.as_str()));
    data.get_mut_at::<Individual>(0).unwrap().events[0].citations[0].page =
        Some(Text::new(page.as_str()));
    data.get_mut_at::<SharedNote>(0).unwrap().text = Text::new(shared_note.as_str());

    for (ending, eol, max) in [
        (LineEnding::Lf, "\n", 255),
        (LineEnding::CrLf, "\r\n", 255),
        (LineEnding::Lf, "\n", 80),
    ] {
        let written = GedcomWriter::new()
            .line_ending(ending)
            .max_line_length(max)
            .write_to_string(&data)
            .unwrap();
        assert!(written.contains(" CONC "), "{written}");
        for line in written.split(eol) {
            // Every line but the last (`0 TRLR`) is followed by its terminator.
            assert!(
                line.len() + eol.len() <= max,
                "a line of {} characters with its terminator: {line:?}",
                line.len() + eol.len()
            );
        }

        let back = read(&written);
        let person = &back.individuals()[0];
        assert_eq!(note_text(&back, &person.notes[0]), note);
        assert_eq!(note_text(&back, &person.notes[1]), at_sign);
        assert_eq!(
            back.sources()[0].title.as_ref().unwrap().to_str(&back),
            multi_line
        );
        assert_eq!(
            person.events[0].citations[0]
                .page
                .as_ref()
                .unwrap()
                .to_str(&back),
            page
        );
        assert_eq!(back.notes()[0].text.to_str(&back), shared_note);
    }
}

#[test]
fn every_line_ends_with_the_terminator() {
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
        if ending != LineEnding::CrLf {
            assert_eq!(written.matches(['\r', '\n']).count(), 5, "{written:?}");
        }
    }
}

#[test]
fn tabs_are_spaces_in_551_and_kept_in_7() {
    let mut data = read("0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NOTE x\n0 TRLR");
    data.get_mut_at::<Individual>(0).unwrap().notes[0] = Note::text("a\tb");
    let mut bytes = Vec::new();
    let report = GedcomWriter::new().write(&mut bytes, &data).unwrap();
    assert!(String::from_utf8(bytes).unwrap().contains("1 NOTE a b\n"));
    assert_eq!(report.repairs.len(), 1, "{:?}", report.repairs);
    assert_eq!(report.repairs[0].kind, RepairKind::Characters);
    assert!(write(&data, GedcomVersion::V7_0).contains("1 NOTE a\tb\n"));
}

// Byte order marks and output encodings

#[test]
fn byte_order_marks() {
    let data = read("0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n0 TRLR");
    let bytes = |version, bom| {
        let mut out = Vec::new();
        GedcomWriter::new()
            .gedcom_version(version)
            .bom(bom)
            .write(&mut out, &data)
            .unwrap();
        out
    };
    let mark = "\u{feff}".as_bytes();
    // By default: a mark for 7.x, none for 5.5.1 in UTF-8.
    assert!(bytes(GedcomVersion::V7_0, Bom::default()).starts_with(mark));
    assert!(bytes(GedcomVersion::V7_1, Bom::default()).starts_with(mark));
    assert!(bytes(GedcomVersion::V5_5_1, Bom::default()).starts_with(b"0 HEAD"));
    for version in VERSIONS {
        assert!(bytes(version, Bom::Always).starts_with(mark), "{version}");
        assert!(
            bytes(version, Bom::Never).starts_with(b"0 HEAD"),
            "{version}"
        );
    }
    // No mark in a string, unless asked for.
    assert!(write(&data, GedcomVersion::V7_0).starts_with("0 HEAD"));
    let text = GedcomWriter::new()
        .gedcom_version(GedcomVersion::V7_0)
        .bom(Bom::Always)
        .write_to_string(&data)
        .unwrap();
    assert!(text.starts_with('\u{feff}'));
}

#[test]
fn output_encodings() {
    let data = read("0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NAME Zoé /Exemple/\n0 TRLR");
    let name = |data: &Dataset| {
        data.individuals()[0].names[0]
            .value
            .to_str(data)
            .into_owned()
    };

    let mut ansel = Vec::new();
    GedcomWriter::new()
        .output_encoding(OutputEncoding::Ansel)
        .write(&mut ansel, &data)
        .unwrap();
    assert!(ansel.windows(12).any(|w| w == b"1 CHAR ANSEL"));
    assert!(!ansel.contains(&0xC3), "UTF-8 bytes in ANSEL output");
    let back = GedcomBuilder::new().build_from_bytes(ansel).unwrap();
    assert_eq!(name(&back), "Zoé /Exemple/");

    let mut utf16 = Vec::new();
    GedcomWriter::new()
        .output_encoding(OutputEncoding::Utf16Le)
        .write(&mut utf16, &data)
        .unwrap();
    assert!(utf16.starts_with(&[0xFF, 0xFE, b'0', 0]));
    let back = GedcomBuilder::new().build_from_bytes(utf16).unwrap();
    assert_eq!(name(&back), "Zoé /Exemple/");
    assert_eq!(
        back.header()
            .as_ref()
            .unwrap()
            .charset
            .as_ref()
            .unwrap()
            .value,
        ged_io::model::CharacterSet::Unicode
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
}

// Identifiers, levels and characters

#[test]
fn identifiers_are_valid_and_unique() {
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
        "0 @F1@ FAM\n",
        // A 7.x pointer to `@VOID@` stays the null pointer, though the
        // record named `@VOID@` was renamed.
        "1 HUSB @VOID@\n",
        "1 WIFE @I2@\n",
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
fn records_without_identifiers_get_one() {
    let mut data = Dataset::default();
    data.push(Record::Individual(Box::default()));
    let xref = xref(&mut data, "@I1@");
    data.push(Record::Individual(Box::new(Individual {
        xref,
        ..Individual::default()
    })));
    let written = write(&data, GedcomVersion::V7_0);
    assert!(written.contains("0 @I2@ INDI\n0 @I1@ INDI\n"), "{written}");
}

#[test]
fn valid_551_identifiers_are_kept_as_they_are() {
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

    // The typed model keeps them too.
    let data = read(original);
    let v5 = write(&data, GedcomVersion::V5_5_1);
    for line in [
        "0 @I 1@ INDI\n1 FAMS @F:1@\n",
        "0 @_I2@ INDI\n",
        "0 @F:1@ FAM\n1 HUSB @I 1@\n",
    ] {
        assert!(v5.contains(line), "{line:?} in\n{v5}");
    }
    // 7.0 has none of them.
    let v7 = write(&data, GedcomVersion::V7_0);
    assert!(v7.contains("0 @I_1@ INDI\n1 FAMS @F_1@\n"), "{v7}");
    assert!(v7.contains("0 @F_1@ FAM\n"), "{v7}");
}

#[test]
fn deep_nesting_and_banned_characters() {
    let mut data = Dataset::default();
    let mut tag = node(&mut data, "_X", Some("bell\u{7} and\u{85} more"));
    for _ in 0..300 {
        let mut parent = node(&mut data, "_X", None);
        parent.children.push(tag);
        tag = parent;
    }
    data.push(Record::Other(Box::new(tag)));
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
fn long_tags_fit_the_551_line() {
    let mut data = Dataset::default();
    let mut record = node(&mut data, &"-".repeat(300), Some("value"));
    record.xref = xref(&mut data, "@X1@");
    data.push(Record::Other(Box::new(record)));
    let written = write(&data, GedcomVersion::V5_5_1);
    let line = written.lines().find(|l| l.starts_with("0 @X1@")).unwrap();
    assert_eq!(line, format!("0 @X1@ _{} value", "_".repeat(30)));
}

#[test]
fn extension_tags_under_record_text_read_back() {
    // An extension tag right below the text of a note record, as the
    // writer makes from an unknown tag there (found by fuzzing).
    let original = "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @N1@ SNOTE A shared note\n1 CONT more\n\
                    1 _X kept\n0 @N2@ SNOTE Another\n1 Sou\u{b5}rce kept too\n0 TRLR\n";
    let data = read(original);
    for version in VERSIONS {
        let written = write(&data, version);
        let back = read(&written);
        assert_eq!(back.notes().len(), 2, "{written}");
        assert_eq!(back.notes()[0].text.to_str(&back), "A shared note\nmore");
        assert_eq!(back.notes()[1].text.to_str(&back), "Another");
    }
}

// Trees and structures

#[test]
fn trees_are_written_under_the_line_rules() {
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

/// Every `.ged` file under `dir`.
fn ged_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            ged_files(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ged")) {
            out.push(p);
        }
    }
}

/// Writing is idempotent: write(read(write(x))) == write(x), on every
/// fixture, in every version, for the dataset and for the lossless tree.
#[test]
fn writing_is_idempotent() {
    let mut files = Vec::new();
    ged_files(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        &mut files,
    );
    files.sort();
    assert!(files.len() > 20);
    for path in files {
        let bytes = std::fs::read(&path).unwrap();
        for version in VERSIONS {
            let data = Dataset::from_bytes(bytes.as_slice());
            let out = write(&data, version);
            assert_eq!(write(&read(&out), version), out, "{}", path.display());
            let tree = |bytes: &[u8]| {
                let mut out = Vec::new();
                GedcomWriter::new()
                    .gedcom_version(version)
                    .bom(Bom::Never)
                    .write_tree(&mut out, &Tree::from_bytes(bytes))
                    .unwrap();
                out
            };
            let out = tree(&bytes);
            assert_eq!(tree(&out), out, "{}", path.display());
        }
    }
}

/// A date or an age is converted as it is written, without the characters
/// the version bans: `> \u{81}69y` (a C1 control) is the 5.5.1 age
/// `> 69y`, which reads back valid and is written again as it is (fuzz
/// `writer`).
#[test]
fn values_are_converted_as_written() {
    let input = "0 HEAD\n1 SOUR EXAMPLE\n1 SUBM @U1@\n1 GEDC\n2 VERS 5.5.1\n0 @U1@ SUBM\n1 NAME A\n0 @I1@ INDI\n1 DEAT Y\n2 DATE \u{81}1 JAN 1900\n2 AGE > \u{81}69y 11m\n1 BURI\n2 AGE about seventy\n3 PHRASE \u{1}\n1 CREM\n2 AGE nearly eighty\n3 PHRASE\n0 TRLR\n";
    for version in VERSIONS {
        let out = write(&read(input), version);
        if version == GedcomVersion::V5_5_1 {
            assert!(out.contains("2 AGE > 69y 11m\n"), "{out}");
            assert!(out.contains("2 DATE 1 JAN 1900\n"), "{out}");
        }
        assert!(!out.contains('\u{81}') && !out.contains('\u{1}'), "{out}");
        assert_eq!(write(&read(&out), version), out);
    }
}

/// A record the header documents as an alias of a standard record
/// (`_USER` for `record-SUBM`, 7.x `HEAD.SCHMA`) is a record of that type,
/// whichever way the writer goes: the pointers to it stay standard (or
/// documented aliases), written natively as converted from a file with no
/// version, which a second write once wrote as extensions (fuzz
/// `entry_points`).
#[test]
fn pointers_to_a_documented_alias_record_stay_standard() {
    let schma = "1 SCHMA\n2 TAG _USER https://gedcom.io/terms/v7/record-SUBM\n2 TAG _CREATOR https://gedcom.io/terms/v7/SUBM\n";
    let input = format!("0 HEAD\n{schma}0 @U1@ _USER\n1 _CREATOR @U1@\n1 GEDC\n");
    for version in VERSIONS {
        let out = write(&read(&input), version);
        assert_eq!(write(&read(&out), version), out, "{version}");
    }
    let input = format!(
        "0 HEAD\n1 GEDC\n2 VERS 7.0\n{schma}0 @U1@ _USER\n1 NAME Example Submitter\n0 @I1@ INDI\n1 SUBM @U1@\n1 _CREATOR @U1@\n0 TRLR\n"
    );
    let out = write(&read(&input), GedcomVersion::V7_0);
    assert!(out.contains("1 SUBM @U1@\n1 _CREATOR @U1@\n"), "{out}");
    // A header whose `SCHMA` the repair makes an extension (it has a
    // payload here: a line without a level continues it) declares no
    // alias: the pointer is to an extension record (fuzz `model`).
    let input = "0 HEAD\n1 SCHMA\n2$TA://example.com/calendar\n2 TAG _USER https://gedcom.io/terms/v7/record-SUBM\n1 SUBM @U1@\n0 @U1@ _USER";
    for version in VERSIONS {
        let out = write(&read(input), version);
        let left = ged_io::spec::validate_text(&out);
        assert!(left.is_empty(), "{version}: {left:?}\n{out}");
        assert_eq!(write(&read(&out), version), out, "{version}");
    }
}

/// A pointer field (or a note) holding a pointer and banned characters is
/// written as the pointer, as it is written without them: written as text,
/// it would read back as that pointer (fuzz `writer`).
#[test]
fn pointer_typing_follows_what_is_written() {
    let input = "0 HEAD\n1 SOUR EXAMPLE\n1 SUBM @1@\u{1}\u{2}\n1 GEDC\n2 VERS 5.5.1\n0 @1@ SUBM\n1 NAME A\n1 NOTE \u{9d}@N1@\n0 @N1@ NOTE A shared note\n0 TRLR\n";
    for version in VERSIONS {
        let out = write(&read(input), version);
        if version == GedcomVersion::V5_5_1 {
            assert!(out.contains("1 SUBM @1@\n"), "{out}");
        }
        assert_eq!(write(&read(&out), version), out);
    }
}
