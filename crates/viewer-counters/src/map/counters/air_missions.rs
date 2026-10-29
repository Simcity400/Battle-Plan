//! Regional Air mission entries. Registration determines membership; the selected
//! mission mask counts the wing's aircraft once in each selected mission.
use super::*;
#[cfg(test)]
use replay_data::rows::AirRegionalMissionRow;
use replay_data::rows::AirWingRow;
#[cfg(test)]
use replay_derive::air_mission_counts::count;
use replay_derive::air_mission_counts::{MISSIONS, RegionalWings, regional_counts};
use std::collections::HashMap;
use viewer_foundation::map::{
    gui_resources::{count_noted, count_refused, hit_rectangle},
    icon_windows::{Owner, Plan, Retry},
    native_gui::{
        self,
        lifecycle::{Instance, MapIconPool, SlotId},
    },
    native_resources::Context,
    native_windows::{AIR_MISSION_ENTRY_WINDOW as ENTRY, AIR_MISSION_WINDOW as WINDOW},
};

const AIR_SUPPLY: usize = 10;

/// The native supply pass rebuilds its ordered consumer membership every hour.
/// Recording that membership requires either an additional hourly full walk or
/// synchronous per-consumer event acquisition, both excluded by the cost rules.
/// Rounded unit footers cannot prove raw demand, severity or membership.
const SUPPLY_REFUSAL: &str = "Air-supply regional summary operands are unavailable: exact consumer membership is rebuilt hourly; an extra full scan or synchronous per-consumer observation violates the recording cost rules";

/// Supply status updates are reached for every drawn regional icon, even when
/// no registered Air-supply wing admits its mission entry. Keep those refusals
/// separate from the known registration count and interception warning.
fn note_supply_refusals(count: i32, notes: &mut BTreeMap<String, usize>) {
    let mut refuse = |fragment: &str| {
        *notes
            .entry(format!("{WINDOW} {fragment} withheld: {SUPPLY_REFUSAL}"))
            .or_default() += 1;
    };
    if count < 0 {
        refuse("Air-supply mission entry admission");
    } else if count == 0 {
        refuse("Air-supply zero-aircraft warning colour");
    }
    refuse("Air-supply severity icon");
    refuse("Air-supply amount text");
    refuse("Air-supply regional hover");
}

#[derive(Default)]
pub(super) struct Prepared {
    regions: BTreeMap<u32, [i32; MISSIONS]>,
    alerts: BTreeSet<(u32, usize)>,
    uncertain: BTreeMap<(u32, usize), &'static str>,
    refused: BTreeMap<String, usize>,
    hovers: BTreeMap<(u32, usize), Arc<viewer_foundation::tooltip_text::Tooltip>>,
    ground: BTreeMap<u32, Ground>,
    ground_hovers: BTreeMap<u32, Arc<viewer_foundation::tooltip_text::Tooltip>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ground {
    frame: i32,
    enabled: bool,
}

fn ground(active: u32, power: Option<i64>, cost: i64) -> Result<Ground, String> {
    match active {
        1 => Ok(Ground {
            frame: 3,
            enabled: true,
        }),
        0 => power
            .map(|power| {
                if power < cost {
                    Ground {
                        frame: 2,
                        enabled: false,
                    }
                } else {
                    Ground {
                        frame: 1,
                        enabled: true,
                    }
                }
            })
            .ok_or("country command power is unavailable".into()),
        _ => Err("Ground Crews flag is outside its native boolean domain".into()),
    }
}

fn interception_alert(
    entries: impl IntoIterator<Item = (Option<bool>, Result<bool, &'static str>)>,
) -> Result<bool, &'static str> {
    let mut unknown = None;
    for (gate, damage) in entries {
        if gate == Some(false) || damage == Ok(false) {
            continue;
        }
        match (gate, damage) {
            (Some(true), Ok(true)) => return Ok(true),
            (_, Err(reason)) => {
                unknown.get_or_insert(reason);
            }
            (None, Ok(true)) => {
                unknown.get_or_insert(
                    "the registered mission's trace-count interception gate is unavailable",
                );
            }
            _ => {}
        }
    }
    unknown.map_or(Ok(false), Err)
}

pub(super) struct HourInputs<'a> {
    pub(super) content: &'a CounterContent,
    pub(super) assets: &'a CounterAssets,
    pub(super) localisation: &'a installed_content_localisation::localisation::Localisation,
}

