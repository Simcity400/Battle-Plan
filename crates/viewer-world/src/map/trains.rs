//! Supply trains from recorded supply routes and content-defined consists.
//!
//! One railway-manager countdown attempts a departure at an inclusive whole-
//! millisecond interval between the timetable's ordered first and last entries.
//! Eligible supply nodes have a clear blocked flag, an admitted kind, and positive
//! available supply minus consumed supply, rounded up to whole fixed5 units.
//! A weighted draw chooses a node; its parent chain removes consecutive duplicate
//! provinces and reverses each land portion, travelling from the capital side to
//! the chosen hub. The viewing country's inventory counts cap the fleet and
//! weight equipment selection. Content lookup tries the variant creator's
//! original-country table, culture table and default, each by type then archetype.
//!
//! One tick creates a consist per land portion and checks the fleet cap once
//! before route construction. Distance advances by `NGraphics.TRAIN_MAP_SPEED`
//! times frame seconds even when hidden. Invalid or access-denied routes,
//! cooling-down provinces and travel beyond the final segment plus consist
//! length destroy the train. Cars draw while railway fade is at least 0.001,
//! with that fade in their TrainColor alpha.
//!
//! Ordered minimum/maximum wagon defines supply an inclusive random count,
//! increased by one and floored to the number of authored fixed wagon slots.
//! Fixed `wagon_0/1/2` entries precede random entries; `wagon_last` follows them.
//! Authored length, scale, rotation and shift link the cars. Chord fitting takes
//! at most ten iterations on bends. Back couplings use water-floored terrain
//! height plus shift; the body faces the preceding car ([`car_yaw`]). Endpoint
//! planes clip and fade the cars over 2.5 units. Consist length uses the authored
//! car-length sum, which can differ from the game's mesh-bound-based length.
//!
//! # Recorded inputs and presentation phase
//!
//! `SupplyNodes`, `TrainInventory` and selected parent edges replace live path
//! finding and equipment selection. Missing complete inputs suppress trains;
//! every parent-chain vertex needs a row. Missing-route breaks derive from
//! recorded supply nodes, countries, links and provinces ([`FallbackEndpoints`]).
//! Route geometry uses the railway pass's curves through `RailGeometry::train_route`.
//! Equipment tokens resolve through the hour's `LexicalTokens` and the content
//! registry's token identity (`train_gfx::token_key`).
//!
//! Variant creator links choose a signed-positive original tag before the own
//! tag. Raw creator zero uses reserved-country naming. A selected tag must be
//! signed-positive to reach either the country or culture table. Culture lookup
//! resolves the recorded runtime source through the content registry ([`Cultures`]);
//! the reserved creator's unrecorded runtime culture refuses that variant.
//!
//! Scheduler state, pre-existing trains and original animation phase are
//! unrecorded. Presentation continues while paused and keeps its fleet, timetable
//! and wheel/particle history across forward skips ([`TrainPlan::carry_presentation`]).
//! Positive window time samples visible idle-state wheels and retained move-state
//! exhaust through `entity_animation`; time-zero dumps retain the default pose.

use installed_content_core::{
    ResolvedLayers, defines::Defines, train_gfx, train_gfx::Consist, train_gfx::Wagon,
};
use installed_content_image::native::Decode;
use replay_data::{
    rows::CountryAppearanceRow, rows::CountryLinkRow, rows::CountryRow, rows::ProvinceRow,
    rows::SupplyNodeRow, rows::TrainAllocation, rows::VariantDefinitionRow, rows::tables,
};
use replay_frame::frame::DecodedFrame;
#[cfg(test)]
use std::cell::Cell;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
use viewer_foundation::map::presentation::Continuity;
use viewer_foundation::table_access::TokenNames;
use {
    super::entity_animation::Random,
    super::models::{KeyedInstance, ModelField, ModelInstance, ModelLibrary},
    viewer_foundation::map::{
        province_map::HeightField,
        tables::{core, rows},
    },
};

pub const SEED: u32 = 0x5452_4149;

/// Consists longer than this are refused rather than allocated.
const CAR_LIMIT: i32 = u16::MAX as i32;

/// The content's train operands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrainDefines {
    /// `NGraphics.TRAIN_MAP_SPEED`, world units per second.
    pub speed: f32,
    /// `NGraphics.TUNNELBANA_TIMETABLE`'s first and last entries, ordered,
    /// in whole milliseconds.
    pub timetable: [i32; 2],
    /// `NGraphics.MIN_TRAIN_WAGON_COUNT` and `MAX_TRAIN_WAGON_COUNT`, each at
    /// least 1 as its loader leaves it, then ordered.
    pub wagon_count: [i32; 2],
}

impl TrainDefines {
    pub fn load(defines: &Defines) -> io::Result<Self> {
        let speed = defines.number("NGraphics", "TRAIN_MAP_SPEED")? as f32;
        if !(speed.is_finite() && speed > 0.0) {
            return Err(invalid(format!(
                "NDefines.NGraphics.TRAIN_MAP_SPEED {speed} is not a positive speed; \
                 a stationary or reversing train fleet is not supported"
            )));
        }
        // The timetable reads the first and last entries of an integer array.
        // Conversion of a fractional entry remains unverified
        // (truncated here).
        let table = defines.numbers("NGraphics", "TUNNELBANA_TIMETABLE")?;
        let whole = |value: f64| {
            let value = value.trunc();
            (f64::from(i32::MIN)..=f64::from(i32::MAX))
                .contains(&value)
                .then_some(value as i32)
        };
        let (Some(first), Some(last)) = (
            table.first().copied().and_then(whole),
            table.last().copied().and_then(whole),
        ) else {
            return Err(invalid(
                "NDefines.NGraphics.TUNNELBANA_TIMETABLE needs integral first and last entries",
            ));
        };
        let timetable = [first.min(last), first.max(last)];
        if timetable[0] < 1 {
            return Err(invalid(format!(
                "NDefines.NGraphics.TUNNELBANA_TIMETABLE {timetable:?} allows a zero interval, \
                 one departure per rendered frame, which is not supported"
            )));
        }
        // Each loader raises counts below 1 to 1, including the zero stored
        // for missing or non-number values, before ordering the pair.
        let min = defines.int32("NGraphics", "MIN_TRAIN_WAGON_COUNT")?.max(1);
        let max = defines.int32("NGraphics", "MAX_TRAIN_WAGON_COUNT")?.max(1);
        let wagon_count = [min.min(max), min.max(max)];
        if wagon_count[1] >= CAR_LIMIT {
            return Err(invalid(format!(
                "NDefines.NGraphics.MAX_TRAIN_WAGON_COUNT {} is beyond the supported {CAR_LIMIT} cars",
                wagon_count[1]
            )));
        }
        Ok(Self {
            speed,
            timetable,
            wagon_count,
        })
    }

    /// The next timetable interval, in seconds.
    fn interval(&self, random: u32) -> f64 {
        let [lo, hi] = self.timetable.map(i64::from);
        (lo + i64::from(random & 0x7fff_ffff) % (hi - lo + 1)) as f64 / 1000.0
    }

    /// The drawn car count before the authored slots' floor.
    fn cars(&self, random: u32) -> i64 {
        let [lo, hi] = self.wagon_count.map(i64::from);
        lo + 1 + i64::from(random & 0x7fff_ffff) % (hi - lo + 1)
    }
}

#[derive(Debug)]
pub struct TrainAssets {
    pub library: ModelLibrary,
    /// The native graphical-culture registry, for the reached runtime lookup.
    pub cultures: Cultures,
    database: train_gfx::Database,
    /// The declared equipment types by the lexer's token identity.
    equipment: installed_content_core::equipment::Database,
    pub defines: TrainDefines,
    /// Route curves by province path, kept while the rail network they follow is unchanged.
    routes: RouteCache,
}

/// A path's curve and cumulative distances, or `None` where no train runs it.
type Curve = Option<Arc<(super::railways::TrainRoute, Vec<f32>)>>;

/// Curves of the networks hours still hold: hours built at once may hold different networks.
#[derive(Default)]
struct RouteCache(std::sync::Mutex<Vec<(CurveKey, Arc<std::sync::Mutex<std::collections::HashMap<Vec<u32>, Curve>>>)>>);

/// The rail geometry, heightmap and network a curve was sampled on.
struct CurveKey {
    rails: u64,
    heights: (usize, usize),
    network: std::sync::Weak<super::railways::RailNetwork>,
}

impl RouteCache {
    fn curves(
        &self,
        rails: &super::railways::RailGeometry,
        heights: &HeightField<'_>,
        network: &Arc<super::railways::RailNetwork>,
    ) -> Arc<std::sync::Mutex<std::collections::HashMap<Vec<u32>, Curve>>> {
        let key = (rails.projection_id(), (heights.values.as_ptr() as usize, heights.values.len()));
        let mut kept = self.0.lock().unwrap_or_else(|error| error.into_inner());
        kept.retain(|(held, _)| held.network.strong_count() > 0);
        let network = Arc::downgrade(network);
        if let Some((_, curves)) = kept.iter().find(|(held, _)| {
            held.rails == key.0 && held.heights == key.1 && held.network.ptr_eq(&network)
        }) {
            return Arc::clone(curves);
        }
        let curves = Arc::default();
        kept.push((
            CurveKey {
                rails: key.0,
                heights: key.1,
                network,
            },
            Arc::clone(&curves),
        ));
        curves
    }
}

impl std::fmt::Debug for RouteCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RouteCache")
    }
}

