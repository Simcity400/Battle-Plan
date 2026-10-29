use super::*;
use dashboard_base::{
    prepared::{
        coverage::{self, Coverage},
        frame_changes::FrameChanges,
        prefix::Prefix,
        series::{Reader, tests::Fixture},
    },
    products::country_history::HistoryScan,
    test_fixtures,
};
use replay_data::{format::FormatVersion, rows::*};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};
const A: u32 = 1000;
const B: u32 = 1001;
fn flag() -> AtomicBool {
    AtomicBool::new(false)
}
fn frame(hour: i32, read: i32) -> DecodedFrame {
    let mut frame = test_fixtures::source(1, hour, None);
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        CountryRow {
            id: A,
            canonical_slot: 1,
            tag: b"first".to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: B,
            canonical_slot: 2,
            tag: b"second".to_vec(),
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: A,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: B,
                canonical_slot: 2,
            },
            CountryLinkRow {
                id: 9000,
                canonical_slot: 1,
            },
        ]));
    for table in [
        TableId::RelationOpinions,
        TableId::RelationTuples,
        TableId::IncomingDiplomaticActions,
        TableId::LexicalTokens,
    ] {
        *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
    }
    let mut reads = Vec::new();
    for table in [TableId::RelationOpinions, TableId::RelationTuples] {
        for owner in [A, B] {
            reads.push(SliceReadRow {
                table: table.index() as u32,
                owner,
                sample_hour: read,
            });
        }
    }
    reads.sort_by_key(|row| (row.table, row.owner));
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(reads));
    frame
}
fn opinions(frame: &mut DecodedFrame, rows: &[(u32, u32, i32)]) {
    let mut rows: Vec<_> = rows
        .iter()
        .map(
            |&(owner_country, target_country, opinion)| RelationOpinionRow {
                owner_country,
                target_country,
                opinion,
            },
        )
        .collect();
    rows.sort_by_key(RowCodec::key);
    *frame.tables[TableId::RelationOpinions.index()] =
        TableView::Complete(TableRows::RelationOpinions(rows));
}
fn observe(scan: &mut prepared::Prepared, frame: &DecodedFrame) {
    scan.observe(
        frame,
        FormatVersion::CURRENT,
        &FrameChanges::alone(frame, FormatVersion::CURRENT),
        &flag(),
    )
    .unwrap();
}
fn source(frames: &[DecodedFrame]) -> dashboard_base::Source {
    test_fixtures::file_source(test_fixtures::replay(frames.to_vec())).0
}
fn prefix(source: &dashboard_base::Source, last: i32, complete: bool) -> Prefix {
    Prefix::new(
        source,
        1,
        Coverage {
            schema: coverage::SCHEMA,
            first_hour: source.block_windows(1)[0].first_hour,
            last_hour: last,
            complete,
        },
        None,
    )
}
fn publish(fixture: &Fixture, scan: &mut prepared::Prepared, frames: &[DecodedFrame]) {
    let mut writer = fixture.writer();
    scan.flush(&mut writer, 1, &flag()).unwrap();
    scan.save(&mut writer, 1).unwrap();
    let mut history = HistoryScan::new(false, &[]);
    for frame in frames {
        history.observe(frame);
    }
    writer
        .write_at(
            "country-identities-1",
            0,
            serde_json::to_vec(&history).unwrap().as_slice(),
        )
        .unwrap();
    writer.commit().unwrap();
}
fn ask(
    source: &dashboard_base::Source,
    fixture: &Fixture,
    value: serde_json::Value,
    last: i32,
) -> serde_json::Value {
    let params = serde_json::from_value(value).unwrap();
    query::query(
        source,
        &params,
        None,
        &fixture.store,
        &fixture.key,
        prefix(source, last, true),
        &flag(),
    )
    .unwrap()
}
fn opinion_params(hour: i32) -> serde_json::Value {
    serde_json::json!({"view":"opinions","branchId":"1","hour":hour,"tags":["first","second"]})
}
#[test]
fn sparse_zero_self_asymmetric_extremes_and_clear_are_exact() {
    let mut first = frame(0, 0);
    opinions(
        &mut first,
        &[(A, A, i32::MIN), (A, B, i32::MAX), (B, A, -17)],
    );
    let mut held = first.clone();
    held.hour = 1;
    let cleared = frame(2, 2);
    let frames = vec![first, held, cleared];
    let source = source(&frames);
    let fixture = Fixture::new();
    let mut scan = prepared::Prepared::default();
    for frame in &frames {
        observe(&mut scan, frame);
    }
    publish(&fixture, &mut scan, &frames);
    let reply = ask(&source, &fixture, opinion_params(1), 2);
    assert_eq!(reply["owners"][0]["known"], true);
    assert_eq!(reply["owners"][0]["readHour"], 0);
    assert_eq!(
        reply["owners"][0]["opinions"],
        serde_json::json!([{"target":"first","value":i32::MIN},{"target":"second","value":i32::MAX}])
    );
    let cleared = ask(&source, &fixture, opinion_params(2), 2);
    assert_eq!(cleared["owners"][0]["known"], true);
    assert_eq!(cleared["owners"][0]["opinions"], serde_json::json!([]));
    let pair = ask(
        &source,
        &fixture,
        serde_json::json!({"view":"pair","branchId":"1","first":"first","second":"second","hour":0}),
        2,
    );
    assert_eq!(pair["forward"][0]["value"], i32::MAX);
    assert_eq!(pair["reverse"][0]["value"], -17);
    assert_eq!(pair["forward"][1]["value"], 0);
}
#[test]
fn aliases_unresolved_targets_and_unavailable_reads_refuse_not_zero() {
    let mut duplicate = frame(0, 0);
    opinions(&mut duplicate, &[(A, B, 1), (9000, B, 2)]);
    let mut missing = frame(1, 1);
    *missing.tables[TableId::RelationOpinions.index()] = TableView::Unavailable {
        reason: "synthetic failure".into(),
    };
    let mut unknown = frame(2, 2);
    opinions(&mut unknown, &[(A, 99999, 7)]);
    let frames = vec![duplicate, missing, unknown];
    let source = source(&frames);
    let fixture = Fixture::new();
    let mut scan = prepared::Prepared::default();
    for frame in &frames {
        observe(&mut scan, frame);
    }
    publish(&fixture, &mut scan, &frames);
    for hour in [0, 1, 2] {
        let reply = ask(&source, &fixture, opinion_params(hour), 2);
        assert_eq!(reply["owners"][0]["known"], false);
        assert_eq!(reply["owners"][0]["opinions"], serde_json::json!([]));
    }
    assert!(
        ask(&source, &fixture, opinion_params(0), 2)["unresolved"]
            .as_u64()
            .unwrap()
            > 0
    );
}
#[test]
fn an_unproven_selected_country_leaves_every_other_pair_known() {
    let mut first = frame(0, 0);
    opinions(&mut first, &[(A, B, 5)]);
    // A third country appears after both owners' carried reads, which were made without it.
    let mut joined = frame(1, 0);
    opinions(&mut joined, &[(A, B, 5)]);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *joined.tables[TableId::Countries.index()]
    {
        rows.push(CountryRow {
            id: 1002,
            canonical_slot: 3,
            tag: b"third".to_vec(),
            ..Default::default()
        });
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *joined.tables[TableId::CountryLinks.index()]
    {
        rows.insert(
            2,
            CountryLinkRow {
                id: 1002,
                canonical_slot: 3,
            },
        );
    }
    let frames = vec![first, joined];
    let source = source(&frames);
    let fixture = Fixture::new();
    let mut scan = prepared::Prepared::default();
    for frame in &frames {
        observe(&mut scan, frame);
    }
    publish(&fixture, &mut scan, &frames);
    let matrix = |hour: i32, third: &str| {
        ask(
            &source,
            &fixture,
            serde_json::json!({"view":"opinions","branchId":"1","hour":hour,"tags":["first","second",third]}),
            1,
        )
    };
    // A country with no holder at the hour is unknown in every pair, and only there.
    let absent = matrix(0, "absent");
    for owner in [0_usize, 1] {
        assert_eq!(absent["owners"][owner]["known"], true);
        assert_eq!(
            absent["owners"][owner]["unknownTargets"],
            serde_json::json!(["absent"])
        );
    }
    assert_eq!(
        absent["owners"][0]["opinions"],
        serde_json::json!([{"target":"second","value":5}])
    );
    assert_eq!(absent["owners"][1]["opinions"], serde_json::json!([]));
    assert_eq!(absent["owners"][2]["identity"], serde_json::Value::Null);
    assert_eq!(absent["owners"][2]["known"], false);
    // A read made before a country existed proves nothing of it, yet the rest of it holds.
    let later = matrix(1, "third");
    for owner in [0_usize, 1] {
        assert_eq!(later["owners"][owner]["known"], true);
        assert_eq!(later["owners"][owner]["readHour"], 0);
        assert_eq!(
            later["owners"][owner]["unknownTargets"],
            serde_json::json!(["third"])
        );
    }
    assert_eq!(
        later["owners"][0]["opinions"],
        serde_json::json!([{"target":"second","value":5}])
    );
}
#[test]
fn owner_read_helper_preserves_rowless_scheduled_dirty_and_full_evidence() {
    let slot = replay_data::cadence::slice_of(A) as i32;
    let mut frame = frame(slot, slot);
    *frame.tables[TableId::SliceReads.index()] =
        TableView::Complete(TableRows::SliceReads(Vec::new()));
    assert_eq!(
        evidence::owner_read(&frame, FormatVersion::CURRENT, TableId::RelationOpinions, A),
        Some(slot)
    );
    frame.hour = slot + 1;
    assert_eq!(
        evidence::owner_read(&frame, FormatVersion::CURRENT, TableId::RelationOpinions, A),
        None
    );
    let daily = TableId::ALL
        .into_iter()
        .find(|&table| {
            table != TableId::CountryIntel
                && matches!(
                    replay_data::cadence::cadence(table),
                    replay_data::cadence::Cadence::Daily(_)
                )
                && !replay_data::cadence::is_refresh(table, frame.hour)
        })
        .unwrap();
    frame.metrics.table_ns[daily.index()] = 1;
    assert!(evidence::full_refresh(&frame, FormatVersion::CURRENT));
    assert_eq!(
        evidence::owner_read(&frame, FormatVersion::CURRENT, TableId::RelationOpinions, A),
        Some(frame.hour)
    );
    *frame.tables[TableId::RelationOpinions.index()] = TableView::Unavailable {
        reason: "not read".into(),
    };
    assert_eq!(
        evidence::owner_read(&frame, FormatVersion::CURRENT, TableId::RelationOpinions, A),
        None
    );
}
#[test]
fn owner_runs_give_each_owner_and_its_row_count_in_order() {
    let rows = [3u32, 3, 4, 9, 9, 9];
    let runs: Vec<_> = evidence::owner_runs(&rows, |row| *row).collect();
    assert_eq!(runs, [(3, 2), (4, 1), (9, 3)]);
    assert_eq!(evidence::owner_runs(&[] as &[u32], |row| *row).count(), 0);
}
#[test]
fn expired_reads_and_gaps_do_not_interpolate() {
    let mut first = frame(0, 0);
    opinions(&mut first, &[(A, B, 29)]);
    let mut expired = first.clone();
    expired.hour = 24;
    let frames = vec![first, expired];
    let source = source(&frames);
    let fixture = Fixture::new();
    let mut scan = prepared::Prepared::default();
    for frame in &frames {
        observe(&mut scan, frame);
    }
    publish(&fixture, &mut scan, &frames);
    let reply = ask(&source, &fixture, opinion_params(24), 24);
    assert_eq!(reply["owners"][0]["known"], false);
    let pair = ask(
        &source,
        &fixture,
        serde_json::json!({"view":"pair","branchId":"1","first":"first","second":"second","hour":0}),
        24,
    );
    assert!(
        pair["forward"]
            .as_array()
            .unwrap()
            .iter()
            .any(|run| run["value"].is_null())
    );
}
fn tuple(kind: u32, ordinal: u16, first: u32, second: u32) -> RelationTupleRow {
    RelationTupleRow {
        owner_country: A,
        target_country: B,
        type_id: kind,
        ordinal,
        first_country: first,
        second_country: second,
        start_hour: -40,
        end_hour: 777,
        cancel: 0,
        retired: 0,
    }
}
#[test]
fn all_seventeen_kinds_ordered_occurrences_offer_directions_and_raw_flags_survive() {
    let mut first = frame(0, 0);
    let mut tuples = Vec::new();
    let mut tokens = Vec::new();
    let recorded_kinds = [
        engine_native::cguarantee_relation::TYPE,
        engine_native::cnon_aggression_pact_relation::TYPE,
        engine_native::cimprove_relations_relation::TYPE,
        engine_native::cboost_party_popularity_relation::TYPE,
        engine_native::cembargo_relation::TYPE,
        engine_native::cnaval_blockade_relation::TYPE,
        engine_native::cattache_relation::TYPE,
        engine_native::clend_lease_relation::TYPE,
        engine_native::cequipment_purchase_relation::TYPE,
        engine_native::cmarket_access_rights_relation::TYPE,
        engine_native::clicence_production_access_relation::TYPE,
        engine_native::cmilitary_access_relation::TYPE,
        engine_native::coffer_military_access_relation::TYPE,
        engine_native::cdocking_rights_relation::TYPE,
        engine_native::coffer_docking_rights_relation::TYPE,
        engine_native::cair_base_access_relation::TYPE,
        engine_native::coffer_air_base_access_relation::TYPE,
    ];
    for (index, &kind) in recorded_kinds.iter().enumerate() {
        let mut row = tuple(
            kind,
            (index * 3) as u16,
            if index % 2 == 0 { 9000 } else { B },
            if index % 2 == 0 { B } else { 9000 },
        );
        if index == 16 {
            row.cancel = 9;
            row.retired = 4;
        }
        tuples.push(row);
        tokens.push(LexicalTokenRow {
            token: kind,
            literal: format!("synthetic_kind_{index}").into_bytes(),
        });
    }
    tuples.sort_by_key(RowCodec::key);
    tokens.sort_by_key(|row| row.token);
    *first.tables[TableId::RelationTuples.index()] =
        TableView::Complete(TableRows::RelationTuples(tuples));
    *first.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(tokens));
    let frames = vec![first];
    let source = source(&frames);
    let fixture = Fixture::new();
    let mut scan = prepared::Prepared::default();
    observe(&mut scan, &frames[0]);
    publish(&fixture, &mut scan, &frames);
    let reply = ask(
        &source,
        &fixture,
        serde_json::json!({"view":"pacts","branchId":"1","tags":null}),
        0,
    );
    assert_eq!(reply["pacts"].as_array().unwrap().len(), 17);
    assert_eq!(reply["kinds"].as_array().unwrap().len(), 17);
    let by_ordinal = |ordinal: u64| {
        reply["pacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["ordinal"] == ordinal)
            .unwrap()
    };
    assert_eq!(by_ordinal(0)["first"], "first");
    assert_eq!(by_ordinal(3)["first"], "second");
    assert_eq!(by_ordinal(3)["second"], "first");
    assert_eq!(by_ordinal(48)["cancelRaw"], 9);
    assert_eq!(by_ordinal(48)["cancelled"], serde_json::Value::Null);
    assert_eq!(reply["pacts"][0]["startHour"], -40);
    assert_eq!(reply["pacts"][0]["firstSeen"], 0);
    assert!(
        reply["kinds"]
            .as_array()
            .unwrap()
            .iter()
            .all(|kind| kind["name"].is_null() && kind["reason"].is_string())
    );
}
fn queue(frame: &mut DecodedFrame, stored: i32, ordinal: u32) {
    *frame.tables[TableId::IncomingDiplomaticActions.index()] =
        TableView::Complete(TableRows::IncomingDiplomaticActions(vec![
            IncomingDiplomaticActionRow {
                country: A,
                ordinal,
                handle: u64::MAX,
                status: 255,
                action_type: -12345,
                actor: B,
                recipient: A,
                original_actor: A,
                original_recipient: B,
                hour: stored,
                last_command_hour: stored + 7,
            },
        ]));
}
#[test]
fn daily_queue_null_slots_reordering_disappearance_handle_reuse_and_gap_bounds() {
    let mut a = frame(22, 22);
    queue(&mut a, 8, 9);
    let mut b = frame(46, 46);
    queue(&mut b, 8, 77);
    let c = frame(70, 70);
    let mut d = frame(94, 94);
    queue(&mut d, 90, 4);
    let mut gap = frame(142, 142);
    queue(&mut gap, 90, 5);
    let gone = frame(166, 166);
    // Provide continuous hours between daily reads, with explicit stamps.
    let mut frames = Vec::new();
    for hour in 22..=166 {
        if (110..142).contains(&hour) {
            continue;
        }
        let base = if hour >= 166 {
            &gone
        } else if hour >= 142 {
            &gap
        } else if hour >= 94 {
            &d
        } else if hour >= 70 {
            &c
        } else if hour >= 46 {
            &b
        } else {
            &a
        };
        let mut frame = base.clone();
        frame.hour = hour;
        frames.push(frame);
    }
    let source = source(&frames);
    let fixture = Fixture::new();
    let mut scan = prepared::Prepared::default();
    for frame in &frames {
        observe(&mut scan, frame);
    }
    publish(&fixture, &mut scan, &frames);
    let reply = ask(
        &source,
        &fixture,
        serde_json::json!({"view":"proposals","branchId":"1","tags":["second"]}),
        166,
    );
    let proposals = reply["proposals"].as_array().unwrap();
    assert_eq!(proposals.len(), 3);
    assert_eq!(proposals[0]["handle"], u64::MAX.to_string());
    assert_eq!(proposals[0]["goneBy"], 70);
    assert_eq!(proposals[0]["goneByBound"], false);
    assert_eq!(proposals[0]["firstSeen"], 22);
    assert_eq!(proposals[0]["lastSeen"], 46);
    assert_eq!(proposals[1]["goneByBound"], true);
    assert_eq!(proposals[2]["goneBy"], 166);
    assert!(proposals.iter().all(|p| p.get("kind").is_none()
        && p.get("outcome").is_none()
        && p.get("status").is_none()
        && p.get("actionType").is_none()));
}
#[test]
fn prepared_snapshot_parity_cold_pending_and_resume_append() {
    let mut frames = Vec::new();
    for hour in 0..=8 {
        let mut f = frame(hour, hour);
        opinions(&mut f, &[(A, B, if hour < 4 { 7 } else { -9 })]);
        frames.push(f);
    }
    let source = source(&frames);
    let fixture = Fixture::new();
    let cold = Prefix::new(
        &source,
        1,
        dashboard_base::prepared::prefix::extent(&source, 1, None).unwrap(),
        None,
    );
    let cold_reply = query::query(
        &source,
        &serde_json::from_value::<DiplomacyParams>(
            serde_json::json!({"view":"pacts","branchId":"1","tags":null}),
        )
        .unwrap(),
        None,
        &fixture.store,
        &fixture.key,
        cold,
        &flag(),
    )
    .unwrap();
    assert_eq!(cold_reply["pacts"], serde_json::json!([]));
    assert!(!cold_reply["pending"].as_array().unwrap().is_empty());
    let mut scan = prepared::Prepared::default();
    for frame in &frames[..4] {
        observe(&mut scan, frame);
    }
    publish(&fixture, &mut scan, &frames[..4]);
    let partial = query::query(
        &source,
        &serde_json::from_value(opinion_params(8)).unwrap(),
        None,
        &fixture.store,
        &fixture.key,
        prefix(&source, 3, false),
        &flag(),
    )
    .unwrap();
    assert_eq!(partial["snapshot"], true);
    assert_eq!(partial["owners"][0]["opinions"][0]["value"], -9);
    let saved = fixture
        .store
        .read(&fixture.key, &prepared::state_section(1), 0)
        .unwrap()
        .unwrap();
    scan = prepared::Prepared::restore(saved).unwrap();
    for frame in &frames[4..] {
        observe(&mut scan, frame);
    }
    publish(&fixture, &mut scan, &frames);
    let prepared = ask(&source, &fixture, opinion_params(8), 8);
    assert_eq!(prepared["owners"], partial["owners"]);
    let mut appended = fixture;
    let mut writer = appended.append_writer();
    let mut f = frame(9, 9);
    opinions(&mut f, &[(A, B, 33)]);
    observe(&mut scan, &f);
    scan.flush(&mut writer, 1, &flag()).unwrap();
    scan.save(&mut writer, 1).unwrap();
    writer.commit().unwrap();
    let reader_cancelled = flag();
    let mut reader = Reader::new(
        &appended.store,
        &appended.key,
        prepared::section(1),
        &reader_cancelled,
    )
    .unwrap();
    let a = DiplomacyCountry {
        tag: "first".into(),
        slot: 1,
        country_id: A,
    };
    let b = DiplomacyCountry {
        tag: "second".into(),
        slot: 2,
        country_id: B,
    };
    assert_eq!(
        reader
            .value_at(scan.keys[&prepared::pair_key(&a, &b)], 9)
            .unwrap()
            .unwrap()
            .value,
        33
    );
}
#[test]
fn pair_follows_rename_and_excludes_a_reused_tag_holder() {
    let mut frames = Vec::new();
    for hour in 0..=3 {
        let mut f = frame(hour, hour);
        opinions(&mut f, &[(A, B, 10 + hour)]);
        if hour >= 1
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *f.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"renamed".to_vec();
        }
        if hour >= 2 {
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *f.tables[TableId::Countries.index()]
            {
                rows.push(CountryRow {
                    id: 2000,
                    canonical_slot: 3,
                    tag: b"first".to_vec(),
                    ..Default::default()
                });
            }
            if let TableView::Complete(TableRows::CountryLinks(rows)) =
                &mut *f.tables[TableId::CountryLinks.index()]
            {
                rows.push(CountryLinkRow {
                    id: 2000,
                    canonical_slot: 3,
                });
                rows.sort_by_key(|r| r.id);
            }
        }
        frames.push(f);
    }
    let source = source(&frames);
    let fixture = Fixture::new();
    let mut scan = prepared::Prepared::default();
    for f in &frames {
        observe(&mut scan, f);
    }
    publish(&fixture, &mut scan, &frames);
    let reply = ask(
        &source,
        &fixture,
        serde_json::json!({"view":"pair","branchId":"1","first":"first","second":"second","hour":0}),
        3,
    );
    assert_eq!(reply["first"]["countryId"], A);
    assert_eq!(
        reply["forward"].as_array().unwrap().last().unwrap()["value"],
        13
    );
    assert!(
        reply["forward"]
            .as_array()
            .unwrap()
            .iter()
            .any(|run| run["firstHour"] == 1 && run["startsSegment"] == true)
    );
    // Chosen after the tag passed to another country, the pair is that holder's.
    let later = ask(
        &source,
        &fixture,
        serde_json::json!({"view":"pair","branchId":"1","first":"first","second":"second","hour":3}),
        3,
    );
    assert_ne!(later["first"]["countryId"], A);
}
#[test]
fn prepass_and_hourly_preparation_publish_equal_payloads() {
    let frames: Vec<_> = (0..=5)
        .map(|hour| {
            let mut f = frame(hour, hour);
            opinions(&mut f, &[(A, B, hour + 1)]);
            f
        })
        .collect();
    let replay = test_fixtures::replay(frames.clone());
    let mut direct = prepared::Prepared::default();
    for f in &frames {
        observe(&mut direct, f);
    }
    let mut folded = prepared::Prepared::default();
    for index in &replay.index {
        let facts =
            dashboard_base::prepared::prepass::extract_alone(&replay, index, &block::FAMILY)
                .unwrap();
        block::fold(
            &mut folded,
            facts,
            &replay,
            &mut |_, result| {
                result?;
                Ok(true)
            },
            &mut || Ok(()),
        )
        .unwrap();
    }
    let left = Fixture::new();
    let right = Fixture::new();
    publish(&left, &mut direct, &frames);
    publish(&right, &mut folded, &frames);
    let select = |fixture: &Fixture| {
        fixture
            .payloads()
            .into_iter()
            .filter(|((section, _), _)| section.starts_with("diplomacy"))
            .collect::<BTreeMap<_, _>>()
    };
    assert_eq!(select(&left), select(&right));
}
#[test]
fn cancellation_and_oversized_inventory_refuse_explicitly() {
    let f = frame(0, 0);
    let mut scan = prepared::Prepared::default();
    let cancelled = AtomicBool::new(true);
    assert_eq!(
        scan.observe(
            &f,
            FormatVersion::CURRENT,
            &FrameChanges::alone(&f, FormatVersion::CURRENT),
            &cancelled
        )
        .unwrap_err()
        .kind(),
        std::io::ErrorKind::Interrupted
    );
    let reply = DiplomacyProposalsReply {
        proposals: Vec::new(),
        reads: Vec::new(),
        unobserved: Vec::new(),
        coverage: Coverage {
            schema: coverage::SCHEMA,
            first_hour: 0,
            last_hour: 0,
            complete: true,
        },
        pending: Vec::new(),
        warnings: vec!["x".repeat(dashboard_base::products::MAX_PRODUCT_REPLY_BYTES)],
    };
    assert!(
        dashboard_base::products::check_reply_budget(&reply)
            .unwrap_err()
            .to_string()
            .contains("reply budget")
    );
}
fn assert_relation_title_mod_override(token: u32, expected_key: &str) {
    let root = std::env::temp_dir().join(format!("diplomacy-mod-{}", uuid::Uuid::new_v4()));
    let base = root.join("base");
    // A mod's plain file, and one in a `replace` folder, which overwrites earlier definitions.
    let overlay = root.join("mod");
    let replacing = root.join("replacing");
    for path in [
        base.join("localisation/english"),
        overlay.join("localisation/english"),
        replacing.join("localisation/english/replace"),
    ] {
        std::fs::create_dir_all(path).unwrap();
    }
    std::fs::write(
        base.join("localisation/english/base.yml"),
        format!("\u{feff}l_english:\n {expected_key}:0 \"Synthetic original\"\n"),
    )
    .unwrap();
    let changed = format!("\u{feff}l_english:\n {expected_key}:0 \"Changed by synthetic mod\"\n");
    std::fs::write(overlay.join("localisation/english/mod.yml"), &changed).unwrap();
    std::fs::write(
        replacing.join("localisation/english/replace/mod.yml"),
        &changed,
    )
    .unwrap();
    let base_layers = installed_content_core::ResolvedLayers::from_folders(&[(&base, &[])]).unwrap();
    let local = installed_content_localisation::localisation::read(&base_layers, "english").unwrap();
    let key = installed_content_core::diplomacy::relation_title_key(token, "synthetic relation").unwrap();
    assert_eq!(key, expected_key);
    assert_eq!(
        dashboard_text::catalogue::localised(&local, key, dashboard_text::catalogue::View::Plain)
            .unwrap(),
        "Synthetic original"
    );
    // A plain duplicate keeps the first definition, as the game does; a `replace` file or a
    // replace_path of the localisation folder gives the mod's title.
    for (mod_folder, replace, title) in [
        (&overlay, &[][..], "Synthetic original"),
        (&replacing, &[][..], "Changed by synthetic mod"),
        (&overlay, &["localisation"][..], "Changed by synthetic mod"),
    ] {
        let layers =
            installed_content_core::ResolvedLayers::from_folders(&[(&base, &[]), (mod_folder, replace)])
                .unwrap();
        let local = installed_content_localisation::localisation::read(&layers, "english").unwrap();
        assert_eq!(
            dashboard_text::catalogue::localised(
                &local,
                key,
                dashboard_text::catalogue::View::Plain
            )
            .unwrap(),
            title
        );
    }
}
macro_rules! relation_title_mod_test {
    ($name:ident, $kind:ident, $key:literal) => {
        #[test]
        fn $name() {
            assert_relation_title_mod_override(engine_native::$kind::TYPE, $key);
        }
    };
}
relation_title_mod_test!(
    guarantee_title_uses_recorded_mod,
    cguarantee_relation,
    "DIPLOMACY_GUARANTEE_TITLE"
);
relation_title_mod_test!(
    non_aggression_title_uses_recorded_mod,
    cnon_aggression_pact_relation,
    "DIPLOMACY_NONAGGRESSIONPACT"
);
relation_title_mod_test!(
    improve_relations_title_uses_recorded_mod,
    cimprove_relations_relation,
    "DIPLOMACY_IMPROVERELATION_TITLE"
);
relation_title_mod_test!(
    embargo_title_uses_recorded_mod,
    cembargo_relation,
    "DIPLOMACY_EMBARGO_TITLE"
);
relation_title_mod_test!(
    naval_blockade_title_uses_recorded_mod,
    cnaval_blockade_relation,
    "DIPLOMACY_NAVAL_BLOCKADE_TITLE"
);
relation_title_mod_test!(
    attache_title_uses_recorded_mod,
    cattache_relation,
    "DIPLOMACY_SEND_ATTACHE"
);
relation_title_mod_test!(
    market_access_title_uses_recorded_mod,
    cmarket_access_rights_relation,
    "DIPLOMACY_INTERNATIONAL_MARKET_ACCESS_RIGHTS"
);
relation_title_mod_test!(
    licence_access_title_uses_recorded_mod,
    clicence_production_access_relation,
    "DIPLOMACY_LICACC"
);
relation_title_mod_test!(
    military_access_title_uses_recorded_mod,
    cmilitary_access_relation,
    "DIPLOMACY_MILACC"
);
relation_title_mod_test!(
    offered_military_access_title_uses_recorded_mod,
    coffer_military_access_relation,
    "DIPLOMACY_MILACC"
);
relation_title_mod_test!(
    docking_rights_title_uses_recorded_mod,
    cdocking_rights_relation,
    "DIPLOMACY_DOCKING_RIGHTS"
);
relation_title_mod_test!(
    offered_docking_rights_title_uses_recorded_mod,
    coffer_docking_rights_relation,
    "DIPLOMACY_DOCKING_RIGHTS"
);
relation_title_mod_test!(
    air_base_access_title_uses_recorded_mod,
    cair_base_access_relation,
    "DIPLOMACY_AIR_BASE_ACCESS"
);
relation_title_mod_test!(
    offered_air_base_access_title_uses_recorded_mod,
    coffer_air_base_access_relation,
    "DIPLOMACY_AIR_BASE_ACCESS"
);

