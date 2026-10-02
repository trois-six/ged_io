//! Unknown (non-standard, non-underscore) tags keep their substructures.
//!
//! Before, the parser skipped only the line of a tag it did not recognise, and
//! the substructures of that tag were then read as if they belonged to the
//! enclosing structure.

use ged_io::GedcomBuilder;

#[test]
fn test_unknown_record_tag_keeps_its_substructures() {
    let source = "\
0 HEAD
1 GEDC
2 VERS 5.5.1
0 @I1@ INDI
1 NAME Ann /Example/
1 MILI Infantry
2 DATE 1 MAR 1915
2 PLAC Sampletown
2 NOTE Served two years
2 SOUR @S1@
3 PAGE Register, p. 4
2 OBJE @M1@
0 @S1@ SOUR
1 TITL Sample register
0 @M1@ OBJE
1 FILE photo.jpg
0 TRLR";
    let data = GedcomBuilder::new().build_from_str(source).unwrap();
    let person = &data.individuals[0];

    // The substructures are not taken for the person's own.
    assert!(person.note.is_none());
    assert!(person.source.is_empty());
    assert!(person.multimedia.is_empty());

    // The unknown structure is kept whole.
    assert_eq!(person.custom_data.len(), 1);
    let mili = &person.custom_data[0];
    assert_eq!(mili.tag, "MILI");
    assert_eq!(mili.value.as_deref(), Some("Infantry"));
    let children: Vec<(&str, Option<&str>)> = mili
        .children
        .iter()
        .map(|c| (c.tag.as_str(), c.value.as_deref()))
        .collect();
    assert_eq!(
        children,
        vec![
            ("DATE", Some("1 MAR 1915")),
            ("PLAC", Some("Sampletown")),
            ("NOTE", Some("Served two years")),
            ("SOUR", Some("@S1@")),
            ("OBJE", Some("@M1@")),
        ]
    );
    let page = &mili.children[3].children[0];
    assert_eq!(page.tag, "PAGE");
    assert_eq!(page.value.as_deref(), Some("Register, p. 4"));
}

#[test]
fn test_unknown_family_tag_keeps_its_substructures() {
    let source = "\
0 HEAD
1 GEDC
2 VERS 5.5.1
0 @F1@ FAM
1 HUSB @I1@
1 XMAR Civil
2 NOTE Not the family's note
2 SOUR @S1@
0 TRLR";
    let data = GedcomBuilder::new().build_from_str(source).unwrap();
    let family = &data.families[0];

    assert!(family.notes.is_empty());
    assert!(family.sources.is_empty());
    assert_eq!(family.custom_data.len(), 1);
    assert_eq!(family.custom_data[0].tag, "XMAR");
    assert_eq!(family.custom_data[0].children.len(), 2);
}

#[test]
fn test_unknown_event_substructure_does_not_overwrite_the_event() {
    let source = "\
0 HEAD
1 GEDC
2 VERS 5.5.1
0 @I1@ INDI
1 BIRT
2 DATE 1 JAN 1900
2 XYZ Other record
3 DATE 2 FEB 1901
3 PLAC Otherville
3 TYPE Not the event's type
2 PLAC Sampletown
1 OCCU Carpenter
2 XYZ Other record
3 DATE 3 MAR 1902
3 NOTE Not the attribute's note
0 TRLR";
    let data = GedcomBuilder::new().build_from_str(source).unwrap();
    let person = &data.individuals[0];

    let birth = &person.events[0];
    assert_eq!(
        birth.date.as_ref().unwrap().value.as_deref(),
        Some("1 JAN 1900")
    );
    assert_eq!(
        birth.place.as_ref().unwrap().value.as_deref(),
        Some("Sampletown")
    );
    assert!(birth.event_type.is_none());

    let occupation = &person.attributes[0];
    assert_eq!(occupation.value.as_deref(), Some("Carpenter"));
    assert!(occupation.date.is_none());
    assert!(occupation.note.is_none());
}

#[test]
fn test_unread_substructure_of_known_tag_is_skipped_whole() {
    // RESN is read as a line value only; a substructure under it must not
    // leak its own substructures into the person.
    let source = "\
0 HEAD
1 GEDC
2 VERS 5.5.1
0 @I1@ INDI
1 RESN locked
2 XYZ Other record
3 NOTE Not the person's note
1 NAME Ann /Example/
0 TRLR";
    let data = GedcomBuilder::new().build_from_str(source).unwrap();
    let person = &data.individuals[0];

    assert_eq!(person.restriction.as_deref(), Some("locked"));
    assert!(person.note.is_none());
    assert!(person.custom_data.is_empty());
    assert_eq!(person.names.len(), 1);
}

#[test]
fn test_unknown_substructure_of_age_does_not_hang() {
    let source = "\
0 HEAD
1 GEDC
2 VERS 5.5.1
0 @I1@ INDI
1 DEAT
2 AGE 30y
3 XYZ Other record
2 PLAC Sampletown
0 TRLR";
    let data = GedcomBuilder::new().build_from_str(source).unwrap();
    let death = &data.individuals[0].events[0];

    assert!(death.age.is_some());
    assert_eq!(
        death.place.as_ref().unwrap().value.as_deref(),
        Some("Sampletown")
    );
}
