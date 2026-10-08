//! Multimedia links and files.

use super::driver::gedcom_struct;
use super::enums::{Medium, MultimediaFormat, Phrased};
use super::list::ThinVec;
use super::text::{Store, Text, XrefId};

gedcom_struct! {
    /// A multimedia link (`OBJE`): a pointer to a multimedia record, with
    /// the part of the image shown (7.x `CROP`) and a title (7.x `TITL`);
    /// or, in 5.5.1, the files themselves (`FILE`) and their title.
    pub struct MultimediaLink {
        @payload
        /// The multimedia record (none for a 5.5.1 link that holds its
        /// files).
        object: Option<XrefId>;
        /// The files of a 5.5.1 link without a record (`FILE`).
        "FILE" => files: ThinVec<File>,
        /// The part of the image shown (`CROP`).
        "CROP" => crop: Option<Box<Crop>>,
        /// The title (`TITL`).
        "TITL" => title: Option<Text>,
    }
    spec {
        v551: ["MULTIMEDIA_LINK.OBJE", "MULTIMEDIA_LINK.OBJE#2"],
        v70: ["OBJE"],
        v71: ["OBJE"],
    }
}

gedcom_struct! {
    /// The part of an image a link shows (7.x `CROP`), in pixels from the
    /// top left corner. A value that is not a non-negative integer stays in
    /// `extra`, as written.
    pub struct Crop {
        /// Pixels from the top (`TOP`).
        "TOP" => top: Option<u32>,
        /// Pixels from the left (`LEFT`).
        "LEFT" => left: Option<u32>,
        /// Height in pixels (`HEIGHT`).
        "HEIGHT" => height: Option<u32>,
        /// Width in pixels (`WIDTH`).
        "WIDTH" => width: Option<u32>,
    }
    spec {
        v551: [],
        v70: ["CROP"],
        v71: ["CROP"],
    }
}

gedcom_struct! {
    /// A multimedia file (`FILE`): its path or URL, its format (`FORM`), a
    /// title (`TITL`) and alternative versions (7.x `TRAN`).
    pub struct File {
        @payload
        /// The file's path or URL.
        path: Text;
        /// The format and medium (`FORM`).
        "FORM" => form: Option<FileForm>,
        /// The title (`TITL`).
        "TITL" => title: Option<Text>,
        /// Alternative versions of the file (`TRAN`).
        "TRAN" => translations: ThinVec<FileTranslation>,
    }
    spec {
        v551: ["MULTIMEDIA_LINK.OBJE#2.FILE", "MULTIMEDIA_RECORD.OBJE.FILE"],
        v70: ["FILE"],
        v71: ["FILE"],
    }
}

gedcom_struct! {
    /// The format of a file (`FORM`): a media type in 7.x (`image/jpeg`),
    /// a format of [`MultimediaFormat`] in 5.5.1 (`jpg`), kept as written;
    /// and the medium of the original (`MEDI`).
    pub struct FileForm {
        @payload
        /// The format as written.
        format: Text;
        /// The medium (`MEDI`), with a phrase in 7.x.
        "MEDI" => medium: Option<Box<Phrased<Medium>>>,
        /// The medium of a file of a 5.5.1 multimedia record (`TYPE`, which
        /// links and 7.x name `MEDI`), kept as read (D12).
        "TYPE" => medium_type: Option<Box<Medium>>,
    }
    spec {
        v551: [
            "MULTIMEDIA_LINK.OBJE#2.FILE.FORM",
            "MULTIMEDIA_RECORD.OBJE.FILE.FORM",
        ],
        v70: ["FORM"],
        v71: ["FORM"],
    }
}

impl FileForm {
    /// The format as a 5.5.1 multimedia format.
    #[must_use]
    pub fn multimedia_format<S: AsRef<Store> + ?Sized>(&self, store: &S) -> MultimediaFormat {
        MultimediaFormat::parse(&self.format.to_str(store))
    }
}

gedcom_struct! {
    /// An alternative version of a file (7.x `FILE.TRAN`), such as a
    /// thumbnail or a transcript, and its format.
    pub struct FileTranslation {
        @payload
        /// The path or URL of the version.
        path: Text;
        /// Its format (`FORM`).
        "FORM" => form: Option<FileForm>,
    }
    spec {
        v551: [],
        v70: ["FILE-TRAN"],
        v71: ["FILE-TRAN"],
    }
}
