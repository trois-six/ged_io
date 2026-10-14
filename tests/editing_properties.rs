//! Random edit sequences keep a dataset whole.
//!
//! A small deterministic generator (no dependency) builds fictitious
//! datasets, with pointers of every kind (family links, citations with
//! notes under them, associations, repositories, multimedia links,
//! submitters, extension structures), then runs random edits on them:
//! records added, removed (refused, detached, left dangling), renamed,
//! redirected, linked and unlinked both ways, lent and edited (guards
//! forgotten too), detached, pushed unchecked. After each edit:
//!
//! * the count of pointers to each identifier is the count of pointers
//!   the dataset's structures hold;
//! * handles and positions agree, and a lookup by identifier finds the
//!   first record that has it;
//! * the dangling pointers are those the counts say: none, unless the
//!   sequence asked for some (left dangling, pushed unchecked).
//!
//! And every few edits the dataset is written in each version: the output
//! validates, and writing what it reads back gives the same bytes.

use std::collections::BTreeMap;

use oxidged::model::{
    ChildLink, Citation, CitationDetail, CitationSource, Dataset, EditError, Family, Holder,
    Individual, IndividualRef, Link, Multimedia, Node, Note, Record, RecordHandle, RecordKind,
    RecordType, Removal, Repository, RepositoryCitation, SharedNote, Source, Submitter, Value,
    XrefId,
};
use oxidged::tree::{Payload, Structure};
use oxidged::{GedcomVersion, GedcomWriter};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}

/// A fictitious dataset with pointers of every kind and none dangling.
fn generated(rng: &mut Rng, v7: bool) -> String {
    let people = 3 + rng.below(8);
    let families = 1 + rng.below(4);
    let sources = 1 + rng.below(3);
    let mut s = format!(
        "0 HEAD\n1 GEDC\n2 VERS {}\n1 SUBM @U1@\n0 @U1@ SUBM\n1 NAME Example submitter\n",
        if v7 { "7.0" } else { "5.5.1" }
    );
    let note_tag = if v7 { "SNOTE" } else { "NOTE" };
    s += &format!("0 @N1@ {note_tag} A shared note\n");
    s += "0 @R1@ REPO\n1 NAME Example archive\n";
    for k in 1..=sources {
        s += &format!("0 @S{k}@ SOUR\n1 TITL Register {k}\n1 REPO @R1@\n");
    }
    s += "0 @M1@ OBJE\n1 FILE photo.jpg\n2 FORM image/jpeg\n";
    for i in 1..=people {
        s += &format!("0 @I{i}@ INDI\n1 NAME Given{i} /Example/\n");
        if rng.chance(50) {
            s += &format!(
                "1 SOUR @S{}@\n2 PAGE p. {i}\n2 {note_tag} @N1@\n",
                1 + rng.below(sources)
            );
        }
        if rng.chance(30) {
            s += &format!("1 ASSO @I{}@\n", 1 + rng.below(people));
            s += if v7 {
                "2 ROLE FRIEND\n"
            } else {
                "2 RELA Friend\n"
            };
        }
        if rng.chance(30) {
            s += &format!("1 _GODPARENT @I{}@\n", 1 + rng.below(people));
        }
        if rng.chance(20) {
            s += "1 OBJE @M1@\n";
        }
        if rng.chance(20) {
            s += "1 SUBM @U1@\n";
        }
    }
    for f in 1..=families {
        s += &format!("0 @F{f}@ FAM\n");
        let husband = 1 + rng.below(people);
        let wife = 1 + rng.below(people);
        s += &format!("1 HUSB @I{husband}@\n1 WIFE @I{wife}@\n");
        for _ in 0..rng.below(3) {
            s += &format!("1 CHIL @I{}@\n", 1 + rng.below(people));
        }
        if rng.chance(30) {
            s += &format!("1 {note_tag} @N1@\n");
        }
    }
    s += "0 @L1@ _LOC Sampletown\n1 _REF @I1@\n";
    s += "0 TRLR\n";
    s
}

