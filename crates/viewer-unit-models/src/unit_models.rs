use content_reader_mesh::mesh::MeshProducts as _;
use viewer_unit_history::unit_history::MESH_FORWARD_OFFSET;

use std::cell::{OnceCell, RefCell};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::RwLock;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
use viewer_runtime::debug_fields;
use viewer_unit_reads::aircraft_model;
use viewer_unit_reads::model_admission::Gates;
use viewer_gpu::province_admission::province_boxes;
use viewer_gpu::province_map::HeightField;
use viewer_gpu::province_map::ProvinceMap;
use viewer_foundation::movement_timing::Etas;
use viewer_runtime::profiling::BuilderSpans;
use dashboard_text::table_access::TokenNames;

use content_reader_layers::ResolvedLayers;
use content_reader_defines::defines::Defines;
use content_reader_map::map::MapData;
use content_reader_mesh::mesh;
use content_reader_key_names::terrain::DefaultMap;
use content_reader_image::native::Decode;
use content_reader_localisation::localisation::Localisation;
use replay_data::{
    rows::DivisionModelInputRow, rows::DivisionStatusRow, rows::DivisionTemplateIconRow,
    rows::TableId, rows::tables,
};
use replay_frame::frame::{DecodedFrame, SharedView, ViewIdentity, view_reads};

mod ships;
mod retained_bridge;
use model_selection::profile::{Stage, Stages};

pub use ships::{Force, ForcePart, ResolvedHull, ResolvedRoster, ShipGround, ShipPlaces, naval_differ};

use crate::{
    camouflage,
    equipment_graphics::{self, bridge::{self, CensusRead}},
    model_labels,
    model_multi_fallback,
    model_selection::{self, Definition, SelectedPair, SubUnit, Template},
};
use content_reader_equipment_graphics::equipment_graphics::{
    context,
    countries::{self as model_countries, ModelCountry},
};
use replay_frame::frame::kept;
use viewer_models::{ModelField, ModelInstance, ModelLibrary, hash_rotation};
use viewer_unit_history::unit_history::{Field, Kind, NavalSources, Target, Targets};
use viewer_unit_reads::counter_move;
use viewer_foundation::{
    perspective::{Sight, Vision},
    tables::{core, rows},
    unit_rules::STANDSTILL,
};
use viewer_runtime::unit_handle::Unit;

pub const ENTITY_SUFFIX: &str = "_entity";

#[derive(Clone, Copy, Debug)]
pub enum ModelStatus<'a> {
    Status(&'a DivisionStatusRow),
    Witness(&'a DivisionModelInputRow),
}

impl<'a> From<&'a DivisionStatusRow> for ModelStatus<'a> {
    fn from(row: &'a DivisionStatusRow) -> Self {
        Self::Status(row)
    }
}

impl<'a> ModelStatus<'a> {
    pub fn division(self) -> u64 {
        match self {
            Self::Status(row) => row.division,
            Self::Witness(row) => row.division,
        }
    }

    pub fn model_number(self) -> Option<i32> {
        match self {
            Self::Status(row) => Some(row.model_number),
            Self::Witness(row) => row.model_number,
        }
    }

    pub fn exercising(self) -> Option<bool> {
        match self {
            Self::Status(row) => Some(row.exercising != 0),
            Self::Witness(row) => row.exercising.map(|exercising| exercising != 0),
        }
    }

}

fn owned(hour: &Hour<'_>, frame: &DecodedFrame, ours: u32) -> io::Result<BTreeSet<u32>> {
    Ok(core::<tables::Provinces>(frame)?
        .iter()
        .filter(|row| hour.country.links.get(&row.owner) == Some(&ours))
        .map(|row| row.id)
        .collect())
}

#[derive(Clone, Copy)]
enum Statuses<'a> {
    Status(&'a [DivisionStatusRow]),
    Witness(&'a [DivisionModelInputRow]),
}

impl Default for Statuses<'_> {
    fn default() -> Self {
        Self::Status(&[])
    }
}

impl<'a> Statuses<'a> {
    fn read(frame: &'a DecodedFrame) -> io::Result<Self> {
        Ok(
            if frame.recorded(replay_data::rows::TableId::DivisionStatus) {
                Self::Status(rows::<tables::DivisionStatus>(frame)?.rows())
            } else {
                Self::Witness(rows::<tables::DivisionModelInputs>(frame)?.rows())
            },
        )
    }

    fn get(self, division: u64) -> Option<ModelStatus<'a>> {
        match self {
            Self::Status(rows) => rows
                .binary_search_by_key(&division, |row| row.division)
                .ok()
                .map(|at| ModelStatus::Status(&rows[at])),
            Self::Witness(rows) => rows
                .binary_search_by_key(&division, |row| row.division)
                .ok()
                .map(|at| ModelStatus::Witness(&rows[at])),
        }
    }
}

pub const TRANSPORT_KEY: &str = "transport";

pub const INFANTRY_KEY: &str = "infantry";

pub const PLACEHOLDER_ENTITY: &str = "test_object_entity";

pub const RIG_RADIUS: f32 = 0.5;

pub const UNIT_TILE: f32 = 128.0;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn entity_refusal(roster: &mesh::Roster, name: &str) -> Option<String> {
    let (reached, path) = roster.unreadable_attachment(name)?;
    Some(format!(
        "entity {name} reaches {reached}, whose declaration is undecided after unreadable descriptor {path}"
    ))
}


pub fn entity_name(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        format!("{key}{ENTITY_SUFFIX}")
    } else {
        format!("{prefix}_{key}{ENTITY_SUFFIX}")
    }
}

pub fn entity_keys<'a>(key: &'a str, sprite: Option<&'a str>) -> Vec<&'a str> {
    let mut keys = vec![key];
    if let Some(sprite) = sprite
        && sprite != key
    {
        keys.push(sprite);
    }
    keys
}

pub fn entity_prefixes<'a>(
    cosmetic: Option<&'a str>,
    tag: Option<&'a str>,
    culture: Option<&'a str>,
) -> Vec<&'a str> {
    let mut prefixes = Vec::with_capacity(4);
    for prefix in [cosmetic, tag, culture].into_iter().flatten() {
        if !prefix.is_empty() && !prefixes.contains(&prefix) {
            prefixes.push(prefix);
        }
    }
    prefixes.push("");
    prefixes
}

pub fn candidates(
    cosmetic: Option<&str>,
    tag: Option<&str>,
    culture: Option<&str>,
    key: &str,
    sprite: Option<&str>,
) -> Vec<(Level, String)> {
    let prefixes = entity_prefixes(cosmetic, tag, culture);
    let mut out = Vec::new();
    for key in entity_keys(key, sprite) {
        if cosmetic == Some("") {
            out.push((Level::Cosmetic, format!("_{key}{ENTITY_SUFFIX}")));
        }
        for prefix in &prefixes {
            if prefix.is_empty() && culture == Some("") && cosmetic != Some("") {
                out.push((Level::Culture, format!("_{key}{ENTITY_SUFFIX}")));
            }
            let level = if prefix.is_empty() {
                Level::Bare
            } else if Some(*prefix) == cosmetic {
                Level::Cosmetic
            } else if Some(*prefix) == tag {
                Level::Tag
            } else {
                Level::Culture
            };
            out.push((level, entity_name(prefix, key)));
        }
    }
    out
}

fn model_owner(roster: u32, expeditionary: u32, transport: bool) -> u32 {
    if transport && (expeditionary as i32) > 0 {
        expeditionary
    } else {
        roster
    }
}

fn resolve_prefixes(
    cosmetic: &str,
    tag: Option<&str>,
    culture: Option<&str>,
    key: &str,
    sprite: Option<&str>,
    mut lookup: impl FnMut(&str) -> bool,
    placeholder: bool,
) -> Option<(Level, String)> {
    if let (Some(tag), Some(culture)) = (tag, culture) {
        for (level, name) in candidates(Some(cosmetic), Some(tag), Some(culture), key, sprite) {
            if lookup(&name) {
                return Some((level, name));
            }
        }
        return (placeholder && lookup(PLACEHOLDER_ENTITY))
            .then(|| (Level::Placeholder, PLACEHOLDER_ENTITY.to_owned()));
    }
    let name = format!("{cosmetic}_{key}{ENTITY_SUFFIX}");
    if lookup(&name) {
        return Some((Level::Cosmetic, name));
    }
    let tag = tag?;
    let name = entity_name(tag, key);
    lookup(&name).then_some((Level::Tag, name))
}

fn resolve_model_numbered(
    country: &ModelCountry,
    key: &str,
    sprite: Option<&str>,
    number: i8,
    mut lookup: impl FnMut(&str) -> bool,
) -> Option<(Level, String)> {
    let culture = country.graphical_culture.as_deref();
    resolve_prefixes(
        &country.cosmetic,
        Some(&country.tag),
        culture,
        &format!("{key}_{number}"),
        None,
        &mut lookup,
        false,
    )
    .or_else(|| {
        resolve_prefixes(
            &country.cosmetic,
            Some(&country.tag),
            culture,
            key,
            sprite,
            lookup,
            true,
        )
    })
}

fn resolve_transport(
    country: &ModelCountry,
    lookup: impl FnMut(&str) -> bool,
) -> Option<(Level, String)> {
    resolve_prefixes(
        &country.cosmetic,
        country.original_tag.as_deref(),
        country.original_graphical_culture.as_deref(),
        TRANSPORT_KEY,
        None,
        lookup,
        true,
    )
}

fn resolve_infantry(
    country: &ModelCountry,
    sprite: Option<&str>,
    lookup: impl FnMut(&str) -> bool,
) -> Option<(Level, String)> {
    resolve_prefixes(
        &country.cosmetic,
        Some(&country.tag),
        country.graphical_culture.as_deref(),
        INFANTRY_KEY,
        sprite,
        lookup,
        true,
    )
}

#[derive(Clone, Debug, PartialEq)]
pub struct Figure {
    pub entity: String,
    pub level: Level,
    pub mesh: usize,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Template,
    Variant,
    Graphics,
    Conventional,
    Cosmetic,
    Tag,
    Culture,
    #[default]
    Bare,
    Placeholder,
}


#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelReport {
    pub stacks: usize,
    pub ship_hulls: usize,
    pub by_level: BTreeMap<Level, usize>,
    pub by_entity: BTreeMap<String, usize>,
    pub sprite_key: usize,
    pub transports: usize,
    pub moving: usize,
    pub untemplated: usize,
    pub unknown_identity: usize,
    pub selection_errors: BTreeMap<String, usize>,
    pub disabled: Option<String>,
    pub unanchored: usize,
    pub unseen: usize,
    pub attached: usize,
    pub rigs: usize,
    pub undecided_mission_formations: usize,
    pub support_locator_rng: usize,
    pub bake_warnings: Vec<String>,
}

impl ModelReport {
    pub fn error(&mut self, reason: impl Into<String>) {
        *self.selection_errors.entry(reason.into()).or_default() += 1;
    }

    pub fn absorb(&mut self, other: &Self) {
        let counts = [(&mut self.stacks, other.stacks), (&mut self.ship_hulls, other.ship_hulls), (&mut self.sprite_key, other.sprite_key), (&mut self.transports, other.transports), (&mut self.moving, other.moving), (&mut self.untemplated, other.untemplated), (&mut self.unknown_identity, other.unknown_identity), (&mut self.unanchored, other.unanchored), (&mut self.unseen, other.unseen), (&mut self.attached, other.attached), (&mut self.rigs, other.rigs), (&mut self.undecided_mission_formations, other.undecided_mission_formations), (&mut self.support_locator_rng, other.support_locator_rng)];
        for (mine, theirs) in counts {
            *mine += theirs;
        }
        for (level, count) in &other.by_level {
            *self.by_level.entry(*level).or_default() += count;
        }
        for (name, count) in &other.by_entity {
            *self.by_entity.entry(name.clone()).or_default() += count;
        }
        for (reason, count) in &other.selection_errors {
            *self.selection_errors.entry(reason.clone()).or_default() += count;
        }
    }

    pub fn warnings(&self) -> Vec<String> {
        self.selection_errors
            .iter()
            .map(|(reason, count)| format!("Unit models: no model for {count} stacks: {reason}."))
            .chain((self.undecided_mission_formations > 0).then(|| {
                format!(
                    "Unit models: {} ship formations not drawn: their task force's mission skip                      operands (counter inputs or current region) are not recorded.",
                    self.undecided_mission_formations
                )
            }))
            .chain((self.support_locator_rng > 0).then(|| format!(
                "Unit models: naval_support_locator_rng: {} formations use viewer support positions; game support locators use an unrecorded performance-counter seed.",
                self.support_locator_rng
            )))
            .chain(
                self.bake_warnings
                    .iter()
                    .map(|line| format!("Unit models: {line}")),
            )
            .collect()
    }
}

