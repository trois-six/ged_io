//! The header of a written file, completed for the target version.
//!
//! Whatever header the data carries (or none), the written `HEAD` has:
//!
//! - `GEDC` with `VERS` naming the target version and, in 5.5.1 only,
//!   `FORM LINEAGE-LINKED`;
//! - in 5.5.1, `CHAR` naming the encoding of the output bytes; the input's
//!   `CHAR` (and its `VERS`, which describes the input bytes) is not
//!   carried, and 7.x has no `CHAR`;
//! - in 5.5.1, the required `SOUR` (`ged_io` and its version when the data
//!   has none) and `SUBM` pointer;
//! - no cross-reference identifier.
//!
//! Every other substructure is kept, in its order.

use crate::tree::{Payload, Structure, Xref};
use crate::version::VersionRules;

/// The product written as `HEAD.SOUR` when the data names none.
pub(crate) const PRODUCT: &str = "ged_io";

/// Whether the completed header needs a `SUBM` pointer that `head` lacks.
pub(crate) fn needs_submitter(head: &Structure, rules: &VersionRules) -> bool {
    rules.head_sour_subm && head.first("SUBM").is_none()
}

/// Completes `head` for `rules`. `charset` is the `CHAR` payload of the
/// output; `submitter` the pointer to add when [`needs_submitter`] holds.
pub(crate) fn complete(
    head: &mut Structure,
    rules: &VersionRules,
    charset: &str,
    submitter: Option<String>,
) {
    head.xref = None;
    head.tag = "HEAD".into();
    head.payload = Payload::None;

    // GEDC first, with its VERS (and 5.5.1 FORM) first.
    let mut gedc = match head.substructures.iter().position(|s| s.tag == "GEDC") {
        Some(at) => head.substructures.remove(at),
        None => Structure::new("GEDC"),
    };
    gedc.xref = None;
    gedc.payload = Payload::None;
    gedc.substructures
        .retain(|s| s.tag != "VERS" && s.tag != "FORM");
    let mut front = vec![text("VERS", rules.vers_payload)];
    if let Some(form) = rules.gedc_form {
        front.push(text("FORM", form));
    }
    gedc.substructures.splice(0..0, front);

    head.substructures.retain(|s| s.tag != "CHAR");
    let mut front = vec![gedc];
    if rules.head_char {
        front.push(text("CHAR", charset));
    }
    if rules.head_sour_subm && head.first("SOUR").is_none() {
        let mut sour = text("SOUR", PRODUCT);
        sour.substructures
            .push(text("VERS", env!("CARGO_PKG_VERSION")));
        sour.substructures.push(text("NAME", PRODUCT));
        front.push(sour);
    }
    head.substructures.splice(0..0, front);
    if let Some(submitter) = submitter {
        head.substructures.push(Structure {
            payload: Payload::Pointer(Xref::new(submitter)),
            ..Structure::new("SUBM")
        });
    }
}

/// The stub submitter record a 5.5.1 file without one points to.
pub(crate) fn stub_submitter(xref: String) -> Structure {
    let mut subm = Structure {
        xref: Some(Xref::new(xref)),
        ..Structure::new("SUBM")
    };
    subm.substructures.push(text("NAME", "Unknown"));
    subm
}

fn text(tag: &str, value: &str) -> Structure {
    Structure {
        payload: Payload::Text(value.into()),
        ..Structure::new(tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::parse_tree;
    use crate::version::{V551, V70};

    fn head(text: &str) -> Structure {
        parse_tree(text).to_structures().remove(0)
    }

    #[test]
    fn seven_has_no_char_and_no_form() {
        let mut h = head(
            "0 HEAD\n1 CHAR ANSEL\n2 VERS 1\n1 GEDC\n2 VERS 5.5\n2 FORM LINEAGE-LINKED\n1 _X y\n",
        );
        complete(&mut h, &V70, "UTF-8", None);
        assert_eq!(
            h.to_gedcom(0, crate::GedcomVersion::V7_0),
            "0 HEAD\n1 GEDC\n2 VERS 7.0\n1 _X y\n"
        );
    }

    #[test]
    fn five_gets_its_required_lines() {
        let mut h =
            head("0 @H@ HEAD\n1 CHAR UNICODE\n1 CHAR ANSEL\n1 GEDC\n2 FORM Lineage-Linked\n");
        assert!(needs_submitter(&h, &V551));
        complete(&mut h, &V551, "UTF-8", Some("@U1@".into()));
        let out = h.to_gedcom(0, crate::GedcomVersion::V5_5_1);
        assert!(out.starts_with(
            "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n2 FORM LINEAGE-LINKED\n1 CHAR UTF-8\n1 SOUR ged_io\n2 VERS "
        ), "{out}");
        assert!(out.ends_with("2 NAME ged_io\n1 SUBM @U1@\n"), "{out}");
    }
}
