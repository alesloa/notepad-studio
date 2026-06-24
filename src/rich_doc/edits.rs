//! Structured edit operations for rich-document AST mutation.
//!
//! Phase 2 granularity is **whole block** — every operation targets the runs
//! inside a single block by index. Sub-range / cross-block selection lands in
//! Phase 2g once the renderer can map clicks back to (block, run, byte_offset).
//!
//! The edit log on `RichDocPayload` lets save decide between "byte-perfect
//! re-write of the original zip" (empty log) and "re-serialize
//! `word/document.xml` from the mutated AST" (non-empty log).

use super::ast::{Alignment, Block, BlockProps, Document, ListMarker};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// Set bold on every run in `block`. `on = false` clears bold.
    SetBlockBold { block: usize, on: bool },
    /// Set italic on every run in `block`.
    SetBlockItalic { block: usize, on: bool },
    /// Set underline on every run in `block`.
    SetBlockUnderline { block: usize, on: bool },
    /// Set strikethrough on every run in `block`.
    SetBlockStrike { block: usize, on: bool },
    /// Phase 2g.H — apply bold to a sub-range that may span multiple runs
    /// (and multiple blocks). Splits runs at the from/to byte offsets so the
    /// affected region becomes its own run(s), then sets `style.bold = on`
    /// on those runs only. No-op outside paragraph / heading blocks.
    SetRangeBold {
        from_block: usize,
        from_run: usize,
        from_byte: usize,
        to_block: usize,
        to_run: usize,
        to_byte: usize,
        on: bool,
    },
    /// Phase 2g.H — italic sub-range (same shape as `SetRangeBold`).
    SetRangeItalic {
        from_block: usize,
        from_run: usize,
        from_byte: usize,
        to_block: usize,
        to_run: usize,
        to_byte: usize,
        on: bool,
    },
    /// Phase 2g.H — underline sub-range.
    SetRangeUnderline {
        from_block: usize,
        from_run: usize,
        from_byte: usize,
        to_block: usize,
        to_run: usize,
        to_byte: usize,
        on: bool,
    },
    /// Strikethrough sub-range (same shape as `SetRangeUnderline`).
    SetRangeStrike {
        from_block: usize,
        from_run: usize,
        from_byte: usize,
        to_block: usize,
        to_run: usize,
        to_byte: usize,
        on: bool,
    },
    /// Set the RGB color (0xRRGGBB) on every run in `block`. `None` clears.
    SetBlockColor { block: usize, color: Option<u32> },
    /// Set the hyperlink URL on every run in `block`. `None` unlinks.
    SetBlockLink { block: usize, url: Option<String> },
    /// Replace the paragraph alignment on `block` (Left/Center/Right/Justify).
    SetBlockAlignment { block: usize, alignment: Alignment },
    /// Bump the left indent on `block` by `delta` levels (saturates at 0,
    /// caps at 16 — beyond that the indent is more confusing than useful).
    SetBlockIndent { block: usize, delta: i8 },
    /// Toggle list membership on `block`. If `block` is already in a list of
    /// the requested kind (ordered = bulleted-vs-numbered match) the list is
    /// cleared; otherwise the block is promoted into a list of `ordered` kind
    /// at level 0. Switching kinds (bullet → number) replaces the marker.
    ToggleBlockList { block: usize, ordered: bool },
    /// Phase 2g.A — insert `text` into `block`'s `runs[run]` at intra-run
    /// `byte`. Clamps `byte` to a UTF-8 char boundary inside the target run.
    /// No-op if (block, run) is out of range, the block isn't a paragraph /
    /// heading, or the run is empty and `text` is empty.
    InsertText {
        block: usize,
        run: usize,
        byte: usize,
        text: String,
    },
    /// Phase 2g.A — delete the byte range `[(run_lo, byte_lo), (run_hi, byte_hi))`
    /// inside `block`. Splices across runs but never crosses blocks. Empty runs
    /// remaining after the delete are left in place (style preservation); run
    /// merging is a Phase 2g polish step.
    DeleteRange {
        block: usize,
        run_lo: usize,
        byte_lo: usize,
        run_hi: usize,
        byte_hi: usize,
    },
    /// Phase 2g.B — split `block` at (run, byte). Content before the split
    /// stays in `block`; content after becomes a new Paragraph inserted at
    /// `block + 1` with cloned BlockProps. Headings split into a heading
    /// followed by a paragraph — pressing Enter inside a title shouldn't
    /// continue the title style. List membership is preserved on both halves
    /// so Enter inside a bullet adds another bullet.
    SplitBlock { block: usize, run: usize, byte: usize },
    /// Phase 2g.B — merge `block + 1` into `block`. Runs from the next block
    /// are appended to this block; the next block is removed. No-op when
    /// either side isn't a paragraph/heading. BlockProps from `block` win.
    MergeWithNext { block: usize },
    /// Phase 2g.B — delete a span that crosses block boundaries. The
    /// post-edit state collapses to a single block at `from_block` with
    /// content before `from` plus content after `to`. Intermediate blocks
    /// are removed. No-op when from_block >= to_block or either side isn't
    /// a paragraph/heading.
    DeleteAcrossBlocks {
        from_block: usize,
        from_run: usize,
        from_byte: usize,
        to_block: usize,
        to_run: usize,
        to_byte: usize,
    },
    /// Convert `block` between Paragraph and Heading kinds while preserving
    /// runs + BlockProps. `level = Some(1..=6)` makes it a Heading at that
    /// level (clamped); `level = None` makes it a Paragraph. Tables / Images
    /// are no-ops. Used by the toolbar Promote / Demote Heading buttons.
    SetHeadingLevel { block: usize, level: Option<u8> },
    /// Phase 2g.K — splice formatted blocks into the document at a position.
    /// Used by paste-with-formatting. The host block at `at_block` is split at
    /// (at_run, at_byte); the first incoming paragraph merges into the prefix
    /// (so paste at end-of-paragraph extends it), the last incoming paragraph
    /// merges with the suffix (so paste at start-of-paragraph extends it), and
    /// middle blocks become standalone blocks. Non-paragraph hosts (Table /
    /// Image) cause the new blocks to be inserted *after* the host without
    /// splitting.
    InsertBlocks {
        at_block: usize,
        at_run: usize,
        at_byte: usize,
        blocks: Vec<Block>,
    },
}

