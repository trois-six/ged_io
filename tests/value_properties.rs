//! Bounded-input properties of every date, age and time parser and
//! conversion: none panics or runs away on any input, the strict parses
//! agree with the lenient ones, formatting reads back, and a conversion
//! either conforms to its target version or leaves the value as written.
//!
//! The inputs are generated from a fixed seed by a small xorshift generator,
//! so every run checks the same cases and a failure is reproducible.

use ged_io::types::age::{Age, AgeValue};
use ged_io::types::date::{CalendarDate, Date, DateExact, DatePeriod, DateValue, Time};
use ged_io::GedcomVersion;

const V551: GedcomVersion = GedcomVersion::V5_5_1;
const V7: GedcomVersion = GedcomVersion::V7_0;

/// Inputs generated per property.
const CASES: usize = 20_000;

/// The longest input generated, in words.
const MAX_WORDS: usize = 8;

/// A xorshift64* generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next() % bound as u64).unwrap()
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

/// Words that date and age payloads are made of, and their usual
/// corruptions.
const WORDS: &[&str] = &[
    "ABT",
    "abt",
    "CAL",
    "EST",
    "BEF",
    "AFT",
    "BET",
    "AND",
    "and",
    "FROM",
    "TO",
    "to",
    "INT",
    "(",
    ")",
    "(phrase)",
    "(a (b) c)",
    "@#DGREGORIAN@",
    "@#DJULIAN@",
    "@#DHEBREW@",
    "@#DFRENCH R@",
    "@#DROMAN@",
    "@#DUNKNOWN@",
    "@#DMYCAL@",
    "@#D",
    "@#Djulian@",
    "@@",
    "@",
    "GREGORIAN",
    "JULIAN",
    "HEBREW",
    "FRENCH_R",
    "ROMAN",
    "_ROMAN",
    "_MYCAL",
    "_mon",
    "_MON",
    "JAN",
    "jan",
    "FEB",
    "DEC",
    "June",
    "VEND",
    "COMP",
    "TSH",
    "ADR",
    "ADS",
    "ELL",
    "0",
    "1",
    "01",
    "29",
    "30",
    "31",
    "36",
    "40",
    "255",
    "256",
    "1699",
    "1699/00",
    "1701/99",
    "1401/8",
    "5/0000",
    "1/",
    "/1",
    "7/11/1959",
    "20B.C.",
    "B.C.",
    "BCE",
    "bc",
    "B.C.E.",
    "1900",
    "2000",
    "4294967295",
    "4294967296",
    "99999999999999999999",
    "6000000",
    "1000000",
    "1000001",
    "<",
    ">",
    "<8",
    "> 99",
    "y",
    "m",
    "w",
    "d",
    "1y",
    "2M",
    "3w",
    "4D",
    "1y6m",
    "400d",
    "0Y",
    "CHILD",
    "infant",
    "Stillborn",
    "12:30",
    "2:5",
    "24:00",
    "23:59:59.999",
    "1:00Z",
    "z",
    ":",
    ".",
    "é",
    "\u{0}",
    "\t",
    "",
    " ",
];

/// A random payload of up to [`MAX_WORDS`] words with random separators.
fn payload(rng: &mut Rng) -> String {
    let mut out = String::new();
    for _ in 0..rng.below(MAX_WORDS + 1) {
        out.push_str(rng.pick(&["", " ", " ", " ", "  ", "\t"]));
        out.push_str(rng.pick(WORDS));
    }
    if rng.below(4) == 0 {
        out.push(' ');
    }
    out
}

