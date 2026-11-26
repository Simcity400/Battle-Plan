
use super::{
    EventFeedCategory, EventFeedKind, EventFeedRow, LOG, Notices, invalid,
    decision_state::{self, DecisionLifecycleState},
    families,
    row::{Columns, FeedEvent, Slot},
};
use dashboard_base::{Source, catalogue::{Catalogue, OwnRefusals}, products::LoadContext, queries::country_ids::Names};
use dashboard_query::{
    Query,
    identity::Holder,
    log::{Reader, occurrence::{Index, Occurrence, Role}},
    series::MissingSpan,
};
use replay_data::rows::FeedEventRow;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, Mutex, PoisonError, Weak, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ScriptStatus {
    Active,
    Taken,
    Completed,
    Failed,
    Cancelled,
    TimedOut,
    Removed,
    Unsupported,
    Awaiting,
    Chosen,
    NotRecorded,
    Available,
}

const STATUSES: [ScriptStatus; 12] = [
    ScriptStatus::Active,
    ScriptStatus::Taken,
    ScriptStatus::Completed,
    ScriptStatus::Failed,
    ScriptStatus::Cancelled,
    ScriptStatus::TimedOut,
    ScriptStatus::Removed,
    ScriptStatus::Unsupported,
    ScriptStatus::Awaiting,
    ScriptStatus::Chosen,
    ScriptStatus::NotRecorded,
    ScriptStatus::Available,
];

impl ScriptStatus {
    pub fn code(self) -> u8 {
        STATUSES.iter().position(|held| *held == self).unwrap_or(0) as u8
    }

    pub fn of_code(code: u8) -> Option<Self> {
        STATUSES.get(usize::from(code)).copied()
    }
}

pub(crate) const LIVE: [u8; 2] = [0, 8];

