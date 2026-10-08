//! A date holding words the date model does not recognise keeps its
//! wording, so that parsing and formatting a date never drops part of it.
//!
//! Carried over from upstream PR #109, onto the date grammar of both
//! versions: a date's qualifier now belongs to the date value, and each date
//! of a range or period keeps its own wording.

use ged_io::types::date::{Approximation, Calendar, CalendarDate, Date, DateValue};
use ged_io::GedcomVersion;

const V551: GedcomVersion = GedcomVersion::V5_5_1;
const V7: GedcomVersion = GedcomVersion::V7_0;

#[test]
fn test_unrecognised_words_of_a_bound_are_kept() {
    // Free text where a date is expected, as some programs write it.
    let date = CalendarDate::parse("vers 1850");
    assert_eq!(date.text.as_deref(), Some("vers 1850"));
    assert_eq!(date.year, None);
    assert_eq!(date.to_gedcom(V551), "vers 1850");

    // What is recognised is still read; the wording stays whole.
    let value = DateValue::parse("ABT @#DJULIAN@ 1850 environ");
    let DateValue::Approximated(Approximation::About, date) = &value else {
        panic!("not an approximated date: {value:?}");
    };
    assert_eq!(date.calendar, Calendar::Julian);
    assert_eq!(date.year, Some(1850));
    assert_eq!(date.text.as_deref(), Some("1850 environ"));
    assert_eq!(value.to_gedcom(V551), "ABT @#DJULIAN@ 1850 environ");
    assert_eq!(value.to_gedcom(V7), "ABT JULIAN 1850 environ");

    // A date read whole has no text of its own.
    let value = DateValue::parse("ABT 2 jan 1850");
    assert!(!value.has_unrecognised_text());
    assert_eq!(value.to_gedcom(V551), "ABT 2 JAN 1850");
}

#[test]
fn test_date_value_keeps_the_wording_of_each_bound() {
    for (value, written) in [
        ("vers 1850", "vers 1850"),
        ("BET vers 1850 AND 1860", "BET vers 1850 AND 1860"),
        ("FROM 1850 TO later", "FROM 1850 TO later"),
        ("BEF @#DJULIAN@ 1700 or so", "BEF @#DJULIAN@ 1700 or so"),
        ("INT 1850? (about 1850)", "INT 1850? (about 1850)"),
    ] {
        let parsed = DateValue::parse(value);
        assert_eq!(parsed.to_gedcom(V551), written, "{parsed:?}");
    }

    let DateValue::Between(start, end) = DateValue::parse("BET vers 1850 AND 1860") else {
        panic!("not a range");
    };
    assert_eq!(start.text.as_deref(), Some("vers 1850"));
    assert_eq!(end.year, Some(1860));
    assert_eq!(end.text, None);
}

#[test]
fn test_normalizing_a_date_keeps_unrecognised_words() {
    let date = Date {
        value: Some("abt vers 1850".to_string()),
        time: None,
        phrase: None,
    };
    assert_eq!(date.normalize(V551).value.as_deref(), Some("ABT vers 1850"));
    // Converting to another version cannot express the words: the date is
    // left as written.
    assert_eq!(date.to_version(V7), date);
}

#[cfg(feature = "calendar")]
#[test]
fn test_a_bound_with_unrecognised_words_is_not_converted() {
    // Converting would write the recognised day, month and year in the
    // other calendar and drop the rest of the wording.
    let date = CalendarDate::parse("@#DJULIAN@ 1 JAN 1700 old style");
    assert!(date.convert_to(&Calendar::Gregorian).is_err());
    let value = DateValue::parse("BET @#DJULIAN@ 1 JAN 1700 old style AND 1710");
    assert!(value.convert_to(&Calendar::Gregorian).is_err());
}
