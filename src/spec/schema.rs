//! The types of the generated specification tables (`tables.rs`) and their
//! lookups.
//!
//! One [`Schema`] per GEDCOM version lists every structure type the
//! specification defines, numbered: [`StructDef`] gives a type's tag, its
//! payload type and the range of its permitted substructures in the flat
//! [`Schema::subs`] array, each a [`SubDef`] with its cardinality. Type 0 is
//! the dataset itself, whose substructures are the records and the header
//! and trailer. Tags are indices into the shared, sorted [`TAGS`] list, and
//! the type names of a version share one string, so a version's tables are
//! a few kilobytes of plain data.
//!
//! This file depends on nothing else in the crate: the test suite includes
//! it, with `tables.rs`, to read the same facts with its own checker.
//!
//! [`TAGS`]: super::tables::TAGS

use super::tables::TAGS;

/// The index of a structure type in [`Schema::structs`].
pub(crate) type StructId = u16;

/// The dataset: the pseudo-type whose substructures are the records.
pub(crate) const DATASET: StructId = 0;

/// The payload type of a structure type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum Kind {
    /// No payload.
    None,
    /// `Y` or nothing.
    Y,
    /// Any text.
    Text,
    /// A comma-separated list of texts.
    ListText,
    /// A non-negative integer.
    Int,
    /// A personal name: at most one pair of slashes around the surname.
    Name,
    /// A date value.
    Date,
    /// An exact date.
    DateExact,
    /// A date period.
    DatePeriod,
    /// An age.
    Age,
    /// A time.
    Time,
    /// A language tag (7.x: BCP 47).
    Lang,
    /// A media type.
    MediaType,
    /// A file path or URI reference.
    FilePath,
    /// An absolute URI.
    Uri,
    /// A latitude.
    Lat,
    /// A longitude.
    Long,
    /// An extension tag and its URI (`SCHMA.TAG`).
    TagDef,
    /// A pointer to a record of type `arg`.
    Pointer,
    /// A pointer to a record of type `arg`, or nothing (5.5.1 `REPO`).
    NullablePointer,
    /// A value of the enumeration set `arg`.
    Enum,
    /// A comma-separated list of values of the enumeration set `arg`.
    ListEnum,
}

/// Flag of a 7.1 structure type named under `https://gedcom.io/terms/v7.1/`.
pub(crate) const URI71: u8 = 1;

/// A structure type.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StructDef {
    name: u16,
    name_len: u8,
    tag: u8,
    kind: Kind,
    flags: u8,
    arg: u16,
    subs: u16,
    nsubs: u8,
}

/// A structure type: its name (`offset`, `len` into [`Schema::names`]), its
/// tag (index into `TAGS`), its payload `kind` and `arg`, its substructures
/// (`subs`, `nsubs` into [`Schema::subs`]) and its `flags`.
#[allow(clippy::too_many_arguments)] // One argument per field of a table row.
pub(crate) const fn d(
    name: u16,
    name_len: u8,
    tag: u8,
    kind: Kind,
    arg: u16,
    subs: u16,
    nsubs: u8,
    flags: u8,
) -> StructDef {
    StructDef {
        name,
        name_len,
        tag,
        kind,
        flags,
        arg,
        subs,
        nsubs,
    }
}

/// A permitted substructure: its type and how many times it may appear
/// (`max == 0`: no upper bound).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SubDef {
    pub(crate) id: StructId,
    pub(crate) min: u8,
    pub(crate) max: u8,
}

/// A substructure row.
pub(crate) const fn u(id: StructId, min: u8, max: u8) -> SubDef {
    SubDef { id, min, max }
}

/// An enumeration set. An open set also admits values of the user's own.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EnumSet {
    pub(crate) name: &'static str,
    pub(crate) values: &'static [&'static str],
    pub(crate) open: bool,
}

/// An enumeration row.
pub(crate) const fn e(name: &'static str, values: &'static [&'static str], open: bool) -> EnumSet {
    EnumSet { name, values, open }
}

/// A calendar with its month tags and epoch markers. The date grammars of
/// [`crate::types::date`] carry the same facts; a test compares them.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Calendar {
    pub(crate) tag: &'static str,
    pub(crate) months: &'static [&'static str],
    pub(crate) epochs: &'static [&'static str],
}

