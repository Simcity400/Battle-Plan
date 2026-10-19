//! Naval combat and retained naval result map icons.
//!
//! Combat placement is per province ([`crate::map::naval_combat_occurrences`]).
//! Engaged combats update the icon bound to their own province. HQ frame selection
//! also runs for every live-combat icon. Each province has one visible retained-result
//! icon. Engagement comes from the viewing country's shared index.
//!
//! Persistent pooled windows retain the background frame from their last engaged
//! update. A new view can establish it from [`viewer_foundation::engagement_history::kept_frames`];
//! otherwise the binding's initial frame one is used with a counted history gap.
//! Placement, censuses, HQ frames and results are prepared per hour and perspective;
//! camera changes only place and draw the prepared icons.

use super::*;
use replay_data::{
    rows::NavalCombatDetailsRow, rows::NavalCombatShipRow, rows::NavalProvinceCombatRow,
    rows::NavalResultRow,
};
use std::cell::OnceCell;
use viewer_foundation::map::gui_resources::HitRectangle;
use viewer_foundation::map::gui_resources::hit_rectangle;
use viewer_foundation::map::gui_resources::viewport;
use viewer_foundation::naval_icon_frames::{Census, admitted, censuses, convoy_variant};
use {
    super::super::naval_combat_grid as grid, crate::map::naval_combat_occurrences as occurrences,
    crate::map::naval_combat_occurrences::Combat, crate::map::naval_combat_occurrences::Direct,
    crate::map::naval_combat_occurrences::Occurrence,
    crate::map::naval_combat_occurrences::Sources, crate::map::naval_combat_occurrences::Transfers,
    crate::map::naval_combat_occurrences::Units,
};
use {
    viewer_foundation::engagement_history::CombatFlags,
    viewer_foundation::{
        engaged::Engaged,
        icon_frames::{self, Kept},
        map::native_gui,
    },
};

/// The sprite literals: every visible
/// result a naval strike, every one a port strike, a plain battle, a convoy battle.
pub const NAVAL_RESULT_SPRITES: [&str; 4] = [
    "GFX_naval_combat_result_navalstrikes",
    "GFX_naval_combat_result_portstrikes",
    "GFX_naval_combat_result",
    "GFX_naval_combat_result_convoys",
];

/// The inputs both icon kinds share for one hour and perspective.
pub(super) struct MarkerContext<'a> {
    pub(super) vision: &'a Vision,
    pub(super) owner: &'a dyn Fn(u32) -> Option<u32>,
    /// The naval counter operands the grid tiles share with the port items: the unit passes'
    /// context, or, when no unit counter draws (`units_shown`, which does not gate the combat
    /// icons), what the grids build it from the first time a large icon needs them.
    pub(super) naval: Result<&'a NavalContext<'a>, NavalSources<'a>>,
    pub(super) countries: &'a Countries,
    /// This hour's engaged flag of every naval, land and border-war combat for the viewer.
    pub(super) flags: &'a CombatFlags,
}

/// The viewing country and how a raw country reference resolves: all a per-hour decision
/// ([`NavalMarkerTables`]) needs of the perspective.
#[derive(Clone, Copy)]
pub(super) struct Viewer<'a> {
    pub(super) vision: &'a Vision,
    pub(super) owner: &'a dyn Fn(u32) -> Option<u32>,
}

impl Viewer<'_> {
    /// Whether any raw country reference resolves to the viewer's canonical slot.
    /// `None` when no match is found and a reference does not resolve.
    fn contains_viewer(&self, list: &[u32]) -> Option<bool> {
        let mut unresolved = false;
        for &raw in list {
            match (self.owner)(raw) {
                Some(slot) if slot == self.vision.ours() => return Some(true),
                Some(_) => {}
                None => unresolved = true,
            }
        }
        (!unresolved).then_some(false)
    }
}

impl MarkerContext<'_> {
    /// Side allegiance used for progress direction and background selection.
    fn side_allegiance(&self, side: &str, countries: &[u32]) -> Result<Allegiance, String> {
        let ours = self.vision.ours();
        for &raw in countries {
            let slot = (self.owner)(raw)
                .ok_or_else(|| format!("{side} side country {raw} has no CountryLinks row"))?;
            if slot == ours {
                return Ok(Allegiance::Ours);
            }
            if self
                .vision
                .shares_vision_proven(slot)
                .map_err(|cause| cause.describe())?
            {
                return Ok(Allegiance::Allied);
            }
            if self
                .vision
                .at_war_proven(slot)
                .map_err(|cause| cause.describe())?
            {
                return Ok(Allegiance::Enemy);
            }
        }
        Ok(Allegiance::Other)
    }

    /// the convoy addition's background: by `allegiance` (the walk
    /// above) when `counter_color_mode` is 1; otherwise the UI colour of the side's first
    /// country, or `COUNTER_MODE_ALLEGIANCE_OTHER` for an
    /// empty list.
    fn convoy_tint(
        &self,
        inputs: &CounterInputs<'_>,
        allegiance: Allegiance,
        countries: &[u32],
    ) -> Result<[f32; 4], String> {
        let first = countries.first().map(|&raw| (self.owner)(raw).ok_or(raw));
        convoy_color(
            inputs.settings.color_mode,
            &inputs.content.defines.tints,
            self.countries,
            allegiance,
            first,
        )
    }

    /// The flip, [`Vision::turns_naval_progress`] over the
    /// attacker's primary countries in order. Undecided, it names why.
    fn flip(&self, detail: &NavalCombatDetailsRow) -> Result<bool, String> {
        self.vision
            .turns_naval_progress(
                detail.sides[0]
                    .summary
                    .countries
                    .iter()
                    .map(|&raw| (self.owner)(raw)),
            )
            .map_err(|cause| cause.describe())
    }
}

/// A convoy addition uses the side's allegiance colour in allegiance mode. In
/// country mode it uses the first country's UI colour (`first`, its slot or an
/// unresolved raw reference), or `COUNTER_MODE_ALLEGIANCE_OTHER` for an empty list.
fn convoy_color(
    mode: CounterColorMode,
    tints: &allegiance::Tints,
    countries: &Countries,
    allegiance: Allegiance,
    first: Option<Result<u32, u32>>,
) -> Result<[f32; 4], String> {
    match (mode, first) {
        (CounterColorMode::Allegiance, _) => Ok(tints.of(allegiance)),
        (CounterColorMode::Country, None) => Ok(tints.other),
        (CounterColorMode::Country, Some(Ok(slot))) => countries.ui_color(slot),
        (CounterColorMode::Country, Some(Err(raw))) => {
            Err(format!("its first country {raw} is not a linked country"))
        }
    }
}

/// Each cause a naval combat icon or icon part is omitted or undecided for, with the combats it
/// affects: the `("Naval", cause)` entries of [`CounterReport::combat_gaps`], which the windowed
/// diagnostics, the map details and `--dump` print as one line per cause naming its combats.
pub(super) fn naval_gap(
    report: &mut CounterReport,
    cause: String,
    combats: impl IntoIterator<Item = u64>,
) {
    report
        .combat_gaps
        .entry(("Naval", cause))
        .or_default()
        .extend(combats);
}

