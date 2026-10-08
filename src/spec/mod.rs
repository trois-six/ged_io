//! The GEDCOM specifications as data, and a validator over
//! [`tree`](crate::tree) structures, for GEDCOM 5.5.1, 7.0 and 7.1.
//!
//! # Tables
//!
//! Every structure type of each version, its tag, its payload type, its
//! permitted substructures with their cardinalities and every enumeration
//! set are generated at development time by `tools/spec-tables/gen.py`
//! into `src/spec/tables.rs`, which holds facts only:
//!
//! - GEDCOM 7.0 and 7.1 from FamilySearch/GEDCOM `extracted-files/`
//!   (Apache-2.0, attributed in `NOTICE`);
//! - GEDCOM 5.5.1 from the gedcom-go transcription of the specification
//!   (MIT, attributed in `NOTICE`), corrected and completed against the
//!   PDF by `tools/spec-tables/551-errata.tsv`, which cites a page for each
//!   row.
//!
//! The tables take a few kilobytes: types are numbered, tags are indices
//! into one shared list, and no build script or dependency is involved.
//!
//! # Validation
//!
//! [`validate`] checks a tree (parsed, or owned [`Structure`]s) against the
//! tables of a version; [`validate_text`] and [`validate_bytes`] also check
//! what only the text shows. Every deviation is reported, with its line,
//! not just the first:
//!
//! | Rule | [`DeviationKind`] |
//! |---|---|
//! | 7.x is UTF-8; a 5.5.1 file is in the character set `HEAD.CHAR` names | `Encoding` |
//! | Line grammar: a level, single spaces, a tag, no empty payload after a space; 7.x: no blank line, nothing before the level, no LF CR | `LineSyntax` |
//! | Levels start at 0, rise by one at most, have no leading zero (5.5.1: at most two digits) | `Level` |
//! | Identifiers: 7.x `@[A-Z0-9_]+@` but not `@VOID@`; 5.5.1 a letter, digit or `_` first, no control character, at most 22 characters; on records only (not `HEAD` or `TRLR`); unique | `Xref` |
//! | `@` in text: 7.x doubles a leading one, 5.5.1 every one outside an `@#…@` escape | `Escape` |
//! | 5.5.1 lines of at most 255 characters, terminator included | `LineLength` |
//! | `CONT` (5.5.1 also `CONC`) right under the line it continues, before the other substructures, never under a pointer, without substructures; 5.5.1 `CONC` never splits at a space; 7.x has no `CONC` | `Continuation` |
//! | 5.5.1 records under 32K | `RecordSize` |
//! | Tags: 7.x `[A-Z][A-Z0-9_]*` or `_[A-Z0-9_]+`, 5.5.1 alphanumerics, upper case unless the tag starts with `_`; a standard tag is one the version defines | `UnknownTag` |
//! | Each standard tag under a superstructure that permits it | `Misplaced` |
//! | At most the permitted number of each substructure | `Cardinality` |
//! | Every required substructure; 7.x: a note translation has a `MIME` or a `LANG` | `MissingRequired` |
//! | Payloads of their type: none, `Y`, integers, names, dates, ages, times, languages, media types (7.x `MIME` of a text: `text/…`), file paths, URIs, coordinates, tag definitions, pointers where pointers belong and only there; 7.x: a structure has a payload or a substructure (records and pseudo-structures aside) | `Payload` |
//! | Enumeration values: 7.x exact or an extension value, 5.5.1 in any case (p. 21); open sets admit any value | `EnumValue` |
//! | Pointers name a record of the type the structure points to | `PointerTarget` |
//! | Pointers name a record of the file (7.x `@VOID@` and 5.5.1 substructure and network pointers aside) | `DanglingPointer` |
//! | `HEAD` first and once, `TRLR` last, once and empty, `GEDC.VERS` naming the version | `Header` |
//! | No banned character in a payload: 7.x C0 controls but tab and line breaks, DEL, C1 controls, U+FFFE and U+FFFF; 5.5.1 no control character at all | `Character` |
//!
//! Extension structures (`_TAG`) are checked for what holds everywhere —
//! tags, identifiers, pointers, characters — and their substructures mean
//! what their definer says. Payload sizes of 5.5.1 (`{Size=1:90}`) are not
//! checked: they describe storage limits of 1999-era systems, and no reader
//! relies on them.
//!
//! ```rust
//! use ged_io::spec::{validate, validate_text, DeviationKind};
//! use ged_io::tree::parse_tree;
//! use ged_io::GedcomVersion;
//!
//! let text = "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 SEX male\n0 TRLR\n";
//! let issues = validate(&parse_tree(text), GedcomVersion::V7_0);
//! assert_eq!(issues.len(), 1);
//! assert_eq!(issues[0].kind, DeviationKind::EnumValue);
//! assert_eq!(issues[0].to_string(), "line 5: SEX \"male\": not a value of enumset-SEX");
//!
//! // The text also shows the line grammar: two spaces before a tag.
//! let issues = validate_text("0 HEAD\n1 GEDC\n2  VERS 7.0\n0 TRLR\n");
//! assert_eq!(issues[0].kind, DeviationKind::LineSyntax);
//! ```

