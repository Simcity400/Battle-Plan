use content_reader_mesh::mesh::MeshProducts as _;

use content_reader_layers::ResolvedLayers;
use content_reader_defines::defines::Defines;
use content_reader_gui::train_gfx::{self, Consist, Wagon};
use content_reader_image::native::Decode;
use replay_data::{
    country_flags::original_tag_of, rows::CountryAppearanceRow, rows::CountryLinkRow,
    rows::CountryRow, rows::ProvinceRow, rows::Recorded, rows::SparseBytes, rows::SupplyNodeRow,
    rows::TrainAllocation, rows::tables,
};
#[cfg(test)]
use replay_data::rows::VariantDefinitionRow;
use replay_frame::frame::DecodedFrame;
#[cfg(test)]
use std::cell::Cell;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::Arc,
};
use viewer_runtime::presentation::Continuity;
use dashboard_text::table_access::TokenNames;
use viewer_models::{entity_animation::Random, KeyedInstance, ModelField, ModelInstance, ModelLibrary};
use viewer_gpu::province_map::HeightField;
use viewer_foundation::tables::{core, rows};

pub const SEED: u32 = 0x5452_4149;

const CAR_LIMIT: i32 = u16::MAX as i32;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TrainDefines {
    pub speed: f32,
    pub timetable: [i32; 2],
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
        let table = defines.numbers("NGraphics", "TUNNELBANA_TIMETABLE")?;
        if table.is_empty() {
            return Err(invalid("NDefines.NGraphics.TUNNELBANA_TIMETABLE is empty"));
        }
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
                "NDefines.NGraphics.TUNNELBANA_TIMETABLE first and last entries must be finite \
                 numbers within 32-bit signed integer range",
            ));
        };
        let timetable = [first.min(last), first.max(last)];
        if timetable[0] < 1 {
            return Err(invalid(format!(
                "NDefines.NGraphics.TUNNELBANA_TIMETABLE {timetable:?} allows a non-positive \
                 interval, which is not supported"
            )));
        }
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

    fn interval(&self, random: u32) -> f64 {
        let [lo, hi] = self.timetable.map(i64::from);
        (lo + i64::from(random & 0x7fff_ffff) % (hi - lo + 1)) as f64 / 1000.0
    }

    fn cars(&self, random: u32) -> i64 {
        let [lo, hi] = self.wagon_count.map(i64::from);
        lo + 1 + i64::from(random & 0x7fff_ffff) % (hi - lo + 1)
    }
}

#[derive(Debug, Default)]
pub struct TrainAssets {
    pub library: ModelLibrary,
    pub cultures: Cultures,
    database: train_gfx::Database,
    equipment: content_reader_key_names::equipment::Database,
    pub defines: TrainDefines,
    routes: RouteCache,
}

pub type Curve = Option<Arc<(super::railways::TrainRoute, Vec<f32>)>>;

#[derive(Default)]
struct RouteCache(std::sync::Mutex<Vec<(CurveKey, Arc<std::sync::Mutex<std::collections::HashMap<Vec<u32>, Curve>>>)>>);

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
    pub fn load(
        layers: &ResolvedLayers,
        decode: Decode,
        defines: &Defines,
        warnings: &mut Vec<content_reader_parser::Diagnostic>,
    ) -> io::Result<Self> {
        let defines = TrainDefines::load(defines)?;
        let roster = layers.descriptor_roster()?;
        let index = country_tag_index(layers)?;
        warnings.extend(
            index
                .warning()
                .map(|message| (content_reader_parser::DiagnosticKind::Warning, message)),
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
                content_reader_parser::DiagnosticKind::Refused,
                format!(
                    "Train database entries the engine also rejects: {}.",
                    database.diagnostics.join("; ")
                ),
            ));
        }
        let equipment = content_reader_key_names::equipment::Database::load_complete(layers)?;
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

    fn equipment_key(&self, spelling: &str) -> Result<&str, String> {
        let at = self.equipment.get(spelling).ok_or_else(|| {
            format!(
                "equipment {spelling} is not declared in {}",
                content_reader_key_names::equipment::DIRECTORY
            )
        })?;
        self.name(at)
    }

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

use content_reader_key_names::country_tags::country_tag_index;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cultures {
    registered: BTreeSet<String>,
}

impl Cultures {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let registered = train_gfx::cultures(layers)?;
        Ok(Self { registered })
    }

    fn resolve(&self, bytes: &[u8]) -> Result<Option<&str>, String> {
        let name = std::str::from_utf8(bytes)
            .map_err(|_| "train variant creator runtime graphical culture is not UTF-8")?;
        Ok(self.registered.get(name).map(String::as_str))
    }
}

struct CreatorLookup<'a> {
    frame: &'a DecodedFrame,
    links: BTreeMap<u32, u32>,
    countries: BTreeMap<u32, &'a CountryRow>,
    appearance: BTreeMap<u32, &'a CountryAppearanceRow>,
    appearance_complete: bool,
    cultures: &'a Cultures,
}

impl CreatorLookup<'_> {
    #[cfg(test)]
    fn consist(
        &self,
        database: &train_gfx::Database,
        variant: &VariantDefinitionRow,
        equipment: &str,
        archetype: &str,
    ) -> Result<Option<Consist>, String> {
        creator_consist(self, self.cultures, database, variant.id, variant.creator_link, equipment, archetype)
    }

    #[cfg(test)]
    fn lookup<'d>(
        &self,
        database: &'d train_gfx::Database,
        creator_link: u32,
        equipment: &str,
        archetype: &str,
    ) -> Result<Option<(train_gfx::Level, &'d Consist)>, String> {
        creator_lookup(self, self.cultures, database, creator_link, equipment, archetype)
    }
}

