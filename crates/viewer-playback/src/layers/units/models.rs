//! What the window draws beside the land figures: the hour's ships, raid units and convoys. They
//! come from today's unit models without their land stacks, made on the layer's own worker over
//! rows at the head, never on the frame. The land figures' meshes are the layer's own
//! ([`super::meshes`]).
//!
//! The worker owns its reader of the map side's view store, whose rows are made from the side's
//! own mirrors inside the step, so they stand at the hour the side applied. The layer tells it at
//! the step's apply what the models are made for (the viewer, the convoys layer's lines, whether
//! the ships are made): it builds the newest frame of rows it has not built, publishes the
//! models at the hour of those rows, and asks for the next. A build begun is always finished and
//! always published, and only the newest frame is ever taken, so the models are at most one build
//! behind the rows and the hour they stand at only moves forward. A step's settle takes what is
//! published and waits, within the step's budget, only while the models it holds stand before the
//! step's hour. So the build runs beside the step's other makers instead of after them, and a step
//! whose budget is already spent is offered the models of an earlier hour rather than none.
//!
//! Their rows are the tables the ships, raids and convoys are known to read ([`tables`]); a table
//! read beyond them is learned as it is read and its rows arrive with a later frame, the older
//! models standing meanwhile. A build is begun only for rows whose tables changed.

use super::meshes::{self, Chosen};
use crate::playhead_host::{
    timeline::{self, Mark},
    views::{self, Need},
};
use replay_data::{Hour, rows::TableId};
use replay_frame::frame::{DecodedFrame, discovering};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    io,
    sync::{Arc, Condvar, Mutex, MutexGuard},
    thread::JoinHandle,
    time::{Duration, Instant},
};
use viewer_foundation::map::{memo::TableKey, perspective::Choice};
use viewer_world::map::{
    trade_routes::ConvoyLine,
    unit_models::{KeptChoices, PreparedModels, SelectInputs, StackModel, Told},
};

/// How long a settle that waits however long the models take gives them.
const WHOLE: Duration = Duration::from_secs(10);

/// How long the worker waits for the rows to move before looking at what the layer asks of it (a
/// new viewing country, new convoy lines): half a frame of a 60 Hz display, since the rows of every
/// step move the Clock the models read, so the rows are what wakes it while the map plays.
const LOOK: Duration = Duration::from_millis(8);

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
pub fn tables() -> Vec<TableId> {
    let mut tables: Vec<TableId> = NAMED.iter().chain(viewer_foundation::map::perspective::VISION_TABLES).copied().collect();
    tables.sort_unstable_by_key(|table| table.index());
    tables.dedup();
    tables
}

/// `tables` widened by `learned`, ascending.
fn declared(learned: &[TableId]) -> Vec<TableId> {
    let mut tables = tables();
    tables.extend_from_slice(learned);
    tables.sort_unstable_by_key(|table| table.index());
    tables.dedup();
    tables
}

/// Today's unit models of a frame of rows, with the raid units and convoys the hour adds.
pub trait Select: Send + Sync {
    /// `convoys` are the convoys layer's lines while it draws them, today's trade routes are then
    /// not made; no battle plan or air order is made here (the raid units are the raid arrows').
    /// Without `naval` the ships are not made: the ships layer's are joined ([`Models::join_ships`]).
    /// `kept`: what the build before kept for this one, and where each part's time goes.
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
            viewer_foundation::view_record::add("units", "stacks_walked", stacks.len() as u64);
            viewer_foundation::view_record::add("units", "stacks_asked", asked.len() as u64);
            viewer_foundation::view_record::add("units", "stacks_sessions", 1);
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
                let made = layered.is_none().then(|| hour.trade_route_convoys_kept(ours, convoys));
                lap("convoy_lines");
                let lines = match (&made, layered) {
                    (Some(made), _) => (**made).clone(),
                    (None, Some(lines)) => lines.to_vec(),
                    (None, None) => Vec::new(),
                };
                lap("convoy_copy");
                units.add_convoys(lines, defines.convoys);
            }
            lap("convoy_add");
            Ok(units)
        })
    }
}

