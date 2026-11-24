use super::{
    EventFeedKind, Names,
    cards::{ScriptCurrency, edge},
    cause, derived, families,
    row::{Columns, FeedEvent, Party, Slot},
    text::Notices,
};
use dashboard_query::log::{Cell, Row};
use replay_data::{
    event_feed_rows::{EventFeedRoute, EventFeedScope, EventFeedSource, SCRIPT_FAMILY_NAMES},
    rows::{FeedEventRow, TableId, TagHolderRow},
    value::Value,
};
use std::collections::HashMap;

fn column(name: &str) -> usize {
    dashboard_declare::field(TableId::FeedEvents, name).unwrap()
}

fn row(generation: u64, sequence: u64, set: &[(&str, Cell)]) -> Row {
    let mut cells = vec![Cell::Unrecorded; TableId::FeedEvents.field_count()];
    for (name, cell) in set {
        cells[column(name)] = cell.clone();
    }
    Row { hour: 0, key: u128::from(generation) << 64 | u128::from(sequence), facet: 0, cells }
}

fn number(value: i64) -> Cell {
    Cell::Number(value)
}

fn party(reference: u32, slot: Slot) -> Party {
    Party { reference, id: Some(reference), slot }
}

fn event(code: u8, hour: i32, actor: Slot, target: Slot) -> FeedEvent {
    FeedEvent {
        generation: 1,
        sequence: 7,
        hour,
        read_hour: hour,
        code,
        actor: party(11, actor),
        target: party(12, target),
        cause: None,
        token: None,
        native_event: None,
        instance: None,
        option: None,
        country_target: None,
        numeric_target: None,
        lifecycle: None,
        days: None,
        scopes: None,
        opener: None,
        dated: None,
        costs: None,
        coverage: None,
        outstanding: None,
        refusals: None,
        failure: Vec::new(),
        incomplete: None,
        window: None,
        presentation: None,
    }
}

#[test]
fn a_feed_rows_cells_read_back_typed_and_what_was_not_recorded_stays_so() {
    let scope = EventFeedScope { country: 31, random: [5, 6], bindings: [0, 1, 0], state: 4, country_attempted: true, country_id: Some(310), ..EventFeedScope::default() };
    let mut scopes = Vec::new();
    vec![scope].encode(&mut scopes).unwrap();
    let columns = Columns::new().unwrap();
    let held = row(
        1,
        42,
        &[
            ("hour", number(100)),
            ("read_hour", number(101)),
            ("kind", number(i64::from(EventFeedSource::Join as u8))),
            ("actor_reference", number(11)),
            ("actor_id", number(110)),
            ("actor_slot", number(7)),
            ("target_reference", number(0)),
            ("target_slot", number(i64::from(TagHolderRow::NONE))),
            ("route", number(i64::from(EventFeedRoute::CallAlly as u8))),
            ("within", number(0)),
            ("conference", Cell::Missing),
            ("state", Cell::Missing),
            ("ally_reference", number(13)),
            ("ally_id", Cell::Missing),
            ("ally_instance", Cell::Missing),
            ("ally_slot", Cell::Unrecorded),
            ("reason", number(4)),
            ("flags", number(1)),
            ("token", Cell::Text("news.1".into())),
            ("native_event_kind", number(50)),
            ("native_event_id", number(9)),
            ("instance", Cell::Missing),
            ("option", number(-1)),
            ("country_target", Cell::Missing),
            ("numeric_target", Cell::Missing),
            ("lifecycle", Cell::Missing),
            ("days", Cell::Missing),
            ("scopes", Cell::List(scopes)),
            ("opener", number(40)),
            ("opener_hour", number(99)),
            ("dated", Cell::Missing),
            ("political_power", number(-5_000_000)),
            ("command_power", Cell::Missing),
            ("coverage", Cell::Missing),
            ("outstanding", Cell::Missing),
            ("refusals", Cell::Missing),
            ("failure", Cell::Text(String::new())),
            ("backlog", number(0)),
        ],
    );
    let read = columns.read(&held).unwrap();
    assert_eq!((read.generation, read.sequence, read.hour, read.read_hour), (1, 42, 100, 101));
    assert_eq!(read.actor, Party { reference: 11, id: Some(110), slot: Slot::Of(7) });
    assert_eq!(read.target.slot, Slot::None, "reference zero names no country");
    let cause = read.cause.unwrap();
    assert_eq!((cause.route, cause.reason, cause.flags, cause.conference), (EventFeedRoute::CallAlly as u8, Some(4), 1, None));
    assert_eq!(cause.ally.map(|ally| (ally.reference, ally.slot)), Some((13, Slot::NotRecorded)), "an ally no link witnessed is not recorded, not none");
    assert_eq!(read.token.as_deref(), Some(&b"news.1"[..]));
    assert_eq!((read.native_event, read.option, read.instance), (Some((50, 9)), Some(-1), None));
    assert_eq!(read.scopes, Some(vec![scope]));
    assert_eq!(read.opener, Some(40));
    assert_eq!(read.costs, Some([Some(-5_000_000), None]));
    assert_eq!(read.id(), "native:1:42");

    let bare = row(FeedEventRow::DERIVED, 5 << 32 | 3, &[("hour", number(5)), ("read_hour", number(5)), ("kind", number(i64::from(FeedEventRow::CEASED_TO_EXIST))), ("actor_reference", number(11)), ("target_reference", number(0))]);
    let read = columns.read(&bare).unwrap();
    assert_eq!((&read.cause, &read.scopes, &read.costs, &read.token), (&None, &None, &None, &None));
    assert_eq!(read.actor.slot, Slot::NotRecorded);
    assert_eq!(read.id(), "derived:5:3");
    assert!(columns.read(&row(1, 1, &[("hour", number(5))])).is_err());
}

