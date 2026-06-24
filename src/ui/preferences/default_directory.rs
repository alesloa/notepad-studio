// NPP `Default Directory` panel — picks where every Open / Save / Save As
// dialog (`io.rs`, `workspace/find_in_files.rs`) lands. Three modes:
// follow the active document's folder, remember whatever folder the user
// last picked, or pin a hardcoded directory. The custom-path text input +
// Browse button only render when the `Custom` radio is chosen so the panel
// stays uncluttered for the common cases.

use gpui::{
    AsyncApp, CursorStyle, IntoElement, MouseButton, ParentElement, SharedString, Styled,
    WeakEntity, div, prelude::*, rems, rgb,
};

use crate::settings::{self, DirectoryMode};
use crate::theme::Theme;

use super::widgets::{DialogTextInput, npp_radio, render_group_box};
use super::{FieldId, PreferencesDialog};

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    body: gpui::Div,
) -> gpui::AnyElement {
    let snap = settings::snapshot();
    let mode = snap.default_directory_mode;

    let radio_follow = mode_radio(
        theme,
        weak,
        "prefs-defdir-follow",
        "Follow current document",
        mode,
        DirectoryMode::FollowDocument,
    );
    let radio_remember = mode_radio(
        theme,
        weak,
        "prefs-defdir-remember",
        "Remember last used directory",
        mode,
        DirectoryMode::RememberLast,
    );
    let radio_custom = mode_radio(
        theme,
        weak,
        "prefs-defdir-custom",
        "Set your default directory:",
        mode,
        DirectoryMode::Custom,
    );

    let last_used_label = if matches!(mode, DirectoryMode::RememberLast) {
        let s = snap.last_used_directory.trim();
        let display = if s.is_empty() { "(none yet)".to_string() } else { s.to_string() };
        Some(
            div()
                .pl(rems(28. / 16.))
                .text_size(rems(11. / 16.))
                .text_color(rgb(theme.chrome_text))
                .child(SharedString::from(format!("Last picked: {display}"))),
        )
    } else {
        None
    };

    let mut group_children: Vec<gpui::AnyElement> =
        vec![radio_follow, radio_remember, radio_custom];

    if let Some(lbl) = last_used_label {
        group_children.push(lbl.into_any_element());
    }

    if matches!(mode, DirectoryMode::Custom) {
        group_children.push(custom_path_row(theme, weak));
    }

    body.child(render_group_box(
        theme,
        "Default Open/Save file Directory",
        group_children,
    ))
    .into_any_element()
}

fn mode_radio(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    id: &'static str,
    label: &'static str,
    current: DirectoryMode,
    target: DirectoryMode,
) -> gpui::AnyElement {
    let weak = weak.clone();
    npp_radio(theme, id, label, current == target, move |cx| {
        let _ = weak.update(cx, |this, cx| {
            let mut next = settings::snapshot();
            next.default_directory_mode = target;
            settings::replace_and_save(next);
            this.last_action = Some(format!("Default directory: {}", radio_status(target)));
            cx.notify();
        });
    })
}

fn radio_status(m: DirectoryMode) -> &'static str {
    match m {
        DirectoryMode::FollowDocument => "follow current document",
        DirectoryMode::RememberLast => "remember last used",
        DirectoryMode::Custom => "custom path",
    }
}

fn custom_path_row(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
) -> gpui::AnyElement {
    let input = DialogTextInput {
        dialog: weak.clone(),
        field_id: FieldId::DefaultDirectoryCustom,
    };
    let input_box = div()
        .w(rems(360. / 16.))
        .h(rems(24. / 16.))
        .bg(rgb(theme.bg))
        .border_1()
        .border_color(rgb(theme.border))
        .px(rems(6. / 16.))
        .child(input);

    let weak_browse = weak.clone();
    let browse_btn = div()
        .id("prefs-defdir-browse")
        .ml(rems(8. / 16.))
        .px(rems(10. / 16.))
        .py(rems(2. / 16.))
        .bg(rgb(theme.button_bg))
        .border_1()
        .border_color(rgb(theme.border))
        .text_size(rems(11. / 16.))
        .text_color(rgb(theme.text))
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.bg(rgb(theme.button_hover_bg)))
        .active(|s| s.bg(rgb(theme.button_pressed_bg)))
        .child("…")
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx: &mut gpui::App| {
            let weak = weak_browse.clone();
            cx.spawn(async move |cx: &mut AsyncApp| {
                let Some(handle) = rfd::AsyncFileDialog::new()
                    .set_title("Default Directory")
                    .pick_folder()
                    .await
                else {
                    return;
                };
                let path = handle.path().to_path_buf();
                let path_str = path.display().to_string();
                let _ = cx.update(|cx| {
                    let _ = weak.update(cx, |this, cx| {
                        let f = this.fields.get_mut(FieldId::DefaultDirectoryCustom);
                        f.text = path_str.clone();
                        f.cursor = f.text.len();
                        f.anchor = f.cursor;
                        let mut next = settings::snapshot();
                        next.default_directory_custom = path_str;
                        settings::replace_and_save(next);
                        this.last_action = Some("Default directory updated".to_string());
                        cx.notify();
                    });
                });
            })
            .detach();
        });

    div()
        .pl(rems(28. / 16.))
        .flex()
        .flex_row()
        .items_center()
        .gap(rems(6. / 16.))
        .child(input_box)
        .child(browse_btn)
        .into_any_element()
}
