use super::*;
use replay_capture_spool::commands;
use replay_recording::{command_packet::{Actor, Batch, Category, Event, Kind, Origin, Packet, Span}, control::ControlRecord};

const HOURS: i32 = 400;

fn recording(per_hour: impl Fn(i32) -> u64) -> Arc<Index> {
    let mut session = dashboard_base::test_fixtures::session(7);
    session.qpc_frequency = 1;
    let actor = |id: u64, tag: &[u8]| Actor { id, player: id as i32, country: id as u32, since_qpc: 0, at_qpc: 0, user: Vec::new(), name: b"player".to_vec(), tag: tag.to_vec() };
    let mut events = Vec::new();
    let mut spans = Vec::new();
    let mut sequence = 0;
    for hour in 0..HOURS {
        let from = hour as u64 * 60;
        let paused = hour % 10 == 9;
        spans.push(Span { from_qpc: from, to_qpc: from + 60, complete: true, hour, paused });
        for at in 0..per_hour(hour) {
            sequence += 1;
            let player = 1 + at % 2;
            events.push(Event { qpc: from + at % 60, hour, actor: player, player: player as i32, kind: 1, command_country: Some(player as u32),
                origin: Origin::PlayerOrder, sequence, paused });
        }
    }
    let batch = Batch {
        kinds: vec![Kind { id: 1, token: 1, name: "synthetic order".into(), category: Category::Army }],
        actors: vec![actor(1, b"GER"), actor(2, b"ITA")],
        events,
        spans,
        ..Default::default()
    };
    let packet = ControlRecord::PlayerCommands { session: 7, packet: Packet::pack(4, 1, 2, 0, 0, HOURS as u64 * 60, 0, &batch).unwrap() };
    Arc::new(commands::index(&[session], &[packet], 4).unwrap())
}

fn followed(index: &Arc<Index>) -> ApmHead {
    ApmHead::new(Arc::clone(index), &ApmHeadParams {
        branch_id: "4".into(), group: ApmGroup::Country, tags: None, membership: None, players: Vec::new(), first_hour: 0, last_hour: HOURS - 1, window_hours: 168,
    })
    .unwrap()
}

#[test]
fn a_followed_eapm_is_the_request_paths_summary_of_the_trailing_window_at_every_hour() {
    let index = recording(|hour| (hour % 7) as u64);
    let mut head = followed(&index);
    for hour in [0, 9, 10, 100, 167, 168, 169, 250, 333, HOURS - 1] {
        let (first, last) = head.window_at(hour).unwrap();
        let amounts = head.amounts(first, last);
        let reply = query(&index, &ApmParams { branch_id: "4".into(), group: ApmGroup::Country, registry: false, first_hour: Some(first), last_hour: Some(last),
            tags: None, membership: None, players: Vec::new(), buckets: 1 }, &AtomicBool::new(false)).unwrap();
        assert_eq!(amounts.len(), reply.series.len(), "the series at hour {hour}");
        for series in &reply.series {
            let at = head.ids.binary_search(&series.id).expect("a laid series");
            let played = &amounts[&at];
            assert_eq!(serde_json::to_value(played.amount.value()).unwrap(), serde_json::to_value(&series.summary).unwrap(), "{} at hour {hour}", series.id);
            assert_eq!(played.tag, series.tag.clone().unwrap());
            assert_eq!(played.amount.last_hour, Some(series.hour));
        }
    }
}