pub struct UnitModelAssets {
    library: RwLock<ModelLibrary>,
    field_identity: Arc<()>,
    bakes: Mutex<BTreeMap<String, Arc<Mutex<bool>>>>,
    bake_warnings: Mutex<BTreeSet<String>>,
    conventional: retained_bridge::RetainedBridge,
    selections: KeptChoices,
    roster: Arc<mesh::Roster>,
    layers: ResolvedLayers,
    pub sub_units: model_selection::Content,
    pub gates: Gates,
    pub bounds: Arc<BTreeMap<u32, [f32; 4]>>,
    graphics: content_reader_equipment_graphics::equipment_graphics::Content,
    geography: context::Content,
    camouflage: camouflage::Content,
    registered: HashSet<String>,
    ships: ships::ShipAssets,
    dynamic_tags: Result<BTreeSet<String>, String>,
    disabled: Option<String>,
}
debug_fields!(UnitModelAssets(this) {
    library, bakes, bake_warnings, roster, sub_units, gates, bounds, graphics, geography, camouflage,
    ships, disabled
} {
    registered => viewer_runtime::debug_fields::sorted_set(&this.registered)
});

pub struct PrepareInputs<'a> {
    pub frame: &'a DecodedFrame,
    pub limits: equipment_graphics::Limits<'a>,
    pub vision: &'a Vision,
    pub etas: &'a Etas,
    pub anchor: &'a dyn Fn(u32, u32) -> Option<[f32; 3]>,
    pub world: Option<&'a counter_move::RouteWorld<'a>>,
    pub terrain: &'a BTreeMap<u32, String>,
    pub routes: Option<&'a counter_move::RouteMemo>,
    pub ships: &'a ShipPlaces<'a>,
    pub localisation: &'a Localisation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoadStack {
    Shown,
    Recorded,
}

pub const LAYER_OFF: &str = "model layer: unit models";

impl UnitModelAssets {
    #[expect(clippy::too_many_arguments, reason = "the layers and decoder, the defines, the map and default map, and the warnings and refusals")]
    pub fn load(
        layers: &ResolvedLayers,
        decode: Decode,
        defines: &Defines,
        recorded: Option<&Defines>,
        map: &MapData,
        default_map: &DefaultMap,
        warnings: &mut Vec<String>,
        refused: impl FnMut(String),
    ) -> Self {
        let below = layers.without_overlays();
        Self::load_with(
            layers,
            defines,
            recorded,
            below.is_some(),
            warnings,
            refused,
            |stack, defines, notes| {
                let layers = match (stack, &below) {
                    (LoadStack::Recorded, Some(below)) => below,
                    _ => layers,
                };
                Self::try_load(layers, decode, defines, map, default_map, notes)
            },
        )
    }

    fn load_with(
        layers: &ResolvedLayers,
        defines: &Defines,
        recorded: Option<&Defines>,
        below: bool,
        warnings: &mut Vec<String>,
        mut refused: impl FnMut(String),
        mut try_load: impl FnMut(
            LoadStack,
            &Defines,
            &mut Vec<content_reader_parser::Diagnostic>,
        ) -> io::Result<Self>,
    ) -> Self {
        let mut notes = Vec::new();
        let mut loaded = try_load(LoadStack::Shown, defines, &mut notes);
        if let (Err(error), Some(recorded)) = (&loaded, recorded) {
            let why = format!("graphics defines for the unit models ({error})");
            let mut recorded_notes = Vec::new();
            if let Ok(assets) = try_load(LoadStack::Shown, recorded, &mut recorded_notes) {
                refused(why);
                loaded = Ok(assets);
                notes = recorded_notes;
            }
        }
        if let (Err(error), true) = (&loaded, below) {
            let why = format!("unit models ({error})");
            let mut recorded_notes = Vec::new();
            if let Ok(assets) = try_load(
                LoadStack::Recorded,
                recorded.unwrap_or(defines),
                &mut recorded_notes,
            ) {
                refused(why);
                loaded = Ok(assets);
                notes = recorded_notes;
            }
        }
        if loaded.is_err() {
            layers.note_refused(LAYER_OFF.into());
        }
        warnings.extend(notes.into_iter().map(|(_, message)| message));
        Self::scoped(loaded, warnings)
    }

    fn scoped(loaded: io::Result<Self>, warnings: &mut Vec<String>) -> Self {
        loaded.unwrap_or_else(|error| {
            warnings.push(format!(
                "3D unit models are off for this content ({error}); the rest of the map draws."
            ));
            Self::off(error.to_string())
        })
    }

    fn off(reason: String) -> Self {
        Self {
            library: RwLock::new(ModelLibrary::default()),
            field_identity: Arc::new(()),
            bakes: Mutex::new(BTreeMap::new()),
            bake_warnings: Mutex::default(),
            conventional: retained_bridge::RetainedBridge::default(),
            selections: KeptChoices::default(),
            roster: Arc::new(mesh::Roster::default()),
            layers: ResolvedLayers::default(),
            sub_units: model_selection::Content::default(),
            gates: Gates::OFF,
            bounds: Default::default(),
            graphics: content_reader_equipment_graphics::equipment_graphics::Content::default(),
            geography: context::Content::default(),
            camouflage: camouflage::Content::default(),
            registered: HashSet::new(),
            ships: ships::ShipAssets::default(),
            dynamic_tags: Err(reason.clone()),
            disabled: Some(reason),
        }
    }

    pub fn try_load(
        layers: &ResolvedLayers,
        decode: Decode,
        defines: &Defines,
        map: &MapData,
        default_map: &DefaultMap,
        warnings: &mut Vec<content_reader_parser::Diagnostic>,
    ) -> io::Result<Self> {
        let roster = layers.descriptor_roster()?;
        if !roster.unreadable.is_empty() {
            warnings.push((
                content_reader_parser::DiagnosticKind::ParseFailure,
                format!(
                    "Unit model descriptors that do not parse even when closed are skipped: {}",
                    roster.unreadable.join(", ")
                ),
            ));
        }
        if !roster.logged.is_empty() {
            warnings.push((
                content_reader_parser::DiagnosticKind::Logged,
                format!(
                    "Unit model entity registration diagnostics: {}",
                    roster.logged.join("; ")
                ),
            ));
        }
        let sub_units = model_selection::Content::load(layers)?;
        if !sub_units.dropped.is_empty() {
            warnings.push((
                content_reader_parser::DiagnosticKind::Warning,
                format!(
                    "Sub-unit equipment needs the engine drops: {}",
                    sub_units.dropped.join("; ")
                ),
            ));
        }
        let mut name_warnings = Vec::new();
        let names = content_reader_equipment_graphics::equipment_graphics::Names::read(layers, default_map, &mut name_warnings)?;
        warnings.extend(
            name_warnings
                .into_iter()
                .map(|message| (content_reader_parser::DiagnosticKind::Warning, message)),
        );
        let graphics = content_reader_equipment_graphics::equipment_graphics::Content::load(layers, &names)?;
        let geography = context::Content::load(layers, default_map, &map.definitions, defines)?;
        let camouflage = camouflage::Content::load(layers, defines)?;
        let mut ship_warnings = Vec::new();
        let ships = ships::ShipAssets::load(layers, &roster, &mut ship_warnings)?;
        warnings.extend(ship_warnings);
        Ok(Self {
            library: RwLock::new(ModelLibrary::empty(decode)),
            field_identity: Arc::new(()),
            bakes: Mutex::new(BTreeMap::new()),
            bake_warnings: Mutex::default(),
            conventional: retained_bridge::RetainedBridge::default(),
            selections: KeptChoices::default(),
            sub_units,
            gates: Gates::read(defines)?,
            bounds: Arc::new(province_boxes(&ProvinceMap {
                width: map.width,
                height: map.height,
                ids: &map.province_ids,
            })),
            graphics,
            geography,
            camouflage,
            registered: roster
                .entity_names()
                .chain(roster.unreadable_names.keys().map(String::as_str))
                .map(str::to_owned)
                .collect(),
            ships,
            dynamic_tags: content_reader_key_names::country_tags::read_definitions(layers)
                .map(|tags| {
                    tags.into_iter()
                        .filter(|(_, tag)| tag.dynamic)
                        .map(|(name, _)| name)
                        .collect()
                })
                .map_err(|error| format!("common/country_tags: {error}")),
            disabled: None,
            roster,
            layers: layers.clone(),
        })
    }

    pub fn refusals(&self) -> Vec<(Option<String>, String)> {
        let mut refusals = Vec::new();
        refusals.extend(
            self.camouflage
                .refusals()
                .map(|why| (Some("common/acclimatation.txt".to_owned()), why)),
        );
        if let Err(why) = &self.dynamic_tags {
            refusals.push((
                None,
                format!("fallback model labels for dynamic creators refuse: {why}"),
            ));
        }
        refusals
    }

