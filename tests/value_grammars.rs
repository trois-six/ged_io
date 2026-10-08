//! The date, age and time grammars of GEDCOM 5.5.1 and 7.0.
//!
//! - Calendars (test-suite plan §c3 item 3): one date per calendar and month,
//!   per epoch, and per date form, valid in both versions, converted from
//!   one to the other, and re-emitted identically by the writer.
//! - Payload types (§c3 item 4): the valid samples of each payload type are
//!   accepted by the strict grammar and re-emitted identically; the invalid
//!   ones are rejected by it, read leniently and kept verbatim.
//!
//! The samples come from the ArmidaleSoftware/gedcom7 validator's tests (MIT)
//! and the gedcom7code/test-files `age-*` and `date-*` files (Unlicense).

use ged_io::types::age::{Age, AgeValue};
use ged_io::types::date::{Calendar, Date, DateExact, DatePeriod, DateValue, Month, Time};
use ged_io::types::GedcomData;
use ged_io::{GedcomBuilder, GedcomVersion, GedcomWriter};

const V551: GedcomVersion = GedcomVersion::V5_5_1;
const V7: GedcomVersion = GedcomVersion::V7_0;

/// A calendar: its 7.0 keyword, its 5.5.1 escape and its months.
struct CalendarCase {
    keyword: &'static str,
    escape: &'static str,
    months: &'static [&'static str],
    bce: bool,
}

const CALENDARS: [CalendarCase; 4] = [
    CalendarCase {
        keyword: "GREGORIAN",
        escape: "@#DGREGORIAN@",
        months: &[
            "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
        ],
        bce: true,
    },
    CalendarCase {
        keyword: "JULIAN",
        escape: "@#DJULIAN@",
        months: &[
            "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
        ],
        bce: true,
    },
    CalendarCase {
        keyword: "FRENCH_R",
        escape: "@#DFRENCH R@",
        months: &[
            "VEND", "BRUM", "FRIM", "NIVO", "PLUV", "VENT", "GERM", "FLOR", "PRAI", "MESS", "THER",
            "FRUC", "COMP",
        ],
        bce: false,
    },
    CalendarCase {
        keyword: "HEBREW",
        escape: "@#DHEBREW@",
        months: &[
            "TSH", "CSH", "KSL", "TVT", "SHV", "ADR", "ADS", "NSN", "IYR", "SVN", "TMZ", "AAV",
            "ELL",
        ],
        bce: false,
    },
];

/// A date in both versions' spellings.
struct Pair {
    v7: String,
    v551: String,
}

/// One date per calendar and month, per epoch, and per date form.
fn calendar_cases() -> Vec<Pair> {
    let mut cases = Vec::new();
    for calendar in &CALENDARS {
        // The Gregorian calendar is the default: no marker in either version.
        let (kw, esc) = if calendar.keyword == "GREGORIAN" {
            (String::new(), String::new())
        } else {
            (
                format!("{} ", calendar.keyword),
                format!("{} ", calendar.escape),
            )
        };
        for (index, month) in calendar.months.iter().enumerate() {
            // Within the five complementary days, the shortest month.
            let day = index % 5 + 1;
            cases.push(Pair {
                v7: format!("{kw}{day} {month} 1801"),
                v551: format!("{esc}{day} {month} 1801"),
            });
        }
        if calendar.bce {
            cases.push(Pair {
                v7: format!("{kw}20 BCE"),
                v551: format!("{esc}20 B.C."),
            });
        }
        for (v7, v551) in [
            ("{d}", "{d}"),
            ("ABT {d}", "ABT {d}"),
            ("CAL {d}", "CAL {d}"),
            ("EST {d}", "EST {d}"),
            ("BEF {d}", "BEF {d}"),
            ("AFT {d}", "AFT {d}"),
            ("BET {d} AND {e}", "BET {d} AND {e}"),
            ("FROM {d}", "FROM {d}"),
            ("TO {e}", "TO {e}"),
            ("FROM {d} TO {e}", "FROM {d} TO {e}"),
        ] {
            let month = calendar.months[1];
            let fill = |form: &str, marker: &str| {
                form.replace("{d}", &format!("{marker}{month} 1801"))
                    .replace("{e}", &format!("{marker}1802"))
            };
            cases.push(Pair {
                v7: fill(v7, &kw),
                v551: fill(v551, &esc),
            });
        }
    }
    cases
}

