
use super::{EventFeedKind, FeedEvent, LOG, Paged, invalid, logs, row::{Columns, Slot}};
use dashboard_base::{Source, queries::country_ids::Names};
use dashboard_world::products::groups::MembershipRun;
use dashboard_query::{
    Query,
    identity::{Holder, Holders},
    log::{self, Counts, FacetFilter, Pattern, Reader},
    series::{MissingSpan, PAGE_LIMIT, hour_of},
};
use replay_columnar::summary::bucket_hours;
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, Mutex, PoisonError},
};

const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub hour: i32,
    pub at: i32,
    pub key: u128,
    pub facet: u128,
    pub slots: [u32; 3],
}

pub(crate) fn entry(hour: i32, key: u128, facet: u128, event: &FeedEvent, names: &Names) -> Entry {
    let slot = |slot: Slot| match slot {
        Slot::Of(slot) => slot,
        Slot::None | Slot::NotRecorded => NONE,
    };
    let (actor, target) = (slot(event.actor.slot), slot(event.target.slot));
    if !event.derived() {
        let ally = event.cause.and_then(|cause| cause.ally).map_or(NONE, |ally| slot(ally.slot));
        return Entry { hour, at: event.hour, key, facet, slots: [actor, target, ally] };
    }
    if EventFeedKind::of_code(event.code) == Some(EventFeedKind::CeasedToExist) {
        return Entry { hour, at: event.dated.unwrap_or(event.read_hour), key, facet, slots: [actor, NONE, NONE] };
    }
    let start = event.dated.unwrap_or(event.read_hour);
    let exact = [actor, target].iter().all(|&slot| names.slot_tag(slot, start).is_some() && names.slot_tag(slot, start) == names.slot_tag(slot, event.read_hour));
    Entry { hour, at: if exact { start } else { event.read_hour }, key, facet, slots: [actor, target, NONE] }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Scope(HashMap<u32, Vec<(i32, i32)>>);

impl Scope {
    pub fn resolve(holders: &Holders, runs: &[MembershipRun]) -> Self {
        let mut slots: HashMap<u32, Vec<(i32, i32)>> = HashMap::new();
        for run in runs {
            for held in holders.within(&run.tag, run.first_hour, run.last_hour) {
                if let Holder::Found { slot, .. } = held.holder {
                    slots.entry(slot).or_default().push((held.first, held.last));
                }
            }
        }
        for spans in slots.values_mut() {
            spans.sort_unstable();
        }
        Self(slots)
    }

    pub fn admits(&self, entry: &Entry) -> bool {
        entry.slots.iter().any(|slot| self.0.get(slot).is_some_and(|spans| spans.iter().any(|&(first, last)| first <= entry.at && entry.at <= last)))
    }
}

pub(crate) fn matched<'i>(index: &'i [(u32, Arc<Vec<Entry>>)], patterns: &[Pattern], scope: &Scope) -> Vec<(u32, &'i Entry)> {
    let mut out: Vec<(u32, &Entry)> = Vec::new();
    for (run, entries) in index {
        let mine: Vec<&Pattern> = patterns.iter().filter(|pattern| pattern.run == *run).collect();
        out.extend(
            entries
                .iter()
                .filter(|entry| mine.iter().any(|pattern| pattern.first <= entry.hour && entry.hour <= pattern.last && pattern.holds(entry.facet)) && scope.admits(entry))
                .map(|entry| (*run, entry)),
        );
    }
    out.sort_by_key(|(_, entry)| (entry.hour, entry.key));
    out
}

#[cfg(test)]
pub(crate) fn window(matched: &[(u32, &Entry)], from: i32, to: i32, cursor: u64, size: u32) -> (u64, u64, u64, u64) {
    let base = matched.partition_point(|(_, entry)| entry.hour < from) as u64;
    let last = matched.partition_point(|(_, entry)| entry.hour <= to) as u64;
    let first = cursor.max(base);
    (base, first, first.saturating_add(u64::from(size)).min(last), last)
}

pub(crate) fn count(matched: &[(u32, &Entry)], first: i32, last: i32, bin_hours: i64) -> io::Result<Counts> {
    if bin_hours < 1 || (bin_hours >= bucket_hours(0) && bin_hours % bucket_hours(0) != 0) {
        return Err(invalid("A log's bins are 1 to 5 hours or whole six-hour buckets"));
    }
    let first_bin = i64::from(first).div_euclid(bin_hours) * bin_hours;
    let mut bins: BTreeMap<u32, u64> = BTreeMap::new();
    let mut before = 0;
    for (_, entry) in matched {
        if entry.hour < first {
            before += 1;
        } else if entry.hour <= last {
            *bins.entry(((i64::from(entry.hour) - first_bin).div_euclid(bin_hours)) as u32).or_default() += 1;
        }
    }
    Ok(Counts { before, bins: bins.into_iter().collect() })
}

