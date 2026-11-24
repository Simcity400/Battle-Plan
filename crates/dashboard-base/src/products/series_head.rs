use crate::Source;
use dashboard_core::config::LoadContext;
use dashboard_follow::panel::{Declared, Panel, PanelData, ProductRead, ProductReader, ProductStep};
use dashboard_query::{
    Query,
    head::{
        BranchHeadIndex, HeadLine, HeadMarks, HeadRoster, HeadRosterEntry, HeadValue, MarkPrefixIndex,
        OpenBinAcc, PRODUCT, ParsedHeadParams, bin_bounds, compute_cold_counted, compute_single_line_cold,
        coverage_missing, parse_params,
    },
    identity::{Country, Holder},
    read,
    series::{Ask, Line, board},
};
use playhead::changes::{Changes, Written};
use replay_columnar::{
    manifest::Status,
    store::Value as CellValue,
    summary::{Cells, Computed, Source as ColumnSource},
};
use replay_data::rows::TableId;
use serde_json::Value;
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, Mutex, PoisonError},
};

#[derive(Default)]
struct BranchIndexes(HashMap<u64, Arc<BranchHeadIndex>>);

#[derive(Default)]
struct MarksCache(Arc<MarkPrefixIndex>);

pub fn forget(source: &Source) {
    source.state::<BranchIndexes>().lock().unwrap_or_else(PoisonError::into_inner).0.clear();
    source.state::<MarksCache>().lock().unwrap_or_else(PoisonError::into_inner).0.clear();
}

fn branch_index(source: &Source, query: &Query<'_>) -> Arc<BranchHeadIndex> {
    let state = source.state::<BranchIndexes>();
    if let Some(found) = state.lock().unwrap_or_else(PoisonError::into_inner).0.get(&query.timeline.id) {
        return Arc::clone(found);
    }
    let built = Arc::new(BranchHeadIndex::build(query));
    state.lock().unwrap_or_else(PoisonError::into_inner).0.insert(query.timeline.id, Arc::clone(&built));
    built
}

fn marks_cache(source: &Source) -> Arc<MarkPrefixIndex> {
    Arc::clone(&source.state::<MarksCache>().lock().unwrap_or_else(PoisonError::into_inner).0)
}

pub fn dated_panel(source: &Source, load: &LoadContext, asked: &Value) -> Result<Box<dyn Panel>, String> {
    let branch = dashboard_query::branch(asked).map_err(|error| error.to_string())?;
    let query = crate::queries::query(source, branch).map_err(|error| error.to_string())?;
    let mut request = asked.clone();
    if let Some(fields) = request.as_object_mut() {
        fields.remove("panel");
    }
    let (parsed, _) = parse_params(&query, &request, false).map_err(|error| error.to_string())?;
    let parsed = Arc::new(parsed);
    let index = branch_index(source, &query);
    let marks = marks_cache(source);

    let advance = super::at_hour::shared_reader(source, load, PRODUCT, asked, || {
        make_reader(source, Arc::clone(&parsed), Arc::clone(&index), Arc::clone(&marks), query.holders.clone(), query.timeline.clone())
    });

    let (replay, holders, timeline) = (source.replay(), query.holders, query.timeline);
    let cold_parsed = Arc::clone(&parsed);
    let cold_marks = Arc::clone(&marks);
    let cold: dashboard_follow::panel::ExactReader = Arc::new(move |hour, cancelled| {
        if cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Err("Dashboard request cancelled".to_owned());
        }
        let query = Query { replay: &replay, timeline: Arc::clone(&timeline), holders: Arc::clone(&holders) };
        let line = Line::new(&query, cold_parsed.series, cold_parsed.measure).map_err(|error| error.to_string())?;
        let (reply, _) = compute_cold_counted(&line, &cold_parsed, hour, Some(&cold_marks)).map_err(|error| error.to_string())?;
        serde_json::to_value(reply).map_err(|error| error.to_string())
    });

    Ok(super::at_hour::product_panel(advance, cold))
}

