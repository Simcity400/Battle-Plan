use super::{
    DecisionsTail, EventChoicesTail, EventFeedActivity, EventFeedCategory, EventFeedKind, EventFeedKinds, EventFeedLine, EventFeedTail, EventFeedTailParams, ScriptTailParams,
    tails::EventFeedActivityParams,
};
use dashboard_base::{
    Source, SourceKey,
    products::{BranchParams, LoadContext, Product},
    test_fixtures::{CAMPAIGN, records, source},
};
use replay::campaign::Replay;
use replay_data::{
    event_feed_rows::{EventFeedCountry, EventFeedCounts, EventFeedCoverage, EventFeedEvent, EventFeedHistoryRow, EventFeedSource},
    rows::{CountryLinkRow, CountryRow, FactionMemberRow, FactionRow, Recorded, TableId, TableRows, WarRelationRow},
};
use replay_frame::frame::{DecodedFrame, TableView};
use serde_json::{Value, json};
use std::sync::{Arc, atomic::AtomicBool};

fn base_frame(hour: i32, countries: Option<Vec<(u32, u32, &[u8])>>, links: Option<Vec<(u32, u32)>>) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1));
    *frame.tables[TableId::Countries.index()] = match countries {
        Some(mut rows) => {
            rows.sort_by_key(|&(id, ..)| id);
            TableView::Complete(TableRows::Countries(
                rows.into_iter()
                    .map(|(id, canonical_slot, tag)| CountryRow {
                        id,
                        canonical_slot,
                        tag: tag.to_vec(),
                        border_land_doctrine_level: Recorded::Value(None),
                        ..CountryRow::default()
                    })
                    .collect(),
            ))
        }
        None => TableView::Unavailable { reason: "countries unread".into() },
    };
    *frame.tables[TableId::CountryLinks.index()] = match links {
        Some(mut rows) => {
            rows.sort_by_key(|&(id, _)| id);
            TableView::Complete(TableRows::CountryLinks(
                rows.into_iter().map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot }).collect(),
            ))
        }
        None => TableView::Unavailable { reason: "links unread".into() },
    };
    *frame.tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(Vec::new()));
    *frame.tables[TableId::FactionMembers.index()] = TableView::Complete(TableRows::FactionMembers(Vec::new()));
    *frame.tables[TableId::WarRelations.index()] = TableView::Complete(TableRows::WarRelations(Vec::new()));
    frame.metrics.table_ns[TableId::EventFeedHistory.index()] = replay_frame::frame::READ_TIME_NOT_RECORDED;
    *frame.tables[TableId::EventFeedHistory.index()] = TableView::Complete(TableRows::EventFeedHistory(vec![EventFeedHistoryRow {
        generation: 1,
        read_hour: hour,
        coverage: EventFeedCoverage::Complete,
        counts: Some(EventFeedCounts::default()),
        outstanding: 0,
        failure: None,
        script_refusals: 0,
        events: Vec::new(),
        script_tokens: Vec::new(),
        script_scopes: Vec::new(),
        windows: Default::default(),
    }]));
    frame
}

fn set_factions(frame: &mut DecodedFrame, factions: Option<Vec<(u64, &[u8])>>, members: Option<Vec<(u32, u64, u32)>>) {
    *frame.tables[TableId::Factions.index()] = match factions {
        Some(mut rows) => {
            rows.sort_by_key(|&(id, _)| id);
            TableView::Complete(TableRows::Factions(
                rows.into_iter().map(|(id, name)| FactionRow { id, name: name.to_vec(), ..FactionRow::default() }).collect(),
            ))
        }
        None => TableView::Unavailable { reason: "factions unread".into() },
    };
    *frame.tables[TableId::FactionMembers.index()] = match members {
        Some(mut rows) => {
            rows.sort_by_key(|&(country, ..)| country);
            TableView::Complete(TableRows::FactionMembers(
                rows.into_iter().map(|(country, faction, ordinal)| FactionMemberRow { country, faction, ordinal }).collect(),
            ))
        }
        None => TableView::Unavailable { reason: "faction members unread".into() },
    };
}

fn set_wars(frame: &mut DecodedFrame, wars: Option<Vec<(u32, u32, i32, i32)>>) {
    *frame.tables[TableId::WarRelations.index()] = match wars {
        Some(mut rows) => {
            rows.sort_by_key(|&(first_country, second_country, ..)| (first_country, second_country));
            TableView::Complete(TableRows::WarRelations(
                rows.into_iter()
                    .map(|(first_country, second_country, start_hour, end_hour)| WarRelationRow {
                        first_country,
                        second_country,
                        start_hour,
                        end_hour,
                        ..WarRelationRow::default()
                    })
                    .collect(),
            ))
        }
        None => TableView::Unavailable { reason: "wars unread".into() },
    };
}

fn set_native_events(frame: &mut DecodedFrame, read_hour: i32, events: Vec<EventFeedEvent>) {
    let count = events.len() as u64;
    *frame.tables[TableId::EventFeedHistory.index()] = TableView::Complete(TableRows::EventFeedHistory(vec![EventFeedHistoryRow {
        generation: 1,
        read_hour,
        coverage: EventFeedCoverage::Complete,
        counts: Some(EventFeedCounts { acquired: count, settled: count, failed: 0, rejected: 0 }),
        outstanding: 0,
        failure: None,
        script_refusals: 0,
        events,
        script_tokens: Vec::new(),
        script_scopes: Vec::new(),
        windows: Default::default(),
    }]));
}

