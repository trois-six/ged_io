//! Untyped structures: what typed structures keep in `extra`.

use std::ops::Deref;

use crate::tree::{Payload, Structure, Tag, Xref};

use super::driver::WriteCx;
use super::text::{Source, TagId, Text, XrefId};

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
/// they have no field for.
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

    /// An owned [`Structure`] with the same contents, read from `source`.
    #[must_use]
    pub fn to_structure<S: AsRef<Source> + ?Sized>(&self, source: &S) -> Structure {
        let source = source.as_ref();
        Structure {
            tag: Tag::new(source.tag(self.tag)),
            xref: self.xref.map(|x| Xref::new(source.xref(x))),
            payload: match &self.payload {
                Value::None => Payload::None,
                Value::Pointer(p) => Payload::Pointer(Xref::new(source.xref(*p))),
                Value::Text(t) => WriteCx::str(t.as_str(source)),
            },
            substructures: self
                .children
                .iter()
                .map(|c| c.to_structure(source))
                .collect(),
            line: 0,
        }
    }
}

/// The substructures of a typed structure that no field holds, in order.
///
/// Empty, it takes one word and no allocation.
// Boxing the vector keeps an empty `extra`, the common case, to one word
// instead of three, on every typed structure.
#[allow(clippy::box_collection)]
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Extra(Option<Box<Vec<Node>>>);

impl Extra {
    /// Appends a node.
    pub fn push(&mut self, node: Node) {
        super::driver::push_exact(self.0.get_or_insert_with(Box::default), node);
    }

    /// The nodes, to edit.
    pub fn as_vec_mut(&mut self) -> &mut Vec<Node> {
        self.0.get_or_insert_with(Box::default)
    }

    /// Writes the nodes.
    pub(crate) fn write(&self, cx: &WriteCx<'_>, out: &mut Vec<Structure>) {
        for n in self {
            out.push(n.to_structure(cx.source));
        }
    }
}

impl Deref for Extra {
    type Target = [Node];

    fn deref(&self) -> &[Node] {
        self.0.as_deref().map_or(&[], Vec::as_slice)
    }
}

impl From<Vec<Node>> for Extra {
    fn from(nodes: Vec<Node>) -> Self {
        if nodes.is_empty() {
            Self(None)
        } else {
            Self(Some(Box::new(nodes)))
        }
    }
}

impl FromIterator<Node> for Extra {
    fn from_iter<I: IntoIterator<Item = Node>>(iter: I) -> Self {
        iter.into_iter().collect::<Vec<_>>().into()
    }
}

impl<'a> IntoIterator for &'a Extra {
    type Item = &'a Node;
    type IntoIter = std::slice::Iter<'a, Node>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
