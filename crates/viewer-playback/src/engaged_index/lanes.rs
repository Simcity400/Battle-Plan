
use super::Pass;
use viewer_layer_core::lane_rows::{Kept, Table};
use playhead::{Piece, Timeline, pages::Pages};
use replay_columnar::manifest::Status;
use replay_data::{
    BranchId, Hour,
    rows::{TableId, TableSchema, tables},
    scope::CaptureMode,
};
use replay_frame::frame::{Metrics, OperandStatus, Rowset, Tables};
use std::{
    io,
    sync::{Arc, atomic::Ordering},
    time::Instant,
};
use viewer_engagements::engagement_history::History;
use viewer_foundation::perspective::Country;

const READ: &[(TableId, Option<&[&str]>)] = &[
    (TableId::Campaign, None),
    (TableId::CountryLinks, None),
    (TableId::Countries, Some(&["canonical_slot", "tag"])),
    (TableId::LocalPlayer, None),
    (TableId::Players, Some(&["country"])),
    (TableId::HumanControllers, None),
    (TableId::FactionMembers, Some(&["faction"])),
    (TableId::WarRelations, Some(&["start_hour"])),
    (TableId::Subjects, Some(&["start_hour"])),
    (TableId::LandCombats, Some(&["attackers", "defenders"])),
    (TableId::LandCombatSides, Some(&["indicator", "attacker_primary", "attacker_secondary", "defender_primary", "defender_secondary"])),
    (TableId::NavalCombats, Some(&["province"])),
    (TableId::NavalCombatDetails, Some(&["sides", "custom_sea", "custom_sea_error"])),
    (TableId::NavalCombatParticipants, Some(&["ship"])),
    (TableId::NavalCombatShips, Some(&["ship", "member_source", "group_index", "live_ship", "live_convoy", "formation_flags"])),
    (TableId::TaskForces, Some(&["roster_owner", "logical_owner"])),
    (TableId::CountryCounterInputs, Some(&["forward_visibility", "zero_war", "zero_subject"])),
    (TableId::ReservedCountryNaming, Some(&["faction", "reverse_visibility"])),
    (TableId::CombatEngagementRefreshes, None),
];

const VIEWER: [TableId; 10] = [
    TableId::CountryLinks,
    TableId::Countries,
    TableId::LocalPlayer,
    TableId::Players,
    TableId::HumanControllers,
    TableId::FactionMembers,
    TableId::WarRelations,
    TableId::Subjects,
    TableId::CountryCounterInputs,
    TableId::ReservedCountryNaming,
];

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub struct Store {
    tables: Vec<Option<Box<dyn Kept>>>,
    branch: BranchId,
    hour: Hour,
    metrics: Metrics,
}

macro_rules! open {
    ($pages:expr, $run:expr, $tables:expr, $($table:ident),+) => {
        $(
            let fields = READ.iter().find(|(id, _)| *id == TableId::$table).and_then(|(_, fields)| *fields);
            $tables[TableId::$table.index()] = Some(Box::new(Table::<tables::$table>::open($pages, $run, fields, "The engaged index")?));
        )+
    };
}

fn open_optional<T: TableSchema + 'static>(
    pages: &Arc<dyn Pages>,
    run: usize,
    held: &mut [Option<Box<dyn Kept>>],
) -> io::Result<()>
where
    T::Row: Send + Default,
{
    let Some(kinds) = pages.kinds(T::ID.index()) else {
        return Ok(());
    };
    let requested = READ
        .iter()
        .find(|(id, _)| *id == T::ID)
        .and_then(|(_, fields)| *fields);
    match requested {
        None => {
            held[T::ID.index()] = Some(Box::new(Table::<T>::open(
                pages,
                run,
                None,
                "The engaged index",
            )?));
        }
        Some(fields) => {
            let present: Vec<&str> = fields
                .iter()
                .copied()
                .filter(|&name| {
                    (0..kinds.len()).any(|column| T::ID.field_name(column) == Some(name))
                })
                .collect();
            if !present.is_empty() {
                held[T::ID.index()] = Some(Box::new(Table::<T>::open(
                    pages,
                    run,
                    Some(&present),
                    "The engaged index",
                )?));
            }
        }
    }
    Ok(())
}