/// What one models build keeps for the next, the worker's own: the trade routes' lines while the
/// columns they read and the viewer are the same, and their convoys made again from those lines
/// while only the convoys assigned moved. And where the build's time went, by part.
#[derive(Default)]
pub struct Kept {
    convoys: Option<crate::document::KeptConvoys>,
    parts: Vec<(&'static str, u64)>,
}

/// What the models are made for besides their rows: the viewer, the convoys layer's lines with the
/// number of the product they came from, and whether the ships are made.
#[derive(Clone)]
struct Asked {
    perspective: Choice,
    convoys: Option<(u64, Arc<Vec<ConvoyLine>>)>,
    naval: bool,
}

/// What of an ask decides a build: two asks that key the same make the same models of the same rows.
type Key = (Choice, Option<u64>, bool);

impl Asked {
    fn key(&self) -> Key {
        (self.perspective.clone(), self.convoys.as_ref().map(|&(seq, _)| seq), self.naval)
    }
}

/// The newest models the worker made, at the hour of the rows they were made from.
#[derive(Clone)]
struct Made {
    /// The picture they were made for ([`Held::generation`]).
    generation: u64,
    /// Models published up to and including these, so a settle takes only ones it does not hold.
    stamp: u64,
    /// The hour of the rows they were made from.
    hour: Hour,
    /// The newest hour those rows stand at: a step that changed no table of theirs moves it with no
    /// build at all ([`RowReader::through`]).
    stands: Hour,
    /// The frame they were made from while that frame is the reader's newest, which is while
    /// `stands` can still move; let go after.
    frame: Option<Arc<DecodedFrame>>,
    prepared: Arc<PreparedModels>,
}

/// What the models' worker did since last asked: builds answered, its time on them, and that time
/// by part of a build.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub jobs: u64,
    pub busy_ns: u64,
    pub parts: Vec<(&'static str, u64)>,
}

impl Counts {
    fn add(&mut self, busy_ns: u64, parts: Vec<(&'static str, u64)>) {
        (self.jobs, self.busy_ns) = (self.jobs + 1, self.busy_ns + busy_ns);
        for (name, ns) in parts {
            match self.parts.iter_mut().find(|(held, _)| *held == name) {
                Some((_, sum)) => *sum += ns,
                None => self.parts.push((name, ns)),
            }
        }
    }
}

/// What the layer and its worker share.
#[derive(Default)]
struct Held {
    /// What the models are made for; none until the layer has a perspective.
    asked: Option<Asked>,
    /// The picture models are made for, bumped by every compose: models of the picture before are
    /// still drawn, and never counted as this picture's.
    generation: u64,
    made: Option<Made>,
    published: u64,
    /// The hour the newest rows the worker has answered stand at: it owes nothing up to there,
    /// whether it built them, found them keyed as the models it holds, or was refused by them.
    answered: Option<Hour>,
    /// Why the models could not be made, by name.
    refused: Option<String>,
    /// A build is running, or the rows of tables a build learned it reads have not arrived.
    working: bool,
    /// The tables its rows are declared with, the learned ones included.
    tables: Vec<TableId>,
    counts: Counts,
    stop: bool,
}

struct Shared {
    held: Mutex<Held>,
    /// Models published, or the rows stopped.
    done: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Waits for the worker to publish, until `until`.
    fn wait(&self, until: Instant) {
        let held = self.lock();
        let _ = self.done.wait_timeout(held, until.saturating_duration_since(Instant::now()));
    }

    /// The worker has answered the rows standing at `stands`: it owes nothing up to there.
    fn answered(&self, stands: Hour) {
        let mut held = self.lock();
        (held.answered, held.working) = (held.answered.max(Some(stands)), false);
        drop(held);
        self.done.notify_all();
    }

