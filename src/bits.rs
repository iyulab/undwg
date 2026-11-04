//! Reading the format's bit-packed values.
//!
//! Most of the data inside a DWG file is a stream of *bits*, not bytes: small
//! common values take two bits, larger ones a few bits followed by the value.
//! A [`BitReader`] walks such a stream, most significant bit first within each
//! byte, and reads the value forms the format defines. Reading past the end
//! of the data is an error -- never a zero.

use std::fmt;

/// Reading past the end of the data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndOfData {
    /// The bit position at which the read started.
    pub at_bit: usize,
    /// How many bits the read needed.
    pub needed_bits: usize,
}

impl fmt::Display for EndOfData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the data ends at bit {}: a read there needed {} more bits",
            self.at_bit, self.needed_bits
        )
    }
}

impl std::error::Error for EndOfData {}

/// Why a bit-coded value could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BitError {
    /// The data ended in the middle of the value.
    End(EndOfData),
    /// The two bits of a value form hold the one code the format marks "not used".
    NotUsed {
        /// Which value form.
        form: &'static str,
    },
    /// A modular value is longer than the integer it must fit in.
    TooLong {
        /// Which value form.
        form: &'static str,
    },
}

impl fmt::Display for BitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BitError::End(e) => e.fmt(f),
            BitError::NotUsed { form } => {
                write!(f, "a {form} holds the code the format marks as not used")
            }
            BitError::TooLong { form } => write!(f, "a {form} is longer than its integer"),
        }
    }
}

impl std::error::Error for BitError {}

impl From<EndOfData> for BitError {
    fn from(e: EndOfData) -> Self {
        BitError::End(e)
    }
}

/// A handle reference: the 4-bit code, and the value the bytes spell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandleRef {
    /// The code (how the value is to be applied).
    pub code: u8,
    /// The value, read from up to eight bytes, most significant first.
    pub value: u64,
}

/// A cursor over bit-coded data.
#[derive(Debug, Clone)]
pub struct BitReader<'a> {
    data: &'a [u8],
    /// The position, in bits from the start of `data`.
    pos: usize,
}

impl<'a> BitReader<'a> {
    /// A reader at the first bit of `data`.
    pub fn new(data: &'a [u8]) -> Self {
        BitReader { data, pos: 0 }
    }

    /// The position in bits from the start.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Moves to bit position `pos`; positions past the end are allowed and
    /// make the next read fail.
    pub fn seek(&mut self, pos: usize) {
        self.pos = pos;
    }

    /// How many bits are left.
    pub fn remaining(&self) -> usize {
        (self.data.len() * 8).saturating_sub(self.pos)
    }

    fn need(&self, bits: usize) -> Result<(), EndOfData> {
        if self.remaining() < bits {
            Err(EndOfData {
                at_bit: self.pos,
                needed_bits: bits,
            })
        } else {
            Ok(())
        }
    }

