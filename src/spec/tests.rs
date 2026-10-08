//! Unit tests of the tables, the validator and every row of the repair
//! table.

use super::*;
use crate::tree::parse_tree;
use crate::types::date::Calendar;

/// The calendars of the tables and of the date grammar agree: same
/// calendars, same months in the same order; in 7.x, an epoch exactly for
/// the calendars that count years before one.
#[test]
fn calendars_agree_with_the_date_grammar() {
    let known = [
        Calendar::Gregorian,
        Calendar::Julian,
        Calendar::Hebrew,
        Calendar::FrenchRepublican,
        Calendar::Roman,
        Calendar::Unknown,
    ];
    for version in [
        GedcomVersion::V5_5_1,
        GedcomVersion::V7_0,
        GedcomVersion::V7_1,
    ] {
        let schema = version.rules().spec;
        let v7 = schema.version.starts_with('7');
        for cal in schema.calendars {
            let ours = known
                .iter()
                .find(|c| {
                    if v7 {
                        c.gedcom7_tag() == cal.tag
                    } else {
                        c.gedcom551_escape() == format!("@#D{}@", cal.tag)
                    }
                })
                .unwrap_or_else(|| panic!("{} calendar {}", schema.version, cal.tag));
            let months: Vec<&str> = ours.months().iter().map(|m| m.tag()).collect();
            assert_eq!(months, cal.months, "{} {}", schema.version, cal.tag);
            if v7 {
                assert_eq!(ours.has_bce(), !cal.epochs.is_empty(), "{}", cal.tag);
            }
        }
    }
}

/// The kinds and lines `validate_text` reports.
fn found(text: &str) -> Vec<(u32, DeviationKind)> {
    validate_text(text)
        .iter()
        .map(|d| (d.line, d.kind))
        .collect()
}

const H70: &str = "0 HEAD\n1 GEDC\n2 VERS 7.0\n";
const H551: &str = "0 HEAD\n1 SOUR EXAMPLE\n1 SUBM @U1@\n1 GEDC\n2 VERS 5.5.1\n2 FORM LINEAGE-LINKED\n1 CHAR UTF-8\n0 @U1@ SUBM\n1 NAME Example\n";

fn v70(body: &str) -> Vec<(u32, DeviationKind)> {
    found(&format!("{H70}{body}0 TRLR\n"))
}

fn v551(body: &str) -> Vec<(u32, DeviationKind)> {
    found(&format!("{H551}{body}0 TRLR\n"))
}

use DeviationKind as K;

#[test]
fn minimal_datasets_are_clean() {
    assert_eq!(v70(""), []);
    assert_eq!(v551(""), []);
    assert_eq!(found("0 HEAD\n1 GEDC\n2 VERS 7.1\n0 TRLR\n"), []);
    assert_eq!(found("0 HEAD\n1 GEDC\n2 VERS 7.0.14\n0 TRLR\n"), []);
}

#[test]
fn encoding() {
    let utf16: Vec<u8> = "\u{feff}0 HEAD\n1 GEDC\n2 VERS 7.0\n0 TRLR\n"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let d = validate_bytes(&utf16);
    assert!(d.iter().any(|d| d.kind == K::Encoding), "{d:?}");
    // A Windows-1252 byte under CHAR ASCII.
    let mut bytes = format!("{H551}0 @I1@ INDI\n1 NAME Caf_\n0 TRLR\n")
        .replace("CHAR UTF-8", "CHAR ASCII")
        .into_bytes();
    if let Some(b) = bytes.iter_mut().find(|b| **b == b'_') {
        *b = 0xE9;
    }
    assert!(validate_bytes(&bytes).iter().any(|d| d.kind == K::Encoding));
    assert_eq!(validate_bytes(format!("{H551}0 TRLR\n").as_bytes()), []);
}

#[test]
fn line_syntax() {
    assert_eq!(
        found("0 HEAD\n1 GEDC\n2  VERS 7.0\n0 TRLR\n"),
        [(3, K::LineSyntax)]
    );
    assert_eq!(
        found("0 HEAD\n1 GEDC \n2 VERS 7.0\n0 TRLR\n"),
        [(2, K::LineSyntax)]
    );
    assert_eq!(
        found("0 HEAD\n1 GEDC\n2 VERS 7.0\n\n0 TRLR\n"),
        [(4, K::LineSyntax)]
    );
    assert_eq!(
        found("0 HEAD\n1 GEDC\n 2 VERS 7.0\n0 TRLR\n"),
        [(3, K::LineSyntax)]
    );
    assert_eq!(
        found("0 HEAD\n1 GEDC\n2 VERS 7.0\n0 TRLR"),
        [(4, K::LineSyntax)]
    );
    assert!(found("0 HEAD\n1 GEDC\n2 VERS 7.0\nno level\n0 TRLR\n").contains(&(4, K::LineSyntax)));
    assert!(found("0 HEAD\n1\tGEDC\n2 VERS 7.0\n0 TRLR\n").contains(&(2, K::LineSyntax)));
    // 5.5.1 readers skip white space and blank lines before a line (p. 11).
    assert_eq!(v551("\n  0 @I1@ INDI\n"), []);
}

