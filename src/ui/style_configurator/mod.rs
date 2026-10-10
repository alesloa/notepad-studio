// Style Configurator — Notepad++-style theme/syntax editor.
//
// Settings ▸ Style Configurator… opens a separate top-level window that
// lets the user pick a theme, a language, and a per-language style entry,
// then edit colour / font / bitmask attributes on that style. Save & Close
// writes the theme XML back to disk; Cancel discards in-memory edits.
//
// Lifecycle mirrors `shortcut_mapper.rs`: a single window-handle cached at
// module scope, recreated on each open. Re-opening focuses the existing
// window instead of stacking another.
//
// Phase 2 scope: dialog opens with three bordered placeholder regions
// (header / body-left / body-right) and a footer with Save & Close
// (disabled — `dirty` is always false in this phase) plus Cancel (closes
// the window). Theme combo, language listbox, style listbox, colour /
// font controls, and persistence land in Phase 3+.

mod colour_picker;
mod dirty_prompt;
mod font_picker;
mod footer;
mod header;
pub(crate) mod hsv_surface;
mod layout;
mod left_column;
mod right_column;
mod state;
mod user_ext_input;

use state::DialogState;

use std::cell::RefCell;

use gpui::{
    App, Bounds, Context, CursorStyle, EntityInputHandler, FocusHandle, Focusable, IntoElement,
    KeyDownEvent, ParentElement, Pixels, Render, Styled, TitlebarOptions, UTF16Selection, Window,
    WindowHandle, WindowKind, WindowOptions, div, point, prelude::*, px, rems, rgb, size,
};
use std::ops::Range;

use crate::theme::{Theme, ThemeMode};
use crate::workspace::Workspace;

thread_local! {
    /// Cached window handle so a second open call activates the existing
    /// window instead of stacking another. Cleared via `observe_release`
    /// when the window closes.
    static OPEN_WINDOW: RefCell<Option<WindowHandle<StyleConfiguratorDialog>>> =
        const { RefCell::new(None) };
}

