use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

fn write_temp_gedcom(contents: &str) -> PathBuf {
    let mut path = env::temp_dir();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);

    path.push(format!(
        "ged_io_cli_test_{}_{}.ged",
        std::process::id(),
        seq
    ));

    fs::write(&path, contents).expect("write temp gedcom");
    path
}

fn write_temp_gedcom_bytes(contents: &[u8]) -> PathBuf {
    let mut path = env::temp_dir();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    path.push(format!(
        "ged_io_cli_test_{}_{}.ged",
        std::process::id(),
        seq
    ));
    fs::write(&path, contents).expect("write temp gedcom");
    path
}

fn run_cli(args: &[&str]) -> std::process::Output {
    let exe = env!("CARGO_BIN_EXE_ged_io");
    Command::new(exe)
        .args(args)
        .output()
        .expect("run ged_io binary")
}

/// A conformant 5.5.1 file.
const VALID_551: &str = "0 HEAD\n1 SOUR EXAMPLE\n1 SUBM @U1@\n1 GEDC\n2 VERS 5.5.1\n2 FORM LINEAGE-LINKED\n1 CHAR UTF-8\n0 @U1@ SUBM\n1 NAME Example Submitter\n0 TRLR\n";

#[test]
fn validate_a_conformant_file() {
    let path = write_temp_gedcom(VALID_551);
    for level in ["lenient", "strict"] {
        let output = run_cli(&[
            "--validate",
            "--validation-level",
            level,
            path.to_str().unwrap(),
        ]);
        assert!(output.status.success(), "{level}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains(&format!("Validation: {level} - errors: 0, warnings: 0")));
        assert!(!stdout.contains("GEDCOM Data Stats"));
    }
}

#[test]
fn validate_lenient_reports_warnings() {
    let sample = VALID_551.replace("0 TRLR", "0 @I1@ INDI\n1 SEX male\n1 FAMC @F9@\n0 TRLR");
    let path = write_temp_gedcom(&sample);

    let output = run_cli(&["--validate", path.to_str().unwrap()]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Validation: lenient - errors: 0, warnings: 2"),
        "{stdout}"
    );
    assert!(
        stdout.contains("warning: line 11: SEX \"male\""),
        "{stdout}"
    );
    assert!(stdout.contains("warning: line 12: FAMC @F9@"), "{stdout}");
}

#[test]
fn validate_strict_reports_errors() {
    let sample = "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @F1@ FAM\n1 HUSB @I999@\n0 TRLR\n";
    let path = write_temp_gedcom(sample);

    let output = run_cli(&[
        "--validate",
        "--validation-level",
        "strict",
        path.to_str().unwrap(),
    ]);

    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Validation: strict - errors: 1, warnings: 0"),
        "{stdout}"
    );
    assert!(stdout.contains("error: line 5: HUSB @I999@: no record has this identifier"));
}

#[test]
fn validate_reads_the_encoding_and_the_line_grammar() {
    // CR line endings and a Windows-1252 byte under CHAR ANSEL: the bytes
    // are read, and the label is reported.
    let bytes = b"0 HEAD\r1 SOUR EXAMPLE\r1 SUBM @U1@\r1 GEDC\r2 VERS 5.5.1\r2 FORM LINEAGE-LINKED\r1 CHAR UTF-8\r0 @U1@ SUBM\r1 NAME Caf\xe9\r0 TRLR\r";
    let path = write_temp_gedcom_bytes(bytes);
    let output = run_cli(&[
        "--validate",
        "--validation-level",
        "strict",
        path.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("HEAD.CHAR says UTF-8"), "{stdout}");
}

#[test]
fn validation_level_requires_validate_flag() {
    let sample = "0 HEAD\n1 GEDC\n2 VERS 5.5\n0 TRLR";
    let path = write_temp_gedcom(sample);

    let output = run_cli(&["--validation-level", "strict", path.to_str().unwrap()]);

    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("requires --validate"));
}