#[test]
fn a_cause_reads_in_words_with_its_flags_and_a_scripts_operands_follow_it() {
    let mut join = event(EventFeedSource::Join as u8, 10, Slot::Of(7), Slot::Of(8));
    join.cause = Some(super::row::Cause { route: EventFeedRoute::CallAlly as u8, within: EventFeedRoute::OnAction as u8, conference: Some(77), state: None, ally: None, reason: Some(4), flags: 1 });
    let words = cause::words(&join, Some("ITA".into())).unwrap();
    assert_eq!((words.how.as_deref(), words.within.as_deref(), words.reason.as_deref()), (Some("a call to arms"), Some("an on_action"), Some("guarantee")));
    assert_eq!((words.conference.as_deref(), words.ally.as_deref()), (Some("77"), Some("ITA")));
    assert_eq!(words.notes, ["against every enemy of the ally's side"]);
    assert_eq!(cause::words(&event(EventFeedSource::Annex as u8, 10, Slot::Of(7), Slot::Of(8)), None), None, "no cause recorded, no words");

    let mut choice = event(EventFeedSource::EventOption as u8, 10, Slot::Of(7), Slot::Of(7));
    choice.native_event = Some((50, 9));
    choice.option = Some(0);
    choice.scopes = Some(vec![EventFeedScope { country: 3, references: [0, 0, 5, 0, 0, 0, 0, 0, 0], ..EventFeedScope::default() }]);
    let words = cause::words(&choice, None).unwrap();
    assert_eq!(words.how, None);
    assert_eq!(
        words.notes,
        [
            "Native event identity: 9 (type 50).",
            "Selected native option ordinal: 0.",
            "Scope 0: country reference 3; state 0; ROOT 0, FROM 0, PREV 0; random words 0, 0.",
            "Scope 0 ace reference: 5.",
        ]
    );
    let mut annex = event(EventFeedSource::Annex as u8, 10, Slot::Of(7), Slot::Of(8));
    annex.cause = Some(super::row::Cause { route: EventFeedRoute::AnnexEffect as u8, within: EventFeedRoute::CivilWarAnnex as u8, conference: None, state: None, ally: None, reason: None, flags: 0 });
    assert!(cause::civil_war(&annex), "an annexation inside a civil war's resolution");
}

#[test]
fn an_incomplete_read_is_one_notice_counted_with_examples_and_its_refused_families_each_one() {
    let mut notices = Notices::default();
    for hour in [100, 101, 102, 103] {
        let mut read = event(FeedEventRow::READ_INCOMPLETE, hour, Slot::None, Slot::None);
        read.coverage = Some(if hour == 100 { 1 } else { 0 });
        read.outstanding = Some(0);
        read.refusals = Some(if hour == 101 { 0b11 } else { 0 });
        read.failure = if hour == 103 { b"identity differs".to_vec() } else { Vec::new() };
        families::read_notices(&read, &[(90, 100)], &mut notices);
    }
    let mut complete = event(FeedEventRow::READ_INCOMPLETE, 104, Slot::None, Slot::None);
    complete.coverage = Some(2);
    complete.refusals = Some(1);
    families::read_notices(&complete, &[], &mut notices);
    let list = notices.into_list();
    let coverage = list.iter().find(|notice| notice.kind == "nativeEventCoverage").unwrap();
    assert_eq!(coverage.count, Some(4), "a complete read is no coverage notice");
    assert!(coverage.reason.starts_with("Partial read at 100: The recorder was detached"), "{}", coverage.reason);
    assert_eq!(coverage.reason.matches("; example: ").count(), 2, "three examples at most, the first the reason");
    let families: Vec<_> = list.iter().filter(|notice| notice.kind == "nativeScriptFamilyCoverage").map(|notice| (notice.label.as_str(), notice.count)).collect();
    assert_eq!(families, [(SCRIPT_FAMILY_NAMES[0].1, Some(2)), (SCRIPT_FAMILY_NAMES[1].1, Some(1))]);
}

#[test]
fn a_decisions_cost_is_listed_only_when_proven_and_only_the_currencies_it_charged() {
    let row = |event: &FeedEvent| super::EventFeedRow {
        id: event.id(),
        hour: event.hour,
        read_hour: event.read_hour,
        name_hour: event.hour,
        exact: true,
        category: super::EventFeedCategory::Politics,
        kind: EventFeedKind::DecisionSelected,
        tags: Vec::new(),
        participants: None,
        item: None,
        amount: None,
        cause: None,
        strike: None,
        decision_state: None,
        window: None,
    };
    let mut selected = event(EventFeedSource::DecisionSelected as u8, 10, Slot::Of(7), Slot::None);
    selected.costs = Some([Some(-5_000_000), Some(0)]);
    let costs = edge(&selected, row(&selected)).costs.unwrap();
    assert_eq!(costs.iter().map(|cost| (cost.currency, cost.amount.as_str())).collect::<Vec<_>>(), [(ScriptCurrency::PoliticalPower, "-5000000")]);
    selected.costs = Some([None, None]);
    assert!(edge(&selected, row(&selected)).costs.is_none(), "a kind that charges nothing proves nothing");
    selected.costs = None;
    assert!(edge(&selected, row(&selected)).costs.is_none(), "not proven is null, never zero");
}

#[test]
fn a_derived_row_is_dated_by_what_its_countries_held() {
    let names = Names { slots: HashMap::from([(7, vec![("GER".to_owned(), 0, 99), ("DDR".to_owned(), 100, 200)]), (8, vec![("POL".to_owned(), 0, 200)])]) };
    let mut notices = Notices::default();
    let mut ceased = event(FeedEventRow::CEASED_TO_EXIST, 150, Slot::Of(7), Slot::None);
    ceased.generation = FeedEventRow::DERIVED;
    ceased.dated = Some(90);
    let row = derived(&ceased, EventFeedKind::CeasedToExist, ceased.id(), &names, &mut notices).unwrap();
    assert_eq!((row.hour, row.name_hour, row.exact, row.tags.as_slice()), (150, 90, false, &["GER".to_owned()][..]), "named at its last read with states");
    let mut guarantee = event(FeedEventRow::GUARANTEE_GRANTED, 150, Slot::Of(8), Slot::Of(7));
    guarantee.generation = FeedEventRow::DERIVED;
    guarantee.dated = Some(120);
    let row = derived(&guarantee, EventFeedKind::GuaranteeGranted, guarantee.id(), &names, &mut notices).unwrap();
    assert_eq!((row.hour, row.exact, row.tags.as_slice()), (120, true, &["POL".to_owned(), "DDR".to_owned()][..]));
    guarantee.dated = Some(80);
    let row = derived(&guarantee, EventFeedKind::GuaranteeGranted, guarantee.id(), &names, &mut notices).unwrap();
    assert_eq!((row.hour, row.exact), (150, false), "a tag that changed since the start dates it at its read");
    guarantee.target.slot = Slot::NotRecorded;
    assert!(derived(&guarantee, EventFeedKind::GuaranteeGranted, guarantee.id(), &names, &mut notices).is_none());
    assert!(notices.into_list().iter().any(|notice| notice.kind == "guaranteeGranted"));
}

