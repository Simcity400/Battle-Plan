use crate::base_windows;
use content_reader_gui::gui::native;
use crate::{naval_markers, naval_mine_badges, naval_missions, naval_windows};
use crate::naval::{NavalContext, NavalSources, NavalUnit};
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::io;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use viewer_counter_kit::{
    CounterFrame, CounterInputs, CounterReport, Countries, NavalRegionalControl, NavalRegionalHover,
    gui_windows, visible,
};
use viewer_foundation::allegiance::Allegiance;
use viewer_foundation::mode_visibility::PriorityTable;
use viewer_foundation::perspective::{Sight, Vision};
use viewer_runtime::unit_handle::Unit;
use viewer_unit_reads::counter_inputs;
struct RaidingGrid<'a> {
    active: u8,
    row: &'a replay_data::rows::NavalMissionTaskForceRow,
}
use viewer_runtime::msvc_stable_sort::sort;

pub type Fact<T> = Result<T, String>;

#[derive(Debug)]
pub struct Region {
    pub badges: naval_mine_badges::Packet,
    pub grids: Vec<naval_windows::GridFill>,
    pub combats: Fact<Vec<u64>>,
    pub results: Fact<Vec<u64>>,
}

impl Default for Region {
    fn default() -> Self {
        Self {
            grids: Vec::new(),
            badges: Default::default(),
            combats: Ok(Vec::new()),
            results: Ok(Vec::new()),
        }
    }
}

fn keyed<T, K: Ord>(rows: &[T], key: K, field: impl FnMut(&T) -> K) -> Option<&T> {
    rows.binary_search_by_key(&key, field)
        .ok()
        .map(|at| &rows[at])
}

fn raiding_visible(grid: &RaidingGrid<'_>, region: u32) -> Fact<bool> {
    if grid.active == 0 {
        return Ok(false);
    }
    if grid
        .row
        .raiding_region
        .ok_or("regional raider retained region is unavailable")?
        != region
    {
        return Ok(false);
    }
    let targets = grid
        .row
        .raiding_targets
        .ok_or("regional raider target references are unavailable")?;
    if targets.contains(&Some(true)) {
        return Ok(true);
    }
    if targets.iter().all(Option::is_some) {
        return Ok(false);
    }
    Err("a regional raider target's native liveness is unavailable".into())
}

fn assigned(
    kind: u32,
    region: u32,
    current: u32,
    home: Option<u32>,
    bombardment: Option<u32>,
    accessible: &[u32],
) -> Fact<bool> {
    Ok(match kind {
        0 | 2 | 8 => false,
        1 => current == region,
        7 => home.ok_or("training task force home sea region is unavailable")? == region,
        9 => bombardment.ok_or("invasion support bombardment region is unavailable")? == region,
        _ => accessible.contains(&region),
    })
}

fn searched(
    row: &replay_data::rows::TaskForceCounterInputRow,
    hour: i32,
    ours: u32,
    owner: &impl Fn(u32) -> Option<u32>,
) -> Fact<bool> {
    let packet = row
        .port_mission
        .require("TaskForceCounterInputs.port_mission")?
        .as_ref()
        .ok_or("task force searching references are unavailable")?;
    if !packet.sample_hour.fresh_at(hour) {
        return Err("task force searching references are stale".into());
    }
    let list = packet
        .searching
        .as_ref()
        .ok_or("task force searching list is unavailable")?;
    let mut unknown = false;
    for searcher in list {
        match searcher.resolves {
            Some(false) => {}
            Some(true) => match searcher.roster_owner.and_then(owner) {
                Some(slot) if slot == ours => return Ok(true),
                Some(_) => {}
                None => unknown = true,
            },
            None => unknown = true,
        }
    }
    if unknown {
        Err("a task force searcher's owner or liveness is unavailable".into())
    } else {
        Ok(false)
    }
}

