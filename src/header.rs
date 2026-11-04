//! The file header: where the rest of the file is found.
//!
//! Two layouts exist among the versions this reader reads.
//!
//! * **R2000** keeps a plain header: a code page, then a list of *section
//!   locators* -- a record number, an absolute offset and a size for the
//!   header variables, the class definitions and the object map -- closed by
//!   a 16-bit checksum and a fixed 16-byte sentinel.
//! * **R2004 and later** (R2004, R2010, R2013, R2018) keep a short plain
//!   prefix and a 0x6c-byte block at offset 0x80 that is masked with a fixed
//!   pseudo-random byte sequence. Unmasked, it holds the position of the
//!   page map and the section map, guarded by a 32-bit checksum.
//!
//! R2007 has a third layout (its header is error-corrected) that this reader
//! does not read yet; asking for it is an explicit [`HeaderError::NotYetRead`].
//!
//! Nothing is guessed: a checksum that does not match, a sentinel that is not
//! the documented one, or a block that does not carry the documented file
//! identifier is an error that says what was found.

use std::fmt;

use crate::checksum::{crc16, crc32};
use crate::version::{detect_version, Error as VersionError, Version};

/// One entry of the R2000 section-locator list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionLocator {
    /// The record number: 0 header variables, 1 class definitions, 2 object
    /// map; higher numbers are listed as found.
    pub record: u8,
    /// Absolute offset of the section from the start of the file.
    pub seeker: u32,
    /// Size of the section in bytes.
    pub size: u32,
}

/// The header of an R2000 file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderR2000 {
    /// Absolute offset of the preview image data, as the file states it.
    pub image_seeker: u32,
    /// The code page number the file declares.
    pub code_page: u16,
    /// The section locators, in file order.
    pub locators: Vec<SectionLocator>,
}

/// The unmasked block of an R2004-and-later header.
///
/// Field names follow the specification. Offsets are as stored, except where
/// a field says otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderR2004 {
    /// The code page number the file declares.
    pub code_page: u16,
    /// The maintenance release of the application that wrote the file.
    pub maintenance_version: u8,
    /// Security flags (`0x1` data is encrypted, `0x2` properties are
    /// encrypted, `0x10` signed, `0x20` time-stamped).
    pub security_flags: u32,
    /// Offset of the summary-info page plus the page header size, or 0.
    pub summary_info_address: u32,
    /// Offset of the VBA project, or 0 when there is none.
    pub vba_project_address: u32,
    /// Offset of the preview page plus the page header size.
    pub preview_address: u32,
    /// Root tree node gap.
    pub root_tree_node_gap: u32,
    /// Lowermost left tree node gap.
    pub lowermost_left_tree_node_gap: u32,
    /// Lowermost right tree node gap.
    pub lowermost_right_tree_node_gap: u32,
    /// Id of the last section page.
    pub last_section_page_id: u32,
    /// The end address of the last section page, as the file states it. Real
    /// files often state a value past their own end, so it is not a bound on
    /// anything.
    pub last_section_page_end_address: u64,
    /// Address of the repeated copy of this block near the end of the file.
    pub second_header_address: u64,
    /// Number of gaps.
    pub gap_amount: u32,
    /// Number of section pages.
    pub section_page_amount: u32,
    /// Id of the section page map.
    pub section_page_map_id: u32,
    /// Address of the section page map **as stored**; the page itself is
    /// found [`SECTION_PAGE_MAP_BIAS`] bytes after it.
    pub section_page_map_address: u64,
    /// Id of the section map.
    pub section_map_id: u32,
    /// Size of the section page array.
    pub section_page_array_size: u32,
    /// Size of the gap array.
    pub gap_array_size: u32,
}

/// What is added to [`HeaderR2004::section_page_map_address`] to get the
/// offset of the page map in the file.
pub const SECTION_PAGE_MAP_BIAS: u64 = 0x100;

