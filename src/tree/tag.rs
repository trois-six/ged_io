//! Tags: the standard ones interned as a small index, the others as text.

use std::borrow::Borrow;
use std::cmp::Ordering;
use std::fmt;

/// Every tag defined by GEDCOM 5.5.1 (Appendix A, including the 5.5 tags it
/// still lists) or GEDCOM 7.0 (`FamilySearch` `extracted-files/substructures.tsv`),
/// sorted. A tag's index in this table is its identifier.
pub(crate) static STANDARD_TAGS: [&str; 156] = [
    "ABBR", "ADDR", "ADOP", "ADR1", "ADR2", "ADR3", "AFN", "AGE", "AGNC", "ALIA", "ANCE", "ANCI",
    "ANUL", "ASSO", "AUTH", "BAPL", "BAPM", "BARM", "BASM", "BIRT", "BLES", "BLOB", "BURI", "CALN",
    "CAST", "CAUS", "CENS", "CHAN", "CHAR", "CHIL", "CHR", "CHRA", "CITY", "CONC", "CONF", "CONL",
    "CONT", "COPR", "CORP", "CREA", "CREM", "CROP", "CTRY", "DATA", "DATE", "DEAT", "DESC", "DESI",
    "DEST", "DIV", "DIVF", "DSCR", "EDUC", "EMAI", "EMAIL", "EMIG", "ENDL", "ENGA", "EVEN", "EXID",
    "FACT", "FAM", "FAMC", "FAMF", "FAMS", "FAX", "FCOM", "FILE", "FONE", "FORM", "GEDC", "GIVN",
    "GRAD", "HEAD", "HEIGHT", "HUSB", "IDNO", "IMMI", "INDI", "INIL", "LANG", "LATI", "LEFT",
    "LONG", "MAP", "MARB", "MARC", "MARL", "MARR", "MARS", "MEDI", "MIME", "NAME", "NATI", "NATU",
    "NCHI", "NICK", "NMR", "NO", "NOTE", "NPFX", "NSFX", "OBJE", "OCCU", "ORDI", "ORDN", "PAGE",
    "PEDI", "PHON", "PHRASE", "PLAC", "POST", "PROB", "PROP", "PUBL", "QUAY", "REFN", "RELA",
    "RELI", "REPO", "RESI", "RESN", "RETI", "RFN", "RIN", "ROLE", "ROMN", "SCHMA", "SDATE", "SEX",
    "SLGC", "SLGS", "SNOTE", "SOUR", "SPFX", "SSN", "STAE", "STAT", "SUBM", "SUBN", "SURN", "TAG",
    "TEMP", "TEXT", "TIME", "TITL", "TOP", "TRAN", "TRLR", "TYPE", "UID", "VERS", "WIDTH", "WIFE",
    "WILL", "WWW",
];

/// A tag of at most seven bytes as an integer: its bytes, big-endian and
/// zero-padded, then its length in the low byte. The integers sort as the
/// tags do, and a NUL byte cannot make two tags equal.
const fn key(tag: &[u8]) -> Option<u64> {
    if tag.len() > 7 {
        return None;
    }
    let mut key = 0_u64;
    let mut i = 0;
    while i < 7 {
        key <<= 8;
        if i < tag.len() {
            key |= tag[i] as u64;
        }
        i += 1;
    }
    Some(key << 8 | tag.len() as u64)
}

/// [`STANDARD_TAGS`] as keys, for a lookup without string comparisons.
static STANDARD_KEYS: [u64; STANDARD_TAGS.len()] = {
    let mut keys = [0; STANDARD_TAGS.len()];
    let mut i = 0;
    while i < keys.len() {
        keys[i] = match key(STANDARD_TAGS[i].as_bytes()) {
            Some(k) => k,
            None => panic!("standard tags are at most seven bytes"),
        };
        i += 1;
    }
    keys
};

/// Slots of the open-addressing table of standard tags (a power of two,
/// about three times the number of tags).
const SLOTS: usize = 512;

const fn slot(key: u64) -> usize {
    (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 55) as usize
}

