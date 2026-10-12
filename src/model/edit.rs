//! Editing a dataset with its references kept whole: handles to records,
//! the index of identifiers, the count of pointers to each, and the
//! operations that keep them true.
//!
//! # Design
//!
//! Identifiers and pointers are interned [`XrefId`]s, so the integrity of
//! a dataset is a matter of tables keyed by id, not of the pointers
//! themselves:
//!
//! - the **definitions**: for each id, the record that has it (the first
//!   in [`Dataset::records`] order, with the others kept aside when several
//!   have it), and how many record identifiers each version's grammar
//!   refuses. Built as a dataset is read (one look at each record's
//!   identifier), they make every lookup by identifier and the writer's
//!   identifier check constant-time;
//! - the **counts**: for each id, how many pointers of the dataset hold it.
//!   They are built the first time they are asked for (one walk of every
//!   pointer: reading and writing never pay for them), then kept by every
//!   edit;
//! - the **handles**: a slot per record, which follows the record when the
//!   records before it are removed, and refuses to give a removed record's
//!   successor (a generation).
//!
//! Records are reached for writing only through the dataset, which counts
//! a record's pointers out when it lends it ([`Dataset::get_mut`]) and in
//! again when the loan ends ([`RecordMut`]'s drop): any field may be edited,
//! pointer fields and extension structures included, and the counts stay
//! exact, at a cost of the record's size per loan. A loan that is never
//! returned (a guard forgotten with `mem::forget`) is settled by the next
//! edit; until then the queries fall back to walking the records.
//!
//! A pointer per `Arc` (each pointer a counted reference to its target's
//! name) would count by construction too, but at 16 bytes and an atomic
//! operation per pointer, a heap allocation per identifier, pointers that
//! another dataset made counted in the wrong one, and fields still public
//! for a program to fill with pointers no dataset counted. Here a pointer
//! stays a 4-byte id, a dataset holds a few bytes per record and per
//! identifier, and renaming an identifier changes one entry of the store:
//! every pointer follows.

use std::fmt;
use std::marker::PhantomData;
use std::sync::OnceLock;

use super::citation::{Citation, CitationSource, RepositoryCitation};
use super::dataset::{Dataset, Record, RecordRef, XrefKey};
use super::header::Header;
use super::link::{Association, ChildLink, IndividualRef, SpouseLink};
use super::list::ThinVec;
use super::multimedia::MultimediaLink;
use super::node::{Node, Value};
use super::note::Note;
use super::pointers::{Cut, Pointers};
use super::record::{
    Family, Individual, Multimedia, Repository, SharedNote, Source, Submission, Submitter,
};
use super::text::{Store, TagId, XrefId};

/// The type of a record, in the order [`Dataset::records`] gives them.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RecordKind {
    /// A submitter (`SUBM`).
    Submitter,
    /// A submission (5.5.1 `SUBN`).
    Submission,
    /// An individual (`INDI`).
    Individual,
    /// A family (`FAM`).
    Family,
    /// A shared note (5.5.1 `NOTE`, 7.x `SNOTE`).
    Note,
    /// A source (`SOUR`).
    Source,
    /// A repository (`REPO`).
    Repository,
    /// A multimedia object (`OBJE`).
    Multimedia,
    /// A record of no type the model has (an extension record).
    Other,
}

const KINDS: [RecordKind; 9] = [
    RecordKind::Submitter,
    RecordKind::Submission,
    RecordKind::Individual,
    RecordKind::Family,
    RecordKind::Note,
    RecordKind::Source,
    RecordKind::Repository,
    RecordKind::Multimedia,
    RecordKind::Other,
];

/// The kind bits of a free slot.
const FREE: u32 = 0xF;

impl RecordKind {
    pub(crate) const fn index(self) -> usize {
        self as usize
    }

    fn of(bits: u32) -> Option<Self> {
        KINDS.get(bits as usize).copied()
    }

    /// The prefix of the identifiers [`Dataset::add`] gives records of
    /// this kind (`@I1@`, `@F1@`, …), as the writer does.
    fn prefix(self) -> Option<&'static str> {
        Some(match self {
            RecordKind::Individual => "I",
            RecordKind::Family => "F",
            RecordKind::Source => "S",
            RecordKind::Repository => "R",
            RecordKind::Multimedia => "M",
            RecordKind::Submitter => "U",
            RecordKind::Submission => "SUBN",
            RecordKind::Note => "N",
            RecordKind::Other => return None,
        })
    }

    /// The record's type in words, for errors.
    fn name(self) -> &'static str {
        match self {
            RecordKind::Submitter => "a submitter",
            RecordKind::Submission => "a submission",
            RecordKind::Individual => "an individual",
            RecordKind::Family => "a family",
            RecordKind::Note => "a shared note",
            RecordKind::Source => "a source",
            RecordKind::Repository => "a repository",
            RecordKind::Multimedia => "a multimedia object",
            RecordKind::Other => "an extension record",
        }
    }
}

/// A record of a dataset, whatever its type: see [`Handle`].
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct RecordHandle {
    slot: u32,
    /// The slot's generation (high bits) and the record's kind (4 bits).
    tag: u32,
}

impl RecordHandle {
    /// The type of the record.
    #[must_use]
    pub fn kind(self) -> RecordKind {
        RecordKind::of(self.tag & FREE).unwrap_or(RecordKind::Other)
    }

    /// The handle with its type, when the record is a `T`.
    #[must_use]
    pub fn typed<T: RecordType>(self) -> Option<Handle<T>> {
        (self.kind() == T::KIND).then_some(Handle {
            raw: self,
            record: PhantomData,
        })
    }
}

impl fmt::Debug for RecordHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}#{}v{}", self.kind(), self.slot, self.tag >> 4)
    }
}

/// A record of a dataset, of type `T`: what [`Dataset::add`] returns and
/// [`Dataset::get`], [`Dataset::get_mut`] and [`Dataset::remove`] take.
///
/// A handle stays valid across every edit of the dataset (records added
/// or removed before it, identifiers renamed, links made) until its record
/// is removed; after that, no operation finds a record by it, even when a
/// new record takes its place. A handle is of the dataset that made it and
/// of its clones.
pub struct Handle<T> {
    raw: RecordHandle,
    record: PhantomData<fn() -> T>,
}

impl<T> Handle<T> {
    /// The handle without its type.
    #[must_use]
    pub fn erase(self) -> RecordHandle {
        self.raw
    }
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Handle<T> {}

impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<T> Eq for Handle<T> {}

impl<T> std::hash::Hash for Handle<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

impl<T> fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.raw.fmt(f)
    }
}

impl<T> From<Handle<T>> for RecordHandle {
    fn from(h: Handle<T>) -> Self {
        h.raw
    }
}

/// What holds a pointer: the header or a record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Holder {
    /// The header.
    Header,
    /// A record.
    Record(RecordHandle),
}

impl From<RecordHandle> for Holder {
    fn from(h: RecordHandle) -> Self {
        Holder::Record(h)
    }
}

impl<T> From<Handle<T>> for Holder {
    fn from(h: Handle<T>) -> Self {
        Holder::Record(h.raw)
    }
}

/// What [`Dataset::remove`] does when other structures point to the
/// record.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Removal {
    /// Refuse to remove it ([`EditError::StillReferenced`]).
    #[default]
    Refuse,
    /// Remove every structure that points to it, with its substructures
    /// (a family's `CHIL`, an individual's `FAMC`, a citation, an
    /// extension structure; an untyped record whose payload points to it
    /// loses its payload), as [`Dataset::detach`] does.
    Detach,
    /// Remove it and leave the pointers to it dangling, as a file may
    /// have them ([`Dataset::dangling_references`] lists them).
    LeaveDangling,
}

/// A kind of pointer from a record (or the header) to another record:
/// what [`Dataset::link`] adds and [`Dataset::unlink`] removes.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Link {
    /// An individual's family as a child (`FAMC`), to a family.
    ChildOf,
    /// An individual's family as a spouse (`FAMS`), to a family.
    SpouseOf,
    /// A family's husband or first partner (`HUSB`), to an individual.
    Husband,
    /// A family's wife or second partner (`WIFE`), to an individual.
    Wife,
    /// A family's child (`CHIL`), to an individual.
    Child,
    /// An individual's alias (`ALIA`), to an individual.
    Alias,
    /// An association (`ASSO`) of an individual or a family, to an
    /// individual.
    Association,
    /// A citation (`SOUR`) of an individual, a family, a multimedia
    /// object or a shared note, to a source.
    Citation,
    /// A note (`NOTE @N1@`, 7.x `SNOTE`) of the header or a typed record,
    /// to a shared note.
    Note,
    /// A multimedia link (`OBJE`) of an individual, a family, a source or
    /// a submitter, to a multimedia object.
    Multimedia,
    /// A source's repository (`REPO`), to a repository.
    Repository,
    /// The submitter (`SUBM`) of the header, an individual, a family or a
    /// submission, to a submitter.
    Submitter,
    /// An individual's submitter interested in their ancestors (`ANCI`).
    AncestorInterest,
    /// An individual's submitter interested in their descendants
    /// (`DESI`).
    DescendantInterest,
    /// The header's submission (5.5.1 `HEAD.SUBN`), to a submission.
    Submission,
    /// An extension structure (`_TAG @X1@`) of any record or the header,
    /// to a record of any type: the node is added to its `extra`.
    Extension(TagId),
}

impl Link {
    /// The type of record the link points to; `None`: any.
    fn target(self) -> Option<RecordKind> {
        Some(match self {
            Link::ChildOf | Link::SpouseOf => RecordKind::Family,
            Link::Husband | Link::Wife | Link::Child | Link::Alias | Link::Association => {
                RecordKind::Individual
            }
            Link::Citation => RecordKind::Source,
            Link::Note => RecordKind::Note,
            Link::Multimedia => RecordKind::Multimedia,
            Link::Repository => RecordKind::Repository,
            Link::Submitter | Link::AncestorInterest | Link::DescendantInterest => {
                RecordKind::Submitter
            }
            Link::Submission => RecordKind::Submission,
            Link::Extension(_) => return None,
        })
    }
}

