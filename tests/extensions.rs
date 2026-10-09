//! Extension (`_X`) and unknown tags, records and values: kept whole in the
//! `extra` of their real parent (or of the dataset), never mistaken for the
//! parent's own substructures, and written back with their continuations.
//! Every name and value is fictitious.

use std::io::BufReader;

use ged_io::model::{Dataset, Node, OrdinanceStatus, Text, Value};
use ged_io::tree::Structure;
use ged_io::{GedcomStreamParser, GedcomVersion, GedcomWriter};

// ----------------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------------

/// A node's tag and payload, as written.
fn describe(data: &Dataset, node: &Node) -> (String, Option<String>) {
    let payload = match &node.payload {
        Value::None => None,
        Value::Pointer(p) => Some(data.store().xref(*p).to_string()),
        Value::Text(t) => Some(t.to_str(data).into_owned()),
    };
    (data.store().tag(node.tag).to_string(), payload)
}

fn tag<'a>(data: &'a Dataset, node: &Node) -> &'a str {
    data.store().tag(node.tag)
}

fn payload(data: &Dataset, node: &Node) -> Option<String> {
    describe(data, node).1
}

fn write(data: &Dataset) -> String {
    GedcomWriter::new().write_to_string(data).unwrap()
}

fn structures(data: &Dataset, nodes: &[Node]) -> Vec<Structure> {
    nodes.iter().map(|n| n.to_structure(data)).collect()
}

/// A node with a text payload, its tag interned in `data`.
fn node(data: &mut Dataset, tag: &str, value: &str, children: Vec<Node>) -> Node {
    Node {
        tag: data.store_mut().intern_tag(tag),
        xref: None,
        payload: Value::Text(Text::new(value)),
        children,
    }
}

// ----------------------------------------------------------------------------
// Unknown tags keep their substructures
// ----------------------------------------------------------------------------

#[test]
fn unknown_record_tag_keeps_its_substructures() {
    let data = Dataset::parse(
        "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NAME Ann /Example/\n1 MILI Infantry\n\
         2 DATE 1 MAR 1915\n2 PLAC Sampletown\n2 NOTE Served two years\n2 SOUR @S1@\n\
         3 PAGE Register, p. 4\n2 OBJE @M1@\n0 @S1@ SOUR\n1 TITL Sample register\n\
         0 @M1@ OBJE\n1 FILE photo.jpg\n0 TRLR",
    );
    let person = &data.individuals[0];

    // The substructures are not taken for the person's own.
    assert!(person.notes.is_empty());
    assert!(person.citations.is_empty());
    assert!(person.detail().multimedia.is_empty());

    // The unknown structure is kept whole.
    assert_eq!(person.extra.len(), 1);
    let mili = &person.extra[0];
    assert_eq!(
        describe(&data, mili),
        ("MILI".into(), Some("Infantry".into()))
    );
    let children: Vec<_> = mili.children.iter().map(|c| describe(&data, c)).collect();
    let expected: Vec<(String, Option<String>)> = [
        ("DATE", "1 MAR 1915"),
        ("PLAC", "Sampletown"),
        ("NOTE", "Served two years"),
        ("SOUR", "@S1@"),
        ("OBJE", "@M1@"),
    ]
    .into_iter()
    .map(|(t, v)| (t.to_string(), Some(v.to_string())))
    .collect();
    assert_eq!(children, expected);
    assert_eq!(
        describe(&data, &mili.children[3].children[0]),
        ("PAGE".into(), Some("Register, p. 4".into()))
    );
}

#[test]
fn unknown_family_tag_keeps_its_substructures() {
    let data = Dataset::parse(
        "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @F1@ FAM\n1 HUSB @I1@\n1 XMAR Civil\n\
         2 NOTE Not the family's note\n2 SOUR @S1@\n0 TRLR",
    );
    let family = &data.families[0];
    assert!(family.notes.is_empty());
    assert!(family.citations.is_empty());
    assert_eq!(family.extra.len(), 1);
    assert_eq!(tag(&data, &family.extra[0]), "XMAR");
    assert_eq!(family.extra[0].children.len(), 2);
}

