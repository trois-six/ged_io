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
}