/// Every pointer of the dataset's structures, by identifier.
fn recount(data: &Dataset) -> BTreeMap<String, usize> {
    fn walk(s: &Structure, out: &mut BTreeMap<String, usize>) {
        if let Payload::Pointer(p) = &s.payload {
            *out.entry(p.as_str().to_string()).or_default() += 1;
        }
        for c in &s.substructures {
            walk(c, out);
        }
    }
    let mut out = BTreeMap::new();
    for s in data.to_structures() {
        walk(&s, &mut out);
    }
    out
}

fn check_positions<T: RecordType + PartialEq + std::fmt::Debug>(data: &Dataset) {
    let all = data.all::<T>();
    let handles: Vec<_> = data.handles::<T>().collect();
    assert_eq!(handles.len(), all.len());
    for (pos, (h, record)) in handles.iter().zip(all).enumerate() {
        assert_eq!(data.position(*h), Some(pos));
        assert_eq!(data.handle_at::<T>(pos), Some(*h));
        assert!(data.get(*h) == Some(record));
        assert_eq!(
            data.get(*h).map(|r| r.into_owned()),
            Some(record.into_owned())
        );
    }
}

/// The invariants: counts, handles, lookups and dangling pointers.
fn check(data: &Dataset, dangling_allowed: bool, step: &str) {
    let expected = recount(data);
    let mut ids: Vec<XrefId> = data.records().filter_map(|r| r.xref()).collect();
    for xref in expected.keys() {
        ids.extend(data.store().find_xref(xref));
    }
    for id in &ids {
        let xref = data.store().xref(*id);
        let n = expected.get(xref).copied().unwrap_or(0);
        assert_eq!(data.references(*id), n, "{step}: count of {xref}");
    }
    check_positions::<Individual>(data);
    check_positions::<Family>(data);
    check_positions::<Source>(data);
    check_positions::<Repository>(data);
    check_positions::<Multimedia>(data);
    check_positions::<Submitter>(data);
    check_positions::<SharedNote>(data);
    check_positions::<Node>(data);
    for record in data.records() {
        let Some(id) = record.xref() else { continue };
        let first = data.records().find(|r| r.xref() == Some(id));
        assert_eq!(
            data.find(id),
            first,
            "{step}: find {}",
            data.store().xref(id)
        );
    }
    let dangling = data.dangling_references();
    let defined: std::collections::HashSet<XrefId> =
        data.records().filter_map(|r| r.xref()).collect();
    let void = data
        .version()
        .is_v7()
        .then(|| data.store().find_xref("@VOID@"))
        .flatten();
    let expected_dangling: usize = expected
        .iter()
        .filter_map(|(xref, n)| {
            let id = data.store().find_xref(xref)?;
            (!defined.contains(&id) && Some(id) != void).then_some(*n)
        })
        .sum();
    assert_eq!(dangling.len(), expected_dangling, "{step}: dangling");
    if !dangling_allowed {
        assert!(dangling.is_empty(), "{step}: {dangling:?}");
    }
}

/// Writes in each version: conformant, and stable through a read back.
fn check_writes(data: &Dataset, step: &str) {
    for version in [
        GedcomVersion::V5_5_1,
        GedcomVersion::V7_0,
        GedcomVersion::V7_1,
    ] {
        let writer = GedcomWriter::new().gedcom_version(version);
        let out = writer.write_to_string(data).unwrap();
        let deviations = oxidged::spec::validate_text(&out);
        assert!(
            deviations.is_empty(),
            "{step} {version}: {deviations:?}\n{out}"
        );
        let again = writer
            .write_to_string(&Dataset::parse(out.as_str()))
            .unwrap();
        assert!(again == out, "{step} {version}: not stable\n{out}\n{again}");
    }
}

