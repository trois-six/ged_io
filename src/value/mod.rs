//! The payloads of dates, ages and times, read and written by the grammars
//! of GEDCOM 5.5.1 and 7.x.
//!
//! The model keeps every payload as written (a [`Date`](crate::model::Date)'s
//! value, an [`Age`](crate::model::Age)'s), so that wording no grammar
//! understands is never lost; these types interpret it on demand:
//!
//! - [`DateValue`] (the `DATE` payload: a date, an approximated date, a
//!   range or a period, each date in its [`Calendar`]; 5.5.1 interpreted
//!   dates and date phrases), [`DateExact`] and [`DatePeriod`];
//! - [`Time`];
//! - [`AgeValue`].
//!
//! Every payload type offers two parses:
//!
//! - a **lenient** one, `parse(text)`, that never fails and keeps any
//!   wording it does not understand verbatim, so that reading a file never
//!   loses data;
//! - a **strict** one, `parse_strict(text, version)`, that accepts exactly
//!   the grammar of one GEDCOM version and says why it rejects anything
//!   else.
//!
//! Both share a single parser: the strict parse is the lenient one with a
//! checker that records the first deviation from the version's grammar.
//! `to_gedcom(version)` writes a value in a version's grammar. With the
//! `calendar` feature, dates convert between calendars and compare
//! chronologically.
//!
//! ```rust
//! use ged_io::value::{Approximation, Calendar, DateValue};
//! use ged_io::GedcomVersion;
//!
//! let DateValue::Approximated(Approximation::About, date) = DateValue::parse("ABT @#DJULIAN@ 1700")
//! else {
//!     panic!()
//! };
//! assert_eq!((date.calendar.clone(), date.year), (Calendar::Julian, Some(1700)));
//! assert_eq!(
//!     DateValue::parse("ABT @#DJULIAN@ 1700").to_gedcom(GedcomVersion::V7_0),
//!     "ABT JULIAN 1700"
//! );
//! ```

mod age;
mod calendar;
#[cfg(feature = "calendar")]
mod conversion;
mod date;
mod time;

pub use age::{AgeBound, AgeDuration, AgeKeyword, AgeValue};
pub use calendar::{Calendar, Epoch, Month};
#[cfg(feature = "calendar")]
pub use conversion::{CalendarError, Weekday, MAX_RATA_DIE, MAX_YEAR};
pub use date::{Approximation, CalendarDate, DateExact, DatePeriod, DateValue};
pub use time::Time;

pub(crate) use age::AgeTexts;
pub(crate) use date::DateTexts;

use std::fmt;

use crate::GedcomVersion;

/// Why a payload does not follow the grammar of a GEDCOM version.
///
/// Returned by the `parse_strict` functions of the payload types, such as
/// [`DateValue::parse_strict`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueError {
    /// The payload type that was expected, as the specifications name it
    /// (`DateValue`, `DateExact`, `DatePeriod`, `Time` or `Age`).
    pub expected: &'static str,
    /// The payload that was rejected.
    pub text: String,
    /// What in the payload breaks the grammar.
    pub reason: String,
}

impl fmt::Display for ValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} is not a valid {}: {}",
            self.text, self.expected, self.reason
        )
    }
}

impl std::error::Error for ValueError {}

/// The grammar a payload is checked against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Grammar {
    /// GEDCOM 5.5.1 (and the earlier 5.x versions read as 5.5.1).
    V551,
    /// GEDCOM 7.0.
    V7,
}

impl From<GedcomVersion> for Grammar {
    /// The grammar of `version`: 7.x is 7.0, anything else is 5.5.1.
    fn from(version: GedcomVersion) -> Grammar {
        if version.is_v7() {
            Grammar::V7
        } else {
            Grammar::V551
        }
    }
}

/// Records the first deviation from a grammar while a payload is parsed.
///
/// With no grammar, the parse is lenient and nothing is recorded; the reason
/// closures are then never called, so leniency costs nothing.
pub(crate) struct Checker {
    grammar: Option<Grammar>,
    violation: Option<String>,
}

impl Checker {
    /// A checker for a lenient parse.
    pub(crate) fn lenient() -> Checker {
        Checker {
            grammar: None,
            violation: None,
        }
    }

    /// A checker for a strict parse in `grammar`.
    pub(crate) fn strict(grammar: Grammar) -> Checker {
        Checker {
            grammar: Some(grammar),
            violation: None,
        }
    }

    /// The grammar checked, if any.
    pub(crate) fn grammar(&self) -> Option<Grammar> {
        self.grammar
    }

    /// Whether the GEDCOM 7.0 grammar is checked.
    pub(crate) fn is_v7(&self) -> bool {
        self.grammar == Some(Grammar::V7)
    }

    /// Whether the GEDCOM 5.5.1 grammar is checked.
    pub(crate) fn is_v551(&self) -> bool {
        self.grammar == Some(Grammar::V551)
    }

    /// Records a deviation, unless one is already recorded or the parse is
    /// lenient.
    pub(crate) fn fail(&mut self, reason: impl FnOnce() -> String) {
        if self.grammar.is_some() && self.violation.is_none() {
            self.violation = Some(reason());
        }
    }

