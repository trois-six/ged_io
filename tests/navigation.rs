//! The dataset-level API: record lookups, family relationships, name
//! search, record counts, dangling references, the constant-time
//! [`IndexedDataset`] (first record wins on a duplicate identifier,
//! reindexing after edits), and how the model clones and compares. Every
//! name and value is fictitious.

use ged_io::model::{Dataset, Individual, IndividualRef, Name, RecordRef, Sex, XrefId};
use ged_io::IndexedDataset;

/// A family of three, their sources and the other record types.
const FAMILY: &str = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n2 FORM LINEAGE-LINKED\n\
    0 @I1@ INDI\n1 NAME John Robert /Example/ Jr.\n1 SEX M\n\
    1 BIRT\n2 DATE 1 JAN 1900\n2 PLAC Sampleton\n1 DEAT\n2 DATE 31 DEC 1980\n2 PLAC Exampleville\n\
    1 SOUR @S1@\n2 PAGE 42\n1 FAMS @F1@\n1 FAMS @F2@\n\
    0 @I2@ INDI\n1 NAME Jane /Example/\n1 SEX F\n1 FAMS @F1@\n\
    0 @I3@ INDI\n1 NAME Child /Example/\n1 FAMC @F1@\n1 FAMC @F3@\n\
    0 @I4@ INDI\n1 NAME Bob /Sample/\n1 FAMS @F2@\n\
    0 @F1@ FAM\n1 HUSB @I1@\n1 WIFE @I2@\n1 CHIL @I3@\n1 MARR\n2 DATE 15 JUN 1950\n\
    0 @F2@ FAM\n1 HUSB @I1@\n1 WIFE @I4@\n\
    0 @F3@ FAM\n1 WIFE @I9@\n1 CHIL @I3@\n\
    0 @S1@ SOUR\n1 TITL Census of Sampleton\n1 AUTH Sampleton Registry\n1 REPO @R1@\n\
    0 @R1@ REPO\n1 NAME Sampleton Archives\n\
    0 @M1@ OBJE\n1 FILE photo.jpg\n2 FORM jpg\n2 TITL Wedding photo\n\
    0 @U1@ SUBM\n1 NAME Ann Sample\n\
    0 @N1@ NOTE A shared note\n\
    0 @X1@ _LOC Sampleton\n\
    0 TRLR\n";

fn family() -> Dataset {
    Dataset::parse(FAMILY)
}

fn names<'a>(data: &Dataset, people: impl Iterator<Item = &'a Individual>) -> Vec<String> {
    people.filter_map(|i| i.full_name(data)).collect()
}

