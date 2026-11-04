//! The decompressor of the R2004-family sections.
//!
//! The format is a variation of LZ77: a run of literal bytes, then a series
//! of opcodes, each of which copies bytes from earlier in the output and may
//! be followed by more literal bytes, until the terminator opcode. The
//! caller says how many bytes the section decompresses to; any other length,
//! any copy from before the start of the output and any opcode the format
//! does not use is an error. Nothing is produced past the stated length.

use std::fmt;

/// Why a compressed section could not be decompressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LzError {
    /// The compressed data ended in the middle of an opcode or a run.
    Truncated,
    /// An opcode the format does not use.
    UnusedOpcode(u8),
    /// A copy reaches back past the start of the output.
    CopyBeforeStart {
        /// How far back the copy reaches.
        distance: usize,
        /// How many bytes had been produced.
        produced: usize,
    },
    /// The output would be longer than the stated length.
    Overrun {
        /// The stated length.
        expected: usize,
    },
    /// The data ended (terminator or end of input) with fewer bytes than stated.
    Short {
        /// How many bytes were produced.
        produced: usize,
        /// The stated length.
        expected: usize,
    },
}

impl fmt::Display for LzError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LzError::Truncated => write!(f, "compressed data ends in the middle of an opcode"),
            LzError::UnusedOpcode(op) => {
                write!(f, "compressed data holds the unused opcode {op:#04x}")
            }
            LzError::CopyBeforeStart { distance, produced } => write!(
                f,
                "a copy reaches back {distance} bytes but only {produced} have been produced"
            ),
            LzError::Overrun { expected } => {
                write!(
                    f,
                    "compressed data produces more than the stated {expected} bytes"
                )
            }
            LzError::Short { produced, expected } => {
                write!(
                    f,
                    "compressed data produces {produced} bytes, {expected} were stated"
                )
            }
        }
    }
}

impl std::error::Error for LzError {}

struct Input<'a> {
    src: &'a [u8],
    pos: usize,
}

impl Input<'_> {
    fn byte(&mut self) -> Result<u8, LzError> {
        let b = *self.src.get(self.pos).ok_or(LzError::Truncated)?;
        self.pos += 1;
        Ok(b)
    }

    fn peek(&self) -> Result<u8, LzError> {
        self.src.get(self.pos).copied().ok_or(LzError::Truncated)
    }

    /// A literal length. A byte with a bit set in its high nibble is not a
    /// length at all: the length is 0 and the byte is the next opcode.
    fn literal_length(&mut self) -> Result<usize, LzError> {
        let b = self.peek()?;
        if b & 0xF0 != 0 {
            return Ok(0);
        }
        self.pos += 1;
        if b == 0 {
            Ok(self.run_total(0x0F)? + 3)
        } else {
            Ok(usize::from(b) + 3)
        }
    }

    /// A length stored as a run: each zero byte adds 0xFF, the first
    /// non-zero byte adds itself and ends the run.
    fn run_total(&mut self, start: usize) -> Result<usize, LzError> {
        let mut total = start;
        loop {
            match self.byte()? {
                0 => total += 0xFF,
                n => return Ok(total + usize::from(n)),
            }
        }
    }

    /// A long copy length: a non-zero byte is the value; zero starts a run
    /// from 0xFF.
    fn long_length(&mut self) -> Result<usize, LzError> {
        match self.byte()? {
            0 => self.run_total(0xFF),
            n => Ok(usize::from(n)),
        }
    }

    /// Two bytes holding an offset and a literal count of 0 to 3.
    fn two_byte_offset(&mut self) -> Result<(usize, usize), LzError> {
        let first = self.byte()?;
        let second = self.byte()?;
        Ok((
            usize::from(first >> 2) | (usize::from(second) << 6),
            usize::from(first & 0x03),
        ))
    }
}

