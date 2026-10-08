//! Reading any encoding and any line terminator, in memory and streaming.
//!
//! The cases follow the conformance suite's encoding (`enc-*`, `eol-*`) and
//! line (`G7-*`) cases; all data is fictitious.

use std::io::BufReader;

use ged_io::encoding::{decode, encode_to_bytes, DecodeReader, GedcomEncoding};
use ged_io::types::GedcomData;
use ged_io::{GedcomBuilder, GedcomStreamParser};

/// A 5.5.1 file with one individual named `name`, `CHAR` set to `char`.
fn file(char: &str, name: &[u8]) -> Vec<u8> {
    let mut bytes = format!(
        "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n2 FORM LINEAGE-LINKED\n1 CHAR {char}\n0 @I1@ INDI\n1 NAME "
    )
    .into_bytes();
    bytes.extend_from_slice(name);
    bytes.extend_from_slice(b" /Exemple/\n0 TRLR\n");
    bytes
}

fn read(bytes: &[u8]) -> GedcomData {
    GedcomBuilder::new().build_from_bytes(bytes).unwrap()
}

fn stream(bytes: &[u8]) -> GedcomData {
    GedcomStreamParser::new(BufReader::with_capacity(7, bytes))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn name(data: &GedcomData) -> &str {
    data.individuals[0].names[0].value.as_deref().unwrap()
}

/// Reads `bytes` both ways and checks the name and the reported encoding.
fn check(bytes: &[u8], want_name: &str, want_encoding: GedcomEncoding) {
    let memory = read(bytes);
    assert_eq!(name(&memory), want_name);
    assert_eq!(stream(bytes), memory);
    assert_eq!(decode(bytes).encoding, want_encoding);
    assert_eq!(
        DecodeReader::new(bytes).unwrap().encoding(),
        want_encoding,
        "stream encoding"
    );
}

#[test]
fn enc_ansel_combining_is_composed() {
    check(
        &file("ANSEL", b"Andr\xE2e"),
        "André /Exemple/",
        GedcomEncoding::Ansel,
    );
}

#[test]
fn enc_ansel_cr() {
    let bytes: Vec<u8> = file("ANSEL", b"Andr\xE2e")
        .into_iter()
        .map(|b| if b == b'\n' { b'\r' } else { b })
        .collect();
    check(&bytes, "André /Exemple/", GedcomEncoding::Ansel);
}

#[test]
fn enc_ansel_mark_split_by_conc() {
    let bytes = b"0 HEAD\r\n1 CHAR ANSEL\r\n0 @I1@ INDI\r\n1 NOTE Andr\xE2\r\n2 CONC e lives here\r\n0 TRLR\r\n";
    let data = read(bytes);
    assert_eq!(
        data.individuals[0].notes[0].value.as_deref(),
        Some("André lives here")
    );
    assert_eq!(stream(bytes), data);
}

#[test]
fn enc_ansi_latin1() {
    check(
        &file("ANSI", b"Andr\xE9"),
        "André /Exemple/",
        GedcomEncoding::Windows1252,
    );
}

#[test]
fn enc_ascii_declared_latin1_bytes() {
    check(
        &file("ASCII", b"Andr\xE9"),
        "André /Exemple/",
        GedcomEncoding::Windows1252,
    );
}

#[test]
fn enc_cp1252_euro() {
    check(
        &file("WINDOWS-1252", b"Prix 5\x80 \x91q\x92"),
        "Prix 5€ ‘q’ /Exemple/",
        GedcomEncoding::Windows1252,
    );
    check(
        &file("cp1252", b"Prix 5\x80"),
        "Prix 5€ /Exemple/",
        GedcomEncoding::Windows1252,
    );
}

#[test]
fn enc_latin1_declared() {
    check(
        &file("ISO-8859-1", b"Andr\xE9"),
        "André /Exemple/",
        GedcomEncoding::Iso8859_1,
    );
}

#[test]
fn enc_macintosh_and_ibmpc() {
    check(
        &file("MACINTOSH", b"Andr\x8E"),
        "André /Exemple/",
        GedcomEncoding::MacRoman,
    );
    check(
        &file("IBMPC", b"Andr\x82"),
        "André /Exemple/",
        GedcomEncoding::Cp437,
    );
}

#[test]
fn enc_undeclared_latin1() {
    let bytes = b"0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NAME Andr\xE9 /Exemple/\n0 TRLR\n";
    check(bytes, "André /Exemple/", GedcomEncoding::Windows1252);
}

#[test]
fn enc_utf8_mislabelled_unicode_and_ansel() {
    check(
        &file("UNICODE", "André".as_bytes()),
        "André /Exemple/",
        GedcomEncoding::Utf8,
    );
    check(
        &file("ANSEL", "André".as_bytes()),
        "André /Exemple/",
        GedcomEncoding::Utf8,
    );
}

#[test]
fn enc_utf8_cr_and_bom_crlf() {
    let cr: Vec<u8> = file("UTF-8", "André".as_bytes())
        .into_iter()
        .map(|b| if b == b'\n' { b'\r' } else { b })
        .collect();
    check(&cr, "André /Exemple/", GedcomEncoding::Utf8);
    let mut bom = vec![0xEF, 0xBB, 0xBF];
    bom.extend_from_slice(
        "0 HEAD\r\n1 GEDC\r\n2 VERS 7.0\r\n0 @I1@ INDI\r\n1 NAME André /Exemple/\r\n0 TRLR\r\n"
            .as_bytes(),
    );
    check(&bom, "André /Exemple/", GedcomEncoding::Utf8);
}

#[test]
fn enc_utf16_with_and_without_bom() {
    let text = String::from_utf8(file("UNICODE", "André".as_bytes())).unwrap();
    for eol in ["\n", "\r"] {
        let text = text.replace('\n', eol);
        let le = encode_to_bytes(&text, GedcomEncoding::Utf16Le).unwrap();
        let be = encode_to_bytes(&text, GedcomEncoding::Utf16Be).unwrap();
        check(&le, "André /Exemple/", GedcomEncoding::Utf16Le);
        check(&be, "André /Exemple/", GedcomEncoding::Utf16Be);
        // Without the byte order mark.
        check(&le[2..], "André /Exemple/", GedcomEncoding::Utf16Le);
        check(&be[2..], "André /Exemple/", GedcomEncoding::Utf16Be);
    }
}

#[test]
fn eol_lfcr_and_mixed() {
    let lfcr = b"0 HEAD\n\r1 GEDC\n\r2 VERS 5.5.1\n\r1 CHAR UTF-8\n\r0 @I1@ INDI\n\r1 NAME Ann /Example/\n\r0 TRLR\n\r";
    assert_eq!(name(&read(lfcr)), "Ann /Example/");
    assert_eq!(stream(lfcr), read(lfcr));
    let mixed =
        b"0 HEAD\n1 GEDC\n2 VERS 5.5.1\n1 CHAR UTF-8\n0 @I1@ INDI\r1 NAME Ann /Example/\n0 TRLR\n";
    assert_eq!(name(&read(mixed)), "Ann /Example/");
    assert_eq!(stream(mixed), read(mixed));
}

#[test]
fn blank_and_whitespace_only_lines_are_skipped() {
    let bytes = b"\n0 HEAD\n1 GEDC\n2 VERS 7.0\n\n0 @I1@ INDI\n  \t\n1 SEX M\n\r\n0 TRLR\n\n";
    let data = read(bytes);
    assert!(data.individuals[0].sex.is_some());
    assert_eq!(stream(bytes), data);
}

#[test]
fn empty_cont_with_trailing_delimiter_in_crlf() {
    // D1: `2 CONT ` then CRLF used to swallow the next line.
    let bytes = b"0 HEAD\r\n1 GEDC\r\n2 VERS 5.5.1\r\n0 @I1@ INDI\r\n1 NOTE first\r\n2 CONT \r\n2 CONT third\r\n0 TRLR\r\n";
    let data = read(bytes);
    assert_eq!(
        data.individuals[0].notes[0].value.as_deref(),
        Some("first\n\nthird")
    );
    assert_eq!(stream(bytes), data);
}

/// The b1-gen matrix: one fictitious body in six encodings and three line
/// terminators reads to the same model, in memory and streaming.
#[test]
fn encoding_and_terminator_matrix() {
    let body = "0 @I1@ INDI\n1 NAME Zoé /Straße/\n1 SEX F\n1 BIRT\n2 DATE 1 JAN 1900\n\
                2 PLAC Ærøskøbing\n1 NOTE Première ligne\n2 CONT seconde ligne\n\
                0 @I2@ INDI\n1 NAME Jürgen /Ødegård/\n0 @F1@ FAM\n1 WIFE @I1@\n1 HUSB @I2@\n0 TRLR\n";
    let mut reference: Option<GedcomData> = None;
    // UTF-16 output carries its own byte order mark; `bom` adds the UTF-8 one.
    for (label, encoding, bom) in [
        ("UTF-8", GedcomEncoding::Utf8, false),
        ("UTF-8", GedcomEncoding::Utf8, true),
        ("UNICODE", GedcomEncoding::Utf16Le, false),
        ("UNICODE", GedcomEncoding::Utf16Be, false),
        ("ANSEL", GedcomEncoding::Ansel, false),
        ("ISO-8859-1", GedcomEncoding::Iso8859_1, false),
    ] {
        for eol in ["\n", "\r\n", "\r"] {
            let text =
                format!("0 HEAD\n1 GEDC\n2 VERS 5.5.1\n1 CHAR {label}\n{body}").replace('\n', eol);
            let mut bytes = if bom {
                vec![0xEF, 0xBB, 0xBF]
            } else {
                Vec::new()
            };
            bytes.extend(encode_to_bytes(&text, encoding).unwrap());
            let mut data = read(&bytes);
            assert_eq!(stream(&bytes), data, "{label} {eol:?}");
            // Only the declared character set differs.
            data.header.as_mut().unwrap().encoding = None;
            match &reference {
                None => reference = Some(data),
                Some(r) => assert_eq!(&data, r, "{label} {eol:?}"),
            }
        }
    }
    let r = reference.unwrap();
    assert_eq!(name(&r), "Zoé /Straße/");
    assert_eq!(
        r.individuals[0].notes[0].value.as_deref(),
        Some("Première ligne\nseconde ligne")
    );
}