impl TrainAssets {
    /// The train database, its entities from the whole descriptor roster, the
    /// equipment archetypes and the train defines, all from the content stack.
    pub fn load(
        layers: &ResolvedLayers,
        decode: Decode,
        defines: &Defines,
        warnings: &mut Vec<installed_content_core::Diagnostic>,
    ) -> io::Result<Self> {
        let defines = TrainDefines::load(defines)?;
        let roster = layers.descriptor_roster()?;
        let index = country_tag_index(layers)?;
        warnings.extend(
            index
                .warning()
                .map(|message| (installed_content_core::DiagnosticKind::Warning, message)),
        );
        let tags = index.files;
        let cultures = Cultures::read(layers)?;
        let is_tag = |tag: &str| tags.contains_key(tag);
        let has_entity = |entity: &str| roster.descriptors.entities.contains_key(entity);
        let database = train_gfx::Database::read(
            layers,
            &train_gfx::Registries {
                is_tag: &is_tag,
                cultures: &cultures.registered,
                has_entity: &has_entity,
            },
        )?;
        if !database.diagnostics.is_empty() {
            warnings.push((
                installed_content_core::DiagnosticKind::Refused,
                format!(
                    "Train database entries the engine also rejects: {}.",
                    database.diagnostics.join("; ")
                ),
            ));
        }
        let equipment = installed_content_core::equipment::Database::load_complete(layers)?;
        let names: Vec<String> = database
            .wagons
            .iter()
            .map(|wagon| wagon.entity.clone())
            .collect();
        let library = ModelLibrary::load(layers, decode, &roster.descriptors, &names, warnings)?;
        Ok(Self {
            library,
            cultures,
            database,
            equipment,
            defines,
            routes: RouteCache::default(),
        })
    }

    /// The declared equipment type a variant's token spelling names. The
    /// engine builds a type's name and token from one authored key, and the
    /// database finds a name without case, as
    /// the lexer's fold makes one token of names equal under it.
    fn equipment_key(&self, spelling: &str) -> Result<&str, String> {
        let at = self.equipment.get(spelling).ok_or_else(|| {
            format!(
                "equipment {spelling} is not declared in {}",
                installed_content_core::equipment::DIRECTORY
            )
        })?;
        self.name(at)
    }

    /// The archetype name, whose token is tried after the equipment type's.
    /// A type without an archetype points at the null object, whose token the
    /// content does not give: refused by name.
    fn archetype(&self, equipment: &str) -> Result<&str, String> {
        let archetype = self
            .equipment
            .named(equipment)
            .and_then(|kind| kind.archetype)
            .ok_or_else(|| {
                format!(
                    "equipment {equipment} has no archetype; its null-object fallback token is unavailable, so its trains are not drawn"
                )
            })?;
        self.name(archetype)
    }

    /// A type's name as the port's text, or why it is not.
    fn name(&self, at: usize) -> Result<&str, String> {
        let kind = &self.equipment.equipment[at];
        kind.name_str().map_err(|refused| {
            format!(
                "equipment {} cannot be named: {refused}",
                String::from_utf8_lossy(&kind.name)
            )
        })
    }

    fn wagon(&self, index: usize) -> &Wagon {
        &self.database.wagons[index]
    }
}

/// The country tag index, shared through `installed_content_core::country_tags`.
pub use installed_content_core::country_tags::country_tag_index;

/// The names the game registers from the recorded content stack, including
/// null object. Resolve the recorded runtime culture source into a content entry by exact length and bytes, returning null
/// for an unknown name. No country history is read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cultures {
    registered: BTreeSet<String>,
}

impl Cultures {
    /// Reads the authored registry and admits the native index-zero object to
    /// both the train database reader and runtime source resolution.
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let registered = train_gfx::cultures(layers)?;
        Ok(Self { registered })
    }

    /// Exact source-name identity. Unknown names resolve to no entry
    /// and skip the culture table; bytes this reader cannot represent refuse.
    fn resolve(&self, bytes: &[u8]) -> Result<Option<&str>, String> {
        let name = std::str::from_utf8(bytes)
            .map_err(|_| "train variant creator runtime graphical culture is not UTF-8")?;
        Ok(self.registered.get(name).map(String::as_str))
    }
}

/// Resolve the creator reference, including the
/// reserved zero object, and gates both non-default tables on the selected
/// signed-positive tag. The game tries type then archetype within each table.
struct CreatorLookup<'a> {
    frame: &'a DecodedFrame,
    links: BTreeMap<u32, u32>,
    countries: BTreeMap<u32, &'a CountryRow>,
    appearance: BTreeMap<u32, &'a CountryAppearanceRow>,
    appearance_complete: bool,
    cultures: &'a Cultures,
}

impl CreatorLookup<'_> {
    fn consist(
        &self,
        database: &train_gfx::Database,
        variant: &VariantDefinitionRow,
        equipment: &str,
        archetype: &str,
    ) -> Result<Option<Consist>, String> {
        self.lookup(database, variant.creator_link, equipment, archetype)
            .map(|found| found.map(|(_, consist)| consist.clone()))
            .map_err(|reason| {
                format!(
                    "variant {} creator: {reason}; its trains are not drawn",
                    variant.id
                )
            })
    }

    fn lookup<'d>(
        &self,
        database: &'d train_gfx::Database,
        creator_link: u32,
        equipment: &str,
        archetype: &str,
    ) -> Result<Option<(train_gfx::Level, &'d Consist)>, String> {
        let default = || database.lookup(None, None, equipment, archetype);
        let (slot, creator) = if creator_link == 0 {
            (0, None)
        } else {
            let slot = self
                .links
                .get(&creator_link)
                .ok_or("train variant creator absent from CountryLinks")?;
            (
                *slot,
                Some(
                    *self
                        .countries
                        .get(slot)
                        .ok_or("train variant creator absent from Countries")?,
                ),
            )
        };
        // The game gates both tables on the selected raw word, independently
        // of tag-string length. A positive-empty original retains this gate.
        if installed_content_country_names::country_names::original_or_own_reference(self.frame, slot)? <= 0
        {
            return Ok(default());
        }
        let (tag, appearance) = if let Some(creator) = creator {
            let appearance = self.appearance.get(&creator.id).ok_or({
                if self.appearance_complete {
                    "train variant creator absent from CountryAppearance"
                } else {
                    "CountryAppearance is incomplete this hour: train variant creator is unknown"
                }
            })?;
            (
                installed_content_country_names::country_names::original_tag(creator, appearance)?,
                Some(*appearance),
            )
        } else {
            (
                installed_content_country_names::country_names::original_or_own_tag(self.frame, 0)?,
                None,
            )
        };
        if let Some(found @ (train_gfx::Level::Country, _)) =
            database.lookup(Some(tag), None, equipment, archetype)
        {
            return Ok(Some(found));
        }
        // A country-table hit returns before culture lookup. The reserved
        // singleton has no recorded runtime culture source, which cannot
        // be inferred from its tag.
        let appearance =
            appearance.ok_or("reserved creator runtime graphical culture is not recorded")?;
        let culture = self.cultures.resolve(&appearance.graphical_culture)?;
        Ok(database.lookup(Some(tag), culture, equipment, archetype))
    }
}

/// the game's car list: wagon indices, front first.
fn consist_cars(consist: &Consist, defines: &TrainDefines, random: &mut Random) -> Vec<usize> {
    let mut cars: Vec<usize> = consist.fixed().collect();
    let last = usize::from(consist.wagon_last.is_some());
    let target = defines
        .cars(random.next_word())
        .max((cars.len() + last) as i64) as usize
        + last;
    if !consist.wagons.is_empty() {
        while cars.len() < target {
            cars.push(consist.wagons[random.next_word() as usize % consist.wagons.len()]);
        }
    }
    cars.extend(consist.wagon_last);
    cars
}

#[derive(Clone, Default)]
pub struct TrainPlan {
    routes: Vec<Route>,
    /// Per eligible node, its departure weight and land-portion routes.
    weights: Vec<i64>,
    departures: Vec<Vec<usize>>,
    allocations: Vec<TrainAllocation>,
    /// Variant to the consist its creator resolves; `None` draws nothing.
    consists: BTreeMap<u64, Option<Consist>>,
    /// Variants whose consist could not be resolved, one line each.
    unresolved: Vec<String>,
    /// Variants whose equipment type token this hour cannot name, with why;
    /// reported only once a departure selects one.
    token_refusals: BTreeMap<u64, String>,
    /// The reasons of [`Self::token_refusals`] a departure selected.
    refused: BTreeSet<String>,
    fleet_cap: i32,
    branch: replay_data::BranchId,
    hour: replay_data::Hour,
    country: u32,
    services: Vec<Service>,
    next_departure: f64,
    last_time: Option<f32>,
    random: Random,
    /// [`TrainPlan::field`]'s buffers, kept so a frame allocates nothing.
    scratch: FieldScratch,
}
/// The per-frame placement buffers and the field they build.
#[derive(Clone, Default)]
struct FieldScratch {
    placed: Vec<(usize, ModelInstance)>,
    keyed: Vec<KeyedInstance>,
    field: ModelField,
}
#[derive(Clone)]
struct Service {
    route: usize,
    departed: f64,
    /// Indices into the train database's wagons, front first.
    cars: Vec<usize>,
}
#[derive(Clone)]
struct Route {
    path: Vec<u32>,
    points: Vec<[f32; 3]>,
    distance: Vec<f32>,
    seed: u32,
    /// The next province has no segment: the train dies when its head gets there.
    open_end: bool,
}
impl Route {
    /// How far the head runs before the game destroys the train.
    fn lifetime(&self, cars: &[usize], assets: &TrainAssets) -> f32 {
        let length = *self.distance.last().unwrap();
        if self.open_end {
            length
        } else {
            length
                + cars
                    .iter()
                    .map(|&car| assets.wagon(car).length)
                    .sum::<f32>()
        }
    }

    fn at(&self, distance: f32) -> ([f32; 3], [f32; 3]) {
        #[cfg(test)]
        ROUTE_SAMPLES.with(|count| count.set(count.get() + 1));
        let i = self
            .distance
            .partition_point(|d| *d < distance)
            .clamp(1, self.points.len() - 1);
        let a = self.points[i - 1];
        let b = self.points[i];
        let t = (distance - self.distance[i - 1])
            / (self.distance[i] - self.distance[i - 1]).max(0.0001);
        (
            std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t),
            std::array::from_fn(|k| b[k] - a[k]),
        )
    }
}

// `Route::at` calls on this thread, so a test can bound one frame's work
// without a wall clock.
#[cfg(test)]
thread_local! {
    static ROUTE_SAMPLES: Cell<usize> = const { Cell::new(0) };
}

impl TrainPlan {
    pub fn route_count(&self) -> usize {
        self.routes.len()
    }

