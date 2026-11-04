//! The table of contents of an R2004-family file: the section page map and
//! the section map.
//!
//! The file header says where two *system* pages are. The first, the
//! **section page map**, lists every page in the file in order, with its
//! size; a page's position in the file is the sum of the sizes before it,
//! starting at 0x100. The second, the **section map**, lists the named
//! sections (`AcDb:Header`, `AcDb:AcDbObjects`, ...) and, for each, the
//! pages it is made of. Both are compressed and checksummed.
//!
//! Data pages begin with a 32-byte header masked with a value that depends on
//! the page's own position; [`read_data_page`] unmasks it and checks the
//! header and payload checksums.

use std::fmt;

use crate::checksum::page_checksum;
use crate::header::{HeaderR2004, SECTION_PAGE_MAP_BIAS};
use crate::lz::{decompress, LzError};

const SYSTEM_PAGE_HEADER: usize = 0x14;
const PAGE_MAP_TYPE: u32 = 0x4163_0E3B;
const SECTION_MAP_TYPE: u32 = 0x4163_003B;
const DATA_PAGE_TYPE: u32 = 0x4163_043B;
const DATA_PAGE_HEADER: usize = 0x20;

/// The largest decompressed system page that is accepted (the pages hold a
/// table of contents; a larger claim is a damaged or hostile file).
const MAX_SYSTEM_PAGE: u32 = 1 << 26;

/// Why a table of contents or a page could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SectionError {
    /// The file ends before what the header or a page says is there.
    Truncated {
        /// The offset the read needed to reach.
        needed: u64,
        /// The length of the file.
        len: usize,
    },
    /// A checksum did not match what the page stores.
    Checksum {
        /// Which checksum.
        what: &'static str,
        /// The value computed from the bytes.
        computed: u32,
        /// The value the file stores.
        stored: u32,
    },
    /// A field holds something the format does not document.
    Unexpected {
        /// Which field.
        what: &'static str,
    },
    /// The compressed payload could not be decompressed.
    Compression(LzError),
    /// No page-map entry carries the number the header names.
    MissingPage(i64),
}

impl fmt::Display for SectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SectionError::Truncated { needed, len } => {
                write!(
                    f,
                    "a page needs bytes up to {needed:#x}; the file has {len}"
                )
            }
            SectionError::Checksum {
                what,
                computed,
                stored,
            } => write!(
                f,
                "{what} checksum mismatch: computed {computed:#x}, the file stores {stored:#x}"
            ),
            SectionError::Unexpected { what } => {
                write!(f, "a page's {what} is not what the format documents")
            }
            SectionError::Compression(e) => e.fmt(f),
            SectionError::MissingPage(n) => write!(f, "the page map has no page numbered {n}"),
        }
    }
}

impl std::error::Error for SectionError {}

impl From<LzError> for SectionError {
    fn from(e: LzError) -> Self {
        SectionError::Compression(e)
    }
}

/// One entry of the section page map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageMapEntry {
    /// The page number; negative for a gap (unused space).
    pub number: i32,
    /// The size of the page (or gap) in the file.
    pub size: u32,
    /// The offset of the page in the file: 0x100 plus the sizes before it.
    pub address: u64,
}

/// One page of a section, as the section map lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionPage {
    /// The page number: an index into the page map, starting at 1.
    pub page_number: u32,
    /// The bytes of payload stored after the page header.
    pub data_size: u32,
    /// Where the page's content begins in the decompressed section.
    pub start_offset: u64,
}

/// One section, as the section map describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionDescription {
    /// The size of the section once decompressed and assembled.
    pub size: u64,
    /// The number of pages written to the file (all-zero pages are not).
    pub page_count: u32,
    /// The decompressed size of a full page of this section.
    pub max_decompressed_size: u32,
    /// Whether the pages are compressed (`2`) or stored (`1`).
    pub compressed: u32,
    /// The section id.
    pub id: u32,
    /// Whether the section is encrypted (`0` no, `1` yes, `2` unknown).
    pub encrypted: u32,
    /// The section name (`AcDb:Header`, ...); empty for the first section.
    pub name: String,
    /// The section's pages, in the order the map lists them.
    pub pages: Vec<SectionPage>,
}

/// Where everything is: the page map and the named sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionDirectory {
    /// The pages of the file, in order.
    pub page_map: Vec<PageMapEntry>,
    /// The sections, in the order the section map lists them.
    pub sections: Vec<SectionDescription>,
}

impl SectionDirectory {
    /// The page-map entry with page number `number`.
    pub fn page(&self, number: u32) -> Option<&PageMapEntry> {
        self.page_map
            .iter()
            .find(|p| i64::from(p.number) == i64::from(number))
    }

    /// The section named `name`.
    pub fn section(&self, name: &str) -> Option<&SectionDescription> {
        self.sections.iter().find(|s| s.name == name)
    }
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut w = [0u8; 8];
    w.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(w)
}

