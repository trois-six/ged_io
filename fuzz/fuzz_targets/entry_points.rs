//! No input may make any entry point panic or hang: every reader (text,
//! bytes, strict mode, a reader, streaming, GEDZIP), version detection,
//! reference queries, the writer in every version followed by a re-read,
//! `Debug`, JSON (read back with the same structures), the indexed view,
//! navigation, and the date, age and time
//! grammars with their conversions between versions and calendars.
#![no_main]

use ged_io::model::{Age, Dataset, Date};
use ged_io::value::{AgeValue, Calendar, DateExact, DatePeriod, DateValue, Time};
use ged_io::{GedcomBuilder, GedcomVersion, GedcomWriter, IndexedDataset};
use libfuzzer_sys::fuzz_target;

const VERSIONS: [GedcomVersion; 3] = [
    GedcomVersion::V5_5_1,
    GedcomVersion::V7_0,
    GedcomVersion::V7_1,
];

/// Every grammar, strict and lenient, every conversion, on one payload.
fn values(text: &str) {
    let store = Dataset::default();
    let mut date = Date::new(text);
    date.detail_mut().time = Some(text.into());
    let age = Age::new(text);
    let value = DateValue::parse(text);
    for version in VERSIONS {
        let _ = DateValue::parse_strict(text, version);
        let _ = DatePeriod::parse_strict(text, version);
        let _ = DateExact::parse_strict(text, version);
        let _ = Time::parse_strict(text, version);
        let _ = AgeValue::parse_strict(text, version);
        let _ = value.to_gedcom(version);
        let _ = date.to_version(&store, version).normalize(&store, version);
        let _ = date.datetime(&store);
        let _ = age.to_version(&store, version);
        for calendar in [
            Calendar::Gregorian,
            Calendar::Julian,
            Calendar::Hebrew,
            Calendar::FrenchRepublican,
        ] {
            let _ = date.convert_to(&store, &calendar, version);
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

/// The dataset-level queries, plain and indexed.
fn queries(data: Dataset) {
    let _ = format!("{data:?}");
    if let Ok(json) = serde_json::to_string(&data) {
        let back: Dataset = serde_json::from_str(&json).expect("a dataset's JSON reads back");
        assert!(
            back.to_structures() == data.to_structures(),
            "JSON round trip"
        );
    }
    let _ = data.dangling_references();
    let _ = data.search_individuals("a").count();
    for indi in data.individuals.iter().take(16) {
        let _ = indi.full_name(&data);
        let _ = data.families_as_spouse(indi.xref).count();
        let _ = data.families_as_child(indi.xref).count();
    }
    for family in data.families.iter().take(16) {
        let _ = data.parents(family).count();
        let _ = data.children(family).count();
    }
    let indexed = IndexedDataset::new(data);
    let _ = indexed.find("@I1@");
    for indi in indexed.individuals.iter().take(16) {
        let _ = indexed.families_as_spouse(indi.xref).count();
        let _ = indexed.families_as_child(indi.xref).count();
    }
}

fuzz_target!(|data: &[u8]| {
    let d = Dataset::from_bytes(data);
    for version in VERSIONS {
        if let Ok(out) = GedcomWriter::new()
            .gedcom_version(version)
            .write_to_string(&d)
        {
            let _ = GedcomBuilder::new().strict(true).build_from_str(out);
        }
    }
    queries(d);
    let _ = GedcomBuilder::new().strict(true).build_from_bytes(data);
    let _ = GedcomBuilder::new()
        .max_file_size(64)
        .build_from_reader(data);
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = GedcomBuilder::new().build_from_str(text);
        let _ = ged_io::version::detect_version(text);
        // Each line's payload, after its level and tag, and the whole text.
        for line in text.lines().take(64) {
            values(line.splitn(3, ' ').nth(2).unwrap_or(line));
        }
        values(text);
    }
    if let Ok(parser) = ged_io::GedcomStreamParser::new(data) {
        let streamed: Dataset = parser.take(10_000).filter_map(Result::ok).collect();
        let _ = streamed.dangling_references();
    }
    if let Ok(parser) = ged_io::GedcomStreamParser::new(data) {
        let _ = parser.nodes().take(10_000).count();
    }
    let _ = ged_io::gedzip::read_gedzip(data);
});