    /// The railway manager owns one presentation clock and a live fleet across
    /// simulation ticks. Keep it when the same country is still viewed on the
    /// same branch during continuous playback; retain each consist only if its
    /// exact route geometry still exists. A route that changed under a train
    /// cannot be projected onto guessed new track.
    /// The random stream's position, the timetable and the live fleet, for comparing two plans
    /// stepped alike.
    #[cfg(test)]
    pub(crate) fn presentation_state(&self) -> impl PartialEq + std::fmt::Debug {
        (
            (self.random.seed, self.random.index),
            self.next_departure,
            self.last_time,
            self.services
                .iter()
                .map(|service| (service.route, service.departed, service.cars.clone()))
                .collect::<Vec<_>>(),
            self.refused.clone(),
        )
    }

    /// How many trains are running.
    #[cfg(test)]
    pub(crate) fn live_trains(&self) -> usize {
        self.services.len()
    }

    pub fn carry_presentation(&mut self, previous: &mut Self, continuity: Continuity) {
        // The frame buffers hold no state; taking them spares the next frame's allocation.
        std::mem::swap(&mut self.scratch, &mut previous.scratch);
        if self.country == 0
            || self.country != previous.country
            || !continuity.keeps((previous.branch, previous.hour), (self.branch, self.hour))
            || self.routes.is_empty()
            || previous.routes.is_empty()
        {
            return;
        }
        self.next_departure = previous.next_departure;
        self.last_time = previous.last_time;
        self.random = previous.random.clone();
        // A published refusal stays while this hour still refuses a variant
        // for the same reason; a recovered or changed variant's goes.
        previous
            .refused
            .retain(|reason| self.token_refusals.values().any(|kept| kept == reason));
        std::mem::swap(&mut self.refused, &mut previous.refused);
        let routes: BTreeMap<_, _> = self
            .routes
            .iter()
            .enumerate()
            .map(|(index, route)| (route.path.as_slice(), index))
            .collect();
        self.services = previous
            .services
            .drain(..)
            .filter_map(|mut service| {
                let old_route = &previous.routes[service.route];
                let &new_index = routes.get(old_route.path.as_slice())?;
                let new_route = &self.routes[new_index];
                if old_route.points != new_route.points || old_route.open_end != new_route.open_end
                {
                    return None;
                }
                service.route = new_index;
                Some(service)
            })
            .collect();
    }

    /// Dump diagnostics: production inputs/derived positions, never capture data.
    pub fn diagnostics(&self, assets: &TrainAssets) -> Vec<String> {
        let time = self.last_time.unwrap_or(0.0);
        let open = self.routes.iter().filter(|r| r.open_end).count();
        let open = (open != 0).then(|| format!("Supply trains: {open} routes reach a province whose railway lacks their segment; the train is destroyed there"));
        let speed = f64::from(assets.defines.speed);
        open.into_iter()
            .chain(self.unresolved.iter().map(|line| format!("Supply trains: {line}")))
            .chain(self.refused.iter().map(|line| format!("Supply trains: {line}")))
            .chain(self.services.iter().map(|service| {
                let route = &self.routes[service.route];
                let distance = ((f64::from(time) - service.departed) * speed) as f32;
                let (head, _) = route.at(distance);
                format!("Train route seed {:08x}: {} cars; head {:.3},{:.3},{:.3}; distance {:.3}/{:.3}", route.seed, service.cars.len(), head[0], head[1], head[2], distance, route.distance.last().unwrap())
            }))
            .collect()
    }

    /// Build shared route geometry once per hour, from the simulation's own edges.
    pub fn build(
        frame: &DecodedFrame,
        ours: u32,
        rails: &super::railways::RailGeometry,
        heights: &HeightField<'_>,
        assets: &TrainAssets,
        cultures: &Cultures,
    ) -> io::Result<Self> {
        Self::build_with_graph(frame, ours, rails, heights, assets, cultures, || {
            super::railways::RailNetwork::read(frame).map(Arc::new)
        })
    }

    pub fn build_with_graph(
        frame: &DecodedFrame,
        ours: u32,
        rails: &super::railways::RailGeometry,
        heights: &HeightField<'_>,
        assets: &TrainAssets,
        cultures: &Cultures,
        graph: impl FnOnce() -> io::Result<Arc<super::railways::RailNetwork>>,
    ) -> io::Result<Self> {
        let mut plan = Self {
            branch: frame.branch,
            hour: frame.hour,
            ..Self::default()
        };
        let countries = core::<tables::Countries>(frame)?;
        let Some(country) = countries.iter().find(|c| c.canonical_slot == ours) else {
            return Ok(plan);
        };
        plan.country = country.id;
        let nodes = rows::<tables::SupplyNodes>(frame)?;
        let inventory = rows::<tables::TrainInventory>(frame)?;
        let variants = rows::<tables::VariantDefinitions>(frame)?;
        let tokens = TokenNames::new(frame)?;
        let edges = rows::<tables::RailEdges>(frame)?;
        let cooldown = rows::<tables::RailCooldowns>(frame)?;
        let supply = rows::<tables::ProvinceSupply>(frame)?;
        let naval_bases = rows::<tables::NavalBases>(frame)?;
        let appearance = rows::<tables::CountryAppearance>(frame)?;
        if !nodes.is_complete()
            || !inventory.is_complete()
            || !variants.is_complete()
            || !edges.is_complete()
            || !cooldown.is_complete()
            || !supply.is_complete()
            || !naval_bases.is_complete()
        {
            return Ok(plan);
        }
        // The same graph, spurs and harbour anchors the rail pass draws.
        let network = graph()?;
        let Some(inventory) = inventory.rows().iter().find(|r| r.country == country.id) else {
            return Err(invalid("complete TrainInventory lacks viewing country"));
        };
        plan.allocations = inventory.allocations.clone();
        plan.fleet_cap = allocation_sum(&plan.allocations);
        let country_links = core::<tables::CountryLinks>(frame)?;
        let creators = CreatorLookup {
            frame,
            links: country_links
                .iter()
                .map(|row| (row.id, row.canonical_slot))
                .collect(),
            countries: countries
                .iter()
                .map(|row| (row.canonical_slot, row))
                .collect(),
            appearance: appearance
                .rows()
                .iter()
                .map(|row| (row.country, row))
                .collect(),
            appearance_complete: appearance.is_complete(),
            cultures,
        };
        let definitions: BTreeMap<_, _> = variants.rows().iter().map(|r| (r.id, r)).collect();
        for variant in plan.allocations.iter().map(|entry| entry.variant) {
            if variant == 0 || plan.consists.contains_key(&variant) {
                continue;
            }
            let Some(definition) = definitions.get(&variant) else {
                plan.consists.insert(variant, None);
                continue;
            };
            // A token this hour cannot name draws no consist, like a variant
            // without a definition row; it is reported once a departure
            // selects the variant.
            let equipment = match tokens
                .name(definition.equipment_token)
                .map_err(|reason| reason.to_string())
                .and_then(|spelling| assets.equipment_key(spelling))
            {
                Ok(equipment) => equipment,
                Err(reason) => {
                    plan.token_refusals.insert(
                        variant,
                        format!(
                            "variant {variant} equipment type: {reason}; its trains are not drawn"
                        ),
                    );
                    plan.consists.insert(variant, None);
                    continue;
                }
            };
            let archetype = match assets.archetype(equipment) {
                Ok(archetype) => archetype,
                Err(reason) => {
                    plan.unresolved.push(reason);
                    plan.consists.insert(variant, None);
                    continue;
                }
            };
            let consist = match creators.consist(&assets.database, definition, equipment, archetype)
            {
                Ok(consist) => consist,
                Err(reason) => {
                    plan.unresolved.push(reason);
                    plan.consists.insert(variant, None);
                    continue;
                }
            };
            if consist.is_none() {
                plan.unresolved.push(format!(
                    "no train database entry for {equipment} ({archetype}); its trains are not drawn"
                ));
            }
            plan.consists.insert(variant, consist);
        }
        let provinces = core::<tables::Provinces>(frame)?;
        let fallback = FallbackEndpoints::new(nodes.rows(), countries, country_links, provinces)?;
        let rows: Vec<_> = nodes
            .rows()
            .iter()
            .filter(|n| n.country == country.id)
            .collect();
        let nodes: Vec<_> = rows.iter().copied().filter(|n| !n.is_vertex()).collect();
        if nodes.len() != inventory.node_count as usize {
            return Err(invalid(
                "SupplyNodes census differs from TrainInventory.node_count",
            ));
        }
        // Duplicate provinces across kinds have the same province-keyed parent/edge.
        let mut by_province: BTreeMap<u32, &SupplyNodeRow> = BTreeMap::new();
        for &node in &rows {
            if let Some(old) = by_province.insert(node.province, node)
                && (
                    old.parent,
                    old.route_from,
                    old.route_to,
                    old.break_after,
                    &old.path,
                ) != (
                    node.parent,
                    node.route_from,
                    node.route_to,
                    node.break_after,
                    &node.path,
                )
            {
                return Err(invalid(
                    "SupplyNodes has conflicting province-keyed parent edges",
                ));
            }
        }
        let closed: BTreeSet<_> = cooldown
            .rows()
            .iter()
            .filter(|r| r.cooldown_ticks > 0)
            .map(|r| r.province)
            .collect();
        let allowed: BTreeSet<_> = provinces
            .iter()
            .filter(|p| {
                let slot = if p.controller == 0 {
                    Some(0)
                } else {
                    creators.links.get(&p.controller).copied()
                };
                !closed.contains(&p.id)
                    && slot
                        .and_then(|slot| inventory.controller_access.get(slot))
                        .is_some_and(|b| b != 0)
            })
            .map(|p| p.id)
            .collect();
        let rail: BTreeSet<_> = edges
            .rows()
            .iter()
            .flat_map(|e| [(e.from, e.to), (e.to, e.from)])
            .collect();
        let curves = assets.routes.curves(rails, heights, &network);
        let mut shared = BTreeMap::new();
        for node in nodes {
            let weight = departure_weight(node);
            if weight == 0 {
                continue;
            }
            let mut routes = Vec::new();
            for path in land_portions(node, &by_province, &fallback)? {
                // Controller access, cooldown and adjacent railway entries.
                if path.iter().any(|p| !allowed.contains(p))
                    || path.windows(2).any(|p| !rail.contains(&(p[0], p[1])))
                {
                    continue;
                }
                let index = if let Some(&index) = shared.get(&path) {
                    index
                } else {
                    let seed = path.iter().fold(SEED, |h, p| h.rotate_left(5) ^ p);
                    let kept = curves
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .get(&path)
                        .cloned();
                    let curve = match kept {
                        Some(curve) => curve,
                        None => {
                            // The train runs on the drawn railway curves.
                            let curve = rails.train_route(&path, &network, heights).map(|route| {
                                let mut distance = vec![0.0];
                                for pair in route.points.windows(2) {
                                    distance.push(
                                        distance.last().unwrap()
                                            + (pair[0][0] - pair[1][0]).hypot(pair[0][2] - pair[1][2]),
                                    );
                                }
                                Arc::new((route, distance))
                            });
                            curves
                                .lock()
                                .unwrap_or_else(|error| error.into_inner())
                                .insert(path.clone(), curve.clone());
                            curve
                        }
                    };
                    let Some(curve) = curve else {
                        continue;
                    };
                    let (route, distance) = &*curve;
                    let index = plan.routes.len();
                    plan.routes.push(Route {
                        path: path.clone(),
                        points: route.points.clone(),
                        distance: distance.clone(),
                        seed,
                        open_end: route.open_end,
                    });
                    shared.insert(path, index);
                    index
                };
                routes.push(index);
            }
            // Access is not part of the node draw: invalid routes consume a departure.
            plan.weights.push(weight);
            plan.departures.push(routes);
        }
        Ok(plan)
    }