/// A random record of the dataset, any type.
fn any_record(data: &Dataset, rng: &mut Rng) -> Option<RecordHandle> {
    let all: Vec<RecordHandle> = data.records_with_handles().map(|(h, _)| h).collect();
    (!all.is_empty()).then(|| all[rng.below(all.len())])
}

fn any_of<T: RecordType>(data: &Dataset, rng: &mut Rng) -> Option<oxidged::model::Handle<T>> {
    let n = data.all::<T>().len();
    (n > 0).then(|| data.handle_at::<T>(rng.below(n)).unwrap())
}

fn xref_of<T: RecordType>(data: &Dataset, rng: &mut Rng) -> Option<XrefId> {
    let h = any_of::<T>(data, rng)?;
    data.get_any(h.erase()).and_then(|r| r.xref())
}

const LINKS: [Link; 15] = [
    Link::ChildOf,
    Link::SpouseOf,
    Link::Husband,
    Link::Wife,
    Link::Child,
    Link::Alias,
    Link::Association,
    Link::Citation,
    Link::Note,
    Link::Multimedia,
    Link::Repository,
    Link::Submitter,
    Link::AncestorInterest,
    Link::DescendantInterest,
    Link::Submission,
];

/// One random edit; whether it may have left pointers dangling.
fn edit(data: &mut Dataset, rng: &mut Rng, dangling_allowed: bool, fresh: &mut usize) -> String {
    match rng.below(16) {
        0 => {
            let mut indi = Individual::default();
            if let Some(f) = xref_of::<Family>(data, rng) {
                indi.child_of.push(ChildLink::new(f));
            }
            if let Some(s) = xref_of::<Source>(data, rng) {
                let mut c = Citation {
                    source: CitationSource::Pointer(s),
                    ..Citation::default()
                };
                if let Some(n) = xref_of::<SharedNote>(data, rng) {
                    c.detail = Some(Box::new(CitationDetail {
                        notes: vec![Note::shared(n)],
                        ..CitationDetail::default()
                    }));
                }
                indi.citations.push(c);
            }
            if let Some(i) = xref_of::<Individual>(data, rng) {
                let tag = data.store_mut().intern_tag("_GODPARENT");
                indi.extra.push(Node {
                    payload: Value::Pointer(i),
                    ..Node::new(tag)
                });
            }
            let r = data.add(indi);
            assert!(r.is_ok(), "{r:?}");
            "add individual".into()
        }
        1 => {
            let mut fam = Family {
                husband: xref_of::<Individual>(data, rng).map(IndividualRef::new),
                ..Family::default()
            };
            for _ in 0..rng.below(3) {
                if let Some(c) = xref_of::<Individual>(data, rng) {
                    fam.children.push(IndividualRef::new(c));
                }
            }
            data.add(fam).unwrap();
            "add family".into()
        }
        2 => {
            match rng.below(5) {
                0 => {
                    let mut s = Source::default();
                    if let Some(r) = xref_of::<Repository>(data, rng) {
                        s.repositories.push(RepositoryCitation {
                            repository: Some(r),
                            ..RepositoryCitation::default()
                        });
                    }
                    data.add(s).unwrap();
                }
                1 => {
                    data.add(SharedNote {
                        text: "A note".into(),
                        ..SharedNote::default()
                    })
                    .unwrap();
                }
                2 => {
                    data.add(Repository::default()).unwrap();
                }
                3 => {
                    data.add(Submitter::default()).unwrap();
                }
                _ => {
                    data.add(Multimedia::default()).unwrap();
                }
            }
            "add another record".into()
        }
        3 | 4 => {
            let Some(holder) = any_record(data, rng) else {
                return "nothing to link".into();
            };
            let holder: Holder = if rng.chance(10) {
                Holder::Header
            } else {
                holder.into()
            };
            let link = if rng.chance(10) {
                Link::Extension(data.store_mut().intern_tag("_LINK"))
            } else {
                LINKS[rng.below(LINKS.len())]
            };
            // Mostly a record of the type the link points to.
            let target = if rng.chance(20) {
                any_record(data, rng)
                    .and_then(|h| data.get_any(h))
                    .and_then(|r| r.xref())
            } else {
                match link {
                    Link::ChildOf | Link::SpouseOf => xref_of::<Family>(data, rng),
                    Link::Citation => xref_of::<Source>(data, rng),
                    Link::Note => xref_of::<SharedNote>(data, rng),
                    Link::Multimedia => xref_of::<Multimedia>(data, rng),
                    Link::Repository => xref_of::<Repository>(data, rng),
                    Link::Submitter | Link::AncestorInterest | Link::DescendantInterest => {
                        xref_of::<Submitter>(data, rng)
                    }
                    Link::Extension(_) => xref_of::<Node>(data, rng),
                    _ => xref_of::<Individual>(data, rng),
                }
            };
            let r = data.link(holder, link, target);
            let u = rng.chance(40).then(|| data.unlink(holder, link, target));
            format!("link {link:?} {r:?} {u:?}")
        }
        5 => {
            let (Some(fam), Some(indi)) =
                (any_of::<Family>(data, rng), any_of::<Individual>(data, rng))
            else {
                return "no family".into();
            };
            let r = match rng.below(4) {
                0 => data.link_child(fam, indi).map(|()| 0),
                1 => data.link_spouse(fam, indi).map(|()| 0),
                2 => data.unlink_child(fam, indi),
                _ => data.unlink_spouse(fam, indi),
            };
            format!("family link {r:?}")
        }
        6 | 7 => {
            let Some(h) = any_record(data, rng) else {
                return "nothing to remove".into();
            };
            let how = match rng.below(if dangling_allowed { 3 } else { 2 }) {
                0 => Removal::Refuse,
                1 => Removal::Detach,
                _ => Removal::LeaveDangling,
            };
            let r = match h.kind() {
                RecordKind::Individual => data
                    .remove(h.typed::<Individual>().unwrap(), how)
                    .map(|_| ()),
                RecordKind::Family => data.remove(h.typed::<Family>().unwrap(), how).map(|_| ()),
                RecordKind::Source => data.remove(h.typed::<Source>().unwrap(), how).map(|_| ()),
                RecordKind::Repository => data
                    .remove(h.typed::<Repository>().unwrap(), how)
                    .map(|_| ()),
                RecordKind::Multimedia => data
                    .remove(h.typed::<Multimedia>().unwrap(), how)
                    .map(|_| ()),
                RecordKind::Submitter => data
                    .remove(h.typed::<Submitter>().unwrap(), how)
                    .map(|_| ()),
                RecordKind::Note => data
                    .remove(h.typed::<SharedNote>().unwrap(), how)
                    .map(|_| ()),
                RecordKind::Other => data.remove(h.typed::<Node>().unwrap(), how).map(|_| ()),
                _ => Ok(()),
            };
            assert!(
                matches!(r, Ok(()) | Err(EditError::StillReferenced { .. })),
                "{r:?}"
            );
            format!("remove {h:?} {how:?} {r:?}")
        }
        8 => {
            let Some(id) = any_record(data, rng)
                .and_then(|h| data.get_any(h))
                .and_then(|r| r.xref())
            else {
                return "nothing to rename".into();
            };
            *fresh += 1;
            let new = match rng.below(4) {
                0 => {
                    // Another record's: refused.
                    let other = any_record(data, rng)
                        .and_then(|h| data.get_any(h))
                        .and_then(|r| r.xref())
                        .map(|x| data.store().xref(x).to_string());
                    other.unwrap_or_else(|| format!("@X{fresh}@"))
                }
                1 => format!("@x{fresh}@"), // invalid in 7.x: the writer repairs
                _ => format!("@X{fresh}@"),
            };
            let r = data.rename(id, &new);
            format!("rename to {new} {r:?}")
        }
        9 => {
            let (Some(a), Some(b)) = (
                xref_of::<Individual>(data, rng),
                xref_of::<Individual>(data, rng),
            ) else {
                return "nobody".into();
            };
            let r = data.redirect(a, b);
            format!("redirect {r:?}")
        }
        10 | 11 => {
            let Some(h) = any_of::<Individual>(data, rng) else {
                return "nobody to edit".into();
            };
            let source = xref_of::<Source>(data, rng);
            let note = xref_of::<SharedNote>(data, rng);
            let new_xref = (dangling_allowed && rng.chance(20))
                .then(|| xref_of::<Individual>(data, rng))
                .flatten();
            let choice = rng.below(5);
            let mut guard = data.get_mut(h).unwrap();
            match choice {
                0 => guard.citations.clear(),
                1 => {
                    if let Some(s) = source {
                        guard.citations.push(Citation {
                            source: CitationSource::Pointer(s),
                            ..Citation::default()
                        });
                    }
                }
                2 => {
                    if let Some(n) = note {
                        guard.notes.push(Note::shared(n));
                    }
                }
                3 => guard.extra.clear(),
                _ => {
                    if new_xref.is_some() {
                        guard.xref = new_xref;
                    }
                }
            }
            if rng.chance(10) {
                std::mem::forget(guard);
                // A query between: the record is still lent.
                let _ = data.references("@I1@");
                "edit, guard forgotten".into()
            } else {
                "edit".into()
            }
        }
        12 => {
            let Some(id) = any_record(data, rng)
                .and_then(|h| data.get_any(h))
                .and_then(|r| r.xref())
            else {
                return "nothing to detach".into();
            };
            let n = data.detach(id);
            format!("detach {n}")
        }
        13 => {
            let submitter = xref_of::<Submitter>(data, rng);
            if let Some(mut h) = data.header_mut() {
                h.submitter = submitter;
            }
            "header".into()
        }
        14 if dangling_allowed => {
            let id = data
                .store_mut()
                .intern_xref(&format!("@GONE{}@", rng.below(5)));
            let mut indi = Individual::default();
            if let Some(id) = id {
                indi.child_of.push(ChildLink::new(id));
            }
            data.push(Record::Individual(Box::new(indi)));
            "push unchecked".into()
        }
        _ => {
            let edits = data.individuals().len();
            data.edit_all::<Individual>(|i| {
                i.notes
                    .retain(|n| !matches!(n.content, oxidged::model::NoteContent::Text(_)));
            });
            format!("edit all {edits}")
        }
    }
}