    pub fn bake_warnings(&self) -> Vec<String> {
        self.bake_warnings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    pub fn declares(&self, name: &str) -> bool {
        self.registered(name)
    }

    pub fn infantry_entity(
        &self,
        frame: &DecodedFrame,
        ours: u32,
    ) -> Result<(Level, String), String> {
        if let Some(reason) = &self.disabled {
            return Err(reason.clone());
        }
        let countries = model_countries::read(frame).map_err(|error| error.to_string())?;
        let country = countries.get(&ours).ok_or_else(|| {
            format!("the viewing country (slot {ours}) has no recorded tag or appearance this hour")
        })?;
        let sprite = self
            .sub_units
            .subunit(INFANTRY_KEY)
            .map(|unit| unit.sprite.as_str());
        resolve_infantry(country, sprite, |name| self.registered(name)).ok_or_else(|| {
            if country.graphical_culture.is_none() {
                format!(
                    "{}'s graphical culture is unreadable this hour",
                    country.tag
                )
            } else {
                format!(
                    "no {INFANTRY_KEY} entity resolves for {} and {PLACEHOLDER_ENTITY} is not registered",
                    country.tag
                )
            }
        })
    }

    pub fn infantry_figure(&self, frame: &DecodedFrame, ours: u32) -> Result<Figure, String> {
        let (level, entity) = self.infantry_entity(frame, ours)?;
        if let Some(reason) = self.refusal(&entity) {
            return Err(reason);
        }
        let mesh = self
            .bake_slot(&entity)
            .ok_or_else(|| format!("registered entity {entity} did not bake"))?;
        let height = self
            .library()
            .meshes
            .get(mesh)
            .map_or(0.0, |mesh| mesh.height);
        Ok(Figure {
            entity,
            level,
            mesh,
            height,
        })
    }

    pub fn aircraft_variant_model(
        &self,
        frame: &DecodedFrame,
        limits: equipment_graphics::Limits<'_>,
        variant: u64,
        country: u32,
    ) -> Result<Option<String>, String> {
        if let Some(reason) = &self.disabled {
            return Err(reason.clone());
        }
        let definitions = frame
            .complete::<tables::VariantDefinitions>()
            .map_err(|error| error.to_string())?;
        let row = definitions
            .iter()
            .find(|row| row.id == variant)
            .ok_or_else(|| format!("equipment variant {variant} has no definition"))?;
        let saved = std::str::from_utf8(&row.explicit_entity)
            .map_err(|_| format!("equipment variant {variant} saved model is not text"))?;
        if !saved.is_empty() {
            return Ok(Some(saved.to_owned()));
        }
        let tokens = TokenNames::new(frame).map_err(|error| error.to_string())?;
        let key = self.sub_units.equipment_key(&tokens, row)?;
        let subunits = self
            .sub_units
            .archetype_subunits(key)
            .map_err(|error| error.to_string())?;
        let Some(&subunit) = subunits.first() else {
            return Ok(None);
        };
        let terrain = BTreeMap::new();
        static NO_LABELS: std::sync::LazyLock<Localisation> =
            std::sync::LazyLock::new(Localisation::default);
        let hour = Hour::read(self, frame, &terrain, limits, &NO_LABELS)
            .map_err(|error| error.to_string())?;
        let pair = SelectedPair {
            variant: row,
            subunit,
            identity: Definition::Global(0),
        };
        Ok(graphics_model(self, &hour, pair, None, country)?.map(|(_, name)| name))
    }

    pub fn aircraft_geography(
        &self,
        frame: &DecodedFrame,
    ) -> Result<context::Prepared, String> {
        replay_frame::frame::view_reads::unrecorded(|| {
            context::Prepared::read(frame, &self.geography)
        })
    }

    pub fn census(&self, frame: &DecodedFrame) -> Result<Arc<bridge::Retained>, String> {
        self.conventional.read(frame, &self.sub_units)
    }

    pub fn aircraft_entity(
        &self,
        frame: &DecodedFrame,
        limits: equipment_graphics::Limits<'_>,
        variant: u64,
        raw_country: u32,
    ) -> Result<Option<String>, String> {
        if let Some(reason) = &self.disabled {
            return Err(reason.clone());
        }
        kept::note_row(TableId::VariantDefinitions, u128::from(variant), u64::MAX);
        let definitions = frame
            .complete::<tables::VariantDefinitions>()
            .map_err(|error| error.to_string())?;
        let row = definitions
            .iter()
            .find(|row| row.id == variant)
            .ok_or_else(|| format!("equipment variant {variant} has no definition"))?;
        kept::note_row(
            TableId::LexicalTokens,
            u128::from(row.equipment_token),
            replay_data::rows::mask::<replay_data::rows::LexicalTokenRow>(&["literal"]),
        );
        kept::note_table(
            TableId::CountryLinks,
            replay_data::rows::mask::<replay_data::rows::CountryLinkRow>(&["canonical_slot"]),
        );
        kept::note_table(
            TableId::CountryAppearance,
            replay_data::rows::mask::<replay_data::rows::CountryAppearanceRow>(&[
                "cosmetic_tag",
                "graphical_culture",
            ]),
        );
        let [features] = frame
            .complete::<tables::SimulationFeatures>()
            .map_err(|error| error.to_string())?
        else {
            return Err("SimulationFeatures must have one complete row".into());
        };
        fn text(bytes: &[u8]) -> Result<&str, String> {
            content_reader_parser::parser::text(bytes).map_err(|error| error.to_string())
        }
        let key = || {
            let tokens = TokenNames::new(frame).map_err(|error| error.to_string())?;
            self.sub_units.equipment_key(&tokens, row)
        };
        let slot = || {
            let links = frame
                .complete::<tables::CountryLinks>()
                .map_err(|error| error.to_string())?;
            links
                .iter()
                .find(|link| link.id == raw_country)
                .map(|link| link.canonical_slot)
                .ok_or_else(|| format!("aircraft country {raw_country} has no CountryLinks row"))
        };
        let appearance = || {
            let slot = slot()?;
            let links = frame
                .complete::<tables::CountryLinks>()
                .map_err(|error| error.to_string())?;
            rows::<tables::CountryAppearance>(frame)
                .map_err(|error| error.to_string())?
                .rows()
                .iter()
                .find(|row| {
                    links
                        .iter()
                        .any(|link| link.id == row.country && link.canonical_slot == slot)
                })
                .ok_or_else(|| "aircraft country has no CountryAppearance row".to_owned())
        };
        let designer = || {
            kept::record::<CensusRead, _>(frame, || {
                self.aircraft_variant_model(frame, limits, variant, raw_country)
            })
            .0
        };
        let graphical = || {
            let key = key()?;
            let appearance = appearance()?;
            let culture = std::str::from_utf8(&appearance.graphical_culture)
                .map_err(|_| "aircraft graphical culture is not text")?;
            let links = frame
                .complete::<tables::CountryLinks>()
                .map_err(|error| error.to_string())?;
            let slot = slot()?;
            kept::note_table(
                TableId::PoliticalParties,
                replay_data::rows::mask::<replay_data::rows::PoliticalPartyRow>(&[
                    "is_ruling",
                    "token",
                ]),
            );
            let parties = frame
                .complete::<tables::PoliticalParties>()
                .map_err(|error| error.to_string())?;
            let ideology = parties
                .iter()
                .rfind(|party| {
                    party.is_ruling != 0
                        && links
                            .iter()
                            .any(|link| link.id == party.country && link.canonical_slot == slot)
                })
                .map(|party| std::str::from_utf8(&party.token))
                .transpose()
                .map_err(|_| "aircraft ruling ideology is not text")?;
            let database = self.sub_units.equipment_content();
            let equipment = database
                .named(key)
                .ok_or_else(|| format!("equipment {key} is not in the content"))?;
            let archetype = equipment
                .archetype
                .map(|at| text(&database.equipment[at].name))
                .transpose()?;
            let family = &database.equipment[equipment.family];
            let family_key = text(&family.name)?;
            let family = if family_key == key {
                None
            } else {
                Some((
                    family_key,
                    family
                        .archetype
                        .map(|at| text(&database.equipment[at].name))
                        .transpose()?,
                ))
            };
            let geography = self.aircraft_geography(frame)?;
            let geography = geography.context(raw_country)?;
            let capital_continent = match geography {
                Some(country) => country
                    .capital_continent
                    .as_ref()
                    .map(Option::as_deref)
                    .map_err(String::as_str),
                None => Ok(None),
            };
            self.graphics.aircraft_override(
                &content_reader_equipment_graphics::equipment_graphics::Context {
                    equipment: key,
                    archetype,
                    family,
                    subunit: None,
                    ideology,
                    culture: Some(culture),
                    country: geography.map(|country| country.country_tag.as_str()),
                    faction_tags: geography.map_or(&[], |country| country.faction_tags.as_slice()),
                    capital_continent,
                },
                &limits,
                |name| self.registered(name),
            )
        };
        let prefixed = || {
            let key = key()?;
            let database = self.sub_units.equipment_content();
            let kind = database
                .named(key)
                .ok_or_else(|| format!("equipment {key} is not in the content"))?;
            aircraft_model::prefixed(
                &|| text(&appearance()?.cosmetic_tag).map(str::to_owned),
                &|| {
                    let slot = slot()?;
                    kept::note_table(
                        TableId::Countries,
                        replay_data::rows::mask::<replay_data::rows::CountryRow>(&[
                            "canonical_slot",
                            "tag",
                        ]),
                    );
                    let countries = frame
                        .complete::<tables::Countries>()
                        .map_err(|error| error.to_string())?;
                    let country = countries
                        .iter()
                        .find(|country| country.canonical_slot == slot)
                        .ok_or_else(|| "aircraft country has no Countries row".to_owned())?;
                    text(&country.tag).map(str::to_owned)
                },
                &|index| match index {
                    0 => Ok(key.to_owned()),
                    1 => kind
                        .archetype
                        .map(|at| text(&database.equipment[at].name).map(str::to_owned))
                        .transpose()
                        .map(Option::unwrap_or_default),
                    _ => text(&kind.sprite).map(str::to_owned),
                },
                &|name| self.registered(name),
            )
        };
        aircraft_model::resolve_lazy(
            row,
            features.dlc_mask,
            &designer,
            &graphical,
            &prefixed,
            &|name| self.registered(name),
        )
    }

    fn registered(&self, name: &str) -> bool {
        self.registered.contains(name)
    }

    pub fn library(&self) -> std::sync::RwLockReadGuard<'_, ModelLibrary> {
        self.library
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(test)]
    fn ensure(&self, names: &[String]) {
        self.ensure_with_spans(names, &mut BuilderSpans::new(false));
    }