impl Prepared {
    pub(super) fn read(
        inputs: &HourInputs<'_>,
        frame: &DecodedFrame,
        vision: &Vision,
    ) -> io::Result<Self> {
        let result = (|| {
            let links = frame
                .complete::<tables::CountryLinks>()
                .map_err(|e| e.to_string())?;
            let wings = frame
                .complete::<tables::AirWings>()
                .map_err(|e| e.to_string())?;
            let registrations = frame
                .complete::<tables::AirRegionalMissions>()
                .map_err(|e| e.to_string())?;
            let links: HashMap<_, _> = links
                .iter()
                .map(|link| (link.id, link.canonical_slot))
                .collect();
            regional_counts(registrations, wings, vision.ours(), |raw| {
                links.get(&raw).copied()
            })
        })();
        Ok(match result {
            Ok((regions, regional_wings)) => {
                let mut prepared = Self {
                    regions,
                    ..Default::default()
                };
                let combat = frame.complete::<tables::AirRegionCombat>();
                let links: HashMap<_, _> = frame
                    .complete::<tables::CountryLinks>()
                    .map_err(|e| invalid(e.to_string()))?
                    .iter()
                    .map(|link| (link.id, link.canonical_slot))
                    .collect();
                // Match the regional native rosters: neither countries nor
                // unrelated wings/combat holders are searched for each entry.
                let holders: HashMap<_, _> = combat
                    .as_ref()
                    .ok()
                    .into_iter()
                    .flat_map(|rows| rows.iter())
                    .filter(|row| links.get(&row.country) == Some(&vision.ours()))
                    .fold(HashMap::new(), |mut holders, row| {
                        holders.entry(row.region).or_insert(row);
                        holders
                    });
                let gates: HashMap<_, _> = frame
                    .complete::<tables::AirMissionInputs>()
                    .ok()
                    .into_iter()
                    .flatten()
                    .map(|row| {
                        (
                            row.wing,
                            match row.interception_alert_gate {
                                replay_data::rows::Recorded::Value(Some(0)) => Some(false),
                                replay_data::rows::Recorded::Value(Some(1)) => Some(true),
                                _ => None,
                            },
                        )
                    })
                    .collect();
                for (&region, counts) in &prepared.regions {
                    let holder = holders.get(&region).copied();
                    for (kind, &count) in
                        counts.iter().enumerate().filter(|(_, count)| **count >= 0)
                    {
                        let mut damage = [None; 2];
                        let alert = interception_alert(regional_wings[&region].iter().filter(|wing| {
                            wing.mission_mask & (1 << kind) != 0
                        }).map(|wing| {
                            let gate = gates.get(&wing.id).copied().flatten();
                            if gate == Some(false) { return (gate, Ok(false)); }
                            let classification = u8::from(wing.mission_mask & 0x1f7fa != 0);
                            let received = *damage[classification as usize].get_or_insert_with(|| {
                                if combat.is_err() { return Err("the regional combat census is incomplete"); }
                                let Some(holder) = holder else { return Ok(false); };
                                replay_derive::air_region_derive::received(
                                    replay_derive::air_region_derive::Retained { own: &holder.own, foreign: &holder.foreign },
                                    frame.hour, false,
                                    replay_derive::air_region_derive::Filter { missions: 1 << kind, window: 24, classification },
                                ).map(|sum| sum.amounts[9] > 0)
                                    .map_err(|_| "the regional interception aggregate cannot be derived")
                            });
                            (gate, received)
                        }));
                        if alert == Ok(true) {
                            prepared.alerts.insert((region, kind));
                        }
                        let reason = alert.err();
                        let reason =
                            reason.or((kind == AIR_SUPPLY && count == 0).then_some(SUPPLY_REFUSAL));
                        if let Some(reason) = reason {
                            prepared.uncertain.insert((region, kind), reason);
                            *prepared
                                .refused
                                .entry(format!("{ENTRY} fragment withheld: {reason}"))
                                .or_default() += 1;
                        }
                    }
                }
                prepared.prepare_hovers(inputs, frame, &regional_wings);
                prepared.prepare_ground(inputs, frame, vision.ours());
                prepared
            }
            Err(reason) => Self {
                refused: BTreeMap::from([(format!("{WINDOW} counts withheld: {reason}"), 1)]),
                ..Default::default()
            },
        })
    }
}

