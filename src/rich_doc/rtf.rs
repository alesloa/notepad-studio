//! RTF parser + writer. Phase 3: read + whole-block bold/italic/underline,
//! plus editable raw view (RTF is plain text).
//!
//! Mapping strategy: scrivener-rtf gives us a low-level group/control-word tree.
//! We flatten the body group into a sequence of paragraphs by tracking a running
//! character-format state, splitting on `\par`, and emitting an `InlineRun` per
//! contiguous text span with the same style. Headings are detected via the
//! `\outlinelevel N` paragraph control (most word processors emit this for
//! styled headings).
//!
//! Save modes (decided by the caller in `mod.rs::save`):
//! - empty edit_log + no rope edits → write `original_bytes` byte-for-byte
//! - empty edit_log + rope edits     → write rope text as the new file
//!   (raw-view editing path)
//! - non-empty edit_log              → regenerate from AST via scrivener-rtf's
//!   `Document::to_rtf` and write that

use super::{ast, DocFormat, RichDocError, RichDocPayload, Result};
use scrivener_rtf as srtf;
use std::path::Path;

pub fn open(path: &Path) -> Result<RichDocPayload> {
    let original_bytes = std::fs::read(path)?;
    // RTF is 7-bit ASCII for control tokens with code-page-decoded text spans
    // already resolved into UTF-8 by scrivener-rtf's hex-escape pass, so the
    // raw view shows the source text exactly as on disk (modulo any non-UTF-8
    // bytes, which we lossy-decode for display only — original_bytes preserves
    // the true bytes for byte-perfect save).
    let raw_view_text = String::from_utf8_lossy(&original_bytes).into_owned();
    let ast = parse_bytes(&original_bytes)?;
    Ok(RichDocPayload {
        format: DocFormat::Rtf,
        ast,
        raw_view_text,
        original_bytes,
        edit_log: Vec::new(),
        active_block: None,
        selection: None,
        drag_active: false,
        pending_drag_anchor: None,
        undo_stack: Vec::new(),
        redo_stack: Vec::new(),
        headers: Vec::new(),
        footers: Vec::new(),
    })
}

/// Save dispatch — see module docs for the three branches.
pub fn save(payload: &RichDocPayload, path: &Path, raw_view_override: Option<&str>) -> Result<()> {
    if !payload.edit_log.is_empty() {
        super::ensure_text_only(&payload.ast)?;
        let new_rtf = serialize_ast(&payload.ast)?;
        super::write_document_bytes(path, new_rtf.as_bytes())?;
        return Ok(());
    }
    if let Some(raw) = raw_view_override {
        super::write_document_bytes(path, raw.as_bytes())?;
        return Ok(());
    }
    super::write_document_bytes(path, &payload.original_bytes)?;
    Ok(())
}

fn parse_bytes(bytes: &[u8]) -> Result<ast::Document> {
    check_lexable(bytes)?;
    let doc = srtf::parse(bytes).map_err(|e| RichDocError::Rtf(format!("{e:?}")))?;
    Ok(flatten_to_ast(&doc))
}

/// Refuse input that scrivener-rtf 0.1 would panic on instead of returning
/// an error. Its lexer (`lexer.rs` `lex_control_word`) reads a control word
/// as `\` + ASCII letters + optional `-` + digits and `expect`s the digits to
/// parse as an i32, so `\b99999999999` aborts the app. This scan tokenizes
/// control words the same way and reports the bad parameter instead. It also
/// counts group nesting: the crate's parser and `walk_group` recurse once
/// per `{`, so nesting past `MAX_NESTING_DEPTH` is refused.
fn check_lexable(bytes: &[u8]) -> Result<()> {
    let mut i = 0usize;
    let mut depth = 0usize;
    while let Some(&b) = bytes.get(i) {
        if b == b'{' {
            depth = depth.saturating_add(1);
            if depth > super::MAX_NESTING_DEPTH {
                return Err(RichDocError::Rtf(format!(
                    "groups nest more than {} deep at byte {i}",
                    super::MAX_NESTING_DEPTH
                )));
            }
        } else if b == b'}' {
            depth = depth.saturating_sub(1);
        }
        if b != b'\\' {
            i += 1;
            continue;
        }
        let name_start = i + 1;
        let mut j = name_start;
        while bytes.get(j).is_some_and(u8::is_ascii_alphabetic) {
            j += 1;
        }
        if j == name_start {
            // Control symbol (`\{`, `\\`, `\'`, `\*`, …): one byte follows.
            i = name_start.saturating_add(1);
            continue;
        }
        let mut k = j;
        if bytes.get(k) == Some(&b'-') {
            k += 1;
        }
        let digits_start = k;
        while bytes.get(k).is_some_and(u8::is_ascii_digit) {
            k += 1;
        }
        if k == digits_start {
            // No parameter; a lone `-` stays in the text stream.
            i = j;
            continue;
        }
        let param = bytes.get(j..k).unwrap_or_default();
        let fits = std::str::from_utf8(param)
            .ok()
            .and_then(|s| s.parse::<i32>().ok())
            .is_some();
        if !fits {
            let name = String::from_utf8_lossy(bytes.get(name_start..j).unwrap_or_default());
            return Err(RichDocError::Rtf(format!(
                "control word \\{name} at byte {i} has a parameter outside the 32-bit range"
            )));
        }
        i = k;
    }
    Ok(())
}

/// Walk the parsed RTF tree and produce a paragraph/run AST. The root group's
/// header (font table, color table, stylesheet, generator, info, etc.) is
/// skipped — only body content contributes text.
fn flatten_to_ast(doc: &srtf::Document) -> ast::Document {
    let mut state = ParseState::default();
    if let Some(root) = doc.groups.first() {
        walk_group(root, &mut state, /* in_root */ true);
    }
    state.finish();
    ast::Document { blocks: state.blocks }
}