/// Why an edit was refused. The dataset is as it was.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    /// The handle's record was removed.
    StaleHandle,
    /// Another record has this identifier.
    DuplicateXref(Box<str>),
    /// No record has this identifier.
    UnknownXref(Box<str>),
    /// An identifier the dataset's store does not have: an id of another
    /// dataset, or of none.
    ForeignXref,
    /// Not an identifier: `@`, a character, `@`, without `@`, spaces or
    /// control characters inside.
    InvalidXref(Box<str>),
    /// Pointers of the dataset point to this identifier, which names no
    /// record: renaming a record to it would take them.
    XrefInUse(Box<str>),
    /// Other structures point to the record ([`Removal::Refuse`]).
    StillReferenced {
        /// The record's identifier.
        xref: Box<str>,
        /// The pointers to it from other records and the header.
        references: usize,
    },
    /// The record is not of the type the link points to.
    WrongType {
        /// The identifier.
        xref: Box<str>,
        /// The type the link points to.
        expected: RecordKind,
        /// The type of the record that has the identifier.
        found: RecordKind,
    },
    /// The holder has no structure of this kind (a family has no `FAMC`).
    NotApplicable(Link),
    /// The holder already points there with a structure of this kind.
    AlreadyLinked(Link),
    /// The holder has no structure of this kind pointing there.
    NotLinked(Link),
    /// The holder's only structure of this kind points elsewhere (a
    /// family's `HUSB`; both partners for [`Dataset::link_spouse`]).
    Occupied(Link),
    /// The dataset has no header.
    NoHeader,
    /// The record has no identifier to point to.
    NoXref,
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditError::StaleHandle => f.write_str("the record was removed"),
            EditError::DuplicateXref(x) => write!(f, "{x}: another record has this identifier"),
            EditError::UnknownXref(x) => write!(f, "{x}: no record has this identifier"),
            EditError::ForeignXref => f.write_str("an identifier of another dataset"),
            EditError::InvalidXref(x) => write!(f, "{x:?}: not an identifier"),
            EditError::XrefInUse(x) => write!(f, "{x}: pointers name this identifier"),
            EditError::StillReferenced { xref, references } => {
                write!(f, "{xref}: {references} pointers to the record")
            }
            EditError::WrongType {
                xref,
                expected,
                found,
            } => write!(f, "{xref}: {}, not {}", found.name(), expected.name()),
            EditError::NotApplicable(l) => write!(f, "no {l:?} link from this record"),
            EditError::AlreadyLinked(l) => write!(f, "already a {l:?} link to this record"),
            EditError::NotLinked(l) => write!(f, "no {l:?} link to this record"),
            EditError::Occupied(l) => write!(f, "the {l:?} link points to another record"),
            EditError::NoHeader => f.write_str("the dataset has no header"),
            EditError::NoXref => f.write_str("the record has no identifier"),
        }
    }
}

impl std::error::Error for EditError {}

pub(crate) mod sealed {
    use super::super::dataset::Dataset;
    use super::super::text::XrefId;

    /// What the dataset needs of a record type: where its records are,
    /// and their pointers. Not nameable outside the crate, so that only the
    /// dataset reaches its lists.
    pub trait Record: Sized {
        fn list(data: &Dataset) -> &[Self];
        fn at_mut(data: &mut Dataset, pos: usize) -> Option<&mut Self>;
        fn push_to(data: &mut Dataset, record: Self);
        fn remove_from(data: &mut Dataset, pos: usize) -> Self;
        fn xref_of(&self) -> Option<XrefId>;
        fn set_xref(&mut self, xref: Option<XrefId>);
        fn pointers(&self, f: &mut dyn FnMut(XrefId));
    }

    /// What can be lent by [`RecordMut`](super::RecordMut): a record or
    /// the header.
    pub trait Edit {
        fn lent(data: &Dataset, pos: usize) -> Option<&Self>;
        fn lent_mut(data: &mut Dataset, pos: usize) -> Option<&mut Self>;
    }
}

/// A type of record: [`Individual`], [`Family`], [`Source`],
/// [`Repository`], [`Multimedia`], [`Submitter`], [`Submission`],
/// [`SharedNote`], or [`Node`] for the records of no type the model has.
pub trait RecordType: sealed::Record + Editable + 'static {
    /// The kind of record.
    const KIND: RecordKind;
}

/// What a [`RecordMut`] lends: a record ([`RecordType`]) or the
/// [`Header`].
pub trait Editable: sealed::Edit {}

macro_rules! record_type {
    ($($ty:ty => $kind:ident, $list:ident;)*) => {$(
        impl sealed::Record for $ty {
            fn list(data: &Dataset) -> &[Self] {
                &data.$list
            }
            fn at_mut(data: &mut Dataset, pos: usize) -> Option<&mut Self> {
                data.$list.get_mut(pos)
            }
            fn push_to(data: &mut Dataset, record: Self) {
                data.$list.push(record);
            }
            fn remove_from(data: &mut Dataset, pos: usize) -> Self {
                data.$list.remove(pos)
            }
            fn xref_of(&self) -> Option<XrefId> {
                self.xref
            }
            fn set_xref(&mut self, xref: Option<XrefId>) {
                self.xref = xref;
            }
            fn pointers(&self, f: &mut dyn FnMut(XrefId)) {
                Pointers::each(self, f);
            }
        }

        impl sealed::Edit for $ty {
            fn lent(data: &Dataset, pos: usize) -> Option<&Self> {
                data.$list.get(pos)
            }
            fn lent_mut(data: &mut Dataset, pos: usize) -> Option<&mut Self> {
                data.$list.get_mut(pos)
            }
        }

        impl Editable for $ty {}

        impl RecordType for $ty {
            const KIND: RecordKind = RecordKind::$kind;
        }
    )*};
}

record_type! {
    Submitter => Submitter, submitters;
    Submission => Submission, submissions;
    Individual => Individual, individuals;
    Family => Family, families;
    SharedNote => Note, notes;
    Source => Source, sources;
    Repository => Repository, repositories;
    Multimedia => Multimedia, multimedia;
    Node => Other, extra;
}

impl sealed::Edit for Header {
    fn lent(data: &Dataset, _pos: usize) -> Option<&Self> {
        data.header.as_ref()
    }
    fn lent_mut(data: &mut Dataset, _pos: usize) -> Option<&mut Self> {
        data.header.as_mut()
    }
}

impl Editable for Header {}

/// Runs `$body` with `$r` the record of `$kind` at `$pos` of `$data`
/// (mutable), whatever its type: `Some` of the result, `None` when there
/// is no such record.
macro_rules! on_record_mut {
    ($data:expr, $kind:expr, $pos:expr, |$r:ident| $body:expr) => {
        match $kind {
            RecordKind::Submitter => $data.submitters.get_mut($pos).map(|$r| $body),
            RecordKind::Submission => $data.submissions.get_mut($pos).map(|$r| $body),
            RecordKind::Individual => $data.individuals.get_mut($pos).map(|$r| $body),
            RecordKind::Family => $data.families.get_mut($pos).map(|$r| $body),
            RecordKind::Note => $data.notes.get_mut($pos).map(|$r| $body),
            RecordKind::Source => $data.sources.get_mut($pos).map(|$r| $body),
            RecordKind::Repository => $data.repositories.get_mut($pos).map(|$r| $body),
            RecordKind::Multimedia => $data.multimedia.get_mut($pos).map(|$r| $body),
            RecordKind::Other => $data.extra.get_mut($pos).map(|$r| $body),
        }
    };
}

/// The record of `$kind` at `$pos` of `$data`, as a [`RecordRef`],
/// borrowing only the list it is in.
macro_rules! record_ref {
    ($data:expr, $kind:expr, $pos:expr) => {
        match $kind {
            RecordKind::Submitter => $data.submitters.get($pos).map(RecordRef::Submitter),
            RecordKind::Submission => $data.submissions.get($pos).map(RecordRef::Submission),
            RecordKind::Individual => $data.individuals.get($pos).map(RecordRef::Individual),
            RecordKind::Family => $data.families.get($pos).map(RecordRef::Family),
            RecordKind::Note => $data.notes.get($pos).map(RecordRef::Note),
            RecordKind::Source => $data.sources.get($pos).map(RecordRef::Source),
            RecordKind::Repository => $data.repositories.get($pos).map(RecordRef::Repository),
            RecordKind::Multimedia => $data.multimedia.get($pos).map(RecordRef::Multimedia),
            RecordKind::Other => $data.extra.get($pos).map(RecordRef::Other),
        }
    };
}

/// A handle's slot: the record's position in its list, and the slot's
/// generation and the record's kind (`FREE` when the slot is free).
#[derive(Clone, Copy, Debug)]
struct Slot {
    pos: u32,
    tag: u32,
}

/// Where a lent record is: the header, or a record's kind and position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lent {
    Header,
    Record(RecordKind, usize),
}

/// The bits of a [`Place`] that hold the position.
const POS_BITS: u32 = 28;
/// Where a [`Place`]'s kind bits start.
pub(crate) const PLACE_KIND_SHIFT: u32 = POS_BITS;
/// A [`Place`]'s position part for a record too far in its list to say
/// where: it is searched for.
const FAR: u32 = (1 << POS_BITS) - 1;

/// Where a record is: its kind (the high 4 bits) and its position plus one
/// (the low 28 bits, [`FAR`] past them); 0 for none.
type Place = u32;

fn place(kind: RecordKind, pos: usize) -> Place {
    let pos = u32::try_from(pos)
        .ok()
        .and_then(|p| p.checked_add(1))
        .filter(|&p| p < FAR)
        .unwrap_or(FAR);
    (u32::try_from(kind.index()).unwrap_or(0) << POS_BITS) | pos
}

fn place_kind(place: Place) -> Option<RecordKind> {
    (place != 0)
        .then(|| RecordKind::of(place >> POS_BITS))
        .flatten()
}

/// The position of a place; `None` for none, or a record too far.
fn place_pos(place: Place) -> Option<usize> {
    let pos = place & FAR;
    (place != 0 && pos != FAR).then(|| pos as usize - 1)
}

/// The handles of a dataset's records: built the first time a handle is
/// asked for, then kept by every edit.
#[derive(Clone, Debug, Default)]
struct Handles {
    /// By handle slot.
    slots: Vec<Slot>,
    /// Free slots, to reuse.
    free: Vec<u32>,
    /// By kind, the slot of each record, in order.
    order: [Vec<u32>; 9],
    /// By kind, the number the next identifier [`Dataset::add`] makes
    /// starts from.
    fresh: [u32; 9],
}

impl Handles {
    /// The handles of records whose lists have `lens` records, by kind.
    fn new(lens: [usize; 9]) -> Self {
        let mut handles = Self {
            slots: Vec::with_capacity(lens.iter().sum()),
            ..Self::default()
        };
        for kind in KINDS {
            let n = lens.get(kind.index()).copied().unwrap_or(0);
            if let Some(order) = handles.order.get_mut(kind.index()) {
                order.reserve_exact(n);
            }
            for pos in 0..n {
                handles.alloc(kind, pos);
            }
        }
        handles
    }

    fn slot(&self, slot: u32) -> Option<Slot> {
        self.slots.get(slot as usize).copied()
    }

    /// The kind and position of a live handle's record.
    fn locate(&self, h: RecordHandle) -> Option<(RecordKind, usize)> {
        let s = self.slot(h.slot).filter(|s| s.tag == h.tag)?;
        Some((RecordKind::of(s.tag & FREE)?, s.pos as usize))
    }

