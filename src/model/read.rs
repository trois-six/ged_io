//! Reading decoded text into the typed model, one record at a time.
//!
//! Every reader — [`Dataset::parse`], the
//! [`GedcomBuilder`](crate::GedcomBuilder), the
//! [`GedcomStreamParser`](crate::GedcomStreamParser) and GEDZIP archives —
//! lexes records into the arena of [`crate::tree`] and types each one
//! through [`dispatch_record`], the one place that knows which type a
//! level-0 tag stands for.

use crate::tree::{head_version, segment_bounds, Builder, Escaping, TagInterner};
use crate::version::GedcomVersion;

use super::dataset::{Dataset, Record};
use super::driver::{Arena, FromNode, NodeRef, ReadCx};
use super::header::Header;
use super::node::Node;
use super::record::{
    Family, Individual, Multimedia, Repository, SharedNote, Source, Submission, Submitter,
};
use super::text::Store;

/// The largest text one pass of the lexer indexes: offsets are 32-bit, and
/// a piece's length takes 31 bits.
const SEGMENT_LIMIT: usize = (u32::MAX / 2) as usize;

/// Where [`dispatch_record`] puts the records it types.
pub(crate) trait RecordSink {
    /// Whether a header is already kept: a second one stays untyped.
    fn has_header(&self) -> bool;
    /// Takes a typed record.
    fn typed(&mut self, record: Typed);
    /// Takes a record of no type the model has, or that did not fit its
    /// type.
    fn other(&mut self, node: Node);
}

/// A record typed by [`dispatch_record`], before it is placed.
// Each value is moved once, from the reader into its list: boxing the
// larger variants would allocate once more per record.
#[allow(clippy::large_enum_variant)]
pub(crate) enum Typed {
    Header(Header),
    Individual(Individual),
    Family(Family),
    Source(Source),
    Repository(Repository),
    Multimedia(Multimedia),
    Submitter(Submitter),
    Submission(Submission),
    Note(SharedNote),
}

/// Types the record at `node` by its tag and hands it to `sink`.
///
/// A record whose tag the model has no type for (an extension or unknown
/// record, a header after the first, a trailer with content), or that does
/// not fit its type (`0 INDI payload`), is handed over untyped, whole. An
/// empty trailer carries nothing and is dropped: writers end every file
/// with their own.
pub(crate) fn dispatch_record(node: NodeRef<'_>, cx: &mut ReadCx<'_>, sink: &mut dyn RecordSink) {
    let typed = match node.standard_tag() {
        Some("HEAD") if !sink.has_header() => Header::from_node(node, cx).map(Typed::Header),
        Some("INDI") => Individual::from_node(node, cx).map(Typed::Individual),
        Some("FAM") => Family::from_node(node, cx).map(Typed::Family),
        Some("SOUR") => Source::from_node(node, cx).map(Typed::Source),
        Some("REPO") => Repository::from_node(node, cx).map(Typed::Repository),
        Some("OBJE") => Multimedia::from_node(node, cx).map(Typed::Multimedia),
        Some("SUBM") => Submitter::from_node(node, cx).map(Typed::Submitter),
        Some("SUBN") => Submission::from_node(node, cx).map(Typed::Submission),
        Some("NOTE" | "SNOTE") => SharedNote::from_node(node, cx).map(Typed::Note),
        Some("TRLR") if !node.has_children() && node.has_no_payload() && !node.has_xref() => {
            return;
        }
        _ => None,
    };
    if let Some(record) = typed {
        sink.typed(record);
    } else {
        let n = cx.node(node);
        sink.other(n);
    }
}

impl RecordSink for Dataset {
    fn has_header(&self) -> bool {
        self.header.is_some()
    }

    fn typed(&mut self, record: Typed) {
        match record {
            Typed::Header(r) => self.header = Some(r),
            Typed::Individual(r) => self.individuals.push(r),
            Typed::Family(r) => self.families.push(r),
            Typed::Source(r) => self.sources.push(r),
            Typed::Repository(r) => self.repositories.push(r),
            Typed::Multimedia(r) => self.multimedia.push(r),
            Typed::Submitter(r) => self.submitters.push(r),
            Typed::Submission(r) => self.submissions.push(r),
            Typed::Note(r) => self.notes.push(r),
        }
    }

    fn other(&mut self, node: Node) {
        self.extra.push(node);
    }
}

impl From<Typed> for Record {
    fn from(record: Typed) -> Self {
        match record {
            Typed::Header(r) => Record::Header(Box::new(r)),
            Typed::Individual(r) => Record::Individual(Box::new(r)),
            Typed::Family(r) => Record::Family(Box::new(r)),
            Typed::Source(r) => Record::Source(Box::new(r)),
            Typed::Repository(r) => Record::Repository(Box::new(r)),
            Typed::Multimedia(r) => Record::Multimedia(Box::new(r)),
            Typed::Submitter(r) => Record::Submitter(Box::new(r)),
            Typed::Submission(r) => Record::Submission(Box::new(r)),
            Typed::Note(r) => Record::Note(Box::new(r)),
        }
    }
}

/// The version a file declares, and how its text escapes `@`.
fn version_of(declared: Option<&str>) -> (GedcomVersion, Escaping) {
    let version = declared.map_or(GedcomVersion::V5_5_1, GedcomVersion::from_version_str);
    (version, Escaping::of(declared))
}

/// Reads a dataset from decoded text, keeping the text as its store.
pub(crate) fn read_string(text: String) -> Dataset {
    read_segmented(text, SEGMENT_LIMIT)
}

