//! The shared text buffer of a dataset and the compact handles the model
//! keeps into it: [`Text`] for payloads, [`XrefId`] for identifiers and
//! pointers, [`TagId`] for the tags of [`Node`](super::Node)s.

use std::collections::HashMap;
use std::fmt;
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::num::NonZeroU32;

use crate::tree::{TagHasher, STANDARD_TAGS};

/// Text of the model: a payload, a value, a phrase.
///
/// Read from a file, a text is a span (32-bit offset and length) into the
/// decoded input that the dataset's [`Source`] keeps, or into its side
/// buffer when reading rewrote it (continuations joined, `@@` unescaped):
/// it costs 16 bytes and no allocation. Made by a program
/// ([`Text::new`], `From<&str>`, `From<String>`), it owns its characters,
/// and [`Text::make_owned`] turns a span into an owned copy before an edit.
///
/// A span means nothing without its source: [`Text::as_str`] takes the
/// [`Source`] (or anything that holds one, such as a
/// [`Dataset`](super::Dataset)). Equality compares the representations:
/// two spans of different sources with the same characters differ. An
/// empty text stands for an absent payload.
///
/// ```rust
/// use ged_io::next::{read_str, Text};
///
/// let data = read_str("0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @N1@ SNOTE Shared\n0 TRLR\n");
/// let owned = Text::from("Shared");
/// assert_eq!(owned.as_str(&data), "Shared");
/// assert!(Text::default().is_empty());
/// ```
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct Text(Repr);

#[derive(Clone, PartialEq, Eq, Hash)]
enum Repr {
    /// A span of the decoded input.
    Input { start: u32, len: u32 },
    /// A span of the side buffer.
    Side { start: u32, len: u32 },
    /// Characters of its own. Boxed twice so that a text stays two words
    /// with a spare niche: `Option<Text>` is 16 bytes too.
    Owned(Box<Owned>),
}

/// The characters of an owned [`Text`].
#[derive(Clone, PartialEq, Eq, Hash)]
struct Owned(Box<str>);

impl Default for Repr {
    fn default() -> Self {
        Repr::Input { start: 0, len: 0 }
    }
}

impl Text {
    /// A text that owns its characters.
    #[must_use]
    pub fn new(text: impl Into<Box<str>>) -> Self {
        let text = text.into();
        if text.is_empty() {
            Self::default()
        } else {
            Self(Repr::Owned(Box::new(Owned(text))))
        }
    }

    /// A span of the decoded input, or an owned copy when the offsets do
    /// not fit in 32 bits.
    pub(crate) fn input(input: &str, start: usize, len: usize) -> Self {
        match (u32::try_from(start), u32::try_from(len)) {
            (Ok(start), Ok(len)) if len > 0 => Self(Repr::Input { start, len }),
            _ if len == 0 => Self::default(),
            _ => Self::new(input.get(start..start + len).unwrap_or_default()),
        }
    }

    /// Appends `text` to the side buffer and spans it there, or owns a copy
    /// when the side buffer outgrew 32-bit offsets.
    pub(crate) fn side(side: &mut String, text: &str) -> Self {
        if text.is_empty() {
            return Self::default();
        }
        let start = side.len();
        match (u32::try_from(start), u32::try_from(text.len())) {
            (Ok(s), Ok(len)) if s.checked_add(len).is_some() => {
                side.push_str(text);
                Self(Repr::Side { start: s, len })
            }
            _ => Self::new(text),
        }
    }

    /// The characters, read from `source` when this text is a span of it.
    #[must_use]
    pub fn as_str<'a, S: AsRef<Source> + ?Sized>(&'a self, source: &'a S) -> &'a str {
        let source = source.as_ref();
        match &self.0 {
            Repr::Input { start, len } => span(&source.input, *start, *len),
            Repr::Side { start, len } => span(&source.side, *start, *len),
            Repr::Owned(owned) => &owned.0,
        }
    }

    /// The characters when this text owns them; `None` for a span.
    #[must_use]
    pub fn as_owned(&self) -> Option<&str> {
        match &self.0 {
            Repr::Owned(owned) => Some(&owned.0),
            Repr::Input { len: 0, .. } => Some(""),
            _ => None,
        }
    }

    /// Whether the text is empty (an absent payload).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match &self.0 {
            Repr::Input { len, .. } | Repr::Side { len, .. } => *len == 0,
            Repr::Owned(owned) => owned.0.is_empty(),
        }
    }

    /// The length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        match &self.0 {
            Repr::Input { len, .. } | Repr::Side { len, .. } => *len as usize,
            Repr::Owned(owned) => owned.0.len(),
        }
    }

    /// Turns a span into an owned copy of its characters (copy on write),
    /// so that the text no longer depends on `source`.
    pub fn make_owned<S: AsRef<Source> + ?Sized>(&mut self, source: &S) {
        if !matches!(self.0, Repr::Owned(_)) && !self.is_empty() {
            *self = Self::new(self.as_str(source));
        }
    }
}

