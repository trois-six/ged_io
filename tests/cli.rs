//! The `ged-io` binary: the summary of a file, one individual, the name
//! filters, validation (lenient and strict), rewriting in another version,
//! and usage errors (exit code 3). Every name and place is fictitious.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use ged_io::{GedcomBuilder, GedcomVersion};

static SEQ: AtomicUsize = AtomicUsize::new(0);

/// A file in the temporary directory, removed when dropped.
struct TempFile(PathBuf);

impl TempFile {
    fn new(contents: impl AsRef<[u8]>) -> Self {
        let path = std::env::temp_dir().join(format!(
            "ged_io_cli_test_{}_{}.ged",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, contents).expect("write a temporary file");
        TempFile(path)
    }

    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ged-io"))
        .args(args)
        .output()
        .expect("run the ged-io binary")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A conformant 5.5.1 file.
const VALID_551: &str = "0 HEAD\n1 SOUR EXAMPLE\n1 SUBM @U1@\n1 GEDC\n2 VERS 5.5.1\n\
                         2 FORM LINEAGE-LINKED\n1 CHAR UTF-8\n0 @U1@ SUBM\n\
                         1 NAME Example Submitter\n0 TRLR\n";

/// A small 5.5.1 family: two parents and a child.
const FAMILY: &str = "0 HEAD\n1 SOUR EXAMPLE\n1 SUBM @U1@\n1 GEDC\n2 VERS 5.5.1\n\
                      2 FORM LINEAGE-LINKED\n1 CHAR UTF-8\n0 @U1@ SUBM\n1 NAME Example Submitter\n\
                      0 @I1@ INDI\n1 NAME Ann /Example/\n1 SEX F\n1 BIRT\n2 DATE 1 JAN 1900\n\
                      2 PLAC Sampleton\n1 FAMS @F1@\n\
                      0 @I2@ INDI\n1 NAME Bob /Sample/\n1 SEX M\n1 DEAT\n2 DATE 1950\n1 FAMS @F1@\n\
                      0 @I3@ INDI\n1 NAME Cleo /Sample/\n1 SEX F\n1 FAMC @F1@\n\
                      0 @F1@ FAM\n1 HUSB @I2@\n1 WIFE @I1@\n1 CHIL @I3@\n\
                      0 @N1@ NOTE A shared note\n0 TRLR\n";

// Inspecting a file

#[test]
fn summary_counts_the_records() {
    let file = TempFile::new(FAMILY);
    let output = run(&[file.path()]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.starts_with("GEDCOM 5.5.1"), "{text}");
    for line in [
        "  individuals: 3\n",
        "  families: 1\n",
        "  shared notes: 1\n",
        "  submitters: 1\n",
        "  sources: 0\n",
    ] {
        assert!(text.contains(line), "{line:?} in\n{text}");
    }
    assert!(!text.contains("pointers to no record"), "{text}");

    let dangling = TempFile::new(FAMILY.replace("1 CHIL @I3@", "1 CHIL @I9@"));
    let text = stdout(&run(&[dangling.path()]));
    assert!(text.contains("  pointers to no record: 1\n"), "{text}");
}

#[test]
fn one_individual() {
    let file = TempFile::new(FAMILY);
    let output = run(&["--individual", "@I1@", file.path()]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert_eq!(
        text,
        "@I1@ Ann Example\n  sex: female\n  born: 1 JAN 1900, Sampleton\n\
         \x20 partner: Bob Sample\n  child: Cleo Sample\n"
    );

    let text = stdout(&run(&["--individual", "@I3@", file.path()]));
    assert!(
        text.contains("  child of: Bob Sample and Ann Example\n"),
        "{text}"
    );

    // No such individual: a usage error.
    let output = run(&["--individual", "@I9@", file.path()]);
    assert_eq!(output.status.code(), Some(3));
    assert!(stderr(&output).contains("no individual @I9@"));
}

#[test]
fn name_filters() {
    let file = TempFile::new(FAMILY);
    let names = |args: &[&str]| -> Vec<String> {
        let mut all = args.to_vec();
        all.push(file.path());
        let output = run(&all);
        assert!(output.status.success(), "{}", stderr(&output));
        stdout(&output)
            .lines()
            .filter(|line| line.starts_with('@'))
            .map(str::to_string)
            .collect()
    };
    // Case-insensitive, on the surname or on the given names.
    assert_eq!(
        names(&["--individual-lastname", "sample"]),
        ["@I2@ Bob Sample", "@I3@ Cleo Sample"]
    );
    assert_eq!(
        names(&["--individual-firstname", "ANN"]),
        ["@I1@ Ann Example"]
    );
    // Both: the individuals matching both.
    assert_eq!(
        names(&[
            "--individual-lastname",
            "Sample",
            "--individual-firstname",
            "cleo"
        ]),
        ["@I3@ Cleo Sample"]
    );
    assert!(names(&["--individual-lastname", "Nobody"]).is_empty());
}

// Rewriting

#[test]
fn write_7_0_reads_back_and_validates() {
    let file = TempFile::new(FAMILY.replace("2 DATE 1950", "2 DATE @#DJULIAN@ 1950"));
    let output = run(&["--write", "7.0", file.path()]);
    assert!(output.status.success(), "{}", stderr(&output));
    let written = stdout(&output);
    assert!(written.starts_with("\u{feff}0 HEAD\n1 GEDC\n2 VERS 7.0\n"));
    assert!(written.contains("2 DATE JULIAN 1950\n"), "{written}");
    assert!(
        written.contains("0 @N1@ SNOTE A shared note\n"),
        "{written}"
    );

    let data = GedcomBuilder::new()
        .strict(true)
        .build_from_str(written.as_str())
        .unwrap_or_else(|e| panic!("{e}\n{written}"));
    assert_eq!(data.version(), GedcomVersion::V7_0);
    assert_eq!(data.individuals.len(), 3);
    assert_eq!(data.families.len(), 1);
    assert_eq!(data.notes.len(), 1);

    let rewritten = TempFile::new(written.as_bytes());
    let output = run(&[
        "--validate",
        "--validation-level",
        "strict",
        rewritten.path(),
    ]);
    assert!(output.status.success(), "{}", stdout(&output));
    assert!(stdout(&output).contains("Validation: strict - errors: 0, warnings: 0"));
}

#[test]
fn write_lists_the_repairs_on_standard_error() {
    let file = TempFile::new(FAMILY.replace("1 SEX F\n1 FAMC", "1 SEX female\n1 FAMC"));
    let output = run(&["--write", "7.0", file.path()]);
    assert!(output.status.success(), "{}", stderr(&output));
    let repairs = stderr(&output);
    assert!(
        repairs.lines().all(|l| l.starts_with("repair: ")),
        "{repairs}"
    );
    assert!(repairs.contains("SEX"), "{repairs}");
    // The value 7.0 does not permit is kept as an extension.
    let written = stdout(&output);
    assert!(written.contains("1 _SEX female\n"), "{written}");
}

// Validation

#[test]
fn validate_a_conformant_file() {
    let file = TempFile::new(VALID_551);
    for level in ["lenient", "strict"] {
        let output = run(&["--validate", "--validation-level", level, file.path()]);
        assert!(output.status.success(), "{level}");
        let text = stdout(&output);
        assert!(text.contains(&format!("Validation: {level} - errors: 0, warnings: 0")));
        assert!(!text.contains("individuals:"), "{text}");
    }
}

#[test]
fn validate_lenient_reports_warnings() {
    let file =
        TempFile::new(VALID_551.replace("0 TRLR", "0 @I1@ INDI\n1 SEX male\n1 FAMC @F9@\n0 TRLR"));
    let output = run(&["--validate", file.path()]);
    assert!(output.status.success());
    let text = stdout(&output);
    assert!(
        text.contains("Validation: lenient - errors: 0, warnings: 2"),
        "{text}"
    );
    assert!(text.contains("warning: line 11: SEX \"male\""), "{text}");
    assert!(text.contains("warning: line 12: FAMC @F9@"), "{text}");
}

#[test]
fn validate_strict_reports_errors() {
    let file = TempFile::new("0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @F1@ FAM\n1 HUSB @I999@\n0 TRLR\n");
    let output = run(&["--validate", "--validation-level", "strict", file.path()]);
    assert_eq!(output.status.code(), Some(2));
    let text = stdout(&output);
    assert!(
        text.contains("Validation: strict - errors: 1, warnings: 0"),
        "{text}"
    );
    assert!(
        text.contains("error: line 5: HUSB @I999@: no record has this identifier"),
        "{text}"
    );
}

#[test]
fn validate_reads_the_encoding_and_the_line_grammar() {
    // CR line endings and a Windows-1252 byte under CHAR UTF-8: the bytes
    // are read, and the label is reported.
    let file = TempFile::new(
        b"0 HEAD\r1 SOUR EXAMPLE\r1 SUBM @U1@\r1 GEDC\r2 VERS 5.5.1\r2 FORM LINEAGE-LINKED\r\
          1 CHAR UTF-8\r0 @U1@ SUBM\r1 NAME Caf\xe9 Example\r0 TRLR\r",
    );
    let output = run(&["--validate", "--validation-level", "strict", file.path()]);
    assert_eq!(output.status.code(), Some(2));
    let text = stdout(&output);
    assert!(text.contains("HEAD.CHAR says UTF-8"), "{text}");
}

// Usage errors

#[test]
fn usage_errors_exit_with_code_3() {
    let file = TempFile::new(VALID_551);
    let path = file.path();
    for (args, message) in [
        (
            vec!["--validation-level", "strict", path],
            "requires --validate",
        ),
        (
            vec!["--validate", "--validation-level", "pedantic", path],
            "unknown validation level: pedantic",
        ),
        (vec!["--write", "6.0", path], "unknown version: 6.0"),
        (vec!["--write"], "--write expects a version"),
        (vec!["--frobnicate", path], "unknown option: --frobnicate"),
        (vec![], "missing file"),
        (vec![path, path], "expected one file"),
        (
            vec!["--validate", "--write", "7.0", path],
            "cannot be combined",
        ),
        (
            vec!["--individual", "@I1@", "--validate", path],
            "cannot be combined",
        ),
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(3), "{args:?}");
        let error = stderr(&output);
        assert!(error.contains("usage error"), "{args:?}: {error}");
        assert!(error.contains(message), "{args:?}: {error}");
        assert!(output.stdout.is_empty(), "{args:?}");
    }
}

#[test]
fn help_and_io_errors() {
    let output = run(&["--help"]);
    assert!(output.status.success());
    assert!(stdout(&output).contains("--write <VERSION>"));

    let missing = Path::new(&std::env::temp_dir()).join("ged_io_cli_test_no_such_file.ged");
    let output = run(&[missing.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("I/O error"));
}
