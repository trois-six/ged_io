//! A dataset with an index: records found by identifier, and families
//! found from individuals, in constant time.
//!
//! [`Dataset`]'s lookups are linear searches, which suits a few lookups;
//! [`IndexedDataset`] indexes a dataset once (a few bytes per identifier
//! and per link) for many.
//!
//! ```rust
//! use ged_io::model::Dataset;
//! use ged_io::IndexedDataset;
//!
//! let data = Dataset::parse(
//!     "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 NAME Ann /Example/\n0 @I2@ INDI\n\
//!      0 @F1@ FAM\n1 WIFE @I1@\n1 CHIL @I2@\n0 TRLR\n",
//! );
//! let indexed = IndexedDataset::new(data);
//! let ann = indexed.find_individual("@I1@").unwrap();
//! let family = indexed.families_as_spouse(ann.xref).next().unwrap();
//! assert_eq!(indexed.children(family).count(), 1);
//! assert_eq!(indexed.individuals.len(), 2); // a dataset still
//! ```

use std::ops::{Deref, DerefMut};

use crate::model::{
    Dataset, Family, Individual, Multimedia, RecordRef, Repository, SharedNote, Source, Submitter,
    XrefId, XrefKey,
};

/// The type of the record an identifier names in the index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    None,
    Individual,
    Family,
    Source,
    Repository,
    Multimedia,
    Submitter,
    Submission,
    Note,
    Other,
}

/// The record an identifier names: its type and its position in its list.
#[derive(Clone, Copy, Debug)]
struct Slot {
    kind: Kind,
    at: u32,
}

const EMPTY: Slot = Slot {
    kind: Kind::None,
    at: 0,
};

/// Lists of positions per item, in one buffer: the families of each
/// individual.
#[derive(Clone, Debug, Default)]
struct Links {
    /// For item `i`, its positions are `items[starts[i]..starts[i + 1]]`.
    starts: Vec<u32>,
    items: Vec<u32>,
}

impl Links {
    /// Builds the lists from `(item, position)` pairs over `count` items.
    fn build(count: usize, pairs: &[(u32, u32)]) -> Self {
        let mut starts = vec![0_u32; count + 1];
        for &(item, _) in pairs {
            if let Some(s) = starts.get_mut(item as usize + 1) {
                *s += 1;
            }
        }
        for i in 1..starts.len() {
            starts[i] += starts[i - 1];
        }
        let mut items = vec![0_u32; pairs.len()];
        let mut fill = starts.clone();
        for &(item, at) in pairs {
            if let Some(f) = fill.get_mut(item as usize) {
                if let Some(slot) = items.get_mut(*f as usize) {
                    *slot = at;
                }
                *f += 1;
            }
        }
        Self { starts, items }
    }

    fn of(&self, item: usize) -> &[u32] {
        let start = self.starts.get(item).copied().unwrap_or(0) as usize;
        let end = self.starts.get(item + 1).copied().unwrap_or(0) as usize;
        self.items.get(start..end).unwrap_or_default()
    }
}

/// A [`Dataset`] and its index: see the [module documentation](self).
///
/// When several records have one identifier, the first one is found, as
/// [`Dataset::find`] finds it. The index is built by [`new`](Self::new)
/// and rebuilt after every edit through [`data_mut`](Self::data_mut); the
/// dataset is readable through `Deref`.
#[derive(Clone, Debug)]
pub struct IndexedDataset {
    data: Dataset,
    /// The record of each identifier, by [`XrefId`].
    records: Vec<Slot>,
    /// The families each identifier is a partner in, by [`XrefId`].
    as_spouse: Links,
    /// The families each identifier is a child in, by [`XrefId`].
    as_child: Links,
}

impl IndexedDataset {
    /// Indexes a dataset.
    #[must_use]
    pub fn new(data: Dataset) -> Self {
        let mut indexed = Self {
            data,
            records: Vec::new(),
            as_spouse: Links::default(),
            as_child: Links::default(),
        };
        indexed.reindex();
        indexed
    }

