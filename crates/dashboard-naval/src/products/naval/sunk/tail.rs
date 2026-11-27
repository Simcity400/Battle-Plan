use super::super::sinking_details::{BattleIndex, ShipSinkingAttribution, ShipSinkingBattle, battle_not_recorded};
use super::{Content, Named, ShipClassInfo, stored, text};
use dashboard_base::{
    Source,
    catalogue::{self, Catalogue},
    products::{
        LoadContext, Product,
        occurrences::{self, OccurrenceReply},
    },
    queries::{self, country_ids::Names},
    span::HourSpan,
};
use dashboard_follow::panel::Panel;
use dashboard_query::{
    Query,
    identity::{Country, Holders},
    log::occurrence::{Ask, Direction, Index, Occurrence, Order, PlacedGap, Role, Selection},
    series::{MissingSpan, ranked::GroupSelection},
};
use dashboard_text::metadata::check_cancel;
use replay_columnar::manifest::Status;
use replay_data::rows::{SunkShipEntry, TableId};
use replay_lanes::{
    at::Seeker,
    lanes::{self, Cell, History, Keys, Reading},
};
use replay_recording::columnar_frames::TableCoverage;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShipsSunkTailParams {
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hour: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub from: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub to: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub countries: Option<Vec<Country>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub offset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub classes: Option<Vec<u32>>,
    pub limit: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ShipClassChip {
    pub class: ShipClassInfo,
    pub first_hour: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SinkingRow {
    pub key: String,
    pub hour: i32,
    pub observed: i32,
    pub name: Option<String>,
    pub class_token: u32,
    pub recorded_class: Option<String>,
    pub victim: Option<String>,
    pub attacker: Option<String>,
    pub sea_zone: Option<String>,
    pub sinking_cause: ShipSinkingAttribution,
    pub battle: ShipSinkingBattle,
    pub battle_province: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ShipsSunkTailReply {
    pub branch_id: String,
    pub total: String,
    pub offset: String,
    pub classes: Vec<ShipClassChip>,
    pub rows: Vec<SinkingRow>,
    pub missing: Vec<PlacedGap>,
    pub next_change: Option<i32>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ShipsSunkTailReply);

impl OccurrenceReply for ShipsSunkTailReply {
    fn next_change(&self) -> Option<i32> {
        self.next_change
    }
    fn row_count(&self) -> usize {
        self.rows.len()
    }
}

struct Tail {
    index: Index<SinkingRow>,
    classes: Vec<ShipClassChip>,
    kinds: HashMap<u32, u16>,
    warnings: Vec<String>,
}

struct Sighting {
    entry: SunkShipEntry,
    observed: i32,
}

struct ProvinceAsk {
    combat: bool,
    run: u32,
    key: u128,
    hour: i32,
    before: Option<i32>,
}

fn province_ask(query: &Query<'_>, battle: &ShipSinkingBattle, entry: &SunkShipEntry) -> Option<ProvinceAsk> {
    let ShipSinkingBattle::Linked { first_hour, result_id, result_hour, .. } = battle else { return None };
    if let Some(first) = *first_hour {
        let hour = entry.sink_hour;
        let at = query.timeline.at(hour)?;
        let before = (hour > first && hour > at.first).then_some(hour - 1);
        return Some(ProvinceAsk { combat: true, run: at.run, key: u128::from(entry.battle), hour, before });
    }
    let id = result_id.as_deref().and_then(super::super::battles::parse_result_id)?;
    let hour = (*result_hour)?;
    let at = query.timeline.at(hour)?;
    Some(ProvinceAsk { combat: false, run: at.run, key: u128::from(id), hour, before: None })
}

fn province_at(history: &History, key: u128, hour: i32) -> Option<Option<u32>> {
    let at = history.statuses.partition_point(|held| held.last < hour);
    history.statuses.get(at).filter(|held| held.first <= hour && matches!(held.status, Status::Complete))?;
    Some(match history.reading(key, hour) {
        Reading::Cells([Cell::Number(province), ..]) => u32::try_from(*province).ok(),
        _ => None,
    })
}

fn battle_provinces(query: &Query<'_>, asks: &[Option<ProvinceAsk>]) -> io::Result<Vec<Option<u32>>> {
    let recording = query.replay.recording();
    let mut groups = BTreeMap::<(bool, u32), (BTreeSet<u128>, i32, i32)>::new();
    for ask in asks.iter().flatten() {
        let (keys, from, to) = groups.entry((ask.combat, ask.run)).or_insert_with(|| (BTreeSet::new(), ask.hour, ask.hour));
        keys.insert(ask.key);
        *from = (*from).min(ask.before.unwrap_or(ask.hour));
        *to = (*to).max(ask.hour);
    }
    let mut histories = HashMap::new();
    for ((combat, run), (keys, from, to)) in groups {
        let table = if combat { TableId::NavalCombats } else { TableId::NavalResults };
        let index = table.index();
        let from = match recording.table_coverage(index, run) {
            TableCoverage::Valid => from,
            TableCoverage::WithheldAll(_) => continue,
            TableCoverage::WithheldBefore { ordinal, .. } => from.max(recording.run(run)?.first_hour.saturating_add_unsigned(ordinal)),
        };
        if from > to {
            continue;
        }
        let keys: Vec<u128> = keys.into_iter().collect();
        match dashboard_query::cost::storage(|| lanes::history(recording, run, table, &[0], Keys::These(&keys), from, to)) {
            Ok(history) => {
                histories.insert((combat, run), history);
            }
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {}
            Err(error) => return Err(error),
        }
    }
    Ok(asks
        .iter()
        .map(|ask| {
            let ask = ask.as_ref()?;
            let history = histories.get(&(ask.combat, ask.run))?;
            match province_at(history, ask.key, ask.hour)? {
                Some(province) => Some(province),
                None => province_at(history, ask.key, ask.before?).flatten(),
            }
        })
        .collect())
}

fn scan(source: &Source, query: &Query<'_>, content: &Content, cancelled: &AtomicBool) -> io::Result<Tail> {
    let recording = query.replay.recording();
    let table = TableId::SunkShips;
    let mut seeker = Seeker::new(recording)?;
    let mut sightings: Vec<Sighting> = Vec::new();
    let mut seen = HashMap::<i32, Vec<(usize, usize)>>::new();
    let mut gaps = Vec::new();
    let mut literals = HashMap::new();
    let mut covered = Vec::new();
    for stretch in &query.timeline.stretches {
        check_cancel(cancelled)?;
        if recording.predates(table.index()).is_some() || recording.withheld_reason(table.index()).is_some() {
            continue;
        }
        let history = lanes::history(recording, stretch.run, table, &[0], Keys::All, stretch.first, stretch.last)?;
        let start = sightings.len();
        let steps = history.keys.first().map_or(&[][..], |(_, steps)| steps.as_slice());
        for span in history.statuses.iter().filter(|span| span.status == Status::Complete) {
            let a = steps.partition_point(|step| step.hour <= span.first);
            let b = steps.partition_point(|step| step.hour <= span.last);
            for hour in std::iter::once(span.first).chain(steps[a..b].iter().map(|step| step.hour)) {
                check_cancel(cancelled)?;
                let Some((_, entries)) = super::sinkings_seen(&mut seeker, stretch.run, hour)? else { continue };
                let end = steps.get(steps.partition_point(|step| step.hour <= hour)).map_or(span.last, |step| span.last.min(step.hour - 1));
                covered.push(HourSpan { first_hour: hour, last_hour: end });
                let mut in_read = HashMap::<usize, usize>::new();
                for entry in entries {
                    let copies = seen.entry(entry.sink_hour).or_default();
                    if let Some((at, count)) = copies.iter_mut().find(|(at, _)| sightings[*at].entry == entry) {
                        let current = in_read.entry(*at).or_default();
                        *current += 1;
                        if *current <= *count {
                            continue;
                        }
                        *count = *current;
                    } else {
                        let at = sightings.len();
                        copies.push((at, 1));
                        in_read.insert(at, 1);
                    }
                    sightings.push(Sighting { observed: hour.max(entry.sink_hour), entry });
                }
            }
        }
        let tokens = sightings[start..]
            .iter()
            .flat_map(|s| {
                std::iter::once(s.entry.class_token)
                    .chain((s.entry.attribution_class_present != 0).then_some(s.entry.attribution_class_token))
            })
            .filter(|token| !literals.contains_key(token))
            .collect();
        literals.extend(super::literals(&mut seeker, stretch.run, stretch.last, &tokens)?);
    }
    let mut first = query.timeline.first;
    let reason = recording
        .predates(table.index())
        .or_else(|| recording.withheld_reason(table.index()))
        .map_or_else(|| "Ship sinking list not recorded".to_owned(), |reason| reason.to_string());
    for span in dashboard_world::products::subjects::settle(covered) {
        if first < span.first_hour {
            gaps.push(MissingSpan { first_hour: first, last_hour: span.first_hour - 1, reason: reason.clone() });
        }
        first = span.last_hour.saturating_add(1);
    }
    if first <= query.timeline.last {
        gaps.push(MissingSpan { first_hour: first, last_hour: query.timeline.last, reason });
    }
    let held: Vec<_> = sightings.iter().map(|s| &s.entry).collect();
    let tokens = held.iter().map(|e| e.class_token).collect();
    let attr_tokens = held.iter().filter(|e| e.attribution_class_present != 0).map(|e| e.attribution_class_token).collect();
    let mut named = Named::from_literals(tokens, attr_tokens, &literals, &held, content);
    let combats = held.iter().map(|entry| entry.battle).filter(|id| *id != 0).collect();
    let battles = BattleIndex::build(source, query, query.timeline.last, &combats, &BTreeSet::new())?;
    let resolved: Vec<_> = sightings.iter().map(|sight| battles.resolve_sinking(sight.entry.battle, sight.entry.sink_hour)).collect();
    let asks: Vec<_> = sightings.iter().zip(&resolved).map(|(sight, battle)| province_ask(query, battle, &sight.entry)).collect();
    let provinces = battle_provinces(query, &asks)?;
    let names = Names::new(&query.holders);
    let mut kinds = HashMap::new();
    let mut chips = BTreeMap::<u32, ShipClassChip>::new();
    let mut entries = Vec::with_capacity(sightings.len());
    let mut refused = std::mem::take(&mut named.refused);
    for ((sight, battle), battle_province) in sightings.into_iter().zip(resolved).zip(provinces) {
        let entry = sight.entry;
        let hour = entry.sink_hour;
        let observed = sight.observed;
        let at = query.timeline.at(hour);
        let links = at.map(|at| queries::links(source, at.run)).transpose()?;
        let slot = |raw| links.as_ref().and_then(|links| (raw != 0).then(|| links.slot_at(raw, hour)).flatten());
        let (attacker, victim) = (slot(entry.attacker_country), slot(entry.victim_country));
        let class = &named.classes[&entry.class_token];
        let next = kinds.len();
        let kind = match kinds.entry(entry.class_token) {
            std::collections::hash_map::Entry::Occupied(e) => *e.get(),
            std::collections::hash_map::Entry::Vacant(e) => *e.insert(
                u16::try_from(next).map_err(|_| dashboard_core::invalid("Too many distinct ship classes for the occurrence index"))?,
            ),
        };
        chips
            .entry(entry.class_token)
            .and_modify(|chip| chip.first_hour = chip.first_hour.min(observed))
            .or_insert_with(|| ShipClassChip { class: class.clone(), first_hour: observed });
        let id = entries.len();
        entries.push(Occurrence {
            id: id as u128,
            date: hour,
            observed,
            kind,
            roles: [Role::dated(victim, hour, observed), Role::dated(attacker, hour, observed)].into(),
            subjects: Box::default(),
            sort_keys: Box::default(),
            states: Box::default(),
            text: Default::default(),
            payload: SinkingRow {
                key: id.to_string(),
                hour,
                observed,
                name: stored(&entry.ship_name, "ship name", content, &mut refused),
                class_token: entry.class_token,
                recorded_class: text(&entry.class_name, "recorded class name", &mut refused),
                victim: victim.and_then(|slot| names.slot_tag(slot, hour)).map(str::to_owned),
                attacker: attacker.and_then(|slot| names.slot_tag(slot, hour)).map(str::to_owned),
                sea_zone: (entry.province_present != 0).then(|| named.places.get(&entry.province).cloned()).flatten(),
                sinking_cause: named.attribution_for(&entry, content, &mut refused),
                battle,
                battle_province,
            },
        });
    }
    let mut classes: Vec<_> = chips.into_values().collect();
    classes.sort_by_key(|chip| (chip.first_hour, chip.class.token));
    let mut warnings = named.warnings;
    warnings.extend(refused.lines("Ships sunk fields"));
    Ok(Tail { index: Index::new(entries, gaps, &[])?, classes, kinds, warnings })
}

#[derive(Default)]
struct Branches(HashMap<u64, (Result<Arc<Catalogue>, String>, Arc<Tail>)>);

struct Prepared {
    tail: Arc<Tail>,
    selection: Option<Selection>,
    kinds: Option<Vec<u16>>,
}

fn prepare(
    source: &Source,
    query: &Query<'_>,
    load: &LoadContext,
    params: &ShipsSunkTailParams,
    cancelled: &AtomicBool,
) -> io::Result<Prepared> {
    let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
    let binding = load.content().and_then(|options| catalogue::catalogue(source, options)).map_err(|err| err.to_string());
    let state = source.state::<Branches>();
    let mut branches = state.lock().unwrap_or_else(PoisonError::into_inner);
    let tail = if let Some((_, tail)) = branches.0.get(&branch).filter(|(held, _)| match (held, &binding) {
        (Ok(a), Ok(b)) => Arc::ptr_eq(a, b),
        (Err(a), Err(b)) => a == b,
        _ => false,
    }) {
        Arc::clone(tail)
    } else {
        let tail = Arc::new(scan(source, query, &Content::from_catalogue(binding.clone()), cancelled)?);
        branches.0.insert(branch, (binding, Arc::clone(&tail)));
        tail
    };
    drop(branches);
    let kinds = params.classes.as_ref().map(|tokens| tokens.iter().filter_map(|token| tail.kinds.get(token).copied()).collect());
    let selection = dashboard_world::products::group_members::dated(
        source,
        query,
        load,
        params.countries.as_deref(),
        params.group.as_ref(),
        cancelled,
    )?;
    Ok(Prepared { tail, selection, kinds })
}

fn reply(prepared: &Prepared, holders: &Holders, params: &ShipsSunkTailParams, hour: i32) -> io::Result<ShipsSunkTailReply> {
    let (offset, limit) = occurrences::window(params.offset.as_deref(), params.limit)?;
    let tail = &prepared.tail;
    let answer = tail.index.ask(
        holders,
        &Ask {
            hour,
            from: params.from,
            to: params.to,
            gap_end: None,
            selection: prepared.selection.as_ref(),
            kinds: prepared.kinds.as_deref(),
            subject: None,
            states: None,
            search: None,
            order: Order::Date(Direction::Descending),
            live_first: false,
            offset,
            limit,
            chips: &[],
        },
    )?;
    let end = tail.classes.partition_point(|chip| chip.first_hour <= hour);
    let mut next_change = answer.next_change.into_iter().chain(tail.classes.get(end).map(|chip| chip.first_hour)).min();
    let rows = answer
        .rows
        .iter()
        .map(|shown| {
            let mut row = tail.index.get(shown.at).payload.clone();
            if let ShipSinkingBattle::Linked { live_id, result_id, result_hour, .. } = &mut row.battle
                && let Some(at) = *result_hour
                && at > hour
            {
                next_change = Some(next_change.map_or(at, |next| next.min(at)));
                if live_id.is_some() {
                    *result_id = None;
                    *result_hour = None;
                } else {
                    row.battle = battle_not_recorded();
                    row.battle_province = None;
                }
            }
            row
        })
        .collect();
    let mut warnings = tail.warnings.clone();
    if answer.unattributed > 0 {
        warnings.push(format!("Sinkings without a recorded country for this filter: {}", answer.unattributed));
    }
    let reply = ShipsSunkTailReply {
        branch_id: params.branch_id.clone(),
        total: answer.total.to_string(),
        offset: answer.offset.to_string(),
        next_change,
        classes: tail.classes[..end].to_vec(),
        rows,
        missing: answer.gaps,
        warnings,
    };
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

pub struct ShipsSunkTail;

impl Product for ShipsSunkTail {
    const KEY: &'static str = "ships-sunk-tail";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = ShipsSunkTailParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, hour: Read, from: Start, to: End, countries, "countries[]", group, offset, classes, "classes[]", limit];
    type Reply = ShipsSunkTailReply;
    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            ShipsSunkTailParams::decl(),
            ShipsSunkTailReply::decl(),
            ShipClassChip::decl(),
            ShipClassInfo::decl(),
            SinkingRow::decl(),
            ShipSinkingAttribution::decl(),
            ShipSinkingBattle::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        params.countries.iter().flatten().try_for_each(dashboard_base::products::validate_country)
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        check_cancel(cancelled)?;
        let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let hour = occurrences::cutoff(&query, params.hour)?;
        let prepared = prepare(source, &query, load, params, cancelled)?;
        reply(&prepared, &query.holders, params, hour)
    }
}

pub fn dated_panel(source: &Source, load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn Panel>, String> {
    let mut request = asked.clone();
    request.as_object_mut().ok_or("Invalid ships-sunk-tail panel")?.remove("panel");
    let params: ShipsSunkTailParams =
        dashboard_query::named::named(&request).map_err(|error| format!("Invalid ships-sunk-tail panel: {error}"))?;
    ShipsSunkTail::validate(&params)?;
    let branch = params.branch_id.parse().map_err(|_| "Invalid branch")?;
    let query = dashboard_base::queries::query(source, branch).map_err(|error| error.to_string())?;
    let (finish, prepared) = dashboard_base::products::Finish::open(source, load, ShipsSunkTail::CONTENT_DEPENDENT, || prepare(source, &query, load, &params, &AtomicBool::new(false)));
    let prepared = prepared.map_err(|error| error.to_string())?;
    let (first, last, holders) = (query.timeline.first, query.timeline.last, query.holders);
    Ok(occurrences::occurrence_panel(source.tables(), finish, move |hour, cancelled| {
        check_cancel(cancelled)?;
        if hour < first {
            return Err(dashboard_core::invalid("The hour asked is before the branch"));
        }
        reply(&prepared, &holders, &params, hour.min(last))
    }))
}