pub(crate) fn kind_code(kind: EventFeedKind) -> u16 {
    kind as u16
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Locator {
    pub run: u32,
    pub hour: i32,
    pub key: u128,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Item {
    Log { head: Locator, edges: Box<[Locator]> },
    Family(Box<EventFeedRow>),
}

#[derive(Clone, Debug)]
struct Brief {
    at: Locator,
    kind: EventFeedKind,
    generation: u64,
    sequence: u64,
    hour: i32,
    opener: Option<u64>,
    slots: [Option<u32>; 3],
    option: Option<i32>,
    days: Option<i32>,
    token: Option<String>,
}

pub(crate) struct Scanned {
    briefs: Vec<Brief>,
    native: Vec<Occurrence<Item>>,
    pub incomplete: Vec<(i32, u128, FeedEvent)>,
    pub windows_unrecorded: Vec<(i32, i32, String)>,
    states: HashMap<u128, DecisionLifecycleState>,
    resolved: Mutex<HashMap<u128, Option<DecisionLifecycleState>>>,
    presented: Mutex<Presented>,
    breaks: Vec<(i32, i32)>,
    decision_breaks: Vec<(i32, i32)>,
    pub gaps: Vec<MissingSpan>,
}

pub(crate) type HeldRow = Arc<(Option<EventFeedRow>, Notices, OwnRefusals)>;

const PRESENTED_REQUESTS: u64 = 32;

#[derive(Default)]
struct Presented {
    content: Option<Arc<Catalogue>>,
    request: u64,
    rows: HashMap<(u64, u64, i32, bool), (u64, HeldRow)>,
}

fn same_content(held: Option<&Arc<Catalogue>>, asked: Option<&Arc<Catalogue>>) -> bool {
    match (held, asked) {
        (Some(held), Some(asked)) => Arc::ptr_eq(held, asked),
        (None, None) => true,
        _ => false,
    }
}

fn presented_key(event: &FeedEvent, effects: bool) -> (u64, u64, i32, bool) {
    (event.generation, event.sequence, event.read_hour, effects)
}

impl Scanned {
    pub(crate) fn locate(&self, id: &str) -> Option<u128> {
        let mut parts = id.split(':');
        let (kind, first, second) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() {
            return None;
        }
        let found = match kind {
            "native" => {
                let (generation, sequence): (u64, u64) = (first.parse().ok()?, second.parse().ok()?);
                self.briefs.iter().find(|brief| brief.generation == generation && brief.sequence == sequence && generation != FeedEventRow::DERIVED)
            }
            "derived" => {
                let (hour, sequence): (i32, u32) = (first.parse().ok()?, second.parse().ok()?);
                self.briefs.iter().find(|brief| brief.generation == FeedEventRow::DERIVED && brief.at.hour == hour && brief.sequence as u32 == sequence)
            }
            _ => None,
        };
        found.map(|brief| brief.at.key)
    }

    pub(crate) fn held_rows(&self, content: Option<&Arc<Catalogue>>, effects: bool, events: &[&FeedEvent]) -> (u64, Vec<Option<HeldRow>>) {
        let mut memo = self.presented.lock().unwrap_or_else(PoisonError::into_inner);
        if !same_content(memo.content.as_ref(), content) {
            memo.rows.clear();
            memo.content = content.cloned();
        }
        memo.request += 1;
        let request = memo.request;
        let held = events
            .iter()
            .map(|event| {
                memo.rows.get_mut(&presented_key(event, effects)).map(|(used, row)| {
                    *used = request;
                    Arc::clone(row)
                })
            })
            .collect();
        (request, held)
    }

    pub(crate) fn keep_rows(&self, content: Option<&Arc<Catalogue>>, effects: bool, request: u64, made: Vec<(&FeedEvent, HeldRow)>) {
        let mut memo = self.presented.lock().unwrap_or_else(PoisonError::into_inner);
        if !same_content(memo.content.as_ref(), content) {
            return;
        }
        for (event, row) in made {
            let held = memo.rows.entry(presented_key(event, effects)).or_insert((request, Arc::clone(&row)));
            held.0 = held.0.max(request);
        }
        let floor = memo.request.saturating_sub(PRESENTED_REQUESTS);
        memo.rows.retain(|_, (used, _)| *used > floor);
    }

    pub(crate) fn decision_states(&self, query: &Query<'_>, events: &[&FeedEvent]) -> io::Result<Vec<Option<DecisionLifecycleState>>> {
        let key = |event: &FeedEvent| (u128::from(event.generation) << 64) | u128::from(event.sequence);
        let mut out: Vec<Option<DecisionLifecycleState>> = vec![None; events.len()];
        let mut missing: Vec<usize> = Vec::new();
        {
            let held = self.resolved.lock().unwrap_or_else(PoisonError::into_inner);
            for (at, event) in events.iter().enumerate() {
                if !decision_state::applies(event) {
                    continue;
                }
                let key = key(event);
                match (self.states.get(&key), held.get(&key)) {
                    (Some(state), _) => out[at] = Some(state.clone()),
                    (None, Some(state)) => out[at] = state.clone(),
                    (None, None) => missing.push(at),
                }
            }
        }
        if missing.is_empty() {
            return Ok(out);
        }
        let asked: Vec<&FeedEvent> = missing.iter().map(|&at| events[at]).collect();
        let resolved = decision_state::resolve_all(query, &asked)?;
        let mut held = self.resolved.lock().unwrap_or_else(PoisonError::into_inner);
        for (&at, state) in missing.iter().zip(resolved) {
            held.insert(key(events[at]), state.clone());
            out[at] = state;
        }
        Ok(out)
    }
}

pub(crate) struct Feed {
    pub index: Arc<Index<Item>>,
    pub notices: Vec<Notice>,
    pub warnings: families::DatedNotes,
    pub unattributed: Vec<i32>,
    pub unrecorded: Vec<families::Unrecorded>,
}

#[derive(Default)]
struct Branch {
    scanned: Option<Arc<Scanned>>,
    decisions: Option<Arc<Index<Item>>>,
    choices: Option<Arc<Index<Item>>>,
    feed: Option<(Option<Weak<Catalogue>>, Arc<Feed>)>,
}

#[derive(Default)]
pub(crate) struct Kept(HashMap<u64, Arc<Mutex<Branch>>>);

fn branch_of(source: &Source, branch: u64) -> Arc<Mutex<Branch>> {
    let kept = source.state::<Kept>();
    let mut kept = kept.lock().unwrap_or_else(PoisonError::into_inner);
    Arc::clone(kept.0.entry(branch).or_default())
}

pub(crate) fn forget(source: &Source) {
    *source.state::<Kept>().lock().unwrap_or_else(PoisonError::into_inner) = Kept::default();
}

fn slot_of(slot: Slot) -> Option<u32> {
    match slot {
        Slot::Of(slot) => Some(slot),
        Slot::None | Slot::NotRecorded => None,
    }
}

pub(crate) fn internal(kind: EventFeedKind) -> bool {
    use EventFeedKind::*;
    matches!(kind, DecisionRemovalEffect | DecisionCancellationEffect | DecisionCompletionEffect | DecisionTimeoutEffect | DecisionDurationChanged | DecisionReenabled)
}

fn script_opener(kind: EventFeedKind) -> bool {
    matches!(kind, EventFeedKind::ScriptEventFired | EventFeedKind::NewsEventFired)
}

fn text_of(token: Option<&str>, tags: impl Iterator<Item = String>) -> Box<str> {
    let mut text: Vec<String> = token.map(str::to_owned).into_iter().collect();
    text.extend(tags);
    text.join(" ").into_boxed_str()
}

fn feed_occurrence(event: &FeedEvent, kind: EventFeedKind, at: Locator, names: &Names) -> Occurrence<Item> {
    let tags = |roles: &[Role]| roles.iter().filter_map(|role| role.slot.and_then(|slot| names.slot_tag(slot, role.at)).map(str::to_owned)).collect::<Vec<_>>();
    let (date, roles): (i32, Vec<Role>) = if !event.derived() {
        let ally = event.cause.and_then(|cause| cause.ally).and_then(|ally| slot_of(ally.slot));
        (event.hour.min(at.hour), [slot_of(event.actor.slot), slot_of(event.target.slot), ally].into_iter().map(|slot| Role { slot, at: event.hour }).collect())
    } else if kind == EventFeedKind::CeasedToExist {
        (event.read_hour.min(at.hour), vec![Role { slot: slot_of(event.actor.slot), at: event.dated.unwrap_or(event.read_hour) }])
    } else {
        let (actor, target) = (slot_of(event.actor.slot), slot_of(event.target.slot));
        let start = event.dated.unwrap_or(event.read_hour);
        let exact = [actor, target].iter().all(|slot| slot.is_some_and(|slot| names.slot_tag(slot, start).is_some() && names.slot_tag(slot, start) == names.slot_tag(slot, event.read_hour)));
        let date = if exact { start } else { event.read_hour };
        (date.min(at.hour), vec![Role { slot: actor, at: date }, Role { slot: target, at: date }])
    };
    let token = event.token.as_deref().and_then(|bytes| std::str::from_utf8(bytes).ok());
    let text = text_of(token, tags(&roles).into_iter());
    Occurrence { id: at.key, date, observed: at.hour, kind: kind_code(kind), roles: roles.into_boxed_slice(), subjects: Box::new([]), states: Box::new([]), sort_keys: Box::new([]), text, payload: Item::Log { head: at, edges: Box::new([]) } }
}

fn scan(source: &Source, query: &Query<'_>, branch: u64, cancelled: &AtomicBool) -> io::Result<Scanned> {
    let declared = dashboard_declare::log(LOG).ok_or_else(|| invalid("The feed's log is not declared"))?;
    let reader = Reader::new(query, declared)?;
    let columns = Columns::new()?;
    let names = Names::new(&query.holders);
    let mut runs: Vec<u32> = query.timeline.stretches.iter().map(|stretch| stretch.run).collect();
    runs.sort_unstable();
    runs.dedup();
    let mut briefs = Vec::new();
    let mut native = Vec::new();
    let mut incomplete = Vec::new();
    let mut windows_unrecorded = Vec::new();
    let mut lifecycles: Vec<FeedEvent> = Vec::new();
    for run in runs {
        dashboard_text::metadata::check_cancel(cancelled)?;
        reader.each(run, |row| {
            if query.timeline.at(row.hour).is_none_or(|stretch| stretch.run != run) {
                return Ok(());
            }
            let event = columns.read(&row)?;
            if event.code == FeedEventRow::READ_INCOMPLETE {
                incomplete.push((row.hour, row.key, event));
                return Ok(());
            }
            if event.code == FeedEventRow::WINDOWS_UNRECORDED {
                windows_unrecorded.push((row.hour, row.key, event));
                return Ok(());
            }
            let Some(kind) = EventFeedKind::of_code(event.code) else { return Ok(()) };
            let at = Locator { run, hour: row.hour, key: row.key };
            if decision_state::applies(&event) && event.lifecycle.is_some() {
                lifecycles.push(event.clone());
            }
            if !internal(kind) {
                native.push(feed_occurrence(&event, kind, at, &names));
            }
            briefs.push(Brief {
                at,
                kind,
                generation: event.generation,
                sequence: event.sequence,
                hour: event.hour.min(row.hour),
                opener: event.opener,
                slots: [slot_of(event.actor.slot), slot_of(event.target.slot), event.cause.and_then(|cause| cause.ally).and_then(|ally| slot_of(ally.slot))],
                option: event.option,
                days: event.days,
                token: event.token.as_deref().and_then(|bytes| std::str::from_utf8(bytes).ok()).map(str::to_owned),
            });
            Ok(())
        })?;
    }
    briefs.sort_by_key(|brief| brief.at);
    incomplete.sort_by_key(|(hour, key, _)| (*hour, *key));
    windows_unrecorded.sort_by_key(|(hour, key, _)| (*hour, *key));
    let mut reasons: Vec<(i32, i32, String)> = Vec::new();
    for (hour, _, read) in &windows_unrecorded {
        let reason = String::from_utf8_lossy(&read.failure).into_owned();
        match reasons.iter_mut().find(|(.., held)| *held == reason) {
            Some(held) => held.1 = *hour,
            None => reasons.push((*hour, *hour, reason)),
        }
    }
    let refs: Vec<&FeedEvent> = lifecycles.iter().collect();
    let resolved = decision_state::resolve_all(query, &refs)?;
    let states = lifecycles.iter().zip(resolved).filter_map(|(event, state)| state.map(|state| ((u128::from(event.generation) << 64) | u128::from(event.sequence), state))).collect();
    let mut held: Vec<(i32, i32)> = query.timeline.unheld.iter().map(|gap| (gap.first, gap.last)).collect();
    held.extend(source.replay_ref().event_losses(branch).map(|(after, hour)| (after + 1, hour)));
    let complete = replay_data::event_feed_rows::EventFeedCoverage::Complete as u32;
    let lost = |decisions: bool| incomplete.iter().filter(move |(_, _, read)| read.coverage != Some(complete) || (decisions && read.refusals != Some(0))).map(|&(hour, ..)| (hour, hour));
    let breaks = spans(held.iter().copied().chain(lost(false)).collect());
    let decision_breaks = spans(held.into_iter().chain(lost(true)).collect());
    let gaps = dashboard_base::products::log_page::missing(query, declared, query.timeline.first, query.timeline.last)?;
    Ok(Scanned { briefs, native, incomplete, windows_unrecorded: reasons, states, resolved: Mutex::default(), presented: Mutex::default(), breaks, decision_breaks, gaps })
}

fn spans(mut breaks: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    breaks.retain(|(first, last)| first <= last);
    breaks.sort_unstable();
    let mut out: Vec<(i32, i32)> = Vec::with_capacity(breaks.len());
    for (first, last) in breaks {
        match out.last_mut() {
            Some(held) if first <= held.1.saturating_add(1) => held.1 = held.1.max(last),
            _ => out.push((first, last)),
        }
    }
    out
}

fn break_after(breaks: &[(i32, i32)], from: i32) -> Option<i32> {
    breaks.get(breaks.partition_point(|&(_, last)| last <= from)).map(|&(first, _)| first.max(from.saturating_add(1)))
}

struct Card<'b> {
    head: &'b Brief,
    opened: bool,
    edges: Vec<&'b Brief>,
}

fn cards<'b>(briefs: &'b [Brief], openers: &[EventFeedKind], continuing: &[EventFeedKind]) -> Vec<Card<'b>> {
    let mut out: Vec<Card<'b>> = Vec::new();
    let mut heads: HashMap<(u64, u64), usize> = HashMap::new();
    for brief in briefs {
        if brief.opener.is_none() && openers.contains(&brief.kind) {
            heads.insert((brief.generation, brief.sequence), out.len());
            out.push(Card { head: brief, opened: true, edges: vec![brief] });
        }
    }
    for brief in briefs {
        if !continuing.contains(&brief.kind) || (brief.opener.is_none() && openers.contains(&brief.kind)) {
            continue;
        }
        match brief.opener.and_then(|opener| heads.get(&(brief.generation, opener))) {
            Some(&at) => out[at].edges.push(brief),
            None => out.push(Card { head: brief, opened: false, edges: vec![brief] }),
        }
    }
    for card in &mut out {
        card.edges.sort_by_key(|edge| edge.at);
    }
    out
}

