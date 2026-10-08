//! Memory layout of the typed model (`ged_io::next`), checked at compile
//! time.
//!
//! The model reads into one shared text store (§1.13 of the redesign
//! plan): its budget is a peak of at most three times the decoded input,
//! the model itself at most twice. A GEDCOM line averages 20 to 30 bytes;
//! the targets below keep each line's share of the model under that:
//!
//! - a text (payload, value) is 16 bytes, and so is an optional one,
//!   whether a span of the input, pieces of it or owned; an identifier or
//!   pointer is a 4-byte interned id;
//! - an enumeration value is 16 bytes, its unknown text included;
//! - `extra`, empty on almost every structure, is one word, and so is a
//!   list of substructures that seldom repeat ([`ThinVec`]: one allocation
//!   for one item);
//! - the fields few occurrences of a structure use are in its boxed
//!   detail, one word when none is set: the frequent structures — records,
//!   events, names, links, notes, citations, places, dates — keep inline
//!   only what most of them have.

use std::mem::size_of;

use ged_io::next::{
    Address, Age, Association, CallNumber, ChangeDate, ChildLink, Citation, CitationData,
    CitedEvent, Crop, Date, Event, EventDetail, EventFamily, EventSpouse, ExactDate, Exid, Extra,
    Family, File, FileForm, Header, Individual, IndividualDetail, IndividualRef, LdsStatus, Map,
    Multimedia, MultimediaLink, Name, NamePiece, Node, Note, NoteContent, Ordinance, Pedigree,
    Period, Phrased, Place, Refn, Repository, RepositoryCitation, Role, SharedNote, Source,
    SourceText, SpouseLink, Submitter, TagId, Text, ThinVec, Value, XrefId,
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

// Records: one of each per record, in the dataset's lists.
const _: () = assert!(size_of::<Individual>() <= 176);
const _: () = assert!(size_of::<Family>() <= 208);
const _: () = assert!(size_of::<Source>() <= 368);
const _: () = assert!(size_of::<Repository>() <= 376);
const _: () = assert!(size_of::<Multimedia>() <= 208);
const _: () = assert!(size_of::<Submitter>() <= 408);
const _: () = assert!(size_of::<SharedNote>() <= 256);
const _: () = assert!(size_of::<Header>() <= 800);

// The frequent substructures: what most occurrences have, inline.
const _: () = assert!(size_of::<Event>() <= 120);
const _: () = assert!(size_of::<Name>() <= 40);
const _: () = assert!(size_of::<NamePiece>() <= 24);
const _: () = assert!(size_of::<ChildLink>() <= 24);
const _: () = assert!(size_of::<SpouseLink>() <= 24);
const _: () = assert!(size_of::<IndividualRef>() <= 24);
const _: () = assert!(size_of::<Note>() <= 32);
const _: () = assert!(size_of::<Citation>() <= 48);
const _: () = assert!(size_of::<Place>() <= 32);
const _: () = assert!(size_of::<Date>() <= 32);
const _: () = assert!(size_of::<ChangeDate>() <= 56);
const _: () = assert!(size_of::<Ordinance>() <= 120);

// Details and rarer structures, allocated when present.
const _: () = assert!(size_of::<IndividualDetail>() <= 384);
const _: () = assert!(size_of::<EventDetail>() <= 664);
const _: () = assert!(size_of::<Address>() <= 136);
const _: () = assert!(size_of::<ExactDate>() <= 40);
const _: () = assert!(size_of::<Period>() <= 40);
const _: () = assert!(size_of::<Age>() <= 40);
const _: () = assert!(size_of::<EventSpouse>() <= 48);
const _: () = assert!(size_of::<EventFamily>() <= 56);
const _: () = assert!(size_of::<Refn>() <= 40);
const _: () = assert!(size_of::<Exid>() <= 40);
const _: () = assert!(size_of::<MultimediaLink>() <= 48);
const _: () = assert!(size_of::<File>() <= 88);
const _: () = assert!(size_of::<FileForm>() <= 40);
const _: () = assert!(size_of::<Crop>() <= 40);
const _: () = assert!(size_of::<Map>() <= 40);
const _: () = assert!(size_of::<RepositoryCitation>() <= 48);
const _: () = assert!(size_of::<CallNumber>() <= 64);
const _: () = assert!(size_of::<CitationData>() <= 88);
const _: () = assert!(size_of::<CitedEvent>() <= 80);
const _: () = assert!(size_of::<SourceText>() <= 56);
const _: () = assert!(size_of::<Association>() <= 104);
const _: () = assert!(size_of::<LdsStatus>() <= 64);

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
        Individual,
        IndividualDetail,
        Family,
        Source,
        Repository,
        Multimedia,
        Submitter,
        SharedNote,
        Header,
        Event,
        EventDetail,
        Name,
        NamePiece,
        ChildLink,
        SpouseLink,
        IndividualRef,
        Note,
        Citation,
        Place,
        Date,
        ChangeDate,
        Ordinance,
        Address,
        ExactDate,
        Period,
        Age,
        EventSpouse,
        EventFamily,
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
        LdsStatus
    );
}