    /// The models published stand at `stands` too: the rows there key the same as theirs, so no
    /// build is owed for them.
    fn stood(&self, generation: u64, stands: Hour, frame: &Arc<DecodedFrame>) {
        let mut held = self.lock();
        let stamp = held.published + 1;
        let moved = held.generation == generation
            && held.made.as_mut().is_some_and(|made| {
                if made.generation != generation {
                    return false;
                }
                (made.stands, made.frame, made.stamp) = (made.stands.max(stands), Some(Arc::clone(frame)), stamp);
                true
            });
        if moved {
            held.published = stamp;
        }
        drop(held);
        self.done.notify_all();
    }
}

/// The worker: every frame of rows it has not built, built whole and published.
fn work(shared: &Shared, select: &dyn Select, reader: &views::Reader, maker: timeline::Maker) {
    let mut kept = Kept::default();
    let mut tables = shared.lock().tables.clone();
    // The rows and ask of the newest build begun, and those the models published were made from.
    let (mut built, mut standing): (Option<(TableKey, Key)>, Option<(TableKey, Key)>) = (None, None);
    // The frame it has answered, built or stood, by the store's count of frames made for it.
    let mut handled: Option<(u64, Key)> = None;
    let mut generation = 0;
    loop {
        let (asked, want) = {
            let held = shared.lock();
            if held.stop {
                return;
            }
            (held.asked.clone(), held.generation)
        };
        // A new picture: models of the one before are not its models, however new the rows they hold.
        if want != generation {
            (generation, built, standing, handled) = (want, None, None, None);
        }
        let Some(asked) = asked else {
            shared.lock().working = false;
            reader.wait_newer(u64::MAX, LOOK);
            continue;
        };
        let key = asked.key();
        let seen = handled.as_ref().map_or(0, |&(published, _)| published);
        let Some((newest, published)) = reader.newest() else {
            reader.wait_newer(seen, LOOK);
            continue;
        };
        // Rows of the seek before are not this picture's: the step after its compose makes the
        // rows of the hour sought.
        let Some((through, seek)) = reader.through_seek(Arc::as_ptr(&newest)) else {
            continue;
        };
        if seek != reader.seek() {
            reader.wait_newer(published, LOOK);
            continue;
        }
        let stands = through.max(newest.hour);
        // The frame it has answered already, built or stood: nothing to look at until a newer one,
        // and its rows stand at every later hour no step changed them by.
        if handled.as_ref() == Some(&(published, key.clone())) {
            shared.answered(stands);
            reader.wait_newer(published, LOOK);
            continue;
        }
        handled = Some((published, key.clone()));
        timeline::mark(maker, Mark::Rows);
        let at = (TableKey::of(&*newest, &tables), key);
        // The same tables in the same allocations and the same ask: the models built from them
        // stand at these rows' hour with no build at all, as the hour they are of is their last change.
        if built.as_ref() == Some(&at) {
            if standing.as_ref() == Some(&at) {
                shared.stood(generation, stands, &newest);
            }
            shared.answered(stands);
            continue;
        }
        let frame = newest;
        let Asked { perspective, convoys, naval } = asked;
        shared.lock().working = true;
        timeline::mark(maker, Mark::BuildBegan);
        let started = Instant::now();
        let (prepared, read) = discovering(|| select.prepare(&frame, &perspective, convoys.as_ref().map(|(_, lines)| &lines[..]), naval, &mut kept));
        timeline::mark(maker, Mark::BuildEnded);
        let busy_ns = started.elapsed().as_nanos() as u64;
        let parts = std::mem::take(&mut kept.parts);
        built = Some(at.clone());
        shared.lock().counts.add(busy_ns, parts);
        // Tables the build read that the frame did not hold: what was made is not taken, and the
        // frame after the declaration holds them.
        let missed: Vec<TableId> = read.into_iter().filter(|table| !tables.contains(table)).collect();
        if !missed.is_empty() {
            tables.extend(missed);
            tables.sort_unstable_by_key(|table| table.index());
            tables.dedup();
            reader.declare_only(&tables);
            let mut held = shared.lock();
            (held.tables, held.working) = (tables.clone(), true);
            drop(held);
            (built, standing, handled) = (None, None, None);
            continue;
        }
        let mut held = shared.lock();
        if held.stop {
            return;
        }
        // A compose while it built: the models are the picture before's and are not published as
        // this one's.
        if held.generation != generation {
            (built, standing, handled) = (None, None, None);
            continue;
        }
        match prepared {
            Ok(prepared) => {
                let stamp = held.published + 1;
                (held.published, held.refused) = (stamp, None);
                held.made = Some(Made { generation, stamp, hour: frame.hour, stands, frame: Some(frame), prepared: Arc::new(prepared) });
                standing = Some(at);
            }
            Err(error) => held.refused = Some(format!("Today's unit models cannot be made from the rows at the head: {error}")),
        }
        (held.answered, held.working) = (held.answered.max(Some(stands)), false);
        drop(held);
        shared.done.notify_all();
    }
}

pub struct Models {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    /// The side's rows the worker builds from; the layer asks it the hour they stand at.
    reader: Arc<views::Reader>,
    maker: timeline::Maker,
    /// The convoy lines last given to the worker, with the number of the product they came from:
    /// copied again only when the convoys layer publishes a new one.
    lines: Option<(u64, Arc<Vec<ConvoyLine>>)>,
    /// The newest models taken from the worker: drawn whatever picture they were made for, counted
    /// as this picture's only while their generation is this one's.
    made: Option<Made>,
    /// The picture it asks models for, bumped by every compose.
    generation: u64,
    /// The models as made without ships, and the ships layer's number joined to them in `prepared`.
    shipless: Option<Arc<PreparedModels>>,
    joined: Option<u64>,
    pub prepared: Option<(Hour, Arc<PreparedModels>)>,
    pub refused: Option<String>,
}

impl Models {
    /// Its rows declared with [`tables`] and the tables `learned` before, read from the map side's
    /// view store, and the worker that builds from them.
    pub fn new(select: Arc<dyn Select>, views: &views::Views, learned: &[TableId]) -> Self {
        let tables = declared(learned);
        let reader = Arc::new(views.reader("unit_models", &tables.iter().copied().map(Need::every).collect::<Vec<Need>>()));
        let maker = timeline::maker("unit_models");
        let shared = Arc::new(Shared { held: Mutex::new(Held { tables, ..Held::default() }), done: Condvar::new() });
        let worker = std::thread::Builder::new().name("unit models".into()).spawn({
            let (shared, reader) = (Arc::clone(&shared), Arc::clone(&reader));
            move || {
                // A step's settle joins its product: at the head's priority, never below normal.
                viewer_foundation::worker_priority::enter(viewer_foundation::worker_priority::Class::Kept);
                work(&shared, &*select, &reader, maker);
            }
        });
        let (worker, refused) = match worker {
            Ok(worker) => (Some(worker), None),
            Err(error) => (None, Some(format!("The unit models' worker cannot start: {error}"))),
        };
        Self { shared, worker, reader, maker, lines: None, made: None, generation: 0, shipless: None, joined: None, prepared: None, refused }
    }