fn transitions(observed: i32, mut candidates: Vec<i32>, status: impl Fn(i32) -> Option<ScriptStatus>) -> Box<[(i32, u8)]> {
    candidates.push(observed);
    candidates.retain(|&hour| hour >= observed);
    candidates.sort_unstable();
    candidates.dedup();
    let mut out: Vec<(i32, u8)> = Vec::new();
    for hour in candidates {
        let Some(now) = status(hour) else { continue };
        out.push((hour, now.code()));
    }
    out.into_boxed_slice()
}

fn timer_end(edges: &[&Brief]) -> Option<i32> {
    use EventFeedKind::*;
    let edge = edges.iter().rev().find(|edge| matches!(edge.kind, DecisionSelected | DecisionActivated | DecisionDurationChanged) && edge.days.is_some())?;
    edge.days.filter(|days| *days > 0).map(|days| edge.hour.saturating_add(days.saturating_mul(24)))
}

fn decision_status(card: &Card<'_>, states: &HashMap<u128, DecisionLifecycleState>, breaks: &[(i32, i32)], superseded: Option<i32>, hour: i32) -> ScriptStatus {
    use EventFeedKind::*;
    let edges: Vec<&Brief> = card.edges.iter().copied().filter(|edge| edge.at.hour <= hour).collect();
    let ends = [DecisionCompleted, DecisionCancelled, DecisionTimedOut, DecisionRemoved];
    let end = edges.iter().rev().find(|edge| ends.contains(&edge.kind) || edge.kind == DecisionActivated).filter(|edge| ends.contains(&edge.kind));
    let ends_at = timer_end(&edges);
    let timer = ends_at.is_some();
    let lifecycle = |edge: &Brief| states.get(&((u128::from(edge.generation) << 64) | u128::from(edge.sequence))).filter(|state| !matches!(state, DecisionLifecycleState::NotApplicable { .. }));
    let state = end.and_then(|edge| lifecycle(edge)).or_else(|| edges.iter().rev().find_map(|edge| lifecycle(edge)));
    let mut status = match end.map(|edge| edge.kind) {
        Some(DecisionCompleted) => ScriptStatus::Completed,
        Some(DecisionCancelled) => ScriptStatus::Cancelled,
        Some(DecisionTimedOut) => ScriptStatus::TimedOut,
        Some(_) => ScriptStatus::Removed,
        None if timer || edges.iter().any(|edge| matches!(edge.kind, DecisionActivated | DecisionDurationChanged)) => ScriptStatus::Active,
        None => ScriptStatus::Taken,
    };
    if end.is_none() && !timer && !edges.iter().any(|edge| edge.kind == DecisionSelected) && matches!(state, Some(DecisionLifecycleState::Available)) {
        return ScriptStatus::Available;
    }
    match state {
        Some(DecisionLifecycleState::Failed { .. }) => status = ScriptStatus::Failed,
        Some(DecisionLifecycleState::Unsupported { .. }) => status = ScriptStatus::Unsupported,
        Some(DecisionLifecycleState::Completed) if end.is_none() && status == ScriptStatus::Active => status = ScriptStatus::Completed,
        Some(DecisionLifecycleState::Aborted) if end.is_none() && status == ScriptStatus::Active => status = ScriptStatus::Cancelled,
        _ => {}
    }
    let unwitnessed = edges.last().is_some_and(|last| break_after(breaks, last.at.hour).is_some_and(|at| at <= hour))
        || superseded.is_some_and(|at| at <= hour)
        || ends_at.is_some_and(|at| at <= hour);
    if status == ScriptStatus::Active && unwitnessed {
        status = ScriptStatus::NotRecorded;
    }
    status
}

