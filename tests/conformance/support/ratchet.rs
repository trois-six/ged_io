//! The known-gaps ratchet.
//!
//! `tests/fixtures/conformance/known_gaps.tsv` lists every check that is
//! expected to fail today, with its root cause (TS1–TS26 of the test-suite
//! plan) and its failure class. A test family fails when one of its checks
//! fails without a row, when a row's class no longer matches, or when a row
//! matches nothing any more (the gap was fixed: delete the row in the same
//! change). Gaps therefore cannot hide and fixes cannot go unnoticed.
//!
//! Columns: `id`, `check`, `cause`, `current`, `next`, `note`.
//! * `id` is `<family>/<case>`; a `*` matches any run of characters.
//! * `check` is the failing check (`parse`, `keep`, `roundtrip`, `reparse`,
//!   `output:<rule>`, …); `*` matches any check.
//! * `current` is the failure class on the public pipeline (`FATAL`, `PANIC`,
//!   `LOST`, `CARD`, `MOVED`, `CHANGED`, `DIFF`, `NONCONFORMANT`, …), or `-`
//!   when the check passes there.
//! * `next` is the same for the pipeline under construction (`-` while there
//!   is none); `RATCHET_TIER=next` selects that column.
//!
//! `RATCHET_PRINT=1` prints a ready-to-paste row for every unlisted failure.

use std::collections::BTreeSet;
use std::sync::OnceLock;

const GAPS: &str = include_str!("../../fixtures/conformance/known_gaps.tsv");

/// One failing check of one case.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Failure {
    pub id: String,
    pub check: String,
    pub class: String,
    pub detail: String,
}

impl Failure {
    pub fn new(
        id: impl Into<String>,
        check: impl Into<String>,
        class: &str,
        detail: impl Into<String>,
    ) -> Self {
        Failure {
            id: id.into(),
            check: check.into(),
            class: class.to_string(),
            detail: detail.into(),
        }
    }
}

#[derive(Debug)]
pub struct Gap {
    pub id: String,
    pub check: String,
    pub cause: String,
    pub class: String,
    pub line: usize,
}

/// The pipeline under test: `current` (default) or `next`.
pub fn tier() -> &'static str {
    static T: OnceLock<String> = OnceLock::new();
    T.get_or_init(|| std::env::var("RATCHET_TIER").unwrap_or_else(|_| "current".into()))
}

pub fn gaps() -> &'static [Gap] {
    static G: OnceLock<Vec<Gap>> = OnceLock::new();
    G.get_or_init(|| {
        let col = if tier() == "next" { 4 } else { 3 };
        let mut out = Vec::new();
        for (i, line) in GAPS.lines().enumerate() {
            if i == 0 || line.is_empty() || line.starts_with('#') {
                continue;
            }
            let f: Vec<&str> = line.split('\t').collect();
            assert!(
                f.len() >= 5,
                "known_gaps.tsv line {}: expected at least 5 columns",
                i + 1
            );
            assert!(
                f[2].starts_with("TS"),
                "known_gaps.tsv line {}: cause must be a TS id",
                i + 1
            );
            if f[col] == "-" {
                continue;
            }
            out.push(Gap {
                id: f[0].to_string(),
                check: f[1].to_string(),
                cause: f[2].to_string(),
                class: f[col].to_string(),
                line: i + 1,
            });
        }
        out
    })
}

/// `*` matches any run of characters.
pub fn glob(pattern: &str, s: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == s;
    }
    let mut rest = s;
    for (i, p) in parts.iter().enumerate() {
        if i == 0 {
            match rest.strip_prefix(p) {
                Some(r) => rest = r,
                None => return false,
            }
        } else if i == parts.len() - 1 {
            return rest.ends_with(p);
        } else {
            match rest.find(p) {
                Some(at) => rest = &rest[at + p.len()..],
                None => return false,
            }
        }
    }
    true
}

/// Checks a family's failures against the ratchet; panics with a full
/// report. `family` is the id prefix (`case`, `place70`, …) that the family
/// owns: rows under it that matched nothing are reported as fixed.
pub fn verify(family: &str, ran: usize, failures: Vec<Failure>) {
    verify_skipping(family, ran, failures, &[]);
}

