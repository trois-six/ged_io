//! Dates: the `DATE` structure and the date, time and calendar payloads.
//!
//! [`Date`] is the `DATE` structure as read: its payload, `TIME` and
//! `PHRASE`, kept verbatim. Its payload is interpreted on demand by
//! [`DateValue`], whose grammar reads both versions; [`Time`] reads the
//! `TIME`. [`Date::to_version`] rewrites a date into the grammar of GEDCOM
//! 5.5.1 or 7.0. With the `calendar` feature, dates convert between calendars
//! and compare chronologically.

pub mod calendar;
pub mod change_date;
#[cfg(feature = "calendar")]
pub mod conversion;
pub mod time;
pub mod value;

use crate::{
    parser::{parse_subset, Parser},
    tokenizer::Tokenizer,
    types::value::Grammar,
    GedcomError, GedcomVersion,
};

#[cfg(feature = "json")]
use serde::{Deserialize, Serialize};

pub use calendar::{Calendar, Epoch, Month};
#[cfg(feature = "calendar")]
pub use conversion::{CalendarError, Weekday, MAX_RATA_DIE, MAX_YEAR};
pub use time::Time;
pub use value::{Approximation, CalendarDate, DateExact, DatePeriod, DateValue};

/// The `DATE` structure: a date payload, with its `TIME` and `PHRASE`.
///
/// The three strings are kept exactly as read, so that a date the grammar
/// does not understand loses nothing. [`date_value`](Self::date_value) and
/// [`time_value`](Self::time_value) interpret them.
///
/// See <https://gedcom.io/specifications/FamilySearchGEDCOMv7.html#DATE>.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "json", derive(Serialize, Deserialize))]
pub struct Date {
    /// The payload: a `DateValue`, `DateExact` or `DatePeriod` depending on
    /// where the date stands, as written.
    pub value: Option<String>,
    /// The `TIME` substructure, as written.
    pub time: Option<String>,
    /// The `PHRASE` substructure (GEDCOM 7.0): the date in the words of the
    /// source, when the payload cannot say it.
    pub phrase: Option<String>,
}

impl Date {
    /// Creates a new `Date` from a `Tokenizer`.
    ///
    /// # Errors
    ///
    /// This function will return an error if parsing fails.
    pub fn new(tokenizer: &mut Tokenizer<'_>, level: u8) -> Result<Date, GedcomError> {
        let mut date = Date::default();
        date.parse(tokenizer, level)?;
        Ok(date)
    }

    /// The payload, interpreted by the date grammar of both versions; an
    /// absent payload is [`DateValue::Empty`]. Never fails: what the
    /// grammar does not understand is kept in [`CalendarDate::text`].
    ///
    /// ```
    /// use ged_io::types::date::{Approximation, Calendar, Date, DateValue};
    ///
    /// let date = Date { value: Some("ABT @#DJULIAN@ 1700".into()), ..Date::default() };
    /// let DateValue::Approximated(Approximation::About, year) = date.date_value() else { panic!() };
    /// assert_eq!((year.calendar, year.year), (Calendar::Julian, Some(1700)));
    /// ```
    #[must_use]
    pub fn date_value(&self) -> DateValue {
        self.value
            .as_deref()
            .map_or(DateValue::Empty, DateValue::parse)
    }

    /// The `TIME`, interpreted; `None` without one or when it is not a time.
    #[must_use]
    pub fn time_value(&self) -> Option<Time> {
        self.time.as_deref().and_then(Time::parse)
    }

    /// The payload and the time in one string (`2 OCT 2019 12:00`), or the
    /// time alone without a payload; `None` without a time.
    #[must_use]
    pub fn datetime(&self) -> Option<String> {
        let time = self.time.as_deref()?;
        Some(match self.value.as_deref().filter(|v| !v.is_empty()) {
            Some(value) => format!("{value} {time}"),
            None => time.to_string(),
        })
    }