fn span(buffer: &str, start: u32, len: u32) -> &str {
    let start = start as usize;
    buffer
        .get(start..start.saturating_add(len as usize))
        .unwrap_or_default()
}

impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Repr::Input { start, len } => write!(f, "Text(input {start}+{len})"),
            Repr::Side { start, len } => write!(f, "Text(side {start}+{len})"),
            Repr::Owned(owned) => write!(f, "Text({:?})", owned.0),
        }
    }
}

impl From<&str> for Text {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

impl From<String> for Text {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

impl From<Box<str>> for Text {
    fn from(text: Box<str>) -> Self {
        Self::new(text)
    }
}

/// A cross-reference identifier of a dataset (`@I1@`), interned: the
/// identifier of a record and every pointer to it share one 32-bit id.
///
/// [`Source::xref`] gives its text, delimiters included;
/// [`Source::find_xref`] finds the id of a text and [`Source::intern_xref`]
/// makes one. Pointers to identifiers no record holds have ids too.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct XrefId(NonZeroU32);

impl XrefId {
    /// The position in the table of identifiers.
    fn index(self) -> usize {
        self.0.get() as usize - 1
    }

    fn from_index(index: usize) -> Option<Self> {
        u32::try_from(index + 1)
            .ok()
            .and_then(NonZeroU32::new)
            .map(Self)
    }
}

/// The tag of a [`Node`](super::Node): a standard tag's index in the
/// crate's table of every 5.5.1, 7.0 and 7.1 tag, or an index past it into
/// the tags of the dataset ([`Source::tag`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TagId(u32);

impl TagId {
    /// The id of a standard tag; `None` for any other tag, which only a
    /// [`Source`] can intern ([`Source::intern_tag`]).
    #[must_use]
    pub fn standard(tag: &str) -> Option<Self> {
        crate::tree::standard_index(tag).map(|i| Self(u32::from(i)))
    }

    /// The id of the empty tag: the tag of a line that had no level.
    pub(crate) const fn raw(id: u32) -> Self {
        Self(id)
    }

    pub(crate) const fn get(self) -> u32 {
        self.0
    }
}

/// The identifiers of a dataset: their texts, by id, and a hash index from
/// text to id. Identifiers that hash alike are chained.
#[derive(Clone, Default)]
pub(crate) struct XrefTable {
    names: Vec<Text>,
    /// Hash of a text to the most recent id with that hash.
    index: HashMap<u64, u32, BuildHasherDefault<IdHasher>>,
    /// For each id, the previous id with the same hash (0: none).
    chain: Vec<u32>,
}

/// The hasher of [`XrefTable::index`], whose keys are hashes already.
#[derive(Default)]
pub(crate) struct IdHasher(u64);

impl Hasher for IdHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(b);
        }
    }

    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

fn hash(text: &str) -> u64 {
    let mut h = TagHasher::default();
    text.hash(&mut h);
    h.finish()
}

impl XrefTable {
    fn find(&self, text: &str, input: &str, side: &str) -> Option<XrefId> {
        let mut id = *self.index.get(&hash(text))?;
        while id != 0 {
            let i = id as usize - 1;
            let name = self.names.get(i)?;
            if resolve(name, input, side) == text {
                return XrefId::from_index(i);
            }
            id = self.chain.get(i).copied().unwrap_or(0);
        }
        None
    }

    /// The id of `text`; a new one, holding `make()`, when it has none.
    pub(crate) fn intern(
        &mut self,
        text: &str,
        input: &str,
        side: &str,
        make: impl FnOnce() -> Text,
    ) -> Option<XrefId> {
        if let Some(id) = self.find(text, input, side) {
            return Some(id);
        }
        let id = XrefId::from_index(self.names.len())?;
        self.names.push(make());
        let previous = self.index.insert(hash(text), id.0.get()).unwrap_or(0);
        self.chain.push(previous);
        Some(id)
    }

    pub(crate) fn len(&self) -> usize {
        self.names.len()
    }

    pub(crate) fn shrink(&mut self) {
        self.names.shrink_to_fit();
        self.chain.shrink_to_fit();
        self.index.shrink_to_fit();
    }
}

/// The characters of a text, given the buffers of its source.
fn resolve<'a>(text: &'a Text, input: &'a str, side: &'a str) -> &'a str {
    match &text.0 {
        Repr::Input { start, len } => span(input, *start, *len),
        Repr::Side { start, len } => span(side, *start, *len),
        Repr::Owned(owned) => &owned.0,
    }
}

/// The text a dataset was read from, and the tables its model points into.
///
/// It holds the decoded input, a side buffer for the payloads reading
/// rewrote, the identifiers ([`XrefId`]) and the tags that are not standard
/// ([`TagId`]). Every [`Text`] span and id of a dataset resolves against
/// its source.
#[derive(Clone, Default)]
pub struct Source {
    input: String,
    side: String,
    xrefs: XrefTable,
    tags: Vec<Box<str>>,
}

impl Source {
    pub(crate) fn new(input: String) -> Self {
        Self {
            input,
            ..Self::default()
        }
    }

