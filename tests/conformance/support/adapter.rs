//! The only module of the suite that touches ged_io's API.
//!
//! Every test reads and writes through these functions and asserts on
//! behaviour: what the model keeps (searched as text), what the writer emits
//! and what reads back. When the crate's types or entry points change, this
//! file is the one to update; the cases, tables and checks stay as they are.

use ged_io::{GedcomBuilder, GedcomVersion, GedcomWriter, LineEnding};
use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;

/// A parsed dataset, opaque to the tests.
pub struct Model(ged_io::types::GedcomData);

thread_local! {
    static QUIET: Cell<bool> = const { Cell::new(false) };
}

/// Runs `f`, turning a panic into `Err("PANIC: …")` without printing it.
/// Panics outside `guard` (failing assertions) still print as usual.
pub fn guard<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let default = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if !QUIET.with(Cell::get) {
                default(info);
            }
        }));
    });
    QUIET.with(|q| q.set(true));
    let r = panic::catch_unwind(AssertUnwindSafe(f));
    QUIET.with(|q| q.set(false));
    match r {
        Ok(r) => r,
        Err(e) => {
            let msg = e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_default();
            Err(format!("PANIC: {msg}"))
        }
    }
}

/// Reads bytes with the default (lenient) configuration.
pub fn read(bytes: &[u8]) -> Result<Model, String> {
    guard(|| {
        GedcomBuilder::new()
            .build_from_bytes(bytes)
            .map(Model)
            .map_err(|e| e.to_string())
    })
}

/// Reads bytes and reports dangling pointers as an error (the reference
/// check the README documents for application backends).
pub fn read_checking_references(bytes: &[u8]) -> Result<Model, String> {
    guard(|| {
        GedcomBuilder::new()
            .validate_references(true)
            .build_from_bytes(bytes)
            .map(Model)
            .map_err(|e| e.to_string())
    })
}

/// Target of a write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The version the data declares (5.5.1 when it declares none).
    Same,
    V551,
    V70,
    V71,
}

/// Writes the model with the default configuration for `target`.
pub fn write(m: &Model, target: Target) -> Result<String, String> {
    write_with(m, target, None)
}

/// Writes with an explicit line terminator.
pub fn write_with(m: &Model, target: Target, eol: Option<&str>) -> Result<String, String> {
    guard(|| {
        let mut w = GedcomWriter::new();
        w = match target {
            Target::Same => w,
            Target::V551 => w.gedcom_version(GedcomVersion::V5_5_1),
            Target::V70 => w.gedcom_version(GedcomVersion::V7_0),
            Target::V71 => w.gedcom_version(GedcomVersion::V7_1),
        };
        if let Some(eol) = eol {
            w = w.line_ending(match eol {
                "\r\n" => LineEnding::CrLf,
                "\r" => LineEnding::Cr,
                _ => LineEnding::Lf,
            });
        }
        w.write_to_string(&m.0).map_err(|e| e.to_string())
    })
}

/// Writes the model as GEDCOM 5.5.1 bytes in ANSEL (`CHAR ANSEL`).
pub fn write_ansel(m: &Model) -> Result<Vec<u8>, String> {
    guard(|| {
        let mut bytes = Vec::new();
        GedcomWriter::new()
            .gedcom_version(GedcomVersion::V5_5_1)
            .output_encoding(ged_io::OutputEncoding::Ansel)
            .write(&mut bytes, &m.0)
            .map_err(|e| e.to_string())?;
        Ok(bytes)
    })
}

/// The whole model as text, for "the model keeps this value" checks.
pub fn dump(m: &Model) -> String {
    use std::fmt::Write;
    let mut d = format!("{:?}", m.0);
    // The `Debug` of an event shows little of it: add the dates and ages,
    // which the model keeps as written.
    let mut add = |date: &Option<ged_io::types::date::Date>,
                   age: &Option<ged_io::types::age::Age>| {
        let _ = write!(d, "\n{date:?} {age:?}");
    };
    for i in &m.0.individuals {
        for e in &i.events {
            add(&e.date, &e.age);
        }
        for a in &i.attributes {
            add(&a.date, &a.age);
        }
    }
    for f in &m.0.families {
        for e in &f.events {
            add(&e.date, &e.age);
            for detail in &e.family_event_details {
                add(&None, &detail.age);
            }
        }
    }
    d
}