#[derive(Default)]
pub(crate) struct Index(HashMap<u32, Arc<Mutex<Option<Arc<Vec<Entry>>>>>>);

pub(crate) type ScopeIndex = Arc<Mutex<Index>>;

pub(crate) fn index(source: &Source) -> ScopeIndex {
    source.state::<Index>()
}

pub(crate) fn forget(source: &Source) {
    *source.state::<Index>().lock().unwrap_or_else(PoisonError::into_inner) = Index::default();
}

fn entries_in(index: &ScopeIndex, reader: &Reader<'_, '_>, names: &Names, run: u32) -> io::Result<Arc<Vec<Entry>>> {
    let slot = Arc::clone(index.lock().unwrap_or_else(PoisonError::into_inner).0.entry(run).or_default());
    let mut held = slot.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(found) = held.as_ref() {
        return Ok(Arc::clone(found));
    }
    let columns = Columns::new()?;
    let mut made = Vec::new();
    reader.each(run, |row| {
        made.push(entry(row.hour, row.key, row.facet, &columns.read(&row)?, names));
        Ok(())
    })?;
    made.sort_unstable_by_key(|entry| (entry.hour, entry.key));
    made.shrink_to_fit();
    let made = Arc::new(made);
    *held = Some(Arc::clone(&made));
    Ok(made)
}

fn entries(source: &Source, reader: &Reader<'_, '_>, names: &Names, run: u32) -> io::Result<Arc<Vec<Entry>>> {
    entries_in(&index(source), reader, names, run)
}

pub fn warm(source: &Source, branch: u64) -> io::Result<()> {
    let query = dashboard_base::queries::query(source, branch)?;
    let reader = Reader::new(&query, declared()?)?;
    let names = Names::new(&query.holders);
    let mut runs: Vec<u32> = Vec::new();
    for stretch in query.timeline.stretches.iter().rev() {
        if !runs.contains(&stretch.run) {
            runs.push(stretch.run);
        }
    }
    for run in runs {
        entries(source, &reader, &names, run)?;
    }
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
pub fn kept(source: &Source) -> Vec<u32> {
    let index = source.state::<Index>();
    let index = index.lock().unwrap_or_else(PoisonError::into_inner);
    let mut runs: Vec<u32> = index.0.iter().filter(|(_, slot)| slot.lock().unwrap_or_else(PoisonError::into_inner).is_some()).map(|(run, _)| *run).collect();
    runs.sort_unstable();
    runs
}

fn scoped_in(
    scope_index: &ScopeIndex,
    query: &Query<'_>,
    reader: &Reader<'_, '_>,
    facets: &[FacetFilter],
    to: i32,
) -> io::Result<(Vec<Pattern>, Vec<(u32, Arc<Vec<Entry>>)>)> {
    if facets.iter().any(|filter| !filter.countries.is_empty()) {
        return Err(invalid("A country scope is not asked with a filter by tags"));
    }
    let selection = reader.select(facets, query.timeline.first, to)?;
    let names = Names::new(&query.holders);
    let mut runs: Vec<u32> = selection.patterns.iter().map(|pattern| pattern.run).collect();
    runs.sort_unstable();
    runs.dedup();
    let index = runs.into_iter().map(|run| Ok((run, entries_in(scope_index, reader, &names, run)?))).collect::<io::Result<_>>()?;
    Ok((selection.patterns, index))
}

fn scoped(source: &Source, query: &Query<'_>, reader: &Reader<'_, '_>, facets: &[FacetFilter], scope: &[MembershipRun], to: i32) -> io::Result<(Vec<Pattern>, Vec<(u32, Arc<Vec<Entry>>)>, Scope)> {
    let (patterns, index) = scoped_in(&index(source), query, reader, facets, to)?;
    Ok((patterns, index, Scope::resolve(&query.holders, scope)))
}

fn declared() -> io::Result<&'static dashboard_declare::Log> {
    dashboard_declare::log(LOG).ok_or_else(|| invalid("The feed's log is not declared"))
}