/// Root entity for the Style Configurator window. Owns its working
/// `dirty` flag, the discovered theme list + currently-loaded document
/// (`state`), and the focus handles for any text inputs the dialog holds.
pub(crate) struct StyleConfiguratorDialog {
    focus: FocusHandle,
    /// Theme mode — read from the active editor window if there is one,
    /// otherwise dark (the app default). Cached at construction so render
    /// doesn't have to chase down a Workspace handle every frame.
    theme_mode: ThemeMode,
    /// In-memory theme + cascade selection. Phase 3 mutates this on every
    /// theme/lang/style click; Phase 5 reads `state.dirty` for Save&Close.
    pub(in crate::ui::style_configurator) state: DialogState,
    /// Whether the header's theme combo dropdown panel is open.
    pub(in crate::ui::style_configurator) theme_dropdown_open: bool,
    /// Whether the right-column font name dropdown is open.
    pub(in crate::ui::style_configurator) font_dropdown_open: bool,
    /// Whether the right-column font size dropdown is open.
    pub(in crate::ui::style_configurator) font_size_dropdown_open: bool,
    /// Active colour picker target (FG or BG of the current style) — `None`
    /// when no picker is showing. The popover renders three picking
    /// surfaces — HSV gradient + hue strip (`hsv_surface.rs`), quick-pick
    /// swatch grid, hex input — all writing to `picker_buffer`.
    pub(in crate::ui::style_configurator) picker_target: Option<colour_picker::PickerTarget>,
    /// In-progress hex string the picker is editing. Lives on the dialog so
    /// the value survives re-renders without leaking into `DialogState`
    /// (which is the persisted document model).
    pub(in crate::ui::style_configurator) picker_buffer: String,
    /// Cursor inside `picker_buffer` (byte offset). Edits clamp to bounds.
    pub(in crate::ui::style_configurator) picker_cursor: usize,
    /// Selection anchor inside `picker_buffer` (byte offset). Equals
    /// `picker_cursor` when no selection is active. Drives selection
    /// rendering and Ctrl+A behavior.
    pub(in crate::ui::style_configurator) picker_anchor: usize,
    /// True between a mouse-down inside the hex input and the matching
    /// mouse-up. While set, the window-global `MouseMoveEvent` handler
    /// extends `picker_cursor` to the cursor's x position so the user
    /// can click-and-drag a selection.
    pub(in crate::ui::style_configurator) picker_drag_active: bool,
    /// HSV decomposition of `picker_buffer`. The buffer (hex string) stays
    /// the source of truth — these are derived on every buffer change so the
    /// SV surface / hue strip thumbs paint at the right spots. Kept on the
    /// dialog (not recomputed every render) so the user's hue choice
    /// survives a drag through grey (s=0) where hue is mathematically
    /// undefined: `sync_from_buffer` only overwrites hue when s > 0.
    pub(in crate::ui::style_configurator) picker_hue: f32,
    pub(in crate::ui::style_configurator) picker_sat: f32,
    pub(in crate::ui::style_configurator) picker_val: f32,
    /// Which HSV surface (SV gradient or hue strip) is being dragged. The
    /// window-global mouse-move handler in `hsv_surface` uses this to know
    /// which channel(s) to update on each move event.
    pub(in crate::ui::style_configurator) picker_hsv_drag: hsv_surface::HsvDrag,
    /// Snapshot of the active style's colour at the moment the picker
    /// was opened (`None` for unset, `Some(rgb)` for a previously-set
    /// value). Cancel restores the active style's colour to this so the
    /// in-progress live preview from swatch picks doesn't survive a
    /// dismissal. Cleared by `close_colour_picker`.
    pub(in crate::ui::style_configurator) picker_seed: Option<u32>,
    /// Focus handle for the User-ext text input. Anchored via
    /// `.track_focus` per the GPUI focus-handle caveat. The input itself
    /// (`user_ext_input::UserExtInput`) routes typed chars / paste / IME
    /// through this dialog's `EntityInputHandler` impl — the impl
    /// dispatches by focus to either `picker_buffer` (when the hex input
    /// is focused) or `user_ext_buffer` (when this one is).
    pub(in crate::ui::style_configurator) user_ext_focus: FocusHandle,
    /// Live editable buffer for the User-ext field. Synced from the active
    /// lexer's `lexer_user_ext` whenever the language or theme changes
    /// (`sync_user_ext_from_state`). Every keystroke / paste / drag-edit
    /// writes the new value back via `state::set_current_user_ext`.
    pub(in crate::ui::style_configurator) user_ext_buffer: String,
    /// Cursor inside `user_ext_buffer` (byte offset). Edits clamp to bounds.
    pub(in crate::ui::style_configurator) user_ext_cursor: usize,
    /// Selection anchor inside `user_ext_buffer` (byte offset). Equals
    /// `user_ext_cursor` when no selection is active.
    pub(in crate::ui::style_configurator) user_ext_anchor: usize,
    /// True between mouse-down inside the user-ext input and the matching
    /// mouse-up. Same drag-tracking pattern as `picker_drag_active`.
    pub(in crate::ui::style_configurator) user_ext_drag_active: bool,
    /// Focus handle for the colour-picker hex text input — required even
    /// for the simple non-Element variant so when the picker grows a real
    /// `handle_input` Element later, the dispatch-tree node is already in
    /// place per the GPUI focus-handle caveat.
    pub(in crate::ui::style_configurator) hex_input_focus: FocusHandle,
    /// Cached system font family list. Populated lazily on first paint of
    /// the font dropdown so opening the dialog stays cheap when the user
    /// never opens the font picker.
    pub(in crate::ui::style_configurator) font_names: Vec<String>,
    /// Latched flag: `true` between a Save / Cancel button press and the
    /// window actually being torn down. `Window::remove_window()` is async
    /// (it just sets a flag and lets the event loop unwind), which means
    /// the dialog keeps painting and accepting clicks for several frames
    /// — a frustrated user clicking Save & Close repeatedly would re-fire
    /// the save handler each time. The footer's button handlers no-op
    /// when this flag is set so a single press is all that ever lands.
    pub(in crate::ui::style_configurator) closing: bool,
}

impl StyleConfiguratorDialog {
    fn new(theme_mode: ThemeMode, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let user_ext_focus = cx.focus_handle();
        let hex_input_focus = cx.focus_handle();
        let mut this = Self {
            focus,
            theme_mode,
            state: DialogState::new(),
            theme_dropdown_open: false,
            font_dropdown_open: false,
            font_size_dropdown_open: false,
            picker_target: None,
            picker_buffer: String::new(),
            picker_cursor: 0,
            picker_anchor: 0,
            picker_drag_active: false,
            picker_hue: 0.0,
            picker_sat: 0.0,
            picker_val: 0.0,
            picker_hsv_drag: hsv_surface::HsvDrag::None,
            picker_seed: None,
            user_ext_focus,
            user_ext_buffer: String::new(),
            user_ext_cursor: 0,
            user_ext_anchor: 0,
            user_ext_drag_active: false,
            hex_input_focus,
            font_names: Vec::new(),
            closing: false,
        };
        this.sync_user_ext_from_state();
        this
    }

