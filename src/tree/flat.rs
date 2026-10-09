//! Structures written into a flat arena, their texts borrowed where they
//! can be: what a typed model writes, one record at a time, for the writer
//! to check and emit without an owned [`Structure`] per line.
//!
//! A record of the typed model writing itself for the conformance check
//! types its structures as it goes ([`Flat::open_typed`], [`Typing`]):
//! when every structure is typed and placed, the check looks only at the
//! payloads that need it ([`Flat::checks`]) instead of walking the record.

use std::borrow::Cow;

use super::{Node, Payload, PayloadRef, Structure, Tag, Xref};
use crate::spec::conform::Typing;
use crate::spec::schema::{Kind, StructId, DATASET};
use crate::writer::special_bytes;

/// The type of a structure not typed ([`Flat::open`]).
const UNTYPED: StructId = StructId::MAX;

/// A payload in a [`Flat`] arena.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum FlatPayload<'s> {
    /// No payload.
    #[default]
    None,
    /// A pointer, delimiters included.
    Pointer(Cow<'s, str>),
    /// Text, unescaped, continuations joined.
    Text(Cow<'s, str>),
}

impl From<Payload> for FlatPayload<'_> {
    fn from(p: Payload) -> Self {
        match p {
            Payload::None => FlatPayload::None,
            Payload::Pointer(x) => FlatPayload::Pointer(Cow::Owned(x.as_str().to_string())),
            Payload::Text(t) => FlatPayload::Text(Cow::Owned(t.into_string())),
        }
    }
}

#[derive(Clone, Debug)]
struct FlatNode<'s> {
    tag: Cow<'s, str>,
    xref: Option<Cow<'s, str>>,
    payload: FlatPayload<'s>,
    /// Index just past this node's subtree.
    end: u32,
    /// The number of structures above it.
    depth: u32,
    /// The index of the tag among the standard tags, when it is one.
    standard: Option<u16>,
    /// Its structure type, when it was typed ([`Flat::open_typed`]).
    ty: StructId,
    /// The [`special_bytes`] of a text payload, found as it is written:
    /// the check and the emitter read them instead of the text.
    special: u8,
}

/// Structures in write order (depth first), each with the end of its
/// subtree. Cleared and refilled record after record, it keeps its room;
/// or records are written one after the other ([`Flat::begin`]), and kept.
#[derive(Clone, Default)]
pub(crate) struct Flat<'s> {
    nodes: Vec<FlatNode<'s>>,
    /// Where the record being written starts.
    start: usize,
    /// The structures open.
    depth: u32,
    /// The tables that type the structures as they are written, while a
    /// record writes itself for the conformance check.
    typing: Option<Typing>,
    /// Whether a structure of the record being written was not typed, or
    /// not placed where its type permits it.
    untyped: bool,
    /// The typed structures open, while typing.
    open: Vec<u32>,
    /// The next typed structure is one more of a field that holds several.
    repeat: bool,
    /// The typed structures whose payloads need a look.
    checks: Vec<u32>,
    /// An upper bound of the size of the typed structures as written in
    /// 5.5.1.
    size: usize,
    /// The depth of the extension structure being written under a typed
    /// one, while one is.
    extension: Option<u32>,
    /// The extension structures under typed ones, which the check walks.
    extensions: Vec<u32>,
}

impl std::fmt::Debug for Flat<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Flat")
            .field("nodes", &self.nodes)
            .field("typed", &self.typed())
            .finish_non_exhaustive()
    }
}

impl<'s> Flat<'s> {
    /// The number of structures.
    pub(crate) fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Room for `n` more structures.
    pub(crate) fn reserve(&mut self, n: usize) {
        self.nodes.reserve(n);
    }

    /// Empties the arena, keeping its room (and its typing).
    pub(crate) fn clear(&mut self) {
        self.nodes.clear();
        self.begin();
    }

