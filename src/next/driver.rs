//! The generic driver behind every typed structure.
//!
//! [`gedcom_struct!`](super::gedcom_struct) declares a structure: its
//! identifier, tag and payload when it has them, its substructures by tag
//! and its field types. The field types give the cardinality through
//! [`Slot`]: an `Option` holds one occurrence (a second one goes to
//! `extra`), a `Vec` or a [`ThinVec`](super::ThinVec) any number. The macro
//! generates the struct, its field table and a few short methods
//! ([`Fields`]); everything else is done here, once for all structures —
//! the loops below have one copy, which every type calls through
//! `dyn Fields`:
//!
//! - [`read`] fills a structure from a node of the record arena: the
//!   identifier, the tag and the payload, then each substructure through
//!   the field its tag selects (a table of the standard tags, computed at
//!   compile time), and whatever no field takes — an unknown or extension
//!   tag, a repeated singleton, a substructure whose shape its field cannot
//!   hold — into `extra`, in order. Reading never fails: a node that does
//!   not fit a type is refused ([`FromNode`] returns `None`) and its
//!   superstructure keeps it whole in its own `extra`.
//! - [`write`] turns a structure back into a [`Structure`]: identifier,
//!   payload, fields in declaration order, then `extra`.
//!
//! A structure may keep the fields that few of its occurrences use in a
//! boxed *detail* (`@detail`): one word when none is used, allocated with
//! the first. Its fields read, write and are checked by the coverage
//! ledger like the others.

use std::borrow::Cow;

use crate::spec::schema::Kind;
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

    /// The tag, when it is standard.
    pub(crate) fn standard_tag(self) -> Option<&'static str> {
        STANDARD_TAGS.get(self.tag_id() as usize).copied()
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

/// What a `[convert = …]` function of [`gedcom_struct!`] makes of a value
/// for the target version.
pub(crate) enum Converted<T> {
    /// Written as it is: valid in the target's grammar as this kind of
    /// payload when known ([`PayloadField::claim`]).
    AsIs(Option<Kind>),
    /// Written as this value instead.
    Into(T),
}

impl<T> Converted<T> {
    /// The value to write instead, if any.
    pub(crate) fn into_value(self) -> Option<T> {
        match self {
            Converted::Into(v) => Some(v),
            Converted::AsIs(_) => None,
        }
    }
}