#[test]
fn unknown_event_substructure_does_not_overwrite_the_event() {
    let data = Dataset::parse(
        "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 BIRT\n2 DATE 1 JAN 1900\n\
         2 XYZ Other record\n3 DATE 2 FEB 1901\n3 PLAC Otherville\n3 TYPE Not the event's type\n\
         2 PLAC Sampletown\n1 OCCU Carpenter\n2 XYZ Other record\n3 DATE 3 MAR 1902\n\
         3 NOTE Not the attribute's note\n0 TRLR",
    );
    let person = &data.individuals[0];

    let birth = &person.events[0];
    assert_eq!(
        birth.date.as_ref().unwrap().value.to_str(&data),
        "1 JAN 1900"
    );
    assert_eq!(
        birth.place.as_ref().unwrap().name.to_str(&data),
        "Sampletown"
    );
    assert!(birth.detail().classification.is_none());
    // The unknown structure is kept whole in the event.
    assert_eq!(birth.extra.len(), 1);
    assert_eq!(tag(&data, &birth.extra[0]), "XYZ");
    assert_eq!(birth.extra[0].children.len(), 3);

    let occupation = &person.events[1];
    assert_eq!(occupation.value.to_str(&data), "Carpenter");
    assert!(occupation.date.is_none());
    assert!(occupation.detail().notes.is_empty());
    assert_eq!(occupation.extra.len(), 1);
    assert_eq!(occupation.extra[0].children.len(), 2);

    // Written back under its own structure, not merged into the event; an
    // unknown tag is written as an extension tag in 5.5.1.
    let written = write(&data);
    assert!(
        written.contains("2 _XYZ Other record\n3 DATE 2 FEB 1901\n"),
        "{written}"
    );
    let reread = Dataset::parse(written);
    let birth = &reread.individuals[0].events[0];
    assert_eq!(
        birth.date.as_ref().unwrap().value.to_str(&reread),
        "1 JAN 1900"
    );
    assert_eq!(birth.extra.len(), 1);
}

#[test]
fn unknown_substructure_of_a_value_structure_stays_with_it() {
    // RESN is a value; a substructure under it must not leak its own
    // substructures into the person.
    let data = Dataset::parse(
        "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 RESN locked\n2 XYZ Other record\n\
         3 NOTE Not the person's note\n1 NAME Ann /Example/\n0 TRLR",
    );
    let person = &data.individuals[0];
    assert!(person.notes.is_empty());
    assert_eq!(person.names.len(), 1);
    // A value structure with substructures it has no room for is kept
    // whole in `extra`.
    assert!(person.detail().restriction.is_none());
    assert_eq!(person.extra.len(), 1);
    let resn = &person.extra[0];
    assert_eq!(
        describe(&data, resn),
        ("RESN".into(), Some("locked".into()))
    );
    assert_eq!(tag(&data, &resn.children[0]), "XYZ");
    assert!(
        write(&data).contains("1 RESN locked\n2 _XYZ Other record\n3 NOTE Not the person's note\n")
    );
}

#[test]
fn unknown_tag_under_nested_structures_is_kept_there() {
    // REFN, STAT, DATA and CALN read their own substructures; an unknown tag
    // there must not hand its substructures to them.
    let data = Dataset::parse(
        "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NAME Ann /Example/\n1 REFN 42\n\
         2 XYZ Other record\n3 TYPE Not the reference's type\n1 BAPL\n2 STAT COMPLETED\n\
         3 XYZ Other record\n4 DATE 1 JAN 2000\n0 @S1@ SOUR\n1 DATA\n2 XYZ Other record\n\
         3 NOTE Not the data's note\n1 REPO @R1@\n2 CALN 123\n3 XYZ Other record\n\
         4 MEDI Not the call number's medium\n0 @R1@ REPO\n1 NAME Sample archive\n0 TRLR",
    );
    let person = &data.individuals[0];
    let refn = &person.detail().refns[0];
    assert_eq!(refn.value.to_str(&data), "42");
    assert!(refn.kind.is_none());
    assert_eq!(tag(&data, &refn.extra[0]), "XYZ");

    let status = person.detail().ordinances[0].status.as_ref().unwrap();
    assert_eq!(status.value, OrdinanceStatus::Completed);
    assert!(status.date.is_none());
    assert_eq!(tag(&data, &status.extra[0]), "XYZ");

    let source = &data.sources[0];
    let source_data = source.data.as_ref().unwrap();
    assert!(source_data.notes.is_empty());
    assert_eq!(tag(&data, &source_data.extra[0]), "XYZ");
    let call_number = &source.repositories[0].call_numbers[0];
    assert_eq!(call_number.value.to_str(&data), "123");
    assert!(call_number.medium.is_none());
    assert_eq!(tag(&data, &call_number.extra[0]), "XYZ");
}