    fn ensure_with_spans(&self, names: &[String], spans: &mut BuilderSpans) {
        let missing: Vec<String> = {
            let library = spans.measure("units.ensure.scan_read_wait", |_| self.library());
            spans.measure("units.ensure.scan", |_| {
                names
                    .iter()
                    .filter(|name| {
                        library.slot(name).is_none()
                            && self.registered(name)
                            && self.refusal(name).is_none()
                    })
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            })
        };
        if missing.is_empty() {
            return;
        }
        let slots: Vec<_> = {
            let mut bakes = spans.measure("units.ensure.registry_wait", |_| {
                self.bakes
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
            });
            spans.measure("units.ensure.registry", |_| {
                missing
                    .iter()
                    .map(|name| Arc::clone(bakes.entry(name.clone()).or_default()))
                    .collect()
            })
        };
        let mut completed: Vec<_> = spans.measure("units.ensure.name_wait", |_| {
            slots
                .iter()
                .map(|slot| slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
                .collect()
        });
        let missing: Vec<_> = {
            let library = spans.measure("units.ensure.recheck_read_wait", |_| self.library());
            spans.measure("units.ensure.recheck", |_| {
                missing
                    .into_iter()
                    .zip(&completed)
                    .filter_map(|(name, complete)| {
                        (!**complete && library.slot(&name).is_none()).then_some(name)
                    })
                    .collect()
            })
        };
        if missing.is_empty() {
            return;
        }
        let mut warnings = Vec::new();
        let decode = spans
            .measure("units.ensure.decode_read_wait", |_| self.library())
            .decode();
        let (recorded, own): (Vec<String>, Vec<String>) =
            spans.measure("units.ensure.partition", |_| {
                missing
                    .iter()
                    .cloned()
                    .partition(|name| entity_refusal(&self.roster, name).is_some())
            });
        let mut failed = Vec::new();
        match spans.measure("units.ensure.load_shown", |spans| {
            ModelLibrary::load_profiled(
                &self.layers,
                decode,
                self.roster.descriptor_job().view(),
                &own,
                &mut warnings,
                spans,
            )
        }) {
            Ok(shard) => {
                let mut library = spans.measure("units.ensure.merge_shown_wait", |_| {
                    self.library
                        .write()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                });
                spans.measure("units.ensure.merge_shown", |_| library.merge(shard));
            }
            Err(error) => failed.push(format!("{} did not bake: {error}", own.join(", "))),
        }
        if !recorded.is_empty() {
            match spans.measure("units.ensure.load_recorded", |spans| {
                ModelLibrary::load_recorded_profiled(
                    &self.layers,
                    decode,
                    None,
                    &recorded,
                    |name| entity_refusal(&self.roster, name).unwrap_or_default(),
                    spans,
                )
            }) {
                Ok(shard) => {
                    let mut library = spans.measure("units.ensure.merge_recorded_wait", |_| {
                        self.library
                            .write()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                    });
                    spans.measure("units.ensure.merge_recorded", |_| library.merge(shard));
                }
                Err(error) => failed.push(format!(
                    "{} did not bake from the recording: {error}",
                    recorded.join(", ")
                )),
            }
        }
        self.bake_warnings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend(
                warnings
                    .into_iter()
                    .map(|(_, line)| line)
                    .chain(failed),
            );
        spans.measure("units.ensure.complete", |_| {
            for complete in &mut completed {
                **complete = true;
            }
        });
    }

    fn refusal(&self, name: &str) -> Option<String> {
        let reason = entity_refusal(&self.roster, name)?;
        let recorded = self.layers.recorded_descriptor_roster().ok().flatten();
        if recorded.is_some_and(|recorded| {
            entity_refusal(&recorded, name).is_none()
                && recorded.has_entity(name)
        }) {
            return None;
        }
        self.layers.note_refused(format!("model entity {name}"));
        Some(reason)
    }

    pub(crate) fn bake_slot(&self, name: &str) -> Option<usize> {
        self.bake_slot_with_spans(name, &mut BuilderSpans::new(false))
    }

    fn bake_slot_with_spans(&self, name: &str, spans: &mut BuilderSpans) -> Option<usize> {
        if !self.registered(name) || self.refusal(name).is_some() {
            return None;
        }
        if let Some(slot) = self.library().slot(name) {
            return Some(slot);
        }
        self.ensure_with_spans(&[name.to_owned()], spans);
        self.library().slot(name)
    }

    fn bake_slot_for_hour(&self, name: &str, hour: Option<&Hour<'_>>) -> Option<usize> {
        match hour {
            Some(hour) => self.bake_slot_with_spans(name, &mut hour.spans.borrow_mut()),
            None => self.bake_slot(name),
        }
    }

    pub fn prepare(&self, inputs: PrepareInputs<'_>) -> io::Result<PreparedModels> {
        self.prepare_profiled(inputs, &mut |_| {})
    }

    pub fn prepare_profiled(
        &self,
        inputs: PrepareInputs<'_>,
        lap: &mut dyn FnMut(&'static str),
    ) -> io::Result<PreparedModels> {
        self.prepare_with_spans(inputs, lap, &mut BuilderSpans::new(false))
    }

    pub fn prepare_with_spans(
        &self,
        inputs: PrepareInputs<'_>,
        lap: &mut dyn FnMut(&'static str),
        spans: &mut BuilderSpans,
    ) -> io::Result<PreparedModels> {
        self.prepare_naval_or_not(inputs, true, lap, spans)
    }

    pub fn prepare_naval_or_not(
        &self,
        inputs: PrepareInputs<'_>,
        naval: bool,
        lap: &mut dyn FnMut(&'static str),
        spans: &mut BuilderSpans,
    ) -> io::Result<PreparedModels> {
        if let Some(reason) = &self.disabled {
            return Ok(PreparedModels::disabled(reason));
        }
        let PrepareInputs {
            frame,
            limits,
            vision,
            etas,
            anchor,
            world,
            terrain,
            routes,
            ships,
            localisation,
        } = inputs;
        let country = self.selections.country();
        let mut hour = Hour::read_profiled(
            self,
            frame,
            terrain,
            limits,
            localisation,
            country.as_ref(),
            lap,
        )?;
        hour.spans = RefCell::new(spans.fork());
        hour.selection = Stages::new(spans.enabled());
        let uv_moving: BTreeSet<_> = rows::<tables::DivisionMovement>(frame)?
            .rows()
            .iter()
            .filter(|r| r.path_total > 0 && r.target == 0)
            .map(|r| r.division)
            .collect();
        let anchor_standstill = |id| anchor(id, STANDSTILL);
        let map = world.map(|world| world.map);
        let movements = match routes {
            Some(memo) => Arc::clone(
                &counter_move::DivisionRoutes::read_kept(frame, etas, map, &anchor_standstill, memo)?
                    .by_division,
            ),
            None => counter_move::DivisionRoutes::read(frame, etas, map, &anchor_standstill)?.by_division,
        };
        lap("um_routes");
        let alternates: BTreeMap<u64, u64> = rows::<tables::ArmyIconInputs>(frame)?
            .rows()
            .iter()
            .filter(|row| row.alternate_template != 0)
            .map(|row| (row.division, row.alternate_template))
            .collect();
        let mut combat_states = BTreeMap::new();
        for combat in rows::<tables::LandCombats>(frame)?.rows() {
            for id in &combat.attackers {
                combat_states.insert(*id, viewer_models::entity_animation::ATTACK);
            }
            for id in &combat.defenders {
                combat_states.insert(*id, viewer_models::entity_animation::DEFEND);
            }
        }
        let mut stacks: BTreeMap<StackKey, Stack> = BTreeMap::new();
        for division in rows::<tables::Divisions>(frame)?.rows() {
            if division.province == 0 {
                continue;
            }
            let Some(slot) = hour.country.links.get(&division.logical_owner).copied() else {
                continue;
            };
            let route = movements.get(&division.id);
            let signature = route.and_then(Route::signature);
            let stack = stacks
                .entry((division.province, slot, signature))
                .or_insert_with(|| Stack {
                    oldest: u64::MAX,
                    template: 0,
                    roster_owner: 0,
                    expeditionary_owner: 0,
                    visible: 0,
                    route: route.cloned(),
                });
            if vision.sees_land(division.id, slot, division.province) == Sight::None {
                continue;
            }
            stack.visible += 1;
            if division.id < stack.oldest {
                stack.oldest = division.id;
                stack.template = alternates
                    .get(&division.id)
                    .copied()
                    .unwrap_or(division.template);
                stack.roster_owner = division.roster_owner;
                stack.expeditionary_owner = division.expeditionary_owner;
                stack.route = route.cloned();
            }
        }

        lap("um_stacks");
        let mut report = ModelReport {
            stacks: stacks.len(),
            ..ModelReport::default()
        };
        let mut chosen = Vec::new();
        self.with_land_models(&hour, vision.ours(), &self.selections, None, &mut |_| {}, |models| {
            for ((province, owner, _), stack) in &stacks {
                if stack.visible == 0 {
                    report.unseen += 1;
                    continue;
                }
                let transport = stack
                    .route
                    .as_ref()
                    .is_some_and(|route| route.naval_transfer);
                if transport {
                    report.transports += 1;
                }
                let key = SelectionKey {
                    transport,
                    template: stack.template,
                    oldest: stack.oldest,
                    roster_owner: stack.roster_owner,
                    expeditionary_owner: stack.expeditionary_owner,
                };
                match models.choose(*province, key) {
                    StackModel::Chosen(model) => {
                        chosen.push((*province, *owner, stack, model, transport));
                    }
                    StackModel::UnknownIdentity => report.unknown_identity += 1,
                    StackModel::Untemplated => report.untemplated += 1,
                    StackModel::Withheld(reason) => report.error(reason),
                }
            }
        });
        lap("um_selection");
        spans.append(hour.spans.replace(spans.fork()));
        hour.selection.append_to(spans);
        let names: Vec<String> = chosen.iter().map(|chosen| chosen.3.entity.clone()).collect();
        self.ensure_with_spans(&names, spans);
        lap("um_ensure");
        let library = self.library();
        let clock_unavailable = counter_move::MovementFrame::clock_speed(frame).is_none();
        let mut placed = Vec::new();
        for (province, owner, stack, model, transport) in chosen {
            let slot = match self.chosen_slot(&library, &model.entity) {
                Ok(slot) => slot,
                Err(reason) => {
                    report.error(reason);
                    continue;
                }
            };
            if !self.bounds.contains_key(&province) {
                report.error(format!(
                    "province {province} has no content bounds for model admission"
                ));
                continue;
            }
            if model.sprite_keyed(self) {
                report.sprite_key += 1;
            }
            let ChosenModel {
                level,
                entity: name,
                ..
            } = model;
            let figure = land_figure(
                &LandStack {
                    province,
                    owner,
                    oldest: stack.oldest,
                    route: stack.route.as_ref(),
                    combat: combat_states.get(&stack.oldest).copied(),
                    training: hour
                        .statuses
                        .get(stack.oldest)
                        .and_then(|status| status.exercising()),
                    uv_moving: uv_moving.contains(&stack.oldest),
                    scale: if transport {
                        self.gates.transport_scale
                    } else {
                        1.0
                    },
                },
                world,
                anchor,
            );
            let (mut target, position) = match figure {
                Ok(figure) => figure,
                Err(Unplaced::Unanchored) => {
                    report.unanchored += 1;
                    continue;
                }
                Err(Unplaced::Exercise) => {
                    report.error("division exercise is unavailable this hour");
                    continue;
                }
            };
            if position.motion.is_some() {
                report.moving += 1;
                if clock_unavailable {
                    report.error("unit movement: Clock speed is unavailable");
                }
            }
            if let Some(mesh) = library.meshes.get(slot) {
                if mesh.own_radius < RIG_RADIUS {
                    report.rigs += 1;
                }
                if mesh.attachments > 0 {
                    report.attached += 1;
                }
            }
            *report.by_level.entry(level).or_default() += 1;
            *report.by_entity.entry(name).or_default() += 1;
            target.mesh = slot;
            placed.push((target, position));
        }
        drop(library);
        lap("um_targets");
        let ships = if naval {
            let ships = self.prepare_naval_with_spans(
                frame,
                vision,
                anchor,
                world,
                ships,
                Some(&hour),
                &mut report,
                spans,
            );
            spans.append(hour.spans.replace(spans.fork()));
            ships?
        } else {
            NavalSources::default()
        };
        lap("um_naval");
        let owned = core::<tables::Provinces>(frame)?
            .iter()
            .filter(|row| hour.country.links.get(&row.owner) == Some(&vision.ours()))
            .map(|row| row.id)
            .collect();
        lap("um_owned");
        report.bake_warnings = self.bake_warnings();
        Ok(PreparedModels {
            staged_field: Mutex::new(None),
            targets: None,
            raid: Vec::new(),
            convoys: None,
            placed,
            ships,
            naval,
            owned,
            report,
        })
    }

    pub fn prepare_without_land(
        &self,
        inputs: PrepareInputs<'_>,
        naval: bool,
    ) -> io::Result<PreparedModels> {
        if let Some(reason) = &self.disabled {
            return Ok(PreparedModels::disabled(reason));
        }
        let PrepareInputs {
            frame,
            limits,
            vision,
            anchor,
            world,
            terrain,
            ships,
            localisation,
            ..
        } = inputs;
        let hour = Hour::read_naval(self, frame, terrain, limits, localisation)?;
        let mut report = ModelReport::default();
        let ships = if naval {
            self.prepare_naval_with_spans(
                frame,
                vision,
                anchor,
                world,
                ships,
                Some(&hour),
                &mut report,
                &mut BuilderSpans::new(false),
            )?
        } else {
            NavalSources::default()
        };
        let owned = owned(&hour, frame, vision.ours())?;
        report.bake_warnings = self.bake_warnings();
        Ok(PreparedModels {
            staged_field: Mutex::new(None),
            targets: None,
            raid: Vec::new(),
            convoys: None,
            placed: Vec::new(),
            ships,
            naval,
            owned,
            report,
        })
    }

    pub fn prepare_shipless(
        &self,
        frame: &DecodedFrame,
        ours: u32,
        terrain: &BTreeMap<u32, String>,
        limits: equipment_graphics::Limits<'_>,
        localisation: &Localisation,
    ) -> io::Result<PreparedModels> {
        if let Some(reason) = &self.disabled {
            return Ok(PreparedModels::disabled(reason));
        }
        let hour = Hour::read_naval(self, frame, terrain, limits, localisation)?;
        let owned = owned(&hour, frame, ours)?;
        let report = ModelReport { bake_warnings: self.bake_warnings(), ..ModelReport::default() };
        Ok(PreparedModels {
            staged_field: Mutex::new(None),
            targets: None,
            raid: Vec::new(),
            convoys: None,
            placed: Vec::new(),
            ships: NavalSources::default(),
            naval: false,
            owned,
            report,
        })
    }

    pub fn land_models<R>(
        &self,
        inputs: SelectInputs<'_>,
        kept: &KeptChoices,
        told: &Told,
        choose: impl FnOnce(&mut LandModels<'_, '_, '_>) -> R,
    ) -> io::Result<R> {
        if let Some(reason) = &self.disabled {
            return Err(io::Error::other(reason.clone()));
        }
        let SelectInputs {
            frame,
            limits,
            ours,
            terrain,
            localisation,
            divisions,
            lap,
        } = inputs;
        let mut untimed = |_: &'static str| {};
        let lap: &mut dyn FnMut(&'static str) = match lap {
            Some(lap) => lap,
            None => &mut untimed,
        };
        let country = kept.country();
        lap("um_start");
        let mut hour = Hour::read_profiled(
            self,
            frame,
            terrain,
            limits,
            localisation,
            country.as_ref(),
            lap,
        )?;
        lap("um_joins_rest");
        if let Ok(equipment) = &mut hour.equipment {
            equipment.asked = divisions;
        }
        Ok(self.with_land_models(&hour, ours, kept, Some(told), lap, choose))
    }

    pub fn chosen_meshes(&self, entities: &[String]) -> Vec<Result<usize, String>> {
        self.ensure_with_spans(entities, &mut BuilderSpans::new(false));
        let library = self.library();
        entities
            .iter()
            .map(|entity| self.chosen_slot(&library, entity))
            .collect()
    }

    fn with_land_models<'a, R>(
        &'a self,
        hour: &Hour<'a>,
        ours: u32,
        kept: &KeptChoices,
        told: Option<&Told>,
        lap: &mut dyn FnMut(&'static str),
        choose: impl FnOnce(&mut LandModels<'_, '_, '_>) -> R,
    ) -> R {
        let frame = hour.frame;
        let view = model_multi_fallback::view_country_ref(frame, ours);
        lap("um_view");
        let census = hour
            .retained_conventional()
            .map(|(retained, _)| Arc::clone(retained))
            .map_err(Clone::clone);
        lap("um_census");
        let now = Arc::new(frame.clone());
        let held = kept
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .filter(|kept| {
                kept.context.same(hour, &view) && hour.limits.decide_the_same(&kept.decided)
            });
        lap("um_context");
        let (context, then, held_census, chosen_answers, graphical_answers, fallback_answers) =
            match &held {
                Some(held) => (
                    Some(Arc::clone(&held.context)),
                    Arc::clone(&held.frame),
                    held.census.clone(),
                    held.chosen.clone(),
                    held.graphical.clone(),
                    held.fallbacks.clone(),
                ),
                None => (
                    None,
                    Arc::clone(&now),
                    Err(String::new()),
                    kept::Kept::indexed(),
                    kept::Kept::indexed(),
                    kept::Kept::indexed(),
                ),
            };
        let (members, camouflaged) = match held {
            Some(held) => (held.members.clone(), held.camouflage.clone()),
            None => (kept::Kept::checked(), kept::Kept::indexed()),
        };
        let between = ToldChanges {
            between: kept::Between::new(&then, &now),
            then: &then,
            told,
        };
        let census_changed: &dyn Fn(&CensusRead) -> bool = &|read| match (&held_census, &census) {
            (Ok(was), Ok(is)) => !(Arc::ptr_eq(was, is) || was.same_read(is, read)),
            (Err(was), Err(is)) => was != is,
            _ => true,
        };
        let graphical_answers = RefCell::new(graphical_answers.hour(&between, census_changed));
        let fallback_answers = RefCell::new(fallback_answers.hour(&between, census_changed));
        let mut models = LandModels {
            selector: Selector {
                assets: self,
                hour,
                frame,
                graphical: BTreeMap::new(),
                fallbacks: BTreeMap::new(),
                view: view.clone(),
                graphical_answers: &graphical_answers,
                fallback_answers: &fallback_answers,
            },
            chosen: chosen_answers.hour(&between, census_changed),
            templates: BTreeMap::new(),
            members: members.hour(&between, census_changed),
            state_members: None,
            camouflages: camouflaged.hour(&between, census_changed),
            made: Made {
                contexts: usize::from(
                    kept.country()
                        .is_none_or(|was| !Arc::ptr_eq(&was, &hour.country)),
                ),
                ..Made::default()
            },
        };
        lap("um_kept_answers");
        let chose = choose(&mut models);
        lap("um_session");
        let LandModels {
            selector,
            chosen,
            members,
            camouflages,
            ..
        } = models;
        drop(selector);
        let (chosen_answers, graphical_answers, fallback_answers) = (
            chosen.finish(),
            graphical_answers.into_inner().finish(),
            fallback_answers.into_inner().finish(),
        );
        let (members, camouflage) = (members.finish(), camouflages.finish());
        drop(between);
        let context = match context {
            Some(context) if Arc::ptr_eq(&context.country, &hour.country) => context,
            Some(context) => Arc::new(SelectionContext {
                country: Arc::clone(&hour.country),
                ..SelectionContext::clone(&context)
            }),
            None => Arc::new(SelectionContext::capture(hour, view)),
        };
        let selections = Selections {
            context,
            frame: now,
            decided: hour.limits.decided(),
            census,
            chosen: chosen_answers,
            graphical: graphical_answers,
            fallbacks: fallback_answers,
            members,
            camouflage,
        };
        *kept
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::new(selections));
        lap("um_keep");
        chose
    }

    pub fn chosen_slot(&self, library: &ModelLibrary, entity: &str) -> Result<usize, String> {
        if let Some(reason) = self.refusal(entity) {
            return Err(reason);
        }
        library
            .slot(entity)
            .ok_or_else(|| format!("registered entity {entity} did not bake"))
    }
}

#[derive(Default)]
pub struct KeptChoices(Mutex<Option<Arc<Selections>>>);

impl KeptChoices {
    fn country(&self) -> Option<Arc<CountryContext>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(|kept| Arc::clone(&kept.context.country))
    }
}

#[derive(Clone, Default)]
pub struct Told(Vec<(TableId, SharedView, Vec<(u128, u64)>)>);

impl Told {
    pub fn new(mut tables: Vec<(TableId, SharedView, Vec<(u128, u64)>)>) -> Self {
        tables.sort_unstable_by_key(|(table, ..)| table.index());
        Self(tables)
    }

    fn rows(&self, table: TableId, then: &DecodedFrame) -> Option<&[(u128, u64)]> {
        let at = self
            .0
            .binary_search_by_key(&table.index(), |(held, ..)| held.index())
            .ok()?;
        let (_, from, rows) = &self.0[at];
        SharedView::ptr_eq(from, &then.tables[table.index()]).then_some(rows.as_slice())
    }
}

struct ToldChanges<'a> {
    between: kept::Between<'a>,
    then: &'a DecodedFrame,
    told: Option<&'a Told>,
}

impl ToldChanges<'_> {
    fn told(&self, table: TableId) -> Option<&[(u128, u64)]> {
        self.told?.rows(table, self.then)
    }
}

impl kept::Changes for ToldChanges<'_> {
    fn table(&self, table: TableId) -> kept::Change<'_> {
        match self.told(table) {
            Some([]) => kept::Change::Same,
            Some(rows) => kept::Change::Rows(rows, &[]),
            None => self.between.table(table),
        }
    }