impl CreatorRows for CreatorLookup<'_> {
    fn link(&self, raw: u32) -> Option<u32> {
        self.links.get(&raw).copied()
    }

    fn country(&self, slot: u32) -> Option<(u32, Vec<u8>)> {
        self.countries.get(&slot).map(|row| (row.id, row.tag.clone()))
    }

    fn appearance(&self, country: u32) -> Option<CreatorAppearance> {
        self.appearance.get(&country).map(|row| CreatorAppearance {
            original_tag_reference: row.original_tag_reference,
            original_tag: row.original_tag.clone(),
            graphical_culture: row.graphical_culture.clone(),
        })
    }

    fn appearance_complete(&self) -> bool {
        self.appearance_complete
    }

    fn reference(&self, slot: u32) -> Result<i32, String> {
        content_reader_country_names::country_names::original_or_own_reference(self.frame, slot)
    }

    fn reserved_tag(&self) -> Result<String, String> {
        content_reader_country_names::country_names::original_or_own_tag(self.frame, 0).map(str::to_owned)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CreatorAppearance {
    pub original_tag_reference: Recorded<Option<i32>>,
    pub original_tag: Recorded<Option<Vec<u8>>>,
    pub graphical_culture: Vec<u8>,
}

pub trait CreatorRows {
    fn link(&self, raw: u32) -> Option<u32>;
    fn country(&self, slot: u32) -> Option<(u32, Vec<u8>)>;
    fn appearance(&self, country: u32) -> Option<CreatorAppearance>;
    fn appearance_complete(&self) -> bool;
    fn reference(&self, slot: u32) -> Result<i32, String>;
    fn reserved_tag(&self) -> Result<String, String>;
}

fn creator_consist(
    rows: &impl CreatorRows,
    cultures: &Cultures,
    database: &train_gfx::Database,
    variant: u64,
    creator_link: u32,
    equipment: &str,
    archetype: &str,
) -> Result<Option<Consist>, String> {
    creator_lookup(rows, cultures, database, creator_link, equipment, archetype)
        .map(|found| found.map(|(_, consist)| consist.clone()))
        .map_err(|reason| format!("variant {variant} creator: {reason}; its trains are not drawn"))
}

fn creator_lookup<'d>(
    rows: &impl CreatorRows,
    cultures: &Cultures,
    database: &'d train_gfx::Database,
    creator_link: u32,
    equipment: &str,
    archetype: &str,
) -> Result<Option<(train_gfx::Level, &'d Consist)>, String> {
    let default = || database.lookup(None, None, equipment, archetype);
    let (slot, creator) = if creator_link == 0 {
        (0, None)
    } else {
        let slot = rows
            .link(creator_link)
            .ok_or("train variant creator absent from CountryLinks")?;
        (
            slot,
            Some(
                rows.country(slot)
                    .ok_or("train variant creator absent from Countries")?,
            ),
        )
    };
    if rows.reference(slot)? <= 0 {
        return Ok(default());
    }
    let (tag, appearance) = if let Some((id, own)) = creator {
        let appearance = rows.appearance(id).ok_or({
            if rows.appearance_complete() {
                "train variant creator absent from CountryAppearance"
            } else {
                "CountryAppearance is incomplete this hour: train variant creator is unknown"
            }
        })?;
        (
            original_tag_of(
                &appearance.original_tag_reference,
                &appearance.original_tag,
                &own,
            )?
            .to_owned(),
            Some(appearance),
        )
    } else {
        (rows.reserved_tag()?, None)
    };
    if let Some(found @ (train_gfx::Level::Country, _)) =
        database.lookup(Some(&tag), None, equipment, archetype)
    {
        return Ok(Some(found));
    }
    let appearance =
        appearance.ok_or("reserved creator runtime graphical culture is not recorded")?;
    let culture = cultures.resolve(&appearance.graphical_culture)?;
    Ok(database.lookup(Some(&tag), culture, equipment, archetype))
}

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
    weights: Vec<i64>,
    departures: Vec<Vec<usize>>,
    allocations: Vec<TrainAllocation>,
    consists: BTreeMap<u64, Option<Consist>>,
    unresolved: Vec<String>,
    token_refusals: BTreeMap<u64, String>,
    refused: BTreeSet<String>,
    fleet_cap: i32,
    branch: replay_data::BranchId,
    hour: replay_data::Hour,
    country: u32,
    services: Vec<Service>,
    next_departure: f64,
    last_time: Option<f32>,
    random: Random,
    scratch: FieldScratch,
    drawn: Vec<(Vec<u32>, u64, [f32; 3])>,
    drawn_time: Option<f32>,
}
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
    cars: Vec<usize>,
}
pub type RouteGeometry = Arc<(super::railways::TrainRoute, Vec<f32>)>;

#[derive(Clone)]
struct Route {
    path: Vec<u32>,
    curve: RouteGeometry,
    seed: u32,
}
impl Route {
    fn points(&self) -> &[[f32; 3]] {
        &self.curve.0.points
    }

    fn distance(&self) -> &[f32] {
        &self.curve.1
    }

    fn open_end(&self) -> bool {
        self.curve.0.open_end
    }

    fn same_track(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.curve, &other.curve) || (self.points() == other.points() && self.open_end() == other.open_end())
    }

    fn lifetime(&self, cars: &[usize], assets: &TrainAssets) -> f32 {
        let length = *self.distance().last().unwrap();
        if self.open_end() {
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
        let (points, distances) = (self.points(), self.distance());
        let i = distances
            .partition_point(|d| *d < distance)
            .clamp(1, points.len() - 1);
        let a = points[i - 1];
        let b = points[i];
        let t = (distance - distances[i - 1])
            / (distances[i] - distances[i - 1]).max(0.0001);
        (
            std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t),
            std::array::from_fn(|k| b[k] - a[k]),
        )
    }
}

#[cfg(test)]
thread_local! {
    static ROUTE_SAMPLES: Cell<usize> = const { Cell::new(0) };
}

fn train_arrival(now: f32, enter: f32, leave: f32, tail: f32) -> Option<f32> {
    (now <= leave + tail).then(|| (enter - now).max(0.))
}