    /// Re-seed `user_ext_buffer` from the active lexer's stored value.
    /// Called from dialog construction and from every code path that
    /// changes the active lexer or swaps the document (lang switch,
    /// theme switch, restore-from-snapshot). Resets cursor / anchor to
    /// end-of-buffer and clears any in-progress drag so the user can't
    /// extend a selection that started under the old document.
    pub(in crate::ui::style_configurator) fn sync_user_ext_from_state(&mut self) {
        let value = self
            .state
            .current_default_ext()
            .map(str::to_string)
            .unwrap_or_default();
        self.user_ext_cursor = value.len();
        self.user_ext_anchor = value.len();
        self.user_ext_buffer = value;
        self.user_ext_drag_active = false;
    }

    fn theme(&self) -> Theme {
        Theme::for_mode(self.theme_mode)
    }

    /// Save & Close is enabled iff the in-memory document differs from the
    /// snapshot taken on load. Phase 3 always reports `false`; Phase 4
    /// flips this when a right-column edit lands.
    pub(in crate::ui::style_configurator) fn dirty(&self) -> bool {
        self.state.dirty
    }

    /// Push the current in-memory document into the active-theme preview
    /// override and ask every editor window to repaint. Call this from any
    /// handler that mutates `state.document` (theme switch, colour edit,
    /// font edit, override toggle) so the editor reflects the pending
    /// state live, with no save round trip. Cancel / window-close clears
    /// the preview; the editor then falls back to the on-disk file.
    pub(in crate::ui::style_configurator) fn refresh_preview(
        &self,
        cx: &mut Context<Self>,
    ) {
        crate::style_model::active_theme::set_preview(Some(self.state.document.clone()));
        notify_all_workspaces(cx);
        cx.notify();
    }

    /// Drop the picker popover state without touching the active style.
    /// Picking a swatch already wrote the colour onto the style and
    /// pushed it into the live preview — dismissing the picker (via
    /// click-outside, theme switch, or another dropdown opening) keeps
    /// the user's pick committed. NPP behaves the same way: closing the
    /// picker without an explicit Cancel commits whatever you last
    /// chose. The Cancel button does its own explicit revert before
    /// calling this, using `picker_seed` to write the original colour
    /// back onto the style.
    pub(in crate::ui::style_configurator) fn close_colour_picker(&mut self) {
        self.picker_target = None;
        self.picker_buffer.clear();
        self.picker_cursor = 0;
        self.picker_anchor = 0;
        self.picker_drag_active = false;
        self.picker_hue = 0.0;
        self.picker_sat = 0.0;
        self.picker_val = 0.0;
        self.picker_hsv_drag = hsv_surface::HsvDrag::None;
        self.picker_seed = None;
    }

    /// Revert the active style's colour to the value captured when the
    /// picker opened, then close the picker. Wired to the picker's
    /// Cancel button only — every other dismissal path (scrim, theme
    /// switch, dropdown open) keeps the user's last pick.
    pub(in crate::ui::style_configurator) fn cancel_colour_picker(&mut self) {
        if let Some(target) = self.picker_target {
            let seed = self.picker_seed;
            if let Some(style) = self.state.current_style_mut() {
                match target {
                    colour_picker::PickerTarget::Foreground => style.fg_color = seed,
                    colour_picker::PickerTarget::Background => style.bg_color = seed,
                }
            }
        }
        self.close_colour_picker();
    }