#[derive(Clone, Copy)]
pub struct SortKey {
    pub selected: bool,
    pub own: bool,
    pub full: bool,
    pub refused: bool,
    pub reserve: bool,
    pub fleet: u64,
    pub handle: u64,
    pub mission: u32,
}
pub fn compare(a: SortKey, b: SortKey, coarse: bool) -> i32 {
    let first = |yes| if yes { -1 } else { 1 };
    if a.selected != b.selected {
        return first(a.selected);
    }
    if a.own != b.own {
        return first(a.own);
    }
    if !a.own {
        if a.full != b.full {
            return first(a.full);
        }
    } else {
        if a.refused != b.refused {
            return first(a.refused);
        }
        if a.reserve != b.reserve {
            return first(b.reserve);
        }
        if !a.reserve && a.fleet != b.fleet {
            return viewer_counter_kit::naval_combat_grid::fleet_difference(a.fleet, b.fleet);
        }
    }
    if a.selected {
        return viewer_counter_kit::naval_combat_grid::fleet_difference(a.handle, b.handle);
    }
    if !coarse {
        return a.mission.cmp(&b.mission) as i32;
    }
    0
}

pub fn allegiance_break(
    first: (u32, Option<Allegiance>),
    next: (u32, Option<Allegiance>),
    first_selected: bool,
    next_selected: bool,
) -> bool {
    match (first.1, next.1) {
        (Some(Allegiance::Ours), Some(Allegiance::Ours)) => first_selected && !next_selected,
        (Some(first), Some(next)) => {
            let rank = |side| match side {
                Allegiance::Ours => 0,
                Allegiance::Allied => 1,
                Allegiance::Enemy => 2,
                Allegiance::Other => 3,
            };
            rank(first) < rank(next)
        }
        _ => first.0 != next.0,
    }
}

pub fn sort_own(
    units: &mut [NavalUnit<'_>],
    selected_units: &BTreeSet<Unit>,
    context: &NavalContext<'_>,
    coarse: bool,
) -> Result<Vec<SortKey>, &'static str> {
    let mut keys = Vec::with_capacity(units.len());
    let mut own_counts = [0usize; 2];
    for unit in units
        .iter()
        .filter(|unit| unit.slot == context.vision.ours())
    {
        own_counts[usize::from(selected_units.contains(&Unit::TaskForce(unit.force.id)))] += 1;
    }
    for unit in units.iter() {
        let own = unit.slot == context.vision.ours();
        let selected = selected_units.contains(&Unit::TaskForce(unit.force.id));
        let refused = if own && own_counts[usize::from(selected)] > 1 {
            let row = context
                .native
                .task_force(unit.force.id)
                .ok_or("regional own sort lacks engagement inputs")?;
            counter_inputs::engagement_refused(
                row.running_away,
                row.engagement_rule,
                unit.force.mission,
                context.aggression,
            )
            .ok_or("regional own sort lacks engagement content")?
        } else {
            false
        };
        keys.push(SortKey {
            selected,
            own,
            full: unit.byte >= 250,
            refused,
            reserve: unit.force.mission == 8,
            fleet: unit.force.fleet,
            handle: unit.force.id,
            mission: unit.force.mission,
        });
    }
    let mut order: Vec<_> = (0..units.len()).collect();
    sort(&mut order, |&a, &b| compare(keys[a], keys[b], coarse) < 0);
    let mut placed = vec![false; units.len()];
    for start in 0..units.len() {
        let mut at = start;
        while !placed[at] {
            placed[at] = true;
            let from = order[at];
            if from == start {
                break;
            }
            units.swap(at, from);
            at = from;
        }
    }
    Ok(order.iter().map(|&index| keys[index]).collect())
}