    fn rows(&self, table: TableId) -> Option<(&[(u128, u64)], &[u128])> {
        match self.told(table) {
            Some(rows) => Some((rows, &[][..])),
            None => self.between.rows(table),
        }
    }

    fn same_rows(&self, table: TableId, from: u128, to: u128, fields: u64) -> Option<bool> {
        match self.told(table) {
            Some(_) => None,
            None => self.between.same_rows(table, from, to, fields),
        }
    }
}

pub struct SelectInputs<'a> {
    pub frame: &'a DecodedFrame,
    pub limits: equipment_graphics::Limits<'a>,
    pub ours: u32,
    pub terrain: &'a BTreeMap<u32, String>,
    pub localisation: &'a Localisation,
    pub divisions: Option<&'a BTreeSet<u64>>,
    pub lap: Option<&'a mut dyn FnMut(&'static str)>,
}

pub struct LandModels<'a, 'h, 'k> {
    selector: Selector<'a, 'h, 'k>,
    chosen: CensusChecked<'k, SelectionKey, Chosen>,
    templates: Templates<'a>,
    members: CensusChecked<'k, (), camouflage::StateMembers>,
    state_members: Option<Arc<camouflage::StateMembers>>,
    camouflages: CensusChecked<'k, u64, Camouflaged>,
    made: Made,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Made {
    pub selections: usize,
    pub camouflages: usize,
    pub contexts: usize,
}

impl LandModels<'_, '_, '_> {
    pub fn made(&self) -> Made {
        self.made
    }

    pub fn stands(&mut self, province: u32, key: SelectionKey) -> bool {
        self.chosen.get(&key).is_some()
            && (key.transport
                || self
                    .camouflages
                    .get(&key.oldest)
                    .is_some_and(|answer| answer.value.0 == province))
    }

    pub fn choose(&mut self, province: u32, key: SelectionKey) -> StackModel {
        let Self {
            selector,
            chosen,
            templates,
            members,
            state_members,
            camouflages,
            made,
        } = self;
        let selected = match chosen.get(&key) {
            Some(answer) => answer.value,
            None => {
                made.selections += 1;
                let (selected, reads) = kept::record::<CensusRead, _>(selector.frame, || {
                    let selected = if key.transport {
                        selector.transport(&key)
                    } else {
                        selector.land(&key, templates)
                    };
                    selected.map(|selected| {
                        selected.map(|(level, name, key)| (level, name, key.map(str::to_owned)))
                    })
                });
                chosen.put(key, selected, reads).value
            }
        };
        let (level, base) = match &*selected {
            Ok(Some((level, name, _))) => (*level, name.clone()),
            Ok(None) => return StackModel::UnknownIdentity,
            Err(Missing::Template) => return StackModel::Untemplated,
            Err(Missing::Input(reason)) => return StackModel::Withheld(reason.clone()),
        };
        let mut entity = base.clone();
        if !key.transport {
            let (assets, hour) = (selector.assets, selector.hour);
            let held = camouflages
                .get(&key.oldest)
                .filter(|answer| answer.value.0 == province);
            let answer = match held {
                Some(answer) => answer.value,
                None => {
                    made.camouflages += 1;
                    let (suffix, reads) = kept::record::<CensusRead, _>(hour.frame, || {
                        hour.selection.measure(Stage::Camouflage, || {
                            let fields = camouflage::fields();
                            let division = u128::from(key.oldest);
                            kept::note_row(TableId::DivisionStatus, division, fields.status);
                            kept::note_row(TableId::DivisionModelInputs, division, fields.witness);
                            let status = hour.statuses.get(key.oldest).ok_or_else(|| {
                                "division acclimatization status is unavailable".to_owned()
                            })?;
                            let members = state_members.get_or_insert_with(|| {
                                members.get_or_build((), hour.frame, || {
                                    camouflage::StateMembers::read(hour.camouflage.provinces())
                                })
                            });
                            hour.camouflage
                                .suffix(
                                    &assets.camouflage,
                                    hour.sources.tokens,
                                    status,
                                    province,
                                    members,
                                )
                                .map(|suffix| suffix.map(str::to_owned))
                                .map_err(|error| error.to_string())
                        })
                    });
                    camouflages.put(key.oldest, (province, suffix), reads).value
                }
            };
            match &answer.1 {
                Ok(suffix) => {
                    entity = camouflage::entity(&base, suffix.as_deref(), |candidate| {
                        assets.registered(candidate)
                    });
                }
                Err(reason) => return StackModel::Withheld(reason.clone()),
            }
        }
        StackModel::Chosen(ChosenModel {
            level,
            entity,
            base,
            answer: selected,
        })
    }
}

#[derive(Clone, Debug)]
pub enum StackModel {
    Chosen(ChosenModel),
    UnknownIdentity,
    Untemplated,
    Withheld(String),
}

#[derive(Clone)]
pub struct ChosenModel {
    pub level: Level,
    pub entity: String,
    base: String,
    answer: Arc<Chosen>,
}

impl std::fmt::Debug for ChosenModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChosenModel")
            .field("level", &self.level)
            .field("entity", &self.entity)
            .field("base", &self.base)
            .finish()
    }
}

impl ChosenModel {
    fn sprite_keyed(&self, assets: &UnitModelAssets) -> bool {
        let subunit = match &*self.answer {
            Ok(Some((_, _, key))) => key.as_deref(),
            _ => None,
        };
        subunit
            .and_then(|key| assets.sub_units.subunit(key))
            .is_some_and(|unit| {
                unit.sprite != unit.key && self.base.ends_with(&entity_name("", &unit.sprite))
            })
    }
}


struct Hour<'a> {
    country: Arc<CountryContext>,
    templates: &'a [DivisionTemplateIconRow],
    sources: model_selection::Sources<'a>,
    variants: model_selection::Variants<'a>,
    tokens: TokenNames<'a>,
    equipment: Result<Equipment<'a>, String>,
    statuses: Statuses<'a>,
    camouflage: camouflage::Context<'a>,
    spans: RefCell<BuilderSpans>,
    selection: Stages,
    classification: OnceCell<Option<model_selection::facts::Hour>>,
    conventional: OnceCell<Result<(Arc<bridge::Retained>, bridge::Prepared<'a>), String>>,
    assets: &'a UnitModelAssets,
    frame: &'a DecodedFrame,
    limits: equipment_graphics::Limits<'a>,
    labels: model_labels::Labels<'a>,
}

impl<'a> Hour<'a> {
    fn read(
        assets: &'a UnitModelAssets,
        frame: &'a DecodedFrame,
        terrain: &'a BTreeMap<u32, String>,
        limits: equipment_graphics::Limits<'a>,
        localisation: &'a Localisation,
    ) -> io::Result<Self> {
        Self::read_profiled(assets, frame, terrain, limits, localisation, None, &mut |_| {})
    }

    fn read_profiled(
        assets: &'a UnitModelAssets,
        frame: &'a DecodedFrame,
        terrain: &'a BTreeMap<u32, String>,
        limits: equipment_graphics::Limits<'a>,
        localisation: &'a Localisation,
        kept: Option<&Arc<CountryContext>>,
        lap: &mut dyn FnMut(&'static str),
    ) -> io::Result<Self> {
        let equipment = Equipment::read(frame)?;
        lap("um_equipment");
        let country = match kept.filter(|kept| kept.stands(frame)) {
            Some(kept) => Arc::clone(kept),
            None => Arc::new(CountryContext::read(frame)?),
        };
        Ok(Self {
            labels: model_labels::Labels::read(
                frame,
                localisation,
                assets.dynamic_tags.as_ref().map_err(Clone::clone),
            ),
            country,
            templates: rows::<tables::DivisionTemplateIcons>(frame)?.rows(),
            sources: model_selection::Sources::read(frame)?,
            variants: model_selection::Variants::new(
                rows::<tables::VariantDefinitions>(frame)?.rows(),
            ),
            tokens: TokenNames::new(frame)?,
            equipment,
            statuses: Statuses::read(frame)?,
            camouflage: {
                lap("um_model_joins");
                let camouflage = camouflage::Context::read(frame, terrain)?;
                lap("um_camouflage");
                camouflage
            },
            spans: RefCell::new(BuilderSpans::new(false)),
            selection: Stages::new(false),
            classification: OnceCell::new(),
            conventional: OnceCell::new(),
            assets,
            frame,
            limits,
        })
    }

    fn read_naval(
        assets: &'a UnitModelAssets,
        frame: &'a DecodedFrame,
        terrain: &'a BTreeMap<u32, String>,
        limits: equipment_graphics::Limits<'a>,
        localisation: &'a Localisation,
    ) -> io::Result<Self> {
        Ok(Self {
            labels: model_labels::Labels::read(
                frame,
                localisation,
                assets.dynamic_tags.as_ref().map_err(Clone::clone),
            ),
            country: Arc::new(CountryContext::read(frame)?),
            templates: &[],
            sources: model_selection::Sources::read(frame)?,
            variants: model_selection::Variants::new(
                rows::<tables::VariantDefinitions>(frame)?.rows(),
            ),
            tokens: TokenNames::new(frame)?,
            equipment: Err("division equipment is not read for ships".into()),
            statuses: Statuses::default(),
            camouflage: camouflage::Context::none(terrain),
            spans: RefCell::new(BuilderSpans::new(false)),
            selection: Stages::new(false),
            classification: OnceCell::new(),
            conventional: OnceCell::new(),
            assets,
            frame,
            limits,
        })
    }

    fn geography(&self) -> Result<&context::Prepared, String> {
        self.country
            .geography
            .get_or_init(|| {
                self.spans
                    .borrow_mut()
                    .measure("units.selection.geography_read", |_| {
                        view_reads::unrecorded(|| {
                            view_reads::record(|| {
                                context::Prepared::read(self.frame, &self.assets.geography)
                            })
                        })
                    })
            })
            .0
            .as_ref()
            .map_err(Clone::clone)
    }

    fn conventional(&self) -> Result<&bridge::Prepared<'a>, String> {
        self.retained_conventional()
            .map(|(_, prepared)| prepared)
            .map_err(Clone::clone)
    }

