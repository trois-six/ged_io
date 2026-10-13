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
            Some(Inner::Many(items)) => push_exact(items, item),
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

    /// Removes and returns the item at `index`, shifting the items after
    /// it.
    ///
    /// # Panics
    ///
    /// When `index` is out of bounds, as `Vec::remove`.
    pub fn remove(&mut self, index: usize) -> T {
        let mut items = std::mem::take(self).into_vec();
        let item = items.remove(index);
        *self = items.into();
        item
    }

    /// Keeps the items for which `keep` is true, in order.
    pub fn retain(&mut self, mut keep: impl FnMut(&T) -> bool) {
        self.retain_mut(|item| keep(item));
    }

    /// Keeps the items for which `keep` is true, in order, letting it edit
    /// each.
    pub fn retain_mut(&mut self, mut keep: impl FnMut(&mut T) -> bool) {
        match self.0.as_deref_mut() {
            None => {}
            Some(Inner::One(item)) => {
                if !keep(item) {
                    self.0 = None;
                }
            }
            Some(Inner::Many(items)) => {
                let before = items.len();
                items.retain_mut(keep);
                if items.len() < before {
                    *self = std::mem::take(items).into();
                }
            }
        }
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

/// Pushes onto a list that is usually short and is kept: short lists grow
/// one item at a time, so that they need no shrinking (a reallocation);
/// longer ones double.
fn push_exact<T>(list: &mut Vec<T>, item: T) {
    if list.len() == list.capacity() {
        if list.len() < 8 {
            list.reserve_exact(1);
        } else {
            list.reserve(list.len());
        }
    }
    list.push(item);
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
        let mut list: ThinVec<u8> = vec![1, 2, 3, 4].into();
        list.retain(|&i| i % 2 == 0);
        assert_eq!(&*list, [2, 4]);
        assert_eq!(list.remove(0), 2);
        assert_eq!(&*list, [4]);
        list.retain_mut(|i| {
            *i += 1;
            true
        });
        assert_eq!(&*list, [5]);
        list.retain(|_| false);
        assert!(list.is_empty());
        assert_eq!(std::mem::size_of::<ThinVec<String>>(), 8);
    }
}
