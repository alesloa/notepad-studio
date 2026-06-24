// Character Panel — Edit ▸ Character Panel.
//
// Lightweight floating window that lists printable ASCII (32-126) and the
// Latin-1 Supplement range (160-255). Click a row to insert that character
// at the active editor's caret. Useful for inserting characters that aren't
// easy to type from the user's keyboard layout (typographic punctuation,
// extended Latin, HTML-named entities like &copy;, &nbsp;).
//
// Notepad++ source:
//   PowerEditor/src/WinControls/AnsiCharPanel/ansiCharPanel.cpp
//   PowerEditor/src/NppCommands.cpp — IDM_EDIT_CHAR_PANEL
//
// NPP renders this as a dockable side panel that the user can pin to either
// side of the main editor. Notepad Studio v1 ships a floating popup window
// for the same reason the Clipboard History panel does: the Workspace
// doesn't yet host arbitrary dockable side panels, and the floating window
// already covers the "give me a character I can't easily type" use case.

use std::cell::RefCell;

use gpui::{
    App, Context, CursorStyle, Entity, FocusHandle, Focusable, IntoElement, MouseButton,
    ParentElement, Render, SharedString, Styled, TitlebarOptions, Window, WindowHandle,
    WindowKind, WindowOptions, div, point, prelude::*, px, rems, rgb, size,
};

use crate::theme::{Theme, ThemeMode};
use crate::workspace::Workspace;

thread_local! {
    /// Cached window handle so a second open call activates the existing
    /// window instead of stacking another. Cleared via `observe_release`
    /// when the window closes.
    static OPEN_WINDOW: RefCell<Option<WindowHandle<CharacterPanel>>> =
        const { RefCell::new(None) };
}

/// Lowest value shown when the extended range is on or off. 32 = SPACE,
/// the first printable ASCII char. Control chars below it (NULL, BEL,
/// BS, TAB, LF, CR, …) are intentionally skipped — the panel is for
/// characters users want to *insert into text*, and these have keyboard
/// equivalents or aren't meaningful as standalone glyphs.
const ASCII_START: u32 = 32;
/// Highest ASCII value shown. 126 = `~`. 127 = DEL, skipped.
const ASCII_END: u32 = 126;
/// Lowest Latin-1 Supplement value shown when the extended range is on.
/// 128-159 are C1 control codes (unassigned in Latin-1) — skipped.
const LATIN1_START: u32 = 160;
/// Highest Latin-1 Supplement value shown. 255 = ÿ.
const LATIN1_END: u32 = 255;

pub(crate) struct CharacterPanel {
    focus: FocusHandle,
    theme_mode: ThemeMode,
    /// Workspace window handle the panel was opened from. Insert-on-click
    /// routes through this handle so the character lands in the editor the
    /// user was working in when they opened the panel — even if they
    /// alt-tabbed to a different workspace window in between.
    source_workspace: Option<WindowHandle<Workspace>>,
    /// When true, the list also shows Latin-1 Supplement (160-255). Off
    /// by default to keep the initial view focused on the common ASCII
    /// glyphs; the user can flip it on for accented letters, currency
    /// symbols, etc.
    show_extended: bool,
    /// Transient toast shown when an action runs ("Inserted '©' (U+00A9)",
    /// "Active tab can't accept …"). Auto-clears on the next render that
    /// fires after the user interacts again — kept simple, no timer.
    status: Option<String>,
}

