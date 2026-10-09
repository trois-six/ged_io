//! The typed model under construction (`ged_io::next`) is total and its
//! output is stable: for any bytes, reading never panics; for every
//! version, the written output reads back with every structure the model
//! types read typed (`ledger::untyped`), and writing what was read back
//! gives the same bytes: write(read(write(x))) == write(x).
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
        // Writing is idempotent: what was written, read back and written
        // again, is the same bytes.
        let twice = write_string(&again, &writer).expect("a written dataset writes");
        assert!(twice == out, "{version}: not stable\n{out}\n{twice}");
    }
});
