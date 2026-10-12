//! Writing the typed model: each record writes its structures for the
//! target version into the writer's flat arena, when the writer needs them,
//! so that a dataset is never copied as owned structures.

use std::borrow::Cow;

use crate::spec::conform::Build;
use crate::tree::{Flat, FlatPayload, Structure};
use crate::writer::GedcomWriter;

use super::dataset::{Dataset, RecordRef};
use super::driver::WriteCx;

/// A record of a dataset that writes its structures for the target version
/// when the writer needs them.
pub(crate) struct Built<'d> {
    /// The record; `None` for the trailer, which the dataset does not keep.
    record: Option<RecordRef<'d>>,
    cx: WriteCx<'d>,
}

impl<'d> Build<'d> for Built<'d> {
    fn tag(&self) -> &'d str {
        self.record
            .map_or("TRLR", |r| r.tag(self.cx.store, self.cx.version))
    }

    fn xref(&self) -> Option<&'d str> {
        self.record?.xref().map(|x| self.cx.store.xref(x))
    }

    fn is_empty(&self) -> bool {
        self.record.is_none_or(|r| r.is_empty())
    }

    fn build(&self, out: &mut Flat<'d>) {
        if let Some(r) = self.record {
            r.to_flat(&self.cx, out);
        } else {
            let at = out.open(Cow::Borrowed("TRLR"), None, FlatPayload::None);
            out.close(at);
        }
    }

    fn owned(&self) -> Option<Structure> {
        let header = self.record.filter(|r| matches!(r, RecordRef::Header(_)))?;
        Some(header.to_structure(&self.cx))
    }
}

impl Dataset {
    /// The records, to write with `writer` (in its version, unless one is
    /// configured), then the trailer.
    pub(crate) fn built(&self, writer: &GedcomWriter) -> Vec<Built<'_>> {
        let cx = WriteCx {
            store: &self.store,
            version: writer.config().version.unwrap_or(self.version),
            convert: true,
        };
        self.records()
            .map(Some)
            .chain([None])
            .map(|record| Built { record, cx })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::conform::Typing;
    use crate::tree::Node as _;
    use crate::version::GedcomVersion;

    /// Each record of `text` but the header, written typed for `version`:
    /// whether it typed itself, the tags of the structures whose payloads
    /// the check looks at, and of its extension structures.
    fn typed(text: &str, version: GedcomVersion) -> Vec<(bool, Vec<String>, Vec<String>)> {
        let data = Dataset::parse(text);
        let cx = WriteCx {
            store: &data.store,
            version,
            convert: true,
        };
        let mut flat = Flat::default();
        flat.set_typing(Some(Typing::of(version)));
        data.records()
            .filter(|r| !matches!(r, RecordRef::Header(_)))
            .map(|r| {
                flat.clear();
                r.to_flat(&cx, &mut flat);
                let checks = flat.check_tags().map(str::to_string).collect();
                let ext = flat.extensions().map(|e| e.tag().to_string()).collect();
                (flat.typed(), checks, ext)
            })
            .collect()
    }

    /// A typed record places each structure by its type and trusts the
    /// values its types made valid (an enumeration value, a date of the
    /// commonest shapes); its texts and pointers are left to the check.
    #[test]
    fn typed_records_trust_their_typed_values() {
        let text = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NAME Ann /Example/\n2 GIVN Ann\n1 SEX F\n1 BIRT\n2 DATE 1 JAN 1900\n2 PLAC Sampleton\n1 FAMS @F1@\n1 _UID 0123\n0 @F1@ FAM\n1 WIFE @I1@\n0 TRLR\n";
        let records = typed(text, GedcomVersion::V5_5_1);
        let (typed, checks, ext) = &records[0];
        assert!(typed);
        assert_eq!(checks, &["NAME", "GIVN", "PLAC", "FAMS"]);
        assert_eq!(ext, &["_UID"]);
        assert_eq!(records[1], (true, vec!["WIFE".to_string()], vec![]));
    }