#[test]
fn relation_titles_refuse_unrecorded_operands_empty_names_and_unknown_kinds() {
    for (token, reason) in [
        (
            engine_native::cboost_party_popularity_relation::TYPE,
            "ideology-group",
        ),
        (
            engine_native::clend_lease_relation::TYPE,
            "supplies no title",
        ),
        (
            engine_native::cequipment_purchase_relation::TYPE,
            "supplies no title",
        ),
        (123, "not supported"),
    ] {
        let refusal =
            installed_content_core::diplomacy::relation_title_key(token, "mod-unknown-kind").unwrap_err();
        assert!(
            refusal.contains("mod-unknown-kind")
                && refusal.contains("count: 1")
                && refusal.contains(reason)
        );
    }
}

#[test]
fn opinions_do_not_wait_for_unavailable_tuples_or_daily_queue() {
    let mut f = frame(0, 0);
    opinions(&mut f, &[(A, B, 765)]);
    for table in [TableId::RelationTuples, TableId::IncomingDiplomaticActions] {
        *f.tables[table.index()] = TableView::Unavailable {
            reason: "independently unavailable".into(),
        };
    }
    let frames = vec![f];
    let source = source(&frames);
    let fixture = Fixture::new();
    let mut scan = prepared::Prepared::default();
    observe(&mut scan, &frames[0]);
    publish(&fixture, &mut scan, &frames);
    let reply = ask(&source, &fixture, opinion_params(0), 0);
    assert_eq!(reply["owners"][0]["known"], true);
    assert_eq!(reply["owners"][0]["opinions"][0]["value"], 765);
}

