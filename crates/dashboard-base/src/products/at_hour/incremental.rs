use super::{placed::{Pool, Taken, Warm}, *};
use dashboard_follow::panel::{ProductReader, ProductStep};
use playhead::{Step, changes::Changes, lane::Lane, pages::WorldPages};
use replay_columnar::{manifest::Status, store::TableView as StoreView};
use replay_data::{rows::{RowChanges, TableRows}, scope::CaptureMode};
use replay_frame::frame::{SharedView, TableView, Unsayable, cells_rows_of};
use std::collections::hash_map::Entry;

pub(super) struct Projection {
    run: usize,
    columns: Vec<usize>,
    format: u16,
    fields: u64,
    validity: u64,
    selection: Option<Arc<Keys>>,
    at: Option<i32>,
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

fn field_mask(table: TableId, columns: Option<&[usize]>) -> u64 {
    columns.map_or(table.full_mask(), |v| v.iter().fold(0, |m, &c| m | 1 << c))
}

pub(super) fn capture_mode(pages: &WorldPages) -> io::Result<CaptureMode> {
    CaptureMode::from_byte(pages.recording().root_identity().capture_mode)
}

impl Projection {
    pub(super) fn new(pages: &WorldPages, run: usize, table: TableId, columns: Option<Vec<usize>>, cold: Option<Vec<usize>>) -> io::Result<Self> {
        if let Some(reason) = pages.withheld(table.index()) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, reason.to_owned()));
        }
        let fields = field_mask(table, columns.as_deref());
        let omitted = field_mask(table, cold.as_deref()) & !fields;
        let mut validity = 0;
        for column in 0..64 {
            let bit = 1u64 << column;
            if omitted & bit != 0 && table.rows_from_wire_fields(&[0], bit, &mut |_, _| Ok(replay_data::rows::Field::NotRecorded)).is_err() {
                validity |= bit;
            }
        }
        let recording = pages.recording();
        let mut physical = match columns {
            Some(mut columns) => {
                columns.extend((0..64).filter(|&c| validity & (1 << c) != 0));
                columns
            }
            None => recording.read_columns(table.index()),
        };
        let recorded = recording.columns()[table.index()];
        physical.retain(|&column| column < recorded);
        physical.sort_unstable();
        physical.dedup();
        Ok(Self { run, columns: physical, format: recording.format(), fields, validity, selection: None, at: None })
    }

    fn lane<'s>(&self, table: TableId, hour: i32, step: ProductStep<'s>) -> Option<(&'s Step, &'s Lane, &'s Changes)> {
        if step.step.last() != hour { return None; }
        let lane = step.lanes.get(table.index())?;
        let change = step.step.table(table.index())?;
        (lane.run() == self.run && change.head() == Some(hour) && change.log().is_none() && lane.keyed()
            && self.columns.iter().all(|column| lane.requested().contains(column))).then_some((step.step, lane, change))
    }

    fn read(&mut self, input: &mut Input<'_>, read: &Read, sample: bool, view: &mut SharedView, edits: &mut Edits) -> io::Result<()> {
        if self.selection.as_deref() != Some(&read.keys) {
            self.at = None;
            self.selection = Some(Arc::new(read.keys.clone()));
        }
        let Input { pool, hour, count, step, placed, mode, .. } = input;
        let (run, table) = (self.run, read.table.index());
        match step.and_then(|step| self.lane(read.table, *hour, step)) {
            Some((step, lane, change)) => self.from_lane(read, sample, view, edits, count, lane, Some((step, change)), *mode)?,
            None => {
                let warm = if step.is_some_and(|step| step.lanes_table(table)) {
                    let columns = self.columns.clone();
                    let wake = step.and_then(|step| step.wake.cloned());
                    pool.warm(run, table, &columns, *hour, wake, |lane| self.from_lane(read, sample, view, edits, count, lane, None, *mode))
                } else { Warm::Cold };
                match warm {
                    Warm::Read(result) => result?,
                    Warm::Warming => return Err(io::Error::new(io::ErrorKind::WouldBlock, format!("{:?} is being placed beside the step", read.table))),
                    Warm::Cold => {
                        let placed = placed.get_or_insert_with(|| pool.take(run, *hour));
                        let lane = placed.lane(table, &self.columns, *hour)?;
                        self.from_lane(read, sample, view, edits, count, lane, None, *mode)?;
                    }
                }
            }
        }
        *view = view.clone().with_keys(self.selection.clone());
        self.at = Some(*hour);
        Ok(())
    }

    #[expect(clippy::too_many_arguments, reason = "one table's read, its lane and the reply's edits")]
    pub(super) fn from_lane(&mut self, read: &Read, sample: bool, view: &mut SharedView, edits: &mut Edits, count: &mut u64, lane: &Lane, step: Option<(&Step, &Changes)>, mode: CaptureMode) -> io::Result<()> {
        let table = read.table;
        let (status, sampled) = lane.status();
        let other = match status {
            Status::Complete | Status::Truncated { .. } => None,
            Status::NotRecorded => Some(TableView::NotRecorded { mode }),
            Status::Unavailable { .. } => Some(TableView::Unavailable { reason: lane.reason().and_then(|reason| std::str::from_utf8(reason).ok()).unwrap_or("A table's reason is not text").to_owned() }),
        };
        if let Some(made) = other {
            if *view != made {
                *view = made.into();
                edits.replaced.insert(table);
            }
            return Ok(());
        }
        let cells = lane.view();
        let continues = |step: &Step, change: &Changes| self.at.is_some_and(|at| at.checked_add(1) == Some(step.first())) && !change.reset();
        if let (TableView::Truncated { missing, .. }, Status::Truncated { missing: still }, Some((step, change))) = (view.held(), status, step)
            && *missing == still && continues(step, change) && !change.read_rows_changed() && change.renumber().is_none()
            && change.inserted().is_empty() && change.deleted().is_empty() && change.columns().iter().all(|written| written.is_empty()) {
            return Ok(());
        }
        let (held, change) = match (view.held(), step) {
            (TableView::Complete(held), Some((step, change))) if status == Status::Complete && continues(step, change) => (held, change),
            _ => return self.whole(read, &cells, status, lane.read_rows(), sampled, sample, view, edits, count),
        };
        let mut dirty: BTreeMap<u128, u64> = BTreeMap::new();
        for &key in change.deleted().keys().into_iter().flatten().chain(change.inserted().keys().into_iter().flatten()) {
            if read.keys.contains(key) { dirty.insert(key, u64::MAX); }
        }
        let renumber = change.renumber();
        let now = |slot: u32| match renumber {
            Some(old) => old.binary_search(&slot).ok().map(|at| at as u32),
            None => Some(slot),
        };
        for column in change.columns() {
            let mask = 1u64 << column.column();
            if (self.fields | self.validity) & mask == 0 { continue; }
            let validity = (self.fields & mask == 0).then(|| lane.cells().column(column.column())).flatten();
            for slot in column.slots().iter().filter_map(|&slot| now(slot)) {
                if validity.is_some_and(|cells| cells.recorded(slot)) { continue; }
                if let Some(key) = lane.cells().key(slot).filter(|&k| read.keys.contains(k)) {
                    *dirty.entry(key).or_default() |= mask;
                }
            }
        }
        if dirty.is_empty() {
            if view.sampled_at() != sampled {
                *view = view.clone().with_sampled_at(sampled);
                if sample { edits.replaced.insert(table); }
            }
            return Ok(());
        }
        let mut keyed = Vec::new();
        let mut deleted = Vec::new();
        for &key in dirty.keys() {
            if let Some(slot) = cells.slot(key) { keyed.push((key, slot)); }
            else if held.contains_key(key) { deleted.push(key); }
        }
        *count += keyed.len() as u64;
        let fresh = match cells_rows_of(table, &cells, &keyed, self.format, self.fields | self.validity) {
            Ok(rows) => rows,
            Err(e) if Unsayable::of(&e).is_some() => return self.whole(read, &cells, status, None, sampled, sample, view, edits, count),
            Err(e) => return Err(e),
        };
        if let TableView::Complete(old) = &mut **view {
            let previous = std::mem::replace(old, TableRows::empty(table));
            *old = previous.apply_changes(RowChanges::new(fresh, deleted))?;
        }
        *view = view.clone().with_sampled_at(sampled);
        edits.tables.insert(table, dirty);
        Ok(())
    }

    #[expect(clippy::too_many_arguments, reason = "one table's cells at an hour and the reply's edits")]
    fn whole(&self, read: &Read, cells: &StoreView<'_>, status: Status, read_rows: Option<&[u32]>, sampled: Option<i32>, sample: bool, view: &mut SharedView, edits: &mut Edits, count: &mut u64) -> io::Result<()> {
        let mut keyed = Vec::new();
        let mut keep = |slot| {
            if let Some(key) = cells.key(slot).filter(|&k| read.keys.contains(k)) { keyed.push((key, slot)); }
        };
        if let Some(rows) = read_rows { rows.iter().copied().for_each(keep); }
        else if let Keys::These(keys) = &read.keys { keyed.extend(keys.iter().filter_map(|&key| Some((key, cells.slot(key)?)))); }
        else { cells.rows().for_each(&mut keep); }
        keyed.sort_unstable();
        *count += keyed.len() as u64;
        let made = cells_rows_of(read.table, cells, &keyed, self.format, self.fields | self.validity).map(|rows| match status {
            Status::Truncated { missing } => TableView::Truncated { rows, missing },
            _ => TableView::Complete(rows),
        });
        settle(read.table, made, sampled, sample, view, edits)
    }
}

