//! Untyped structures: what typed structures keep in `extra`.

use std::ops::{Deref, DerefMut};

use std::borrow::Cow;

use crate::tree::{Flat, FlatPayload, Payload, Structure, Tag, Xref};

use super::driver::WriteCx;
use super::list::ThinVec;
use super::text::{Store, TagId, Text, XrefId};

/// The payload of a [`Node`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Value {
    /// No payload.
    #[default]
    None,
    /// A pointer.
    Pointer(XrefId),
    /// Text, unescaped, continuations joined.
    Text(Text),
}

/// A structure kept as it was read: a tag, an identifier, a payload and
/// substructures. Typed structures keep in their `extra` every substructure
/// they have no field for, and a dataset keeps so the records it has no
/// type for.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Node {
    /// The tag.
    pub tag: TagId,
    /// An identifier, which a substructure seldom has.
    pub xref: Option<XrefId>,
    /// The payload.
    pub payload: Value,
    /// The substructures, in order.
    pub children: Vec<Node>,
}

impl Node {
    /// A node with a tag and nothing else.
    #[must_use]
    pub fn new(tag: TagId) -> Self {
        Self {
            tag,
            xref: None,
            payload: Value::None,
            children: Vec::new(),
        }
    }

    /// An owned [`Structure`] with the same contents, read from `store`.
    #[must_use]
    pub fn to_structure<S: AsRef<Store> + ?Sized>(&self, store: &S) -> Structure {
        let store = store.as_ref();
        Structure {
            tag: Tag::new(store.tag(self.tag)),
            xref: self.xref.map(|x| Xref::new(store.xref(x))),
            payload: match &self.payload {
                Value::None => Payload::None,
                Value::Pointer(p) => Payload::Pointer(Xref::new(store.xref(*p))),
                Value::Text(t) => WriteCx::str(&t.to_str(store)),
            },
            substructures: self
                .children
                .iter()
                .map(|c| c.to_structure(store))
                .collect(),
            line: 0,
        }
    }

    /// The payload, borrowed.
    pub(crate) fn payload_flat<'s>(&'s self, store: &'s Store) -> FlatPayload<'s> {
        match &self.payload {
            Value::None => FlatPayload::None,
            Value::Pointer(p) => FlatPayload::Pointer(Cow::Borrowed(store.xref(*p))),
            Value::Text(t) if t.is_empty() => FlatPayload::None,
            Value::Text(t) => FlatPayload::Text(t.to_str(store)),
        }
    }

    /// [`Node::to_structure`], into a flat arena, borrowing the texts.
    pub(crate) fn to_flat<'s>(&'s self, store: &'s Store, out: &mut Flat<'s>) {
        let payload = self.payload_flat(store);
        let standard = u16::try_from(self.tag.get())
            .ok()
            .filter(|&t| usize::from(t) < crate::tree::STANDARD_TAGS.len());
        let at = out.open_standard(
            Cow::Borrowed(store.tag(self.tag)),
            standard,
            self.xref.map(|x| Cow::Borrowed(store.xref(x))),
            payload,
        );
        for c in &self.children {
            c.to_flat(store, out);
        }
        out.close(at);
    }
}

/// The substructures of a typed structure that no field holds, in order.
///
/// Empty, it takes one word and no allocation; it dereferences to a
/// [`ThinVec`] of [`Node`]s.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extra(ThinVec<Node>);

impl Extra {
    /// No node.
    #[must_use]
    pub const fn new() -> Self {
        Self(ThinVec::new())
    }
}

impl Deref for Extra {
    type Target = ThinVec<Node>;

    fn deref(&self) -> &ThinVec<Node> {
        &self.0
    }
}

impl DerefMut for Extra {
    fn deref_mut(&mut self) -> &mut ThinVec<Node> {
        &mut self.0
    }
}

impl From<Vec<Node>> for Extra {
    fn from(nodes: Vec<Node>) -> Self {
        Self(nodes.into())
    }
}

impl FromIterator<Node> for Extra {
    fn from_iter<I: IntoIterator<Item = Node>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl<'a> IntoIterator for &'a Extra {
    type Item = &'a Node;
    type IntoIter = std::slice::Iter<'a, Node>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}