fn set_tuples(frame: &mut DecodedFrame, mut rows: Vec<RelationTupleRow>) {
    let mut tokens = rows
        .iter()
        .map(|row| LexicalTokenRow {
            token: row.type_id,
            literal: format!("synthetic relation {}", row.type_id).into_bytes(),
        })
        .collect::<Vec<_>>();
    tokens.sort_by_key(|row| row.token);
    tokens.dedup_by_key(|row| row.token);
    rows.sort_by_key(RowCodec::key);
    *frame.tables[TableId::RelationTuples.index()] =
        TableView::Complete(TableRows::RelationTuples(rows));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(tokens));
}
#[test]
fn unlinked_opinion_owners_are_counted_and_tuple_owners_are_not_dropped() {
    let mut f = frame(0, 0);
    opinions(&mut f, &[(99999, B, 3)]);
    let mut row = tuple(111, 0, A, B);
    row.owner_country = 99999;
    set_tuples(&mut f, vec![row]);
    if let TableView::Complete(TableRows::SliceReads(reads)) =
        &mut *f.tables[TableId::SliceReads.index()]
    {
        reads.push(SliceReadRow {
            table: TableId::RelationTuples.index() as u32,
            owner: 99999,
            sample_hour: 0,
        });
        reads.sort_by_key(|row| (row.table, row.owner));
    }
    let frames = vec![f];
    let source = source(&frames);
    let fixture = Fixture::new();
    let mut scan = prepared::Prepared::default();
    observe(&mut scan, &frames[0]);
    publish(&fixture, &mut scan, &frames);
    assert_eq!(
        ask(&source, &fixture, opinion_params(0), 0)["unresolved"],
        1
    );
    let reply = ask(
        &source,
        &fixture,
        serde_json::json!({"view":"pacts","branchId":"1","tags":null}),
        0,
    );
    assert_eq!(reply["pacts"].as_array().unwrap().len(), 1);
    assert!(reply["pacts"][0]["owner"].is_null());
    assert_eq!(reply["pacts"][0]["rawCountries"][0], 99999);
    assert!(!reply["warnings"].as_array().unwrap().is_empty());
}
#[test]
fn target_only_inventory_selection_includes_storage_and_queue_owner_reads() {
    let mut present = frame(22, 22);
    set_tuples(&mut present, vec![tuple(111, 0, A, B)]);
    queue(&mut present, 8, 0);
    if let TableView::Complete(TableRows::IncomingDiplomaticActions(rows)) =
        &mut *present.tables[TableId::IncomingDiplomaticActions.index()]
    {
        rows[0].actor = A;
        rows[0].recipient = B;
    }
    let absent = frame(46, 46);
    let frames: Vec<_> = (22..=46)
        .map(|hour| {
            let mut frame = if hour == 46 { &absent } else { &present }.clone();
            frame.hour = hour;
            frame
        })
        .collect();
    let source = source(&frames);
    let fixture = Fixture::new();
    let mut scan = prepared::Prepared::default();
    for frame in &frames {
        observe(&mut scan, frame);
    }
    publish(&fixture, &mut scan, &frames);
    let owner = serde_json::json!({"tag":"first","slot":1,"countryId":A});
    for view in ["pacts", "proposals"] {
        let reply = ask(
            &source,
            &fixture,
            serde_json::json!({"view":view,"branchId":"1","tags":["second"]}),
            46,
        );
        assert_eq!(reply[view].as_array().unwrap().len(), 1);
        assert_eq!(reply[view][0]["owner"], "first");
        assert_eq!(reply[view][0]["lastSeen"], 22);
        assert_eq!(reply[view][0]["presentAtEnd"], false);
        let reads: Vec<_> = reply["reads"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|read| read["owner"] == owner)
            .cloned()
            .collect();
        assert_eq!(
            reads,
            vec![
                serde_json::json!({"owner":owner,"firstHour":22,"lastHour":45,"readHour":22}),
                serde_json::json!({"owner":owner,"firstHour":46,"lastHour":46,"readHour":46}),
            ]
        );
    }
}
#[test]
fn queue_owner_read_evidence_stops_at_owner_identity_changes() {
    let frames: Vec<_> = (22..=46)
        .map(|hour| {
            let mut frame = frame(hour, if hour == 46 { 46 } else { 22 });
            queue(&mut frame, 8, 0);
            if hour > 22
                && let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
            {
                rows[0].tag = b"renamed".to_vec();
            }
            frame
        })
        .collect();
    let source = source(&frames);
    let fixture = Fixture::new();
    let mut scan = prepared::Prepared::default();
    for frame in &frames {
        observe(&mut scan, frame);
    }
    publish(&fixture, &mut scan, &frames);
    let reply = ask(
        &source,
        &fixture,
        serde_json::json!({"view":"proposals","branchId":"1","tags":["second"]}),
        46,
    );
    assert_eq!(reply["proposals"].as_array().unwrap().len(), 2);
    let reads: Vec<_> = reply["reads"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|read| !read["owner"].is_null())
        .cloned()
        .collect();
    assert_eq!(
        reads,
        vec![
            serde_json::json!({
                "owner":{"tag":"first","slot":1,"countryId":A},
                "firstHour":22,"lastHour":22,"readHour":22
            }),
            serde_json::json!({
                "owner":{"tag":"renamed","slot":1,"countryId":A},
                "firstHour":46,"lastHour":46,"readHour":46
            }),
        ]
    );
}
#[test]
fn tuple_read_proofs_close_on_unavailable_tables_and_resume_without_bridge() {
    let mut first = frame(0, 0);
    set_tuples(&mut first, vec![tuple(111, 3, A, B)]);
    let mut held = first.clone();
    held.hour = 1;
    let mut missing = held.clone();
    missing.hour = 2;
    *missing.tables[TableId::RelationTuples.index()] = TableView::Unavailable {
        reason: "synthetic unavailable".into(),
    };
    let mut again = frame(3, 3);
    set_tuples(&mut again, vec![tuple(111, 3, A, B)]);
    let frames = vec![first, held, missing, again];
    let source = source(&frames);
    let fixture = Fixture::new();
    let mut scan = prepared::Prepared::default();
    for f in &frames {
        observe(&mut scan, f);
    }
    publish(&fixture, &mut scan, &frames);
    let reply = ask(
        &source,
        &fixture,
        serde_json::json!({"view":"pacts","branchId":"1","tags":null}),
        3,
    );
    assert_eq!(reply["pacts"].as_array().unwrap().len(), 2);
    assert_eq!(reply["pacts"][0]["endedBy"], true);
    assert_eq!(reply["pacts"][0]["lastSeen"], 0);
    assert_eq!(reply["pacts"][1]["firstSeen"], 3);
    assert_eq!(
        reply["unobserved"],
        serde_json::json!([{"firstHour":2,"lastHour":2}])
    );
    assert!(
        reply["reads"]
            .as_array()
            .unwrap()
            .iter()
            .all(|read| read["firstHour"].as_i64().unwrap() > 2
                || read["lastHour"].as_i64().unwrap() < 2)
    );
}
#[test]
fn endpoint_identity_changes_end_pacts_and_queue_episodes_before_the_next_read() {
    let mut frames = Vec::new();
    for hour in 22..=46 {
        let mut f = frame(hour, if hour == 46 { 46 } else { 22 });
        opinions(&mut f, &[(A, B, 19)]);
        set_tuples(&mut f, vec![tuple(111, 0, A, B)]);
        queue(&mut f, 3, 4);
        if hour >= 23
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *f.tables[TableId::Countries.index()]
        {
            rows[1].tag = b"renamed".to_vec();
        }
        frames.push(f);
    }
    let source = source(&frames);
    let fixture = Fixture::new();
    let mut scan = prepared::Prepared::default();
    for f in &frames {
        observe(&mut scan, f);
    }
    publish(&fixture, &mut scan, &frames);
    let pacts = ask(
        &source,
        &fixture,
        serde_json::json!({"view":"pacts","branchId":"1","tags":null}),
        46,
    );
    assert_eq!(pacts["pacts"].as_array().unwrap().len(), 2);
    assert_eq!(pacts["pacts"][0]["endedBy"], true);
    assert_eq!(pacts["pacts"][0]["target"], "second");
    assert_eq!(pacts["pacts"][1]["target"], "renamed");
    let cancelled = flag();
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        prepared::section(1),
        &cancelled,
    )
    .unwrap();
    let old_target = DiplomacyCountry {
        tag: "second".into(),
        slot: 2,
        country_id: B,
    };
    let owner = DiplomacyCountry {
        tag: "first".into(),
        slot: 1,
        country_id: A,
    };
    let key = scan.keys[&prepared::pair_key(&owner, &old_target)];
    assert!(
        reader
            .runs(key, 22, 46)
            .unwrap()
            .iter()
            .all(|run| run.last_hour <= 22)
    );
    let proposals = ask(
        &source,
        &fixture,
        serde_json::json!({"view":"proposals","branchId":"1","tags":null}),
        46,
    );
    assert_eq!(proposals["proposals"].as_array().unwrap().len(), 2);
    assert_eq!(proposals["proposals"][0]["presentAtEnd"], false);
    assert_eq!(proposals["proposals"][0]["goneByBound"], true);
    assert!(proposals["proposals"][0]["goneBy"].is_null());
    assert_eq!(proposals["proposals"][1]["sender"], "renamed");
}
#[test]
fn unrelated_country_balances_do_not_reproject_held_opinion_rows() {
    let mut first = frame(0, 0);
    opinions(&mut first, &[(A, B, 19)]);
    let mut scan = prepared::Prepared::default();
    observe(&mut scan, &first);
    let count = scan.projected_rows;
    let mut held = first.clone();
    held.hour = 1;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *held.tables[TableId::Countries.index()]
    {
        rows[0].political_power_raw = Some(123456);
    }
    observe(&mut scan, &held);
    assert_eq!(scan.projected_rows, count);
}

