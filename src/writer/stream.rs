//! Writing a file one record at a time: [`GedcomStreamWriter`].

use std::borrow::Cow;
use std::collections::HashMap;
use std::io;

use super::emit::{candidate_xref, is_valid_pointer, new_xref, LineSink};
use super::head;
use super::xref::{numbered_xref, XrefIndex};
use super::{
    map_pointers, put_flat, put_record, put_structure, record_prefix, Emitter, GedcomWriter,
    Pointers, Repair, RepairKind, RepairPolicy, WriteError, WriteReport, WriterConfig,
};
use crate::model::driver::WriteCx;
use crate::model::write::Built;
use crate::model::Header;
use crate::model::{RecordRef, Store};
use crate::spec::conform::streamed::{Outcome, Streamed};
use crate::tree::{Node, PayloadRef, Structure, Tag, Xref};
use crate::version::{VersionRules, XrefGrammar};

/// The identifier of the stub submitter a 5.5.1 header without one points
/// to.
const STUB: &str = "@U1@";

/// Writes GEDCOM records one at a time, with memory bounded by one record.
///
/// [`GedcomStreamWriter`] writes the header, then each record as it is
/// given, then the trailer, with the conformance repair and the line
/// emitter of [`GedcomWriter`]: every line it writes follows the
/// version's grammar, and every record the version's structure rules. It
/// keeps one record and a block of output at a time, and the identifiers
/// of the records it wrote (16 bytes each for nearly all), so that they
/// stay unique.
///
/// # Differences from [`GedcomWriter`]
///
/// Records are written in the order they are given; [`GedcomWriter`]
/// writes a [`Dataset`] by record type. Given the records of a dataset in
/// the order [`Dataset::records`] yields them, the output is the one
/// [`GedcomWriter`] writes, except where knowing the records to come would
/// change it:
///
/// - **Identifiers.** A record identifier valid in the version and not
///   written yet is kept. An invalid one is written as the valid form
///   [`GedcomWriter`] gives it first (`@i1@` as `@I1@` in 7.x), with a
///   `_2`, `_3`… suffix when that form is taken, and so is a repeated
///   one; a standard record without one gets the first free `@I1@`,
///   `@F1@`…. Free means: not written yet. A later record holding that
///   identifier is then renamed, where [`GedcomWriter`], knowing every
///   identifier first, renames the earlier one.
/// - **Pointers.** A pointer is written as it is when it is valid, and in
///   the valid form of its identifier otherwise (the form a record with
///   that identifier gets, when no suffix was needed): pointers keep
///   naming the records they named. A pointer is taken to name a record
///   of the type its structure points to — that record may come later —
///   so a dangling pointer, or one to a record of another type, is not
///   repaired.
/// - **Header.** The header is written first, completed for the version:
///   a 5.5.1 header without `SUBM` points to a stub submitter `@U1@`,
///   written right after it ([`GedcomWriter`] points to the first
///   submitter of the dataset, when there is one). A structure that a
///   repair relocates in 7.x becomes a plain extension structure, the
///   header being written, where [`GedcomWriter`] declares its tag in
///   `HEAD.SCHMA`.
/// - **Large records.** The notes moved out of a 5.5.1 record over 32K
///   go to `NOTE` records written right after it.
///
/// A record that a repair makes an extension record is kept until the
/// end and written before the trailer, where [`GedcomWriter`] writes it
/// too: such records (rare: a record type the version does not have, a
/// record that cannot be repaired otherwise) are the only ones held.
///
/// Under [`RepairPolicy::Error`], the first repair is an error as soon as
/// it is found; what was written before stays written.
///
/// [`Dataset`]: crate::Dataset
/// [`Dataset::records`]: crate::Dataset::records
///
/// # Example
///
/// ```rust
/// use ged_io::{GedcomStreamParser, GedcomStreamWriter, GedcomVersion, WriterConfig};
/// use ged_io::model::RecordRef;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let file: &[u8] = b"0 HEAD\n1 GEDC\n2 VERS 5.5.1\n1 SUBM @U1@\n0 @U1@ SUBM\n1 NAME Ann\n0 @I1@ INDI\n1 NAME Ann /Example/\n0 TRLR\n";
/// let mut records = GedcomStreamParser::new(file)?;
/// let head = records.next().unwrap()?;
/// let RecordRef::Header(header) = head.record() else { unreachable!() };
/// let config = WriterConfig { version: Some(GedcomVersion::V7_0), ..WriterConfig::default() };
/// let mut writer = GedcomStreamWriter::new(Vec::new(), config, header, &head)?;
/// for record in records {
///     let record = record?;
///     writer.write_record(record.record(), &record)?;
/// }
/// let (bytes, report) = writer.finish()?;
/// let text = String::from_utf8(bytes)?;
/// assert!(text.contains("0 @I1@ INDI\n1 NAME Ann /Example/\n"));
/// assert!(text.ends_with("0 TRLR\n"));
/// assert!(report.repairs.is_empty());
/// # Ok(())
/// # }
/// ```
pub struct GedcomStreamWriter<W: io::Write> {
    writer: W,
    policy: RepairPolicy,
    rules: &'static VersionRules,
    /// The output not handed to `writer` yet, at most a block and a record.
    sink: LineSink<'static>,
    conform: Streamed,
    xrefs: StreamXrefs,
    /// The records a repair made extension records, with the identifiers
    /// they are written with: written last, as [`GedcomWriter`] writes
    /// them.
    held: Vec<(Structure, Option<String>)>,
}