pub(crate) fn tail_with_index(
    scope_index: &ScopeIndex,
    query: &Query<'_>,
    facets: &[FacetFilter],
    from: Option<f64>,
    to: i32,
    offset: u64,
    limit: u32,
    count_facets: &[logs::LogFacetCountAsk],
    scope: &[MembershipRun],
    caches: Option<&logs::LogCaches>,
) -> io::Result<(log::NewestPage, Vec<Paged>, Vec<logs::LogFacetTotal>, Vec<MissingSpan>)> {
    let log = declared()?;
    let reader = match caches {
        Some(caches) => Reader::with_cache(query, log, Arc::clone(&caches.buckets))?,
        None => Reader::new(query, log)?,
    };
    let from = from.map(|from| hour_of(Some(from), "from")).transpose()?.unwrap_or(query.timeline.first);
    if !(1..=PAGE_LIMIT).contains(&limit) {
        return Err(invalid("Invalid tail limit"));
    }
    if from > to {
        let totals = count_facets.iter().map(|ask| logs::LogFacetTotal { key: ask.key.clone(), total: "0".into() }).collect();
        return Ok((log::NewestPage { base: 0, offset: 0, total: 0, rows: Vec::new() }, Vec::new(), totals, Vec::new()));
    }
    let (patterns, entries_by_run) = scoped_in(scope_index, query, &reader, facets, to)?;
    let resolved = Scope::resolve(&query.holders, scope);
    let admitted = matched(&entries_by_run, &patterns, &resolved);
    let base = admitted.partition_point(|(_, entry)| entry.hour < from) as u64;
    let last = admitted.partition_point(|(_, entry)| entry.hour <= to) as u64;
    let total = last.saturating_sub(base);
    let rows = if offset >= total {
        Vec::new()
    } else {
        let want_first = offset;
        let want_end = offset.saturating_add(u64::from(limit)).min(total);
        let mut shown: Vec<(u32, &Entry, u64)> = Vec::with_capacity((want_end - want_first) as usize);
        let mut seen_newer = 0u64;
        let mut h_hi = last as usize;
        let b = base as usize;
        while h_hi > b && seen_newer < want_end {
            let hour = admitted[h_hi - 1].1.hour;
            let mut h_lo = h_hi - 1;
            while h_lo > b && admitted[h_lo - 1].1.hour == hour {
                h_lo -= 1;
            }
            let hour_count = (h_hi - h_lo) as u64;
            if seen_newer + hour_count > want_first {
                let lo_in = want_first.saturating_sub(seen_newer) as usize;
                let hi_in = want_end.saturating_sub(seen_newer).min(hour_count) as usize;
                for i in lo_in..hi_in {
                    let pos = h_lo + i;
                    let (run, entry) = admitted[pos];
                    shown.push((run, entry, pos as u64));
                }
            }
            seen_newer += hour_count;
            h_hi = h_lo;
        }
        let mut ordinals: HashMap<(i32, u128), u64> = shown
            .iter()
            .map(|(_, entry, ordinal)| ((entry.hour, entry.key), *ordinal))
            .collect();
        let mut hours_by_run: BTreeMap<u32, Vec<i32>> = BTreeMap::new();
        for (run, entry, _) in &shown {
            hours_by_run.entry(*run).or_default().push(entry.hour);
        }
        let mut by_run: BTreeMap<u32, Vec<(i32, i32)>> = BTreeMap::new();
        for (run, mut hours) in hours_by_run {
            hours.sort_unstable();
            hours.dedup();
            let ranges = by_run.entry(run).or_default();
            for hour in hours {
                match ranges.last_mut() {
                    Some(held) if held.1 + 1 >= hour => held.1 = held.1.max(hour),
                    _ => ranges.push((hour, hour)),
                }
            }
        }
        let columns = Columns::new()?;
        let mut rows = Vec::with_capacity(shown.len());
        for (run, ranges) in by_run {
            for (_, row) in reader.rows(run, &patterns, &ranges)? {
                if let Some(ordinal) = ordinals.remove(&(row.hour, row.key)) {
                    rows.push(Paged { event: columns.read(&row)?, hour: row.hour, key: row.key, ordinal, own: ordinal, tag: None });
                }
            }
        }
        rows.sort_by_key(|paged| (std::cmp::Reverse(paged.hour), paged.ordinal));
        rows
    };
    let mut totals = Vec::with_capacity(count_facets.len());
    for ask in count_facets {
        let (c_patterns, c_entries) = scoped_in(scope_index, query, &reader, &ask.facets, to)?;
        let c_admitted = matched(&c_entries, &c_patterns, &resolved);
        let c_base = c_admitted.partition_point(|(_, entry)| entry.hour < from) as u64;
        let c_last = c_admitted.partition_point(|(_, entry)| entry.hour <= to) as u64;
        totals.push(logs::LogFacetTotal { key: ask.key.clone(), total: c_last.saturating_sub(c_base).to_string() });
    }
    let mut gaps = logs::missing_with(query, log, from, to, caches.map(|c| c.coverage.as_ref()))?;
    logs::append_unobserved(&mut gaps, facets, from, to);
    let page = log::NewestPage { base, offset, total, rows: Vec::new() };
    Ok((page, rows, totals, gaps))
}

pub(crate) fn counts(source: &Source, query: &Query<'_>, facets: &[FacetFilter], scope: &[MembershipRun], from: i32, to: i32, bin_hours: i64) -> io::Result<Counts> {
    let reader = Reader::new(query, declared()?)?;
    let (patterns, index, scope) = scoped(source, query, &reader, facets, scope, to)?;
    count(&matched(&index, &patterns, &scope), from, to, bin_hours)
}
