use super::*;
use dashboard_follow::panel::{ProductReader, ProductStep};
use replay::{cells::TableReader, columnar_frames::Recording};
use replay_columnar::manifest::Status;
use replay_data::{rows::{RowChanges, TableRows}, scope::CaptureMode};
use replay_frame::frame::{SharedView, TableView, Unsayable, cells_rows_of, cells_view};
use std::collections::hash_map::Entry;

impl Keys {
    fn contains(&self, key: u128) -> bool {
        match self {
            Self::All => true,
            Self::These(keys) => keys.binary_search(&key).is_ok(),
            Self::Range(lo, hi) => (*lo..=*hi).contains(&key),
            Self::Ranges(ranges) => ranges.get(ranges.partition_point(|&(_, hi)| hi < key)).is_some_and(|&(lo, _)| lo <= key),
        }
    }
}

struct Projection {
    reader: TableReader,
    selection: Option<Keys>,
    fields: u64,
    validity: u64,
}

#[derive(Default)]
pub struct Edits {
    pub previous: BTreeMap<TableId, SharedView>,
    pub tables: BTreeMap<TableId, BTreeMap<u128, u64>>,
    pub replaced: BTreeSet<TableId>,
}

impl Edits {
    pub fn fields(&self, table: TableId, mask: u64) -> bool {
        self.replaced.contains(&table) || self.tables.get(&table).is_some_and(|keys| keys.values().any(|fields| mask & fields != 0))
    }

    pub fn only_columns(&self, table: TableId, mask: u64) -> bool {
        self.replaced.is_empty() && self.tables.iter().all(|(id, rows)| *id == table && rows.values().all(|fields| fields & !mask == 0))
    }
}

impl Projection {
    fn new(recording: &Recording, run: u32, table: TableId, columns: Option<Vec<usize>>, cold: Option<Vec<usize>>) -> io::Result<Self> {
        let mask = |cols: Option<&[usize]>| cols.map_or(table.full_mask(), |v| v.iter().fold(0, |m, &c| m | 1 << c));
        let fields = mask(columns.as_deref());
        let omitted = mask(cold.as_deref()) & !fields;
        let mut validity = 0;
        for column in 0..64 {
            let bit = 1u64 << column;
            if omitted & bit != 0 && table.rows_from_wire_fields(&[0], bit, &mut |_, _| Ok(replay_data::rows::Field::NotRecorded)).is_err() {
                validity |= bit;
            }
        }
        let physical = columns.map(|mut cols| {
            cols.extend((0..64).filter(|&c| validity & (1 << c) != 0));
            cols
        });
        let mut reader = TableReader::new(recording, run, table, physical.as_deref())?;
        reader.index_keys();
        Ok(Self { reader, selection: None, fields, validity })
    }

    fn step_edits(&self, read: &Read, target: i32, step: ProductStep<'_>) -> Option<(BTreeMap<u128, u64>, Option<i32>)> {
        let table = read.table.index();
        let lane = step.lanes.get(table)?;
        let change = step.step.table(table)?;
        if change.reset() || change.log().is_some() || change.renumber().is_some() || change.head() != Some(target)
            || lane.status().0 != Status::Complete || !lane.keyed()
            || self.reader.columns().iter().any(|col| lane.requested().binary_search(col).is_err()) { return None; }
        let mut edits = BTreeMap::new();
        for &key in change.deleted().keys()?.iter().chain(change.inserted().keys()?) {
            if read.keys.contains(key) { edits.insert(key, u64::MAX); }
        }
        for column in change.columns() {
            let mask = 1u64 << column.column();
            if (self.fields | self.validity) & mask == 0 { continue; }
            let validity = (self.fields & mask == 0).then(|| lane.cells().column(column.column())).flatten();
            for &slot in column.slots() {
                if validity.is_some_and(|cells| cells.recorded(slot)) { continue; }
                if let Some(key) = lane.cells().key(slot).filter(|&k| read.keys.contains(k)) {
                    *edits.entry(key).or_default() |= mask;
                }
            }
        }
        Some((edits, lane.status().1))
    }

