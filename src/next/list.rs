//! A list that costs one word when empty, for substructures that may
//! repeat but seldom appear at all.

use std::ops::Deref;

use crate::tree::{Flat, Structure};

use super::driver::{FromNode, NodeRef, ReadCx, Slot, ToNodes, WriteCx};

/// A list of structures that most structures do not have (translations,
/// external identifiers, phonetic variations, …): one word when empty,
/// a boxed `Vec` otherwise. It dereferences to a slice; [`ThinVec::push`]
/// and [`ThinVec::as_vec_mut`] edit it.
// Boxing the vector keeps an empty list, the common case, to one word
// instead of three.
#[allow(clippy::box_collection)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThinVec<T>(Option<Box<Vec<T>>>);

impl<T> Default for ThinVec<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<T> ThinVec<T> {
    /// An empty list.
    #[must_use]
    pub const fn new() -> Self {
        Self(None)
    }

    /// Appends an item.
    pub fn push(&mut self, item: T) {
        super::driver::push_exact(self.as_vec_mut(), item);
    }

    /// The items, to edit.
    pub fn as_vec_mut(&mut self) -> &mut Vec<T> {
        self.0.get_or_insert_with(Box::default)
    }
}

impl<T> Deref for ThinVec<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        self.0.as_deref().map_or(&[], Vec::as_slice)
    }
}

impl<T> From<Vec<T>> for ThinVec<T> {
    fn from(items: Vec<T>) -> Self {
        if items.is_empty() {
            Self(None)
        } else {
            Self(Some(Box::new(items)))
        }
    }
}

impl<T> FromIterator<T> for ThinVec<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        iter.into_iter().collect::<Vec<_>>().into()
    }
}

impl<'a, T> IntoIterator for &'a ThinVec<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
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
        if let Some(v) = &mut self.0 {
            if v.capacity() > v.len() {
                v.shrink_to_fit();
            }
        }
    }
}