/// A calendar row.
#[cfg(test)]
pub(crate) const fn c(
    tag: &'static str,
    months: &'static [&'static str],
    epochs: &'static [&'static str],
) -> Calendar {
    Calendar {
        tag,
        months,
        epochs,
    }
}

/// The tables of one GEDCOM version.
#[derive(Debug)]
pub(crate) struct Schema {
    /// The version: `5.5.1`, `7.0` or `7.1`.
    pub(crate) version: &'static str,
    /// The type names, one after the other.
    pub(crate) names: &'static str,
    /// The structure types; [`DATASET`] first.
    pub(crate) structs: &'static [StructDef],
    /// The permitted substructures of every type, type after type.
    pub(crate) subs: &'static [SubDef],
    /// The enumeration sets.
    pub(crate) enums: &'static [EnumSet],
    /// The calendars.
    #[cfg(test)]
    pub(crate) calendars: &'static [Calendar],
}

/// The index of a tag in `TAGS`, if the tables know it.
pub(crate) fn tag_index(tag: &str) -> Option<u8> {
    TAGS.binary_search(&tag)
        .ok()
        .and_then(|i| u8::try_from(i).ok())
}

impl Schema {
    fn def(&self, id: StructId) -> Option<&StructDef> {
        self.structs.get(usize::from(id))
    }

    /// The name of a structure type: its URI without the prefix in 7.x
    /// (`INDI-NAME`), its context path in 5.5.1
    /// (`PERSONAL_NAME_STRUCTURE.NAME`); empty for the dataset.
    pub(crate) fn name(&self, id: StructId) -> &'static str {
        self.def(id).map_or("", |d| {
            let start = usize::from(d.name);
            self.names
                .get(start..start + usize::from(d.name_len))
                .unwrap_or("")
        })
    }

    /// The tag of a structure type; empty for the dataset.
    pub(crate) fn tag(&self, id: StructId) -> &'static str {
        match self.def(id) {
            Some(d) if id != DATASET => TAGS.get(usize::from(d.tag)).copied().unwrap_or(""),
            _ => "",
        }
    }

    /// The index of the tag of a structure type in `TAGS`.
    pub(crate) fn tag_id(&self, id: StructId) -> Option<u8> {
        self.def(id).filter(|_| id != DATASET).map(|d| d.tag)
    }

    /// The payload type of a structure type and its argument: the record
    /// type of a pointer, the enumeration set of an enumeration.
    pub(crate) fn kind(&self, id: StructId) -> (Kind, u16) {
        self.def(id).map_or((Kind::None, 0), |d| (d.kind, d.arg))
    }

    /// The URI of a 7.x structure type: `https://gedcom.io/terms/v7/` and
    /// its name, or `.../v7.1/` for a type 7.1 defines anew.
    pub(crate) fn uri(&self, id: StructId) -> String {
        let base = if self.def(id).is_some_and(|d| d.flags & URI71 != 0) {
            "https://gedcom.io/terms/v7.1/"
        } else {
            "https://gedcom.io/terms/v7/"
        };
        format!("{base}{}", self.name(id))
    }

    /// The permitted substructures of a structure type.
    pub(crate) fn subs(&self, id: StructId) -> &'static [SubDef] {
        self.def(id).map_or(&[], |d| {
            let start = usize::from(d.subs);
            self.subs
                .get(start..start + usize::from(d.nsubs))
                .unwrap_or(&[])
        })
    }

    /// The permitted substructures of `id` tagged `tag` (several in 5.5.1,
    /// where a pointer form and a text form share a tag). The generator
    /// sorts each type's substructures by tag, and tags are indices into
    /// the sorted `TAGS`, so they are found by a binary search.
    pub(crate) fn subs_tagged(&self, id: StructId, tag: u8) -> impl Iterator<Item = &SubDef> {
        let subs = self.subs(id);
        let start = subs.partition_point(|s| self.tag_id(s.id) < Some(tag));
        subs.get(start..)
            .unwrap_or(&[])
            .iter()
            .take_while(move |s| self.tag_id(s.id) == Some(tag))
    }

    /// The enumeration set `index`.
    pub(crate) fn enum_set(&self, index: u16) -> Option<&'static EnumSet> {
        self.enums.get(usize::from(index))
    }

    /// The type of the record tagged `tag`, if the version defines one.
    pub(crate) fn record(&self, tag: &str) -> Option<StructId> {
        let tag = tag_index(tag)?;
        self.subs_tagged(DATASET, tag).next().map(|s| s.id)
    }
}
