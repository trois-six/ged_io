//! Reading the flat arena the builder fills.

use super::lexer::{Kind, RawNode, Span};
use super::tag::STANDARD_TAGS;
use super::{PayloadRef, Structure, Tag, Xref};

/// The buffers of one arena: a segment of a tree, or the record a
/// [`TreeReader`](super::TreeReader) just read.
#[derive(Clone, Copy)]
pub(crate) struct View<'a> {
    pub text: &'a str,
    pub side: &'a str,
    pub nodes: &'a [RawNode],
    /// `(node, identifier)`, by node.
    pub xrefs: &'a [(u32, Span)],
    /// The tags that are not standard.
    pub tags: &'a [Box<str>],
}

impl<'a> View<'a> {
    #[inline]
    pub(crate) fn tag(self, node: &RawNode) -> &'a str {
        let id = node.tag as usize;
        match STANDARD_TAGS.get(id) {
            Some(tag) => tag,
            None => self.tags.get(id - STANDARD_TAGS.len()).map_or("", |t| t),
        }
    }

    pub(crate) fn tag_owned(self, node: &RawNode) -> Tag {
        match u16::try_from(node.tag) {
            Ok(i) if usize::from(i) < STANDARD_TAGS.len() => Tag::standard(i),
            _ => Tag::new(self.tag(node)),
        }
    }

    #[inline]
    pub(crate) fn xref(self, index: usize) -> Option<&'a str> {
        if !self.nodes.get(index)?.has_xref {
            return None;
        }
        let index = u32::try_from(index).ok()?;
        let at = self.xrefs.binary_search_by_key(&index, |&(n, _)| n).ok()?;
        self.xrefs.get(at).map(|(_, span)| span.get(self.text))
    }

    #[inline]
    pub(crate) fn payload(self, node: &RawNode) -> PayloadRef<'a> {
        match node.kind {
            Kind::None => PayloadRef::None,
            Kind::Pointer => PayloadRef::Pointer(node.payload.get(self.text)),
            Kind::Text => PayloadRef::Text(node.payload.get(self.text)),
            Kind::Side => PayloadRef::Text(node.payload.get(self.side)),
        }
    }

    /// An owned copy of the structure at `index` and its substructures.
    pub(crate) fn to_structure(self, index: usize) -> Structure {
        let Some(node) = self.nodes.get(index) else {
            return Structure::default();
        };
        let mut substructures = Vec::new();
        let mut child = index + 1;
        while child < node.end as usize {
            substructures.push(self.to_structure(child));
            child = self
                .nodes
                .get(child)
                .map_or(child + 1, |c| (c.end as usize).max(child + 1));
        }
        Structure {
            tag: self.tag_owned(node),
            xref: self.xref(index).map(Xref::new),
            payload: self.payload(node).to_payload(),
            substructures,
            line: node.line,
        }
    }
}
