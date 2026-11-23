use super::*;
use replay_data::rows::LandBorderWarSide;
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::Arc;
use viewer_foundation::tooltip_text::NoIcons;

fn localisation() -> Localisation {
    [
        (BORDER_CONFLICT_IF_YOU_WIN, "Win"),
        (BORDER_CONFLICT_IF_YOU_LOSE, "Lose"),
        (BORDER_CONFLICT_CHANGE_STATE, "$STATE$ to $COUNTRY$"),
        (BORDER_CONFLICT_FOR_ENEMY, "For $COUNTRY$"),
        (EFFECT_COUNTRY_EVENT, "Event $EVENT$"),
    ]
    .into_iter()
    .map(|(key, text)| (key.into(), text.into()))
    .collect()
}

#[test]
fn border_marker_tail_keeps_transfer_and_unbracketed_enemy_markers_in_native_order() {
    let border = LandBorderWarInputs {
        transfer_state: 128,
        combat_width: replay_data::rows::Recorded::Value(0),
        sides: [
            LandBorderWarSide {
                country: 7,
                state: 11,
                win_event: b"own_win".to_vec(),
                lose_event: b"own_lose".to_vec(),
            },
            LandBorderWarSide {
                country: 9,
                state: 13,
                win_event: b"enemy_win".to_vec(),
                lose_event: b"enemy_lose".to_vec(),
            },
        ],
    };
    let mut body = "Marker".into();
    let mut refused = BTreeMap::new();
    build(
        &NoIcons,
        &localisation(),
        &border,
        0,
        &mut body,
        &|id, transfer| {
            Ok(country_marker(
                &format!("C{id}"),
                &format!("SYN{id}"),
                transfer,
            ))
        },
        &|id| Ok(format!("S{id}")),
        &|id, _, _| Description {
            body: format!("{}\n", String::from_utf8_lossy(id)),
            ..Default::default()
        },
        &mut refused,
    )
    .unwrap();
    assert_eq!(
        body,
        format!(
            "Marker\n\nWin\nS13 to C7 ({INLINE_FLAG}SYN7)\nown_win\nFor C9 {INLINE_FLAG}SYN9\nenemy_lose\n\nLose\nS11 to C9 ({INLINE_FLAG}SYN9)\nown_lose\nFor C9 {INLINE_FLAG}SYN9\nenemy_win\n"
        )
    );
    assert!(refused.is_empty());
    let mut body = "Marker".into();
    build(
        &NoIcons,
        &localisation(),
        &LandBorderWarInputs::default(),
        1,
        &mut body,
        &|_, _| unreachable!(),
        &|_| unreachable!(),
        &|_, _, _| unreachable!(),
        &mut refused,
    )
    .unwrap();
    assert_eq!(body, "Marker");
}

#[test]
fn border_marker_membership_and_event_scopes_use_native_alias_and_copy_links() {
    let links = [(7, 70), (9, 90)].into_iter().collect();
    assert_eq!(member(7, 70, &links), Ok(true));
    assert_eq!(member(9, 70, &links), Ok(false));
    assert_eq!(member(0, 70, &links), Ok(false));
    assert!(member(11, 70, &links).is_err());
    let values = scopes(70, 11, 90, 13).scopes;
    assert_eq!(values[0].root, Link::Scope(1));
    assert_eq!(values[0].from, Link::Scope(2));
    assert_eq!(values[2].from, Link::Scope(3));
    assert_eq!(values[3].from, Link::Scope(5));
    assert_eq!(values[5].from, Link::Itself);
    assert_eq!(values[0].prev, Link::Itself);
}

