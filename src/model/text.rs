//! The shared text store of a dataset and the compact handles the model
//! keeps into it: [`Text`] for payloads, [`XrefId`] for identifiers and
//! pointers, [`TagId`] for the tags of [`Node`](super::Node)s.

use std::borrow::Cow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::num::NonZeroU32;

use crate::tree::{TagHasher, TextPiece, STANDARD_TAGS};

/// Text of the model: a payload, a value, a phrase.
///
/// Read from a file, a text points into the decoded input that the
/// dataset's [`Store`] keeps, and costs 16 bytes and no allocation: a span
/// (32-bit offset and length) of the input, or, when reading rewrote it
/// (continuation lines joined, `@@` unescaped), a run of the store's
/// pieces — spans of the input, each after a newline or not — so that
/// nothing is copied. Made by a program ([`Text::new`], `From<&str>`,
/// `From<String>`), it owns its characters, and [`Text::make_owned`] turns
/// a text read into an owned copy before an edit.
///
/// A text read means nothing without its store: [`Text::to_str`] takes the
/// [`Store`] (or anything that holds one, such as a
/// [`Dataset`](super::Dataset)), and borrows its characters unless they
/// are joined from several pieces; [`Text::chunks`] gives them without a
/// copy. Equality compares the representations: two texts of different
/// stores with the same characters differ. An empty text stands for an
/// absent payload.
///
/// ```rust
/// use ged_io::model::{Dataset, Text};
///
/// let data = Dataset::parse("0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @N1@ SNOTE Shared\n1 CONT text\n0 TRLR\n");
/// let text = &data.notes[0].text;
/// assert_eq!(text.to_str(&data), "Shared\ntext");
/// assert_eq!(text.chunks(&data).collect::<Vec<_>>(), ["Shared", "\n", "text"]);
/// assert_eq!(Text::from("Shared").to_str(&data), "Shared");
/// assert!(Text::default().is_empty());
/// ```
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct Text(Repr);

#[derive(Clone, PartialEq, Eq, Hash)]
enum Repr {
    /// A span of the decoded input.
    Input { start: u32, len: u32 },
    /// `count` pieces of the store, from `start`.
    Pieces { start: u32, count: u32 },
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

    /// The text made of `pieces[at..]`, which the caller just pushed: a
    /// plain span when there is one piece without a newline (it is then
    /// taken back), nothing when there is none.
    pub(crate) fn from_pieces(input: &str, pieces: &mut Vec<TextPiece>, at: usize) -> Self {
        match pieces.get(at..) {
            None | Some([]) => Self::default(),
            Some(&[piece]) if !piece.newline() => {
                pieces.truncate(at);
                Self::input(input, piece.start() as usize, piece.len() as usize)
            }
            Some(run) => {
                if let (Ok(start), Ok(count)) = (u32::try_from(at), u32::try_from(run.len())) {
                    Self(Repr::Pieces { start, count })
                } else {
                    let text = join(input, run);
                    pieces.truncate(at);
                    Self::new(text)
                }
            }
        }
    }

    /// The characters, read from `store` when this text points into it:
    /// borrowed, unless they are joined from several pieces.
    #[must_use]
    pub fn to_str<'a, S: AsRef<Store> + ?Sized>(&'a self, store: &'a S) -> Cow<'a, str> {
        let store = store.as_ref();
        match &self.0 {
            Repr::Input { start, len } => Cow::Borrowed(span(&store.input, *start, *len)),
            Repr::Owned(owned) => Cow::Borrowed(&owned.0),
            Repr::Pieces { .. } => Cow::Owned(join(&store.input, self.pieces(store))),
        }
    }

