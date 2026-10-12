//! The GEDCOM 7.x rules on pointers between records (7.0.17), which a
//! record alone cannot show:
//!
//! - an `ALIA` does not point to its own individual record (§ALIA: "should
//!   not point to the superstructure of the `ALIA`", which 7.0.17's release
//!   notes call prohibited);
//! - source records and multimedia records, and source records and shared
//!   note records, may point to each other, but these pointers form no
//!   cycle (§`MULTIMEDIA_RECORD`, §`SOURCE_RECORD`: "applications must not
//!   create datasets where these mutual pointers form a cycle").
//!
//! The pointers of the second rule are those of a source record (at any
//! depth of its standard structures) to multimedia and shared note
//! records, and those of multimedia and shared note records to source
//! records: one graph, so a cycle may pass through both kinds of record.
//! A cycle of individuals and families (someone their own ancestor) is
//! permitted (§`FAMILY_RECORD`), and checked by nothing.
//!
//! The rules are decided record by record, in dataset order, so that the
//! validator, the conformance repair of a whole dataset and the writers
//! that check records one at a time decide alike: a pointer breaks the
//! cycle rule when the record it names already reaches, through the
//! pointers kept so far, the record that holds it. The pointers kept never
//! form a cycle, and removing those found leaves a dataset without one.

use std::collections::{HashMap, HashSet};

/// The pointers of the second rule kept so far: for each record, the
/// records it points to.
#[derive(Debug, Default)]
pub(crate) struct Graph {
    edges: HashMap<Box<str>, Vec<Box<str>>>,
}

/// Which rule a pointer breaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Broken {
    /// An `ALIA` that points to its own record.
    SelfAlias,
    /// A pointer that closes a cycle.
    Cycle,
}

impl Broken {
    /// What is wrong, in words, for a pointer `tag p` of record `xref`.
    pub(crate) fn detail(self, tag: &str, p: &str, xref: &str) -> String {
        match self {
            Broken::SelfAlias => format!("{tag} {p} points to its own record"),
            Broken::Cycle => format!(
                "{tag} {p} closes a cycle of pointers between source, multimedia and shared note records back to {xref}"
            ),
        }
    }
}

/// A pointer of a record the rules concern: its payload and whether it is
/// an `ALIA`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Pointer<'p> {
    pub(crate) to: &'p str,
    pub(crate) alias: bool,
}

impl Graph {
    /// Which of the pointers of record `xref` break a rule, by index, given
    /// the pointers kept so far; then keeps the others. `None` as `xref`
    /// (a record without an identifier, which nothing points to) breaks
    /// nothing and is kept out of the graph.
    pub(crate) fn record(
        &mut self,
        xref: Option<&str>,
        pointers: &[Pointer<'_>],
    ) -> Vec<(usize, Broken)> {
        let broken = self.judge(xref, pointers);
        self.keep(xref, pointers, &broken);
        broken
    }

    /// [`Graph::record`] without keeping anything.
    pub(crate) fn judge(
        &self,
        xref: Option<&str>,
        pointers: &[Pointer<'_>],
    ) -> Vec<(usize, Broken)> {
        let Some(xref) = xref else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (i, p) in pointers.iter().enumerate() {
            if p.to == "@VOID@" {
                continue;
            }
            if p.alias {
                if p.to == xref {
                    out.push((i, Broken::SelfAlias));
                }
            } else if p.to == xref || self.reaches(p.to, xref) {
                out.push((i, Broken::Cycle));
            }
        }
        out
    }

    /// Keeps the pointers of record `xref` when none breaks a rule: whether
    /// it did (a writer that checks records one at a time leaves a record
    /// that breaks one to the whole pass).
    pub(crate) fn keep_if_clean(&mut self, xref: Option<&str>, pointers: &[Pointer<'_>]) -> bool {
        if pointers.is_empty() {
            return true;
        }
        if !self.judge(xref, pointers).is_empty() {
            return false;
        }
        self.keep(xref, pointers, &[]);
        true
    }

    /// Keeps the pointers of record `xref` but those `broken` names.
    pub(crate) fn keep(
        &mut self,
        xref: Option<&str>,
        pointers: &[Pointer<'_>],
        broken: &[(usize, Broken)],
    ) {
        let Some(xref) = xref else { return };
        let mut kept = pointers
            .iter()
            .enumerate()
            .filter(|(i, p)| !p.alias && p.to != "@VOID@" && !broken.iter().any(|(b, _)| b == i))
            .map(|(_, p)| Box::from(p.to))
            .peekable();
        if kept.peek().is_none() {
            return;
        }
        self.edges.entry(Box::from(xref)).or_default().extend(kept);
    }

    /// Whether `from` reaches `to` through the pointers kept.
    fn reaches(&self, from: &str, to: &str) -> bool {
        if self.edges.is_empty() {
            return false;
        }
        let mut seen: HashSet<&str> = HashSet::new();
        let mut stack = vec![from];
        while let Some(at) = stack.pop() {
            if at == to {
                return true;
            }
            if !seen.insert(at) {
                continue;
            }
            if let Some(next) = self.edges.get(at) {
                stack.extend(next.iter().map(|n| &**n));
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ptr(to: &str) -> Pointer<'_> {
        Pointer { to, alias: false }
    }

    #[test]
    fn the_pointer_that_closes_a_cycle_breaks_the_rule() {
        let mut g = Graph::default();
        // @S1@ -> @O1@, then @O1@ -> @S1@ closes the cycle.
        assert_eq!(g.record(Some("@S1@"), &[ptr("@O1@")]), []);
        assert_eq!(
            g.record(Some("@O1@"), &[ptr("@S1@"), ptr("@S2@")]),
            [(0, Broken::Cycle)]
        );
        // Longer: @S2@ -> @N1@ -> ... @O1@ -> @S2@ was kept above, so
        // @S2@ -> @O1@ closes it.
        assert_eq!(
            g.record(Some("@S2@"), &[ptr("@N1@"), ptr("@O1@")]),
            [(1, Broken::Cycle)]
        );
        // A pointer to a record not seen yet is kept, and the record's own
        // pointer back is the one found.
        assert_eq!(g.record(Some("@S3@"), &[ptr("@O3@")]), []);
        assert_eq!(g.record(Some("@O3@"), &[ptr("@S3@")]), [(0, Broken::Cycle)]);
        // `@VOID@` and records without an identifier are left alone.
        assert_eq!(g.record(Some("@O4@"), &[ptr("@VOID@")]), []);
        assert_eq!(g.record(None, &[ptr("@S1@")]), []);
    }

    #[test]
    fn an_alias_to_its_own_record_breaks_the_rule() {
        let mut g = Graph::default();
        let alias = |to| Pointer { to, alias: true };
        assert_eq!(
            g.record(Some("@I1@"), &[alias("@I2@"), alias("@I1@")]),
            [(1, Broken::SelfAlias)]
        );
        // Aliases form no graph: mutual aliases are fine.
        assert_eq!(g.record(Some("@I2@"), &[alias("@I1@")]), []);
    }
}
