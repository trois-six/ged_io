//! A dataset: the header and the records of a file, typed, and the store
//! their texts point into.

use crate::tree::{Flat, Structure};
use crate::version::GedcomVersion;

use super::driver::{FromNode, NodeRef, ReadCx, StdTag, ToNodes, WriteCx};
use super::header::Header;
use super::node::{Extra, Node, Value};
use super::records::{
    Family, Individual, Multimedia, Repository, SharedNote, Source, Submission, Submitter,
};
use super::text::{Store, XrefId};

/// A GEDCOM dataset: its header and records, typed, and the store their
/// texts and identifiers resolve against.
///
/// Records are kept by type, each type in file order; records of no type
/// the model has (extension and unknown records, a second header, a
/// trailer with content) are kept in `extra`, in order. The trailer itself
/// carries nothing and is written by the writer.
///
/// ```rust
/// use ged_io::next::{read_str, RecordRef};
///
/// let data = read_str("0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n0 @F1@ FAM\n1 CHIL @I1@\n0 _LOC Place\n0 TRLR\n");
/// assert_eq!(data.individuals.len(), 1);
/// assert_eq!(data.families[0].children.len(), 1);
/// assert_eq!(data.extra.len(), 1); // the extension record
/// assert!(matches!(data.find("@F1@"), Some(RecordRef::Family(_))));
/// assert_eq!(data.records().count(), 4); // the header, two records, the extension
/// ```
#[derive(Debug, Default)]
pub struct Dataset {
    /// The text the dataset was read from, and its tables.
    pub store: Store,
    /// The header (`HEAD`), when the file has one.
    pub header: Option<Header>,
    /// The individuals (`INDI`).
    pub individuals: Vec<Individual>,
    /// The families (`FAM`).
    pub families: Vec<Family>,
    /// The sources (`SOUR`).
    pub sources: Vec<Source>,
    /// The repositories (`REPO`).
    pub repositories: Vec<Repository>,
    /// The multimedia objects (`OBJE`).
    pub multimedia: Vec<Multimedia>,
    /// The submitters (`SUBM`).
    pub submitters: Vec<Submitter>,
    /// The submissions (5.5.1 `SUBN`).
    pub submissions: Vec<Submission>,
    /// The shared notes (5.5.1 `NOTE`, 7.x `SNOTE`).
    pub notes: Vec<SharedNote>,
    /// The records of no type the model has, in order.
    pub extra: Extra,
    pub(crate) version: GedcomVersion,
    pub(crate) declared_version: Option<Box<str>>,
}

impl AsRef<Store> for Dataset {
    fn as_ref(&self) -> &Store {
        &self.store
    }
}

