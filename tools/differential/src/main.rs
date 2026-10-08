//! Differential run between the published ged_io 0.17.0 and this checkout.
//!
//! For each file, both versions read it and write it back in its own
//! version; the tool reports every `path = value` line the published version
//! writes and this checkout does not. The new pipeline must keep a superset
//! of what 0.17 kept, apart from reviewed, intended changes.
//!
//! ```sh
//! cargo run --manifest-path tools/differential/Cargo.toml -- target/corpora/**/*.ged
//! ```

use std::collections::BTreeMap;
use std::process::ExitCode;

/// `path = value` lines of a written stream (CONT/CONC folded), with their count.
fn lines(text: &str) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    let mut stack: Vec<String> = Vec::new();
    let mut last: Option<String> = None;
    for raw in text.split(['\n', '\r']).filter(|l| !l.is_empty()) {
        let mut parts = raw.splitn(3, ' ');
        let (Some(level), Some(mut tag)) = (parts.next(), parts.next()) else {
            continue;
        };
        let Ok(level) = level.parse::<usize>() else {
            continue;
        };
        let mut rest = parts.next().unwrap_or("");
        if tag.starts_with('@') {
            let mut p = rest.splitn(2, ' ');
            tag = p.next().unwrap_or("");
            rest = p.next().unwrap_or("");
        }
        if tag == "CONT" || tag == "CONC" {
            if let Some(k) = last.take() {
                let n = out.remove(&k).unwrap_or(1);
                let k = format!("{k}{}{rest}", if tag == "CONT" { "\n" } else { "" });
                *out.entry(k.clone()).or_insert(0) += n;
                last = Some(k);
            }
            continue;
        }
        stack.truncate(level);
        stack.push(tag.to_string());
        let k = format!("{} = {rest}", stack.join("/"));
        *out.entry(k.clone()).or_insert(0) += 1;
        last = Some(k);
    }
    out
}

fn published(bytes: &[u8]) -> Result<String, String> {
    let d = ged_io_published::GedcomBuilder::new()
        .build_from_bytes(bytes)
        .map_err(|e| e.to_string())?;
    ged_io_published::GedcomWriter::new()
        .write_to_string(&d)
        .map_err(|e| e.to_string())
}

fn current(bytes: &[u8]) -> Result<String, String> {
    let d = ged_io::GedcomBuilder::new()
        .build_from_bytes(bytes)
        .map_err(|e| e.to_string())?;
    ged_io::GedcomWriter::new()
        .write_to_string(&d)
        .map_err(|e| e.to_string())
}

fn main() -> ExitCode {
    let mut regressions = 0;
    for path in std::env::args().skip(1) {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("{path}: {e}");
                return ExitCode::FAILURE;
            }
        };
        match (published(&bytes), current(&bytes)) {
            (Ok(old), Ok(new)) => {
                let (old, new) = (lines(&old), lines(&new));
                for (k, n) in &old {
                    let m = new.get(k).copied().unwrap_or(0);
                    if m < *n {
                        regressions += 1;
                        println!("{path}\tlost\t{k:?} (x{})", n - m);
                    }
                }
            }
            (Ok(_), Err(e)) => {
                regressions += 1;
                println!("{path}\tnow fails\t{e}");
            }
            (Err(_), Ok(_)) => println!("{path}\tnow reads"),
            (Err(_), Err(_)) => {}
        }
    }
    if regressions == 0 {
        ExitCode::SUCCESS
    } else {
        eprintln!("{regressions} regressions against ged_io 0.17.0");
        ExitCode::FAILURE
    }
}