    fn read(&mut self, recording: &Recording, read: &Read, hour: i32, sample: bool, view: &mut SharedView, edits: &mut Edits, count: &mut u64, step: Option<ProductStep<'_>>) -> io::Result<()> {
        let table = read.table;
        let mut reset = self.selection.as_ref() != Some(&read.keys);
        let mut dirty = BTreeMap::new();
        let stepped = step.filter(|_| !reset && matches!(view.held(), TableView::Complete(_))).and_then(|s| self.step_edits(read, hour, s));
        let reader = &mut self.reader;
        if let Some((changes, sampled)) = stepped {
            if changes.is_empty() {
                if view.sampled_at() != sampled {
                    *view = view.clone().with_sampled_at(sampled);
                    if sample { edits.replaced.insert(table); }
                }
                return Ok(());
            }
            dirty = changes;
            reader.go_to(recording, recording.run(reader.run())?.ordinal(hour).unwrap())?;
        } else if reader.hour().is_none_or(|h| h > hour) {
            let ordinal = recording.run(reader.run())?.ordinal(hour).ok_or_else(|| io::Error::other("Hour outside the recorded run"))?;
            reader.seek(recording, ordinal)?;
            reset = true;
        } else {
            let before = reader.status();
            while reader.hour().is_some_and(|h| h < hour) {
                if !reader.step(recording)? { return Err(io::Error::other("Product reader reached the end of its run")); }
                let changes = reader.changes();
                if reset { continue; }
                for &(_, key) in &changes.deleted {
                    if read.keys.contains(key) { dirty.insert(key, u64::MAX); }
                }
                for &slot in &changes.inserted {
                    if let Some(key) = reader.cells().key(slot).filter(|&k| read.keys.contains(k)) { dirty.insert(key, u64::MAX); }
                }
                for (&column, slots) in reader.columns().iter().zip(&changes.written) {
                    let validity = (self.fields & (1 << column) == 0 && matches!(view.held(), TableView::Complete(_))).then(|| reader.cells().column(column)).flatten();
                    for &slot in slots {
                        if validity.is_some_and(|cells| cells.recorded(slot)) { continue; }
                        if let Some(key) = reader.cells().key(slot).filter(|&k| read.keys.contains(k)) {
                            *dirty.entry(key).or_default() |= 1u64 << column;
                        }
                    }
                }
            }
            reset |= before != reader.status() || matches!(reader.status(), Status::Truncated { .. });
        }
        self.selection = Some(read.keys.clone());
        let format = recording.format();
        let status = reader.status();
        let cells = reader.view();
        let sample_hour = (status == Status::Complete).then(|| reader.sampled_at()).flatten();
        reset |= !dirty.is_empty() && !matches!(view.held(), TableView::Complete(_)) && status == Status::Complete;
        if !reset && dirty.is_empty() {
            if view.sampled_at() != sample_hour && matches!(view.held(), TableView::Complete(_)) {
                *view = view.clone().with_sampled_at(sample_hour);
                if sample { edits.replaced.insert(table); }
            }
            return Ok(());
        }
        let decode = |keyed: &[(u128, u32)]| cells_rows_of(table, &cells, keyed, format, self.fields | self.validity);
        if reset {
            let mut keyed = Vec::new();
            let mut keep = |slot| {
                if let Some(key) = cells.key(slot).filter(|&k| read.keys.contains(k)) { keyed.push((key, slot)); }
            };
            let made = match status {
                Status::Complete | Status::Truncated { .. } => {
                    if let Some(rows) = reader.read_rows() { rows.iter().copied().for_each(keep); }
                    else if let Keys::These(keys) = &read.keys { keyed.extend(keys.iter().filter_map(|&key| Some((key, reader.slot(key)?)))); }
                    else { cells.rows().for_each(&mut keep); }
                    keyed.sort_unstable();
                    *count += keyed.len() as u64;
                    decode(&keyed).map(|rows| match status {
                        Status::Truncated { missing } => TableView::Truncated { rows, missing },
                        _ => TableView::Complete(rows),
                    })
                }
                _ => cells_view(table, &cells, status, None, CaptureMode::from_byte(recording.root_identity().capture_mode)?, format),
            };
            *view = match made {
                Ok(TableView::Complete(rows)) => SharedView::from(TableView::Complete(rows)).with_sampled_at(sample_hour),
                Ok(view) => view.into(),
                Err(e) if Unsayable::of(&e).is_some() => TableView::Unavailable { reason: e.to_string() }.into(),
                Err(e) => return Err(e),
            };
            edits.replaced.insert(table);
        } else if let TableView::Complete(old) = &mut **view {
            let mut keyed = Vec::new();
            let mut deleted = Vec::new();
            for &key in dirty.keys() {
                if let Some(slot) = reader.slot(key) { keyed.push((key, slot)); }
                else if old.contains_key(key) { deleted.push(key); }
            }
            *count += keyed.len() as u64;
            let fresh = match decode(&keyed) {
                Ok(rows) => rows,
                Err(e) if Unsayable::of(&e).is_some() => {
                    self.selection = None;
                    return self.read(recording, read, hour, sample, view, edits, count, None);
                }
                Err(e) => return Err(e),
            };
            let previous = std::mem::replace(old, TableRows::empty(table));
            *old = previous.apply_changes(RowChanges::new(fresh, deleted))?;
            *view = view.clone().with_sampled_at(sample_hour);
            edits.tables.insert(table, dirty);
        }
        Ok(())
    }
}