    /// Write the active theme's XML to disk + persist `Settings.theme_path`.
    /// Returns `Ok(saved_path)` on success, `Err(message)` on failure. The
    /// dialog stays open on failure so the user can retry.
    pub(in crate::ui::style_configurator) fn save_active_theme(
        &mut self,
    ) -> Result<std::path::PathBuf, String> {
        use crate::style_model::paths;
        use crate::style_model::xml_save;

        let target = state::save_target(&self.state)
            .ok_or_else(|| "active theme has no writable location".to_string())?;
        if let Err(err) = paths::ensure_user_dirs() {
            return Err(format!("failed to create user data directory: {err}"));
        }
        if let Some(parent) = target.parent() {
            if let Err(err) = std::fs::create_dir_all(parent) {
                return Err(format!("failed to create {parent:?}: {err}"));
            }
        }
        let xml = xml_save::write(&self.state.document);
        if let Err(err) = std::fs::write(&target, xml) {
            return Err(format!("write {target:?} failed: {err}"));
        }

        // Snapshot becomes the post-save state; future Cancels revert here.
        self.state.take_snapshot();
        self.state.last_save_error = None;
        // Refresh the theme list so a freshly-created user override now
        // appears as a UserFile entry on next dropdown open.
        self.state.refresh_themes();

        // Persist the chosen theme path + global-override flags so the next
        // launch reopens onto the same selection with the same overrides.
        // `global_override` lives in Settings (not theme XML) — it's a
        // user-level pref per UI spec §10.
        let mut next_settings = crate::settings::snapshot();
        next_settings.theme_path = Some(target.clone());
        next_settings.global_override = self.state.global_override;
        crate::settings::replace_and_save(next_settings);
        // Drop the cached parsed document so the editor's next render
        // re-reads the freshly-written XML and repaints with the new
        // colours immediately.
        crate::style_model::active_theme::invalidate();

        Ok(target)
    }
}

impl hsv_surface::HsvHost for StyleConfiguratorDialog {
    fn picker_hue(&self) -> f32 { self.picker_hue }
    fn picker_sat(&self) -> f32 { self.picker_sat }
    fn picker_val(&self) -> f32 { self.picker_val }
    fn picker_hsv_drag(&self) -> hsv_surface::HsvDrag { self.picker_hsv_drag }
    fn set_picker_hue(&mut self, h: f32) { self.picker_hue = h; }
    fn set_picker_sat(&mut self, s: f32) { self.picker_sat = s; }
    fn set_picker_val(&mut self, v: f32) { self.picker_val = v; }
    fn set_picker_hsv_drag(&mut self, drag: hsv_surface::HsvDrag) { self.picker_hsv_drag = drag; }
    fn picker_hex_text(&self) -> &str { &self.picker_buffer }
    fn set_picker_hex_text(&mut self, text: String) {
        self.picker_cursor = text.len();
        self.picker_anchor = text.len();
        self.picker_buffer = text;
    }
    fn on_hsv_picker_buffer_changed(&mut self, cx: &mut Context<Self>) {
        // Route through the existing colour-picker buffer-changed path so
        // the live preview + sync logic stays in one place. The function
        // already skips HSV resync while a drag is active (it checks
        // `picker_hsv_drag` itself), so calling it from the HSV handler
        // path is safe — the drag flag is set by the surface before
        // `set_buffer_from_hsv` runs.
        colour_picker::on_buffer_changed(self, cx);
    }
}

impl Focusable for StyleConfiguratorDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Open the Style Configurator window. If a previous instance is still on
/// screen, activate it; otherwise spawn a fresh window.
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
        // Stale handle (window already destroyed) — clear and recreate.
        OPEN_WINDOW.with(|cell| *cell.borrow_mut() = None);
    }

    let theme_mode = current_theme_mode(cx);
    // NPP's WordStyleDlg resource is 500x246 dialog units, which renders
    // ~760x380 px at the standard MS Sans Serif metric (per the UI spec).
    // Notepad Studio ships fixed-size like NPP — no resize.
    let bounds = crate::app::window::modal_bounds_centered_on_workspace(
        cx,
        size(px(760.), px(380.)),
    );
    let result = cx.open_window(
        WindowOptions {
            window_bounds: Some(bounds),
            kind: WindowKind::Floating,
            is_resizable: false,
            titlebar: Some(TitlebarOptions {
                title: Some("Style Configurator".into()),
                appears_transparent: false,
                traffic_light_position: Some(point(px(10.), px(10.))),
            }),
            ..Default::default()
        },
        |_window, cx| cx.new(|cx| StyleConfiguratorDialog::new(theme_mode, cx)),
    );

    let handle = match result {
        Ok(h) => h,
        Err(err) => {
            eprintln!("failed to open Style Configurator: {err}");
            return;
        }
    };

    OPEN_WINDOW.with(|cell| *cell.borrow_mut() = Some(handle));
    if let Ok(entity) = handle.entity(cx) {
        cx.observe_release(&entity, move |_, cx| {
            OPEN_WINDOW.with(|cell| *cell.borrow_mut() = None);
            // Drop the live-preview override so the editor stops mirroring
            // the (now-defunct) dialog's in-memory document. Falling back
            // to `Settings.theme_path` returns the editor to whichever
            // theme is persisted — i.e. the pre-open theme on Cancel /
            // close, or the freshly-saved theme after Save & Close.
            crate::style_model::active_theme::set_preview(None);
            crate::style_model::active_theme::invalidate();
            notify_all_workspaces(cx);
        })
        .detach();
    }
    let _ = handle.update(cx, |this, window, cx| {
        window.focus(&this.focus, cx);
        cx.notify();
    });
}

