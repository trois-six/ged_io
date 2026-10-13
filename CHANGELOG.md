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
  lookups `find*` by text or `XrefId`, in constant time; navigation
  (`families_as_spouse`, `families_as_child`, `parents`, `children`,
  `spouse`, `search_individuals`); `dangling_references()`.
- Referential integrity and editing (`ged_io::model`, README "Editing"):
  `Handle<T>` and `RecordHandle`, valid across every edit until their
  record is removed (`handle`, `handles`, `handle_at`, `record_handle`,
  `get`, `get_any`, `position`, `records_with_handles`); `add` (refuses a
  duplicate identifier, pointers to no record and ids of another store;
  gives a record without identifier a free one), `remove` with `Removal`
  (`Refuse` while referenced, `Detach` the structures pointing to it,
  `LeaveDangling`), `rename` (every pointer follows, in constant time),
  `redirect`, `detach`; `link`/`unlink` of every pointer kind (`Link`:
  family links, aliases, associations, citations, notes, media,
  repositories, submitters, interests, the header's submission,
  extension structures), `link_child`/`link_spouse` and their unlinks
  both ways; `get_mut`, `get_mut_at`, `header_mut` and `edit_all` lend
  records (`RecordMut`) whose pointers are counted again when the loan
  ends; `references` (constant time), `referrers`, `pointer`;
  `EditError`. The counts and handles are built the first time they are
  asked for: reading and writing never pay for them.
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
- `IndexedDataset`: constant-time lookups of the families of individuals
  (records are found by the dataset itself); `data_mut` reindexes.
- `GedcomWriter::write` to any `io::Write`, `write_tree`,
  `write_structures`, `write_to_string_with_report`; `WriteReport` of the
  repairs; `RepairPolicy`; `OutputEncoding` (5.5.1 in ANSEL, ASCII,
  UTF-16); `Bom`; GEDCOM 7.1 as a write target.
- `GedcomStreamWriter`: writes a header, then records one at a time (from a
  `GedcomStreamParser` or a dataset), then the trailer, with the
  conformance repair, conversion between version families and line rules
  of `GedcomWriter` and memory bounded by one record; its documentation
  lists where it differs from writing the whole dataset (identifiers,
  pointers, stub submitter, relocations, records a conversion adds,
  `HEAD.SCHMA` declarations).
- Encodings: Windows-1252 (`ANSI`), IBM PC (cp437), Macintosh, UTF-16
  without BOM; ANSEL composed to Unicode NFC; `DecodeReader` for streams;
  decoding never fails.
- Feature `serde`: `Serialize`/`Deserialize` for `Dataset` (texts and
  identifiers as strings), `Serialize` for `StreamedRecord`.
- 5.5.1 file formats (`FILE.FORM`) are an open list, by decision: any
  word is accepted and written as given (`png`, `pdf`), a media type
  (`image/png`) is written as its file format, and a missing `FORM` is
  derived from the file's extension; the validator reports only a media
  type there. 7.x keeps media types.
- CLI: `ged-io --write <5.5.1|7.0|7.1>` writes a file conformant;
  `--validate` reports every deviation with its line.
- `ged_io::convert`: conversion between GEDCOM 5.5.1 and 7.x, run by the
  writer when the target is of the other family, and as functions
  (`convert_structures`, `convert_dataset`): multimedia links holding files
  and source citations in words become records, `ASSO.RELA` the `ROLE` it
  names (`FRIEND`, `GODP`, `NGHBR`, …, else `OTHER`) with a `PHRASE`,
  `FONE`/`ROMN` `TRAN` with a language, language names BCP 47 tags, file
  formats media types and file paths URIs, `RIN`/`AFN`/`RFN` `EXID`s of the
  registered types, escapes in text calendar names, `AGE STILLBORN` a death
  at `0y`; `SUBN`, `HEAD.FILE`, `HEAD.CHAR` and `GEDC.FORM` are left out; a
  5.5.1 record named `@VOID@` is renamed with its pointers; extension tags
  of known meaning are declared in `HEAD.SCHMA`. Each is reversed when
  converting to 5.5.1. `ConversionOptions::keep_obsolete` (writer
  `conversion`, CLI `--keep-obsolete`) keeps what 7.x drops as declared
  extensions, restored in 5.5.1.