/// Why `table` is not whole this hour, if it is not.
fn whole<T: replay_data::rows::TableSchema>(
    table: &Available<'_, T>,
    name: &str,
) -> Option<String> {
    match table {
        Available::Complete(_) => None,
        Available::Truncated { missing, .. } => Some(format!(
            "the {name} table is truncated ({missing} rows unread)"
        )),
        Available::Unavailable { reason } => {
            Some(format!("the {name} table is unavailable ({reason})"))
        }
    }
}

/// How `table` lacks the row for `key` this hour.
fn missing<T: replay_data::rows::TableSchema>(
    table: &Available<'_, T>,
    name: &str,
    key: impl std::fmt::Display,
) -> String {
    match table {
        Available::Complete(_) => format!("the {name} table has no row for {key}"),
        Available::Truncated { .. } => format!("the truncated {name} table did not reach {key}"),
        Available::Unavailable { reason } => {
            format!("the {name} table is unavailable ({reason}), so {key} is unknown")
        }
    }
}

/// A task force the HQ frame reads: the row and its attached naval-HQ count, or which census
/// lacks it.
type ForceInput<'a> = Result<(&'a replay_data::rows::TaskForceRow, i32), String>;

/// The side whose primary list holds the viewer (attacker first) shows the HQ
/// frame when any live ship in its forward groups belongs to a viewer-owned task
/// force with attached naval headquarters. `members` are the combat's
/// [`grid::ForwardMembers`]. The game collects all forward members' task forces
/// before testing them, so their order cannot change the positive result.
/// A known qualifying force decides it; otherwise unresolved members leave it
/// undecided, named by the first such member's cause.
fn hq_frame<'a>(
    viewer: Viewer<'_>,
    detail: &NavalCombatDetailsRow,
    members: impl IntoIterator<Item = &'a NavalCombatShipRow>,
    ship_forces: &BTreeMap<u64, u64>,
    forces: &BTreeMap<u64, ForceInput<'_>>,
) -> Result<bool, String> {
    let mut side = None;
    for (index, candidate) in detail.sides.iter().enumerate() {
        let contains = viewer
            .contains_viewer(&candidate.summary.countries)
            .ok_or_else(|| {
                let raw = candidate
                    .summary
                    .countries
                    .iter()
                    .find(|raw| (viewer.owner)(**raw).is_none());
                format!(
                    "side {index} country {} has no CountryLinks row",
                    raw.copied().unwrap_or_default()
                )
            })?;
        if contains {
            side = Some(index as u8);
            break;
        }
    }
    let Some(side) = side else {
        return Ok(false);
    };
    let mut unresolved = None;
    for member in members
        .into_iter()
        .filter(|member| member.side == side && member.live_ship != 0)
    {
        let qualifies = (|| {
            let force = ship_forces
                .get(&member.ship)
                .ok_or_else(|| format!("the Ships table has no row for ship {}", member.ship))?;
            let (row, headquarters) = forces.get(force).cloned().unwrap_or_else(|| {
                Err(format!(
                    "the TaskForces table has no row for task force {force}"
                ))
            })?;
            let owner = (viewer.owner)(row.roster_owner).ok_or_else(|| {
                format!(
                    "task force {force}'s roster owner {} has no CountryLinks row",
                    row.roster_owner
                )
            })?;
            Ok::<_, String>(owner == viewer.vision.ours() && headquarters > 0)
        })();
        match qualifies {
            Ok(true) => return Ok(true),
            Ok(false) => {}
            Err(cause) => {
                unresolved.get_or_insert(cause);
            }
        }
    }
    unresolved.map_or(Ok(false), Err)
}

/// Each detailed combat's [`hq_frame`], or why a census it reads is incomplete.
type HqFrames = Result<BTreeMap<u64, Result<bool, String>>, String>;

/// [`hq_frame`] for every detailed combat, or why a census it reads is incomplete. Only the ships
/// the combats' forward members name, their task forces and those forces' inputs are read, each
/// found in its key-sorted census. `members` is the member census with its forward members, or
/// why that census is not whole.
fn hq_frames(
    frame: &DecodedFrame,
    viewer: Viewer<'_>,
    members: Result<(&[NavalCombatShipRow], &grid::ForwardMembers), &String>,
    details: &BTreeMap<u64, usize>,
    detail_rows: &[NavalCombatDetailsRow],
) -> io::Result<HqFrames> {
    let ships = rows::<tables::Ships>(frame)?;
    let forces = rows::<tables::TaskForces>(frame)?;
    let inputs = rows::<tables::TaskForceCounterInputs>(frame)?;
    let (members, forward) = match members {
        Ok(members) => members,
        Err(cause) => return Ok(Err(cause.clone())),
    };
    if let Some(cause) = whole(&ships, "Ships")
        .or_else(|| whole(&forces, "TaskForces"))
        .or_else(|| whole(&inputs, "TaskForceCounterInputs"))
    {
        return Ok(Err(cause));
    }
    let (ships, forces, inputs) = (ships.rows(), forces.rows(), inputs.rows());
    let ship_forces: BTreeMap<u64, u64> = forward
        .indices()
        .filter_map(|index| {
            let ship = members[index].ship;
            let found = ships.binary_search_by_key(&ship, |row| row.id).ok()?;
            Some((ship, ships[found].task_force))
        })
        .collect();
    let force_inputs: BTreeMap<u64, ForceInput<'_>> = ship_forces
        .values()
        .map(|&id| {
            let input = (|| {
                let force = forces
                    .binary_search_by_key(&id, |row| row.id)
                    .map_err(|_| format!("the TaskForces table has no row for task force {id}"))?;
                let input = inputs
                    .binary_search_by_key(&id, |row| row.task_force)
                    .map_err(|_| {
                        format!("the TaskForceCounterInputs table has no row for task force {id}")
                    })?;
                Ok((&forces[force], inputs[input].attached_headquarters))
            })();
            (id, input)
        })
        .collect();
    Ok(Ok(details
        .iter()
        .map(|(&combat, &index)| {
            let shown = hq_frame(
                viewer,
                &detail_rows[index],
                forward.of(combat, members),
                &ship_forces,
                &force_inputs,
            );
            (combat, shown)
        })
        .collect()))
}

/// Run only for an icon whose bound province equals its combat's province. The
/// bound province comes from the call's anchor, independently of the bucket that
/// owns the icon.
fn rotates(home: u32, occurrence: &Occurrence) -> bool {
    home == occurrence.anchor_province
}

/// The widget size off the coarse zoom: the small one once two
/// entries of the combat province's whole combat vector are engaged, whatever
/// their class. Undecided, it names why.
fn two_engaged(
    flags: &CombatFlags,
    province: u32,
    row: Option<&NavalProvinceCombatRow>,
    complete: bool,
) -> Result<bool, String> {
    flags
        .engaged_entries(row, complete, 2)
        .ok_or_else(|| match row {
            None => format!(
                "province {province}'s combat vector is unknown (the NavalProvinceCombats table \
                 is incomplete)"
            ),
            Some(_) => {
                format!("an engaged flag in province {province}'s combat vector is undecided")
            }
        })
}

/// Where a placed icon's `bg_btn` frame comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FrameSource {
    /// Frame written this hour for a rotating icon of an engaged combat.
    Engaged,
    /// A rotating icon of an unengaged combat keeps its last engaged update's frame: its pool
    /// slot's when the view established it, else the engaged index's answer for the combat
    /// ([`viewer_foundation::engagement_history::kept_frames`]); an unknown history uses the binding's initial
    /// frame with a limitation ([`naval_windows`]).
    Kept(Kept),
    /// A spill-over icon, which the game never rotates: its pool slot's last frame.
    Spillover,
}