fn decision_key<'b>(card: &Card<'b>) -> Option<(u32, &'b str, Option<u32>)> {
    Some((card.head.slots[0]?, card.head.token.as_deref()?, card.head.slots[1]))
}

fn superseded<'b>(cards: &[Card<'b>]) -> Vec<Option<i32>> {
    let mut by_key: HashMap<(u32, &'b str, Option<u32>), Vec<usize>> = HashMap::new();
    for (at, card) in cards.iter().enumerate() {
        if card.opened
            && let Some(key) = decision_key(card)
        {
            by_key.entry(key).or_default().push(at);
        }
    }
    let mut out = vec![None; cards.len()];
    for mut opened in by_key.into_values() {
        opened.sort_by_key(|&at| (cards[at].head.at.hour, cards[at].head.at.key));
        for pair in opened.windows(2) {
            out[pair[0]] = Some(cards[pair[1]].head.at.hour);
        }
    }
    out
}

fn chosen(edge: &Brief) -> bool {
    edge.kind == EventFeedKind::EventOptionSelected && edge.option.is_none_or(|option| option >= 0)
}

fn choice_status(edges: &[&Brief], observed: i32, breaks: &[(i32, i32)], hour: i32) -> ScriptStatus {
    if edges.iter().any(|edge| edge.at.hour <= hour && chosen(edge)) {
        ScriptStatus::Chosen
    } else if break_after(breaks, observed).is_some_and(|at| at <= hour) {
        ScriptStatus::NotRecorded
    } else {
        ScriptStatus::Awaiting
    }
}


fn card_occurrence(card: &Card<'_>, head: &Brief, names: &Names, states: Box<[(i32, u8)]>) -> Occurrence<Item> {
    let roles: Vec<Role> = card.edges.iter().flat_map(|edge| edge.slots.iter().map(|&slot| Role { slot, at: edge.hour })).filter(|role| role.slot.is_some()).collect();
    let tags = roles.iter().filter_map(|role| role.slot.and_then(|slot| names.slot_tag(slot, role.at)).map(str::to_owned)).collect::<BTreeSet<_>>();
    Occurrence {
        id: head.at.key,
        date: head.hour,
        observed: head.at.hour,
        kind: kind_code(head.kind),
        roles: roles.into_boxed_slice(),
        subjects: Box::new([]),
        states,
        sort_keys: Box::new([]),
        text: text_of(card.edges.iter().find_map(|edge| edge.token.as_deref()), tags.into_iter()),
        payload: Item::Log { head: head.at, edges: card.edges.iter().map(|edge| edge.at).collect() },
    }
}

fn decisions(scanned: &Scanned, names: &Names) -> io::Result<Index<Item>> {
    let list = super::cards::List::Decisions;
    let found = cards(&scanned.briefs, list.openers(), list.continuing());
    let later = superseded(&found);
    let made = found
        .iter()
        .zip(later)
        .map(|(card, later)| {
            let mut candidates: Vec<i32> = card.edges.iter().map(|edge| edge.at.hour).collect();
            for (at, edge) in card.edges.iter().enumerate() {
                candidates.extend(break_after(&scanned.decision_breaks, edge.at.hour));
                candidates.extend(timer_end(&card.edges[..=at]));
            }
            candidates.extend(later);
            let states = transitions(card.head.at.hour, candidates, |hour| Some(decision_status(card, &scanned.states, &scanned.decision_breaks, later, hour)));
            card_occurrence(card, card.head, names, states)
        })
        .collect();
    Index::new(made, scanned.gaps.clone(), &LIVE)
}

fn choices(scanned: &Scanned, names: &Names) -> io::Result<Index<Item>> {
    let list = super::cards::List::EventChoices;
    let mut made = Vec::new();
    for card in cards(&scanned.briefs, list.openers(), list.continuing()) {
        let head = if card.opened { card.head } else {
            let Some(first) = card.edges.iter().copied().find(|edge| chosen(edge)) else { continue };
            first
        };
        let mut candidates: Vec<i32> = card.edges.iter().map(|edge| edge.at.hour).collect();
        candidates.extend(break_after(&scanned.breaks, head.at.hour));
        let states = transitions(head.at.hour, candidates, |hour| Some(choice_status(&card.edges, head.at.hour, &scanned.breaks, hour)));
        made.push(card_occurrence(&card, head, names, states));
    }
    Index::new(made, scanned.gaps.clone(), &LIVE)
}

pub(crate) fn family_at(index: &Index<Item>, id: &str) -> Option<u32> {
    index.find(family_id(id)).filter(|&at| matches!(&index.get(at).payload, Item::Family(row) if row.id == id))
}

fn family_id(text: &str) -> u128 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    (u128::from(FeedEventRow::DERIVED - 1) << 64) | u128::from(hash)
}

