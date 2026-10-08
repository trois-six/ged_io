//! Latter-day Saint ordinances.

use super::citation::Citation;
use super::dates::Date;
use super::driver::{gedcom_struct, tag_enum};
use super::enums::LdsStatus;
use super::list::ThinVec;
use super::note::Note;
use super::place::Place;
use super::text::{Text, XrefId};

tag_enum! {
    /// Which ordinance an [`Ordinance`] is.
    pub enum OrdinanceKind {
        /// Baptism (`BAPL`).
        Baptism = "BAPL",
        /// Confirmation (`CONL`).
        Confirmation = "CONL",
        /// Endowment (`ENDL`).
        Endowment = "ENDL",
        /// Initiatory (7.x `INIL`).
        Initiatory = "INIL",
        /// Sealing of a child to their parents (`SLGC`).
        ChildSealing = "SLGC",
        /// Sealing of a couple (`SLGS`, of a family).
        SpouseSealing = "SLGS",
    }
}

gedcom_struct! {
    /// A Latter-day Saint ordinance (`BAPL`, `CONL`, `ENDL`, `INIL`, `SLGC`
    /// of an individual, `SLGS` of a family): when and where, in which
    /// temple, its status, the family of a child's sealing, notes and
    /// sources.
    pub struct Ordinance {
        @tag
        /// Which ordinance.
        kind: OrdinanceKind;
        /// When (`DATE`).
        "DATE" => date: Option<Date>,
        /// The temple (`TEMP`).
        "TEMP" => temple: Option<Text>,
        /// Where (`PLAC`).
        "PLAC" => place: Option<Place>,
        /// The status and when it was set (`STAT`).
        "STAT" => status: Option<Box<LdsStatus>>,
        /// The family a child is sealed to (`FAMC` of `SLGC`).
        "FAMC" => family: Option<XrefId>,
        /// Notes (`NOTE`, 7.x `SNOTE`).
        "NOTE" | "SNOTE" => notes: ThinVec<Note>,
        /// Sources (`SOUR`).
        "SOUR" => citations: ThinVec<Citation>,
    }
    spec {
        v551: [
            "LDS_INDIVIDUAL_ORDINANCE.BAPL",
            "LDS_INDIVIDUAL_ORDINANCE.CONL",
            "LDS_INDIVIDUAL_ORDINANCE.ENDL",
            "LDS_INDIVIDUAL_ORDINANCE.SLGC",
            "LDS_SPOUSE_SEALING.SLGS",
        ],
        v70: ["BAPL", "CONL", "ENDL", "INIL", "SLGC", "SLGS"],
        v71: ["BAPL", "CONL", "ENDL", "INIL", "SLGC", "SLGS"],
    }
}

impl Ordinance {
    /// An ordinance of this kind, with nothing else.
    #[must_use]
    pub fn new(kind: OrdinanceKind) -> Self {
        Self {
            kind,
            ..Self::default()
        }
    }
}