/// What one placed combat icon shows, before any camera: the game for its
/// background frame and progress, then the game for its HQ frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Decision {
    /// `bg_btn`'s frame.
    variant: u32,
    /// Whether the progress children are filled: a rotating icon of an engaged combat.
    progress: bool,
    frame: FrameSource,
}

/// A spill-over icon keeps its initial state (no children; its frame [`FrameSource::Spillover`]);
/// a rotating one of an unengaged combat hides its progress and keeps its frame, which `kept`
/// (the index's answer, [`Kept::Undecided`] when it holds none) gives; an engaged one takes
/// The variant. Undecided, it names why.
fn decide(
    rotates: bool,
    engaged: Engaged,
    census: Result<[Census; 2], &str>,
    kept: Option<Kept>,
) -> Result<Decision, String> {
    if !rotates {
        return Ok(Decision {
            variant: 0,
            progress: false,
            frame: FrameSource::Spillover,
        });
    }
    match engaged {
        Engaged::No => {
            let kept = kept.unwrap_or(Kept::Undecided);
            Ok(Decision {
                variant: match kept {
                    Kept::Frame(frame) => frame,
                    _ => 0,
                },
                progress: false,
                frame: FrameSource::Kept(kept),
            })
        }
        Engaged::Yes => Ok(Decision {
            variant: u32::from(convoy_variant(
                census.map_err(|cause| format!("convoy variant undecided: {cause}"))?,
            )),
            progress: true,
            frame: FrameSource::Engaged,
        }),
        // The icon is omitted with the ambiguity's cause, as the land marker names it.
        Engaged::Ambiguous(cause) => Err(cause.reason().into()),
        Engaged::Unknown => Err("the combat's engaged flag for the viewer is unknown".into()),
    }
}

/// The naval icon decisions no camera moves, made once per hour and perspective and kept in the
/// hour tables; a camera's rebuild only places and draws them.
pub(super) struct NavalMarkerTables {
    /// `None` for an hour without a naval combat to place: a complete, empty combat census and
    /// no province vector naming a naval combat.
    combats: Option<CombatTables>,
    /// The hour's retained results.
    results: Arc<ResultTables>,
    /// Each province's combat vector by `NavalProvinceCombats` row.
    vectors: BTreeMap<u32, usize>,
    direct_complete: bool,
    /// Every combat the hour holds.
    live: BTreeSet<u64>,
    /// Why [`Self::live`] is not every combat: the `NavalCombats` census is not whole, so whether
    /// a kept icon's combat still resolves is undecided.
    live_undecided: Option<String>,
    /// Combats with a side of no countries: the game erases their icons.
    gated: BTreeSet<u64>,
}

/// One placed combat icon, decided before any camera.
#[derive(Debug, PartialEq)]
struct Icon {
    combat: u64,
    /// The province whose graphics own it.
    province: u32,
    /// Pass its early returns: the icon stands in its
    /// combat's own province.
    rotates: bool,
    /// The combat's own province.
    home: u32,
    /// The combat's `NavalCombatDetails` row.
    detail: usize,
    /// The bound province's `Standstill` unit-stack anchor.
    anchor: [f32; 3],
    /// Its place in its province's icon stack.
    ordinal: u32,
    decision: Decision,
    /// [`hq_frame`], or why it is undecided.
    hq: Result<bool, String>,
}

/// An admitted icon the passes placed whose decision the hour cannot make: its owner, whether it
/// rotates, where it stands when that is known, and why.
#[derive(Debug, PartialEq)]
struct Omitted {
    combat: u64,
    province: u32,
    rotates: bool,
    /// Its anchor and stack place, when the hour decides them.
    place: Option<([f32; 3], u32)>,
    cause: String,
}

/// The placed combat icons and what each shows before any camera, with every cause one is
/// omitted or undecided for.
struct CombatTables {
    /// Every admitted icon the passes placed, in bucket order: decided, or omitted with its
    /// owner and cause.
    icons: Vec<Result<Icon, Omitted>>,
    /// The placement passes' own causes with the combats each affects: a direct call whose
    /// combat the census lacks or whose side countries are unknown (each an omitted icon), and a
    /// stopped spillover pass.
    placement: Vec<(String, Vec<u64>)>,
    /// Direct calls that placed no icon.
    unplaced: usize,
    /// Missing live-combat details leave icon existence undecided. Retain the
    /// known distinction between erased icons and icons awaiting their next update.
    undecided: Vec<Omitted>,
    /// Each combat's forward members, which the HQ frames and the grids walk; `None` while the
    /// member census is incomplete.
    forward: Option<grid::ForwardMembers>,
    /// Every combat's naval-HQ frame ([`hq_frame`]), which an icon the hour no longer places
    /// still takes.
    hq: HqFrames,
    /// Why this hour's naval engaged flags are load-rebuilt
    /// ([`viewer_foundation::engagement_history::Provenance::limitation`]), if they are.
    provenance: Option<&'static str>,
}

impl NavalMarkerTables {
    /// `kept` is what each live icon keeps for its frame
    /// ([`viewer_foundation::engagement_history::kept_frames`] of the naval icons).
    pub(super) fn read(
        frame: &DecodedFrame,
        viewer: Viewer<'_>,
        flags: &CombatFlags,
        kept: &BTreeMap<u64, Kept>,
        province_kind: &dyn Fn(u32) -> Option<ProvinceKind>,
        anchor: &dyn Fn(u32) -> Option<[f32; 3]>,
    ) -> io::Result<Self> {
        let direct = rows::<tables::NavalProvinceCombats>(frame)?;
        let details = rows::<tables::NavalCombatDetails>(frame)?;
        let combats = rows::<tables::NavalCombats>(frame)?;
        Ok(Self {
            live: combats.rows().iter().map(|row| row.id).collect(),
            live_undecided: whole(&combats, "NavalCombats"),
            gated: details
                .rows()
                .iter()
                .filter(|detail| admitted(Some(detail)) == Some(false))
                .map(|detail| detail.combat)
                .collect(),
            combats: CombatTables::read(
                frame,
                viewer,
                flags,
                kept,
                province_kind,
                anchor,
                &direct,
            )?,
            results: Arc::new(result_tables(frame, viewer)?),
            vectors: direct
                .rows()
                .iter()
                .enumerate()
                .map(|(index, row)| (row.province, index))
                .collect(),
            direct_complete: direct.is_complete(),
        })
    }
}

