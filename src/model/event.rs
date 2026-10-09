//! Events and attributes of individuals and families, and the events an
//! individual is known not to have had.

use super::address::Address;
use super::citation::Citation;
use super::date::{Age, Date, Period};
use super::driver::gedcom_struct;
use super::enums::{Adoption, BirthKind, EnumList, EventKind, Phrased, Restriction};
use super::link::Association;
use super::list::ThinVec;
use super::multimedia::MultimediaLink;
use super::note::Note;
use super::place::Place;
use super::text::{Text, XrefId};

gedcom_struct! {
    /// An event or an attribute of an individual or a family: a birth
    /// (`BIRT`), a marriage (`MARR`), an occupation (`OCCU`), a generic
    /// event or fact (`EVEN`, `FACT`) with its `TYPE`, …
    ///
    /// Its tag is its [`kind`](Self::kind); every event and attribute has
    /// the same detail (`EVENT_DETAIL` in 5.5.1, `EVENT_DETAIL` with
    /// `INDIVIDUAL_EVENT_DETAIL` or `FAMILY_EVENT_DETAIL` in 7.x). The date,
    /// place and sources, which most events have, are fields of the event;
    /// the rest is in its [`detail`](Self::detail).
    ///
    /// ```rust
    /// use ged_io::model::{Dataset, EventKind};
    ///
    /// let data = Dataset::parse("0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 BIRT\n2 DATE 1 JAN 1900\n2 AGE 0y\n1 OCCU Miller\n0 TRLR\n");
    /// let indi = &data.individuals[0];
    /// let birth = &indi.events[0];
    /// assert_eq!(birth.kind, EventKind::Birth);
    /// assert_eq!(birth.date.as_ref().unwrap().value.to_str(&data), "1 JAN 1900");
    /// assert_eq!(birth.detail().age.as_ref().unwrap().value.to_str(&data), "0y");
    /// assert_eq!(indi.events[1].value.to_str(&data), "Miller");
    /// ```
    pub struct Event {
        @tag
        /// What happened: the event's or attribute's tag.
        kind: EventKind;
        @payload
        /// The payload: an attribute's value (`OCCU Miller`), the number of
        /// `NCHI`, `Y` for an event known to have happened without a date
        /// or a place, or, in 5.5.1, words about the event.
        value: Text;
        /// When (`DATE`).
        "DATE" => date: Option<Date>,
        /// Where (`PLAC`).
        "PLAC" => place: Option<Place>,
        /// Sources (`SOUR`).
        "SOUR" => citations: ThinVec<Citation>,
        @detail
        /// What fewer events have: a type, ages, an address and contacts,
        /// the responsible agency, religion and cause, a restriction, a
        /// sort date, associations, notes, media, identifiers, the family
        /// of a birth or adoption, the kind of a birth.
        EventDetail {
            /// What kind of event or fact it is, in words (`TYPE`); `EVEN`
            /// and `FACT` require it.
            "TYPE" => classification: Option<Text>,
            /// The individual's age (`AGE`).
            "AGE" => age: Option<Age>,
            /// The husband's age at a family event (`HUSB`).
            "HUSB" => husband: Option<EventSpouse>,
            /// The wife's age at a family event (`WIFE`).
            "WIFE" => wife: Option<EventSpouse>,
            /// The family a child was born into or adopted by (`FAMC` of
            /// `BIRT`, `CHR` or `ADOP`).
            "FAMC" => family: Option<EventFamily>,
            /// Where, as an address (`ADDR`).
            "ADDR" => address: Option<Address>,
            /// Telephone numbers (`PHON`).
            "PHON" => phones: Vec<Text>,
            /// E-mail addresses (`EMAIL`).
            "EMAIL" => emails: Vec<Text>,
            /// Fax numbers (`FAX`).
            "FAX" => faxes: Vec<Text>,
            /// Web pages (`WWW`).
            "WWW" => websites: Vec<Text>,
            /// The organisation responsible (`AGNC`).
            "AGNC" => agency: Option<Text>,
            /// The religion (`RELI`).
            "RELI" => religion: Option<Text>,
            /// The cause (`CAUS`).
            "CAUS" => cause: Option<Text>,
            /// Restrictions on the event (`RESN`).
            "RESN" => restriction: Option<EnumList<Restriction>>,
            /// A date to sort by (7.x `SDATE`).
            "SDATE" => sort_date: Option<Date>,
            /// Individuals associated with the event (`ASSO`; 7.x).
            "ASSO" => associations: Vec<Association>,
            /// Notes (`NOTE`, 7.x `SNOTE`).
            "NOTE" | "SNOTE" => notes: Vec<Note>,
            /// Media (`OBJE`).
            "OBJE" => multimedia: Vec<MultimediaLink>,
            /// Unique identifiers (7.x `UID`).
            "UID" => uids: Vec<Text>,
            /// Whether a child was born alive (7.1 `KIND` of `BIRT`).
            "KIND" => birth_kinds: Vec<BirthKind>,
        }
    }
    spec {
        v551: [
            "FAMILY_EVENT_STRUCTURE.ANUL",
            "FAMILY_EVENT_STRUCTURE.CENS",
            "FAMILY_EVENT_STRUCTURE.DIV",
            "FAMILY_EVENT_STRUCTURE.DIVF",
            "FAMILY_EVENT_STRUCTURE.ENGA",
            "FAMILY_EVENT_STRUCTURE.EVEN",
            "FAMILY_EVENT_STRUCTURE.MARB",
            "FAMILY_EVENT_STRUCTURE.MARC",
            "FAMILY_EVENT_STRUCTURE.MARL",
            "FAMILY_EVENT_STRUCTURE.MARR",
            "FAMILY_EVENT_STRUCTURE.MARS",
            "FAMILY_EVENT_STRUCTURE.RESI",
            "FAM_RECORD.FAM.NCHI",
            "INDIVIDUAL_ATTRIBUTE_STRUCTURE.CAST",
            "INDIVIDUAL_ATTRIBUTE_STRUCTURE.DSCR",
            "INDIVIDUAL_ATTRIBUTE_STRUCTURE.EDUC",
            "INDIVIDUAL_ATTRIBUTE_STRUCTURE.FACT",
            "INDIVIDUAL_ATTRIBUTE_STRUCTURE.IDNO",
            "INDIVIDUAL_ATTRIBUTE_STRUCTURE.NATI",
            "INDIVIDUAL_ATTRIBUTE_STRUCTURE.NCHI",
            "INDIVIDUAL_ATTRIBUTE_STRUCTURE.NMR",
            "INDIVIDUAL_ATTRIBUTE_STRUCTURE.OCCU",
            "INDIVIDUAL_ATTRIBUTE_STRUCTURE.PROP",
            "INDIVIDUAL_ATTRIBUTE_STRUCTURE.RELI",
            "INDIVIDUAL_ATTRIBUTE_STRUCTURE.RESI",
            "INDIVIDUAL_ATTRIBUTE_STRUCTURE.SSN",
            "INDIVIDUAL_ATTRIBUTE_STRUCTURE.TITL",
            "INDIVIDUAL_EVENT_STRUCTURE.ADOP",
            "INDIVIDUAL_EVENT_STRUCTURE.BAPM",
            "INDIVIDUAL_EVENT_STRUCTURE.BARM",
            "INDIVIDUAL_EVENT_STRUCTURE.BASM",
            "INDIVIDUAL_EVENT_STRUCTURE.BIRT",
            "INDIVIDUAL_EVENT_STRUCTURE.BLES",
            "INDIVIDUAL_EVENT_STRUCTURE.BURI",
            "INDIVIDUAL_EVENT_STRUCTURE.CENS",
            "INDIVIDUAL_EVENT_STRUCTURE.CHR",
            "INDIVIDUAL_EVENT_STRUCTURE.CHRA",
            "INDIVIDUAL_EVENT_STRUCTURE.CONF",
            "INDIVIDUAL_EVENT_STRUCTURE.CREM",
            "INDIVIDUAL_EVENT_STRUCTURE.DEAT",
            "INDIVIDUAL_EVENT_STRUCTURE.EMIG",
            "INDIVIDUAL_EVENT_STRUCTURE.EVEN",
            "INDIVIDUAL_EVENT_STRUCTURE.FCOM",
            "INDIVIDUAL_EVENT_STRUCTURE.GRAD",
            "INDIVIDUAL_EVENT_STRUCTURE.IMMI",
            "INDIVIDUAL_EVENT_STRUCTURE.NATU",
            "INDIVIDUAL_EVENT_STRUCTURE.ORDN",
            "INDIVIDUAL_EVENT_STRUCTURE.PROB",
            "INDIVIDUAL_EVENT_STRUCTURE.RETI",
            "INDIVIDUAL_EVENT_STRUCTURE.WILL",
        ],
        v70: [
            "ADOP", "ANUL", "BAPM", "BARM", "BASM", "BIRT", "BLES", "BURI", "CAST", "CHR", "CHRA",
            "CONF", "CREM", "DEAT", "DIV", "DIVF", "DSCR", "EDUC", "EMIG", "ENGA", "FAM-CENS",
            "FAM-EVEN", "FAM-FACT", "FAM-NCHI", "FAM-RESI", "FCOM", "GRAD", "IDNO", "IMMI",
            "INDI-CENS", "INDI-EVEN", "INDI-FACT", "INDI-NCHI", "INDI-RELI", "INDI-RESI",
            "INDI-TITL", "MARB", "MARC", "MARL", "MARR", "MARS", "NATI", "NATU", "NMR", "OCCU",
            "ORDN", "PROB", "PROP", "RETI", "SSN", "WILL",
        ],
        v71: [
            "ADOP", "ANUL", "BAPM", "BARM", "BASM", "BIRT", "BLES", "BURI", "CAST", "CHR", "CHRA",
            "CONF", "CREM", "DEAT", "DIV", "DIVF", "DSCR", "EDUC", "EMIG", "ENGA", "FAM-CENS",
            "FAM-EVEN", "FAM-FACT", "FAM-NCHI", "FAM-RESI", "FCOM", "GRAD", "IDNO", "IMMI",
            "INDI-CENS", "INDI-EVEN", "INDI-FACT", "INDI-NCHI", "INDI-RELI", "INDI-RESI",
            "INDI-TITL", "MARB", "MARC", "MARL", "MARR", "MARS", "NATI", "NATU", "NMR", "OCCU",
            "ORDN", "PROB", "PROP", "RETI", "SSN", "WILL",
        ],
    }
}

