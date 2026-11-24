use super::{entry, paths_named, province::Facts, withhold_named};
use crate::binding::{self, Bindings};
use crate::content::Content;
use crate::tooltips::{hover_text, refuse_hovers, set_hover};
use content_reader_localisation::localisation::Argument;
use dashboard_text::table_access::TokenNames;
use content_reader_key_names::buildings::BuildingType;
use content_reader_gui::gui::native;
use content_reader_parser::parser::Item;
use replay_data::rows::{BuildingRow, ProvinceRow, Recorded, StateBuildingInputRow, StateRow, tables};
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;
use viewer_foundation::{building_facts, perspective::Vision};

const RAILWAY: &str = "rail_way";
const GUN_EMPLACEMENT_FEATURE: u32 = 1 << 13;
const NAVAL_HEADQUARTER_FEATURE: u32 = 1 << 15;
const SLOT_STRIP: &str = "GFX_buildings_strip";
const NO_BUILDING: &str = "GFX_no_building";
const NO_INTEL: &str = "GFX_building_intel_icon";
const DAMAGED_OVERLAY: i64 = 3;
const WHOLE_HEALTH: i64 = 10_000_000;
const SLOT_NO_INTEL_HOVER: &str = "state view shared slot hover without intelligence: its estimate of the state's factories from the observed country's intelligence is not ported";
const SLOT_EMPTY_HOVER: &str = "state view empty shared slot hover: its slot modifier breakdown is not recorded";
const MAX_LEVEL: &str = "the building's current maximum level is not recorded";
const ENABLED: &str = "state view building pictures' enabled state: whether the observed country can construct each needs the building's current maximum level, which is not recorded";
const BUILDING_TOOLTIP: &str = "state view building tooltip: it gives the building's current maximum level, which is not recorded";

pub(super) fn damage_frame(level: i16, healthy: i16) -> i64 {
    let level = i64::from(level);
    if level == 0 {
        return 0;
    }
    let damaged = i64::from(level as i16 - healthy);
    let ratio = damaged.wrapping_mul(10_000_000_000) / level.wrapping_mul(100_000);
    ratio * 100 / 100_000
}

pub(super) fn railway_health(row: Option<&BuildingRow>) -> i64 {
    let Some(row) = row else { return 0 };
    let (level, healthy) = (i64::from(row.level), i64::from(row.healthy_level));
    if level == 0 {
        return 0;
    }
    if healthy == level {
        return 100_000;
    }
    (|| {
        let health = (row.partial_health.checked_mul(100_000)? / 10_000_000).checked_add(healthy.checked_mul(100_000)?)?;
        health.checked_mul(100_000)?.checked_div(level.checked_mul(100_000)?)
    })()
    .unwrap_or(-1)
}

#[derive(Debug, PartialEq)]
struct Railway {
    damage: i64,
    levels: Result<(i32, i32), String>,
}

impl Default for Railway {
    fn default() -> Self {
        Self { damage: 0, levels: Ok((0, 0)) }
    }
}

fn railway_level(edges: &[replay_data::rows::RailEdgeRow], province: u32) -> i32 {
    let first = edges.partition_point(|edge| edge.from < province);
    edges[first..].iter().take_while(|edge| edge.from == province).map(|edge| edge.level).fold(0, i32::max)
}

fn damaged_levels(level: i32, damage: i64) -> i32 {
    let product = i64::from(level).wrapping_mul(damage);
    ((product / 100_000) as i32).wrapping_add(i32::from(product % 100_000 > 0))
}

fn railway(frame: &DecodedFrame, names: &TokenNames<'_>, facts: &Facts<'_>, members: &[u32], province_rows: &[BuildingRow]) -> Result<Railway, String> {
    let clicked = facts.province;
    let supply = located(province_rows, clicked.id)
        .iter()
        .map(|row| row.country)
        .find(|&country| (country as i32) > 0)
        .unwrap_or(clicked.controller);
    if (supply as i32) <= 0 {
        return Ok(Railway::default());
    }
    let inventories = frame.complete::<tables::TrainInventory>().map_err(|e| e.to_string())?;
    let access = &inventories
        .binary_search_by_key(&supply, |row| row.country)
        .map(|at| &inventories[at])
        .map_err(|_| format!("the supply country {supply} has no TrainInventory row"))?
        .controller_access;
    let infos = frame.complete::<tables::RailCooldowns>().map_err(|e| e.to_string())?;
    let provinces = frame.complete::<tables::Provinces>().map_err(|e| e.to_string())?;
    let edges = frame.complete::<tables::RailEdges>().map_err(|e| format!("the railway levels are not recorded: {e}"));
    let mut railway = Railway::default();
    for &id in members {
        let province: &ProvinceRow = provinces
            .binary_search_by_key(&id, |row| row.id)
            .map(|at| &provinces[at])
            .map_err(|_| format!("Provinces province {id} is absent"))?;
        let slot = facts.slot(province.controller)?;
        let marked = access.get(slot).ok_or_else(|| format!("the supply access array holds no slot {slot}"))? != 0;
        let Ok(at) = infos.binary_search_by_key(&province.id, |row| row.province) else { continue };
        if !marked {
            continue;
        }
        let level = edges.as_ref().map(|edges| railway_level(edges, id)).map_err(Clone::clone);
        let health = match infos[at].building_bound {
            Recorded::Value(Some(true)) => {
                let mut building = None;
                for row in province_rows.iter().filter(|row| row.location == province.id) {
                    if names.name(row.building_type).map_err(|e| e.to_string())?.eq_ignore_ascii_case(RAILWAY) {
                        building = Some(row);
                    }
                }
                railway_health(building)
            }
            Recorded::Value(Some(false)) => 0,
            Recorded::Value(None) => return Err(format!("province {}'s railway building was refused when read", province.id)),
            Recorded::NotRecorded => return Err("whether each railway info holds a building is not recorded".into()),
        };
        railway.step(100_000_i64.wrapping_sub(health), level);
    }
    Ok(railway)
}