#[test]
fn test_date_values() {
    let mut rng = Rng(0x0DA7_E5EE_D000_0001);
    for _ in 0..CASES {
        let text = payload(&mut rng);
        let lenient = DateValue::parse(&text);
        for version in [V551, V7] {
            // Strict parsing is lenient parsing that may refuse.
            if let Ok(strict) = DateValue::parse_strict(&text, version) {
                assert_eq!(strict, lenient, "{text:?}");
            }
            let _ = DatePeriod::parse_strict(&text, version);
            let _ = DateExact::parse_strict(&text, version);
            let _ = Time::parse_strict(&text, version);
        }
        let _ = DatePeriod::parse(&text);
        let _ = DateExact::parse(&text);
        let _ = Time::parse(&text);
        let _ = CalendarDate::parse(&text);

        // What is read whole writes back to itself in 5.5.1, whose grammar
        // can say everything the model holds.
        if !lenient.has_unrecognised_text() && lenient != DateValue::Empty {
            let written = lenient.to_gedcom(V551);
            assert_eq!(
                DateValue::parse(&written),
                lenient,
                "{text:?} → {written:?}"
            );
        }
        let _ = lenient.to_gedcom(V7);
    }
}

#[test]
fn test_date_conversions_conform_or_keep_the_value() {
    let mut rng = Rng(0x0DA7_E5EE_D000_0002);
    for _ in 0..CASES {
        let date = Date {
            value: (rng.below(5) > 0).then(|| payload(&mut rng)),
            time: (rng.below(3) == 0).then(|| payload(&mut rng)),
            phrase: (rng.below(3) == 0).then(|| rng.pick(WORDS).to_string()),
        };
        for version in [V551, V7] {
            let converted = date.to_version(version);
            let value = converted.value.as_deref().unwrap_or_default();
            let conforms = DateValue::parse_strict(value, version).is_ok();
            assert!(
                conforms || converted.value == date.value,
                "{date:?} → {converted:?}"
            );
            // Converting again changes nothing.
            assert_eq!(converted.to_version(version), converted, "{date:?}");
            let _ = date.normalize(version);
            let _ = date.datetime();
        }
    }
}

#[test]
fn test_ages() {
    let mut rng = Rng(0x0A6E_5EED_0000_0003);
    for _ in 0..CASES {
        let text = payload(&mut rng);
        let lenient = AgeValue::parse(&text);
        for version in [V551, V7] {
            if let Ok(strict) = AgeValue::parse_strict(&text, version) {
                assert_eq!(strict, lenient, "{text:?}");
            }
            let age = Age {
                value: Some(text.clone()),
                phrase: (rng.below(3) == 0).then(|| rng.pick(WORDS).to_string()),
            };
            let converted = age.to_version(version);
            let value = converted.value.as_deref().unwrap_or_default();
            assert!(
                AgeValue::parse_strict(value, version).is_ok() || converted == age,
                "{age:?} → {converted:?}"
            );
            assert_eq!(converted.to_version(version), converted, "{age:?}");
        }
        // A duration writes back to itself in 7.0, whose grammar has weeks.
        if let AgeValue::Duration { .. } = lenient {
            let written = lenient.to_gedcom(V7);
            assert_eq!(AgeValue::parse(&written), lenient, "{text:?} → {written:?}");
            assert!(AgeValue::parse_strict(&written, V7).is_ok(), "{written:?}");
        }
        let _ = lenient.to_gedcom(V551);
    }
}

#[test]
fn test_times() {
    let mut rng = Rng(0x7135_EED0_0000_0004);
    for _ in 0..CASES {
        let text = format!(
            "{}:{}{}{}",
            rng.below(30),
            rng.pick(&["0", "5", "59", "60", "07", ""]),
            rng.pick(&["", ":00", ":59", ":60", ":5"]),
            rng.pick(&[
                "",
                ".0",
                ".123456789012345678901234567890",
                "Z",
                ".5Z",
                "z",
                "."
            ]),
        );
        let lenient = Time::parse(&text);
        for version in [V551, V7] {
            if let Ok(strict) = Time::parse_strict(&text, version) {
                assert_eq!(Some(strict), lenient, "{text:?}");
            }
        }
        if let Some(time) = lenient {
            let written = time.to_gedcom(V7);
            assert_eq!(Time::parse(&written), Some(time.clone()), "{text:?}");
            assert!(Time::parse_strict(&written, V7).is_ok(), "{written:?}");
            assert!(Time::parse_strict(&time.to_gedcom(V551), V551).is_ok());
        }
    }
}

#[cfg(feature = "calendar")]
mod calendar {
    use super::*;
    use ged_io::types::date::{Calendar, CalendarError, Epoch, Month};