    /// Its place in the step's timeline.
    pub fn maker(&self) -> timeline::Maker {
        self.maker
    }

    /// The tables its rows are declared with, the ones learned included.
    pub fn tables(&self) -> Vec<TableId> {
        self.shared.lock().tables.clone()
    }

    /// The models held are the old picture's: they are drawn until ones made from rows after the
    /// reset arrive, and counted as this picture's only then.
    pub fn composed(&mut self) {
        self.generation += 1;
        let mut held = self.shared.lock();
        (held.generation, held.answered) = (self.generation, None);
    }

    /// Models made without ships carry the ships layer's, joined again when its number moves on.
    /// Models made with ships replace them; until those arrive the ships layer's stay, so the ships
    /// are never missing between the two. Whether the models changed.
    pub fn join_ships(&mut self, ships: Option<&crate::layers::ships::Ships>) -> bool {
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

    /// What the worker builds the models of the rows it is given next for: the viewer, the convoys
    /// layer's lines while it draws them with the number of its product, and whether the ships are
    /// made (the ships layer does not stand in for them). Told at the step's apply, so the build
    /// runs beside the step's other makers.
    pub fn ask(&mut self, perspective: &Choice, convoys: Option<(u64, &[ConvoyLine])>, naval: bool) {
        // The lines are copied only for a product the worker was not given yet.
        self.lines = match (convoys, self.lines.take()) {
            (None, _) => None,
            (Some((seq, _)), Some((held, lines))) if held == seq => Some((seq, lines)),
            (Some((seq, lines)), _) => Some((seq, Arc::new(lines.to_vec()))),
        };
        let asked = Asked { perspective: perspective.clone(), convoys: self.lines.clone(), naval };
        self.shared.lock().asked = Some(asked);
        timeline::mark(self.maker, Mark::Asked);
    }

    /// The models of the rows at `hour`: the newest models published are taken, and a step waits
    /// for nothing, keeping the models it has at their own hour and joining newer ones at the step
    /// after. A settle that waits ([`crate::layers::Settling::waits`]) waits for models standing
    /// at or past `hour`; one whose rows the worker has answered without making models (refused,
    /// or a table it learned) waits no longer. Whether the models changed.
    pub fn join(&mut self, hour: Option<Hour>, settling: crate::layers::Settling) -> bool {
        let Some(hour) = hour else { return self.take() };
        let until = Instant::now() + settling.waiting(WHOLE);
        let mut arrived = false;
        loop {
            arrived |= self.take();
            if self.stands().is_some_and(|stands| stands >= hour) || self.shared.lock().answered.is_some_and(|at| at >= hour) {
                return arrived;
            }
            if Instant::now() >= until {
                return arrived;
            }
            self.shared.wait(until);
        }
    }

    /// Takes the newest models published for the picture being drawn, and moves the hour the models
    /// it holds stand at to every later hour no step changed a table of their rows. Whether new
    /// models arrived.
    fn take(&mut self) -> bool {
        let mut arrived = false;
        {
            let held = self.shared.lock();
            self.refused.clone_from(&held.refused);
            if let Some(made) = held.made.as_ref().filter(|made| made.generation == self.generation)
                && self.made.as_ref().is_none_or(|ours| ours.stamp < made.stamp)
            {
                arrived = self.made.as_ref().is_none_or(|ours| !Arc::ptr_eq(&ours.prepared, &made.prepared));
                self.made = Some(made.clone());
            }
        }
        if let Some(made) = &mut self.made
            && let Some(frame) = &made.frame
        {
            match self.reader.through(Arc::as_ptr(frame)) {
                Some(through) => made.stands = made.stands.max(through),
                None => made.frame = None,
            }
        }
        if arrived && let Some(made) = &self.made {
            let prepared = Arc::clone(&made.prepared);
            (self.shipless, self.joined) = ((!prepared.naval()).then(|| Arc::clone(&prepared)), None);
            self.prepared = Some((made.hour, prepared));
        }
        arrived
    }

    /// What the worker did since the last call.
    pub fn take_counts(&self) -> Counts {
        std::mem::take(&mut self.shared.lock().counts)
    }

    /// The hour of the rows the models drawn stand for; none before the first of this picture.
    pub fn stands(&self) -> Option<Hour> {
        self.made.as_ref().filter(|made| made.generation == self.generation).map(|made| made.stands)
    }

    /// Whether a build is running or the rows of tables a build learned it reads are on their way.
    #[cfg(any(test, feature = "test-support"))]
    pub fn in_flight(&self) -> usize {
        usize::from(self.shared.lock().working)
    }

    /// The hour of the newest rows its worker has answered: it owes nothing up to there, whether
    /// it built them, found them keyed as the models it holds, or was refused by them.
    #[cfg(any(test, feature = "test-support"))]
    pub fn answered(&self) -> Option<Hour> {
        self.shared.lock().answered
    }

    /// Whether models are on their way: a build running, or the rows of tables a build learned it
    /// reads.
    pub fn waiting(&self) -> bool {
        self.shared.lock().working
    }
}

impl Drop for Models {
    fn drop(&mut self) {
        self.shared.lock().stop = true;
        self.shared.done.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests;