impl CharacterPanel {
    fn new(
        theme_mode: ThemeMode,
        source_workspace: Option<WindowHandle<Workspace>>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus: cx.focus_handle(),
            theme_mode,
            source_workspace,
            show_extended: false,
            status: None,
        }
    }

    fn theme(&self) -> Theme {
        Theme::for_mode(self.theme_mode)
    }

    /// Insert the character with codepoint `value` into the source
    /// workspace's active editor. Updates `status`; does NOT close the
    /// panel — Notepad++ keeps its panel open across insertions so the
    /// user can insert several characters in sequence.
    fn insert_char(&mut self, value: u32, cx: &mut Context<Self>) {
        let Some(ch) = char::from_u32(value) else {
            self.status = Some(format!("Codepoint U+{value:04X} is not valid."));
            cx.notify();
            return;
        };
        let Some(handle) = self.source_workspace else {
            self.status = Some("No editor window to insert into.".into());
            cx.notify();
            return;
        };
        let inserted_text = ch.to_string();
        let inserted = handle
            .update(cx, |ws, _window, cx| {
                ws.paste_string_into_editor(&inserted_text, cx)
            })
            .unwrap_or(false);
        self.status = Some(if inserted {
            let glyph = display_glyph(value, ch);
            format!("Inserted {glyph} (U+{value:04X}, decimal {value}).")
        } else {
            "Active tab can't accept a character insertion right now.".into()
        });
        cx.notify();
    }

    fn toggle_extended(&mut self, cx: &mut Context<Self>) {
        self.show_extended = !self.show_extended;
        self.status = Some(if self.show_extended {
            "Showing ASCII + Latin-1 Supplement (160-255).".into()
        } else {
            "Showing printable ASCII only (32-126).".into()
        });
        cx.notify();
    }
}

impl Focusable for CharacterPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Open the Character Panel. If a previous instance is still on screen,
/// activate it; otherwise spawn a fresh window anchored to the active
/// workspace so insert-on-click targets the right editor.
pub(crate) fn open_or_focus(cx: &mut App) {
    let cached = OPEN_WINDOW.with(|cell| cell.borrow().clone());
    if let Some(handle) = cached {
        let result = handle.update(cx, |_, window, cx| {
            window.activate_window();
            cx.notify();
        });
        if result.is_ok() {
            return;
        }
        OPEN_WINDOW.with(|cell| *cell.borrow_mut() = None);
    }

    let theme_mode = current_theme_mode(cx);
    let source = active_workspace(cx);

    let bounds = crate::app::window::modal_bounds_centered_on_workspace(
        cx,
        size(px(520.), px(440.)),
    );
    let result = cx.open_window(
        WindowOptions {
            window_bounds: Some(bounds),
            kind: WindowKind::Floating,
            is_resizable: true,
            window_min_size: Some(size(px(380.), px(260.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Character Panel".into()),
                appears_transparent: false,
                traffic_light_position: Some(point(px(10.), px(10.))),
            }),
            ..Default::default()
        },
        |_window, cx| cx.new(|cx| CharacterPanel::new(theme_mode, source, cx)),
    );

    let handle = match result {
        Ok(h) => h,
        Err(err) => {
            eprintln!("failed to open Character Panel: {err}");
            return;
        }
    };

    OPEN_WINDOW.with(|cell| *cell.borrow_mut() = Some(handle));
    if let Ok(entity) = handle.entity(cx) {
        cx.observe_release(&entity, move |_, _| {
            OPEN_WINDOW.with(|cell| *cell.borrow_mut() = None);
        })
        .detach();
    }
    let _ = handle.update(cx, |this, window, cx| {
        window.focus(&this.focus, cx);
        cx.notify();
    });
}

fn current_theme_mode(cx: &mut App) -> ThemeMode {
    active_workspace(cx)
        .and_then(|h| h.read(cx).ok().map(|w| w.theme_mode))
        .unwrap_or(ThemeMode::Dark)
}

fn active_workspace(cx: &mut App) -> Option<WindowHandle<Workspace>> {
    cx.active_window()
        .and_then(|h| h.downcast::<Workspace>())
        .or_else(|| {
            cx.windows()
                .into_iter()
                .find_map(|h| h.downcast::<Workspace>())
        })
}

impl Render for CharacterPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme();
        let entity = cx.entity().clone();
        let entries = build_entries(self.show_extended);

        let root = div()
            .key_context("CharacterPanel")
            .track_focus(&self.focus)
            .cursor(CursorStyle::Arrow)
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(theme.bg))
            .text_color(rgb(theme.text))
            .font_family("Menlo")
            .text_size(rems(13. / 16.))
            .line_height(rems(20. / 16.));

        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .px(rems(12. / 16.))
            .h(rems(28. / 16.))
            .bg(rgb(theme.chrome))
            .border_b_1()
            .border_color(rgb(theme.border))
            .text_size(rems(11.5 / 16.))
            .text_color(rgb(theme.chrome_text))
            .child(format!(
                "Click a row to insert at caret — {} character{}",
                entries.len(),
                if entries.len() == 1 { "" } else { "s" },
            ));

        // Column headings — fixed widths matching render_row.
        let column_head = div()
            .flex()
            .flex_row()
            .items_center()
            .px(rems(10. / 16.))
            .h(rems(22. / 16.))
            .gap(rems(8. / 16.))
            .bg(rgb(theme.chrome))
            .border_b_1()
            .border_color(rgb(theme.border))
            .text_color(rgb(theme.chrome_text))
            .text_size(rems(11. / 16.))
            .child(div().w(rems(40. / 16.)).child("Dec"))
            .child(div().w(rems(48. / 16.)).child("Hex"))
            .child(div().w(rems(48. / 16.)).child("Char"))
            .child(div().flex_1().child("HTML name"));

        let list = {
            let mut list = div()
                .flex_1()
                .flex()
                .flex_col()
                .min_h(px(0.))
                .id(SharedString::from("char_panel_list"))
                .overflow_y_scroll();
            for &value in &entries {
                list = list.child(self.render_row(&theme, value, &entity));
            }
            list
        };

        let status_bar = self.render_status_bar(&theme);
        let footer = self.render_footer(&theme, &entity);

        root.child(header)
            .child(column_head)
            .child(list)
            .child(status_bar)
            .child(footer)
            .into_any_element()
    }
}