    fn reindex(&mut self) {
        let data = &self.data;
        let mut records = vec![EMPTY; data.store().xref_count()];
        let mut put = |id: Option<XrefId>, kind: Kind, at: usize| {
            let (Some(id), Ok(at)) = (id, u32::try_from(at)) else {
                return;
            };
            if let Some(slot) = records.get_mut(id.index()) {
                // The first record of an identifier is the one found.
                if slot.kind == Kind::None {
                    *slot = Slot { kind, at };
                }
            }
        };
        let lists: [(Kind, Vec<Option<XrefId>>); 9] = [
            (
                Kind::Individual,
                data.individuals.iter().map(|r| r.xref).collect(),
            ),
            (Kind::Family, data.families.iter().map(|r| r.xref).collect()),
            (Kind::Source, data.sources.iter().map(|r| r.xref).collect()),
            (
                Kind::Repository,
                data.repositories.iter().map(|r| r.xref).collect(),
            ),
            (
                Kind::Multimedia,
                data.multimedia.iter().map(|r| r.xref).collect(),
            ),
            (
                Kind::Submitter,
                data.submitters.iter().map(|r| r.xref).collect(),
            ),
            (
                Kind::Submission,
                data.submissions.iter().map(|r| r.xref).collect(),
            ),
            (Kind::Note, data.notes.iter().map(|r| r.xref).collect()),
            (Kind::Other, data.extra.iter().map(|r| r.xref).collect()),
        ];
        // In the order of `Dataset::records`, so that the first is the same.
        for kind in [
            Kind::Submitter,
            Kind::Submission,
            Kind::Individual,
            Kind::Family,
            Kind::Note,
            Kind::Source,
            Kind::Repository,
            Kind::Multimedia,
            Kind::Other,
        ] {
            if let Some((_, ids)) = lists.iter().find(|(k, _)| *k == kind) {
                for (at, id) in ids.iter().enumerate() {
                    put(*id, kind, at);
                }
            }
        }
        // The families each identifier is a partner and a child in, whether
        // or not a record has it (as the dataset's own navigation finds them).
        let mut spouses = Vec::new();
        let mut children = Vec::new();
        let index = |id: XrefId| u32::try_from(id.index()).ok();
        for (at, family) in data.families.iter().enumerate() {
            let Ok(at) = u32::try_from(at) else { break };
            for partner in [family.husband_id(), family.wife_id()]
                .into_iter()
                .flatten()
            {
                if let Some(i) = index(partner) {
                    spouses.push((i, at));
                }
            }
            for child in family.children.iter().filter_map(|c| c.individual) {
                if let Some(i) = index(child) {
                    children.push((i, at));
                }
            }
        }
        // One list entry per family, even when an individual is both
        // partners of it.
        spouses.dedup();
        let count = data.store().xref_count();
        self.as_spouse = Links::build(count, &spouses);
        self.as_child = Links::build(count, &children);
        self.records = records;
    }

    /// The dataset.
    #[must_use]
    pub fn data(&self) -> &Dataset {
        &self.data
    }