    /// Starts a record after those written: where it starts.
    pub(crate) fn begin(&mut self) -> usize {
        self.start = self.nodes.len();
        self.depth = 0;
        self.untyped = false;
        self.open.clear();
        self.repeat = false;
        self.checks.clear();
        self.size = 0;
        self.extension = None;
        self.extensions.clear();
        self.start
    }

    /// Drops the records from `start` on.
    pub(crate) fn truncate(&mut self, start: usize) {
        self.nodes.truncate(start);
        self.begin();
    }

    /// Types the structures written from now on with `typing`, or not.
    pub(crate) fn set_typing(&mut self, typing: Option<Typing>) {
        self.typing = typing;
    }

    /// Whether every structure of the record being written was typed and
    /// placed where its type permits it, and has what its type requires:
    /// the conformance check then needs to look at [`Flat::checks`] and
    /// [`Flat::extensions`] only.
    pub(crate) fn typed(&self) -> bool {
        self.typing.is_some() && !self.untyped && self.nodes.len() > self.start
    }

    /// The type of the record being written, once typed.
    pub(crate) fn root_type(&self) -> Option<StructId> {
        self.nodes
            .get(self.start)
            .map(|n| n.ty)
            .filter(|&t| t != UNTYPED)
    }

    /// The payloads of the typed structures that need a look, with their
    /// types and [`special_bytes`].
    pub(crate) fn checks(&self) -> impl Iterator<Item = (PayloadRef<'_>, StructId, u8)> {
        self.checks.iter().filter_map(|&i| {
            let n = self.nodes.get(i as usize)?;
            Some((payload_ref(&n.payload), n.ty, n.special))
        })
    }

    /// The tags of [`Flat::checks`].
    #[cfg(test)]
    pub(crate) fn check_tags(&self) -> impl Iterator<Item = &str> {
        self.checks
            .iter()
            .filter_map(|&i| self.nodes.get(i as usize).map(|n| &*n.tag))
    }