impl Store {
    fn open(pages: &Arc<dyn Pages>, run: usize, branch: BranchId) -> io::Result<Self> {
        let mut held: Vec<Option<Box<dyn Kept>>> = (0..TableId::ALL.len()).map(|_| None).collect();
        open!(pages, run, held, Campaign, CountryLinks, Countries, LocalPlayer, Players, HumanControllers, FactionMembers, WarRelations, Subjects, LandCombats, LandCombatSides, NavalCombats, NavalCombatDetails, NavalCombatParticipants, NavalCombatShips, TaskForces);
        open_optional::<tables::CountryCounterInputs>(pages, run, &mut held)?;
        open_optional::<tables::ReservedCountryNaming>(pages, run, &mut held)?;
        open_optional::<tables::CombatEngagementRefreshes>(pages, run, &mut held)?;
        Ok(Self { tables: held, branch, hour: 0, metrics: Metrics::default() })
    }

    fn each(&mut self, mut each: impl FnMut(&mut dyn Kept) -> io::Result<()>) -> io::Result<()> {
        self.tables.iter_mut().flatten().try_for_each(|table| each(&mut **table))
    }

    fn table<T: TableSchema>(&self) -> Option<&[T::Row]> {
        T::borrow(self.tables.get(T::ID.index())?.as_ref()?.rows())
    }

    pub fn steps(&self) -> (u64, u64) {
        self.tables.iter().flatten().map(|table| table.steps()).fold((0, 0), |(a, b), (c, d)| (a + c, b + d))
    }

    pub fn held(&self) -> (usize, playhead::lane::Held) {
        let mut total = (0, playhead::lane::Held::default());
        for table in self.tables.iter().flatten() {
            let (rows, lane) = table.bytes();
            total.0 += rows;
            total.1.cells += lane.cells;
            total.1.parts += lane.parts;
            total.1.part_bytes += lane.part_bytes;
            total.1.tracks += lane.tracks;
            total.1.track_bytes += lane.track_bytes;
        }
        total
    }
}

impl Tables for Store {
    fn of_branch(&self) -> BranchId {
        self.branch
    }

    fn at_hour(&self) -> Hour {
        self.hour
    }

    fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    fn table_status(&self, table: TableId) -> OperandStatus<'_> {
        let Some(held) = self.tables.get(table.index()).and_then(Option::as_ref) else { return OperandStatus::NotRecorded(CaptureMode::Map) };
        match held.status().0 {
            Status::Complete => OperandStatus::Complete,
            Status::Truncated { missing } => OperandStatus::Truncated(missing),
            Status::Unavailable { .. } => OperandStatus::Unavailable(held.reason()),
            Status::NotRecorded => OperandStatus::NotRecorded(CaptureMode::Map),
        }
    }

    fn table_sampled_at(&self, table: TableId) -> Option<Hour> {
        self.tables.get(table.index())?.as_ref()?.status().1
    }

    fn rows<T: TableSchema>(&self, _: u64) -> Option<Rowset<'_, T::Row>> {
        let held = self.tables.get(T::ID.index())?.as_ref()?;
        if !matches!(held.status().0, Status::Complete | Status::Truncated { .. }) {
            return None;
        }
        T::borrow(held.rows()).map(Rowset::Decoded)
    }

    fn complete_rows<T: TableSchema>(&self, mask: u64) -> io::Result<Rowset<'_, T::Row>> {
        match self.table_status(T::ID) {
            OperandStatus::Complete => self.rows::<T>(mask).ok_or_else(|| invalid(format!("hour {} {:?} is not held", self.hour, T::ID))),
            _ => Err(invalid(format!("hour {} {:?} is not whole", self.hour, T::ID))),
        }
    }
}

type FoldKey = (String, Vec<Piece>, BranchId, Country);

static FOLDS: std::sync::Mutex<Vec<(FoldKey, std::sync::Weak<History>)>> = std::sync::Mutex::new(Vec::new());

