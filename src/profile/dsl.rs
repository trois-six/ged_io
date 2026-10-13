//! The `macro_rules!` language the profiles are written in: compile-checked
//! static data, no build script, no procedural macro, no dependency.
//!
//! ```text
//! import_profile! {
//!     FTM = "ftm", "Family Tree Maker", tier 1, parent Some("generic");
//!     detect [ sig!(Sour, Eq("FTM"), 100), sig!(Tag, Eq("_FREL"), 10) ];
//!     charset [ ("ANSI", GedcomEncoding::Windows1252) ];
//!     stats true;
//!     rules [
//!         rule!("FAM.CHIL", "_FREL", Op::Pedigree, Partial, "X-056"),
//!     ];
//! }
//! ```

/// A detection signature: a header field, a test, a weight.
macro_rules! sig {
    ($field:ident, $test:ident($value:expr), $weight:expr) => {
        $crate::profile::detect::Sig {
            field: $crate::profile::detect::Field::$field,
            test: $crate::profile::detect::Test::$test($value),
            weight: $weight,
        }
    };
}

/// An import rule: context, tag, operation, fidelity and the catalogue rows
/// it implements; optionally gated on the producer's version.
macro_rules! rule {
    ($ctx:expr, $tag:expr, $op:expr, $fid:ident, $($row:literal),+) => {
        $crate::profile::interpret::Rule {
            ctx: $ctx,
            tag: $tag,
            op: $op,
            fidelity: $crate::profile::interpret::Fidelity::$fid,
            rows: &[$($row),+],
            gate: $crate::profile::detect::Gate::Always,
        }
    };
    ($ctx:expr, $tag:expr, $op:expr, $fid:ident, $($row:literal),+ ; $gate:expr) => {
        $crate::profile::interpret::Rule {
            ctx: $ctx,
            tag: $tag,
            op: $op,
            fidelity: $crate::profile::interpret::Fidelity::$fid,
            rows: &[$($row),+],
            gate: $gate,
        }
    };
}

/// An import profile, as a `static`.
macro_rules! import_profile {
    (
        $(#[$meta:meta])*
        $name:ident = $id:literal, $label:literal, tier $tier:literal, parent $parent:expr;
        detect [$($sig:expr),* $(,)?];
        charset [$($cs:expr),* $(,)?];
        stats $stats:literal;
        rules [$($rule:expr),* $(,)?];
    ) => {
        $(#[$meta])*
        pub(crate) static $name: $crate::profile::Profile = $crate::profile::Profile {
            id: $id,
            label: $label,
            tier: $tier,
            parent: $parent,
            detect: &[$($sig),*],
            charset: &[$($cs),*],
            stats: $stats,
            rules: &[$($rule),*],
        };
    };
}

/// An export step: a fold and the rows of `export.tsv` it implements.
macro_rules! step {
    ($fold:expr, $($row:literal),+) => {
        $crate::profile::export::Step { fold: $fold, rows: &[$($row),+] }
    };
}

/// An export profile, as a `static`.
macro_rules! export_profile {
    (
        $(#[$meta:meta])*
        $name:ident = $id:literal, $label:literal, tier $tier:literal;
        target $target:expr;
        steps [$($step:expr),* $(,)?];
    ) => {
        $(#[$meta])*
        pub(crate) static $name: $crate::profile::ExportProfile = $crate::profile::ExportProfile {
            id: $id,
            label: $label,
            tier: $tier,
            target: $target,
            steps: &[$($step),*],
        };
    };
}

pub(crate) use {export_profile, import_profile, rule, sig, step};
