//! The typed model is total and its output conformant and stable: for any
//! bytes, reading never panics; for every version, the written output
//! follows the version's specification (the crate's validator finds
//! nothing), reads back with every structure the model types read typed
//! (`ledger::untyped`), and writing what was read back gives the same
//! structures (records and siblings of different tags in any order):
//! read(write(read(x))) holds what read(x) holds.
#![no_main]

use ged_io::model::{ledger, Dataset};
use ged_io::{GedcomVersion, GedcomWriter};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let dataset = Dataset::from_bytes(data);
    let _ = format!("{dataset:?}");
    let _ = dataset.to_structures();
    for version in [
        GedcomVersion::V5_5_1,
        GedcomVersion::V7_0,
        GedcomVersion::V7_1,
    ] {
        let writer = GedcomWriter::new().gedcom_version(version);
        let Ok(out) = writer.write_to_string(&dataset) else {
            continue;
        };
        let deviations = ged_io::spec::validate_text(&out);
        assert!(deviations.is_empty(), "{version}: {deviations:?}");
        let again = Dataset::parse(out.as_str());
        let untyped = ledger::untyped(&again);
        assert!(untyped.is_empty(), "{version}: {untyped:?}");
        let twice = writer
            .write_to_string(&again)
            .expect("a written dataset writes");
        let canonical = |text: &str| {
            let mut records = ged_io::tree::parse_tree(text).to_structures();
            fn sort(s: &mut ged_io::tree::Structure) {
                s.substructures.iter_mut().for_each(sort);
                s.substructures
                    .sort_by(|a, b| a.tag.as_str().cmp(b.tag.as_str()));
            }
            records.iter_mut().for_each(sort);
            records.sort_by(|a, b| a.tag.as_str().cmp(b.tag.as_str()));
            records
        };
        assert!(
            canonical(&twice) == canonical(&out),
            "{version}: not stable"
        );
    }
});
