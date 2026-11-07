use super::HourKey;
use crate::test_fixtures::{CAMPAIGN, records, source};
use installed_content_localisation::localisation::Localisation;
use replay::campaign::Replay;
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use std::sync::{Arc, atomic::AtomicBool};

#[test]
fn an_hour_named_with_another_localisation_is_read_again() {
    let english: Arc<Localisation> = Arc::new(Localisation::default());
    let french: Arc<Localisation> = Arc::new(Localisation::default());
    let key = |localisation: Option<&Arc<Localisation>>| HourKey {
        branch: 1,
        hour: 100,
        localisation: localisation.cloned(),
    };
    assert!(key(Some(&english)).matches(&key(Some(&english))));
    assert!(!key(Some(&english)).matches(&key(Some(&french))));
    assert!(!key(None).matches(&key(Some(&english))));
    assert!(key(None).matches(&key(None)));
}

fn styled_frame(hour: i32, red: f32) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1));
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            replay_data::rows::CountryAppearanceRow {
                country: 9001,
                red: red.to_bits(),
                green: 0.5_f32.to_bits(),
                blue: 1.5_f32.to_bits(),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![replay_data::rows::PoliticsRow {
            country: 9001,
            ..Default::default()
        }]));
    frame
}

#[test]
fn country_styles_take_the_engine_name_and_the_recorded_map_colour() {
    // Colour codes in the game's text are removed from chart names.
    let localisation: installed_content_localisation::localisation::Localisation =
        [("GER".to_owned(), "§YGerman§! Reich".to_owned())]
            .into_iter()
            .collect();
    let styles = crate::products::country_styles::HourStyles::read(
        &styled_frame(100, 0.2),
        Some(&localisation),
    )
    .unwrap();
    let style = styles.get("GER").unwrap();
    assert_eq!(style.name.as_deref(), Some("German Reich"));
    // The engine truncates f * 255, and out-of-range floats saturate.
    assert_eq!(style.color, Some([51, 127, 255]));
    let untranslated =
        crate::products::country_styles::HourStyles::read(&styled_frame(100, 0.2), None).unwrap();
    assert_eq!(untranslated.get("GER").unwrap().name, None);
    assert!(untranslated.get("FRA").is_none());
}

#[test]
fn country_styles_answer_per_hour_and_keep_missing_values_missing() {
    use crate::products::{
        LoadContext, Product,
        country_styles::{CountryStyleParams, CountryStyles},
    };
    let mut no_colour = source(1, 101, Some(1));
    *no_colour.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![styled_frame(100, 0.2), no_colour])],
        )
        .unwrap(),
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "styles".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let request = |hour: i32| CountryStyleParams {
        branch_id: "1".into(),
        hour,
        tags: vec!["GER".into(), "XXX".into()],
    };
    let context = LoadContext { content: None };
    let first =
        CountryStyles::load(&source, &request(100), &context, &AtomicBool::new(false)).unwrap();
    assert_eq!(first.styles[0].color, Some([51, 127, 255]));
    assert_eq!(first.styles[0].name, None);
    assert_eq!(
        first.styles[1],
        crate::products::country_styles::CountryStyle {
            tag: "XXX".into(),
            name: None,
            color: None
        }
    );
    assert!(
        first
            .warnings
            .iter()
            .any(|w| w.contains("until the game folders"))
    );
    assert!(first.warnings.iter().any(|w| w.starts_with("XXX:")));
    let second =
        CountryStyles::load(&source, &request(101), &context, &AtomicBool::new(false)).unwrap();
    assert_eq!(second.styles[0].color, None);
    assert!(
        second
            .warnings
            .iter()
            .any(|w| w
                .starts_with("GER: no recorded map colour (the appearance table is incomplete"))
    );
    assert!(
        CountryStyles::validate(&CountryStyleParams {
            branch_id: "1".into(),
            hour: 1,
            tags: vec![]
        })
        .is_err()
    );
    assert!(
        CountryStyles::validate(&CountryStyleParams {
            branch_id: "x".into(),
            hour: 1,
            tags: vec!["GER".into()]
        })
        .is_err()
    );
}

#[test]
fn country_styles_report_an_incomplete_country_table_as_incomplete() {
    let mut frame = source(1, 100, Some(1));
    *frame.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let styles = crate::products::country_styles::HourStyles::read(&frame, None).unwrap();
    assert!(styles.get("GER").is_none());
    assert!(styles.missing_reason("GER").contains("incomplete"));
    let complete =
        crate::products::country_styles::HourStyles::read(&source(1, 100, Some(1)), None).unwrap();
    assert_eq!(complete.missing_reason("XXX"), "not a country at this hour");
}