impl CombatTables {
    fn read(
        frame: &DecodedFrame,
        viewer: Viewer<'_>,
        flags: &CombatFlags,
        kept: &BTreeMap<u64, Kept>,
        province_kind: &dyn Fn(u32) -> Option<ProvinceKind>,
        anchor: &dyn Fn(u32) -> Option<[f32; 3]>,
        direct: &Available<'_, tables::NavalProvinceCombats>,
    ) -> io::Result<Option<Self>> {
        let combats = rows::<tables::NavalCombats>(frame)?;
        // The direct pass admits only a vector's naval entries (combat type 2).
        let direct_refs: Vec<Direct> = direct
            .rows()
            .iter()
            .map(|row| Direct {
                province: row.province,
                combats: row
                    .combats
                    .iter()
                    .zip(&row.kinds)
                    .filter(|(_, kind)| **kind == NavalProvinceCombatRow::NAVAL)
                    .map(|(combat, _)| *combat)
                    .collect(),
            })
            .collect();
        // A complete, empty census ends the spillover pass before it reads a transfer.
        let empty_census = combats.is_complete() && combats.rows().is_empty();
        if empty_census && direct_refs.iter().all(|row| row.combats.is_empty()) {
            return Ok(None);
        }
        let details_table = rows::<tables::NavalCombatDetails>(frame)?;
        let detail_rows = details_table.rows();
        let details: BTreeMap<u64, usize> = detail_rows
            .iter()
            .enumerate()
            .map(|(index, row)| (row.combat, index))
            .collect();
        let detail = |id| details.get(&id).map(|&index| &detail_rows[index]);
        let transfers = rows::<tables::NavalCombatTransfers>(frame)?;
        let units = rows::<tables::NavalTransferUnits>(frame)?;
        let combat_refs: Vec<Combat> = combats
            .rows()
            .iter()
            .map(|row| Combat {
                id: row.id,
                home: row.province,
                registry_ordinal: row.registry_ordinal,
            })
            .collect();
        let transfer_refs: Vec<Transfers> = transfers
            .rows()
            .iter()
            .map(|row| Transfers {
                combat: row.combat,
                transfers: row.transfers.clone(),
            })
            .collect();
        let unit_refs: Vec<Units> = units
            .rows()
            .iter()
            .map(|row| Units {
                transfer: row.transfer,
                units: row.units.clone(),
            })
            .collect();
        let locations = if empty_census {
            None
        } else {
            unit_locations(frame, &unit_refs)?
        };
        let placed = occurrences::derive(
            Sources {
                combats: &combat_refs,
                combats_complete: combats.is_complete(),
                direct: &direct_refs,
                direct_complete: direct.is_complete(),
                transfers: transfers.is_complete().then_some(transfer_refs.as_slice()),
                units: units.is_complete().then_some(unit_refs.as_slice()),
                unit_locations: locations.as_ref(),
                province_kind,
            },
            &|id| admitted(detail(id)),
            &|id| icon_frames::engaged_seen(flags.entry(id, NavalProvinceCombatRow::NAVAL)),
        );
        let mut placement = Vec::new();
        for &(province, combat) in &placed.direct_gaps {
            let key = format!("combat {combat}, which province {province} names");
            placement.push((
                format!("icon omitted: {}", missing(&combats, "NavalCombats", key)),
                vec![combat],
            ));
        }
        let mut undecided = Vec::new();
        for &(province, combat) in &placed.admission_gaps {
            let key = format!("combat {combat}, which province {province} names");
            let cause = format!(
                "icon omitted: its side countries are unknown: {}",
                missing(&details_table, "NavalCombatDetails", key)
            );
            undecided.push(Omitted {
                combat,
                province,
                rotates: combat_refs
                    .iter()
                    .any(|row| row.id == combat && row.home == province),
                place: None,
                cause: cause.clone(),
            });
            placement.push((cause, vec![combat]));
        }
        if let Some(reason) = placed.spillover_gap {
            // The pass commits no spillover until every chain is proven, so a stop omits every
            // combat's spillover icons.
            placement.push((
                format!("spillover icons omitted: {reason}"),
                combat_refs.iter().map(|combat| combat.id).collect(),
            ));
        }
        let members = rows::<tables::NavalCombatShips>(frame)?;
        // The forward members the HQ frames and the grids walk, grouped once per hour.
        let forward = match whole(&members, "NavalCombatShips") {
            Some(cause) => Err(cause),
            None => Ok(grid::ForwardMembers::read(members.rows())),
        };
        let censuses = match &forward {
            Err(cause) => Err(cause.clone()),
            Ok(_) => Ok(censuses(members.rows())),
        };
        let hq = hq_frames(
            frame,
            viewer,
            forward.as_ref().map(|forward| (members.rows(), forward)),
            &details,
            detail_rows,
        )?;
        let homes: BTreeMap<u64, u32> = combats
            .rows()
            .iter()
            .map(|row| (row.id, row.province))
            .collect();
        let mut icons = Vec::new();
        for occurrence in placed.by_province.values().flatten() {
            let Some(&home) = homes.get(&occurrence.combat) else {
                continue;
            };
            let Some(&index) = details.get(&occurrence.combat) else {
                continue;
            };
            if admitted(Some(&detail_rows[index])) != Some(true) {
                continue;
            }
            let combat = occurrence.combat;
            let rotating = rotates(home, occurrence);
            let ordinal = occurrence.ordinal.ok_or_else(|| {
                format!(
                    "its place in province {}'s icon stack is unknown",
                    occurrence.province
                )
            });
            let anchored = anchor(occurrence.anchor_province).ok_or_else(|| {
                format!(
                    "the content places no Standstill unit-stack anchor for province {}",
                    occurrence.anchor_province
                )
            });
            let place = anchored.clone().ok().zip(ordinal.clone().ok());
            let icon = (|| {
                let ordinal = ordinal?;
                let anchor = anchored?;
                // Engagement uses either side's derived viewer flag. A combat absent from
                // this hour is unknown.
                let engaged = if rotating {
                    flags.entry(combat, NavalProvinceCombatRow::NAVAL)
                } else {
                    Engaged::No
                };
                let census = match &censuses {
                    Err(cause) => Err(cause.as_str()),
                    Ok(censuses) => match censuses.get(&combat) {
                        Some(Err(cause)) => Err(cause.as_str()),
                        Some(Ok(census)) => Ok(*census),
                        // A combat with no forward member has empty sides.
                        None => Ok(Default::default()),
                    },
                };
                Ok(Icon {
                    combat,
                    province: occurrence.province,
                    rotates: rotating,
                    home,
                    detail: index,
                    anchor,
                    ordinal,
                    decision: decide(rotating, engaged, census, kept.get(&combat).copied())?,
                    hq: match &hq {
                        Err(cause) => Err(cause.clone()),
                        Ok(hq) => hq[&combat].clone(),
                    },
                })
            })();
            icons.push(icon.map_err(|cause: String| Omitted {
                combat,
                province: occurrence.province,
                rotates: rotating,
                place,
                cause: format!("icon omitted: {cause}"),
            }));
        }
        Ok(Some(Self {
            icons,
            placement,
            unplaced: placed.direct_gaps.len() + placed.admission_gaps.len(),
            undecided,
            forward: forward.ok(),
            hq,
            provenance: flags.naval_provenance.limitation(),
        }))
    }

    /// Counts the icons the hour omits, by cause with their combats, into `report`: the
    /// placement passes' causes, then each omitted icon's. `refused` names a layout refusal that
    /// omits every placed icon instead.
    fn report_omitted(&self, refused: Option<&str>, report: &mut CounterReport) {
        report.unavailable_naval_combats += self.unplaced;
        for (cause, combats) in &self.placement {
            naval_gap(report, cause.clone(), combats.iter().copied());
        }
        for icon in &self.icons {
            let (combat, cause) = match (icon, refused) {
                (Ok(icon), Some(refused)) => (icon.combat, refused.to_owned()),
                (Err(omitted), Some(refused)) => (omitted.combat, refused.to_owned()),
                (Err(omitted), None) => (omitted.combat, omitted.cause.clone()),
                (Ok(icon), None) => {
                    // A load-rebuilt engaged flag is this hour's limitation for every placed
                    // icon; the frame an unengaged icon keeps is its pool's
                    // ([`naval_windows`]).
                    if let Some(limit) = self.provenance {
                        report
                            .combat_limits
                            .entry(("Naval", limit.to_owned()))
                            .or_default()
                            .insert(icon.combat);
                    }
                    continue;
                }
            };
            report.unavailable_naval_combats += 1;
            naval_gap(report, cause, [combat]);
        }
    }
}