    /// The date rewritten in the grammar of `version`, keeping everything it
    /// says.
    ///
    /// A payload that already follows the version's grammar is kept as
    /// written. Otherwise the payload is read and written back in the
    /// version's grammar:
    ///
    /// - to GEDCOM 7.0, calendar escapes become keywords (`@#DROMAN@` the
    ///   extension calendar `_ROMAN`), `B.C.` becomes `BCE`, `INT date
    ///   (phrase)` the date with the phrase as `PHRASE`, and `(phrase)` an
    ///   empty payload with the `PHRASE`. A dual year becomes one year (the
    ///   later one when it is the year that follows, as in `1648/49`, the
    ///   year written otherwise), or a `BET … AND …` range for a year alone,
    ///   and the original wording becomes the `PHRASE`;
    /// - to GEDCOM 5.5.1, keywords become escapes, `BCE` becomes `B.C.`, and
    ///   the `PHRASE` moves into the payload, as `INT date (phrase)` after a
    ///   single date or `(phrase)` alone. Next to a range or a period it
    ///   stays in [`phrase`](Self::phrase), which 5.5.1 cannot write.
    ///
    /// A payload the grammar cannot fully read, or that the version cannot
    /// express, is left as written, to be repaired by the writer. The time
    /// is rewritten likewise; GEDCOM 5.5.1 has no `Z` for UTC.
    #[must_use]
    pub fn to_version(&self, version: GedcomVersion) -> Date {
        self.convert(Grammar::from(version))
    }

    /// [`to_version`](Self::to_version) in `grammar`.
    fn convert(&self, grammar: Grammar) -> Date {
        let time = self.time.as_ref().map(|raw| convert_time(raw, grammar));
        let unchanged = || Date {
            value: self.value.clone(),
            time: time.clone(),
            phrase: self.phrase.clone(),
        };
        let raw = self
            .value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let phrase = self.phrase.as_deref().filter(|p| !p.is_empty());
        match grammar {
            Grammar::V7 => {
                let Some(raw) = raw else {
                    return unchanged();
                };
                if DateValue::strict(raw, grammar).is_ok() {
                    return unchanged();
                }
                let value = DateValue::parse(raw);
                if value.has_unrecognised_text() {
                    return unchanged();
                }
                let has_dual_year = value.dates().any(|d| d.dual_year.is_some());
                let single = matches!(value, DateValue::Date(_));
                let (mut value, mut carried) = match value {
                    DateValue::Phrase(text) => (DateValue::Empty, Some(text)),
                    DateValue::Interpreted { date, phrase } => (DateValue::Date(date), phrase),
                    value => (value, None),
                };
                if has_dual_year {
                    carried = Some(raw.to_string());
                    value = resolve_dual_years(value, single);
                }
                let payload = value.format(grammar);
                if DateValue::strict(&payload, grammar).is_err() {
                    return unchanged();
                }
                let phrase = match (phrase, carried) {
                    (Some(own), Some(carried)) if own != carried => return unchanged(),
                    (Some(own), _) => Some(own.to_string()),
                    (None, carried) => carried,
                };
                Date {
                    value: (!payload.is_empty()).then_some(payload),
                    time,
                    phrase,
                }
            }
            Grammar::V551 => {
                let value = match raw {
                    None => DateValue::Empty,
                    Some(raw) => {
                        if phrase.is_none() && DateValue::strict(raw, grammar).is_ok() {
                            return unchanged();
                        }
                        let value = DateValue::parse(raw);
                        if value.has_unrecognised_text() {
                            return unchanged();
                        }
                        value
                    }
                };
                let (value, leftover) = match (value, phrase) {
                    (DateValue::Empty, None) => return unchanged(),
                    (DateValue::Empty, Some(text)) => (DateValue::Phrase(text.to_string()), None),
                    (
                        DateValue::Date(date) | DateValue::Interpreted { date, phrase: None },
                        Some(text),
                    ) => (
                        DateValue::Interpreted {
                            date,
                            phrase: Some(text.to_string()),
                        },
                        None,
                    ),
                    (value, phrase) => (value, phrase.map(str::to_string)),
                };
                let payload = value.format(grammar);
                if DateValue::strict(&payload, grammar).is_err() {
                    return unchanged();
                }
                Date {
                    value: Some(payload),
                    time,
                    phrase: leftover,
                }
            }
        }
    }

