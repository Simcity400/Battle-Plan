//! Diplomacy's replies of synthetic walks, each hour written as the native recorder writes it,
//! projected, emitted and published as the segment driver does; and the same replies however
//! the walk is cut into segments, published and resumed.
use super::*;
use dashboard_base::{
    prepared::{
        coverage::{self, Coverage},
        prefix::Prefix,
    },
    products::country_history::{HistoryScan, RecordedHistories},
    test_fixtures,
};
use dashboard_series::{
    Blob, Budget, Cache, Identity, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator, Start},
};
use replay_data::{
    format::FormatVersion,
    rows::{tables, *},
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};
use std::sync::{Arc, atomic::AtomicBool};

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

fn write<T: TableSchema>(walk: &mut Synthetic, input: &DecodedFrame) {
    let read = input.metrics.table_ns[T::ID.index()] != 0;
    let sample = Sample::FromReadBit;
    let input = match &*input.tables[T::ID.index()] {
        TableView::Complete(rows) => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: None,
            read,
            sample,
        },
        TableView::Truncated { rows, missing } => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: Some(*missing),
            read,
            sample,
        },
        TableView::Unavailable { reason } => Input::Unavailable {
            reason,
            read,
            sample,
        },
        TableView::NotRecorded { .. } => Input::NotRecorded { read, sample },
    };
    walk.table(T::ID, input).unwrap();
}

fn write_hour(walk: &mut Synthetic, input: &DecodedFrame) {
    write::<tables::Campaign>(walk, input);
    write::<tables::Countries>(walk, input);
    write::<tables::CountryLinks>(walk, input);
    write::<tables::SliceReads>(walk, input);
    write::<tables::RelationOpinions>(walk, input);
    write::<tables::RelationTuples>(walk, input);
    write::<tables::IncomingDiplomaticActions>(walk, input);
    write::<tables::LexicalTokens>(walk, input);
}

/// `frames` as runs of consecutive hours: an hour missing between two is a gap.
fn runs(frames: &[DecodedFrame]) -> Vec<&[DecodedFrame]> {
    let mut runs = Vec::new();
    let mut first = 0;
    for at in 1..=frames.len() {
        if at == frames.len() || frames[at].hour != frames[at - 1].hour + 1 {
            runs.push(&frames[first..at]);
            first = at;
        }
    }
    runs
}

/// The projected hours of `frames`, each run walked from empty tables, a projector of its own
/// starting a segment at each frame index `restarts` holds.
fn project(frames: &[DecodedFrame], restarts: &[usize]) -> Vec<facts::Hour> {
    let format = FormatVersion::CURRENT;
    let mut hours = Vec::new();
    let mut index = 0;
    for (run, inputs) in runs(frames).into_iter().enumerate() {
        let mut walk = Synthetic::new(facts::TABLES, 1, inputs[0].hour).unwrap();
        let mut held = None;
        let mut projector = facts::Projector::new(format).unwrap();
        projector
            .start(Start::Run { continues: run > 0 }, 1, inputs[0].hour)
            .unwrap();
        let mut first = 0;
        for (ordinal, input) in inputs.iter().enumerate() {
            if ordinal > 0 && restarts.contains(&(index + ordinal)) {
                projector.end(&mut hours).unwrap();
                projector = facts::Projector::new(format).unwrap();
                projector
                    .start(Start::Restart(walk.state()), 1, input.hour)
                    .unwrap();
                first = ordinal;
            }
            write_hour(&mut walk, input);
            walk.end(|state, manifest, frame, walk_starts| {
                assert_eq!(frame.hour, input.hour);
                let world = dashboard_world::World::at(
                    &mut held,
                    walk_starts,
                    &facts::needs().unwrap(),
                    (format, CaptureMode::Full),
                    state,
                    manifest,
                )
                .unwrap();
                projector
                    .hour(
                        &segments::Hour {
                            branch: 1,
                            hour: frame.hour,
                            run: run as u32,
                            ordinal: ordinal as u32,
                            starts: ordinal == first,
                            opens: ordinal == first,
                            state,
                            manifest,
                            world,
                        },
                        &mut hours,
                    )
                    .unwrap();
            })
            .unwrap();
        }
        projector.end(&mut hours).unwrap();
        index += inputs.len();
    }
    hours
}

fn identity() -> Identity {
    Identity {
        source: [1; 32],
        content: [2; 32],
        branch: [3; 32],
        schema: emit::SCHEMA,
    }
}

