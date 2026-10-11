// Literal Replace in Files uses the same limits as capture expansion, without
// interpreting dollar signs or retaining a replacement object for every match.
use std::borrow::Cow;
use regex::Regex;
use super::regex_replacement::{MAX_CACHED_MATCHES, MAX_EXPANDED_BYTES, MAX_SOURCE_BYTES, MAX_TEMPLATE_BYTES};

pub(crate) fn replace<'a>(re: &Regex, text: &'a str, replacement: &str) -> Result<(Cow<'a, str>, usize), String> {
    replace_with_limits(re, text, replacement, MAX_CACHED_MATCHES, MAX_EXPANDED_BYTES,
        MAX_SOURCE_BYTES.saturating_add(MAX_EXPANDED_BYTES))
}

fn replace_with_limits<'a>(re: &Regex, text: &'a str, replacement: &str,
    max_matches: usize, max_expanded: usize, max_output: usize) -> Result<(Cow<'a, str>, usize), String>
{
    if text.len() > MAX_SOURCE_BYTES || replacement.len() > MAX_TEMPLATE_BYTES {
        return Err("Replacement source or template exceeds the size limit".into());
    }
    let mut count = 0usize;
    let mut removed = 0usize;
    let mut expanded = 0usize;
    for found in re.find_iter(text) {
        count = count.checked_add(1).ok_or("Too many replacements")?;
        if count > max_matches { return Err("Too many replacements".into()); }
        removed = removed.checked_add(found.len()).ok_or("Replacement size overflow")?;
        expanded = expanded.checked_add(replacement.len()).ok_or("Replacement size overflow")?;
        if expanded > max_expanded { return Err("Expanded replacement exceeds the size limit".into()); }
    }
    if count == 0 { return Ok((Cow::Borrowed(text), 0)); }
    let output_len = text.len().checked_sub(removed).and_then(|len| len.checked_add(expanded))
        .ok_or("Replacement size overflow")?;
    if output_len > max_output { return Err("Replacement output exceeds the size limit".into()); }
    let mut output = String::new();
    output.try_reserve_exact(output_len).map_err(|_| "Not enough memory for replacement")?;
    if output.capacity() > max_output { return Err("Replacement allocation exceeds the size limit".into()); }
    let mut end = 0;
    for found in re.find_iter(text) {
        output.push_str(text.get(end..found.start()).ok_or("Invalid replacement range")?);
        output.push_str(replacement);
        end = found.end();
    }
    output.push_str(text.get(end..).ok_or("Invalid replacement range")?);
    Ok((Cow::Owned(output), count))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_replacement_keeps_dollars_unicode_and_line_endings() {
        let re = Regex::new("中").unwrap();
        let (text, count) = replace(&re, "café中\r\n中😀", "$5\\n").unwrap();
        assert_eq!(text, "café$5\\n\r\n$5\\n😀");
        assert_eq!(count, 2);
    }

    #[test]
    fn literal_replacement_exact_boundaries_and_refusals() {
        let re = Regex::new("a").unwrap();
        assert_eq!(replace_with_limits(&re, "aa", "bb", 2, 4, 4).unwrap().0, "bbbb");
        assert!(replace_with_limits(&re, "aa", "bb", 1, 4, 4).is_err());
        assert!(replace_with_limits(&re, "aa", "bb", 2, 3, 4).is_err());
        assert!(replace_with_limits(&re, "xaa", "bb", 2, 4, 4).is_err());
        assert!(replace(&re, "a", &"b".repeat(MAX_TEMPLATE_BYTES + 1)).is_err());
    }

    #[test]
    fn literal_replacement_empty_matches_deletion_identity_and_no_match() {
        let empty = Regex::new("").unwrap();
        let (text, count) = replace(&empty, "中😀", "x").unwrap();
        assert_eq!(text, "x中x😀x"); assert_eq!(count, 3);
        let re = Regex::new("a").unwrap();
        assert_eq!(replace(&re, "a中a", "").unwrap(), (Cow::Owned("中".into()), 2));
        assert_eq!(replace(&re, "aa", "a").unwrap().1, 2);
        assert!(matches!(replace(&re, "中", "x").unwrap().0, Cow::Borrowed(_)));
    }
}