    /// This frame's cars. Work is proportional to the live fleet; departures
    /// since the last call are replayed from the timetable. The field is the
    /// plan's own, rebuilt in place each call.
    pub fn field(&mut self, assets: &TrainAssets, time: f32) -> &mut ModelField {
        let FieldScratch {
            placed,
            keyed,
            field,
        } = &mut self.scratch;
        placed.clear();
        if self.routes.is_empty() {
            field.rebuild(&assets.library, placed, keyed, 128.0);
            return field;
        }
        let defines = &assets.defines;
        let speed = f64::from(defines.speed);
        let time = time.max(0.0);
        if self.last_time.is_none_or(|old| time < old) {
            self.services.clear();
            self.random = Random::new(self.routes.iter().fold(SEED, |s, r| s ^ r.seed));
            // Warm from an empty fleet farther back than any train's lifetime.
            // This is presentation history, not game state or an inventory estimate.
            let longest = self
                .routes
                .iter()
                .map(|r| *r.distance.last().unwrap())
                .fold(0.0, f32::max);
            let car_length = assets
                .database
                .wagons
                .iter()
                .map(|w| w.length)
                .fold(0.0, f32::max);
            let cars = defines.wagon_count[1].max(4) as f32 + 2.0;
            self.next_departure = -f64::from(longest + car_length * cars) / speed;
        }
        while self.next_departure <= f64::from(time) {
            let departed = self.next_departure;
            // The game draws the next interval before testing cap/weights.
            self.next_departure += defines.interval(self.random.next_word());
            self.services.retain(|service| {
                service.departed
                    + f64::from(self.routes[service.route].lifetime(&service.cars, assets)) / speed
                    >= departed
            });
            if self.services.len() as i64 >= i64::from(self.fleet_cap) {
                continue;
            }
            let Some(node) = weighted_node(&self.weights, self.random.next_word()) else {
                continue;
            };
            // One chosen node may emit several land portions; native checks the cap once.
            for &route in &self.departures[node] {
                let variant = selected_variant(&self.allocations, self.random.next_word());
                let Some(Some(consist)) = self.consists.get(&variant) else {
                    if let Some(reason) = self.token_refusals.get(&variant)
                        && !self.refused.contains(reason)
                    {
                        self.refused.insert(reason.clone());
                    }
                    continue;
                };
                let mut random = Random::new(self.routes[route].seed ^ self.random.next_word());
                let cars = consist_cars(consist, defines, &mut random);
                if !cars.is_empty() {
                    self.services.push(Service {
                        route,
                        departed,
                        cars,
                    });
                }
            }
        }
        self.last_time = Some(time);
        self.services.retain(|service| {
            let lifetime = self.routes[service.route].lifetime(&service.cars, assets);
            (f64::from(time) - service.departed) * speed <= f64::from(lifetime)
        });
        for service in &self.services {
            let route = &self.routes[service.route];
            let length = *route.distance.last().unwrap();
            let head = ((f64::from(time) - service.departed) * speed) as f32;
            let mut offset = 0.0;
            for (car_index, &car) in service.cars.iter().enumerate() {
                let car = assets.wagon(car);
                let front = head - offset;
                offset += car.length;
                let mut back = head - offset;
                let (front_position, _) = route.at(front);
                // Solve the link's straight chord, at most ten steps.
                for _ in 0..10 {
                    let (b, _) = route.at(back);
                    let error = ((front_position[0] - b[0]).powi(2)
                        + (front_position[1] - b[1]).powi(2)
                        + (front_position[2] - b[2]).powi(2))
                    .sqrt()
                        - car.length;
                    if error.abs() < 0.001 {
                        break;
                    }
                    back += error;
                }
                offset = head - back;
                if front < 0.0 || back > length {
                    continue;
                }
                let (a, _) = route.at(front);
                let (b, tangent) = route.at(back);
                let dx = a[0] - b[0];
                let dz = a[2] - b[2];
                let (dx, dz) = if dx.hypot(dz) < 0.0001 {
                    (tangent[0], tangent[2])
                } else {
                    (dx, dz)
                };
                if let Some(mesh) = assets.library.slot(&car.entity) {
                    placed.push((
                        mesh,
                        ModelInstance {
                            position: [
                                b[0] + car.shift[0],
                                b[1] + car.shift[1],
                                b[2] + car.shift[2],
                            ],
                            rotation: car_yaw(dx, dz, car.rotation),
                            scale: car.scale,
                            pitch: (a[1] - b[1]).atan2(dx.hypot(dz)),
                            roll: 0.0,
                            train_clip: if back < 2.5 {
                                let (p, t) = route.at(0.0);
                                let n = t[0].hypot(t[2]).max(0.0001);
                                [p[0], p[2], t[0] / n, t[2] / n]
                            } else if !route.open_end && front > length - 2.5 {
                                let (p, t) = route.at(length);
                                let n = t[0].hypot(t[2]).max(0.0001);
                                [p[0], p[2], -t[0] / n, -t[2] / n]
                            } else {
                                [0.0; 4]
                            },
                            animation: super::entity_animation::TRAIN,
                            animation_seed: route.seed
                                ^ service.departed.to_bits() as u32
                                ^ car_index as u32,
                            animation_phase: -service.departed as f32,
                            ..ModelInstance::default()
                        },
                    ));
                }
            }
        }
        field.rebuild(&assets.library, placed, keyed, 128.0);
        field
    }
}

