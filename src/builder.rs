//! Reading a dataset with options: a size limit, strict mode, a given
//! encoding, a reader or a GEDZIP archive.
//!
//! ```rust
//! use ged_io::GedcomBuilder;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let text = "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 NAME Ann /Example/\n0 TRLR\n";
//! let data = GedcomBuilder::new().max_file_size(50_000_000).build_from_str(text)?;
//! assert_eq!(data.individuals[0].full_name(&data).as_deref(), Some("Ann Example"));
//! # Ok(())
//! # }
//! ```

use std::io::Read;

use crate::encoding::{decode_as, decode_owned, GedcomEncoding};
use crate::model::Dataset;
use crate::spec::{validate_bytes, validate_text, Deviation};
use crate::GedcomError;

/// Reads datasets, with options.
///
/// By default reading is lenient and has no limit: any input is read, its
/// data kept, and only I/O can fail. Options:
///
/// - [`max_file_size`](Self::max_file_size) refuses an input larger than a
///   limit before reading it;
/// - [`strict`](Self::strict) refuses an input that does not follow the
///   specification of the version it declares, with every deviation
///   ([`crate::spec::validate_bytes`]); what strict mode accepts reads
///   exactly as leniently.
///
/// A builder is cheap to clone and reusable.
#[derive(Clone, Debug, Default)]
pub struct GedcomBuilder {
    strict: bool,
    max_file_size: Option<usize>,
}

impl GedcomBuilder {
    /// A builder that reads leniently, without limit.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether to refuse an input that does not follow its specification
    /// (default: no).
    ///
    /// In strict mode, reading runs the validator of [`crate::spec`] on the
    /// input — its encoding, its lines, its structures, its pointers — and
    /// fails with [`GedcomError::NonConformant`] and every deviation found.
    ///
    /// ```rust
    /// use ged_io::{GedcomBuilder, GedcomError};
    ///
    /// let text = "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 SEX male\n0 TRLR\n";
    /// assert!(GedcomBuilder::new().build_from_str(text).is_ok());
    /// let Err(GedcomError::NonConformant(deviations)) =
    ///     GedcomBuilder::new().strict(true).build_from_str(text)
    /// else {
    ///     panic!()
    /// };
    /// assert_eq!(deviations[0].to_string(), "line 5: SEX \"male\": not a value of enumset-SEX");
    /// ```
    #[must_use]
    pub fn strict(mut self, enabled: bool) -> Self {
        self.strict = enabled;
        self
    }

    /// Refuses an input larger than `bytes` with
    /// [`GedcomError::FileTooLarge`], before reading it (default: no
    /// limit). For a GEDZIP archive, the limit applies to its `gedcom.ged`.
    #[must_use]
    pub fn max_file_size(mut self, bytes: usize) -> Self {
        self.max_file_size = Some(bytes);
        self
    }

    /// Whether strict mode is on.
    #[must_use]
    pub fn is_strict(&self) -> bool {
        self.strict
    }

    /// The size limit, if any.
    #[must_use]
    pub fn file_size_limit(&self) -> Option<usize> {
        self.max_file_size
    }

    fn check_size(&self, size: usize) -> Result<(), GedcomError> {
        match self.max_file_size {
            Some(max) if size > max => Err(GedcomError::FileTooLarge { size, max }),
            _ => Ok(()),
        }
    }

    fn check(&self, deviations: impl FnOnce() -> Vec<Deviation>) -> Result<(), GedcomError> {
        if !self.strict {
            return Ok(());
        }
        let deviations = deviations();
        if deviations.is_empty() {
            Ok(())
        } else {
            Err(GedcomError::NonConformant(deviations.into()))
        }
    }

    /// Reads decoded text. A `String` is kept as the dataset's store,
    /// without a copy.
    ///
    /// # Errors
    ///
    /// [`GedcomError::FileTooLarge`] over the limit; in strict mode,
    /// [`GedcomError::NonConformant`].
    pub fn build_from_str(&self, text: impl Into<String>) -> Result<Dataset, GedcomError> {
        let text = text.into();
        self.check_size(text.len())?;
        self.check(|| validate_text(&text))?;
        Ok(Dataset::parse(text))
    }

    /// Reads bytes, decoded by the evidence they hold and their `HEAD.CHAR`
    /// (see [`crate::encoding`]); never fails to decode. A `Vec<u8>` of
    /// UTF-8 becomes the dataset's store without a copy.
    ///
    /// # Errors
    ///
    /// [`GedcomError::FileTooLarge`] over the limit; in strict mode,
    /// [`GedcomError::NonConformant`] (an encoding that contradicts the
    /// declared one included).
    ///
    /// ```rust
    /// use ged_io::GedcomBuilder;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let data = GedcomBuilder::new()
    ///     .build_from_bytes(b"0 HEAD\r1 CHAR ANSI\r0 @I1@ INDI\r1 NAME Ren\xE9e /Example/\r0 TRLR\r")?;
    /// assert_eq!(data.individuals[0].full_name(&data).as_deref(), Some("Renée Example"));
    /// # Ok(())
    /// # }
    /// ```
    pub fn build_from_bytes(&self, bytes: impl Into<Vec<u8>>) -> Result<Dataset, GedcomError> {
        let bytes = bytes.into();
        self.check_size(bytes.len())?;
        self.check(|| validate_bytes(&bytes))?;
        Ok(Dataset::parse(decode_owned(bytes).text))
    }

