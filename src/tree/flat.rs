//! Structures written into a flat arena, their texts borrowed where they
//! can be: what a typed model writes, one record at a time, for the writer
//! to check and emit without an owned [`Structure`] per line.

use std::borrow::Cow;

use super::{Node, Payload, PayloadRef, Structure, Tag, Xref};

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
}

/// Structures in write order (depth first), each with the end of its
/// subtree. Cleared and refilled record after record, it keeps its room.
#[derive(Clone, Debug, Default)]
pub(crate) struct Flat<'s> {
    nodes: Vec<FlatNode<'s>>,
    /// The structures open.
    depth: u32,
}

impl<'s> Flat<'s> {
    /// The number of structures.
    pub(crate) fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Empties the arena, keeping its room.
    pub(crate) fn clear(&mut self) {
        self.nodes.clear();
        self.depth = 0;
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
        self.nodes.push(FlatNode {
            standard,
            tag,
            xref,
            payload,
            end: 0,
            depth: self.depth,
        });
        self.depth += 1;
        self.nodes.len() - 1
    }

    /// Ends the structure [`Flat::open`] started at `at`.
    pub(crate) fn close(&mut self, at: usize) {
        let end = u32::try_from(self.nodes.len()).unwrap_or(u32::MAX);
        if let Some(node) = self.nodes.get_mut(at) {
            node.end = end;
        }
        self.depth = self.depth.saturating_sub(1);
    }

    /// A structure without substructures.
    pub(crate) fn leaf(&mut self, tag: Cow<'s, str>, payload: FlatPayload<'s>) {
        let at = self.open(tag, None, payload);
        self.close(at);
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

    /// Every structure, in write order, with its depth below the first.
    pub(crate) fn preorder(&self) -> impl Iterator<Item = (usize, FlatRef<'_>)> {
        let nodes: &[FlatNode<'_>] = &self.nodes;
        (0..nodes.len()).filter_map(move |i| {
            let depth = nodes.get(i)?.depth;
            Some((
                depth as usize,
                FlatRef {
                    nodes,
                    index: u32::try_from(i).ok()?,
                },
            ))
        })
    }

    /// The first structure written, with its substructures.
    pub(crate) fn root(&self) -> Option<FlatRef<'_>> {
        (!self.nodes.is_empty()).then_some(FlatRef {
            nodes: &self.nodes,
            index: 0,
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
        match self.node().map(|n| &n.payload) {
            Some(FlatPayload::Pointer(p)) => PayloadRef::Pointer(p),
            Some(FlatPayload::Text(t)) => PayloadRef::Text(t),
            Some(FlatPayload::None) | None => PayloadRef::None,
        }
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
        let levels: Vec<(usize, &str)> = flat.preorder().map(|(l, n)| (l, n.tag())).collect();
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
