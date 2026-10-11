// PowerEditor/src/Notepad_plus.cpp:2979-3091: Cut, Paste, Remove, Inverse.
// Notepad_plus.rc:735-740; Parameters.cpp:306-311: order and unbound defaults.
// NppCommands.cpp:2385-2407,4440,4562-4566: dispatch and recorded commands.
use crate::{actions::*, buffer::{ContentRevision, Selection, prepared::{Metadata, PrefixEdit, PreparedGroup}},
    macros::MacroEvent, text::line_ending::LineEnding, workspace::Workspace};
use gpui::{AppContext, Context, Window};
use ropey::Rope;
use std::{collections::BTreeSet, sync::atomic::{AtomicBool, Ordering}};

const LIMIT: usize = 128 * 1024 * 1024;
const MAX_RANGES: usize = 262_144;
const MAX_CLIPBOARD_ENTRIES: usize = 4096;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BookmarkCommand { Cut, Paste, Remove, RemoveNon, Inverse }

impl BookmarkCommand {
    pub(crate) fn event(self) -> MacroEvent {
        match self {
            Self::Cut => MacroEvent::CutBookmarkedLines,
            Self::Paste => MacroEvent::PasteToBookmarkedLines,
            Self::Remove => MacroEvent::RemoveBookmarkedLines,
            Self::RemoveNon => MacroEvent::RemoveNonBookmarkedLines,
            Self::Inverse => MacroEvent::InverseBookmarks,
        }
    }
}

pub(crate) struct BookmarkResult {
    pub(crate) group: PreparedGroup,
    pub(crate) metadata: Metadata,
    pub(crate) clipboard: Option<String>,
}

pub(crate) struct BookmarkInput {
    pub(crate) ending: LineEnding,
    pub(crate) paste: Option<String>,
}

pub(crate) fn prepare_paste(entries: impl IntoIterator<Item = gpui::ClipboardEntry>, cancel: &AtomicBool) -> Result<String, &'static str> {
    let mut text: Option<String> = None;
    for (index, entry) in entries.into_iter().enumerate() {
        if index >= MAX_CLIPBOARD_ENTRIES { return Err("Bookmark paste supports at most 4,096 clipboard entries"); }
        if cancel.load(Ordering::Relaxed) { return Err("Bookmark paste preparation cancelled"); }
        let gpui::ClipboardEntry::String(entry) = entry else { continue; };
        let mut part = entry.into_text();
        let (mut offset, mut nul) = (0usize, None);
        for chunk in part.as_bytes().chunks(64 * 1024) {
            if cancel.load(Ordering::Relaxed) { return Err("Bookmark paste preparation cancelled"); }
            if let Some(index) = chunk.iter().position(|&byte| byte == 0) { nul = Some(offset.saturating_add(index)); break; }
            offset = offset.saturating_add(chunk.len());
        }
        if let Some(nul) = nul { part.truncate(nul); }
        if let Some(text) = text.as_mut() {
            if text.len().checked_add(part.len()).is_none_or(|length| length > LIMIT) {
                return Err("Bookmark paste exceeds the 128 MiB clipboard limit");
            }
            text.try_reserve(part.len()).map_err(|_| "Not enough memory to prepare bookmark paste")?;
            text.push_str(&part);
        } else {
            if part.len() > LIMIT { return Err("Bookmark paste exceeds the 128 MiB clipboard limit"); }
            text = Some(part);
        }
        if nul.is_some() { break; }
    }
    Ok(text.unwrap_or_default())
}

