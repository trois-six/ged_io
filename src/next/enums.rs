//! Every enumeration of GEDCOM 5.5.1, 7.0 and 7.1, typed.
//!
//! An enumeration value is read without ever failing: a value the
//! enumeration names, in either version's spelling and in any case
//! (5.5.1 p. 21: controlled values are case-insensitive), becomes its
//! variant; anything else — an extension value (`_X`), a 5.5.1 user value
//! of an open set, a 7.x value read from a 5.5.1 file, a misspelling —
//! becomes `Unknown` with the text as written. A variant is written in the
//! target version's spelling (5.5.1 `adopted`, 7.x `ADOPTED`; 5.5.1
//! `DNS/CAN`, 7.x `DNS_CAN`), or in the other version's when its own has
//! none; `Unknown` is written as read. The writer's conformance pass then
//! repairs what the target does not permit (7.x `OTHER` and a `PHRASE`, or
//! an extension structure).
//!
//! The 7.x `PHRASE` that qualifies a value is kept by [`Phrased`], the
//! structure of an enumeration payload with its phrase.
//!
//! Languages are not an enumeration here: 7.x tags them in BCP 47, an open
//! grammar, and 5.5.1's `LANGUAGE_ID` names are kept as text too.

use crate::tree::{Payload, Structure};
use crate::version::GedcomVersion;

use super::driver::{gedcom_struct, NodeRef, PayloadField, ReadCx, WriteCx};
use super::text::Text;

/// One value of an enumeration: its variant name and its spellings in
/// 5.5.1 and in 7.x, when the version has it.
#[derive(Clone, Copy, Debug)]
pub struct EnumValue {
    /// The variant.
    pub variant: &'static str,
    /// The 5.5.1 spelling.
    pub v551: Option<&'static str>,
    /// The 7.x spelling.
    pub v7: Option<&'static str>,
}

/// An enumeration type: its values and the sets of the specification
/// tables it stands for.
#[derive(Clone, Copy, Debug)]
pub struct EnumDesc {
    /// The type's name.
    pub name: &'static str,
    /// The 5.5.1 enumeration sets it stands for.
    pub v551: &'static [&'static str],
    /// The 7.x enumeration sets it stands for.
    pub v7: &'static [&'static str],
    /// The values the type names.
    pub values: &'static [EnumValue],
}

/// A value an enumeration names, written in a version.
pub(crate) trait Enumeration: Sized + Default {
    /// The type's description.
    const DESC: EnumDesc;
    /// The variant `s` names, in either spelling and any case; with a
    /// version, only among the variants that version has.
    fn known(s: &str, version: Option<GedcomVersion>) -> Option<Self>;
    /// The spelling of a known value in `version`.
    fn spelling(&self, version: GedcomVersion) -> Option<&'static str>;
    /// The text of an unknown value.
    fn unknown(&self) -> Option<&Text>;
    /// An unknown value.
    fn from_unknown(text: Text) -> Self;
}