/// The records of the model as text, header excluded.
pub fn dump_records(m: &Model) -> String {
    let mut data = m.0.clone();
    data.header = None;
    format!("{data:?}")
}

/// True when `needle` appears in the model. A needle is matched against the
/// model's text form, where a newline reads `\n` and a quote `\"`.
pub fn model_contains(m: &Model, needle: &str) -> bool {
    let d = dump(m);
    let quoted = format!("{needle:?}");
    d.contains(needle) || d.contains(&quoted[1..quoted.len() - 1])
}

/// Number of records of every kind, header excluded.
pub fn record_count(m: &Model) -> usize {
    m.0.total_records()
}

/// Reads with the streaming parser, one record at a time. Returns the text
/// form of each record.
pub fn read_streaming(bytes: &[u8]) -> Result<Vec<String>, String> {
    guard(|| {
        let parser = ged_io::GedcomStreamParser::new(std::io::Cursor::new(bytes))
            .map_err(|e| e.to_string())?;
        parser
            .map(|r| r.map(|rec| format!("{rec:?}")).map_err(|e| e.to_string()))
            .collect()
    })
}

/// Looks an xref up through the indexed (O(1)) view; returns the record's
/// text form.
pub fn indexed_find(m: Model, xrefs: &[&str]) -> Vec<Option<String>> {
    let idx = ged_io::indexed::IndexedGedcomData::from(m.0);
    xrefs
        .iter()
        .map(|x| {
            idx.find_individual(x)
                .map(|r| format!("{r:?}"))
                .or_else(|| idx.find_family(x).map(|r| format!("{r:?}")))
                .or_else(|| idx.find_source(x).map(|r| format!("{r:?}")))
                .or_else(|| idx.find_repository(x).map(|r| format!("{r:?}")))
                .or_else(|| idx.find_multimedia(x).map(|r| format!("{r:?}")))
                .or_else(|| idx.find_submitter(x).map(|r| format!("{r:?}")))
        })
        .collect()
}

/// Serialises the model to JSON and back; returns the JSON text and whether
/// the value read back equals the original.
#[cfg(feature = "json")]
pub fn json_round_trip(m: &Model) -> Result<(String, bool), String> {
    guard(|| {
        let json = serde_json::to_string(&m.0).map_err(|e| e.to_string())?;
        let back: ged_io::types::GedcomData =
            serde_json::from_str(&json).map_err(|e| e.to_string())?;
        Ok((json, back == m.0))
    })
}

/// Writes a GEDZIP archive with media files, reads it back and returns the
/// model and the media names found in the archive.
#[cfg(feature = "gedzip")]
pub fn gedzip_round_trip(
    m: &Model,
    media: &[(&str, &[u8])],
) -> Result<(Model, Vec<String>), String> {
    guard(|| {
        let files: std::collections::HashMap<String, Vec<u8>> = media
            .iter()
            .map(|(n, b)| ((*n).to_string(), b.to_vec()))
            .collect();
        let bytes =
            ged_io::gedzip::write_gedzip_with_media(&m.0, &files).map_err(|e| e.to_string())?;
        let mut reader = ged_io::gedzip::GedzipReader::new(std::io::Cursor::new(bytes.as_slice()))
            .map_err(|e| e.to_string())?;
        let names = reader
            .media_files()
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let data = reader.parse_gedcom().map_err(|e| e.to_string())?;
        Ok((Model(data), names))
    })
}

/// Reads a GEDZIP archive.
#[cfg(feature = "gedzip")]
pub fn read_gedzip(bytes: &[u8]) -> Result<Model, String> {
    guard(|| {
        ged_io::gedzip::read_gedzip(bytes)
            .map(Model)
            .map_err(|e| e.to_string())
    })
}
