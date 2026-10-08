//! The only module of the suite that touches ged_io's API.
//!
//! Every test reads and writes through these functions and asserts on
//! behaviour: what the model keeps (searched as text), what the writer emits
//! and what reads back. When the crate's types or entry points change, this
//! file is the one to update; the cases, tables and checks stay as they are.
//!
//! Two pipelines are under test, selected by `RATCHET_TIER` (see
//! `ratchet.rs`): `current`, the public model (`GedcomBuilder`,
//! `GedcomWriter::write_to_string`), and `next`, the model under
//! construction (`ged_io::next`), whose results the `next` column of the
//! known-gaps file records.

use ged_io::{GedcomBuilder, GedcomVersion, GedcomWriter, LineEnding};
use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;

/// A parsed dataset, opaque to the tests.
pub enum Model {
    /// The public model.
    Current(Box<ged_io::types::GedcomData>),
    /// The model under construction.
    Next(ged_io::next::Dataset),
}

/// Whether the suite runs the `next` pipeline.
fn next_tier() -> bool {
    super::ratchet::tier() == "next"
}

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
        if next_tier() {
            return Ok(Model::Next(ged_io::next::read_bytes(bytes)));
        }
        GedcomBuilder::new()
            .build_from_bytes(bytes)
            .map(|d| Model::Current(Box::new(d)))
            .map_err(|e| e.to_string())
    })
}

/// Reads bytes and reports dangling pointers as an error (the reference
/// check the README documents for application backends).
pub fn read_checking_references(bytes: &[u8]) -> Result<Model, String> {
    guard(|| {
        if next_tier() {
            let data = ged_io::next::read_bytes(bytes);
            return match dangling(&data) {
                Some(p) => Err(format!("dangling pointer {p}")),
                None => Ok(Model::Next(data)),
            };
        }
        GedcomBuilder::new()
            .validate_references(true)
            .build_from_bytes(bytes)
            .map(|d| Model::Current(Box::new(d)))
            .map_err(|e| e.to_string())
    })
}

/// The first pointer of a `next` dataset to no record (7.x `@VOID@`
/// aside).
fn dangling(data: &ged_io::next::Dataset) -> Option<String> {
    let records = data.to_structures();
    let defined: std::collections::HashSet<&str> =
        records.iter().filter_map(|r| r.xref.as_deref()).collect();
    let mut stack: Vec<&ged_io::tree::Structure> = records.iter().collect();
    while let Some(s) = stack.pop() {
        if let Some(p) = s.pointer() {
            let void = data.version().is_v7() && p.is_void();
            if !void && !defined.contains(p.as_str()) {
                return Some(p.to_string());
            }
        }
        stack.extend(&s.substructures);
    }
    None
}

