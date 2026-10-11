// Original bounded tree-sitter collection; file-sized work runs on the background executor.
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};

use tree_sitter_highlight::{Highlight, HighlightEvent};

use super::{HighlightSpan, LanguageId};

pub(crate) const MAX_SPANS: usize = 1_000_000;
const MAX_DEPTH: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HighlightError {
    Cancelled,
    SizeLimit,
    SpanLimit,
    DepthLimit,
    Allocation,
    Unavailable,
}

impl fmt::Display for HighlightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "syntax parsing was cancelled",
            Self::SizeLimit => "syntax highlighting stops above 50 MiB",
            Self::SpanLimit => {
                "syntax highlighting exceeded one million spans; use Normal Text for this document"
            }
            Self::DepthLimit => {
                "syntax highlighting exceeded its nesting limit; use Normal Text for this document"
            }
            Self::Allocation => "there is not enough memory for syntax highlighting",
            Self::Unavailable => "the syntax grammar could not parse this document",
        })
    }
}

pub(crate) fn cancelled(flag: Option<&AtomicUsize>) -> bool {
    flag.is_some_and(|flag| flag.load(Ordering::Relaxed) != 0)
}

pub(crate) fn highlight_cancellable(
    language: LanguageId,
    source: &str,
    cancel: Option<&AtomicUsize>,
) -> Result<Vec<HighlightSpan>, HighlightError> {
    if source.len() as u64 > crate::io_stream::HIGHLIGHT_SIZE_LIMIT {
        return Err(HighlightError::SizeLimit);
    }
    if cancelled(cancel) {
        return Err(HighlightError::Cancelled);
    }
    super::HIGHLIGHTER.with(|cell| {
        let mut highlighter = cell
            .try_borrow_mut()
            .map_err(|_| HighlightError::Unavailable)?;
        super::CONFIGS.with(|cell| {
            let mut configs = cell
                .try_borrow_mut()
                .map_err(|_| HighlightError::Unavailable)?;
            if let std::collections::hash_map::Entry::Vacant(entry) = configs.entry(language) {
                let mut config =
                    super::grammars::build_config(language).ok_or(HighlightError::Unavailable)?;
                config.configure(super::HIGHLIGHT_NAMES);
                entry.insert(config);
            }
            let config = configs.get(&language).ok_or(HighlightError::Unavailable)?;
            let events = highlighter
                .highlight(config, source.as_bytes(), cancel, |_| None)
                .map_err(|_| {
                    if cancelled(cancel) {
                        HighlightError::Cancelled
                    } else {
                        HighlightError::Unavailable
                    }
                })?;
            collect(events, source, cancel, MAX_SPANS, MAX_DEPTH)
        })
    })
}

fn reserve<T>(items: &mut Vec<T>, maximum: usize) -> Result<(), HighlightError> {
    if items.len() == items.capacity() {
        let additional = items
            .capacity()
            .max(64)
            .min(maximum.saturating_sub(items.len()));
        items
            .try_reserve_exact(additional)
            .map_err(|_| HighlightError::Allocation)?;
    }
    Ok(())
}

fn collect(
    events: impl Iterator<Item = Result<HighlightEvent, tree_sitter_highlight::Error>>,
    source: &str,
    cancel: Option<&AtomicUsize>,
    max_spans: usize,
    max_depth: usize,
) -> Result<Vec<HighlightSpan>, HighlightError> {
    let mut stack = Vec::new();
    let mut spans = Vec::new();
    for event in events {
        if cancelled(cancel) {
            return Err(HighlightError::Cancelled);
        }
        match event.map_err(|_| HighlightError::Unavailable)? {
            HighlightEvent::Source { start, end } => {
                if start >= end {
                    continue;
                }
                if source.get(start..end).is_none() {
                    return Err(HighlightError::Unavailable);
                }
                if let Some(kind) = stack.last().and_then(|&index| super::index_to_kind(index)) {
                    if spans.len() >= max_spans {
                        return Err(HighlightError::SpanLimit);
                    }
                    reserve(&mut spans, max_spans)?;
                    spans.push(HighlightSpan {
                        range: start..end,
                        kind,
                    });
                }
            }
            HighlightEvent::HighlightStart(Highlight(index)) => {
                if stack.len() >= max_depth {
                    return Err(HighlightError::DepthLimit);
                }
                reserve(&mut stack, max_depth)?;
                stack.push(index);
            }
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
        }
    }
    if cancelled(cancel) {
        return Err(HighlightError::Cancelled);
    }
    Ok(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events() -> impl Iterator<Item = Result<HighlightEvent, tree_sitter_highlight::Error>> {
        [
            HighlightEvent::HighlightStart(Highlight(0)),
            HighlightEvent::Source { start: 0, end: 2 },
            HighlightEvent::Source { start: 2, end: 4 },
            HighlightEvent::HighlightEnd,
        ]
        .into_iter()
        .map(Ok)
    }

    #[test]
    fn collection_refuses_before_growing_past_caps_or_invalid_boundaries() {
        assert!(matches!(
            collect(events(), "abcd", None, 1, 2),
            Err(HighlightError::SpanLimit)
        ));
        assert!(matches!(
            collect(events(), "abcd", None, 2, 0),
            Err(HighlightError::DepthLimit)
        ));
        assert!(matches!(
            collect(events(), "中a", None, 2, 2),
            Err(HighlightError::Unavailable)
        ));
    }

    #[test]
    fn cancellation_never_returns_partial_spans() {
        let cancel = AtomicUsize::new(1);
        assert!(matches!(
            highlight_cancellable(LanguageId::Rust, "fn café() {}", Some(&cancel)),
            Err(HighlightError::Cancelled)
        ));
        assert!(matches!(
            collect(events(), "abcd", Some(&cancel), 2, 2),
            Err(HighlightError::Cancelled)
        ));
    }

    #[test]
    fn a_reentrant_highlighter_refuses_instead_of_panicking() {
        super::super::HIGHLIGHTER.with(|cell| {
            let _borrowed = cell.borrow_mut();
            assert!(matches!(
                highlight_cancellable(LanguageId::Rust, "fn main() {}", None),
                Err(HighlightError::Unavailable)
            ));
        });
        super::super::CONFIGS.with(|cell| {
            let _borrowed = cell.borrow_mut();
            assert!(matches!(
                highlight_cancellable(LanguageId::Rust, "fn main() {}", None),
                Err(HighlightError::Unavailable)
            ));
        });
    }

    #[test]
    fn unicode_crlf_spans_are_valid_utf8_ranges() {
        let text = "// café 中😀\r\nfn main() { let value = \"中\"; }\r\n";
        let spans = highlight_cancellable(LanguageId::Rust, text, None).unwrap();
        assert!(!spans.is_empty());
        assert!(spans
            .iter()
            .all(|span| text.get(span.range.clone()).is_some()));
    }
}
