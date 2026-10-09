//! `Serialize` and `Deserialize` for the typed model (feature `serde`).
//!
//! A text or an identifier of the model means something only with the store
//! it points into, so the model is serialized with its store: a
//! [`Dataset`] (and a [`StreamedRecord`](crate::StreamedRecord)) serializes
//! its records with their texts and identifiers as strings, and a dataset
//! deserializes into a fresh store, its texts owned and its identifiers
//! interned. The traits below carry the store through every type; the
//! macros that declare the model implement them for its structures.

use std::borrow::Cow;
use std::fmt;
use std::marker::PhantomData;

use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::version::GedcomVersion;

use super::dataset::{Dataset, Record};
use super::list::ThinVec;
use super::node::{Extra, Node, Value};
use super::text::{Store, TagId, Text, XrefId};

/// A value of the model, serialized with the store its texts point into.
pub(crate) trait SerializeIn {
    /// Serializes the value, its texts and identifiers read from `store`.
    fn serialize_in<S: Serializer>(&self, store: &Store, s: S) -> Result<S::Ok, S::Error>;

    /// Whether the value is empty: a field that is, is left out.
    fn is_empty_in(&self) -> bool {
        false
    }
}

/// A value of the model, deserialized into a store.
pub(crate) trait DeserializeIn: Sized {
    /// Deserializes the value, its identifiers and tags interned in
    /// `store`, its texts owned.
    fn deserialize_in<'de, D: Deserializer<'de>>(store: &mut Store, d: D)
        -> Result<Self, D::Error>;
}

/// A value and its store, to serialize.
pub(crate) struct In<'a, T: ?Sized>(pub(crate) &'a Store, pub(crate) &'a T);

impl<T: SerializeIn + ?Sized> Serialize for In<'_, T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.1.serialize_in(self.0, s)
    }
}

/// A store to deserialize a value of type `T` into.
pub(crate) struct Seed<'s, T>(&'s mut Store, PhantomData<T>);

impl<'s, T> Seed<'s, T> {
    pub(crate) fn new(store: &'s mut Store) -> Self {
        Self(store, PhantomData)
    }
}

impl<'de, T: DeserializeIn> DeserializeSeed<'de> for Seed<'_, T> {
    type Value = T;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<T, D::Error> {
        T::deserialize_in(self.0, d)
    }
}

/// A string, map key or value: borrowed when the format can.
pub(crate) struct Str<'de>(pub(crate) Cow<'de, str>);

impl<'de> Deserialize<'de> for Str<'de> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Str<'de>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a string")
            }
            fn visit_borrowed_str<E: de::Error>(self, v: &'de str) -> Result<Str<'de>, E> {
                Ok(Str(Cow::Borrowed(v)))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Str<'de>, E> {
                Ok(Str(Cow::Owned(v.to_string())))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Str<'de>, E> {
                Ok(Str(Cow::Owned(v)))
            }
        }
        d.deserialize_str(V)
    }
}

/// Skips the value of a map entry no field takes.
pub(crate) fn skip<'de, A: MapAccess<'de>>(map: &mut A) -> Result<(), A::Error> {
    map.next_value::<IgnoredAny>().map(|_| ())
}

/// An enumeration value no variant names: `{"Unknown": "text"}`.
pub(crate) fn serialize_unknown<S: Serializer>(
    text: &Text,
    store: &Store,
    s: S,
) -> Result<S::Ok, S::Error> {
    let mut map = s.serialize_map(Some(1))?;
    map.serialize_entry("Unknown", &In(store, text))?;
    map.end()
}

