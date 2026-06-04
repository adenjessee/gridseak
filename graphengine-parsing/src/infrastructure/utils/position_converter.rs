//! Provides utilities for converting between different text offset representations,
//! such as byte offsets, line/character positions, and UTF-8/UTF-16 offsets.
//! This is crucial for interoperability with tools like Tree-sitter (byte offsets)
//! and Language Server Protocol (LSP, UTF-16 offsets).

use anyhow::{anyhow, Result};
use tracing::instrument;

/// Utility for converting between different text position formats.
#[derive(Debug, Clone, Default)]
pub struct PositionConverter;

impl PositionConverter {
    /// Converts a byte offset within a string to a (line, character) tuple.
    /// Lines are 1-based, characters are 0-based.
    #[instrument(skip(self, text))]
    pub fn byte_offset_to_line_char(&self, text: &str, byte_offset: usize) -> Result<(u32, u32)> {
        if byte_offset > text.len() {
            return Err(anyhow!(
                "Byte offset {} is out of bounds for text length {}",
                byte_offset,
                text.len()
            ));
        }

        let mut line = 1u32;
        let mut char_pos = 0u32;
        let mut _current_byte_offset = 0usize;

        for (i, ch) in text.char_indices() {
            if i >= byte_offset {
                break;
            }

            if ch == '\n' {
                line += 1;
                char_pos = 0;
            } else {
                char_pos += 1;
            }

            _current_byte_offset = i + ch.len_utf8();
        }

        Ok((line, char_pos))
    }

    /// Converts a (line, character) tuple to a byte offset within a string.
    /// Lines are 1-based, characters are 0-based.
    #[instrument(skip(self, text))]
    pub fn line_char_to_byte_offset(
        &self,
        text: &str,
        target_line: u32,
        target_char: u32,
    ) -> Result<usize> {
        if target_line == 0 {
            return Err(anyhow!("Line number cannot be 0 (must be 1-based)"));
        }

        let mut current_line = 1u32;
        let mut current_char = 0u32;
        let mut byte_offset = 0usize;

        for (i, ch) in text.char_indices() {
            if current_line == target_line && current_char == target_char {
                return Ok(byte_offset);
            }

            if ch == '\n' {
                current_line += 1;
                current_char = 0;
            } else {
                current_char += 1;
            }
            byte_offset = i + ch.len_utf8();
        }

        // Handle case where target is at the very end of the file
        if current_line == target_line && current_char == target_char {
            return Ok(byte_offset);
        }

        Err(anyhow!(
            "Target line/char ({}:{}) not found in text",
            target_line,
            target_char
        ))
    }

    /// Converts a UTF-8 byte offset to a UTF-16 offset.
    /// This is useful for converting Tree-sitter offsets to LSP-compatible offsets.
    #[instrument(skip(self, text))]
    pub fn utf8_offset_to_utf16_offset(&self, text: &str, utf8_offset: usize) -> Result<u32> {
        if utf8_offset > text.len() {
            return Err(anyhow!(
                "UTF-8 offset {} is out of bounds for text length {}",
                utf8_offset,
                text.len()
            ));
        }

        let mut utf16_offset = 0u32;
        let mut _current_utf8_offset = 0usize;

        for (i, ch) in text.char_indices() {
            if i >= utf8_offset {
                break;
            }

            utf16_offset += ch.len_utf16() as u32;
            _current_utf8_offset = i + ch.len_utf8();
        }

        Ok(utf16_offset)
    }

    /// Converts a UTF-16 offset to a UTF-8 byte offset.
    /// This is useful for converting LSP-compatible offsets to Tree-sitter offsets.
    #[instrument(skip(self, text))]
    pub fn utf16_offset_to_utf8_offset(&self, text: &str, utf16_offset: u32) -> Result<usize> {
        let mut current_utf16_offset = 0u32;
        let mut byte_offset = 0usize;

        for (i, ch) in text.char_indices() {
            if current_utf16_offset == utf16_offset {
                return Ok(byte_offset);
            }

            current_utf16_offset += ch.len_utf16() as u32;
            byte_offset = i + ch.len_utf8();
        }

        if current_utf16_offset == utf16_offset {
            return Ok(byte_offset);
        }

        Err(anyhow!(
            "UTF-16 offset {} not found in text (max: {})",
            utf16_offset,
            current_utf16_offset
        ))
    }

