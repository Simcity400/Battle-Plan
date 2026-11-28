
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use viewer_gpu::province_map::province_at;
use viewer_foundation::trigger_content::TriggerContent;
pub mod cycle;
mod details;
pub mod eligibility;
pub mod launch;
pub mod offers;
pub mod paths;
pub mod unit_inputs;
use viewer_unit_reads::movement_reads as reach;
pub mod script;
pub mod tooltips;
pub mod windows;
pub use windows::{PreparedRaids, prepare};

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
};

use content_reader_layers::ResolvedLayers;
use content_reader_key_names::buildings::{BuildingType, Buildings};
use content_reader_gui::gui::native;
use content_reader_map::{map::{MapData, ProvinceKind}, province_geometry::ProvinceGeometry};
use content_reader_raids::{raids::Catalog, raids::RaidType};
use replay_data::{rows::RaidRow, rows::RaidScope, rows::tables};
use replay_frame::frame::DecodedFrame;

use script::{Eval, REFUSED, Scope, Scopes, World, refused};
use viewer_foundation::mode_visibility::RaidReach;
use viewer_gpu::camera::Camera;
use viewer_native_gui::{
    native_gui::{self, SpriteSetter, lifecycle::MapIconPool},
    sprites::{self, Batch},
};
use dashboard_text::table_access::{Refusals, TokenKeys, refuse};

pub const UNKNOWN_GLYPH: &str = "GFX_raid_type_icon_unknown";
pub const OTHER_GLYPH: &str = "GFX_raid_type_icon_other_raid";
use viewer_native_gui::native_windows::RAID_WINDOW as WINDOW;
const FRAME_SPRITE: &[u8] = b"GFX_on_map_raids_icons_frame_strip";

type TemplateNames = HashMap<Box<[u8]>, Result<String, String>>;
type GlyphKeys = HashMap<(usize, Option<String>), Arc<str>>;

pub struct RaidAssets {
    pub trigger_content: TriggerContent,
    glyphs: BTreeSet<String>,
    pub catalog: Catalog,
    pub tooltip: tooltips::content::Content,
    pub priority: i8,
    pub geometry: Arc<ProvinceGeometry>,
    pub paths: Result<paths::PathSearch, String>,
    pub moves: Result<content_reader_unit_path::unit_path::MoveGraph, String>,
    pub(crate) recorded_indexes: launch::RecordedIndexes,
    pub(crate) table_indexes: launch::TableIndexes,
    batch: Result<usize, String>,
    type_keys: OnceLock<TokenKeys>,
    template_names: Mutex<TemplateNames>,
    glyph_keys: Mutex<GlyphKeys>,
    template_keys: OnceLock<TokenKeys>,
    pub(crate) source_distances: launch::SourceDistanceSets,
    pub regions: BTreeMap<u32, u32>,
    pub region_provinces: BTreeMap<u32, Vec<u32>>,
    day_night: Result<replay_data::day_night::Defines, String>,
    pub transport_sub_units: Result<content_reader_script::sub_units::SubUnits, String>,
    pub combat_minimum_time: i32,
    pub naval_combat_min_duration: Result<i32, String>,
    pub avoid_enemy: bool,
    sites: BTreeMap<(u32, Site), Vec<u32>>,
    refused_sites: Option<String>,
    pub equipment: content_reader_key_names::equipment::Database,
    pub equipment_groups: content_reader_key_names::equipment::Groups,
    pub equipment_lost: Option<String>,
    pub defines: RaidDefines,
    pub warnings: Vec<content_reader_parser::Diagnostic>,
}
impl std::fmt::Debug for RaidAssets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("RaidAssets");
        debug.field("glyphs", &self.glyphs);
        debug.field("catalog", &self.catalog);
        debug.field("triggers", &self.trigger_content.triggers);
        debug.field("tag_aliases", &self.trigger_content.tag_aliases);
        debug.field("tooltip", &self.tooltip);
        debug.field("priority", &self.priority);
        debug.field("buildings", &self.trigger_content.buildings);
        debug.field("dlcs", &self.trigger_content.dlcs);
        debug.field("state_provinces", &self.trigger_content.state_provinces);
        debug.field("impassable", &self.trigger_content.impassable);
        debug.field("neighbor_states", &self.trigger_content.neighbor_states);
        debug.field("terrain", &self.trigger_content.terrain);
        debug.field("resource_bonuses", &self.trigger_content.resource_bonuses);
        debug.field("geometry", &self.geometry);
        debug.field("paths", &self.paths);
        debug.field("batch", &self.batch);
        debug.field("regions", &self.regions);
        debug.field("region_provinces", &self.region_provinces);
        debug.field("day_night", &self.day_night);
        debug.field("transport_sub_units", &self.transport_sub_units);
        debug.field("combat_minimum_time", &self.combat_minimum_time);
        debug.field("naval_combat_min_duration", &self.naval_combat_min_duration);
        debug.field("avoid_enemy", &self.avoid_enemy);
        debug.field("sites", &self.sites);
        debug.field("refused_sites", &self.refused_sites);
        debug.field("equipment", &self.equipment);
        debug.field("equipment_groups", &self.equipment_groups);
        debug.field("equipment_lost", &self.equipment_lost);
        debug.field("leader_roles", &self.trigger_content.leader_roles);
        debug.field(
            "invasion_prepare_days",
            &self.trigger_content.invasion_prepare_days,
        );
        debug.field("script_constants", &self.trigger_content.script_constants);
        debug.field("defines", &self.defines);
        debug.field("warnings", &self.warnings);
        debug.field(
            "moves",
            &self.moves.as_ref().map(|graph| graph.debug_values()),
        );
        debug.finish()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RaidDefines {
    pub base_max_command_power: i64,
    pub naval_invasion_priority: i32,
    pub map_scale: i64,
    pub naval_commando_distance: i64,
    pub ace_wing_size: i64,
    pub ace_wing_size_max_bonus: i64,
    pub unit_speeds: Option<[i64; 3]>,
}