impl Railway {
    fn step(&mut self, damage: i64, level: Result<i32, String>) {
        self.levels = match (std::mem::replace(&mut self.levels, Ok((0, 0))), level) {
            (Ok((mut worst, current)), Ok(level)) => {
                if damage > self.damage {
                    worst = level;
                }
                Ok((worst, current.max(level)))
            }
            (Err(why), _) | (_, Err(why)) => Err(why),
        };
        self.damage = self.damage.max(damage);
    }
}

pub(super) fn building_inputs<'f>(frame: &'f DecodedFrame, state: &StateRow) -> Result<&'f StateBuildingInputRow, String> {
    let rows = frame.complete::<tables::StateBuildingInputs>().map_err(|e| e.to_string())?;
    rows.binary_search_by_key(&state.id, |row| row.state)
        .map(|at| &rows[at])
        .map_err(|_| format!("state {} has no StateBuildingInputs row", state.id))
}

pub(super) fn shared_slots(inputs: &StateBuildingInputRow, owner: u32, maximum: i32) -> Result<i32, String> {
    if inputs.owner != owner {
        return Err("its building inputs were read for another owner".into());
    }
    let (Some(local), Some(local_factor), Some(global), Some(global_factor)) =
        (inputs.local_building_slots, inputs.local_building_slots_factor, inputs.global_building_slots, inputs.global_building_slots_factor)
    else {
        return Err("a building slot modifier was refused when read".into());
    };
    let mut base = local.wrapping_add(global);
    if base > 0 {
        base = local_factor.wrapping_add(100_000).wrapping_add(global_factor).wrapping_mul(base) / 100_000;
    }
    Ok(((base / 100_000) as i32).wrapping_add(inputs.extra_shared_slots).max(0).min(maximum))
}

pub(super) fn energy_usage(inputs: &StateBuildingInputRow, controller: u32, infrastructure: i64, buildings: &[(i16, content_reader_key_names::buildings::FactoryFlags)]) -> Result<i64, String> {
    if inputs.controller != controller {
        return Err("its building inputs were read for another controller".into());
    }
    let (Some(factories), Some(local), Some(per_infrastructure), Some(country), Some(factor)) = (
        inputs.local_factories,
        inputs.local_factory_energy_consumption,
        inputs.local_factory_energy_consumption_per_infrastructure,
        inputs.factory_energy_consumption,
        inputs.factory_energy_factor,
    ) else {
        return Err("a factory energy modifier was refused when read".into());
    };
    let a = factories.wrapping_add(100_000).max(0);
    let b = local
        .wrapping_add(100_000)
        .wrapping_add(country)
        .wrapping_add(infrastructure.wrapping_mul(100_000).wrapping_mul(per_infrastructure) / 100_000)
        .max(0);
    let mut sum = 0_i64;
    for &(level, flags) in buildings {
        let each = i64::from(level).wrapping_mul(b).wrapping_mul(a) / 100_000;
        for counts in [flags.military(), flags.naval(), flags.civilian()] {
            if counts {
                sum = sum.wrapping_add(each);
            }
        }
    }
    Ok(sum.wrapping_mul(factor) / 100_000)
}

pub(super) fn state_energy(content: &Content, frame: &DecodedFrame, state: &StateRow) -> Result<i64, String> {
    let inputs = building_inputs(frame, state)?;
    let names = TokenNames::new(frame).map_err(|e| e.to_string())?;
    let rows = frame.complete::<tables::StateBuildings>().map_err(|e| e.to_string())?;
    let buildings = &content.trigger_content.buildings;
    let (mut infrastructure, mut active) = (0_i64, Vec::new());
    for (name, row) in by_location(rows, &names, state.id)? {
        let building = named(&buildings.types, &name).ok_or_else(|| format!("building type {name} has no declaration"))?;
        if building.infrastructure {
            infrastructure = i64::from(row.healthy_level);
        }
        if row.active {
            if buildings.ambiguous_factory_types.iter().any(|key| key.eq_ignore_ascii_case(&name)) {
                return Err(format!("building type {name} is declared again with other factory groups"));
            }
            let flags = named(&buildings.factory_flags, &name).copied().unwrap_or_default();
            active.push((row.level, flags));
        }
    }
    energy_usage(inputs, state.controller, infrastructure, &active)
}

fn named<'m, V>(map: &'m BTreeMap<String, V>, name: &str) -> Option<&'m V> {
    map.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value)
}

pub(super) fn round_whole(value: i64) -> i64 {
    let half = if value < 0 { -50_000 } else { 50_000 };
    value.wrapping_add(half) / 100_000
}

fn frame(native: i64) -> u32 {
    (native as u32).wrapping_sub(1)
}

fn dlc_hidden(building: &BuildingType, mask: u32, allowed: impl FnOnce(&[Item]) -> Result<bool, String>) -> Result<bool, String> {
    if building.gun_emplacement && mask & GUN_EMPLACEMENT_FEATURE == 0 || building.naval_headquarter && mask & NAVAL_HEADQUARTER_FEATURE == 0 {
        return Ok(true);
    }
    if building.dlc_allowed.is_empty() {
        return Ok(false);
    }
    allowed(&building.dlc_allowed).map(|allowed| !allowed)
}

fn has_state_modifier(building: &BuildingType) -> Result<bool, String> {
    let mut any = false;
    for item in &building.state_modifiers {
        let value = item
            .value
            .scalar()
            .and_then(content_reader_parser::fixed5::script_fixed5)
            .ok_or_else(|| format!("building type {}'s state modifier {} is not a number", building.name, String::from_utf8_lossy(item.key.as_deref().unwrap_or_default())))?;
        any |= value != 0;
    }
    Ok(any)
}