#[test]
fn unknown_tag_under_crop_is_kept_there() {
    let data = Dataset::parse(
        "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 OBJE @M1@\n2 CROP\n3 TOP 10\n\
         3 XYZ Other record\n4 LEFT 99\n3 LEFT 5\n2 TITL Photo\n0 @M1@ OBJE\n\
         1 FILE photo.jpg\n2 FORM image/jpeg\n0 TRLR\n",
    );
    let link = &data.individuals[0].detail().multimedia[0];
    let crop = link.crop.as_ref().unwrap();
    assert_eq!(crop.top, Some(10));
    assert_eq!(crop.left, Some(5));
    assert_eq!(tag(&data, &crop.extra[0]), "XYZ");
    assert_eq!(link.title.as_ref().unwrap().to_str(&data), "Photo");
    let file = &data.multimedia[0].files[0];
    assert_eq!(
        file.form.as_ref().unwrap().format.to_str(&data),
        "image/jpeg"
    );
}

#[test]
fn unknown_substructure_of_age_is_kept_there() {
    let data = Dataset::parse(
        "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 DEAT\n2 AGE 30y\n3 XYZ Other record\n\
         2 PLAC Sampletown\n0 TRLR",
    );
    let death = &data.individuals[0].events[0];
    let age = death.detail().age.as_ref().unwrap();
    assert_eq!(age.value.to_str(&data), "30y");
    assert_eq!(tag(&data, &age.extra[0]), "XYZ");
    assert_eq!(
        death.place.as_ref().unwrap().name.to_str(&data),
        "Sampletown"
    );
}

// ----------------------------------------------------------------------------
// Extension tags and records survive a read and a write
// ----------------------------------------------------------------------------

#[test]
fn extension_tags_round_trip() {
    // User-defined tags survive a read and a write wherever they appear,
    // including top-level extension records that carry an identifier.
    let original = "0 HEAD
1 GEDC
2 VERS 5.5.1
1 _HME @I1@
0 @I1@ INDI
1 NAME Ann /Example/
2 _AKA Annie
1 BIRT
2 DATE 1 JAN 1900
2 PLAC Sampletown
3 _LOC @L1@
2 _PRIM Y
1 OCCU Weaver
2 _SALARY low
1 OBJE @M1@
2 _PRIM Y
1 SOUR @S1@
2 PAGE p. 1
2 DATA
3 TEXT Excerpt
3 _QUAL good
2 _APID 1,1::1
1 _UID 0D7A3C9E
1 _MILT
2 DATE 1918
2 _UNIT Sample regiment
1 _CUSTOM value
0 @F1@ FAM
1 _STAT married
0 @S1@ SOUR
1 TITL Sample register
1 _MEDI book
0 @M1@ OBJE
1 FILE photo.jpg
2 FORM jpg
1 _DATE 1950
0 @L1@ _LOC Sampletown
1 NAME Sampletown
2 DATE FROM 1900
0 _PUBLISH
1 _TREE Sample tree
0 TRLR";

    let data1 = Dataset::parse(original);
    assert_eq!(data1.extra.len(), 2);
    let loc = &data1.extra[0];
    assert_eq!(loc.xref.map(|x| data1.store().xref(x)), Some("@L1@"));
    assert_eq!(loc.children.len(), 1);
    assert_eq!(data1.extra[1].children.len(), 1);
    let person_extra: Vec<&str> = data1.individuals[0]
        .extra
        .iter()
        .map(|n| tag(&data1, n))
        .collect();
    assert_eq!(person_extra, ["_UID", "_MILT", "_CUSTOM"]);

    let written = write(&data1);
    for expected in [
        "1 _HME @I1@\n",
        "1 NAME Ann /Example/\n",
        "2 _AKA Annie\n",
        "3 _LOC @L1@\n",
        "2 _PRIM Y\n",
        "2 _SALARY low\n",
        "1 OBJE @M1@\n2 _PRIM Y\n",
        "3 _QUAL good\n",
        "2 _APID 1,1::1\n",
        "1 _UID 0D7A3C9E\n",
        "1 _MILT\n2 DATE 1918\n2 _UNIT Sample regiment\n",
        "1 _CUSTOM value\n",
        "1 _STAT married\n",
        "1 _MEDI book\n",
        "1 _DATE 1950\n",
        "0 @L1@ _LOC Sampletown\n1 NAME Sampletown\n2 DATE FROM 1900\n",
        "0 _PUBLISH\n1 _TREE Sample tree\n",
    ] {
        assert!(
            written.contains(expected),
            "missing {expected:?} in written output:\n{written}"
        );
    }

    let data2 = Dataset::parse(written);
    let records = |data: &Dataset| -> Vec<Structure> {
        data.to_structures()
            .into_iter()
            .filter(|s| {
                ["INDI", "FAM", "SOUR", "OBJE", "_LOC", "_PUBLISH"].contains(&s.tag.as_ref())
            })
            .collect()
    };
    assert_eq!(records(&data1), records(&data2));
    assert_eq!(
        structures(&data1, &data1.header.as_ref().unwrap().extra),
        structures(&data2, &data2.header.as_ref().unwrap().extra)
    );
}

