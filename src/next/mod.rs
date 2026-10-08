//! The typed model under construction, declared once per structure and
//! read and written by one generic driver.
//!
//! **Unstable**: this module is the next model of the crate, built step by
//! step beside the current one, and exercised by the conformance suite in
//! its `next` tier. It becomes the crate's model when it is complete.
//!
//! # Model
//!
//! A [`Dataset`] holds the header and every record typed: [`Individual`],
//! [`Family`], [`Source`], [`Repository`], [`Multimedia`], [`Submitter`],
//! [`Submission`] and [`SharedNote`]; records of no type the model has stay
//! [`Node`]s. Every structure of GEDCOM 5.5.1, 7.0 and 7.1 has a type
//! ([`Event`], [`Name`], [`ChildLink`], [`Ordinance`], [`Citation`], …),
//! with one field per substructure the specifications permit — an `Option`
//! when they permit one, a list when any version permits several — and
//! `extra`, the substructures it has no field for, in order.
//!
//! # Layout
//!
//! The model is laid out for a peak of at most three times the input:
//!
//! - A [`Dataset`] keeps the decoded text it was read from in its
//!   [`Store`]. Every text of the model is a [`Text`]: a span of that
//!   buffer, or, for a payload reading rewrote (continuations joined, `@@`
//!   unescaped), a run of the store's 8-byte pieces of it — 16 bytes and no
//!   allocation either way, and no copy of the characters; text a program
//!   sets owns its characters.
//! - Identifiers and pointers are interned [`XrefId`]s (4 bytes); the
//!   store maps them to their text and back.
//! - Enumeration values are enums ([`Pedigree`], [`Role`], …) whose
//!   unknown values keep their text; tags of untyped [`Node`]s are
//!   [`TagId`]s.
//! - A structure's fields that few occurrences use are in its boxed
//!   *detail* ([`Event::detail`], [`Individual::detail`], …): one word when
//!   none is used. Lists of substructures that seldom repeat are
//!   [`ThinVec`]s (one word when empty, one allocation for one item), and
//!   `extra` is one too. `tests/sizes.rs` holds the sizes to their budget.
//! - Records are read one at a time from the lexer's arena, which is reused:
//!   the whole file is never held as a tree.
//!
//! # Reading and writing
//!
//! Reading never fails and keeps everything: what a type has no field for
//! goes to its `extra` where it was; a value an enumeration does not name
//! is kept as written; a node that does not fit its type stays untyped.
//! Writing produces structures that [`GedcomWriter`] repairs
//! ([`crate::spec::conform`]) and emits, so the output is conformant to
//! the target version.
//!
//! ```rust
//! use ged_io::next::{read_str, write_string, NoteContent, Pedigree};
//! use ged_io::GedcomWriter;
//!
//! let text = "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 NOTE Hello\n2 LANG en\n\
//!             1 FAMC @F1@\n2 PEDI OTHER\n3 PHRASE Guardianship\n0 @F1@ FAM\n0 TRLR\n";
//! let data = read_str(text);
//! let indi = &data.individuals[0];
//! let note = &indi.notes[0];
//! assert!(matches!(&note.content, NoteContent::Text(t) if t.to_str(&data) == "Hello"));
//! assert_eq!(note.detail().language.as_ref().unwrap().to_str(&data), "en");
//!
//! let pedigree = indi.child_of[0].detail().pedigree.as_ref().unwrap();
//! assert_eq!(pedigree.value, Pedigree::Other);
//!
//! let out = write_string(&data, &GedcomWriter::new()).unwrap();
//! assert!(out.contains("2 PEDI OTHER\n3 PHRASE Guardianship\n"));
//! ```

pub(crate) mod driver;

mod citation;
mod dataset;
mod dates;
mod enums;
mod event;
mod header;
mod identifiers;
mod lds;
mod ledger_impl;
mod link;
mod list;
mod multimedia;
mod name;
mod node;
mod note;
mod place;
mod records;
mod text;

use std::io;

use crate::spec::conform::Build;
use crate::tree::{head_version, Builder, Escaping, Flat, FlatPayload, TagInterner};
use crate::version::GedcomVersion;
use crate::writer::{GedcomWriter, WriteError, WriteReport};

use driver::WriteCx;

pub use citation::{
    CallNumber, Citation, CitationData, CitationDetail, CitationSource, CitedEvent,
    RepositoryCitation,
};
pub use dataset::{Dataset, RecordRef};
pub use dates::{Age, ChangeDate, CreationDate, Date, DateDetail, ExactDate, Period};
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
pub use identifiers::{Address, Exid, Refn};
pub use lds::{Ordinance, OrdinanceKind};
pub use link::{ChildLink, ChildLinkDetail, IndividualRef, IndividualRefDetail, SpouseLink};
pub use list::ThinVec;
pub use multimedia::{Crop, File, FileForm, FileTranslation, MultimediaLink};
pub use name::{
    Name, NameDetail, NamePiece, NamePieceKind, NameTranslation, PhoneticName, RomanizedName,
};
pub use node::{Extra, Node, Value};
pub use note::{Note, NoteContent, NoteDetail, NoteTranslation, SourceText, TextTranslation};
pub use place::{
    Association, Map, PhoneticVariation, Place, PlaceDetail, PlaceTranslation, RomanizedVariation,
};
pub use records::{
    Family, FamilyDetail, Individual, IndividualDetail, Multimedia, RecordedEvents, Repository,
    SharedNote, Source, SourceData, Submission, Submitter,
};
pub use text::{Store, TagId, Text, XrefId};