#[test]
fn tagged_parameter_types_match_the_json_contract() {
    let declaration = DiplomacyParams::decl();
    assert!(declaration.contains("branchId"), "{declaration}");
    assert!(!declaration.contains("branch_id"), "{declaration}");
    for params in [
        opinion_params(0),
        serde_json::json!({"view":"pair","branchId":"1","first":"first","second":"second","hour":0}),
        serde_json::json!({"view":"pacts","branchId":"1","tags":null}),
        serde_json::json!({"view":"proposals","branchId":"1","tags":null}),
    ] {
        let decoded: DiplomacyParams = serde_json::from_value(params.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), params);
    }
}

#[test]
fn duplicate_queue_handles_refuse_only_the_queue_inventory() {
    let mut f = frame(22, 22);
    opinions(&mut f, &[(A, B, 51)]);
    queue(&mut f, 3, 4);
    if let TableView::Complete(TableRows::IncomingDiplomaticActions(rows)) =
        &mut *f.tables[TableId::IncomingDiplomaticActions.index()]
    {
        let mut duplicate = rows[0].clone();
        duplicate.ordinal = 7;
        rows.push(duplicate);
    }
    let frames = vec![f];
    let source = source(&frames);
    let fixture = Fixture::new();
    let mut scan = prepared::Prepared::default();
    observe(&mut scan, &frames[0]);
    publish(&fixture, &mut scan, &frames);
    let opinion = ask(&source, &fixture, opinion_params(22), 22);
    assert_eq!(opinion["owners"][0]["opinions"][0]["value"], 51);
    let error = query::query(
        &source,
        &DiplomacyParams::Proposals {
            branch_id: "1".into(),
            tags: None,
        },
        None,
        &fixture.store,
        &fixture.key,
        prefix(&source, 22, true),
        &flag(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("queue handles (count: 1)"));
}
#[test]
fn block_cancellation_is_checked_during_row_projection() {
    let mut f = frame(0, 0);
    opinions(&mut f, &[(A, A, 1), (A, B, 2)]);
    let mut scan = prepared::Prepared::default();
    let mut checks = 0;
    let error = scan
        .observe_checked(
            &f,
            FormatVersion::CURRENT,
            &FrameChanges::alone(&f, FormatVersion::CURRENT),
            &mut || {
                checks += 1;
                if checks >= 2 {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "synthetic cancel",
                    ))
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
}

#[test]
fn continuous_queue_handle_reuse_splits_and_bounds_the_previous_row() {
    let mut frames = Vec::new();
    for hour in 22..=46 {
        let mut f = frame(hour, hour);
        queue(&mut f, if hour < 46 { 3 } else { 40 }, 4);
        frames.push(f);
    }
    let source = source(&frames);
    let fixture = Fixture::new();
    let mut scan = prepared::Prepared::default();
    for f in &frames {
        observe(&mut scan, f);
    }
    publish(&fixture, &mut scan, &frames);
    let reply = ask(
        &source,
        &fixture,
        serde_json::json!({"view":"proposals","branchId":"1","tags":null}),
        46,
    );
    assert_eq!(reply["proposals"].as_array().unwrap().len(), 2);
    assert_eq!(reply["proposals"][0]["goneBy"], 46);
    assert_eq!(reply["proposals"][0]["goneByBound"], false);
    assert_eq!(reply["proposals"][1]["presentAtEnd"], true);
}

#[test]
fn reserved_target_zero_is_not_resolved_through_a_recorded_alias() {
    let mut f = frame(0, 0);
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *f.tables[TableId::CountryLinks.index()]
    {
        rows.insert(
            0,
            CountryLinkRow {
                id: 0,
                canonical_slot: 1,
            },
        );
    }
    opinions(&mut f, &[(A, 0, 37)]);
    let frames = vec![f];
    let source = source(&frames);
    let fixture = Fixture::new();
    let mut scan = prepared::Prepared::default();
    observe(&mut scan, &frames[0]);
    publish(&fixture, &mut scan, &frames);
    let reply = ask(&source, &fixture, opinion_params(0), 0);
    assert_eq!(reply["owners"][0]["known"], false);
    assert_eq!(reply["unresolved"], 1);
}
