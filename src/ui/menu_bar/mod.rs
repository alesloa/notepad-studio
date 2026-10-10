// In-window menu bar for Windows and Linux.
//
// gpui draws `App::set_menus` as a native menu bar only on macOS; on Windows
// and Linux it stores the tree (`App::get_menus`) and draws nothing. This
// view renders that same tree and drives it the way a Win32 menu bar works:
//   PowerEditor/src/Notepad_plus.rc — IDR_M30_MENU: titles, labels, `&` keys
//   PowerEditor/src/WinControls/shortcut/shortcut.cpp:294 — accelerator text
// Alt or F10 arms the bar, Alt+<key> opens a menu, the arrows walk it, Enter
// or an item's access key runs it, Esc backs out one level.

pub(crate) mod keys;
pub(crate) mod mnemonic;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    Action, AnyElement, Bounds, Context, DispatchPhase, FocusHandle, HighlightStyle, IntoElement,
    Keystroke, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, OwnedMenu, OwnedMenuItem,
    Pixels, Point, Render, ScrollHandle, StyledText, Subscription, UnderlineStyle, Window,
    WindowId, anchored, canvas, deferred, div, point, prelude::*, px, rems, rgb,
};

use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Closed,
    /// Title highlighted from the keyboard, no dropdown showing.
    Armed(usize),
    /// Dropdown of this title showing.
    Open(usize),
}

/// Which row a freshly opened dropdown highlights.
#[derive(Clone, Copy)]
enum Select {
    None,
    First,
    Last,
}

/// Element bounds from the last paint, used to place dropdowns and to
/// hit-test titles under the dropdown backdrop.
#[derive(Default)]
struct Geometry {
    titles: Vec<Bounds<Pixels>>,
    /// Keyed by (dropdown level, row index).
    rows: HashMap<(usize, usize), Bounds<Pixels>>,
}

pub(crate) struct MenuBar {
    /// Workspace focus, for resolving the chord shown beside each item.
    workspace_focus: FocusHandle,
    menus: Vec<OwnedMenu>,
    mode: Mode,
    /// Highlighted row per open dropdown. Dropdown `k + 1` is the submenu at
    /// dropdown `k`'s highlighted row.
    panels: Vec<Option<usize>>,
    scroll: Vec<ScrollHandle>,
    /// Underline access keys. Win32 shows them once the keyboard drives the
    /// menu and hides them for mouse use.
    show_keys: bool,
    geometry: Rc<RefCell<Geometry>>,
    /// Armed or open. Shared with the keystroke interceptor, which runs on
    /// every key press in the app and must not lease the entity for nothing.
    engaged: Rc<Cell<bool>>,
    /// A mouse button went down with Alt held, so the Alt release that
    /// follows is not a menu tap (Alt+drag is column selection).
    alt_mouse: Rc<Cell<bool>>,
    pub(crate) theme: Theme,
    _subscriptions: Vec<Subscription>,
}