/// [`read_string`], lexing at most `limit` bytes at a time.
pub(crate) fn read_segmented(text: String, limit: usize) -> Dataset {
    let declared = head_version(&text);
    let (version, escaping) = version_of(declared.as_deref());
    let mut data = Dataset::new(version);
    data.declared_version = declared.map(Into::into);
    let mut store = Store::new(text);
    let mut tags = TagInterner::default();
    {
        let (input, pieces, xrefs) = store.parts_mut();
        let mut cx = ReadCx::new(input, pieces, xrefs, version);
        let mut builder = Builder::new(escaping, 1).with_spans();
        for (start, end, first_line) in segment_bounds(input, limit) {
            let part = input.get(start..end).unwrap_or_default();
            builder.reset(first_line);
            builder.read_records(part, &mut tags, |b, _| {
                let arena = Arena {
                    text: part,
                    base: start,
                    joined: &b.joined,
                    nodes: &b.nodes,
                    xrefs: &b.xrefs,
                };
                if let Some(root) = NodeRef::root(&arena) {
                    dispatch_record(root, &mut cx, &mut data);
                }
            });
        }
    }
    store.set_tags(tags.others);
    store.shrink();
    data.store = store;
    data.shrink();
    data
}

/// Reads a file one record at a time, each into a store of its own: what
/// a stream reads.
pub(crate) struct RecordReader {
    builder: Builder,
    version: GedcomVersion,
    header_seen: bool,
}

impl RecordReader {
    /// A reader of a 5.5.1 file, until [`set_version`](Self::set_version)
    /// says otherwise.
    pub(crate) fn new() -> Self {
        Self {
            builder: Builder::new(Escaping::V551, 1).with_spans(),
            version: GedcomVersion::V5_5_1,
            header_seen: false,
        }
    }

    /// The version the file declares, and how it escapes `@`.
    pub(crate) fn set_version(&mut self, version: GedcomVersion, escaping: Escaping) {
        self.version = version;
        self.builder.set_escaping(escaping);
    }

    /// Reads the records of `text` (normally one; lines without a level
    /// before the first record form one of their own) into `out`, and
    /// returns the store they point into, which keeps `text`.
    pub(crate) fn read(&mut self, text: String, first_line: u32, out: &mut Vec<Record>) -> Store {
        struct Sink<'o> {
            header_seen: &'o mut bool,
            out: &'o mut Vec<Record>,
        }
        impl RecordSink for Sink<'_> {
            fn has_header(&self) -> bool {
                *self.header_seen
            }
            fn typed(&mut self, record: Typed) {
                *self.header_seen |= matches!(record, Typed::Header(_));
                self.out.push(record.into());
            }
            fn other(&mut self, node: Node) {
                self.out.push(Record::Other(Box::new(node)));
            }
        }

        let mut store = Store::new(text);
        let mut tags = TagInterner::default();
        {
            let (input, pieces, xrefs) = store.parts_mut();
            let mut cx = ReadCx::new(input, pieces, xrefs, self.version);
            let mut sink = Sink {
                header_seen: &mut self.header_seen,
                out,
            };
            self.builder.reset(first_line);
            self.builder.read_records(input, &mut tags, |b, _| {
                let arena = Arena {
                    text: input,
                    base: 0,
                    joined: &b.joined,
                    nodes: &b.nodes,
                    xrefs: &b.xrefs,
                };
                if let Some(root) = NodeRef::root(&arena) {
                    dispatch_record(root, &mut cx, &mut sink);
                }
            });
        }
        store.set_tags(tags.others);
        store.shrink();
        store
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NoteContent;

    #[test]
    fn segments_read_as_one_text() {
        let text = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NOTE a@@b\n2 CONT c\n1 BIRT\n2 DATE 1 JAN 1900\n2 PLAC Sampleton\n0 @I2@ INDI\n1 ASSO @I1@\n2 RELA Friend\n0 TRLR\n";
        let whole = Dataset::parse(text);
        // Segments of whole records (the largest is 75 bytes).
        for limit in [80, 120, 200] {
            let parts = read_segmented(text.to_owned(), limit);
            assert_eq!(parts.to_structures(), whole.to_structures(), "{limit}");
        }
        assert_eq!(
            whole
                .find_individual("@I2@")
                .map(|r| r.detail().associations.len()),
            Some(1)
        );
        let note = &whole.individuals[0].notes[0];
        assert!(matches!(&note.content, NoteContent::Text(t) if t.to_str(&whole) == "a@b\nc"));
    }

    #[test]
    fn records_of_one_text() {
        let mut reader = RecordReader::new();
        reader.set_version(GedcomVersion::V7_0, Escaping::V70);
        let mut out = Vec::new();
        reader.read("0 HEAD\n1 GEDC\n2 VERS 7.0\n".into(), 1, &mut out);
        assert!(matches!(out.as_slice(), [Record::Header(_)]), "{out:?}");
        out.clear();
        let store = reader.read(
            "0 @I1@ INDI\n1 NAME Ann /Example/\n1 _X y\n".into(),
            4,
            &mut out,
        );
        let [Record::Individual(indi)] = out.as_slice() else {
            panic!("{out:?}")
        };
        assert_eq!(store.xref(indi.xref.unwrap()), "@I1@");
        assert_eq!(store.tag(indi.extra[0].tag), "_X");
        out.clear();
        // A second header is kept untyped; an empty trailer, not at all.
        reader.read("0 HEAD\n".into(), 8, &mut out);
        assert!(matches!(out.as_slice(), [Record::Other(_)]), "{out:?}");
        out.clear();
        reader.read("0 TRLR\n".into(), 9, &mut out);
        assert!(out.is_empty());
    }
}
