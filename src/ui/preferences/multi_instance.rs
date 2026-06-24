// NPP `Multi-Instance & Date` panel. Two groups:
//   1. Multi-instance settings — drives `single_instance::acquire_or_raise`.
//      Changes don't take effect until the next launch (the lock is read once
//      at startup); the asterisk + footnote match NPP's UX exactly.
//   2. Customize insert Date Time — strftime template for `Edit ▸ Insert ▸
//      Date/Time Customized`.

use gpui::{IntoElement, ParentElement, SharedString, Styled, WeakEntity, div, rems, rgb};

use crate::settings::{self, MultiInstanceMode};
use crate::theme::Theme;

use super::widgets::{npp_radio, render_field, render_group_box};
use super::{FieldId, PreferencesDialog};

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    body: gpui::Div,
) -> gpui::AnyElement {
    let snap = settings::snapshot();
    let mode = snap.multi_instance_mode;

    let multi_group = render_group_box(
        theme,
        "Multi-instance settings *",
        vec![
            mode_radio(
                theme,
                weak,
                "prefs-multi-mono",
                "Default (mono-instance)",
                mode,
                MultiInstanceMode::MonoInstance,
            ),
            mode_radio(
                theme,
                weak,
                "prefs-multi-always",
                "Always in multi-instance mode",
                mode,
                MultiInstanceMode::AlwaysMulti,
            ),
            mode_radio(
                theme,
                weak,
                "prefs-multi-session",
                "Open session in a new instance (session handoff partial)",
                mode,
                MultiInstanceMode::NewInstanceForSession,
            ),
        ],
    );

    let restart_note = div()
        .text_size(rems(11. / 16.))
        .text_color(rgb(theme.chrome_text))
        .child(SharedString::from(
            "* Restart required for changes to take effect.",
        ));

    let datetime_group = render_group_box(
        theme,
        "Customize insert Date Time",
        vec![render_field(
            theme,
            weak,
            FieldId::DateTimeFormat,
            "Custom format (strftime):",
            rems(220. / 16.),
        )],
    );

    let datetime_hint = div()
        .text_color(rgb(theme.chrome_text))
        .text_size(rems(11. / 16.))
        .child(SharedString::from(
            "e.g. %Y-%m-%d %H:%M:%S → 2026-05-08 13:45:22",
        ));

    body.child(multi_group)
        .child(restart_note)
        .child(datetime_group)
        .child(datetime_hint)
        .into_any_element()
}

fn mode_radio(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    id: &'static str,
    label: &'static str,
    current: MultiInstanceMode,
    target: MultiInstanceMode,
) -> gpui::AnyElement {
    let weak = weak.clone();
    npp_radio(theme, id, label, current == target, move |cx| {
        let _ = weak.update(cx, |this, cx| {
            let mut next = settings::snapshot();
            next.multi_instance_mode = target;
            settings::replace_and_save(next);
            this.last_action = Some(format!(
                "Multi-instance: {} (restart to apply)",
                radio_status(target)
            ));
            cx.notify();
        });
    })
}

fn radio_status(m: MultiInstanceMode) -> &'static str {
    match m {
        MultiInstanceMode::MonoInstance => "mono-instance",
        MultiInstanceMode::AlwaysMulti => "always multi",
        MultiInstanceMode::NewInstanceForSession => "new instance for session",
    }
}
