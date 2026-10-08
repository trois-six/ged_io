//! The conformance repair pass: the repair table of the [module
//! documentation](super#conformance-repair), applied to owned structures.
//!
//! The pass walks every record with its structure type, repairing payloads
//! and substructures bottom-up: a structure that cannot be made valid where
//! it is becomes an extension structure (`_TAG`) in the same place, with
//! its payload and substructures. As that can change what a pointer points
//! to (a record that becomes an extension is no longer of its type), the
//! pass runs again until it changes nothing; each repair moves data towards
//! extensions only, so this ends, in two passes in practice.
//!
//! Most records need no repair. Each pass first looks at every record
//! without changing it ([`clean`]): a record the repair would leave as it is
//! stays as it was given — borrowed from a parsed tree, never copied — and
//! only the others are copied and repaired. A record found so stays so in
//! the following passes while what it depends on (the record each
//! identifier names, the renamed identifiers, the aliases) is unchanged.

mod clean;

use clean::Walk;

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use super::payload::{self, is_ext_tag, Family};
use super::schema::{tag_index, Kind, Schema, StructId, DATASET};
use super::tables::{self, TAGS};
use super::validate::{
    bounds, encoded_len, is_external_pointer, pick, xref_error, Aliases, Pick, Picker,
    MAX_RECORD_BYTES,
};
use crate::tree::{Node, Payload, PayloadRef, Structure, Tag, Xref};
use crate::version::VersionRules;
use crate::writer::{
    complete_head, extension_tag, needs_submitter, new_xref, numbered_xref, stub_submitter, Repair,
    RepairKind, XrefIndex, PLACEHOLDER,
};
use crate::GedcomVersion;

/// The most passes [`conform`] makes; each one after the first only follows
/// up on records that became extensions.
const MAX_PASSES: usize = 8;

/// The pointers looked up together ([`clean::Walk::pending`]): 2 MB.
const LOOKUP_BATCH: usize = 1 << 16;

/// Repairs `records` (a whole dataset: header, records and trailer, in
/// order) so that [`validate`](super::validate) finds nothing for
/// `version`, and returns what was changed, in the order it was changed.
///
/// Every repair is deterministic and keeps the data: what has no standard
/// place becomes an extension structure where it was. The [module
/// documentation](super#conformance-repair) lists every repair. The one
/// deviation that may remain is a 5.5.1 record over 32K with no inline note
/// left to move out ([`DeviationKind::RecordSize`]), which 5.5.1 only
/// recommends against.
///
/// [`DeviationKind::RecordSize`]: super::DeviationKind::RecordSize
pub fn conform(records: &mut Vec<Structure>, version: GedcomVersion) -> Vec<Repair> {
    let mut recs: Vec<Rec<'_, &Structure>> = records.drain(..).map(Rec::owned).collect();
    let repairs = conform_records(&mut recs, version).repairs;
    records.extend(recs.into_iter().map(Rec::into_structure));
    repairs
}

/// What [`conform_records`] did, and the identifiers it left.
pub(crate) struct Conformed<'n> {
    /// The repairs, in the order they were made.
    pub(crate) repairs: Vec<Repair>,
    xrefs: XrefIndex<'n, Option<StructId>>,
}

impl Conformed<'_> {
    /// Whether a record has this identifier. Every record identifier is
    /// valid and unique once repaired.
    pub(crate) fn knows(&self, xref: &str) -> bool {
        self.xrefs.contains_key(xref)
    }
}

/// [`conform`] on records as they are given ([`Rec`]): only the records
/// that need a repair are copied.
pub(crate) fn conform_records<'n, N: Node<'n>>(
    records: &mut Vec<Rec<'n, N>>,
    version: GedcomVersion,
) -> Conformed<'n> {
    let mut c = Conformer::new(version);
    for pass in 0..MAX_PASSES {
        let before = c.repairs.len();
        c.pass(records, pass);
        if pass == 0 && c.family == Family::V551 {
            c.record_sizes(records);
        }
        if c.repairs.len() == before && pass > 0 {
            break;
        }
    }
    Conformed {
        repairs: c.repairs,
        xrefs: c.xrefs,
    }
}

/// A record of a dataset being repaired: as it was given (a node of a
/// parsed tree, a borrowed structure), as long as the repair leaves it so,
/// or an owned copy, which the repair changes.
pub(crate) struct Rec<'n, N> {
    body: Body<N>,
    /// The tag and identifier of a record as given (read once: they are
    /// asked for pass after pass, and a tree finds an identifier by a
    /// search), and the tag's index among the standard tags.
    tag: &'n str,
    xref: Option<&'n str>,
    standard: Option<u16>,
    /// The generation of what the record depends on ([`Conformer::state`])
    /// under which it was found to need no repair; 0 when it was not.
    clean: u32,
    /// Its record type then (the dataset's type for none), which counts
    /// towards the records of that type the dataset permits.
    rtype: StructId,
    /// An upper bound of its size as written, from its last look (5.5.1
    /// records over 32K: [`Conformer::record_sizes`]); `u32::MAX` when
    /// unknown.
    size: u32,
}

enum Body<N> {
    Given(N),
    Owned(Box<Structure>),
}

/// A record, borrowed: given or owned.
pub(crate) enum RecRef<'b, N> {
    Given(N),
    Owned(&'b Structure),
}

/// Runs `$body` with `$n` bound to record `$rec` as a node: as given or as
/// owned.
macro_rules! with_node {
    ($rec:expr, |$n:ident| $body:expr) => {
        match $rec {
            RecRef::Given($n) => $body,
            RecRef::Owned($n) => $body,
        }
    };
}

impl<'n, N: Node<'n>> Rec<'n, N> {
    /// A record as it is given.
    pub(crate) fn given(node: N) -> Self {
        Self::of(
            node.tag(),
            node.xref(),
            node.standard_tag(),
            Body::Given(node),
        )
    }

    /// An owned record.
    pub(crate) fn owned(s: Structure) -> Self {
        Self::of("", None, None, Body::Owned(Box::new(s)))
    }

    fn of(tag: &'n str, xref: Option<&'n str>, standard: Option<u16>, body: Body<N>) -> Self {
        Self {
            body,
            tag,
            xref,
            standard,
            clean: 0,
            rtype: DATASET,
            size: u32::MAX,
        }
    }

    /// The record, borrowed.
    pub(crate) fn get(&self) -> RecRef<'_, N> {
        match &self.body {
            Body::Given(n) => RecRef::Given(*n),
            Body::Owned(s) => RecRef::Owned(s),
        }
    }

    pub(crate) fn tag<'b>(&'b self) -> &'b str
    where
        'n: 'b,
    {
        match &self.body {
            Body::Given(_) => self.tag,
            Body::Owned(s) => s.tag.as_str(),
        }
    }

    pub(crate) fn xref<'b>(&'b self) -> Option<&'b str>
    where
        'n: 'b,
    {
        match &self.body {
            Body::Given(_) => self.xref,
            Body::Owned(s) => s.xref.as_deref(),
        }
    }

    /// The index of the tag among the standard tags.
    fn standard_tag(&self) -> Option<u16> {
        match &self.body {
            Body::Given(_) => self.standard,
            Body::Owned(s) => s.tag.standard_index(),
        }
    }

    fn line(&self) -> u32 {
        match &self.body {
            Body::Given(n) => n.line(),
            Body::Owned(s) => s.line,
        }
    }

    /// Whether the record has neither payload nor substructure.
    pub(crate) fn is_empty(&self) -> bool {
        match &self.body {
            Body::Given(n) => n.payload() == PayloadRef::None && n.children().next().is_none(),
            Body::Owned(s) => s.payload == Payload::None && s.substructures.is_empty(),
        }
    }

    /// The record as an owned structure, as it is.
    fn to_structure(&self) -> Structure {
        with_node!(self.get(), |n| n.to_owned_structure())
    }

    /// The record, copied first if it is not owned yet.
    fn make_owned(&mut self) -> &mut Structure {
        if !matches!(self.body, Body::Owned(_)) {
            self.body = Body::Owned(Box::new(self.to_structure()));
        }
        match &mut self.body {
            Body::Owned(s) => s,
            Body::Given(_) => unreachable!("made owned above"),
        }
    }

    /// The record as an owned structure, taken out (an owned record is
    /// left empty).
    pub(crate) fn take_structure(&mut self) -> Structure {
        match &mut self.body {
            Body::Owned(s) => std::mem::take(s),
            Body::Given(_) => self.to_structure(),
        }
    }

    /// The record as an owned structure.
    pub(crate) fn into_structure(self) -> Structure {
        match self.body {
            Body::Owned(s) => *s,
            Body::Given(_) => self.to_structure(),
        }
    }
}

