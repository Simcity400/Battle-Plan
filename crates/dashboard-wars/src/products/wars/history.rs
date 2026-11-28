use super::{WarTimeline, WarsReply, at, branch, catalogue_of, contributions::WarContributions, inputs::{self, Change, Input, Shown}, measures::MEASURES, native::pair, participation, walked};
use dashboard_base::{Source, products::{Country, LoadContext, Product, at_hour::AtHour, check_reply_budget, validate_country}, queries::country_ids::RunLinks, span::HourSpan};
use dashboard_query::{Query, cost, identity::{Holder, Holders}, series::MissingSpan, timeline::Stretch};
use replay_columnar::manifest::Status;
use replay_data::{rows::TableId, war_participation::{self as derive, DeliveryHistory, Factors, Scores}};
use replay_lanes::lanes::{self, Cell, History, Keys, Reading};
use serde::{Deserialize, Serialize};
use std::{collections::{BTreeMap, BTreeSet, HashMap, HashSet}, io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct WarContributionHistory;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WarContributionHistoryParams {
    pub branch_id: String,
    pub viewpoint: Option<Country>,
    pub war_id: String,
    pub through_hour: i32,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WarContributionHistoryReply {
    pub war: WarTimeline,
    pub measure_keys: [super::WarMeasureKey; MEASURES.len()],
    pub contributors: Vec<WarHistoryContributor>,
    pub sides: [WarHistorySide; 2],
    pub gaps: Vec<MissingSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(WarContributionHistoryReply);

type ChangeRuns = [Vec<(i32, String)>; MEASURES.len()];

#[derive(Clone, Debug, Default, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WarHistoryContributor {
    pub slot: u32,
    pub id: u32,
    pub membership: Vec<(i32, i32, u8, String)>,
    pub amounts: ChangeRuns,
    pub partial_casualties: Vec<HourSpan>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WarHistorySide {
    pub totals: ChangeRuns,
    pub missing: Vec<MissingSpan>,
}

impl Product for WarContributionHistory {
    const KEY: &'static str = "war-contribution-history";
    const CONTENT_DEPENDENT: bool = true;
    const PREPARE: Option<dashboard_base::products::read_ahead::Prepare<Self::Params>> = Some(|source, params, context|
        super::prepare(source, branch(&params.branch_id)?, params.viewpoint.clone(), context));
    const INTERACTIVE: bool = true;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, viewpoint, warId, throughHour: End];
    type Params = WarContributionHistoryParams;
    type Reply = WarContributionHistoryReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![WarContributionHistoryParams::decl(), WarHistoryContributor::decl(), WarHistorySide::decl(), WarContributionHistoryReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.viewpoint.as_ref().map_or(Ok(()), validate_country)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = branch(&params.branch_id)?;
        let cat = catalogue_of(source, context)?;
        let factors = participation::factors(cat.as_deref());
        let wars = walked(source, branch, params.viewpoint.as_ref(), context, cancelled)?;
        let war = wars.wars.iter().find(|war| war.id == params.war_id && war.first_hour <= params.through_hour)
            .ok_or_else(|| dashboard_core::invalid("The recorded wars do not include this war at the requested hour"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        read(source, &query, &wars, war, params.through_hour, factors, cancelled)
    }
}

type Identity = (u32, u32);
type Roster<'a> = [Vec<(&'a str, Identity)>; 2];

const LEDGER: TableId = TableId::WarParticipation;
const DELIVERY: TableId = TableId::WarLendLeaseHistories;

fn reason(history: &History, table: TableId, hour: i32, allow_truncated: bool) -> Option<String> {
    let at = history.statuses.partition_point(|span| span.last < hour);
    let status = history.statuses.get(at).filter(|span| span.first <= hour).map(|span| span.status);
    match status {
        Some(Status::Complete) => None,
        Some(Status::Truncated { .. }) if allow_truncated => None,
        Some(Status::Truncated { missing }) => Some(format!("{table:?}: {missing} rows not read")),
        Some(Status::Unavailable { reason }) => Some(history.reasons.get(&reason).map_or_else(|| format!("{table:?} unavailable"), |reason| format!("{table:?}: {reason}"))),
        _ => Some(format!("{table:?} not recorded")),
    }
}

fn endpoints(key: u128) -> (u32, u32) {
    ((key >> 32) as u32, key as u32)
}

fn ledger_keys(first: u32, second: u32) -> [u128; 2] {
    let key = u128::from(first) << 64 | u128::from(second) << 32;
    [key, key | 1]
}

fn ledger_value(cells: &[Cell]) -> Option<Scores> {
    use Cell::Number as N;
    let [N(equipment_damage), N(province_capture), N(air_damage), N(strategic_bombing), N(sunk_ships),
        N(sunk_convoys), N(casualties), N(unknown_casualties), N(lend_lease_sent), N(lend_lease_received)] = *cells else { return None };
    Some(Scores { equipment_damage, province_capture, air_damage, strategic_bombing, sunk_ships,
        sunk_convoys, casualties, unknown_casualties, lend_lease_sent, lend_lease_received, ..Scores::default() })
}

fn delivery_value(cells: &[Cell]) -> Option<DeliveryHistory> {
    use Cell::Number as N;
    let [N(industry_sent), N(fuel_sent), N(industry_received), N(fuel_received)] = *cells else { return None };
    Some(DeliveryHistory { industry_sent, fuel_sent, industry_received, fuel_received })
}

fn roster<'a>(tags: [&'a [String]; 2], holders: &Holders, hour: i32) -> Option<Roster<'a>> {
    let side = |tags: &'a [String]| tags.iter().map(|tag| match holders.at(tag.as_str(), hour)?.holder {
        Holder::Found { slot, id } => Some((tag.as_str(), (slot, id))),
        Holder::Unresolved => None,
    }).collect::<Option<_>>();
    Some([side(tags[0])?, side(tags[1])?])
}

