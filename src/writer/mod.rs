//! Writing GEDCOM files: conformant GEDCOM 5.5.1, 7.0 or 7.1 lines.
//!
//! [`GedcomWriter`] writes a [`GedcomData`] model, or a lossless
//! [`Tree`](crate::tree::Tree) of structures, in a target version. Every
//! line goes through one emitter that applies the line rules of the
//! version ([`VersionRules`]):
//!
//! | Rule | GEDCOM 5.5.1 | GEDCOM 7.x |
//! |---|---|---|
//! | Line length | 255 characters, the whole line and its terminator; longer payloads continue with `CONC` | No limit, no `CONC` |
//! | Line breaks in text | `CONT` (from LF, CR LF or CR) | `CONT` |
//! | `@` in text | Every `@` doubled, `@#…@` escapes kept | A leading `@` doubled |
//! | Identifiers | A letter or digit then visible ASCII, at most 22 characters | `@[A-Z0-9_]+@`, never `@VOID@` |
//! | Header | `GEDC.VERS 5.5.1`, `GEDC.FORM LINEAGE-LINKED`, `CHAR` of the output encoding, `SOUR`, `SUBM` | `GEDC.VERS 7.0` (or `7.1`); no `FORM`, no `CHAR` |
//!
//! In both versions every line, the last one included, ends with the
//! terminator, record identifiers are unique, identifiers appear on records
//! only, tags follow the version's grammar and banned control characters
//! are not written. Pointers are written only from pointer fields, never
//! guessed from text.
//!
//! Data that does not fit these rules is rewritten into them (an invalid
//! identifier renamed, a tag made an extension tag, a control character
//! removed, …) and each rewrite is reported as a [`Repair`] in the
//! [`WriteReport`]; [`RepairPolicy::Error`] makes the first one an error
//! instead.
//!
//! # Example
//!
//! ```rust
//! use ged_io::{GedcomBuilder, GedcomVersion, GedcomWriter};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let source = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NAME Ann /Example/\n0 TRLR\n";
//! let data = GedcomBuilder::new().build_from_str(source)?;
//!
//! let output = GedcomWriter::new()
//!     .gedcom_version(GedcomVersion::V7_0)
//!     .write_to_string(&data)?;
//! assert!(output.starts_with("0 HEAD\n1 GEDC\n2 VERS 7.0\n"));
//! assert!(output.ends_with("0 TRLR\n"));
//!
//! // Or straight into any `io::Write`, with a byte order mark in 7.x.
//! let mut bytes = Vec::new();
//! let report = GedcomWriter::new().write(&mut bytes, &data)?;
//! assert!(report.repairs.is_empty());
//! # Ok(())
//! # }
//! ```

mod emit;
mod head;
mod model;
mod xref;

use std::fmt;
use std::io;

use crate::tree::{Node, PayloadRef, Structure, Tree};
use crate::types::GedcomData;
use crate::version::{GedcomVersion, VersionRules};
use emit::{emit, LineSink};
use xref::XrefMap;

pub(crate) use emit::{extension_tag, new_xref};
pub(crate) use head::{complete as complete_head, needs_submitter, stub_submitter, PLACEHOLDER};
pub(crate) use xref::numbered_xref;

/// The line terminator of written files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LineEnding {
    /// LF (`\n`), the default.
    #[default]
    Lf,
    /// CR LF (`\r\n`).
    CrLf,
    /// CR (`\r`).
    Cr,
}

impl LineEnding {
    /// The terminator's characters.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
            LineEnding::Cr => "\r",
        }
    }
}

/// Whether written bytes start with a byte order mark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Bom {
    /// The default: a mark for GEDCOM 7.x (which recommends one, §1.1) and
    /// for UTF-16 output, none otherwise. [`GedcomWriter::write_to_string`]
    /// never writes one in this mode.
    #[default]
    Auto,
    /// Always a mark (none for the single-byte encodings, which have none).
    Always,
    /// Never a mark.
    Never,
}

/// The character encoding of written bytes, named by the 5.5.1 `HEAD.CHAR`.
///
/// GEDCOM 7.x is always UTF-8: the other encodings apply to 5.5.1 output
/// only, and a 7.x file is written in UTF-8 whatever this says.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum OutputEncoding {
    /// UTF-8 (`CHAR UTF-8`), the default.
    #[default]
    Utf8,
    /// UTF-16 little endian (`CHAR UNICODE`).
    Utf16Le,
    /// ANSEL (`CHAR ANSEL`).
    Ansel,
    /// ASCII (`CHAR ASCII`).
    Ascii,
}