#[test]
fn border_marker_multi_option_enemy_event_appears_only_under_win() {
    content_reader_script::test_game::enter();
    let events = content_reader_events::events::parse(
        b"add_namespace=s country_event={id=s.5 title=VICTORY option={name=A} option={name=B}} country_event={id=s.6 title=DEFEAT option={name=A} option={name=B}}",
        "events/synthetic.txt",
        &content_reader_script::key_names::KeyNames::default(),
    );
    let border = LandBorderWarInputs {
        sides: [
            LandBorderWarSide {
                country: 7,
                state: 763,
                win_event: b"s.5".to_vec(),
                lose_event: b"s.6".to_vec(),
            },
            LandBorderWarSide {
                country: 9,
                state: 814,
                win_event: b"s.6".to_vec(),
                lose_event: b"s.5".to_vec(),
            },
        ],
        ..Default::default()
    };
    let base = localisation();
    let loc = base
        .iter()
        .map(|(key, text)| (key.into(), text.into()))
        .chain([(EFFECT_COUNTRY_EVENT.into(), "Event $EVENT$\n".into())])
        .collect();
    let describe =
        |_: &[Item], _: &mut EventScopes| -> effect_description::Description { unreachable!() };
    let render = |key: &[u8], _: &EventScopes| Ok(String::from_utf8_lossy(key).into_owned());
    for (mine, win, lose) in [(0, "VICTORY", "DEFEAT"), (1, "DEFEAT", "VICTORY")] {
        let mut body = "Marker".into();
        let mut refused = BTreeMap::new();
        let event = |id: &[u8], side: usize, enemy_win: bool| {
            let own = &border.sides[side];
            let enemy = &border.sides[1 - side];
            event_description(
                events.get(id).unwrap(),
                enemy_win,
                &describe,
                &mut scopes(
                    own.country as u32,
                    own.state as u32,
                    enemy.country as u32,
                    enemy.state as u32,
                ),
                &render,
                &NoIcons,
                &loc,
            )
        };
        build(
            &NoIcons,
            &loc,
            &border,
            mine,
            &mut body,
            &|_, _| Ok("Enemy".into()),
            &|_| unreachable!(),
            &event,
            &mut refused,
        )
        .unwrap();
        assert_eq!(
            body,
            format!("Marker\n\nWin\nEvent {win}\nFor Enemy\nEvent {win}\n\nLose\nEvent {lose}\n")
        );
        assert!(refused.is_empty(), "{refused:?}");
    }
}

