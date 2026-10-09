//! Moving structures from one store to another.
//!
//! A record a stream reads points into a store of its own; added to a
//! dataset ([`Dataset::extend`](super::Dataset)), its texts, identifiers
//! and tags are rewritten to point into the dataset's store, which takes a
//! copy of the record's text: nothing is copied text by text, and texts the
//! program set (owned) stay as they are.

use super::list::ThinVec;
use super::node::{Extra, Node, Value};
use super::text::{Store, TagId, XrefId};

/// What moving structures from `from` into `to` maps: input offsets,
/// pieces, identifiers and tags.
pub(crate) struct Relocation<'a> {
    pub(crate) from: &'a Store,
    pub(crate) to: &'a mut Store,
    /// Where `from`'s input and pieces start in `to`'s; `None` when `to`
    /// could not take them (its offsets would pass 32 bits): texts are then
    /// copied.
    pub(crate) bases: Option<(u32, u32)>,
    xrefs: Vec<Option<XrefId>>,
    tags: Vec<Option<TagId>>,
}

impl<'a> Relocation<'a> {
    /// Appends `from`'s input and pieces to `to`'s, for the structures of
    /// `from` to move into `to`.
    pub(crate) fn new(from: &'a Store, to: &'a mut Store) -> Self {
        let bases = to.append(from);
        Self {
            from,
            to,
            bases,
            xrefs: Vec::new(),
            tags: Vec::new(),
        }
    }

    /// The id in `to` of an identifier of `from`.
    pub(crate) fn xref(&mut self, id: XrefId) -> XrefId {
        let at = id.index();
        if let Some(Some(known)) = self.xrefs.get(at) {
            return *known;
        }
        let moved = self.to.intern_xref(self.from.xref(id)).unwrap_or(id);
        if self.xrefs.len() <= at {
            self.xrefs.resize(at + 1, None);
        }
        if let Some(slot) = self.xrefs.get_mut(at) {
            *slot = Some(moved);
        }
        moved
    }

    /// The id in `to` of a tag of `from`: standard tags keep theirs.
    pub(crate) fn tag(&mut self, id: TagId) -> TagId {
        let Some(at) = id.local_index() else {
            return id;
        };
        if let Some(Some(known)) = self.tags.get(at) {
            return *known;
        }
        let moved = self.to.intern_tag(self.from.tag(id));
        if self.tags.len() <= at {
            self.tags.resize(at + 1, None);
        }
        if let Some(slot) = self.tags.get_mut(at) {
            *slot = Some(moved);
        }
        moved
    }
}

/// A value that holds texts, identifiers or tags of a store.
pub(crate) trait Relocate {
    /// Rewrites them to point into the relocation's target store.
    fn relocate(&mut self, r: &mut Relocation<'_>);
}

impl<T: Relocate> Relocate for Option<T> {
    fn relocate(&mut self, r: &mut Relocation<'_>) {
        if let Some(v) = self {
            v.relocate(r);
        }
    }
}

impl<T: Relocate> Relocate for Vec<T> {
    fn relocate(&mut self, r: &mut Relocation<'_>) {
        for v in self {
            v.relocate(r);
        }
    }
}

impl<T: Relocate> Relocate for ThinVec<T> {
    fn relocate(&mut self, r: &mut Relocation<'_>) {
        for v in self.iter_mut() {
            v.relocate(r);
        }
    }
}

impl<T: Relocate> Relocate for Box<T> {
    fn relocate(&mut self, r: &mut Relocation<'_>) {
        (**self).relocate(r);
    }
}

impl Relocate for XrefId {
    fn relocate(&mut self, r: &mut Relocation<'_>) {
        *self = r.xref(*self);
    }
}

impl Relocate for TagId {
    fn relocate(&mut self, r: &mut Relocation<'_>) {
        *self = r.tag(*self);
    }
}

impl Relocate for u32 {
    fn relocate(&mut self, _r: &mut Relocation<'_>) {}
}

impl Relocate for Value {
    fn relocate(&mut self, r: &mut Relocation<'_>) {
        match self {
            Value::None => {}
            Value::Pointer(p) => p.relocate(r),
            Value::Text(t) => t.relocate(r),
        }
    }
}

impl Relocate for Node {
    fn relocate(&mut self, r: &mut Relocation<'_>) {
        self.tag.relocate(r);
        self.xref.relocate(r);
        self.payload.relocate(r);
        self.children.relocate(r);
    }
}

impl Relocate for Extra {
    fn relocate(&mut self, r: &mut Relocation<'_>) {
        for n in self.iter_mut() {
            n.relocate(r);
        }
    }
}

/// Implements [`Relocate`] for types that hold nothing of a store.
macro_rules! relocate_nothing {
    ($($ty:ty),* $(,)?) => {$(
        impl $crate::model::relocate::Relocate for $ty {
            fn relocate(&mut self, _r: &mut $crate::model::relocate::Relocation<'_>) {}
        }
    )*};
}
pub(crate) use relocate_nothing;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Text;

    #[test]
    fn texts_identifiers_and_tags_move() {
        let mut from = Store::new("0 @I1@ INDI\n1 _X ab@@c".into());
        let id = from.intern_xref("@I1@").unwrap();
        let tag = from.intern_tag("_X");
        let span = Text::input("0 @I1@ INDI\n1 _X ab@@c", 2, 4);
        let mut to = Store::new("0 HEAD\n".into());
        let other = to.intern_xref("@F1@").unwrap();
        let mut node = Node {
            tag,
            xref: Some(id),
            payload: Value::Text(span),
            children: vec![Node::new(TagId::standard("NAME").unwrap())],
        };
        let owned = Text::new("owned");
        let mut owned_copy = owned.clone();
        {
            let mut r = Relocation::new(&from, &mut to);
            node.relocate(&mut r);
            owned_copy.relocate(&mut r);
        }
        assert_eq!(owned_copy, owned);
        assert_eq!(to.tag(node.tag), "_X");
        let moved = node.xref.unwrap();
        assert_eq!(to.xref(moved), "@I1@");
        assert_ne!(moved, other);
        let Value::Text(t) = &node.payload else {
            panic!()
        };
        assert_eq!(t.to_str(&to), "@I1@");
        assert_eq!(to.tag(node.children[0].tag), "NAME");
    }
}