struct Conformer<'n> {
    rules: &'static VersionRules,
    schema: &'static Schema,
    family: Family,
    repairs: Vec<Repair>,
    /// Every record identifier in use, with the type of the record it
    /// names as the pass found it (`None` for extension records).
    xrefs: XrefIndex<'n, Option<StructId>>,
    /// The new identifier of each identifier that was invalid.
    renamed: HashMap<Box<str>, Box<str>>,
    /// Extension tags of relocated standard structures, with their URIs.
    schma: Vec<(String, String)>,
    /// The tags from the record to the structure being repaired.
    path: Vec<String>,
    /// The submitter a synthesised 5.5.1 `HEAD.SUBM` points to.
    submitter: Option<Box<str>>,
    /// Records to add before the trailer.
    new_records: Vec<Structure>,
    /// The aliases `HEAD.SCHMA` declares (7.x).
    aliases: Aliases,
    /// Every extension tag of the dataset as the pass began, when a repair
    /// may need it (7.x relocations).
    used: Option<HashSet<String>>,
    /// The generation of what a record found clean depends on: the types
    /// in `xrefs`, `renamed` and `aliases`. It changes when they do.
    state: u32,
    /// The number of repairs when the last pass began.
    pass_start: usize,
    /// What the check needs of each type, computed once per version.
    tables: &'static TypeTables,
    /// The type of each substructure, by tag.
    picker: Picker,
}

/// A required substructure: its tag, its minimum and the first
/// substructure type with that tag.
type Required = (u8, u8, StructId);

/// What the check needs of each structure type of a version's tables.
struct TypeTables {
    /// The required substructures of each type: (tag, minimum, the first
    /// substructure type with that tag), in the order of the tables.
    required: Box<[Box<[Required]>]>,
    /// The types whose rules the tables cannot state: 7.x `NOTE-TRAN` and
    /// `MIME`.
    note_tran: Box<[StructId]>,
    mime: Box<[StructId]>,
    /// The record type of each tag of the tables ([`Schema::record`]), 0
    /// for none.
    records: Box<[StructId]>,
}

impl TypeTables {
    fn of(schema: &'static Schema) -> &'static TypeTables {
        static TABLES: [OnceLock<TypeTables>; 3] = [const { OnceLock::new() }; 3];
        static OTHER: OnceLock<TypeTables> = OnceLock::new();
        let slot = [&tables::V551, &tables::V70, &tables::V71]
            .into_iter()
            .position(|s| std::ptr::eq(s, schema))
            .and_then(|i| TABLES.get(i))
            .unwrap_or(&OTHER);
        slot.get_or_init(|| TypeTables::build(schema))
    }

    fn build(schema: &Schema) -> TypeTables {
        let ids = || (0..schema.structs.len()).filter_map(|i| StructId::try_from(i).ok());
        let required = ids()
            .map(|ty| {
                let mut seen: Vec<u8> = Vec::new();
                let mut req: Vec<Required> = Vec::new();
                for sub in schema.subs(ty) {
                    let Some(t) = schema.tag_id(sub.id) else {
                        continue;
                    };
                    if seen.contains(&t) {
                        continue;
                    }
                    seen.push(t);
                    let (min, _) = bounds(schema, ty, t);
                    if min > 0 {
                        req.push((t, min, sub.id));
                    }
                }
                req.into_boxed_slice()
            })
            .collect();
        let named = |name: &str| -> Box<[StructId]> {
            if schema.version.starts_with('7') {
                ids().filter(|&ty| schema.name(ty) == name).collect()
            } else {
                Box::default()
            }
        };
        let records = TAGS
            .iter()
            .map(|tag| schema.record(tag).unwrap_or(DATASET))
            .collect();
        TypeTables {
            required,
            note_tran: named("NOTE-TRAN"),
            mime: named("MIME"),
            records,
        }
    }
}

/// What becomes of a structure after its repair.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fate {
    Keep,
    /// It becomes an extension structure; the repair is reported already.
    Extension,
    /// It holds nothing and goes; the repair is reported already.
    Remove,
}

fn text(tag: &str, value: &str) -> Structure {
    Structure {
        payload: Payload::Text(value.into()),
        ..Structure::new(tag)
    }
}

fn pointer(tag: &str, xref: &str) -> Structure {
    Structure {
        payload: Payload::Pointer(Xref::new(xref)),
        ..Structure::new(tag)
    }
}

/// The media type of a file from its extension; `application/octet-stream`
/// when unknown.
fn media_type(path: &str) -> &'static str {
    let ext = extension(path);
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "htm" | "html" => "text/html",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "mp4" => "video/mp4",
        "mpg" | "mpeg" => "video/mpeg",
        "avi" => "video/x-msvideo",
        _ => "application/octet-stream",
    }
}

/// The 5.5.1 `MULTIMEDIA_FORMAT` of a file from its extension (p. 54), if
/// the list has one.
fn multimedia_format(path: &str) -> Option<&'static str> {
    match extension(path).as_str() {
        "bmp" => Some("bmp"),
        "gif" => Some("gif"),
        "jpg" | "jpeg" => Some("jpg"),
        "ole" => Some("ole"),
        "pcx" => Some("pcx"),
        "tif" | "tiff" => Some("tif"),
        "wav" => Some("wav"),
        _ => None,
    }
}