fn family_occurrence(query: &Query<'_>, row: EventFeedRow) -> Occurrence<Item> {
    let roles: Vec<Role> = row
        .tags
        .iter()
        .map(|tag| Role {
            slot: query.holders.at(tag.as_str(), row.name_hour).and_then(|run| match run.holder {
                Holder::Found { slot, .. } => Some(slot),
                _ => None,
            }),
            at: row.name_hour,
        })
        .collect();
    let name = row.item.as_ref().and_then(|item| item.name.clone());
    let text = text_of(name.as_deref(), row.tags.iter().cloned());
    Occurrence {
        id: family_id(&row.id),
        date: row.hour,
        observed: row.read_hour.max(row.hour),
        kind: kind_code(row.kind),
        roles: roles.into_boxed_slice(),
        subjects: Box::new([]),
        states: Box::new([]),
        sort_keys: Box::new([]),
        text,
        payload: Item::Family(Box::new(row)),
    }
}

fn script_feed(scanned: &Scanned) -> Vec<Occurrence<Item>> {
    let mut openers: HashMap<(u64, u64), usize> = HashMap::new();
    let mut native = scanned.native.clone();
    for (at, occurrence) in native.iter().enumerate() {
        if let Some(kind) = EventFeedKind::of_index(occurrence.kind)
            && script_opener(kind)
        {
            openers.insert(((occurrence.id >> 64) as u64, occurrence.id as u64), at);
        }
    }
    let mut merged: BTreeSet<u128> = BTreeSet::new();
    let mut edges: BTreeMap<usize, Vec<&Brief>> = BTreeMap::new();
    for brief in &scanned.briefs {
        if brief.kind != EventFeedKind::EventOptionSelected {
            continue;
        }
        if let Some(&at) = brief.opener.and_then(|opener| openers.get(&(brief.generation, opener))) {
            edges.entry(at).or_default().push(brief);
            merged.insert(brief.at.key);
        }
    }
    for (&at, chosen_edges) in &edges {
        let occurrence = &mut native[at];
        let observed = occurrence.observed;
        let candidates = chosen_edges.iter().map(|edge| edge.at.hour).chain(break_after(&scanned.breaks, observed)).collect();
        occurrence.states = transitions(observed, candidates, |hour| Some(choice_status(chosen_edges, observed, &scanned.breaks, hour)));
        if let Item::Log { edges, .. } = &mut occurrence.payload {
            *edges = chosen_edges.iter().map(|edge| edge.at).collect();
        }
    }
    for (at, occurrence) in native.iter_mut().enumerate() {
        if occurrence.states.is_empty()
            && !edges.contains_key(&at)
            && EventFeedKind::of_index(occurrence.kind).is_some_and(script_opener)
        {
            let observed = occurrence.observed;
            occurrence.states = transitions(observed, break_after(&scanned.breaks, observed).into_iter().collect(), |hour| Some(choice_status(&[], observed, &scanned.breaks, hour)));
        }
    }
    native.retain(|occurrence| !merged.contains(&occurrence.id));
    native
}