pub(crate) fn prepare_bookmarks(rope: Rope, revision: ContentRevision, selection: Selection,
    metadata: &Metadata, command: BookmarkCommand, input: BookmarkInput, cancel: &AtomicBool,
) -> Result<BookmarkResult, &'static str> {
    let BookmarkInput { ending, paste } = input;
    if paste.as_ref().is_some_and(|text| text.len() > LIMIT) { return Err("Bookmark paste exceeds the 128 MiB clipboard limit"); }
    let mut edits: Vec<PrefixEdit> = Vec::new();
    let mut inverse = BTreeSet::new();
    let mut removed_bookmarks = BTreeSet::new();
    let mut bytes = rope.bytes().peekable();
    let (mut start, mut end, mut body_end, mut planned) = (0usize, 0usize, 0usize, 0usize);
    loop {
        if cancel.load(Ordering::Relaxed) { return Err("Bookmark edit cancelled"); }
        while let Some(byte) = bytes.next() {
            end = end.saturating_add(1);
            if byte == b'\r' {
                body_end = end.saturating_sub(1);
                if bytes.peek() == Some(&b'\n') { bytes.next(); end = end.saturating_add(1); }
                break;
            }
            if byte == b'\n' { body_end = end.saturating_sub(1); break; }
            body_end = end;
            if end.is_multiple_of(8192) && cancel.load(Ordering::Relaxed) { return Err("Bookmark edit cancelled"); }
        }
        let marked = if start == end { metadata.bookmarks.contains(&start) }
            else { metadata.bookmarks.range(start..end).next().is_some() };
        if command == BookmarkCommand::Inverse {
            if !marked {
                if inverse.len().saturating_add(metadata.marks.len()) >= 65_536 { return Err("Inverse Bookmarks would exceed the 65,536 annotation limit"); }
                inverse.insert(start);
            }
        } else if marked != (command == BookmarkCommand::RemoveNon) {
            let range = if command == BookmarkCommand::Paste { start..body_end } else { start..end };
            if command != BookmarkCommand::Paste {
                if start == end { if marked { removed_bookmarks.insert(start); } }
                else { removed_bookmarks.extend(metadata.bookmarks.range(start..end).copied()); }
            }
            let text = if command == BookmarkCommand::Paste { paste.as_deref().unwrap_or_default() } else { "" };
            planned = planned.checked_add(range.len()).and_then(|n| n.checked_add(text.len()))
                .filter(|&n| n <= LIMIT).ok_or("Bookmark edits exceed the 128 MiB history limit")?;
            if range.start != range.end || !text.is_empty() {
                // Consecutive removals are one target. This keeps Remove Non
                // linear even when nearly every line in a large file is removed.
                if command != BookmarkCommand::Paste && edits.last().is_some_and(|edit| edit.range.end == range.start) {
                    if let Some(edit) = edits.last_mut() { edit.range.end = range.end; }
                } else {
                    if edits.len() >= MAX_RANGES { return Err("Bookmark edits support at most 262,144 separate targets"); }
                    edits.try_reserve(1).map_err(|_| "Not enough memory to prepare bookmark edits")?;
                    let mut inserted = String::new();
                    inserted.try_reserve_exact(text.len()).map_err(|_| "Not enough memory to prepare bookmark edits")?;
                    inserted.push_str(text);
                    edits.push(PrefixEdit { range, text: inserted });
                }
            }
        }
        if end == rope.len_bytes() {
            if start != end && body_end != end { start = end; body_end = end; continue; }
            break;
        }
        start = end; body_end = end;
    }
    let clipboard = if command == BookmarkCommand::Cut {
        let mut bookmarks = Vec::new();
        bookmarks.try_reserve_exact(metadata.bookmarks.len()).map_err(|_| "Not enough memory to prepare bookmark cut")?;
        bookmarks.extend(metadata.bookmarks.iter().copied());
        Some(super::bookmark_clipboard::bookmarked_text(&rope, &bookmarks, ending, LIMIT, cancel)?)
    } else { None };
    let mut group = PreparedGroup::build_line_edits(rope, revision, selection, edits, command != BookmarkCommand::Paste, cancel)?;
    group.follow_inserted_lines();
    let mut source = metadata.clone();
    for bookmark in removed_bookmarks { source.bookmarks.remove(&bookmark); }
    let mut mapped = if group.ops().is_empty() { source } else { group.remap(&source, false, cancel)? };
    if command == BookmarkCommand::Inverse { mapped.bookmarks = inverse; }
    Ok(BookmarkResult { group, metadata: mapped, clipboard })
}

impl Workspace {
    pub(crate) fn run_bookmark_command(&mut self, command: BookmarkCommand, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.cancel_bookmark_copy(cx);
        if !self.bookmark_source_available(command, window, cx) { return false; }
        let paste = if command == BookmarkCommand::Paste {
            let Some(item) = cx.read_from_clipboard() else { crate::macros::record(command.event()); return true; };
            if item.entries().len() > MAX_CLIPBOARD_ENTRIES {
                cx.background_spawn(async move { drop(item); }).detach();
                self.set_status("Bookmark paste supports at most 4,096 clipboard entries"); cx.notify(); return false;
            }
            if !item.entries().iter().any(|entry| matches!(entry, gpui::ClipboardEntry::String(_))) {
                cx.background_spawn(async move { drop(item); }).detach();
                crate::macros::record(command.event()); return true;
            }
            Some(item)
        } else { None };
        self.start_prepared_bookmarks(command, paste, window, cx)
    }
    pub(crate) fn cut_bookmarked_lines(&mut self, _: &CutBookmarkedLines, window: &mut Window, cx: &mut Context<Self>) { self.run_bookmark_command(BookmarkCommand::Cut, window, cx); }
    pub(crate) fn paste_to_bookmarked_lines(&mut self, _: &PasteToBookmarkedLines, window: &mut Window, cx: &mut Context<Self>) { self.run_bookmark_command(BookmarkCommand::Paste, window, cx); }
    pub(crate) fn remove_bookmarked_lines(&mut self, _: &RemoveBookmarkedLines, window: &mut Window, cx: &mut Context<Self>) { self.run_bookmark_command(BookmarkCommand::Remove, window, cx); }
    pub(crate) fn remove_non_bookmarked_lines(&mut self, _: &RemoveNonBookmarkedLines, window: &mut Window, cx: &mut Context<Self>) { self.run_bookmark_command(BookmarkCommand::RemoveNon, window, cx); }
    pub(crate) fn inverse_bookmarks(&mut self, _: &InverseBookmarks, window: &mut Window, cx: &mut Context<Self>) { self.run_bookmark_command(BookmarkCommand::Inverse, window, cx); }
}