impl RaidDefines {
    pub fn read(defines: &content_reader_defines::defines::Defines) -> io::Result<Self> {
        Ok(Self {
            base_max_command_power: defines.fixed5("NCountry", "BASE_MAX_COMMAND_POWER")?,
            naval_invasion_priority: defines.int32("NNavy", "NAVAL_INVASION_PRIORITY")?,
            map_scale: defines.fixed5("NGame", "MAP_SCALE_PIXEL_TO_KM")?,
            naval_commando_distance: defines
                .fixed5("NRaids", "BASE_NAVAL_COMMANDO_RAID_DISTANCE")?,
            ace_wing_size: defines.fixed5("NAir", "ACE_WING_SIZE")?,
            ace_wing_size_max_bonus: defines.fixed5("NAir", "ACE_WING_SIZE_MAX_BONUS")?,
            unit_speeds: (|| -> io::Result<[i64; 3]> {
                Ok([
                    defines.fixed5("NMilitary", "LAND_SPEED_MODIFIER")?,
                    defines.fixed5("NNavy", "NAVAL_SPEED_MODIFIER")?,
                    defines.fixed5("NNavy", "NAVAL_TRANSFER_BASE_SPEED")?,
                ])
            })()
            .ok(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Site {
    Radar,
    AirBase,
    Rocket,
    Gun,
}

impl RaidAssets {
    pub fn movement_reads(&self) -> reach::ReadOperands<'_> {
        reach::ReadOperands {
            moves: &self.moves,
            trigger_content: &self.trigger_content,
            regions: &self.regions,
            geometry: &self.geometry,
            combat_minimum_time: self.combat_minimum_time,
            avoid_enemy: self.avoid_enemy,
            day_night: &self.day_night,
            naval_combat_min_duration: &self.naval_combat_min_duration,
        }
    }

    pub fn refusals(&self) -> Vec<String> {
        let tooltip = &self.tooltip;
        [
            self.equipment_lost.as_ref(),
            self.trigger_content.leader_roles.as_ref().err(),
            self.trigger_content.invasion_prepare_days.as_ref().err(),
            tooltip.intel.as_ref().err(),
            tooltip.speed.as_ref().err(),
            tooltip.prepare_days.as_ref().err(),
            tooltip.max_wing_xp.as_ref().err(),
            tooltip.night_threshold.as_ref().err(),
            tooltip.thresholds.as_ref().err(),
            tooltip.terrain_names.as_ref().err(),
            tooltip.unit_names.as_ref().err(),
        ]
        .into_iter()
        .flatten()
        .cloned()
        .collect()
    }

    pub fn equipment(
        &self,
    ) -> Result<
        (
            &content_reader_key_names::equipment::Database,
            &content_reader_key_names::equipment::Groups,
        ),
        String,
    > {
        match &self.equipment_lost {
            Some(lost) => Err(lost.clone()),
            None => Ok((&self.equipment, &self.equipment_groups)),
        }
    }

    pub fn equipment_category(&self, name: &str) -> Result<u64, String> {
        let (equipment, _) = self.equipment()?;
        equipment
            .named(name)
            .map(|kind| equipment.category(kind))
            .ok_or_else(|| format!("essential equipment {name} is not in the content"))
    }

    pub fn load(
        layers: &ResolvedLayers,
        defines: &content_reader_defines::defines::Defines,
        map: &MapData,
        registry: &native::resources::Registry,
    ) -> io::Result<Self> {
        Self::load_with_products(None, layers, defines, map, registry)
    }

    pub fn load_with_products(
        products: Option<&map_content::loader::Loader>,
        layers: &ResolvedLayers,
        defines: &content_reader_defines::defines::Defines,
        map: &MapData,
        registry: &native::resources::Registry,
    ) -> io::Result<Self> {
        let mut assets = Self::load_content_with_products(products, layers, defines, map)?;
        assets.retain_glyphs(registry);
        Ok(assets)
    }

    pub fn retain_glyphs(&mut self, registry: &native::resources::Registry) {
        self.glyphs
            .retain(|name| registry.sprite(name.as_bytes()).is_some());
    }

    pub fn load_content_with_products(
        products: Option<&map_content::loader::Loader>,
        layers: &ResolvedLayers,
        defines: &content_reader_defines::defines::Defines,
        map: &MapData,
    ) -> io::Result<Self> {
        let product = map_content::loader::TriggerLoader::new(layers, products);
        let mut warnings = Vec::new();
        let catalog = content_reader_raids::raids::read(layers)?;
        if !catalog.refused.is_empty() {
            warnings.push((content_reader_parser::DiagnosticKind::Refused, format!(
                "Raid types declared more than once cannot be used; targets they may be offered against are withheld: {}.",
                catalog.refused.join(", ")
            )));
        }
        for (name, reason) in &catalog.unsupported {
            warnings.push((content_reader_parser::DiagnosticKind::Unsupported, format!(
                "Raid type {name} cannot be read ({reason}); targets it may be offered against are withheld."
            )));
        }
        warnings.extend(catalog.misreads.iter().map(|(kind, misread)| {
            (
                *kind,
                format!("Raid content read as the game reads it: {misread}"),
            )
        }));
        let buildings = product.buildings()?;
        let tag_aliases = product.tag_aliases()?;
        warnings.extend(tag_aliases.unknown.iter().map(|reason| {
            (
                content_reader_parser::DiagnosticKind::Unsettled,
                format!("Raids: tag aliases: {reason}."),
            )
        }));
        let equipment = content_reader_key_names::equipment::Database::load(layers)?;
        let equipment_groups = content_reader_key_names::equipment::Groups::load(layers)?;
        let equipment_lost = match (&equipment.lost, &equipment_groups.lost) {
            (Some(lost), _) => Some(format!(
                "the equipment database cannot be followed past {lost}"
            )),
            (None, Some(lost)) => Some(format!(
                "the equipment groups cannot be followed past {lost}"
            )),
            (None, None) => None,
        };
        if let Some(lost) = &equipment_lost {
            warnings.push((
                content_reader_parser::DiagnosticKind::Unsettled,
                format!("Raids: {lost}, so every raid fact that reads equipment stays unknown."),
            ));
        }
        warnings.extend(
            equipment
                .misreads
                .iter()
                .chain(&equipment_groups.misreads)
                .map(|misread| {
                    (content_reader_parser::DiagnosticKind::Misread, format!(
                        "Equipment content: {misread}; raid requirements follow the game's reading."
                    ))
                }),
        );
        for (name, building) in &buildings.types {
            if let Some(reason) = &building.raid_refused {
                warnings.push((content_reader_parser::DiagnosticKind::Refused, format!(
                    "Building {name}'s raid facts are refused, so raids that read them stay unknown: {reason}."
                )));
            }
        }
        let mut wanted: BTreeSet<String> =
            [UNKNOWN_GLYPH.to_owned(), OTHER_GLYPH.to_owned()].into();
        for kind in &catalog.types {
            let base = kind.base_icon();
            for building in buildings.types.keys() {
                wanted.insert(format!("{base}_{building}"));
            }
            wanted.insert(base);
        }
        let glyphs = wanted;
        let leader_roles = product.leader_roles();
        if let Err(error) = &leader_roles {
            warnings.push((content_reader_parser::DiagnosticKind::Refused, format!(
                "Character templates cannot be read ({error}); unit leaders whose visibility has a condition are undecided."
            )));
        }
        let (states, _) = product.states(defines, map)?;
        let states = states.definitions();
        let (graph, geometry) = product.graph()?;
        if let Err(error) = &graph {
            warnings.push((content_reader_parser::DiagnosticKind::Refused, format!(
                "The province graph cannot be built ({error}); naval and building raid sources cannot be ranged."
            )));
        }
        let paths = graph.map(paths::PathSearch::new);
        let moves = paths
            .as_ref()
            .map(|paths| content_reader_unit_path::unit_path::MoveGraph::new(paths.graph(), &geometry))
            .map_err(Clone::clone);
        if let Ok(moves) = &moves {
            for rule in moves.rules() {
                if let Some(reason) = &rule.unsupported {
                    warnings.push((content_reader_parser::DiagnosticKind::Unsupported, format!(
                        "Adjacency rule {} is refused ({reason}); army paths across it are unknown.",
                        rule.name
                    )));
                }
            }
        }
        let (state_provinces, impassable) = map_content::loader::state_content(states);
        let neighbor_states =
            product.neighbor_states(paths.as_ref().map(|paths| paths.graph()), &state_provinces);
        if let Err(error) = &neighbor_states {
            warnings.push((content_reader_parser::DiagnosticKind::Refused, format!(
                "State neighbours cannot be derived ({error}); any_neighbor_state and all_neighbor_state are unknown."
            )));
        }
        let mut regions = BTreeMap::new();
        let database = content_reader_regions::regions::read(layers)?.regions;
        let region_provinces: BTreeMap<_, _> = database
            .into_iter()
            .map(|(id, region)| (id, region.provinces))
            .collect();
        let day_night = replay_data::day_night::Defines::read(
            |group, name| defines.int32(group, name),
            |group, name| defines.fixed5(group, name),
        )
        .map_err(|error| error.to_string());
        if let Err(error) = &day_night {
            warnings.push((content_reader_parser::DiagnosticKind::Refused, format!(
                "The day/night defines cannot be read ({error}); province clocks the game updated hourly are unknown."
            )));
        }
        for (&id, provinces) in &region_provinces {
            for &province in provinces {
                if regions
                    .insert(province, id)
                    .is_some_and(|previous| previous != id)
                {
                    return Err(invalid(format!(
                        "province {province} is in more than one strategic region"
                    )));
                }
            }
        }
        let priority =
            map_icon_priority(defines.number("NMapIcons", "TOP_MAP_ICON")?).map_err(invalid)?;
        let dlcs = product.dlcs()?;
        let resource_bonuses = product.resource_bonuses(defines)?;
        let recorded_indexes = Default::default();
        let table_indexes = Default::default();
        let batch = target_batch(defines);
        let type_keys = Default::default();
        let template_names = Default::default();
        let glyph_keys = Default::default();
        let template_keys = Default::default();
        let source_distances = Default::default();
        let transport_sub_units = Err("the shared sub-unit database is not loaded".into());
        let combat_minimum_time = defines.int32("NMilitary", "COMBAT_MINIMUM_TIME")?;
        let naval_combat_min_duration = defines
            .int32("NNavy", "COMBAT_MIN_DURATION")
            .map_err(|error| error.to_string());
        let avoid_enemy = defines.boolean("NRaids", "ARMY_TRANSFER_AVOID_ENEMY")?;
        let sites = sites(layers, map, states, &buildings)?;
        let refused_sites = buildings.types.iter().find_map(|(name, kind)| {
                kind.raid_refused.as_ref().map(|reason| {
                    format!(
                        "raid site anchors need building {name}'s raid facts, which are refused: {reason}"
                    )
                })
            });
        let continents = product.continents();
        let tag_aliases = Arc::new(tag_aliases);
        let tooltip = tooltips::content::Content::read(defines, states, layers);
        let surrender_limits = dashboard_text::triggers::SurrenderLimits::read(defines);
        let invasion_prepare_days = product.invasion_prepare_days(defines);
        let script_constants = product.constants();
        let defines = RaidDefines::read(defines)?;
        let trigger_content = map_content::loader::assemble_trigger_content(
            tag_aliases,
            leader_roles,
            buildings,
            state_provinces,
            impassable,
            neighbor_states,
            dlcs,
            resource_bonuses,
            continents,
            surrender_limits,
            invasion_prepare_days,
            script_constants,
        );
        Ok(Self {
            trigger_content,
            glyphs,
            priority,
            geometry,
            paths,
            moves,
            recorded_indexes,
            table_indexes,
            batch,
            type_keys,
            template_names,
            glyph_keys,
            template_keys,
            source_distances,
            regions,
            region_provinces,
            day_night,
            transport_sub_units,
            combat_minimum_time,
            naval_combat_min_duration,
            avoid_enemy,
            sites,
            refused_sites,
            catalog,
            tooltip,
            equipment,
            equipment_groups,
            equipment_lost,
            defines,
            warnings,
        })
    }

    pub fn mission_topology(
        &self,
    ) -> Result<
        (
            &content_reader_map::province_graph::ProvinceGraph,
            &ProvinceGeometry,
        ),
        String,
    > {
        Ok((
            self.paths.as_ref().map_err(Clone::clone)?.graph(),
            &self.geometry,
        ))
    }

    fn glyph_key(&self, kind: &RaidType, template: Option<&str>) -> String {
        kind.icon(template, |name| self.glyphs.contains(name))
            .unwrap_or_else(|| UNKNOWN_GLYPH.to_owned())
    }

    pub fn raid_type(&self, world: &World<'_>, token: u32) -> Result<TypeToken, String> {
        let spelling = spelling(world, token, "raid type")?;
        let keys = made_once(&self.type_keys, || {
            TokenKeys::new(
                self.catalog
                    .types
                    .iter()
                    .map(|kind| &kind.name)
                    .chain(self.catalog.uncertain.iter().map(|kind| &kind.name)),
            )
        });
        let key = keys
            .resolve(spelling)
            .map_err(|reason| refused(format!("raid type {reason}")))?
            .ok_or_else(|| refused(format!("raid type {spelling} is not in the content")))?;
        if let Some(index) = self.catalog.types.iter().position(|kind| kind.name == key) {
            return Ok(TypeToken::Type(index));
        }
        Ok(TypeToken::Refused(
            self.catalog
                .uncertain
                .iter()
                .position(|kind| kind.name == key)
                .expect("a key of the catalogue"),
        ))
    }

    pub fn building(&self, world: &World<'_>, token: u32) -> Result<&BuildingType, String> {
        let spelling = spelling(world, token, "building template")?;
        let known = self
            .template_names
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(spelling.as_bytes())
            .cloned();
        if let Some(known) = known {
            return known.map(|key| &self.trigger_content.buildings.types[&key]);
        }
        let keys = made_once(&self.template_keys, || TokenKeys::new(self.trigger_content.buildings.types.keys()));
        let key = keys
            .resolve(spelling)
            .map_err(|reason| refused(format!("building template {reason}")))
            .and_then(|key| {
                key.map(str::to_owned).ok_or_else(|| {
                    refused(format!(
                        "building template {spelling} is not an authored building type"
                    ))
                })
            });
        self.template_names
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(spelling.as_bytes().into(), key.clone());
        key.map(|key| &self.trigger_content.buildings.types[&key])
    }

    fn glyph_key_of(&self, index: usize, template: Option<&str>) -> Arc<str> {
        let key = (index, template.map(str::to_owned));
        if let Some(known) = self.glyph_keys.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(&key) {
            return Arc::clone(known);
        }
        let glyph: Arc<str> = self.glyph_key(&self.catalog.types[index], template).into();
        self.glyph_keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key, Arc::clone(&glyph));
        glyph
    }

    pub fn province(&self, scope: &RaidScope) -> Option<u32> {
        if scope.province != 0 {
            return Some(scope.province);
        }
        if scope.building_type != 0 && scope.building_flags & 1 != 0 {
            return Some(scope.building_location);
        }
        if scope.leader_province != 0 {
            return Some(scope.leader_province);
        }
        let state = if scope.state != 0 {
            scope.state
        } else if scope.building_type != 0 && scope.building_flags & 2 != 0 {
            scope.building_location
        } else {
            return None;
        };
        self.geometry
            .nearest_to_centre(self.trigger_content.state_provinces.get(&state)?)
    }

    pub fn anchor_province(
        &self,
        world: &World<'_>,
        scope: &RaidScope,
    ) -> Result<Option<u32>, String> {
        if scope.building_type != 0 && scope.building_flags & 2 != 0 {
            let building = self.building(world, scope.building_type)?;
            if let Some(reason) = &self.refused_sites {
                return Err(reason.clone());
            }
            let state = scope.building_location;
            let wanted: Vec<Site> = [
                (building.air_base, Site::AirBase),
                (building.rocket_launch_capacity > 0, Site::Rocket),
                (building.gun_emplacement, Site::Gun),
                (building.radar, Site::Radar),
            ]
            .into_iter()
            .filter_map(|(has, site)| has.then_some(site))
            .collect();
            for province in world.state_order(state).map_err(|why| why.to_string())? {
                if wanted.iter().any(|&site| {
                    self.sites
                        .get(&(state, site))
                        .is_some_and(|provinces| provinces.contains(province))
                }) {
                    return Ok(Some(*province));
                }
            }
        }
        Ok(self.province(scope))
    }
}

fn map_icon_priority(define: f64) -> Result<i8, String> {
    if define.fract() != 0.0 || !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&define) {
        return Err("NDefines.NMapIcons.TOP_MAP_ICON is not an integer".into());
    }
    Ok(define as i32 as i8)
}

fn target_batch(defines: &content_reader_defines::defines::Defines) -> Result<usize, String> {
    defines
        .int32("NRaids", "MAX_TARGETS_TO_UPDATE_PER_FRAME")
        .map(|rows| rows.max(1) as usize)
        .map_err(|error| error.to_string())
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn sites(
    layers: &ResolvedLayers,
    map: &MapData,
    states: &BTreeMap<u32, content_reader_states::states::StateDefinition>,
    buildings: &Buildings,
) -> io::Result<BTreeMap<(u32, Site), Vec<u32>>> {
    let mut out: BTreeMap<(u32, Site), Vec<u32>> = BTreeMap::new();
    for slot in content_reader_map::positions::read_buildings(layers)? {
        let Some(province) = province_at(map, slot.x, slot.z) else {
            continue;
        };
        let admitted = map
            .definitions
            .get(&province)
            .is_some_and(|definition| definition.kind == ProvinceKind::Land)
            && states
                .get(&slot.state)
                .is_some_and(|state| state.provinces.contains(&province));
        if !admitted {
            continue;
        }
        let candidates: Vec<&BuildingType> = buildings.slot_candidates(&slot.kind).collect();
        for (site, has) in [
            (
                Site::Rocket,
                candidates
                    .iter()
                    .any(|kind| kind.rocket_launch_capacity > 0),
            ),
            (
                Site::Gun,
                candidates.iter().any(|kind| kind.gun_emplacement),
            ),
            (Site::AirBase, candidates.iter().any(|kind| kind.air_base)),
            (Site::Radar, candidates.iter().any(|kind| kind.radar)),
        ] {
            if has {
                out.entry((slot.state, site)).or_default().push(province);
            }
        }
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq)]
pub struct RaidIcon {
    pub group: u32,
    pub province: u32,
    pub frame: u32,
    pub enabled: bool,
    pub glyph: Option<Arc<str>>,
    pub plus: bool,
    pub tooltip: Option<tooltips::Key>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RaidReport {
    pub icons: usize,
    pub own: usize,
    pub incoming: usize,
    pub offers: usize,
    pub unverified_tint: usize,
    pub withheld: usize,
    pub unanchored: usize,
    pub unresolved: usize,
    pub wrapped: usize,
    pub missing_glyphs: usize,
    pub unknown_glyphs: usize,
    pub pending: bool,
    pub worker_failed: bool,
    pub cycle: (usize, usize),
    pub reasons: BTreeMap<String, usize>,
    pub refused_tokens: Refusals,
}

impl RaidReport {
    fn note(&mut self, reasons: impl IntoIterator<Item = String>) {
        for reason in reasons {
            match reason.strip_prefix(REFUSED) {
                Some(refusal) => refuse(&mut self.refused_tokens, refusal.to_owned()),
                None => *self.reasons.entry(reason).or_default() += 1,
            }
        }
    }

    fn absorb(&mut self, other: &Self) {
        for (to, from) in [
            (&mut self.icons, other.icons),
            (&mut self.own, other.own),
            (&mut self.incoming, other.incoming),
            (&mut self.offers, other.offers),
            (&mut self.unverified_tint, other.unverified_tint),
            (&mut self.withheld, other.withheld),
            (&mut self.unanchored, other.unanchored),
            (&mut self.unresolved, other.unresolved),
            (&mut self.wrapped, other.wrapped),
            (&mut self.missing_glyphs, other.missing_glyphs),
            (&mut self.unknown_glyphs, other.unknown_glyphs),
        ] {
            *to += from;
        }
        for (reason, count) in &other.reasons {
            *self.reasons.entry(reason.clone()).or_default() += count;
        }
        for (reason, count) in &other.refused_tokens {
            *self.refused_tokens.entry(reason.clone()).or_default() += count;
        }
    }

    fn glyph(&mut self, assets: &RaidAssets, glyph: &str) {
        if !assets.glyphs.contains(glyph) {
            self.missing_glyphs += 1;
            self.note([format!("no interface file declares {glyph}")]);
        }
    }

    pub fn cycle_complete(&self) -> bool {
        self.cycle.0 == self.cycle.1
    }

    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.pending {
            out.push(
                "Raid icons for this hour and viewing country are still being built; none are drawn yet."
                    .to_owned(),
            );
        }
        if self.worker_failed {
            out.push(
                "Evaluating this hour's raid offers failed; the offers shown are those evaluated before."
                    .to_owned(),
            );
        }
        if !self.cycle_complete() {
            out.push(format!(
                "Raid offers: {} of {} targets evaluated at this hour; the others show their last evaluation since the reset, or nothing.",
                self.cycle.0, self.cycle.1
            ));
        }
        let gaps = [
            (
                self.unverified_tint,
                "offers drawn grey with an unverified tint",
            ),
            (self.withheld, "provinces withheld"),
            (self.unanchored, "icons with no anchor"),
            (self.unresolved, "raid targets with no province"),
            (self.wrapped, "byte counters wrapped"),
            (
                self.missing_glyphs,
                "icons drawn without their undeclared glyph",
            ),
            (
                self.unknown_glyphs,
                "incoming raids of a type the port cannot use (refused content or a refused token), drawn with the unknown glyph",
            ),
        ];
        let listed: Vec<String> = gaps
            .iter()
            .filter(|(count, _)| *count > 0)
            .map(|(count, what)| format!("{count} {what}"))
            .collect();
        if !listed.is_empty() {
            out.push(format!("Raid icons: {}.", listed.join(", ")));
        }
        out.extend(
            self.reasons
                .iter()
                .map(|(reason, count)| format!("Raid icons: {reason} ({count})")),
        );
        out.extend(
            self.refused_tokens
                .iter()
                .map(|(reason, count)| format!("Raid icons: {count} refused tokens: {reason}.")),
        );
        out
    }
}

pub fn spelling<'a>(world: &World<'a>, token: u32, what: &str) -> Result<&'a str, String> {
    let literal = world
        .token(token)
        .map_err(|reason| refused(format!("{what} token {token}: {reason}")))?
        .ok_or_else(|| refused(format!("{what} token {token} has no recorded literal")))?;
    std::str::from_utf8(literal)
        .ok()
        .filter(|name| !name.contains('\0'))
        .ok_or_else(|| refused(format!("{what} token {token} is not spelled as text")))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeToken {
    Type(usize),
    Refused(usize),
}

#[derive(Clone, Debug, Default)]
pub struct RaidIcons {
    pub icons: Vec<RaidIcon>,
    pub report: RaidReport,
    pub built_for: Option<(replay_data::BranchId, replay_data::Hour, u32)>,
}

impl RaidIcons {
    pub fn carried(
        icons: Arc<Self>,
        branch: replay_data::BranchId,
        hour: replay_data::Hour,
        ours: u32,
    ) -> Arc<Self> {
        match icons.built_for {
            Some(built) if built == (branch, hour, ours) => icons,
            _ => Arc::new(Self {
                report: RaidReport {
                    pending: true,
                    ..RaidReport::default()
                },
                ..Self::default()
            }),
        }
    }

    pub fn drawable_at(
        &self,
        branch: replay_data::BranchId,
        hour: replay_data::Hour,
        ours: u32,
    ) -> bool {
        match self.built_for {
            Some(built) => built == (branch, hour, ours),
            None => self.report.worker_failed && self.icons.is_empty(),
        }
    }

    pub fn unavailable(frame: &DecodedFrame, ours: u32, reason: String) -> Self {
        let mut out = Self {
            built_for: Some((frame.branch, frame.hour, ours)),
            ..Self::default()
        };
        out.report
            .note([format!("raid icons need the core tables: {reason}")]);
        out
    }
}

#[derive(Clone, Copy)]
struct Bound<'a> {
    raid: &'a RaidRow,
    scope: &'a RaidScope,
    raid_type: u32,
}

#[derive(Default)]
struct Entry<'a> {
    own: Vec<Bound<'a>>,
    incoming: Vec<Bound<'a>>,
    own_count: u8,
    incoming_count: u8,
}