/// The current province of every unit a transfer names, found in the three
/// key-sorted unit censuses, or `None` while any census is incomplete. The spillover
/// pass looks up no other unit.
fn unit_locations(frame: &DecodedFrame, units: &[Units]) -> io::Result<Option<BTreeMap<u64, u32>>> {
    let divisions = rows::<tables::Divisions>(frame)?;
    let forces = rows::<tables::TaskForces>(frame)?;
    let guns = rows::<tables::RailwayGunCounterInputs>(frame)?;
    if !(divisions.is_complete() && forces.is_complete() && guns.is_complete()) {
        return Ok(None);
    }
    let (divisions, forces, guns) = (divisions.rows(), forces.rows(), guns.rows());
    let locate = |unit: u64| {
        if let Ok(index) = guns.binary_search_by_key(&unit, |row| row.gun) {
            return Some(guns[index].province);
        }
        if let Ok(index) = forces.binary_search_by_key(&unit, |row| row.id) {
            return Some(forces[index].province);
        }
        let index = divisions.binary_search_by_key(&unit, |row| row.id).ok()?;
        Some(divisions[index].province)
    };
    Ok(Some(
        units
            .iter()
            .flat_map(|row| &row.units)
            .filter_map(|&unit| Some((unit, locate(unit)?)))
            .collect(),
    ))
}

/// Draws the hour's naval combat icons and retained result icons.
pub(super) fn naval_markers(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    context: &MarkerContext<'_>,
    tables: &NavalMarkerTables,
    out: &mut CounterFrame,
) -> io::Result<()> {
    combat_icons(inputs, frame, context, tables, out)?;
    result_pass(inputs, tables, out);
    Ok(())
}

/// Naval-combat icons are shown strictly below their content distance cutoff
/// and only when the corresponding display flag permits the pass.
fn combat_pass(height: f32, defines: &CounterDefines) -> bool {
    visible(height, defines.naval_combat_cutoff)
}

/// Counts one undecided icon or icon part under `reason`.
fn gap<K: Ord + From<&'static str>>(gaps: &mut BTreeMap<K, usize>, reason: &'static str) {
    *gaps.entry(reason.into()).or_default() += 1;
}

/// The cause every placed icon is omitted for when the naval combat window the map places does
/// not resolve.
fn unconstructed(reason: &str) -> String {
    format!("icon omitted: {NAVAL_COMBAT_WINDOW} does not lay out: {reason}")
}

fn combat_icons(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    context: &MarkerContext<'_>,
    tables: &NavalMarkerTables,
    out: &mut CounterFrame,
) -> io::Result<()> {
    let windows = inputs.windows;
    let window = windows.naval_combat();
    let resources = gui_windows::gui_assets(inputs, None);
    let defines = &inputs.content.defines;
    // Off its cutoff, or in the raids map mode, the pass erases every icon: no pass is planned.
    if !inputs.visibility.province_icons || !inputs.passes(|height| combat_pass(height, defines)) {
        return Ok(());
    }
    // Resolve the map window's size in the GUI viewport. It determines every
    // icon's placement; failure omits every placed icon.
    let window_frame = match windows.naval_combat_frame(viewport(inputs.viewport, inputs.ui_scale))
    {
        Ok(window_frame) => window_frame,
        Err(reason) => {
            if let Some(combats) = &tables.combats {
                combats.report_omitted(Some(&unconstructed(&reason)), &mut out.report);
            }
            out.naval.combats_undecided = Some(unconstructed(&reason).into());
            return Ok(());
        }
    };
    let mut pass = naval_windows::CombatPass {
        icons: Vec::new(),
        gated: tables.gated.clone(),
        live: tables.live.clone(),
        live_undecided: tables.live_undecided.as_ref().map(|why| {
            format!("icon omitted: whether its combat resolves is undecided: {why}").into()
        }),
        hq: Ok(BTreeMap::new()),
        size: window_frame.size,
    };
    // An hour without a naval combat places nothing; every kept icon is then empty.
    let Some(combats) = &tables.combats else {
        out.naval.combat = Some(pass);
        return Ok(());
    };
    combats.report_omitted(None, &mut out.report);
    pass.hq = combats.hq.clone();
    // Counted with the placement causes already.
    for omitted in &combats.undecided {
        pass.icons.push(naval_windows::CombatIcon::omitted(
            (omitted.province, omitted.combat),
            None,
            omitted.rotates,
            &omitted.cause,
        ));
    }
    let height = window_frame.size[1] as f32;
    let details = rows::<tables::NavalCombatDetails>(frame)?.rows();
    let direct = rows::<tables::NavalProvinceCombats>(frame)?.rows();
    // The rosters the grids read, indexed the first time a large icon needs its grids, then
    // shared by every combat.
    let members = rows::<tables::NavalCombatShips>(frame)?;
    let ships = rows::<tables::Ships>(frame)?;
    let forces = rows::<tables::TaskForces>(frame)?;
    let inputs_rows = rows::<tables::TaskForceCounterInputs>(frame)?;
    let own_naval: OnceCell<NavalContext<'_>> = OnceCell::new();
    let mut grid_index: Option<grid::Index<'_>> = None;
    for icon in &combats.icons {
        let icon = match icon {
            Ok(icon) => icon,
            // The game places and updates it all the same: it keeps its identity, and what its
            // update writes is unknown (counted with its cause by `report_omitted`).
            Err(omitted) => {
                pass.icons.push(naval_windows::CombatIcon::omitted(
                    (omitted.province, omitted.combat),
                    omitted.place,
                    omitted.rotates,
                    &omitted.cause,
                ));
                continue;
            }
        };
        // Move the icon down by its ordinal times the window's
        // integer height.
        if occurrences::screen_y_offset(icon.ordinal, height).is_none() {
            let cause =
                "icon omitted: the naval combat window height is not a whole number of pixels";
            out.report.unavailable_naval_combats += 1;
            naval_gap(&mut out.report, cause.into(), [icon.combat]);
            pass.icons.push(naval_windows::CombatIcon::omitted(
                (icon.province, icon.combat),
                None,
                icon.rotates,
                cause,
            ));
            continue;
        }
        // A rotating icon of an engaged combat fills its progress: the small one when the
        // camera is coarse or two engaged combats share the combat province's vector, the large
        // one otherwise.
        let detail = &details[icon.detail];
        let engaged = icon.rotates && icon.decision.progress;
        let small = if !engaged || coarse(inputs) {
            Ok(true)
        } else {
            two_engaged(
                context.flags,
                icon.home,
                tables.vectors.get(&icon.home).map(|&index| &direct[index]),
                tables.direct_complete,
            )
        };
        // Which bar and how: undecided, the shown bar is the update's unknown part.
        let progress = || match (&small, context.flip(detail)) {
            (Ok(small), Ok(flip)) => Ok(gui_windows::CombatProgress {
                small: *small,
                progress: detail.progress,
                flip,
            }),
            (Err(cause), _) => Err(cause.clone()),
            (_, Err(cause)) => Err(cause),
        };
        let state = gui_windows::CombatState {
            rotation: icon.rotates.then(|| gui_windows::Rotation {
                engaged: icon.decision.progress.then(|| gui_windows::Engaged {
                    variant: icon.decision.variant,
                    progress: progress(),
                }),
            }),
            // Update the HQ frame after combat rotation for every live-combat icon.
            hq: icon.hq.clone(),
        };
        // A part it leaves unknown is counted where the icon would draw ([`naval_windows`]), for
        // as long as its window keeps it.
        let mut update = gui_windows::combat_calls(window, &state, &resources);
        update.wrote.push(naval_windows::PLACE);
        // A rotation clears the grids; the game fills an engaged large
        // icon's alongside its progress, a small icon never. Grids the hour cannot decide are the
        // update's unknown part.
        let grids = match small {
            _ if !engaged => Ok(Vec::new()),
            Err(cause) => Err(cause),
            Ok(true) => Ok(Vec::new()),
            Ok(false) => {
                let naval = match context.naval {
                    Ok(naval) => naval,
                    Err(sources) => match own_naval.get() {
                        Some(naval) => naval,
                        None => {
                            let built = NavalContext::build(frame, sources)?;
                            own_naval.get_or_init(|| built)
                        }
                    },
                };
                let index = grid_index.get_or_insert_with(|| {
                    grid::Index::new(grid::Sources {
                        // The forward members grouped once per hour with the HQ frames.
                        combat_ships: members
                            .is_complete()
                            .then(|| members.rows())
                            .zip(combats.forward.as_ref()),
                        // The census the naval context indexed for this hour, not another.
                        ships: ships.is_complete().then(|| grid::ShipCensus {
                            task_forces: Cow::Borrowed(&naval.ship_task_forces),
                            counts: Cow::Borrowed(&naval.ships),
                            rows: ships.rows().len(),
                        }),
                        task_forces: forces.is_complete().then_some(forces.rows()),
                        counter_inputs: inputs_rows.is_complete().then_some(inputs_rows.rows()),
                        aggression: Some(inputs.content.aggression.as_slice()),
                    })
                });
                let tiles = grid::derive(
                    index,
                    icon.combat,
                    context.vision,
                    &context.owner,
                    &context.countries.sides,
                )
                .and_then(|sides| Ok((combat_tiles(context, naval, &sides)?, sides)));
                tiles
                    .map(|tiles| grid_fills(inputs, context, window, detail, &tiles, out))
                    .map_err(|cause| cause.to_string())
            }
        };
        let grids = match grids {
            Ok(grids) => {
                if icon.rotates {
                    update.wrote.push(gui_windows::GRIDS);
                }
                grids
            }
            Err(cause) => {
                update.unknown.push((
                    gui_windows::GRIDS,
                    format!("icon omitted: its grids are undecided: {cause}"),
                ));
                Vec::new()
            }
        };
        pass.icons.push(naval_windows::CombatIcon {
            owner: (icon.province, icon.combat),
            place: Some((icon.anchor, icon.ordinal)),
            update,
            rotates: icon.rotates,
            grids,
            frame: Some(icon.decision.frame),
            omitted: false,
        });
    }
    out.naval.combat = Some(pass);
    Ok(())
}

