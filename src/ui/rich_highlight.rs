// Original endpoint resolver preserving gpui LineLayout::x_for_index traversal order.
use gpui::{LineLayout, Pixels};

const SCRATCH_BYTES: usize = 8 * 1024 * 1024;
struct Endpoint {
    byte: usize,
    order: usize,
    x: Pixels,
}

pub(super) fn visit_ranges(
    line: &LineLayout,
    matches: &[(usize, usize, bool)],
    prefix: usize,
    visit: impl FnMut(Pixels, Pixels, bool),
) {
    visit_with(
        line,
        matches,
        prefix,
        SCRATCH_BYTES,
        |v, count| v.try_reserve_exact(count).map_err(|_| ()),
        visit,
    );
}

fn visit_with(
    line: &LineLayout,
    matches: &[(usize, usize, bool)],
    prefix: usize,
    limit: usize,
    reserve: impl FnOnce(&mut Vec<Endpoint>, usize) -> Result<(), ()>,
    mut visit: impl FnMut(Pixels, Pixels, bool),
) {
    if matches.is_empty() {
        return;
    }
    let chunk_size = matches
        .len()
        .min(limit / (2 * std::mem::size_of::<Endpoint>()));
    let bank = chunk_size
        .checked_mul(2)
        .and_then(|count| reserve_bank(count, limit, reserve));
    if let Some(mut endpoints) = bank.filter(|_| chunk_size > 0) {
        for chunk in matches.chunks(chunk_size) {
            resolve_into(line, chunk, prefix, &mut endpoints);
            for (lo, hi) in endpoints
                .iter()
                .step_by(2)
                .zip(endpoints.iter().skip(1).step_by(2))
            {
                let Some(&(_, _, current)) = chunk.get(lo.order / 2) else {
                    continue;
                };
                visit(lo.x, hi.x, current);
            }
        }
    } else {
        // Scratch refusal changes cost only; every original match is still painted.
        for &(lo, hi, current) in matches {
            let lo = lo.saturating_add(prefix).min(line.len);
            let hi = hi.saturating_add(prefix).min(line.len);
            if hi <= lo {
                continue;
            }
            #[cfg(test)]
            record_linear_queries(2);
            visit(line.x_for_index(lo), line.x_for_index(hi), current);
        }
    }
}

fn reserve_bank(
    count: usize,
    limit: usize,
    reserve: impl FnOnce(&mut Vec<Endpoint>, usize) -> Result<(), ()>,
) -> Option<Vec<Endpoint>> {
    let bytes = count.checked_mul(std::mem::size_of::<Endpoint>())?;
    if bytes > limit {
        return None;
    }
    let mut endpoints = Vec::new();
    reserve(&mut endpoints, count).ok()?;
    if endpoints.capacity() < count
        || endpoints
            .capacity()
            .checked_mul(std::mem::size_of::<Endpoint>())?
            > limit
    {
        return None;
    }
    Some(endpoints)
}

#[cfg(test)]
fn resolve(
    line: &LineLayout,
    matches: &[(usize, usize, bool)],
    prefix: usize,
    limit: usize,
    reserve: impl FnOnce(&mut Vec<Endpoint>, usize) -> Result<(), ()>,
) -> Option<Vec<Endpoint>> {
    let mut endpoints = reserve_bank(matches.len().checked_mul(2)?, limit, reserve)?;
    resolve_into(line, matches, prefix, &mut endpoints);
    Some(endpoints)
}