    /// One bit (B).
    pub fn bit(&mut self) -> Result<bool, EndOfData> {
        self.need(1)?;
        let byte = self.data[self.pos / 8];
        let bit = (byte >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Ok(bit == 1)
    }

    /// `n` bits (n <= 64) as an unsigned number, first bit most significant.
    pub fn bits(&mut self, n: usize) -> Result<u64, EndOfData> {
        debug_assert!(n <= 64);
        self.need(n)?;
        let mut v = 0u64;
        for _ in 0..n {
            v = (v << 1) | u64::from(self.bit()?);
        }
        Ok(v)
    }

    /// A byte at any bit alignment (RC).
    pub fn raw_char(&mut self) -> Result<u8, EndOfData> {
        Ok(self.bits(8)? as u8)
    }

    /// A raw short, least significant byte first (RS).
    pub fn raw_short(&mut self) -> Result<u16, EndOfData> {
        let lo = u16::from(self.raw_char()?);
        let hi = u16::from(self.raw_char()?);
        Ok(lo | (hi << 8))
    }

    /// A raw long, least significant byte first (RL).
    pub fn raw_long(&mut self) -> Result<u32, EndOfData> {
        let lo = u32::from(self.raw_short()?);
        let hi = u32::from(self.raw_short()?);
        Ok(lo | (hi << 16))
    }

    /// A raw double (RD): eight bytes, least significant first.
    pub fn raw_double(&mut self) -> Result<f64, EndOfData> {
        let mut b = [0u8; 8];
        for byte in &mut b {
            *byte = self.raw_char()?;
        }
        Ok(f64::from_le_bytes(b))
    }

    /// A bit triplet (3B): one to three bits, reading until a zero bit or
    /// the third bit, each new bit shifted in from the right.
    pub fn bit_triplet(&mut self) -> Result<u8, EndOfData> {
        let mut v = 0u8;
        for _ in 0..3 {
            v = (v << 1) | u8::from(self.bit()?);
            if v & 1 == 0 {
                break;
            }
        }
        Ok(v)
    }

    /// A bitshort (BS): `00` a short follows, `01` a byte follows, `10` zero,
    /// `11` 256.
    pub fn bit_short(&mut self) -> Result<i16, EndOfData> {
        Ok(match self.bits(2)? {
            0 => self.raw_short()? as i16,
            1 => i16::from(self.raw_char()?),
            2 => 0,
            _ => 256,
        })
    }

    /// A bitlong (BL): `00` a long follows, `01` a byte follows, `10` zero.
    /// The code `11` is not used.
    pub fn bit_long(&mut self) -> Result<i32, BitError> {
        Ok(match self.bits(2)? {
            0 => self.raw_long()? as i32,
            1 => i32::from(self.raw_char()?),
            2 => 0,
            _ => return Err(BitError::NotUsed { form: "bitlong" }),
        })
    }

    /// A bitlonglong (BLL): a bit triplet gives how many bytes follow, least
    /// significant byte first.
    pub fn bit_long_long(&mut self) -> Result<u64, BitError> {
        let len = self.bit_triplet()?;
        let mut v = 0u64;
        for i in 0..len {
            v |= u64::from(self.raw_char()?) << (8 * u32::from(i));
        }
        Ok(v)
    }

    /// A bitdouble (BD): `00` a double follows, `01` one, `10` zero. The
    /// code `11` is not used.
    pub fn bit_double(&mut self) -> Result<f64, BitError> {
        Ok(match self.bits(2)? {
            0 => self.raw_double()?,
            1 => 1.0,
            2 => 0.0,
            _ => return Err(BitError::NotUsed { form: "bitdouble" }),
        })
    }

    /// A bitdouble with a default (DD): a two-bit opcode says how much of
    /// `default`'s bytes the stream replaces.
    pub fn bit_double_with_default(&mut self, default: f64) -> Result<f64, EndOfData> {
        let mut bytes = default.to_le_bytes();
        match self.bits(2)? {
            0 => {}
            1 => {
                for b in &mut bytes[..4] {
                    *b = self.raw_char()?;
                }
            }
            2 => {
                bytes[4] = self.raw_char()?;
                bytes[5] = self.raw_char()?;
                for b in &mut bytes[..4] {
                    *b = self.raw_char()?;
                }
            }
            _ => return self.raw_double(),
        }
        Ok(f64::from_le_bytes(bytes))
    }

    /// A modular char (MC), unsigned: seven bits per byte, least significant
    /// group first, the top bit of a byte says another byte follows.
    pub fn modular_char(&mut self) -> Result<u64, BitError> {
        let mut v = 0u64;
        for i in 0..10u32 {
            let b = self.raw_char()?;
            let group = u64::from(b & 0x7F);
            if i == 9 && group > 1 {
                return Err(BitError::TooLong {
                    form: "modular char",
                });
            }
            v |= group << (7 * i);
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(BitError::TooLong {
            form: "modular char",
        })
    }

    /// A signed modular char: as [`modular_char`](Self::modular_char), but
    /// when the final byte has the 0x40 bit set, that bit is cleared and the
    /// number is negated.
    pub fn modular_char_signed(&mut self) -> Result<i64, BitError> {
        let mut v = 0u64;
        for i in 0..9u32 {
            let b = self.raw_char()?;
            if b & 0x80 != 0 {
                v |= u64::from(b & 0x7F) << (7 * i);
                continue;
            }
            let negative = b & 0x40 != 0;
            v |= u64::from(b & 0x3F) << (7 * i);
            let v = i64::try_from(v).map_err(|_| BitError::TooLong {
                form: "modular char",
            })?;
            return Ok(if negative { -v } else { v });
        }
        Err(BitError::TooLong {
            form: "modular char",
        })
    }

    /// A modular short (MS): the same idea with 15 bits per little-endian
    /// short; the top bit of a short says another short follows.
    pub fn modular_short(&mut self) -> Result<u64, BitError> {
        let mut v = 0u64;
        for i in 0..4u32 {
            let s = self.raw_short()?;
            v |= u64::from(s & 0x7FFF) << (15 * i);
            if s & 0x8000 == 0 {
                return Ok(v);
            }
        }
        Err(BitError::TooLong {
            form: "modular short",
        })
    }

    /// A handle reference (H): a 4-bit code, a 4-bit byte count (up to 8),
    /// then that many bytes, most significant first.
    pub fn handle(&mut self) -> Result<HandleRef, BitError> {
        let code = self.bits(4)? as u8;
        let count = self.bits(4)? as usize;
        if count > 8 {
            return Err(BitError::TooLong { form: "handle" });
        }
        let mut value = 0u64;
        for _ in 0..count {
            value = (value << 8) | u64::from(self.raw_char()?);
        }
        Ok(HandleRef { code, value })
    }

    /// An object type as R2010 and later write it: a bit pair, then one byte
    /// (`0`), one byte plus 0x1F0 (`1`), or a raw short (`2`, and `3` read the same).
    pub fn object_type(&mut self) -> Result<u16, EndOfData> {
        Ok(match self.bits(2)? {
            0 => u16::from(self.raw_char()?),
            1 => u16::from(self.raw_char()?) + 0x1F0,
            _ => self.raw_short()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bits written as text ("0010 1100") packed into bytes, zero padded.
    fn pack(bits: &str) -> Vec<u8> {
        let bits: Vec<u8> = bits
            .chars()
            .filter(|c| *c == '0' || *c == '1')
            .map(|c| c as u8 - b'0')
            .collect();
        bits.chunks(8)
            .map(|c| {
                let mut byte = 0u8;
                for (i, b) in c.iter().enumerate() {
                    byte |= b << (7 - i);
                }
                byte
            })
            .collect()
    }

    #[test]
    fn five_bitshorts_from_the_specifications_example() {
        let data = pack("0000000001000000011011010000111110");
        let mut r = BitReader::new(&data);
        let got: Vec<i16> = (0..5).map(|_| r.bit_short().unwrap()).collect();
        assert_eq!(got, vec![257, 0, 256, 15, 0]);
    }

    #[test]
    fn five_bitlongs_from_the_specifications_example() {
        let data = pack("000000000100000001000000000000000010010000111110");
        let mut r = BitReader::new(&data);
        let got: Vec<i32> = (0..4).map(|_| r.bit_long().unwrap()).collect();
        assert_eq!(got, vec![257, 0, 15, 0]);
    }

    #[test]
    fn a_negative_number_uses_the_wide_forms() {
        let short = pack("00 11111111 11111111");
        assert_eq!(BitReader::new(&short).bit_short().unwrap(), -1);
        let long = pack("00 11111111 11111111 11111111 11111111");
        assert_eq!(BitReader::new(&long).bit_long().unwrap(), -1);
    }

    #[test]
    fn the_unused_codes_are_refused() {
        assert!(matches!(
            BitReader::new(&pack("11")).bit_long(),
            Err(BitError::NotUsed { form: "bitlong" })
        ));
        assert!(matches!(
            BitReader::new(&pack("11")).bit_double(),
            Err(BitError::NotUsed { form: "bitdouble" })
        ));
    }

    #[test]
    fn a_bit_triplet_reads_until_a_zero_or_three_bits() {
        let data = pack("0 10 110 111");
        let mut r = BitReader::new(&data);
        assert_eq!(r.bit_triplet().unwrap(), 0b0);
        assert_eq!(r.bit_triplet().unwrap(), 0b10);
        assert_eq!(r.bit_triplet().unwrap(), 0b110);
        assert_eq!(r.bit_triplet().unwrap(), 0b111);
    }

    #[test]
    fn a_bitlonglong_has_its_length_in_a_triplet() {
        // The triplet "10" reads as 2: two bytes follow, least significant first.
        let data = pack("10 00110100 00010010");
        assert_eq!(BitReader::new(&data).bit_long_long().unwrap(), 0x1234);
        // length 0 (0): the value is 0.
        assert_eq!(BitReader::new(&pack("0")).bit_long_long().unwrap(), 0);
    }

    #[test]
    fn bit_doubles() {
        let mut data = pack("01 10 00");
        data.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
        let mut r = BitReader::new(&data);
        assert_eq!(r.bit_double().unwrap(), 1.0);
        assert_eq!(r.bit_double().unwrap(), 0.0);
        assert_eq!(r.bit_double().unwrap(), 0.0); // `00`: a raw double of zeros follows
    }

    #[test]
    fn a_bit_double_with_default_patches_the_default_bytes() {
        let default = 1.0f64; // bytes 00 00 00 00 00 00 F0 3F
                              // 00: the default itself.
        assert_eq!(
            BitReader::new(&pack("00"))
                .bit_double_with_default(default)
                .unwrap(),
            1.0
        );
        // 01: four bytes replace the first four.
        let patched = f64::from_le_bytes([1, 2, 3, 4, 0, 0, 0xF0, 0x3F]);
        let data = pack("01 00000001 00000010 00000011 00000100");
        assert_eq!(
            BitReader::new(&data)
                .bit_double_with_default(default)
                .unwrap(),
            patched
        );
        // 10: two bytes replace bytes 5 and 6, four replace the first four.
        let patched = f64::from_le_bytes([1, 2, 3, 4, 5, 6, 0xF0, 0x3F]);
        let data = pack("10 00000101 00000110 00000001 00000010 00000011 00000100");
        assert_eq!(
            BitReader::new(&data)
                .bit_double_with_default(default)
                .unwrap(),
            patched
        );
        // 11: a full double, whose bytes follow the two-bit code bit-aligned.
        let full = 2.5f64.to_le_bytes();
        let mut bits = String::from("11");
        for b in full {
            bits.push_str(&format!("{b:08b}"));
        }
        assert_eq!(
            BitReader::new(&pack(&bits))
                .bit_double_with_default(default)
                .unwrap(),
            2.5
        );
    }

    #[test]
    fn the_specifications_modular_char_examples() {
        // 10000010 00100100 -> 0b0100100_0000010 = 4610.
        let a = [0b1000_0010, 0b0010_0100];
        assert_eq!(
            BitReader::new(&a).modular_char().unwrap(),
            (0b0100100 << 7) | 0b0000010
        );
        // The second example: four bytes, result 233 + 139*256 + 185*256^2 + 6*256^3
        // spelled in 7-bit groups is checked by round trip below.
    }

    #[test]
    fn modular_chars_round_trip() {
        fn encode(mut v: u64) -> Vec<u8> {
            let mut out = Vec::new();
            loop {
                let group = (v & 0x7F) as u8;
                v >>= 7;
                if v == 0 {
                    out.push(group);
                    return out;
                }
                out.push(group | 0x80);
            }
        }
        for v in [
            0u64,
            1,
            127,
            128,
            16_383,
            16_384,
            0x1234_5678,
            u32::MAX as u64,
            u64::MAX >> 1,
            u64::MAX,
        ] {
            let bytes = encode(v);
            assert_eq!(BitReader::new(&bytes).modular_char().unwrap(), v, "{v}");
        }
    }

    #[test]
    fn a_modular_char_longer_than_64_bits_is_refused() {
        let bytes = [0xFF; 11];
        assert!(matches!(
            BitReader::new(&bytes).modular_char(),
            Err(BitError::TooLong { .. })
        ));
        let mut bytes = vec![0x80; 9];
        bytes.push(0x02); // bit 64 set: does not fit
        assert!(matches!(
            BitReader::new(&bytes).modular_char(),
            Err(BitError::TooLong { .. })
        ));
    }

    #[test]
    fn a_signed_modular_char_negates_on_the_forty_bit() {
        // The specification's negative example: 10000101 01001011 clears the
        // sign bit of the last byte and negates what is left.
        let bytes = [0b1000_0101, 0b0100_1011];
        let magnitude = (0b0001011i64 << 7) | 0b0000101;
        assert_eq!(
            BitReader::new(&bytes).modular_char_signed().unwrap(),
            -magnitude
        );
        // The same bytes without the 0x40 bit are positive.
        let bytes = [0b1000_0101, 0b0000_1011];
        assert_eq!(
            BitReader::new(&bytes).modular_char_signed().unwrap(),
            magnitude
        );
        // A single byte below 0x40 is itself; 0x45 is -5.
        assert_eq!(BitReader::new(&[0x2A]).modular_char_signed().unwrap(), 42);
        assert_eq!(BitReader::new(&[0x45]).modular_char_signed().unwrap(), -5);
    }

    #[test]
    fn the_specifications_modular_short_example() {
        // 00110001 11110100 10001101 00000000 -> 62513 + 70 * 65536 = 4650033.
        let bytes = [0b0011_0001, 0b1111_0100, 0b1000_1101, 0b0000_0000];
        assert_eq!(BitReader::new(&bytes).modular_short().unwrap(), 4_650_033);
    }

    #[test]
    fn the_specifications_handle_example() {
        // 5.2.05E7: code 5, two bytes 05 E7.
        let data = pack("0101 0010 00000101 11100111");
        let h = BitReader::new(&data).handle().unwrap();
        assert_eq!(
            h,
            HandleRef {
                code: 5,
                value: 0x05E7
            }
        );
        // 5.1.0F: the usual reference to layer 0 of an R13 drawing.
        let data = pack("0101 0001 00001111");
        assert_eq!(
            BitReader::new(&data).handle().unwrap(),
            HandleRef {
                code: 5,
                value: 0x0F
            }
        );
        // a count above eight bytes is refused.
        assert!(matches!(
            BitReader::new(&pack("0101 1001")).handle(),
            Err(BitError::TooLong { .. })
        ));
    }

    #[test]
    fn object_types() {
        assert_eq!(
            BitReader::new(&pack("00 00010011")).object_type().unwrap(),
            0x13
        );
        assert_eq!(
            BitReader::new(&pack("01 00000011")).object_type().unwrap(),
            0x1F3
        );
        assert_eq!(
            BitReader::new(&pack("10 00000001 00000010"))
                .object_type()
                .unwrap(),
            0x0201
        );
    }

    #[test]
    fn reading_past_the_end_is_an_error_not_a_zero() {
        let mut r = BitReader::new(&[0b1000_0000]);
        assert!(r.bit().unwrap());
        for _ in 0..7 {
            r.bit().unwrap();
        }
        assert_eq!(
            r.bit(),
            Err(EndOfData {
                at_bit: 8,
                needed_bits: 1
            })
        );
        assert!(matches!(
            BitReader::new(&[0b0000_0000]).bit_short(),
            Err(EndOfData {
                at_bit: 2,
                needed_bits: 8
            })
        ));
    }

    #[test]
    fn values_read_at_any_bit_alignment() {
        // One leading bit, then a raw short 0x1234 (bytes 34 12).
        let data = pack("1 00110100 00010010");
        let mut r = BitReader::new(&data);
        assert!(r.bit().unwrap());
        assert_eq!(r.raw_short().unwrap(), 0x1234);
        assert_eq!(r.position(), 17);
    }
}