/// Decompresses `src`, which must decompress to exactly `expected` bytes.
pub fn decompress(src: &[u8], expected: usize) -> Result<Vec<u8>, LzError> {
    let mut input = Input { src, pos: 0 };
    let mut out: Vec<u8> = Vec::with_capacity(expected.min(1 << 24));

    let copy_literals = |input: &mut Input<'_>, out: &mut Vec<u8>, n: usize| {
        if out.len() + n > expected {
            return Err(LzError::Overrun { expected });
        }
        let end = input.pos.checked_add(n).ok_or(LzError::Truncated)?;
        let run = input.src.get(input.pos..end).ok_or(LzError::Truncated)?;
        out.extend_from_slice(run);
        input.pos = end;
        Ok(())
    };

    let first = input.literal_length()?;
    copy_literals(&mut input, &mut out, first)?;

    loop {
        let op = input.byte()?;
        let (copy_len, distance, mut literals) = match op {
            0x00..=0x0F => return Err(LzError::UnusedOpcode(op)),
            0x11 => break,
            0x10 => {
                let len = input.long_length()? + 9;
                let (offset, lit) = input.two_byte_offset()?;
                (len, offset + 0x3FFF, lit)
            }
            0x12..=0x1F => {
                let (offset, lit) = input.two_byte_offset()?;
                (usize::from(op & 0x0F) + 2, offset + 0x3FFF, lit)
            }
            0x20 => {
                let len = input.long_length()? + 0x21;
                let (offset, lit) = input.two_byte_offset()?;
                (len, offset, lit)
            }
            0x21..=0x3F => {
                let (offset, lit) = input.two_byte_offset()?;
                (usize::from(op) - 0x1E, offset, lit)
            }
            0x40..=0xFF => {
                let second = input.byte()?;
                (
                    usize::from((op & 0xF0) >> 4) - 1,
                    (usize::from(second) << 2) | usize::from((op & 0x0C) >> 2),
                    usize::from(op & 0x03),
                )
            }
        };
        if literals == 0 {
            literals = input.literal_length()?;
        }

        // The stored offset is one less than the distance copied from.
        let distance = distance + 1;
        if distance > out.len() {
            return Err(LzError::CopyBeforeStart {
                distance,
                produced: out.len(),
            });
        }
        if out.len() + copy_len > expected {
            return Err(LzError::Overrun { expected });
        }
        // A copy may overlap its own output, so it goes byte by byte.
        let start = out.len() - distance;
        for from in start..start + copy_len {
            let b = out[from];
            out.push(b);
        }
        copy_literals(&mut input, &mut out, literals)?;
    }

    if out.len() != expected {
        return Err(LzError::Short {
            produced: out.len(),
            expected,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_literal_run_then_the_terminator() {
        // literal length byte 0x02 -> 5 literals, then 0x11.
        let src = [0x02, b'h', b'e', b'l', b'l', b'o', 0x11];
        assert_eq!(decompress(&src, 5).unwrap(), b"hello");
    }

    #[test]
    fn a_long_literal_length_adds_runs_of_ff() {
        // 0x00 starts the run at 0x0F; a 0x00 adds 0xFF; 0x02 ends it; + 3.
        let total = 0x0F + 0xFF + 2 + 3;
        let mut src = vec![0x00, 0x00, 0x02];
        src.extend(std::iter::repeat_n(b'x', total));
        src.push(0x11);
        assert_eq!(decompress(&src, total).unwrap(), vec![b'x'; total]);
    }

    #[test]
    fn a_short_copy_may_overlap_its_own_output() {
        // 4 literals "abab"... use literal length 0x01 -> 4 literals "abcd";
        // opcode 0x40 | (4 << 4) is >= 0x40: copy length ((0x50 & 0xF0) >> 4) - 1 = 4,
        // offset (second << 2) | ((op & 0x0C) >> 2); distance = offset + 1.
        // Opcode 0x50, second 0x00 -> offset 0, distance 1, no literals (op & 3 = 0
        // reads a literal length; the next byte 0x11 has a high bit -> 0).
        let src = [0x01, b'a', b'b', b'c', b'd', 0x50, 0x00, 0x11];
        assert_eq!(decompress(&src, 8).unwrap(), b"abcddddd");
    }

    #[test]
    fn a_two_byte_offset_copy_reaches_back_by_its_offset_plus_one() {
        // "abcdef" (literal length 0x03 -> 6), then opcode 0x21 (length 3):
        // two-byte offset first = (offset 5) << 2 | 0 literals, second = 0 -> offset 5,
        // distance 6 -> copies "abc"; literal count 0 reads a literal length
        // from the next byte (0x11 has a high bit -> 0).
        let src = [
            0x03,
            b'a',
            b'b',
            b'c',
            b'd',
            b'e',
            b'f',
            0x21,
            5 << 2,
            0x00,
            0x11,
        ];
        assert_eq!(decompress(&src, 9).unwrap(), b"abcdefabc");
    }

    #[test]
    fn a_copy_from_before_the_start_is_refused() {
        let src = [0x01, b'a', b'b', b'c', b'd', 0x50, 0x20, 0x11];
        assert!(matches!(
            decompress(&src, 8),
            Err(LzError::CopyBeforeStart { .. })
        ));
    }

    #[test]
    fn the_unused_opcodes_are_refused() {
        let src = [0x01, b'a', b'b', b'c', b'd', 0x05];
        assert_eq!(decompress(&src, 4), Err(LzError::UnusedOpcode(0x05)));
    }

    #[test]
    fn the_stated_length_is_enforced_both_ways() {
        let src = [0x02, b'h', b'e', b'l', b'l', b'o', 0x11];
        assert_eq!(decompress(&src, 4), Err(LzError::Overrun { expected: 4 }));
        assert_eq!(
            decompress(&src, 6),
            Err(LzError::Short {
                produced: 5,
                expected: 6
            })
        );
    }

    #[test]
    fn data_that_ends_without_the_terminator_is_truncated() {
        let src = [0x02, b'h', b'e', b'l', b'l', b'o'];
        assert_eq!(decompress(&src, 5), Err(LzError::Truncated));
    }
}
