//! The class definitions: the table that names the application-defined
//! object types a drawing uses.
//!
//! The `AcDb:Classes` section of an R2004-family file opens with a sentinel,
//! a byte count and the class data, closes with a 16-bit checksum and a second
//! sentinel. R2004 stores class names as plain text inside the bit stream;
//! R2010 and later move every name into a separate *string stream* at the end
//! of the data, and the main stream and the string stream must meet exactly.

use std::fmt;

use crate::bits::{BitError, BitReader, EndOfData};
use crate::checksum::crc16;
use crate::version::Version;

const OPENING_SENTINEL: [u8; 16] = [
    0x8D, 0xA1, 0xC4, 0xB8, 0xC4, 0xA9, 0xF8, 0xC5, 0xC0, 0xDC, 0xF4, 0x5F, 0xE7, 0xCF, 0xB6, 0x8A,
];
const CLOSING_SENTINEL: [u8; 16] = [
    0x72, 0x5E, 0x3B, 0x47, 0x3B, 0x56, 0x07, 0x3A, 0x3F, 0x23, 0x0B, 0xA0, 0x18, 0x30, 0x49, 0x75,
];

/// The seed of the checksum that covers the byte count and the class data.
const CRC_SEED: u16 = 0xC0C1;

/// One class definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassDef {
    /// The class number (500 and up for application classes).
    pub number: i16,
    /// What an application that does not know the class may do with its objects.
    pub proxy_flags: i16,
    /// The name of the application that defined the class.
    pub app_name: String,
    /// The name of the class in the application's code.
    pub cplusplus_name: String,
    /// The name of the class in DXF files.
    pub dxf_name: String,
    /// Whether the class was written for objects that were not understood.
    pub was_a_zombie: bool,
    /// `0x1F2` for classes that produce entities, `0x1F3` for objects.
    pub item_class_id: i16,
    /// How many objects of the class the drawing holds, as the file states
    /// it; `None` in R2000, which does not store it. (Later files do not
    /// always keep it up to date.)
    pub instance_count: Option<i32>,
    /// The version of the application that defined the class (in R2000 a
    /// field the format does not fully explain).
    pub dwg_version: Option<i16>,
    /// The maintenance release of that version; `None` in R2000.
    pub maintenance_version: Option<i16>,
}

/// Why the class definitions could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClassError {
    /// The section is not laid out the way its version documents.
    Unexpected {
        /// What was wrong.
        what: &'static str,
    },
    /// The checksum does not match.
    Checksum {
        /// The value computed from the bytes.
        computed: u16,
        /// The value the section stores.
        stored: u16,
    },
    /// A value could not be read from the bit stream.
    Bits(BitError),
    /// A name is not text this reader can give: bytes that are not ASCII in a
    /// plain-text name, or UTF-16 that is not valid.
    Text,
}

impl fmt::Display for ClassError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClassError::Unexpected { what } => {
                write!(f, "the class section's {what} is not what the format documents")
            }
            ClassError::Checksum { computed, stored } => write!(
                f,
                "class section checksum mismatch: computed {computed:#06x}, the file stores {stored:#06x}"
            ),
            ClassError::Bits(e) => e.fmt(f),
            ClassError::Text => write!(f, "a class name is not valid text"),
        }
    }
}

impl std::error::Error for ClassError {}

impl From<BitError> for ClassError {
    fn from(e: BitError) -> Self {
        ClassError::Bits(e)
    }
}

