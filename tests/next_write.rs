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
            let expected = ged_io::next::write_structures(
                &data.to_structures_for(version),
                &writer,
                &mut owned,
            )
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

/// Writes `data` in `version`, reads the output back and writes it again:
/// the second output must be the first, line for line.
fn assert_idempotent(input: &str, version: GedcomVersion) -> String {
    let writer = GedcomWriter::new().gedcom_version(version);
    let out = write_string(&ged_io::next::read_str(input), &writer).unwrap();
    let again = write_string(&ged_io::next::read_str(&out), &writer).unwrap();
    assert_eq!(again, out, "{version}: written anew from\n{input}");
    out
}

/// Writing the typed model is idempotent on every fixture, in every
/// version: write(read(write(x))) == write(x).
#[test]
fn writing_the_model_is_idempotent() {
    let mut files = Vec::new();
    ged_files(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        &mut files,
    );
    files.sort();
    for path in files {
        let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
        for version in [
            GedcomVersion::V5_5_1,
            GedcomVersion::V7_0,
            GedcomVersion::V7_1,
        ] {
            assert_idempotent(&text, version);
        }
    }
}

/// A 5.5.1 pointer of the substructure or network form (`!`, `:`) is one
/// only when it is an identifier: `@!@` names nothing, and is kept as text
/// rather than renamed into a pointer to nothing (`@X_@`), which the next
/// write would have kept as text in turn (fuzz `model`).
#[test]
fn an_invalid_pointer_is_kept_as_text() {
    let input = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 E @!@\n1 FAMS @!@\n0 TRLR\n";
    let out = assert_idempotent(input, GedcomVersion::V5_5_1);
    assert!(out.contains("1 _E @@!@@\n"), "{out}");
    assert!(out.contains("1 _FAMS @@!@@\n"), "{out}");
    let out = assert_idempotent(input, GedcomVersion::V7_0);
    assert!(out.contains("1 _FAMS @@!@\n"), "{out}");
    // A valid one is kept as it is.
    let input = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 ASSO @I1!2@\n2 RELA self\n0 TRLR\n";
    let out = assert_idempotent(input, GedcomVersion::V5_5_1);
    assert!(out.contains("1 ASSO @I1!2@\n"), "{out}");
}

/// What the repair adds or changes goes where the model writes it once
/// read back: the stub submitter and the header's pointer to it, a required
/// substructure, a structure made an extension, a pointer made text, and
/// `HEAD.SCHMA` beside a value kept aside.
#[test]
fn repairs_follow_the_order_of_the_model() {
    let v551 = GedcomVersion::V5_5_1;
    // The stub submitter comes before the other records, `HEAD.SUBM` after
    // `SOUR` and `DATE`, before an extension of the header.
    let out = assert_idempotent(
        "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n1 _HEAD x\n0 @I1@ INDI\n0 TRLR\n",
        v551,
    );
    assert!(
        out.contains("1 SUBM @U1@\n1 _HEAD x\n0 @U1@ SUBM\n1 NAME Unknown\n0 @I1@ INDI\n"),
        "{out}"
    );
    // A submitter without its name gets one, first; a name that is a
    // pointer, written as text, is a name again.
    for input in [
        "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n1 SUBM @U1@\n0 @U1@ SUBM\n1 NOTE a note\n0 TRLR\n",
        "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n1 SUBM @U1@\n0 @U1@ SUBM\n1 NOTE a note\n1 NAME @I1@\n0 TRLR\n",
    ] {
        let out = assert_idempotent(input, v551);
        assert!(out.contains("0 @U1@ SUBM\n1 NAME "), "{out}");
    }
    // A 5.5.1 relation, which 7.0 has not, goes after the role.
    let out = assert_idempotent(
        "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 ASSO @I2@\n2 RELA friend\n0 @I2@ INDI\n0 TRLR\n",
        GedcomVersion::V7_0,
    );
    assert!(
        out.contains("1 ASSO @I2@\n2 ROLE OTHER\n2 _RELA friend\n"),
        "{out}"
    );
    // A value with an extension substructure is kept aside: `SCHMA`, which
    // a relocation adds, goes before it.
    assert_idempotent(
        "0 HEAD\n1 GEDC\n2 VERS 7.0\n1 LANG en\n2 _X y\n0 @I1@ INDI\n1 BIRT\n2 DATE 1900\n2 DATE 1901\n0 TRLR\n",
        GedcomVersion::V7_0,
    );
}
