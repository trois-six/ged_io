//! Ages: the `AGE` structure and its payload.
//!
//! GEDCOM 7.0's `Age` is a duration of years, months, weeks and days, with an
//! optional bound (`< 8y`, `> 80y`); GEDCOM 5.5.1's `AGE_AT_EVENT` has no
//! weeks, writes the bound against the number (`<8y`), and adds the keywords
//! `CHILD`, `INFANT` and `STILLBORN`, which 7.0 writes as durations with a
//! `PHRASE`.

use std::fmt::Write as _;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{Checker, Grammar, ValueError};
use crate::GedcomVersion;

/// An age payload and its `PHRASE` as texts, which the model's
/// [`Age`](crate::model::Age) converts between versions with.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct AgeTexts {
    /// The payload, as written; `None` when the line has none.
    pub value: Option<String>,
    /// The `PHRASE` substructure (GEDCOM 7.0): the age in the words of the
    /// source.
    pub phrase: Option<String>,
}

/// An age payload, interpreted.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum AgeValue {
    /// No payload, which GEDCOM 7.0 allows with a `PHRASE`.
    #[default]
    Empty,
    /// A duration, possibly bounded: `25y 3m`, `< 8y`, `> 80y`.
    Duration {
        /// `<` or `>`, when the real age is less or greater.
        bound: Option<AgeBound>,
        /// The years, months, weeks and days.
        duration: AgeDuration,
    },
    /// A GEDCOM 5.5.1 keyword: `CHILD`, `INFANT` or `STILLBORN`.
    Keyword(AgeKeyword),
    /// A payload that is not an age, kept as written (without surrounding
    /// whitespace).
    Text(String),
}

/// The bound of an age.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum AgeBound {
    /// `<`: the real age was less than the given age.
    LessThan,
    /// `>`: the real age was greater than the given age (or equal, as ages
    /// are rounded down).
    GreaterThan,
}

/// A duration in years, months, weeks and days, each optional. Odd values
/// such as `8w 30d` or `1y 400d` are valid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct AgeDuration {
    /// Full years (`y`).
    pub years: Option<u32>,
    /// Months (`m`).
    pub months: Option<u32>,
    /// Weeks (`w`); GEDCOM 5.5.1 has none.
    pub weeks: Option<u32>,
    /// Days (`d`).
    pub days: Option<u32>,
}

/// The age keywords of GEDCOM 5.5.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum AgeKeyword {
    /// `CHILD`: younger than 8 years.
    Child,
    /// `INFANT`: younger than 1 year.
    Infant,
    /// `STILLBORN`: died just before, at, or near birth; 0 years.
    Stillborn,
}

impl AgeKeyword {
    /// The keyword as GEDCOM 5.5.1 writes it.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            AgeKeyword::Child => "CHILD",
            AgeKeyword::Infant => "INFANT",
            AgeKeyword::Stillborn => "STILLBORN",
        }
    }

    /// Reads a keyword of any case.
    #[must_use]
    pub fn from_word(word: &str) -> Option<AgeKeyword> {
        [AgeKeyword::Child, AgeKeyword::Infant, AgeKeyword::Stillborn]
            .into_iter()
            .find(|keyword| word.eq_ignore_ascii_case(keyword.tag()))
    }

    /// The bounded duration the keyword stands for, as GEDCOM 7.0 writes it:
    /// `< 8y`, `< 1y` and `0y`.
    #[must_use]
    pub fn duration(self) -> (Option<AgeBound>, AgeDuration) {
        let years = |years| AgeDuration {
            years: Some(years),
            ..AgeDuration::default()
        };
        match self {
            AgeKeyword::Child => (Some(AgeBound::LessThan), years(8)),
            AgeKeyword::Infant => (Some(AgeBound::LessThan), years(1)),
            AgeKeyword::Stillborn => (None, years(0)),
        }
    }
}

impl AgeDuration {
    /// Whether no component is given.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.years.is_none() && self.months.is_none() && self.weeks.is_none() && self.days.is_none()
    }

    /// The same duration without weeks, for GEDCOM 5.5.1: weeks become seven
    /// days each. `None` if the days overflow.
    #[must_use]
    pub fn without_weeks(&self) -> Option<AgeDuration> {
        let Some(weeks) = self.weeks else {
            return Some(*self);
        };
        let days = weeks.checked_mul(7)?.checked_add(self.days.unwrap_or(0))?;
        Some(AgeDuration {
            weeks: None,
            days: Some(days),
            ..*self
        })
    }
}

