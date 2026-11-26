use super::*;
use super::super::{ApmParams, query};
use replay_capture_spool::commands;
use replay_recording::{command_packet::{Actor, Batch, Category, Event, Kind, Origin, Packet, Span}, control::ControlRecord};

const HOURS: i32 = 400;

fn recording(per_hour: impl Fn(i32) -> u64) -> Arc<Index> {
    let mut session = dashboard_base::test_fixtures::session(7);
    session.qpc_frequency = 7;
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
    let packet = ControlRecord::PlayerCommands { session: 7, packet: Box::new(Packet::pack(4, 1, 2, 0, 0, HOURS as u64 * 60, 0, &batch).unwrap()) };
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
        head.update_window(hour, false).unwrap();
        let reply = query(&index, &ApmParams { branch_id: "4".into(), group: ApmGroup::Country, registry: false, hour: None, first_hour: Some(first), last_hour: Some(last),
            tags: None, membership: None, players: Vec::new(), buckets: 1 }, &AtomicBool::new(false)).unwrap();
        assert_eq!(head.ranking.len(), reply.series.len(), "the series at hour {hour}");
        for (rank, series) in head.ranking.iter().zip(reply.series) {
            assert_eq!(head.ids[rank.series], series.id);
            let participant = &head.participants[&rank.series];
            assert_eq!(&participant.cached.as_ref().unwrap().1, &series.summary, "{} at hour {hour}", series.id);
            assert_eq!(participant.identity().tag, series.tag.unwrap());
            assert_eq!(participant.total.last_hour, Some(series.hour));
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
    let packet = ControlRecord::PlayerCommands { session: 9, packet: Box::new(Packet::pack(4, 1, 2, 0, 0, 60 * 60, 0, &batch).unwrap()) };
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
    head.update_window(25, true).unwrap();
    let reply = query(&index, &ApmParams {
        branch_id: "4".into(),
        group: ApmGroup::Player,
        registry: false,
        hour: None,
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
    assert_eq!(head.participants[&0].identity().tag, "ITA");
    for hour in 26..=41 {
        head.update_window(hour, false).unwrap();
        assert_eq!(head.participants[&0].identity().tag, if hour < 40 { "ITA" } else { "GER" });
    }
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
    let packet = ControlRecord::PlayerCommands { session: 12, packet: Box::new(Packet::pack(4, 1, 2, 0, 0, 40 * 60, 0, &batch).unwrap()) };
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
    query_head.update_window(14, true).unwrap();
    let reply_early = query(&index, &ApmParams {
        branch_id: "4".into(),
        group: ApmGroup::Player,
        registry: false,
        hour: None,
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
    assert_eq!(query_head.participants[&p1_idx].identity().name.as_deref(), Some("EarlyName"));
    assert_eq!(query_head.participants[&p1_idx].identity().tag, "CUSTOM_MOD_TAG_12345");
    assert_eq!(query_head.participants[&p2_idx].identity().name, None);
    assert_eq!(query_head.participants[&p2_idx].identity().name_missing.as_deref(), Some("Player name is not valid UTF-8."));

    query_head.update_window(39, false).unwrap();
    assert_eq!(query_head.participants[&p1_idx].identity().name.as_deref(), Some("LateName"));
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
    let packet = ControlRecord::PlayerCommands { session: 15, packet: Box::new(Packet::pack(4, 1, 2, 0, 0, 20 * 60, 0, &batch).unwrap()) };
    let index = Arc::new(commands::index(&[session], &[packet], 4).unwrap());
    let reply = query(&index, &ApmParams {
        branch_id: "4".into(),
        group: ApmGroup::Player,
        registry: false,
        hour: None,
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

#[test]
fn multi_hour_apm_tile_preserves_hourly_min_max_endpoints_and_gap_breaks() {
    let mut session = dashboard_base::test_fixtures::session(21);
    session.qpc_frequency = 1;
    let actor = Actor {
        id: 1,
        player: 1,
        country: 1,
        since_qpc: 0,
        at_qpc: 0,
        user: b"p1".to_vec(),
        name: b"Burst".to_vec(),
        tag: b"GER".to_vec(),
    };
    let spans = vec![
        Span { from_qpc: 0, to_qpc: 60, complete: true, hour: 0, paused: false },
        Span { from_qpc: 60, to_qpc: 120, complete: true, hour: 1, paused: false },
        Span { from_qpc: 240, to_qpc: 300, complete: true, hour: 4, paused: false },
    ];
    let mut events = Vec::new();
    for seq in 0..20u64 {
        events.push(Event {
            qpc: 60 + seq,
            hour: 1,
            actor: 1,
            player: 1,
            kind: 1,
            command_country: Some(1),
            origin: Origin::PlayerOrder,
            sequence: seq as u32 + 1,
            paused: false,
        });
    }
    let batch = Batch {
        kinds: vec![Kind { id: 1, token: 1, name: "synthetic order".into(), category: Category::Army }],
        actors: vec![actor],
        events,
        spans,
        ..Default::default()
    };
    let packet = ControlRecord::PlayerCommands { session: 21, packet: Box::new(Packet::pack(4, 1, 2, 0, 0, 300, 0, &batch).unwrap()) };
    let index = Arc::new(commands::index(&[session], &[packet], 4).unwrap());

    let h0 = query(&index, &ApmParams {
        branch_id: "4".into(), group: ApmGroup::Player, registry: false, hour: Some(0), first_hour: None, last_hour: None,
        tags: None, membership: None, players: Vec::new(), buckets: 1,
    }, &AtomicBool::new(false)).unwrap();
    let h1 = query(&index, &ApmParams {
        branch_id: "4".into(), group: ApmGroup::Player, registry: false, hour: Some(1), first_hour: None, last_hour: None,
        tags: None, membership: None, players: Vec::new(), buckets: 1,
    }, &AtomicBool::new(false)).unwrap();
    assert_eq!(h0.series[0].summary.apm, Some(0.0));
    assert_eq!(h1.series[0].summary.apm, Some(20.0));

    let wanted = BTreeSet::from(["user:7031"]);
    let per_hour = hourly_amounts(&index, ApmGroup::Player, 0, 5, None, None, &[], &wanted).unwrap();
    let p1 = &per_hour["user:7031"];
    assert_eq!(p1[&0].value().apm, Some(0.0));
    assert_eq!(p1[&1].value().apm, Some(20.0));

    let ask: TileAsk = dashboard_query::named::named(&serde_json::json!({ "binHours": 2, "index": 0, "countries": ["user:7031"] })).unwrap();
    let family = TileFamily { group: ApmGroup::Player, tags: None, membership: None, players: Vec::new(), value: "apm".into() };
    let (first, last) = tile::hours(ask.bin_hours, ask.index).unwrap();
    let tile_reply = tile_from_index(&index, 4, 5, &family, &ask, first, last).unwrap();
    assert!(tile_reply.lines[0].missing.iter().any(|m| m.first_hour == 5 && m.reason == GAP_REASON));
}

#[test]
fn apm_played_hours_update_only_entering_and_expiring_slices_and_ranks() {
    use dashboard_base::test_fixtures::{played, replay, source};
    use dashboard_follow::panel::{AtHour, ProductRead};
    use std::sync::Mutex;

    let index = recording(|hour| (hour % 5) as u64);
    let membership = vec![
        ApmMembership { tag: "GER".into(), first_hour: 0, last_hour: HOURS - 1 },
        ApmMembership { tag: "ITA".into(), first_hour: 50, last_hour: 120 },
    ];
    let params = ApmHeadParams {
        branch_id: "4".into(), group: ApmGroup::Country, tags: None, membership: Some(membership.clone()), players: vec![],
        first_hour: 0, last_hour: HOURS - 1, window_hours: 168,
    };
    let head = Mutex::new(ApmHead::new(Arc::clone(&index), &params).unwrap());
    let panel = AtHour {
        cold: Arc::new(|_, _| unreachable!()), read: Arc::new(|_, _| unreachable!()),
        advance: Arc::new(move |hour, reset, _, _| {
            let mut head = head.lock().unwrap();
            let before = head.participants_ranked;
            head.update_window(hour, reset).unwrap();
            let rows: Vec<_> = head.ranking.iter().map(|rank| {
                let mut value = head.participants[&rank.series].cached.as_ref().unwrap().1.clone();
                value.gaps = !head.cached_whole;
                (&head.ids[rank.series], value)
            }).collect();
            let value = serde_json::to_value(rows).unwrap();
            ProductRead { value: Arc::new(Ok(value)), rows: (head.participants_ranked - before) as u64, fresh: true, pending: false, live: None, lanes: vec![] }
        }),
    };
    let replay = replay((170..=176).map(|hour| source(1, hour, None)).collect()).into_timelines(&Default::default()).unwrap();
    let reads = played(&replay, &panel, 170, 176);
    let mut costs = Vec::new();
    for read in reads.iter().filter(|read| read.hour > 170) {
        let actual = read.value.as_ref().as_ref().unwrap();
        let exact = query(&index, &ApmParams {
            branch_id: "4".into(), group: ApmGroup::Country, registry: false, hour: None, first_hour: Some(read.hour - 167), last_hour: Some(read.hour),
            tags: None, membership: Some(membership.clone()), players: vec![], buckets: 1,
        }, &AtomicBool::new(false)).unwrap();
        let expected: Vec<_> = exact.series.iter().map(|row| (&row.id, &row.summary)).collect();
        assert_eq!(actual, &serde_json::to_value(expected).unwrap(), "rank and exact operands at {}", read.hour);
        costs.push((read.hour, read.visited, read.rows));
    }
    assert_eq!(costs.len(), 6);
    assert!(costs.iter().all(|&(_, slices, ranked)| slices == 2 && ranked <= 1), "(hour, participant slices summed, participants ranked): {costs:?}");
}

#[test]
fn expiring_a_slice_restores_the_remaining_clock_quality_and_latest_observation() {
    let mut player = WindowSeries::default();
    for (hour, session, minutes, incomplete) in [
        (5, 2, Minutes { num: 1, den: 3 }, true),
        (6, 1, Minutes { num: 1, den: 5 }, false),
    ] {
        player.enter(hour, HourSlice { shown: Some((session, 0)), minutes, incomplete, paused_incomplete: incomplete, ..Default::default() }).unwrap();
    }
    assert_eq!(player.identity().shown, Some((2, 0)));
    assert!(!player.summary().complete);
    player.expire().unwrap();
    let value = player.summary();
    assert_eq!(player.identity().shown, Some((1, 0)));
    assert_eq!(value.unpaused_minutes_num.as_deref(), Some("1"));
    assert_eq!(value.unpaused_minutes_den.as_deref(), Some("5"));
    assert_eq!(value.paused_orders.as_deref(), Some("0"));
}

fn coverage_fixture() -> Index {
    let mut session = dashboard_base::test_fixtures::session(7);
    session.qpc_frequency = 1;
    let packet = |capture, start, hours: &[i32]| {
        let spans: Vec<_> = hours.iter().enumerate().map(|(i, &hour)| {
            let from_qpc = start + i as u64 * 60;
            Span { from_qpc, to_qpc: from_qpc + 60, hour, complete: true, paused: hour == 1 }
        }).collect();
        let end = spans.last().unwrap().to_qpc;
        let actor = Actor { id: 1, player: 1, country: 1, since_qpc: start, at_qpc: start,
            user: vec![], name: b"Sender".to_vec(), tag: b"AAA".to_vec() };
        let batch = Batch { kinds: vec![Kind { id: 1, token: 1, name: "synthetic order".into(), category: Category::Army }], actors: vec![actor], spans, ..Default::default() };
        ControlRecord::PlayerCommands { session: 7, packet: Box::new(Packet::pack(4, capture, 2, 0, start, end, 0, &batch).unwrap()) }
    };
    commands::index(&[session], &[packet(1, 0, &[0, 1, 2, 3]), packet(2, 600, &[3, 4, 5, 12])], 4).unwrap()
}

fn coverage_tile(index: &Index) -> SeriesTileReply {
    let family = TileFamily { group: ApmGroup::Country, tags: None, membership: None, players: vec![], value: "apm".into() };
    let ask = TileAsk { bin_hours: 4, index: 0, countries: vec!["AAA".into(), "BBB".into()] };
    let (first, last) = tile::hours(ask.bin_hours, ask.index).unwrap();
    tile_from_index(index, 4, 12, &family, &ask, first, last).unwrap()
}

#[test]
fn apm_tile_coverage_names_a_completely_uncovered_bin() {
    for line in coverage_tile(&coverage_fixture()).lines {
        assert!(line.missing.iter().any(|m| m.first_hour == 6 && m.last_hour == 11 && m.reason == GAP_REASON), "{:?}", line.missing);
    }
}

#[test]
fn apm_tile_coverage_preserves_a_paused_hour_between_active_hours() {
    let tile = coverage_tile(&coverage_fixture());
    assert!(tile.lines[0].missing.iter().any(|m| m.first_hour == 1 && m.last_hour == 1 && m.reason == "No unpaused time was observed in this period."));
    assert!(!tile.lines[0].missing.iter().any(|m| m.first_hour <= 2 && m.last_hour >= 2));
}

#[test]
fn apm_tile_coverage_marks_the_same_reattachment_hour_as_the_exact_product() {
    let index = coverage_fixture();
    let tile = coverage_tile(&index);
    for hour in 2..=4 {
        let reply = query(&index, &ApmParams {
            branch_id: "4".into(), group: ApmGroup::Country, registry: false, hour: Some(hour), first_hour: None, last_hour: None,
            tags: None, membership: None, players: vec![], buckets: 1,
        }, &AtomicBool::new(false)).unwrap();
        assert_eq!(reply.series[0].summary.gaps, hour == 3);
        assert_eq!(tile.lines[0].missing.iter().any(|m| m.first_hour <= hour && hour <= m.last_hour && m.reason == GAP_REASON), reply.series[0].summary.gaps, "hour {hour}");
    }
}

#[test]
fn an_unrepresentable_clock_sum_is_reported_by_the_exact_tile_and_followed_reads() {
    let frequencies = [1_000_000_007, 1_000_000_009, 998_244_353, 1_004_535_809, 469_762_049];
    let mut sessions = Vec::new();
    let mut packets = Vec::new();
    let batch = Batch {
        actors: vec![Actor { id: 1, player: 1, country: 1, at_qpc: 0, since_qpc: 0, user: vec![], name: b"Sender".to_vec(), tag: b"AAA".to_vec() }],
        kinds: vec![Kind { id: 1, token: 1, name: "synthetic order".into(), category: Category::Army }],
        spans: vec![Span { hour: 0, from_qpc: 0, to_qpc: 60, complete: true, paused: false }],
        ..Default::default()
    };
    for (id, frequency) in frequencies.into_iter().enumerate() {
        let mut session = dashboard_base::test_fixtures::session(id as u64 + 1);
        session.qpc_frequency = frequency;
        packets.push(ControlRecord::PlayerCommands { session: session.id, packet: Packet::pack(4, 1, 2, 0, 0, 60, 0, &batch).unwrap() });
        sessions.push(session);
    }
    let index = Arc::new(commands::index(&sessions, &packets, 4).unwrap());
    let params = ApmParams { branch_id: "4".into(), group: ApmGroup::Country, registry: false, hour: Some(0),
        first_hour: None, last_hour: None, tags: None, membership: None, players: vec![], buckets: 1 };
    assert_eq!(query(&index, &params, &false.into()).unwrap_err().to_string(), MINUTES_OVERFLOW);
    let tile = hourly_amounts(&index, ApmGroup::Country, 0, 0, None, None, &[], &BTreeSet::from(["AAA"]));
    assert_eq!(tile.err().unwrap().to_string(), MINUTES_OVERFLOW);
    assert_eq!(followed(&index).update_window(0, true).unwrap_err(), MINUTES_OVERFLOW);
}