impl OutputEncoding {
    /// The `HEAD.CHAR` payload naming this encoding in GEDCOM 5.5.1.
    #[must_use]
    pub fn char_label(self) -> &'static str {
        match self {
            OutputEncoding::Utf8 => "UTF-8",
            OutputEncoding::Utf16Le => "UNICODE",
            OutputEncoding::Ansel => "ANSEL",
            OutputEncoding::Ascii => "ASCII",
        }
    }
}

impl fmt::Display for OutputEncoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.char_label())
    }
}

/// What the writer does with data that does not fit the target version.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RepairPolicy {
    /// Rewrite it into a conformant form and report a [`Repair`] (the
    /// default).
    #[default]
    Repair,
    /// Fail with [`WriteError::NonConformant`] on the first repair.
    Error,
}

/// A change made so that written data conforms to its version.
///
/// The writer's line emitter reports its rewrites with this type (row (h)
/// of the conformance repair table: identifiers, banned characters, and
/// tags and levels outside the line grammar); the structure-level rows
/// are listed in [`RepairKind`].
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Repair {
    /// The 1-based source line of the structure changed; 0 for a structure
    /// that was not read from a file (typed data), or was synthesised.
    pub line: u32,
    /// The kind of repair: its row of the repair table.
    pub kind: RepairKind,
    /// What was changed, in words (with the output line for line repairs).
    pub detail: Box<str>,
}

impl Repair {
    pub(crate) fn new(line: u32, kind: RepairKind, detail: impl Into<Box<str>>) -> Self {
        Self {
            line,
            kind,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Repair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            f.write_str(&self.detail)
        } else {
            write!(f, "line {}: {}", self.line, self.detail)
        }
    }
}

/// The row of the conformance repair table a [`Repair`] applies.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RepairKind {
    /// (a) An enumeration value not in its set.
    EnumValue,
    /// (b) An enumeration value in the wrong case.
    EnumCase,
    /// (c) A pointer missing, misplaced, dangling or to the wrong record type.
    Pointer,
    /// (d) A payload outside its type's grammar.
    Payload,
    /// (e) A structure not permitted where it is, or a malformed tag (also a
    /// structure tagged `CONT`, `CONC`, or `HEAD`/`TRLR` as a record).
    Misplaced,
    /// (f) More occurrences than permitted.
    Repeated,
    /// (g) A required substructure synthesised, or its superstructure made an
    /// extension when none can be.
    Required,
    /// (g) A 7.x structure with neither payload nor substructure.
    Empty,
    /// (h) An identifier renamed or left out.
    Xref,
    /// (h) Banned characters left out (5.5.1 tabs written as spaces).
    Characters,
    /// (h) A structure nested deeper than the version allows, written at
    /// the deepest level.
    Level,
    /// (i) The header or the trailer placed, completed or rebuilt.
    Header,
    /// (j) Text moved out of a 5.5.1 record over 32K.
    RecordSize,
}

/// What a write did besides writing: the repairs it made.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WriteReport {
    /// Every repair, in output order (identifier renames first).
    pub repairs: Vec<Repair>,
}

/// Why a write failed. Its variants are boxed, so that every write call
/// returns a small `Result`.
#[non_exhaustive]
#[derive(Debug)]
pub enum WriteError {
    /// The underlying writer failed.
    Io(io::Error),
    /// The data needed a repair and the policy is [`RepairPolicy::Error`].
    NonConformant(Box<Repair>),
    /// The output encoding cannot represent a character of the data.
    Unencodable(Box<Unencodable>),
}

/// A character the output encoding cannot represent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unencodable {
    /// The output encoding.
    pub encoding: OutputEncoding,
    /// The first character it cannot represent.
    pub character: char,
    /// The output line.
    pub line: u64,
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WriteError::Io(e) => write!(f, "write failed: {e}"),
            WriteError::NonConformant(repair) => write!(f, "non-conformant data: {repair}"),
            WriteError::Unencodable(u) => write!(
                f,
                "line {}: {} cannot represent {:?} (U+{:04X})",
                u.line,
                u.encoding,
                u.character,
                u32::from(u.character)
            ),
        }
    }
}

impl std::error::Error for WriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WriteError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for WriteError {
    fn from(e: io::Error) -> Self {
        WriteError::Io(e)
    }
}

