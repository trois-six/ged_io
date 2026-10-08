//! Project-authored "everything" fixtures, fictitious throughout.
//!
//! * `maximal551.ged`: every GEDCOM 5.5.1 structure at least once and every
//!   repeatable one twice; it replaces `allged.ged` (non-commercial terms) as
//!   the all-structures fixture and the benchmark input.
//! * `extensions70.ged`: GEDCOM 7.0 extensions in every documented form.
//! * `full551.ged` and `full70.ged`: the hand-written files of the 5.5.1
//!   and 7.0 conformance reviews, every structure of their chapters with
//!   invented content. `full551.ged` has a few values 5.5.1 does not permit
//!   (`DIV Y`, a Julian dual year): a leniency case.

use crate::support::cases::{self, Case, Kind};
use crate::support::checker::{self, Target};
use crate::support::ratchet;
use crate::support::read_fixture;

/// Structure types the fixture does not use, with the reason.
const NOT_COVERED_551: [(&str, &str); 0] = [];

/// Every structure type of the 5.5.1 tables is used at least once, and every
/// repeatable one twice under one superstructure. (Splices such as
/// EVENT_DETAIL are covered once, not under each of their 40 events.)
/// The structure types of the 5.5.1 tables the text lacks, or does not
/// repeat: (type, found, wanted).
fn missing_551(text: &str) -> Vec<(&'static str, usize, usize)> {
    let cov = checker::coverage(text, Target::V551);
    let mut types: std::collections::BTreeMap<&'static str, (usize, bool)> =
        std::collections::BTreeMap::new();
    for s in Target::V551.spec().subs {
        if s.tag == "TRLR" {
            continue;
        }
        let n = cov.get(&(s.sup, s.ty)).copied().unwrap_or(0);
        let e = types.entry(s.ty).or_insert((0, false));
        if s.max != 1 {
            e.1 = true;
            e.0 = e.0.max(n);
        } else {
            e.0 = e.0.max(n.min(1));
        }
    }
    types
        .into_iter()
        .filter(|(ty, _)| !NOT_COVERED_551.iter().any(|(t, _)| t == ty))
        .map(|(ty, (n, rep))| (ty, n, if rep { 2 } else { 1 }))
        .filter(|(_, n, want)| n < want)
        .collect()
}

/// Appends one fictitious record per missing structure type, built by the
/// probe generator. Run once with `MAXIMAL551_COMPLETE=1` to extend the
/// fixture after a table change; the file is then reviewed and committed.
fn complete_551(seed: &str) -> String {
    use crate::generated::Gen;
    let g = Gen::new(Target::V551);
    let paths = g.paths();
    let mut extra = String::new();
    let mut n = 0;
    // The seed's own records stand in for the generator's background ones.
    let seed_xrefs: Vec<(String, String)> = seed
        .lines()
        .filter_map(|l| {
            let mut p = l.split(' ');
            (p.next() == Some("0")).then(|| {
                (
                    p.next().unwrap_or("").to_string(),
                    p.next().unwrap_or("").to_string(),
                )
            })
        })
        .filter(|(x, _)| x.starts_with('@'))
        .collect();
    let map = |frag: &str| -> String {
        let mut f = frag.to_string();
        for (tag, pre) in [
            ("INDI", "I"),
            ("FAM", "F"),
            ("SOUR", "S"),
            ("REPO", "R"),
            ("OBJE", "O"),
            ("NOTE", "N"),
            ("SUBM", "U"),
            ("SUBN", "B"),
        ] {
            let mine: Vec<&String> = seed_xrefs
                .iter()
                .filter(|(_, t)| t == tag)
                .map(|(x, _)| x)
                .collect();
            for i in 1..=2 {
                if let Some(x) = mine.get(i - 1).or(mine.first()) {
                    f = f.replace(&format!("@{pre}{i}@"), x);
                }
            }
        }
        f
    };
    for (ty, _, want) in missing_551(seed) {
        let Some(sub) = Target::V551
            .spec()
            .subs
            .iter()
            .find(|s| s.ty == ty && (want == 1 || s.max != 1) && paths.contains_key(s.sup))
        else {
            continue;
        };
        let parent = sub.sup.rsplit(['-', '.']).next().unwrap_or("");
        let values: Vec<Option<String>> = (0..want)
            .map(|i| g.sample(sub.ty, sub.tag, parent, i))
            .collect();
        let Some((doc, _)) = g.build(sub, &values, &paths) else {
            continue;
        };
        // The probe record is the first record after HEAD.
        let mut lines = doc.lines().skip_while(|l| {
            !l.starts_with("0 @P0@") && !(l.starts_with("0 ") && l.contains("@P0@"))
        });
        let Some(first) = lines.next() else { continue };
        n += 1;
        let mut frag = format!("{}\n", first.replace("@P0@", &format!("@MX{n}@")));
        for l in lines.take_while(|l| !l.starts_with("0 ")) {
            frag += l;
            frag += "\n";
        }
        extra += &map(&frag);
    }
    seed.replacen("0 TRLR", &format!("{extra}0 TRLR"), 1)
}

#[test]
fn maximal551_covers_the_tables() {
    let mut text = String::from_utf8(read_fixture("maximal551.ged")).unwrap();
    if std::env::var_os("MAXIMAL551_COMPLETE").is_some() {
        text = complete_551(&text);
        std::fs::write(crate::support::fixture("maximal551.ged"), &text).unwrap();
    }
    let missing: Vec<String> = missing_551(&text)
        .iter()
        .map(|(t, n, w)| format!("{t}\t{n}/{w}"))
        .collect();
    assert!(
        missing.is_empty(),
        "{} structure types missing or not repeated:\n{}",
        missing.len(),
        missing.join("\n")
    );
}

#[test]
fn maximal551_is_conformant() {
    let text = String::from_utf8(read_fixture("maximal551.ged")).unwrap();
    let issues = checker::check(&text, Target::V551, None);
    assert!(
        issues.is_empty(),
        "{}",
        issues
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Both fixtures as conformance cases: read, written, re-read, checked.
#[test]
fn maximal_fixtures_round_trip() {
    let mut failures = Vec::new();
    let ext = read_fixture("extensions70.ged");
    let mut c = Case::new("extensions70", Kind::C, ext.clone());
    c.purpose = "every documented form of 7.0 extension".into();
    c.wants = String::from_utf8(ext)
        .unwrap()
        .lines()
        .filter(|l| l.contains('_') && !l.starts_with("2 TAG"))
        .map(String::from)
        .collect();
    failures.extend(cases::run("fixture", &c).failures);
    let mut m = Case::new("maximal551", Kind::C, read_fixture("maximal551.ged"));
    m.purpose = "every 5.5.1 structure type".into();
    failures.extend(cases::run("fixture", &m).failures);
    let mut f = Case::new("full551", Kind::L, read_fixture("full551.ged"));
    f.purpose = "the 5.5.1 review's every-structure file".into();
    failures.extend(cases::run("fixture", &f).failures);
    let mut f = Case::new("full70", Kind::C, read_fixture("full70.ged"));
    f.purpose = "the 7.0 review's every-structure file".into();
    failures.extend(cases::run("fixture", &f).failures);
    ratchet::verify("fixture", 4, failures);
}

#[test]
fn extensions70_is_conformant() {
    let text = String::from_utf8(read_fixture("extensions70.ged")).unwrap();
    let issues = checker::check(&text, Target::V70, None);
    assert!(
        issues.is_empty(),
        "{}",
        issues
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    );
}
