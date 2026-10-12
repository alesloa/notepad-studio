// ScintillaEditView.cpp:4806–4871; Notepad_plus.cpp:1759–1842.
use std::{collections::{HashMap, hash_map::DefaultHasher}, hash::Hasher, sync::atomic::AtomicBool};
use crate::buffer::prepared::PrefixEdit;
use super::line_sort::{Line, check_cancel};

const MEMORY: &str = "Not enough memory to prepare duplicate line removal";
const MERGE: &str = "Line operation skipped: mixed CR and LF endings would merge two lines";

fn equal(left: &str, right: &str, cancel: &AtomicBool) -> Result<bool, &'static str> {
    if left.len() != right.len() { return Ok(false); }
    for (a,b) in left.as_bytes().chunks(8192).zip(right.as_bytes().chunks(8192)) {
        check_cancel(cancel)?; if a != b { return Ok(false); }
    }
    Ok(true)
}

fn digest(body: &str, cancel: &AtomicBool) -> Result<u64, &'static str> {
    let mut hash = DefaultHasher::new();
    for chunk in body.as_bytes().chunks(8192) { check_cancel(cancel)?; hash.write(chunk); }
    Ok(hash.finish())
}

pub(super) fn any_duplicate(text: &str, lines: &[Line<'_>], start: usize, after_cr: bool,
    cancel: &AtomicBool) -> Result<Vec<PrefixEdit>, &'static str>
{
    let mut seen: HashMap<u64, Vec<usize>> = HashMap::new();
    let mut kept = Vec::new(); kept.try_reserve_exact(lines.len()).map_err(|_| MEMORY)?;
    for (index,line) in lines.iter().enumerate() {
        check_cancel(cancel)?;
        let hash = digest(line.body,cancel)?;
        let mut duplicate = false;
        if let Some(bucket) = seen.get(&hash) {
            for &previous in bucket {
                check_cancel(cancel)?;
                let previous = lines.get(previous).ok_or("Invalid duplicate line key")?;
                if equal(previous.body,line.body,cancel)? { duplicate = true; break; }
            }
        }
        if duplicate { continue; }
        if !seen.contains_key(&hash) {
            seen.try_reserve(1).map_err(|_| MEMORY)?;
        }
        let bucket = seen.entry(hash).or_default();
        bucket.try_reserve(1).map_err(|_| MEMORY)?; bucket.push(index); kept.push(index);
    }
    let mut edits = Vec::new();
    if kept.len() == lines.len() { return Ok(edits); }
    let mut output = String::new(); output.try_reserve_exact(text.len()).map_err(|_| MEMORY)?;
    let mut previous_cr = after_cr;
    for (position,index) in kept.iter().copied().enumerate() {
        check_cancel(cancel)?;
        let body = lines.get(index).ok_or("Invalid duplicate line order")?.body;
        let terminator = if position.saturating_add(1) == kept.len() { lines.last() } else { lines.get(position) }
            .ok_or("Invalid duplicate line terminator")?.eol;
        if previous_cr && body.is_empty() && terminator.starts_with('\n') { return Err(MERGE); }
        output.push_str(body); output.push_str(terminator);
        if !(body.is_empty() && terminator.is_empty()) { previous_cr = terminator == "\r"; }
    }
    edits.try_reserve_exact(1).map_err(|_| MEMORY)?;
    let end = start.checked_add(text.len()).ok_or("Duplicate line position overflow")?;
    edits.push(PrefixEdit { range: start..end, text: output });
    Ok(edits)
}

pub(super) fn consecutive(text: &str, lines: &[Line<'_>], start: usize, after_cr: bool,
    cancel: &AtomicBool) -> Result<Vec<PrefixEdit>, &'static str>
{
    let mut removed = Vec::new(); removed.try_reserve_exact(lines.len()).map_err(|_| MEMORY)?;
    removed.resize(lines.len(),false);
    let mut previous = 0usize;
    for (index,line) in lines.iter().enumerate().skip(1) {
        check_cancel(cancel)?;
        let before = lines.get(previous).ok_or("Invalid duplicate line position")?;
        if line.eol == before.eol && equal(line.body,before.body,cancel)? {
            *removed.get_mut(index).ok_or("Invalid duplicate line position")? = true;
        } else if index.saturating_add(1) == lines.len() && line.eol.is_empty()
            && !before.eol.is_empty() && equal(line.body,before.body,cancel)? {
            *removed.get_mut(previous).ok_or("Invalid duplicate line position")? = true;
            previous = index;
        } else { previous = index; }
    }
    let mut edits: Vec<PrefixEdit> = Vec::new();
    for (index,line) in lines.iter().enumerate() {
        check_cancel(cancel)?;
        if !removed.get(index).copied().ok_or("Invalid duplicate line position")? { continue; }
        let from = start.checked_add(line.start).ok_or("Duplicate line position overflow")?;
        let to = from.checked_add(line.body.len()).and_then(|n| n.checked_add(line.eol.len()))
            .ok_or("Duplicate line position overflow")?;
        if let Some(previous) = edits.last_mut().filter(|previous| previous.range.end == from) { previous.range.end = to; }
        else { edits.try_reserve(1).map_err(|_| MEMORY)?; edits.push(PrefixEdit { range: from..to, text: String::new() }); }
    }
    for edit in &edits {
        check_cancel(cancel)?;
        let from = edit.range.start.checked_sub(start).ok_or("Invalid duplicate line range")?;
        let to = edit.range.end.checked_sub(start).ok_or("Invalid duplicate line range")?;
        let before_cr = if from == 0 { after_cr } else { text.as_bytes().get(from.saturating_sub(1)) == Some(&b'\r') };
        if before_cr && text.as_bytes().get(to) == Some(&b'\n') { return Err(MERGE); }
    }
    Ok(edits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::line_sort::lines_in;

    fn apply(text: &str, consecutive_only: bool) -> String {
        let cancel = AtomicBool::new(false); let lines = lines_in(text,true,&cancel).unwrap();
        let edits = if consecutive_only { consecutive(text,&lines,0,false,&cancel) }
            else { any_duplicate(text,&lines,0,false,&cancel) }.unwrap();
        let mut result = text.to_owned();
        for edit in edits.into_iter().rev() { result.replace_range(edit.range,&edit.text); }
        result
    }

    #[test]
    fn first_occurrence_and_unterminated_eof_rules() {
        for ending in ["\n","\r\n","\r"] {
            assert_eq!(apply(&format!("b{ending}a{ending}b{ending}c"),false),format!("b{ending}a{ending}c"));
            for source in [format!("a{ending}a"),format!("a{ending}a{ending}a")] {
                assert_eq!(apply(&source,true),"a");
            }
            assert_eq!(apply(&format!("a{ending}a{ending}"),true),format!("a{ending}"));
            assert_eq!(apply(&format!("a{ending}{ending}"),true),format!("a{ending}"));
        }
    }

    #[test]
    fn full_unicode_body_and_original_terminators_are_compared() {
        assert_eq!(apply("é\na\né\né\0tail",false),"é\na\né\0tail");
        assert_eq!(apply("a\r\na\na",true),"a\r\na");
        assert_eq!(apply("a\u{2028}b\na\u{2028}b",true),"a\u{2028}b");
        assert_eq!(apply("a\r\nb\rc\nb",false),"a\r\nb\rc");
    }

    #[test]
    fn cancellation_and_cr_lf_merge_refuse_without_an_edit() {
        let cancel=AtomicBool::new(false); let text="a\ra\n\nb";
        let lines=lines_in(text,true,&cancel).unwrap();
        assert!(matches!(any_duplicate(text,&lines,0,false,&cancel),Err(MERGE)));
        let text="a\ra\r\n";
        let lines=lines_in(text,true,&cancel).unwrap();
        // The existing CRLF is indivisible; deleting a row cannot split it.
        assert!(consecutive(text,&lines,0,false,&cancel).is_ok());
        let text="a\ra\r\nb\n";
        let lines=lines_in(text,true,&cancel).unwrap();
        cancel.store(true,std::sync::atomic::Ordering::Relaxed);
        assert!(consecutive(text,&lines,0,false,&cancel).is_err());
        assert!(any_duplicate(text,&lines,0,false,&cancel).is_err());
    }
}