fn extension(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or("");
    let name = name.split(['?', '#']).next().unwrap_or("");
    name.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

impl<'n> Conformer<'n> {
    fn repair(&mut self, line: u32, kind: RepairKind, detail: String) {
        self.repairs.push(Repair {
            line,
            kind,
            detail: detail.into(),
        });
    }

    /// The path of the structure being repaired, ending with `tag`.
    fn here(&self, tag: &str) -> String {
        let mut p = self.path.join(".");
        if !p.is_empty() {
            p.push('.');
        }
        p.push_str(tag);
        p
    }

    fn new(version: GedcomVersion) -> Self {
        let rules = version.rules();
        let schema = rules.spec;
        let family = Family::of(rules);
        Conformer {
            rules,
            schema,
            family,
            repairs: Vec::new(),
            xrefs: XrefIndex::default(),
            renamed: HashMap::new(),
            schma: Vec::new(),
            path: Vec::new(),
            submitter: None,
            new_records: Vec::new(),
            aliases: Aliases::default(),
            used: None,
            state: 1,
            pass_start: 0,
            tables: TypeTables::of(schema),
            picker: Picker::of(rules),
        }
    }

    fn pass<N: Node<'n>>(&mut self, records: &mut Vec<Rec<'n, N>>, pass: usize) {
        // Whether nothing was repaired since the last pass began.
        let quiet = pass > 0 && self.repairs.len() == self.pass_start;
        self.pass_start = self.repairs.len();
        self.header_and_trailer(records);
        let quiet = quiet && self.repairs.len() == self.pass_start;
        let head = records.first().filter(|r| r.tag() == "HEAD");
        let aliases = match head.map(Rec::get) {
            Some(h) => with_node!(h, |h| Aliases::of(Some(h), self.schema)),
            None => Aliases::of(None::<&Structure>, self.schema),
        };
        let aliases_changed = aliases != self.aliases;
        self.aliases = aliases;
        // The identifiers and the types they name only change with a
        // repair: after a pass that repaired nothing, they are as they were
        // (the records it added are in `xrefs` already).
        if !quiet || aliases_changed {
            let renamed = self.renamed.len();
            let previous = std::mem::take(&mut self.xrefs);
            self.xrefs(records);
            if pass == 0
                || aliases_changed
                || previous != self.xrefs
                || self.renamed.len() != renamed
            {
                self.state += 1;
            }
        }
        self.submitter = self.submitter.take().or_else(|| {
            records
                .iter()
                .find(|r| r.tag() == "SUBM" && r.xref().is_some())
                .and_then(|r| r.xref().map(Box::from))
        });
        // The extension tags of the header before it is completed (as the
        // pass begins), for relocations.
        let head_tags = if self.family == Family::V7 {
            records
                .first()
                .filter(|r| r.tag() == "HEAD")
                .map(|h| extension_tags(h))
        } else {
            None
        };
        // Completing the header twice changes nothing: after a quiet pass,
        // it is complete.
        if !quiet {
            self.complete_head(records);
        }

        // Which records need a repair, without changing any.
        let head_ty = self.record_of("HEAD");
        let dirty = self.find_dirty(records, head_ty);
        let any_dirty = !dirty.is_empty();
        // A relocation (7.x) picks an extension tag no record uses yet.
        self.used = (self.family == Family::V7 && any_dirty).then(|| {
            let mut used: HashSet<String> = head_tags.unwrap_or_default();
            for r in records.iter().skip(1) {
                used.extend(extension_tags(r));
            }
            used
        });

        self.repair_dirty(records, head_ty, dirty);
        self.path.clear();
        if !self.new_records.is_empty() {
            let at = records.len().saturating_sub(1);
            records.splice(at..at, self.new_records.drain(..).map(Rec::owned));
        }
        if self.family == Family::V7 {
            self.declare(records);
        }
        self.used = None;
    }

    /// Which records need a repair, found without changing any; their
    /// pointers are looked up in batches. In record order.
    fn find_dirty<N: Node<'n>>(
        &mut self,
        records: &mut [Rec<'n, N>],
        head_ty: Option<StructId>,
    ) -> Vec<usize> {
        let mut dirty: Vec<usize> = Vec::new();
        let mut pending = Vec::with_capacity(LOOKUP_BATCH.min(4 * records.len()));
        let mut failed = Vec::new();
        for (i, r) in records.iter_mut().enumerate() {
            let clean = match r.tag() {
                "TRLR" => true,
                "HEAD" if i == 0 && r.clean == self.state => true,
                "HEAD" if i == 0 => {
                    let clean = match head_ty {
                        Some(ty) => with_node!(r.get(), |h| self.is_clean(h, ty, true)),
                        None => true,
                    };
                    if clean {
                        r.clean = self.state;
                    }
                    clean
                }
                _ if r.clean == self.state => true,
                _ => {
                    let mark = pending.len();
                    let mut walk = Walk {
                        size: 0,
                        pending: Some(&mut pending),
                        record: u32::try_from(i).unwrap_or(u32::MAX),
                        rtype: DATASET,
                    };
                    let xref = r.xref();
                    let clean = with_node!(r.get(), |n| self.is_clean_record(n, xref, &mut walk));
                    let (size, rtype) = (walk.size, walk.rtype);
                    if clean {
                        r.clean = self.state;
                        r.rtype = rtype;
                        r.size = u32::try_from(size).unwrap_or(u32::MAX);
                    } else {
                        pending.truncate(mark);
                    }
                    if pending.len() >= LOOKUP_BATCH {
                        self.look_up(&mut pending, &mut failed);
                    }
                    clean
                }
            };
            if !clean {
                dirty.push(i);
            }
        }
        self.look_up(&mut pending, &mut failed);
        if !failed.is_empty() {
            for &i in &failed {
                if let Some(r) = records.get_mut(i as usize) {
                    r.clean = 0;
                }
            }
            dirty.extend(failed.iter().map(|&i| i as usize));
            dirty.sort_unstable();
            dirty.dedup();
        }
        dirty
    }

    /// The repairs, in record order: the dirty records repaired, the others
    /// counted.
    fn repair_dirty<N: Node<'n>>(
        &mut self,
        records: &mut [Rec<'n, N>],
        head_ty: Option<StructId>,
        dirty: Vec<usize>,
    ) {
        let mut counts: Vec<(u8, u32)> = Vec::new();
        let mut next_dirty = dirty.into_iter().peekable();
        for (i, r) in records.iter_mut().enumerate() {
            let is_dirty = next_dirty.next_if_eq(&i).is_some();
            match r.tag() {
                "TRLR" => continue,
                "HEAD" if i == 0 => {
                    if let (Some(head), true) = (head_ty, is_dirty) {
                        self.path.clear();
                        r.clean = 0;
                        let _ = self.fix(r.make_owned(), head, true);
                    }
                    continue;
                }
                _ => {}
            }
            self.path.clear();
            if is_dirty {
                (r.clean, r.size) = (0, u32::MAX);
                self.record(r.make_owned(), &mut counts);
                continue;
            }
            #[cfg(debug_assertions)]
            self.assert_clean(r);
            // A clean record of a standard type still counts towards the
            // records of its type the dataset permits.
            if r.rtype != DATASET {
                let t = self.schema.tag_id(r.rtype).unwrap_or(0);
                let n = bump(&mut counts, t);
                if self
                    .picker
                    .max(DATASET, t)
                    .is_some_and(|max| n > u32::from(max))
                {
                    if let Some((_, n)) = counts.iter_mut().find(|(k, _)| *k == t) {
                        *n -= 1;
                    }
                    (r.clean, r.size) = (0, u32::MAX);
                    self.record(r.make_owned(), &mut counts);
                }
            }
        }
    }

    /// Checks, in debug builds, that a record found clean is one the repair
    /// leaves as it is.
    #[cfg(debug_assertions)]
    fn assert_clean<N: Node<'n>>(&mut self, r: &Rec<'n, N>) {
        let original = r.to_structure();
        let mut copy = original.clone();
        let before = self.repairs.len();
        let mut counts = Vec::new();
        self.record(&mut copy, &mut counts);
        self.path.clear();
        assert!(
            self.repairs.len() == before && copy == original,
            "a record found clean is repaired: {:?}",
            self.repairs.get(before..)
        );
    }

    /// (i) Completes the header as the writer does for the version
    /// ([`crate::writer`]): `GEDC.VERS` (and in 5.5.1 `GEDC.FORM`,
    /// `CHAR`, `SOUR` and `SUBM`, with a submitter record when there is
    /// none), without an identifier; 7.x drops `CHAR` and `GEDC.FORM`,
    /// which describe the bytes and the form of the input. The writer
    /// rewrites the header of every file it writes, so this is no repair.
    fn complete_head<N: Node<'n>>(&mut self, records: &mut [Rec<'n, N>]) {
        let Some(head) = records.first_mut().filter(|r| r.tag() == "HEAD") else {
            return;
        };
        head.clean = 0;
        let head = head.make_owned();
        let submitter = needs_submitter(head, self.rules).then(|| {
            if let Some(x) = &self.submitter {
                return x.to_string();
            }
            let x = self.fresh("U", self.record_of("SUBM"));
            self.new_records.push(stub_submitter(x.clone()));
            self.submitter = Some(x.as_str().into());
            x
        });
        complete_head(head, self.rules, "UTF-8", submitter);
    }

    /// The first `@<prefix>n@` no record holds; taken, for a record of type
    /// `ty`.
    fn fresh(&mut self, prefix: &str, ty: Option<StructId>) -> String {
        let mut n = 0_usize;
        loop {
            n += 1;
            let candidate = format!("@{prefix}{n}@");
            if !self.xrefs.contains_key(candidate.as_str()) {
                self.xrefs.insert(Cow::Owned(candidate.clone()), ty);
                return candidate;
            }
        }
    }

    fn record(&mut self, r: &mut Structure, counts: &mut Vec<(u8, u32)>) {
        let is_ptr = matches!(r.payload, Payload::Pointer(_));
        let tag = r.tag.as_str().to_string();
        match pick(self.rules, DATASET, &tag, is_ptr) {
            Pick::Type(ty) => {
                if self.fix(r, ty, true) == Fate::Extension {
                    self.make_extension(r);
                    return;
                }
                let t = self.schema.tag_id(ty).unwrap_or(0);
                let n = bump(counts, t);
                if let Some(max) = self.picker.max(DATASET, t) {
                    if n > u32::from(max) {
                        self.repair(
                            r.line,
                            RepairKind::Repeated,
                            format!("a {tag} record more than the {max} permitted, written as an extension record"),
                        );
                        self.make_extension(r);
                    }
                }
            }
            // A documented alias of a standard record is that record.
            Pick::Extension => match self.aliases.structs.get(&tag).copied() {
                Some(ty) if self.fix(r, ty, true) == Fate::Extension => self.make_extension(r),
                Some(_) => {}
                None => self.extension(r),
            },
            _ => {
                let new = self.plain_tag(&tag);
                self.repair(
                    r.line,
                    RepairKind::Misplaced,
                    format!(
                        "{tag:?} is not a record of GEDCOM {}: written as {new}",
                        self.schema.version
                    ),
                );
                self.make_extension(r);
            }
        }
    }

    /// (i) One `HEAD` first, one empty `TRLR` last.
    fn header_and_trailer<N: Node<'n>>(&mut self, records: &mut Vec<Rec<'n, N>>) {
        match records.iter().position(|r| r.tag() == "HEAD") {
            None => {
                records.insert(0, Rec::owned(Structure::new("HEAD")));
                self.repair(0, RepairKind::Header, "HEAD added".into());
            }
            Some(0) => {}
            Some(at) => {
                let head = records.remove(at);
                self.repair(head.line(), RepairKind::Header, "HEAD moved first".into());
                records.insert(0, head);
            }
        }
        for r in records.iter_mut().skip(1).filter(|r| r.tag() == "HEAD") {
            r.make_owned().tag = Tag::new("_HEAD");
            self.repair(
                r.line(),
                RepairKind::Header,
                "a second HEAD written as a _HEAD record".into(),
            );
        }
        if let Some(head) = records.first_mut().filter(|h| h.xref().is_some()) {
            if let Some(x) = head.make_owned().xref.take() {
                self.repair(
                    head.line(),
                    RepairKind::Xref,
                    format!("identifier {x} of HEAD left out"),
                );
            }
        }
        let trailers: Vec<usize> = records
            .iter()
            .enumerate()
            .filter(|(_, r)| r.tag() == "TRLR")
            .map(|(i, _)| i)
            .collect();
        let well_formed = trailers.len() == 1
            && trailers.first() == Some(&(records.len() - 1))
            && records
                .last()
                .is_some_and(|t| t.is_empty() && t.xref().is_none());
        if well_formed {
            return;
        }
        let mut line = 0;
        for &i in trailers.iter().rev() {
            let Some(t) = records.get_mut(i) else {
                continue;
            };
            line = t.line();
            if t.is_empty() {
                records.remove(i);
            } else {
                let t = t.make_owned();
                t.tag = Tag::new("_TRLR");
                t.xref = None;
                self.repair(
                    t.line,
                    RepairKind::Header,
                    "a TRLR with content written as a _TRLR record".into(),
                );
            }
        }
        records.push(Rec::owned(Structure::new("TRLR")));
        self.repair(
            line,
            RepairKind::Header,
            "one empty TRLR written last".into(),
        );
    }

    /// (h) Valid, unique identifiers on records; invalid ones renamed.
    /// Fills `xrefs` with every identifier and the type of its record.
    fn xrefs<N: Node<'n>>(&mut self, records: &mut [Rec<'n, N>]) {
        self.xrefs.clear();
        self.xrefs.reserve(records.len());
        let mut keep = Vec::with_capacity(records.len());
        for r in records.iter() {
            let tag = r.tag();
            let ok = match (tag, &r.body) {
                ("HEAD" | "TRLR", _) => true,
                (_, Body::Given(_)) => r
                    .xref
                    .is_none_or(|x| self.take(Cow::Borrowed(x), tag, r.standard)),
                (_, Body::Owned(s)) => s.xref.as_deref().is_none_or(|x| {
                    self.take(Cow::Owned(x.to_string()), tag, s.tag.standard_index())
                }),
            };
            keep.push(ok);
        }
        for r in &self.new_records {
            if let Some(x) = r.xref.as_deref() {
                let ty = self.record_type(r.tag.as_str());
                self.xrefs.insert(Cow::Owned(x.to_string()), ty);
            }
        }
        for (r, ok) in records.iter_mut().zip(keep) {
            if ok {
                continue;
            }
            let r = r.make_owned();
            let Some(old) = r.xref.take() else { continue };
            let ty = self.record_type(r.tag.as_str());
            let new = self.unique(&new_xref(self.rules, &old), ty);
            if xref_error(&old, self.rules).is_some() && !self.renamed.contains_key(old.as_str()) {
                self.renamed
                    .insert(old.as_str().into(), new.as_str().into());
            }
            self.repair(
                r.line,
                RepairKind::Xref,
                format!("identifier {old} written as {new}"),
            );
            r.xref = Some(Xref::new(new));
        }
    }

    /// Takes `xref`, the valid identifier of a record tagged `tag`, unless
    /// a record holds it already.
    fn take(&mut self, xref: Cow<'n, str>, tag: &str, standard: Option<u16>) -> bool {
        if xref_error(&xref, self.rules).is_some() {
            return false;
        }
        let ty = self.record_type_of(tag, standard);
        self.xrefs.insert_new(xref, ty)
    }

    /// `candidate`, or `candidate` with the first free `_2`, `_3`… suffix;
    /// taken, for a record of type `ty`.
    fn unique(&mut self, candidate: &str, ty: Option<StructId>) -> String {
        let mut new = candidate.to_string();
        let mut n = 1_usize;
        while self.xrefs.contains_key(new.as_str()) {
            n += 1;
            new = numbered_xref(self.rules, candidate, n);
        }
        self.xrefs.insert(Cow::Owned(new.clone()), ty);
        new
    }

    /// The type of the record tagged `tag`: a standard one, or one an alias
    /// stands for.
    fn record_type(&self, tag: &str) -> Option<StructId> {
        self.record_type_of(tag, crate::tree::node::standard_index(tag))
    }

    /// [`Conformer::record_type`] of a tag whose standard index is known.
    fn record_type_of(&self, tag: &str, standard: Option<u16>) -> Option<StructId> {
        self.record_of_standard(tag, standard)
            .or_else(|| self.aliases.structs.get(tag).copied())
    }

    /// The type of the record tagged `tag` the version defines
    /// ([`Schema::record`], from a table).
    fn record_of(&self, tag: &str) -> Option<StructId> {
        self.record_of_standard(tag, crate::tree::node::standard_index(tag))
    }

    fn record_of_standard(&self, tag: &str, standard: Option<u16>) -> Option<StructId> {
        match standard.and_then(|t| self.picker.spec_tag(t)) {
            Some(t) => self
                .tables
                .records
                .get(usize::from(t))
                .copied()
                .filter(|&ty| ty != DATASET),
            None => self.schema.record(tag),
        }
    }

    /// The extension tag a structure tagged `tag` is written with when it
    /// cannot stay standard: `_TAG`, and as many `_` more as it takes not to
    /// be an alias of a standard structure.
    fn plain_tag(&self, tag: &str) -> String {
        let mut new = extension_tag(self.rules, tag);
        while self.aliases.structs.contains_key(&new) || self.schma.iter().any(|(t, _)| *t == new) {
            new.push('_');
        }
        new
    }

    /// Makes `s` an extension structure: its tag, then its content.
    fn make_extension(&mut self, s: &mut Structure) {
        s.tag = Tag::new(&self.plain_tag(s.tag.as_str()));
        self.extension_content(s, true);
    }

    /// (e, f) Moves a standard structure of type `ty` (valid as such) out
    /// of a place that does not permit it: in 7.x as an extension tag that
    /// `HEAD.SCHMA` declares with the type's URI, when that tag is free;
    /// otherwise as a plain extension. Returns the tag and how it was
    /// declared, for the repair.
    fn relocate(&mut self, s: &mut Structure, ty: Option<StructId>) -> (String, String) {
        let tag = s.tag.as_str().to_string();
        let candidate = extension_tag(self.rules, &tag);
        if let (Family::V7, Some(ty)) = (self.family, ty) {
            let uri = self.schema.uri(ty);
            let ours = self.aliases.structs.get(&candidate) == Some(&ty)
                || self.schma.iter().any(|(t, u)| *t == candidate && *u == uri);
            let free = !self.aliases.structs.contains_key(&candidate)
                && !self.used.as_ref().is_some_and(|u| u.contains(&candidate))
                && !self.schma.iter().any(|(t, _)| *t == candidate);
            if ours || free {
                if free {
                    self.schma.push((candidate.clone(), uri.clone()));
                }
                s.tag = Tag::new(&candidate);
                return (candidate, format!(" (declared as {uri})"));
            }
        }
        self.make_extension(s);
        (s.tag.as_str().to_string(), String::new())
    }

    /// An extension structure: tags in the grammar, no identifier below the
    /// record, pointers that name a record, no banned character.
    fn extension(&mut self, s: &mut Structure) {
        self.extension_content(s, true);
    }

    fn extension_content(&mut self, s: &mut Structure, top: bool) {
        if !top {
            if !self.rules.is_valid_tag(s.tag.as_str()) {
                let new = extension_tag(self.rules, s.tag.as_str());
                self.repair(
                    s.line,
                    RepairKind::Misplaced,
                    format!("{:?} is not a tag: written as {new}", s.tag.as_str()),
                );
                s.tag = Tag::new(&new);
            }
            self.drop_xref(s);
        }
        match &s.payload {
            Payload::Pointer(p) => {
                let p = p.as_str().to_string();
                if self.resolve(s, &p).is_err() {
                    self.repair(
                        s.line,
                        RepairKind::Pointer,
                        format!("{} {p} names no record: kept as text", s.tag.as_str()),
                    );
                    s.payload = Payload::Text(p.into());
                }
            }
            Payload::Text(_) => self.characters(s),
            Payload::None => {}
        }
        for c in &mut s.substructures {
            self.extension_content(c, false);
        }
    }

    fn drop_xref(&mut self, s: &mut Structure) {
        if let Some(x) = s.xref.take() {
            self.repair(
                s.line,
                RepairKind::Xref,
                format!(
                    "identifier {x} of {} left out: only records have one",
                    s.tag.as_str()
                ),
            );
        }
    }

    /// Follows a renamed identifier. `Ok(Some(type))` for a record of a
    /// standard type, `Ok(None)` for an extension record, `@VOID@` (7.x) or a
    /// 5.5.1 pointer to another file; `Err` for a pointer to nothing.
    fn resolve(&mut self, s: &mut Structure, p: &str) -> Result<Option<StructId>, ()> {
        let p = match self.renamed.get(p) {
            Some(new) => {
                let new = new.to_string();
                s.payload = Payload::Pointer(Xref::new(new.as_str()));
                new
            }
            None => p.to_string(),
        };
        if self.family == Family::V7 && p == "@VOID@" || is_external_pointer(&p, self.family) {
            return Ok(None);
        }
        match self.xrefs.get(p.as_str()) {
            Some(ty) => Ok(*ty),
            None => Err(()),
        }
    }

    /// (h) Leaves out banned characters (5.5.1 tabs become spaces).
    fn characters(&mut self, s: &mut Structure) {
        let Payload::Text(t) = &s.payload else { return };
        let (family, rules) = (self.family, self.rules);
        if !t.chars().any(|c| payload::is_banned(c, rules)) {
            return;
        }
        let mut removed = 0;
        let clean: String = t
            .chars()
            .filter_map(|c| match c {
                '\t' if family == Family::V551 => Some(' '),
                c if payload::is_banned(c, rules) => {
                    removed += 1;
                    None
                }
                c => Some(c),
            })
            .collect();
        let what = if removed == 0 {
            "tabs written as spaces".to_string()
        } else {
            format!("{removed} banned character(s) left out")
        };
        let here = self.here(s.tag.as_str());
        self.repair(s.line, RepairKind::Characters, format!("{here}: {what}"));
        s.payload = Payload::Text(clean.into());
    }

    /// Whether `ty` takes a `tag` substructure (in its text form).
    fn takes(&self, ty: StructId, tag: &str) -> Option<StructId> {
        let tag = tag_index(tag)?;
        self.schema
            .subs_tagged(ty, tag)
            .find(|s| {
                !matches!(
                    self.schema.kind(s.id).0,
                    Kind::Pointer | Kind::NullablePointer
                )
            })
            .map(|s| s.id)
    }

    /// Repairs a structure of type `ty` and its substructures.
    fn fix(&mut self, s: &mut Structure, ty: StructId, record: bool) -> Fate {
        if !record {
            self.drop_xref(s);
        }
        if self.payload(s, ty) == Fate::Extension {
            return Fate::Extension;
        }
        let children = std::mem::take(&mut s.substructures);
        let mut kept = Vec::with_capacity(children.len());
        let mut counts: Vec<(u8, u32)> = Vec::new();
        self.path.push(s.tag.as_str().to_string());
        for mut c in children {
            if self.child(&mut c, ty, &mut counts) {
                kept.push(c);
            }
        }
        self.path.pop();
        s.substructures = kept;
        if self.required(s, ty, &counts) == Fate::Extension {
            return Fate::Extension;
        }
        self.special(s, ty);
        // 7.x §1.2: a structure has a payload or a substructure.
        if self.family == Family::V7
            && !record
            && s.substructures.is_empty()
            && s.payload.as_str().is_none_or(str::is_empty)
        {
            let here = self.here(s.tag.as_str());
            if self.schema.kind(ty).0 == Kind::Y {
                s.payload = Payload::Text("Y".into());
                self.repair(
                    s.line,
                    RepairKind::Empty,
                    format!("{here}: empty, written as Y"),
                );
            } else {
                self.repair(
                    s.line,
                    RepairKind::Empty,
                    format!("{here}: empty, left out"),
                );
                return Fate::Remove;
            }
        }
        Fate::Keep
    }

    /// Repairs `c`, a substructure of a structure of type `ty`, counting the
    /// standard ones in `counts`. Returns whether it stays.
    fn child(&mut self, c: &mut Structure, ty: StructId, counts: &mut Vec<(u8, u32)>) -> bool {
        let schema = self.schema;
        let is_ptr = matches!(c.payload, Payload::Pointer(_));
        let tag = c.tag.as_str().to_string();
        match self.picker.pick(ty, &*c, is_ptr) {
            Pick::Type(cty) => {
                // A substructure that needs nothing is left as it is.
                let fate = if self.is_clean(&*c, cty, false) {
                    Fate::Keep
                } else {
                    self.fix(c, cty, false)
                };
                match fate {
                    Fate::Remove => return false,
                    Fate::Extension => self.make_extension(c),
                    Fate::Keep => {
                        let t = schema.tag_id(cty).unwrap_or(0);
                        let n = bump(counts, t);
                        if let Some(max) = self.picker.max(ty, t).filter(|&m| n > u32::from(m)) {
                            let here = self.here(&tag);
                            let (new, declared) = self.relocate(c, Some(cty));
                            self.repair(
                            c.line,
                            RepairKind::Repeated,
                            format!("{here}: more than the {max} permitted, written as {new}{declared}"),
                        );
                        }
                    }
                }
            }
            // A documented alias of a standard structure is that structure,
            // wherever it is (7.x §1.5.1: relocated).
            Pick::Extension => match self.aliases.structs.get(&tag).copied() {
                Some(aty) if self.is_clean(&*c, aty, false) => {}
                Some(aty) => match self.fix(c, aty, false) {
                    Fate::Keep => {}
                    Fate::Remove => return false,
                    Fate::Extension => self.make_extension(c),
                },
                None if self.is_clean_extension(&*c) => {}
                None => self.extension(c),
            },
            other => {
                let here = self.here(&tag);
                let why = match other {
                    Pick::Misplaced => "not permitted here",
                    Pick::Continuation => "a continuation is not a structure",
                    Pick::Unknown => "not a tag of the version",
                    _ => "not a tag",
                };
                // A standard structure that one type only can stand for
                // keeps its meaning where it moves, when it is valid.
                let only = (other == Pick::Misplaced && self.family == Family::V7)
                    .then(|| self.only_type(&tag))
                    .flatten();
                let fate = only.map(|oty| (oty, self.fix(c, oty, false)));
                let (new, declared) = match fate {
                    Some((_, Fate::Remove)) => return false,
                    Some((oty, Fate::Keep)) => self.relocate(c, Some(oty)),
                    Some((_, Fate::Extension)) | None => {
                        self.make_extension(c);
                        (c.tag.as_str().to_string(), String::new())
                    }
                };
                self.repair(
                    c.line,
                    RepairKind::Misplaced,
                    format!("{here}: {why}, written as {new}{declared}"),
                );
            }
        }
        true
    }

    /// The structure type a tag names when the version has exactly one with
    /// that tag.
    fn only_type(&self, tag: &str) -> Option<StructId> {
        let t = tag_index(tag)?;
        let mut found = None;
        for id in 1..self.schema.structs.len() {
            let Ok(id) = StructId::try_from(id) else {
                break;
            };
            if self.schema.tag_id(id) == Some(t) {
                if found.is_some() {
                    return None;
                }
                found = Some(id);
            }
        }
        found
    }

    /// Adds the `HEAD.SCHMA.TAG` declarations of relocated structures.
    fn declare<N: Node<'n>>(&mut self, records: &mut [Rec<'n, N>]) {
        if self.schma.is_empty() {
            return;
        }
        let Some(head) = records.first_mut().filter(|h| h.tag() == "HEAD") else {
            return;
        };
        head.clean = 0;
        let head = head.make_owned();
        let at = head
            .substructures
            .iter()
            .position(|s| s.tag == "SCHMA")
            .unwrap_or_else(|| {
                head.substructures.push(Structure::new("SCHMA"));
                head.substructures.len() - 1
            });
        let Some(schma) = head.substructures.get_mut(at) else {
            return;
        };
        let declared: HashSet<String> = schma
            .substructures
            .iter()
            .filter(|t| t.tag == "TAG")
            .filter_map(|t| t.text()?.split(' ').next().map(str::to_string))
            .collect();
        for (tag, uri) in &self.schma {
            if !declared.contains(tag) {
                schma
                    .substructures
                    .push(text("TAG", &format!("{tag} {uri}")));
            }
        }
    }

    /// The payload of a structure of type `ty`.
    fn payload(&mut self, s: &mut Structure, ty: StructId) -> Fate {
        let (kind, arg) = self.schema.kind(ty);
        let here = self.here(s.tag.as_str());
        if matches!(kind, Kind::Pointer | Kind::NullablePointer) {
            return self.pointer_payload(s, ty, arg, &here);
        }
        if let Payload::Pointer(p) = &s.payload {
            let p = p.as_str().to_string();
            self.repair(
                s.line,
                RepairKind::Pointer,
                format!("{here} {p}: a pointer where text belongs, written as text"),
            );
            s.payload = Payload::Text(p.into());
        }
        self.characters(s);
        let raw = s.text().unwrap_or("").to_string();
        let set = matches!(kind, Kind::Enum | Kind::ListEnum)
            .then(|| self.schema.enum_set(arg))
            .flatten();
        match (kind, set) {
            (Kind::Enum, Some(set)) if !raw.is_empty() => {
                return self.enumeration(s, ty, set, &raw, &here)
            }
            (Kind::ListEnum, Some(set)) if !raw.is_empty() => {
                return self.list(s, set, &raw, &here)
            }
            _ => {}
        }
        let error = if raw.is_empty() && payload::empty_is_valid(kind) {
            None
        } else if matches!(kind, Kind::Date | Kind::DateExact | Kind::DatePeriod) {
            payload::check(kind, set, &self.aliases.unalias(&raw), self.family)
        } else {
            payload::check(kind, set, &raw, self.family)
        };
        let mime_text = self.family == Family::V7
            && self.schema.name(ty) == "MIME"
            && !raw.is_empty()
            && !raw.to_ascii_lowercase().starts_with("text/");
        match error.or_else(|| mime_text.then(|| "the media type of a text is text/…".into())) {
            Some(why) => self.invalid_payload(s, ty, &raw, &why, &here),
            None => Fate::Keep,
        }
    }

    /// (d) A payload outside its type's grammar, `why`.
    fn invalid_payload(
        &mut self,
        s: &mut Structure,
        ty: StructId,
        raw: &str,
        why: &str,
        here: &str,
    ) -> Fate {
        let kind = self.schema.kind(ty).0;
        let line = s.line;
        match kind {
            // A date or an age: PHRASE in 7.x, a date phrase in 5.5.1.
            Kind::Date | Kind::DatePeriod | Kind::Age if self.family == Family::V7 => {
                if self.takes(ty, "PHRASE").is_some() && s.first("PHRASE").is_none() {
                    s.payload = Payload::None;
                    s.substructures.push(text("PHRASE", raw));
                    self.repair(
                        line,
                        RepairKind::Payload,
                        format!("{here} {raw:?}: {why}; moved to PHRASE"),
                    );
                    return Fate::Keep;
                }
            }
            Kind::Date => {
                let phrase = format!("({raw})");
                if payload::check(kind, None, &phrase, self.family).is_none() {
                    self.repair(
                        line,
                        RepairKind::Payload,
                        format!("{here} {raw:?}: {why}; written as the date phrase {phrase}"),
                    );
                    s.payload = Payload::Text(phrase.into());
                    return Fate::Keep;
                }
            }
            // An event with text: `Y` and a note; text where none belongs:
            // a note.
            Kind::Y | Kind::None if self.takes(ty, "NOTE").is_some() => {
                let (payload, to) = if kind == Kind::Y {
                    (Payload::Text("Y".into()), "Y")
                } else {
                    (Payload::None, "nothing")
                };
                s.payload = payload;
                s.substructures.push(text("NOTE", raw));
                self.repair(
                    line,
                    RepairKind::Payload,
                    format!("{here} {raw:?}: {why}; payload written as {to}, the text as a NOTE"),
                );
                return Fate::Keep;
            }
            _ => {}
        }
        let new = self.plain_tag(s.tag.as_str());
        self.repair(
            line,
            RepairKind::Payload,
            format!("{here} {raw:?}: {why}; written as {new}"),
        );
        Fate::Extension
    }

    /// (a, b) An enumeration value.
    fn enumeration(
        &mut self,
        s: &mut Structure,
        ty: StructId,
        set: &super::schema::EnumSet,
        raw: &str,
        here: &str,
    ) -> Fate {
        if let Some(fixed) = canonical(set, raw, self.family) {
            if fixed != raw {
                self.repair(
                    s.line,
                    RepairKind::EnumCase,
                    format!("{here} {raw:?} written as {fixed}"),
                );
                s.payload = Payload::Text(fixed.into());
            }
            return Fate::Keep;
        }
        if self.family == Family::V7
            && set.values.contains(&"OTHER")
            && self.takes(ty, "PHRASE").is_some()
            && s.first("PHRASE").is_none()
        {
            s.payload = Payload::Text("OTHER".into());
            s.substructures.push(text("PHRASE", raw));
            self.repair(
                s.line,
                RepairKind::EnumValue,
                format!(
                    "{here} {raw:?}: not a value of {}; written as OTHER with a PHRASE",
                    set.name
                ),
            );
            return Fate::Keep;
        }
        let new = extension_tag(self.rules, s.tag.as_str());
        self.repair(
            s.line,
            RepairKind::EnumValue,
            format!(
                "{here} {raw:?}: not a value of {}; written as {new}",
                set.name
            ),
        );
        Fate::Extension
    }

    /// (a, b) A list of enumeration values.
    fn list(
        &mut self,
        s: &mut Structure,
        set: &super::schema::EnumSet,
        raw: &str,
        here: &str,
    ) -> Fate {
        let mut items = Vec::new();
        for item in payload::list_items(raw).filter(|i| !i.is_empty()) {
            let Some(v) = canonical(set, item, self.family) else {
                let new = self.plain_tag(s.tag.as_str());
                self.repair(
                    s.line,
                    RepairKind::EnumValue,
                    format!(
                        "{here} {raw:?}: {item:?} is not a value of {}; written as {new}",
                        set.name
                    ),
                );
                return Fate::Extension;
            };
            items.push(v);
        }
        if items.is_empty() {
            let new = self.plain_tag(s.tag.as_str());
            self.repair(
                s.line,
                RepairKind::EnumValue,
                format!("{here} {raw:?}: no value; written as {new}"),
            );
            return Fate::Extension;
        }
        let fixed = items.join(", ");
        if fixed != raw {
            self.repair(
                s.line,
                RepairKind::EnumCase,
                format!("{here} {raw:?} written as {fixed}"),
            );
            s.payload = Payload::Text(fixed.into());
        }
        Fate::Keep
    }

    /// (c) The payload of a pointer structure.
    fn pointer_payload(
        &mut self,
        s: &mut Structure,
        ty: StructId,
        target: u16,
        here: &str,
    ) -> Fate {
        let kind = self.schema.kind(ty).0;
        let v7 = self.family == Family::V7;
        let line = s.line;
        let new = self.plain_tag(s.tag.as_str());
        let phrase_ok = self.takes(ty, "PHRASE").is_some() && s.first("PHRASE").is_none();
        let to_void = |s: &mut Structure, phrase: Option<String>| {
            s.payload = Payload::Pointer(Xref::new(Xref::VOID));
            if let Some(p) = phrase {
                s.substructures.push(text("PHRASE", &p));
            }
        };
        match s.payload.clone() {
            Payload::Pointer(p) => {
                let p = p.as_str().to_string();
                match self.resolve(s, &p) {
                    Ok(Some(t)) if t == target => Fate::Keep,
                    Ok(None) if v7 && s.pointer().is_some_and(Xref::is_void) => Fate::Keep,
                    Ok(None) if is_external_pointer(&p, self.family) => Fate::Keep,
                    Ok(_) => {
                        let what = self.schema.tag(target);
                        self.repair(
                            line,
                            RepairKind::Pointer,
                            format!("{here} {p}: not a {what} record; written as {new}"),
                        );
                        Fate::Extension
                    }
                    Err(()) if v7 && phrase_ok => {
                        to_void(s, Some(p.clone()));
                        self.repair(
                            line,
                            RepairKind::Pointer,
                            format!("{here} {p}: names no record; written as @VOID@ with a PHRASE"),
                        );
                        Fate::Keep
                    }
                    Err(()) => {
                        s.payload = Payload::Text(p.as_str().into());
                        self.repair(
                            line,
                            RepairKind::Pointer,
                            format!("{here} {p}: names no record; written as {new} with the text"),
                        );
                        Fate::Extension
                    }
                }
            }
            Payload::Text(t) if !t.is_empty() => {
                self.characters(s);
                let t = s.text().unwrap_or("").to_string();
                if v7 && phrase_ok {
                    to_void(s, Some(t.clone()));
                    self.repair(line, RepairKind::Pointer, format!("{here} {t:?}: text where a pointer belongs; written as @VOID@ with a PHRASE"));
                    Fate::Keep
                } else {
                    self.repair(
                        line,
                        RepairKind::Pointer,
                        format!("{here} {t:?}: text where a pointer belongs; written as {new}"),
                    );
                    Fate::Extension
                }
            }
            _ if kind == Kind::NullablePointer => {
                s.payload = Payload::None;
                Fate::Keep
            }
            _ if v7 => {
                to_void(s, None);
                self.repair(
                    line,
                    RepairKind::Pointer,
                    format!("{here}: no pointer; written as @VOID@"),
                );
                Fate::Keep
            }
            _ => {
                self.repair(
                    line,
                    RepairKind::Pointer,
                    format!("{here}: no pointer; written as {new}"),
                );
                Fate::Extension
            }
        }
    }

    /// (g) Required substructures.
    fn required(&mut self, s: &mut Structure, ty: StructId, counts: &[(u8, u32)]) -> Fate {
        let schema = self.schema;
        let required = self
            .tables
            .required
            .get(usize::from(ty))
            .map_or(&[][..], |r| r);
        for &(t, min, sub) in required {
            let found = counts.iter().find(|(k, _)| *k == t).map_or(0, |(_, n)| *n);
            if found >= u32::from(min) {
                continue;
            }
            let tag = schema.tag(sub);
            let here = self.here(s.tag.as_str());
            let Some(made) = self.synthesise(s, sub) else {
                let new = self.plain_tag(s.tag.as_str());
                self.repair(
                    s.line,
                    RepairKind::Required,
                    format!("{here} lacks {tag}, which nothing can stand for; written as {new}"),
                );
                return Fate::Extension;
            };
            let shown = made.to_gedcom(0, self.rules.version);
            let shown = shown
                .lines()
                .next()
                .unwrap_or("")
                .trim_start_matches("0 ")
                .to_string();
            self.repair(
                0,
                RepairKind::Required,
                format!("{here} lacks {tag}: {shown} added"),
            );
            s.substructures.push(made);
        }
        Fate::Keep
    }

    /// The minimal structure that stands for a missing required `sub` of
    /// `parent`, from the documented list; `None` when there is none.
    fn synthesise(&mut self, parent: &Structure, sub: StructId) -> Option<Structure> {
        let schema = self.schema;
        let v7 = self.family == Family::V7;
        let tag = schema.tag(sub);
        let parent_tag = parent.tag.as_str();
        let made = match (parent_tag, tag) {
            (_, "FORM") if parent_tag == "FILE" || parent_tag == "TRAN" => {
                let file = parent.text().unwrap_or("");
                if v7 {
                    text("FORM", media_type(file))
                } else {
                    text("FORM", multimedia_format(file)?)
                }
            }
            ("TRAN", "LANG") if v7 => text("LANG", "und"),
            ("ASSO", "ROLE") if v7 => text("ROLE", "OTHER"),
            ("SLGC", "FAMC") if v7 => pointer("FAMC", Xref::VOID),
            _ => {
                let (kind, arg) = schema.kind(sub);
                let open = kind == Kind::Enum && schema.enum_set(arg).is_some_and(|e| e.open);
                // A placeholder only stands for a leaf: a structure with
                // required parts of its own (a FILE needs its FORM) is not
                // invented.
                let leaf = schema.subs(sub).iter().all(|s| s.min == 0);
                if (kind == Kind::Text || open) && leaf {
                    text(tag, PLACEHOLDER)
                } else {
                    return None;
                }
            }
        };
        Some(made)
    }

    /// Rules the tables cannot state: a 7.x note translation says its
    /// language or its media type.
    fn special(&mut self, s: &mut Structure, ty: StructId) {
        if self.family == Family::V7
            && self.schema.name(ty) == "NOTE-TRAN"
            && !s
                .substructures
                .iter()
                .any(|c| c.tag == "MIME" || c.tag == "LANG")
        {
            let here = self.here(s.tag.as_str());
            self.repair(
                0,
                RepairKind::Required,
                format!("{here} lacks MIME and LANG: LANG und added"),
            );
            s.substructures.push(text("LANG", "und"));
        }
    }

    /// (j) Moves inline notes out of 5.5.1 records over 32K, longest first,
    /// until the record fits; a record that moving every note would not
    /// bring under the limit is left as it is.
    fn record_sizes<N: Node<'n>>(&mut self, records: &mut Vec<Rec<'n, N>>) {
        let mut added = Vec::new();
        for rec in records.iter_mut() {
            let Some(ty) = self.record_of_standard(rec.tag(), rec.standard_tag()) else {
                continue;
            };
            // The bound the last look found, when there is one.
            if rec.size as usize <= MAX_RECORD_BYTES {
                continue;
            }
            let mut size = with_node!(rec.get(), |n| encoded_len(n, 0, self.family));
            if size <= MAX_RECORD_BYTES {
                continue;
            }
            let r = rec.make_owned();
            let mut notes = Vec::new();
            movable_notes(self.rules, r, ty, &mut Vec::new(), &mut notes);
            // The bytes each move saves: the note's lines, less the pointer
            // line left in its place (`n NOTE @N…@`, at most 32 bytes).
            let saved = |n: &(usize, Vec<usize>)| n.0.saturating_sub(32);
            if size.saturating_sub(notes.iter().map(saved).sum()) > MAX_RECORD_BYTES {
                continue;
            }
            // Longest first; between equals, file order.
            notes.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            let tag = r.tag.as_str().to_string();
            for note in &notes {
                if size <= MAX_RECORD_BYTES {
                    break;
                }
                size = size.saturating_sub(saved(note));
                let xref = self.unique("@N1@", self.record_of("NOTE"));
                let Some(at) = node_at(r, &note.1) else {
                    continue;
                };
                let line = at.line;
                let moved = Structure {
                    xref: Some(Xref::new(xref.as_str())),
                    payload: std::mem::replace(
                        &mut at.payload,
                        Payload::Pointer(Xref::new(xref.as_str())),
                    ),
                    substructures: std::mem::take(&mut at.substructures),
                    ..Structure::new("NOTE")
                };
                self.repair(
                    line,
                    RepairKind::RecordSize,
                    format!("a {tag} record over 32K: a note moved to the NOTE record {xref}"),
                );
                added.push(moved);
            }
            (rec.clean, rec.size) = (0, u32::MAX);
        }
        if !added.is_empty() {
            let at = records.len().saturating_sub(1);
            records.splice(at..at, added.into_iter().map(Rec::owned));
        }
    }
}

