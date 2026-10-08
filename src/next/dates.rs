//! Dates, ages and times, and the change and creation dates of records.
//!
//! Values are kept as written ([`Text`]), so that nothing a grammar does not
//! know is lost; the grammars of [`crate::types::date`] and
//! [`crate::types::age`] read them on demand.

use crate::types::age::{self, AgeValue};
use crate::types::date as values;
use crate::types::date::time::Time;
use crate::types::date::value::{DateExact, DatePeriod, DateValue};

use super::driver::{gedcom_struct, WriteCx};
use super::list::ThinVec;
use super::note::Note;
use super::text::{Source, Text};

/// The characters of an optional text as they will be written — without
/// the characters the target version bans, which the writer leaves out (a
/// 5.5.1 tab becomes a space) — `None` when empty.
fn owned(text: Option<&Text>, cx: &WriteCx<'_>) -> Option<String> {
    let rules = cx.version.rules();
    text.map(|t| t.as_str(cx.source))
        .map(|s| {
            s.chars()
                .map(|c| {
                    if c == '\t' && rules.is_banned(c) {
                        ' '
                    } else {
                        c
                    }
                })
                .filter(|&c| !rules.is_banned(c))
                .collect::<String>()
        })
        .filter(|s| !s.is_empty())
}

/// A date, its time and its phrase in the target version's grammar
/// ([`values::Date::to_version`]): `None` when they are in it already, or
/// when the grammar cannot read them (they are written as they are).
fn date_to_version(
    value: &Text,
    time: Option<&Text>,
    phrase: Option<&Text>,
    cx: &WriteCx<'_>,
) -> Option<values::Date> {
    let date = values::Date {
        value: owned(Some(value), cx),
        time: owned(time, cx),
        phrase: owned(phrase, cx),
    };
    let converted = date.to_version(cx.version);
    (converted != date).then_some(converted)
}

fn text(s: Option<String>) -> Option<Text> {
    s.map(Text::new)
}

fn convert_date(date: &Date, cx: &WriteCx<'_>) -> Option<Date> {
    let c = date_to_version(&date.value, date.time.as_ref(), date.phrase.as_ref(), cx)?;
    Some(Date {
        value: Text::new(c.value.unwrap_or_default()),
        time: text(c.time),
        phrase: text(c.phrase),
        extra: date.extra.clone(),
    })
}

fn convert_exact(date: &ExactDate, cx: &WriteCx<'_>) -> Option<ExactDate> {
    let c = date_to_version(&date.value, date.time.as_ref(), None, cx)?;
    // An exact date has no phrase to carry what the payload cannot say.
    c.phrase.is_none().then(|| ExactDate {
        value: Text::new(c.value.unwrap_or_default()),
        time: text(c.time),
        extra: date.extra.clone(),
    })
}

fn convert_period(period: &Period, cx: &WriteCx<'_>) -> Option<Period> {
    let c = date_to_version(&period.value, None, period.phrase.as_ref(), cx)?;
    c.time.is_none().then(|| Period {
        value: Text::new(c.value.unwrap_or_default()),
        phrase: text(c.phrase),
        extra: period.extra.clone(),
    })
}

/// An age and its phrase in the target version's grammar
/// ([`age::Age::to_version`]).
fn convert_age(a: &Age, cx: &WriteCx<'_>) -> Option<Age> {
    let age = age::Age {
        value: owned(Some(&a.value), cx),
        phrase: owned(a.phrase.as_ref(), cx),
    };
    let c = age.to_version(cx.version);
    (c != age).then(|| Age {
        value: Text::new(c.value.unwrap_or_default()),
        phrase: text(c.phrase),
        extra: a.extra.clone(),
    })
}

gedcom_struct! {
    /// A date (`DATE`, 7.x also `SDATE`): a date value, the time of day
    /// (7.x) and the date in words (7.x `PHRASE`).
    ///
    /// Written in another version's grammar, it is converted
    /// ([`values::Date::to_version`]): `@#DJULIAN@` and `JULIAN`, `B.C.`
    /// and `BCE`, dual years, phrases.
    pub struct Date [convert = convert_date] {
        @payload
        /// The date value as written (`ABT 1900`, `@#DJULIAN@ 1 JAN 1700`).
        value: Text;
        /// The time (`TIME`).
        "TIME" => time: Option<Text>,
        /// The date in words (`PHRASE`).
        "PHRASE" => phrase: Option<Text>,
    }
    spec {
        v551: [
            "EVENT_DETAIL.DATE",
            "LDS_INDIVIDUAL_ORDINANCE.BAPL.DATE",
            "LDS_INDIVIDUAL_ORDINANCE.ENDL.DATE",
            "LDS_INDIVIDUAL_ORDINANCE.SLGC.DATE",
            "LDS_SPOUSE_SEALING.SLGS.DATE",
            "SOURCE_CITATION.SOUR.DATA.DATE",
        ],
        v70: ["DATE", "SDATE"],
        v71: ["DATE", "SDATE"],
    }
}