fn make_reader(
    source: &Source,
    parsed: Arc<ParsedHeadParams>,
    index: Arc<BranchHeadIndex>,
    marks: Arc<MarkPrefixIndex>,
    holders: Arc<dashboard_query::identity::Holders>,
    timeline: Arc<dashboard_query::timeline::Timeline>,
) -> ProductReader {
    let replay = source.replay();
    let engine = Mutex::new(Engine::new(parsed, index, marks));
    Arc::new(move |hour, _values, cancelled, step| {
        if cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return ProductRead {
                value: Arc::new(Err("Dashboard request cancelled".to_owned())),
                rows: 0,
                fresh: false,
                pending: false,
                live: None,
                lanes: Vec::new(),
            };
        }
        let query = Query { replay: &replay, timeline: Arc::clone(&timeline), holders: Arc::clone(&holders) };
        let mut guard = engine.lock().unwrap_or_else(PoisonError::into_inner);
        guard.read(&query, hour, step)
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CellEval {
    Value(i64),
    NotApplicable(&'static str),
    Unrecorded,
}

impl CellEval {
    fn value(self) -> Option<i64> {
        match self {
            Self::Value(v) => Some(v),
            Self::NotApplicable(_) | Self::Unrecorded => None,
        }
    }
}

struct RowInputs<'a> {
    columns: &'a [usize],
    values: &'a [Option<i64>],
}

impl Cells for RowInputs<'_> {
    fn number(&self, column: usize, _slot: u32) -> Option<i64> {
        let pos = self.columns.iter().position(|&c| c == column)?;
        self.values[pos]
    }

    fn key(&self, _slot: u32) -> Option<u128> {
        Some(0)
    }

    fn slot(&self, _key: u128) -> Option<u32> {
        Some(0)
    }

    fn each_row(&self, each: &mut dyn FnMut(u32)) {
        each(0);
    }
}

fn eval_inputs(source: ColumnSource, columns: &[usize], values: &[Option<i64>]) -> CellEval {
    let row = RowInputs { columns, values };
    match source.read(&row, 0) {
        Computed::Value(v) => CellEval::Value(v),
        Computed::NotApplicable(reason) => CellEval::NotApplicable(reason),
        Computed::Unrecorded => CellEval::Unrecorded,
    }
}

fn cell_number(value: CellValue<'_>) -> Option<i64> {
    match value {
        CellValue::Number(bits) => Some(bits as i64),
        CellValue::Missing | CellValue::Unrecorded | CellValue::Text(_) | CellValue::List(_) => None,
    }
}

fn read_slot_inputs(cells: &replay_columnar::store::TableState, slot: u32, columns: &[usize]) -> Vec<Option<i64>> {
    columns.iter().map(|&col| cell_number(cells.value(col, slot))).collect()
}

fn read_written_at(written: &Written, idx: usize) -> Option<i64> {
    cell_number(written.value(idx))
}

#[derive(Clone, Debug)]
struct SelectedLineState {
    ask: Ask,
    part_text: Option<String>,
    current_key: Option<u128>,
    current_slot: Option<u32>,
    holder_starts: Option<i32>,
    raw_inputs: Vec<Option<i64>>,
    raw_cell: CellEval,
    acc: OpenBinAcc,
    marks: Option<(u64, u64)>,
    missing: Vec<dashboard_query::series::MissingSpan>,
    continuous_in_bin: bool,
}

#[derive(Clone, Debug)]
struct SelectedState {
    run: u32,
    hour: i32,
    bin_first: i32,
    lines: Vec<SelectedLineState>,
    edges: Vec<i64>,
    watched_keys: HashMap<u128, Vec<usize>>,
    watched_slots: HashMap<u32, Vec<usize>>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum RankKey {
    Most(Reverse<i64>, String),
    Least(i64, String),
}

impl RankKey {
    fn new(value: i64, tag: String, least: bool) -> Self {
        if least { Self::Least(value, tag) } else { Self::Most(Reverse(value), tag) }
    }

    fn parts(&self) -> (i64, &str) {
        match self {
            Self::Most(Reverse(v), tag) => (*v, tag.as_str()),
            Self::Least(v, tag) => (*v, tag.as_str()),
        }
    }
}

#[derive(Clone, Debug)]
struct RankedTagState {
    current_key: Option<u128>,
    current_eval: CellEval,
    ranked_value: Option<i64>,
}

#[derive(Clone, Debug)]
struct RankedOpenLine {
    acc: OpenBinAcc,
    marks: Option<(u64, u64)>,
    missing: Vec<dashboard_query::series::MissingSpan>,
    continuous_in_bin: bool,
    last_hour: i32,
    last_val: Option<i64>,
}

#[derive(Clone, Debug)]
struct RankedState {
    run: u32,
    hour: i32,
    bin_first: i32,
    slot_to_key: HashMap<u32, u128>,
    rows_by_key: HashMap<u128, Vec<Option<i64>>>,
    eval_by_key: HashMap<u128, CellEval>,
    key_to_tags: HashMap<u128, Vec<String>>,
    tags: HashMap<String, RankedTagState>,
    ranked: BTreeSet<RankKey>,
    open_bins: HashMap<String, RankedOpenLine>,
}

enum IncrementalState {
    Selected(SelectedState),
    Ranked(RankedState),
}

pub struct Engine {
    parsed: Arc<ParsedHeadParams>,
    index: Arc<BranchHeadIndex>,
    marks: Arc<MarkPrefixIndex>,
    columns: Vec<usize>,
    source: Option<ColumnSource>,
    measure_index: Option<u32>,
    lanes: Vec<Declared>,
    state: Option<IncrementalState>,
    cached: Option<(i32, Arc<PanelData>)>,
}

impl Engine {
    pub fn new(parsed: Arc<ParsedHeadParams>, index: Arc<BranchHeadIndex>, marks: Arc<MarkPrefixIndex>) -> Self {
        let source = dashboard_declare::source(parsed.series.table, parsed.measure).ok();
        let columns = source.map(|s| read::columns_of(&s)).unwrap_or_default();
        let measure_index = parsed.series.measures.iter().position(|m| std::ptr::eq(m, parsed.measure)).map(|at| at as u32);
        let mut lanes = vec![Declared {
            table: parsed.series.table.index(),
            columns: columns.clone(),
            keyed: true,
            passing: false,
            hourly: true,
        }];
        if parsed.with_marks && parsed.series.drops {
            lanes.push(Declared {
                table: TableId::BalanceDrops.index(),
                columns: Vec::new(),
                keyed: true,
                passing: false,
                hourly: false,
            });
        }
        Self { parsed, index, marks, columns, source, measure_index, lanes, state: None, cached: None }
    }

    pub fn read(&mut self, query: &Query<'_>, hour: i32, step: Option<ProductStep<'_>>) -> ProductRead {
        let reset = step.is_some_and(|ps| ps.step.reset());
        if !reset
            && let Some((cached_hour, value)) = &self.cached
            && *cached_hour == hour
            && self.state.is_some()
        {
            return ProductRead {
                value: Arc::clone(value),
                rows: 0,
                fresh: false,
                pending: false,
                live: None,
                lanes: self.lanes.clone(),
            };
        }

        let line = match Line::new(query, self.parsed.series, self.parsed.measure) {
            Ok(line) => line,
            Err(error) => {
                self.state = None;
                self.cached = None;
                return ProductRead {
                    value: Arc::new(Err(error.to_string())),
                    rows: 0,
                    fresh: true,
                    pending: false,
                    live: None,
                    lanes: self.lanes.clone(),
                };
            }
        };

        if !reset && let Some(ps) = step {
            if let Some(advanced) = self.try_advance(&line, hour, ps) {
                match advanced {
                    Ok((reply, rows)) => {
                        let encoded = Arc::new(serde_json::to_value(reply).map_err(|error| error.to_string()));
                        self.cached = Some((hour, Arc::clone(&encoded)));
                        return ProductRead {
                            value: encoded,
                            rows,
                            fresh: true,
                            pending: false,
                            live: None,
                            lanes: self.lanes.clone(),
                        };
                    }
                    Err(error) => {
                        self.state = None;
                        self.cached = None;
                        return ProductRead {
                            value: Arc::new(Err(error.to_string())),
                            rows: 0,
                            fresh: true,
                            pending: false,
                            live: None,
                            lanes: self.lanes.clone(),
                        };
                    }
                }
            }
        }

        let (result, rows) = self.seed_cold(&line, hour, step);
        let value = Arc::new(result);
        if value.is_ok() {
            self.cached = Some((hour, Arc::clone(&value)));
        } else {
            self.cached = None;
        }
        ProductRead {
            value,
            rows,
            fresh: true,
            pending: false,
            live: None,
            lanes: self.lanes.clone(),
        }
    }
}

fn lane_ready<'s>(
    parsed: &ParsedHeadParams,
    columns: &[usize],
    line: &Line<'_, '_>,
    prev_run: u32,
    prev_hour: i32,
    hour: i32,
    ps: ProductStep<'s>,
) -> Option<(&'s playhead::lane::Lane, &'s Changes)> {
    if ps.step.reset() || ps.step.last() != hour || i64::from(ps.step.first()) != i64::from(prev_hour) + 1 {
        return None;
    }
    let stretch = line.query().timeline.at(hour)?;
    if stretch.run != prev_run || stretch.first > prev_hour {
        return None;
    }
    if line.withheld_span(stretch.run, (i64::from(prev_hour) + 1) as i32, hour).is_some() {
        return None;
    }
    let table = parsed.series.table.index();
    let lane = ps.lanes.get(table)?;
    let change = ps.step.table(table)?;
    if !lane.keyed()
        || !columns.iter().all(|c| lane.requested().contains(c))
        || change.head() != Some(hour)
        || change.reset()
        || change.short() > 0
        || change.log().is_some()
        || change.status().is_some()
        || change.status_left().is_some()
        || lane.status().0 != Status::Complete
    {
        return None;
    }
    if ps.step.first() < ps.step.last() && !change.hourly() {
        return None;
    }
    Some((lane, change))
}

impl Engine {

    fn try_advance(&mut self, line: &Line<'_, '_>, hour: i32, ps: ProductStep<'_>) -> Option<io::Result<(dashboard_query::head::SeriesHeadReply, u64)>> {
        let source = self.source?;
        let parsed = &self.parsed;
        let columns = &self.columns;
        let measure_index = self.measure_index;
        let index = &self.index;
        let marks = &self.marks;
        match self.state.as_mut()? {
            IncrementalState::Selected(state) => {
                if hour <= state.hour {
                    return None;
                }
                let (lane, change) = lane_ready(parsed, columns, line, state.run, state.hour, hour, ps)?;
                let (bin_first, bin_last, closed_through) = match bin_bounds(hour, parsed.bin_hours) {
                    Ok(bounds) => bounds,
                    Err(error) => return Some(Err(error)),
                };
                let from_hour = (i64::from(state.hour) + 1) as i32;
                let hour_slices: &[Changes] = if ps.step.first() == ps.step.last() {
                    std::slice::from_ref(change)
                } else {
                    ps.step.hours_of(parsed.series.table.index())
                };
                let edge_start = state.edges.partition_point(|&e| e < i64::from(from_hour));
                let edge_end = edge_start + state.edges[edge_start..].partition_point(|&e| e <= i64::from(hour));
                let step_edges = &state.edges[edge_start..edge_end];
                let has_step_edges = !step_edges.is_empty();
                let query = line.query();

                for &edge in step_edges {
                    let eh = edge as i32;
                    for ls in &state.lines {
                        let new_key = holder_key_of(query, parsed.series, &ls.ask, eh).map(|(k, _)| k);
                        if new_key != ls.current_key && eh < hour {
                            return None;
                        }
                    }
                }

                let mut rows_visited = 0u64;
                let scale = line.scale();
                let mut cursor = from_hour;
                let mut slice_idx = 0usize;
                let mut edge_idx = 0usize;
                let old_bin_first = state.bin_first;
                let crossed_bin = bin_first != old_bin_first;

                if crossed_bin {
                    for ls in &mut state.lines {
                        ls.continuous_in_bin = true;
                        ls.missing.clear();
                    }
                }

                while slice_idx < hour_slices.len() || edge_idx < step_edges.len() {
                    let next_slice_h = hour_slices.get(slice_idx).and_then(Changes::head);
                    let next_edge_h = step_edges.get(edge_idx).map(|&e| e as i32);
                    let ev_h = match (next_slice_h, next_edge_h) {
                        (Some(a), Some(b)) => a.min(b),
                        (Some(a), None) => a,
                        (None, Some(b)) => b,
                        (None, None) => break,
                    };
                    if ev_h > cursor {
                        let quiet_to = (i64::from(ev_h) - 1) as i32;
                        for ls in &mut state.lines {
                            let val = if parsed.within_contains(&ls.ask.country, cursor) && ls.current_key.is_some() {
                                ls.raw_cell.value()
                            } else {
                                None
                            };
                            if val.is_none() && quiet_to >= bin_first {
                                ls.continuous_in_bin = false;
                            }
                            ls.acc.push_span(cursor, quiet_to, val, false, scale);
                        }
                    }

                    let mut is_edge = false;
                    if next_edge_h == Some(ev_h) {
                        is_edge = true;
                        edge_idx += 1;
                    }
                    let mut hc_opt: Option<&Changes> = None;
                    if next_slice_h == Some(ev_h) {
                        hc_opt = Some(&hour_slices[slice_idx]);
                        slice_idx += 1;
                    }

                    if is_edge {
                        let mut rebind_needed = false;
                        for (li, ls) in state.lines.iter_mut().enumerate() {
                            let resolved = holder_key_of(query, parsed.series, &ls.ask, ev_h);
                            let new_key = resolved.map(|(k, _)| k);
                            let new_starts = resolved.and_then(|(_, s)| s);
                            ls.holder_starts = new_starts;
                            if new_key != ls.current_key {
                                rebind_needed = true;
                                ls.current_key = new_key;
                                ls.current_slot = new_key.and_then(|k| lane.cells().find(k));
                                if let Some(slot) = ls.current_slot {
                                    rows_visited += 1;
                                    ls.raw_inputs = read_slot_inputs(lane.cells(), slot, columns);
                                    ls.raw_cell = eval_inputs(source, columns, &ls.raw_inputs);
                                } else {
                                    ls.raw_inputs.fill(None);
                                    ls.raw_cell = if parsed.series.absent_none && new_key.is_some() {
                                        CellEval::Value(0)
                                    } else {
                                        CellEval::Unrecorded
                                    };
                                }
                                let _ = li;
                            }
                        }
                        if rebind_needed {
                            rebuild_selected_watches(&state.lines, &mut state.watched_keys, &mut state.watched_slots);
                        }
                    }

                    if let Some(hc) = hc_opt {
                        let mut dirty_lines: Vec<usize> = Vec::new();
                        let mut rebind_slots = false;
                        if !hc.deleted().is_empty() {
                            rows_visited += hc.deleted().len() as u64;
                            if let Some(keys) = hc.deleted().keys() {
                                for &key in keys {
                                    if let Some(indices) = state.watched_keys.get(&key) {
                                        for &li in indices {
                                            let ls = &mut state.lines[li];
                                            ls.current_slot = None;
                                            ls.raw_inputs.fill(None);
                                            ls.raw_cell = if parsed.series.absent_none { CellEval::Value(0) } else { CellEval::Unrecorded };
                                            rebind_slots = true;
                                        }
                                    }
                                }
                            }
                        }
                        if !hc.inserted().is_empty() {
                            rows_visited += hc.inserted().len() as u64;
                            if let Some(keys) = hc.inserted().keys() {
                                for (&slot, &key) in hc.inserted().slots().iter().zip(keys) {
                                    if let Some(indices) = state.watched_keys.get(&key) {
                                        for &li in indices {
                                            let ls = &mut state.lines[li];
                                            ls.current_slot = Some(slot);
                                            ls.raw_inputs.fill(None);
                                            rebind_slots = true;
                                            dirty_lines.push(li);
                                        }
                                    }
                                }
                            }
                        }
                        if rebind_slots {
                            rebuild_selected_watches(&state.lines, &mut state.watched_keys, &mut state.watched_slots);
                        }
                        for written in hc.columns() {
                            let Some(col_pos) = columns.iter().position(|&c| c == written.column()) else { continue };
                            if written.is_empty() {
                                continue;
                            }
                            if state.watched_slots.len() <= written.len() {
                                for (&slot, indices) in &state.watched_slots {
                                    if let Ok(at) = written.slots().binary_search(&slot) {
                                        rows_visited += 1;
                                        let v = read_written_at(written, at);
                                        for &li in indices {
                                            state.lines[li].raw_inputs[col_pos] = v;
                                            dirty_lines.push(li);
                                        }
                                    }
                                }
                            } else {
                                rows_visited += written.len() as u64;
                                for (at, &slot) in written.slots().iter().enumerate() {
                                    if let Some(indices) = state.watched_slots.get(&slot) {
                                        let v = read_written_at(written, at);
                                        for &li in indices {
                                            state.lines[li].raw_inputs[col_pos] = v;
                                            dirty_lines.push(li);
                                        }
                                    }
                                }
                            }
                        }
                        dirty_lines.sort_unstable();
                        dirty_lines.dedup();
                        for li in dirty_lines {
                            let ls = &mut state.lines[li];
                            ls.raw_cell = if ls.current_slot.is_some() {
                                eval_inputs(source, columns, &ls.raw_inputs)
                            } else if parsed.series.absent_none && ls.current_key.is_some() {
                                CellEval::Value(0)
                            } else {
                                CellEval::Unrecorded
                            };
                        }
                    }

                    for ls in &mut state.lines {
                        let starts = is_edge && ls.holder_starts == Some(ev_h);
                        let val = if parsed.within_contains(&ls.ask.country, ev_h) && ls.current_key.is_some() {
                            ls.raw_cell.value()
                        } else {
                            None
                        };
                        if val.is_none() && ev_h >= bin_first {
                            ls.continuous_in_bin = false;
                        }
                        ls.acc.push_span(ev_h, ev_h, val, starts, scale);
                    }
                    cursor = (i64::from(ev_h) + 1) as i32;
                }

                if cursor <= hour {
                    for ls in &mut state.lines {
                        let val = if parsed.within_contains(&ls.ask.country, cursor) && ls.current_key.is_some() {
                            ls.raw_cell.value()
                        } else {
                            None
                        };
                        if val.is_none() && hour >= bin_first {
                            ls.continuous_in_bin = false;
                        }
                        ls.acc.push_span(cursor, hour, val, false, scale);
                    }
                }

                if let Some(old_slots) = change.renumber() {
                    let mut moved = false;
                    for ls in &mut state.lines {
                        if let Some(slot) = ls.current_slot {
                            ls.current_slot = old_slots.binary_search(&slot).ok().map(|at| at as u32);
                            moved = true;
                        }
                    }
                    if moved {
                        rebuild_selected_watches(&state.lines, &mut state.watched_keys, &mut state.watched_slots);
                    }
                }

                if parsed.with_marks && parsed.series.drops {
                    let drop_applied = apply_selected_marks_step(parsed, measure_index, line, state, from_hour, hour, old_bin_first, bin_first, ps, &mut rows_visited);
                    if !drop_applied {
                        if let Err(error) = refresh_selected_marks_cold(marks, line, state, bin_first, hour) {
                            return Some(Err(error));
                        }
                    }
                }

                let need_missing = has_step_edges || crossed_bin;
                let mut reply_lines = Vec::with_capacity(state.lines.len());
                for ls in &mut state.lines {
                    if need_missing || !ls.continuous_in_bin || !ls.missing.is_empty() {
                        let parts = line.held(&ls.ask, bin_first, hour);
                        match coverage_missing(line, &ls.ask.country, bin_first, hour, &parts) {
                            Ok(missing) => ls.missing = missing,
                            Err(error) => return Some(Err(error)),
                        }
                    }
                    let head_value = build_head_value_complete(line, parsed, &ls.ask, hour, ls.current_key.is_some(), ls.raw_cell, scale);
                    reply_lines.push(HeadLine {
                        country: ls.ask.country.clone(),
                        part: ls.part_text.clone(),
                        open_bin: ls.acc.bin.clone(),
                        value: head_value,
                        missing: ls.missing.clone(),
                        marks: ls.marks.map(|(before, count)| HeadMarks { before: before.to_string(), count }),
                    });
                }

                state.hour = hour;
                state.bin_first = bin_first;
                Some(Ok((
                    dashboard_query::head::SeriesHeadReply {
                        branch_id: parsed.branch_id.clone(),
                        measure: line.measure_name().to_owned(),
                        decimals: line.decimals(),
                        hour,
                        bin_hours: parsed.bin_hours,
                        bin_first_hour: bin_first,
                        bin_last_hour: bin_last,
                        closed_through,
                        lines: reply_lines,
                        roster: None,
                    },
                    rows_visited,
                )))
            }
            IncrementalState::Ranked(state) => {
                if hour <= state.hour {
                    return None;
                }
                let (_lane, change) = lane_ready(parsed, columns, line, state.run, state.hour, hour, ps)?;
                let (bin_first, bin_last, closed_through) = match bin_bounds(hour, parsed.bin_hours) {
                    Ok(bounds) => bounds,
                    Err(error) => return Some(Err(error)),
                };
                let from_hour = (i64::from(state.hour) + 1) as i32;
                let hour_slices: &[Changes] = if ps.step.first() == ps.step.last() {
                    std::slice::from_ref(change)
                } else {
                    ps.step.hours_of(parsed.series.table.index())
                };
                let holder_changes = index.holder_changes_in(from_hour, hour);
                let query = line.query();
                let scale = line.scale();
                let least = parsed.least;
                let absent_none = parsed.series.absent_none;
                let old_bin_first = state.bin_first;
                let crossed_bin = bin_first != old_bin_first;
                let mut rows_visited = 0u64;

                if crossed_bin {
                    for open in state.open_bins.values_mut() {
                        open.continuous_in_bin = true;
                        open.missing.clear();
                    }
                }

                let mut slice_idx = 0usize;
                let mut hc_idx = 0usize;
                while slice_idx < hour_slices.len() || hc_idx < holder_changes.len() {
                    let next_slice_h = hour_slices.get(slice_idx).and_then(Changes::head);
                    let next_hc_h = holder_changes.get(hc_idx).map(|(h, _)| *h);
                    let ev_h = match (next_slice_h, next_hc_h) {
                        (Some(a), Some(b)) => a.min(b),
                        (Some(a), None) => a,
                        (None, Some(b)) => b,
                        (None, None) => break,
                    };

                    let mut touched_open: BTreeMap<String, bool> = BTreeMap::new();

                    if next_slice_h == Some(ev_h) {
                        let hc = &hour_slices[slice_idx];
                        slice_idx += 1;
                        let mut dirty_keys: Vec<u128> = Vec::new();
                        if !hc.deleted().is_empty() {
                            rows_visited += hc.deleted().len() as u64;
                            if let Some(keys) = hc.deleted().keys() {
                                for (&slot, &key) in hc.deleted().slots().iter().zip(keys) {
                                    state.slot_to_key.remove(&slot);
                                    state.rows_by_key.remove(&key);
                                    state.eval_by_key.remove(&key);
                                    dirty_keys.push(key);
                                }
                            }
                        }
                        if !hc.inserted().is_empty() {
                            rows_visited += hc.inserted().len() as u64;
                            if let Some(keys) = hc.inserted().keys() {
                                for (&slot, &key) in hc.inserted().slots().iter().zip(keys) {
                                    state.slot_to_key.insert(slot, key);
                                    state.rows_by_key.insert(key, vec![None; columns.len()]);
                                    dirty_keys.push(key);
                                }
                            }
                        }
                        for written in hc.columns() {
                            let Some(col_pos) = columns.iter().position(|&c| c == written.column()) else { continue };
                            if written.is_empty() {
                                continue;
                            }
                            rows_visited += written.len() as u64;
                            for (at, &slot) in written.slots().iter().enumerate() {
                                if let Some(&key) = state.slot_to_key.get(&slot) {
                                    let inputs = state.rows_by_key.entry(key).or_insert_with(|| vec![None; columns.len()]);
                                    inputs[col_pos] = read_written_at(written, at);
                                    dirty_keys.push(key);
                                }
                            }
                        }
                        dirty_keys.sort_unstable();
                        dirty_keys.dedup();
                        for key in dirty_keys {
                            let new_eval = match state.rows_by_key.get(&key) {
                                Some(inputs) => {
                                    let ev = eval_inputs(source, columns, inputs);
                                    state.eval_by_key.insert(key, ev);
                                    ev
                                }
                                None => {
                                    state.eval_by_key.remove(&key);
                                    if absent_none { CellEval::Value(0) } else { CellEval::Unrecorded }
                                }
                            };
                            let new_ranked = new_eval.value();
                            if let Some(tags) = state.key_to_tags.get(&key) {
                                for tag in tags {
                                    if let Some(ts) = state.tags.get_mut(tag) {
                                        if ts.ranked_value != new_ranked {
                                            if let Some(old_v) = ts.ranked_value {
                                                state.ranked.remove(&RankKey::new(old_v, tag.clone(), least));
                                            }
                                            if let Some(v) = new_ranked {
                                                state.ranked.insert(RankKey::new(v, tag.clone(), least));
                                            }
                                            ts.ranked_value = new_ranked;
                                        }
                                        ts.current_eval = new_eval;
                                    }
                                    if state.open_bins.contains_key(tag) {
                                        touched_open.entry(tag.clone()).or_insert(false);
                                    }
                                }
                            }
                        }
                    }

                    if next_hc_h == Some(ev_h) {
                        let (_, changed_tags) = &holder_changes[hc_idx];
                        hc_idx += 1;
                        for tag in changed_tags {
                            let new_holder = query.holders.at(tag, ev_h);
                            let new_key = new_holder.and_then(|run| match run.holder {
                                Holder::Found { slot, id } => Some(parsed.series.key.of(slot, id)),
                                Holder::Unresolved => None,
                            });
                            let starts = new_holder.is_some_and(|run| run.first == ev_h && run.starts_segment);
                            if let Some(old_ts) = state.tags.get(tag) {
                                if let Some(old_k) = old_ts.current_key
                                    && let Some(list) = state.key_to_tags.get_mut(&old_k)
                                {
                                    list.retain(|t| t != tag);
                                    if list.is_empty() {
                                        state.key_to_tags.remove(&old_k);
                                    }
                                }
                                if let Some(old_v) = old_ts.ranked_value {
                                    state.ranked.remove(&RankKey::new(old_v, tag.clone(), least));
                                }
                            }
                            let new_eval = match new_key {
                                Some(k) => state.eval_by_key.get(&k).copied().unwrap_or(if absent_none { CellEval::Value(0) } else { CellEval::Unrecorded }),
                                None => CellEval::Unrecorded,
                            };
                            let new_ranked = new_eval.value();
                            if let Some(k) = new_key {
                                state.key_to_tags.entry(k).or_default().push(tag.clone());
                            }
                            if let Some(v) = new_ranked {
                                state.ranked.insert(RankKey::new(v, tag.clone(), least));
                            }
                            state.tags.insert(tag.clone(), RankedTagState { current_key: new_key, current_eval: new_eval, ranked_value: new_ranked });
                            if state.open_bins.contains_key(tag) {
                                let entry = touched_open.entry(tag.clone()).or_insert(false);
                                *entry |= starts;
                            }
                        }
                    }

                    for (tag, starts) in touched_open {
                        let new_ranked = state.tags.get(&tag).and_then(|ts| ts.ranked_value);
                        if let Some(open) = state.open_bins.get_mut(&tag) {
                            advance_ranked_open(open, ev_h, new_ranked, starts, bin_first, scale);
                        }
                    }
                }

                if let Some(old_slots) = change.renumber() {
                    let mut remapped = HashMap::with_capacity(state.slot_to_key.len());
                    for (&old_slot, &key) in &state.slot_to_key {
                        if let Ok(new_slot) = old_slots.binary_search(&old_slot) {
                            remapped.insert(new_slot as u32, key);
                        }
                    }
                    state.slot_to_key = remapped;
                }

                for (tag, open) in &mut state.open_bins {
                    if open.last_hour < hour {
                        let start = (i64::from(open.last_hour) + 1) as i32;
                        let val = state.tags.get(tag).and_then(|ts| ts.ranked_value);
                        if val.is_none() && hour >= bin_first {
                            open.continuous_in_bin = false;
                        }
                        open.acc.push_span(start, hour, val, false, scale);
                        open.last_hour = hour;
                        open.last_val = val;
                    }
                }

                if parsed.with_marks && parsed.series.drops {
                    let drop_applied = apply_ranked_marks_step(parsed, measure_index, line, state, from_hour, hour, old_bin_first, bin_first, ps, &mut rows_visited);
                    if !drop_applied {
                        for (tag, open) in &mut state.open_bins {
                            match compute_marks_cold(line, &Country::from(tag.as_str()), bin_first, hour, marks) {
                                Ok(m) => open.marks = Some(m),
                                Err(error) => return Some(Err(error)),
                            }
                        }
                    }
                }

                let top = parsed.top.expect("ranked") as usize;
                let top_entries: Vec<(i64, String)> = state.ranked.iter().take(top).map(|rk| {
                    let (v, tag) = rk.parts();
                    (v, tag.to_owned())
                }).collect();

                let mut roster_entries = Vec::with_capacity(top_entries.len());
                let mut reply_lines = Vec::with_capacity(top_entries.len());
                let mut keep_tags: Vec<String> = Vec::with_capacity(top_entries.len());

                for (raw, tag) in top_entries {
                    keep_tags.push(tag.clone());
                    roster_entries.push(HeadRosterEntry {
                        country: Country::from(tag.as_str()),
                        value_raw: raw.to_string(),
                        value: raw as f64 / scale,
                    });
                    if !state.open_bins.contains_key(&tag) {
                        let ask = Ask::whole(Country::from(tag.as_str()));
                        match compute_single_line_cold(line, parsed, &ask, hour, Some(marks)) {
                            Ok((hl, acc, backfill_rows)) => {
                                rows_visited += backfill_rows;
                                let marks_val = hl.marks.as_ref().map(|m| (m.before.parse::<u64>().unwrap_or(0), m.count));
                                state.open_bins.insert(tag.clone(), RankedOpenLine {
                                    acc,
                                    marks: marks_val,
                                    missing: hl.missing,
                                    continuous_in_bin: false,
                                    last_hour: hour,
                                    last_val: Some(raw),
                                });
                            }
                            Err(error) => return Some(Err(error)),
                        }
                    }
                    let open = state.open_bins.get_mut(&tag).expect("present");
                    if crossed_bin || !holder_changes.is_empty() || !open.continuous_in_bin || !open.missing.is_empty() {
                        let ask = Ask::whole(Country::from(tag.as_str()));
                        let parts = line.held(&ask, bin_first, hour);
                        match coverage_missing(line, &ask.country, bin_first, hour, &parts) {
                            Ok(missing) => open.missing = missing,
                            Err(error) => return Some(Err(error)),
                        }
                    }
                    reply_lines.push(HeadLine {
                        country: Country::from(tag.as_str()),
                        part: None,
                        open_bin: open.acc.bin.clone(),
                        value: HeadValue {
                            value_raw: Some(raw.to_string()),
                            value: Some(raw as f64 / scale),
                            missing: None,
                            not_applicable: None,
                        },
                        missing: open.missing.clone(),
                        marks: open.marks.map(|(before, count)| HeadMarks { before: before.to_string(), count }),
                    });
                }

                let warm_cap = (top * 4).max(16);
                if state.open_bins.len() > warm_cap {
                    let mut removable: Vec<String> = state.open_bins.keys().filter(|t| !keep_tags.contains(t)).cloned().collect();
                    let drop_count = state.open_bins.len().saturating_sub(warm_cap);
                    for tag in removable.drain(..drop_count.min(removable.len())) {
                        state.open_bins.remove(&tag);
                    }
                }

                state.hour = hour;
                state.bin_first = bin_first;
                Some(Ok((
                    dashboard_query::head::SeriesHeadReply {
                        branch_id: parsed.branch_id.clone(),
                        measure: line.measure_name().to_owned(),
                        decimals: line.decimals(),
                        hour,
                        bin_hours: parsed.bin_hours,
                        bin_first_hour: bin_first,
                        bin_last_hour: bin_last,
                        closed_through,
                        lines: reply_lines,
                        roster: Some(HeadRoster {
                            top: top as u32,
                            least,
                            entries: roster_entries,
                        }),
                    },
                    rows_visited,
                )))
            }
        }
    }

}

#[allow(clippy::too_many_arguments, reason = "incremental mark update across step range and bin boundaries")]
fn apply_selected_marks_step(
    parsed: &ParsedHeadParams,
    measure_index: Option<u32>,
    line: &Line<'_, '_>,
    state: &mut SelectedState,
    from_hour: i32,
    to_hour: i32,
    old_bin_first: i32,
    new_bin_first: i32,
    ps: ProductStep<'_>,
    rows_visited: &mut u64,
) -> bool {
    let Some(measure_idx) = measure_index else { return false };
    let Some(drop_change) = ps.step.table(TableId::BalanceDrops.index()) else { return false };
    if drop_change.reset() {
        return false;
    }
    let Some(logged) = drop_change.log() else { return false };
    let Some(keys) = logged.keys() else { return false };
    if new_bin_first > old_bin_first {
        for ls in &mut state.lines {
            if let Some((before, count)) = &mut ls.marks {
                *before += *count;
                *count = 0;
            }
        }
    }
    *rows_visited += logged.len() as u64;
    let query = line.query();
    for (&drop_hour, &key) in logged.hours().iter().zip(keys) {
        if drop_hour < from_hour || drop_hour > to_hour {
            continue;
        }
        if ((key >> 64) as u32) != measure_idx {
            continue;
        }
        let drop_country = (key >> 32) as u32;
        for ls in &mut state.lines {
            let held_key = holder_key_of(query, parsed.series, &ls.ask, drop_hour).map(|(k, _)| k as u32);
            if held_key == Some(drop_country)
                && let Some((before, count)) = &mut ls.marks
            {
                if drop_hour < new_bin_first {
                    *before += 1;
                } else {
                    *count += 1;
                }
            }
        }
    }
    true
}

fn refresh_selected_marks_cold(
    marks: &Arc<MarkPrefixIndex>,
    line: &Line<'_, '_>,
    state: &mut SelectedState,
    bin_first: i32,
    hour: i32,
) -> io::Result<()> {
    for ls in &mut state.lines {
        ls.marks = Some(compute_marks_cold(line, &ls.ask.country, bin_first, hour, marks)?);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments, reason = "incremental mark update for ranked lines across step range")]
fn apply_ranked_marks_step(
    parsed: &ParsedHeadParams,
    measure_index: Option<u32>,
    line: &Line<'_, '_>,
    state: &mut RankedState,
    from_hour: i32,
    to_hour: i32,
    old_bin_first: i32,
    new_bin_first: i32,
    ps: ProductStep<'_>,
    rows_visited: &mut u64,
) -> bool {
    let Some(measure_idx) = measure_index else { return false };
    let Some(drop_change) = ps.step.table(TableId::BalanceDrops.index()) else { return false };
    if drop_change.reset() {
        return false;
    }
    let Some(logged) = drop_change.log() else { return false };
    let Some(keys) = logged.keys() else { return false };
    if new_bin_first > old_bin_first {
        for open in state.open_bins.values_mut() {
            if let Some((before, count)) = &mut open.marks {
                *before += *count;
                *count = 0;
            }
        }
    }
    *rows_visited += logged.len() as u64;
    let query = line.query();
    for (&drop_hour, &key) in logged.hours().iter().zip(keys) {
        if drop_hour < from_hour || drop_hour > to_hour {
            continue;
        }
        if ((key >> 64) as u32) != measure_idx {
            continue;
        }
        let drop_country = (key >> 32) as u32;
        for (tag, open) in &mut state.open_bins {
            let held = query.holders.at(tag, drop_hour).and_then(|run| match run.holder {
                Holder::Found { slot, id } => Some(parsed.series.key.of(slot, id) as u32),
                Holder::Unresolved => None,
            });
            if held == Some(drop_country)
                && let Some((before, count)) = &mut open.marks
            {
                if drop_hour < new_bin_first {
                    *before += 1;
                } else {
                    *count += 1;
                }
            }
        }
    }
    true
}

impl Engine {

    fn seed_cold(&mut self, line: &Line<'_, '_>, hour: i32, step: Option<ProductStep<'_>>) -> (PanelData, u64) {
        self.state = None;
        let (reply, mut rows) = match compute_cold_counted(line, &self.parsed, hour, Some(&self.marks)) {
            Ok(pair) => pair,
            Err(error) => return (Err(error.to_string()), 0),
        };
        let encoded = match serde_json::to_value(&reply) {
            Ok(value) => value,
            Err(error) => return (Err(error.to_string()), rows),
        };
        if let (Some(source), Some(ps), Some(stretch)) = (self.source, step, line.query().timeline.at(hour))
            && line.withheld_at(stretch.run, hour).is_none()
            && let Some(lane) = ps.lanes.get(self.parsed.series.table.index())
            && lane.keyed()
            && self.columns.iter().all(|c| lane.requested().contains(c))
            && lane.head() == Some(hour)
            && lane.status().0 == Status::Complete
        {
            let query = line.query();
            let cells = lane.cells();
            let bin_first = reply.bin_first_hour;
            if self.parsed.top.is_none() {
                let mut edges: Vec<i64> = self.index.timeline_edges().to_vec();
                let mut lines_state = Vec::with_capacity(self.parsed.asks.len());
                for (ask, hl) in self.parsed.asks.iter().zip(&reply.lines) {
                    edges.extend_from_slice(&self.index.country_edges(&query.holders, &ask.country));
                    if let Some(within) = &self.parsed.within
                        && let Some(spans) = within.get(&ask.country)
                    {
                        for &(first, last) in spans {
                            edges.push(i64::from(first));
                            edges.push(i64::from(last) + 1);
                        }
                    }
                    let resolved = holder_key_of(query, self.parsed.series, ask, hour);
                    let current_key = resolved.map(|(k, _)| k);
                    let holder_starts = resolved.and_then(|(_, s)| s);
                    let current_slot = current_key.and_then(|k| cells.find(k));
                    let (raw_inputs, raw_cell) = match current_slot {
                        Some(slot) => {
                            rows += 1;
                            let inputs = read_slot_inputs(cells, slot, &self.columns);
                            let ev = eval_inputs(source, &self.columns, &inputs);
                            (inputs, ev)
                        }
                        None => (
                            vec![None; self.columns.len()],
                            if self.parsed.series.absent_none && current_key.is_some() { CellEval::Value(0) } else { CellEval::Unrecorded },
                        ),
                    };
                    let prev_present = hl.open_bin.as_ref().is_some_and(|b| b.last_hour == hour);
                    let prev_value = if prev_present { raw_cell.value() } else { None };
                    let acc = OpenBinAcc {
                        bin_first,
                        bin_hours: self.parsed.bin_hours,
                        prev_hour: Some(hour),
                        prev_present,
                        prev_value,
                        bin: hl.open_bin.clone(),
                    };
                    let marks = hl.marks.as_ref().map(|m| (m.before.parse::<u64>().unwrap_or(0), m.count));
                    let continuous_in_bin = hl.missing.is_empty() && prev_present;
                    lines_state.push(SelectedLineState {
                        ask: ask.clone(),
                        part_text: hl.part.clone(),
                        current_key,
                        current_slot,
                        holder_starts,
                        raw_inputs,
                        raw_cell,
                        acc,
                        marks,
                        missing: hl.missing.clone(),
                        continuous_in_bin,
                    });
                }
                edges.sort_unstable();
                edges.dedup();
                let mut watched_keys = HashMap::new();
                let mut watched_slots = HashMap::new();
                rebuild_selected_watches(&lines_state, &mut watched_keys, &mut watched_slots);
                self.state = Some(IncrementalState::Selected(SelectedState {
                    run: stretch.run,
                    hour,
                    bin_first,
                    lines: lines_state,
                    edges,
                    watched_keys,
                    watched_slots,
                }));
            } else {
                let mut slot_to_key = HashMap::new();
                let mut rows_by_key = HashMap::new();
                let mut eval_by_key = HashMap::new();
                for slot in cells.rows() {
                    rows += 1;
                    let Some(key) = cells.key(slot) else { continue };
                    let inputs = read_slot_inputs(cells, slot, &self.columns);
                    let ev = eval_inputs(source, &self.columns, &inputs);
                    slot_to_key.insert(slot, key);
                    rows_by_key.insert(key, inputs);
                    eval_by_key.insert(key, ev);
                }
                let mut key_to_tags: HashMap<u128, Vec<String>> = HashMap::new();
                let mut tags = HashMap::new();
                let mut ranked = BTreeSet::new();
                let least = self.parsed.least;
                let absent_none = self.parsed.series.absent_none;
                for (tag, runs) in &query.holders.tags {
                    let at = runs.partition_point(|run| run.last < hour);
                    let key = runs.get(at).filter(|run| run.first <= hour).and_then(|run| match run.holder {
                        Holder::Found { slot, id } => Some(self.parsed.series.key.of(slot, id)),
                        Holder::Unresolved => None,
                    });
                    let ev = match key {
                        Some(k) => eval_by_key.get(&k).copied().unwrap_or(if absent_none { CellEval::Value(0) } else { CellEval::Unrecorded }),
                        None => CellEval::Unrecorded,
                    };
                    let rv = ev.value();
                    if let Some(k) = key {
                        key_to_tags.entry(k).or_default().push(tag.clone());
                    }
                    if let Some(v) = rv {
                        ranked.insert(RankKey::new(v, tag.clone(), least));
                    }
                    tags.insert(tag.clone(), RankedTagState { current_key: key, current_eval: ev, ranked_value: rv });
                }
                let mut open_bins = HashMap::new();
                for hl in &reply.lines {
                    if let Some(tag) = hl.country.tag() {
                        let prev_present = hl.open_bin.as_ref().is_some_and(|b| b.last_hour == hour);
                        let raw_val = hl.value.value_raw.as_deref().and_then(|s| s.parse::<i64>().ok());
                        let acc = OpenBinAcc {
                            bin_first,
                            bin_hours: self.parsed.bin_hours,
                            prev_hour: Some(hour),
                            prev_present,
                            prev_value: if prev_present { raw_val } else { None },
                            bin: hl.open_bin.clone(),
                        };
                        let marks = hl.marks.as_ref().map(|m| (m.before.parse::<u64>().unwrap_or(0), m.count));
                        open_bins.insert(tag.to_owned(), RankedOpenLine {
                            acc,
                            marks,
                            missing: hl.missing.clone(),
                            continuous_in_bin: hl.missing.is_empty() && prev_present,
                            last_hour: hour,
                            last_val: raw_val,
                        });
                    }
                }
                self.state = Some(IncrementalState::Ranked(RankedState {
                    run: stretch.run,
                    hour,
                    bin_first,
                    slot_to_key,
                    rows_by_key,
                    eval_by_key,
                    key_to_tags,
                    tags,
                    ranked,
                    open_bins,
                }));
            }
        }
        (Ok(encoded), rows)
    }
}

fn advance_ranked_open(open: &mut RankedOpenLine, ev_h: i32, new_val: Option<i64>, starts: bool, bin_first: i32, scale: f64) {
    if ev_h > open.last_hour + 1 {
        let quiet_from = (i64::from(open.last_hour) + 1) as i32;
        let quiet_to = (i64::from(ev_h) - 1) as i32;
        if open.last_val.is_none() && quiet_to >= bin_first {
            open.continuous_in_bin = false;
        }
        open.acc.push_span(quiet_from, quiet_to, open.last_val, false, scale);
    }
    if ev_h > open.last_hour {
        if new_val.is_none() && ev_h >= bin_first {
            open.continuous_in_bin = false;
        }
        open.acc.push_span(ev_h, ev_h, new_val, starts, scale);
        open.last_hour = ev_h;
        open.last_val = new_val;
    } else if ev_h == open.last_hour && open.last_val != new_val {
        open.last_val = new_val;
    }
}

fn rebuild_selected_watches(
    lines: &[SelectedLineState],
    watched_keys: &mut HashMap<u128, Vec<usize>>,
    watched_slots: &mut HashMap<u32, Vec<usize>>,
) {
    watched_keys.clear();
    watched_slots.clear();
    for (idx, ls) in lines.iter().enumerate() {
        if let Some(key) = ls.current_key {
            watched_keys.entry(key).or_default().push(idx);
        }
        if let Some(slot) = ls.current_slot {
            watched_slots.entry(slot).or_default().push(idx);
        }
    }
}

fn holder_key_of(query: &Query<'_>, series: &'static dashboard_declare::Series, ask: &Ask, hour: i32) -> Option<(u128, Option<i32>)> {
    if series.key.unowned() {
        let key = ask.country.tag().and_then(|tag| series.key.fixed(tag))?;
        let stretch = query.timeline.at(hour)?;
        let starts = (stretch.first == hour && !matches!(stretch.join, dashboard_query::timeline::Join::Load { .. })).then_some(hour);
        return Some((key, starts));
    }
    if query.holders.unknown.iter().any(|&(first, last, _)| first <= hour && hour <= last) {
        return None;
    }
    let run = query.holders.at(&ask.country, hour)?;
    let Holder::Found { slot, id } = run.holder else {
        return None;
    };
    let key = series.row_key(slot, id, ask.part);
    let starts = (run.first == hour && run.starts_segment).then_some(hour);
    Some((key, starts))
}

fn build_head_value_complete(
    line: &Line<'_, '_>,
    parsed: &ParsedHeadParams,
    ask: &Ask,
    hour: i32,
    held: bool,
    cell: CellEval,
    scale: f64,
) -> HeadValue {
    if !parsed.within_contains(&ask.country, hour) {
        return HeadValue { value_raw: None, value: None, missing: None, not_applicable: None };
    }
    if !held {
        let reason = line.missing_at(&ask.country, hour).unwrap_or_else(|| line.absent(&ask.country, hour));
        return HeadValue { value_raw: None, value: None, missing: Some(reason), not_applicable: None };
    }
    match cell {
        CellEval::Value(raw) => HeadValue {
            value_raw: Some(raw.to_string()),
            value: Some(raw as f64 / scale),
            missing: None,
            not_applicable: None,
        },
        CellEval::NotApplicable(reason) => HeadValue {
            value_raw: None,
            value: None,
            missing: Some(reason.to_owned()),
            not_applicable: Some(true),
        },
        CellEval::Unrecorded => HeadValue {
            value_raw: None,
            value: None,
            missing: Some(board::measure_unrecorded(&line.query().holders.name(&ask.country, hour), line.measure_name())),
            not_applicable: matches!(line.source(), ColumnSource::Computed { .. }).then_some(false),
        },
    }
}

fn compute_marks_cold(line: &Line<'_, '_>, country: &Country, bin_first: i32, hour: i32, cache: &MarkPrefixIndex) -> io::Result<(u64, u64)> {
    let query = line.query();
    let before = if i64::from(bin_first) > i64::from(query.timeline.first) {
        cache.prefix_through(line, country, (i64::from(bin_first) - 1) as i32)?
    } else {
        0
    };
    let count = if hour >= query.timeline.first {
        line.drop_interval_count(country, bin_first.max(query.timeline.first), hour)?
    } else {
        0
    };
    if hour >= query.timeline.first {
        cache.record(query.timeline.id, line.series().kind, line.measure_name(), &query.holders, country, hour, before + count);
    }
    Ok((before, count))
}

#[cfg(test)]
mod tests;