pub fn prepare<'a>(
    inputs: &CounterInputs<'_>,
    frame: &'a DecodedFrame,
    vision: &'a Vision,
    owner: &impl Fn(u32) -> Option<u32>,
    context: Result<&NavalContext<'a>, NavalSources<'a>>,
    countries: &Countries,
    out: &mut CounterFrame<impl Sized, base_windows::PreparedBases, impl Sized, naval_windows::Plan>,
) -> io::Result<()> {
    if inputs.visibility.priorities != PriorityTable::StrategicNavy {
        return Ok(());
    }
    out.naval.missions.mode = 2;
    let Some(regions) = out.naval.missions.regions.as_ref() else {
        return Ok(());
    };
    let region_ids: Vec<_> = regions.keys().copied().collect();
    let mut badge_packets = naval_mine_badges::prepare(inputs, frame, vision, &region_ids);
    let owned;
    let context = match context {
        Ok(c) => c,
        Err(s) => {
            owned = NavalContext::build(frame, s)?;
            &owned
        }
    };
    let forces: BTreeMap<_, _> = context.forces.iter().map(|r| (r.id, r)).collect();
    let ledgers = frame
        .complete::<tables::NavalMissionLedgers>()
        .map_err(|e| e.to_string());
    let missions = frame
        .complete::<tables::TaskForceMissions>()
        .map_err(|e| e.to_string());
    let locations = frame
        .complete::<tables::Provinces>()
        .map_err(|e| e.to_string());
    let fleets = frame
        .complete::<tables::Fleets>()
        .map_err(|e| e.to_string());
    let order = frame
        .complete::<tables::ProvinceUnitOrder>()
        .map_err(|e| e.to_string());
    let combats = frame
        .complete::<tables::NavalProvinceCombats>()
        .map_err(|e| e.to_string());
    let details = frame
        .complete::<tables::NavalCombatDetails>()
        .map_err(|e| e.to_string());
    let raiders = frame
        .complete::<tables::NavalMissionTaskForces>()
        .map_err(|e| e.to_string());
    let results = naval_markers::result_tables(frame, naval_markers::Viewer { vision, owner })?;
    let ours = vision.ours();
    let ledgers = ledgers.map(|rows| {
        rows.iter()
            .filter(|row| owner(row.owner) == Some(ours))
            .map(|row| (row.region, row))
            .collect::<BTreeMap<_, _>>()
    });
    let art = gui_windows::CounterArt {
        assets: inputs.assets,
        countries,
    };
    let resources = viewer_counter_kit::gui_windows::gui_assets(inputs, None);
    let navy = gui_windows::NavyContext {
        settings: inputs.settings,
        selected: inputs.selected,
        defines: &inputs.content.defines,
        art: &art,
        gui: &resources,
    };
    let template = inputs.windows.navy();
    let root = naval_missions::MissionWindows::find(inputs.windows).ok();
    for region in region_ids {
        let mut display = Region {
            badges: badge_packets.remove(&region).unwrap_or_default(),
            ..Region::default()
        };
        let height = inputs
            .content
            .region_icon(region)
            .map_or(inputs.camera.height, |anchor| inputs.icon_height(anchor));
        let detailed = visible(height, inputs.content.defines.icons_cutoff);
        let allegiance = inputs
            .content
            .naval_mission_allegiance_group_cutoff
            .as_ref()
            .map(|&cutoff| height > cutoff)
            .map_err(Clone::clone);
        let coarse = inputs
            .content
            .defines
            .coarse(height, inputs.visibility.strategic_grouping)
            || allegiance == Ok(true);
        if !detailed {
            out.naval.missions.regional.insert(region, display);
            continue;
        }
        let sea: Vec<_> = inputs
            .content
            .region_provinces(region)
            .iter()
            .copied()
            .filter(|&id| !viewer_counter_kit::is_land(inputs, id))
            .collect();
        let own_units = (|| -> Fact<(Vec<NavalUnit<'a>>, Vec<SortKey>)> {
            if !context.forces_complete || !context.ships_complete {
                return Err("regional own grid needs complete task-force and ship censuses".into());
            }
            let ledger = ledgers
                .as_ref()
                .map_err(Clone::clone)?
                .get(&region)
                .copied();
            let mut units = Vec::new();
            for (ordinal, &handle) in ledger
                .into_iter()
                .flat_map(|r| r.task_forces.iter())
                .enumerate()
            {
                let force = *forces
                    .get(&handle)
                    .ok_or("regional naval registration task force is unavailable")?;
                if matches!(force.mission, 0 | 2 | 8)
                    || force.fleet == 0
                    || force.combat_references != 0
                    || context.ships.get(&handle).is_none_or(|s| s.0 == 0)
                    || inputs.content.region(force.province) != Some(region)
                {
                    continue;
                }
                let mission = keyed(missions.as_ref().map_err(Clone::clone)?, handle, |r| {
                    r.task_force
                });
                let home = if force.mission == 7 {
                    let fleet = keyed(fleets.as_ref().map_err(Clone::clone)?, force.fleet, |r| {
                        r.id
                    })
                    .ok_or("training fleet is unavailable")?;
                    let port = keyed(
                        locations.as_ref().map_err(Clone::clone)?,
                        fleet.home_base,
                        |r| r.id,
                    )
                    .ok_or("training home base is unavailable")?;
                    Some(inputs.content.region(port.port_sea as u32).unwrap_or(0))
                } else {
                    None
                };
                if !assigned(
                    force.mission,
                    region,
                    mission.map_or(0, |r| r.current_region),
                    home,
                    context
                        .native
                        .task_force(handle)
                        .map(|r| r.bombardment_region),
                    mission.map_or(&[], |r| r.regions.as_slice()),
                )? {
                    continue;
                }
                let unit = context
                    .unit(force, owner, ordinal as u32, None)
                    .ok_or("regional registered task-force owner or visibility is unavailable")?;
                units.push(unit);
            }
            let keys = sort_own(&mut units, inputs.selected, context, coarse)?;
            Ok((units, keys))
        })();
        let mut own_calls = Vec::new();
        match own_units {
            Err(why) => own_calls.push(Err(why.into())),
            Ok((units, keys)) => {
                let mut start = 0;
                while start < units.len() {
                    let mut end = start + 1;
                    match &allegiance {
                        Ok(true) => {
                            while end < units.len()
                                && !allegiance_break(
                                    (
                                        units[start].slot,
                                        countries.known_side(units[start].slot),
                                    ),
                                    (units[end].slot, countries.known_side(units[end].slot)),
                                    keys[start].selected,
                                    keys[end].selected,
                                )
                            {
                                end += 1;
                            }
                        }
                        Ok(false) => {
                            while end < units.len() && compare(keys[start], keys[end], coarse) >= 0
                            {
                                end += 1;
                            }
                        }
                        Err(why) => {
                            own_calls.push(Err(why.clone().into()));
                            break;
                        }
                    }
                    let members: Vec<_> = units[start..end].iter().collect();
                    let flotilla = context.flotilla(&members, true, false);
                    own_calls.push(Ok(gui_windows::navy_calls(
                        &navy,
                        template,
                        &gui_windows::NavyState {
                            flotilla: &flotilla,
                            countries,
                        },
                        &mut out.report.navy_counter_gaps,
                    )));
                    start = end;
                }
            }
        }
        let enemy_calls = (|| -> Fact<Vec<Result<viewer_counter_kit::gui_windows::Update, Cow<'static, str>>>> {
            if !context.forces_complete || !context.ships_complete {
                return Err(
                    "regional enemy grid needs complete task-force and ship censuses".into(),
                );
            }
            let order = order.as_ref().map_err(Clone::clone)?;
            let mut calls = Vec::new();
            for province in &sea {
                let roster = keyed(order, *province, |r| r.province)
                    .ok_or("regional sea province occupant order is unavailable")?;
                for (ordinal, &handle) in roster.mixed.iter().enumerate() {
                    let Some(&force) = forces.get(&handle) else {
                        continue;
                    };
                    if force.combat_references != 0
                        || context.ships.get(&handle).is_none_or(|s| s.0 == 0)
                    {
                        continue;
                    }
                    let slot = owner(force.roster_owner)
                        .ok_or("regional enemy task force country is unavailable")?;
                    let byte = vision
                        .navy_byte_known(force.id, slot)
                        .ok_or("regional enemy task force visibility is unavailable")?;
                    if byte == 0 {
                        let row = context
                            .native
                            .task_force(handle)
                            .ok_or("regional enemy searching inputs are unavailable")?;
                        if !searched(row, frame.hour, ours, owner)? {
                            continue;
                        }
                    }
                    if !vision.at_war_proven(slot).map_err(|e| e.describe())? {
                        continue;
                    }
                    let mut unit =
                        context
                            .unit(force, owner, ordinal as u32, None)
                            .unwrap_or(NavalUnit {
                                force,
                                ordinal: ordinal as u32,
                                slot,
                                logical: owner(force.logical_owner),
                                flag_owner: slot,
                                sight: Sight::None,
                                byte,
                                ships: Some(context.ships[&handle].0),
                                position: None,
                                strength: context.ships[&handle].1,
                            });
                    unit.byte = byte;
                    let flotilla = context.flotilla(&[&unit], true, false);
                    calls.push(Ok(gui_windows::navy_calls(
                        &navy,
                        template,
                        &gui_windows::NavyState {
                            flotilla: &flotilla,
                            countries,
                        },
                        &mut out.report.navy_counter_gaps,
                    )));
                }
            }
            Ok(calls)
        })()
        .unwrap_or_else(|why| vec![Err(why.into())]);
        let convoy_calls = (|| -> Fact<Vec<Result<viewer_counter_kit::gui_windows::Update, Cow<'static, str>>>> {
            let mut calls = Vec::new();
            let ledger = ledgers
                .as_ref()
                .map_err(Clone::clone)?
                .get(&region)
                .copied();
            for &handle in ledger.into_iter().flat_map(|r| &r.task_forces) {
                let force = *forces
                    .get(&handle)
                    .ok_or("regional convoy registration task force is unavailable")?;
                if force.mission != 3 {
                    continue;
                }
                let packet = keyed(raiders.as_ref().map_err(Clone::clone)?, handle, |r| {
                    r.task_force
                })
                .ok_or("regional raider grid inputs are unavailable")?;
                let active = context
                    .native
                    .task_force(handle)
                    .ok_or("regional raider activation is unavailable")?
                    .is_in_regions;
                if !raiding_visible(
                    &RaidingGrid {
                        active,
                        row: packet,
                    },
                    region,
                )? {
                    continue;
                }
                let tiny = inputs
                    .windows
                    .tiny()
                    .ok_or(gui_windows::CONVOY_WINDOW_ABSENT)?;
                let mut update = viewer_counter_kit::gui_windows::Update {
                    naval_convoy: Some(handle),
                    calls: gui_windows::convoy_calls(
                        &inputs.assets.gui,
                        tiny,
                        0,
                        inputs.content.defines.tints.enemy,
                        &mut out.report.naval_grid_gaps,
                    ),
                    ..viewer_counter_kit::gui_windows::Update::default()
                };
                if let Some(count) = tiny.find(native::Kind::TextBox, "count") {
                    update.calls.push((
                        count.path.clone(),
                        viewer_native_gui::native_gui::lifecycle::Call::SetText(
                            "?".to_owned(),
                        ),
                    ));
                }
                if let Some(container) =
                    tiny.find(native::Kind::Container, "spotting_bar_container")
                {
                    update.calls.push((
                        container.path.clone(),
                        viewer_native_gui::native_gui::lifecycle::Call::SetHideFlag(false),
                    ));
                    update.calls.push((
                        container.path.clone(),
                        viewer_native_gui::native_gui::lifecycle::Call::Show,
                    ));
                }
                if let Some(foreground) = tiny.find(native::Kind::Icon, "spotting_bar_fg") {
                    let extent = viewer_native_gui::native_gui::drawable_extent(
                        foreground,
                        &resources,
                        &Default::default(),
                    )
                    .map_err(|why| {
                        why.iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join("; ")
                    })?;
                    let height = extent.map_or(0, |size| size[1]);
                    let y = content_reader_parser::crt::truncate_f32(
                        (1.0 - (packet
                            .raiding_spotting
                            .ok_or("regional convoy spotting is unavailable")?
                            as f32
                            / 100_000.0)
                            / 100.0)
                            * height as f32,
                    );
                    let mut bound = viewer_native_gui::native_gui::Bound::default();
                    bound.position = Some([0, y]);
                    update.calls.push((
                        foreground.path.clone(),
                        viewer_native_gui::native_gui::lifecycle::Call::Control(Box::new(
                            bound,
                        )),
                    ));
                }
                calls.push(Ok(update));
            }
            Ok(calls)
        })()
        .unwrap_or_else(|why| vec![Err(why.into())]);
        for (name, side, items) in [
            ("unit_grid", Some(0), own_calls),
            ("enemy_unit_grid", Some(1), enemy_calls),
            ("enemy_convoy_grid", None, convoy_calls),
        ] {
            if let Some(grid) = root
                .as_ref()
                .and_then(|r| r.root.find(native::Kind::Grid, name))
            {
                display.grids.push(naval_windows::GridFill {
                    grid: grid.path.clone(),
                    side,
                    items,
                });
            }
        }
        display.combats = (|| -> Fact<Vec<u64>> {
            let direct = combats.as_ref().map_err(Clone::clone)?;
            let details = details.as_ref().map_err(Clone::clone)?;
            let mut visible = Vec::new();
            for province in &sea {
                if let Some(row) = keyed(direct, *province, |r| r.province) {
                    for (&combat, &kind) in row.combats.iter().zip(&row.kinds) {
                        if kind != replay_data::rows::NavalProvinceCombatRow::NAVAL {
                            continue;
                        }
                        let detail = keyed(details, combat, |r| r.combat)
                            .ok_or("regional naval combat country lists are unavailable")?;
                        let mut unresolved = false;
                        let admitted = detail
                            .sides
                            .iter()
                            .flat_map(|side| &side.summary.countries)
                            .any(|&raw| match owner(raw) {
                                Some(slot) => slot == ours,
                                None => {
                                    unresolved = true;
                                    false
                                }
                            });
                        if admitted {
                            visible.push(combat);
                        } else if unresolved {
                            return Err("regional naval combat country is unresolved".into());
                        }
                    }
                }
            }
            Ok(visible)
        })();
        display.results = (|| -> Fact<Vec<u64>> {
            let mut visible = Vec::new();
            for province in &sea {
                visible.extend(results.visible(*province).map_err(str::to_owned)?);
            }
            Ok(visible)
        })();
        out.naval.missions.regional.insert(region, display);
    }
    Ok(())
}

