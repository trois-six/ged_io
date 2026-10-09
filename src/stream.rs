//! Reading a file one record at a time, with memory bounded by the largest
//! record.
//!
//! [`GedcomStreamParser`] reads GEDCOM bytes of any encoding from any
//! [`BufRead`], with the reading rules of the in-memory reader, and yields
//! each record typed, as a [`StreamedRecord`]: the [`Record`] and the
//! [`Store`] its texts point into, which holds that record's text alone.
//! [`GedcomStreamParser::nodes`] yields lossless structures instead, as
//! [`TreeReader`] does.
//!
//! ```rust
//! use ged_io::model::RecordRef;
//! use ged_io::GedcomStreamParser;
//!
//! # fn main() -> Result<(), ged_io::GedcomError> {
//! let file: &[u8] = b"0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 NAME Ann /Example/\n0 @F1@ FAM\n0 TRLR\n";
//! let mut names = Vec::new();
//! for record in GedcomStreamParser::new(file)? {
//!     let record = record?;
//!     if let RecordRef::Individual(indi) = record.record() {
//!         names.extend(indi.full_name(&record));
//!     }
//! }
//! assert_eq!(names, ["Ann Example"]);
//! # Ok(())
//! # }
//! ```
//!
//! A dataset collects streamed records (`FromIterator`, `Extend`), so that
//! a stream can be filtered into a smaller dataset without reading the
//! whole file at once.

use std::collections::VecDeque;
use std::io::BufRead;

use crate::encoding::GedcomEncoding;
use crate::model::read::RecordReader;
use crate::model::relocate::{Relocate, Relocation};
use crate::model::{Dataset, Record, RecordRef, Store};
use crate::tree::{RecordSource, TreeReader};
use crate::version::GedcomVersion;
use crate::GedcomError;

/// A record read by a [`GedcomStreamParser`], with the store its texts and
/// identifiers resolve against.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamedRecord {
    record: Record,
    store: Store,
    version: GedcomVersion,
    line: u32,
}

impl StreamedRecord {
    /// The record.
    #[must_use]
    pub fn record(&self) -> RecordRef<'_> {
        self.record.record_ref()
    }

    /// The store the record's texts and identifiers resolve against.
    #[must_use]
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// The version of the file.
    #[must_use]
    pub fn version(&self) -> GedcomVersion {
        self.version
    }

    /// The line the record starts on.
    #[must_use]
    pub fn line(&self) -> u32 {
        self.line
    }

    /// The record and its store.
    #[must_use]
    pub fn into_parts(self) -> (Record, Store) {
        (self.record, self.store)
    }
}

impl AsRef<Store> for StreamedRecord {
    fn as_ref(&self) -> &Store {
        &self.store
    }
}

/// Reads GEDCOM bytes one record at a time: see the [module
/// documentation](self).
///
/// The input may be in any encoding the in-memory reader reads, with any
/// line terminators: it is decoded on the fly ([`DecodeReader`]), and each
/// record is read by the same rules and types as [`Dataset::parse`] reads
/// it. Memory is bounded by the largest record and a 64 KiB buffer.
///
/// Reading fails only when the input does: an I/O error ends the stream.
///
/// [`DecodeReader`]: crate::encoding::DecodeReader
pub struct GedcomStreamParser<R> {
    source: RecordSource<R>,
    reader: RecordReader,
    /// Records read from the last text and not yet yielded, with the line
    /// of that text.
    pending: VecDeque<(Record, u32)>,
    /// The store of the pending records.
    store: Option<Store>,
    done: bool,
}

impl<R: BufRead> GedcomStreamParser<R> {
    /// Starts reading; this reads the first 64 KiB to choose the decoding.
    ///
    /// # Errors
    ///
    /// [`GedcomError::Io`] when `reader` fails.
    pub fn new(reader: R) -> Result<Self, GedcomError> {
        Ok(Self {
            source: RecordSource::new(reader)?,
            reader: RecordReader::new(),
            pending: VecDeque::new(),
            store: None,
            done: false,
        })
    }

    /// The encoding the input is decoded with.
    #[must_use]
    pub fn encoding(&self) -> GedcomEncoding {
        self.source.encoding()
    }

    /// The version the file declares, once the record that tells it (the
    /// first one, normally `HEAD`) has been read.
    #[must_use]
    pub fn version(&self) -> GedcomVersion {
        self.source.version()
    }

    /// The `HEAD.GEDC.VERS` payload as written, once read.
    #[must_use]
    pub fn declared_version(&self) -> Option<&str> {
        self.source.declared_version()
    }

    /// The records not read yet, as lossless structures (see
    /// [`TreeReader`]) instead of typed records.
    #[must_use]
    pub fn nodes(self) -> TreeReader<R> {
        let version = self.source.version();
        let pending = match &self.store {
            Some(store) => self
                .pending
                .iter()
                .map(|(r, _)| r.record_ref().to_structure_in(store, version))
                .collect(),
            None => VecDeque::new(),
        };
        TreeReader::from_source(self.source, pending)
    }

    fn read_next(&mut self) -> Result<Option<StreamedRecord>, GedcomError> {
        loop {
            if let Some((record, line)) = self.pending.pop_front() {
                // The last record of a text takes its store; the others,
                // seldom any, a copy.
                let store = if self.pending.is_empty() {
                    self.store.take().unwrap_or_default()
                } else {
                    self.store.clone().unwrap_or_default()
                };
                return Ok(Some(StreamedRecord {
                    record,
                    store,
                    version: self.source.version(),
                    line,
                }));
            }
            let mut text = String::new();
            let Some(line) = self.source.next_record(&mut text)? else {
                return Ok(None);
            };
            self.reader
                .set_version(self.source.version(), self.source.escaping());
            let mut records = Vec::new();
            let store = self.reader.read(text, line, &mut records);
            self.pending.extend(records.into_iter().map(|r| (r, line)));
            self.store = Some(store);
        }
    }
}

