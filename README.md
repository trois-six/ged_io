# ged_io

**Read any GEDCOM file, write conformant GEDCOM 5.5.1, 7.0 and 7.1**

[![Crates.io](https://img.shields.io/crates/v/ged_io.svg)](https://crates.io/crates/ged_io)
[![Documentation](https://docs.rs/ged_io/badge.svg)](https://docs.rs/ged_io)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

---

## What is ged_io?

`ged_io` is a Rust library for reading and writing
[GEDCOM](https://en.wikipedia.org/wiki/GEDCOM) files, the standard for
exchanging genealogical data between family tree software.

It reads real-world files as they are — any encoding, any line
terminators, extensions, misplaced or repeated structures, values outside
the specification — and keeps all their data, with no warning and no
failure. It writes strictly conformant GEDCOM 5.5.1, 7.0 or 7.1, whatever
the data holds, repairing what the target version does not permit and
reporting each repair.

### Key Features

| Feature | Description |
|---------|-------------|
| **Three versions** | GEDCOM 5.5.1, 7.0 and 7.1, read and written; every structure of each has a type |
| **Conversion** | 5.5.1 to 7.x and back, following the FamilySearch migration guide: multimedia and source records, roles, translations, languages, media types and file URIs, identifiers, dates and ages |
| **Read & write** | A lossless, lenient reader, and a writer conformant by construction; data that does not fit a type is kept, in place, and written back |
| **Streaming** | Read and write one record at a time, in any encoding and with any line terminators, memory bounded by the largest record |
| **Indexed lookups** | Records by identifier and families from individuals in constant time |
| **GEDZIP** | Read and write `.gdz` archives bundling a GEDCOM 7.x dataset with its media files, converting 5.5.1 (feature `gedzip`) |
| **Encodings** | UTF-8, UTF-16 LE/BE (with or without BOM), ANSEL (to Unicode NFC), ASCII, Windows-1252 (`ANSI`), ISO-8859-1, ISO-8859-15, IBM PC (cp437), Macintosh; decoding never fails; in memory and streaming |
| **JSON** | `Serialize` and `Deserialize` for the dataset with serde, for JSON and other self-describing formats (feature `serde`) |
| **Type safe** | A Rust type per structure, enumerations whose unknown values keep their text, typed identifiers and pointers |
| **Validation** | The 5.5.1, 7.0 and 7.1 specification tables: every deviation of a file with its line, an opt-in strict mode, and a repair pass that makes any tree conformant without losing data |
| **Values** | Date, age and time grammars of both versions, conversions between versions, and calendar arithmetic (feature `calendar`) |
| **Fast** | 1.3 to 4 times as fast as 0.17 to read and up to 4 times as fast to write, at less than three times the input's size in memory (see [Performance](#performance)) |

---

## Installation

```toml
[dependencies]
ged_io = "0.18"
```

### Optional Features

```toml
# Serialize and deserialize the dataset with serde (JSON and others)
ged_io = { version = "0.18", features = ["serde"] }

# GEDZIP archive support (.gdz files)
ged_io = { version = "0.18", features = ["gedzip"] }

# Calendar arithmetic: conversions between calendars, day numbers
ged_io = { version = "0.18", features = ["calendar"] }
```

---

## Quick Start

### Read a file

```rust,no_run
use ged_io::GedcomBuilder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Any encoding: the bytes are decoded by what they hold and their CHAR.
    let data = GedcomBuilder::new().build_from_reader(std::fs::File::open("family.ged")?)?;

    println!("GEDCOM {}", data.version());
    for person in data.individuals() {
        let name = person.full_name(&data).unwrap_or_default();
        let born = person
            .birth()
            .and_then(|b| b.date.as_ref())
            .map(|d| d.value.to_str(&data).into_owned())
            .unwrap_or_default();
        println!("{name} {born}");
    }
    Ok(())
}
```

A dataset keeps the text it was read from, and its model points into it:
a text is read back with `text.to_str(&data)`, an identifier with
`data.store().xref(id)`. Fields that few structures use are in a boxed
*detail* (`person.detail().refns`, `event.detail().age`).

### Write a file

```rust,no_run
use ged_io::{Dataset, GedcomVersion, GedcomWriter};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data = Dataset::from_bytes(std::fs::read("input.ged")?);

    // In the version the data declares…
    std::fs::write("output.ged", GedcomWriter::new().write_to_string(&data)?)?;

    // …or as GEDCOM 7.0, straight into a file, with the repairs it made.
    let file = std::io::BufWriter::new(std::fs::File::create("output70.ged")?);
    let report = GedcomWriter::new()
        .gedcom_version(GedcomVersion::V7_0)
        .write(file, &data)?;
    for repair in &report.repairs {
        println!("{repair}");
    }
    Ok(())
}
```

---

## Reading

Reading never fails and loses nothing. Bytes of any encoding are decoded
(a byte order mark, then a UTF-16 NUL pattern, then valid UTF-8 decide,
before the `HEAD.CHAR` declaration), and every line is read by fixed,
silent rules:

- CR, LF, CR LF and LF CR terminators, mixed or not; blank lines; tabs or
  several spaces between the parts of a line; leading zeros in levels;
  level jumps (the line nests in the deepest open structure of its record);
- `CONT` and `CONC` under any tag, at any level, also after substructures;
- identifiers on substructures, missing `HEAD` or `TRLR`, content after
  `TRLR`;
- extension and unknown tags, under their real parent at their real level.

Each record is then typed as far as it fits the model. What a type has no
field for — an extension, an unknown tag, a second occurrence of a
structure that occurs once, a structure of another shape — is kept in its
`extra`, in order, where it was, and written back. An enumeration value no
variant names is kept as written (`Pedigree::Unknown("stepchild")`), as is
any date, age or time: their grammars read them on demand.

```rust
use ged_io::model::{Dataset, Pedigree, Sex};

let data = Dataset::parse(
    "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NAME Ann /Example/\n1 SEX f\n\
     1 FAMC @F1@\n2 PEDI stepchild\n1 _HOBBY Weaving\n0 @F1@ FAM\n0 TRLR\n",
);
let ann = &data.individuals()[0];
assert_eq!(ann.sex, Some(Sex::Female)); // 5.5.1 values ignore case
let pedigree = &ann.child_of[0].detail().pedigree.as_ref().unwrap().value;
assert!(matches!(pedigree, Pedigree::Unknown(t) if t.to_str(&data) == "stepchild"));
assert_eq!(data.store().tag(ann.extra[0].tag), "_HOBBY"); // kept, in place
```

### Builder options and strict mode

```rust
use ged_io::{GedcomBuilder, GedcomError};

let text = "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 SEX male\n0 TRLR\n";
let builder = GedcomBuilder::new().max_file_size(50_000_000);
assert!(builder.build_from_str(text).is_ok()); // lenient: read, value kept

match builder.clone().strict(true).build_from_str(text) {
    Err(GedcomError::NonConformant(deviations)) => {
        // line 5: SEX "male": not a value of enumset-SEX
        println!("{}", deviations[0]);
    }
    _ => unreachable!(),
}
```

| Option | Default | Description |
|--------|---------|-------------|
| `strict(bool)` | `false` | Refuse input that does not follow the specification of the version it declares, with every deviation (encoding, lines, structures, cardinalities, payloads, enumerations, pointers, header) |
| `max_file_size(bytes)` | none | Refuse input larger than this before reading it (for GEDZIP, its `gedcom.ged` as it is decompressed) |

Inputs: `build_from_str`, `build_from_bytes` (a `Vec<u8>` of UTF-8 is not
copied), `build_from_bytes_with_encoding`, `build_from_reader` and
`build_from_gedzip`. `Dataset::parse` and `Dataset::from_bytes` read
without options. Reading fails only on I/O, a size limit, strict mode or
a GEDZIP container (`GedcomError`).

---

## The dataset

| Type | Description |
|------|-------------|
| `Dataset` | The header and the records by type, in file order, and `extra`: records of no type the model has |
| `Individual`, `Family`, `Source`, `Repository`, `Multimedia`, `Submitter`, `Submission`, `SharedNote` | The records (`INDI`, `FAM`, `SOUR`, `REPO`, `OBJE`, `SUBM`, 5.5.1 `SUBN`, 5.5.1 `NOTE` and 7.x `SNOTE`) |
| `Event`, `Name`, `ChildLink`, `SpouseLink`, `IndividualRef`, `Association`, `Citation`, `Note`, `Place`, `Date`, `Age`, `Ordinance`, … | Every substructure of 5.5.1, 7.0 and 7.1 |
| `Text`, `XrefId`, `Store` | Texts, interned identifiers and the store they resolve against |
| `Node` | A structure kept as read (an extension, an unknown tag) |

Lookups and navigation:

```rust
use ged_io::Dataset;

let data = Dataset::parse(
    "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 NAME Ann /Example/\n0 @I2@ INDI\n\
     1 NAME Bob /Example/\n0 @F1@ FAM\n1 HUSB @I2@\n1 WIFE @I1@\n1 FAMC @F9@\n0 TRLR\n",
);
let ann = data.find_individual("@I1@").unwrap();
let family = data.families_as_spouse(ann.xref).next().unwrap();
let bob = data.spouse(ann.xref, family).unwrap();
assert_eq!(bob.full_name(&data).as_deref(), Some("Bob Example"));
assert_eq!(data.search_individuals("example").count(), 2);
// Pointers to no record are a query, not an error.
assert_eq!(data.dangling_references().len(), 1);
```

`find`, `find_individual`, `find_family`, `find_source`,
`find_repository`, `find_multimedia`, `find_submitter` and `find_note`
take an identifier as text or as an `XrefId`; when several records have
one, the first is found. They search linearly; `IndexedDataset` finds
records and families in constant time:

```rust
use ged_io::{Dataset, IndexedDataset};

let indexed = IndexedDataset::new(Dataset::parse("0 HEAD\n0 @I1@ INDI\n0 TRLR\n"));
assert!(indexed.find_individual("@I1@").is_some());
```

---

## Writing

The writer emits conformant lines in GEDCOM 5.5.1, 7.0 or 7.1, whatever
the data holds:

| Rule | 5.5.1 | 7.0 and 7.1 |
|---|---|---|
| Line length | At most 255 characters, the whole line counted; longer payloads continue with `CONC`, never split at a space or before a combining mark (text with no such place has a space written as a line break, `CONT`) | No limit, no `CONC` |
| Line breaks in text (LF, CR LF, CR) | `CONT` lines | `CONT` lines |
| `@` in text | Every `@` doubled; `@#…@` escapes kept | A leading `@` doubled |
| Identifiers | Valid (at most 22 characters) and unique | `@[A-Z0-9_]+@`, unique, never `@VOID@` |
| Header | `GEDC.VERS`, `GEDC.FORM LINEAGE-LINKED`, `CHAR` naming the output encoding, `SOUR`, `SUBM` | `GEDC.VERS` only, no `FORM` or `CHAR` |
| Last line | Terminated | Terminated |

Before the lines, the structures are repaired for the target version
(`ged_io::spec::conform`): an enumeration value outside its set, text
where a pointer belongs, a date outside the grammar, a structure the
version does not permit or permits once, a missing required substructure.
Nothing is dropped: data with no standard form becomes an extension
structure (`_TAG`), or 7.x `OTHER`/`@VOID@` with a `PHRASE`. Dates, ages and
times are written in the target's grammar (`@#DJULIAN@` and `JULIAN`,
`B.C.` and `BCE`, `INT … (…)` and `PHRASE`). Each repair is reported as a
`Repair`; `.on_nonconformant(RepairPolicy::Error)` makes the writer fail
instead. `.output_encoding(OutputEncoding::Ansel)` (or `Utf16Le`,
`Ascii`) writes 5.5.1 bytes in another encoding, with a matching `CHAR`;
`.line_ending(…)`, `.max_line_length(…)` and `.bom(…)` set the rest.

### Conversion between 5.5.1 and 7.x

Written in a version of the other family, a dataset is converted first
(`ged_io::convert`), as the FamilySearch migration guide says and, where it
leaves a choice, as the gedcom7code conversions do:

| 5.5.1 | 7.x |
|---|---|
| `NOTE` record, `NOTE @N1@` | `SNOTE` record, `SNOTE @N1@` |
| Multimedia link holding its files | A new `OBJE` record; the link points to it and keeps its title |
| Source citation in words (`SOUR text`) | A new `SOUR` record with the text as its `NOTE`; the citation's `TEXT`s under `DATA` |
| `ASSO.RELA Godfather` | `ASSO.ROLE GODP` + `PHRASE Godfather`; a word no role names is `OTHER` + `PHRASE` |
| `FONE`/`ROMN` with `TYPE` | `TRAN` with `LANG` (`kana` is `ja-Hrkt`, `romaji` `ja-Latn`, …) |
| `LANG English` | `LANG en` (an unknown name `und` + its phrase) |
| `FILE C:\photos\a b.jpg`, `FORM jpg`, `FORM.TYPE photo` | `FILE file:///C:/photos/a%20b.jpg`, `FORM image/jpeg`, `FORM.MEDI PHOTO` |
| `RIN`, `AFN`, `RFN system:id`, `_FSFTID` | `EXID` with its registered `TYPE` |
| `@#DJULIAN@` in text, `30 JAN 1648/49`, `AGE STILLBORN` | `JULIAN`, `30 JAN 1649` + `PHRASE`, `AGE 0y` + `PHRASE` (and a death at that age) |
| `SUBN`, `HEAD.FILE`, `HEAD.CHAR`, `GEDC.FORM`; a record named `@VOID@` | Left out; renamed, with its pointers |
| Extension tags of known meaning (`_LOC`, `_GOV`, …) | Declared in `HEAD.SCHMA` |

Converting to 5.5.1 reverses each of these where 5.5.1 can say it; what it
has no place for is kept as extension structures. With
`.conversion(ConversionOptions::default().keep_obsolete(true))`, the 5.5.1
structures 7.x drops are kept as extensions `HEAD.SCHMA` declares with their
5.5.1 URIs, and restored when converting back, so that 5.5.1 data survives a
round trip through 7.x.

```rust
use ged_io::convert::{convert_dataset, ConversionOptions};
use ged_io::{Dataset, GedcomVersion, GedcomWriter};

let data = Dataset::parse(
    "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 OBJE\n2 FILE photo.jpg\n3 FORM jpg\n\
     2 TITL Portrait\n0 TRLR\n",
);
let out = GedcomWriter::new()
    .gedcom_version(GedcomVersion::V7_0)
    .write_to_string(&data)
    .unwrap();
assert!(out.contains("1 OBJE @M1@\n2 TITL Portrait\n"));
assert!(out.contains("0 @M1@ OBJE\n1 FILE photo.jpg\n2 FORM image/jpeg\n"));

// Or as a dataset of the other version.
let v7 = convert_dataset(&data, GedcomVersion::V7_0, &ConversionOptions::default());
assert_eq!(v7.multimedia().len(), 1);
```

---

## Streaming

```rust,no_run
use ged_io::model::RecordRef;
use ged_io::{Dataset, GedcomStreamParser};

fn main() -> Result<(), ged_io::GedcomError> {
    let file = std::io::BufReader::new(std::fs::File::open("huge.ged")?);
    let mut kept = Dataset::default();
    for record in GedcomStreamParser::new(file)? {
        let record = record?;
        if let RecordRef::Individual(person) = record.record() {
            println!("{}", person.full_name(&record).unwrap_or_default());
        }
        // Keep the records of interest, without reading the whole file.
        if matches!(record.record(), RecordRef::Header(_) | RecordRef::Family(_)) {
            kept.extend([record]);
        }
    }
    Ok(())
}
```

The streaming parser reads any encoding and any line terminator the
in-memory reader reads, with the same rules and types, decoding on the fly:
memory stays bounded by the largest record. Each record comes with the
store of its own text. `parser.nodes()` yields the records as lossless
structures instead; for a whole file as structures, nothing interpreted,
use `ged_io::tree::Tree` (in memory) or `ged_io::tree::TreeReader`
(streaming).

`GedcomStreamWriter` writes the other way: a header, then each record as
it is given (from a `GedcomStreamParser`, or a dataset), repaired and
converted for the target version as `GedcomWriter` does, in memory bounded
by one record and the identifiers written:

```rust,no_run
use ged_io::model::{Header, RecordRef};
use ged_io::{GedcomStreamParser, GedcomStreamWriter, GedcomVersion, WriterConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::io::BufReader::new(std::fs::File::open("huge.ged")?);
    let output = std::io::BufWriter::new(std::fs::File::create("huge-7.ged")?);
    let mut records = GedcomStreamParser::new(input)?;
    let head = records.next().ok_or("empty file")??;
    let default = Header::default();
    let header = match head.record() {
        RecordRef::Header(h) => h,
        _ => &default,
    };
    let config = WriterConfig { version: Some(GedcomVersion::V7_0), ..WriterConfig::default() };
    let mut writer = GedcomStreamWriter::new(output, config, header, &head)?;
    for record in records {
        let record = record?;
        writer.write_record(record.record(), &record)?;
    }
    let (_, report) = writer.finish()?;
    eprintln!("{} repairs", report.repairs.len());
    Ok(())
}
```

Knowing one record at a time, it writes what `GedcomWriter` writes of the
same records but where the records to come would change it (a later
record named as an earlier one, the records a conversion adds, the
extension tags a 7.x header declares); its documentation lists each case.

---

## GEDZIP

```rust,no_run
use ged_io::gedzip::{write_gedzip_with_media, GedzipReader};
use ged_io::GedcomBuilder;
use std::collections::HashMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Read the dataset of an archive through the builder (limits, strict mode).
    let data = GedcomBuilder::new().build_from_gedzip(std::fs::File::open("family.gdz")?)?;

    // Extract its media files.
    let mut reader = GedzipReader::new(std::fs::File::open("family.gdz")?)?.max_entry_size(1 << 30);
    for name in reader.media_files().iter().map(|n| n.to_string()).collect::<Vec<_>>() {
        let bytes = reader.read_media_file(&name)?;
        println!("{name}: {} bytes", bytes.len());
    }

    // Write a new archive with media.
    let mut media = HashMap::new();
    media.insert("photos/portrait.jpg".to_string(), std::fs::read("portrait.jpg")?);
    std::fs::write("new.gdz", write_gedzip_with_media(&data, &media)?)?;
    Ok(())
}
```

An archive holds GEDCOM 7.x (§4): a 5.5.1 dataset is converted to 7.0 and
written in UTF-8 with a byte order mark. `add_media_file` refuses the names
GEDZIP reserves (`gedcom.ged`, `META-INF/…`, `MANIFEST.MF`); `media_files`
lists files, not directories; `find_entry` matches a `FILE` reference to its
entry by its exact name first, then decoded (`%20`), then leniently.

---

## JSON

With the `serde` feature, a `Dataset` serializes as a map of its records,
each structure a map of its non-empty fields, texts and identifiers as
strings, enumeration values by name (`"Female"`) or as written
(`{"Unknown": "x"}`); it deserializes back into a dataset.

What may nest to any depth is listed flat, so that the JSON of any dataset
nests a few levels deep and reads back within the nesting limit of
`serde_json` (128): a structure kept untyped (`extra`) lists its
`descendants`, each with its `level` below it, and a list of notes (or of
citations) gives each note and citation held in another after it, as
`[depth, {…}]` (a citation at depth 1 of a list of notes, a note at depth
2, and so on).

```rust
fn main() -> Result<(), serde_json::Error> {
    let data = ged_io::Dataset::parse("0 HEAD\n0 @I1@ INDI\n1 NAME Ann /Example/\n0 TRLR\n");
    let json = serde_json::to_string(&data)?;
    assert_eq!(
        json,
        r#"{"version":"5.5.1","header":{},"individuals":[{"xref":"@I1@","names":[{"value":"Ann /Example/"}]}]}"#
    );
    let back: ged_io::Dataset = serde_json::from_str(&json)?;
    assert_eq!(back.to_structures(), data.to_structures());
    Ok(())
}
```

---

## Validation and Conformance Repair

`ged_io::spec` holds the specifications as data (generated from the
FamilySearch GEDCOM 7.0 and 7.1 tables and a 5.5.1 transcription checked
against the PDF; see `NOTICE`). It validates a file, a parsed tree or owned
structures, and repairs a tree so that it validates:

```rust
use ged_io::spec::{conform, validate, validate_bytes};
use ged_io::tree::parse_tree;
use ged_io::GedcomVersion;

let bytes = b"0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 SEX male\n0 TRLR\n";
for deviation in validate_bytes(bytes) {
    println!("{deviation}"); // line 5: SEX "male": not a value of enumset-SEX
}

let mut records = parse_tree("0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 SEX male\n0 TRLR\n")
    .to_structures();
let repairs = conform(&mut records, GedcomVersion::V7_0); // SEX male becomes _SEX male
assert!(validate(&records, GedcomVersion::V7_0).is_empty());
```

Every rule the validator checks and every repair are listed in the
[`spec` module documentation](https://docs.rs/ged_io/latest/ged_io/spec/).

One deliberate exception to strict conformance: the 5.5.1 file formats
(`FILE.FORM`, `MULTIMEDIA_FORMAT`) are treated as an open list. The
specification lists only `bmp`, `gif`, `jpg`, `ole`, `pcx`, `tif` and `wav`;
`png`, `pdf` or `webp` are accepted and written as given, a media type
(`image/png`) is written as its file format (`png`), and a missing `FORM` is
derived from the file's extension. GEDCOM 7.0 output keeps media types.

---

## Dates, Ages and Times

Dates, ages and times are kept exactly as written, and read on demand by
the grammars of both versions (`ged_io::value`), which never fail: wording
they do not understand is kept verbatim. `parse_strict` checks a payload
against one version's grammar; `Date::to_version` and `Age::to_version`
rewrite a value in the other version's syntax, as the writer does.

```rust
use ged_io::model::Date;
use ged_io::value::{Approximation, Calendar, DateValue};
use ged_io::{Dataset, GedcomVersion};

let store = Dataset::default();
let date = Date::new("ABT @#DJULIAN@ 1700");
let DateValue::Approximated(Approximation::About, d) = date.parse(&store) else { panic!() };
assert_eq!((d.calendar, d.year), (Calendar::Julian, Some(1700)));
assert_eq!(date.to_version(&store, GedcomVersion::V7_0).value.to_str(&store), "ABT JULIAN 1700");
```

- **Exact**: `15 MAR 1950`; **range**: `BET 1900 AND 1910`, `BEF 1900`,
  `AFT 1900`; **period**: `FROM 1900 TO 1910`; **approximate**: `ABT`,
  `CAL`, `EST`
- **Interpreted and phrases** (5.5.1): `INT 1900 (about 1900)`, `(unknown)`
- **Dual years** (5.5.1): `15 APR 1699/00`; **before the common era**:
  `44 B.C.`, `44 BCE`
- **Ages**: `25y 3m`, `< 8y`, `> 1y 400d`, `CHILD`; **times**:
  `12:34:56.789`, `02:50Z`

| Calendar | 5.5.1 | 7.0 | Arithmetic (`calendar` feature) |
|----------|-------|-----|:-------------------------------:|
| Gregorian | `@#DGREGORIAN@` | `GREGORIAN` | ✅ |
| Julian | `@#DJULIAN@` | `JULIAN` | ✅ |
| Hebrew | `@#DHEBREW@` | `HEBREW` | ✅ |
| French Republican | `@#DFRENCH R@` | `FRENCH_R` | ✅ |
| Roman, unknown | `@#DROMAN@`, `@#DUNKNOWN@` | `_ROMAN`, `_UNKNOWN` | - |
| Extension calendars | - | `_MYCAL` | - |

---

## Command Line Tool

```bash
# Install
cargo install ged_io

# A summary: version, records, pointers to no record
ged-io family.ged

# One individual, or every individual whose surname contains EXAMPLE
ged-io --individual @I1@ family.ged
ged-io --individual-lastname example family.ged

# Check a file against the specification of the version it declares
ged-io --validate --validation-level strict family.ged
Validation: strict - errors: 2, warnings: 0
error: line 14: SEX "male": not a value of enumset-SEX
error: line 27: FAMC @F9@: no record has this identifier

# Rewrite any file as conformant GEDCOM 7.0 (repairs on standard error),
# converting a 5.5.1 file; --keep-obsolete keeps what 7.0 drops as extensions
ged-io --write 7.0 family.ged > family70.ged
ged-io --write 7.0 --keep-obsolete family.ged > family70.ged
```

`ged-io --help` lists every option. Exit codes: 0 success, 1 I/O error,
2 validation errors in strict mode, 3 usage error.

---

## Performance

Measured against the published 0.17.0 with `tools/differential` (`perf`:
one crate and one operation per process, release build, on a performance
core of an Intel Core Ultra 7 265H). Small files: CPU cycles per
operation (`perf stat`, steady state over thousands of runs: free of the
clock's changes), instructions in parentheses:

| Input | Read 0.17 | Read 0.18 | Write 0.17 | Write 0.18 |
|-------|----------:|----------:|-----------:|-----------:|
| `tests/fixtures/simple.ged` (47 lines) | 27.0k (115k) | 20.0k (91k) | 10.7k (52k) | 9.4k (45k) |
| `tests/fixtures/sample.ged` (96 lines) | 53.5k (231k) | 34.7k (166k) | 18.8k (94k) | 20.5k (97k) |
| `tests/fixtures/conformance/maximal551.ged` (704 lines) | 500k (1.69M) | 305k (1.26M) | 123k (608k) | 128k (642k) |
| `tests/fixtures/washington.ged` (234 KB) | 8.37M (31.0M) | 4.43M (17.5M) | 2.36M (10.9M) | 2.23M (10.3M) |
| generated, 1,000 individuals (457 KB) | 14.8M (51.4M) | 5.71M (26.0M) | 3.72M (17.8M) | 3.02M (15.5M) |

0.17 writes 653 lines of `maximal551.ged`, 0.18 the 700 it holds; written
in the other version family (`dataset-converted`), 0.18 converts and
repairs what 0.17 writes as it is: a dataset that needs nothing converted
costs no more than its write (generated: 14.6M instructions), one that
does several times more (`washington.ged`, whose individuals carry
`_UID` and `_FSFTID`: 43M).

Large files, wall time (median of 5) and peak resident set (Linux
`VmHWM`), with `tools/perf`:

| Generated input | Read 0.17 | Read 0.18 | Write 0.17 | Write 0.18 |
|-------|----------:|----------:|-----------:|-----------:|
| 20,000 individuals (9.4 MB) | 89 ms (106 MB/s) | 28 ms (333 MB/s) | 45 ms | 13 ms |
| 172,000 individuals (82 MB) | 1.10 s (75 MB/s) | 0.25 s (332 MB/s) | 0.48 s | 0.12 s |

| Peak memory, 82 MB input | 0.17 | 0.18 |
|---|---:|---:|
| In memory (`GedcomBuilder::build_from_bytes`) | 1.9 GB (23× the input) | 231 MB (2.8×) |
| Streaming read (`GedcomStreamParser`) | — (UTF-8 only) | 4.4 MB, any encoding |
| Streaming write (`GedcomStreamParser` into `GedcomStreamWriter`, 140 MB/s) | — | 23 MB |

A dataset keeps the decoded input once and points into it: about 2.6
times the input in all, the rest of the peak being the allocator's. A
written file is checked and repaired for its version as it is written,
one record at a time; the specification tables are built at compile time.
`tools/perf check` runs every pipeline against the limits the CI checks;
Criterion benchmarks: `cargo bench --bench read`, `--bench write`,
`--bench memory`.

---

## Building from Source

```bash
git clone https://github.com/ge3224/ged_io.git
cd ged_io
cargo build --release
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo bench
```

The conformance suite (`tests/conformance`) runs the vendored and
generated cases with every test; `tools/fetch-corpora.sh` fetches the
opt-in corpora for `cargo test --all-features --test conformance --
--ignored`, and `tools/differential` compares what this checkout keeps of
them with the published 0.17.0.

---

## Documentation

- [API Documentation](https://docs.rs/ged_io) - Full API reference
- [MIGRATION.md](MIGRATION.md) - GEDCOM 5.5.1 to 7.0 migration guide
- [ROADMAP.md](ROADMAP.md) - Project roadmap and planned features
- GEDCOM specifications (bundled in this repo):
  - [GEDCOM 7.0 Specification (PDF)](docs/FamilySearchGEDCOMv7.pdf)
  - [GEDCOM 5.5.1 Specification (PDF)](docs/ged551.pdf)

---

## Contributing

Contributions are welcome! Areas where help is appreciated:

- Bug reports and feature requests
- Additional test cases and edge cases
- Documentation improvements
- Performance optimizations

Please feel free to open issues or submit pull requests.

---

## License

This project is licensed under the [MIT License](license.md). The
specification tables derived from FamilySearch/GEDCOM are Apache-2.0; see
[NOTICE](NOTICE).

---

## Acknowledgments

Originally forked from [`pirtleshell/rust-gedcom`](https://github.com/pirtleshell/rust-gedcom).

GEDCOM is a specification maintained by [FamilySearch](https://www.familysearch.org/).
