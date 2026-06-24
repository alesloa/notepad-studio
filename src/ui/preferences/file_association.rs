// File Association panel — placeholder.
//
// NPP's File Association panel registers file extensions with the OS so
// double-clicking a file opens it in the editor. On macOS this requires the
// app to ship as a `.app` bundle with `Info.plist::CFBundleDocumentTypes`
// declarations and (for "set as default") `LSSetDefaultRoleHandlerForContentType`
// from LaunchServices.
//
// Notepad Studio currently builds as a plain executable, so registration cannot be
// wired today. This panel renders an honest explanation rather than a fake
// list of toggles. Once `.app` packaging lands (cargo-bundle or hand-rolled),
// the panel will be replaced with extension-pick controls.

use gpui::{IntoElement, ParentElement, SharedString, Styled, div, rems, rgb};

use crate::theme::Theme;

use super::widgets::render_group_box;

const CANDIDATE_EXTENSIONS: &[&str] = &[
    ".txt", ".md", ".rs", ".toml", ".json", ".yaml", ".yml", ".html", ".css",
    ".js", ".ts", ".tsx", ".py", ".go", ".c", ".cpp", ".h", ".hpp", ".sh",
    ".log", ".csv", ".xml",
];

pub(super) fn render(theme: &Theme, body: gpui::Div) -> gpui::AnyElement {
    let blocker_group = render_group_box(
        theme,
        "Status",
        vec![
            div()
                .text_color(rgb(theme.chrome_text))
                .text_size(rems(11. / 16.))
                .child(SharedString::from(
                    "File-type registration requires Notepad Studio to ship as a macOS .app bundle so LaunchServices can route opens to it. Notepad Studio currently builds as a plain executable, so this panel cannot register handlers yet.",
                ))
                .into_any_element(),
            div()
                .text_color(rgb(theme.chrome_text))
                .text_size(rems(11. / 16.))
                .child(SharedString::from(
                    "Once .app packaging is added (Info.plist with CFBundleDocumentTypes plus cargo-bundle), this panel will let you pick which extensions Notepad Studio handles by default.",
                ))
                .into_any_element(),
        ],
    );

    let extensions_label = div()
        .text_color(rgb(theme.chrome_text))
        .text_size(rems(11. / 16.))
        .child(SharedString::from(
            "When packaging lands, Notepad Studio plans to register these extensions:",
        ))
        .into_any_element();

    let extensions_grid = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(rems(8. / 16.))
        .children(CANDIDATE_EXTENSIONS.iter().map(|ext| {
            div()
                .text_color(rgb(theme.chrome_text))
                .text_size(rems(11. / 16.))
                .child(SharedString::from(*ext))
                .into_any_element()
        }))
        .into_any_element();

    let preview_group = render_group_box(
        theme,
        "Planned extensions",
        vec![extensions_label, extensions_grid],
    );

    body.child(blocker_group).child(preview_group).into_any_element()
}
