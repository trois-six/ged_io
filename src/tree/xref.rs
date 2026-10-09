//! Cross-reference identifiers and pointers.

use std::borrow::Borrow;
use std::fmt;
use std::ops::Deref;

/// A cross-reference identifier (`@I1@`), or a pointer to one.
///
/// It is stored with its `@` delimiters, as it appears in the file.
///
/// GEDCOM 5.5.1 (p. 16) also defines pointers to a substructure
/// (`@I132!1@`), intra-record pointers (`@!1@`) and network references that
/// contain a `:`; [`Xref::form`] tells them apart. They are kept verbatim.
///
/// ```rust
/// use ged_io::tree::{Xref, XrefForm};
///
/// let xref = Xref::new("@I1@");
/// assert_eq!(xref.id(), "I1");
/// assert_eq!(xref, "@I1@");
/// assert_eq!(Xref::new("@I132!1@").form(), XrefForm::Substructure { record: "I132", substructure: "1" });
/// assert!(Xref::new(Xref::VOID).is_void());
/// ```
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(transparent)
)]
pub struct Xref(Box<str>);

/// The shape of a cross-reference identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XrefForm<'a> {
    /// A record identifier such as `@I1@` (the only form GEDCOM 7.0 has).
    Record,
    /// The GEDCOM 7.0 null pointer `@VOID@`.
    Void,
    /// A 5.5.1 pointer to a substructure of a record, `@I132!1@`.
    Substructure {
        /// The record part, `I132`.
        record: &'a str,
        /// The substructure part, `1`.
        substructure: &'a str,
    },
    /// A 5.5.1 pointer to a substructure of the same record, `@!1@`.
    IntraRecord {
        /// The substructure part, `1`.
        substructure: &'a str,
    },
    /// A 5.5.1 network reference: the identifier contains a `:`.
    Network,
}

impl Xref {
    /// The GEDCOM 7.0 null pointer.
    pub const VOID: &'static str = "@VOID@";

    /// Makes an identifier from its text, delimiters included.
    #[must_use]
    pub fn new(xref: impl Into<Box<str>>) -> Self {
        Self(xref.into())
    }

    /// The identifier with its delimiters, `@I1@`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The identifier without its delimiters, `I1`.
    #[must_use]
    pub fn id(&self) -> &str {
        let s = self.as_str();
        let s = s.strip_prefix('@').unwrap_or(s);
        s.strip_suffix('@').unwrap_or(s)
    }

    /// Whether this is the GEDCOM 7.0 null pointer `@VOID@`.
    #[must_use]
    pub fn is_void(&self) -> bool {
        self.as_str() == Self::VOID
    }

    /// The shape of the identifier.
    #[must_use]
    pub fn form(&self) -> XrefForm<'_> {
        let id = self.id();
        if self.is_void() {
            XrefForm::Void
        } else if id.contains(':') {
            XrefForm::Network
        } else if let Some(substructure) = id.strip_prefix('!') {
            XrefForm::IntraRecord { substructure }
        } else if let Some((record, substructure)) = id.split_once('!') {
            XrefForm::Substructure {
                record,
                substructure,
            }
        } else {
            XrefForm::Record
        }
    }
}

impl fmt::Debug for Xref {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for Xref {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Deref for Xref {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Xref {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for Xref {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl From<&str> for Xref {
    fn from(xref: &str) -> Self {
        Self::new(xref)
    }
}

impl From<String> for Xref {
    fn from(xref: String) -> Self {
        Self::new(xref)
    }
}

impl From<Xref> for String {
    fn from(xref: Xref) -> Self {
        xref.0.into_string()
    }
}

impl PartialEq<str> for Xref {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Xref {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<String> for Xref {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms() {
        assert_eq!(Xref::new("@I1@").form(), XrefForm::Record);
        assert_eq!(Xref::new("@VOID@").form(), XrefForm::Void);
        assert_eq!(
            Xref::new("@!2@").form(),
            XrefForm::IntraRecord { substructure: "2" }
        );
        assert_eq!(Xref::new("@NET:I1@").form(), XrefForm::Network);
        assert_eq!(Xref::new("@NoTe ref@").id(), "NoTe ref");
    }
}