    const CALENDARS: [Calendar; 6] = [
        Calendar::Gregorian,
        Calendar::Julian,
        Calendar::Hebrew,
        Calendar::FrenchRepublican,
        Calendar::Roman,
        Calendar::Unknown,
    ];

    /// A year anywhere in `u32`, mostly near the calendars' range.
    fn year(rng: &mut Rng) -> u32 {
        match rng.below(4) {
            0 => u32::try_from(rng.next() >> 32).unwrap(),
            1 => u32::try_from(rng.below(7_000_000)).unwrap(),
            _ => u32::try_from(rng.below(6_000)).unwrap(),
        }
    }

    #[test]
    fn test_conversions_terminate_and_round_trip() {
        let mut rng = Rng(0xCA1E_5EED_0000_0005);
        for _ in 0..CASES {
            let calendar = CALENDARS[rng.below(CALENDARS.len())].clone();
            let months = calendar.months();
            let date = CalendarDate {
                day: (rng.below(8) > 0).then(|| u8::try_from(rng.below(40)).unwrap()),
                month: match months.len() {
                    0 => Some(Month::Extension("_M".into())),
                    count => Month::of(&calendar, u8::try_from(rng.below(count + 2)).unwrap()),
                },
                year: Some(year(&mut rng)),
                epoch: (rng.below(4) == 0).then_some(Epoch::Bce),
                calendar,
                ..CalendarDate::default()
            };
            let Ok(rata_die) = date.to_rata_die() else {
                let _ = date.ordering_key();
                continue;
            };
            for target in &CALENDARS[..4] {
                match date.convert_to(target) {
                    Ok(converted) => assert_eq!(converted.to_rata_die(), Ok(rata_die), "{date:?}"),
                    Err(error) => assert_eq!(error, CalendarError::OutOfRange, "{date:?}"),
                }
            }
            let _ = date.weekday();
            let _ = date.add_days(i64::try_from(rng.next() >> 1).unwrap());
            let _ = date.add_days(-i64::try_from(rng.next() >> 1).unwrap());
        }
    }

    #[test]
    fn test_any_day_number_converts_or_is_out_of_range() {
        let mut rng = Rng(0xCA1E_5EED_0000_0006);
        for _ in 0..CASES {
            let rata_die = match rng.below(3) {
                0 => i64::from_ne_bytes(rng.next().to_ne_bytes()),
                1 => i64::try_from(rng.below(900_000_000)).unwrap() - 450_000_000,
                _ => i64::try_from(rng.below(2_000_000)).unwrap() - 500_000,
            };
            for calendar in &CALENDARS {
                match CalendarDate::from_rata_die(rata_die, calendar) {
                    Ok(date) => assert_eq!(date.to_rata_die(), Ok(rata_die), "{rata_die}"),
                    Err(error) => assert!(
                        matches!(
                            error,
                            CalendarError::OutOfRange | CalendarError::UnsupportedCalendar(_)
                        ),
                        "{rata_die}: {error:?}"
                    ),
                }
                let _ = CalendarDate::from_julian_day_number(rata_die, calendar);
            }
        }
    }

    #[test]
    fn test_payload_conversions_terminate() {
        let mut rng = Rng(0xCA1E_5EED_0000_0007);
        for _ in 0..CASES {
            let value = DateValue::parse(&payload(&mut rng));
            for calendar in &CALENDARS {
                let _ = value.convert_to(calendar);
            }
            let date = Date {
                value: Some(payload(&mut rng)),
                ..Date::default()
            };
            let _ = date.convert_to(&Calendar::Hebrew, V7);
        }
    }

    #[test]
    fn test_hebrew_hang_is_gone() {
        // 2 OCT 6000000 used to loop forever in a release build.
        let date = CalendarDate::parse("2 OCT 6000000");
        assert_eq!(
            date.convert_to(&Calendar::Hebrew),
            Err(CalendarError::OutOfRange)
        );
        let edge = CalendarDate::parse("31 DEC 990000");
        let hebrew = edge.convert_to(&Calendar::Hebrew).unwrap();
        assert_eq!(hebrew.to_rata_die(), edge.to_rata_die());
    }
}
