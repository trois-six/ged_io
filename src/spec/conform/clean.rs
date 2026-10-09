//! Whether a structure needs a repair, found without changing it: the
//! read-only twin of the repair pass ([`Conformer::fix`]).
//!
//! Each check answers whether the repair of a structure would leave it as
//! it is and report nothing, under the state of the pass (the identifiers
//! and the types of their records, the renamed identifiers, the aliases).
//! It may answer no for a structure the repair leaves alone (the repair
//! then runs on a copy and changes nothing), never yes for one it changes:
//! debug builds check every record it passes against the repair itself.
//!
//! The check allocates nothing and finds tags by their index: it is the
//! cost of conformance on a dataset that needs no repair.

use super::{payload, Conformer, Family, Kind, Pick, StructId, DATASET};
use crate::spec::schema::EnumSet;
use crate::spec::validate::is_external_pointer;
use crate::tree::{Flat, Node, PayloadRef};
use crate::writer::XrefIndex;

/// The occurrences of each tag among the substructures of one structure:
/// as many tags as fit inline, the rest in a list.
struct Counts {
    inline: [(u8, u32); 8],
    len: usize,
    more: Vec<(u8, u32)>,
}

impl Counts {
    fn new() -> Self {
        Self {
            inline: [(0, 0); 8],
            len: 0,
            more: Vec::new(),
        }
    }

    /// Counts one more `t`; returns the count.
    #[inline]
    fn bump(&mut self, t: u8) -> u32 {
        let inline = self.inline.get_mut(..self.len).unwrap_or_default();
        if let Some((_, n)) = inline.iter_mut().find(|(k, _)| *k == t) {
            *n += 1;
            return *n;
        }
        if let Some((_, n)) = self.more.iter_mut().find(|(k, _)| *k == t) {
            *n += 1;
            return *n;
        }
        match self.inline.get_mut(self.len) {
            Some(slot) => {
                *slot = (t, 1);
                self.len += 1;
            }
            None => self.more.push((t, 1)),
        }
        1
    }

    #[inline]
    fn get(&self, t: u8) -> u32 {
        self.inline
            .get(..self.len)
            .unwrap_or_default()
            .iter()
            .chain(&self.more)
            .find(|(k, _)| *k == t)
            .map_or(0, |(_, n)| *n)
    }
}

/// What a check gathers as it walks.
#[derive(Default)]
pub(super) struct Walk<'w> {
    /// An upper bound of the 5.5.1 size of what it walked.
    pub(super) size: usize,
    /// Where pointers go to be looked up later, all together, with the
    /// record they are in: lookups in a row wait for memory together, one
    /// at a time between nodes they each wait alone. `None`: at once.
    pub(super) pending: Option<&'w mut Vec<Lookup>>,
    /// The record walked.
    pub(super) record: u32,
    /// Its record type, when it is a standard record (the dataset's type
    /// otherwise).
    pub(super) rtype: StructId,
}

/// A pointer to look up: its packed identifier, the type of record it must
/// name ([`ANY`]: any record), and the record it is in.
pub(super) struct Lookup {
    key: u128,
    want: StructId,
    pub(super) record: u32,
}

/// [`Lookup::want`] of a pointer that may name a record of any type.
const ANY: StructId = StructId::MAX;

/// What the check reads of a node once: its identifier and its payload.
#[derive(Clone, Copy)]
struct At<'a> {
    xref: Option<&'a str>,
    payload: PayloadRef<'a>,
}

/// An upper bound of what a structure adds to the size of its record as
/// written in 5.5.1 ([`encoded_len`](crate::spec::validate::encoded_len)
/// counts at most two level digits, an `@` doubled, a `CONT` line per line
/// break and a `CONC` line per 200 bytes, each of at most ten bytes more).
#[inline]
fn size_bound<'a, N: Node<'a>>(n: N, xref: Option<&str>, payload: PayloadRef<'_>) -> usize {
    // A standard tag has at most six letters.
    let tag = n.standard_tag().map_or_else(|| n.tag().len(), |_| 6);
    let payload = payload.as_str().map_or(0, str::len);
    8 + tag + xref.map_or(0, str::len) + 13 * payload
}