#[test]
fn a_logged_strike_reads_back_whole_and_a_refused_part_is_never_totalled() {
    use replay_data::{
        event_feed_rows::EventFeedCountry,
        rows::{NuclearBuildingDamage, NuclearEquipmentLoss, NuclearStrikePart, NuclearStrikeRefusal, NuclearStrikeRow, NuclearUnitDamage, NuclearWarSupport},
    };
    let strike = NuclearStrikeRow {
        id: 9,
        hour: 40,
        launcher: EventFeedCountry { reference: 3, id: 3, instance: 1 },
        nuke_type: 1,
        province: 13_000,
        state: 1_204,
        owner: EventFeedCountry { reference: 7, id: 7, instance: 1 },
        controller: 7,
        war_support: vec![NuclearWarSupport { country: 7, before_raw: 60_000, after_raw: 35_000 }],
        buildings: vec![NuclearBuildingDamage { amount_raw: 20_000_000, ..Default::default() }, NuclearBuildingDamage { amount_raw: 5_000_000, ..Default::default() }],
        units: vec![NuclearUnitDamage { manpower_before: 9_000, manpower_after: 6_500, ..Default::default() }],
        equipment: vec![NuclearEquipmentLoss { amount_raw: 1_200_000, ..Default::default() }],
        refusals: vec![NuclearStrikeRefusal { part: NuclearStrikePart::Equipment, reason: b"equipment destruction writers not forwarded".to_vec(), count: 1 }],
        wings: Vec::new(),
    };
    fn list<T: Value>(value: &T) -> Cell {
        let mut bytes = Vec::new();
        value.encode(&mut bytes).unwrap();
        Cell::List(bytes)
    }
    let names = ["hour", "launcher", "nuke_type", "province", "state", "owner", "controller", "war_support", "buildings", "units", "equipment", "refusals", "wings"];
    let cells = vec![
        Cell::Number(40),
        list(&strike.launcher),
        Cell::Number(1),
        Cell::Number(13_000),
        Cell::Number(1_204),
        list(&strike.owner),
        Cell::Number(7),
        list(&strike.war_support),
        list(&strike.buildings),
        list(&strike.units),
        list(&strike.equipment),
        list(&strike.refusals),
        list(&strike.wings),
    ];
    let columns = super::nuclear::Columns::of(&names);
    let logged = Row { hour: 41, key: 9, facet: 0, cells: cells.clone() };
    assert_eq!(columns.strike(&logged).unwrap(), strike);
    let mut unread = cells;
    unread[6] = Cell::Unrecorded;
    assert!(columns.strike(&Row { hour: 41, key: 9, facet: 0, cells: unread }).is_err(), "a cell not recorded refuses the strike");
    let row = super::nuclear::row(&strike, 41, [Some("USA".into()), Some("JAP".into())], None);
    assert_eq!((row.id.as_str(), row.hour, row.read_hour, row.kind), ("nuclear:9", 40, 41, EventFeedKind::NuclearStrike));
    let detail = row.strike.unwrap();
    assert_eq!((detail.state, detail.building_levels, detail.war_support, detail.units), (Some(1_204), Some(2.5), Some(-25.0), Some(1)));
    assert_eq!((detail.manpower_lost, detail.equipment_lost), (None, None));
    assert_eq!(detail.not_recorded, ["equipment: equipment destruction writers not forwarded (1×)"]);
    assert_eq!(super::nuclear::detail(&NuclearStrikeRow::default()).state, None, "a province without a state");
}

#[test]
fn scripted_text_not_expanded_is_marked_where_it_stood_never_as_an_authored_ellipsis() {
    use super::text::{SCRIPTED_GAP, elide_scripted};
    assert_eq!(elide_scripted("Recruit in [FROM.GetName]"), format!("Recruit in {SCRIPTED_GAP}"));
    assert_eq!(
        elide_scripted("Seize the Gold Reserve of [FROM.GetName]\u{2026} [ROOT.GetAdjective] plan [open"),
        format!("Seize the Gold Reserve of {SCRIPTED_GAP}\u{2026} {SCRIPTED_GAP} plan [open")
    );
    assert_ne!(SCRIPTED_GAP, '\u{2026}');
}

#[test]
fn a_request_for_the_kinds_alone_gives_every_code_and_reads_nothing_else() {
    let reply = families::kinds_only("7", 10, 20);
    assert_eq!(reply.kinds.len(), super::CODES.len());
    for (kind, &(code, expected)) in reply.kinds.iter().zip(super::CODES.iter()) {
        assert_eq!((kind.code, kind.kind, kind.category), (code, expected, expected.category()));
    }
    assert!(reply.rows.is_empty() && reply.unavailable.is_empty() && reply.warnings.is_empty());
    assert_eq!((reply.branch_id.as_str(), reply.first_hour, reply.last_hour), ("7", 10, 20));
    let asked: families::EventFeedSpanParams = serde_json::from_value(serde_json::json!({ "branchId": "7", "categories": [], "kindsOnly": true })).unwrap();
    assert_eq!(asked.kinds_only, Some(true));
    let plain: families::EventFeedSpanParams = serde_json::from_value(serde_json::json!({ "branchId": "7", "categories": [] })).unwrap();
    assert_eq!(plain.kinds_only, None);
}