/// Reads an enumeration payload: a known value, or the text as written.
/// No payload, or a pointer, does not fit.
pub(crate) fn read_enum<E: Enumeration>(node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> Option<E> {
    if node.is_pointer() || node.has_no_payload() {
        return None;
    }
    match E::known(node.payload_str(), Some(cx.version)) {
        Some(v) => Some(v),
        None => cx.text(node).map(E::from_unknown),
    }
}

/// Writes an enumeration payload.
pub(crate) fn write_enum<E: Enumeration>(value: &E, cx: &WriteCx<'_>) -> Payload {
    match (value.spelling(cx.version), value.unknown()) {
        (Some(s), _) => WriteCx::str(s),
        (None, Some(t)) => cx.text(t),
        (None, None) => Payload::None,
    }
}

/// Declares an enumeration: `Variant = 5.5.1 spelling / 7.x spelling`, `_`
/// where the version has none.
macro_rules! gedcom_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident (v551 = [$($set551:literal),*], v7 = [$($set7:literal),*]) {
            $(
                $(#[$vmeta:meta])*
                $variant:ident = $s551:tt / $s7:tt,
            )*
        }
    ) => {
        $(#[$meta])*
        #[non_exhaustive]
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub enum $name {
            $($(#[$vmeta])* $variant,)*
            /// A value this enumeration does not name, kept as written.
            Unknown(Text),
        }

        impl Default for $name {
            /// An empty unknown value.
            fn default() -> Self {
                Self::Unknown(Text::default())
            }
        }

        impl $name {
            /// The variants, in the order of `DESC.values`.
            const KNOWN: &'static [Self] = &[$(Self::$variant,)*];

            /// The value `s` names: a variant when `s` spells one in
            /// 5.5.1 or 7.x, in any case and with surrounding spaces;
            /// otherwise `Unknown`, owning `s`.
            #[must_use]
            pub fn parse(s: &str) -> Self {
                <Self as Enumeration>::known(s, None).unwrap_or_else(|| Self::Unknown(Text::new(s)))
            }

            /// The spelling of a known value in `version`: its own, or the
            /// other version's when it has none. `None` for `Unknown`.
            #[must_use]
            pub fn as_str(&self, version: GedcomVersion) -> Option<&'static str> {
                <Self as Enumeration>::spelling(self, version)
            }

            /// Whether the value is one the enumeration names.
            #[must_use]
            pub fn is_known(&self) -> bool {
                !matches!(self, Self::Unknown(_))
            }
        }

        impl Enumeration for $name {
            const DESC: EnumDesc = EnumDesc {
                name: stringify!($name),
                v551: &[$($set551),*],
                v7: &[$($set7),*],
                values: &[$(EnumValue {
                    variant: stringify!($variant),
                    v551: gedcom_enum!(@opt $s551),
                    v7: gedcom_enum!(@opt $s7),
                },)*],
            };

            fn known(s: &str, version: Option<GedcomVersion>) -> Option<Self> {
                find_known(Self::DESC.values, s, version).and_then(|i| Self::KNOWN.get(i).cloned())
            }

            fn spelling(&self, version: GedcomVersion) -> Option<&'static str> {
                let (v551, v7): (Option<&'static str>, Option<&'static str>) = match self {
                    $(Self::$variant => (gedcom_enum!(@opt $s551), gedcom_enum!(@opt $s7)),)*
                    Self::Unknown(_) => return None,
                };
                if version.is_v7() {
                    v7.or(v551)
                } else {
                    v551.or(v7)
                }
            }

            fn unknown(&self) -> Option<&Text> {
                match self {
                    Self::Unknown(t) => Some(t),
                    _ => None,
                }
            }

            fn from_unknown(text: Text) -> Self {
                Self::Unknown(text)
            }
        }

        impl PayloadField for $name {
            fn read(node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> Option<Self> {
                read_enum(node, cx)
            }

            fn write(&self, cx: &WriteCx<'_>) -> Payload {
                write_enum(self, cx)
            }
        }

        $crate::next::driver::leaf!($name);
    };
    (@opt _) => { None };
    (@opt $l:literal) => { Some($l) };
}

/// The index of the value `s` spells, in either version and any case,
/// with surrounding spaces. With a version, a value that version does not
/// have is not recognised: a 5.5.1 file's `Other` stays as written rather
/// than becoming 7.x's `OTHER`, which 5.5.1 could not write back.
fn find_known(values: &[EnumValue], s: &str, version: Option<GedcomVersion>) -> Option<usize> {
    let s = s.trim();
    values.iter().position(|v| {
        let own = match version {
            Some(version) if version.is_v7() => v.v7.is_some(),
            Some(_) => v.v551.is_some(),
            None => true,
        };
        own && (v.v551.is_some_and(|x| x.eq_ignore_ascii_case(s))
            || v.v7.is_some_and(|x| x.eq_ignore_ascii_case(s)))
    })
}

gedcom_enum! {
    /// Who adopted a child (`ADOP`): 5.5.1 `ADOPTED_BY_WHICH_PARENT`, 7.x
    /// `enumset-ADOP`.
    pub enum Adoption (v551 = ["ADOPTED_BY_WHICH_PARENT"], v7 = ["enumset-ADOP"]) {
        /// Adopted by the husband (`HUSB`).
        Husband = "HUSB" / "HUSB",
        /// Adopted by the wife (`WIFE`).
        Wife = "WIFE" / "WIFE",
        /// Adopted by both (`BOTH`).
        Both = "BOTH" / "BOTH",
    }
}

gedcom_enum! {
    /// The quality of a source's evidence (`QUAY`): 5.5.1
    /// `CERTAINTY_ASSESSMENT`, 7.x `enumset-QUAY`.
    pub enum Certainty (v551 = ["CERTAINTY_ASSESSMENT"], v7 = ["enumset-QUAY"]) {
        /// `0`: unreliable evidence or estimated data.
        Unreliable = "0" / "0",
        /// `1`: questionable reliability of evidence.
        Questionable = "1" / "1",
        /// `2`: secondary evidence.
        Secondary = "2" / "2",
        /// `3`: direct and primary evidence.
        Primary = "3" / "3",
    }
}

gedcom_enum! {
    /// The character set a 5.5.1 file declares (`HEAD.CHAR`); 7.x is
    /// always UTF-8 and has no such structure.
    pub enum CharacterSet (v551 = ["CHARACTER_SET"], v7 = []) {
        /// ANSEL (ANSI Z39.47).
        Ansel = "ANSEL" / _,
        /// UTF-8.
        Utf8 = "UTF-8" / _,
        /// UTF-16 (`UNICODE`).
        Unicode = "UNICODE" / _,
        /// ASCII.
        Ascii = "ASCII" / _,
    }
}

gedcom_enum! {
    /// How sure a child-to-family link is (`FAMC.STAT`): 5.5.1
    /// `CHILD_LINKAGE_STATUS`, 7.x `enumset-FAMC-STAT`.
    pub enum ChildStatus (v551 = ["CHILD_LINKAGE_STATUS"], v7 = ["enumset-FAMC-STAT"]) {
        /// Linking the child to the family is suspect.
        Challenged = "challenged" / "CHALLENGED",
        /// The link has been disproven.
        Disproven = "disproven" / "DISPROVEN",
        /// The link has been proven.
        Proven = "proven" / "PROVEN",
    }
}

gedcom_enum! {
    /// The form of a 5.5.1 file (`HEAD.GEDC.FORM`).
    pub enum GedcomForm (v551 = ["GEDCOM_FORM"], v7 = []) {
        /// `LINEAGE-LINKED`.
        LineageLinked = "LINEAGE-LINKED" / _,
    }
}

gedcom_enum! {
    /// The status of a Latter-day Saint ordinance (`STAT`): the 5.5.1
    /// `LDS_*_DATE_STATUS` sets together, 7.x `enumset-ord-STAT`.
    pub enum OrdinanceStatus (
            v551 = [
                "LDS_BAPTISM_DATE_STATUS",
                "LDS_CHILD_SEALING_DATE_STATUS",
                "LDS_ENDOWMENT_DATE_STATUS",
                "LDS_SPOUSE_SEALING_DATE_STATUS"
            ],
            v7 = ["enumset-ord-STAT"]
        ) {
        /// Born in the covenant.
        BornInCovenant = "BIC" / "BIC",
        /// The sealing was cancelled.
        Canceled = "CANCELED" / "CANCELED",
        /// Died before eight years old.
        Child = "CHILD" / "CHILD",
        /// Completed.
        Completed = "COMPLETED" / "COMPLETED",
        /// Do not seal.
        DoNotSeal = "DNS" / "DNS",
        /// Do not seal, previous sealing cancelled (5.5.1 `DNS/CAN`, 7.x
        /// `DNS_CAN`).
        DoNotSealCanceled = "DNS/CAN" / "DNS_CAN",
        /// Excluded from this ordinance.
        Excluded = "EXCLUDED" / "EXCLUDED",
        /// Died before less than one year old.
        Infant = "INFANT" / "INFANT",
        /// Completed before 1970 (5.5.1 `PRE-1970`, 7.x `PRE_1970`).
        Pre1970 = "PRE-1970" / "PRE_1970",
        /// Stillborn.
        Stillborn = "STILLBORN" / "STILLBORN",
        /// Submitted, not yet completed.
        Submitted = "SUBMITTED" / "SUBMITTED",
        /// Not cleared.
        Uncleared = "UNCLEARED" / "UNCLEARED",
    }
}

gedcom_enum! {
    /// The format of a 5.5.1 multimedia file (`FORM`); 7.x gives a media
    /// type instead, which [`FileForm`](super::FileForm) keeps as text.
    pub enum MultimediaFormat (v551 = ["MULTIMEDIA_FORMAT"], v7 = []) {
        /// Bitmap.
        Bmp = "bmp" / _,
        /// GIF.
        Gif = "gif" / _,
        /// JPEG.
        Jpg = "jpg" / _,
        /// OLE.
        Ole = "ole" / _,
        /// PCX.
        Pcx = "pcx" / _,
        /// TIFF.
        Tif = "tif" / _,
        /// WAV audio.
        Wav = "wav" / _,
    }
}

gedcom_enum! {
    /// The kind of a name (`NAME.TYPE`): 5.5.1 `NAME_TYPE` (an open set),
    /// 7.x `enumset-NAME-TYPE`.
    pub enum NameType (v551 = ["NAME_TYPE"], v7 = ["enumset-NAME-TYPE"]) {
        /// Also known as.
        Aka = "aka" / "AKA",
        /// Name given at or near birth.
        Birth = "birth" / "BIRTH",
        /// Name assumed at immigration.
        Immigrant = "immigrant" / "IMMIGRANT",
        /// Maiden name.
        Maiden = "maiden" / "MAIDEN",
        /// Name assumed at marriage.
        Married = "married" / "MARRIED",
        /// Name used professionally (7.x).
        Professional = _ / "PROFESSIONAL",
        /// Another kind, which a `PHRASE` describes (7.x).
        Other = _ / "OTHER",
    }
}

gedcom_enum! {
    /// Whether ordinances of a 5.5.1 submission are to be processed
    /// (`SUBN.ORDI`).
    pub enum OrdinanceFlag (v551 = ["ORDINANCE_PROCESS_FLAG"], v7 = []) {
        /// `yes`.
        Yes = "yes" / _,
        /// `no`.
        No = "no" / _,
    }
}

gedcom_enum! {
    /// How a child belongs to a family (`PEDI`): 5.5.1
    /// `PEDIGREE_LINKAGE_TYPE`, 7.x `enumset-PEDI`.
    pub enum Pedigree (v551 = ["PEDIGREE_LINKAGE_TYPE"], v7 = ["enumset-PEDI"]) {
        /// Adopted.
        Adopted = "adopted" / "ADOPTED",
        /// Birth parents.
        Birth = "birth" / "BIRTH",
        /// Foster child.
        Foster = "foster" / "FOSTER",
        /// Sealed to the family.
        Sealing = "sealing" / "SEALING",
        /// Another relationship, which a `PHRASE` describes (7.x).
        Other = _ / "OTHER",
    }
}

gedcom_enum! {
    /// The method of a 5.5.1 phonetic variation (`FONE.TYPE`), an open set.
    pub enum PhoneticType (v551 = ["PHONETIC_TYPE"], v7 = []) {
        /// Korean hangul.
        Hangul = "hangul" / _,
        /// Japanese kana.
        Kana = "kana" / _,
    }
}

gedcom_enum! {
    /// The method of a 5.5.1 romanized variation (`ROMN.TYPE`), an open
    /// set.
    pub enum RomanizedType (v551 = ["ROMANIZED_TYPE"], v7 = []) {
        /// Chinese pinyin.
        Pinyin = "pinyin" / _,
        /// Japanese romaji.
        Romaji = "romaji" / _,
        /// Chinese Wade-Giles.
        WadeGiles = "wadegiles" / _,
    }
}

gedcom_enum! {
    /// A restriction on a record or structure (`RESN`): 5.5.1
    /// `RESTRICTION_NOTICE`, 7.x `enumset-RESN` (a list of them).
    pub enum Restriction (v551 = ["RESTRICTION_NOTICE"], v7 = ["enumset-RESN"]) {
        /// Not to be distributed.
        Confidential = "confidential" / "CONFIDENTIAL",
        /// Not to be changed.
        Locked = "locked" / "LOCKED",
        /// Private: details removed.
        Privacy = "privacy" / "PRIVACY",
    }
}

gedcom_enum! {
    /// A role in an event or association (`ROLE`): 5.5.1 `ROLE_IN_EVENT`
    /// (an open set), 7.x `enumset-ROLE`.
    pub enum Role (v551 = ["ROLE_IN_EVENT"], v7 = ["enumset-ROLE"]) {
        /// Child.
        Child = "CHIL" / "CHIL",
        /// Religious official (7.x).
        Clergy = _ / "CLERGY",
        /// Father.
        Father = "FATH" / "FATH",
        /// Friend (7.x).
        Friend = _ / "FRIEND",
        /// Godparent (7.x).
        Godparent = _ / "GODP",
        /// Husband.
        Husband = "HUSB" / "HUSB",
        /// Mother.
        Mother = "MOTH" / "MOTH",
        /// Several roles (7.x).
        Multiple = _ / "MULTIPLE",
        /// Neighbour (7.x).
        Neighbor = _ / "NGHBR",
        /// Officiator (7.x).
        Officiator = _ / "OFFICIATOR",
        /// Another role, which a `PHRASE` describes (7.x).
        Other = _ / "OTHER",
        /// Parent (7.x).
        Parent = _ / "PARENT",
        /// Spouse.
        Spouse = "SPOU" / "SPOU",
        /// Wife.
        Wife = "WIFE" / "WIFE",
        /// Witness (7.x).
        Witness = _ / "WITN",
    }
}

gedcom_enum! {
    /// Sex (`SEX`): 5.5.1 `SEX_VALUE`, 7.x `enumset-SEX`.
    pub enum Sex (v551 = ["SEX_VALUE"], v7 = ["enumset-SEX"]) {
        /// Female (`F`).
        Female = "F" / "F",
        /// Male (`M`).
        Male = "M" / "M",
        /// Cannot be determined from the sources (`U`).
        Undetermined = "U" / "U",
        /// Neither only male nor only female (7.x `X`).
        Nonbinary = _ / "X",
    }
}

gedcom_enum! {
    /// The medium of a source (`MEDI`): 5.5.1 `SOURCE_MEDIA_TYPE`, 7.x
    /// `enumset-MEDI`.
    pub enum Medium (v551 = ["SOURCE_MEDIA_TYPE"], v7 = ["enumset-MEDI"]) {
        /// Audio recording.
        Audio = "audio" / "AUDIO",
        /// Bound book.
        Book = "book" / "BOOK",
        /// Card file.
        Card = "card" / "CARD",
        /// Electronic.
        Electronic = "electronic" / "ELECTRONIC",
        /// Microfiche.
        Fiche = "fiche" / "FICHE",
        /// Microfilm.
        Film = "film" / "FILM",
        /// Magazine.
        Magazine = "magazine" / "MAGAZINE",
        /// Manuscript.
        Manuscript = "manuscript" / "MANUSCRIPT",
        /// Map.
        Map = "map" / "MAP",
        /// Newspaper.
        Newspaper = "newspaper" / "NEWSPAPER",
        /// Photo.
        Photo = "photo" / "PHOTO",
        /// Tombstone.
        Tombstone = "tombstone" / "TOMBSTONE",
        /// Video recording.
        Video = "video" / "VIDEO",
        /// Another medium, which a `PHRASE` describes (7.x).
        Other = _ / "OTHER",
    }
}

gedcom_enum! {
    /// An event or attribute type, by its tag: what a citation's `EVEN`
    /// or a source's `DATA.EVEN` names, and what `NO` denies (7.x
    /// `enumset-EVENATTR`, which includes `enumset-EVEN`). 5.5.1 writes the
    /// same tags as text (its `ATTRIBUTE_TYPE` set lists the attributes).
    pub enum EventKind (v551 = ["ATTRIBUTE_TYPE"], v7 = ["enumset-EVENATTR", "enumset-EVEN"]) {
        /// `ADOP`.
        Adoption = "ADOP" / "ADOP",
        /// `ANUL`.
        Annulment = "ANUL" / "ANUL",
        /// `BAPM`.
        Baptism = "BAPM" / "BAPM",
        /// `BARM`.
        BarMitzvah = "BARM" / "BARM",
        /// `BASM`.
        BasMitzvah = "BASM" / "BASM",
        /// `BIRT`.
        Birth = "BIRT" / "BIRT",
        /// `BLES`.
        Blessing = "BLES" / "BLES",
        /// `BURI`.
        Burial = "BURI" / "BURI",
        /// `CAST`.
        Caste = "CAST" / "CAST",
        /// `CENS`.
        Census = "CENS" / "CENS",
        /// `CHR`.
        Christening = "CHR" / "CHR",
        /// `CHRA`.
        AdultChristening = "CHRA" / "CHRA",
        /// `CONF`.
        Confirmation = "CONF" / "CONF",
        /// `CREM`.
        Cremation = "CREM" / "CREM",
        /// `DEAT`.
        Death = "DEAT" / "DEAT",
        /// `DIV`.
        Divorce = "DIV" / "DIV",
        /// `DIVF`.
        DivorceFiled = "DIVF" / "DIVF",
        /// `DSCR`.
        PhysicalDescription = "DSCR" / "DSCR",
        /// `EDUC`.
        Education = "EDUC" / "EDUC",
        /// `EMIG`.
        Emigration = "EMIG" / "EMIG",
        /// `ENGA`.
        Engagement = "ENGA" / "ENGA",
        /// `EVEN`.
        Event = "EVEN" / "EVEN",
        /// `FACT`.
        Fact = "FACT" / "FACT",
        /// `FCOM`.
        FirstCommunion = "FCOM" / "FCOM",
        /// `GRAD`.
        Graduation = "GRAD" / "GRAD",
        /// `IDNO`.
        IdNumber = "IDNO" / "IDNO",
        /// `IMMI`.
        Immigration = "IMMI" / "IMMI",
        /// `MARB`.
        MarriageBann = "MARB" / "MARB",
        /// `MARC`.
        MarriageContract = "MARC" / "MARC",
        /// `MARL`.
        MarriageLicense = "MARL" / "MARL",
        /// `MARR`.
        Marriage = "MARR" / "MARR",
        /// `MARS`.
        MarriageSettlement = "MARS" / "MARS",
        /// `NATI`.
        Nationality = "NATI" / "NATI",
        /// `NATU`.
        Naturalization = "NATU" / "NATU",
        /// `NCHI`.
        ChildrenCount = "NCHI" / "NCHI",
        /// `NMR`.
        MarriageCount = "NMR" / "NMR",
        /// `OCCU`.
        Occupation = "OCCU" / "OCCU",
        /// `ORDN`.
        Ordination = "ORDN" / "ORDN",
        /// `PROB`.
        Probate = "PROB" / "PROB",
        /// `PROP`.
        Property = "PROP" / "PROP",
        /// `RELI`.
        Religion = "RELI" / "RELI",
        /// `RESI`.
        Residence = "RESI" / "RESI",
        /// `RETI`.
        Retirement = "RETI" / "RETI",
        /// `SSN`.
        SocialSecurityNumber = "SSN" / "SSN",
        /// `TITL`.
        Title = "TITL" / "TITL",
        /// `WILL`.
        Will = "WILL" / "WILL",
    }
}

gedcom_enum! {
    /// Whether a child was born alive (7.1 `BIRT.KIND`, `enumset-BIRT-KIND`).
    pub enum BirthKind (v551 = [], v7 = ["enumset-BIRT-KIND"]) {
        /// Born dead.
        BornDead = _ / "BORN_DEAD",
        /// Born alive.
        BornLive = _ / "BORN_LIVE",
    }
}

gedcom_enum! {
    /// What a note is about (7.1 `NOTE.KIND`, `enumset-NOTE-KIND`).
    pub enum NoteKind (v551 = [], v7 = ["enumset-NOTE-KIND"]) {
        /// Data transcribed or abstracted.
        Data = _ / "DATA",
        /// Another kind, which a `PHRASE` describes.
        Other = _ / "OTHER",
        /// Reasoning behind a conclusion.
        Reasoning = _ / "REASONING",
        /// Research notes.
        Research = _ / "RESEARCH",
        /// The scope of a work.
        Scope = _ / "SCOPE",
        /// A story.
        Story = _ / "STORY",
        /// Work to do.
        Todo = _ / "TODO",
    }
}

/// Every enumeration type, for the coverage ledger.
pub(crate) const ENUMS: &[EnumDesc] = &[
    Adoption::DESC,
    BirthKind::DESC,
    Certainty::DESC,
    CharacterSet::DESC,
    ChildStatus::DESC,
    EventKind::DESC,
    GedcomForm::DESC,
    Medium::DESC,
    MultimediaFormat::DESC,
    NameType::DESC,
    NoteKind::DESC,
    OrdinanceFlag::DESC,
    OrdinanceStatus::DESC,
    Pedigree::DESC,
    PhoneticType::DESC,
    Restriction::DESC,
    Role::DESC,
    RomanizedType::DESC,
    Sex::DESC,
];

/// A comma-separated list of enumeration values (`RESN CONFIDENTIAL,
/// LOCKED`): each item typed, written `A, B`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnumList<E>(pub Vec<E>);

impl<E: Enumeration> PayloadField for EnumList<E> {
    fn read(node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> Option<Self> {
        if node.is_pointer() || node.has_no_payload() {
            return None;
        }
        let items = node
            .payload_str()
            .split(',')
            .map(|item| {
                E::known(item, Some(cx.version))
                    .unwrap_or_else(|| E::from_unknown(Text::new(item.trim())))
            })
            .collect();
        Some(Self(items))
    }

    fn write(&self, cx: &WriteCx<'_>) -> Payload {
        let items: Vec<&str> = self
            .0
            .iter()
            .map(|e| match (e.spelling(cx.version), e.unknown()) {
                (Some(s), _) => s,
                (None, Some(t)) => t.as_str(cx.source),
                (None, None) => "",
            })
            .collect();
        WriteCx::str(&items.join(", "))
    }
}

impl<E: Enumeration> super::driver::FromNode for EnumList<E> {
    fn from_node(node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> Option<Self> {
        super::driver::read_leaf(node, cx)
    }
}

impl<E: Enumeration> super::driver::ToNodes for EnumList<E> {
    fn to_node(&self, tag: &'static str, cx: &WriteCx<'_>) -> Structure {
        super::driver::write_leaf(self, tag, cx)
    }
}

/// The structure of an enumeration value with the 7.x `PHRASE` that words
/// it (`PEDI OTHER` + `PHRASE Guardianship`): `PEDI`, `FAMC.STAT`,
/// `ADOP`, `ROLE`, `NAME.TYPE`, `MEDI`, 7.1 `NOTE.KIND`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Phrased<E> {
    /// The value.
    pub value: E,
    /// The value in words (7.x).
    pub phrase: Option<Text>,
    /// Substructures no field holds.
    pub extra: super::Extra,
}

/// The fields of [`Phrased`]: `PHRASE`.
const PHRASED_FIELDS: &[super::driver::FieldDesc] = &[super::driver::FieldDesc {
    tags: &["PHRASE"],
    name: "phrase",
    many: false,
}];

/// The field of each standard tag in [`Phrased`].
static PHRASED_LOOKUP: super::driver::Lookup = super::driver::lookup(PHRASED_FIELDS);

impl<E: Enumeration + Clone + PartialEq + std::fmt::Debug> super::driver::Struct for Phrased<E> {
    const FIELDS: &'static [super::driver::FieldDesc] = PHRASED_FIELDS;
    const LOOKUP: super::driver::Lookup = super::driver::lookup(PHRASED_FIELDS);
    const SPEC: super::driver::SpecNames = super::driver::SpecNames {
        v551: &[],
        v70: &[],
        v71: &[],
    };
}

impl<E: Enumeration> super::driver::Fields for Phrased<E> {
    fn lookup(&self) -> &'static super::driver::Lookup {
        &PHRASED_LOOKUP
    }

    fn read_payload(&mut self, node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> bool {
        match read_enum(node, cx) {
            Some(v) => {
                self.value = v;
                true
            }
            None => false,
        }
    }

    fn accept(&mut self, field: usize, node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> bool {
        field == 0 && super::driver::Slot::accept(&mut self.phrase, node, cx)
    }

    fn finish(&mut self) {}

    fn extra(&self) -> &super::Extra {
        &self.extra
    }

    fn extra_mut(&mut self) -> &mut super::Extra {
        &mut self.extra
    }

    fn write_payload(&self, cx: &WriteCx<'_>) -> Payload {
        write_enum(&self.value, cx)
    }

    fn write_fields(&self, cx: &WriteCx<'_>, out: &mut Vec<Structure>) {
        super::driver::Slot::write(&self.phrase, "PHRASE", cx, out);
    }
}

impl<E: Enumeration + Clone + PartialEq + std::fmt::Debug> super::driver::FromNode for Phrased<E> {
    fn from_node(node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> Option<Self> {
        super::driver::read(node, cx)
    }
}

impl<E: Enumeration> super::driver::ToNodes for Phrased<E> {
    fn to_node(&self, tag: &'static str, cx: &WriteCx<'_>) -> Structure {
        super::driver::write(self, tag, cx)
    }
}

impl<E> Phrased<E> {
    /// A value without a phrase.
    pub fn new(value: E) -> Self {
        Self {
            value,
            phrase: None,
            extra: super::Extra::default(),
        }
    }
}

gedcom_struct! {
    /// The status of a Latter-day Saint ordinance (`STAT`) and the date it
    /// was set (5.5.1 `LDS_*.STAT`, 7.x `ord-STAT`).
    pub struct LdsStatus {
        @payload
        /// The status.
        value: OrdinanceStatus;
        /// When the status was set (`DATE`, with an optional `TIME`).
        "DATE" => date: Option<super::ExactDate>,
    }
    spec {
        v551: [
            "LDS_INDIVIDUAL_ORDINANCE.BAPL.STAT",
            "LDS_INDIVIDUAL_ORDINANCE.ENDL.STAT",
            "LDS_INDIVIDUAL_ORDINANCE.SLGC.STAT",
            "LDS_SPOUSE_SEALING.SLGS.STAT",
        ],
        v70: ["ord-STAT"],
        v71: ["ord-STAT"],
    }
}
