use super::*;

fn ger(raw: u32) -> Option<Named> {
    match raw {
        1 | 7 => Some(("GER".into(), "1:1".into())),
        2 => Some(("SOV".into(), "2:2".into())),
        _ => None,
    }
}

fn params(family: MilitaryAirFamily) -> MilitaryAirParams {
    MilitaryAirParams { branch_id: "1".into(), family, regions: Vec::new(), causes: Vec::new(), from_hour: None, window: 0, top: None }
}

fn reply_of(fold: &Fold, params: &MilitaryAirParams) -> MilitaryAirReply {
    reply(fold, (10, 13), &countries(), params, &Openings::new(), Vec::new(), &AtomicBool::new(false)).unwrap()
}

fn air(id: u64, game_hour: i32, country: u32, credited_enemy: bool, amount: i64, opponent: Option<u32>) -> Event {
    Event::Air { id, game_hour, country, region: 5, cause: 1, credited_enemy, amount, opponent }
}

fn hour(hour: i32, events: Option<Vec<Event>>) -> (Span, Vec<(i32, Vec<Event>)>) {
    match events {
        None => (Span { first: hour, last: hour, held: Held::Unread }, Vec::new()),
        Some(events) => (Span { first: hour, last: hour, held: Held::Observed }, vec![(hour, events)]),
    }
}

fn countries() -> BTreeMap<Named, Vec<HourSpan>> {
    [(("GER".to_owned(), "1:1".to_owned()), vec![HourSpan { first_hour: 10, last_hour: 20 }]), (("SOV".to_owned(), "2:2".to_owned()), vec![HourSpan { first_hour: 10, last_hour: 20 }])].into()
}

fn fold(family: MilitaryAirFamily, hours: &[(Span, Vec<(i32, Vec<Event>)>)]) -> io::Result<Fold> {
    let mut fold = Fold::default();
    for (span, reads) in hours {
        fold.span(family, *span, reads, |raw, _| ger(raw), |token| (token == 9).then(|| "air_base".to_owned()))?;
    }
    Ok(fold)
}

#[test]
fn air_losses_are_named_at_the_read_and_a_range_starts_at_the_events_own_hour() {
    let hours = [hour(10, None), hour(11, Some(vec![air(1, 11, 7, false, 300_000, Some(2))])), hour(12, Some(vec![])), hour(13, Some(vec![air(2, 9, 1, false, 100_000, None), air(3, 13, 3, true, 50_000, None)]))];
    let fold = fold(MilitaryAirFamily::AirLosses, &hours).unwrap();
    assert_eq!(fold.observed, vec![HourSpan { first_hour: 11, last_hour: 13 }], "an unread hour certifies nothing, an empty read certifies none");
    let reply = reply_of(&fold, &params(MilitaryAirFamily::AirLosses));
    let ger: Vec<_> = reply.parts.iter().filter(|part| part.tag.as_deref() == Some("GER")).collect();
    assert_eq!(ger.len(), 2);
    let known = ger.iter().find(|part| part.opponent_known == Some(true)).unwrap();
    assert_eq!(known.points, vec![(11, 11, "300000".to_owned())]);
    let unknown = ger.iter().find(|part| part.opponent_known == Some(false)).unwrap();
    assert_eq!(unknown.points, vec![(9, 13, "100000".to_owned())], "a late-read event keeps its own game hour");
    let unnamed = reply.parts.iter().find(|part| part.tag.is_none()).unwrap();
    assert!(unnamed.dealt);
    assert!(reply.notes.contains(&MilitaryAirNote { reason: "air event endpoint is not resolved by its recorded country links".into(), events: 1 }));
    let ranged = MilitaryAirParams { from_hour: Some(10), ..params(MilitaryAirFamily::AirLosses) };
    let reply = reply_of(&fold, &ranged);
    let unknown = reply.parts.iter().find(|part| part.tag.as_deref() == Some("GER") && part.opponent_known == Some(false)).unwrap();
    assert!(unknown.points.is_empty(), "an event read inside the range but dated before it is outside it");
}