fn resolve_into(
    line: &LineLayout,
    matches: &[(usize, usize, bool)],
    prefix: usize,
    endpoints: &mut Vec<Endpoint>,
) {
    endpoints.clear();
    for (index, &(lo, hi, _)) in matches.iter().enumerate() {
        let lo = lo.saturating_add(prefix).min(line.len);
        let hi = hi.saturating_add(prefix).min(line.len);
        if hi <= lo {
            continue;
        }
        let order = index.saturating_mul(2);
        endpoints.push(Endpoint {
            byte: lo,
            order,
            x: line.width,
        });
        endpoints.push(Endpoint {
            byte: hi,
            order: order.saturating_add(1),
            x: line.width,
        });
    }
    endpoints.sort_unstable_by_key(|endpoint| endpoint.byte);
    let mut next = 0usize;
    // Sorting glyphs by byte index would change first-qualifying-glyph semantics.
    for glyph in line.runs.iter().flat_map(|run| &run.glyphs) {
        if next == endpoints.len() {
            break;
        }
        #[cfg(test)]
        record(|s| s.glyphs += 1);
        while let Some(endpoint) = endpoints.get_mut(next).filter(|e| e.byte <= glyph.index) {
            endpoint.x = glyph.position.x;
            next = next.saturating_add(1);
        }
    }
    endpoints.sort_unstable_by_key(|endpoint| endpoint.order);
}