/// Apply `edit` to `doc` in place. Out-of-range block indices are a no-op so
/// callers don't need to validate; the renderer can shift blocks asynchronously
/// and a stale edit just drops.
pub fn apply(edit: &Edit, doc: &mut Document) {
    match edit {
        Edit::SetBlockBold { block, on } => mutate_runs(doc, *block, |s| s.bold = *on),
        Edit::SetBlockItalic { block, on } => mutate_runs(doc, *block, |s| s.italic = *on),
        Edit::SetBlockUnderline { block, on } => mutate_runs(doc, *block, |s| s.underline = *on),
        Edit::SetBlockStrike { block, on } => mutate_runs(doc, *block, |s| s.strike = *on),
        Edit::SetRangeBold {
            from_block,
            from_run,
            from_byte,
            to_block,
            to_run,
            to_byte,
            on,
        } => {
            apply_range_style(
                doc,
                (*from_block, *from_run, *from_byte),
                (*to_block, *to_run, *to_byte),
                |s| s.bold = *on,
            );
        }
        Edit::SetRangeItalic {
            from_block,
            from_run,
            from_byte,
            to_block,
            to_run,
            to_byte,
            on,
        } => {
            apply_range_style(
                doc,
                (*from_block, *from_run, *from_byte),
                (*to_block, *to_run, *to_byte),
                |s| s.italic = *on,
            );
        }
        Edit::SetRangeUnderline {
            from_block,
            from_run,
            from_byte,
            to_block,
            to_run,
            to_byte,
            on,
        } => {
            apply_range_style(
                doc,
                (*from_block, *from_run, *from_byte),
                (*to_block, *to_run, *to_byte),
                |s| s.underline = *on,
            );
        }
        Edit::SetRangeStrike {
            from_block,
            from_run,
            from_byte,
            to_block,
            to_run,
            to_byte,
            on,
        } => {
            apply_range_style(
                doc,
                (*from_block, *from_run, *from_byte),
                (*to_block, *to_run, *to_byte),
                |s| s.strike = *on,
            );
        }
        Edit::SetBlockColor { block, color } => mutate_runs(doc, *block, |s| s.color = *color),
        Edit::SetBlockLink { block, url } => {
            let url = url.clone();
            mutate_runs(doc, *block, |s| s.link = url.clone());
        }
        Edit::SetBlockAlignment { block, alignment } => {
            mutate_props(doc, *block, |p| p.alignment = *alignment);
        }
        Edit::SetBlockIndent { block, delta } => {
            mutate_props(doc, *block, |p| {
                let next = (p.indent_level as i32) + (*delta as i32);
                p.indent_level = next.clamp(0, 16) as u8;
            });
        }
        Edit::ToggleBlockList { block, ordered } => {
            mutate_props(doc, *block, |p| {
                p.list = match p.list {
                    Some(m) if m.ordered == *ordered => None,
                    _ => Some(ListMarker { ordered: *ordered, level: 0 }),
                };
            });
        }
        Edit::InsertText { block, run, byte, text } => {
            insert_text_in_run(doc, *block, *run, *byte, text);
        }
        Edit::DeleteRange { block, run_lo, byte_lo, run_hi, byte_hi } => {
            delete_range_in_block(doc, *block, *run_lo, *byte_lo, *run_hi, *byte_hi);
        }
        Edit::SplitBlock { block, run, byte } => {
            split_block(doc, *block, *run, *byte);
        }
        Edit::MergeWithNext { block } => {
            merge_with_next(doc, *block);
        }
        Edit::DeleteAcrossBlocks {
            from_block,
            from_run,
            from_byte,
            to_block,
            to_run,
            to_byte,
        } => {
            delete_across_blocks(
                doc, *from_block, *from_run, *from_byte, *to_block, *to_run, *to_byte,
            );
        }
        Edit::InsertBlocks { at_block, at_run, at_byte, blocks } => {
            insert_blocks(doc, *at_block, *at_run, *at_byte, blocks);
        }
        Edit::SetHeadingLevel { block, level } => {
            set_heading_level(doc, *block, *level);
        }
    }
}

fn set_heading_level(doc: &mut Document, block: usize, level: Option<u8>) {
    let Some(b) = doc.blocks.get_mut(block) else {
        return;
    };
    let (runs, props) = match b {
        Block::Paragraph { runs, props } | Block::Heading { runs, props, .. } => {
            (std::mem::take(runs), props.clone())
        }
        Block::Table { .. } | Block::Image { .. } => return,
    };
    *b = match level {
        Some(n) => Block::Heading { level: n.clamp(1, 6), runs, props },
        None => Block::Paragraph { runs, props },
    };
}