/// A type read from a node and its substructures.
pub(crate) trait FromNode: Sized {
    /// Whether the type is a leaf: a payload without substructures (and
    /// without `extra`, so that a node with substructures does not fit).
    const LEAF: bool = false;
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
    fn to_flat<'s>(&'s self, tag: StdTag, cx: &WriteCx<'s>, out: &mut Flat<'s>) {
        out.push_structure(self.to_node(tag.name, cx));
    }

    /// Calls `f` with this value and its tag when it is a typed structure
    /// (leaves are not).
    fn visit(&self, tag: &'static str, f: &mut dyn FnMut(&'static str, &dyn Fields)) {
        let _ = (tag, f);
    }

    /// An untyped node of this type's tag, kept in `extra` (a repeated
    /// singleton, or one after it), written as this type converts its
    /// values for the target version; `None`: as it is.
    fn write_extra(node: &Node, tag: &'static str, cx: &WriteCx<'_>) -> Option<Structure>
    where
        Self: Sized,
    {
        let _ = (node, tag, cx);
        None
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

    /// The kind of payload [`PayloadField::payload`] is valid for by
    /// construction (a known enumeration value, a number), which the
    /// conformance check then trusts ([`Flat::open_typed`]); `None` for a
    /// value that may be invalid (a text). By default, none.
    fn claim(&self, cx: &WriteCx<'_>) -> Option<Kind> {
        let _ = cx;
        None
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

/// A tag that a structure keeps as a field (`@tag`): the structure stands
/// for several tags (`BIRT`, `DEAT`, … of an event), read from its node
/// and written back.
pub(crate) trait TagField: Sized {
    /// The value of a standard tag; `None` for a tag the type does not
    /// name.
    fn from_tag(tag: &'static str) -> Option<Self>;
    /// The tag to write.
    fn tag(&self) -> &'static str;
    /// The same, with its index among the standard tags.
    fn std_tag(&self) -> StdTag {
        StdTag::of(self.tag())
    }
}

/// A standard tag a typed structure is written with, and its index among
/// the standard tags ([`crate::tree::standard_index`]), found once: in a
/// constant for the tags of fields.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StdTag {
    /// The tag.
    pub(crate) name: &'static str,
    /// Its index among the standard tags.
    pub(crate) index: Option<u16>,
}

impl StdTag {
    /// A tag, its index found by a search (for a constant).
    pub(crate) const fn new(name: &'static str) -> Self {
        Self {
            name,
            index: crate::tree::standard_index_const(name),
        }
    }

    /// A tag known at run time.
    pub(crate) fn of(name: &'static str) -> Self {
        Self {
            name,
            index: crate::tree::standard_index(name),
        }
    }
}

/// Declares a fieldless enumeration of tags, for a [`TagField`]:
/// `Variant = "TAG"`.
macro_rules! tag_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $(#[$fmeta:meta])* $first:ident = $ftag:literal,
            $($(#[$vmeta:meta])* $variant:ident = $tag:literal,)*
        }
    ) => {
        $(#[$meta])*
        #[non_exhaustive]
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        pub enum $name {
            $(#[$fmeta])*
            #[default]
            $first,
            $($(#[$vmeta])* $variant,)*
        }

        impl $name {
            /// The tag.
            #[must_use]
            pub const fn tag(self) -> &'static str {
                match self {
                    Self::$first => $ftag,
                    $(Self::$variant => $tag,)*
                }
            }
        }

        impl $crate::next::driver::TagField for $name {
            fn from_tag(tag: &'static str) -> Option<Self> {
                match tag {
                    $ftag => Some(Self::$first),
                    $($tag => Some(Self::$variant),)*
                    _ => None,
                }
            }

            fn tag(&self) -> &'static str {
                (*self).tag()
            }

            fn std_tag(&self) -> $crate::next::driver::StdTag {
                match self {
                    Self::$first => const { $crate::next::driver::StdTag::new($ftag) },
                    $(Self::$variant => const { $crate::next::driver::StdTag::new($tag) },)*
                }
            }
        }
    };
}
pub(crate) use tag_enum;

/// A field: how many occurrences of its substructure it holds.
pub(crate) trait Slot {
    /// Whether the field holds any number of occurrences.
    const MANY: bool;
    /// Whether its substructure is a leaf ([`FromNode::LEAF`]).
    const LEAF: bool;
    /// Takes an occurrence; `false` when the field is full or the node does
    /// not fit (the node then goes to `extra`).
    fn accept(&mut self, node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> bool;
    /// Writes the occurrences.
    fn write(&self, tag: &'static str, cx: &WriteCx<'_>, out: &mut Vec<Structure>);
    /// Writes the occurrences into a flat arena ([`ToNodes::to_flat`]).
    fn to_flat<'s>(&'s self, tag: StdTag, cx: &WriteCx<'s>, out: &mut Flat<'s>) {
        let mut owned = Vec::new();
        self.write(tag.name, cx, &mut owned);
        for s in owned {
            out.push_structure(s);
        }
    }
    /// Calls `f` with each occurrence that is a typed structure.
    fn visit(&self, tag: &'static str, f: &mut dyn FnMut(&'static str, &dyn Fields));
    /// Releases spare capacity once the structure is read.
    fn finish(&mut self) {}
    /// An untyped node of the field's tag, as its type writes it
    /// ([`ToNodes::write_extra`]).
    fn write_extra(node: &Node, tag: &'static str, cx: &WriteCx<'_>) -> Option<Structure>;
}

impl<T: FromNode + ToNodes> Slot for Option<T> {
    const MANY: bool = false;
    const LEAF: bool = T::LEAF;

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

    fn to_flat<'s>(&'s self, tag: StdTag, cx: &WriteCx<'s>, out: &mut Flat<'s>) {
        if let Some(v) = self {
            v.to_flat(tag, cx, out);
        }
    }

    fn write_extra(node: &Node, tag: &'static str, cx: &WriteCx<'_>) -> Option<Structure> {
        T::write_extra(node, tag, cx)
    }

    fn visit(&self, tag: &'static str, f: &mut dyn FnMut(&'static str, &dyn Fields)) {
        if let Some(v) = self {
            v.visit(tag, f);
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
    const LEAF: bool = T::LEAF;

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

    fn to_flat<'s>(&'s self, tag: StdTag, cx: &WriteCx<'s>, out: &mut Flat<'s>) {
        for (i, v) in self.iter().enumerate() {
            out.repeat(i > 0);
            v.to_flat(tag, cx, out);
        }
    }

    fn write_extra(node: &Node, tag: &'static str, cx: &WriteCx<'_>) -> Option<Structure> {
        T::write_extra(node, tag, cx)
    }

    fn visit(&self, tag: &'static str, f: &mut dyn FnMut(&'static str, &dyn Fields)) {
        for v in self {
            v.visit(tag, f);
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
    const LEAF: bool = T::LEAF;

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

    fn to_flat<'s>(&'s self, tag: StdTag, cx: &WriteCx<'s>, out: &mut Flat<'s>) {
        for (i, v) in self.iter().enumerate() {
            out.repeat(i > 0);
            v.to_flat(tag, cx, out);
        }
    }

    fn write_extra(node: &Node, tag: &'static str, cx: &WriteCx<'_>) -> Option<Structure> {
        T::write_extra(node, tag, cx)
    }

    fn visit(&self, tag: &'static str, f: &mut dyn FnMut(&'static str, &dyn Fields)) {
        for v in self {
            v.visit(tag, f);
        }
    }

    fn finish(&mut self) {
        self.shrink();
    }
}

impl<T: FromNode> FromNode for Box<T> {
    const LEAF: bool = T::LEAF;

    fn from_node(node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> Option<Self> {
        T::from_node(node, cx).map(Box::new)
    }
}

impl<T: ToNodes> ToNodes for Box<T> {
    fn to_node(&self, tag: &'static str, cx: &WriteCx<'_>) -> Structure {
        (**self).to_node(tag, cx)
    }

    fn to_flat<'s>(&'s self, tag: StdTag, cx: &WriteCx<'s>, out: &mut Flat<'s>) {
        (**self).to_flat(tag, cx, out);
    }

    fn write_extra(node: &Node, tag: &'static str, cx: &WriteCx<'_>) -> Option<Structure> {
        T::write_extra(node, tag, cx)
    }

    fn visit(&self, tag: &'static str, f: &mut dyn FnMut(&'static str, &dyn Fields)) {
        (**self).visit(tag, f);
    }
}

