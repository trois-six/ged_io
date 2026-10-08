//! Streaming reads exactly what in-memory reading reads.
//!
//! For every file: the records of [`TreeReader`] equal those of
//! [`Tree::from_bytes`] (line numbers included), the tree's dump reads back
//! to the same tree, and the typed model of [`GedcomStreamParser`] equals the
//! one of [`GedcomBuilder::build_from_bytes`] whenever the token-based parser
//! accepts the file.
//!
//! The crate's fixtures run with the normal suite. The external corpora run
//! on request: `GED_IO_CORPORA=dir1:dir2 cargo test --test
//! streaming_equivalence -- --ignored`.

use std::io::BufReader;
use std::path::{Path, PathBuf};

use ged_io::tree::{parse_tree, Structure, Tree, TreeReader};
use ged_io::types::GedcomData;
use ged_io::{GedcomBuilder, GedcomStreamParser};

fn lines(s: &Structure, out: &mut Vec<u32>) {
    out.push(s.line);
    for c in &s.substructures {
        lines(c, out);
    }
}

/// Compares the two paths on one input; returns a description of the first
/// difference.
fn compare(bytes: &[u8]) -> Result<(), String> {
    let tree = Tree::from_bytes(bytes);
    let memory = tree.to_structures();
    for capacity in [4096, 61] {
        let streamed: Vec<Structure> = TreeReader::new(BufReader::with_capacity(capacity, bytes))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        if streamed.len() != memory.len() {
            return Err(format!(
                "tree: {} records streamed, {} in memory",
                streamed.len(),
                memory.len()
            ));
        }
        for (i, (s, m)) in streamed.iter().zip(&memory).enumerate() {
            if s != m {
                return Err(format!("tree: record {i} differs:\n{s:?}\n{m:?}"));
            }
            let (mut a, mut b) = (Vec::new(), Vec::new());
            lines(s, &mut a);
            lines(m, &mut b);
            if a != b {
                return Err(format!("tree: record {i} line numbers differ"));
            }
        }
    }
    if parse_tree(&tree.to_gedcom()).to_structures() != memory {
        return Err("tree: the dump does not read back to the same tree".into());
    }

    let in_memory = GedcomBuilder::new().build_from_bytes(bytes);
    let streamed: Result<GedcomData, _> =
        GedcomStreamParser::new(BufReader::with_capacity(97, bytes)).and_then(|p| p.collect());
    if let (Ok(m), Ok(s)) = (&in_memory, &streamed) {
        if m != s {
            return Err("model: streaming and in-memory models differ".into());
        }
    }
    Ok(())
}

fn ged_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            ged_files(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("ged"))
        {
            out.push(path);
        }
    }
}

fn run(dirs: &[PathBuf]) -> usize {
    let mut files = Vec::new();
    for dir in dirs {
        ged_files(dir, &mut files);
    }
    files.sort();
    let mut failures = Vec::new();
    for file in &files {
        let bytes = std::fs::read(file).unwrap();
        if let Err(e) = compare(&bytes) {
            failures.push(format!("{}: {e}", file.display()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    files.len()
}

#[test]
fn fixtures() {
    let n = run(&[Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")]);
    assert!(n >= 5);
}

#[test]
#[ignore = "reads the external corpora listed in GED_IO_CORPORA"]
fn corpora() {
    let dirs: Vec<PathBuf> = std::env::var("GED_IO_CORPORA")
        .expect("GED_IO_CORPORA: colon-separated corpus directories")
        .split(':')
        .map(PathBuf::from)
        .collect();
    let n = run(&dirs);
    eprintln!("{n} files compared");
}
