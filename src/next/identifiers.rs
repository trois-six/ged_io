//! Identifiers given to records by users and systems, and addresses.

use super::driver::gedcom_struct;
use super::text::Text;

gedcom_struct! {
    /// A user reference number (`REFN`) and its kind (`TYPE`). A record
    /// may have any number of them.
    pub struct Refn {
        @payload
        /// The number.
        value: Text;
        /// What kind of number it is (`TYPE`).
        "TYPE" => kind: Option<Text>,
    }
    spec {
        v551: [
            "FAM_RECORD.FAM.REFN",
            "INDIVIDUAL_RECORD.INDI.REFN",
            "MULTIMEDIA_RECORD.OBJE.REFN",
            "NOTE_RECORD.NOTE.REFN",
            "REPOSITORY_RECORD.REPO.REFN",
            "SOURCE_RECORD.SOUR.REFN",
        ],
        v70: ["REFN"],
        v71: ["REFN"],
    }
}

gedcom_struct! {
    /// An identifier in an external system (7.x `EXID`) and the URI of the
    /// system that issued it (`TYPE`).
    pub struct Exid {
        @payload
        /// The identifier.
        value: Text;
        /// The issuer's URI (`TYPE`).
        "TYPE" => kind: Option<Text>,
    }
    spec {
        v551: [],
        v70: ["EXID"],
        v71: ["EXID"],
    }
}

gedcom_struct! {
    /// A postal address (`ADDR`): the address as a whole, with its lines,
    /// and its parts. The contacts that go with it (`PHON`, `EMAIL`, `FAX`,
    /// `WWW`) are fields of the structure that holds it.
    pub struct Address {
        @payload
        /// The whole address, lines joined with newlines.
        value: Text;
        /// First address line (`ADR1`).
        "ADR1" => line1: Option<Text>,
        /// Second address line (`ADR2`).
        "ADR2" => line2: Option<Text>,
        /// Third address line (`ADR3`).
        "ADR3" => line3: Option<Text>,
        /// City (`CITY`).
        "CITY" => city: Option<Text>,
        /// State or province (`STAE`).
        "STAE" => state: Option<Text>,
        /// Postal code (`POST`).
        "POST" => postal_code: Option<Text>,
        /// Country (`CTRY`).
        "CTRY" => country: Option<Text>,
    }
    spec {
        v551: ["ADDRESS_STRUCTURE.ADDR"],
        v70: ["ADDR"],
        v71: ["ADDR"],
    }
}