/// The empty value of a field, as a constant: the fields of a detail that
/// has not been allocated read as these.
pub(crate) trait ConstDefault {
    /// The empty value.
    const EMPTY: Self;
}

impl<T> ConstDefault for Option<T> {
    const EMPTY: Self = None;
}

impl<T> ConstDefault for Vec<T> {
    const EMPTY: Self = Vec::new();
}

impl<T> ConstDefault for ThinVec<T> {
    const EMPTY: Self = ThinVec::new();
}

/// Gives a substructure to a field of a structure's detail, allocating the
/// detail for it; a detail allocated for a node that does not fit is
/// released.
pub(crate) fn accept_detail<D: Default, S: Slot>(
    detail: &mut Option<Box<D>>,
    field: impl FnOnce(&mut D) -> &mut S,
    node: NodeRef<'_>,
    cx: &mut ReadCx<'_>,
) -> bool {
    let fresh = detail.is_none();
    let accepted = field(detail.get_or_insert_with(Box::default)).accept(node, cx);
    if !accepted && fresh {
        *detail = None;
    }
    accepted
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
    tag: StdTag,
    cx: &WriteCx<'s>,
    out: &mut Flat<'s>,
) {
    out.leaf_typed(tag.name, tag.index, value.payload(cx), value.claim(cx));
}

/// A structure with a tag and a payload.
pub(crate) fn leaf_structure(tag: &'static str, payload: Payload) -> Structure {
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
            const LEAF: bool = true;

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
                tag: $crate::next::driver::StdTag,
                cx: &$crate::next::driver::WriteCx<'s>,
                out: &mut $crate::tree::Flat<'s>,
            ) {
                $crate::next::driver::write_leaf_flat(self, tag, cx, out);
            }
        }
    )*};
}
pub(crate) use leaf;

