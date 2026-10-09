//! Links between individuals and families: a family's partners and
//! children, an individual's families as a child and as a spouse, aliases
//! and associations.
//!
//! Every link is kept, in order, duplicates and `@VOID@` included: two
//! links to one family say two things.

use super::citation::Citation;
use super::driver::gedcom_struct;
use super::enums::{ChildStatus, Pedigree, Phrased, Role};
use super::list::ThinVec;
use super::note::Note;
use super::text::{Text, XrefId};

gedcom_struct! {
    /// A pointer to an individual with its phrase: a family's husband or
    /// wife (`HUSB`, `WIFE`) or child (`CHIL`), an individual's alias
    /// (`ALIA`). 7.x points to `@VOID@` and says who in the `PHRASE` when
    /// the individual has no record.
    pub struct IndividualRef {
        @payload
        /// The individual.
        individual: Option<XrefId>;
        @detail
        /// What few pointers to an individual have: a phrase.
        IndividualRefDetail {
            /// The individual in words (7.x `PHRASE`).
            "PHRASE" => phrase: Option<Text>,
        }
    }
    spec {
        v551: [
            "FAM_RECORD.FAM.CHIL",
            "FAM_RECORD.FAM.HUSB",
            "FAM_RECORD.FAM.WIFE",
            "INDIVIDUAL_RECORD.INDI.ALIA",
        ],
        v70: ["ALIA", "CHIL", "FAM-HUSB", "FAM-WIFE"],
        v71: ["ALIA", "CHIL", "FAM-HUSB", "FAM-WIFE"],
    }
}

impl IndividualRef {
    /// A pointer to an individual.
    #[must_use]
    pub fn new(individual: XrefId) -> Self {
        Self {
            individual: Some(individual),
            ..Self::default()
        }
    }
}

gedcom_struct! {
    /// The family an individual is a child of (`FAMC`): how (`PEDI`), how
    /// sure the link is (`STAT`), with notes.
    pub struct ChildLink {
        @payload
        /// The family (7.x: `@VOID@` when unknown).
        family: Option<XrefId>;
        @detail
        /// What fewer links have: a pedigree, a status and notes.
        ChildLinkDetail {
            /// How the child belongs to the family (`PEDI`), with a phrase
            /// in 7.x.
            "PEDI" => pedigree: Option<Phrased<Pedigree>>,
            /// How sure the link is (`STAT`), with a phrase in 7.x.
            "STAT" => status: Option<Phrased<ChildStatus>>,
            /// Notes (`NOTE`, 7.x `SNOTE`).
            "NOTE" | "SNOTE" => notes: Vec<Note>,
        }
    }
    spec {
        v551: ["CHILD_TO_FAMILY_LINK.FAMC"],
        v70: ["INDI-FAMC"],
        v71: ["INDI-FAMC"],
    }
}

impl ChildLink {
    /// A link to a family.
    #[must_use]
    pub fn new(family: XrefId) -> Self {
        Self {
            family: Some(family),
            ..Self::default()
        }
    }
}

gedcom_struct! {
    /// A family an individual is a spouse or partner in (`FAMS`), with
    /// notes.
    pub struct SpouseLink {
        @payload
        /// The family (7.x: `@VOID@` when unknown).
        family: Option<XrefId>;
        /// Notes (`NOTE`, 7.x `SNOTE`).
        "NOTE" | "SNOTE" => notes: ThinVec<Note>,
    }
    spec {
        v551: ["SPOUSE_TO_FAMILY_LINK.FAMS"],
        v70: ["FAMS"],
        v71: ["FAMS"],
    }
}

impl SpouseLink {
    /// A link to a family.
    #[must_use]
    pub fn new(family: XrefId) -> Self {
        Self {
            family: Some(family),
            ..Self::default()
        }
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