/// A file of the given version holding one birth per date.
fn file_with_dates(version: &str, dates: &[&str]) -> String {
    let mut file = format!("0 HEAD\n1 GEDC\n2 VERS {version}\n0 @I1@ INDI\n");
    for date in dates {
        file.push_str(&format!("1 BIRT\n2 DATE {date}\n"));
    }
    file.push_str("0 TRLR\n");
    file
}

/// The dates of the births of the first individual.
fn dates_of(data: &GedcomData) -> Vec<Option<String>> {
    data.individuals[0]
        .events
        .iter()
        .map(|event| event.date.as_ref().and_then(|date| date.value.clone()))
        .collect()
}

fn write(data: &GedcomData, version: &str) -> GedcomData {
    let written = GedcomWriter::new()
        .gedcom_version(version)
        .write_to_string(data)
        .unwrap();
    GedcomBuilder::new().build_from_str(&written).unwrap()
}

#[test]
fn test_calendars_months_epochs_and_forms() {
    let cases = calendar_cases();
    assert!(cases.len() >= 60, "{}", cases.len());
    for case in &cases {
        let v7 = DateValue::parse_strict(&case.v7, V7)
            .unwrap_or_else(|error| panic!("{}: {error}", case.v7));
        let v551 = DateValue::parse_strict(&case.v551, V551)
            .unwrap_or_else(|error| panic!("{}: {error}", case.v551));
        // One meaning, two spellings.
        assert_eq!(v7, v551, "{} / {}", case.v7, case.v551);
        assert_eq!(v7.to_gedcom(V7), case.v7);
        assert_eq!(v551.to_gedcom(V551), case.v551);
        // Each version's spelling is invalid in the other, but for the
        // Gregorian dates without an epoch, which both spell alike.
        if case.v7 != case.v551 {
            assert!(
                DateValue::parse_strict(&case.v7, V551).is_err(),
                "{}",
                case.v7
            );
            assert!(
                DateValue::parse_strict(&case.v551, V7).is_err(),
                "{}",
                case.v551
            );
        }
        let date = |value: &str| Date {
            value: Some(value.to_string()),
            ..Date::default()
        };
        assert_eq!(
            date(&case.v7).to_version(V551).value.as_deref(),
            Some(case.v551.as_str())
        );
        assert_eq!(
            date(&case.v551).to_version(V7).value.as_deref(),
            Some(case.v7.as_str())
        );
    }
}

#[test]
fn test_calendar_dates_are_re_emitted_identically() {
    let cases = calendar_cases();
    let v7: Vec<&str> = cases.iter().map(|case| case.v7.as_str()).collect();
    let v551: Vec<&str> = cases.iter().map(|case| case.v551.as_str()).collect();
    let expected = |dates: &[&str]| -> Vec<Option<String>> {
        dates.iter().map(|d| Some((*d).to_string())).collect()
    };

    let data7 = GedcomBuilder::new()
        .build_from_str(&file_with_dates("7.0", &v7))
        .unwrap();
    assert_eq!(dates_of(&data7), expected(&v7));
    assert_eq!(dates_of(&write(&data7, "7.0")), expected(&v7));
    assert_eq!(dates_of(&write(&data7, "5.5.1")), expected(&v551));

    let data551 = GedcomBuilder::new()
        .build_from_str(&file_with_dates("5.5.1", &v551))
        .unwrap();
    assert_eq!(dates_of(&write(&data551, "5.5.1")), expected(&v551));
    assert_eq!(dates_of(&write(&data551, "7.0")), expected(&v7));
}

#[cfg(feature = "calendar")]
#[test]
fn test_every_calendar_month_converts_and_back() {
    use ged_io::types::date::CalendarDate;
    for case in calendar_cases() {
        let DateValue::Date(date) = DateValue::parse(&case.v7) else {
            continue;
        };
        if date.day.is_none() {
            continue;
        }
        let rata_die = date
            .to_rata_die()
            .unwrap_or_else(|error| panic!("{}: {error}", case.v7));
        for calendar in [
            Calendar::Gregorian,
            Calendar::Julian,
            Calendar::Hebrew,
            Calendar::FrenchRepublican,
        ] {
            match date.convert_to(&calendar) {
                Ok(converted) => assert_eq!(converted.to_rata_die(), Ok(rata_die), "{}", case.v7),
                // 1801 is before the French Republican calendar for the
                // other calendars' early days: only that may fail.
                Err(error) => assert_eq!(
                    (calendar, error),
                    (
                        Calendar::FrenchRepublican,
                        ged_io::types::date::CalendarError::OutOfRange
                    ),
                    "{}",
                    case.v7
                ),
            }
        }
        assert_eq!(
            CalendarDate::from_rata_die(rata_die, &date.calendar)
                .unwrap()
                .to_rata_die(),
            Ok(rata_die)
        );
    }
}