#[test]
fn levels() {
    assert!(v70("0 @I1@ INDI\n2 _X x\n").contains(&(5, K::Level)));
    assert!(v70("0 @I1@ INDI\n01 _X x\n").contains(&(5, K::Level)));
    assert!(v551("0 @I1@ INDI\n100 _X x\n").contains(&(11, K::Level)));
    assert!(found("1 HEAD\n0 TRLR\n").contains(&(1, K::Level)));
}

#[test]
fn identifiers() {
    assert_eq!(v70("0 @I_1@ INDI\n"), []);
    assert_eq!(v70("0 @i1@ INDI\n"), [(4, K::Xref)]);
    assert_eq!(v70("0 @VOID@ INDI\n"), [(4, K::Xref)]);
    assert_eq!(v70("0 @I1@ INDI\n0 @I1@ INDI\n"), [(5, K::Xref)]);
    assert_eq!(v70("0 @I1@ INDI\n1 @N1@ NOTE x\n"), [(5, K::Xref)]);
    assert_eq!(v551("0 @i 1@ INDI\n"), []);
    // Spaces, `!`, `:` and a leading `_` are 5.5.1 identifier characters.
    assert_eq!(v551("0 @_X@ INDI\n0 @A:B@ FAM\n0 @I1!2@ _X\n"), []);
    assert_eq!(v551("0 @#1@ INDI\n"), [(10, K::Xref)]);
    assert_eq!(v551("0 @ABCDEFGHIJKLMNOPQRSTU@ INDI\n"), [(10, K::Xref)]);
    assert!(found("0 @H@ HEAD\n1 GEDC\n2 VERS 7.0\n0 TRLR\n").contains(&(1, K::Xref)));
}

#[test]
fn escapes() {
    assert_eq!(
        v70("0 @I1@ INDI\n1 NOTE @@ leading\n1 NOTE inner @ sign\n"),
        []
    );
    assert_eq!(v70("0 @I1@ INDI\n1 NOTE @ leading\n"), [(5, K::Escape)]);
    assert_eq!(v551("0 @I1@ INDI\n1 NOTE a @@ b @#DJULIAN@ c\n"), []);
    assert_eq!(v551("0 @I1@ INDI\n1 NOTE a @ b\n"), [(11, K::Escape)]);
}

#[test]
fn line_length_and_record_size() {
    let long = "x".repeat(250);
    assert_eq!(
        v551(&format!("0 @I1@ INDI\n1 NOTE {long}\n")),
        [(11, K::LineLength)]
    );
    assert_eq!(v70(&format!("0 @I1@ INDI\n1 NOTE {long}{long}\n")), []);
    let mut big = String::from("0 @I1@ INDI\n");
    for _ in 0..200 {
        big.push_str(&format!("1 NOTE {}\n", "y".repeat(200)));
    }
    assert!(v551(&big).contains(&(10, K::RecordSize)));
    let tree = parse_tree(&format!("{H551}{big}0 TRLR\n"));
    assert!(validate(&tree, GedcomVersion::V5_5_1)
        .iter()
        .any(|d| d.kind == K::RecordSize));
}

#[test]
fn continuations() {
    assert_eq!(v70("0 @I1@ INDI\n1 NOTE a\n2 CONT b\n"), []);
    assert!(v70("0 @I1@ INDI\n1 NOTE a\n2 CONC b\n").contains(&(6, K::Continuation)));
    assert!(v70("0 @I1@ INDI\n1 NOTE a\n2 LANG en\n2 CONT b\n").contains(&(7, K::Continuation)));
    assert!(
        v70("0 @I1@ INDI\n1 FAMC @F1@\n2 CONT b\n0 @F1@ FAM\n1 HUSB @I1@\n")
            .contains(&(6, K::Continuation))
    );
    assert!(v70("0 @I1@ INDI\n1 NOTE a\n2 CONT b\n3 _X c\n").contains(&(7, K::Continuation)));
    assert_eq!(v551("0 @I1@ INDI\n1 NOTE ab\n2 CONC cd\n"), []);
    assert!(v551("0 @I1@ INDI\n1 NOTE ab \n2 CONC cd\n").contains(&(12, K::Continuation)));
}

