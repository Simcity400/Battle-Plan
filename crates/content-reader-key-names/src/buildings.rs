use crate::key_name_products::KeyNamesExt as _;

use content_reader_script::key_names::KeyNames;
use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    io,
};

use content_reader_layers::ResolvedLayers;
use content_reader_parser::{crt::{Scanned, scan_i32}, fields::{invalid, key_str}, parser::{self, Value}};
use content_reader_script::{modifiers, native::{self, Shape}};

pub const ENTITY_PREFIX: &str = "building_";
pub const DESTROYED_SUFFIX: &str = "_destroyed";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BuildingType {
    pub name: String,
    pub show_on_map: Option<i32>,
    pub show_on_map_meshes: i32,
    pub always_shown: bool,
    pub drawn_at_distance: bool,
    pub disable_grow_animation: bool,
    pub need_detection: bool,
    pub specializations: Vec<String>,
    pub air_defence: i32,
    pub anti_air: bool,
    pub land_fort: i32,
    pub naval_fort: i32,
    pub infrastructure: bool,
    pub shares_slots: bool,
    pub group_by: Option<Vec<u8>>,
    pub exclusive_with: Vec<Vec<u8>>,
    pub icon_frame: i32,
    pub detecting_intel_type: usize,
    pub tooltip_refused: Option<String>,
    pub supply_node: bool,
    pub is_port: bool,
    pub naval_headquarter: bool,
    pub only_costal: bool,
    pub specialization: Vec<String>,
    pub has_destroyed_mesh: bool,
    pub spawn_point: Option<String>,
    pub state_max: i32,
    pub province_max: Option<u32>,
    pub tags: Vec<String>,
    pub air_base: bool,
    pub gun_emplacement: bool,
    pub radar: bool,
    pub rocket_launch_capacity: i32,
    pub local_resources: Vec<(String, i64)>,
    pub raid_refused: Option<String>,
    pub only_display_if_exists: bool,
    pub special_icon: String,
    pub dlc_allowed: Vec<content_reader_parser::parser::Item>,
    pub state_modifiers: Vec<content_reader_parser::parser::Item>,
}

impl BuildingType {
    pub fn slot_name(&self) -> &str {
        self.spawn_point.as_deref().unwrap_or(&self.name)
    }

    pub fn provincial(&self) -> bool {
        self.province_max.is_some_and(|maximum| maximum > 0)
    }

    pub fn base_maximum(&self, defines: &content_reader_defines::defines::Defines) -> io::Result<i32> {
        let province_max = self.province_max.unwrap_or(0) as i32;
        if province_max > 0 {
            Ok(province_max)
        } else if self.state_max > 0 {
            Ok(self.state_max)
        } else {
            defines.int32("NBuildings", "MAX_BUILDING_LEVELS")
        }
    }

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

