# ged_io Roadmap

---

## Phase 1: Date Arithmetic & Age Parsing (v0.13) ✅

Dates are the backbone of genealogy. Every downstream app (timelines, reports, tree renderers) needs date comparison, sorting, and age calculation.

### Serial Day Number (SDN) Conversion ✅
- `CalendarDate::to_rata_die()` / `to_julian_day_number()` and `from_rata_die()` / `from_julian_day_number()` (behind the `calendar` feature) — `src/value/conversion.rs`
- `chronological_cmp()` across calendars, `ordering_key()` for sorting
- `days_until()`, `add_days()` for date arithmetic
- `CalendarDate::convert_to()`, `DateValue::convert_to()`, `model::Date::convert_to()` between calendars

### Age Parsing ✅
- `value::AgeValue`: a bounded duration (`< 8y`, `25y 3m`) or a 5.5.1 keyword (`CHILD`, `INFANT`, `STILLBORN`), any other text kept
- Both 5.5.1 and 7.0 forms, `PHRASE`, conversion between versions (`model::Age::to_version`)

### Day of Week Calculation ✅
- `CalendarDate::weekday() -> Option<Weekday>`

### Date Normalization ✅
- `model::Date::normalize(store, version)`: upper case, single spaces, well-formed calendar escapes or keywords
- Lenient and strict grammars of both versions for dates, ages and times (`value::*::parse`, `parse_strict`)

---

## Phase 2: Record Manipulation API (v0.15)

Any app that edits genealogy data needs operations that keep the dataset consistent.

### Records
- ✅ Every record type is a plain struct in a list of the `Dataset`; `Dataset::push(Record)` adds one, `Dataset::store_mut().intern_xref()` gives identifiers, `Text::new` owned texts — `src/model/dataset.rs`
- `remove_*(xref)` returning the record; generated unique identifiers for new records
- Clean up or report pointers left dangling by a removal (`Dataset::dangling_references()` ✅ finds them)

### Cross-reference Linking/Unlinking
- ✅ Relationship traversal: `families_as_spouse`, `families_as_child`, `parents`, `children`, `spouse`, on `Dataset` and, in constant time, on `IndexedDataset`
- `link_child_to_family(indi, fam)` — adds `CHIL` to the family and `FAMC` to the individual
- `link_spouse_to_family(indi, fam, role)`, `unlink_individual_from_family(indi, fam)`

### Index Sync
- ✅ `IndexedDataset::new(Dataset)` builds the index; `data_mut()` reindexes when the edit ends — `src/indexed.rs`

---

## Phase 3: Error Handling Modes & Compatibility (v0.16) ✅

Real-world GEDCOM files are messy: reading never fails and keeps their data; conformance is checked, and enforced on output.

- ✅ Lenient reading of any input, data kept in place (`extra`) — `src/tree/`, `src/model/`
- ✅ `GedcomBuilder::strict(true)`: the input is refused with every deviation from its specification (`GedcomError::NonConformant`) — `src/builder.rs`
- ✅ `spec::validate*`: every deviation with its line, without failing — `src/spec/`
- ✅ `GedcomWriter`: conformant 5.5.1, 7.0 or 7.1 output, every repair reported — `src/writer/`
- ✅ Sanitizer CLI: `ged-io --write <version> <file.ged>`

---

## Phase 4: Conversion Tools & Visitor Pattern (v0.17+)

### GEDCOM Version Conversion
- ✅ `GedcomVersion` (5.5.1, 7.0, 7.1) with a `VersionRules` table per version — `src/version.rs`
- ✅ Values converted between versions when written (dates, ages, times, enumeration spellings); structures the target does not permit kept as extensions
- Dataset-level conversion following the migration guides (inline `OBJE` to records, `SNOTE` ↔ `NOTE` record, `SUBN`, `ASSO.RELA` ↔ `ROLE`, media types and file URIs, language tags)

### Streaming
- ✅ `GedcomStreamParser`: typed records one at a time, any encoding — `src/stream.rs`
- A streaming writer: records written one at a time, with the writer's conformance repairs
- `GedcomVisitor` trait with `on_individual()`, `on_family()`, etc. returning `ControlFlow`

### Encoding Conversion
- ✅ `encoding::{decode, decode_as, detect_encoding, encode}` and `DecodeReader`; UTF-8, UTF-16 LE/BE, ANSEL, ASCII, Windows-1252, ISO-8859-1/15, IBM PC, Macintosh — `src/encoding/`
- ✅ 5.5.1 output in ANSEL, ASCII or UTF-16 (`GedcomWriter::output_encoding`)

---

## Excluded

- **Locale-aware string handling** — Rust uses UTF-8 natively; consuming apps handle display
- **Raw SAX-like tag parser** — `GedcomStreamParser` (typed) and `tree::TreeReader` (lossless structures) cover it

---

## Summary

| Phase | Version | Theme | Key Deliverables |
|-------|---------|-------|-----------------|
| 1 | v0.13 | Date & Age | SDN conversion, ages, date ordering, day-of-week, normalization |
| 2 | v0.15 | Record Mutation | Record removal, xref linking/unlinking |
| 3 | v0.16 | Error & Compat | Lenient reading, strict mode, validation, conformant writer, sanitizer CLI |
| 4 | v0.17+ | Conversion & Visitor | Dataset conversion between versions, streaming writer, visitor trait |