pub fn icons(
    assets: &RaidAssets,
    frame: &DecodedFrame,
    ours: u32,
    paths: &(dyn launch::RaidPaths + Sync),
) -> RaidIcons {
    let world = match World::new(frame, assets.trigger_content.facts()) {
        Ok(world) => world,
        Err(reason) => return RaidIcons::unavailable(frame, ours, reason.to_string()),
    };
    let layer = HourLayer::new(assets, &world, ours, paths);
    let mut cycle = cycle::Cycle::new(frame.branch, ours);
    if cycle.begin(assets, &layer.sweep).is_ok() {
        while !cycle.complete() {
            cycle.step(&layer.sweep);
        }
    }
    layer.icons(&cycle, &mut IconCache::default())
}

#[derive(Default)]
pub struct IconCache {
    tokens: Option<replay_frame::frame::ViewIdentity>,
    icons: HashMap<u32, CachedIcon>,
}

struct CachedIcon {
    shown: Arc<cycle::Shown>,
    order: Result<(), dashboard_text::triggers::Failure>,
    icon: Option<RaidIcon>,
    report: RaidReport,
}

impl IconCache {
    fn hour(&mut self, frame: &dyn replay_frame::frame::DecodedSource) {
        let table = replay_data::rows::TableId::LexicalTokens;
        let view = frame.shared(table.index());
        let complete = matches!(&*view, replay_frame::frame::TableView::Complete(rows) if rows.table() == table);
        if !(complete && self.tokens.as_ref().is_some_and(|tokens| tokens.is(&view))) {
            self.icons.clear();
            self.tokens = complete.then(|| view.identity());
        }
    }
}

