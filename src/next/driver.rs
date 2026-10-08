//! The generic driver behind every typed structure.
//!
//! [`gedcom_struct!`](super::gedcom_struct) declares a structure: its
//! payload, its substructures by tag and its field types. The field types
//! give the cardinality through [`Slot`]: an `Option` holds one occurrence
//! (a second one goes to `extra`), a `Vec` or a
//! [`ThinVec`](super::ThinVec) any number. The macro generates the struct,
//! its field table and a few short methods ([`Fields`]); everything else is
//! done here, once for all structures — the loops below have one copy, which
//! every type calls through `dyn Fields`:
//!
//! - [`read`] fills a structure from a node of the record arena: the payload
//!   through [`PayloadField`], each substructure through the field its tag
//!   selects (a table of the standard tags, computed at compile time), and
//!   whatever no field takes — an unknown or extension tag, a repeated
//!   singleton, a substructure whose shape its field cannot hold — into
//!   `extra`, in order. Reading never fails: a node that does not fit a
//!   type is refused ([`FromNode`] returns `None`) and its superstructure
//!   keeps it whole in its own `extra`.
//! - [`write`] turns a structure back into a [`Structure`]: payload, fields
//!   in declaration order, then `extra`.

use std::borrow::Cow;

use crate::tree::{
    unescape_spans, Escaping, Flat, FlatPayload, Payload, RawKind, RawNode, RawSpan, Structure,
    Tag, TextPiece, Xref, STANDARD_TAGS,
};
use crate::version::GedcomVersion;

use super::list::ThinVec;
use super::node::{Extra, Node, Value};
use super::text::{Store, TagId, Text, XrefId, XrefTable};

/// The arena of the record being read: the builder's nodes, identifiers
/// and joined pieces, with offsets relative to `text`, which starts at
/// `base` in the store's input.
#[derive(Clone, Copy)]
pub(crate) struct Arena<'a> {
    /// The text the builder read (a segment of the store's input).
    pub text: &'a str,
    /// The offset of `text` in the store's input.
    pub base: usize,
    pub joined: &'a [TextPiece],
    pub nodes: &'a [RawNode],
    pub xrefs: &'a [(u32, RawSpan)],
}

/// A node of the record arena, with its entry.
#[derive(Clone, Copy)]
pub(crate) struct NodeRef<'a> {
    arena: &'a Arena<'a>,
    raw: RawNode,
    index: u32,
}

impl<'a> NodeRef<'a> {
    /// The record at the start of the arena.
    pub(crate) fn root(arena: &'a Arena<'a>) -> Option<Self> {
        Self::at(arena, 0)
    }

    fn at(arena: &'a Arena<'a>, index: u32) -> Option<Self> {
        let raw = *arena.nodes.get(index as usize)?;
        Some(Self { arena, raw, index })
    }

    fn raw(self) -> RawNode {
        self.raw
    }

    /// The tag's identifier: a standard tag's index, or past the table.
    pub(crate) fn tag_id(self) -> u32 {
        self.raw().tag
    }

    /// Whether the node has substructures.
    pub(crate) fn has_children(self) -> bool {
        self.raw().end > self.index + 1
    }

    /// Whether the node holds an identifier.
    pub(crate) fn has_xref(self) -> bool {
        self.raw().has_xref
    }

    /// Whether the payload is a pointer.
    pub(crate) fn is_pointer(self) -> bool {
        self.raw().kind == RawKind::Pointer
    }

    /// Whether there is no payload.
    pub(crate) fn has_no_payload(self) -> bool {
        self.raw().kind == RawKind::None
    }