type Notice = (Vec<EventFeedCategory>, String, String, String, Option<u32>);

fn family<T>(made: io::Result<T>, categories: &[EventFeedCategory], kind: &str, label: &str, refused: &mut Vec<Notice>, cancelled: &AtomicBool) -> io::Result<Option<T>> {
    match made {
        Ok(made) => Ok(Some(made)),
        Err(error) if cancelled.load(std::sync::atomic::Ordering::Relaxed) => Err(error),
        Err(error) => {
            refused.push((categories.to_vec(), kind.to_owned(), label.to_owned(), error.to_string(), None));
            Ok(None)
        }
    }
}

fn feed(source: &Source, query: &Query<'_>, context: &LoadContext, scanned: &Scanned, cancelled: &AtomicBool) -> io::Result<Feed> {
    use EventFeedCategory::{Diplomacy, Milestones, WarsPeace};
    let (first, last) = (query.timeline.first, query.timeline.last);
    let mut refused: Vec<Notice> = Vec::new();
    let mut rows: Vec<EventFeedRow> = Vec::new();
    let mut warnings: families::DatedNotes = Vec::new();
    let mut unattributed = Vec::new();
    let mut unrecorded = Vec::new();
    if let Some((made, notes, left_out, unobserved)) = family(families::group_rows(source, query, context, first, last, &[WarsPeace, Diplomacy], cancelled), &[WarsPeace, Diplomacy], "countryGroups", "Wars and factions", &mut refused, cancelled)? {
        rows.extend(made);
        warnings.extend(notes);
        unattributed = left_out;
        unrecorded.extend(unobserved);
    }
    if let Some(made) = family(families::subject_rows(source, query, first, last, cancelled), &[Diplomacy], "subjects", "Subjects", &mut refused, cancelled)? {
        rows.extend(made);
    }
    if let Some((made, notes, projects)) = family(families::project_completions(source, query, context, first, last, cancelled), &[Milestones], "projectFinished", "Special projects", &mut refused, cancelled)? {
        rows.extend(made);
        warnings.extend(notes.into_iter().map(|note| (first, note)));
        unrecorded.push(projects);
    }
    let localisation = match context.content.as_ref() {
        Some(content) => dashboard_base::content::localisation(source, content).map_err(|error| format!("Nuclear type names are unavailable: {error}")),
        None => Err("Nuclear type names are unavailable until the game folders are chosen".to_owned()),
    };
    if let Err(reason) = &localisation {
        refused.push((vec![WarsPeace], "nuclearTypeNames".to_owned(), "Nuclear type names".to_owned(), reason.clone(), None));
    }
    let localise = |spelling: &str| localisation.as_ref().ok().and_then(|held| dashboard_text::catalogue::localised(held, spelling, dashboard_text::catalogue::View::Plain).ok());
    let mut notices = Notices::default();
    if let Some(made) = family(super::nuclear::strike_rows(query, first, last, &localise, &mut notices, cancelled), &[WarsPeace], "nuclearStrike", "Nuclear strikes", &mut refused, cancelled)? {
        rows.extend(made);
    }
    refused.extend(notices.into_list().into_iter().map(|notice| (vec![WarsPeace], notice.kind, notice.label, notice.reason, notice.count)));
    let mut occurrences = script_feed(scanned);
    let mut seen: BTreeSet<u128> = occurrences.iter().map(|occurrence| occurrence.id).collect();
    for row in rows {
        let occurrence = family_occurrence(query, row);
        if seen.insert(occurrence.id) {
            occurrences.push(occurrence);
        }
    }
    warnings.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
    warnings.dedup_by(|later, earlier| later.1 == earlier.1);
    warnings.sort();
    Ok(Feed { index: Arc::new(Index::new(occurrences, scanned.gaps.clone(), &LIVE)?), notices: refused, warnings, unattributed, unrecorded })
}

