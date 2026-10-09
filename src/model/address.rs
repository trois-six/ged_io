//! Postal addresses.

use super::driver::gedcom_struct;
use super::text::Text;

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
