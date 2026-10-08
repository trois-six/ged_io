//! The typed model under construction (`ged_io::next`): shared
//! substructures and enumerations read typed and lossless, written
//! conformant. Every name and value is fictitious.

use ged_io::next::{
    read_str, write_string, Adoption, Certainty, CitationSource, Dataset, Event, EventKind,
    FileForm, Individual, Medium, NameType, Note, NoteContent, OrdinanceStatus, Pedigree, Role,
    Sex, Source, Text,
};
use ged_io::{GedcomVersion, GedcomWriter};

fn indi<'a>(data: &'a Dataset, xref: &str) -> &'a Individual {
    data.individual(xref).expect("individual")
}

fn source<'a>(data: &'a Dataset, xref: &str) -> &'a Source {
    let id = data.store.find_xref(xref).expect("xref");
    data.sources
        .iter()
        .find(|s| s.xref == Some(id))
        .expect("source")
}

/// The first event of a kind.
fn event(indi: &Individual, kind: EventKind) -> &Event {
    indi.events.iter().find(|e| e.kind == kind).expect("event")
}

fn text(data: &Dataset, t: &Text) -> String {
    t.to_str(data).into_owned()
}

fn write(data: &Dataset) -> String {
    write_string(data, &GedcomWriter::new()).expect("write")
}

fn write_as(data: &Dataset, version: GedcomVersion) -> String {
    write_string(data, &GedcomWriter::new().gedcom_version(version)).expect("write")
}

fn v7(body: &str) -> String {
    format!("0 HEAD\n1 GEDC\n2 VERS 7.0\n{body}0 TRLR\n")
}

fn v551(body: &str) -> String {
    format!(
        "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n2 FORM LINEAGE-LINKED\n1 CHAR UTF-8\n1 SOUR EXAMPLE\n1 SUBM @U1@\n{body}0 @U1@ SUBM\n1 NAME Example Submitter\n0 TRLR\n"
    )
}

/// Extensions and unknown tags stay under their real parent, in place,
/// through a read and a write (F1, R2, D21).
#[test]
fn extensions_are_kept_where_they_are() {
    let data = read_str(&v551(
        "0 @I1@ INDI\n1 BIRT\n2 DATE 1 JAN 1900\n3 _DX date extension\n1 NOTE A note\n2 _NX under a note\n2 FOO unknown standard-looking tag\n1 CHAN\n2 DATE 1 JAN 2000\n3 _CX change extension\n",
    ));
    let note = &indi(&data, "@I1@").notes[0];
    assert_eq!(note.extra.len(), 2);
    assert_eq!(data.store.tag(note.extra[0].tag), "_NX");
    let out = write(&data);
    assert!(
        out.contains("2 DATE 1 JAN 1900\n3 _DX date extension\n"),
        "{out}"
    );
    assert!(
        out.contains("1 NOTE A note\n2 _NX under a note\n2 _FOO unknown standard-looking tag\n"),
        "{out}"
    );
    assert!(
        out.contains("2 DATE 1 JAN 2000\n3 _CX change extension\n"),
        "{out}"
    );
}

/// Repeatable identifiers are all kept, with their types (F2, R5,
/// G7-UID-MULTI, G7-EXID-TYPE).
#[test]
fn identifiers_are_repeatable_and_keep_their_type() {
    let data = read_str(&v7(
        "0 @I1@ INDI\n1 REFN a\n2 TYPE user\n1 REFN b\n1 UID 0b0f2b0a-0000-4000-8000-000000000001\n1 UID 0b0f2b0a-0000-4000-8000-000000000002\n1 EXID 123\n2 TYPE http://example.com\n",
    ));
    let detail = indi(&data, "@I1@").detail();
    let refns = &detail.refns;
    assert_eq!(refns.len(), 2);
    assert_eq!(text(&data, &refns[0].value), "a");
    assert_eq!(text(&data, refns[0].kind.as_ref().unwrap()), "user");
    assert_eq!(detail.uids.len(), 2);
    let exid = &detail.exids[0];
    assert_eq!(
        text(&data, exid.kind.as_ref().unwrap()),
        "http://example.com"
    );
    let out = write(&data);
    for line in [
        "1 REFN a\n2 TYPE user\n1 REFN b\n",
        "1 UID 0b0f2b0a-0000-4000-8000-000000000001\n1 UID 0b0f2b0a-0000-4000-8000-000000000002\n",
        "1 EXID 123\n2 TYPE http://example.com\n",
    ] {
        assert!(out.contains(line), "{out}");
    }
}

