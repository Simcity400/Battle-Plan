pub fn channel_label(version: &str) -> &'static str {
    if version.contains("-nightly.") {
        "Nightly"
    } else {
        "Alpha"
    }
}