fn gap(spans: &mut Vec<MissingSpan>, first_hour: i32, last_hour: i32, reason: &str) {
    match spans.last_mut() {
        Some(span) if span.last_hour.checked_add(1) == Some(first_hour) && span.reason == reason => span.last_hour = last_hour,
        _ => spans.push(MissingSpan { first_hour, last_hour, reason: reason.into() }),
    }
}

fn changes<T: Copy + ToString + PartialEq>(out: &mut ChangeRuns, held: &mut [T; MEASURES.len()], hour: i32, values: [T; MEASURES.len()]) {
    for ((run, last), value) in out.iter_mut().zip(held).zip(values) {
        if run.is_empty() || *last != value {
            run.push((hour, value.to_string()));
            *last = value;
        }
    }
}

#[derive(Default)]
struct Member {
    reply: WarHistoryContributor,
    last: [i64; MEASURES.len()],
}

impl Member {
    fn admit(&mut self, first: i32, last: i32, side: u8, tag: &str) {
        match self.reply.membership.last_mut() {
            Some(run) if run.1.checked_add(1) == Some(first) && (run.2, run.3.as_str()) == (side, tag) => run.1 = last,
            Some(run) if run.0 <= first && last <= run.1 && (run.2, run.3.as_str()) == (side, tag) => {}
            _ => self.reply.membership.push((first, last, side, tag.into())),
        }
    }
}

struct Reply {
    output: WarContributionHistoryReply,
    members: BTreeMap<Identity, Member>,
    totals: [[i128; MEASURES.len()]; 2],
}

impl Reply {
    fn append(&mut self, side: usize, members: &[(&str, Identity)], scores: &HashMap<u32, Inputs>, factors: Factors, first: i32, last: i32) {
        let score = |slot| scores.get(&slot).map_or_else(|| derive::breakdown([], [], factors), |input| {
            derive::breakdown(input.ledgers.iter().copied(), input.deliveries.iter().copied(), factors)
        });
        let scored: Vec<_> = members.iter().map(|&(_, (slot, _))| score(slot)).collect();
        let denominator = derive::share_denominator(scored.iter().map(|score| score.total(false)));
        let mut totals = [0_i128; MEASURES.len()];
        let mut seen = HashSet::new();
        for (&(_, identity), score) in members.iter().zip(scored) {
            if !seen.insert(identity) {
                continue;
            }
            let participation = derive::share_of(score.total(false), denominator);
            let values = [participation, score.equipment_damage, score.province_capture, score.air_damage, score.sunk_ships,
                score.casualties, score.strategic_bombing, score.sunk_convoys, score.lend_lease_sent, score.lend_lease_received];
            let member = self.members.get_mut(&identity).expect("the roster was admitted");
            changes(&mut member.reply.amounts, &mut member.last, first, values);
            if score.unknown_casualties != 0 {
                super::walk::span(&mut member.reply.partial_casualties, first, last);
            }
            for (total, value) in totals.iter_mut().zip(values) {
                *total += i128::from(value).abs();
            }
        }
        totals[0] = 100_000;
        changes(&mut self.output.sides[side].totals, &mut self.totals[side], first, totals);
    }
}