#[test]
fn stream_parser_keeps_extension_records_with_identifiers() {
    let gedcom = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @L1@ _LOC Sampletown\n1 NAME Sampletown\n0 TRLR";
    let data: Dataset = GedcomStreamParser::new(BufReader::new(gedcom.as_bytes()))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let record = &data.extra[0];
    assert_eq!(record.xref.map(|x| data.store().xref(x)), Some("@L1@"));
    assert_eq!(tag(&data, record), "_LOC");
    assert_eq!(tag(&data, &record.children[0]), "NAME");
}

// ----------------------------------------------------------------------------
// Extension values are continued when written and read back whole
// ----------------------------------------------------------------------------

fn sample(version: &str) -> Dataset {
    Dataset::parse(format!(
        "0 HEAD\n1 GEDC\n2 VERS {version}\n0 @I1@ INDI\n1 NAME Ann /Example/\n0 TRLR"
    ))
}

/// The dataset written in `version`, and read back.
fn round_trip(data: &Dataset, version: GedcomVersion) -> (String, Dataset) {
    let written = GedcomWriter::new()
        .gedcom_version(version)
        .write_to_string(data)
        .unwrap();
    let reread = Dataset::parse(written.as_str());
    (written, reread)
}

/// Asserts that the extensions of the individual and the dataset read back
/// as they were set.
fn assert_same_extensions(data: &Dataset, reread: &Dataset, context: &str) {
    assert_eq!(
        structures(reread, &reread.individuals[0].extra),
        structures(data, &data.individuals[0].extra),
        "{context}"
    );
    assert_eq!(
        structures(reread, &reread.extra),
        structures(data, &data.extra),
        "{context}"
    );
}

#[test]
fn multi_line_extension_value_round_trip() {
    for (version, target) in [
        ("5.5.1", GedcomVersion::V5_5_1),
        ("7.0", GedcomVersion::V7_0),
    ] {
        let mut data = sample(version);
        let child = node(&mut data, "_Y", "Child line\nChild continued", vec![]);
        let x = node(
            &mut data,
            "_X",
            "First line\nSecond line\n\nFourth line",
            vec![child],
        );
        data.individuals[0].extra.push(x);
        let mut record = node(&mut data, "_LOC", "Sampletown\nNorth quarter", vec![]);
        record.xref = data.store_mut().intern_xref("@L1@");
        data.extra.push(record);

        let (written, reread) = round_trip(&data, target);
        for expected in [
            "1 _X First line\n2 CONT Second line\n2 CONT\n2 CONT Fourth line\n",
            "2 _Y Child line\n3 CONT Child continued\n",
            "0 @L1@ _LOC Sampletown\n1 CONT North quarter\n",
        ] {
            assert!(
                written.contains(expected),
                "{version}: missing {expected:?} in written output:\n{written}"
            );
        }
        assert_same_extensions(&data, &reread, version);
    }
}

