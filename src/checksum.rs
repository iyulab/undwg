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

#[cfg(test)]
mod tests {
    use super::*;

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
