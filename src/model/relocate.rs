//! Moving structures from one store to another.
//!
//! A record a stream reads points into a store of its own; added to a
//! dataset ([`Dataset::extend`](super::Dataset)), its rows are copied into
//! the dataset's tables and its texts, identifiers and tags rewritten to
//! point into the dataset's store, which takes a copy of the record's text:
//! nothing is copied text by text.

use super::text::{Span, Store, TagId, XrefId};

/// What moving structures from `from` into `to` maps: text offsets,
/// pieces, big texts, identifiers and tags.
pub(crate) struct Relocation<'a> {
    pub(crate) from: &'a Store,
    pub(crate) to: &'a mut Store,
    /// Where `from`'s text, pieces and big texts start in `to`'s; `None`
    /// when `to` could not take them (its offsets would pass 32 bits):
    /// texts are then copied one by one.
    bases: Option<(u32, u32, u32)>,
    xrefs: Vec<Option<XrefId>>,
    tags: Vec<Option<TagId>>,
}

impl<'a> Relocation<'a> {
    /// Appends `from`'s text, pieces and big texts to `to`'s, for the
    /// structures of `from` to move into `to`.
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

    /// A span of `from`, in `to`.
    pub(crate) fn span(&mut self, s: Span) -> Span {
        if let Some(id) = s.as_pointer() {
            return Span::pointer(self.xref(id));
        }
        match self.bases {
            Some((text, pieces, big)) => s.shifted(text, pieces, big),
            None => self.to.copy_span(self.from, s),
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

/// Implements [`Pointers`](super::pointers::Pointers) for types that hold
/// no pointer.
macro_rules! no_pointers {
    ($($ty:ty),* $(,)?) => {$(
        impl $crate::model::pointers::Pointers for $ty {}
    )*};
}
pub(crate) use no_pointers;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts_identifiers_and_tags_move() {
        let mut from = Store::new("0 @I1@ INDI\n1 _X ab@@c".into());
        let id = from.intern_xref("@I1@").unwrap();
        let tag = from.intern_tag("_X");
        let span = Span::of_input("0 @I1@ INDI\n1 _X ab@@c", 2, 4);
        let mut to = Store::new("0 HEAD\n".into());
        let other = to.intern_xref("@F1@").unwrap();
        let (moved_span, moved, moved_tag, pointer) = {
            let mut r = Relocation::new(&from, &mut to);
            (
                r.span(span),
                r.xref(id),
                r.tag(tag),
                r.span(Span::pointer(id)),
            )
        };
        assert_eq!(to.tag(moved_tag), "_X");
        assert_eq!(to.xref(moved), "@I1@");
        assert_ne!(moved, other);
        assert_eq!(to.span_str(moved_span), "@I1@");
        assert_eq!(pointer.as_pointer(), Some(moved));
        assert_eq!(
            to.tag(TagId::standard("NAME").unwrap()),
            "NAME",
            "standard tags keep their ids"
        );
    }
}