/// A file header, in the layout its version uses.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FileHeader {
    /// R2000.
    R2000(HeaderR2000),
    /// R2004, R2010, R2013 or R2018.
    R2004(Version, HeaderR2004),
}

/// Why a file header could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HeaderError {
    /// The version bytes are not a version this reader reads.
    Version(VersionError),
    /// The layout of this version is documented but not read yet.
    NotYetRead(Version),
    /// The file ends before the header does.
    Truncated {
        /// How many bytes the header needs.
        needed: usize,
        /// How many bytes there were.
        len: usize,
    },
    /// A checksum did not match what the file stores.
    Checksum {
        /// Which checksum.
        what: &'static str,
        /// The value computed from the bytes.
        computed: u32,
        /// The value the file stores.
        stored: u32,
    },
    /// A field that must hold a documented constant holds something else.
    Unexpected {
        /// Which field.
        what: &'static str,
    },
    /// The R2000 locator list has a length whose checksum constant is not
    /// documented.
    LocatorCount(u32),
}

impl fmt::Display for HeaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HeaderError::Version(e) => e.fmt(f),
            HeaderError::NotYetRead(v) => {
                write!(f, "the file header of {v} is not read by this version")
            }
            HeaderError::Truncated { needed, len } => {
                write!(
                    f,
                    "the file header needs {needed} bytes; the file has {len}"
                )
            }
            HeaderError::Checksum {
                what,
                computed,
                stored,
            } => write!(
                f,
                "{what} checksum mismatch: computed {computed:#x}, the file stores {stored:#x}"
            ),
            HeaderError::Unexpected { what } => {
                write!(
                    f,
                    "the file header's {what} is not what the format documents"
                )
            }
            HeaderError::LocatorCount(n) => {
                write!(
                    f,
                    "the file header lists {n} section locators; 3 to 6 are documented"
                )
            }
        }
    }
}

impl std::error::Error for HeaderError {}

impl From<VersionError> for HeaderError {
    fn from(e: VersionError) -> Self {
        HeaderError::Version(e)
    }
}

/// Reads the file header of `bytes` (the whole file, or a prefix long enough
/// to hold the header).
///
/// The layout is chosen by the version the first six bytes name.
pub fn read_header(bytes: &[u8]) -> Result<FileHeader, HeaderError> {
    match detect_version(bytes)? {
        Version::R2000 => read_r2000(bytes).map(FileHeader::R2000),
        Version::R2007 => Err(HeaderError::NotYetRead(Version::R2007)),
        v @ (Version::R2004 | Version::R2010 | Version::R2013 | Version::R2018) => {
            read_r2004(bytes).map(|h| FileHeader::R2004(v, h))
        }
    }
}

fn need(bytes: &[u8], needed: usize) -> Result<(), HeaderError> {
    if bytes.len() < needed {
        Err(HeaderError::Truncated {
            needed,
            len: bytes.len(),
        })
    } else {
        Ok(())
    }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut w = [0u8; 8];
    w.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(w)
}

/// The sentinel that closes the R2000 header.
const R2000_SENTINEL: [u8; 16] = [
    0x95, 0xA0, 0x4E, 0x28, 0x99, 0x82, 0x1A, 0xE5, 0x5E, 0x41, 0xE0, 0x5F, 0x9D, 0x3A, 0x4D, 0x00,
];

/// The documented most the locator list is known to have.
const R2000_MAX_LOCATORS: u32 = 6;

fn r2000_crc_mask(count: u32) -> Option<u16> {
    match count {
        3 => Some(0xA598),
        4 => Some(0x8101),
        5 => Some(0x3CC4),
        6 => Some(0x8461),
        _ => None,
    }
}