pub struct HourLayer<'s, 'f> {
    assets: &'s RaidAssets,
    world: &'s World<'f>,
    ours: u32,
    membership: Result<(BTreeMap<u32, Entry<'f>>, RaidReport), String>,
    pub sweep: offers::Sweep<'s, 'f>,
    glyphs: RefCell<GlyphsByToken>,
}

type GlyphsByToken = HashMap<(usize, u32), (Arc<str>, Option<String>)>;

impl<'s, 'f> HourLayer<'s, 'f> {
    pub fn new(
        assets: &'s RaidAssets,
        world: &'s World<'f>,
        ours: u32,
        paths: &'s (dyn launch::RaidPaths + Sync),
    ) -> Self {
        let mut report = RaidReport::default();
        let membership =
            membership(assets, world, ours, &mut report).map(|entries| (entries, report));
        Self {
            assets,
            world,
            ours,
            membership,
            sweep: offers::Sweep::new(assets, world, ours, paths),
            glyphs: Default::default(),
        }
    }

    fn glyph(&self, index: usize, token: u32, report: &mut RaidReport) -> Arc<str> {
        let mut glyphs = self.glyphs.borrow_mut();
        let (glyph, refused) = glyphs.entry((index, token)).or_insert_with(|| {
            let (template, refused) = if token == 0 {
                (None, None)
            } else {
                match self.assets.building(self.world, token) {
                    Ok(building) => (Some(building.name.as_str()), None),
                    Err(reason) => (None, Some(reason)),
                }
            };
            (self.assets.glyph_key_of(index, template), refused)
        });
        if let Some(reason) = refused {
            report.note([reason.clone()]);
        }
        Arc::clone(glyph)
    }

