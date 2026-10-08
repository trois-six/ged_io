//! Record identifiers of a written file: valid, unique and consistent with
//! the pointers.
//!
//! Before anything is written, [`XrefMap`] looks at every record's
//! identifier once, in write order:
//!
//! 1. the first record holding a valid identifier keeps it;
//! 2. a record whose identifier is invalid for the target version (`@i1@` in
//!    7.x, 25 characters in 5.5.1, `@VOID@` in 7.x) gets the nearest valid
//!    one ([`candidate_xref`]), made unique with a `_2`, `_3`… suffix, and
//!    every pointer to the old identifier follows it;
//! 3. a record holding an identifier already taken by an earlier record gets
//!    a new one; pointers keep resolving to the first record, as lookups do;
//! 4. a standard record without an identifier gets the first free `@I1@`,
//!    `@F1@`, … of its kind.
//!
//! A pointer to no record is kept when valid and otherwise rewritten the
//! same way, without taking an identifier of a record.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use super::emit::{candidate_xref, is_valid_pointer};
use super::{Repair, RepairKind};
use crate::version::{VersionRules, XrefGrammar};

/// The identifiers of the records of one written file.
pub(crate) struct XrefMap<'a> {
    rules: &'static VersionRules,
    /// The identifiers of the data that records keep.
    taken: HashSet<&'a str>,
    /// The identifiers the map made.
    made: HashSet<Box<str>>,
    /// The new identifier of each record that does not keep its own, by its
    /// position in write order.
    records: HashMap<usize, Box<str>>,
    /// The new identifier of each invalid identifier, for pointers.
    renamed: HashMap<Cow<'a, str>, Box<str>>,
    /// The renames, to report.
    pub(crate) repairs: Vec<Repair>,
}

/// A record as the map sees it: the prefix of the identifiers generated for
/// its kind (`None` for records that need none, such as extension records),
/// and its identifier.
pub(crate) type RecordXref<'a> = (Option<&'static str>, Option<&'a str>);