/// A note keeps its media type, language, translations and sources
/// (G7-NOTE-TRAN); a shared note is a pointer, `@@` text is not (H12).
#[test]
fn notes() {
    let data = read_str(&v7(
        "0 @I1@ INDI\n1 NOTE <p>Bonjour</p>\n2 MIME text/html\n2 LANG fr\n2 TRAN Hello\n3 LANG en\n2 SOUR @S1@\n3 PAGE 4\n1 SNOTE @N1@\n1 NOTE @@N1@ is an address\n0 @N1@ SNOTE Shared\n0 @S1@ SOUR\n1 TITL Register\n",
    ));
    let notes = &indi(&data, "@I1@").notes;
    assert_eq!(notes.len(), 3);
    let detail = notes[0].detail();
    assert_eq!(text(&data, detail.mime.as_ref().unwrap()), "text/html");
    assert_eq!(text(&data, &detail.translations[0].text), "Hello");
    assert_eq!(detail.citations.len(), 1);
    assert!(matches!(notes[1].content, NoteContent::Shared(id) if data.store.xref(id) == "@N1@"));
    assert!(
        matches!(&notes[2].content, NoteContent::Text(t) if t.to_str(&data) == "@N1@ is an address")
    );
    let out = write(&data);
    assert!(out.contains("1 NOTE <p>Bonjour</p>\n2 MIME text/html\n2 LANG fr\n2 TRAN Hello\n3 LANG en\n2 SOUR @S1@\n3 PAGE 4\n"), "{out}");
    assert!(
        out.contains("1 SNOTE @N1@\n1 NOTE @@N1@ is an address\n"),
        "{out}"
    );
    // 5.5.1 points to a note record with NOTE.
    let data = read_str(&v551("0 @I1@ INDI\n1 NOTE @N1@\n0 @N1@ NOTE Shared\n"));
    let note = &indi(&data, "@I1@").notes[0];
    assert!(matches!(note.content, NoteContent::Shared(_)));
    assert!(write(&data).contains("1 NOTE @N1@\n"));
    // 7.x points to a shared note with SNOTE (the record itself becomes a
    // shared note record with the 5.5.1 to 7.x conversion).
    let records = data.to_structures_for(GedcomVersion::V7_0);
    let record = records.iter().find(|r| r.tag == "INDI").unwrap();
    assert_eq!(record.substructures[0].tag, "SNOTE");
    assert!(records.iter().any(|r| r.tag == "SNOTE"));
}