fn scanned(source: &Source, query: &Query<'_>, branch: u64, held: &mut Branch, cancelled: &AtomicBool) -> io::Result<Arc<Scanned>> {
    if let Some(found) = &held.scanned {
        return Ok(Arc::clone(found));
    }
    let made = Arc::new(scan(source, query, branch, cancelled)?);
    held.scanned = Some(Arc::clone(&made));
    Ok(made)
}

pub(crate) fn scanned_for(source: &Source, query: &Query<'_>, branch: u64, cancelled: &AtomicBool) -> io::Result<Arc<Scanned>> {
    let kept = branch_of(source, branch);
    let mut held = kept.lock().unwrap_or_else(PoisonError::into_inner);
    scanned(source, query, branch, &mut held, cancelled)
}

pub(crate) fn decisions_for(source: &Source, query: &Query<'_>, branch: u64, cancelled: &AtomicBool) -> io::Result<Arc<Index<Item>>> {
    let kept = branch_of(source, branch);
    let mut held = kept.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(found) = &held.decisions {
        return Ok(Arc::clone(found));
    }
    let scanned = scanned(source, query, branch, &mut held, cancelled)?;
    let made = Arc::new(decisions(&scanned, &Names::new(&query.holders))?);
    held.decisions = Some(Arc::clone(&made));
    Ok(made)
}

