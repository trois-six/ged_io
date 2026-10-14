# Migrating from ged_io 0.17

oxidged is a fork of [ged_io](https://github.com/ge3224/ged_io) 0.17.0 with a
new model and new reading and writing pipelines. This guide maps the names
and types of ged_io 0.17 to oxidged's. For the differences between GEDCOM
5.5.1 and 7.0 themselves, see [docs/gedcom-7.md](docs/gedcom-7.md).

The examples marked "ged_io 0.17" are not compiled; the oxidged examples are
compiled and run as documentation tests.

## Package, features and command line

| ged_io 0.17 | oxidged | Notes |
|---|---|---|
| crate `ged_io` | crate `oxidged` | `use ged_io::…` becomes `use oxidged::…` |
| feature `json` (`serde` and `serde_json`) | feature `serde` | add `serde_json` to your own dependencies |
| feature `gedzip` | feature `gedzip` | |
| feature `calendar` (with `chrono`) | feature `calendar` | `chrono` is no longer a dependency |
| binary `ged_io`, built by the library package | binary `oxidged`, package `oxidged-cli` | `cargo install oxidged-cli`; the options of 0.17 (`--individual`, `--individual-lastname`, `--individual-firstname`, `--validate`, `--validation-level`) keep their names |
| no `rust-version` | MSRV 1.88 | |

## Reading

| ged_io 0.17 | oxidged | Notes |
|---|---|---|
| `Gedcom::new(text.chars())?.parse_data()?` | `Dataset::parse(text)` or `GedcomBuilder::new().build_from_str(text)?` | `Dataset::parse` returns a `Dataset` (reading never fails) and interprets no producer conventions; the builder interprets them without loss by default |
| `GedcomData` | `Dataset` | |
| `GedcomBuilder` (consumed by `build*`) | `GedcomBuilder` (`build*` take `&self`; `Clone`) | |
| `.build(chars)` | `.build_from_str(text)` | |
| `.build_from_str(&str)`, `.build_from_bytes(&[u8])` | `.build_from_str(impl Into<String>)`, `.build_from_bytes(impl Into<Vec<u8>>)` | an owned `String` or UTF-8 `Vec<u8>` is kept without a copy |
| `.build_from_bytes_with_encoding` | same | |
| `.build_from_gedzip(&[u8])` | `.build_from_gedzip(impl Read + Seek)` | `std::io::Cursor::new(bytes)` or a `File` |
| — | `.build_from_reader`, `.build_from_*_with_report` (`ReadReport`) | |
| `ParserConfig`, `.config()` | removed | getters `is_strict()`, `file_size_limit()`, `interpretation_level()` |
| `strict_mode` | `.strict(true)` | refuses non-conformant input with `GedcomError::NonConformant` (every deviation) |
| `validate_references` | `Dataset::dangling_references()`, or `.strict(true)` | |
| `max_file_size` | `.max_file_size` | error `GedcomError::FileTooLarge { size, max }` |
| `ignore_unknown_tags`, `encoding_detection`, `date_validation`, `preserve_formatting` | removed | unknown tags are always kept (`extra`), encodings always detected, dates read on demand (`oxidged::value`) |
| — | `.interpretation(Interpretation)`, `.profile(Some(profile))` | producer profiles (`oxidged::profile`) |

### Errors

| ged_io 0.17 | oxidged |
|---|---|
| `GedcomError::{ParseError, InvalidFormat, EncodingError, InvalidTag, UnexpectedLevel, MissingRequiredValue, InvalidValueFormat}` | removed: reading does not fail on content |
| `GedcomError::FileSizeLimitExceeded { size, max_size }` | `GedcomError::FileTooLarge { size, max }` |
| `GedcomError::IoError(String)` | `GedcomError::Io(std::io::Error)` |
| — | `GedcomError::NonConformant(Box<[Deviation]>)`, `GedcomError::Gedzip(GedzipError)` (feature `gedzip`) |

`GedcomError` is `#[non_exhaustive]`.

## Reading the data

ged_io 0.17 gave owned structs with public fields; oxidged's dataset keeps
every structure as a row of a table and gives *views*:

- `View<'_, T>` is a `Copy` handle with a method per field
  (`person.names()`, `event.date()`); `Items<'_, T>` a list (`len`,
  `get`, `first`, `last`, `iter`); a text is a `TextRef` (`Display`,
  `to_str()`, compares with `&str`).
- Identifiers and pointers are `XrefId`s: `data.store().xref(id)` is
  their text. Lookups take the text or the id.
- `view.into_owned()` copies a structure out as its owned type
  (`Individual`, `Event`, …); owned types build records, which
  `Dataset::add` stores (checked) and `Dataset::push` stores as reading
  does.
- The record lists are not public fields: records are changed with
  `add`, `remove`, `rename`, `link`/`unlink`, `get_mut`, … (README,
  "Editing").

ged_io 0.17:

```rust,ignore
use ged_io::GedcomBuilder;

let data = GedcomBuilder::new().build_from_bytes(&std::fs::read("family.ged")?)?;
for person in &data.individuals {
    let xref = person.xref.as_deref().unwrap_or("?");
    let name = person.full_name().unwrap_or_default();
    println!("{xref} {name} {}", person.birth_date().unwrap_or(""));
}
```

oxidged:

```rust
use oxidged::GedcomBuilder;

fn main() -> Result<(), oxidged::GedcomError> {
    // From a file: GedcomBuilder::new().build_from_reader(std::fs::File::open("family.ged")?)?
    let text = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n1 CHAR UTF-8\n\
                0 @I1@ INDI\n1 NAME Ann /Example/\n1 BIRT\n2 DATE 1 JAN 1900\n\
                0 @I2@ INDI\n1 NAME Bob /Example/\n0 TRLR\n";
    let data = GedcomBuilder::new().build_from_str(text)?;

    let mut lines = Vec::new();
    for person in data.individuals() {
        let xref = person.xref().map_or("?", |id| data.store().xref(id));
        let name = person.full_name().unwrap_or_default();
        let born = person.birth().and_then(|b| b.date()).map(|d| d.value().to_string());
        lines.push(format!("{xref} {name} {}", born.unwrap_or_default()));
    }
    assert_eq!(lines, ["@I1@ Ann Example 1 JAN 1900", "@I2@ Bob Example "]);
    Ok(())
}
```

### `GedcomData` and `Dataset`

| `GedcomData` (0.17) | `Dataset` (oxidged) | Notes |
|---|---|---|
| `header` | `header()` | |
| `individuals`, `families`, `sources`, `repositories`, `multimedia`, `submitters`, `submissions` | methods of the same names, returning `Items` | |
| `shared_notes` | `notes()` | 5.5.1 `NOTE` and 7.x `SNOTE` records |
| `custom_data` | `extra()` (`Node`s) | |
| `add_individual(i)`, … | `add(i)?` or `push(Record::…)` | |
| `find_individual`, `find_family`, `find_source`, `find_repository`, `find_multimedia`, `find_submitter` | same names, returning views, in constant time | also `find(xref)` for any record |
| `find_shared_note` | `find_note` | |
| `get_families_as_spouse`, `get_families_as_child` | `families_as_spouse`, `families_as_child` (iterators) | |
| `get_children(&family)`, `get_parents(&family)`, `get_spouse(xref, &family)` | `children(family)`, `parents(family)`, `spouse(xref, family)` | |
| `search_individuals_by_name` | `search_individuals` | |
| `total_records()` | `record_count()` | counts the header and `extra` too |
| `gedcom_version()` | `declared_version()`; `version()` | |
| `is_gedcom_7()`, `is_gedcom_5()` | `version().is_v7()`, `version().is_v5()` | |
| `get_individuals_with_event`, `resolve_note`, `is_empty`, `stats`, `count_source_citations`, `SourceCitationStats` | removed | filter `individuals()` by `events_of(kind)`; `find_note(id)` |

### Individuals and families

| 0.17 | oxidged |
|---|---|
| `xref: Option<String>` | `xref() -> Option<XrefId>` |
| `names: Vec<Name>` | `names()`; `name()` (the first) |
| `full_name()` | `full_name()` |
| `given_name()`, `surname()` | `name()?.given()`, `name()?.surname()` (the `GIVN`/`SURN` pieces); `surname_in_value()` (between slashes) |
| `sex: Option<Gender>`, `is_male()`, `is_female()` | `sex() -> Option<Sex>` (`Female`, `Male`, `Undetermined`, `Nonbinary`, `Unknown(Text)`) |
| `families: Vec<FamilyLink>` | `child_of()` (`FAMC`, `ChildLink`), `spouse_of()` (`FAMS`, `SpouseLink`) |
| `events`, `attributes` | `events()`; `events_of(EventKind)` |
| `birth()`, `death()` | same, returning `Option<View<Event>>` |
| `birth_date()`, `birth_place()`, … | `birth()?.date()?.value()`, `birth()?.place()?.name()` |
| `source`, `notes`, `multimedia` | `citations()`, `notes()`, `multimedia()` |
| `change_date` | `change()` |
| `lds_ordinances`, `associations`, `non_events` | `ordinances()`, `associations()`, `non_events()` |
| `user_reference_number` | `refns()` |
| `automated_record_id` | `record_id()` |
| `custom_data` | `extra()` (on every view) |
| `Family::individual1`, `individual2` | `husband()`, `wife()` (`husband_id()`, `wife_id()`) |
| `Family::children: Vec<String>` | `children()` (`IndividualRef`s) |
| `Family::num_children` | an `NCHI` event |

### Types

`ged_io::types::…` becomes `oxidged::model::…` unless stated.

| ged_io 0.17 | oxidged |
|---|---|
| `GedcomData` | `Dataset` |
| `Individual`, `Family`, `Source`, `Repository`, `Multimedia`, `Submitter`, `Submission`, `SharedNote` | same names (rarely used fields in their `detail`) |
| `custom::UserDefinedTag` | `Node` |
| `event::Event` (the kinds) | `EventKind` |
| `event::detail::Detail`, `individual::attribute::AttributeDetail` | `Event` |
| `event::util::HasEvents` | removed |
| `individual::name::Name`, `NameType` | `Name`, `NameType` (pieces as `NamePiece`) |
| `individual::gender::{Gender, GenderType}` | `Sex` |
| `individual::family_link::FamilyLink` | `ChildLink`, `SpouseLink` |
| `Pedigree`, `ChildLinkStatus`, `AdoptedByWhichParent` | `Pedigree`, `ChildStatus`, `Adoption` |
| `note::Note` | `Note` (`NoteContent::Text` or `NoteContent::Shared`) |
| `date::Date`, `date::change_date::ChangeDate`, `gedcom7::CreationDate` | `Date`, `ChangeDate`, `CreationDate` |
| `date::{Calendar, DateValue}` | `oxidged::value::{Calendar, DateValue}` |
| `age::Age` | `Age`, read by `oxidged::value::AgeValue` |
| `place::Place`, `MapCoordinates` | `Place`, `Map` |
| `source::citation::Citation`, `source::citation::data::SourceCitationData` | `Citation`, `CitationData` |
| `source::data::Data`, `source::quay::CertaintyAssessment` | `SourceData`, `Certainty` |
| `repository::citation::Citation` | `RepositoryCitation` |
| `multimedia::file::Reference`, `multimedia::format::Format`, `multimedia::link::Link` | `File`, `FileForm`, `MultimediaLink` |
| `lds::{LdsOrdinance, LdsOrdinanceType, LdsOrdinanceStatus}` | `Ordinance`, `OrdinanceKind`, `OrdinanceStatus` |
| `gedcom7::{Crop, NonEvent}` | `Crop`, `NonEvent` |
| `header::Header`, `header::source::HeadSour`, `header::meta::HeadMeta`, `header::encoding::Encoding` | `Header`, `HeaderSource`, `GedcomInfo`, `Charset` |
| `header::schema::Schema` | `Schema` (`tags`) |

## Writing

| ged_io 0.17 | oxidged | Notes |
|---|---|---|
| `GedcomWriter::gedcom_version("7.0")` | `.gedcom_version(GedcomVersion::V7_0)` | the dataset is converted between 5.5.1 and 7.x, and repaired for the target |
| `.line_ending("\r\n")` | `.line_ending(LineEnding::CrLf)` | |
| `write_to_string(&data) -> Result<String, io::Error>` | `write_to_string(&data) -> Result<String, WriteError>`; `write_to_string_with_report` | every repair in a `WriteReport` |
| `write_to(&mut impl fmt::Write, &data)` | `write(impl io::Write, &data) -> Result<WriteReport, WriteError>` | |
| `WriterConfig { gedcom_version: String, line_ending: String, max_line_length, include_empty_fields }` | `WriterConfig { version, line_ending, max_line_length, bom, output_encoding, on_nonconformant, conversion, credentials }` | `include_empty_fields` is gone |
| — | `GedcomStreamWriter`, `write_tree`, `write_structures` | |

```rust
use oxidged::model::{Dataset, Individual, Name, Sex};
use oxidged::{GedcomVersion, GedcomWriter};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A file read as 5.5.1, written as 7.0.
    let data = Dataset::parse(
        "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n1 CHAR UTF-8\n0 @I1@ INDI\n1 NAME Ann /Example/\n0 TRLR\n",
    );
    let writer = GedcomWriter::new().gedcom_version(GedcomVersion::V7_0);
    let (out, report) = writer.write_to_string_with_report(&data)?;
    assert!(out.contains("2 VERS 7.0\n"));
    assert!(!out.contains("CHAR"));
    println!("{} repairs", report.repairs.len());

    // A dataset built from owned types.
    let mut built = Dataset::new(GedcomVersion::V7_0);
    built.add(Individual {
        names: vec![Name::new("Ann /Example/")],
        sex: Some(Sex::Female),
        ..Individual::default()
    })?;
    let mut bytes = Vec::new(); // or std::fs::File::create("family.ged")?
    writer.write(&mut bytes, &built)?;
    let text = String::from_utf8(bytes)?;
    assert!(text.contains("0 @I1@ INDI\n1 NAME Ann /Example/\n1 SEX F\n"));
    Ok(())
}
```

## Streaming and indexing

| ged_io 0.17 | oxidged |
|---|---|
| `GedcomStreamParser::new(reader)`, items `GedcomRecord` | same constructor, items `StreamedRecord`; `record()` is a `RecordRef` (`Header`, `Individual`, `Family`, …, `Note`, `Other`) |
| `GedcomRecord::as_*`, `into_*` | `if let RecordRef::Individual(view) = record.record()`; `into_parts()` |
| `FromIterator<GedcomRecord> for GedcomData` | `FromIterator<StreamedRecord>` and `Extend<StreamedRecord>` for `Dataset` |
| — | `nodes()` (lossless structures), `visit(&mut impl Visitor)` |
| `indexed::IndexedGedcomData` | `IndexedDataset` (`new`, `data`, `data_mut`, `into_inner`, `families_as_spouse`, `families_as_child`; the dataset's lookups through `Deref`) |
| `IndexStats`, `index_stats()` | removed |

```rust
use oxidged::model::{Dataset, RecordRef};
use oxidged::GedcomStreamParser;

fn main() -> Result<(), oxidged::GedcomError> {
    // A file: GedcomStreamParser::new(std::io::BufReader::new(std::fs::File::open("big.ged")?))?
    let file: &[u8] = b"0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 NAME Ann /Example/\n\
                        0 @F1@ FAM\n1 WIFE @I1@\n0 TRLR\n";
    let mut names = Vec::new();
    for record in GedcomStreamParser::new(file)? {
        let record = record?;
        if let RecordRef::Individual(person) = record.record() {
            names.extend(person.full_name());
        }
    }
    assert_eq!(names, ["Ann Example"]);

    // Keep some records, reading one at a time.
    let people: Dataset = GedcomStreamParser::new(file)?
        .filter_map(Result::ok)
        .filter(|r| matches!(r.record(), RecordRef::Header(_) | RecordRef::Individual(_)))
        .collect();
    assert_eq!((people.individuals().len(), people.families().len()), (1, 0));
    Ok(())
}
```

## Encodings, versions and GEDZIP

| ged_io 0.17 | oxidged |
|---|---|
| `decode_gedcom_bytes(bytes) -> Result<(String, GedcomEncoding), _>` | `encoding::decode(bytes) -> Decoded { text, encoding, declared }` (never fails) |
| `encoding::decode_with_encoding` | `encoding::decode_as` |
| `detect_encoding` (crate root) | `encoding::detect_encoding` |
| `encoding::encode_to_bytes` | `encoding::encode` (`EncodeError`) |
| `GedcomEncoding::Unknown` | removed; new: `Windows1252`, `Windows1250`, `Windows1251`, `Iso8859_2`, `Cp437`, `MacRoman` |
| `detect_version` (crate root) | `version::detect_version` |
| `GedcomVersion::{V5_5_1, V7_0, Unknown(_)}` | `GedcomVersion::{V5_5_1, V7_0, V7_1}` (`Copy`); other declarations read as 5.5.1 |
| `VersionFeatures`, `supports_conc()`, `doubles_all_at_signs()`, `supports_char_encoding()` | `VersionRules` (`version.rules()`): `uses_conc()`, `doubles_every_at_sign()`, `has_head_char()` |
| `version::appears_to_be_v7` | `detect_version(text).is_v7()` |
| `gedzip::read_gedzip(bytes) -> Result<GedcomData, GedzipError>` | `read_gedzip(bytes) -> Result<Dataset, GedcomError>` |
| `write_gedzip`, `write_gedzip_with_media` | same names, taking a `Dataset`; written as GEDCOM 7.x |
| `GedzipReader::parse_gedcom()` | `read_dataset(&GedcomBuilder)` |
| `GedzipWriter::write_gedcom` | `write_dataset`, `write_dataset_with(&data, &writer)` |
| `GedzipError::{ZipError, MissingGedcomFile, MissingMediaFile, IoError, …}` | `GedzipError::{Zip, MissingGedcom, MissingMedia, Io, EntryTooLarge, Write, ReservedName}` |

## Removed modules

| ged_io 0.17 | oxidged |
|---|---|
| `tokenizer`, `parser` (`Tokenizer`, `StreamTokenizer`, `Parser`, `StreamParser`, `parse_subset`) | `oxidged::tree` (`Tree`, `parse_tree`, `TreeReader`, `Structure`) and `GedcomStreamParser::nodes()` |
| `display` (`Display` of the records) | removed; `TextRef`, `Tree`, `Tag` and `Xref` implement `Display` |
| `debug` (`ImprovedDebug`) | removed; `Debug` of a view prints its structure |
| `util` (`escape_at_signs`, …) | removed; the reader unescapes and the writer escapes by the version's rules |

## JSON

With the `serde` feature (and `serde_json` in your dependencies), a
`Dataset` serializes and deserializes; the model's structs and views do
not on their own. The shape follows the new model: `version`,
`declared_version`, `header`, then the non-empty lists (`submitters`,
`submissions`, `individuals`, `families`, `notes`, `sources`,
`repositories`, `multimedia`, `extra`); empty fields are left out, texts
and identifiers are strings, enumeration values are written by name
(`"Female"`) or as written (`{"Unknown": "x"}`). ged_io 0.17 wrote every
field of `GedcomData`, `null` and `[]` included.
