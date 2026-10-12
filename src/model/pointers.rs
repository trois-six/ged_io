//! The pointers of a structure: found, rewritten or cut, at any depth.
//!
//! Every type of the model that can hold a pointer ([`XrefId`] in a field
//! or a payload, [`Value::Pointer`] in an untyped node) implements
//! [`Pointers`], generated with the structures by `gedcom_struct!`, so that
//! a dataset counts, redirects and detaches pointers without knowing where
//! each type keeps them. Identifiers (a record's `xref`, an untyped node's)
//! are not pointers: they name, they do not point.

use super::list::ThinVec;
use super::node::{Extra, Node, Value};
use super::text::XrefId;

/// What [`Pointers::retain`] cuts: the structures whose own pointer `keep`
/// refuses, each removed with its substructures; `removed` hears of every
/// pointer that goes with them (theirs and their substructures').
pub(crate) struct Cut<'a> {
    keep: &'a mut dyn FnMut(XrefId) -> bool,
    removed: &'a mut dyn FnMut(XrefId),
    /// The number of structures removed.
    pub(crate) count: usize,
}

impl<'a> Cut<'a> {
    pub(crate) fn new(
        keep: &'a mut dyn FnMut(XrefId) -> bool,
        removed: &'a mut dyn FnMut(XrefId),
    ) -> Self {
        Self {
            keep,
            removed,
            count: 0,
        }
    }

    /// Whether `value` goes: its own pointer is refused. Its pointers are
    /// then counted out.
    fn drops<T: Pointers + ?Sized>(&mut self, value: &T) -> bool {
        match value.own() {
            Some(p) if !(self.keep)(p) => {
                value.each(self.removed);
                self.count += 1;
                true
            }
            _ => false,
        }
    }
}

/// A value of the model that may hold pointers.
pub(crate) trait Pointers {
    /// The pointer that is this structure's payload: what it points to
    /// (`FAMC @F1@` points to `@F1@`).
    fn own(&self) -> Option<XrefId> {
        None
    }

    /// Calls `f` with every pointer: its own, then its substructures', in
    /// order.
    fn each(&self, _f: &mut dyn FnMut(XrefId)) {}

    /// Rewrites every pointer to `f`'s.
    fn map(&mut self, _f: &mut dyn FnMut(XrefId) -> XrefId) {}

    /// Removes the substructures, at any depth, whose own pointer `cut`
    /// refuses. The value itself stays, whatever its own pointer.
    fn retain(&mut self, _cut: &mut Cut<'_>) {}
}

impl Pointers for XrefId {
    fn own(&self) -> Option<XrefId> {
        Some(*self)
    }

    fn each(&self, f: &mut dyn FnMut(XrefId)) {
        f(*self);
    }

    fn map(&mut self, f: &mut dyn FnMut(XrefId) -> XrefId) {
        *self = f(*self);
    }
}

impl<T: Pointers> Pointers for Option<T> {
    fn own(&self) -> Option<XrefId> {
        self.as_ref().and_then(Pointers::own)
    }

    fn each(&self, f: &mut dyn FnMut(XrefId)) {
        if let Some(v) = self {
            v.each(f);
        }
    }

    fn map(&mut self, f: &mut dyn FnMut(XrefId) -> XrefId) {
        if let Some(v) = self {
            v.map(f);
        }
    }

    fn retain(&mut self, cut: &mut Cut<'_>) {
        if let Some(v) = self {
            if cut.drops(v) {
                *self = None;
            } else {
                v.retain(cut);
            }
        }
    }
}

impl<T: Pointers> Pointers for Vec<T> {
    fn each(&self, f: &mut dyn FnMut(XrefId)) {
        for v in self {
            v.each(f);
        }
    }

    fn map(&mut self, f: &mut dyn FnMut(XrefId) -> XrefId) {
        for v in self {
            v.map(f);
        }
    }

