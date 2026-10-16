use super::{RecordedName, recorded_name};

#[test]
fn fixed_ledger_text_resolves_once_per_key_without_caching_recorded_names() {
    use crate::{
        catalogue::View,
        prepared::observer_tests::{self, Work},
    };
    use std::sync::Arc;
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for politics content");
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/mod.lua",
            "NDefines={NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        ),
        (
            "localisation/test_l_english.yml",
            "\u{feff}l_english:\n fixed:0 \"Fixed payment\"\n empty:0 \"\"\n refused:0 \"Authored ¤ control\"\n saved:0 \"Recorded [ROOT.GetLeader]\"\n",
        ),
    ]);
    let content = super::PoliticsContent::load(Arc::new(catalogue)).unwrap();
    let keys = ["fixed", "empty", "missing", "refused"];
    let expected: Vec<_> = keys
        .iter()
        .map(|key| content.localised(key, View::Plain))
        .collect();
    assert_eq!(
        expected,
        [
            Some("Fixed payment".into()),
            Some(String::new()),
            None,
            None
        ]
    );
    let refusals = content.catalogue.refusal_lines();
    assert!(!refusals.is_empty());
    observer_tests::scaled_work(10, |hours| {
        for _ in 0..hours {
            for (key, expected) in keys.iter().zip(&expected) {
                assert_eq!(&content.localised(key, View::Plain), expected);
                assert_eq!(content.fixed_text.loads(&key.to_string()), 1);
            }
            assert_eq!(
                content.recorded_name("saved"),
                RecordedName::Named("Recorded [ROOT.GetLeader]".into())
            );
        }
        assert_eq!(content.catalogue.refusal_lines(), refusals);
        assert_eq!(content.fixed_text.loads(&"saved".to_string()), 0);
        Ok(Work {
            content_loads: keys
                .iter()
                .map(|key| content.fixed_text.loads(&key.to_string()) - 1)
                .sum(),
            ..Default::default()
        })
    })
    .map(observer_tests::assert_flat)
    .unwrap();
}

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