impl AgeTexts {
    /// The payload, interpreted; an absent payload is [`AgeValue::Empty`].
    #[cfg(test)]
    pub(crate) fn age_value(&self) -> AgeValue {
        self.value
            .as_deref()
            .map_or(AgeValue::Empty, AgeValue::parse)
    }

    /// The age rewritten in the grammar of `version`, keeping everything it
    /// says.
    ///
    /// A payload that already follows the version's grammar is kept as
    /// written. Otherwise:
    ///
    /// - to GEDCOM 7.0, a keyword becomes its duration with the keyword as
    ///   it was written as the `PHRASE` (`CHILD` becomes `< 8y` and `PHRASE
    ///   CHILD`), a duration is written in 7.0's form (`<8Y` becomes
    ///   `< 8y`; a bare number is a number of years), and text that is not
    ///   an age moves to the `PHRASE`, the payload left empty, unless the
    ///   age already has another `PHRASE`;
    /// - to GEDCOM 5.5.1, a duration is written in 5.5.1's form (`<8y`),
    ///   weeks counted as days; a duration with a `PHRASE` that names the
    ///   keyword it stands for becomes that keyword. Any other `PHRASE` stays
    ///   in [`phrase`](Self::phrase), which 5.5.1 cannot write.
    ///
    /// Otherwise a payload that is not an age is left as written.
    #[must_use]
    pub(crate) fn to_version(&self, version: GedcomVersion) -> AgeTexts {
        let unchanged = || self.clone();
        let Some(raw) = self
            .value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        else {
            return unchanged();
        };
        let grammar = Grammar::from(version);
        let valid = AgeValue::strict(raw, grammar).is_ok();
        if valid && (grammar == Grammar::V7 || self.phrase.is_none()) {
            return unchanged();
        }
        let converted = match (AgeValue::parse(raw), grammar) {
            // GEDCOM 7.0 keeps an age it cannot read in the PHRASE, with an
            // empty payload; 5.5.1 has no PHRASE.
            (AgeValue::Text(_), Grammar::V7) => {
                return match self.phrase.as_deref() {
                    None => AgeTexts {
                        value: None,
                        phrase: Some(raw.to_string()),
                    },
                    Some(phrase) if phrase == raw => AgeTexts {
                        value: None,
                        phrase: self.phrase.clone(),
                    },
                    Some(_) => unchanged(),
                };
            }
            (AgeValue::Text(_) | AgeValue::Empty, _) => return unchanged(),
            (AgeValue::Keyword(keyword), Grammar::V7) => {
                let (bound, duration) = keyword.duration();
                AgeTexts {
                    value: Some(format_duration(bound, &duration, Grammar::V7)),
                    phrase: self.phrase.clone().or_else(|| Some(raw.to_string())),
                }
            }
            (AgeValue::Duration { bound, duration }, Grammar::V7) => AgeTexts {
                value: Some(format_duration(bound, &duration, Grammar::V7)),
                phrase: self.phrase.clone(),
            },
            (AgeValue::Keyword(_), Grammar::V551) => AgeTexts {
                value: Some(raw.to_string()),
                phrase: self.phrase.clone(),
            },
            (AgeValue::Duration { bound, duration }, Grammar::V551) => {
                let keyword = self
                    .phrase
                    .as_deref()
                    .and_then(AgeKeyword::from_word)
                    .filter(|keyword| keyword.duration() == (bound, duration));
                match (keyword, duration.without_weeks()) {
                    (Some(_), _) => AgeTexts {
                        value: self.phrase.clone(),
                        phrase: None,
                    },
                    (None, Some(duration)) => AgeTexts {
                        value: Some(format_duration(bound, &duration, Grammar::V551)),
                        phrase: self.phrase.clone(),
                    },
                    (None, None) => return unchanged(),
                }
            }
        };
        match converted.value.as_deref() {
            Some(value) if AgeValue::strict(value, grammar).is_ok() => converted,
            _ => unchanged(),
        }
    }
}

