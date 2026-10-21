//! The unit-model layer: one division figure at each land stack's own anchor,
//! drawn through `crate::map::models`.
//!
//! # Admission and model selection
//!
//! Content defines supply `NGraphics.UNITS_DISTANCE_CUTOFF` and
//! `SHIPS_DISTANCE_CUTOFF`, compared against raw camera height. Standing models
//! share the counter's `Standstill` anchor; moving models share its spline point
//! ([`viewer_foundation::map::counter_move`]). [`crate::map::unit_history`] owns popup scale, camera
//! admission and entity lifetime.
//!
//! A land division uses its preferred template and tries these candidates:
//! 1. The template's saved explicit model, when registered.
//! 2. The equipment/sub-unit pair ([`crate::map::model_selection`]): its saved model,
//!    authored graphic pools and conventional candidates
//!    ([`crate::map::equipment_graphics`]). Without a pair, the six fallback queues
//!    ([`crate::map::model_multi_fallback`]) use the viewing country's rosters.
//! 3. The numbered ladder, with `<key>_<n>_entity`, `<key>_entity`, then
//!    `<sprite>_entity`. The key is the pair's sub-unit or stored representative;
//!    the model number is the sign-extended low byte of the recorded value.
//!    Each key tries cosmetic tag, country tag, runtime graphical culture and
//!    bare name in that order, retaining an empty cosmetic tag. A complete miss
//!    selects [`PLACEHOLDER_ENTITY`].
//! 4. An available camouflage suffix whose condition wins
//!    ([`crate::map::camouflage`]).
//!
//! The ordinary land country is the roster owner. A selected pair can instead
//! use the variant's signed-positive origin or creator.
//!
//! # Naval transfer and facing
//!
//! A recorded naval-transfer link replaces the land entity with [`TRANSPORT_KEY`].
//! Lookup tries the current cosmetic, original-country tag, graphical culture
//! and bare name. Current country means a signed-positive expeditionary owner,
//! otherwise the roster owner. The replacement is omitted when the land entity
//! declares a `naval_move` animation and the unit is over water. Transfer visual
//! states `0x13` and `0x14` use `NGraphics.SHIP_IN_PORT_SCALE`.
//! Railway guns use their own sub-unit key through the same prefix ladder.
//!
//! Moving models face the spline tangent. The game's frame updates damp toward
//! that tangent with `UNIT_TURN_SPEED`, snapping for an initial heading or a
//! changed route. Hourly recordings contain no frame interval, so [`heading`]
//! uses the tangent at the drawn point. Standing headings are unrecorded and
//! use the same deterministic hash as sprawl and building placement.
//!
//! # Recorded inputs and presentation
//!
//! `DivisionTemplateIcons` records template inputs hourly. Unreadable model
//! inputs withhold and count the division's figure while preserving counters.
//! `DivisionEquipment` and country best-variant candidates refresh daily;
//! `NewDivisionEquipment` adds newly deployed divisions hourly. Mid-day changes
//! to existing equipment become visible at the next daily refresh.
//! Without a pair, the fallback's labelled option list decides between rows
//! naming different entities ([`super::model_labels`]); a label the recording
//! or content cannot build refuses the stack by name.
//!
//! Authored states and attachments use `entity_animation`, with seed 0x414e494d
//! XOR the representative handle. Time zero retains the default first pose;
//! original entry times and RNG ordering are unrecorded.
//! Ships at sea use [`ships`] and stand with their counters
//! ([`viewer_foundation::map::task_force_visual`]). Docked formations in visual slot 19 hide
//! their root and all four hull attachments. Static naval-base geometry belongs
//! to `map_buildings`; division transports still draw as described above.
//!
//! # The walker's figure
//!
//! The walking camera mode draws the viewing country's infantry
//! ([`UnitModelAssets::infantry_figure`]): the land ladder's step 3 for the
//! [`INFANTRY_KEY`] sub-unit and its sprite, with the viewing country's
//! cosmetic tag, tag, graphical culture and bare name, falling back to
//! [`PLACEHOLDER_ENTITY`] as a division's complete miss does. A walker is no
//! division: it has no template, equipment, stored model number or
//! acclimatization, so steps 1, 2 and 4 have no operands.

use std::cell::{OnceCell, RefCell};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
use viewer_foundation::debug_fields;
use viewer_foundation::map::aircraft_model;
use viewer_foundation::map::model_admission::Gates;
use viewer_foundation::map::province_admission::province_boxes;
use viewer_foundation::map::province_map::HeightField;
use viewer_foundation::map::province_map::ProvinceMap;
use viewer_foundation::movement_timing::Etas;
use viewer_foundation::profiling::BuilderSpans;
use viewer_foundation::table_access::TokenNames;

use installed_content_core::{ResolvedLayers, defines::Defines, map::MapData, mesh, terrain::DefaultMap};
use installed_content_image::native::Decode;
use installed_content_localisation::localisation::Localisation;
use replay_data::{rows::DivisionTemplateIconRow, rows::VariantDefinitionRow, rows::tables};
use replay_frame::frame::DecodedFrame;

mod ships;
mod retained_bridge;
use model_selection::profile::{Stage, Stages};

pub use ships::ShipPlaces;

use {
    crate::map::camouflage, crate::map::equipment_graphics, crate::map::equipment_graphics::bridge,
    crate::map::equipment_graphics::context, crate::map::model_labels,
    crate::map::model_multi_fallback, crate::map::model_selection,
    crate::map::model_selection::Definition, crate::map::model_selection::SelectedPair,
    crate::map::model_selection::SubUnit, crate::map::model_selection::Template,
    crate::map::models::ModelField, crate::map::models::ModelInstance,
    crate::map::models::ModelLibrary, crate::map::models::hash_rotation,
    crate::map::unit_history::Field, crate::map::unit_history::Kind,
    crate::map::unit_history::NavalSources, crate::map::unit_history::Target,
    crate::map::unit_history::Targets,
    installed_content_equipment_graphics::equipment_graphics::countries as model_countries,
    installed_content_equipment_graphics::equipment_graphics::countries::ModelCountry,
    viewer_foundation::map::counter_move, viewer_foundation::map::perspective::Sight,
    viewer_foundation::map::perspective::Vision, viewer_foundation::map::tables::core,
    viewer_foundation::map::tables::rows, viewer_foundation::map::unit_rules::STANDSTILL,
};

/// The suffix every unit entity's name ends with (seven bytes).
pub const ENTITY_SUFFIX: &str = "_entity";

/// The key a division in naval transfer takes instead of its own.
pub const TRANSPORT_KEY: &str = "transport";

/// The engine's infantry unit type, and the key of the sub-unit and figure
/// entities the walking camera mode's figure takes.
pub const INFANTRY_KEY: &str = "infantry";

/// What the game draws when every candidate misses.
pub const PLACEHOLDER_ENTITY: &str = "test_object_entity";

/// Below this horizontal radius, in world units, an entity's own mesh is an
/// attachment rig rather than a figure.
///
/// `gfx/models/units/vehicles/motorized_frame.mesh` is a 0.102 x 0.001 x 0.102
/// plane sitting 21.5 units below its own origin: the picture the game draws
/// for a motorized, mechanized or cavalry division is entirely in the `attach`
/// blocks of the entity. `crate::map::models` composes those attachments at the joints
/// the entity's default state poses; [`ModelReport::rigs`] counts the stacks
/// whose figure is entirely its attachments, because a rig that failed to
/// compose would otherwise draw nothing at all and say nothing.
pub const RIG_RADIUS: f32 = 0.5;