/// Splice `incoming` blocks into `doc` at the given position. See
/// `Edit::InsertBlocks` for the user-visible contract. Returns silently on
/// out-of-range positions so stale edits drop cleanly during async block
/// shifts in the renderer.
fn insert_blocks(
    doc: &mut Document,
    at_block: usize,
    at_run: usize,
    at_byte: usize,
    incoming: &[Block],
) {
    if incoming.is_empty() {
        return;
    }
    // Append-at-end case: trailing or out-of-range index lands the new
    // blocks after every existing block.
    if at_block >= doc.blocks.len() {
        for b in incoming {
            doc.blocks.push(b.clone());
        }
        return;
    }
    let host_is_text = matches!(
        doc.blocks[at_block],
        Block::Paragraph { .. } | Block::Heading { .. }
    );
    if !host_is_text {
        // Table / Image hosts can't be split — drop incoming blocks after.
        let mut insert_at = at_block + 1;
        for b in incoming {
            doc.blocks.insert(insert_at, b.clone());
            insert_at += 1;
        }
        return;
    }
    // Single-paragraph paste: splice runs into the host without leaving a
    // split behind. Split + extend + merge-with-next yields exactly one
    // block: prefix_runs + new_runs + suffix_runs.
    if incoming.len() == 1 {
        if let Block::Paragraph { runs: new_runs, .. } = &incoming[0] {
            split_block(doc, at_block, at_run, at_byte);
            let cleaned: Vec<super::ast::InlineRun> = new_runs
                .iter()
                .filter(|r| !r.text.is_empty())
                .cloned()
                .collect();
            if let Some(
                Block::Paragraph { runs, .. } | Block::Heading { runs, .. },
            ) = doc.blocks.get_mut(at_block)
            {
                runs.extend(cleaned);
            }
            merge_with_next(doc, at_block);
            return;
        }
        // Non-paragraph single block (heading / table / image) — splice it
        // between the prefix and suffix without merging.
        split_block(doc, at_block, at_run, at_byte);
        doc.blocks.insert(at_block + 1, incoming[0].clone());
        return;
    }

    split_block(doc, at_block, at_run, at_byte);
    // After split: prefix at `at_block`, suffix at `at_block + 1`.
    let prefix_idx = at_block;
    let suffix_idx = at_block + 1;
    let mut to_insert: Vec<Block> = incoming.to_vec();

    // Merge first incoming paragraph into the prefix block.
    if let Some(first) = to_insert.first().cloned() {
        if let Block::Paragraph { runs: new_runs, .. } = first {
            if let Some(Block::Paragraph { runs: host_runs, .. } | Block::Heading { runs: host_runs, .. }) =
                doc.blocks.get_mut(prefix_idx)
            {
                let cleaned: Vec<super::ast::InlineRun> =
                    new_runs.into_iter().filter(|r| !r.text.is_empty()).collect();
                host_runs.extend(cleaned);
                to_insert.remove(0);
            }
        }
    }

    // Merge last incoming paragraph into the suffix block.
    if let Some(last) = to_insert.last().cloned() {
        if let Block::Paragraph { runs: new_runs, .. } = last {
            if let Some(Block::Paragraph { runs: suffix_runs, .. } | Block::Heading { runs: suffix_runs, .. }) =
                doc.blocks.get_mut(suffix_idx)
            {
                let cleaned: Vec<super::ast::InlineRun> =
                    new_runs.into_iter().filter(|r| !r.text.is_empty()).collect();
                let old_suffix = std::mem::replace(suffix_runs, cleaned);
                suffix_runs.extend(old_suffix);
                to_insert.pop();
            }
        }
    }

    // Anything left is inserted between prefix and suffix as standalone blocks.
    let mut insert_at = prefix_idx + 1;
    for b in to_insert {
        doc.blocks.insert(insert_at, b);
        insert_at += 1;
    }
}

fn split_block(doc: &mut Document, block: usize, run: usize, byte: usize) {
    let Some(b) = doc.blocks.get(block) else {
        return;
    };
    let (existing_runs, props_clone): (Vec<super::ast::InlineRun>, BlockProps) = match b {
        Block::Paragraph { runs, props } => (runs.clone(), props.clone()),
        Block::Heading { runs, props, .. } => (runs.clone(), props.clone()),
        Block::Table { .. } | Block::Image { .. } => return,
    };
    let run = run.min(existing_runs.len().saturating_sub(1).max(0));
    let mut left: Vec<super::ast::InlineRun> = Vec::new();
    let mut right: Vec<super::ast::InlineRun> = Vec::new();
    for (i, r) in existing_runs.iter().enumerate() {
        if i < run {
            left.push(r.clone());
        } else if i == run {
            if existing_runs.is_empty() {
                continue;
            }
            let b_clamped = round_down_char_boundary(&r.text, byte.min(r.text.len()));
            let (l, h) = r.text.split_at(b_clamped);
            if !l.is_empty() {
                left.push(super::ast::InlineRun {
                    text: l.to_string(),
                    style: r.style.clone(),
                });
            }
            if !h.is_empty() {
                right.push(super::ast::InlineRun {
                    text: h.to_string(),
                    style: r.style.clone(),
                });
            }
        } else {
            right.push(r.clone());
        }
    }
    // Write left back into the original block (preserving its kind).
    {
        let target = doc.blocks.get_mut(block).unwrap();
        match target {
            Block::Paragraph { runs, .. } | Block::Heading { runs, .. } => {
                *runs = left;
            }
            _ => {}
        }
    }
    // Insert the right half as a Paragraph after the original — pressing Enter
    // inside a heading shouldn't continue heading style, and Word/Pages behave
    // the same way.
    let new_block = Block::Paragraph { runs: right, props: props_clone };
    let insert_at = (block + 1).min(doc.blocks.len());
    doc.blocks.insert(insert_at, new_block);
}

fn merge_with_next(doc: &mut Document, block: usize) {
    if block + 1 >= doc.blocks.len() {
        return;
    }
    // Pull runs from block+1; only proceed when both sides are paragraph/heading.
    let next_runs: Vec<super::ast::InlineRun> = match &doc.blocks[block + 1] {
        Block::Paragraph { runs, .. } | Block::Heading { runs, .. } => runs.clone(),
        Block::Table { .. } | Block::Image { .. } => return,
    };
    let into = doc.blocks.get_mut(block).unwrap();
    match into {
        Block::Paragraph { runs, .. } | Block::Heading { runs, .. } => {
            runs.extend(next_runs);
        }
        Block::Table { .. } | Block::Image { .. } => return,
    }
    doc.blocks.remove(block + 1);
}