    pub fn icons(&self, cycle: &cycle::Cycle, cache: &mut IconCache) -> RaidIcons {
        let frame = self.world.frame();
        let mut out = RaidIcons {
            built_for: Some((frame.branch(), frame.hour(), self.ours)),
            ..RaidIcons::default()
        };
        match &self.membership {
            Err(reason) => {
                out.report.note([reason.clone()]);
                out.report.cycle = cycle.progress();
                out
            }
            Ok((entries, report)) => {
                out.report = report.clone();
                out.report.cycle = cycle.progress();
                if let Some(reason) = cycle.restart() {
                    out.report
                        .note([format!("raid offers started their cycle again: {reason}")]);
                }
                out.report.note(self.sweep.reasons.iter().cloned());
                finish(self, entries, cycle, cache, out)
            }
        }
    }
}

fn finish(
    layer: &HourLayer<'_, '_>,
    entries: &BTreeMap<u32, Entry<'_>>,
    cycle: &cycle::Cycle,
    cache: &mut IconCache,
    mut out: RaidIcons,
) -> RaidIcons {
    let (assets, world, ours) = (layer.assets, layer.world, layer.ours);
    let empty = BTreeMap::new();
    let shown = if cycle.withheld().is_empty() {
        out.report.unresolved += cycle.unresolved();
        for shown in cycle.provinces().values() {
            out.report.note(shown.reasons.iter().cloned());
            out.report.wrapped += usize::from(shown.wrapped);
        }
        cycle.provinces()
    } else {
        out.report.note(cycle.withheld().iter().cloned());
        &empty
    };
    cache.hour(world.frame());
    let provinces: BTreeSet<u32> = entries
        .keys()
        .copied()
        .chain(
            shown
                .iter()
                .filter(|(_, shown)| shown.withheld || shown.offer.is_some())
                .map(|(province, _)| *province),
        )
        .collect();
    for province in provinces {
        let entry = entries.get(&province);
        let bound = entry.and_then(|entry| {
            if entry.incoming_count != 0 {
                entry.incoming.first().map(|bound| (bound, true))
            } else if entry.own_count != 0 {
                entry.own.first().map(|bound| (bound, false))
            } else {
                None
            }
        });
        let icon = match bound {
            Some((bound, incoming)) => raid_icon(
                assets,
                world,
                ours,
                province,
                bound,
                incoming,
                &mut out.report,
            ),
            None => match shown.get(&province) {
                Some(shown) if shown.withheld => {
                    out.report.withheld += 1;
                    continue;
                }
                Some(shown) => cached_offer_icon(layer, cache, province, shown, &mut out.report),
                None => continue,
            },
        };
        if let Some(icon) = icon {
            out.icons.push(icon);
        }
    }
    out.report.icons = out.icons.len();
    out
}

pub fn target_country(world: &World<'_>, scope: &RaidScope) -> Result<Option<u32>, String> {
    content_reader_raids::raids::target::country(world.frame(), scope)?
        .map(|raw| {
            world
                .slot(raw)
                .ok_or_else(|| format!("a raid target's country {raw} has no CountryLinks row"))
        })
        .transpose()
}

fn membership<'a>(
    assets: &RaidAssets,
    world: &World<'a>,
    ours: u32,
    report: &mut RaidReport,
) -> Result<BTreeMap<u32, Entry<'a>>, String> {
    let frame = world.frame();
    let complete = |error: io::Error| format!("raid icons are withheld: {error}");
    let parents = frame
        .complete::<tables::RaidCountries>()
        .map_err(complete)?;
    let raids = frame.complete::<tables::Raids>().map_err(complete)?;
    let targets: BTreeMap<(u32, u32), &replay_data::rows::RaidTargetRow> = frame
        .complete::<tables::RaidTargets>()
        .map_err(complete)?
        .iter()
        .map(|row| ((row.id, row.reference_type), row))
        .collect();
    let mut by_parent: BTreeMap<u32, Vec<&RaidRow>> = BTreeMap::new();
    for raid in raids {
        by_parent.entry(raid.parent_slot).or_default().push(raid);
    }
    let mut entries: BTreeMap<u32, Entry<'a>> = BTreeMap::new();
    let mut slots: Vec<u32> = parents.iter().map(|parent| parent.canonical_slot).collect();
    slots.sort_unstable();
    for slot in slots {
        let Some(raids) = by_parent.get_mut(&slot) else {
            continue;
        };
        raids.sort_by_key(|raid| (raid.phase == 5, raid.ordinal));
        for raid in raids.iter() {
            let target = targets
                .get(&(raid.id, raid.reference_type))
                .ok_or("a raid has no recorded target")?;
            let own = slot == ours;
            let aimed = match target_country(world, &target.scope) {
                Ok(country) => country == Some(ours),
                Err(_) if own => false,
                Err(reason) => {
                    report.withheld += 1;
                    report.note([reason]);
                    continue;
                }
            };
            if !own && !aimed {
                continue;
            }
            let ended = raid.phase == 5;
            let shown = raid
                .show_for
                .iter()
                .any(|&raw| world.slot(raw) == Some(ours));
            let admitted = (!ended || (shown && raid.outcome != 5))
                && (own || (aimed && target.detected != 0) || ended);
            if !admitted {
                continue;
            }
            let Some(province) = assets.province(&target.scope) else {
                report.unresolved += 1;
                continue;
            };
            let entry = entries.entry(province).or_default();
            let bound = Bound {
                raid,
                scope: &target.scope,
                raid_type: target.raid_type,
            };
            let count = if own {
                entry.own.push(bound);
                &mut entry.own_count
            } else {
                entry.incoming.push(bound);
                &mut entry.incoming_count
            };
            *count = count.wrapping_add(1);
            if *count == 0 {
                report.wrapped += 1;
            }
        }
    }
    Ok(entries)
}

