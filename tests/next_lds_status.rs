//! An LDS ordinance status is never dropped: a value of either version's
//! enumeration reads as its variant, any other as `Unknown` with its text,
//! and both are written back with their `DATE` (upstream #107, ported to
//! the typed model `ged_io::next`).

use ged_io::next::{read_str, write_string, OrdinanceKind, OrdinanceStatus};
use ged_io::GedcomWriter;

#[test]
fn test_round_trip_lds_status_outside_the_enumeration_gedcom_5() {
    let source = "\
0 HEAD
1 GEDC
2 VERS 5.5.1
2 FORM LINEAGE-LINKED
1 CHAR UTF-8
0 @I1@ INDI
1 NAME Ann /Example/
1 BAPL
2 DATE 15 MAR 1990
2 STAT EXCLUDED
3 DATE 1 JAN 2000
1 ENDL
2 STAT COMPLETED
0 TRLR";
    let data = read_str(source);

    let ordinances = &data.individual("@I1@").unwrap().detail().ordinances;
    assert_eq!(ordinances[0].kind, OrdinanceKind::Baptism);
    assert_eq!(ordinances[1].kind, OrdinanceKind::Endowment);
    let excluded = ordinances[0].status.as_ref().unwrap();
    let completed = ordinances[1].status.as_ref().unwrap();
    assert_eq!(excluded.value, OrdinanceStatus::Excluded);
    assert_eq!(
        excluded.date.as_ref().unwrap().value.to_str(&data),
        "1 JAN 2000"
    );
    assert_eq!(completed.value, OrdinanceStatus::Completed);

    let written = write_string(&data, &GedcomWriter::new()).unwrap();
    assert!(
        written.contains("1 BAPL\n2 DATE 15 MAR 1990\n2 STAT EXCLUDED\n3 DATE 1 JAN 2000\n"),
        "{written}"
    );
    // A 5.5.1 status requires its DATE: without one, the conformant writer
    // keeps it as an extension structure, value included.
    assert!(written.contains("1 ENDL\n2 _STAT COMPLETED\n"), "{written}");
    let read = read_str(&written);
    let back = read.individual("@I1@").unwrap().detail().ordinances[0]
        .status
        .as_ref()
        .unwrap();
    assert_eq!(back.value, excluded.value);
}

#[test]
fn test_round_trip_lds_extension_status_gedcom_7() {
    let source = "\
0 HEAD
1 GEDC
2 VERS 7.0
0 @F1@ FAM
1 SLGS
2 STAT _PENDING
3 DATE 2 APR 2001
0 TRLR";
    let data = read_str(source);

    let slgs = &data.family("@F1@").unwrap().detail().ordinances[0];
    assert_eq!(slgs.kind, OrdinanceKind::SpouseSealing);
    let sealing = slgs.status.as_ref().unwrap();
    assert!(matches!(&sealing.value, OrdinanceStatus::Unknown(t) if t.to_str(&data) == "_PENDING"));

    let written = write_string(&data, &GedcomWriter::new()).unwrap();
    assert!(
        written.contains("1 SLGS\n2 STAT _PENDING\n3 DATE 2 APR 2001\n"),
        "{written}"
    );
    let read = read_str(&written);
    let back = read.family("@F1@").unwrap().detail().ordinances[0]
        .status
        .as_ref()
        .unwrap();
    assert!(matches!(&back.value, OrdinanceStatus::Unknown(t) if t.to_str(&read) == "_PENDING"));
}
