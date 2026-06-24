// Editing 2 panel — NPP `Editing 2` parity.
//
// Live controls (everything except multi-editing):
//   * `whitespace_visible` — persisted default; live View ▸ Show Symbol toggle
//     drives the per-window flag (see workspace::edits::toggle_show_whitespace).
//   * `eol_visible` + `eol_display_mode` — driven by editor_element's
//     prepaint_eol_glyphs (PlainText vs RoundedRect outline behind the text).
//   * `npc_visible` + `npc_display_mode` + `npc_include_cc_uni_eol` — driven by
//     editor_element's prepaint_npc_glyphs (C0+DEL via byte walk;
//     C1+Unicode-EOL via char_indices when the "Apply…" toggle is on).
//   * `prevent_c0_input` — gates Workspace::replace_text_in_range so pasted
//     C0 control bytes are stripped before insertion.
//
// The one engine-deferred control is `multi_editing_enabled` — Notepad Studio's
// selection model is single-cursor; toggle persists for when a multi-cursor
// engine ships.

use gpui::{IntoElement, ParentElement, SharedString, Styled, WeakEntity, div, rems, rgb};

use crate::settings::{self, EolDisplayMode, NpcDisplayMode};
use crate::theme::Theme;

use super::PreferencesDialog;
use super::widgets::{
    npp_checkbox, npp_checkbox_disabled, npp_radio, npp_radio_disabled, render_group_box,
};

pub(super) fn render(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    body: gpui::Div,
) -> gpui::AnyElement {
    let snap = settings::snapshot();

    // --- Multi-editing -------------------------------------------------
    let weak_multi = weak.clone();
    let multi_toggle = npp_checkbox(
        theme,
        "prefs-editing2-multi-editing",
        "Enable Multi-editing (Ctrl+Mouse click/selection)",
        snap.multi_editing_enabled,
        move |checked, cx| {
            let _ = weak_multi.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.multi_editing_enabled = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "Multi-editing enabled (engine pending)".to_string()
                } else {
                    "Multi-editing disabled".to_string()
                });
                cx.notify();
            });
        },
    );
    let multi_caveat = caveat(
        theme,
        "Engine deferred — single-cursor model active until multi-cursor lands.",
    );
    let multi_group = render_group_box(theme, "Multi-editing", vec![multi_toggle, multi_caveat]);

    // --- Display Whitespace and Tab ------------------------------------
    let weak_ws = weak.clone();
    let ws_toggle = npp_checkbox(
        theme,
        "prefs-editing2-whitespace",
        "Show white space and tab",
        snap.whitespace_visible,
        move |checked, cx| {
            let _ = weak_ws.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.whitespace_visible = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "Whitespace glyphs: shown (paint pending)".to_string()
                } else {
                    "Whitespace glyphs: hidden".to_string()
                });
                cx.notify();
            });
        },
    );
    let ws_caveat = caveat(
        theme,
        "View ▸ Show Symbol ▸ Show Whitespace is the live knob; this preference is the persisted default.",
    );
    let ws_group = render_group_box(
        theme,
        "Display Whitespace and Tab",
        vec![ws_toggle, ws_caveat],
    );

    // --- Display EOL ---------------------------------------------------
    let weak_eol = weak.clone();
    let eol_toggle = npp_checkbox(
        theme,
        "prefs-editing2-eol",
        "Show end of line",
        snap.eol_visible,
        move |checked, cx| {
            let _ = weak_eol.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.eol_visible = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "EOL markers shown".to_string()
                } else {
                    "EOL markers: hidden".to_string()
                });
                cx.notify();
            });
        },
    );
    let eol_mode = snap.eol_display_mode;
    let eol_on = snap.eol_visible;
    let eol_plain = if eol_on {
        eol_mode_radio(
            theme,
            weak,
            "prefs-editing2-eol-plain",
            "Plain text",
            eol_mode,
            EolDisplayMode::PlainText,
        )
    } else {
        npp_radio_disabled(theme, "Plain text", eol_mode == EolDisplayMode::PlainText)
    };
    let eol_rounded = if eol_on {
        eol_mode_radio(
            theme,
            weak,
            "prefs-editing2-eol-rounded",
            "Rounded rectangle",
            eol_mode,
            EolDisplayMode::RoundedRect,
        )
    } else {
        npp_radio_disabled(
            theme,
            "Rounded rectangle",
            eol_mode == EolDisplayMode::RoundedRect,
        )
    };
    let eol_group = render_group_box(
        theme,
        "Display EOL",
        vec![eol_toggle, eol_plain, eol_rounded],
    );

    // --- Display Non-Printing Characters -------------------------------
    let weak_npc = weak.clone();
    let npc_toggle = npp_checkbox(
        theme,
        "prefs-editing2-npc",
        "Show non-printing characters",
        snap.npc_visible,
        move |checked, cx| {
            let _ = weak_npc.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.npc_visible = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "NPC glyphs shown".to_string()
                } else {
                    "NPC glyphs: hidden".to_string()
                });
                cx.notify();
            });
        },
    );
    let npc_mode = snap.npc_display_mode;
    let npc_on = snap.npc_visible;
    let npc_abbrev = if npc_on {
        npc_mode_radio(
            theme,
            weak,
            "prefs-editing2-npc-abbrev",
            "Abbreviation (e.g. NUL)",
            npc_mode,
            NpcDisplayMode::Abbreviation,
        )
    } else {
        npp_radio_disabled(
            theme,
            "Abbreviation (e.g. NUL)",
            npc_mode == NpcDisplayMode::Abbreviation,
        )
    };
    let npc_codepoint = if npc_on {
        npc_mode_radio(
            theme,
            weak,
            "prefs-editing2-npc-codepoint",
            "Codepoint (e.g. U+0000)",
            npc_mode,
            NpcDisplayMode::Codepoint,
        )
    } else {
        npp_radio_disabled(
            theme,
            "Codepoint (e.g. U+0000)",
            npc_mode == NpcDisplayMode::Codepoint,
        )
    };
    let npc_cc_toggle = if npc_on {
        let weak_cc = weak.clone();
        npp_checkbox(
            theme,
            "prefs-editing2-npc-cc-uni-eol",
            "Apply appearance to C0, C1 & Unicode EOL",
            snap.npc_include_cc_uni_eol,
            move |checked, cx| {
                let _ = weak_cc.update(cx, |this, cx| {
                    let mut next = settings::snapshot();
                    next.npc_include_cc_uni_eol = checked;
                    settings::replace_and_save(next);
                    this.last_action = Some(if checked {
                        "NPC: C0/C1/Uni-EOL included".to_string()
                    } else {
                        "NPC: C0/C1/Uni-EOL excluded".to_string()
                    });
                    cx.notify();
                });
            },
        )
    } else {
        npp_checkbox_disabled(
            theme,
            "Apply appearance to C0, C1 & Unicode EOL",
            snap.npc_include_cc_uni_eol,
        )
    };
    let weak_c0 = weak.clone();
    let npc_c0_toggle = npp_checkbox(
        theme,
        "prefs-editing2-prevent-c0",
        "Prevent control character (C0 code) typing into document",
        snap.prevent_c0_input,
        move |checked, cx| {
            let _ = weak_c0.update(cx, |this, cx| {
                let mut next = settings::snapshot();
                next.prevent_c0_input = checked;
                settings::replace_and_save(next);
                this.last_action = Some(if checked {
                    "C0 control-byte input: blocked".to_string()
                } else {
                    "C0 control-byte input: allowed".to_string()
                });
                cx.notify();
            });
        },
    );
    let npc_group = render_group_box(
        theme,
        "Display Non-Printing Characters",
        vec![
            npc_toggle,
            npc_abbrev,
            npc_codepoint,
            npc_cc_toggle,
            npc_c0_toggle,
        ],
    );

    body.child(multi_group)
        .child(ws_group)
        .child(eol_group)
        .child(npc_group)
        .into_any_element()
}