fn delete_across_blocks(
    doc: &mut Document,
    from_block: usize,
    from_run: usize,
    from_byte: usize,
    to_block: usize,
    to_run: usize,
    to_byte: usize,
) {
    if from_block >= to_block || to_block >= doc.blocks.len() {
        return;
    }
    // Both ends must be paragraph/heading.
    let to_block_runs: Vec<super::ast::InlineRun> = match &doc.blocks[to_block] {
        Block::Paragraph { runs, .. } | Block::Heading { runs, .. } => runs.clone(),
        Block::Table { .. } | Block::Image { .. } => return,
    };
    match &doc.blocks[from_block] {
        Block::Paragraph { .. } | Block::Heading { .. } => {}
        Block::Table { .. } | Block::Image { .. } => return,
    }
    // 1) Trim runs in from_block after (from_run, from_byte).
    {
        let into = doc.blocks.get_mut(from_block).unwrap();
        let runs = match into {
            Block::Paragraph { runs, .. } | Block::Heading { runs, .. } => runs,
            _ => return,
        };
        if !runs.is_empty() {
            let from_run = from_run.min(runs.len().saturating_sub(1));
            let from_b = round_down_char_boundary(
                &runs[from_run].text,
                from_byte.min(runs[from_run].text.len()),
            );
            let len = runs[from_run].text.len();
            runs[from_run].text.replace_range(from_b..len, "");
            // Drop runs after from_run.
            runs.truncate(from_run + 1);
        }
    }
    // 2) Build the surviving tail of to_block (runs from to_run with byte_hi trimmed off the left, plus runs after to_run).
    let mut tail_runs: Vec<super::ast::InlineRun> = Vec::new();
    if !to_block_runs.is_empty() {
        let to_run = to_run.min(to_block_runs.len().saturating_sub(1));
        for (i, r) in to_block_runs.iter().enumerate() {
            if i < to_run {
                continue;
            } else if i == to_run {
                let to_b =
                    round_down_char_boundary(&r.text, to_byte.min(r.text.len()));
                let kept = &r.text[to_b..];
                if !kept.is_empty() {
                    tail_runs.push(super::ast::InlineRun {
                        text: kept.to_string(),
                        style: r.style.clone(),
                    });
                }
            } else {
                tail_runs.push(r.clone());
            }
        }
    }
    // 3) Append the tail to from_block.
    {
        let into = doc.blocks.get_mut(from_block).unwrap();
        let runs = match into {
            Block::Paragraph { runs, .. } | Block::Heading { runs, .. } => runs,
            _ => return,
        };
        runs.extend(tail_runs);
    }
    // 4) Remove blocks (from_block + 1) .. (to_block) inclusive.
    for _ in 0..(to_block - from_block) {
        doc.blocks.remove(from_block + 1);
    }
}

fn insert_text_in_run(doc: &mut Document, block: usize, run: usize, byte: usize, text: &str) {
    if text.is_empty() {
        return;
    }
    let Some(b) = doc.blocks.get_mut(block) else {
        return;
    };
    let runs = match b {
        Block::Paragraph { runs, .. } | Block::Heading { runs, .. } => runs,
        Block::Table { .. } | Block::Image { .. } => return,
    };
    // Block may have no runs — synthesize an empty plain run at index 0 so
    // typing into a freshly-cleared paragraph still works. Phase 2g.A doesn't
    // do paragraph split / merge, so this is the only "empty-block" path we
    // need to handle.
    if runs.is_empty() {
        runs.push(super::ast::InlineRun {
            text: String::new(),
            style: super::ast::InlineStyle::default(),
        });
    }
    let Some(target) = runs.get_mut(run) else {
        // Out-of-range run index — append to the last run so the keystroke
        // doesn't silently disappear. Cursor recovery happens next render.
        let last = runs.len() - 1;
        runs[last].text.push_str(text);
        return;
    };
    let byte = byte.min(target.text.len());
    let byte = round_down_char_boundary(&target.text, byte);
    target.text.insert_str(byte, text);
}

fn delete_range_in_block(
    doc: &mut Document,
    block: usize,
    run_lo: usize,
    byte_lo: usize,
    run_hi: usize,
    byte_hi: usize,
) {
    let Some(b) = doc.blocks.get_mut(block) else {
        return;
    };
    let runs = match b {
        Block::Paragraph { runs, .. } | Block::Heading { runs, .. } => runs,
        Block::Table { .. } | Block::Image { .. } => return,
    };
    if runs.is_empty() {
        return;
    }
    let run_hi = run_hi.min(runs.len().saturating_sub(1));
    let run_lo = run_lo.min(run_hi);
    if run_lo == run_hi {
        let r = &mut runs[run_lo];
        let lo = round_down_char_boundary(&r.text, byte_lo.min(r.text.len()));
        let hi = round_down_char_boundary(&r.text, byte_hi.min(r.text.len()));
        if hi > lo {
            r.text.replace_range(lo..hi, "");
        }
        return;
    }
    // Multi-run delete: trim trailing in lo, leading in hi, clear middles.
    let lo_text_len = runs[run_lo].text.len();
    let lo_b = round_down_char_boundary(&runs[run_lo].text, byte_lo.min(lo_text_len));
    runs[run_lo].text.replace_range(lo_b..lo_text_len, "");
    for r in runs.iter_mut().take(run_hi).skip(run_lo + 1) {
        r.text.clear();
    }
    let hi_text_len = runs[run_hi].text.len();
    let hi_b = round_down_char_boundary(&runs[run_hi].text, byte_hi.min(hi_text_len));
    runs[run_hi].text.replace_range(0..hi_b, "");
}

/// Split `runs[run_idx]` at intra-run byte `byte`. The original run becomes
/// `text[0..byte]`; a new run with the same style is inserted at `run_idx + 1`
/// holding `text[byte..]`. No-op when `byte == 0`, `byte >= text.len()`, or
/// the byte sits in the middle of a multi-byte UTF-8 codepoint (in the latter
/// case the byte is rounded DOWN to the nearest char boundary first; only if
/// that lands at 0 or len do we no-op).
fn split_run_at(runs: &mut Vec<super::ast::InlineRun>, run_idx: usize, byte: usize) {
    let Some(r) = runs.get(run_idx) else {
        return;
    };
    let len = r.text.len();
    if byte == 0 || byte >= len {
        return;
    }
    let b = round_down_char_boundary(&r.text, byte);
    if b == 0 || b >= len {
        return;
    }
    let style = r.style.clone();
    let (left, right) = r.text.split_at(b);
    let right_text = right.to_string();
    let left_text = left.to_string();
    runs[run_idx].text = left_text;
    runs.insert(
        run_idx + 1,
        super::ast::InlineRun { text: right_text, style },
    );
}

