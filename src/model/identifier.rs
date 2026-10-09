//! Identifiers given to records by users and by external systems.

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
