// SPDX-License-Identifier: Apache-2.0
//
// Monospace font resolution per platform.
//
// The bundled themes name "Menlo" as the Default Style font, and the UI used
// to hardcode it. Menlo only ships on macOS; on Windows and Linux gpui then
// falls back to a proportional UI face (Segoe UI), which breaks column
// alignment, Column Mode and every width computed from a monospace advance.
//
// `init` captures the platform's installed families once at startup (the
// same enumerator the Style Configurator's font combo uses). `mono()` is the
// first installed family from a per-OS candidate list; `installed(name)`
// lets the theme's Default Style font win when the user actually has it,
// mirroring Notepad++ where Style Configurator ▸ Global Styles ▸ Default
// Style ▸ Font name drives the editor face.

use std::sync::OnceLock;

use gpui::{App, SharedString};

static INSTALLED: OnceLock<Vec<String>> = OnceLock::new();
static MONO: OnceLock<SharedString> = OnceLock::new();

/// Candidates in preference order. The last entry is the answer when the
/// enumerator returned nothing (headless runs, tests): it is the family most
/// likely to exist on that OS.
#[cfg(target_os = "macos")]
const MONO_CANDIDATES: &[&str] = &["Menlo", "Monaco", "Courier New", "Courier"];
#[cfg(target_os = "windows")]
const MONO_CANDIDATES: &[&str] = &["Consolas", "Cascadia Mono", "Lucida Console", "Courier New"];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const MONO_CANDIDATES: &[&str] = &[
    "DejaVu Sans Mono",
    "Liberation Mono",
    "Noto Sans Mono",
    "Ubuntu Mono",
    "Cascadia Mono",
    "Courier New",
    "FreeMono",
    "monospace",
];

/// Capture installed font families. Call once from `App::run`, before the
/// first window renders. Later calls are no-ops.
pub(crate) fn init(cx: &App) {
    let names = cx.text_system().all_font_names();
    let mono = SharedString::from(pick(MONO_CANDIDATES, &names).to_string());
    let _ = INSTALLED.set(names);
    let _ = MONO.set(mono);
}

/// The platform monospace family for UI text and the editor.
pub(crate) fn mono() -> SharedString {
    MONO.get()
        .cloned()
        .unwrap_or_else(|| SharedString::from(fallback(MONO_CANDIDATES)))
}

/// `name` as spelled by the font enumerator, when that family is installed.
/// Case-insensitive, because hand-edited theme XML often differs in case
/// from the family name the OS reports.
pub(crate) fn installed(name: &str) -> Option<&'static str> {
    let names: &'static Vec<String> = INSTALLED.get()?;
    find_family(names, name)
}

/// Editor family for a theme's Default Style `fontName`: the theme's font when
/// installed, otherwise the platform monospace.
pub(crate) fn editor_family(theme_font: Option<&str>) -> SharedString {
    match theme_font {
        Some(name) => SharedString::from(name.to_string()),
        None => mono(),
    }
}

fn find_family<'a>(names: &'a [String], wanted: &str) -> Option<&'a str> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return None;
    }
    names
        .iter()
        .find(|n| n.eq_ignore_ascii_case(wanted))
        .map(String::as_str)
}

fn pick<'a>(candidates: &[&'a str], installed: &'a [String]) -> &'a str {
    candidates
        .iter()
        .find_map(|c| find_family(installed, c))
        .unwrap_or_else(|| fallback(candidates))
}

fn fallback<'a>(candidates: &[&'a str]) -> &'a str {
    candidates.last().copied().unwrap_or("monospace")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn pick_takes_first_installed_candidate() {
        let installed = names(&["Arial", "Courier New", "Consolas"]);
        assert_eq!(pick(&["Menlo", "Consolas", "Courier New"], &installed), "Consolas");
    }

    #[test]
    fn pick_returns_enumerator_spelling() {
        let installed = names(&["dejavu sans mono"]);
        assert_eq!(pick(&["DejaVu Sans Mono"], &installed), "dejavu sans mono");
    }

    #[test]
    fn pick_falls_back_to_last_candidate_when_nothing_installed() {
        assert_eq!(pick(&["Menlo", "Courier New"], &[]), "Courier New");
        assert_eq!(pick(&[], &[]), "monospace");
    }

    #[test]
    fn find_family_ignores_blank_and_case() {
        let installed = names(&["Consolas"]);
        assert_eq!(find_family(&installed, "  "), None);
        assert_eq!(find_family(&installed, ""), None);
        assert_eq!(find_family(&installed, "CONSOLAS"), Some("Consolas"));
        assert_eq!(find_family(&installed, "Menlo"), None);
    }

    #[test]
    fn editor_family_prefers_theme_font() {
        assert_eq!(editor_family(Some("Fira Code")).as_ref(), "Fira Code");
        assert_eq!(editor_family(None), mono());
    }
}
