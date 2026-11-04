//! The object map and the frames of the objects it points to.
//!
//! The `AcDb:Handles` section is the table of contents of the object data: a
//! list of (handle, position) pairs, written as increments, in blocks that
//! each end in a checksum. Each position is an offset into the logical
//! `AcDb:AcDbObjects` section, where the object begins with its size in bytes
//! (a modular short), and ends with a 16-bit checksum.
//!
//! Only the *frame* of an object is read here -- size, type, checksum -- not
//! what the type's fields mean.

use std::fmt;

use crate::bits::{BitError, BitReader};
use crate::checksum::crc16;
use crate::version::Version;

/// The seed of the checksums in the object map and on every object.
const CRC_SEED: u16 = 0xC0C1;

/// One entry of the object map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectEntry {
    /// The object's handle.
    pub handle: u64,
    /// The object's offset in the logical `AcDb:AcDbObjects` section.
    pub offset: u64,
}

/// The frame of one object: where it is and what kind it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectFrame {
    /// The object's handle, from the object map.
    pub handle: u64,
    /// The object's type code.
    pub type_code: u16,
    /// The object's size in bytes, not counting its checksum.
    pub size: u32,
    /// The bytes the object spans in the section, counting its size field and its checksum.
    pub span: usize,
}

/// Why the object map or an object could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ObjectError {
    /// The data is not laid out the way the format documents.
    Unexpected {
        /// What was wrong.
        what: &'static str,
    },
    /// A checksum does not match.
    Checksum {
        /// Which one.
        what: &'static str,
        /// The value computed from the bytes.
        computed: u16,
        /// The value the data stores.
        stored: u16,
    },
    /// A value could not be read from the bit stream.
    Bits(BitError),
    /// The version has no object reader.
    NotRead(Version),
}

impl fmt::Display for ObjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObjectError::Unexpected { what } => {
                write!(
                    f,
                    "the object data's {what} is not what the format documents"
                )
            }
            ObjectError::Checksum {
                what,
                computed,
                stored,
            } => write!(
                f,
                "{what} checksum mismatch: computed {computed:#06x}, the file stores {stored:#06x}"
            ),
            ObjectError::Bits(e) => e.fmt(f),
            ObjectError::NotRead(v) => write!(f, "the objects of {v} are not read by this version"),
        }
    }
}

impl std::error::Error for ObjectError {}

impl From<BitError> for ObjectError {
    fn from(e: BitError) -> Self {
        ObjectError::Bits(e)
    }
}

impl From<crate::bits::EndOfData> for ObjectError {
    fn from(e: crate::bits::EndOfData) -> Self {
        ObjectError::Bits(BitError::End(e))
    }
}

/// Reads the object map from the assembled `AcDb:Handles` section.
///
/// Entries come back in file order. Handles must strictly increase, and every
/// block's checksum must hold.
pub fn read_object_map(section: &[u8]) -> Result<Vec<ObjectEntry>, ObjectError> {
    let mut entries = Vec::new();
    let mut at = 0usize;
    let mut handle = 0u64;
    let mut offset = 0i64;
    loop {
        if section.len() < at + 2 {
            return Err(ObjectError::Unexpected { what: "map block" });
        }
        // The block size counts its own two bytes, is big-endian, and is
        // followed by the entries and a big-endian checksum.
        let size = usize::from(u16::from_be_bytes([section[at], section[at + 1]]));
        if size == 2 {
            // The empty closing block.
            if section.len() < at + 4 {
                return Err(ObjectError::Unexpected {
                    what: "closing block",
                });
            }
            let stored = u16::from_be_bytes([section[at + 2], section[at + 3]]);
            let computed = crc16(CRC_SEED, &section[at..at + 2]);
            if computed != stored {
                return Err(ObjectError::Checksum {
                    what: "closing object map block",
                    computed,
                    stored,
                });
            }
            return Ok(entries);
        }
        // The specification cuts blocks off at 2032 bytes; real files hold blocks of
        // 2033, so the size is bounded by the data that is there, not by that number.
        if size < 4 || section.len() < at + size + 2 {
            return Err(ObjectError::Unexpected { what: "block size" });
        }
        let stored = u16::from_be_bytes([section[at + size], section[at + size + 1]]);
        let computed = crc16(CRC_SEED, &section[at..at + size]);
        if computed != stored {
            return Err(ObjectError::Checksum {
                what: "object map block",
                computed,
                stored,
            });
        }
        let mut r = BitReader::new(&section[at + 2..at + size]);
        while r.remaining() >= 8 {
            let step = r.modular_char()?;
            if step == 0 {
                return Err(ObjectError::Unexpected {
                    what: "handle step",
                });
            }
            handle = handle
                .checked_add(step)
                .ok_or(ObjectError::Unexpected { what: "handle" })?;
            offset = offset
                .checked_add(r.modular_char_signed()?)
                .ok_or(ObjectError::Unexpected { what: "offset" })?;
            let at_offset = u64::try_from(offset).map_err(|_| ObjectError::Unexpected {
                what: "negative offset",
            })?;
            entries.push(ObjectEntry {
                handle,
                offset: at_offset,
            });
        }
        at += size + 2;
    }
}

/// Reads the frame of the object at `entry` in the assembled
/// `AcDb:AcDbObjects` section of a file of `version`.
pub fn read_object_frame(
    objects: &[u8],
    entry: ObjectEntry,
    version: Version,
) -> Result<ObjectFrame, ObjectError> {
    let from_r2010 = match version {
        Version::R2004 => false,
        Version::R2010 | Version::R2013 | Version::R2018 => true,
        v => return Err(ObjectError::NotRead(v)),
    };
    let start = usize::try_from(entry.offset).map_err(|_| ObjectError::Unexpected {
        what: "object offset",
    })?;
    let tail = objects.get(start..).ok_or(ObjectError::Unexpected {
        what: "object offset",
    })?;
    let mut r = BitReader::new(tail);
    let size = r.modular_short()?;
    let size = u32::try_from(size).map_err(|_| ObjectError::Unexpected {
        what: "object size",
    })?;
    if from_r2010 {
        // The size of the handle stream, in bits. It is written before the
        // object proper and is not part of the size counted above.
        r.modular_char()?;
    }
    // The size and (from R2010) the handle-stream size are whole bytes.
    let prefix = r.position() / 8;
    let type_code = if from_r2010 {
        r.object_type()?
    } else {
        r.bit_short()? as u16
    };
    // The checksum follows the counted bytes and covers everything from the
    // start of the object.
    let covered = prefix
        .checked_add(size as usize)
        .ok_or(ObjectError::Unexpected {
            what: "object size",
        })?;
    let span = covered + 2;
    if tail.len() < span {
        return Err(ObjectError::Unexpected {
            what: "object size",
        });
    }
    let stored = u16::from_le_bytes([tail[covered], tail[covered + 1]]);
    let computed = crc16(CRC_SEED, &tail[..covered]);
    if computed != stored {
        return Err(ObjectError::Checksum {
            what: "object",
            computed,
            stored,
        });
    }
    Ok(ObjectFrame {
        handle: entry.handle,
        type_code,
        size,
        span,
    })
}