impl Event {
    /// An event of this kind, with nothing else.
    #[must_use]
    pub fn new(kind: EventKind) -> Self {
        Self {
            kind,
            ..Self::default()
        }
    }
}

gedcom_struct! {
    /// A spouse at a family event (`HUSB` or `WIFE` under it) and their age
    /// then.
    pub struct EventSpouse {
        /// The age (`AGE`).
        "AGE" => age: Option<Age>,
    }
    spec {
        v551: ["FAMILY_EVENT_DETAIL.HUSB", "FAMILY_EVENT_DETAIL.WIFE"],
        v70: ["HUSB", "WIFE"],
        v71: ["HUSB", "WIFE"],
    }
}

gedcom_struct! {
    /// The family of a birth, christening or adoption (`FAMC` under the
    /// event) and, for an adoption, who adopted (`ADOP`).
    pub struct EventFamily {
        @payload
        /// The family (7.x: `@VOID@` when unknown).
        family: Option<XrefId>;
        /// Who adopted the child (`ADOP`), with a phrase in 7.x.
        "ADOP" => adopted_by: Option<Phrased<Adoption>>,
    }
    spec {
        v551: [
            "INDIVIDUAL_EVENT_STRUCTURE.ADOP.FAMC",
            "INDIVIDUAL_EVENT_STRUCTURE.BIRT.FAMC",
        ],
        v70: ["ADOP-FAMC", "FAMC"],
        v71: ["ADOP-FAMC", "FAMC"],
    }
}

gedcom_struct! {
    /// An event an individual or a family is known not to have had (7.x
    /// `NO`), possibly within a period.
    pub struct NonEvent {
        @payload
        /// The event that did not happen.
        kind: EventKind;
        /// The period it did not happen in (`DATE`).
        "DATE" => date: Option<Period>,
        /// Notes (`NOTE`, `SNOTE`).
        "NOTE" | "SNOTE" => notes: ThinVec<Note>,
        /// Sources (`SOUR`).
        "SOUR" => citations: ThinVec<Citation>,
    }
    spec {
        v551: [],
        v70: ["NO"],
        v71: ["NO"],
    }
}