impl<W: io::Write> std::fmt::Debug for GedcomStreamWriter<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GedcomStreamWriter")
            .field("version", &self.rules.version())
            .finish_non_exhaustive()
    }
}

impl<W: io::Write> GedcomStreamWriter<W> {
    /// Starts writing to `writer` with `config`: writes the header (whose
    /// texts and identifiers resolve against `store`), completed and
    /// repaired for the version. The version is the configured one, else
    /// the one the header declares, else 5.5.1.
    ///
    /// # Errors
    ///
    /// [`WriteError::Io`] when `writer` fails, [`WriteError::Unencodable`]
    /// when the output encoding cannot represent the header, and
    /// [`WriteError::NonConformant`] on a repair under
    /// [`RepairPolicy::Error`].
    pub fn new<S: AsRef<Store> + ?Sized>(
        writer: W,
        config: WriterConfig,
        header: &Header,
        store: &S,
    ) -> Result<Self, WriteError> {
        let store = store.as_ref();
        let policy = config.on_nonconformant;
        let gw = GedcomWriter::with_config(config);
        let rules = gw.rules_for(header.declared_version(store).as_deref());
        let encoding = gw.encoding(rules);
        let mut sink = gw.sink(None, rules, encoding, policy);
        if gw.wants_bom(rules, encoding, false) {
            sink.bom();
        }
        let cx = WriteCx {
            store,
            version: rules.version,
            convert: true,
        };
        let mut head = RecordRef::Header(header).to_structure(&cx);
        let submitter = head::needs_submitter(&head, rules).then(|| STUB.to_string());
        let conform = Streamed::header(rules.version, &mut head, submitter.clone());
        let mut this = Self {
            writer,
            policy,
            rules,
            sink,
            conform,
            xrefs: StreamXrefs::new(rules),
            held: Vec::new(),
        };
        this.repairs_since(0)?;
        let stub = submitter.map(|x| {
            this.xrefs.take(&x);
            head::stub_submitter(x)
        });
        map_pointers(&mut head, &mut this.xrefs);
        let charset = this.sink.charset();
        head::complete(&mut head, rules, charset, None);
        let mut out = Emitter {
            rules,
            sink: &mut this.sink,
        };
        put_structure(&mut out, &head)?;
        if let Some(stub) = stub {
            put_structure(&mut out, &stub)?;
        }
        this.sink.drain_to(&mut this.writer, false)?;
        Ok(this)
    }