fn xrefs<'a, T: 'a>(
    data: &Dataset,
    records: impl Iterator<Item = &'a T>,
    xref: impl Fn(&T) -> Option<XrefId>,
) -> Vec<String> {
    records
        .filter_map(xref)
        .map(|id| data.store().xref(id).to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Lookups
// ---------------------------------------------------------------------------

#[test]
fn records_are_found_by_identifier() {
    let data = family();
    let john = data.find_individual("@I1@").unwrap();
    assert_eq!(
        john.full_name(&data).as_deref(),
        Some("John Robert Example Jr.")
    );
    assert_eq!(
        data.find_family("@F1@").unwrap().husband_id(),
        data.store().find_xref("@I1@")
    );
    let source = data.find_source("@S1@").unwrap();
    assert!(source
        .title
        .as_ref()
        .unwrap()
        .eq_str(&data, "Census of Sampleton"));
    let repo = data.find_repository("@R1@").unwrap();
    assert!(repo
        .name
        .as_ref()
        .unwrap()
        .eq_str(&data, "Sampleton Archives"));
    assert!(data.find_multimedia("@M1@").is_some());
    assert!(data.find_submitter("@U1@").is_some());
    assert!(data
        .find_note("@N1@")
        .unwrap()
        .text
        .eq_str(&data, "A shared note"));
    assert!(matches!(data.find("@X1@"), Some(RecordRef::Other(_))));
    assert!(matches!(data.find("@F2@"), Some(RecordRef::Family(_))));

    // Unknown identifiers, and identifiers of another record type.
    assert!(data.find_individual("@I999@").is_none());
    assert!(data.find_family("@F999@").is_none());
    assert!(data.find("@I9@").is_none()); // pointed to, but no record
    assert!(data.find_family("@I1@").is_none());
    assert!(data.find_individual("I1").is_none());
}

#[test]
fn every_kind_of_key_finds_the_same_record() {
    let data = family();
    let id = data.store().find_xref("@I2@").unwrap();
    let by_str = data.find_individual("@I2@").unwrap();
    assert_eq!(by_str.xref, Some(id));
    assert!(std::ptr::eq(data.find_individual(id).unwrap(), by_str));
    assert!(std::ptr::eq(
        data.find_individual(Some(id)).unwrap(),
        by_str
    ));
    assert!(std::ptr::eq(
        data.find_individual(String::from("@I2@")).unwrap(),
        by_str
    ));
    assert!(data.find_individual(None::<XrefId>).is_none());
}

#[test]
fn the_first_record_of_a_duplicate_identifier_wins() {
    let data = Dataset::parse(
        "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 NAME Ann /Example/\n\
         0 @I1@ INDI\n1 NAME Duplicate /Example/\n0 TRLR\n",
    );
    // Both are kept.
    assert_eq!(data.individuals.len(), 2);
    let found = data.find_individual("@I1@").unwrap();
    assert_eq!(found.full_name(&data).as_deref(), Some("Ann Example"));
    let indexed = IndexedDataset::new(data.clone());
    let found = indexed.find_individual("@I1@").unwrap();
    assert_eq!(found.full_name(&*indexed).as_deref(), Some("Ann Example"));
    assert_eq!(indexed.find("@I1@"), data.find("@I1@"));
}

#[test]
fn record_counts_include_the_header() {
    let data = family();
    // The header and 13 records; the trailer is not a record.
    assert_eq!(data.record_count(), 14);
    assert_eq!(data.records().count(), 14);
    assert!(matches!(data.records().next(), Some(RecordRef::Header(_))));
    assert_eq!(data.extra.len(), 1);

    let empty = Dataset::parse("0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 TRLR");
    assert_eq!(empty.record_count(), 1);
    assert_eq!(empty.declared_version(), Some("5.5.1"));
    assert_eq!(Dataset::default().record_count(), 0);
}

// ---------------------------------------------------------------------------
// Relationships
// ---------------------------------------------------------------------------

#[test]
fn families_of_an_individual() {
    let data = family();
    let f = |x| xrefs(&data, data.families_as_spouse(x), |f| f.xref);
    assert_eq!(f("@I1@"), ["@F1@", "@F2@"]);
    assert_eq!(f("@I4@"), ["@F2@"]);
    assert!(f("@I999@").is_empty());

    let c = |x| xrefs(&data, data.families_as_child(x), |f| f.xref);
    assert_eq!(c("@I3@"), ["@F1@", "@F3@"]);
    assert!(c("@I1@").is_empty());
}

#[test]
fn parents_children_and_spouses() {
    let data = family();
    let f1 = data.find_family("@F1@").unwrap();
    assert_eq!(
        names(&data, data.parents(f1)),
        ["John Robert Example Jr.", "Jane Example"]
    );
    assert_eq!(names(&data, data.children(f1)), ["Child Example"]);
    assert_eq!(
        data.spouse("@I1@", f1)
            .and_then(|s| s.full_name(&data))
            .as_deref(),
        Some("Jane Example")
    );
    assert_eq!(
        data.spouse("@I2@", f1)
            .and_then(|s| s.full_name(&data))
            .as_deref(),
        Some("John Robert Example Jr.")
    );
    assert!(data.spouse("@I3@", f1).is_none()); // a child, not a partner

    // Partners and children without a record are left out.
    let f3 = data.find_family("@F3@").unwrap();
    assert_eq!(data.parents(f3).count(), 0);
    assert_eq!(data.children(f3).count(), 1);
    assert!(data.spouse("@I9@", f3).is_none());
}

#[test]
fn names_are_searched_ignoring_case_and_slashes() {
    let data = family();
    let search = |q| names(&data, data.search_individuals(q));
    assert_eq!(search("example").len(), 3);
    assert_eq!(search("EXAMPLE").len(), 3);
    assert_eq!(search("jane"), ["Jane Example"]);
    assert_eq!(search("jane example"), ["Jane Example"]);
    assert_eq!(search("bob sample"), ["Bob Sample"]);
    assert!(search("/example/").is_empty());
    assert!(search("xyz").is_empty());
}

#[test]
fn individual_helpers() {
    let data = family();
    let john = data.find_individual("@I1@").unwrap();
    let jane = data.find_individual("@I2@").unwrap();
    assert_eq!(john.sex, Some(Sex::Male));
    assert_eq!(jane.sex, Some(Sex::Female));
    assert!(data.find_individual("@I3@").unwrap().sex.is_none());

    let name = john.name().unwrap();
    assert_eq!(name.value.to_str(&data), "John Robert /Example/ Jr.");
    assert_eq!(name.surname_in_value(&data).as_deref(), Some("Example"));

    let birth = john.birth().unwrap();
    assert_eq!(
        birth.date.as_ref().unwrap().value.to_str(&data),
        "1 JAN 1900"
    );
    assert_eq!(
        birth.place.as_ref().unwrap().name.to_str(&data),
        "Sampleton"
    );
    let death = john.death().unwrap();
    assert_eq!(
        death.date.as_ref().unwrap().value.to_str(&data),
        "31 DEC 1980"
    );
    assert_eq!(
        death.place.as_ref().unwrap().name.to_str(&data),
        "Exampleville"
    );

    assert_eq!(john.events.len(), 2);
    assert!(jane.events.is_empty());
    assert!(jane.birth().is_none());
    assert_eq!(john.citations.len(), 1);
    assert!(jane.citations.is_empty());
}

// ---------------------------------------------------------------------------
// Dangling references
// ---------------------------------------------------------------------------

#[test]
fn dangling_references_are_listed_in_record_order() {
    let data = family();
    let dangling = data.dangling_references();
    let found: Vec<_> = dangling
        .iter()
        .map(|d| {
            (
                d.record.map(|r| data.store().xref(r).to_string()),
                d.tag.as_str().to_string(),
                data.store().xref(d.pointer).to_string(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [(
            Some("@F3@".to_string()),
            "WIFE".to_string(),
            "@I9@".to_string()
        )]
    );
}

#[test]
fn dangling_references_of_every_kind_but_void() {
    // Within a record, in the order the model writes its structures.
    let data = Dataset::parse(
        "0 HEAD\n1 GEDC\n2 VERS 7.0\n1 SUBM @U9@\n\
         0 @I1@ INDI\n1 FAMC @F9@\n1 SOUR @S9@\n1 SNOTE @N9@\n1 OBJE @M9@\n1 FAMS @VOID@\n\
         0 @S1@ SOUR\n1 REPO @R9@\n0 TRLR\n",
    );
    let found: Vec<_> = data
        .dangling_references()
        .iter()
        .map(|d| format!("{} {}", d.tag.as_str(), data.store().xref(d.pointer)))
        .collect();
    assert_eq!(
        found,
        [
            "SUBM @U9@",
            "FAMC @F9@",
            "SNOTE @N9@",
            "SOUR @S9@",
            "OBJE @M9@",
            "REPO @R9@"
        ]
    );
    assert!(data.dangling_references()[0].record.is_none()); // the header
    assert!(Dataset::parse("0 HEAD\n1 GEDC\n2 VERS 7.0\n0 TRLR\n")
        .dangling_references()
        .is_empty());
}

// ---------------------------------------------------------------------------
// IndexedDataset
// ---------------------------------------------------------------------------

#[test]
fn the_index_agrees_with_the_linear_lookups() {
    let data = family();
    let indexed = IndexedDataset::from(data.clone());
    for xref in [
        "@I1@",
        "@I2@",
        "@I3@",
        "@I4@",
        "@F1@",
        "@F2@",
        "@F3@",
        "@S1@",
        "@R1@",
        "@M1@",
        "@U1@",
        "@N1@",
        "@X1@",
        "@I9@", // pointed to, but no record: its families are found alike
        "@nothing@",
    ] {
        assert_eq!(indexed.find(xref), data.find(xref), "{xref}");
        assert_eq!(
            indexed.find_individual(xref),
            data.find_individual(xref),
            "{xref}"
        );
        assert_eq!(indexed.find_family(xref), data.find_family(xref), "{xref}");
        assert_eq!(indexed.find_source(xref), data.find_source(xref), "{xref}");
        assert_eq!(
            indexed.find_repository(xref),
            data.find_repository(xref),
            "{xref}"
        );
        assert_eq!(
            indexed.find_multimedia(xref),
            data.find_multimedia(xref),
            "{xref}"
        );
        assert_eq!(
            indexed.find_submitter(xref),
            data.find_submitter(xref),
            "{xref}"
        );
        assert_eq!(indexed.find_note(xref), data.find_note(xref), "{xref}");
        let spouse: Vec<_> = indexed.families_as_spouse(xref).collect();
        assert_eq!(
            spouse,
            data.families_as_spouse(xref).collect::<Vec<_>>(),
            "{xref}"
        );
        let child: Vec<_> = indexed.families_as_child(xref).collect();
        assert_eq!(
            child,
            data.families_as_child(xref).collect::<Vec<_>>(),
            "{xref}"
        );
    }
    for family in &data.families {
        assert_eq!(
            indexed.parents(family).collect::<Vec<_>>(),
            data.parents(family).collect::<Vec<_>>()
        );
        assert_eq!(
            indexed.children(family).collect::<Vec<_>>(),
            data.children(family).collect::<Vec<_>>()
        );
        for xref in ["@I1@", "@I2@", "@I4@"] {
            assert_eq!(indexed.spouse(xref, family), data.spouse(xref, family));
        }
    }
    // A dataset still, through Deref and data().
    assert_eq!(indexed.individuals.len(), 4);
    assert_eq!(indexed.data(), &data);
    assert_eq!(indexed.into_inner(), data);
}

#[test]
fn an_individual_who_is_both_partners_has_the_family_once() {
    let data = Dataset::parse(
        "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n0 @F1@ FAM\n1 HUSB @I1@\n1 WIFE @I1@\n0 TRLR\n",
    );
    let indexed = IndexedDataset::new(data);
    assert_eq!(indexed.families_as_spouse("@I1@").count(), 1);
    assert_eq!(indexed.data().families_as_spouse("@I1@").count(), 1);
}

#[test]
fn edits_through_data_mut_reindex() {
    let mut indexed = IndexedDataset::new(family());
    {
        let mut data = indexed.data_mut();
        let id = data.store_mut().intern_xref("@I7@").unwrap();
        let mut eve = Individual {
            xref: Some(id),
            ..Individual::default()
        };
        eve.names.push(Name::new("Eve /Example/"));
        data.individuals.push(eve);
        // Eve replaces Bob in @F2@, and joins @F1@ as a child.
        let f2 = data.find_family("@F2@").unwrap().xref;
        let at = data.families.iter().position(|f| f.xref == f2).unwrap();
        data.families[at].wife = Some(IndividualRef::new(id));
        data.families[0].children.push(IndividualRef::new(id));
        // A removed record is no longer found.
        data.repositories.clear();
    }
    let eve = indexed.find_individual("@I7@").unwrap();
    assert_eq!(eve.full_name(&*indexed).as_deref(), Some("Eve Example"));
    assert_eq!(indexed.families_as_spouse("@I7@").count(), 1);
    assert_eq!(indexed.families_as_child("@I7@").count(), 1);
    assert_eq!(indexed.families_as_spouse("@I4@").count(), 0);
    let f1 = indexed.find_family("@F1@").unwrap();
    assert_eq!(indexed.children(f1).count(), 2);
    assert!(indexed.find_repository("@R1@").is_none());
    assert!(indexed.find("@R1@").is_none());
}

#[test]
fn an_unedited_guard_changes_nothing() {
    let mut indexed = IndexedDataset::new(family());
    drop(indexed.data_mut());
    assert!(indexed.find_individual("@I1@").is_some());
    assert_eq!(indexed.families_as_spouse("@I1@").count(), 2);
}

// ---------------------------------------------------------------------------
// Clone and PartialEq
// ---------------------------------------------------------------------------

#[test]
fn a_cloned_dataset_equals_its_original() {
    let data = family();
    let cloned = data.clone();
    assert_eq!(data, cloned);
    assert_eq!(cloned.individuals.len(), 4);
    // Its texts resolve against its own store.
    let john = cloned.find_individual("@I1@").unwrap();
    assert_eq!(
        john.full_name(&cloned).as_deref(),
        Some("John Robert Example Jr.")
    );
    assert_eq!(cloned.to_structures(), data.to_structures());
}

#[test]
fn cloned_records_equal_their_originals() {
    let data = family();
    assert_eq!(data.header.clone(), data.header);
    assert_eq!(data.individuals[0].clone(), data.individuals[0]);
    assert_eq!(data.families[0].clone(), data.families[0]);
    assert_eq!(data.sources[0].clone(), data.sources[0]);
    assert_eq!(data.repositories[0].clone(), data.repositories[0]);
    assert_eq!(data.multimedia[0].clone(), data.multimedia[0]);
    assert_eq!(data.submitters[0].clone(), data.submitters[0]);
    assert_eq!(data.notes[0].clone(), data.notes[0]);
    assert_eq!(data.extra[0].clone(), data.extra[0]);
    let individuals = data.individuals.to_vec();
    assert_eq!(individuals, data.individuals);
    assert!(data.individuals.contains(&individuals[2]));
}

#[test]
fn a_modified_clone_differs_and_leaves_the_original_alone() {
    let mut data = family();
    let original = data.individuals[0].clone();
    let mut modified = original.clone();
    let id = data.store_mut().intern_xref("@I8@").unwrap();
    modified.xref = Some(id);
    modified.sex = Some(Sex::Undetermined);
    assert_ne!(original, modified);
    assert_eq!(data.individuals[0], original);
    assert_eq!(data.store().xref(original.xref.unwrap()), "@I1@");

    let mut changed = data.clone();
    changed.individuals[0].names.clear();
    assert_ne!(changed, data);
}

#[test]
fn equal_input_reads_to_equal_datasets() {
    assert_eq!(family(), family());
    assert_eq!(family().individuals[1], family().individuals[1]);
    assert_eq!(family().families[0], family().families[0]);

    let other = Dataset::parse(FAMILY.replace("1 SEX F", "1 SEX M"));
    assert_ne!(other, family());
    assert_ne!(other.individuals[1], family().individuals[1]);
    let other = Dataset::parse(FAMILY.replace("1 WIFE @I4@", "1 WIFE @I3@"));
    assert_ne!(other.families[1], family().families[1]);
}

#[test]
fn texts_compare_as_held_so_records_of_other_datasets_compare_by_their_structures() {
    // The same individual at another place of another file holds other
    // spans: compare such records by their structures.
    let a =
        Dataset::parse("0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 NAME Ann /Example/\n0 TRLR\n");
    let b = Dataset::parse(
        "0 HEAD\n1 GEDC\n2 VERS 7.0\n1 LANG en\n0 @I1@ INDI\n1 NAME Ann /Example/\n0 TRLR\n",
    );
    let structure =
        |d: &Dataset| RecordRef::Individual(&d.individuals[0]).to_structure_in(d, d.version());
    assert_eq!(structure(&a), structure(&b));
    assert_eq!(
        a.individuals[0].full_name(&a),
        b.individuals[0].full_name(&b)
    );
}
