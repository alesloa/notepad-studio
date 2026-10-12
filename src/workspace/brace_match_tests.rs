use super::*;
use std::cell::Cell;

thread_local! { static VISITED: Cell<usize> = const { Cell::new(0) }; }
pub(super) fn visited_byte() { VISITED.with(|count| count.set(count.get().saturating_add(1))); }
pub(in crate::workspace) fn take_visited() -> usize { VISITED.with(|count| count.replace(0)) }

fn pair(text: &str, head: usize, spans: &[HighlightSpan]) -> Option<(usize, usize)> {
    locate(&Rope::from_str(text), head, spans, &AtomicBool::new(false)).unwrap().map(Pair::ordered)
}

#[test]
fn brace_at_caret_and_before_caret_priority() {
    assert_eq!(pair("(é)", 0, &[]), Some((0, 3)));
    assert_eq!(pair(")(x)", 1, &[]), None);
    assert_eq!(pair("(x)[y]", 3, &[]), Some((0, 2)));
}

#[test]
fn string_and_comment_origins_match_only_their_own_style() {
    for kind in [HighlightKind::String, HighlightKind::Comment] {
        assert_eq!(pair("(é)", 1, &[HighlightSpan { range: 0..4, kind }]), Some((0, 3)));
        let spans = [HighlightSpan { range: 1..4, kind }];
        assert_eq!(pair("(\")\")", 1, &spans), Some((0, 4)));
        assert_eq!(pair("(\")\")", 3, &spans), None);
    }
}

#[test]
fn nested_unicode_and_backward_scans_share_one_policy() {
    let text = "{é[中](😀)}";
    for head in [0, 1, text.len()] {
        assert_eq!(pair(text, head, &[]), Some((0, text.len()-1)));
    }
    assert_eq!(pair("(é(中)\")\")", 1, &[HighlightSpan { range: 8..11, kind: HighlightKind::String }]), Some((0, 11)));
}

#[test]
fn hostile_carets_and_styles_are_refused_without_indexing_panics() {
    for head in [2, usize::MAX] { assert_eq!(pair("(é)", head, &[]), None); }
    assert_eq!(pair("", 0, &[]), None);
    for spans in [vec![HighlightSpan { range: 2..3, kind: HighlightKind::String }],
        vec![HighlightSpan { range: 0..usize::MAX, kind: HighlightKind::Comment }],
        vec![HighlightSpan { range: 0..4, kind: HighlightKind::String }, HighlightSpan { range: 1..4, kind: HighlightKind::Comment }]] {
        assert!(locate(&Rope::from_str("(é)"), 1, &spans, &AtomicBool::new(false)).is_err());
    }
}

#[test]
fn cancellation_and_deep_nesting_use_checked_iterative_work() {
    let text = format!("{}{}", "(".repeat(40_000), ")".repeat(40_000));
    assert_eq!(pair(&text, 1, &[]), Some((0, text.len()-1)));
    assert!(locate(&Rope::from_str(&text), 1, &[], &AtomicBool::new(true)).is_err());
    let spans = vec![HighlightSpan { range: 0..1, kind: HighlightKind::String }; MAX_STYLE_SPANS+1];
    assert!(locate(&Rope::from_str("()"), 1, &spans, &AtomicBool::new(false)).is_err());
}