impl AgeValue {
    /// Reads an age payload, never failing.
    ///
    /// Both versions' forms are read, and their common deviations: units and
    /// keywords of any case, a bound with or without a space, components in
    /// any order, with or without spaces between them or between a number
    /// and its unit (`99y11m`, `0 y`), and a number without unit, read as
    /// years. Anything else is [`AgeValue::Text`]; each unit may appear
    /// once.
    #[must_use]
    pub fn parse(text: &str) -> AgeValue {
        parse_age(text, &mut Checker::lenient())
    }

    /// Reads an age payload that follows the grammar of `version` exactly.
    ///
    /// GEDCOM 7.0: `[< |> ]` then `y`, `m`, `w`, `d` components in that
    /// order, lower case, separated by single spaces; or nothing.
    /// GEDCOM 5.5.1: `[<|>]` against the number, `y`, `m`, `d` components of
    /// any case in that order, or `CHILD`, `INFANT`, `STILLBORN`.
    ///
    /// # Errors
    ///
    /// Returns a [`ValueError`] saying why the payload is not a valid `Age`
    /// (7.0) or `AGE_AT_EVENT` (5.5.1).
    pub fn parse_strict(text: &str, version: GedcomVersion) -> Result<AgeValue, ValueError> {
        AgeValue::strict(text, Grammar::from(version))
    }

    /// [`parse_strict`](Self::parse_strict) in `grammar`.
    pub(crate) fn strict(text: &str, grammar: Grammar) -> Result<AgeValue, ValueError> {
        let mut ck = Checker::strict(grammar);
        let value = parse_age(text, &mut ck);
        ck.finish("Age", text, value)
    }

    /// The age in the grammar of `version`.
    ///
    /// In GEDCOM 7.0 a keyword is written as the duration it stands for (its
    /// name belongs in a `PHRASE`, as [`Age::to_version`](crate::model::Age::to_version) does); in GEDCOM
    /// 5.5.1 weeks are written as days. Text is written as it is.
    #[must_use]
    pub fn to_gedcom(&self, version: GedcomVersion) -> String {
        self.format(Grammar::from(version))
    }

    /// [`to_gedcom`](Self::to_gedcom) in `grammar`.
    pub(crate) fn format(&self, grammar: Grammar) -> String {
        match self {
            AgeValue::Empty => String::new(),
            AgeValue::Text(text) => text.clone(),
            AgeValue::Keyword(keyword) if grammar == Grammar::V551 => keyword.tag().to_string(),
            AgeValue::Keyword(keyword) => {
                let (bound, duration) = keyword.duration();
                format_duration(bound, &duration, grammar)
            }
            AgeValue::Duration { bound, duration } => {
                let duration = match grammar {
                    Grammar::V551 => duration.without_weeks().unwrap_or(*duration),
                    Grammar::V7 => *duration,
                };
                format_duration(*bound, &duration, grammar)
            }
        }
    }
}

/// Writes a bounded duration in `grammar`.
fn format_duration(bound: Option<AgeBound>, duration: &AgeDuration, grammar: Grammar) -> String {
    let mut out = String::new();
    match (bound, grammar) {
        (Some(AgeBound::LessThan), Grammar::V7) => out.push_str("< "),
        (Some(AgeBound::GreaterThan), Grammar::V7) => out.push_str("> "),
        (Some(AgeBound::LessThan), Grammar::V551) => out.push('<'),
        (Some(AgeBound::GreaterThan), Grammar::V551) => out.push('>'),
        (None, _) => {}
    }
    let start = out.len();
    for (count, unit) in [
        (duration.years, 'y'),
        (duration.months, 'm'),
        (duration.weeks, 'w'),
        (duration.days, 'd'),
    ] {
        if let Some(count) = count {
            if out.len() > start {
                out.push(' ');
            }
            let _ = write!(out, "{count}{unit}");
        }
    }
    out
}

