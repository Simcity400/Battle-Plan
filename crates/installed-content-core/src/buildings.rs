//! `common/buildings/*.txt`: which building types the map draws, and where.
//!
//! Only the keys a map pass needs are kept. `show_on_map = X` is the slot
//! limit of the type's generated own-name spawn bucket; `show_on_map_meshes = N`
//! is the number of entity variants. `always_shown` exempts the type from the
//! fog gate; `need_detection` hides a foreign building until it is detected.
//! `supply_node` marks the hub type, whose slot is left to port or city
//! graphics. `drawn_at_distance` keeps an entity created below
//! `MAP_BUILDINGS_SHRINK_DISTANCE` alive above it; `is_port` marks a port
//! building, and `tags` feed raid building selectors.
//!
//! Spawn buckets are the names `map/buildings.txt` slots carry. One is generated
//! per building; explicit `spawn_points` declarations add more, and
//! `spawn_point` redirects a building into one of those. Entity names use the
//! *building* name, not the bucket name. Bucket references match under the
//! lexer's ASCII case fold ([`Buildings::slot`]).
//!
//! [`FactoryFlags`] keeps each type's `military_production`,
//! `naval_production`, `general_production` and `rocket_production`, which
//! decide the stored factory groups its levels count in.

use crate::key_names::{KeyNames, Point};
use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    io,
};

use crate::{
    ResolvedLayers,
    crt::{Scanned, scan_i32},
    fields::{invalid, key_str},
    modifiers,
    parser::{
        self, Value,
        native::{self, Shape},
    },
};

/// The prefix used to construct entity lookup names.
pub const ENTITY_PREFIX: &str = "building_";
/// The suffix of the destroyed variant.
pub const DESTROYED_SUFFIX: &str = "_destroyed";

/// One `buildings` entry, reduced to what the map draws.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BuildingType {
    pub name: String,
    /// The generated bucket's signed slot limit; absent means zero.
    pub show_on_map: Option<i32>,
    /// Signed entity variant count; absent means one.
    pub show_on_map_meshes: i32,
    pub always_shown: bool,
    pub drawn_at_distance: bool,
    /// Suppresses popup growth; absent declarations default to false.
    pub disable_grow_animation: bool,
    pub need_detection: bool,
    /// Tooltip operands: specialization names and the detecting intelligence lane.
    pub specializations: Vec<String>,
    /// Nonzero air-defense weights identify air-defense buildings.
    pub air_defence: i32,
    /// Signed land-fort weight; defaults to zero.
    pub land_fort: i32,
    /// Signed naval-fort weight; defaults to zero. A positive land or naval
    /// fort weight lets a game-wide fort level override replace the type's
    /// maximum level.
    pub naval_fort: i32,
    /// `infrastructure = yes`: the infrastructure map mode and its hover read
    /// the first such type in database order.
    pub infrastructure: bool,
    /// `level_cap = { shares_slots = yes }`: the type's maximum level is also
    /// limited by the state's free shared building slots.
    pub shares_slots: bool,
    /// `level_cap = { group_by = X }`: the type's level allowance counts every
    /// building of its group, in the state and in its provinces.
    pub group_by: Option<String>,
    /// `level_cap = { exclusive_with = ... }`: types whose presence in the
    /// location makes this type's maximum level zero.
    pub exclusive_with: Vec<String>,
    /// `icon_frame`: the one-based frame in the interface building strip used
    /// by the construction queue; zero when not declared.
    pub icon_frame: i32,
    pub detecting_intel_type: usize,
    pub tooltip_refused: Option<String>,
    pub supply_node: bool,
    pub is_port: bool,
    /// `naval_headquarter`: inclusion in the naval-HQ map icon's building list.
    pub naval_headquarter: bool,
    /// `only_costal`: its map icons take the coastal offsets.
    pub only_costal: bool,
    /// `specialization` tokens in authored order. The facility icon uses
    /// `GFX_mapicon_facility_` followed by the first token.
    pub specialization: Vec<String>,
    /// The type has a `building_<name>_destroyed` entity.
    pub has_destroyed_mesh: bool,
    /// The explicit bucket this type's slots carry. Only a declared explicit
    /// bucket that no building name occupies is kept.
    pub spawn_point: Option<String>,
    /// `level_cap = { state_max = N }`, normalized after each block; absent is zero.
    pub state_max: i32,
    /// `level_cap = { province_max = N }` normalized by the
    /// same validator; a positive cap makes it provincial.
    pub province_max: Option<u32>,
    /// `tags`, which raid building selectors match.
    pub tags: Vec<String>,
    /// Map site flags mark a slot's province as an air base, gun emplacement
    /// or rocket site when its bucket holds such a type.
    pub air_base: bool,
    pub gun_emplacement: bool,
    pub radar: bool,
    pub rocket_launch_capacity: i32,
    /// `local_resources_<resource> = X`, fixed5, in first-appearance order
    /// with repeats summed. Names outside the active resource registry are dropped.
    pub local_resources: Vec<(String, i64)>,
    /// Why the raid-only fields (`tags` to `local_resources`) could not be
    /// read. They then hold their defaults, the map fields stay usable, and
    /// a raid consumer refuses the type rather than read the defaults.
    pub raid_refused: Option<String>,
}