/// Face from the back coupling toward the front, then apply the authored rotation.
/// [`super::entity_animation::world_transform`] uses that yaw directly.
fn car_yaw(dx: f32, dz: f32, rotation: f32) -> f32 {
    dz.atan2(dx) + rotation
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// The game uses signed native subtraction and rounds positive fixed5 up.
fn departure_weight(node: &SupplyNodeRow) -> i64 {
    let spare = node.available.wrapping_sub(node.consumed);
    if node.flag != 0 || node.kind & !5 == 0 || spare <= 0 {
        return 0;
    }
    (spare / 100000 + i64::from(spare % 100000 != 0)).wrapping_mul(100000)
}

/// Positive signed weights, double scaling by 2^-31.
fn weighted_node(weights: &[i64], random: u32) -> Option<usize> {
    let total = weights
        .iter()
        .filter(|w| **w > 0)
        .fold(0u64, |sum, w| sum.wrapping_add(*w as u64));
    let draw = (f64::from(random & 0x7fff_ffff) * (1.0 / 2147483648.0) * total as f64) as u64;
    let mut sum = 0u64;
    weights.iter().position(|weight| {
        if *weight <= 0 {
            return false;
        }
        sum = sum.wrapping_add(*weight as u64);
        draw < sum
    })
}

fn allocation_sum(entries: &[TrainAllocation]) -> i32 {
    entries
        .iter()
        .fold(0i32, |sum, entry| sum.wrapping_add(entry.count))
}

/// The game preserves order and signed counts; unresolved/null winner tries first entry.
fn selected_variant(entries: &[TrainAllocation], random: u32) -> u64 {
    let total = allocation_sum(entries);
    if total > 0 {
        let mut draw = (random & 0x7fff_ffff) as i32 % total;
        for entry in entries {
            draw = draw.wrapping_sub(entry.count);
            if draw < 0 {
                if entry.variant != 0 {
                    return entry.variant;
                }
                break;
            }
        }
    }
    entries.first().map_or(0, |entry| entry.variant)
}

/// The game resolves each missing-route endpoint through its own current
/// province controller, not through the train's country. Node creation
/// registers every kind-2..=5 node in that supply companion's subtype-zero
/// province map when they create it; kinds 0 and 1 never enter that map.
/// Only map membership matters for the game's synthetic naval break, so
/// the complete recorded node census and province controllers are sufficient.
#[derive(Default)]
struct FallbackEndpoints {
    controllers: BTreeMap<u32, u32>,
    links: BTreeMap<u32, u32>,
    subtype_zero: BTreeSet<(u32, u32)>,
}

impl FallbackEndpoints {
    fn new(
        nodes: &[SupplyNodeRow],
        countries: &[CountryRow],
        links: &[CountryLinkRow],
        provinces: &[ProvinceRow],
    ) -> io::Result<Self> {
        let country_slots: BTreeMap<_, _> = countries
            .iter()
            .map(|country| (country.id, country.canonical_slot))
            .collect();
        let mut subtype_zero = BTreeSet::new();
        for node in nodes.iter().filter(|node| !node.is_vertex()) {
            if node.kind > 5 {
                return Err(invalid("SupplyNodes has an unknown node kind"));
            }
            if node.kind < 2 {
                continue;
            }
            let slot = country_slots
                .get(&node.country)
                .ok_or_else(|| invalid("SupplyNodes country absent from Countries"))?;
            subtype_zero.insert((*slot, node.province));
        }
        Ok(Self {
            controllers: provinces
                .iter()
                .map(|row| (row.id, row.controller))
                .collect(),
            links: links
                .iter()
                .map(|row| (row.id, row.canonical_slot))
                .collect(),
            subtype_zero,
        })
    }

    fn contains(&self, province: u32) -> io::Result<bool> {
        let controller = self
            .controllers
            .get(&province)
            .ok_or_else(|| invalid("supply fallback endpoint absent from Provinces"))?;
        let slot = self
            .links
            .get(controller)
            .ok_or_else(|| invalid("supply fallback controller absent from CountryLinks"))?;
        Ok(self.subtype_zero.contains(&(*slot, province)))
    }

    fn emits(&self, parent: u32, province: u32) -> io::Result<bool> {
        // Native evaluates both endpoint lookups, even if the first misses.
        let source = self.contains(parent)?;
        let destination = self.contains(province)?;
        Ok(source && destination)
    }
}

/// A 100-step parent walk followed by flushing, deduplication and reversal.
/// A missing parent refuses the route, including any earlier closed portions.
fn land_portions(
    start: &SupplyNodeRow,
    nodes: &BTreeMap<u32, &SupplyNodeRow>,
    fallback: &FallbackEndpoints,
) -> io::Result<Vec<Vec<u32>>> {
    let mut out = Vec::new();
    let mut path = Vec::new();
    let flush = |path: &mut Vec<u32>, out: &mut Vec<Vec<u32>>| {
        path.dedup();
        path.reverse();
        if path.len() > 1 {
            out.push(std::mem::take(path));
        } else {
            path.clear();
        }
    };
    let mut node = start;
    for _ in 0..100 {
        if node.parent < 1 {
            break;
        }
        if node.route_from != 0 {
            path.extend_from_slice(&node.path);
            if node.break_after != 0 {
                flush(&mut path, &mut out);
            }
        } else if fallback.emits(node.parent as u32, node.province)? {
            flush(&mut path, &mut out);
        }
        let Some(parent) = nodes.get(&(node.parent as u32)) else {
            return Err(invalid(
                "SupplyNodes parent chain reaches a province without a row",
            ));
        };
        node = parent;
    }
    flush(&mut path, &mut out);
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use replay_data::{
        rows::Recorded, rows::ReservedCountryNamingRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::{frame::Metrics, frame::TableView};
    use viewer_foundation::map::presentation::Continuity;

    fn creator_frame(original_tag: Recorded<Option<Vec<u8>>>) -> DecodedFrame {
        let original_tag_reference = Recorded::Value(Some(
            if matches!(&original_tag, Recorded::Value(Some(bytes)) if bytes.is_empty()) {
                0
            } else {
                1
            },
        ));
        let mut tables = std::array::from_fn(|_| TableView::Unavailable {
            reason: "unrelated fixture table".into(),
        });
        tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 400,
                canonical_slot: 30,
                tag: b"OWN".to_vec(),
                ..Default::default()
            }]));
        tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: 400,
                canonical_slot: 30,
            }]));
        tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
                country: 400,
                original_tag,
                original_tag_reference,
                // A static country's name generator can have no original
                // operand even while original-tag selection returns a stored value.
                name_tag: Vec::new(),
                graphical_culture: b"runtime_gfx".to_vec(),
                cosmetic_tag: vec![0xff],
                leader_ideology: vec![0xff],
                overlord: 999,
                ..Default::default()
            }]));
        DecodedFrame {
            branch: 1,
            hour: 0,
            tables: tables.map(Into::into),
            metrics: Metrics::default(),
        }
    }

    fn creator_lookup<'a>(frame: &'a DecodedFrame, cultures: &'a Cultures) -> CreatorLookup<'a> {
        let appearance = rows::<tables::CountryAppearance>(frame).unwrap();
        CreatorLookup {
            frame,
            links: frame
                .complete::<tables::CountryLinks>()
                .unwrap_or_default()
                .iter()
                .map(|row| (row.id, row.canonical_slot))
                .collect(),
            countries: frame
                .complete::<tables::Countries>()
                .unwrap_or_default()
                .iter()
                .map(|row| (row.canonical_slot, row))
                .collect(),
            appearance: appearance
                .rows()
                .iter()
                .map(|row| (row.country, row))
                .collect(),
            appearance_complete: appearance.is_complete(),
            cultures,
        }
    }

    fn creator_database() -> train_gfx::Database {
        let table = |key: &str, wagon| {
            BTreeMap::from([(
                train_gfx::token_key(key),
                Consist {
                    wagon_0: Some(wagon),
                    ..Default::default()
                },
            )])
        };
        train_gfx::Database {
            // Country archetype precedes another table's equipment type.
            countries: BTreeMap::from([
                ("ORG".into(), table("archetype", 0)),
                ("OWN".into(), table("equipment", 1)),
                ("GEN".into(), table("equipment", 2)),
            ]),
            cultures: BTreeMap::from([
                ("own_gfx".into(), table("equipment", 3)),
                ("original_gfx".into(), table("equipment", 4)),
                ("runtime_gfx".into(), table("equipment", 6)),
                (train_gfx::NULL_CULTURE_NAME.into(), table("equipment", 7)),
            ]),
            default: table("equipment", 5),
            ..Default::default()
        }
    }

    #[test]
    fn train_original_tag_is_independent_of_the_name_generator_and_preserves_own_fallback() {
        let database = creator_database();
        let cultures = Cultures {
            registered: BTreeSet::from(["runtime_gfx".into()]),
        };
        for (original, reference, expected) in [
            (b"ORG".to_vec(), 1, (train_gfx::Level::Country, 0)),
            (b"OWN".to_vec(), 1, (train_gfx::Level::Country, 1)),
            (Vec::new(), 0, (train_gfx::Level::Country, 1)),
            (Vec::new(), -7, (train_gfx::Level::Country, 1)),
            // Positive-empty original bytes preserve the positive gate and
            // do not become the creator's own country-table key.
            (Vec::new(), 1, (train_gfx::Level::Culture, 6)),
        ] {
            let mut frame = creator_frame(Recorded::Value(Some(original)));
            for generator_tag in [Vec::new(), b"GEN".to_vec(), vec![0xff]] {
                if let TableView::Complete(TableRows::CountryAppearance(rows)) =
                    &mut *frame.tables[TableId::CountryAppearance.index()]
                {
                    rows[0].name_tag = generator_tag;
                    rows[0].original_tag_reference = Recorded::Value(Some(reference));
                }
                let (level, consist) = creator_lookup(&frame, &cultures)
                    .lookup(&database, 400, "equipment", "archetype")
                    .unwrap()
                    .unwrap();
                assert_eq!((level, consist.wagon_0), (expected.0, Some(expected.1)));
            }
        }
    }

    #[test]
    fn train_original_tag_creator_zero_uses_the_reserved_witness_and_signed_positive_gate() {
        let mut frame = creator_frame(Recorded::NotRecorded);
        // Raw zero never joins the positive country or appearance census.
        for id in [
            TableId::Countries,
            TableId::CountryLinks,
            TableId::CountryAppearance,
        ] {
            *frame.tables[id.index()] = TableView::Unavailable {
                reason: "unrelated positive census".into(),
            };
        }
        let mut database = creator_database();
        let cultures = Cultures::default();
        *frame.tables[TableId::ReservedCountryNaming.index()] =
            TableView::Complete(TableRows::ReservedCountryNaming(vec![
                ReservedCountryNamingRow {
                    tag: b"OWN".to_vec(),
                    original_tag: Recorded::Value(Some(b"ORG".to_vec())),
                    original_tag_reference: Recorded::Value(Some(1)),
                    name_tag: b"GEN".to_vec(),
                    cosmetic_tag: vec![0xff],
                    leader_ideology: vec![0xff],
                    overlord: 999,
                    ..Default::default()
                },
            ]));
        assert_eq!(
            creator_lookup(&frame, &cultures)
                .lookup(&database, 0, "equipment", "archetype")
                .unwrap()
                .map(|(level, consist)| (level, consist.wagon_0)),
            Some((train_gfx::Level::Country, Some(0)))
        );
        database.countries.remove("ORG");
        assert!(
            creator_lookup(&frame, &cultures)
                .lookup(&database, 0, "equipment", "archetype")
                .unwrap_err()
                .contains("reserved creator runtime graphical culture is not recorded")
        );
        if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
            &mut *frame.tables[TableId::ReservedCountryNaming.index()]
        {
            rows[0].original_tag = Recorded::Value(Some(Vec::new()));
            rows[0].original_tag_reference = Recorded::Value(Some(1));
        }
        assert!(
            creator_lookup(&frame, &cultures)
                .lookup(&database, 0, "equipment", "archetype")
                .unwrap_err()
                .contains("reserved creator runtime graphical culture is not recorded")
        );
        if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
            &mut *frame.tables[TableId::ReservedCountryNaming.index()]
        {
            rows[0].tag = vec![0xff];
            rows[0].name_tag = vec![0xff];
        }
        for reference in [0, -7] {
            if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
                &mut *frame.tables[TableId::ReservedCountryNaming.index()]
            {
                rows[0].original_tag_reference = Recorded::Value(Some(reference));
            }
            let (level, consist) = creator_lookup(&frame, &cultures)
                .lookup(&database, 0, "equipment", "archetype")
                .unwrap()
                .unwrap();
            assert_eq!(level, train_gfx::Level::Default);
            assert_eq!(consist.wagon_0, Some(5));
        }
    }

    #[test]
    fn train_original_tag_reads_captured_runtime_culture_only_after_the_country_misses() {
        let mut frame = creator_frame(Recorded::Value(Some(b"ORG".to_vec())));
        let mut database = creator_database();
        let cultures = Cultures {
            registered: BTreeSet::from([
                "own_gfx".into(),
                "original_gfx".into(),
                "runtime_gfx".into(),
                train_gfx::NULL_CULTURE_NAME.into(),
            ]),
        };
        database.countries.remove("ORG");
        let (level, consist) = creator_lookup(&frame, &cultures)
            .lookup(&database, 400, "equipment", "archetype")
            .unwrap()
            .unwrap();
        assert_eq!(level, train_gfx::Level::Culture);
        assert_eq!(consist.wagon_0, Some(6));
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].tag = vec![0xff];
        }
        // An original-country hit does not inspect the runtime source or
        // the creator's unrelated own-tag bytes.
        if let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *frame.tables[TableId::CountryAppearance.index()]
        {
            rows[0].graphical_culture = vec![0xff];
        }
        let country_database = creator_database();
        assert_eq!(
            creator_lookup(&frame, &cultures)
                .lookup(&country_database, 400, "equipment", "archetype")
                .unwrap()
                .unwrap()
                .0,
            train_gfx::Level::Country
        );
        assert!(
            creator_lookup(&frame, &cultures)
                .lookup(&database, 400, "equipment", "archetype")
                .unwrap_err()
                .contains("creator runtime graphical culture is not UTF-8")
        );
        for (source, expected) in [
            ("runtime_gfx", (train_gfx::Level::Culture, 6)),
            ("original_gfx", (train_gfx::Level::Culture, 4)),
            ("own_gfx", (train_gfx::Level::Culture, 3)),
            (train_gfx::NULL_CULTURE_NAME, (train_gfx::Level::Culture, 7)),
            ("RUNTIME_GFX", (train_gfx::Level::Default, 5)),
            ("unregistered_gfx", (train_gfx::Level::Default, 5)),
            ("", (train_gfx::Level::Default, 5)),
        ] {
            if let TableView::Complete(TableRows::CountryAppearance(rows)) =
                &mut *frame.tables[TableId::CountryAppearance.index()]
            {
                rows[0].graphical_culture = source.as_bytes().to_vec();
            }
            let (level, consist) = creator_lookup(&frame, &cultures)
                .lookup(&database, 400, "equipment", "archetype")
                .unwrap()
                .unwrap();
            assert_eq!(
                (level, consist.wagon_0),
                (expected.0, Some(expected.1)),
                "{source}"
            );
        }
    }

    #[test]
    fn train_original_tag_missing_or_faulted_captures_refuse_each_variant_by_name() {
        let database = creator_database();
        let cultures = Cultures::default();
        for (captured, expected_reason) in [
            (Recorded::NotRecorded, "not recorded"),
            (Recorded::Value(None), "capture refused"),
            (Recorded::Value(Some(vec![0xff])), "not UTF-8"),
        ] {
            let mut frame = creator_frame(captured.clone());
            *frame.tables[TableId::ReservedCountryNaming.index()] =
                TableView::Complete(TableRows::ReservedCountryNaming(vec![
                    ReservedCountryNamingRow {
                        tag: b"OWN".to_vec(),
                        original_tag: captured,
                        original_tag_reference: Recorded::Value(Some(1)),
                        name_tag: b"GEN".to_vec(),
                        ..Default::default()
                    },
                ]));
            for creator_link in [400, 0] {
                let reason = creator_lookup(&frame, &cultures)
                    .consist(
                        &database,
                        &VariantDefinitionRow {
                            id: 71,
                            creator_link,
                            ..Default::default()
                        },
                        "equipment",
                        "archetype",
                    )
                    .unwrap_err();
                assert!(reason.starts_with("variant 71 creator: "), "{reason}");
                assert!(reason.contains(expected_reason), "{reason}");
                assert!(reason.ends_with("its trains are not drawn"), "{reason}");
            }
        }
        for (missing, creator_link, expected_reason) in [
            (TableId::CountryLinks, 400, "absent from CountryLinks"),
            (TableId::Countries, 400, "absent from Countries"),
            (
                TableId::CountryAppearance,
                400,
                "CountryAppearance is incomplete",
            ),
            (
                TableId::ReservedCountryNaming,
                0,
                "ReservedCountryNaming is unavailable",
            ),
        ] {
            let mut frame = creator_frame(Recorded::Value(Some(b"ORG".to_vec())));
            *frame.tables[missing.index()] = TableView::Unavailable {
                reason: "not sampled".into(),
            };
            let reason = creator_lookup(&frame, &cultures)
                .consist(
                    &database,
                    &VariantDefinitionRow {
                        id: 72,
                        creator_link,
                        ..Default::default()
                    },
                    "equipment",
                    "archetype",
                )
                .unwrap_err();
            assert!(reason.starts_with("variant 72 creator: "), "{reason}");
            assert!(reason.contains(expected_reason), "{reason}");
        }
        for captured_reference in [Recorded::NotRecorded, Recorded::Value(None)] {
            let mut frame = creator_frame(Recorded::Value(Some(b"ORG".to_vec())));
            if let TableView::Complete(TableRows::CountryAppearance(rows)) =
                &mut *frame.tables[TableId::CountryAppearance.index()]
            {
                rows[0].original_tag_reference = captured_reference;
            }
            *frame.tables[TableId::ReservedCountryNaming.index()] =
                TableView::Complete(TableRows::ReservedCountryNaming(vec![
                    ReservedCountryNamingRow {
                        tag: b"OWN".to_vec(),
                        original_tag: Recorded::Value(Some(b"ORG".to_vec())),
                        original_tag_reference: captured_reference,
                        ..Default::default()
                    },
                ]));
            for creator_link in [400, 0] {
                let reason = creator_lookup(&frame, &cultures)
                    .consist(
                        &database,
                        &VariantDefinitionRow {
                            id: 73,
                            creator_link,
                            ..Default::default()
                        },
                        "equipment",
                        "archetype",
                    )
                    .unwrap_err();
                assert!(reason.starts_with("variant 73 creator: "), "{reason}");
                assert!(reason.contains("original_tag_reference"), "{reason}");
            }
        }
    }

    #[test]
    fn weights_are_spare_supply_ceiled_and_drawn_on_the_native_scale() {
        let mut node = SupplyNodeRow {
            kind: 2,
            available: 100001,
            ..Default::default()
        };
        assert_eq!(departure_weight(&node), 200000);
        node.consumed = 1;
        assert_eq!(departure_weight(&node), 100000);
        node.flag = 2;
        assert_eq!(departure_weight(&node), 0);
        node.flag = 0;
        node.kind = 5;
        assert_eq!(departure_weight(&node), 0);
        assert_eq!(weighted_node(&[100000, 300000], 0), Some(0));
        assert_eq!(weighted_node(&[100000, 300000], 1 << 29), Some(1));
        assert_eq!(weighted_node(&[0, -1], 42), None);
    }

    #[test]
    fn signed_ordered_allocations_and_null_fallback_are_retained() {
        let entries = vec![
            TrainAllocation {
                variant: 1,
                count: -2,
            },
            TrainAllocation {
                variant: 2,
                count: 5,
            },
            TrainAllocation {
                variant: 2,
                count: 1,
            },
        ];
        assert_eq!(allocation_sum(&entries), 4);
        assert_eq!(selected_variant(&entries, 0), 2);
        assert_eq!(
            selected_variant(
                &[
                    TrainAllocation {
                        variant: 7,
                        count: 1
                    },
                    TrainAllocation {
                        variant: 0,
                        count: 3
                    }
                ],
                2
            ),
            7
        );
        assert_eq!(
            allocation_sum(&[
                TrainAllocation {
                    variant: 1,
                    count: i32::MAX
                },
                TrainAllocation {
                    variant: 2,
                    count: 1
                }
            ]),
            i32::MIN
        );
    }

    #[test]
    fn naval_fallback_uses_each_endpoints_controller_and_subtype_zero_node_kind() {
        let countries = [
            CountryRow {
                id: 7,
                canonical_slot: 1,
                ..Default::default()
            },
            CountryRow {
                id: 9,
                canonical_slot: 2,
                ..Default::default()
            },
        ];
        let links = [
            CountryLinkRow {
                id: 100,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 101,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 200,
                canonical_slot: 2,
            },
        ];
        let provinces = [
            ProvinceRow {
                id: 3,
                controller: 101,
                ..Default::default()
            },
            ProvinceRow {
                id: 4,
                controller: 200,
                ..Default::default()
            },
        ];
        let nodes = [
            SupplyNodeRow {
                country: 7,
                province: 3,
                kind: 2,
                ..Default::default()
            },
            SupplyNodeRow {
                country: 9,
                province: 4,
                kind: 5,
                ..Default::default()
            },
            SupplyNodeRow {
                country: 7,
                province: 4,
                kind: 1,
                ..Default::default()
            },
            SupplyNodeRow {
                country: 9,
                province: 3,
                kind: 0,
                ..Default::default()
            },
            SupplyNodeRow {
                country: 7,
                index: SupplyNodeRow::VERTEX | 4,
                province: 4,
                kind: 2,
                ..Default::default()
            },
        ];
        let fallback = FallbackEndpoints::new(&nodes, &countries, &links, &provinces).unwrap();
        assert!(fallback.emits(3, 4).unwrap());
        assert!(fallback.emits(4, 3).unwrap());
        let switched = [
            provinces[0].clone(),
            ProvinceRow {
                controller: 100,
                ..provinces[1].clone()
            },
        ];
        let fallback = FallbackEndpoints::new(&nodes, &countries, &links, &switched).unwrap();
        assert!(!fallback.emits(3, 4).unwrap());
        let missing_link = [
            provinces[0].clone(),
            ProvinceRow {
                controller: 201,
                ..provinces[1].clone()
            },
        ];
        let fallback = FallbackEndpoints::new(&nodes, &countries, &links, &missing_link).unwrap();
        assert!(fallback.emits(3, 4).is_err());
    }

    #[test]
    fn parent_edges_reverse_each_land_portion_and_missing_edges_only_flush_with_both_nodes() {
        let root = SupplyNodeRow {
            province: 1,
            parent: -1,
            ..Default::default()
        };
        let bridge = SupplyNodeRow {
            province: 3,
            parent: 1,
            route_from: 1,
            path: vec![3, 2, 1],
            ..Default::default()
        };
        let sea = SupplyNodeRow {
            province: 4,
            parent: 3,
            ..Default::default()
        };
        let end = SupplyNodeRow {
            province: 6,
            parent: 4,
            route_from: 4,
            path: vec![6, 5, 5, 4],
            ..Default::default()
        };
        let nodes = BTreeMap::from([(1, &root), (3, &bridge), (4, &sea), (6, &end)]);
        let mut fallback = FallbackEndpoints {
            controllers: BTreeMap::from([(3, 10), (4, 10)]),
            links: BTreeMap::from([(10, 1)]),
            subtype_zero: BTreeSet::from([(1, 3), (1, 4)]),
        };
        assert_eq!(
            land_portions(&end, &nodes, &fallback).unwrap(),
            vec![vec![4, 5, 6], vec![1, 2, 3]]
        );
        fallback.subtype_zero.remove(&(1, 4));
        assert_eq!(
            land_portions(&end, &nodes, &fallback).unwrap(),
            vec![vec![1, 2, 3, 4, 5, 6]]
        );
    }

    #[test]
    fn missing_parent_refuses_both_closed_and_open_portions() {
        // Province 1 is an ally's hub: the viewer's census has no row for it.
        let mut bridge = SupplyNodeRow {
            province: 3,
            parent: 1,
            route_from: 1,
            path: vec![3, 2, 1],
            ..Default::default()
        };
        let sea = SupplyNodeRow {
            province: 4,
            parent: 3,
            ..Default::default()
        };
        let end = SupplyNodeRow {
            province: 6,
            parent: 4,
            route_from: 4,
            path: vec![6, 5, 4],
            ..Default::default()
        };
        let nodes = BTreeMap::from([(3, &bridge), (4, &sea), (6, &end)]);
        let fallback = FallbackEndpoints {
            controllers: BTreeMap::from([(3, 10), (4, 10)]),
            links: BTreeMap::from([(10, 1)]),
            subtype_zero: BTreeSet::from([(1, 3), (1, 4)]),
        };
        assert!(land_portions(&end, &nodes, &fallback).is_err());
        bridge.break_after = 1;
        let nodes = BTreeMap::from([(3, &bridge), (4, &sea), (6, &end)]);
        assert!(land_portions(&end, &nodes, &fallback).is_err());
    }

    #[test]
    fn vertex_rows_carry_the_walk_through_an_allys_hub_to_the_capital() {
        // Province 3 is an ally's hub, recorded as a vertex row.
        let capital = SupplyNodeRow {
            province: 1,
            parent: 0,
            ..Default::default()
        };
        let hub = SupplyNodeRow {
            index: SupplyNodeRow::VERTEX | 3,
            province: 3,
            parent: 1,
            route_from: 1,
            route_to: 3,
            path: vec![3, 2, 1],
            ..Default::default()
        };
        let end = SupplyNodeRow {
            kind: 2,
            province: 6,
            parent: 3,
            route_from: 3,
            route_to: 6,
            break_after: 1,
            path: vec![6, 5, 4, 3],
            ..Default::default()
        };
        let rows = BTreeMap::from([(1, &capital), (3, &hub), (6, &end)]);
        assert_eq!(
            land_portions(&end, &rows, &FallbackEndpoints::default()).unwrap(),
            vec![vec![3, 4, 5, 6], vec![1, 2, 3]]
        );
        assert_eq!(departure_weight(&hub), 0);
    }

    const DEFINES: TrainDefines = TrainDefines {
        speed: 3.0,
        timetable: [9200, 12000],
        wagon_count: [3, 6],
    };

    fn consist() -> Consist {
        Consist {
            wagon_0: Some(0),
            wagon_1: Some(0),
            wagon_2: None,
            wagon_last: Some(0),
            wagons: vec![0],
        }
    }

    pub(crate) fn assets() -> TrainAssets {
        TrainAssets {
            library: ModelLibrary::default(),
            cultures: Cultures::default(),
            database: train_gfx::Database {
                wagons: vec![Wagon {
                    entity: "car".into(),
                    scale: 1.0,
                    rotation: 0.0,
                    shift: [0.0; 3],
                    length: 2.0,
                }],
                ..Default::default()
            },
            equipment: installed_content_core::equipment::Database::default(),
            defines: DEFINES,
            routes: RouteCache::default(),
        }
    }

    /// The next hour of [`plan`], with fewer trains allocated and a lower fleet cap.
    pub(crate) fn next_hour_plan() -> TrainPlan {
        let mut plan = plan();
        plan.hour += 1;
        plan.allocations[0].count = 3;
        plan.fleet_cap = 3;
        plan
    }

    pub(crate) fn plan() -> TrainPlan {
        TrainPlan {
            routes: vec![Route {
                path: vec![1, 2],
                points: vec![[0.0, 10.0, 0.0], [600.0, 10.0, 0.0]],
                distance: vec![0.0, 600.0],
                seed: SEED,
                open_end: false,
            }],
            weights: vec![100000],
            departures: vec![vec![0]],
            allocations: vec![TrainAllocation {
                variant: 70,
                count: 100,
            }],
            consists: BTreeMap::from([(70, Some(consist()))]),
            fleet_cap: 100,
            branch: 1,
            hour: 10,
            country: 1,
            ..TrainPlan::default()
        }
    }

    #[test]
    fn car_yaw_matches_the_back_to_front_heading() {
        // Back (2969.10, 1564.24), the car ahead's back
        // (2967.11, 1564.95); its world matrix sends local +x to (-0.337, -0.942).
        let yaw = car_yaw(2967.11 - 2969.10, 1564.95 - 1564.24, 90f32.to_radians());
        // world_transform's yaw sends local +x to (cos, sin).
        assert!((yaw.cos() + 0.337).abs() < 0.005, "{yaw}");
        assert!((yaw.sin() + 0.942).abs() < 0.005, "{yaw}");
    }

    #[test]
    fn last_wagon_is_appended_after_random_fill() {
        let counts: BTreeSet<_> = (0..100)
            .map(|seed| consist_cars(&consist(), &DEFINES, &mut Random::new(seed)).len())
            .collect();
        assert_eq!(counts, BTreeSet::from([6, 7, 8, 9]));
        // The draw is min+1..=max+1 whatever order the defines give.
        let reversed = TrainDefines {
            wagon_count: [6, 3],
            ..DEFINES
        };
        let [lo, hi] = reversed.wagon_count;
        let ordered = TrainDefines {
            wagon_count: [lo.min(hi), lo.max(hi)],
            ..DEFINES
        };
        let draws: BTreeSet<_> = (0..200).map(|r| ordered.cars(r * 7919)).collect();
        assert_eq!(draws, BTreeSet::from([4, 5, 6, 7]));
        // Without a random list only the authored slots are drawn.
        let bare = Consist {
            wagons: Vec::new(),
            ..consist()
        };
        assert_eq!(
            consist_cars(&bare, &DEFINES, &mut Random::default()),
            [0, 0, 0]
        );
    }

    /// Each wagon count is raised to 1 before the pair is ordered, whether the getter stored 0 for
    /// a missing or non-number value or the content gave less than 1.
    #[test]
    fn wagon_counts_are_each_at_least_one_before_ordering() {
        let wagons = |counts: &str| {
            let defines = Defines::new().unwrap();
            defines
                .load(
                    "common/defines/00_graphics.lua",
                    format!(
                        "NDefines = {{ NGraphics = {{ TRAIN_MAP_SPEED = 3, \
                         TUNNELBANA_TIMETABLE = {{ 9200, 12000 }}, {counts} }} }}"
                    )
                    .as_bytes(),
                )
                .unwrap();
            TrainDefines::load(&defines).map(|read| read.wagon_count)
        };
        assert_eq!(
            wagons("MIN_TRAIN_WAGON_COUNT = 3, MAX_TRAIN_WAGON_COUNT = 6").unwrap(),
            [3, 6]
        );
        // Both missing: [1, 1], so `cars` draws 2, not the 1 of [0, 0].
        let missing = TrainDefines {
            wagon_count: wagons("").unwrap(),
            ..DEFINES
        };
        assert_eq!(missing.wagon_count, [1, 1]);
        assert_eq!(missing.cars(12345), 2);
        assert_eq!(
            wagons("MAX_TRAIN_WAGON_COUNT = 6").unwrap(),
            [1, 6],
            "a missing minimum"
        );
        assert_eq!(
            wagons(r#"MIN_TRAIN_WAGON_COUNT = "3", MAX_TRAIN_WAGON_COUNT = true"#).unwrap(),
            [1, 1],
            "non-number counts"
        );
        // A count below 1 is raised before ordering, so -5 never becomes the minimum.
        assert_eq!(
            wagons("MIN_TRAIN_WAGON_COUNT = 4, MAX_TRAIN_WAGON_COUNT = -5").unwrap(),
            [1, 4]
        );
        assert_eq!(
            wagons("MIN_TRAIN_WAGON_COUNT = 0.5, MAX_TRAIN_WAGON_COUNT = 0").unwrap(),
            [1, 1]
        );
    }

    #[test]
    fn timetable_intervals_span_the_ordered_define_range() {
        let intervals: Vec<_> = (0..500).map(|r| DEFINES.interval(r * 104729)).collect();
        assert!(intervals.iter().all(|i| (9.2..=12.0).contains(i)));
        assert_eq!(DEFINES.interval(0), 9.2);
        assert_eq!(DEFINES.interval(2800), 12.0);
    }

    #[test]
    fn frame_partition_and_backward_seek_do_not_change_departures() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let assets = assets();
        let mut direct = plan();
        direct.field(&assets, 45.0);
        let departures = |p: &TrainPlan| {
            p.services
                .iter()
                .map(|s| (s.route, s.departed, s.cars.len()))
                .collect::<Vec<_>>()
        };
        let expected = departures(&direct);
        assert!(!expected.is_empty());
        let mut stepped = plan();
        for time in 0..=450 {
            stepped.field(&assets, time as f32 / 10.0);
        }
        assert_eq!(departures(&stepped), expected);
        stepped.field(&assets, 0.0);
        stepped.field(&assets, 45.0);
        assert_eq!(departures(&stepped), expected);
        for pair in expected.windows(2) {
            assert!((9.2 - 1e-8..=12.0 + 1e-8).contains(&(pair[1].1 - pair[0].1)));
        }
    }

    #[test]
    fn a_field_rebuilt_in_place_equals_a_fresh_one() {
        let mut assets = assets();
        // Slot 0 is enough to place a car; the field's bounds read no mesh radius.
        assets.library.index.insert("car".into(), 0);
        let mut fresh = plan();
        let expected = fresh.field(&assets, 45.0).clone();
        assert!(!expected.instances.is_empty());
        let mut reused = plan();
        for time in 0..=450 {
            reused.field(&assets, time as f32 / 10.0);
        }
        assert_eq!(*reused.field(&assets, 45.0), expected);
    }

    #[test]
    fn an_empty_route_field_drops_its_previous_source_spine() {
        let assets = assets();
        let mut plan = plan();
        plan.scratch.field = ModelField::build(
            &assets.library,
            vec![(
                0,
                ModelInstance {
                    scale: 1.,
                    ..Default::default()
                },
            )],
            128.,
        );
        assert_eq!(plan.scratch.field.native_order.len(), 1);
        plan.routes.clear();
        let field = plan.field(&assets, 1.);
        assert!(
            field.instances.is_empty() && field.groups.is_empty() && field.native_order.is_empty()
        );
        let mut pass = super::super::models::ModelPass::new(&assets.library, "trains");
        pass.replace_field(&assets.library, field);
        pass.animate(1., None, &[0.]);
        assert!(pass.events.is_empty());
    }

    #[test]
    fn token_spellings_name_declared_equipment_under_the_lexers_fold() {
        let (equipment, _) = installed_content_core::equipment::fixture::load(&[(
            "common/units/equipment/trains.txt",
            "equipments = { train_equipment = { is_archetype = yes }
                train_equipment_1 = { archetype = TRAIN_EQUIPMENT } orphan = { } }",
        )]);
        let assets = TrainAssets {
            equipment,
            ..assets()
        };
        // The recorded spelling may differ in ASCII case from the declared key.
        let key = assets.equipment_key("Train_Equipment_1").unwrap();
        assert_eq!(key, "train_equipment_1");
        assert_eq!(
            assets.equipment_key("armored_train"),
            Err(format!(
                "equipment armored_train is not declared in {}",
                installed_content_core::equipment::DIRECTORY
            ))
        );
        // The null object's token, which the lookup would try, is unknown.
        assert!(
            assets
                .archetype("orphan")
                .unwrap_err()
                .contains("null-object fallback token is unavailable")
        );
        // The declared key finds a database entry written in another case.
        let mut database = train_gfx::Database::default();
        let (is_tag, has_entity) = (|_: &str| false, |_: &str| true);
        database
            .add(
                b"wagon_data = { entity = car } default = { TRAIN_EQUIPMENT_1 = { wagon_0 = car } }",
                "t.txt",
                &train_gfx::Registries {
                    is_tag: &is_tag,
                    cultures: &BTreeSet::new(),
                    has_entity: &has_entity,
                },
                &mut BTreeMap::new(),
            )
            .unwrap();
        let archetype = assets.archetype(key).unwrap();
        assert_eq!(archetype, "train_equipment");
        assert!(matches!(
            database.lookup(None, None, key, archetype),
            Some((train_gfx::Level::Default, found)) if found.wagon_0 == Some(0)
        ));
    }

    #[test]
    fn a_token_refusal_is_reported_only_when_a_departure_selects_it() {
        let assets = assets();
        let reason = "variant 71 equipment type: token 9 is among LexicalTokens' missing rows; its trains are not drawn";
        // Variant 71 has no allocation, so no departure selects it.
        let mut unselected = plan();
        unselected.consists.insert(71, None);
        unselected.token_refusals.insert(71, reason.to_owned());
        unselected.field(&assets, 45.0);
        assert!(!unselected.services.is_empty());
        assert!(unselected.refused.is_empty());
        let mut selected = plan();
        selected.consists.insert(70, None);
        selected.token_refusals.insert(70, reason.to_owned());
        selected.field(&assets, 45.0);
        assert!(selected.services.is_empty());
        assert_eq!(selected.refused, BTreeSet::from([reason.to_owned()]));
        assert!(
            selected
                .diagnostics(&assets)
                .contains(&format!("Supply trains: {reason}"))
        );
        // The next hour keeps the published reason before any departure,
        // and drops it once the variant is named again.
        let refused_next = || {
            let mut next = plan();
            next.hour += 1;
            next.consists.insert(70, None);
            next.token_refusals.insert(70, reason.to_owned());
            next
        };
        let mut next = refused_next();
        next.carry_presentation(&mut selected, Continuity::Continue);
        assert_eq!(next.refused, BTreeSet::from([reason.to_owned()]));
        let mut recovered = plan();
        recovered.hour += 2;
        recovered.carry_presentation(&mut next, Continuity::Continue);
        assert!(recovered.refused.is_empty());
    }

    #[test]
    fn an_unresolved_variant_draws_no_train() {
        let assets = assets();
        let mut plan = plan();
        plan.consists.insert(70, None);
        plan.field(&assets, 45.0);
        assert!(plan.services.is_empty());
    }

    #[test]
    fn playback_skips_keep_the_same_timetable_consists_and_positions() {
        let assets = assets();
        for skip in [0, 1, 20, 1000] {
            let mut previous = plan();
            previous.field(&assets, 45.0);
            assert!(!previous.services.is_empty());
            let mut next = plan();
            next.hour += skip;
            next.carry_presentation(&mut previous, Continuity::Continue);
            assert!(previous.services.is_empty());
            assert_eq!(next.last_time, Some(45.0));
            next.field(&assets, 46.0);
            let mut uninterrupted = plan();
            uninterrupted.field(&assets, 46.0);
            let services = |plan: &TrainPlan| {
                plan.services
                    .iter()
                    .map(|service| (service.route, service.departed, service.cars.clone()))
                    .collect::<Vec<_>>()
            };
            assert_eq!(services(&next), services(&uninterrupted));
            assert_eq!(
                next.diagnostics(&assets),
                uninterrupted.diagnostics(&assets)
            );
            assert_eq!(next.next_departure, uninterrupted.next_departure);
            assert_eq!(next.random.index, uninterrupted.random.index);
            assert_eq!(next.random.seed, uninterrupted.random.seed);
        }
    }

    #[test]
    fn a_seek_resets_even_the_same_or_next_hour_and_a_step_back_never_carries() {
        let assets = assets();
        for skip in [0, 1, 20] {
            let mut previous = plan();
            previous.field(&assets, 45.0);
            let mut next = plan();
            next.hour += skip;
            next.carry_presentation(&mut previous, Continuity::Reset);
            assert_eq!(next.last_time, None);
            assert!(next.services.is_empty());
            assert!(!previous.services.is_empty());
        }
        let mut previous = plan();
        previous.field(&assets, 45.0);
        let mut next = plan();
        next.hour -= 1;
        next.carry_presentation(&mut previous, Continuity::Continue);
        assert_eq!(next.last_time, None);
        assert!(next.services.is_empty());
    }

    #[test]
    fn hour_change_remaps_only_unchanged_routes_and_country() {
        let mut previous = plan();
        previous.routes.push(Route {
            path: vec![3, 4],
            points: vec![[0.0, 10.0, 1.0], [600.0, 10.0, 1.0]],
            distance: vec![0.0, 600.0],
            seed: SEED ^ 3 ^ 4,
            open_end: false,
        });
        previous.services = vec![
            Service {
                route: 0,
                departed: 10.0,
                cars: Vec::new(),
            },
            Service {
                route: 1,
                departed: 20.0,
                cars: Vec::new(),
            },
        ];
        previous.last_time = Some(30.0);
        previous.next_departure = 35.0;
        previous.random.index = 7;
        let mut next = plan();
        next.hour += 1;
        next.routes.push(previous.routes[1].clone());
        next.routes.swap(0, 1);
        next.carry_presentation(&mut previous, Continuity::Continue);
        assert_eq!(
            next.services.iter().map(|s| s.route).collect::<Vec<_>>(),
            [1, 0]
        );
        assert_eq!(next.last_time, Some(30.0));
        assert_eq!(next.next_departure, 35.0);
        assert_eq!(next.random.index, 7);

        let mut changed = plan();
        changed.hour += 2;
        changed.routes[0].points[0][0] = 1.0;
        changed.routes.push(next.routes[0].clone());
        changed.carry_presentation(&mut next, Continuity::Continue);
        assert_eq!(changed.services.len(), 1);
        assert_eq!(changed.services[0].departed, 20.0);
        let mut other_country = plan();
        other_country.country = 2;
        other_country.hour = changed.hour;
        other_country.carry_presentation(&mut changed, Continuity::Continue);
        assert!(other_country.services.is_empty());
        assert_eq!(other_country.last_time, None);

        let mut seek = plan();
        seek.hour += 20;
        seek.carry_presentation(&mut changed, Continuity::Reset);
        assert_eq!(seek.last_time, None);
        let mut new_branch = plan();
        new_branch.branch += 1;
        new_branch.hour = changed.hour;
        new_branch.carry_presentation(&mut changed, Continuity::Continue);
        assert_eq!(new_branch.last_time, None);
    }

    #[test]
    fn endpoint_extrapolation_keeps_cars_from_piling_up() {
        let plan = plan();
        let route = &plan.routes[0];
        assert_eq!(route.at(-2.0).0, [-2.0, 10.0, 0.0]);
        assert!((route.at(603.0).0[0] - 603.0).abs() < 0.001);
        assert_eq!(route.at(9.0).0, [9.0, 10.0, 0.0]);
    }

    #[test]
    fn a_frame_samples_the_route_a_bounded_number_of_times_per_live_car() {
        // Deterministic: one frame's route samples are bounded by its live
        // cars (front, the chord solve's at most ten steps, the two ends and a
        // clip), whatever the route's node count.
        let assets = assets();
        for nodes in [40usize, 4000] {
            let mut plan = plan();
            plan.routes[0].points = (0..=nodes).map(|i| [i as f32, 10.0, 0.0]).collect();
            plan.routes[0].distance = (0..=nodes).map(|i| i as f32).collect();
            plan.weights = vec![100000; 2000];
            plan.departures = vec![vec![0]; 2000];
            plan.field(&assets, 3600.0);
            for frame in 1..=10 {
                // The frame's retain and lifetime tests do not sample the route.
                let cars: usize = plan.services.iter().map(|s| s.cars.len()).sum();
                ROUTE_SAMPLES.with(|count| count.set(0));
                plan.field(&assets, 3600.0 + frame as f32 / 60.0);
                let samples = ROUTE_SAMPLES.with(Cell::get);
                let live: usize = plan.services.iter().map(|s| s.cars.len()).sum();
                // Every car sampled this frame was live before it or after it.
                let bound = 14 * (cars + live);
                assert!(
                    samples <= bound,
                    "{samples} samples against a bound of {bound} on {nodes} nodes"
                );
            }
            assert!(!plan.services.is_empty());
        }
    }
}