/// Side of the unit-model cull grid, in world units. A stack is one anchor, so
/// this only decides how many draws a frame issues; 128 keeps a few provinces
/// per group.
pub const UNIT_TILE: f32 = 128.0;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// An uncertain later declaration must also refuse a known entity that uses
/// it as an attachment; baking the earlier definition would guess its content.
fn entity_refusal(roster: &mesh::Roster, name: &str) -> Option<String> {
    if roster.unreadable_names.is_empty() {
        return None;
    }
    let mut pending = vec![name];
    let mut seen = BTreeSet::new();
    while let Some(current) = pending.pop() {
        if !seen.insert(current) {
            continue;
        }
        if let Some(path) = roster.unreadable_names.get(current) {
            return Some(format!(
                "entity {name} reaches {current}, whose declaration is undecided after unreadable descriptor {path}"
            ));
        }
        if let Some(entity) = roster.descriptors.inherit(current) {
            pending.extend(
                entity
                    .attachments
                    .into_iter()
                    .map(|attachment| attachment.entity.as_str()),
            );
        }
    }
    None
}

// ------------------------------------------------------------------ the rule

/// The entity name one prefix and one key make.
///
/// An empty prefix is the engine's fourth and last level, the bare name.
pub fn entity_name(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        format!("{key}{ENTITY_SUFFIX}")
    } else {
        format!("{prefix}_{key}{ENTITY_SUFFIX}")
    }
}

/// The unnumbered keys the game tries, in order, for a sub-unit `key`
/// whose `sprite` field is `sprite`. A sub-unit whose sprite is its own name
/// contributes one key, not two.
pub fn entity_keys<'a>(key: &'a str, sprite: Option<&'a str>) -> Vec<&'a str> {
    let mut keys = vec![key];
    if let Some(sprite) = sprite
        && sprite != key
    {
        keys.push(sprite);
    }
    keys
}

/// Nonempty prefixes the game tries, followed by the bare level.
/// [`candidates`] keeps its unusual `_key_entity` empty-cosmetic lookup apart.
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

/// Every candidate name and the level it stands for, in the engine's own order:
/// key outermost, prefix innermost
/// once per key.
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
        // Append an underscore to the cosmetic tag unconditionally,
        // even when that string is empty.
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

/// Ordinary land fallback uses the roster owner. Transports prefer a
/// signed-positive expeditionary owner, otherwise the roster owner. The
/// logical owner remains the counter's grouping and visibility key.
fn model_owner(roster: u32, expeditionary: u32, transport: bool) -> u32 {
    if transport && (expeditionary as i32) > 0 {
        expeditionary
    } else {
        roster
    }
}

/// Prefix traversal uses operands lazily: a valid early hit does not depend on
/// missing later operands, but missing data never becomes a lookup miss.
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
    // The first key's cosmetic and tag are all that can be tried before an
    // unknown tag or culture. Its bare candidate and later keys depend on
    // proving that the unavailable prefix did not resolve.
    let name = format!("{cosmetic}_{key}{ENTITY_SUFFIX}");
    if lookup(&name) {
        return Some((Level::Cosmetic, name));
    }
    let tag = tag?;
    let name = entity_name(tag, key);
    lookup(&name).then_some((Level::Tag, name))
}

/// The signed-byte model number before the ordinary sub-unit and
/// sprite names. A numbered miss continues through the ordinary ladder; only
/// its last step may choose the placeholder entity.
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

/// The game uses the current cosmetic followed by the original country's tag
/// and graphical culture. It has only one key.
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

/// The entity the land ladder gives `country`'s [`INFANTRY_KEY`] sub-unit,
/// whose authored sprite is `sprite`: no model number, then the placeholder
/// as a division's complete miss takes it. `None` when a prefix the ladder
/// still has to try is unknown, or nothing, the placeholder included, is
/// registered.
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

/// The walking camera mode's figure ([`UnitModelAssets::infantry_figure`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Figure {
    pub entity: String,
    pub level: Level,
    /// The entity's slot in [`UnitModelAssets::library`].
    pub mesh: usize,
    /// How far the figure reaches above its own origin at the size the game
    /// draws its units, in world units.
    pub height: f32,
}

/// Which level of the ladder a stack's model came from, for the report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Saved explicit division-template model choice.
    Template,
    /// Saved explicit equipment-variant model choice.
    Variant,
    /// Authored graphical equipment pool.
    Graphics,
    /// Native conventional equipment candidate ranking.
    Conventional,
    /// `<cosmetic_tag>_<key>_entity`.
    Cosmetic,
    /// `<TAG>_<key>_entity`.
    Tag,
    /// `<culture>_<key>_entity`.
    Culture,
    /// `<key>_entity`.
    #[default]
    Bare,
    /// Nothing resolved; [`PLACEHOLDER_ENTITY`].
    Placeholder,
}

// -------------------------------------------------------------- this hour

/// What the layer drew, for the viewer to state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelReport {
    /// Stacks of this hour, whatever the camera.
    pub stacks: usize,
    pub ship_hulls: usize,
    /// How many resolved at each level of the ladder.
    pub by_level: BTreeMap<Level, usize>,
    /// Entity name to the number of stacks that took it, in ascending order.
    pub by_entity: BTreeMap<String, usize>,
    /// Stacks that took `<sprite>_entity` because the sub-unit has no entity
    /// of its own.
    pub sprite_key: usize,
    /// Stacks in naval transfer, drawn as [`TRANSPORT_KEY`].
    pub transports: usize,
    /// Stacks standing at a point along their route rather than on a province.
    pub moving: usize,
    /// Stacks omitted because this hour does not record their template.
    pub untemplated: usize,
    /// Stacks omitted because their country's model operands are unavailable.
    pub unknown_identity: usize,
    /// Stacks omitted for a missing input, by reason.
    pub selection_errors: BTreeMap<String, usize>,
    /// Why the whole 3D-model layer is off; `None` while it draws.
    pub disabled: Option<String>,
    /// Stacks whose province has no anchor at all.
    pub unanchored: usize,
    /// Stacks no division of which this perspective can see; the engine draws
    /// no model for them.
    pub unseen: usize,
    /// Stacks whose figure is composed of `attach` blocks as well as its own
    /// mesh.
    pub attached: usize,
    /// Stacks whose entity's **own** mesh is an attachment rig narrower than
    /// [`RIG_RADIUS`], so the whole figure is its attachments.
    pub rigs: usize,
    /// Ship formations not drawn because the hour does not record the operands of the mission
    /// skip ([`viewer_foundation::map::task_force_visual::MissionSkips`]) that would decide them.
    pub undecided_mission_formations: usize,
    /// Client RNG chooses support locators; hull identities and counts are derived.
    pub support_locator_rng: usize,
}

impl ModelReport {
    fn error(&mut self, reason: impl Into<String>) {
        *self.selection_errors.entry(reason.into()).or_default() += 1;
    }

    /// The stacks omitted for a missing or refused input, one diagnostics
    /// line per reason.
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
            .collect()
    }
}

