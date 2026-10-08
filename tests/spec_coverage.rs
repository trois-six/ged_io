//! The coverage ledger of the typed model (`ged_io::next`), checked against
//! the specification tables (`src/spec/tables.rs`, read here as data).
//!
//! For every typed structure and every structure type of 5.5.1, 7.0 and 7.1
//! it stands for:
//! 1. every substructure the tables permit is a field, or is listed in
//!    [`OPAQUE`] (kept in `extra`) with a reason;
//! 2. a field holding one occurrence (`Option`) stands for a substructure
//!    of cardinality `{0:1}` or `{1:1}`, a field holding many (`Vec`) for
//!    `{0:M}`, `{1:M}` or `{0:3}`;
//! 3. every field's tags are permitted by at least one of its types.
//!
//! For every enumeration type:
//! 4. its sets exist, and each value of a set is a variant spelled exactly
//!    so in that version, and each variant's spelling is a value of one of
//!    its sets (but for the types listed in [`WIDER`]);
//! 5. every enumeration set of the tables has a type, but those listed in
//!    [`TEXT_SETS`].
//!
//! Cardinalities thus come from the specifications and cannot drift.

#[allow(dead_code)]
#[path = "../src/spec/schema.rs"]
mod schema;
#[allow(dead_code)]
#[path = "../src/spec/tables.rs"]
#[rustfmt::skip]
mod tables;

use std::collections::{BTreeSet, HashMap};

use ged_io::next::ledger::{self, FieldDesc};
use schema::{Schema, StructId};

/// Substructures of typed structures kept in `extra` rather than in a
/// field: (structure, tag, reason).
const OPAQUE: &[(&str, &str, &str)] = &[];

/// Enumeration types whose spellings in a version go beyond its sets:
/// (type, version, reason).
const WIDER: &[(&str, &str, &str)] = &[(
    "EventKind",
    "5.5.1",
    "5.5.1 cites events and attributes by tag, as text; its ATTRIBUTE_TYPE set lists the attributes only",
)];

/// Enumeration sets modelled as text: (set, reason).
const TEXT_SETS: &[(&str, &str)] = &[(
    "LANGUAGE_ID",
    "a language is text: 7.x tags it in BCP 47, an open grammar; the 5.5.1 names are kept as written",
)];

fn versions() -> [(&'static str, &'static Schema); 3] {
    [
        ("5.5.1", &tables::V551),
        ("7.0", &tables::V70),
        ("7.1", &tables::V71),
    ]
}

fn names_of(spec: &ledger::SpecNames, version: &str) -> &'static [&'static str] {
    match version {
        "5.5.1" => spec.v551,
        "7.0" => spec.v70,
        _ => spec.v71,
    }
}

fn ids(schema: &Schema) -> impl Iterator<Item = StructId> + '_ {
    (0..schema.structs.len()).filter_map(|i| StructId::try_from(i).ok())
}

fn by_name(schema: &Schema) -> HashMap<&'static str, StructId> {
    ids(schema).map(|id| (schema.name(id), id)).collect()
}

fn field_of<'f>(fields: &'f [FieldDesc], tag: &str) -> Option<&'f FieldDesc> {
    fields.iter().find(|f| f.tags.contains(&tag))
}

#[test]
fn every_substructure_is_a_field_with_the_right_cardinality() {
    let mut problems = Vec::new();
    let mut checked = 0;
    for (name, spec, fields) in ledger::structures() {
        let mut permitted: BTreeSet<&str> = BTreeSet::new();
        let mut typed = 0;
        for (version, schema) in versions() {
            let types = by_name(schema);
            for type_name in names_of(spec, version) {
                typed += 1;
                let Some(&id) = types.get(type_name) else {
                    problems.push(format!("{name}: {version} has no type {type_name}"));
                    continue;
                };
                for sub in schema.subs(id) {
                    let tag = schema.tag(sub.id);
                    permitted.insert(tag);
                    checked += 1;
                    let Some(field) = field_of(fields, tag) else {
                        if !OPAQUE.iter().any(|(s, t, _)| s == name && *t == tag) {
                            problems
                                .push(format!("{name}: {version} {type_name}.{tag} has no field"));
                        }
                        continue;
                    };
                    let many = sub.max != 1;
                    if field.many != many {
                        problems.push(format!(
                            "{name}.{}: {} for {version} {type_name}.{tag} {{{}:{}}}",
                            field.name,
                            if field.many { "Vec" } else { "Option" },
                            sub.min,
                            if sub.max == 0 {
                                "M".to_string()
                            } else {
                                sub.max.to_string()
                            },
                        ));
                    }
                }
            }
        }
        assert!(typed > 0, "{name} stands for no structure type");
        for field in *fields {
            if !field.tags.iter().any(|t| permitted.contains(t)) {
                problems.push(format!(
                    "{name}.{}: no type permits {:?}",
                    field.name, field.tags
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    assert!(checked > 100, "only {checked} substructures checked");
}

#[test]
fn every_enumeration_value_is_a_variant() {
    let mut problems = Vec::new();
    let mut covered: BTreeSet<(&str, &str)> = BTreeSet::new();
    for desc in ledger::enumerations() {
        for (version, schema) in versions() {
            let v7 = version != "5.5.1";
            let sets = if v7 { desc.v7 } else { desc.v551 };
            let spelling = |v: &ledger::EnumValue| if v7 { v.v7 } else { v.v551 };
            let mut values_of_sets: BTreeSet<&str> = BTreeSet::new();
            for set in sets {
                let Some(e) = schema.enums.iter().find(|e| e.name == *set) else {
                    // 7.1 sets 7.0 does not have.
                    if version != "7.0" {
                        problems.push(format!("{}: {version} has no set {set}", desc.name));
                    }
                    continue;
                };
                covered.insert((version, set));
                for value in e.values {
                    values_of_sets.insert(value);
                    if !desc.values.iter().any(|v| spelling(v) == Some(value)) {
                        problems.push(format!(
                            "{}: no variant spelled {value:?} in {version} ({set})",
                            desc.name
                        ));
                    }
                }
            }
            if sets.is_empty() {
                continue;
            }
            for v in desc.values {
                if let Some(s) = spelling(v) {
                    let in_7_1_only = version == "7.0" && values_of_sets.is_empty();
                    let wider = WIDER
                        .iter()
                        .any(|(t, v, _)| *t == desc.name && *v == version);
                    if !values_of_sets.contains(s) && !in_7_1_only && !wider {
                        problems.push(format!(
                            "{}::{}: {s:?} is not a {version} value of {sets:?}",
                            desc.name, v.variant
                        ));
                    }
                }
            }
        }
    }
    for (version, schema) in versions() {
        for e in schema.enums {
            let text = TEXT_SETS.iter().any(|(s, _)| *s == e.name);
            if !text && !covered.contains(&(version, e.name)) {
                problems.push(format!("{version} set {} has no type", e.name));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