#[expect(clippy::too_many_arguments, reason = "the region, the window's root and layout, its placement, the resources, the layer and the report")]
pub fn control_hovers(
    region: u32,
    data: Option<&Region>,
    root: &native::Element,
    layout: &viewer_native_gui::native_gui::Layout,
    origin: [i32; 2],
    resources: &dyn viewer_native_gui::native_gui::Resources,
    layer: u64,
    report: &mut CounterReport,
) {
    for hit_region in &layout.regions {
        let Some(element) = viewer_counter_kit::gui_windows::find_path(root, &hit_region.path) else {
            continue;
        };
        let arm = [
            ("ongoing_combat_container", 0),
            ("combat_results_container", 1),
            ("mine_status_container", 2),
            ("accidents_container", 3),
        ]
        .into_iter()
        .find_map(|(name, arm)| {
            let container = root.find(native::Kind::Container, name)?;
            (element.path == container.path
                || element.path.starts_with(&format!("{}.", container.path)))
            .then_some(arm)
        });
        let Some(arm) = arm else {
            continue;
        };
        let control = match arm {
            0 => NavalRegionalControl::Combat(
                data.and_then(|data| data.combats.as_ref().ok())
                    .map_or(0, Vec::len),
            ),
            1 => NavalRegionalControl::Results(
                data.and_then(|data| data.results.as_ref().ok())
                    .map_or(0, Vec::len),
            ),
            2 => NavalRegionalControl::Mines,
            _ => NavalRegionalControl::Accidents,
        };
        let Some(hit) = viewer_counter_kit::gui_windows::native_hit_rectangle(
            element,
            layout,
            origin,
            resources,
            &mut report.icon_gui_notes,
        ) else {
            continue;
        };
        let text = match control {
            NavalRegionalControl::Combat(count) | NavalRegionalControl::Results(count) => {
                let key = if arm == 0 {
                    "ACTIVE_COMBAT_STATUS"
                } else {
                    "COMBAT_RESULT_STATUS"
                };
                let mut tooltip = viewer_foundation::tooltip_text::Tooltip::default();
                tooltip.line(
                    false,
                    0,
                    key,
                    viewer_foundation::tooltip_text::localise(
                        resources.localisation(),
                        key,
                        &[(
                            "COUNT",
                            content_reader_localisation::localisation::Argument::Int32(
                                count as u32 as i32,
                            ),
                        )],
                    ),
                );
                Some(Arc::new(tooltip))
            }
            _ => None,
        };
        report.naval_region_hovers.push(NavalRegionalHover {
            region,
            control,
            rect: hit.rect,
            layer,
            hit: hit.geometry,
            supported: hit.supported,
            text,
        });
    }
}

