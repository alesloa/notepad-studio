// Action types split by feature area. Each submodule owns the `actions!` block
// for its feature; this index re-exports every type so existing call sites
// (`use crate::actions::*` / `use crate::actions::ActionName`) keep working.

pub mod bookmarks;
pub mod edit;
pub mod edit_ops;
pub mod encoding;
pub mod file;
pub mod help;
pub mod language;
pub mod macros;
pub mod markdown;
pub mod menu_bar;
pub mod plugins;
pub mod proofreading;
pub mod rich_doc;
pub mod run;
pub mod search;
pub mod settings;
pub mod tabs;
pub mod tools;
pub mod view;

pub use bookmarks::*;
pub use edit::*;
pub use edit_ops::*;
pub use encoding::*;
pub use file::*;
pub use help::*;
pub use language::*;
pub use macros::*;
pub use markdown::*;
pub use menu_bar::*;
pub use plugins::*;
pub use proofreading::*;
pub use rich_doc::*;
pub use run::*;
pub use search::*;
pub use settings::*;
pub use tabs::*;
pub use tools::*;
pub use view::*;
