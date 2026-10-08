//! No input may make any entry point panic or hang: every reader (bytes,
//! text, the deprecated `Gedcom`, streaming, GEDZIP), version detection,
//! reference validation, the writer in both versions followed by a re-read,
//! JSON, `Debug`/`Display`, the indexed view, and the date, age and time
//! grammars with their conversions between versions and calendars.
#![no_main]

use ged_io::types::age::{Age, AgeValue};
use ged_io::types::date::{Calendar, Date, DateExact, DatePeriod, DateValue, Time};
use ged_io::{GedcomBuilder, GedcomVersion, GedcomWriter};
use libfuzzer_sys::fuzz_target;

/// Every grammar, strict and lenient, every conversion, on one payload.
fn values(text: &str) {
    let date = Date {
        value: Some(text.to_string()),
        time: Some(text.to_string()),
        phrase: None,
    };
    let age = Age {
        value: Some(text.to_string()),
        phrase: None,
    };
    let value = DateValue::parse(text);
    for version in [GedcomVersion::V5_5_1, GedcomVersion::V7_0] {
        let _ = DateValue::parse_strict(text, version.clone());
        let _ = DatePeriod::parse_strict(text, version.clone());
        let _ = DateExact::parse_strict(text, version.clone());
        let _ = Time::parse_strict(text, version.clone());
        let _ = AgeValue::parse_strict(text, version.clone());
        let _ = value.to_gedcom(version.clone());
        let _ = date.to_version(version.clone()).normalize(version.clone());
        let _ = age.to_version(version.clone());
        for calendar in [
            Calendar::Gregorian,
            Calendar::Julian,
            Calendar::Hebrew,
            Calendar::FrenchRepublican,
        ] {
            let _ = date.convert_to(&calendar, version.clone());
        }
    }
    for date in value.dates() {
        let _ = date.ordering_key();
        let _ = date.weekday();
        let _ = date.add_days(1_000_000);
    }
    let _ = Time::parse(text);
    let _ = AgeValue::parse(text);
}

fuzz_target!(|data: &[u8]| {
    if let Ok(d) = GedcomBuilder::new().build_from_bytes(data) {
        for version in ["5.5.1", "7.0"] {
            if let Ok(out) = GedcomWriter::new()
                .gedcom_version(ged_io::GedcomVersion::from_version_str(version))
                .write_to_string(&d)
            {
                let _ = GedcomBuilder::new().build_from_str(&out);
            }
        }
        let _ = format!("{d:?}");
        let _ = format!("{d}");
        let _ = serde_json::to_string(&d);
        let indexed = ged_io::indexed::IndexedGedcomData::from(d);
        let _ = indexed.find_individual("@I1@");
    }
    let _ = GedcomBuilder::new()
        .validate_references(true)
        .build_from_bytes(data);
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = GedcomBuilder::new().build_from_str(text);
        let _ = ged_io::detect_version(text);
        // Each line's payload, after its level and tag, and the whole text.
        for line in text.lines().take(64) {
            values(line.splitn(3, ' ').nth(2).unwrap_or(line));
        }
        values(text);
        if let Ok(mut g) = ged_io::Gedcom::new(text.chars()) {
            let _ = g.parse_data();
        }
    }
    if let Ok(parser) = ged_io::GedcomStreamParser::new(std::io::Cursor::new(data)) {
        for record in parser.take(10_000) {
            let _ = record;
        }
    }
    let _ = ged_io::gedzip::read_gedzip(data);
});