struct Hour<'a> {
    content: &'a Content,
    frame: &'a DecodedFrame,
    facts: &'a Facts<'a>,
    mask: u32,
    world: std::cell::OnceCell<Result<dashboard_text::triggers::World<'a>, String>>,
}

impl<'a> Hour<'a> {
    fn hidden(&self, building: &BuildingType) -> Result<bool, String> {
        dlc_hidden(building, self.mask, |items| {
            let world = self
                .world
                .get_or_init(|| dashboard_text::triggers::World::new(self.frame, self.content.trigger_content.facts()).map_err(|why| why.to_string()))
                .as_ref()
                .map_err(Clone::clone)?;
            world.evaluate(items, dashboard_text::triggers::Scopes::empty()).decided()
        })
        .map_err(|why| format!("building type {}'s dlc_allowed: {why}", building.name))
    }
}

fn by_location<'f>(rows: &'f [BuildingRow], names: &TokenNames<'_>, location: u32) -> Result<BTreeMap<String, &'f BuildingRow>, String> {
    rows.iter()
        .filter(|row| row.location == location)
        .map(|row| Ok((names.name(row.building_type).map_err(|e| e.to_string())?.to_ascii_lowercase(), row)))
        .collect()
}

pub(super) fn bind(content: &Content, frame: &DecodedFrame, root: &native::Element, out: &mut Bindings, facts: &Facts<'_>, vision: &Vision) {
    const GRIDS: [&str; 5] = ["state_building_entries", "state_shared_slot_building_entries", "province_building_entries", "province_custom_icon_building_entries", "building_modifiers_icon"];
    let prepared = (|| -> Result<_, String> {
        let names = TokenNames::new(frame).map_err(|e| e.to_string())?;
        let state_rows = frame.complete::<tables::StateBuildings>().map_err(|e| e.to_string())?;
        let province_rows = frame.complete::<tables::ProvinceBuildings>().map_err(|e| e.to_string())?;
        let mask = match frame.complete::<tables::SimulationFeatures>() {
            Ok([features]) => features.dlc_mask,
            Ok(_) => return Err("the DLC mask needs one SimulationFeatures row".into()),
            Err(why) => return Err(why.to_string()),
        };
        Ok((names, state_rows, province_rows, mask))
    })();
    let (names, state_rows, province_rows, mask) = match prepared {
        Ok(prepared) => prepared,
        Err(why) => {
            for name in GRIDS {
                withhold_named(out, root, &[name], &format!("state view buildings: {why}"));
            }
            return;
        }
    };
    let hour = Hour { content, frame, facts, mask, world: Default::default() };
    let buildings = &content.trigger_content.buildings;
    let types: Vec<Result<&BuildingType, String>> = buildings
        .declaration_order
        .iter()
        .map(|name| {
            if buildings.repeated_types.contains_key(name) {
                return Err(format!("building type {name} is declared more than once with differing fields"));
            }
            buildings.types.get(name).ok_or_else(|| format!("building type {name} has no declaration"))
        })
        .collect();
    let state = by_location(state_rows, &names, facts.state.id);
    let province = by_location(province_rows, &names, facts.province.id);
    let mut enabled_refused = false;
    let railway = members(content, facts.state.id).and_then(|members| railway(frame, &names, facts, members, province_rows));
    match state.as_ref().map_err(Clone::clone).and_then(|rows| shared_slots_bound(out, &hour, &names, &types, rows, vision)) {
        Ok(entries) => grid(out, root, "state_shared_slot_building_entries", entry::SHARED_SLOT, entries, &mut |_, _| {}),
        Err(why) => withhold_named(out, root, &["state_shared_slot_building_entries"], &format!("state view shared slot buildings: {why}")),
    }
    match state.as_ref().map_err(Clone::clone).and_then(|rows| state_entries(out, &hour, &types, rows, railway)) {
        Ok(entries) => {
            enabled_refused |= !entries.is_empty();
            grid(out, root, "state_building_entries", entry::STATE_BUILDING, entries, &mut |out, values| {
                out.withhold_in(values, &["level"], format!("state view building level: {MAX_LEVEL}"));
            });
        }
        Err(why) => withhold_named(out, root, &["state_building_entries"], &format!("state view state buildings: {why}")),
    }
    match province.as_ref().map_err(Clone::clone).and_then(|rows| province_entries(out, &hour, &types, rows, vision)) {
        Ok((plain, landmarks)) => {
            enabled_refused |= !plain.is_empty();
            grid(out, root, "province_building_entries", entry::PROVINCE_BUILDING, plain, &mut |out, values| {
                out.withhold_in(values, &["level"], format!("state view province building level: {MAX_LEVEL}"));
            });
            grid(out, root, "province_custom_icon_building_entries", entry::LANDMARK, landmarks, &mut |_, _| {});
        }
        Err(why) => withhold_named(out, root, &["province_building_entries", "province_custom_icon_building_entries"], &format!("state view province buildings: {why}")),
    }
    if enabled_refused {
        out.refuse(ENABLED.into());
    }
    match modifiers_shown(frame, &names, state_rows, province_rows, facts, buildings) {
        Ok(true) => {
            let text = modifiers_hover(content, frame, &names, state_rows, province_rows, facts).map_err(|why| format!("state view building modifiers hover: {why}"));
            for path in paths_named(root, "building_modifiers_icon") {
                out.values.entry(path.clone()).or_default().hidden = false;
                match &text {
                    Ok(text) => set_hover(&mut out.values, path, hover_text(text), None),
                    Err(why) => {
                        set_hover(&mut out.values, path, None, None);
                        out.refuse(why.clone());
                    }
                }
            }
        }
        Ok(false) => {
            for path in paths_named(root, "building_modifiers_icon") {
                binding::hide(&mut out.values, &path);
            }
        }
        Err(why) => withhold_named(out, root, &["building_modifiers_icon"], &format!("state view building modifiers icon: {why}")),
    }
}