/// An enumeration value: a variant's name, or `{"Unknown": "text"}`.
pub(crate) fn deserialize_enum<'de, D: Deserializer<'de>, E>(
    d: D,
    names: &'static [&'static str],
    known: fn(&str) -> Option<E>,
    unknown: fn(Text) -> E,
) -> Result<E, D::Error> {
    struct V<E> {
        names: &'static [&'static str],
        known: fn(&str) -> Option<E>,
        unknown: fn(Text) -> E,
    }
    impl<'de, E> Visitor<'de> for V<E> {
        type Value = E;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a variant's name or {\"Unknown\": text}")
        }
        fn visit_str<Er: de::Error>(self, v: &str) -> Result<E, Er> {
            (self.known)(v).ok_or_else(|| de::Error::unknown_variant(v, self.names))
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<E, A::Error> {
            let mut out = None;
            while let Some(key) = map.next_key::<Str<'de>>()? {
                if &*key.0 == "Unknown" {
                    out = Some((self.unknown)(Text::new(map.next_value::<Str<'de>>()?.0)));
                } else {
                    return Err(de::Error::unknown_variant(&key.0, &["Unknown"]));
                }
            }
            out.ok_or_else(|| de::Error::missing_field("Unknown"))
        }
    }
    d.deserialize_any(V {
        names,
        known,
        unknown,
    })
}

/// A value of a variant that holds one value: `{"Variant": value}`.
pub(crate) fn serialize_tagged<S: Serializer, T: SerializeIn>(
    variant: &'static str,
    value: &T,
    store: &Store,
    s: S,
) -> Result<S::Ok, S::Error> {
    let mut map = s.serialize_map(Some(1))?;
    map.serialize_entry(variant, &In(store, value))?;
    map.end()
}

/// A value of one of two variants that hold one value each:
/// `{"A": a}` or `{"B": b}`.
pub(crate) fn deserialize_either<'de, D, A, B, T>(
    store: &mut Store,
    d: D,
    names: &'static [&'static str; 2],
    a: fn(A) -> T,
    b: fn(B) -> T,
) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    A: DeserializeIn,
    B: DeserializeIn,
{
    struct V<'s, A, B, T> {
        store: &'s mut Store,
        names: &'static [&'static str; 2],
        a: fn(A) -> T,
        b: fn(B) -> T,
    }
    impl<'de, A: DeserializeIn, B: DeserializeIn, T> Visitor<'de> for V<'_, A, B, T> {
        type Value = T;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                f,
                "{{\"{}\": …}} or {{\"{}\": …}}",
                self.names[0], self.names[1]
            )
        }
        fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<T, M::Error> {
            let mut out = None;
            while let Some(key) = map.next_key::<Str<'de>>()? {
                if *key.0 == *self.names[0] {
                    out = Some((self.a)(map.next_value_seed(Seed::new(&mut *self.store))?));
                } else if *key.0 == *self.names[1] {
                    out = Some((self.b)(map.next_value_seed(Seed::new(&mut *self.store))?));
                } else {
                    return Err(de::Error::unknown_variant(&key.0, self.names));
                }
            }
            out.ok_or_else(|| de::Error::invalid_length(0, &"one variant"))
        }
    }
    d.deserialize_map(V { store, names, a, b })
}

impl SerializeIn for Text {
    fn serialize_in<S: Serializer>(&self, store: &Store, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_str(store))
    }

    fn is_empty_in(&self) -> bool {
        self.is_empty()
    }
}

impl DeserializeIn for Text {
    fn deserialize_in<'de, D: Deserializer<'de>>(_: &mut Store, d: D) -> Result<Self, D::Error> {
        Ok(Text::new(Str::deserialize(d)?.0))
    }
}

impl SerializeIn for XrefId {
    fn serialize_in<S: Serializer>(&self, store: &Store, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(store.xref(*self))
    }
}

impl DeserializeIn for XrefId {
    fn deserialize_in<'de, D: Deserializer<'de>>(
        store: &mut Store,
        d: D,
    ) -> Result<Self, D::Error> {
        let xref = Str::deserialize(d)?;
        store
            .intern_xref(&xref.0)
            .ok_or_else(|| de::Error::custom("too many identifiers"))
    }
}

impl SerializeIn for u32 {
    fn serialize_in<S: Serializer>(&self, _: &Store, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u32(*self)
    }
}

impl DeserializeIn for u32 {
    fn deserialize_in<'de, D: Deserializer<'de>>(_: &mut Store, d: D) -> Result<Self, D::Error> {
        u32::deserialize(d)
    }
}

impl<T: SerializeIn> SerializeIn for Box<T> {
    fn serialize_in<S: Serializer>(&self, store: &Store, s: S) -> Result<S::Ok, S::Error> {
        (**self).serialize_in(store, s)
    }

    fn is_empty_in(&self) -> bool {
        (**self).is_empty_in()
    }
}

impl<T: DeserializeIn> DeserializeIn for Box<T> {
    fn deserialize_in<'de, D: Deserializer<'de>>(
        store: &mut Store,
        d: D,
    ) -> Result<Self, D::Error> {
        T::deserialize_in(store, d).map(Box::new)
    }
}

