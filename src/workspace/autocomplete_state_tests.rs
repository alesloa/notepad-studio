// Headless gpui handler tests; these do not verify native popup placement.
use gpui::{AppContext, Context, EntityInputHandler, FocusHandle, InteractiveElement, IntoElement, ParentElement, Render, TestAppContext, VisualContext, Window, WindowAppearance};
use super::Workspace;
use crate::{actions::*, tab::{InputFocus, Tab}};

const SAMPLE: &str = "anchor android andromeda\nan";

fn with_workspace(text: &str, test: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>)) {
    let mut cx = TestAppContext::single();
    let entity = new_workspace(text, &mut cx);
    {
        let cx = cx.add_empty_window();
        cx.update_window_entity(&entity, |w, window, cx| {
            window.focus(&w.focus_handle, cx);
            w.autocomplete_word(&EditAutocompleteWord, window, cx);
            assert!(w.autocomplete.is_some(), "synthetic fixture opens completion");
            test(w, window, cx);
            w.window_closed = true; // Never run synthetic recovery jobs against user state.
        });
    }
    cx.quit();
}

fn new_workspace(text: &str, cx: &mut TestAppContext) -> gpui::Entity<Workspace> {
    let text = text.to_string();
    cx.new(move |cx| {
        let mut w = Workspace::new(
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            cx.focus_handle(), cx.focus_handle(), cx.focus_handle(),
            1., WindowAppearance::Dark,
        );
        let mut tab = Tab::empty();
        tab.buffer.rope = ropey::Rope::from_str(&text);
        tab.selection.collapse_to(text.len());
        w.tabs = vec![tab];
        w.active = 0;
        w
    })
}

fn candidates(w: &Workspace) -> Vec<&str> {
    w.autocomplete.as_ref().expect("completion visible").candidates.iter().map(String::as_str).collect()
}

#[test]
fn typing_filters_and_dismisses_unmatched_completion() {
    with_workspace(SAMPLE, |w, window, cx| {
        w.autocomplete_nav(2, cx); // Preserve the selected word as rows disappear.
        w.replace_text_in_range(None, "d", window, cx);
        assert_eq!(candidates(w), ["android", "andromeda"]);
        assert_eq!(w.autocomplete.as_ref().unwrap().selected, 1);
        w.replace_text_in_range(None, "z", window, cx);
        assert!(w.autocomplete.is_none(), "no stale label after an incompatible typed character");
        assert!(w.buffer().rope.to_string().ends_with("andz"));
    });
}

#[test]
fn backspace_refilters_the_opened_candidate_pool() {
    with_workspace(SAMPLE, |w, window, cx| {
        w.replace_text_in_range(None, "d", window, cx);
        w.backspace(&EditBackspace, window, cx);
        assert_eq!(candidates(w), ["anchor", "android", "andromeda"]);
        w.replace_text_in_range(None, " ", window, cx);
        assert!(w.autocomplete.is_none(), "separator cancels completion");
    });
}

#[test]
fn committed_ime_text_filters_and_marked_text_cancels_completion() {
    with_workspace(SAMPLE, |w, window, cx| {
        w.replace_text_in_range(None, "drom", window, cx);
        assert_eq!(candidates(w), ["andromeda"]);
        w.replace_and_mark_text_in_range(None, "e", None, window, cx);
        assert!(w.autocomplete.is_none(), "marked composition must not show completion");
    });
}

#[test]
fn caret_selection_tab_and_dialog_changes_dismiss_completion() {
    for action in 0..8 {
        with_workspace(SAMPLE, |w, window, cx| {
            match action {
                0 => w.move_left(&MoveLeft, window, cx),
                1 => w.select_left(&SelectLeft, window, cx),
                2 => { w.tabs.push(Tab::empty()); w.set_active_tab(1, cx); }
                3 => w.focus_input(InputFocus::Replacement, window, cx),
                4 => w.select_up(&SelectUp, window, cx),
                5 => w.page_down(&EditPageDown, window, cx),
                6 => w.select_page_up(&SelectPageUp, window, cx),
                _ => w.move_doc_start(&MoveDocStart, window, cx),
            }
            assert!(w.autocomplete.is_none(), "context change {action}");
        });
    }
}