    /// Records a deviation when `condition` holds.
    pub(crate) fn require(&mut self, condition: bool, reason: impl FnOnce() -> String) {
        if !condition {
            self.fail(reason);
        }
    }

    /// The parse's result: `value`, or the first deviation recorded.
    pub(crate) fn finish<T>(
        self,
        expected: &'static str,
        text: &str,
        value: T,
    ) -> Result<T, ValueError> {
        match self.violation {
            None => Ok(value),
            Some(reason) => Err(ValueError {
                expected,
                text: text.to_string(),
                reason,
            }),
        }
    }
}

/// A word of a payload, with the byte range it spans in it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Word<'a> {
    pub(crate) text: &'a str,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

/// The longest text read between `@#` and `@` as one calendar escape.
const MAX_ESCAPE_NAME: usize = 24;

/// Splits `text` into words separated by whitespace.
///
/// A GEDCOM 5.5.1 calendar escape is one word even when it holds a space
/// (`@#DFRENCH R@`): it runs from `@#` to the next `@`.
pub(crate) fn words(text: &str) -> Vec<Word<'_>> {
    let mut words = Vec::new();
    let mut rest = text.char_indices().peekable();
    while let Some(&(start, c)) = rest.peek() {
        if c.is_whitespace() {
            rest.next();
            continue;
        }
        // Escape names are short (`@#DGREGORIAN@` is the longest standard
        // one): a far `@` belongs to something else.
        let escape_end = text[start..]
            .starts_with("@#")
            .then(|| text[start + 2..].find('@'))
            .flatten()
            .filter(|&close| close <= MAX_ESCAPE_NAME)
            .map(|close| start + 2 + close + 1);
        let end = escape_end.unwrap_or_else(|| {
            text[start..]
                .find(char::is_whitespace)
                .map_or(text.len(), |at| start + at)
        });
        words.push(Word {
            text: &text[start..end],
            start,
            end,
        });
        while rest.peek().is_some_and(|&(at, _)| at < end) {
            rest.next();
        }
    }
    words
}

/// Whether `text` has no leading or trailing whitespace and its words are
/// separated by exactly one space (U+0020), as both grammars require.
pub(crate) fn single_spaced(text: &str, words: &[Word<'_>]) -> bool {
    let Some((first, last)) = words.first().zip(words.last()) else {
        return text.is_empty();
    };
    first.start == 0
        && last.end == text.len()
        && words
            .windows(2)
            .all(|pair| &text[pair[0].end..pair[1].start] == " ")
}

/// The text from the first to the last word of `words`, as written.
pub(crate) fn span<'a>(text: &'a str, words: &[Word<'_>]) -> &'a str {
    match (words.first(), words.last()) {
        (Some(first), Some(last)) => &text[first.start..last.end],
        _ => "",
    }
}

/// Whether `word` is a GEDCOM `Integer`: one or more ASCII digits.
pub(crate) fn is_integer(word: &str) -> bool {
    !word.is_empty() && word.bytes().all(|b| b.is_ascii_digit())
}

/// Whether `word` is a GEDCOM 7.0 extension tag: `_` followed by one or more
/// upper-case letters, digits or underscores.
pub(crate) fn is_ext_tag(word: &str) -> bool {
    word.len() > 1
        && word.starts_with('_')
        && word[1..]
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// Whether `word` reads as an extension tag leniently: `_` followed by
/// letters, digits or underscores of any case.
pub(crate) fn is_lenient_ext_tag(word: &str) -> bool {
    word.len() > 1
        && word.starts_with('_')
        && word[1..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(text: &str) -> Vec<&str> {
        words(text).iter().map(|w| w.text).collect()
    }

    #[test]
    fn test_words_keep_calendar_escapes_whole() {
        assert_eq!(
            texts("BET @#DFRENCH R@ 1 VEND 2 AND 3"),
            ["BET", "@#DFRENCH R@", "1", "VEND", "2", "AND", "3"]
        );
        assert_eq!(texts("  a\tb  "), ["a", "b"]);
        assert_eq!(texts("@#DJULIAN 1700"), ["@#DJULIAN", "1700"]);
        assert_eq!(texts("@#DFRENCH R@"), ["@#DFRENCH R@"]);
        assert_eq!(texts("é  ü"), ["é", "ü"]);
        assert!(texts("").is_empty());
    }

    #[test]
    fn test_single_spacing() {
        let check = |text: &str| single_spaced(text, &words(text));
        assert!(check("1 JAN 1900"));
        assert!(check(""));
        assert!(!check("1  JAN 1900"));
        assert!(!check(" 1 JAN 1900"));
        assert!(!check("1 JAN\t1900"));
        assert!(!check("1900 "));
    }

    #[test]
    fn test_tags_and_integers() {
        assert!(is_integer("0123"));
        assert!(!is_integer("12a"));
        assert!(!is_integer(""));
        assert!(is_ext_tag("_MY_CAL2"));
        assert!(!is_ext_tag("_my"));
        assert!(!is_ext_tag("_"));
        assert!(is_lenient_ext_tag("_my"));
    }
}
