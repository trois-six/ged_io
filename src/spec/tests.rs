//! Unit tests of the tables, the validator and every row of the repair
//! table.

use super::*;
use crate::tree::parse_tree;
use crate::value::Calendar;

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

/// Conforms `text` read as a tree, for `version`; checks that the result
/// validates; returns the repairs and the result as text.
fn repaired(text: &str, version: GedcomVersion) -> (Vec<Repair>, String) {
    let mut records = parse_tree(text).to_structures();
    let repairs = conform(&mut records, version);
    let left = validate(&records, version);
    let out: String = records.iter().map(|r| r.to_gedcom(0, version)).collect();
    assert!(left.is_empty(), "{text}\n=> {out}\n{left:?}\n{repairs:?}");
    (repairs, out)
}

/// The repairs of `body` (records between a header and a trailer), with
/// their kinds, and the output.
fn repair70(body: &str) -> (Vec<RepairKind>, String) {
    let (r, out) = repaired(&format!("{H70}{body}0 TRLR\n"), GedcomVersion::V7_0);
    (r.iter().map(|r| r.kind).collect(), out)
}

fn repair551(body: &str) -> (Vec<RepairKind>, String) {
    let (r, out) = repaired(&format!("{H551}{body}0 TRLR\n"), GedcomVersion::V5_5_1);
    (r.iter().map(|r| r.kind).collect(), out)
}

use RepairKind as R;

/// A 7.x record named `@VOID@` is renamed; a pointer to `@VOID@` stays the
/// null pointer it is in 7.x.
#[test]
fn void_pointers_do_not_follow_a_renamed_void_record() {
    let (_, out) = repair70("0 @VOID@ INDI\n0 @I2@ INDI\n0 @F1@ FAM\n1 HUSB @VOID@\n1 WIFE @I2@\n");
    assert!(out.contains("0 @VOID_@ INDI\n"), "{out}");
    assert!(out.contains("1 HUSB @VOID@\n"), "{out}");
}

#[test]
fn repair_a_enumeration_value() {
    // 7.x: OTHER and a PHRASE where both exist.
    let (k, out) = repair70("0 @I1@ INDI\n1 FAMC @VOID@\n2 PEDI stepchild\n");
    assert_eq!(k, [R::EnumValue]);
    assert!(out.contains("2 PEDI OTHER\n3 PHRASE stepchild\n"), "{out}");
    // 7.x: an extension value is a value already.
    assert_eq!(repair70("0 @I1@ INDI\n1 SEX _NB\n").0, []);
    // Otherwise the structure becomes an extension, in both versions.
    let (k, out) = repair70("0 @I1@ INDI\n1 SEX male\n");
    assert_eq!(k, [R::EnumValue]);
    assert!(out.contains("1 _SEX male\n"), "{out}");
    let (k, out) = repair551("0 @I1@ INDI\n1 SEX _NB\n");
    assert_eq!(k, [R::EnumValue]);
    assert!(out.contains("1 _SEX _NB\n"), "{out}");
    // A list with a value outside the set.
    let (k, out) = repair70("0 @I1@ INDI\n1 RESN LOCKED, SECRET\n");
    assert_eq!(k, [R::EnumValue]);
    assert!(out.contains("1 _RESN LOCKED, SECRET\n"), "{out}");
}

#[test]
fn repair_b_enumeration_case() {
    let (k, out) = repair70("0 @I1@ INDI\n1 FAMC @VOID@\n2 PEDI birth\n1 RESN locked,privacy,\n");
    assert_eq!(k, [R::EnumCase, R::EnumCase]);
    assert!(out.contains("2 PEDI BIRTH\n"), "{out}");
    assert!(out.contains("1 RESN LOCKED, PRIVACY\n"), "{out}");
    let (k, out) = repair551("0 @I1@ INDI\n1 FAMC @F1@\n2 PEDI Birth\n2 STAT PROVEN\n0 @F1@ FAM\n");
    assert_eq!(k, [R::EnumCase, R::EnumCase]);
    assert!(out.contains("2 PEDI birth\n2 STAT proven\n"), "{out}");
    // An open set: a value of its own in another case is spelled as the
    // set spells it, a value of the user's own is kept.
    let (k, out) = repair551("0 @I1@ INDI\n1 NAME A /B/\n2 TYPE AKA\n1 NAME C /D/\n2 TYPE Stage\n");
    assert_eq!(k, [R::EnumCase]);
    assert!(out.contains("2 TYPE aka\n"), "{out}");
    assert!(out.contains("2 TYPE Stage\n"), "{out}");
}

