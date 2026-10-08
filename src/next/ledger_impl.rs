//! The declarations of the typed model, for the coverage ledger, and the
//! structures a dataset kept untyped although the model has a type for
//! them.

use super::dataset::Dataset;
use super::driver::{FieldDesc, Fields, SpecNames, Struct};
use super::enums::{LdsStatus, PHRASED_FIELDS};
use super::header::{
    Charset, Corporation, GedcomInfo, Header, HeaderPlace, HeaderSource, HeaderSourceData,
    HeaderText, Schema,
};
use super::records::{
    Family, Individual, Multimedia, RecordedEvents, Repository, SharedNote, Source, SourceData,
    Submission, Submitter,
};
use super::text::{Store, TagId};
use super::{
    Address, Age, Association, CallNumber, ChangeDate, ChildLink, Citation, CitationData,
    CitedEvent, CreationDate, Crop, Date, Event, EventFamily, EventSpouse, ExactDate, Exid, File,
    FileForm, FileTranslation, IndividualRef, Map, MultimediaLink, Name, NameTranslation, NonEvent,
    Note, NoteTranslation, Ordinance, Period, PhoneticName, PhoneticVariation, Place,
    PlaceTranslation, Refn, RepositoryCitation, RomanizedName, RomanizedVariation, SourceText,
    SpouseLink, TextTranslation,
};

/// The typed structures and the structure types each stands for.
macro_rules! structures {
    ($($ty:ty),* $(,)?; phrased: $($p:ident: [$($v70:literal),*] [$($v71:literal),*];)*) => {
        /// Every typed structure: its name, its structure types and its
        /// fields, for the coverage ledger. An enumeration value with its
        /// phrase ([`Phrased`](super::Phrased)) stands for the 7.x types
        /// listed with it.
        pub(crate) const STRUCTURES: &[(&str, SpecNames, &[FieldDesc])] = &[
            $((stringify!($ty), <$ty as Struct>::SPEC, <$ty as Struct>::FIELDS),)*
            $((
                concat!("Phrased<", stringify!($p), ">"),
                SpecNames { v551: &[], v70: &[$($v70),*], v71: &[$($v71),*] },
                PHRASED_FIELDS,
            ),)*
        ];
    };
}

structures!(
    Address,
    Age,
    Association,
    CallNumber,
    ChangeDate,
    Charset,
    ChildLink,
    Citation,
    CitationData,
    CitedEvent,
    Corporation,
    CreationDate,
    Crop,
    Date,
    Event,
    EventFamily,
    EventSpouse,
    ExactDate,
    Exid,
    Family,
    File,
    FileForm,
    FileTranslation,
    GedcomInfo,
    Header,
    HeaderPlace,
    HeaderSource,
    HeaderSourceData,
    HeaderText,
    Individual,
    IndividualRef,
    LdsStatus,
    Map,
    Multimedia,
    MultimediaLink,
    Name,
    NameTranslation,
    NonEvent,
    Note,
    NoteTranslation,
    Ordinance,
    Period,
    PhoneticName,
    PhoneticVariation,
    Place,
    PlaceTranslation,
    RecordedEvents,
    Refn,
    Repository,
    RepositoryCitation,
    RomanizedName,
    RomanizedVariation,
    Schema,
    SharedNote,
    Source,
    SourceData,
    SourceText,
    SpouseLink,
    Submission,
    Submitter,
    TextTranslation;
    phrased:
    Adoption: ["FAMC-ADOP"] ["FAMC-ADOP"];
    ChildStatus: ["FAMC-STAT"] ["FAMC-STAT"];
    Medium: ["MEDI"] ["MEDI"];
    NameType: ["NAME-TYPE"] ["NAME-TYPE"];
    Pedigree: ["PEDI"] ["PEDI"];
    Role: ["ROLE"] ["ROLE"];
    NoteKind: [] ["NOTE-KIND"];
);

/// The tags of the records the model types.
const RECORD_TAGS: &[&str] = &[
    "HEAD", "INDI", "FAM", "SOUR", "REPO", "OBJE", "SUBM", "SUBN", "NOTE", "SNOTE",
];

/// The paths (`INDI/BIRT/DATE`) of the structures of `data` kept untyped
/// in an `extra` although a field of their superstructure takes their tag
/// (they did not fit their type, or repeated a singleton), or with a
/// payload or identifier their type has no place for. A conformant file
/// has none, but for leaves (values without substructures) that carry
/// extension substructures, which stay whole. Only the first of a tag is
/// listed: the later ones follow it in `extra` to keep their order.
pub(crate) fn untyped(data: &Dataset) -> Vec<String> {
    let mut out = Vec::new();
    let mut path = Vec::new();
    for record in data.records() {
        let cx_tag = match record {
            super::RecordRef::Other(node) => {
                let tag = data.store.tag(node.tag);
                if RECORD_TAGS.contains(&tag) {
                    out.push(tag.to_string());
                }
                continue;
            }
            super::RecordRef::Header(r) => ("HEAD", r as &dyn Fields),
            super::RecordRef::Individual(r) => ("INDI", r as &dyn Fields),
            super::RecordRef::Family(r) => ("FAM", r as &dyn Fields),
            super::RecordRef::Source(r) => ("SOUR", r as &dyn Fields),
            super::RecordRef::Repository(r) => ("REPO", r as &dyn Fields),
            super::RecordRef::Multimedia(r) => ("OBJE", r as &dyn Fields),
            super::RecordRef::Submitter(r) => ("SUBM", r as &dyn Fields),
            super::RecordRef::Submission(r) => ("SUBN", r as &dyn Fields),
            super::RecordRef::Note(r) => ("SNOTE", r as &dyn Fields),
        };
        path.push(cx_tag.0);
        check(cx_tag.1, &data.store, &mut path, &mut out);
        path.pop();
    }
    out
}

fn check(s: &dyn Fields, store: &Store, path: &mut Vec<&'static str>, out: &mut Vec<String>) {
    let (lookup, fields) = (s.lookup(), s.fields());
    let mut seen = Vec::new();
    for node in s.extra() {
        // A payload or identifier the type had no place for.
        if node.tag == TagId::ASIDE {
            out.push(path.join("/"));
            continue;
        }
        let field = lookup.get(node.tag.get() as usize).copied().unwrap_or(0);
        let Some(desc) = usize::from(field)
            .checked_sub(1)
            .and_then(|f| fields.get(f))
        else {
            continue;
        };
        if seen.contains(&node.tag) {
            continue;
        }
        seen.push(node.tag);
        // A leaf has no `extra`: with substructures of its own, it is kept
        // whole where it is.
        if !desc.leaf || node.children.is_empty() {
            out.push(format!("{}/{}", path.join("/"), store.tag(node.tag)));
        }
    }
    s.walk(&mut |tag, child| {
        path.push(tag);
        check(child, store, path, out);
        path.pop();
    });
}
