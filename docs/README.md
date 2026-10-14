# A note on GEDCOM Specifications

## Versions

- Version `5.5.1` was the draft standard the 1999-10-02 and the official standard at 2019-11-15
- Version `5.5.5` is a cleanup and stricter version of the Version `5.5.1`
- Version `7.0` is an upgrade of the Version `5.5.1`

## The specifications in this directory

- `FamilySearchGEDCOMv7.pdf`: The FamilySearch GEDCOM Specification 7.0.18
  (17 February 2026), the latest 7.0 release, as FamilySearch publishes it:
  the `gedcom.pdf` asset of the
  [v7.0.18 release](https://github.com/FamilySearch/GEDCOM/releases/tag/v7.0.18)
  (tag commit `2f0b863`, SHA-256
  `ec4475db84781878086bb3882d751c685fea6fa3d3d68b1ed90f581ec7dd832a`),
  Apache License 2.0, © 1984-2026 Intellectual Reserve, Inc. The crate's 7.0
  tables come from the same release (`tools/spec-tables/gen.py`). 7.0.17
  (10 February 2026) added the URI, tag definition, latitude and longitude data
  types and clarified pointer cycles, file paths and GEDZIP names; 7.0.18
  corrected its longitude ABNF. GEDCOM 7.1 is not released: the crate
  follows FamilySearch's `v7.1` branch as a draft.
- `ged551.pdf`: GEDCOM 5.5.1 (2019-11-15 release of the 1999 draft).
- `gedcom-7.md`: the differences between GEDCOM 5.5.1 and 7.0, and how
  oxidged reads and writes each (its examples are compiled and run as
  documentation tests).
- `extensions.md`: the producers' extension tags oxidged keeps and declares
  in GEDCOM 7.x headers, one anchor (the URI's fragment) per producer and
  tag; generated from the profiles (`tests/profiles.rs`).

## GEDZip

[GEDZip](https://gedcom.io/gedzip/) is a specification included in the Version `7.0`. It is used to transmit a gedcom file together with a set of external files and it's a ZIP file (following the ZIP specifications) containing the data.

## Gedcom X

[Gedcom X](https://gedcomx.org/) is an alternative format and different format to GEDCOM. Gedcom X Specifications are available on the repository <https://github.com/FamilySearch/gedcomx>

## Sources

- <https://en.wikipedia.org/wiki/GEDCOM>
- <https://fr.geneawiki.com/wiki/Norme_Gedcom>
- <https://www.gedcom.org/versions.html>
- <https://gedcomx.org/>
