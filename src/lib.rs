/*!
Reads and writes GEDCOM, the genealogical data format, in versions 5.5.1,
7.0 and 7.1.

- **Reading never fails and loses nothing.** Bytes of any encoding are
  decoded ([`encoding`]), any line is read by fixed, lenient rules
  ([`tree`]), and every record is typed as far as it fits the model
  ([`model`]): what a type has no field for is kept, where it was. An
  opt-in strict mode refuses input that does not follow its specification
  ([`GedcomBuilder::strict`]).
- **Writing is conformant.** [`GedcomWriter`] writes a dataset in its
  version or another, repairing what the target does not permit
  ([`spec::conform`]) and emitting every line by the target's line rules.
- **Streaming and indexing.** [`GedcomStreamParser`] reads one record at a
  time in bounded memory; [`IndexedDataset`] finds records and families in
  constant time; `ged_io::gedzip` reads and writes GEDZIP archives (feature
  `gedzip`).
- **The specifications as data.** [`spec`] validates a file against the
  tables of its version; [`value`] reads and converts dates, ages and
  times.

```rust
use ged_io::{Dataset, GedcomVersion, GedcomWriter};

let data = Dataset::parse(
    "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NAME Ann /Example/\n1 BIRT\n\
     2 DATE ABT @#DJULIAN@ 1700\n0 TRLR\n",
);
let ann = &data.individuals[0];
assert_eq!(ann.full_name(&data).as_deref(), Some("Ann Example"));
let birth = ann.birth().and_then(|b| b.date.as_ref()).unwrap();
assert_eq!(birth.value.to_str(&data), "ABT @#DJULIAN@ 1700");

let out = GedcomWriter::new()
    .gedcom_version(GedcomVersion::V7_0)
    .write_to_string(&data)
    .unwrap();
assert!(out.contains("2 DATE ABT JULIAN 1700\n"));
```

# Features

- `serde`: `Serialize` and `Deserialize` for the values of [`value`]
  and the structures of [`tree`].
- `gedzip`: GEDZIP archives (`ged_io::gedzip`).
- `calendar`: conversions between calendars and day numbers
  ([`value::CalendarDate`]).
*/
#![cfg_attr(not(test), deny(clippy::cargo, clippy::pedantic, clippy::panic))]
#![deny(clippy::all)]
#![deny(missing_docs)]

pub mod builder;
pub mod encoding;
pub mod error;
#[cfg(feature = "gedzip")]
pub mod gedzip;
pub mod indexed;
pub mod model;
pub mod spec;
pub mod stream;
pub mod tree;
pub mod value;
pub mod version;
pub mod writer;

pub use builder::GedcomBuilder;
pub use encoding::GedcomEncoding;
pub use error::GedcomError;
pub use indexed::IndexedDataset;
pub use model::Dataset;
pub use stream::{GedcomStreamParser, StreamedRecord};
pub use version::{GedcomVersion, VersionRules};
pub use writer::{
    Bom, GedcomWriter, LineEnding, OutputEncoding, Repair, RepairKind, RepairPolicy, Unencodable,
    WriteError, WriteReport, WriterConfig,
};
