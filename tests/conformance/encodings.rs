//! Encodings and line terminators, with byte streams built at test time from
//! Unicode text by the suite's own encoders (so no binary fixture is needed).
//!
//! * `encoding/…`: one fictitious record per character set, label and
//!   terminator case (C when the label is a 5.5.1 character set matching the
//!   bytes, L otherwise).
//! * `matrix/<encoding>-<eol>`: one 30-line body in six encodings and three
//!   terminators; every cell must read to the same data.
//! * `ansel/…`: every ANSEL spacing character, every combining mark on every
//!   ASCII letter and stacked marks, decoded to NFC (`support/ansel_table.rs`).

use crate::support::adapter;
use crate::support::ansel_table::{COMPOSED, MARKS, SPACING, STACKED};
use crate::support::cases::{self, Case, Kind};
use crate::support::ratchet::{self, Failure};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enc {
    Utf8,
    Utf8Bom,
    Utf16Le { bom: bool },
    Utf16Be { bom: bool },
    Latin1,
    Latin9,
    Cp1252,
    MacRoman,
    Cp437,
    Ansel,
}

const CP1252_HIGH: [u32; 32] = [
    0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039,
    0x0152, 0, 0x017D, 0, 0, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC,
    0x2122, 0x0161, 0x203A, 0x0153, 0, 0x017E, 0x0178,
];

fn ansel_map() -> HashMap<char, Vec<u8>> {
    let mut m = HashMap::new();
    for &(b, c, _) in SPACING.iter().rev() {
        // Reversed so that GEDCOM's CF wins over MARC-8's C7 for eszett.
        m.insert(c, vec![b]);
    }
    for &(b, base, nfc) in COMPOSED {
        let mut chars = nfc.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            m.entry(c).or_insert_with(|| vec![b, base as u8]);
        }
    }
    m
}