/// The screen rectangle `bg_btn` covers as drawn in `layout` (what a click hits): its quad's
/// footprint, relative to the window's origin.
#[cfg(test)]
pub(super) fn background_footprint(
    window: &native::Element,
    layout: &native_gui::Layout,
) -> Option<[f32; 4]> {
    // Find combat-window controls through local typed lookup.
    let background = window.find(native::Kind::Button, "bg_btn")?;
    layout
        .parts
        .iter()
        .filter(|part| part.path == background.path)
        .find_map(|part| match part.kind {
            native_gui::PartKind::Sprite { geometry, .. } => Some(footprint(&geometry)),
            _ => None,
        })
}

/// The game binds the tooltip handler to both buttons and both progress
/// icons, but the click observer only to bg_btn/frame_btn. Keep the actual
/// visible controls separate: the progress icon alone does not open a battle.
pub(super) fn combat_footprints(
    window: &native::Element,
    layout: &native_gui::Layout,
    origin: [i32; 2],
    resources: &dyn native_gui::Resources,
    notes: &mut BTreeMap<String, usize>,
) -> Vec<(HitRectangle, bool)> {
    let mut hits: Vec<_> = [
        (native::Kind::Button, "bg_btn", true),
        (native::Kind::Button, "frame_btn", true),
        (native::Kind::Icon, "progress", false),
        (native::Kind::Icon, "progress_small", false),
    ]
    .into_iter()
    .filter_map(|(kind, name, clickable)| {
        let control = window.find(kind, name)?;
        let hit = hit_rectangle(control, layout, origin, resources, notes)?;
        Some((hit, clickable))
    })
    .collect();
    hits.sort_by_key(|(hit, _)| hit.order);
    hits
}