    /// The decoded input, which spans point into.
    #[cfg(test)]
    pub(crate) fn input(&self) -> &str {
        &self.input
    }

    /// The buffers being filled while reading: the input, the side buffer
    /// and the identifiers.
    pub(crate) fn parts_mut(&mut self) -> (&str, &mut String, &mut XrefTable) {
        (&self.input, &mut self.side, &mut self.xrefs)
    }

    pub(crate) fn set_tags(&mut self, tags: Vec<Box<str>>) {
        self.tags = tags;
    }

    pub(crate) fn shrink(&mut self) {
        self.side.shrink_to_fit();
        self.xrefs.shrink();
    }

    /// The text of an identifier, delimiters included: `@I1@`.
    #[must_use]
    pub fn xref(&self, id: XrefId) -> &str {
        self.xrefs
            .names
            .get(id.index())
            .map_or("", |t| resolve(t, &self.input, &self.side))
    }

    /// The id of an identifier, if the dataset has it.
    #[must_use]
    pub fn find_xref(&self, xref: &str) -> Option<XrefId> {
        self.xrefs.find(xref, &self.input, &self.side)
    }

    /// The id of an identifier, made when the dataset does not have it
    /// yet. `None` only when four billion identifiers are taken.
    pub fn intern_xref(&mut self, xref: &str) -> Option<XrefId> {
        self.xrefs
            .intern(xref, &self.input, &self.side, || Text::new(xref))
    }

    /// The number of identifiers, pointers to no record included.
    #[must_use]
    pub fn xref_count(&self) -> usize {
        self.xrefs.len()
    }

    /// The text of a tag.
    #[must_use]
    pub fn tag(&self, id: TagId) -> &str {
        let i = id.0 as usize;
        match STANDARD_TAGS.get(i) {
            Some(tag) => tag,
            None => self.tags.get(i - STANDARD_TAGS.len()).map_or("", |t| t),
        }
    }

    /// The id of a tag, made when it is neither standard nor known to the
    /// dataset yet.
    pub fn intern_tag(&mut self, tag: &str) -> TagId {
        if let Some(id) = TagId::standard(tag) {
            return id;
        }
        let at = self
            .tags
            .iter()
            .position(|t| &**t == tag)
            .unwrap_or_else(|| {
                self.tags.push(tag.into());
                self.tags.len() - 1
            });
        TagId(u32::try_from(STANDARD_TAGS.len() + at).unwrap_or(u32::MAX))
    }

    /// Bytes held: the input, the side buffer and the tables.
    #[must_use]
    pub fn heap_size(&self) -> usize {
        self.input.capacity()
            + self.side.capacity()
            + self.xrefs.names.capacity() * std::mem::size_of::<Text>()
            + self.xrefs.chain.capacity() * 4
            + self.xrefs.index.capacity() * 16
            + self.tags.iter().map(|t| t.len() + 16).sum::<usize>()
    }
}

impl AsRef<Source> for Source {
    fn as_ref(&self) -> &Source {
        self
    }
}

impl fmt::Debug for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Source")
            .field("input", &self.input.len())
            .field("side", &self.side.len())
            .field("xrefs", &self.xrefs.len())
            .field("tags", &self.tags)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_owned_and_empty() {
        let mut source = Source::new("0 @I1@ INDI".into());
        let t = Text::input(source.input(), 2, 4);
        assert_eq!(t.as_str(&source), "@I1@");
        let (_, side, _) = source.parts_mut();
        let s = Text::side(side, "joined\ntext");
        assert_eq!(s.as_str(&source), "joined\ntext");
        let mut copy = t.clone();
        copy.make_owned(&source);
        assert_eq!(copy.as_owned(), Some("@I1@"));
        assert_eq!(Text::from(""), Text::default());
        assert!(Text::input("", 0, 0).is_empty());
    }

    #[test]
    fn identifiers_are_interned_once() {
        let mut source = Source::new("@I1@ @I1@ @F1@".into());
        let a = source.intern_xref("@I1@").unwrap();
        let b = source.intern_xref("@I1@").unwrap();
        let c = source.intern_xref("@F1@").unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(source.xref(c), "@F1@");
        assert_eq!(source.find_xref("@F1@"), Some(c));
        assert_eq!(source.find_xref("@X@"), None);
        assert_eq!(source.xref_count(), 2);
    }

    #[test]
    fn tags() {
        let mut source = Source::default();
        let name = TagId::standard("NAME").unwrap();
        assert_eq!(source.tag(name), "NAME");
        let ext = source.intern_tag("_X");
        assert_eq!(source.intern_tag("_X"), ext);
        assert_eq!(source.tag(ext), "_X");
    }

    #[test]
    fn sizes() {
        assert_eq!(std::mem::size_of::<Text>(), 16);
        assert_eq!(std::mem::size_of::<Option<Text>>(), 16);
        assert_eq!(std::mem::size_of::<Option<XrefId>>(), 4);
    }
}