/// The extension tags of a record and its substructures.
fn extension_tags<'n, N: Node<'n>>(r: &Rec<'n, N>) -> HashSet<String> {
    fn walk<'a, M: Node<'a>>(n: M, out: &mut HashSet<String>) {
        let mut stack = vec![n];
        while let Some(n) = stack.pop() {
            let tag = n.tag();
            if tag.starts_with('_') && !out.contains(tag) {
                out.insert(tag.to_string());
            }
            stack.extend(n.children());
        }
    }
    let mut out = HashSet::new();
    with_node!(r.get(), |n| walk(n, &mut out));
    out
}

/// The spelling of `value` in `set`: 5.5.1 matches without regard to case
/// (p. 21) and writes the specification's spelling; 7.x upper-cases (and
/// keeps extension values, upper-cased). `None` when no value matches.
fn canonical(set: &super::schema::EnumSet, value: &str, family: Family) -> Option<String> {
    if set.open {
        return Some(value.to_string());
    }
    if let Some(v) = set.values.iter().find(|v| v.eq_ignore_ascii_case(value)) {
        return Some((*v).to_string());
    }
    if family == Family::V7 {
        let upper = value.to_ascii_uppercase();
        if is_ext_tag(&upper) {
            return Some(upper);
        }
    }
    None
}