/// Configuration of a [`GedcomWriter`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriterConfig {
    /// The version to write; `None` (the default) writes the version the
    /// data declares in `HEAD.GEDC.VERS`, 5.5.1 when it declares none.
    pub version: Option<GedcomVersion>,
    /// The line terminator (default LF).
    pub line_ending: LineEnding,
    /// The longest 5.5.1 line, counting the level, identifier, tag,
    /// payload, delimiters and terminator, in bytes (an upper bound of its
    /// characters). Default and maximum 255; 7.x has no limit.
    pub max_line_length: usize,
    /// Whether to write a byte order mark.
    pub bom: Bom,
    /// The encoding of the bytes [`GedcomWriter::write`] produces, which
    /// `HEAD.CHAR` names (5.5.1 only).
    pub output_encoding: OutputEncoding,
    /// What to do with data that does not fit the target version.
    pub on_nonconformant: RepairPolicy,
}

impl Default for WriterConfig {
    fn default() -> Self {
        Self {
            version: None,
            line_ending: LineEnding::Lf,
            max_line_length: 255,
            bom: Bom::Auto,
            output_encoding: OutputEncoding::Utf8,
            on_nonconformant: RepairPolicy::Repair,
        }
    }
}

/// Writes GEDCOM files. See the [module documentation](self).
#[derive(Clone, Debug, Default)]
pub struct GedcomWriter {
    config: WriterConfig,
}

/// One line-level structure sink: the emitter, or a capture of structures.
pub(crate) trait Out {
    /// Writes one structure (its substructures follow at deeper levels).
    fn put(
        &mut self,
        level: usize,
        xref: Option<&str>,
        tag: &str,
        payload: PayloadRef<'_>,
    ) -> Result<(), WriteError>;
}

/// The emitter as an [`Out`].
struct Emitter<'s, 'w> {
    rules: &'static VersionRules,
    sink: &'s mut LineSink<'w>,
}

impl Out for Emitter<'_, '_> {
    fn put(
        &mut self,
        level: usize,
        xref: Option<&str>,
        tag: &str,
        payload: PayloadRef<'_>,
    ) -> Result<(), WriteError> {
        emit(self.rules, self.sink, level, xref, tag, payload)
    }
}

/// Collects structures instead of writing them (for the header, which is
/// completed before it is written).
#[derive(Default)]
struct Capture {
    roots: Vec<Structure>,
}

impl Out for Capture {
    fn put(
        &mut self,
        level: usize,
        xref: Option<&str>,
        tag: &str,
        payload: PayloadRef<'_>,
    ) -> Result<(), WriteError> {
        let node = Structure {
            xref: xref.map(Into::into),
            payload: payload.to_payload(),
            ..Structure::new(tag)
        };
        let mut siblings = &mut self.roots;
        for _ in 0..level {
            if siblings.is_empty() {
                break;
            }
            let last = siblings.len() - 1;
            siblings = &mut siblings[last].substructures;
        }
        siblings.push(node);
        Ok(())
    }
}

impl GedcomWriter {
    /// A writer with the default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A writer with `config`.
    #[must_use]
    pub fn with_config(config: WriterConfig) -> Self {
        Self { config }
    }

    /// Sets the version to write, whatever the data declares.
    #[must_use]
    pub fn gedcom_version(mut self, version: GedcomVersion) -> Self {
        self.config.version = Some(version);
        self
    }

    /// Sets the line terminator.
    #[must_use]
    pub fn line_ending(mut self, ending: LineEnding) -> Self {
        self.config.line_ending = ending;
        self
    }

    /// Sets the longest 5.5.1 line, the whole line and its terminator
    /// counted; values above 255 are taken as 255.
    #[must_use]
    pub fn max_line_length(mut self, length: usize) -> Self {
        self.config.max_line_length = length;
        self
    }

    /// Sets whether to write a byte order mark.
    #[must_use]
    pub fn bom(mut self, bom: Bom) -> Self {
        self.config.bom = bom;
        self
    }

    /// Sets the encoding of the bytes [`write`](Self::write) produces.
    #[must_use]
    pub fn output_encoding(mut self, encoding: OutputEncoding) -> Self {
        self.config.output_encoding = encoding;
        self
    }

    /// Sets what to do with data that does not fit the target version.
    #[must_use]
    pub fn on_nonconformant(mut self, policy: RepairPolicy) -> Self {
        self.config.on_nonconformant = policy;
        self
    }

    /// The configuration.
    #[must_use]
    pub fn config(&self) -> &WriterConfig {
        &self.config
    }

