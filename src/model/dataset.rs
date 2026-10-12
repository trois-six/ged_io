//! A dataset: the header and the records of a file, typed, and the store
//! their texts point into.

use crate::tree::{Flat, Node as _, PayloadRef, Structure, Tag};
use crate::version::GedcomVersion;

use super::driver::{StdTag, ToNodes, WriteCx};
use super::header::Header;
use super::node::{Extra, Node, Value};
use super::record::{
    Family, Individual, Multimedia, Repository, SharedNote, Source, Submission, Submitter,
};
use super::text::{Store, XrefId};

/// A GEDCOM dataset: its header and records, typed, and the store their
/// texts and identifiers resolve against.
///
/// Records are kept by type, each type in file order
/// ([`Dataset::individuals`], [`Dataset::families`], …); records of no type
/// the model has (extension and unknown records, a second header, a
/// trailer with content) are kept in [`Dataset::extra`], in order. The
/// trailer itself carries nothing and is written by the writer.
///
/// A dataset keeps its references whole as it is edited: the records are
/// read through it and changed through it ([`Dataset::add`],
/// [`Dataset::remove`], [`Dataset::rename`], [`Dataset::link`],
/// [`Dataset::get_mut`], …), which knows the record of each identifier and
/// counts the pointers to it ([`Dataset::references`]). Lookups by
/// identifier take constant time. See [`crate::model`] (Editing).
///
/// ```rust
/// use ged_io::model::{Dataset, RecordRef};
///
/// let data = Dataset::parse("0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n0 @F1@ FAM\n1 CHIL @I1@\n0 _LOC Place\n0 TRLR\n");
/// assert_eq!(data.individuals().len(), 1);
/// assert_eq!(data.families()[0].children.len(), 1);
/// assert_eq!(data.extra().len(), 1); // the extension record
/// assert!(matches!(data.find("@F1@"), Some(RecordRef::Family(_))));
/// assert_eq!(data.records().count(), 4); // the header, two records, the extension
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dataset {
    /// The header (`HEAD`), when the file has one.
    pub(crate) header: Option<Header>,
    /// The individuals (`INDI`).
    pub(crate) individuals: Vec<Individual>,
    /// The families (`FAM`).
    pub(crate) families: Vec<Family>,
    /// The sources (`SOUR`).
    pub(crate) sources: Vec<Source>,
    /// The repositories (`REPO`).
    pub(crate) repositories: Vec<Repository>,
    /// The multimedia objects (`OBJE`).
    pub(crate) multimedia: Vec<Multimedia>,
    /// The submitters (`SUBM`).
    pub(crate) submitters: Vec<Submitter>,
    /// The submissions (5.5.1 `SUBN`).
    pub(crate) submissions: Vec<Submission>,
    /// The shared notes (5.5.1 `NOTE`, 7.x `SNOTE`).
    pub(crate) notes: Vec<SharedNote>,
    /// The records of no type the model has, in order.
    pub(crate) extra: Extra,
    pub(crate) store: Store,
    pub(crate) version: GedcomVersion,
    pub(crate) declared_version: Option<Box<str>>,
    /// Handles, identifiers and pointer counts (see [`super::edit`]).
    pub(crate) index: super::edit::Index,
}

impl AsRef<Store> for Dataset {
    fn as_ref(&self) -> &Store {
        &self.store
    }
}

/// A record of any type, owned: what a
/// [`GedcomStreamParser`](crate::GedcomStreamParser) yields, and what
/// [`Dataset::push`] takes. Its variants are boxed, so that it is two
/// words.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum Record {
    /// The header.
    Header(Box<Header>),
    /// An individual.
    Individual(Box<Individual>),
    /// A family.
    Family(Box<Family>),
    /// A source.
    Source(Box<Source>),
    /// A repository.
    Repository(Box<Repository>),
    /// A multimedia object.
    Multimedia(Box<Multimedia>),
    /// A submitter.
    Submitter(Box<Submitter>),
    /// A submission.
    Submission(Box<Submission>),
    /// A shared note.
    Note(Box<SharedNote>),
    /// A record of no type the model has, or one that did not fit its type.
    Other(Box<Node>),
}

impl Record {
    /// The record, borrowed.
    #[must_use]
    pub fn record_ref(&self) -> RecordRef<'_> {
        match self {
            Record::Header(r) => RecordRef::Header(r),
            Record::Individual(r) => RecordRef::Individual(r),
            Record::Family(r) => RecordRef::Family(r),
            Record::Source(r) => RecordRef::Source(r),
            Record::Repository(r) => RecordRef::Repository(r),
            Record::Multimedia(r) => RecordRef::Multimedia(r),
            Record::Submitter(r) => RecordRef::Submitter(r),
            Record::Submission(r) => RecordRef::Submission(r),
            Record::Note(r) => RecordRef::Note(r),
            Record::Other(n) => RecordRef::Other(n),
        }
    }
}