    /// The extension structures written under typed ones (or as the
    /// record), which the check walks as it walks any extension.
    pub(crate) fn extensions(&self) -> impl Iterator<Item = FlatRef<'_>> {
        self.extensions.iter().map(|&index| FlatRef {
            nodes: &self.nodes,
            index,
        })
    }

    /// An upper bound of the size of the typed structures as written in
    /// 5.5.1.
    pub(crate) fn size(&self) -> usize {
        self.size
    }

    /// Something not typed is written: the check walks the record.
    pub(crate) fn untrusted(&mut self) {
        self.untyped = true;
    }

    /// Whether the next typed structure is one more of a field that holds
    /// several.
    pub(crate) fn repeat(&mut self, repeat: bool) {
        self.repeat = repeat;
    }

    /// Starts a typed structure: placed by its tag under the structure
    /// open ([`Typing::place`]), its payload trusted when `claim` is the
    /// kind of payload of its type (a typed value made valid for it) or
    /// left to the check.
    #[inline]
    pub(crate) fn open_typed(
        &mut self,
        tag: &'static str,
        standard: Option<u16>,
        xref: Option<Cow<'s, str>>,
        payload: FlatPayload<'s>,
        claim: Option<Kind>,
    ) -> usize {
        let repeat = std::mem::take(&mut self.repeat);
        let index = self.nodes.len();
        let mut ty = UNTYPED;
        if let (Some(typing), false) = (self.typing, self.untyped) {
            let parent = self.open.last().map(|&p| p as usize);
            let sup = match parent {
                Some(p) => self.nodes.get(p).map_or(UNTYPED, |n| n.ty),
                None => DATASET,
            };
            // Only a record has an identifier.
            let fits = sup != UNTYPED && (xref.is_none() || parent.is_none());
            let is_pointer = matches!(payload, FlatPayload::Pointer(_));
            // One more of a field that holds several: the occurrences of
            // its tag so far.
            let count = match (repeat, parent) {
                (true, Some(p)) => self.count_children(p, standard) + 1,
                _ => 1,
            };
            match fits
                .then(|| typing.place(sup, standard, is_pointer, count))
                .flatten()
            {
                Some(found) => {
                    ty = found;
                    if !typing.trusts(found, &payload, claim) {
                        self.checks.push(u32::try_from(index).unwrap_or(u32::MAX));
                    }
                    self.size += size_bound(tag, standard, xref.as_deref(), &payload);
                    self.open.push(u32::try_from(index).unwrap_or(u32::MAX));
                }
                None => self.untyped = true,
            }
        }
        self.push(Cow::Borrowed(tag), standard, xref, payload, ty)
    }

    /// A typed structure without substructures ([`Flat::open_typed`]).
    #[inline]
    pub(crate) fn leaf_typed(
        &mut self,
        tag: &'static str,
        standard: Option<u16>,
        payload: FlatPayload<'s>,
        claim: Option<Kind>,
    ) {
        let at = self.open_typed(tag, standard, None, payload, claim);
        self.close(at);
    }

    /// Starts a structure; its substructures follow until [`Flat::close`].
    pub(crate) fn open(
        &mut self,
        tag: Cow<'s, str>,
        xref: Option<Cow<'s, str>>,
        payload: FlatPayload<'s>,
    ) -> usize {
        let standard = super::standard_index(&tag);
        self.open_standard(tag, standard, xref, payload)
    }

    /// [`Flat::open`], the tag's standard index known (`standard`).
    pub(crate) fn open_standard(
        &mut self,
        tag: Cow<'s, str>,
        standard: Option<u16>,
        xref: Option<Cow<'s, str>>,
        payload: FlatPayload<'s>,
    ) -> usize {
        self.repeat = false;
        if let (Some(_), false) = (self.typing, self.untyped) {
            // An extension structure under a typed one (or as the record)
            // is left to the check; anything else untyped is not.
            if self.extension.is_none() {
                if tag.starts_with('_') {
                    self.extensions
                        .push(u32::try_from(self.nodes.len()).unwrap_or(u32::MAX));
                    self.extension = Some(self.depth);
                } else {
                    self.untyped = true;
                }
            }
        }
        self.push(tag, standard, xref, payload, UNTYPED)
    }

    /// The substructures with the standard tag `standard` of the structure
    /// at `parent`, written so far.
    fn count_children(&self, parent: usize, standard: Option<u16>) -> u32 {
        let mut n = 0;
        let mut i = parent + 1;
        while let Some(c) = self.nodes.get(i) {
            if c.standard == standard {
                n += 1;
            }
            i = (c.end as usize).max(i + 1);
        }
        n
    }

    #[inline]
    fn push(
        &mut self,
        tag: Cow<'s, str>,
        standard: Option<u16>,
        xref: Option<Cow<'s, str>>,
        payload: FlatPayload<'s>,
        ty: StructId,
    ) -> usize {
        let special = match &payload {
            FlatPayload::Text(t) => special_bytes(t),
            FlatPayload::None | FlatPayload::Pointer(_) => 0,
        };
        self.nodes.push(FlatNode {
            standard,
            tag,
            xref,
            payload,
            end: 0,
            depth: self.depth,
            ty,
            special,
        });
        self.depth += 1;
        self.nodes.len() - 1
    }

    /// Ends the structure [`Flat::open`] started at `at`.
    #[inline]
    pub(crate) fn close(&mut self, at: usize) {
        let end = u32::try_from(self.nodes.len()).unwrap_or(u32::MAX);
        if let Some(node) = self.nodes.get_mut(at) {
            node.end = end;
        }
        self.depth = self.depth.saturating_sub(1);
        if let (Some(typing), false) = (self.typing, self.untyped) {
            if let Some(depth) = self.extension {
                if depth == self.depth {
                    self.extension = None;
                }
                return;
            }
            self.open.pop();
            if !self.has_required(&typing, at) {
                self.untyped = true;
            }
        }
    }

    /// Whether the typed structure at `at`, complete, has the
    /// substructures its type requires and, in 7.x, a payload or a
    /// substructure ([`Conformer::clean`](crate::spec::conform)).
    fn has_required(&self, typing: &Typing, at: usize) -> bool {
        let Some(node) = self.nodes.get(at) else {
            return false;
        };
        let children = || {
            FlatRef {
                nodes: &self.nodes,
                index: u32::try_from(at).unwrap_or(u32::MAX),
            }
            .children()
        };
        for (tag, min) in typing.required(node.ty) {
            let n = children()
                .filter(|c| {
                    c.standard_tag()
                        .and_then(|s| typing.spec_tag(s))
                        .is_some_and(|t| t == tag)
                })
                .count();
            if n < usize::from(min) {
                return false;
            }
        }
        if typing.v7() {
            let empty = match &node.payload {
                FlatPayload::None => true,
                FlatPayload::Pointer(_) => false,
                FlatPayload::Text(t) => t.is_empty(),
            };
            let record = self.open.is_empty();
            if !record && empty && node.end as usize == at + 1 {
                return false;
            }
            if typing.is_note_tran(node.ty)
                && !children().any(|c| matches!(c.tag(), "MIME" | "LANG"))
            {
                return false;
            }
        }
        true
    }

    /// An owned structure and its substructures.
    pub(crate) fn push_structure(&mut self, s: Structure) {
        let tag = match s.tag.standard_index() {
            Some(_) => Cow::Borrowed(tag_text(&s.tag)),
            None => Cow::Owned(s.tag.as_str().to_string()),
        };
        let at = self.open(
            tag,
            s.xref.map(|x| Cow::Owned(x.as_str().to_string())),
            s.payload.into(),
        );
        for c in s.substructures {
            self.push_structure(c);
        }
        self.close(at);
    }

    /// The record being written (or the last one), with its
    /// substructures.
    pub(crate) fn root(&self) -> Option<FlatRef<'_>> {
        self.root_at(self.start)
    }

    /// The record written from `start`.
    pub(crate) fn root_at(&self, start: impl TryInto<u32>) -> Option<FlatRef<'_>> {
        let index = start.try_into().ok()?;
        self.nodes.get(index as usize).map(|_| FlatRef {
            nodes: &self.nodes,
            index,
        })
    }
}