#[test]
fn repair_c_pointers() {
    // Text where a pointer belongs: @VOID@ and a PHRASE in 7.x...
    let (k, out) = repair70("0 @F1@ FAM\n1 HUSB Unknown father\n");
    assert_eq!(k, [R::Pointer]);
    assert!(
        out.contains("1 HUSB @VOID@\n2 PHRASE Unknown father\n"),
        "{out}"
    );
    // ... an extension where there is no PHRASE, and in 5.5.1.
    let (k, out) = repair70("0 @I1@ INDI\n1 FAMC Some family\n");
    assert_eq!(k, [R::Pointer]);
    assert!(out.contains("1 _FAMC Some family\n"), "{out}");
    let (k, out) = repair551("0 @F1@ FAM\n1 HUSB Unknown father\n");
    assert_eq!(k, [R::Pointer]);
    assert!(out.contains("1 _HUSB Unknown father\n"), "{out}");
    // No pointer: @VOID@ in 7.x.
    let (k, out) = repair70("0 @F1@ FAM\n1 HUSB\n2 PHRASE x\n");
    assert_eq!(k, [R::Pointer]);
    assert!(out.contains("1 HUSB @VOID@\n"), "{out}");
    // A nullable 5.5.1 pointer may stay empty.
    assert_eq!(repair551("0 @S1@ SOUR\n1 REPO\n2 CALN 12\n").0, []);
    // A dangling pointer.
    let (k, out) = repair70("0 @F1@ FAM\n1 WIFE @I9@\n");
    assert_eq!(k, [R::Pointer]);
    assert!(out.contains("1 WIFE @VOID@\n2 PHRASE @@I9@\n"), "{out}");
    let (k, out) = repair551("0 @F1@ FAM\n1 WIFE @I9@\n");
    assert_eq!(k, [R::Pointer]);
    assert!(out.contains("1 _WIFE @@I9@@\n"), "{out}");
    // A pointer to the wrong record type.
    let (k, out) = repair70("0 @F1@ FAM\n1 WIFE @F1@\n");
    assert_eq!(k, [R::Pointer]);
    assert!(out.contains("1 _WIFE @F1@\n"), "{out}");
    // A pointer where text belongs.
    let (k, out) = repair70("0 @I1@ INDI\n1 NOTE @N1@\n0 @N1@ SNOTE x\n");
    assert_eq!(k, [R::Pointer]);
    assert!(out.contains("1 NOTE @@N1@\n"), "{out}");
    // A pointer of an extension to nothing becomes its text.
    let (k, out) = repair70("0 @I1@ INDI\n1 _LINK @X9@\n");
    assert_eq!(k, [R::Pointer]);
    assert!(out.contains("1 _LINK @@X9@\n"), "{out}");
}