fn slice(bytes: &[u8], at: u64, len: u64) -> Result<&[u8], SectionError> {
    let end = at.checked_add(len).ok_or(SectionError::Truncated {
        needed: u64::MAX,
        len: bytes.len(),
    })?;
    if end > bytes.len() as u64 {
        return Err(SectionError::Truncated {
            needed: end,
            len: bytes.len(),
        });
    }
    Ok(&bytes[at as usize..end as usize])
}

/// Reads and decompresses the system page at `at`, which must be of `kind`.
fn read_system_page(bytes: &[u8], at: u64, kind: u32) -> Result<Vec<u8>, SectionError> {
    let head = slice(bytes, at, SYSTEM_PAGE_HEADER as u64)?;
    if u32_at(head, 0) != kind {
        return Err(SectionError::Unexpected {
            what: "system page type",
        });
    }
    let decompressed = u32_at(head, 4);
    let compressed = u32_at(head, 8);
    if u32_at(head, 0x0C) != 2 {
        return Err(SectionError::Unexpected {
            what: "system page compression type",
        });
    }
    let stored = u32_at(head, 0x10);
    if decompressed > MAX_SYSTEM_PAGE {
        return Err(SectionError::Unexpected {
            what: "system page decompressed size",
        });
    }
    let data = slice(bytes, at + SYSTEM_PAGE_HEADER as u64, u64::from(compressed))?;

    // The checksum is taken in two stages: the header with its own checksum
    // field zeroed, then the compressed data seeded with the first result.
    let mut zeroed = [0u8; SYSTEM_PAGE_HEADER];
    zeroed.copy_from_slice(head);
    zeroed[0x10..0x14].fill(0);
    let computed = page_checksum(page_checksum(0, &zeroed), data);
    if computed != stored {
        return Err(SectionError::Checksum {
            what: "system page",
            computed,
            stored,
        });
    }
    Ok(decompress(data, decompressed as usize)?)
}

fn parse_page_map(data: &[u8]) -> Result<Vec<PageMapEntry>, SectionError> {
    let mut entries = Vec::new();
    let mut at = 0usize;
    let mut address = SECTION_PAGE_MAP_BIAS;
    while at < data.len() {
        if data.len() - at < 8 {
            return Err(SectionError::Unexpected {
                what: "page map length",
            });
        }
        let number = u32_at(data, at) as i32;
        let size = u32_at(data, at + 4);
        at += 8;
        if number < 0 {
            // A gap is followed by four more words: parent, left, right, zero.
            if data.len() - at < 16 {
                return Err(SectionError::Unexpected {
                    what: "page map gap record",
                });
            }
            at += 16;
        }
        entries.push(PageMapEntry {
            number,
            size,
            address,
        });
        address += u64::from(size);
    }
    Ok(entries)
}

fn parse_section_map(data: &[u8]) -> Result<Vec<SectionDescription>, SectionError> {
    const BAD: SectionError = SectionError::Unexpected {
        what: "section map length",
    };
    if data.len() < 0x14 {
        return Err(BAD);
    }
    let count = u32_at(data, 0) as usize;
    let mut at = 0x14usize;
    let mut sections = Vec::new();
    for _ in 0..count {
        if data.len() < at + 0x60 {
            return Err(BAD);
        }
        let size = u64_at(data, at);
        let page_count = u32_at(data, at + 8);
        let max_decompressed_size = u32_at(data, at + 0x0C);
        let compressed = u32_at(data, at + 0x14);
        let id = u32_at(data, at + 0x18);
        let encrypted = u32_at(data, at + 0x1C);
        let raw_name = &data[at + 0x20..at + 0x60];
        let name_len = raw_name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(raw_name.len());
        let name = String::from_utf8_lossy(&raw_name[..name_len]).into_owned();
        at += 0x60;

        let pages_len = (page_count as usize).checked_mul(16).ok_or(BAD)?;
        if data.len() < at + pages_len {
            return Err(BAD);
        }
        let pages = (0..page_count as usize)
            .map(|i| {
                let p = at + 16 * i;
                SectionPage {
                    page_number: u32_at(data, p),
                    data_size: u32_at(data, p + 4),
                    start_offset: u64_at(data, p + 8),
                }
            })
            .collect();
        at += pages_len;
        sections.push(SectionDescription {
            size,
            page_count,
            max_decompressed_size,
            compressed,
            id,
            encrypted,
            name,
            pages,
        });
    }
    if at != data.len() {
        return Err(SectionError::Unexpected {
            what: "section map length",
        });
    }
    Ok(sections)
}