impl From<EndOfData> for ClassError {
    fn from(e: EndOfData) -> Self {
        ClassError::Bits(BitError::End(e))
    }
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Reads the class definitions from the assembled `AcDb:Classes` section.
pub fn read_classes(section: &[u8], version: Version) -> Result<Vec<ClassDef>, ClassError> {
    // `unicode`: the strings are in a string stream (R2007 and later);
    // `extra_count`: a second count precedes the data (R2010 and later).
    let (unicode, r2000, extra_count) = match version {
        Version::R2000 => (false, true, false),
        Version::R2004 => (false, false, false),
        Version::R2007 => (true, false, false),
        Version::R2010 | Version::R2013 | Version::R2018 => (true, false, true),
    };
    // Sentinel and byte count.
    if section.len() < 20 || section[..16] != OPENING_SENTINEL {
        return Err(ClassError::Unexpected {
            what: "opening sentinel",
        });
    }
    let size = u32_at(section, 16) as usize;
    // From R2010 a second count precedes the data; it is zero in every file read.
    let region_at = if extra_count { 24 } else { 20 };
    if section.len() < region_at {
        return Err(ClassError::Unexpected { what: "length" });
    }
    if extra_count && u32_at(section, 20) != 0 {
        return Err(ClassError::Unexpected {
            what: "second count",
        });
    }
    let crc_at = region_at
        .checked_add(size)
        .ok_or(ClassError::Unexpected { what: "length" })?;
    // The checksum, the closing sentinel and the trailing eight bytes.
    if section.len() < crc_at + 2 + 16 {
        return Err(ClassError::Unexpected { what: "length" });
    }
    let stored = u16::from_le_bytes([section[crc_at], section[crc_at + 1]]);
    let computed = crc16(CRC_SEED, &section[16..crc_at]);
    if computed != stored {
        return Err(ClassError::Checksum { computed, stored });
    }
    if section[crc_at + 2..crc_at + 18] != CLOSING_SENTINEL {
        return Err(ClassError::Unexpected {
            what: "closing sentinel",
        });
    }

    let region = &section[region_at..crc_at];
    let mut main = BitReader::new(region);
    // The end of the main stream, and (R2010+) where the string stream starts.
    let (main_end, mut strings) = if unicode {
        let total_bits = main.raw_long()? as usize;
        if total_bits > region.len() * 8 || total_bits < 33 {
            return Err(ClassError::Unexpected { what: "bit count" });
        }
        let (start, stream) = locate_string_stream(region, total_bits)?;
        (start, Some(stream))
    } else {
        (region.len() * 8, None)
    };

    if !r2000 {
        // Maximum class number, two zero bytes and a true bit.
        let _max_class = main.bit_short()?;
        if main.raw_char()? != 0 || main.raw_char()? != 0 {
            return Err(ClassError::Unexpected {
                what: "reserved bytes",
            });
        }
        if !main.bit()? {
            return Err(ClassError::Unexpected {
                what: "reserved flag",
            });
        }
    }

    let mut classes = Vec::new();
    // A byte-aligned end leaves up to seven padding bits.
    while main_end.saturating_sub(main.position()) >= 8 {
        let number = main.bit_short()?;
        // The second short: proxy flags in R2004 and later, a version in R2000.
        let second_short = main.bit_short()?;
        let (app_name, cplusplus_name, dxf_name) = match strings.as_mut() {
            Some(s) => (read_unicode(s)?, read_unicode(s)?, read_unicode(s)?),
            None => (
                read_plain(&mut main)?,
                read_plain(&mut main)?,
                read_plain(&mut main)?,
            ),
        };
        let was_a_zombie = main.bit()?;
        let item_class_id = main.bit_short()?;
        let (proxy_flags, instance_count, dwg_version, maintenance_version) = if r2000 {
            (0, None, Some(second_short), None)
        } else {
            let instance_count = main.bit_long()?;
            let dwg_version = main.bit_short()?;
            let maintenance_version = main.bit_short()?;
            main.bit_long()?; // unknown, zero in every file read
            main.bit_long()?; // unknown, zero in every file read
            (
                second_short,
                Some(instance_count),
                Some(dwg_version),
                Some(maintenance_version),
            )
        };
        classes.push(ClassDef {
            number,
            proxy_flags,
            app_name,
            cplusplus_name,
            dxf_name,
            was_a_zombie,
            item_class_id,
            instance_count,
            dwg_version,
            maintenance_version,
        });
    }
    // The main stream must end exactly where its data does (or in the padding).
    if main.position() > main_end {
        return Err(ClassError::Unexpected { what: "stream end" });
    }
    Ok(classes)
}

/// Finds the string stream of data that ends at bit `end` and returns the bit
/// position where the main stream must stop, and a reader on the strings.
fn locate_string_stream(region: &[u8], end: usize) -> Result<(usize, BitReader<'_>), ClassError> {
    let mut probe = BitReader::new(region);
    // The last bit says whether a string stream is present.
    probe.seek(end - 1);
    if !probe.bit()? {
        return Err(ClassError::Unexpected {
            what: "string stream flag",
        });
    }
    let mut at = end - 1;
    if at < 16 {
        return Err(ClassError::Unexpected {
            what: "string stream size",
        });
    }
    at -= 16;
    probe.seek(at);
    let mut size = u32::from(probe.raw_short()?);
    if size & 0x8000 != 0 {
        if at < 16 {
            return Err(ClassError::Unexpected {
                what: "string stream size",
            });
        }
        at -= 16;
        probe.seek(at);
        let high = u32::from(probe.raw_short()?);
        size = (size & 0x7FFF) | (high << 15);
    }
    let start = at
        .checked_sub(size as usize)
        .ok_or(ClassError::Unexpected {
            what: "string stream size",
        })?;
    let mut strings = BitReader::new(region);
    strings.seek(start);
    Ok((start, strings))
}

/// A plain-text name: a bitshort length and that many ASCII bytes.
fn read_plain(r: &mut BitReader<'_>) -> Result<String, ClassError> {
    let len = r.bit_short()?;
    let len = usize::try_from(len).map_err(|_| ClassError::Text)?;
    let mut bytes = Vec::with_capacity(len);
    for _ in 0..len {
        bytes.push(r.raw_char()?);
    }
    while bytes.last() == Some(&0) {
        bytes.pop();
    }
    if !bytes.is_ascii() {
        return Err(ClassError::Text);
    }
    String::from_utf8(bytes).map_err(|_| ClassError::Text)
}

/// A Unicode name: a bitshort character count and two bytes per character.
fn read_unicode(r: &mut BitReader<'_>) -> Result<String, ClassError> {
    let len = r.bit_short()?;
    let len = usize::try_from(len).map_err(|_| ClassError::Text)?;
    let mut units = Vec::with_capacity(len);
    for _ in 0..len {
        units.push(r.raw_short()?);
    }
    while units.last() == Some(&0) {
        units.pop();
    }
    String::from_utf16(&units).map_err(|_| ClassError::Text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_section_without_the_opening_sentinel_is_refused() {
        let bytes = vec![0u8; 64];
        assert_eq!(
            read_classes(&bytes, Version::R2004),
            Err(ClassError::Unexpected {
                what: "opening sentinel"
            })
        );
    }

    #[test]
    fn a_checksum_that_does_not_match_is_refused() {
        // An empty class area: sentinel, size 0, a checksum that is wrong, the closing sentinel.
        let mut bytes = OPENING_SENTINEL.to_vec();
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0x1234u16.to_le_bytes());
        bytes.extend_from_slice(&CLOSING_SENTINEL);
        bytes.extend_from_slice(&[0u8; 8]);
        assert!(matches!(
            read_classes(&bytes, Version::R2004),
            Err(ClassError::Checksum { stored: 0x1234, .. })
        ));
    }

    #[test]
    fn a_section_that_stops_before_its_stated_size_is_refused() {
        let mut bytes = OPENING_SENTINEL.to_vec();
        bytes.extend_from_slice(&1000u32.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 40]);
        assert_eq!(
            read_classes(&bytes, Version::R2004),
            Err(ClassError::Unexpected { what: "length" })
        );
    }
}