type Projections = HashMap<TableId, Result<Projection, String>>;

struct Input<'a> {
    recording: &'a Recording,
    run: u32,
    hour: i32,
    count: &'a mut u64,
    step: Option<ProductStep<'a>>,
}

impl Input<'_> {
    fn read(&mut self, tables: &mut Projections, read: &Read, view: &mut SharedView, columns: impl Fn() -> (Option<Vec<usize>>, Option<Vec<usize>>), sample: bool, edits: &mut Edits) -> io::Result<()> {
        let table = read.table;
        let slot = match tables.entry(table) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(match {
                let (selected, cold) = columns();
                Projection::new(self.recording, self.run, table, selected, cold)
            } {
                Ok(reader) => Ok(reader),
                Err(e) if e.kind() == io::ErrorKind::InvalidData => Err(e.to_string()),
                Err(e) => return Err(e),
            }),
        };
        let reason = match slot {
            Ok(reader) => match reader.read(self.recording, read, self.hour, sample, view, edits, self.count, self.step) {
                Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                    tables.remove(&table);
                    e.to_string()
                }
                result => return result,
            },
            Err(reason) if view.held().is_not_selected() => reason.clone(),
            Err(_) => return Ok(()),
        };
        *view = TableView::Unavailable { reason }.into();
        edits.replaced.insert(table);
        Ok(())
    }
}

#[derive(Default)]
struct Supplemental {
    reads: Vec<Read>,
    tables: Projections,
    views: HashMap<TableId, SharedView>,
}

impl Supplemental {
    fn advance<P: AtHour>(&mut self, input: &mut Input<'_>) -> io::Result<bool> {
        let mut edits = Edits::default();
        for read in &self.reads {
            let view = self.views.entry(read.table).or_insert_with(not_selected);
            input.read(&mut self.tables, read, view, || (P::columns(read.table), P::columns(read.table)), false, &mut edits)?;
        }
        Ok(!edits.tables.is_empty() || !edits.replaced.is_empty())
    }

    fn values(&mut self, live: &Live, frame: &mut DecodedFrame) -> Value {
        for (table, view) in &mut self.views { std::mem::swap(view, &mut frame.tables[table.index()]); }
        let values = live.values(frame);
        for (table, view) in &mut self.views { std::mem::swap(view, &mut frame.tables[table.index()]); }
        live.json(&values.into_iter().map(GroupState::Values).collect::<Vec<_>>())
    }
}

struct Stream {
    tables: Projections,
    extra: Supplemental,
    plans: Vec<(Vec<Read>, i32)>,
    frame: Box<DecodedFrame>,
    run: u32,
    identity_through: i32,
    live: Live,
    data: Arc<PanelData>,
    encoding: Option<super::encoding::Encoding>,
}