/// Valid `DateValue` samples in every version (Armidale).
const DATE_VALUES: &[&str] = &[
    "3 DEC 2023",
    "DEC 2023",
    "2023",
    "TO 3 DEC 2023",
    "TO DEC 2023",
    "TO 2023",
    "FROM 03 DEC 2023",
    "FROM 2000 TO 2020",
    "FROM MAR 2000 TO JUN 2000",
    "FROM 30 NOV 2000 TO 1 DEC 2000",
    "BEF 3 DEC 2023",
    "BEF DEC 2023",
    "BEF 2023",
    "AFT 03 DEC 2023",
    "BET 2000 AND 2020",
    "BET MAR 2000 AND JUN 2000",
    "BET 30 NOV 2000 AND 1 DEC 2000",
    "ABT 3 DEC 2023",
    "CAL DEC 2023",
];

/// Invalid `DateValue` samples in every version (Armidale). The validator
/// also rejects lower-case months in 5.5.1; the 5.5.1 specification makes
/// controlled values case-insensitive (p. 21), so they are valid here.
const INVALID_DATE_VALUES: &[&str] = &[
    "TO 40 DEC 2023",
    "TO 3 JUNE 2023",
    "TO ABC 2023",
    "BEF 40 DEC 2023",
    "BEF 3 JUNE 2023",
    "BEF ABC 2023",
    "BET 2000",
];

/// Valid 5.5.1 dates (Armidale; test-files `5/date-dual-valid.ged`).
const DATE_VALUES_551: &[&str] = &[
    "1740/41",
    "@#DGREGORIAN@ 1740/41",
    "@#DGREGORIAN@ 20 B.C.",
    "@#DHEBREW@ 1 TSH 1",
    "TO @#DGREGORIAN@ 20 B.C.",
    "FROM @#DHEBREW@ 1 TSH 1",
    "FROM @#DGREGORIAN@ 20 B.C. TO @#DGREGORIAN@ 12 B.C.",
    "BEF @#DGREGORIAN@ 20 B.C.",
    "AFT @#DHEBREW@ 1 TSH 1",
    "BET @#DGREGORIAN@ 20 B.C. AND @#DGREGORIAN@ 12 B.C.",
    "EST @#DGREGORIAN@ 20 B.C.",
    "1699/00",
    "JAN 1699/00",
    "8 JAN 1699/00",
    "ABT 8 JAN 1699/00",
    "FROM JAN 1699/00 TO FEB 1699/00",
    "BET JAN 1699/00 AND FEB 1699/00",
    "INT 1 JAN 1900 (New Year's day)",
    "(the year of the flood)",
    "@#DROMAN@ 1860",
    "@#DUNKNOWN@ 1871",
    "TO 3 dec 2023",
];

/// Invalid 5.5.1 dates (Armidale; test-files `5/date-dual-invalid.ged`).
const INVALID_DATE_VALUES_551: &[&str] = &[
    "FROM @#DHEBREW@ 1 TSH 1 B.C.",
    "AFT @#DHEBREW@ 1 TSH 1 B.C.",
    "1701/99",
    "JAN 1701/03",
    "1751/9",
    "GREGORIAN 20 BCE",
    "INT 1900",
    "",
];

/// Valid 7.0 dates (Armidale).
const DATE_VALUES_7: &[&str] = &[
    "GREGORIAN 20 BCE",
    "HEBREW 1 TSH 1",
    "TO GREGORIAN 20 BCE",
    "FROM HEBREW 1 TSH 1",
    "FROM GREGORIAN 20 BCE TO GREGORIAN 12 BCE",
    "BEF GREGORIAN 20 BCE",
    "AFT HEBREW 1 TSH 1",
    "BET GREGORIAN 20 BCE AND GREGORIAN 12 BCE",
    "EST GREGORIAN 20 BCE",
    "_MYCAL 12 _MON 1500",
    "_ROMAN 28",
    "",
];

