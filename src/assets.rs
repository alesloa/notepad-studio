// Embedded asset source. Maps `icons/*.svg` paths to `include_bytes!` blobs
// so the binary stays standalone (no on-disk asset dir needed at runtime).
//
// Wired into `Application::new().with_assets(NotepadStudioAssets)` from `main.rs`.
// gpui's `svg().path("icons/foo.svg")` resolves through this.

use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};

pub(crate) struct NotepadStudioAssets;

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        &[$(
            ($name, include_bytes!(concat!("../assets/icons/", $name)).as_slice()),
        )*]
    };
}

#[rustfmt::skip]
const ICONS: &[(&str, &[u8])] = icons![
    "plus.svg",
    "folder_open.svg",
    "save.svg",
    "save_all.svg",
    "close.svg",
    "scissors.svg",
    "copy.svg",
    "paste.svg",
    "undo.svg",
    "rotate_cw.svg",
    "magnifying_glass.svg",
    "replace.svg",
    "folder_search.svg",
    "book.svg",
    "zoom_in.svg",
    "zoom_out.svg",
    "zoom_reset.svg",
    "whitespace.svg",
    "word_wrap.svg",
    "maximize.svg",
    "circle.svg",
    "stop.svg",
    "play_outlined.svg",
    "play_filled.svg",
    "book_copy.svg",
    "folder.svg",
    "download.svg",
    "font_size.svg",
    "space.svg",
    "return.svg",
    "wrench.svg",
    "code.svg",
    "book_open.svg",
    "format_bold.svg",
    "format_italic.svg",
    "format_underline.svg",
    "format_color.svg",
    "format_align_left.svg",
    "format_align_center.svg",
    "format_align_right.svg",
    "format_align_justify.svg",
    "format_indent_increase.svg",
    "format_indent_decrease.svg",
    "format_strike.svg",
    "format_inline_code.svg",
    "heading_1.svg",
    "heading_2.svg",
    "heading_3.svg",
    "list_bullet.svg",
    "list_ordered.svg",
    "list_checks.svg",
    "link.svg",
    "quote.svg",
    "horizontal_rule.svg",
    "code_block.svg",
    "image.svg",
];

impl AssetSource for NotepadStudioAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let trimmed = path.strip_prefix("icons/").unwrap_or(path);
        for (name, bytes) in ICONS {
            if *name == trimmed {
                return Ok(Some(Cow::Borrowed(bytes)));
            }
        }
        Ok(None)
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS.iter().map(|(n, _)| SharedString::from(*n)).collect())
    }
}
