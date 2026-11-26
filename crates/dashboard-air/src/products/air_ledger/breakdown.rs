use super::{Fold, LeafKey, MilitaryAirFamily, building_name, scan};
use dashboard_base::{
    Source, catalogue,
    products::{
        Country, LoadContext, Product,
        breakdown::{self, Amount, Breakdown, Exact, Leaf, Node, Unrecorded},
        check_reply_budget,
    },
    span::HourSpan,
};
use dashboard_query::{
    Query,
    identity::{Holder, Holders, TAG_UNLINKED},
};
use dashboard_text::scripted_text::{self, StateNameUsage};
use replay_columnar::manifest::Status;
use replay_data::rows::{StateRow, TableId};
use replay_lanes::lanes::{self, Cell, History, Keys, StatusRun};
use replay_recording::columnar_frames::available;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct BombingDamageBreakdown;

const AMOUNT_SCALE: i128 = 10_000_000;
const NOT_OBSERVED: &str = "Bombing damage was not recorded at this hour";
const UNJOINED: &str = "Country links were not recorded at this hour; bombing cannot be credited to a country";
const NOT_PRESENT: &str = "This country is not present at this hour";
const HOLDER_UNRESOLVED: &str = "This country's holder was not recorded at this hour";
const GAP_BEFORE: &str = "Bombing damage or country links had an unread gap before this hour";
const INCOMPLETE_READ: &str = "Some bombing damage reads were incomplete or named a country the recording cannot link";
const UNSUPPORTED_CAUSE: &str = "Some of this country's building damage has an unsupported cause; whether it was aircraft bombing is not known";
const UNATTRIBUTED_TAKEN: &str = "Some bombing damage had no resolved victim";
const UNATTRIBUTED_DEALT: &str = "Some bombing damage had no recorded attacker";
const CONTENT_UNAVAILABLE: &str = "Recorded content stack unavailable; choose matching game and user folders";
const BUILDING_UNSPELLED: &str = "Building type is not spelled by the recorded lexical dictionary";
const STATES_NOT_RECORDED: &str = "States table was not recorded at this hour";
const STATES_IN_PART: &str = "States were read only in part at this hour; this state's row was not among them";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum BombingDirection {
    Taken,
    Dealt,
}