#[test]
fn hookless_tail_needs_no_country_links_and_outcomes_keep_native_presence_gates() {
    let mut border = LandBorderWarInputs {
        sides: [
            LandBorderWarSide {
                country: 7,
                ..Default::default()
            },
            LandBorderWarSide {
                country: 9,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    assert_eq!(tail_side(&border, 70, &BTreeMap::new()), Ok(None));
    let links = [(7, 70), (9, 90)].into_iter().collect();
    for side in 0..2 {
        for win in [true, false] {
            let id = if win {
                &mut border.sides[side].win_event
            } else {
                &mut border.sides[side].lose_event
            };
            *id = b"synthetic_hook".to_vec();
            assert_eq!(tail_side(&border, 70, &links), Ok(Some(0)));
            assert_eq!(tail_side(&border, 90, &links), Ok(Some(1)));
            let mut body = "Marker".into();
            let mut refused = BTreeMap::new();
            build(
                &NoIcons,
                &localisation(),
                &border,
                0,
                &mut body,
                &|_, _| Ok("Enemy".into()),
                &|_| unreachable!(),
                &|_, _, _| Description {
                    body: "Event\n".into(),
                    ..Default::default()
                },
                &mut refused,
            )
            .unwrap();
            assert_eq!(body.contains("Win"), win == (side == 0));
            assert_eq!(body.contains("Lose"), win != (side == 0));
            assert!(refused.is_empty());
            border.sides[side].win_event.clear();
            border.sides[side].lose_event.clear();
        }
    }
    border.transfer_state = 128;
    assert_eq!(tail_side(&border, 70, &links), Ok(Some(0)));
}

struct Folder(PathBuf);
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn with_effects(loc: &Localisation, run: impl FnOnce(&EffectContext<'_, '_>)) {
    let root = std::env::temp_dir().join(format!(
        "hoi4_border_effect_adapter_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _folder = Folder(root.clone());
    std::fs::create_dir_all(root.join("common/defines")).unwrap();
    std::fs::create_dir_all(root.join("common/buildings")).unwrap();
    std::fs::create_dir_all(root.join("map")).unwrap();
    std::fs::write(
        root.join("common/defines/synthetic.lua"),
        "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 }, NBuildings = { MAX_BUILDING_LEVELS = 10, INFRASTRUCTURE_RESOURCE_BONUS = 0, SUPPLY_ROUTE_RESOURCE_BONUS = 0 } }",
    )
    .unwrap();
    std::fs::write(
        root.join("common/buildings/synthetic.txt"),
        "buildings={synthetic={}}",
    )
    .unwrap();
    std::fs::write(
        root.join("map/default.map"),
        "definitions=\"definition.csv\" provinces=\"p.bmp\" terrain=\"t.bmp\" rivers=\"r.bmp\" heightmap=\"h.bmp\" tree_definition=\"trees.bmp\"",
    )
    .unwrap();
    std::fs::write(
        root.join("map/definition.csv"),
        "0;0;0;0;land;false;unknown;0\n1;1;1;1;land;false;synthetic;1\n",
    )
    .unwrap();
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    let content = dashboard_base::catalogue::read_scripted_content(
        Arc::new(content_reader_layers::content::ResolvedContent {
            layers,
            stats: Default::default(),
            changed: Vec::new(),
            notes: Vec::new(),
        }),
        "english".into(),
    )
    .unwrap();
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 10,
        tables: std::array::from_fn(|i| {
            replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::empty(
                replay_data::rows::TableId::ALL[i],
            ))
            .into()
        }),
        metrics: Default::default(),
    };
    for (id, tag) in [(7, "AAA"), (9, "BBB")] {
        let replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::Countries(rows)) =
            &mut *frame.tables[replay_data::rows::TableId::Countries.index()]
        else {
            unreachable!()
        };
        rows.push(replay_data::rows::CountryRow {
            id,
            canonical_slot: id,
            tag: tag.as_bytes().to_vec(),
            ..Default::default()
        });
        let replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::CountryLinks(
            rows,
        )) = &mut *frame.tables[replay_data::rows::TableId::CountryLinks.index()]
        else {
            unreachable!()
        };
        rows.push(replay_data::rows::CountryLinkRow {
            id,
            canonical_slot: id,
        });
    }
    let scripted = ScriptedText::new(&frame, loc, &content, 7);
    let context = EffectContext::new(&scripted, loc, 7, "synthetic event", Default::default());
    run(&context);
}

#[test]
fn border_marker_event_native_effects_or_title_follow_option_count() {
    content_reader_script::test_game::enter();
    let events = content_reader_events::events::parse(b"add_namespace=s country_event={id=s.1 title={text=IGNORED text=TITLE trigger={always=no}} immediate={custom_effect_tooltip=FIRST hidden_effect={custom_effect_tooltip=HIDDEN} add_political_power=2} option={name=A effect_tooltip={custom_effect_tooltip=SECOND}}}", "events/synthetic.txt", &content_reader_script::key_names::KeyNames::default());
    let mut event = events.get(b"S.01").unwrap().clone();
    let base = localisation();
    let loc = base
        .iter()
        .map(|(key, text)| (key.into(), text.into()))
        .chain(
            [
                ("FIRST", "Text FIRST"),
                ("HIDDEN", "Text HIDDEN"),
                ("SECOND", "Text SECOND"),
                ("POLITICS_ADD_POLITICAL_POWER", "PP $VAL|=+0$"),
            ]
            .into_iter()
            .map(|(key, text)| (key.into(), text.into())),
        )
        .collect();
    with_effects(&loc, |context| {
        let calls = Cell::new(0);
        let describe = |items: &[Item], graph: &mut EventScopes| {
            calls.set(calls.get() + 1);
            effect_description::describe(items, context, graph)
        };
        let render =
            |key: &[u8], _: &EventScopes| Ok(format!("Text {}", String::from_utf8_lossy(key)));
        let mut graph = scopes(7, 11, 9, 13);
        let text = event_description(
            &event, false, &describe, &mut graph, &render, &NoIcons, &loc,
        );
        assert!(text.refused.is_empty(), "{:?}", text.refused);
        assert_eq!(text.body, "Text FIRST\nPP \u{11}G+2\u{11}!\nText SECOND\n");
        assert_eq!(calls.get(), 2);
        let text = event_description(&event, true, &describe, &mut graph, &render, &NoIcons, &loc);
        assert_eq!(text.body, "Text FIRST\nPP \u{11}G+2\u{11}!\nText SECOND\n");
        assert!(text.refused.is_empty());
        assert_eq!(calls.get(), 4);
        let mut zero_options = event.clone();
        zero_options.options.clear();
        let text = event_description(
            &zero_options,
            true,
            &describe,
            &mut graph,
            &render,
            &NoIcons,
            &loc,
        );
        assert_eq!(
            text.body,
            "Text FIRST\nPP \u{11}G+2\u{11}!\nEvent Text TITLE"
        );
        assert!(text.refused.is_empty());
        assert_eq!(calls.get(), 5);
        event.options.push(Vec::new());
        let text = event_description(
            &event, false, &describe, &mut graph, &render, &NoIcons, &loc,
        );
        assert_eq!(text.body, "Event Text TITLE");
        assert!(text.refused.is_empty());
        assert_eq!(calls.get(), 5);
        let text = event_description(
            &event,
            true,
            &describe,
            &mut graph,
            &|_, _| unreachable!(),
            &NoIcons,
            &loc,
        );
        assert!(text.body.is_empty());
        assert!(text.refused.is_empty());
        assert_eq!(calls.get(), 5);
        event.titles.push(event.titles[0].clone());
        let text = event_description(
            &event, false, &describe, &mut graph, &render, &NoIcons, &loc,
        );
        assert_eq!(text.body, "Event Text TITLE");
        assert!(text.refused.is_empty());
        event.titles[1].value = content_reader_parser::parser::Value::Scalar(b"OTHER_TITLE".to_vec());
        let text = event_description(
            &event, false, &describe, &mut graph, &render, &NoIcons, &loc,
        );
        assert!(text.body.is_empty());
        assert!(
            text.refused
                .keys()
                .next()
                .unwrap()
                .contains("high-resolution seed")
        );
        assert_eq!(calls.get(), 5);
    });
}

#[test]
fn border_marker_empty_enemy_description_has_no_enemy_heading_and_counts_failure() {
    let border = LandBorderWarInputs {
        sides: [
            LandBorderWarSide::default(),
            LandBorderWarSide {
                lose_event: b"failed".to_vec(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let mut body = String::new();
    let mut refused = BTreeMap::new();
    build(
        &NoIcons,
        &localisation(),
        &border,
        0,
        &mut body,
        &|_, _| unreachable!(),
        &|_| unreachable!(),
        &|_, _, _| Description {
            refused: [("synthetic refusal".into(), 3)].into_iter().collect(),
            ..Default::default()
        },
        &mut refused,
    )
    .unwrap();
    assert_eq!(body, "\nWin\n");
    assert_eq!(refused["synthetic refusal"], 3);
}

#[test]
fn covered_events_keep_supported_operative_text_and_refuse_missing_scoped_readers() {
    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!(
        "hoi4-covered-border-events-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _folder = Folder(root.clone());
    std::fs::create_dir_all(root.join("events")).unwrap();
    std::fs::write(
        root.join("events/synthetic.txt"),
        b"add_namespace=s country_event={id=s.1 immediate={custom_effect_tooltip=TEXT}} operative_leader_event={id=s.2 immediate={custom_effect_tooltip=TEXT}} operative_leader_event={id=s.3 title=TITLE option={name=A hidden_effect={remove_unit_leader_role={character=fixture}}}}",
    )
    .unwrap();
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    let events = content_reader_events::events::LazyEvents::new(Arc::new(layers));
    let render = |key: &[u8], _: &EventScopes| Ok(String::from_utf8_lossy(key).into_owned());
    let loc = localisation();
    with_effects(&loc, |context| {
        let describe = |items: &[Item], graph: &mut EventScopes| {
            effect_description::describe(items, context, graph)
        };
        let mut graph = scopes(7, 11, 9, 13);
        let text = event_description(
            &events.get(b"s.1").unwrap(),
            false,
            &describe,
            &mut graph,
            &render,
            &NoIcons,
            &loc,
        );
        assert_eq!(text.body, "TEXT\n");
        assert!(text.refused.is_empty());
        let text = event_description(
            &events.get(b"s.2").unwrap(),
            false,
            &describe,
            &mut graph,
            &render,
            &NoIcons,
            &loc,
        );
        assert_eq!(text.body, "TEXT\n");
        assert!(text.refused.is_empty());
        let text = event_description(
            &events.get(b"s.3").unwrap(),
            false,
            &describe,
            &mut graph,
            &render,
            &NoIcons,
            &loc,
        );
        assert!(text.body.is_empty());
        assert_eq!(text.refused.values().sum::<usize>(), 1);
        assert!(
            text.refused
                .keys()
                .any(|key| key.contains("scoped-object parser helper"))
        );
    });
}
