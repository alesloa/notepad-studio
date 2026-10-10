pub(crate) const APP_NAME: &str = "Notepad Studio";
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) const TAB_SPACES: usize = 4;
pub(crate) const CARET_WIDTH_PX: f32 = 1.5;
// Overtype caret: Scintilla's overstrike bar, 2px tall under the next
// character and never narrower than 3px (scintilla/src/EditView.cxx:1533,
// :1549).
pub(crate) const OVERSTRIKE_CARET_HEIGHT_PX: f32 = 2.0;
pub(crate) const OVERSTRIKE_CARET_MIN_WIDTH_PX: f32 = 3.0;

// VS Code-style zoom. `ZOOM_REM_BASE` is gpui's default rem size (16px); the
// active rem size is `ZOOM_REM_BASE * zoom_level`, which scales every
// rems()-based UI dimension uniformly.
pub(crate) const ZOOM_REM_BASE: f32 = 16.0;
pub(crate) const ZOOM_DEFAULT: f32 = 1.0;
pub(crate) const ZOOM_STEP: f32 = 1.1;
pub(crate) const ZOOM_MIN: f32 = 0.5;
pub(crate) const ZOOM_MAX: f32 = 3.0;
