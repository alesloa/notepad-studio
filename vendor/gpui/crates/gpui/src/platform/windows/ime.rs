const MAX_IME_BYTES: usize = 1024 * 1024;

pub(super) fn read_ime_bytes(mut read: impl FnMut(Option<&mut [u8]>) -> i32) -> Option<Vec<u8>> {
    let size = usize::try_from(read(None)).ok()?;
    if size > MAX_IME_BYTES {
        return None;
    }
    if size == 0 {
        return Some(Vec::new());
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(size).ok()?;
    if bytes.capacity() > MAX_IME_BYTES {
        return None;
    }
    bytes.resize(size, 0);
    let copied = usize::try_from(read(Some(&mut bytes))).ok()?;
    if copied > size {
        return None;
    }
    bytes.truncate(copied);
    Some(bytes)
}

pub(super) fn cursor_adjacent_to_input(attrs: &[u8], cursor: usize, input: u8) -> bool {
    attrs.get(cursor) == Some(&input)
        || cursor.checked_sub(1).and_then(|before| attrs.get(before)) == Some(&input)
}

pub(super) fn decode_ime_utf16(bytes: &[u8]) -> Option<Vec<u16>> {
    let (pairs, remainder) = bytes.as_chunks::<2>();
    if !remainder.is_empty() || bytes.len() > MAX_IME_BYTES {
        return None;
    }
    let mut text = Vec::new();
    text.try_reserve_exact(bytes.len() / 2).ok()?;
    if text.capacity().checked_mul(std::mem::size_of::<u16>())? > MAX_IME_BYTES {
        return None;
    }
    text.extend(pairs.iter().map(|pair| u16::from_le_bytes(*pair)));
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_query_results_do_not_allocate_or_read() {
        for error in [-1, -2, i32::MIN] {
            let mut calls = 0;
            assert!(read_ime_bytes(|buffer| {
                calls += 1;
                assert!(buffer.is_none());
                error
            }).is_none());
            assert_eq!(calls, 1);
        }
    }

    #[test]
    fn oversized_query_is_refused_before_read() {
        for size in [MAX_IME_BYTES as i32 + 1, i32::MAX] {
            assert!(read_ime_bytes(|buffer| {
                assert!(buffer.is_none());
                size
            }).is_none());
        }
    }

    #[test]
    fn empty_query_needs_no_second_read() {
        assert_eq!(read_ime_bytes(|buffer| {
            assert!(buffer.is_none());
            0
        }), Some(Vec::new()));
    }

    #[test]
    fn second_read_errors_and_growth_are_refused() {
        for result in [-2, -1, 5] {
            assert!(read_ime_bytes(|buffer| if buffer.is_none() { 4 } else { result }).is_none());
        }
    }

    #[test]
    fn short_read_does_not_treat_zero_filled_tail_as_input() {
        let attrs = read_ime_bytes(|buffer| match buffer {
            None => 4,
            Some(buffer) => { buffer[0] = 1; 1 }
        }).unwrap();
        assert_eq!(attrs, [1]);
        assert!(!cursor_adjacent_to_input(&attrs, 2, 0));
    }

    #[test]
    fn zero_second_read_is_empty() {
        assert_eq!(read_ime_bytes(|buffer| if buffer.is_none() { 4 } else { 0 }), Some(Vec::new()));
    }

    #[test]
    fn exact_budget_is_allowed() {
        let bytes = read_ime_bytes(|buffer| match buffer {
            None => MAX_IME_BYTES as i32,
            Some(buffer) => { buffer.fill(7); buffer.len() as i32 }
        }).unwrap();
        assert_eq!(bytes.len(), MAX_IME_BYTES);
        assert_eq!(bytes.first(), Some(&7));
        assert_eq!(bytes.last(), Some(&7));
    }

    #[test]
    fn cursor_checks_both_neighbors_without_overflow() {
        let attrs = [1, 0, 1];
        assert!(cursor_adjacent_to_input(&attrs, 1, 0));
        assert!(cursor_adjacent_to_input(&attrs, 2, 0));
        assert!(!cursor_adjacent_to_input(&attrs, 0, 0));
        assert!(!cursor_adjacent_to_input(&attrs, 3, 0));
        assert!(!cursor_adjacent_to_input(&attrs, usize::MAX, 0));
        assert!(!cursor_adjacent_to_input(&[], 0, 0));
    }
    #[test]
    fn utf16_units_preserve_bmp_surrogates_and_crlf() {
        let bytes = [0x61, 0, 0x3d, 0xd8, 0, 0xde, 13, 0, 10, 0, 0xe9, 0];
        let units = decode_ime_utf16(&bytes).unwrap();
        assert_eq!(units, [0x61, 0xd83d, 0xde00, 13, 10, 0xe9]);
        assert_eq!(String::from_utf16(&units).unwrap(), "a\u{1f600}\r\n\u{e9}");
        assert_eq!(decode_ime_utf16(&[]), Some(Vec::new()));
    }

    #[test]
    fn utf16_odd_and_oversized_bytes_are_refused() {
        assert!(decode_ime_utf16(&[0x61]).is_none());
        assert!(decode_ime_utf16(&vec![0; MAX_IME_BYTES + 2]).is_none());
    }

}
