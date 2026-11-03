//! The version a DWG file declares in its first six bytes.
//!
//! The specification's file header begins with six ASCII bytes naming the
//! format version (`AC1015` for R2000, and so on); the rest of the header
//! differs by version. This module reads only those six bytes. It does not
//! look at anything after them: the specification says the bytes that follow
//! have been seen with other values than the documented ones, so they say
//! nothing about whether the six-byte answer is right.

use std::fmt;

/// A format version this reader reads.
///
/// The list is the span the reader is built for: R2000 through R2018. A
/// version outside it is reported by [`detect_version`] as an [`Error`], not
/// approximated by a neighbour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Version {
    /// `AC1015`.
    R2000,
    /// `AC1018`.
    R2004,
    /// `AC1021`.
    R2007,
    /// `AC1024`.
    R2010,
    /// `AC1027`.
    R2013,
    /// `AC1032`.
    R2018,
}

impl Version {
    /// Every version the reader reads, oldest first.
    pub const ALL: [Version; 6] = [
        Version::R2000,
        Version::R2004,
        Version::R2007,
        Version::R2010,
        Version::R2013,
        Version::R2018,
    ];

    /// The six bytes a file of this version starts with.
    pub const fn magic(self) -> [u8; 6] {
        match self {
            Version::R2000 => *b"AC1015",
            Version::R2004 => *b"AC1018",
            Version::R2007 => *b"AC1021",
            Version::R2010 => *b"AC1024",
            Version::R2013 => *b"AC1027",
            Version::R2018 => *b"AC1032",
        }
    }

    /// The version whose file starts with `magic`, if the reader reads it.
    pub fn from_magic(magic: [u8; 6]) -> Option<Version> {
        Version::ALL.into_iter().find(|v| v.magic() == magic)
    }

    /// The product-release name of the version (`"R2000"`).
    pub const fn name(self) -> &'static str {
        match self {
            Version::R2000 => "R2000",
            Version::R2004 => "R2004",
            Version::R2007 => "R2007",
            Version::R2010 => "R2010",
            Version::R2013 => "R2013",
            Version::R2018 => "R2018",
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({})",
            self.name(),
            String::from_utf8_lossy(&self.magic())
        )
    }
}

/// Why a version is not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Unsupported {
    /// The specification documents this version (R13 or R14) but the reader
    /// does not read it.
    Documented(&'static str),
    /// The first six bytes are not a version the specification lists.
    Unrecognized,
}

/// Why the version of a file could not be determined as a readable one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Fewer than six bytes: there is no version to read.
    Truncated {
        /// How many bytes there were.
        len: usize,
    },
    /// The file declares a version the reader does not read. The six bytes
    /// are kept as found, so a message can name them.
    UnsupportedVersion {
        /// The first six bytes of the file.
        magic: [u8; 6],
        /// Whether the specification documents the version.
        kind: Unsupported,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Truncated { len } => write!(
                f,
                "a DWG file starts with six version bytes; this has {len}"
            ),
            Error::UnsupportedVersion { magic, kind } => {
                let shown = String::from_utf8_lossy(magic);
                match kind {
                    Unsupported::Documented(name) => {
                        write!(f, "unsupported DWG version {shown} ({name}); this reader reads AC1015 to AC1032 (R2000 to R2018)")
                    }
                    Unsupported::Unrecognized => {
                        write!(f, "unrecognized DWG version bytes {shown:?}; this reader reads AC1015 to AC1032 (R2000 to R2018)")
                    }
                }
            }
        }
    }
}

impl std::error::Error for Error {}

/// The version a file declares in its first six bytes.
///
/// Only the first six bytes are looked at, so a prefix of a file is enough.
/// A version outside R2000 to R2018 is an [`Error::UnsupportedVersion`] that
/// says whether the specification documents it.
pub fn detect_version(bytes: &[u8]) -> Result<Version, Error> {
    let Some(head) = bytes.first_chunk::<6>() else {
        return Err(Error::Truncated { len: bytes.len() });
    };
    if let Some(version) = Version::from_magic(*head) {
        return Ok(version);
    }
    let kind = match head {
        b"AC1012" => Unsupported::Documented("R13"),
        b"AC1014" => Unsupported::Documented("R14"),
        _ => Unsupported::Unrecognized,
    };
    Err(Error::UnsupportedVersion { magic: *head, kind })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_documented_magic_is_its_version_and_round_trips() {
        for (magic, expected) in [
            (b"AC1015", Version::R2000),
            (b"AC1018", Version::R2004),
            (b"AC1021", Version::R2007),
            (b"AC1024", Version::R2010),
            (b"AC1027", Version::R2013),
            (b"AC1032", Version::R2018),
        ] {
            assert_eq!(detect_version(magic), Ok(expected));
            assert_eq!(expected.magic(), *magic);
            assert_eq!(Version::from_magic(*magic), Some(expected));
        }
        assert_eq!(Version::ALL.len(), 6);
        assert!(Version::ALL.windows(2).all(|w| w[0] < w[1]), "oldest first");
    }

    #[test]
    fn only_the_first_six_bytes_decide() {
        // What follows the six bytes is not looked at (the specification warns it varies).
        let mut file = b"AC1032".to_vec();
        file.extend_from_slice(&[0xFF; 100]);
        assert_eq!(detect_version(&file), Ok(Version::R2018));
        assert_eq!(
            detect_version(b"AC1018"),
            Ok(Version::R2004),
            "exactly six bytes is enough"
        );
    }

    #[test]
    fn a_file_shorter_than_six_bytes_has_no_version() {
        assert_eq!(detect_version(b""), Err(Error::Truncated { len: 0 }));
        assert_eq!(detect_version(b"AC101"), Err(Error::Truncated { len: 5 }));
    }

    #[test]
    fn the_older_documented_versions_are_named_and_refused() {
        assert_eq!(
            detect_version(b"AC1012\0"),
            Err(Error::UnsupportedVersion {
                magic: *b"AC1012",
                kind: Unsupported::Documented("R13")
            })
        );
        assert_eq!(
            detect_version(b"AC1014\0"),
            Err(Error::UnsupportedVersion {
                magic: *b"AC1014",
                kind: Unsupported::Documented("R14")
            })
        );
    }

    #[test]
    fn anything_else_is_unrecognized_not_approximated() {
        // A neighbour of a readable version is not that version.
        for bytes in [
            b"AC1016",
            b"AC1033",
            b"AC1009",
            b"ac1015",
            b"AC 015",
            b"\0\0\0\0\0\0",
            b"MC0.0 ",
        ] {
            assert!(
                matches!(
                    detect_version(bytes),
                    Err(Error::UnsupportedVersion {
                        kind: Unsupported::Unrecognized,
                        ..
                    })
                ),
                "{bytes:?}"
            );
        }
    }

    #[test]
    fn messages_name_what_was_found_and_what_is_read() {
        let r14 = detect_version(b"AC1014").unwrap_err().to_string();
        assert!(
            r14.contains("AC1014") && r14.contains("R14") && r14.contains("R2000 to R2018"),
            "{r14}"
        );
        let junk = detect_version(b"\x01\x02ABCD").unwrap_err().to_string();
        assert!(
            junk.contains("unrecognized") && junk.contains("R2000 to R2018"),
            "{junk}"
        );
        assert_eq!(Version::R2010.to_string(), "R2010 (AC1024)");
    }
}
