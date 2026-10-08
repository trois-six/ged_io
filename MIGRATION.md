# Migration Guide: GEDCOM 5.5.1 to GEDCOM 7.0

This guide explains the key differences between GEDCOM 5.5.1 and GEDCOM 7.0, and how to migrate your applications using the `ged_io` library.

## Key Differences

### 1. Encoding

| Feature | GEDCOM 5.5.1 | GEDCOM 7.0 |
|---------|--------------|------------|
| Character Encoding | Multiple (ANSEL, ASCII, UTF-8, UNICODE) | UTF-8 only |
| `CHAR` tag | Required | Removed |
| BOM | Not specified | Optional UTF-8 BOM recommended |

**Migration Note:** If you're creating GEDCOM 7.0 files, always use UTF-8 encoding. The `CHAR` tag should not be included.

```rust
use ged_io::version::GedcomVersion;

let rules = GedcomVersion::V7_0.rules();
assert!(!rules.has_head_char()); // 7.0 is UTF-8 only: no CHAR
```

### 2. Line Continuation

| Feature | GEDCOM 5.5.1 | GEDCOM 7.0 |
|---------|--------------|------------|
| `CONT` tag | Supported | Supported |
| `CONC` tag | Supported | **Removed** |

**Migration Note:** GEDCOM 7.0 removes the `CONC` tag. Use only `CONT` for multi-line text. The library handles this automatically when writing.

```rust
use ged_io::version::GedcomVersion;

assert!(GedcomVersion::V5_5_1.rules().uses_conc());
assert!(!GedcomVersion::V7_0.rules().uses_conc());
```

### 3. @ Sign Escaping

| Feature | GEDCOM 5.5.1 | GEDCOM 7.0 |
|---------|--------------|------------|
| Escaping Rule | All `@` doubled (`@@`) | Only leading `@` doubled |

**Migration Note:** Use the utility functions for proper escaping:

```rust
use ged_io::util::{escape_at_signs, unescape_at_signs};

// GEDCOM 5.5.1: all @ doubled
let escaped_v5 = escape_at_signs("email@example.com", false);
assert_eq!(escaped_v5, "email@@example.com");

// GEDCOM 7.0: only leading @ doubled
let escaped_v7 = escape_at_signs("email@example.com", true);
assert_eq!(escaped_v7, "email@example.com");

let escaped_v7_leading = escape_at_signs("@reference", true);
assert_eq!(escaped_v7_leading, "@@reference");
```

### 4. New Record Types

#### Shared Notes (`SNOTE`)

GEDCOM 7.0 introduces `SNOTE` records for notes that can be referenced by multiple structures:

```rust
use ged_io::Gedcom;

let gedcom_7 = "\
    0 HEAD\n\
    1 GEDC\n\
    2 VERS 7.0\n\
    0 @N1@ SNOTE This note can be referenced by multiple records.\n\
    1 MIME text/plain\n\
    1 LANG en\n\
    0 TRLR";

let mut parser = Gedcom::new(gedcom_7.chars()).unwrap();
let data = parser.parse_data().unwrap();

assert_eq!(data.shared_notes.len(), 1);
let note = data.find_shared_note("@N1@").unwrap();
assert!(note.text.contains("referenced"));
```

#### Schema (`SCHMA`)

GEDCOM 7.0 formalizes extension tags via the `SCHMA` structure:

```rust
use ged_io::Gedcom;

let gedcom_7 = "\
    0 HEAD\n\
    1 GEDC\n\
    2 VERS 7.0\n\
    1 SCHMA\n\
    2 TAG _CUSTOM http://example.com/gedcom-extensions/custom\n\
    0 TRLR";

let mut parser = Gedcom::new(gedcom_7.chars()).unwrap();
let data = parser.parse_data().unwrap();

let header = data.header.unwrap();
assert_eq!(
    header.find_extension_uri("_CUSTOM"),
    Some("http://example.com/gedcom-extensions/custom")
);
```

### 5. New Substructures

#### Sort Date (`SDATE`)

A date used for sorting when the actual date is vague:

```text
1 BIRT
2 DATE BEF 1820
2 SDATE 1818
```

#### Non-Events (`NO`)

Asserts that an event did NOT occur (distinct from unknown):

```text
0 @I1@ INDI
1 NO MARR
2 NOTE Never married per family records.
```

#### Phrases (`PHRASE`)

Free-text representation of dates:

```text
1 BIRT
2 DATE 15 MAR 1820
3 PHRASE The Ides of March, in the year 1820
```