/// Static model definitions and content rules. Mutable country identity and
/// explicit model choices are joined from the current replay frame.
///
/// The entity library bakes an entity the first time an hour places it, so
/// only entities the recording actually draws are held: baking every entity
/// any country could reach held gigabytes of geometry and textures.
pub struct UnitModelAssets {
    library: RwLock<ModelLibrary>,
    /// A staged field belongs to this library, whose mesh slots are append-only.
    field_identity: Arc<()>,
    /// Each name owns its bake, including an unsuccessful attempt. The map
    /// lock only locates these slots; cold loads never hold it.
    bakes: Mutex<BTreeMap<String, Arc<Mutex<bool>>>>,
    conventional: retained_bridge::RetainedBridge,
    roster: Arc<mesh::Roster>,
    /// The content the library bakes from.
    layers: ResolvedLayers,
    pub sub_units: model_selection::Content,
    pub gates: Gates,
    pub bounds: Arc<BTreeMap<u32, [f32; 4]>>,
    graphics: equipment_graphics::Content,
    geography: context::Content,
    camouflage: camouflage::Content,
    registered: HashSet<String>,
    /// The naval formation template and its scale.
    ships: ships::ShipAssets,
    /// Tags content declares after its dynamic-tag marker, which the fallback
    /// option labels read; why they did not read.
    dynamic_tags: Result<BTreeSet<String>, String>,
    /// Why the layer is off: a content form it refuses, or content it needs
    /// and lacks. Only this layer is off; the rest of the map draws.
    disabled: Option<String>,
}
debug_fields!(UnitModelAssets(this) {
    library, bakes, roster, sub_units, gates, bounds, graphics, geography, camouflage, ships, disabled
} {
    registered => viewer_foundation::debug_fields::sorted_set(&this.registered)
});

/// The hour's replay and matching content the layer reads.
pub struct PrepareInputs<'a> {
    pub frame: &'a DecodedFrame,
    pub limits: equipment_graphics::Limits<'a>,
    pub vision: &'a Vision,
    pub etas: &'a Etas,
    pub anchor: &'a dyn Fn(u32, u32) -> Option<[f32; 3]>,
    /// The route geometry the counter layer shares; `None` without one.
    pub world: Option<&'a counter_move::RouteWorld<'a>>,
    /// Each province's authored terrain, for camouflage conditions.
    pub terrain: &'a BTreeMap<u32, String>,
    /// Where each task force's counter stands, for its formation.
    pub ships: &'a ShipPlaces<'a>,
    /// The display language the fallback option labels read.
    pub localisation: &'a Localisation,
}

/// The stack an attempt to load the unit model layer reads ([`UnitModelAssets::load_with`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoadStack {
    /// The stack the layer was handed: the presentation stack while visual mods show.
    Shown,
    /// The recorded layers below its visual mods.
    Recorded,
}

/// The stable name the map's fallback compares when the unit model layer is off
/// ([`ResolvedLayers::note_refused`]).
pub const LAYER_OFF: &str = "model layer: unit models";

impl UnitModelAssets {
    /// Loads every entity any recorded country, template or variant could
    /// reach. A failure turns off this layer alone, with a warning; the map
    /// never refuses to open for it.
    ///
    /// `defines` are the graphics defines merged with the visual mods' scripts when `recorded`
    /// holds the recorded ones: a layer those cannot load (a unit distance cutoff a mod sets to
    /// a string) loads with the recorded defines instead, `refused` told why, as every graphics
    /// define falls back. A layer the visual mods' content breaks (their naval combat
    /// formations, a formation mesh) loads from the recorded layers below them, named the same
    /// way. A layer that still cannot load is off, and reported by [`LAYER_OFF`] for the map's
    /// fallback.
    #[allow(clippy::too_many_arguments)]
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

    /// [`Self::load`] with its content read by `try_load` from the stack it names; `below`
    /// whether recorded layers lie below visual mods on the shown stack. `layers` hears the
    /// layer's refusal when it is off.
    #[allow(clippy::too_many_arguments)]
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
            &mut Vec<installed_content_core::Diagnostic>,
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
        // Content the visual mods supply that the layer cannot load: the recording's own.
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