impl CharacterPanel {
    fn render_row(
        &self,
        theme: &Theme,
        value: u32,
        entity: &Entity<CharacterPanel>,
    ) -> gpui::AnyElement {
        let ch = char::from_u32(value).unwrap_or('?');
        let glyph = display_glyph(value, ch);
        let html = html_name(value).unwrap_or("");
        let hover_bg = theme.button_hover_bg;
        let entity_click = entity.clone();
        div()
            .id(SharedString::from(format!("char_panel_row::{value}")))
            .flex()
            .flex_row()
            .items_center()
            .h(rems(24. / 16.))
            .px(rems(10. / 16.))
            .gap(rems(8. / 16.))
            .bg(rgb(theme.bg))
            .text_color(rgb(theme.text))
            .text_size(rems(12. / 16.))
            .border_b_1()
            .border_color(rgb(theme.border))
            .cursor(CursorStyle::PointingHand)
            .hover(move |s| s.bg(rgb(hover_bg)))
            .on_mouse_down(MouseButton::Left, move |_e, _window, cx| {
                let click = entity_click.clone();
                cx.defer(move |cx| {
                    let _ = click.update(cx, |this, cx| this.insert_char(value, cx));
                });
            })
            .child(
                div()
                    .w(rems(40. / 16.))
                    .text_color(rgb(theme.chrome_text))
                    .child(format!("{value}")),
            )
            .child(
                div()
                    .w(rems(48. / 16.))
                    .text_color(rgb(theme.chrome_text))
                    .child(format!("0x{value:02X}")),
            )
            .child(div().w(rems(48. / 16.)).child(glyph))
            .child(div().flex_1().text_color(rgb(theme.chrome_text)).child(html))
            .into_any_element()
    }

    fn render_status_bar(&self, theme: &Theme) -> gpui::AnyElement {
        let status = self.status.clone().unwrap_or_default();
        div()
            .h(rems(20. / 16.))
            .px(rems(12. / 16.))
            .flex()
            .items_center()
            .bg(rgb(theme.chrome))
            .border_t_1()
            .border_color(rgb(theme.border))
            .text_color(rgb(theme.chrome_text))
            .text_size(rems(11. / 16.))
            .child(status)
            .into_any_element()
    }