fn settle(table: TableId, made: io::Result<TableView>, sampled: Option<i32>, sample: bool, view: &mut SharedView, edits: &mut Edits) -> io::Result<()> {
    let made = match made {
        Ok(made) => made,
        Err(e) if Unsayable::of(&e).is_some() => TableView::Unavailable { reason: e.to_string() },
        Err(e) => return Err(e),
    };
    if let (TableView::Complete(held), TableView::Complete(rows)) = (view.held(), &made) {
        let changes = rows.changes_since(held)?.ok_or_else(|| io::Error::other(format!("{table:?} rows of another table")))?;
        let keys: BTreeMap<u128, u64> = changes.rows().iter().map(|row| row.key()).chain(changes.deleted().iter().copied()).map(|key| (key, u64::MAX)).collect();
        if keys.is_empty() {
            if view.sampled_at() != sampled {
                *view = view.clone().with_sampled_at(sampled);
                if sample { edits.replaced.insert(table); }
            }
            return Ok(());
        }
        *view = SharedView::from(made).with_sampled_at(sampled);
        edits.tables.insert(table, keys);
        return Ok(());
    }
    if matches!(made, TableView::Truncated { .. }) && view.held() == &made { return Ok(()); }
    *view = match made {
        TableView::Complete(rows) => SharedView::from(TableView::Complete(rows)).with_sampled_at(sampled),
        made => made.into(),
    };
    edits.replaced.insert(table);
    Ok(())
}