- `GedzipError::ReservedName`.
- GEDCOM 7.0.18, the latest 7.0 release (FamilySearch/GEDCOM v7.0.17 and
  v7.0.18, February 2026): the 7.0 tables come from the release, and its
  normative changes are implemented. The `URI`, `TagDef`, `Latitude` and
  `Longitude` data types are validated by their grammars (`EXID.TYPE` an
  RFC 3986 URI reference, relative ones included; `SCHMA.TAG` an extension
  tag and a URI reference; at most two digits of latitude degrees, the
  longitude ABNF as 7.0.18 corrected it); a file path is a WHATWG valid
  URL string whose non-ASCII characters are URL code points. The pointer
  rules between records are enforced: no `ALIA` to its own record, no
  cycle of pointers between source and multimedia records or source and
  shared note records (`DeviationKind::Cycle`, repaired as a plain
  extension structure keeping its pointer: `RepairKind::Cycle`, by the
  in-memory and the streaming writer alike). The repair writes a 7.x file
  path that is no URL string percent-encoded (a local path as a `file`
  URL) and a coordinate in its canonical form instead of making them
  extensions. `GedzipWriter::add_media_file` takes a `FILE` payload and
  stores the media under its decoded name, GEDZIP names not being
  percent-escaped. GEDCOM 7.1 remains a draft target, generated from
  FamilySearch's `v7.1` branch (`c451779`, its head).

### Changed

- A dataset keeps every structure but the header as a row of the table of
  its type, its texts as 8-byte spans of the store and its pointers as
  interned ids: reading allocates no structure on its own, and a dataset
  takes about twice the size of its input (2.8 times before). Its
  accessors give views: `View<'_, T>` (a `Copy` handle with a method per
  field, detail fields included: `person.names()`, `event.date()`,
  `person.refns()`), `Items<'_, T>` (a list: `len`, `get`, `first`,
  `last`, iteration) and `TextRef` (a text that reads, compares with a
  `&str` and prints without the store). `individuals()` and the other
  accessors return `Items`; `find*`, `families_as_*`, `parents`,
  `children`, `spouse`, `search_individuals` and `get` return views;
  `RecordRef`'s variants are views (`HeaderRef` for the header);
  `View::into_owned` and `RecordRef::into_owned` copy a structure out as its
  owned type, which `add` and `push` store. `get_mut` lends a copy, stored
  back as new rows when the loan ends (the rows it replaces are reclaimed
  once they outnumber the others); a guard forgotten with `mem::forget`
  changes nothing. `StreamedRecord` is a dataset of one record
  (`dataset()`, `record()`, `into_parts()`), and
  `GedcomStreamWriter::write_record` takes the record's view alone.
- A dataset's records are read through accessors (`individuals()`,
  `families()`, …, `extra()`, `header()`, `all::<T>()`) and changed
  through its editing methods, which keep its references whole; its record
  lists are no longer public fields.
- `ThinVec` gains `remove`, `retain` and `retain_mut`.
- A dataset whose identifiers are valid and unique is written without a
  look at every record's identifier: its index vouches for them.
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
- Writing is idempotent: writing what was read back from written output
  gives the same bytes, in every version.
- A 5.5.1 individual's `EVEN` keeps its descriptor as its payload (an
  erratum of the specification's grammar line, p. 34 against p. 48).
- The writer no longer writes `FORM`/`CHAR` in 7.0, `CONC` in 7.0, lines
  over 255 characters in 5.5.1, lower-case 7.0 enumeration values, raw
  control characters, duplicate or `@VOID@` record identifiers, text as
  pointers, or a value whose line breaks inject records; the last line is
  terminated.
- The date, age and time grammars no longer allocate to check a value.
- No panic or hang from any input (Hebrew dates, levels over 255, the
  `Display` of notes, version detection).
- A dual year converted to 7.x takes the later year only for a date from
  January to March (`30 JAN 1648/49` is `30 JAN 1649`), keeps the year
  written otherwise and before the common era, and converts back to the
  5.5.1 date it came from.
- A 5.5.1 note over 32K is split at line breaks into consecutive `NOTE`
  records, so that every record it is moved out of fits.
- GEDZIP: the dataset is always written as GEDCOM 7.x; `add_media_file`
  refuses `gedcom.ged`, `META-INF/…` and `MANIFEST.MF`; directories are not
  listed as media; `find_entry` prefers an exact, then a case-sensitive
  decoded name.
