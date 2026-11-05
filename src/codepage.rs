//! The code page of a drawing's text.
//!
//! Before R2007 a drawing stores its strings as bytes in the code page the file header declares by a
//! number. The number is not a Windows code page number, and the public file specification does not give
//! the table. This module therefore knows **only the numbers that have been observed against a second
//! statement of the same drawing**: a DWG whose header states a number, next to a DXF of the same drawing
//! whose `$DWGCODEPAGE` names the page. Any other number is not decoded -- its text stays as bytes, and
//! the conversion says so -- because a wrong name turns text into wrong characters without a trace.
//!
//! Observed in the public corpus (each row: DWG header number -> the DXF's name; files whose twins state it):
//!
//! | number | `$DWGCODEPAGE` | twins |
//! |---|---|---|
//! | 28 | `ANSI_1250` | 2 |
//! | 29 | `ANSI_1251` | 1 |
//! | 30 | `ANSI_1252` | 41 |
//!
//! Two more numbers (34 and 39) each have one twin that states `ANSI_1252`. A twin may have been written by
//! a program that re-encodes, so it is not evidence of what the original file's number means: neither
//! number is in the table.

use encoding_rs::{Encoding, WINDOWS_1250, WINDOWS_1251, WINDOWS_1252};

/// How a drawing's code page bytes become characters: through a known page, or not at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodePage(Option<&'static Encoding>);

impl CodePage {
    /// The page for the number a file header declares; no page for a number not in the table above.
    pub fn of(number: u16) -> CodePage {
        CodePage(match number {
            28 => Some(WINDOWS_1250),
            29 => Some(WINDOWS_1251),
            30 => Some(WINDOWS_1252),
            _ => None,
        })
    }

    /// No page: only ASCII text can be read.
    pub const UNKNOWN: CodePage = CodePage(None);

    /// The characters of `bytes` in this page, or `None` when the page is unknown or a byte has no
    /// character in it (a malformed text is not repaired with a replacement character).
    pub fn decode(self, bytes: &[u8]) -> Option<String> {
        let encoding = self.0?;
        encoding
            .decode_without_bom_handling_and_without_replacement(bytes)
            .map(|text| text.into_owned())
    }

    /// Whether this page is known.
    pub fn is_known(self) -> bool {
        self.0.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_numbers_observed_against_a_twin_are_known() {
        for number in [28, 29, 30] {
            assert!(CodePage::of(number).is_known(), "{number}");
        }
        // Not observed (or contradicted by the only twin): not guessed.
        for number in [0, 1, 27, 31, 34, 38, 39, 41, 4608, 52992] {
            assert!(!CodePage::of(number).is_known(), "{number}");
        }
    }

    #[test]
    fn a_page_turns_bytes_into_its_characters() {
        // 1252: 0xB0 is the degree sign, 0xE9 is e-acute, 0x80 is the euro sign.
        assert_eq!(
            CodePage::of(30).decode(&[b'A', 0xB0]).as_deref(),
            Some("A\u{B0}")
        );
        assert_eq!(
            CodePage::of(30).decode(&[0xE9, 0x80]).as_deref(),
            Some("\u{E9}\u{20AC}")
        );
        // 1251: 0xC0 is the Cyrillic capital A.
        assert_eq!(CodePage::of(29).decode(&[0xC0]).as_deref(), Some("\u{410}"));
        // 1250: 0xA3 is the Polish crossed l (capital).
        assert_eq!(CodePage::of(28).decode(&[0xA3]).as_deref(), Some("\u{141}"));
    }

    #[test]
    fn an_unknown_page_decodes_nothing_not_even_a_guess() {
        assert_eq!(CodePage::UNKNOWN.decode(b"A"), None);
        assert_eq!(CodePage::of(39).decode(&[0xB0]), None);
    }
}