/// Reads the page map and the section map of an R2004-family file.
pub fn read_directory(
    bytes: &[u8],
    header: &HeaderR2004,
) -> Result<SectionDirectory, SectionError> {
    let map_at = header
        .section_page_map_address
        .checked_add(SECTION_PAGE_MAP_BIAS)
        .ok_or(SectionError::Unexpected {
            what: "page map address",
        })?;
    let page_map = parse_page_map(&read_system_page(bytes, map_at, PAGE_MAP_TYPE)?)?;

    let wanted = i64::from(header.section_map_id);
    let section_map_page = page_map
        .iter()
        .find(|p| i64::from(p.number) == wanted)
        .ok_or(SectionError::MissingPage(wanted))?;
    let sections = parse_section_map(&read_system_page(
        bytes,
        section_map_page.address,
        SECTION_MAP_TYPE,
    )?)?;
    Ok(SectionDirectory { page_map, sections })
}

/// The unmasked header of a data page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataPageHeader {
    /// The number of the section the page belongs to.
    pub section_number: u32,
    /// The bytes of payload stored after the header.
    pub data_size: u32,
    /// The physical size of the page, rounded up to a multiple of 0x20.
    pub page_size: u32,
    /// Where the page's content begins in the decompressed section.
    pub start_offset: u64,
}

/// Reads the data page at `address`: unmasks its header, checks both
/// checksums, and returns the header and the stored payload.
pub fn read_data_page(bytes: &[u8], address: u64) -> Result<(DataPageHeader, &[u8]), SectionError> {
    let raw = slice(bytes, address, DATA_PAGE_HEADER as u64)?;
    let mask = 0x4164_536Bu32 ^ (address as u32);
    let mut head = [0u8; DATA_PAGE_HEADER];
    for i in 0..8 {
        let word = u32_at(raw, 4 * i) ^ mask;
        head[4 * i..4 * i + 4].copy_from_slice(&word.to_le_bytes());
    }
    if u32_at(&head, 0) != DATA_PAGE_TYPE {
        return Err(SectionError::Unexpected {
            what: "data page type",
        });
    }
    let parsed = DataPageHeader {
        section_number: u32_at(&head, 4),
        data_size: u32_at(&head, 8),
        page_size: u32_at(&head, 0x0C),
        start_offset: u64_at(&head, 0x10),
    };
    let header_checksum = u32_at(&head, 0x18);
    let data_checksum = u32_at(&head, 0x1C);

    let data = slice(
        bytes,
        address + DATA_PAGE_HEADER as u64,
        u64::from(parsed.data_size),
    )?;
    let computed_data = page_checksum(0, data);
    if computed_data != data_checksum {
        return Err(SectionError::Checksum {
            what: "data page payload",
            computed: computed_data,
            stored: data_checksum,
        });
    }
    let mut zeroed = head;
    zeroed[0x18..0x1C].fill(0);
    let computed_header = page_checksum(computed_data, &zeroed);
    if computed_header != header_checksum {
        return Err(SectionError::Checksum {
            what: "data page header",
            computed: computed_header,
            stored: header_checksum,
        });
    }
    Ok((parsed, data))
}

/// The largest section that is assembled (a section is held in memory).
const MAX_SECTION: u64 = 1 << 31;

/// Assembles the section named `name`: reads each of its pages, decompresses
/// those that are compressed, and joins the pages' content in the order the
/// section map lists them. `Ok(None)` when the file has no such section.
///
/// Pages the writer left out because they were all zero are put back as
/// zeros, so the result is always exactly the section's stated size. A
/// section the file marks as encrypted is refused, not returned scrambled.
pub fn read_section(
    bytes: &[u8],
    directory: &SectionDirectory,
    name: &str,
) -> Result<Option<Vec<u8>>, SectionError> {
    let Some(section) = directory.section(name) else {
        return Ok(None);
    };
    if section.encrypted == 1 {
        return Err(SectionError::Unexpected {
            what: "encrypted section (not read)",
        });
    }
    if section.size > MAX_SECTION {
        return Err(SectionError::Unexpected {
            what: "section size",
        });
    }
    let size = section.size as usize;
    let page_size = section.max_decompressed_size as usize;
    let mut out: Vec<u8> = Vec::with_capacity(size.min(1 << 24));

    for page in &section.pages {
        let entry = directory
            .page(page.page_number)
            .ok_or(SectionError::MissingPage(i64::from(page.page_number)))?;
        let (_, payload) = read_data_page(bytes, entry.address)?;

        let start = usize::try_from(page.start_offset).map_err(|_| SectionError::Unexpected {
            what: "page start offset",
        })?;
        if start < out.len() || start > size {
            return Err(SectionError::Unexpected {
                what: "page start offset",
            });
        }
        out.resize(start, 0);

        let content: std::borrow::Cow<'_, [u8]> = match section.compressed {
            2 => std::borrow::Cow::Owned(decompress(payload, page_size)?),
            1 => std::borrow::Cow::Borrowed(payload),
            _ => {
                return Err(SectionError::Unexpected {
                    what: "section compression flag",
                })
            }
        };
        let logical = (size - start).min(page_size);
        if content.len() < logical {
            return Err(SectionError::Unexpected {
                what: "stored page shorter than its page size",
            });
        }
        out.extend_from_slice(&content[..logical]);
    }
    out.resize(size, 0);
    Ok(Some(out))
}
