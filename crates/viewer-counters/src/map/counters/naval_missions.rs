//! Naval mission map icons for strategic regions with registered missions.
//!
//! Entries are created in mission-kind order from the resolved mission sprites.
//! Each entry retains its counters, status and controls; supported visibility and
//! membership decisions use current-hour recorded inputs. Refused reached operands
//! withhold the affected fragment with a named count.

use super::naval_windows::{Mark, mark, marks};
use super::*;
use std::collections::HashMap;
use viewer_foundation::map::gui_resources::GuiAssets;
use viewer_foundation::map::gui_resources::count_noted;
use viewer_foundation::map::gui_resources::count_refused;
use viewer_foundation::map::gui_resources::viewport as gui_resources_viewport;
use viewer_foundation::map::icon_windows::POOL_EXHAUSTED;
use viewer_foundation::map::icon_windows::Retry;
use viewer_foundation::map::native_gui::{
    self,
    lifecycle::{
        Admission, Held, Instance, MapIconPool, Pool, Rewrote, SlotId, Tick, Unknown, Withheld,
    },
};
use viewer_foundation::map::native_text::COLOR;
use viewer_foundation::map::native_windows::NativeWindows;
use viewer_foundation::map::province_schedule::{Schedule, Update};

/// Sprite lookup keys by mission kind; kinds without a key have no entry.
/// Resolve each key through the content's sprite registry. An undefined name
/// keeps the authored sprite, which is drawn while the missing name is counted.
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

/// Registered mission kinds that this display neither counts nor shows.
const UNCOUNTED: [u32; 3] = [0, 8, 10];

pub const MISSION_ENTRY_WINDOW: &str = "naval_mission_mapicon_entry";
pub const MISSION_WINDOW: &str = "naval_mission_mapicon";
/// The window the game moves.
const CONTENT_WINDOW: &str = "content";
/// Mission content container name.
const MISSIONS_LIST: &str = "missions_list";

/// Mission UI mode selected by the update.
const DRAWN_UI_MODE: i32 = 0;

/// The recorded range/convoy operands or content needed for this child are incomplete.
const P2_ACTIVE: &str = "P2 active registrations need complete native range operands";
const P3_RAIDING: &str = "P3 raiding efficiency needs complete native operands";
const P3_ESCORT: &str = "P3 escort efficiency needs complete native operands";
const P3_ESCORT_ALERT: &str = "P3 escort alert needs complete efficiency and presence operands";

/// Why a registration's pride of the fleet is
/// undecided this hour: its roster owner has no recorded country.
const PRIDE_OWNER_UNLINKED: &str = "a registered task force's roster owner is not linked to a \
     recorded country";
/// The country's pride of the fleet (`CountryCounterInputs`) is not recorded.
const PRIDE_SHIP_UNRECORDED: &str = "a registered task force's country has no recorded pride of \
     the fleet";
/// The pride ship is absent from an incomplete Ships census.
const PRIDE_SHIPS_INCOMPLETE: &str =
    "the pride of the fleet is absent from an incomplete Ships census";
pub(super) const ENGAGEMENT_UNKNOWN: &str =
    "a registered task force has no TaskForceCounterInputs engagement rule";

/// An entry's pride marker (`pride`), which the game shows or hides from
/// its record's registrations: left unknown when they are undecided, and known again only by a
/// decided write.
const PRIDE: &str = "its pride marker";
/// An entry's count text, which the update writes only while the entry is listed.
const COUNT: &str = "its count";
/// Mission entry display inputs retained by the update.
const ENTRY_PLACE: &str = "its place in the missions list";
/// The parts of an entry an update of its icon may leave as they were: an entry the update does
/// not list keeps its count and place.
const ENTRY_KEPT: [&str; 2] = [COUNT, ENTRY_PLACE];
/// Retained mission-list position in the icon.
const LIST_PLACE: &str = "its missions list's place";

/// Naval map mode, UI mode 2: the only mode that counts the units shown by
/// `content`'s containers.
const NAVAL_UI_MODE: i32 = 2;

/// Counted mission containers reset their counts on update. Own and enemy
/// mine values are counted only in naval UI mode and at the detailed camera.
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

/// Resolve the mission root, its content container, list and entry template
/// through their local authored hierarchy.
pub(super) struct MissionWindows<'a> {
    pub(super) root: &'a native::Element,
    content: &'a native::Element,
    list: &'a native::Element,
    pub(super) entry: &'a native::Element,
}