/// `(key, index + 1)` of each standard tag, by hash with linear probing; a
/// zero key is an empty slot (a real key has its length in the low byte).
// `as` is the only conversion a constant can use; the index is below 156.
#[allow(clippy::cast_possible_truncation)]
static STANDARD_SLOTS: [(u64, u16); SLOTS] = {
    let mut table = [(0_u64, 0_u16); SLOTS];
    let mut i = 0;
    while i < STANDARD_KEYS.len() {
        let mut s = slot(STANDARD_KEYS[i]);
        while table[s].0 != 0 {
            s = (s + 1) % SLOTS;
        }
        table[s] = (STANDARD_KEYS[i], i as u16 + 1);
        i += 1;
    }
    table
};

/// The index of a standard tag.
#[inline]
pub(crate) fn standard_index(tag: &str) -> Option<u16> {
    let key = key(tag.as_bytes())?;
    let mut s = slot(key);
    loop {
        let &(k, index) = STANDARD_SLOTS.get(s)?;
        if k == key {
            return index.checked_sub(1);
        }
        if k == 0 {
            return None;
        }
        s = (s + 1) % SLOTS;
    }
}

/// A GEDCOM tag.
///
/// Standard tags (those of GEDCOM 5.5.1 or 7.0) are stored as a two-byte
/// index into a static table; any other tag, such as an extension tag (`_FOO`)
/// or a misspelt one, keeps its text. Tags are case-sensitive, as both
/// specifications define them: `Name` is not `NAME`.
///
/// ```rust
/// use ged_io::tree::Tag;
///
/// let name = Tag::new("NAME");
/// assert!(name.is_standard());
/// assert_eq!(name, "NAME");
/// assert!(Tag::new("_MILT").is_extension());
/// ```
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Tag(Repr);

#[derive(Clone, PartialEq, Eq, Hash)]
enum Repr {
    Standard(u16),
    Other(Box<str>),
}

impl Tag {
    /// Makes a tag from its text.
    #[must_use]
    pub fn new(tag: &str) -> Self {
        match standard_index(tag) {
            Some(i) => Self(Repr::Standard(i)),
            None => Self(Repr::Other(tag.into())),
        }
    }

    /// A standard tag from its table index.
    pub(crate) fn standard(index: u16) -> Self {
        Self(Repr::Standard(index))
    }

    /// The tag's text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match &self.0 {
            Repr::Standard(i) => STANDARD_TAGS.get(usize::from(*i)).copied().unwrap_or(""),
            Repr::Other(s) => s,
        }
    }

    /// Whether GEDCOM 5.5.1 or 7.0 defines this tag. It may still be
    /// misplaced: that is a question for the structure that holds it.
    #[must_use]
    pub fn is_standard(&self) -> bool {
        matches!(self.0, Repr::Standard(_))
    }

    /// Whether this is an extension tag: one that starts with `_`.
    #[must_use]
    pub fn is_extension(&self) -> bool {
        self.as_str().starts_with('_')
    }
}

impl Default for Tag {
    /// The empty tag, which the reader gives to a line that has no level.
    fn default() -> Self {
        Self(Repr::Other(Box::default()))
    }
}

impl fmt::Debug for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for Tag {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for Tag {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl From<&str> for Tag {
    fn from(tag: &str) -> Self {
        Self::new(tag)
    }
}

impl PartialEq<str> for Tag {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Tag {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialOrd for Tag {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Tag {
    /// Tags sort by their text.
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

#[cfg(feature = "json")]
impl serde::Serialize for Tag {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "json")]
impl<'de> serde::Deserialize<'de> for Tag {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str> as serde::Deserialize>::deserialize(deserializer)?;
        Ok(Self::new(&s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_and_unique() {
        assert!(STANDARD_TAGS.windows(2).all(|w| w[0] < w[1]));
        assert!(STANDARD_KEYS.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(standard_index("NAME\0"), None);
        assert_eq!(standard_index("NAMES"), None);
        assert_eq!(standard_index("A_VERY_LONG_TAG"), None);
    }

    #[test]
    fn standard_and_other_tags() {
        for tag in STANDARD_TAGS {
            let t = Tag::new(tag);
            assert!(t.is_standard());
            assert_eq!(t.as_str(), tag);
        }
        let ext = Tag::new("_FOO");
        assert!(!ext.is_standard() && ext.is_extension());
        assert_ne!(Tag::new("name"), Tag::new("NAME"));
        assert!(Tag::new("ABBR") < Tag::new("_A"));
        assert_eq!(Tag::default().as_str(), "");
    }
}
