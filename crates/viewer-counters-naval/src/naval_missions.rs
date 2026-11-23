
use crate::{base_windows, naval_windows};
use crate::naval_windows::{Mark, mark, marks};
use content_reader_core::gui::native;
use crate::{naval_mine_badges, naval_regional};
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::io;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use viewer_counter_kit::{
    CounterDefines, CounterFrame, CounterInputs, CounterReport, NavalMissionHit,
    NavalRegionalControl, visible,
};
use viewer_foundation::mode_visibility::PriorityIcon;
use viewer_foundation::perspective::Vision;
use viewer_foundation::tables::rows;
use viewer_gpu::icon_tracks::IconFamily;
use viewer_native_gui::sprites::{Batch, layer};
use viewer_unit_reads::counter_inputs;
use viewer_unit_reads::counter_inputs::NativeInputs;
#[cfg(test)]
use viewer_native_gui::native_windows::MAPICONS_GUI;
use std::collections::HashMap;
use viewer_native_gui::gui_resources::GuiAssets;
use viewer_native_gui::gui_resources::count_noted;
use viewer_native_gui::gui_resources::count_refused;
use viewer_native_gui::gui_resources::viewport as gui_resources_viewport;
use viewer_native_gui::icon_windows::POOL_EXHAUSTED;
use viewer_native_gui::icon_windows::Retry;
use viewer_native_gui::native_gui::{
    self,
    lifecycle::{Admission, Held, Instance, MapIconPool, Pool, Rewrote, SlotId, Tick, Unknown, Withheld},
};
use viewer_native_gui::native_text::COLOR;
use viewer_native_gui::native_windows::NativeWindows;
use viewer_gpu::province_schedule::{Schedule, Update};

pub const MISSION_SPRITES: [(u32, &str); 8] = [
    (1, "GFX_mapicon_naval_mission_patrol"),
    (2, "GFX_mapicon_naval_mission_strike_force"),
    (3, "GFX_mapicon_naval_mission_convoy_raid"),
    (4, "GFX_mapicon_naval_mission_convoy_escort"),
    (5, "GFX_mapicon_naval_mission_mines_planting"),
    (6, "GFX_mapicon_naval_mission_mines_sweeping"),
    (7, "GFX_mapicon_naval_mission_training"),
    (9, "GFX_mapicon_naval_invasion_support"),
];

const UNCOUNTED: [u32; 3] = [0, 8, 10];

pub const MISSION_ENTRY_WINDOW: &str = "naval_mission_mapicon_entry";
pub const MISSION_WINDOW: &str = "naval_mission_mapicon";
const CONTENT_WINDOW: &str = "content";
const MISSIONS_LIST: &str = "missions_list";

const DRAWN_UI_MODE: i32 = 0;

const P2_ACTIVE: &str = "P2 active registrations need complete native range operands";
const P3_RAIDING: &str = "P3 raiding efficiency needs complete native operands";
const P3_ESCORT: &str = "P3 escort efficiency needs complete native operands";
const P3_ESCORT_ALERT: &str = "P3 escort alert needs complete efficiency and presence operands";

const PRIDE_OWNER_UNLINKED: &str = "a registered task force's roster owner is not linked to a \
     recorded country";
const PRIDE_SHIP_UNRECORDED: &str = "a registered task force's country has no recorded pride of \
     the fleet";
const PRIDE_SHIPS_INCOMPLETE: &str =
    "the pride of the fleet is absent from an incomplete Ships census";
pub const ENGAGEMENT_UNKNOWN: &str =
    "a registered task force has no TaskForceCounterInputs engagement rule";

const PRIDE: &str = "its pride marker";
const COUNT: &str = "its count";
const ENTRY_PLACE: &str = "its place in the missions list";
const ENTRY_KEPT: [&str; 2] = [COUNT, ENTRY_PLACE];
const LIST_PLACE: &str = "its missions list's place";
const UNANCHORED_PLACE: &str = "its strategic region has no icon position";
const UNPROJECTED_PLACE: &str = "its icon position cannot be projected";

const NAVAL_UI_MODE: i32 = 2;

const COUNTED_CONTAINERS: [(&str, &str); 4] = [
    (
        "mine_status_container",
        "DLC feature 0x13 and the region's own and enemy mine values",
    ),
    (
        "ongoing_combat_container",
        "the region's visible naval combats",
    ),
    (
        "combat_results_container",
        "the region's visible naval combat results",
    ),
    (
        "accidents_container",
        "the strategic navy's accident reports in the region",
    ),
];

pub struct MissionWindows<'a> {
    pub root: &'a native::Element,
    content: &'a native::Element,
    list: &'a native::Element,
    pub entry: &'a native::Element,
}