    /// The dataset, to edit: the index is rebuilt when the returned guard
    /// is dropped.
    pub fn data_mut(&mut self) -> DatasetMut<'_> {
        DatasetMut { indexed: self }
    }

    /// The dataset, without its index.
    #[must_use]
    pub fn into_inner(self) -> Dataset {
        self.data
    }

    fn slot(&self, xref: impl XrefKey) -> Option<Slot> {
        let id = xref.id_in(self.data.store())?;
        self.records
            .get(id.index())
            .copied()
            .filter(|s| s.kind != Kind::None)
    }

    fn at(&self, xref: impl XrefKey, kind: Kind) -> Option<usize> {
        self.slot(xref)
            .filter(|s| s.kind == kind)
            .map(|s| s.at as usize)
    }

    /// The record with this identifier.
    #[must_use]
    pub fn find(&self, xref: impl XrefKey) -> Option<RecordRef<'_>> {
        let slot = self.slot(xref)?;
        let at = slot.at as usize;
        let data = &self.data;
        Some(match slot.kind {
            Kind::None => return None,
            Kind::Individual => RecordRef::Individual(data.individuals.get(at)?),
            Kind::Family => RecordRef::Family(data.families.get(at)?),
            Kind::Source => RecordRef::Source(data.sources.get(at)?),
            Kind::Repository => RecordRef::Repository(data.repositories.get(at)?),
            Kind::Multimedia => RecordRef::Multimedia(data.multimedia.get(at)?),
            Kind::Submitter => RecordRef::Submitter(data.submitters.get(at)?),
            Kind::Submission => RecordRef::Submission(data.submissions.get(at)?),
            Kind::Note => RecordRef::Note(data.notes.get(at)?),
            Kind::Other => RecordRef::Other(data.extra.get(at)?),
        })
    }

    /// The individual with this identifier.
    #[must_use]
    pub fn find_individual(&self, xref: impl XrefKey) -> Option<&Individual> {
        self.data.individuals.get(self.at(xref, Kind::Individual)?)
    }

    /// The family with this identifier.
    #[must_use]
    pub fn find_family(&self, xref: impl XrefKey) -> Option<&Family> {
        self.data.families.get(self.at(xref, Kind::Family)?)
    }

    /// The source with this identifier.
    #[must_use]
    pub fn find_source(&self, xref: impl XrefKey) -> Option<&Source> {
        self.data.sources.get(self.at(xref, Kind::Source)?)
    }

    /// The repository with this identifier.
    #[must_use]
    pub fn find_repository(&self, xref: impl XrefKey) -> Option<&Repository> {
        self.data.repositories.get(self.at(xref, Kind::Repository)?)
    }

    /// The multimedia object with this identifier.
    #[must_use]
    pub fn find_multimedia(&self, xref: impl XrefKey) -> Option<&Multimedia> {
        self.data.multimedia.get(self.at(xref, Kind::Multimedia)?)
    }

    /// The submitter with this identifier.
    #[must_use]
    pub fn find_submitter(&self, xref: impl XrefKey) -> Option<&Submitter> {
        self.data.submitters.get(self.at(xref, Kind::Submitter)?)
    }

    /// The shared note with this identifier.
    #[must_use]
    pub fn find_note(&self, xref: impl XrefKey) -> Option<&SharedNote> {
        self.data.notes.get(self.at(xref, Kind::Note)?)
    }

    fn families_in<'s>(
        &'s self,
        links: &'s Links,
        individual: impl XrefKey,
    ) -> impl Iterator<Item = &'s Family> {
        let positions = individual
            .id_in(self.data.store())
            .map_or(&[][..], |id| links.of(id.index()));
        positions
            .iter()
            .filter_map(|&at| self.data.families.get(at as usize))
    }

    /// The families in which this individual is a partner (`HUSB` or
    /// `WIFE`), in order.
    pub fn families_as_spouse(&self, individual: impl XrefKey) -> impl Iterator<Item = &Family> {
        self.families_in(&self.as_spouse, individual)
    }

    /// The families in which this individual is a child (`CHIL`), in
    /// order.
    pub fn families_as_child(&self, individual: impl XrefKey) -> impl Iterator<Item = &Family> {
        self.families_in(&self.as_child, individual)
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
        let id = individual.id_in(self.data.store())?;
        let other = if family.husband_id() == Some(id) {
            family.wife_id()
        } else if family.wife_id() == Some(id) {
            family.husband_id()
        } else {
            None
        };
        self.find_individual(other?)
    }
}

impl From<Dataset> for IndexedDataset {
    fn from(data: Dataset) -> Self {
        Self::new(data)
    }
}

impl Deref for IndexedDataset {
    type Target = Dataset;

    fn deref(&self) -> &Dataset {
        &self.data
    }
}

/// The dataset of an [`IndexedDataset`], to edit: dropping it rebuilds the
/// index.
#[derive(Debug)]
pub struct DatasetMut<'a> {
    indexed: &'a mut IndexedDataset,
}

impl Deref for DatasetMut<'_> {
    type Target = Dataset;

    fn deref(&self) -> &Dataset {
        &self.indexed.data
    }
}