    fn render_footer(
        &self,
        theme: &Theme,
        entity: &Entity<CharacterPanel>,
    ) -> gpui::AnyElement {
        let entity_toggle = entity.clone();
        let entity_close = entity.clone();
        let hover_bg = theme.button_hover_bg;
        let toggle_label = if self.show_extended {
            "Hide Latin-1"
        } else {
            "Show Latin-1"
        };
        div()
            .h(rems(36. / 16.))
            .px(rems(12. / 16.))
            .flex()
            .flex_row()
            .items_center()
            .justify_end()
            .gap(rems(8. / 16.))
            .bg(rgb(theme.chrome))
            .border_t_1()
            .border_color(rgb(theme.border))
            .child(
                div()
                    .id(SharedString::from("char_panel_toggle"))
                    .px(rems(10. / 16.))
                    .py(rems(4. / 16.))
                    .bg(rgb(theme.button_bg))
                    .border_1()
                    .border_color(rgb(theme.border))
                    .text_color(rgb(theme.text))
                    .text_size(rems(12. / 16.))
                    .cursor(CursorStyle::PointingHand)
                    .hover(move |s| s.bg(rgb(hover_bg)))
                    .on_mouse_down(MouseButton::Left, move |_e, _window, cx| {
                        let toggle = entity_toggle.clone();
                        cx.defer(move |cx| {
                            let _ = toggle.update(cx, |this, cx| this.toggle_extended(cx));
                        });
                    })
                    .child(toggle_label),
            )
            .child(
                div()
                    .id(SharedString::from("char_panel_close"))
                    .px(rems(10. / 16.))
                    .py(rems(4. / 16.))
                    .bg(rgb(theme.button_bg))
                    .border_1()
                    .border_color(rgb(theme.border))
                    .text_color(rgb(theme.text))
                    .text_size(rems(12. / 16.))
                    .cursor(CursorStyle::PointingHand)
                    .hover(move |s| s.bg(rgb(hover_bg)))
                    .on_mouse_down(MouseButton::Left, move |_e, _window, _cx| {
                        let _ = entity_close.clone();
                        _window.remove_window();
                    })
                    .child("Close"),
            )
            .into_any_element()
    }
}

/// Build the codepoint list for the current `show_extended` setting.
fn build_entries(show_extended: bool) -> Vec<u32> {
    let mut out: Vec<u32> = (ASCII_START..=ASCII_END).collect();
    if show_extended {
        out.extend(LATIN1_START..=LATIN1_END);
    }
    out
}

/// Render a friendly display glyph for a given codepoint. Most chars
/// just show themselves; whitespace and otherwise-invisible chars get
/// a readable label so the row isn't blank.
fn display_glyph(value: u32, ch: char) -> String {
    match value {
        32 => "(SP)".into(),
        160 => "(NBSP)".into(),
        173 => "(SHY)".into(),
        _ => ch.to_string(),
    }
}