#[test]
fn repair_d_payloads() {
    // 7.x dates and ages: empty payload and a PHRASE.
    let (k, out) = repair70("0 @I1@ INDI\n1 BIRT\n2 DATE 30 FEB 1900\n1 DEAT\n2 AGE about 80\n");
    assert_eq!(k, [R::Payload, R::Payload]);
    assert!(out.contains("2 DATE\n3 PHRASE 30 FEB 1900\n"), "{out}");
    assert!(out.contains("2 AGE\n3 PHRASE about 80\n"), "{out}");
    // A date that has a PHRASE already becomes an extension.
    let (k, out) = repair70("0 @I1@ INDI\n1 BIRT\n2 DATE spring\n3 PHRASE in spring\n");
    assert_eq!(k, [R::Payload]);
    assert!(
        out.contains("2 _DATE spring\n3 PHRASE in spring\n"),
        "{out}"
    );
    // 5.5.1 date value: a date phrase; age: an extension.
    let (k, out) = repair551("0 @I1@ INDI\n1 BIRT\n2 DATE spring 1900\n1 DEAT\n2 AGE about 80\n");
    assert_eq!(k, [R::Payload, R::Payload]);
    assert!(out.contains("2 DATE (spring 1900)\n"), "{out}");
    assert!(out.contains("2 _AGE about 80\n"), "{out}");
    // An event with text: Y and a NOTE.
    let (k, out) = repair70("0 @I1@ INDI\n1 BIRT at home\n");
    assert_eq!(k, [R::Payload]);
    assert!(out.contains("1 BIRT Y\n2 NOTE at home\n"), "{out}");
    // A payload where none belongs: a NOTE, where one is permitted.
    let (k, out) = repair551("0 @I1@ INDI text\n");
    assert_eq!(k, [R::Payload]);
    assert!(out.contains("0 @I1@ INDI\n1 NOTE text\n"), "{out}");
    // Any other grammar: an extension.
    let (k, out) = repair70("0 @I1@ INDI\n1 NAME a/b/c/d\n1 NCHI two\n");
    assert_eq!(k, [R::Payload, R::Payload]);
    assert!(out.contains("1 _NAME a/b/c/d\n1 _NCHI two\n"), "{out}");
    // MIME of a text.
    let (k, out) = repair70("0 @N1@ SNOTE x\n1 MIME image/png\n");
    assert_eq!(k, [R::Payload]);
    assert!(out.contains("1 _MIME image/png\n"), "{out}");
}

#[test]
fn repair_e_misplaced() {
    // A standard structure one type stands for keeps its meaning (7.x).
    let (k, out) = repair70("0 @I1@ INDI\n1 ADDR 1 Example Street\n2 CITY Sampleton\n");
    assert_eq!(k, [R::Misplaced]);
    assert!(
        out.contains("1 SCHMA\n2 TAG _ADDR https://gedcom.io/terms/v7/ADDR\n"),
        "{out}"
    );
    assert!(
        out.contains("1 _ADDR 1 Example Street\n2 CITY Sampleton\n"),
        "{out}"
    );
    // A tag several types share does not.
    let (k, out) = repair70("0 @I1@ INDI\n1 DATE 1900\n");
    assert_eq!(k, [R::Misplaced]);
    assert!(!out.contains("SCHMA"), "{out}");
    // A tag already used as an extension is not declared.
    let (_, out) = repair70("0 @I1@ INDI\n1 ADDR x\n1 _ADDR mine\n");
    assert!(!out.contains("SCHMA"), "{out}");
    // Unknown, malformed, lower-case and CONT structures; 7.0 structures
    // in 5.5.1.
    let (k, out) = repair70("0 @I1@ INDI\n1 FOO x\n1 name y\n1 _ext z\n");
    assert_eq!(k, [R::Misplaced, R::Misplaced, R::Misplaced]);
    assert!(out.contains("1 _FOO x\n1 _NAME y\n1 _EXT z\n"), "{out}");
    let (k, out) = repair551("0 @I1@ INDI\n1 UID 1234\n1 ADDR x\n");
    assert_eq!(k, [R::Misplaced, R::Misplaced]);
    assert!(out.contains("1 _UID 1234\n1 _ADDR x\n"), "{out}");
    let mut records = parse_tree(&format!(
        "{H70}0 @I1@ INDI\n1 FAMC @VOID@\n2 CONT x\n0 TRLR\n"
    ))
    .to_structures();
    let k: Vec<_> = conform(&mut records, GedcomVersion::V7_0)
        .iter()
        .map(|r| r.kind)
        .collect();
    assert_eq!(k, [R::Misplaced]);
    assert!(validate(&records, GedcomVersion::V7_0).is_empty());
    // A record the version does not define.
    let (k, out) = repair70("0 @C1@ COPR x\n");
    assert_eq!(k, [R::Misplaced]);
    assert!(out.contains("0 @C1@ _COPR x\n"), "{out}");
}

