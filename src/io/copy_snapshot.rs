// Native save snapshots omit histories and capture large strings in bounded UI turns.
use crate::{buffer::Buffer, rich_doc::{RichDocPayload, ast::{Block, BlockProps, InlineRun, InlineStyle}}};

const CAPTURE_BYTES: usize = 64 * 1024;
const CAPTURE_ITEMS: usize = 128;
const MAX_CAPTURE_BYTES: usize = 512 * 1024 * 1024;
const LIMIT_ERROR: &str = "native document exceeds the 512 MiB copy snapshot limit; use Save As";
struct Budget { used: usize, limit: usize }
impl Budget {
    fn charge(&mut self, amount: usize) -> Result<(), String> {
        self.used = self.used.checked_add(amount).filter(|used| *used <= self.limit).ok_or(LIMIT_ERROR)?;
        Ok(())
    }
}
struct Run { text: Vec<String>, link: Vec<String>, style: InlineStyle, has_link: bool }
struct Paragraph { level: Option<u8>, props: BlockProps, runs: Vec<Run> }
pub(super) struct Snapshot {
    buffer: Buffer, raw: Vec<String>, paragraphs: Vec<Paragraph>, raw_offset: usize,
    block: usize, run: usize, text_offset: usize, link_offset: usize, edited: bool, capture_raw: bool,
    budget: Budget,
}

fn piece(text: &str, offset: &mut usize, budget: &mut usize, allocation: &mut Budget) -> Result<Option<String>, String> {
    if *offset >= text.len() || *budget == 0 { return Ok(None); }
    let mut end = offset.saturating_add(*budget).min(text.len());
    while end > *offset && !text.is_char_boundary(end) { end = end.saturating_sub(1); }
    if end == *offset { return Ok(None); }
    let text = text.get(*offset..end).ok_or("native snapshot has an invalid character boundary")?;
    // Include peak space for chunks, joined strings, and vector capacity growth.
    allocation.charge(text.len().checked_mul(3).and_then(|bytes| bytes.checked_add(2 * std::mem::size_of::<String>())).ok_or(LIMIT_ERROR)?)?;
    let mut part = String::new(); part.try_reserve_exact(text.len()).map_err(|_| "not enough memory to prepare this native copy")?;
    allocation.charge(part.capacity().saturating_sub(text.len()).checked_mul(3).ok_or(LIMIT_ERROR)?)?;
    part.push_str(text);
    *budget = budget.saturating_sub(end.saturating_sub(*offset)); *offset = end;
    Ok(Some(part))
}

fn append<T>(items: &mut Vec<T>, item: T, allocation: &mut Budget) -> Result<(), String> {
    if items.len() == items.capacity() {
        let growth = items.capacity().max(4);
        let capacity = items.capacity().checked_add(growth).ok_or(LIMIT_ERROR)?;
        // Charge the new allocation before reserving; the old allocation can coexist during growth.
        allocation.charge(capacity.checked_mul(std::mem::size_of::<T>()).ok_or(LIMIT_ERROR)?)?;
        items.try_reserve_exact(growth).map_err(|_| "not enough memory to prepare this native copy")?;
        allocation.charge(items.capacity().saturating_sub(capacity).checked_mul(std::mem::size_of::<T>()).ok_or(LIMIT_ERROR)?)?;
    }
    items.push(item); Ok(())
}
fn join(parts: Vec<String>, allocation: &mut Budget) -> Result<String, String> {
    let length = parts.iter().try_fold(0usize, |sum, part| sum.checked_add(part.len())).ok_or(LIMIT_ERROR)?;
    let mut joined = String::new(); joined.try_reserve_exact(length).map_err(|_| "not enough memory to prepare this native copy")?;
    allocation.charge(joined.capacity().saturating_sub(length))?;
    for part in parts { joined.push_str(&part); }
    Ok(joined)
}