/// The hover averages current sortie efficiencies weighted by each selected
/// wing's aircraft, with native signed accumulation and fixed5 operations.
fn average(values: impl IntoIterator<Item = (i32, i64)>) -> Option<i64> {
    let (mut total, mut weighted) = (0_i32, 0_i64);
    for (planes, efficiency) in values {
        total = total.wrapping_add(planes);
        weighted = weighted.wrapping_add(
            i64::from(planes)
                .wrapping_mul(efficiency)
                .wrapping_mul(100_000)
                / 100_000,
        );
    }
    if total <= 0 {
        return None;
    }
    let denominator = i64::from(total) * 100_000;
    Some(
        if (weighted as u64).wrapping_add(0x7fff_ffff_ffff_fffe) > 0xffff_ffff_ffff_fffc {
            4_294_967_295
        } else {
            weighted.wrapping_mul(100_000) / denominator
        },
    )
}

impl Prepared {
    fn prepare_ground(&mut self, inputs: &HourInputs<'_>, frame: &DecodedFrame, viewer: u32) {
        use installed_content_localisation::localisation::Argument;
        use viewer_foundation::tooltip_text::Tooltip;
        let flags = frame.complete::<tables::AirGroundCrews>();
        let power = frame
            .complete::<tables::Countries>()
            .ok()
            .and_then(|rows| rows.iter().find(|row| row.canonical_slot == viewer))
            .map(|row| row.command_power_raw);
        for (&region, counts) in &self.regions {
            if !counts.iter().any(|count| *count >= 0) {
                continue;
            }
            let result = (|| -> Result<(Ground, Arc<Tooltip>), String> {
                let (cost, boost) = *inputs
                    .content
                    .ground_crew_defines
                    .as_ref()
                    .map_err(Clone::clone)?;
                let flags = flags.as_ref().map_err(|error| error.to_string())?;
                let active = match flags
                    .binary_search_by_key(&(viewer, region), |row| (row.country, row.region))
                {
                    Ok(at) => flags[at].enabled.ok_or(
                        "Ground Crews writer certificate or enabled-byte read is unavailable",
                    )?,
                    Err(_) => 0,
                };
                let state = ground(active, power, cost)?;
                let loc = inputs.localisation;
                let name = loc
                    .localise(
                        inputs
                            .content
                            .region_names
                            .get(&region)
                            .ok_or("strategic region name is unavailable")?
                            .as_bytes(),
                    )
                    .map_err(|error| error.to_string())?;
                let mut tooltip = Tooltip::default();
                tooltip.line(
                    false,
                    0,
                    "ground crews",
                    loc.localise(b"AIR_MORE_GROUND_CREWS")
                        .and_then(|name| {
                            loc.localise_with(b"HEADER", &[("KEY", Argument::Text(&name))])
                        })
                        .map_err(|error| error.to_string()),
                );
                tooltip.body = if active != 0 {
                    loc.localise_with(
                        b"AIR_MORE_GROUND_CREWS_CANCEL_DESC",
                        &[("NAME", Argument::Text(&name))],
                    )
                } else {
                    loc.localise_with(
                        b"AIR_MORE_GROUND_CREWS_DESC",
                        &[
                            ("NAME", Argument::Text(&name)),
                            ("VALUE", Argument::Fixed5(cost)),
                            ("VAL", Argument::Fixed5(boost)),
                        ],
                    )
                }
                .map_err(|error| error.to_string())?;
                if !state.enabled {
                    tooltip.body.push('\n');
                    tooltip.body.push_str(
                        &loc.localise(b"AIR_MORE_GROUND_CREWS_NOT_ENOUGH")
                            .map_err(|error| error.to_string())?,
                    );
                }
                Ok((state, Arc::new(tooltip)))
            })();
            match result {
                Ok((state, hover)) => {
                    self.ground.insert(region, state);
                    self.ground_hovers.insert(region, hover);
                }
                Err(reason) => {
                    *self
                        .refused
                        .entry(format!("{WINDOW} Ground Crews withheld: {reason}"))
                        .or_default() += 1;
                }
            }
        }
    }

