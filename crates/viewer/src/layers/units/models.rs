//! What the window draws beside the land figures: the hour's ships, raid units and convoys. They
//! come from today's unit models without their land stacks, made on the layer's own worker over
//! rows at the head (at the step that changes a table they read while playing, at once when paused
//! or after a seek), never
//! on the frame. Their rows are the tables the ships, raids and convoys are known to read
//! ([`tables`]); a table read beyond them is learned as it is read and arrives with a later step,
//! the older models standing meanwhile. A job is sent only for rows whose tables changed. The land
//! figures' meshes are the layer's own ([`super::meshes`]).

use super::meshes::{self, Chosen};
use crate::playhead_host::rows::{Cadence, RowReader, Rows, Stands};
use replay_data::{Hour, rows::TableId};
use replay_frame::frame::{DecodedFrame, discovering};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    io,
    sync::{
        Arc, Weak,
        mpsc::{self, Receiver, Sender},
    },
};
use viewer_foundation::map::{memo::TableKey, perspective::Choice};
use viewer_world::map::{
    trade_routes::ConvoyLine,
    unit_models::{KeptChoices, PreparedModels, SelectInputs, StackModel, Told},
};

/// The tables today's ships, raid units and convoys are known to read, besides the vision's
/// ([`viewer_foundation::map::perspective::VISION_TABLES`]); each group by why. A table the content's
/// scripts read beyond them is learned.
const NAMED: &[TableId] = &[
    // The clock their movement reads, and the provinces the viewer owns.
    TableId::Clock,
    TableId::Provinces,
    TableId::States,
    // A hull's model: its variant, and the country's appearance, politics and scripted values the
    // graphics rules evaluate.
    TableId::VariantDefinitions,
    TableId::VariantUpgrades,
    TableId::AvailableEquipmentVariants,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::CountryLocalSubunits,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Factions,
    TableId::FactionMembers,
    TableId::LexicalTokens,
    TableId::ScriptVariables,
    TableId::SimulationFeatures,
    // The ships, their formations and naval combats.
    TableId::Ships,
    TableId::ShipModels,
    TableId::TaskForces,
    TableId::TaskForceMapMotion,
    TableId::TaskForceCounterInputs,
    TableId::TaskForceMissions,
    TableId::NavalCombats,
    TableId::NavalCombatDetails,
    TableId::NavalCombatShips,
    // The raid arrows' units: raids, their sources, targets and the equipment they fly.
    TableId::Raids,
    TableId::RaidTargets,
    TableId::RaidCountries,
    TableId::RaidCountryInputs,
    TableId::RaidCooldowns,
    TableId::RaidEquipment,
    TableId::RaidAirSites,
    TableId::RaidPathDistances,
    TableId::RaidSpeedInputs,
    TableId::AceRangeInputs,
    TableId::AirBases,
    TableId::AirWings,
    TableId::WingEquipment,
    TableId::NavalBases,
    TableId::Nukes,
    TableId::ProvinceBuildings,
    TableId::StateFlags,
    TableId::Stockpiles,
    TableId::StockpileArchetypes,
    TableId::VariantModules,
    TableId::VariantStats,
    TableId::CountryCommandPower,
    TableId::HumanControllers,
    TableId::Players,
    TableId::ReservedCountryNaming,
    TableId::VolunteerHosts,
    // The trade routes' convoys, while the convoys layer does not draw them.
    TableId::TradeRoutes,
    TableId::ConvoyClients,
    TableId::ResourceOrigins,
    TableId::ResourceExchanges,
    TableId::RegionalConvoyEfficiency,
];

/// The tables the models' rows are declared with: [`NAMED`] and the vision's, ascending.
pub(crate) fn tables() -> Vec<TableId> {
    let mut tables: Vec<TableId> = NAMED.iter().chain(viewer_foundation::map::perspective::VISION_TABLES).copied().collect();
    tables.sort_unstable_by_key(|table| table.index());
    tables.dedup();
    tables
}

