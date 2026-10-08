//! One view over the two forms of a tree — the arena of a parsed
//! [`Tree`](super::Tree) and owned [`Structure`]s — so that the writer and
//! the validator walk both with one code path.

use super::{PayloadRef, Structure, StructureRef, Substructures};

/// A structure, borrowed for `'n`.
pub(crate) trait Node<'n>: Copy {
    /// The substructures.
    type Children: Iterator<Item = Self>;
    /// The tag.
    fn tag(self) -> &'n str;
    /// The identifier.
    fn xref(self) -> Option<&'n str>;
    /// The payload.
    fn payload(self) -> PayloadRef<'n>;
    /// The 1-based source line; 0 when not read from a file.
    fn line(self) -> u32;
    /// The substructures.
    fn children(self) -> Self::Children;
    /// An owned copy.
    fn to_owned_structure(self) -> Structure;
}

impl<'t> Node<'t> for StructureRef<'t> {
    type Children = Substructures<'t>;
    fn tag(self) -> &'t str {
        StructureRef::tag(self)
    }
    fn xref(self) -> Option<&'t str> {
        StructureRef::xref(self)
    }
    fn payload(self) -> PayloadRef<'t> {
        StructureRef::payload(self)
    }
    fn line(self) -> u32 {
        StructureRef::line(self)
    }
    fn children(self) -> Self::Children {
        self.substructures()
    }
    fn to_owned_structure(self) -> Structure {
        self.to_structure()
    }
}

impl<'n> Node<'n> for &'n Structure {
    type Children = std::slice::Iter<'n, Structure>;
    fn tag(self) -> &'n str {
        self.tag.as_str()
    }
    fn xref(self) -> Option<&'n str> {
        self.xref.as_deref()
    }
    fn payload(self) -> PayloadRef<'n> {
        self.payload.borrowed()
    }
    fn line(self) -> u32 {
        self.line
    }
    fn children(self) -> Self::Children {
        self.substructures.iter()
    }
    fn to_owned_structure(self) -> Structure {
        self.clone()
    }
}
