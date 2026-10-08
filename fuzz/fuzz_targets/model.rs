//! The typed model under construction (`ged_io::next`) is total and its
//! output is stable: for any bytes, reading never panics; for every
//! version, the written output reads back with every structure the model
//! types read typed (`ledger::untyped`), and writing what was read back
//! gives the same text.
#![no_main]

use ged_io::next::{ledger, read_bytes, read_str, write_string};
use ged_io::{GedcomVersion, GedcomWriter};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let dataset = read_bytes(data.to_vec());
    let _ = format!("{dataset:?}");
    let _ = dataset.to_structures();
    for version in [
        GedcomVersion::V5_5_1,
        GedcomVersion::V7_0,
        GedcomVersion::V7_1,
    ] {
        let writer = GedcomWriter::new().gedcom_version(version);
        let Ok(out) = write_string(&dataset, &writer) else {
            continue;
        };
        let again = read_str(&out);
        let untyped = ledger::untyped(&again);
        assert!(untyped.is_empty(), "{version}: {untyped:?}");
        let twice = write_string(&again, &writer).expect("a written dataset writes");
        let canonical = |text: &str| {
            let mut records = ged_io::tree::parse_tree(text).to_structures();
            fn sort(s: &mut ged_io::tree::Structure) {
                s.substructures.iter_mut().for_each(sort);
                s.substructures
                    .sort_by(|a, b| a.tag.as_str().cmp(b.tag.as_str()));
            }
            records.iter_mut().for_each(sort);
            records
        };
        assert!(
            canonical(&twice) == canonical(&out),
            "{version}: not stable"
        );
    }
});
