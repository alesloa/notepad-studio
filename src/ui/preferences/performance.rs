// Performance panel — large-file restriction.
//
// NPP `Preferences ▸ Performance` (preferenceDlg.cpp:6760-6920) gates the
// styling pipeline on a per-file size check. Mirrors:
//   IDC_CHECK_PERFORMANCE_ENABLE  → large_file_restriction_enabled
//   IDC_EDIT_PERFORMANCE_FILESIZE → large_file_threshold_bytes (in MB)
//   IDC_CHECK_PERFORMANCE_DEFERRED_HILITE / _ALLOWBRACEMATCH /
//   _ALLOWAUTOCOMPLETION / _ALLOWSMARTHILITE / _ALLOWURLHILITECLICK
//     → collapsed in Notepad Studio to one knob: large_file_disable_syntax,
//       since Notepad Studio only has tree-sitter today (no separate brace
//       match / smart-highlight / autocomplete / URL pipelines).

use gpui::{IntoElement, ParentElement, WeakEntity, rems};

use crate::settings;
use crate::theme::Theme;

use super::widgets::{
    npp_checkbox, npp_checkbox_disabled, render_field, render_field_disabled, render_group_box,
};
use super::{FieldId, PreferencesDialog};

/// Floor on the threshold input in MB. NPP doesn't enforce an explicit
/// minimum (treats 0 as "reset to default" in `EN_KILLFOCUS`,
/// preferenceDlg.cpp:6863-6876). 1 MB is the smallest value that doesn't
/// flag essentially every editor buffer as "large".
pub(super) const THRESHOLD_MIN_MB: u64 = 1;

/// Ceiling on the threshold input in MB. NPP caps the field at
/// `(INT32_MAX - 1MB) / 1MB` (preferenceDlg.cpp:6845) so the
/// `bufferSizeRequested` math in `FileManager::loadFileData` stays
/// within `int32_t`. Same value here so settings.toml round-trips
/// across NPP-flavoured editors (= 2046 MB).
pub(super) const THRESHOLD_MAX_MB: u64 = ((i32::MAX as u64) - 1024 * 1024) / 1024 / 1024;

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    body: gpui::Div,
) -> gpui::AnyElement {
    let snap = settings::snapshot();
    let restriction_on = snap.large_file_restriction_enabled;

    let weak_for_master = weak.clone();
    let master = npp_checkbox(
        theme,
        "prefs-perf-restriction-enabled",
        "Enable Large File Restriction",
        restriction_on,
        move |checked, cx| {
            let _ = weak_for_master.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.large_file_restriction_enabled = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "Large file restriction enabled".to_string()
                } else {
                    "Large file restriction disabled".to_string()
                });
                cx.notify();
            });
        },
    );

    let threshold_field = if restriction_on {
        render_field(
            theme,
            weak,
            FieldId::LargeFileThresholdMb,
            "Define large file size (MB):",
            rems(80. / 16.),
        )
    } else {
        render_field_disabled(
            theme,
            FieldId::LargeFileThresholdMb,
            "Define large file size (MB):",
            &(snap.large_file_threshold_bytes / (1024 * 1024)).to_string(),
            rems(80. / 16.),
        )
    };

    let sub = if restriction_on {
        let weak_for_sub = weak.clone();
        npp_checkbox(
            theme,
            "prefs-perf-disable-syntax",
            "Disable syntax highlighting for large files",
            snap.large_file_disable_syntax,
            move |checked, cx| {
                let _ = weak_for_sub.update(cx, |this, cx| {
                    let mut next = settings::snapshot();
                    next.large_file_disable_syntax = checked;
                    settings::replace_and_save(next);
                    this.last_action = Some(if checked {
                        "Disable syntax for large files: on".to_string()
                    } else {
                        "Disable syntax for large files: off".to_string()
                    });
                    cx.notify();
                });
            },
        )
    } else {
        npp_checkbox_disabled(
            theme,
            "Disable syntax highlighting for large files",
            snap.large_file_disable_syntax,
        )
    };

    body.child(render_group_box(
        theme,
        "Large File Restriction",
        vec![master, threshold_field, sub],
    ))
    .into_any_element()
}