    fn retain(&mut self, cut: &mut Cut<'_>) {
        self.retain_mut(|v| {
            let keep = !cut.drops(v);
            if keep {
                v.retain(cut);
            }
            keep
        });
    }
}

impl<T: Pointers> Pointers for ThinVec<T> {
    fn each(&self, f: &mut dyn FnMut(XrefId)) {
        for v in self {
            v.each(f);
        }
    }

    fn map(&mut self, f: &mut dyn FnMut(XrefId) -> XrefId) {
        for v in self {
            v.map(f);
        }
    }

    fn retain(&mut self, cut: &mut Cut<'_>) {
        self.retain_mut(|v| {
            let keep = !cut.drops(v);
            if keep {
                v.retain(cut);
            }
            keep
        });
    }
}

impl<T: Pointers + ?Sized> Pointers for Box<T> {
    fn own(&self) -> Option<XrefId> {
        (**self).own()
    }

    fn each(&self, f: &mut dyn FnMut(XrefId)) {
        (**self).each(f);
    }

    fn map(&mut self, f: &mut dyn FnMut(XrefId) -> XrefId) {
        (**self).map(f);
    }

    fn retain(&mut self, cut: &mut Cut<'_>) {
        Pointers::retain(&mut **self, cut);
    }
}

impl Pointers for Value {
    fn own(&self) -> Option<XrefId> {
        match self {
            Value::Pointer(p) => Some(*p),
            Value::None | Value::Text(_) => None,
        }
    }

    fn each(&self, f: &mut dyn FnMut(XrefId)) {
        if let Value::Pointer(p) = self {
            f(*p);
        }
    }

    fn map(&mut self, f: &mut dyn FnMut(XrefId) -> XrefId) {
        if let Value::Pointer(p) = self {
            *p = f(*p);
        }
    }
}

impl Pointers for Node {
    fn own(&self) -> Option<XrefId> {
        self.payload.own()
    }

    fn each(&self, f: &mut dyn FnMut(XrefId)) {
        self.payload.each(f);
        self.children.each(f);
    }

    fn map(&mut self, f: &mut dyn FnMut(XrefId) -> XrefId) {
        self.payload.map(f);
        self.children.map(f);
    }

    fn retain(&mut self, cut: &mut Cut<'_>) {
        Pointers::retain(&mut self.children, cut);
    }
}

impl Pointers for Extra {
    fn each(&self, f: &mut dyn FnMut(XrefId)) {
        (**self).each(f);
    }

    fn map(&mut self, f: &mut dyn FnMut(XrefId) -> XrefId) {
        (**self).map(f);
    }

    fn retain(&mut self, cut: &mut Cut<'_>) {
        Pointers::retain(&mut **self, cut);
    }
}

impl Pointers for u32 {}
impl Pointers for super::text::Text {}
impl Pointers for super::text::TagId {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Store;

    #[test]
    fn nodes_are_cut_with_their_pointers() {
        let mut store = Store::default();
        let a = store.intern_xref("@A@").unwrap();
        let b = store.intern_xref("@B@").unwrap();
        let tag = store.intern_tag("_X");
        let leaf = |p: XrefId| Node {
            payload: Value::Pointer(p),
            ..Node::new(tag)
        };
        let mut root = Node::new(tag);
        let mut inner = leaf(a);
        inner.children.push(leaf(b));
        root.children = vec![inner, leaf(b), Node::new(tag)];
        let mut seen = Vec::new();
        root.each(&mut |p| seen.push(p));
        assert_eq!(seen, [a, b, b]);

        let mut gone = Vec::new();
        let mut keep = |p: XrefId| p != a;
        let mut removed = |p: XrefId| gone.push(p);
        let mut cut = Cut::new(&mut keep, &mut removed);
        root.retain(&mut cut);
        assert_eq!(cut.count, 1);
        assert_eq!(gone, [a, b]);
        assert_eq!(root.children.len(), 2);

        root.map(&mut |p| if p == b { a } else { p });
        let mut seen = Vec::new();
        root.each(&mut |p| seen.push(p));
        assert_eq!(seen, [a]);
    }
}