    /// The rules of the version `data` is written in.
    fn rules_for(&self, declared: Option<&str>) -> &'static VersionRules {
        self.config
            .version
            .or_else(|| declared.map(GedcomVersion::from_version_str))
            .unwrap_or_default()
            .rules()
    }

    /// A sink for `rules`: the configured terminator, line length and
    /// repair policy, and `encoding`.
    fn sink<'w>(
        &self,
        writer: Option<&'w mut dyn io::Write>,
        rules: &VersionRules,
        encoding: OutputEncoding,
    ) -> LineSink<'w> {
        let max_line = rules
            .max_line_len
            .map(|max| max.min(self.config.max_line_length));
        LineSink::new(
            writer,
            self.config.line_ending.as_str(),
            max_line,
            encoding,
            self.config.on_nonconformant,
        )
    }

    /// The encoding of output bytes for `rules`.
    fn encoding(&self, rules: &VersionRules) -> OutputEncoding {
        if rules.head_char {
            self.config.output_encoding
        } else {
            OutputEncoding::Utf8
        }
    }

    fn wants_bom(&self, rules: &VersionRules, encoding: OutputEncoding, to_string: bool) -> bool {
        match self.config.bom {
            Bom::Always => true,
            Bom::Never => false,
            Bom::Auto => !to_string && (!rules.head_char || encoding == OutputEncoding::Utf16Le),
        }
    }

    /// Writes `data` to `writer` in the configured encoding and returns the
    /// repairs made.
    ///
    /// # Errors
    ///
    /// [`WriteError::Io`] when `writer` fails, [`WriteError::Unencodable`]
    /// when the output encoding cannot represent the data, and
    /// [`WriteError::NonConformant`] on the first repair under
    /// [`RepairPolicy::Error`].
    pub fn write<W: io::Write>(
        &self,
        mut writer: W,
        data: &GedcomData,
    ) -> Result<WriteReport, WriteError> {
        let rules = self.rules_for(data.gedcom_version());
        let encoding = self.encoding(rules);
        let mut sink = self.sink(Some(&mut writer), rules, encoding);
        if self.wants_bom(rules, encoding, false) {
            sink.bom();
        }
        model::write_data(rules, &mut sink, data)?;
        sink.finish()?;
        Ok(WriteReport {
            repairs: sink.repairs,
        })
    }

    /// Writes `data` as text: UTF-8 (whatever the output encoding), labelled
    /// `CHAR UTF-8` in 5.5.1, with a byte order mark only under
    /// [`Bom::Always`].
    ///
    /// # Errors
    ///
    /// [`WriteError::NonConformant`] on the first repair under
    /// [`RepairPolicy::Error`].
    pub fn write_to_string(&self, data: &GedcomData) -> Result<String, WriteError> {
        let rules = self.rules_for(data.gedcom_version());
        let mut sink = self.sink(None, rules, OutputEncoding::Utf8);
        if self.wants_bom(rules, OutputEncoding::Utf8, true) {
            sink.bom();
        }
        model::write_data(rules, &mut sink, data)?;
        Ok(sink.into_string())
    }

    /// Writes a lossless [`Tree`] to `writer` in the target version (the
    /// tree's own version unless one is configured): structures that the
    /// version does not permit as they are are first repaired by
    /// [`spec::conform`](crate::spec::conform) (as extension structures,
    /// keeping their data), then every line follows the line rules: header
    /// completed, identifiers valid and unique, text escaped and continued,
    /// a final `TRLR`. The report lists the structural repairs, then the
    /// line repairs.
    ///
    /// # Errors
    ///
    /// As [`write`](Self::write).
    pub fn write_tree<W: io::Write>(
        &self,
        mut writer: W,
        tree: &Tree,
    ) -> Result<WriteReport, WriteError> {
        let rules = self.rules_for(tree.declared_version());
        self.write_conformed(&mut writer, rules, tree.to_structures())
    }

    /// Writes owned records — such as [`Tree::to_structures`] gives — like
    /// [`write_tree`](Self::write_tree). With no version configured, they
    /// are written in the version their `HEAD.GEDC.VERS` declares.
    ///
    /// # Errors
    ///
    /// As [`write`](Self::write).
    pub fn write_structures<W: io::Write>(
        &self,
        mut writer: W,
        records: &[Structure],
    ) -> Result<WriteReport, WriteError> {
        let rules = self.rules_for_records(records);
        self.write_conformed(&mut writer, rules, records.to_vec())
    }

    /// The version owned records are written in: the configured one, else
    /// the one their `HEAD.GEDC.VERS` declares.
    fn rules_for_records(&self, records: &[Structure]) -> &'static VersionRules {
        let declared = records
            .iter()
            .find(|r| r.tag == "HEAD")
            .and_then(|h| h.first("GEDC"))
            .and_then(|g| g.first("VERS"))
            .and_then(Structure::text);
        self.rules_for(declared)
    }

    /// [`write_structures`](Self::write_structures), taking the records.
    pub(crate) fn write_owned<W: io::Write>(
        &self,
        mut writer: W,
        records: Vec<Structure>,
    ) -> Result<WriteReport, WriteError> {
        let rules = self.rules_for_records(&records);
        self.write_conformed(&mut writer, rules, records)
    }

    /// Writes owned records as text, as
    /// [`write_to_string`](Self::write_to_string) writes a dataset.
    pub(crate) fn write_owned_to_string(
        &self,
        mut records: Vec<Structure>,
    ) -> Result<(String, WriteReport), WriteError> {
        let rules = self.rules_for_records(&records);
        let mut repairs = crate::spec::conform(&mut records, rules.version);
        if self.config.on_nonconformant == RepairPolicy::Error {
            if let Some(first) = repairs.into_iter().next() {
                return Err(WriteError::NonConformant(Box::new(first)));
            }
            repairs = Vec::new();
        }
        let mut sink = self.sink(None, rules, OutputEncoding::Utf8);
        if self.wants_bom(rules, OutputEncoding::Utf8, true) {
            sink.bom();
        }
        write_records(rules, &mut sink, records.iter())?;
        repairs.append(&mut sink.repairs);
        Ok((sink.into_string(), WriteReport { repairs }))
    }

    /// Repairs `records` for `rules` ([`crate::spec::conform`]), then writes
    /// them.
    fn write_conformed(
        &self,
        writer: &mut dyn io::Write,
        rules: &'static VersionRules,
        mut records: Vec<Structure>,
    ) -> Result<WriteReport, WriteError> {
        let repairs = crate::spec::conform(&mut records, rules.version);
        if self.config.on_nonconformant == RepairPolicy::Error {
            if let Some(first) = repairs.into_iter().next() {
                return Err(WriteError::NonConformant(Box::new(first)));
            }
            return self.write_nodes(writer, rules, records.iter());
        }
        let mut report = self.write_nodes(writer, rules, records.iter())?;
        report.repairs.splice(0..0, repairs);
        Ok(report)
    }

    fn write_nodes<'n, N, I>(
        &self,
        writer: &mut dyn io::Write,
        rules: &'static VersionRules,
        records: I,
    ) -> Result<WriteReport, WriteError>
    where
        N: Node<'n>,
        I: Iterator<Item = N> + Clone,
    {
        let encoding = self.encoding(rules);
        let mut sink = self.sink(Some(writer), rules, encoding);
        if self.wants_bom(rules, encoding, false) {
            sink.bom();
        }
        write_records(rules, &mut sink, records)?;
        sink.finish()?;
        Ok(WriteReport {
            repairs: sink.repairs,
        })
    }
}