impl BombingDirection {
    fn dealt(self) -> bool {
        matches!(self, Self::Dealt)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BombingDamageBreakdownParams {
    pub branch_id: String,
    pub country: Country,
    pub hour: i32,
    pub direction: BombingDirection,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BombingDamageBreakdownReply {
    pub from: i32,
    pub hour: i32,
    pub through: i32,
    pub direction: BombingDirection,
    pub breakdown: Breakdown,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(BombingDamageBreakdownReply);

#[derive(Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    branch: u64,
    country: Country,
    direction: BombingDirection,
    first: i32,
    last: i32,
}

#[derive(Clone)]
struct HolderSpan {
    first: i32,
    last: i32,
    holder: Option<String>,
}

struct PartSeries {
    building_id: u32,
    building_token: Option<String>,
    state: u32,
    events: Vec<(i32, i128)>,
}

struct StateStretch {
    first: i32,
    last: i32,
    history: History,
    by_state: HashMap<u32, usize>,
}

struct SharedFold {
    fold: Fold,
    by_holder: HashMap<(bool, String), Vec<LeafKey>>,
    unattributed_first: [Option<i32>; 2],
}

impl SharedFold {
    fn new(fold: Fold) -> Self {
        let mut by_holder: HashMap<(bool, String), Vec<LeafKey>> = HashMap::new();
        let mut unattributed_first = [None, None];
        for (leaf, deltas) in &fold.leaves {
            match &leaf.country {
                Some((_, identity)) => by_holder.entry((leaf.dealt, identity.clone())).or_default().push(leaf.clone()),
                None => {
                    if let Some(&(read_hour, _)) = deltas.keys().next() {
                        let held: &mut Option<i32> = &mut unattributed_first[usize::from(leaf.dealt)];
                        *held = Some(held.map_or(read_hour, |held| held.min(read_hour)));
                    }
                }
            }
        }
        Self { fold, by_holder, unattributed_first }
    }
}

#[derive(PartialEq)]
struct FoldKey {
    branch: u64,
    first: i32,
    last: i32,
}

struct BombingIndex {
    last: i32,
    from: i32,
    stretches: Vec<(i32, i32)>,
    unknown_holders: Vec<(i32, i32, String)>,
    unlinked: Vec<(i32, i32)>,
    holder_runs: Vec<HolderSpan>,
    observed: Vec<HourSpan>,
    incomplete: Vec<HourSpan>,
    unsupported: Vec<(i32, Option<String>, Option<String>)>,
    unread: Vec<(i32, i32, String)>,
    unjoined: Vec<HourSpan>,
    unattributed_first: Option<i32>,
    by_holder: HashMap<String, Vec<PartSeries>>,
    state_stretches: Vec<StateStretch>,
    catalogue: Result<Arc<catalogue::Catalogue>, String>,
    buildings: HashMap<String, Result<String, String>>,
    warnings: Vec<String>,
}

fn unlinked_spans(holders: &Holders) -> Vec<(i32, i32)> {
    let mut spans: Vec<(i32, i32)> = holders.tags.values().flatten().filter(|run| run.holder == Holder::Unresolved).map(|run| (run.first, run.last)).collect();
    spans.sort_unstable();
    let mut out: Vec<(i32, i32)> = Vec::with_capacity(spans.len());
    for (first, last) in spans {
        match out.last_mut() {
            Some(held) if i64::from(first) <= i64::from(held.1) + 1 => held.1 = held.1.max(last),
            _ => out.push((first, last)),
        }
    }
    out
}

fn slot_runs(holders: &Holders, slot: u32) -> Vec<HolderSpan> {
    let mut spans: Vec<HolderSpan> = holders
        .tags
        .values()
        .flatten()
        .filter_map(|run| match run.holder {
            Holder::Found { slot: found, id } if found == slot => Some(HolderSpan { first: run.first, last: run.last, holder: Some(format!("{found}:{id}")) }),
            _ => None,
        })
        .collect();
    spans.sort_by_key(|span| (span.first, span.last));
    let mut out: Vec<HolderSpan> = Vec::with_capacity(spans.len());
    for span in spans {
        match out.last_mut() {
            Some(held) if held.holder == span.holder && i64::from(span.first) <= i64::from(held.last) + 1 => {
                held.last = held.last.max(span.last);
            }
            _ => out.push(span),
        }
    }
    out
}

fn tag_runs(holders: &Holders, tag: &str) -> Vec<HolderSpan> {
    holders
        .tags
        .get(tag)
        .map(|runs| {
            runs.iter()
                .map(|run| HolderSpan {
                    first: run.first,
                    last: run.last,
                    holder: match run.holder {
                        Holder::Found { slot, id } => Some(format!("{slot}:{id}")),
                        Holder::Unresolved => None,
                    },
                })
                .collect()
        })
        .unwrap_or_default()
}

fn covering<'a, T>(runs: &'a [T], hour: i32, bounds: impl Fn(&T) -> (i32, i32)) -> Option<&'a T> {
    let at = runs.partition_point(|run| bounds(run).1 < hour);
    runs.get(at).filter(|run| bounds(run).0 <= hour)
}

fn next_span_edge<T>(runs: &[T], hour: i32, bounds: impl Fn(&T) -> (i32, i32)) -> Option<i32> {
    let at = runs.partition_point(|run| bounds(run).1 < hour);
    let run = runs.get(at)?;
    let (first, last) = bounds(run);
    if first <= hour { last.checked_add(1).filter(|&edge| edge > hour) } else { Some(first) }
}

fn tighten(through: &mut i32, edge: Option<i32>) {
    if let Some(edge) = edge
        && let Some(until) = edge.checked_sub(1)
    {
        *through = (*through).min(until);
    }
}

fn not_recorded(reason: impl Into<String>) -> Breakdown {
    breakdown::breakdown(&[Leaf { path: Vec::new(), amount: Amount::NotRecorded(reason.into()), count: None, last: None }], &Unrecorded::default())
}

fn exact_damage(raw: i128) -> Amount {
    match Exact::new(raw, AMOUNT_SCALE) {
        Some(value) => Amount::Known(value),
        None => Amount::NotRecorded("Bombing damage could not be represented exactly".into()),
    }
}

fn status_at(statuses: &[StatusRun], hour: i32) -> (Status, Option<i32>) {
    match covering(statuses, hour, |run| (run.first, run.last)) {
        Some(run) => (run.status, run.last.checked_add(1)),
        None => (Status::NotRecorded, next_span_edge(statuses, hour, |run| (run.first, run.last))),
    }
}

fn text_bytes(history: &History, cell: Option<&Cell>) -> Vec<u8> {
    match cell {
        Some(Cell::Text(id)) => history.texts.get(*id as usize).map(|bytes| bytes.to_vec()).unwrap_or_default(),
        _ => Vec::new(),
    }
}

impl BombingIndex {
    fn build(source: &Source, query: &Query<'_>, branch: u64, country: &Country, direction: BombingDirection, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self> {
        let (first, last) = (query.timeline.first, query.timeline.last);
        let dealt = direction.dealt();
        let shared = breakdown::held(source, context, FoldKey { branch, first, last }, || scan(source, query, MilitaryAirFamily::Bombing, (first, last), cancelled).map(SharedFold::new))?;
        let fold = &shared.fold;
        let from = fold.observed.first().map_or(first, |span| span.first_hour);
        let holder_runs = match country {
            Country::Slot(slot) => slot_runs(&query.holders, *slot),
            Country::Tag(tag) => tag_runs(&query.holders, tag),
        };
        let wanted_holders: BTreeSet<String> = holder_runs.iter().filter_map(|run| run.holder.clone()).collect();
        let unattributed_first = shared.unattributed_first[usize::from(dealt)];
        let mut grouped: HashMap<String, BTreeMap<(u32, Option<String>, u32), BTreeMap<i32, i128>>> = HashMap::new();
        let mut buildings_needed: BTreeSet<String> = BTreeSet::new();
        let mut states_needed: BTreeSet<u32> = BTreeSet::new();
        let wanted = wanted_holders.iter().flat_map(|identity| shared.by_holder.get(&(dealt, identity.clone())).into_iter().flatten());
        for leaf in wanted {
            let Some(deltas) = fold.leaves.get(leaf) else { continue };
            let Some((_, identity)) = leaf.country.as_ref() else { continue };
            let Some((building_id, building_token)) = leaf.building.as_ref() else { continue };
            let Some(state) = leaf.state else { continue };
            if let Some(token) = building_token {
                buildings_needed.insert(token.clone());
            }
            if state != 0 {
                states_needed.insert(state);
            }
            let by_read = grouped.entry(identity.clone()).or_default().entry((*building_id, building_token.clone(), state)).or_default();
            for (&(read_hour, _), &amount) in deltas {
                let entry = by_read.entry(read_hour).or_default();
                *entry = entry.checked_add(amount).ok_or_else(|| dashboard_core::invalid("bombing breakdown sum overflow"))?;
            }
        }
        let mut by_holder: HashMap<String, Vec<PartSeries>> = HashMap::with_capacity(grouped.len());
        for (identity, parts) in grouped {
            let mut series = Vec::with_capacity(parts.len());
            for ((building_id, building_token, state), by_read) in parts {
                let mut total = 0_i128;
                let mut events = Vec::with_capacity(by_read.len());
                for (read_hour, delta) in by_read {
                    if delta == 0 {
                        continue;
                    }
                    total = total.checked_add(delta).ok_or_else(|| dashboard_core::invalid("bombing breakdown running sum overflow"))?;
                    events.push((read_hour, total));
                }
                if !events.is_empty() {
                    series.push(PartSeries { building_id, building_token, state, events });
                }
            }
            by_holder.insert(identity, series);
        }
        let state_stretches = if states_needed.is_empty() {
            Vec::new()
        } else {
            let custom_col = dashboard_declare::field(TableId::States, "custom_name")?;
            let owner_col = dashboard_declare::field(TableId::States, "owner_name")?;
            let keys: Vec<u128> = states_needed.iter().map(|&id| u128::from(id)).collect();
            let recording = query.replay.recording();
            let mut stretches = Vec::with_capacity(query.timeline.stretches.len());
            for stretch in &query.timeline.stretches {
                dashboard_text::metadata::check_cancel(cancelled)?;
                let history = lanes::history(recording, stretch.run, TableId::States, &[custom_col, owner_col], Keys::These(&keys), stretch.first, stretch.last)?;
                let by_state = history.keys.iter().enumerate().map(|(index, (key, _))| (*key as u32, index)).collect();
                stretches.push(StateStretch { first: stretch.first, last: stretch.last, history, by_state });
            }
            stretches
        };
        let mut warnings = Vec::new();
        let mut buildings = HashMap::new();
        let cat = context.content().and_then(|options| catalogue::catalogue(source, options));
        let catalogue = match cat {
            Ok(cat) => {
                warnings.extend(cat.verification.iter().cloned());
                warnings.extend(cat.warnings.iter().cloned());
                if !buildings_needed.is_empty() {
                    let content = cat
                        .content
                        .layers
                        .derived::<catalogue::CachedResult<content_reader_key_names::buildings::Buildings>>()
                        .get_or_init(|| content_reader_key_names::buildings::read(&cat.content.layers))
                        .map_err(|error| error.to_string());
                    for token in buildings_needed {
                        let resolved = content.as_ref().map_err(Clone::clone).and_then(|db| building_name(&cat, db, &token));
                        buildings.insert(token, resolved);
                    }
                }
                Ok(cat)
            }
            Err(_) => {
                warnings.push(CONTENT_UNAVAILABLE.to_owned());
                Err(CONTENT_UNAVAILABLE.to_owned())
            }
        };
        Ok(Self {
            last,
            from,
            stretches: query.timeline.stretches.iter().map(|stretch| (stretch.first, stretch.last)).collect(),
            unknown_holders: query.holders.unknown.clone(),
            unlinked: unlinked_spans(&query.holders),
            holder_runs,
            observed: fold.observed.clone(),
            incomplete: fold.in_part.clone(),
            unsupported: fold.unsupported.clone(),
            unread: fold.unread.clone(),
            unjoined: fold.unjoined.clone(),
            unattributed_first,
            by_holder,
            state_stretches,
            catalogue,
            buildings,
            warnings,
        })
    }

