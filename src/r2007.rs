//! The container of R2007 files (`AC1021`).
//!
//! R2007 keeps the page and section structure of R2004 but wraps every page
//! in Reed-Solomon error correction and uses a different compressor:
//!
//! - The file header is one 0x400-byte block at 0x80: three interleaved
//!   Reed-Solomon (255, 239) codewords whose data is a small record, itself
//!   compressed.
//! - The page map and the section map are *system pages*: data repeated and
//!   encoded with the same (255, 239) code, interleaved over as many
//!   codewords as the header says.
//! - A section's data pages use a (255, 251) code. A compressed page is
//!   interleaved over `size / 255` codewords; an uncompressed page is its
//!   data followed by the parity of all of it.
//!
//! The parity is *checked*, not used to correct: a codeword whose syndromes
//! are not all zero is an error that names the damage, and so is a page whose
//! 32-bit checksum does not hold. The 64-bit checks in the file header and
//! the section map are not read; the parity and the page checksum already
//! decide whether the bytes are intact.

use crate::checksum::page_sum_r2007;
use crate::header::HeaderError;
use crate::lz::LzError;
use crate::sections::SectionError;

/// Where the file header block starts.
const HEADER_AT: usize = 0x80;
/// How long the file header block is.
const HEADER_LEN: usize = 0x400;
/// Where the pages (the page map first) start; the header stores addresses
/// relative to this.
pub const PAGES_START: u64 = 0x480;

const CODEWORD: usize = 255;
const SYSTEM_DATA: usize = 239;
const SYSTEM_PARITY: usize = 16;
const PAGE_DATA: usize = 251;
const PAGE_PARITY: usize = 4;
/// The file header holds three codewords.
const HEADER_CODEWORDS: usize = 3;
/// The decompressed size of the header record.
const HEADER_RECORD: usize = 0x110;
/// The largest decompressed system page that is accepted.
const MAX_SYSTEM_PAGE: u64 = 1 << 26;
/// The largest section that is accepted.
const MAX_SECTION: u64 = 1 << 31;

// ---------------------------------------------------------------------------
// Reed-Solomon over GF(256): checked, not corrected.

struct Field {
    exp: [u8; 512],
    log: [u8; 256],
}

const fn field(primitive: u16) -> Field {
    let mut exp = [0u8; 512];
    let mut log = [0u8; 256];
    let mut x: u16 = 1;
    let mut i = 0;
    while i < 255 {
        exp[i] = x as u8;
        log[x as usize] = i as u8;
        x <<= 1;
        if x & 0x100 != 0 {
            x ^= primitive;
        }
        i += 1;
    }
    let mut j = 255;
    while j < 512 {
        exp[j] = exp[j - 255];
        j += 1;
    }
    Field { exp, log }
}

/// x^8 + x^6 + x^5 + x^3 + 1: the file header and the system pages.
const SYSTEM_FIELD: Field = field(0x169);
/// x^8 + x^4 + x^3 + x^2 + 1: the data pages.
const PAGE_FIELD: Field = field(0x11D);

/// Whether the 255 bytes are a codeword: the first `parity` powers of the
/// field's generator are all roots. Byte `i` is the coefficient of x^i.
fn codeword_clear(word: &[u8; CODEWORD], field: &Field, parity: usize) -> bool {
    for j in 1..=parity {
        let mut syndrome = 0u8;
        for (i, &c) in word.iter().enumerate() {
            if c != 0 {
                syndrome ^= field.exp[(usize::from(field.log[usize::from(c)]) + j * i) % 255];
            }
        }
        if syndrome != 0 {
            return false;
        }
    }
    true
}