/// What a level-0 structure of a tree is to the writer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    /// The first `HEAD`, completed and written first.
    Head,
    /// An empty `TRLR`, replaced by the writer's own.
    Trailer,
    /// Any other record, written in order.
    Record,
}

fn roles<'n, N: Node<'n>>(
    records: impl Iterator<Item = N> + Clone,
) -> impl Iterator<Item = (Role, N)> + Clone {
    let mut head_seen = false;
    records.map(move |r| {
        let role = match r.tag() {
            "HEAD" if !head_seen => {
                head_seen = true;
                Role::Head
            }
            "TRLR" if r.payload() == PayloadRef::None && r.children().next().is_none() => {
                Role::Trailer
            }
            _ => Role::Record,
        };
        (role, r)
    })
}

/// The prefix of the identifiers generated for a standard record without
/// one; `None` for other records, which need none.
fn record_prefix(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "INDI" => "I",
        "FAM" => "F",
        "SOUR" => "S",
        "REPO" => "R",
        "OBJE" => "M",
        "SUBM" => "U",
        "SUBN" => "SUBN",
        "NOTE" | "SNOTE" => "N",
        _ => return None,
    })
}

/// Writes the records of a tree: the completed header, the records in
/// order (identifiers mapped), then `TRLR`.
fn write_records<'n, N, I>(
    rules: &'static VersionRules,
    sink: &mut LineSink<'_>,
    records: I,
) -> Result<(), WriteError>
where
    N: Node<'n>,
    I: Iterator<Item = N> + Clone,
{
    let others = roles(records.clone())
        .filter(|(role, _)| *role == Role::Record)
        .map(|(_, r)| r);
    let mut xrefs = XrefMap::new(
        rules,
        others.clone().map(|r| (record_prefix(r.tag()), r.xref())),
    );
    for repair in std::mem::take(&mut xrefs.repairs) {
        sink.repair(repair)?;
    }

    let mut head = roles(records)
        .find(|(role, _)| *role == Role::Head)
        .map_or_else(|| Structure::new("HEAD"), |(_, h)| h.to_owned_structure());
    map_pointers(&mut head, &mut xrefs);
    let mut stub = None;
    let submitter = if head::needs_submitter(&head, rules) {
        let first = others
            .clone()
            .enumerate()
            .find(|(_, r)| r.tag() == "SUBM")
            .and_then(|(i, r)| xrefs.record(i, r.xref()).map(str::to_string));
        Some(first.unwrap_or_else(|| {
            let xref = xrefs.fresh("U");
            stub = Some(head::stub_submitter(xref.clone()));
            xref
        }))
    } else {
        None
    };
    head::complete(&mut head, rules, sink.charset(), submitter);
    let mut out = Emitter { rules, sink };
    put_structure(&mut out, &head)?;
    if let Some(stub) = stub {
        put_structure(&mut out, &stub)?;
    }

    for (index, record) in others.enumerate() {
        let xref = xrefs.record(index, record.xref()).map(Box::<str>::from);
        out.sink.source_line = record.line();
        let tag = match record.tag() {
            "HEAD" | "TRLR" => {
                let to = format!("_{}", record.tag());
                let detail = format!(
                    "output line {}: record {} written as {to}",
                    out.sink.line_no(),
                    record.tag()
                );
                out.sink
                    .repair(Repair::new(record.line(), RepairKind::Misplaced, detail))?;
                std::borrow::Cow::Owned(to)
            }
            tag => std::borrow::Cow::Borrowed(tag),
        };
        out.sink.source_line = record.line();
        put_node(
            &mut out,
            &mut xrefs,
            0,
            xref.as_deref(),
            &tag,
            record.payload(),
        )?;
        // The substructures, depth first, without recursion.
        let mut stack = vec![(1_usize, record.children())];
        while let Some((level, children)) = stack.last_mut() {
            let level = *level;
            let Some(child) = children.next() else {
                stack.pop();
                continue;
            };
            out.sink.source_line = child.line();
            put_node(
                &mut out,
                &mut xrefs,
                level,
                child.xref(),
                child.tag(),
                child.payload(),
            )?;
            stack.push((level + 1, child.children()));
        }
    }
    out.sink.source_line = 0;
    out.put(0, None, "TRLR", PayloadRef::None)
}