    /// The date in the canonical spelling of `version`: converted as by
    /// [`to_version`](Self::to_version), then written back from its
    /// interpretation, upper case and single-spaced. Wording the grammar
    /// does not understand is kept as written.
    #[must_use]
    pub fn normalize(&self, version: GedcomVersion) -> Date {
        let grammar = Grammar::from(version);
        let converted = self.convert(grammar);
        let v7 = grammar == Grammar::V7;
        let value = converted.value.as_deref().map(|raw| {
            let value = DateValue::parse(raw);
            // A 7.0 payload has no room for a phrase that could not move.
            if v7 && value.phrase().is_some() {
                raw.to_string()
            } else {
                value.format(grammar)
            }
        });
        let time = converted
            .time
            .map(|raw| Time::parse(&raw).map_or(raw, |time| time.format(grammar)));
        Date {
            value,
            time,
            phrase: converted.phrase,
        }
    }

    /// The date with every date of its payload converted to `calendar`,
    /// written in the grammar of `version`; qualifiers, ranges, periods,
    /// the time and the phrase are kept.
    ///
    /// ```
    /// use ged_io::types::date::{Calendar, Date};
    /// use ged_io::GedcomVersion;
    ///
    /// let date = Date { value: Some("@#DJULIAN@ 15 MAR 1582".into()), ..Date::default() };
    /// let gregorian = date.convert_to(&Calendar::Gregorian, GedcomVersion::V5_5_1).unwrap();
    /// assert_eq!(gregorian.value.as_deref(), Some("25 MAR 1582"));
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a [`CalendarError`] when a date of the payload cannot be
    /// converted: it is incomplete, holds unrecognised words, is invalid or
    /// out of range, or its calendar has no arithmetic.
    #[cfg(feature = "calendar")]
    pub fn convert_to(
        &self,
        calendar: &Calendar,
        version: GedcomVersion,
    ) -> Result<Date, CalendarError> {
        let value = self.date_value().convert_to(calendar)?;
        let payload = value.format(Grammar::V551);
        let converted = Date {
            value: (!payload.is_empty()).then_some(payload),
            time: self.time.clone(),
            phrase: self.phrase.clone(),
        };
        Ok(converted.to_version(version))
    }
}

/// The time in `grammar`: kept when it follows it, rewritten when it is a
/// time, left as written otherwise.
fn convert_time(raw: &str, grammar: Grammar) -> String {
    if Time::strict(raw, grammar).is_ok() {
        return raw.to_string();
    }
    Time::parse(raw).map_or_else(|| raw.to_string(), |time| time.format(grammar))
}

/// Replaces dual years, which GEDCOM 7.0 does not have: a `single` date
/// that is a year alone becomes the range of its two years; otherwise the
/// later year is kept when it is the year that follows (`1648/49`, the
/// change of new year), and the year written when it is not.
fn resolve_dual_years(value: DateValue, single: bool) -> DateValue {
    if let (DateValue::Date(date), true) = (&value, single) {
        if let (None, None, None, Some(dual)) = (date.day, &date.month, &date.epoch, date.dual_year)
        {
            let first = CalendarDate {
                dual_year: None,
                ..date.clone()
            };
            let second = CalendarDate {
                year: Some(dual),
                ..first.clone()
            };
            return DateValue::Between(first, second);
        }
    }
    let mut value = value;
    for date in value.dates_mut() {
        if let Some(dual) = date.dual_year.take() {
            if date.year.and_then(|year| year.checked_add(1)) == Some(dual) {
                date.year = Some(dual);
            }
        }
    }
    value
}

impl Parser for Date {
    /// parse handles the DATE tag
    fn parse(&mut self, tokenizer: &mut Tokenizer<'_>, level: u8) -> Result<(), GedcomError> {
        self.value = Some(tokenizer.take_line_value()?);

        let handle_subset = |tag: &str, tokenizer: &mut Tokenizer<'_>| -> Result<(), GedcomError> {
            match tag {
                "TIME" => self.time = Some(tokenizer.take_line_value()?),
                "PHRASE" => self.phrase = Some(tokenizer.take_line_value()?),
                _ => {
                    // Leave unknown tags to `parse_subset`, which keeps them with
                    // their substructures.
                }
            }
            Ok(())
        };
        parse_subset(tokenizer, level, handle_subset)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::Gedcom;