/// Today's unit models of a frame of rows, with the raid units and convoys the hour adds.
pub(crate) trait Select: Send + Sync {
    /// `convoys` are the convoys layer's lines while it draws them, today's trade routes are then
    /// not made; no battle plan or air order is made here (the raid units are the raid arrows').
    /// Without `naval` the ships are not made: the ships layer's are joined ([`Models::join_ships`]).
    /// `kept`: what the job before kept for this one, and where each part's time goes.
    fn prepare(&self, frame: &DecodedFrame, perspective: &Choice, convoys: Option<&[ConvoyLine]>, naval: bool, kept: &mut Kept) -> io::Result<PreparedModels>;

    /// One session of the land selection over `frame` for the viewer whose canonical slot is
    /// `ours`, from the answers `kept` holds, `told` the rows made anew since they were kept: the
    /// mesh of each of `stacks` that `asked` names or whose kept answers no longer stand
    /// ([`meshes`]).
    /// `divisions`: the divisions whose equipment rows `frame` holds, when it holds them by key.
    /// `parts`: each part of the session as it ends, with its nanoseconds.
    fn choose_meshes(&self, frame: &DecodedFrame, ours: u32, kept: (&KeptChoices, Option<&BTreeSet<u64>>), stacks: &HashMap<u64, meshes::Asked>, asked: &HashSet<u64>, told: &Told, parts: &mut dyn FnMut(&'static str, u64)) -> io::Result<Chosen>;
}

impl Select for crate::document::Content {
    fn choose_meshes(&self, frame: &DecodedFrame, ours: u32, (kept, divisions): (&KeptChoices, Option<&BTreeSet<u64>>), stacks: &HashMap<u64, meshes::Asked>, asked: &HashSet<u64>, told: &Told, parts: &mut dyn FnMut(&'static str, u64)) -> io::Result<Chosen> {
        // The triggers the graphics pools evaluate read the recording's executable tables.
        engine_executable::executable_tables::scope(self.tables, || {
            let mut since = std::time::Instant::now();
            let mut lap = |name: &'static str| {
                let now = std::time::Instant::now();
                parts(name, now.duration_since(since).as_nanos() as u64);
                since = now;
            };
            let hour = crate::document::Hour::new(self, frame, 0.0);
            let values = viewer_world::map::equipment_graphics::LimitValues::default();
            let evaluate = |items: &[installed_content_core::parser::Item], scopes| hour.trigger_world().evaluate(items, scopes);
            let (mut standing, mut choosing) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
            let inputs = SelectInputs {
                frame,
                limits: viewer_world::map::equipment_graphics::Limits::new(&values, &evaluate),
                ours,
                terrain: &self.counters.terrain.combat_terrain,
                localisation: &self.names.localisation,
                divisions,
                lap: Some(&mut lap),
            };
            let (models, made) = self.unit_models.land_models(inputs, kept, told, |models| {
                let mut chosen = Vec::new();
                for (&oldest, &(province, key)) in stacks {
                    let started = std::time::Instant::now();
                    let stands = !asked.contains(&oldest) && models.stands(province, key);
                    standing += started.elapsed();
                    if !stands {
                        let started = std::time::Instant::now();
                        chosen.push((oldest, models.choose(province, key)));
                        choosing += started.elapsed();
                    }
                }
                (chosen, models.made())
            })?;
            // Within the session: every held stack's check, and the stacks chosen.
            parts("um_session_stands", standing.as_nanos() as u64);
            parts("um_session_choose", choosing.as_nanos() as u64);
            let mut since = std::time::Instant::now();
            let entities: Vec<String> = models.iter().filter_map(|(_, model)| match model {
                StackModel::Chosen(chosen) => Some(chosen.entity.clone()),
                _ => None,
            }).collect();
            let mut slots = self.unit_models.chosen_meshes(&entities).into_iter();
            parts("bake", since.elapsed().as_nanos() as u64);
            since = std::time::Instant::now();
            let meshes = models
                .into_iter()
                .map(|(oldest, model)| {
                    let mesh = match model {
                        StackModel::Chosen(_) => slots.next().map_or_else(|| Err("a chosen model was not baked".to_owned()), |slot| slot.map(Some)),
                        StackModel::UnknownIdentity | StackModel::Untemplated => Ok(None),
                        StackModel::Withheld(reason) => Err(reason),
                    };
                    (oldest, mesh)
                })
                .collect();
            parts("slots", since.elapsed().as_nanos() as u64);
            Ok(Chosen { meshes, made })
        })
    }

    /// Today's models without their land stacks (the layer chooses those), with the raid units and
    /// convoys today's `orders_and_convoys_with` gives them: a raid unit is only ever placed with its
    /// drawn raid arrow, so they are the raid arrows' own, and the plan, air orders and ribbon
    /// fields it also makes are not made.
    fn prepare(&self, frame: &DecodedFrame, perspective: &Choice, layered: Option<&[ConvoyLine]>, naval: bool, kept: &mut Kept) -> io::Result<PreparedModels> {
        // The triggers the selection evaluates read the recording's executable tables.
        engine_executable::executable_tables::scope(self.tables, || {
            let Kept { convoys, parts } = kept;
            let mut since = std::time::Instant::now();
            let mut lap = |name: &'static str| {
                let now = std::time::Instant::now();
                parts.push((name, now.duration_since(since).as_nanos() as u64));
                since = now;
            };
            let hour = crate::document::Hour::new(self, frame, 0.0);
            lap("hour");
            // Whose country the viewer is, as the vision resolves it, without making the vision.
            let ours = crate::layers::intel_icons::ours(frame, perspective)?;
            lap("viewer");
            let mut units = hour.prepare_unit_models_without_land(perspective, naval, ours, &mut lap)?;
            let _cost = replay_frame::cpu::scope("viewer", Some("actors/units"));
            units.add_raid_units(&self.unit_models, &hour.raid_arrows(ours).0.raid_units);
            lap("raid_units");
            if let Ok(defines) = &self.trade_routes {
                let lines = layered.map_or_else(|| hour.trade_route_lines_kept(ours, convoys).convoys.clone(), <[ConvoyLine]>::to_vec);
                units.add_convoys(lines, defines.convoys);
            }
            lap("convoys");
            Ok(units)
        })
    }
}

/// What one models job keeps for the next, the worker's own: the trade routes' convoy lines while
/// the columns they read and the viewer are the same. And where the job's time went, by part.
#[derive(Default)]
pub(crate) struct Kept {
    convoys: Option<(crate::document::TradeRouteRead, u32, Arc<viewer_world::map::trade_routes::TradeRoutes>)>,
    parts: Vec<(&'static str, u64)>,
}

struct Job {
    frame: Arc<DecodedFrame>,
    convoys: Option<Arc<Vec<ConvoyLine>>>,
    tables: Vec<TableId>,
    perspective: Choice,
    naval: bool,
}

struct Made {
    /// The jobs it answers: newer rows replace older ones not started.
    jobs: usize,
    hour: Hour,
    /// The rows it was made from, held weakly.
    frame: Weak<DecodedFrame>,
    /// Tables read that the frame did not hold: what was made is not taken.
    missed: Vec<TableId>,
    prepared: Result<PreparedModels, String>,
    /// The worker's time on the job, and by part.
    busy_ns: u64,
    parts: Vec<(&'static str, u64)>,
}

/// What the models' worker did since last asked: jobs answered, its time on them, and that time
/// by part of a job.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub jobs: u64,
    pub busy_ns: u64,
    pub parts: Vec<(&'static str, u64)>,
}

/// What a job is made from besides its rows: the viewer, the convoys layer's product and whether
/// the ships are made.
type Asked = (Choice, Option<u64>, bool);

pub(crate) struct Models {
    select: Arc<dyn Select>,
    reader: RowReader,
    tables: Vec<TableId>,
    worker: Option<(Sender<Job>, Receiver<Made>)>,
    /// The tables of the newest job sent and what else it was made from; the frame of rows they
    /// were last keyed from, held weakly so an old frame is not kept.
    sent: Option<(TableKey, Asked)>,
    keyed: Weak<DecodedFrame>,
    in_flight: usize,
    /// Tables a job read undeclared are declared and their rows not here yet.
    learning: bool,
    pub(crate) prepared: Option<(Hour, Arc<PreparedModels>)>,
    /// The rows the models drawn stand for: those of their job, or newer ones keyed the same.
    stands: Option<Stands>,
    /// The models as made without ships, and the ships layer's number joined to them in `prepared`.
    shipless: Option<Arc<PreparedModels>>,
    joined: Option<u64>,
    pub(crate) refused: Option<String>,
    counts: std::sync::Mutex<Counts>,
}

impl Models {
    /// Its rows declared with [`tables`] and the tables `learned` before.
    pub(crate) fn new(select: Arc<dyn Select>, rows: &Arc<Rows>, learned: &[TableId]) -> Self {
        let mut tables = tables();
        tables.extend_from_slice(learned);
        tables.sort_unstable_by_key(|table| table.index());
        tables.dedup();
        Self { select, reader: rows.reader(&tables, Cadence::Ready), tables, worker: None, sent: None, keyed: Weak::new(), in_flight: 0, learning: false, prepared: None, stands: None, shipless: None, joined: None, refused: None, counts: std::sync::Mutex::default() }
    }