impl<T: SerializeIn> SerializeIn for Option<T> {
    fn serialize_in<S: Serializer>(&self, store: &Store, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Some(v) => s.serialize_some(&In(store, v)),
            None => s.serialize_none(),
        }
    }

    fn is_empty_in(&self) -> bool {
        self.is_none()
    }
}

impl<T: DeserializeIn> DeserializeIn for Option<T> {
    fn deserialize_in<'de, D: Deserializer<'de>>(
        store: &mut Store,
        d: D,
    ) -> Result<Self, D::Error> {
        struct V<'s, T>(&'s mut Store, PhantomData<T>);
        impl<'de, T: DeserializeIn> Visitor<'de> for V<'_, T> {
            type Value = Option<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an optional value")
            }
            fn visit_none<E: de::Error>(self) -> Result<Option<T>, E> {
                Ok(None)
            }
            fn visit_unit<E: de::Error>(self) -> Result<Option<T>, E> {
                Ok(None)
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Option<T>, D::Error> {
                T::deserialize_in(self.0, d).map(Some)
            }
        }
        d.deserialize_option(V(store, PhantomData))
    }
}

/// Serializes the items of a list.
fn serialize_items<'a, T: SerializeIn + 'a, S: Serializer>(
    items: &'a [T],
    store: &Store,
    s: S,
) -> Result<S::Ok, S::Error> {
    let mut seq = s.serialize_seq(Some(items.len()))?;
    for item in items {
        seq.serialize_element(&In(store, item))?;
    }
    seq.end()
}

/// Deserializes the items of a list, each given to `push`.
fn deserialize_items<'de, T: DeserializeIn, D: Deserializer<'de>>(
    store: &mut Store,
    d: D,
    push: &mut dyn FnMut(T),
) -> Result<(), D::Error> {
    struct V<'s, 'p, T>(&'s mut Store, &'p mut dyn FnMut(T));
    impl<'de, T: DeserializeIn> Visitor<'de> for V<'_, '_, T> {
        type Value = ();
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a list")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
            while let Some(item) = seq.next_element_seed(Seed::<T>::new(&mut *self.0))? {
                (self.1)(item);
            }
            Ok(())
        }
    }
    d.deserialize_seq(V(store, push))
}

impl<T: SerializeIn> SerializeIn for [T] {
    fn serialize_in<S: Serializer>(&self, store: &Store, s: S) -> Result<S::Ok, S::Error> {
        serialize_items(self, store, s)
    }

    fn is_empty_in(&self) -> bool {
        self.is_empty()
    }
}

impl<T: SerializeIn> SerializeIn for Vec<T> {
    fn serialize_in<S: Serializer>(&self, store: &Store, s: S) -> Result<S::Ok, S::Error> {
        serialize_items(self, store, s)
    }

    fn is_empty_in(&self) -> bool {
        self.is_empty()
    }
}

impl<T: DeserializeIn> DeserializeIn for Vec<T> {
    fn deserialize_in<'de, D: Deserializer<'de>>(
        store: &mut Store,
        d: D,
    ) -> Result<Self, D::Error> {
        let mut out = Vec::new();
        deserialize_items(store, d, &mut |item| out.push(item))?;
        out.shrink_to_fit();
        Ok(out)
    }
}

impl<T: SerializeIn> SerializeIn for ThinVec<T> {
    fn serialize_in<S: Serializer>(&self, store: &Store, s: S) -> Result<S::Ok, S::Error> {
        serialize_items(self, store, s)
    }

    fn is_empty_in(&self) -> bool {
        self.is_empty()
    }
}

impl<T: DeserializeIn> DeserializeIn for ThinVec<T> {
    fn deserialize_in<'de, D: Deserializer<'de>>(
        store: &mut Store,
        d: D,
    ) -> Result<Self, D::Error> {
        let mut out = ThinVec::new();
        deserialize_items(store, d, &mut |item| out.push(item))?;
        Ok(out)
    }
}

impl SerializeIn for Extra {
    fn serialize_in<S: Serializer>(&self, store: &Store, s: S) -> Result<S::Ok, S::Error> {
        serialize_items(self, store, s)
    }

    fn is_empty_in(&self) -> bool {
        self.is_empty()
    }
}

impl DeserializeIn for Extra {
    fn deserialize_in<'de, D: Deserializer<'de>>(
        store: &mut Store,
        d: D,
    ) -> Result<Self, D::Error> {
        let mut out = Extra::new();
        deserialize_items(store, d, &mut |item| out.push(item))?;
        Ok(out)
    }
}