/// HTML 4 named entity for the codepoint, if one exists. Returned values
/// include the `&` and `;` so they render as the entity the user would
/// type into HTML source. Only the standard Latin-1 named entities are
/// covered — HTML5 added `&apos;` for U+0027 and others, but `&apos;`
/// isn't recognised by older browsers/parsers, so the panel sticks to
/// the HTML 4 set.
fn html_name(value: u32) -> Option<&'static str> {
    Some(match value {
        34 => "&quot;",
        38 => "&amp;",
        60 => "&lt;",
        62 => "&gt;",
        160 => "&nbsp;",
        161 => "&iexcl;",
        162 => "&cent;",
        163 => "&pound;",
        164 => "&curren;",
        165 => "&yen;",
        166 => "&brvbar;",
        167 => "&sect;",
        168 => "&uml;",
        169 => "&copy;",
        170 => "&ordf;",
        171 => "&laquo;",
        172 => "&not;",
        173 => "&shy;",
        174 => "&reg;",
        175 => "&macr;",
        176 => "&deg;",
        177 => "&plusmn;",
        178 => "&sup2;",
        179 => "&sup3;",
        180 => "&acute;",
        181 => "&micro;",
        182 => "&para;",
        183 => "&middot;",
        184 => "&cedil;",
        185 => "&sup1;",
        186 => "&ordm;",
        187 => "&raquo;",
        188 => "&frac14;",
        189 => "&frac12;",
        190 => "&frac34;",
        191 => "&iquest;",
        192 => "&Agrave;",
        193 => "&Aacute;",
        194 => "&Acirc;",
        195 => "&Atilde;",
        196 => "&Auml;",
        197 => "&Aring;",
        198 => "&AElig;",
        199 => "&Ccedil;",
        200 => "&Egrave;",
        201 => "&Eacute;",
        202 => "&Ecirc;",
        203 => "&Euml;",
        204 => "&Igrave;",
        205 => "&Iacute;",
        206 => "&Icirc;",
        207 => "&Iuml;",
        208 => "&ETH;",
        209 => "&Ntilde;",
        210 => "&Ograve;",
        211 => "&Oacute;",
        212 => "&Ocirc;",
        213 => "&Otilde;",
        214 => "&Ouml;",
        215 => "&times;",
        216 => "&Oslash;",
        217 => "&Ugrave;",
        218 => "&Uacute;",
        219 => "&Ucirc;",
        220 => "&Uuml;",
        221 => "&Yacute;",
        222 => "&THORN;",
        223 => "&szlig;",
        224 => "&agrave;",
        225 => "&aacute;",
        226 => "&acirc;",
        227 => "&atilde;",
        228 => "&auml;",
        229 => "&aring;",
        230 => "&aelig;",
        231 => "&ccedil;",
        232 => "&egrave;",
        233 => "&eacute;",
        234 => "&ecirc;",
        235 => "&euml;",
        236 => "&igrave;",
        237 => "&iacute;",
        238 => "&icirc;",
        239 => "&iuml;",
        240 => "&eth;",
        241 => "&ntilde;",
        242 => "&ograve;",
        243 => "&oacute;",
        244 => "&ocirc;",
        245 => "&otilde;",
        246 => "&ouml;",
        247 => "&divide;",
        248 => "&oslash;",
        249 => "&ugrave;",
        250 => "&uacute;",
        251 => "&ucirc;",
        252 => "&uuml;",
        253 => "&yacute;",
        254 => "&thorn;",
        255 => "&yuml;",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_only_range_excludes_latin1() {
        let entries = build_entries(false);
        assert_eq!(entries.first(), Some(&32));
        assert_eq!(entries.last(), Some(&126));
        assert_eq!(entries.len(), (126 - 32 + 1) as usize);
        assert!(!entries.contains(&160));
    }

    #[test]
    fn extended_range_includes_latin1_supplement() {
        let entries = build_entries(true);
        assert!(entries.contains(&32));
        assert!(entries.contains(&126));
        assert!(entries.contains(&160));
        assert!(entries.contains(&255));
        // No C1 control chars (128-159).
        assert!(!entries.contains(&127));
        assert!(!entries.contains(&128));
        assert!(!entries.contains(&159));
        // Right count: 95 ASCII + 96 Latin-1.
        assert_eq!(entries.len(), 95 + 96);
    }

    #[test]
    fn display_glyph_labels_invisible_chars() {
        assert_eq!(display_glyph(32, ' '), "(SP)");
        assert_eq!(display_glyph(160, '\u{a0}'), "(NBSP)");
        assert_eq!(display_glyph(173, '\u{ad}'), "(SHY)");
    }

    #[test]
    fn display_glyph_returns_visible_glyphs() {
        assert_eq!(display_glyph(65, 'A'), "A");
        assert_eq!(display_glyph(169, '©'), "©");
        assert_eq!(display_glyph(247, '÷'), "÷");
    }

    #[test]
    fn html_name_covers_ascii_entities() {
        assert_eq!(html_name(34), Some("&quot;"));
        assert_eq!(html_name(38), Some("&amp;"));
        assert_eq!(html_name(60), Some("&lt;"));
        assert_eq!(html_name(62), Some("&gt;"));
        // No entity for plain letters / digits.
        assert_eq!(html_name(65), None);
        assert_eq!(html_name(48), None);
    }

    #[test]
    fn html_name_covers_full_latin1_supplement() {
        // Every codepoint 160..=255 should have a named entity.
        for v in 160u32..=255u32 {
            assert!(
                html_name(v).is_some(),
                "missing HTML entity for codepoint {v}"
            );
        }
    }
}
