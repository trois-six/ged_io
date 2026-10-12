//! The conformance repair of records written one at a time, with nothing
//! of the dataset at hand but the header
//! ([`GedcomStreamWriter`](crate::GedcomStreamWriter)).
//!
//! Each record is checked and repaired by the code of the whole pass
//! ([`conform_records`](super::conform_records)), under three differences
//! that come from not seeing the other records:
//!
//! - a pointer valid in the version's grammar is taken to name a record
//!   of its type, which may come later: dangling pointers and pointers to
//!   a record of another type are not repaired;
//! - a structure the whole pass relocates under a `HEAD.SCHMA`
//!   declaration (7.x) becomes a plain extension structure, the header
//!   being written already;
//! - the notes moved out of a 5.5.1 record over 32K go to `NOTE` records
//!   written right after it, instead of with the other notes.
//!
//! The records of each type are counted as they come, so that one more
//! than its type permits becomes an extension record as in the whole
//! pass.

use super::clean::Walk;
use super::{bump, encoded_len, Aliases, Conformer, Family, Typing, DATASET};
use crate::spec::conform::Build;
use crate::spec::validate::MAX_RECORD_BYTES;
use crate::tree::Node;
use crate::tree::{Flat, Structure};
use crate::writer::{complete_head, Repair, RepairKind};
use crate::GedcomVersion;

/// The state of records written one at a time: see the [module
/// documentation](self).
pub(crate) struct Streamed {
    version: GedcomVersion,
    /// The aliases the header declares (7.x).
    aliases: Aliases,
    /// The records of each standard type so far.
    counts: Vec<(u8, u32)>,
    /// The repairs, in order.
    pub(crate) repairs: Vec<Repair>,
}

/// A record as [`Streamed::record`] leaves it.
pub(crate) enum Outcome<'r> {
    /// As it wrote itself, typed, in this arena.
    Clean(Flat<'r>),
    /// Repaired, with the notes moved out of it.
    Repaired(Structure, Vec<Structure>),
}

impl Streamed {
    /// Completes and repairs `head`, the header of records of `version`,
    /// as the whole pass does, pointing to `submitter` when it needs one.
    pub(crate) fn header(
        version: GedcomVersion,
        head: &mut Structure,
        submitter: Option<String>,
    ) -> Self {
        let mut this = Self {
            version,
            aliases: Aliases::default(),
            counts: Vec::new(),
            repairs: Vec::new(),
        };
        let mut c = this.conformer();
        if let Some(x) = head.xref.take() {
            c.repair(
                head.line,
                RepairKind::Xref,
                format!("identifier {x} of HEAD left out"),
            );
        }
        c.aliases = Aliases::of(Some(&*head), c.schema);
        let head_ty = c.record_of("HEAD");
        complete_head(head, c.rules, "UTF-8", submitter);
        if let Some(ty) = head_ty {
            c.model_order_to(head, ty, 2);
            if !c.is_clean(&*head, ty, true) {
                c.path.clear();
                let _ = c.fix(head, ty, true);
                c.model_order(head, ty);
                complete_head(head, c.rules, "UTF-8", None);
                c.model_order_to(head, ty, 2);
            }
        }
        this.aliases = Aliases::of(Some(&*head), c.schema);
        this.repairs = c.repairs;
        this
    }

    /// A conformance pass for one record.
    fn conformer<'r>(&self) -> Conformer<'r> {
        let mut c = Conformer::new(self.version);
        c.order = Some(crate::model::field_order(self.version));
        c.aliases = self.aliases.clone();
        c.trust = true;
        c.no_declare = true;
        c
    }

    /// Writes `record` typed and checks it; repairs it when it needs it.
    /// `fresh` makes the identifier of a `NOTE` record a note moves to.
    pub(crate) fn record<'r>(
        &mut self,
        record: &dyn Build<'r>,
        fresh: &mut dyn FnMut() -> String,
    ) -> Outcome<'r> {
        let mut c = self.conformer();
        let mut flat = Flat::default();
        flat.set_typing(Some(Typing::of(self.version)));
        record.build(&mut flat);
        let mut walk = Walk::default();
        let clean = c.is_clean_written(&flat, &mut walk);
        if clean {
            let mut over = false;
            if walk.rtype != DATASET {
                let t = c.schema.tag_id(walk.rtype).unwrap_or(0);
                let n = bump(&mut self.counts, t);
                if c.picker
                    .max(DATASET, t)
                    .is_some_and(|max| n > u32::from(max))
                {
                    if let Some((_, n)) = self.counts.iter_mut().find(|(k, _)| *k == t) {
                        *n -= 1;
                    }
                    over = true;
                }
            }
            let big = c.family == Family::V551
                && walk.size > MAX_RECORD_BYTES
                && flat
                    .root()
                    .is_some_and(|n| encoded_len(n, 0, c.family) > MAX_RECORD_BYTES);
            if !over && !big {
                return Outcome::Clean(flat);
            }
        }
        let mut s = flat
            .root()
            .map(Node::to_owned_structure)
            .unwrap_or_default();
        c.path.clear();
        c.record(&mut s, &mut self.counts);
        let notes = Self::notes(&mut c, &mut s, fresh);
        self.repairs.append(&mut c.repairs);
        Outcome::Repaired(s, notes)
    }

    /// (j) The notes moved out of `s` when it is a 5.5.1 record over 32K.
    fn notes(
        c: &mut Conformer<'_>,
        s: &mut Structure,
        fresh: &mut dyn FnMut() -> String,
    ) -> Vec<Structure> {
        let Some(ty) = c.record_of_standard(s.tag.as_str(), s.tag.standard_index()) else {
            return Vec::new();
        };
        if c.family != Family::V551 {
            return Vec::new();
        }
        let size = encoded_len(&*s, 0, c.family);
        if size <= MAX_RECORD_BYTES {
            return Vec::new();
        }
        c.move_notes(s, ty, size, &mut |_| fresh())
            .unwrap_or_default()
    }
}