    fn handle(&self, slot: u32) -> Option<RecordHandle> {
        let s = self.slot(slot)?;
        (s.tag & FREE != FREE).then_some(RecordHandle { slot, tag: s.tag })
    }

    /// The handle of the record of `kind` at `pos`.
    fn at(&self, kind: RecordKind, pos: usize) -> Option<RecordHandle> {
        self.handle(*self.order.get(kind.index())?.get(pos)?)
    }

    /// A slot for a record of `kind` at `pos`, the last of its list.
    fn alloc(&mut self, kind: RecordKind, pos: usize) -> RecordHandle {
        let pos = u32::try_from(pos).unwrap_or(u32::MAX);
        let kind_bits = u32::try_from(kind.index()).unwrap_or(FREE);
        let slot = if let Some(slot) = self.free.pop() {
            if let Some(s) = self.slots.get_mut(slot as usize) {
                *s = Slot {
                    pos,
                    tag: (s.tag & !FREE) | kind_bits,
                };
            }
            slot
        } else {
            let slot = u32::try_from(self.slots.len()).unwrap_or(u32::MAX);
            self.slots.push(Slot {
                pos,
                tag: kind_bits,
            });
            slot
        };
        if let Some(order) = self.order.get_mut(kind.index()) {
            order.push(slot);
        }
        let tag = self.slot(slot).map_or(0, |s| s.tag);
        RecordHandle { slot, tag }
    }

    /// Frees the slot of the record of `kind` at `pos`, moving the slots of
    /// the records after it back by one.
    fn release(&mut self, kind: RecordKind, pos: usize) {
        let Some(order) = self.order.get_mut(kind.index()) else {
            return;
        };
        if pos >= order.len() {
            return;
        }
        let slot = order.remove(pos);
        for &later in order.iter().skip(pos) {
            if let Some(s) = self.slots.get_mut(later as usize) {
                s.pos -= 1;
            }
        }
        if let Some(s) = self.slots.get_mut(slot as usize) {
            // The next generation, free.
            s.tag = (s.tag & !FREE).wrapping_add(1 << 4) | FREE;
        }
        self.free.push(slot);
    }
}

/// The tables of a dataset's integrity: see the [module
/// documentation](self). Derived from the records: two datasets with the
/// same records are equal whatever their tables.
#[derive(Clone, Default)]
pub(crate) struct Index {
    /// By identifier id: the place of the first record (in
    /// [`Dataset::records`] order) that has it; 0 when none has.
    defs: Vec<Place>,
    /// The other records that have an identifier a record has before
    /// them: (identifier, place).
    more: Vec<(XrefId, Place)>,
    /// The records whose identifier is invalid in 5.5.1, in 7.x.
    invalid: [u32; 2],
    /// By identifier id, the pointers that hold it; built when first asked
    /// for.
    refs: OnceLock<Vec<u32>>,
    /// The handles; built when first asked for.
    handles: OnceLock<Box<Handles>>,
    /// The record lent out and not given back yet, which the other tables
    /// leave out.
    lent: Option<Lent>,
}

impl PartialEq for Index {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl fmt::Debug for Index {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Index")
            .field("duplicates", &self.more.len())
            .field("handles", &self.handles.get().is_some())
            .field("counted", &self.refs.get().is_some())
            .finish_non_exhaustive()
    }
}

impl Index {
    fn def(&self, id: XrefId) -> Place {
        self.defs.get(id.index()).copied().unwrap_or(0)
    }

    /// Records that the record at `at` has identifier `id`. An id the store
    /// does not have is no identifier: left out.
    fn define(&mut self, id: XrefId, at: Place, store: &Store) {
        if !store.has_xref(id) {
            return;
        }
        let (v551, v7) = store.xref_validity(id);
        self.invalid[0] += u32::from(!v551);
        self.invalid[1] += u32::from(!v7);
        self.place_def(id, at);
    }

    /// Records where the record that has `id` is, as [`Index::define`]
    /// does, its validity left to be counted.
    fn place_def(&mut self, id: XrefId, at: Place) {
        let i = id.index();
        if self.defs.len() <= i {
            // Ids come mostly in order: room for the next ones at once.
            let len = (i + 1).max(self.defs.len() * 2).max(64);
            self.defs.resize(len, 0);
        }
        let Some(first) = self.defs.get_mut(i) else {
            return;
        };
        if *first == 0 {
            *first = at;
        } else if at < *first {
            // Places compare in record order (kind, then position).
            self.more.push((id, *first));
            *first = at;
        } else {
            self.more.push((id, at));
        }
    }

    /// Forgets that the record at `at` has identifier `id`.
    fn undefine(&mut self, id: XrefId, at: Place, store: &Store) {
        if !store.has_xref(id) {
            return;
        }
        let (v551, v7) = store.xref_validity(id);
        self.invalid[0] -= u32::from(!v551);
        self.invalid[1] -= u32::from(!v7);
        if self.def(id) == at {
            // The next record with it, if any, comes first now.
            let next = self
                .more
                .iter()
                .enumerate()
                .filter(|(_, (x, _))| *x == id)
                .min_by_key(|(_, (_, p))| *p)
                .map(|(i, &(_, p))| (i, p));
            let new = match next {
                Some((i, p)) => {
                    self.more.swap_remove(i);
                    p
                }
                None => 0,
            };
            if let Some(d) = self.defs.get_mut(id.index()) {
                *d = new;
            }
        } else if let Some(i) = self.more.iter().position(|&m| m == (id, at)) {
            self.more.swap_remove(i);
        }
    }

    /// Adds `delta` to the count of each pointer `f` gives, when the counts
    /// are built. Ids from `limit` on are not the store's: they are not
    /// counted.
    fn count(&mut self, delta: i64, limit: usize, each: impl FnOnce(&mut dyn FnMut(XrefId))) {
        let Some(refs) = self.refs.get_mut() else {
            return;
        };
        each(&mut |p: XrefId| {
            let i = p.index();
            if i >= limit {
                return;
            }
            if refs.len() <= i {
                refs.resize(i + 1, 0);
            }
            if let Some(c) = refs.get_mut(i) {
                *c = u32::try_from(i64::from(*c) + delta).unwrap_or(0);
            }
        });
    }
}

/// A record or the header, lent by [`Dataset::get_mut`] or
/// [`Dataset::header_mut`]: it dereferences to the structure, every field
/// of which may be edited. When it is dropped, the dataset counts the
/// structure's pointers and identifier again.
pub struct RecordMut<'a, T: Editable> {
    data: &'a mut Dataset,
    pos: usize,
    record: PhantomData<fn() -> T>,
}

impl<T: Editable> std::ops::Deref for RecordMut<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        match T::lent(self.data, self.pos) {
            Some(r) => r,
            // Lent with the dataset borrowed: it stays where it was.
            None => unreachable!("a lent record is where it was"),
        }
    }
}

impl<T: Editable> std::ops::DerefMut for RecordMut<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        match T::lent_mut(self.data, self.pos) {
            Some(r) => r,
            None => unreachable!("a lent record is where it was"),
        }
    }
}

impl<T: Editable> Drop for RecordMut<'_, T> {
    fn drop(&mut self) {
        self.data.settle();
    }
}

impl<T: Editable + fmt::Debug> fmt::Debug for RecordMut<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

/// Whether `xref` has the shape of an identifier: `@`, at least one
/// character, `@`, with no `@`, space or control character inside.
fn is_xref_shaped(xref: &str) -> bool {
    xref.strip_prefix('@')
        .and_then(|x| x.strip_suffix('@'))
        .is_some_and(|inner| {
            !inner.is_empty()
                && !inner
                    .chars()
                    .any(|c| c == '@' || c.is_whitespace() || c.is_control())
        })
}

impl Dataset {
    // ---- Reading the records ------------------------------------------

    /// The header (`HEAD`), when the file has one.
    #[must_use]
    pub fn header(&self) -> Option<&Header> {
        self.header.as_ref()
    }

    /// The individuals (`INDI`), in order.
    #[must_use]
    pub fn individuals(&self) -> &[Individual] {
        &self.individuals
    }

    /// The families (`FAM`), in order.
    #[must_use]
    pub fn families(&self) -> &[Family] {
        &self.families
    }

    /// The sources (`SOUR`), in order.
    #[must_use]
    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    /// The repositories (`REPO`), in order.
    #[must_use]
    pub fn repositories(&self) -> &[Repository] {
        &self.repositories
    }

    /// The multimedia objects (`OBJE`), in order.
    #[must_use]
    pub fn multimedia(&self) -> &[Multimedia] {
        &self.multimedia
    }

    /// The submitters (`SUBM`), in order.
    #[must_use]
    pub fn submitters(&self) -> &[Submitter] {
        &self.submitters
    }

    /// The submissions (5.5.1 `SUBN`), in order.
    #[must_use]
    pub fn submissions(&self) -> &[Submission] {
        &self.submissions
    }

    /// The shared notes (5.5.1 `NOTE`, 7.x `SNOTE`), in order.
    #[must_use]
    pub fn notes(&self) -> &[SharedNote] {
        &self.notes
    }

    /// The records of no type the model has, in order.
    #[must_use]
    pub fn extra(&self) -> &[Node] {
        &self.extra
    }

    /// The records of type `T`, in order.
    #[must_use]
    pub fn all<T: RecordType>(&self) -> &[T] {
        T::list(self)
    }

    // ---- Handles --------------------------------------------------------

    /// The handles, built the first time they are asked for: reading and
    /// writing never need them.
    fn handle_table(&self) -> &Handles {
        self.index
            .handles
            .get_or_init(|| Box::new(Handles::new(self.lens())))
    }

    /// The handles, to change with the records.
    fn handle_table_mut(&mut self) -> &mut Handles {
        if self.index.handles.get().is_none() {
            let handles = Box::new(Handles::new(self.lens()));
            let _ = self.index.handles.set(handles);
        }
        match self.index.handles.get_mut() {
            Some(h) => h,
            None => unreachable!("the handles were just built"),
        }
    }

    /// The number of records of each kind.
    fn lens(&self) -> [usize; 9] {
        KINDS.map(|kind| self.len_of(kind))
    }

    fn len_of(&self, kind: RecordKind) -> usize {
        match kind {
            RecordKind::Submitter => self.submitters.len(),
            RecordKind::Submission => self.submissions.len(),
            RecordKind::Individual => self.individuals.len(),
            RecordKind::Family => self.families.len(),
            RecordKind::Note => self.notes.len(),
            RecordKind::Source => self.sources.len(),
            RecordKind::Repository => self.repositories.len(),
            RecordKind::Multimedia => self.multimedia.len(),
            RecordKind::Other => self.extra.len(),
        }
    }