#[test]
fn tags() {
    assert_eq!(
        v70("0 @I1@ INDI\n1 _X x\n2 NAME standard tag under an extension\n"),
        []
    );
    assert_eq!(v70("0 @I1@ INDI\n1 name x\n"), [(5, K::UnknownTag)]);
    assert_eq!(v70("0 @I1@ INDI\n1 _x x\n"), [(5, K::UnknownTag)]);
    assert_eq!(v70("0 @I1@ INDI\n1 FOO x\n"), [(5, K::UnknownTag)]);
    assert_eq!(v551("0 @I1@ INDI\n1 _lower x\n"), []);
    assert_eq!(v551("0 @I1@ INDI\n1 UID x\n"), [(11, K::Misplaced)]);
}

#[test]
fn placement_and_cardinality() {
    assert_eq!(v70("0 @I1@ INDI\n1 ADDR x\n"), [(5, K::Misplaced)]);
    assert_eq!(
        found("0 HEAD\n1 GEDC\n2 VERS 7.0\n1 PHON 1\n0 TRLR\n"),
        [(4, K::Misplaced)]
    );
    assert_eq!(v70("0 @C1@ COPR\n"), [(4, K::Misplaced)]);
    assert_eq!(
        v70("0 @I1@ INDI\n1 SEX M\n1 SEX F\n"),
        [(4, K::Cardinality)]
    );
    assert!(found("0 HEAD\n0 TRLR\n").contains(&(1, K::MissingRequired)));
    assert_eq!(v70("0 @O1@ OBJE\n"), [(4, K::MissingRequired)]);
    assert_eq!(v70("0 @N1@ SNOTE x\n1 TRAN y\n"), [(5, K::MissingRequired)]);
    assert_eq!(v70("0 @N1@ SNOTE x\n1 TRAN y\n2 LANG fr\n"), []);
    // 5.5.1 pointer and text forms of one tag share their cardinality.
    assert_eq!(
        v551("0 @I1@ INDI\n1 NOTE @N1@\n1 NOTE text\n0 @N1@ NOTE n\n"),
        []
    );
}

#[test]
fn payloads() {
    assert_eq!(v70("0 @I1@ INDI\n1 BIRT N\n"), [(5, K::Payload)]);
    assert_eq!(
        v70("0 @I1@ INDI\n1 BIRT\n2 DATE 30 FEB 1900\n"),
        [(6, K::Payload)]
    );
    assert_eq!(
        v70("0 @I1@ INDI\n1 BIRT\n2 DATE 1900\n3 TIME 25:00\n"),
        [(7, K::Payload)]
    );
    assert_eq!(v70("0 @I1@ INDI\n1 DEAT\n2 AGE 79\n"), [(6, K::Payload)]);
    assert_eq!(v70("0 @I1@ INDI\n1 NAME a/b/c/d\n"), [(5, K::Payload)]);
    assert_eq!(v70("0 @I1@ INDI\n1 NCHI x\n"), [(5, K::Payload)]);
    assert_eq!(v70("0 @I1@ INDI\n1 BIRT\n"), [(5, K::Payload)]);
    assert_eq!(
        v70("0 @I1@ INDI\n1 NOTE @N1@\n0 @N1@ SNOTE x\n"),
        [(5, K::Payload)]
    );
    assert_eq!(v70("0 @I1@ INDI\n1 FAMC x\n"), [(5, K::Payload)]);
    assert_eq!(v70("0 @I1@ INDI\n1 FAMC @VOID@\n"), []);
    assert_eq!(v70("0 @N1@ SNOTE x\n1 MIME image/png\n"), [(5, K::Payload)]);
    assert_eq!(
        v70("0 @O1@ OBJE\n1 FILE a b\n2 FORM image/png\n"),
        [(5, K::Payload)]
    );
    assert_eq!(
        v70("0 @I1@ INDI\n1 RESI\n2 PLAC x\n3 MAP\n4 LATI N91\n4 LONG E1\n"),
        [(8, K::Payload)]
    );
    assert_eq!(
        found("0 HEAD\n1 GEDC\n2 VERS 7.0\n1 SCHMA\n2 TAG X y\n0 TRLR\n"),
        [(5, K::Payload)]
    );
    assert_eq!(v551("0 @I1@ INDI\n1 DEAT\n2 AGE > 79y\n"), []);
    assert_eq!(v551("0 @I1@ INDI\n1 DEAT\n2 AGE <79y\n"), []);
    assert_eq!(v551("0 @I1@ INDI\n1 DEAT\n2 DATE (some day)\n"), []);
}