/// [`verify`], for a family some of whose checks did not run (a cargo
/// feature is off): rows under the `skipped` id prefixes are left alone.
pub fn verify_skipping(family: &str, ran: usize, failures: Vec<Failure>, skipped: &[&str]) {
    let prefix = format!("{family}/");
    let rows: Vec<&Gap> = gaps()
        .iter()
        .filter(|g| g.id.starts_with(&prefix) && !skipped.iter().any(|s| g.id.starts_with(s)))
        .collect();
    let mut used = vec![false; rows.len()];
    let mut unexpected = Vec::new();
    let mut wrong_class = Vec::new();
    let failures: BTreeSet<Failure> = failures.into_iter().collect();
    // Keep one failure per (id, check): several issues of one rule count once.
    let mut seen = BTreeSet::new();
    for f in &failures {
        if !seen.insert((f.id.clone(), f.check.clone())) {
            continue;
        }
        let hit = rows
            .iter()
            .position(|g| glob(&g.id, &f.id) && (g.check == "*" || glob(&g.check, &f.check)));
        match hit {
            Some(i) => {
                used[i] = true;
                if rows[i].class != f.class && rows[i].class != "*" {
                    wrong_class.push(format!(
                        "  {} {}: now {} (row {} says {}): {}",
                        f.id, f.check, f.class, rows[i].line, rows[i].class, f.detail
                    ));
                }
            }
            None => unexpected.push(f),
        }
    }
    let fixed: Vec<String> = rows
        .iter()
        .zip(&used)
        .filter(|(_, u)| !**u)
        .map(|(g, _)| {
            format!(
                "  line {}: {} {} ({}): delete this row",
                g.line, g.id, g.check, g.cause
            )
        })
        .collect();
    if std::env::var_os("RATCHET_PRINT").is_some() {
        for f in &unexpected {
            println!(
                "RATCHET\t{}\t{}\tTS?\t{}\t-\t{}",
                f.id,
                f.check,
                f.class,
                f.detail.replace(['\t', '\n'], " ")
            );
        }
    }
    let n_expected = used.iter().filter(|u| **u).count();
    if unexpected.is_empty() && wrong_class.is_empty() && fixed.is_empty() {
        eprintln!(
            "{family}: {ran} cases, {} failing checks, all listed ({n_expected} rows)",
            seen.len()
        );
        return;
    }
    let mut report = format!(
        "{family}: {ran} cases checked against known_gaps.tsv ({} tier)\n",
        tier()
    );
    if !unexpected.is_empty() {
        report += &format!(
            "{} unlisted failures (fix them, or add a row with its root cause):\n",
            unexpected.len()
        );
        for f in &unexpected {
            let mut d = f.detail.replace('\n', " | ");
            if d.len() > 300 {
                let mut cut = 300;
                while !d.is_char_boundary(cut) {
                    cut -= 1;
                }
                d.truncate(cut);
            }
            report += &format!("  {}\t{}\t{}\t{d}\n", f.id, f.check, f.class);
        }
    }
    if !wrong_class.is_empty() {
        report += &format!(
            "{} failures changed class:\n{}\n",
            wrong_class.len(),
            wrong_class.join("\n")
        );
    }
    if !fixed.is_empty() {
        report += &format!(
            "{} listed gaps now pass:\n{}\n",
            fixed.len(),
            fixed.join("\n")
        );
    }
    panic!("{report}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matches() {
        assert!(glob("case/*", "case/G7-EOL-CR"));
        assert!(glob("output:*", "output:escape"));
        assert!(glob("a*b*c", "a-x-b-y-c"));
        assert!(!glob("a*b", "a-x-c"));
        assert!(glob("exact", "exact"));
    }

    #[test]
    fn every_row_is_well_formed() {
        let families = [
            "case",
            "encoding",
            "matrix",
            "ansel",
            "vendored",
            "convert",
            "place551",
            "place70",
            "place71",
            "enum551",
            "enum70",
            "enum71",
            "calendar",
            "payload",
            "selftest",
            "feature",
            "corpus",
            "fixture",
            "references",
        ];
        for g in gaps() {
            let fam = g.id.split('/').next().unwrap_or("");
            assert!(
                families.contains(&fam),
                "line {}: unknown family {fam}",
                g.line
            );
            let n: u32 = g.cause[2..].parse().unwrap_or(0);
            assert!(
                (1..=35).contains(&n),
                "line {}: cause {} is not TS1–TS35",
                g.line,
                g.cause
            );
        }
    }
}
