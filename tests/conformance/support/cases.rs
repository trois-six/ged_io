//! Hand-written cases: the file format and the checks every case runs.
//!
//! Each case is labelled:
//! * **C** (conformance): valid input. The output must be conformant, equal
//!   to the input under the semantic rules, and keep the listed values.
//! * **L** (leniency): invalid or exotic input. It must be read silently, its
//!   data kept ("nothing lost": every payload and structure of the input is
//!   somewhere in the output), and the output must be conformant. The writer
//!   repairs what the target version does not permit (the table of
//!   `spec::conform`), so the data of invalid input is kept in the forms
//!   that table gives it, which the keep and round-trip checks accept (see
//!   `keeps` and `semantic::lost`): an extension for a value outside a
//!   closed set or a misplaced structure (`_PEDI stepchild`), the version's
//!   case for an enumeration value (`SEX M` for 5.5.1 `SEX m`), a date
//!   phrase for a date outside the grammar, a pointer to no record kept as
//!   text. A conformance case accepts nothing but its input.

use super::adapter::{self, Target as WriteTarget};
use super::checker::{self, Target};
use super::ratchet::Failure;
use super::semantic;
use super::tree;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    C,
    L,
}

#[derive(Clone, Debug)]
pub struct Case {
    pub id: String,
    pub kind: Kind,
    pub purpose: String,
    pub input: Vec<u8>,
    /// Lines the written output must contain.
    pub wants: Vec<String>,
    /// Texts the model must keep.
    pub model: Vec<String>,
    /// Texts the written output must not contain.
    pub lacks: Vec<String>,
    /// Write target other than the input's own version.
    pub target: Option<Target>,
    /// The input as text, when its bytes are in an encoding the suite's own
    /// reader does not decode (ANSEL, code pages).
    pub text: Option<String>,
}

impl Case {
    pub fn new(id: &str, kind: Kind, input: impl Into<Vec<u8>>) -> Self {
        Case {
            id: id.to_string(),
            kind,
            purpose: String::new(),
            input: input.into(),
            wants: Vec::new(),
            model: Vec::new(),
            lacks: Vec::new(),
            target: None,
            text: None,
        }
    }
}

/// Expands the tokens of the case format into the input text.
pub fn expand(body: &str) -> String {
    let mut out = String::new();
    let lines: Vec<&str> = body.split('\n').collect();
    for (i, line) in lines.iter().enumerate() {
        let mut l = line.to_string();
        let mut eol = if i + 1 < lines.len() { "\n" } else { "" };
        if let Some(s) = l.strip_suffix("<CRLF>") {
            l = s.to_string();
            eol = "\r\n";
        } else if let Some(s) = l.strip_suffix("<CR>") {
            l = s.to_string();
            eol = "\r";
        } else if let Some(s) = l.strip_suffix("<NOEOL>") {
            l = s.to_string();
            eol = "";
        }
        out += &tokens(&l);
        out += eol;
    }
    out
}

/// `<TAB>`, `<BOM>` and `<U+XXXX>`.
pub fn tokens(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(at) = rest.find('<') {
        out += &rest[..at];
        let tail = &rest[at..];
        let end = tail.find('>').map(|e| e + 1);
        let tok = end.map(|e| &tail[..e]);
        let rep = match tok {
            Some("<TAB>") => Some("\t".to_string()),
            Some("<BOM>") => Some("\u{feff}".to_string()),
            Some(t) if t.starts_with("<U+") => u32::from_str_radix(&t[3..t.len() - 1], 16)
                .ok()
                .and_then(char::from_u32)
                .map(String::from),
            _ => None,
        };
        match (rep, end) {
            (Some(r), Some(e)) => {
                out += &r;
                rest = &tail[e..];
            }
            _ => {
                out.push('<');
                rest = &tail[1..];
            }
        }
    }
    out + rest
}

/// Parses a case file (see `tests/fixtures/conformance/cases/*.txt`).
pub fn parse_file(text: &str) -> Vec<Case> {
    let mut out = Vec::new();
    let text = format!("\n{text}");
    for block in text.split("\n### ").skip(1) {
        let (hdr, body) = block.split_once('\n').unwrap_or((block, ""));
        let parts: Vec<&str> = hdr.split('|').map(str::trim).collect();
        let kind = match parts.get(1) {
            Some(&"C") => Kind::C,
            Some(&"L") => Kind::L,
            other => panic!("case {}: kind must be C or L, not {other:?}", parts[0]),
        };
        let mut case = Case::new(parts[0], kind, Vec::new());
        case.purpose = parts.get(2).unwrap_or(&"").to_string();
        let mut ged = Vec::new();
        for l in body.split('\n') {
            if let Some(w) = l.strip_prefix("#want ") {
                case.wants.push(tokens(w));
            } else if let Some(w) = l.strip_prefix("#model ") {
                case.model.push(tokens(w));
            } else if let Some(w) = l.strip_prefix("#lacks ") {
                case.lacks.push(tokens(w));
            } else if let Some(w) = l.strip_prefix("#target ") {
                case.target = Some(Target::from_vers(w.trim()));
            } else if l.starts_with('#') {
                // A comment.
            } else {
                ged.push(l);
            }
        }
        while ged.last() == Some(&"") {
            ged.pop();
        }
        let mut input = expand(&ged.join("\n"));
        if !input.is_empty()
            && !input.ends_with(['\n', '\r'])
            && !ged.last().is_some_and(|l| l.ends_with("<NOEOL>"))
        {
            input.push('\n');
        }
        case.input = input.into_bytes();
        out.push(case);
    }
    out
}