/// Encodes text; panics on a character the encoding cannot represent (a
/// mistake in the test, not in ged_io).
pub fn encode(text: &str, enc: Enc) -> Vec<u8> {
    let one = |c: char, f: &dyn Fn(char) -> Option<u8>| -> u8 {
        f(c).unwrap_or_else(|| panic!("{c:?} has no {enc:?} code"))
    };
    match enc {
        Enc::Utf8 => text.as_bytes().to_vec(),
        Enc::Utf8Bom => [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat(),
        Enc::Utf16Le { bom } => {
            let mut v = if bom { vec![0xFF, 0xFE] } else { vec![] };
            text.encode_utf16().for_each(|u| v.extend(u.to_le_bytes()));
            v
        }
        Enc::Utf16Be { bom } => {
            let mut v = if bom { vec![0xFE, 0xFF] } else { vec![] };
            text.encode_utf16().for_each(|u| v.extend(u.to_be_bytes()));
            v
        }
        Enc::Latin1 => text
            .chars()
            .map(|c| one(c, &|c| u8::try_from(c as u32).ok()))
            .collect(),
        Enc::Latin9 => text
            .chars()
            .map(|c| {
                one(c, &|c| {
                    if c == '€' {
                        Some(0xA4)
                    } else {
                        u8::try_from(c as u32).ok()
                    }
                })
            })
            .collect(),
        Enc::Cp1252 => text
            .chars()
            .map(|c| {
                one(c, &|c| {
                    CP1252_HIGH
                        .iter()
                        .position(|&u| u != 0 && u == c as u32)
                        .map(|i| 0x80 + i as u8)
                        .or_else(|| {
                            u8::try_from(c as u32)
                                .ok()
                                .filter(|b| !(0x80..0xA0).contains(b))
                        })
                })
            })
            .collect(),
        Enc::MacRoman => text
            .chars()
            .map(|c| {
                one(c, &|c| {
                    if c == 'é' {
                        Some(0x8E)
                    } else {
                        u8::try_from(c as u32).ok().filter(u8::is_ascii)
                    }
                })
            })
            .collect(),
        Enc::Cp437 => text
            .chars()
            .map(|c| {
                one(c, &|c| {
                    if c == 'é' {
                        Some(0x82)
                    } else {
                        u8::try_from(c as u32).ok().filter(u8::is_ascii)
                    }
                })
            })
            .collect(),
        Enc::Ansel => {
            // Marks precede their base in ANSEL, in the order Unicode lists them.
            let m = ansel_map();
            let marks: HashMap<char, u8> = MARKS.iter().map(|&(b, c, _)| (c, b)).collect();
            let mut v = Vec::new();
            let (mut base_at, mut n_marks) = (0, 0);
            for c in text.chars() {
                if let Some(&b) = marks.get(&c) {
                    v.insert(base_at + n_marks, b);
                    n_marks += 1;
                    continue;
                }
                base_at = v.len();
                if c.is_ascii() {
                    v.push(c as u8);
                    n_marks = 0;
                } else {
                    let code = m
                        .get(&c)
                        .unwrap_or_else(|| panic!("{c:?} has no ANSEL code"));
                    n_marks = code.len() - 1;
                    v.extend(code);
                }
            }
            v
        }
    }
}

fn with_eol(text: &str, eol: &str) -> String {
    text.replace('\n', eol)
}

/// A 5.5.1 dataset around `body`, declaring `char`.
fn dataset551(char: Option<&str>, body: &str) -> String {
    let char = char.map(|c| format!("1 CHAR {c}\n")).unwrap_or_default();
    format!(
        "0 HEAD\n1 SOUR EXAMPLE_APP\n1 SUBM @U0@\n1 GEDC\n2 VERS 5.5.1\n2 FORM LINEAGE-LINKED\n{char}{body}0 @U0@ SUBM\n1 NAME Example Submitter\n0 TRLR\n"
    )
}

fn enc_case(id: &str, kind: Kind, text: String, bytes: Vec<u8>, keep: &[&str]) -> Case {
    let mut c = Case::new(id, kind, bytes);
    c.purpose = id.into();
    c.model = keep.iter().map(|s| (*s).to_string()).collect();
    c.text = Some(text);
    c
}

pub fn encoding_cases() -> Vec<Case> {
    let name = "0 @I1@ INDI\n1 NAME André /Exemple/\n";
    let euro = "0 @I1@ INDI\n1 NAME Prix 5€ ‘q’ /Exemple/\n";
    let euro9 = "0 @I1@ INDI\n1 NAME Prix 5€ /Exemple/\n";
    let mut v = Vec::new();
    let mut add = |id: &str,
                   kind: Kind,
                   label: Option<&str>,
                   body: &str,
                   enc: Enc,
                   eol: &str,
                   keep: &[&str]| {
        let text = with_eol(&dataset551(label, body), eol);
        v.push(enc_case(id, kind, text.clone(), encode(&text, enc), keep));
    };
    add(
        "utf8-cr",
        Kind::C,
        Some("UTF-8"),
        name,
        Enc::Utf8,
        "\r",
        &["André"],
    );
    add(
        "utf8-mislabel-unicode",
        Kind::L,
        Some("UNICODE"),
        name,
        Enc::Utf8,
        "\n",
        &["André"],
    );
    add(
        "utf16le-nobom",
        Kind::C,
        Some("UNICODE"),
        name,
        Enc::Utf16Le { bom: false },
        "\n",
        &["André"],
    );
    add(
        "utf16le-cr",
        Kind::C,
        Some("UNICODE"),
        name,
        Enc::Utf16Le { bom: true },
        "\r",
        &["André"],
    );
    add(
        "utf16be-crlf",
        Kind::C,
        Some("UNICODE"),
        name,
        Enc::Utf16Be { bom: true },
        "\r\n",
        &["André"],
    );
    add(
        "latin1-declared",
        Kind::L,
        Some("ISO-8859-1"),
        name,
        Enc::Latin1,
        "\n",
        &["André"],
    );
    add(
        "latin9-euro",
        Kind::L,
        Some("ISO-8859-15"),
        euro9,
        Enc::Latin9,
        "\n",
        &["Prix 5€"],
    );
    add(
        "ansi-cp1252",
        Kind::L,
        Some("ANSI"),
        euro,
        Enc::Cp1252,
        "\n",
        &["Prix 5€ ‘q’"],
    );
    add(
        "windows-1252-euro",
        Kind::L,
        Some("WINDOWS-1252"),
        euro,
        Enc::Cp1252,
        "\n",
        &["Prix 5€ ‘q’"],
    );
    add(
        "cp1252-lower-case-label",
        Kind::L,
        Some("cp1252"),
        euro,
        Enc::Cp1252,
        "\n",
        &["Prix 5€ ‘q’"],
    );
    add(
        "ascii-declared-latin1-bytes",
        Kind::L,
        Some("ASCII"),
        name,
        Enc::Latin1,
        "\n",
        &["André"],
    );
    add(
        "undeclared-latin1",
        Kind::L,
        None,
        name,
        Enc::Latin1,
        "\n",
        &["André"],
    );
    add(
        "macintosh",
        Kind::L,
        Some("MACINTOSH"),
        name,
        Enc::MacRoman,
        "\n",
        &["André"],
    );
    add(
        "ibmpc-cp437",
        Kind::L,
        Some("IBMPC"),
        name,
        Enc::Cp437,
        "\n",
        &["André"],
    );
    add(
        "ansel-lf",
        Kind::C,
        Some("ANSEL"),
        name,
        Enc::Ansel,
        "\n",
        &["André"],
    );
    add(
        "ansel-cr",
        Kind::C,
        Some("ANSEL"),
        name,
        Enc::Ansel,
        "\r",
        &["André"],
    );
    add(
        "eol-lfcr",
        Kind::C,
        Some("UTF-8"),
        name,
        Enc::Utf8,
        "\n\r",
        &["André"],
    );
    // UTF-8 with a BOM, 7.0, CRLF.
    let t7 =
        "0 HEAD\r\n1 GEDC\r\n2 VERS 7.0\r\n0 @I1@ INDI\r\n1 NAME André /Exemple/\r\n0 TRLR\r\n"
            .to_string();
    v.push(enc_case(
        "utf8-bom-v7-crlf",
        Kind::C,
        t7.clone(),
        encode(&t7, Enc::Utf8Bom),
        &["André"],
    ));
    // A lone CR inside an LF file.
    let mixed = dataset551(Some("UTF-8"), "0 @I1@ INDI\r1 NAME Ann /Example/\n");
    v.push(enc_case(
        "eol-mixed-cr-lf",
        Kind::L,
        mixed.clone(),
        mixed.into_bytes(),
        &["Ann /Example/"],
    ));
    // An ANSEL mark at the end of a line, its base letter on the CONC line.
    let text = dataset551(
        Some("ANSEL"),
        "0 @I1@ INDI\n1 NOTE Born in Andr\u{301}\n2 CONC e Sampleton\n",
    );
    let mut bytes = encode(
        &dataset551(
            Some("ANSEL"),
            "0 @I1@ INDI\n1 NOTE Born in Andr~\n2 CONC e Sampleton\n",
        ),
        Enc::Ansel,
    );
    let tilde = bytes.iter().position(|&b| b == b'~').unwrap();
    bytes[tilde] = 0xE2;
    let mut c = enc_case(
        "ansel-conc-diacritic",
        Kind::C,
        text,
        bytes,
        &["Born in André Sampleton"],
    );
    c.text = Some(dataset551(
        Some("ANSEL"),
        "0 @I1@ INDI\n1 NOTE Born in André Sampleton\n",
    ));
    v.push(c);
    v
}

#[test]
fn encodings_and_terminators() {
    let all = encoding_cases();
    let mut failures = Vec::new();
    for c in &all {
        failures.extend(cases::run("encoding", c).failures);
    }
    ratchet::verify("encoding", all.len(), failures);
}

/// The 30-line body of the matrix. `extended` lines need a Unicode encoding.
fn matrix_body(extended: bool) -> String {
    let mut b = String::from(
        "0 @I1@ INDI\n1 NAME Zoé /Exemple/\n2 GIVN Zoé\n2 SURN Exemple\n1 SEX F\n1 BIRT\n2 DATE 12 MAR 1901\n2 PLAC Saint-Étienne, Région\n\
         1 FAMS @F1@\n1 NOTE Première ligne\n2 CONT Deuxième ligne, Weiß\n0 @I2@ INDI\n1 NAME Jürgen /Weiß/\n2 GIVN Jürgen\n2 SURN Weiß\n1 SEX M\n\
         1 FAMS @F1@\n1 OCCU Bäcker\n0 @F1@ FAM\n1 HUSB @I2@\n1 WIFE @I1@\n1 MARR\n2 DATE 3 JUN 1925\n2 PLAC Façade, Île\n0 @S1@ SOUR\n1 TITL Registre paroissial\n",
    );
    if extended {
        b += "0 @I3@ INDI\n1 NAME Жанна /Пример/\n1 NOTE Prix 5 €\n";
    }
    b
}

#[test]
fn encoding_and_terminator_matrix() {
    let encs: [(&str, Enc, &str, Kind, bool); 6] = [
        ("utf8", Enc::Utf8, "UTF-8", Kind::C, true),
        ("utf8-bom", Enc::Utf8Bom, "UTF-8", Kind::C, true),
        (
            "utf16le",
            Enc::Utf16Le { bom: true },
            "UNICODE",
            Kind::C,
            true,
        ),
        (
            "utf16be",
            Enc::Utf16Be { bom: true },
            "UNICODE",
            Kind::C,
            true,
        ),
        ("ansel", Enc::Ansel, "ANSEL", Kind::C, false),
        ("latin1", Enc::Latin1, "ISO-8859-1", Kind::L, false),
    ];
    let eols = [("lf", "\n"), ("crlf", "\r\n"), ("cr", "\r")];
    let mut failures = Vec::new();
    let mut n = 0;
    for (ename, enc, label, kind, ext) in encs {
        for (lname, eol) in eols {
            n += 1;
            let text = with_eol(&dataset551(Some(label), &matrix_body(ext)), eol);
            let mut keep = vec![
                "Zoé",
                "Saint-Étienne, Région",
                "Deuxième ligne, Weiß",
                "Jürgen",
                "Façade, Île",
            ];
            if ext {
                keep.extend(["Жанна", "Prix 5 €"]);
            }
            let c = enc_case(
                &format!("{ename}-{lname}"),
                kind,
                text.clone(),
                encode(&text, enc),
                &keep,
            );
            failures.extend(cases::run("matrix", &c).failures);
        }
    }
    ratchet::verify("matrix", n, failures);
}

#[test]
fn ansel_repertoire() {
    let mut body = String::from("0 @I1@ INDI\n1 NAME Ann /Example/\n");
    let mut expect: Vec<(String, String)> = Vec::new();
    for &(b, c, _) in SPACING {
        body += &format!("1 NOTE spacing {b:02X}: x{c}y\n");
        expect.push((format!("char-{b:02X}"), format!("spacing {b:02X}: x{c}y")));
    }
    for &(m, _, _) in MARKS {
        let words: Vec<&str> = COMPOSED.iter().filter(|r| r.0 == m).map(|r| r.2).collect();
        let line = format!("mark {m:02X}: {}", words.join(" "));
        body += &format!("1 NOTE {line}\n");
        expect.push((format!("mark-{m:02X}"), line));
    }
    for (i, &(_, _, nfc)) in STACKED.iter().enumerate() {
        let line = format!("stacked {i}: {nfc}");
        body += &format!("1 NOTE {line}\n");
        expect.push((format!("stacked-{i}"), line));
    }
    let text = dataset551(Some("ANSEL"), &body);
    // Encode: composed letters through the table, stacked marks by hand.
    let mut bytes = Vec::new();
    for line in text.split_inclusive('\n') {
        if let Some(rest) = line.strip_prefix("1 NOTE stacked ") {
            let i: usize = rest[..rest.find(':').unwrap()].parse().unwrap();
            let (marks, base, _) = STACKED[i];
            bytes.extend(format!("1 NOTE stacked {i}: ").as_bytes());
            bytes.extend(marks);
            bytes.push(base as u8);
            bytes.push(b'\n');
        } else {
            bytes.extend(encode(line, Enc::Ansel));
        }
    }
    let mut failures = Vec::new();
    let model = match adapter::read(&bytes) {
        Ok(m) => m,
        Err(e) => {
            failures.push(Failure::new("ansel/file", "parse", "FATAL", e));
            ratchet::verify("ansel", 1, failures);
            return;
        }
    };
    for (id, want) in &expect {
        if !adapter::model_contains(&model, want) {
            failures.push(Failure::new(
                format!("ansel/{id}"),
                "decode",
                "CHANGED",
                format!("model lacks {want:?}"),
            ));
        }
    }
    // Written as UTF-8 and as ANSEL, the text reads back unchanged.
    let c = enc_case("file", Kind::C, text, bytes, &[]);
    let run = cases::run("ansel", &c);
    failures.extend(run.failures);
    if let Some(out) = run.output {
        let back =
            adapter::encode_ansel(&out).and_then(|b| adapter::read(&b).map(|m| adapter::dump(&m)));
        match back {
            Ok(d) if d == adapter::dump(&adapter::read(out.as_bytes()).unwrap()) => {}
            Ok(_) => failures.push(Failure::new(
                "ansel/file",
                "ansel-write",
                "CHANGED",
                "ANSEL write then read differs",
            )),
            Err(e) => failures.push(Failure::new("ansel/file", "ansel-write", "FATAL", e)),
        }
    }
    ratchet::verify("ansel", expect.len() + 1, failures);
}