/// Invalid 7.0 dates (Armidale and the 7.0 grammar).
const INVALID_DATE_VALUES_7: &[&str] = &[
    "FROM HEBREW 1 TSH 1 BCE",
    "AFT HEBREW 1 TSH 1 BCE",
    "TO 3 dec 2023",
    "BEF 3 dec 2023",
    "1740/41",
    "20 B.C.",
    "@#DJULIAN@ 1700",
    "INT 1 JAN 1900 (New Year's day)",
    "(the year of the flood)",
    "ROMAN 28",
    "1 JAN",
];

#[test]
fn test_date_value_samples() {
    for (version, valid, invalid) in [
        (V551, DATE_VALUES_551, INVALID_DATE_VALUES_551),
        (V7, DATE_VALUES_7, INVALID_DATE_VALUES_7),
    ] {
        for value in DATE_VALUES.iter().chain(valid) {
            let parsed = DateValue::parse_strict(value, version.clone())
                .unwrap_or_else(|error| panic!("{version:?}: {error}"));
            // The strict reading is the lenient one.
            assert_eq!(parsed, DateValue::parse(value), "{value}");
        }
        for value in INVALID_DATE_VALUES.iter().chain(invalid) {
            assert!(
                DateValue::parse_strict(value, version.clone()).is_err(),
                "{version:?}: {value}"
            );
        }
    }
}

#[test]
fn test_date_value_samples_through_the_reader_and_writer() {
    for (version, valid, invalid) in [
        ("5.5.1", DATE_VALUES_551, INVALID_DATE_VALUES_551),
        ("7.0", DATE_VALUES_7, INVALID_DATE_VALUES_7),
    ] {
        let target = GedcomVersion::from_version_str(version);
        let samples: Vec<&str> = DATE_VALUES
            .iter()
            .chain(valid)
            .chain(invalid)
            .copied()
            .filter(|value| !value.is_empty())
            .collect();
        let data = GedcomBuilder::new()
            .build_from_str(&file_with_dates(version, &samples))
            .unwrap();
        // Read verbatim, valid or not.
        let read = dates_of(&data);
        assert_eq!(
            read,
            samples
                .iter()
                .map(|d| Some((*d).to_string()))
                .collect::<Vec<_>>()
        );
        let written = write(&data, version);
        for (index, sample) in samples.iter().enumerate() {
            let event = &written.individuals[0].events[index];
            let date = event.date.as_ref().unwrap();
            if DateValue::parse_strict(sample, target.clone()).is_ok() {
                // Valid: re-emitted identically.
                assert_eq!(date.value.as_deref(), Some(*sample));
            } else {
                // Invalid: what it says is kept, rewritten in the version's
                // grammar when it can be, verbatim otherwise.
                let kept = date.value.as_deref() == Some(*sample)
                    || date.phrase.as_deref() == Some(*sample)
                    || DateValue::parse(date.value.as_deref().unwrap_or_default())
                        == DateValue::parse(sample)
                    || date.to_version(V551).value.as_deref() == Some(*sample);
                assert!(kept, "{version}: {sample} became {date:?}");
            }
        }
    }
}

#[test]
fn test_period_and_exact_samples() {
    for version in [V551, V7] {
        for valid in [
            "TO 3 DEC 2023",
            "TO DEC 2023",
            "TO 2023",
            "FROM 03 DEC 2023",
            "FROM 2000 TO 2020",
            "FROM MAR 2000 TO JUN 2000",
            "FROM 30 NOV 2000 TO 1 DEC 2000",
        ] {
            assert!(
                DatePeriod::parse_strict(valid, version.clone()).is_ok(),
                "{valid}"
            );
        }
        for invalid in [
            "2023",
            "TO 40 DEC 2023",
            "TO 3 JUNE 2023",
            "TO ABC 2023",
            "BEF 2023",
        ] {
            assert!(
                DatePeriod::parse_strict(invalid, version.clone()).is_err(),
                "{invalid}"
            );
        }
        for valid in ["3 DEC 2023", "03 DEC 2023"] {
            assert!(
                DateExact::parse_strict(valid, version.clone()).is_ok(),
                "{valid}"
            );
        }
        for invalid in [
            "invalid",
            "3 JUNE 2023",
            "DEC 2023",
            "2023",
            "ABT 3 DEC 2023",
        ] {
            assert!(
                DateExact::parse_strict(invalid, version.clone()).is_err(),
                "{invalid}"
            );
        }
    }
    assert!(DateExact::parse_strict("3 dec 2023", V7).is_err());
    assert!(DatePeriod::parse_strict("FROM HEBREW 1 TSH 1 BCE", V7).is_err());
    assert!(DatePeriod::parse_strict("FROM @#DHEBREW@ 1 TSH 1 B.C.", V551).is_err());
    assert!(DatePeriod::parse_strict("", V7).is_ok());
    assert!(DatePeriod::parse_strict("", V551).is_err());
}