#[derive(Default)]
struct Inputs {
    ledgers: Vec<Scores>,
    deliveries: Vec<DeliveryHistory>,
}

fn score_inputs(ledger: &History, delivery: &History, relations: &BTreeSet<(u32, u32)>, links: &RunLinks, roster: &Roster<'_>, hour: i32) -> (HashMap<u32, Inputs>, [Option<String>; 2]) {
    let mut sides = HashMap::new();
    for (side, rows) in roster.iter().enumerate() {
        for &(_, (slot, _)) in rows {
            sides.entry(slot).or_insert((side, 0)).1 += 1;
        }
    }
    let slot = |raw| {
        let runs = links.slots.get(&raw)?;
        runs.get(runs.partition_point(|run| run.1 < hour)).filter(|run| run.0 <= hour)?.2.map(|slot| slot as u32)
    };
    let mut active = HashSet::new();
    let mut values = HashMap::<u32, Inputs>::new();
    let mut unavailable: [Option<String>; 2] = std::array::from_fn(|_| reason(ledger, LEDGER, hour, true).or_else(|| reason(delivery, DELIVERY, hour, false)));
    let mut seen = HashSet::new();
    for &(a, b) in relations {
        let (Some(first), Some(second)) = (slot(a), slot(b)) else { continue };
        let (Some(&(left, first_count)), Some(&(right, second_count))) = (sides.get(&first), sides.get(&second)) else { continue };
        if left == right {
            active.insert(pair(first, second));
            continue;
        }
        for (direction, (country, side, count)) in [(first, left, second_count), (second, right, first_count)].into_iter().enumerate() {
            let read = [ledger_keys(a, b)[direction], ledger_keys(b, a)[1 - direction]].map(|key| match ledger.reading(key, hour) {
                Reading::Cells(cells) => Some(ledger_value(cells)),
                _ => None,
            });
            match read {
                [Some(Some(score)), None] | [None, Some(Some(score))] => values.entry(country).or_default().ledgers.extend(std::iter::repeat_n(score, count)),
                [None, None] => { unavailable[side].get_or_insert_with(|| reason(ledger, LEDGER, hour, false).unwrap_or_else(|| "Directional participation ledger not recorded".into())); }
                [Some(_), Some(_)] => { unavailable[side].get_or_insert_with(|| "Ambiguous directional participation ledger".into()); }
                _ => { unavailable[side].get_or_insert_with(|| "Directional participation ledger has unread cells".into()); }
            }
        }
    }
    for &(key, _) in &delivery.keys {
        let Reading::Cells(cells) = delivery.reading(key, hour) else { continue };
        let (raw, target) = endpoints(key);
        let Some((country, &(side, _))) = slot(raw).and_then(|country| Some((country, sides.get(&country)?))) else { continue };
        let Some(target) = slot(target) else {
            unavailable[side].get_or_insert_with(|| "Delivery partner's recorded country link is unread".into());
            continue;
        };
        let Some(&(target_side, count)) = sides.get(&target) else { continue };
        if country == target || target_side != side || active.contains(&pair(country, target)) {
            continue;
        }
        match delivery_value(cells) {
            Some(value) if seen.insert((country, target)) => values.entry(country).or_default().deliveries.extend(std::iter::repeat_n(value, count)),
            Some(_) => { unavailable[side].get_or_insert_with(|| "Ambiguous participation delivery history".into()); }
            None => { unavailable[side].get_or_insert_with(|| "Participation delivery history has unread cells".into()); }
        }
    }
    (values, unavailable)
}

