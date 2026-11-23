use super::*;
use content_reader_military::doctrines::{Database, Name, RewardSource};
use content_reader_script::localization_binding::BoundLocalization;

fn bound(body: &str) -> BoundLocalization {
    BoundLocalization::from_value(
        &content_reader_parser::parser::parse(body.as_bytes())
            .unwrap()
            .items[0]
            .value,
    )
}

fn database() -> Database {
    let mut database = Database::default();
    let grand = Token::c_str(b"g");
    let sub = Token::c_str(b"s");
    let track = Token::c_str(b"t");
    database.grand.insert(
        grand.clone(),
        Name {
            value: b"GRAND".to_vec(),
            refusal: None,
        },
    );
    database.sub.insert(
        sub.clone(),
        Name {
            value: b"SUB".to_vec(),
            refusal: None,
        },
    );
    let sources = &mut database.tactic_sources;
    sources.tracks.insert(
        track.clone(),
        bound("name={localization_key=TRACK A={localization_key=INNER N=VALUE}}"),
    );
    sources.grand_order = vec![grand.clone(), grand.clone()];
    sources.sub_order = vec![sub.clone()];
    sources.grand.insert(
        grand,
        DefinitionSources {
            available: content_reader_parser::parser::parse(
                b"ROOT={tag=AAA} FROM={tag=AAA} PREV={tag=AAA}",
            )
            .unwrap()
            .items,
            tactics: vec![b"Target".to_vec(), b"Target".to_vec()],
            tracks: vec![track.clone(), track.clone()],
            tracks_requested: true,
            milestones: vec![vec![b"Target".to_vec()], vec![b"Other".to_vec()]],
            rewards: Vec::new(),
            ..Default::default()
        },
    );
    sources.sub.insert(
        sub,
        DefinitionSources {
            tactics: vec![b"Target".to_vec()],
            tracks: vec![track.clone(), track],
            rewards: vec![
                RewardSource {
                    tactics: vec![b"Other".to_vec()],
                    mastery: 200_000,
                    ..Default::default()
                },
                RewardSource {
                    tactics: vec![b"Target".to_vec(), b"Target".to_vec()],
                    mastery: 0,
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
    );
    sources.default_reward_mastery = Some(300_000);
    database
}

fn run(
    database: &Database,
    country: u32,
) -> (Result<Vec<String>, String>, BTreeMap<String, usize>) {
    let mut fixture = super::super::tests::Fixture::new();
    *fixture.frame.tables[replay_data::rows::TableId::CompletedFocuses.index()] =
        replay_frame::frame::TableView::Unavailable {
            reason: "synthetic completed focuses unavailable".into(),
        };
    let (catalogue, _folder) = crate::fixtures::folder_catalogue(&[(
        "localisation/test_l_english.yml",
        concat!(
            "\u{feff}l_english:\n",
            " GRAND:0 \"Grand\"\n SUB:0 \"Sub\"\n VALUE:0 \"V\"\n",
            " INNER:0 \"Inner $N$\"\n TRACK:0 \"Track $A$ [ROOT.GetName]\"\n",
            " DOCTRINE_EFFECT_SOURCE_GRAND_DOCTRINE:0 \"G:$GRAND_DOCTRINE$\"\n",
            " DOCTRINE_EFFECT_SOURCE_MILESTONE:0 \"M:$GRAND_DOCTRINE$/$TRACK$\"\n",
            " DOCTRINE_EFFECT_SOURCE_SUBDOCTRINE:0 \"S:$SUBDOCTRINE$/$TRACK$\"\n",
            " DOCTRINE_EFFECT_SOURCE_REWARD:0 \"R:$SUBDOCTRINE$/$TRACK$/$MASTERY|0$\"\n"
        ),
    )]);
    let mut refused = BTreeMap::new();
    let value = fixture.with_renderer(
        &mut super::super::tests::fresh(),
        Content::default(),
        |renderer| {
            let context = Context::new(
                renderer.context.text,
                &catalogue.localisation,
                3,
                "synthetic",
                Content {
                    doctrines: Some(database),
                    ..Default::default()
                },
            );
            doctrine_tactic_sources(
                &context,
                country,
                7,
                |token| Ok(if token == b"Target" { 7 } else { 0 }),
                &mut refused,
            )
        },
    );
    (value, refused)
}

#[test]
fn occurrences_order_nested_names_and_all_reward_mastery_are_preserved() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (sources, refused) = run(&database(), 1);
    assert!(refused.is_empty(), "{refused:?}");
    let track = "Track Inner V [ROOT.GetName]";
    assert_eq!(
        sources.unwrap(),
        vec![
            "G:Grand".to_owned(),
            format!("M:Grand/{track}"),
            "G:Grand".to_owned(),
            format!("M:Grand/{track}"),
            format!("S:Sub/{track}"),
            format!("S:Sub/{track}"),
            format!("R:Sub/{track}/5"),
            format!("R:Sub/{track}/5"),
        ]
    );
    let (sources, refused) = run(&database(), 2);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(sources.unwrap().len(), 4);
}

#[test]
fn local_refusals_preserve_known_sources_and_never_claim_unknown_empty() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut database = database();
    database.tactic_sources.sub_unread = Some("synthetic sub tail is unsettled".into());
    let (sources, refused) = run(&database, 1);
    assert_eq!(sources.unwrap().len(), 4);
    assert!(refused.keys().any(|why| why.contains("sub tail")));
    database.tactic_sources.sub_unread = None;
    database.tactic_sources.tracks_unread = Some("synthetic track tail is unsettled".into());
    let (sources, refused) = run(&database, 1);
    assert_eq!(sources.unwrap(), ["G:Grand", "G:Grand"]);
    assert!(refused.keys().any(|why| why.contains("track tail")));
    let (sources, refused) = run(&database, 2);
    assert!(sources.unwrap_err().contains("track tail"));
    assert!(!refused.is_empty());
    for definition in database
        .tactic_sources
        .grand
        .values_mut()
        .chain(database.tactic_sources.sub.values_mut())
    {
        definition.tracks.clear();
        definition.tracks_requested = false;
    }
    let (sources, refused) = run(&database, 1);
    assert_eq!(sources.unwrap(), ["G:Grand", "G:Grand"]);
    assert!(refused.is_empty(), "{refused:?}");
}

#[test]
fn bound_name_failure_and_default_define_shortage_are_feature_local() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut database = database();
    let bad = Token::c_str(b"bad");
    database
        .tactic_sources
        .tracks
        .insert(bad.clone(), bound("name=unported|VALUE"));
    database
        .tactic_sources
        .sub
        .get_mut(&Token::c_str(b"s"))
        .unwrap()
        .tracks
        .push(bad);
    database.tactic_sources.default_reward_mastery = None;
    let (sources, refused) = run(&database, 1);
    assert_eq!(sources.unwrap().len(), 6);
    assert!(refused.keys().any(|why| why.contains("unported")));
    assert!(
        refused
            .keys()
            .any(|why| why.contains("DEFAULT_REWARD_MASTERY"))
    );
}

#[test]
fn unknown_available_gate_keeps_independent_sources_and_is_not_false() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut database = database();
    database
        .tactic_sources
        .grand
        .get_mut(&Token::c_str(b"g"))
        .unwrap()
        .available = content_reader_parser::parser::parse(b"has_completed_focus=synthetic_focus")
        .unwrap()
        .items;
    let (sources, refused) = run(&database, 1);
    assert_eq!(sources.unwrap().len(), 4);
    assert!(refused.keys().any(|why| why.contains("CompletedFocuses")));
    database.tactic_sources.sub_order.clear();
    let (sources, refused) = run(&database, 1);
    assert!(sources.unwrap_err().contains("CompletedFocuses"));
    assert!(!refused.is_empty());
}
