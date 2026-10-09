# Changelog

All notable changes to this project are documented in this file. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased] — 0.18.0

A new model and a new reading and writing pipeline: reading never fails and
keeps all data; writing is conformant to GEDCOM 5.5.1, 7.0 or 7.1.

### Added

- `ged_io::model`: a typed model of every structure of GEDCOM 5.5.1, 7.0
  and 7.1 (`Dataset`, `Individual`, `Family`, `Source`, `Repository`,
  `Multimedia`, `Submitter`, `Submission`, `SharedNote`, `Event`, `Name`,
  `Citation`, `Place`, `Ordinance`, …). Each structure keeps what it has no
  field for in `extra`, in place; enumerations keep unknown values
  (`Unknown(Text)`) and 7.x `PHRASE`s (`Phrased`). Texts are spans of the
  decoded input (`Text`, `Store`), identifiers interned (`XrefId`).
- `Dataset::parse`, `Dataset::from_bytes`, `Dataset::new`, `Dataset::push`;
  lookups `find*` by text or `XrefId`; navigation (`families_as_spouse`,
  `families_as_child`, `parents`, `children`, `spouse`,
  `search_individuals`); `dangling_references()`.
- `ged_io::tree`: a lossless tree of any file (`Tree`, `parse_tree`,
  `TreeReader`, `Structure`, `Xref`, `Tag`).
- `ged_io::spec`: the 5.5.1, 7.0 and 7.1 specification tables, a validator
  (`validate`, `validate_text`, `validate_bytes`: every deviation with its
  line) and a conformance repair (`conform`).
- `ged_io::value`: date, age and time grammars of both versions, lenient
  and strict, conversions between versions; `Date::to_version`,
  `Date::normalize`, `Age::to_version`.
- `GedcomBuilder::strict`: refuse input that does not follow its
  specification, with every deviation (`GedcomError::NonConformant`).
- `GedcomBuilder::build_from_reader`; GEDZIP read through the builder
  (`GedzipReader::read_dataset`).
- `GedcomStreamParser::nodes`; `Dataset: FromIterator<StreamedRecord> +
  Extend<StreamedRecord>`.
- `IndexedDataset`: constant-time lookups of every record type (shared notes
  included) and of families from individuals; `data_mut` reindexes.
- `GedcomWriter::write` to any `io::Write`, `write_tree`,
  `write_structures`, `write_to_string_with_report`; `WriteReport` of the
  repairs; `RepairPolicy`; `OutputEncoding` (5.5.1 in ANSEL, ASCII,
  UTF-16); `Bom`; GEDCOM 7.1 as a write target.
- Encodings: Windows-1252 (`ANSI`), IBM PC (cp437), Macintosh, UTF-16
  without BOM; ANSEL composed to Unicode NFC; `DecodeReader` for streams;
  decoding never fails.
- Feature `serde`: `Serialize`/`Deserialize` for `Dataset` (texts and
  identifiers as strings), `Serialize` for `StreamedRecord`.
- CLI: `ged-io --write <5.5.1|7.0|7.1>` writes a file conformant;
  `--validate` reports every deviation with its line.

### Changed

- `GedcomBuilder` returns a `Dataset`, takes `&self` and has two options,
  `strict` and `max_file_size`; `build_from_str` and `build_from_bytes`
  keep an owned `String` or UTF-8 `Vec<u8>` as the dataset's text, without
  a copy.
- `GedcomStreamParser` yields `StreamedRecord`s, reads any encoding and line
  terminator, and gives correct line numbers.
- `GedcomError` is `#[non_exhaustive]`: `Io`, `FileTooLarge`,
  `NonConformant`, `Gedzip`. `GedzipError` is `#[non_exhaustive]`: `Zip`,
  `MissingGedcom`, `MissingMedia`, `EntryTooLarge`, `Io`, `Write`.
- `GedcomWriter` writes a `Dataset`; its output is conformant to the target
  version (lines, escapes, identifiers, header, structures, values) and
  every repair is reported; `GedcomVersion` is an enum
  (`V5_5_1`, `V7_0`, `V7_1`) with `VersionRules`.
- `encoding::encode_to_bytes` is `encoding::encode`, failing with
  `EncodeError`.
- The feature `json` is `serde`; `serde_json` is no longer a dependency.
- The command-line binary is `ged-io` (it was `ged_io`, the library's name).
- The JSON shape follows the new model.
- MSRV 1.88.

### Removed

- The old model (`ged_io::types`, `GedcomData`, `UserDefinedTag`,
  `HasEvents`), `Gedcom::new(…).parse_data()`, the `tokenizer` and `parser`
  modules (`Tokenizer`, `StreamTokenizer`, `TokenizerTrait`, `StreamParser`,
  `parse_subset`), `util`, the `Display` and `ImprovedDebug` impls,
  `ParserConfig` (`strict_mode`, `validate_references`,
  `ignore_unknown_tags`, `encoding_detection`, `date_validation`,
  `preserve_formatting`), `GedcomData::stats`, `appears_to_be_v7`, and the
  error variants `ParseError`, `InvalidFormat`, `EncodingError`,
  `InvalidTag`, `UnexpectedLevel`, `MissingRequiredValue`,
  `InvalidValueFormat`, `FileSizeLimitExceeded`, `IoError`.

### Fixed

- Reading accepts CR, LF CR and mixed terminators, blank lines, lines
  without a level, level jumps, `CONT`/`CONC` anywhere, extension tags under
  any structure, unknown records, a second `HUSB`/`WIFE`, xrefs on
  substructures, unknown enumeration values and content after `TRLR`,
  keeping every line's data.
- Repeated structures, `REFN` types, `EXID` types, every `PHRASE`, name
  pieces, contacts, notes, citations and the other substructures 0.17
  dropped or overwrote are kept; no `SURN` is derived from a name.
- The writer no longer writes `FORM`/`CHAR` in 7.0, `CONC` in 7.0, lines
  over 255 characters in 5.5.1, lower-case 7.0 enumeration values, raw
  control characters, duplicate or `@VOID@` record identifiers, text as
  pointers, or a value whose line breaks inject records; the last line is
  terminated.
- No panic or hang from any input (Hebrew dates, levels over 255, the
  `Display` of notes, version detection).
