//! The two checksums the file header uses.
//!
//! Both are table-driven cyclic redundancy checks of the common reflected
//! kind. The specification prints their lookup tables; here the tables are
//! computed from the generator polynomials, and the tests compare the first
//! entries with the printed ones.

/// The 16-bit table (generator polynomial 0xA001, reflected).
const fn table16() -> [u16; 256] {
    let mut table = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u16;
        let mut bit = 0;
        while bit < 8 {
            c = if c & 1 != 0 {
                (c >> 1) ^ 0xA001
            } else {
                c >> 1
            };
            bit += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

/// The 32-bit table (generator polynomial 0xEDB88320, reflected).
const fn table32() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut bit = 0;
        while bit < 8 {
            c = if c & 1 != 0 {
                (c >> 1) ^ 0xEDB8_8320
            } else {
                c >> 1
            };
            bit += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

static TABLE16: [u16; 256] = table16();
static TABLE32: [u32; 256] = table32();

/// The 16-bit checksum of `data`, continuing from `initial` (0 to start).
pub(crate) fn crc16(initial: u16, data: &[u8]) -> u16 {
    let mut crc = initial;
    for &byte in data {
        let index = byte ^ (crc & 0xFF) as u8;
        crc = (crc >> 8) ^ TABLE16[usize::from(index)];
    }
    crc
}

/// The 32-bit checksum of `data`, continuing from `seed` (0 to start).
pub(crate) fn crc32(seed: u32, data: &[u8]) -> u32 {
    let mut inverted = !seed;
    for &byte in data {
        inverted = (inverted >> 8) ^ TABLE32[usize::from((inverted as u8) ^ byte)];
    }
    !inverted
}

/// The checksum of a section page, continuing from `seed` (0 to start).
///
/// Two running sums are kept modulo 0xFFF1 and folded together; the data is
/// taken in chunks of 0x15B0 bytes so the sums cannot overflow before the
/// reduction.
pub(crate) fn page_checksum(seed: u32, data: &[u8]) -> u32 {
    let mut sum1 = seed & 0xFFFF;
    let mut sum2 = seed >> 16;
    for chunk in data.chunks(0x15B0) {
        for &byte in chunk {
            sum1 += u32::from(byte);
            sum2 += sum1;
        }
        sum1 %= 0xFFF1;
        sum2 %= 0xFFF1;
    }
    (sum2 << 16) | (sum1 & 0xFFFF)
}

/// How the checksum of an R2007 data page takes the 0 to 7 bytes after its last
/// whole eight: `(offset, count)` runs, in this order.
const R2007_TAIL: [&[(usize, usize)]; 8] = [
    &[],
    &[(0, 1)],
    &[(0, 2)],
    &[(0, 2), (2, 1)],
    &[(2, 2), (0, 2)],
    &[(2, 2), (0, 2), (4, 1)],
    &[(2, 2), (0, 2), (4, 2)],
    &[(2, 2), (0, 2), (4, 2), (6, 1)],
];

/// The 32-bit checksum of the decompressed data of an R2007 data page.
///
/// It starts from a value derived from the length, keeps two running sums
/// and reduces them modulo 0xFFF1 every 0x15B0 bytes. Within a block of eight
/// bytes the pairs are taken last to first.
pub(crate) fn page_sum_r2007(data: &[u8]) -> u32 {
    let seed = (data.len() as u64)
        .wrapping_mul(0x343FD)
        .wrapping_add(0x269EC3);
    let mut sum1 = seed & 0xFFFF;
    let mut sum2 = (seed >> 16) & 0xFFFF;
    for chunk in data.chunks(0x15B0) {
        let (blocks, rest) = chunk.as_chunks::<8>();
        for block in blocks {
            for at in [6usize, 4, 2, 0] {
                sum1 += u64::from(block[at]);
                sum2 += sum1;
                sum1 += u64::from(block[at + 1]);
                sum2 += sum1;
            }
        }
        for &(at, count) in R2007_TAIL[rest.len()] {
            for &byte in &rest[at..at + count] {
                sum1 += u64::from(byte);
                sum2 += sum1;
            }
        }
        sum1 %= 0xFFF1;
        sum2 %= 0xFFF1;
    }
    (((sum2 << 16) | (sum1 & 0xFFFF)) & 0xFFFF_FFFF) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_checksum_of_nothing_is_the_seed() {
        assert_eq!(page_checksum(0, b""), 0);
        assert_eq!(page_checksum(0x0001_0002, b""), 0x0001_0002);
    }

    #[test]
    fn the_page_checksum_is_the_adler_pair_of_sums() {
        // Seeded with 1, the two sums are Adler-32's: the check value of "Wikipedia".
        assert_eq!(page_checksum(1, b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn the_page_checksum_continues_across_a_split_and_across_chunks() {
        let data: Vec<u8> = (0..20_000u32).map(|i| (i * 7 + 3) as u8).collect();
        let (a, b) = data.split_at(9_999);
        assert_eq!(
            page_checksum(page_checksum(0, a), b),
            page_checksum(0, &data)
        );
    }

    #[test]
    fn the_16_bit_table_starts_as_the_specification_prints_it() {
        assert_eq!(
            &TABLE16[..8],
            &[0x0000, 0xC0C1, 0xC181, 0x0140, 0xC301, 0x03C0, 0x0280, 0xC241]
        );
        assert_eq!(TABLE16[255], 0x4040);
    }

    #[test]
    fn the_32_bit_table_starts_as_the_specification_prints_it() {
        assert_eq!(
            &TABLE32[..4],
            &[0x0000_0000, 0x7707_3096, 0xEE0E_612C, 0x9909_51BA]
        );
        assert_eq!(TABLE32[255], 0x2D02_EF8D);
    }

    #[test]
    fn the_32_bit_checksum_of_a_known_string() {
        // The widely published check value of this checksum.
        assert_eq!(crc32(0, b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn a_checksum_continues_across_a_split() {
        let data = b"the quick brown fox";
        let (a, b) = data.split_at(7);
        assert_eq!(crc16(crc16(0, a), b), crc16(0, data));
        assert_eq!(crc32(crc32(0, a), b), crc32(0, data));
    }
}