impl<R: BufRead> Iterator for GedcomStreamParser<R> {
    type Item = Result<StreamedRecord, GedcomError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let next = self.read_next().transpose();
        if !matches!(next, Some(Ok(_))) {
            self.done = true;
        }
        next
    }
}

impl Extend<StreamedRecord> for Dataset {
    /// Adds streamed records to the dataset: their texts move into its
    /// store. The first header sets the dataset's version (an empty
    /// dataset's) and becomes its header; another is kept in `extra`.
    fn extend<I: IntoIterator<Item = StreamedRecord>>(&mut self, iter: I) {
        for streamed in iter {
            let StreamedRecord {
                mut record,
                store,
                version,
                ..
            } = streamed;
            {
                let mut r = Relocation::new(&store, &mut self.store);
                record.relocate(&mut r);
            }
            match record {
                Record::Header(header) if self.header.is_some() => {
                    let cx_version = self.version;
                    let node = header_as_node(&header, &mut self.store, cx_version);
                    self.extra.push(node);
                }
                Record::Header(header) => {
                    if self.record_count() == 0 {
                        self.version = version;
                    }
                    self.declared_version = header.declared_version(&self.store).map(Into::into);
                    self.header = Some(*header);
                }
                other => self.push(other),
            }
        }
    }
}

/// A header the dataset has already one of, as an untyped record.
fn header_as_node(
    header: &crate::model::Header,
    store: &mut Store,
    version: GedcomVersion,
) -> crate::model::Node {
    let structure = RecordRef::Header(header).to_structure_in(store, version);
    crate::model::Node::from_structure(&structure, store)
}

impl FromIterator<StreamedRecord> for Dataset {
    fn from_iter<I: IntoIterator<Item = StreamedRecord>>(iter: I) -> Self {
        let mut data = Dataset::default();
        data.extend(iter);
        data
    }
}

impl Relocate for Record {
    fn relocate(&mut self, r: &mut Relocation<'_>) {
        match self {
            Record::Header(x) => x.relocate(r),
            Record::Individual(x) => x.relocate(r),
            Record::Family(x) => x.relocate(r),
            Record::Source(x) => x.relocate(r),
            Record::Repository(x) => x.relocate(r),
            Record::Multimedia(x) => x.relocate(r),
            Record::Submitter(x) => x.relocate(r),
            Record::Submission(x) => x.relocate(r),
            Record::Note(x) => x.relocate(r),
            Record::Other(x) => x.relocate(r),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RecordRef;

    const FILE: &str = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NAME Ann /Example/\n1 NOTE a@@b\n2 CONC c\n1 _X y\n0 @F1@ FAM\n1 HUSB @I1@\n0 _LOC Here\n0 TRLR\n";

    fn stream(bytes: &[u8]) -> Vec<StreamedRecord> {
        GedcomStreamParser::new(bytes)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn records_typed_one_at_a_time() {
        let records = stream(FILE.as_bytes());
        assert_eq!(records.len(), 4);
        assert!(matches!(records[0].record(), RecordRef::Header(_)));
        let RecordRef::Individual(indi) = records[1].record() else {
            panic!()
        };
        assert_eq!(records[1].line(), 4);
        assert_eq!(indi.full_name(&records[1]).as_deref(), Some("Ann Example"));
        assert_eq!(records[1].store().tag(indi.extra[0].tag), "_X");
        assert!(matches!(records[3].record(), RecordRef::Other(_)));
    }

    #[test]
    fn collected_equals_read_in_memory() {
        let whole = Dataset::parse(FILE);
        let collected: Dataset = stream(FILE.as_bytes()).into_iter().collect();
        assert_eq!(collected.to_structures(), whole.to_structures());
        assert_eq!(collected.version(), whole.version());
        assert_eq!(collected.declared_version(), Some("5.5.1"));
        let indi = collected.find_individual("@I1@").unwrap();
        assert_eq!(collected.families_as_spouse(indi.xref).count(), 1);
    }

    #[test]
    fn any_encoding_and_terminator() {
        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain(
                FILE.replace('\n', "\r")
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes),
            )
            .collect();
        let records = stream(&utf16);
        assert_eq!(records.len(), 4);
        let RecordRef::Individual(indi) = records[1].record() else {
            panic!()
        };
        assert_eq!(indi.notes.len(), 1);
    }

    #[test]
    fn nodes_after_typed_records() {
        let mut parser = GedcomStreamParser::new(FILE.as_bytes()).unwrap();
        assert!(parser.next().is_some());
        let rest: Vec<_> = parser.nodes().collect::<Result<_, _>>().unwrap();
        let tags: Vec<_> = rest.iter().map(|s| s.tag.as_str().to_string()).collect();
        assert_eq!(tags, ["INDI", "FAM", "_LOC", "TRLR"]);
        assert_eq!(rest[0].first("NOTE").unwrap().text(), Some("a@bc"));
    }

    #[test]
    fn empty_and_trailer_only() {
        assert!(stream(b"").is_empty());
        assert!(stream(b"0 TRLR\n").is_empty());
    }

    #[test]
    fn io_errors_end_the_stream() {
        struct Failing;
        impl std::io::Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("broken"))
            }
        }
        let parser = GedcomStreamParser::new(std::io::BufReader::new(Failing));
        assert!(matches!(parser, Err(GedcomError::Io(_))));
    }
}