/// A publication of `hours`, cut after each of `cuts`' hours; at each of `resume`'s, the emit
/// is saved and goes on from the publication, as a preparation of a grown recording does: the
/// hour after one starts a segment, as a grown recording's next segment does.
fn build(hours: Vec<facts::Hour>, cuts: &[i32], resume: &[i32]) -> (Memory, u64) {
    let budget = Budget::new(1 << 30);
    let cache = Cache::new(&budget);
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(&budget, identity(), first).unwrap();
    let mut emit = emit::Emit::new();
    let mut root = 0;
    let mut hours = hours.into_iter().peekable();
    for (generation, &cut) in (1..).zip(cuts) {
        while let Some(hour) = hours.next_if(|hour| hour.hour <= cut) {
            emit.hour(&mut writer, hour).unwrap();
        }
        emit.cut(&mut writer, cut).unwrap();
        let publication = Publication {
            generation,
            segment: generation - 1,
            accepted: Span::new(first, cut).unwrap(),
            reached: cut,
            refusal: None,
        };
        root = publish(&mut writer, &mut pages, &cache, publication, Blob::EMPTY).unwrap();
        if resume.contains(&cut) {
            let mut saved = Vec::new();
            emit.save(&mut saved).unwrap();
            writer = Writer::resume(&mut pages, &cache, &budget, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), &cache, root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, emit::SCHEMA).unwrap();
            emit = emit::Emit::load(&directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none(), "every hour is cut");
    (pages, root)
}

/// A prepared walk of `frames`: its source, publication and identity histories.
struct Prepared {
    source: dashboard_base::Source,
    pages: Memory,
    root: u64,
    history: RecordedHistories,
}

impl Prepared {
    /// `frames` projected as one segment per run and published once.
    fn new(frames: &[DecodedFrame]) -> Self {
        let last = frames.last().unwrap().hour;
        Self::cut(frames, &[], &[last], &[])
    }

    fn cut(frames: &[DecodedFrame], restarts: &[usize], cuts: &[i32], resume: &[i32]) -> Self {
        let (pages, root) = build(project(frames, restarts), cuts, resume);
        let mut history = HistoryScan::new(false, &[]);
        for frame in frames {
            history.observe(frame);
        }
        Self {
            source: test_fixtures::file_source(test_fixtures::replay(frames.to_vec())).0,
            pages,
            root,
            history: history.finish(),
        }
    }

    fn prefix(&self, last: i32) -> Prefix {
        Prefix::new(
            &self.source,
            1,
            Coverage {
                schema: coverage::SCHEMA,
                first_hour: self.source.block_windows(1)[0].first_hour,
                last_hour: last,
                complete: true,
            },
            None,
        )
    }

    /// The reply through `last`, read from the publication of the last cut by then, as a query
    /// pins the gate's prefix.
    fn reply(&self, value: serde_json::Value, last: i32) -> std::io::Result<serde_json::Value> {
        let budget = Budget::new(1 << 30);
        let cache = Cache::new(&budget);
        let mut snapshot = Snapshot::open(self.pages.clone(), &cache, self.root, identity())
            .unwrap()
            .through(last)
            .unwrap()
            .unwrap();
        let directory = Directory::read(&mut snapshot, emit::SCHEMA).unwrap();
        let params: DiplomacyParams = serde_json::from_value(value).unwrap();
        let reply = query::reply(
            &self.source,
            &params,
            None,
            &mut snapshot,
            &directory,
            Some(&self.history),
            self.prefix(last),
            &flag(),
        )?;
        Ok(serde_json::to_value(reply).unwrap())
    }

    fn ask(&self, value: serde_json::Value, last: i32) -> serde_json::Value {
        self.reply(value, last).unwrap()
    }

    /// The run of `actor`'s `measure`, split by `dimension`, at `hour`.
    fn value(
        &self,
        actor: &DiplomacyCountry,
        measure: u64,
        dimension: Option<&DiplomacyCountry>,
        hour: i32,
    ) -> Option<dashboard_series::Run> {
        let budget = Budget::new(1 << 30);
        let cache = Cache::new(&budget);
        let mut snapshot =
            Snapshot::open(self.pages.clone(), &cache, self.root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, emit::SCHEMA).unwrap();
        let actor_of = |country| directory.actor(&emit::country_name(country)).unwrap();
        let key = directory.key(actor_of(actor), measure, dimension.map(actor_of))?;
        dashboard_base::prepared::engine::run_at(&mut snapshot, key, hour)
            .unwrap()
            .map(|(run, _)| run)
    }
}

fn opinion_params(hour: i32) -> serde_json::Value {
    serde_json::json!({"view":"opinions","branchId":"1","hour":hour,"tags":["first","second"]})
}

fn first() -> DiplomacyCountry {
    DiplomacyCountry {
        tag: "first".into(),
        slot: 1,
        country_id: A,
    }
}

fn second() -> DiplomacyCountry {
    DiplomacyCountry {
        tag: "second".into(),
        slot: 2,
        country_id: B,
    }
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
    let prepared = Prepared::new(&[first, held, cleared]);
    let reply = prepared.ask(opinion_params(1), 2);
    assert_eq!(reply["owners"][0]["known"], true);
    assert_eq!(reply["owners"][0]["readHour"], 0);
    assert_eq!(
        reply["owners"][0]["opinions"],
        serde_json::json!([{"target":"first","value":i32::MIN},{"target":"second","value":i32::MAX}])
    );
    let cleared = prepared.ask(opinion_params(2), 2);
    assert_eq!(cleared["owners"][0]["known"], true);
    assert_eq!(cleared["owners"][0]["opinions"], serde_json::json!([]));
    let pair = prepared.ask(
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
    let prepared = Prepared::new(&[duplicate, missing, unknown]);
    for hour in [0, 1, 2] {
        let reply = prepared.ask(opinion_params(hour), 2);
        assert_eq!(reply["owners"][0]["known"], false);
        assert_eq!(reply["owners"][0]["opinions"], serde_json::json!([]));
    }
    assert!(prepared.ask(opinion_params(0), 2)["unresolved"].as_u64().unwrap() > 0);
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
    let prepared = Prepared::new(&[first, joined]);
    let matrix = |hour: i32, third: &str| {
        prepared.ask(
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
fn rowless_owners_are_read_on_their_own_scheduled_slice_alone() {
    let slot = replay_data::cadence::slice_of(A) as i32;
    let frames: Vec<_> = [slot, slot + 1]
        .into_iter()
        .map(|hour| {
            let mut frame = frame(hour, hour);
            *frame.tables[TableId::SliceReads.index()] =
                TableView::Complete(TableRows::SliceReads(Vec::new()));
            frame
        })
        .collect();
    let hours = project(&frames, &[]);
    let read = |hour: &facts::Hour| {
        hour.opinions
            .as_ref()
            .unwrap()
            .current
            .iter()
            .map(|&(country, read, _)| (country, read))
            .collect::<Vec<_>>()
    };
    // The first is country 0, its tag ordering first.
    assert!(read(&hours[0]).contains(&(0, Some(slot))));
    assert!(read(&hours[1]).iter().all(|&(country, _)| country != 0));
    let mut unread = frames[0].clone();
    *unread.tables[TableId::RelationOpinions.index()] = TableView::Unavailable {
        reason: "not read".into(),
    };
    assert!(project(&[unread], &[])[0].opinions.is_none());
}

#[test]
fn expired_reads_and_gaps_do_not_interpolate() {
    let mut first = frame(0, 0);
    opinions(&mut first, &[(A, B, 29)]);
    let mut expired = first.clone();
    expired.hour = 24;
    let prepared = Prepared::new(&[first, expired]);
    let reply = prepared.ask(opinion_params(24), 24);
    assert_eq!(reply["owners"][0]["known"], false);
    let pair = prepared.ask(
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
    let prepared = Prepared::new(&[first]);
    let reply = prepared.ask(
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
    assert_eq!(reply["pacts"][0]["id"], "pact:0:0");
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
    let prepared = Prepared::new(&frames);
    let reply = prepared.ask(
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
    assert_eq!(proposals[0]["id"], "proposal:22:0");
    assert_eq!(proposals[1]["goneByBound"], true);
    assert_eq!(proposals[2]["goneBy"], 166);
    assert!(proposals.iter().all(|p| p.get("kind").is_none()
        && p.get("outcome").is_none()
        && p.get("status").is_none()
        && p.get("actionType").is_none()));
}

#[test]
fn an_unpublished_branch_answers_every_accepted_hour_unobserved() {
    let frames = vec![frame(0, 0)];
    let source = test_fixtures::file_source(test_fixtures::replay(frames)).0;
    let cold = Prefix::new(
        &source,
        1,
        dashboard_base::prepared::prefix::extent(&source, 1, None).unwrap(),
        None,
    );
    let params = serde_json::from_value::<DiplomacyParams>(
        serde_json::json!({"view":"pacts","branchId":"1","tags":null}),
    )
    .unwrap();
    let reply = serde_json::to_value(query::unpublished(&params, cold).unwrap()).unwrap();
    assert_eq!(reply["pacts"], serde_json::json!([]));
    assert_eq!(reply["unobserved"], serde_json::json!([]));
    assert!(!reply["pending"].as_array().unwrap().is_empty());
}

/// Every view's reply at every hour of `frames`.
fn replies(prepared: &Prepared, frames: &[DecodedFrame], last: i32) -> Vec<serde_json::Value> {
    let mut replies = Vec::new();
    for view in ["pacts", "proposals"] {
        for tags in [serde_json::Value::Null, serde_json::json!(["second"])] {
            replies.push(prepared.ask(
                serde_json::json!({"view":view,"branchId":"1","tags":tags}),
                last,
            ));
        }
    }
    for frame in frames.iter().filter(|frame| frame.hour <= last) {
        replies.push(prepared.ask(opinion_params(frame.hour), last));
        replies.push(prepared.ask(
            serde_json::json!({"view":"pair","branchId":"1","first":"first","second":"second","hour":frame.hour}),
            last,
        ));
    }
    replies
}

/// Opinions, relations and the queue changing over two daily reads, a rename between them.
fn campaign() -> Vec<DecodedFrame> {
    (22..=70)
        .map(|hour| {
            let read = hour - (hour - 22) % 24;
            let mut f = frame(hour, read);
            opinions(&mut f, &[(A, B, if hour < 46 { 19 } else { -4 }), (B, A, 3)]);
            let mut back = tuple(112, 0, B, A);
            back.owner_country = B;
            back.target_country = A;
            back.end_hour = if hour < 50 { 900 } else { 950 };
            set_tuples(&mut f, vec![tuple(111, 0, A, B), back]);
            queue(&mut f, 3, 4);
            if hour >= 40
                && let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *f.tables[TableId::Countries.index()]
            {
                rows[1].tag = b"renamed".to_vec();
            }
            f
        })
        .collect()
}

#[test]
fn segments_cuts_and_resumes_publish_the_same_replies() {
    let frames = campaign();
    let last = frames.last().unwrap().hour;
    let whole = replies(&Prepared::new(&frames), &frames, last);
    for (restarts, cuts, resume) in [
        (&[10_usize, 30][..], &[last][..], &[][..]),
        (&[24][..], &[30, 45, last][..], &[45][..]),
        (&[8, 24, 25][..], &[29, 45, 46, last][..], &[29, 46][..]),
    ] {
        let prepared = Prepared::cut(&frames, restarts, cuts, resume);
        assert_eq!(
            replies(&prepared, &frames, last),
            whole,
            "{restarts:?} {cuts:?} {resume:?}"
        );
    }
    // A cut's publication answers its own prefix as a walk ending there does.
    let early = Prepared::cut(&frames, &[24], &[45, last], &[45]);
    let shorter: Vec<_> = frames.iter().filter(|f| f.hour <= 45).cloned().collect();
    assert_eq!(
        replies(&Prepared::new(&shorter), &shorter, 45),
        replies(&early, &frames, 45)
    );
}

#[test]
fn a_resumed_emit_goes_on_with_what_it_held() {
    let mut frames = Vec::new();
    for hour in 0..=9 {
        let mut f = frame(hour, hour);
        opinions(&mut f, &[(A, B, if hour < 4 { 7 } else if hour < 9 { -9 } else { 33 })]);
        frames.push(f);
    }
    let resumed = Prepared::cut(&frames, &[4, 9], &[3, 8, 9], &[3, 8]);
    assert_eq!(
        resumed.ask(opinion_params(8), 9)["owners"][0]["opinions"][0]["value"],
        -9
    );
    assert_eq!(
        resumed
            .value(&first(), emit::OPINION, Some(&second()), 9)
            .unwrap()
            .value,
        33
    );
    assert_eq!(
        replies(&resumed, &frames, 9),
        replies(&Prepared::new(&frames), &frames, 9)
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
    let prepared = Prepared::new(&frames);
    let reply = prepared.ask(
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
    let later = prepared.ask(
        serde_json::json!({"view":"pair","branchId":"1","first":"first","second":"second","hour":3}),
        3,
    );
    assert_ne!(later["first"]["countryId"], A);
}

#[test]
fn oversized_inventory_refuses_explicitly() {
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
    let prepared = Prepared::new(&[f]);
    let reply = prepared.ask(opinion_params(0), 0);
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
    let prepared = Prepared::new(&[f]);
    assert_eq!(prepared.ask(opinion_params(0), 0)["unresolved"], 1);
    let reply = prepared.ask(
        serde_json::json!({"view":"pacts","branchId":"1","tags":null}),
        0,
    );
    assert_eq!(reply["pacts"].as_array().unwrap().len(), 1);
    assert!(reply["pacts"][0]["owner"].is_null());
    assert_eq!(reply["pacts"][0]["rawCountries"][0], 99999);
    assert_eq!(reply["pacts"][0]["lastSeen"], 0);
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
    let prepared = Prepared::new(&frames);
    let owner = serde_json::json!({"tag":"first","slot":1,"countryId":A});
    for view in ["pacts", "proposals"] {
        let reply = prepared.ask(
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
    let prepared = Prepared::new(&frames);
    let reply = prepared.ask(
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
    let prepared = Prepared::new(&[first, held, missing, again]);
    let reply = prepared.ask(
        serde_json::json!({"view":"pacts","branchId":"1","tags":null}),
        3,
    );
    assert_eq!(reply["pacts"].as_array().unwrap().len(), 2);
    assert_eq!(reply["pacts"][0]["endedBy"], true);
    assert_eq!(reply["pacts"][0]["lastSeen"], 0);
    assert_eq!(reply["pacts"][1]["firstSeen"], 3);
    assert_eq!(reply["pacts"][1]["lastSeen"], 3);
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
    let prepared = Prepared::new(&frames);
    let pacts = prepared.ask(
        serde_json::json!({"view":"pacts","branchId":"1","tags":null}),
        46,
    );
    assert_eq!(pacts["pacts"].as_array().unwrap().len(), 2);
    assert_eq!(pacts["pacts"][0]["endedBy"], true);
    assert_eq!(pacts["pacts"][0]["target"], "second");
    assert_eq!(pacts["pacts"][1]["target"], "renamed");
    // The pair toward the old identity ended where it left.
    assert!(
        prepared
            .value(&first(), emit::OPINION, Some(&second()), 23)
            .is_none()
    );
    assert_eq!(
        prepared
            .value(&first(), emit::OPINION, Some(&second()), 22)
            .unwrap()
            .span
            .last,
        22
    );
    let proposals = prepared.ask(
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
fn a_departed_identity_keeps_off_the_carry_what_its_return_reads() {
    // The first country takes another tag every hour and comes back once, unread then.
    let tags = ["first", "away1", "away2", "away3", "first"];
    let frames: Vec<_> = tags
        .iter()
        .enumerate()
        .map(|(hour, tag)| {
            let hour = hour as i32;
            let mut f = frame(hour, hour.min(3));
            opinions(&mut f, &[(A, B, 7)]);
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *f.tables[TableId::Countries.index()]
            {
                rows[0].tag = tag.as_bytes().to_vec();
            }
            f
        })
        .collect();
    let back = Prepared::new(&frames).ask(opinion_params(4), 4);
    // The read it left with, which proves nothing now.
    assert_eq!(back["owners"][0]["readHour"], 0);
    assert_eq!(back["owners"][0]["known"], false);
    let budget = Budget::new(1 << 30);
    let mut writer = Writer::new(&budget, identity(), 0).unwrap();
    let mut emit = emit::Emit::new();
    let mut open = Vec::new();
    for hour in project(&frames[..4], &[]) {
        let at = hour.hour;
        emit.hour(&mut writer, hour).unwrap();
        emit.cut(&mut writer, at).unwrap();
        open.push(emit.open_runs());
    }
    // Past each cut, one live first country's runs, however many tags it held before.
    assert_eq!(open[1], open[2]);
    assert_eq!(open[2], open[3]);
}

#[test]
fn unrelated_country_balances_do_not_reproject_held_opinion_rows() {
    let mut first = frame(0, 0);
    opinions(&mut first, &[(A, B, 19)]);
    let mut held = frame(1, 1);
    opinions(&mut held, &[(A, B, 19)]);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *held.tables[TableId::Countries.index()]
    {
        rows[0].political_power_raw = Some(123456);
    }
    let hours = project(&[first, held], &[]);
    let values = |hour: &facts::Hour| Arc::clone(&hour.opinions.as_ref().unwrap().current[0].2);
    assert!(hours[1].ids.is_none());
    assert!(Arc::ptr_eq(&values(&hours[0]), &values(&hours[1])));
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
    let prepared = Prepared::new(&[f]);
    let opinion = prepared.ask(opinion_params(22), 22);
    assert_eq!(opinion["owners"][0]["opinions"][0]["value"], 51);
    let error = prepared
        .reply(
            serde_json::json!({"view":"proposals","branchId":"1","tags":null}),
            22,
        )
        .unwrap_err();
    assert!(error.to_string().contains("queue handles (count: 1)"));
}

#[test]
fn a_handle_held_across_interruptions_ends_every_episode_at_its_absence_alone() {
    let frames: Vec<_> = (22..=118)
        .map(|hour| {
            let mut f = frame(hour, hour);
            if hour < 118 {
                queue(&mut f, 8, 0);
            }
            if hour == 50 || hour == 75 {
                *f.tables[TableId::IncomingDiplomaticActions.index()] = TableView::Unavailable {
                    reason: "synthetic interruption".into(),
                };
            }
            f
        })
        .collect();
    let prepared = Prepared::cut(&frames, &[], &[117, 118], &[]);
    let reply = prepared.ask(
        serde_json::json!({"view":"proposals","branchId":"1","tags":null}),
        118,
    );
    let field = |name: &str| {
        reply["proposals"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p[name].clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(field("lastSeen"), [46, 70, 94]);
    assert_eq!(field("goneBy"), [118, 118, 118]);
    assert_eq!(field("goneByBound"), [true, true, false]);
    // Before the read without the handle, what the interruptions left stays unknown.
    let before = prepared.ask(
        serde_json::json!({"view":"proposals","branchId":"1","tags":null}),
        117,
    );
    assert!(
        before["proposals"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["goneBy"].is_null())
    );
    // Only present proposals are held past a cut, however many episodes the handle had.
    let budget = Budget::new(1 << 30);
    let mut writer = Writer::new(&budget, identity(), 22).unwrap();
    let mut emit = emit::Emit::new();
    let mut held = Vec::new();
    for hour in project(&frames, &[]) {
        let at = hour.hour;
        emit.hour(&mut writer, hour).unwrap();
        if [49, 74, 99, 118].contains(&at) {
            emit.cut(&mut writer, at).unwrap();
            held.push(emit.held_proposals());
        }
    }
    assert_eq!(held, [1, 1, 1, 0]);
}

#[test]
fn a_kind_names_the_token_its_latest_read_names() {
    let guarantee = engine_native::cguarantee_relation::TYPE;
    let frames: Vec<_> = (0..=1)
        .map(|hour| {
            let mut f = frame(hour, 0);
            let mut other = tuple(123, 0, B, A);
            other.owner_country = B;
            other.target_country = A;
            let mut rows = vec![tuple(guarantee, 0, A, B), other];
            rows.sort_by_key(RowCodec::key);
            *f.tables[TableId::RelationTuples.index()] =
                TableView::Complete(TableRows::RelationTuples(rows));
            let mut tokens = vec![
                LexicalTokenRow {
                    token: guarantee,
                    literal: b"same".to_vec(),
                },
                LexicalTokenRow {
                    token: 123,
                    literal: b"same".to_vec(),
                },
            ];
            tokens.sort_by_key(|row| row.token);
            *f.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(tokens));
            // Only the first owner's relations are read again, unchanged.
            if hour == 1
                && let TableView::Complete(TableRows::SliceReads(reads)) =
                    &mut *f.tables[TableId::SliceReads.index()]
            {
                for read in reads.iter_mut().filter(|read| {
                    read.owner == A && read.table == TableId::RelationTuples.index() as u32
                }) {
                    read.sample_hour = 1;
                }
            }
            f
        })
        .collect();
    let reason = |last: usize| {
        Prepared::new(&frames[..=last]).ask(
            serde_json::json!({"view":"pacts","branchId":"1","tags":null}),
            last as i32,
        )["kinds"][0]["reason"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert!(reason(0).contains("not supported"), "{}", reason(0));
    assert!(
        reason(1).contains("recorded content is not configured"),
        "{}",
        reason(1)
    );
}

#[test]
fn continuous_queue_handle_reuse_splits_and_bounds_the_previous_row() {
    let mut frames = Vec::new();
    for hour in 22..=46 {
        let mut f = frame(hour, hour);
        queue(&mut f, if hour < 46 { 3 } else { 40 }, 4);
        frames.push(f);
    }
    let prepared = Prepared::new(&frames);
    let reply = prepared.ask(
        serde_json::json!({"view":"proposals","branchId":"1","tags":null}),
        46,
    );
    assert_eq!(reply["proposals"].as_array().unwrap().len(), 2);
    assert_eq!(reply["proposals"][0]["goneBy"], 46);
    assert_eq!(reply["proposals"][0]["goneByBound"], false);
    assert_eq!(reply["proposals"][1]["presentAtEnd"], true);
    assert_eq!(reply["proposals"][1]["lastSeen"], 46);
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
    let prepared = Prepared::new(&[f]);
    let reply = prepared.ask(opinion_params(0), 0);
    assert_eq!(reply["owners"][0]["known"], false);
    assert_eq!(reply["unresolved"], 1);
}

#[test]
fn reads_expire_exactly_a_period_after_their_supplying_hour() {
    let frames: Vec<_> = (0..=24)
        .map(|hour| {
            let mut f = frame(hour, 0);
            opinions(&mut f, &[(A, B, 5), (B, A, 6)]);
            let mut back = tuple(112, 0, B, A);
            back.owner_country = B;
            back.target_country = A;
            let mut orphan = tuple(113, 0, A, B);
            orphan.owner_country = 99999;
            set_tuples(&mut f, vec![tuple(111, 0, A, B), back, orphan]);
            if let TableView::Complete(TableRows::SliceReads(reads)) =
                &mut *f.tables[TableId::SliceReads.index()]
            {
                reads.push(SliceReadRow {
                    table: TableId::RelationTuples.index() as u32,
                    owner: 99999,
                    sample_hour: 0,
                });
                // Only B is read again before its first read expires.
                for read in reads
                    .iter_mut()
                    .filter(|read| read.owner == B && hour >= 12)
                {
                    read.sample_hour = 12;
                }
                reads.sort_by_key(|row| (row.table, row.owner));
            }
            f
        })
        .collect();
    let through = |last: usize| {
        let prepared = Prepared::new(&frames[..=last]);
        let hour = last as i32;
        let reads = [first(), second()].map(|country| {
            let owner = prepared
                .value(&country, emit::OWNER, None, hour)
                .and_then(|run| (run.value & 1 == 1).then_some(run.value >> 1));
            let tuples = prepared
                .value(&country, emit::TUPLES, None, hour)
                .map(|run| run.value);
            (owner, tuples)
        });
        let pacts = prepared.ask(
            serde_json::json!({"view":"pacts","branchId":"1","tags":null}),
            hour,
        );
        let mut present: Vec<_> = pacts["pacts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|pact| {
                (
                    pact["rawCountries"][0].as_u64().unwrap(),
                    pact["presentAtEnd"].as_bool().unwrap(),
                )
            })
            .collect();
        present.sort_unstable();
        (reads, present)
    };
    let (reads, present) = through(23);
    assert_eq!(reads, [(Some(0), Some(0)), (Some(12), Some(12))]);
    assert_eq!(present, [(1000, true), (1001, true), (99999, true)]);
    let (reads, present) = through(24);
    assert_eq!(reads, [(None, None), (Some(12), Some(12))]);
    assert_eq!(present, [(1000, false), (1001, true), (99999, false)]);
}

#[test]
fn unread_opinions_leave_their_orphan_rows_out_of_the_unresolved_count() {
    let mut frames: Vec<_> = (0..3).map(|hour| frame(hour, hour)).collect();
    for f in &mut frames {
        opinions(f, &[(99999, B, 3)]);
    }
    *frames[1].tables[TableId::RelationOpinions.index()] = TableView::Unavailable {
        reason: "synthetic failure".into(),
    };
    let prepared = Prepared::new(&frames);
    let unresolved = |hour| prepared.ask(opinion_params(hour), 2)["unresolved"].clone();
    assert_eq!(unresolved(0), 1);
    assert_eq!(unresolved(1), 0);
    assert_eq!(unresolved(2), 1);
}