fn grid(out: &mut Bindings, root: &native::Element, name: &str, template: &str, mut entries: Vec<crate::Values>, each: &mut dyn FnMut(&mut Bindings, &mut crate::Values)) {
    for values in &mut entries {
        each(out, values);
    }
    for path in paths_named(root, name) {
        let widget = out.values.entry(path).or_default();
        widget.entry_template = Some(template.into());
        widget.entries = entries.clone();
    }
}

fn building_entry(out: &mut Bindings, content: &Content, building: &BuildingType, row: Option<&BuildingRow>) -> crate::Values {
    let mut values = crate::Values::new();
    values.entry("building_picture".into()).or_default().frame = Some(frame(i64::from(building.icon_frame)));
    let (level, healthy) = row.map_or((0, 0), |row| (row.level, row.healthy_level));
    values.entry("damage_bar".into()).or_default().frame = Some(frame(damage_frame(level, healthy)));
    damage_hover(out, content, &mut values, level, healthy);
    building_tooltip(out, &mut values, &["building_picture", "level_bg"]);
    values
}

fn damage_hover(out: &mut Bindings, content: &Content, values: &mut crate::Values, level: i16, healthy: i16) {
    let text = damaged_text(content, level, healthy);
    match text {
        Ok(text) => set_hover(values, "damage_bar", hover_text(&text), None),
        Err(why) => {
            set_hover(values, "damage_bar", None, None);
            out.refuse(format!("state view building damage hover: {why}"));
        }
    }
}

fn damaged_text(content: &Content, level: i16, healthy: i16) -> Result<String, String> {
    let damaged = level.wrapping_sub(healthy);
    super::loc(
        content,
        "BUILDING_DAMAGED",
        &[("DAMAGED", Argument::Int32(i32::from(damaged))), ("CURRENT", Argument::Int32(i32::from(level)))],
    )
}

fn building_tooltip(out: &mut Bindings, values: &mut crate::Values, keys: &[&str]) {
    refuse_hovers(values, keys.iter().copied());
    out.refuse(BUILDING_TOOLTIP.into());
}

fn state_entries(out: &mut Bindings, hour: &Hour<'_>, types: &[Result<&BuildingType, String>], rows: &BTreeMap<String, &BuildingRow>, damage: Result<Railway, String>) -> Result<Vec<crate::Values>, String> {
    let mut entries = Vec::new();
    let mut railway = None;
    for building in types {
        let building = (*building).as_ref().map_err(Clone::clone)?;
        if building.provincial() {
            if building.name.eq_ignore_ascii_case(RAILWAY) {
                railway = Some(*building);
            }
            continue;
        }
        if building.shares_slots || hour.hidden(building)? {
            continue;
        }
        entries.push(building_entry(out, hour.content, building, rows.get(&building.name.to_ascii_lowercase()).copied()));
    }
    if let Some(railway) = railway {
        let mut values = crate::Values::new();
        values.entry("building_picture".into()).or_default().frame = Some(frame(i64::from(railway.icon_frame)));
        match damage {
            Ok(Railway { damage, levels }) => {
                values.entry("damage_bar".into()).or_default().frame = Some(frame(damage.wrapping_mul(100) / 100_000));
                let text = levels.and_then(|(worst, current)| {
                    super::loc(
                        hour.content,
                        "BUILDING_DAMAGED",
                        &[("DAMAGED", Argument::Int32(damaged_levels(worst, damage))), ("CURRENT", Argument::Int32(current))],
                    )
                });
                match text {
                    Ok(text) => set_hover(&mut values, "damage_bar", hover_text(&text), None),
                    Err(why) => {
                        set_hover(&mut values, "damage_bar", None, None);
                        out.refuse(format!("state view railway damage hover: {why}"));
                    }
                }
            }
            Err(why) => out.withhold_in(&mut values, &["damage_bar"], format!("state view railway damage bar: {why}")),
        }
        building_tooltip(out, &mut values, &["building_picture", "level_bg"]);
        entries.push(values);
    }
    Ok(entries)
}

fn members(content: &Content, state: u32) -> Result<&[u32], String> {
    content
        .air_context
        .as_ref()
        .ok_or("the state content is not loaded")?
        .province_hover
        .state_members()
        .get(&state)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("state {state} has no provinces in the content"))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Slot<'a> {
    Level(&'a BuildingType, &'a BuildingRow, i16),
    Empty { unlocked: bool },
}

fn shared_slot_fill<'a>(types: &[(&'a BuildingType, Option<&'a BuildingRow>)], unlocked: i32, maximum: i32) -> Vec<Slot<'a>> {
    let maximum = usize::try_from(maximum).unwrap_or(0);
    let mut slots = Vec::with_capacity(maximum);
    for &(building, row) in types {
        let Some(row) = row else { continue };
        for index in 1..=row.level {
            if slots.len() >= maximum {
                break;
            }
            slots.push(Slot::Level(building, row, index));
        }
    }
    while slots.len() < maximum {
        let unlocked = i32::try_from(slots.len()).is_ok_and(|at| at < unlocked);
        slots.push(Slot::Empty { unlocked });
    }
    slots
}

fn level_health(row: &BuildingRow, index: i16) -> i64 {
    if row.level == 0 {
        return 0;
    }
    if index <= row.healthy_level {
        WHOLE_HEALTH
    } else if i32::from(index) <= i32::from(row.healthy_level) + 1 {
        row.partial_health
    } else {
        0
    }
}