/// A citation: pointer or description, page, data, event and role with
/// phrases, quality; a repeated singleton goes to `extra`, in order.
#[test]
fn citations() {
    let data = read_str(&v7(
        "0 @I1@ INDI\n1 BIRT\n2 SOUR @S1@\n3 PAGE 12\n3 PAGE 13\n3 DATA\n4 DATE 1 JAN 1900\n4 TEXT Born at home\n5 LANG en\n3 EVEN BIRT\n4 PHRASE Birth\n4 ROLE OTHER\n5 PHRASE Midwife\n3 QUAY 3\n0 @S1@ SOUR\n1 TITL Register\n",
    ));
    let citation = &event(indi(&data, "@I1@"), EventKind::Birth).citations[0];
    assert!(matches!(citation.source, CitationSource::Pointer(_)));
    assert_eq!(text(&data, citation.page.as_ref().unwrap()), "12");
    assert_eq!(citation.extra.len(), 1, "the second PAGE");
    assert_eq!(citation.detail().quality, Some(Certainty::Primary));
    let event = citation.detail().event.as_ref().unwrap();
    let role = event.role.as_ref().unwrap();
    assert_eq!(role.value, Role::Other);
    assert_eq!(text(&data, role.phrase.as_ref().unwrap()), "Midwife");
    let out = write(&data);
    assert!(out.contains("2 SOUR @S1@\n3 PAGE 12\n3 DATA\n4 DATE 1 JAN 1900\n4 TEXT Born at home\n5 LANG en\n3 EVEN BIRT\n4 PHRASE Birth\n4 ROLE OTHER\n5 PHRASE Midwife\n3 QUAY 3\n"), "{out}");
    // The second PAGE is kept, as an extension the version permits.
    assert!(out.contains("3 _PAGE 13\n"), "{out}");

    // 5.5.1: a source described in the citation, with its text.
    let data = read_str(&v551(
        "0 @I1@ INDI\n1 SOUR Parish register\n2 TEXT Born on a Sunday\n2 QUAY 2\n",
    ));
    let citation = &indi(&data, "@I1@").citations[0];
    assert!(
        matches!(&citation.source, CitationSource::Description(t) if t.to_str(&data) == "Parish register")
    );
    assert_eq!(
        text(&data, &citation.detail().texts[0].text),
        "Born on a Sunday"
    );
    let out = write(&data);
    assert!(
        out.contains("1 SOUR Parish register\n2 QUAY 2\n2 TEXT Born on a Sunday\n"),
        "{out}"
    );
}

/// Repository citations: call numbers and their medium, an extension
/// medium kept as written (H14).
#[test]
fn repository_citations() {
    let data = read_str(&v7(
        "0 @S1@ SOUR\n1 REPO @R1@\n2 CALN 123\n3 MEDI _MYMEDIUM\n2 CALN 456\n3 MEDI OTHER\n4 PHRASE Glass plate\n0 @R1@ REPO\n1 NAME Archive\n",
    ));
    let repo = &source(&data, "@S1@").repositories[0];
    assert_eq!(repo.call_numbers.len(), 2);
    let medium = repo.call_numbers[0].medium.as_ref().unwrap();
    assert!(matches!(&medium.value, Medium::Unknown(t) if t.to_str(&data) == "_MYMEDIUM"));
    assert_eq!(
        repo.call_numbers[1].medium.as_ref().unwrap().value,
        Medium::Other
    );
    let out = write(&data);
    assert!(
        out.contains(
            "2 CALN 123\n3 MEDI _MYMEDIUM\n2 CALN 456\n3 MEDI OTHER\n4 PHRASE Glass plate\n"
        ),
        "{out}"
    );
}

/// Multimedia links: a 7.x link's title and crop in pixels
/// (G7-OBJE-LINK-TITL-CROP, M1), a 5.5.1 link's files with `FORM.MEDI`
/// read and written as `MEDI` (D12).
#[test]
fn multimedia_links() {
    let data = read_str(&v7(
        "0 @I1@ INDI\n1 OBJE @O1@\n2 CROP\n3 TOP 10\n3 LEFT 20\n3 HEIGHT 300\n3 WIDTH 50%\n2 TITL party\n0 @O1@ OBJE\n1 FILE media/a.jpg\n2 FORM image/jpeg\n",
    ));
    let link = &indi(&data, "@I1@").detail().multimedia[0];
    let crop = link.crop.as_ref().unwrap();
    assert_eq!(
        (crop.top, crop.left, crop.height, crop.width),
        (Some(10), Some(20), Some(300), None)
    );
    assert_eq!(
        crop.extra.len(),
        1,
        "a width that is not in pixels is kept as written"
    );
    assert_eq!(text(&data, link.title.as_ref().unwrap()), "party");
    let out = write(&data);
    assert!(
        out.contains("1 OBJE @O1@\n2 CROP\n3 TOP 10\n3 LEFT 20\n3 HEIGHT 300\n"),
        "{out}"
    );
    assert!(out.contains("2 TITL party\n"), "{out}");
    assert!(out.contains("50%"), "{out}");

    let data = read_str(&v551(
        "0 @I1@ INDI\n1 OBJE\n2 FILE photo.jpg\n3 FORM jpg\n4 MEDI photo\n2 TITL Portrait\n",
    ));
    let link = &indi(&data, "@I1@").detail().multimedia[0];
    let form: &FileForm = link.files[0].form.as_ref().unwrap();
    assert_eq!(form.medium.as_ref().unwrap().value, Medium::Photo);
    let out = write(&data);
    assert!(
        out.contains("1 OBJE\n2 FILE photo.jpg\n3 FORM jpg\n4 MEDI photo\n2 TITL Portrait\n"),
        "{out}"
    );
}