    /// The payload's characters: the pointer or the text, unescaped and
    /// joined (borrowed unless joined from several lines).
    pub(crate) fn payload_str(self, escaping: Escaping) -> Cow<'a, str> {
        let raw = self.raw();
        let text = self.arena.text;
        match raw.kind {
            RawKind::None | RawKind::Side => Cow::Borrowed(""),
            RawKind::Pointer | RawKind::Text => Cow::Borrowed(raw.payload.get(text)),
            RawKind::Escaped => {
                let raw = raw.payload.get(text);
                let mut out = String::with_capacity(raw.len());
                crate::tree::unescape_into(raw, escaping, &mut out);
                Cow::Owned(out)
            }
            RawKind::Joined => {
                let mut out = String::new();
                for piece in self.joined(raw) {
                    if piece.newline() {
                        out.push('\n');
                    }
                    let start = piece.start() as usize;
                    out.push_str(
                        text.get(start..start + piece.len() as usize)
                            .unwrap_or_default(),
                    );
                }
                Cow::Owned(out)
            }
        }
    }

    fn joined(self, raw: RawNode) -> &'a [TextPiece] {
        let start = raw.payload.start as usize;
        self.arena
            .joined
            .get(start..start + raw.payload.len as usize)
            .unwrap_or_default()
    }

    /// The substructures, in order.
    pub(crate) fn children(self) -> Children<'a> {
        let raw = self.raw();
        Children {
            arena: self.arena,
            next: self.index + 1,
            end: raw.end,
        }
    }

    fn xref_span(self) -> Option<RawSpan> {
        if !self.raw().has_xref {
            return None;
        }
        let at = self
            .arena
            .xrefs
            .binary_search_by_key(&self.index, |&(n, _)| n)
            .ok()?;
        self.arena.xrefs.get(at).map(|&(_, span)| span)
    }
}

/// The substructures of a [`NodeRef`].
pub(crate) struct Children<'a> {
    arena: &'a Arena<'a>,
    next: u32,
    end: u32,
}

impl<'a> Iterator for Children<'a> {
    type Item = NodeRef<'a>;

    fn next(&mut self) -> Option<NodeRef<'a>> {
        if self.next >= self.end {
            return None;
        }
        let child = NodeRef::at(self.arena, self.next)?;
        self.next = child.raw.end.max(self.next + 1);
        Some(child)
    }
}

/// What reading needs besides the node: the store's tables, being filled,
/// how the file escapes `@` and the version it declares.
pub(crate) struct ReadCx<'s> {
    input: &'s str,
    pieces: &'s mut Vec<TextPiece>,
    xrefs: &'s mut XrefTable,
    escaping: Escaping,
    /// The version of the file.
    pub(crate) version: GedcomVersion,
}

impl<'s> ReadCx<'s> {
    pub(crate) fn new(
        input: &'s str,
        pieces: &'s mut Vec<TextPiece>,
        xrefs: &'s mut XrefTable,
        version: GedcomVersion,
    ) -> Self {
        Self {
            input,
            pieces,
            xrefs,
            escaping: if version.is_v7() {
                Escaping::V70
            } else {
                Escaping::V551
            },
            version,
        }
    }

