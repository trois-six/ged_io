//! The header of a dataset (`HEAD`) and its substructures.

use super::dates::ExactDate;
use super::driver::gedcom_struct;
use super::enums::{CharacterSet, GedcomForm};
use super::identifiers::Address;
use super::note::{Note, TextTranslation};
use super::text::{Text, XrefId};

gedcom_struct! {
    /// The header (`HEAD`): what the file is (`GEDC`), its character set
    /// (5.5.1 `CHAR`), the product that wrote it (`SOUR`), its recipient,
    /// date, submitter, copyright, language and place form, a note, the
    /// extension tags it uses (7.x `SCHMA`), and 7.1's title and
    /// description.
    ///
    /// The writer completes it for the target version: it writes `GEDC`
    /// (and in 5.5.1 `CHAR`, `SOUR` and `SUBM`) as that version requires,
    /// whatever the header holds.
    pub struct Header {
        /// The GEDCOM version and form (`GEDC`).
        "GEDC" => gedcom: Option<GedcomInfo>,
        /// The character set of a 5.5.1 file (`CHAR`).
        "CHAR" => charset: Option<Charset>,
        /// The product that wrote the file (`SOUR`).
        "SOUR" => source: Option<HeaderSource>,
        /// The product the file is for (`DEST`).
        "DEST" => destination: Option<Text>,
        /// When the file was written (`DATE`).
        "DATE" => date: Option<ExactDate>,
        /// The submitter of the file (`SUBM`).
        "SUBM" => submitter: Option<XrefId>,
        /// The submission record (5.5.1 `SUBN`).
        "SUBN" => submission: Option<XrefId>,
        /// The file's name (5.5.1 `FILE`).
        "FILE" => file: Option<Text>,
        /// The copyright of the file (`COPR`).
        "COPR" => copyright: Option<Text>,
        /// The language of the file's texts (`LANG`).
        "LANG" => language: Option<Text>,
        /// The jurisdictions places name by default (`PLAC`).
        "PLAC" => place: Option<HeaderPlace>,
        /// Notes about the file (`NOTE`, 7.x `SNOTE`; one before 7.1).
        "NOTE" | "SNOTE" => notes: Vec<Note>,
        /// The extension tags the file uses and their URIs (7.x `SCHMA`).
        "SCHMA" => schema: Option<Schema>,
        /// The title of the file (7.1 `TITL`).
        "TITL" => title: Option<HeaderText>,
        /// A description of the file (7.1 `DESC`).
        "DESC" => description: Option<HeaderText>,
    }
    spec {
        v551: ["HEADER.HEAD"],
        v70: ["HEAD"],
        v71: ["HEAD"],
    }
}

gedcom_struct! {
    /// What a file is (`GEDC`): the GEDCOM version (`VERS`) and, in 5.5.1,
    /// its form (`FORM`), any value of which is kept (D15f).
    pub struct GedcomInfo {
        /// The version (`VERS`).
        "VERS" => version: Option<Text>,
        /// The form (5.5.1 `FORM`).
        "FORM" => form: Option<GedcomForm>,
    }
    spec {
        v551: ["HEADER.HEAD.GEDC"],
        v70: ["GEDC"],
        v71: ["GEDC"],
    }
}

gedcom_struct! {
    /// The character set of a 5.5.1 file (`CHAR`) and its version.
    pub struct Charset {
        @payload
        /// The character set.
        value: CharacterSet;
        /// Its version (`VERS`).
        "VERS" => version: Option<Text>,
    }
    spec {
        v551: ["HEADER.HEAD.CHAR"],
        v70: [],
        v71: [],
    }
}

gedcom_struct! {
    /// The product that wrote a file (`SOUR` of `HEAD`): its identifier,
    /// version and name, its maker (`CORP`) and the data it was taken from
    /// (`DATA`).
    pub struct HeaderSource {
        @payload
        /// The product's identifier.
        product: Text;
        /// Its version (`VERS`).
        "VERS" => version: Option<Text>,
        /// Its name (`NAME`).
        "NAME" => name: Option<Text>,
        /// Its maker (`CORP`).
        "CORP" => corporation: Option<Corporation>,
        /// The data it was taken from (`DATA`).
        "DATA" => data: Option<HeaderSourceData>,
    }
    spec {
        v551: ["HEADER.HEAD.SOUR"],
        v70: ["HEAD-SOUR"],
        v71: ["HEAD-SOUR"],
    }
}

gedcom_struct! {
    /// The maker of a product (`CORP`): its name, address and contacts,
    /// every one kept (D8, D9).
    pub struct Corporation {
        @payload
        /// The name.
        name: Text;
        /// The address (`ADDR`).
        "ADDR" => address: Option<Address>,
        /// Telephone numbers (`PHON`).
        "PHON" => phones: Vec<Text>,
        /// E-mail addresses (`EMAIL`).
        "EMAIL" => emails: Vec<Text>,
        /// Fax numbers (`FAX`).
        "FAX" => faxes: Vec<Text>,
        /// Web pages (`WWW`).
        "WWW" => websites: Vec<Text>,
    }
    spec {
        v551: ["HEADER.HEAD.SOUR.CORP"],
        v70: ["CORP"],
        v71: ["CORP"],
    }
}

gedcom_struct! {
    /// The data a product took a file from (`DATA` of `HEAD.SOUR`): its
    /// name, date and copyright.
    pub struct HeaderSourceData {
        @payload
        /// The name of the data.
        name: Text;
        /// Its date (`DATE`).
        "DATE" => date: Option<ExactDate>,
        /// Its copyright (`COPR`).
        "COPR" => copyright: Option<Text>,
    }
    spec {
        v551: ["HEADER.HEAD.SOUR.DATA"],
        v70: ["HEAD-SOUR-DATA"],
        v71: ["HEAD-SOUR-DATA"],
    }
}

gedcom_struct! {
    /// The default jurisdictions of a file's places (`PLAC` of `HEAD`).
    pub struct HeaderPlace {
        /// The jurisdictions, comma-separated (`FORM`).
        "FORM" => form: Option<Text>,
    }
    spec {
        v551: ["HEADER.HEAD.PLAC"],
        v70: ["HEAD-PLAC"],
        v71: ["HEAD-PLAC"],
    }
}

gedcom_struct! {
    /// The extension tags a file uses (7.x `SCHMA`): each `TAG` names a tag
    /// and its URI (`_LOC http://…`).
    pub struct Schema {
        /// The tags and their URIs (`TAG`).
        "TAG" => tags: Vec<Text>,
    }
    spec {
        v551: [],
        v70: ["SCHMA"],
        v71: ["SCHMA"],
    }
}

gedcom_struct! {
    /// A title or description of a file (7.1 `TITL` or `DESC` of `HEAD`),
    /// its language and translations.
    pub struct HeaderText {
        @payload
        /// The text.
        text: Text;
        /// Its language (`LANG`).
        "LANG" => language: Option<Text>,
        /// Translations (`TRAN`).
        "TRAN" => translations: Vec<TextTranslation>,
    }
    spec {
        v551: [],
        v70: [],
        v71: ["HEAD-DESC", "HEAD-TITL"],
    }
}