impl<'a> MissionWindows<'a> {
    pub(super) fn find(windows: &'a NativeWindows) -> Result<Self, native_gui::Refusal> {
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
            // Fixed-window entries are incompatible with this visibility walk: it treats
            // each entry as a container and reads control arrays from uninitialised
            // callback storage.
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

/// One shown entry: its kind's counts and the children the recorded inputs decide.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct MissionEntry {
    pub(super) kind: u32,
    pub(super) total: u32,
    /// Active registrations, where the kind counts them (1, 2 and 9); the input it waits for when
    /// unknown.
    pub(super) active: Result<Option<u32>, &'static str>,
    /// Whether this kind carries the pride marker; why not, when unknown.
    pub(super) pride: Result<bool, &'static str>,
    /// The input it waits for when unknown.
    pub(super) alert: Result<bool, &'static str>,
    pub(super) engagement: Result<bool, &'static str>,
    pub(super) efficiency: Option<Result<i64, &'static str>>,
}

/// A ledger registration: its mission row, its kind and whether its task force holds the pride
/// of the fleet (why not, when unknown).
type Registration = (u64, u32, u32, Result<bool, &'static str>);

/// One hour's mission icons for the viewing country, by strategic region.
#[derive(Clone, Debug, Default)]
pub struct MissionIcons {
    pub(super) hovers: Arc<super::naval_mission_inputs::hover::Hovers>,
    pub(super) convoy_hovers: Arc<BTreeMap<u64, super::naval_mission_inputs::hover::Prepared>>,
    pub(super) naval_dominance_sources:
        Option<Arc<viewer_foundation::map::naval_dominance_sources::RegionalDominanceSourcesMap>>,
    pub(super) regions: BTreeMap<u32, Vec<MissionEntry>>,
    pub(super) input_gaps: BTreeMap<String, usize>,
    /// Why no icon is decided this hour, when none is.
    unavailable: Option<&'static str>,
}

impl MissionIcons {
    /// The recorded viewing navy's ledger, including repeats and regions
    /// outside the accessible lists.
    pub(super) fn read(
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
        // No current mission region is represented by zero.
        let missions: BTreeMap<_, _> = missions
            .rows()
            .iter()
            .map(|row| (row.task_force, row))
            .collect();
        // Index only the pride references requested by registered task forces.
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

/// Mission entries for one region's registrations, in kind order.
fn entries(
    region: u32,
    registrations: &[Registration],
    native: &NativeInputs,
) -> Vec<MissionEntry> {
    let mut totals: BTreeMap<u32, u32> = BTreeMap::new();
    let mut patrolling = 0;
    // every counted pride registration replaces the chosen kind.
    let mut last_pride: Result<Option<u32>, &'static str> = Ok(None);
    let mut no_engagement: BTreeMap<u32, Result<bool, &'static str>> = BTreeMap::new();
    for (handle, current_region, kind, pride) in registrations {
        if UNCOUNTED.contains(kind) {
            continue;
        }
        *totals.entry(*kind).or_default() += 1;
        if *kind == viewer_foundation::map::unit_rules::PATROL && *current_region == region {
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
                // Mission activity and efficiency are independent reached terms.
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

/// Count text uses the total, or active over total for mission kinds with
/// an activity count.
fn count_text(entry: &MissionEntry) -> Result<(String, bool), &'static str> {
    match entry.active? {
        Some(active) if active != entry.total => {
            Ok((format!("{active}/{}", entry.total), active == 0))
        }
        _ => Ok((entry.total.to_string(), false)),
    }
}

impl CounterReport {
    /// What the hour's naval mission icons could not draw, for the warning area.
    pub fn naval_mission_warnings(&self) -> Vec<String> {
        let mut warnings: Vec<String> = self
            .naval_mission_limits
            .iter()
            .map(|reason| format!("Naval mission icons are not drawn this hour: {reason}."))
            .collect();
        warnings.extend(
            self.undecided_naval_mission_children
                .iter()
                .map(|(child, count)| format!("{count} x a naval mission entry hides {child}.")),
        );
        warnings.extend(
            self.naval_mission_gaps
                .iter()
                .map(|(reason, count)| format!("{count} x {reason}.")),
        );
        if self.unanchored_naval_missions != 0 {
            warnings.push(format!(
                "{} naval mission icons are not drawn: their strategic region has no icon position.",
                self.unanchored_naval_missions
            ));
        }
        if self.authored_naval_mission_buttons != 0 {
            warnings.push(format!(
                "{} naval mission entries draw their button's authored sprite: the content defines no sprite {}.",
                self.authored_naval_mission_buttons,
                self.undefined_naval_mission_sprites
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        warnings
    }
}

/// Records a sprite the content does not define.
fn undefined(report: &mut CounterReport, sprite: &str) {
    if !report.undefined_naval_mission_sprites.contains(sprite) {
        report
            .undefined_naval_mission_sprites
            .insert(sprite.to_owned());
    }
}

/// Prepare one registered mission kind's display decision.
#[derive(Clone, Debug, PartialEq)]
struct Record<'e> {
    /// The kind's shown entry when the region has registrations of that kind.
    listed: Option<&'e MissionEntry>,
    /// `Ok(shown)`, or the P2/P3 input it waits for (hidden and counted, [`undecided`]).
    alert: Result<bool, &'static str>,
    /// `Ok(shown)`, or why its registrations do not decide it (an unknown part, [`PRIDE`]).
    pride: Result<bool, &'static str>,
    /// `None` hidden (the value `-100000`); `Some(Err(input))` waiting for a P3 input (hidden and
    /// counted); `Some(Ok(text))` shown with `text`.
    efficiency: Option<Result<String, &'static str>>,
}

/// Format the percentage with zero decimal precision. Values below 75000
/// receive the red colour control, followed by reset.
fn efficiency_text(value: i64) -> String {
    let text = installed_content_core::fixed5::fixed5_percent(value.clamp(0, 100_000), 0, true);
    let code = COLOR;
    if value < 75_000 {
        format!("{code}R{text}{code}!")
    } else if value < 100_000 {
        format!("{code}Y{text}{code}!")
    } else {
        text
    }
}

/// The record of `kind` this frame, for a region whose shown entries are `entries`.
fn record(kind: u32, entries: &[MissionEntry]) -> Record<'_> {
    match entries.iter().find(|entry| entry.kind == kind) {
        Some(entry) => Record {
            listed: (entry.total != 0).then_some(entry),
            alert: entry.alert,
            pride: entry.pride,
            // Convoy raiding and escort
            // compute their efficiency from the recorded inputs (P3).
            efficiency: entry.efficiency.and_then(|value| match value {
                Ok(-100_000) => None,
                other => Some(other.map(efficiency_text)),
            }),
        },
        // No registration of the kind: every test runs over none.
        None => Record {
            listed: None,
            alert: match kind {
                // No patrol is registered in the region.
                1 => Ok(true),
                // No kind-3 registration has engagement rule 0, and the game's average
                // over no registration is 0, below 100000.
                3 => Ok(true),
                // Read the escort's inputs (P3).
                4 => Err(P3_ESCORT_ALERT),
                // Kind 2: `0 < 0` fails and no registration has engagement rule 0; kind 9
                // alike; the other kinds have no alert.
                _ => Ok(false),
            },
            // `value` is always the kind of a counted registration.
            pride: Ok(false),
            efficiency: match kind {
                3 => Some(Ok(efficiency_text(0))),
                4 => Some(Err(P3_ESCORT)),
                _ => None,
            },
        },
    }
}

/// The children of a drawn entry of `kind` (its sprite `sprite`) its record hides for a P2 or P3
/// input no recording holds yet (the count, the alert and the efficiency), each named by child,
/// kind and input.
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

/// Showing a child clears its hide bit before GUI Show, reaching a container's
/// descendants; hiding calls Hide before setting that bit.
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

/// Every resource the mission windows draw, found as construction finds them
/// ([`native_gui::assets_in`] over the root and the entry window its list makes), and the
/// executable's kind sprites the content's registry defines: `sprite assignment` replaces the
/// button's sprite only with a defined one. The texts the code binds (counts and efficiencies:
/// digits, `/`, `%` and the colour codes) hold no inline icon, so they add nothing.
pub(super) fn mission_assets(
    windows: &NativeWindows,
    registry: &native::resources::Registry,
    localisation: &installed_content_localisation::localisation::Localisation,
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

/// The mission icons' part of a prepared hour: what the code decides, made without any window.
#[derive(Debug, Default)]
pub(super) struct MissionPass {
    pub(super) hovers: Arc<super::naval_mission_inputs::hover::Hovers>,
    pub(super) convoy_hovers: Arc<BTreeMap<u64, super::naval_mission_inputs::hover::Prepared>>,
    /// The regions whose ledger is not empty, with their shown entries;
    /// `None` at or above the cutoff, where the game passes
    /// the gate 0 and every icon is erased.
    pub(super) regions: Option<BTreeMap<u32, Vec<MissionEntry>>>,
    /// Why the pass runs below the cutoff but cannot be planned (an incomplete census): every
    /// icon is erased, which the game does not do ([`MissionView::diverged`]).
    pub(super) undecided: Option<&'static str>,
    pub(super) mode: i32,
    pub(super) regional: BTreeMap<u32, naval_regional::Region>,
}

/// The hour's mission pass: the regions the game admits below the cutoff, or none at or
/// above it; what the hour cannot decide is counted.
pub(super) fn naval_missions(
    inputs: &CounterInputs<'_>,
    icons: &MissionIcons,
    out: &mut CounterFrame,
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
    // The air and raids map modes draw no naval mission icons.
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

/// One pooled icon's entries: the windows the game built for it, one per
/// record, and what its list holds.
#[derive(Debug)]
struct PooledIcon {
    /// By record, in [`MISSION_SPRITES`] order.
    entries: Vec<Instance>,
    /// Record indices in append order. Show and hide propagation reaches every
    /// retained entry in this list.
    listed: Vec<usize>,
    /// Retain entry registration order independently of the visible list. Appended
    /// entries move to the end of paint order; clearing a list does not detach old
    /// entries, so retained hidden entries still participate in lifecycle calls.
    attached: Vec<usize>,
    count_unknown: Vec<Option<&'static str>>,
    held_grids: Vec<Held>,
}

/// The mission icons' windows of one map view: the type-12 pool keyed by strategic region
/// and each pooled icon's entries, built from one registry in one GUI
/// viewport. A refused update withholds its windows through the module's one refusal state: an
/// icon's in its pool's slot ([`Pool::refuse`]), an entry's in [`Self::entries_withheld`].
#[derive(Debug, Default)]
pub(super) struct MissionView {
    built: Option<(NativeWindows, [i32; 2])>,
    pool: Option<Pool<u32>>,
    icons: HashMap<SlotId, PooledIcon>,
    /// The entries whose places an update left unknown ([`Placement::Unknown`]), by pooled slot
    /// and record: the entry windows are the pooled icon's, so
    /// the state stays with the slot, live or free, as the windows do.
    entries_withheld: Withheld<(SlotId, usize)>,
    /// Why the pool's slots may no longer be the game's: a request or erasure the view cannot
    /// reproduce (a pass it cannot plan, a region with no decided icon place never requested, the
    /// frame a growth completes), after which the game's round robin may give a
    /// region another slot than the game's, with another history. Every slot allocated
    /// afterwards leaves the parts an update may keep unknown ([`Self::allocated`]); kept until
    /// the windows are built again.
    diverged: Option<Cow<'static, str>>,
    /// The windows whose fade and movement an update left unknown, with why, for the view's one
    /// mechanism (`naval_windows::transition_unknown`), which takes them at the frame's tick.
    unsettled: Vec<(MissionWindow, String)>,
    /// A live icon's region is reached this frame only through provinces out of view whose
    /// presentation-dirty bit keeps them in the update list: the view presents frames until the
    /// schedule's phase clears them.
    kept_off_screen: bool,
    pub(super) updated_grids: Vec<(u32, SlotId)>,
    pub(super) mode: i32,
    pub(super) badge_counts: BTreeMap<u32, [Result<usize, String>; 2]>,
    badge_data: BTreeMap<u32, naval_mine_badges::Packet>,
}

/// A mission-view window in the GUI transition list: a pooled icon's root,
/// or one of its entries by record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum MissionWindow {
    Icon(SlotId),
    Entry(SlotId, usize),
}

/// One region reached by the frame's province schedule. Its gate determines
/// entries and icon placement; unreached regions are swept separately.
#[derive(Clone, Debug)]
struct Reached<'p> {
    region: u32,
    entries: Option<&'p [MissionEntry]>,
    point: [i32; 2],
}

/// A live icon's layout: nothing drawn, or the refusals of the module.
type Laid = Result<Option<LaidIcon>, native_gui::Refusals>;

/// One region's icon as the frame lays it out: the root's layout, the entries each holder of it
/// holds, placed, and how many entries were laid out.
struct LaidIcon {
    slot: SlotId,
    layout: native_gui::Layout,
    items: Vec<Vec<native_gui::Part>>,
    entries: usize,
    hits: Vec<(u32, super::naval_mission_inputs::hover::Control, [f32; 4])>,
}

/// Construct one retained entry window per mission record, assigning the
/// resolved kind sprite in the existing update order.
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

/// Apply shown or hidden state to every retained list entry as the list receives
/// its lifecycle call, preserving registration order.
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

/// A refusal of `path` of `kind`, for a reason the module gives as text.
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

/// A refusal of `element` for a reason the lifecycle gives as text.
fn refused(element: &native::Element) -> impl Fn(String) -> native_gui::Refusals + '_ {
    move |why| {
        vec![refusal(
            &element.path,
            native_gui::RefusalKind::MissingInput,
            why,
        )]
    }
}

/// Update content in call order: hide unused counted containers, set their
/// hide bits, empty every grid, then append the mode's prepared items.
/// Grid items remain governed by the current update's membership.
fn content_calls(
    windows: &MissionWindows<'_>,
    mode: i32,
    root: &mut Instance,
) -> Result<(), native_gui::Refusals> {
    // Grid membership is supplied independently by the regional preparation.
    // These controls keep their authored backgrounds while every update clears items.
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

/// Apply the mission update after common icon placement and visibility,
/// preserving child text, count, frame, grid and visibility call order.
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
        let mut update = gui_windows::Update::default();
        if let Some(listed) = record.listed {
            if let Some(button) = button {
                window
                    .set_frame(entry, button, 2)
                    .map_err(refused(button))?;
            }
            if let Some(count) = count {
                // Write the count text through its bound control.
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
            // Leave an unlisted entry's count untouched. A last listing
            // that hid an undecidable P2 count cannot supply its fade-out text (F3).
            update
                .unknown
                .push((COUNT, format!("its last listed count is unknown: {why}")));
        }
        // An alert waiting for its P2 or P3 input is hidden and counted.
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
        // A conditional write whose operands the hour lacks is not made: the marker is left as
        // it was, unknown until a decided write.
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
    // Read the first entry window's size when positioning the mission list.
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
        // The root draws nothing, the list with it: the list's layout returns.
        Ok(None) => return Ok(updated(Placement::Placed(Vec::new()))),
        Err(refusals) => return Ok(updated(unplaced(icon, windows, &refusals))),
    };
    let Some(holder) = layout
        .holders
        .iter()
        .find(|holder| holder.path == windows.list.path)
    else {
        // The list or a window above it is hidden: the list's layout returns, and every entry
        // stands where it stood.
        return Ok(updated(Placement::Placed(Vec::new())));
    };
    // Its only error is an element that is no overlapping list, as the reader built it.
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

/// What an update that made all its calls wrote and left unknown: each record's entry's parts
/// (its count and pride marker, [`gui_windows::Update::wrote`] and
/// [`gui_windows::Update::unknown`], taken by the module's refusal state through [`marks`]), the
/// listed entries' places, and whether it placed the list ([`LIST_PLACE`]).
struct Updated {
    placement: Placement,
    entries: Vec<gui_windows::Update>,
    list_placed: bool,
}

/// Entry places after an update completed all its calls. Showing the list lays
/// it out, placing each listed entry and re-parenting it last. Every icon update
/// that lays out the list writes its listed entries' places again; other updates
/// leave those places unchanged.
#[derive(Debug)]
enum Placement {
    /// The records whose entries the list's layout set at their places: none when nothing is
    /// listed, or when the list or a window above it is hidden (the layout returns, leaving every
    /// entry where it stood).
    Placed(Vec<usize>),
    /// The module refuses what the places come from (the root's layout, or the list's places),
    /// for the reason given: where the listed records' entries now stand is unknown.
    Unknown(Vec<usize>, String),
}

/// The listed entries whose places the module cannot compute (`refusals`): the list's layout
/// re-parents them last all the same, but where it set them is
/// unknown ([`Placement::Unknown`]).
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

    /// The pool for `windows` in `viewport`, rebuilt for another registry or viewport.
    /// The game constructs 0x40 hidden mission icons and their entries. Here entries
    /// are built when their slot is first taken, since ticking a window that has never
    /// been shown changes nothing.
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

    /// Present the prepared hour in the shared schedule order: request or erase
    /// reached regions, sweep unreached owners, update every retained live icon
    /// and apply its mission display writes.
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
        // A frame that stops leaves the icons it did not reach as they were, and the failure is
        // structural, the same every frame: every icon is withheld until the windows are built
        // again (the entries draw only through their icon).
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

    /// A region's newly allocated slot after the view's allocations may have left the game's
    /// ([`Self::diverged`]): it may hold another region's history there, so the parts an update
    /// may keep are unknown, the icon's ([`LIST_PLACE`]) and its entries' ([`ENTRY_KEPT`]), each
    /// entry waits for its next update ([`Unknown::Update`]), and the icon's and every entry's
    /// fade and movement are unknown ([`Self::unsettled`]).
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

    pub(super) fn take_grid_held(&mut self) -> Vec<(SlotId, Vec<Held>)> {
        self.icons
            .iter_mut()
            .filter_map(|(&slot, icon)| {
                (!icon.held_grids.is_empty()).then(|| (slot, std::mem::take(&mut icon.held_grids)))
            })
            .collect()
    }

    /// The windows whose fade and movement this frame's update left unknown, taken.
    pub(super) fn take_unsettled(&mut self) -> Vec<(MissionWindow, String)> {
        std::mem::take(&mut self.unsettled)
    }

    /// The instance of a mission window, when it is built.
    pub(super) fn window(&self, window: MissionWindow) -> Option<&Instance> {
        match window {
            MissionWindow::Icon(slot) => self.pool.as_ref()?.instance(slot),
            MissionWindow::Entry(slot, record) => self.icons.get(&slot)?.entries.get(record),
        }
    }

    /// A refusal on a mission window's refusal state: an icon's in its pool slot, an entry's in
    /// [`Self::entries_withheld`].
    pub(super) fn refuse(
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

    /// `part` of a mission window known again.
    pub(super) fn known(&mut self, window: MissionWindow, part: &'static str) {
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
        let pass = pool.update(root, |region, _| {
            match reached.iter().find(|owner| owner.region == *region) {
                Some(owner) => Admission::Inside(owner.point),
                // Unreachable: every live icon's region was reached above.
                None => Admission::Empty,
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
            let entries = owner.entries.unwrap_or_default();
            let (Some(instance), Some(icon)) = (pool.get(slot), self.icons.get_mut(&slot)) else {
                continue;
            };
            // The base's `gui_show` reached the list.
            let held = instance.take_held();
            reach_entries(icon, windows, &held)?;
            match update_icon(windows, resources, instance, icon, entries, viewport) {
                // Every call made. the game makes the same calls on every
                // update, so this one wrote again what an earlier one left unknown of its own
                // calls, bar the operand-dependent parts it left unknown; a structural refusal
                // it does not lift.
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
                        // The list's layout set these entries at their places
                        // (the game from
                        // run by the list's Show in
                        // every update): where an earlier update left one unknown is known again.
                        Placement::Placed(records) => {
                            for record in records {
                                self.entries_withheld
                                    .rewrote(&(slot, record), Rewrote::Part(ENTRY_PLACE));
                            }
                        }
                        // Only the next update that lays the list out with the entry listed
                        // sets it again: an entry that leaves the list first stays withheld
                        // while its hide runs.
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
                // A call that does not apply stopped the update part-way: what followed it is
                // unknown, and the same call fails on every update of these windows, so no later
                // update writes it all again. The icon and the entries it draws are withheld
                // until the windows are built again (`MissionView::build`: a new pool).
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

    /// The frame's first phase over every mission window: the pool's roots, in use or not, and
    /// every entry each pooled icon built, shown or not ([`Pool::tick_clocks`],
    /// [`Instance::tick_clocks`]). the game advances every shown
    /// frame-animated sprite and looping
    /// cornered tile before any window's transition, so an
    /// entry that a root's ended hide reaches has had this frame's clock already.
    pub(super) fn tick_clocks(
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

    /// Every mission window's position in the GUI transition list
    /// ([`native_gui::lifecycle::Registration`]): live and free pool roots and each
    /// pooled icon's entries.
    pub(super) fn registrations(
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

    /// Tick transitions in reverse registration order. A root whose fade or hide
    /// movement ends immediately hides its registered children, including retained
    /// entries outside the current visible list.
    pub(super) fn tick_transitions_of(
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

    /// The frame's last phase over every mission window ([`Pool::tick_draws`],
    /// [`Instance::tick_draws`]), the pool's own diagnostics counted.
    pub(super) fn tick_draws(
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

    /// Whether a mission window changes on its own ([`Pool::active`], [`Instance::active`]), or
    /// an icon is kept off screen by the presentation-dirty bit ([`Self::kept_off_screen`]).
    /// Whether a tick would change any mission window: one whose fade and movement this frame's
    /// update left unknown, a pool with a growth waiting, or a window that changes on its own or
    /// was written since its last tick ([`Pool::tick_due`], [`Instance::idle`]).
    pub(super) fn tick_due(&self) -> bool {
        !self.unsettled.is_empty()
            || self.pool.as_ref().is_some_and(Pool::tick_due)
            || self
                .icons
                .values()
                .any(|icon| icon.entries.iter().any(|entry| !entry.idle()))
    }

    pub(super) fn active(&self) -> bool {
        self.kept_off_screen
            || self.pool.as_ref().is_some_and(Pool::active)
            || self
                .icons
                .values()
                .any(|icon| icon.entries.iter().any(Instance::active))
    }

    /// Every icon of the manager's visible list laid out ([`Pool::visible`]: the live icons whose
    /// window is visible, in the live list's order; an erased icon fading out draws nothing):
    /// its root, and the entries attached under its list in paint order whose windows draw, each
    /// placed where its window stands ([`native_gui::Holder::place`]). An icon or entry a refused
    /// update withholds draws nothing and is counted under its reason. The drawn entries'
    /// undecided children and undefined sprites are counted.
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
                                        use super::naval_mission_inputs::hover::Control;
                                        for (element_kind, name, control) in [
                                            (native::Kind::Button,"button",Control::Button),
                                            (native::Kind::TextBox,"count",Control::Status),
                                            (native::Kind::Icon,"alert",Control::Status),
                                            (native::Kind::TextBox,"efficiency",Control::Status),
                                            (native::Kind::Icon,"pride",Control::Pride),
                                        ] {
                                            let Some(button)=windows.entry.find(element_kind,name) else {continue;};
                                            if let Some(hit) = viewer_foundation::map::gui_resources::hit_rectangle(
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

/// The presented frame's updates of the mission icons on the view's windows
/// ([`MissionView::frame`]); the windows they found, which the view's one ordered pass then ticks
/// ([`naval_windows::tick`]) and [`draw`] draws, or `None` when there are none. The regions the
/// frame's province update list reaches ([`reach`]) are gated on their ledger;
/// a region's window stands at its icon position projected.
pub(super) fn update<'n>(
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
            Some(anchor) => inputs.screen(anchor),
            None => {
                if entries.is_some() {
                    report.unanchored_naval_missions += 1;
                }
                None
            }
        };
        // A region with no icon position, or one behind the eye, keeps no icon: the game asks
        // for one, so the pool's slots may no longer be the game's.
        let (entries, point) = match point {
            Some(point) => (entries, point.map(|value| value.round() as i32)),
            None => {
                if entries.is_some() {
                    view.diverged = Some(
                        format!("region {region}'s icon with no decided place is not requested")
                            .into(),
                    );
                }
                (None, [0; 2])
            }
        };
        view.kept_off_screen |= !in_view && entries.is_some();
        reached.push(Reached {
            region,
            entries,
            point,
        });
    }
    // A pass that runs but cannot be planned erases every icon, which the game does not.
    if let Some(why) = pass.undecided {
        view.diverged = Some(format!("the pass runs but cannot be planned: {why}").into());
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

/// Regions are reached once through their first scheduled province. Camera
/// admission and retained dirty provinces share the update list. A region
/// with no scheduled province is swept after its dirty phase clears.
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

/// Each icon of the manager's visible list drawn at its window's place under the mission
/// priority, one ordinal per icon in the live list's order, after the frame's tick.
// Keep the pass, retained windows and output sinks borrowed from their existing owners.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
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
        let drawn = gui_windows::draw_window(
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

/// Mission icons show strictly below the content's mission-icon distance cutoff.
fn mission_pass(height: f32, defines: &CounterDefines) -> bool {
    visible(height, defines.naval_mission_cutoff)
}

#[cfg(test)]
mod tests;