#[cfg(test)]
mod tests {
use super::{
        Allegiance, RaidingGrid, SortKey, allegiance_break, assigned, compare, raiding_visible, searched,
    };
    #[test]
    fn regional_raider_admission_keeps_native_or_and_demand_gates() {
        let mut row = replay_data::rows::NavalMissionTaskForceRow {
            raiding_region: Some(4),
            raiding_targets: Some([None, Some(true)]),
            ..Default::default()
        };
        assert_eq!(
            raiding_visible(
                &RaidingGrid {
                    active: 1,
                    row: &row
                },
                4
            ),
            Ok(true)
        );
        row.raiding_targets = Some([None, Some(false)]);
        assert!(
            raiding_visible(
                &RaidingGrid {
                    active: 1,
                    row: &row
                },
                4
            )
            .is_err()
        );
        assert_eq!(
            raiding_visible(
                &RaidingGrid {
                    active: 1,
                    row: &row
                },
                5
            ),
            Ok(false)
        );
        assert_eq!(
            raiding_visible(
                &RaidingGrid {
                    active: 0,
                    row: &row
                },
                4
            ),
            Ok(false)
        );
        row.raiding_targets = Some([Some(false); 2]);
        assert_eq!(
            raiding_visible(
                &RaidingGrid {
                    active: 2,
                    row: &row
                },
                4
            ),
            Ok(false)
        );
    }
    #[test]
    fn regional_assignment_preserves_kind_specific_regions() {
        assert_eq!(assigned(10, 4, 0, None, None, &[4]), Ok(true));
        for kind in [0, 2, 8] {
            assert_eq!(assigned(kind, 4, 4, Some(4), Some(4), &[4]), Ok(false));
        }
        assert_eq!(assigned(1, 4, 3, None, None, &[4]), Ok(false));
        assert_eq!(assigned(7, 4, 3, Some(4), None, &[]), Ok(true));
        assert_eq!(assigned(9, 4, 0, None, Some(4), &[]), Ok(true));
        assert!(assigned(7, 4, 4, None, None, &[4]).is_err());
    }
    #[test]
    fn alternate_detection_accepts_a_later_matching_searcher_without_inventing_a_negative() {
        use replay_data::{
            rows::NavalPortMissionInputs, rows::NavalSearcher, rows::PacketHour, rows::Recorded,
            rows::TaskForceCounterInputRow,
        };
        let mut row = TaskForceCounterInputRow {
            port_mission: Recorded::Value(Some(NavalPortMissionInputs {
                sample_hour: PacketHour::Host,
                searching: Some(vec![
                    NavalSearcher::default(),
                    NavalSearcher {
                        reference: Some(9),
                        resolves: Some(true),
                        roster_owner: Some(17),
                    },
                ]),
                ..Default::default()
            })),
            ..Default::default()
        };
        assert_eq!(searched(&row, 5, 17, &Some), Ok(true));
        row.port_mission
            .require_mut("packet")
            .unwrap()
            .as_mut()
            .unwrap()
            .searching
            .as_mut()
            .unwrap()
            .pop();
        assert!(searched(&row, 5, 17, &Some).is_err());
        row.port_mission
            .require_mut("packet")
            .unwrap()
            .as_mut()
            .unwrap()
            .searching = Some(vec![NavalSearcher {
            reference: Some(0),
            resolves: Some(false),
            roster_owner: None,
        }]);
        assert_eq!(searched(&row, 5, 17, &Some), Ok(false));
    }