#[test]
fn repair_f_repeated() {
    let (k, out) = repair70("0 @I1@ INDI\n1 SEX M\n1 SEX F\n");
    assert_eq!(k, [R::Repeated]);
    assert!(out.contains("1 SEX M\n1 _SEX F\n"), "{out}");
    assert!(
        out.contains("2 TAG _SEX https://gedcom.io/terms/v7/SEX\n"),
        "{out}"
    );
    let (k, out) = repair551("0 @I1@ INDI\n1 SEX M\n1 SEX F\n");
    assert_eq!(k, [R::Repeated]);
    assert!(out.contains("1 SEX M\n1 _SEX F\n"), "{out}");
    // A second SUBN record.
    let (k, out) = repair551("0 @B1@ SUBN\n0 @B2@ SUBN\n");
    assert_eq!(k, [R::Repeated]);
    assert!(out.contains("0 @B2@ _SUBN\n"), "{out}");
}

#[test]
fn repair_g_required() {
    // 7.x: synthesised from the list.
    let (k, out) = repair70(
        "0 @O1@ OBJE\n1 FILE media/a.jpg\n0 @I1@ INDI\n1 NAME A /B/\n2 TRAN a b\n1 ASSO @I1@\n1 FACT x\n0 @R1@ REPO\n",
    );
    assert_eq!(k, [R::Required; 5]);
    assert!(
        out.contains("1 FILE media/a.jpg\n2 FORM image/jpeg\n"),
        "{out}"
    );
    assert!(out.contains("2 TRAN a b\n3 LANG und\n"), "{out}");
    assert!(out.contains("1 ASSO @I1@\n2 ROLE OTHER\n"), "{out}");
    assert!(out.contains("1 FACT x\n2 TYPE Unknown\n"), "{out}");
    assert!(out.contains("0 @R1@ REPO\n1 NAME Unknown\n"), "{out}");
    let (k, out) = repair70("0 @N1@ SNOTE x\n1 TRAN y\n");
    assert_eq!(k, [R::Required]);
    assert!(out.contains("1 TRAN y\n2 LANG und\n"), "{out}");
    let (_, out) = repair70("0 @O1@ OBJE\n1 FILE media/a.xyz\n");
    assert!(out.contains("2 FORM application/octet-stream\n"), "{out}");
    // Nothing stands for a missing date: the superstructure becomes an
    // extension, and its own superstructure follows when it needs it.
    let (k, out) = repair70("0 @I1@ INDI\n1 CHAN\n2 NOTE changed\n0 @O1@ OBJE\n");
    assert_eq!(k, [R::Required, R::Required]);
    assert!(out.contains("1 _CHAN\n2 NOTE changed\n"), "{out}");
    assert!(out.contains("0 @O1@ _OBJE\n"), "{out}");
    // The header is completed as the writer completes every header, which
    // is no repair: 5.5.1 gets its parts and a submitter.
    let (k, out) = repaired("0 HEAD\n0 TRLR\n", GedcomVersion::V5_5_1);
    assert_eq!(k, [], "{k:?}");
    assert_eq!(
        out,
        format!(
            "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n2 FORM LINEAGE-LINKED\n1 CHAR UTF-8\n1 SOUR ged_io\n2 VERS {}\n2 NAME ged_io\n1 SUBM @U1@\n0 @U1@ SUBM\n1 NAME Unknown\n0 TRLR\n",
            env!("CARGO_PKG_VERSION")
        )
    );
    let (k, out) = repaired(
        "0 HEAD\n1 GEDC\n2 VERS 5.5\n2 FORM LINEAGE-LINKED\n1 CHAR ANSEL\n0 TRLR\n",
        GedcomVersion::V7_0,
    );
    assert_eq!(k, [], "{k:?}");
    assert_eq!(out, "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 TRLR\n");
    // A FILE without a 5.5.1 format to stand for its FORM.
    let (_, out) = repair551("0 @O1@ OBJE\n1 FILE a.png\n0 @O2@ OBJE\n1 FILE b.jpeg\n");
    assert!(out.contains("0 @O1@ _OBJE\n1 _FILE a.png\n"), "{out}");
    assert!(
        out.contains("0 @O2@ OBJE\n1 FILE b.jpeg\n2 FORM jpg\n"),
        "{out}"
    );
    // 5.5.1 ASSO.RELA.
    let (_, out) = repair551("0 @I1@ INDI\n1 ASSO @I1@\n");
    assert!(out.contains("1 ASSO @I1@\n2 RELA Unknown\n"), "{out}");
}