#[test]
fn a_read_in_part_counts_its_events_but_certifies_no_complete_total() {
    let in_part = |hour, events| (Span { first: hour, last: hour, held: Held::InPart }, vec![(hour, events)]);
    let hours = [hour(11, Some(vec![air(1, 11, 7, false, 300_000, None)])), in_part(12, vec![]), hour(13, Some(vec![]))];
    let fold = fold(MilitaryAirFamily::AirLosses, &hours).unwrap();
    assert_eq!(fold.observed, vec![HourSpan { first_hour: 11, last_hour: 13 }]);
    assert_eq!(fold.incomplete, vec![HourSpan { first_hour: 12, last_hour: 12 }], "an empty read in part is no evidence of zero losses");
}

#[test]
fn bombing_counts_only_air_callers_once_on_each_side() {
    let bombing = |id, cause| Event::Bombing { id, game_hour: 11, attacker: Some(1), victim: 2, state: 0, building: 9, cause, amount: 2_500_000 };
    let hours = [hour(11, Some(vec![bombing(1, BuildingBombingDamageCause::AirMission), bombing(2, BuildingBombingDamageCause::LandCombat), bombing(3, BuildingBombingDamageCause::Unknown)]))];
    let fold = fold(MilitaryAirFamily::Bombing, &hours).unwrap();
    assert_eq!(fold.incomplete, vec![HourSpan { first_hour: 11, last_hour: 11 }]);
    let reply = reply_of(&fold, &params(MilitaryAirFamily::Bombing));
    assert_eq!(reply.parts.len(), 2, "a land-combat caller is not aircraft bombing; an unknown one is not counted");
    for part in &reply.parts {
        assert_eq!((part.tag.as_deref(), part.building_token.as_deref(), part.state), (Some(if part.dealt { "GER" } else { "SOV" }), Some("air_base"), Some(0)));
        assert_eq!(part.points, vec![(11, 11, "2500000".to_owned())]);
    }
    assert!(reply.notes.iter().any(|note| note.events == 1 && note.reason.starts_with("building damage has an unsupported caller")));
}

#[test]
fn a_restart_after_an_unread_interval_is_new_events_only_when_dated_within_it() {
    let restarted = [hour(11, Some(vec![air(5, 11, 1, false, 1, None)])), hour(12, None), hour(14, Some(vec![air(1, 13, 1, false, 1, None)]))];
    assert!(fold(MilitaryAirFamily::AirLosses, &restarted).is_ok());
    let again = [hour(11, Some(vec![air(5, 11, 1, false, 1, None)])), hour(12, None), hour(14, Some(vec![air(1, 11, 1, false, 1, None)]))];
    assert!(fold(MilitaryAirFamily::AirLosses, &again).err().expect("a restart dated before the gap is refused").to_string().contains("earlier events recorded again"));
    let backwards = [hour(11, Some(vec![air(5, 11, 1, false, 1, None)])), hour(12, Some(vec![air(5, 12, 1, false, 1, None)]))];
    assert!(fold(MilitaryAirFamily::AirLosses, &backwards).is_err());
}

#[test]
fn observed_hours_are_those_both_tables_were_available() {
    let run = |first, last, status| StatusRun { first, last, status };
    let events = [run(10, 19, Status::Complete), run(20, 24, Status::Unavailable { reason: 1 }), run(25, 40, Status::Truncated { missing: 2 })];
    let links = [run(10, 14, Status::Complete), run(15, 16, Status::Unavailable { reason: 3 }), run(17, 18, Status::Complete), run(19, 22, Status::Truncated { missing: 1 }), run(30, 40, Status::Complete)];
    let span = |first, last, held| Span { first, last, held };
    assert_eq!(
        spans(8, 42, &events, &links),
        vec![
            span(8, 9, Held::Unread),
            span(10, 14, Held::Observed),
            span(15, 16, Held::Unjoined),
            span(17, 18, Held::Observed),
            span(19, 19, Held::InPart),
            span(20, 24, Held::Unread),
            span(25, 29, Held::Unjoined),
            span(30, 40, Held::InPart),
            span(41, 42, Held::Unread),
        ]
    );
}

