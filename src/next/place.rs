//! Places and associations.

use super::citation::Citation;
use super::driver::gedcom_struct;
use super::enums::{PhoneticType, Phrased, Role, RomanizedType};
use super::identifiers::Exid;
use super::list::ThinVec;
use super::note::Note;
use super::text::{Text, XrefId};

gedcom_struct! {
    /// A place (`PLAC`): its name, as comma-separated jurisdictions, with
    /// the form of those jurisdictions (`FORM`), its language and
    /// translations (7.x `LANG`, `TRAN`), its coordinates (`MAP`),
    /// identifiers (7.x `EXID`), notes, and 5.5.1 phonetic and romanized
    /// variations (`FONE`, `ROMN`).
    pub struct Place {
        @payload
        /// The name, jurisdictions from the smallest.
        name: Text;
        @detail
        /// What fewer places have besides their name: the form of the
        /// jurisdictions, a language, translations, coordinates,
        /// identifiers, notes and variations.
        PlaceDetail {
            /// The jurisdictions named, in order (`FORM`).
            "FORM" => form: Option<Text>,
            /// The language of the name (`LANG`).
            "LANG" => language: Option<Text>,
            /// The name in other languages (`TRAN`).
            "TRAN" => translations: Vec<PlaceTranslation>,
            /// The coordinates (`MAP`).
            "MAP" => map: Option<Map>,
            /// Identifiers in external systems (`EXID`).
            "EXID" => exids: Vec<Exid>,
            /// Notes (`NOTE`, 7.x `SNOTE`).
            "NOTE" | "SNOTE" => notes: Vec<Note>,
            /// Phonetic variations of the name (5.5.1 `FONE`).
            "FONE" => phonetic: Vec<PhoneticVariation>,
            /// Romanized variations of the name (5.5.1 `ROMN`).
            "ROMN" => romanized: Vec<RomanizedVariation>,
        }
    }
    spec {
        v551: ["PLACE_STRUCTURE.PLAC"],
        v70: ["PLAC"],
        v71: ["PLAC"],
    }
}

gedcom_struct! {
    /// A place name in another language (7.x `PLAC.TRAN`).
    pub struct PlaceTranslation {
        @payload
        /// The translated name.
        name: Text;
        /// Its language (`LANG`).
        "LANG" => language: Option<Text>,
    }
    spec {
        v551: [],
        v70: ["PLAC-TRAN"],
        v71: ["PLAC-TRAN"],
    }
}

gedcom_struct! {
    /// The coordinates of a place (`MAP`), as written (`N18.150944`,
    /// `E168.150944`).
    pub struct Map {
        /// Latitude (`LATI`).
        "LATI" => latitude: Option<Text>,
        /// Longitude (`LONG`).
        "LONG" => longitude: Option<Text>,
    }
    spec {
        v551: ["PLACE_STRUCTURE.PLAC.MAP"],
        v70: ["MAP"],
        v71: ["MAP"],
    }
}

gedcom_struct! {
    /// A phonetic variation of a place name (5.5.1 `PLAC.FONE`) and its
    /// method (`TYPE`).
    pub struct PhoneticVariation {
        @payload
        /// The variation.
        name: Text;
        /// The method (`TYPE`).
        "TYPE" => kind: Option<PhoneticType>,
    }
    spec {
        v551: ["PLACE_STRUCTURE.PLAC.FONE"],
        v70: [],
        v71: [],
    }
}

gedcom_struct! {
    /// A romanized variation of a place name (5.5.1 `PLAC.ROMN`) and its
    /// method (`TYPE`).
    pub struct RomanizedVariation {
        @payload
        /// The variation.
        name: Text;
        /// The method (`TYPE`).
        "TYPE" => kind: Option<RomanizedType>,
    }
    spec {
        v551: ["PLACE_STRUCTURE.PLAC.ROMN"],
        v70: [],
        v71: [],
    }
}

gedcom_struct! {
    /// An association with another individual (`ASSO`): who, how they are
    /// related (7.x `ROLE`, 5.5.1 `RELA`), with notes and sources.
    pub struct Association {
        @payload
        /// The individual associated (7.x: `@VOID@` when not recorded).
        individual: Option<XrefId>;
        /// The individual in words (7.x `PHRASE`).
        "PHRASE" => phrase: Option<Text>,
        /// The role of the individual associated (7.x `ROLE`).
        "ROLE" => role: Option<Phrased<Role>>,
        /// The relation, in words (5.5.1 `RELA`).
        "RELA" => relation: Option<Text>,
        /// Notes (`NOTE`, 7.x `SNOTE`).
        "NOTE" | "SNOTE" => notes: ThinVec<Note>,
        /// Sources (`SOUR`).
        "SOUR" => citations: ThinVec<Citation>,
    }
    spec {
        v551: ["ASSOCIATION_STRUCTURE.ASSO"],
        v70: ["ASSO"],
        v71: ["ASSO"],
    }
}
