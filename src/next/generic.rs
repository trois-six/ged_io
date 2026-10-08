//! Structures the model does not type yet: records, until each has its
//! own type.
//!
//! A [`Generic`] structure keeps its tag, identifier, payload and
//! substructures; each substructure whose structure type (from the
//! specification tables of the file's version, by its superstructure's
//! type, tag and payload) is one the model types is read as that type — a
//! [`Note`](super::Note), a [`Citation`](super::Citation), an enumeration
//! value — and the others stay generic, typed further down. Nothing is
//! lost: a node that does not fit its type stays generic.

use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::sync::OnceLock;

use crate::spec::schema::{tag_index, Kind, Schema, StructId, DATASET};
use crate::tree::{Structure, Tag, Xref, STANDARD_TAGS};
use crate::version::GedcomVersion;

use super::driver::{FromNode, NodeRef, ReadCx, SpecNames, Struct, ToNodes, WriteCx};
use super::enums::{self, EnumDesc, EnumList, Enumeration, LdsStatus, Phrased};
use super::node::Value;
use super::text::{Source, TagId, XrefId};
use super::{
    Address, Age, Association, CallNumber, ChangeDate, Citation, CitationData, CitedEvent,
    CreationDate, Crop, Date, ExactDate, Exid, File, FileForm, FileTranslation, Map,
    MultimediaLink, Note, NoteTranslation, Period, PhoneticVariation, Place, PlaceTranslation,
    Refn, RepositoryCitation, RomanizedVariation, SourceText, TextTranslation,
};

/// A structure the model does not type yet, with its substructures typed
/// where the model can.
#[derive(Debug)]
pub struct Generic {
    /// The tag.
    pub tag: TagId,
    /// The identifier.
    pub xref: Option<XrefId>,
    /// The payload.
    pub payload: Value,
    /// The substructures, in order.
    pub children: Vec<Child>,
}

/// A substructure of a [`Generic`] structure.
#[derive(Debug)]
pub enum Child {
    /// A substructure of a type the model has.
    Typed(Typed),
    /// Any other substructure.
    Generic(Generic),
}

/// A typed substructure of a [`Generic`] structure: its tag and its value,
/// which [`Typed::get`] gives as its type.
pub struct Typed {
    tag: TagId,
    value: Box<dyn TypedValue>,
}

impl Typed {
    /// The tag the structure was read with.
    #[must_use]
    pub fn tag(&self) -> TagId {
        self.tag
    }

    /// The value, if it is a `T`.
    #[must_use]
    pub fn get<T: Any>(&self) -> Option<&T> {
        self.value.as_any().downcast_ref()
    }
}

impl fmt::Debug for Typed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt_dyn(f)
    }
}

/// A typed value behind [`Typed`].
trait TypedValue {
    fn write(&self, tag: &'static str, cx: &WriteCx<'_>) -> Structure;
    fn as_any(&self) -> &dyn Any;
    fn fmt_dyn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result;
}

impl<T: ToNodes + fmt::Debug + 'static> TypedValue for T {
    fn write(&self, tag: &'static str, cx: &WriteCx<'_>) -> Structure {
        self.to_node(tag, cx)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn fmt_dyn(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// Reads a node as a typed value.
type Ctor = fn(NodeRef<'_>, &mut ReadCx<'_>) -> Option<Box<dyn TypedValue>>;

fn ctor<T: FromNode + ToNodes + fmt::Debug + 'static>(
    node: NodeRef<'_>,
    cx: &mut ReadCx<'_>,
) -> Option<Box<dyn TypedValue>> {
    match T::from_node(node, cx) {
        Some(v) => Some(Box::new(v)),
        None => None,
    }
}

/// The typed structures and the structure types each stands for.
macro_rules! structures {
    ($($ty:ty),* $(,)?) => {
        /// Every typed structure: its name, its structure types and its
        /// fields, for the coverage ledger.
        pub(crate) const STRUCTURES: &[(&str, SpecNames, &[super::driver::FieldDesc])] = &[
            $((stringify!($ty), <$ty as Struct>::SPEC, <$ty as Struct>::FIELDS),)*
        ];
        const CTORS: &[(SpecNames, Ctor)] = &[$((<$ty as Struct>::SPEC, ctor::<$ty>),)*];
    };
}

structures!(
    Address,
    Age,
    Association,
    CallNumber,
    ChangeDate,
    Citation,
    CitationData,
    CitedEvent,
    CreationDate,
    Crop,
    Date,
    ExactDate,
    Exid,
    File,
    FileForm,
    FileTranslation,
    LdsStatus,
    Map,
    MultimediaLink,
    Note,
    NoteTranslation,
    Period,
    PhoneticVariation,
    Place,
    PlaceTranslation,
    Refn,
    RepositoryCitation,
    RomanizedVariation,
    SourceText,
    TextTranslation,
);

/// How an enumeration value stands in a structure type: alone (a leaf), as
/// a list, or with its phrase.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Leaf,
    List,
    Phrased,
}