impl MenuBar {
    pub(crate) fn new(
        workspace_focus: FocusHandle,
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let window_id: WindowId = window.window_handle().window_id();
        let engaged = Rc::new(Cell::new(false));
        let gate = engaged.clone();
        let weak = cx.weak_entity();
        // Interceptors run before key bindings, so while the menu is engaged
        // it owns the keyboard and the editor sees nothing.
        let keys = cx.intercept_keystrokes(move |event, window, cx| {
            if !gate.get() || window.window_handle().window_id() != window_id {
                return;
            }
            let Some(bar) = weak.upgrade() else {
                return;
            };
            let keystroke = event.keystroke.clone();
            if bar.update(cx, |bar, cx| bar.handle_key(&keystroke, window, cx)) {
                cx.stop_propagation();
            }
        });
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.close(cx);
            }
        });
        Self {
            workspace_focus,
            menus: cx.get_menus().unwrap_or_default(),
            mode: Mode::Closed,
            panels: Vec::new(),
            scroll: Vec::new(),
            show_keys: false,
            geometry: Rc::new(RefCell::new(Geometry::default())),
            engaged,
            alt_mouse: Rc::new(Cell::new(false)),
            theme,
            _subscriptions: vec![keys, activation],
        }
    }

    /// Alt pressed and released on its own.
    pub(crate) fn alt_tap(&mut self, cx: &mut Context<Self>) {
        if self.alt_mouse.replace(false) {
            return;
        }
        self.toggle(cx);
    }

    /// F10, or a plain Alt tap: arm the bar, or leave menu mode.
    pub(crate) fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.mode != Mode::Closed {
            self.close(cx);
            return;
        }
        self.refresh_menus(cx);
        if self.menus.is_empty() {
            return;
        }
        self.mode = Mode::Armed(0);
        self.show_keys = true;
        self.engaged.set(true);
        cx.notify();
    }

    /// Alt+<key>: open the menu whose title carries that access key.
    pub(crate) fn open_by_key(&mut self, key: char, cx: &mut Context<Self>) {
        self.refresh_menus(cx);
        if let Some(i) = self.title_for_key(mnemonic::lower(key)) {
            self.show_keys = true;
            self.open(i, Select::First, cx);
        }
    }

    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        if self.mode == Mode::Closed {
            return;
        }
        self.mode = Mode::Closed;
        self.panels.clear();
        self.scroll.clear();
        self.show_keys = false;
        self.engaged.set(false);
        cx.notify();
    }

    /// Rebuild the tree so dynamic items (recent files, macros, checkmarks)
    /// are current, then take a copy to draw from.
    fn refresh_menus(&mut self, cx: &mut Context<Self>) {
        crate::app::menus::rebuild_menus(cx);
        self.menus = cx.get_menus().unwrap_or_default();
    }

    fn title_for_key(&self, key: char) -> Option<usize> {
        self.menus
            .iter()
            .position(|m| mnemonic::access_key(&mnemonic::parse(&m.name)) == Some(key))
    }

    fn open(&mut self, title: usize, select: Select, cx: &mut Context<Self>) {
        if title >= self.menus.len() {
            return;
        }
        self.mode = Mode::Open(title);
        self.panels.clear();
        self.scroll.clear();
        let highlight = match (select, self.panel_items(title, 0)) {
            (Select::First, Some(items)) => keys::step(&selectable(items), None, true),
            (Select::Last, Some(items)) => keys::step(&selectable(items), None, false),
            _ => None,
        };
        self.push_panel(highlight);
        self.engaged.set(true);
        cx.notify();
    }

    fn push_panel(&mut self, highlight: Option<usize>) {
        let level = self.panels.len();
        self.panels.push(highlight);
        self.scroll.truncate(level);
        self.scroll.push(ScrollHandle::new());
        if let (Some(handle), Some(row)) = (self.scroll.get(level), highlight) {
            handle.scroll_to_item(row);
        }
    }

    fn pop_panel(&mut self) {
        if self.panels.len() > 1 {
            self.panels.pop();
            self.scroll.truncate(self.panels.len());
        }
    }

    /// Items of dropdown `level` under `title`, following the highlighted
    /// submenu rows down from the top.
    fn panel_items(&self, title: usize, level: usize) -> Option<&[OwnedMenuItem]> {
        let mut items: &[OwnedMenuItem] = &self.menus.get(title)?.items;
        for k in 0..level {
            let row = (*self.panels.get(k)?)?;
            match items.get(row)? {
                OwnedMenuItem::Submenu(m) => items = &m.items,
                _ => return None,
            }
        }
        Some(items)
    }

    fn set_highlight(&mut self, level: usize, row: Option<usize>, cx: &mut Context<Self>) {
        if level >= self.panels.len() {
            return;
        }
        self.panels.truncate(level + 1);
        self.scroll.truncate(level + 1);
        self.panels[level] = row;
        if let (Some(handle), Some(row)) = (self.scroll.get(level), row) {
            handle.scroll_to_item(row);
        }
        cx.notify();
    }

    fn hover_row(&mut self, level: usize, row: usize, cx: &mut Context<Self>) {
        let Mode::Open(title) = self.mode else {
            return;
        };
        let Some(item) = self.panel_items(title, level).and_then(|items| items.get(row)) else {
            return;
        };
        if !is_selectable(item) {
            return;
        }
        let opens_submenu = matches!(item, OwnedMenuItem::Submenu(m) if !m.items.is_empty());
        let already = self.panels.get(level) == Some(&Some(row))
            && self.panels.len() == level + 1 + usize::from(opens_submenu);
        if already {
            return;
        }
        self.panels.truncate(level + 1);
        self.scroll.truncate(level + 1);
        self.panels[level] = Some(row);
        if opens_submenu {
            self.push_panel(None);
        }
        cx.notify();
    }

    /// Run the row: open its submenu, or close the menu and dispatch its
    /// action to whatever had focus. `keyboard` highlights the submenu's
    /// first row, as Win32 does for Enter and Right.
    fn activate(
        &mut self,
        level: usize,
        row: usize,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        enum Hit {
            Submenu(Option<usize>),
            Action(Box<dyn Action>),
        }
        let Mode::Open(title) = self.mode else {
            return;
        };
        let hit = match self.panel_items(title, level).and_then(|items| items.get(row)) {
            Some(OwnedMenuItem::Submenu(m)) if !m.items.is_empty() => {
                let first = if keyboard {
                    keys::step(&selectable(&m.items), None, true)
                } else {
                    None
                };
                Hit::Submenu(first)
            }
            Some(OwnedMenuItem::Action { action, .. }) => Hit::Action(action.boxed_clone()),
            _ => return,
        };
        match hit {
            Hit::Submenu(first) => {
                if level >= self.panels.len() {
                    return;
                }
                self.panels.truncate(level + 1);
                self.scroll.truncate(level + 1);
                self.panels[level] = Some(row);
                self.push_panel(first);
                cx.notify();
            }
            Hit::Action(action) => {
                if !action_enabled(action.as_ref(), window, cx) {
                    return;
                }
                self.close(cx);
                window.dispatch_action(action, cx);
            }
        }
    }

    /// Keystroke while engaged. Returns whether the menu consumed it; it
    /// consumes everything but Alt+F4, which still closes the window.
    fn handle_key(&mut self, ks: &Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if ks.modifiers.alt && ks.key == "f4" {
            self.close(cx);
            return false;
        }
        match self.mode {
            Mode::Closed => return false,
            Mode::Armed(title) => self.key_armed(title, ks, cx),
            Mode::Open(title) => self.key_open(title, ks, window, cx),
        }
        true
    }

    fn key_armed(&mut self, title: usize, ks: &Keystroke, cx: &mut Context<Self>) {
        let n = self.menus.len();
        if n == 0 {
            self.close(cx);
            return;
        }
        match ks.key.as_str() {
            "escape" | "f10" => self.close(cx),
            "left" => {
                self.mode = Mode::Armed((title + n - 1) % n);
                cx.notify();
            }
            "right" => {
                self.mode = Mode::Armed((title + 1) % n);
                cx.notify();
            }
            "down" | "enter" => self.open(title, Select::First, cx),
            "up" => self.open(title, Select::Last, cx),
            _ => {
                if let Some(i) = typed_char(ks).and_then(|c| self.title_for_key(c)) {
                    self.open(i, Select::First, cx);
                }
            }
        }
    }

    fn key_open(&mut self, title: usize, ks: &Keystroke, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.menus.len().max(1);
        let level = self.panels.len().saturating_sub(1);
        let current = self.panels.get(level).copied().flatten();
        let items_selectable = self
            .panel_items(title, level)
            .map(selectable)
            .unwrap_or_default();
        match ks.key.as_str() {
            "escape" => {
                if self.panels.len() > 1 {
                    self.pop_panel();
                    cx.notify();
                } else {
                    self.mode = Mode::Armed(title);
                    self.panels.clear();
                    self.scroll.clear();
                    cx.notify();
                }
            }
            "f10" => self.close(cx),
            "down" => {
                let next = keys::step(&items_selectable, current, true);
                self.set_highlight(level, next, cx);
            }
            "up" => {
                let next = keys::step(&items_selectable, current, false);
                self.set_highlight(level, next, cx);
            }
            "home" => {
                let next = keys::step(&items_selectable, None, true);
                self.set_highlight(level, next, cx);
            }
            "end" => {
                let next = keys::step(&items_selectable, None, false);
                self.set_highlight(level, next, cx);
            }
            "left" => {
                if self.panels.len() > 1 {
                    self.pop_panel();
                    cx.notify();
                } else {
                    self.open((title + n - 1) % n, Select::First, cx);
                }
            }
            "right" => {
                let opens_submenu = current
                    .and_then(|row| self.panel_items(title, level)?.get(row))
                    .is_some_and(|item| matches!(item, OwnedMenuItem::Submenu(m) if !m.items.is_empty()));
                match current {
                    Some(row) if opens_submenu => self.activate(level, row, true, window, cx),
                    _ => self.open((title + 1) % n, Select::First, cx),
                }
            }
            "enter" => {
                if let Some(row) = current {
                    self.activate(level, row, true, window, cx);
                }
            }
            _ => {
                let Some(c) = typed_char(ks) else {
                    return;
                };
                let row_keys: Vec<Option<char>> = self
                    .panel_items(title, level)
                    .map(|items| items.iter().map(item_key).collect())
                    .unwrap_or_default();
                match keys::matching(&row_keys, c, current).as_slice() {
                    [] => {}
                    [only] => {
                        let only = *only;
                        self.set_highlight(level, Some(only), cx);
                        self.activate(level, only, true, window, cx);
                    }
                    [next, ..] => self.set_highlight(level, Some(*next), cx),
                }
            }
        }
    }

    fn title_at(&self, position: Point<Pixels>) -> Option<usize> {
        let geometry = self.geometry.try_borrow().ok()?;
        geometry.titles.iter().position(|b| b.contains(&position))
    }

    fn backdrop_down(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        match self.title_at(position) {
            Some(i) if self.mode == Mode::Open(i) => self.close(cx),
            Some(i) => self.open(i, Select::None, cx),
            None => self.close(cx),
        }
    }

    fn backdrop_move(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some(i) = self.title_at(position) {
            if self.mode != Mode::Open(i) {
                self.open(i, Select::None, cx);
            }
        }
    }

    fn click_title(&mut self, title: usize, cx: &mut Context<Self>) {
        if self.mode == Mode::Open(title) {
            self.close(cx);
            return;
        }
        if self.mode == Mode::Closed {
            self.refresh_menus(cx);
            self.show_keys = false;
        }
        self.open(title, Select::None, cx);
    }

    /// Where each open dropdown goes, from the bounds painted last frame:
    /// under its title, or beside its parent row. `None` until that anchor
    /// has been painted once.
    fn panel_origins(&self, window: &Window) -> Vec<Option<Point<Pixels>>> {
        let Mode::Open(title) = self.mode else {
            return Vec::new();
        };
        let Ok(geometry) = self.geometry.try_borrow() else {
            return Vec::new();
        };
        let lift = rems(PANEL_PAD).to_pixels(window.rem_size()) + px(1.);
        (0..self.panels.len())
            .map(|level| {
                if level == 0 {
                    return geometry
                        .titles
                        .get(title)
                        .filter(|b| b.size.width > px(0.))
                        .map(|b| b.bottom_left());
                }
                let row = (*self.panels.get(level - 1)?)?;
                geometry
                    .rows
                    .get(&(level - 1, row))
                    .map(|b| point(b.right() - px(2.), b.top() - lift))
            })
            .collect()
    }

    /// The accelerator shown beside an item: its first chord in binding
    /// order, which is the catalog's primary chord (Zoom In shows Ctrl+=,
    /// not the Ctrl+Shift+= alias), skipping the `cmd-` mirrors that only
    /// mean something on macOS.
    fn shortcut_text(&self, action: &dyn Action, window: &Window) -> Option<String> {
        let mac = cfg!(target_os = "macos");
        window
            .bindings_for_action_in(action, &self.workspace_focus)
            .iter()
            .find_map(|binding| {
                let strokes = binding.keystrokes();
                if strokes.is_empty() || (!mac && strokes.iter().any(|k| k.modifiers().platform)) {
                    return None;
                }
                let text: Vec<String> = strokes
                    .iter()
                    .map(|k| keys::chord_label(k.modifiers(), k.key()))
                    .collect();
                Some(text.join(", "))
            })
    }

    /// Window-wide mouse watchers, re-registered every paint.
    fn input_watch(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let alt_mouse = self.alt_mouse.clone();
        let engaged = self.engaged.clone();
        let weak = cx.weak_entity();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let on_down = alt_mouse.clone();
                window.on_mouse_event(move |e: &MouseDownEvent, phase, _, _| {
                    if matches!(phase, DispatchPhase::Capture) {
                        on_down.set(e.modifiers.alt);
                    }
                });
                let on_move = alt_mouse.clone();
                window.on_mouse_event(move |e: &MouseMoveEvent, _, _, _| {
                    if !e.modifiers.alt {
                        on_move.set(false);
                    }
                });
                // Any click leaves keyboard menu mode, as in Win32. An open
                // dropdown has its own backdrop and handles its clicks.
                window.on_mouse_event(move |_: &MouseDownEvent, phase, _, cx| {
                    if !matches!(phase, DispatchPhase::Capture) || !engaged.get() {
                        return;
                    }
                    if let Some(bar) = weak.upgrade() {
                        bar.update(cx, |bar, cx| {
                            if matches!(bar.mode, Mode::Armed(_)) {
                                bar.close(cx);
                            }
                        });
                    }
                });
            },
        )
        .absolute()
        .size_0()
    }

    fn title(&self, i: usize, menu: &OwnedMenu, active: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let label = mnemonic::parse(&menu.name);
        let geometry = self.geometry.clone();
        div()
            .id(("menu-title", i))
            .relative()
            .h_full()
            .flex()
            .items_center()
            .px(rems(7. / 16.))
            .when(active, |d| d.bg(rgb(theme.toolbar_hover_bg)))
            .hover(|s| s.bg(rgb(theme.toolbar_hover_bg)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _, cx| this.click_title(i, cx)),
            )
            .child(label_text(&label, self.show_keys))
            .child(
                canvas(
                    move |bounds, _, _| {
                        if let Ok(mut g) = geometry.try_borrow_mut() {
                            if let Some(slot) = g.titles.get_mut(i) {
                                *slot = bounds;
                            }
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
    }

    fn dropdowns(
        &self,
        title: usize,
        origins: &[Option<Point<Pixels>>],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let viewport = window.viewport_size();
        let margin = px(2.);
        let mut layer = div()
            .id("menu-backdrop")
            .w(viewport.width)
            .h(viewport.height)
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, cx| this.backdrop_down(e.position, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _: &MouseDownEvent, _, cx| this.close(cx)),
            )
            .on_mouse_move(
                cx.listener(|this, e: &MouseMoveEvent, _, cx| this.backdrop_move(e.position, cx)),
            );
        for level in 0..self.panels.len() {
            let Some(origin) = origins.get(level).copied().flatten() else {
                // Its anchor hasn't been painted yet; place it next frame.
                window.request_animation_frame();
                break;
            };
            let Some(items) = self.panel_items(title, level) else {
                break;
            };
            // The top dropdown stays below the bar; submenus may slide up
            // to fit, as Win32 does.
            let room = if level == 0 {
                viewport.height - origin.y - margin
            } else {
                viewport.height - margin * 2.
            };
            let max_h = room.max(px(60.));
            layer = layer.child(
                anchored()
                    .position(origin)
                    .snap_to_window_with_margin(margin)
                    .child(self.panel(level, items, max_h, window, cx)),
            );
        }
        deferred(anchored().position(point(px(0.), px(0.))).child(layer)).with_priority(10)
    }

    fn panel(
        &self,
        level: usize,
        items: &[OwnedMenuItem],
        max_h: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = self.theme;
        let highlighted = self.panels.get(level).copied().flatten();
        let mut panel = div()
            .id(("menu-panel", level))
            .occlude()
            .flex()
            .flex_col()
            .py(rems(PANEL_PAD))
            .min_w(rems(160. / 16.))
            .max_h(max_h)
            .overflow_y_scroll()
            .bg(rgb(theme.chrome))
            .border_1()
            .border_color(rgb(theme.border))
            .shadow_md();
        if let Some(handle) = self.scroll.get(level) {
            panel = panel.track_scroll(handle);
        }
        for (row, item) in items.iter().enumerate() {
            panel = panel.child(self.row(level, row, item, highlighted == Some(row), window, cx));
        }
        panel
    }

    fn row(
        &self,
        level: usize,
        row: usize,
        item: &OwnedMenuItem,
        highlighted: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let (name, checked, shortcut, enabled, submenu) = match item {
            OwnedMenuItem::Separator => {
                return div()
                    .flex_none()
                    .py(rems(3. / 16.))
                    .child(div().h(px(1.)).bg(rgb(theme.border)))
                    .into_any_element();
            }
            OwnedMenuItem::SystemMenu(_) => return div().into_any_element(),
            OwnedMenuItem::Action {
                name,
                action,
                checked,
                ..
            } => (
                name.as_str(),
                *checked,
                self.shortcut_text(action.as_ref(), window),
                action_enabled(action.as_ref(), window, cx),
                false,
            ),
            OwnedMenuItem::Submenu(m) => (m.name.as_ref(), false, None, !m.items.is_empty(), true),
        };
        let label = mnemonic::parse(name);
        let fg = if enabled { theme.text } else { theme.disabled_text };
        let dim = if enabled { theme.chrome_text } else { theme.disabled_text };
        let geometry = self.geometry.clone();
        div()
            .id(("menu-row", row))
            .relative()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .h(rems(22. / 16.))
            .text_color(rgb(fg))
            .when(highlighted, |d| d.bg(rgb(theme.toolbar_hover_bg)))
            .child(
                div()
                    .w(rems(24. / 16.))
                    .flex_none()
                    .flex()
                    .justify_center()
                    .when(checked, |d| d.child("✓")),
            )
            .child(div().flex_1().whitespace_nowrap().child(label_text(&label, self.show_keys)))
            .child(
                div()
                    .flex_none()
                    .pl(rems(28. / 16.))
                    .whitespace_nowrap()
                    .text_color(rgb(dim))
                    .when_some(shortcut, |d, s| d.child(s)),
            )
            .child(
                div()
                    .w(rems(20. / 16.))
                    .flex_none()
                    .flex()
                    .justify_center()
                    .when(submenu, |d| d.child("›")),
            )
            .child(
                canvas(
                    move |bounds, _, _| {
                        if let Ok(mut g) = geometry.try_borrow_mut() {
                            g.rows.insert((level, row), bounds);
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| {
                this.hover_row(level, row, cx)
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseUpEvent, window, cx| {
                    this.activate(level, row, false, window, cx)
                }),
            )
            .into_any_element()
    }
}

/// Vertical padding inside a dropdown, in rems.
const PANEL_PAD: f32 = 3. / 16.;

impl Render for MenuBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let origins = self.panel_origins(window);
        if let Ok(mut g) = self.geometry.try_borrow_mut() {
            // Refilled by this frame's prepaint.
            g.rows.clear();
            g.titles.resize(self.menus.len(), Bounds::default());
        }
        let active = match self.mode {
            Mode::Armed(t) | Mode::Open(t) => Some(t),
            Mode::Closed => None,
        };
        let mut bar = div()
            .id("menu-bar")
            .relative()
            .flex()
            .flex_row()
            .flex_none()
            .w_full()
            .h(rems(20. / 16.))
            .bg(rgb(theme.chrome))
            .text_color(rgb(theme.text))
            .font_family(crate::fonts::ui())
            .text_size(rems(12. / 16.))
            .line_height(rems(20. / 16.))
            .child(self.input_watch(cx));
        for (i, menu) in self.menus.iter().enumerate() {
            bar = bar.child(self.title(i, menu, active == Some(i), cx));
        }
        if let Mode::Open(title) = self.mode {
            bar = bar.child(self.dropdowns(title, &origins, window, cx));
        }
        bar
    }
}

fn label_text(label: &mnemonic::Label, show_keys: bool) -> StyledText {
    let text = StyledText::new(label.text.clone());
    match (&label.underline, show_keys) {
        (Some(range), true) => text.with_highlights([(
            range.clone(),
            HighlightStyle {
                underline: Some(UnderlineStyle {
                    thickness: px(1.),
                    color: None,
                    wavy: false,
                }),
                ..Default::default()
            },
        )]),
        _ => text,
    }
}

fn is_selectable(item: &OwnedMenuItem) -> bool {
    matches!(item, OwnedMenuItem::Action { .. } | OwnedMenuItem::Submenu(_))
}

fn selectable(items: &[OwnedMenuItem]) -> Vec<bool> {
    items.iter().map(is_selectable).collect()
}

fn item_key(item: &OwnedMenuItem) -> Option<char> {
    let name = match item {
        OwnedMenuItem::Action { name, .. } => name.as_str(),
        OwnedMenuItem::Submenu(m) => m.name.as_ref(),
        _ => return None,
    };
    mnemonic::access_key(&mnemonic::parse(name))
}

/// Greyed unless something on the focus path or an app-level handler takes
/// the action, as Win32 greys commands with no handler.
fn action_enabled(action: &dyn Action, window: &mut Window, cx: &mut gpui::App) -> bool {
    window.is_action_available(action, cx) || cx.is_action_available(action)
}

/// The character a key press types, for access-key matching. Chords with
/// Ctrl or the platform key never match, as in Win32.
fn typed_char(ks: &Keystroke) -> Option<char> {
    if ks.modifiers.control || ks.modifiers.platform {
        return None;
    }
    if ks.key == "/" && ks.modifiers.shift && ks.key_char.is_none() {
        return Some('?');
    }
    let source = ks
        .key_char
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(ks.key.as_str());
    let mut chars = source.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    Some(mnemonic::lower(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ks(source: &str) -> Keystroke {
        Keystroke::parse(source).unwrap()
    }

    #[test]
    fn typed_char_reads_plain_and_alt_letters() {
        assert_eq!(typed_char(&ks("f")), Some('f'));
        assert_eq!(typed_char(&ks("alt-f")), Some('f'));
        assert_eq!(typed_char(&ks("shift-f")), Some('f'));
        assert_eq!(typed_char(&ks("shift-/")), Some('?'));
    }

    #[test]
    fn typed_char_ignores_ctrl_and_named_keys() {
        assert_eq!(typed_char(&ks("ctrl-f")), None);
        assert_eq!(typed_char(&ks("cmd-f")), None);
        assert_eq!(typed_char(&ks("enter")), None);
        assert_eq!(typed_char(&ks("f10")), None);
    }
}