    fn retained_conventional(
        &self,
    ) -> Result<&(Arc<bridge::Retained>, bridge::Prepared<'a>), &String> {
        kept::note(|| CensusRead::Census);
        self.conventional
            .get_or_init(|| {
                self.spans
                    .borrow_mut()
                    .measure("units.selection.bridge_read", |_| {
                        self.assets
                            .conventional
                            .read(self.frame, &self.assets.sub_units)
                            .map(|retained| {
                                let prepared = retained.prepared();
                                (retained, prepared)
                            })
                    })
            })
            .as_ref()
    }

    fn classification(&self) -> Option<&model_selection::facts::Hour> {
        self.classification
            .get_or_init(|| self.assets.sub_units.classification_facts(self.frame))
            .as_ref()
    }

    fn template(&self, template: u64) -> Option<&'a DivisionTemplateIconRow> {
        self.templates
            .binary_search_by_key(&template, |row| row.template)
            .ok()
            .map(|at| &self.templates[at])
    }

    fn country(&self, raw: u32) -> Option<&ModelCountry> {
        self.country
            .links
            .get(&raw)
            .and_then(|slot| self.country.countries.get(slot))
    }
}

struct CountryContext {
    links: BTreeMap<u32, u32>,
    countries: BTreeMap<u32, ModelCountry>,
    views: Vec<ViewIdentity>,
    geography: OnceLock<(Result<context::Prepared, String>, Vec<ViewIdentity>)>,
}

impl CountryContext {
    fn read(frame: &DecodedFrame) -> io::Result<Self> {
        let (read, views) = view_reads::record(|| -> io::Result<_> {
            let links: BTreeMap<u32, u32> = core::<tables::CountryLinks>(frame)?
                .iter()
                .map(|link| (link.id, link.canonical_slot))
                .collect();
            Ok((links, model_countries::read(frame)?))
        });
        let (links, countries) = read?;
        Ok(Self {
            links,
            countries,
            views,
            geography: OnceLock::new(),
        })
    }

    fn joined(&self) -> Option<Result<&context::Prepared, &str>> {
        self.geography
            .get()
            .map(|(geography, _)| geography.as_ref().map_err(String::as_str))
    }

    fn stands(&self, frame: &DecodedFrame) -> bool {
        let held = |view: &ViewIdentity| frame.tables.iter().any(|table| view.is(table));
        self.views.iter().all(held)
            && self
                .geography
                .get()
                .is_none_or(|(_, views)| views.iter().all(held))
    }
}

fn graphics_model(
    assets: &UnitModelAssets,
    hour: &Hour<'_>,
    pair: SelectedPair<'_>,
    first_archetype: Option<&str>,
    roster_owner: u32,
) -> Result<Option<(Level, String)>, String> {
    let raw_country = model_selection::model_country(pair, roster_owner);
    graphics_model_for_country(assets, hour, pair, first_archetype, raw_country)
}

fn graphics_model_for_country(
    assets: &UnitModelAssets,
    hour: &Hour<'_>,
    pair: SelectedPair<'_>,
    first_archetype: Option<&str>,
    raw_country: u32,
) -> Result<Option<(Level, String)>, String> {
    hour.selection.measure(Stage::Graphics, || {
        let content = &assets.sub_units;
        let country = hour
            .country(raw_country)
            .ok_or_else(|| format!("model country {raw_country} is unavailable"))?;
        let equipment_key = content.equipment_key(&hour.tokens, pair.variant)?;
        let equipment = content
            .equipment_model(equipment_key)
            .map_err(|error| error.to_string())?;
        let family = Some((
            equipment.family,
            Some(
                content
                    .equipment_model(equipment.family)
                    .map_err(|error| error.to_string())?
                    .archetype,
            ),
        ));
        let (ideology, culture) =
            model_multi_fallback::creator_operands(
                pair.variant,
                &hour.country.links,
                &hour.country.countries,
            )?;
        let geography = hour.geography()?.context(raw_country)?;
        let capital_continent = match geography {
            Some(country) => country
                .capital_continent
                .as_ref()
                .map(Option::as_deref)
                .map_err(String::as_str),
            None => Ok(None),
        };
        let graphical = hour.selection.measure(Stage::GraphicsRules, || {
            assets.graphics.lookup_with_limits(
                &content_reader_equipment_graphics::equipment_graphics::Context {
                    equipment: equipment_key,
                    archetype: Some(equipment.archetype),
                    family,
                    subunit: Some(&pair.subunit.key),
                    ideology,
                    culture,
                    country: geography.map(|country| country.country_tag.as_str()),
                    faction_tags: geography.map_or(&[], |country| country.faction_tags.as_slice()),
                    capital_continent,
                },
                &hour.limits,
                |name| assets.registered(name),
            )
        })?;
        let conventional = hour.conventional()?;
        hour.selection.measure(Stage::GraphicsResolve, || {
            Ok(conventional
                .resolve(
                    raw_country,
                    country,
                    bridge::Selection {
                        subunit: pair.subunit,
                        first_archetype,
                        variant: Some(pair.variant.id),
                    },
                    &graphical,
                    |name| assets.registered(name),
                )?
                .map(|name| {
                    let level = if graphical.iter().any(|candidate| candidate.entity == name) {
                        Level::Graphics
                    } else {
                        Level::Conventional
                    };
                    (level, name)
                }))
        })
    })
}

struct Equipment<'a> {
    daily: &'a [replay_data::rows::DivisionEquipmentRow],
    new: &'a [replay_data::rows::DivisionEquipmentRow],
    new_complete: bool,
    asked: Option<&'a BTreeSet<u64>>,
}

impl<'a> Equipment<'a> {
    fn read(frame: &'a DecodedFrame) -> io::Result<Result<Self, String>> {
        let daily = rows::<tables::DivisionEquipment>(frame)?;
        if !daily.is_complete() {
            return Ok(Err("division equipment is not a complete census".to_owned()));
        }
        let new = rows::<tables::NewDivisionEquipment>(frame)?;
        Ok(Ok(Self::join(daily.rows(), new.rows(), new.is_complete())))
    }

    fn join(
        daily: &'a [replay_data::rows::DivisionEquipmentRow],
        new: &'a [replay_data::rows::DivisionEquipmentRow],
        new_complete: bool,
    ) -> Self {
        Self {
            daily,
            new,
            new_complete,
            asked: None,
        }
    }

    fn of(&self, division: u64) -> Result<&'a [replay_data::rows::DivisionEquipmentRow], String> {
        if self.asked.is_some_and(|asked| !asked.contains(&division)) {
            return Err(format!("division {division}'s equipment was not asked of the rows at the head"));
        }
        fn rows_of(
            rows: &[replay_data::rows::DivisionEquipmentRow],
            division: u64,
        ) -> &[replay_data::rows::DivisionEquipmentRow] {
            let start = rows.partition_point(|row| row.division < division);
            let end = start + rows[start..].partition_point(|row| row.division == division);
            &rows[start..end]
        }
        let new = rows_of(self.new, division);
        if !new.is_empty() {
            return Ok(new);
        }
        match rows_of(self.daily, division) {
            [] if self.new_complete => Ok(&[]),
            [] => Err("division equipment unknown until the next daily refresh".to_owned()),
            daily => Ok(daily),
        }
    }
}

type Chosen = Result<Option<(Level, String, Option<String>)>, Missing>;
type GraphicalAnswer = Result<Option<(Level, String)>, String>;
type FallbackAnswer = Result<Option<model_multi_fallback::Resolved>, String>;
type Camouflaged = (u32, Result<Option<String>, String>);

struct Selections {
    context: Arc<SelectionContext>,
    frame: Arc<DecodedFrame>,
    decided: equipment_graphics::Decided,
    census: Result<Arc<bridge::Retained>, String>,
    chosen: kept::Kept<SelectionKey, Chosen, CensusRead>,
    graphical: kept::Kept<GraphicalAnswerKey, GraphicalAnswer, CensusRead>,
    fallbacks: kept::Kept<u64, FallbackAnswer, CensusRead>,
    members: kept::Kept<(), camouflage::StateMembers, CensusRead>,
    camouflage: kept::Kept<u64, Camouflaged, CensusRead>,
}

#[derive(Clone)]
struct SelectionContext {
    view: Result<u32, String>,
    country: Arc<CountryContext>,
    labels: (BTreeMap<u32, model_labels::Country>, Option<String>),
    sources: Option<Arc<model_selection::SourceIndex>>,
    tokens: Option<replay_frame::frame::SharedView>,
}

impl SelectionContext {
    fn capture(hour: &Hour<'_>, view: Result<u32, String>) -> Self {
        let (labels, unavailable) = hour.labels.operands();
        let _ = hour.geography();
        Self {
            view,
            country: Arc::clone(&hour.country),
            labels: (labels.clone(), unavailable.cloned()),
            sources: Some(Arc::clone(hour.sources.index())),
            tokens: Some(hour.frame.tables[TableId::LexicalTokens.index()].clone()),
        }
    }

    fn same_countries(&self, hour: &Hour<'_>) -> bool {
        if Arc::ptr_eq(&self.country, &hour.country) {
            return true;
        }
        let _ = hour.geography();
        self.country.links == hour.country.links
            && self.country.countries == hour.country.countries
            && self.country.joined() == hour.country.joined()
    }