#[test]
fn test_time_samples() {
    for version in [V551, V7] {
        for valid in ["02:50", "2:50", "12:34:56.789"] {
            assert!(
                Time::parse_strict(valid, version.clone()).is_ok(),
                "{valid}"
            );
        }
        for invalid in [
            " ", "invalid", "000:00", "24:00:00", "2:5", "2:60", "2:00:60",
        ] {
            assert!(
                Time::parse_strict(invalid, version.clone()).is_err(),
                "{invalid}"
            );
            // Kept by the date that holds it.
            let date = Date {
                value: Some("1 DEC 2023".into()),
                time: Some(invalid.into()),
                phrase: None,
            };
            let converted = date.to_version(version.clone());
            let kept = converted.time.as_deref() == Some(invalid)
                || converted.time.as_deref().and_then(Time::parse) == Time::parse(invalid);
            assert!(kept, "{invalid}: {converted:?}");
        }
    }
    assert!(Time::parse_strict("2:50:00.00Z", V7).is_ok());
    assert!(Time::parse_strict("2:50:00.00Z", V551).is_err());
}

/// Valid 7.0 ages (Armidale; test-files `7/age-valid.ged`).
const AGES_7: &[&str] = &[
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
    "< 8y",
    "0y",
    "< 0m",
    "> 0d",
    "99y 11m 30d",
    "1y 400d",
    "8w 30d",
];

/// Invalid 7.0 ages (Armidale; test-files `5/age-*.ged` forms).
const INVALID_AGES_7: &[&str] = &[
    " ",
    "invalid",
    "d",
    "79",
    "1d 1m",
    "<>1y",
    ">79y",
    "<79y 1m 1w 1d",
    "CHILD",
    "0Y",
    "99y11m",
];

/// Valid 5.5.1 ages (test-files `5/age-valid.ged`).
const AGES_551: &[&str] = &[
    "child",
    "CHILD",
    "Child",
    "infant",
    "INFANT",
    "stillborn",
    "STILLBORN",
    "0y",
    "0Y",
    "<0y",
    "<0Y",
    ">0y",
    "0m",
    "0M",
    "<0m",
    ">0M",
    "0d",
    "<0D",
    ">0d",
    "99y",
    ">99y",
    "11m",
    ">11m",
    "99y 11m",
    "30d",
    ">30d",
    "99y 30d",
    "11m 30d",
    "99y 11m 30d",
];

/// Invalid 5.5.1 ages (test-files `5/age-invalid.ged`).
const INVALID_AGES_551: &[&str] = &[
    "0",
    "<8",
    "> 99",
    "< 0y",
    "> 0Y",
    "0 y",
    "<0 y",
    "< 0 Y",
    "< 0m",
    "0 m",
    "< 0d",
    "0 D",
    "< 99y",
    "99y11m",
    ">99y11m",
    "< 99y 11m",
    "11m99y",
    "11m 99y",
    "< 11m 99y",
    "99y30d",
    "30d99y",
    "11m30d",
    "30d 11m",
    "99y11m30d",
    "99y 30d 11m",
    "11m 30d 99y",
    "1w",
    "",
];

