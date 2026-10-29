use super::*;
use crate::products::politics_state::tests::{Changed, balance, countries, frame, member, set};
use dashboard_base::prepared::series::tests::Fixture;
use replay_data::rows::*;
use replay_frame::frame::TableView;

fn publish(state: &mut Prepared, fixture: &Fixture) {
    let flag = AtomicBool::new(false);
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
}

fn ask(fixture: &Fixture, params: PoliticsHistoryParams, through: i32) -> PoliticsHistoryReply {
    query(
        &fixture.store,
        &fixture.key,
        &params,
        Some(through),
        None,
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn params(mode: PoliticsHistoryMode, tag: Option<&str>) -> PoliticsHistoryParams {
    PoliticsHistoryParams {
        branch_id: "1".into(),
        mode,
        tag: tag.map(str::to_owned),
        first_hour: None,
        last_hour: None,
    }
}

fn window(first: i32, last: i32) -> PoliticsHistoryParams {
    PoliticsHistoryParams {
        first_hour: Some(first),
        last_hour: Some(last),
        ..params(PoliticsHistoryMode::Exiles, None)
    }
}

fn span(first_hour: i32, last_hour: i32) -> HourSpan {
    HourSpan {
        first_hour,
        last_hour,
    }
}

fn politics(frame: &mut DecodedFrame, rows: Vec<PoliticsRow>) {
    set(frame, TableId::Politics, TableRows::Politics(rows));
}

fn owned_states(frame: &mut DecodedFrame, owned: &[(u32, i32)]) {
    let rows = owned
        .iter()
        .map(|&(country, owned_state_count)| CountryCounterInputRow {
            country,
            owned_state_count,
            ..Default::default()
        })
        .collect();
    set(frame, TableId::CountryCounterInputs, TableRows::CountryCounterInputs(rows));
}

fn run(first: i32, last: i32, value: &str) -> PoliticsRun {
    PoliticsRun {
        first_hour: first,
        last_hour: last,
        value: value.into(),
    }
}

#[test]
fn election_runs_change_only_with_the_schedule_and_never_cross_a_gap() {
    let fixture = Fixture::new();
    let mut state = Prepared::default();
    let schedule = |held: Recorded<bool>, next: i32| PoliticsRow {
        country: 1,
        // Interval zero keeps the stored date: the next election is the last.
        last_election: engine_native::cdate::HOUR_EPOCH + 1936 * 365 * 24 + next,
        has_elections: held,
        election_interval: 0.into(),
        ..Default::default()
    };
    for (hour, row) in [
        (100, schedule(true.into(), 5_000)),
        (101, schedule(true.into(), 5_000)),
        (102, schedule(false.into(), 6_000)),
        // A gap: 103 and 104 were not recorded.
        (105, schedule(Recorded::NotRecorded, 6_000)),
    ] {
        let mut at = frame(hour);
        countries(&mut at, &[(1, 1, "ELA")], &[]);
        owned_states(&mut at, &[(1, 3)]);
        politics(&mut at, vec![row]);
        state.step(&at);
    }
    publish(&mut state, &fixture);
    let reply = ask(&fixture, params(PoliticsHistoryMode::Elections, None), 105);
    let history = &reply.elections[0];
    assert_eq!(history.tags[0].tag, "ELA");
    // Known "not held" is its own value, an unrecorded one has no run.
    assert_eq!(history.held, vec![run(100, 101, "1"), run(102, 102, "0")]);
    assert_eq!(history.interval[0].first_hour, 100);
    // A stored date is the term's start; it changes with the schedule.
    assert_eq!(history.last.len(), 3);
    assert_eq!(history.last[0].first_hour, 100);
    assert_eq!(
        history.next,
        [(100, 101, 5_000), (102, 102, 6_000), (105, 105, 6_000)]
            .map(|(first, last, offset)| run(
                first,
                last,
                &(engine_native::cdate::HOUR_EPOCH + 1936 * 365 * 24 + offset).to_string()
            ))
            .to_vec()
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("eligibility not recorded"))
    );
}

#[test]
fn elections_are_held_only_by_countries_owning_states() {
    let fixture = Fixture::new();
    let mut state = Prepared::default();
    let row = PoliticsRow {
        country: 1,
        last_election: engine_native::cdate::HOUR_EPOCH + 1936 * 365 * 24,
        has_elections: true.into(),
        election_interval: 0.into(),
        ..Default::default()
    };
    // The schedule never changes; only the states owned do (none recorded at 102).
    for (hour, owned) in [(100, &[(1, 2)][..]), (101, &[(1, 0)]), (102, &[]), (103, &[(1, 4)])] {
        let mut at = frame(hour);
        countries(&mut at, &[(1, 1, "ELA")], &[]);
        owned_states(&mut at, owned);
        politics(&mut at, vec![row.clone()]);
        state.step(&at);
    }
    publish(&mut state, &fixture);
    let reply = ask(&fixture, params(PoliticsHistoryMode::Elections, None), 103);
    assert_eq!(
        reply.elections[0].held,
        vec![run(100, 100, "1"), run(101, 101, "0"), run(103, 103, "1")]
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("country existence not recorded"))
    );
}