    /// The tables its rows are declared with, the ones learned included.
    pub(crate) fn tables(&self) -> &[TableId] {
        &self.tables
    }

    /// Models made without ships carry the ships layer's, joined again when its number moves on.
    /// Models made with ships replace them; until those arrive the ships layer's stay, so the ships
    /// are never missing between the two. Whether the models changed.
    pub(crate) fn join_ships(&mut self, ships: Option<&crate::layers::ships::Ships>) -> bool {
        let Some((hour, prepared)) = self.prepared.clone() else { return false };
        let (Some(shipless), Some(ships)) = (&self.shipless, ships) else { return false };
        if self.joined == Some(ships.seq()) {
            return false;
        }
        let (sources, report) = ships.handover();
        let joined = shipless.with_ships(sources, &report);
        joined.inherit_convoys(&prepared);
        (self.prepared, self.joined) = (Some((hour, Arc::new(joined))), Some(ships.seq()));
        true
    }

    /// Sends the newest rows when they, the viewer or the switch differ from the last sent, and
    /// takes what the worker made; whether new models arrived.
    /// `convoys`: the convoys layer's lines and their product's sequence number, while it draws.
    /// `whole`: the head of a caller that settles whole, which waits for the rows there and for the
    /// models made from them (not for tables learned: their resets come with a later step).
    /// `naval`: the ships are made with them (the ships layer does not stand in for them).
    pub(crate) fn exchange(&mut self, perspective: &Choice, convoys: Option<(u64, &[ConvoyLine])>, naval: bool, whole: Option<Hour>) -> bool {
        if let Some(head) = whole {
            // A whole settle wants the rows of its hour whatever is on the worker.
            self.reader.ready();
            let _ = self.reader.wait_through(head, std::time::Duration::from_secs(10));
        }
        if let Some(frame) = self.reader.frame() {
            let asked = (perspective.clone(), convoys.map(|(seq, _)| seq), naval);
            // The tables are keyed again only for a frame not keyed yet: the rows' cadence, not the frame rate.
            let tables = match &self.sent {
                Some(_) if std::ptr::eq(self.keyed.as_ptr(), Arc::as_ptr(&frame)) => None,
                Some((tables, _)) => Some(TableKey::of(&*frame, &self.tables)).filter(|key| key != tables),
                None => Some(TableKey::of(&*frame, &self.tables)),
            };
            // Keyed the same as the job the models drawn answer: they stand for these rows too.
            if tables.is_none()
                && self.in_flight == 0
                && !self.learning
                && let Some(stands) = &mut self.stands
                && !std::ptr::eq(stands.frame().as_ptr(), Arc::as_ptr(&frame))
            {
                *stands = Stands::at(Arc::downgrade(&frame), stands.hour().max(frame.hour));
            }
            self.keyed = Arc::downgrade(&frame);
            let key = match (tables, &self.sent) {
                (Some(tables), _) => Some((tables, asked)),
                (None, Some((tables, sent))) if *sent != asked => Some((tables.clone(), asked)),
                _ => None,
            };
            if let Some(key) = key {
                if self.worker.is_none() {
                    match spawn(Arc::clone(&self.select)) {
                        Ok(worker) => self.worker = Some(worker),
                        Err(error) => self.refused = Some(format!("The unit models' worker cannot start: {error}")),
                    }
                }
                if let Some((jobs, _)) = &self.worker
                    && jobs.send(Job { frame, convoys: convoys.map(|(_, lines)| Arc::new(lines.to_vec())), tables: self.tables.clone(), perspective: perspective.clone(), naval }).is_ok()
                {
                    self.in_flight += 1;
                    self.sent = Some(key);
                } else {
                    self.keyed = Weak::new();
                }
            }
        }
        let Some((_, made)) = &self.worker else { return false };
        let (mut arrived, mut learned) = (false, Vec::new());
        loop {
            let next = if whole.is_some() && self.in_flight > 0 { made.recv().ok() } else { made.try_recv().ok() };
            let Some(made) = next else { break };
            self.in_flight = self.in_flight.saturating_sub(made.jobs);
            let counts = self.counts.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner);
            (counts.jobs, counts.busy_ns) = (counts.jobs + 1, counts.busy_ns + made.busy_ns);
            for (name, ns) in made.parts {
                match counts.parts.iter_mut().find(|(held, _)| *held == name) {
                    Some((_, sum)) => *sum += ns,
                    None => counts.parts.push((name, ns)),
                }
            }
            if !made.missed.is_empty() {
                learned.extend(made.missed);
                continue;
            }
            match made.prepared {
                Ok(prepared) => {
                    self.learning = false;
                    let prepared = Arc::new(prepared);
                    (self.shipless, self.joined) = ((!prepared.naval()).then(|| Arc::clone(&prepared)), None);
                    self.prepared = Some((made.hour, prepared));
                    self.stands = Some(Stands::at(made.frame, made.hour));
                    self.refused = None;
                    arrived = true;
                }
                Err(error) => {
                    self.learning = false;
                    self.refused = Some(format!("Today's unit models cannot be made from the rows at the head: {error}"));
                }
            }
        }
        if !learned.is_empty() {
            self.tables.extend(learned);
            self.tables.sort_unstable_by_key(|table| table.index());
            self.tables.dedup();
            self.reader.declare(&self.tables);
            self.sent = None;
            self.learning = true;
        }
        // Its last job answered, it has finished with the rows it took.
        if self.in_flight == 0 {
            self.reader.ready();
        }
        let reader = &self.reader;
        if let Some(stands) = &mut self.stands {
            stands.look(|frame| reader.through(frame));
        }
        arrived
    }