pub(super) fn reader<P: AtHour>(source: &Source, params: Arc<P::Params>, context: Arc<P::Context>, query: Query<'_>, warnings: Vec<String>) -> ProductReader
where P::Context: Sync, P::Params: Sync {
    let (replay, timeline, holders, executable) = (source.replay(), query.timeline, query.holders, source.tables());
    use dashboard_query::identity::{Country, Holder};
    let owner = P::owner(&params);
    let owned = |tag: &str, run: &dashboard_query::identity::HolderRun| match owner {
        None => true,
        Some(Country::Tag(owned)) => owned == tag,
        Some(Country::Slot(slot)) => matches!(run.holder, Holder::Found { slot: held, .. } if held == *slot),
    };
    let mut identity_edges: Vec<i32> = holders.tags.iter()
        .flat_map(|(tag, runs)| runs.iter().filter(move |run| owned(tag, run)))
        .flat_map(|r| [Some(r.first), r.last.checked_add(1)].into_iter().flatten()).collect();
    identity_edges.sort_unstable();
    identity_edges.dedup();
    let stream = Mutex::new(None::<Stream>);
    Arc::new(move |hour, reset, cancelled, step| {
        let mut held = stream.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut count = 0;
        let result = engine_executable::executable_tables::scope(executable, || -> io::Result<_> {
            if let Some(state) = held.as_ref().filter(|s| s.frame.hour == hour) { return Ok(state.data.clone()); }
            let run = timeline.at(hour).ok_or_else(|| dashboard_core::refused(format!("{} hour is not recorded", P::TITLE)))?.run;
            let owner = holder::<P>(&holders, &params, hour)?;
            let seed = reset || held.as_ref().is_none_or(|s| s.run != run || hour < s.frame.hour);
            if seed {
                *held = Some(Stream { tables: HashMap::new(), extra: Supplemental::default(), encoding: None, plans: Vec::new(), frame: Box::new(empty(timeline.id, hour)), run, identity_through: hour, live: Live::default(), data: Arc::new(Err(String::new())) });
            }
            let state = held.as_mut().expect("seeded");
            let previous_hour = state.frame.hour;
            state.frame.hour = hour;
            let identity_changed = seed || hour > state.identity_through;
            if identity_changed {
                state.identity_through = identity_edges.get(identity_edges.partition_point(|&h| h <= hour)).map_or(i32::MAX, |&h| h - 1);
            }
            let mut edits = Edits::default();
            let mut selected = BTreeSet::new();
            let at = ReadsAt { holders: &holders, context: &*context, run, hour };
            let mut input = Input { recording: replay.recording(), run, hour, count: &mut count, step: step.filter(|s| !seed && previous_hour.checked_add(1) == Some(s.step.first()) && s.step.last() == hour) };
            for stage in 0.. {
                crate::metadata::check_cancel(cancelled)?;
                if stage >= state.plans.len() {
                    state.plans.push(P::retained_reads(&params, owner, stage, &state.frame, &at));
                } else if identity_changed || hour > state.plans[stage].1 || P::plan_changes(stage, &edits) {
                    state.plans[stage] = P::retained_reads(&params, owner, stage, &state.frame, &at);
                }
                let (asks, _) = &state.plans[stage];
                if asks.is_empty() { state.plans.truncate(stage + 1); break; }
                for read in asks {
                    let table = read.table;
                    selected.insert(table);
                    let view = &mut state.frame.tables[table.index()];
                    if !seed && P::compares_previous(table) { edits.previous.insert(table, view.clone()); }
                    input.read(&mut state.tables, read, view, || (P::projection(&params, table), P::columns(table)), P::uses_sample(table), &mut edits)?;
                }
            }
            state.tables.retain(|table, _| {
                if selected.contains(table) { true } else {
                    state.frame.tables[table.index()] = not_selected();
                    edits.replaced.insert(*table);
                    false
                }
            });
            let cells = Cells::default();
            let at = At { owner, holders: &holders, context: &*context, cancelled, hour, live: &cells };
            if !identity_changed && selected.iter().all(|table| matches!(state.frame.tables[table.index()].held(), TableView::Complete(_))) {
                let changed = !edits.tables.is_empty() || !edits.replaced.is_empty();
                if !changed && !P::CLOCK_DEPENDENT {
                    if state.extra.advance::<P>(&mut input)? && let Ok(data) = Arc::make_mut(&mut state.data) {
                        data["live"] = state.extra.values(&state.live, &mut state.frame);
                    }
                    return Ok(state.data.clone());
                }
                if state.extra.reads.is_empty() && let Ok(data) = Arc::make_mut(&mut state.data) {
                    if P::advance(&state.frame, &params, &at, &edits, &state.live, data)? {
                        if let Some(encoding) = &mut state.encoding { encoding.patch(data, &state.live, &edits)?; }
                        return Ok(state.data.clone());
                    }
                }
            }
            let mut own = OwnRefusals::default();
            let mut reply = own_refusals(&mut own, || P::reply(&state.frame, &params, &at))?;
            finish(reply.warnings_mut(), &own, &warnings);
            state.live = cells.finish();
            state.extra.reads = supplemental(&state.live, |g| state.plans.iter().flat_map(|(reads, _)| reads).rev().find(|r| r.table == g.table).is_some_and(|r| covers(r, g)));
            state.extra.views.retain(|table, _| state.extra.reads.iter().any(|r| r.table == *table));
            state.extra.tables.retain(|table, _| state.extra.views.contains_key(table));
            state.extra.advance::<P>(&mut input)?;
            let live = (!state.live.is_empty()).then(|| state.extra.values(&state.live, &mut state.frame));
            let data = serde_json::to_value(WithLive { reply, live })?;
            state.encoding = P::ENCODE_LIVE_CELLS.then(|| super::encoding::Encoding::new(&data)).transpose()?;
            state.data = Arc::new(Ok(data));
            Ok(state.data.clone())
        });
        let result = result.unwrap_or_else(|e| { *held = None; Arc::new(Err(e.to_string())) });
        let encoded = held.as_ref().and_then(|s| s.encoding.as_ref()).map(|e| e.body.clone());
        let through = held.as_ref().filter(|_| !P::CLOCK_DEPENDENT && result.is_ok()).and_then(|s| {
            let horizon = timeline.at(hour)?.last.min(s.identity_through);
            let horizon = s.plans.iter().fold(horizon, |end, (_, limit)| end.min(*limit));
            s.tables.iter().chain(&s.extra.tables).try_fold(horizon, |end, (table, projection)| {
                Some(end.min(projection.as_ref().ok()?.reader.unchanged_through(P::uses_sample(*table))?))
            })
        }).unwrap_or(hour).max(hour);
        dashboard_follow::panel::ProductRead { value: result, rows: count, encoded, through }
    })
}