#[test]
fn country_history_follows_the_holder_not_a_reused_tag() {
    let fixture = Fixture::new();
    let mut state = Prepared::default();
    let observe = |state: &mut Prepared, hour, slot_tag: &[(u32, u32, &str)], value| {
        let mut at = frame(hour);
        countries(&mut at, slot_tag, &[]);
        set(
            &mut at,
            TableId::PowerBalances,
            TableRows::PowerBalances(vec![PowerBalanceRow {
                active_range: b"mod_range_a".to_vec(),
                ..balance(0, "mod_balance", value)
            }]),
        );
        set(
            &mut at,
            TableId::PowerBalanceMembers,
            TableRows::PowerBalanceMembers(vec![member(0, 0, 1)]),
        );
        state.step(&at);
    };
    observe(&mut state, 100, &[(1, 1, "BPA")], -10_000);
    observe(&mut state, 101, &[(1, 1, "BPA")], -20_000);
    // The holder takes another tag, and a new holder takes the old one.
    observe(&mut state, 102, &[(1, 1, "BPZ"), (2, 2, "BPA")], -30_000);
    publish(&mut state, &fixture);
    // Every holder that held the tag, each whole: the caller picks the one
    // holding it at its hour.
    let reply = ask(&fixture, params(PoliticsHistoryMode::Country, Some("BPA")), 102);
    assert_eq!(
        reply.holder_balances.iter().map(|holder| holder.holder.as_str()).collect::<Vec<_>>(),
        ["1", "2"]
    );
    let renamed = &reply.holder_balances[0];
    let history = &renamed.balances[0];
    assert_eq!(history.token, "mod_balance");
    // The holder's history continues through its rename.
    assert_eq!(
        history.values,
        vec![run(100, 100, "-10000"), run(101, 101, "-20000"), run(102, 102, "-30000")]
    );
    assert_eq!(history.range, vec![run(100, 102, "mod_range_a")]);
    assert_eq!(renamed.shown, vec![run(100, 102, "mod_balance")]);
    // The new holder of the old tag has no balance history.
    assert!(reply.holder_balances[1].balances.is_empty());
    // Content is unavailable: names are absent, never invented.
    assert!(reply.names.is_empty());
    // Asking by the new tag finds the same holder, BPA-era history included.
    let by_new = ask(&fixture, params(PoliticsHistoryMode::Country, Some("BPZ")), 102);
    assert_eq!(by_new.holder_balances.len(), 1);
    assert_eq!(by_new.holder_balances[0].balances[0].values.len(), 3);
    assert_eq!(by_new.known, vec![span(100, 102)]);
}