/// Places: form, language, translations, coordinates, identifiers, notes,
/// and 5.5.1 phonetic and romanized variations (G5-MAP).
#[test]
fn places() {
    let data = read_str(&v7(
        "0 @I1@ INDI\n1 BIRT\n2 PLAC Sampleton, Example County\n3 FORM City, County\n3 LANG en\n3 TRAN Sampleville, Exemple\n4 LANG fr\n3 MAP\n4 LATI N18.150944\n4 LONG E168.150944\n3 EXID 77\n4 TYPE http://example.com/places\n3 NOTE Near the river\n",
    ));
    let place = event(indi(&data, "@I1@"), EventKind::Birth)
        .place
        .as_ref()
        .unwrap()
        .detail();
    assert_eq!(text(&data, place.form.as_ref().unwrap()), "City, County");
    assert_eq!(
        text(&data, &place.translations[0].name),
        "Sampleville, Exemple"
    );
    let map = place.map.as_ref().unwrap();
    assert_eq!(text(&data, map.latitude.as_ref().unwrap()), "N18.150944");
    assert_eq!(place.exids.len(), 1);
    assert_eq!(place.notes.len(), 1);

    let data = read_str(&v551(
        "0 @I1@ INDI\n1 BIRT\n2 PLAC Sample City\n3 FONE Sanpuru\n4 TYPE kana\n3 ROMN Sanpuru Shi\n4 TYPE romaji\n3 MAP\n4 LATI N35.0\n4 LONG E135.0\n",
    ));
    let place = event(indi(&data, "@I1@"), EventKind::Birth)
        .place
        .as_ref()
        .unwrap()
        .detail();
    assert_eq!(place.phonetic.len(), 1);
    assert_eq!(place.romanized.len(), 1);
    let out = write(&data);
    assert!(out.contains("2 PLAC Sample City\n3 MAP\n4 LATI N35.0\n4 LONG E135.0\n3 FONE Sanpuru\n4 TYPE kana\n3 ROMN Sanpuru Shi\n4 TYPE romaji\n"), "{out}");
}

/// Associations: 7.x role and phrase, 5.5.1 relation.
#[test]
fn associations() {
    let data = read_str(&v7(
        "0 @I1@ INDI\n1 ASSO @I2@\n2 PHRASE The neighbour\n2 ROLE NGHBR\n0 @I2@ INDI\n",
    ));
    let asso = &indi(&data, "@I1@").detail().associations[0];
    assert_eq!(asso.role.as_ref().unwrap().value, Role::Neighbor);
    let data = read_str(&v551(
        "0 @I1@ INDI\n1 ASSO @I2@\n2 RELA Godfather\n0 @I2@ INDI\n",
    ));
    let asso = &indi(&data, "@I1@").detail().associations[0];
    assert_eq!(text(&data, asso.relation.as_ref().unwrap()), "Godfather");
    assert!(write(&data).contains("1 ASSO @I2@\n2 RELA Godfather\n"));
}