/// Apply `mutator` to every run in `block` whose body falls inside
/// `[from_byte (in run from_run), to_byte (in run to_run))`. Splits runs at
/// the from/to byte offsets first so the affected region becomes its own
/// run(s). Empty range collapses to a no-op. Out-of-range block, non-text
/// block, or empty `runs` → no-op.
fn apply_style_to_range_in_block(
    doc: &mut Document,
    block: usize,
    from_run: usize,
    from_byte: usize,
    to_run: usize,
    to_byte: usize,
    mutator: &mut dyn FnMut(&mut super::ast::InlineStyle),
) {
    let Some(b) = doc.blocks.get_mut(block) else {
        return;
    };
    let runs = match b {
        Block::Paragraph { runs, .. } | Block::Heading { runs, .. } => runs,
        Block::Table { .. } | Block::Image { .. } => return,
    };
    if runs.is_empty() {
        return;
    }
    let from_run = from_run.min(runs.len().saturating_sub(1));
    let to_run = to_run.min(runs.len().saturating_sub(1));
    let from_byte = from_byte.min(runs[from_run].text.len());
    let to_byte = to_byte.min(runs[to_run].text.len());
    if (from_run, from_byte) >= (to_run, to_byte) {
        return;
    }
    // Split high end FIRST so low indices stay valid. After this:
    //   runs[to_run] = original to_run text[0..to_byte] (will be styled)
    //   runs[to_run + 1] = original text[to_byte..] (untouched), if split happened
    if to_byte > 0 && to_byte < runs[to_run].text.len() {
        split_run_at(runs, to_run, to_byte);
    }
    // Now split low end. If split happens, runs[from_run] = original text[0..from_byte]
    // (untouched); runs[from_run + 1] = text[from_byte..] (start of styled range).
    // Adjust new_lo and new_hi to account for the inserted run.
    let (new_lo, new_hi) = if from_byte > 0 && from_byte < runs[from_run].text.len() {
        split_run_at(runs, from_run, from_byte);
        let lo = from_run + 1;
        let hi = if to_run >= from_run { to_run + 1 } else { to_run };
        (lo, hi)
    } else {
        (from_run, to_run)
    };
    // Apply mutator to runs[new_lo..=new_hi].
    let hi = new_hi.min(runs.len().saturating_sub(1));
    let lo = new_lo.min(hi);
    for r in runs[lo..=hi].iter_mut() {
        mutator(&mut r.style);
    }
}

/// Apply a style mutator to a sub-range that may span multiple blocks.
/// Within `from_block`: applies from (from_run, from_byte) to end-of-block.
/// Middle blocks (`from_block + 1 ..= to_block - 1`): full coverage.
/// Within `to_block`: applies from (run 0, byte 0) to (to_run, to_byte).
/// Single-block range (from_block == to_block) collapses to the in-block
/// helper directly. Non-text blocks in the span are skipped silently.
fn apply_range_style(
    doc: &mut Document,
    from: (usize, usize, usize),
    to: (usize, usize, usize),
    mut mutator: impl FnMut(&mut super::ast::InlineStyle),
) {
    let (from_block, from_run, from_byte) = from;
    let (to_block, to_run, to_byte) = to;
    if from_block > to_block {
        return;
    }
    if from_block == to_block {
        apply_style_to_range_in_block(
            doc,
            from_block,
            from_run,
            from_byte,
            to_run,
            to_byte,
            &mut mutator,
        );
        return;
    }
    // 1) from_block: (from_run, from_byte) → end of block.
    let from_end = {
        let Some(b) = doc.blocks.get(from_block) else {
            return;
        };
        match b {
            Block::Paragraph { runs, .. } | Block::Heading { runs, .. } => {
                let last_idx = runs.len().saturating_sub(1);
                let last_len = runs.get(last_idx).map(|r| r.text.len()).unwrap_or(0);
                Some((last_idx, last_len))
            }
            _ => None,
        }
    };
    if let Some((end_run, end_byte)) = from_end {
        apply_style_to_range_in_block(
            doc,
            from_block,
            from_run,
            from_byte,
            end_run,
            end_byte,
            &mut mutator,
        );
    }
    // 2) Middle blocks: full coverage. Splits aren't needed — every run is in range.
    for mid_block in (from_block + 1)..to_block {
        let Some(b) = doc.blocks.get_mut(mid_block) else {
            continue;
        };
        let runs = match b {
            Block::Paragraph { runs, .. } | Block::Heading { runs, .. } => runs,
            _ => continue,
        };
        for r in runs.iter_mut() {
            mutator(&mut r.style);
        }
    }
    // 3) to_block: (run 0, byte 0) → (to_run, to_byte).
    apply_style_to_range_in_block(doc, to_block, 0, 0, to_run, to_byte, &mut mutator);
}

fn round_down_char_boundary(s: &str, byte: usize) -> usize {
    let mut b = byte.min(s.len());
    while b > 0 && !s.is_char_boundary(b) {
        b -= 1;
    }
    b
}

fn mutate_props(doc: &mut Document, block: usize, mut f: impl FnMut(&mut BlockProps)) {
    let Some(b) = doc.blocks.get_mut(block) else {
        return;
    };
    match b {
        Block::Paragraph { props, .. } | Block::Heading { props, .. } => f(props),
        Block::Table { .. } | Block::Image { .. } => {}
    }
}

fn mutate_runs(doc: &mut Document, block: usize, mut f: impl FnMut(&mut super::ast::InlineStyle)) {
    let Some(b) = doc.blocks.get_mut(block) else {
        return;
    };
    let runs = match b {
        Block::Paragraph { runs, .. } | Block::Heading { runs, .. } => runs,
        // Phase 5a: tables and images at the top level aren't editable via
        // whole-block toolbar toggles. Sub-cell editing lands in a later phase.
        Block::Table { .. } | Block::Image { .. } => return,
    };
    for r in runs.iter_mut() {
        f(&mut r.style);
    }
}

#[cfg(test)]
mod tests {
    use super::super::ast::{Block, Document, InlineRun, InlineStyle};
    use super::*;

    fn doc_with(blocks: Vec<Block>) -> Document {
        Document { blocks }
    }