fn run(seed: u64, steps: usize) {
    let mut rng = Rng(seed);
    let v7 = rng.chance(50);
    let dangling_allowed = rng.chance(30);
    let mut data = Dataset::parse(generated(&mut rng, v7));
    check(&data, false, "read");
    let mut fresh = 0;
    for step in 0..steps {
        let what = edit(&mut data, &mut rng, dangling_allowed, &mut fresh);
        let label = format!("seed {seed} step {step}: {what}");
        if std::env::var_os("EDIT_LOG").is_some() {
            eprintln!("{label}");
        }
        check(&data, dangling_allowed, &label);
        if step % 8 == 7 {
            check_writes(&data, &label);
        }
    }
    check_writes(&data, &format!("seed {seed} end"));
    // Every record's handle still finds it in a clone.
    let copy = data.clone();
    assert_eq!(copy, data);
    for (h, r) in data.records_with_handles() {
        assert_eq!(
            copy.get_any(h).map(oxidged::model::RecordRef::into_owned),
            Some(r.into_owned())
        );
    }
}

#[test]
fn random_edit_sequences_keep_the_dataset_whole() {
    for seed in 1..=120_u64 {
        run(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15), 48);
    }
}

/// Longer sequences, opt-in.
#[test]
#[ignore = "long: cargo test --test editing_properties -- --ignored"]
fn long_random_edit_sequences() {
    for seed in 1..=2_000_u64 {
        run(seed.wrapping_mul(0xD1B5_4A32_D192_ED03), 200);
    }
}