/// Enumerations never fail: `OTHER` with its phrase, extension and unknown
/// values kept as written (C3, D15a-c, M3, G7-ENUM-*, G5-PEDI-NONSTD).
#[test]
fn enumeration_values_are_never_fatal() {
    let data = read_str(&v7(
        "0 @I1@ INDI\n1 SEX _I\n1 FAMC @F1@\n2 PEDI OTHER\n3 PHRASE Guardianship\n2 STAT _DOUBTED\n1 FAMC @F2@\n2 PEDI _ENUMVAL\n0 @F1@ FAM\n0 @F2@ FAM\n",
    ));
    let person = indi(&data, "@I1@");
    assert!(matches!(&person.sex, Some(Sex::Unknown(t)) if t.to_str(&data) == "_I"));
    let famc = person.child_of[0].detail();
    let pedi = famc.pedigree.as_ref().unwrap();
    assert_eq!(pedi.value, Pedigree::Other);
    assert_eq!(text(&data, pedi.phrase.as_ref().unwrap()), "Guardianship");
    let stat = famc.status.as_ref().unwrap();
    assert!(!stat.value.is_known());
    let out = write(&data);
    for line in [
        "1 SEX _I\n",
        "2 PEDI OTHER\n3 PHRASE Guardianship\n",
        "2 STAT _DOUBTED\n",
        "2 PEDI _ENUMVAL\n",
    ] {
        assert!(out.contains(line), "{out}");
    }

    // 5.5.1: controlled values in any case; values outside a closed set kept.
    let data = read_str(&v551(
        "0 @I1@ INDI\n1 SEX m\n1 FAMC @F1@\n2 PEDI stepchild\n1 SOUR @S1@\n2 QUAY 5\n0 @F1@ FAM\n0 @S1@ SOUR\n",
    ));
    let person = indi(&data, "@I1@");
    assert_eq!(person.sex, Some(Sex::Male));
    let pedi = &person.child_of[0].detail().pedigree.as_ref().unwrap().value;
    assert!(matches!(pedi, Pedigree::Unknown(t) if t.to_str(&data) == "stepchild"));
    let citation = &person.citations[0];
    assert!(
        matches!(&citation.detail().quality, Some(Certainty::Unknown(t)) if t.to_str(&data) == "5")
    );
    let out = write(&data);
    assert!(out.contains("1 SEX M\n"), "{out}");
    // Written conformant, with nothing lost: as extensions.
    assert!(out.contains("2 _PEDI stepchild\n"), "{out}");
    assert!(out.contains("2 _QUAY 5\n"), "{out}");
}

/// Each version's spelling: 7.x upper case (H3, TS19), `BIRTH` is not
/// `AKA` (H4, D10), 5.5.1 lower case.
#[test]
fn enumeration_spellings_per_version() {
    let data = read_str(&v551(
        "0 @I1@ INDI\n1 NAME Ann /Sample/\n2 TYPE birth\n1 FAMC @F1@\n2 PEDI adopted\n2 STAT challenged\n0 @F1@ FAM\n1 CHIL @I1@\n2 ADOP BOTH\n",
    ));
    let name = &indi(&data, "@I1@").names[0];
    assert_eq!(
        name.detail().kind.as_ref().map(|k| &k.value),
        Some(&NameType::Birth)
    );
    let out = write(&data);
    assert!(out.contains("2 TYPE birth\n"), "{out}");
    assert!(out.contains("2 PEDI adopted\n2 STAT challenged\n"), "{out}");
    let out = write_as(&data, GedcomVersion::V7_0);
    assert!(out.contains("2 TYPE BIRTH\n"), "{out}");
    assert!(out.contains("2 PEDI ADOPTED\n2 STAT CHALLENGED\n"), "{out}");
    assert_eq!(Adoption::parse("both"), Adoption::Both);
    assert_eq!(Adoption::Both.as_str(GedcomVersion::V7_0), Some("BOTH"));
    assert_eq!(Pedigree::Other.as_str(GedcomVersion::V5_5_1), Some("OTHER"));
}

