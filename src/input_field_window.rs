/// A single-line field can contain arbitrary pasted text. Bound the work sent
/// to the text shaper without changing the model or its byte-based selection.
pub(super) const MAX_SHAPING_BYTES: usize = 16 * 1024;

#[derive(Clone)]
pub(crate) struct FieldWindow {
    pub(super) text: String,
    pub(super) start: usize,
    pub(super) total_len: usize,
    pub(super) cursor: usize,
    pub(super) anchor: usize,
}

impl FieldWindow {
    pub(super) fn new(text: &str, cursor: usize, anchor: usize) -> Self {
        let cursor = floor_boundary(text, cursor);
        let anchor = floor_boundary(text, anchor);
        let start = if text.len() <= MAX_SHAPING_BYTES {
            0
        } else {
            floor_boundary(text, cursor.saturating_sub(MAX_SHAPING_BYTES / 2))
        };
        let end = floor_boundary(text, start.saturating_add(MAX_SHAPING_BYTES));
        let visible = text.get(start..end).unwrap_or_default();
        Self {
            text: visible.to_string(),
            start,
            total_len: text.len(),
            cursor: cursor.saturating_sub(start).min(visible.len()),
            anchor: anchor.saturating_sub(start).min(visible.len()),
        }
    }

    pub(super) fn source_byte(&self, visible_byte: usize) -> usize {
        self.start
            .saturating_add(floor_boundary(&self.text, visible_byte))
            .min(self.total_len)
    }
}

fn floor_boundary(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte = byte.saturating_sub(1);
    }
    byte
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_window_keeps_source_offsets_for_multibyte_and_crlf_text() {
        let text = "café中\r\n🙂".repeat(3000);
        for cursor in [0, 1, 4, 6, 7, text.len() / 2, text.len(), usize::MAX] {
            for anchor in [0, 4, text.len(), usize::MAX] {
                let window = FieldWindow::new(&text, cursor, anchor);
                assert!(window.text.len() <= MAX_SHAPING_BYTES);
                assert!(text.is_char_boundary(window.start));
                assert!(text.is_char_boundary(window.start + window.text.len()));
                assert!(window.text.is_char_boundary(window.cursor));
                assert!(window.text.is_char_boundary(window.anchor));
                assert_eq!(
                    text.get(window.start..window.start + window.text.len()),
                    Some(window.text.as_str())
                );
                assert_eq!(
                    window.source_byte(window.cursor),
                    floor_boundary(&text, cursor)
                );
                assert_eq!(
                    window.source_byte(window.anchor),
                    floor_boundary(&text, anchor)
                        .clamp(window.start, window.start + window.text.len())
                );
                assert_eq!(
                    window.source_byte(usize::MAX),
                    window.start + window.text.len()
                );
            }
        }
    }

    #[test]
    fn malformed_visible_indices_clamp_without_slicing_a_character() {
        let window = FieldWindow::new("café中🙂", usize::MAX, usize::MAX);
        for (byte, expected) in [(0, 0), (4, 3), (6, 5), (7, 5), (10, 8), (usize::MAX, 12)] {
            assert!(window.text.is_char_boundary(window.source_byte(byte)));
            assert_eq!(window.source_byte(byte), expected);
        }
        assert_eq!(window.cursor, window.text.len());
        assert_eq!(window.anchor, window.text.len());
    }
}