#[test]
fn player_display_tag_uses_only_versions_present_in_the_window() {
    let mut session = dashboard_base::test_fixtures::session(9);
    session.qpc_frequency = 1;
    let actor = |at_qpc: u64, tag: &[u8]| Actor {
        id: 1,
        player: 1,
        country: 1,
        since_qpc: 0,
        at_qpc,
        user: Vec::new(),
        name: b"switcher".to_vec(),
        tag: tag.to_vec(),
    };
    let spans: Vec<Span> = (0..60)
        .map(|hour| Span { from_qpc: hour as u64 * 60, to_qpc: (hour as u64 + 1) * 60, complete: true, hour, paused: false })
        .collect();
    let batch = Batch {
        kinds: vec![Kind { id: 1, token: 1, name: "synthetic order".into(), category: Category::Army }],
        actors: vec![actor(0, b"GER"), actor(10 * 60, b"ITA"), actor(40 * 60, b"GER")],
        events: vec![
            Event { qpc: 2 * 60, hour: 2, actor: 1, player: 1, kind: 1, command_country: Some(1), origin: Origin::PlayerOrder, sequence: 1, paused: false },
            Event { qpc: 15 * 60, hour: 15, actor: 1, player: 1, kind: 1, command_country: Some(1), origin: Origin::PlayerOrder, sequence: 2, paused: false },
            Event { qpc: 45 * 60, hour: 45, actor: 1, player: 1, kind: 1, command_country: Some(1), origin: Origin::PlayerOrder, sequence: 3, paused: false },
        ],
        spans,
        ..Default::default()
    };
    let packet = ControlRecord::PlayerCommands { session: 9, packet: Packet::pack(4, 1, 2, 0, 0, 60 * 60, 0, &batch).unwrap() };
    let index = Arc::new(commands::index(&[session], &[packet], 4).unwrap());
    let mut head = ApmHead::new(Arc::clone(&index), &ApmHeadParams {
        branch_id: "4".into(),
        group: ApmGroup::Player,
        tags: None,
        membership: None,
        players: Vec::new(),
        first_hour: 0,
        last_hour: 59,
        window_hours: 30,
    })
    .unwrap();
    let amounts = head.amounts(0, 25);
    let reply = query(&index, &ApmParams {
        branch_id: "4".into(),
        group: ApmGroup::Player,
        registry: false,
        first_hour: Some(0),
        last_hour: Some(25),
        tags: None,
        membership: None,
        players: Vec::new(),
        buckets: 1,
    }, &AtomicBool::new(false))
    .unwrap();
    assert_eq!(reply.series.len(), 1);
    assert_eq!(reply.series[0].tag.as_deref(), Some("ITA"));
    assert_eq!(amounts.values().next().unwrap().tag, "ITA");
}

#[test]
fn player_apm_head_carries_dated_name_long_utf8_tag_and_explicit_missing_name_reason() {
    let mut session = dashboard_base::test_fixtures::session(12);
    session.qpc_frequency = 1;
    let mk_actor = |id: u64, at_qpc: u64, name: &[u8], tag: &[u8]| Actor {
        id,
        player: id as i32,
        country: id as u32,
        since_qpc: 0,
        at_qpc,
        user: format!("u{id}").into_bytes(),
        name: name.to_vec(),
        tag: tag.to_vec(),
    };
    let spans: Vec<Span> = (0..40)
        .map(|hour| Span { from_qpc: hour as u64 * 60, to_qpc: (hour as u64 + 1) * 60, complete: true, hour, paused: false })
        .collect();
    let long_tag = b"CUSTOM_MOD_TAG_12345";
    let batch = Batch {
        kinds: vec![Kind { id: 1, token: 1, name: "synthetic order".into(), category: Category::Army }],
        actors: vec![
            mk_actor(1, 0, b"EarlyName", long_tag),
            mk_actor(1, 25 * 60, b"LateName", long_tag),
            mk_actor(2, 0, &[0xff, 0xfe], b"GER"),
            mk_actor(3, 0, b"   ", b"FRA"),
        ],
        events: vec![
            Event { qpc: 5 * 60, hour: 5, actor: 1, player: 1, kind: 1, command_country: Some(1), origin: Origin::PlayerOrder, sequence: 1, paused: false },
            Event { qpc: 5 * 60, hour: 5, actor: 2, player: 2, kind: 1, command_country: Some(2), origin: Origin::PlayerOrder, sequence: 2, paused: false },
            Event { qpc: 5 * 60, hour: 5, actor: 3, player: 3, kind: 1, command_country: Some(3), origin: Origin::PlayerOrder, sequence: 3, paused: false },
            Event { qpc: 30 * 60, hour: 30, actor: 1, player: 1, kind: 1, command_country: Some(1), origin: Origin::PlayerOrder, sequence: 4, paused: false },
        ],
        spans,
        ..Default::default()
    };
    let packet = ControlRecord::PlayerCommands { session: 12, packet: Packet::pack(4, 1, 2, 0, 0, 40 * 60, 0, &batch).unwrap() };
    let index = Arc::new(commands::index(&[session], &[packet], 4).unwrap());
    let mut query_head = ApmHead::new(Arc::clone(&index), &ApmHeadParams {
        branch_id: "4".into(),
        group: ApmGroup::Player,
        tags: None,
        membership: None,
        players: Vec::new(),
        first_hour: 0,
        last_hour: 39,
        window_hours: 15,
    })
    .unwrap();
    let amounts_early = query_head.amounts(0, 14);
    let reply_early = query(&index, &ApmParams {
        branch_id: "4".into(),
        group: ApmGroup::Player,
        registry: false,
        first_hour: Some(0),
        last_hour: Some(14),
        tags: None,
        membership: None,
        players: Vec::new(),
        buckets: 1,
    }, &AtomicBool::new(false))
    .unwrap();
    let p1 = reply_early.series.iter().find(|s| s.id == "user:7531").unwrap();
    let p2 = reply_early.series.iter().find(|s| s.id == "user:7532").unwrap();
    let p3 = reply_early.series.iter().find(|s| s.id == "user:7533").unwrap();
    assert_eq!(p1.name.as_deref(), Some("EarlyName"));
    assert_eq!(p1.tag.as_deref(), Some("CUSTOM_MOD_TAG_12345"));
    assert_eq!(p2.name, None);
    assert_eq!(p2.name_missing.as_deref(), Some("Player name is not valid UTF-8."));
    assert_eq!(p3.name, None);
    assert_eq!(p3.name_missing.as_deref(), Some("Player name was not recorded."));

    let p1_idx = query_head.ids.binary_search(&"user:7531".to_owned()).unwrap();
    let p2_idx = query_head.ids.binary_search(&"user:7532".to_owned()).unwrap();
    assert_eq!(amounts_early[&p1_idx].name.as_deref(), Some("EarlyName"));
    assert_eq!(amounts_early[&p1_idx].tag, "CUSTOM_MOD_TAG_12345");
    assert_eq!(amounts_early[&p2_idx].name, None);
    assert_eq!(amounts_early[&p2_idx].name_missing.as_deref(), Some("Player name is not valid UTF-8."));

    let amounts_late = query_head.amounts(25, 39);
    assert_eq!(amounts_late[&p1_idx].name.as_deref(), Some("LateName"));
}