    fn prepare_hovers(
        &mut self,
        inputs: &HourInputs<'_>,
        frame: &DecodedFrame,
        regional_wings: &RegionalWings<'_>,
    ) {
        use installed_content_localisation::localisation::Argument;
        use viewer_foundation::{
            map::{
                air_reachability::MissionCountrySources, air_registry_sources::AirRegistrySources,
                aircraft_count::SortieSources,
            },
            tooltip_text::Tooltip,
        };
        const KEYS: [&str; MISSIONS] = [
            "AIRWING_MISSION_TYPE_AIR_SUPERIORITY",
            "AIRWING_MISSION_TYPE_CAS",
            "AIRWING_MISSION_TYPE_INTERCEPTION",
            "AIRWING_MISSION_TYPE_STRATEGIC_BOMBER",
            "AIRWING_MISSION_TYPE_NAVAL_BOMBER",
            "AIRWING_MISSION_TYPE_DROP_NUKE",
            "AIRWING_MISSION_TYPE_PARADROP",
            "AIRWING_MISSION_TYPE_NAVAL_KAMIKAZE",
            "AIRWING_MISSION_TYPE_PORT_STRIKE",
            "AIRWING_MISSION_TYPE_ATTACK_LOGISTICS",
            "AIRWING_MISSION_TYPE_AIR_SUPPLY",
            "AIRWING_MISSION_TYPE_TRAINING",
            "AIRWING_MISSION_TYPE_NAVAL_MINE_PLANTING",
            "AIRWING_MISSION_TYPE_NAVAL_MINE_SWEEPING",
            "AIRWING_MISSION_TYPE_RECON",
            "AIRWING_MISSION_TYPE_NAVAL_PATROL",
            "AIRWING_MISSION_TYPE_BARRAGE",
            "AIRWING_MISSION_TYPE_SAM",
        ];
        let loc = inputs.localisation;
        // Keep each region's wing order and per-wing refusals inside the fallible preparation.
        #[allow(clippy::type_complexity)]
        let efficiencies =
            (|| -> Result<BTreeMap<u32, Vec<(&AirWingRow, Result<i64, String>)>>, String> {
                let links: BTreeMap<_, _> = frame
                    .complete::<tables::CountryLinks>()
                    .map_err(|e| e.to_string())?
                    .iter()
                    .map(|r| (r.id, r.canonical_slot))
                    .collect();
                let sources = SortieSources::read(frame)
                    .map_err(|e| e.to_string())?
                    .map_err(str::to_owned)?;
                let registry = AirRegistrySources::read(frame)
                    .map_err(|e| e.to_string())?
                    .ok_or("air base registry is incomplete")?;
                let countries = MissionCountrySources::read(frame, &links, &registry)
                    .map_err(|e| e.to_string())?;
                let defines = inputs
                    .assets
                    .air_sortie_defines
                    .as_ref()
                    .map_err(Clone::clone)?;
                let mut out = BTreeMap::<_, Vec<_>>::new();
                for (&region, wings) in regional_wings {
                    for &wing in wings {
                        let efficiency = (|| -> Result<i64, String> {
                            // A zero-plane wing contributes zero for every native
                            // efficiency, so its otherwise unused operands are not demanded.
                            if wing.planes == 0 {
                                return Ok(0);
                            }
                            sources
                                .wing_efficiency(
                                    wing,
                                    &registry,
                                    &countries,
                                    defines.fuel_penalty_raw,
                                )
                                .map_err(str::to_owned)
                        })();
                        out.entry(region).or_default().push((wing, efficiency));
                    }
                }
                Ok(out)
            })();
        for (&region, counts) in &self.regions {
            for (kind, _) in counts.iter().enumerate().filter(|(_, count)| **count >= 0) {
                let mut tooltip = Tooltip::default();
                let name = loc
                    .localise(KEYS[kind].as_bytes())
                    .map_err(|e| e.to_string());
                tooltip.line(
                    false,
                    0,
                    "mission name",
                    name.and_then(|name| {
                        loc.localise_with(b"HEADER", &[("KEY", Argument::Text(&name))])
                            .map_err(|e| e.to_string())
                    }),
                );
                let value = efficiencies
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|regions| {
                        let values = regions
                            .get(&region)
                            .into_iter()
                            .flatten()
                            .filter(|(wing, _)| wing.mission_mask & (1 << kind) != 0)
                            .map(|(wing, value)| value.clone().map(|value| (wing.planes, value)))
                            .collect::<Result<Vec<_>, _>>()?;
                        Ok(average(values))
                    });
                if self.alerts.contains(&(region, kind)) {
                    tooltip.line(
                        false,
                        1,
                        "interception warning",
                        loc.localise(b"AIR_VIEW_INTERCEPTED_EN_ROUTE_BY_ENEMY")
                            .map_err(|e| e.to_string()),
                    );
                }
                match value {
                    Ok(Some(value)) => tooltip.line(
                        false,
                        1,
                        "mission efficiency",
                        loc.localise_with(
                            b"AIRWING_MISSION_EFFICIENCY",
                            &[("AMOUNT", Argument::Fixed5(value))],
                        )
                        .map_err(|e| e.to_string()),
                    ),
                    Ok(None) => {}
                    Err(reason) => tooltip.refuse("mission efficiency", reason),
                }
                if let Some(reason) = self.uncertain.get(&(region, kind)) {
                    tooltip.refuse("mission count and alert", (*reason).into());
                }
                match loc.localise(b"LMB_TO_SELECT_ALL") {
                    Ok(text) => tooltip.body = text,
                    Err(error) => tooltip.refuse("mission selection hint", error.to_string()),
                }
                self.hovers.insert((region, kind), Arc::new(tooltip));
            }
        }
    }
}

