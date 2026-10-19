//! Shared recorded source-label recovery for combat hovers.
#[cfg(test)]
use installed_content_localisation::localisation::Localisation;
pub use viewer_foundation::recorded_labels::RecordedLabels;

/// Labels of an English recording with no localisation keys, for tests.
#[cfg(test)]
pub(crate) fn english() -> &'static RecordedLabels {
    static ENGLISH: std::sync::LazyLock<RecordedLabels> = std::sync::LazyLock::new(|| {
        let mut english = Localisation::default();
        english.language = "english".into();
        RecordedLabels::from_localisation(&english)
    });
    &ENGLISH
}