    /// The characters in order, without a copy: the spans of the input and
    /// the newlines between them (a text of one span is one chunk).
    pub fn chunks<'a, S: AsRef<Store> + ?Sized>(
        &'a self,
        store: &'a S,
    ) -> impl Iterator<Item = &'a str> + 'a {
        let store = store.as_ref();
        let (single, pieces): (Option<&str>, &[TextPiece]) = match &self.0 {
            Repr::Input { start, len } => (Some(span(&store.input, *start, *len)), &[]),
            Repr::Owned(owned) => (Some(&owned.0), &[]),
            Repr::Pieces { .. } => (None, self.pieces(store)),
        };
        single
            .filter(|s| !s.is_empty())
            .into_iter()
            .chain(pieces.iter().flat_map(move |p| {
                let text = span(&store.input, p.start(), p.len());
                p.newline()
                    .then_some("\n")
                    .into_iter()
                    .chain(Some(text).filter(|t| !t.is_empty()))
            }))
    }

    /// Whether the characters are `other`, without a copy.
    #[must_use]
    pub fn eq_str<S: AsRef<Store> + ?Sized>(&self, store: &S, other: &str) -> bool {
        let mut rest = other;
        for chunk in self.chunks(store) {
            match rest.strip_prefix(chunk) {
                Some(r) => rest = r,
                None => return false,
            }
        }
        rest.is_empty()
    }

    fn pieces<'a>(&self, store: &'a Store) -> &'a [TextPiece] {
        match &self.0 {
            Repr::Pieces { start, count } => {
                let start = *start as usize;
                store
                    .pieces
                    .get(start..start.saturating_add(*count as usize))
                    .unwrap_or_default()
            }
            _ => &[],
        }
    }

    /// The characters when this text owns them; `None` for a text read.
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
            Repr::Input { len, .. } => *len == 0,
            // Pieces are only made of a non-empty text.
            Repr::Pieces { .. } => false,
            Repr::Owned(owned) => owned.0.is_empty(),
        }
    }

    /// The length in bytes.
    #[must_use]
    pub fn len<S: AsRef<Store> + ?Sized>(&self, store: &S) -> usize {
        self.chunks(store).map(str::len).sum()
    }

    /// Turns a text read into an owned copy of its characters (copy on
    /// write), so that it no longer depends on `store`.
    pub fn make_owned<S: AsRef<Store> + ?Sized>(&mut self, store: &S) {
        if !matches!(self.0, Repr::Owned(_)) && !self.is_empty() {
            *self = Self::new(self.to_str(store));
        }
    }
}

fn span(buffer: &str, start: u32, len: u32) -> &str {
    let start = start as usize;
    buffer
        .get(start..start.saturating_add(len as usize))
        .unwrap_or_default()
}

/// The characters of `pieces` of `input`, joined.
fn join(input: &str, pieces: &[TextPiece]) -> String {
    let mut out = String::with_capacity(pieces.iter().map(|p| p.len() as usize + 1).sum());
    for p in pieces {
        if p.newline() {
            out.push('\n');
        }
        out.push_str(span(input, p.start(), p.len()));
    }
    out
}

impl super::relocate::Relocate for Text {
    fn relocate(&mut self, r: &mut super::relocate::Relocation<'_>) {
        match (&mut self.0, r.bases) {
            (Repr::Owned(_) | Repr::Input { len: 0, .. }, _) => {}
            (Repr::Input { start, .. }, Some((input, _))) => *start += input,
            (Repr::Pieces { start, .. }, Some((_, pieces))) => *start += pieces,
            (Repr::Input { .. } | Repr::Pieces { .. }, None) => {
                *self = Self::new(self.to_str(r.from));
            }
        }
    }
}

impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Repr::Input { start, len } => write!(f, "Text(input {start}+{len})"),
            Repr::Pieces { start, count } => write!(f, "Text(pieces {start}+{count})"),
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

impl From<Cow<'_, str>> for Text {
    fn from(text: Cow<'_, str>) -> Self {
        Self::new(text)
    }
}

/// A cross-reference identifier of a dataset (`@I1@`), interned: the
/// identifier of a record and every pointer to it share one 32-bit id.
///
/// [`Store::xref`] gives its text, delimiters included;
/// [`Store::find_xref`] finds the id of a text and [`Store::intern_xref`]
/// makes one. Pointers to identifiers no record holds have ids too.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct XrefId(NonZeroU32);