    fn building_node(&self, building_id: u32, building_token: Option<&str>) -> Node {
        let key = match building_token {
            Some(token) => format!("building:{token}"),
            None => format!("building:{building_id}"),
        };
        let Some(token) = building_token else {
            return Node::new(key, format!("Building {building_id}"), building_id).name_missing(Some(BUILDING_UNSPELLED.to_owned()));
        };
        if let Err(reason) = &self.catalogue {
            return Node::new(key, token, building_id).name_missing(Some(reason.clone()));
        }
        match self.buildings.get(token) {
            Some(Ok(name)) => Node::new(key, name, building_id),
            Some(Err(reason)) => Node::new(key, token, building_id).name_missing(Some(reason.clone())),
            None => Node::new(key, token, building_id).name_missing(Some(format!("Building {token} absent from the recorded stack"))),
        }
    }

    fn state_node(&self, state: u32, hour: i32, through: &mut i32) -> Node {
        if state == 0 {
            return Node::new("state:0", "State unknown", 0).other();
        }
        let key = format!("state:{state}");
        let fallback = format!("State {state}");
        let Ok(cat) = &self.catalogue else {
            return Node::new(key, fallback, state).name_missing(Some(CONTENT_UNAVAILABLE.to_owned()));
        };
        let Some(stretch) = covering(&self.state_stretches, hour, |stretch| (stretch.first, stretch.last)) else {
            return Node::new(key, fallback, state).name_missing(Some(STATES_NOT_RECORDED.to_owned()));
        };
        let (status, edge) = status_at(&stretch.history.statuses, hour);
        tighten(through, edge);
        if !available(status) {
            return Node::new(key, fallback, state).name_missing(Some(STATES_NOT_RECORDED.to_owned()));
        }
        let absent = || match status {
            Status::Truncated { .. } => STATES_IN_PART.to_owned(),
            _ => format!("Not in this recording: state {state} at this hour"),
        };
        let Some(&entry_index) = stretch.by_state.get(&state) else {
            return Node::new(key, fallback, state).name_missing(Some(absent()));
        };
        let steps = &stretch.history.keys[entry_index].1;
        let at = steps.partition_point(|step| step.hour <= hour);
        if let Some(next_step) = steps.get(at) {
            tighten(through, Some(next_step.hour));
        }
        let Some(cells) = at.checked_sub(1).and_then(|index| steps.get(index)).and_then(|step| step.cells.as_deref()) else {
            return Node::new(key, fallback, state).name_missing(Some(absent()));
        };
        let (custom_name, owner_name) = (text_bytes(&stretch.history, cells.first()), text_bytes(&stretch.history, cells.get(1)));
        let row = StateRow { id: state, custom_name, owner_name, ..StateRow::default() };
        let states = cat.cached_states();
        let names = states.as_ref().map(|states| &states.names).map_err(String::as_str);
        match scripted_text::state_display_name(&row, cat, names, StateNameUsage::Place) {
            Ok(name) => Node::new(key, name, state),
            Err(reason) => Node::new(key, fallback, state).name_missing(Some(reason)),
        }
    }