/// Writes one structure with its pointer mapped.
fn put_node(
    out: &mut dyn Out,
    xrefs: &mut XrefMap<'_>,
    level: usize,
    xref: Option<&str>,
    tag: &str,
    payload: PayloadRef<'_>,
) -> Result<(), WriteError> {
    match payload {
        PayloadRef::Pointer(p) => {
            let p = xrefs.pointer(p);
            out.put(level, xref, tag, PayloadRef::Pointer(&p))
        }
        payload => out.put(level, xref, tag, payload),
    }
}

/// Writes an owned structure and its substructures, as they are.
fn put_structure(out: &mut dyn Out, root: &Structure) -> Result<(), WriteError> {
    out.put(
        0,
        root.xref.as_deref(),
        root.tag.as_str(),
        root.payload.borrowed(),
    )?;
    let mut stack = vec![(1_usize, root.substructures.iter())];
    while let Some((level, children)) = stack.last_mut() {
        let level = *level;
        let Some(child) = children.next() else {
            stack.pop();
            continue;
        };
        out.put(
            level,
            child.xref.as_deref(),
            child.tag.as_str(),
            child.payload.borrowed(),
        )?;
        stack.push((level + 1, child.substructures.iter()));
    }
    Ok(())
}

/// Maps every pointer of an owned structure.
fn map_pointers(root: &mut Structure, xrefs: &mut XrefMap<'_>) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if let crate::tree::Payload::Pointer(p) = &node.payload {
            let mapped = xrefs.pointer(p.as_str()).into_owned();
            node.payload = crate::tree::Payload::Pointer(mapped.into());
        }
        stack.extend(node.substructures.iter_mut());
    }
}