impl Snapshot {
    pub(super) fn new(buffer: &Buffer) -> Self {
        let mut captured = super::copy_writer::snapshot(buffer);
        let mut edited = false; let mut capture_raw = false;
        if let Some(payload) = &buffer.rich_doc {
            edited = !payload.edit_log.is_empty(); capture_raw = buffer.dirty || edited;
            captured.rich_doc = Some(RichDocPayload { revision: payload.revision.clone(), format: payload.format,
                ast: Default::default(), raw_view_text: String::new(), original_bytes: payload.original_bytes.clone(),
                // Native writers only inspect emptiness; they serialize the AST instead of replaying this log.
                edit_log: if edited { vec![crate::rich_doc::edits::Edit::SetBlockBold { block: 0, on: false }] } else { Vec::new() },
                active_block: None, selection: None, drag_active: false, pending_drag_anchor: None,
                headers: Vec::new(), footers: Vec::new(), undo_stack: Vec::new(), redo_stack: Vec::new(), edit_group_start: None });
        }
        #[cfg(test)]
        let limit = tests::limit();
        #[cfg(not(test))]
        let limit = MAX_CAPTURE_BYTES;
        Self { buffer: captured, raw: Vec::new(), paragraphs: Vec::new(), raw_offset: 0, block: 0,
            run: 0, text_offset: 0, link_offset: 0, edited, capture_raw, budget: Budget { used: 0, limit } }
    }

    pub(super) fn step(&mut self, buffer: &Buffer) -> Result<bool, String> {
        let Some(payload) = &buffer.rich_doc else { return Ok(true); };
        let mut budget = CAPTURE_BYTES;
        for _ in 0..CAPTURE_ITEMS {
            if self.capture_raw && self.raw_offset < payload.raw_view_text.len() {
                if let Some(part) = piece(&payload.raw_view_text, &mut self.raw_offset, &mut budget, &mut self.budget)? { append(&mut self.raw, part, &mut self.budget)?; }
                else { return Ok(false); }
                continue;
            }
            if !self.edited { return Ok(true); }
            let Some(block) = payload.ast.blocks.get(self.block) else { return Ok(true); };
            let (level, runs, props) = match block {
                Block::Paragraph { runs, props } => (None, runs, props),
                Block::Heading { level, runs, props } => (Some(*level), runs, props),
                _ => return Err(crate::rich_doc::RichDocError::UnsupportedContent.to_string()),
            };
            if self.paragraphs.len() == self.block {
                self.budget.charge(2 * std::mem::size_of::<Paragraph>() + std::mem::size_of::<Block>())?;
                append(&mut self.paragraphs, Paragraph { level, props: props.clone(), runs: Vec::new() }, &mut self.budget)?;
            }
            let Some(run) = runs.get(self.run) else {
                self.block = self.block.saturating_add(1); self.run = 0; continue;
            };
            let paragraph = self.paragraphs.last_mut().ok_or("native snapshot lost its paragraph")?;
            if paragraph.runs.len() == self.run {
                let style = &run.style;
                self.budget.charge(2 * std::mem::size_of::<Run>() + std::mem::size_of::<InlineRun>())?;
                append(&mut paragraph.runs, Run { text: Vec::new(), link: Vec::new(), has_link: style.link.is_some(),
                    style: InlineStyle { bold: style.bold, italic: style.italic, underline: style.underline,
                        strike: style.strike, color: style.color, link: None } }, &mut self.budget)?;
            }
            let captured = paragraph.runs.last_mut().ok_or("native snapshot lost its run")?;
            if let Some(link) = &run.style.link {
                if self.link_offset < link.len() {
                    if let Some(part) = piece(link, &mut self.link_offset, &mut budget, &mut self.budget)? { append(&mut captured.link, part, &mut self.budget)?; }
                    else { return Ok(false); }
                    continue;
                }
            }
            if self.text_offset < run.text.len() {
                if let Some(part) = piece(&run.text, &mut self.text_offset, &mut budget, &mut self.budget)? { append(&mut captured.text, part, &mut self.budget)?; }
                else { return Ok(false); }
                continue;
            }
            self.run = self.run.saturating_add(1); self.text_offset = 0; self.link_offset = 0;
        }
        Ok(false)
    }