    /// What a type cannot vouch for leaves the record to the check's walk:
    /// a repeated singleton kept in `extra`, a structure the target version
    /// does not have; an enumeration value of another set is looked at.
    #[test]
    fn untyped_structures_leave_the_record_to_the_walk() {
        let text = "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 SEX F\n1 SEX M\n0 @I2@ INDI\n1 NO MARR\n0 @I3@ INDI\n1 CONL\n2 STAT DNS_CAN\n3 DATE 1 JAN 2000\n0 TRLR\n";
        let records = typed(text, GedcomVersion::V5_5_1);
        assert!(!records[0].0, "{records:?}");
        assert!(!records[1].0, "{records:?}");
        assert_eq!(records[2], (true, vec!["STAT".to_string()], vec![]));
        let records = typed(text, GedcomVersion::V7_0);
        assert!(records[1].0, "{records:?}");
    }

    /// Writing a dataset one record at a time writes what the whole
    /// conformance pass writes, with the same repairs, on every fixture in
    /// every version, whatever the output; most fixtures need no restart.
    #[test]
    fn the_direct_write_is_the_whole_pass() {
        let mut files = Vec::new();
        ged_files(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
            &mut files,
        );
        files.sort();
        let restarts = || crate::writer::RESTARTS.with(std::cell::Cell::get);
        let (mut writes, before) = (0, restarts());
        let mut restarted = Vec::new();
        for path in files {
            let data = Dataset::from_bytes(std::fs::read(&path).unwrap());
            for version in [
                GedcomVersion::V5_5_1,
                GedcomVersion::V7_0,
                GedcomVersion::V7_1,
            ] {
                let name = path.display();
                let writer = GedcomWriter::new().gedcom_version(version);
                let at = restarts();
                let direct = writer.write_to_string_with_report(&data).unwrap();
                if restarts() > at {
                    restarted.push(format!("{name} {version}"));
                }
                let whole = writer.write_to_string_whole(&data).unwrap();
                assert_eq!(direct.0, whole.0, "{name} as {version}");
                assert_eq!(direct.1, whole.1, "{name} as {version}");
                // The first repair as the error, under the strict policy.
                let strict = writer.clone().on_nonconformant(crate::RepairPolicy::Error);
                let error = |r: Result<(String, crate::WriteReport), crate::WriteError>| match r {
                    Ok((text, _)) => Ok(text),
                    Err(e) => Err(e.to_string()),
                };
                assert_eq!(
                    error(strict.write_to_string_with_report(&data)),
                    error(strict.write_to_string_whole(&data)),
                    "{name} as {version}"
                );
                writes += 1;
            }
        }
        let restarts = restarts() - before;
        eprintln!("{restarts} of {writes} direct writes restarted: {restarted:#?}");
        assert!(restarts * 4 < writes, "{restarts} of {writes}");
    }

    /// Every `.ged` file under `dir`.
    fn ged_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                ged_files(&p, out);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ged")) {
                out.push(p);
            }
        }
    }

    /// A dataset's records write their structures where they are, one at a
    /// time: what the writer writes of them is what it writes of the owned
    /// structures they give, in the order of the model, repairs included.
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
            let data = Dataset::from_bytes(std::fs::read(&path).unwrap());
            for version in [
                GedcomVersion::V5_5_1,
                GedcomVersion::V7_0,
                GedcomVersion::V7_1,
            ] {
                let writer = GedcomWriter::new()
                    .gedcom_version(version)
                    .bom(crate::Bom::Never);
                let mut direct = Vec::new();
                let report = writer.write(&mut direct, &data).unwrap();
                let mut owned = Vec::new();
                let expected = writer
                    .write_model_structures(
                        &mut owned,
                        &data.to_structures_for(version),
                        data.version(),
                        super::super::field_order,
                    )
                    .unwrap();
                let name = path.display();
                assert_eq!(
                    String::from_utf8_lossy(&direct),
                    String::from_utf8_lossy(&owned),
                    "{name} as {version}"
                );
                assert_eq!(report.repairs, expected.repairs, "{name} as {version}");
                assert_eq!(
                    writer.write_to_string(&data).unwrap().as_bytes(),
                    direct.as_slice(),
                    "{name} as {version}"
                );
            }
        }
    }
}
