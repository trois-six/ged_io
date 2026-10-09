//! Personal names, their pieces, translations and variations.

use crate::tree::{Flat, Structure};

use super::citation::Citation;
use super::driver::{
    gedcom_struct, leaf_structure, tag_enum, FromNode, NodeRef, ReadCx, StdTag, TagField, ToNodes,
    WriteCx,
};
use super::enums::{NameType, PhoneticType, Phrased, RomanizedType};
use super::list::ThinVec;
use super::note::Note;
use super::text::{Store, Text};

tag_enum! {
    /// Which part of a name a [`NamePiece`] is.
    pub enum NamePieceKind {
        /// A prefix (`NPFX`: `Dr.`).
        Prefix = "NPFX",
        /// A given name (`GIVN`).
        Given = "GIVN",
        /// A nickname (`NICK`).
        Nickname = "NICK",
        /// A surname prefix (`SPFX`: `van`).
        SurnamePrefix = "SPFX",
        /// A surname (`SURN`).
        Surname = "SURN",
        /// A suffix (`NSFX`: `Jr.`).
        Suffix = "NSFX",
    }
}

/// A piece of a name (`GIVN`, `SURN`, …) as written: a name keeps the
/// pieces it was given, in order, and gains none (a surname between
/// slashes in the name itself is not a `SURN`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NamePiece {
    /// Which part of the name.
    pub kind: NamePieceKind,
    /// The piece. 5.5.1 writes several of a kind in one, comma-separated.
    pub value: Text,
}

impl FromNode for NamePiece {
    const LEAF: bool = true;

    fn from_node(node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> Option<Self> {
        if node.has_children() || node.has_xref() {
            return None;
        }
        Some(Self {
            kind: NamePieceKind::from_tag(node.standard_tag()?)?,
            value: cx.text(node)?,
        })
    }
}

impl ToNodes for NamePiece {
    fn to_node(&self, _tag: &'static str, cx: &WriteCx<'_>) -> Structure {
        leaf_structure(self.kind.tag(), cx.text(&self.value))
    }

    fn to_flat<'s>(&'s self, _tag: StdTag, cx: &WriteCx<'s>, out: &mut Flat<'s>) {
        let tag = self.kind.std_tag();
        out.leaf_typed(tag.name, tag.index, cx.text_flat(&self.value), None);
    }
}

impl super::relocate::Relocate for NamePiece {
    fn relocate(&mut self, r: &mut super::relocate::Relocation<'_>) {
        self.value.relocate(r);
    }
}

/// The pieces of `kind` among `pieces`.
fn pieces_of(pieces: &[NamePiece], kind: NamePieceKind) -> impl Iterator<Item = &Text> {
    pieces
        .iter()
        .filter(move |p| p.kind == kind)
        .map(|p| &p.value)
}

gedcom_struct! {
    /// A personal name (`NAME`): the name as written, the surname between
    /// slashes (`John /Smith/`), its pieces, and what kind of name it is.
    ///
    /// ```rust
    /// use ged_io::model::{Dataset, NameType};
    ///
    /// let data = Dataset::parse("0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 NAME Ann /Example/\n2 TYPE BIRTH\n2 GIVN Ann\n2 GIVN Marie\n0 TRLR\n");
    /// let name = &data.individuals[0].names[0];
    /// assert_eq!(name.surname_in_value(&data).as_deref(), Some("Example"));
    /// assert_eq!(name.givens().count(), 2);
    /// assert_eq!(name.surnames().count(), 0); // none is invented
    /// assert_eq!(name.detail().kind.as_ref().unwrap().value, NameType::Birth);
    /// ```
    pub struct Name {
        @payload
        /// The name as written, the surname between slashes.
        value: Text;
        /// The pieces (`NPFX`, `GIVN`, `NICK`, `SPFX`, `SURN`, `NSFX`), in
        /// order.
        "NPFX" | "GIVN" | "NICK" | "SPFX" | "SURN" | "NSFX" => pieces: ThinVec<NamePiece>,
        @detail
        /// What fewer names have: a kind, translations, variations, notes
        /// and sources.
        NameDetail {
            /// What kind of name it is (`TYPE`), with a phrase in 7.x.
            "TYPE" => kind: Option<Phrased<NameType>>,
            /// The name in other languages or scripts (7.x `TRAN`).
            "TRAN" => translations: Vec<NameTranslation>,
            /// Phonetic variations (5.5.1 `FONE`).
            "FONE" => phonetic: Vec<PhoneticName>,
            /// Romanized variations (5.5.1 `ROMN`).
            "ROMN" => romanized: Vec<RomanizedName>,
            /// Notes (`NOTE`, 7.x `SNOTE`).
            "NOTE" | "SNOTE" => notes: Vec<Note>,
            /// Sources (`SOUR`).
            "SOUR" => citations: Vec<Citation>,
        }
    }
    spec {
        v551: ["PERSONAL_NAME_STRUCTURE.NAME"],
        v70: ["INDI-NAME"],
        v71: ["INDI-NAME"],
    }
}

