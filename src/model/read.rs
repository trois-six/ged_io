//! Reading decoded text into the typed model, one record at a time.
//!
//! Every reader — [`Dataset::parse`], the
//! [`GedcomBuilder`](crate::GedcomBuilder), the
//! [`GedcomStreamParser`](crate::GedcomStreamParser) and GEDZIP archives —
//! lexes records into the arena of [`crate::tree`] and types each one
//! through [`dispatch_record`], the one place that knows which type a
//! level-0 tag stands for, into the tables of a dataset.

use crate::tree::{head_version, segment_bounds, Builder, Escaping, TagInterner};
use crate::version::GedcomVersion;

use super::dataset::Dataset;
use super::driver::{Arena, NodeRef, ReadCx, Scratch};
use super::edit::RecordKind;
use super::header::Header;
use super::record::{
    Family, Individual, Multimedia, Repository, SharedNote, Source, Submission, Submitter,
};
use super::table::HasTable;
use super::text::{Store, XrefId};

/// The largest text one pass of the lexer indexes: offsets are 32-bit, and
/// a piece's length takes 31 bits.
const SEGMENT_LIMIT: usize = (u32::MAX / 2) as usize;

/// What [`dispatch_record`] tells of the records it types into the
/// tables.
pub(crate) trait RecordSink {
    /// Whether a header is already kept: a second one stays untyped.
    fn has_header(&self) -> bool;
    /// Takes the header.
    fn header(&mut self, header: Header);
    /// A record was placed: the `pos`-th of its kind, with identifier
    /// `xref`.
    fn placed(&mut self, kind: RecordKind, pos: usize, xref: Option<XrefId>);
}

/// Types the record at `node` by its tag into the tables of `cx`, and
/// tells `sink`.
///
/// A record whose tag the model has no type for (an extension or unknown
/// record, a header after the first, a trailer with content), or that does
/// not fit its type (`0 INDI payload`), is kept untyped, whole. An empty
/// trailer carries nothing and is dropped: writers end every file with
/// their own.
pub(crate) fn dispatch_record(node: NodeRef<'_>, cx: &mut ReadCx<'_>, sink: &mut dyn RecordSink) {
    let typed = match node.standard_tag() {
        Some("HEAD") if !sink.has_header() => match read_header(node, cx) {
            Some(header) => {
                sink.header(header);
                true
            }
            None => false,
        },
        Some("INDI") => typed::<Individual>(node, cx, sink),
        Some("FAM") => typed::<Family>(node, cx, sink),
        Some("SOUR") => typed::<Source>(node, cx, sink),
        Some("REPO") => typed::<Repository>(node, cx, sink),
        Some("OBJE") => typed::<Multimedia>(node, cx, sink),
        Some("SUBM") => typed::<Submitter>(node, cx, sink),
        Some("SUBN") => typed::<Submission>(node, cx, sink),
        Some("NOTE" | "SNOTE") => typed::<SharedNote>(node, cx, sink),
        Some("TRLR") if !node.has_children() && node.has_no_payload() && !node.has_xref() => {
            return;
        }
        _ => false,
    };
    if !typed {
        let row = cx.node_row(node);
        let pos = cx.tables.others.rows.len();
        cx.tables.others.rows.push(row);
        sink.placed(RecordKind::Other, pos, row.xref);
    }
}

/// Reads the header, which a dataset keeps as its owned type: read into
/// the tables as any structure, then copied out. Its rows, a few, stay in
/// the tables, held by nothing (a compaction drops them).
fn read_header(node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> Option<Header> {
    super::table::read_one_row::<Header>(node, cx)
        .map(|at| <Header as super::table::Item>::load_one(at, cx.tables))
}

/// Reads a record of type `T` into its table: whether it fits.
fn typed<T>(node: NodeRef<'_>, cx: &mut ReadCx<'_>, sink: &mut dyn RecordSink) -> bool
where
    T: super::edit::RecordRows
        + HasTable
        + super::table::sealed::Listed<Elem = <T as super::table::sealed::Stored>::Row>,
{
    let at = super::table::reserve(&mut T::table_mut(cx.tables).rows, 1, T::fill());
    let Some(row) = T::read_elem(node, at, cx) else {
        let rows = &mut T::table_mut(cx.tables).rows;
        if rows.len() == at as usize + 1 {
            rows.pop();
        } else {
            cx.tables.garbage += 1;
        }
        return false;
    };
    let xref = T::row_xref(&row);
    if let Some(slot) = T::table_mut(cx.tables).rows.get_mut(at as usize) {
        *slot = row;
    }
    sink.placed(T::ROWS_KIND, at as usize, xref);
    true
}

/// A dataset's header and index, which reading fills as it places the
/// records.
struct DatasetSink<'d> {
    header: &'d mut Option<Box<Header>>,
    index: &'d mut super::edit::Index,
}