#### Creation Date (`CREA`)

Records when a structure was first created (vs `CHAN` for last modified):

```text
0 @I1@ INDI
1 CREA
2 DATE 15 MAR 2020
```

#### Image Cropping (`CROP`)

Defines a region of an image to display:

```text
1 FILE photo.jpg
2 CROP
3 TOP 10
3 LEFT 15
3 HEIGHT 50
3 WIDTH 40
```

### 6. LDS Ordinances

#### New: `INIL` (Initiatory)

GEDCOM 7.0 adds the `INIL` tag for LDS initiatory ordinances:

```rust
use ged_io::types::lds::{LdsOrdinance, LdsOrdinanceType};

let ordinance = LdsOrdinance::with_type(LdsOrdinanceType::Initiatory)
    .with_date("15 MAR 1990")
    .with_temple("SLAKE");

assert!(ordinance.is_gedcom_7_only());
```

**Available LDS Ordinance Types:**

| Tag | Type | Records | GEDCOM 7.0 Only |
|-----|------|---------|-----------------|
| `BAPL` | Baptism | Individual | No |
| `CONL` | Confirmation | Individual | No |
| `INIL` | Initiatory | Individual | **Yes** |
| `ENDL` | Endowment | Individual | No |
| `SLGC` | Sealing to Parents | Individual | No |
| `SLGS` | Sealing to Spouse | Family | No |

### 7. Removed Structures

| Structure | Status in 7.0 |
|-----------|--------------|
| `SUBN` (Submission record) | Removed |
| `CHAR` (Character encoding) | Removed |
| `CONC` (Concatenation) | Removed |

## Version Detection

The library automatically detects the GEDCOM version:

```rust
use ged_io::{detect_version, GedcomVersion};

let content = std::fs::read_to_string("my_file.ged").unwrap();
let version = detect_version(&content);

// 7.x declarations read as 7.0 or 7.1; anything else follows 5.5.1.
if version.is_v7() {
    println!("GEDCOM {version} file");
} else {
    println!("GEDCOM 5.5.1 file");
}

// The writing rules of the version
let rules = version.rules();
println!("longest line: {:?}", rules.max_line_length());
```

## Checking Version Programmatically

```rust
use ged_io::GedcomBuilder;

let data = GedcomBuilder::new().build_from_str(content)?;

if data.is_gedcom_7() {
    // Use GEDCOM 7.0 features
    for note in &data.shared_notes {
        println!("Shared note: {}", note.text);
    }
} else {
    // Handle GEDCOM 5.5.1
}
```

## Writing Version-Specific Files

```rust
use ged_io::{GedcomVersion, GedcomWriter};

// Write in the version the data declares (5.5.1 when it declares none)
let writer = GedcomWriter::new();
let output_551 = writer.write_to_string(&data)?;

// Write as GEDCOM 7.0
let writer = GedcomWriter::new().gedcom_version(GedcomVersion::V7_0);
let output_70 = writer.write_to_string(&data)?;
```

## Best Practices for Migration

1. **Version Detection First**: Always detect the version before processing
2. **Graceful Degradation**: Handle missing 7.0 features when reading 5.5.1 files
3. **UTF-8 Always**: Use UTF-8 encoding for all new files
4. **Test Round-Trips**: Verify data integrity when converting between versions
5. **Handle Shared Notes**: If converting to 7.0, consider extracting common notes to `SNOTE` records
6. **Document Extensions**: Use `SCHMA` to document any custom tags in 7.0 files

## Common Migration Patterns

### Converting Inline Notes to Shared Notes

```rust
use ged_io::types::shared_note::SharedNote;

// Create a shared note from common text
let shared_note = SharedNote::with_text("@N1@", "Common note text used in multiple places");
data.add_shared_note(shared_note);
```

### Handling CONC in 7.0

Reading joins `CONC` continuations into the text; writing GEDCOM 7.0 never
splits a payload, so 7.0 output has no `CONC` at all:

```rust
let writer = GedcomWriter::new().gedcom_version(GedcomVersion::V7_0);
// Long text stays on one line; line breaks become CONT lines.
```

## Additional Resources

- [GEDCOM 7.0 Specification](https://gedcom.io/specifications/FamilySearchGEDCOMv7.html)
- [GEDCOM 5.5.1 Specification](https://gedcom.io/specifications/ged551.pdf)
- [ged_io Documentation](https://docs.rs/ged_io)

---

*This migration guide is part of the `ged_io` library documentation.*