/// The text of a standard tag, for as long as the program runs.
fn tag_text(tag: &Tag) -> &'static str {
    tag.standard_index()
        .and_then(|i| super::STANDARD_TAGS.get(usize::from(i)))
        .copied()
        .unwrap_or("")
}

/// An upper bound of what a structure adds to the size of its record as
/// written in 5.5.1, as the conformance check bounds it: a standard tag has
/// at most six letters.
fn size_bound(
    tag: &str,
    standard: Option<u16>,
    xref: Option<&str>,
    payload: &FlatPayload<'_>,
) -> usize {
    let tag = standard.map_or(tag.len(), |_| 6);
    let payload = match payload {
        FlatPayload::None => 0,
        FlatPayload::Pointer(p) | FlatPayload::Text(p) => p.len(),
    };
    8 + tag + xref.map_or(0, str::len) + 13 * payload
}

/// A payload, borrowed.
#[inline]
fn payload_ref<'a>(payload: &'a FlatPayload<'_>) -> PayloadRef<'a> {
    match payload {
        FlatPayload::Pointer(p) => PayloadRef::Pointer(p),
        FlatPayload::Text(t) => PayloadRef::Text(t),
        FlatPayload::None => PayloadRef::None,
    }
}

/// A structure of a [`Flat`] arena.
#[derive(Clone, Copy)]
pub(crate) struct FlatRef<'a> {
    nodes: &'a [FlatNode<'a>],
    index: u32,
}