#[test]
fn player_apm_ranks_by_apm_descending_and_tiles_accept_countries_with_unrecorded_reason() {
    let mut session = dashboard_base::test_fixtures::session(15);
    session.qpc_frequency = 1;
    let mk_actor = |id: u64, tag: &[u8], name: &[u8]| Actor {
        id,
        player: id as i32,
        country: id as u32,
        since_qpc: 0,
        at_qpc: 0,
        user: format!("p{id}").into_bytes(),
        name: name.to_vec(),
        tag: tag.to_vec(),
    };
    let spans: Vec<Span> = (0..20)
        .map(|hour| Span { from_qpc: hour as u64 * 60, to_qpc: (hour as u64 + 1) * 60, complete: true, hour, paused: false })
        .collect();
    let mut events = Vec::new();
    for seq in 0..10u64 {
        events.push(Event { qpc: 60 + seq, hour: 1, actor: 2, player: 2, kind: 1, command_country: Some(2), origin: Origin::PlayerOrder, sequence: seq as u32 + 1, paused: false });
    }
    events.push(Event { qpc: 70, hour: 1, actor: 1, player: 1, kind: 1, command_country: Some(1), origin: Origin::PlayerOrder, sequence: 99, paused: false });
    let batch = Batch {
        kinds: vec![Kind { id: 1, token: 1, name: "synthetic order".into(), category: Category::Army }],
        actors: vec![mk_actor(1, b"GER", b"Slow"), mk_actor(2, b"ITA", b"Fast")],
        events,
        spans,
        ..Default::default()
    };
    let packet = ControlRecord::PlayerCommands { session: 15, packet: Packet::pack(4, 1, 2, 0, 0, 20 * 60, 0, &batch).unwrap() };
    let index = Arc::new(commands::index(&[session], &[packet], 4).unwrap());
    let reply = query(&index, &ApmParams {
        branch_id: "4".into(),
        group: ApmGroup::Player,
        registry: false,
        first_hour: Some(0),
        last_hour: Some(19),
        tags: None,
        membership: None,
        players: Vec::new(),
        buckets: 1,
    }, &AtomicBool::new(false))
    .unwrap();
    let ordered: Vec<&str> = reply.series.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ordered, ["user:7032", "user:7031"]);

    let ask: TileAsk = dashboard_query::named::named(&serde_json::json!({ "binHours": 1, "index": 0, "countries": ["user:7032", "user:7031"] })).unwrap();
    let family = TileFamily { group: ApmGroup::Player, tags: None, membership: None, players: Vec::new(), value: "apm".into() };
    let (first, last) = tile::hours(ask.bin_hours, ask.index).unwrap();
    let tile_reply = tile_from_index(&index, 4, 19, &family, &ask, first, last).unwrap();
    assert_eq!(tile_reply.lines.len(), 2);
    assert_eq!(tile_reply.lines[0].country.key(), "user:7032");
    assert!(tile_reply.lines[0].missing.is_empty());

    let empty = Arc::new(commands::index(&[], &[], 4).unwrap());
    let unrecorded = tile_from_index(&empty, 4, 19, &family, &ask, first, last).unwrap();
    assert_eq!(unrecorded.lines.len(), 2);
    assert_eq!(unrecorded.lines[0].missing.len(), 1);
    assert_eq!(unrecorded.lines[0].missing[0].reason, "Player orders were not recorded in this campaign.");
}