#[test]
fn exile_and_balance_histories_cover_every_holder_with_independent_fields() {
    let fixture = Fixture::new();
    let mut state = Prepared::default();
    let observe = |state: &mut Prepared, hour: i32, leaders: Recorded<Option<i32>>, host: i32| {
        let mut at = frame(hour);
        countries(&mut at, &[(1, 1, "EXA"), (2, 2, "HSA"), (3, 3, "BPC")], &[]);
        set(
            &mut at,
            TableId::CountryStabilityInputs,
            TableRows::CountryStabilityInputs(vec![
                CountryStabilityInputRow {
                    country: 1,
                    exile_host: Some(host).into(),
                    exile_legitimacy: Some(30_000).into(),
                    exile_manpower: Some(12_400).into(),
                    exile_manpower_cap: Some(24_000).into(),
                    exile_leaders: leaders,
                    ..Default::default()
                },
                CountryStabilityInputRow {
                    country: 3,
                    exile_host: Some(0).into(),
                    ..Default::default()
                },
            ]),
        );
        set(
            &mut at,
            TableId::PowerBalances,
            TableRows::PowerBalances(vec![
                balance(0, "mod_second", 5_000),
                balance(1, "mod_first", -5_000),
            ]),
        );
        set(
            &mut at,
            TableId::PowerBalanceMembers,
            TableRows::PowerBalanceMembers(vec![member(1, 0, 3), member(0, 0, 2), member(0, 1, 3)]),
        );
        state.step(&at);
    };
    observe(&mut state, 100, Recorded::NotRecorded, 2);
    observe(&mut state, 101, Some(0).into(), 2);
    // The government returns home: a nonpositive host is no exile.
    observe(&mut state, 102, Some(0).into(), 0);
    publish(&mut state, &fixture);
    let reply = ask(&fixture, window(100, 102), 102);
    assert_eq!(reply.exile_history.len(), 1);
    assert_eq!(reply.known, vec![span(100, 102)]);
    let exile = &reply.exile_history[0];
    assert_eq!(exile.tags[0].tag, "EXA");
    assert_eq!(exile.host, vec![run(100, 101, "2")]);
    assert_eq!(reply.holders["2"][0].tag, "HSA");
    assert_eq!(exile.legitimacy, vec![run(100, 101, "30000")]);
    assert_eq!(exile.manpower, vec![run(100, 101, "12400")]);
    // An unrecorded field leaves only its own value unknown; a known zero stays.
    assert_eq!(exile.leaders, vec![run(101, 101, "0")]);
    let reply = ask(&fixture, params(PoliticsHistoryMode::Balances, None), 102);
    let holder = |tag: &str| {
        reply
            .holder_balances
            .iter()
            .find(|holder| holder.tags[0].tag == tag)
            .unwrap()
    };
    // BPC belongs to both records; the first stored one is shown for it.
    let shared = holder("BPC");
    assert_eq!(
        shared
            .balances
            .iter()
            .map(|balance| balance.token.as_str())
            .collect::<Vec<_>>(),
        ["mod_first", "mod_second"]
    );
    assert_eq!(shared.shown, vec![run(100, 102, "mod_second")]);
    assert_eq!(
        holder("HSA").balances[0].values,
        vec![run(100, 102, "5000")]
    );
    assert!(!reply.holder_balances.iter().any(|holder| holder.tags[0].tag == "EXA"));
}

#[test]
fn an_unreadable_balance_table_leaves_values_unknown_not_ended_wrongly() {
    let fixture = Fixture::new();
    let mut state = Prepared::default();
    let mut at = frame(100);
    countries(&mut at, &[(1, 1, "BPA")], &[]);
    set(
        &mut at,
        TableId::PowerBalances,
        TableRows::PowerBalances(vec![balance(0, "mod_balance", 1)]),
    );
    set(
        &mut at,
        TableId::PowerBalanceMembers,
        TableRows::PowerBalanceMembers(vec![member(0, 0, 1)]),
    );
    state.step(&at);
    let mut next = at.clone();
    next.hour = 101;
    *next.tables[TableId::PowerBalances.index()] = TableView::Unavailable {
        reason: "not recorded".into(),
    };
    state.step(&next);
    publish(&mut state, &fixture);
    let reply = ask(&fixture, params(PoliticsHistoryMode::Country, Some("BPA")), 101);
    assert_eq!(reply.holder_balances[0].balances[0].values, vec![run(100, 100, "1")]);
    // Hour 101 was not read: the run's end there is unknown, not an ending.
    assert_eq!(reply.known, vec![span(100, 100)]);
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("balance reads unavailable"))
    );
}

#[test]
fn equal_records_stay_apart_and_shared_ones_are_witnessed_at_their_read() {
    let fixture = Fixture::new();
    let mut state = Prepared::default();
    let mut at = frame(100);
    countries(&mut at, &[(1, 1, "BPA"), (2, 2, "BPB"), (3, 3, "BPC")], &[]);
    // Two records of one balance with every field equal, and a third member
    // sharing the first record.
    set(
        &mut at,
        TableId::PowerBalances,
        TableRows::PowerBalances(vec![balance(0, "mod_balance", 7), balance(1, "mod_balance", 7)]),
    );
    set(
        &mut at,
        TableId::PowerBalanceMembers,
        TableRows::PowerBalanceMembers(vec![member(0, 0, 1), member(0, 1, 3), member(1, 0, 2)]),
    );
    state.step(&at);
    publish(&mut state, &fixture);
    let reply = ask(&fixture, params(PoliticsHistoryMode::Balances, None), 100);
    let record = |tag: &str| {
        let holder = reply
            .holder_balances
            .iter()
            .find(|holder| holder.tags[0].tag == tag)
            .unwrap();
        holder.balances[0].record.clone()
    };
    assert_eq!(record("BPA"), vec![run(100, 100, "0")]);
    assert_eq!(record("BPC"), vec![run(100, 100, "0")]);
    assert_eq!(record("BPB"), vec![run(100, 100, "1")]);
}