#[derive(Default)]
pub(super) struct View {
    pool: PoolView,
    entries: HashMap<SlotId, Vec<Instance>>,
    last: Option<f32>,
    active: bool,
}

impl View {
    pub(super) fn retry(&self) -> Option<Retry> {
        self.pool.retry()
    }
    pub(super) fn take_active(&mut self) -> bool {
        let pool = self.pool.take_running();
        std::mem::take(&mut self.active) || pool
    }
}

fn named<'a>(
    root: &'a native::Element,
    kind: native::Kind,
    name: &str,
) -> Result<&'a native::Element, String> {
    root.find(kind, name)
        .ok_or_else(|| format!("{} has no {name}", root.path))
}

fn setup_entry(
    root: &native::Element,
    kind: usize,
    viewport: [i32; 2],
    resources: &dyn native_gui::Resources,
) -> Result<Instance, String> {
    let mut entry = Instance::construct(root, viewport, resources)?;
    entry.set_frame(
        root,
        named(root, native::Kind::Button, "button")?,
        kind as i32 + 1,
    )?;
    for name in ["supply_count", "supply_count_small"] {
        let control = named(root, native::Kind::TextBox, name)?;
        entry.hide(root, control)?;
        entry.set_hide_flag(root, control, true)?;
    }
    for name in ["alert", "supply_icon"] {
        let control = named(root, native::Kind::Icon, name)?;
        entry.hide(root, control)?;
        entry.set_hide_flag(root, control, true)?;
    }
    entry.hide(root, root)?;
    Ok(entry)
}

