//! The typed model: a [`Dataset`] of records, every structure of GEDCOM
//! 5.5.1, 7.0 and 7.1 a Rust type.
//!
//! # Records and structures
//!
//! A [`Dataset`] holds the [`Header`] and the records, each type in file
//! order: [`Individual`], [`Family`], [`Source`], [`Repository`],
//! [`Multimedia`], [`Submitter`], [`Submission`] and [`SharedNote`].
//! Records of no type the model has (extension records, unknown records)
//! stay untyped [`Node`]s in [`Dataset::extra`].
//!
//! Each structure is a type ([`Event`], [`Name`], [`ChildLink`],
//! [`Ordinance`], [`Citation`], [`Place`], …) with one field per
//! substructure the specifications permit — an `Option` when every version
//! permits one, a list when one permits several — and `extra`, the
//! substructures it has no field for (extensions, unknown tags, a second
//! occurrence of a singleton, a structure whose shape does not fit its
//! type), in order, where they were. Enumeration values are enums
//! ([`Sex`], [`Pedigree`], [`Role`], …) whose `Unknown` variant keeps any
//! other value as written; the 7.x `PHRASE` that words a value is kept
//! with it ([`Phrased`]).
//!
//! # Reading
//!
//! Reading never fails and loses nothing: a file is decoded whatever its
//! encoding ([`crate::encoding`]), its lines are read by the lenient rules
//! of [`crate::tree`], and each record is typed as far as it fits. Values
//! are kept as written: dates and ages are read by the grammars of
//! [`crate::value`] on demand. [`Dataset::parse`] and
//! [`Dataset::from_bytes`] read text and bytes;
//! [`GedcomBuilder`](crate::GedcomBuilder) adds limits, strict mode and
//! GEDZIP archives; [`GedcomStreamParser`](crate::GedcomStreamParser) reads
//! one record at a time.
//!
//! # Texts and identifiers
//!
//! A dataset keeps the decoded text it was read from in its [`Store`], and
//! its model points into it:
//!
//! - every text is a [`Text`]: a span of the input, or, for a payload
//!   reading rewrote (continuation lines joined, `@@` unescaped), a run of
//!   the store's pieces of it — 16 bytes and no copy of the characters;
//!   text a program sets owns its characters;
//! - identifiers and pointers are interned [`XrefId`]s (4 bytes), which the
//!   store maps to their text (`@I1@`) and back;
//! - the tags of untyped [`Node`]s are [`TagId`]s.
//!
//! [`Text::to_str`] and [`Store::xref`] read them back; both take the
//! dataset (or anything that holds a store). Fields that few occurrences of
//! a structure use are in its boxed *detail* ([`Event::detail`],
//! [`Individual::detail`], …), allocated with the first of them, and
//! lists that seldom repeat are [`ThinVec`]s: a dataset takes about 2.5
//! times the size of its input.
//!
//! # Writing
//!
//! [`GedcomWriter`](crate::GedcomWriter) writes a dataset in its version or
//! another: values are converted to the target's grammars, the structures
//! are repaired to conform to its specification ([`crate::spec::conform`])
//! and every line follows its line rules.
//!
//! ```rust
//! use ged_io::model::{Dataset, NoteContent, Pedigree};
//! use ged_io::GedcomWriter;
//!
//! let data = Dataset::parse(
//!     "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 NOTE Hello\n2 LANG en\n\
//!      1 FAMC @F1@\n2 PEDI OTHER\n3 PHRASE Guardianship\n0 @F1@ FAM\n0 TRLR\n",
//! );
//! let indi = &data.individuals[0];
//! let note = &indi.notes[0];
//! assert!(matches!(&note.content, NoteContent::Text(t) if t.to_str(&data) == "Hello"));
//! assert_eq!(note.detail().language.as_ref().unwrap().to_str(&data), "en");
//!
//! let pedigree = indi.child_of[0].detail().pedigree.as_ref().unwrap();
//! assert_eq!(pedigree.value, Pedigree::Other);
//!
//! let out = GedcomWriter::new().write_to_string(&data).unwrap();
//! assert!(out.contains("2 PEDI OTHER\n3 PHRASE Guardianship\n"));
//! ```

pub(crate) mod driver;

mod address;
mod citation;
mod dataset;
mod date;
mod enums;
mod event;
mod header;
mod identifier;
mod lds;
mod ledger_impl;
mod link;
mod list;
mod multimedia;
mod name;
mod node;
mod note;
mod place;
pub(crate) mod read;
mod record;
pub(crate) mod relocate;
#[cfg(feature = "serde")]
pub(crate) mod serde;
mod text;
pub(crate) mod write;

pub use address::Address;
pub use citation::{
    CallNumber, Citation, CitationData, CitationDetail, CitationSource, CitedEvent,
    RepositoryCitation,
};
pub use dataset::{DanglingReference, Dataset, Record, RecordRef, XrefKey};
pub use date::{Age, ChangeDate, CreationDate, Date, DateDetail, ExactDate, Period};
pub use enums::{
    Adoption, BirthKind, Certainty, CharacterSet, ChildStatus, EnumList, EventKind, GedcomForm,
    LdsStatus, Medium, MultimediaFormat, NameType, NoteKind, OrdinanceFlag, OrdinanceStatus,
    Pedigree, PhoneticType, Phrased, Restriction, Role, RomanizedType, Sex,
};
pub use event::{Event, EventDetail, EventFamily, EventSpouse, NonEvent};
pub use header::{
    Charset, Corporation, GedcomInfo, Header, HeaderPlace, HeaderSource, HeaderSourceData,
    HeaderText, Schema,
};
pub use identifier::{Exid, Refn};
pub use lds::{Ordinance, OrdinanceKind};
pub use link::{
    Association, ChildLink, ChildLinkDetail, IndividualRef, IndividualRefDetail, SpouseLink,
};
pub use list::ThinVec;
pub use multimedia::{Crop, File, FileForm, FileTranslation, MultimediaLink};
pub use name::{
    Name, NameDetail, NamePiece, NamePieceKind, NameTranslation, PhoneticName, RomanizedName,
};
pub use node::{Extra, Node, Value};
pub use note::{Note, NoteContent, NoteDetail, NoteTranslation, SourceText, TextTranslation};
pub use place::{Map, PhoneticVariation, Place, PlaceDetail, PlaceTranslation, RomanizedVariation};
pub use record::{
    Family, FamilyDetail, Individual, IndividualDetail, Multimedia, RecordedEvents, Repository,
    SharedNote, Source, SourceData, Submission, Submitter,
};
pub use text::{Store, TagId, Text, XrefId};

/// The declarations of the typed model, for the coverage ledger
/// (`tests/spec_coverage.rs`), which checks them against the specification
/// tables.
#[doc(hidden)]
pub mod ledger {
    pub use super::driver::{FieldDesc, SpecNames};
    pub use super::enums::{EnumDesc, EnumValue};

    /// Every typed structure: its name, the structure types it stands for
    /// and its fields.
    #[must_use]
    pub fn structures() -> &'static [(&'static str, SpecNames, &'static [FieldDesc])] {
        super::ledger_impl::STRUCTURES
    }

    /// Every enumeration type.
    #[must_use]
    pub fn enumerations() -> &'static [EnumDesc] {
        super::enums::ENUMS
    }

    /// The paths of the structures of `data` kept untyped although a field
    /// takes their tag: they did not fit their type.
    #[must_use]
    pub fn untyped(data: &super::Dataset) -> Vec<String> {
        super::ledger_impl::untyped(data)
    }
}