impl BuildingType {
    /// The bucket name this type's rows carry in `map/buildings.txt`.
    pub fn slot_name(&self) -> &str {
        self.spawn_point.as_deref().unwrap_or(&self.name)
    }

    /// Whether `level_cap` scopes the type to provinces.
    pub fn provincial(&self) -> bool {
        self.province_max.is_some_and(|maximum| maximum > 0)
    }

    /// The positive province cap, then the positive state cap, otherwise the
    /// exact signed content define.
    pub fn base_maximum(&self, defines: &crate::defines::Defines) -> io::Result<i32> {
        let province_max = self.province_max.unwrap_or(0) as i32;
        if province_max > 0 {
            Ok(province_max)
        } else if self.state_max > 0 {
            Ok(self.state_max)
        } else {
            defines.int32("NBuildings", "MAX_BUILDING_LEVELS")
        }
    }

    /// The entity names of the variants, in variant order:
    /// `building_<type>` for one, `building_<type>_<k>` for `k` in `1..=n`,
    /// and none for a non-positive count.
    pub fn entity_names(&self) -> Vec<String> {
        match self.show_on_map_meshes {
            n if n <= 0 => Vec::new(),
            1 => vec![format!("{ENTITY_PREFIX}{}", self.name)],
            n => (1..=n)
                .map(|k| format!("{ENTITY_PREFIX}{}_{k}", self.name))
                .collect(),
        }
    }

    pub fn destroyed_entity_name(&self) -> String {
        format!("{ENTITY_PREFIX}{}{DESTROYED_SUFFIX}", self.name)
    }

    /// floor(meshes * level / NBuildings.MAX_BUILDING_LEVELS), capped at the
    /// last mesh.
    pub fn variant_for_level(&self, level: i32, max_building_levels: i32) -> usize {
        let n = self.show_on_map_meshes.max(1) as u32;
        ((n as f32 * (level.max(0) as f32 / max_building_levels as f32)) as u32).min(n - 1) as usize
    }
}

/// A native map spawn bucket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnPoint {
    pub name: String,
    /// Signed slot limit.
    pub max: i32,
    /// Signed mesh count; a non-positive count creates no candidates.
    pub meshes: i32,
    /// Province scope when true; state scope otherwise.
    pub provincial: bool,
    /// Port status is explicit or inherited from a redirected
    /// `is_port` type.
    pub is_port: bool,
}

/// Every building type and spawn bucket under `common/buildings`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Buildings {
    pub types: BTreeMap<String, BuildingType>,
    pub spawn_points: BTreeMap<String, SpawnPoint>,
    /// Types in database order: filename order, then authored block order.
    pub declaration_order: Vec<String>,
    /// Buckets a differing repeated type or bucket declaration touches. The
    /// native database keeps every repeat as a record; which one a slot uses
    /// is unverified, so a consumer refuses these buckets.
    pub ambiguous_slots: BTreeSet<String>,
    /// Every declaration, in database order and as authored (before redirects resolve), of
    /// each type declared more than once with differing fields. `types` keeps only the last,
    /// but the native database keeps each as its own record, so which record a lookup by
    /// name or by list position reaches is unverified: a consumer refuses these types by
    /// name, including one whose last declaration drops a classification an earlier one had.
    pub repeated_types: BTreeMap<String, Vec<BuildingType>>,
    /// Each bucket name under the lexer's fold, to the bucket a reference spelled so
    /// reaches ([`Self::slot`]).
    pub buckets: BTreeMap<String, String>,
    /// Each type's factory group flags, by name, from its last declaration.
    pub factory_flags: BTreeMap<String, FactoryFlags>,
    /// Types declared again with different factory flags. Which record a
    /// building uses is unverified, so a consumer refuses these types.
    pub ambiguous_factory_types: BTreeSet<String>,
}

/// Flags that put a building type's levels into the country's factory groups.
/// Each is a signed i32; a building counts in every group whose flag is positive.
/// Absent flags are zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FactoryFlags {
    pub military_production: i32,
    pub naval_production: i32,
    pub general_production: i32,
    pub rocket_production: i32,
}