use driver::{Arena, NodeRef, ReadCx};

/// The largest text one pass of the lexer indexes: offsets are 32-bit, and
/// a piece's length takes 31 bits.
const SEGMENT_LIMIT: usize = (u32::MAX / 2) as usize;

/// Reads a dataset from decoded text. Never fails.
#[must_use]
pub fn read_str(text: &str) -> Dataset {
    read_string(text.to_owned())
}

/// Reads a dataset from bytes of any encoding (see
/// [`crate::encoding::decode`]). Given a `Vec<u8>` of UTF-8, the dataset
/// keeps that buffer as its text, without a copy.
#[must_use]
pub fn read_bytes(bytes: impl Into<Vec<u8>>) -> Dataset {
    read_string(crate::encoding::decode_owned(bytes.into()).text)
}

/// Reads a dataset from decoded text, keeping the text as its store.
#[must_use]
pub fn read_string(text: String) -> Dataset {
    read_segmented(text, SEGMENT_LIMIT)
}

fn read_segmented(text: String, limit: usize) -> Dataset {
    let declared = head_version(&text);
    let version = declared
        .as_deref()
        .map_or(GedcomVersion::V5_5_1, GedcomVersion::from_version_str);
    let escaping = Escaping::of(declared.as_deref());
    let mut store = Store::new(text);
    let mut data = Dataset {
        version,
        declared_version: declared.map(Into::into),
        ..Dataset::default()
    };
    let mut tags = TagInterner::default();
    {
        let (input, pieces, xrefs) = store.parts_mut();
        let mut cx = ReadCx::new(input, pieces, xrefs, version);
        let mut builder = Builder::new(escaping, 1).with_spans();
        for (start, end, first_line) in crate::tree::segment_bounds(input, limit) {
            let part = input.get(start..end).unwrap_or_default();
            builder.reset(first_line);
            builder.read_records(part, &mut tags, |b, _| {
                let arena = Arena {
                    text: part,
                    base: start,
                    joined: &b.joined,
                    nodes: &b.nodes,
                    xrefs: &b.xrefs,
                };
                if let Some(root) = NodeRef::root(&arena) {
                    data.add(root, &mut cx);
                }
            });
        }
    }
    store.set_tags(tags.others);
    store.shrink();
    data.store = store;
    data.shrink();
    data
}

/// Writes a dataset with `writer`'s configuration: in its version, unless
/// one is configured; repaired to be conformant
/// ([`crate::spec::conform`]); then emitted line by line.
///
/// # Errors
///
/// As [`GedcomWriter::write`].
pub fn write<W: io::Write>(
    data: &Dataset,
    writer: &GedcomWriter,
    out: W,
) -> Result<WriteReport, WriteError> {
    writer.write_built(out, &data.built(writer))
}

/// Writes a dataset as text, as [`GedcomWriter::write_to_string`] does.
///
/// # Errors
///
/// [`WriteError::NonConformant`] on the first repair under
/// [`RepairPolicy::Error`](crate::writer::RepairPolicy::Error).
pub fn write_string(data: &Dataset, writer: &GedcomWriter) -> Result<String, WriteError> {
    writer
        .write_built_to_string(&data.built(writer))
        .map(|(text, _)| text)
}

/// A record of a dataset that writes its structures for the target version
/// when the writer needs them, into the writer's flat arena: the dataset is
/// never copied as owned structures.
struct Built<'d> {
    /// The record; `None` for the trailer, which the dataset does not keep.
    record: Option<RecordRef<'d>>,
    cx: WriteCx<'d>,
}

impl<'d> Build<'d> for Built<'d> {
    fn tag(&self) -> &'d str {
        self.record.map_or("TRLR", |r| r.tag(&self.cx))
    }

    fn xref(&self) -> Option<&'d str> {
        self.record?.xref().map(|x| self.cx.store.xref(x))
    }

    fn is_empty(&self) -> bool {
        self.record.is_none_or(|r| r.is_empty())
    }

    fn build(&self, out: &mut Flat<'d>) {
        if let Some(r) = self.record {
            r.to_flat(&self.cx, out);
        } else {
            let at = out.open(std::borrow::Cow::Borrowed("TRLR"), None, FlatPayload::None);
            out.close(at);
        }
    }
}

impl Dataset {
    /// The records, to write with `writer` (in its version, unless one is
    /// configured), then the trailer.
    fn built(&self, writer: &GedcomWriter) -> Vec<Built<'_>> {
        let cx = WriteCx {
            store: &self.store,
            version: writer.config().version.unwrap_or(self.version),
            convert: true,
        };
        self.records()
            .map(Some)
            .chain([None])
            .map(|record| Built { record, cx })
            .collect()
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_read_as_one_text() {
        let text = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NOTE a@@b\n2 CONT c\n1 BIRT\n2 DATE 1 JAN 1900\n2 PLAC Sampleton\n0 @I2@ INDI\n1 ASSO @I1@\n2 RELA Friend\n0 TRLR\n";
        let whole = read_str(text);
        // Segments of whole records (the largest is 75 bytes).
        for limit in [80, 120, 200] {
            let parts = read_segmented(text.to_owned(), limit);
            assert_eq!(parts.to_structures(), whole.to_structures(), "{limit}");
        }
        assert_eq!(
            whole
                .individual("@I2@")
                .map(|r| r.detail().associations.len()),
            Some(1)
        );
        let note = &whole.individuals[0].notes[0];
        assert!(matches!(&note.content, NoteContent::Text(t) if t.to_str(&whole) == "a@b\nc"));
    }
}