impl<'a> FlatRef<'a> {
    #[inline]
    fn node(self) -> Option<&'a FlatNode<'a>> {
        self.nodes.get(self.index as usize)
    }

    /// The [`special_bytes`] of the text payload.
    pub(crate) fn special(self) -> Option<u8> {
        self.node().map(|n| n.special)
    }

    /// The structure and its substructures, in write order, with their
    /// depth below it.
    pub(crate) fn preorder(self) -> impl Iterator<Item = (usize, FlatRef<'a>)> {
        let nodes = self.nodes;
        let (end, base) = self
            .node()
            .map_or((0, 0), |n| (n.end.max(self.index + 1), n.depth));
        (self.index..end).filter_map(move |index| {
            let depth = nodes.get(index as usize)?.depth.checked_sub(base)?;
            Some((depth as usize, FlatRef { nodes, index }))
        })
    }
}

/// The substructures of a [`FlatRef`].
#[derive(Clone)]
pub(crate) struct FlatChildren<'a> {
    nodes: &'a [FlatNode<'a>],
    next: u32,
    end: u32,
}

impl<'a> Iterator for FlatChildren<'a> {
    type Item = FlatRef<'a>;

    #[inline]
    fn next(&mut self) -> Option<FlatRef<'a>> {
        if self.next >= self.end {
            return None;
        }
        let index = self.next;
        let end = self.nodes.get(index as usize).map_or(index + 1, |n| n.end);
        self.next = end.max(index + 1);
        Some(FlatRef {
            nodes: self.nodes,
            index,
        })
    }
}

impl<'a> Node<'a> for FlatRef<'a> {
    type Children = FlatChildren<'a>;

    #[inline]
    fn tag(self) -> &'a str {
        self.node().map_or("", |n| &n.tag)
    }

    #[inline]
    fn standard_tag(self) -> Option<u16> {
        self.node()?.standard
    }

    #[inline]
    fn xref(self) -> Option<&'a str> {
        self.node()?.xref.as_deref()
    }

    #[inline]
    fn payload(self) -> PayloadRef<'a> {
        self.node()
            .map_or(PayloadRef::None, |n| payload_ref(&n.payload))
    }

    fn line(self) -> u32 {
        0
    }

    fn children(self) -> FlatChildren<'a> {
        FlatChildren {
            nodes: self.nodes,
            next: self.index + 1,
            end: self.node().map_or(0, |n| n.end),
        }
    }

    fn to_owned_structure(self) -> Structure {
        Structure {
            tag: Tag::new(self.tag()),
            xref: self.xref().map(Xref::new),
            payload: self.payload().to_payload(),
            substructures: self.children().map(Node::to_owned_structure).collect(),
            line: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_arena_reads_as_its_structures() {
        let tree = crate::tree::parse_tree(
            "0 @I1@ INDI\n1 NAME A /B/\n2 GIVN A\n1 FAMS @F1@\n1 _X\n2 _Y y\n0 TRLR\n",
        );
        let indi = tree.to_structures().remove(0);
        let mut flat = Flat::default();
        flat.push_structure(indi.clone());
        let root = flat.root().unwrap();
        assert_eq!(root.to_owned_structure(), indi);
        assert_eq!(root.children().count(), 3);
        assert_eq!(root.xref(), Some("@I1@"));
        let fams = root.children().nth(1).unwrap();
        assert_eq!(fams.payload(), PayloadRef::Pointer("@F1@"));
        let levels: Vec<(usize, &str)> = root.preorder().map(|(l, n)| (l, n.tag())).collect();
        assert_eq!(
            levels,
            [
                (0, "INDI"),
                (1, "NAME"),
                (2, "GIVN"),
                (1, "FAMS"),
                (1, "_X"),
                (2, "_Y")
            ]
        );
        flat.clear();
        assert!(flat.root().is_none());
    }
}