#[test]
fn stale_completion_does_not_steal_dialog_navigation() {
    with_workspace(SAMPLE, |w, window, cx| {
        let caret = w.selection().head;
        window.focus(&w.find_focus, cx); // Focus can change without focus_input.
        w.move_down(&MoveDown, window, cx);
        assert!(w.autocomplete.is_none());
        assert_eq!(w.selection().head, caret, "dialog arrows must not move the editor caret");
    });
}

#[test]
fn accepting_completion_rejects_stale_or_foreign_choices() {
    for invalid in 0..7 {
        with_workspace(SAMPLE, |w, window, cx| {
            let choice = if invalid == 0 { "unlisted" } else { "android" };
            match invalid {
                1 => { // Simulate a changed same-start prefix before a stale click arrives.
                    let at = w.buffer().rope.len_chars();
                    w.active_tab_mut().buffer.rope.insert(at, "zz");
                    let end = w.buffer().rope.len_bytes();
                    w.active_tab_mut().selection.collapse_to(end);
                }
                2 => w.active_tab_mut().selection.anchor = 0,
                3 => w.active_tab_mut().buffer.read_only = true,
                4 => window.focus(&w.find_focus, cx),
                5 => { // Same caret and start, different current prefix.
                    let end = w.buffer().rope.len_chars();
                    w.active_tab_mut().buffer.rope.remove(end - 2..end);
                    w.active_tab_mut().buffer.rope.insert(end - 2, "zz");
                }
                6 => { w.tabs.push(Tab::empty()); w.active = 1; }
                _ => {}
            }
            let before = w.buffer().rope.to_string();
            assert!(!w.accept_autocomplete_with(choice, window, cx), "invalid acceptance {invalid}");
            assert_eq!(w.buffer().rope.to_string(), before);
            assert!(w.autocomplete.is_none());
        });
    }
}

#[test]
fn manual_empty_prefix_accepts_a_current_candidate() {
    with_workspace("android anchor\n", |w, window, cx| {
        assert!(w.accept_autocomplete_with("android", window, cx));
        assert!(w.buffer().rope.to_string().ends_with("\nandroid"));
    });
}

#[test]
fn deleting_the_opened_prefix_cancels_completion() {
    with_workspace(SAMPLE, |w, window, cx| {
        w.backspace(&EditBackspace, window, cx);
        w.backspace(&EditBackspace, window, cx);
        assert!(w.autocomplete.is_none());
    });
}


#[test]
fn oversized_identifier_is_not_a_completion_candidate() {
    let text = "a".repeat(16 * 1024 * 1024);
    let rope = ropey::Rope::from_str(&text);
    let started = std::time::Instant::now();
    let found = super::collect_word_candidates(&rope, "a", false, rope.len_bytes());
    eprintln!("16MiB identifier scan: {:?}; candidates={}, maxbytes={}", started.elapsed(), found.len(), found.iter().map(String::len).max().unwrap_or(0));
    assert!(found.iter().all(|word| word.len() <= 256), "oversized words must not be copied into the popup");
}

#[test]
fn candidate_collection_stops_at_its_scan_budget() {
    let text = format!("{}android", " ".repeat(128 * 1024 + 1));
    let rope = ropey::Rope::from_str(&text);
    assert!(super::collect_word_candidates(&rope, "an", false, 0).is_empty());
}

struct CompletionFocusFixture { editor: FocusHandle, dialog: FocusHandle }

impl Render for CompletionFocusFixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div().child(gpui::div().track_focus(&self.editor))
            .child(gpui::div().track_focus(&self.dialog))
    }
}