// regression: d12ed98d96 Answer the politics review: read coverage, record identity and dated ordering
#[test]
fn an_exile_window_clips_carried_daily_reads_and_needs_bounds() {
    let fixture = Fixture::new();
    let mut state = Prepared::default();
    const EXILES: u32 = 8;
    const DAYS: i32 = 8;
    const FIRST: i32 = 96;
    // Each exile is read once a day, a quarter of them at each of four hours,
    // and every read changes every field: the densest history a day's reads make.
    let offset = |country: u32| (country % 4) as i32 * 6;
    let host = (EXILES + 1) as i32;
    let slots: Vec<(u32, u32, String)> = (1..=EXILES + 1)
        .map(|slot| (slot, slot, format!("X{slot:04}")))
        .collect();
    let tags: Vec<(u32, u32, &str)> =
        slots.iter().map(|(id, slot, tag)| (*id, *slot, tag.as_str())).collect();
    let mut base = frame(FIRST);
    countries(&mut base, &tags, &[]);
    let last = FIRST + DAYS * 24 - 1;
    for hour in FIRST..=last {
        // Each exile's latest read by this hour, and its day.
        let read = |country: u32| {
            let since = hour - FIRST - offset(country);
            let day = since.div_euclid(24);
            (FIRST + day * 24 + offset(country), day)
        };
        let mut at = base.clone();
        at.hour = hour;
        set(
            &mut at,
            TableId::CountryStabilityInputs,
            TableRows::CountryStabilityInputs(
                (1..=EXILES)
                    .map(|country| {
                        let (_, day) = read(country);
                        CountryStabilityInputRow {
                            country,
                            exile_host: Some(host).into(),
                            exile_legitimacy: Some(i64::from(day) * 100 + i64::from(country)).into(),
                            exile_manpower: Some(day).into(),
                            exile_manpower_cap: Some(day + 1).into(),
                            exile_leaders: Some(day).into(),
                            ..Default::default()
                        }
                    })
                    .collect(),
            ),
        );
        set(
            &mut at,
            TableId::SliceReads,
            TableRows::SliceReads(
                (1..=EXILES)
                    .map(|country| SliceReadRow {
                        table: TableId::CountryStabilityInputs.index() as u32,
                        owner: country,
                        sample_hour: read(country).0,
                    })
                    .collect(),
            ),
        );
        let changed = if (hour - FIRST) % 6 == 0 {
            Changed::fields(TableId::CountryStabilityInputs, &["exile_legitimacy", "exile_manpower"])
                .and(TableId::SliceReads, &["sample_hour"])
        } else {
            Changed::default()
        };
        state.observe_changes(&at, &changed).unwrap();
    }
    // Only the hours a read changed observed the exiles, every accepted hour between carried.
    assert_eq!(
        state
            .observed
            .iter()
            .filter(|(_, part)| *part == Part::Exiles)
            .count(),
        (DAYS * 4) as usize
    );
    publish(&mut state, &fixture);
    let (from, to) = (last - 7 * 24 + 1, last);
    let reply = ask(&fixture, window(from, to), last);
    assert_eq!(reply.exile_history.len(), EXILES as usize);
    assert_eq!(reply.known, vec![span(from, to)]);
    for (index, exile) in reply.exile_history.iter().enumerate() {
        let country = index as u32 + 1;
        // A run per daily read in the window, and the one carried into it,
        // covering every hour of the window without a gap.
        let reads = if offset(country) == 0 { 7 } else { 8 };
        for runs in [&exile.legitimacy, &exile.manpower, &exile.read] {
            assert_eq!(runs.len(), reads, "{country}");
            assert_eq!(runs[0].first_hour, from);
            assert_eq!(runs.last().unwrap().last_hour, to);
            assert!(runs.windows(2).all(|pair| pair[0].last_hour + 1 == pair[1].first_hour));
        }
    }
    assert!(dashboard_base::products::check_reply_budget(&reply).is_ok());
    // A history without its window is refused, not answered whole.
    assert!(
        query(
            &fixture.store,
            &fixture.key,
            &params(PoliticsHistoryMode::Exiles, None),
            Some(last),
            None,
            &AtomicBool::new(false),
        )
        .is_err()
    );
}

