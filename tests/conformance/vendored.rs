//! Vendored fixtures with licences that allow it.
//!
//! * gedcom7code/test-files at 4418c40 (Unlicense): 26 GEDCOM 5.5.1 files and
//!   their expected 7.0 conversions, in `fixtures/conformance/test-files/`.
//!   Each file is read and written in its own version (`vendored/5/…`,
//!   `vendored/7/…`), and each 5.5.1 file is written as 7.0 and compared with
//!   its expected conversion (`convert/…`).
//! * ArmidaleSoftware/gedcom7 at dabc9a9 (MIT): the five tiny samples are
//!   inlined in `support/semantic.rs` as unit tests of the equivalence rules.

use crate::support::adapter::{self, Target as WriteTarget};
use crate::support::cases::{self, Case, Kind};
use crate::support::checker::{self, Target};
use crate::support::ratchet::{self, Failure};
use crate::support::semantic;
use crate::support::tree;

fn names(dir: &str) -> Vec<String> {
    let mut v: Vec<String> =
        std::fs::read_dir(crate::support::fixture(&format!("test-files/{dir}")))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|n| n.ends_with(".ged"))
            .map(|n| n.trim_end_matches(".ged").to_string())
            .collect();
    v.sort();
    v
}

/// Files whose input is not conformant 5.5.1 (leniency cases), with the reason.
const LENIENT: [(&str, &str); 10] = [
    ("5/age-invalid", "ages outside the grammar"),
    (
        "5/age-valid",
        "lower-case age keywords, and descriptions on EVEN, which takes no payload in 5.5.1",
    ),
    ("5/atsign", "single `@` in text"),
    ("5/char_ascii_2", "CHAR LATIN1 is not a 5.5.1 character set"),
    ("5/char_utf8-2", "UTF-8 bytes labelled UNICODE"),
    ("5/date-all", "records over 32K and one-digit dual years"),
    ("5/date-dual-invalid", "invalid dual years"),
    ("5/obje-1", "FORM `other` is not a 5.5.1 multimedia format"),
    ("5/tiny-1", "header without GEDC, SOUR or SUBM"),
    (
        "5/xref-case",
        "a dangling pointer: xrefs are case-sensitive (`@test@` against `@TEST@`)",
    ),
];

/// Labels the text checker cannot confirm: they are about the bytes.
const ENCODING_LABELS: [&str; 2] = ["5/char_ascii_2", "5/char_utf8-2"];

/// Values each file must keep in the model.
fn model_expectations(name: &str) -> Vec<&'static str> {
    match name {
        n if n.starts_with("5/char_utf") || n.starts_with("7/char_utf") => vec!["¶ ☺ 𒍅"],
        "5/atsign" | "7/atsign" => vec!["doubled@internal no space", "single@internal no space"],
        "5/xref-case" => vec!["mixed case and space"],
        _ => vec![],
    }
}

fn case_for(name: &str) -> Case {
    let bytes = crate::support::read_fixture(&format!("test-files/{name}.ged"));
    let kind = if LENIENT.iter().any(|(n, _)| *n == name) {
        Kind::L
    } else {
        Kind::C
    };
    let mut c = Case::new(name, kind, bytes);
    c.purpose = "gedcom7code/test-files".into();
    c.model = model_expectations(name)
        .into_iter()
        .map(String::from)
        .collect();
    c
}

/// The labels above are facts about the inputs: the checker agrees.
#[test]
fn vendored_labels_match_the_inputs() {
    let mut wrong = Vec::new();
    for dir in ["5", "7"] {
        for n in names(dir) {
            let name = format!("{dir}/{n}");
            let text = tree::decode_bytes(&crate::support::read_fixture(&format!(
                "test-files/{name}.ged"
            )));
            let target = Target::from_vers(&tree::declared_vers(&text).unwrap_or_default());
            let issues = checker::check(&text, target, None);
            if ENCODING_LABELS.contains(&name.as_str()) {
                continue;
            }
            let lenient = LENIENT.iter().any(|(l, _)| *l == name);
            // Inputs may lack a final terminator or use another encoding's CHAR;
            // those are reader concerns, not what makes a case lenient.
            let real: Vec<_> = issues
                .iter()
                .filter(|i| !matches!(i.rule, "eol-final" | "char-mismatch"))
                .collect();
            if real.is_empty() == lenient {
                wrong.push(format!("{name}: lenient={lenient} but issues={real:?}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn same_version_round_trips() {
    let mut failures = Vec::new();
    let mut n = 0;
    for dir in ["5", "7"] {
        for name in names(dir) {
            n += 1;
            failures.extend(cases::run("vendored", &case_for(&format!("{dir}/{name}"))).failures);
        }
    }
    ratchet::verify("vendored", n, failures);
}

/// 5.5.1 → 7.0 against the expected conversions, under the semantic rules of
/// conversion mode.
#[test]
fn conversion_to_7_matches_the_expected_files() {
    let mut failures = Vec::new();
    let sevens = names("7");
    let mut n = 0;
    for name in names("5").into_iter().filter(|n| sevens.contains(n)) {
        n += 1;
        let id = format!("convert/{name}");
        let src = crate::support::read_fixture(&format!("test-files/5/{name}.ged"));
        let expected = tree::decode_bytes(&crate::support::read_fixture(&format!(
            "test-files/7/{name}.ged"
        )));
        let m = match adapter::read(&src) {
            Ok(m) => m,
            Err(e) => {
                failures.push(Failure::new(
                    &id,
                    "parse",
                    if e.starts_with("PANIC") {
                        "PANIC"
                    } else {
                        "FATAL"
                    },
                    e,
                ));
                continue;
            }
        };
        let out = match adapter::write(&m, WriteTarget::V70) {
            Ok(o) => o,
            Err(e) => {
                failures.push(Failure::new(&id, "write", "FATAL", e));
                continue;
            }
        };
        for i in checker::check(&out, Target::V70, Some(&expected)) {
            failures.push(Failure::new(
                &id,
                format!("output:{}", i.rule),
                "NONCONFORMANT",
                i.to_string(),
            ));
        }
        let d = semantic::compare(&expected, &out, &semantic::Options::conversion());
        if !d.is_empty() {
            failures.push(Failure::new(
                &id,
                "convert",
                "DIFF",
                format!("{} differences: {}", d.len(), d.join("; ")),
            ));
        }
    }
    ratchet::verify("convert", n, failures);
}