#[test]
fn blur_subscription_dismisses_without_an_editor_command() {
    let mut cx = TestAppContext::single();
    let entity = new_workspace(SAMPLE, &mut cx);
    {
        // Blur callbacks use the painted dispatch tree, so give both handles
        // real focus nodes instead of using the other tests' Empty root.
        let (_, cx) = cx.add_window_view(|_, cx| CompletionFocusFixture {
            editor: entity.read(cx).focus_handle.clone(),
            dialog: entity.read(cx).find_focus.clone(),
        });
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        cx.update_window_entity(&entity, |w, window, cx| {
            window.focus(&w.focus_handle, cx);
            w.autocomplete_word(&EditAutocompleteWord, window, cx);
            assert!(w.autocomplete_blur_subscription.is_some());
        });
        cx.update(|window, cx| { let _ = window.draw(cx); });
        cx.run_until_parked();
        cx.update_window_entity(&entity, |w, window, cx| {
            window.focus(&w.find_focus, cx);
        });
        cx.update(|window, cx| { let _ = window.draw(cx); });
        cx.run_until_parked();
        cx.update_window_entity(&entity, |w, _, _| {
            assert!(w.autocomplete.is_none(), "blur must close without another keystroke");
            assert!(w.autocomplete_blur_subscription.is_none());
            w.window_closed = true;
        });
    }
    cx.quit();
}

#[test]
fn keyboard_navigation_and_acceptance_use_filtered_rows() {
    for tab_key in [false, true] {
        with_workspace(SAMPLE, |w, window, cx| {
            let initial_caret = w.selection().head;
            w.move_down(&MoveDown, window, cx);
            assert_eq!(w.selection().head, initial_caret);
            w.replace_text_in_range(None, "d", window, cx);
            assert_eq!(w.autocomplete.as_ref().unwrap().selected, 0);
            w.move_down(&MoveDown, window, cx);
            if tab_key { w.insert_tab(&EditInsertTab, window, cx); }
            else { w.insert_newline(&EditInsertNewline, window, cx); }
            assert!(w.autocomplete.is_none());
            assert!(w.buffer().rope.to_string().ends_with("\nandromeda"));
            w.undo(&EditUndo, window, cx);
            assert!(w.buffer().rope.to_string().ends_with("\nand"));
        });
    }
}

#[test]
fn auto_trigger_counts_characters_and_handles_committed_ime() {
    assert!(!super::prefix_reaches_threshold("中", 2));
    assert!(super::prefix_reaches_threshold("中文", 2));
    assert!(super::prefix_reaches_threshold("é", 0));
    with_workspace("android andromeda\n", |w, window, cx| {
        w.close_autocomplete(cx);
        w.replace_text_in_range(None, "and", window, cx);
        assert_eq!(candidates(w), ["android", "andromeda"]);
        w.close_autocomplete(cx);
        w.replace_and_mark_text_in_range(None, "r", None, window, cx);
        assert!(w.autocomplete.is_none());
        assert!(!w.autocomplete_auto_trigger_suppressed, "composition suppression is scoped");
    });
}

#[test]
fn oversized_prefix_refuses_opening_and_stale_acceptance() {
    with_workspace(SAMPLE, |w, window, cx| {
        let mut stale = w.autocomplete.clone().unwrap();
        let text = "a".repeat(16 * 1024 * 1024);
        w.active_tab_mut().buffer.rope = ropey::Rope::from_str(&text);
        w.active_tab_mut().selection.collapse_to(text.len());
        w.close_autocomplete(cx);
        let now = std::time::Instant::now();
        w.maybe_auto_trigger_autocomplete(window, cx);
        assert!(w.autocomplete.is_none());
        w.autocomplete_word(&EditAutocompleteWord, window, cx);
        assert!(w.autocomplete.is_none());
        stale.caret = text.len();
        stale.replace_start = 0;
        w.autocomplete = Some(stale);
        assert!(!w.accept_autocomplete_with("android", window, cx));
        assert_eq!(w.buffer().rope.len_bytes(), text.len());
        eprintln!("16MiB prefix open/accept: {:?}", now.elapsed());
    });
}

#[test]
fn nearby_words_are_discovered_at_a_deep_caret() {
    let text = format!("outside {}android andromeda an", " ".repeat(256 * 1024));
    let rope = ropey::Rope::from_str(&text);
    assert_eq!(super::collect_word_candidates(&rope, "an", false, rope.len_bytes()), ["android", "andromeda"]);
    assert!(super::collect_word_candidates(&rope, "an", false, 0).is_empty());
}