/// Append attacker task-force tiles in allegiance runs, then its convoy
/// addition when present, followed by the corresponding defender items.
fn grid_fills(
    inputs: &CounterInputs<'_>,
    context: &MarkerContext<'_>,
    window: &native::Element,
    detail: &NavalCombatDetailsRow,
    (tiles, sides): &([Vec<Flotilla>; 2], [grid::SideGrid<'_>; 2]),
    out: &mut CounterFrame,
) -> Vec<naval_windows::GridFill> {
    let mut fills = Vec::new();
    let navy = inputs.windows.navy();
    let art = gui_windows::CounterArt {
        assets: inputs.assets,
        countries: context.countries,
    };
    let resources = gui_windows::gui_assets(inputs, None);
    let ctx = gui_windows::NavyContext {
        settings: inputs.settings,
        selected: inputs.selected,
        defines: &inputs.content.defines,
        art: &art,
        gui: &resources,
    };
    for (side, roles) in [
        [GridRole::Tiles(0), GridRole::Adds(0)],
        [GridRole::Tiles(1), GridRole::Adds(1)],
    ]
    .into_iter()
    .enumerate()
    {
        for role in roles {
            let Some(name) = GRID_ROLES
                .iter()
                .find(|(_, own)| *own == role)
                .map(|(name, _)| *name)
            else {
                continue;
            };
            let Some(element) = window.find(native::Kind::Grid, name) else {
                continue;
            };
            match role {
                GridRole::Tiles(_) => {
                    let items = tiles[side]
                        .iter()
                        .map(|tile| {
                            Ok(gui_windows::navy_calls(
                                &ctx,
                                navy,
                                &gui_windows::NavyState {
                                    flotilla: tile,
                                    countries: context.countries,
                                },
                                &mut out.report.navy_counter_gaps,
                            ))
                        })
                        .collect();
                    fills.push(naval_windows::GridFill {
                        grid: element.path.clone(),
                        side: Some(side),
                        items,
                    });
                }
                GridRole::Adds(_) => {
                    let convoys = sides[side].convoys;
                    if convoys == 0 {
                        continue;
                    }
                    let countries = &detail.sides[side].summary.countries;
                    let name = if side == 0 { "attacking" } else { "defending" };
                    let item = context
                        .side_allegiance(name, countries)
                        .and_then(|allegiance| context.convoy_tint(inputs, allegiance, countries))
                        .map(|tint| {
                            gui_windows::Update::calls(match inputs.windows.tiny() {
                                Some(tiny) => gui_windows::convoy_calls(
                                    &inputs.assets.gui,
                                    tiny,
                                    convoys,
                                    tint,
                                    &mut out.report.naval_grid_gaps,
                                ),
                                None => Vec::new(),
                            })
                        })
                        .map_err(|reason| {
                            Cow::Owned(format!("a convoy addition is not drawn: {reason}"))
                        });
                    fills.push(naval_windows::GridFill {
                        grid: element.path.clone(),
                        side: None,
                        items: vec![item],
                    });
                }
            }
        }
    }
    fills
}

/// The screen rectangle a drawn quad covers: its turned corners' bounds.
#[cfg(test)]
fn footprint(geometry: &native_gui::Geometry) -> [f32; 4] {
    let corners = geometry.corners();
    let [mut x0, mut y0] = corners[0];
    let [mut x1, mut y1] = corners[0];
    for [x, y] in corners {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    [x0, y0, x1 - x0, y1 - y0]
}

/// Prepared naval-combat grid items: side convoy additions and task-force tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GridRole {
    Adds(usize),
    Tiles(usize),
}

const GRID_ROLES: [(&str, GridRole); 4] = [
    ("attacker_adds_grid", GridRole::Adds(0)),
    ("defender_adds_grid", GridRole::Adds(1)),
    ("attacker_grid", GridRole::Tiles(0)),
    ("defender_grid", GridRole::Tiles(1)),
];

/// Count each convoy addition or task-force tile beyond its grid's slot limits,
/// which the game does not add.
pub(super) const GRID_FULL: &str =
    "a naval combat grid item is past its grid's slot limits, which do not add it; not drawn";

/// Each grid group becomes a mission-list unit item, built like a port item from
/// the first member's visibility byte. An unresolved expeditionary owner refuses
/// the grids.
fn combat_tiles(
    context: &MarkerContext<'_>,
    naval: &NavalContext<'_>,
    sides: &[grid::SideGrid<'_>; 2],
) -> Result<[Vec<Flotilla>; 2], &'static str> {
    let mut tiles = [Vec::new(), Vec::new()];
    for (side, grid) in sides.iter().enumerate() {
        for group in &grid.groups {
            let units = group
                .task_forces
                .iter()
                .map(|member| {
                    let force = member.force;
                    Ok(NavalUnit {
                        force,
                        ordinal: 0,
                        slot: member.country,
                        logical: (context.owner)(force.logical_owner),
                        // the expeditionary owner when positive.
                        flag_owner: if (force.expeditionary_owner as i32) > 0 {
                            (context.owner)(force.expeditionary_owner)
                                .ok_or("a grid task force's expeditionary owner is unresolved")?
                        } else {
                            member.country
                        },
                        sight: if member.visibility >= 250 {
                            Sight::Full
                        } else {
                            Sight::Partial
                        },
                        byte: member.visibility,
                        ships: member.ships,
                        position: None,
                        // The mixed-owner flag rule reads each task force's summed strength
                        // As a port item's does.
                        strength: naval
                            .ships
                            .get(&force.id)
                            .map_or(0, |&(_, strength)| strength),
                    })
                })
                .collect::<Result<Vec<_>, &'static str>>()?;
            // `derive` starts a group only with a task force.
            tiles[side].push(naval.flotilla(&units.iter().collect::<Vec<_>>(), true, false));
        }
    }
    Ok(tiles)
}

/// When the content does not define [`CONVOY_ADDITION_ICON`], keep and draw
/// the icon's authored sprite.
pub(super) const CONVOY_ICON_GAP: &str = "a convoy addition's GFX_unit_transport_icon_medium_white is not defined; drawn with the icon's authored sprite";

/// Whether `list` of result `result` holds the viewer; `None` for an unresolved reference.
fn result_list_contains(
    viewer: Viewer<'_>,
    countries: &BTreeMap<(u64, u8), Vec<u32>>,
    result: u64,
    list: u8,
) -> Option<bool> {
    viewer.contains_viewer(countries.get(&(result, list)).map_or(&[], Vec::as_slice))
}

/// One retained naval result as read by the result icon for the viewing country.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ResultFacts {
    /// Whether country lists 0, 1 and 2 hold the viewer; `None` for an unresolved reference.
    lists: [Option<bool>; 3],
    /// Whether this is a port strike.
    flag_a: bool,
    /// Whether this is a naval strike.
    flag_b: bool,
    /// Whether this is a convoy battle.
    subtype: bool,
    /// Whether the result uses the neutral readiness frame.
    readiness: bool,
    winning_side: u8,
    losing_side: u8,
    /// A member with a copied sinking string.
    present: bool,
}

impl ResultFacts {
    /// the viewer is admitted (list 2), is
    /// on a side (list 0 or 1), and the result is a strike or has a present member; `None` when a
    /// list it reaches is unresolved.
    fn visible(&self) -> Option<bool> {
        if !self.lists[2]? {
            return Some(false);
        }
        if !(self.lists[0]? || self.lists[1]?) {
            return Some(false);
        }
        Some(self.flag_a || self.flag_b || self.present)
    }

    /// which the game asks of each result
    /// its icon holds: list 2 holds the viewer.
    pub(super) fn admitted(&self) -> Option<bool> {
        self.lists[2]
    }
}

/// Select the sprite and zero-based frame from the icon's still-resolving `results`.
/// Expired references are skipped but remain in `held`, the convoy-majority
/// denominator. All naval strikes select that sprite, then all port strikes;
/// otherwise use a plain battle unless more than half of `held` are convoy battles.
/// The frame is 2 when every resolving result has its readiness flag; otherwise
/// it is lost 0, won 1 or mixed 2. A result where the viewer is only on the losing
/// side clears "won", and one where it is only on the winning side clears "lost".
/// With none resolving, every test holds vacuously. `Err` names the unresolved country.
pub(super) fn result_presentation(
    results: &[ResultFacts],
    held: usize,
) -> Result<(&'static str, u32), &'static str> {
    let sprite = if results.iter().all(|result| result.flag_b) {
        NAVAL_RESULT_SPRITES[0]
    } else if results.iter().all(|result| result.flag_a) {
        NAVAL_RESULT_SPRITES[1]
    } else if results.iter().filter(|result| result.subtype).count() <= held / 2 {
        NAVAL_RESULT_SPRITES[2]
    } else {
        NAVAL_RESULT_SPRITES[3]
    };
    let frame = if results.iter().all(|result| result.readiness) {
        2
    } else {
        let (mut won, mut lost) = (true, true);
        for result in results {
            let side = |selector: u8| match selector {
                1 | 2 => result.lists[usize::from(selector - 1)],
                _ => Some(false),
            };
            match (side(result.winning_side), side(result.losing_side)) {
                (Some(winner), Some(loser)) if winner != loser => {
                    if winner {
                        lost = false;
                    } else {
                        won = false;
                    }
                }
                (Some(_), Some(_)) => {}
                _ => return Err("a retained naval result's country is unresolved"),
            }
        }
        match (won, lost) {
            (false, true) => 0,
            (true, false) => 1,
            _ => 2,
        }
    };
    Ok((sprite, frame))
}

/// An ordinary convoy battle the viewer won, for other modules' tests.
#[cfg(test)]
pub(super) fn convoy_result_for_test() -> ResultFacts {
    ResultFacts {
        lists: [Some(true), Some(false), Some(true)],
        subtype: true,
        winning_side: 1,
        losing_side: 2,
        present: true,
        ..Default::default()
    }
}

/// The hour's retained naval results, decided once
/// per hour and perspective.
pub(super) struct ResultTables {
    /// Each province holding a result, its results in bucket order (`province_ordinal`).
    pub(super) provinces: BTreeMap<u32, Vec<u64>>,
    /// Every result's facts for the viewer.
    pub(super) facts: BTreeMap<u64, ResultFacts>,
    /// Why the tables are not whole (`NavalResults`, `NavalResultMembers`,
    /// `NavalResultCountries`): a missing row could change which results a province shows, and
    /// every result icon's decision is then undecided.
    pub(super) undecided: Option<&'static str>,
}

impl ResultTables {
    /// Results admitted for `province` in bucket order; `Err` when one is undecided.
    pub(super) fn visible(&self, province: u32) -> Result<Vec<u64>, &'static str> {
        if let Some(why) = self.undecided {
            return Err(why);
        }
        let mut visible = Vec::new();
        for id in self.provinces.get(&province).into_iter().flatten() {
            match self.facts.get(id).and_then(ResultFacts::visible) {
                Some(true) => visible.push(*id),
                Some(false) => {}
                None => return Err("a retained naval result's country is unresolved"),
            }
        }
        Ok(visible)
    }
}

/// The retained results by province, with their facts for the viewer.
pub(super) fn result_tables(frame: &DecodedFrame, viewer: Viewer<'_>) -> io::Result<ResultTables> {
    let results = rows::<tables::NavalResults>(frame)?;
    let members = rows::<tables::NavalResultMembers>(frame)?;
    let country_rows = rows::<tables::NavalResultCountries>(frame)?;
    let mut countries: BTreeMap<(u64, u8), Vec<(u32, u32)>> = BTreeMap::new();
    for row in country_rows.rows() {
        countries
            .entry((row.result, row.list))
            .or_default()
            .push((row.ordinal, row.country));
    }
    let countries: BTreeMap<(u64, u8), Vec<u32>> = countries
        .into_iter()
        .map(|(key, mut list)| {
            list.sort_unstable();
            (key, list.into_iter().map(|(_, country)| country).collect())
        })
        .collect();
    let present: BTreeSet<u64> = members
        .rows()
        .iter()
        .filter(|member| member.presence_value != 0)
        .map(|member| member.result)
        .collect();
    let mut by_province: BTreeMap<u32, Vec<&NavalResultRow>> = BTreeMap::new();
    for result in results.rows() {
        by_province.entry(result.province).or_default().push(result);
    }
    let facts = results
        .rows()
        .iter()
        .map(|result| {
            let list = |list| result_list_contains(viewer, &countries, result.id, list);
            (
                result.id,
                ResultFacts {
                    lists: [list(0), list(1), list(2)],
                    flag_a: result.flag_a != 0,
                    flag_b: result.flag_b != 0,
                    subtype: result.subtype != 0,
                    readiness: result.readiness != 0,
                    winning_side: result.winning_side,
                    losing_side: result.losing_side,
                    present: present.contains(&result.id),
                },
            )
        })
        .collect();
    Ok(ResultTables {
        provinces: by_province
            .into_iter()
            .map(|(province, mut results)| {
                results.sort_unstable_by_key(|result| result.province_ordinal);
                (province, results.iter().map(|result| result.id).collect())
            })
            .collect(),
        facts,
        undecided: (!(results.is_complete()
            && members.is_complete()
            && country_rows.is_complete()))
        .then_some("the retained naval results or their members or countries are incomplete"),
    })
}

/// The result window's refusal every shown result icon is omitted for: the game finds
/// `bg_btn` as a button, and the game fails without one.
const REFUSED_RESULT_WINDOW: &str =
    "the naval result window or its buttonType bg_btn is refused or not declared";

/// Result icons use the province's applicable land or sea anchor.
pub(super) fn result_anchor(
    inputs: &CounterInputs<'_>,
    raised: &BTreeSet<u32>,
    whole: bool,
    province: u32,
) -> Result<[f32; 3], &'static str> {
    match inputs.content.province_kind(province) {
        Some(ProvinceKind::Land) => inputs
            .content
            .anchor(province, SHIP_IN_PORT)
            .ok_or("the content places no unit-stack slot 19 in the result's land province"),
        Some(_) => {
            let raised = raised.contains(&province);
            if !raised && !whole {
                return Err("a sea province's combat vector is unknown");
            }
            inputs
                .content
                .anchor(province, STANDSTILL)
                .map(|mut anchor| {
                    if raised {
                        anchor[2] += 5.0;
                    }
                    anchor
                })
                .ok_or("the content places no Standstill anchor in the result's province")
        }
        None => Err("the map does not define the result's province"),
    }
}

