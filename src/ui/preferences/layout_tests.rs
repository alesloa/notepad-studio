use super::*;
use gpui::{AppContext, TestAppContext};

#[test]
fn preferences_constructor_keeps_nonfinite_saved_width_out_of_layout() {
    for value in ["inf", "nan", "-inf"] {
        let saved: settings::Settings = toml::from_str(&format!(
            "preferences_sidebar_width = {value}\n"
        )).unwrap();
        let _settings = settings::override_for_test(saved);
        let mut cx = TestAppContext::single();
        let dialog = cx.new(|cx| PreferencesDialog::new(ThemeMode::Dark, None, cx));
        let width = cx.update(|cx| f32::from(dialog.read(cx).sidebar_width));
        assert!(width.is_finite(), "saved {value} produced layout width {width}");
        assert_eq!(width, settings::DEFAULT_PREFERENCES_SIDEBAR_WIDTH);
        cx.quit();
    }
}

#[test]
fn preferences_constructor_preserves_finite_saved_width_and_minimum() {
    for saved_width in [-1., 0., 99., 100., 190., 420.] {
        let saved = settings::Settings {
            preferences_sidebar_width: saved_width,
            ..Default::default()
        };
        let _settings = settings::override_for_test(saved);
        let mut cx = TestAppContext::single();
        let dialog = cx.new(|cx| PreferencesDialog::new(ThemeMode::Dark, None, cx));
        let width = cx.update(|cx| f32::from(dialog.read(cx).sidebar_width));
        assert_eq!(width, saved_width.max(SIDEBAR_MIN_WIDTH_PX));
        cx.quit();
    }
}