    /// The loaded layer, or the layer turned off with the reason.
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
            conventional: retained_bridge::RetainedBridge::default(),
            roster: Arc::new(mesh::Roster::default()),
            layers: ResolvedLayers::default(),
            sub_units: model_selection::Content::default(),
            gates: Gates::OFF,
            bounds: Default::default(),
            graphics: equipment_graphics::Content::default(),
            geography: context::Content::default(),
            camouflage: camouflage::Content::default(),
            registered: HashSet::new(),
            ships: ships::ShipAssets::default(),
            dynamic_tags: Err(reason.clone()),
            disabled: Some(reason),
        }
    }

    /// [`Self::load`]'s content read, or why the layer is off.
    pub fn try_load(
        layers: &ResolvedLayers,
        decode: Decode,
        defines: &Defines,
        map: &MapData,
        default_map: &DefaultMap,
        warnings: &mut Vec<installed_content_core::Diagnostic>,
    ) -> io::Result<Self> {
        let roster = layers.descriptor_roster()?;
        if !roster.unreadable.is_empty() {
            warnings.push((
                installed_content_core::DiagnosticKind::ParseFailure,
                format!(
                    "Unit model descriptors that do not parse even when closed are skipped: {}",
                    roster.unreadable.join(", ")
                ),
            ));
        }
        if !roster.logged.is_empty() {
            warnings.push((
                installed_content_core::DiagnosticKind::Logged,
                format!(
                    "Unit model entity registration diagnostics: {}",
                    roster.logged.join("; ")
                ),
            ));
        }
        let sub_units = model_selection::Content::load(layers)?;
        if !sub_units.dropped.is_empty() {
            warnings.push((
                installed_content_core::DiagnosticKind::Warning,
                format!(
                    "Sub-unit equipment needs the engine drops: {}",
                    sub_units.dropped.join("; ")
                ),
            ));
        }
        let mut name_warnings = Vec::new();
        let names = equipment_graphics::Names::read(layers, default_map, &mut name_warnings)?;
        warnings.extend(
            name_warnings
                .into_iter()
                .map(|message| (installed_content_core::DiagnosticKind::Warning, message)),
        );
        let graphics = equipment_graphics::Content::load(layers, &names)?;
        let geography = context::Content::load(layers, default_map, &map.definitions, defines)?;
        let camouflage = camouflage::Content::load(layers, defines)?;
        let mut ship_warnings = Vec::new();
        let ships = ships::ShipAssets::load(layers, &roster, &mut ship_warnings)?;
        warnings.extend(ship_warnings);
        Ok(Self {
            library: RwLock::new(ModelLibrary::empty(decode)),
            field_identity: Arc::new(()),
            bakes: Mutex::new(BTreeMap::new()),
            conventional: retained_bridge::RetainedBridge::default(),
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
            // An unreadable declaration is a candidate with an unknown native
            // definition. Select it and refuse it; do not guess a later model.
            registered: roster
                .descriptors
                .entities
                .keys()
                .chain(roster.unreadable_names.keys())
                .cloned()
                .collect(),
            ships,
            dynamic_tags: installed_content_core::country_tags::read_definitions(layers)
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

    /// What the prepared content refuses when a lookup meets it, each with
    /// the file it comes from when one does: acclimatization categories and sub-units
    /// that are one token under the lexer's fold.
    pub fn refusals(&self) -> Vec<(Option<String>, String)> {
        let mut refusals = Vec::new();
        refusals.extend(
            self.camouflage
                .refusals()
                .map(|why| (Some("common/acclimatation.txt".to_owned()), why)),
        );
        refusals.extend(self.sub_units.refusals().into_iter().map(|why| (None, why)));
        if let Err(why) = &self.dynamic_tags {
            refusals.push((
                None,
                format!("fallback model labels for dynamic creators refuse: {why}"),
            ));
        }
        refusals
    }

    /// Whether any `gfx` descriptor declares the entity.
    pub fn declares(&self, name: &str) -> bool {
        self.registered(name)
    }

    /// The entity the walking camera mode draws this hour for the viewing
    /// country, whose canonical slot is `ours` ([`resolve_infantry`]).
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

    /// [`Self::infantry_entity`], baked into [`Self::library`].
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

    /// Variant model selection for the aircraft-designer feature `0x26`.
    /// A saved model wins before graphics selection. When the equipment type
    /// carries an archetype sub-unit list, use its first sub-unit and `country`
    /// for the graphics branch; otherwise use only the saved model.
    /// `Ok(None)` lets the caller continue through its own ladder.
    /// `country` is a raw country reference.
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
        // Both model-selection paths return the saved name before graphics selection.
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
        // A selected pair never reaches the labelled fallback list.
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

    /// The complete airborne ladder, shared by conventional wings and raids.
    /// All pool context is the supplied country, rather than
    /// the variant creator used by the earlier designer branch.
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
        let definitions = frame
            .complete::<tables::VariantDefinitions>()
            .map_err(|error| error.to_string())?;
        let row = definitions
            .iter()
            .find(|row| row.id == variant)
            .ok_or_else(|| format!("equipment variant {variant} has no definition"))?;
        let [features] = frame
            .complete::<tables::SimulationFeatures>()
            .map_err(|error| error.to_string())?
        else {
            return Err("SimulationFeatures must have one complete row".into());
        };
        fn text(bytes: &[u8]) -> Result<&str, String> {
            installed_content_core::parser::text(bytes).map_err(|error| error.to_string())
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
        let designer = || self.aircraft_variant_model(frame, limits, variant, raw_country);
        let graphical = || {
            let key = key()?;
            let appearance = appearance()?;
            let culture = std::str::from_utf8(&appearance.graphical_culture)
                .map_err(|_| "aircraft graphical culture is not text")?;
            let links = frame
                .complete::<tables::CountryLinks>()
                .map_err(|error| error.to_string())?;
            let slot = slot()?;
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
            let geography = context::Prepared::read(frame, &self.geography)?;
            let geography = geography.context(raw_country)?;
            self.graphics.aircraft_override(
                &equipment_graphics::Context {
                    equipment: key,
                    archetype,
                    family,
                    subunit: None,
                    ideology,
                    culture: Some(culture),
                    country: geography.map(|country| country.country_tag.as_str()),
                    faction_tags: geography.map_or(&[], |country| country.faction_tags.as_slice()),
                    capital_continent: geography
                        .and_then(|country| country.capital_continent.as_deref()),
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

    /// The entities baked so far.
    pub fn library(&self) -> std::sync::RwLockReadGuard<'_, ModelLibrary> {
        self.library
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Bakes missing registered names in sorted batches. Warm names only read
    /// the library; cold batches wait only for names they share. A failed
    /// attempt is remembered just like a successful one.
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
        // Always claim names in the same order: overlapping batches can wait
        // for one another without a cycle, while disjoint batches load freely.
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
        // An entity the visual mods' descriptors leave undecided, though the recording's own do
        // not, is baked from the recording ([`Self::refusal`]).
        let (recorded, own): (Vec<String>, Vec<String>) =
            spans.measure("units.ensure.partition", |_| {
                missing
                    .iter()
                    .cloned()
                    .partition(|name| entity_refusal(&self.roster, name).is_some())
            });
        if let Ok(shard) = spans.measure("units.ensure.load_shown", |spans| {
            ModelLibrary::load_profiled(
                &self.layers,
                decode,
                &self.roster.descriptors,
                &own,
                &mut warnings,
                spans,
            )
        }) {
            let mut library = spans.measure("units.ensure.merge_shown_wait", |_| {
                self.library
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
            });
            spans.measure("units.ensure.merge_shown", |_| library.merge(shard));
        }
        if !recorded.is_empty()
            && let Ok(shard) = spans.measure("units.ensure.load_recorded", |spans| {
                ModelLibrary::load_recorded_profiled(
                    &self.layers,
                    decode,
                    &recorded,
                    |name| entity_refusal(&self.roster, name).unwrap_or_default(),
                    spans,
                )
            })
        {
            let mut library = spans.measure("units.ensure.merge_recorded_wait", |_| {
                self.library
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
            });
            spans.measure("units.ensure.merge_recorded", |_| library.merge(shard));
        }
        // An entity a visual mod supplies that does not bake was baked from the recording
        // instead, named, by the library load itself (`ModelLibrary::load`).
        spans.measure("units.ensure.complete", |_| {
            for complete in &mut completed {
                **complete = true;
            }
        });
    }

    /// Why `name` is refused ([`entity_refusal`]), `None` when it is drawn: an entity the
    /// visual mods' descriptors leave undecided, though the recording's own declare it
    /// readably, is drawn from the recording ([`ModelLibrary::load_recorded`]). A refusal is
    /// recorded by name for the map's fallback ([`ResolvedLayers::note_refused`]).
    fn refusal(&self, name: &str) -> Option<String> {
        let reason = entity_refusal(&self.roster, name)?;
        let recorded = self.layers.recorded_descriptor_roster().ok().flatten();
        if recorded.is_some_and(|recorded| {
            entity_refusal(&recorded, name).is_none()
                && recorded.descriptors.entities.contains_key(name)
        }) {
            return None;
        }
        self.layers.note_refused(format!("model entity {name}"));
        Some(reason)
    }

    /// A registered entity's slot, baking it first.
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

    /// Naval helpers share the current hour's recorder without changing their
    /// lazy model-selection demand or the ordinary non-builder entry point.
    fn bake_slot_for_hour(&self, name: &str, hour: Option<&Hour<'_>>) -> Option<usize> {
        match hour {
            Some(hour) => self.bake_slot_with_spans(name, &mut hour.spans.borrow_mut()),
            None => self.bake_slot(name),
        }
    }

    /// Places one figure per land stack of this hour.
    ///
    /// A stack is a (province, owner, route) triple, which is exactly how
    /// Counters group divisions into a counter, and the template is the
    /// oldest visible member's for the same reason, so a figure and its counter
    /// always stand together and show the same unit. Selection runs here, once
    /// an hour on the hour builder; `PreparedModels::field` only advances each
    /// figure's position and heading.
    pub fn prepare(&self, inputs: PrepareInputs<'_>) -> io::Result<PreparedModels> {
        self.prepare_profiled(inputs, &mut |_| {})
    }

    /// Builds the same hour, reporting coarse sequential parts to its builder.
    /// The caller owns clocks and reporting; ordinary preparation samples none.
    pub fn prepare_profiled(
        &self,
        inputs: PrepareInputs<'_>,
        lap: &mut dyn FnMut(&'static str),
    ) -> io::Result<PreparedModels> {
        self.prepare_with_spans(inputs, lap, &mut BuilderSpans::new(false))
    }

    /// The same preparation with job-local nested scopes on the built line.
    pub fn prepare_with_spans(
        &self,
        inputs: PrepareInputs<'_>,
        lap: &mut dyn FnMut(&'static str),
        spans: &mut BuilderSpans,
    ) -> io::Result<PreparedModels> {
        if let Some(reason) = &self.disabled {
            return Ok(PreparedModels {
                staged_field: Mutex::new(None),
                targets: None,
                raid: Vec::new(),
                convoys: None,
                placed: Vec::new(),
                ships: NavalSources::default(),
                owned: BTreeSet::new(),
                report: ModelReport {
                    disabled: Some(reason.clone()),
                    ..ModelReport::default()
                },
            });
        }
        let PrepareInputs {
            frame,
            limits,
            vision,
            etas,
            anchor,
            world,
            terrain,
            ships,
            localisation,
        } = inputs;
        let mut hour = Hour::read_profiled(self, frame, terrain, limits, localisation, lap)?;
        hour.spans = RefCell::new(spans.fork());
        hour.selection = Stages::new(spans.enabled());
        // The game reads the ordinary route count and movement-target pointer,
        //independently of the displayed route or selected animation state.
        let uv_moving: BTreeSet<_> = rows::<tables::DivisionMovement>(frame)?
            .rows()
            .iter()
            .filter(|r| r.path_total > 0 && r.target == 0)
            .map(|r| r.division)
            .collect();
        let movements =
            counter_move::DivisionRoutes::read(frame, etas, world.map(|world| world.map), &|id| {
                anchor(id, STANDSTILL)
            })?
            .by_division;
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
                combat_states.insert(*id, crate::map::entity_animation::ATTACK);
            }
            for id in &combat.defenders {
                combat_states.insert(*id, crate::map::entity_animation::DEFEND);
            }
        }
        let training: BTreeSet<u64> = hour
            .statuses
            .values()
            .filter(|r| r.exercising != 0)
            .map(|r| r.division)
            .collect();

        let mut stacks: BTreeMap<StackKey, Stack> = BTreeMap::new();
        for division in rows::<tables::Divisions>(frame)?.rows() {
            if division.province == 0 {
                continue;
            }
            let Some(slot) = hour.links.get(&division.logical_owner).copied() else {
                continue;
            };
            let route = movements.get(&division.id);
            let signature = route.and_then(Route::signature);
            let stack = stacks
                .entry((division.province, slot, signature))
                .or_insert_with(|| Stack {
                    // The oldest **visible** member supplies the template, for
                    // the reason counter grouping gives: a member the
                    // perspective never saw cannot be the one the map shows.
                    oldest: u64::MAX,
                    template: 0,
                    roster_owner: 0,
                    expeditionary_owner: 0,
                    visible: 0,
                    route: route.cloned(),
                });
            // The game compares the unit's visibility byte against 150,
            // which is [`Sight::None`]'s own threshold, so a division this
            // perspective cannot see has no model.
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
        let mut selector = Selector {
            assets: self,
            hour: &hour,
            frame,
            vision,
            graphical: BTreeMap::new(),
            fallbacks: BTreeMap::new(),
            view: None,
        };
        let mut templates = BTreeMap::new();
        // Selection first; the chosen entities are then baked in one batch.
        let mut chosen = Vec::new();
        for ((province, owner, _), stack) in &stacks {
            if stack.visible == 0 {
                report.unseen += 1;
                continue;
            }
            let transport = stack
                .route
                .as_ref()
                .is_some_and(|route| route.naval_transfer);
            let selection = if transport {
                report.transports += 1;
                selector.transport(stack)
            } else {
                selector.land(stack, &mut templates)
            };
            let (level, mut name, key) = match selection {
                Ok(Some(selected)) => selected,
                Ok(None) => {
                    report.unknown_identity += 1;
                    continue;
                }
                Err(Missing::Template) => {
                    report.untemplated += 1;
                    continue;
                }
                Err(Missing::Input(reason)) => {
                    report.error(reason);
                    continue;
                }
            };
            let base = name.clone();
            if !transport {
                let Some(status) = hour.statuses.get(&stack.oldest) else {
                    report.error("division acclimatization status is unavailable");
                    continue;
                };
                let camouflaged = hour.selection.measure(Stage::Camouflage, || {
                    hour.camouflage
                        .suffix(&self.camouflage, hour.sources.tokens, status, *province)
                        .map(|suffix| {
                            camouflage::entity(&name, suffix, |candidate| {
                                self.registered(candidate)
                            })
                        })
                });
                match camouflaged {
                    Ok(selected) => name = selected,
                    Err(error) => {
                        report.error(error.to_string());
                        continue;
                    }
                }
            }
            chosen.push((*province, *owner, stack, level, name, base, key, transport));
        }
        lap("um_selection");
        spans.append(hour.spans.replace(spans.fork()));
        hour.selection.append_to(spans);
        let names: Vec<String> = chosen.iter().map(|chosen| chosen.4.clone()).collect();
        self.ensure_with_spans(&names, spans);
        lap("um_ensure");
        let library = self.library();
        let mut placed = Vec::new();
        for (province, owner, stack, level, name, base, key, transport) in chosen {
            if let Some(reason) = self.refusal(&name) {
                report.error(reason);
                continue;
            }
            let Some(slot) = library.slot(&name) else {
                report.error(format!("registered entity {name} did not bake"));
                continue;
            };
            if !self.bounds.contains_key(&province) {
                report.error(format!(
                    "province {province} has no content bounds for model admission"
                ));
                continue;
            }
            if key
                .and_then(|key| self.sub_units.subunit(key))
                .is_some_and(|unit| {
                    unit.sprite != unit.key && base.ends_with(&entity_name("", &unit.sprite))
                })
            {
                report.sprite_key += 1;
            }
            let position = match &stack.route {
                Some(route) => route.position(province, world, anchor),
                None => anchor(province, STANDSTILL).map(|standing| counter_move::Position {
                    standing,
                    motion: None,
                }),
            };
            let Some(position) = position else {
                report.unanchored += 1;
                continue;
            };
            if position.motion.is_some() {
                report.moving += 1;
            }
            if let Some(mesh) = library.meshes.get(slot) {
                if mesh.own_radius < RIG_RADIUS {
                    report.rigs += 1;
                }
                if mesh.attachments > 0 {
                    report.attached += 1;
                }
            }
            // `mesh::Descriptors::resolve` applies the whole `clone` chain's
            // scale and `crate::map::models` bakes it into the vertices, so the only
            // instance scale left is the transport one.
            let scale = if transport {
                self.gates.transport_scale
            } else {
                1.0
            };
            *report.by_level.entry(level).or_default() += 1;
            *report.by_entity.entry(name).or_default() += 1;
            placed.push((
                Target {
                    province,
                    identity: stack.oldest,
                    order: stack
                        .route
                        .as_ref()
                        .map_or(STANDSTILL, |route| route.stack_kind),
                    occupant: stack.oldest,
                    part: 0,
                    kind: Kind::Land,
                    mesh: slot,
                    origin: position.standing,
                    instance: ModelInstance {
                        position: position.standing,
                        native_flags: 1 | (u32::from(uv_moving.contains(&stack.oldest)) << 1),
                        rotation: hash_rotation(province.wrapping_mul(2_654_435_761) ^ owner),
                        scale,
                        pitch: 0.0,
                        train_clip: [0.0; 4],
                        animation: animation_state(
                            movements.get(&stack.oldest),
                            combat_states.get(&stack.oldest).copied(),
                            training.contains(&stack.oldest),
                        ),
                        animation_seed: stack.oldest as u32 ^ (stack.oldest >> 32) as u32,
                        ..ModelInstance::default()
                    },
                },
                position,
            ));
        }
        drop(library);
        lap("um_targets");
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
        let ships = ships?;
        lap("um_naval");
        let owned = core::<tables::Provinces>(frame)?
            .iter()
            .filter(|row| hour.links.get(&row.owner) == Some(&vision.ours()))
            .map(|row| row.id)
            .collect();
        lap("um_owned");
        Ok(PreparedModels {
            staged_field: Mutex::new(None),
            targets: None,
            raid: Vec::new(),
            convoys: None,
            placed,
            ships,
            owned,
            report,
        })
    }
}

// ------------------------------------------------------------ the selection

/// One hour's joined tables. The tables a selection step needs but this hour
/// lacks are kept as their reason, so only the stacks that reach them fail.
struct Hour<'a> {
    links: BTreeMap<u32, u32>,
    countries: BTreeMap<u32, ModelCountry>,
    templates: BTreeMap<u64, &'a DivisionTemplateIconRow>,
    sources: model_selection::Sources<'a>,
    variants: BTreeMap<u64, &'a VariantDefinitionRow>,
    /// Spells the variants' equipment type tokens.
    tokens: TokenNames<'a>,
    equipment: Result<Equipment<'a>, String>,
    statuses: BTreeMap<u64, &'a replay_data::rows::DivisionStatusRow>,
    camouflage: camouflage::Context<'a>,
    spans: RefCell<BuilderSpans>,
    selection: Stages,
    classification: OnceCell<Option<model_selection::facts::Hour>>,
    /// Joined on first use: most hours need neither.
    geography: OnceCell<Result<context::Prepared, String>>,
    conventional: OnceCell<Result<bridge::Prepared<'a>, String>>,
    assets: &'a UnitModelAssets,
    frame: &'a DecodedFrame,
    limits: equipment_graphics::Limits<'a>,
    /// The fallback option list's labels.
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
        Self::read_profiled(assets, frame, terrain, limits, localisation, &mut |_| {})
    }

    fn read_profiled(
        assets: &'a UnitModelAssets,
        frame: &'a DecodedFrame,
        terrain: &'a BTreeMap<u32, String>,
        limits: equipment_graphics::Limits<'a>,
        localisation: &'a Localisation,
        lap: &mut dyn FnMut(&'static str),
    ) -> io::Result<Self> {
        let equipment = Equipment::read(frame)?;
        lap("um_equipment");
        Ok(Self {
            labels: model_labels::Labels::read(
                frame,
                localisation,
                assets.dynamic_tags.as_ref().map_err(Clone::clone),
            ),
            links: core::<tables::CountryLinks>(frame)?
                .iter()
                .map(|link| (link.id, link.canonical_slot))
                .collect(),
            countries: model_countries::read(frame)?,
            templates: rows::<tables::DivisionTemplateIcons>(frame)?
                .rows()
                .iter()
                .map(|row| (row.template, row))
                .collect(),
            sources: model_selection::Sources::read(frame)?,
            variants: rows::<tables::VariantDefinitions>(frame)?
                .rows()
                .iter()
                .map(|row| (row.id, row))
                .collect(),
            tokens: TokenNames::new(frame)?,
            equipment,
            statuses: rows::<tables::DivisionStatus>(frame)?
                .rows()
                .iter()
                .map(|row| (row.division, row))
                .collect(),
            camouflage: {
                lap("um_model_joins");
                let camouflage = camouflage::Context::read(frame, terrain)?;
                lap("um_camouflage");
                camouflage
            },
            spans: RefCell::new(BuilderSpans::new(false)),
            selection: Stages::new(false),
            classification: OnceCell::new(),
            geography: OnceCell::new(),
            conventional: OnceCell::new(),
            assets,
            frame,
            limits,
        })
    }

    fn geography(&self) -> Result<&context::Prepared, String> {
        self.geography
            .get_or_init(|| {
                self.spans
                    .borrow_mut()
                    .measure("units.selection.geography_read", |_| {
                        context::Prepared::read(self.frame, &self.assets.geography)
                    })
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    fn conventional(&self) -> Result<&bridge::Prepared<'a>, String> {
        self.conventional
            .get_or_init(|| {
                self.spans
                    .borrow_mut()
                    .measure("units.selection.bridge_read", |_| {
                        self.assets.conventional.read(self.frame, &self.assets.sub_units)
                    })
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    fn classification(&self) -> Option<&model_selection::facts::Hour> {
        self.classification
            .get_or_init(|| self.assets.sub_units.classification_facts(self.frame))
            .as_ref()
    }

    fn country(&self, raw: u32) -> Option<&ModelCountry> {
        self.links
            .get(&raw)
            .and_then(|slot| self.countries.get(slot))
    }
}

/// The variant-model graphics branch
/// for a (variant, sub-unit) pair: the authored graphic pools,
/// then the conventional candidates, for the country the game picks.
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
        // The equipment family is the type itself unless another is named.
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
            model_multi_fallback::creator_operands(pair.variant, &hour.links, &hour.countries)?;
        let geography = hour.geography()?.context(raw_country)?;
        let graphical = hour.selection.measure(Stage::GraphicsRules, || {
            assets.graphics.lookup_with_limits(
                &equipment_graphics::Context {
                    equipment: equipment_key,
                    archetype: Some(equipment.archetype),
                    family,
                    subunit: Some(&pair.subunit.key),
                    ideology,
                    culture,
                    country: geography.map(|country| country.country_tag.as_str()),
                    faction_tags: geography.map_or(&[], |country| country.faction_tags.as_slice()),
                    capital_continent: geography
                        .and_then(|country| country.capital_continent.as_deref()),
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

/// Every division's equipment rows: the daily census, joined with the hourly
/// rows of divisions minted since it. With both complete, a division in
/// neither holds no equipment; without the hourly table, a division absent
/// from the daily one is unknown. A division in both takes the hourly rows
/// alone: they are the newer capture, and its rows are never doubled.
struct Equipment<'a> {
    by_division: BTreeMap<u64, Vec<&'a replay_data::rows::DivisionEquipmentRow>>,
    new_complete: bool,
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
        let mut by_division: BTreeMap<u64, Vec<_>> = BTreeMap::new();
        for row in new {
            by_division.entry(row.division).or_default().push(row);
        }
        let hourly: BTreeSet<u64> = by_division.keys().copied().collect();
        for row in daily.iter().filter(|row| !hourly.contains(&row.division)) {
            by_division.entry(row.division).or_default().push(row);
        }
        Self {
            by_division,
            new_complete,
        }
    }

    fn of(&self, division: u64) -> Result<&[&'a replay_data::rows::DivisionEquipmentRow], String> {
        match self.by_division.get(&division) {
            Some(rows) => Ok(rows),
            None if self.new_complete => Ok(&[]),
            None => Err("division equipment unknown until the next daily refresh".to_owned()),
        }
    }
}

/// Why a stack has no figure.
enum Missing {
    /// This hour does not record the stack's template.
    Template,
    /// A recorded or authored input is missing.
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

/// The resolved entity, its level and the sub-unit whose key it came from.
/// `Ok(None)` is a country whose model operands are unavailable.
type Selected<'a> = Result<Option<(Level, String, Option<&'a str>)>, Missing>;

type GraphicalKey = (u64, String, Definition, u32);

/// Each template resolved against content once an hour.
type Templates<'a> = BTreeMap<u64, Result<Template<'a>, String>>;

/// Memoised selection over one hour: stacks sharing a pair or a fallback
/// share the answer.
struct Selector<'a, 'h> {
    assets: &'a UnitModelAssets,
    hour: &'h Hour<'a>,
    frame: &'a DecodedFrame,
    vision: &'a Vision,
    graphical: BTreeMap<GraphicalKey, Result<Option<(Level, String)>, String>>,
    fallbacks: BTreeMap<u64, Result<Option<model_multi_fallback::Resolved>, String>>,
    view: Option<Result<u32, String>>,
}

impl<'a> Selector<'a, '_> {
    fn exists(&self, name: &str) -> bool {
        self.assets.registered(name)
    }

    fn transport(&self, stack: &Stack) -> Selected<'a> {
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

    fn land(&mut self, stack: &Stack, templates: &mut Templates<'a>) -> Selected<'a> {
        let hour = self.hour;
        hour.selection.measure(Stage::Land, || {
            let template = *self
                .hour
                .templates
                .get(&stack.template)
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
            // The game ranks the whole native equipment list.
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
                &self.hour.variants,
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
            // The representative's numbered ladder; the null object
            // draws the placeholder.
            let Some(representative) = &composition.representative else {
                return self.placeholder();
            };
            self.numbered(stack, stack.roster_owner, representative.subunit)
        })
    }

    fn paired(
        &mut self,
        stack: &Stack,
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
                let resolved = self.graphical_model(pair, first, stack.roster_owner);
                self.graphical.insert(memo.clone(), resolved);
            }
            if let Some((level, name)) = self.graphical[&memo].clone()? {
                return Ok(Some((level, name, key)));
            }
            let country = model_selection::model_country(pair, stack.roster_owner);
            self.numbered(stack, country, pair.subunit)
        })
    }

    /// Variant-model selection for a selected pair.
    fn graphical_model(
        &self,
        pair: SelectedPair<'_>,
        first_archetype: Option<&str>,
        roster_owner: u32,
    ) -> Result<Option<(Level, String)>, String> {
        graphics_model(self.assets, self.hour, pair, first_archetype, roster_owner)
    }

    /// Fallback queues over the viewing country's rosters, once per template.
    fn fallback(
        &mut self,
        template: &DivisionTemplateIconRow,
        composition: &Template<'a>,
    ) -> Result<Option<model_multi_fallback::Resolved>, Missing> {
        let hour = self.hour;
        hour.selection.measure(Stage::Fallback, || {
            if !self.fallbacks.contains_key(&template.template) {
                let view = self
                    .view
                    .get_or_insert_with(|| {
                        model_multi_fallback::view_country_ref(self.frame, self.vision)
                    })
                    .clone();
                let resolved = view.and_then(|view| {
                    model_multi_fallback::resolve(&model_multi_fallback::Inputs {
                        template,
                        composition,
                        content: &self.assets.sub_units,
                        graphics: &self.assets.graphics,
                        geography: self.hour.geography()?,
                        conventional: self.hour.conventional()?,
                        countries: &self.hour.countries,
                        links: &self.hour.links,
                        definitions: &self.hour.variants,
                        tokens: &self.hour.tokens,
                        viewing_country_ref: view,
                        limits: &self.hour.limits,
                        registered: &self.assets.registered,
                        labels: &self.hour.labels,
                    })
                });
                self.fallbacks.insert(template.template, resolved);
            }
            Ok(self.fallbacks[&template.template].clone()?)
        })
    }

    /// The numbered model ladder with the stored number, for `country`.
    fn numbered(&self, stack: &Stack, country: u32, subunit: &'a SubUnit) -> Selected<'a> {
        let hour = self.hour;
        hour.selection.measure(Stage::Numbered, || {
            let Some(model_country) = self.hour.country(country) else {
                return Ok(None);
            };
            let status = self.hour.statuses.get(&stack.oldest).ok_or_else(|| {
                Missing::Input("division model number status is unavailable".into())
            })?;
            // The game sign-extends the stored number's low byte.
            let number = status.model_number as i8;
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

/// Selector results and spline geometry for one recorded hour and perspective.
pub struct PreparedModels {
    /// One builder-materialized field, moved into the first view without a clone.
    staged_field: Mutex<Option<StagedField>>,
    /// Keep the immutable target index shared with installs and repeated fields.
    targets: Option<Targets>,
    ships: NavalSources,
    /// The raid arrows' units ([`Self::add_raid_units`]). They are no unit
    /// slot's: each arrow creates its entity directly, so
    /// they bypass the unit history, its camera gates and popup scale.
    raid: Vec<RaidModel>,
    /// The viewed country's trade-route convoys ([`Self::add_convoys`]), posed each frame by
    /// the presentation clock ([`Self::convoy_units`]).
    convoys: Option<(
        Vec<crate::map::trade_routes::ConvoyLine>,
        crate::map::trade_routes::ConvoyDefines,
        std::sync::Mutex<Option<crate::map::trade_routes::ConvoyFleet>>,
    )>,
    placed: Vec<(Target, counter_move::Position)>,
    /// The viewing country's provinces, whose unit slots a switch to it
    /// clears ([`viewer_foundation::map::presentation::UnitRestart::ViewingCountry`]).
    owned: BTreeSet<u32>,
    pub report: ModelReport,
}
struct StagedField {
    assets: Arc<()>,
    field: Field,
}

impl PreparedModels {
    /// Materializes the upload field on the hour builder, after raids and convoys
    /// have joined it. Later views or scene recreations rebuild the same field.
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

    /// Adds the raid arrows' units, each entity baked by name, posed where
    /// the hour placed them ([`RaidModel::posed`]).
    pub fn add_raid_units(
        &mut self,
        assets: &UnitModelAssets,
        units: &[viewer_foundation::map::unit_pose::UnitPlacement],
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
                viewer_foundation::map::unit_pose::UnitPose {
                    position: unit.position,
                    facing: unit.facing,
                    progress: unit.progress,
                },
            );
            self.raid.push(RaidModel { instance, ..model });
        }
    }

    /// Raid units draw at every camera height. `playback` is the heightfield and the playback
    /// fraction a playing window places them at every rendered frame, the
    /// tick's progress read by movement updates each frame;
    /// `None` keeps them where the hour placed them.
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

    /// Adds the viewed country's trade-route convoys ([`crate::map::trade_routes`]). Like the
    /// raid units they are no unit slot's and bypass the unit history.
    pub fn add_convoys(
        &mut self,
        lines: Vec<crate::map::trade_routes::ConvoyLine>,
        defines: crate::map::trade_routes::ConvoyDefines,
    ) {
        self.clear_staged_field();
        self.convoys = (!lines.is_empty()).then_some((lines, defines, std::sync::Mutex::new(None)));
    }

    /// Preserve this view's sailing stream across unchanged forward-hour route refreshes.
    /// Seeks and viewing-country changes recreate presentation through the host's restart rule.
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

    /// A seek starts a fresh convoy presentation even when it reuses selector results.
    pub fn reset_convoys(&self) {
        if let Some((_, _, fleet)) = &self.convoys {
            *fleet
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        }
    }

    /// Whether the hour has convoys to sail.
    pub fn has_convoys(&self) -> bool {
        self.convoys.is_some()
    }

    /// The convoys sailing at `time` seconds of presentation time, when they show
    /// ([`crate::map::trade_routes::convoys_shown`]); `height` is the drawn surface at a world
    /// `(x, z)`. Entities that do not bake are counted in `refused`.
    pub fn convoy_units(
        &self,
        assets: &UnitModelAssets,
        time: f32,
        height: &dyn Fn(f32, f32) -> f32,
        shown: impl Fn(&crate::map::trade_routes::ConvoyDefines) -> bool,
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
            crate::map::trade_routes::ConvoyFleet::new(
                lines,
                defines,
                &crate::map::trade_routes::presentation_counter,
                &|name| radii.get(name).copied().flatten(),
                height,
            )
        });
        *refused.entry("convoy presentation history: native counter seed, cached positions and sleep state are not recorded; fresh viewer presentation".to_owned()).or_default() += lines.iter().map(|line|line.count as usize).sum::<usize>();
        let poses = fleet.advance(defines, time, height);
        crate::map::trade_routes::convoy_instances(assets, lines, &poses, refused)
    }

    /// Every figure and ship of the hour, whatever the camera: the unit
    /// history admits them ([`crate::map::unit_history`]); and the raid units
    /// where the hour placed them, which bypass it.
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
                .map(|(target, position)| {
                    let mut target = *target;
                    if let Some((at, heading)) =
                        position.motion.as_ref().map(|m| m.position_heading())
                    {
                        target.instance.position = at;
                        target.instance.rotation = heading + MESH_FORWARD_OFFSET;
                    }
                    target
                })
                .collect::<Vec<_>>()
                .into()
        });
        let raids = self.raid_units(assets, None);
        // The raid meshes are uploaded with the hour's, so they are resident.
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
        // Upload each convoy mesh without creating or stepping its sailing fleet.
        // These zero-scale residency instances never enter the unit history or draw
        // a convoy; the presentation frame supplies the sailing instances.
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

/// A raid arrow's unit: its entity, the instance the hour placed and what
/// places it again.
struct RaidModel {
    slot: usize,
    instance: ModelInstance,
    track: viewer_foundation::map::unit_pose::UnitTrack,
    stages: Vec<installed_content_raids::raids::RaidAnimationStage>,
}

impl RaidModel {
    /// The instance at `pose`. The transform follows the actor basis: the facing
    /// is the entity's local +Z (the basis rows are right,
    /// up, facing with world up), which [`crate::map::entity_animation::world_transform`] places at
    /// `atan2(-f.x, f.z)` about Y and `asin(-f.y)` about X, as `aircraft.rs`
    /// reads an aircraft's basis; the scale is uniform.
    ///
    /// The game requests `idle`; a reached stage the
    /// entity has replaces it, through its intro when the stage changes under
    /// a shown entity ([`crate::map::entity_animation::named_stage`]).
    fn posed(
        &self,
        assets: &UnitModelAssets,
        pose: viewer_foundation::map::unit_pose::UnitPose,
    ) -> ModelInstance {
        let [fx, fy, fz] = pose.facing;
        let has_state = |state: &str| assets.library().has_state(self.slot, state);
        let animation = viewer_foundation::map::unit_pose::animation_state(
            &self.stages,
            pose.progress,
            &has_state,
        )
        .map_or(crate::map::entity_animation::IDLE, |stage| {
            crate::map::entity_animation::named_stage(&stage.state, stage.intro.as_deref())
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

/// One division's route this hour, shared with the counter layer.
type Route = counter_move::DivisionRoute;

/// What one stack is keyed by: its province, its owner and the route it is on,
/// the same triple used to group a counter.
type StackKey = (u32, u32, Option<[u32; 2]>);

/// Slot 0 is idle/training, 19 idle, movement slots select
/// attack/support_attack before retreat/move. Slots 9/10 fall through to defend.
/// LandCombats supplies the side when format14 has no DivisionMovement row.
fn animation_state(route: Option<&Route>, combat: Option<u32>, training: bool) -> u32 {
    use crate::map::entity_animation::*;
    if let Some(route) = route {
        if matches!(route.stack_kind, 9 | 10) {
            return DEFEND;
        }
        if route.stack_kind == 19 {
            return IDLE;
        }
        if route.stack_kind == 0 {
            return if training { TRAINING } else { IDLE };
        }
        let side = match route.combat_role {
            1 => Some(ATTACK),
            2 => Some(DEFEND),
            _ => combat,
        };
        if let Some(side) = side {
            return if side == ATTACK && route.path.is_empty() {
                SUPPORT
            } else {
                side
            };
        }
        return if route.retreat { RETREAT } else { MOVE };
    }
    combat.unwrap_or(if training { TRAINING } else { IDLE })
}

/// One stack's representative division: the oldest, whose template names the
/// figure, exactly as the counter picks the one whose glyph the counter
/// draws.
struct Stack {
    /// Members this perspective is shown at all.
    visible: usize,
    oldest: u64,
    /// The oldest member's preferred template.
    template: u64,
    roster_owner: u32,
    expeditionary_owner: u32,
    route: Option<Route>,
}

// ------------------------------------------------------------------ geometry

// The heading is [`counter_move::Motion::position_heading`]'s: the game's
// `atan2f(tangent.z, tangent.x)` of the drawn segment. The world rotation that
// carries a mesh's local +X to that direction is the same angle, which is what
// `models::ModelInstance::rotation` is (`entity_animation::world_transform`: local +X goes to
// `(cos, 0, sin)`).

/// The turn from the engine's heading to [`ModelInstance::rotation`].
///
/// The engine's heading is `atan2f(tangent.z, tangent.x)`, so heading zero
/// points along world **+X**. `entity_animation::world_transform` turns a mesh's local +X to
/// `(cos r, 0, sin r)`, so a rotation of `r` points local +X along the same
/// direction a heading of `r` does --- but a unit mesh is not authored facing
/// local +X. `gfx/models/units/GER_infantry.mesh` is 5.46 wide in x and 1.74
/// deep in z, which is a figure with its arms out across x and its body across
/// z, and a render of the layer with every rotation forced to zero shows the
/// rifleman's front --- collar, buttons, belt --- toward the bottom of the
/// screen, which at these cameras is world **-Z**. Turning local -Z onto the
/// route's direction takes `atan2(dx, -dz)`, a quarter turn on from the
/// heading.
pub const MESH_FORWARD_OFFSET: f32 = std::f32::consts::FRAC_PI_2;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod aircraft_ladder_tests {
    use super::*;
    use replay_data::{
        rows::CountryAppearanceRow, rows::CountryLinkRow, rows::LexicalTokenRow,
        rows::SimulationFeatureRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::{frame::Metrics, frame::TableView};

    fn frame(dlc_mask: u32, saved: &[u8]) -> DecodedFrame {
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

    fn limits(values: &equipment_graphics::LimitValues) -> equipment_graphics::Limits<'_> {
        fn evaluate(
            _: &[installed_content_core::parser::Item],
            _: dashboard_text::triggers::Scopes,
        ) -> dashboard_text::triggers::Eval {
            panic!("a declared saved or cosmetic model never evaluates graphical pool limits")
        }
        equipment_graphics::Limits::new(values, &evaluate)
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