#[derive(Default)]
struct ParseState {
    blocks: Vec<ast::Block>,
    cur_runs: Vec<ast::InlineRun>,
    /// Running character style (toggled by `\b`, `\i`, `\ul`, `\plain`, etc.)
    style: ast::InlineStyle,
    /// Outline level set by `\outlinelevel N` on the current paragraph; flushed
    /// on `\par`. RTF outline levels are 0-based (0 = top-level heading).
    cur_outline_level: Option<u8>,
    /// Paragraph-level alignment + indent built up from `\ql/\qc/\qr/\qj` and
    /// `\liN` (or `\linN`) controls. Flushed on `\par`, reset to defaults at
    /// the start of the next paragraph.
    cur_props: ast::BlockProps,
    /// Color table parsed from the document's `{\colortbl;…}` header group.
    /// Indexed by RTF `\cf<N>` parameter (`N=0` = auto / no color, mapped to
    /// `None`; `N >= 1` is `colortbl[N-1]`). Empty when the document had no
    /// color table — `\cf<N>` then degrades to a no-op.
    colortbl: Vec<u32>,
}

impl ParseState {
    /// Emit a text span using the current style and append it to the in-flight
    /// paragraph. Empty text is a no-op so trailing `\par` doesn't add stub runs.
    fn push_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.cur_runs.push(ast::InlineRun {
            text: text.to_string(),
            style: self.style.clone(),
        });
    }

    /// Close the current paragraph (called on `\par` and at end of document).
    /// Drops empty paragraphs entirely so a stray trailing `\par` doesn't add
    /// a blank block to the AST (the renderer would draw it as an empty row).
    fn end_paragraph(&mut self) {
        let runs = std::mem::take(&mut self.cur_runs);
        let level = self.cur_outline_level.take();
        let props = std::mem::take(&mut self.cur_props);
        if runs.is_empty() {
            return;
        }
        let block = match level {
            Some(n) if n < 6 => ast::Block::Heading {
                level: n + 1,
                runs,
                props,
            },
            _ => ast::Block::Paragraph { runs, props },
        };
        self.blocks.push(block);
    }

    fn finish(&mut self) {
        self.end_paragraph();
    }
}

/// Recursively walk a group. `in_root` distinguishes the top-level `{\rtf1 …}`
/// (whose header destination groups must be skipped) from inner content groups
/// (which may contain text we want).
fn walk_group(group: &srtf::Group, state: &mut ParseState, in_root: bool) {
    // Intercept named groups whose existence we care about BEFORE the
    // destination short-circuit. `\*\pn` is marked as a destination by the
    // `\*` prefix but its sub-control-words (`\pnlvlblt` / `\pnlvlbody`) are
    // exactly how old-style RTF declares list membership. `\pntext` is a
    // sibling group holding the manually-formatted marker that non-list-aware
    // readers display; we don't emit its text but its presence still means
    // the paragraph is list-bearing.
    if let Some(first) = first_control_word(group) {
        match first.as_str() {
            "pn" => {
                parse_pn_group(group, state);
                return;
            }
            "pntext" => {
                if state.cur_props.list.is_none() {
                    state.cur_props.list = Some(ast::ListMarker { ordered: false, level: 0 });
                }
                return;
            }
            _ => {}
        }
    }
    if group.is_destination {
        // `\*\generator`, `\*\listtable`, `\*\rsidtbl`, etc. — RTF readers
        // are required to skip destinations they don't understand. We
        // understand zero of them in Phase 3.
        return;
    }
    // `{\colortbl …}` is a header group: we extract the color table and
    // skip its body so the `\red`/`\green`/`\blue` triples don't leak
    // into the paragraph stream. Recognized by the first non-text element
    // being `\colortbl`.
    if first_control_word(group).as_deref() == Some("colortbl") {
        parse_colortbl_into(group, &mut state.colortbl);
        return;
    }
    // Style snapshot so a nested group's character formatting doesn't leak
    // into surrounding text (RTF's `{ \b bold } normal` scoping rule).
    // Outline level is paragraph-scoped (not group-scoped) so only style is saved.
    let saved_style = state.style.clone();
    for content in &group.content {
        match content {
            srtf::Content::Group(sub) => {
                walk_group(sub, state, false);
            }
            srtf::Content::Text(s) => {
                state.push_text(s);
            }
            srtf::Content::ControlWord(name, param) => {
                handle_control_word(name, *param, state, in_root);
            }
            srtf::Content::ControlSymbol(c) => {
                handle_control_symbol(*c, state);
            }
        }
    }
    state.style = saved_style;
}

/// Name of the first control word in `group`, if any. Used to recognize
/// header destinations by their leading token (`colortbl`, `fonttbl`,
/// `stylesheet`, etc.) without iterating into their body.
fn first_control_word(group: &srtf::Group) -> Option<String> {
    for c in &group.content {
        if let srtf::Content::ControlWord(name, _) = c {
            return Some(name.clone());
        }
    }
    None
}

/// Parse a `{\colortbl;\redN\greenN\blueN;…}` group into `out`. Pushes one
/// `u32` per `\red\green\blue` triple (in order). The leading empty entry
/// (the bare `;` after `\colortbl`) is RTF's "auto" slot — we don't store
/// it; `\cf0` is handled separately as "no color" in `handle_control_word`.
fn parse_colortbl_into(group: &srtf::Group, out: &mut Vec<u32>) {
    let mut r: Option<u8> = None;
    let mut g: Option<u8> = None;
    let mut b: Option<u8> = None;
    for content in &group.content {
        match content {
            srtf::Content::ControlWord(name, param) => match name.as_str() {
                "red" => r = Some(param.unwrap_or(0).clamp(0, 255) as u8),
                "green" => g = Some(param.unwrap_or(0).clamp(0, 255) as u8),
                "blue" => b = Some(param.unwrap_or(0).clamp(0, 255) as u8),
                _ => {}
            },
            srtf::Content::Text(s) => {
                // Every `;` closes a slot. If the slot is fully populated,
                // push it; otherwise it's the leading `;` (slot 0 = auto)
                // or a malformed entry — skip without pushing.
                for ch in s.chars() {
                    if ch == ';' {
                        if let (Some(rv), Some(gv), Some(bv)) = (r, g, b) {
                            let packed = ((rv as u32) << 16) | ((gv as u32) << 8) | (bv as u32);
                            out.push(packed);
                        }
                        r = None;
                        g = None;
                        b = None;
                    }
                }
            }
            _ => {}
        }
    }
}