#[test]
fn test_age_samples() {
    for (version, valid, invalid) in [
        (V7, AGES_7, INVALID_AGES_7),
        (V551, AGES_551, INVALID_AGES_551),
    ] {
        for value in valid {
            let parsed = AgeValue::parse_strict(value, version.clone())
                .unwrap_or_else(|error| panic!("{version:?}: {error}"));
            assert_eq!(parsed, AgeValue::parse(value), "{value}");
        }
        for value in invalid {
            assert!(
                AgeValue::parse_strict(value, version.clone()).is_err(),
                "{version:?}: {value}"
            );
        }
    }
}

#[test]
fn test_age_conversion_matches_the_test_files() {
    // test-files 5/age-*.ged converted to 7/age-*.ged.
    for (v551, v7, phrase) in [
        ("child", "< 8y", Some("child")),
        ("Infant", "< 1y", Some("Infant")),
        ("STILLBORN", "0y", Some("STILLBORN")),
        ("0Y", "0y", None),
        ("<0y", "< 0y", None),
        (">0D", "> 0d", None),
        ("0", "0y", None),
        ("<8", "< 8y", None),
        ("> 99", "> 99y", None),
        ("< 0 Y", "< 0y", None),
        ("99y11m", "99y 11m", None),
        (">11m99y", "> 99y 11m", None),
        ("< 30d 11m 99y", "< 99y 11m 30d", None),
        ("30d11m99y", "99y 11m 30d", None),
    ] {
        let age = Age {
            value: Some(v551.into()),
            phrase: None,
        };
        let converted = age.to_version(V7);
        assert_eq!(converted.value.as_deref(), Some(v7), "{v551}");
        assert_eq!(converted.phrase.as_deref(), phrase, "{v551}");
        // And back: the duration in 5.5.1's form, or the keyword.
        let back = converted.to_version(V551);
        let back_value = back.value.as_deref().unwrap();
        assert!(
            AgeValue::parse_strict(back_value, V551).is_ok(),
            "{back_value}"
        );
        if let Some(phrase) = phrase {
            assert_eq!(back_value, phrase);
        }
    }
}

#[test]
fn test_ages_are_kept_by_the_model() {
    // TS14: an invalid age and ages whose components overflow a byte are
    // kept as written, and written back in the version's form.
    let file = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 DEAT\n2 AGE 79\n1 BURI\n2 AGE 1y 400d\n1 CREM\n2 AGE 1y 30m 100w 400d\n1 EVEN\n2 AGE about thirty\n0 TRLR\n";
    let data = GedcomBuilder::new().build_from_str(file).unwrap();
    let ages: Vec<Option<String>> = data.individuals[0]
        .events
        .iter()
        .map(|event| event.age.as_ref().and_then(|age| age.value.clone()))
        .collect();
    assert_eq!(
        ages,
        [
            Some("79".to_string()),
            Some("1y 400d".to_string()),
            Some("1y 30m 100w 400d".to_string()),
            Some("about thirty".to_string()),
        ]
    );
    let ages_in = |data: &GedcomData| -> Vec<Option<String>> {
        data.individuals[0]
            .events
            .iter()
            .map(|event| event.age.as_ref().and_then(|age| age.value.clone()))
            .collect()
    };
    let v7 = write(&data, "7.0");
    assert_eq!(
        ages_in(&v7),
        [
            Some("79y".to_string()),
            Some("1y 400d".to_string()),
            Some("1y 30m 100w 400d".to_string()),
            None,
        ]
    );
    // GEDCOM 7.0 keeps the text that is not an age in the PHRASE.
    let text = v7.individuals[0].events[3].age.as_ref().unwrap();
    assert_eq!(text.phrase.as_deref(), Some("about thirty"));
    assert_eq!(
        ages_in(&write(&data, "5.5.1")),
        [
            Some("79y".to_string()),
            Some("1y 400d".to_string()),
            Some("1y 30m 1100d".to_string()),
            Some("about thirty".to_string()),
        ]
    );
}

#[test]
fn test_months_are_tags() {
    for calendar in [
        Calendar::Gregorian,
        Calendar::Julian,
        Calendar::FrenchRepublican,
        Calendar::Hebrew,
    ] {
        for (index, month) in calendar.months().iter().enumerate() {
            assert_eq!(Month::from_tag(month.tag()).as_ref(), Some(month));
            assert_eq!(usize::from(month.number().unwrap()), index + 1);
            assert_eq!(
                Month::of(&calendar, month.number().unwrap()).as_ref(),
                Some(month)
            );
        }
    }
}