/// Reads an age, checking it against the checker's grammar.
fn parse_age(text: &str, ck: &mut Checker) -> AgeValue {
    let trimmed = text.trim();
    ck.require(trimmed.len() == text.len(), || {
        "leading or trailing whitespace".to_string()
    });
    if trimmed.is_empty() {
        ck.require(!ck.is_v551(), || {
            "GEDCOM 5.5.1 has no empty age".to_string()
        });
        return AgeValue::Empty;
    }
    if let Some(keyword) = AgeKeyword::from_word(trimmed) {
        ck.require(!ck.is_v7(), || {
            "GEDCOM 7.0 has no age keywords; it uses a duration and PHRASE".to_string()
        });
        return AgeValue::Keyword(keyword);
    }
    if let Some((bound, duration)) = parse_duration(trimmed, ck) {
        AgeValue::Duration { bound, duration }
    } else {
        ck.fail(|| "not an age".to_string());
        AgeValue::Text(trimmed.to_string())
    }
}

/// Reads `[bound] components`; `None` unless the whole text is read.
fn parse_duration(text: &str, ck: &mut Checker) -> Option<(Option<AgeBound>, AgeDuration)> {
    let (bound, rest) = match text.as_bytes()[0] {
        b'<' => (Some(AgeBound::LessThan), &text[1..]),
        b'>' => (Some(AgeBound::GreaterThan), &text[1..]),
        _ => (None, text),
    };
    if bound.is_some() {
        match ck.grammar() {
            Some(Grammar::V7) => ck.require(
                rest.starts_with(' ') && !rest[1..].starts_with(char::is_whitespace),
                || "the bound is followed by one space".to_string(),
            ),
            Some(Grammar::V551) => ck.require(!rest.starts_with(char::is_whitespace), || {
                "the bound is written against the number".to_string()
            }),
            None => {}
        }
    }
    let body = rest.trim_start();

    let mut duration = AgeDuration::default();
    let mut order = Vec::with_capacity(4);
    let mut rest = body;
    while !rest.is_empty() {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        let count: u32 = rest[..digits].parse().ok()?;
        let after = &rest[digits..];
        let unit_at = after.trim_start();
        let (unit, tail) = match unit_at.chars().next() {
            Some(c @ ('y' | 'm' | 'w' | 'd' | 'Y' | 'M' | 'W' | 'D')) => {
                ck.require(unit_at.len() == after.len(), || {
                    "the unit is written against its number".to_string()
                });
                ck.require(!ck.is_v7() || c.is_ascii_lowercase(), || {
                    format!("the unit {c:?} must be lower case")
                });
                (c.to_ascii_lowercase(), &unit_at[1..])
            }
            // A number without unit, first: years.
            _ if order.is_empty() => {
                ck.fail(|| "a number needs its unit".to_string());
                ('y', unit_at)
            }
            _ => return None,
        };
        let slot = match unit {
            'y' => &mut duration.years,
            'm' => &mut duration.months,
            'w' => {
                ck.require(!ck.is_v551(), || "GEDCOM 5.5.1 has no weeks".to_string());
                &mut duration.weeks
            }
            _ => &mut duration.days,
        };
        if slot.replace(count).is_some() {
            return None;
        }
        order.push(unit);
        let next = tail.trim_start();
        if !next.is_empty() {
            ck.require(
                tail.starts_with(' ') && tail.len() == next.len() + 1,
                || "components are separated by one space".to_string(),
            );
        }
        rest = next;
    }
    if duration.is_empty() {
        return None;
    }
    let rank = |unit: &char| "ymwd".find(*unit);
    ck.require(
        order.windows(2).all(|pair| rank(&pair[0]) < rank(&pair[1])),
        || "components are written years, months, weeks, days".to_string(),
    );
    Some((bound, duration))
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::model::Dataset;

    const V551: GedcomVersion = GedcomVersion::V5_5_1;
    const V7: GedcomVersion = GedcomVersion::V7_0;

    fn read_age(age_value: &str) -> AgeTexts {
        let sample = format!(
            "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NAME Test /Person/\n1 DEAT Y\n2 AGE {age_value}\n0 TRLR"
        );
        let data = Dataset::parse(sample);
        data.individuals[0].events[0]
            .detail()
            .age
            .as_ref()
            .unwrap()
            .texts(&data)
    }

    fn duration(
        bound: Option<AgeBound>,
        years: Option<u32>,
        months: Option<u32>,
        weeks: Option<u32>,
        days: Option<u32>,
    ) -> AgeValue {
        AgeValue::Duration {
            bound,
            duration: AgeDuration {
                years,
                months,
                weeks,
                days,
            },
        }
    }

    #[test]
    fn test_payload_is_kept_as_written() {
        for raw in [
            "CHILD",
            "75y 3m",
            "79",
            "1y 400d",
            "majeur",
            "environ 30 ans",
            "<0Y",
        ] {
            assert_eq!(read_age(raw).value.as_deref(), Some(raw));
        }
    }

    #[test]
    fn test_keywords_of_any_case() {
        assert_eq!(
            AgeValue::parse("CHILD"),
            AgeValue::Keyword(AgeKeyword::Child)
        );
        assert_eq!(
            AgeValue::parse("infant"),
            AgeValue::Keyword(AgeKeyword::Infant)
        );
        assert_eq!(
            AgeValue::parse("Stillborn"),
            AgeValue::Keyword(AgeKeyword::Stillborn)
        );
    }

    #[test]
    fn test_durations() {
        assert_eq!(
            AgeValue::parse("75y 3m"),
            duration(None, Some(75), Some(3), None, None)
        );
        assert_eq!(
            AgeValue::parse("25"),
            duration(None, Some(25), None, None, None)
        );
        assert_eq!(
            AgeValue::parse("25 4m 3w 5d"),
            duration(None, Some(25), Some(4), Some(3), Some(5))
        );
        assert_eq!(
            AgeValue::parse("1y6m"),
            duration(None, Some(1), Some(6), None, None)
        );
        assert_eq!(
            AgeValue::parse("0 Y"),
            duration(None, Some(0), None, None, None)
        );
        assert_eq!(
            AgeValue::parse("30d 11m 99y"),
            duration(None, Some(99), Some(11), None, Some(30))
        );
        assert_eq!(
            AgeValue::parse("> 80y"),
            duration(Some(AgeBound::GreaterThan), Some(80), None, None, None)
        );
        assert_eq!(
            AgeValue::parse("<6m"),
            duration(Some(AgeBound::LessThan), None, Some(6), None, None)
        );
        // Components beyond 255, up to u32.
        assert_eq!(
            AgeValue::parse("1y 30m 100w 400d"),
            duration(None, Some(1), Some(30), Some(100), Some(400))
        );
        assert_eq!(
            AgeValue::parse("300m"),
            duration(None, None, Some(300), None, None)
        );
    }

    #[test]
    fn test_free_text() {
        for text in [
            "majeur",
            "environ 30 ans",
            "30y 2y",
            "about 30",
            "ca. 2y",
            "25z",
            "30é",
            "âgé",
            "4294967296y",
            "<",
            "y",
        ] {
            assert_eq!(AgeValue::parse(text), AgeValue::Text(text.into()), "{text}");
        }
        assert_eq!(AgeValue::parse(""), AgeValue::Empty);
        assert_eq!(AgeValue::parse("  "), AgeValue::Empty);
    }

    #[test]
    fn test_strict_gedcom_7() {
        // The Armidale validator's samples.
        for valid in [
            "79y",
            "79y 1d",
            "79y 1w",
            "79y 1w 1d",
            "79y 1m",
            "79y 1m 1d",
            "79y 1m 1w",
            "79y 1m 1w 1d",
            "79m",
            "1m 1d",
            "1m 1w",
            "1m 1w 1d",
            "79w",
            "79w 1d",
            "79d",
            "> 79y",
            "< 79y 1m 1w 1d",
            "",
        ] {
            assert!(AgeValue::parse_strict(valid, V7).is_ok(), "{valid}");
        }
        for invalid in [
            " ",
            "invalid",
            "d",
            "79",
            "1d 1m",
            "<>1y",
            ">79y",
            "<79y 1m 1w 1d",
            "CHILD",
            "1Y",
            "1y  1m",
            "1y1m",
        ] {
            assert!(AgeValue::parse_strict(invalid, V7).is_err(), "{invalid}");
        }
    }

    #[test]
    fn test_strict_gedcom_551() {
        // gedcom7code test-files `5/age-valid.ged` and `age-invalid.ged`.
        for valid in [
            "child",
            "CHILD",
            "Infant",
            "stillborn",
            "0y",
            "0Y",
            "<0y",
            ">0Y",
            "0m",
            "<0M",
            ">0d",
            "99y 11m",
            "99y 30d",
            "11m 30d",
            "99y 11m 30d",
        ] {
            assert!(AgeValue::parse_strict(valid, V551).is_ok(), "{valid}");
        }
        for invalid in [
            "0",
            "<8",
            "> 99",
            "< 0y",
            "0 y",
            "<0 y",
            "< 0 Y",
            "99y11m",
            "11m99y",
            "11m 99y",
            "30d 99y 11m",
            "1w",
            "",
            "<CHILD",
        ] {
            assert!(AgeValue::parse_strict(invalid, V551).is_err(), "{invalid}");
        }
    }

    #[test]
    fn test_to_gedcom_7() {
        let age = |value: &str| AgeTexts {
            value: Some(value.into()),
            phrase: None,
        };
        for (value, expected, phrase) in [
            ("child", "< 8y", Some("child")),
            ("INFANT", "< 1y", Some("INFANT")),
            ("Stillborn", "0y", Some("Stillborn")),
            ("<0Y", "< 0y", None),
            ("0", "0y", None),
            ("> 99", "> 99y", None),
            ("11m99y", "99y 11m", None),
            ("< 30d 11m 99y", "< 99y 11m 30d", None),
            ("1y 400d", "1y 400d", None),
        ] {
            let converted = age(value).to_version(V7);
            assert_eq!(converted.value.as_deref(), Some(expected), "{value}");
            assert_eq!(converted.phrase.as_deref(), phrase, "{value}");
        }
    }

    #[test]
    fn test_text_moves_to_the_gedcom_7_phrase() {
        let text = AgeTexts {
            value: Some("majeur".into()),
            phrase: None,
        };
        let converted = text.to_version(V7);
        assert_eq!(
            (converted.value, converted.phrase.as_deref()),
            (None, Some("majeur"))
        );
        let clash = AgeTexts {
            value: Some("majeur".into()),
            phrase: Some("of full age".into()),
        };
        assert_eq!(clash.to_version(V7), clash);
    }

    #[test]
    fn test_to_gedcom_551() {
        let age = |value: &str, phrase: Option<&str>| AgeTexts {
            value: Some(value.into()),
            phrase: phrase.map(str::to_string),
        };
        for (value, phrase, expected, leftover) in [
            ("< 8y", Some("Child"), "Child", None),
            ("0y", Some("STILLBORN"), "STILLBORN", None),
            ("> 80y", Some("over eighty"), ">80y", Some("over eighty")),
            ("< 1y", None, "<1y", None),
            ("8w 3d", None, "59d", None),
            ("2y 1w", None, "2y 7d", None),
            ("0Y", None, "0Y", None),
            ("majeur", None, "majeur", None),
        ] {
            let converted = age(value, phrase).to_version(V551);
            assert_eq!(converted.value.as_deref(), Some(expected), "{value}");
            assert_eq!(converted.phrase.as_deref(), leftover, "{value}");
        }
        assert_eq!(
            AgeValue::parse("4294967295w").to_gedcom(V551),
            "4294967295w"
        );
    }

    #[test]
    fn test_phrase_is_read() {
        let sample = "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 DEAT Y\n2 AGE 0y\n3 PHRASE STILLBORN\n1 BURI\n2 AGE\n3 PHRASE of full age\n0 TRLR";
        let data = Dataset::parse(sample);
        let events = &data.individuals[0].events;
        let age = |i: usize| {
            let e: &crate::model::Event = &events[i];
            e.detail().age.as_ref().map(|a| a.texts(&data))
        };
        assert_eq!(
            age(0),
            Some(AgeTexts {
                value: Some("0y".into()),
                phrase: Some("STILLBORN".into())
            })
        );
        assert_eq!(
            age(1),
            Some(AgeTexts {
                value: None,
                phrase: Some("of full age".into())
            })
        );
        assert_eq!(age(1).unwrap().age_value(), AgeValue::Empty);
    }
}