    /// The handles of the records of type `T`, in order.
    pub fn handles<T: RecordType>(&self) -> impl Iterator<Item = Handle<T>> + '_ {
        let handles = self.handle_table();
        handles
            .order
            .get(T::KIND.index())
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .filter_map(|&slot| handles.handle(slot))
            .filter_map(RecordHandle::typed)
    }

    /// The handle of the record of type `T` at `position` in its list.
    #[must_use]
    pub fn handle_at<T: RecordType>(&self, position: usize) -> Option<Handle<T>> {
        self.handle_table().at(T::KIND, position)?.typed()
    }

    /// The position of the first record of `kind` with identifier `id`, in
    /// constant time (but for a record lent, or past 2^28 records of its
    /// kind: then a search).
    pub(crate) fn first_of(&self, kind: RecordKind, id: XrefId) -> Option<usize> {
        let search = || match kind {
            RecordKind::Submitter => self.submitters.iter().position(|r| r.xref == Some(id)),
            RecordKind::Submission => self.submissions.iter().position(|r| r.xref == Some(id)),
            RecordKind::Individual => self.individuals.iter().position(|r| r.xref == Some(id)),
            RecordKind::Family => self.families.iter().position(|r| r.xref == Some(id)),
            RecordKind::Note => self.notes.iter().position(|r| r.xref == Some(id)),
            RecordKind::Source => self.sources.iter().position(|r| r.xref == Some(id)),
            RecordKind::Repository => self.repositories.iter().position(|r| r.xref == Some(id)),
            RecordKind::Multimedia => self.multimedia.iter().position(|r| r.xref == Some(id)),
            RecordKind::Other => self.extra.iter().position(|r| r.xref == Some(id)),
        };
        if self.index.lent.is_some() {
            return search();
        }
        let first = self.index.def(id);
        // The first of its kind: where the index says, else among those
        // after a record of another kind.
        let at = if place_kind(first) == Some(kind) {
            first
        } else {
            self.index
                .more
                .iter()
                .filter(|(x, p)| *x == id && place_kind(*p) == Some(kind))
                .map(|&(_, p)| p)
                .min()?
        };
        match place_pos(at) {
            Some(pos) => Some(pos),
            None => search(),
        }
    }

    /// The handle of the record of type `T` with this identifier (the
    /// first, when several have it). Constant time.
    #[must_use]
    pub fn handle<T: RecordType>(&self, xref: impl XrefKey) -> Option<Handle<T>> {
        let id = xref.id_in(&self.store)?;
        self.handle_at(self.first_of(T::KIND, id)?)
    }

    /// The first record with identifier `id`, whatever its type: its kind
    /// and position.
    fn first_record(&self, id: XrefId) -> Option<(RecordKind, usize)> {
        if self.index.lent.is_some() {
            return KINDS
                .into_iter()
                .find_map(|kind| Some((kind, self.first_of(kind, id)?)));
        }
        let kind = place_kind(self.index.def(id))?;
        Some((kind, self.first_of(kind, id)?))
    }

    /// The handle of the record with this identifier, whatever its type
    /// (the first, when several have it). Constant time.
    #[must_use]
    pub fn record_handle(&self, xref: impl XrefKey) -> Option<RecordHandle> {
        let (kind, pos) = self.first_record(xref.id_in(&self.store)?)?;
        self.handle_table().at(kind, pos)
    }

    /// The record with identifier `id`, whatever its type: the first, when
    /// several have it. Constant time, and no handle is made.
    pub(crate) fn find_record(&self, id: XrefId) -> Option<RecordRef<'_>> {
        let (kind, pos) = self.first_record(id)?;
        self.record_at(kind, pos)
    }

    /// Every record but the header with its handle, in
    /// [`Dataset::records`] order.
    pub fn records_with_handles(&self) -> impl Iterator<Item = (RecordHandle, RecordRef<'_>)> {
        let handles = self.handle_table();
        KINDS.into_iter().flat_map(move |kind| {
            handles
                .order
                .get(kind.index())
                .map_or(&[][..], Vec::as_slice)
                .iter()
                .enumerate()
                .filter_map(move |(pos, &slot)| {
                    Some((handles.handle(slot)?, self.record_at(kind, pos)?))
                })
        })
    }

    /// The record of a handle.
    #[must_use]
    pub fn get<T: RecordType>(&self, handle: Handle<T>) -> Option<&T> {
        let (_, pos) = self.handle_table().locate(handle.raw)?;
        T::list(self).get(pos)
    }

    /// The record of a handle, whatever its type.
    #[must_use]
    pub fn get_any(&self, handle: RecordHandle) -> Option<RecordRef<'_>> {
        let (kind, pos) = self.handle_table().locate(handle)?;
        self.record_at(kind, pos)
    }

    /// The position of a handle's record in its list.
    #[must_use]
    pub fn position<T: RecordType>(&self, handle: Handle<T>) -> Option<usize> {
        self.handle_table().locate(handle.raw).map(|(_, pos)| pos)
    }

    /// The record of a handle, to edit: any field, pointers and
    /// identifier included. The dataset counts its pointers and its
    /// identifier again when the returned guard is dropped (see
    /// [`RecordMut`]); an identifier another record has is kept as reading
    /// keeps one, the first record having it ([`Dataset::rename`] refuses
    /// it instead).
    pub fn get_mut<T: RecordType>(&mut self, handle: Handle<T>) -> Option<RecordMut<'_, T>> {
        self.settle();
        let (kind, pos) = self.handle_table().locate(handle.raw)?;
        self.lend(Lent::Record(kind, pos));
        Some(RecordMut {
            data: self,
            pos,
            record: PhantomData,
        })
    }

    /// The record of type `T` at `position` in its list, to edit (see
    /// [`Dataset::get_mut`]).
    pub fn get_mut_at<T: RecordType>(&mut self, position: usize) -> Option<RecordMut<'_, T>> {
        self.settle();
        T::list(self).get(position)?;
        self.lend(Lent::Record(T::KIND, position));
        Some(RecordMut {
            data: self,
            pos: position,
            record: PhantomData,
        })
    }

    /// Edits every record of type `T` in turn, in order: each is lent to
    /// `edit` as [`Dataset::get_mut`] lends it.
    pub fn edit_all<T: RecordType>(&mut self, mut edit: impl FnMut(&mut T)) {
        for position in 0..T::list(self).len() {
            if let Some(mut record) = self.get_mut_at::<T>(position) {
                edit(&mut record);
            }
        }
    }

    /// The header, to edit (see [`Dataset::get_mut`]).
    pub fn header_mut(&mut self) -> Option<RecordMut<'_, Header>> {
        self.settle();
        self.header.as_ref()?;
        self.lend(Lent::Header);
        Some(RecordMut {
            data: self,
            pos: 0,
            record: PhantomData,
        })
    }

    /// Replaces the header (with none: removes it); the one it had.
    pub fn set_header(&mut self, header: Option<Header>) -> Option<Header> {
        self.settle();
        if let Some(old) = &self.header {
            self.index
                .count(-1, self.store.xref_count(), |f| old.each(f));
        }
        let old = std::mem::replace(&mut self.header, header);
        if let Some(new) = &self.header {
            self.index
                .count(1, self.store.xref_count(), |f| new.each(f));
        }
        old
    }

    // ---- The tables -----------------------------------------------------

    /// The record of `kind` at `pos`.
    fn record_at(&self, kind: RecordKind, pos: usize) -> Option<RecordRef<'_>> {
        record_ref!(self, kind, pos)
    }

    /// The tables built anew from the records, in one pass over their
    /// identifiers: handles made before are no longer valid. For datasets
    /// made whole (read, deserialized).
    #[cfg(feature = "serde")]
    pub(crate) fn reindex(&mut self) {
        self.index = Index::default();
        macro_rules! walk {
            ($kind:ident, $list:expr) => {
                for (pos, r) in $list.iter().enumerate() {
                    if let Some(id) = r.xref {
                        self.index.place_def(id, place(RecordKind::$kind, pos));
                    }
                }
            };
        }
        walk!(Submitter, self.submitters);
        walk!(Submission, self.submissions);
        walk!(Individual, self.individuals);
        walk!(Family, self.families);
        walk!(Note, self.notes);
        walk!(Source, self.sources);
        walk!(Repository, self.repositories);
        walk!(Multimedia, self.multimedia);
        walk!(Other, self.extra);
        self.index_read();
    }

    /// Notes, as a reader places it, that the record at `pos` of `kind` has
    /// identifier `id` (the store not yet at hand: [`Dataset::index_read`]
    /// completes the tables).
    pub(crate) fn define_read(&mut self, kind: RecordKind, pos: usize, id: Option<XrefId>) {
        if let Some(id) = id {
            self.index.place_def(id, place(kind, pos));
        }
    }

    /// Completes the tables once every record is read and the store is
    /// the dataset's: the identifiers of another store left out, the
    /// invalid ones counted, from the compact tables, not the records.
    pub(crate) fn index_read(&mut self) {
        let store = &self.store;
        let index = &mut self.index;
        if index.defs.len() > store.xref_count() {
            // Ids of no identifier of the store: none is defined.
            index.defs.truncate(store.xref_count());
        }
        index.defs.shrink_to_fit();
        index.more.retain(|(id, _)| store.has_xref(*id));
        let mut invalid = [0_u32; 2];
        let ids = index
            .defs
            .iter()
            .enumerate()
            .filter(|(_, &d)| d != 0)
            .map(|(i, _)| i)
            .chain(index.more.iter().map(|(id, _)| id.index()));
        for i in ids {
            let (v551, v7) = store.xref_validity_at(i);
            invalid[0] += u32::from(!v551);
            invalid[1] += u32::from(!v7);
        }
        index.invalid = invalid;
    }

    /// Every pointer of the dataset, counted by id, leaving out the record
    /// lent.
    fn count_all(&self) -> Vec<u32> {
        let mut refs = vec![0_u32; self.store.xref_count()];
        let mut add = |p: XrefId| {
            if let Some(c) = refs.get_mut(p.index()) {
                *c += 1;
            }
        };
        if self.index.lent != Some(Lent::Header) {
            if let Some(h) = &self.header {
                h.each(&mut add);
            }
        }
        let lent = match self.index.lent {
            Some(Lent::Record(kind, pos)) => Some((kind, pos)),
            _ => None,
        };
        for kind in KINDS {
            let skip = lent.filter(|(k, _)| *k == kind).map(|(_, pos)| pos);
            macro_rules! walk {
                ($list:expr) => {
                    for (pos, r) in $list.iter().enumerate() {
                        if Some(pos) != skip {
                            r.each(&mut add);
                        }
                    }
                };
            }
            match kind {
                RecordKind::Submitter => walk!(self.submitters),
                RecordKind::Submission => walk!(self.submissions),
                RecordKind::Individual => walk!(self.individuals),
                RecordKind::Family => walk!(self.families),
                RecordKind::Note => walk!(self.notes),
                RecordKind::Source => walk!(self.sources),
                RecordKind::Repository => walk!(self.repositories),
                RecordKind::Multimedia => walk!(self.multimedia),
                RecordKind::Other => walk!(self.extra),
            }
        }
        refs
    }

    /// The counts of pointers by id, built when first asked for.
    fn refs(&self) -> &[u32] {
        self.index.refs.get_or_init(|| self.count_all())
    }

    /// Counts the counts in, when they are not yet.
    fn counted(&mut self) {
        if self.index.refs.get().is_none() {
            let refs = self.count_all();
            let _ = self.index.refs.set(refs);
        }
    }

    /// Takes a record or the header out of the tables, before lending it.
    fn lend(&mut self, at: Lent) {
        self.counted();
        self.unlist(at);
        self.index.lent = Some(at);
    }

    /// Gives the record lent back to the tables. Every edit begins with
    /// it: a guard forgotten instead of dropped leaves its record lent.
    pub(crate) fn settle(&mut self) {
        if let Some(at) = self.index.lent.take() {
            self.list(at);
        }
    }

    /// Takes the identifier and pointers of the structure at `at` out of
    /// the tables (`delta` -1), or puts them in (+1).
    fn tally(&mut self, at: Lent, delta: i64) {
        let limit = self.store.xref_count();
        match at {
            Lent::Header => {
                if let Some(h) = &self.header {
                    self.index.count(delta, limit, |f| h.each(f));
                }
            }
            Lent::Record(kind, pos) => {
                let Some(record) = record_ref!(self, kind, pos) else {
                    return;
                };
                let xref = record.xref();
                self.index.count(delta, limit, |f| record.each_pointer(f));
                if let Some(id) = xref {
                    if delta < 0 {
                        self.index.undefine(id, place(kind, pos), &self.store);
                    } else {
                        self.index.define(id, place(kind, pos), &self.store);
                    }
                }
            }
        }
    }

    fn unlist(&mut self, at: Lent) {
        self.tally(at, -1);
    }

    fn list(&mut self, at: Lent) {
        self.tally(at, 1);
    }

    /// Whether a pointer to `id` points to nothing by definition: 7.x
    /// `@VOID@`, a 5.5.1 pointer into a record or another dataset.
    pub(crate) fn points_nowhere(&self, id: XrefId) -> bool {
        let text = self.store.xref(id);
        (self.version.is_v7() && text == crate::tree::Xref::VOID)
            || crate::spec::is_external_pointer(
                text,
                crate::spec::payload::Family::of(self.version.rules()),
            )
    }

    /// Whether a record has `id`.
    fn is_defined(&self, id: XrefId) -> bool {
        if self.index.lent.is_some() {
            return self.records().any(|r| r.xref() == Some(id));
        }
        self.index.def(id) != 0
    }

    /// Whether the identifiers of the records are unique and valid in the
    /// grammar of `rules`, and no record is lent: the writer then renames
    /// none, and the index tells the record of each identifier. Constant
    /// time.
    pub(crate) fn xrefs_conform(&self, rules: &crate::version::VersionRules) -> bool {
        let i = usize::from(matches!(rules.xref, crate::version::XrefGrammar::V7));
        self.index.lent.is_none()
            && self.index.more.is_empty()
            && self.index.invalid.get(i) == Some(&0)
    }

    /// By identifier id, where the record that has it is: its kind
    /// ([`RecordKind`] order) from bit [`PLACE_KIND_SHIFT`] on; 0 when no
    /// record has it.
    pub(crate) fn places(&self) -> &[u32] {
        &self.index.defs
    }

    // ---- References -----------------------------------------------------

    /// The number of pointers to this identifier, from the records and the
    /// header. Constant time once the counts are built, which the first
    /// query (or edit that needs them) does, walking every pointer once.
    #[must_use]
    pub fn references(&self, xref: impl XrefKey) -> usize {
        let Some(id) = xref.id_in(&self.store) else {
            return 0;
        };
        let mut n = self.refs().get(id.index()).copied().unwrap_or(0) as usize;
        if let Some(at) = self.index.lent {
            // The record lent is out of the counts.
            self.each_pointer_at(at, &mut |p| n += usize::from(p == id));
        }
        n
    }

    /// The header and records holding a pointer to this identifier, in
    /// [`Dataset::records`] order. Walks the records, unless the counts
    /// say none.
    #[must_use]
    pub fn referrers(&self, xref: impl XrefKey) -> Vec<Holder> {
        let Some(id) = xref.id_in(&self.store) else {
            return Vec::new();
        };
        if self.references(id) == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        if let Some(h) = &self.header {
            let mut found = false;
            h.each(&mut |p| found |= p == id);
            if found {
                out.push(Holder::Header);
            }
        }
        for (handle, record) in self.records_with_handles() {
            let mut found = false;
            record.each_pointer(&mut |p| found |= p == id);
            if found {
                out.push(Holder::Record(handle));
            }
        }
        out
    }

    fn each_pointer_at(&self, at: Lent, f: &mut dyn FnMut(XrefId)) {
        match at {
            Lent::Header => {
                if let Some(h) = &self.header {
                    h.each(f);
                }
            }
            Lent::Record(kind, pos) => {
                if let Some(r) = self.record_at(kind, pos) {
                    r.each_pointer(f);
                }
            }
        }
    }

    /// The id of the identifier of a record of the dataset, to point to it.
    ///
    /// # Errors
    ///
    /// [`EditError::UnknownXref`] when no record has it.
    pub fn pointer(&self, xref: impl XrefKey) -> Result<XrefId, EditError> {
        let text = xref.text_in(&self.store);
        match xref.id_in(&self.store) {
            Some(id) if self.is_defined(id) => Ok(id),
            _ => Err(EditError::UnknownXref(text)),
        }
    }

    /// The id of `xref` when a record of the expected kind has it.
    fn target(
        &self,
        xref: impl XrefKey,
        expected: Option<RecordKind>,
    ) -> Result<XrefId, EditError> {
        let unknown = EditError::UnknownXref(xref.text_in(&self.store));
        let id = xref.id_in(&self.store).ok_or(unknown)?;
        if !self.store.has_xref(id) {
            return Err(EditError::ForeignXref);
        }
        let text = || -> Box<str> { self.store.xref(id).into() };
        let (found, _) = self
            .first_record(id)
            .ok_or_else(|| EditError::UnknownXref(text()))?;
        match expected {
            // A record of the expected type may have it after another.
            Some(kind) if found != kind && self.first_of(kind, id).is_none() => {
                Err(EditError::WrongType {
                    xref: text(),
                    expected: kind,
                    found,
                })
            }
            _ => Ok(id),
        }
    }

    // ---- Adding and removing records -------------------------------------

    /// Adds a record, checked: its identifier is the dataset's and no other
    /// record has it, and each of its pointers points to a record of the
    /// dataset (or to itself, or nowhere by definition: 7.x `@VOID@`).
    /// A record without an identifier is given a free one of its type
    /// (`@I1@`, `@F1@`, …, numbered on from the last one given; an
    /// extension record none). Its handle.
    ///
    /// Its identifier and pointers are ids of this dataset's store
    /// ([`Dataset::pointer`], [`Dataset::store_mut`]).
    ///
    /// # Errors
    ///
    /// [`EditError::DuplicateXref`] when another record has its identifier,
    /// [`EditError::UnknownXref`] when a pointer points to no record,
    /// [`EditError::ForeignXref`] when an id is not of this dataset.
    ///
    /// ```rust
    /// use ged_io::model::{ChildLink, Dataset, Family, Individual, IndividualRef};
    /// use ged_io::GedcomVersion;
    ///
    /// let mut data = Dataset::new(GedcomVersion::V7_0);
    /// let ann = data.add(Individual::default()).unwrap();
    /// let ann_id = data.get(ann).unwrap().xref.unwrap();
    /// assert_eq!(data.store().xref(ann_id), "@I1@");
    /// let family = data
    ///     .add(Family { children: vec![IndividualRef::new(ann_id)], ..Family::default() })
    ///     .unwrap();
    /// assert_eq!(data.references(ann_id), 1);
    /// ```
    pub fn add<T: RecordType>(&mut self, mut record: T) -> Result<Handle<T>, EditError> {
        self.settle();
        let own = if let Some(id) = record.xref_of() {
            if !self.store.has_xref(id) {
                return Err(EditError::ForeignXref);
            }
            if self.is_defined(id) {
                return Err(EditError::DuplicateXref(self.store.xref(id).into()));
            }
            Some(id)
        } else {
            let fresh = self.fresh_xref(T::KIND);
            record.set_xref(fresh);
            fresh
        };
        let mut problem = None;
        record.pointers(&mut |p| {
            if problem.is_some() || Some(p) == own {
                return;
            }
            if !self.store.has_xref(p) {
                problem = Some(EditError::ForeignXref);
            } else if !self.is_defined(p) && !self.points_nowhere(p) {
                problem = Some(EditError::UnknownXref(self.store.xref(p).into()));
            }
        });
        if let Some(problem) = problem {
            return Err(problem);
        }
        Ok(self.place(record))
    }

    /// Puts a record at the end of its list, in the tables: unchecked.
    fn place<T: RecordType>(&mut self, record: T) -> Handle<T> {
        // Built first, the handles take the record as it is placed.
        self.handle_table_mut();
        let pos = self.place_unhandled(record);
        let raw = self.handle_table().at(T::KIND, pos);
        Handle {
            raw: raw.unwrap_or(RecordHandle {
                slot: u32::MAX,
                tag: FREE,
            }),
            record: PhantomData,
        }
    }

    /// [`Dataset::place`], making no handle unless the handles are built:
    /// the record's position.
    fn place_unhandled<T: RecordType>(&mut self, record: T) -> usize {
        let pos = T::list(self).len();
        T::push_to(self, record);
        self.list(Lent::Record(T::KIND, pos));
        if let Some(handles) = self.index.handles.get_mut() {
            handles.alloc(T::KIND, pos);
        }
        pos
    }

    /// Adds a record as [`Dataset::push`] does, making no handle: for
    /// readers, which add many.
    pub(crate) fn push_record(&mut self, record: Record) {
        self.settle();
        match record {
            Record::Header(r) => {
                self.set_header(Some(*r));
            }
            Record::Individual(r) => {
                self.place_unhandled(*r);
            }
            Record::Family(r) => {
                self.place_unhandled(*r);
            }
            Record::Source(r) => {
                self.place_unhandled(*r);
            }
            Record::Repository(r) => {
                self.place_unhandled(*r);
            }
            Record::Multimedia(r) => {
                self.place_unhandled(*r);
            }
            Record::Submitter(r) => {
                self.place_unhandled(*r);
            }
            Record::Submission(r) => {
                self.place_unhandled(*r);
            }
            Record::Note(r) => {
                self.place_unhandled(*r);
            }
            Record::Other(n) => {
                self.place_unhandled(*n);
            }
        }
    }

    /// Adds a record unchecked, as reading adds one: an identifier another
    /// record has, pointers to no record are kept. A header replaces the
    /// header (`None`); [`Record::Other`] goes to the untyped records.
    /// Its texts and identifiers are of this dataset (owned texts, ids of
    /// [`Dataset::store_mut`]).
    pub fn push(&mut self, record: Record) -> Option<RecordHandle> {
        self.settle();
        Some(match record {
            Record::Header(r) => {
                self.set_header(Some(*r));
                return None;
            }
            Record::Individual(r) => self.place(*r).erase(),
            Record::Family(r) => self.place(*r).erase(),
            Record::Source(r) => self.place(*r).erase(),
            Record::Repository(r) => self.place(*r).erase(),
            Record::Multimedia(r) => self.place(*r).erase(),
            Record::Submitter(r) => self.place(*r).erase(),
            Record::Submission(r) => self.place(*r).erase(),
            Record::Note(r) => self.place(*r).erase(),
            Record::Other(n) => self.place(*n).erase(),
        })
    }

    /// An identifier `@<prefix><n>@` no record has and no pointer holds,
    /// `n` counting on from the last one given to a record of `kind`.
    fn fresh_xref(&mut self, kind: RecordKind) -> Option<XrefId> {
        let prefix = kind.prefix()?;
        let mut n = self
            .handle_table_mut()
            .fresh
            .get(kind.index())
            .copied()?
            .max(1);
        loop {
            let candidate = format!("@{prefix}{n}@");
            let free = match self.store.find_xref(&candidate) {
                None => self.store.intern_xref(&candidate),
                Some(id) if !self.is_defined(id) && self.references(id) == 0 => Some(id),
                Some(_) => None,
            };
            n = n.checked_add(1)?;
            if let Some(id) = free {
                if let Some(next) = self.handle_table_mut().fresh.get_mut(kind.index()) {
                    *next = n;
                }
                return Some(id);
            }
        }
    }

    /// Removes a record, and its pointers from the counts: the record
    /// removed, its pointers as they were.
    ///
    /// When other structures point to it (unless another record has its
    /// identifier too, which they then point to), `how` says what to do:
    /// refuse, remove them ([`Removal::Detach`]) or leave them dangling.
    ///
    /// # Errors
    ///
    /// [`EditError::StaleHandle`] when the record was removed already,
    /// [`EditError::StillReferenced`] under [`Removal::Refuse`].
    ///
    /// ```rust
    /// use ged_io::model::{Dataset, Individual, Removal};
    ///
    /// let mut data = Dataset::parse(
    ///     "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n0 @F1@ FAM\n1 CHIL @I1@\n0 TRLR\n",
    /// );
    /// let ann = data.handle::<Individual>("@I1@").unwrap();
    /// assert!(data.remove(ann, Removal::Refuse).is_err()); // the family's CHIL
    /// data.remove(ann, Removal::Detach).unwrap();
    /// assert!(data.families()[0].children.is_empty());
    /// assert!(data.dangling_references().is_empty());
    /// ```
    pub fn remove<T: RecordType>(
        &mut self,
        handle: Handle<T>,
        how: Removal,
    ) -> Result<T, EditError> {
        self.settle();
        self.counted();
        let (kind, pos) = self
            .handle_table()
            .locate(handle.raw)
            .ok_or(EditError::StaleHandle)?;
        let record = T::list(self).get(pos).ok_or(EditError::StaleHandle)?;
        let xref = record.xref_of();
        // The pointers to it that its removal leaves without a record.
        let orphaned = match xref {
            Some(id)
                if self.index.def(id) == place(kind, pos)
                    && !self.index.more.iter().any(|m| m.0 == id) =>
            {
                let mut own = 0_usize;
                record.pointers(&mut |p| own += usize::from(p == id));
                self.references(id).saturating_sub(own)
            }
            _ => 0,
        };
        if orphaned > 0 && how == Removal::Refuse {
            let xref = xref.map_or_else(Box::default, |id| self.store.xref(id).into());
            return Err(EditError::StillReferenced {
                xref,
                references: orphaned,
            });
        }
        self.unlist(Lent::Record(kind, pos));
        self.handle_table_mut().release(kind, pos);
        let record = T::remove_from(self, pos);
        self.shift_places(kind, pos);
        if let (Some(id), true, Removal::Detach) = (xref, orphaned > 0, how) {
            self.detach(id);
        }
        Ok(record)
    }

    /// The places of the records of `kind` from `pos` on, one back: the
    /// record that was at `pos` is gone.
    fn shift_places(&mut self, kind: RecordKind, pos: usize) {
        let index = &mut self.index;
        let mut shift = |at: usize, id: Option<XrefId>| {
            let Some(id) = id else { return };
            let (old, new) = (place(kind, at + 1), place(kind, at));
            if let Some(d) = index.defs.get_mut(id.index()) {
                if *d == old {
                    *d = new;
                }
            }
        };
        macro_rules! walk {
            ($list:expr) => {
                for (at, r) in $list.iter().enumerate().skip(pos) {
                    shift(at, r.xref);
                }
            };
        }
        match kind {
            RecordKind::Submitter => walk!(self.submitters),
            RecordKind::Submission => walk!(self.submissions),
            RecordKind::Individual => walk!(self.individuals),
            RecordKind::Family => walk!(self.families),
            RecordKind::Note => walk!(self.notes),
            RecordKind::Source => walk!(self.sources),
            RecordKind::Repository => walk!(self.repositories),
            RecordKind::Multimedia => walk!(self.multimedia),
            RecordKind::Other => walk!(self.extra),
        }
        for (_, p) in &mut self.index.more {
            if place_kind(*p) == Some(kind) && place_pos(*p).is_some_and(|at| at > pos) {
                *p = place(kind, place_pos(*p).unwrap_or(pos) - 1);
            }
        }
    }

    // ---- Identifiers ------------------------------------------------------

    /// Renames an identifier: the records that have it and every pointer
    /// to it follow (pointers are ids, the store gives an id its text), in
    /// constant time. The new identifier's id.
    ///
    /// # Errors
    ///
    /// [`EditError::InvalidXref`] when `new` is not shaped as an
    /// identifier (`@…@`), [`EditError::DuplicateXref`] when a record has
    /// `new`, [`EditError::XrefInUse`] when pointers hold `new` (redirect
    /// them first: [`Dataset::redirect`]), [`EditError::UnknownXref`] when
    /// the dataset has no `xref`.
    ///
    /// ```rust
    /// use ged_io::model::Dataset;
    ///
    /// let mut data = Dataset::parse(
    ///     "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n0 @F1@ FAM\n1 CHIL @I1@\n0 TRLR\n",
    /// );
    /// data.rename("@I1@", "@ANN@").unwrap();
    /// let out = ged_io::GedcomWriter::new().write_to_string(&data).unwrap();
    /// assert!(out.contains("0 @ANN@ INDI\n") && out.contains("1 CHIL @ANN@\n"));
    /// ```
    pub fn rename(&mut self, xref: impl XrefKey, new: &str) -> Result<XrefId, EditError> {
        self.settle();
        let unknown = EditError::UnknownXref(xref.text_in(&self.store));
        let id = xref.id_in(&self.store).ok_or(unknown)?;
        if !self.store.has_xref(id) {
            return Err(EditError::ForeignXref);
        }
        if !is_xref_shaped(new) {
            return Err(EditError::InvalidXref(new.into()));
        }
        if self.store.xref(id) == new {
            return Ok(id);
        }
        if let Some(other) = self.store.find_xref(new) {
            if self.is_defined(other) {
                return Err(EditError::DuplicateXref(new.into()));
            }
            if self.references(other) > 0 {
                return Err(EditError::XrefInUse(new.into()));
            }
        }
        // The records that have it change validity with it.
        let holders = usize::from(self.index.def(id) != 0)
            + self.index.more.iter().filter(|m| m.0 == id).count();
        let (v551, v7) = self.store.xref_validity(id);
        if !self.store.rename_xref(id, new) {
            return Err(EditError::InvalidXref(new.into()));
        }
        let (n551, n7) = self.store.xref_validity(id);
        let n = u32::try_from(holders).unwrap_or(u32::MAX);
        self.index.invalid[0] = self.index.invalid[0] - n * u32::from(!v551) + n * u32::from(!n551);
        self.index.invalid[1] = self.index.invalid[1] - n * u32::from(!v7) + n * u32::from(!n7);
        Ok(id)
    }

    /// Points every pointer to `from` to `to` instead (to merge two
    /// records: redirect, then remove the one left without pointers). The
    /// number of pointers redirected.
    ///
    /// # Errors
    ///
    /// [`EditError::UnknownXref`] when no record has `to`.
    pub fn redirect(&mut self, from: impl XrefKey, to: impl XrefKey) -> Result<usize, EditError> {
        self.settle();
        let to = self.pointer(to)?;
        let Some(from) = from.id_in(&self.store) else {
            return Ok(0);
        };
        if from == to || self.references(from) == 0 {
            return Ok(0);
        }
        let mut n = 0_usize;
        let mut f = |p: XrefId| {
            if p == from {
                n += 1;
                to
            } else {
                p
            }
        };
        if let Some(h) = &mut self.header {
            h.map(&mut f);
        }
        for kind in KINDS {
            for pos in 0..self.len_of(kind) {
                on_record_mut!(self, kind, pos, |r| r.map(&mut f));
            }
        }
        let delta = i64::try_from(n).unwrap_or(i64::MAX);
        self.index
            .count(-delta, self.store.xref_count(), |c| c(from));
        self.index.count(delta, self.store.xref_count(), |c| c(to));
        Ok(n)
    }

    /// Removes every structure that points to this identifier, with its
    /// substructures, from the header and every record: the number
    /// removed. An untyped record whose own payload points there keeps its
    /// substructures and loses its payload. Walks the records holding such
    /// pointers, unless the counts say none.
    pub fn detach(&mut self, xref: impl XrefKey) -> usize {
        self.settle();
        let Some(id) = xref.id_in(&self.store) else {
            return 0;
        };
        if self.references(id) == 0 {
            return 0;
        }
        let mut removed = Vec::new();
        let mut keep = |p: XrefId| p != id;
        let mut gone = |p: XrefId| removed.push(p);
        let mut cut = Cut::new(&mut keep, &mut gone);
        if let Some(h) = &mut self.header {
            h.retain(&mut cut);
        }
        for kind in KINDS {
            for pos in 0..self.len_of(kind) {
                on_record_mut!(self, kind, pos, |r| {
                    let mut has = false;
                    r.each(&mut |p| has |= p == id);
                    if has {
                        r.retain(&mut cut);
                        r.cut_own(&mut cut);
                    }
                });
            }
        }
        let n = cut.count;
        self.index.count(-1, self.store.xref_count(), |c| {
            removed.into_iter().for_each(c);
        });
        n
    }

    // ---- Links ------------------------------------------------------------

    /// Adds a pointer of kind `link` from `holder` to the record with
    /// identifier `target`: a structure appended to its list (`FAMC`,
    /// `CHIL`, a citation, an extension structure, …), or set in its place
    /// (`HUSB`). Only this side: [`Dataset::link_child`] and
    /// [`Dataset::link_spouse`] link both a family and an individual.
    ///
    /// # Errors
    ///
    /// [`EditError::UnknownXref`] or [`EditError::WrongType`] when no
    /// record of the type the link points to has `target`,
    /// [`EditError::NotApplicable`] when `holder` has no such structure,
    /// [`EditError::AlreadyLinked`] when it already points there so,
    /// [`EditError::Occupied`] when its one structure of the kind points
    /// elsewhere, [`EditError::StaleHandle`], [`EditError::NoHeader`].
    pub fn link(
        &mut self,
        holder: impl Into<Holder>,
        link: Link,
        target: impl XrefKey,
    ) -> Result<(), EditError> {
        self.settle();
        let target = self.target(target, link.target())?;
        self.with_holder(holder.into(), |h| h.link(link, target))
    }

    /// Removes the pointers of kind `link` from `holder` to `target`, with
    /// their structures (and their substructures): the number removed.
    ///
    /// # Errors
    ///
    /// [`EditError::NotLinked`] when there is none,
    /// [`EditError::NotApplicable`], [`EditError::StaleHandle`],
    /// [`EditError::NoHeader`].
    pub fn unlink(
        &mut self,
        holder: impl Into<Holder>,
        link: Link,
        target: impl XrefKey,
    ) -> Result<usize, EditError> {
        self.settle();
        let Some(target) = target.id_in(&self.store) else {
            return Err(EditError::NotLinked(link));
        };
        match self.with_holder(holder.into(), |h| h.unlink(link, target))? {
            0 => Err(EditError::NotLinked(link)),
            n => Ok(n),
        }
    }

    /// Links a child and a family both ways: the family's `CHIL` and the
    /// individual's `FAMC`. Either side already there is kept.
    ///
    /// # Errors
    ///
    /// [`EditError::AlreadyLinked`] when both sides are,
    /// [`EditError::StaleHandle`], [`EditError::NoXref`] when either
    /// record has no identifier.
    ///
    /// ```rust
    /// use ged_io::model::{Dataset, Family, Individual};
    /// use ged_io::GedcomVersion;
    ///
    /// let mut data = Dataset::new(GedcomVersion::V7_0);
    /// let family = data.add(Family::default()).unwrap();
    /// let ann = data.add(Individual::default()).unwrap();
    /// data.link_child(family, ann).unwrap();
    /// let out = ged_io::GedcomWriter::new().write_to_string(&data).unwrap();
    /// assert!(out.contains("0 @F1@ FAM\n1 CHIL @I1@\n"));
    /// assert!(out.contains("0 @I1@ INDI\n1 FAMC @F1@\n"));
    /// ```
    pub fn link_child(
        &mut self,
        family: Handle<Family>,
        child: Handle<Individual>,
    ) -> Result<(), EditError> {
        self.link_both(family, child, &[Link::Child], Link::ChildOf)
    }

    /// Links a spouse and a family both ways: the family's `HUSB` (or
    /// `WIFE`, when `HUSB` is taken) and the individual's `FAMS`.
    ///
    /// # Errors
    ///
    /// [`EditError::Occupied`] when both partners are other individuals,
    /// [`EditError::AlreadyLinked`] when both sides are linked, as
    /// [`Dataset::link_child`].
    pub fn link_spouse(
        &mut self,
        family: Handle<Family>,
        spouse: Handle<Individual>,
    ) -> Result<(), EditError> {
        self.link_both(family, spouse, &[Link::Husband, Link::Wife], Link::SpouseOf)
    }

    /// Removes the links of [`Dataset::link_child`], both ways: the number
    /// of structures removed.
    ///
    /// # Errors
    ///
    /// [`EditError::NotLinked`] when neither side is linked.
    pub fn unlink_child(
        &mut self,
        family: Handle<Family>,
        child: Handle<Individual>,
    ) -> Result<usize, EditError> {
        self.unlink_both(family, child, &[Link::Child], Link::ChildOf)
    }

    /// Removes the links of [`Dataset::link_spouse`], both ways: the
    /// number of structures removed.
    ///
    /// # Errors
    ///
    /// [`EditError::NotLinked`] when neither side is linked.
    pub fn unlink_spouse(
        &mut self,
        family: Handle<Family>,
        spouse: Handle<Individual>,
    ) -> Result<usize, EditError> {
        self.unlink_both(family, spouse, &[Link::Husband, Link::Wife], Link::SpouseOf)
    }

    fn ids_of(
        &self,
        family: Handle<Family>,
        individual: Handle<Individual>,
    ) -> Result<(XrefId, XrefId), EditError> {
        let f = self.get(family).ok_or(EditError::StaleHandle)?.xref;
        let i = self.get(individual).ok_or(EditError::StaleHandle)?.xref;
        f.zip(i).ok_or(EditError::NoXref)
    }

    fn link_both(
        &mut self,
        family: Handle<Family>,
        individual: Handle<Individual>,
        family_side: &[Link],
        individual_side: Link,
    ) -> Result<(), EditError> {
        self.settle();
        let (f, i) = self.ids_of(family, individual)?;
        let fam = self.get(family).ok_or(EditError::StaleHandle)?;
        let linked = |l: Link| match l {
            Link::Husband => fam.husband_id() == Some(i),
            Link::Wife => fam.wife_id() == Some(i),
            _ => fam.children.iter().any(|c| c.individual == Some(i)),
        };
        let family_done = family_side.iter().any(|&l| linked(l));
        let free = family_side.iter().copied().find(|&l| match l {
            Link::Husband => fam.husband_id().is_none(),
            Link::Wife => fam.wife_id().is_none(),
            _ => true,
        });
        let indi = self.get(individual).ok_or(EditError::StaleHandle)?;
        let individual_done = match individual_side {
            Link::ChildOf => indi.child_of.iter().any(|c| c.family == Some(f)),
            _ => indi.spouse_of.iter().any(|c| c.family == Some(f)),
        };
        if family_done && individual_done {
            return Err(EditError::AlreadyLinked(individual_side));
        }
        let family_link = match (family_done, free) {
            (true, _) => None,
            (false, Some(l)) => Some(l),
            (false, None) => return Err(EditError::Occupied(family_side[0])),
        };
        if let Some(l) = family_link {
            self.with_holder(family.into(), |h| h.link(l, i))?;
        }
        if !individual_done {
            self.with_holder(individual.into(), |h| h.link(individual_side, f))?;
        }
        Ok(())
    }

    fn unlink_both(
        &mut self,
        family: Handle<Family>,
        individual: Handle<Individual>,
        family_side: &[Link],
        individual_side: Link,
    ) -> Result<usize, EditError> {
        self.settle();
        let (f, i) = self.ids_of(family, individual)?;
        let mut n = 0;
        for &l in family_side {
            n += self.with_holder(family.into(), |h| h.unlink(l, i))?;
        }
        n += self.with_holder(individual.into(), |h| h.unlink(individual_side, f))?;
        if n == 0 {
            return Err(EditError::NotLinked(individual_side));
        }
        Ok(n)
    }

    /// Runs `edit` on the holder lent, its pointers counted again after.
    fn with_holder<R>(
        &mut self,
        holder: Holder,
        edit: impl FnOnce(HolderMut<'_>) -> Result<R, EditError>,
    ) -> Result<R, EditError> {
        let (at, kind, pos) = match holder {
            Holder::Header => {
                if self.header.is_none() {
                    return Err(EditError::NoHeader);
                }
                (Lent::Header, None, 0)
            }
            Holder::Record(h) => {
                let (kind, pos) = self
                    .handle_table()
                    .locate(h)
                    .ok_or(EditError::StaleHandle)?;
                (Lent::Record(kind, pos), Some(kind), pos)
            }
        };
        self.lend(at);
        let result = match kind {
            None => match self.header.as_mut() {
                Some(h) => edit(HolderMut::Header(h)),
                None => Err(EditError::NoHeader),
            },
            Some(kind) => match kind {
                RecordKind::Submitter => self.submitters.get_mut(pos).map(HolderMut::Submitter),
                RecordKind::Submission => self.submissions.get_mut(pos).map(HolderMut::Submission),
                RecordKind::Individual => self.individuals.get_mut(pos).map(HolderMut::Individual),
                RecordKind::Family => self.families.get_mut(pos).map(HolderMut::Family),
                RecordKind::Note => self.notes.get_mut(pos).map(HolderMut::Note),
                RecordKind::Source => self.sources.get_mut(pos).map(HolderMut::Source),
                RecordKind::Repository => self.repositories.get_mut(pos).map(HolderMut::Repository),
                RecordKind::Multimedia => self.multimedia.get_mut(pos).map(HolderMut::Multimedia),
                RecordKind::Other => self.extra.get_mut(pos).map(HolderMut::Other),
            }
            .map_or(Err(EditError::StaleHandle), edit),
        };
        self.settle();
        result
    }
}