/// Counts one more occurrence of tag `t`; returns the count.
fn bump(counts: &mut Vec<(u8, u32)>, t: u8) -> u32 {
    if let Some((_, n)) = counts.iter_mut().find(|(k, _)| *k == t) {
        *n += 1;
        *n
    } else {
        counts.push((t, 1));
        1
    }
}

/// The structure at `path` (indices of substructures) under `r`.
fn node_at<'s>(r: &'s mut Structure, path: &[usize]) -> Option<&'s mut Structure> {
    let mut cur = r;
    for &i in path {
        cur = cur.substructures.get_mut(i)?;
    }
    Some(cur)
}

/// The inline notes under `s` (of type `ty`) whose superstructure also
/// takes a note pointer: (size as written, path).
fn movable_notes(
    rules: &VersionRules,
    s: &Structure,
    ty: StructId,
    path: &mut Vec<usize>,
    out: &mut Vec<(usize, Vec<usize>)>,
) {
    let (schema, family) = (rules.spec, Family::of(rules));
    let takes_pointer = matches!(
        pick(rules, ty, "NOTE", true),
        Pick::Type(id) if schema.kind(id).0 == Kind::Pointer
    );
    for (i, c) in s.substructures.iter().enumerate() {
        let is_ptr = matches!(c.payload, Payload::Pointer(_));
        let Pick::Type(cty) = pick(rules, ty, c.tag.as_str(), is_ptr) else {
            continue;
        };
        path.push(i);
        if c.tag == "NOTE" && takes_pointer && c.text().is_some() {
            out.push((encoded_len(c, path.len(), family), path.clone()));
        } else {
            movable_notes(rules, c, cty, path, out);
        }
        path.pop();
    }
}
