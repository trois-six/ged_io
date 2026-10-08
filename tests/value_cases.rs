//! The test-suite plan's date, age and time cases (`G7-AGE-*`, `G7-DATE-*`,
//! `G7-TIME`, `G5-DATES`, `G5-AGES`, `G5-AMBIG-NUMERIC-DATE`): each file is
//! read, then written in its own version, whose output must hold the `want`
//! lines; the `has` texts must be in the dates and ages read. `G7-AGE-OVERFLOW` and
//! `G7-AGE-INVALID` failed on 0.17 (root cause TS14).

use ged_io::{GedcomBuilder, GedcomWriter};

struct Case {
    id: &'static str,
    file: &'static str,
    want: &'static [&'static str],
    has: &'static [&'static str],
}

const CASES: &[Case] = &[
    Case {
        id: "G7-AGE-ALL",
        file: "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 DEAT\n2 AGE > 79y 1m 1w 1d\n1 BURI\n2 AGE < 1m\n1 CHR\n2 AGE 0y\n3 PHRASE Stillborn\n0 TRLR\n",
        want: &["2 AGE > 79y 1m 1w 1d", "2 AGE < 1m", "3 PHRASE Stillborn"],
        has: &[],
    },
    Case {
        id: "G7-AGE-OVERFLOW",
        file: "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 CHR\n2 AGE 1y 30m 100w 400d\n0 TRLR\n",
        want: &["2 AGE 1y 30m 100w 400d"],
        has: &[],
    },
    Case {
        id: "G7-AGE-INVALID",
        file: "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 DEAT\n2 AGE 79\n0 TRLR\n",
        want: &[],
        has: &["\"79\""],
    },
    Case {
        id: "G7-DATE-FORMS",
        file: "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 BIRT\n2 DATE BET GREGORIAN 20 BCE AND GREGORIAN 12 BCE\n1 DEAT\n2 DATE FROM HEBREW 1 TSH 1\n1 BURI\n2 DATE EST JULIAN 16 AUG 918\n1 CHR\n2 DATE ABT 3 DEC 2023\n1 CREM\n2 DATE FRENCH_R 1 VEND 1\n0 TRLR\n",
        want: &[
            "2 DATE BET GREGORIAN 20 BCE AND GREGORIAN 12 BCE",
            "2 DATE FROM HEBREW 1 TSH 1",
            "2 DATE EST JULIAN 16 AUG 918",
            "2 DATE FRENCH_R 1 VEND 1",
        ],
        has: &[],
    },
    Case {
        id: "G7-DATE-INVALID",
        file: "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 BIRT\n2 DATE 32 JAN 2000\n1 DEAT\n2 DATE BET 2000\n0 TRLR\n",
        want: &[],
        has: &["32 JAN 2000", "BET 2000"],
    },
    Case {
        id: "G7-DATE-EXT-CAL",
        file: "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 BIRT\n2 DATE _MAYAN 1 POP 1\n0 TRLR\n",
        want: &["2 DATE _MAYAN 1 POP 1"],
        has: &[],
    },
    Case {
        id: "G7-DATE-PHRASE-ONLY",
        file: "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 BIRT\n2 DATE\n3 PHRASE Easter\n0 TRLR\n",
        want: &["3 PHRASE Easter"],
        has: &[],
    },
    Case {
        id: "G7-TIME",
        file: "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 CHAN\n2 DATE 1 DEC 2023\n3 TIME 2:50:00.00Z\n0 TRLR\n",
        want: &["3 TIME 2:50:00.00Z"],
        has: &[],
    },
    Case {
        id: "G5-AMBIG-NUMERIC-DATE",
        file: "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n2 FORM LINEAGE-LINKED\n1 CHAR UTF-8\n0 @I1@ INDI\n1 BIRT\n2 DATE 7/11/1959\n1 DEAT\n2 DATE 25/3/1934\n0 TRLR\n",
        want: &["2 DATE 7/11/1959", "2 DATE 25/3/1934"],
        has: &[],
    },
    Case {
        id: "G5-DATES",
        file: "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n2 FORM LINEAGE-LINKED\n1 CHAR UTF-8\n0 @I1@ INDI\n1 BIRT\n2 DATE 1740/41\n1 CHR\n2 DATE @#DGREGORIAN@ 20 B.C.\n1 DEAT\n2 DATE @#DHEBREW@ 1 TSH 5760\n1 BURI\n2 DATE @#DFRENCH R@ 1 VEND 1\n1 CREM\n2 DATE INT 1900 (about nineteen hundred)\n1 PROB\n2 DATE (Easter)\n1 WILL\n2 DATE @#DJULIAN@ 1 JAN 1700\n0 TRLR\n",
        want: &[
            "2 DATE 1740/41",
            "2 DATE @#DGREGORIAN@ 20 B.C.",
            "2 DATE @#DHEBREW@ 1 TSH 5760",
            "2 DATE @#DFRENCH R@ 1 VEND 1",
            "2 DATE INT 1900 (about nineteen hundred)",
            "2 DATE (Easter)",
            "2 DATE @#DJULIAN@ 1 JAN 1700",
        ],
        has: &[],
    },
    Case {
        id: "G5-AGES",
        file: "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n2 FORM LINEAGE-LINKED\n1 CHAR UTF-8\n0 @I1@ INDI\n1 DEAT\n2 AGE STILLBORN\n1 BURI\n2 AGE <1y\n1 CREM\n2 AGE >79y\n0 TRLR\n",
        want: &["2 AGE STILLBORN"],
        has: &["1y", "79y"],
    },
];

#[test]
fn test_date_age_and_time_cases() {
    for case in CASES {
        let data = GedcomBuilder::new()
            .build_from_str(case.file)
            .unwrap_or_else(|error| panic!("{}: {error}", case.id));
        let written = GedcomWriter::new().write_to_string(&data).unwrap();
        for want in case.want {
            assert!(
                written.lines().any(|line| line == *want),
                "{}: missing {want:?} in:\n{written}",
                case.id
            );
        }
        // The dates and ages read (an event's `Debug` leaves them out).
        let dump: String = data
            .individuals
            .iter()
            .flat_map(|individual| &individual.events)
            .map(|event| format!("{:?} {:?}\n", event.date, event.age))
            .collect();
        for has in case.has {
            assert!(dump.contains(has), "{}: lacks {has:?}", case.id);
        }
    }
}

#[test]
fn test_g5_dates_as_gedcom_7() {
    let data = GedcomBuilder::new().build_from_str(CASES[9].file).unwrap();
    let written = GedcomWriter::new()
        .gedcom_version(ged_io::GedcomVersion::V7_0)
        .write_to_string(&data)
        .unwrap();
    for want in [
        "2 DATE BET 1740 AND 1741\n3 PHRASE 1740/41",
        "2 DATE 20 BCE",
        "2 DATE HEBREW 1 TSH 5760",
        "2 DATE FRENCH_R 1 VEND 1",
        "2 DATE 1900\n3 PHRASE about nineteen hundred",
        "2 DATE\n3 PHRASE Easter",
        "2 DATE JULIAN 1 JAN 1700",
    ] {
        assert!(written.contains(want), "missing {want:?} in:\n{written}");
    }
    assert!(!written.contains("@#D"), "{written}");
}
