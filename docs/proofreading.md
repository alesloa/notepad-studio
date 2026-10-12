# English spelling and grammar

Spelling and grammar checking runs locally using a bundled English dictionary.
Document text is never sent to a service. The initial dictionary uses American
English. Both checks are enabled by default; change them independently under
**Edit > Spelling and Grammar**. The Shortcut Mapper can assign shortcuts to
the two switches.

Misspelled words have a wavy underline in the theme's error colour. Grammar
and writing suggestions use the theme's link colour. Right-click an underlined
word to see the explanation and up to three replacements. Selecting a replacement
makes one undoable edit. Spelling suggestions also offer **Ignore Word for This
Session**, which applies in that window until it closes.

Checks apply to Normal Text and Markdown Source View. Markdown Reader, rich
documents, CSV views and programming-language buffers keep their existing
behaviour. URLs, email addresses, domains, paths, numeric tokens, dotted names,
snake_case and camelCase identifiers are excluded. Markdown code fences, inline
code and complete links are excluded too. This conservative filtering can skip
ordinary text attached to a path or identifier without whitespace.

Grammar includes article agreement, repeated words, punctuation, capitalization
and writing-style rules. Runs of spaces are layout, not grammar, so Harper's
`Spaces` rule is off: table padding and aligned columns stay unmarked. It is a
rule-based assistant and can miss errors or
make unwanted suggestions. It does not rewrite text automatically. Grammar
runs only on complete paragraphs that fit within 2,048 Unicode characters and
8 KiB; fragments beside excluded text and size splits receive spelling checks
only. This avoids inventing sentence or quotation boundaries around links.

Work starts after a 250 ms typing pause on a background executor. Results and
right-click targets are discarded after intervening edits, Undo/Redo activity,
tab changes, settings changes or document replacement. Corrections respect
read-only state and are recorded in macros. Replay requires the exact original
text selected or immediately before the caret in the focused source editor.

Documents above 1 MiB pause checking with a status message. Each snapshot admits
at most 512 engine-emitted diagnostics, sorted for painting afterward. That
limit can leave later text unchecked, even if filtering or deduplication returns
fewer than 512 marks. Only visible editor rows shape and paint underlines.
Cancellation is checked between bounded chunks; it cannot interrupt one Harper
lint call. The adapter limits its own buffers, not Harper's internal cache memory.

The engine is the unmodified Apache-2.0 [harper-core 0.33.0 release](https://docs.rs/crate/harper-core/0.33.0),
pinned to source revision `1c87e4f63c1f3debd3d42876eb90a55608cd5d9b`.
Its licence is retained in `licenses/harper-core-0.33.0/LICENSE` and attribution
is in `NOTICE`. This release avoids the newer neural model and HTML dependency
tree. We rely on the published upstream Apache licence for the curated dictionary;
the historical dictionary import does not identify every original data source.
The adapter and UI were written in Rust for Notepad Studio. No Notepad++ code,
icons, themes or GPL spell-checker implementation was copied.
