//! The `TIME` payload: GEDCOM 7.0's `Time` and 5.5.1's `TIME_VALUE`.

use std::fmt::Write as _;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{Checker, Grammar, ValueError};
use crate::GedcomVersion;

/// A time of day on a 24-hour clock: `hh:mm[:ss[.fraction]][Z]`.
///
/// ```
/// use ged_io::value::Time;
/// use ged_io::GedcomVersion;
///
/// let time = Time::parse("2:50:00.25Z").unwrap();
/// assert_eq!((time.hour, time.minute, time.second), (2, 50, Some(0)));
/// assert_eq!(time.fraction.as_deref(), Some("25"));
/// assert!(time.utc);
/// assert_eq!(time.to_gedcom(GedcomVersion::V7_0), "02:50:00.25Z");
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Time {
    /// The hour, 0 to 23.
    pub hour: u8,
    /// The minute, 0 to 59.
    pub minute: u8,
    /// The second, 0 to 59.
    pub second: Option<u8>,
    /// The decimal fraction of the second, as its digits were written
    /// (`"25"` for `.25`), so that no precision is lost.
    pub fraction: Option<String>,
    /// Whether the time is in Coordinated Universal Time (`Z`, GEDCOM 7.0);
    /// otherwise it is local to the event.
    pub utc: bool,
}

impl Time {
    /// Reads a time leniently: surrounding whitespace, one-digit minutes and
    /// seconds and a lower-case `z` are accepted. `None` when the payload is
    /// not a time; the raw payload is kept by its holder.
    #[must_use]
    pub fn parse(text: &str) -> Option<Time> {
        parse_time(text, &mut Checker::lenient())
    }

    /// Reads a time that follows the grammar of `version` exactly. GEDCOM
    /// 5.5.1 has no `Z`.
    ///
    /// # Errors
    ///
    /// Returns a [`ValueError`] saying why the payload is not a valid
    /// `Time` (7.0) or `TIME_VALUE` (5.5.1).
    pub fn parse_strict(text: &str, version: GedcomVersion) -> Result<Time, ValueError> {
        Time::strict(text, Grammar::from(version))
    }

    /// [`parse_strict`](Self::parse_strict) in `grammar`.
    pub(crate) fn strict(text: &str, grammar: Grammar) -> Result<Time, ValueError> {
        let mut ck = Checker::strict(grammar);
        let time = parse_time(text, &mut ck);
        if time.is_none() {
            ck.fail(|| "not a time".to_string());
        }
        ck.finish("Time", text, time.unwrap_or_default())
    }

    /// The time in the grammar of `version`, with two-digit fields.
    ///
    /// GEDCOM 5.5.1 cannot say that a time is in UTC: the `Z` is left out.
    #[must_use]
    pub fn to_gedcom(&self, version: GedcomVersion) -> String {
        self.format(Grammar::from(version))
    }

    /// [`to_gedcom`](Self::to_gedcom) in `grammar`.
    pub(crate) fn format(&self, grammar: Grammar) -> String {
        let mut out = format!("{:02}:{:02}", self.hour, self.minute);
        if let Some(second) = self.second {
            let _ = write!(out, ":{second:02}");
            if let Some(fraction) = &self.fraction {
                let _ = write!(out, ".{fraction}");
            }
        }
        if self.utc && grammar == Grammar::V7 {
            out.push('Z');
        }
        out
    }
}

/// Reads a time, checking it against the checker's grammar.
fn parse_time(text: &str, ck: &mut Checker) -> Option<Time> {
    let trimmed = text.trim();
    ck.require(trimmed.len() == text.len(), || {
        "leading or trailing whitespace".to_string()
    });
    let (clock, utc) = match trimmed.strip_suffix(['Z', 'z']) {
        Some(clock) => {
            ck.require(trimmed.ends_with('Z'), || "UTC is written Z".to_string());
            ck.require(!ck.is_v551(), || {
                "GEDCOM 5.5.1 has no UTC marker".to_string()
            });
            (clock, true)
        }
        None => (trimmed, false),
    };
    let (clock, fraction) = match clock.split_once('.') {
        Some((clock, fraction)) => {
            if fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            (clock, Some(fraction.to_string()))
        }
        None => (clock, None),
    };
    let mut fields = clock.split(':');
    let hour = field(fields.next()?, 23, ck, false)?;
    let minute = field(fields.next()?, 59, ck, true)?;
    let second = match fields.next() {
        Some(second) => Some(field(second, 59, ck, true)?),
        None => None,
    };
    if fields.next().is_some() || (fraction.is_some() && second.is_none()) {
        return None;
    }
    Some(Time {
        hour,
        minute,
        second,
        fraction,
        utc,
    })
}

/// Reads a field of one or two digits up to `max`; `two_digits` fields must
/// have exactly two in a strict parse.
fn field(text: &str, max: u8, ck: &mut Checker, two_digits: bool) -> Option<u8> {
    if text.is_empty() || text.len() > 2 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if two_digits {
        ck.require(text.len() == 2, || format!("{text:?} must have two digits"));
    }
    text.parse().ok().filter(|value| *value <= max)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V551: GedcomVersion = GedcomVersion::V5_5_1;
    const V7: GedcomVersion = GedcomVersion::V7_0;

    #[test]
    fn test_lenient_times() {
        let time = Time::parse("12:34:56.789").unwrap();
        assert_eq!((time.hour, time.minute, time.second), (12, 34, Some(56)));
        assert_eq!(time.fraction.as_deref(), Some("789"));
        assert!(!time.utc);
        assert_eq!(Time::parse(" 0:00:00 ").map(|t| t.hour), Some(0));
        assert_eq!(Time::parse("2:5").map(|t| t.minute), Some(5));
        assert!(Time::parse("2:50z").unwrap().utc);
        for invalid in [
            "",
            " ",
            "noon",
            "24:00",
            "2:60",
            "2:00:60",
            "000:00",
            "1:2:3:4",
            "1:00.5",
            "1:00:00.",
            "1:00:00.x",
        ] {
            assert_eq!(Time::parse(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn test_strict_times() {
        // The Armidale validator's samples.
        for valid in ["02:50", "2:50", "2:50:00.00Z"] {
            assert!(Time::parse_strict(valid, V7).is_ok(), "{valid}");
        }
        for invalid in [
            " ", "invalid", "000:00", "24:00:00", "2:5", "2:60", "2:00:60",
        ] {
            assert!(Time::parse_strict(invalid, V7).is_err(), "{invalid}");
        }
        assert!(Time::parse_strict("0:00:00", V551).is_ok());
        assert!(Time::parse_strict("12:34:56.789", V551).is_ok());
        assert!(Time::parse_strict("12:34Z", V551).is_err());
        assert!(Time::parse_strict("12:34z", V7).is_err());
    }

    #[test]
    fn test_formatting_per_version() {
        let time = Time::parse("2:50:00.00Z").unwrap();
        assert_eq!(time.to_gedcom(V7), "02:50:00.00Z");
        assert_eq!(time.to_gedcom(V551), "02:50:00.00");
        assert_eq!(Time::parse("12:34").unwrap().to_gedcom(V7), "12:34");
    }
}
