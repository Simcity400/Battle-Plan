use super::{RecordedName, recorded_name};

#[test]
fn a_saved_name_is_localised_scoped_or_literal_as_recorded() {
    let localisation = crate::catalogue::tests::loaded(concat!(
        "l_english:\n",
        " MOD_advisor:0 \"Current Name\"\n",
        " SCOPED_NAME:0 \"[ROOT.GetLeader]\"\n",
        " BROKEN_NAME:0 \"$MISSING_REFERENCE$\"\n",
        " GAP_NAME:0 \"Jos¤é\"\n",
    ));
    let name = |saved: &str| recorded_name(&localisation, saved);
    // A key is localised.
    assert_eq!(
        name("MOD_advisor"),
        RecordedName::Named("Current Name".into())
    );
    // Character names are plain: brackets are glyphs.
    assert_eq!(
        name("SCOPED_NAME"),
        RecordedName::Named("[ROOT.GetLeader]".into())
    );
    // The refusal names the key and the control, not an unexplained gap.
    assert_eq!(
        name("GAP_NAME"),
        RecordedName::Unresolved(
            "localisation GAP_NAME holds the control byte 0x14 (an authored `¤`), which plain text cannot show".into()
        )
    );
    // A nested key the content lacks writes nothing.
    assert_eq!(name("BROKEN_NAME"), RecordedName::Named(String::new()));
    assert_eq!(name("Álex Reed"), RecordedName::Named("Álex Reed".into()));
    // A saved name equal to the script token is localised the same way:
    // one the content does not define is its own bytes.
    assert_eq!(name("MOD_other"), RecordedName::Named("MOD_other".into()));
    assert_eq!(name(""), RecordedName::None);
}