/// The readers of the enumeration values, for the shapes the tables give
/// each enumeration (other shapes, such as an ordinance status with its
/// date, are structures of their own).
macro_rules! enumerations {
    (leaf: $($l:ident),*; phrased: $($p:ident),*; list: $($s:ident),*;) => {
        const ENUM_CTORS: &[(EnumDesc, Shape, Ctor)] = &[
            $((<enums::$l as Enumeration>::DESC, Shape::Leaf, ctor::<enums::$l>),)*
            $((<enums::$p as Enumeration>::DESC, Shape::Phrased, ctor::<Phrased<enums::$p>>),)*
            $((<enums::$s as Enumeration>::DESC, Shape::List, ctor::<EnumList<enums::$s>>),)*
        ];
    };
}

enumerations!(
    leaf: Adoption, BirthKind, Certainty, ChildStatus, GedcomForm, Medium, NameType, OrdinanceFlag,
        Pedigree, PhoneticType, Restriction, Role, RomanizedType, Sex;
    phrased: Adoption, ChildStatus, Medium, NameType, NoteKind, Pedigree, Role;
    list: Restriction;
);

/// What reading a version needs of its tables, computed once: the reader
/// of every structure type the model types, and the substructure types of
/// every type by tag.
struct Registry {
    schema: &'static Schema,
    ctors: Vec<Option<Ctor>>,
    /// For each type, its substructures as (tag identifier, type, whether
    /// the type's payload is a pointer), sorted by tag.
    children: Vec<Box<[(u32, StructId, bool)]>>,
}

fn registry(version: GedcomVersion) -> &'static Registry {
    static V551: OnceLock<Registry> = OnceLock::new();
    static V70: OnceLock<Registry> = OnceLock::new();
    static V71: OnceLock<Registry> = OnceLock::new();
    let cell = match version {
        GedcomVersion::V7_1 => &V71,
        v if v.is_v7() => &V70,
        _ => &V551,
    };
    cell.get_or_init(|| build_registry(version))
}

fn ids(schema: &Schema) -> impl Iterator<Item = StructId> {
    (0..schema.structs.len()).filter_map(|i| StructId::try_from(i).ok())
}

fn build_registry(version: GedcomVersion) -> Registry {
    let schema = version.rules().spec;
    let by_name: HashMap<&str, StructId> = ids(schema).map(|id| (schema.name(id), id)).collect();
    let mut table: Vec<Option<Ctor>> = vec![None; schema.structs.len()];
    for (spec, ctor) in CTORS {
        let names = match version {
            GedcomVersion::V7_1 => spec.v71,
            v if v.is_v7() => spec.v70,
            _ => spec.v551,
        };
        for name in names {
            if let Some(slot) = by_name
                .get(name)
                .and_then(|&id| table.get_mut(usize::from(id)))
            {
                *slot = Some(*ctor);
            }
        }
    }
    let phrase = tag_index("PHRASE");
    for id in ids(schema) {
        let (kind, arg) = schema.kind(id);
        let Some(slot) = table.get_mut(usize::from(id)) else {
            continue;
        };
        if slot.is_some() || !matches!(kind, Kind::Enum | Kind::ListEnum) {
            continue;
        }
        let Some(set) = schema.enum_set(arg) else {
            continue;
        };
        let sets = |d: &EnumDesc| if version.is_v7() { d.v7 } else { d.v551 };
        let subs = schema.subs(id);
        let shape = if subs.is_empty() {
            if kind == Kind::ListEnum {
                Shape::List
            } else {
                Shape::Leaf
            }
        } else if kind == Kind::Enum && subs.iter().all(|s| schema.tag_id(s.id) == phrase) {
            Shape::Phrased
        } else {
            continue;
        };
        *slot = ENUM_CTORS
            .iter()
            .find(|(d, sh, _)| *sh == shape && sets(d).contains(&set.name))
            .map(|(.., ctor)| *ctor);
    }
    let children = ids(schema)
        .map(|id| {
            let mut subs: Vec<(u32, StructId, bool)> = schema
                .subs(id)
                .iter()
                .filter_map(|s| {
                    let tag = crate::tree::standard_index(schema.tag(s.id))?;
                    let pointer =
                        matches!(schema.kind(s.id).0, Kind::Pointer | Kind::NullablePointer);
                    Some((u32::from(tag), s.id, pointer))
                })
                .collect();
            subs.sort_unstable();
            subs.into_boxed_slice()
        })
        .collect();
    Registry {
        schema,
        ctors: table,
        children,
    }
}

impl Registry {
    /// The structure type of a substructure tagged `tag` under a structure
    /// of type `parent`: of two with the tag (a 5.5.1 pointer form and a
    /// text form), the one its payload fits.
    fn child_type(&self, parent: StructId, tag: TagId, pointer: bool) -> Option<StructId> {
        let subs = self.children.get(usize::from(parent))?;
        let tag = tag.get();
        let start = subs.partition_point(|s| s.0 < tag);
        let mut found = None;
        for &(t, id, is_pointer) in subs.get(start..)? {
            if t != tag {
                break;
            }
            if is_pointer == pointer {
                return Some(id);
            }
            found.get_or_insert(id);
        }
        found
    }