#[cfg(test)]
#[derive(Clone, Default, Debug)]
struct PaintStats {
    prepaint_shapes: usize,
    linear_queries: usize,
    glyphs: usize,
    matches: usize,
    quads: Vec<ObservedQuad>,
}
#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
struct ObservedQuad {
    bounds: gpui::Bounds<Pixels>,
    background: gpui::Background,
    mask: gpui::ContentMask<Pixels>,
}
#[cfg(test)]
thread_local! {
    static PROBE: std::cell::RefCell<Option<std::rc::Rc<std::cell::RefCell<PaintStats>>>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
fn record(f: impl FnOnce(&mut PaintStats)) {
    PROBE.with(|slot| {
        if let Some(stats) = slot.borrow().as_ref() {
            f(&mut stats.borrow_mut());
        }
    });
}
#[cfg(test)]
pub(super) fn record_prepaint_shape() {
    record(|s| s.prepaint_shapes += 1);
}
#[cfg(test)]
pub(super) fn record_quad(quad: &gpui::PaintQuad, mask: gpui::ContentMask<Pixels>) {
    record(|s| {
        if s.quads.len() < 16_384 {
            s.quads.push(ObservedQuad {
                bounds: quad.bounds,
                background: quad.background,
                mask,
            });
        }
    });
}
#[cfg(test)]
pub(super) fn record_match() {
    record(|s| s.matches += 1);
}
#[cfg(test)]
pub(super) fn record_linear_queries(count: usize) {
    record(|s| s.linear_queries += count);
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;
    use gpui::{AppContext, TestAppContext};

    struct ProbeGuard(Option<std::rc::Rc<std::cell::RefCell<PaintStats>>>);
    impl Drop for ProbeGuard {
        fn drop(&mut self) {
            PROBE.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    fn paint_stats(clipped: bool) -> PaintStats {
        let stats = std::rc::Rc::new(std::cell::RefCell::new(PaintStats::default()));
        let _guard = ProbeGuard(PROBE.with(|slot| slot.borrow_mut().replace(stats.clone())));
        let _settings = crate::settings::override_for_test(Default::default());
        let mut cx = TestAppContext::single();
        let workspace = cx.new(|cx| {
            let mut w = Workspace::new(
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                1.,
                gpui::WindowAppearance::Dark,
            );
            w.window_closed = true;
            w
        });
        let visual = cx.add_empty_window();
        let theme = Theme::for_mode(crate::theme::ThemeMode::Dark);
        let text = "a ".repeat(128);
        let mut matches: Vec<_> = (0..128).map(|i| (i * 2, i * 2 + 1, i == 64)).collect();
        matches.rotate_left(37); // Paint order is independent of endpoint byte sorting.
        let (combined, runs) = build_combined(
            Some("1. "),
            &[InlineRun {
                text: text.clone(),
                style: Default::default(),
            }],
            &theme,
            16.,
            FontWeight::NORMAL,
        );
        let (line, line_height, mask) = visual.update(|window, _| {
            (
                window
                    .text_system()
                    .shape_line(combined.into(), px(16.), &runs, None),
                window.line_height(),
                gpui::ContentMask {
                    bounds: Bounds::new(Default::default(), window.viewport_size()),
                },
            )
        });
        let mask = if clipped {
            gpui::ContentMask {
                bounds: Bounds::new(point(px(10.), px(20.)), size(px(120.), px(30.))),
            }
            .intersect(&mask)
        } else {
            mask
        };
        let expected: Vec<_> = matches
            .iter()
            .map(|&(lo, hi, current)| {
                let lo = px(10.) + line.x_for_index(lo + 3);
                let hi = px(10.) + line.x_for_index(hi + 3);
                ObservedQuad {
                    bounds: Bounds::new(point(lo, px(20.)), size(hi - lo, line_height)),
                    background: Hsla::from(rgba(if current { 0xff8c4290 } else { 0xfff39960 }))
                        .into(),
                    mask: mask.clone(),
                }
            })
            .collect();
        visual.draw(
            point(px(10.), px(20.)),
            size(px(800.), px(200.)),
            |_, cx| {
                let inline = EditableInline {
                    runs: vec![InlineRun {
                        text: text.clone(),
                        style: InlineStyle::default(),
                    }],
                    prefix: Some("1. ".to_owned()),
                    prefix_len: 3,
                    block_idx: 0,
                    selection: None,
                    is_cursor_block: false,
                    is_first_textual: true,
                    is_last_textual: true,
                    workspace: workspace.downgrade(),
                    focus: cx.focus_handle(),
                    theme,
                    base_size_px: 16.,
                    base_weight: FontWeight::NORMAL,
                    match_highlights: matches.clone(),
                    scroll_handle: ScrollHandle::new(),
                };
                if clipped {
                    div()
                        .w(px(120.))
                        .h(px(30.))
                        .overflow_hidden()
                        .child(inline)
                        .into_any_element()
                } else {
                    inline.into_any_element()
                }
            },
        );
        let result = stats.borrow().clone();
        assert_eq!(result.quads, expected);
        if clipped {
            assert!(
                result
                    .quads
                    .iter()
                    .any(|q| q.bounds.right() > q.mask.bounds.right())
            );
        }
        eprintln!(
            "rich-highlight provenance: manifest={} source={} prepaint_shapes={} linear_queries={} glyphs={} matches={} observed_quads={}",
            env!("CARGO_MANIFEST_DIR"),
            option_env!("RICH_HIGHLIGHT_SOURCE").unwrap_or("unset"),
            result.prepaint_shapes,
            result.linear_queries,
            result.glyphs,
            result.matches,
            result.quads.len()
        );
        cx.quit();
        result
    }
    #[test]
    fn rich_highlight_actual_paint_reuses_prepaint_shaping() {
        assert_eq!(paint_stats(false).prepaint_shapes, 1);
    }
    #[test]
    fn rich_highlight_actual_paint_does_not_rescan_glyphs_per_match() {
        let stats = paint_stats(false);
        assert_eq!(stats.linear_queries, 0);
        assert!(stats.glyphs <= 259);
        assert_eq!(stats.matches, 128);
    }
    #[test]
    fn rich_highlight_actual_paint_preserves_rectangles_colors_order_prefix_and_clip_mask() {
        assert_eq!(paint_stats(true).matches, 128);
    }
    fn shaped(texts: &[&str]) -> ShapedLine {
        let mut cx = TestAppContext::single();
        let visual = cx.add_empty_window();
        let runs: Vec<_> = texts
            .iter()
            .enumerate()
            .map(|(i, text)| InlineRun {
                text: (*text).to_owned(),
                style: InlineStyle {
                    bold: i % 2 == 1,
                    ..Default::default()
                },
            })
            .collect();
        let theme = Theme::for_mode(crate::theme::ThemeMode::Dark);
        let (text, runs) = build_combined(None, &runs, &theme, 16., FontWeight::NORMAL);
        let line = visual.update(|window, _| {
            window
                .text_system()
                .shape_line(text.into(), px(16.), &runs, None)
        });
        cx.quit();
        line
    }
    fn expected(
        line: &gpui::LineLayout,
        matches: &[(usize, usize, bool)],
        prefix: usize,
    ) -> Vec<(Pixels, Pixels, bool)> {
        matches
            .iter()
            .filter_map(|&(lo, hi, current)| {
                let lo = lo.saturating_add(prefix).min(line.len);
                let hi = hi.saturating_add(prefix).min(line.len);
                (hi > lo).then(|| (line.x_for_index(lo), line.x_for_index(hi), current))
            })
            .collect()
    }
    fn equivalent(line: &gpui::LineLayout) {
        let matches = [
            (3, 7, true),
            (0, 1, false),
            (1, 4, false),
            (0, line.len + 10, true),
            (7, 3, false),
            (line.len, line.len + 1, false),
            (0, usize::MAX, true),
            (3, 7, false),
        ];
        for prefix in [0, 3, usize::MAX] {
            let mut actual = Vec::new();
            visit_ranges(line, &matches, prefix, |lo, hi, current| {
                actual.push((lo, hi, current))
            });
            assert_eq!(actual, expected(line, &matches, prefix));
        }
    }
    #[test]
    fn rich_highlight_lookup_equals_vendored_lookup_on_actual_unicode_and_style_runs() {
        for texts in [
            vec!["fi ffi e\u{301} 😀 中 العربية"],
            vec!["café", " e\u{301} fi ", "😀 中"],
            vec![""],
        ] {
            equivalent(&shaped(&texts));
        }
    }
    #[test]
    fn rich_highlight_lookup_preserves_duplicate_decreasing_indices_and_empty_runs() {
        let base = shaped(&["abcdef"]);
        let glyph = base
            .runs
            .iter()
            .flat_map(|r| &r.glyphs)
            .next()
            .unwrap()
            .clone();
        let glyphs = [
            (3, 8.),
            (3, -2.),
            (1, 10.),
            (10, 4.),
            (5, 20.),
            (12, 29.),
            (usize::MAX, 30.),
        ]
        .into_iter()
        .map(|(index, x)| {
            let mut glyph = glyph.clone();
            glyph.index = index;
            glyph.position.x = px(x);
            glyph
        })
        .collect();
        let line = gpui::LineLayout {
            width: px(31.),
            len: 12,
            runs: vec![
                gpui::ShapedRun {
                    font_id: gpui::FontId(0),
                    glyphs: Vec::new(),
                },
                gpui::ShapedRun {
                    font_id: gpui::FontId(0),
                    glyphs,
                },
                gpui::ShapedRun {
                    font_id: gpui::FontId(0),
                    glyphs: Vec::new(),
                },
            ],
            ..Default::default()
        };
        equivalent(&line);
        for index in [line.len, usize::MAX] {
            let mut only = glyph.clone();
            only.index = index;
            only.position.x = px(-7.);
            equivalent(&gpui::LineLayout {
                width: px(31.),
                len: 12,
                runs: vec![gpui::ShapedRun {
                    font_id: gpui::FontId(0),
                    glyphs: vec![only],
                }],
                ..Default::default()
            });
        }
        equivalent(&gpui::LineLayout {
            width: px(31.),
            len: 12,
            ..Default::default()
        });
    }
    #[test]
    fn rich_highlight_scratch_count_capacity_and_reservation_refusal_preserve_all_matches() {
        let line = shaped(&["abcdef"]);
        let matches = [(0, 2, false), (1, 3, true), (4, 5, false)];
        let exact = matches.len() * 2 * std::mem::size_of::<Endpoint>();
        let reserve = |v: &mut Vec<Endpoint>, count| v.try_reserve_exact(count).map_err(|_| ());
        let xs = resolve(&line, &matches, 0, exact, reserve).unwrap();
        assert!(xs.capacity() * std::mem::size_of::<Endpoint>() <= exact);
        for limit in [0, exact - 1, exact] {
            let mut actual = Vec::new();
            visit_with(&line, &matches, 0, limit, reserve, |lo, hi, current| {
                actual.push((lo, hi, current))
            });
            assert_eq!(actual, expected(&line, &matches, 0));
        }
        let mut actual = Vec::new();
        visit_with(
            &line,
            &matches,
            0,
            exact,
            |_, _| Err(()),
            |lo, hi, current| actual.push((lo, hi, current)),
        );
        assert_eq!(actual, expected(&line, &matches, 0));
        assert!(
            resolve(&line, &matches, 0, exact, |v, count| v
                .try_reserve_exact(count + 1)
                .map_err(|_| ()))
            .is_none()
        );
        let count = SCRATCH_BYTES / (2 * std::mem::size_of::<Endpoint>()) + 1;
        let many: Vec<_> = (0..count).map(|i| (i % 4, i % 4 + 2, i % 3 == 0)).collect();
        let stats = std::rc::Rc::new(std::cell::RefCell::new(PaintStats::default()));
        let _guard = ProbeGuard(PROBE.with(|slot| slot.borrow_mut().replace(stats.clone())));
        let mut visited = 0;
        let mut reservations = 0;
        visit_with(
            &line,
            &many,
            0,
            SCRATCH_BYTES,
            |v, count| {
                reservations += 1;
                v.try_reserve_exact(count).map_err(|_| ())?;
                assert!(v.capacity() * std::mem::size_of::<Endpoint>() <= SCRATCH_BYTES);
                Ok(())
            },
            |lo, hi, current| {
                let &(start, end, flag) = many.get(visited).unwrap();
                assert_eq!(
                    (lo, hi, current),
                    (line.x_for_index(start), line.x_for_index(end), flag)
                );
                visited += 1;
            },
        );
        assert_eq!(visited, count);
        assert_eq!(reservations, 1);
        assert_eq!(stats.borrow().linear_queries, 0);
        assert!(
            stats.borrow().glyphs <= 2 * line.runs.iter().map(|r| r.glyphs.len()).sum::<usize>()
        );
        stats.borrow_mut().linear_queries = 0;
        let mut refused = 0;
        visit_with(
            &line,
            &many,
            0,
            SCRATCH_BYTES,
            |_, _| Err(()),
            |lo, hi, current| {
                let &(start, end, flag) = many.get(refused).unwrap();
                assert_eq!(
                    (lo, hi, current),
                    (line.x_for_index(start), line.x_for_index(end), flag)
                );
                refused += 1;
            },
        );
        assert_eq!(refused, count);
        assert_eq!(stats.borrow().linear_queries, 2 * count);
    }

    #[test]
    #[ignore = "matched paragraph paint measurement; run only in an exclusive slot"]
    fn rich_highlight_matched_large_rtf_paragraph_draws() {
        let _settings = crate::settings::override_for_test(Default::default());
        let directory = tempfile::tempdir().unwrap();
        let body = "a ".repeat(8192);
        let mut source = format!("{{\\rtf1\\ansi {}\\par ", body);
        let padding = "padding ".repeat(1024);
        for _ in 0..1300 {
            source.push_str(&padding);
            source.push_str("\\par ");
        }
        source.push('}');
        assert!(source.len() > 10 * 1024 * 1024);
        let path = directory.path().join("paragraph-paint.rtf");
        std::fs::write(&path, source.as_bytes()).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let fixture_bytes = bytes.len();
        let payload = crate::rich_doc::rtf::open_bytes(bytes).unwrap();
        let crate::rich_doc::ast::Block::Paragraph { runs, .. } =
            payload.ast.blocks.first().unwrap()
        else {
            panic!("fixture must begin with a paragraph");
        };
        assert_eq!(
            runs.iter().map(|r| r.text.as_str()).collect::<String>(),
            body
        );
        let mut cx = TestAppContext::single();
        let workspace = cx.new(|cx| {
            let mut w = Workspace::new(
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                1.,
                gpui::WindowAppearance::Dark,
            );
            w.window_closed = true;
            w
        });
        let visual = cx.add_empty_window();
        let theme = Theme::for_mode(crate::theme::ThemeMode::Dark);
        let (combined, shaped_runs) =
            build_combined(Some("1. "), runs, &theme, 16., FontWeight::NORMAL);
        let (line, line_height, mask) = visual.update(|window, _| {
            (
                window
                    .text_system()
                    .shape_line(combined.into(), px(16.), &shaped_runs, None),
                window.line_height(),
                gpui::ContentMask {
                    bounds: Bounds::new(Default::default(), window.viewport_size()),
                },
            )
        });
        for count in [128, 8192] {
            let stride = body.len() / count;
            let matches: Vec<_> = (0..count)
                .map(|i| (i * stride, i * stride + 1, i == count / 2))
                .collect();
            let expected: Vec<_> = matches
                .iter()
                .map(|&(lo, hi, current)| {
                    let lo = px(10.) + line.x_for_index(lo + 3);
                    let hi = px(10.) + line.x_for_index(hi + 3);
                    ObservedQuad {
                        bounds: Bounds::new(
                            point(lo, px(20.)),
                            size(hi - lo, line_height),
                        ),
                        background: Hsla::from(rgba(if current { 0xff8c4290 } else { 0xfff39960 }))
                            .into(),
                        mask: mask.clone(),
                    }
                })
                .collect();
            let mut draw = || {
                visual.draw(
                    point(px(10.), px(20.)),
                    size(px(800.), px(200.)),
                    |_, cx| EditableInline {
                        runs: runs.clone(),
                        prefix: Some("1. ".to_owned()),
                        prefix_len: 3,
                        block_idx: 0,
                        selection: None,
                        is_cursor_block: false,
                        is_first_textual: true,
                        is_last_textual: true,
                        workspace: workspace.downgrade(),
                        focus: cx.focus_handle(),
                        theme,
                        base_size_px: 16.,
                        base_weight: FontWeight::NORMAL,
                        match_highlights: matches.clone(),
                        scroll_handle: ScrollHandle::new(),
                    },
                );
            };
            let verify = |draw: &mut dyn FnMut()| {
                let stats = std::rc::Rc::new(std::cell::RefCell::new(PaintStats::default()));
                let _guard =
                    ProbeGuard(PROBE.with(|slot| slot.borrow_mut().replace(stats.clone())));
                draw();
                assert_eq!(stats.borrow().quads, expected);
                assert_eq!(stats.borrow().matches, count);
            };
            verify(&mut draw);
            assert!(PROBE.with(|slot| slot.borrow().is_none()));
            let frames = if cfg!(debug_assertions) { 1 } else { 5 };
            let rounds = if cfg!(debug_assertions) { 1 } else { 3 };
            if !cfg!(debug_assertions) {
                for _ in 0..2 {
                    draw();
                }
            }
            let mut elapsed = Vec::new();
            for _ in 0..rounds {
                let start = std::time::Instant::now();
                for _ in 0..frames {
                    draw();
                }
                elapsed.push(start.elapsed().as_secs_f64() * 1000. / f64::from(frames));
            }
            verify(&mut draw);
            eprintln!(
                "rich-highlight matched headless paragraph: manifest={} source={} fixture_bytes={} paragraph_bytes={} matches={} prefix=3 font_px=16 origin=10,20 draw_bounds=800,200 viewport={:?} frames_per_round={} ms_per_draw={:?}",
                env!("CARGO_MANIFEST_DIR"),
                option_env!("RICH_HIGHLIGHT_SOURCE").unwrap_or("unset"),
                fixture_bytes,
                body.len(),
                count,
                mask.bounds.size,
                frames,
                elapsed
            );
        }
        cx.quit();
    }
}