pub fn category_admits(assets: &RaidAssets, world: &World<'_>, ours: u32, kind: &RaidType) -> Eval {
    let Some(name) = kind.category.as_deref() else {
        return Eval::unknown(format!("raid type {} has no category", kind.name));
    };
    let Some(category) = assets.catalog.categories.get(name) else {
        return Eval::True;
    };
    if category.free_targeting {
        return Eval::True;
    }
    let scopes = Scopes::of(Scope::Country(ours));
    world
        .evaluate(&category.visible, scopes)
        .and(|| world.evaluate(&category.available, scopes))
}

fn raid_icon(
    assets: &RaidAssets,
    world: &World<'_>,
    ours: u32,
    group: u32,
    bound: &Bound<'_>,
    incoming: bool,
    report: &mut RaidReport,
) -> Option<RaidIcon> {
    let kind = assets.raid_type(world, bound.raid_type);
    if !incoming {
        let admitted = match &kind {
            Ok(TypeToken::Type(index)) => {
                category_admits(assets, world, ours, &assets.catalog.types[*index])
            }
            other => Eval::unknown(unusable_type(assets, other, "own")),
        };
        match admitted {
            Eval::True => {}
            Eval::False => return None,
            Eval::Unknown(reasons) => {
                report.withheld += 1;
                report.note(reasons.into_iter().map(|why| why.to_string()));
                return None;
            }
        }
    }
    let province = match assets.anchor_province(world, bound.scope) {
        Ok(Some(province)) => province,
        Ok(None) => {
            report.unanchored += 1;
            return None;
        }
        Err(reason) => {
            report.withheld += 1;
            report.note([reason]);
            return None;
        }
    };
    let template = template_name(assets, world, bound.scope, report);
    let glyph: Arc<str> = match &kind {
        Ok(TypeToken::Type(index)) => assets.glyph_key(&assets.catalog.types[*index], template),
        other => {
            report.unknown_glyphs += 1;
            report.note([unusable_type(assets, other, "incoming")]);
            UNKNOWN_GLYPH.to_owned()
        }
    }
    .into();
    report.glyph(assets, &glyph);
    if incoming {
        report.incoming += 1;
    } else {
        report.own += 1;
    }
    Some(RaidIcon {
        group,
        province,
        frame: if incoming { 4 } else { 2 } + u32::from(bound.raid.phase == 5),
        enabled: true,
        glyph: Some(glyph),
        plus: false,
        tooltip: Some(tooltips::Key::Raid {
            id: bound.raid.id,
            reference_type: bound.raid.reference_type,
            incoming,
        }),
    })
}

