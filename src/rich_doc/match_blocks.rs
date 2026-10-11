// Flat-search highlight intersections; coordinates and match indices stay in input order.
use std::ops::Range;

type Offset = (usize, usize, usize);
type Highlight = (usize, usize, usize, usize);

pub(super) fn project(offsets: &[Offset], matches: &[Range<usize>]) -> Vec<Highlight> {
    project_with_visit(offsets, matches, || {})
}

fn project_with_visit(offsets: &[Offset], matches: &[Range<usize>], mut visit: impl FnMut()) -> Vec<Highlight> {
    if offsets.is_empty() || matches.is_empty() { return Vec::new(); }
    let mut previous_end = None;
    let ordered = offsets.iter().all(|&(_, start, end)| {
        visit();
        let valid = start <= end && previous_end.is_none_or(|previous| previous <= start);
        previous_end = Some(end);
        valid
    });
    let mut out = Vec::new();
    for (mi, m) in matches.iter().enumerate() {
        // Empty/inverted matches have no positive intersection, including on the legacy path.
        if m.start >= m.end { continue; }
        let candidates = if ordered {
            let first = offsets.partition_point(|&(_, _, end)| { visit(); end <= m.start });
            let last = offsets.partition_point(|&(_, start, _)| { visit(); start < m.end });
            offsets.get(first..last).unwrap_or_default()
        } else {
            offsets
        };
        for &(block, start, end) in candidates {
            visit();
            let lo = m.start.max(start);
            let hi = m.end.min(end);
            if hi > lo { out.push((block, lo - start, hi - start, mi)); }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // The source-before loop from RichDocPayload::matches_per_block, counted without changing its output.
    fn source_before(offsets: &[Offset], matches: &[Range<usize>]) -> (Vec<Highlight>, usize) {
        let mut out = Vec::new(); let mut visits = 0;
        for (mi, m) in matches.iter().enumerate() {
            for &(block, start, end) in offsets {
                visits += 1;
                let lo = m.start.max(start); let hi = m.end.min(end);
                if hi > lo { out.push((block, lo - start, hi - start, mi)); }
            }
        }
        (out, visits)
    }

    #[test]
    fn empty_matches_or_offsets_do_not_visit_any_offsets() {
        let offsets = [(0, 0, 3), (1, 4, 7)];
        let mut visits = 0;
        assert!(project_with_visit(&offsets, &[], || visits += 1).is_empty());
        assert_eq!(visits, 0, "no-hit render must skip map validation");
        let range = 1..2;
        let mut visits = 0;
        assert!(project_with_visit(&[], std::slice::from_ref(&range), || visits += 1).is_empty());
        assert_eq!(visits, 0);
    }

    #[test]
    fn original_match_and_offset_order_survive_cross_block_and_duplicate_ids() {
        let offsets = [(7, 0, 3), (7, 4, 4), (2, 5, 9), (4, 10, 12)];
        let matches = [6..11, 0..12, 3..5, 9..9, Range { start: 8, end: 2 }, 1..2];
        assert_eq!(project(&offsets, &matches), source_before(&offsets, &matches).0);
        assert_eq!(project(&offsets, std::slice::from_ref(&(6..11))), vec![(2, 1, 4, 0), (4, 0, 1, 0)]);
    }

    #[test]
    fn malformed_and_overlapping_legacy_offsets_keep_exact_fallback_order() {
        let cases = [vec![(8, 5, 9), (2, 0, 4)], vec![(1, 0, 8), (2, 3, 9)],
            vec![(1, 8, 2), (2, 0, 9)], vec![(1, 0, 4), (1, 0, 4)]];
        let matches = [0..10, 6..7, Range { start: 10, end: 0 }, 4..4];
        for offsets in cases { assert_eq!(project(&offsets, &matches), source_before(&offsets, &matches).0); }
    }

    #[test]
    fn exhaustive_small_offsets_ranges_and_extreme_coordinates_are_equivalent() {
        let matches: Vec<_> = (0..=4).flat_map(|start| (0..=4).map(move |end| start..end)).collect();
        for a in 0..=4 { for b in 0..=4 { for c in 0..=4 { for d in 0..=4 {
            let offsets = [(3, a, b), (1, c, d)];
            assert_eq!(project(&offsets, &matches), source_before(&offsets, &matches).0);
        } } } }
        let offsets = [(usize::MAX, 0, 0), (1, 1, usize::MAX), (2, usize::MAX, usize::MAX)];
        let ranges = [0..usize::MAX, usize::MAX..usize::MAX, Range { start: usize::MAX, end: 0 }, 1..2];
        assert_eq!(project(&offsets, &ranges), source_before(&offsets, &ranges).0);
        assert!(project(&[], &ranges).is_empty());
    }

    #[test]
    fn actual_payload_flat_search_offsets_project_unicode_empty_and_nontext_blocks() {
        use crate::rich_doc::{ast::{Block, BlockProps, InlineRun}, rtf};
        let mut payload = rtf::open_bytes(br"{\rtf1\ansi fixture}".to_vec()).unwrap();
        payload.ast.blocks = vec![
            Block::Paragraph { runs: vec![InlineRun::plain("é中")], props: BlockProps::default() },
            Block::Table { rows: vec![] },
            Block::Paragraph { runs: vec![], props: BlockProps::default() },
            Block::Heading { level: 2, runs: vec![InlineRun::plain("😀z")], props: BlockProps::default() },
        ];
        let (text, offsets) = payload.flat_search_text();
        assert_eq!(text, "é中\n\n😀z");
        assert_eq!(offsets, vec![(0, 0, 5), (2, 6, 6), (3, 7, 12)]);
        let matches = [2..11, 7..11, 5..7, 6..6];
        assert_eq!(payload.matches_per_block(&offsets, &matches), source_before(&offsets, &matches).0);
        assert_eq!(payload.matches_per_block(&offsets, std::slice::from_ref(&(2..11))), vec![(0, 2, 5, 0), (3, 0, 4, 0)]);
    }

    #[test]
    fn sparse_complete_map_avoids_source_before_match_times_block_work() {
        let offsets: Vec<_> = (0..4096).map(|i| (i, i * 5, i * 5 + 4)).collect();
        let matches: Vec<_> = offsets.iter().rev().map(|&(_, start, _)| start + 1..start + 2).collect();
        let (expected, before) = source_before(&offsets, &matches);
        let mut after = 0;
        let actual = project_with_visit(&offsets, &matches, || after += 1);
        assert_eq!(actual, expected); assert_eq!(before, 16_777_216);
        assert!(after < 120_000, "actual validation/binary/intersection visits={after}");
        println!("source_before_intersections={before} candidate_total_visits={after}");
    }

    #[test]
    #[ignore = "matched release actual payload projection; coordinate equivalence asserted per sample"]
    fn optimized_actual_payload_projection_diagnostic() {
        use crate::rich_doc::{ast::{Block, BlockProps, InlineRun}, rtf};
        use std::{hint::black_box, time::Instant};
        println!("match_blocks_fixture_manifest={}", env!("CARGO_MANIFEST_DIR"));
        for empty in [false, true] {
            let mut payload = rtf::open_bytes(br"{\rtf1\ansi fixture}".to_vec()).unwrap();
            let mut blocks = Vec::new();
            for _ in 0..25_000 {
                if empty { blocks.push(Block::Paragraph { runs: vec![], props: BlockProps::default() }); }
                blocks.push(Block::Paragraph {
                    runs: vec![InlineRun::plain(format!("{}!", "x".repeat(511)))], props: BlockProps::default(),
                });
            }
            payload.ast.blocks = blocks;
            let (text, offsets) = payload.flat_search_text();
            assert!(text.len() > 10 * 1024 * 1024);
            let matches: Vec<_> = offsets.iter().rev().filter(|&&(_, start, end)| end > start)
                .map(|&(_, _, end)| end - 1..end).collect();
            let expected: Vec<_> = offsets.iter().rev().filter(|&&(_, start, end)| end > start)
                .enumerate().map(|(mi, &(block, _, _))| (block, 511, 512, mi)).collect();
            let warm = payload.matches_per_block(black_box(&offsets), black_box(&matches));
            assert_eq!(warm, expected);
            for sample in 0..5 {
                let started = Instant::now();
                let actual = payload.matches_per_block(black_box(&offsets), black_box(&matches));
                let elapsed = started.elapsed();
                assert_eq!(actual, expected);
                println!("empty={empty} sample={sample} bytes={} blocks={} matches={} output={} method_us={}",
                    text.len(), offsets.len(), matches.len(), actual.len(), elapsed.as_micros());
            }
        }
    }

}
