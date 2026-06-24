// SPDX-License-Identifier: Apache-2.0
//
// Style Configurator layout primitives.
//
// Phase 4 replaces the right-column placeholder with the real Colour Style
// + Font Style group boxes (see `right_column.rs`). The left column was
// landed in Phase 3.

use gpui::{AnyElement, Entity, IntoElement, ParentElement, Styled, div, rems};

use crate::theme::Theme;

use super::{left_column, right_column, StyleConfiguratorDialog};

/// Two-column body: left = Language / Style listboxes; right = Colour &
/// Font controls. The dialog's footer + header rows are drawn by `mod.rs`.
pub(super) fn body(
    theme: &Theme,
    dialog: &StyleConfiguratorDialog,
    entity: &Entity<StyleConfiguratorDialog>,
) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .flex_1()
        .gap(rems(8. / 16.))
        .child(left_column::render(theme, dialog, entity))
        .child(right_column::render(theme, dialog, entity))
        .into_any_element()
}