impl RecordSink for DatasetSink<'_> {
    fn has_header(&self) -> bool {
        self.header.is_some()
    }

    fn header(&mut self, header: Header) {
        *self.header = Some(Box::new(header));
    }

    /// Each record placed is noted in the dataset's index while it is at
    /// hand ([`super::edit::Index::define_read`]).
    fn placed(&mut self, kind: RecordKind, pos: usize, xref: Option<XrefId>) {
        self.index.define_read(kind, pos, xref);
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
    let mut scratch = Scratch::default();
    {
        let Dataset {
            header,
            tables,
            index,
            ..
        } = &mut data;
        let mut sink = DatasetSink { header, index };
        let parts = store.parts_mut();
        let input = parts.text;
        let mut cx = ReadCx::new(parts, tables, &mut scratch, version);
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
                    dispatch_record(root, &mut cx, &mut sink);
                }
            });
        }
    }
    store.set_tags(tags.others);
    store.shrink();
    // The spare rows of a small file, a few kilobytes at most, are not
    // worth reallocating every table.
    let small = store.input_len() < SMALL_INPUT;
    data.store = store;
    if !small {
        data.shrink();
    }
    data.index_read();
    data
}

/// The size of input under which a dataset keeps its tables' spare rows.
const SMALL_INPUT: usize = 64 * 1024;

/// A record a [`RecordReader`] read: the header, or the position of a
/// record of a kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Placed {
    Header,
    Record(RecordKind, usize),
}

/// Reads a file one record at a time, each into a dataset of its own: what
/// a stream reads.
pub(crate) struct RecordReader {
    builder: Builder,
    version: GedcomVersion,
    header_seen: bool,
    scratch: Scratch,
}

impl RecordReader {
    /// A reader of a 5.5.1 file, until [`set_version`](Self::set_version)
    /// says otherwise.
    pub(crate) fn new() -> Self {
        Self {
            builder: Builder::new(Escaping::V551, 1).with_spans(),
            version: GedcomVersion::V5_5_1,
            header_seen: false,
            scratch: Scratch::default(),
        }
    }

    /// The version the file declares, and how it escapes `@`.
    pub(crate) fn set_version(&mut self, version: GedcomVersion, escaping: Escaping) {
        self.version = version;
        self.builder.set_escaping(escaping);
    }

    /// Reads the records of `text` (normally one; lines without a level
    /// before the first record form one of their own) into a dataset of
    /// their own, which keeps `text` as its store: the dataset and where
    /// each record is in it.
    pub(crate) fn read(&mut self, text: String, first_line: u32, out: &mut Vec<Placed>) -> Dataset {
        struct Sink<'o> {
            header_seen: &'o mut bool,
            header: &'o mut Option<Box<Header>>,
            out: &'o mut Vec<Placed>,
        }
        impl RecordSink for Sink<'_> {
            fn has_header(&self) -> bool {
                *self.header_seen
            }
            fn header(&mut self, header: Header) {
                *self.header_seen = true;
                *self.header = Some(Box::new(header));
                self.out.push(Placed::Header);
            }
            fn placed(&mut self, kind: RecordKind, pos: usize, _xref: Option<XrefId>) {
                self.out.push(Placed::Record(kind, pos));
            }
        }

        let mut data = Dataset::new(self.version);
        let mut store = Store::of_record(text);
        let mut tags = TagInterner::default();
        {
            let Dataset { header, tables, .. } = &mut data;
            let mut sink = Sink {
                header_seen: &mut self.header_seen,
                header,
                out,
            };
            let parts = store.parts_mut();
            let input = parts.text;
            let mut cx = ReadCx::new(parts, tables, &mut self.scratch, self.version);
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
        data.store = store;
        data
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
                .map(|r| r.associations().len()),
            Some(1)
        );
        let note = whole
            .individuals()
            .first()
            .unwrap()
            .notes()
            .first()
            .unwrap();
        assert!(matches!(note.content(), NoteContent::Text(t) if t.to_str(&whole) == "a@b\nc"));
    }

    #[test]
    fn records_of_one_text() {
        let mut reader = RecordReader::new();
        reader.set_version(GedcomVersion::V7_0, Escaping::V70);
        let mut out = Vec::new();
        reader.read("0 HEAD\n1 GEDC\n2 VERS 7.0\n".into(), 1, &mut out);
        assert_eq!(out, [Placed::Header]);
        out.clear();
        let data = reader.read(
            "0 @I1@ INDI\n1 NAME Ann /Example/\n1 _X y\n".into(),
            4,
            &mut out,
        );
        assert_eq!(out, [Placed::Record(RecordKind::Individual, 0)]);
        let indi = data.individuals().first().unwrap();
        assert_eq!(data.store().xref(indi.xref().unwrap()), "@I1@");
        assert_eq!(indi.extra().first().unwrap().tag(), "_X");
        out.clear();
        // A second header is kept untyped; an empty trailer, not at all.
        reader.read("0 HEAD\n".into(), 8, &mut out);
        assert_eq!(out, [Placed::Record(RecordKind::Other, 0)]);
        out.clear();
        reader.read("0 TRLR\n".into(), 9, &mut out);
        assert!(out.is_empty());
    }
}