/// Parse a `{\*\pn …}` paragraph-numbering destination. We only need to know
/// whether the paragraph is bulleted or numbered (and optionally the level)
/// — bullet glyph / numbering format / pn-text overrides are dropped because
/// our renderer regenerates markers from `ListMarker` on display.
/// - `\pnlvlblt` → bulleted list (level taken from `\pnlvlN` if present, else 0)
/// - `\pnlvlbody` → numbered list at top level
/// - `\pnlvlN` (N=1..9) without `\pnlvlblt` → numbered at level N-1
fn parse_pn_group(group: &srtf::Group, state: &mut ParseState) {
    let mut ordered: Option<bool> = None;
    let mut level: u8 = 0;
    for content in &group.content {
        if let srtf::Content::ControlWord(name, param) = content {
            match name.as_str() {
                "pnlvlblt" => ordered = Some(false),
                "pnlvlbody" => ordered = Some(true),
                "pnlvl" => {
                    if let Some(p) = param {
                        // \pnlvlN (N=1..9) — numbered nested level. The control
                        // word arrives as name=`pnlvl` param=N because the lexer
                        // splits the numeric suffix.
                        level = (p.saturating_sub(1).max(0) as u8).min(8);
                        if ordered.is_none() {
                            ordered = Some(true);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    // Bare `\pn` group with no recognized sub-marker defaults to bullet — most
    // legacy RTF list-style declarations omit `\pnlvlblt` and rely on `\pntext`.
    let ordered = ordered.unwrap_or(false);
    state.cur_props.list = Some(ast::ListMarker { ordered, level });
}

/// Handle a `\word` control. Most we don't care about; we toggle the inline
/// style flags relevant to Phase 3 and treat `\par` as a paragraph boundary.
/// Header destinations (`fonttbl`, `colortbl`, `stylesheet`, `info`) drop the
/// whole containing group when encountered at the root.
fn handle_control_word(name: &str, param: Option<i32>, state: &mut ParseState, in_root: bool) {
    // Style toggles use the RTF convention: bare control = on,
    // `\bN` where N=0 = off, any other N = on. `\plain` clears everything.
    let on = || param.map(|p| p != 0).unwrap_or(true);
    match name {
        "b" => state.style.bold = on(),
        "i" => state.style.italic = on(),
        "ul" => state.style.underline = on(),
        "ulnone" => state.style.underline = false,
        "strike" => state.style.strike = on(),
        "striked" => state.style.strike = on(),
        "plain" => state.style = ast::InlineStyle::default(),
        "par" => state.end_paragraph(),
        "line" => state.push_text("\n"),
        "tab" => state.push_text("\t"),
        "outlinelevel" => {
            // RTF outline level is 0-based; we map 0→Heading1, 1→Heading2, etc.
            if let Some(p) = param {
                if (0..=8).contains(&p) {
                    state.cur_outline_level = Some(p as u8);
                }
            }
        }
        // Paragraph alignment. RTF: \ql left, \qc center, \qr right, \qj justify.
        // \pard resets to defaults — set Left so a fresh paragraph after a
        // centered one paints normally.
        "ql" => state.cur_props.alignment = ast::Alignment::Left,
        "qc" => state.cur_props.alignment = ast::Alignment::Center,
        "qr" => state.cur_props.alignment = ast::Alignment::Right,
        "qj" => state.cur_props.alignment = ast::Alignment::Justify,
        "pard" => state.cur_props = ast::BlockProps::default(),
        // Left indent in twips. 720 twips = 0.5 inch = our "level 1". Negative
        // values are an RTF outdent — we clamp to 0 since Phase 5d doesn't
        // model outdents.
        "li" | "lin" => {
            if let Some(p) = param {
                let twips = p.max(0) as u32;
                state.cur_props.indent_level = (twips / 720).min(255) as u8;
            }
        }
        "cf" => {
            // \cf0 = "auto" / default; clear the color so the run reverts.
            // \cfN (N >= 1) indexes into the parsed color table (1-based).
            // Missing table entries silently no-op — better than painting a
            // wrong color from a stale state if the table parse failed.
            match param {
                Some(0) | None => state.style.color = None,
                Some(n) if n > 0 => {
                    let idx = (n as usize).saturating_sub(1);
                    if let Some(&c) = state.colortbl.get(idx) {
                        state.style.color = Some(c);
                    }
                }
                _ => {}
            }
        }
        // Everything else (font tables, paragraph metrics, view kind,
        // header markers) is irrelevant to Phase 3's body-only extraction.
        _ => {
            let _ = in_root;
        }
    }
}

fn handle_control_symbol(c: char, state: &mut ParseState) {
    match c {
        '~' => state.push_text("\u{00A0}"), // non-breaking space
        '-' => state.push_text("\u{00AD}"), // optional hyphen
        '_' => state.push_text("-"),        // non-breaking hyphen
        '\\' | '{' | '}' => state.push_text(&c.to_string()),
        _ => {}
    }
}

/// Re-serialize an AST `Document` back to RTF source. Uses scrivener-rtf's
/// generator on a freshly-built `srtf::Document` whose root group is:
///   `{ \rtf1\ansi {\colortbl;…;} <body> }`
/// with `<body>` = paragraph runs separated by `\par`. Color table is built
/// from the unique non-default colors used anywhere in the AST — entry 0 is
/// always the empty "auto" slot (`;` after `\colortbl`); each colored run
/// emits `\cf<idx>` referencing its slot. Linked runs are emitted as RTF
/// `{\field{\*\fldinst HYPERLINK "url"}{\fldrslt body}}` groups so Word /
/// LibreOffice render the body as a clickable link.
///
/// We still drop the original font table — Phase 5 doesn't model fonts, so
/// re-emitting a stale font table that nothing references would just bloat
/// the file. Word and LibreOffice accept RTF with only the implicit
/// default font.
fn serialize_ast(doc: &ast::Document) -> Result<String> {
    let mut content = Vec::new();
    content.push(srtf::Content::ControlWord("rtf".into(), Some(1)));
    content.push(srtf::Content::ControlWord("ansi".into(), None));

    let colors = collect_unique_colors(doc);
    if !colors.is_empty() {
        content.push(srtf::Content::Group(Box::new(build_colortbl_group(&colors))));
    }

    for (i, block) in doc.blocks.iter().enumerate() {
        serialize_block(block, &mut content, &colors);
        // Separate paragraphs with `\par`. No trailing `\par` after the last
        // block — RTF treats final-line content as a paragraph implicitly and
        // adding one would create a blank trailing paragraph on re-open.
        if i + 1 < doc.blocks.len() {
            content.push(srtf::Content::ControlWord("par".into(), None));
        }
    }
    let out_doc = srtf::Document {
        groups: vec![srtf::Group {
            content,
            is_destination: false,
        }],
        properties: srtf::DocumentProperties::default(),
    };
    Ok(out_doc.to_rtf())
}

/// Walk every block/run in `doc` and collect each distinct color in
/// first-appearance order. The returned vec is the color-table source: a
/// color's slot index is `vec.iter().position(...).map(|i| i + 1)` (the
/// `+ 1` accounts for slot 0 = default "auto").
fn collect_unique_colors(doc: &ast::Document) -> Vec<u32> {
    let mut seen: Vec<u32> = Vec::new();
    for block in &doc.blocks {
        let runs = match block {
            ast::Block::Paragraph { runs, .. } | ast::Block::Heading { runs, .. } => runs,
            _ => continue,
        };
        for run in runs {
            if let Some(c) = run.style.color {
                if !seen.contains(&c) {
                    seen.push(c);
                }
            }
        }
    }
    seen
}

/// `{\colortbl;\red…\green…\blue…;…}` group. RTF spec requires a leading
/// empty entry (the bare `;` after `\colortbl`) to denote "auto" / default.
/// We emit that as a literal `;` text token followed by `\red\green\blue;`
/// triples for each entry in `colors`.
fn build_colortbl_group(colors: &[u32]) -> srtf::Group {
    let mut content = Vec::with_capacity(1 + 4 * colors.len());
    content.push(srtf::Content::ControlWord("colortbl".into(), None));
    // Leading `;` is slot 0 = auto/default. The same `;` separator follows
    // every color triple, so we open with one and let the per-color loop
    // emit the closing one after each triple.
    content.push(srtf::Content::Text(";".into()));
    for c in colors {
        let r = ((c >> 16) & 0xFF) as i32;
        let g = ((c >> 8) & 0xFF) as i32;
        let b = (c & 0xFF) as i32;
        content.push(srtf::Content::ControlWord("red".into(), Some(r)));
        content.push(srtf::Content::ControlWord("green".into(), Some(g)));
        content.push(srtf::Content::ControlWord("blue".into(), Some(b)));
        content.push(srtf::Content::Text(";".into()));
    }
    srtf::Group {
        content,
        is_destination: false,
    }
}

fn serialize_block(block: &ast::Block, out: &mut Vec<srtf::Content>, colors: &[u32]) {
    let (runs, outline, props) = match block {
        ast::Block::Paragraph { runs, props } => (runs, None, props),
        ast::Block::Heading { level, runs, props } => {
            (runs, Some(level.saturating_sub(1)), props)
        }
        // Save rejects these blocks before calling the text serializer.
        ast::Block::Table { .. } | ast::Block::Image { .. } => return,
    };
    out.push(srtf::Content::ControlWord("pard".into(), None));
    if let Some(n) = outline {
        out.push(srtf::Content::ControlWord("outlinelevel".into(), Some(n as i32)));
    }
    // Alignment goes on every paragraph except left (left is the default
    // after \pard so emitting \ql would just bloat the file). Indent emits
    // only when non-zero; \li0 would also be a no-op but a safer signal to
    // editors that the paragraph has zero indent intentionally — still
    // skipped for file-size reasons.
    match props.alignment {
        ast::Alignment::Left => {}
        ast::Alignment::Center => out.push(srtf::Content::ControlWord("qc".into(), None)),
        ast::Alignment::Right => out.push(srtf::Content::ControlWord("qr".into(), None)),
        ast::Alignment::Justify => out.push(srtf::Content::ControlWord("qj".into(), None)),
    }
    // List markup goes BEFORE the indent control words — the `\pntext` group
    // is the manually-formatted marker non-list-aware readers display in line
    // with the paragraph, and the `\*\pn` destination is the actual numbering
    // definition list-aware readers use. The combined left indent is the
    // user's base indent (`indent_level * 720`) plus one level for the list
    // hang itself (so the marker sits in the gutter), with `\fi-360` pulling
    // the first-line marker back into that gutter.
    let list_extra_twips: i32 = if let Some(marker) = props.list {
        emit_list_markup(out, marker);
        ((marker.level as i32) + 1) * 720
    } else {
        0
    };
    let total_li = (props.indent_level as i32) * 720 + list_extra_twips;
    if props.list.is_some() {
        out.push(srtf::Content::ControlWord("fi".into(), Some(-360)));
        out.push(srtf::Content::ControlWord("li".into(), Some(total_li)));
    } else if total_li > 0 {
        out.push(srtf::Content::ControlWord("li".into(), Some(total_li)));
    }
    for run in runs {
        serialize_run(run, out, colors);
    }
}

/// Emit the `{\pntext …}{\*\pn …}` pair for a list paragraph. The leading
/// `\pntext` group is the inline-rendered marker for readers that don't
/// understand `\pn` (e.g. plain-text RTF viewers); the `\*\pn` destination
/// carries the structured list definition that Word / LibreOffice / TextEdit
/// honor. Bullet uses `\'b7` (Latin-1 middle dot); numbered emits a literal
/// "1." in `\pntext` and lets the reader auto-increment via `\pnstart1` +
/// shared `\pn` definitions across consecutive list paragraphs.
fn emit_list_markup(out: &mut Vec<srtf::Content>, marker: ast::ListMarker) {
    let bullet_text = "\u{00B7}"; // middle dot — scrivener-rtf escapes to \'b7
    if marker.ordered {
        // \pntext{1.\tab} visible marker + \*\pn definition (\pnlvlbody = body
        // numbering, \pnstart1 = restart at 1, \pntxta. = suffix ".").
        out.push(srtf::Content::Group(Box::new(srtf::Group {
            is_destination: false,
            content: vec![
                srtf::Content::ControlWord("pntext".into(), None),
                srtf::Content::ControlWord("f".into(), Some(0)),
                srtf::Content::Text(" 1.\t".into()),
            ],
        })));
        out.push(srtf::Content::Group(Box::new(srtf::Group {
            is_destination: true,
            content: vec![
                srtf::Content::ControlWord("pn".into(), None),
                srtf::Content::ControlWord("pnlvlbody".into(), None),
                srtf::Content::ControlWord("pnf".into(), Some(0)),
                srtf::Content::ControlWord("pnindent".into(), Some(0)),
                srtf::Content::ControlWord("pnstart".into(), Some(1)),
                srtf::Content::Group(Box::new(srtf::Group {
                    is_destination: false,
                    content: vec![
                        srtf::Content::ControlWord("pntxta".into(), None),
                        srtf::Content::Text(".".into()),
                    ],
                })),
            ],
        })));
    } else {
        // Bullet: \pntext\f0 \'b7\tab visible + \*\pn\pnlvlblt\pnf0{\pntxtb\'b7}
        out.push(srtf::Content::Group(Box::new(srtf::Group {
            is_destination: false,
            content: vec![
                srtf::Content::ControlWord("pntext".into(), None),
                srtf::Content::ControlWord("f".into(), Some(0)),
                srtf::Content::Text(format!(" {}\t", bullet_text)),
            ],
        })));
        out.push(srtf::Content::Group(Box::new(srtf::Group {
            is_destination: true,
            content: vec![
                srtf::Content::ControlWord("pn".into(), None),
                srtf::Content::ControlWord("pnlvlblt".into(), None),
                srtf::Content::ControlWord("pnf".into(), Some(0)),
                srtf::Content::Group(Box::new(srtf::Group {
                    is_destination: false,
                    content: vec![
                        srtf::Content::ControlWord("pntxtb".into(), None),
                        srtf::Content::Text(bullet_text.into()),
                    ],
                })),
            ],
        })));
    }
}

/// Emit one run as `{ <toggles> text }`. Wrapping in a group keeps each run's
/// formatting scoped — sibling runs don't inherit, so we never need a matching
/// `\b0` after a bold run; scope close handles it. If `style.link` is set,
/// the run is emitted as an RTF `{\field}` hyperlink group instead, with the
/// styled text in the `\fldrslt` slot.
fn serialize_run(run: &ast::InlineRun, out: &mut Vec<srtf::Content>, colors: &[u32]) {
    if let Some(url) = run.style.link.as_deref() {
        out.push(srtf::Content::Group(Box::new(build_hyperlink_group(
            url, run, colors,
        ))));
        return;
    }
    let mut group_content = Vec::new();
    push_style_toggles(&run.style, &mut group_content, colors);
    group_content.push(srtf::Content::Text(run.text.clone()));
    out.push(srtf::Content::Group(Box::new(srtf::Group {
        content: group_content,
        is_destination: false,
    })));
}

/// Push the style-toggle control words (`\b`, `\i`, `\ul`, `\cfN`) that
/// match `style`. Color lookup is index-into-`colors` + 1 (slot 0 = auto).
/// A color not in `colors` is a serializer bug — silently skip rather than
/// emit `\cf0` which would override to default and look wrong on save.
fn push_style_toggles(style: &ast::InlineStyle, out: &mut Vec<srtf::Content>, colors: &[u32]) {
    if style.bold {
        out.push(srtf::Content::ControlWord("b".into(), None));
    }
    if style.italic {
        out.push(srtf::Content::ControlWord("i".into(), None));
    }
    if style.underline {
        out.push(srtf::Content::ControlWord("ul".into(), None));
    }
    if style.strike {
        out.push(srtf::Content::ControlWord("strike".into(), None));
    }
    if let Some(c) = style.color {
        if let Some(idx) = colors.iter().position(|x| *x == c) {
            out.push(srtf::Content::ControlWord("cf".into(), Some((idx + 1) as i32)));
        }
    }
}

/// Build the `{\field{\*\fldinst HYPERLINK "url"}{\fldrslt …}}` group for
/// a linked run. The inner `\fldinst` group is a destination (the `\*`
/// prefix marks the destination so RTF readers that don't understand the
/// field skip the URL instead of rendering it as body text). The
/// `\fldrslt` group carries the user-visible text with the same style
/// toggles a non-linked run would get — color/bold/italic survive.
fn build_hyperlink_group(url: &str, run: &ast::InlineRun, colors: &[u32]) -> srtf::Group {
    let mut field_content = Vec::with_capacity(3);
    field_content.push(srtf::Content::ControlWord("field".into(), None));

    let fldinst = srtf::Group {
        is_destination: true,
        content: vec![
            srtf::Content::ControlWord("fldinst".into(), None),
            // Leading space plus the HYPERLINK instruction. Quotes around
            // the URL are required by the RTF spec; embedded `"` chars in
            // the URL get backslash-escaped via the text-emitter (the
            // generator handles RTF text escaping for us).
            srtf::Content::Text(format!(" HYPERLINK \"{}\"", url)),
        ],
    };
    field_content.push(srtf::Content::Group(Box::new(fldinst)));

    let mut fldrslt_content = Vec::new();
    fldrslt_content.push(srtf::Content::ControlWord("fldrslt".into(), None));
    push_style_toggles(&run.style, &mut fldrslt_content, colors);
    fldrslt_content.push(srtf::Content::Text(run.text.clone()));
    let fldrslt = srtf::Group {
        is_destination: false,
        content: fldrslt_content,
    };
    field_content.push(srtf::Content::Group(Box::new(fldrslt)));

    srtf::Group {
        is_destination: false,
        content: field_content,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_word_param_outside_i32_is_an_error() {
        // scrivener-rtf's lexer `expect`s every `\wordN` parameter to fit
        // an i32; one digit too many used to abort the app.
        for src in [
            &br"{\rtf1 \b99999999999 x}"[..],
            &br"{\rtf1 \fs-2147483649 x}"[..],
            &br"{\rtf1 \cf4294967296 x}"[..],
        ] {
            assert!(parse_bytes(src).is_err(), "{}", String::from_utf8_lossy(src));
        }
        // The i32 extremes themselves still parse.
        assert!(parse_bytes(br"{\rtf1 \fs2147483647 \li-2147483648 x}").is_ok());
    }

    fn nested_rtf(depth: usize) -> Vec<u8> {
        let mut s = b"{\\rtf1 ".to_vec();
        s.extend(std::iter::repeat_n(b'{', depth));
        s.push(b'x');
        s.extend(std::iter::repeat_n(b'}', depth));
        s.push(b'}');
        s
    }

    #[test]
    fn group_nesting_past_the_limit_is_an_error() {
        // scrivener-rtf and `walk_group` recurse once per `{`; 100k levels
        // overflowed the stack.
        let err = parse_bytes(&nested_rtf(100_000)).expect_err("too deep");
        assert!(matches!(err, RichDocError::Rtf(_)), "{err:?}");
        // The root group counts as one level.
        assert!(parse_bytes(&nested_rtf(crate::rich_doc::MAX_NESTING_DEPTH)).is_err());
        assert!(parse_bytes(&nested_rtf(crate::rich_doc::MAX_NESTING_DEPTH - 1)).is_ok());
        // Escaped braces are text, not groups.
        let mut escaped = b"{\\rtf1 ".to_vec();
        for _ in 0..100_000 {
            escaped.extend_from_slice(b"\\{");
        }
        escaped.push(b'}');
        assert!(parse_bytes(&escaped).is_ok());
    }

    #[test]
    fn hostile_rtf_is_an_error_or_a_document() {
        // Built with `bs` so no `\u` escape appears in source.
        let bs = '\\';
        let unicode = format!(
            "{{{bs}rtf1 {bs}u-70000? {bs}u2147483647 {bs}u-2147483648? {bs}uc0{bs}u65 x}}"
        );
        let refused: [&[u8]; 5] = [
            b"{",
            b"}",
            b"{\\rtf1 x",
            b"{\\rtf1\\",
            b"{\\rtf1 \\'",
        ];
        for src in refused {
            assert!(parse_bytes(src).is_err(), "{}", String::from_utf8_lossy(src));
        }
        let parsed: [&[u8]; 10] = [
            b"",
            b"{}",
            unicode.as_bytes(),
            b"{\\rtf1 \\'ZZ \\'4 \\'\\'}",
            b"{\\rtf1 \\bin2147483647 x}",
            b"{\\rtf1 \x00\xff\xfe x}",
            b"{\\rtf1\\ansicpg65001 \\'ff\\'fe\\'c3}",
            b"{\\rtf1{\\colortbl;\\red999\\green-5\\blue;}\\cf2147483647 a\\cf-1 b\\cf1 c}",
            b"{\\rtf1{\\*\\pn\\pnlvl-2147483648}\\outlinelevel-1 \\li2147483647 x\\par}",
            b"{\\rtf1 a}{\\rtf1 b}",
        ];
        for src in parsed {
            assert!(parse_bytes(src).is_ok(), "{}", String::from_utf8_lossy(src));
        }
    }

    #[test]
    fn parses_plain_hello() {
        let rtf = br"{\rtf1\ansi Hello}";
        let d = parse_bytes(rtf).expect("parses");
        assert_eq!(d.blocks.len(), 1);
        match &d.blocks[0] {
            ast::Block::Paragraph { runs, .. } => {
                assert_eq!(runs.len(), 1);
                assert_eq!(runs[0].text, "Hello");
                assert!(!runs[0].style.bold);
            }
            _ => panic!("expected paragraph"),
        }
    }

    #[test]
    fn parses_bold_run_inside_group() {
        let rtf = br"{\rtf1\ansi {\b bold} normal}";
        let d = parse_bytes(rtf).expect("parses");
        let runs = match &d.blocks[0] {
            ast::Block::Paragraph { runs, .. } => runs,
            _ => panic!(),
        };
        // Two runs: "bold" (bold), " normal" (not bold). The order matters
        // because scope close after `{\b bold}` restores style → " normal".
        assert!(runs.iter().any(|r| r.text == "bold" && r.style.bold));
        assert!(runs.iter().any(|r| r.text == " normal" && !r.style.bold));
    }

    #[test]
    fn par_starts_new_block() {
        let rtf = br"{\rtf1\ansi first\par second}";
        let d = parse_bytes(rtf).expect("parses");
        assert_eq!(d.blocks.len(), 2);
        let texts: Vec<String> = d
            .blocks
            .iter()
            .map(|b| match b {
                ast::Block::Paragraph { runs, .. } | ast::Block::Heading { runs, .. } => {
                    runs.iter().map(|r| r.text.clone()).collect::<String>()
                }
                _ => unreachable!("test corpus only contains paragraphs/headings"),
            })
            .collect();
        assert_eq!(texts, vec!["first".to_string(), "second".to_string()]);
    }

    #[test]
    fn outlinelevel_promotes_to_heading() {
        let rtf = br"{\rtf1\ansi \outlinelevel0 Title\par body}";
        let d = parse_bytes(rtf).expect("parses");
        assert_eq!(d.blocks.len(), 2);
        match &d.blocks[0] {
            ast::Block::Heading { level, runs, .. } => {
                assert_eq!(*level, 1);
                assert_eq!(runs[0].text, "Title");
            }
            _ => panic!("expected heading from outlinelevel0"),
        }
        match &d.blocks[1] {
            ast::Block::Paragraph { runs, .. } => assert_eq!(runs[0].text, "body"),
            _ => panic!("expected paragraph for body"),
        }
    }

    #[test]
    fn destination_groups_are_skipped() {
        let rtf = br"{\rtf1\ansi {\*\generator Notepad Studio}Visible}";
        let d = parse_bytes(rtf).expect("parses");
        let text: String = match &d.blocks[0] {
            ast::Block::Paragraph { runs, .. } | ast::Block::Heading { runs, .. } => {
                runs.iter().map(|r| r.text.clone()).collect()
            }
            _ => unreachable!("RTF parser never emits tables/images in Phase 3"),
        };
        assert_eq!(text, "Visible");
    }

    #[test]
    fn save_with_no_edits_writes_original_bytes() {
        let bytes: Vec<u8> = b"{\\rtf1\\ansi Hello}".to_vec();
        let payload = RichDocPayload {
            format: DocFormat::Rtf,
            ast: ast::Document::default(),
            raw_view_text: String::from_utf8(bytes.clone()).unwrap(),
            original_bytes: bytes.clone(),
            edit_log: Vec::new(),
            active_block: None,
            selection: None,
            drag_active: false,
        pending_drag_anchor: None,
        undo_stack: Vec::new(),
        redo_stack: Vec::new(),
            headers: Vec::new(),
            footers: Vec::new(),
        };
        let tmp = std::env::temp_dir().join("notepad-studio-rtf-noedit.rtf");
        save(&payload, &tmp, None).expect("saves");
        let read_back = std::fs::read(&tmp).expect("reads back");
        assert_eq!(read_back, bytes);
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn save_with_raw_override_writes_override_when_no_edits() {
        let bytes: Vec<u8> = b"{\\rtf1\\ansi Hello}".to_vec();
        let payload = RichDocPayload {
            format: DocFormat::Rtf,
            ast: ast::Document::default(),
            raw_view_text: String::from_utf8(bytes.clone()).unwrap(),
            original_bytes: bytes,
            edit_log: Vec::new(),
            active_block: None,
            selection: None,
            drag_active: false,
        pending_drag_anchor: None,
        undo_stack: Vec::new(),
        redo_stack: Vec::new(),
            headers: Vec::new(),
            footers: Vec::new(),
        };
        let tmp = std::env::temp_dir().join("notepad-studio-rtf-rawoverride.rtf");
        let edited = "{\\rtf1\\ansi HELLO}";
        save(&payload, &tmp, Some(edited)).expect("saves");
        let read_back = std::fs::read_to_string(&tmp).expect("reads back");
        assert_eq!(read_back, edited);
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn round_trip_with_bold_edit_keeps_bold_after_reopen() {
        let bytes = b"{\\rtf1\\ansi Hello world}".to_vec();
        let mut payload = RichDocPayload {
            format: DocFormat::Rtf,
            ast: parse_bytes(&bytes).expect("parses src"),
            raw_view_text: String::from_utf8(bytes.clone()).unwrap(),
            original_bytes: bytes,
            edit_log: Vec::new(),
            active_block: None,
            selection: None,
            drag_active: false,
        pending_drag_anchor: None,
        undo_stack: Vec::new(),
        redo_stack: Vec::new(),
            headers: Vec::new(),
            footers: Vec::new(),
        };
        payload.apply_edit(super::super::edits::Edit::SetBlockBold { block: 0, on: true });

        let tmp = std::env::temp_dir().join("notepad-studio-rtf-roundtrip-bold.rtf");
        save(&payload, &tmp, None).expect("saves");

        let reopened_bytes = std::fs::read(&tmp).expect("reads saved");
        let reopened = parse_bytes(&reopened_bytes).expect("reparses");
        let runs = match &reopened.blocks[0] {
            ast::Block::Paragraph { runs, .. } | ast::Block::Heading { runs, .. } => runs,
            _ => unreachable!(),
        };
        assert!(
            runs.iter().all(|r| r.style.bold),
            "bold flag must survive AST serialize → parse round-trip"
        );
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn round_trip_with_color_edit_keeps_color_after_reopen() {
        let bytes = b"{\\rtf1\\ansi Hello world}".to_vec();
        let mut payload = RichDocPayload {
            format: DocFormat::Rtf,
            ast: parse_bytes(&bytes).expect("parses src"),
            raw_view_text: String::from_utf8(bytes.clone()).unwrap(),
            original_bytes: bytes,
            edit_log: Vec::new(),
            active_block: None,
            selection: None,
            drag_active: false,
        pending_drag_anchor: None,
        undo_stack: Vec::new(),
        redo_stack: Vec::new(),
            headers: Vec::new(),
            footers: Vec::new(),
        };
        payload.apply_edit(super::super::edits::Edit::SetBlockColor {
            block: 0,
            color: Some(0xFF_00_AA),
        });

        let tmp = std::env::temp_dir().join("notepad-studio-rtf-roundtrip-color.rtf");
        save(&payload, &tmp, None).expect("saves");

        let reopened_bytes = std::fs::read(&tmp).expect("reads saved");
        let reopened = parse_bytes(&reopened_bytes).expect("reparses");
        let runs = match &reopened.blocks[0] {
            ast::Block::Paragraph { runs, .. } | ast::Block::Heading { runs, .. } => runs,
            _ => unreachable!(),
        };
        assert!(
            runs.iter().all(|r| r.style.color == Some(0xFF_00_AA)),
            "color must survive serialize → reparse round-trip"
        );
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn round_trip_with_alignment_and_indent_edits_persist() {
        let bytes = b"{\\rtf1\\ansi Hello world\\par Second line}".to_vec();
        let mut payload = RichDocPayload {
            format: DocFormat::Rtf,
            ast: parse_bytes(&bytes).expect("parses src"),
            raw_view_text: String::from_utf8(bytes.clone()).unwrap(),
            original_bytes: bytes,
            edit_log: Vec::new(),
            active_block: None,
            selection: None,
            drag_active: false,
        pending_drag_anchor: None,
        undo_stack: Vec::new(),
        redo_stack: Vec::new(),
            headers: Vec::new(),
            footers: Vec::new(),
        };
        payload.apply_edit(super::super::edits::Edit::SetBlockAlignment {
            block: 0,
            alignment: ast::Alignment::Center,
        });
        payload.apply_edit(super::super::edits::Edit::SetBlockIndent { block: 1, delta: 2 });

        let tmp = std::env::temp_dir().join("notepad-studio-rtf-roundtrip-props.rtf");
        save(&payload, &tmp, None).expect("saves");
        let reopened_bytes = std::fs::read(&tmp).expect("reads saved");
        let reopened = parse_bytes(&reopened_bytes).expect("reparses");
        let p0_props = match &reopened.blocks[0] {
            ast::Block::Paragraph { props, .. } | ast::Block::Heading { props, .. } => props.clone(),
            _ => unreachable!(),
        };
        let p1_props = match &reopened.blocks[1] {
            ast::Block::Paragraph { props, .. } | ast::Block::Heading { props, .. } => props.clone(),
            _ => unreachable!(),
        };
        assert_eq!(p0_props.alignment, ast::Alignment::Center, "centered survives");
        assert_eq!(p1_props.indent_level, 2, "indent level 2 survives");
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn parse_colortbl_indexes_into_style_color() {
        // \cf2 = green, \cf1 = red, \cf0 = clear. Build by hand because
        // scrivener-rtf would otherwise need a full doc context.
        let rtf = b"{\\rtf1\\ansi {\\colortbl;\\red255\\green0\\blue0;\\red0\\green255\\blue0;}\\cf1 red\\cf2 green\\cf0 normal}";
        let d = parse_bytes(rtf).expect("parses");
        let runs = match &d.blocks[0] {
            ast::Block::Paragraph { runs, .. } | ast::Block::Heading { runs, .. } => runs,
            _ => unreachable!("test corpus only contains a paragraph"),
        };
        // We expect runs in order: "red"(red), " green"(green), " normal"(none).
        // Whitespace lands in the trailing run because RTF separates control
        // words from text via a single space that the parser doesn't strip.
        let red_run = runs.iter().find(|r| r.text.trim() == "red").expect("red run");
        assert_eq!(red_run.style.color, Some(0xFF_00_00));
        let green_run = runs.iter().find(|r| r.text.trim() == "green").expect("green run");
        assert_eq!(green_run.style.color, Some(0x00_FF_00));
        let normal_run = runs.iter().find(|r| r.text.trim() == "normal").expect("normal run");
        assert_eq!(normal_run.style.color, None);
    }

    #[test]
    fn round_trip_with_list_toggles_preserves_kind() {
        // Two paragraphs: first becomes a bullet, second becomes numbered.
        // After serialize → reparse, both should still report the same
        // ListMarker kind. Level stays at 0 (Phase 5d.2 toolbar only emits 0).
        let bytes = b"{\\rtf1\\ansi First\\par Second}".to_vec();
        let mut payload = RichDocPayload {
            format: DocFormat::Rtf,
            ast: parse_bytes(&bytes).expect("parses src"),
            raw_view_text: String::from_utf8(bytes.clone()).unwrap(),
            original_bytes: bytes,
            edit_log: Vec::new(),
            active_block: None,
            selection: None,
            drag_active: false,
        pending_drag_anchor: None,
        undo_stack: Vec::new(),
        redo_stack: Vec::new(),
            headers: Vec::new(),
            footers: Vec::new(),
        };
        payload.apply_edit(super::super::edits::Edit::ToggleBlockList {
            block: 0,
            ordered: false,
        });
        payload.apply_edit(super::super::edits::Edit::ToggleBlockList {
            block: 1,
            ordered: true,
        });

        let tmp = std::env::temp_dir().join("notepad-studio-rtf-roundtrip-list.rtf");
        save(&payload, &tmp, None).expect("saves");
        let reopened_bytes = std::fs::read(&tmp).expect("reads saved");
        let reopened = parse_bytes(&reopened_bytes).expect("reparses");
        assert_eq!(reopened.blocks.len(), 2, "two paragraphs round-trip");
        let p0_list = match &reopened.blocks[0] {
            ast::Block::Paragraph { props, .. } | ast::Block::Heading { props, .. } => props.list,
            _ => unreachable!(),
        };
        let p1_list = match &reopened.blocks[1] {
            ast::Block::Paragraph { props, .. } | ast::Block::Heading { props, .. } => props.list,
            _ => unreachable!(),
        };
        assert_eq!(
            p0_list,
            Some(ast::ListMarker { ordered: false, level: 0 }),
            "bullet kind survives round-trip"
        );
        assert_eq!(
            p1_list,
            Some(ast::ListMarker { ordered: true, level: 0 }),
            "numbered kind survives round-trip"
        );
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn heading_round_trips_via_outlinelevel() {
        // Build AST manually since RTF source rarely has a bare \outlinelevel0
        // at the open of a paragraph in real files — Word emits via stylesheet.
        // The point of this test is that our writer + reader agree on the
        // outline-level convention end-to-end.
        let doc = ast::Document {
            blocks: vec![
                ast::Block::Heading {
                    level: 2,
                    runs: vec![ast::InlineRun::plain("Section")],
                    props: ast::BlockProps::default(),
                },
                ast::Block::Paragraph {
                    runs: vec![ast::InlineRun::plain("body")],
                    props: ast::BlockProps::default(),
                },
            ],
        };
        let rtf = serialize_ast(&doc).expect("serializes");
        let reparsed = parse_bytes(rtf.as_bytes()).expect("reparses");
        assert_eq!(reparsed.blocks.len(), 2);
        match &reparsed.blocks[0] {
            ast::Block::Heading { level, .. } => assert_eq!(*level, 2),
            _ => panic!("expected heading after round-trip"),
        }
    }
}