impl Date {
    /// The date value read by the date grammar of either version.
    #[must_use]
    pub fn parse<S: AsRef<Source> + ?Sized>(&self, source: &S) -> DateValue {
        DateValue::parse(self.value.as_str(source))
    }

    /// The time read by the time grammar.
    #[must_use]
    pub fn parse_time<S: AsRef<Source> + ?Sized>(&self, source: &S) -> Option<Time> {
        Time::parse(self.time.as_ref()?.as_str(source))
    }
}

gedcom_struct! {
    /// An exact date (`DATE` of `CHAN`, `CREA`, an ordinance status or the
    /// header) and its time.
    pub struct ExactDate [convert = convert_exact] {
        @payload
        /// The date as written (`1 JAN 2000`).
        value: Text;
        /// The time (`TIME`).
        "TIME" => time: Option<Text>,
    }
    spec {
        v551: [
            "CHANGE_DATE.CHAN.DATE",
            "HEADER.HEAD.DATE",
            "HEADER.HEAD.SOUR.DATA.DATE",
            "LDS_INDIVIDUAL_ORDINANCE.BAPL.STAT.DATE",
            "LDS_INDIVIDUAL_ORDINANCE.ENDL.STAT.DATE",
            "LDS_INDIVIDUAL_ORDINANCE.SLGC.STAT.DATE",
            "LDS_SPOUSE_SEALING.SLGS.STAT.DATE",
        ],
        v70: ["DATE-exact", "HEAD-DATE"],
        v71: ["DATE-exact", "HEAD-DATE"],
    }
}

impl ExactDate {
    /// The date read by the exact-date grammar; `None` when it does not
    /// follow it.
    #[must_use]
    pub fn parse<S: AsRef<Source> + ?Sized>(&self, source: &S) -> Option<DateExact> {
        DateExact::parse(self.value.as_str(source))
    }

    /// The time read by the time grammar.
    #[must_use]
    pub fn parse_time<S: AsRef<Source> + ?Sized>(&self, source: &S) -> Option<Time> {
        Time::parse(self.time.as_ref()?.as_str(source))
    }
}

gedcom_struct! {
    /// A date period (`DATE` of a source's recorded events, 7.x `NO`) and
    /// its phrase.
    pub struct Period [convert = convert_period] {
        @payload
        /// The period as written (`FROM 1900 TO 1910`).
        value: Text;
        /// The period in words (`PHRASE`).
        "PHRASE" => phrase: Option<Text>,
    }
    spec {
        v551: ["SOURCE_RECORD.SOUR.DATA.EVEN.DATE"],
        v70: ["DATA-EVEN-DATE", "NO-DATE"],
        v71: ["DATA-EVEN-DATE", "NO-DATE"],
    }
}

impl Period {
    /// The period read by the date-period grammar; `None` when it does not
    /// follow it.
    #[must_use]
    pub fn parse<S: AsRef<Source> + ?Sized>(&self, source: &S) -> Option<DatePeriod> {
        DatePeriod::parse(self.value.as_str(source))
    }
}

gedcom_struct! {
    /// An age at an event (`AGE`) and its phrase. Written in another
    /// version's grammar, it is converted ([`age::Age::to_version`]).
    pub struct Age [convert = convert_age] {
        @payload
        /// The age as written (`> 25y 3m`, `CHILD`).
        value: Text;
        /// The age in words (`PHRASE`).
        "PHRASE" => phrase: Option<Text>,
    }
    spec {
        v551: [
            "FAMILY_EVENT_DETAIL.HUSB.AGE",
            "FAMILY_EVENT_DETAIL.WIFE.AGE",
            "INDIVIDUAL_EVENT_DETAIL.AGE",
        ],
        v70: ["AGE"],
        v71: ["AGE"],
    }
}

impl Age {
    /// The age read by the age grammar of either version.
    #[must_use]
    pub fn parse<S: AsRef<Source> + ?Sized>(&self, source: &S) -> AgeValue {
        AgeValue::parse(self.value.as_str(source))
    }
}

gedcom_struct! {
    /// When a record last changed (`CHAN`), with notes about the change.
    pub struct ChangeDate {
        /// The date and time of the change (`DATE`).
        "DATE" => date: Option<ExactDate>,
        /// Notes about the change (`NOTE`, 7.x `SNOTE`).
        "NOTE" | "SNOTE" => notes: ThinVec<Note>,
    }
    spec {
        v551: ["CHANGE_DATE.CHAN"],
        v70: ["CHAN"],
        v71: ["CHAN"],
    }
}

gedcom_struct! {
    /// When a record was created (7.x `CREA`).
    pub struct CreationDate {
        /// The date and time of the creation (`DATE`).
        "DATE" => date: Option<ExactDate>,
    }
    spec {
        v551: [],
        v70: ["CREA"],
        v71: ["CREA"],
    }
}
