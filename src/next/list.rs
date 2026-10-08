//! A list that costs one word when empty and one allocation for one item,
//! for substructures that may repeat but seldom appear more than once.

use std::ops::{Deref, DerefMut};

/// A list of structures that most structures have at most one of (notes,
/// citations, translations, identifiers, …): one word when empty; one
/// allocation holding the item when it has one; a boxed `Vec` beyond. It
/// dereferences to a slice, which edits the items in place;
/// [`ThinVec::push`], [`ThinVec::clear`], `From<Vec<T>>` and
/// [`ThinVec::into_vec`] change them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThinVec<T>(Option<Box<Inner<T>>>);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Inner<T> {
    One(T),
    Many(Vec<T>),
}

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
        match self.0.as_deref_mut() {
            None => self.0 = Some(Box::new(Inner::One(item))),
            Some(Inner::Many(items)) => super::driver::push_exact(items, item),
            Some(inner @ Inner::One(_)) => {
                let first = std::mem::replace(inner, Inner::Many(Vec::new()));
                if let (Inner::One(first), Inner::Many(items)) = (first, inner) {
                    items.reserve_exact(2);
                    items.push(first);
                    items.push(item);
                }
            }
        }
    }

    /// Removes every item.
    pub fn clear(&mut self) {
        self.0 = None;
    }

    /// The items, as a `Vec`.
    #[must_use]
    pub fn into_vec(self) -> Vec<T> {
        match self.0.map(|inner| *inner) {
            None => Vec::new(),
            Some(Inner::One(item)) => vec![item],
            Some(Inner::Many(items)) => items,
        }
    }

    /// Releases spare capacity.
    pub(crate) fn shrink(&mut self) {
        if let Some(Inner::Many(items)) = self.0.as_deref_mut() {
            if items.capacity() > items.len() {
                items.shrink_to_fit();
            }
        }
    }
}

impl<T> Deref for ThinVec<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        match self.0.as_deref() {
            None => &[],
            Some(Inner::One(item)) => std::slice::from_ref(item),
            Some(Inner::Many(items)) => items,
        }
    }
}

impl<T> DerefMut for ThinVec<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        match self.0.as_deref_mut() {
            None => &mut [],
            Some(Inner::One(item)) => std::slice::from_mut(item),
            Some(Inner::Many(items)) => items,
        }
    }
}

impl<T> From<Vec<T>> for ThinVec<T> {
    fn from(mut items: Vec<T>) -> Self {
        match items.len() {
            0 => Self(None),
            1 => Self(items.pop().map(|item| Box::new(Inner::One(item)))),
            _ => Self(Some(Box::new(Inner::Many(items)))),
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

impl<'a, T> IntoIterator for &'a mut ThinVec<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<T> IntoIterator for ThinVec<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_vec().into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_item_then_many() {
        let mut list = ThinVec::new();
        assert!(list.is_empty());
        list.push(1);
        assert_eq!(&*list, [1]);
        list.push(2);
        list.push(3);
        assert_eq!(&*list, [1, 2, 3]);
        list[1] = 5;
        assert_eq!(list.clone().into_vec(), [1, 5, 3]);
        let mut one: ThinVec<u8> = vec![7].into();
        one[0] = 8;
        assert_eq!(&*one, [8]);
        one.clear();
        assert!(one.is_empty());
        assert_eq!(std::mem::size_of::<ThinVec<String>>(), 8);
    }
}