#[test]
fn the_scope_index_warmed_at_open_holds_every_run_of_the_timeline() {
    use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
    use replay::campaign::Replay;
    use std::sync::{Arc, atomic::AtomicBool};
    let frames = (100..=140).map(|hour| source(1, hour, Some(1))).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = dashboard_base::Source::from_replay(dashboard_base::SourceKey { source_id: "feed-scope-warm".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    assert!(super::scope::kept(&source).is_empty(), "nothing is kept before the open reads ahead");
    super::scope::warm(&source, 1).unwrap();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let mut runs: Vec<u32> = query.timeline.stretches.iter().map(|stretch| stretch.run).collect();
    runs.sort_unstable();
    runs.dedup();
    assert!(!runs.is_empty());
    assert_eq!(super::scope::kept(&source), runs, "every run of the timeline kept");
    dashboard_base::queries::forget(&source);
    crate::forget(&source);
    assert!(super::scope::kept(&source).is_empty(), "forgotten with the recording's index");
}

#[test]
fn the_spans_coverage_notices_from_the_incomplete_facet_equal_a_walk_of_its_reads() {
    use dashboard_query::log::{Pattern, facet_shift};
    let declared = dashboard_declare::log(super::LOG).unwrap();
    let at = declared.facets.iter().position(|facet| facet.name == "incomplete").unwrap();
    let kind = declared.facets.iter().position(|facet| facet.name == "kind").unwrap();
    let base = Pattern { run: 1, first: 0, last: 1_000, mask: u128::from(u32::MAX) << facet_shift(kind), value: u128::from(FeedEventRow::READ_INCOMPLETE) << facet_shift(kind), group: 0 };
    let patterns = families::notice_patterns(&[base], facet_shift(at));
    let reads = [(100, 1, 0), (101, 0, 0b11), (102, 2, 0b100), (103, 0, 0), (104, 1, 0b1), (105, 2, 0b1), (106, 0, 0b10000), (107, 0, 0b1)];
    let rows: Vec<(u128, u128, FeedEvent)> = reads
        .iter()
        .enumerate()
        .map(|(sequence, &(hour, coverage, refusals))| {
            let mut read = event(FeedEventRow::READ_INCOMPLETE, hour, Slot::None, Slot::None);
            (read.sequence, read.coverage, read.outstanding, read.refusals) = (sequence as u64, Some(coverage), Some(2), Some(refusals));
            let mut facet = u128::MAX;
            for (shift, value) in [(facet_shift(kind), u32::from(FeedEventRow::READ_INCOMPLETE)), (facet_shift(at), FeedEventRow::incomplete_of(coverage as u8, refusals as u8))] {
                facet = facet & !(u128::from(u32::MAX) << shift) | u128::from(value) << shift;
            }
            (sequence as u128, facet, read)
        })
        .collect();
    let losses = [(90, 101)];
    let mut walked = Notices::default();
    for (_, _, read) in &rows {
        families::read_notices(read, &losses, &mut walked);
    }
    let raised = (0..families::NOTICES)
        .map(|notice| {
            let held: Vec<_> = rows.iter().filter(|(_, facet, _)| patterns.iter().any(|pattern| pattern.group == notice && facet & pattern.mask == pattern.value)).collect();
            (notice, held.len() as u64, held.iter().take(3).map(|(key, _, read)| (read.hour, *key, read.clone())).collect())
        })
        .collect();
    let mut counted = Notices::default();
    families::raised_notices(raised, &losses, &mut counted);
    let brief = |notices: Notices| notices.into_list().into_iter().map(|notice| (notice.kind, notice.label, notice.reason, notice.count)).collect::<Vec<_>>();
    let (walked, counted) = (brief(walked), brief(counted));
    assert_eq!(walked.len(), 5, "coverage, then the families 1, 2, 4 and 16 in the order the reads meet them: {walked:?}");
    assert_eq!(counted, walked, "counts, examples and order equal a walk of every read");
    let unheld = u128::from(u32::MAX) << facet_shift(at) | u128::from(FeedEventRow::READ_INCOMPLETE) << facet_shift(kind);
    assert!(!patterns.iter().any(|pattern| unheld & pattern.mask == pattern.value), "a row the facet does not hold raises no notice");
}

#[test]
fn a_scope_keeps_the_rows_naming_its_countries_by_every_role_at_the_hour_they_are_matched() {
    use super::scope::{Scope, entry};
    use dashboard_world::products::groups::MembershipRun;
    use dashboard_query::identity::{Holder, HolderRun, Holders};
    let names = Names { slots: HashMap::from([(7, vec![("GER".to_owned(), 0, 99), ("DDR".to_owned(), 100, 200)]), (8, vec![("POL".to_owned(), 0, 200)])]) };
    let held = |slot: u32, first: i32, last: i32| HolderRun { first, last, holder: Holder::Found { slot, id: slot }, starts_segment: true };
    let holders = Holders::new(
        [("GER".to_owned(), vec![held(7, 0, 99)]), ("DDR".to_owned(), vec![held(7, 100, 200)]), ("POL".to_owned(), vec![held(8, 0, 200)])].into_iter().collect(),
        Vec::new(),
    );
    let scope = |tag: &str, first_hour: i32, last_hour: i32| Scope::resolve(&holders, &[MembershipRun { tag: tag.to_owned(), first_hour, last_hour }]);
    let war = entry(50, 1, 0, &event(1, 50, Slot::Of(7), Slot::Of(8)), &names);
    assert_eq!((war.at, war.slots), (50, [7, 8, u32::MAX]));
    let mut joined = event(1, 60, Slot::Of(9), Slot::NotRecorded);
    joined.cause = Some(super::row::Cause { route: 0, within: 0, conference: None, state: None, ally: Some(party(13, Slot::Of(8))), reason: None, flags: 0 });
    let joined = entry(60, 2, 0, &joined, &names);
    assert_eq!(joined.slots, [9, u32::MAX, 8], "an unwitnessed target names no one; the ally is a role");
    let mut ceased = event(FeedEventRow::CEASED_TO_EXIST, 150, Slot::Of(7), Slot::None);
    (ceased.generation, ceased.dated) = (FeedEventRow::DERIVED, Some(90));
    let ceased = entry(150, 3, 0, &ceased, &names);
    assert_eq!((ceased.hour, ceased.at), (150, 90));
    let mut guarantee = event(FeedEventRow::GUARANTEE_GRANTED, 150, Slot::Of(8), Slot::Of(7));
    (guarantee.generation, guarantee.dated) = (FeedEventRow::DERIVED, Some(120));
    let exact = entry(150, 4, 0, &guarantee, &names);
    guarantee.dated = Some(80);
    let late = entry(150, 5, 0, &guarantee, &names);
    assert_eq!((exact.at, late.at), (120, 150), "dated at its start only where its tags held then");

    let germany = scope("GER", 0, 200);
    assert_eq!([war, joined, ceased, exact].map(|row| germany.admits(&row)), [true, false, true, false], "Germany held slot 7 through 99 only");
    let east = scope("DDR", 0, 200);
    assert_eq!([war, ceased, exact, late].map(|row| east.admits(&row)), [false, false, true, true]);
    let poland = scope("POL", 55, 65);
    assert_eq!([war, joined].map(|row| poland.admits(&row)), [false, true], "the ally counts, and only in the scope's hours");
    assert!(![war, joined, ceased, exact].iter().any(|row| scope("ITA", 0, 200).admits(row)), "a tag no one held keeps nothing");
}

#[test]
fn a_scoped_page_and_its_counts_number_only_the_rows_kept() {
    use super::scope::{Entry, Scope, count, matched, window};
    use dashboard_world::products::groups::MembershipRun;
    use dashboard_query::{identity::{Holder, HolderRun, Holders}, log::Pattern};
    use std::sync::Arc;
    let holders = Holders::new([("GER".to_owned(), vec![HolderRun { first: 0, last: 400, holder: Holder::Found { slot: 7, id: 7 }, starts_segment: true }])].into_iter().collect(), Vec::new());
    let scope = Scope::resolve(&holders, &[MembershipRun { tag: "GER".to_owned(), first_hour: 0, last_hour: 400 }]);
    let row = |hour: i32, key: u128, kind: u128, slot: u32| Entry { hour, at: hour, key, facet: kind << 96, slots: [slot, u32::MAX, u32::MAX] };
    let first = Arc::new(vec![row(10, 1, 1, 7), row(10, 2, 1, 8), row(20, 3, 2, 7), row(30, 4, 1, 7), row(40, 5, 1, 7)]);
    let second = Arc::new(vec![row(210, 1, 1, 7), row(220, 2, 1, 9), row(230, 3, 1, 7)]);
    let index = vec![(2, second), (1, first)];
    let kind = |run: u32, first: i32, last: i32| Pattern { run, first, last, mask: u128::from(u32::MAX) << 96, value: 1 << 96, group: 0 };
    let kept = matched(&index, &[kind(1, 0, 199), kind(2, 200, 400)], &scope);
    assert_eq!(kept.iter().map(|(run, entry)| (*run, entry.hour)).collect::<Vec<_>>(), [(1, 10), (1, 30), (1, 40), (2, 210), (2, 230)], "kind 2 and other countries left out, runs in hour order");
    assert_eq!(matched(&index, &[kind(1, 0, 35)], &scope).len(), 2, "a run no pattern reaches gives nothing, a pattern's hours bound it");
    assert_eq!(window(&kept, 30, 230, 0, 2), (1, 1, 3, 5), "the rows before the range are its base");
    assert_eq!(window(&kept, 30, 230, 3, 2), (1, 3, 5, 5));
    assert_eq!(window(&kept, 30, 215, 4, 100), (1, 4, 4, 4), "a cursor past the range's rows is an empty page");
    let counts = count(&kept, 30, 230, 12).unwrap();
    assert_eq!((counts.before, counts.bins), (1, vec![(0, 1), (1, 1), (15, 1), (17, 1)]), "bins on the absolute grid from the one holding the first hour");
    assert!(count(&kept, 30, 230, 7).is_err(), "bins of seven hours are refused as every log's are");
}

#[test]
fn two_projects_completing_in_one_hour_are_two_rows() {
    use dashboard_research::products::special_projects::ProjectCompletion;
    let completion = |project, ordinal| ProjectCompletion { tag: Some("GER".into()), date: 100, observed_hour: 101, owner: 9, project, ordinal, name: None, icon: None, icon_missing: None };
    let ids: std::collections::BTreeSet<String> =
        [completion(1, 4), completion(2, 4), completion(1, 7)].into_iter().filter_map(families::completion_row).map(|row| row.id).collect();
    assert_eq!(ids.len(), 3, "each project's completion marker is its own row: {ids:?}");
}

#[test]
fn decision_failed_state_requires_container_and_decodes_each_container_without_guessing() {
    use super::decision_state::{
        COUNTRY_DECISION_STATE_REFUSED, DECISION_CONTAINER_NOT_RECORDED, DECISION_INSTANCE_MISMATCH,
        DECISION_NO_MISSION_LIFECYCLE, DECISION_OCCURRENCE_NOT_FOUND, DecisionLifecycleState,
        EVENT_FEED_HISTORY_INCOMPLETE, decode, enforce_same_incarnation, not_applicable, not_recorded,
        resolve_in_pack,
    };
    use replay_data::event_feed_rows::{
        EventFeedCountry, EventFeedCounts, EventFeedCoverage, EventFeedEvent, EventFeedHistoryRow, EventFeedScript, EventFeedScriptChange,
        decisions::{DecisionChange, Operation, REMOVAL, REENABLE, TAKEN, TARGETED, TARGETED_TIMED, TIMED},
    };
    use replay_data::rows::Recorded;

    assert_eq!(decode(TIMED, 0, 100), DecisionLifecycleState::Active);
    assert_eq!(decode(TIMED, 1, 100), DecisionLifecycleState::Completed);
    assert_eq!(decode(TIMED, 2, 100), DecisionLifecycleState::Failed { observed_hour: 100 });
    assert_eq!(decode(TIMED, 3, 100), DecisionLifecycleState::Aborted);
    assert_eq!(decode(TIMED, 4, 100), DecisionLifecycleState::ReEnableCooldown);
    assert_eq!(decode(TIMED, 99, 100), DecisionLifecycleState::Unsupported { raw: 99 });

    for container in [TARGETED, TARGETED_TIMED] {
        assert_eq!(decode(container, 0, 100), DecisionLifecycleState::Available);
        assert_eq!(decode(container, 1, 100), DecisionLifecycleState::Completed);
        assert_eq!(decode(container, 2, 100), DecisionLifecycleState::ReEnableCooldown);
        assert_eq!(decode(container, 3, 100), DecisionLifecycleState::Failed { observed_hour: 100 });
        assert_eq!(decode(container, 4, 100), DecisionLifecycleState::Aborted);
        assert_eq!(decode(container, -7, 100), DecisionLifecycleState::Unsupported { raw: -7 });
    }
    for container in [TAKEN, REENABLE, REMOVAL] {
        assert_eq!(decode(container, 2, 100), not_applicable(DECISION_NO_MISSION_LIFECYCLE));
    }
    assert_eq!(decode(255, 2, 100), not_recorded(DECISION_CONTAINER_NOT_RECORDED));

    let actor = EventFeedCountry { reference: 11, id: 11, instance: 1 };
    let feed_ev = |sequence: u64, source: EventFeedSource, instance: Option<u32>, lifecycle: Option<i32>| {
        let mut ev = event(source as u8, 100, Slot::Of(7), Slot::None);
        ev.sequence = sequence;
        ev.read_hour = 101;
        ev.instance = instance;
        ev.lifecycle = lifecycle;
        ev
    };
    let history_ev = |sequence: u64, source: EventFeedSource, change: Option<DecisionChange>, instance: Option<u32>, lifecycle: Option<i32>| EventFeedEvent {
        sequence,
        generation: 1,
        hour: 100,
        source,
        actor,
        target: EventFeedCountry::default(),
        cause: Recorded::NotRecorded,
        script: Some(EventFeedScript {
            instance,
            lifecycle,
            change: change.map(EventFeedScriptChange::Decision),
            ..EventFeedScript::default()
        }),
    };
    let make_pack = |coverage: EventFeedCoverage, refusals: u8, events: Vec<EventFeedEvent>| EventFeedHistoryRow {
        generation: 1,
        read_hour: 101,
        coverage,
        counts: Some(EventFeedCounts::default()),
        outstanding: 0,
        failure: None,
        script_refusals: refusals,
        events,
        script_tokens: Vec::new(),
        script_scopes: Vec::new(),
        windows: Default::default(),
    };

    let timed_fail = feed_ev(10, EventFeedSource::DecisionTimedOut, Some(50), Some(2));
    let ok_pack = make_pack(
        EventFeedCoverage::Complete,
        0,
        vec![history_ev(10, EventFeedSource::DecisionTimedOut, Some(DecisionChange::new(Operation::Update, TIMED)), Some(50), Some(2))],
    );
    assert_eq!(resolve_in_pack(&timed_fail, &ok_pack), DecisionLifecycleState::Failed { observed_hour: 101 });

    let targeted_not_failed = make_pack(
        EventFeedCoverage::Complete,
        0,
        vec![history_ev(10, EventFeedSource::DecisionTimedOut, Some(DecisionChange::new(Operation::Update, TARGETED_TIMED)), Some(50), Some(2))],
    );
    assert_eq!(resolve_in_pack(&timed_fail, &targeted_not_failed), DecisionLifecycleState::ReEnableCooldown);

    let instant = feed_ev(12, EventFeedSource::DecisionSelected, None, None);
    let instant_pack = make_pack(
        EventFeedCoverage::Complete,
        0,
        vec![history_ev(12, EventFeedSource::DecisionSelected, None, None, None)],
    );
    assert_eq!(resolve_in_pack(&instant, &instant_pack), not_applicable(DECISION_NO_MISSION_LIFECYCLE));
    let taken_pack = make_pack(
        EventFeedCoverage::Complete,
        0,
        vec![history_ev(12, EventFeedSource::DecisionSelected, Some(DecisionChange::new(Operation::Store, TAKEN)), None, None)],
    );
    assert_eq!(resolve_in_pack(&instant, &taken_pack), not_applicable(DECISION_NO_MISSION_LIFECYCLE));
    let effect = feed_ev(13, EventFeedSource::DecisionTimeoutEffect, None, None);
    let effect_pack = make_pack(
        EventFeedCoverage::Complete,
        0,
        vec![history_ev(13, EventFeedSource::DecisionTimeoutEffect, None, None, None)],
    );
    assert_eq!(resolve_in_pack(&effect, &effect_pack), not_applicable(DECISION_NO_MISSION_LIFECYCLE));

    let partial_instant = make_pack(EventFeedCoverage::Partial, 0, instant_pack.events.clone());
    assert_eq!(resolve_in_pack(&instant, &partial_instant), not_recorded(EVENT_FEED_HISTORY_INCOMPLETE));
    let refused_instant = make_pack(EventFeedCoverage::Complete, 1, instant_pack.events.clone());
    assert_eq!(resolve_in_pack(&instant, &refused_instant), not_recorded(EVENT_FEED_HISTORY_INCOMPLETE));

    let partial = make_pack(EventFeedCoverage::Partial, 0, ok_pack.events.clone());
    assert_eq!(resolve_in_pack(&timed_fail, &partial), not_recorded(EVENT_FEED_HISTORY_INCOMPLETE));
    let unavailable = make_pack(EventFeedCoverage::Unavailable, 0, ok_pack.events.clone());
    assert_eq!(resolve_in_pack(&timed_fail, &unavailable), not_recorded(EVENT_FEED_HISTORY_INCOMPLETE));
    let refused_family = make_pack(EventFeedCoverage::Complete, 1, ok_pack.events.clone());
    assert_eq!(resolve_in_pack(&timed_fail, &refused_family), not_recorded(EVENT_FEED_HISTORY_INCOMPLETE));

    for bad_op in [Operation::Refused, Operation::Invalidate] {
        let mut events = ok_pack.events.clone();
        events.push(history_ev(11, EventFeedSource::ScriptState, Some(DecisionChange::new(bad_op, 0)), None, None));
        assert_eq!(resolve_in_pack(&timed_fail, &make_pack(EventFeedCoverage::Complete, 0, events)), not_recorded(COUNTRY_DECISION_STATE_REFUSED));

        let mut instant_events = instant_pack.events.clone();
        instant_events.push(history_ev(14, EventFeedSource::ScriptState, Some(DecisionChange::new(bad_op, 0)), None, None));
        assert_eq!(resolve_in_pack(&instant, &make_pack(EventFeedCoverage::Complete, 0, instant_events)), not_recorded(COUNTRY_DECISION_STATE_REFUSED));
    }

    let empty_pack = make_pack(EventFeedCoverage::Complete, 0, Vec::new());
    assert_eq!(resolve_in_pack(&timed_fail, &empty_pack), not_recorded(DECISION_OCCURRENCE_NOT_FOUND));

    let missing_lifecycle_timeout = feed_ev(15, EventFeedSource::DecisionTimedOut, Some(50), None);
    let missing_lifecycle_pack = make_pack(
        EventFeedCoverage::Complete,
        0,
        vec![history_ev(15, EventFeedSource::DecisionTimedOut, Some(DecisionChange::new(Operation::Update, TIMED)), Some(50), None)],
    );
    assert_eq!(resolve_in_pack(&missing_lifecycle_timeout, &missing_lifecycle_pack), not_recorded(DECISION_CONTAINER_NOT_RECORDED));

    let no_container = make_pack(
        EventFeedCoverage::Complete,
        0,
        vec![history_ev(10, EventFeedSource::DecisionTimedOut, None, Some(50), Some(2))],
    );
    assert_eq!(resolve_in_pack(&timed_fail, &no_container), not_recorded(DECISION_CONTAINER_NOT_RECORDED));

    let no_instance = make_pack(
        EventFeedCoverage::Complete,
        0,
        vec![history_ev(10, EventFeedSource::DecisionTimedOut, Some(DecisionChange::new(Operation::Update, TIMED)), None, Some(2))],
    );
    assert_eq!(resolve_in_pack(&timed_fail, &no_instance), not_recorded(DECISION_CONTAINER_NOT_RECORDED));

    let erase_op = make_pack(
        EventFeedCoverage::Complete,
        0,
        vec![history_ev(10, EventFeedSource::DecisionTimedOut, Some(DecisionChange::new(Operation::Erase, TIMED)), Some(50), Some(2))],
    );
    assert_eq!(resolve_in_pack(&timed_fail, &erase_op), not_recorded(DECISION_CONTAINER_NOT_RECORDED));

    let opener = feed_ev(9, EventFeedSource::DecisionActivated, Some(49), Some(0));
    let mut rows = vec![
        Some(super::EventFeedRow {
            id: opener.id(),
            hour: 100,
            read_hour: 100,
            name_hour: 100,
            exact: true,
            category: super::EventFeedCategory::Politics,
            kind: EventFeedKind::DecisionActivated,
            tags: Vec::new(),
            participants: None,
            item: None,
            amount: None,
            cause: None,
            strike: None,
            decision_state: Some(DecisionLifecycleState::Active),
            window: None,
        }),
        Some(super::EventFeedRow {
            id: timed_fail.id(),
            hour: 100,
            read_hour: 101,
            name_hour: 100,
            exact: true,
            category: super::EventFeedCategory::Politics,
            kind: EventFeedKind::DecisionTimedOut,
            tags: Vec::new(),
            participants: None,
            item: None,
            amount: None,
            cause: None,
            strike: None,
            decision_state: Some(DecisionLifecycleState::Failed { observed_hour: 101 }),
            window: None,
        }),
    ];
    enforce_same_incarnation(&[&opener, &timed_fail], &mut rows);
    assert_eq!(
        rows[0].as_ref().and_then(|row| row.decision_state.as_ref()),
        Some(&DecisionLifecycleState::Active),
        "earlier same-incarnation edge stays unaffected before a later mismatch"
    );
    assert_eq!(
        rows[1].as_ref().and_then(|row| row.decision_state.as_ref()),
        Some(&DecisionLifecycleState::NotRecorded { reason: DECISION_INSTANCE_MISMATCH.to_owned() })
    );
}

#[test]
fn decision_failed_state_handles_same_hour_duplicates_branch_boundaries_and_missing_inputs() {
    use super::{
        Decisions, EventFeed, FeedPageParams,
        decision_state::{
            DECISION_CONTAINER_NOT_RECORDED, DECISION_INSTANCE_MISMATCH, DECISION_NO_MISSION_LIFECYCLE,
            DecisionLifecycleState, EVENT_FEED_HISTORY_NOT_RECORDED, resolve_all,
        },
    };
    use dashboard_base::{
        products::{LoadContext, Product},
        test_fixtures::{CAMPAIGN, records, source},
    };
    use replay::campaign::Replay;
    use replay_data::{
        event_feed_rows::{
            EventFeedCountry, EventFeedCounts, EventFeedCoverage, EventFeedEvent, EventFeedHistoryRow, EventFeedScript, EventFeedScriptChange,
            decisions::{DecisionChange, Operation, TAKEN, TARGETED_TIMED, TIMED},
        },
        rows::{Recorded, TableId, TableRows},
    };
    use replay_frame::frame::TableView;
    use std::sync::{Arc, atomic::AtomicBool};

    let ger = EventFeedCountry { reference: 9001, id: 9001, instance: 1 };
    let tokens = b"mod_mission_amod_mission_bmod_mission_cmod_mission_dmod_instant_e";
    let span = |index: u32| (index * 13, 13);
    let dec_ev = |sequence: u64, hour: i32, source: EventFeedSource, tok: u32, change: Option<DecisionChange>, instance: Option<u32>, lifecycle: Option<i32>, target: Option<u32>| {
        let (token_start, token_len) = span(tok);
        EventFeedEvent {
            sequence,
            generation: 1,
            hour,
            source,
            actor: ger,
            target: EventFeedCountry::default(),
            cause: Recorded::NotRecorded,
            script: Some(EventFeedScript {
                token_start,
                token_len,
                instance,
                lifecycle,
                country_target: target,
                change: change.map(EventFeedScriptChange::Decision),
                ..EventFeedScript::default()
            }),
        }
    };
    let pack = |read_hour: i32, coverage: EventFeedCoverage, events: Vec<EventFeedEvent>| {
        let count = events.len() as u64;
        EventFeedHistoryRow {
            generation: 1,
            read_hour,
            coverage,
            counts: Some(EventFeedCounts { acquired: count, settled: count, failed: 0, rejected: 0 }),
            outstanding: 0,
            failure: None,
            script_refusals: 0,
            events,
            script_tokens: tokens.to_vec(),
            script_scopes: Vec::new(),
            windows: Default::default(),
        }
    };
    let with_pack = |branch: u64, hour: i32, row: Option<EventFeedHistoryRow>| {
        let mut frame = source(branch, hour, Some(1));
        *frame.tables[TableId::EventFeedHistory.index()] = match row {
            Some(row) => {
                frame.metrics.table_ns[TableId::EventFeedHistory.index()] = replay_frame::frame::READ_TIME_NOT_RECORDED;
                TableView::Complete(TableRows::EventFeedHistory(vec![row]))
            }
            None => TableView::Unavailable { reason: "older recording lacks event feed history".into() },
        };
        frame
    };

    let b1_h100 = pack(
        100,
        EventFeedCoverage::Complete,
        vec![
            dec_ev(1, 100, EventFeedSource::DecisionActivated, 0, Some(DecisionChange::new(Operation::Store, TIMED)), Some(10), Some(0), None),
            dec_ev(2, 100, EventFeedSource::DecisionActivated, 1, Some(DecisionChange::new(Operation::Store, TIMED)), Some(11), Some(0), None),
            dec_ev(3, 100, EventFeedSource::DecisionActivated, 2, Some(DecisionChange::new(Operation::Store, TARGETED_TIMED)), Some(12), Some(0), Some(9001)),
            dec_ev(4, 100, EventFeedSource::DecisionActivated, 3, Some(DecisionChange::new(Operation::Store, TIMED)), Some(13), Some(0), None),
            dec_ev(9, 100, EventFeedSource::DecisionSelected, 4, Some(DecisionChange::new(Operation::Store, TAKEN)), None, None, None),
        ],
    );
    let b1_h101 = pack(
        101,
        EventFeedCoverage::Complete,
        vec![
            dec_ev(5, 101, EventFeedSource::DecisionTimedOut, 0, Some(DecisionChange::new(Operation::Update, TIMED)), Some(10), Some(1), None),
            dec_ev(6, 101, EventFeedSource::DecisionTimedOut, 1, Some(DecisionChange::new(Operation::Update, TIMED)), Some(11), Some(2), None),
            dec_ev(7, 101, EventFeedSource::DecisionTimedOut, 2, Some(DecisionChange::new(Operation::Update, TARGETED_TIMED)), Some(12), Some(3), Some(9001)),
            dec_ev(8, 101, EventFeedSource::DecisionCompleted, 3, Some(DecisionChange::new(Operation::Update, TIMED)), Some(99), Some(2), None),
        ],
    );
    let b2_h100 = pack(
        100,
        EventFeedCoverage::Complete,
        vec![dec_ev(2, 100, EventFeedSource::DecisionActivated, 1, None, Some(11), Some(0), None)],
    );
    let b2_h101 = pack(
        101,
        EventFeedCoverage::Complete,
        vec![dec_ev(6, 101, EventFeedSource::DecisionTimedOut, 1, None, Some(11), Some(2), None)],
    );

    let replay = Replay::from_records(
        recorder_derive::writers(),
        CAMPAIGN.into(),
        &[
            records(1, vec![with_pack(1, 100, Some(b1_h100)), with_pack(1, 101, Some(b1_h101))]),
            records(2, vec![with_pack(2, 100, Some(b2_h100)), with_pack(2, 101, Some(b2_h101))]),
            records(3, vec![with_pack(3, 100, None), with_pack(3, 101, None)]),
        ],
    )
    .unwrap()
    .into_timelines(&Default::default())
    .unwrap();
    let replay_source = dashboard_base::Source::from_replay(
        dashboard_base::SourceKey { source_id: "decision-failed".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let ask = |branch_id: &str| FeedPageParams {
        branch_id: branch_id.into(),
        facets: Vec::new(),
        from: Some(100.0),
        to: 101.0,
        cursor: None,
        page_size: 64,
        scope: None,
    };

    let feed_b1 = EventFeed::load(&replay_source, &ask("1"), &context, &cancel).unwrap();
    let state_of = |page: &super::EventFeedPage, id: &str| {
        page.rows.iter().filter_map(|row| row.row.as_ref()).find(|row| row.id == id).and_then(|row| row.decision_state.clone())
    };
    assert_eq!(state_of(&feed_b1, "native:1:5"), Some(DecisionLifecycleState::Completed));
    assert_eq!(state_of(&feed_b1, "native:1:6"), Some(DecisionLifecycleState::Failed { observed_hour: 101 }));
    assert_eq!(state_of(&feed_b1, "native:1:7"), Some(DecisionLifecycleState::Failed { observed_hour: 101 }));
    assert_eq!(state_of(&feed_b1, "native:1:8"), Some(DecisionLifecycleState::Failed { observed_hour: 101 }));
    assert_eq!(
        state_of(&feed_b1, "native:1:9"),
        Some(DecisionLifecycleState::NotApplicable { reason: DECISION_NO_MISSION_LIFECYCLE.to_owned() })
    );

    let card_end_state = |page: &super::cards::ScriptOccurrencePage, opener_id: &str| {
        page.rows
            .iter()
            .filter_map(|row| row.occurrence.as_ref())
            .find(|occ| occ.id == opener_id)
            .and_then(|occ| occ.edges.last())
            .and_then(|edge| edge.row.decision_state.clone())
    };
    let cards_b1_at_100 = Decisions::load(
        &replay_source,
        &FeedPageParams { branch_id: "1".into(), facets: Vec::new(), from: Some(100.0), to: 100.0, cursor: None, page_size: 64, scope: None },
        &context,
        &cancel,
    )
    .unwrap();
    assert_eq!(card_end_state(&cards_b1_at_100, "native:1:2"), Some(DecisionLifecycleState::Active));
    assert_eq!(card_end_state(&cards_b1_at_100, "native:1:4"), Some(DecisionLifecycleState::Active));
    assert_eq!(
        card_end_state(&cards_b1_at_100, "native:1:9"),
        Some(DecisionLifecycleState::NotApplicable { reason: DECISION_NO_MISSION_LIFECYCLE.to_owned() })
    );

    let cards_b1 = Decisions::load(&replay_source, &ask("1"), &context, &cancel).unwrap();
    assert_eq!(card_end_state(&cards_b1, "native:1:1"), Some(DecisionLifecycleState::Completed));
    assert_eq!(card_end_state(&cards_b1, "native:1:2"), Some(DecisionLifecycleState::Failed { observed_hour: 101 }));
    assert_eq!(card_end_state(&cards_b1, "native:1:3"), Some(DecisionLifecycleState::Failed { observed_hour: 101 }));
    assert_eq!(
        card_end_state(&cards_b1, "native:1:4"),
        Some(DecisionLifecycleState::NotRecorded { reason: DECISION_INSTANCE_MISMATCH.to_owned() }),
        "different instance incarnation across opener and end must withhold failure state"
    );

    let feed_b2 = EventFeed::load(&replay_source, &ask("2"), &context, &cancel).unwrap();
    assert_eq!(
        state_of(&feed_b2, "native:1:6"),
        Some(DecisionLifecycleState::NotRecorded { reason: DECISION_CONTAINER_NOT_RECORDED.to_owned() }),
        "branch 2 lacks container metadata and must never join branch 1"
    );

    let query_b3 = dashboard_base::queries::query(&replay_source, 3).unwrap();
    let mut bare_timed_out = event(EventFeedSource::DecisionTimedOut as u8, 101, Slot::Of(7), Slot::None);
    bare_timed_out.sequence = 6;
    bare_timed_out.actor = party(9001, Slot::Of(7));
    bare_timed_out.instance = Some(11);
    bare_timed_out.lifecycle = Some(2);
    let mut bare_instant = event(EventFeedSource::DecisionSelected as u8, 100, Slot::Of(7), Slot::None);
    bare_instant.sequence = 9;
    bare_instant.actor = party(9001, Slot::Of(7));
    assert_eq!(
        resolve_all(&query_b3, &[&bare_timed_out, &bare_instant]).unwrap(),
        vec![
            Some(DecisionLifecycleState::NotRecorded { reason: EVENT_FEED_HISTORY_NOT_RECORDED.to_owned() }),
            Some(DecisionLifecycleState::NotRecorded { reason: EVENT_FEED_HISTORY_NOT_RECORDED.to_owned() }),
        ],
        "missing EventFeedHistory table must refuse by name rather than guessing or returning notApplicable"
    );
}