impl XrefId {
    /// The position in the table of identifiers.
    pub(crate) fn index(self) -> usize {
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
/// the tags of the dataset ([`Store::tag`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TagId(u32);

impl TagId {
    /// The id of a standard tag; `None` for any other tag, which only a
    /// [`Store`] can intern ([`Store::intern_tag`]).
    #[must_use]
    pub fn standard(tag: &str) -> Option<Self> {
        crate::tree::standard_index(tag).map(|i| Self(u32::from(i)))
    }

    /// The tag of the node a typed structure keeps in its `extra` for the
    /// payload or identifier its type has no place for (text where its
    /// pointer belongs, a payload on a record): it is written back as the
    /// structure's own. It names no tag ([`Store::tag`] gives `""`).
    pub const ASIDE: Self = Self(u32::MAX);

    /// The id of a tag of the lexer's arena.
    pub(crate) const fn raw(id: u32) -> Self {
        Self(id)
    }

    pub(crate) const fn get(self) -> u32 {
        self.0
    }

    /// The index among the tags of a store, for a tag that is neither
    /// standard nor [`TagId::ASIDE`].
    pub(crate) fn local_index(self) -> Option<usize> {
        (self != Self::ASIDE)
            .then(|| (self.0 as usize).checked_sub(STANDARD_TAGS.len()))
            .flatten()
    }
}

/// The identifiers of a dataset: their texts, by id, and an
/// open-addressing index from text to id.
///
/// The texts are copied together into one buffer, ended by a 32-bit offset
/// each: a writer resolving every pointer reads them there, not across the
/// whole input. The index has 8 bytes a slot (the id and 32 bits of its
/// text's hash) and is kept at most three quarters full.
#[derive(Clone, Default)]
pub(crate) struct XrefTable {
    /// The texts, one after the other.
    names: String,
    /// For each id, the end of its text in `names`.
    ends: Vec<u32>,
    /// `0`: empty; otherwise the hash (high half) and the id (low half).
    slots: Vec<u64>,
}

fn hash(text: &str) -> u32 {
    let mut h = TagHasher::default();
    text.hash(&mut h);
    // The finished hash's low half, which the table's index and tag use.
    #[allow(clippy::cast_possible_truncation)] // Truncation is the intent.
    let h = h.finish() as u32;
    h
}

impl XrefTable {
    /// The text of the identifier at `index`.
    fn name(&self, index: usize) -> Option<&str> {
        let end = *self.ends.get(index)? as usize;
        let start = index
            .checked_sub(1)
            .and_then(|i| self.ends.get(i))
            .map_or(0, |&e| e as usize);
        self.names.get(start..end)
    }

    /// The slot of `text`: where it is, or the empty slot it would take.
    fn slot(&self, text: &str, hash: u32) -> (usize, Option<XrefId>) {
        let mask = self.slots.len().wrapping_sub(1);
        let mut i = hash as usize & mask;
        loop {
            let Some(&s) = self.slots.get(i) else {
                return (i, None);
            };
            if s == 0 {
                return (i, None);
            }
            #[allow(clippy::cast_possible_truncation)] // The id is the low half.
            let id = s as u32;
            if (s >> 32) as u32 == hash && self.name(id as usize - 1) == Some(text) {
                return (i, XrefId::from_index(id as usize - 1));
            }
            i = (i + 1) & mask;
        }
    }

    fn find(&self, text: &str) -> Option<XrefId> {
        if self.slots.is_empty() {
            return None;
        }
        self.slot(text, hash(text)).1
    }

    /// The id of `text`; a new one when it has none. `None` when four
    /// billion identifiers, or 4 GiB of them, are taken.
    pub(crate) fn intern(&mut self, text: &str) -> Option<XrefId> {
        if (self.ends.len() + 1) * 4 > self.slots.len() * 3 {
            self.grow();
        }
        let hash = hash(text);
        let (at, found) = self.slot(text, hash);
        if found.is_some() {
            return found;
        }
        let id = XrefId::from_index(self.ends.len())?;
        let end = u32::try_from(self.names.len() + text.len()).ok()?;
        self.names.push_str(text);
        self.ends.push(end);
        if let Some(slot) = self.slots.get_mut(at) {
            *slot = (u64::from(hash) << 32) | u64::from(id.0.get());
        }
        Some(id)
    }

    /// Doubles the index, at least 64 slots.
    fn grow(&mut self) {
        let len = (self.slots.len() * 2).max(64);
        let old = std::mem::replace(&mut self.slots, vec![0; len]);
        let mask = len - 1;
        for s in old.into_iter().filter(|&s| s != 0) {
            let mut i = (s >> 32) as usize & mask;
            while self.slots.get(i).is_some_and(|&t| t != 0) {
                i = (i + 1) & mask;
            }
            if let Some(slot) = self.slots.get_mut(i) {
                *slot = s;
            }
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.ends.len()
    }

    pub(crate) fn shrink(&mut self) {
        self.names.shrink_to_fit();
        self.ends.shrink_to_fit();
    }

    fn heap_size(&self) -> usize {
        self.names.capacity() + self.ends.capacity() * 4 + self.slots.capacity() * 8
    }
}

/// The text a dataset was read from, and the tables its model points into.
///
/// It holds the decoded input, the pieces of the texts reading rewrote
/// (8 bytes each, instead of a copy of their characters), the identifiers
/// ([`XrefId`]) and the tags that are not standard ([`TagId`]). Every
/// [`Text`] and id of a dataset resolves against its store.
#[derive(Clone, Default)]
pub struct Store {
    input: String,
    pieces: Vec<TextPiece>,
    xrefs: XrefTable,
    tags: Vec<Box<str>>,
}

impl Store {
    pub(crate) fn new(input: String) -> Self {
        Self {
            input,
            ..Self::default()
        }
    }

    /// The decoded input, which texts point into.
    #[cfg(test)]
    pub(crate) fn input(&self) -> &str {
        &self.input
    }

    /// The tables being filled while reading: the input, the pieces and
    /// the identifiers.
    pub(crate) fn parts_mut(&mut self) -> (&str, &mut Vec<TextPiece>, &mut XrefTable) {
        (&self.input, &mut self.pieces, &mut self.xrefs)
    }

    pub(crate) fn set_tags(&mut self, tags: Vec<Box<str>>) {
        self.tags = tags;
    }

    pub(crate) fn shrink(&mut self) {
        self.pieces.shrink_to_fit();
        self.xrefs.shrink();
    }

    /// Appends the input and pieces of `from`, for its texts to move here
    /// ([`Relocation`](super::relocate::Relocation)): where they start in
    /// this store's, or `None` when the offsets would pass the 32 bits
    /// texts hold (nothing is appended then).
    pub(crate) fn append(&mut self, from: &Store) -> Option<(u32, u32)> {
        let input = u32::try_from(self.input.len()).ok()?;
        let pieces = u32::try_from(self.pieces.len()).ok()?;
        let end = self.input.len().checked_add(from.input.len())?;
        if end > (u32::MAX / 2) as usize
            || u32::try_from(pieces as usize + from.pieces.len()).is_err()
        {
            return None;
        }
        self.input.push_str(&from.input);
        self.pieces
            .extend(from.pieces.iter().map(|p| p.shifted(input)));
        Some((input, pieces))
    }

    /// The text of an identifier, delimiters included: `@I1@`.
    #[must_use]
    pub fn xref(&self, id: XrefId) -> &str {
        self.xrefs.name(id.index()).unwrap_or("")
    }

    /// The id of an identifier, if the dataset has it.
    #[must_use]
    pub fn find_xref(&self, xref: &str) -> Option<XrefId> {
        self.xrefs.find(xref)
    }

    /// The id of an identifier, made when the dataset does not have it
    /// yet. `None` only when four billion identifiers are taken.
    pub fn intern_xref(&mut self, xref: &str) -> Option<XrefId> {
        self.xrefs.intern(xref)
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
            None => self
                .tags
                .get(i.saturating_sub(STANDARD_TAGS.len()))
                .map_or("", |t| t),
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

    /// Bytes held: the input, the pieces and the tables.
    #[must_use]
    pub fn heap_size(&self) -> usize {
        self.input.capacity()
            + self.pieces.capacity() * std::mem::size_of::<TextPiece>()
            + self.xrefs.heap_size()
            + self.tags.iter().map(|t| t.len() + 16).sum::<usize>()
    }
}

/// Equal stores hold the same input, pieces, identifiers (in the same
/// order) and tags: what their texts and ids resolve to.
impl PartialEq for Store {
    fn eq(&self, other: &Self) -> bool {
        self.input == other.input
            && self.pieces == other.pieces
            && self.xrefs.names == other.xrefs.names
            && self.xrefs.ends == other.xrefs.ends
            && self.tags == other.tags
    }
}

impl AsRef<Store> for Store {
    fn as_ref(&self) -> &Store {
        self
    }
}

impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Store")
            .field("input", &self.input.len())
            .field("pieces", &self.pieces.len())
            .field("xrefs", &self.xrefs.len())
            .field("tags", &self.tags)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_pieces_owned_and_empty() {
        let mut store = Store::new("0 @I1@ INDI\nab@@cd".into());
        let t = Text::input(store.input(), 2, 4);
        assert_eq!(t.to_str(&store), "@I1@");
        let (_, pieces, _) = store.parts_mut();
        pieces.extend([
            TextPiece::new(12, 3, false),
            TextPiece::new(16, 2, false),
            TextPiece::new(7, 4, true),
        ]);
        let joined = Text(Repr::Pieces { start: 0, count: 3 });
        assert_eq!(joined.to_str(&store), "ab@cd\nINDI");
        assert_eq!(joined.len(&store), 10);
        assert!(joined.eq_str(&store, "ab@cd\nINDI"));
        assert!(!joined.eq_str(&store, "ab@cd\nIND"));
        assert!(!joined.eq_str(&store, "ab@cd\nINDIX"));
        let mut copy = joined.clone();
        copy.make_owned(&store);
        assert_eq!(copy.as_owned(), Some("ab@cd\nINDI"));
        assert_eq!(Text::from(""), Text::default());
        assert!(Text::input("", 0, 0).is_empty());
        assert_eq!(Text::default().chunks(&store).count(), 0);
    }

    #[test]
    fn a_single_piece_is_a_span() {
        let mut store = Store::new("abcdef".into());
        let (input, pieces, _) = store.parts_mut();
        pieces.push(TextPiece::new(1, 3, false));
        let t = Text::from_pieces(input, pieces, 0);
        assert!(pieces.is_empty());
        assert_eq!(t, Text(Repr::Input { start: 1, len: 3 }));
        assert_eq!(Text::from_pieces(input, pieces, 0), Text::default());
    }

    #[test]
    fn identifiers_are_interned_once() {
        let mut store = Store::new("@I1@ @I1@ @F1@".into());
        let a = store.intern_xref("@I1@").unwrap();
        let b = store.intern_xref("@I1@").unwrap();
        let c = store.intern_xref("@F1@").unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(store.xref(c), "@F1@");
        assert_eq!(store.find_xref("@F1@"), Some(c));
        assert_eq!(store.find_xref("@X@"), None);
        assert_eq!(store.xref_count(), 2);
    }

    #[test]
    fn many_identifiers() {
        let text: String = (0..5000).map(|i| format!("@I{i}@")).collect();
        let mut store = Store::new(text.clone());
        let (_, _, xrefs) = store.parts_mut();
        let mut ids = Vec::new();
        for i in 0..5000 {
            let x = format!("@I{i}@");
            ids.push(xrefs.intern(&x).unwrap());
        }
        assert_eq!(store.xref_count(), 5000);
        for (i, id) in ids.iter().enumerate() {
            let x = format!("@I{i}@");
            assert_eq!(store.xref(*id), x);
            assert_eq!(store.find_xref(&x), Some(*id));
        }
        let made = store.intern_xref("@NEW@").unwrap();
        assert_eq!(store.xref(made), "@NEW@");
        assert_eq!(store.intern_xref("@I42@"), Some(ids[42]));
    }

    #[test]
    fn tags() {
        let mut store = Store::default();
        let name = TagId::standard("NAME").unwrap();
        assert_eq!(store.tag(name), "NAME");
        let ext = store.intern_tag("_X");
        assert_eq!(store.intern_tag("_X"), ext);
        assert_eq!(store.tag(ext), "_X");
    }

    #[test]
    fn sizes() {
        assert_eq!(std::mem::size_of::<Text>(), 16);
        assert_eq!(std::mem::size_of::<Option<Text>>(), 16);
        assert_eq!(std::mem::size_of::<Option<XrefId>>(), 4);
        assert_eq!(std::mem::size_of::<TextPiece>(), 8);
    }
}