/// A node: `{"tag", "xref", "pointer" or "text", "children"}`; the payload
/// or identifier a typed structure keeps aside has no tag.
impl SerializeIn for Node {
    fn serialize_in<S: Serializer>(&self, store: &Store, s: S) -> Result<S::Ok, S::Error> {
        let tag = (self.tag != TagId::ASIDE).then(|| store.tag(self.tag));
        let payload = match &self.payload {
            Value::None => None,
            Value::Pointer(p) => Some(("pointer", store.xref(*p).into())),
            Value::Text(t) if t.is_empty() => None,
            Value::Text(t) => Some(("text", t.to_str(store))),
        };
        let count = usize::from(tag.is_some())
            + usize::from(self.xref.is_some())
            + usize::from(payload.is_some())
            + usize::from(!self.children.is_empty());
        let mut map = s.serialize_map(Some(count))?;
        if let Some(tag) = tag {
            map.serialize_entry("tag", tag)?;
        }
        if let Some(xref) = self.xref {
            map.serialize_entry("xref", store.xref(xref))?;
        }
        if let Some((key, value)) = payload {
            map.serialize_entry(key, &value)?;
        }
        if !self.children.is_empty() {
            map.serialize_entry("children", &In(store, &self.children))?;
        }
        map.end()
    }
}

impl DeserializeIn for Node {
    fn deserialize_in<'de, D: Deserializer<'de>>(
        store: &mut Store,
        d: D,
    ) -> Result<Self, D::Error> {
        struct V<'s>(&'s mut Store);
        impl<'de> Visitor<'de> for V<'_> {
            type Value = Node;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a node")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Node, A::Error> {
                let store = self.0;
                let mut node = Node::new(TagId::ASIDE);
                while let Some(key) = map.next_key::<Str<'de>>()? {
                    match &*key.0 {
                        "tag" => node.tag = store.intern_tag(&map.next_value::<Str<'de>>()?.0),
                        "xref" => node.xref = map.next_value_seed(Seed::new(&mut *store))?,
                        "pointer" => {
                            node.payload =
                                Value::Pointer(map.next_value_seed(Seed::new(&mut *store))?);
                        }
                        "text" => {
                            node.payload =
                                Value::Text(map.next_value_seed(Seed::new(&mut *store))?);
                        }
                        "children" => {
                            node.children = map.next_value_seed(Seed::new(&mut *store))?;
                        }
                        _ => skip(&mut map)?,
                    }
                }
                Ok(node)
            }
        }
        d.deserialize_map(V(store))
    }
}

/// A record, externally tagged by its type: `{"Individual": {…}}`.
impl SerializeIn for Record {
    fn serialize_in<S: Serializer>(&self, store: &Store, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(1))?;
        match self {
            Record::Header(r) => map.serialize_entry("Header", &In(store, r))?,
            Record::Individual(r) => map.serialize_entry("Individual", &In(store, r))?,
            Record::Family(r) => map.serialize_entry("Family", &In(store, r))?,
            Record::Source(r) => map.serialize_entry("Source", &In(store, r))?,
            Record::Repository(r) => map.serialize_entry("Repository", &In(store, r))?,
            Record::Multimedia(r) => map.serialize_entry("Multimedia", &In(store, r))?,
            Record::Submitter(r) => map.serialize_entry("Submitter", &In(store, r))?,
            Record::Submission(r) => map.serialize_entry("Submission", &In(store, r))?,
            Record::Note(r) => map.serialize_entry("Note", &In(store, r))?,
            Record::Other(r) => map.serialize_entry("Other", &In(store, r))?,
        }
        map.end()
    }
}

/// Serializes a list of records under `name`, unless it is empty.
fn records<M: SerializeMap, T: SerializeIn>(
    map: &mut M,
    store: &Store,
    name: &'static str,
    list: &[T],
) -> Result<(), M::Error> {
    if list.is_empty() {
        return Ok(());
    }
    map.serialize_entry(name, &In(store, list))
}

