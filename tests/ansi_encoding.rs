//! `CHAR ANSI` is the Windows code page of the producing system, Windows-1252
//! for Western European files: its bytes are decoded as such, not as UTF-8.
//! (Carried over from upstream PR #105.)

use ged_io::encoding::{decode, detect_encoding, GedcomEncoding};
use ged_io::GedcomBuilder;

/// A 5.5.1 file declaring `CHAR ANSI` whose names hold Windows-1252 bytes:
/// `é` (0xE9, shared with Latin-1), `œ` (0x9C) and `€` (0x80), the latter two
/// only in Windows-1252.
const ANSI_FILE: &[u8] = b"0 HEAD\n\
                           1 GEDC\n\
                           2 VERS 5.5.1\n\
                           1 CHAR ANSI\n\
                           0 @I1@ INDI\n\
                           1 NAME Ren\xE9e /Exampl\x9Cuf/\n\
                           1 NOTE Paid 5 \x80\n\
                           0 TRLR\n";

#[test]
fn test_ansi_file_is_decoded_as_windows_1252() {
    let data = GedcomBuilder::new().build_from_bytes(ANSI_FILE).unwrap();

    let individual = &data.individuals[0];
    assert_eq!(
        individual.names[0].value.as_deref(),
        Some("Renée /Examplœuf/")
    );
    assert_eq!(individual.notes[0].value.as_deref(), Some("Paid 5 €"));
}

#[test]
fn test_ansi_bytes_are_decoded_as_windows_1252() {
    assert_eq!(detect_encoding(ANSI_FILE), GedcomEncoding::Windows1252);
    let decoded = decode(ANSI_FILE);

    assert_eq!(decoded.encoding, GedcomEncoding::Windows1252);
    assert_eq!(decoded.declared.as_deref(), Some("ANSI"));

    assert!(decoded.text.contains("1 NAME Renée /Examplœuf/\n"));
    assert!(decoded.text.contains("1 NOTE Paid 5 €\n"));
}

#[test]
fn test_ansi_file_holding_utf8_is_read_as_utf8() {
    // Some producers declare `ANSI` and write UTF-8: the bytes win.
    let bytes = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n1 CHAR ANSI\n\
                 0 @I1@ INDI\n1 NAME Renée /Example/\n0 TRLR\n"
        .as_bytes();

    assert_eq!(detect_encoding(bytes), GedcomEncoding::Utf8);
    let data = GedcomBuilder::new().build_from_bytes(bytes).unwrap();
    assert_eq!(
        data.individuals[0].names[0].value.as_deref(),
        Some("Renée /Example/")
    );
}