    fn ctor(&self, id: StructId) -> Option<Ctor> {
        self.ctors.get(usize::from(id)).copied().flatten()
    }
}

/// The text of a standard tag, for writing.
fn static_tag(id: TagId) -> &'static str {
    STANDARD_TAGS.get(id.get() as usize).copied().unwrap_or("")
}

impl Generic {
    /// The typed substructures that are `T`s, in order.
    pub fn typed<T: Any>(&self) -> impl Iterator<Item = &T> {
        self.children.iter().filter_map(|c| match c {
            Child::Typed(t) => t.get(),
            Child::Generic(_) => None,
        })
    }

    /// The first generic substructure tagged `tag`.
    #[must_use]
    pub fn generic<'a>(&'a self, tag: &str, source: &Source) -> Option<&'a Generic> {
        self.children.iter().find_map(|c| match c {
            Child::Generic(g) if source.tag(g.tag) == tag => Some(g),
            _ => None,
        })
    }

    /// Reads a record (`spec`: its structure type, if the version has it).
    pub(crate) fn read(node: NodeRef<'_>, cx: &mut ReadCx<'_>) -> Self {
        let registry = registry(cx.version);
        let tag = TagId::raw(node.tag_id());
        let spec = registry.child_type(DATASET, tag, node.is_pointer());
        Self::read_typed(node, spec, registry, cx)
    }

    fn read_typed(
        node: NodeRef<'_>,
        spec: Option<StructId>,
        registry: &Registry,
        cx: &mut ReadCx<'_>,
    ) -> Self {
        let n = cx.node_shallow(node);
        let mut children = Vec::with_capacity(node.children().count());
        for child in node.children() {
            let tag = TagId::raw(child.tag_id());
            let ty = spec.and_then(|s| registry.child_type(s, tag, child.is_pointer()));
            let ctor = ty.and_then(|t| registry.ctor(t));
            if let Some(value) = ctor.and_then(|c| c(child, cx)) {
                children.push(Child::Typed(Typed { tag, value }));
            } else {
                children.push(Child::Generic(Self::read_typed(child, ty, registry, cx)));
            }
        }
        Self {
            tag: n.tag,
            xref: n.xref,
            payload: n.payload,
            children,
        }
    }

    /// The structure, written for `cx.version`.
    pub(crate) fn to_structure(&self, cx: &WriteCx<'_>) -> Structure {
        let source: &Source = cx.source;
        Structure {
            tag: Tag::new(source.tag(self.tag)),
            xref: self.xref.map(|x| Xref::new(source.xref(x))),
            payload: match &self.payload {
                Value::None => crate::tree::Payload::None,
                Value::Pointer(p) => cx.pointer(*p),
                Value::Text(t) => cx.text(t),
            },
            substructures: self
                .children
                .iter()
                .map(|c| match c {
                    Child::Typed(t) => t.value.write(static_tag(t.tag), cx),
                    Child::Generic(g) => g.to_structure(cx),
                })
                .collect(),
            line: 0,
        }
    }
}

/// The paths (`INDI/BIRT/DATE`) of the generic structures of `records`
/// whose structure type the model types: structures that did not fit their
/// type. A conformant file has none, but for leaves (types without
/// substructures, read as a value alone) that carry extension
/// substructures, which stay whole.
pub(crate) fn untyped(records: &[Generic], source: &Source, version: GedcomVersion) -> Vec<String> {
    let registry = registry(version);
    let mut out = Vec::new();
    for record in records {
        let pointer = matches!(record.payload, Value::Pointer(_));
        let spec = registry.child_type(DATASET, record.tag, pointer);
        let mut path = vec![source.tag(record.tag)];
        walk_untyped(record, spec, registry, source, &mut path, &mut out);
    }
    out
}

fn walk_untyped<'a>(
    node: &Generic,
    spec: Option<StructId>,
    registry: &Registry,
    source: &'a Source,
    path: &mut Vec<&'a str>,
    out: &mut Vec<String>,
) {
    for child in &node.children {
        let Child::Generic(g) = child else { continue };
        let pointer = matches!(g.payload, Value::Pointer(_));
        let ty = spec.and_then(|s| registry.child_type(s, g.tag, pointer));
        path.push(source.tag(g.tag));
        // A leaf (a type without substructures) with substructures of its
        // own, extensions, is kept whole: leaves have no `extra`.
        let leaf_with_children =
            ty.is_some_and(|t| registry.schema.subs(t).is_empty()) && !g.children.is_empty();
        if !leaf_with_children && ty.is_some_and(|t| registry.ctor(t).is_some()) {
            out.push(path.join("/"));
        }
        walk_untyped(g, ty, registry, source, path, out);
        path.pop();
    }
}