    /// Static convenience methods for direct usage without instantiation
    pub fn utf8_offset_to_line_char(text: &str, byte_offset: usize) -> Result<(u32, u32)> {
        let converter = PositionConverter;
        converter.byte_offset_to_line_char(text, byte_offset)
    }

    pub fn line_char_to_utf8_offset(
        text: &str,
        target_line: u32,
        target_char: u32,
    ) -> Result<usize> {
        let converter = PositionConverter;
        converter.line_char_to_byte_offset(text, target_line, target_char)
    }

    pub fn utf8_offset_to_utf16_offset_static(text: &str, utf8_offset: usize) -> Result<u32> {
        let converter = PositionConverter;
        converter.utf8_offset_to_utf16_offset(text, utf8_offset)
    }

    pub fn utf16_offset_to_utf8_offset_static(text: &str, utf16_offset: u32) -> Result<usize> {
        let converter = PositionConverter;
        converter.utf16_offset_to_utf8_offset(text, utf16_offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_utf8_offset_to_line_char() {
        let text = "hello\nworld\n";

        // First line
        assert_eq!(
            PositionConverter::utf8_offset_to_line_char(text, 0).unwrap(),
            (1, 0)
        );
        assert_eq!(
            PositionConverter::utf8_offset_to_line_char(text, 5).unwrap(),
            (1, 5)
        );

        // Second line
        assert_eq!(
            PositionConverter::utf8_offset_to_line_char(text, 6).unwrap(),
            (2, 0)
        );
        assert_eq!(
            PositionConverter::utf8_offset_to_line_char(text, 11).unwrap(),
            (2, 5)
        );

        // End of file
        assert_eq!(
            PositionConverter::utf8_offset_to_line_char(text, 12).unwrap(),
            (3, 0)
        );
    }

    #[test]
    fn test_line_char_to_utf8_offset() {
        let text = "hello\nworld\n";

        // First line
        assert_eq!(
            PositionConverter::line_char_to_utf8_offset(text, 1, 0).unwrap(),
            0
        );
        assert_eq!(
            PositionConverter::line_char_to_utf8_offset(text, 1, 5).unwrap(),
            5
        );

        // Second line
        assert_eq!(
            PositionConverter::line_char_to_utf8_offset(text, 2, 0).unwrap(),
            6
        );
        assert_eq!(
            PositionConverter::line_char_to_utf8_offset(text, 2, 5).unwrap(),
            11
        );

        // End of file
        assert_eq!(
            PositionConverter::line_char_to_utf8_offset(text, 3, 0).unwrap(),
            12
        );
    }

    #[test]
    fn test_utf8_to_utf16_conversion() {
        let text = "hello\nworld\n";

        // ASCII characters (1:1 mapping)
        assert_eq!(
            PositionConverter::utf8_offset_to_utf16_offset_static(text, 0).unwrap(),
            0
        );
        assert_eq!(
            PositionConverter::utf8_offset_to_utf16_offset_static(text, 5).unwrap(),
            5
        );

        // After newline
        assert_eq!(
            PositionConverter::utf8_offset_to_utf16_offset_static(text, 6).unwrap(),
            6
        );
        assert_eq!(
            PositionConverter::utf8_offset_to_utf16_offset_static(text, 11).unwrap(),
            11
        );
    }

    #[test]
    fn test_unicode_handling() {
        let text = "héllo\nwörld\n"; // Contains multi-byte UTF-8 characters

        // Test UTF-8 to line/char conversion
        assert_eq!(
            PositionConverter::utf8_offset_to_line_char(text, 0).unwrap(),
            (1, 0)
        );
        assert_eq!(
            PositionConverter::utf8_offset_to_line_char(text, 2).unwrap(),
            (1, 2)
        ); // 'é' is 2 bytes, so position 2 is 'l'

        // Test line/char to UTF-8 conversion
        assert_eq!(
            PositionConverter::line_char_to_utf8_offset(text, 1, 0).unwrap(),
            0
        );
        assert_eq!(
            PositionConverter::line_char_to_utf8_offset(text, 1, 1).unwrap(),
            1
        ); // Character 1 is 'é', which starts at byte 1
    }

    #[test]
    fn test_error_handling() {
        let text = "hello\nworld\n";

        // Test out of bounds
        assert!(PositionConverter::utf8_offset_to_line_char(text, 100).is_err());
        assert!(PositionConverter::line_char_to_utf8_offset(text, 10, 0).is_err());

        // Test invalid line number
        assert!(PositionConverter::line_char_to_utf8_offset(text, 0, 0).is_err());
    }
}