fn caveat(theme: &Theme, text: &'static str) -> gpui::AnyElement {
    div()
        .text_size(rems(11. / 16.))
        .text_color(rgb(theme.chrome_text))
        .child(SharedString::from(text))
        .into_any_element()
}

fn eol_mode_radio(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    id: &'static str,
    label: &'static str,
    current: EolDisplayMode,
    target: EolDisplayMode,
) -> gpui::AnyElement {
    let weak = weak.clone();
    npp_radio(theme, id, label, current == target, move |cx| {
        let _ = weak.update(cx, |this, cx| {
            let mut next = settings::snapshot();
            next.eol_display_mode = target;
            settings::replace_and_save(next);
            this.last_action = Some(format!(
                "EOL display: {} (paint pending)",
                eol_label(target)
            ));
            cx.notify();
        });
    })
}

fn eol_label(m: EolDisplayMode) -> &'static str {
    match m {
        EolDisplayMode::PlainText => "plain text",
        EolDisplayMode::RoundedRect => "rounded rectangle",
    }
}

fn npc_mode_radio(
    theme: &Theme,
    weak: &WeakEntity<PreferencesDialog>,
    id: &'static str,
    label: &'static str,
    current: NpcDisplayMode,
    target: NpcDisplayMode,
) -> gpui::AnyElement {
    let weak = weak.clone();
    npp_radio(theme, id, label, current == target, move |cx| {
        let _ = weak.update(cx, |this, cx| {
            let mut next = settings::snapshot();
            next.npc_display_mode = target;
            settings::replace_and_save(next);
            this.last_action = Some(format!(
                "NPC mode: {} (paint pending)",
                npc_label(target)
            ));
            cx.notify();
        });
    })
}

fn npc_label(m: NpcDisplayMode) -> &'static str {
    match m {
        NpcDisplayMode::Abbreviation => "abbreviation",
        NpcDisplayMode::Codepoint => "codepoint",
    }
}
