//! The typed model under construction, declared once per structure and
//! read and written by one generic driver.
//!
//! **Unstable**: this module is the next model of the crate, built step by
//! step beside the current one, and exercised by the conformance suite in
//! its `next` tier. It becomes the crate's model when it is complete.
//!
//! # Layout
//!
//! - A [`Dataset`] keeps the decoded text it was read from in its
//!   [`Source`]. Every text of the model is a [`Text`]: a span of that
//!   buffer (or of a side buffer for payloads reading rewrote), 16 bytes
//!   and no allocation; text a program sets owns its characters.
//! - Identifiers and pointers are interned [`XrefId`]s (4 bytes); the
//!   source maps them to their text and back.
//! - Enumeration values are enums ([`Pedigree`], [`Role`], …) whose
//!   unknown values keep their text; tags of untyped [`Node`]s are
//!   [`TagId`]s.
//! - Every typed structure has `extra`, the substructures it has no field
//!   for, in order: one word when empty. Lists of substructures that seldom
//!   appear are [`ThinVec`]s (one word when empty), rare large parts are
//!   boxed; `tests/sizes.rs` holds the sizes to their budget.
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
//! Records are [`Generic`] until each has its type: their substructures
//! are typed wherever the specification tables say a typed structure
//! stands — a note, a citation, a place, an enumeration value — at any
//! depth.
//!
//! ```rust
//! use ged_io::next::{read_str, write_string, Child, Note, NoteContent, Pedigree, Phrased};
//! use ged_io::GedcomWriter;
//!
//! let text = "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 NOTE Hello\n2 LANG en\n\
//!             1 FAMC @F1@\n2 PEDI OTHER\n3 PHRASE Guardianship\n0 @F1@ FAM\n0 TRLR\n";
//! let data = read_str(text);
//! let indi = &data.records[1];
//! let Child::Typed(note) = &indi.children[0] else { panic!() };
//! let note: &Note = note.get().unwrap();
//! assert!(matches!(&note.content, NoteContent::Text(t) if t.as_str(&data) == "Hello"));
//! assert_eq!(note.language.as_ref().unwrap().as_str(&data), "en");
//!
//! let Child::Generic(famc) = &indi.children[1] else { panic!() };
//! let Child::Typed(pedi) = &famc.children[0] else { panic!() };
//! let pedi: &Phrased<Pedigree> = pedi.get().unwrap();
//! assert_eq!(pedi.value, Pedigree::Other);
//!
//! let out = write_string(&data, &GedcomWriter::new()).unwrap();
//! assert!(out.contains("2 PEDI OTHER\n3 PHRASE Guardianship\n"));
//! ```

pub(crate) mod driver;

mod citation;
mod dates;
mod enums;
mod generic;
mod identifiers;
mod list;
mod multimedia;
mod node;
mod note;
mod place;
mod text;

use std::io;

use crate::spec::conform::Build;
use crate::tree::{head_version, Builder, Escaping, Flat, Structure, TagInterner};
use crate::version::GedcomVersion;
use crate::writer::{GedcomWriter, WriteError, WriteReport};

pub use citation::{
    CallNumber, Citation, CitationData, CitationSource, CitedEvent, RepositoryCitation,
};
pub use dates::{Age, ChangeDate, CreationDate, Date, ExactDate, Period};
pub use enums::{
    Adoption, BirthKind, Certainty, CharacterSet, ChildStatus, EnumList, EventKind, GedcomForm,
    LdsStatus, Medium, MultimediaFormat, NameType, NoteKind, OrdinanceFlag, OrdinanceStatus,
    Pedigree, PhoneticType, Phrased, Restriction, Role, RomanizedType, Sex,
};
pub use generic::{Child, Generic, Typed};
pub use identifiers::{Address, Exid, Refn};
pub use list::ThinVec;
pub use multimedia::{Crop, File, FileForm, FileTranslation, MultimediaLink};
pub use node::{Extra, Node, Value};
pub use note::{Note, NoteContent, NoteTranslation, SourceText, TextTranslation};
pub use place::{Association, Map, PhoneticVariation, Place, PlaceTranslation, RomanizedVariation};
pub use text::{Source, TagId, Text, XrefId};

use driver::{Arena, NodeRef, ReadCx, WriteCx};

/// A GEDCOM dataset: its records, and the source their texts and
/// identifiers resolve against.
#[derive(Debug, Default)]
pub struct Dataset {
    /// The text the dataset was read from, and its tables.
    pub source: Source,
    /// The records, the header first when the file has one, in file
    /// order.
    pub records: Vec<Generic>,
    version: GedcomVersion,
    declared_version: Option<Box<str>>,
}