    /// Writes `record`, whose texts and identifiers resolve against
    /// `store` (a [`StreamedRecord`](crate::StreamedRecord) is both),
    /// repaired for the version when it needs it. A header is written as a
    /// `_HEAD` extension record, a trailer with content as `_TRLR`; an
    /// empty trailer is left out, the writer writing its own.
    ///
    /// # Errors
    ///
    /// As [`GedcomStreamWriter::new`].
    pub fn write_record<S: AsRef<Store> + ?Sized>(
        &mut self,
        record: RecordRef<'_>,
        store: &S,
    ) -> Result<(), WriteError> {
        let cx = WriteCx {
            store: store.as_ref(),
            version: self.rules.version,
            convert: true,
        };
        let mark = self.conform.repairs.len();
        match record {
            RecordRef::Header(_) => {
                let mut s = record.to_structure(&cx);
                s.tag = Tag::new("_HEAD");
                self.conform.repairs.push(Repair::new(
                    0,
                    RepairKind::Header,
                    "a second HEAD written as a _HEAD record",
                ));
                self.repairs_since(mark)?;
                self.put_owned(&s, None)?;
            }
            RecordRef::Other(n) if store.as_ref().tag(n.tag) == "TRLR" => {
                if !record.is_empty() || n.xref.is_some() {
                    let mut s = record.to_structure(&cx);
                    s.tag = Tag::new("_TRLR");
                    s.xref = None;
                    self.conform.repairs.push(Repair::new(
                        0,
                        RepairKind::Header,
                        "a TRLR with content written as a _TRLR record",
                    ));
                    self.repairs_since(mark)?;
                    self.put_owned(&s, None)?;
                }
            }
            _ => {
                let built = Built::new(record, cx);
                let xrefs = &mut self.xrefs;
                let outcome = self.conform.record(&built, &mut || xrefs.unique("@N1@"));
                self.repairs_since(mark)?;
                match outcome {
                    Outcome::Clean(flat) => {
                        if let Some(root) = flat.root() {
                            let xref = self.record_xref(root.xref(), root.tag())?;
                            let mut out = Emitter {
                                rules: self.rules,
                                sink: &mut self.sink,
                            };
                            put_flat(&mut out, &mut self.xrefs, xref.as_deref(), root)?;
                        }
                    }
                    Outcome::Repaired(s, notes) => {
                        let tag = record.tag(store.as_ref(), self.rules.version);
                        if s.tag.as_str().starts_with('_') && !tag.starts_with('_') {
                            let xref = self
                                .record_xref(s.xref.as_ref().map(Xref::as_str), s.tag.as_str())?;
                            self.held.push((s, xref));
                            return self.sink.drain_to(&mut self.writer, false);
                        }
                        self.put_owned(&s, None)?;
                        for note in &notes {
                            let xref = note.xref.as_ref().map(|x| x.as_str().to_string());
                            self.put_owned(note, xref)?;
                        }
                    }
                }
            }
        }
        self.sink.drain_to(&mut self.writer, false)
    }

    /// Writes the trailer, hands every byte to the writer and flushes it.
    /// Returns the writer and the repairs made, those of the structures
    /// first, then those of the lines.
    ///
    /// # Errors
    ///
    /// [`WriteError::Io`] when the writer fails.
    pub fn finish(mut self) -> Result<(W, WriteReport), WriteError> {
        for (s, xref) in std::mem::take(&mut self.held) {
            self.put_owned(&s, xref)?;
        }
        let mut out = Emitter {
            rules: self.rules,
            sink: &mut self.sink,
        };
        out.sink.source_line = 0;
        out.put(0, None, "TRLR", PayloadRef::None)?;
        self.sink.drain_to(&mut self.writer, true)?;
        self.writer.flush()?;
        let mut repairs = std::mem::take(&mut self.conform.repairs);
        repairs.append(&mut self.sink.repairs);
        Ok((self.writer, WriteReport { repairs }))
    }

    /// Fails with the first repair made since `mark` under
    /// [`RepairPolicy::Error`].
    fn repairs_since(&self, mark: usize) -> Result<(), WriteError> {
        match self.conform.repairs.get(mark..).and_then(<[_]>::first) {
            Some(first) if self.policy == RepairPolicy::Error => {
                Err(WriteError::NonConformant(Box::new(first.clone())))
            }
            _ => Ok(()),
        }
    }