impl<'a> XrefMap<'a> {
    /// Assigns the identifiers of `records`, given in write order.
    pub(crate) fn new<I>(rules: &'static VersionRules, records: I) -> Self
    where
        I: Iterator<Item = RecordXref<'a>>,
    {
        let mut map = Self {
            rules,
            taken: HashSet::with_capacity(records.size_hint().0),
            made: HashSet::new(),
            records: HashMap::new(),
            renamed: HashMap::new(),
            repairs: Vec::new(),
        };
        // One pass: the valid identifiers first, so that a rewritten one
        // never takes them; the other records are handled after it.
        let mut irregular = Vec::new();
        for (index, (prefix, xref)) in records.enumerate() {
            let keep = xref.is_some_and(|x| rules.is_valid_xref(x) && map.taken.insert(x));
            if !keep && (xref.is_some() || prefix.is_some()) {
                irregular.push((index, prefix, xref));
            }
        }
        let mut seen_invalid: HashSet<&'a str> = HashSet::new();
        let mut counters: HashMap<&'static str, usize> = HashMap::new();
        for (index, prefix, xref) in irregular {
            let new = match xref {
                Some(x) => {
                    let new = map.unique(candidate_xref(rules, x));
                    if !rules.is_valid_xref(x) && seen_invalid.insert(x) {
                        map.renamed.insert(x.into(), new.clone().into());
                    }
                    map.repairs.push(Repair::new(
                        0,
                        RepairKind::Xref,
                        format!("identifier {x} written as {new}"),
                    ));
                    new
                }
                None => match prefix {
                    Some(prefix) => {
                        let n = counters.entry(prefix).or_insert(0);
                        loop {
                            *n += 1;
                            let candidate = format!("@{prefix}{n}@");
                            if !map.is_taken(&candidate) {
                                map.made.insert(candidate.as_str().into());
                                break candidate;
                            }
                        }
                    }
                    None => continue,
                },
            };
            map.records.insert(index, new.into());
        }
        map
    }

    /// Whether a record already has `xref`.
    fn is_taken(&self, xref: &str) -> bool {
        self.taken.contains(xref) || self.made.contains(xref)
    }

    /// `candidate`, or `candidate` with the first free `_n` suffix; taken.
    fn unique(&mut self, candidate: String) -> String {
        let mut new = candidate;
        if self.is_taken(&new) {
            let base = new.trim_end_matches('@').to_string();
            let mut n = 1_usize;
            new = loop {
                n += 1;
                let suffix = format!("_{n}@");
                let base = match self.rules.xref {
                    XrefGrammar::V551 { max_len } => {
                        let keep = max_len.saturating_sub(suffix.len()).min(base.len());
                        let mut keep = keep;
                        while !base.is_char_boundary(keep) {
                            keep -= 1;
                        }
                        &base[..keep]
                    }
                    XrefGrammar::V7 => base.as_str(),
                };
                let candidate = format!("{base}{suffix}");
                if !self.is_taken(&candidate) {
                    break candidate;
                }
            };
        }
        self.made.insert(new.as_str().into());
        new
    }

    /// A new identifier for a record that is not in the map (the stub
    /// submitter of a 5.5.1 header).
    pub(crate) fn fresh(&mut self, prefix: &str) -> String {
        let mut n = 0_usize;
        loop {
            n += 1;
            let candidate = format!("@{prefix}{n}@");
            if !self.is_taken(&candidate) {
                self.made.insert(candidate.as_str().into());
                return candidate;
            }
        }
    }

    /// The identifier written for the record at `index` in write order,
    /// whose own identifier is `xref`.
    pub(crate) fn record<'x>(&'x self, index: usize, xref: Option<&'x str>) -> Option<&'x str> {
        if self.records.is_empty() {
            return xref;
        }
        match self.records.get(&index) {
            Some(new) => Some(new),
            None => xref,
        }
    }

    /// The identifier a pointer is written with.
    pub(crate) fn pointer<'p>(&mut self, pointer: &'p str) -> Cow<'p, str> {
        // `@VOID@` points to nothing in 7.x, even when a record had it.
        if self.rules.xref == XrefGrammar::V7 && pointer == "@VOID@" {
            return Cow::Borrowed(pointer);
        }
        if !self.renamed.is_empty() {
            if let Some(new) = self.renamed.get(pointer) {
                return Cow::Owned(new.to_string());
            }
        }
        if is_valid_pointer(self.rules, pointer) {
            return Cow::Borrowed(pointer);
        }
        // A pointer to no record, outside the grammar: rewritten once, the
        // same way everywhere, never onto a record's identifier.
        let new = self.unique(candidate_xref(self.rules, pointer));
        self.renamed
            .insert(Cow::Owned(pointer.to_string()), new.clone().into());
        Cow::Owned(new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::{V551, V70};

    #[test]
    fn valid_unique_identifiers_are_kept() {
        let records = [(Some("I"), Some("@I1@")), (Some("F"), Some("@F1@"))];
        let mut map = XrefMap::new(&V70, records.iter().copied());
        assert_eq!(map.record(0, Some("@I1@")), Some("@I1@"));
        assert_eq!(map.pointer("@F1@"), "@F1@");
        assert!(map.repairs.is_empty());
    }

    #[test]
    fn invalid_duplicate_void_and_missing() {
        let records = [
            (Some("I"), Some("@i1@")),
            (Some("I"), Some("@I1@")),
            (Some("I"), Some("@I1@")),
            (Some("I"), Some("@VOID@")),
            (Some("I"), None),
            (None, None),
        ];
        let mut map = XrefMap::new(&V70, records.iter().copied());
        // The valid @I1@ keeps its name; @i1@ moves aside, and so does the
        // second @I1@.
        assert_eq!(map.record(1, Some("@I1@")), Some("@I1@"));
        assert_eq!(map.record(0, Some("@i1@")), Some("@I1_2@"));
        assert_eq!(map.record(2, Some("@I1@")), Some("@I1_3@"));
        assert_eq!(map.record(3, Some("@VOID@")), Some("@VOID_@"));
        assert_eq!(map.record(4, None), Some("@I2@"));
        assert_eq!(map.record(5, None), None);
        assert_eq!(map.pointer("@i1@"), "@I1_2@");
        assert_eq!(map.pointer("@I1@"), "@I1@");
        assert_eq!(map.pointer("@VOID@"), "@VOID@");
        // A dangling invalid pointer never lands on a record.
        assert_eq!(map.pointer("@i2@"), "@I2_2@");
        assert_eq!(map.repairs.len(), 3);
    }

    #[test]
    fn long_identifiers_in_551() {
        let long = "@IABCDEFGHIJKLMNOPQRSTUVWXYZ0123@";
        let records = [(Some("I"), Some(long)), (Some("I"), Some(long))];
        let mut map = XrefMap::new(&V551, records.iter().copied());
        let first = map.record(0, Some(long)).unwrap().to_string();
        let second = map.record(1, Some(long)).unwrap().to_string();
        assert!(V551.is_valid_xref(&first), "{first}");
        assert!(V551.is_valid_xref(&second), "{second}");
        assert_ne!(first, second);
        assert_eq!(map.pointer(long), first);
    }
}
