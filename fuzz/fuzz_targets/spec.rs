//! The validator and the conformance repair are total and agree: for any
//! bytes, validation never panics, and for every version the repaired tree
//! validates (but for 5.5.1 records over 32K that hold no note to move) and
//! a second repair changes nothing.
#![no_main]

use ged_io::spec::{conform, validate, validate_bytes, DeviationKind};
use ged_io::tree::Tree;
use ged_io::GedcomVersion;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = validate_bytes(data);
    let records = Tree::from_bytes(data).to_structures();
    for version in [
        GedcomVersion::V5_5_1,
        GedcomVersion::V7_0,
        GedcomVersion::V7_1,
    ] {
        let mut t = records.clone();
        let _ = validate(&t, version);
        conform(&mut t, version);
        let left: Vec<_> = validate(&t, version)
            .into_iter()
            .filter(|d| d.kind != DeviationKind::RecordSize)
            .collect();
        assert!(left.is_empty(), "{version}: {left:?}");
        assert!(conform(&mut t, version).is_empty(), "{version}");
    }
});