    /// The payload's characters (see [`NodeRef::payload_str`]).
    pub(crate) fn payload_str<'a>(&self, node: NodeRef<'a>) -> Cow<'a, str> {
        node.payload_str(self.escaping)
    }

    /// The text payload of a node (empty when it has none); `None` for a
    /// pointer. A payload reading rewrote is pieces of the input, not a
    /// copy.
    pub(crate) fn text(&mut self, node: NodeRef<'_>) -> Option<Text> {
        let raw = node.raw();
        let base = node.arena.base;
        match raw.kind {
            RawKind::None => Some(Text::default()),
            RawKind::Pointer => None,
            RawKind::Text => Some(Text::input(
                self.input,
                base + raw.payload.start as usize,
                raw.payload.len as usize,
            )),
            RawKind::Escaped => {
                let at = self.pieces.len();
                let start = base + raw.payload.start as usize;
                let mut fits = true;
                let pieces = &mut *self.pieces;
                unescape_spans(raw.payload.get(node.arena.text), self.escaping, |s, len| {
                    if len > 0 {
                        match (u32::try_from(start + s), u32::try_from(len)) {
                            (Ok(s), Ok(len)) => pieces.push(TextPiece::new(s, len, false)),
                            _ => fits = false,
                        }
                    }
                });
                Some(self.finish_pieces(at, fits, node))
            }
            RawKind::Joined => {
                let at = self.pieces.len();
                let shift = u32::try_from(base).ok();
                let mut fits = shift.is_some();
                for piece in node.joined(raw) {
                    let end = shift.and_then(|b| b.checked_add(piece.start() + piece.len()));
                    fits &= end.is_some();
                    self.pieces.push(piece.shifted(shift.unwrap_or(0)));
                }
                Some(self.finish_pieces(at, fits, node))
            }
            // Only a builder outside span mode makes these.
            RawKind::Side => Some(Text::new(&*self.payload_str(node))),
        }
    }

    /// The text of the pieces pushed from `at`; an owned copy when their
    /// offsets do not fit in 32 bits.
    fn finish_pieces(&mut self, at: usize, fits: bool, node: NodeRef<'_>) -> Text {
        if fits {
            Text::from_pieces(self.input, self.pieces, at)
        } else {
            self.pieces.truncate(at);
            Text::new(&*self.payload_str(node))
        }
    }

    /// The pointer payload of a node, interned; `None` when the payload is
    /// not a pointer.
    pub(crate) fn pointer(&mut self, node: NodeRef<'_>) -> Option<XrefId> {
        let raw = node.raw();
        (raw.kind == RawKind::Pointer)
            .then(|| self.intern(node.arena, raw.payload))
            .flatten()
    }

    /// The identifier of a node, interned.
    pub(crate) fn xref(&mut self, node: NodeRef<'_>) -> Option<XrefId> {
        let span = node.xref_span()?;
        self.intern(node.arena, span)
    }

    fn intern(&mut self, arena: &Arena<'_>, span: RawSpan) -> Option<XrefId> {
        self.xrefs.intern(span.get(arena.text))
    }

    /// A node without its substructures, untyped.
    pub(crate) fn node_shallow(&mut self, node: NodeRef<'_>) -> Node {
        let payload = match self.pointer(node) {
            Some(id) => Value::Pointer(id),
            None if node.is_pointer() => Value::Text(self.text_of_pointer(node)),
            None => match self.text(node) {
                Some(t) if !t.is_empty() => Value::Text(t),
                _ => Value::None,
            },
        };
        Node {
            tag: TagId::raw(node.tag_id()),
            xref: self.xref(node),
            payload,
            children: Vec::new(),
        }
    }

    /// A node and its substructures, untyped.
    pub(crate) fn node(&mut self, node: NodeRef<'_>) -> Node {
        let mut n = self.node_shallow(node);
        let count = node.children().count();
        if count > 0 {
            n.children = Vec::with_capacity(count);
            for child in node.children() {
                let c = self.node(child);
                n.children.push(c);
            }
        }
        n
    }

    /// A pointer that could not be interned (four billion identifiers), as
    /// text.
    fn text_of_pointer(&mut self, node: NodeRef<'_>) -> Text {
        let raw = node.raw();
        Text::input(
            self.input,
            node.arena.base + raw.payload.start as usize,
            raw.payload.len as usize,
        )
    }
}

/// What writing needs: the store the model points into and the target
/// version.
#[derive(Clone, Copy)]
pub(crate) struct WriteCx<'s> {
    pub(crate) store: &'s Store,
    pub(crate) version: GedcomVersion,
    /// Whether values are converted to `version`'s grammars (dates, ages,
    /// times), as for writing a file, or given as the model holds them.
    pub(crate) convert: bool,
}

impl<'a> WriteCx<'a> {
    /// A text payload; none when empty.
    pub(crate) fn text(&self, text: &Text) -> Payload {
        if text.is_empty() {
            Payload::None
        } else {
            Payload::Text(text.to_str(self.store).into())
        }
    }

    /// A pointer payload.
    pub(crate) fn pointer(&self, id: XrefId) -> Payload {
        Payload::Pointer(Xref::new(self.store.xref(id)))
    }

    /// [`WriteCx::text`], borrowed.
    pub(crate) fn text_flat<'s>(&self, text: &'s Text) -> FlatPayload<'s>
    where
        'a: 's,
    {
        if text.is_empty() {
            FlatPayload::None
        } else {
            FlatPayload::Text(text.to_str(self.store))
        }
    }

    /// [`WriteCx::pointer`], borrowed.
    pub(crate) fn pointer_flat(&self, id: XrefId) -> FlatPayload<'a> {
        FlatPayload::Pointer(Cow::Borrowed(self.store.xref(id)))
    }

    /// [`WriteCx::str`] of static characters, borrowed.
    pub(crate) fn static_flat(s: &'static str) -> FlatPayload<'static> {
        if s.is_empty() {
            FlatPayload::None
        } else {
            FlatPayload::Text(Cow::Borrowed(s))
        }
    }

    /// A text payload from static or computed characters.
    pub(crate) fn str(s: &str) -> Payload {
        if s.is_empty() {
            Payload::None
        } else {
            Payload::Text(s.into())
        }
    }
}

/// A type read from a node and its substructures.
pub(crate) trait FromNode: Sized {
    /// Reads `node`; `None` when its shape does not fit the type (its
    /// superstructure then keeps it whole in `extra`).
    fn from_node(node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> Option<Self>;
}

/// A type written as one structure.
pub(crate) trait ToNodes {
    /// The structure, tagged `tag` unless the type chooses its own tag.
    fn to_node(&self, tag: &'static str, cx: &WriteCx<'_>) -> Structure;