fn level_damage_frame(health: i64) -> i64 {
    100 - health.div_euclid(100_000)
}

fn queued_shared(frame: &DecodedFrame, names: &TokenNames<'_>, state: u32, shared: &[(&BuildingType, Option<&BuildingRow>)]) -> Result<(), String> {
    let rows = frame.complete::<tables::BuildingConstruction>().map_err(|e| e.to_string())?;
    for row in rows.iter().filter(|row| row.target_state == state) {
        let name = names.name(row.building_type).map_err(|e| e.to_string())?;
        if shared.iter().any(|(building, _)| building.name.eq_ignore_ascii_case(name)) {
            return Err(format!("construction of {name} is queued in the state, and the levels its lines add to the slots are not recorded"));
        }
    }
    Ok(())
}

fn shared_slots_bound(out: &mut Bindings, hour: &Hour<'_>, names: &TokenNames<'_>, types: &[Result<&BuildingType, String>], rows: &BTreeMap<String, &BuildingRow>, vision: &Vision) -> Result<Vec<crate::Values>, String> {
    let content = hour.content;
    let facts = hour.facts;
    let maximum = content.assets.max_shared_slots.clone()?;
    let unlocked = shared_slots(building_inputs(hour.frame, facts.state)?, facts.state.owner, maximum)?;
    let mut shared = Vec::new();
    for building in types {
        let building = (*building).as_ref().map_err(Clone::clone)?;
        if building.provincial() || !building.shares_slots || hour.hidden(building)? {
            continue;
        }
        shared.push((*building, rows.get(&building.name.to_ascii_lowercase()).copied()));
    }
    queued_shared(hour.frame, names, facts.state.id, &shared)?;
    let controller = facts.slot(facts.state.controller)?;
    let sees = vision.shares_vision_proven(controller).map_err(|why| why.clause())?;
    let status = rows.values().map(|row| row.country).find(|&country| (country as i32) > 0).unwrap_or(facts.state.controller);
    let status_sees = vision.shares_vision_proven(facts.slot(status)?).map_err(|why| why.clause());
    let slots = shared_slot_fill(&shared, unlocked, maximum);
    Ok(slots.iter().map(|slot| shared_slot_entry(out, content, *slot, sees, controller == vision.ours(), &status_sees)).collect())
}

fn shared_slot_entry(out: &mut Bindings, content: &Content, slot: Slot<'_>, sees: bool, enabled: bool, status_sees: &Result<bool, String>) -> crate::Values {
    let mut values = crate::Values::new();
    let picture = values.entry("building_picture".into()).or_default();
    picture.hidden = false;
    picture.enabled = Some(enabled);
    match slot {
        Slot::Level(building, row, index) if sees => {
            picture.sprite = Some(SLOT_STRIP.into());
            picture.frame = Some(frame(i64::from(building.icon_frame)));
            let health = level_health(row, index);
            let bar = values.entry("damage_bar".into()).or_default();
            bar.hidden = false;
            bar.frame = Some(frame(level_damage_frame(health)));
            if row.healthy_level < index {
                let overlay = values.entry("building_status_overlay".into()).or_default();
                overlay.hidden = false;
                overlay.frame = Some(frame(DAMAGED_OVERLAY));
            } else {
                binding::hide(&mut values, "building_status_overlay");
            }
            match status_sees {
                Ok(false) => {}
                Ok(true) => match super::loc(content, "BUILDING_LEVEL_HEALTH", &[("VALUE", Argument::Fixed5(health.wrapping_mul(1000) / 100_000))]) {
                    Ok(text) => set_hover(&mut values, "damage_bar", hover_text(&text), None),
                    Err(why) => {
                        set_hover(&mut values, "damage_bar", None, None);
                        out.refuse(format!("state view shared slot damage hover: {why}"));
                    }
                },
                Err(why) => {
                    set_hover(&mut values, "damage_bar", None, None);
                    out.refuse(format!("state view shared slot damage hover: {why}"));
                }
            }
            building_tooltip(out, &mut values, &["building_picture"]);
        }
        _ => {
            let unlocked = matches!(slot, Slot::Empty { unlocked: true });
            picture.sprite = Some(NO_BUILDING.into());
            picture.frame = Some(frame(2 - i64::from(unlocked)));
            if !sees {
                picture.sprite = Some(NO_INTEL.into());
                picture.frame = Some(frame(1));
            }
            binding::hide(&mut values, "building_status_overlay");
            binding::hide(&mut values, "damage_bar");
            refuse_hovers(&mut values, ["building_picture"]);
            out.refuse(if sees { SLOT_EMPTY_HOVER } else { SLOT_NO_INTEL_HOVER }.into());
        }
    }
    binding::hide(&mut values, "remove");
    values
}

fn province_entries(
    out: &mut Bindings,
    hour: &Hour<'_>,
    types: &[Result<&BuildingType, String>],
    rows: &BTreeMap<String, &BuildingRow>,
    vision: &Vision,
) -> Result<(Vec<crate::Values>, Vec<crate::Values>), String> {
    let facts = hour.facts;
    let viewer = vision.ours();
    let controller = facts.slot(facts.province.controller)?;
    let mut plain = Vec::new();
    let mut landmarks = Vec::new();
    for building in types {
        let building = (*building).as_ref().map_err(Clone::clone)?;
        if !building.provincial() || building.name.eq_ignore_ascii_case(RAILWAY) || hour.hidden(building)? {
            continue;
        }
        let row = rows.get(&building.name.to_ascii_lowercase()).copied();
        let built = row.is_some_and(|row| row.level >= 1);
        if building.only_display_if_exists && !built {
            continue;
        }
        if building.need_detection {
            let shown = match row.filter(|row| row.level != 0) {
                None => false,
                Some(row) => detected(hour, row, viewer)?,
            };
            if !shown {
                continue;
            }
        }
        if building.special_icon.is_empty() {
            plain.push(building_entry(out, hour.content, building, row));
            continue;
        }
        let sees = vision
            .shares_vision_proven(controller)
            .map_err(|why| why.clause())?
            || vision
                .sees_province_object(facts.province.id, building.always_shown)
                .ok_or("the observed country's visibility of the province is not recorded")?
            || controller == viewer;
        if !sees {
            continue;
        }
        landmarks.push(landmark(out, hour.content, building, row));
    }
    Ok((plain, landmarks))
}