    fn same(&self, hour: &Hour<'_>, view: &Result<u32, String>) -> bool {
        let (labels, unavailable) = hour.labels.operands();
        let tokens = &hour.frame.tables[TableId::LexicalTokens.index()];
        self.view == *view
            && self.same_countries(hour)
            && self.labels.0 == *labels
            && self.labels.1.as_ref() == unavailable
            && self.sources.as_ref().is_some_and(|sources| {
                Arc::ptr_eq(sources, hour.sources.index()) || **sources == **hour.sources.index()
            })
            && self.tokens.as_ref().is_some_and(|was| {
                replay_frame::frame::SharedView::ptr_eq(was, tokens) || **was == **tokens
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SelectionKey {
    pub transport: bool,
    pub template: u64,
    pub oldest: u64,
    pub roster_owner: u32,
    pub expeditionary_owner: u32,
}

#[derive(Clone)]
enum Missing {
    Template,
    Input(String),
}

impl From<io::Error> for Missing {
    fn from(error: io::Error) -> Self {
        Self::Input(error.to_string())
    }
}

impl From<String> for Missing {
    fn from(error: String) -> Self {
        Self::Input(error)
    }
}

type Selected<'a> = Result<Option<(Level, String, Option<&'a str>)>, Missing>;

type GraphicalKey = (u64, String, Definition, u32);

type GraphicalAnswerKey = (GraphicalKey, Option<String>);

type Templates<'a> = BTreeMap<u64, Result<Template<'a>, String>>;

struct Selector<'a, 'h, 'k> {
    assets: &'a UnitModelAssets,
    hour: &'h Hour<'a>,
    frame: &'a DecodedFrame,
    graphical: BTreeMap<GraphicalKey, kept::Answer<GraphicalAnswer, CensusRead>>,
    fallbacks: BTreeMap<u64, kept::Answer<FallbackAnswer, CensusRead>>,
    view: Result<u32, String>,
    graphical_answers: &'h RefCell<CensusChecked<'k, GraphicalAnswerKey, GraphicalAnswer>>,
    fallback_answers: &'h RefCell<CensusChecked<'k, u64, FallbackAnswer>>,
}

type CensusChecked<'h, K, V> =
    kept::Checked<'h, K, V, CensusRead, ToldChanges<'h>, &'h dyn Fn(&CensusRead) -> bool>;

impl<'a> Selector<'a, '_, '_> {
    fn exists(&self, name: &str) -> bool {
        self.assets.registered(name)
    }

    fn transport(&self, stack: &SelectionKey) -> Selected<'a> {
        let hour = self.hour;
        hour.selection.measure(Stage::Transport, || {
            let raw = model_owner(stack.roster_owner, stack.expeditionary_owner, true);
            let Some(country) = self.hour.country(raw) else {
                return Ok(None);
            };
            match resolve_transport(country, |name| self.exists(name)) {
                Some((level, name)) => Ok(Some((level, name, None))),
                None if country.original_tag.is_none()
                    || country.original_graphical_culture.is_none() =>
                {
                    Ok(None)
                }
                None => Err(Missing::Input(format!(
                    "{PLACEHOLDER_ENTITY} is not registered"
                ))),
            }
        })
    }

    fn land(&mut self, stack: &SelectionKey, templates: &mut Templates<'a>) -> Selected<'a> {
        let hour = self.hour;
        hour.selection.measure(Stage::Land, || {
            kept::note_row(
                TableId::DivisionTemplateIcons,
                u128::from(stack.template),
                u64::MAX,
            );
            let template = self
                .hour
                .template(stack.template)
                .ok_or(Missing::Template)?;
            if template.model_unreadable != 0 {
                return Err(Missing::Input(
                    "template model inputs were unreadable this hour".into(),
                ));
            }
            let explicit = std::str::from_utf8(&template.model_entity)
                .map_err(|_| Missing::Input("template model entity is not UTF-8".into()))?;
            if !explicit.is_empty() && self.exists(explicit) {
                return Ok(Some((Level::Template, explicit.to_owned(), None)));
            }
            let division = u128::from(stack.oldest) << 32;
            for table in [
                TableId::NewDivisionEquipment,
                TableId::DivisionEquipment,
            ] {
                kept::note_range(table, division, division + (1 << 32), u64::MAX);
            }
            let equipment = self
                .hour
                .equipment
                .as_ref()
                .map_err(Clone::clone)?
                .of(stack.oldest)?;
            let composition = templates.entry(template.template).or_insert_with(|| {
                self.hour.selection.measure(Stage::Template, || {
                    self.assets
                        .sub_units
                        .template(template, &self.hour.sources)
                        .map_err(|error| error.to_string())
                })
            });
            let composition = composition
                .as_ref()
                .map_err(|error| Missing::Input(error.clone()))?;
            let facts = if composition.representative.is_some() && !equipment.is_empty() {
                hour.classification()
            } else {
                None
            };
            let pair = self.assets.sub_units.select_pair_classified(
                composition,
                template.model_category,
                &template.excluded_variants,
                equipment,
                self.hour.variants,
                &self.hour.tokens,
                &self.hour.selection,
                facts,
            )?;
            if let Some(pair) = pair {
                return self.paired(stack, composition, pair);
            }
            if model_multi_fallback::has_qualifying_requirements(template, composition)
                && let Some(resolved) = self.fallback(template, composition)?
            {
                let key = self
                    .assets
                    .sub_units
                    .subunit(&resolved.subunit_key)
                    .map(|unit| unit.key.as_str());
                let level = if resolved.graphical {
                    Level::Graphics
                } else {
                    Level::Conventional
                };
                return Ok(Some((level, resolved.entity, key)));
            }
            let Some(representative) = &composition.representative else {
                return self.placeholder();
            };
            self.numbered(stack, stack.roster_owner, representative.subunit)
        })
    }

    fn paired(
        &mut self,
        stack: &SelectionKey,
        composition: &Template<'a>,
        pair: SelectedPair<'a>,
    ) -> Selected<'a> {
        let hour = self.hour;
        hour.selection.measure(Stage::Paired, || {
            let key = Some(pair.subunit.key.as_str());
            let explicit = std::str::from_utf8(&pair.variant.explicit_entity)
                .map_err(|_| Missing::Input("variant model entity is not UTF-8".into()))?;
            if !explicit.is_empty() && self.exists(explicit) {
                return Ok(Some((Level::Variant, explicit.to_owned(), key)));
            }
            let memo = (
                pair.variant.id,
                pair.subunit.key.clone(),
                pair.identity,
                stack.roster_owner,
            );
            if !self.graphical.contains_key(&memo) {
                let first = composition
                    .groups
                    .iter()
                    .chain(&composition.representative)
                    .find(|group| group.identity == pair.identity)
                    .and_then(Template::first_requirement);
                let key = (memo.clone(), first.map(str::to_owned));
                let kept = self.graphical_answers.borrow_mut().get(&key);
                let answer = match kept {
                    Some(kept) => kept,
                    None => {
                        let (resolved, reads) = kept::record::<CensusRead, _>(self.frame, || {
                            kept::note_row(
                                TableId::VariantDefinitions,
                                u128::from(pair.variant.id),
                                u64::MAX,
                            );
                            self.graphical_model(pair, first, stack.roster_owner)
                        });
                        self.graphical_answers.borrow_mut().put(key, resolved, reads)
                    }
                };
                self.graphical.insert(memo.clone(), answer);
            } else {
                kept::note_all(&self.graphical[&memo].reads);
            }
            if let Some((level, name)) = (*self.graphical[&memo].value).clone()? {
                return Ok(Some((level, name, key)));
            }
            let country = model_selection::model_country(pair, stack.roster_owner);
            self.numbered(stack, country, pair.subunit)
        })
    }

    fn graphical_model(
        &self,
        pair: SelectedPair<'_>,
        first_archetype: Option<&str>,
        roster_owner: u32,
    ) -> Result<Option<(Level, String)>, String> {
        graphics_model(self.assets, self.hour, pair, first_archetype, roster_owner)
    }

    fn fallback(
        &mut self,
        template: &DivisionTemplateIconRow,
        composition: &Template<'a>,
    ) -> Result<Option<model_multi_fallback::Resolved>, Missing> {
        let hour = self.hour;
        hour.selection.measure(Stage::Fallback, || {
            if let Some(answer) = self.fallbacks.get(&template.template) {
                kept::note_all(&answer.reads);
            } else {
                let kept = self.fallback_answers.borrow_mut().get(&template.template);
                let answer = match kept {
                    Some(kept) => kept,
                    None => {
                        let view = self.view.clone();
                        let (resolved, reads) = kept::record::<CensusRead, _>(self.frame, || {
                            kept::note_row(
                                TableId::DivisionTemplateIcons,
                                u128::from(template.template),
                                u64::MAX,
                            );
                            view.and_then(|view| {
                                model_multi_fallback::resolve(&model_multi_fallback::Inputs {
                                    template,
                                    composition,
                                    content: &self.assets.sub_units,
                                    graphics: &self.assets.graphics,
                                    geography: self.hour.geography()?,
                                    conventional: self.hour.conventional()?,
                                    countries: &self.hour.country.countries,
                                    links: &self.hour.country.links,
                                    definitions: self.hour.variants,
                                    tokens: &self.hour.tokens,
                                    viewing_country_ref: view,
                                    limits: &self.hour.limits,
                                    registered: &self.assets.registered,
                                    labels: &self.hour.labels,
                                })
                            })
                        });
                        self.fallback_answers.borrow_mut().put(template.template, resolved, reads)
                    }
                };
                self.fallbacks.insert(template.template, answer);
            }
            Ok((*self.fallbacks[&template.template].value).clone()?)
        })
    }

    fn numbered(&self, stack: &SelectionKey, country: u32, subunit: &'a SubUnit) -> Selected<'a> {
        let hour = self.hour;
        hour.selection.measure(Stage::Numbered, || {
            let Some(model_country) = self.hour.country(country) else {
                return Ok(None);
            };
            for table in [
                TableId::DivisionStatus,
                TableId::DivisionModelInputs,
            ] {
                kept::note_row(table, u128::from(stack.oldest), u64::MAX);
            }
            let status = self.hour.statuses.get(stack.oldest).ok_or_else(|| {
                Missing::Input("division model number status is unavailable".into())
            })?;
            let number = status.model_number().ok_or_else(|| {
                Missing::Input("division model number is unavailable this hour".into())
            })?;
            let number = number as i8;
            match resolve_model_numbered(
                model_country,
                &subunit.key,
                Some(&subunit.sprite),
                number,
                |name| self.exists(name),
            ) {
                Some((level, name)) => Ok(Some((level, name, Some(subunit.key.as_str())))),
                None if model_country.graphical_culture.is_none() => Ok(None),
                None => self.placeholder(),
            }
        })
    }

    fn placeholder(&self) -> Selected<'a> {
        if self.exists(PLACEHOLDER_ENTITY) {
            Ok(Some((
                Level::Placeholder,
                PLACEHOLDER_ENTITY.to_owned(),
                None,
            )))
        } else {
            Err(Missing::Input(format!(
                "{PLACEHOLDER_ENTITY} is not registered"
            )))
        }
    }
}

pub struct PreparedModels {
    staged_field: Mutex<Option<StagedField>>,
    targets: Option<Targets>,
    ships: NavalSources,
    raid: Vec<RaidModel>,
    convoys: Option<(
        Vec<crate::trade_routes::ConvoyLine>,
        crate::trade_routes::ConvoyDefines,
        std::sync::Mutex<Option<crate::trade_routes::ConvoyFleet>>,
    )>,
    placed: Vec<(Target, counter_move::Position)>,
    naval: bool,
    owned: BTreeSet<u32>,
    pub report: ModelReport,
}
struct StagedField {
    assets: Arc<()>,
    field: Field,
}

impl PreparedModels {
    fn disabled(reason: &str) -> Self {
        Self {
            staged_field: Mutex::new(None),
            targets: None,
            raid: Vec::new(),
            convoys: None,
            placed: Vec::new(),
            ships: NavalSources::default(),
            naval: true,
            owned: BTreeSet::new(),
            report: ModelReport {
                disabled: Some(reason.to_owned()),
                ..ModelReport::default()
            },
        }
    }

    pub fn withheld(reason: &str) -> Self {
        Self::disabled(reason)
    }

    pub fn ships(&self) -> &NavalSources {
        &self.ships
    }

    pub fn naval(&self) -> bool {
        self.naval
    }

    pub fn with_ships(&self, ships: NavalSources, report: &ModelReport) -> Self {
        let mut joined = self.report.clone();
        joined.absorb(report);
        Self {
            staged_field: Mutex::new(None),
            targets: self.targets.clone(),
            raid: self.raid.clone(),
            convoys: self.convoys.as_ref().map(|(lines, defines, _)| (lines.clone(), *defines, std::sync::Mutex::new(None))),
            placed: self.placed.clone(),
            ships,
            naval: true,
            owned: self.owned.clone(),
            report: joined,
        }
    }

    pub fn movement_positions(
        &self,
    ) -> impl Iterator<Item = (Unit, &counter_move::Position, f32, f32)> {
        let land = self
            .placed
            .iter()
            .map(|(target, position)| land_movement(target, position));
        land.chain(self.ship_movement_positions())
    }

    pub fn ship_movement_positions(
        &self,
    ) -> impl Iterator<Item = (Unit, &counter_move::Position, f32, f32)> {
        self.ships.positions.iter().map(|placed| {
            (
                Unit::TaskForce(placed.force),
                &placed.position,
                placed.rotation,
                placed.scale,
            )
        })
    }

    pub fn stage_field(&mut self, assets: &UnitModelAssets) {
        let field = self.build_field(assets);
        self.targets = Some(field.targets.clone());
        *self
            .staged_field
            .get_mut()
            .unwrap_or_else(|error| error.into_inner()) = Some(StagedField {
            assets: Arc::clone(&assets.field_identity),
            field,
        });
    }

    fn clear_staged_field(&mut self) {
        *self
            .staged_field
            .get_mut()
            .unwrap_or_else(|error| error.into_inner()) = None;
    }

    pub fn owned(&self) -> &BTreeSet<u32> {
        &self.owned
    }

    pub fn add_raid_units(
        &mut self,
        assets: &UnitModelAssets,
        units: &[viewer_gpu::unit_pose::UnitPlacement],
    ) {
        self.clear_staged_field();
        for unit in units {
            let Some(name) = &unit.entity else {
                continue;
            };
            let Some(slot) = assets.bake_slot(name) else {
                self.report
                    .error(format!("raid unit entity {name} did not bake"));
                continue;
            };
            let model = RaidModel {
                slot,
                instance: ModelInstance {
                    scale: unit.scale,
                    animation_seed: unit.seed,
                    ..ModelInstance::default()
                },
                track: unit.track.clone(),
                stages: unit.stages.clone(),
            };
            let instance = model.posed(
                assets,
                viewer_gpu::unit_pose::UnitPose {
                    position: unit.position,
                    facing: unit.facing,
                    progress: unit.progress,
                },
            );
            self.raid.push(RaidModel { instance, ..model });
        }
    }