/// Latter-day Saint ordinance statuses of both versions, with their dates
/// (D10, M2, #107).
#[test]
fn ordinance_statuses() {
    let data = read_str(&v7(
        "0 @I1@ INDI\n1 BAPL\n2 STAT PRE_1970\n3 DATE 27 MAR 2022\n1 INIL\n2 STAT EXCLUDED\n3 DATE 27 MAR 2022\n0 @F1@ FAM\n1 SLGS\n2 STAT DNS_CAN\n3 DATE 1 JAN 2000\n",
    ));
    let stat = indi(&data, "@I1@").detail().ordinances[0]
        .status
        .as_ref()
        .unwrap();
    assert_eq!(stat.value, OrdinanceStatus::Pre1970);
    assert_eq!(
        text(&data, &stat.date.as_ref().unwrap().value),
        "27 MAR 2022"
    );
    let out = write(&data);
    assert!(
        out.contains("2 STAT PRE_1970\n3 DATE 27 MAR 2022\n"),
        "{out}"
    );
    assert!(
        out.contains("2 STAT EXCLUDED\n3 DATE 27 MAR 2022\n"),
        "{out}"
    );
    assert!(out.contains("2 STAT DNS_CAN\n3 DATE 1 JAN 2000\n"), "{out}");
    let out = write_as(&data, GedcomVersion::V5_5_1);
    assert!(out.contains("2 STAT PRE-1970\n"), "{out}");
    assert!(out.contains("2 STAT DNS/CAN\n"), "{out}");
    assert_eq!(
        OrdinanceStatus::parse("dns/can"),
        OrdinanceStatus::DoNotSealCanceled
    );
}

/// Dates, ages and times are kept as written, read by the grammars on
/// demand, and written in the target version's grammar.
#[test]
fn dates_and_ages() {
    let data = read_str(&v551(
        "0 @I1@ INDI\n1 BIRT\n2 DATE @#DJULIAN@ 1 JAN 1700\n1 DEAT\n2 DATE 2 FEB 1750\n2 AGE 50y\n1 CHAN\n2 DATE 1 JAN 2000\n3 TIME 12:00:00\n",
    ));
    let person = indi(&data, "@I1@");
    let date = event(person, EventKind::Birth).date.as_ref().unwrap();
    assert_eq!(text(&data, &date.value), "@#DJULIAN@ 1 JAN 1700");
    assert!(matches!(
        date.parse(&data),
        ged_io::types::date::value::DateValue::Date(_)
    ));
    let age = event(person, EventKind::Death)
        .detail()
        .age
        .as_ref()
        .unwrap();
    assert_eq!(text(&data, &age.value), "50y");
    let out = write(&data);
    assert!(out.contains("2 DATE @#DJULIAN@ 1 JAN 1700\n"), "{out}");
    let out = write_as(&data, GedcomVersion::V7_0);
    assert!(out.contains("2 DATE JULIAN 1 JAN 1700\n"), "{out}");
    assert!(
        out.contains("2 DATE 1 JAN 2000\n3 TIME 12:00:00\n"),
        "{out}"
    );
}

/// A payload or an identifier a structure's type has no place for (text
/// where a pointer belongs, an identifier on a substructure) is kept aside
/// and written back: the structure and its substructures stay typed.
#[test]
fn payloads_that_do_not_fit_are_kept_aside() {
    let data = read_str(&v7(
        "0 @I1@ INDI\n1 ASSO a neighbour\n2 ROLE NGHBR\n1 @X1@ NOTE A note with an identifier\n0 @F1@ FAM stray text\n1 MARR\n2 DATE 1 JAN 1900\n",
    ));
    let person = indi(&data, "@I1@");
    let asso = &person.detail().associations[0];
    assert_eq!(asso.individual, None);
    assert_eq!(asso.role.as_ref().unwrap().value, Role::Neighbor);
    assert_eq!(asso.extra[0].tag, ged_io::next::TagId::ASIDE);
    assert!(person.notes[0].extra[0].xref.is_some());
    assert_eq!(
        data.families[0].events.len(),
        1,
        "a record with a payload is typed"
    );
    let untyped = ged_io::next::ledger::untyped(&data);
    assert_eq!(untyped, ["INDI/NOTE", "INDI/ASSO", "FAM"]);
    let structures = data.to_structures();
    let indi = structures.iter().find(|r| r.tag == "INDI").unwrap();
    assert_eq!(indi.substructures[1].payload.as_str(), Some("a neighbour"));
    let out = write(&data);
    assert!(out.contains("a neighbour"), "{out}");
    assert!(out.contains("A note with an identifier"), "{out}");
    assert!(out.contains("stray text"), "{out}");
}

