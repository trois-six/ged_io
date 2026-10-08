//! No input may make any entry point panic or hang: every reader (bytes,
//! text, the deprecated `Gedcom`, streaming, GEDZIP), version detection,
//! reference validation, the writer in both versions followed by a re-read,
//! JSON, `Debug`/`Display` and the indexed view.
//!
//! The date conversions of the `calendar` feature are left out until their
//! known hang on far-future Hebrew years is fixed; add them then.
#![no_main]

use ged_io::{GedcomBuilder, GedcomWriter};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(d) = GedcomBuilder::new().build_from_bytes(data) {
        for version in ["5.5.1", "7.0"] {
            if let Ok(out) = GedcomWriter::new()
                .gedcom_version(version)
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