    fn para(text: &str) -> Block {
        Block::Paragraph {
            runs: vec![InlineRun::plain(text)],
            props: super::super::ast::BlockProps::default(),
        }
    }

    #[test]
    fn set_block_bold_flips_every_run_in_target_block_only() {
        let mut d = doc_with(vec![para("a"), para("b"), para("c")]);
        apply(&Edit::SetBlockBold { block: 1, on: true }, &mut d);
        let bold_of = |idx: usize| match &d.blocks[idx] {
            Block::Paragraph { runs, .. } => runs[0].style.bold,
            _ => panic!(),
        };
        assert!(!bold_of(0));
        assert!(bold_of(1));
        assert!(!bold_of(2));
    }

    #[test]
    fn set_block_bold_off_clears_existing_bold() {
        let mut d = doc_with(vec![Block::Paragraph {
            runs: vec![InlineRun {
                text: "x".into(),
                style: InlineStyle {
                    bold: true,
                    ..Default::default()
                },
            }],
            props: super::super::ast::BlockProps::default(),
        }]);
        apply(&Edit::SetBlockBold { block: 0, on: false }, &mut d);
        match &d.blocks[0] {
            Block::Paragraph { runs, .. } => assert!(!runs[0].style.bold),
            _ => panic!(),
        }
    }

    #[test]
    fn out_of_range_block_is_noop() {
        let mut d = doc_with(vec![para("a")]);
        apply(&Edit::SetBlockBold { block: 99, on: true }, &mut d);
        match &d.blocks[0] {
            Block::Paragraph { runs, .. } => assert!(!runs[0].style.bold),
            _ => panic!(),
        }
    }

