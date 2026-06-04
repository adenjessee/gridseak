//! LSP column-encoding conversions (Sprint F.2).
//!
//! Tree-sitter gives us byte-offset columns inside a line. The LSP
//! specification (§ Position) mandates UTF-16 code-unit offsets by
//! default, with servers and clients allowed to negotiate
//! `general.positionEncodings` to opt into UTF-8 or UTF-32. We do not
//! negotiate (yet), so we must convert every outgoing column to
//! UTF-16 code units or risk jorje / rust-analyzer resolving the
//! wrong identifier the moment a source file contains a non-ASCII
//! character above the cursor line.
//!
//! Apex sources are ASCII almost always, so the practical impact for
//! the pilot is small, but correctness is non-negotiable and other
//! languages (TypeScript in multilingual projects, comments in CJK)
//! absolutely need this.
//!
//! # Algorithm
//!
//! Scan the line up to `byte_col`. For each `char` observed:
//! * 1 UTF-16 code unit for BMP scalar values (`u <= 0xFFFF`),
//! * 2 UTF-16 code units for supplementary scalar values (surrogate
//!   pair).
//!
//! Byte-col arguments are clamped to the line length to handle edge
//! cases where the caller's line ends before the reported column
//! (happens on multi-line parse errors).
//!
//! # Design notes
//!
//! * The converter takes a `&str` for the specific line to keep the
//!   function pure and trivially testable. Callers that only hold a
//!   file path use [`utf16_column_for_file`], which wraps a tiny
//!   cache-free read.
//! * Negative / zero values are not meaningful — the function
//!   operates on `u32` in both directions, matching
//!   `domain::Range::start_char`.

use std::fs;
use std::path::Path;

/// Convert a byte offset within `line_source` to a UTF-16 code-unit
/// offset. Out-of-bounds byte columns are clamped to the end of the
/// line (which is what we want when jorje reports a line-end
/// identifier and we then point slightly past it).
pub fn byte_col_to_utf16(line_source: &str, byte_col: u32) -> u32 {
    let byte_col = byte_col as usize;
    let mut utf16 = 0u32;
    let mut byte_cursor = 0usize;
    for ch in line_source.chars() {
        if byte_cursor >= byte_col {
            break;
        }
        byte_cursor += ch.len_utf8();
        utf16 = utf16.saturating_add(ch.len_utf16() as u32);
    }
    utf16
}

/// Convert `byte_col` on 1-based `line_1based` of `file_path` to UTF-16.
///
/// Returns `byte_col` unchanged on any I/O or indexing error. This is
/// a graceful degradation: the worst case (byte col used for
/// positions past a non-ASCII character) was also the pre-F.2
/// behaviour, so we never regress definition lookups when the file
/// is unreadable.
pub fn utf16_column_for_file(file_path: &Path, line_1based: u32, byte_col: u32) -> u32 {
    // Read the whole file: Apex/TS source files are small and the
    // orchestrator already holds them in memory for Tree-sitter. If
    // this ever becomes a hot path we can introduce an explicit
    // `DocumentCache`, but for F.2 the cost is negligible next to
    // the LSP round trip.
    let Ok(contents) = fs::read_to_string(file_path) else {
        return byte_col;
    };
    let Some(line) = contents
        .lines()
        .nth((line_1based.saturating_sub(1)) as usize)
    else {
        return byte_col;
    };
    byte_col_to_utf16(line, byte_col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_line_passes_through_unchanged() {
        let line = "public class AccountTriggerHandler {";
        assert_eq!(byte_col_to_utf16(line, 0), 0);
        assert_eq!(
            byte_col_to_utf16(line, 13),
            13,
            "ASCII byte-col == utf16-col"
        );
    }

    #[test]
    fn bmp_non_ascii_character_is_one_utf16_unit() {
        // '漢' is three UTF-8 bytes but one UTF-16 code unit.
        let line = "// 漢 AccountTriggerHandler";
        // After the "// " prefix (3 bytes) we see '漢' at byte col 3.
        // After '漢' (+3 bytes, +1 utf16), byte col 6 corresponds to
        // utf16 col 4.
        assert_eq!(byte_col_to_utf16(line, 3), 3, "col before the glyph");
        assert_eq!(
            byte_col_to_utf16(line, 6),
            4,
            "glyph consumes 3 bytes but only 1 utf16 code unit"
        );
    }

    #[test]
    fn supplementary_character_is_two_utf16_units() {
        // 😀 (U+1F600) is 4 UTF-8 bytes, 2 UTF-16 code units (surrogate pair).
        let line = "// 😀 foo()";
        // "// " = 3 bytes, then emoji = 4 bytes → byte col 7.
        assert_eq!(byte_col_to_utf16(line, 3), 3, "col before the emoji");
        assert_eq!(
            byte_col_to_utf16(line, 7),
            5,
            "emoji consumes 4 bytes but 2 utf16 code units"
        );
    }

    #[test]
    fn overshoot_is_clamped_to_line_end() {
        let line = "abc";
        assert_eq!(
            byte_col_to_utf16(line, 9999),
            3,
            "byte col past the line end must clamp to len() in code units"
        );
    }

    #[test]
    fn zero_byte_col_is_zero_utf16_col() {
        let line = "漢字";
        assert_eq!(byte_col_to_utf16(line, 0), 0);
    }

    #[test]
    fn utf16_column_for_file_round_trips_multibyte_line() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        // Line 1 = ASCII, line 2 has a BMP multibyte char before
        // "foo". Expect the byte column of "foo" on line 2 (6) to
        // convert to utf16 col 4.
        std::fs::write(tmp.path(), "hello\n// 漢 foo()\n").unwrap();
        assert_eq!(
            utf16_column_for_file(tmp.path(), 2, 6),
            4,
            "utf16 offset of 'foo' after multibyte char"
        );
    }

    #[test]
    fn utf16_column_for_file_degrades_on_io_error() {
        let missing = Path::new("/tmp/graphengine-does-not-exist-xxxx.cls");
        assert_eq!(
            utf16_column_for_file(missing, 1, 42),
            42,
            "on I/O error we return the byte col unchanged — graceful degradation"
        );
    }
}