fn read_stretch(source: &Source, holders: &Holders, war: &WarTimeline, stretch: Stretch, factors: Result<Factors, &str>, reply: &mut Reply, cancelled: &AtomicBool) -> io::Result<()> {
    let observations = &war.observations;
    let tags: BTreeSet<_> = observations.iter().filter(|o| o.first_hour <= stretch.last && stretch.first <= o.last_hour)
        .flat_map(|o| o.viewpoint_side.iter().chain(&o.opposing_side)).cloned().collect();
    let mut hours = BTreeSet::from([stretch.first]);
    let mut add_span = |first: i32, last: i32| {
        hours.extend([first, last.saturating_add(1)].into_iter().filter(|&hour| stretch.first < hour && hour <= stretch.last));
    };
    for o in observations {
        add_span(o.first_hour, o.last_hour);
    }
    let mut slots = HashSet::new();
    for tag in tags {
        for run in holders.runs(tag.as_str()).iter().filter(|run| run.first <= stretch.last && stretch.first <= run.last) {
            add_span(run.first, run.last);
            if let Holder::Found { slot, .. } = run.holder {
                slots.insert(slot);
            }
        }
    }
    let links = dashboard_base::queries::links(source, stretch.run)?;
    let mut raws: BTreeSet<_> = links.slots.iter().filter_map(|(&raw, runs)| runs.iter().any(|&(first, last, slot)| {
        first <= stretch.last && stretch.first <= last && slot.is_some_and(|slot| slots.contains(&(slot as u32)))
    }).then_some(raw)).collect();
    let wars = super::cached_wars(source, stretch.run)?;
    let related = |change: &&Change| {
        let (a, b) = endpoints(change.key);
        change.input == Input::Relation && change.hour <= stretch.last && raws.contains(&a) && raws.contains(&b)
    };
    let relation_changes: Vec<_> = wars.changes.iter().filter(related).collect();
    let ledger_columns = WarContributions::columns(LEDGER).expect("the ledger uses projected columns");
    let delivery_columns = inputs::columns(DELIVERY, &["industry_sent", "fuel_sent", "industry_received", "fuel_received"])?;
    let histories = factors.map_err(str::to_owned).and_then(|factors| {
        let ledgers: Vec<_> = relation_changes.iter().filter(|change| matches!(change.shown, Shown::Row(_))).flat_map(|change| {
            let (a, b) = endpoints(change.key);
            ledger_keys(a, b).into_iter().chain(ledger_keys(b, a))
        }).collect();
        let ranges: Vec<_> = raws.iter().map(|&raw| {
            let first = u128::from(raw) << 32;
            (first, first | u128::from(u32::MAX))
        }).collect();
        let projected = |table, columns, keys| cost::storage(|| lanes::history(source.replay_ref().recording(), stretch.run, table, columns, keys, stretch.first, stretch.last));
        let ledger = projected(LEDGER, &ledger_columns, Keys::These(&ledgers));
        let delivery = projected(DELIVERY, &delivery_columns, Keys::Ranges(&ranges));
        ledger.map_err(|e| format!("{LEDGER:?}: {e}")).and_then(|ledger| {
            delivery.map(|delivery| (ledger, delivery, factors)).map_err(|e| format!("{DELIVERY:?}: {e}"))
        })
    });
    if let Ok((ledger, delivery, _)) = &histories {
        for history in [ledger, delivery] {
            for status in &history.statuses {
                add_span(status.first, status.last);
            }
        }
        raws.extend(delivery.keys.iter().map(|&(key, _)| endpoints(key).1));
    }
    for raw in raws {
        for &(first, last, _) in links.slots.get(&raw).into_iter().flatten() {
            add_span(first, last);
        }
    }
    if let Ok((ledger, delivery, _)) = &histories {
        hours.extend([ledger, delivery].into_iter().flat_map(|h| &h.keys).flat_map(|(_, steps)| steps.iter().map(|s| s.hour)));
    }
    hours.extend(relation_changes.iter().filter_map(|change| (stretch.first < change.hour).then_some(change.hour)));
    let mut changes = relation_changes.into_iter().peekable();
    let mut relations = BTreeSet::new();
    let mut hours = hours.into_iter().peekable();
    while let Some(hour) = hours.next() {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let last = hours.peek().map_or(stretch.last, |next| next - 1);
        while let Some(change) = changes.next_if(|change| change.hour <= hour) {
            let pair = endpoints(change.key);
            if matches!(change.shown, Shown::Row(_)) {
                relations.insert(pair);
            } else {
                relations.remove(&pair);
            }
        }
        let at = observations.partition_point(|o| o.last_hour < hour);
        let Some(observation) = observations.get(at).filter(|o| o.first_hour <= hour) else { continue };
        let Some(members) = roster([&observation.viewpoint_side, &observation.opposing_side], holders, hour) else {
            gap(&mut reply.output.gaps, hour, last, "War contributor identity not recorded");
            continue;
        };
        let first_side: HashSet<_> = members[0].iter().map(|&(_, identity)| identity).collect();
        if members[1].iter().any(|&(_, identity)| first_side.contains(&identity)) {
            gap(&mut reply.output.gaps, hour, last, "War contributor side is ambiguous");
            continue;
        }
        for (side, members) in members.iter().enumerate() {
            for &(tag, (slot, id)) in members {
                reply.members.entry((slot, id)).or_insert_with(|| Member { reply: WarHistoryContributor { slot, id, ..Default::default() }, ..Default::default() })
                    .admit(hour, last, side as u8, tag);
            }
        }
        let roster = match &observation.contribution_group {
            Some(sides) => roster([&sides[0], &sides[1]], holders, hour).expect("the group is a subset of the resolved roster"),
            None => members,
        };
        match &histories {
            Ok((ledger, delivery, factors)) => {
                let (values, missing) = score_inputs(ledger, delivery, &relations, &links, &roster, hour);
                for (side, (members, missing)) in roster.iter().zip(missing).enumerate() {
                    if let Some(reason) = missing {
                        gap(&mut reply.output.sides[side].missing, hour, last, &reason);
                    } else {
                        reply.append(side, members, &values, *factors, hour, last);
                    }
                }
            }
            Err(reason) => {
                for side in &mut reply.output.sides {
                    gap(&mut side.missing, hour, last, reason);
                }
            }
        }
    }
    Ok(())
}