    #[test]
    fn test_parse_date_with_phrase() {
        let sample = "\
            0 HEAD\n\
            1 GEDC\n\
            2 VERS 7.0\n\
            0 @I1@ INDI\n\
            1 NAME John /Doe/\n\
            1 BIRT\n\
            2 DATE 15 MAR 1820\n\
            3 PHRASE The Ides of March, 1820\n\
            0 TRLR";

        let mut doc = Gedcom::new(sample.chars()).unwrap();
        let data = doc.parse_data().unwrap();

        let birt_date = data.individuals[0].events[0].date.as_ref().unwrap();
        assert_eq!(birt_date.value.as_ref().unwrap(), "15 MAR 1820");
        assert_eq!(
            birt_date.phrase.as_ref().unwrap(),
            "The Ides of March, 1820"
        );
    }

    #[test]
    fn test_parse_date_record() {
        let sample = "\
            0 HEAD\n\
            1 GEDC\n\
            2 VERS 5.5\n\
            1 DATE 2 Oct 2019\n\
            2 TIME 0:00:00\n\
            0 @I1@ INDI\n\
            1 NAME Ancestor\n\
            1 BIRT\n\
            2 DATE BEF 1828\n\
            1 RESI\n\
            2 PLAC 100 Broadway, New York, NY 10005\n\
            2 DATE from 1900 to 1905\n\
            0 TRLR";

        let mut doc = Gedcom::new(sample.chars()).unwrap();
        let data = doc.parse_data().unwrap();

        let head_date = data.header.unwrap().date.unwrap();
        assert_eq!(head_date.value.unwrap(), "2 Oct 2019");

        let birt_date = data.individuals[0].events[0].date.as_ref().unwrap();
        assert_eq!(birt_date.value.as_ref().unwrap(), "BEF 1828");

        let resi_date = data.individuals[0].attributes[0].date.as_ref().unwrap();
        assert_eq!(resi_date.value.as_ref().unwrap(), "from 1900 to 1905");
    }

    #[test]
    fn test_parse_change_date_record() {
        let sample = "\
            0 HEAD\n\
            1 GEDC\n\
            2 VERS 5.5\n\
            2 FORM LINEAGE-LINKED\n\
            0 @MEDIA1@ OBJE\n\
            1 FILE /home/user/media/file_name.bmp\n\
            1 CHAN\n\
            2 DATE 1 APR 1998\n\
            3 TIME 12:34:56.789\n\
            2 NOTE A note\n\
            0 TRLR";

        let mut doc = Gedcom::new(sample.chars()).unwrap();
        let gedcom_data = doc.parse_data().unwrap();
        assert_eq!(gedcom_data.multimedia.len(), 1);

        let object = &gedcom_data.multimedia[0];

        let chan = object.change_date.as_ref().unwrap();
        let date = chan.date.as_ref().unwrap();
        assert_eq!(date.value.as_ref().unwrap(), "1 APR 1998");
        assert_eq!(date.time.as_ref().unwrap(), "12:34:56.789");

        let chan_note = &chan.notes[0];
        assert_eq!(chan_note.value.as_ref().unwrap(), "A note");
    }

    #[test]
    fn test_parse_calendar_escapes() {
        // Test all 4 GEDCOM calendar types are preserved
        let calendars = [
            ("GREGORIAN", "@#DGREGORIAN@ 31 DEC 1997"),
            ("JULIAN", "@#DJULIAN@ 15 MAR 1582"),
            ("HEBREW", "@#DHEBREW@ 15 TSH 5784"),
            ("FRENCH_R", "@#DFRENCH R@ 1 VEND 1"),
        ];

        for (name, date_str) in calendars {
            let sample = format!(
                "0 HEAD\n\
                1 GEDC\n\
                2 VERS 5.5.1\n\
                0 @I1@ INDI\n\
                1 NAME Test /Person/\n\
                1 BIRT\n\
                2 DATE {date_str}\n\
                0 TRLR"
            );

            let mut doc = Gedcom::new(sample.chars()).unwrap();
            let gedcom_data = doc.parse_data().unwrap();

            let birt_date = gedcom_data.individuals[0].events[0].date.as_ref().unwrap();
            assert_eq!(
                birt_date.value.as_ref().unwrap(),
                date_str,
                "{name} calendar date should be preserved exactly"
            );
        }
    }
    use super::{Date, DateValue};
    use crate::GedcomVersion;