impl TrainPlan {
    pub fn route_count(&self) -> usize {
        self.routes.len()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn presentation_state(&self) -> impl PartialEq + std::fmt::Debug {
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn live_trains(&self) -> usize {
        self.services.len()
    }

    pub fn carry_presentation(&mut self, previous: &mut Self, continuity: Continuity) {
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
        let running = previous.services.len();
        self.drawn = std::mem::take(&mut previous.drawn);
        self.drawn_time = previous.drawn_time;
        self.services = previous
            .services
            .drain(..)
            .filter_map(|mut service| {
                let old_route = &previous.routes[service.route];
                let &new_index = routes.get(old_route.path.as_slice())?;
                if !old_route.same_track(&self.routes[new_index]) {
                    return None;
                }
                service.route = new_index;
                Some(service)
            })
            .collect();
        viewer_runtime::view_record::add("trains", "installs", 1);
        viewer_runtime::view_record::add("trains", "dropped_at_install", (running - self.services.len()) as u64);
    }

    pub fn diagnostics(&self, assets: &TrainAssets) -> Vec<String> {
        let time = self.last_time.unwrap_or(0.0);
        let open = self.routes.iter().filter(|r| r.open_end()).count();
        let open = (open != 0).then(|| format!("Supply trains: {open} routes reach a province whose railway lacks their segment; the train is destroyed there"));
        let speed = f64::from(assets.defines.speed);
        open.into_iter()
            .chain(self.unresolved.iter().map(|line| format!("Supply trains: {line}")))
            .chain(self.refused.iter().map(|line| format!("Supply trains: {line}")))
            .chain(self.services.iter().map(|service| {
                let route = &self.routes[service.route];
                let distance = ((f64::from(time) - service.departed) * speed) as f32;
                let (head, _) = route.at(distance);
                format!("Train route seed {:08x}: {} cars; head {:.3},{:.3},{:.3}; distance {:.3}/{:.3}", route.seed, service.cars.len(), head[0], head[1], head[2], distance, route.distance().last().unwrap())
            }))
            .collect()
    }

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
        let rows = FrameRows::new(frame, cultures)?;
        let mut graph = Some(graph);
        let mut network: Option<Arc<super::railways::RailNetwork>> = None;
        let mut curves = None;
        let mut curve = |path: &[u32]| -> io::Result<Curve> {
            if network.is_none() {
                network = Some((graph.take().expect("the graph is read once"))()?);
            }
            let network = network.as_ref().expect("read above");
            let curves = curves.get_or_insert_with(|| assets.routes.curves(rails, heights, network));
            if let Some(curve) = curves.lock().unwrap_or_else(|error| error.into_inner()).get(path).cloned() {
                return Ok(curve);
            }
            let curve = route_curve(rails, path, network, heights);
            curves
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .insert(path.to_vec(), curve.clone());
            Ok(curve)
        };
        Self::build_from(&rows, frame.branch, frame.hour, ours, assets, rows.creators.cultures, &mut curve, None)
    }

    pub fn build_from(
        rows: &impl TrainRows,
        branch: replay_data::BranchId,
        hour: replay_data::Hour,
        ours: u32,
        assets: &TrainAssets,
        cultures: &Cultures,
        curve: &mut dyn FnMut(&[u32]) -> io::Result<Curve>,
        mut cache: Option<&mut PlanCache>,
    ) -> io::Result<Self> {
        let mut plan = Self {
            branch,
            hour,
            ..Self::default()
        };
        let Some(country) = rows.first_country(ours)? else {
            return Ok(plan);
        };
        plan.country = country;
        if !rows.complete()? {
            return Ok(plan);
        }
        let Some((node_count, allocations, access)) = rows.inventory(country)? else {
            return Err(invalid("complete TrainInventory lacks viewing country"));
        };
        plan.allocations = allocations;
        plan.fleet_cap = allocation_sum(&plan.allocations);
        rows.links_ready()?;
        let variants: Vec<u64> = plan.allocations.iter().map(|entry| entry.variant).collect();
        for variant in variants {
            if variant == 0 || plan.consists.contains_key(&variant) {
                continue;
            }
            let Some((equipment_token, creator_link)) = rows.variant(variant) else {
                plan.consists.insert(variant, None);
                continue;
            };
            let equipment = match rows
                .token(equipment_token)
                .and_then(|spelling| assets.equipment_key(&spelling))
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
            let consist = match creator_consist(rows, cultures, &assets.database, variant, creator_link, equipment, archetype) {
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
        rows.provinces_ready()?;
        let fallback = rows.fallback()?;
        let rows_of_country = rows.nodes(country)?;
        let nodes: Vec<_> = rows_of_country.iter().filter(|n| !n.is_vertex()).collect();
        if nodes.len() != node_count as usize {
            return Err(invalid(
                "SupplyNodes census differs from TrainInventory.node_count",
            ));
        }
        let mut by_province: BTreeMap<u32, &NodeValues> = BTreeMap::new();
        for node in &rows_of_country {
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
        let mut shared = BTreeMap::new();
        for node in nodes {
            let weight = departure_weight(node);
            if weight == 0 {
                continue;
            }
            let mut routes = Vec::new();
            let cached = cache.as_deref().and_then(|cache| cache.nodes.get(&node.index)).cloned();
            let paths = match cached {
                Some(paths) => paths,
                None => {
                    let mut read = Vec::new();
                    let mut paths = land_portions_reading(node, &by_province, &*fallback, &mut read)?;
                    paths.retain(|path| path.iter().all(|&p| rows.allowed(p, &access)) && path.windows(2).all(|p| rows.rail(p[0], p[1])));
                    let paths = Arc::new(paths);
                    if let Some(cache) = cache.as_deref_mut() {
                        cache.keep(node.index, Arc::clone(&paths), read);
                    }
                    paths
                }
            };
            for path in paths.iter() {
                let index = if let Some(&index) = shared.get(path) {
                    index
                } else {
                    let seed = path.iter().fold(SEED, |h, p| h.rotate_left(5) ^ p);
                    let Some(curve) = curve(path)? else {
                        continue;
                    };
                    let index = plan.routes.len();
                    plan.routes.push(Route {
                        path: path.clone(),
                        curve,
                        seed,
                    });
                    shared.insert(path.clone(), index);
                    index
                };
                routes.push(index);
            }
            plan.weights.push(weight);
            plan.departures.push(routes);
        }
        Ok(plan)
    }

    pub fn differ(&self, other: &Self) -> Option<String> {
        let bits = |values: &[f32]| values.iter().map(|value| value.to_bits()).collect::<Vec<_>>();
        if (self.country, self.fleet_cap, &self.weights, &self.departures, &self.allocations) != (other.country, other.fleet_cap, &other.weights, &other.departures, &other.allocations) {
            return Some(format!("country, fleet cap, weights, departures or allocations: {} routes and {} weights against {} and {}", self.routes.len(), self.weights.len(), other.routes.len(), other.weights.len()));
        }
        if self.routes.len() != other.routes.len() {
            return Some(format!("routes: {} against {}", self.routes.len(), other.routes.len()));
        }
        for (at, (a, b)) in self.routes.iter().zip(&other.routes).enumerate() {
            if (&a.path, a.seed, a.open_end(), bits(a.points().as_flattened()), bits(a.distance())) != (&b.path, b.seed, b.open_end(), bits(b.points().as_flattened()), bits(b.distance())) {
                return Some(format!("route {at} over {} provinces", a.path.len()));
            }
        }
        if (&self.consists, &self.unresolved, &self.token_refusals) != (&other.consists, &other.unresolved, &other.token_refusals) {
            return Some(format!("consists or refusals: {:?} against {:?}", self.unresolved, other.unresolved));
        }
        let services = |plan: &Self| plan.services.iter().map(|service| (service.route, service.departed.to_bits(), service.cars.clone())).collect::<Vec<_>>();
        if (services(self), self.next_departure.to_bits(), self.last_time.map(f32::to_bits), (self.random.seed, self.random.index), &self.refused)
            != (services(other), other.next_departure.to_bits(), other.last_time.map(f32::to_bits), (other.random.seed, other.random.index), &other.refused)
        {
            return Some("the presentation (fleet, timetable or random stream)".into());
        }
        None
    }

    pub fn set_weights(&mut self, weights: Vec<i64>) -> bool {
        if weights.len() != self.weights.len() {
            return false;
        }
        self.weights = weights;
        true
    }

    pub fn counts(&self) -> [usize; 3] {
        [self.routes.len(), self.weights.len(), self.consists.len()]
    }

    pub fn route_paths(&self) -> impl Iterator<Item = &[u32]> + '_ {
        self.routes.iter().map(|route| route.path.as_slice())
    }

    pub fn route_geometry(&self, path: &[u32]) -> Option<&RouteGeometry> {
        self.routes.iter().find(|route| route.path == path).map(|route| &route.curve)
    }

    pub fn running(&self) -> Vec<(&[u32], f64)> {
        self.services.iter().map(|service| (self.routes[service.route].path.as_slice(), service.departed)).collect()
    }

    pub fn next_arrival(
        &self,
        assets: &TrainAssets,
        view: &viewer_models::motion_view::MotionView,
        seconds: f32,
    ) -> f32 {
        let speed = assets.defines.speed;
        if self.fleet_cap <= 0 || speed <= 0.0 {
            return f32::INFINITY;
        }
        use viewer_models::motion_view::Reach;
        let reach = |car: &Wagon| {
            let slot = assets.library.slot(&car.entity)?;
            Some(Reach::of(
                &assets.library.meshes[slot],
                car.scale,
                car.shift,
            ))
        };
        let mut next = f32::INFINITY;
        let mut passage = |route: &Route, reach, departure: f32, tail: f32| {
            let path = route
                .distance()
                .iter()
                .zip(route.points())
                .map(|(&distance, &position)| (departure + distance / speed, position));
            view.times(path, reach, |start, end| {
                if let Some(delay) = train_arrival(seconds, start, end, tail) {
                    next = next.min(delay);
                }
            });
        };
        for service in &self.services {
            let mut tail = 0.;
            for &index in &service.cars {
                let car = assets.wagon(index);
                tail += car.length / speed;
                if let Some(reach) = reach(car) {
                    passage(
                        &self.routes[service.route],
                        reach,
                        service.departed as f32,
                        tail,
                    );
                }
            }
        }
        let mut future = None;
        for consist in self.consists.values().flatten() {
            for index in consist
                .fixed()
                .chain(consist.wagon_last)
                .chain(consist.wagons.iter().copied())
            {
                if let Some(car) = reach(assets.wagon(index)) {
                    future.get_or_insert_with(Reach::default).enclose(car);
                }
            }
        }
        if let Some(reach) = future {
            for (weight, routes) in self.weights.iter().zip(&self.departures) {
                if *weight > 0 {
                    for &route in routes {
                        passage(&self.routes[route], reach, self.next_departure as f32, 0.);
                    }
                }
            }
        }
        next
    }

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
            let longest = self
                .routes
                .iter()
                .map(|r| *r.distance().last().unwrap())
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
        if viewer_runtime::view_record::on() {
            let delta = self.drawn_time.map_or(0.0, |was| (time - was).max(0.0));
            let allowed = speed as f32 * delta + 0.1;
            let mut drawn = Vec::with_capacity(self.services.len());
            let (mut largest, mut jumps) = (0.0f32, 0u64);
            for service in &self.services {
                let route = &self.routes[service.route];
                let (head, _) = route.at(((f64::from(time) - service.departed) * speed) as f32);
                let key = (route.path.clone(), service.departed.to_bits());
                if let Some((_, _, was)) = self.drawn.iter().find(|(path, departed, _)| *path == key.0 && *departed == key.1) {
                    let moved = ((head[0] - was[0]).powi(2) + (head[2] - was[2]).powi(2)).sqrt();
                    largest = largest.max(moved - allowed + 0.1);
                    jumps += u64::from(moved > allowed);
                }
                drawn.push((key.0, key.1, head));
            }
            self.drawn = drawn;
            self.drawn_time = Some(time);
            viewer_runtime::view_record::set("trains", "running", self.services.len() as u64);
            viewer_runtime::view_record::add("trains", "jumps", jumps);
            viewer_runtime::view_record::set("trains", "largest_move_past_own_speed_milli", (largest.max(0.0) * 1000.0) as u64);
        }
        for service in &self.services {
            let route = &self.routes[service.route];
            let length = *route.distance().last().unwrap();
            let head = ((f64::from(time) - service.departed) * speed) as f32;
            let mut offset = 0.0;
            for (car_index, &car) in service.cars.iter().enumerate() {
                let car = assets.wagon(car);
                let front = head - offset;
                offset += car.length;
                let mut back = head - offset;
                let (front_position, _) = route.at(front);
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
                            } else if !route.open_end() && front > length - 2.5 {
                                let (p, t) = route.at(length);
                                let n = t[0].hypot(t[2]).max(0.0001);
                                [p[0], p[2], -t[0] / n, -t[2] / n]
                            } else {
                                [0.0; 4]
                            },
                            animation: viewer_models::entity_animation::TRAIN,
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

fn car_yaw(dx: f32, dz: f32, rotation: f32) -> f32 {
    dz.atan2(dx) + rotation
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub trait TrainRows: CreatorRows {
    fn first_country(&self, slot: u32) -> io::Result<Option<u32>>;
    fn complete(&self) -> io::Result<bool>;
    fn inventory(&self, country: u32) -> io::Result<Option<(u32, Vec<TrainAllocation>, SparseBytes)>>;
    fn links_ready(&self) -> io::Result<()>;
    fn variant(&self, id: u64) -> Option<(u32, u32)>;
    fn token(&self, token: u32) -> Result<String, String>;
    fn provinces_ready(&self) -> io::Result<()>;
    fn fallback(&self) -> io::Result<Box<dyn Fallback + '_>>;
    fn nodes(&self, country: u32) -> io::Result<Vec<NodeValues>>;
    fn allowed(&self, province: u32, access: &SparseBytes) -> bool;
    fn rail(&self, from: u32, to: u32) -> bool;
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NodeValues {
    pub index: u32,
    pub province: u32,
    pub kind: u32,
    pub flag: u8,
    pub available: i64,
    pub consumed: i64,
    pub parent: i32,
    pub route_from: u16,
    pub route_to: u16,
    pub break_after: u8,
    pub path: Vec<u32>,
}

impl NodeValues {
    pub fn of(row: &SupplyNodeRow) -> Self {
        Self {
            index: row.index,
            province: row.province,
            kind: row.kind,
            flag: row.flag,
            available: row.available,
            consumed: row.consumed,
            parent: row.parent,
            route_from: row.route_from,
            route_to: row.route_to,
            break_after: row.break_after,
            path: row.path.clone(),
        }
    }

    pub fn is_vertex(&self) -> bool {
        self.index & SupplyNodeRow::VERTEX != 0
    }
}

pub trait SupplyNode {
    fn province(&self) -> u32;
    fn kind(&self) -> u32;
    fn flag(&self) -> u8;
    fn spare(&self) -> i64;
    fn parent(&self) -> i32;
    fn route_from(&self) -> u16;
    fn break_after(&self) -> u8;
    fn path(&self) -> &[u32];
}

impl SupplyNode for SupplyNodeRow {
    fn province(&self) -> u32 {
        self.province
    }
    fn kind(&self) -> u32 {
        self.kind
    }
    fn flag(&self) -> u8 {
        self.flag
    }
    fn spare(&self) -> i64 {
        self.available.wrapping_sub(self.consumed)
    }
    fn parent(&self) -> i32 {
        self.parent
    }
    fn route_from(&self) -> u16 {
        self.route_from
    }
    fn break_after(&self) -> u8 {
        self.break_after
    }
    fn path(&self) -> &[u32] {
        &self.path
    }
}

impl SupplyNode for NodeValues {
    fn province(&self) -> u32 {
        self.province
    }
    fn kind(&self) -> u32 {
        self.kind
    }
    fn flag(&self) -> u8 {
        self.flag
    }
    fn spare(&self) -> i64 {
        self.available.wrapping_sub(self.consumed)
    }
    fn parent(&self) -> i32 {
        self.parent
    }
    fn route_from(&self) -> u16 {
        self.route_from
    }
    fn break_after(&self) -> u8 {
        self.break_after
    }
    fn path(&self) -> &[u32] {
        &self.path
    }
}

pub fn route_curve(
    rails: &super::railways::RailGeometry,
    path: &[u32],
    network: &super::railways::RailNetwork,
    heights: &HeightField<'_>,
) -> Curve {
    rails.train_route(path, network, heights).map(|route| {
        let mut distance = vec![0.0];
        for pair in route.points.windows(2) {
            distance.push(
                distance.last().unwrap() + (pair[0][0] - pair[1][0]).hypot(pair[0][2] - pair[1][2]),
            );
        }
        Arc::new((route, distance))
    })
}

struct FrameRows<'a> {
    frame: &'a DecodedFrame,
    creators: CreatorLookup<'a>,
    tokens: TokenNames<'a>,
    complete: bool,
    controllers: BTreeMap<u32, u32>,
    closed: BTreeSet<u32>,
    rail: BTreeSet<(u32, u32)>,
}

impl<'a> FrameRows<'a> {
    fn new(frame: &'a DecodedFrame, cultures: &'a Cultures) -> io::Result<Self> {
        let complete = rows::<tables::SupplyNodes>(frame)?.is_complete()
            && rows::<tables::TrainInventory>(frame)?.is_complete()
            && rows::<tables::VariantDefinitions>(frame)?.is_complete()
            && rows::<tables::RailEdges>(frame)?.is_complete()
            && rows::<tables::RailCooldowns>(frame)?.is_complete()
            && rows::<tables::ProvinceSupply>(frame)?.is_complete()
            && rows::<tables::NavalBases>(frame)?.is_complete();
        let appearance = rows::<tables::CountryAppearance>(frame)?;
        let creators = CreatorLookup {
            frame,
            links: rows::<tables::CountryLinks>(frame)?.rows().iter().map(|row| (row.id, row.canonical_slot)).collect(),
            countries: rows::<tables::Countries>(frame)?.rows().iter().map(|row| (row.canonical_slot, row)).collect(),
            appearance: appearance.rows().iter().map(|row| (row.country, row)).collect(),
            appearance_complete: appearance.is_complete(),
            cultures,
        };
        Ok(Self {
            frame,
            creators,
            tokens: TokenNames::new(frame)?,
            complete,
            controllers: rows::<tables::Provinces>(frame)?.rows().iter().map(|row| (row.id, row.controller)).collect(),
            closed: rows::<tables::RailCooldowns>(frame)?.rows().iter().filter(|r| r.cooldown_ticks > 0).map(|r| r.province).collect(),
            rail: rows::<tables::RailEdges>(frame)?.rows().iter().flat_map(|e| [(e.from, e.to), (e.to, e.from)]).collect(),
        })
    }
}

impl CreatorRows for FrameRows<'_> {
    fn link(&self, raw: u32) -> Option<u32> {
        self.creators.link(raw)
    }
    fn country(&self, slot: u32) -> Option<(u32, Vec<u8>)> {
        self.creators.country(slot)
    }
    fn appearance(&self, country: u32) -> Option<CreatorAppearance> {
        self.creators.appearance(country)
    }
    fn appearance_complete(&self) -> bool {
        self.creators.appearance_complete
    }
    fn reference(&self, slot: u32) -> Result<i32, String> {
        self.creators.reference(slot)
    }
    fn reserved_tag(&self) -> Result<String, String> {
        self.creators.reserved_tag()
    }
}

impl TrainRows for FrameRows<'_> {
    fn first_country(&self, slot: u32) -> io::Result<Option<u32>> {
        Ok(core::<tables::Countries>(self.frame)?.iter().find(|c| c.canonical_slot == slot).map(|c| c.id))
    }

    fn complete(&self) -> io::Result<bool> {
        Ok(self.complete)
    }

    fn inventory(&self, country: u32) -> io::Result<Option<(u32, Vec<TrainAllocation>, SparseBytes)>> {
        Ok(rows::<tables::TrainInventory>(self.frame)?
            .rows()
            .iter()
            .find(|r| r.country == country)
            .map(|row| (row.node_count, row.allocations.clone(), row.controller_access.clone())))
    }

    fn links_ready(&self) -> io::Result<()> {
        core::<tables::CountryLinks>(self.frame).map(drop)
    }

    fn variant(&self, id: u64) -> Option<(u32, u32)> {
        rows::<tables::VariantDefinitions>(self.frame).ok()?.rows().iter().find(|row| row.id == id).map(|row| (row.equipment_token, row.creator_link))
    }

    fn token(&self, token: u32) -> Result<String, String> {
        self.tokens.name(token).map(str::to_owned).map_err(|reason| reason.to_string())
    }

    fn provinces_ready(&self) -> io::Result<()> {
        core::<tables::Provinces>(self.frame).map(drop)
    }

    fn fallback(&self) -> io::Result<Box<dyn Fallback + '_>> {
        let fallback = FallbackEndpoints::new(
            rows::<tables::SupplyNodes>(self.frame)?.rows(),
            core::<tables::Countries>(self.frame)?,
            core::<tables::CountryLinks>(self.frame)?,
            core::<tables::Provinces>(self.frame)?,
        )?;
        Ok(Box::new(fallback))
    }

    fn nodes(&self, country: u32) -> io::Result<Vec<NodeValues>> {
        Ok(rows::<tables::SupplyNodes>(self.frame)?.rows().iter().filter(|n| n.country == country).map(NodeValues::of).collect())
    }

    fn allowed(&self, province: u32, access: &SparseBytes) -> bool {
        self.controllers.get(&province).is_some_and(|&controller| {
            let slot = if controller == 0 { Some(0) } else { self.creators.links.get(&controller).copied() };
            !self.closed.contains(&province) && slot.and_then(|slot| access.get(slot)).is_some_and(|b| b != 0)
        })
    }

    fn rail(&self, from: u32, to: u32) -> bool {
        self.rail.contains(&(from, to))
    }
}

pub fn node_weight(node: &NodeValues) -> i64 {
    departure_weight(node)
}

#[derive(Default)]
pub struct PlanCache {
    nodes: HashMap<u32, Arc<Vec<Vec<u32>>>>,
    by_province: HashMap<u32, HashSet<u32>>,
    by_pair: HashMap<(u32, u32), HashSet<u32>>,
}

impl PlanCache {
    fn keep(&mut self, node: u32, paths: Arc<Vec<Vec<u32>>>, read: Vec<u32>) {
        for province in read.into_iter().chain(paths.iter().flatten().copied()) {
            self.by_province.entry(province).or_default().insert(node);
        }
        for pair in paths.iter().flat_map(|path| path.windows(2).map(|pair| (pair[0].min(pair[1]), pair[0].max(pair[1])))) {
            self.by_pair.entry(pair).or_default().insert(node);
        }
        self.nodes.insert(node, paths);
    }

    pub fn forget_provinces(&mut self, provinces: impl IntoIterator<Item = u32>) {
        for province in provinces {
            for node in self.by_province.remove(&province).unwrap_or_default() {
                self.nodes.remove(&node);
            }
        }
    }

    pub fn forget_pairs(&mut self, pairs: impl IntoIterator<Item = (u32, u32)>) {
        for (a, b) in pairs {
            for node in self.by_pair.remove(&(a.min(b), a.max(b))).unwrap_or_default() {
                self.nodes.remove(&node);
            }
        }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn held(&self) -> (usize, usize) {
        let paths: usize = self.nodes.values().map(|paths| paths.iter().map(|path| path.len() * 4 + 24).sum::<usize>() + 48).sum();
        let index: usize = self.by_province.values().chain(self.by_pair.values()).map(|nodes| nodes.len() * 8 + 48).sum();
        (self.nodes.len(), paths + index)
    }
}

fn departure_weight(node: &impl SupplyNode) -> i64 {
    let spare = node.spare();
    if node.flag() != 0 || node.kind() & !5 == 0 || spare <= 0 {
        return 0;
    }
    (spare / 100000 + i64::from(spare % 100000 != 0)).wrapping_mul(100000)
}

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

}

pub trait Fallback {
    fn emits(&self, parent: u32, province: u32) -> io::Result<bool>;
}

impl Fallback for FallbackEndpoints {
    fn emits(&self, parent: u32, province: u32) -> io::Result<bool> {
        let source = self.contains(parent)?;
        let destination = self.contains(province)?;
        Ok(source && destination)
    }
}

#[cfg(test)]
fn land_portions<N: SupplyNode>(
    start: &N,
    nodes: &BTreeMap<u32, &N>,
    fallback: &(impl Fallback + ?Sized),
) -> io::Result<Vec<Vec<u32>>> {
    land_portions_reading(start, nodes, fallback, &mut Vec::new())
}

fn land_portions_reading<N: SupplyNode>(
    start: &N,
    nodes: &BTreeMap<u32, &N>,
    fallback: &(impl Fallback + ?Sized),
    read: &mut Vec<u32>,
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
    read.push(node.province());
    for _ in 0..100 {
        if node.parent() < 1 {
            break;
        }
        read.push(node.parent() as u32);
        if node.route_from() != 0 {
            path.extend_from_slice(node.path());
            if node.break_after() != 0 {
                flush(&mut path, &mut out);
            }
        } else if fallback.emits(node.parent() as u32, node.province())? {
            flush(&mut path, &mut out);
        }
        let Some(parent) = nodes.get(&(node.parent() as u32)) else {
            return Err(invalid(
                "SupplyNodes parent chain reaches a province without a row",
            ));
        };
        node = parent;
    }
    flush(&mut path, &mut out);
    Ok(out)
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(test)]
mod tests {
    use super::*;
    use super::test_support::{DEFINES, assets, consist, plan, route};
    use replay_data::{
        rows::Recorded, rows::ReservedCountryNamingRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::{frame::Metrics, frame::TableView};
    use viewer_runtime::presentation::Continuity;

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
                "country appearance is incomplete",
            ),
            (
                TableId::ReservedCountryNaming,
                0,
                "reserved country naming is unavailable",
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
                assert!(reason.contains("original tag reference"), "{reason}");
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

    #[test]
    fn car_yaw_matches_the_back_to_front_heading() {
        let yaw = car_yaw(2967.11 - 2969.10, 1564.95 - 1564.24, 90f32.to_radians());
        assert!((yaw.cos() + 0.337).abs() < 0.005, "{yaw}");
        assert!((yaw.sin() + 0.942).abs() < 0.005, "{yaw}");
    }

    #[test]
    fn last_wagon_is_appended_after_random_fill() {
        let counts: BTreeSet<_> = (0..100)
            .map(|seed| consist_cars(&consist(), &DEFINES, &mut Random::new(seed)).len())
            .collect();
        assert_eq!(counts, BTreeSet::from([6, 7, 8, 9]));
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
        let bare = Consist {
            wagons: Vec::new(),
            ..consist()
        };
        assert_eq!(
            consist_cars(&bare, &DEFINES, &mut Random::default()),
            [0, 0, 0]
        );
    }

    #[test]
    fn wagon_counts_are_each_at_least_one_before_ordering() {
        let wagons = |counts: &str| {
            let defines = Defines::new().unwrap();
            defines
                .load(
                    "common/defines/00_graphics.lua",
                    format!(
                        "NDefines = {{ NGraphics = {{ TRAIN_MAP_SPEED = 4, \
                         TUNNELBANA_TIMETABLE = {{ 9100, 12100 }}, {counts} }} }}"
                    )
                    .as_bytes(),
                )
                .unwrap();
            TrainDefines::load(&defines).map(|read| read.wagon_count)
        };
        assert_eq!(
            wagons("MIN_TRAIN_WAGON_COUNT = 2, MAX_TRAIN_WAGON_COUNT = 7").unwrap(),
            [2, 7]
        );
        let missing = TrainDefines {
            wagon_count: wagons("").unwrap(),
            ..DEFINES
        };
        assert_eq!(missing.wagon_count, [1, 1]);
        assert_eq!(missing.cars(12345), 2);
        assert_eq!(
            wagons("MAX_TRAIN_WAGON_COUNT = 7").unwrap(),
            [1, 7],
            "a missing minimum"
        );
        assert_eq!(
            wagons(r#"MIN_TRAIN_WAGON_COUNT = "3", MAX_TRAIN_WAGON_COUNT = true"#).unwrap(),
            [1, 1],
            "non-number counts"
        );
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
    fn timetable_endpoints_follow_native_truncation_and_refuse_invalid_ranges() {
        let load = |entries: &str| {
            let defines = Defines::new().unwrap();
            defines
                .load(
                    "common/defines/00_graphics.lua",
                    format!(
                        "NDefines = {{ NGraphics = {{ TRAIN_MAP_SPEED = 4, \
                         TUNNELBANA_TIMETABLE = {{ {entries} }}, \
                         MIN_TRAIN_WAGON_COUNT = 2, MAX_TRAIN_WAGON_COUNT = 5 }} }}"
                    )
                    .as_bytes(),
                )
                .unwrap();
            TrainDefines::load(&defines)
        };
        assert_eq!(load("9100, 12100").unwrap().timetable, [9100, 12100]);
        assert_eq!(load("12100, 9100").unwrap().timetable, [9100, 12100]);
        assert_eq!(load("5000").unwrap().timetable, [5000, 5000]);
        assert_eq!(load("1.8, 3.9").unwrap().timetable, [1, 3]);
        assert_eq!(load("12.9, 4.1").unwrap().timetable, [4, 12]);
        let zero_from_fraction = load("0.8, 3.0").unwrap_err().to_string();
        assert!(
            zero_from_fraction.contains("TUNNELBANA_TIMETABLE")
                && zero_from_fraction.contains("non-positive"),
            "{zero_from_fraction}"
        );
        for negative in ["-5, 100", "-1.8, 100", "100, -1"] {
            let error = load(negative).unwrap_err().to_string();
            assert!(
                error.contains("TUNNELBANA_TIMETABLE") && error.contains("non-positive"),
                "{negative}: {error}"
            );
        }
        let empty = load("").unwrap_err().to_string();
        assert!(
            empty.contains("TUNNELBANA_TIMETABLE") && empty.contains("empty"),
            "{empty}"
        );
        for out_of_range in ["3000000000, 4000000000", "-3000000000, 100", "1/0, 100", "0/0, 100"] {
            let error = load(out_of_range).unwrap_err().to_string();
            assert!(
                error.contains("TUNNELBANA_TIMETABLE") && error.contains("32-bit signed integer"),
                "{out_of_range}: {error}"
            );
        }
    }

    #[test]
    fn frame_partition_and_backward_seek_do_not_change_departures() {
        content_reader_script::test_game::enter();
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
        let mut pass = viewer_models::ModelPass::new(&assets.library, "trains");
        pass.replace_field(&assets.library, field);
        pass.animate(1., None, &[0.]);
        assert!(pass.events.is_empty());
    }

    #[test]
    fn token_spellings_name_declared_equipment_under_the_lexers_fold() {
        let (equipment, _) = content_reader_key_names::equipment::fixture::load(&[(
            "common/units/equipment/trains.txt",
            "equipments = { train_equipment = { is_archetype = yes }
                train_equipment_1 = { archetype = TRAIN_EQUIPMENT } orphan = { } }",
        )]);
        let assets = TrainAssets {
            equipment,
            ..assets()
        };
        let key = assets.equipment_key("Train_Equipment_1").unwrap();
        assert_eq!(key, "train_equipment_1");
        assert_eq!(
            assets.equipment_key("armored_train"),
            Err(format!(
                "equipment armored_train is not declared in {}",
                content_reader_key_names::equipment::DIRECTORY
            ))
        );
        assert!(
            assets
                .archetype("orphan")
                .unwrap_err()
                .contains("null-object fallback token is unavailable")
        );
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
        assert_eq!(train_arrival(8., 10., 12., 3.), Some(2.));
        assert_eq!(train_arrival(14., 10., 12., 3.), Some(0.));
        assert_eq!(train_arrival(16., 10., 12., 3.), None);
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
        previous.routes.push(route(vec![3, 4], vec![[0.0, 10.0, 1.0], [600.0, 10.0, 1.0]], vec![0.0, 600.0], SEED ^ 3 ^ 4));
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
        Arc::make_mut(&mut changed.routes[0].curve).0.points[0][0] = 1.0;
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
    fn a_remade_route_leaves_the_trains_on_the_others_in_place() {
        let assets = assets();
        let mut previous = plan();
        previous.routes.push(route(vec![3, 4], vec![[0.0, 10.0, 1.0], [600.0, 10.0, 1.0]], vec![0.0, 600.0], SEED ^ 3 ^ 4));
        previous.weights = vec![100000, 100000];
        previous.departures = vec![vec![0], vec![1]];
        previous.field(&assets, 45.0);
        let on = |plan: &TrainPlan, path: &[u32]| plan.running().into_iter().filter(|(on, _)| *on == path).map(|(_, departed)| departed.to_bits()).collect::<Vec<_>>();
        let kept = on(&previous, &[3, 4]);
        assert!(!kept.is_empty() && !on(&previous, &[1, 2]).is_empty(), "trains run on both routes");
        let mut next = plan();
        next.hour += 1;
        next.routes = vec![route(vec![1, 2], vec![[0.0, 10.0, 0.0], [300.0, 10.0, 0.0], [600.0, 10.0, 5.0]], vec![0.0, 300.0, 600.0], SEED), previous.routes[1].clone()];
        next.weights = previous.weights.clone();
        next.departures = previous.departures.clone();
        assert!(Arc::ptr_eq(next.route_geometry(&[3, 4]).unwrap(), previous.route_geometry(&[3, 4]).unwrap()), "the unchanged route's curve is shared, not copied");
        next.carry_presentation(&mut previous, Continuity::Continue);
        assert_eq!(on(&next, &[3, 4]), kept, "the trains on the unchanged route keep their places");
        assert!(on(&next, &[1, 2]).is_empty(), "the remade route's trains go");
        assert_eq!(next.last_time, Some(45.0));
    }

    #[test]
    fn endpoint_extrapolation_keeps_cars_from_piling_up() {
        let plan = plan();
        let route = &plan.routes[0];
        assert_eq!(route.at(-2.0).0, [-2.0, 10.0, 0.0]);
        assert!((route.at(603.0).0[0] - 603.0).abs() < 0.001);
        assert_eq!(route.at(9.0).0, [9.0, 10.0, 0.0]);
    }
}
