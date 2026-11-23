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
fn a_step_of_fifteen_hours_reads_the_parts_at_the_windows_edges_never_the_commands_inside_it() {
    let sparse = recording(|_| 2);
    let dense = recording(|_| 60);
    let step = |index: &Arc<Index>, from: i32| {
        let mut head = followed(index);
        let (first, last) = head.window_at(from).unwrap();
        head.amounts(first, last);
        let before = head.visits();
        let (first, last) = head.window_at(from + 15).unwrap();
        head.amounts(first, last);
        head.visits() - before
    };
    let at_200 = step(&sparse, 200);
    assert_eq!(step(&dense, 200), at_200, "thirty times the commands in the window, the same lookups");
    assert_eq!(step(&dense, 300), at_200, "another hour, the same lookups");
    assert_eq!(at_200, 1 + 2 * (1 + 2 + 1) + 1, "what one step reads");
}