fn unusable_type(assets: &RaidAssets, kind: &Result<TypeToken, String>, raid: &str) -> String {
    match kind {
        Ok(TypeToken::Refused(at)) => {
            let kind = &assets.catalog.uncertain[*at];
            format!(
                "{raid} raid type {} cannot be used: {}",
                kind.name, kind.reason
            )
        }
        Ok(TypeToken::Type(_)) => format!("an {raid} raid's type is usable"),
        Err(reason) => reason.clone(),
    }
}

fn template_name<'a>(
    assets: &'a RaidAssets,
    world: &World<'_>,
    scope: &RaidScope,
    report: &mut RaidReport,
) -> Option<&'a str> {
    if scope.building_type == 0 {
        return None;
    }
    match assets.building(world, scope.building_type) {
        Ok(building) => Some(building.name.as_str()),
        Err(reason) => {
            report.note([reason]);
            None
        }
    }
}

fn cached_offer_icon(
    layer: &HourLayer<'_, '_>,
    cache: &mut IconCache,
    province: u32,
    shown: &Arc<cycle::Shown>,
    report: &mut RaidReport,
) -> Option<RaidIcon> {
    let offer = shown.offer.as_ref()?;
    let scope = &offer.anchor;
    let order = if scope.building_type != 0 && scope.building_flags & 2 != 0 {
        layer.world.state_order(scope.building_location).map(|_| ())
    } else {
        Ok(())
    };
    if let Some(held) = cache.icons.get(&province)
        && Arc::ptr_eq(&held.shown, shown)
        && held.order == order
    {
        report.absorb(&held.report);
        return held.icon.clone();
    }
    let mut laid = RaidReport::default();
    let icon = offer_icon(layer, offer, &mut laid);
    report.absorb(&laid);
    cache.icons.insert(
        province,
        CachedIcon {
            shown: Arc::clone(shown),
            order,
            icon: icon.clone(),
            report: laid,
        },
    );
    icon
}

