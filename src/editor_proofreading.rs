use gpui::{TextRun, UnderlineStyle, px, rgb};

use crate::proofreading::{Issue, IssueKind};
use crate::theme::Theme;

pub(super) fn apply(
    runs: &mut Vec<TextRun>, text: &str, start: usize, issues: &[Issue], theme: &Theme,
) {
    let end = start.saturating_add(text.len());
    let visible: Vec<_> = issues.iter()
        .take_while(|issue| issue.range.start < end)
        .filter(|issue| issue.range.end > start).collect();
    if visible.is_empty() { return; }
    let mut result = Vec::with_capacity(runs.len().saturating_add(visible.len().saturating_mul(2)));
    let mut cursor: usize = 0;
    for run in std::mem::take(runs) {
        let run_end = cursor.saturating_add(run.len).min(text.len());
        let mut position = cursor;
        for issue in &visible {
            let lo = text.floor_char_boundary(issue.range.start.saturating_sub(start))
                .max(position).min(run_end);
            let hi = text.floor_char_boundary(issue.range.end.saturating_sub(start))
                .min(run_end);
            if hi <= lo { continue; }
            if position < lo {
                let mut piece = run.clone(); piece.len = lo - position; result.push(piece);
            }
            let mut piece = run.clone(); piece.len = hi - lo;
            piece.underline = Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(rgb(match issue.kind {
                    IssueKind::Spelling => theme.error,
                    IssueKind::Grammar => theme.link,
                }).into()),
                wavy: true,
            });
            result.push(piece);
            position = hi;
        }
        if position < run_end {
            let mut piece = run; piece.len = run_end - position; result.push(piece);
        }
        cursor = run_end;
    }
    *runs = result;
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Font, Hsla};

    fn run(len: usize) -> TextRun {
        TextRun { len, font: Font::default(), color: Hsla::default(),
            background_color: None, underline: None, strikethrough: None }
    }
    fn issue(range: std::ops::Range<usize>, kind: IssueKind) -> Issue {
        Issue { range, kind, message: String::new(), suggestions: vec![], original: String::new() }
    }
    #[test]
    fn spelling_and_grammar_add_waves_without_changing_glyphs_or_fonts() {
        let theme = Theme::for_mode(crate::theme::ThemeMode::Dark);
        let mut runs = vec![run(7), run(5)];
        apply(&mut runs, "badword test", 0,
            &[issue(0..7, IssueKind::Spelling), issue(8..12, IssueKind::Grammar)], &theme);
        assert_eq!(runs.iter().map(|run| run.len).sum::<usize>(), 12);
        assert_eq!(runs.iter().filter(|run| run.underline.is_some()).count(), 2);
        assert!(runs.iter().all(|run| run.font == Font::default()));
        assert!(runs.iter().filter_map(|run| run.underline).all(|style| style.wavy));
        assert_eq!(runs[0].underline.unwrap().color, Some(rgb(theme.error).into()));
        assert_eq!(runs[2].underline.unwrap().color, Some(rgb(theme.link).into()));
    }
    #[test]
    fn wrapped_unicode_segment_clamps_absolute_ranges_on_char_boundaries() {
        let theme = Theme::for_mode(crate::theme::ThemeMode::Light);
        let mut runs = vec![run("é typo".len())];
        apply(&mut runs, "é typo", 10,
            &[issue(8..11, IssueKind::Grammar), issue(13..100, IssueKind::Spelling)], &theme);
        let mut position = 0;
        for run in runs { position += run.len; assert!("é typo".is_char_boundary(position)); }
        assert_eq!(position, "é typo".len());
    }
    #[test]
    fn distant_issues_leave_existing_link_underlines_untouched() {
        let theme = Theme::for_mode(crate::theme::ThemeMode::Dark);
        let mut original = run(4);
        original.underline = Some(UnderlineStyle { thickness: px(1.), color: None, wavy: false });
        let mut runs = vec![original.clone()];
        apply(&mut runs, "link", 0, &[issue(100..104, IssueKind::Spelling)], &theme);
        assert_eq!(runs, vec![original]);
    }
}