#[test]
fn repair_g_empty() {
    let (k, out) = repair70("0 @I1@ INDI\n1 BIRT\n1 NAME\n1 FAMC @VOID@\n2 PEDI BIRTH\n2 NOTE\n");
    assert_eq!(k, [R::Empty, R::Empty, R::Empty]);
    assert!(
        out.contains("0 @I1@ INDI\n1 BIRT Y\n1 FAMC @VOID@\n2 PEDI BIRTH\n0 TRLR"),
        "{out}"
    );
    // 5.5.1 has no such rule.
    assert_eq!(repair551("0 @I1@ INDI\n1 BIRT\n").0, []);
}

#[test]
fn repair_h_identifiers_and_characters() {
    let (k, out) = repair70(
        "0 @i 1@ INDI\n1 FAMS @F1@\n0 @F1@ FAM\n1 HUSB @i 1@\n1 WIFE @I2@\n0 @I2@ INDI\n0 @I2@ INDI\n0 @VOID@ INDI\n1 NOTE a\u{7}b\n2 @X1@ LANG en\n",
    );
    assert_eq!(k, [R::Xref, R::Xref, R::Xref, R::Characters, R::Xref]);
    assert!(out.contains("0 @I_1@ INDI\n1 FAMS @F1@\n"), "{out}");
    assert!(out.contains("1 HUSB @I_1@\n1 WIFE @I2@\n"), "{out}");
    assert!(
        out.contains("0 @I2@ INDI\n0 @I2_2@ INDI\n0 @VOID_@ INDI\n1 NOTE ab\n2 LANG en\n"),
        "{out}"
    );
    let (k, out) = repair551("0 @I1@ INDI\n1 NOTE a\tb\n");
    assert_eq!(k, [R::Characters]);
    assert!(out.contains("1 NOTE a b\n"), "{out}");
    let (k, out) = repair551("0 @#1@ INDI\n0 @ABCDEFGHIJKLMNOPQRSTUVWXYZ@ INDI\n");
    assert_eq!(k, [R::Xref, R::Xref]);
    assert!(
        out.contains("0 @X#1@ INDI\n0 @ABCDEFGHIJKLMNOPQRST@ INDI\n"),
        "{out}"
    );
    // Valid 5.5.1 identifiers stay as they are: spaces, `!`, `:` and a
    // leading `_`; a new one keeps to visible ASCII without `!` or `:`.
    let (k, out) = repair551(
        "0 @_X 1@ INDI\n1 FAMS @A:B@\n0 @A:B@ FAM\n1 HUSB @_X 1@\n0 @I1!2@ _NOTE x\n0 @I1!2@ _NOTE y\n",
    );
    assert_eq!(k, [R::Xref]);
    assert!(
        out.contains("0 @_X 1@ INDI\n1 FAMS @A:B@\n0 @A:B@ FAM\n1 HUSB @_X 1@\n"),
        "{out}"
    );
    assert!(
        out.contains("0 @I1!2@ _NOTE x\n0 @I1_2@ _NOTE y\n"),
        "{out}"
    );
}