fn unread_spans(query: &Query<'_>, wars: &WarsReply, first: i32, last: i32, through: i32) -> Vec<MissingSpan> {
    let mut gaps = Vec::new();
    for span in wars.unobserved.iter().take_while(|span| span.first_hour <= last.saturating_add(1)) {
        let (start, end) = (span.first_hour.max(first), span.last_hour.min(through));
        if start > end {
            continue;
        }
        let pauses = query.timeline.unheld_within(start, end);
        let partials: Vec<_> = wars.partial_relations.iter().copied().filter(|&(a, b)| a <= end && start <= b).collect();
        let mut edges = BTreeSet::from([start]);
        for (a, b) in pauses.iter().map(|gap| (gap.first, gap.last)).chain(partials.iter().copied()) {
            edges.insert(a.max(start));
            if b < end {
                edges.insert(b + 1);
            }
        }
        let mut edges = edges.into_iter().peekable();
        while let Some(hour) = edges.next() {
            let reason = match pauses.iter().find(|gap| gap.first <= hour && hour <= gap.last) {
                Some(pause) => pause.reason.as_str(),
                None if partials.iter().any(|&(a, b)| a <= hour && hour <= b) => "War membership is partial because a recorded relation table was truncated",
                None => "War membership not recorded",
            };
            gaps.push(MissingSpan { first_hour: hour, last_hour: edges.peek().map_or(end, |next| next - 1), reason: reason.into() });
        }
    }
    gaps
}

pub(super) fn read(source: &Source, query: &Query<'_>, wars: &WarsReply, war: &WarTimeline, hour: i32, factors: Result<Factors, String>, cancelled: &AtomicBool) -> io::Result<WarContributionHistoryReply> {
    let cut = at::cut_war(war, hour);
    let (first, last) = (cut.first_hour, cut.last_hour);
    let mut reply = Reply { output: WarContributionHistoryReply {
        war: cut, measure_keys: MEASURES, contributors: Vec::new(), sides: Default::default(), gaps: Vec::new(), warnings: Vec::new(),
    }, members: BTreeMap::new(), totals: Default::default() };
    for stretch in query.timeline.within(first, last) {
        read_stretch(source, &query.holders, war, stretch, factors.as_ref().copied().map_err(String::as_str), &mut reply, cancelled)?;
    }
    reply.output.gaps.extend(unread_spans(query, wars, first, last, if war.ended_by { hour } else { last }));
    reply.output.gaps.sort_by_key(|gap| gap.first_hour);
    reply.output.contributors = reply.members.into_values().map(|member| member.reply).collect();
    check_reply_budget(&reply.output)?;
    Ok(reply.output)
}

#[cfg(test)]
pub(super) use tests::assert_snapshot;
#[cfg(test)]
mod tests;
