
use super::{CountrySpan, UnobservedSpan};
use crate::{
    identity::{Country, Holder, Holders, TAG_UNLINKED},
    series::MissingSpan,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SortValue {
    Number(i64),
    Text(Box<str>),
}

impl SortValue {
    fn cmp_value(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => a.cmp(b),
            (Self::Text(a), Self::Text(b)) => a
                .chars()
                .flat_map(char::to_lowercase)
                .cmp(b.chars().flat_map(char::to_lowercase))
                .then_with(|| a.cmp(b)),
            (Self::Number(_), Self::Text(_)) => std::cmp::Ordering::Less,
            (Self::Text(_), Self::Number(_)) => std::cmp::Ordering::Greater,
        }
    }
}

fn ranked(a: (Option<&SortValue>, u32), b: (Option<&SortValue>, u32), direction: Direction) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let order = match (a.0, b.0) {
        (Some(x), Some(y)) if direction == Direction::Descending => y.cmp_value(x),
        (Some(x), Some(y)) => x.cmp_value(y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    };
    order.then(a.1.cmp(&b.1))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SortKey {
    Static(Option<SortValue>),
    Dated(Box<[(i32, Option<SortValue>)]>),
}

impl SortKey {
    pub fn number(value: Option<i64>) -> Self {
        Self::Static(value.map(SortValue::Number))
    }

    pub fn text(value: Option<impl Into<Box<str>>>) -> Self {
        Self::Static(value.map(|text| SortValue::Text(text.into())))
    }

    pub fn dated_numbers(steps: impl IntoIterator<Item = (i32, Option<i64>)>) -> Self {
        Self::Dated(steps.into_iter().map(|(at, value)| (at, value.map(SortValue::Number))).collect())
    }

    pub fn dated_text(steps: impl IntoIterator<Item = (i32, Option<Box<str>>)>) -> Self {
        Self::Dated(steps.into_iter().map(|(at, value)| (at, value.map(SortValue::Text))).collect())
    }

    pub fn at(&self, hour: i32) -> Option<&SortValue> {
        match self {
            Self::Static(value) => value.as_ref(),
            Self::Dated(steps) => {
                let at = steps.partition_point(|&(from, _)| from <= hour);
                at.checked_sub(1).and_then(|at| steps[at].1.as_ref())
            }
        }
    }

    fn after(&self, hour: i32) -> &[(i32, Option<SortValue>)] {
        match self {
            Self::Static(_) => &[],
            Self::Dated(steps) => {
                let at = steps.partition_point(|&(from, _)| from <= hour);
                &steps[at..]
            }
        }
    }

    pub fn next_change(&self, hour: i32) -> Option<i32> {
        self.after(hour).first().map(|step| step.0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Occurrence<P> {
    pub id: u128,
    pub date: i32,
    pub observed: i32,
    pub kind: u16,
    pub roles: Box<[Role]>,
    pub subjects: Box<[u128]>,
    pub states: Box<[(i32, u8)]>,
    pub sort_keys: Box<[SortKey]>,
    pub text: Box<str>,
    pub payload: P,
}

#[cfg(test)]
thread_local! { static STATE_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

impl<P> Occurrence<P> {
    pub fn state_at(&self, hour: i32) -> Option<u8> {
        #[cfg(test)]
        STATE_READS.set(STATE_READS.get() + 1);
        let at = self.states.partition_point(|&(from, _)| from <= hour);
        at.checked_sub(1).map(|at| self.states[at].1)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub membership: Vec<CountrySpan>,
    pub unobserved: Vec<UnobservedSpan>,
    pub unknown: Vec<MissingSpan>,
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
    Key(usize, Direction),
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
    pub gap_end: Option<i32>,
    pub selection: Option<&'a Selection>,
    pub subject: Option<u128>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct Signature {
    from: Option<i32>,
    to: Option<i32>,
    subject: Option<u128>,
    kinds: Option<Vec<u16>>,
    selection: Option<(Vec<(String, i32, i32)>, Vec<(i32, i32)>, Vec<(i32, i32, String)>)>,
    search: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PageShape {
    order: Order,
    live_first: bool,
    gap_end: i32,
    states: Option<Vec<u8>>,
    chips: Vec<Chip>,
}

struct Recent<K, V>(VecDeque<(K, Arc<V>, usize)>);

impl<K: PartialEq, V> Recent<K, V> {
    fn new() -> Self { Self(VecDeque::new()) }

    fn get(&mut self, key: &K) -> Option<Arc<V>> {
        let at = self.0.iter().position(|(kept, ..)| kept == key)?;
        let entry = self.0.remove(at)?;
        let found = Arc::clone(&entry.1);
        self.0.push_front(entry);
        Some(found)
    }

    fn insert(&mut self, key: K, value: V, rows: usize, mut remaining: usize) -> Arc<V> {
        if let Some(found) = self.get(&key) { return found; }
        let keep = self.0.iter().take(KEPT_SIGNATURES - 1).take_while(|(_, _, size)| {
            if *size > remaining { return false; }
            remaining -= size;
            true
        }).count();
        self.0.truncate(keep);
        let made = Arc::new(value);
        self.0.push_front((key, Arc::clone(&made), rows));
        made
    }
}

struct Filtered {
    display: Vec<(u32, i32)>,
    unattributed_rows: Vec<u32>,
    unknown: Vec<MissingSpan>,
    views: Mutex<Recent<PageShape, View>>,
}

struct View {
    changes: Vec<i32>,
    dated: Mutex<Option<Arc<PageOrder>>>,
}

struct PageOrder {
    hour: i32,
    until: Option<i32>,
    ordered: Vec<u32>,
    total: u64,
    live: u64,
    chips: Vec<u64>,
    gaps: Vec<PlacedGap>,
    unattributed: u64,
    changing_keys: Vec<(u32, i32)>,
}

pub struct Index<P> {
    entries: Vec<Occurrence<P>>,
    by_subject: HashMap<u128, Vec<u32>>,
    subject_gaps: HashMap<u128, Vec<MissingSpan>>,
    gaps: Vec<MissingSpan>,
    live: u64,
    filtered: Mutex<Recent<Signature, Filtered>>,
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

fn merge_missing_spans(mut spans: Vec<MissingSpan>) -> Vec<MissingSpan> {
    spans.retain(|span| span.first_hour <= span.last_hour);
    spans.sort_unstable_by(|a, b| (&a.reason, a.first_hour, a.last_hour).cmp(&(&b.reason, b.first_hour, b.last_hour)));
    let mut merged: Vec<MissingSpan> = Vec::with_capacity(spans.len());
    for span in spans {
        match merged.last_mut() {
            Some(held) if held.reason == span.reason && i64::from(held.last_hour) + 1 >= i64::from(span.first_hour) => {
                held.last_hour = held.last_hour.max(span.last_hour);
            }
            _ => merged.push(span),
        }
    }
    merged.sort_unstable_by(|a, b| (a.first_hour, a.last_hour, &a.reason).cmp(&(b.first_hour, b.last_hour, &b.reason)));
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
    unknown: Vec<MissingSpan>,
    unknown_hours: Vec<(i32, i32)>,
}

impl Members {
    fn resolve(holders: &Holders, selection: &Selection) -> Self {
        let mut slots: HashMap<u32, Vec<(i32, i32)>> = HashMap::new();
        let mut unknown: Vec<MissingSpan> = selection
            .unobserved
            .iter()
            .filter(|span| span.first_hour <= span.last_hour)
            .map(|span| MissingSpan { first_hour: span.first_hour, last_hour: span.last_hour, reason: MEMBERSHIP_UNKNOWN.to_owned() })
            .chain(selection.unknown.iter().filter(|span| span.first_hour <= span.last_hour).cloned())
            .collect();
        for span in &selection.membership {
            if let Country::Slot(slot) = span.country {
                slots.entry(slot).or_default().push((span.first_hour, span.last_hour));
                continue;
            }
            let Country::Tag(tag) = &span.country else { continue };
            for run in holders.within(&span.country, span.first_hour, span.last_hour) {
                let first = run.first.max(span.first_hour);
                let last = run.last.min(span.last_hour);
                if first > last {
                    continue;
                }
                match run.holder {
                    Holder::Found { slot, .. } => {
                        slots.entry(slot).or_default().push((first, last));
                    }
                    Holder::Unresolved => {
                        unknown.push(MissingSpan { first_hour: first, last_hour: last, reason: format!("Who holds {tag} is not known: {TAG_UNLINKED}") });
                    }
                }
            }
            for (u_first, u_last, reason) in &holders.unknown {
                let first = (*u_first).max(span.first_hour);
                let last = (*u_last).min(span.last_hour);
                if first <= last {
                    unknown.push(MissingSpan { first_hour: first, last_hour: last, reason: format!("Who holds {tag} is not known: {reason}") });
                }
            }
        }
        for spans in slots.values_mut() {
            *spans = sorted_spans(std::mem::take(spans));
        }
        let unknown = merge_missing_spans(unknown);
        let unknown_hours = sorted_spans(unknown.iter().map(|span| (span.first_hour, span.last_hour)).collect());
        Self { slots, unknown, unknown_hours }
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
            unknown |= within(&self.unknown_hours, role.at);
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
        let expected_keys = entries.first().map_or(0, |entry| entry.sort_keys.len());
        for entry in &entries {
            if entry.date > entry.observed {
                return Err(invalid(format!("Occurrence {} is dated {} after it was observed at {}", entry.id, entry.date, entry.observed)));
            }
            if entry.states.first().is_some_and(|&(from, _)| from != entry.observed) || entry.states.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
                return Err(invalid(format!("Occurrence {} has states out of order", entry.id)));
            }
            if entry.sort_keys.len() != expected_keys {
                return Err(invalid(format!("Occurrence {} has {} sort keys, expected {}", entry.id, entry.sort_keys.len(), expected_keys)));
            }
            for key in &entry.sort_keys {
                if let SortKey::Dated(steps) = key
                    && steps.windows(2).any(|pair| pair[0].0 >= pair[1].0)
                {
                    return Err(invalid(format!("Occurrence {} has dated sort keys out of order", entry.id)));
                }
            }
        }
        let mut ids = HashSet::with_capacity(entries.len());
        if entries.iter().any(|entry| !ids.insert(entry.id)) {
            return Err(invalid("Two occurrences share one id"));
        }
        drop(ids);
        entries.sort_unstable_by(|a, b| b.date.cmp(&a.date).then(b.id.cmp(&a.id)));
        let mut by_subject: HashMap<u128, Vec<u32>> = HashMap::new();
        for (at, entry) in entries.iter().enumerate() {
            for &subject in &entry.subjects {
                let list = by_subject.entry(subject).or_default();
                if list.last().copied() != Some(at as u32) {
                    list.push(at as u32);
                }
            }
        }
        gaps.retain(|gap| gap.first_hour <= gap.last_hour);
        gaps.sort_by_key(|gap| (gap.first_hour, gap.last_hour));
        let live = live.iter().fold(0u64, |mask, &state| mask | 1 << state);
        Ok(Self { entries, by_subject, subject_gaps: HashMap::new(), gaps, live, filtered: Mutex::new(Recent::new()) })
    }

    pub fn with_subject_gaps(mut self, gaps: HashMap<u128, Vec<MissingSpan>>) -> Self {
        self.subject_gaps = gaps.into_iter().map(|(subject, spans)| (subject, merge_missing_spans(spans))).collect();
        self
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
        let selection = ask.selection.map(|selection| {
            let mut membership: Vec<(String, i32, i32)> = selection.membership.iter().map(|span| (span.country.key().into_owned(), span.first_hour, span.last_hour)).collect();
            membership.sort_unstable();
            let unknown: Vec<(i32, i32, String)> = merge_missing_spans(selection.unknown.clone())
                .into_iter()
                .map(|span| (span.first_hour, span.last_hour, span.reason))
                .collect();
            (membership, sorted_spans(selection.unobserved.iter().map(|span| (span.first_hour, span.last_hour)).collect()), unknown)
        });
        let search = ask.search.map(str::trim).filter(|text| !text.is_empty()).map(str::to_lowercase);
        Signature { from: ask.from, to: ask.to, subject: ask.subject, kinds, selection, search }
    }

    fn changes(&self, entry: &Occurrence<P>, admitted: i32, ask: &Ask<'_>, counted: bool, mut change: impl FnMut(i32)) {
        let accepts = |state: Option<u8>| ask.states.is_none_or(|states| state.is_some_and(|state| states.contains(&state)));
        let live = |state: Option<u8>| state.is_some_and(|state| self.live & 1 << state != 0);
        let mut previous = entry.state_at(admitted);
        if accepts(previous) { change(admitted); }
        for &(hour, state) in &entry.states[entry.states.partition_point(|&(hour, _)| hour <= admitted)..] {
            let state = Some(state);
            if accepts(previous) != accepts(state)
                || (counted && accepts(state) && (live(previous) != live(state) || ask.chips.iter().any(|chip| chip.holds(entry.kind, previous) != chip.holds(entry.kind, state))))
            {
                change(hour);
            }
            previous = state;
        }
    }

    fn filtered(&self, holders: &Holders, ask: &Ask<'_>) -> Arc<Filtered> {
        let signature = Self::signature(ask);
        if let Some(found) = self.filtered.lock().unwrap_or_else(PoisonError::into_inner).get(&signature) {
            return found;
        }
        let members = ask.selection.map(|selection| Members::resolve(holders, selection));
        let (from, to) = (signature.from.unwrap_or(i32::MIN), signature.to.unwrap_or(i32::MAX));
        let mut display = Vec::new();
        let mut unattributed_rows = Vec::new();
        let mut visit = |at: usize| {
            let entry = &self.entries[at];
            if entry.date < from || entry.date > to || signature.kinds.as_ref().is_some_and(|kinds| kinds.binary_search(&entry.kind).is_err()) {
                return;
            }
            if signature.search.as_ref().is_some_and(|needle| !entry.text.to_lowercase().contains(needle.as_str())) {
                return;
            }
            match members.as_ref().map_or(Admit::Yes(entry.observed), |members| members.admits(&entry.roles, entry.observed)) {
                Admit::Yes(from) => display.push((at as u32, from)),
                Admit::Unattributed => unattributed_rows.push(at as u32),
                Admit::No | Admit::Unknown => {}
            }
        };
        if let Some(subject) = signature.subject {
            if let Some(indices) = self.by_subject.get(&subject) {
                for &at in indices {
                    visit(at as usize);
                }
            }
        } else {
            for at in 0..self.entries.len() {
                visit(at);
            }
        }
        let mut unknown = members.map_or_else(Vec::new, |members| members.unknown);
        if let Some(spans) = signature.subject.and_then(|id| self.subject_gaps.get(&id)) {
            unknown.extend_from_slice(spans);
        }
        let rows = display.len() + unattributed_rows.len();
        let made = Filtered { display, unattributed_rows, unknown, views: Mutex::new(Recent::new()) };
        self.filtered.lock().unwrap_or_else(PoisonError::into_inner).insert(signature, made, rows, self.entries.len())
    }

    fn view(&self, filtered: &Filtered, ask: &Ask<'_>, gap_end: i32) -> Arc<View> {
        let shape = PageShape { order: ask.order, live_first: ask.live_first, gap_end, states: ask.states.map(<[u8]>::to_vec), chips: ask.chips.to_vec() };
        if let Some(found) = filtered.views.lock().unwrap_or_else(PoisonError::into_inner).get(&shape) {
            return found;
        }
        let mut changes = Vec::new();
        for &(at, admitted) in &filtered.display {
            self.changes(self.get(at), admitted, ask, true, |hour| changes.push(hour));
        }
        for &at in &filtered.unattributed_rows {
            let entry = self.get(at);
            self.changes(entry, entry.observed, ask, false, |hour| changes.push(hour));
        }
        changes.sort_unstable();
        changes.dedup();
        let rows = filtered.display.len() + filtered.unattributed_rows.len();
        let made = View { changes, dated: Mutex::default() };
        filtered.views.lock().unwrap_or_else(PoisonError::into_inner).insert(shape, made, rows, rows)
    }

    fn visible_gaps(&self, filtered: &Filtered, from: i32, hour: i32, gap_end: i32) -> Vec<MissingSpan> {
        let mut out: Vec<MissingSpan> = Vec::new();
        let clip = |first: i32, last: i32, reason: &str, out: &mut Vec<MissingSpan>| {
            let (first, last) = (first.max(from), last.min(gap_end));
            if first <= last && first <= hour {
                out.push(MissingSpan { first_hour: first, last_hour: last, reason: reason.to_owned() });
            }
        };
        for gap in &self.gaps {
            clip(gap.first_hour, gap.last_hour, &gap.reason, &mut out);
        }
        for gap in &filtered.unknown {
            clip(gap.first_hour, gap.last_hour, &gap.reason, &mut out);
        }
        out.sort_by_key(|gap| (gap.first_hour, gap.last_hour));
        out
    }

    fn next_gap(&self, filtered: &Filtered, hour: i32, from: i32, gap_end: i32) -> Option<i32> {
        let mut next = None;
        let spans = self.gaps.iter().chain(&filtered.unknown).map(|gap| (gap.first_hour, gap.last_hour));
        for (first, last) in spans {
            let (first, last) = (first.max(from), last.min(gap_end));
            if first > last { continue; }
            if first > hour { next = Some(next.map_or(first, |held: i32| held.min(first))); }
        }
        next
    }

    pub fn ask(&self, holders: &Holders, ask: &Ask<'_>) -> io::Result<Answer> {
        self.window(holders, ask, None)
    }

    pub fn ask_visible(&self, holders: &Holders, ask: &Ask<'_>, visible: impl Fn(&P) -> bool) -> io::Result<Answer> {
        self.window(holders, ask, Some(&visible))
    }

    fn window(&self, holders: &Holders, ask: &Ask<'_>, visible: Option<&dyn Fn(&P) -> bool>) -> io::Result<Answer> {
        if ask.limit == 0 {
            return Err(invalid("An occurrence window holds one row or more"));
        }
        if let Order::Key(at, _) = ask.order
            && self.entries.first().is_some_and(|entry| at >= entry.sort_keys.len())
        {
            return Err(invalid(format!("This log has no sort column {at}")));
        }
        let filtered = self.filtered(holders, ask);
        let gap_end = ask.gap_end.unwrap_or(i32::MAX).min(ask.to.unwrap_or(i32::MAX));
        let order = if visible.is_some() {
            Arc::new(self.order(&filtered, ask, gap_end, visible, None))
        } else {
            let view = self.view(&filtered, ask, gap_end);
            let next = view.changes.get(view.changes.partition_point(|&at| at <= ask.hour)).copied();
            if matches!(ask.order, Order::Date(_)) {
                let mut kept = view.dated.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some(order) = kept.as_ref()
                    && order.hour <= ask.hour && order.until.is_none_or(|until| ask.hour < until)
                {
                    Arc::clone(order)
                } else {
                    let order = Arc::new(self.order(&filtered, ask, gap_end, None, next));
                    *kept = Some(Arc::clone(&order));
                    order
                }
            } else {
                Arc::new(self.order(&filtered, ask, gap_end, None, next))
            }
        };
        let offset = ask.offset.min(order.total);
        let start = (offset as usize).min(order.ordered.len());
        let end = start.saturating_add(ask.limit as usize).min(order.ordered.len());
        let mut next_change = order.until;
        let rows: Vec<_> = order.ordered[start..end].iter().enumerate().map(|(index, &at)| {
            let entry = self.get(at);
            let after = entry.states.partition_point(|&(hour, _)| hour <= ask.hour);
            next_change = next_change.into_iter().chain(entry.states.get(after).map(|step| step.0)).min();
            let state = entry.state_at(ask.hour);
            Shown { at, ordinal: offset + index as u64, live: state.is_some_and(|state| self.live & 1 << state != 0), state }
        }).collect();
        if let Order::Key(column, direction) = ask.order {
            if ask.live_first {
                next_change = next_change.into_iter().chain(order.changing_keys.iter().map(|change| change.1)).min();
            } else {
                next_change = self.next_rank_change(&rows, &order.changing_keys, ask.hour, (column, direction), next_change);
            }
        }
        Ok(Answer { hour: ask.hour, total: order.total, live: order.live, offset, rows, chips: order.chips.clone(), gaps: order.gaps.clone(), unattributed: order.unattributed, next_change })
    }

    fn order(&self, filtered: &Filtered, ask: &Ask<'_>, gap_end: i32, visible: Option<&dyn Fn(&P) -> bool>, next: Option<i32>) -> PageOrder {
        let hour = ask.hour;
        let from = ask.from.unwrap_or(i32::MIN);
        let mut gaps = self.visible_gaps(filtered, from, hour, gap_end);
        let state_ok = |state: Option<u8>| ask.states.is_none_or(|states| state.is_some_and(|state| states.contains(&state)));
        let mut chips = vec![0u64; ask.chips.len()];
        let mut live_rows = Vec::new();
        let mut live_count = 0u64;
        let mut next = next.into_iter().chain(self.next_gap(filtered, hour, from, gap_end)).min();
        let mut history = Vec::new();
        let mut history_total = 0u64;
        let mut changing_keys = Vec::new();
        let wanted_end = ask.offset.saturating_add(u64::from(ask.limit));
        let newest = ask.order == Order::Date(Direction::Descending);
        let mut placed: Vec<(usize, u64)> = Vec::new();
        let mut descending: Vec<usize> = (0..gaps.len()).collect();
        descending.sort_by_key(|&at| std::cmp::Reverse((gaps[at].last_hour, gaps[at].first_hour)));
        let mut next_gap = 0usize;
        let start = if visible.is_some() { 0 } else { filtered.display.partition_point(|&(at, _)| self.entries[at as usize].date > hour) };
        for &(at, from) in &filtered.display[start..] {
            let entry = &self.entries[at as usize];
            if let Some(visible) = visible {
                if !visible(&entry.payload) { continue; }
                self.changes(entry, from, ask, true, |at| {
                    if at > hour { next = Some(next.map_or(at, |held| held.min(at))); }
                });
            }
            if from > hour {
                continue;
            }
            let state = entry.state_at(hour);
            if !state_ok(state) {
                continue;
            }
            if let Order::Key(column, _) = ask.order
                && let Some(change) = entry.sort_keys[column].next_change(hour)
            {
                changing_keys.push((at, change));
            }
            for (count, chip) in chips.iter_mut().zip(ask.chips) {
                if chip.holds(entry.kind, state) {
                    *count += 1;
                }
            }
            let live = state.is_some_and(|state| self.live & 1 << state != 0);
            live_count += u64::from(live);
            if ask.live_first && live {
                live_rows.push(at);
                continue;
            }
            if newest {
                while next_gap < descending.len() && gaps[descending[next_gap]].last_hour >= entry.date {
                    placed.push((descending[next_gap], history_total));
                    next_gap += 1;
                }
            }
            history.push(at);
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
                    while seen < history.len() && self.entries[history[seen] as usize].date < gaps[at].first_hour {
                        seen += 1;
                    }
                    placed.push((at, seen as u64));
                }
            }
            Order::Key(column, direction) => {
                let key = |at: u32| self.entries[at as usize].sort_keys[column].at(hour);
                let by_key = |&a: &u32, &b: &u32| {
                    ranked((key(a), a), (key(b), b), direction)
                };
                let sort_prefix = |rows: &mut Vec<_>, through: u64| {
                    if through < rows.len() as u64 {
                        rows.select_nth_unstable_by(through as usize, by_key);
                        rows.truncate(through as usize);
                    }
                    rows.sort_unstable_by(by_key);
                };
                sort_prefix(&mut history, wanted_end.saturating_sub(live_total));
                sort_prefix(&mut live_rows, wanted_end);
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
        let unattributed = filtered.unattributed_rows.iter().filter(|&&at| {
            let entry = self.get(at);
            if visible.is_some() {
                self.changes(entry, entry.observed, ask, false, |at| {
                    if at > hour { next = Some(next.map_or(at, |held| held.min(at))); }
                });
            }
            entry.observed <= hour && state_ok(entry.state_at(hour))
        }).count() as u64;
        if live_rows.is_empty() {
            live_rows = history;
        } else {
            live_rows.extend(history);
        }
        PageOrder { hour, until: next, ordered: live_rows, total, live: live_count, chips, gaps: ordered_gaps, unattributed, changing_keys }
    }

    fn next_rank_change(&self, rows: &[Shown], changing: &[(u32, i32)], hour: i32, order: (usize, Direction), mut next: Option<i32>) -> Option<i32> {
        let (Some(first), Some(last)) = (rows.first(), rows.last()) else { return next };
        let (column, direction) = order;
        let key = |at: u32| &self.get(at).sort_keys[column];
        for row in rows {
            next = next.into_iter().chain(key(row.at).next_change(hour)).min();
        }
        let top = (key(first.at).at(hour), first.at);
        let bottom = (key(last.at).at(hour), last.at);
        let mut crossings = std::collections::BTreeMap::<i32, i64>::new();
        for &(at, change) in changing {
            if next.is_some_and(|end| change >= end) { continue; }
            let series = key(at);
            let mut before = ranked((series.at(hour), at), top, direction).is_lt();
            for (time, value) in series.after(hour) {
                if next.is_some_and(|end| *time >= end) { break; }
                let position = (value.as_ref(), at);
                let now_before = ranked(position, top, direction).is_lt();
                if (!now_before || first.ordinal == 0) && !ranked(position, bottom, direction).is_gt() {
                    next = Some(*time);
                    break;
                }
                if before != now_before {
                    *crossings.entry(*time).or_default() += if now_before { 1 } else { -1 };
                    before = now_before;
                }
            }
        }
        let shifted = crossings.into_iter().find_map(|(hour, count)| (count != 0).then_some(hour));
        next.into_iter().chain(shifted).min()
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

    pub fn coverage(&self, holders: &Holders, ask: &Ask<'_>, from: i32, hour: i32) -> Vec<MissingSpan> {
        self.visible_gaps(&self.filtered(holders, ask), from, hour, hour)
    }
}