mod lines;
mod payload;
mod schema;
#[rustfmt::skip]
pub(crate) mod tables;
mod validate;

use std::fmt;

use crate::encoding::decode;
use crate::tree::{Structure, Tree};
use crate::GedcomVersion;

use payload::Family;
pub(crate) use schema::Schema;
use sealed::Sealed;

/// A way in which a dataset does not follow its specification.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Deviation {
    /// The 1-based line where it shows; 0 when no line holds it (a missing
    /// record, the encoding of the whole file).
    pub line: u32,
    /// The rule broken.
    pub kind: DeviationKind,
    /// What is wrong, in words.
    pub detail: Box<str>,
}

impl fmt::Display for Deviation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            f.write_str(&self.detail)
        } else {
            write!(f, "line {}: {}", self.line, self.detail)
        }
    }
}

/// The rule a [`Deviation`] breaks. The [module documentation](self) lists
/// what each covers.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeviationKind {
    /// The bytes are not in the encoding the version or the header says.
    Encoding,
    /// A line does not follow the line grammar.
    LineSyntax,
    /// A level number is malformed or jumps.
    Level,
    /// An identifier is malformed, duplicated or misplaced.
    Xref,
    /// An `@` in text is not escaped as the version wants.
    Escape,
    /// A 5.5.1 line is longer than 255 characters.
    LineLength,
    /// A continuation line is misplaced, or 7.x uses `CONC`.
    Continuation,
    /// A 5.5.1 record is larger than 32K.
    RecordSize,
    /// A tag is malformed, or not one the version defines.
    UnknownTag,
    /// A standard structure where its superstructure does not permit it.
    Misplaced,
    /// More occurrences of a substructure than permitted.
    Cardinality,
    /// A required substructure is missing.
    MissingRequired,
    /// A payload is not of the structure's payload type.
    Payload,
    /// An enumeration value is not in its set.
    EnumValue,
    /// A pointer names a record of the wrong type.
    PointerTarget,
    /// A pointer names no record.
    DanglingPointer,
    /// The header or the trailer is missing, misplaced, repeated or wrong.
    Header,
    /// A character the version bans.
    Character,
}

/// A dataset [`validate`] reads: a parsed [`Tree`] or owned
/// [`Structure`]s (records, header and trailer included).
pub trait Dataset: sealed::Sealed {}

impl Dataset for Tree {}
impl Dataset for [Structure] {}
impl Dataset for Vec<Structure> {}

mod sealed {
    use super::{validate, Deviation, GedcomVersion, Structure, Tree};

    /// Runs the validator.
    pub trait Sealed {
        fn deviations(&self, version: GedcomVersion, record_size: bool) -> Vec<Deviation>;
    }

    impl Sealed for Tree {
        fn deviations(&self, version: GedcomVersion, record_size: bool) -> Vec<Deviation> {
            validate::run(
                self.records(),
                version.rules(),
                validate::Options { record_size },
            )
        }
    }

    impl Sealed for [Structure] {
        fn deviations(&self, version: GedcomVersion, record_size: bool) -> Vec<Deviation> {
            validate::run(
                self.iter(),
                version.rules(),
                validate::Options { record_size },
            )
        }
    }

    impl Sealed for Vec<Structure> {
        fn deviations(&self, version: GedcomVersion, record_size: bool) -> Vec<Deviation> {
            self.as_slice().deviations(version, record_size)
        }
    }
}

/// The grammar family of a version.
fn family(version: GedcomVersion) -> Family {
    Family::of(version.rules())
}

/// Validates a dataset against the specification of `version`: structure,
/// payloads, identifiers, pointers, header and trailer, characters, and the
/// estimated size of 5.5.1 records. See the [module documentation](self) for
/// the rules; the line grammar needs the text ([`validate_text`]).
///
/// The deviations come in line order.
pub fn validate<D: Dataset + ?Sized>(data: &D, version: GedcomVersion) -> Vec<Deviation> {
    data.deviations(version, true)
}

/// Validates GEDCOM text, decoded already, against the specification of the
/// version its header declares (`HEAD.GEDC.VERS`, as
/// [`GedcomVersion::from_version_str`] reads it):
/// the line grammar and everything [`validate`] checks.
///
/// The deviations come in line order.
#[must_use]
pub fn validate_text(text: &str) -> Vec<Deviation> {
    let tree = Tree::parse(text);
    let version = tree.version();
    let mut out = lines::check(text, family(version));
    out.extend(tree.deviations(version, false));
    out.sort_by_key(|d| d.line);
    out
}

/// Validates GEDCOM bytes: their encoding (7.x is UTF-8; 5.5.1 bytes match
/// `HEAD.CHAR`), then the text as [`validate_text`] does.
#[must_use]
pub fn validate_bytes(bytes: &[u8]) -> Vec<Deviation> {
    let decoded = decode(bytes);
    let tree = Tree::parse(decoded.text.as_str());
    let version = tree.version();
    let fam = family(version);
    let mut out: Vec<Deviation> = lines::encoding(&decoded, fam).into_iter().collect();
    out.extend(lines::check(&decoded.text, fam));
    out.extend(tree.deviations(version, false));
    out.sort_by_key(|d| d.line);
    out
}

#[cfg(test)]
mod tests;