fn read_r2000(bytes: &[u8]) -> Result<HeaderR2000, HeaderError> {
    need(bytes, 0x19)?;
    let count = u32_at(bytes, 0x15);
    if count > R2000_MAX_LOCATORS {
        return Err(HeaderError::LocatorCount(count));
    }
    let mask = r2000_crc_mask(count).ok_or(HeaderError::LocatorCount(count))?;
    let records_end = 0x19 + 9 * count as usize;
    let end = records_end + 2 + R2000_SENTINEL.len();
    need(bytes, end)?;

    let computed = crc16(0, &bytes[..records_end]) ^ mask;
    let stored = u16_at(bytes, records_end);
    if computed != stored {
        return Err(HeaderError::Checksum {
            what: "R2000 file header",
            computed: u32::from(computed),
            stored: u32::from(stored),
        });
    }
    if bytes[records_end + 2..end] != R2000_SENTINEL {
        return Err(HeaderError::Unexpected {
            what: "closing sentinel",
        });
    }

    let locators = (0..count as usize)
        .map(|i| {
            let at = 0x19 + 9 * i;
            SectionLocator {
                record: bytes[at],
                seeker: u32_at(bytes, at + 1),
                size: u32_at(bytes, at + 5),
            }
        })
        .collect();
    Ok(HeaderR2000 {
        image_seeker: u32_at(bytes, 0x0D),
        code_page: u16_at(bytes, 0x13),
        locators,
    })
}

/// Where the masked block starts, and how long it is.
const R2004_BLOCK_AT: usize = 0x80;
const R2004_BLOCK_LEN: usize = 0x6c;

/// The identifier the unmasked block starts with (and the byte after it).
const R2004_FILE_ID: &[u8; 12] = b"AcFssFcAJMB\0";

/// The byte sequence the block is masked with: a linear congruential
/// generator seeded with 1, taking the third byte of each state.
fn mask_byte_sequence() -> [u8; R2004_BLOCK_LEN] {
    let mut seed: u32 = 1;
    let mut out = [0u8; R2004_BLOCK_LEN];
    for b in &mut out {
        seed = seed.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
        *b = (seed >> 16) as u8;
    }
    out
}