/// The data of `blocks` interleaved codewords (block `b` is the bytes
/// `raw[b]`, `raw[b + blocks]`, ...): the first `data` bytes of each,
/// joined. `Err(b)` names the first block whose parity does not hold.
fn extract_interleaved(
    raw: &[u8],
    blocks: usize,
    data: usize,
    field: &Field,
    parity: usize,
) -> Result<Vec<u8>, usize> {
    let mut out = Vec::with_capacity(blocks * data);
    let mut word = [0u8; CODEWORD];
    for b in 0..blocks {
        for (i, w) in word.iter_mut().enumerate() {
            *w = raw[i * blocks + b];
        }
        if !codeword_clear(&word, field, parity) {
            return Err(b);
        }
        out.extend_from_slice(&word[..data]);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// The decompressor.

/// How the compressor lays out the bytes of a literal run of 1 to 32 bytes:
/// `(size, at)` takes `size` bytes from offset `at` of the run, in this
/// order. Every block is itself laid out the same way (a block of two
/// bytes comes out reversed, a block of sixteen with its halves swapped).
const LAYOUT: [&[(u8, u8)]; 32] = [
    &[(1, 0)],
    &[(1, 1), (1, 0)],
    &[(1, 2), (1, 1), (1, 0)],
    &[(1, 0), (1, 1), (1, 2), (1, 3)],
    &[(1, 4), (4, 0)],
    &[(1, 5), (4, 1), (1, 0)],
    &[(2, 5), (4, 1), (1, 0)],
    &[(4, 0), (4, 4)],
    &[(1, 8), (8, 0)],
    &[(1, 9), (8, 1), (1, 0)],
    &[(2, 9), (8, 1), (1, 0)],
    &[(4, 8), (8, 0)],
    &[(1, 12), (4, 8), (8, 0)],
    &[(1, 13), (4, 9), (8, 1), (1, 0)],
    &[(2, 13), (4, 9), (8, 1), (1, 0)],
    &[(8, 8), (8, 0)],
    &[(8, 9), (1, 8), (8, 0)],
    &[(1, 17), (16, 1), (1, 0)],
    &[(3, 16), (16, 0)],
    &[(4, 16), (16, 0)],
    &[(1, 20), (4, 16), (16, 0)],
    &[(2, 20), (4, 16), (16, 0)],
    &[(3, 20), (4, 16), (16, 0)],
    &[(8, 16), (16, 0)],
    &[(8, 17), (1, 16), (16, 0)],
    &[(1, 25), (8, 17), (1, 16), (16, 0)],
    &[(2, 25), (8, 17), (1, 16), (16, 0)],
    &[(4, 24), (8, 16), (16, 0)],
    &[(1, 28), (4, 24), (8, 16), (16, 0)],
    &[(2, 28), (4, 24), (8, 16), (16, 0)],
    &[(1, 30), (4, 26), (8, 18), (16, 2), (2, 0)],
    &[(16, 16), (16, 0)],
];

fn lay_out(chunk: &[u8], out: &mut Vec<u8>) {
    let n = chunk.len();
    if n <= 1 {
        out.extend_from_slice(chunk);
        return;
    }
    for &(size, at) in LAYOUT[n - 1] {
        let part = &chunk[usize::from(at)..usize::from(at) + usize::from(size)];
        lay_out(part, out);
    }
}

/// Appends a literal run: blocks of 32 bytes first, then the remainder.
fn push_literal(run: &[u8], out: &mut Vec<u8>) {
    let mut rest = run;
    while rest.len() >= 32 {
        lay_out(&rest[..32], out);
        rest = &rest[32..];
    }
    if !rest.is_empty() {
        lay_out(rest, out);
    }
}

/// Decompresses `src` to exactly `expected` bytes.
///
/// A literal run is followed by copies from earlier in the output, each
/// opcode also carrying the length of the literal run that follows it. A copy
/// from before the start of the output, output past `expected` and a stream
/// that stops short of it are errors.
fn decompress(src: &[u8], expected: usize) -> Result<Vec<u8>, LzError> {
    let mut out: Vec<u8> = Vec::with_capacity(expected.min(1 << 24));
    let get = |i: usize| src.get(i).copied().ok_or(LzError::Truncated);
    let mut si = 0usize;
    let mut opcode = get(si)?;
    si += 1;
    let mut length = 0usize;
    if opcode >> 4 == 2 {
        si += 2;
        length = usize::from(get(si)? & 7);
        si += 1;
    }
    while si < src.len() {
        // A literal run: its length is the one the last opcode carried, or
        // (when that was zero) comes from the opcode that starts the run.
        if length == 0 {
            length = usize::from(opcode) + 8;
            if length == 0x17 {
                let n = usize::from(get(si)?);
                si += 1;
                length += n;
                if n == 0xFF {
                    loop {
                        let n = usize::from(get(si)?) | (usize::from(get(si + 1)?) << 8);
                        si += 2;
                        length += n;
                        if n != 0xFFFF {
                            break;
                        }
                    }
                }
            }
        }
        let run = src.get(si..si + length).ok_or(LzError::Truncated)?;
        if out.len() + length > expected {
            return Err(LzError::Overrun { expected });
        }
        push_literal(run, &mut out);
        si += length;
        if si >= src.len() {
            break;
        }
        opcode = get(si)?;
        si += 1;
        // One or more copies from the output.
        loop {
            let (distance, count);
            match opcode >> 4 {
                0 => {
                    let low = usize::from(get(si)?);
                    si += 1;
                    let n = usize::from(opcode & 0xF) + 0x13;
                    opcode = get(si)?;
                    si += 1;
                    count = ((usize::from(opcode) >> 3) & 0x10) + n;
                    distance = ((usize::from(opcode) & 0x78) << 5) + 1 + low;
                }
                1 => {
                    let low = usize::from(get(si)?);
                    si += 1;
                    let n = usize::from(opcode & 0xF) + 3;
                    opcode = get(si)?;
                    si += 1;
                    count = n;
                    distance = ((usize::from(opcode) & 0xF8) << 5) + 1 + low;
                }
                2 => {
                    let mut d = usize::from(get(si)?);
                    si += 1;
                    d |= usize::from(get(si)?) << 8;
                    si += 1;
                    let n = usize::from(opcode & 7);
                    if opcode & 8 == 0 {
                        opcode = get(si)?;
                        si += 1;
                        count = (usize::from(opcode) & 0xF8) + n;
                        distance = d;
                    } else {
                        let mid = usize::from(get(si)?);
                        si += 1;
                        opcode = get(si)?;
                        si += 1;
                        count = (((usize::from(opcode) & 0xF8) << 8) + (mid << 3) + n) + 0x100;
                        distance = d + 1;
                    }
                }
                _ => {
                    let n = usize::from(opcode >> 4);
                    let low = usize::from(opcode & 15);
                    opcode = get(si)?;
                    si += 1;
                    count = n;
                    distance = ((usize::from(opcode) & 0xF8) << 1) + low + 1;
                }
            }
            if distance > out.len() {
                return Err(LzError::CopyBeforeStart {
                    distance,
                    produced: out.len(),
                });
            }
            if out.len() + count > expected {
                return Err(LzError::Overrun { expected });
            }
            for _ in 0..count {
                let byte = out[out.len() - distance];
                out.push(byte);
            }
            length = usize::from(opcode & 7);
            if length != 0 || si >= src.len() {
                break;
            }
            opcode = get(si)?;
            si += 1;
            if opcode >> 4 == 0 {
                break;
            }
            if opcode >> 4 == 15 {
                opcode &= 15;
            }
        }
    }
    if out.len() != expected {
        return Err(LzError::Short {
            produced: out.len(),
            expected,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// The file header.

/// The file header of an R2007 file: what the error-corrected block says about
/// where the page map and the section map are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderR2007 {
    /// The code page number the file declares.
    pub code_page: u16,
    /// The length of the file the header states.
    pub file_size: u64,
    /// Where the page map is, **relative to** [`PAGES_START`].
    pub page_map_address: u64,
    /// The size of the page map as stored (compressed), in bytes.
    pub page_map_compressed_size: u64,
    /// The size of the page map once decompressed.
    pub page_map_size: u64,
    /// How many times the page map's data is repeated in its page.
    pub page_map_factor: u64,
    /// The largest page number the page map uses.
    pub max_page_id: u64,
    /// The page number of the section map.
    pub section_map_id: u64,
    /// The size of the section map as stored (compressed), in bytes.
    pub section_map_compressed_size: u64,
    /// The size of the section map once decompressed.
    pub section_map_size: u64,
    /// How many times the section map's data is repeated in its page.
    pub section_map_factor: u64,
    /// The number of sections plus one.
    pub section_count: u64,
    /// The stream version (normally 0x60100).
    pub stream_version: u64,
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut w = [0u8; 8];
    w.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(w)
}

fn i32_at(b: &[u8], at: usize) -> i32 {
    i32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Reads the file header of an R2007 file (`bytes` is the whole file, or a
/// prefix that holds the header and the page map).
pub fn read_header_r2007(bytes: &[u8]) -> Result<HeaderR2007, HeaderError> {
    let needed = HEADER_AT + HEADER_LEN;
    if bytes.len() < needed {
        return Err(HeaderError::Truncated {
            needed,
            len: bytes.len(),
        });
    }
    let code_page = u16::from_le_bytes([bytes[0x13], bytes[0x14]]);
    let raw = &bytes[HEADER_AT..HEADER_AT + HEADER_CODEWORDS * CODEWORD];
    let block = extract_interleaved(
        raw,
        HEADER_CODEWORDS,
        SYSTEM_DATA,
        &SYSTEM_FIELD,
        SYSTEM_PARITY,
    )
    .map_err(|_| HeaderError::Unexpected {
        what: "Reed-Solomon parity of the file header (the header is damaged)",
    })?;
    // 8 check value, 8 key, 8 data check value, 4 size, 4 second size, data.
    let compressed_size = i32_at(&block, 24);
    let record = if compressed_size < 0 {
        let n = compressed_size.unsigned_abs() as usize;
        block
            .get(32..32 + n)
            .filter(|r| r.len() == HEADER_RECORD)
            .map(<[u8]>::to_vec)
            .ok_or(HeaderError::Unexpected {
                what: "size of the uncompressed file header record",
            })?
    } else {
        let n = compressed_size as usize;
        let data = block.get(32..32 + n).ok_or(HeaderError::Unexpected {
            what: "size of the compressed file header record",
        })?;
        decompress(data, HEADER_RECORD).map_err(HeaderError::Compression)?
    };
    if u64_at(&record, 0) != 0x70 {
        return Err(HeaderError::Unexpected {
            what: "size of the R2007 file header record",
        });
    }
    let file_size = u64_at(&record, 0x08);
    Ok(HeaderR2007 {
        code_page,
        file_size,
        page_map_address: u64_at(&record, 0x38),
        page_map_compressed_size: u64_at(&record, 0x50),
        page_map_size: u64_at(&record, 0x58),
        page_map_factor: u64_at(&record, 0x18),
        max_page_id: u64_at(&record, 0x68),
        section_map_id: u64_at(&record, 0xC0),
        section_map_compressed_size: u64_at(&record, 0xB0),
        section_map_size: u64_at(&record, 0xC8),
        section_map_factor: u64_at(&record, 0xD8),
        section_count: u64_at(&record, 0xA0),
        stream_version: u64_at(&record, 0xE8),
    })
}

// ---------------------------------------------------------------------------
// The page map and the section map.

/// One entry of the page map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageMapEntry2007 {
    /// The page number as stored (negative for a gap).
    pub id: i64,
    /// The size of the page in the file.
    pub size: u64,
    /// Where the page starts, counted from [`PAGES_START`].
    pub offset: u64,
}

/// A page of a section, as the section map lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionPage2007 {
    /// Where the page's data starts within the section.
    pub start_offset: u64,
    /// The size the page was written with (its data area before parity).
    pub page_size: u64,
    /// The page number: the page map's key.
    pub id: u64,
    /// The size of the page's data once decompressed.
    pub size: u64,
    /// The size of the page's data as stored.
    pub compressed_size: u64,
    /// The 32-bit checksum of the decompressed data.
    pub checksum: u64,
}

/// A section of an R2007 file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section2007 {
    /// The section's name.
    pub name: String,
    /// The size of the whole section once assembled.
    pub size: u64,
    /// The largest page size the section uses.
    pub max_page_size: u64,
    /// 1 when the data is encrypted.
    pub encrypted: u64,
    /// How the pages are encoded: 1 (not interleaved) or 4 (interleaved,
    /// compressed when that is shorter).
    pub encoding: u64,
    /// The pages, in the order the section map lists them.
    pub pages: Vec<SectionPage2007>,
}

/// The page map and the section map of an R2007 file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directory2007 {
    /// The page map, in file order.
    pub page_map: Vec<PageMapEntry2007>,
    /// The sections.
    pub sections: Vec<Section2007>,
}

impl Directory2007 {
    /// The page-map entry of page `id`.
    pub fn page(&self, id: u64) -> Option<&PageMapEntry2007> {
        self.page_map
            .iter()
            .find(|p| p.id.unsigned_abs() == id && p.id > 0)
    }

    /// The section called `name`.
    pub fn section(&self, name: &str) -> Option<&Section2007> {
        self.sections.iter().find(|s| s.name == name)
    }
}

fn truncated(needed: u64, len: usize) -> SectionError {
    SectionError::Truncated { needed, len }
}

/// Reads and decodes a system page: `compressed` bytes of data (decompressing
/// to `size`), repeated `factor` times and encoded at `at`.
fn read_system_page(
    bytes: &[u8],
    at: u64,
    compressed: u64,
    size: u64,
    factor: u64,
) -> Result<Vec<u8>, SectionError> {
    if size > MAX_SYSTEM_PAGE || compressed > MAX_SYSTEM_PAGE || factor == 0 || factor > 4096 {
        return Err(SectionError::Unexpected {
            what: "size of a system page",
        });
    }
    let padded = compressed.div_ceil(8) * 8;
    let blocks = (factor * padded).div_ceil(SYSTEM_DATA as u64);
    let end = blocks
        .checked_mul(CODEWORD as u64)
        .and_then(|n| n.checked_add(at))
        .ok_or(SectionError::Unexpected {
            what: "address of a system page",
        })?;
    let raw = usize::try_from(at)
        .ok()
        .zip(usize::try_from(end).ok())
        .and_then(|(a, e)| bytes.get(a..e))
        .ok_or_else(|| truncated(end, bytes.len()))?;
    let data = extract_interleaved(
        raw,
        blocks as usize,
        SYSTEM_DATA,
        &SYSTEM_FIELD,
        SYSTEM_PARITY,
    )
    .map_err(|_| SectionError::Unexpected {
        what: "Reed-Solomon parity of a system page (the page is damaged)",
    })?;
    let (compressed, size) = (compressed as usize, size as usize);
    if compressed < size {
        let body = data.get(..compressed).ok_or(SectionError::Unexpected {
            what: "size of a compressed system page",
        })?;
        Ok(decompress(body, size)?)
    } else {
        data.get(..size)
            .map(<[u8]>::to_vec)
            .ok_or(SectionError::Unexpected {
                what: "size of a system page",
            })
    }
}

fn parse_page_map(data: &[u8]) -> Result<Vec<PageMapEntry2007>, SectionError> {
    let mut entries = Vec::with_capacity(data.len() / 16);
    let mut offset = 0u64;
    for pair in data.as_chunks::<16>().0 {
        let size = u64_at(pair, 0);
        let id = u64_at(pair, 8) as i64;
        entries.push(PageMapEntry2007 { id, size, offset });
        offset = offset.checked_add(size).ok_or(SectionError::Unexpected {
            what: "page map sizes",
        })?;
    }
    Ok(entries)
}

fn parse_section_map(data: &[u8]) -> Result<Vec<Section2007>, SectionError> {
    let mut sections = Vec::new();
    let mut p = 0usize;
    let short = || SectionError::Unexpected {
        what: "section map (it ends inside a record)",
    };
    while p < data.len() {
        let head = data.get(p..p + 0x40).ok_or_else(short)?;
        let (size, max_page_size, encrypted, name_bytes, encoding, page_count) = (
            u64_at(head, 0x00),
            u64_at(head, 0x08),
            u64_at(head, 0x10),
            u64_at(head, 0x20),
            u64_at(head, 0x30),
            u64_at(head, 0x38),
        );
        p += 0x40;
        // The name is UTF-16 and its stored length counts the closing zero.
        let name_len = usize::try_from(name_bytes)
            .ok()
            .filter(|n| n % 2 == 0 && *n <= 512)
            .ok_or(SectionError::Unexpected {
                what: "length of a section name",
            })?;
        let name_data = data.get(p..p + name_len).ok_or_else(short)?;
        p += name_len;
        let units: Vec<u16> = name_data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .take_while(|&u| u != 0)
            .collect();
        let name = String::from_utf16(&units).map_err(|_| SectionError::Unexpected {
            what: "section name text",
        })?;
        let count = usize::try_from(page_count)
            .ok()
            .filter(|n| n.checked_mul(56).is_some_and(|b| b <= data.len() - p))
            .ok_or_else(short)?;
        let mut pages = Vec::with_capacity(count);
        for _ in 0..count {
            let r = &data[p..p + 56];
            pages.push(SectionPage2007 {
                start_offset: u64_at(r, 0),
                page_size: u64_at(r, 8),
                id: u64_at(r, 16),
                size: u64_at(r, 24),
                compressed_size: u64_at(r, 32),
                checksum: u64_at(r, 40),
            });
            p += 56;
        }
        sections.push(Section2007 {
            name,
            size,
            max_page_size,
            encrypted,
            encoding,
            pages,
        });
    }
    Ok(sections)
}

/// Reads the page map and the section map of an R2007 file.
pub fn read_directory_r2007(
    bytes: &[u8],
    header: &HeaderR2007,
) -> Result<Directory2007, SectionError> {
    let map_at =
        PAGES_START
            .checked_add(header.page_map_address)
            .ok_or(SectionError::Unexpected {
                what: "page map address",
            })?;
    let page_map = parse_page_map(&read_system_page(
        bytes,
        map_at,
        header.page_map_compressed_size,
        header.page_map_size,
        header.page_map_factor,
    )?)?;
    let wanted = header.section_map_id;
    let entry = page_map
        .iter()
        .find(|p| p.id > 0 && p.id.unsigned_abs() == wanted)
        .ok_or(SectionError::MissingPage(wanted as i64))?;
    let sections = parse_section_map(&read_system_page(
        bytes,
        PAGES_START + entry.offset,
        header.section_map_compressed_size,
        header.section_map_size,
        header.section_map_factor,
    )?)?;
    Ok(Directory2007 { page_map, sections })
}

// ---------------------------------------------------------------------------
// Data pages and sections.

/// Reads and decodes one data page of `section`.
fn decode_data_page(
    bytes: &[u8],
    directory: &Directory2007,
    section: &Section2007,
    page: &SectionPage2007,
) -> Result<Vec<u8>, SectionError> {
    let entry = directory
        .page(page.id)
        .ok_or(SectionError::MissingPage(page.id as i64))?;
    let start = PAGES_START + entry.offset;
    let end = start
        .checked_add(entry.size)
        .ok_or(SectionError::Unexpected {
            what: "address of a data page",
        })?;
    let raw = usize::try_from(start)
        .ok()
        .zip(usize::try_from(end).ok())
        .and_then(|(a, e)| bytes.get(a..e))
        .ok_or_else(|| truncated(end, bytes.len()))?;
    let size = usize::try_from(page.size)
        .ok()
        .filter(|&n| n as u64 <= MAX_SECTION)
        .ok_or(SectionError::Unexpected {
            what: "size of a data page",
        })?;
    let data = match section.encoding {
        4 => {
            let blocks = raw.len() / CODEWORD;
            if blocks == 0 {
                return Err(SectionError::Unexpected {
                    what: "size of an interleaved data page",
                });
            }
            let body = extract_interleaved(
                &raw[..blocks * CODEWORD],
                blocks,
                PAGE_DATA,
                &PAGE_FIELD,
                PAGE_PARITY,
            )
            .map_err(|_| SectionError::Unexpected {
                what: "Reed-Solomon parity of a data page (the page is damaged)",
            })?;
            let stored = usize::try_from(page.compressed_size).unwrap_or(usize::MAX);
            if stored < size {
                let body = body.get(..stored).ok_or(SectionError::Unexpected {
                    what: "size of a compressed data page",
                })?;
                decompress(body, size)?
            } else {
                body.get(..size)
                    .map(<[u8]>::to_vec)
                    .ok_or(SectionError::Unexpected {
                        what: "size of a data page",
                    })?
            }
        }
        1 => raw
            .get(..size)
            .map(<[u8]>::to_vec)
            .ok_or(SectionError::Unexpected {
                what: "size of an uncompressed data page",
            })?,
        _ => {
            return Err(SectionError::Unexpected {
                what: "page encoding of a section",
            })
        }
    };
    let computed = page_sum_r2007(&data);
    let stored = page.checksum as u32;
    if page.checksum > u64::from(u32::MAX) || computed != stored {
        return Err(SectionError::Checksum {
            what: "R2007 data page",
            computed,
            stored,
        });
    }
    Ok(data)
}

/// Assembles the section named `name`: decodes each page, checks its
/// parity and its checksum, and joins them at the offsets the section map
/// gives, putting back as zeros the pages the writer left out. `Ok(None)`
/// when the file has no such section.
pub fn read_section_r2007(
    bytes: &[u8],
    directory: &Directory2007,
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
    let total = section.size as usize;
    let mut out: Vec<u8> = Vec::with_capacity(total.min(1 << 24));
    for page in &section.pages {
        let at = usize::try_from(page.start_offset).map_err(|_| SectionError::Unexpected {
            what: "page offset within a section",
        })?;
        if at < out.len() || at > total {
            return Err(SectionError::Unexpected {
                what: "page offset within a section",
            });
        }
        out.resize(at, 0);
        out.extend_from_slice(&decode_data_page(bytes, directory, section, page)?);
        if out.len() > total {
            return Err(SectionError::Unexpected {
                what: "pages longer than their section",
            });
        }
    }
    out.resize(total, 0);
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A systematic encoder for the tests, in the file's layout: the data
    /// first (byte `i` is the coefficient of x^i), then `parity` bytes chosen
    /// so that the first `parity` powers of the generator are roots.
    fn encode(data: &[u8], field: &Field, parity: usize) -> [u8; CODEWORD] {
        let mul = |a: u8, b: u8| -> u8 {
            if a == 0 || b == 0 {
                0
            } else {
                field.exp[usize::from(field.log[usize::from(a)])
                    + usize::from(field.log[usize::from(b)])]
            }
        };
        let inv = |a: u8| field.exp[255 - usize::from(field.log[usize::from(a)])];
        let k = CODEWORD - parity;
        assert_eq!(data.len(), k);
        // Row j (1-based): sum_t p_t * a^(j*(k+t)) = sum_i d_i * a^(j*i).
        let mut rows: Vec<Vec<u8>> = Vec::new();
        for j in 1..=parity {
            let mut row: Vec<u8> = (0..parity)
                .map(|t| field.exp[(j * (k + t)) % 255])
                .collect();
            let mut rhs = 0u8;
            for (i, &d) in data.iter().enumerate() {
                if d != 0 {
                    rhs ^= field.exp[(usize::from(field.log[usize::from(d)]) + j * i) % 255];
                }
            }
            row.push(rhs);
            rows.push(row);
        }
        for col in 0..parity {
            let pivot = (col..parity)
                .find(|&r| rows[r][col] != 0)
                .expect("invertible");
            rows.swap(col, pivot);
            let scale = inv(rows[col][col]);
            for x in rows[col].iter_mut() {
                *x = mul(*x, scale);
            }
            let pivot_row = rows[col].clone();
            for (r, row) in rows.iter_mut().enumerate() {
                if r != col && row[col] != 0 {
                    let factor = row[col];
                    for (x, &p) in row.iter_mut().zip(&pivot_row) {
                        *x ^= mul(factor, p);
                    }
                }
            }
        }
        let mut word = [0u8; CODEWORD];
        word[..k].copy_from_slice(data);
        for t in 0..parity {
            word[k + t] = rows[t][parity];
        }
        word
    }

    #[test]
    fn a_codeword_passes_and_one_damaged_byte_does_not() {
        for (field, parity) in [(&SYSTEM_FIELD, 16usize), (&PAGE_FIELD, 4usize)] {
            let data: Vec<u8> = (0..CODEWORD - parity).map(|i| (i * 31 + 7) as u8).collect();
            let word = encode(&data, field, parity);
            assert!(codeword_clear(&word, field, parity));
            for at in [0usize, 5, 100, 254] {
                let mut bad = word;
                bad[at] ^= 0x40;
                assert!(!codeword_clear(&bad, field, parity), "byte {at}");
            }
        }
    }

    #[test]
    fn interleaved_blocks_come_back_in_order_and_name_a_damaged_block() {
        let blocks = 3usize;
        let mut expected = Vec::new();
        let mut raw = vec![0u8; blocks * CODEWORD];
        for b in 0..blocks {
            let data: Vec<u8> = (0..SYSTEM_DATA).map(|i| (i + b * 50) as u8).collect();
            let word = encode(&data, &SYSTEM_FIELD, SYSTEM_PARITY);
            expected.extend_from_slice(&data);
            for (i, &c) in word.iter().enumerate() {
                raw[i * blocks + b] = c;
            }
        }
        assert_eq!(
            extract_interleaved(&raw, blocks, SYSTEM_DATA, &SYSTEM_FIELD, SYSTEM_PARITY).unwrap(),
            expected
        );
        // Damage one byte of block 1.
        raw[10 * blocks + 1] ^= 1;
        assert_eq!(
            extract_interleaved(&raw, blocks, SYSTEM_DATA, &SYSTEM_FIELD, SYSTEM_PARITY),
            Err(1)
        );
    }

    #[test]
    fn every_literal_layout_is_a_permutation() {
        for n in 1..=32usize {
            let chunk: Vec<u8> = (0..n as u8).collect();
            let mut out = Vec::new();
            lay_out(&chunk, &mut out);
            let mut sorted = out.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, chunk, "length {n}");
        }
    }

    #[test]
    fn small_literal_layouts_match_the_table() {
        let lay = |n: u8| {
            let chunk: Vec<u8> = (0..n).collect();
            let mut out = Vec::new();
            lay_out(&chunk, &mut out);
            out
        };
        assert_eq!(lay(2), [1, 0]);
        assert_eq!(lay(3), [2, 1, 0]);
        assert_eq!(lay(4), [0, 1, 2, 3]);
        assert_eq!(lay(5), [4, 0, 1, 2, 3]);
        // Sixteen bytes: the halves swap, each half keeps its order.
        assert_eq!(
            lay(16),
            [8, 9, 10, 11, 12, 13, 14, 15, 0, 1, 2, 3, 4, 5, 6, 7]
        );
        // Eleven bytes: two reversed, then eight in order, then the first.
        assert_eq!(lay(11), [10, 9, 1, 2, 3, 4, 5, 6, 7, 8, 0]);
    }

    #[test]
    fn a_literal_only_stream_decompresses() {
        // Opcode 0x20 starts a stream whose first run is 3 bytes long.
        let src = [0x20, 0x00, 0x00, 0x03, 0xAA, 0xBB, 0xCC];
        assert_eq!(decompress(&src, 3).unwrap(), [0xCC, 0xBB, 0xAA]);
    }

    #[test]
    fn a_stream_that_stops_short_or_runs_over_is_refused() {
        let src = [0x20, 0x00, 0x00, 0x03, 0xAA, 0xBB, 0xCC];
        assert_eq!(
            decompress(&src, 4),
            Err(LzError::Short {
                produced: 3,
                expected: 4
            })
        );
        assert_eq!(decompress(&src, 2), Err(LzError::Overrun { expected: 2 }));
        assert_eq!(decompress(&src[..5], 3), Err(LzError::Truncated));
        assert_eq!(decompress(&[], 0), Err(LzError::Truncated));
    }

    #[test]
    fn a_copy_from_before_the_start_is_refused() {
        // A one-byte literal run, then opcode 0x10 (copy 3 bytes) whose
        // distance (1 + the next byte, 5) reaches back past the start.
        let src = [0x21, 0x00, 0x00, 0x01, 0xAA, 0x10, 0x04, 0x00];
        assert!(matches!(
            decompress(&src, 4),
            Err(LzError::CopyBeforeStart { .. })
        ));
    }

    #[test]
    fn the_page_checksum_matches_the_documented_algorithm() {
        let data = |n: usize| -> Vec<u8> { (0..n).map(|i| ((i * 7 + 3) & 0xFF) as u8).collect() };
        assert_eq!(page_sum_r2007(&data(0)), 0x0026_9EC3);
        assert_eq!(page_sum_r2007(&data(1)), 0xE2EC_E2C3);
        assert_eq!(page_sum_r2007(&data(7)), 0x5D40_7B56);
        assert_eq!(page_sum_r2007(&data(8)), 0xFACB_BF87);
        assert_eq!(page_sum_r2007(&data(100)), 0x866B_5C1D);
        // Across the 0x15B0-byte reduction boundary.
        assert_eq!(page_sum_r2007(&data(5552)), 0x0D23_E8F1);
        assert_eq!(page_sum_r2007(&data(5553)), 0x1053_2DD0);
        assert_eq!(page_sum_r2007(&data(6000)), 0x0D7A_C4E0);
    }

    #[test]
    fn a_short_file_is_a_named_error() {
        assert_eq!(
            read_header_r2007(&[0u8; 100]),
            Err(HeaderError::Truncated {
                needed: 0x480,
                len: 100
            })
        );
    }
}