/// Every result icon a content without a usable result window omits, counted by that refusal:
/// each province whose visible results the hour holds or cannot decide (a hidden icon is not
/// counted).
fn refused_results(tables: &ResultTables, report: &mut CounterReport) {
    for &province in tables.provinces.keys() {
        if tables
            .visible(province)
            .is_ok_and(|visible| visible.is_empty())
        {
            continue;
        }
        report.unavailable_naval_results += 1;
        gap(&mut report.naval_result_gaps, REFUSED_RESULT_WINDOW);
    }
}

/// The naval result icons' pass for this camera ([`naval_windows::ResultPass`]): the hour's
/// result tables with their completeness, the combat vectors the anchors read, and the window's
/// frame. An hour whose result census is complete and empty plans no pass (every icon is then
/// empty); an incomplete one plans one even when it holds no row, so an icon the census may lack
/// stays undecided rather than known empty. The owner and the manager's pass run
/// on the presented frame. A content with no result window places none, counted.
fn result_pass(inputs: &CounterInputs<'_>, tables: &NavalMarkerTables, out: &mut CounterFrame) {
    // The raids map mode clears naval combat result icons with the other province icons.
    if !inputs.visibility.province_icons
        || tables.results.provinces.is_empty() && tables.results.undecided.is_none()
    {
        return;
    }
    let declared = inputs
        .windows
        .naval_result()
        .is_some_and(|window| window.find(native::Kind::Button, "bg_btn").is_some());
    if !declared {
        refused_results(&tables.results, &mut out.report);
        return;
    }
    let size = match inputs
        .windows
        .naval_result_frame(viewport(inputs.viewport, inputs.ui_scale))
    {
        None => {
            refused_results(&tables.results, &mut out.report);
            return;
        }
        Some(Err(reason)) => {
            *out.report
                .naval_window_notes
                .entry(format!("the naval result icons are not placed: {reason}").into())
                .or_default() += 1;
            out.naval.results_undecided = Some(reason.into());
            return;
        }
        Some(Ok(frame)) => frame.size,
    };
    out.naval.results = Some(naval_windows::ResultPass {
        tables: Arc::clone(&tables.results),
        raised: tables.vectors.keys().copied().collect(),
        vectors_whole: tables.direct_complete,
        size,
    });
}

#[cfg(test)]
mod tests;