impl FactoryFlags {
    /// Counted in the military factory group.
    pub fn military(&self) -> bool {
        self.military_production > 0
    }
    /// Counted in the naval dockyard group.
    pub fn naval(&self) -> bool {
        self.naval_production > 0
    }
    /// Counted in the civilian factory group.
    pub fn civilian(&self) -> bool {
        self.general_production > 0
    }
    /// Counted in the fourth stored group, independently of rocket launch capacity.
    pub fn rocket(&self) -> bool {
        self.rocket_production > 0
    }
}

/// How a `map/buildings.txt` slot name resolves ([`Buildings::slot`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot<'a> {
    Bucket(&'a SpawnPoint),
    /// The bucket reached is in [`Buildings::ambiguous_slots`]: a differing repeated
    /// declaration reaches it, so which record decides it is unverified and consumers refuse
    /// the slot.
    Ambiguous(&'a SpawnPoint),
}

impl<'a> Slot<'a> {
    /// The bucket the name reaches, ambiguous or not.
    pub fn spawn(self) -> &'a SpawnPoint {
        match self {
            Slot::Bucket(spawn) | Slot::Ambiguous(spawn) => spawn,
        }
    }
}

impl Buildings {
    /// The type the infrastructure map mode and its hover read: the first type
    /// in database order declared `infrastructure = yes`. A type declared more
    /// than once with differing fields, one of which declares it so, is
    /// refused by name: which of its records comes first is unverified.
    pub fn infrastructure(&self) -> Result<Option<&BuildingType>, String> {
        for name in &self.declaration_order {
            if let Some(repeats) = self.repeated_types.get(name)
                && repeats.iter().any(|building| building.infrastructure)
            {
                return Err(format!(
                    "building type {name} is declared {} times with differing fields, and one declares it infrastructure",
                    repeats.len()
                ));
            }
            if let Some(building) = self.types.get(name)
                && building.infrastructure
            {
                return Ok(Some(building));
            }
        }
        Ok(None)
    }

    /// A type named by an ASCII-folded lexer token, keeping its declared
    /// spelling. Differing declarations of one token are refused by name,
    /// as with the bucket resolver.
    pub fn by_token(&self, token: &str) -> io::Result<Option<&BuildingType>> {
        let token_fold = fold(token);
        let mut matches = self
            .declaration_order
            .iter()
            .filter(|name| fold(name) == token_fold)
            .filter_map(|name| self.types.get(name));
        let Some(first) = matches.next() else {
            return Ok(None);
        };
        if self
            .repeated_types
            .keys()
            .any(|name| fold(name) == token_fold)
            || matches.any(|later| !same_type(first, later))
        {
            return Err(invalid(format!(
                "building type {token} is declared more than once with differing fields"
            )));
        }
        Ok(Some(first))
    }

    /// The shared resolver for slot names and bucket references. Bucket names use
    /// FNV-1a over ASCII-folded bytes and compare with that same fold, so equal
    /// folded names share a token. Lookup order is the null object, generated
    /// buckets in template order, then explicit buckets. An explicit bucket
    /// sharing a building token is discarded. A slot selects the first matching
    /// hash and folded name: the first generated bucket, otherwise the surviving
    /// explicit bucket, or the null object when no name matches.
    pub fn slot(&self, name: &str) -> Option<Slot<'_>> {
        let bucket = self.buckets.get(&fold(name))?;
        let spawn = self.spawn_points.get(bucket)?;
        Some(if self.ambiguous_slots.contains(bucket) {
            Slot::Ambiguous(spawn)
        } else {
            Slot::Bucket(spawn)
        })
    }

    /// A bucket's candidates in database order; drawing takes the first with a
    /// positive level. Each type enters its generated bucket and follows its redirect.
    pub fn slot_candidates<'a>(&'a self, slot: &str) -> impl Iterator<Item = &'a BuildingType> {
        self.declaration_order
            .iter()
            .filter_map(|name| self.types.get(name))
            .filter(move |building| building.name == slot || building.slot_name() == slot)
    }
}

/// Parses one file without a resource registry. Resource-bearing definitions
/// require a resource registry so unknown resource names can be skipped.
/// Its triggers are read against `names`.
pub fn parse(input: &[u8], names: &KeyNames) -> io::Result<Buildings> {
    parse_with_registry(input, None, names)
}

#[cfg(test)]
pub fn parse_with_resources(
    input: &[u8],
    resources: &[crate::resources::Resource],
    names: &KeyNames,
) -> io::Result<Buildings> {
    parse_with_registry(input, Some(resources), names)
}

fn parse_with_registry(
    input: &[u8],
    resources: Option<&[crate::resources::Resource]>,
    names: &KeyNames,
) -> io::Result<Buildings> {
    let mut buildings = parse_declarations(input, resources, names)?;
    resolve(&mut buildings);
    Ok(buildings)
}

