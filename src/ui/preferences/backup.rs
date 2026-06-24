// NPP `Preferences ▸ Backup` panel — three groups in NPP order
// (preference.rc:341-394, preferenceDlg.cpp:5060-5246):
//
//   ┌── Session snapshot and periodic backup ──┐
//   │ [✓] Enable session snapshot and …        │
//   │     Backup interval (ms): [_____] [Reset]│
//   └──────────────────────────────────────────┘
//   ┌── Backup on save ─────────────────────────┐
//   │ ( ) None  ( ) Simple backup  ( ) Verbose  │
//   └──────────────────────────────────────────┘
//   ┌── Custom Backup Directory ────────────────┐
//   │ [✓] Use custom backup directory           │
//   │     [_______________________ path ___] […]│
//   └──────────────────────────────────────────┘
//
// Field plumbing (FieldId / FieldState / focus / commit_all / reset_field /
// cycle_focus) is wired in `mod.rs`. This file owns layout + click handlers.

use gpui::{
    AsyncApp, CursorStyle, IntoElement, MouseButton, ParentElement, SharedString, Styled,
    WeakEntity, div, prelude::*, rems, rgb,
};

use crate::settings::{self, BackupOnSaveMode};
use crate::theme::Theme;

use super::widgets::{
    DialogTextInput, npp_checkbox, npp_radio, render_field, render_field_disabled,
    render_group_box,
};
use super::{FieldId, PreferencesDialog};

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    body: gpui::Div,
) -> gpui::AnyElement {
    let snap = settings::snapshot();

    body.child(snapshot_group(theme, weak, &snap))
        .child(on_save_group(theme, weak, &snap))
        .child(custom_dir_group(theme, weak, &snap))
        .into_any_element()
}

fn snapshot_group(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    snap: &settings::Settings,
) -> gpui::AnyElement {
    let weak_master = weak.clone();
    let master = npp_checkbox(
        theme,
        "prefs-backup-snapshot-enabled",
        "Enable session snapshot and periodic backup",
        snap.backup_periodic_enabled,
        move |checked, cx| {
            let _ = weak_master.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.backup_periodic_enabled = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "Periodic snapshot enabled".to_string()
                } else {
                    "Periodic snapshot disabled".to_string()
                });
                cx.notify();
            });
        },
    );

    let interval_field = if snap.backup_periodic_enabled {
        render_field(
            theme,
            weak,
            FieldId::BackupPeriodicIntervalMs,
            "Backup interval (ms):",
            rems(80. / 16.),
        )
    } else {
        render_field_disabled(
            theme,
            FieldId::BackupPeriodicIntervalMs,
            "Backup interval (ms):",
            &snap.backup_periodic_interval_ms.to_string(),
            rems(80. / 16.),
        )
    };

    render_group_box(
        theme,
        "Session snapshot and periodic backup",
        vec![master, interval_field],
    )
    .into_any_element()
}

fn on_save_group(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    snap: &settings::Settings,
) -> gpui::AnyElement {
    let mode = snap.backup_on_save_mode;
    let row = div()
        .flex()
        .flex_row()
        .gap(rems(20. / 16.))
        .child(mode_radio(
            theme,
            weak,
            "prefs-backup-onsave-none",
            "None",
            mode,
            BackupOnSaveMode::None,
        ))
        .child(mode_radio(
            theme,
            weak,
            "prefs-backup-onsave-simple",
            "Simple backup",
            mode,
            BackupOnSaveMode::Simple,
        ))
        .child(mode_radio(
            theme,
            weak,
            "prefs-backup-onsave-verbose",
            "Verbose backup",
            mode,
            BackupOnSaveMode::Verbose,
        ));

    render_group_box(theme, "Backup on save", vec![row.into_any_element()])
        .into_any_element()
}

fn mode_radio(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    id: &'static str,
    label: &'static str,
    current: BackupOnSaveMode,
    target: BackupOnSaveMode,
) -> gpui::AnyElement {
    let weak = weak.clone();
    npp_radio(theme, id, label, current == target, move |cx| {
        let _ = weak.update(cx, |this, cx| {
            let mut next = settings::snapshot();
            next.backup_on_save_mode = target;
            settings::replace_and_save(next);
            this.last_action = Some(format!("Backup on save: {}", on_save_label(target)));
            cx.notify();
        });
    })
}

fn on_save_label(m: BackupOnSaveMode) -> &'static str {
    match m {
        BackupOnSaveMode::None => "off",
        BackupOnSaveMode::Simple => "simple",
        BackupOnSaveMode::Verbose => "verbose",
    }
}

fn custom_dir_group(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    snap: &settings::Settings,
) -> gpui::AnyElement {
    let weak_toggle = weak.clone();
    let toggle = npp_checkbox(
        theme,
        "prefs-backup-use-custom-dir",
        "Use custom backup directory",
        snap.backup_use_custom_dir,
        move |checked, cx| {
            let _ = weak_toggle.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.backup_use_custom_dir = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "Custom backup directory enabled".to_string()
                } else {
                    "Custom backup directory disabled".to_string()
                });
                cx.notify();
            });
        },
    );

    let mut group_children: Vec<gpui::AnyElement> = vec![toggle];

    if snap.backup_use_custom_dir {
        group_children.push(custom_path_row(theme, weak));
    }

    render_group_box(theme, "Custom Backup Directory", group_children).into_any_element()
}

fn custom_path_row(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
) -> gpui::AnyElement {
    let input = DialogTextInput {
        dialog: weak.clone(),
        field_id: FieldId::BackupCustomDir,
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
        .id("prefs-backup-custom-dir-browse")
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
        .child(SharedString::from("…"))
        .on_mouse_down(MouseButton::Left, move |_e, _w, cx: &mut gpui::App| {
            let weak = weak_browse.clone();
            cx.spawn(async move |cx: &mut AsyncApp| {
                let Some(handle) = rfd::AsyncFileDialog::new()
                    .set_title("Custom Backup Directory")
                    .pick_folder()
                    .await
                else {
                    return;
                };
                let path = handle.path().to_path_buf();
                let path_str = path.display().to_string();
                let _ = cx.update(|cx| {
                    let _ = weak.update(cx, |this, cx| {
                        let f = this.fields.get_mut(FieldId::BackupCustomDir);
                        f.text = path_str.clone();
                        f.cursor = f.text.len();
                        f.anchor = f.cursor;
                        let mut next = settings::snapshot();
                        next.backup_custom_dir = path_str;
                        settings::replace_and_save(next);
                        this.last_action = Some("Custom backup directory updated".to_string());
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
