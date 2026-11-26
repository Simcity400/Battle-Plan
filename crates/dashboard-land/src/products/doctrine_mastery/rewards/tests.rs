use super::*;
use replay_data::rows::{DoctrineTrackRow, LexicalTokenRow, TableRows};
use replay_frame::frame::{Metrics, TableView};

fn frame(rows: Vec<TableRows>) -> DecodedFrame {
    let read = tables();
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 7,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|index| {
            let id = TableId::ALL[index];
            if read.contains(&id) { TableView::Complete(TableRows::empty(id)).into() } else { replay_frame::frame::not_selected() }
        }),
    };
    for rows in rows {
        let id = rows.table();
        *frame.tables[id.index()] = TableView::Complete(rows);
    }
    frame
}

fn content() -> Context {
    let (catalogue, _folder) = dashboard_base::catalogue::tests::folder_catalogue(&[
        ("common/defines/test.lua", "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23}, NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}"),
        ("common/doctrines/subdoctrines/test.txt", "mod_sub={rewards={tier1={mastery=2 effect={add_political_power=2}} tier2={}}}"),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " mod_sub_tier1:0 \"Mod reward one\"\n",
                " mod_sub_tier1_desc:0 \"Generated reward prose\"\n",
                " mod_sub_tier2:0 \"Mod reward two\"\n",
                " POLITICS_ADD_POLITICAL_POWER:0 \"PP$VAL|0$\"\n",
            ),
        ),
    ]);
    let catalogue = dashboard_base::catalogue::tests::with_effect_describer(catalogue, dashboard_effects::effect_describer);
    let catalogue = Arc::new(catalogue);
    catalogue.scripted_content().unwrap().set_synthetic_environment(Default::default());
    let database = Arc::new(content_reader_military::doctrines::read(&catalogue.content.layers).unwrap());
    Context { catalogue, database, default_reward: Ok(100_000) }
}

fn params(track: u32) -> DoctrineRewardsParams {
    DoctrineRewardsParams { branch_id: "1".into(), country: "MOD".into(), folder: 0, track }
}

fn rewards(frame: &DecodedFrame, params: &DoctrineRewardsParams, slot: u32, context: &Context) -> io::Result<DoctrineRewardsReply> {
    super::rewards(frame, params, slot, context, |token, source, warnings| describe(frame, slot, context, token, source, warnings))
}

#[test]
fn a_tracks_rewards_are_its_subdoctrines_in_authored_order_named_with_their_thresholds_and_described_for_the_country() {
    let context = content();
    let frame = frame(vec![
        TableRows::LexicalTokens(vec![LexicalTokenRow { token: 1, literal: b"mod_sub".to_vec() }]),
        TableRows::DoctrineTracks(vec![
            DoctrineTrackRow { slot: 5_000, folder: 0, ordinal: 0, sub_doctrine: Some(1), rewards: 1, ..Default::default() },
            DoctrineTrackRow { slot: 5_000, folder: 0, ordinal: 1, sub_doctrine: None, ..Default::default() },
            DoctrineTrackRow { slot: 6_000, folder: 0, ordinal: 2, sub_doctrine: Some(1), ..Default::default() },
        ]),
    ]);
    let reply = rewards(&frame, &params(0), 5_000, &context).unwrap();
    assert_eq!(reply.sub.as_deref(), Some("mod_sub"));
    assert_eq!(reply.rewards.iter().map(|reward| (reward.id.as_str(), reward.name.as_deref(), reward.threshold.as_deref())).collect::<Vec<_>>(), [
        ("tier1", Some("Mod reward one"), Some("200000")),
        ("tier2", Some("Mod reward two"), Some("100000")),
    ], "a reward authored without mastery takes the define's default");
    let first = reply.rewards[0].description.as_deref().unwrap_or_default();
    assert!(first.contains("Generated reward prose") && first.contains("PP2"), "{first:?} {:?}", reply.warnings);
    assert_eq!(reply.rewards[1].description, None, "a reward the game words nothing for has no description, not an empty one");

    let none = rewards(&frame, &params(1), 5_000, &context).unwrap();
    assert!(none.sub.is_none() && none.rewards.is_empty(), "no subdoctrine chosen has no rewards");
    let other = rewards(&frame, &params(2), 5_000, &context).unwrap_err();
    assert_eq!(dashboard_core::refusal(&other), Some("This doctrine track is not recorded for the country at this hour"), "another country's track is not this one's");
}

#[test]
fn rewards_not_recorded_at_the_hour_refuse_by_name_and_a_subdoctrine_the_content_lacks_is_named() {
    let context = content();
    let mut frame = frame(vec![
        TableRows::LexicalTokens(vec![LexicalTokenRow { token: 1, literal: b"gone_sub".to_vec() }]),
        TableRows::DoctrineTracks(vec![DoctrineTrackRow { slot: 5_000, folder: 0, ordinal: 0, sub_doctrine: Some(1), ..Default::default() }]),
    ]);
    let missing = rewards(&frame, &params(0), 5_000, &context).unwrap();
    assert!(missing.rewards.is_empty());
    assert!(missing.warnings.iter().any(|why| why.contains("not in the content: gone_sub")), "{:?}", missing.warnings);
    *frame.tables[TableId::DoctrineTracks.index()] = TableView::Unavailable { reason: "not read".into() };
    let error = rewards(&frame, &params(0), 5_000, &context).unwrap_err();
    assert!(error.to_string().starts_with("Doctrine rewards are not recorded at this hour"), "{error}");
}