    /// Reads bytes in a given encoding, whatever they declare (see
    /// [`crate::encoding::decode_as`]).
    ///
    /// # Errors
    ///
    /// As [`build_from_str`](Self::build_from_str).
    pub fn build_from_bytes_with_encoding(
        &self,
        bytes: &[u8],
        encoding: GedcomEncoding,
    ) -> Result<Dataset, GedcomError> {
        self.check_size(bytes.len())?;
        let text = decode_as(bytes, encoding);
        self.check(|| validate_text(&text))?;
        Ok(Dataset::parse(text))
    }

    /// Reads everything `reader` gives, as
    /// [`build_from_bytes`](Self::build_from_bytes) reads bytes. With a
    /// size limit, no more than the limit is read.
    ///
    /// # Errors
    ///
    /// [`GedcomError::Io`] when `reader` fails, otherwise as
    /// [`build_from_bytes`](Self::build_from_bytes).
    pub fn build_from_reader(&self, reader: impl Read) -> Result<Dataset, GedcomError> {
        let mut bytes = Vec::new();
        // Past the limit by one byte at most: enough to refuse the input.
        let limit = self.max_file_size.map_or(u64::MAX, |max| {
            u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1)
        });
        reader.take(limit).read_to_end(&mut bytes)?;
        self.build_from_bytes(bytes)
    }

    /// Reads the dataset of a GEDZIP archive (its `gedcom.ged`), as
    /// [`build_from_bytes`](Self::build_from_bytes) reads bytes.
    ///
    /// ```rust,no_run
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use ged_io::GedcomBuilder;
    ///
    /// let data = GedcomBuilder::new().build_from_gedzip(std::fs::File::open("family.gdz")?)?;
    /// println!("{} individuals", data.individuals.len());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`GedcomError::Gedzip`] when the archive cannot be read or has no
    /// `gedcom.ged`; otherwise as [`build_from_bytes`](Self::build_from_bytes).
    #[cfg(feature = "gedzip")]
    pub fn build_from_gedzip<R: Read + std::io::Seek>(
        &self,
        archive: R,
    ) -> Result<Dataset, GedcomError> {
        crate::gedzip::GedzipReader::new(archive)?.read_dataset(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "0 HEAD\n1 GEDC\n2 VERS 5.5.1\n0 @I1@ INDI\n1 NAME Ann /Example/\n0 TRLR\n";

    #[test]
    fn defaults() {
        let builder = GedcomBuilder::new();
        assert!(!builder.is_strict());
        assert_eq!(builder.file_size_limit(), None);
        let data = builder.build_from_str(TEXT).unwrap();
        assert_eq!(data.individuals.len(), 1);
        assert_eq!(builder.build_from_bytes(TEXT.as_bytes()).unwrap(), data);
        assert_eq!(builder.build_from_reader(TEXT.as_bytes()).unwrap(), data);
        assert_eq!(
            builder
                .build_from_bytes_with_encoding(TEXT.as_bytes(), GedcomEncoding::Utf8)
                .unwrap(),
            data
        );
    }

    #[test]
    fn size_limit() {
        let builder = GedcomBuilder::new().max_file_size(10);
        for result in [
            builder.build_from_str(TEXT),
            builder.build_from_bytes(TEXT.as_bytes()),
            builder.build_from_reader(TEXT.as_bytes()),
            builder.build_from_bytes_with_encoding(TEXT.as_bytes(), GedcomEncoding::Utf8),
        ] {
            assert!(matches!(
                result,
                Err(GedcomError::FileTooLarge { max: 10, .. })
            ));
        }
        // The reader stops past the limit.
        assert!(matches!(
            builder.build_from_reader(std::io::repeat(b'0')),
            Err(GedcomError::FileTooLarge { size: 11, max: 10 })
        ));
        assert!(GedcomBuilder::new()
            .max_file_size(TEXT.len())
            .build_from_str(TEXT)
            .is_ok());
    }

    #[test]
    fn strict_mode_lists_every_deviation() {
        let text = "0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 SEX male\n1 FAMC @F9@\n0 TRLR\n";
        let strict = GedcomBuilder::new().strict(true);
        let Err(GedcomError::NonConformant(deviations)) = strict.build_from_str(text) else {
            panic!()
        };
        assert_eq!(deviations.len(), 2, "{deviations:?}");
        assert!(strict.build_from_str(TEXT.replace("5.5.1", "7.0")).is_ok());
        // The encoding counts too: a 7.x file is UTF-8.
        let latin1 = b"0 HEAD\n1 GEDC\n2 VERS 7.0\n0 @I1@ INDI\n1 NAME Ren\xE9e\n0 TRLR\n";
        assert!(GedcomBuilder::new().build_from_bytes(latin1).is_ok());
        assert!(matches!(
            strict.build_from_bytes(latin1),
            Err(GedcomError::NonConformant(_))
        ));
    }
}