#[test]
fn long_extension_value_round_trip() {
    // No space near the split points: CONC splits avoid them anyway.
    let long = format!("Long:{}", "abcdefghij".repeat(60));
    let mut data = sample("5.5.1");
    let child = node(&mut data, "_Y", &long, vec![]);
    let x = node(&mut data, "_X", &long, vec![child]);
    data.individuals[0].extra.push(x);
    let mut record = node(&mut data, "_LOC", &long, vec![]);
    record.xref = data.store_mut().intern_xref("@L1@");
    data.extra.push(record);

    let (written, reread) = round_trip(&data, GedcomVersion::V5_5_1);
    assert!(written.contains("\n2 CONC "), "{written}");
    assert!(written.contains("\n3 CONC "), "{written}");
    assert!(written.contains("\n1 CONC "), "{written}");
    for line in written.lines() {
        // Level, optional identifier, tag, then the value.
        let line_without_xref = line.replacen(" @L1@", "", 1);
        let value = line_without_xref.splitn(3, ' ').nth(2).unwrap_or("");
        assert!(value.len() <= 255, "line over the limit: {line}");
    }
    assert_same_extensions(&data, &reread, "5.5.1");
}

#[test]
fn extension_value_continuations_are_read_into_the_value() {
    // CONC/CONT lines continue the value of the extension tag they stand
    // under; any other substructure stays a child.
    let original = "0 HEAD
1 GEDC
2 VERS 5.5.1
0 @I1@ INDI
1 NAME Ann /Example/
1 _X First
2 CONC  part
2 CONT Second line
2 _Y Child
3 CONT Child continued
3 _Z Grandchild
2 DATE 1 JAN 1900
1 MILI Infantry
2 CONT Sample regiment
2 PLAC Sampletown
0 @L1@ _LOC Sampletown
1 CONT North quarter
1 NAME Sampletown
0 TRLR";

    let check = |data: &Dataset| {
        let extra = &data.individuals[0].extra;
        let x = &extra[0];
        assert_eq!(tag(data, x), "_X");
        assert_eq!(payload(data, x).as_deref(), Some("First part\nSecond line"));
        assert_eq!(x.children.len(), 2, "{x:?}");
        let y = &x.children[0];
        assert_eq!(tag(data, y), "_Y");
        assert_eq!(payload(data, y).as_deref(), Some("Child\nChild continued"));
        assert_eq!(y.children.len(), 1, "{y:?}");
        assert_eq!(tag(data, &y.children[0]), "_Z");
        assert_eq!(
            describe(data, &x.children[1]),
            ("DATE".into(), Some("1 JAN 1900".into()))
        );

        let mili = &extra[1];
        assert_eq!(tag(data, mili), "MILI");
        assert_eq!(
            payload(data, mili).as_deref(),
            Some("Infantry\nSample regiment")
        );
        assert_eq!(mili.children.len(), 1, "{mili:?}");
        assert_eq!(tag(data, &mili.children[0]), "PLAC");

        let loc = &data.extra[0];
        assert_eq!(loc.xref.map(|x| data.store().xref(x)), Some("@L1@"));
        assert_eq!(
            payload(data, loc).as_deref(),
            Some("Sampletown\nNorth quarter")
        );
        assert_eq!(loc.children.len(), 1, "{loc:?}");
        assert_eq!(tag(data, &loc.children[0]), "NAME");
    };

    check(&Dataset::parse(original));
    let streamed: Dataset = GedcomStreamParser::new(BufReader::new(original.as_bytes()))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    check(&streamed);
}

#[test]
fn valueless_extension_tag_at_end_of_file() {
    // A file may end on a valueless extension tag, without a trailer.
    for original in [
        "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 _X",
        "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 _X\n",
    ] {
        let data = Dataset::parse(original);
        let x = &data.individuals[0].extra[0];
        assert_eq!(tag(&data, x), "_X");
        assert_eq!(payload(&data, x), None);
    }
}