#[test]
fn unavailable_parts_count_their_catalogue_without_numeric_runs() {
    let fixture = Fixture::new();
    let mut state = Prepared::default();
    let mut expected = Metadata {
        tags: BTreeMap::from([(3, vec![(10, "AAA".to_owned())])]),
        ..Default::default()
    };
    for hour in [10, 11] {
        let mut at = frame(hour);
        countries(&mut at, &[(7, 3, "AAA")], &[]);
        for (table, scope) in [
            (TableId::Politics, "election schedules"),
            (TableId::CountryStabilityInputs, "exile status"),
            (TableId::PowerBalances, "balance reads"),
            (TableId::CountryCivilWars, "civil-war lists"),
        ] {
            *at.tables[table.index()] = TableView::Unavailable {
                reason: "fixture refusal".into(),
            };
            expected.refusals.insert(
                format!(
                    "{scope} unavailable: hour {hour} {table:?} is unavailable: fixture refusal"
                ),
                1,
            );
        }
        state.step(&at);
        assert_eq!(state.values.bytes(), 0, "no part supplied numeric values");
        assert_eq!(state.metadata, expected);
        let pending = state.bytes();
        let text = serde_json::to_vec(&(BTreeMap::<String, u64>::new(), &expected)).unwrap();
        assert_eq!(
            pending,
            text.len(),
            "exact text of all four named refusals and the holder"
        );
        let mut writer = fixture.writer();
        let before = writer.payload_bytes_change();
        state
            .flush(&mut writer, 1, &AtomicBool::new(false))
            .unwrap();
        assert!(writer.payload_bytes_change() - before <= 8 * pending as i128);
        assert_eq!(
            state.bytes(),
            0,
            "publication releases the catalogue charge"
        );
        writer.commit().unwrap();
        let (_, metadata) =
            series::read_catalogue::<Metadata>(&fixture.store, &fixture.key, FAMILY, 1)
                .unwrap()
                .unwrap();
        assert_eq!(
            metadata, expected,
            "accounting preserves every refusal and its order"
        );
    }
}

#[test]
fn an_exile_or_schedule_naming_a_country_with_no_identity_leaves_its_hour_unknown() {
    let fixture = Fixture::new();
    let mut state = Prepared::default();
    // EXA (1) is exiled to HSA (2); raw 9 is a country no link names.
    let observe = |state: &mut Prepared, hour: i32, exiles: Vec<(u32, Recorded<Option<i32>>)>, schedules: &[u32]| {
        let mut at = frame(hour);
        countries(&mut at, &[(1, 1, "EXA"), (2, 2, "HSA")], &[]);
        owned_states(&mut at, &[(1, 3), (2, 3)]);
        set(
            &mut at,
            TableId::CountryStabilityInputs,
            TableRows::CountryStabilityInputs(
                exiles
                    .into_iter()
                    .map(|(country, exile_host)| CountryStabilityInputRow { country, exile_host, ..Default::default() })
                    .collect(),
            ),
        );
        politics(
            &mut at,
            schedules
                .iter()
                .map(|&country| PoliticsRow {
                    country,
                    last_election: engine_native::cdate::HOUR_EPOCH + 1936 * 365 * 24,
                    has_elections: true.into(),
                    election_interval: 0.into(),
                    ..Default::default()
                })
                .collect(),
        );
        state.step(&at);
    };
    observe(&mut state, 100, vec![(1, Some(2).into())], &[1]);
    // The host can't be named.
    observe(&mut state, 101, vec![(1, Some(9).into())], &[1, 9]);
    // The government in exile can't be named.
    observe(&mut state, 102, vec![(1, Some(2).into()), (9, Some(2).into())], &[1]);
    // The host was not recorded.
    observe(&mut state, 103, vec![(1, Recorded::NotRecorded)], &[1]);
    observe(&mut state, 104, vec![(1, Some(2).into())], &[1]);
    publish(&mut state, &fixture);
    let exiles = ask(&fixture, window(100, 104), 104);
    assert_eq!(exiles.known, vec![span(100, 100), span(104, 104)]);
    for reason in ["an exile's host has no identity", "a government in exile has no identity", "an exile host is not recorded"] {
        assert!(exiles.warnings.iter().any(|warning| warning.contains(reason)), "{reason}");
    }
    let elections = ask(&fixture, params(PoliticsHistoryMode::Elections, None), 104);
    assert_eq!(elections.known, vec![span(100, 100), span(102, 104)]);
    assert!(elections.warnings.iter().any(|warning| warning.contains("election schedule's country has no identity")));
}