/// Reading never fails, whatever the input.
#[test]
fn reading_is_total() {
    let inputs = [
        "",
        "\n\n",
        "garbage without levels",
        "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n9 NOTE deep\n1 NOTE\n2 TRAN\n3 TRAN\n1 OBJE\n2 CROP\n3 TOP -1\n3 TOP 99999999999\n",
        "0 @I1@ INDI\n1 SOUR @S1@ trailing\n1 SOUR\n2 EVEN\n3 ROLE\n1 PLAC\n2 MAP\n3 LATI\n1 SEX\n1 RESN ,,CONFIDENTIAL,\n",
        "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @N1@ NOTE @@\n1 CONT @\n0 @@ INDI\n1 NOTE @\n",
    ];
    for input in inputs {
        let data = read_str(input);
        let _ = write(&data);
        let _ = write_as(&data, GedcomVersion::V5_5_1);
    }
}

/// Text a program sets owns its characters.
#[test]
fn building_a_note() {
    let mut data = read_str(&v7("0 @I1@ INDI\n"));
    let mut note = Note::text("Written by a program");
    note.detail_mut().language = Some("en".into());
    assert_eq!(note.detail().language.as_ref().unwrap().to_str(&data), "en");
    data.individuals[0].notes.push(note);
    assert!(write(&data).contains("1 NOTE Written by a program\n2 LANG en\n"));
    let id = data.store.intern_xref("@N9@").unwrap();
    assert_eq!(data.store.xref(id), "@N9@");
}

/// A small deterministic generator of GEDCOM-like inputs: standard and
/// extension tags at random levels, pointers, enumeration values, dates,
/// odd payloads.
fn generated_inputs(n: usize) -> Vec<String> {
    const TAGS: &[&str] = &[
        "INDI", "FAM", "SOUR", "NOTE", "SNOTE", "OBJE", "REPO", "SUBM", "BIRT", "DEAT", "MARR",
        "NAME", "TYPE", "PEDI", "STAT", "ADOP", "FAMC", "FAMS", "HUSB", "WIFE", "CHIL", "ASSO",
        "ROLE", "RELA", "PHRASE", "DATE", "TIME", "AGE", "PLAC", "MAP", "LATI", "LONG", "FORM",
        "TRAN", "LANG", "MIME", "ADDR", "CITY", "PHON", "CHAN", "CREA", "REFN", "UID", "EXID",
        "PAGE", "DATA", "TEXT", "EVEN", "QUAY", "CALN", "MEDI", "FILE", "TITL", "CROP", "TOP",
        "WIDTH", "SEX", "RESN", "BAPL", "SLGS", "KIND", "FONE", "ROMN", "CONT", "CONC", "_X", "",
    ];
    const PAYLOADS: &[&str] = &[
        "",
        "@I1@",
        "@F1@",
        "@S1@",
        "@N1@",
        "@VOID@",
        "@@x",
        "text",
        "OTHER",
        "birth",
        "BIRTH",
        "_EXT",
        "M",
        "x",
        "1 JAN 1900",
        "@#DJULIAN@ 1 JAN 1700",
        "ABT 1900/01",
        "79",
        "1y 400d",
        "10:00",
        "2:5",
        "N1.5",
        "image/jpeg",
        "en",
        "3",
        "0",
        "-1",
        "99999999999",
        "DNS/CAN",
        "PRE_1970",
        "CONFIDENTIAL, LOCKED",
        "a, b",
        "Y",
    ];
    let mut state: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = |m: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % m as u64) as usize
    };
    (0..n)
        .map(|_| {
            let vers = ["5.5.1", "7.0", "7.1"][next(3)];
            let mut s = format!("0 HEAD\n1 GEDC\n2 VERS {vers}\n");
            let records = 1 + next(4);
            for r in 0..records {
                let xref = ["@I1@", "@F1@", "@S1@", "@N1@", "@X@"][next(5)];
                s += &format!(
                    "0 {xref} {}\n",
                    ["INDI", "FAM", "SOUR", "NOTE", "SNOTE", "_R"][r % 6]
                );
                let mut level: usize = 1;
                for _ in 0..next(25) {
                    level = (level + 2).saturating_sub(next(4)).clamp(1, 6);
                    let tag = TAGS[next(TAGS.len())];
                    let payload = PAYLOADS[next(PAYLOADS.len())];
                    s += &format!("{level} {tag} {payload}\n");
                }
            }
            s + "0 TRLR\n"
        })
        .collect()
}