/// Close the Style Configurator window. Used by the footer's Cancel
/// button. The release observer wired in `open_or_focus` clears the
/// cached handle.
pub(super) fn close_window(cx: &mut App) {
    let handle = OPEN_WINDOW.with(|cell| cell.borrow().clone());
    match handle {
        Some(handle) => {
            let result = handle.update(cx, |_, window, _| window.remove_window());
            if let Err(err) = result {
                eprintln!("style-cfg: close_window: remove_window failed: {err:?}");
            }
        }
        None => {
            eprintln!("style-cfg: close_window: OPEN_WINDOW handle was None");
        }
    }
}

/// Tell every open `Workspace` window to repaint. The Style Configurator
/// runs in its own top-level window, so saving a theme XML doesn't
/// automatically trigger a paint cycle on the editor windows behind it —
/// without this nudge the user sees stale colours until the next time
/// they focus the editor or type. Mirrors `preferences::set_slot_color`.
pub(super) fn notify_all_workspaces(cx: &mut App) {
    for handle in cx.windows() {
        if let Some(ws) = handle.downcast::<Workspace>() {
            let _ = ws.update(cx, |_w, _window, cx| cx.notify());
        }
    }
}

/// Probe the active editor window for its theme mode so the dialog
/// matches. Falls back to dark when no Workspace window is around.
fn current_theme_mode(cx: &mut App) -> ThemeMode {
    let handle = cx
        .active_window()
        .and_then(|h| h.downcast::<Workspace>())
        .or_else(|| {
            cx.windows()
                .into_iter()
                .find_map(|h| h.downcast::<Workspace>())
        });
    let Some(handle) = handle else {
        return ThemeMode::Dark;
    };
    handle
        .read(cx)
        .map(|w| w.theme_mode)
        .unwrap_or(ThemeMode::Dark)
}

impl Render for StyleConfiguratorDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme();
        let entity = cx.entity().clone();
        let dirty = self.dirty();
        let last_error = self.state.last_save_error.clone();
        let prompt_visible = self.state.prompt_visible;
        let inactive_opacity = if window.is_window_active() { 1.0 } else { 0.7 };

        let root = div()
            .key_context("StyleConfigurator")
            .track_focus(&self.focus)
            .cursor(CursorStyle::Arrow)
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(theme.bg))
            .text_color(rgb(theme.text))
            .font_family(crate::fonts::mono())
            .text_size(rems(13. / 16.))
            .line_height(rems(20. / 16.))
            .p(rems(8. / 16.))
            .gap(rems(8. / 16.))
            .opacity(inactive_opacity)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                this.handle_picker_key_down(ev, window, cx);
                this.handle_user_ext_key_down(ev, window, cx);
            }));

        let header_row = header::render(&theme, self, &entity);
        let body = layout::body(&theme, self, &entity);
        let footer_row = footer::render(&theme, dirty, last_error.as_deref(), &entity, cx);
        let theme_dropdown = header::theme_dropdown_overlay(&theme, self, &entity);
        // Hoist every popover to the root so each gets a full-window
        // dismiss scrim. Order matters — popovers paint AFTER the body
        // so they land on top, and the theme_dropdown sits last (most
        // recently opened wins z-order). The colour picker is the most
        // common interaction, then font dropdowns.
        let colour_picker_overlay = colour_picker::overlay(&theme, self, &entity);
        let font_dropdown_overlay = font_picker::font_dropdown_overlay(&theme, self, &entity);
        let font_size_dropdown_overlay = font_picker::size_dropdown_overlay(&theme, self, &entity);

        let mut root = root.child(header_row).child(body).child(footer_row);
        if prompt_visible {
            root = root.child(dirty_prompt::render(&theme, &entity));
        }
        // Popovers stack in deliberate order — colour picker, then font
        // dropdowns, then theme dropdown last. Paint order is z-order,
        // so the theme dropdown ends up on top, which is the intent —
        // it's the broadest-impact action and should be visually
        // primary. Each popover ships with its own full-window dismiss
        // scrim so click-outside closes the popover (without falling
        // through to whatever underlying control would have been hit).
        if let Some(panel) = colour_picker_overlay {
            root = root.child(panel);
        }
        if let Some(panel) = font_dropdown_overlay {
            root = root.child(panel);
        }
        if let Some(panel) = font_size_dropdown_overlay {
            root = root.child(panel);
        }
        if let Some(panel) = theme_dropdown {
            root = root.child(panel);
        }
        root
    }
}