impl RecordRef<'_> {
    /// Calls `f` with every pointer of the record.
    pub(crate) fn each_pointer(&self, f: &mut dyn FnMut(XrefId)) {
        match self {
            RecordRef::Header(r) => r.each(f),
            RecordRef::Individual(r) => r.each(f),
            RecordRef::Family(r) => r.each(f),
            RecordRef::Source(r) => r.each(f),
            RecordRef::Repository(r) => r.each(f),
            RecordRef::Multimedia(r) => r.each(f),
            RecordRef::Submitter(r) => r.each(f),
            RecordRef::Submission(r) => r.each(f),
            RecordRef::Note(r) => r.each(f),
            RecordRef::Other(r) => r.each(f),
        }
    }
}

/// A holder lent to [`Dataset::link`] and [`Dataset::unlink`].
enum HolderMut<'a> {
    Header(&'a mut Header),
    Submitter(&'a mut Submitter),
    Submission(&'a mut Submission),
    Individual(&'a mut Individual),
    Family(&'a mut Family),
    Note(&'a mut SharedNote),
    Source(&'a mut Source),
    Repository(&'a mut Repository),
    Multimedia(&'a mut Multimedia),
    Other(&'a mut Node),
}

/// A list of structures each pointing somewhere: links added at its end,
/// removed by their pointer.
trait PointerList {
    type Item: Pointers;
    fn items(&self) -> &[Self::Item];
    fn add(&mut self, item: Self::Item);
    fn cut(&mut self, target: XrefId) -> usize;
}

impl<T: Pointers> PointerList for Vec<T> {
    type Item = T;
    fn items(&self) -> &[T] {
        self
    }
    fn add(&mut self, item: T) {
        self.push(item);
    }
    fn cut(&mut self, target: XrefId) -> usize {
        let before = self.len();
        self.retain(|i| i.own() != Some(target));
        before - self.len()
    }
}

impl<T: Pointers> PointerList for ThinVec<T> {
    type Item = T;
    fn items(&self) -> &[T] {
        self
    }
    fn add(&mut self, item: T) {
        self.push(item);
    }
    fn cut(&mut self, target: XrefId) -> usize {
        let before = self.len();
        self.retain(|i| i.own() != Some(target));
        before - self.len()
    }
}

/// Adds `item` (pointing to `target`) to `list`, unless an item points
/// there already.
fn add_to<L: PointerList>(
    list: &mut L,
    link: Link,
    target: XrefId,
    item: L::Item,
) -> Result<(), EditError> {
    if list.items().iter().any(|i| i.own() == Some(target)) {
        return Err(EditError::AlreadyLinked(link));
    }
    list.add(item);
    Ok(())
}

/// Sets `slot` to `item` (pointing to `target`), unless it points
/// elsewhere.
fn set_in<T: Pointers>(
    slot: &mut Option<T>,
    link: Link,
    target: XrefId,
    item: T,
) -> Result<(), EditError> {
    match slot.as_ref().and_then(Pointers::own) {
        Some(p) if p == target => Err(EditError::AlreadyLinked(link)),
        Some(_) => Err(EditError::Occupied(link)),
        None => {
            *slot = Some(item);
            Ok(())
        }
    }
}

fn cut_in<T: Pointers>(slot: &mut Option<T>, target: XrefId) -> usize {
    if slot.as_ref().and_then(Pointers::own) == Some(target) {
        *slot = None;
        1
    } else {
        0
    }
}

fn citation(source: XrefId) -> Citation {
    Citation {
        source: CitationSource::Pointer(source),
        ..Citation::default()
    }
}

fn multimedia_link(object: XrefId) -> MultimediaLink {
    MultimediaLink {
        object: Some(object),
        ..MultimediaLink::default()
    }
}

fn association(individual: XrefId) -> Association {
    Association {
        individual: Some(individual),
        ..Association::default()
    }
}

fn repository(repository: XrefId) -> RepositoryCitation {
    RepositoryCitation {
        repository: Some(repository),
        ..RepositoryCitation::default()
    }
}

/// An extension structure `tag @target@`.
fn extension(tag: TagId, target: XrefId) -> Node {
    Node {
        payload: Value::Pointer(target),
        ..Node::new(tag)
    }
}

/// The extension structures of a holder of tag `tag` pointing to `target`.
fn is_extension(n: &Node, tag: TagId, target: XrefId) -> bool {
    n.tag == tag && n.payload == Value::Pointer(target)
}

/// A field of a holder that holds pointers of one kind of link.
trait FieldMut {
    /// Adds a pointer to `target`.
    fn link(&mut self, link: Link, target: XrefId) -> Result<(), EditError>;
    /// Removes the pointers to `target`: how many.
    fn cut(&mut self, target: XrefId) -> usize;
}

/// A list of structures, each made by the function from its pointer.
struct Many<'a, L: PointerList>(&'a mut L, fn(XrefId) -> L::Item);

impl<L: PointerList> FieldMut for Many<'_, L> {
    fn link(&mut self, link: Link, target: XrefId) -> Result<(), EditError> {
        add_to(self.0, link, target, (self.1)(target))
    }
    fn cut(&mut self, target: XrefId) -> usize {
        self.0.cut(target)
    }
}

/// A structure that occurs once.
struct One<'a, T: Pointers>(&'a mut Option<T>, fn(XrefId) -> T);

impl<T: Pointers> FieldMut for One<'_, T> {
    fn link(&mut self, link: Link, target: XrefId) -> Result<(), EditError> {
        set_in(self.0, link, target, (self.1)(target))
    }
    fn cut(&mut self, target: XrefId) -> usize {
        cut_in(self.0, target)
    }
}

/// Extension structures of a tag.
struct Ext<'a>(&'a mut dyn ExtensionList, TagId);

impl FieldMut for Ext<'_> {
    fn link(&mut self, link: Link, target: XrefId) -> Result<(), EditError> {
        if self
            .0
            .nodes()
            .iter()
            .any(|n| is_extension(n, self.1, target))
        {
            return Err(EditError::AlreadyLinked(link));
        }
        self.0.push_node(extension(self.1, target));
        Ok(())
    }
    fn cut(&mut self, target: XrefId) -> usize {
        let tag = self.1;
        self.0.cut_nodes(&mut |n| is_extension(n, tag, target))
    }
}

/// A field of a detail that is not allocated: empty.
struct Absent;

impl FieldMut for Absent {
    fn link(&mut self, link: Link, _target: XrefId) -> Result<(), EditError> {
        Err(EditError::NotApplicable(link))
    }
    fn cut(&mut self, _target: XrefId) -> usize {
        0
    }
}

fn same(id: XrefId) -> XrefId {
    id
}

impl HolderMut<'_> {
    /// Runs `f` on the holder's field for `link`; `None` when the holder
    /// has none. Without `create`, a detail not allocated is not: its
    /// fields are empty.
    fn with_field<R>(
        self,
        link: Link,
        create: bool,
        f: impl FnOnce(&mut dyn FieldMut) -> R,
    ) -> Option<R> {
        use HolderMut as H;
        use Link as L;
        macro_rules! many {
            ($list:expr, $make:expr) => {
                Some(f(&mut Many($list, $make)))
            };
        }
        macro_rules! one {
            ($slot:expr, $make:expr) => {
                Some(f(&mut One($slot, $make)))
            };
        }
        macro_rules! detail {
            ($r:expr, |$d:ident| $list:expr, $make:expr) => {
                if create {
                    let $d = $r.detail_mut();
                    many!($list, $make)
                } else if let Some($d) = $r.detail.as_deref_mut() {
                    many!($list, $make)
                } else {
                    Some(f(&mut Absent))
                }
            };
        }
        match (self, link) {
            (h, L::Extension(tag)) => {
                let list: &mut dyn ExtensionList = match h {
                    H::Header(r) => &mut r.extra,
                    H::Submitter(r) => &mut r.extra,
                    H::Submission(r) => &mut r.extra,
                    H::Individual(r) => &mut r.extra,
                    H::Family(r) => &mut r.extra,
                    H::Note(r) => &mut r.extra,
                    H::Source(r) => &mut r.extra,
                    H::Repository(r) => &mut r.extra,
                    H::Multimedia(r) => &mut r.extra,
                    H::Other(r) => &mut r.children,
                };
                Some(f(&mut Ext(list, tag)))
            }
            (H::Individual(r), L::ChildOf) => many!(&mut r.child_of, ChildLink::new),
            (H::Individual(r), L::SpouseOf) => many!(&mut r.spouse_of, SpouseLink::new),
            (H::Individual(r), L::Citation) => many!(&mut r.citations, citation),
            (H::Individual(r), L::Note) => many!(&mut r.notes, Note::shared),
            (H::Individual(r), L::Alias) => detail!(r, |d| &mut d.aliases, IndividualRef::new),
            (H::Individual(r), L::Association) => {
                detail!(r, |d| &mut d.associations, association)
            }
            (H::Individual(r), L::Multimedia) => {
                detail!(r, |d| &mut d.multimedia, multimedia_link)
            }
            (H::Individual(r), L::Submitter) => detail!(r, |d| &mut d.submitters, same),
            (H::Individual(r), L::AncestorInterest) => {
                detail!(r, |d| &mut d.ancestor_interests, same)
            }
            (H::Individual(r), L::DescendantInterest) => {
                detail!(r, |d| &mut d.descendant_interests, same)
            }
            (H::Family(r), L::Husband) => one!(&mut r.husband, IndividualRef::new),
            (H::Family(r), L::Wife) => one!(&mut r.wife, IndividualRef::new),
            (H::Family(r), L::Child) => many!(&mut r.children, IndividualRef::new),
            (H::Family(r), L::Citation) => many!(&mut r.citations, citation),
            (H::Family(r), L::Note) => many!(&mut r.notes, Note::shared),
            (H::Family(r), L::Association) => detail!(r, |d| &mut d.associations, association),
            (H::Family(r), L::Multimedia) => detail!(r, |d| &mut d.multimedia, multimedia_link),
            (H::Family(r), L::Submitter) => detail!(r, |d| &mut d.submitters, same),
            (H::Source(r), L::Repository) => many!(&mut r.repositories, repository),
            (H::Source(r), L::Note) => many!(&mut r.notes, Note::shared),
            (H::Source(r), L::Multimedia) => many!(&mut r.multimedia, multimedia_link),
            (H::Repository(r), L::Note) => many!(&mut r.notes, Note::shared),
            (H::Multimedia(r), L::Note) => many!(&mut r.notes, Note::shared),
            (H::Multimedia(r), L::Citation) => many!(&mut r.citations, citation),
            (H::Submitter(r), L::Note) => many!(&mut r.notes, Note::shared),
            (H::Submitter(r), L::Multimedia) => many!(&mut r.multimedia, multimedia_link),
            (H::Submission(r), L::Submitter) => one!(&mut r.submitter, same),
            (H::Submission(r), L::Note) => many!(&mut r.notes, Note::shared),
            (H::Note(r), L::Citation) => many!(&mut r.citations, citation),
            (H::Header(r), L::Submitter) => one!(&mut r.submitter, same),
            (H::Header(r), L::Submission) => one!(&mut r.submission, same),
            (H::Header(r), L::Note) => many!(&mut r.notes, Note::shared),
            _ => None,
        }
    }

    fn link(self, link: Link, target: XrefId) -> Result<(), EditError> {
        self.with_field(link, true, |f| f.link(link, target))
            .unwrap_or(Err(EditError::NotApplicable(link)))
    }

    fn unlink(self, link: Link, target: XrefId) -> Result<usize, EditError> {
        self.with_field(link, false, |f| f.cut(target))
            .ok_or(EditError::NotApplicable(link))
    }
}

/// The untyped substructures of a holder: a structure's `extra`, an
/// untyped record's children.
trait ExtensionList {
    fn nodes(&self) -> &[Node];
    fn push_node(&mut self, node: Node);
    fn cut_nodes(&mut self, cut: &mut dyn FnMut(&Node) -> bool) -> usize;
}

impl ExtensionList for super::Extra {
    fn nodes(&self) -> &[Node] {
        self
    }
    fn push_node(&mut self, node: Node) {
        self.push(node);
    }
    fn cut_nodes(&mut self, cut: &mut dyn FnMut(&Node) -> bool) -> usize {
        let before = self.len();
        (**self).retain(|n| !cut(n));
        before - self.len()
    }
}

impl ExtensionList for Vec<Node> {
    fn nodes(&self) -> &[Node] {
        self
    }
    fn push_node(&mut self, node: Node) {
        self.push(node);
    }
    fn cut_nodes(&mut self, cut: &mut dyn FnMut(&Node) -> bool) -> usize {
        let before = self.len();
        self.retain(|n| !cut(n));
        before - self.len()
    }
}