#[test]
fn bounded_scan_does_not_suggest_partial_tokens_or_split_unicode() {
    // Token crosses the right boundary of a scan from the start of the file.
    let rope = ropey::Rope::from_str(&format!("{}android{}", " ".repeat(131070), " ".repeat(1024)));
    assert!(super::collect_word_candidates(&rope, "an", false, 0).is_empty());
    // The left boundary intersects a huge identifier whose suffix looks useful.
    let rope = ropey::Rope::from_str(&format!("{}android anchor", "x".repeat(128 * 1024)));
    assert_eq!(super::collect_word_candidates(&rope, "an", false, rope.len_bytes()), ["anchor"]);
    let rope = ropey::Rope::from_str(&format!("{} café cafe", "中".repeat(65536)));
    for caret in [0, 1, 2, rope.len_bytes(), usize::MAX] {
        let found = super::collect_word_candidates(&rope, "caf", false, caret);
        assert!(found.iter().all(|word| word.len() <= 256));
    }
}

#[test]
fn read_only_and_mouse_context_changes_dismiss_completion() {
    with_workspace(SAMPLE, |w, _, cx| {
        w.set_tab_read_only(0, true, cx);
        assert!(w.autocomplete.is_none());
    });
    with_workspace(SAMPLE, |w, window, cx| {
        w.on_mouse_down(&gpui::MouseDownEvent::default(), window, cx);
        assert!(w.autocomplete.is_none());
    });
}

#[test]
fn find_next_dismisses_completion_when_selecting_a_match() {
    with_workspace(SAMPLE, |w, window, cx| {
        // A newline query exercises F3 without adding persisted Find history.
        w.find_state.query = "\n".to_string();
        w.find_next(&EditFindNext, window, cx);
        let at = SAMPLE.find('\n').unwrap();
        assert_eq!(w.selection().ordered(), at..at + 1);
        assert!(w.autocomplete.is_none());
    });
}

#[test]
fn backspace_broadens_beyond_the_opening_prefix() {
    with_workspace("anchor android andromeda\nandro", |w, window, cx| {
        assert_eq!(candidates(w), ["android", "andromeda"]);
        w.backspace(&EditBackspace, window, cx);
        w.backspace(&EditBackspace, window, cx);
        w.backspace(&EditBackspace, window, cx);
        assert_eq!(candidates(w), ["anchor", "android", "andromeda"]);
        assert!(w.buffer().rope.to_string().ends_with("\nan"));
    });
}

#[test]
fn narrowing_a_capped_pool_discovers_previously_hidden_matches() {
    let words: Vec<String> = (0..70).map(|i| format!("an{i:03}word")).collect();
    let text = format!("{}\nan", words.join(" "));
    with_workspace(&text, |w, window, cx| {
        assert_eq!(candidates(w).len(), super::MAX_CANDIDATES);
        w.replace_text_in_range(None, "06", window, cx);
        let expected: Vec<&str> = words.iter().skip(60).map(String::as_str).collect();
        assert_eq!(candidates(w), expected);
        assert!(w.buffer().rope.to_string().ends_with("\nan06"));
    });
}

#[test]
fn broadening_retains_function_and_combined_sources() {
    for combined in [false, true] {
        with_workspace("return ref relic\nret", |w, window, cx| {
            w.active_tab_mut().buffer.language = Some(crate::syntax::LanguageId::Rust);
            if combined {
                w.autocomplete_function_and_word(&EditAutocompleteFuncAndWord, window, cx);
            } else {
                w.autocomplete_function(&EditAutocompleteFunc, window, cx);
            }
            assert_eq!(candidates(w), ["return"]);
            w.backspace(&EditBackspace, window, cx);
            let words = candidates(w);
            assert!(words.contains(&"ref"), "keyword source remains available");
            assert!(words.contains(&"return"));
            assert_eq!(words.contains(&"relic"), combined, "manual mode stays unchanged");
        });
    }
}