impl Serialize for Dataset {
    /// The dataset as a map: `version` (`5.5.1`, `7.0` or `7.1`),
    /// `declared_version` (`HEAD.GEDC.VERS` as read, when the file has
    /// one), `header`, then the lists of records that are not empty, in the
    /// order they are written.
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let store = &self.store;
        let lists = [
            self.submitters.is_empty(),
            self.submissions.is_empty(),
            self.individuals.is_empty(),
            self.families.is_empty(),
            self.notes.is_empty(),
            self.sources.is_empty(),
            self.repositories.is_empty(),
            self.multimedia.is_empty(),
            self.extra.is_empty(),
        ];
        let count = 1
            + usize::from(self.declared_version.is_some())
            + usize::from(self.header.is_some())
            + lists.iter().filter(|empty| !**empty).count();
        let mut map = s.serialize_map(Some(count))?;
        map.serialize_entry("version", self.version.as_str())?;
        if let Some(declared) = &self.declared_version {
            map.serialize_entry("declared_version", &**declared)?;
        }
        if let Some(header) = &self.header {
            map.serialize_entry("header", &In(store, header))?;
        }
        records(&mut map, store, "submitters", &self.submitters)?;
        records(&mut map, store, "submissions", &self.submissions)?;
        records(&mut map, store, "individuals", &self.individuals)?;
        records(&mut map, store, "families", &self.families)?;
        records(&mut map, store, "notes", &self.notes)?;
        records(&mut map, store, "sources", &self.sources)?;
        records(&mut map, store, "repositories", &self.repositories)?;
        records(&mut map, store, "multimedia", &self.multimedia)?;
        records(&mut map, store, "extra", &self.extra)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for Dataset {
    /// A dataset as [`Serialize`] writes it, into a fresh store: its texts
    /// owned, its identifiers and tags interned.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Dataset;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a dataset")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Dataset, A::Error> {
                let mut data = Dataset::default();
                let mut store = Store::default();
                let s = &mut store;
                while let Some(key) = map.next_key::<Str<'de>>()? {
                    match &*key.0 {
                        "version" => {
                            let v = map.next_value::<Str<'de>>()?;
                            data.version = match &*v.0 {
                                "5.5.1" => GedcomVersion::V5_5_1,
                                "7.0" => GedcomVersion::V7_0,
                                "7.1" => GedcomVersion::V7_1,
                                other => {
                                    return Err(de::Error::invalid_value(
                                        de::Unexpected::Str(other),
                                        &"5.5.1, 7.0 or 7.1",
                                    ))
                                }
                            };
                        }
                        "declared_version" => {
                            data.declared_version = Some(map.next_value::<Str<'de>>()?.0.into());
                        }
                        "header" => data.header = map.next_value_seed(Seed::new(&mut *s))?,
                        "submitters" => {
                            data.submitters = map.next_value_seed(Seed::new(&mut *s))?;
                        }
                        "submissions" => {
                            data.submissions = map.next_value_seed(Seed::new(&mut *s))?;
                        }
                        "individuals" => {
                            data.individuals = map.next_value_seed(Seed::new(&mut *s))?;
                        }
                        "families" => data.families = map.next_value_seed(Seed::new(&mut *s))?,
                        "notes" => data.notes = map.next_value_seed(Seed::new(&mut *s))?,
                        "sources" => data.sources = map.next_value_seed(Seed::new(&mut *s))?,
                        "repositories" => {
                            data.repositories = map.next_value_seed(Seed::new(&mut *s))?;
                        }
                        "multimedia" => {
                            data.multimedia = map.next_value_seed(Seed::new(&mut *s))?;
                        }
                        "extra" => data.extra = map.next_value_seed(Seed::new(&mut *s))?,
                        _ => skip(&mut map)?,
                    }
                }
                store.shrink();
                data.store = store;
                Ok(data)
            }
        }
        d.deserialize_map(V)
    }
}

impl Serialize for crate::StreamedRecord {
    /// The record as a map: `version`, `line` and `record`, externally
    /// tagged by its type (`{"Individual": {…}}`).
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let (record, store) = (self.record_owned(), self.store());
        let mut map = s.serialize_map(Some(3))?;
        map.serialize_entry("version", self.version().as_str())?;
        map.serialize_entry("line", &self.line())?;
        map.serialize_entry("record", &In(store, record))?;
        map.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dataset_round_trips() {
        let data = Dataset::parse("0 HEAD\n0 @I1@ INDI\n1 NAME Ann /Example/\n0 TRLR\n");
        let json = serde_json::to_string(&data).unwrap();
        assert_eq!(
            json,
            r#"{"version":"5.5.1","header":{},"individuals":[{"xref":"@I1@","names":[{"value":"Ann /Example/"}]}]}"#
        );
        let back: Dataset = serde_json::from_str(&json).unwrap();
        assert_eq!(back.to_structures(), data.to_structures());
    }
}