    const V551: GedcomVersion = GedcomVersion::V5_5_1;
    const V7: GedcomVersion = GedcomVersion::V7_0;

    fn date(value: &str) -> Date {
        Date {
            value: Some(value.to_string()),
            ..Date::default()
        }
    }

    fn with_phrase(value: Option<&str>, phrase: &str) -> Date {
        Date {
            value: value.map(str::to_string),
            time: None,
            phrase: Some(phrase.to_string()),
        }
    }

    #[test]
    fn test_calendar_after_qualifier_and_gedcom_7_keyword() {
        use super::Calendar;
        for (value, calendar) in [
            ("ABT @#DJULIAN@ 1700", Calendar::Julian),
            ("BET @#DJULIAN@ 1700 AND @#DJULIAN@ 1710", Calendar::Julian),
            ("JULIAN 10 MAY 1700", Calendar::Julian),
            ("FRENCH_R 1 VEND 2", Calendar::FrenchRepublican),
            ("@#DJULIAN@ ABT 1700", Calendar::Julian),
            ("ABT 1700", Calendar::Gregorian),
            ("@#DROMAN@ 1700", Calendar::Roman),
        ] {
            assert_eq!(date(value).date_value().calendar(), calendar, "{value}");
        }
        // The 5.5.1 escape is written after the qualifier.
        assert_eq!(
            date("@#DJULIAN@ ABT 1700").normalize(V551).value.as_deref(),
            Some("ABT @#DJULIAN@ 1700")
        );
    }

    #[test]
    fn test_datetime_never_panics() {
        assert_eq!(date("2 OCT 2019").datetime(), None);
        let with_time = Date {
            value: Some("2 OCT 2019".into()),
            time: Some("12:00".into()),
            phrase: None,
        };
        assert_eq!(with_time.datetime().as_deref(), Some("2 OCT 2019 12:00"));
        let time_only = Date {
            time: Some("12:00".into()),
            ..Date::default()
        };
        assert_eq!(time_only.datetime().as_deref(), Some("12:00"));
    }

    #[test]
    fn test_to_gedcom_7() {
        for (value, expected, phrase) in [
            ("2 OCT 2019", Some("2 OCT 2019"), None),
            ("@#DJULIAN@ 15 MAR 1582", Some("JULIAN 15 MAR 1582"), None),
            (
                "ABT @#DHEBREW@ 1 TSH 5600",
                Some("ABT HEBREW 1 TSH 5600"),
                None,
            ),
            ("44 B.C.", Some("44 BCE"), None),
            ("@#DROMAN@ 1860", Some("_ROMAN 1860"), None),
            ("from 1900 to 1905", Some("FROM 1900 TO 1905"), None),
            (
                "(the year of the flood)",
                None,
                Some("the year of the flood"),
            ),
            ("INT 1850 (about 1850)", Some("1850"), Some("about 1850")),
            (
                "30 JAN 1648/49",
                Some("30 JAN 1649"),
                Some("30 JAN 1648/49"),
            ),
            ("8 MAR 1401/8", Some("8 MAR 1401"), Some("8 MAR 1401/8")),
            ("1699/00", Some("BET 1699 AND 1700"), Some("1699/00")),
            ("1401/8 B.C.", Some("1401 BCE"), Some("1401/8 B.C.")),
            (
                "INT 799/81 (some text)",
                Some("799"),
                Some("INT 799/81 (some text)"),
            ),
            // Not convertible: left as written.
            ("vers 1850", Some("vers 1850"), None),
        ] {
            let converted = date(value).to_version(V7);
            assert_eq!(converted.value.as_deref(), expected, "{value}");
            assert_eq!(converted.phrase.as_deref(), phrase, "{value}");
        }
        // An own PHRASE wins over an equal one; two different ones cannot
        // both be kept.
        let both = with_phrase(Some("(flood)"), "flood").to_version(V7);
        assert_eq!((both.value, both.phrase.as_deref()), (None, Some("flood")));
        let clash = with_phrase(Some("(flood)"), "storm").to_version(V7);
        assert_eq!(clash.value.as_deref(), Some("(flood)"));
    }

