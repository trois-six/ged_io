//! Memory layout of the typed model (`ged_io::next`), checked at compile
//! time.
//!
//! The model reads into one shared text buffer (§1.13 of the redesign
//! plan): its budget is a peak of at most three times the decoded input,
//! the model itself at most twice. A GEDCOM line averages 20 to 30 bytes;
//! the targets below keep each line's share of the model under that:
//!
//! - a text (payload, value) is a 16-byte span, and so is an optional one;
//!   an identifier or pointer is a 4-byte interned id;
//! - an enumeration value is 16 bytes, its unknown text included;
//! - `extra`, empty on almost every structure, is one word, and so is a
//!   list of substructures that seldom appear ([`ThinVec`]: translations,
//!   identifiers, notes of a citation, …); a rare large part is boxed
//!   (a place's coordinates, a link's crop, a citation's data);
//! - a structure is its fields: 16 bytes per optional text, 24 per common
//!   list, 8 per rare list or part. Notes (80), citations (96) and places
//!   (104), the frequent ones, stay well within the plan's 160 for `Note`.

use std::mem::size_of;

use ged_io::next::{
    Address, Age, Association, CallNumber, ChangeDate, Child, Citation, CitationData, CitedEvent,
    Crop, Date, ExactDate, Exid, Extra, File, FileForm, Generic, LdsStatus, Map, MultimediaLink,
    Node, Note, NoteContent, Pedigree, Period, Phrased, Place, Refn, RepositoryCitation, Role,
    SourceText, TagId, Text, ThinVec, Value, XrefId,
};

const _: () = assert!(size_of::<Text>() == 16);
const _: () = assert!(size_of::<Option<Text>>() == 16);
const _: () = assert!(size_of::<XrefId>() == 4);
const _: () = assert!(size_of::<Option<XrefId>>() == 4);
const _: () = assert!(size_of::<TagId>() == 4);
const _: () = assert!(size_of::<Extra>() == 8);
const _: () = assert!(size_of::<ThinVec<Note>>() == 8);
const _: () = assert!(size_of::<Value>() == 16);
const _: () = assert!(size_of::<Node>() <= 48);
const _: () = assert!(size_of::<Pedigree>() == 16);
const _: () = assert!(size_of::<Option<Pedigree>>() == 16);
const _: () = assert!(size_of::<Phrased<Role>>() <= 40);
const _: () = assert!(size_of::<NoteContent>() == 16);

const _: () = assert!(size_of::<Note>() <= 80);
const _: () = assert!(size_of::<Citation>() <= 96);
const _: () = assert!(size_of::<Place>() <= 104);
const _: () = assert!(size_of::<Address>() <= 136);
const _: () = assert!(size_of::<Date>() <= 56);
const _: () = assert!(size_of::<ExactDate>() <= 40);
const _: () = assert!(size_of::<Period>() <= 40);
const _: () = assert!(size_of::<Age>() <= 40);
const _: () = assert!(size_of::<ChangeDate>() <= 56);
const _: () = assert!(size_of::<Refn>() <= 40);
const _: () = assert!(size_of::<Exid>() <= 40);
const _: () = assert!(size_of::<MultimediaLink>() <= 48);
const _: () = assert!(size_of::<File>() <= 80);
const _: () = assert!(size_of::<FileForm>() <= 32);
const _: () = assert!(size_of::<Crop>() <= 40);
const _: () = assert!(size_of::<Map>() <= 40);
const _: () = assert!(size_of::<RepositoryCitation>() <= 48);
const _: () = assert!(size_of::<CallNumber>() <= 64);
const _: () = assert!(size_of::<CitationData>() <= 88);
const _: () = assert!(size_of::<CitedEvent>() <= 80);
const _: () = assert!(size_of::<SourceText>() <= 56);
const _: () = assert!(size_of::<Association>() <= 104);
const _: () = assert!(size_of::<LdsStatus>() <= 64);
const _: () = assert!(size_of::<Generic>() <= 48);
const _: () = assert!(size_of::<Child>() <= 48);

/// The sizes, for the record (`cargo test --test sizes -- --nocapture`).
#[test]
fn sizes() {
    macro_rules! show {
        ($($t:ty),*) => {$(println!("{:>24} {}", stringify!($t), size_of::<$t>());)*};
    }
    show!(
        Text,
        Option<Text>,
        XrefId,
        Extra,
        Value,
        Node,
        Pedigree,
        Phrased<Role>,
        Note,
        Citation,
        Place,
        Address,
        Date,
        ExactDate,
        Period,
        Age,
        ChangeDate,
        Refn,
        Exid,
        MultimediaLink,
        File,
        FileForm,
        Crop,
        Map,
        RepositoryCitation,
        CallNumber,
        CitationData,
        CitedEvent,
        SourceText,
        Association,
        LdsStatus,
        Generic,
        Child
    );
}
