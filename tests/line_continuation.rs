//! Continuation lines: spaces at `CONC`/`CONT` boundaries are kept, and any
//! value, extension tags included, is continued.

use ged_io::{GedcomBuilder, GedcomStreamParser, GedcomWriter};
use std::io::BufReader;

#[test]
fn test_parse_keeps_spaces_after_conc_and_cont_delimiter() {
    // One space separates the tag from its value; any further space is
    // part of the value.
    let original = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @F1@ FAM\n1 NOTE first\n2 CONC  second\n2 CONT    indented line\n0 TRLR";
    let data = GedcomBuilder::new().build_from_str(original).unwrap();
    assert_eq!(
        data.families[0].notes[0].value.as_deref(),
        Some("first second\n   indented line")
    );
}

#[test]
fn test_round_trip_long_text_keeps_spaces_at_conc_splits() {
    // Words of every length around the 255-byte limit, so that the naive
    // split point falls next to a space in some of them.
    for word_len in 1..12 {
        let word = "x".repeat(word_len);
        let text = vec![word.as_str(); 120].join(" ");
        let original = format!("0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @F1@ FAM\n1 NOTE {text}\n0 TRLR");
        let data1 = GedcomBuilder::new().build_from_str(&original).unwrap();
        let written = GedcomWriter::new().write_to_string(&data1).unwrap();

        for line in written.lines().filter(|l| l.contains(" CONC ")) {
            let value = line.split_once(" CONC ").unwrap().1;
            assert!(
                !value.starts_with(' '),
                "CONC value starts with a space: {line:?}"
            );
        }
        for line in written.lines() {
            assert!(!line.ends_with(' '), "line ends with a space: {line:?}");
        }

        let data2 = GedcomBuilder::new().build_from_str(&written).unwrap();
        assert_eq!(
            data2.families[0].notes[0].value.as_deref(),
            Some(text.as_str()),
            "word length {word_len}"
        );
    }
}

#[test]
fn test_stream_parser_keeps_spaces_after_conc_delimiter() {
    let gedcom = "\
        0 HEAD\n\
        1 GEDC\n\
        2 VERS 5.5.1\n\
        0 @F1@ FAM\n\
        1 NOTE first\n\
        2 CONC  second\n\
        0 TRLR";
    let reader = BufReader::new(gedcom.as_bytes());
    let parser = GedcomStreamParser::new(reader).unwrap();
    let records: Vec<_> = parser.collect::<Result<Vec<_>, _>>().unwrap();
    let family = records[1].as_family().unwrap();
    assert_eq!(family.notes[0].value.as_deref(), Some("first second"));
}

#[test]
fn test_event_and_attribute_values_are_continued() {
    // A value continued before the structure's substructures.
    let original = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 TITL Duke\n2 CONC  of Somewhere\n\
                    1 OCCU Long\n2 CONT second line\n2 DATE 1900\n1 BIRT Born at home\n\
                    2 CONC  during a storm\n2 DATE 1 JAN 1900\n0 TRLR";
    let data = GedcomBuilder::new().build_from_str(original).unwrap();
    let person = &data.individuals[0];
    assert_eq!(
        person.attributes[0].value.as_deref(),
        Some("Duke of Somewhere")
    );
    assert_eq!(
        person.attributes[1].value.as_deref(),
        Some("Long\nsecond line")
    );
    assert!(person.attributes[1].date.is_some());
    assert_eq!(
        person.events[0].value.as_deref(),
        Some("Born at home during a storm")
    );
    assert!(person.attributes.iter().all(|a| a.custom_data.is_empty()));
    assert!(person.events[0].custom_data.is_empty());
}

#[test]
fn test_extension_tags_after_continued_text() {
    // An extension tag right after CONT lines is a substructure, not an
    // error.
    let original = "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @N1@ SNOTE First\n1 CONT second\n1 _X kept\n\
                    0 @I1@ INDI\n1 NOTE a\n2 CONT b\n2 _Y kept\n0 TRLR";
    let data = GedcomBuilder::new().build_from_str(original).unwrap();
    assert_eq!(data.shared_notes[0].text, "First\nsecond");
    assert_eq!(data.individuals[0].notes[0].value.as_deref(), Some("a\nb"));
}