    /// The identifier written for a record with the identifier `xref` and
    /// the tag `tag`, the rename reported.
    fn record_xref(&mut self, xref: Option<&str>, tag: &str) -> Result<Option<String>, WriteError> {
        let (new, repair) = self.xrefs.record(xref, record_prefix(tag));
        if let Some(repair) = repair {
            self.sink.repair(repair)?;
        }
        Ok(new)
    }

    /// Writes an owned record; `xref` is the identifier it is written
    /// with, when it is one the writer made already.
    fn put_owned(&mut self, s: &Structure, xref: Option<String>) -> Result<(), WriteError> {
        let xref = match xref {
            Some(x) => Some(x),
            None => self.record_xref(s.xref.as_ref().map(Xref::as_str), s.tag.as_str())?,
        };
        let mut out = Emitter {
            rules: self.rules,
            sink: &mut self.sink,
        };
        put_record(
            &mut out,
            &mut self.xrefs,
            xref.as_deref(),
            s,
            &mut Vec::new(),
        )
    }
}

/// The identifiers of the records written one at a time: see
/// [`GedcomStreamWriter`].
pub(crate) struct StreamXrefs {
    rules: &'static VersionRules,
    /// Every record identifier written.
    written: XrefIndex<'static, ()>,
    /// The identifier each invalid identifier was first written as.
    renamed: HashMap<Box<str>, Box<str>>,
    /// The last number given to records without an identifier, by prefix.
    counters: HashMap<&'static str, usize>,
}

impl StreamXrefs {
    fn new(rules: &'static VersionRules) -> Self {
        Self {
            rules,
            written: XrefIndex::default(),
            renamed: HashMap::new(),
            counters: HashMap::new(),
        }
    }

    /// Notes `xref` as written.
    fn take(&mut self, xref: &str) {
        self.written.insert(Cow::Owned(xref.to_string()), ());
    }

    /// `candidate`, or `candidate` with the first `_n` suffix not written:
    /// taken.
    fn unique(&mut self, candidate: &str) -> String {
        let mut new = candidate.to_string();
        let mut n = 1_usize;
        while self.written.contains_key(&new) {
            n += 1;
            new = numbered_xref(self.rules, candidate, n);
        }
        self.take(&new);
        new
    }

    /// The first `@<prefix>n@` not written: taken.
    pub(crate) fn fresh(&mut self, prefix: &'static str) -> String {
        let n = self.counters.entry(prefix).or_insert(0);
        let mut next = *n;
        let new = loop {
            next += 1;
            let candidate = format!("@{prefix}{next}@");
            if !self.written.contains_key(&candidate) {
                break candidate;
            }
        };
        *n = next;
        self.take(&new);
        new
    }

    /// The identifier of a record with the identifier `xref`, of a kind
    /// whose generated identifiers start with `prefix`: kept when it is
    /// valid and not written yet, renamed otherwise (with the repair).
    fn record(
        &mut self,
        xref: Option<&str>,
        prefix: Option<&'static str>,
    ) -> (Option<String>, Option<Repair>) {
        let Some(x) = xref else {
            return (prefix.map(|p| self.fresh(p)), None);
        };
        let valid = self.rules.is_valid_xref(x);
        if valid && !self.written.contains_key(x) {
            self.take(x);
            return (Some(x.to_string()), None);
        }
        let new = self.unique(&new_xref(self.rules, x));
        if !valid && !self.renamed.contains_key(x) {
            self.renamed.insert(x.into(), new.as_str().into());
        }
        let repair = Repair::new(
            0,
            RepairKind::Xref,
            format!("identifier {x} written as {new}"),
        );
        (Some(new), Some(repair))
    }
}

impl Pointers for StreamXrefs {
    fn pointer<'p>(&mut self, pointer: &'p str) -> Cow<'p, str> {
        if self.rules.xref == XrefGrammar::V7 && pointer == "@VOID@" {
            return Cow::Borrowed(pointer);
        }
        if let Some(new) = self.renamed.get(pointer) {
            return Cow::Owned(new.to_string());
        }
        if is_valid_pointer(self.rules, pointer) {
            return Cow::Borrowed(pointer);
        }
        Cow::Owned(candidate_xref(self.rules, pointer))
    }
}
