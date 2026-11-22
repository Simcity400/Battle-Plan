#[cfg(test)]
use content_reader_localisation::localisation::Localisation;
pub use viewer_runtime::recorded_labels::RecordedLabels;

#[cfg(test)]
pub(crate) fn english() -> &'static RecordedLabels {
    static ENGLISH: std::sync::LazyLock<RecordedLabels> = std::sync::LazyLock::new(|| {
        let mut english = Localisation::default();
        english.language = "english".into();
        RecordedLabels::from_localisation(&english)
    });
    &ENGLISH
}
