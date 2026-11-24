
use super::{CountrySpan, UnobservedSpan};
use crate::{
    identity::{Country, Holder, Holders},
    series::MissingSpan,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    io,
    sync::{Arc, Mutex, PoisonError},
};
use ts_rs::TS;

#[cfg(test)]
mod tests;

pub const MEMBERSHIP_UNKNOWN: &str = "Group membership is unavailable at this time.";

const KEPT_SIGNATURES: usize = 16;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Role {
    pub slot: Option<u32>,
    pub at: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Occurrence<P> {
    pub id: u128,
    pub date: i32,
    pub observed: i32,
    pub kind: u16,
    pub roles: Box<[Role]>,
    pub states: Box<[(i32, u8)]>,
    pub figures: Box<[Option<i64>]>,
    pub text: Box<str>,
    pub payload: P,
}

impl<P> Occurrence<P> {
    pub fn state_at(&self, hour: i32) -> Option<u8> {
        let at = self.states.partition_point(|&(from, _)| from <= hour);
        at.checked_sub(1).map(|at| self.states[at].1)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub membership: Vec<CountrySpan>,
    pub unobserved: Vec<UnobservedSpan>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "OccurrenceDirection")]
pub enum Direction {
    #[default]
    Descending,
    Ascending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    Date(Direction),
    Figure(usize, Direction),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chip {
    pub kinds: Option<Vec<u16>>,
    pub states: Option<Vec<u8>>,
}

impl Chip {
    fn holds(&self, kind: u16, state: Option<u8>) -> bool {
        self.kinds.as_ref().is_none_or(|kinds| kinds.contains(&kind)) && self.states.as_ref().is_none_or(|states| state.is_some_and(|state| states.contains(&state)))
    }
}

#[derive(Clone, Debug)]
pub struct Ask<'a> {
    pub hour: i32,
    pub from: Option<i32>,
    pub to: Option<i32>,
    pub selection: Option<&'a Selection>,
    pub kinds: Option<&'a [u16]>,
    pub states: Option<&'a [u8]>,
    pub search: Option<&'a str>,
    pub order: Order,
    pub live_first: bool,
    pub offset: u64,
    pub limit: u32,
    pub chips: &'a [Chip],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "OccurrenceGap")]
pub struct PlacedGap {
    pub first_hour: i32,
    pub last_hour: i32,
    pub reason: String,
    #[doc = " The ordinal of the first row after the gap in the reply's order."]
    pub before: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown {
    pub at: u32,
    pub ordinal: u64,
    pub live: bool,
    pub state: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub hour: i32,
    pub total: u64,
    pub live: u64,
    pub offset: u64,
    pub rows: Vec<Shown>,
    pub chips: Vec<u64>,
    pub gaps: Vec<PlacedGap>,
    pub unattributed: u64,
    pub next_change: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct Signature {
    from: Option<i32>,
    to: Option<i32>,
    kinds: Option<Vec<u16>>,
    membership: Vec<(String, i32, i32)>,
    unobserved: Vec<(i32, i32)>,
    search: Option<String>,
}

struct Filtered {
    display: Vec<(u32, i32)>,
    observed: Vec<i32>,
    changes: Vec<i32>,
    unattributed: Vec<i32>,
    unknown: Vec<(i32, i32)>,
}

pub struct Index<P> {
    entries: Vec<Occurrence<P>>,
    gaps: Vec<MissingSpan>,
    live: u64,
    filtered: Mutex<VecDeque<(Signature, Arc<Filtered>)>>,
}

fn sorted_spans(mut spans: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    spans.retain(|&(first, last)| first <= last);
    spans.sort_unstable();
    let mut merged: Vec<(i32, i32)> = Vec::with_capacity(spans.len());
    for (first, last) in spans {
        match merged.last_mut() {
            Some(held) if i64::from(held.1) + 1 >= i64::from(first) => held.1 = held.1.max(last),
            _ => merged.push((first, last)),
        }
    }
    merged
}

fn within(spans: &[(i32, i32)], hour: i32) -> bool {
    let at = spans.partition_point(|&(_, last)| last < hour);
    spans.get(at).is_some_and(|&(first, _)| first <= hour)
}

enum Admit {
    Yes(i32),
    No,
    Unknown,
    Unattributed,
}

struct Members {
    slots: HashMap<u32, Vec<(i32, i32)>>,
    unknown: Vec<(i32, i32)>,
}

impl Members {
    fn resolve(holders: &Holders, selection: &Selection) -> Self {
        let mut slots: HashMap<u32, Vec<(i32, i32)>> = HashMap::new();
        for span in &selection.membership {
            if let Country::Slot(slot) = span.country {
                slots.entry(slot).or_default().push((span.first_hour, span.last_hour));
                continue;
            }
            for run in holders.within(&span.country, span.first_hour, span.last_hour) {
                if let Holder::Found { slot, .. } = run.holder {
                    slots.entry(slot).or_default().push((run.first.max(span.first_hour), run.last.min(span.last_hour)));
                }
            }
        }
        for spans in slots.values_mut() {
            *spans = sorted_spans(std::mem::take(spans));
        }
        Self { slots, unknown: sorted_spans(selection.unobserved.iter().map(|span| (span.first_hour, span.last_hour)).collect()) }
    }

    fn admits(&self, roles: &[Role], observed: i32) -> Admit {
        let mut unknown = false;
        let mut named = false;
        let mut admitted: Option<i32> = None;
        for role in roles {
            let Some(slot) = role.slot else { continue };
            named = true;
            if self.slots.get(&slot).is_some_and(|spans| within(spans, role.at)) {
                let known = role.at.max(observed);
                admitted = Some(admitted.map_or(known, |held| held.min(known)));
                continue;
            }
            unknown |= within(&self.unknown, role.at);
        }
        match (admitted, unknown, named) {
            (Some(hour), ..) => Admit::Yes(hour),
            (None, true, _) => Admit::Unknown,
            (None, false, false) => Admit::Unattributed,
            (None, false, true) => Admit::No,
        }
    }
}

impl<P> Index<P> {
    pub fn new(mut entries: Vec<Occurrence<P>>, mut gaps: Vec<MissingSpan>, live: &[u8]) -> io::Result<Self> {
        for entry in &entries {
            if entry.date > entry.observed {
                return Err(invalid(format!("Occurrence {} is dated {} after it was observed at {}", entry.id, entry.date, entry.observed)));
            }
            if entry.states.first().is_some_and(|&(from, _)| from != entry.observed) || entry.states.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
                return Err(invalid(format!("Occurrence {} has states out of order", entry.id)));
            }
        }
        entries.sort_unstable_by(|a, b| b.date.cmp(&a.date).then(b.id.cmp(&a.id)));
        if entries.windows(2).any(|pair| pair[0].id == pair[1].id) {
            return Err(invalid("Two occurrences share one id"));
        }
        gaps.retain(|gap| gap.first_hour <= gap.last_hour);
        gaps.sort_by_key(|gap| (gap.first_hour, gap.last_hour));
        let live = live.iter().fold(0u64, |mask, &state| mask | 1 << state);
        Ok(Self { entries, gaps, live, filtered: Mutex::default() })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, at: u32) -> &Occurrence<P> {
        &self.entries[at as usize]
    }

    pub fn find(&self, id: u128) -> Option<u32> {
        self.entries.iter().position(|entry| entry.id == id).map(|at| at as u32)
    }

    pub fn gaps(&self) -> &[MissingSpan] {
        &self.gaps
    }

    fn signature(ask: &Ask<'_>) -> Signature {
        let kinds = ask.kinds.map(|kinds| {
            let mut kinds = kinds.to_vec();
            kinds.sort_unstable();
            kinds.dedup();
            kinds
        });
        let (membership, unobserved) = match ask.selection {
            None => (Vec::new(), Vec::new()),
            Some(selection) => {
                let mut membership: Vec<(String, i32, i32)> = selection.membership.iter().map(|span| (span.country.key().into_owned(), span.first_hour, span.last_hour)).collect();
                membership.sort_unstable();
                (membership, sorted_spans(selection.unobserved.iter().map(|span| (span.first_hour, span.last_hour)).collect()))
            }
        };
        let search = ask.search.map(str::trim).filter(|text| !text.is_empty()).map(str::to_lowercase);
        Signature { from: ask.from, to: ask.to, kinds, membership, unobserved, search }
    }

    fn filtered(&self, holders: &Holders, ask: &Ask<'_>) -> Arc<Filtered> {
        let signature = Self::signature(ask);
        {
            let kept = self.filtered.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((_, found)) = kept.iter().find(|(held, _)| *held == signature) {
                return Arc::clone(found);
            }
        }
        let members = ask.selection.filter(|_| !signature.membership.is_empty() || !signature.unobserved.is_empty()).map(|selection| Members::resolve(holders, selection));
        let (from, to) = (signature.from.unwrap_or(i32::MIN), signature.to.unwrap_or(i32::MAX));
        let mut display = Vec::new();
        let mut unattributed = Vec::new();
        for (at, entry) in self.entries.iter().enumerate() {
            if entry.date < from || entry.date > to || signature.kinds.as_ref().is_some_and(|kinds| kinds.binary_search(&entry.kind).is_err()) {
                continue;
            }
            if signature.search.as_ref().is_some_and(|needle| !entry.text.to_lowercase().contains(needle.as_str())) {
                continue;
            }
            match members.as_ref().map_or(Admit::Yes(entry.observed), |members| members.admits(&entry.roles, entry.observed)) {
                Admit::Yes(from) => display.push((at as u32, from)),
                Admit::Unattributed => unattributed.push(entry.observed),
                Admit::No | Admit::Unknown => {}
            }
        }
        let mut observed: Vec<i32> = display.iter().map(|&(_, from)| from).collect();
        observed.sort_unstable();
        let mut changes: Vec<i32> = display.iter().flat_map(|&(at, _)| self.entries[at as usize].states.iter().skip(1).map(|&(from, _)| from)).collect();
        changes.sort_unstable();
        unattributed.sort_unstable();
        let made = Arc::new(Filtered { display, observed, changes, unattributed, unknown: signature.unobserved.clone() });
        let mut kept = self.filtered.lock().unwrap_or_else(PoisonError::into_inner);
        kept.push_back((signature, Arc::clone(&made)));
        while kept.len() > KEPT_SIGNATURES {
            kept.pop_front();
        }
        made
    }

    fn visible_gaps(&self, filtered: &Filtered, from: i32, hour: i32) -> Vec<MissingSpan> {
        let mut out: Vec<MissingSpan> = Vec::new();
        let clip = |first: i32, last: i32, reason: &str, out: &mut Vec<MissingSpan>| {
            let (first, last) = (first.max(from), last.min(hour));
            if first <= last {
                out.push(MissingSpan { first_hour: first, last_hour: last, reason: reason.to_owned() });
            }
        };
        for gap in &self.gaps {
            clip(gap.first_hour, gap.last_hour, &gap.reason, &mut out);
        }
        for &(first, last) in &filtered.unknown {
            clip(first, last, MEMBERSHIP_UNKNOWN, &mut out);
        }
        out.sort_by_key(|gap| (gap.first_hour, gap.last_hour));
        out
    }

    fn next_change(&self, filtered: &Filtered, hour: i32) -> Option<i32> {
        let after = |hours: &[i32]| hours.get(hours.partition_point(|&at| at <= hour)).copied();
        let mut next = [after(&filtered.observed), after(&filtered.changes), after(&filtered.unattributed)].into_iter().flatten().min();
        let spans = self.gaps.iter().map(|gap| (gap.first_hour, gap.last_hour)).chain(filtered.unknown.iter().copied());
        for (first, last) in spans {
            let boundary = if first <= hour && hour < last { hour + 1 } else if first > hour { first } else { continue };
            next = Some(next.map_or(boundary, |held| held.min(boundary)));
        }
        next
    }

    pub fn ask(&self, holders: &Holders, ask: &Ask<'_>) -> io::Result<Answer> {
        if ask.limit == 0 {
            return Err(invalid("An occurrence window holds one row or more"));
        }
        if let Order::Figure(at, _) = ask.order
            && self.entries.first().is_some_and(|entry| at >= entry.figures.len())
        {
            return Err(invalid(format!("This log has no figure {at}")));
        }
        let hour = ask.hour;
        let filtered = self.filtered(holders, ask);
        let from = ask.from.unwrap_or(i32::MIN);
        let mut gaps = self.visible_gaps(&filtered, from, hour);
        let state_ok = |state: Option<u8>| ask.states.is_none_or(|states| state.is_some_and(|state| states.contains(&state)));
        let mut chips = vec![0u64; ask.chips.len()];
        let mut live_rows: Vec<(u32, Option<u8>)> = Vec::new();
        let mut history: Vec<(u32, Option<u8>)> = Vec::new();
        let mut history_total = 0u64;
        let wanted_end = ask.offset.saturating_add(u64::from(ask.limit));
        let newest = ask.order == Order::Date(Direction::Descending);
        let mut placed: Vec<(usize, u64)> = Vec::new();
        let mut descending: Vec<usize> = (0..gaps.len()).collect();
        descending.sort_by_key(|&at| std::cmp::Reverse((gaps[at].last_hour, gaps[at].first_hour)));
        let mut next_gap = 0usize;
        let start = filtered.display.partition_point(|&(at, _)| self.entries[at as usize].date > hour);
        for &(at, from) in &filtered.display[start..] {
            let entry = &self.entries[at as usize];
            if from > hour {
                continue;
            }
            let state = entry.state_at(hour);
            if !state_ok(state) {
                continue;
            }
            for (count, chip) in chips.iter_mut().zip(ask.chips) {
                if chip.holds(entry.kind, state) {
                    *count += 1;
                }
            }
            let live = state.is_some_and(|state| self.live & 1 << state != 0);
            if ask.live_first && live {
                live_rows.push((at, state));
                continue;
            }
            if newest {
                while next_gap < descending.len() && gaps[descending[next_gap]].last_hour >= entry.date {
                    placed.push((descending[next_gap], history_total));
                    next_gap += 1;
                }
                if history_total < wanted_end {
                    history.push((at, state));
                }
            } else {
                history.push((at, state));
            }
            history_total += 1;
        }
        let live_total = live_rows.len() as u64;
        let total = live_total + history_total;
        match ask.order {
            Order::Date(Direction::Descending) => {
                while next_gap < descending.len() {
                    placed.push((descending[next_gap], history_total));
                    next_gap += 1;
                }
            }
            Order::Date(Direction::Ascending) => {
                history.reverse();
                live_rows.reverse();
                let mut ascending: Vec<usize> = (0..gaps.len()).collect();
                ascending.sort_by_key(|&at| (gaps[at].first_hour, gaps[at].last_hour));
                let mut seen = 0usize;
                for at in ascending {
                    while seen < history.len() && self.entries[history[seen].0 as usize].date < gaps[at].first_hour {
                        seen += 1;
                    }
                    placed.push((at, seen as u64));
                }
            }
            Order::Figure(figure, direction) => {
                let key = |at: u32| self.entries[at as usize].figures[figure];
                let by_figure = |a: &(u32, Option<u8>), b: &(u32, Option<u8>)| {
                    let ordered = match (key(a.0), key(b.0)) {
                        (Some(x), Some(y)) if direction == Direction::Descending => y.cmp(&x),
                        (Some(x), Some(y)) => x.cmp(&y),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => std::cmp::Ordering::Equal,
                    };
                    ordered.then(a.0.cmp(&b.0))
                };
                history.sort_by(by_figure);
                live_rows.sort_by(by_figure);
                placed.extend((0..gaps.len()).map(|at| (at, history_total)));
            }
        }
        placed.sort_by_key(|&(at, before)| (before, std::cmp::Reverse(gaps[at].last_hour)));
        let ordered_gaps: Vec<PlacedGap> = placed
            .into_iter()
            .map(|(at, before)| {
                let gap = std::mem::replace(&mut gaps[at], MissingSpan { first_hour: 0, last_hour: 0, reason: String::new() });
                PlacedGap { first_hour: gap.first_hour, last_hour: gap.last_hour, reason: gap.reason, before: (live_total + before).to_string() }
            })
            .collect();
        let mut rows = Vec::new();
        let end = wanted_end.min(total);
        let mut ordinal = ask.offset;
        while ordinal < end {
            let (at, state, live) = if ordinal < live_total {
                let (at, state) = live_rows[ordinal as usize];
                (at, state, true)
            } else {
                let Some(&(at, state)) = history.get((ordinal - live_total) as usize) else { break };
                (at, state, state.is_some_and(|state| self.live & 1 << state != 0))
            };
            rows.push(Shown { at, ordinal, live, state });
            ordinal += 1;
        }
        let unattributed = filtered.unattributed.partition_point(|&observed| observed <= hour) as u64;
        Ok(Answer { hour, total, live: live_total, offset: ask.offset, rows, chips, gaps: ordered_gaps, unattributed, next_change: self.next_change(&filtered, hour) })
    }

    pub fn bins(&self, holders: &Holders, ask: &Ask<'_>, first: i32, last: i32, bin_hours: i64) -> io::Result<(u64, Vec<(u32, u64)>)> {
        if bin_hours < 1 || first > last {
            return Err(invalid("Invalid occurrence bins"));
        }
        let filtered = self.filtered(holders, ask);
        let first_bin = i64::from(first).div_euclid(bin_hours) * bin_hours;
        let mut before = 0u64;
        let mut bins: std::collections::BTreeMap<u32, u64> = std::collections::BTreeMap::new();
        let start = filtered.display.partition_point(|&(at, _)| self.entries[at as usize].date > last.min(ask.hour));
        for &(at, from) in &filtered.display[start..] {
            let entry = &self.entries[at as usize];
            if from > ask.hour || ask.states.is_some_and(|states| entry.state_at(ask.hour).is_none_or(|state| !states.contains(&state))) {
                continue;
            }
            if entry.date < first {
                before += 1;
            } else {
                *bins.entry(((i64::from(entry.date) - first_bin).div_euclid(bin_hours)) as u32).or_default() += 1;
            }
        }
        Ok((before, bins.into_iter().collect()))
    }
}