// IME / OS text input plumbing for the dialog's two custom text inputs:
// the colour-picker hex field (`colour_picker::HexInput`) and the User-ext
// row (`user_ext_input::UserExtInput`). Both register against this dialog
// via `window.handle_input(handle, ElementInputHandler::new(bounds, dialog))`
// in their `paint` methods. GPUI routes IME to the focused input only,
// but the trait method body is shared — every reader/writer below
// dispatches by focus to decide which buffer to operate on.
impl EntityInputHandler for StyleConfiguratorDialog {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let buffer = if self.user_ext_focus.is_focused(window) {
            &self.user_ext_buffer
        } else {
            &self.picker_buffer
        };
        Some(crate::text::utf16::slice_str_chars_utf16(
            buffer,
            range_utf16,
            actual_range,
        ))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if self.user_ext_focus.is_focused(window) {
            Some(crate::text::utf16::input_selection_range_utf16(
                &self.user_ext_buffer,
                self.user_ext_cursor,
                self.user_ext_anchor,
            ))
        } else {
            Some(crate::text::utf16::input_selection_range_utf16(
                &self.picker_buffer,
                self.picker_cursor,
                self.picker_anchor,
            ))
        }
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.user_ext_focus.is_focused(window) {
            // User-ext allows any printable char. NPP stores extension
            // lists as space-separated tokens (e.g. "hpp hxx h++"), so
            // letters, digits, dot, plus, dash, and whitespace are all
            // valid. Strip control characters (tab → drop, newline → drop)
            // — Tab is dialog focus navigation, Enter is reserved for the
            // dialog's Save & Close shortcut.
            let filtered: String = new_text
                .chars()
                .filter(|c| !c.is_control())
                .collect();
            if filtered.is_empty() && !new_text.is_empty() {
                return;
            }
            let cursor = self.user_ext_cursor;
            let anchor = self.user_ext_anchor;
            crate::text::utf16::replace_input_utf16(
                &mut self.user_ext_buffer,
                &mut self.user_ext_cursor,
                &mut self.user_ext_anchor,
                cursor,
                anchor,
                range_utf16,
                &filtered,
            );
            // Hard cap at 256 chars — way past anything NPP ships (e.g.
            // the C lexer's default is ~40 chars). Stops a paste-bomb
            // from blowing up the field.
            if self.user_ext_buffer.len() > 256 {
                self.user_ext_buffer.truncate(256);
                self.user_ext_cursor = self.user_ext_cursor.min(256);
                self.user_ext_anchor = self.user_ext_anchor.min(256);
            }
            self.on_user_ext_buffer_changed(cx);
            return;
        }
        // Filter to hex characters only (uppercase). Anything else is
        // dropped — same policy as the previous on_key_down handler.
        let filtered: String = new_text
            .chars()
            .filter(|c| c.is_ascii_hexdigit())
            .map(|c| c.to_ascii_uppercase())
            .collect();
        if filtered.is_empty() && !new_text.is_empty() {
            return;
        }
        let cursor = self.picker_cursor;
        let anchor = self.picker_anchor;
        crate::text::utf16::replace_input_utf16(
            &mut self.picker_buffer,
            &mut self.picker_cursor,
            &mut self.picker_anchor,
            cursor,
            anchor,
            range_utf16,
            &filtered,
        );
        // Hard cap at 6 hex chars. If a paste-of-larger-string lands,
        // truncate from the end and clamp the cursor.
        if self.picker_buffer.len() > 6 {
            self.picker_buffer.truncate(6);
            self.picker_cursor = self.picker_cursor.min(6);
            self.picker_anchor = self.picker_anchor.min(6);
        }
        colour_picker::on_buffer_changed(self, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range_utf16: Option<Range<usize>>,
        _new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl StyleConfiguratorDialog {
    /// Handle navigation / clipboard / commit keys for the colour-picker
    /// hex input. Wired via `on_key_down` on the dialog root only when
    /// the picker is open and the hex-input focus handle is focused.
    /// Character input itself flows through `EntityInputHandler` (IME
    /// path), not this method.
    pub(in crate::ui::style_configurator) fn handle_picker_key_down(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.hex_input_focus.is_focused(window) || self.picker_target.is_none() {
            return;
        }
        let mods = &ev.keystroke.modifiers;
        let key = ev.keystroke.key.as_str();
        let cmd_or_ctrl = mods.platform || mods.control;
        let shift = mods.shift;

        if key == "enter" && !cmd_or_ctrl && !shift {
            colour_picker::commit(self);
            self.refresh_preview(cx);
            cx.notify();
            return;
        }
        if key == "escape" && !cmd_or_ctrl && !shift {
            // Escape is a cancel-like gesture — revert any in-progress
            // swatch pick before tearing down the picker, matching the
            // Cancel button.
            self.cancel_colour_picker();
            self.refresh_preview(cx);
            cx.notify();
            return;
        }

        let buf_len = self.picker_buffer.len();
        let mut mutated = false;
        match key {
            "left" => {
                let new = self.picker_cursor.saturating_sub(1);
                self.picker_cursor = new;
                if !shift {
                    self.picker_anchor = new;
                }
                mutated = true;
            }
            "right" => {
                let new = (self.picker_cursor + 1).min(buf_len);
                self.picker_cursor = new;
                if !shift {
                    self.picker_anchor = new;
                }
                mutated = true;
            }
            "home" => {
                self.picker_cursor = 0;
                if !shift {
                    self.picker_anchor = 0;
                }
                mutated = true;
            }
            "end" => {
                self.picker_cursor = buf_len;
                if !shift {
                    self.picker_anchor = buf_len;
                }
                mutated = true;
            }
            "backspace" => {
                let lo = self.picker_cursor.min(self.picker_anchor);
                let hi = self.picker_cursor.max(self.picker_anchor);
                if hi > lo {
                    self.picker_buffer.replace_range(lo..hi, "");
                    self.picker_cursor = lo;
                    self.picker_anchor = lo;
                    mutated = true;
                } else if self.picker_cursor > 0 {
                    let prev = self.picker_cursor - 1;
                    self.picker_buffer.replace_range(prev..self.picker_cursor, "");
                    self.picker_cursor = prev;
                    self.picker_anchor = prev;
                    mutated = true;
                }
            }
            "delete" => {
                let lo = self.picker_cursor.min(self.picker_anchor);
                let hi = self.picker_cursor.max(self.picker_anchor);
                if hi > lo {
                    self.picker_buffer.replace_range(lo..hi, "");
                    self.picker_cursor = lo;
                    self.picker_anchor = lo;
                    mutated = true;
                } else if self.picker_cursor < buf_len {
                    let nxt = self.picker_cursor + 1;
                    self.picker_buffer.replace_range(self.picker_cursor..nxt, "");
                    mutated = true;
                }
            }
            "a" if cmd_or_ctrl => {
                self.picker_anchor = 0;
                self.picker_cursor = buf_len;
                mutated = true;
            }
            "c" if cmd_or_ctrl => {
                let lo = self.picker_cursor.min(self.picker_anchor);
                let hi = self.picker_cursor.max(self.picker_anchor);
                if hi > lo {
                    let sel = self.picker_buffer[lo..hi].to_string();
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(sel));
                }
                return;
            }
            "x" if cmd_or_ctrl => {
                let lo = self.picker_cursor.min(self.picker_anchor);
                let hi = self.picker_cursor.max(self.picker_anchor);
                if hi > lo {
                    let sel = self.picker_buffer[lo..hi].to_string();
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(sel));
                    self.picker_buffer.replace_range(lo..hi, "");
                    self.picker_cursor = lo;
                    self.picker_anchor = lo;
                    mutated = true;
                }
            }
            _ => return,
        }
        if mutated {
            colour_picker::on_buffer_changed(self, cx);
        }
    }