impl DerefMut for DatasetMut<'_> {
    fn deref_mut(&mut self) -> &mut Dataset {
        &mut self.indexed.data
    }
}

impl Drop for DatasetMut<'_> {
    fn drop(&mut self) {
        self.indexed.reindex();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{IndividualRef, Name};

    const FILE: &str = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n\
        0 @I1@ INDI\n1 NAME Ann /Example/\n\
        0 @I2@ INDI\n1 NAME Bob /Example/\n\
        0 @I3@ INDI\n1 NAME Cid /Example/\n\
        0 @I1@ INDI\n1 NAME Duplicate /Example/\n\
        0 @F1@ FAM\n1 HUSB @I2@\n1 WIFE @I1@\n1 CHIL @I3@\n1 CHIL @I9@\n\
        0 @F2@ FAM\n1 WIFE @I1@\n\
        0 @S1@ SOUR\n0 @R1@ REPO\n0 @M1@ OBJE\n0 @U1@ SUBM\n0 @N1@ NOTE Shared\n0 @X1@ _LOC\n\
        0 TRLR\n";

    #[test]
    fn finds_every_record_type_first_wins() {
        let indexed = IndexedDataset::new(Dataset::parse(FILE));
        let ann = indexed.find_individual("@I1@").unwrap();
        assert_eq!(ann.full_name(&*indexed).as_deref(), Some("Ann Example"));
        assert!(indexed.find_family("@F1@").is_some());
        assert!(indexed.find_source("@S1@").is_some());
        assert!(indexed.find_repository("@R1@").is_some());
        assert!(indexed.find_multimedia("@M1@").is_some());
        assert!(indexed.find_submitter("@U1@").is_some());
        assert!(indexed.find_note("@N1@").is_some());
        assert!(matches!(indexed.find("@X1@"), Some(RecordRef::Other(_))));
        assert!(indexed.find_family("@I1@").is_none());
        assert!(indexed.find("@I9@").is_none());
        assert!(indexed.find("@nothing@").is_none());
        // Every lookup agrees with the dataset's linear search.
        for xref in ["@I1@", "@I2@", "@F2@", "@S1@", "@N1@", "@X1@", "@I9@"] {
            assert_eq!(indexed.find(xref), indexed.data().find(xref), "{xref}");
        }
    }

    #[test]
    fn navigates_families() {
        let indexed = IndexedDataset::from(Dataset::parse(FILE));
        let ann = indexed.find_individual("@I1@").unwrap();
        let families: Vec<_> = indexed.families_as_spouse(ann.xref).collect();
        assert_eq!(families.len(), 2);
        assert_eq!(indexed.parents(families[0]).count(), 2);
        assert_eq!(indexed.children(families[0]).count(), 1); // @I9@ has no record
        assert_eq!(
            indexed
                .spouse("@I1@", families[0])
                .and_then(|s| s.full_name(&*indexed)),
            Some("Bob Example".to_string())
        );
        assert_eq!(indexed.families_as_child("@I3@").count(), 1);
        assert_eq!(indexed.families_as_child("@I1@").count(), 0);
        // The same answers as the dataset's own navigation.
        let data = indexed.data();
        assert_eq!(
            data.families_as_spouse("@I1@").count(),
            indexed.families_as_spouse("@I1@").count()
        );
    }

    #[test]
    fn edits_reindex() {
        let mut indexed = IndexedDataset::new(Dataset::parse(FILE));
        {
            let mut data = indexed.data_mut();
            let id = data.store_mut().intern_xref("@I7@").unwrap();
            let mut person = Individual {
                xref: Some(id),
                ..Individual::default()
            };
            person.names.push(Name::new("Eve /Example/"));
            data.individuals.push(person);
            let wife = IndividualRef::new(id);
            data.families[1].wife = Some(wife);
        }
        assert!(indexed.find_individual("@I7@").is_some());
        assert_eq!(indexed.families_as_spouse("@I7@").count(), 1);
        assert_eq!(indexed.families_as_spouse("@I1@").count(), 1);
        assert_eq!(indexed.into_inner().individuals.len(), 5);
    }
}