impl Prepared {
    pub(super) fn draw(
        &self,
        inputs: &CounterInputs<'_>,
        time: f32,
        view: &mut View,
        batch: &mut Batch,
        report: &mut CounterReport,
    ) {
        for (reason, count) in &self.refused {
            *report.icon_gui_notes.entry(reason.clone()).or_default() += count;
        }
        let resources = gui_windows::gui_assets(inputs, None);
        let viewport = gui_resources_viewport(inputs.viewport, inputs.ui_scale);
        let context = Context {
            resources: &resources,
            windows: inputs.windows,
            viewport,
            time: Some(time),
        };
        if !view.pool.same(&context) {
            view.entries.clear();
            view.last = None;
        }
        let schedule = inputs.schedule.lock().unwrap_or_else(|p| p.into_inner());
        let mut placed = BTreeMap::new();
        if inputs.visibility.priorities == PriorityTable::StrategicAir {
            for &region in inputs.content.region_provinces.keys() {
                let first = inputs
                    .content
                    .region_provinces(region)
                    .iter()
                    .copied()
                    .filter(|&province| schedule.listed(province))
                    .min();
                let Some(first) = first else {
                    continue;
                };
                let Some(anchor) = inputs.content.region_icon(region) else {
                    continue;
                };
                if !inputs.shows(inputs.content.defines.naval_mission_cutoff) {
                    continue;
                }
                if let Some(screen) = inputs.screen(anchor) {
                    placed.insert(region, (first, anchor, screen));
                }
            }
        }
        let mut owners: Vec<_> = placed
            .iter()
            .map(|(&id, &(first, _, screen))| {
                (
                    first,
                    Owner {
                        id,
                        screen,
                        dirty: schedule.dirty(first),
                    },
                )
            })
            .collect();
        owners.sort_by_key(|(first, _)| *first);
        let owners: Vec<_> = owners.into_iter().map(|(_, owner)| owner).collect();
        let root = inputs.windows.gui_type(WINDOW.as_bytes());
        let item = inputs.windows.gui_type(ENTRY.as_bytes());
        let tick = viewer_foundation::map::icon_windows::frame(&mut view.last, time);
        let drawn = view.pool.present(
            &context,
            &Plan {
                name: WINDOW,
                kind: MapIconPool::AirMission,
                alignment: 0x12,
                owners: &owners,
            },
            Box::new(|instance, root| {
                for (kind, name) in [
                    (native::Kind::Icon, "more_ground_crews_bg"),
                    (native::Kind::Button, "more_ground_crews_icon"),
                ] {
                    instance.hide(root, named(root, kind, name)?)?;
                }
                Ok(())
            }),
            |region, slot, instance, root| {
                let item = item.ok_or_else(|| format!("no GUI template {ENTRY}"))?;
                let entries = view.entries.entry(slot).or_default();
                if entries.is_empty() {
                    for kind in 0..MISSIONS {
                        entries.push(setup_entry(item, kind, viewport, &resources)?);
                    }
                }
                let counts = self.regions.get(&region).copied().unwrap_or([-1; MISSIONS]);
                let background = named(root, native::Kind::Icon, "more_ground_crews_bg")?;
                let button = named(root, native::Kind::Button, "more_ground_crews_icon")?;
                if let Some(ground) = self.ground.get(&region) {
                    instance.show(root, background)?;
                    instance.show(root, button)?;
                    instance.set_frame(root, background, ground.frame)?;
                    instance.control(root, button)?.enabled = Some(ground.enabled);
                } else {
                    instance.hide(root, background)?;
                    instance.hide(root, button)?;
                }
                for (kind, entry) in entries.iter_mut().enumerate() {
                    let count = counts[kind];
                    if count < 0 {
                        entry.hide(item, item)?;
                        continue;
                    }
                    let alerted = self.alerts.contains(&(region, kind));
                    let alert = named(item, native::Kind::Icon, "alert")?;
                    entry.set_hide_flag(item, alert, !alerted)?;
                    if alerted {
                        entry.show(item, alert)?;
                    } else {
                        entry.hide(item, alert)?;
                    }
                    let text = named(item, native::Kind::TextBox, "count")?;
                    if self.uncertain.contains_key(&(region, kind)) {
                        entry.hide(item, text)?;
                        entry.set_hide_flag(item, text, true)?;
                    } else {
                        entry.set_hide_flag(item, text, false)?;
                        entry.show(item, text)?;
                        let text_value = if alerted {
                            let control = viewer_foundation::map::native_text::COLOR;
                            format!("{control}R{count}{control}!")
                        } else {
                            count.to_string()
                        };
                        entry.set_text(item, text, &text_value, &resources)?;
                    }
                    entry.gui_show(item, item)?;
                }
                instance.show(root, named(root, native::Kind::List, "missions_list")?)
            },
            &mut report.icon_gui_notes,
        );
        let slots: HashMap<_, _> = view.pool.live().into_iter().collect();
        let (Some(root), Some(item)) = (root, item) else {
            return;
        };
        let list = match named(root, native::Kind::List, "missions_list") {
            Ok(list) => list,
            Err(_) => return,
        };
        view.active = false;
        for drawn in drawn {
            let draw_layer = layer(inputs.priority(PriorityIcon::AirMission), drawn.owner, 0);
            let Some(entries) = slots
                .get(&drawn.owner)
                .and_then(|slot| view.entries.get_mut(slot))
            else {
                continue;
            };
            let counts = self
                .regions
                .get(&drawn.owner)
                .copied()
                .unwrap_or([-1; MISSIONS]);
            let mut parts = vec![Vec::new(); drawn.layout.holders.len()];
            // Only an icon that draws takes its hits.
            let mut hits = Vec::new();
            let run = || -> Result<(), String> {
                let Some(at) = drawn
                    .layout
                    .holders
                    .iter()
                    .position(|holder| holder.path == list.path)
                else {
                    return Ok(());
                };
                let holder = &drawn.layout.holders[at];
                let size = native_gui::container_frame(item, Some(viewport))?.size;
                let kinds: Vec<_> = (0..MISSIONS).filter(|&kind| counts[kind] >= 0).collect();
                let places = native_gui::list_places(
                    list,
                    holder.content,
                    size,
                    kinds.len(),
                    holder.position,
                )?;
                for (index, place) in places {
                    let entry = &mut entries[kinds[index]];
                    entry.set_position(item, item, place)?;
                }
                for (kind, entry) in entries.iter_mut().enumerate() {
                    entry.tick_clocks(item, &resources, tick)?;
                    entry.tick_transitions(item)?;
                    entry.tick_draws(item, &resources)?;
                    view.active |= entry.active();
                    if let Some(mut layout) = entry
                        .layout(item, &resources, None)
                        .map_err(|e| format!("{e:?}"))?
                    {
                        count_noted(&mut report.icon_gui_notes, ENTRY, &layout.diagnostics);
                        parts[at].extend(holder.place(entry.position(), &layout));
                        layout.parts = holder.place(entry.position(), &layout);
                        layout.regions = holder.place_regions(entry.position(), &layout.regions);
                        if let Some(hit) = hit_rectangle(
                            named(item, native::Kind::Button, "button")?,
                            &layout,
                            drawn.origin,
                            &resources,
                            &mut report.icon_gui_notes,
                        ) {
                            hits.push(AirMissionHover {
                                region: drawn.owner,
                                mission: Some(kind),
                                rect: hit.rect,
                                layer: draw_layer,
                                text: self
                                    .hovers
                                    .get(&(drawn.owner, kind))
                                    .cloned()
                                    .unwrap_or_default(),
                            });
                        }
                    }
                }
                Ok(())
            };
            let mut run = run;
            if let Err(why) = run() {
                *report
                    .icon_gui_notes
                    .entry(format!("{WINDOW} withheld: {why}"))
                    .or_default() += 1;
                continue;
            }
            let Some(&(_, anchor, _)) = placed.get(&drawn.owner) else {
                continue;
            };
            if let Some(hover) = self.ground_hovers.get(&drawn.owner)
                && let Ok(button) = named(root, native::Kind::Button, "more_ground_crews_icon")
                && let Some(hit) = hit_rectangle(
                    button,
                    &drawn.layout,
                    drawn.origin,
                    &resources,
                    &mut report.icon_gui_notes,
                )
            {
                hits.push(AirMissionHover {
                    region: drawn.owner,
                    mission: None,
                    rect: hit.rect,
                    layer: draw_layer,
                    text: Arc::clone(hover),
                });
            }
            batch.at_anchor(inputs.icon_anchor(
                IconFamily::Missions,
                anchor,
                inputs.content.defines.naval_mission_cutoff,
            ));
            batch.at_layer(draw_layer);
            match native_gui::draw_composed(&drawn.layout, &parts, drawn.origin, &resources, batch)
            {
                Ok(()) => report.air_mission_hits.extend(hits),
                Err(why) => count_refused(&mut report.icon_gui_notes, WINDOW, &[why]),
            }
            note_supply_refusals(counts[AIR_SUPPLY], &mut report.icon_gui_notes);
        }
        batch.at_anchor(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supply_refusals_distinguish_unknown_entry_admission_from_known_counts() {
        let fragments = |count| {
            let mut notes = BTreeMap::new();
            note_supply_refusals(count, &mut notes);
            notes
        };
        let absent = fragments(-1);
        assert_eq!(absent.len(), 4);
        assert!(
            absent
                .keys()
                .any(|key| key.contains("mission entry admission"))
        );
        assert!(!absent.keys().any(|key| key.contains("warning colour")));

        let empty = fragments(0);
        assert_eq!(empty.len(), 4);
        assert!(
            !empty
                .keys()
                .any(|key| key.contains("mission entry admission"))
        );
        assert!(
            empty
                .keys()
                .any(|key| key.contains("zero-aircraft warning colour"))
        );

        let positive = fragments(17);
        assert_eq!(positive.len(), 3);
        assert!(
            !positive.keys().any(
                |key| key.contains("mission entry admission") || key.contains("warning colour")
            )
        );
        for fragment in ["severity icon", "amount text", "regional hover"] {
            assert!(positive.keys().any(|key| key.contains(fragment)));
        }
        assert!(positive.values().all(|&count| count == 1));
    }

    #[test]
    fn supply_refusal_counts_follow_regional_icons_without_hiding_positive_counts() {
        let mut notes = BTreeMap::new();
        note_supply_refusals(8, &mut notes);
        note_supply_refusals(8, &mut notes);
        assert_eq!(notes.len(), 3);
        assert!(notes.values().all(|&count| count == 2));
        let mut counts = [-1; MISSIONS];
        count(
            &mut counts,
            &AirWingRow {
                planes: 8,
                mission_mask: 1 << AIR_SUPPLY,
                ..Default::default()
            },
        );
        assert_eq!(counts[AIR_SUPPLY], 8);
    }
    #[test]
    fn ground_crews_keep_active_controls_enabled_and_compare_unrounded_power() {
        assert_eq!(
            ground(1, None, 20),
            Ok(Ground {
                frame: 3,
                enabled: true
            })
        );
        assert_eq!(
            ground(0, Some(19), 20),
            Ok(Ground {
                frame: 2,
                enabled: false
            })
        );
        assert_eq!(
            ground(0, Some(20), 20),
            Ok(Ground {
                frame: 1,
                enabled: true
            })
        );
        assert!(ground(0, None, 20).is_err());
        assert!(ground(2, Some(100), 20).is_err());
    }
    #[test]
    fn interception_uses_the_trace_gate_and_any_proved_alert_overrides_unknown_entries() {
        assert_eq!(interception_alert([(None, Ok(false))]), Ok(false));
        assert_eq!(
            interception_alert([(Some(false), Err("combat unavailable"))]),
            Ok(false)
        );
        assert!(interception_alert([(None, Ok(true))]).is_err());
        assert_eq!(
            interception_alert([(None, Ok(true)), (Some(true), Ok(true))]),
            Ok(true)
        );
        assert_eq!(interception_alert([(Some(true), Ok(true))]), Ok(true));
        assert_eq!(interception_alert([(Some(true), Ok(false))]), Ok(false));
    }

    #[test]
    fn hover_efficiency_weights_aircraft_and_omits_a_nonpositive_total() {
        assert_eq!(average([(10, 20_000), (30, 80_000)]), Some(65_000));
        assert_eq!(average([(0, 80_000)]), None);
        assert_eq!(average([(-10, 80_000)]), None);
        assert_eq!(average([(1, 100_000)]), Some(100_000));
    }
    #[test]
    fn registrations_count_selected_missions_in_native_order_including_empty_wings() {
        let wings = vec![
            AirWingRow {
                id: 7,
                owner: 100,
                mission_region: Some(3),
                planes: 9,
                mission_mask: 5,
                ..Default::default()
            },
            AirWingRow {
                id: 8,
                owner: 100,
                mission_region: Some(3),
                planes: 0,
                mission_mask: 8,
                ..Default::default()
            },
            AirWingRow {
                id: 9,
                owner: 200,
                mission_region: Some(3),
                planes: 400,
                mission_mask: 1,
                ..Default::default()
            },
            AirWingRow {
                id: 10,
                owner: 100,
                mission_region: Some(3),
                planes: 500,
                mission_mask: 1,
                ..Default::default()
            },
        ];
        let registrations = [
            AirRegionalMissionRow {
                wing: 7,
                ordinal: 1,
                ..Default::default()
            },
            AirRegionalMissionRow {
                wing: 8,
                ordinal: 0,
                ..Default::default()
            },
            AirRegionalMissionRow {
                wing: 9,
                ordinal: 0,
                ..Default::default()
            },
        ];
        let (rows, regional_wings) =
            regional_counts(&registrations, &wings, 1, |raw| Some(raw / 100)).unwrap();
        assert_eq!(&rows[&3][..5], &[9, -1, 9, 0, -1]);
        assert_eq!(
            regional_wings[&3]
                .iter()
                .map(|wing| wing.id)
                .collect::<Vec<_>>(),
            [8, 7]
        );
        assert!(
            regional_counts(
                &[AirRegionalMissionRow {
                    wing: 50,
                    ordinal: 0,
                    ..Default::default()
                }],
                &wings,
                1,
                |_| Some(1)
            )
            .is_err()
        );
    }
    #[test]
    fn negative_sums_restart_and_signed_masks_do_not_admit_entries() {
        let mut counts = [-1; MISSIONS];
        for planes in [-4, 9] {
            count(
                &mut counts,
                &AirWingRow {
                    planes,
                    mission_mask: 1,
                    ..Default::default()
                },
            );
        }
        assert_eq!(counts[0], 9);
        count(
            &mut counts,
            &AirWingRow {
                planes: 30,
                mission_mask: 0x8000_0001,
                ..Default::default()
            },
        );
        assert_eq!(counts[0], 9);
    }
}