    /// What the worker did since the last call.
    pub(crate) fn take_counts(&self) -> Counts {
        std::mem::take(&mut *self.counts.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    /// The hour of the rows the models drawn stand for; none before the first.
    pub(crate) fn stands(&self) -> Option<Hour> {
        self.stands.as_ref().map(Stands::hour)
    }

    #[cfg(test)]
    pub(crate) fn in_flight(&self) -> usize {
        self.in_flight
    }

    /// Whether models are on their way: a job out, or the rows of tables a job learned it reads.
    pub(crate) fn waiting(&self) -> bool {
        self.in_flight > 0 || self.learning
    }
}

fn spawn(select: Arc<dyn Select>) -> io::Result<(Sender<Job>, Receiver<Made>)> {
    let (jobs, receive) = mpsc::channel::<Job>();
    let (send, made) = mpsc::channel::<Made>();
    std::thread::Builder::new().name("unit models".into()).spawn(move || {
        viewer_foundation::worker_priority::background_priority();
        let mut kept = Kept::default();
        while let Ok(mut job) = receive.recv() {
            let mut jobs = 1;
            while let Ok(newer) = receive.try_recv() {
                (job, jobs) = (newer, jobs + 1);
            }
            let started = std::time::Instant::now();
            let (prepared, missed) = discovering(|| select.prepare(&job.frame, &job.perspective, job.convoys.as_deref().map(Vec::as_slice), job.naval, &mut kept));
            let parts = std::mem::take(&mut kept.parts);
            let missed = missed.into_iter().filter(|table| !job.tables.contains(table)).collect();
            let busy_ns = started.elapsed().as_nanos() as u64;
            if send.send(Made { jobs, hour: job.frame.hour, frame: Arc::downgrade(&job.frame), missed, prepared: prepared.map_err(|error| error.to_string()), busy_ns, parts }).is_err() {
                return;
            }
        }
    })?;
    Ok((jobs, made))
}

#[cfg(test)]
mod tests;