pub fn shared(replay: &replay::campaign::Replay, units: &viewer_layer_core::playhead_host::units::Units, branch: BranchId, country: &Country) -> io::Result<Arc<History>> {
    let key: FoldKey = (units.name.clone(), units.timeline.pieces().to_vec(), branch, country.clone());
    let mut folds = FOLDS.lock().unwrap_or_else(|p| p.into_inner());
    folds.retain(|(_, history)| history.strong_count() > 0);
    if let Some(history) = folds.iter().find(|(held, _)| *held == key).and_then(|(_, history)| history.upgrade()) {
        return Ok(history);
    }
    let starts = starts(replay, &units.timeline, branch);
    let history = Arc::new(History::default());
    let (held, folded, campaign, pages, timeline) = (Arc::clone(&history), country.clone(), replay.campaign_id.clone(), Arc::clone(&units.pages), units.timeline.clone());
    std::thread::Builder::new().name("engaged lanes".into()).spawn(move || {
        viewer_runtime::worker_priority::lowest_priority();
        if let Err(error) = fold(&pages, &timeline, &|at: usize| starts[at], Some(&campaign), folded, held, None, |_, _, _| {}) {
            eprintln!("The engaged index's lanes stopped: {error}");
        }
    })?;
    folds.push((key, Arc::downgrade(&history)));
    Ok(history)
}

fn starts(replay: &replay::campaign::Replay, timeline: &Timeline, branch: BranchId) -> Vec<PieceStart> {
    let history = replay.pieces(branch);
    let start = |piece: &Piece| {
        let at = history.iter().position(|held| held.first_hour <= piece.first && piece.first <= held.last_hour)?;
        let held = &history[at];
        let loaded = replay.segments_recorded().iter().find(|start| start.id == held.segment).and_then(super::native_load).filter(|hour| (held.first_hour..=held.last_hour).contains(hour));
        Some(match loaded {
            Some(hour) => (hour == piece.first, false),
            None => (false, at > 0 && held.first_hour == piece.first),
        })
    };
    timeline
        .pieces()
        .iter()
        .map(|piece| {
            let (load, restart) = start(piece).unwrap_or((false, false));
            PieceStart { branch, load, restart }
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
pub struct PieceStart {
    pub branch: BranchId,
    pub load: bool,
    pub restart: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Spent {
    pub hours: u64,
    pub read_ns: u64,
    pub fold_ns: u64,
}

pub fn fold(
    pages: &Arc<dyn Pages>,
    timeline: &Timeline,
    started: &dyn Fn(usize) -> PieceStart,
    campaign: Option<&[u8]>,
    country: Country,
    shared: Arc<History>,
    within: Option<(Hour, Hour)>,
    mut each: impl FnMut(Hour, &Store, &Spent),
) -> io::Result<Spent> {
    let mut pass = Pass::new(Arc::clone(&shared), country);
    let mut spent = Spent::default();
    for (at, piece) in timeline.pieces().iter().enumerate() {
        let (first, last) = within.map_or((piece.first, piece.last), |(first, last)| (first.max(piece.first), last.min(piece.last)));
        if first > last {
            continue;
        }
        let start = started(at);
        let began = Instant::now();
        let mut store = Store::open(pages, piece.run, start.branch)?;
        store.each(|table| table.seek(first))?;
        spent.read_ns += began.elapsed().as_nanos() as u64;
        for hour in first..=last {
            if shared.cancelled.load(Ordering::Acquire) || Arc::strong_count(&shared) == 1 {
                return Ok(spent);
            }
            if hour > first {
                let began = Instant::now();
                store.each(|table| table.step(hour))?;
                spent.read_ns += began.elapsed().as_nanos() as u64;
            }
            store.hour = hour;
            let began = Instant::now();
            let readable = campaign.is_none_or(|id| store.table::<tables::Campaign>().is_some_and(|rows| rows.len() == 1 && rows[0].id == id));
            let load = (start.load && hour == piece.first).then_some(hour);
            let restart = start.restart && hour == piece.first;
            let inputs_kept = hour > first && !VIEWER.iter().any(|table| store.tables[table.index()].as_ref().is_some_and(|held| held.touched()));
            let witness_kept = hour > first && !store.tables[TableId::CombatEngagementRefreshes.index()].as_ref().is_some_and(|held| held.touched());
            pass.fold_source(start.branch, hour, readable.then_some(&store), load, restart, inputs_kept, witness_kept);
            spent.fold_ns += began.elapsed().as_nanos() as u64;
            spent.hours += 1;
            each(hour, &store, &spent);
        }
    }
    Ok(spent)
}

#[cfg(test)]
mod tests;