    pub fn variant_for_level(&self, level: i32, max_building_levels: i32) -> usize {
        let n = self.show_on_map_meshes.max(1) as u32;
        ((n as f32 * (level.max(0) as f32 / max_building_levels as f32)) as u32).min(n - 1) as usize
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnPoint {
    pub name: String,
    pub max: i32,
    pub meshes: i32,
    pub provincial: bool,
    pub is_port: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Buildings {
    pub types: BTreeMap<String, BuildingType>,
    pub spawn_points: BTreeMap<String, SpawnPoint>,
    pub declaration_order: Vec<String>,
    pub ambiguous_slots: BTreeSet<String>,
    pub repeated_types: BTreeMap<String, Vec<BuildingType>>,
    pub buckets: BTreeMap<String, String>,
    pub factory_flags: BTreeMap<String, FactoryFlags>,
    pub ambiguous_factory_types: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FactoryFlags {
    pub military_production: i32,
    pub naval_production: i32,
    pub general_production: i32,
    pub rocket_production: i32,
}

impl FactoryFlags {
    pub fn military(&self) -> bool {
        self.military_production > 0
    }
    pub fn naval(&self) -> bool {
        self.naval_production > 0
    }
    pub fn civilian(&self) -> bool {
        self.general_production > 0
    }
    pub fn rocket(&self) -> bool {
        self.rocket_production > 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot<'a> {
    Bucket(&'a SpawnPoint),
    Ambiguous(&'a SpawnPoint),
}

impl<'a> Slot<'a> {
    pub fn spawn(self) -> &'a SpawnPoint {
        match self {
            Slot::Bucket(spawn) | Slot::Ambiguous(spawn) => spawn,
        }
    }
}

impl Buildings {
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

    pub fn slot(&self, name: &str) -> Option<Slot<'_>> {
        let bucket = self.buckets.get(&fold(name))?;
        let spawn = self.spawn_points.get(bucket)?;
        Some(if self.ambiguous_slots.contains(bucket) {
            Slot::Ambiguous(spawn)
        } else {
            Slot::Bucket(spawn)
        })
    }

    pub fn slot_candidates<'a>(&'a self, slot: &str) -> impl Iterator<Item = &'a BuildingType> {
        self.declaration_order
            .iter()
            .filter_map(|name| self.types.get(name))
            .filter(move |building| building.name == slot || building.slot_name() == slot)
    }
}

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

fn parse_declarations(
    input: &[u8],
    resources: Option<&[crate::resources::Resource]>,
    names: &KeyNames,
) -> io::Result<Buildings> {
    use Node::*;
    let mut undecided = Vec::new();
    let mut refused = Vec::new();
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
                        Some(b"only_display_if_exists") => {
                            boolean(bytes, &mut building.only_display_if_exists)
                        }
                        Some(b"special_icon") => {
                            building.special_icon = field
                                .value
                                .read_str()
                                .map_err(|e| invalid(format!("{what}: {e}")))?
                                .into()
                        }
                        Some(b"dlc_allowed") => building
                            .dlc_allowed
                            .extend_from_slice(field.value.items().unwrap_or_default()),
                        Some(b"state_modifiers") => building
                            .state_modifiers
                            .extend_from_slice(field.value.items().unwrap_or_default()),
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
                                        if let Some(group) = cap.value.scalar() {
                                            building.group_by =
                                                Some(parser::c_string(group).to_vec());
                                        }
                                    }
                                    Some(b"exclusive_with") => {
                                        let name = |value: &Value| {
                                            Some(parser::c_string(value.scalar()?).to_vec())
                                        };
                                        match cap.value.items() {
                                            Some(items) => building.exclusive_with.extend(
                                                items.iter().filter_map(|item| name(&item.value)),
                                            ),
                                            None => {
                                                building.exclusive_with.extend(name(&cap.value))
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
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
                        Some(b"anti_air") => boolean(bytes, &mut building.anti_air),
                        Some(b"icon_frame") => integer(bytes, &mut building.icon_frame),
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
                                let amount = content_reader_parser::fixed5::script_fixed5(bytes).unwrap_or(0);
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
    Ok(buildings)
}

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

fn fold(name: &str) -> String {
    name.to_ascii_lowercase()
}

fn same_type(earlier: &BuildingType, later: &BuildingType) -> bool {
    let unnamed = |kind: &BuildingType| BuildingType {
        name: String::new(),
        ..kind.clone()
    };
    unnamed(earlier) == unnamed(later)
}

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
    for name in declaration_order.iter().rev() {
        buckets.insert(fold(name), name.clone());
    }
    *ambiguous_slots = ambiguous_slots
        .iter()
        .filter_map(|name| buckets.get(&fold(name)).cloned())
        .collect();
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Buildings> {
    layers.key_name_products()?.buildings.as_ref().map(|buildings| buildings.as_ref().clone())
        .ok_or_else(|| invalid("no common/buildings file declares any building"))
}

pub(crate) fn read_with(layers: &ResolvedLayers, names: &KeyNames) -> io::Result<Buildings> {
    let paths = content_reader_layers::script_files::database_paths(layers, BUILDINGS)?;
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
    let mut declarations = BTreeMap::<String, Vec<BuildingType>>::new();
    for path in paths {
        let parsed = parse_declarations(&path.read(layers)?, Some(&resources.declarations), names)
            .map_err(|error| invalid(format!("{path}: {error}")))?;
        buildings.ambiguous_slots.extend(parsed.ambiguous_slots);
        buildings
            .ambiguous_factory_types
            .extend(parsed.ambiguous_factory_types);
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

const BUILDINGS: &str = "common/buildings/";

pub(crate) fn is_building_file(path: &[u8]) -> bool {
    content_reader_layers::script_files::is_database_file(path, BUILDINGS)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tooltip_tests;

#[cfg(test)]
mod selected_attack_tests;