    #[test]
    fn italic_and_underline_apply_independently() {
        let mut d = doc_with(vec![para("a")]);
        apply(&Edit::SetBlockItalic { block: 0, on: true }, &mut d);
        apply(&Edit::SetBlockUnderline { block: 0, on: true }, &mut d);
        match &d.blocks[0] {
            Block::Paragraph { runs, .. } => {
                assert!(runs[0].style.italic);
                assert!(runs[0].style.underline);
                assert!(!runs[0].style.bold);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn toggle_list_turns_plain_paragraph_into_bullet_then_clears() {
        use super::super::ast::ListMarker;
        let mut d = doc_with(vec![para("a")]);
        apply(&Edit::ToggleBlockList { block: 0, ordered: false }, &mut d);
        let m = match &d.blocks[0] {
            Block::Paragraph { props, .. } => props.list,
            _ => panic!(),
        };
        assert_eq!(m, Some(ListMarker { ordered: false, level: 0 }));
        // Second toggle of same kind clears.
        apply(&Edit::ToggleBlockList { block: 0, ordered: false }, &mut d);
        let m2 = match &d.blocks[0] {
            Block::Paragraph { props, .. } => props.list,
            _ => panic!(),
        };
        assert_eq!(m2, None);
    }

    #[test]
    fn toggle_list_switches_between_bullet_and_numbered() {
        use super::super::ast::ListMarker;
        let mut d = doc_with(vec![para("a")]);
        apply(&Edit::ToggleBlockList { block: 0, ordered: false }, &mut d);
        apply(&Edit::ToggleBlockList { block: 0, ordered: true }, &mut d);
        let m = match &d.blocks[0] {
            Block::Paragraph { props, .. } => props.list,
            _ => panic!(),
        };
        assert_eq!(m, Some(ListMarker { ordered: true, level: 0 }));
    }

    fn run(text: &str) -> InlineRun {
        InlineRun { text: text.into(), style: InlineStyle::default() }
    }

    fn block_with_runs(runs: Vec<InlineRun>) -> Block {
        Block::Paragraph { runs, props: super::super::ast::BlockProps::default() }
    }

    fn first_block_text(d: &Document) -> String {
        match &d.blocks[0] {
            Block::Paragraph { runs, .. } => runs.iter().map(|r| r.text.as_str()).collect(),
            _ => panic!(),
        }
    }

    #[test]
    fn insert_text_appends_into_target_run() {
        let mut d = doc_with(vec![block_with_runs(vec![run("hel")])]);
        apply(
            &Edit::InsertText { block: 0, run: 0, byte: 3, text: "lo".into() },
            &mut d,
        );
        assert_eq!(first_block_text(&d), "hello");
    }

    #[test]
    fn insert_text_into_empty_block_creates_run() {
        let mut d = doc_with(vec![block_with_runs(vec![])]);
        apply(
            &Edit::InsertText { block: 0, run: 0, byte: 0, text: "x".into() },
            &mut d,
        );
        assert_eq!(first_block_text(&d), "x");
    }

    #[test]
    fn insert_text_with_out_of_range_run_appends_to_last() {
        let mut d = doc_with(vec![block_with_runs(vec![run("a"), run("b")])]);
        apply(
            &Edit::InsertText { block: 0, run: 99, byte: 0, text: "Z".into() },
            &mut d,
        );
        assert_eq!(first_block_text(&d), "abZ");
    }

    #[test]
    fn delete_range_single_run_removes_middle() {
        let mut d = doc_with(vec![block_with_runs(vec![run("hello")])]);
        apply(
            &Edit::DeleteRange { block: 0, run_lo: 0, byte_lo: 1, run_hi: 0, byte_hi: 4 },
            &mut d,
        );
        assert_eq!(first_block_text(&d), "ho");
    }

    #[test]
    fn delete_range_across_runs_splices_and_keeps_empty_runs() {
        let mut d = doc_with(vec![block_with_runs(vec![run("foo"), run("bar"), run("baz")])]);
        // Delete from middle of run 0 to middle of run 2 — trims fo|o, clears bar, removes ba|z prefix
        apply(
            &Edit::DeleteRange { block: 0, run_lo: 0, byte_lo: 2, run_hi: 2, byte_hi: 2 },
            &mut d,
        );
        // Expected runs: "fo", "", "z" — concatenated "foz"
        let runs = match &d.blocks[0] {
            Block::Paragraph { runs, .. } => runs,
            _ => panic!(),
        };
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].text, "fo");
        assert_eq!(runs[1].text, "");
        assert_eq!(runs[2].text, "z");
    }

    #[test]
    fn insert_text_clamps_to_char_boundary() {
        // "héllo" — é is 2 bytes (0xC3 0xA9). Insert at byte 2 (middle of é) →
        // rounds down to byte 1 (before é).
        let mut d = doc_with(vec![block_with_runs(vec![run("héllo")])]);
        apply(
            &Edit::InsertText { block: 0, run: 0, byte: 2, text: "X".into() },
            &mut d,
        );
        // After rounding down to byte 1, "h" + "X" + "éllo" = "hXéllo"
        assert_eq!(first_block_text(&d), "hXéllo");
    }

    #[test]
    fn heading_runs_are_also_mutated() {
        let mut d = doc_with(vec![Block::Heading {
            level: 1,
            runs: vec![InlineRun::plain("title")],
            props: super::super::ast::BlockProps::default(),
        }]);
        apply(&Edit::SetBlockBold { block: 0, on: true }, &mut d);
        match &d.blocks[0] {
            Block::Heading { runs, .. } => assert!(runs[0].style.bold),
            _ => panic!(),
        }
    }

    fn flat_text(d: &Document) -> Vec<String> {
        d.blocks
            .iter()
            .map(|b| match b {
                Block::Paragraph { runs, .. } | Block::Heading { runs, .. } => {
                    runs.iter().map(|r| r.text.as_str()).collect()
                }
                _ => String::new(),
            })
            .collect()
    }

    #[test]
    fn split_block_mid_paragraph_creates_two_blocks() {
        let mut d = doc_with(vec![para("abcdef")]);
        apply(&Edit::SplitBlock { block: 0, run: 0, byte: 3 }, &mut d);
        assert_eq!(flat_text(&d), vec!["abc".to_string(), "def".to_string()]);
        assert_eq!(d.blocks.len(), 2);
    }

    #[test]
    fn split_block_at_end_creates_empty_paragraph() {
        let mut d = doc_with(vec![para("hello")]);
        apply(&Edit::SplitBlock { block: 0, run: 0, byte: 5 }, &mut d);
        assert_eq!(flat_text(&d), vec!["hello".to_string(), String::new()]);
    }

    #[test]
    fn split_block_at_start_pushes_content_down() {
        let mut d = doc_with(vec![para("hello")]);
        apply(&Edit::SplitBlock { block: 0, run: 0, byte: 0 }, &mut d);
        assert_eq!(flat_text(&d), vec![String::new(), "hello".to_string()]);
    }

    #[test]
    fn merge_with_next_concatenates_runs_and_drops_block() {
        let mut d = doc_with(vec![para("hello "), para("world")]);
        apply(&Edit::MergeWithNext { block: 0 }, &mut d);
        assert_eq!(d.blocks.len(), 1);
        match &d.blocks[0] {
            Block::Paragraph { runs, .. } => {
                let combined: String = runs.iter().map(|r| r.text.as_str()).collect();
                assert_eq!(combined, "hello world");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn merge_with_next_noop_when_next_is_table() {
        let mut d = doc_with(vec![
            para("alpha"),
            Block::Table { rows: vec![] },
        ]);
        apply(&Edit::MergeWithNext { block: 0 }, &mut d);
        assert_eq!(d.blocks.len(), 2);
    }

    #[test]
    fn delete_across_blocks_collapses_middle_blocks() {
        let mut d = doc_with(vec![para("abc"), para("middle"), para("def")]);
        apply(
            &Edit::DeleteAcrossBlocks {
                from_block: 0,
                from_run: 0,
                from_byte: 1,
                to_block: 2,
                to_run: 0,
                to_byte: 1,
            },
            &mut d,
        );
        assert_eq!(d.blocks.len(), 1);
        match &d.blocks[0] {
            Block::Paragraph { runs, .. } => {
                let combined: String = runs.iter().map(|r| r.text.as_str()).collect();
                assert_eq!(combined, "aef");
            }
            _ => panic!(),
        }
    }

    fn styled_run(text: &str, style: InlineStyle) -> InlineRun {
        InlineRun { text: text.into(), style }
    }

    fn run_bolds(d: &Document, block: usize) -> Vec<(String, bool)> {
        match &d.blocks[block] {
            Block::Paragraph { runs, .. } | Block::Heading { runs, .. } => {
                runs.iter().map(|r| (r.text.clone(), r.style.bold)).collect()
            }
            _ => vec![],
        }
    }

    #[test]
    fn set_range_bold_splits_single_run_at_both_ends() {
        // "hello world" — bold bytes [6, 11) ("world")
        let mut d = doc_with(vec![block_with_runs(vec![run("hello world")])]);
        apply(
            &Edit::SetRangeBold {
                from_block: 0,
                from_run: 0,
                from_byte: 6,
                to_block: 0,
                to_run: 0,
                to_byte: 11,
                on: true,
            },
            &mut d,
        );
        // Expect two runs: "hello " (not bold), "world" (bold)
        assert_eq!(
            run_bolds(&d, 0),
            vec![("hello ".into(), false), ("world".into(), true)]
        );
    }

    #[test]
    fn set_range_bold_splits_only_low_end_when_high_is_run_end() {
        let mut d = doc_with(vec![block_with_runs(vec![run("hello world")])]);
        apply(
            &Edit::SetRangeBold {
                from_block: 0,
                from_run: 0,
                from_byte: 6,
                to_block: 0,
                to_run: 0,
                to_byte: 11,
                on: true,
            },
            &mut d,
        );
        // Then bold from byte 0..5 inside the now-first run ("hello ").
        apply(
            &Edit::SetRangeBold {
                from_block: 0,
                from_run: 0,
                from_byte: 0,
                to_block: 0,
                to_run: 0,
                to_byte: 5,
                on: true,
            },
            &mut d,
        );
        assert_eq!(
            run_bolds(&d, 0),
            vec![
                ("hello".into(), true),
                (" ".into(), false),
                ("world".into(), true)
            ]
        );
    }

    #[test]
    fn set_range_bold_off_turns_off_only_selected_run() {
        let mut d = doc_with(vec![block_with_runs(vec![
            styled_run("foo", InlineStyle { bold: true, ..Default::default() }),
            styled_run("bar", InlineStyle { bold: true, ..Default::default() }),
        ])]);
        // Turn off bold on bytes [2, 4) — splits "foo" at 2 and "bar" at 1.
        // After: "fo"(B), "o"(NOT), "b"(NOT), "ar"(B)
        apply(
            &Edit::SetRangeBold {
                from_block: 0,
                from_run: 0,
                from_byte: 2,
                to_block: 0,
                to_run: 1,
                to_byte: 1,
                on: false,
            },
            &mut d,
        );
        assert_eq!(
            run_bolds(&d, 0),
            vec![
                ("fo".into(), true),
                ("o".into(), false),
                ("b".into(), false),
                ("ar".into(), true)
            ]
        );
    }

    #[test]
    fn set_range_bold_cross_block_styles_each_block_section() {
        let mut d = doc_with(vec![para("abc"), para("def"), para("ghi")]);
        apply(
            &Edit::SetRangeBold {
                from_block: 0,
                from_run: 0,
                from_byte: 1,
                to_block: 2,
                to_run: 0,
                to_byte: 2,
                on: true,
            },
            &mut d,
        );
        // Block 0 ("abc"): split at 1 → "a" (NOT) + "bc" (B)
        assert_eq!(run_bolds(&d, 0), vec![("a".into(), false), ("bc".into(), true)]);
        // Block 1 ("def"): full coverage → all bold
        assert_eq!(run_bolds(&d, 1), vec![("def".into(), true)]);
        // Block 2 ("ghi"): split at 2 → "gh" (B) + "i" (NOT)
        assert_eq!(run_bolds(&d, 2), vec![("gh".into(), true), ("i".into(), false)]);
    }

    #[test]
    fn set_range_italic_independent_of_bold() {
        let mut d = doc_with(vec![block_with_runs(vec![run("hello")])]);
        apply(
            &Edit::SetRangeItalic {
                from_block: 0,
                from_run: 0,
                from_byte: 0,
                to_block: 0,
                to_run: 0,
                to_byte: 5,
                on: true,
            },
            &mut d,
        );
        match &d.blocks[0] {
            Block::Paragraph { runs, .. } => {
                assert!(runs[0].style.italic);
                assert!(!runs[0].style.bold);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn split_run_at_no_op_on_zero_byte() {
        let mut runs = vec![run("hello")];
        split_run_at(&mut runs, 0, 0);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "hello");
    }

    #[test]
    fn split_run_at_no_op_on_end_byte() {
        let mut runs = vec![run("hello")];
        split_run_at(&mut runs, 0, 5);
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn split_run_at_mid_run_creates_two() {
        let mut runs = vec![run("hello")];
        split_run_at(&mut runs, 0, 2);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].text, "he");
        assert_eq!(runs[1].text, "llo");
    }

    #[test]
    fn delete_across_blocks_two_adjacent_blocks() {
        let mut d = doc_with(vec![para("abcde"), para("12345")]);
        apply(
            &Edit::DeleteAcrossBlocks {
                from_block: 0,
                from_run: 0,
                from_byte: 2,
                to_block: 1,
                to_run: 0,
                to_byte: 2,
            },
            &mut d,
        );
        assert_eq!(d.blocks.len(), 1);
        match &d.blocks[0] {
            Block::Paragraph { runs, .. } => {
                let combined: String = runs.iter().map(|r| r.text.as_str()).collect();
                assert_eq!(combined, "ab345");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn insert_blocks_single_paragraph_merges_into_host() {
        // Caret in the middle of "hello world" — pasting one paragraph
        // "PASTE" should yield "helloPASTE world" in a single paragraph.
        let mut d = doc_with(vec![para("hello world")]);
        apply(
            &Edit::InsertBlocks {
                at_block: 0,
                at_run: 0,
                at_byte: 5,
                blocks: vec![para("PASTE")],
            },
            &mut d,
        );
        assert_eq!(d.blocks.len(), 1);
        let combined: String = match &d.blocks[0] {
            Block::Paragraph { runs, .. } => runs.iter().map(|r| r.text.as_str()).collect(),
            _ => panic!(),
        };
        assert_eq!(combined, "helloPASTE world");
    }

    #[test]
    fn insert_blocks_multiple_paragraphs_inserts_middle_blocks() {
        // Caret in the middle of "abcdef" — pasting 3 paragraphs splits the
        // host: prefix "abc" + first merged → "abcFIRST", middle "MIDDLE",
        // last merged with suffix → "LASTdef".
        let mut d = doc_with(vec![para("abcdef")]);
        apply(
            &Edit::InsertBlocks {
                at_block: 0,
                at_run: 0,
                at_byte: 3,
                blocks: vec![para("FIRST"), para("MIDDLE"), para("LAST")],
            },
            &mut d,
        );
        assert_eq!(d.blocks.len(), 3);
        let texts: Vec<String> = d
            .blocks
            .iter()
            .map(|b| match b {
                Block::Paragraph { runs, .. } => runs.iter().map(|r| r.text.as_str()).collect(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(texts, vec!["abcFIRST", "MIDDLE", "LASTdef"]);
    }

    #[test]
    fn insert_blocks_at_doc_end_appends() {
        // Out-of-range at_block appends instead of crashing.
        let mut d = doc_with(vec![para("first")]);
        apply(
            &Edit::InsertBlocks {
                at_block: 99,
                at_run: 0,
                at_byte: 0,
                blocks: vec![para("second"), para("third")],
            },
            &mut d,
        );
        assert_eq!(d.blocks.len(), 3);
    }

    #[test]
    fn insert_blocks_empty_input_is_noop() {
        let mut d = doc_with(vec![para("alpha")]);
        let before = d.clone();
        apply(
            &Edit::InsertBlocks {
                at_block: 0,
                at_run: 0,
                at_byte: 2,
                blocks: vec![],
            },
            &mut d,
        );
        assert_eq!(d, before);
    }
}