fn open(name: &str, frames: Vec<DecodedFrame>) -> Arc<Source> {
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    Arc::new(
        Source::from_replay(
            SourceKey { source_id: name.into(), revision: "1".into() },
            format!("{name}.battleplan").into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap(),
    )
}

fn tail(source: &Source, hour: i32, categories: Vec<EventFeedCategory>) -> super::EventFeedTailReply {
    EventFeedTail::load(
        source,
        &EventFeedTailParams {
            branch_id: "1".into(),
            hour: Some(hour),
            from: None,
            to: None,
            countries: None,
            group: None,
            search: None,
            row: None,
            sort: None,
            offset: None,
            limit: 64,
            categories,
            exclude: None,
            expanded: None,
            full: None,
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn event_rows(reply: &super::EventFeedTailReply) -> Vec<&super::EventFeedRow> {
    reply
        .rows
        .iter()
        .filter_map(|line| match line {
            EventFeedLine::Event(row) => row.row.as_ref(),
            _ => None,
        })
        .collect()
}

#[test]
fn duplicate_tag_hour_invents_no_faction_leave_or_join() {
    let frames = (100..=102)
        .map(|hour| {
            let countries = if hour == 101 {
                vec![(8001, 5, b"AAA".as_slice()), (9001, 7, b"AAA".as_slice())]
            } else {
                vec![(9001, 7, b"AAA".as_slice())]
            };
            let links = if hour == 101 { vec![(8001, 5), (9001, 7)] } else { vec![(9001, 7)] };
            let mut frame = base_frame(hour, Some(countries), Some(links));
            set_factions(&mut frame, Some(vec![(10, b"Alpha Pact")]), Some(vec![(9001, 10, 0)]));
            frame
        })
        .collect();
    let src = open("dup-tag-faction", frames);
    let reply = tail(&src, 102, vec![EventFeedCategory::Diplomacy]);
    let rows = event_rows(&reply);
    assert!(
        rows.iter().all(|row| !matches!(row.kind, EventFeedKind::FactionJoined | EventFeedKind::FactionLeft)),
        "duplicate tag at 101 must not invent leave/join: {rows:?}"
    );
}

#[test]
fn slot_move_with_unread_countries_invents_no_leave_or_peace() {
    let frames = (119..=120)
        .map(|hour| {
            let countries = (hour == 119).then(|| vec![(9001, 7, b"AAA".as_slice()), (9002, 9, b"ZZZ".as_slice())]);
            let links = if hour == 119 { vec![(9001, 7), (9002, 9)] } else { vec![(9001, 8), (9002, 9)] };
            let mut frame = base_frame(hour, countries, Some(links));
            set_factions(&mut frame, Some(vec![(10, b"Alpha Pact")]), Some(vec![(9001, 10, 0)]));
            set_wars(&mut frame, Some(vec![(9001, 9002, 100, 0)]));
            frame
        })
        .collect();
    let src = open("slot-move-unread", frames);
    let reply = tail(&src, 120, vec![EventFeedCategory::WarsPeace, EventFeedCategory::Diplomacy]);
    let rows = event_rows(&reply);
    assert!(
        rows.iter().all(|row| !matches!(row.kind, EventFeedKind::FactionLeft | EventFeedKind::FactionJoined | EventFeedKind::Peace)),
        "slot move while countries unread must invent neither leave nor peace: {rows:?}"
    );
    assert!(reply.unavailable.iter().any(|u| u.kind == "peace"), "unread countries at 120 must mark peace unrecorded: {:?}", reply.unavailable);
    assert!(reply.unavailable.iter().all(|u| u.kind != "factionMembership"), "immutable-id faction membership stays clean when only countries is unread");
    let act = EventFeedActivity::load(
        &src,
        &EventFeedActivityParams { branch_id: "1".into(), hour: 120, from: 119, bin_hours: 24, categories: vec![EventFeedCategory::WarsPeace], exclude: None, countries: None, group: None },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(act.missing.iter().any(|s| s.first_hour <= 120 && 120 <= s.last_hour), "activity strip must mark hour 120 unrecorded for peace: {:?}", act.missing);
}

#[test]
fn consecutive_slot_reuse_separates_old_and_new_country_transitions() {
    let frames = [100, 101]
        .into_iter()
        .map(|hour| {
            let (countries, links, members, wars) = if hour == 100 {
                (
                    vec![(9001, 7, b"AAA".as_slice()), (9002, 9, b"ZZZ".as_slice())],
                    vec![(9001, 7), (9002, 9)],
                    vec![(9001, 10, 0)],
                    vec![(9001, 9002, 100, 0)],
                )
            } else {
                (
                    vec![(9009, 7, b"AAA".as_slice()), (9002, 9, b"ZZZ".as_slice())],
                    vec![(9001, 7), (9009, 7), (9002, 9)],
                    vec![(9009, 10, 0)],
                    vec![(9009, 9002, 101, 0)],
                )
            };
            let mut frame = base_frame(hour, Some(countries), Some(links));
            set_factions(&mut frame, Some(vec![(10, b"Alpha Pact")]), Some(members));
            set_wars(&mut frame, Some(wars));
            frame
        })
        .collect();
    let src = open("slot-reuse", frames);
    let reply = tail(&src, 101, vec![EventFeedCategory::WarsPeace, EventFeedCategory::Diplomacy]);
    let rows = event_rows(&reply);
    assert!(rows.iter().any(|row| row.kind == EventFeedKind::FactionLeft && row.hour == 101), "old id left at 101: {rows:?}");
    assert!(rows.iter().any(|row| row.kind == EventFeedKind::FactionJoined && row.hour == 101), "new id joined at 101: {rows:?}");
    assert!(rows.iter().any(|row| row.kind == EventFeedKind::Peace && row.hour == 101), "old id war ended at 101: {rows:?}");
}

#[test]
fn tag_change_invents_no_faction_or_war_transition() {
    let frames = [100, 101]
        .into_iter()
        .map(|hour| {
            let tag = if hour == 100 { b"AAA".as_slice() } else { b"BBB".as_slice() };
            let mut frame = base_frame(hour, Some(vec![(9001, 7, tag), (9002, 9, b"ZZZ")]), Some(vec![(9001, 7), (9002, 9)]));
            set_factions(&mut frame, Some(vec![(10, b"Alpha Pact")]), Some(vec![(9001, 10, 0)]));
            set_wars(&mut frame, Some(vec![(9001, 9002, 100, 0)]));
            frame
        })
        .collect();
    let src = open("tag-change", frames);
    let reply = tail(&src, 101, vec![EventFeedCategory::WarsPeace, EventFeedCategory::Diplomacy]);
    let rows = event_rows(&reply);
    assert!(
        rows.iter().all(|row| !matches!(row.kind, EventFeedKind::FactionLeft | EventFeedKind::FactionJoined | EventFeedKind::Peace)),
        "tag change must invent no transitions: {rows:?}"
    );
}

#[test]
fn unread_target_hour_suppresses_inferred_leave_and_peace_and_reports_unrecorded() {
    let frames = [100, 101]
        .into_iter()
        .map(|hour| {
            let mut frame = base_frame(hour, Some(vec![(9001, 7, b"AAA"), (9002, 9, b"ZZZ")]), Some(vec![(9001, 7), (9002, 9)]));
            if hour == 100 {
                set_factions(&mut frame, Some(vec![(10, b"Alpha Pact")]), Some(vec![(9001, 10, 0)]));
                set_wars(&mut frame, Some(vec![(9001, 9002, 100, 0)]));
            } else {
                set_factions(&mut frame, Some(vec![(10, b"Alpha Pact")]), None);
                set_wars(&mut frame, None);
            }
            frame
        })
        .collect();
    let src = open("unread-target", frames);
    let reply = tail(&src, 101, vec![EventFeedCategory::WarsPeace, EventFeedCategory::Diplomacy]);
    let rows = event_rows(&reply);
    assert!(
        rows.iter().all(|row| !matches!(row.kind, EventFeedKind::FactionLeft | EventFeedKind::Peace)),
        "unread target hour must never invent leave or peace: {rows:?}"
    );
    assert!(reply.unavailable.iter().any(|u| u.kind == "peace"), "peace unrecorded notice missing: {:?}", reply.unavailable);
    assert!(reply.unavailable.iter().any(|u| u.kind == "factionMembership"), "faction unrecorded notice missing: {:?}", reply.unavailable);
    let load = LoadContext { content: None };
    let cancelled = AtomicBool::new(false);
    let ask_activity = |categories, exclude| {
        EventFeedActivity::load(
            &src,
            &EventFeedActivityParams { branch_id: "1".into(), hour: 101, from: 100, bin_hours: 24, categories, exclude, countries: None, group: None },
            &load,
            &cancelled,
        )
        .unwrap()
    };
    let both = ask_activity(vec![EventFeedCategory::WarsPeace, EventFeedCategory::Diplomacy], None);
    assert!(both.missing.iter().any(|s| s.first_hour == 101 && s.last_hour == 101 && s.reason.contains("Peace")), "activity must include peace gap: {:?}", both.missing);
    assert!(both.missing.iter().any(|s| s.first_hour == 101 && s.last_hour == 101 && s.reason.contains("Faction")), "activity must include faction gap: {:?}", both.missing);
    let no_peace = ask_activity(vec![EventFeedCategory::WarsPeace], Some(vec!["peace".into()]));
    assert!(no_peace.missing.is_empty(), "excluded peace must not add peace gap: {:?}", no_peace.missing);
    let no_faction = ask_activity(vec![EventFeedCategory::Diplomacy], Some(vec!["factionJoined".into(), "factionLeft".into()]));
    assert!(no_faction.missing.is_empty(), "excluded faction kinds must not add faction gap: {:?}", no_faction.missing);
}

#[test]
fn mixed_war_roster_keeps_witnessed_peace_and_refuses_ambiguous_end() {
    let witnessed = [100, 101]
        .into_iter()
        .map(|hour| {
            let mut frame = base_frame(hour, Some(vec![(9001, 7, b"AAA"), (9002, 9, b"BBB")]), Some(vec![(9001, 7), (9002, 9)]));
            let wars = if hour == 100 { vec![(9001, 9002, 100, 0), (9901, 9902, 100, 0)] } else { Vec::new() };
            set_wars(&mut frame, Some(wars));
            frame
        })
        .collect();
    let src = open("mixed-roster", witnessed);
    let reply = tail(&src, 101, vec![EventFeedCategory::WarsPeace]);
    let rows = event_rows(&reply);
    assert!(
        rows.iter().any(|row| row.kind == EventFeedKind::Peace && row.hour == 101 && row.tags == ["AAA", "BBB"]),
        "witnessed A-B peace must survive unrelated unlinked war at 100: {rows:?}"
    );

    let ambiguous = [100, 101]
        .into_iter()
        .map(|hour| {
            let mut frame = base_frame(hour, Some(vec![(9001, 7, b"AAA"), (9002, 9, b"BBB")]), Some(vec![(9001, 7), (9002, 9)]));
            let wars = if hour == 100 { vec![(9001, 9002, 100, 0)] } else { vec![(9001, 9999, 100, 0)] };
            set_wars(&mut frame, Some(wars));
            frame
        })
        .collect();
    let amb_src = open("ambiguous-end", ambiguous);
    let amb_reply = tail(&amb_src, 101, vec![EventFeedCategory::WarsPeace]);
    assert!(
        event_rows(&amb_reply).iter().all(|row| row.kind != EventFeedKind::Peace),
        "ambiguous unlinked war sharing endpoint at 101 must not invent peace"
    );
    assert!(amb_reply.unavailable.iter().any(|u| u.kind == "peace"));
}

#[test]
fn late_native_war_end_supersedes_inferred_peace_at_observation_on_exact_and_followed_reads() {
    let frames = (100..=103)
        .map(|hour| {
            let mut frame = base_frame(hour, Some(vec![(9001, 7, b"AAA"), (9002, 9, b"BBB")]), Some(vec![(9001, 7), (9002, 9)]));
            set_wars(&mut frame, Some(if hour == 100 { vec![(9001, 9002, 100, 0)] } else { Vec::new() }));
            if hour == 103 {
                set_native_events(
                    &mut frame,
                    103,
                    vec![EventFeedEvent {
                        sequence: 1,
                        generation: 1,
                        hour: 101,
                        source: EventFeedSource::WarEnded,
                        actor: EventFeedCountry { reference: 9001, id: 9001, instance: 1 },
                        target: EventFeedCountry { reference: 9002, id: 9002, instance: 1 },
                        cause: Recorded::NotRecorded,
                        script: None,
                    }],
                );
            }
            frame
        })
        .collect();
    let src = open("late-native-peace", frames);
    let cancelled = AtomicBool::new(false);
    let load = LoadContext { content: None };
    let ask = json!({"panel": "event-feed-tail", "branchId": "1", "limit": 64, "categories": ["warsPeace"]});
    let panel = super::event_feed_dated_panel(&src, &load, &ask).unwrap();
    let followed = panel.exact().unwrap();
    let exact_at = |hour: i32| -> Value {
        crate::registrations::FEED_EVENTFEEDTAIL
            .answer(&src, json!({"branchId": "1", "hour": hour, "limit": 64, "categories": ["warsPeace"]}), &load, &cancelled)
            .unwrap()
    };
    let kinds_of = |reply: &Value| -> Vec<String> {
        reply["rows"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|line| line.get("row").and_then(|r| r.get("kind")).and_then(Value::as_str).map(str::to_owned))
            .collect()
    };
    for hour in [100, 101, 102, 103] {
        let exact = exact_at(hour);
        let step = (followed.advance)(hour, hour == 100, &cancelled, None);
        let held = step.value.as_ref().as_ref().unwrap();
        assert_eq!(held, &exact, "followed and exact disagree at hour {hour}");
        assert_eq!(&(followed.cold)(hour, &cancelled).unwrap(), &exact, "cold followed and exact disagree at hour {hour}");
        let kinds = kinds_of(&exact);
        match hour {
            100 => assert_eq!(kinds, ["warStarted"]),
            101 | 102 => {
                assert!(kinds.contains(&"peace".to_owned()), "inferred peace must appear at {hour} without leaking future native read: {kinds:?}");
                assert!(!kinds.contains(&"warEnded".to_owned()), "future native warEnded must not appear at {hour}: {kinds:?}");
            }
            103 => {
                assert!(kinds.contains(&"warEnded".to_owned()), "native warEnded must appear at 103: {kinds:?}");
                assert!(!kinds.contains(&"peace".to_owned()), "inferred peace must retire at 103 to prevent duplicate: {kinds:?}");
            }
            _ => unreachable!(),
        }
        let activity = EventFeedActivity::load(
            &src,
            &EventFeedActivityParams {
                branch_id: "1".into(),
                hour,
                from: 100,
                bin_hours: 24,
                categories: vec![EventFeedCategory::WarsPeace],
                exclude: None,
                countries: None,
                group: None,
            },
            &load,
            &cancelled,
        )
        .unwrap();
        let total: u64 = activity.lines[0].bins.iter().map(|&(_, count)| count).sum();
        assert_eq!(total, kinds.len() as u64, "activity count must match visible rows at hour {hour}");
    }
}

#[test]
fn slot_moved_native_war_end_deduplicates_without_duplicate_peace() {
    let frames = [119, 120]
        .into_iter()
        .map(|hour| {
            let (slot_a, wars) = if hour == 119 { (7, vec![(9001, 9002, 100, 0)]) } else { (8, Vec::new()) };
            let mut frame = base_frame(hour, Some(vec![(9001, slot_a, b"AAA"), (9002, 9, b"BBB")]), Some(vec![(9001, slot_a), (9002, 9)]));
            set_wars(&mut frame, Some(wars));
            if hour == 120 {
                set_native_events(
                    &mut frame,
                    120,
                    vec![EventFeedEvent {
                        sequence: 1,
                        generation: 1,
                        hour: 120,
                        source: EventFeedSource::WarEnded,
                        actor: EventFeedCountry { reference: 9001, id: 9001, instance: 1 },
                        target: EventFeedCountry { reference: 9002, id: 9002, instance: 1 },
                        cause: Recorded::NotRecorded,
                        script: None,
                    }],
                );
            }
            frame
        })
        .collect();
    let src = open("slot-move-native-end", frames);
    let reply = tail(&src, 120, vec![EventFeedCategory::WarsPeace]);
    let rows = event_rows(&reply);
    assert!(rows.iter().any(|row| row.kind == EventFeedKind::WarEnded && row.hour == 120));
    assert!(rows.iter().all(|row| row.kind != EventFeedKind::Peace), "slot-moved native end must suppress inferred peace: {rows:?}");
}

#[test]
fn proven_transition_with_unresolved_country_stays_listed_with_missing_reason() {
    let frames = [100, 101]
        .into_iter()
        .map(|hour| {
            let countries = if hour == 100 { vec![(9001, 7, b"AAA".as_slice())] } else { Vec::new() };
            let mut frame = base_frame(hour, Some(countries), Some(vec![(9001, 7)]));
            set_factions(&mut frame, Some(vec![(10, b"Alpha Pact")]), Some(if hour == 101 { vec![(9001, 10, 0)] } else { Vec::new() }));
            set_wars(&mut frame, Some(if hour == 100 { vec![(9001, 9999, 100, 0)] } else { Vec::new() }));
            frame
        })
        .collect();
    let src = open("unresolved-party-transition", frames);
    let reply = tail(&src, 101, vec![EventFeedCategory::WarsPeace, EventFeedCategory::Diplomacy]);
    let rows = event_rows(&reply);
    let join = rows.iter().find(|row| row.kind == EventFeedKind::FactionJoined && row.hour == 101).expect("proven join stays listed");
    assert!(join.tags.is_empty());
    assert_eq!(join.participants, Some([None, None]));
    assert!(join.participants_missing.as_ref().is_some_and(|m| m[0].is_some() && m[1].is_none()));
    let peace = rows.iter().find(|row| row.kind == EventFeedKind::Peace && row.hour == 101).expect("proven peace stays listed");
    assert_eq!(peace.tags, ["AAA"]);
    assert_eq!(peace.participants, Some([Some("AAA".into()), None]));
    assert!(peace.participants_missing.as_ref().is_some_and(|m| m[0].is_none() && m[1].is_some()));
}

#[test]
fn transition_read_is_shared_once_per_branch_and_skipped_by_decisions_and_choices() {
    let frames = [100, 101]
        .into_iter()
        .map(|hour| {
            let mut frame = base_frame(hour, Some(vec![(9001, 7, b"AAA")]), Some(vec![(9001, 7)]));
            set_factions(&mut frame, Some(vec![(10, b"Alpha Pact")]), Some(if hour == 101 { vec![(9001, 10, 0)] } else { Vec::new() }));
            frame
        })
        .collect();
    let src = open("read-count-proof", frames);
    let load = LoadContext { content: None };
    let cancelled = AtomicBool::new(false);
    let script_params = ScriptTailParams {
        branch_id: "1".into(),
        hour: Some(101),
        from: None,
        to: None,
        countries: None,
        group: None,
        events: None,
        statuses: None,
        search: None,
        row: None,
        sort: None,
        offset: None,
        limit: 32,
    };
    DecisionsTail::load(&src, &script_params, &load, &cancelled).unwrap();
    EventChoicesTail::load(&src, &script_params, &load, &cancelled).unwrap();
    assert_eq!(super::index::transition_reads(&src, 1), 0, "decisions and choices must not read transitions");
    tail(&src, 101, vec![EventFeedCategory::Diplomacy]);
    tail(&src, 100, vec![EventFeedCategory::WarsPeace]);
    EventFeedActivity::load(
        &src,
        &EventFeedActivityParams {
            branch_id: "1".into(),
            hour: 101,
            from: 100,
            bin_hours: 24,
            categories: vec![EventFeedCategory::Diplomacy],
            exclude: None,
            countries: None,
            group: None,
        },
        &load,
        &cancelled,
    )
    .unwrap();
    assert_eq!(super::index::transition_reads(&src, 1), 1, "all feed views on one branch must share one read");
}

#[test]
fn duplicate_tag_transitions_at_same_hour_both_survive() {
    let frames = [100, 101, 102]
        .into_iter()
        .map(|hour| {
            let mut frame = base_frame(
                hour,
                Some(vec![(8001, 1, b"AAA"), (9001, 2, b"AAA"), (7001, 3, b"BBB")]),
                Some(vec![(8001, 1), (9001, 2), (7001, 3)]),
            );
            set_factions(
                &mut frame,
                Some(vec![(10, b"Alpha Pact")]),
                Some(if hour >= 101 { vec![(8001, 10, 0), (9001, 10, 1)] } else { Vec::new() }),
            );
            set_wars(
                &mut frame,
                Some(if hour <= 101 { vec![(8001, 7001, 100, 0), (9001, 7001, 100, 0)] } else { Vec::new() }),
            );
            frame
        })
        .collect();
    let src = open("duplicate-tag-same-hour", frames);
    let reply = tail(&src, 102, vec![EventFeedCategory::WarsPeace, EventFeedCategory::Diplomacy]);
    let rows = event_rows(&reply);
    let joins: Vec<_> = rows.iter().filter(|r| r.kind == EventFeedKind::FactionJoined && r.hour == 101).collect();
    let peaces: Vec<_> = rows.iter().filter(|r| r.kind == EventFeedKind::Peace && r.hour == 102).collect();
    assert_eq!(joins.len(), 2, "both duplicate-tag joins must survive: {joins:?}");
    assert_ne!(joins[0].id, joins[1].id);
    assert_eq!(peaces.len(), 2, "both duplicate-tag peaces must survive: {peaces:?}");
    assert_ne!(peaces[0].id, peaces[1].id);
}

#[test]
fn war_endpoint_link_reassignment_proves_peace() {
    let frames = [100, 101]
        .into_iter()
        .map(|hour| {
            let mut frame = base_frame(
                hour,
                Some(vec![(1, 1, b"AAA"), (2, 2, b"BBB"), (3, 3, b"CCC")]),
                Some(vec![(7, if hour == 100 { 1 } else { 3 }), (9, 2)]),
            );
            set_wars(&mut frame, Some(vec![(7, 9, 100, 0)]));
            frame
        })
        .collect();
    let src = open("war-link-reassign", frames);
    let reply = tail(&src, 101, vec![EventFeedCategory::WarsPeace]);
    let rows = event_rows(&reply);
    assert!(
        rows.iter().any(|r| r.kind == EventFeedKind::Peace && r.hour == 101 && r.tags == ["AAA", "BBB"]),
        "link moving from A to C on continuing raw war must prove A-B peace: {rows:?}"
    );
}

#[test]
fn war_relations_failure_isolates_to_peace_family() {
    let frames = [100, 101]
        .into_iter()
        .map(|hour| {
            let mut frame = base_frame(hour, Some(vec![(9001, 7, b"AAA"), (9002, 9, b"BBB")]), Some(vec![(9001, 7), (9002, 9)]));
            set_factions(&mut frame, Some(vec![(10, b"Alpha Pact")]), Some(if hour == 101 { vec![(9001, 10, 0)] } else { Vec::new() }));
            set_wars(&mut frame, Some(if hour == 100 { vec![(9001, 9002, 100, 0)] } else { Vec::new() }));
            frame
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let entry = replay.recording().table_entry(0, 0, TableId::WarRelations.index()).unwrap();
    let mut bytes = replay.bytes().to_vec();
    bytes[entry.unit.parts[0].frames[0].offset as usize] ^= 0xff;
    let damaged = Replay::open(bytes).unwrap().into_timelines(&Default::default()).unwrap();
    let src = Arc::new(
        Source::from_replay(
            SourceKey { source_id: "damaged-wars".into(), revision: "1".into() },
            "damaged-wars.battleplan".into(),
            Arc::new(damaged),
            &AtomicBool::new(false),
        )
        .unwrap(),
    );
    let diplo = tail(&src, 101, vec![EventFeedCategory::Diplomacy]);
    assert!(event_rows(&diplo).iter().any(|r| r.kind == EventFeedKind::FactionJoined && r.hour == 101));
    assert!(diplo.unavailable.iter().all(|u| u.kind != "peace"), "diplomacy-only request must not fail or carry peace refusal");
    let both = tail(&src, 101, vec![EventFeedCategory::WarsPeace, EventFeedCategory::Diplomacy]);
    assert!(both.unavailable.iter().any(|u| u.kind == "peace"), "warsPeace must report peace refusal: {:?}", both.unavailable);
    let load = LoadContext { content: None };
    let cancelled = AtomicBool::new(false);
    let kinds = EventFeedKinds::load(&src, &BranchParams { branch_id: "1".into() }, &load, &cancelled).unwrap();
    let peace_info = kinds.kinds.iter().find(|k| k.kind == EventFeedKind::Peace).unwrap();
    let join_info = kinds.kinds.iter().find(|k| k.kind == EventFeedKind::FactionJoined).unwrap();
    assert!(peace_info.unavailable.is_some());
    assert!(join_info.first_observed == Some(101) && join_info.unavailable.is_none());
    let act_wars = EventFeedActivity::load(
        &src,
        &EventFeedActivityParams { branch_id: "1".into(), hour: 101, from: 100, bin_hours: 24, categories: vec![EventFeedCategory::WarsPeace], exclude: None, countries: None, group: None },
        &load,
        &cancelled,
    )
    .unwrap();
    assert!(!act_wars.missing.is_empty(), "activity for warsPeace must report missing span when WarRelations fails");
    let act_diplo = EventFeedActivity::load(
        &src,
        &EventFeedActivityParams { branch_id: "1".into(), hour: 101, from: 100, bin_hours: 24, categories: vec![EventFeedCategory::Diplomacy], exclude: None, countries: None, group: None },
        &load,
        &cancelled,
    )
    .unwrap();
    assert!(act_diplo.missing.is_empty(), "activity for diplomacy must stay clean when only WarRelations fails");
}

#[test]
fn truncated_tables_withhold_inference_and_report_unrecorded() {
    let frames = [100, 101]
        .into_iter()
        .map(|hour| {
            let mut frame = base_frame(hour, Some(vec![(9001, 7, b"AAA"), (9002, 9, b"BBB")]), Some(vec![(9001, 7), (9002, 9)]));
            set_factions(&mut frame, Some(vec![(10, b"Alpha Pact")]), Some(Vec::new()));
            if hour == 101 {
                *frame.tables[TableId::FactionMembers.index()] = TableView::Truncated {
                    rows: TableRows::FactionMembers(vec![FactionMemberRow { country: 9001, faction: 10, ordinal: 0 }]),
                    missing: 1,
                };
            }
            if hour == 100 {
                *frame.tables[TableId::CountryLinks.index()] = TableView::Truncated {
                    rows: TableRows::CountryLinks(vec![CountryLinkRow { id: 9001, canonical_slot: 7 }, CountryLinkRow { id: 9002, canonical_slot: 9 }]),
                    missing: 1,
                };
                set_wars(&mut frame, Some(vec![(9001, 9002, 100, 0)]));
            } else {
                set_wars(&mut frame, Some(Vec::new()));
            }
            frame
        })
        .collect();
    let src = open("truncated-withheld", frames);
    let reply = tail(&src, 101, vec![EventFeedCategory::WarsPeace, EventFeedCategory::Diplomacy]);
    let rows = event_rows(&reply);
    assert!(rows.iter().all(|r| !matches!(r.kind, EventFeedKind::FactionJoined | EventFeedKind::Peace)), "non-complete tables must not infer transitions: {rows:?}");
    assert!(reply.unavailable.iter().any(|u| u.kind == "factionMembership"), "truncated faction members must be reported unrecorded");
    assert!(reply.unavailable.iter().any(|u| u.kind == "peace"), "truncated country links must be reported unrecorded for peace");
    let act = EventFeedActivity::load(
        &src,
        &EventFeedActivityParams { branch_id: "1".into(), hour: 101, from: 100, bin_hours: 24, categories: vec![EventFeedCategory::WarsPeace, EventFeedCategory::Diplomacy], exclude: None, countries: None, group: None },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(act.missing.iter().any(|s| s.first_hour == 101 && s.last_hour == 101 && s.reason.contains("Peace")));
    assert!(act.missing.iter().any(|s| s.first_hour == 101 && s.last_hour == 101 && s.reason.contains("Faction")));
}

#[test]
fn stretch_scoped_table_failure_marks_only_failed_stretch_missing() {
    let stretch0: Vec<_> = [100, 101]
        .into_iter()
        .map(|hour| {
            let mut frame = base_frame(hour, Some(vec![(9001, 7, b"AAA"), (9002, 9, b"BBB")]), Some(vec![(9001, 7), (9002, 9)]));
            set_wars(&mut frame, Some(if hour == 100 { vec![(9001, 9002, 100, 0)] } else { Vec::new() }));
            frame
        })
        .collect();
    let stretch1: Vec<_> = [200, 201]
        .into_iter()
        .map(|hour| {
            let mut frame = base_frame(hour, Some(vec![(9001, 7, b"AAA"), (9002, 9, b"BBB")]), Some(vec![(9001, 7), (9002, 9)]));
            set_wars(&mut frame, Some(if hour == 200 { vec![(9001, 9002, 200, 0)] } else { Vec::new() }));
            frame
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, stretch0.into_iter().chain(stretch1).collect())]).unwrap();
    let entry = replay.recording().table_entry(1, 0, TableId::WarRelations.index()).unwrap();
    let mut bytes = replay.bytes().to_vec();
    bytes[entry.unit.parts[0].frames[0].offset as usize] ^= 0xff;
    let damaged = Replay::open(bytes).unwrap().into_timelines(&Default::default()).unwrap();
    let src = Arc::new(Source::from_replay(SourceKey { source_id: "stretch-fail".into(), revision: "1".into() }, "stretch-fail.battleplan".into(), Arc::new(damaged), &AtomicBool::new(false)).unwrap());
    let early = tail(&src, 101, vec![EventFeedCategory::WarsPeace]);
    assert!(event_rows(&early).iter().any(|r| r.kind == EventFeedKind::Peace && r.hour == 101));
    assert!(early.unavailable.iter().all(|u| u.kind != "peace"), "failure in stretch 1 must not taint stretch 0 tail: {:?}", early.unavailable);
    let load = LoadContext { content: None };
    let cancelled = AtomicBool::new(false);
    let early_act = EventFeedActivity::load(
        &src,
        &EventFeedActivityParams { branch_id: "1".into(), hour: 101, from: 100, bin_hours: 24, categories: vec![EventFeedCategory::WarsPeace], exclude: None, countries: None, group: None },
        &load,
        &cancelled,
    )
    .unwrap();
    assert!(early_act.missing.is_empty(), "failure in stretch 1 must not mark stretch 0 activity missing: {:?}", early_act.missing);
    let whole_act = EventFeedActivity::load(
        &src,
        &EventFeedActivityParams { branch_id: "1".into(), hour: 201, from: 100, bin_hours: 24, categories: vec![EventFeedCategory::WarsPeace], exclude: None, countries: None, group: None },
        &load,
        &cancelled,
    )
    .unwrap();
    assert!(whole_act.missing.iter().any(|s| s.first_hour == 200 && s.last_hour == 201), "failed stretch 1 bounds must be marked missing: {:?}", whole_act.missing);
    assert!(whole_act.missing.iter().all(|s| s.last_hour < 100 || s.first_hour > 101), "stretch 0 hours 100..=101 must stay clean: {:?}", whole_act.missing);
}

#[test]
fn groups_shared_failure_still_emits_proven_raw_transitions() {
    let frames = [100, 101]
        .into_iter()
        .map(|hour| {
            let mut frame = base_frame(hour, Some(vec![(9001, 7, b"AAA"), (9002, 9, b"BBB")]), Some(vec![(9001, 7), (9002, 9)]));
            set_factions(&mut frame, Some(vec![(10, b"Alpha Pact")]), Some(if hour == 101 { vec![(9001, 10, 0)] } else { Vec::new() }));
            set_wars(&mut frame, Some(if hour == 100 { vec![(9001, 9002, 100, 0)] } else { Vec::new() }));
            frame
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let entry = replay.recording().table_entry(0, 0, TableId::GroupNames.index()).unwrap();
    let meta = entry.unit.first.as_ref().unwrap().streams.iter().find(|(id, _)| *id == replay_columnar::unit::restore::META).unwrap().1[0].frame.offset;
    let mut bytes = replay.bytes().to_vec();
    bytes[meta as usize] ^= 0xff;
    let damaged = Replay::open(bytes).unwrap().into_timelines(&Default::default()).unwrap();
    let src = Arc::new(Source::from_replay(SourceKey { source_id: "damaged-war-events".into(), revision: "1".into() }, "damaged-war-events.battleplan".into(), Arc::new(damaged), &AtomicBool::new(false)).unwrap());
    let load = LoadContext { content: None };
    let cancelled = AtomicBool::new(false);
    let query = dashboard_base::queries::query(&src, 1).unwrap();
    assert!(dashboard_world::products::groups::shared(&src, &query, &load, &cancelled).is_err(), "damaged GroupNames must fail groups::shared");
    let reply = tail(&src, 101, vec![EventFeedCategory::WarsPeace, EventFeedCategory::Diplomacy]);
    let rows = event_rows(&reply);
    assert!(rows.iter().any(|r| r.kind == EventFeedKind::Peace && r.hour == 101 && r.tags == ["AAA", "BBB"]), "proven peace must survive damaged GroupNames: {rows:?}");
    let join = rows.iter().find(|r| r.kind == EventFeedKind::FactionJoined && r.hour == 101 && r.tags == ["AAA"]).expect("proven join must survive damaged GroupNames");
    assert!(join.item.as_ref().and_then(|i| i.name_missing.as_deref()).is_some_and(|r| r.contains("Faction names are unavailable")), "faction join must carry missing-name reason: {:?}", join.item);
    assert!(reply.unavailable.iter().any(|u| u.kind == "countryGroups"), "countryGroups refusal must still be reported");
    let wars_act = EventFeedActivity::load(
        &src,
        &EventFeedActivityParams { branch_id: "1".into(), hour: 101, from: 100, bin_hours: 24, categories: vec![EventFeedCategory::WarsPeace], exclude: None, countries: None, group: None },
        &load,
        &cancelled,
    )
    .unwrap();
    assert!(!wars_act.missing.is_empty(), "activity for warsPeace must report missing coverage when countryGroups fails");
    let diplo_act = EventFeedActivity::load(
        &src,
        &EventFeedActivityParams { branch_id: "1".into(), hour: 101, from: 100, bin_hours: 24, categories: vec![EventFeedCategory::Diplomacy], exclude: None, countries: None, group: None },
        &load,
        &cancelled,
    )
    .unwrap();
    assert!(diplo_act.missing.is_empty(), "activity for diplomacy must stay clean when only countryGroups fails");
}

#[test]
fn late_native_war_end_reports_latest_retirement_for_incremental_activity() {
    let frames = (46..=96)
        .map(|hour| {
            let mut frame = base_frame(hour, Some(vec![(9001, 7, b"AAA"), (9002, 9, b"BBB")]), Some(vec![(9001, 7), (9002, 9)]));
            set_wars(&mut frame, Some(if hour < 48 { vec![(9001, 9002, 46, 0)] } else { Vec::new() }));
            if hour == 96 {
                set_native_events(
                    &mut frame,
                    96,
                    vec![EventFeedEvent {
                        sequence: 1,
                        generation: 1,
                        hour: 47,
                        source: EventFeedSource::WarEnded,
                        actor: EventFeedCountry { reference: 9001, id: 9001, instance: 1 },
                        target: EventFeedCountry { reference: 9002, id: 9002, instance: 1 },
                        cause: Recorded::NotRecorded,
                        script: None,
                    }],
                );
            }
            frame
        })
        .collect();
    let src = open("activity-latest-retirement", frames);
    let load = LoadContext { content: None };
    let cancelled = AtomicBool::new(false);
    let at_70 = EventFeedActivity::load(
        &src,
        &EventFeedActivityParams { branch_id: "1".into(), hour: 70, from: 46, bin_hours: 24, categories: vec![EventFeedCategory::WarsPeace], exclude: None, countries: None, group: None },
        &load,
        &cancelled,
    )
    .unwrap();
    assert_eq!(at_70.lines[0].latest_retirement, None);
    let inc_96 = EventFeedActivity::load(
        &src,
        &EventFeedActivityParams { branch_id: "1".into(), hour: 96, from: 72, bin_hours: 24, categories: vec![EventFeedCategory::WarsPeace], exclude: None, countries: None, group: None },
        &load,
        &cancelled,
    )
    .unwrap();
    assert_eq!(inc_96.lines[0].before, "2", "war start + native warEnded sit before 72");
    assert_eq!(inc_96.lines[0].latest_retirement, Some(96), "retired peace at 48 must report its retirement hour so incremental merge invalidates cached bins");
}