type Projections = HashMap<TableId, Result<Projection, String>>;

pub(super) struct Input<'a> {
    pub(super) pool: &'a Arc<Pool>,
    pub(super) run: usize,
    pub(super) hour: i32,
    pub(super) mode: CaptureMode,
    pub(super) count: &'a mut u64,
    pub(super) step: Option<ProductStep<'a>>,
    pub(super) placed: Option<Taken>,
}

impl Input<'_> {
    pub(super) fn frame(&mut self, reads: &[Read], columns: fn(TableId) -> Option<Vec<usize>>, branch: u64) -> io::Result<DecodedFrame> {
        let mut frame = empty(branch, self.hour);
        let mut tables = Projections::new();
        for read in reads {
            let fields = columns(read.table);
            self.read(&mut tables, read, &mut frame.tables[read.table.index()], || (fields.clone(), fields.clone()), false, &mut Edits::default())?;
        }
        count_frame(&frame, reads);
        Ok(frame)
    }

    fn read(&mut self, tables: &mut Projections, read: &Read, view: &mut SharedView, columns: impl Fn() -> (Option<Vec<usize>>, Option<Vec<usize>>), sample: bool, edits: &mut Edits) -> io::Result<()> {
        let table = read.table;
        let (selected, cold) = columns();
        let made = |selected, cold| match Projection::new(&self.pool.pages, self.run, table, selected, cold) {
            Ok(projection) => Ok(Ok(projection)),
            Err(e) if e.kind() == io::ErrorKind::InvalidData => Ok(Err(e.to_string())),
            Err(e) => Err(e),
        };
        let slot = match tables.entry(table) {
            Entry::Occupied(mut entry) if entry.get().as_ref().is_ok_and(|held| held.fields != field_mask(table, selected.as_deref())) => {
                *entry.get_mut() = made(selected, cold)?;
                entry.into_mut()
            }
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(made(selected, cold)?),
        };
        let reason = match slot {
            Ok(projection) => match projection.read(self, read, sample, view, edits) {
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
    columns: HashMap<TableId, Vec<usize>>,
    tables: Projections,
    views: HashMap<TableId, SharedView>,
}

impl Supplemental {
    fn set(&mut self, live: &Live, covered: impl Fn(&Group) -> bool) {
        self.reads = supplemental(live, covered);
        self.columns.clear();
        for group in live.groups.iter().filter(|group| self.reads.iter().any(|read| read.table == group.table)) {
            let columns = self.columns.entry(group.table).or_default();
            columns.extend(&group.columns);
            columns.sort_unstable();
            columns.dedup();
        }
        self.views.retain(|table, _| self.columns.contains_key(table));
        self.tables.retain(|table, _| self.columns.contains_key(table));
    }

    fn advance<P: AtHour>(&mut self, input: &mut Input<'_>) -> io::Result<bool> {
        let mut edits = Edits::default();
        for read in &self.reads {
            let view = self.views.entry(read.table).or_insert_with(not_selected);
            let columns = self.columns.get(&read.table).cloned();
            input.read(&mut self.tables, read, view, || (columns.clone(), P::columns(read.table)), false, &mut edits)?;
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
    context_revision: u64,
    identity_through: i32,
    reply_through: i32,
    live: Arc<Live>,
    data: Arc<PanelData>,
    values_at: Option<i32>,
    reply: Option<Box<dyn std::any::Any + Send>>,
    sampled: Option<BTreeSet<TableId>>,
    rebuild: bool,
}

impl Stream {
    fn lanes(&self) -> Vec<Declared> {
        let declared = |(&table, projection): (&TableId, &Result<Projection, String>)| {
            Some(Declared { table: table.index(), columns: projection.as_ref().ok()?.columns.clone(), keyed: true, passing: false, hourly: false })
        };
        let mut lanes: Vec<_> = self.tables.iter().filter_map(declared).collect();
        for extra in self.extra.tables.iter().filter_map(declared) {
            if let Some(main) = lanes.iter_mut().find(|lane| lane.table == extra.table) {
                main.columns.extend(extra.columns);
                main.columns.sort_unstable();
                main.columns.dedup();
            } else {
                lanes.push(extra);
            }
        }
        lanes
    }
}

pub struct Held<'a>(&'a mut Arc<PanelData>);

impl std::ops::Deref for Held<'_> {
    type Target = Value;
    fn deref(&self) -> &Value {
        static NONE: Value = Value::Null;
        self.0.as_ref().as_ref().unwrap_or(&NONE)
    }
}

impl std::ops::DerefMut for Held<'_> {
    fn deref_mut(&mut self) -> &mut Value {
        let data = Arc::make_mut(self.0);
        if data.is_err() { *data = Ok(Value::Null); }
        match data { Ok(value) => value, Err(_) => unreachable!("made a value above") }
    }
}

fn current_values<P: AtHour>(state: &mut Stream, input: &mut Input<'_>) -> io::Result<Arc<PanelData>> {
    if state.values_at != Some(input.hour) && state.data.is_ok() {
        if !state.live.is_empty() {
            state.extra.advance::<P>(input)?;
            let live = state.extra.values(&state.live, &mut state.frame);
            if let Ok(Value::Object(data)) = Arc::make_mut(&mut state.data) { data.insert("live".into(), live); }
        }
        state.values_at = Some(input.hour);
    }
    Ok(state.data.clone())
}

fn sampling<T>(frame: &DecodedFrame, sampled: &mut Option<BTreeSet<TableId>>, compute: impl FnOnce() -> T) -> T {
    let (value, views) = replay_frame::frame::view_reads::samples(compute);
    let tables = sampled.get_or_insert_with(BTreeSet::new);
    for view in &views {
        if let Some(&table) = TableId::ALL.iter().find(|table| view.is(&frame.tables[table.index()])) { tables.insert(table); }
    }
    value
}

pub(super) fn reader<P: AtHour>(source: &Source, pool: Arc<Pool>, params: Arc<P::Params>, context: Arc<P::Context>, query: Query<'_>, warnings: Vec<String>) -> ProductReader
where P::Context: Sync, P::Params: Sync {
    let (timeline, holders, executable) = (query.timeline, query.holders, source.tables());
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
    Arc::new(move |hour, values, cancelled, step| {
        let mut held = stream.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut count = 0;
        let result = engine_executable::executable_tables::scope(executable, || -> io::Result<_> {
            let run = timeline.at(hour).ok_or_else(|| dashboard_core::refused(format!("{} hour is not recorded", P::TITLE)))?.run;
            let read_run = pool.pages.run_of(run).ok_or_else(|| dashboard_core::refused(format!("{} hour is in a run the recording cannot read", P::TITLE)))?;
            let mut input = Input { pool: &pool, run: read_run, hour, mode: capture_mode(&pool.pages)?, count: &mut count, step, placed: None };
            if let Some(state) = held.as_mut().filter(|s| s.frame.hour == hour && s.run == run && !s.rebuild) {
                return if values { current_values::<P>(state, &mut input) } else { Ok(state.data.clone()) };
            }
            let owner = holder::<P>(&holders, &params, hour)?;
            let seed = held.as_ref().is_none_or(|s| s.run != run || hour < s.frame.hour);
            if seed {
                *held = Some(Stream { tables: HashMap::new(), extra: Supplemental::default(), values_at: None, plans: Vec::new(), frame: Box::new(empty(timeline.id, hour)), run, context_revision: 0, identity_through: hour, reply_through: i32::MAX, live: Arc::default(), data: Arc::new(Err(String::new())), reply: None, sampled: None, rebuild: true });
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
            let at = ReadsAt { holders: &holders, context: &*context, run, hour, since: (!seed).then_some(previous_hour) };
            let mut tried: HashMap<TableId, Vec<u64>> = HashMap::new();
            let clocked = hour != previous_hour && P::clock_dependent(&state.plans, &context);
            loop {
                selected.clear();
                for stage in 0.. {
                    crate::metadata::check_cancel(cancelled)?;
                    if stage >= state.plans.len() {
                        let planned = sampling(&state.frame, &mut state.sampled, || P::retained_reads(&params, owner, stage, &state.frame, &at));
                        state.plans.push(planned);
                    } else if state.rebuild || identity_changed || clocked || hour > state.plans[stage].1 || P::plan_changes(stage, &edits, &state.frame, &at) {
                        state.plans[stage] = sampling(&state.frame, &mut state.sampled, || P::retained_reads(&params, owner, stage, &state.frame, &at));
                    }
                    let (asks, _) = &state.plans[stage];
                    if asks.is_empty() { state.plans.truncate(stage + 1); break; }
                    for read in asks {
                        let table = read.table;
                        selected.insert(table);
                        let view = &mut state.frame.tables[table.index()];
                        if !seed && P::compares_previous(table) { edits.previous.entry(table).or_insert_with(|| view.clone()); }
                        let sample = state.sampled.as_ref().is_none_or(|sampled| sampled.contains(&table));
                        input.read(&mut state.tables, read, view, || (P::projection(&params, table, &state.plans, &context), P::columns(table)), sample, &mut edits)?;
                    }
                }
                let projected = |table: TableId| field_mask(table, P::projection(&params, table, &state.plans, &context).as_deref());
                let moved: Vec<(TableId, u64, u64)> = state.tables.iter()
                    .filter_map(|(&table, held)| held.as_ref().ok().filter(|held| selected.contains(&table) && held.fields != projected(table)).map(|held| (table, held.fields, projected(table))))
                    .collect();
                if moved.is_empty() { break; }
                for (table, read, asked) in moved {
                    let masks = tried.entry(table).or_default();
                    masks.push(read);
                    if masks.contains(&asked) { return Err(io::Error::other(format!("{} reads {table:?} under projections that do not settle", P::TITLE))); }
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
            let revision = P::context_revision(&context);
            let expired = hour > state.reply_through || revision != state.context_revision;
            if !state.rebuild && !identity_changed {
                let changed = !edits.tables.is_empty() || !edits.replaced.is_empty();
                if !changed && !expired && !P::clock_dependent(&state.plans, &context) {
                    return if values { current_values::<P>(state, &mut input) } else { Ok(state.data.clone()) };
                }
                if state.data.is_ok() && !expired && selected.iter().all(|table| matches!(state.frame.tables[table.index()].held(), TableView::Complete(_))) {
                    let kept = state.data.clone();
                    if P::advance(&state.frame, &params, &at, &edits, &state.live, &mut Held(&mut state.data))? {
                        state.reply = None;
                        state.reply_through = P::valid_through(&state.frame, &params, &at);
                        return if values { current_values::<P>(state, &mut input) } else { Ok(state.data.clone()) };
                    }
                    state.data = kept;
                }
            }
            let mut own = OwnRefusals::default();
            #[cfg(any(test, feature = "test-support"))]
            crate::test_fixtures::product_called();
            let result = sampling(&state.frame, &mut state.sampled, || own_refusals(&mut own, || P::reply(&state.frame, &params, &at)));
            let mut reply = match result {
                Ok(reply) => reply,
                Err(error) if dashboard_core::refusal(&error).is_some() && P::holds_refusal(&state.frame) => {
                    state.data = Arc::new(Err(error.to_string()));
                    state.live = Arc::default();
                    state.extra = Supplemental::default();
                    state.reply = None;
                    state.reply_through = sampling(&state.frame, &mut state.sampled, || P::valid_through(&state.frame, &params, &at));
                    state.rebuild = false;
                    state.values_at = Some(hour);
                    return Ok(state.data.clone());
                }
                Err(error) => return Err(error),
            };
            let partial = selected.iter().any(|table| matches!(state.frame.tables[table.index()].held(), TableView::Truncated { .. }));
            let reply_through = sampling(&state.frame, &mut state.sampled, || P::valid_reply_through(&state.frame, &params, &at, &reply, partial));
            finish(reply.warnings_mut(), &own, &warnings);
            let live = cells.finish();
            let projected = |table: TableId| P::projection(&params, table, &state.plans, &context);
            state.extra.set(&live, |g| state.plans.iter().flat_map(|(reads, _)| reads).rev().find(|r| r.table == g.table)
                .is_some_and(|r| covers(r, g) && projected(g.table).is_none_or(|fields| g.columns.iter().all(|column| fields.contains(column)))));
            state.context_revision = revision;
            state.reply_through = reply_through;
            if !seed && !state.rebuild && state.data.is_ok() && *state.live == live
                && state.reply.as_ref().and_then(|last| last.downcast_ref::<P::Reply>()).is_some_and(|last| *last == reply) {
                return if values { current_values::<P>(state, &mut input) } else { Ok(state.data.clone()) };
            }
            state.live = Arc::new(live);
            state.extra.advance::<P>(&mut input)?;
            let live = (!state.live.is_empty()).then(|| state.extra.values(&state.live, &mut state.frame));
            let data = serde_json::to_value(WithLive { reply: &reply, live })?;
            state.reply = Some(Box::new(reply));
            state.rebuild = false;
            state.values_at = Some(hour);
            state.data = Arc::new(Ok(data));
            Ok(state.data.clone())
        });
        let pending = result.as_ref().is_err_and(|e| e.kind() == io::ErrorKind::WouldBlock) && held.is_some();
        let result = match result {
            Ok(data) => data,
            Err(_) if pending => {
                let state = held.as_mut().expect("held while pending");
                state.rebuild = true;
                state.data.clone()
            }
            Err(e) => { *held = None; Arc::new(Err(e.to_string())) }
        };
        let fresh = !pending && held.as_ref().is_some_and(|s| s.values_at == Some(hour));
        let live = held.as_ref().map(|s| s.live.clone()).filter(|live| !live.is_empty());
        let lanes = held.as_ref().map_or_else(Vec::new, Stream::lanes);
        dashboard_follow::panel::ProductRead { value: result, rows: count, fresh, pending, live, lanes }
    })
}