    /// Push `user_ext_buffer` onto the active lexer's `lexer_user_ext`
    /// and re-render. Called from every keystroke / paste / drag-edit on
    /// the user-ext input. `set_current_user_ext` marks the document
    /// dirty so Save & Close lights up.
    pub(in crate::ui::style_configurator) fn on_user_ext_buffer_changed(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let value = self.user_ext_buffer.clone();
        self.state.set_current_user_ext(value);
        cx.notify();
    }

    /// Navigation / clipboard keys for the user-ext input. Mirror of
    /// `handle_picker_key_down` minus the Enter-commit + Escape-cancel
    /// paths — the user-ext field is a free-form text input on the
    /// always-visible left column, not a popover, so it has no
    /// commit/cancel gestures of its own. Character input itself flows
    /// through `EntityInputHandler::replace_text_in_range` (IME path).
    pub(in crate::ui::style_configurator) fn handle_user_ext_key_down(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.user_ext_focus.is_focused(window) {
            return;
        }
        let mods = &ev.keystroke.modifiers;
        let key = ev.keystroke.key.as_str();
        let cmd_or_ctrl = mods.platform || mods.control;
        let shift = mods.shift;

        let buf_len = self.user_ext_buffer.len();
        let mut mutated = false;
        let mut text_changed = false;
        match key {
            "left" => {
                let new = self.user_ext_cursor.saturating_sub(1);
                self.user_ext_cursor = new;
                if !shift {
                    self.user_ext_anchor = new;
                }
                mutated = true;
            }
            "right" => {
                let new = (self.user_ext_cursor + 1).min(buf_len);
                self.user_ext_cursor = new;
                if !shift {
                    self.user_ext_anchor = new;
                }
                mutated = true;
            }
            "home" => {
                self.user_ext_cursor = 0;
                if !shift {
                    self.user_ext_anchor = 0;
                }
                mutated = true;
            }
            "end" => {
                self.user_ext_cursor = buf_len;
                if !shift {
                    self.user_ext_anchor = buf_len;
                }
                mutated = true;
            }
            "backspace" => {
                let lo = self.user_ext_cursor.min(self.user_ext_anchor);
                let hi = self.user_ext_cursor.max(self.user_ext_anchor);
                if hi > lo {
                    self.user_ext_buffer.replace_range(lo..hi, "");
                    self.user_ext_cursor = lo;
                    self.user_ext_anchor = lo;
                    mutated = true;
                    text_changed = true;
                } else if self.user_ext_cursor > 0 {
                    let prev = self.user_ext_cursor - 1;
                    self.user_ext_buffer
                        .replace_range(prev..self.user_ext_cursor, "");
                    self.user_ext_cursor = prev;
                    self.user_ext_anchor = prev;
                    mutated = true;
                    text_changed = true;
                }
            }
            "delete" => {
                let lo = self.user_ext_cursor.min(self.user_ext_anchor);
                let hi = self.user_ext_cursor.max(self.user_ext_anchor);
                if hi > lo {
                    self.user_ext_buffer.replace_range(lo..hi, "");
                    self.user_ext_cursor = lo;
                    self.user_ext_anchor = lo;
                    mutated = true;
                    text_changed = true;
                } else if self.user_ext_cursor < buf_len {
                    let nxt = self.user_ext_cursor + 1;
                    self.user_ext_buffer
                        .replace_range(self.user_ext_cursor..nxt, "");
                    mutated = true;
                    text_changed = true;
                }
            }
            "a" if cmd_or_ctrl => {
                self.user_ext_anchor = 0;
                self.user_ext_cursor = buf_len;
                mutated = true;
            }
            "c" if cmd_or_ctrl => {
                let lo = self.user_ext_cursor.min(self.user_ext_anchor);
                let hi = self.user_ext_cursor.max(self.user_ext_anchor);
                if hi > lo {
                    let sel = self.user_ext_buffer[lo..hi].to_string();
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(sel));
                }
                return;
            }
            "x" if cmd_or_ctrl => {
                let lo = self.user_ext_cursor.min(self.user_ext_anchor);
                let hi = self.user_ext_cursor.max(self.user_ext_anchor);
                if hi > lo {
                    let sel = self.user_ext_buffer[lo..hi].to_string();
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(sel));
                    self.user_ext_buffer.replace_range(lo..hi, "");
                    self.user_ext_cursor = lo;
                    self.user_ext_anchor = lo;
                    mutated = true;
                    text_changed = true;
                }
            }
            _ => return,
        }
        if text_changed {
            self.on_user_ext_buffer_changed(cx);
        } else if mutated {
            cx.notify();
        }
    }
}