/// A pointer leaf (`ANCI @U1@`, `SUBM @U1@`): text in its place does not
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

/// An integer (`HEIGHT 100`): digits only, at most `u32::MAX`; anything
/// else does not fit and is kept as it is, in `extra`.
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

    fn claim(&self, _cx: &WriteCx<'_>) -> Option<Kind> {
        Some(Kind::Int)
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
    /// Whether its substructure is a leaf (a value without substructures).
    pub leaf: bool,
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
    /// The type's fields.
    fn fields(&self) -> &'static [FieldDesc];
    /// The field of each standard tag.
    fn lookup(&self) -> &'static Lookup;
    /// Reads the identifier; `false` when the node has one and the type
    /// has none (a substructure).
    fn read_xref(&mut self, node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> bool;
    /// Reads the tag of a type that keeps it; `false` when the type does
    /// not name it.
    fn read_tag(&mut self, node: NodeRef<'_>) -> bool;
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
    /// The identifier to write.
    fn write_xref(&self, cx: &WriteCx<'_>) -> Option<Xref>;
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
    /// [`Fields::write_xref`], borrowed where it can be.
    fn write_xref_flat<'s>(&'s self, cx: &WriteCx<'s>) -> Option<Cow<'s, str>> {
        self.write_xref(cx)
            .map(|x| Cow::Owned(x.as_str().to_string()))
    }
    /// [`PayloadField::claim`] of the payload.
    fn payload_claim(&self, cx: &WriteCx<'_>) -> Option<Kind> {
        let _ = cx;
        None
    }
    /// Calls `f` with every typed substructure held by a field, and its
    /// tag.
    fn walk(&self, f: &mut dyn FnMut(&'static str, &dyn Fields));
    /// An untyped node of field `field`'s tag, as the field's type writes
    /// it ([`ToNodes::write_extra`]).
    fn write_extra(
        &self,
        field: usize,
        node: &Node,
        tag: &'static str,
        cx: &WriteCx<'_>,
    ) -> Option<Structure>;
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

/// Fills a structure from `node`: its identifier, tag and payload, then
/// each substructure into the field its tag selects, or into `extra`.
/// `false` when the node does not fit.
///
/// Substructures of one tag keep their order (the first is the preferred
/// one): once one does not fit its field (its shape, or a field already
/// full), the later ones of its tag go to `extra` after it, so that the
/// field's, written first, all came before it.
///
/// A payload or an identifier the type has no place for (text where its
/// pointer belongs, a payload on a record, an identifier on a
/// substructure) does not make the structure untyped: it is kept aside, as
/// the first node of `extra`, tagged [`TagId::ASIDE`], and written back in
/// its place.
pub(crate) fn read_into(s: &mut dyn Fields, node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> bool {
    if !s.read_tag(node) {
        return false;
    }
    let xref_fits = s.read_xref(node, cx);
    // No payload where one is expected leaves the type's empty value.
    let payload_fits = s.read_payload(node, cx) || node.has_no_payload();
    if !xref_fits || !payload_fits {
        let mut aside = cx.node_shallow(node);
        aside.tag = TagId::ASIDE;
        if xref_fits {
            aside.xref = None;
        }
        if payload_fits {
            aside.payload = Value::None;
        }
        s.extra_mut().push(aside);
    }
    let lookup = s.lookup();
    // The standard tags already sent to `extra`, as a bit set.
    let mut sent = [0_u64; STANDARD_TAGS.len().div_ceil(64)];
    for child in node.children() {
        let tag = child.tag_id() as usize;
        let field = lookup.get(tag).copied().unwrap_or(0);
        let (word, bit) = (tag / 64, 1_u64 << (tag % 64));
        let blocked = sent.get(word).is_some_and(|w| w & bit != 0);
        if field == 0 || blocked || !s.accept(usize::from(field - 1), child, cx) {
            if let Some(w) = sent.get_mut(word).filter(|_| field != 0) {
                *w |= bit;
            }
            let n = cx.node(child);
            s.extra_mut().push(n);
        }
    }
    s.finish();
    true
}

/// [`write`] into a flat arena, the texts borrowed where they can be:
/// typed ([`Flat::open_typed`]), the payload trusted as `claim` (or the
/// type's [`Fields::payload_claim`]) says.
pub(crate) fn write_flat<'s>(
    s: &'s dyn Fields,
    tag: StdTag,
    cx: &WriteCx<'s>,
    out: &mut Flat<'s>,
    claim: Option<Kind>,
) {
    let mut xref = s.write_xref_flat(cx);
    let mut payload = s.write_payload_flat(cx);
    let mut claim = claim.or_else(|| s.payload_claim(cx));
    let extra = s.extra();
    for node in extra.iter().filter(|n| n.tag == TagId::ASIDE) {
        // What the type has no place for is no typed value.
        out.untrusted();
        claim = None;
        if xref.is_none() {
            xref = node.xref.map(|x| Cow::Borrowed(cx.store.xref(x)));
        }
        match node.payload_flat(cx.store) {
            FlatPayload::None => {}
            p => payload = p,
        }
    }
    let at = out.open_typed(tag.name, tag.index, xref, payload, claim);
    s.write_fields_flat(cx, out);
    let lookup = s.lookup();
    for node in extra.iter().filter(|n| n.tag != TagId::ASIDE) {
        // As in [`write`]: a node a field takes is written as its type
        // writes its values.
        let index = node.tag.get() as usize;
        let field = lookup.get(index).copied().unwrap_or(0);
        let written = match (cx.convert, STANDARD_TAGS.get(index)) {
            (true, Some(std)) if field > 0 => s.write_extra(usize::from(field - 1), node, std, cx),
            _ => None,
        };
        match written {
            Some(w) => out.push_structure(w),
            None => node.to_flat(cx.store, out),
        }
    }
    out.close(at);
}

/// Writes a typed structure as `tag`: its identifier and payload, its
/// fields in order, then `extra`.
pub(crate) fn write(s: &dyn Fields, tag: &'static str, cx: &WriteCx<'_>) -> Structure {
    let mut substructures = Vec::new();
    s.write_fields(cx, &mut substructures);
    let lookup = s.lookup();
    let mut xref = s.write_xref(cx);
    let mut payload = s.write_payload(cx);
    for node in s.extra() {
        if node.tag == TagId::ASIDE {
            let aside = node.to_structure(cx.store);
            xref = xref.or(aside.xref);
            if aside.payload != Payload::None {
                payload = aside.payload;
            }
            continue;
        }
        // A node a field takes is written as the field's type writes its
        // values (a second date, in the target's grammar), so that what
        // reads back typed writes alike.
        let index = node.tag.get() as usize;
        let field = lookup.get(index).copied().unwrap_or(0);
        let written = match (cx.convert, STANDARD_TAGS.get(index)) {
            (true, Some(std)) if field > 0 => s.write_extra(usize::from(field - 1), node, std, cx),
            _ => None,
        };
        substructures.push(written.unwrap_or_else(|| node.to_structure(cx.store)));
    }
    Structure {
        tag: Tag::new(tag),
        xref,
        payload,
        substructures,
        line: 0,
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
///         @xref /// Docs. xref;                   // records only
///         @tag /// Docs. kind: TagType;           // a TagField
///         @payload /// Docs. value: Text;         // a PayloadField
///         /// Docs.
///         "TAG" | "ALT" => field: Option<Type>,   // or Vec<Type>, ThinVec<Type>
///         @detail /// Docs. NameDetail {          // boxed, for rare fields
///             /// Docs.
///             "TAG" => field: Option<Type>,
///         }
///     }
///     spec { v551: ["…"], v70: ["…"], v71: ["…"] }
/// }
/// ```
///
/// Every part but the fields is optional. A `[tag = path]` before the
/// fields names a function choosing the written tag
/// (`fn(&Self, &'static str, &WriteCx) -> &'static str`); a
/// `[convert = path, path]` after it, a function giving the structure to
/// write in place of this one for the target version
/// (`fn(&Self, &WriteCx) -> Converted<Self>`: [`Converted::AsIs`] with the
/// kind of payload it is valid as, when it is known valid), and one
/// reading the type from an untyped node (`fn(&Node, &Store) ->
/// Option<Self>`), with which the nodes of its tag kept in `extra` are
/// converted alike.
///
/// A detail is a struct of its own, with every field of the group; the
/// structure holds it as `detail: Option<Box<…>>`, allocated when one of
/// its fields is read or set, and has `detail()` (the detail, or an empty
/// one) and `detail_mut()` (allocating it).
macro_rules! gedcom_struct {
    (
        $(#[$meta:meta])*
        pub struct $name:ident $([tag = $tagfn:path])? $([convert = $convfn:path, $untypedfn:path])? {
            $(@xref $(#[$xmeta:meta])* $xfield:ident;)?
            $(@tag $(#[$tmeta:meta])* $tfield:ident : $tty:ty;)?
            $(@payload $(#[$pmeta:meta])* $pfield:ident : $pty:ty;)?
            $(
                $(#[$fmeta:meta])*
                $tag:literal $(| $alt:literal)* => $field:ident : $fty:ty,
            )*
            $(
                @detail $(#[$dmeta:meta])* $dname:ident {
                    $(
                        $(#[$gmeta:meta])*
                        $gtag:literal $(| $galt:literal)* => $gfield:ident : $gty:ty,
                    )*
                }
            )?
        }
        spec { v551: [$($v551:literal),* $(,)?], v70: [$($v70:literal),* $(,)?], v71: [$($v71:literal),* $(,)?] $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Default, PartialEq)]
        pub struct $name {
            $($(#[$xmeta])* pub $xfield: Option<$crate::next::XrefId>,)?
            $($(#[$tmeta])* pub $tfield: $tty,)?
            $($(#[$pmeta])* pub $pfield: $pty,)?
            $($(#[$fmeta])* pub $field: $fty,)*
            $(
                #[doc = concat!("The fields few structures use (a [`", stringify!($dname), "`]), when one is: see [`", stringify!($name), "::detail`].")]
                pub detail: Option<Box<$dname>>,
            )?
            /// Substructures no field holds: extensions, unknown tags,
            /// repeated singletons and structures of another shape, in
            /// order.
            pub extra: $crate::next::Extra,
        }

        $(
            $(#[$dmeta])*
            #[derive(Clone, Debug, Default, PartialEq)]
            pub struct $dname {
                $($(#[$gmeta])* pub $gfield: $gty,)*
            }

            impl $dname {
                /// A detail with every field empty.
                const EMPTY: Self = Self {
                    $($gfield: <$gty as $crate::next::driver::ConstDefault>::EMPTY,)*
                };
            }

            impl $name {
                /// The fields few structures use: the detail, or an empty
                /// one when none of them is set.
                #[must_use]
                pub fn detail(&self) -> &$dname {
                    static EMPTY: $dname = $dname::EMPTY;
                    self.detail.as_deref().unwrap_or(&EMPTY)
                }

                /// The fields few structures use, to edit: the detail,
                /// allocated when none of them is set yet.
                pub fn detail_mut(&mut self) -> &mut $dname {
                    self.detail.get_or_insert_with(Box::default)
                }
            }
        )?

        impl $crate::next::driver::Struct for $name {
            const FIELDS: &'static [$crate::next::driver::FieldDesc] = &[
                $(
                    $crate::next::driver::FieldDesc {
                        tags: &[$tag $(, $alt)*],
                        name: stringify!($field),
                        many: <$fty as $crate::next::driver::Slot>::MANY,
                        leaf: <$fty as $crate::next::driver::Slot>::LEAF,
                    },
                )*
                $($(
                    $crate::next::driver::FieldDesc {
                        tags: &[$gtag $(, $galt)*],
                        name: stringify!($gfield),
                        many: <$gty as $crate::next::driver::Slot>::MANY,
                        leaf: <$gty as $crate::next::driver::Slot>::LEAF,
                    },
                )*)?
            ];
            const LOOKUP: $crate::next::driver::Lookup =
                $crate::next::driver::lookup(Self::FIELDS);
            const SPEC: $crate::next::driver::SpecNames = $crate::next::driver::SpecNames {
                v551: &[$($v551),*],
                v70: &[$($v70),*],
                v71: &[$($v71),*],
            };
        }

        impl $crate::next::driver::Fields for $name {
            fn fields(&self) -> &'static [$crate::next::driver::FieldDesc] {
                <Self as $crate::next::driver::Struct>::FIELDS
            }

            fn lookup(&self) -> &'static $crate::next::driver::Lookup {
                &<Self as $crate::next::driver::Struct>::LOOKUP
            }

            fn read_xref(
                &mut self,
                node: $crate::next::driver::NodeRef<'_>,
                cx: &mut $crate::next::driver::ReadCx<'_>,
            ) -> bool {
                gedcom_struct!(@read_xref self node cx $($xfield)?)
            }

            fn read_tag(&mut self, node: $crate::next::driver::NodeRef<'_>) -> bool {
                gedcom_struct!(@read_tag self node $($tfield $tty)?)
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
                $($(
                    if field == i {
                        return $crate::next::driver::accept_detail(
                            &mut self.detail,
                            |d: &mut $dname| &mut d.$gfield,
                            node,
                            cx,
                        );
                    }
                    i += 1;
                )*)?
                false
            }

            fn finish(&mut self) {
                $($crate::next::driver::Slot::finish(&mut self.$field);)*
                $(
                    if let Some(d) = &mut self.detail {
                        let d: &mut $dname = d;
                        $($crate::next::driver::Slot::finish(&mut d.$gfield);)*
                    }
                )?
            }

            fn extra(&self) -> &$crate::next::Extra {
                &self.extra
            }

            fn extra_mut(&mut self) -> &mut $crate::next::Extra {
                &mut self.extra
            }

            fn write_xref(
                &self,
                cx: &$crate::next::driver::WriteCx<'_>,
            ) -> Option<$crate::tree::Xref> {
                gedcom_struct!(@write_xref self cx $($xfield)?)
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
                $(
                    if let Some(d) = &self.detail {
                        let d: &$dname = d;
                        $($crate::next::driver::Slot::write(&d.$gfield, $gtag, cx, out);)*
                    }
                )?
            }

            #[allow(unused_assignments, unused_mut, unused_variables)]
            fn write_extra(
                &self,
                field: usize,
                node: &$crate::next::Node,
                tag: &'static str,
                cx: &$crate::next::driver::WriteCx<'_>,
            ) -> Option<$crate::tree::Structure> {
                let mut i = 0_usize;
                $(
                    if field == i {
                        return <$fty as $crate::next::driver::Slot>::write_extra(node, tag, cx);
                    }
                    i += 1;
                )*
                $($(
                    if field == i {
                        return <$gty as $crate::next::driver::Slot>::write_extra(node, tag, cx);
                    }
                    i += 1;
                )*)?
                None
            }

            #[allow(unused_variables)]
            fn walk(&self, f: &mut dyn FnMut(&'static str, &dyn $crate::next::driver::Fields)) {
                $($crate::next::driver::Slot::visit(&self.$field, $tag, f);)*
                $(
                    if let Some(d) = &self.detail {
                        let d: &$dname = d;
                        $($crate::next::driver::Slot::visit(&d.$gfield, $gtag, f);)*
                    }
                )?
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
                $($crate::next::driver::Slot::to_flat(
                    &self.$field,
                    const { $crate::next::driver::StdTag::new($tag) },
                    cx,
                    out,
                );)*
                $(
                    if let Some(d) = &self.detail {
                        let d: &$dname = d;
                        $($crate::next::driver::Slot::to_flat(
                            &d.$gfield,
                            const { $crate::next::driver::StdTag::new($gtag) },
                            cx,
                            out,
                        );)*
                    }
                )?
            }

            fn write_xref_flat<'s>(
                &'s self,
                cx: &$crate::next::driver::WriteCx<'s>,
            ) -> Option<std::borrow::Cow<'s, str>> {
                gedcom_struct!(@xref_flat self cx $($xfield)?)
            }

            fn payload_claim(
                &self,
                cx: &$crate::next::driver::WriteCx<'_>,
            ) -> Option<$crate::spec::schema::Kind> {
                gedcom_struct!(@payload_claim self cx $($pfield $pty)?)
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
                // A structure that keeps its tag writes it.
                $(let tag = { let _ = tag; $crate::next::driver::TagField::tag(&self.$tfield) };)?
                $(let tag = $tagfn(self, tag, cx);)?
                $(
                    if cx.convert {
                        if let $crate::next::driver::Converted::Into(converted) = $convfn(self, cx) {
                            return $crate::next::driver::write(&converted, tag, cx);
                        }
                    }
                )?
                $crate::next::driver::write(self, tag, cx)
            }

            fn to_flat<'s>(
                &'s self,
                tag: $crate::next::driver::StdTag,
                cx: &$crate::next::driver::WriteCx<'s>,
                out: &mut $crate::tree::Flat<'s>,
            ) {
                $(let tag = { let _ = tag; $crate::next::driver::TagField::std_tag(&self.$tfield) };)?
                $(let tag = $crate::next::driver::StdTag::of($tagfn(self, tag.name, cx));)?
                let claim = None;
                $(
                    let claim = if cx.convert {
                        match $convfn(self, cx) {
                            // A converted structure is a temporary: owned.
                            $crate::next::driver::Converted::Into(converted) => {
                                out.push_structure($crate::next::driver::write(&converted, tag.name, cx));
                                return;
                            }
                            $crate::next::driver::Converted::AsIs(kind) => kind,
                        }
                    } else {
                        claim
                    };
                )?
                $crate::next::driver::write_flat(self, tag, cx, out, claim);
            }

            fn visit(
                &self,
                tag: &'static str,
                f: &mut dyn FnMut(&'static str, &dyn $crate::next::driver::Fields),
            ) {
                f(tag, self);
            }

            $(
                fn write_extra(
                    node: &$crate::next::Node,
                    tag: &'static str,
                    cx: &$crate::next::driver::WriteCx<'_>,
                ) -> Option<$crate::tree::Structure> {
                    let value: Self = $untypedfn(node, cx.store)?;
                    let converted = $convfn(&value, cx).into_value()?;
                    Some($crate::next::driver::write(&converted, tag, cx))
                }
            )?
        }
    };
    (@read_xref $self:ident $node:ident $cx:ident) => {{
        let _ = &$cx;
        // An identifier on a substructure (a 5.5.1 pointer may name it)
        // has no field: the node stays untyped, with it.
        !$node.has_xref()
    }};
    (@read_xref $self:ident $node:ident $cx:ident $xfield:ident) => {{
        $self.$xfield = $cx.xref($node);
        true
    }};
    (@read_tag $self:ident $node:ident) => {{
        let _ = $node;
        true
    }};
    (@read_tag $self:ident $node:ident $tfield:ident $tty:ty) => {
        match $node.standard_tag().and_then(<$tty as $crate::next::driver::TagField>::from_tag) {
            Some(v) => {
                $self.$tfield = v;
                true
            }
            None => false,
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
    (@write_xref $self:ident $cx:ident) => {{
        let _ = $cx;
        None
    }};
    (@write_xref $self:ident $cx:ident $xfield:ident) => {
        $self.$xfield.map(|x| $crate::tree::Xref::new($cx.store.xref(x)))
    };
    (@write_payload $self:ident $cx:ident) => {{
        let _ = $cx;
        $crate::tree::Payload::None
    }};
    (@write_payload $self:ident $cx:ident $pfield:ident $pty:ty) => {
        $crate::next::driver::PayloadField::write(&$self.$pfield, $cx)
    };
    (@payload_claim $self:ident $cx:ident) => {{
        let _ = $cx;
        None
    }};
    (@payload_claim $self:ident $cx:ident $pfield:ident $pty:ty) => {
        $crate::next::driver::PayloadField::claim(&$self.$pfield, $cx)
    };
    (@xref_flat $self:ident $cx:ident) => {{
        let _ = $cx;
        None
    }};
    (@xref_flat $self:ident $cx:ident $xfield:ident) => {
        $self
            .$xfield
            .map(|x| std::borrow::Cow::Borrowed($cx.store.xref(x)))
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