impl Conformer<'_> {
    /// Whether the repair of record `r` would change nothing
    /// ([`Conformer::record`] before its count of records per type). Adds
    /// an upper bound of its 5.5.1 size to `size`.
    /// `xref` is the record's identifier.
    pub(super) fn is_clean_record<'a, N: Node<'a>>(
        &self,
        r: N,
        xref: Option<&str>,
        walk: &mut Walk<'_>,
    ) -> bool {
        let payload = r.payload();
        let is_ptr = matches!(payload, PayloadRef::Pointer(_));
        let at = At { xref, payload };
        match self.picker.pick(DATASET, r, is_ptr) {
            Pick::Type(ty) => {
                walk.rtype = ty;
                self.clean(r, at, ty, true, walk)
            }
            // A documented alias of a standard record is that record.
            Pick::Extension => match self.aliases.structs.get(r.tag()) {
                Some(&ty) => self.clean(r, at, ty, true, walk),
                None => self.clean_extension(r, at, true, walk),
            },
            _ => false,
        }
    }

    /// [`Conformer::is_clean_record`] of a record of the typed model that
    /// typed itself as it was written ([`Flat::typed`]): every structure is
    /// placed where its type permits it and has what its type requires, so
    /// only the payloads it did not make valid need a look.
    /// Its extension structures are walked as any. `None` when one is an
    /// alias of a standard structure ([`Aliases`](super::Aliases)), which
    /// the record's walk checks as that structure.
    pub(super) fn is_clean_typed(&self, flat: &Flat<'_>, walk: &mut Walk<'_>) -> Option<bool> {
        if !self.aliases.structs.is_empty()
            && flat
                .extensions()
                .any(|e| self.aliases.structs.contains_key(e.tag()))
        {
            return None;
        }
        walk.size += flat.size();
        walk.rtype = flat.root_type().unwrap_or(DATASET);
        Some(
            flat.checks().all(|(payload, ty, special)| {
                self.clean_payload_known(payload, ty, Some(special), walk)
            }) && flat.extensions().all(|e| {
                let at = At {
                    xref: e.xref(),
                    payload: e.payload(),
                };
                // An extension tag in the grammar, as a pick finds it.
                self.rules.is_valid_tag(e.tag()) && self.clean_extension(e, at, true, walk)
            }),
        )
    }

    /// [`Conformer::is_clean`] of a record, its size bound added to
    /// `walk`.
    pub(super) fn is_clean_walked<'a, N: Node<'a>>(
        &self,
        n: N,
        ty: StructId,
        walk: &mut Walk<'_>,
    ) -> bool {
        let at = At {
            xref: n.xref(),
            payload: n.payload(),
        };
        self.clean(n, at, ty, true, walk)
    }

    /// Whether the repair of `n`, a structure of type `ty`, would change
    /// nothing ([`Conformer::fix`]).
    pub(super) fn is_clean<'a, N: Node<'a>>(&self, n: N, ty: StructId, record: bool) -> bool {
        let at = At {
            xref: n.xref(),
            payload: n.payload(),
        };
        self.clean(n, at, ty, record, &mut Walk::default())
    }

    fn clean<'a, N: Node<'a>>(
        &self,
        n: N,
        at: At<'_>,
        ty: StructId,
        record: bool,
        walk: &mut Walk<'_>,
    ) -> bool {
        walk.size += size_bound(n, at.xref, at.payload);
        if !record && at.xref.is_some() {
            return false;
        }
        if !self.clean_payload(at.payload, ty, walk) {
            return false;
        }
        let schema = self.schema;
        let mut counts = Counts::new();
        let mut children = false;
        for c in n.children() {
            children = true;
            let at = At {
                xref: c.xref(),
                payload: c.payload(),
            };
            let is_ptr = matches!(at.payload, PayloadRef::Pointer(_));
            match self.picker.pick(ty, c, is_ptr) {
                Pick::Type(cty) => {
                    if !self.clean(c, at, cty, false, walk) {
                        return false;
                    }
                    let t = schema.tag_id(cty).unwrap_or(0);
                    // One occurrence is always permitted.
                    let k = counts.bump(t);
                    if k > 1 && self.picker.max(ty, t).is_some_and(|max| k > u32::from(max)) {
                        return false;
                    }
                }
                // A documented alias of a standard structure is that
                // structure, wherever it is; an extension structure keeps
                // what it holds at its own level, as a record does.
                Pick::Extension => {
                    let clean = match self.aliases.structs.get(c.tag()) {
                        Some(&aty) => self.clean(c, at, aty, false, walk),
                        None => self.clean_extension(c, at, true, walk),
                    };
                    if !clean {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        // (g) Required substructures.
        let required = self
            .tables
            .required
            .get(usize::from(ty))
            .map_or(&[][..], |r| r);
        if required
            .iter()
            .any(|&(t, min, _)| counts.get(t) < u32::from(min))
        {
            return false;
        }
        if self.family == Family::V7 {
            // A note translation says its language or its media type.
            if self.tables.note_tran.contains(&ty)
                && !n.children().any(|c| matches!(c.tag(), "MIME" | "LANG"))
            {
                return false;
            }
            // A structure has a payload or a substructure.
            if !record && !children && at.payload.as_str().is_none_or(str::is_empty) {
                return false;
            }
        }
        true
    }

    /// Whether an extension structure needs nothing at its own level and
    /// below ([`Conformer::extension`]).
    pub(super) fn is_clean_extension<'a, N: Node<'a>>(&self, n: N) -> bool {
        let at = At {
            xref: n.xref(),
            payload: n.payload(),
        };
        self.clean_extension(n, at, true, &mut Walk::default())
    }

    /// Whether an extension structure needs nothing: tags in the grammar,
    /// no identifier below the record, pointers that name a record (and no
    /// renamed one), no banned character ([`Conformer::extension_content`]).
    fn clean_extension<'a, N: Node<'a>>(
        &self,
        n: N,
        at: At<'_>,
        top: bool,
        walk: &mut Walk<'_>,
    ) -> bool {
        walk.size += size_bound(n, at.xref, at.payload);
        if !top {
            let valid_tag = n.standard_tag().is_some() || self.rules.is_valid_tag(n.tag());
            if !valid_tag || at.xref.is_some() {
                return false;
            }
        }
        let payload_ok = match at.payload {
            PayloadRef::Pointer(p) => self.resolves(p, walk),
            PayloadRef::Text(t) => !payload::has_banned(t, self.rules),
            PayloadRef::None => true,
        };
        payload_ok
            && n.children().all(|c| {
                let at = At {
                    xref: c.xref(),
                    payload: c.payload(),
                };
                self.clean_extension(c, at, false, walk)
            })
    }

    /// Whether a pointer stays as it is and names a record (of any type),
    /// `@VOID@` or, in 5.5.1, a substructure or a record of another file
    /// ([`Conformer::resolve`]).
    fn resolves(&self, p: &str, walk: &mut Walk<'_>) -> bool {
        if !self.renamed.is_empty() && self.renamed.contains_key(p) {
            return false;
        }
        (self.family == Family::V7 && p == "@VOID@")
            || is_external_pointer(p, self.family)
            || self.names(p, ANY, walk)
    }

    /// Whether `p` names a record of type `want` ([`ANY`]: of any type),
    /// or will be checked to ([`Walk::pending`]).
    fn names(&self, p: &str, want: StructId, walk: &mut Walk<'_>) -> bool {
        match (XrefIndex::<()>::packed(p), walk.pending.as_deref_mut()) {
            (Some(key), Some(pending)) => {
                pending.push(Lookup {
                    key,
                    want,
                    record: walk.record,
                });
                true
            }
            _ => match self.xrefs.get(p) {
                Some(ty) => want == ANY || *ty == Some(want),
                None => false,
            },
        }
    }

    /// Whether each pending pointer names a record of its type; the records
    /// of those that do not, in `failed`.
    pub(super) fn look_up(&self, pending: &mut Vec<Lookup>, failed: &mut Vec<u32>) {
        for p in pending.drain(..) {
            let ok = match self.xrefs.get_packed(p.key) {
                Some(ty) => p.want == ANY || *ty == Some(p.want),
                None => false,
            };
            if !ok {
                failed.push(p.record);
            }
        }
    }

    /// Whether the payload of `n`, of type `ty`, stays as it is
    /// ([`Conformer::payload`]).
    fn clean_payload(&self, payload: PayloadRef<'_>, ty: StructId, walk: &mut Walk<'_>) -> bool {
        self.clean_payload_known(payload, ty, None, walk)
    }

    /// [`Conformer::clean_payload`], the [`special_bytes`] of a text
    /// payload known when `special` is.
    ///
    /// [`special_bytes`]: crate::writer::special_bytes
    fn clean_payload_known(
        &self,
        payload: PayloadRef<'_>,
        ty: StructId,
        special: Option<u8>,
        walk: &mut Walk<'_>,
    ) -> bool {
        let (kind, arg) = self.schema.kind(ty);
        let raw = match payload {
            p if matches!(kind, Kind::Pointer | Kind::NullablePointer) => {
                return self.clean_pointer(p, kind, arg, walk);
            }
            PayloadRef::Pointer(_) => return false,
            PayloadRef::Text(t) => t,
            PayloadRef::None => "",
        };
        let banned = match special {
            Some(special) => payload::has_banned_known(raw, special, self.rules),
            None => payload::has_banned(raw, self.rules),
        };
        if banned {
            return false;
        }
        // Any text without a banned character is a text.
        if matches!(kind, Kind::Text | Kind::ListText) {
            return true;
        }
        let set = matches!(kind, Kind::Enum | Kind::ListEnum)
            .then(|| self.schema.enum_set(arg))
            .flatten();
        match (kind, set) {
            (Kind::Enum, Some(set)) if !raw.is_empty() => {
                return is_canonical(set, raw, self.family);
            }
            (Kind::ListEnum, Some(set)) if !raw.is_empty() => {
                return is_canonical_list(set, raw, self.family);
            }
            _ => {}
        }
        let valid = if raw.is_empty() && payload::empty_is_valid(kind) {
            true
        } else if matches!(kind, Kind::Date | Kind::DateExact | Kind::DatePeriod) {
            payload::is_valid(kind, set, &self.aliases.unalias(raw), self.family)
        } else {
            payload::is_valid(kind, set, raw, self.family)
        };
        // The media type of a text is a text type.
        let text_type = || {
            raw.get(..5)
                .is_some_and(|p| p.eq_ignore_ascii_case("text/"))
        };
        let mime_of_text = self.family == Family::V7 && self.tables.mime.contains(&ty);
        valid && (!mime_of_text || raw.is_empty() || text_type())
    }

    /// Whether the payload of a pointer structure stays as it is: a pointer
    /// to a record of its type, `@VOID@` (7.x) or, in 5.5.1, a pointer to
    /// a substructure or another file; or none where none is permitted
    /// ([`Conformer::pointer_payload`]).
    fn clean_pointer(
        &self,
        payload: PayloadRef<'_>,
        kind: Kind,
        target: u16,
        walk: &mut Walk<'_>,
    ) -> bool {
        match payload {
            PayloadRef::Pointer(p) => {
                if !self.renamed.is_empty() && self.renamed.contains_key(p) {
                    return false;
                }
                if (self.family == Family::V7 && p == "@VOID@")
                    || is_external_pointer(p, self.family)
                {
                    return true;
                }
                self.names(p, target, walk)
            }
            PayloadRef::None => kind == Kind::NullablePointer,
            PayloadRef::Text(_) => false,
        }
    }
}

/// Whether an enumeration value is written as it is: it is its set's
/// spelling ([`super::canonical`] gives `value` back).
pub(super) fn is_canonical(set: &EnumSet, value: &str, family: Family) -> bool {
    match set.values.iter().find(|v| v.eq_ignore_ascii_case(value)) {
        Some(v) => *v == value,
        None => {
            set.open
                || family == Family::V7
                    && !value.bytes().any(|b| b.is_ascii_lowercase())
                    && payload::is_ext_tag(value)
        }
    }
}

/// Whether a list of enumeration values is written as it is: each item its
/// set's spelling, separated by `, ` ([`Conformer::list`]).
pub(super) fn is_canonical_list(set: &EnumSet, raw: &str, family: Family) -> bool {
    let mut rest = raw;
    let mut first = true;
    for item in payload::list_items(raw).filter(|i| !i.is_empty()) {
        if !is_canonical(set, item, family) {
            return false;
        }
        if !first {
            let Some(r) = rest.strip_prefix(", ") else {
                return false;
            };
            rest = r;
        }
        first = false;
        let Some(r) = rest.strip_prefix(item) else {
            return false;
        };
        rest = r;
    }
    !first && rest.is_empty()
}