/// A record of a [`Dataset`], whatever its type.
#[non_exhaustive]
#[derive(Clone, Copy, Debug)]
pub enum RecordRef<'a> {
    /// The header.
    Header(&'a Header),
    /// An individual.
    Individual(&'a Individual),
    /// A family.
    Family(&'a Family),
    /// A source.
    Source(&'a Source),
    /// A repository.
    Repository(&'a Repository),
    /// A multimedia object.
    Multimedia(&'a Multimedia),
    /// A submitter.
    Submitter(&'a Submitter),
    /// A submission.
    Submission(&'a Submission),
    /// A shared note.
    Note(&'a SharedNote),
    /// A record of no type the model has.
    Other(&'a Node),
}

impl<'a> RecordRef<'a> {
    /// The record's identifier.
    #[must_use]
    pub fn xref(&self) -> Option<XrefId> {
        match self {
            RecordRef::Header(_) => None,
            RecordRef::Individual(r) => r.xref,
            RecordRef::Family(r) => r.xref,
            RecordRef::Source(r) => r.xref,
            RecordRef::Repository(r) => r.xref,
            RecordRef::Multimedia(r) => r.xref,
            RecordRef::Submitter(r) => r.xref,
            RecordRef::Submission(r) => r.xref,
            RecordRef::Note(r) => r.xref,
            RecordRef::Other(n) => n.xref,
        }
    }

    /// The tag the record is written with in `cx`'s version.
    pub(crate) fn tag(&self, cx: &WriteCx<'a>) -> &'a str {
        match self {
            RecordRef::Header(_) => "HEAD",
            RecordRef::Individual(_) => "INDI",
            RecordRef::Family(_) => "FAM",
            RecordRef::Source(_) => "SOUR",
            RecordRef::Repository(_) => "REPO",
            RecordRef::Multimedia(_) => "OBJE",
            RecordRef::Submitter(_) => "SUBM",
            RecordRef::Submission(_) => "SUBN",
            RecordRef::Note(_) if cx.version.is_v7() => "SNOTE",
            RecordRef::Note(_) => "NOTE",
            RecordRef::Other(n) => cx.store.tag(n.tag),
        }
    }

    /// Whether the record has neither payload nor substructure. Asked of
    /// trailers only, which are never typed: a typed record answers no.
    pub(crate) fn is_empty(&self) -> bool {
        match self {
            RecordRef::Other(n) => {
                let payload = match &n.payload {
                    Value::None => true,
                    Value::Pointer(_) => false,
                    Value::Text(t) => t.is_empty(),
                };
                payload && n.children.is_empty()
            }
            _ => false,
        }
    }

    /// The record, into a flat arena, its texts borrowed.
    pub(crate) fn to_flat(self, cx: &WriteCx<'a>, out: &mut Flat<'a>) {
        match self {
            RecordRef::Header(r) => r.to_flat(const { StdTag::new("HEAD") }, cx, out),
            RecordRef::Individual(r) => r.to_flat(const { StdTag::new("INDI") }, cx, out),
            RecordRef::Family(r) => r.to_flat(const { StdTag::new("FAM") }, cx, out),
            RecordRef::Source(r) => r.to_flat(const { StdTag::new("SOUR") }, cx, out),
            RecordRef::Repository(r) => r.to_flat(const { StdTag::new("REPO") }, cx, out),
            RecordRef::Multimedia(r) => r.to_flat(const { StdTag::new("OBJE") }, cx, out),
            RecordRef::Submitter(r) => r.to_flat(const { StdTag::new("SUBM") }, cx, out),
            RecordRef::Submission(r) => r.to_flat(const { StdTag::new("SUBN") }, cx, out),
            RecordRef::Note(r) => r.to_flat(const { StdTag::new("SNOTE") }, cx, out),
            RecordRef::Other(n) => n.to_flat(cx.store, out),
        }
    }

    /// The record as an owned structure.
    pub(crate) fn to_structure(self, cx: &WriteCx<'_>) -> Structure {
        match self {
            RecordRef::Header(r) => r.to_node("HEAD", cx),
            RecordRef::Individual(r) => r.to_node("INDI", cx),
            RecordRef::Family(r) => r.to_node("FAM", cx),
            RecordRef::Source(r) => r.to_node("SOUR", cx),
            RecordRef::Repository(r) => r.to_node("REPO", cx),
            RecordRef::Multimedia(r) => r.to_node("OBJE", cx),
            RecordRef::Submitter(r) => r.to_node("SUBM", cx),
            RecordRef::Submission(r) => r.to_node("SUBN", cx),
            RecordRef::Note(r) => r.to_node("SNOTE", cx),
            RecordRef::Other(n) => n.to_structure(cx.store),
        }
    }
}

/// Reads a record of type `T` into `list`; `false` when it does not fit.
fn push<T: FromNode>(list: &mut Vec<T>, node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> bool {
    match T::from_node(node, cx) {
        Some(record) => {
            list.push(record);
            true
        }
        None => false,
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

    /// Reads a record into its type, or keeps it in `extra`.
    pub(crate) fn add(&mut self, node: NodeRef<'_>, cx: &mut ReadCx<'_>) {
        let kept = match node.standard_tag() {
            Some("HEAD") if self.header.is_none() => {
                self.header = Header::from_node(node, cx);
                self.header.is_some()
            }
            Some("INDI") => push(&mut self.individuals, node, cx),
            Some("FAM") => push(&mut self.families, node, cx),
            Some("SOUR") => push(&mut self.sources, node, cx),
            Some("REPO") => push(&mut self.repositories, node, cx),
            Some("OBJE") => push(&mut self.multimedia, node, cx),
            Some("SUBM") => push(&mut self.submitters, node, cx),
            Some("SUBN") => push(&mut self.submissions, node, cx),
            Some("NOTE" | "SNOTE") => push(&mut self.notes, node, cx),
            // The trailer carries nothing: the writer writes its own.
            Some("TRLR") => !node.has_children() && node.has_no_payload() && !node.has_xref(),
            _ => false,
        };
        if !kept {
            let n = cx.node(node);
            self.extra.push(n);
        }
    }

    /// Releases the spare capacity of the lists of records.
    pub(crate) fn shrink(&mut self) {
        self.individuals.shrink_to_fit();
        self.families.shrink_to_fit();
        self.sources.shrink_to_fit();
        self.repositories.shrink_to_fit();
        self.multimedia.shrink_to_fit();
        self.submitters.shrink_to_fit();
        self.submissions.shrink_to_fit();
        self.notes.shrink_to_fit();
        self.extra.shrink();
    }

    /// Every record, the header first, then by type (submitters,
    /// submissions, individuals, families, notes, sources, repositories,
    /// multimedia objects), then the others, in order.
    pub fn records(&self) -> impl Iterator<Item = RecordRef<'_>> {
        self.header
            .iter()
            .map(RecordRef::Header)
            .chain(self.submitters.iter().map(RecordRef::Submitter))
            .chain(self.submissions.iter().map(RecordRef::Submission))
            .chain(self.individuals.iter().map(RecordRef::Individual))
            .chain(self.families.iter().map(RecordRef::Family))
            .chain(self.notes.iter().map(RecordRef::Note))
            .chain(self.sources.iter().map(RecordRef::Source))
            .chain(self.repositories.iter().map(RecordRef::Repository))
            .chain(self.multimedia.iter().map(RecordRef::Multimedia))
            .chain(self.extra.iter().map(RecordRef::Other))
    }

    /// The record with this identifier (`@I1@`): the first one, when
    /// several have it.
    #[must_use]
    pub fn find(&self, xref: &str) -> Option<RecordRef<'_>> {
        let id = self.store.find_xref(xref)?;
        self.records().find(|r| r.xref() == Some(id))
    }

    /// The individual with this identifier.
    #[must_use]
    pub fn individual(&self, xref: &str) -> Option<&Individual> {
        let id = self.store.find_xref(xref)?;
        self.individuals.iter().find(|r| r.xref == Some(id))
    }

    /// The family with this identifier.
    #[must_use]
    pub fn family(&self, xref: &str) -> Option<&Family> {
        let id = self.store.find_xref(xref)?;
        self.families.iter().find(|r| r.xref == Some(id))
    }

    /// The records as owned structures, then the trailer, as the model
    /// holds them (known enumeration values in the dataset's version's
    /// spelling).
    #[must_use]
    pub fn to_structures(&self) -> Vec<Structure> {
        self.structures(self.version, false)
    }

    /// The records as owned structures, then the trailer, for writing
    /// `version`: dates, ages and times in its grammars, enumeration values
    /// in its spelling.
    #[must_use]
    pub fn to_structures_for(&self, version: GedcomVersion) -> Vec<Structure> {
        self.structures(version, true)
    }

    fn structures(&self, version: GedcomVersion, convert: bool) -> Vec<Structure> {
        let cx = WriteCx {
            store: &self.store,
            version,
            convert,
        };
        // The trailer, which the dataset does not keep, ends them.
        let mut out = Vec::with_capacity(self.record_count() + 1);
        out.extend(self.records().map(|r| r.to_structure(&cx)));
        out.push(Structure::new("TRLR"));
        out
    }

    /// The number of records, the header included.
    fn record_count(&self) -> usize {
        usize::from(self.header.is_some())
            + self.individuals.len()
            + self.families.len()
            + self.sources.len()
            + self.repositories.len()
            + self.multimedia.len()
            + self.submitters.len()
            + self.submissions.len()
            + self.notes.len()
            + self.extra.len()
    }
}