    #[test]
    fn test_to_gedcom_551() {
        for (value, expected) in [
            ("2 OCT 2019", "2 OCT 2019"),
            ("JULIAN 15 MAR 1582", "@#DJULIAN@ 15 MAR 1582"),
            (
                "BET GREGORIAN 20 BCE AND JULIAN 12 BCE",
                "BET 20 B.C. AND @#DJULIAN@ 12 B.C.",
            ),
            ("_ROMAN 1860", "@#DROMAN@ 1860"),
            ("2 oct 2019", "2 oct 2019"),
            ("< 1900", "< 1900"),
            ("_MYCAL 12 _MON 1500", "_MYCAL 12 _MON 1500"),
        ] {
            assert_eq!(
                date(value).to_version(V551).value.as_deref(),
                Some(expected),
                "{value}"
            );
        }
        let interpreted = with_phrase(Some("15 MAR 1820"), "The Ides of March").to_version(V551);
        assert_eq!(
            interpreted.value.as_deref(),
            Some("INT 15 MAR 1820 (The Ides of March)")
        );
        assert_eq!(interpreted.phrase, None);
        let phrase_only = with_phrase(None, "the year of the flood").to_version(V551);
        assert_eq!(
            phrase_only.value.as_deref(),
            Some("(the year of the flood)")
        );
        let range = with_phrase(Some("BET 1820 AND 1825"), "in his youth").to_version(V551);
        assert_eq!(range.value.as_deref(), Some("BET 1820 AND 1825"));
        assert_eq!(range.phrase.as_deref(), Some("in his youth"));
    }

    #[test]
    fn test_time_per_version() {
        let date = Date {
            value: Some("1 DEC 2023".into()),
            time: Some("2:50:00.00Z".into()),
            phrase: None,
        };
        assert_eq!(date.to_version(V7).time.as_deref(), Some("2:50:00.00Z"));
        assert_eq!(date.to_version(V551).time.as_deref(), Some("02:50:00.00"));
        let noon = Date {
            time: Some("noon".into()),
            ..Date::default()
        };
        assert_eq!(noon.to_version(V7).time.as_deref(), Some("noon"));
        assert_eq!(noon.time_value(), None);
    }

    #[test]
    fn test_normalize() {
        for (input, expected) in [
            ("15 mar 1820", "15 MAR 1820"),
            ("@#djulian@ 15 mar 1582", "@#DJULIAN@ 15 MAR 1582"),
            ("15  MAR   1820", "15 MAR 1820"),
            ("abt vers 1850", "ABT vers 1850"),
        ] {
            assert_eq!(
                date(input).normalize(V551).value.as_deref(),
                Some(expected),
                "{input}"
            );
        }
        let date = Date {
            value: Some("15 mar 1820".to_string()),
            time: Some("12:34:56".to_string()),
            phrase: Some("The Ides of March".to_string()),
        };
        let normalized = date.normalize(V7);
        assert_eq!(normalized.value.as_deref(), Some("15 MAR 1820"));
        assert_eq!(normalized.time.as_deref(), Some("12:34:56"));
        assert_eq!(normalized.phrase.as_deref(), Some("The Ides of March"));
        assert_eq!(Date::default().normalize(V7), Date::default());
        assert_eq!(Date::default().date_value(), DateValue::Empty);
    }

    #[cfg(feature = "calendar")]
    #[test]
    fn test_convert_to() {
        use super::{Calendar, CalendarError};
        let julian = date("ABT @#DJULIAN@ 15 MAR 1582");
        let converted = julian.convert_to(&Calendar::Gregorian, V7).unwrap();
        assert_eq!(converted.value.as_deref(), Some("ABT 25 MAR 1582"));
        let back = converted.convert_to(&Calendar::Julian, V551).unwrap();
        assert_eq!(back.value.as_deref(), Some("ABT @#DJULIAN@ 15 MAR 1582"));
        assert_eq!(
            date("1582").convert_to(&Calendar::Julian, V7),
            Err(CalendarError::Incomplete)
        );
    }
}