fn facility_scale(content: &Content) -> Result<f32, String> {
    let template = content
        .assets
        .native_windows
        .gui_type(entry::LANDMARK.as_bytes())
        .ok_or_else(|| format!("the entry template {} is absent from the content stack", entry::LANDMARK))?;
    let place = crate::gui::elements(template)
        .into_iter()
        .find(|(path, _)| path.rsplit('/').next() == Some("facility_building_button_scale"))
        .and_then(|(_, element)| element.position_type())
        .ok_or("the entry template has no facility_building_button_scale position")?;
    Ok(scale_of(place.position[0]))
}

fn scale_of(x: i32) -> f32 {
    x as f32 / 10.0
}

fn landmark(out: &mut Bindings, content: &Content, building: &BuildingType, row: Option<&BuildingRow>) -> crate::Values {
    let mut values = crate::Values::new();
    let (level, healthy) = row.map_or((0, 0), |row| (row.level, row.healthy_level));
    let damage = if level > 0 { damage_frame(level, healthy) } else { 0 };
    let scale = if building.specialization.is_empty() { Ok(None) } else { facility_scale(content).map(Some) };
    match scale {
        Ok(scale) => {
            let frames = content.assets.drawable_sprite(building.special_icon.as_bytes()).map_or(1, |sprite| sprite.frames);
            let button = values.entry("building_button".into()).or_default();
            button.sprite = Some(building.special_icon.clone());
            button.frame = Some(u32::from(frames >= 2 && damage > 0));
            button.scale = scale;
        }
        Err(why) => out.withhold_in(&mut values, &["building_button"], format!("state view facility building button: {why}")),
    }
    values.entry("damage_bar".into()).or_default().frame = Some(frame(damage));
    damage_hover(out, content, &mut values, level, healthy);
    building_tooltip(out, &mut values, &["building_button"]);
    values
}

fn detected(hour: &Hour<'_>, row: &BuildingRow, viewer: u32) -> Result<bool, String> {
    let frame = hour.frame;
    let facts = hour.facts;
    let targets = frame.complete::<tables::RaidTargetStatuses>().ok();
    building_facts::detected(
        (row.building_type, row.location, row.status_flags, row.country),
        |provincial, location| {
            if provincial {
                (location == facts.province.id).then_some(facts.province.controller)
            } else {
                (location == facts.state.id).then_some(facts.state.controller)
            }
        },
        |raw| facts.slot(raw).ok(),
        viewer,
        |scope| {
            let rows = targets?;
            let slot = viewer as usize;
            Some(
                rows.iter()
                    .filter(|target| {
                        let s = &target.scope;
                        s.building_type != 0 && (s.building_type, s.building_location, s.building_flags, s.building_country as u32) == scope
                    })
                    .max_by_key(|target| target.ordinal)
                    .map(|target| target.detected.get(slot / 8).is_some_and(|byte| byte & (1 << (slot % 8)) != 0)),
            )
        },
    )
    .ok_or_else(|| "whether the observed country detected the building cannot be decided this hour".into())
}

fn modifiers_shown(
    frame: &DecodedFrame,
    names: &TokenNames<'_>,
    state_rows: &[BuildingRow],
    province_rows: &[BuildingRow],
    facts: &Facts<'_>,
    buildings: &content_reader_key_names::buildings::Buildings,
) -> Result<bool, String> {
    let provinces: std::collections::BTreeSet<u32> = frame
        .complete::<tables::Provinces>()
        .map_err(|e| e.to_string())?
        .iter()
        .filter(|row| row.state == facts.state.id)
        .map(|row| row.id)
        .collect();
    let rows = state_rows
        .iter()
        .filter(|row| row.location == facts.state.id)
        .chain(province_rows.iter().filter(|row| provinces.contains(&row.location)));
    for row in rows.filter(|row| row.level > 0 && row.healthy_level > 0) {
        let name = names.name(row.building_type).map_err(|e| e.to_string())?;
        let building = buildings
            .by_token(name)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("building type {name} has no declaration"))?;
        if has_state_modifier(building)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn modifiers_hover(
    content: &Content,
    frame: &DecodedFrame,
    names: &TokenNames<'_>,
    state_rows: &[BuildingRow],
    province_rows: &[BuildingRow],
    facts: &Facts<'_>,
) -> Result<String, String> {
    let definitions = crate::modifier_text::Definitions::read(frame, &content.modifier_names)
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let members = content
        .air_context
        .as_ref()
        .ok_or("the state content is not loaded")?
        .province_hover
        .state_members()
        .get(&facts.state.id)
        .ok_or_else(|| format!("state {} has no provinces in the content", facts.state.id))?;
    let provinces = frame.complete::<tables::Provinces>().map_err(|e| e.to_string())?;
    let controller = facts.slot(facts.state.controller)?;
    let mut locations: Vec<&[BuildingRow]> = vec![located(state_rows, facts.state.id)];
    for &id in members {
        let row = provinces
            .binary_search_by_key(&id, |row| row.id)
            .map(|at| &provinces[at])
            .map_err(|_| format!("Provinces province {id} is absent"))?;
        if facts.slot(row.controller)? == controller {
            locations.push(located(province_rows, id));
        }
    }
    let buildings = &content.trigger_content.buildings;
    let mut blocks = Vec::new();
    for rows in locations {
        let mut rows: Vec<&BuildingRow> = rows.iter().collect();
        rows.sort_by_key(|row| row.ordinal);
        for row in rows {
            let name = names.name(row.building_type).map_err(|e| e.to_string())?;
            let building = buildings
                .by_token(name)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("building type {name} has no declaration"))?;
            let Some(block) = scaled_state_modifiers(&definitions, building, row.healthy_level)? else {
                continue;
            };
            blocks.push((super::loc(content, &building.name, &[])?, block));
        }
    }
    let heading = |name: &str| super::loc(content, "STATE_BUILDING_MODIFIER_HEADING", &[("BUILDING", Argument::Text(name))]);
    let prefix = super::loc(content, "STATE_BUILDING_MODIFIER_PREFIX", &[])?;
    let mut text = super::loc(content, "STATE_BUILDING_STATE_MODIFIERS", &[])?;
    end_line(&mut text);
    for (name, block) in blocks {
        text.push_str(&heading(&name)?);
        end_line(&mut text);
        text.push_str(
            &block
                .describe(&definitions, content.assets.as_ref(), &content.localisation, crate::fixed::ONE, &prefix, u32::MAX)
                .map_err(|e| e.to_string())?,
        );
        end_line(&mut text);
    }
    Ok(text)
}

