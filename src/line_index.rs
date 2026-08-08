// Copyright 2026 Kenny Root
//
// SPDX-License-Identifier: MIT

pub fn safe_slice(text: &str, range: impl std::ops::RangeBounds<usize>) -> &str {
    let start = match range.start_bound() {
        std::ops::Bound::Included(&s) => s,
        std::ops::Bound::Excluded(&s) => s.saturating_add(1),
        std::ops::Bound::Unbounded => 0,
    };
    let end = match range.end_bound() {
        std::ops::Bound::Included(&e) => e.saturating_add(1),
        std::ops::Bound::Excluded(&e) => e,
        std::ops::Bound::Unbounded => text.len(),
    };

    let start = start.min(text.len());
    let end = end.min(text.len());

    if start >= end {
        return "";
    }

    let mut s = start;
    while s < text.len() && !text.is_char_boundary(s) {
        s += 1;
    }
    let mut e = end;
    while e > s && !text.is_char_boundary(e) {
        e -= 1;
    }

    if s >= e {
        return "";
    }

    &text[s..e]
}

use lsp_types::{Position, Range};

pub struct LineIndex {
    line_starts: Vec<usize>,
    line_lens_utf16: Vec<usize>,
    len_bytes: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        let mut line_lens_utf16 = Vec::new();

        for line_str in text.split_inclusive('\n') {
            let content = line_str.strip_suffix('\n').unwrap_or(line_str);
            let content = content.strip_suffix('\r').unwrap_or(content);
            line_lens_utf16.push(content.encode_utf16().count());

            if line_str.ends_with('\n') {
                line_starts.push(line_starts.last().copied().unwrap_or(0) + line_str.len());
            }
        }

        if text.ends_with('\n') {
            line_lens_utf16.push(0);
        }

        if line_lens_utf16.is_empty() {
            line_lens_utf16.push(0);
        }

        Self {
            line_starts,
            line_lens_utf16,
            len_bytes: text.len(),
        }
    }

    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.len_bytes);
        let line = match self.line_starts.binary_search(&offset) {
            Ok(line) => line,
            Err(line) => line.saturating_sub(1),
        };
        let col = offset - self.line_starts[line];
        (line, col)
    }

    pub fn clamp_point(&self, text: &str, row: usize, col_bytes: usize) -> Position {
        if self.line_starts.is_empty() {
            return Position::new(0, 0);
        }

        let line = row.min(self.line_starts.len() - 1);
        let max_utf16_col = self.line_lens_utf16[line];

        let line_start_byte = self.line_starts[line];
        let line_end_byte = if line + 1 < self.line_starts.len() {
            self.line_starts[line + 1]
        } else {
            self.len_bytes
        };

        let line_str = safe_slice(text, line_start_byte..line_end_byte);
        let content = line_str.strip_suffix('\n').unwrap_or(line_str);
        let content = content.strip_suffix('\r').unwrap_or(content);

        let clamped_byte_col = col_bytes.min(content.len());

        let utf16_col = safe_slice(content, 0..clamped_byte_col)
            .encode_utf16()
            .count()
            .min(max_utf16_col);

        Position::new(line as u32, utf16_col as u32)
    }

    pub fn clamp_range(
        &self,
        text: &str,
        start_row: usize,
        start_col_bytes: usize,
        end_row: usize,
        end_col_bytes: usize,
    ) -> Range {
        let start = self.clamp_point(text, start_row, start_col_bytes);
        let mut end = self.clamp_point(text, end_row, end_col_bytes);

        if start_row == end_row && start_col_bytes == end_col_bytes {
            let line = start.line as usize;
            let max_utf16_col = self.line_lens_utf16.get(line).copied().unwrap_or(0);
            let extended_col = (start.character + 1).min(max_utf16_col as u32);
            end = Position::new(start.line, extended_col);
        }

        if end.line < start.line || (end.line == start.line && end.character < start.character) {
            end = start;
        }

        Range { start, end }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_slice_handles_range_inside_multibyte_character() {
        assert_eq!(safe_slice("😀", 1..2), "");
    }

    #[test]
    fn clamps_point_to_empty_line_after_trailing_newline() {
        let text = "x\n";
        let index = LineIndex::new(text);

        assert_eq!(index.clamp_point(text, 1, 0), Position::new(1, 0));
    }
}