#[derive(Clone, Copy)]
enum Node {
    File,
    Buildings,
    Building,
    Spawns,
    Spawn,
    Cap,
    CountryModifier,
    Speed,
    Prerequisite,
    Dependency,
    Localization,
    Modifier(modifiers::Node),
}

/// Reads building and spawn-point fields with persistent object semantics.
fn parse_declarations(
    input: &[u8],
    resources: Option<&[crate::resources::Resource]>,
    names: &KeyNames,
) -> io::Result<Buildings> {
    use Node::*;
    let mut undecided = Vec::new();
    let mut refused = Vec::new();
    // Each file's lexer uses mode 0.
    let document = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(File),
        Some(names),
        &mut |node, field| {
            match (node, field.keyword().as_deref()) {
                (File, Some(b"buildings")) => Shape::Object(Buildings),
                (File, Some(b"spawn_points")) => Shape::Object(Spawns),
                (Buildings, _) => Shape::Object(Building),
                (Spawns, _) => Shape::Object(Spawn),
                (Building, Some(b"country_modifiers")) => Shape::Object(CountryModifier),
                (Building, Some(b"state_modifiers")) | (CountryModifier, Some(b"modifiers")) => {
                    Shape::Object(Modifier(modifiers::Node::Modifier))
                }
                (Building, Some(b"construction_speed_factor")) => Shape::Object(Speed),
                (Speed, Some(b"trigger")) | (Building, Some(b"dlc_allowed")) => Shape::Trigger,
                (Speed, Some(b"factor")) => Shape::One,
                (Building, Some(b"level_cap")) => Shape::Object(Cap),
                // Lists read whether or not the value is `{`: tags and
                // specialization and modules take the
                // lexer's tokens, leaving the value slot; the damage modifiers
                // and enable_for_controllers
                // resolve each into it.
                (Building, Some(b"tags" | b"specialization" | b"modules")) => Shape::RawList,
                (Building, Some(b"province_damage_modifiers" | b"state_damage_modifier"))
                | (CountryModifier, Some(b"enable_for_controllers")) => Shape::List,
                (Building, Some(b"building_prerequisite")) => Shape::ObjectIfOpen(Prerequisite),
                (Prerequisite, Some(b"building_dependency")) => Shape::ObjectIfOpen(Dependency),
                (Dependency, Some(b"building" | b"level")) => Shape::One,
                (Cap, Some(b"exclusive_with")) => {
                    if field.opens() {
                        Shape::List
                    } else {
                        Shape::One
                    }
                }
                (Cap, Some(b"state_max" | b"province_max" | b"group_by" | b"shares_slots")) => {
                    Shape::One
                }
                // Unknown cap and country-modifier fields are reported and their blocks skipped.
                (Cap | CountryModifier, _) => Shape::Skip,
                (Spawn, Some(b"type")) => match field.value_keyword().as_deref() {
                    Some(b"province" | b"state") => Shape::One,
                    _ => Shape::Skip,
                },
                (Spawn, _) => Shape::One,
                (Building, Some(b"detecting_intel_type")) => match field.value_keyword().as_deref()
                {
                    Some(b"civilian" | b"army" | b"navy" | b"airforce") => Shape::One,
                    _ => Shape::Skip,
                },
                // Bound localisation is recursive. `localization_key` reads one C-string
                // value, even an opening brace; other keys bind values.
                (Building, Some(b"missing_tech_loc")) => Shape::ObjectIfOpen(Localization),
                (Localization, Some(b"localization_key")) => Shape::One,
                (Localization, _) => Shape::ObjectIfOpen(Localization),
                (
                    Building,
                    Some(
                        b"spawn_point"
                        | b"value"
                        | b"is_buildable"
                        | b"air_defence"
                        | b"base_cost"
                        | b"only_costal"
                        | b"show_on_map"
                        | b"is_port"
                        | b"icon_frame"
                        | b"naval_production"
                        | b"military_production"
                        | b"general_production"
                        | b"anti_air"
                        | b"show_on_map_meshes"
                        | b"radar"
                        | b"air_base"
                        | b"infrastructure"
                        | b"disable_grow_animation"
                        | b"gun_emplacement"
                        | b"naval_headquarter"
                        | b"naval_supply_hub"
                        | b"per_controlled_building_extra_cost"
                        | b"drawn_at_distance"
                        | b"need_supply"
                        | b"allied_build"
                        | b"fuel_silo"
                        | b"disabled_in_dmz"
                        | b"rocket_production"
                        | b"naval_fort"
                        | b"land_fort"
                        | b"refinery"
                        | b"nuclear_reactor"
                        | b"damage_factor"
                        | b"rocket_launch_capacity"
                        | b"per_level_extra_cost"
                        | b"always_shown"
                        | b"base_cost_conversion"
                        | b"has_destroyed_mesh"
                        | b"infrastructure_construction_effect"
                        | b"show_modifier"
                        | b"hide_if_missing_tech"
                        | b"repair_speed_factor"
                        | b"affects_energy"
                        | b"only_display_if_exists"
                        | b"special_icon"
                        | b"disable_auto_nudging"
                        | b"need_detection"
                        | b"supply_node"
                        | b"centered",
                    ),
                ) => Shape::One,
                (Building, _) => {
                    // Resource names use the raw C-string prefix even for quoted keys;
                    // resource lookup is case-insensitive.
                    if let Some(name) =
                        parser::c_string(&field.key.text).strip_prefix(b"local_resources_")
                    {
                        match resources {
                            Some(registry)
                                if registry
                                    .iter()
                                    .any(|r| r.token.as_bytes().eq_ignore_ascii_case(name)) =>
                            {
                                Shape::One
                            }
                            Some(_) => Shape::Skip,
                            None => {
                                refused.push(format!(
                                    "building local_resources_{}: resource registry required",
                                    String::from_utf8_lossy(name)
                                ));
                                Shape::Skip
                            }
                        }
                    } else {
                        Shape::Skip
                    }
                }
                (Modifier(node), _) => {
                    modifiers::shape(node, field, modifiers::UNKNOWN, &mut undecided).map(Modifier)
                }
                _ => Shape::Skip,
            }
        },
    );
    if let Some(cause) = document.unsettled {
        return Err(invalid(format!("buildings: {cause}")));
    }
    if !undecided.is_empty() || !refused.is_empty() {
        return Err(invalid(format!(
            "buildings ({} unsupported forms): {}{}",
            undecided.len() + refused.len(),
            undecided
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
            refused.join("; ")
        )));
    }
    let mut buildings = self::Buildings::default();
    for root in document.items {
        if root.keyword_is("buildings") {
            for item in root.value.items().unwrap_or_default() {
                let name = key_str(item, "building")?.to_owned();
                let mut building = BuildingType {
                    name,
                    show_on_map_meshes: 1,
                    ..Default::default()
                };
                let mut flags = FactoryFlags::default();
                for field in item.value.items().unwrap_or_default() {
                    let what = format!(
                        "building {} field {}",
                        building.name,
                        field.key_str().unwrap_or_default()
                    );
                    let bytes = field.value.scalar().unwrap_or_default();
                    match field.keyword().map(|k| k.folded()).as_deref() {
                        Some(b"show_on_map") => {
                            let mut value = building.show_on_map.unwrap_or(0);
                            integer(bytes, &mut value);
                            building.show_on_map = Some(value);
                        }
                        Some(b"show_on_map_meshes") => {
                            integer(bytes, &mut building.show_on_map_meshes)
                        }
                        Some(b"always_shown") => boolean(bytes, &mut building.always_shown),
                        Some(b"drawn_at_distance") => {
                            boolean(bytes, &mut building.drawn_at_distance)
                        }
                        Some(b"disable_grow_animation") => {
                            boolean(bytes, &mut building.disable_grow_animation)
                        }
                        Some(b"need_detection") => boolean(bytes, &mut building.need_detection),
                        Some(b"supply_node") => boolean(bytes, &mut building.supply_node),
                        Some(b"is_port") => boolean(bytes, &mut building.is_port),
                        Some(b"naval_headquarter") => {
                            boolean(bytes, &mut building.naval_headquarter)
                        }
                        Some(b"only_costal") => boolean(bytes, &mut building.only_costal),
                        Some(b"has_destroyed_mesh") => {
                            boolean(bytes, &mut building.has_destroyed_mesh)
                        }
                        Some(b"spawn_point") => {
                            building.spawn_point = Some(
                                field
                                    .value
                                    .read_str()
                                    .map_err(|e| invalid(format!("{what}: {e}")))?
                                    .into(),
                            )
                        }
                        Some(b"level_cap") => {
                            let mut province_max = building.province_max.map(|value| value as i32);
                            for cap in field.value.items().unwrap_or_default() {
                                let bytes = cap.value.scalar().unwrap_or_default();
                                match cap.keyword().map(|k| k.folded()).as_deref() {
                                    Some(b"state_max") => integer(bytes, &mut building.state_max),
                                    Some(b"province_max") => {
                                        integer(bytes, province_max.get_or_insert(0))
                                    }
                                    Some(b"shares_slots") => {
                                        boolean(bytes, &mut building.shares_slots)
                                    }
                                    Some(b"group_by") => {
                                        if let Ok(group) = cap.value.c_str() {
                                            building.group_by = Some(group.to_owned());
                                        }
                                    }
                                    Some(b"exclusive_with") => match cap.value.items() {
                                        Some(items) => {
                                            building.exclusive_with.extend(items.iter().filter_map(
                                                |item| item.value.c_str().ok().map(str::to_owned),
                                            ))
                                        }
                                        None => building
                                            .exclusive_with
                                            .extend(cap.value.c_str().ok().map(str::to_owned)),
                                    },
                                    _ => {}
                                }
                            }
                            // Validate after each block, before a later block can retain these values
                            // through a failed integer scan.
                            building.state_max = building.state_max.max(0);
                            building.province_max = province_max.map(|value| {
                                let value = value.max(0);
                                if building.state_max > 0 && value > building.state_max {
                                    0
                                } else {
                                    value as u32
                                }
                            });
                        }
                        Some(b"air_defence") => integer(bytes, &mut building.air_defence),
                        Some(b"icon_frame") => integer(bytes, &mut building.icon_frame),
                        // An unrepresentable name refuses only the part that reads it, preserving
                        // map fields. Each specialization field appends its names.
                        Some(b"specialization") => match strings(&field.value, &what) {
                            Ok(names) => {
                                building.specialization.extend(names.iter().cloned());
                                building.specializations.extend(names);
                            }
                            Err(error) => building.tooltip_refused = Some(error.to_string()),
                        },
                        Some(b"detecting_intel_type") => {
                            if let Some(lane) =
                                match field.value_keyword().as_deref().map(str::as_bytes) {
                                    Some(b"civilian") => Some(0),
                                    Some(b"army") => Some(1),
                                    Some(b"navy") => Some(2),
                                    Some(b"airforce") => Some(3),
                                    _ => None,
                                }
                            {
                                building.detecting_intel_type = lane;
                            }
                        }
                        Some(b"military_production") => {
                            integer(bytes, &mut flags.military_production)
                        }
                        Some(b"naval_production") => integer(bytes, &mut flags.naval_production),
                        Some(b"general_production") => {
                            integer(bytes, &mut flags.general_production)
                        }
                        Some(b"land_fort") => integer(bytes, &mut building.land_fort),
                        Some(b"naval_fort") => integer(bytes, &mut building.naval_fort),
                        Some(b"infrastructure") => boolean(bytes, &mut building.infrastructure),
                        Some(b"rocket_production") => integer(bytes, &mut flags.rocket_production),
                        Some(b"air_base") => boolean(bytes, &mut building.air_base),
                        Some(b"gun_emplacement") => boolean(bytes, &mut building.gun_emplacement),
                        Some(b"radar") => boolean(bytes, &mut building.radar),
                        Some(b"rocket_launch_capacity") => {
                            integer(bytes, &mut building.rocket_launch_capacity)
                        }
                        Some(b"tags") => match strings(&field.value, &what) {
                            Ok(tags) => building.tags.extend(tags),
                            Err(error) => building.raid_refused = Some(error.to_string()),
                        },
                        _ => {
                            if let Some(name) = field
                                .key
                                .as_deref()
                                .map(parser::c_string)
                                .and_then(|k| k.strip_prefix(b"local_resources_"))
                                && let Some(resource) = resources
                                    .unwrap_or_default()
                                    .iter()
                                    .find(|r| r.token.as_bytes().eq_ignore_ascii_case(name))
                            {
                                let amount = crate::fixed5::script_fixed5(bytes).unwrap_or(0);
                                if let Some((_, total)) = building
                                    .local_resources
                                    .iter_mut()
                                    .find(|(name, _)| name == &resource.token)
                                {
                                    *total = total.wrapping_add(amount);
                                } else {
                                    building
                                        .local_resources
                                        .push((resource.token.clone(), amount));
                                }
                            }
                        }
                    }
                }
                if buildings
                    .factory_flags
                    .insert(building.name.clone(), flags)
                    .is_some_and(|earlier| earlier != flags)
                {
                    buildings
                        .ambiguous_factory_types
                        .insert(building.name.clone());
                }
                match buildings.types.get(&building.name) {
                    Some(earlier) => {
                        mark_repeat(&mut buildings.ambiguous_slots, earlier, &building);
                        if earlier != &building {
                            buildings
                                .repeated_types
                                .entry(building.name.clone())
                                .or_insert_with(|| vec![earlier.clone()])
                                .push(building.clone());
                        }
                    }
                    None => buildings.declaration_order.push(building.name.clone()),
                }
                buildings.types.insert(building.name.clone(), building);
            }
        } else if root.keyword_is("spawn_points") {
            for item in root.value.items().unwrap_or_default() {
                let name = key_str(item, "spawn point")?.to_owned();
                let mut spawn = SpawnPoint {
                    name,
                    max: 1,
                    meshes: 1,
                    provincial: false,
                    is_port: false,
                };
                for field in item.value.items().unwrap_or_default() {
                    let bytes = field.value.scalar().unwrap_or_default();
                    match field.keyword().map(|k| k.folded()).as_deref() {
                        Some(b"max") => integer(bytes, &mut spawn.max),
                        Some(b"show_on_map_meshes") => integer(bytes, &mut spawn.meshes),
                        Some(b"is_port") => boolean(bytes, &mut spawn.is_port),
                        Some(b"type") => {
                            match field.value_keyword().as_deref().map(str::as_bytes) {
                                Some(b"province") => spawn.provincial = true,
                                Some(b"state") => spawn.provincial = false,
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
                insert_spawn(
                    &mut buildings.spawn_points,
                    &mut buildings.ambiguous_slots,
                    spawn,
                );
            }
        }
    }
    // An empty replacement contributes no declarations; only an entirely
    // empty database is refused by `read`.
    Ok(buildings)
}

// Failed scans keep the stored value.
fn integer(bytes: &[u8], value: &mut i32) {
    if let Scanned::Stored(next) = scan_i32(bytes) {
        *value = next;
    }
}
fn boolean(bytes: &[u8], value: &mut bool) {
    if let Some(next) = parser::read_bool(bytes) {
        *value = next;
    }
}
fn strings(value: &Value, what: &str) -> io::Result<Vec<String>> {
    value
        .items()
        .unwrap_or_default()
        .iter()
        .map(|item| {
            item.value
                .c_str()
                .map(str::to_owned)
                .map_err(|e| invalid(format!("{what}: {e}")))
        })
        .collect()
}

/// A name as the lexer identifies it: only ASCII A-Z fold.
fn fold(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// Case variants of one token may keep different spellings of the same fields.
fn same_type(earlier: &BuildingType, later: &BuildingType) -> bool {
    let unnamed = |kind: &BuildingType| BuildingType {
        name: String::new(),
        ..kind.clone()
    };
    unnamed(earlier) == unnamed(later)
}

/// Adds an explicit bucket declaration. An earlier one equal under the lexer's fold is the
/// same token: a differing repeat, spelling included, makes both spellings ambiguous, and
/// the later one is kept.
fn insert_spawn(
    spawns: &mut BTreeMap<String, SpawnPoint>,
    ambiguous: &mut BTreeSet<String>,
    spawn: SpawnPoint,
) {
    let earlier = spawns
        .keys()
        .find(|name| name.eq_ignore_ascii_case(&spawn.name))
        .cloned();
    if let Some(earlier) = earlier.and_then(|name| spawns.remove(&name))
        && earlier != spawn
    {
        ambiguous.insert(earlier.name);
        ambiguous.insert(spawn.name.clone());
    }
    spawns.insert(spawn.name.clone(), spawn);
}

/// A differing repeat makes every bucket either declaration reaches ambiguous.
fn mark_repeat(ambiguous: &mut BTreeSet<String>, earlier: &BuildingType, later: &BuildingType) {
    if earlier != later {
        ambiguous.insert(later.name.clone());
        ambiguous.extend(
            [&earlier.spawn_point, &later.spawn_point]
                .into_iter()
                .flatten()
                .cloned(),
        );
    }
}

/// Explicit buckets sharing a building token are dropped. Redirects bind by
/// token to the remaining explicit buckets, keeping their declared spelling,
/// and every building gets its generated bucket. A redirected `is_port` type
/// marks its bucket as a port. Ambiguous names are stored as the bucket they reach.
fn resolve(buildings: &mut Buildings) {
    let Buildings {
        types,
        spawn_points,
        declaration_order,
        ambiguous_slots,
        buckets,
        ..
    } = buildings;
    let taken: BTreeSet<String> = types.keys().map(|name| fold(name)).collect();
    spawn_points.retain(|name, _| !taken.contains(&fold(name)));
    let explicit: BTreeMap<String, String> = spawn_points
        .keys()
        .map(|name| (fold(name), name.clone()))
        .collect();
    for building in types.values_mut() {
        building.spawn_point = building
            .spawn_point
            .as_deref()
            .and_then(|name| explicit.get(&fold(name)))
            .cloned();
    }
    for building in types.values() {
        if let Some(redirect) = &building.spawn_point
            && let Some(spawn) = spawn_points.get_mut(redirect)
        {
            spawn.is_port |= building.is_port;
        }
    }
    for building in types.values() {
        spawn_points.insert(
            building.name.clone(),
            SpawnPoint {
                name: building.name.clone(),
                max: building.show_on_map.unwrap_or(0),
                meshes: building.show_on_map_meshes,
                provincial: building.provincial(),
                is_port: building.is_port,
            },
        );
    }
    // Type names equal under the lexer's fold are one token so they are records
    // of one type: differing fields make the bucket ambiguous, as a differing repeat does.
    let mut by_token: BTreeMap<String, &BuildingType> = BTreeMap::new();
    for name in declaration_order.iter() {
        let later = &types[name];
        match by_token.entry(fold(name)) {
            Entry::Vacant(entry) => {
                entry.insert(later);
            }
            Entry::Occupied(entry) => {
                let earlier = *entry.get();
                if !same_type(earlier, later) {
                    ambiguous_slots.insert(earlier.name.clone());
                    mark_repeat(ambiguous_slots, earlier, later);
                }
            }
        }
    }
    *buckets = explicit;
    // The first generated bucket of a fold wins, ahead of any explicit one.
    for name in declaration_order.iter().rev() {
        buckets.insert(fold(name), name.clone());
    }
    *ambiguous_slots = ambiguous_slots
        .iter()
        .filter_map(|name| buckets.get(&fold(name)).cloned())
        .collect();
}

/// Reads direct `common/buildings/*.txt` children in filename order;
/// nested files do not load.
pub fn read(layers: &ResolvedLayers) -> io::Result<Buildings> {
    read_with(layers, &layers.key_names()?.at(Point::BeforeBuildings))
}

/// [`read`], the triggers read against `names`, which
/// [`KeyNames::read`] gives before any building is known.
pub(crate) fn read_with(layers: &ResolvedLayers, names: &KeyNames) -> io::Result<Buildings> {
    // Each file by its bytes: an archive may name a file in bytes that are not UTF-8.
    let mut paths = layers
        .paths()
        .filter(|path| is_building_file(path))
        .map(|path| crate::script_files::ScriptPath::new(path.to_vec()))
        .collect::<Vec<_>>();
    paths.sort();
    let (resources, warnings) = crate::resources::read(layers)?;
    if !warnings.is_empty() {
        return Err(invalid(format!(
            "building resource registry ({} diagnostics): {}",
            warnings.len(),
            warnings
                .iter()
                .map(|(_, message)| message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )));
    }
    let mut buildings = Buildings::default();
    let mut explicit = BTreeMap::<String, SpawnPoint>::new();
    // Every declaration of each type across the files, in database order.
    let mut declarations = BTreeMap::<String, Vec<BuildingType>>::new();
    for path in paths {
        let parsed = parse_declarations(&path.read(layers)?, Some(&resources), names)
            .map_err(|error| invalid(format!("{path}: {error}")))?;
        buildings.ambiguous_slots.extend(parsed.ambiguous_slots);
        buildings
            .ambiguous_factory_types
            .extend(parsed.ambiguous_factory_types);
        // A later file's declaration replaces the flags, as it replaces the type;
        // differing declarations across files are ambiguous, as within one.
        for (name, flags) in parsed.factory_flags {
            if buildings
                .factory_flags
                .insert(name.clone(), flags)
                .is_some_and(|earlier| earlier != flags)
            {
                buildings.ambiguous_factory_types.insert(name);
            }
        }
        for name in parsed.declaration_order {
            let later = &parsed.types[&name];
            let file = declarations.entry(name.clone()).or_default();
            match parsed.repeated_types.get(&name) {
                Some(repeats) => file.extend(repeats.iter().cloned()),
                None => file.push(later.clone()),
            }
            match buildings.types.get(&name) {
                Some(earlier) => mark_repeat(&mut buildings.ambiguous_slots, earlier, later),
                None => buildings.declaration_order.push(name.clone()),
            }
            buildings.types.insert(name, later.clone());
        }
        for spawn in parsed.spawn_points.into_values() {
            insert_spawn(&mut explicit, &mut buildings.ambiguous_slots, spawn);
        }
    }
    if buildings.types.is_empty() {
        return Err(invalid("no common/buildings file declares any building"));
    }
    buildings.spawn_points = explicit;
    buildings.repeated_types = declarations
        .into_iter()
        .filter(|(_, all)| all.windows(2).any(|pair| pair[0] != pair[1]))
        .collect();
    resolve(&mut buildings);
    Ok(buildings)
}

pub(crate) fn is_building_file(path: &[u8]) -> bool {
    path.to_ascii_lowercase()
        .strip_prefix(b"common/buildings/")
        .is_some_and(|name| !name.contains(&b'/') && name.ends_with(b".txt"))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tooltip_tests;

#[cfg(test)]
mod selected_attack_tests;
