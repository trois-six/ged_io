//! The typed model (`ged_io::next`) writes its records where they are,
//! one at a time: what it writes is what writing its owned structures
//! writes. Every name and value is fictitious.

use ged_io::next::write_string;
use ged_io::{GedcomVersion, GedcomWriter};

/// Every `.ged` file under `dir`.
fn ged_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            ged_files(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ged")) {
            out.push(p);
        }
    }
}

/// The writer reads the typed model's records where they are, one at a
/// time, without copying the dataset: what it writes is what writing the
/// model's owned structures writes, line for line and repair for repair,
/// in every version.
#[test]
fn the_model_writes_as_its_owned_structures() {
    let mut files = Vec::new();
    ged_files(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        &mut files,
    );
    files.sort();
    assert!(files.len() > 20, "{}", files.len());
    for path in files {
        let data = ged_io::next::read_bytes(std::fs::read(&path).unwrap());
        for version in [
            GedcomVersion::V5_5_1,
            GedcomVersion::V7_0,
            GedcomVersion::V7_1,
        ] {
            let writer = GedcomWriter::new()
                .gedcom_version(version)
                .bom(ged_io::Bom::Never);
            let mut direct = Vec::new();
            let report = ged_io::next::write(&data, &writer, &mut direct).unwrap();
            let mut owned = Vec::new();
            let expected = writer
                .write_structures(&mut owned, &data.to_structures_for(version))
                .unwrap();
            assert_eq!(
                String::from_utf8_lossy(&direct),
                String::from_utf8_lossy(&owned),
                "{} as {version}",
                path.display()
            );
            assert_eq!(
                report.repairs,
                expected.repairs,
                "{} as {version}",
                path.display()
            );
            assert_eq!(
                write_string(&data, &writer).unwrap().as_bytes(),
                direct.as_slice(),
                "{} as {version}",
                path.display()
            );
        }
    }
}