fn read_r2004(bytes: &[u8]) -> Result<HeaderR2004, HeaderError> {
    need(bytes, R2004_BLOCK_AT + R2004_BLOCK_LEN)?;
    let mask = mask_byte_sequence();
    let mut block = [0u8; R2004_BLOCK_LEN];
    for (i, b) in block.iter_mut().enumerate() {
        *b = bytes[R2004_BLOCK_AT + i] ^ mask[i];
    }
    if block[..12] != R2004_FILE_ID[..] {
        return Err(HeaderError::Unexpected {
            what: "file identifier",
        });
    }

    let stored = u32_at(&block, 0x68);
    let mut zeroed = block;
    zeroed[0x68..0x6c].fill(0);
    let computed = crc32(0, &zeroed);
    if computed != stored {
        return Err(HeaderError::Checksum {
            what: "R2004 file header block",
            computed,
            stored,
        });
    }

    Ok(HeaderR2004 {
        code_page: u16_at(bytes, 0x13),
        maintenance_version: bytes[0x0B],
        security_flags: u32_at(bytes, 0x18),
        summary_info_address: u32_at(bytes, 0x20),
        vba_project_address: u32_at(bytes, 0x24),
        preview_address: u32_at(bytes, 0x0D),
        root_tree_node_gap: u32_at(&block, 0x18),
        lowermost_left_tree_node_gap: u32_at(&block, 0x1C),
        lowermost_right_tree_node_gap: u32_at(&block, 0x20),
        last_section_page_id: u32_at(&block, 0x28),
        last_section_page_end_address: u64_at(&block, 0x2C),
        second_header_address: u64_at(&block, 0x34),
        gap_amount: u32_at(&block, 0x3C),
        section_page_amount: u32_at(&block, 0x40),
        section_page_map_id: u32_at(&block, 0x50),
        section_page_map_address: u64_at(&block, 0x54),
        section_map_id: u32_at(&block, 0x5C),
        section_page_array_size: u32_at(&block, 0x60),
        gap_array_size: u32_at(&block, 0x64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mask_sequence_starts_as_the_specification_lists_it() {
        let m = mask_byte_sequence();
        assert_eq!(&m[..8], &[0x29, 0x23, 0xBE, 0x84, 0xE1, 0x6C, 0xD6, 0xAE]);
        // The printed sequence is 92 bytes long and ends here; the generator
        // is what covers all 0x6c bytes of the block.
        assert_eq!(&m[88..92], &[0x6B, 0xC4, 0x30, 0xB7]);
    }

    /// A header built the way the specification describes it, field by field.
    fn r2004_file(magic: &[u8; 6], tweak: impl FnOnce(&mut [u8; R2004_BLOCK_LEN])) -> Vec<u8> {
        let mut file = vec![0u8; 0x100];
        file[..6].copy_from_slice(magic);
        file[0x0B] = 7;
        file[0x0D..0x11].copy_from_slice(&0x1234u32.to_le_bytes());
        file[0x13..0x15].copy_from_slice(&30u16.to_le_bytes());
        file[0x20..0x24].copy_from_slice(&0x5678u32.to_le_bytes());

        let mut block = [0u8; R2004_BLOCK_LEN];
        block[..12].copy_from_slice(R2004_FILE_ID);
        block[0x10..0x14].copy_from_slice(&0x6cu32.to_le_bytes());
        block[0x14..0x18].copy_from_slice(&4u32.to_le_bytes());
        block[0x28..0x2C].copy_from_slice(&9u32.to_le_bytes());
        block[0x2C..0x34].copy_from_slice(&0xAAAAu64.to_le_bytes());
        block[0x34..0x3C].copy_from_slice(&0xBBBBu64.to_le_bytes());
        block[0x50..0x54].copy_from_slice(&3u32.to_le_bytes());
        block[0x54..0x5C].copy_from_slice(&0x2000u64.to_le_bytes());
        block[0x5C..0x60].copy_from_slice(&4u32.to_le_bytes());
        tweak(&mut block);
        let crc = crc32(0, &block);
        block[0x68..0x6c].copy_from_slice(&crc.to_le_bytes());

        let mask = mask_byte_sequence();
        for (i, b) in block.iter().enumerate() {
            file[R2004_BLOCK_AT + i] = b ^ mask[i];
        }
        file
    }

    #[test]
    fn an_r2004_block_is_unmasked_and_read_field_by_field() {
        let file = r2004_file(b"AC1032", |_| {});
        let FileHeader::R2004(version, h) = read_header(&file).unwrap() else {
            panic!("not an R2004-family header");
        };
        assert_eq!(version, Version::R2018);
        assert_eq!(h.maintenance_version, 7);
        assert_eq!(h.preview_address, 0x1234);
        assert_eq!(h.summary_info_address, 0x5678);
        assert_eq!(h.code_page, 30);
        assert_eq!(h.last_section_page_id, 9);
        assert_eq!(h.last_section_page_end_address, 0xAAAA);
        assert_eq!(h.second_header_address, 0xBBBB);
        assert_eq!(h.section_page_map_id, 3);
        assert_eq!(h.section_page_map_address, 0x2000);
        assert_eq!(h.section_map_id, 4);
    }

    #[test]
    fn a_flipped_bit_in_the_masked_block_is_a_checksum_error() {
        let mut file = r2004_file(b"AC1018", |_| {});
        file[R2004_BLOCK_AT + 0x30] ^= 0x01;
        assert!(matches!(
            read_header(&file),
            Err(HeaderError::Checksum {
                what: "R2004 file header block",
                ..
            })
        ));
    }

    #[test]
    fn a_block_without_the_file_identifier_is_refused_before_its_checksum() {
        let mut file = r2004_file(b"AC1024", |_| {});
        file[R2004_BLOCK_AT] ^= 0x01;
        assert_eq!(
            read_header(&file),
            Err(HeaderError::Unexpected {
                what: "file identifier"
            })
        );
    }

    #[test]
    fn a_short_r2004_file_says_how_much_it_needed() {
        let file = r2004_file(b"AC1027", |_| {});
        assert_eq!(
            read_header(&file[..0x90]),
            Err(HeaderError::Truncated {
                needed: R2004_BLOCK_AT + R2004_BLOCK_LEN,
                len: 0x90
            })
        );
    }

    /// An R2000 header: three locators, the closing checksum and sentinel.
    fn r2000_file(records: &[(u8, u32, u32)]) -> Vec<u8> {
        let mut file = vec![0u8; 0x19];
        file[..6].copy_from_slice(b"AC1015");
        file[0x0D..0x11].copy_from_slice(&0x400u32.to_le_bytes());
        file[0x13..0x15].copy_from_slice(&30u16.to_le_bytes());
        file[0x15..0x19].copy_from_slice(&(records.len() as u32).to_le_bytes());
        for &(r, s, z) in records {
            file.push(r);
            file.extend_from_slice(&s.to_le_bytes());
            file.extend_from_slice(&z.to_le_bytes());
        }
        let crc = crc16(0, &file) ^ r2000_crc_mask(records.len() as u32).unwrap();
        file.extend_from_slice(&crc.to_le_bytes());
        file.extend_from_slice(&R2000_SENTINEL);
        file
    }

    #[test]
    fn an_r2000_header_lists_its_locators_in_order() {
        let file = r2000_file(&[(0, 0x60, 0x200), (1, 0x260, 0x80), (2, 0x300, 0x40)]);
        let FileHeader::R2000(h) = read_header(&file).unwrap() else {
            panic!("not an R2000 header");
        };
        assert_eq!(h.image_seeker, 0x400);
        assert_eq!(h.code_page, 30);
        assert_eq!(
            h.locators,
            vec![
                SectionLocator {
                    record: 0,
                    seeker: 0x60,
                    size: 0x200
                },
                SectionLocator {
                    record: 1,
                    seeker: 0x260,
                    size: 0x80
                },
                SectionLocator {
                    record: 2,
                    seeker: 0x300,
                    size: 0x40
                },
            ]
        );
    }

    #[test]
    fn an_r2000_header_whose_checksum_or_sentinel_is_off_is_refused() {
        let mut file = r2000_file(&[(0, 1, 2), (1, 3, 4), (2, 5, 6)]);
        let n = file.len();
        file[0x1A] ^= 0x01; // inside the first record
        assert!(matches!(
            read_header(&file),
            Err(HeaderError::Checksum { .. })
        ));
        file[0x1A] ^= 0x01;
        file[n - 1] ^= 0x01; // inside the sentinel
        assert_eq!(
            read_header(&file),
            Err(HeaderError::Unexpected {
                what: "closing sentinel"
            })
        );
    }

    #[test]
    fn an_undocumented_locator_count_is_refused() {
        let mut file = r2000_file(&[(0, 1, 2), (1, 3, 4), (2, 5, 6)]);
        file[0x15] = 2;
        assert_eq!(read_header(&file), Err(HeaderError::LocatorCount(2)));
        file[0x15] = 7;
        assert_eq!(read_header(&file), Err(HeaderError::LocatorCount(7)));
    }

    #[test]
    fn r2007_is_named_as_not_yet_read() {
        assert_eq!(
            read_header(b"AC1021\0\0\0\0\0\0\x01"),
            Err(HeaderError::NotYetRead(Version::R2007))
        );
    }

    #[test]
    fn an_unlisted_version_is_the_version_error_unchanged() {
        assert!(matches!(
            read_header(b"AC1014xxxxxxx"),
            Err(HeaderError::Version(
                VersionError::UnsupportedVersion { .. }
            ))
        ));
    }
}