impl AsRef<Source> for Dataset {
    fn as_ref(&self) -> &Source {
        &self.source
    }
}

impl Dataset {
    /// The version the file declares (`HEAD.GEDC.VERS`): 7.0 or 7.1 for
    /// 7.x, 5.5.1 otherwise.
    #[must_use]
    pub fn version(&self) -> GedcomVersion {
        self.version
    }

    /// The `HEAD.GEDC.VERS` payload as written.
    #[must_use]
    pub fn declared_version(&self) -> Option<&str> {
        self.declared_version.as_deref()
    }

    /// The record with this identifier (`@I1@`): the first one, when
    /// several have it.
    #[must_use]
    pub fn find(&self, xref: &str) -> Option<&Generic> {
        let id = self.source.find_xref(xref)?;
        self.records.iter().find(|r| r.xref == Some(id))
    }

    /// The records as owned structures, as the model holds them (known
    /// enumeration values in the dataset's version's spelling).
    #[must_use]
    pub fn to_structures(&self) -> Vec<Structure> {
        self.structures(self.version, false)
    }

    /// The records as owned structures for writing `version`: dates, ages
    /// and times in its grammars, enumeration values in its spelling.
    #[must_use]
    pub fn to_structures_for(&self, version: GedcomVersion) -> Vec<Structure> {
        self.structures(version, true)
    }

    fn structures(&self, version: GedcomVersion, convert: bool) -> Vec<Structure> {
        let cx = WriteCx {
            source: &self.source,
            version,
            convert,
        };
        self.records.iter().map(|r| r.to_structure(&cx)).collect()
    }
}

/// The largest text one pass of the lexer indexes: offsets are 32-bit, and
/// the side buffer of a record can grow to twice its text.
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

/// Reads a dataset from decoded text, keeping the text as its source.
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
    let mut source = Source::new(text);
    let mut records = Vec::new();
    let mut tags = TagInterner::default();
    {
        let (input, side, xrefs) = source.parts_mut();
        let mut cx = ReadCx::new(input, side, xrefs, version);
        let mut builder = Builder::new(escaping, 1);
        for (start, end, first_line) in crate::tree::segment_bounds(input, limit) {
            let part = input.get(start..end).unwrap_or_default();
            builder.reset(first_line);
            builder.read_records(part, &mut tags, |b, _| {
                let arena = Arena {
                    text: part,
                    base: start,
                    side: &b.side,
                    nodes: &b.nodes,
                    xrefs: &b.xrefs,
                };
                if let Some(root) = NodeRef::root(arena) {
                    records.push(Generic::read(root, &mut cx));
                }
            });
        }
    }
    source.set_tags(tags.others);
    source.shrink();
    records.shrink_to_fit();
    Dataset {
        source,
        records,
        version,
        declared_version: declared.map(Into::into),
    }
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
    record: &'d Generic,
    cx: WriteCx<'d>,
}

impl<'d> Build<'d> for Built<'d> {
    fn tag(&self) -> &'d str {
        self.cx.source.tag(self.record.tag)
    }

    fn xref(&self) -> Option<&'d str> {
        self.record.xref.map(|x| self.cx.source.xref(x))
    }

    fn is_empty(&self) -> bool {
        let payload = match &self.record.payload {
            node::Value::None => true,
            node::Value::Pointer(_) => false,
            node::Value::Text(t) => t.as_str(self.cx.source).is_empty(),
        };
        payload && self.record.children.is_empty()
    }

    fn build(&self, out: &mut Flat<'d>) {
        self.record.to_flat(&self.cx, out);
    }
}

impl Dataset {
    /// The records, to write with `writer`: in its version, unless one is
    /// configured.
    fn built(&self, writer: &GedcomWriter) -> Vec<Built<'_>> {
        let cx = WriteCx {
            source: &self.source,
            version: writer.config().version.unwrap_or(self.version),
            convert: true,
        };
        self.records
            .iter()
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
        super::generic::STRUCTURES
    }

    /// Every enumeration type.
    #[must_use]
    pub fn enumerations() -> &'static [EnumDesc] {
        super::enums::ENUMS
    }

    /// The paths of the structures of `data` that the model types but read
    /// as generic structures, because they did not fit their type.
    #[must_use]
    pub fn untyped(data: &super::Dataset) -> Vec<String> {
        super::generic::untyped(&data.records, &data.source, data.version)
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
        assert_eq!(whole.find("@I2@").map(|r| r.children.len()), Some(1));
    }
}