    // Called by the background worker: joining strings never blocks editor input.
    pub(super) fn finish(mut self) -> Result<Buffer, String> {
        if let Some(payload) = &mut self.buffer.rich_doc {
            payload.raw_view_text = join(self.raw, &mut self.budget)?;
            self.budget.charge(self.paragraphs.len().checked_mul(std::mem::size_of::<Block>()).ok_or(LIMIT_ERROR)?)?;
            payload.ast.blocks.try_reserve_exact(self.paragraphs.len()).map_err(|_| "not enough memory to prepare this native copy")?;
            self.budget.charge(payload.ast.blocks.capacity().saturating_sub(self.paragraphs.len()).checked_mul(std::mem::size_of::<Block>()).ok_or(LIMIT_ERROR)?)?;
            for paragraph in self.paragraphs {
                self.budget.charge(paragraph.runs.len().checked_mul(std::mem::size_of::<InlineRun>()).ok_or(LIMIT_ERROR)?)?;
                let mut runs = Vec::new(); runs.try_reserve_exact(paragraph.runs.len()).map_err(|_| "not enough memory to prepare this native copy")?;
                self.budget.charge(runs.capacity().saturating_sub(paragraph.runs.len()).checked_mul(std::mem::size_of::<InlineRun>()).ok_or(LIMIT_ERROR)?)?;
                for run in paragraph.runs {
                    let mut style = run.style; style.link = if run.has_link { Some(join(run.link, &mut self.budget)?) } else { None };
                    runs.push(InlineRun { text: join(run.text, &mut self.budget)?, style });
                }
                payload.ast.blocks.push(match paragraph.level { Some(level) => Block::Heading { level, runs, props: paragraph.props },
                    None => Block::Paragraph { runs, props: paragraph.props } });
            }
        }
        Ok(self.buffer)
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    thread_local! { static LIMIT: std::cell::Cell<usize> = const { std::cell::Cell::new(MAX_CAPTURE_BYTES) }; }
    pub(super) fn limit() -> usize { LIMIT.with(std::cell::Cell::get) }
    pub(crate) struct Limit(usize);
    impl Limit { pub(crate) fn new(limit: usize) -> Self { Self(LIMIT.with(|slot| slot.replace(limit))) } }
    impl Drop for Limit { fn drop(&mut self) { LIMIT.with(|slot| slot.set(self.0)); } }

    #[test]
    fn bounded_snapshot_preserves_unicode_links_raw_headers_and_native_structure_without_history() {
        let _settings = crate::settings::override_for_test(Default::default());
        let mut payload = crate::rich_doc::rtf::open_bytes(br"{\rtf1\ansi seed}".to_vec()).expect("native");
        payload.raw_view_text = "<source café中😀>\r\n".repeat(10_000);
        payload.ast.blocks = vec![Block::Heading { level: 2, props: Default::default(), runs: vec![InlineRun {
            text: "café中😀 ".repeat(10_000), style: InlineStyle { bold: true, link: Some("https://example.test/café中😀".repeat(4000)), ..Default::default() } }] },
            Block::Paragraph { props: Default::default(), runs: Vec::new() }];
        payload.edit_log.push(crate::rich_doc::edits::Edit::SetBlockBold { block: 0, on: true });
        let mut buffer = Buffer::empty(); buffer.rope = ropey::Rope::from_str(&payload.raw_view_text); buffer.rich_doc = Some(payload); buffer.refresh_dirty();
        let mut snapshot = Snapshot::new(&buffer); let mut turns = 0;
        while !snapshot.step(&buffer).expect("capture") { turns += 1; assert!(turns < 1000); }
        assert!(turns > 5);
        let captured = snapshot.finish().expect("join"); let source = buffer.rich_doc.as_ref().expect("source"); let copied = captured.rich_doc.as_ref().expect("captured");
        assert_eq!(source.ast, copied.ast); assert_eq!(source.raw_view_text, copied.raw_view_text);
        assert!(std::sync::Arc::ptr_eq(&source.original_bytes, &copied.original_bytes));
        assert!(copied.undo_stack.is_empty() && copied.redo_stack.is_empty() && copied.headers.is_empty() && copied.footers.is_empty());
    }
}