/// The structures the `next` model types but read untyped (it found them
/// not to fit their type): none for a conformant input. Empty for the
/// current model.
pub fn untyped(m: &Model) -> Vec<String> {
    match m {
        Model::Current(_) => Vec::new(),
        Model::Next(data) => ged_io::next::ledger::untyped(data),
    }
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

fn writer(target: Target, eol: Option<&str>) -> GedcomWriter {
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
    w
}

/// Writes with an explicit line terminator.
pub fn write_with(m: &Model, target: Target, eol: Option<&str>) -> Result<String, String> {
    guard(|| {
        let w = writer(target, eol);
        match m {
            Model::Current(data) => w.write_to_string(data),
            Model::Next(data) => ged_io::next::write_string(data, &w),
        }
        .map_err(|e| e.to_string())
    })
}

/// Writes the model as GEDCOM 5.5.1 bytes in ANSEL (`CHAR ANSEL`).
pub fn write_ansel(m: &Model) -> Result<Vec<u8>, String> {
    guard(|| {
        let mut bytes = Vec::new();
        let w = GedcomWriter::new()
            .gedcom_version(GedcomVersion::V5_5_1)
            .output_encoding(ged_io::OutputEncoding::Ansel);
        match m {
            Model::Current(data) => w.write(&mut bytes, data),
            Model::Next(data) => ged_io::next::write(data, &w, &mut bytes),
        }
        .map_err(|e| e.to_string())?;
        Ok(bytes)
    })
}

/// The whole model as text, for "the model keeps this value" checks.
pub fn dump(m: &Model) -> String {
    use std::fmt::Write;
    let data = match m {
        Model::Current(data) => data,
        // The structures the model writes hold everything it keeps.
        Model::Next(data) => return format!("{:?}", data.to_structures()),
    };
    let mut d = format!("{data:?}");
    // The `Debug` of an event shows little of it: add the dates and ages,
    // which the model keeps as written.
    let mut add = |date: &Option<ged_io::types::date::Date>,
                   age: &Option<ged_io::types::age::Age>| {
        let _ = write!(d, "\n{date:?} {age:?}");
    };
    for i in &data.individuals {
        for e in &i.events {
            add(&e.date, &e.age);
        }
        for a in &i.attributes {
            add(&a.date, &a.age);
        }
    }
    for f in &data.families {
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
    match m {
        Model::Current(data) => {
            let mut data = (**data).clone();
            data.header = None;
            format!("{data:?}")
        }
        Model::Next(data) => {
            let mut records = data.to_structures();
            records.retain(|r| r.tag != "HEAD");
            format!("{records:?}")
        }
    }
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
    match m {
        Model::Current(data) => data.total_records(),
        Model::Next(data) => {
            let tags = ["HEAD", "TRLR"];
            data.records
                .iter()
                .filter(|r| !tags.contains(&data.store.tag(r.tag)))
                .count()
        }
    }
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
    let data = match m {
        Model::Current(data) => data,
        Model::Next(data) => {
            return xrefs
                .iter()
                .map(|x| data.find(x).map(|r| format!("{r:?}")))
                .collect();
        }
    };
    let idx = ged_io::indexed::IndexedGedcomData::from(*data);
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
    guard(|| match m {
        Model::Current(data) => {
            let json = serde_json::to_string(data).map_err(|e| e.to_string())?;
            let back: ged_io::types::GedcomData =
                serde_json::from_str(&json).map_err(|e| e.to_string())?;
            Ok((json, back == **data))
        }
        // The model under construction exports the structures it writes.
        Model::Next(data) => {
            let records = data.to_structures();
            let json = serde_json::to_string(&records).map_err(|e| e.to_string())?;
            let back: Vec<ged_io::tree::Structure> =
                serde_json::from_str(&json).map_err(|e| e.to_string())?;
            Ok((json, back == records))
        }
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
        let bytes = match m {
            Model::Current(data) => {
                let files: std::collections::HashMap<String, Vec<u8>> = media
                    .iter()
                    .map(|(n, b)| ((*n).to_string(), b.to_vec()))
                    .collect();
                ged_io::gedzip::write_gedzip_with_media(data, &files).map_err(|e| e.to_string())?
            }
            Model::Next(data) => {
                let mut gedcom = Vec::new();
                ged_io::next::write(data, &GedcomWriter::new(), &mut gedcom)
                    .map_err(|e| e.to_string())?;
                let mut w = ged_io::gedzip::GedzipWriter::new(std::io::Cursor::new(Vec::new()))
                    .map_err(|e| e.to_string())?;
                w.write_gedcom_bytes(&gedcom).map_err(|e| e.to_string())?;
                for (name, b) in media {
                    w.add_media_file(name, b).map_err(|e| e.to_string())?;
                }
                w.finish().map_err(|e| e.to_string())?.into_inner()
            }
        };
        let mut reader = ged_io::gedzip::GedzipReader::new(std::io::Cursor::new(bytes.as_slice()))
            .map_err(|e| e.to_string())?;
        let names = reader
            .media_files()
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let back = if next_tier() {
            let gedcom = reader.read_gedcom_bytes().map_err(|e| e.to_string())?;
            Model::Next(ged_io::next::read_bytes(gedcom))
        } else {
            Model::Current(Box::new(reader.parse_gedcom().map_err(|e| e.to_string())?))
        };
        Ok((back, names))
    })
}

/// Reads a GEDZIP archive.
#[cfg(feature = "gedzip")]
pub fn read_gedzip(bytes: &[u8]) -> Result<Model, String> {
    guard(|| {
        if next_tier() {
            let mut reader = ged_io::gedzip::GedzipReader::new(std::io::Cursor::new(bytes))
                .map_err(|e| e.to_string())?;
            let gedcom = reader.read_gedcom_bytes().map_err(|e| e.to_string())?;
            return Ok(Model::Next(ged_io::next::read_bytes(gedcom)));
        }
        ged_io::gedzip::read_gedzip(bytes)
            .map(|d| Model::Current(Box::new(d)))
            .map_err(|e| e.to_string())
    })
}

/// The crate's validator on a written or input stream: each deviation as
/// (rule, line, detail), the rule being the `DeviationKind` name.
pub fn validate(text: &str) -> Vec<(String, u32, String)> {
    ged_io::spec::validate_text(text)
        .into_iter()
        .map(|d| (format!("{:?}", d.kind), d.line, d.detail.to_string()))
        .collect()
}