fn offer_icon(
    layer: &HourLayer<'_, '_>,
    offer: &offers::Province,
    report: &mut RaidReport,
) -> Option<RaidIcon> {
    let assets = layer.assets;
    let province = match assets.anchor_province(layer.world, &offer.anchor) {
        Ok(Some(province)) => province,
        Ok(None) => {
            report.unanchored += 1;
            return None;
        }
        Err(reason) => {
            report.withheld += 1;
            report.note([reason]);
            return None;
        }
    };
    let mut keys = offer
        .pairs
        .iter()
        .map(|&(kind, token)| layer.glyph(kind, token, report));
    let glyph = keys.next().map(|first| {
        if keys.all(|key| key == first) {
            first
        } else {
            OTHER_GLYPH.into()
        }
    });
    if let Some(glyph) = &glyph {
        report.glyph(assets, glyph);
    }
    let enabled = match &offer.launchable {
        Eval::True => true,
        Eval::False => false,
        Eval::Unknown(reasons) => {
            report.unverified_tint += 1;
            report.note(reasons.iter().map(|why| why.to_string()));
            false
        }
    };
    report.offers += 1;
    Some(RaidIcon {
        group: province,
        province,
        frame: 0,
        enabled,
        glyph,
        plus: offer.pairs.len() != 1,
        tooltip: Some(tooltips::Key::Offer(offer.targets.clone().into())),
    })
}

pub struct RaidInputs<'a> {
    pub anchor: &'a dyn Fn(u32) -> Option<[f32; 3]>,
    pub camera: Camera,
    pub viewport: [u32; 2],
    pub ui_scale: f32,
    pub reach: Option<RaidReach>,
}

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

fn made_once<T>(cell: &std::sync::OnceLock<T>, make: impl FnOnce() -> T) -> &T {
    if let Some(made) = cell.get() {
        return made;
    }
    let _ = cell.set(make());
    cell.get().expect("set above or by another caller")
}