impl Name {
    /// A name with this value (`John /Smith/`).
    #[must_use]
    pub fn new(value: impl Into<Text>) -> Self {
        Self {
            value: value.into(),
            ..Self::default()
        }
    }

    /// The pieces of a kind, in order.
    pub fn pieces_of(&self, kind: NamePieceKind) -> impl Iterator<Item = &Text> {
        pieces_of(&self.pieces, kind)
    }

    /// The given names (`GIVN`).
    pub fn givens(&self) -> impl Iterator<Item = &Text> {
        self.pieces_of(NamePieceKind::Given)
    }

    /// The surnames (`SURN`).
    pub fn surnames(&self) -> impl Iterator<Item = &Text> {
        self.pieces_of(NamePieceKind::Surname)
    }

    /// The first given name piece (`GIVN`).
    #[must_use]
    pub fn given(&self) -> Option<&Text> {
        self.givens().next()
    }

    /// The first surname piece (`SURN`).
    #[must_use]
    pub fn surname(&self) -> Option<&Text> {
        self.surnames().next()
    }

    /// The name as written, without the slashes around its surname and
    /// with its spaces collapsed: `Ann Example` for `Ann /Example/`.
    #[must_use]
    pub fn full<S: AsRef<Store> + ?Sized>(&self, store: &S) -> String {
        let value = self.value.to_str(store);
        value
            .split(|c: char| c == '/' || c.is_whitespace())
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The surname written between slashes in the name itself, if any:
    /// `Smith` for `John /Smith/`.
    #[must_use]
    pub fn surname_in_value<S: AsRef<Store> + ?Sized>(&self, store: &S) -> Option<String> {
        let value = self.value.to_str(store);
        let (_, rest) = value.split_once('/')?;
        let surname = rest.split_once('/').map_or(rest, |(s, _)| s);
        Some(surname.trim().to_string())
    }
}

gedcom_struct! {
    /// A name in another language or script (7.x `TRAN` of `NAME`): its
    /// language and its pieces.
    pub struct NameTranslation {
        @payload
        /// The name as written.
        value: Text;
        /// The language (`LANG`).
        "LANG" => language: Option<Text>,
        /// The pieces, in order.
        "NPFX" | "GIVN" | "NICK" | "SPFX" | "SURN" | "NSFX" => pieces: ThinVec<NamePiece>,
    }
    spec {
        v551: [],
        v70: ["NAME-TRAN"],
        v71: ["NAME-TRAN"],
    }
}

gedcom_struct! {
    /// A phonetic variation of a name (5.5.1 `FONE` of `NAME`): its method
    /// (`TYPE`), its pieces, notes and sources.
    pub struct PhoneticName {
        @payload
        /// The variation.
        value: Text;
        /// The method (`TYPE`).
        "TYPE" => kind: Option<PhoneticType>,
        /// The pieces, in order.
        "NPFX" | "GIVN" | "NICK" | "SPFX" | "SURN" | "NSFX" => pieces: ThinVec<NamePiece>,
        /// Notes (`NOTE`).
        "NOTE" | "SNOTE" => notes: ThinVec<Note>,
        /// Sources (`SOUR`).
        "SOUR" => citations: ThinVec<Citation>,
    }
    spec {
        v551: ["PERSONAL_NAME_STRUCTURE.NAME.FONE"],
        v70: [],
        v71: [],
    }
}

gedcom_struct! {
    /// A romanized variation of a name (5.5.1 `ROMN` of `NAME`): its method
    /// (`TYPE`), its pieces, notes and sources.
    pub struct RomanizedName {
        @payload
        /// The variation.
        value: Text;
        /// The method (`TYPE`).
        "TYPE" => kind: Option<RomanizedType>,
        /// The pieces, in order.
        "NPFX" | "GIVN" | "NICK" | "SPFX" | "SURN" | "NSFX" => pieces: ThinVec<NamePiece>,
        /// Notes (`NOTE`).
        "NOTE" | "SNOTE" => notes: ThinVec<Note>,
        /// Sources (`SOUR`).
        "SOUR" => citations: ThinVec<Citation>,
    }
    spec {
        v551: ["PERSONAL_NAME_STRUCTURE.NAME.ROMN"],
        v70: [],
        v71: [],
    }
}