fn located(rows: &[BuildingRow], location: u32) -> &[BuildingRow] {
    let first = rows.partition_point(|row| row.location < location);
    let end = first + rows[first..].partition_point(|row| row.location == location);
    &rows[first..end]
}

fn scaled_state_modifiers(
    definitions: &crate::modifier_text::Definitions<'_>,
    building: &BuildingType,
    healthy: i16,
) -> Result<Option<crate::modifier_text::Block>, String> {
    if healthy <= 0 {
        return Ok(None);
    }
    let authored = content_reader_script::modifiers::values(&building.state_modifiers);
    let (mut block, _) = crate::modifier_text::Block::build(definitions, &authored).map_err(|e| e.to_string())?;
    let scale = i64::from(healthy).wrapping_mul(crate::fixed::ONE);
    for value in block.values.values_mut() {
        *value = crate::fixed::mul(*value, scale);
    }
    Ok(block.kept(definitions).map_err(|e| e.to_string())?.then_some(block))
}

fn end_line(text: &mut String) {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_modifiers_scale_by_healthy_levels_and_a_building_without_any_leaves_the_hover() {
        let rows = [replay_data::rows::ModifierDefinitionRow { index: 0, token: 1, name_key: b"A".to_vec(), category_mask: 1, ..Default::default() }];
        let definitions = crate::modifier_text::Definitions::from_parts(&rows, &[(b"a", 0)], Some(0));
        let building = BuildingType { name: "b".into(), state_modifiers: content_reader_parser::parser::parse(b"a = 0.1 undeclared = 1").unwrap().items, ..Default::default() };
        assert!(scaled_state_modifiers(&definitions, &building, 0).unwrap().is_none());
        let block = scaled_state_modifiers(&definitions, &building, 3).unwrap().unwrap();
        assert_eq!(block.values, BTreeMap::from([(0, 30_000)]));
        let none = BuildingType { name: "c".into(), ..Default::default() };
        assert!(scaled_state_modifiers(&definitions, &none, 3).unwrap().is_none());
    }

    #[test]
    fn shared_slots_fill_each_types_levels_in_order_then_empty_slots_up_to_the_maximum() {
        let civilian = BuildingType { name: "industrial_complex".into(), ..Default::default() };
        let military = BuildingType { name: "arms_factory".into(), ..Default::default() };
        let unbuilt = BuildingType { name: "dockyard".into(), ..Default::default() };
        let row = |level| BuildingRow { level, healthy_level: level, ..Default::default() };
        let (two, one) = (row(2), row(1));
        let types = [(&civilian, Some(&two)), (&unbuilt, None), (&military, Some(&one))];
        let slots = shared_slot_fill(&types, 4, 6);
        assert_eq!(
            slots,
            [
                Slot::Level(&civilian, &two, 1),
                Slot::Level(&civilian, &two, 2),
                Slot::Level(&military, &one, 1),
                Slot::Empty { unlocked: true },
                Slot::Empty { unlocked: false },
                Slot::Empty { unlocked: false },
            ]
        );
        assert_eq!(shared_slot_fill(&types, 4, 2), [Slot::Level(&civilian, &two, 1), Slot::Level(&civilian, &two, 2)]);
        assert!(shared_slot_fill(&types, 4, -1).is_empty());
    }

    #[test]
    fn a_slots_level_health_is_whole_partial_or_none_and_its_bar_shows_the_missing_percent() {
        let row = BuildingRow { level: 4, healthy_level: 2, partial_health: 2_550_000, ..Default::default() };
        assert_eq!(level_health(&row, 2), WHOLE_HEALTH);
        assert_eq!(level_health(&row, 3), 2_550_000);
        assert_eq!(level_health(&row, 4), 0);
        assert_eq!(level_health(&BuildingRow::default(), 1), 0);
        assert_eq!(level_damage_frame(WHOLE_HEALTH), 0);
        assert_eq!(level_damage_frame(2_550_000), 75);
        assert_eq!(level_damage_frame(0), 100);
        assert_eq!(level_damage_frame(-1), 101);
    }

    #[test]
    fn the_railway_hover_takes_the_first_worst_provinces_level_and_the_highest_level() {
        use replay_data::rows::RailEdgeRow;
        let edge = |from, to, level| RailEdgeRow { from, to, level };
        let edges = [edge(1, 2, 3), edge(1, 5, 1), edge(2, 1, 3), edge(4, 9, 5)];
        assert_eq!(railway_level(&edges, 1), 3);
        assert_eq!(railway_level(&edges, 3), 0);
        assert_eq!(railway_level(&edges, 4), 5);
        let mut railway = Railway::default();
        railway.step(40_000, Ok(3));
        railway.step(40_000, Ok(1));
        railway.step(0, Ok(5));
        assert_eq!(railway, Railway { damage: 40_000, levels: Ok((3, 5)) });
        assert_eq!(damaged_levels(3, 40_000), 2);
        assert_eq!(damaged_levels(5, 40_000), 2);
        assert_eq!(damaged_levels(0, 100_000), 0);
        railway.step(100_000, Err("the railway levels are not recorded".into()));
        assert_eq!(railway, Railway { damage: 100_000, levels: Err("the railway levels are not recorded".into()) });
    }

    #[test]
    fn railway_health_counts_healthy_levels_and_the_partial_next() {
        let row = |level, healthy_level, partial_health| BuildingRow { level, healthy_level, partial_health, ..Default::default() };
        assert_eq!(railway_health(None), 0);
        assert_eq!(railway_health(Some(&row(0, 0, 0))), 0);
        assert_eq!(railway_health(Some(&row(4, 4, 0))), 100_000);
        assert_eq!(railway_health(Some(&row(4, 2, 5_000_000))), 62_500);
        assert_eq!(railway_health(Some(&row(4, 2, i64::MAX))), -1);
    }

    #[test]
    fn shared_slots_scale_the_owners_slots_and_clamp_to_the_define() {
        let inputs = |local, factor, extra| StateBuildingInputRow {
            owner: 3,
            extra_shared_slots: extra,
            local_building_slots: Some(local),
            local_building_slots_factor: Some(factor),
            global_building_slots: Some(100_000),
            global_building_slots_factor: Some(0),
            ..Default::default()
        };
        assert_eq!(shared_slots(&inputs(300_000, 50_000, 1), 3, 25), Ok(7), "4 slots at +50% is 6, and one extra");
        assert_eq!(shared_slots(&inputs(-500_000, 50_000, 0), 3, 25), Ok(0), "a negative base is not scaled, and floors at zero");
        assert_eq!(shared_slots(&inputs(3_000_000, 0, 0), 3, 25), Ok(25));
        assert!(shared_slots(&inputs(300_000, 0, 0), 4, 25).unwrap_err().contains("another owner"));
        let refused = StateBuildingInputRow { owner: 3, ..Default::default() };
        assert!(shared_slots(&refused, 3, 25).unwrap_err().contains("refused"));
    }

    #[test]
    fn energy_usage_sums_each_active_buildings_groups() {
        use content_reader_key_names::buildings::FactoryFlags;
        let inputs = StateBuildingInputRow {
            controller: 5,
            local_factories: Some(0),
            local_factory_energy_consumption: Some(0),
            local_factory_energy_consumption_per_infrastructure: Some(10_000),
            factory_energy_consumption: Some(0),
            factory_energy_factor: Some(50_000),
            ..Default::default()
        };
        let civilian = FactoryFlags { general_production: 1, ..Default::default() };
        let both = FactoryFlags { military_production: 1, general_production: 1, ..Default::default() };
        assert_eq!(energy_usage(&inputs, 5, 2, &[(3, civilian), (1, both), (9, FactoryFlags::default())]), Ok(300_000));
        assert!(energy_usage(&inputs, 6, 2, &[]).unwrap_err().contains("another controller"));
        assert_eq!(round_whole(250_000), 3);
        assert_eq!(round_whole(-250_000), -3);
        assert_eq!(round_whole(249_999), 2);
    }

    #[test]
    fn the_damage_bar_shows_the_damaged_share_in_whole_percent() {
        assert_eq!(damage_frame(0, 0), 0);
        assert_eq!(damage_frame(5, 5), 0);
        assert_eq!(damage_frame(3, 2), 33);
        assert_eq!(damage_frame(4, 0), 100);
        assert_eq!(frame(0), u32::MAX);
        assert_eq!(frame(33), 32);
    }

    #[test]
    fn dlc_gated_types_hide_without_their_feature_or_a_failing_dlc_trigger() {
        let gun = BuildingType { gun_emplacement: true, ..Default::default() };
        assert_eq!(dlc_hidden(&gun, 0, |_| unreachable!()), Ok(true));
        assert_eq!(dlc_hidden(&gun, GUN_EMPLACEMENT_FEATURE, |_| unreachable!()), Ok(false));
        let headquarter = BuildingType { naval_headquarter: true, ..Default::default() };
        assert_eq!(dlc_hidden(&headquarter, GUN_EMPLACEMENT_FEATURE, |_| unreachable!()), Ok(true));
        let gated = BuildingType { dlc_allowed: content_reader_parser::parser::parse(b"always = no").unwrap().items, ..Default::default() };
        assert_eq!(dlc_hidden(&gated, 0, |_| Ok(false)), Ok(true));
        assert_eq!(dlc_hidden(&gated, 0, |_| Ok(true)), Ok(false));
        assert!(dlc_hidden(&gated, 0, |_| Err("undecided".into())).is_err());
    }

    #[test]
    fn only_a_non_zero_state_modifier_counts() {
        let with = |text: &[u8]| BuildingType { state_modifiers: content_reader_parser::parser::parse(text).unwrap().items, ..Default::default() };
        assert_eq!(has_state_modifier(&with(b"")), Ok(false));
        assert_eq!(has_state_modifier(&with(b"stability_factor = 0")), Ok(false));
        assert_eq!(has_state_modifier(&with(b"stability_factor = 0 local_supplies = 0.5")), Ok(true));
        assert!(has_state_modifier(&with(b"stability_factor = { a = 1 }")).is_err());
    }
}