    #[test]
    fn allegiance_runs_split_only_when_the_first_members_priority_precedes_the_next() {
        assert!(allegiance_break(
            (1, Some(Allegiance::Allied)),
            (2, Some(Allegiance::Enemy)),
            false,
            false
        ));
        assert!(!allegiance_break(
            (2, Some(Allegiance::Enemy)),
            (1, Some(Allegiance::Allied)),
            false,
            false
        ));
        assert!(allegiance_break(
            (0, Some(Allegiance::Ours)),
            (0, Some(Allegiance::Ours)),
            true,
            false
        ));
        assert!(!allegiance_break(
            (0, Some(Allegiance::Ours)),
            (0, Some(Allegiance::Ours)),
            false,
            true
        ));
        assert!(!allegiance_break((1, None), (1, None), false, false));
        assert!(allegiance_break((1, None), (2, None), false, false));
    }

    #[test]
    fn detailed_order_uses_mission_after_native_fleet_and_selection_keys() {
        let a = SortKey {
            selected: false,
            own: true,
            full: true,
            refused: false,
            reserve: false,
            fleet: 1,
            handle: 4,
            mission: 1,
        };
        let mut b = a;
        b.mission = 9;
        assert!(compare(a, b, false) < 0);
        assert_eq!(compare(a, b, true), 0);
        b.selected = true;
        assert!(compare(b, a, false) < 0);
        b = a;
        b.fleet = 2;
        assert!(compare(a, b, false) < 0);
    }
}
