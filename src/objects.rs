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
    /// The object's handle, from the object map; the object states the same one.
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
/// Entries come back in file order. Every block's checksum must hold. Within a
/// block each handle and each position is an increment on the one before; the
/// first of a block is an increment on zero.
pub fn read_object_map(section: &[u8]) -> Result<Vec<ObjectEntry>, ObjectError> {
    let mut entries = Vec::new();
    let mut at = 0usize;
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
        // Each block counts from zero: its first handle and its first position are
        // increments on nothing.
        let mut handle = 0u64;
        let mut offset = 0i64;
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
    open_object(objects, entry, version).map(|(frame, _)| frame)
}

/// Checks the frame of the object at `entry` and returns it with a reader
/// that is positioned just after the type code and cannot read past the
/// object's own bytes.
pub(crate) fn open_object<'a>(
    objects: &'a [u8],
    entry: ObjectEntry,
    version: Version,
) -> Result<(ObjectFrame, BitReader<'a>), ObjectError> {
    let from_r2010 = match version {
        Version::R2000 | Version::R2004 => false,
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
    // The body reader starts here, just after the type.
    let after_type = r.position();
    if !from_r2010 {
        // The size of the object data in bits, not counting the handles.
        r.raw_long()?;
    }
    // The object states its own handle: a check that the map led here.
    let stated = r.handle()?.value;
    if stated != entry.handle {
        return Err(ObjectError::Unexpected {
            what: "object handle",
        });
    }
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
    let mut body = BitReader::new(&tail[..covered]);
    body.seek(after_type);
    Ok((
        ObjectFrame {
            handle: entry.handle,
            type_code,
            size,
            span,
        },
        body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One block of an object map: size, entries, checksum (all big-endian).
    fn block(entries: &[u8]) -> Vec<u8> {
        let size = (entries.len() + 2) as u16;
        let mut b = size.to_be_bytes().to_vec();
        b.extend_from_slice(entries);
        let crc = crc16(CRC_SEED, &b);
        b.extend_from_slice(&crc.to_be_bytes());
        b
    }

    fn closing() -> Vec<u8> {
        block(&[])
    }

    #[test]
    fn increments_add_up_within_a_block() {
        // handle +1 at +4, handle +2 at +50 (one byte each, no continuation).
        let mut map = block(&[0x01, 0x04, 0x02, 0x32]);
        map.extend(closing());
        let entries = read_object_map(&map).unwrap();
        assert_eq!(
            entries,
            vec![
                ObjectEntry {
                    handle: 1,
                    offset: 4
                },
                ObjectEntry {
                    handle: 3,
                    offset: 54
                },
            ]
        );
    }

    #[test]
    fn each_block_counts_its_handles_and_positions_from_zero() {
        let mut map = block(&[0x01, 0x04, 0x01, 0x32]);
        // The second block's first entry is not an increment on the first block's last.
        map.extend(block(&[0x05, 0x0A, 0x01, 0x32]));
        map.extend(closing());
        let entries = read_object_map(&map).unwrap();
        assert_eq!(
            entries[1],
            ObjectEntry {
                handle: 2,
                offset: 54
            }
        );
        assert_eq!(
            entries[2],
            ObjectEntry {
                handle: 5,
                offset: 10
            }
        );
        assert_eq!(
            entries[3],
            ObjectEntry {
                handle: 6,
                offset: 60
            }
        );
    }

    #[test]
    fn a_position_may_step_backwards_but_not_below_zero() {
        // +50, then -40 (0x28 with the 0x40 sign bit).
        let mut ok = block(&[0x01, 0x32, 0x01, 0x68]);
        ok.extend(closing());
        let entries = read_object_map(&ok).unwrap();
        assert_eq!(entries[1].offset, 10);

        // a step of -63 from a position of 4
        let mut bad = block(&[0x01, 0x04, 0x01, 0x7F]);
        bad.extend(closing());
        assert_eq!(
            read_object_map(&bad),
            Err(ObjectError::Unexpected {
                what: "negative offset"
            })
        );
    }

    #[test]
    fn a_block_whose_checksum_is_wrong_is_refused() {
        let mut map = block(&[0x01, 0x04]);
        let n = map.len();
        map[n - 1] ^= 0x01;
        map.extend(closing());
        assert!(matches!(
            read_object_map(&map),
            Err(ObjectError::Checksum { .. })
        ));
    }

    #[test]
    fn a_map_without_the_closing_block_is_refused() {
        let map = block(&[0x01, 0x04]);
        assert_eq!(
            read_object_map(&map),
            Err(ObjectError::Unexpected { what: "map block" })
        );
    }
}