    /// The same structure, written into `out` with its texts borrowed from
    /// the model and its source where they can be: what the writer checks
    /// and emits. By default, [`ToNodes::to_node`]'s structure.
    fn to_flat<'s>(&'s self, tag: &'static str, cx: &WriteCx<'s>, out: &mut Flat<'s>) {
        out.push_structure(self.to_node(tag, cx));
    }
}

/// The payload of a structure.
pub(crate) trait PayloadField: Sized {
    /// Reads the payload of `node`; `None` when it does not fit.
    fn read(node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> Option<Self>;
    /// The payload to write.
    fn write(&self, cx: &WriteCx<'_>) -> Payload;

    /// The same payload, borrowed where it can be. By default,
    /// [`PayloadField::write`]'s.
    fn payload<'s>(&'s self, cx: &WriteCx<'s>) -> FlatPayload<'s> {
        self.write(cx).into()
    }
}

impl PayloadField for Text {
    fn read(node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> Option<Self> {
        cx.text(node)
    }

    fn write(&self, cx: &WriteCx<'_>) -> Payload {
        cx.text(self)
    }

    fn payload<'s>(&'s self, cx: &WriteCx<'s>) -> FlatPayload<'s> {
        cx.text_flat(self)
    }
}

impl PayloadField for Option<XrefId> {
    fn read(node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> Option<Self> {
        if node.has_no_payload() {
            Some(None)
        } else {
            cx.pointer(node).map(Some)
        }
    }

    fn write(&self, cx: &WriteCx<'_>) -> Payload {
        self.map_or(Payload::None, |id| cx.pointer(id))
    }

    fn payload<'s>(&'s self, cx: &WriteCx<'s>) -> FlatPayload<'s> {
        self.map_or(FlatPayload::None, |id| cx.pointer_flat(id))
    }
}

/// A field: how many occurrences of its substructure it holds.
pub(crate) trait Slot {
    /// Whether the field holds any number of occurrences.
    const MANY: bool;
    /// Takes an occurrence; `false` when the field is full or the node does
    /// not fit (the node then goes to `extra`).
    fn accept(&mut self, node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> bool;
    /// Writes the occurrences.
    fn write(&self, tag: &'static str, cx: &WriteCx<'_>, out: &mut Vec<Structure>);
    /// Writes the occurrences into a flat arena ([`ToNodes::to_flat`]).
    fn to_flat<'s>(&'s self, tag: &'static str, cx: &WriteCx<'s>, out: &mut Flat<'s>) {
        let mut owned = Vec::new();
        self.write(tag, cx, &mut owned);
        for s in owned {
            out.push_structure(s);
        }
    }
    /// Releases spare capacity once the structure is read.
    fn finish(&mut self) {}
}

impl<T: FromNode + ToNodes> Slot for Option<T> {
    const MANY: bool = false;

    fn accept(&mut self, node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> bool {
        if self.is_some() {
            return false;
        }
        *self = T::from_node(node, cx);
        self.is_some()
    }

    fn write(&self, tag: &'static str, cx: &WriteCx<'_>, out: &mut Vec<Structure>) {
        if let Some(v) = self {
            out.push(v.to_node(tag, cx));
        }
    }

    fn to_flat<'s>(&'s self, tag: &'static str, cx: &WriteCx<'s>, out: &mut Flat<'s>) {
        if let Some(v) = self {
            v.to_flat(tag, cx, out);
        }
    }
}

/// Pushes onto a list that is usually short and is kept: short lists grow
/// one item at a time, so that they need no shrinking (a reallocation)
/// once read; longer ones double.
pub(crate) fn push_exact<T>(list: &mut Vec<T>, item: T) {
    if list.len() == list.capacity() {
        if list.len() < 8 {
            list.reserve_exact(1);
        } else {
            list.reserve(list.len());
        }
    }
    list.push(item);
}

impl<T: FromNode + ToNodes> Slot for Vec<T> {
    const MANY: bool = true;

    fn accept(&mut self, node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> bool {
        match T::from_node(node, cx) {
            Some(v) => {
                push_exact(self, v);
                true
            }
            None => false,
        }
    }

    fn write(&self, tag: &'static str, cx: &WriteCx<'_>, out: &mut Vec<Structure>) {
        for v in self {
            out.push(v.to_node(tag, cx));
        }
    }

    fn to_flat<'s>(&'s self, tag: &'static str, cx: &WriteCx<'s>, out: &mut Flat<'s>) {
        for v in self {
            v.to_flat(tag, cx, out);
        }
    }

    fn finish(&mut self) {
        if self.capacity() > self.len() {
            self.shrink_to_fit();
        }
    }
}

impl<T: FromNode + ToNodes> Slot for ThinVec<T> {
    const MANY: bool = true;

    fn accept(&mut self, node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> bool {
        match T::from_node(node, cx) {
            Some(v) => {
                self.push(v);
                true
            }
            None => false,
        }
    }

    fn write(&self, tag: &'static str, cx: &WriteCx<'_>, out: &mut Vec<Structure>) {
        for v in self {
            out.push(v.to_node(tag, cx));
        }
    }

    fn to_flat<'s>(&'s self, tag: &'static str, cx: &WriteCx<'s>, out: &mut Flat<'s>) {
        for v in self {
            v.to_flat(tag, cx, out);
        }
    }

    fn finish(&mut self) {
        self.shrink();
    }
}

impl<T: FromNode> FromNode for Box<T> {
    fn from_node(node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> Option<Self> {
        T::from_node(node, cx).map(Box::new)
    }
}

impl<T: ToNodes> ToNodes for Box<T> {
    fn to_node(&self, tag: &'static str, cx: &WriteCx<'_>) -> Structure {
        (**self).to_node(tag, cx)
    }

    fn to_flat<'s>(&'s self, tag: &'static str, cx: &WriteCx<'s>, out: &mut Flat<'s>) {
        (**self).to_flat(tag, cx, out);
    }
}

/// A leaf: a payload without substructures. A node with substructures does
/// not fit a leaf, and goes to `extra` whole.
pub(crate) fn read_leaf<T: PayloadField>(node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> Option<T> {
    if node.has_children() || node.has_xref() {
        return None;
    }
    T::read(node, cx)
}

/// A leaf's structure.
pub(crate) fn write_leaf<T: PayloadField>(
    value: &T,
    tag: &'static str,
    cx: &WriteCx<'_>,
) -> Structure {
    leaf_structure(tag, value.write(cx))
}

/// A leaf's structure, into a flat arena.
pub(crate) fn write_leaf_flat<'s, T: PayloadField>(
    value: &'s T,
    tag: &'static str,
    cx: &WriteCx<'s>,
    out: &mut Flat<'s>,
) {
    out.leaf(Cow::Borrowed(tag), value.payload(cx));
}

/// A structure with a tag and a payload.
fn leaf_structure(tag: &'static str, payload: Payload) -> Structure {
    Structure {
        tag: Tag::new(tag),
        payload,
        ..Structure::default()
    }
}

/// Implements [`FromNode`] and [`ToNodes`] for leaves.
macro_rules! leaf {
    ($($ty:ty),* $(,)?) => {$(
        impl $crate::next::driver::FromNode for $ty {
            fn from_node(
                node: $crate::next::driver::NodeRef<'_>,
                cx: &mut $crate::next::driver::ReadCx<'_>,
            ) -> Option<Self> {
                $crate::next::driver::read_leaf(node, cx)
            }
        }

        impl $crate::next::driver::ToNodes for $ty {
            fn to_node(
                &self,
                tag: &'static str,
                cx: &$crate::next::driver::WriteCx<'_>,
            ) -> $crate::tree::Structure {
                $crate::next::driver::write_leaf(self, tag, cx)
            }

            fn to_flat<'s>(
                &'s self,
                tag: &'static str,
                cx: &$crate::next::driver::WriteCx<'s>,
                out: &mut $crate::tree::Flat<'s>,
            ) {
                $crate::next::driver::write_leaf_flat(self, tag, cx, out);
            }
        }
    )*};
}
pub(crate) use leaf;

/// A pointer leaf (`ALIA @I1@`, `SNOTE @N1@`): text in its place does not
/// fit.
impl PayloadField for XrefId {
    fn read(node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> Option<Self> {
        cx.pointer(node)
    }

    fn write(&self, cx: &WriteCx<'_>) -> Payload {
        cx.pointer(*self)
    }

    fn payload<'s>(&'s self, cx: &WriteCx<'s>) -> FlatPayload<'s> {
        cx.pointer_flat(*self)
    }
}

/// An integer (`HEIGHT 100`, `NCHI 3`): digits only, at most `u32::MAX`;
/// anything else does not fit and is kept as it is, in `extra`.
impl PayloadField for u32 {
    fn read(node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> Option<Self> {
        let s = cx.payload_str(node);
        if node.is_pointer() || s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse().ok()
    }

    fn write(&self, _cx: &WriteCx<'_>) -> Payload {
        Payload::Text(self.to_string().into())
    }
}

leaf!(Text, XrefId, u32);

/// The names of the structure types of the specifications a type stands
/// for, per version (7.x: the URI without its prefix; 5.5.1: the context
/// path of the tables).
#[derive(Clone, Copy, Debug)]
pub struct SpecNames {
    /// GEDCOM 5.5.1.
    pub v551: &'static [&'static str],
    /// GEDCOM 7.0.
    pub v70: &'static [&'static str],
    /// GEDCOM 7.1.
    pub v71: &'static [&'static str],
}

/// A field of a typed structure.
#[derive(Clone, Copy, Debug)]
pub struct FieldDesc {
    /// The tags it takes; the first is the one written.
    pub tags: &'static [&'static str],
    /// The field's name.
    pub name: &'static str,
    /// Whether it holds any number of occurrences (`Vec`) rather than one
    /// (`Option`).
    pub many: bool,
}

/// For each standard tag, the field that takes it (its index plus one), or
/// 0.
pub(crate) type Lookup = [u8; STANDARD_TAGS.len()];

/// The lookup table of a list of fields, at compile time. A field tag that
/// is not standard is a compile error.
// Constant functions can only use `as`; indices are below 256 (checked).
#[allow(clippy::cast_possible_truncation)]
pub(crate) const fn lookup(fields: &[FieldDesc]) -> Lookup {
    assert!(fields.len() < 255, "too many fields");
    let mut table = [0_u8; STANDARD_TAGS.len()];
    let mut f = 0;
    while f < fields.len() {
        let mut t = 0;
        while t < fields[f].tags.len() {
            let index = crate::tree::standard_index_const(fields[f].tags[t]);
            assert!(index.is_some(), "a field tag is not a standard tag");
            if let Some(i) = index {
                table[i as usize] = f as u8 + 1;
            }
            t += 1;
        }
        f += 1;
    }
    table
}

/// What the driver reads and writes of a typed structure. It is
/// dyn-compatible, so that the driver's loops have one copy for every type.
pub(crate) trait Fields {
    /// The field of each standard tag.
    fn lookup(&self) -> &'static Lookup;
    /// Reads the payload; `false` when the node's payload does not fit.
    fn read_payload(&mut self, node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> bool;
    /// Gives a substructure to field `field`; `false` when it does not
    /// take it.
    fn accept(&mut self, field: usize, node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> bool;
    /// Releases spare capacity.
    fn finish(&mut self);
    /// The substructures no field holds.
    fn extra(&self) -> &Extra;
    /// The same, to fill.
    fn extra_mut(&mut self) -> &mut Extra;
    /// The payload to write.
    fn write_payload(&self, cx: &WriteCx<'_>) -> Payload;
    /// The fields' structures, in order.
    fn write_fields(&self, cx: &WriteCx<'_>, out: &mut Vec<Structure>);
    /// [`Fields::write_payload`], borrowed where it can be.
    fn write_payload_flat<'s>(&'s self, cx: &WriteCx<'s>) -> FlatPayload<'s> {
        self.write_payload(cx).into()
    }
    /// [`Fields::write_fields`], into a flat arena.
    fn write_fields_flat<'s>(&'s self, cx: &WriteCx<'s>, out: &mut Flat<'s>) {
        let mut owned = Vec::new();
        self.write_fields(cx, &mut owned);
        for s in owned {
            out.push_structure(s);
        }
    }
}

/// A typed structure, as `gedcom_struct!` declares it: its fields and the
/// structure types it stands for.
pub(crate) trait Struct: Fields + Default {
    /// The fields, in declaration (and writing) order.
    const FIELDS: &'static [FieldDesc];
    /// The field of each standard tag.
    const LOOKUP: Lookup;
    /// The structure types of the specifications this type stands for.
    const SPEC: SpecNames;
}

/// Reads a typed structure from `node`.
pub(crate) fn read<T: Struct>(node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> Option<T> {
    let mut s = T::default();
    read_into(&mut s, node, cx).then_some(s)
}

/// Fills a structure from `node`: its payload, then each substructure into
/// the field its tag selects, or into `extra`. `false` when the node does
/// not fit.
fn read_into(s: &mut dyn Fields, node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> bool {
    // An identifier on a substructure (a 5.5.1 pointer may name it) has no
    // field: the node stays untyped, with it.
    if node.has_xref() || !s.read_payload(node, cx) {
        return false;
    }
    let lookup = s.lookup();
    for child in node.children() {
        let field = lookup.get(child.tag_id() as usize).copied().unwrap_or(0);
        if field == 0 || !s.accept(usize::from(field - 1), child, cx) {
            let n = cx.node(child);
            s.extra_mut().push(n);
        }
    }
    s.finish();
    true
}

/// [`write`] into a flat arena.
pub(crate) fn write_flat<'s>(
    s: &'s dyn Fields,
    tag: &'static str,
    cx: &WriteCx<'s>,
    out: &mut Flat<'s>,
) {
    let at = out.open(Cow::Borrowed(tag), None, s.write_payload_flat(cx));
    s.write_fields_flat(cx, out);
    s.extra().write_flat(cx, out);
    out.close(at);
}

/// Writes a typed structure as `tag`: its payload, its fields in order,
/// then `extra`.
pub(crate) fn write(s: &dyn Fields, tag: &'static str, cx: &WriteCx<'_>) -> Structure {
    let mut substructures = Vec::new();
    s.write_fields(cx, &mut substructures);
    s.extra().write(cx, &mut substructures);
    Structure {
        tag: Tag::new(tag),
        payload: s.write_payload(cx),
        substructures,
        ..Structure::default()
    }
}

/// Declares a typed structure: the struct, its field table and its
/// [`FromNode`] and [`ToNodes`] implementations, which call [`read`] and
/// [`write`].
///
/// ```text
/// gedcom_struct! {
///     /// Docs.
///     pub struct Name {
///         @payload /// Docs. value: Text;          // optional
///         /// Docs.
///         "TAG" | "ALT" => field: Option<Type>,    // or Vec<Type>
///     }
///     spec { v551: ["…"], v70: ["…"], v71: ["…"] }
/// }
/// ```
///
/// A `[tag = path]` before the fields names a function choosing the written
/// tag (`fn(&Self, &'static str, &WriteCx) -> &'static str`); a
/// `[convert = path]` after it, a function giving the structure to write
/// in place of this one for the target version
/// (`fn(&Self, &WriteCx) -> Option<Self>`, `None`: as it is).
macro_rules! gedcom_struct {
    (
        $(#[$meta:meta])*
        pub struct $name:ident $([tag = $tagfn:path])? $([convert = $convfn:path])? {
            $(@payload $(#[$pmeta:meta])* $pfield:ident : $pty:ty;)?
            $(
                $(#[$fmeta:meta])*
                $tag:literal $(| $alt:literal)* => $field:ident : $fty:ty,
            )*
        }
        spec { v551: [$($v551:literal),* $(,)?], v70: [$($v70:literal),* $(,)?], v71: [$($v71:literal),* $(,)?] $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Default, PartialEq)]
        pub struct $name {
            $($(#[$pmeta])* pub $pfield: $pty,)?
            $($(#[$fmeta])* pub $field: $fty,)*
            /// Substructures no field holds: extensions, unknown tags,
            /// repeated singletons and structures of another shape, in
            /// order.
            pub extra: $crate::next::Extra,
        }

        impl $crate::next::driver::Struct for $name {
            const FIELDS: &'static [$crate::next::driver::FieldDesc] = &[$(
                $crate::next::driver::FieldDesc {
                    tags: &[$tag $(, $alt)*],
                    name: stringify!($field),
                    many: <$fty as $crate::next::driver::Slot>::MANY,
                },
            )*];
            const LOOKUP: $crate::next::driver::Lookup =
                $crate::next::driver::lookup(Self::FIELDS);
            const SPEC: $crate::next::driver::SpecNames = $crate::next::driver::SpecNames {
                v551: &[$($v551),*],
                v70: &[$($v70),*],
                v71: &[$($v71),*],
            };
        }

        impl $crate::next::driver::Fields for $name {
            fn lookup(&self) -> &'static $crate::next::driver::Lookup {
                &<Self as $crate::next::driver::Struct>::LOOKUP
            }

            fn read_payload(
                &mut self,
                node: $crate::next::driver::NodeRef<'_>,
                cx: &mut $crate::next::driver::ReadCx<'_>,
            ) -> bool {
                gedcom_struct!(@read_payload self node cx $($pfield $pty)?)
            }

            #[allow(unused_assignments, unused_mut, unused_variables)]
            fn accept(
                &mut self,
                field: usize,
                node: $crate::next::driver::NodeRef<'_>,
                cx: &mut $crate::next::driver::ReadCx<'_>,
            ) -> bool {
                let mut i = 0_usize;
                $(
                    if field == i {
                        return $crate::next::driver::Slot::accept(&mut self.$field, node, cx);
                    }
                    i += 1;
                )*
                false
            }

            fn finish(&mut self) {
                $($crate::next::driver::Slot::finish(&mut self.$field);)*
            }

            fn extra(&self) -> &$crate::next::Extra {
                &self.extra
            }

            fn extra_mut(&mut self) -> &mut $crate::next::Extra {
                &mut self.extra
            }

            fn write_payload(&self, cx: &$crate::next::driver::WriteCx<'_>) -> $crate::tree::Payload {
                gedcom_struct!(@write_payload self cx $($pfield $pty)?)
            }

            #[allow(unused_variables)]
            fn write_fields(
                &self,
                cx: &$crate::next::driver::WriteCx<'_>,
                out: &mut Vec<$crate::tree::Structure>,
            ) {
                $($crate::next::driver::Slot::write(&self.$field, $tag, cx, out);)*
            }

            fn write_payload_flat<'s>(
                &'s self,
                cx: &$crate::next::driver::WriteCx<'s>,
            ) -> $crate::tree::FlatPayload<'s> {
                gedcom_struct!(@payload_flat self cx $($pfield $pty)?)
            }

            #[allow(unused_variables)]
            fn write_fields_flat<'s>(
                &'s self,
                cx: &$crate::next::driver::WriteCx<'s>,
                out: &mut $crate::tree::Flat<'s>,
            ) {
                $($crate::next::driver::Slot::to_flat(&self.$field, $tag, cx, out);)*
            }
        }

        impl $crate::next::driver::FromNode for $name {
            fn from_node(
                node: $crate::next::driver::NodeRef<'_>,
                cx: &mut $crate::next::driver::ReadCx<'_>,
            ) -> Option<Self> {
                $crate::next::driver::read(node, cx)
            }
        }

        impl $crate::next::driver::ToNodes for $name {
            fn to_node(
                &self,
                tag: &'static str,
                cx: &$crate::next::driver::WriteCx<'_>,
            ) -> $crate::tree::Structure {
                $(let tag = $tagfn(self, tag, cx);)?
                $(
                    if let Some(converted) = cx.convert.then(|| $convfn(self, cx)).flatten() {
                        return $crate::next::driver::write(&converted, tag, cx);
                    }
                )?
                $crate::next::driver::write(self, tag, cx)
            }

            fn to_flat<'s>(
                &'s self,
                tag: &'static str,
                cx: &$crate::next::driver::WriteCx<'s>,
                out: &mut $crate::tree::Flat<'s>,
            ) {
                $(let tag = $tagfn(self, tag, cx);)?
                $(
                    // A converted structure is a temporary: owned.
                    if let Some(converted) = cx.convert.then(|| $convfn(self, cx)).flatten() {
                        out.push_structure($crate::next::driver::write(&converted, tag, cx));
                        return;
                    }
                )?
                $crate::next::driver::write_flat(self, tag, cx, out);
            }
        }
    };
    (@read_payload $self:ident $node:ident $cx:ident) => {{
        let _ = $cx;
        $node.has_no_payload()
    }};
    (@read_payload $self:ident $node:ident $cx:ident $pfield:ident $pty:ty) => {
        match <$pty as $crate::next::driver::PayloadField>::read($node, $cx) {
            Some(v) => {
                $self.$pfield = v;
                true
            }
            None => false,
        }
    };
    (@write_payload $self:ident $cx:ident) => {{
        let _ = $cx;
        $crate::tree::Payload::None
    }};
    (@write_payload $self:ident $cx:ident $pfield:ident $pty:ty) => {
        $crate::next::driver::PayloadField::write(&$self.$pfield, $cx)
    };
    (@payload_flat $self:ident $cx:ident) => {{
        let _ = $cx;
        $crate::tree::FlatPayload::None
    }};
    (@payload_flat $self:ident $cx:ident $pfield:ident $pty:ty) => {
        $crate::next::driver::PayloadField::payload(&$self.$pfield, $cx)
    };
}
pub(crate) use gedcom_struct;
