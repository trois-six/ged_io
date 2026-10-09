//! Notes, texts and their translations.

use crate::tree::{FlatPayload, Payload};

use super::citation::Citation;
use super::driver::{gedcom_struct, NodeRef, PayloadField, ReadCx, WriteCx};
use super::enums::{NoteKind, Phrased};
use super::text::{Text, XrefId};

/// What a note holds: its own text, or a pointer to a shared note (5.5.1
/// `NOTE @N1@`, 7.x `SNOTE @N1@`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoteContent {
    /// The note's text, continuations joined.
    Text(Text),
    /// A pointer to a shared note record.
    Shared(XrefId),
}

impl Default for NoteContent {
    fn default() -> Self {
        NoteContent::Text(Text::default())
    }
}

impl PayloadField for NoteContent {
    fn read(node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> Option<Self> {
        match cx.pointer(node) {
            Some(id) => Some(NoteContent::Shared(id)),
            None => cx.text(node).map(NoteContent::Text),
        }
    }

    fn write(&self, cx: &WriteCx<'_>) -> Payload {
        match self {
            NoteContent::Text(t) => cx.text(t),
            NoteContent::Shared(id) => cx.pointer(*id),
        }
    }

    fn payload<'s>(&'s self, cx: &WriteCx<'s>) -> FlatPayload<'s> {
        match self {
            NoteContent::Text(t) => cx.text_flat(t),
            NoteContent::Shared(id) => cx.pointer_flat(*id),
        }
    }
}

impl super::relocate::Relocate for NoteContent {
    fn relocate(&mut self, r: &mut super::relocate::Relocation<'_>) {
        match self {
            NoteContent::Text(t) => t.relocate(r),
            NoteContent::Shared(id) => id.relocate(r),
        }
    }
}

#[cfg(feature = "serde")]
impl super::serde::SerializeIn for NoteContent {
    fn serialize_in<S: ::serde::Serializer>(
        &self,
        store: &super::Store,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match self {
            NoteContent::Text(v) => super::serde::serialize_tagged("Text", v, store, s),
            NoteContent::Shared(v) => super::serde::serialize_tagged("Shared", v, store, s),
        }
    }
}

#[cfg(feature = "serde")]
impl super::serde::DeserializeIn for NoteContent {
    fn deserialize_in<'de, D: ::serde::Deserializer<'de>>(
        store: &mut super::Store,
        d: D,
    ) -> Result<Self, D::Error> {
        super::serde::deserialize_either(
            store,
            d,
            &["Text", "Shared"],
            NoteContent::Text,
            NoteContent::Shared,
        )
    }
}

/// A shared note is written `SNOTE` in 7.x, `NOTE` in 5.5.1.
fn note_tag(note: &Note, tag: &'static str, cx: &WriteCx<'_>) -> &'static str {
    match note.content {
        NoteContent::Shared(_) if cx.version.is_v7() => "SNOTE",
        NoteContent::Shared(_) => "NOTE",
        NoteContent::Text(_) => tag,
    }
}

gedcom_struct! {
    /// A note (`NOTE`), or a pointer to a shared note (5.5.1 `NOTE @N1@`,
    /// 7.x `SNOTE @N1@`).
    ///
    /// 7.x gives a note text a media type (`MIME`), a language (`LANG`),
    /// translations (`TRAN`) and citations (`SOUR`); 7.1 says what kind of
    /// note it is (`KIND`).
    pub struct Note [tag = note_tag] {
        @payload
        /// The note's text, or the shared note it points to.
        content: NoteContent;
        @detail
        /// What few notes have besides their text: a media type, a
        /// language, translations, sources and kinds.
        NoteDetail {
            /// The media type of the text (`MIME`), `text/plain` when absent.
            "MIME" => mime: Option<Text>,
            /// The language of the text (`LANG`).
            "LANG" => language: Option<Text>,
            /// Translations of the text (`TRAN`).
            "TRAN" => translations: Vec<NoteTranslation>,
            /// Sources of the note (`SOUR`).
            "SOUR" => citations: Vec<Citation>,
            /// What kind of note this is (7.1 `KIND`).
            "KIND" => kinds: Vec<Phrased<NoteKind>>,
        }
    }
    spec {
        v551: ["NOTE_STRUCTURE.NOTE", "NOTE_STRUCTURE.NOTE#2"],
        v70: ["NOTE", "SNOTE"],
        v71: ["NOTE", "SNOTE"],
    }
}

impl Note {
    /// A note with this text.
    #[must_use]
    pub fn text(text: impl Into<Text>) -> Self {
        Self {
            content: NoteContent::Text(text.into()),
            ..Self::default()
        }
    }

    /// A pointer to a shared note.
    #[must_use]
    pub fn shared(xref: XrefId) -> Self {
        Self {
            content: NoteContent::Shared(xref),
            ..Self::default()
        }
    }
}

gedcom_struct! {
    /// A translation of a note (7.x `NOTE.TRAN`): the text in another
    /// language or media type.
    pub struct NoteTranslation {
        @payload
        /// The translated text.
        text: Text;
        /// Its media type (`MIME`).
        "MIME" => mime: Option<Text>,
        /// Its language (`LANG`).
        "LANG" => language: Option<Text>,
    }
    spec {
        v551: [],
        v70: ["NOTE-TRAN"],
        v71: ["NOTE-TRAN"],
    }
}

gedcom_struct! {
    /// Text from a source (`TEXT`), with its media type and language (7.x).
    pub struct SourceText {
        @payload
        /// The text.
        text: Text;
        /// Its media type (`MIME`).
        "MIME" => mime: Option<Text>,
        /// Its language (`LANG`).
        "LANG" => language: Option<Text>,
    }
    spec {
        v551: [
            "SOURCE_CITATION.SOUR#2.TEXT",
            "SOURCE_CITATION.SOUR.DATA.TEXT",
            "SOURCE_RECORD.SOUR.TEXT",
        ],
        v70: ["TEXT"],
        v71: ["TEXT"],
    }
}

gedcom_struct! {
    /// A translation of a header's title or description (7.1 `TRAN` of
    /// `HEAD.TITL` or `HEAD.DESC`).
    pub struct TextTranslation {
        @payload
        /// The translated text.
        text: Text;
        /// Its language (`LANG`).
        "LANG" => language: Option<Text>,
    }
    spec {
        v551: [],
        v70: [],
        v71: ["TEXT-TRAN"],
    }
}