#[test]
fn enumerations() {
    assert_eq!(v70("0 @I1@ INDI\n1 SEX _OTHERS\n"), []);
    assert_eq!(v70("0 @I1@ INDI\n1 SEX m\n"), [(5, K::EnumValue)]);
    assert_eq!(v70("0 @I1@ INDI\n1 RESN LOCKED,PRIVACY\n"), []);
    assert_eq!(v70("0 @I1@ INDI\n1 RESN LOCKED, \n"), [(5, K::EnumValue)]);
    assert_eq!(v551("0 @I1@ INDI\n1 SEX m\n"), []);
    assert_eq!(v551("0 @I1@ INDI\n1 SEX _X\n"), [(11, K::EnumValue)]);
    // Open sets admit any value.
    assert_eq!(v551("0 @I1@ INDI\n1 NAME A /B/\n2 TYPE stage\n"), []);
}

#[test]
fn pointers() {
    assert_eq!(v70("0 @I1@ INDI\n1 FAMC @F9@\n"), [(5, K::DanglingPointer)]);
    assert_eq!(v70("0 @I1@ INDI\n1 FAMC @I1@\n"), [(5, K::PointerTarget)]);
    assert_eq!(
        v70("0 @I1@ INDI\n1 _LINK @X9@\n"),
        [(5, K::DanglingPointer)]
    );
    assert_eq!(v70("0 @I1@ INDI\n1 _LINK @I1@\n"), []);
    // 5.5.1 substructure and network pointers name no record of the file.
    assert_eq!(
        v551("0 @I1@ INDI\n1 ASSO @I1!2@\n2 RELA x\n1 FAMC @A:F1@\n"),
        []
    );
    // A documented alias of a standard record is that record (7.x §1.5.1).
    let schma = "1 SCHMA\n2 TAG _USER https://gedcom.io/terms/v7/record-SUBM\n";
    assert_eq!(
        found(&format!(
            "{H70}{schma}1 SUBM @U1@\n0 @U1@ _USER\n1 NAME x\n0 TRLR\n"
        )),
        []
    );
}

#[test]
fn header_and_trailer() {
    assert_eq!(found("0 TRLR\n"), [(1, K::Header)]);
    assert_eq!(found("").len(), 2);
    assert!(found("0 HEAD\n1 GEDC\n2 VERS 7.0\n").contains(&(1, K::Header)));
    assert_eq!(v70("0 TRLR\n"), [(5, K::Header)]);
    assert_eq!(
        found("0 HEAD\n1 GEDC\n2 VERS 7.0\n0 TRLR\n1 _X x\n"),
        [(4, K::Header)]
    );
    assert_eq!(
        found("0 HEAD\n1 GEDC\n2 VERS 7.0\n0 TRLR\n0 @I1@ INDI\n"),
        [(5, K::Header)]
    );
    let tree = parse_tree("0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 TRLR\n");
    assert!(validate(&tree, GedcomVersion::V7_0)
        .iter()
        .any(|d| d.kind == K::Header && d.line == 3));
}

#[test]
fn characters() {
    assert_eq!(v70("0 @I1@ INDI\n1 NOTE a\u{7}b\n"), [(5, K::Character)]);
    assert_eq!(v70("0 @I1@ INDI\n1 NOTE a\tb\n"), []);
    assert_eq!(v551("0 @I1@ INDI\n1 NOTE a\tb\n"), [(11, K::Character)]);
}

#[test]
fn trees_and_structures_agree() {
    let text = format!("{H70}0 @I1@ INDI\n1 SEX m\n1 FAMC @F9@\n0 TRLR\n");
    let tree = parse_tree(&text);
    let records = tree.to_structures();
    let a = validate(&tree, GedcomVersion::V7_0);
    let b = validate(&records, GedcomVersion::V7_0);
    let c = validate(records.as_slice(), GedcomVersion::V7_0);
    assert_eq!(a, b);
    assert_eq!(b, c);
    assert_eq!(a.len(), 2);
    assert_eq!(
        a[0].to_string(),
        "line 5: SEX \"m\": not a value of enumset-SEX"
    );
}