/// What a case's run produced, for the feature and corpus families.
pub struct Run {
    pub output: Option<String>,
    pub failures: Vec<Failure>,
}

/// Runs every check of a case. `family` prefixes the ratchet ids.
pub fn run(family: &str, case: &Case) -> Run {
    let id = format!("{family}/{}", case.id);
    let mut failures = Vec::new();
    let mut fail = |check: &str, class: &str, detail: String| {
        failures.push(Failure::new(&id, check, class, detail));
    };
    let class_of = |e: &str| {
        if e.starts_with("PANIC") {
            "PANIC"
        } else {
            "FATAL"
        }
    };
    let model = match adapter::read(&case.input) {
        Ok(m) => m,
        Err(e) => {
            fail("parse", class_of(&e), e);
            return Run {
                output: None,
                failures,
            };
        }
    };
    let input_text = case
        .text
        .clone()
        .unwrap_or_else(|| tree::decode_bytes(&case.input));
    let declared = Target::from_vers(&tree::declared_vers(&input_text).unwrap_or_default());
    let target = case.target.unwrap_or(declared);
    let wt = match case.target {
        None => WriteTarget::Same,
        Some(Target::V551) => WriteTarget::V551,
        Some(Target::V70) => WriteTarget::V70,
        Some(Target::V71) => WriteTarget::V71,
    };
    let missing: Vec<&String> = case
        .model
        .iter()
        .filter(|m| !adapter::model_contains(&model, m))
        .collect();
    let output = match adapter::write(&model, wt) {
        Ok(o) => o,
        Err(e) => {
            fail("write", class_of(&e), e);
            return Run {
                output: None,
                failures,
            };
        }
    };
    let out_lines: Vec<&str> = tree::split_lines(&output);
    let version = tree::parse(&input_text).0;
    let mut lost: Vec<String> = case
        .wants
        .iter()
        .filter(|w| !keeps(&out_lines, w, case.kind == Kind::L, version))
        .map(|w| format!("missing line `{w}`"))
        .collect();
    lost.extend(missing.iter().map(|m| format!("model lacks `{m}`")));
    if !lost.is_empty() {
        fail("keep", "LOST", lost.join("; "));
    }
    let present: Vec<String> = case
        .lacks
        .iter()
        .filter(|l| output.contains(l.as_str()))
        .map(|l| format!("output has `{}`", l.escape_debug()))
        .collect();
    if !present.is_empty() {
        fail("lacks", "PRESENT", present.join("; "));
    }
    if let Err(e) = adapter::read(output.as_bytes()) {
        fail("reparse", class_of(&e), e);
    }
    for issue in checker::check(&output, target, Some(&input_text)) {
        fail(
            &format!("output:{}", issue.rule),
            "NONCONFORMANT",
            issue.to_string(),
        );
    }
    if case.target.is_none() || case.target == Some(declared) {
        match case.kind {
            Kind::C => {
                let d = semantic::compare(&input_text, &output, &semantic::Options::default());
                if !d.is_empty() {
                    fail("roundtrip", "DIFF", d.join("; "));
                }
            }
            Kind::L => {
                let d = semantic::lost(&input_text, &output);
                if !d.is_empty() {
                    fail("roundtrip", "LOST", d.join("; "));
                }
            }
        }
    }
    Run {
        output: Some(output),
        failures,
    }
}

/// Whether the written lines keep a wanted line: as written, or, for a
/// leniency case, in a form the writer's repair table (`spec::conform`)
/// gives invalid input — the rules of [`semantic::lost`], line by line:
/// the tag relocated as an extension (`_PEDI stepchild`), an enumeration
/// value in another case (`SEX M` for `SEX m`), a date as a 5.5.1 date
/// phrase (`DATE (7/11/1959)`) or as a 7.x `PHRASE` under an empty `DATE`,
/// a pointer kept as text (`_FAMS @@F9@` for a dangling `FAMS @F9@`).
fn keeps(out: &[&str], want: &str, lenient: bool, version: tree::Version) -> bool {
    if out.contains(&want) {
        return true;
    }
    let Some(w) = lenient.then(|| tree::parse_line(want)).flatten() else {
        return false;
    };
    let wanted = w
        .payload
        .map(|p| tree::unescape(p, version))
        .unwrap_or_default();
    out.iter().enumerate().any(|(i, line)| {
        let Some(l) = tree::parse_line(line) else {
            return false;
        };
        if l.level != w.level || l.tag.trim_start_matches('_') != w.tag {
            return false;
        }
        let got = l
            .payload
            .map(|p| tree::unescape(p, version))
            .unwrap_or_default();
        let phrase = || {
            out.get(i + 1)
                .and_then(|n| tree::parse_line(n))
                .filter(|n| n.level == w.level + 1 && n.tag == "PHRASE")
                .and_then(|n| n.payload)
        };
        got == wanted
            || (matches!(
                w.tag,
                "PEDI" | "STAT" | "ADOP" | "MEDI" | "RESN" | "SEX" | "ROLE" | "QUAY"
            ) && got.eq_ignore_ascii_case(&wanted))
            || (w.tag == "DATE"
                && (got == format!("({wanted})") || (got.is_empty() && phrase() == Some(&wanted))))
    })
}