#[test]
fn a_top_ranks_only_holders_above_zero_by_their_totals_since_the_first_hour() {
    let hours = [hour(11, Some(vec![air(1, 11, 1, false, 300_000, None)])), hour(12, Some(vec![])), hour(13, Some(vec![air(2, 13, 2, false, 100_000, None)]))];
    let fold = fold(MilitaryAirFamily::AirLosses, &hours).unwrap();
    let membership = |reply: &MilitaryAirReply, tag: &str| reply.countries.iter().find(|country| country.tag == tag).and_then(|country| country.membership.clone());
    let opened: Openings = [((Some("2:2".to_owned()), false, 1, Some(false)), 400_000)].into();
    for metric in [MilitaryAirMetric::Total, MilitaryAirMetric::Taken] {
        let ranked = MilitaryAirParams { top: Some(MilitaryAirTop { count: 1, metric }), ..params(MilitaryAirFamily::AirLosses) };
        let alone = reply_of(&fold, &ranked);
        assert_eq!(membership(&alone, "GER"), Some(vec![HourSpan { first_hour: 11, last_hour: 20 }]));
        assert_eq!(membership(&alone, "SOV"), Some(Vec::new()), "a holder below the top one is not ranked");
        let carried = reply(&fold, (10, 13), &countries(), &ranked, &opened, Vec::new(), &AtomicBool::new(false)).unwrap();
        assert_eq!(membership(&carried, "SOV"), Some(vec![HourSpan { first_hour: 11, last_hour: 20 }]), "what it carried in ranks it first");
        assert_eq!(membership(&carried, "GER"), Some(Vec::new()));
        assert_eq!(carried.openings, vec![MilitaryAirOpening { identity: Some("2:2".into()), dealt: false, cause: 1, opponent_known: Some(false), total: "400000".into() }]);
    }
    let dealt = MilitaryAirParams { top: Some(MilitaryAirTop { count: 1, metric: MilitaryAirMetric::Dealt }), ..params(MilitaryAirFamily::AirLosses) };
    let carried = reply(&fold, (10, 13), &countries(), &dealt, &opened, Vec::new(), &AtomicBool::new(false)).unwrap();
    assert!(carried.countries.iter().all(|country| country.membership == Some(Vec::new())), "losses carried in are not losses dealt; no holder is ranked at zero");
}

mod windows {
    use super::super::{reference, *};
    use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
    use replay::campaign::Replay;
    use replay_data::rows::{AirLossEventRow, BuildingBombingDamageRow, CountryLinkRow, CountryRow, LexicalTokenRow, TableRows, TableSchema, tables};
    use replay_frame::frame::{DecodedFrame, TableView};
    use std::sync::Arc;

    const FIRST: i32 = 1_000;
    const LAST: i32 = 1_420;
    const UNREAD: std::ops::RangeInclusive<i32> = 1_200..=1_209;

    fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
        *frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
        frame.metrics.table_ns[T::ID.index()] = 1;
    }

    fn loss(id: u64, game_hour: i32, country: u32, (region, native_cause): (u32, u32), credited_enemy: bool, amount_raw: i64, opponent: Option<u32>) -> AirLossEventRow {
        AirLossEventRow { id, game_hour, country, region, native_cause, family_base: 0, credited_enemy, amount_raw, opponent, daily_slot: 0 }
    }

    fn bombing(id: u64, game_hour: i32, attacker: Option<u32>, state: u32, building_type: u32, cause: BuildingBombingDamageCause, amount_raw: i64) -> BuildingBombingDamageRow {
        BuildingBombingDamageRow { id, game_hour, attacker, victim: 9002, state, province: None, building_type, native_cause: 0, cause, amount_raw, credited_raw: amount_raw }
    }

    fn recording() -> Source {
        let frames = (FIRST..=LAST)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                let TableView::Complete(TableRows::Countries(countries)) = &*frame.tables[TableId::Countries.index()] else { unreachable!("the fixture holds its countries") };
                let mut countries = countries.clone();
                countries.push(CountryRow { id: 9002, canonical_slot: 8, tag: b"ITA".to_vec(), ..countries[0].clone() });
                set::<tables::Countries>(&mut frame, countries);
                set::<tables::CountryLinks>(&mut frame, vec![
                    CountryLinkRow { id: 11, canonical_slot: 7 },
                    CountryLinkRow { id: 9001, canonical_slot: 7 },
                    CountryLinkRow { id: 9002, canonical_slot: 8 },
                ]);
                set::<tables::LexicalTokens>(&mut frame, vec![LexicalTokenRow { token: 9, literal: b"air_base".to_vec() }]);
                let id = |at: u64| u64::try_from(hour).unwrap() * 16 + at;
                if UNREAD.contains(&hour) {
                    *frame.tables[TableId::AirLossEvents.index()] = TableView::Unavailable { reason: "not read".into() };
                } else {
                    let mut losses = Vec::new();
                    if hour % 5 == 0 {
                        let region = 5 + u32::try_from(hour / 5 % 2).unwrap();
                        losses.push(loss(id(0), hour, if hour % 10 == 0 { 9001 } else { 11 }, (region, 1), false, 100_000, Some(9002)));
                        losses.push(loss(id(1), hour, 9002, (region, 1), true, 100_000, Some(9001)));
                    }
                    if hour % 7 == 0 {
                        losses.push(loss(id(2), hour - 30, 9002, (6, 3), false, 40_000, None));
                    }
                    if hour % 11 == 0 {
                        losses.push(loss(id(3), hour, 55, (5, 2), false, 70_000, Some(9001)));
                    }
                    if hour % 35 == 0 {
                        losses.push(loss(id(4), hour, 9001, (5, 1), false, -50_000, Some(9002)));
                    }
                    if hour % 50 == 0 {
                        losses.push(loss(id(5), hour, 9002, (5, 9), false, 10_000, None));
                    }
                    set::<tables::AirLossEvents>(&mut frame, losses);
                }
                let mut damage = Vec::new();
                if hour % 6 == 0 {
                    damage.push(bombing(id(8), hour - 2, Some(9001), 3 + u32::try_from(hour / 6 % 2).unwrap(), 9, BuildingBombingDamageCause::AirMission, 2_500_000));
                    damage.push(bombing(id(9), hour, None, 3, 10, BuildingBombingDamageCause::AirNaval, 1_000_000));
                }
                if hour % 12 == 0 {
                    damage.push(bombing(id(10), hour, Some(9001), 3, 9, BuildingBombingDamageCause::LandCombat, 5_000_000));
                }
                if hour % 40 == 0 {
                    damage.push(bombing(id(11), hour, Some(9001), 4, 9, BuildingBombingDamageCause::Unknown, 3_000_000));
                }
                set::<tables::BuildingBombingDamage>(&mut frame, damage);
                frame
            })
            .collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        Source::from_replay(dashboard_base::SourceKey { source_id: "air-windows".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
    }

    fn asked(family: MilitaryAirFamily, regions: &[u32], causes: &[u32], from_hour: Option<i32>, top: Option<(usize, MilitaryAirMetric)>) -> MilitaryAirParams {
        let top = top.map(|(count, metric)| MilitaryAirTop { count, metric });
        MilitaryAirParams { branch_id: "1".into(), family, regions: regions.to_vec(), causes: causes.to_vec(), from_hour, window: 0, top }
    }

    fn cases() -> Vec<MilitaryAirParams> {
        vec![
            asked(MilitaryAirFamily::AirLosses, &[], &[], None, None),
            asked(MilitaryAirFamily::AirLosses, &[], &[], None, Some((1, MilitaryAirMetric::Taken))),
            asked(MilitaryAirFamily::AirLosses, &[5], &[], Some(1_103), Some((1, MilitaryAirMetric::Dealt))),
            asked(MilitaryAirFamily::AirLosses, &[], &[1, 3], None, Some((2, MilitaryAirMetric::Total))),
            asked(MilitaryAirFamily::AirLosses, &[6], &[3], Some(1_103), None),
            asked(MilitaryAirFamily::Bombing, &[], &[], None, Some((1, MilitaryAirMetric::Total))),
            asked(MilitaryAirFamily::Bombing, &[], &[], Some(1_250), None),
        ]
    }

    fn windows_of(source: &Source, query: &Query<'_>, params: &MilitaryAirParams) -> Vec<MilitaryAirReply> {
        let (context, running) = (LoadContext { content: None }, AtomicBool::new(false));
        let mut windows = Vec::new();
        let mut hour = params.from_hour.map_or(query.timeline.first, |from| query.timeline.first + (from - query.timeline.first) / 70 * 70);
        while hour <= query.timeline.last {
            let last = (hour + 69).min(query.timeline.last);
            windows.push(window_reply(source, query, params, (hour, last), &context, &running).unwrap());
            hour = last + 1;
        }
        windows
    }

    #[test]
    fn windows_joined_equal_the_whole_branch_reply() {
        let source = recording();
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let (context, running) = (LoadContext { content: None }, AtomicBool::new(false));
        for params in cases() {
            let windows = windows_of(&source, &query, &params);
            let whole = reference::whole(&source, &query, &params, windows[0].first_hour, &context, &running).unwrap();
            assert!(whole.parts.iter().any(|part| !part.points.is_empty()), "{params:?}: the case counts something");
            assert!(params.top.is_none() || whole.countries.iter().any(|country| country.membership.as_ref().is_some_and(|spans| !spans.is_empty())), "{params:?}: the case ranks someone");
            let from = params.from_hour.or(whole.observed.first().map(|span| span.first_hour));
            assert!(windows.iter().all(|window| window.counted_from == from), "{params:?}: every window counts from the whole's first hour");
            let joined = reference::join(&windows).unwrap();
            assert_eq!(serde_json::to_value(&joined).unwrap(), serde_json::to_value(&whole).unwrap(), "{params:?}: the windows joined are the whole reply");
        }
    }

    #[test]
    fn a_windows_openings_are_the_totals_of_the_windows_before_it() {
        let source = recording();
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let running = AtomicBool::new(false);
        for params in cases() {
            let windows = windows_of(&source, &query, &params);
            for (at, window) in windows.iter().enumerate() {
                let opened: Openings = window
                    .openings
                    .iter()
                    .map(|opening| ((opening.identity.clone(), opening.dealt, opening.cause, opening.opponent_known), opening.total.parse::<i128>().unwrap()))
                    .collect();
                assert_eq!(opened, reference::carried(&windows[..at]).unwrap(), "{params:?}: window from {} opens with the windows before it", window.first_hour);
                assert_eq!(opened, openings(&source, &query, &params, window.first_hour, false, &running).unwrap(), "{params:?}: the tables agree with the events from {}", window.first_hour);
            }
            assert!(windows.last().is_some_and(|window| !window.openings.is_empty()), "{params:?}: the case carries something in");
        }
    }

    #[test]
    fn a_windows_hours_reach_back_to_where_they_began_since_the_first_hour_counted() {
        let source = recording();
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let (context, running) = (LoadContext { content: None }, AtomicBool::new(false));
        for params in cases() {
            let whole = reference::whole(&source, &query, &params, FIRST, &context, &running).unwrap();
            let counted = params.from_hour.unwrap_or(FIRST);
            let windows = windows_of(&source, &query, &params);
            assert!(windows.iter().any(|window| window.observed.first().is_some_and(|span| span.first_hour < window.first_hour)), "{params:?}: some window reaches back");
            for window in &windows {
                let began = whole.observed.iter().find(|span| span.first_hour <= window.first_hour && window.first_hour <= span.last_hour).map_or(window.first_hour, |span| span.first_hour.max(counted).min(window.first_hour));
                let cut = |spans: &[HourSpan]| -> Vec<HourSpan> {
                    spans
                        .iter()
                        .filter_map(|span| {
                            let (first_hour, last_hour) = (span.first_hour.max(began), span.last_hour.min(window.last_hour));
                            (first_hour <= last_hour).then_some(HourSpan { first_hour, last_hour })
                        })
                        .collect()
                };
                assert_eq!(window.observed, cut(&whole.observed), "{params:?}: window from {}", window.first_hour);
                let listed: Vec<_> = window.countries.iter().map(|country| (country.tag.clone(), country.identity.clone(), country.present.clone())).collect();
                let expected: Vec<_> = whole.countries.iter().map(|country| (country.tag.clone(), country.identity.clone(), cut(&country.present))).filter(|(_, _, present)| !present.is_empty()).collect();
                assert_eq!(listed, expected, "{params:?}: holders of the window from {}", window.first_hour);
                let reads: Vec<i32> = whole.incomplete.iter().flat_map(|span| span.first_hour..=span.last_hour).collect();
                let mut hours: Vec<i32> = reads.iter().copied().filter(|hour| (window.first_hour..=window.last_hour).contains(hour)).collect();
                if let Some(&latest) = reads.iter().filter(|hour| (began..window.first_hour).contains(*hour)).max() {
                    hours.insert(0, latest);
                }
                let mut incomplete: Vec<HourSpan> = Vec::new();
                for hour in hours {
                    match incomplete.last_mut() {
                        Some(last) if last.last_hour + 1 == hour => last.last_hour = hour,
                        _ => incomplete.push(HourSpan { first_hour: hour, last_hour: hour }),
                    }
                }
                assert_eq!(window.incomplete, incomplete, "{params:?}: unknown callers of the window from {}", window.first_hour);
            }
        }
        let bombing = windows_of(&source, &query, &asked(MilitaryAirFamily::Bombing, &[], &[], None, None));
        assert!(bombing.iter().any(|window| window.incomplete.first().is_some_and(|span| span.first_hour < window.first_hour)), "a read before a window holding an unknown caller is carried");
    }

    #[test]
    fn the_observed_stretches_kept_at_open_answer_every_window_as_reading_them_does() {
        let (warmed, cold) = (recording(), recording());
        assert!(kept_observed(&warmed).is_empty(), "nothing is kept before the open reads ahead");
        warm_observed(&warmed, 1).unwrap();
        let query = dashboard_base::queries::query(&warmed, 1).unwrap();
        let run = query.timeline.stretches[0].run;
        let mut expected = vec![(run, MilitaryAirFamily::AirLosses.log(), vec![(FIRST, *UNREAD.start() - 1), (*UNREAD.end() + 1, LAST)]), (run, MilitaryAirFamily::Bombing.log(), vec![(FIRST, LAST)])];
        expected.sort_unstable();
        assert_eq!(kept_observed(&warmed), expected, "each family's hours with both tables available, over the whole run");
        let cold_query = dashboard_base::queries::query(&cold, 1).unwrap();
        for params in cases() {
            let json = |windows: Vec<MilitaryAirReply>| serde_json::to_value(windows).unwrap();
            assert_eq!(json(windows_of(&warmed, &query, &params)), json(windows_of(&cold, &cold_query, &params)), "{params:?}");
        }
        dashboard_base::queries::forget(&warmed);
        crate::forget(&warmed);
        assert!(kept_observed(&warmed).is_empty(), "forgotten with the recording's index");
    }

    #[test]
    fn a_window_reaches_back_across_a_read_in_part_and_names_it_incomplete() {
        const IN_PART: i32 = FIRST + 2;
        const AT: i32 = FIRST + 10;
        let frames = (FIRST..=AT)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                let losses = vec![loss(u64::try_from(hour).unwrap(), hour, 9001, (5, 1), false, 1_000, None)];
                if hour == IN_PART {
                    *frame.tables[TableId::AirLossEvents.index()] = TableView::Truncated { rows: tables::AirLossEvents::wrap(losses), missing: 1 };
                    frame.metrics.table_ns[TableId::AirLossEvents.index()] = 1;
                } else {
                    set::<tables::AirLossEvents>(&mut frame, losses);
                }
                frame
            })
            .collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        let source = Source::from_replay(dashboard_base::SourceKey { source_id: "air-in-part".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let (context, running) = (LoadContext { content: None }, AtomicBool::new(false));
        let reply = window_reply(&source, &query, &asked(MilitaryAirFamily::AirLosses, &[], &[], Some(FIRST), None), (AT, AT), &context, &running).unwrap();
        assert_eq!(reply.observed, vec![HourSpan { first_hour: FIRST, last_hour: AT }], "the reads before and after the read in part are one stretch");
        assert_eq!(reply.incomplete, vec![HourSpan { first_hour: IN_PART, last_hour: IN_PART }], "the read in part it reaches back over is named");
    }

    #[test]
    fn a_window_counts_from_its_own_first_hour() {
        let source = recording();
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let (context, running) = (LoadContext { content: None }, AtomicBool::new(false));
        let params = MilitaryAirParams { window: 7, ..asked(MilitaryAirFamily::AirLosses, &[], &[3], None, None) };
        let reply = window_reply(&source, &query, &params, (1_070, 1_139), &context, &running).unwrap();
        assert_eq!((reply.window, reply.first_hour, reply.last_hour), (7, 1_070, 1_139));
        assert_eq!(reply.observed, vec![HourSpan { first_hour: FIRST, last_hour: 1_139 }]);
        let late = reply.parts.iter().find(|part| part.tag.as_deref() == Some("ITA")).unwrap();
        assert_eq!(late.points.first(), Some(&(1_041, 1_071, "40000".to_owned())), "read at 1,071, it happened at 1,041 and counts from the window's start");
        assert_eq!(late.points.len(), 10);
    }
}