    fn at(&self, country: &Country, direction: BombingDirection, hour: i32) -> BombingDamageBreakdownReply {
        let mut through = self.last;
        let reply = |breakdown: Breakdown, through: i32| BombingDamageBreakdownReply {
            from: self.from,
            hour,
            through: through.max(hour),
            direction,
            breakdown,
            warnings: self.warnings.clone(),
        };
        if let Some(stretch) = covering(&self.stretches, hour, |&(first, last)| (first, last)) {
            through = through.min(stretch.1);
        } else {
            tighten(&mut through, next_span_edge(&self.stretches, hour, |&(first, last)| (first, last)));
            return reply(not_recorded(NOT_OBSERVED), through);
        }
        if let Some(unknown) = covering(&self.unknown_holders, hour, |&(first, last, _)| (first, last)) {
            through = through.min(unknown.1);
            return reply(not_recorded(unknown.2.clone()), through);
        }
        tighten(&mut through, next_span_edge(&self.unknown_holders, hour, |&(first, last, _)| (first, last)));

        let holder = match covering(&self.holder_runs, hour, |run| (run.first, run.last)) {
            Some(run) => {
                through = through.min(run.last);
                match &run.holder {
                    Some(identity) => identity.as_str(),
                    None => return reply(not_recorded(HOLDER_UNRESOLVED), through),
                }
            }
            None => {
                tighten(&mut through, next_span_edge(&self.holder_runs, hour, |run| (run.first, run.last)));
                if matches!(country, Country::Slot(_)) {
                    if let Some(span) = covering(&self.unlinked, hour, |&(first, last)| (first, last)) {
                        through = through.min(span.1);
                        return reply(not_recorded(TAG_UNLINKED), through);
                    }
                    tighten(&mut through, next_span_edge(&self.unlinked, hour, |&(first, last)| (first, last)));
                }
                return reply(not_recorded(NOT_PRESENT), through);
            }
        };

        let Some(observed_span) = covering(&self.observed, hour, |span| (span.first_hour, span.last_hour)) else {
            tighten(&mut through, next_span_edge(&self.observed, hour, |span| (span.first_hour, span.last_hour)));
            if let Some(span) = covering(&self.unjoined, hour, |span| (span.first_hour, span.last_hour)) {
                through = through.min(span.last_hour);
                return reply(not_recorded(UNJOINED), through);
            }
            tighten(&mut through, next_span_edge(&self.unjoined, hour, |span| (span.first_hour, span.last_hour)));
            if let Some(&(_, last, ref reason)) = covering(&self.unread, hour, |&(first, last, _)| (first, last)) {
                return reply(not_recorded(reason.clone()), through.min(last));
            }
            tighten(&mut through, next_span_edge(&self.unread, hour, |&(first, last, _)| (first, last)));
            return reply(not_recorded(NOT_OBSERVED), through);
        };
        through = through.min(observed_span.last_hour);

        let mut partial_reasons: Vec<&'static str> = Vec::new();
        if observed_span.first_hour > self.from {
            partial_reasons.push(GAP_BEFORE);
        }
        let has_incomplete = self.incomplete.iter().any(|span| span.first_hour <= hour && span.last_hour >= self.from);
        if has_incomplete {
            partial_reasons.push(INCOMPLETE_READ);
        } else if let Some(next_incomplete) = self.incomplete.iter().find(|span| span.first_hour > hour) {
            tighten(&mut through, Some(next_incomplete.first_hour));
        }
        let touches = |&&(_, ref attacker, ref victim): &&(i32, Option<String>, Option<String>)| {
            let end = if direction.dealt() { attacker } else { victim };
            end.as_deref().is_none_or(|end| end == holder)
        };
        if self.unsupported.iter().filter(touches).any(|&(read, _, _)| read <= hour) {
            partial_reasons.push(UNSUPPORTED_CAUSE);
        } else if let Some(&(read, _, _)) = self.unsupported.iter().filter(touches).find(|&&(read, _, _)| read > hour) {
            tighten(&mut through, Some(read));
        }
        if let Some(first_unattributed) = self.unattributed_first {
            if first_unattributed <= hour {
                partial_reasons.push(if direction.dealt() { UNATTRIBUTED_DEALT } else { UNATTRIBUTED_TAKEN });
            } else {
                tighten(&mut through, Some(first_unattributed));
            }
        }
        let partial_reason = (!partial_reasons.is_empty()).then(|| partial_reasons.join("; "));

        let mut leaves: Vec<Leaf> = Vec::new();
        if let Some(parts) = self.by_holder.get(holder) {
            for part in parts {
                let at = part.events.partition_point(|&(read_hour, _)| read_hour <= hour);
                if let Some(&(next_read, _)) = part.events.get(at) {
                    tighten(&mut through, Some(next_read));
                }
                let Some(&(_, total)) = at.checked_sub(1).and_then(|index| part.events.get(index)) else {
                    continue;
                };
                if total == 0 {
                    continue;
                }
                let amount = match (exact_damage(total), &partial_reason) {
                    (Amount::Known(exact), Some(reason)) => Amount::Partial(exact, reason.clone()),
                    (other, _) => other,
                };
                let building_node = self.building_node(part.building_id, part.building_token.as_deref());
                let state_node = self.state_node(part.state, hour, &mut through);
                leaves.push(Leaf { path: vec![building_node, state_node], amount, count: None, last: None });
            }
        }
        let breakdown = if leaves.is_empty() {
            match partial_reason {
                Some(reason) => not_recorded(reason),
                None => breakdown::breakdown(&[], &Unrecorded::default()),
            }
        } else {
            breakdown::breakdown(&leaves, &Unrecorded::default())
        };
        reply(breakdown, through)
    }
}

impl Product for BombingDamageBreakdown {
    const KEY: &'static str = "bombing-damage-breakdown";
    const CONTENT_DEPENDENT: bool = true;
    type Params = BombingDamageBreakdownParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, hour: End, direction];
    type Reply = BombingDamageBreakdownReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        if matches!(&params.country, Country::Tag(tag) if tag.is_empty()) {
            return Err("Country tag is empty");
        }
        Ok(())
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![BombingDirection::decl(), BombingDamageBreakdownParams::decl(), BombingDamageBreakdownReply::decl()]
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        if params.hour < query.timeline.first || params.hour > query.timeline.last {
            return Err(dashboard_core::invalid("Requested hour is outside the branch"));
        }
        let key = CacheKey {
            branch,
            country: params.country.clone(),
            direction: params.direction,
            first: query.timeline.first,
            last: query.timeline.last,
        };
        let index = breakdown::held(source, context, key, || BombingIndex::build(source, &query, branch, &params.country, params.direction, context, cancelled))?;
        let reply = index.at(&params.country, params.direction, params.hour);
        check_reply_budget(&reply).map_err(|_| dashboard_core::invalid("Bombing damage breakdown reply exceeds transport budget; nothing was truncated"))?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::{
        products::breakdown::BreakdownState,
        test_fixtures::{CAMPAIGN, records, source},
    };
    use replay::campaign::Replay;
    use replay_data::rows::{BuildingBombingDamageCause, BuildingBombingDamageRow, CountryLinkRow, CountryRow, LexicalTokenRow, StateRow, TableRows, TableSchema, tables};
    use replay_frame::frame::{DecodedFrame, TableView};
    use std::sync::Arc;

    fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
        *frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
        frame.metrics.table_ns[T::ID.index()] = 1;
    }

    fn bombing(id: u64, game_hour: i32, attacker: Option<u32>, victim: u32, state: u32, building_type: u32, cause: BuildingBombingDamageCause, amount_raw: i64) -> BuildingBombingDamageRow {
        BuildingBombingDamageRow { id, game_hour, attacker, victim, state, province: None, building_type, native_cause: 0, cause, amount_raw, credited_raw: amount_raw }
    }

    fn make_source(frames: Vec<DecodedFrame>) -> Source {
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        Source::from_replay(dashboard_base::SourceKey { source_id: "bombing-breakdown".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
    }

    #[test]
    fn bombing_breakdown_tracks_slot_across_tag_change_with_exact_fractions_and_quiet_through() {
        let frames: Vec<DecodedFrame> = (100..=130)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                let TableView::Complete(TableRows::Countries(countries)) = &*frame.tables[TableId::Countries.index()] else { unreachable!() };
                let mut countries = countries.clone();
                if hour >= 115 {
                    countries[0].tag = b"BBB".to_vec();
                }
                countries.push(CountryRow { id: 9002, canonical_slot: 8, tag: b"ITA".to_vec(), ..countries[0].clone() });
                set::<tables::Countries>(&mut frame, countries);
                set::<tables::CountryLinks>(&mut frame, vec![CountryLinkRow { id: 9001, canonical_slot: 7 }, CountryLinkRow { id: 9002, canonical_slot: 8 }]);
                set::<tables::LexicalTokens>(&mut frame, vec![LexicalTokenRow { token: 9, literal: b"air_base".to_vec() }, LexicalTokenRow { token: 10, literal: b"infrastructure".to_vec() }]);
                set::<tables::States>(&mut frame, vec![StateRow { id: 42, custom_name: if hour >= 120 { b"Renamed State".to_vec() } else { b"Old State".to_vec() }, ..StateRow::default() }]);
                let mut rows = Vec::new();
                if hour == 105 {
                    rows.push(bombing(1, 105, Some(9002), 9001, 42, 9, BuildingBombingDamageCause::AirMission, 25_000_000));
                    rows.push(bombing(2, 105, Some(9002), 9001, 42, 9, BuildingBombingDamageCause::LandCombat, 90_000_000));
                }
                if hour == 118 {
                    rows.push(bombing(3, 118, Some(9002), 9001, 0, 10, BuildingBombingDamageCause::AirNaval, 1_000));
                }
                set::<tables::BuildingBombingDamage>(&mut frame, rows);
                frame
            })
            .collect();
        let src = make_source(frames);
        let ctx = LoadContext { content: None };
        let cancel = AtomicBool::new(false);

        let before = BombingDamageBreakdown::load(&src, &BombingDamageBreakdownParams { branch_id: "1".into(), country: Country::Slot(7), hour: 102, direction: BombingDirection::Taken }, &ctx, &cancel).unwrap();
        assert_eq!(before.from, 100);
        assert_eq!(before.through, 104);
        assert_eq!(before.breakdown.total.state, BreakdownState::Known);
        assert_eq!(before.breakdown.total.amount.as_ref().unwrap().numerator, "0");
        assert!(before.breakdown.parts.is_empty());

        let first_hit = BombingDamageBreakdown::load(&src, &BombingDamageBreakdownParams { branch_id: "1".into(), country: Country::Slot(7), hour: 106, direction: BombingDirection::Taken }, &ctx, &cancel).unwrap();
        assert_eq!(first_hit.through, 117);
        assert_eq!(first_hit.breakdown.total.state, BreakdownState::Known);
        assert_eq!(first_hit.breakdown.total.amount.as_ref().unwrap().numerator, "5");
        assert_eq!(first_hit.breakdown.total.amount.as_ref().unwrap().denominator, "2");
        assert_eq!(first_hit.breakdown.parts.len(), 1);
        assert_eq!(first_hit.breakdown.parts[0].key, "building:air_base");
        assert_eq!(first_hit.breakdown.parts[0].children[0].key, "state:42");

        let after_rename = BombingDamageBreakdown::load(&src, &BombingDamageBreakdownParams { branch_id: "1".into(), country: Country::Slot(7), hour: 122, direction: BombingDirection::Taken }, &ctx, &cancel).unwrap();
        assert_eq!(after_rename.through, 130);
        assert_eq!(after_rename.breakdown.total.state, BreakdownState::Known);
        assert_eq!(after_rename.breakdown.total.amount.as_ref().unwrap().numerator, "25001");
        assert_eq!(after_rename.breakdown.total.amount.as_ref().unwrap().denominator, "10000");
        assert_eq!(after_rename.breakdown.parts.len(), 2);
        let sliver = after_rename.breakdown.parts.iter().find(|part| part.key == "building:infrastructure").unwrap();
        assert_eq!(sliver.amount.as_ref().unwrap().numerator, "1");
        assert_eq!(sliver.amount.as_ref().unwrap().denominator, "10000");
        assert_eq!(sliver.children[0].key, "state:0");
        assert!(sliver.children[0].other);
        assert_eq!(sliver.children[0].name, "State unknown");

        let dealt = BombingDamageBreakdown::load(&src, &BombingDamageBreakdownParams { branch_id: "1".into(), country: Country::Slot(8), hour: 122, direction: BombingDirection::Dealt }, &ctx, &cancel).unwrap();
        assert_eq!(dealt.breakdown.total.state, BreakdownState::Known);
        assert_eq!(dealt.breakdown.total.amount.as_ref().unwrap().numerator, "25001");
        assert_eq!(dealt.breakdown.total.amount.as_ref().unwrap().denominator, "10000");
    }

    #[test]
    fn bombing_breakdown_withholds_shares_on_unknown_cause_gap_unattributed_and_unlinked_slot() {
        let frames: Vec<DecodedFrame> = (200..=215)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                let TableView::Complete(TableRows::Countries(countries)) = &*frame.tables[TableId::Countries.index()] else { unreachable!() };
                let mut countries = countries.clone();
                countries.push(CountryRow { id: 9002, canonical_slot: 8, tag: b"ITA".to_vec(), ..countries[0].clone() });
                countries.push(CountryRow { id: 9003, canonical_slot: 9, tag: b"HUN".to_vec(), ..countries[0].clone() });
                set::<tables::Countries>(&mut frame, countries);
                if hour == 203 {
                    *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable { reason: "links missing".into() };
                } else {
                    set::<tables::CountryLinks>(&mut frame, vec![CountryLinkRow { id: 9001, canonical_slot: 7 }, CountryLinkRow { id: 9002, canonical_slot: 8 }, CountryLinkRow { id: 9003, canonical_slot: 9 }]);
                }
                set::<tables::LexicalTokens>(&mut frame, vec![LexicalTokenRow { token: 9, literal: b"air_base".to_vec() }]);
                let mut rows = Vec::new();
                if hour == 201 {
                    rows.push(bombing(1, 201, None, 9001, 42, 9, BuildingBombingDamageCause::AirMission, 10_000_000));
                }
                if hour == 210 {
                    rows.push(bombing(2, 210, Some(9002), 9001, 42, 9, BuildingBombingDamageCause::Unknown, 5_000_000));
                }
                set::<tables::BuildingBombingDamage>(&mut frame, rows);
                if hour == 214 {
                    *frame.tables[TableId::BuildingBombingDamage.index()] = TableView::Unavailable { reason: "bombing hooks off".into() };
                }
                frame
            })
            .collect();
        let src = make_source(frames);
        let ctx = LoadContext { content: None };
        let cancel = AtomicBool::new(false);

        let taken_202 = BombingDamageBreakdown::load(&src, &BombingDamageBreakdownParams { branch_id: "1".into(), country: Country::Slot(7), hour: 202, direction: BombingDirection::Taken }, &ctx, &cancel).unwrap();
        assert_eq!(taken_202.breakdown.total.state, BreakdownState::Known);
        assert_eq!(taken_202.through, 202);
        assert!(taken_202.breakdown.parts[0].share.is_some());

        let dealt_202 = BombingDamageBreakdown::load(&src, &BombingDamageBreakdownParams { branch_id: "1".into(), country: Country::Slot(8), hour: 202, direction: BombingDirection::Dealt }, &ctx, &cancel).unwrap();
        assert_eq!(dealt_202.breakdown.total.state, BreakdownState::NotRecorded);
        assert!(dealt_202.breakdown.total.reason.as_deref().unwrap().contains("no recorded attacker"));

        let unjoined_203 = BombingDamageBreakdown::load(&src, &BombingDamageBreakdownParams { branch_id: "1".into(), country: Country::Slot(7), hour: 203, direction: BombingDirection::Taken }, &ctx, &cancel).unwrap();
        assert_eq!(unjoined_203.breakdown.total.state, BreakdownState::NotRecorded);
        assert!(unjoined_203.breakdown.total.reason.as_deref().unwrap().contains("CountryLinks"));

        let after_gap_205 = BombingDamageBreakdown::load(&src, &BombingDamageBreakdownParams { branch_id: "1".into(), country: Country::Slot(7), hour: 205, direction: BombingDirection::Taken }, &ctx, &cancel).unwrap();
        assert_eq!(after_gap_205.breakdown.total.state, BreakdownState::Partial);
        assert!(after_gap_205.breakdown.parts[0].share.is_none());
        assert_eq!(after_gap_205.through, 209);

        let after_unknown_212 = BombingDamageBreakdown::load(&src, &BombingDamageBreakdownParams { branch_id: "1".into(), country: Country::Slot(7), hour: 212, direction: BombingDirection::Taken }, &ctx, &cancel).unwrap();
        assert_eq!(after_unknown_212.breakdown.total.state, BreakdownState::Partial);
        assert!(after_unknown_212.breakdown.total.reason.as_deref().unwrap().contains("unsupported cause"));
        assert!(after_unknown_212.breakdown.parts[0].share.is_none());

        let bystander = BombingDamageBreakdown::load(&src, &BombingDamageBreakdownParams { branch_id: "1".into(), country: Country::Slot(9), hour: 212, direction: BombingDirection::Taken }, &ctx, &cancel).unwrap();
        assert!(!bystander.breakdown.total.reason.as_deref().unwrap_or("").contains("unsupported cause"), "an unsupported event between two other known countries cannot change this one: {:?}", bystander.breakdown.total);

        let withheld = BombingDamageBreakdown::load(&src, &BombingDamageBreakdownParams { branch_id: "1".into(), country: Country::Slot(7), hour: 214, direction: BombingDirection::Taken }, &ctx, &cancel).unwrap();
        assert!(withheld.breakdown.total.reason.as_deref().unwrap_or("").contains("bombing hooks off"), "a withheld read keeps its recorded reason: {:?}", withheld.breakdown.total);
        assert_eq!(withheld.through, 214);
    }
}