pub(crate) fn choices_for(source: &Source, query: &Query<'_>, branch: u64, cancelled: &AtomicBool) -> io::Result<Arc<Index<Item>>> {
    let kept = branch_of(source, branch);
    let mut held = kept.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(found) = &held.choices {
        return Ok(Arc::clone(found));
    }
    let scanned = scanned(source, query, branch, &mut held, cancelled)?;
    let made = Arc::new(choices(&scanned, &Names::new(&query.holders))?);
    held.choices = Some(Arc::clone(&made));
    Ok(made)
}

pub(crate) fn feed_for(source: &Source, query: &Query<'_>, branch: u64, context: &LoadContext, catalogue: Option<&Arc<Catalogue>>, cancelled: &AtomicBool) -> io::Result<Arc<Feed>> {
    let kept = branch_of(source, branch);
    let mut held = kept.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((made_with, found)) = &held.feed
        && made_with.as_ref().and_then(Weak::upgrade).as_ref().map(Arc::as_ptr) == catalogue.map(Arc::as_ptr)
        && made_with.is_some() == catalogue.is_some()
    {
        return Ok(Arc::clone(found));
    }
    let scanned = scanned(source, query, branch, &mut held, cancelled)?;
    let made = Arc::new(feed(source, query, context, &scanned, cancelled)?);
    held.feed = Some((catalogue.map(Arc::downgrade), Arc::clone(&made)));
    Ok(made)
}

#[cfg(any(test, feature = "test-support"))]
pub fn kept(source: &Source) -> Vec<u64> {
    let kept = source.state::<Kept>();
    let kept = kept.lock().unwrap_or_else(PoisonError::into_inner);
    let mut branches: Vec<u64> = kept.0.iter().filter(|(_, held)| held.lock().unwrap_or_else(PoisonError::into_inner).scanned.is_some()).map(|(branch, _)| *branch).collect();
    branches.sort_unstable();
    branches
}

pub fn warm(source: &Source, branch: u64) -> io::Result<()> {
    let query = dashboard_base::queries::query(source, branch)?;
    let cancelled = AtomicBool::new(false);
    decisions_for(source, &query, branch, &cancelled)?;
    choices_for(source, &query, branch, &cancelled)?;
    Ok(())
}

pub(crate) fn read_events(query: &Query<'_>, locators: &[Locator]) -> io::Result<HashMap<u128, FeedEvent>> {
    let declared = dashboard_declare::log(LOG).ok_or_else(|| invalid("The feed's log is not declared"))?;
    let reader = Reader::new(query, declared)?;
    let columns = Columns::new()?;
    let mut by_run: BTreeMap<u32, BTreeSet<i32>> = BTreeMap::new();
    let wanted: BTreeSet<u128> = locators.iter().map(|at| at.key).collect();
    for at in locators {
        by_run.entry(at.run).or_default().insert(at.hour);
    }
    let mut out = HashMap::with_capacity(wanted.len());
    for (run, hours) in by_run {
        let mut ranges: Vec<(i32, i32)> = Vec::new();
        for hour in hours {
            match ranges.last_mut() {
                Some(held) if held.1 + 1 >= hour => held.1 = hour,
                _ => ranges.push((hour, hour)),
            }
        }
        for row in reader.inserted(run, &ranges)? {
            if wanted.contains(&row.key) {
                out.insert(row.key, columns.read(&row)?);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