#[test]
fn repair_i_header_and_trailer() {
    let (k, out) = repaired(
        "0 @I1@ INDI\n0 @H@ HEAD\n1 GEDC\n2 VERS 7.1\n0 TRLR\n1 _X kept\n0 HEAD\n0 TRLR\n0 @I2@ INDI\n",
        GedcomVersion::V7_0,
    );
    let k: Vec<_> = k.iter().map(|r| r.kind).collect();
    assert_eq!(k, [R::Header, R::Header, R::Xref, R::Header, R::Header]);
    assert_eq!(
        out,
        "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n0 _TRLR\n1 _X kept\n0 _HEAD\n0 @I2@ INDI\n0 TRLR\n"
    );
    let (k, out) = repaired("0 @I1@ INDI\n", GedcomVersion::V7_0);
    assert_eq!(k.len(), 2, "{k:?}");
    assert_eq!(out, "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n0 TRLR\n");
}

#[test]
fn repair_j_record_size() {
    let note = "y".repeat(400);
    let mut body = String::from("0 @I1@ INDI\n");
    for _ in 0..100 {
        body.push_str(&format!("1 NOTE {note}\n"));
    }
    let (k, out) = repair551(&body);
    // Enough notes leave for the rest to fit: 40,000 bytes of text, 32,767
    // kept at most.
    let moved = k.iter().filter(|k| **k == R::RecordSize).count();
    assert!((15..30).contains(&moved), "{moved}");
    assert!(out.contains("1 NOTE @N1@\n"), "{out}");
    assert!(
        out.contains(&format!("0 @N1@ NOTE {}", &note[..200])),
        "{out}"
    );
    // A record nothing can shrink is left as it is.
    let mut body = String::from("0 @I1@ INDI\n");
    for _ in 0..3000 {
        body.push_str("1 _X yyyyyyyyyyyy\n");
    }
    let mut records = parse_tree(&format!("{H551}{body}0 TRLR\n")).to_structures();
    let repairs = conform(&mut records, GedcomVersion::V5_5_1);
    assert!(repairs.is_empty());
    let left = validate(&records, GedcomVersion::V5_5_1);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].kind, K::RecordSize);
}

#[test]
fn repair_keeps_aliases() {
    // A documented alias of a standard record is repaired as that record.
    let schma = "1 SCHMA\n2 TAG _USER https://gedcom.io/terms/v7/record-SUBM\n";
    let (k, out) = repaired(
        &format!("{H70}{schma}1 SUBM @U1@\n0 @U1@ _USER\n1 LANG en GB\n0 TRLR\n"),
        GedcomVersion::V7_0,
    );
    let k: Vec<_> = k.iter().map(|r| r.kind).collect();
    assert_eq!(k, [R::Payload, R::Required]);
    assert!(
        out.contains("0 @U1@ _USER\n1 _LANG en GB\n1 NAME Unknown\n"),
        "{out}"
    );
}

#[test]
fn conform_is_idempotent() {
    let text = format!(
        "{H70}0 @I1@ INDI\n1 SEX M\n1 SEX F\n1 ADDR x\n1 BIRT\n2 DATE 30 FEB 1900\n0 TRLR\n"
    );
    let mut records = parse_tree(&text).to_structures();
    assert!(!conform(&mut records, GedcomVersion::V7_0).is_empty());
    let before = records.clone();
    assert_eq!(conform(&mut records, GedcomVersion::V7_0), []);
    assert_eq!(records, before);
}

/// Each type's substructures are sorted by tag, which
/// `Schema::subs_tagged` searches by halves.
#[test]
fn substructures_are_sorted_by_tag() {
    for schema in [
        &super::tables::V551,
        &super::tables::V70,
        &super::tables::V71,
    ] {
        for id in 0..schema.structs.len() {
            let id = super::schema::StructId::try_from(id).unwrap();
            let subs = schema.subs(id);
            assert!(
                subs.windows(2)
                    .all(|w| schema.tag_id(w[0].id) <= schema.tag_id(w[1].id)),
                "{} {}",
                schema.version,
                schema.name(id)
            );
            for sub in subs {
                let tag = schema.tag_id(sub.id).unwrap();
                assert!(schema.subs_tagged(id, tag).any(|s| s == sub));
            }
        }
    }
}