/// A written text's structures, records and siblings of different tags in
/// tag order (their relative order carries no meaning; the order within a
/// tag does).
fn canonical(text: &str) -> Vec<ged_io::tree::Structure> {
    fn sort(s: &mut ged_io::tree::Structure) {
        s.substructures.iter_mut().for_each(sort);
        s.substructures
            .sort_by(|a, b| a.tag.as_str().cmp(b.tag.as_str()));
    }
    let mut records = ged_io::tree::parse_tree(text).to_structures();
    records.iter_mut().for_each(sort);
    records.sort_by(|a, b| a.tag.as_str().cmp(b.tag.as_str()));
    records
}

/// For any input and every version, the written output reads back with
/// every structure the model types read typed, and writing it again gives
/// the same structures.
#[test]
fn written_output_reads_typed_and_is_stable() {
    for input in generated_inputs(400) {
        let data = read_str(&input);
        for version in [
            GedcomVersion::V5_5_1,
            GedcomVersion::V7_0,
            GedcomVersion::V7_1,
        ] {
            let out = write_as(&data, version);
            let again = read_str(&out);
            let untyped = ged_io::next::ledger::untyped(&again);
            assert!(untyped.is_empty(), "{untyped:?}\n{input}\n{out}");
            let twice = write_as(&again, version);
            assert_eq!(
                canonical(&twice),
                canonical(&out),
                "{input}\n{out}\n{twice}"
            );
        }
    }
}

/// Inputs the `model` fuzz target found unstable or untyped once written:
/// each written output reads back typed and writes again the same.
#[test]
fn fuzz_findings_are_stable() {
    for input in [
        "0 HEAD\n1 SCHMA\n2 TAG \n3 TAG _C t:",
        "0 INDI\n1 EVEN\n2 AGE t\n2 AGE > 0m",
        "0 INDI\nE\n1 NAME\n2 TYPE AKA",
        "0 INDI\nh\n1 EVEN\n2 AGE > 9y",
        "0 SOUR\n1 DATA\n2 EVEN",
        "0 SOUR\n1 DATA\n2 EVEN \u{0}\n@",
        "0 SOUR\n1 DATA\n2 EVEN \u{0} J",
        "0 @S1 SOUR\n0 INDI\n2 SOUR @S1@\n3 EVEN",
    ] {
        let data = read_str(input);
        for version in [
            GedcomVersion::V5_5_1,
            GedcomVersion::V7_0,
            GedcomVersion::V7_1,
        ] {
            let out = write_as(&data, version);
            let again = read_str(&out);
            let untyped = ged_io::next::ledger::untyped(&again);
            assert!(
                untyped.is_empty(),
                "{input:?} {version}: {untyped:?}\n{out}"
            );
            let twice = write_as(&again, version);
            assert_eq!(canonical(&twice), canonical(&out), "{input:?} {version}");
        }
    }
}

/// A value only the other version names is kept as written: a 5.5.1
/// `OTHER` medium (7.x only) is not read as 7.x's `OTHER`, which 5.5.1
/// could not write back.
#[test]
fn values_of_the_other_version_keep_their_spelling() {
    let data = read_str(&v551(
        "0 @S1@ SOUR\n1 REPO @R1@\n2 CALN 1\n3 MEDI Other\n2 CALN 2\n3 MEDI Photo\n0 @R1@ REPO\n1 NAME Archive\n",
    ));
    let repo = &source(&data, "@S1@").repositories[0];
    assert!(matches!(
        &repo.call_numbers[0].medium.as_ref().unwrap().value,
        Medium::Unknown(t) if t.to_str(&data) == "Other"
    ));
    assert_eq!(
        repo.call_numbers[1].medium.as_ref().unwrap().value,
        Medium::Photo
    );
    let out = write(&data);
    assert!(out.contains("3 _MEDI Other\n"), "{out}");
    assert!(out.contains("3 MEDI photo\n"), "{out}");
}