impl<'a> MissionWindows<'a> {
    pub fn find(windows: &'a NativeWindows) -> Result<Self, native_gui::Refusal> {
        let missing = |path: &str, detail: String| {
            refusal(path, native_gui::RefusalKind::MissingInput, detail)
        };
        let root = windows
            .gui_type(MISSION_WINDOW.as_bytes())
            .filter(|window| window.kind == native::Kind::Container)
            .ok_or_else(|| {
                missing(
                    MISSION_WINDOW,
                    format!("no .gui file declares the containerWindowType {MISSION_WINDOW}"),
                )
            })?;
        let content = root
            .find(native::Kind::Container, CONTENT_WINDOW)
            .ok_or_else(|| missing(&root.path, format!("declares no window {CONTENT_WINDOW}")))?;
        let list = content
            .find(native::Kind::List, MISSIONS_LIST)
            .ok_or_else(|| missing(&content.path, format!("declares no list {MISSIONS_LIST}")))?;
        let gui_type = |name: &[u8]| windows.gui_type(name);
        let entry = match native_gui::list_entry(
            &list.path,
            MISSION_ENTRY_WINDOW.as_bytes(),
            &gui_type,
        )? {
            native_gui::ListEntry::Container(entry) => entry,
            native_gui::ListEntry::Fixed(window) => {
                return Err(refusal(
                    &list.path,
                    native_gui::RefusalKind::TypeConfusion,
                    format!(
                        "makes its entries of the windowType {}: the icon's construction reads it as a container window, whose control arrays lie in the fixed window's uninitialised callback storage",
                        window.path
                    ),
                ));
            }
        };
        Ok(Self {
            root,
            content,
            list,
            entry,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MissionEntry {
    pub kind: u32,
    pub total: u32,
    pub active: Result<Option<u32>, &'static str>,
    pub pride: Result<bool, &'static str>,
    pub alert: Result<bool, &'static str>,
    pub engagement: Result<bool, &'static str>,
    pub efficiency: Option<Result<i64, &'static str>>,
}

type Registration = (u64, u32, u32, Result<bool, &'static str>);

#[derive(Clone, Debug, Default)]
pub struct MissionIcons {
    pub(crate) hovers: Arc<super::naval_mission_inputs::hover::Hovers>,
    pub(crate) convoy_hovers: Arc<BTreeMap<u64, super::naval_mission_inputs::hover::Prepared>>,
    pub naval_dominance_sources:
        Option<Arc<viewer_unit_reads::naval_dominance_sources::RegionalDominanceSourcesMap>>,
    pub regions: BTreeMap<u32, Vec<MissionEntry>>,
    pub input_gaps: BTreeMap<String, usize>,
    unavailable: Option<&'static str>,
}

impl MissionIcons {
    pub fn read(
        frame: &DecodedFrame,
        vision: &Vision,
        links: &BTreeMap<u32, u32>,
        country_ids: &BTreeMap<u32, u32>,
        native: &NativeInputs,
        forces: &HashMap<u64, &replay_data::rows::TaskForceRow>,
    ) -> io::Result<Self> {
        let missions = rows::<tables::TaskForceMissions>(frame)?;
        let force_rows = rows::<tables::TaskForces>(frame)?;
        let ships = rows::<tables::Ships>(frame)?;
        if !missions.is_complete() || !force_rows.is_complete() {
            return Ok(Self {
                unavailable: Some("the task-force mission or task-force census is incomplete"),
                ..Self::default()
            });
        }
        let ours = vision.ours();
        let ledgers = rows::<tables::NavalMissionLedgers>(frame)?;
        if !ledgers.is_complete() {
            return Ok(Self {
                unavailable: Some("NavalMissionLedgers is incomplete"),
                ..Self::default()
            });
        }
        let missions: BTreeMap<_, _> = missions
            .rows()
            .iter()
            .map(|row| (row.task_force, row))
            .collect();
        let wanted: BTreeSet<_> = ledgers
            .rows()
            .iter()
            .filter(|row| country_ids.get(&ours) == Some(&row.owner))
            .flat_map(|row| &row.task_forces)
            .filter_map(|handle| forces.get(handle))
            .filter_map(|force| {
                links
                    .get(&force.roster_owner)
                    .and_then(|slot| country_ids.get(slot))
                    .and_then(|id| native.pride_ship(*id))
                    .filter(|&ship| ship != 0)
            })
            .collect();
        let ship_forces: HashMap<_, _> = ships
            .rows()
            .iter()
            .filter(|ship| wanted.contains(&ship.id))
            .map(|ship| (ship.id, ship.task_force))
            .collect();
        let mut ledger: BTreeMap<u32, Vec<Registration>> = BTreeMap::new();
        for row in ledgers
            .rows()
            .iter()
            .filter(|row| country_ids.get(&ours) == Some(&row.owner))
        {
            for &handle in &row.task_forces {
                let Some(force) = forces.get(&handle) else {
                    return Ok(Self {
                        unavailable: Some("a naval ledger registration has no task force"),
                        ..Self::default()
                    });
                };
                let current_region = missions.get(&handle).map_or(0, |row| row.current_region);
                let pride = match links
                    .get(&force.roster_owner)
                    .and_then(|slot| country_ids.get(slot))
                {
                    None => Err(PRIDE_OWNER_UNLINKED),
                    Some(id) => match native.pride_ship(*id) {
                        None => Err(PRIDE_SHIP_UNRECORDED),
                        Some(ship) => counter_inputs::pride(
                            force.id,
                            Some(ship),
                            |ship| ship_forces.get(&ship).copied(),
                            ships.is_complete(),
                        )
                        .ok_or(PRIDE_SHIPS_INCOMPLETE),
                    },
                };
                ledger.entry(row.region).or_default().push((
                    handle,
                    current_region,
                    force.mission,
                    pride,
                ));
            }
        }
        let regions = ledger
            .into_iter()
            .map(|(region, registrations)| (region, entries(region, &registrations, native)))
            .collect();
        Ok(Self {
            regions,
            unavailable: None,
            ..Self::default()
        })
    }
}

fn entries(
    region: u32,
    registrations: &[Registration],
    native: &NativeInputs,
) -> Vec<MissionEntry> {
    let mut totals: BTreeMap<u32, u32> = BTreeMap::new();
    let mut patrolling = 0;
    let mut last_pride: Result<Option<u32>, &'static str> = Ok(None);
    let mut no_engagement: BTreeMap<u32, Result<bool, &'static str>> = BTreeMap::new();
    for (handle, current_region, kind, pride) in registrations {
        if UNCOUNTED.contains(kind) {
            continue;
        }
        *totals.entry(*kind).or_default() += 1;
        if *kind == viewer_foundation::unit_rules::PATROL && *current_region == region {
            patrolling += 1;
        }
        match pride {
            Ok(true) => last_pride = Ok(Some(*kind)),
            Ok(false) => {}
            Err(why) => last_pride = Err(why),
        }
        let rule = native
            .task_force(*handle)
            .map(|inputs| inputs.engagement_rule == 0)
            .ok_or(ENGAGEMENT_UNKNOWN);
        let value = no_engagement.entry(*kind).or_insert(Ok(false));
        *value = match (*value, rule) {
            (Ok(true), _) | (_, Ok(true)) => Ok(true),
            (Err(why), _) | (_, Err(why)) => Err(why),
            _ => Ok(false),
        };
    }
    MISSION_SPRITES
        .iter()
        .filter_map(|&(kind, _)| {
            let total = totals.get(&kind).copied().unwrap_or(0);
            if total == 0
                && ![
                    engine_native::naval_mission_inputs::RAIDING,
                    engine_native::naval_mission_inputs::ESCORT,
                ]
                .contains(&kind)
            {
                return None;
            }
            let active = match kind {
                1 => Ok(Some(patrolling)),
                2 | 9 => Err(P2_ACTIVE),
                _ => Ok(None),
            };
            let alert = match kind {
                1 => Ok(patrolling < 1),
                2 | 3 | 9 if no_engagement.get(&kind) == Some(&Ok(true)) => Ok(true),
                2 | 3 | 9 if no_engagement.get(&kind).is_some_and(Result::is_err) => {
                    no_engagement[&kind]
                }
                2 | 9 => Err(P2_ACTIVE),
                3 => Err(P3_RAIDING),
                4 => Err(P3_ESCORT_ALERT),
                _ => Ok(false),
            };
            let pride = last_pride.map(|chosen| chosen == Some(kind));
            Some(MissionEntry {
                kind,
                total,
                active,
                engagement: no_engagement.get(&kind).copied().unwrap_or(Ok(false)),
                pride,
                alert,
                efficiency: match kind {
                    3 => Some(Err(P3_RAIDING)),
                    4 => Some(Err(P3_ESCORT)),
                    _ => None,
                },
            })
        })
        .collect()
}

fn count_text(entry: &MissionEntry) -> Result<(String, bool), &'static str> {
    match entry.active? {
        Some(active) if active != entry.total => {
            Ok((format!("{active}/{}", entry.total), active == 0))
        }
        _ => Ok((entry.total.to_string(), false)),
    }
}

fn undefined(report: &mut CounterReport, sprite: &str) {
    if !report.undefined_naval_mission_sprites.contains(sprite) {
        report
            .undefined_naval_mission_sprites
            .insert(sprite.to_owned());
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Record<'e> {
    listed: Option<&'e MissionEntry>,
    alert: Result<bool, &'static str>,
    pride: Result<bool, &'static str>,
    efficiency: Option<Result<String, &'static str>>,
}

fn efficiency_text(value: i64) -> String {
    let text = content_reader_core::fixed5::fixed5_percent(value.clamp(0, 100_000), 0, true);
    let code = COLOR;
    if value < 75_000 {
        format!("{code}R{text}{code}!")
    } else if value < 100_000 {
        format!("{code}Y{text}{code}!")
    } else {
        text
    }
}

fn record(kind: u32, entries: &[MissionEntry]) -> Record<'_> {
    match entries.iter().find(|entry| entry.kind == kind) {
        Some(entry) => Record {
            listed: (entry.total != 0).then_some(entry),
            alert: entry.alert,
            pride: entry.pride,
            efficiency: entry.efficiency.and_then(|value| match value {
                Ok(-100_000) => None,
                other => Some(other.map(efficiency_text)),
            }),
        },
        None => Record {
            listed: None,
            alert: match kind {
                1 => Ok(true),
                3 => Ok(true),
                4 => Err(P3_ESCORT_ALERT),
                _ => Ok(false),
            },
            pride: Ok(false),
            efficiency: match kind {
                3 => Some(Ok(efficiency_text(0))),
                4 => Some(Err(P3_ESCORT)),
                _ => None,
            },
        },
    }
}

fn undecided(kind: u32, sprite: &str, record: &Record<'_>) -> Vec<String> {
    let count = record.listed.and_then(|entry| count_text(entry).err());
    let efficiency = match &record.efficiency {
        Some(Err(input)) => Some(*input),
        _ => None,
    };
    [
        ("count", count),
        ("alert", record.alert.err()),
        ("efficiency", efficiency),
    ]
    .into_iter()
    .filter_map(|(child, input)| {
        Some(format!(
            "its {child} for mission kind {kind} ({sprite}): {}",
            input?
        ))
    })
    .collect()
}

fn set_shown(
    instance: &mut Instance,
    window: &native::Element,
    element: &native::Element,
    shown: bool,
) -> Result<(), String> {
    if shown {
        instance.set_hide_flag(window, element, false)?;
        instance.gui_show(window, element)
    } else {
        instance.hide(window, element)?;
        instance.set_hide_flag(window, element, true)
    }
}

pub fn mission_assets(
    windows: &NativeWindows,
    registry: &native::resources::Registry,
    localisation: &content_reader_localisation::localisation::Localisation,
) -> native_gui::Assets {
    let mut assets = native_gui::Assets::default();
    let gui_type = |name: &[u8]| windows.gui_type(name);
    if let Ok(found) = MissionWindows::find(windows) {
        assets.extend(native_gui::assets_in(found.root, &gui_type, localisation));
        assets.extend(native_gui::assets_in(found.entry, &gui_type, localisation));
    }
    assets.sprites.extend(
        MISSION_SPRITES
            .iter()
            .map(|(_, name)| name.as_bytes())
            .filter(|name| registry.sprite(name).is_some())
            .map(<[u8]>::to_vec),
    );
    assets
}

#[derive(Debug, Default)]
pub struct MissionPass {
    pub(crate) hovers: Arc<super::naval_mission_inputs::hover::Hovers>,
    pub(crate) convoy_hovers: Arc<BTreeMap<u64, super::naval_mission_inputs::hover::Prepared>>,
    pub regions: Option<BTreeMap<u32, Vec<MissionEntry>>>,
    pub undecided: Option<&'static str>,
    pub mode: i32,
    pub regional: BTreeMap<u32, naval_regional::Region>,
}

pub fn naval_missions(
    inputs: &CounterInputs<'_>,
    icons: &MissionIcons,
    out: &mut CounterFrame<impl Sized, base_windows::PreparedBases, impl Sized, naval_windows::Plan>,
) -> io::Result<()> {
    if let Some(reason) = icons.unavailable {
        out.report.naval_mission_limits.insert(reason);
    }
    for (reason, count) in &icons.input_gaps {
        *out.report
            .naval_mission_gaps
            .entry(reason.clone().into())
            .or_default() += *count;
    }
    let runs = inputs.visibility.naval_missions
        && inputs.passes(|height| mission_pass(height, &inputs.content.defines));
    out.naval.missions = MissionPass {
        hovers: Arc::clone(&icons.hovers),
        convoy_hovers: Arc::clone(&icons.convoy_hovers),
        regions: runs.then(|| icons.regions.clone()),
        undecided: icons.unavailable.filter(|_| runs),
        ..Default::default()
    };
    Ok(())
}

#[derive(Debug)]
struct PooledIcon {
    entries: Vec<Instance>,
    listed: Vec<usize>,
    attached: Vec<usize>,
    count_unknown: Vec<Option<&'static str>>,
    held_grids: Vec<Held>,
}

#[derive(Debug, Default)]
pub struct MissionView {
    built: Option<(NativeWindows, [i32; 2])>,
    pool: Option<Pool<u32>>,
    icons: HashMap<SlotId, PooledIcon>,
    unplaced: HashMap<SlotId, &'static str>,
    entries_withheld: Withheld<(SlotId, usize)>,
    diverged: Option<Cow<'static, str>>,
    census_undecided: Option<&'static str>,
    unsettled: Vec<(MissionWindow, String)>,
    kept_off_screen: bool,
    pub updated_grids: Vec<(u32, SlotId)>,
    pub mode: i32,
    pub badge_counts: BTreeMap<u32, [Result<usize, String>; 2]>,
    badge_data: BTreeMap<u32, naval_mine_badges::Packet>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MissionWindow {
    Icon(SlotId),
    Entry(SlotId, usize),
}

#[derive(Clone, Debug)]
struct Reached<'p> {
    region: u32,
    entries: Option<&'p [MissionEntry]>,
    point: Result<[i32; 2], &'static str>,
}

type Laid = Result<Option<LaidIcon>, native_gui::Refusals>;

struct LaidIcon {
    slot: SlotId,
    layout: native_gui::Layout,
    items: Vec<Vec<native_gui::Part>>,
    entries: usize,
    hits: Vec<(u32, super::naval_mission_inputs::hover::Control, [f32; 4])>,
}

fn rebuild(
    entry: &native::Element,
    viewport: [i32; 2],
    resources: &dyn native_gui::Resources,
) -> Result<PooledIcon, String> {
    use native::Kind::{Button, Icon, TextBox};
    let mut entries = Vec::with_capacity(MISSION_SPRITES.len());
    for (_, sprite) in MISSION_SPRITES {
        let mut instance = Instance::construct(entry, viewport, resources)?;
        for (kind, name) in [(Icon, "efficiency_background"), (TextBox, "efficiency")] {
            if let Some(child) = entry.find(kind, name) {
                instance.hide(entry, child)?;
                instance.set_hide_flag(entry, child, true)?;
            }
        }
        if let Some(button) = entry.find(Button, "button") {
            instance.set_sprite(
                entry,
                button,
                sprite.as_bytes(),
                native_gui::SpriteSetter::ButtonStandard,
            )?;
        }
        entries.push(instance);
    }
    Ok(PooledIcon {
        entries,
        listed: Vec::new(),
        attached: Vec::new(),
        count_unknown: vec![None; MISSION_SPRITES.len()],
        held_grids: Vec::new(),
    })
}

fn reach_entries(
    icon: &mut PooledIcon,
    windows: &MissionWindows<'_>,
    held: &[Held],
) -> Result<(), String> {
    icon.held_grids.extend(
        held.iter()
            .filter(|held| held.path != windows.list.path)
            .cloned(),
    );
    for reached in held.iter().filter(|held| held.path == windows.list.path) {
        for &index in &icon.listed {
            let window = &mut icon.entries[index];
            if reached.shown {
                window.gui_show(windows.entry, windows.entry)?;
            } else {
                window.hide(windows.entry, windows.entry)?;
            }
        }
    }
    Ok(())
}

fn refusal(
    path: &str,
    kind: native_gui::RefusalKind,
    detail: impl Into<String>,
) -> native_gui::Refusal {
    native_gui::Refusal {
        path: path.to_owned(),
        kind,
        detail: detail.into(),
    }
}

fn refused(element: &native::Element) -> impl Fn(String) -> native_gui::Refusals + '_ {
    move |why| {
        vec![refusal(
            &element.path,
            native_gui::RefusalKind::MissingInput,
            why,
        )]
    }
}

fn content_calls(
    windows: &MissionWindows<'_>,
    mode: i32,
    root: &mut Instance,
) -> Result<(), native_gui::Refusals> {
    for (name, _) in COUNTED_CONTAINERS {
        if let Some(container) = windows.content.find(native::Kind::Container, name) {
            root.hide(windows.root, container)
                .map_err(refused(container))?;
            root.set_hide_flag(windows.root, container, true)
                .map_err(refused(container))?;
        }
    }
    let _ = mode;
    Ok(())
}

fn update_icon(
    windows: &MissionWindows<'_>,
    resources: &dyn native_gui::Resources,
    root: &mut Instance,
    icon: &mut PooledIcon,
    entries: &[MissionEntry],
    viewport: [i32; 2],
) -> Result<Updated, native_gui::Refusals> {
    use native::Kind::{Button, Icon, TextBox};
    let entry = windows.entry;
    root.hide(windows.root, windows.list)
        .map_err(refused(windows.list))?;
    let held = root.take_held();
    reach_entries(icon, windows, &held).map_err(refused(entry))?;
    icon.listed.clear();
    let button = entry.find(Button, "button");
    let count = entry.find(TextBox, "count");
    let alert = entry.find(Icon, "alert");
    let pride = entry.find(Icon, "pride");
    let background = entry.find(Icon, "efficiency_background");
    let efficiency = entry.find(TextBox, "efficiency");
    if let Some(button) = button {
        for window in &mut icon.entries {
            window
                .set_frame(entry, button, 1)
                .map_err(refused(button))?;
        }
    }
    let mut written = Vec::with_capacity(MISSION_SPRITES.len());
    for (index, &(kind, _)) in MISSION_SPRITES.iter().enumerate() {
        let record = record(kind, entries);
        let window = &mut icon.entries[index];
        let mut update = viewer_counter_kit::gui_windows::Update::default();
        if let Some(listed) = record.listed {
            if let Some(button) = button {
                window
                    .set_frame(entry, button, 2)
                    .map_err(refused(button))?;
            }
            if let Some(count) = count {
                let text = match count_text(listed) {
                    Ok((text, true)) => {
                        let code = COLOR;
                        format!("{code}R{text}{code}!")
                    }
                    Ok((text, false)) => text,
                    Err(_) => String::new(),
                };
                window
                    .set_text(entry, count, &text, resources)
                    .map_err(refused(count))?;
            }
            icon.count_unknown[index] = listed.active.err();
            update.wrote.push(COUNT);
            icon.listed.push(index);
        } else if let Some(why) = icon.count_unknown[index] {
            update
                .unknown
                .push((COUNT, format!("its last listed count is unknown: {why}")));
        }
        if let Some(alert) = alert {
            set_shown(window, entry, alert, record.alert == Ok(true)).map_err(refused(alert))?;
        }
        match &record.efficiency {
            Some(Ok(text)) => {
                if let Some(efficiency) = efficiency {
                    set_shown(window, entry, efficiency, true).map_err(refused(efficiency))?;
                    window
                        .set_text(entry, efficiency, text, resources)
                        .map_err(refused(efficiency))?;
                }
                if let Some(background) = background {
                    set_shown(window, entry, background, true).map_err(refused(background))?;
                }
            }
            _ => {
                for element in [efficiency, background].into_iter().flatten() {
                    set_shown(window, entry, element, false).map_err(refused(element))?;
                }
            }
        }
        match record.pride {
            Ok(shown) => {
                if let Some(pride) = pride {
                    set_shown(window, entry, pride, shown).map_err(refused(pride))?;
                }
                update.wrote.push(PRIDE);
            }
            Err(why) => update
                .unknown
                .push((PRIDE, format!("its pride marker is undecided: {why}"))),
        }
        written.push(update);
    }
    content_calls(windows, DRAWN_UI_MODE, root)?;
    root.show(windows.root, windows.list)
        .map_err(refused(windows.list))?;
    let held = root.take_held();
    reach_entries(icon, windows, &held).map_err(refused(entry))?;
    let entry_size = native_gui::container_frame(entry, Some(viewport))
        .map_err(|why| vec![refusal(&entry.path, native_gui::RefusalKind::NoParent, why)])?
        .size;
    let shown = icon.listed.len();
    let half = if shown > 0 { entry_size[1] / 2 } else { 0 };
    root.set_position(windows.root, windows.content, [0, -half])
        .map_err(refused(windows.content))?;
    let updated = |placement| Updated {
        placement,
        entries: written,
        list_placed: shown > 0,
    };
    if shown == 0 {
        return Ok(updated(Placement::Placed(Vec::new())));
    }
    let kept = windows
        .list
        .list()
        .expect("found as an overlapping list")
        .position;
    let shift = (shown as i32).wrapping_mul(entry_size[0]) / 2;
    root.control(windows.root, windows.list)
        .map_err(refused(windows.list))?
        .position = Some([kept[0].wrapping_sub(shift), kept[1]]);
    let layout = match root.layout(windows.root, resources, None) {
        Ok(Some(layout)) => layout,
        Ok(None) => return Ok(updated(Placement::Placed(Vec::new()))),
        Err(refusals) => return Ok(updated(unplaced(icon, windows, &refusals))),
    };
    let Some(holder) = layout
        .holders
        .iter()
        .find(|holder| holder.path == windows.list.path)
    else {
        return Ok(updated(Placement::Placed(Vec::new())));
    };
    let places = match native_gui::list_places(
        windows.list,
        holder.content,
        entry_size,
        shown,
        holder.position,
    ) {
        Ok(places) => places,
        Err(why) => {
            let refused = refusal(&windows.list.path, native_gui::RefusalKind::Reader, why);
            return Ok(updated(unplaced(icon, windows, &[refused])));
        }
    };
    let mut placed = Vec::with_capacity(places.len());
    for (index, place) in places {
        let record = icon.listed[index];
        icon.entries[record]
            .set_position(entry, entry, place)
            .map_err(refused(entry))?;
        icon.attached.retain(|&attached| attached != record);
        icon.attached.push(record);
        placed.push(record);
    }
    Ok(updated(Placement::Placed(placed)))
}

struct Updated {
    placement: Placement,
    entries: Vec<viewer_counter_kit::gui_windows::Update>,
    list_placed: bool,
}

#[derive(Debug)]
enum Placement {
    Placed(Vec<usize>),
    Unknown(Vec<usize>, String),
}

fn unplaced(
    icon: &mut PooledIcon,
    windows: &MissionWindows<'_>,
    refusals: &[native_gui::Refusal],
) -> Placement {
    for &record in &icon.listed {
        icon.attached.retain(|&attached| attached != record);
        icon.attached.push(record);
    }
    let why = refusals
        .iter()
        .map(|refusal| refusal.to_string())
        .collect::<Vec<_>>()
        .join("; ");
    Placement::Unknown(
        icon.listed.clone(),
        format!(
            "{MISSION_ENTRY_WINDOW} is not drawn: its place in {} is unknown: {why}",
            windows.list.path
        ),
    )
}

impl MissionView {
    pub(crate) fn retry(&self) -> Option<Retry> {
        self.pool.as_ref().and_then(Pool::retry)
    }

    fn build(
        &mut self,
        native: &NativeWindows,
        windows: &MissionWindows<'_>,
        viewport: [i32; 2],
        resources: &dyn native_gui::Resources,
    ) -> Result<(), String> {
        if self
            .built
            .as_ref()
            .is_some_and(|(built, at)| built.same(native) && *at == viewport)
        {
            return Ok(());
        }
        *self = Self::default();
        self.pool = Some(Pool::new(
            windows.root,
            viewport,
            MapIconPool::Mission,
            Box::new(|_, _| Ok(())),
            resources,
        )?);
        self.built = Some((native.clone(), viewport));
        Ok(())
    }

    fn frame(
        &mut self,
        windows: &MissionWindows<'_>,
        resources: &dyn native_gui::Resources,
        reached: &[Reached<'_>],
        viewport: [i32; 2],
        report: &mut CounterReport,
    ) -> Result<(), String> {
        let Some(mut pool) = self.pool.take() else {
            return Ok(());
        };
        let result = self.frame_on(&mut pool, windows, resources, reached, viewport, report);
        if let Err(why) = &result {
            let slots: Vec<SlotId> = pool.registrations().map(|(_, slot)| slot).collect();
            for slot in slots {
                pool.refuse(
                    slot,
                    Unknown::UntilRebuilt,
                    format!("{MISSION_WINDOW} is not drawn: its frame does not update: {why}")
                        .into(),
                );
            }
        }
        self.pool = Some(pool);
        result
    }

    fn allocated(&mut self, pool: &mut Pool<u32>, slot: SlotId, diverged: &str) {
        let why = format!("its pool slot may not be the game's: {diverged}");
        pool.refuse(
            slot,
            Unknown::Part(LIST_PLACE),
            format!("{MISSION_WINDOW} is not drawn: {why}").into(),
        );
        self.unsettled.push((
            MissionWindow::Icon(slot),
            format!("{MISSION_WINDOW} is not drawn: {why}"),
        ));
        let Some(icon) = self.icons.get(&slot) else {
            return;
        };
        let why = format!("{MISSION_ENTRY_WINDOW} is not drawn: {why}");
        for record in 0..icon.entries.len() {
            let left = std::iter::once(Unknown::Update)
                .chain(ENTRY_KEPT.map(Unknown::Part))
                .map(|unknown| Mark::Refuse(unknown, why.clone().into()))
                .collect();
            mark(&mut self.entries_withheld, &(slot, record), left);
            self.unsettled
                .push((MissionWindow::Entry(slot, record), why.clone()));
        }
    }

    pub fn take_grid_held(&mut self) -> Vec<(SlotId, Vec<Held>)> {
        self.icons
            .iter_mut()
            .filter_map(|(&slot, icon)| {
                (!icon.held_grids.is_empty()).then(|| (slot, std::mem::take(&mut icon.held_grids)))
            })
            .collect()
    }

    pub fn take_unsettled(&mut self) -> Vec<(MissionWindow, String)> {
        std::mem::take(&mut self.unsettled)
    }

    pub fn window(&self, window: MissionWindow) -> Option<&Instance> {
        match window {
            MissionWindow::Icon(slot) => self.pool.as_ref()?.instance(slot),
            MissionWindow::Entry(slot, record) => self.icons.get(&slot)?.entries.get(record),
        }
    }

    pub fn refuse(
        &mut self,
        window: MissionWindow,
        unknown: Unknown,
        why: Cow<'static, str>,
    ) {
        match window {
            MissionWindow::Icon(slot) => {
                if let Some(pool) = self.pool.as_mut() {
                    pool.refuse(slot, unknown, why);
                }
            }
            MissionWindow::Entry(slot, record) => {
                self.entries_withheld.refuse((slot, record), unknown, why)
            }
        }
    }

    pub fn known(&mut self, window: MissionWindow, part: &'static str) {
        match window {
            MissionWindow::Icon(slot) => {
                if let Some(pool) = self.pool.as_mut() {
                    pool.known(slot, part);
                }
            }
            MissionWindow::Entry(slot, record) => {
                self.entries_withheld.known(&(slot, record), part)
            }
        }
    }

    fn hold_undecided(&mut self, why: &'static str) {
        self.census_undecided = Some(why);
        self.updated_grids.clear();
        self.kept_off_screen = false;
        self.diverged = Some(format!("the pass runs but cannot be planned: {why}").into());
    }

    fn frame_on(
        &mut self,
        pool: &mut Pool<u32>,
        windows: &MissionWindows<'_>,
        resources: &dyn native_gui::Resources,
        reached: &[Reached<'_>],
        viewport: [i32; 2],
        report: &mut CounterReport,
    ) -> Result<(), String> {
        let root = windows.root;
        self.census_undecided = None;
        self.updated_grids.clear();
        let mut erased: Vec<(SlotId, Vec<Held>)> = Vec::new();
        for owner in reached {
            if owner.entries.is_some() {
                let new = pool.slot_of(&owner.region).is_none();
                match pool.request(owner.region) {
                    Some(slot) => {
                        if let std::collections::hash_map::Entry::Vacant(vacant) =
                            self.icons.entry(slot)
                        {
                            vacant.insert(rebuild(windows.entry, viewport, resources)?);
                        }
                        if let (true, Some(diverged)) = (new, self.diverged.clone()) {
                            self.allocated(pool, slot, &diverged);
                        }
                    }
                    None => {
                        *report
                            .naval_mission_gaps
                            .entry(POOL_EXHAUSTED.into())
                            .or_default() += 1;
                        self.diverged = Some(naval_windows::GROWTH_FRAME.into());
                    }
                }
            } else if let Some(slot) = pool.slot_of(&owner.region) {
                erased.push((slot, pool.erase(root, &owner.region)?));
            }
        }
        let swept: Vec<u32> = pool
            .live()
            .map(|(region, ..)| *region)
            .filter(|region| !reached.iter().any(|owner| owner.region == *region))
            .collect();
        for region in swept {
            if let Some(slot) = pool.slot_of(&region) {
                erased.push((slot, pool.erase(root, &region)?));
            }
        }
        let pass = pool.update(root, |region, instance| {
            match reached.iter().find(|owner| owner.region == *region) {
                Some(owner) if owner.entries.is_some() => match owner.point {
                    Ok(point) => Admission::Inside(point),
                    Err(_) => Admission::Inside(instance.position()),
                },
                _ => Admission::Empty,
            }
        })?;
        erased.extend(pass.erased.into_iter().map(|gone| (gone.slot, gone.held)));
        for (slot, held) in erased {
            if let Some(icon) = self.icons.get_mut(&slot) {
                reach_entries(icon, windows, &held)?;
            }
        }
        for (region, slot) in pass.updated {
            let Some(owner) = reached.iter().find(|owner| owner.region == region) else {
                continue;
            };
            match owner.point {
                Ok(_) => {
                    self.unplaced.remove(&slot);
                }
                Err(why) => {
                    self.unplaced.insert(slot, why);
                }
            }
            let entries = owner.entries.unwrap_or_default();
            let (Some(instance), Some(icon)) = (pool.get(slot), self.icons.get_mut(&slot)) else {
                continue;
            };
            let held = instance.take_held();
            reach_entries(icon, windows, &held)?;
            match update_icon(windows, resources, instance, icon, entries, viewport) {
                Ok(updated) => {
                    self.updated_grids.push((region, slot));
                    if self.mode == NAVAL_UI_MODE
                        && let Some(counts) = self.badge_counts.get(&region)
                    {
                        for (name, count) in
                            ["ongoing_combat_container", "combat_results_container"]
                                .into_iter()
                                .zip(counts)
                        {
                            if let Some(container) =
                                windows.content.find(native::Kind::Container, name)
                            {
                                match count {
                                    Ok(count) if *count > 0 => {
                                        set_shown(instance, windows.root, container, true)?;
                                        if let Some(text) =
                                            container.find(native::Kind::TextBox, "count")
                                        {
                                            instance.set_text(
                                                windows.root,
                                                text,
                                                &count.to_string(),
                                                resources,
                                            )?;
                                        }
                                    }
                                    Err(why) => {
                                        *report
                                            .naval_mission_gaps
                                            .entry(format!("{name}: {why}").into())
                                            .or_default() += 1;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    if self.mode == NAVAL_UI_MODE
                        && let Some(packet) = self.badge_data.get(&region)
                    {
                        if let Err(why) = naval_mine_badges::apply(
                            instance,
                            windows.root,
                            resources,
                            packet,
                            report,
                        ) {
                            *report
                                .naval_mission_gaps
                                .entry(format!("regional status: {why}").into())
                                .or_default() += 1;
                        }
                        let empty = [Ok(0), Ok(0)];
                        let counts = self.badge_counts.get(&region).unwrap_or(&empty);
                        if let Err(why) = naval_mine_badges::relayout(
                            instance,
                            windows.root,
                            windows.entry,
                            packet,
                            counts,
                            icon.listed.len(),
                            viewport,
                        ) {
                            *report
                                .naval_mission_gaps
                                .entry(format!("regional status layout: {why}").into())
                                .or_default() += 1;
                        }
                    }
                    pool.rewrote(slot, Rewrote::Update);
                    if updated.list_placed {
                        pool.rewrote(slot, Rewrote::Part(LIST_PLACE));
                    }
                    let prefix = format!("{MISSION_ENTRY_WINDOW} is not drawn: ");
                    for (record, update) in updated.entries.iter().enumerate() {
                        let left = marks(update, Ok(()), &prefix);
                        mark(&mut self.entries_withheld, &(slot, record), left);
                    }
                    match updated.placement {
                        Placement::Placed(records) => {
                            for record in records {
                                self.entries_withheld
                                    .rewrote(&(slot, record), Rewrote::Part(ENTRY_PLACE));
                            }
                        }
                        Placement::Unknown(records, why) => {
                            for record in records {
                                self.entries_withheld.refuse(
                                    (slot, record),
                                    Unknown::Part(ENTRY_PLACE),
                                    why.clone().into(),
                                );
                            }
                        }
                    }
                }
                Err(refusals) => {
                    let why = refusals
                        .iter()
                        .map(|refusal| refusal.to_string())
                        .collect::<Vec<_>>()
                        .join("; ");
                    pool.refuse(
                        slot,
                        Unknown::UntilRebuilt,
                        format!("{MISSION_WINDOW} is not drawn: its update does not apply: {why}")
                            .into(),
                    );
                }
            }
        }
        Ok(())
    }

    pub fn tick_clocks(
        &mut self,
        windows: &MissionWindows<'_>,
        resources: &dyn native_gui::Resources,
        tick: Tick,
    ) -> Result<(), String> {
        let Some(pool) = self.pool.as_mut() else {
            return Ok(());
        };
        let mut ticked = pool.tick_clocks(windows.root, resources, tick);
        for icon in self.icons.values_mut() {
            for entry in &mut icon.entries {
                ticked = ticked.and(entry.tick_clocks(windows.entry, resources, tick));
            }
        }
        ticked
    }

    pub fn registrations(
        &self,
    ) -> impl Iterator<Item = (native_gui::lifecycle::Registration, MissionWindow)> + '_ {
        let roots = self.pool.iter().flat_map(|pool| {
            pool.registrations()
                .map(|(at, slot)| (at, MissionWindow::Icon(slot)))
        });
        let entries = self.icons.iter().flat_map(|(slot, icon)| {
            icon.entries
                .iter()
                .enumerate()
                .map(|(record, entry)| (entry.registration(), MissionWindow::Entry(*slot, record)))
        });
        roots.chain(entries)
    }

    pub fn tick_transitions_of(
        &mut self,
        windows: &MissionWindows<'_>,
        window: MissionWindow,
    ) -> Result<(), String> {
        match window {
            MissionWindow::Icon(slot) => {
                let Some(pool) = self.pool.as_mut() else {
                    return Ok(());
                };
                let ticked = pool.tick_transitions_of(windows.root, slot);
                let held = pool.take_held(slot);
                let reached = match self.icons.get_mut(&slot) {
                    Some(icon) => reach_entries(icon, windows, &held),
                    None => Ok(()),
                };
                ticked.and(reached)
            }
            MissionWindow::Entry(slot, record) => match self
                .icons
                .get_mut(&slot)
                .and_then(|icon| icon.entries.get_mut(record))
            {
                Some(entry) => entry.tick_transitions(windows.entry),
                None => Ok(()),
            },
        }
    }

    pub fn tick_draws(
        &mut self,
        windows: &MissionWindows<'_>,
        resources: &dyn native_gui::Resources,
        report: &mut CounterReport,
    ) -> Result<(), String> {
        let Some(pool) = self.pool.as_mut() else {
            return Ok(());
        };
        let mut ticked = pool.tick_draws(windows.root, resources);
        count_noted(
            &mut report.naval_mission_gaps,
            MISSION_WINDOW,
            &pool.take_diagnostics(),
        );
        for icon in self.icons.values_mut() {
            for entry in &mut icon.entries {
                ticked = ticked.and(entry.tick_draws(windows.entry, resources));
            }
        }
        ticked
    }

    pub fn tick_due(&self) -> bool {
        !self.unsettled.is_empty()
            || self.pool.as_ref().is_some_and(Pool::tick_due)
            || self
                .icons
                .values()
                .any(|icon| icon.entries.iter().any(|entry| !entry.idle()))
    }

    pub fn active(&self) -> bool {
        self.kept_off_screen
            || self.pool.as_ref().is_some_and(Pool::active)
            || self
                .icons
                .values()
                .any(|icon| icon.entries.iter().any(Instance::active))
    }

    fn laid(
        &self,
        windows: &MissionWindows<'_>,
        resources: &dyn native_gui::Resources,
        entries_of: &dyn Fn(u32) -> Vec<MissionEntry>,
        defined: &dyn Fn(&str) -> bool,
        report: &mut CounterReport,
    ) -> Vec<([i32; 2], u32, Laid)> {
        let Some(pool) = &self.pool else {
            return Vec::new();
        };
        if let Some(why) = self.census_undecided {
            let count = pool.visible().count();
            if count != 0 {
                *report
                    .naval_mission_gaps
                    .entry(format!("{MISSION_WINDOW} is not drawn: {why}").into())
                    .or_default() += count;
            }
            return Vec::new();
        }
        let mut out = Vec::new();
        for icon in pool.visible() {
            let (region, slot, instance) = (*icon.owner, icon.slot, icon.instance);
            if let Some(why) = icon.withheld {
                *report
                    .naval_mission_gaps
                    .entry(why.to_owned().into())
                    .or_default() += 1;
                continue;
            }
            if let Some(why) = self.unplaced.get(&slot) {
                *report
                    .naval_mission_gaps
                    .entry(format!("{MISSION_WINDOW} is not drawn: {why}").into())
                    .or_default() += 1;
                continue;
            }
            let laid = instance
                .layout_with_regions(windows.root, resources, None, true)
                .map(|layout| {
                    layout.map(|layout| {
                        let mut items = vec![Vec::new(); layout.holders.len()];
                        let mut drawn = 0;
                        let mut hits = Vec::new();
                        let holder = layout
                            .holders
                            .iter()
                            .position(|holder| holder.path == windows.list.path);
                        if let (Some(at), Some(icon)) = (holder, self.icons.get(&slot)) {
                            let entries = entries_of(region);
                            for &index in &icon.attached {
                                let window = &icon.entries[index];
                                if !window.drawn() {
                                    continue;
                                }
                                if let Some(why) = self.entries_withheld.why(&(slot, index)) {
                                    *report
                                        .naval_mission_gaps
                                        .entry(why.to_owned().into())
                                        .or_default() += 1;
                                    continue;
                                }
                                match window.layout(windows.entry, resources, None) {
                                    Ok(Some(mut entry)) => {
                                        count_noted(
                                            &mut report.naval_mission_gaps,
                                            MISSION_ENTRY_WINDOW,
                                            &entry.diagnostics,
                                        );
                                        items[at].extend(
                                            layout.holders[at].place(window.position(), &entry),
                                        );
                                        drawn += 1;
                                        let (kind, sprite) = MISSION_SPRITES[index];
                                        entry.parts = layout.holders[at].place(window.position(), &entry);
                                        entry.regions = layout.holders[at].place_regions(window.position(), &entry.regions);
                                        use crate::naval_mission_inputs::hover::Control;
                                        for (element_kind, name, control) in [
                                            (native::Kind::Button,"button",Control::Button),
                                            (native::Kind::TextBox,"count",Control::Status),
                                            (native::Kind::Icon,"alert",Control::Status),
                                            (native::Kind::TextBox,"efficiency",Control::Status),
                                            (native::Kind::Icon,"pride",Control::Pride),
                                        ] {
                                            let Some(button)=windows.entry.find(element_kind,name) else {continue;};
                                            if let Some(hit) = viewer_native_gui::gui_resources::hit_rectangle(
                                                button, &entry, instance.position(), resources, &mut report.icon_gui_notes,
                                            )
                                                && !hit.disabled { hits.push((kind, control, hit.rect)); }
                                        }
                                        for child in
                                            undecided(kind, sprite, &record(kind, &entries))
                                        {
                                            *report
                                                .undecided_naval_mission_children
                                                .entry(child)
                                                .or_default() += 1;
                                        }
                                        if !defined(sprite) {
                                            undefined(report, sprite);
                                            report.authored_naval_mission_buttons += 1;
                                        }
                                    }
                                    Ok(None) => {}
                                    Err(refusals) => count_refused(
                                        &mut report.naval_mission_gaps,
                                        MISSION_ENTRY_WINDOW,
                                        &refusals,
                                    ),
                                }
                            }
                        }
                        LaidIcon {
                            slot,
                            layout,
                            items,
                            entries: drawn,
                            hits,
                        }
                    })
                });
            out.push((instance.position(), region, laid));
        }
        out
    }
}

pub fn update<'n>(
    inputs: &CounterInputs<'n>,
    pass: &MissionPass,
    view: &mut MissionView,
    resources: &GuiAssets<'_>,
    schedule: &Schedule,
    report: &mut CounterReport,
) -> Option<MissionWindows<'n>> {
    let native = inputs.windows;
    let windows = match MissionWindows::find(native) {
        Ok(windows) => windows,
        Err(refusal) => {
            let entries: usize = pass
                .regions
                .iter()
                .flat_map(BTreeMap::values)
                .map(Vec::len)
                .sum();
            if entries != 0 {
                count_refused(
                    &mut report.naval_mission_gaps,
                    MISSION_WINDOW,
                    std::slice::from_ref(&refusal),
                );
            }
            return None;
        }
    };
    let viewport = gui_resources_viewport(inputs.viewport, inputs.ui_scale);
    if let Err(why) = view.build(native, &windows, viewport, resources) {
        *report
            .naval_mission_gaps
            .entry(format!("{MISSION_WINDOW} is not built: {why}").into())
            .or_default() += 1;
        return None;
    }
    if let Some(why) = pass.undecided {
        view.hold_undecided(why);
        return Some(windows);
    }
    let mut candidates: BTreeSet<u32> = pass
        .regions
        .iter()
        .flat_map(BTreeMap::keys)
        .copied()
        .collect();
    if let Some(pool) = &view.pool {
        candidates.extend(pool.live().map(|(region, ..)| *region));
    }
    let reach = reach(
        candidates,
        |region| inputs.content.region_provinces(region),
        schedule,
    );
    let mut reached = Vec::with_capacity(reach.len());
    view.kept_off_screen = false;
    for (region, in_view) in reach {
        let entries = pass
            .regions
            .as_ref()
            .and_then(|regions| regions.get(&region))
            .map(Vec::as_slice);
        let point = match inputs.content.region_icon(region) {
            Some(anchor) => inputs
                .screen(anchor)
                .map(|point| point.map(|value| value.round() as i32))
                .ok_or(UNPROJECTED_PLACE),
            None => {
                if entries.is_some() {
                    report.unanchored_naval_missions += 1;
                }
                Err(UNANCHORED_PLACE)
            }
        };
        view.kept_off_screen |= !in_view && entries.is_some();
        reached.push(Reached {
            region,
            entries,
            point,
        });
    }
    view.mode = pass.mode;
    view.badge_counts = pass
        .regional
        .iter()
        .map(|(&region, data)| {
            (
                region,
                [
                    data.combats.as_ref().map(Vec::len).map_err(Clone::clone),
                    data.results.as_ref().map(Vec::len).map_err(Clone::clone),
                ],
            )
        })
        .collect();
    view.badge_data = pass
        .regional
        .iter()
        .map(|(&region, data)| {
            let mut packet = data.badges.clone();
            for fill in &data.grids {
                let at = if fill.grid.ends_with("enemy_convoy_grid") {
                    2
                } else if fill.grid.ends_with("enemy_unit_grid") {
                    1
                } else {
                    0
                };
                packet.grid_counts[at] = fill.items.len();
            }
            (region, packet)
        })
        .collect();
    if let Err(why) = view.frame(&windows, resources, &reached, viewport, report) {
        *report
            .naval_mission_gaps
            .entry(format!("{MISSION_WINDOW} does not update: {why}").into())
            .or_default() += 1;
    }
    Some(windows)
}

fn reach<'c>(
    candidates: BTreeSet<u32>,
    provinces: impl Fn(u32) -> &'c [u32],
    schedule: &Schedule,
) -> Vec<(u32, bool)> {
    let mut reached: Vec<(u32, u32, bool)> = candidates
        .into_iter()
        .filter_map(|region| {
            let listed = provinces(region)
                .iter()
                .copied()
                .filter(|&province| schedule.listed(province));
            let first = listed.clone().min()?;
            let in_view = listed
                .into_iter()
                .any(|province| schedule.update(province) == Some(Update::InView));
            Some((first, region, in_view))
        })
        .collect();
    reached.sort_by_key(|(first, ..)| *first);
    reached
        .into_iter()
        .map(|(_, region, in_view)| (region, in_view))
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    inputs: &CounterInputs<'_>,
    pass: &MissionPass,
    view: &MissionView,
    state: &naval_windows::Windows,
    windows: &MissionWindows<'_>,
    resources: &GuiAssets<'_>,
    batch: &mut Batch,
    report: &mut CounterReport,
) -> io::Result<()> {
    let registry = inputs.assets.gui.registry();
    let defined = |sprite: &str| registry.sprite(sprite.as_bytes()).is_some();
    let entries_of = |region: u32| {
        pass.regions
            .as_ref()
            .and_then(|regions| regions.get(&region))
            .cloned()
            .unwrap_or_default()
    };
    let laid = view.laid(windows, resources, &entries_of, &defined, report);
    let priority = inputs.priority(PriorityIcon::NavalMission);
    for (ordinal, (origin, region, laid)) in laid.into_iter().enumerate() {
        batch.at_anchor(inputs.content.region_icon(region).and_then(|anchor| {
            inputs.icon_anchor(
                IconFamily::Missions,
                anchor,
                inputs.content.defines.naval_mission_cutoff,
            )
        }));
        let mut icon = match laid {
            Ok(Some(icon)) => icon,
            Ok(None) => continue,
            Err(refusals) => {
                count_refused(&mut report.naval_mission_gaps, MISSION_WINDOW, &refusals);
                continue;
            }
        };
        count_noted(
            &mut report.naval_mission_gaps,
            MISSION_WINDOW,
            &icon.layout.diagnostics,
        );
        let grid_marks = (
            report.unit_hits.len(),
            report.task_force_port_hovers.len(),
            report.naval_region_hovers.len(),
            report.naval_counters,
        );
        naval_windows::compose_mission_grids(
            state,
            region,
            icon.slot,
            windows.root,
            &icon.layout,
            &mut icon.items,
            origin,
            resources,
            (priority, ordinal as u32),
            report,
        );
        let drawn = viewer_counter_kit::gui_windows::draw_window(
            &icon.layout,
            &icon.items,
            origin,
            resources,
            batch,
            (priority, ordinal as u32),
        )?;
        match drawn {
            Ok(()) => {
                for hit in &mut report.naval_region_hovers[grid_marks.2..] {
                    if let NavalRegionalControl::Convoy(handle) = hit.control {
                        hit.text = pass.convoy_hovers.get(&handle).map(|hover| {
                            Arc::new(hover.render(
                                inputs.localisation,
                                inputs.recorded_language,
                                inputs.content.recorded_labels.as_deref(),
                            ))
                        });
                    }
                }
                naval_regional::control_hovers(
                    region,
                    pass.regional.get(&region),
                    windows.root,
                    &icon.layout,
                    origin,
                    resources,
                    layer(priority, ordinal as u32, 0),
                    report,
                );
                report.naval_mission_entries += icon.entries;
                let key = layer(priority, ordinal as u32, 0);
                report.naval_mission_hits.extend(icon.hits.into_iter().map(
                    |(mission, control, rect)| NavalMissionHit {
                        region,
                        mission: if control == super::naval_mission_inputs::hover::Control::Button {
                            mission
                        } else {
                            0
                        },
                        rect,
                        layer: key,
                        text: pass.hovers.get(&(region, mission, control)).map(|hover| {
                            Arc::new(hover.render(
                                inputs.localisation,
                                inputs.recorded_language,
                                inputs.content.recorded_labels.as_deref(),
                            ))
                        }),
                    },
                ));
            }
            Err(refusal) => {
                report.unit_hits.truncate(grid_marks.0);
                report.task_force_port_hovers.truncate(grid_marks.1);
                report.naval_region_hovers.truncate(grid_marks.2);
                report
                    .regional_naval_unit_hits
                    .retain(|&index| index < grid_marks.0);
                report.naval_counters = grid_marks.3;
                count_refused(
                    &mut report.naval_mission_gaps,
                    MISSION_WINDOW,
                    std::slice::from_ref(&refusal),
                );
            }
        }
    }
    batch.at_anchor(None);
    Ok(())
}

fn mission_pass(height: f32, defines: &CounterDefines) -> bool {
    visible(height, defines.naval_mission_cutoff)
}

#[cfg(test)]
mod tests;