    pub fn raid_units(
        &self,
        assets: &UnitModelAssets,
        playback: Option<(&HeightField<'_>, f32)>,
    ) -> Vec<(usize, ModelInstance)> {
        self.raid
            .iter()
            .map(|raid| {
                let pose = playback.and_then(|(heights, tick)| raid.track.place(heights, tick));
                let instance = pose.map_or(raid.instance, |pose| raid.posed(assets, pose));
                (raid.slot, instance)
            })
            .collect()
    }

    pub fn add_convoys(
        &mut self,
        lines: Vec<crate::trade_routes::ConvoyLine>,
        defines: crate::trade_routes::ConvoyDefines,
    ) {
        self.clear_staged_field();
        self.convoys = (!lines.is_empty()).then_some((lines, defines, std::sync::Mutex::new(None)));
    }

    pub fn inherit_convoys(&self, previous: &Self) {
        let (Some((lines, defines, fleet)), Some((old_lines, old_defines, old_fleet))) =
            (&self.convoys, &previous.convoys)
        else {
            return;
        };
        if lines == old_lines && defines == old_defines {
            *fleet
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = old_fleet
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
        }
    }

    pub fn reset_convoys(&self) {
        if let Some((_, _, fleet)) = &self.convoys {
            *fleet
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        }
    }

    pub fn has_convoys(&self) -> bool {
        self.convoys.is_some()
    }

    pub fn next_convoy_arrival(
        &self,
        assets: &UnitModelAssets,
        view: &viewer_models::motion_view::MotionView,
        height: &dyn Fn(f32, f32) -> f32,
    ) -> f32 {
        let Some((lines, defines, fleet)) = &self.convoys else {
            return f32::INFINITY;
        };
        let held = fleet
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        held.as_ref().map_or(f32::INFINITY, |fleet| {
            fleet.next_arrival(lines, assets, defines, view, height)
        })
    }

    pub fn convoy_units(
        &self,
        assets: &UnitModelAssets,
        time: f32,
        height: &dyn Fn(f32, f32) -> f32,
        shown: impl Fn(&crate::trade_routes::ConvoyDefines) -> bool,
        refused: &mut BTreeMap<String, usize>,
    ) -> Vec<(usize, ModelInstance)> {
        let Some((lines, defines, fleet)) = &self.convoys else {
            return Vec::new();
        };
        if !shown(defines) {
            return Vec::new();
        }
        let mut radii = BTreeMap::new();
        for line in lines {
            if radii.contains_key(&line.entity) {
                continue;
            }
            let radius = assets
                .bake_slot(&line.entity)
                .and_then(|slot| assets.library().meshes[slot].convoy_radius);
            if radius.is_none() {
                *refused
                    .entry(format!(
                        "convoy entity {}: creation-time bounding sphere unavailable",
                        line.entity
                    ))
                    .or_default() += line.count as usize;
            }
            radii.insert(line.entity.clone(), radius);
        }
        let mut fleet = fleet
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let fleet = fleet.get_or_insert_with(|| {
            crate::trade_routes::ConvoyFleet::new(
                lines,
                defines,
                &crate::trade_routes::presentation_counter,
                &|name| radii.get(name).copied().flatten(),
                height,
            )
        });
        *refused.entry("convoy presentation history: native counter seed, cached positions and sleep state are not recorded; fresh viewer presentation".to_owned()).or_default() += lines.iter().map(|line|line.count as usize).sum::<usize>();
        let poses = fleet.advance(defines, time, height);
        crate::trade_routes::convoy_instances(assets, lines, &poses, refused)
    }

    pub fn field(&self, assets: &UnitModelAssets) -> Field {
        if let Some(staged) = self
            .staged_field
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
            && Arc::ptr_eq(&staged.assets, &assets.field_identity)
        {
            return staged.field;
        }
        self.build_field(assets)
    }

    fn build_field(&self, assets: &UnitModelAssets) -> Field {
        let targets = self.targets.clone().unwrap_or_else(|| {
            self.placed
                .iter()
                .map(|(target, position)| drawn(*target, position))
                .collect::<Vec<_>>()
                .into()
        });
        self.field_of(assets, targets)
    }

    pub fn drawn_by_layers() -> Self {
        Self {
            staged_field: Mutex::new(None),
            targets: None,
            raid: Vec::new(),
            convoys: None,
            placed: Vec::new(),
            ships: NavalSources::default(),
            naval: false,
            owned: BTreeSet::new(),
            report: ModelReport::default(),
        }
    }

    pub fn field_with_land(&self, assets: &UnitModelAssets, land: Vec<Target>) -> Field {
        self.field_of(assets, land.into())
    }

    fn field_of(&self, assets: &UnitModelAssets, targets: Targets) -> Field {
        let raids = self.raid_units(assets, None);
        let mut models: Vec<_> = targets.iter().map(|t| (t.mesh, t.instance)).collect();
        models.extend(
            self.ships
                .formations
                .iter()
                .flat_map(|formation| formation.targets())
                .map(|t| (t.mesh, t.instance)),
        );
        models.extend(
            self.ships
                .combats
                .battles
                .values()
                .flat_map(|battle| battle.targets())
                .map(|target| (target.mesh, target.instance)),
        );
        models.extend_from_slice(&raids);
        if let Some((lines, _, _)) = &self.convoys {
            let slots: BTreeSet<_> = lines
                .iter()
                .filter_map(|line| assets.bake_slot(&line.entity))
                .collect();
            models.extend(slots.into_iter().map(|slot| {
                (
                    slot,
                    ModelInstance {
                        scale: 0.0,
                        ..ModelInstance::default()
                    },
                )
            }));
        }
        Field {
            models: ModelField::build(&assets.library(), models, UNIT_TILE),
            targets,
            raids,
            naval: self.ships.clone(),
        }
    }
}

#[derive(Clone)]
struct RaidModel {
    slot: usize,
    instance: ModelInstance,
    track: viewer_gpu::unit_pose::UnitTrack,
    stages: Vec<content_reader_raids::raids::RaidAnimationStage>,
}

impl RaidModel {
    fn posed(
        &self,
        assets: &UnitModelAssets,
        pose: viewer_gpu::unit_pose::UnitPose,
    ) -> ModelInstance {
        let [fx, fy, fz] = pose.facing;
        let has_state = |state: &str| assets.library().has_state(self.slot, state);
        let animation = viewer_gpu::unit_pose::animation_state(
            &self.stages,
            pose.progress,
            &has_state,
        )
        .map_or(viewer_models::entity_animation::IDLE, |stage| {
            viewer_models::entity_animation::named_stage(&stage.state, stage.intro.as_deref())
        });
        ModelInstance {
            position: pose.position,
            rotation: (-fx).atan2(fz),
            pitch: (-fy).clamp(-1.0, 1.0).asin(),
            animation,
            ..self.instance
        }
    }
}

pub struct LandStack<'a> {
    pub province: u32,
    pub owner: u32,
    pub oldest: u64,
    pub route: Option<&'a Route>,
    pub combat: Option<u32>,
    pub training: Option<bool>,
    pub uv_moving: bool,
    pub scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unplaced {
    Unanchored,
    Exercise,
}

pub fn land_figure(
    stack: &LandStack<'_>,
    world: Option<&counter_move::RouteWorld<'_>>,
    anchor: &dyn Fn(u32, u32) -> Option<[f32; 3]>,
) -> Result<(Target, counter_move::Position), Unplaced> {
    let province = stack.province;
    let position = match stack.route {
        Some(route) => route.position(province, world, anchor),
        None => anchor(province, STANDSTILL).map(counter_move::Position::still),
    }
    .ok_or(Unplaced::Unanchored)?;
    let animation =
        animation_state(stack.route, stack.combat, stack.training).ok_or(Unplaced::Exercise)?;
    let target = Target {
        province,
        identity: stack.oldest,
        order: stack.route.map_or(STANDSTILL, |route| route.stack_kind),
        occupant: stack.oldest,
        part: 0,
        kind: Kind::Land,
        mesh: 0,
        origin: position.standing,
        instance: ModelInstance {
            position: position.standing,
            native_flags: 1 | (u32::from(stack.uv_moving) << 1),
            rotation: hash_rotation(province.wrapping_mul(2_654_435_761) ^ stack.owner),
            scale: stack.scale,
            pitch: 0.0,
            train_clip: [0.0; 4],
            animation,
            animation_seed: stack.oldest as u32 ^ (stack.oldest >> 32) as u32,
            ..ModelInstance::default()
        },
    };
    Ok((target, position))
}

pub fn land_movement<'p>(
    target: &Target,
    position: &'p counter_move::Position,
) -> (Unit, &'p counter_move::Position, f32, f32) {
    let rotation = position.motion.map_or(target.instance.rotation, |motion| {
        motion.position_heading().1 + MESH_FORWARD_OFFSET
    });
    (Unit::Division(target.identity), position, rotation, 1.0)
}

pub fn drawn(mut target: Target, position: &counter_move::Position) -> Target {
    if let Some((at, heading)) = position.motion.as_ref().map(|m| m.position_heading()) {
        target.instance.position = at;
        target.instance.rotation = heading + MESH_FORWARD_OFFSET;
    }
    target
}

type Route = counter_move::DivisionRoute;

type StackKey = (u32, u32, Option<[u32; 2]>);

fn animation_state(
    route: Option<&Route>,
    combat: Option<u32>,
    training: Option<bool>,
) -> Option<u32> {
    use viewer_models::entity_animation::*;
    let standing = || training.map(|training| if training { TRAINING } else { IDLE });
    if let Some(route) = route {
        if matches!(route.stack_kind, 9 | 10) {
            return Some(DEFEND);
        }
        if route.stack_kind == 19 {
            return Some(IDLE);
        }
        if route.stack_kind == 0 {
            return standing();
        }
        let side = match route.combat_role {
            1 => Some(ATTACK),
            2 => Some(DEFEND),
            _ => combat,
        };
        if let Some(side) = side {
            return Some(if side == ATTACK && route.path.is_empty() {
                SUPPORT
            } else {
                side
            });
        }
        return Some(if route.retreat { RETREAT } else { MOVE });
    }
    combat.or_else(standing)
}

struct Stack {
    visible: usize,
    oldest: u64,
    template: u64,
    roster_owner: u32,
    expeditionary_owner: u32,
    route: Option<Route>,
}


#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "test-support"))]
pub mod aircraft_ladder_tests {
    use super::*;
    use replay_data::rows::{SimulationFeatureRow, TableId, TableRows, VariantDefinitionRow};
    use replay_frame::{frame::Metrics, frame::TableView};

    pub fn frame(dlc_mask: u32, saved: &[u8]) -> DecodedFrame {
        let mut views = std::array::from_fn(|_| TableView::Unavailable {
            reason: "lower model operands unavailable".into(),
        });
        views[TableId::VariantDefinitions.index()] =
            TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
                id: 5,
                equipment_token: 7,
                explicit_entity: saved.to_vec(),
                ..Default::default()
            }]));
        views[TableId::SimulationFeatures.index()] =
            TableView::Complete(TableRows::SimulationFeatures(vec![SimulationFeatureRow {
                dlc_mask,
                ..Default::default()
            }]));
        DecodedFrame {
            branch: 1,
            hour: 1,
            tables: views.map(Into::into),
            metrics: Metrics::default(),
        }
    }

    pub fn limits(values: &equipment_graphics::LimitValues) -> equipment_graphics::Limits<'_> {
        fn evaluate(
            _: &[content_reader_parser::parser::Item],
            _: dashboard_text::triggers::Scopes,
        ) -> dashboard_text::triggers::Eval {
            panic!("a declared saved or cosmetic model never evaluates graphical pool limits")
        }
        equipment_graphics::Limits::new(values, &evaluate)
    }

    pub fn cosmetic_aircraft_assets() -> UnitModelAssets {
        UnitModelAssets {
            disabled: None,
            registered: ["COS_frame_entity".to_owned(), "NEW_frame_entity".to_owned()].into(),
            sub_units: model_selection::Content::from_test_documents(&[
                b"equipments = { frame = { is_archetype = yes sprite = unused } }",
            ]),
            ..UnitModelAssets::off(String::new())
        }

    }

    #[test]
    fn declared_saved_aircraft_needs_no_country_or_equipment_or_land_tables() {
        let assets = UnitModelAssets {
            disabled: None,
            registered: ["declared_aircraft".to_owned()].into(),
            ..UnitModelAssets::off(String::new())
        };
        for dlc_mask in [0, 0x100] {
            let frame = frame(dlc_mask, b"declared_aircraft");
            let values = equipment_graphics::LimitValues::default();
            let limits = limits(&values);
            assert_eq!(
                assets.aircraft_entity(&frame, limits, 5, 100),
                Ok(Some("declared_aircraft".to_owned()))
            );
            assert_eq!(
                assets.aircraft_variant_model(&frame, limits, 5, 100),
                Ok(Some("declared_aircraft".to_owned()))
            );
        }
    }

    #[test]
    fn cosmetic_equipment_hit_needs_no_country_tag_or_later_keys() {
        use replay_data::rows::{CountryAppearanceRow, CountryLinkRow, LexicalTokenRow};
        let assets = UnitModelAssets {
            disabled: None,
            registered: ["COS_frame_entity".to_owned()].into(),
            sub_units: model_selection::Content::from_test_documents(&[
                b"equipments = { frame = { is_archetype = yes sprite = unused } }",
            ]),
            ..UnitModelAssets::off(String::new())
        };
        let mut frame = frame(0, b"");
        for (id, rows) in [
            (
                TableId::CountryLinks,
                TableRows::CountryLinks(vec![CountryLinkRow {
                    id: 100,
                    canonical_slot: 1,
                }]),
            ),
            (
                TableId::CountryAppearance,
                TableRows::CountryAppearance(vec![CountryAppearanceRow {
                    country: 100,
                    cosmetic_tag: b"COS".to_vec(),
                    ..Default::default()
                }]),
            ),
            (
                TableId::LexicalTokens,
                TableRows::LexicalTokens(vec![LexicalTokenRow {
                    token: 7,
                    literal: b"frame".to_vec(),
                }]),
            ),
        ] {
            frame.tables[id.index()] = TableView::Complete(rows).into();
        }
        let values = equipment_graphics::LimitValues::default();
        assert_eq!(
            assets.aircraft_entity(&frame, limits(&values), 5, 100),
            Ok(Some("COS_frame_entity".to_owned()))
        );
    }

}
#[cfg(test)]
mod content_tests;