/// A record of a [`Dataset`], whatever its type.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
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

    /// The record's tag: the version's tag of its type (5.5.1 `NOTE`, 7.x
    /// `SNOTE` for a shared note), or an untyped record's own.
    #[must_use]
    pub fn tag<S: AsRef<Store> + ?Sized>(&self, store: &'a S, version: GedcomVersion) -> &'a str {
        match self {
            RecordRef::Other(n) => store.as_ref().tag(n.tag),
            RecordRef::Note(_) if version.is_v7() => "SNOTE",
            RecordRef::Note(_) => "NOTE",
            r => r.std_tag(),
        }
    }

    /// The index among the standard tags of the record's tag
    /// ([`RecordRef::tag`]).
    pub(crate) fn standard_tag(&self, version: GedcomVersion) -> Option<u16> {
        use crate::tree::standard_index_const as index;
        match self {
            RecordRef::Other(n) => u16::try_from(n.tag.get())
                .ok()
                .filter(|&t| usize::from(t) < crate::tree::STANDARD_TAGS.len()),
            RecordRef::Note(_) if version.is_v7() => const { index("SNOTE") },
            RecordRef::Note(_) => const { index("NOTE") },
            RecordRef::Header(_) => const { index("HEAD") },
            RecordRef::Individual(_) => const { index("INDI") },
            RecordRef::Family(_) => const { index("FAM") },
            RecordRef::Source(_) => const { index("SOUR") },
            RecordRef::Repository(_) => const { index("REPO") },
            RecordRef::Multimedia(_) => const { index("OBJE") },
            RecordRef::Submitter(_) => const { index("SUBM") },
            RecordRef::Submission(_) => const { index("SUBN") },
        }
    }

    /// The tag of a typed record (`SNOTE` for a shared note).
    fn std_tag(&self) -> &'static str {
        match self {
            RecordRef::Header(_) => "HEAD",
            RecordRef::Individual(_) => "INDI",
            RecordRef::Family(_) => "FAM",
            RecordRef::Source(_) => "SOUR",
            RecordRef::Repository(_) => "REPO",
            RecordRef::Multimedia(_) => "OBJE",
            RecordRef::Submitter(_) => "SUBM",
            RecordRef::Submission(_) => "SUBN",
            RecordRef::Note(_) | RecordRef::Other(_) => "SNOTE",
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

    /// The record as an owned structure, as the dataset holds it: values
    /// as written, enumeration values in `version`'s spelling.
    #[must_use]
    pub fn to_structure_in<S: AsRef<Store> + ?Sized>(
        self,
        store: &S,
        version: GedcomVersion,
    ) -> Structure {
        self.to_structure(&WriteCx {
            store: store.as_ref(),
            version,
            convert: false,
        })
    }
}

/// An identifier to look a record up by: its text (`"@I1@"`) or its
/// [`XrefId`].
pub trait XrefKey: sealed::Sealed {
    /// The id of the identifier in `store`, if it has one.
    fn id_in(self, store: &Store) -> Option<XrefId>;
}

pub(crate) mod sealed {
    use super::{Store, XrefId};

    /// The identifier's text, for errors.
    pub trait Sealed {
        fn text_in(&self, store: &Store) -> Box<str>;
    }
    impl Sealed for &str {
        fn text_in(&self, _store: &Store) -> Box<str> {
            (*self).into()
        }
    }
    impl Sealed for String {
        fn text_in(&self, _store: &Store) -> Box<str> {
            self.as_str().into()
        }
    }
    impl Sealed for &String {
        fn text_in(&self, _store: &Store) -> Box<str> {
            self.as_str().into()
        }
    }
    impl Sealed for XrefId {
        fn text_in(&self, store: &Store) -> Box<str> {
            store.xref(*self).into()
        }
    }
    impl Sealed for Option<XrefId> {
        fn text_in(&self, store: &Store) -> Box<str> {
            self.map_or_else(Box::default, |id| store.xref(id).into())
        }
    }
}

impl XrefKey for &str {
    fn id_in(self, store: &Store) -> Option<XrefId> {
        store.find_xref(self)
    }
}

impl XrefKey for String {
    fn id_in(self, store: &Store) -> Option<XrefId> {
        store.find_xref(&self)
    }
}

impl XrefKey for &String {
    fn id_in(self, store: &Store) -> Option<XrefId> {
        store.find_xref(self)
    }
}

impl XrefKey for XrefId {
    fn id_in(self, _store: &Store) -> Option<XrefId> {
        Some(self)
    }
}

impl XrefKey for Option<XrefId> {
    fn id_in(self, _store: &Store) -> Option<XrefId> {
        self
    }
}

/// A pointer of a dataset to no record of it: see
/// [`Dataset::dangling_references`].
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DanglingReference {
    /// The identifier of the record that holds the pointer; `None` for the
    /// header and records without one.
    pub record: Option<XrefId>,
    /// The tag of the structure whose payload the pointer is.
    pub tag: Tag,
    /// The pointer.
    pub pointer: XrefId,
}

impl Dataset {
    /// An empty dataset of `version`, to fill.
    #[must_use]
    pub fn new(version: GedcomVersion) -> Self {
        Self {
            version,
            ..Self::default()
        }
    }

    /// Reads a dataset from decoded text, leniently: never fails, keeps
    /// everything. The dataset keeps the text as its store (a `String` is
    /// not copied). [`GedcomBuilder`](crate::GedcomBuilder) reads with
    /// limits or in strict mode.
    #[must_use]
    pub fn parse(text: impl Into<String>) -> Self {
        super::read::read_string(text.into())
    }

    /// Reads a dataset from bytes of any encoding (see
    /// [`crate::encoding::decode`]), leniently. Given a `Vec<u8>` of UTF-8,
    /// the dataset keeps that buffer as its text, without a copy.
    #[must_use]
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self::parse(crate::encoding::decode_owned(bytes.into()).text)
    }

    /// The version the file declares (`HEAD.GEDC.VERS`): 7.0 or 7.1 for
    /// 7.x, 5.5.1 otherwise; for a dataset made with [`Dataset::new`], its
    /// version. The writer writes it unless told otherwise.
    #[must_use]
    pub fn version(&self) -> GedcomVersion {
        self.version
    }

    /// The `HEAD.GEDC.VERS` payload as read.
    #[must_use]
    pub fn declared_version(&self) -> Option<&str> {
        self.declared_version.as_deref()
    }

    /// The store the dataset's texts and identifiers resolve against.
    #[must_use]
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// The store, to intern the identifiers and tags of new structures.
    pub fn store_mut(&mut self) -> &mut Store {
        &mut self.store
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

    /// The number of records, the header included.
    #[must_use]
    pub fn record_count(&self) -> usize {
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

    /// The record with this identifier: the first one, when several have
    /// it. Constant time.
    #[must_use]
    pub fn find(&self, xref: impl XrefKey) -> Option<RecordRef<'_>> {
        self.find_record(xref.id_in(&self.store)?)
    }

    /// The individual with this identifier (the first, when several have
    /// it). Constant time, as every `find_*`.
    #[must_use]
    pub fn find_individual(&self, xref: impl XrefKey) -> Option<&Individual> {
        let pos = self.first_of(super::RecordKind::Individual, xref.id_in(&self.store)?)?;
        self.individuals.get(pos)
    }

    /// The family with this identifier.
    #[must_use]
    pub fn find_family(&self, xref: impl XrefKey) -> Option<&Family> {
        let pos = self.first_of(super::RecordKind::Family, xref.id_in(&self.store)?)?;
        self.families.get(pos)
    }

    /// The source with this identifier.
    #[must_use]
    pub fn find_source(&self, xref: impl XrefKey) -> Option<&Source> {
        let pos = self.first_of(super::RecordKind::Source, xref.id_in(&self.store)?)?;
        self.sources.get(pos)
    }

    /// The repository with this identifier.
    #[must_use]
    pub fn find_repository(&self, xref: impl XrefKey) -> Option<&Repository> {
        let pos = self.first_of(super::RecordKind::Repository, xref.id_in(&self.store)?)?;
        self.repositories.get(pos)
    }

    /// The multimedia object with this identifier.
    #[must_use]
    pub fn find_multimedia(&self, xref: impl XrefKey) -> Option<&Multimedia> {
        let pos = self.first_of(super::RecordKind::Multimedia, xref.id_in(&self.store)?)?;
        self.multimedia.get(pos)
    }

    /// The submitter with this identifier.
    #[must_use]
    pub fn find_submitter(&self, xref: impl XrefKey) -> Option<&Submitter> {
        let pos = self.first_of(super::RecordKind::Submitter, xref.id_in(&self.store)?)?;
        self.submitters.get(pos)
    }

    /// The shared note with this identifier.
    #[must_use]
    pub fn find_note(&self, xref: impl XrefKey) -> Option<&SharedNote> {
        let pos = self.first_of(super::RecordKind::Note, xref.id_in(&self.store)?)?;
        self.notes.get(pos)
    }

    /// The families in which this individual is a partner (`HUSB` or
    /// `WIFE`).
    pub fn families_as_spouse(&self, individual: impl XrefKey) -> impl Iterator<Item = &Family> {
        let id = individual.id_in(&self.store);
        self.families
            .iter()
            .filter(move |f| id.is_some() && (f.husband_id() == id || f.wife_id() == id))
    }

    /// The families in which this individual is a child (`CHIL`).
    pub fn families_as_child(&self, individual: impl XrefKey) -> impl Iterator<Item = &Family> {
        let id = individual.id_in(&self.store);
        self.families
            .iter()
            .filter(move |f| id.is_some() && f.children.iter().any(|c| c.individual == id))
    }

    /// The partners of a family (`HUSB`, then `WIFE`) that have a record.
    pub fn parents<'s>(&'s self, family: &'s Family) -> impl Iterator<Item = &'s Individual> {
        [family.husband_id(), family.wife_id()]
            .into_iter()
            .flatten()
            .filter_map(|id| self.find_individual(id))
    }

    /// The children of a family (`CHIL`) that have a record, in order.
    pub fn children<'s>(&'s self, family: &'s Family) -> impl Iterator<Item = &'s Individual> {
        family
            .children
            .iter()
            .filter_map(|c| c.individual)
            .filter_map(|id| self.find_individual(id))
    }

    /// The other partner of `individual` in `family`, when it has a record.
    #[must_use]
    pub fn spouse(&self, individual: impl XrefKey, family: &Family) -> Option<&Individual> {
        let id = individual.id_in(&self.store)?;
        let other = if family.husband_id() == Some(id) {
            family.wife_id()
        } else if family.wife_id() == Some(id) {
            family.husband_id()
        } else {
            None
        };
        other.and_then(|o| self.find_individual(o))
    }

    /// The individuals one of whose names contains `query`, ignoring case
    /// (the surname slashes of a name are ignored: `ann example` finds
    /// `Ann /Example/`).
    pub fn search_individuals<'s>(
        &'s self,
        query: &'s str,
    ) -> impl Iterator<Item = &'s Individual> + 's {
        let query = query.to_lowercase();
        self.individuals.iter().filter(move |i| {
            i.names
                .iter()
                .any(|n| n.full(&self.store).to_lowercase().contains(&query))
        })
    }

    /// Every pointer of the dataset to no record of it, in record order:
    /// pointers of every kind (families, individuals, sources,
    /// repositories, multimedia objects, notes, submitters, extensions'),
    /// but 7.x `@VOID@`, which points to nothing by definition, and the
    /// 5.5.1 pointers into a record (`@I1!2@`, `@!2@`) or to another
    /// dataset (`:`). A query, not a check: reading never fails on a
    /// dangling pointer.
    ///
    /// ```rust
    /// use ged_io::model::Dataset;
    ///
    /// let data = Dataset::parse("0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 FAMC @F9@\n1 SOUR @VOID@\n0 TRLR\n");
    /// let dangling = data.dangling_references();
    /// assert_eq!(dangling.len(), 1);
    /// assert_eq!(data.store().xref(dangling[0].pointer), "@F9@");
    /// assert_eq!(dangling[0].tag, "FAMC");
    /// ```
    #[must_use]
    pub fn dangling_references(&self) -> Vec<DanglingReference> {
        let mut defined = vec![false; self.store.xref_count()];
        for id in self.records().filter_map(|r| r.xref()) {
            if let Some(d) = defined.get_mut(id.index()) {
                *d = true;
            }
        }
        let family = crate::spec::payload::Family::of(self.version.rules());
        let cx = WriteCx {
            store: &self.store,
            version: self.version,
            convert: false,
        };
        let mut out = Vec::new();
        let mut flat = Flat::default();
        for record in self.records() {
            flat.clear();
            record.to_flat(&cx, &mut flat);
            let Some(root) = flat.root() else { continue };
            for (_, n) in root.preorder() {
                let PayloadRef::Pointer(p) = n.payload() else {
                    continue;
                };
                if (self.version.is_v7() && p == crate::tree::Xref::VOID)
                    || crate::spec::is_external_pointer(p, family)
                {
                    continue;
                }
                let Some(id) = self.store.find_xref(p) else {
                    continue;
                };
                if !defined.get(id.index()).copied().unwrap_or(false) {
                    out.push(DanglingReference {
                        record: record.xref(),
                        tag: Tag::new(n.tag()),
                        pointer: id,
                    });
                }
            }
        }
        out
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
}
