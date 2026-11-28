use super::{Ask, Chip, Filtered, Holders, Index, MissingSpan, Order, invalid};
use std::{collections::{BTreeMap, BTreeSet, HashSet}, io, sync::Arc};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Display,
    Unattributed,
}

struct Edge {
    hour: i32,
    at: u32,
    place: Place,
    shown: bool,
    state: Option<u8>,
}

pub struct DatedCursor<P> {
    index: Arc<Index<P>>,
    filtered: Arc<Filtered>,
    from: i32,
    gap_end: i32,
    schedule: Vec<Edge>,
    hour: Option<i32>,
    next: usize,
    rows: BTreeMap<u32, Option<u8>>,
    live: BTreeSet<u32>,
    chips: Vec<Chip>,
    counts: Vec<u64>,
    unattributed: HashSet<u32>,
    changed: Vec<DatedEdit>,
}

pub struct DatedEdit {
    pub at: u32,
    pub visibility_changed: bool,
}

pub struct DatedUpdate<'a> {
    pub reset: bool,
    pub changed: &'a [DatedEdit],
    pub gaps: Vec<MissingSpan>,
    pub live: u64,
    pub chips: &'a [u64],
    pub unattributed: u64,
    pub until: Option<i32>,
    #[cfg(test)]
    pub visited: usize,
    rows: &'a BTreeMap<u32, Option<u8>>,
    live_rows: &'a BTreeSet<u32>,
}

impl DatedUpdate<'_> {
    pub fn newest_first(&self) -> impl DoubleEndedIterator<Item = u32> + ExactSizeIterator + '_ {
        self.rows.keys().copied()
    }

    pub fn newest_within(&self, first: u32, last: u32) -> impl DoubleEndedIterator<Item = u32> + '_ {
        self.rows.range(first..=last).map(|(&at, _)| at)
    }

    pub fn live_newest_first(&self) -> impl DoubleEndedIterator<Item = u32> + ExactSizeIterator + '_ {
        self.live_rows.iter().copied()
    }

    pub fn is_live(&self, at: u32) -> bool {
        self.live_rows.contains(&at)
    }
}

impl<P> Index<P> {
    pub fn dated_cursor(self: &Arc<Self>, holders: &Holders, ask: &Ask<'_>) -> io::Result<DatedCursor<P>> {
        if !matches!(ask.order, Order::Date(_)) {
            return Err(invalid("Dated changes require date order"));
        }
        let filtered = self.filtered(holders, ask);
        let mut schedule = Vec::with_capacity(filtered.display.len() + filtered.unattributed_rows.len());
        let accepts = |state: Option<u8>| ask.states.is_none_or(|states| state.is_some_and(|state| states.contains(&state)));
        let mut collect = |at, admitted, until, place| {
            let entry = self.get(at);
            let end = match (until, self.retired_at(at)) {
                (Some(a), Some(b)) => Some(i32::min(a, b)),
                (a, b) => a.or(b),
            };
            self.changes(entry, admitted, ask, place == Place::Display, |hour| {
                if end.is_none_or(|end| hour < end) {
                    let state = entry.state_at(hour);
                    let shown = accepts(state);
                    schedule.push(Edge { hour, at, place, shown, state });
                }
            });
            if let Some(hour) = end && admitted < hour && accepts(entry.state_at(hour - 1)) {
                schedule.push(Edge { hour, at, place, shown: false, state: None });
            }
        };
        for &(at, admitted) in &filtered.display {
            collect(at, admitted, None, Place::Display);
        }
        for &(at, admitted, until) in &filtered.unattributed_rows {
            collect(at, admitted, until, Place::Unattributed);
        }
        schedule.sort_unstable_by_key(|edge| edge.hour);
        Ok(DatedCursor {
            index: Arc::clone(self),
            filtered,
            from: ask.from.unwrap_or(i32::MIN),
            gap_end: ask.gap_end.unwrap_or(i32::MAX).min(ask.to.unwrap_or(i32::MAX)),
            schedule,
            hour: None,
            next: 0,
            rows: BTreeMap::new(),
            live: BTreeSet::new(),
            chips: ask.chips.to_vec(),
            counts: vec![0; ask.chips.len()],
            unattributed: HashSet::new(),
            changed: Vec::new(),
        })
    }
}

impl<P> DatedCursor<P> {
    pub fn advance(&mut self, hour: i32) -> DatedUpdate<'_> {
        let reset = self.hour.is_none_or(|previous| hour < previous);
        if reset {
            self.next = 0;
            self.rows.clear();
            self.live.clear();
            self.counts.fill(0);
            self.unattributed.clear();
        }
        self.hour = Some(hour);
        self.changed.clear();
        #[cfg(test)]
        let start = self.next;
        while let Some(edge) = self.schedule.get(self.next).filter(|edge| edge.hour <= hour) {
            match edge.place {
                Place::Display => {
                    let previous = if edge.shown { self.rows.insert(edge.at, edge.state) } else { self.rows.remove(&edge.at) };
                    if edge.shown && edge.state.is_some_and(|state| self.index.live & 1 << state != 0) {
                        self.live.insert(edge.at);
                    } else {
                        self.live.remove(&edge.at);
                    }
                    let kind = self.index.get(edge.at).kind;
                    for (chip, count) in self.chips.iter().zip(&mut self.counts) {
                        *count = *count + u64::from(edge.shown && chip.holds(kind, edge.state))
                            - u64::from(previous.is_some_and(|state| chip.holds(kind, state)));
                    }
                    self.changed.push(DatedEdit { at: edge.at, visibility_changed: previous.is_some() != edge.shown });
                }
                Place::Unattributed => {
                    if edge.shown { self.unattributed.insert(edge.at); } else { self.unattributed.remove(&edge.at); }
                }
            }
            self.next += 1;
        }
        let next_gap = self.index.next_gap(&self.filtered, hour, self.from, self.gap_end);
        DatedUpdate {
            reset,
            rows: &self.rows,
            changed: &self.changed,
            gaps: self.index.visible_gaps(&self.filtered, self.from, hour, self.gap_end),
            live: self.live.len() as u64,
            live_rows: &self.live,
            chips: &self.counts,
            unattributed: self.unattributed.len() as u64,
            until: self.schedule.get(self.next).map(|edge| edge.hour).into_iter().chain(next_gap).min(),
            #[cfg(test)]
            visited: self.next - start,
        }
    }
}
