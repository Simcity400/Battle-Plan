//! The Operatives map mode's intel icons for the viewing country: one `intel_map_mode_map_icon`
//! container per state, its overlapping `entries` list holding the state's entries.
//!
//! - **Entries**, from the viewing country only: a counterintelligence shield for each friendly
//!   country (the viewer, or a country sharing its non-null faction or in a subject relation with
//!   it either way, unless an active war between them vetoes it) at its capital state; each
//!   operative of the viewer's active roster on a mission, and each one relocating with a
//!   pending targeted mission; each of the viewer's operation instances.
//! - **States**: build and quiet network, root out resistance, boost ideology and propaganda
//!   missions stand in their target state, the other missions in their target country's capital
//!   state; a relocating operative in its pending state; an operation in its target province's
//!   state, else its target country's capital state.
//! - **Order**: shields by country display name, operatives by their owner's display name then
//!   roster position, operations by the `NMapIcons.OPERATION_PRIORITY_*` of their state then
//!   definition token, then the More entry; a stable sort. `NMapMode.
//!   MAP_MODE_INTEL_MAX_HORIZONTAL_STACK` bounds a state's operations before sorting: one past it
//!   is omitted and the state's More entry stands for it.
//! - **Anchor**: the state's first province with the greatest positive victory points, at its
//!   label anchor; with none, the state's bounding centre at height zero.
//! - **Shown** in the Operatives mode while the camera's eye height is strictly below
//!   `NGraphics.UNITS_ICONS_DISTANCE_CUTOFF`; free cameras use each anchor's distance.
//!
//! What the hour lacks for a part is counted by name; the part is hidden, never guessed.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};

use installed_content_core::ResolvedLayers;
use installed_content_localisation::localisation::Localisation;
use replay_data::{rows::OperationRow, rows::OperativeMissionRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use viewer_foundation::{map::native_gui, tooltip_text::Tooltip};

use super::raids::{RaidAssets, script::World};

mod build;
mod tooltips;
pub mod values;
pub mod windows;

pub use windows::{IntelHit, IntelInputs, IntelView, PreparedIntel, hit_at, hit_seen, prepare};

use viewer_foundation::map::native_windows;

/// The state container and its list.
pub const CONTAINER_WINDOW: &str = native_windows::INTEL_CONTAINER_WINDOW;
pub const ENTRIES_LIST: &str = "entries";
/// The entry windows.
pub const SHIELD_WINDOW: &str = native_windows::INTEL_SHIELD_WINDOW;
pub const OPERATIVE_WINDOW: &str = native_windows::INTEL_OPERATIVE_WINDOW;
pub const OPERATION_WINDOW: &str = native_windows::INTEL_OPERATION_WINDOW;
pub const MORE_WINDOW: &str = native_windows::INTEL_MORE_WINDOW;
/// The sprite the game's portrait setter gives the operative entry's portrait when the
/// operative's own portrait cannot be shown, resolved through the content's interface files.
pub const OPERATIVE_UNKNOWN_PORTRAIT: &str = "GFX_mapicon_operative_unknown";

/// The static modifier whose root-out effectiveness an operative of the target's nationality adds.
const NATIONALITY_MODIFIER: &str = "operative_nationality_mission";
const NATIONALITY_KEY: &str = "root_out_resistance_effectiveness_factor";

/// The `NMapIcons.OPERATION_PRIORITY_*` an operation sorts by, by its state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OperationPriorities {
    pub can_start: i32,
    pub completed: i32,
    pub in_progress: i32,
    pub prepared: i32,
    pub default: i32,
}

/// The counterintelligence operands of the shield's rating and an operative's contribution
/// (`NOperatives`, fixed5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CounterDefines {
    /// `BASE_COUNTER_INTELLIGENCE_RATING`.
    pub base: i64,
    /// `OPERATIVE_BASE_INTEL_AGENCY_DEFENSE`.
    pub operative_base: i64,
    /// `COUNTER_INTELLIGENCE_STACKING_FACTOR`.
    pub stacking: i64,
    /// `COUNTER_INTELLIGENCE_FOREIGN_AGENT_FACTOR`.
    pub foreign_agent: i64,
    /// `COUNTER_INTELLIGENCE_TO_DEFENSE_LOG_FACTOR`.
    pub log_factor: i64,
    /// `COUNTER_INTELLIGENCE_TO_DEFENSE_DIVISOR`.
    pub divisor: i64,
}

/// `NInterface.COUNTERINTELLIGENCE_ACTIVITY_LEVEL_THRESHOLD_VALUES` (fixed5) and `_COLORS`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActivityDefines {
    pub thresholds: Vec<i64>,
    pub colors: Vec<[f32; 4]>,
}

/// Every define the layer reads. The layout's own are required; the others refuse only the
/// part that reads them.
#[derive(Clone, Debug, PartialEq)]
pub struct IntelDefines {
    /// `NGraphics.UNITS_ICONS_DISTANCE_CUTOFF` as stored, fixed5.
    pub cutoff: i64,
    /// `NMapIcons.INTEL_MAP_MODE_MAP_ICON_OFFSET`, each truncated.
    pub offset: [i32; 2],
    /// `NMapMode.MAP_MODE_INTEL_MAX_HORIZONTAL_STACK`.
    pub max_stack: i32,
    pub priorities: OperationPriorities,
    pub counter: Result<CounterDefines, String>,
    pub activity: Result<ActivityDefines, String>,
    /// `NOperatives.OPERATIVE_BASE_ROOT_OUT_RESISTANCE_EFFICIENCY` and
    /// `ROOT_OUT_RESISTANCE_STACKING_FACTOR`.
    pub root_out: Result<[i64; 2], String>,
    /// `NOperatives.CONTROL_TRADE_MAX_INFLUENCE`.
    pub control_trade_max: Result<i64, String>,
    /// `NOperatives.DIPLOMATIC_PRESSURE_MAX_AI_ACCEPTANCE_SCORE_INCREASE`.
    pub diplomatic_max: Result<i64, String>,
    /// `NCountry.MAX_PROPAGANDA_STABILITY_IMPACT` and `MAX_PROPAGANDA_WAR_SUPPORT_IMPACT`.
    pub propaganda_max: Result<[i64; 2], String>,
}

impl IntelDefines {
    pub fn read(d: &installed_content_core::defines::Defines) -> io::Result<Self> {
        let named =
            |group: &str, name: &str, error: io::Error| format!("NDefines.{group}.{name}: {error}");
        let fixed = |group: &'static str, name: &'static str| {
            d.fixed5(group, name)
                .map_err(|error| named(group, name, error))
        };
        let offset = d.numbers("NMapIcons", "INTEL_MAP_MODE_MAP_ICON_OFFSET")?;
        let &[x, y] = offset.as_slice() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "NDefines.NMapIcons.INTEL_MAP_MODE_MAP_ICON_OFFSET does not hold two numbers",
            ));
        };
        let priority = |name| d.int32("NMapIcons", name);
        let activity = (|| -> Result<ActivityDefines, String> {
            let thresholds = d
                .fixed5_array(
                    "NInterface",
                    "COUNTERINTELLIGENCE_ACTIVITY_LEVEL_THRESHOLD_VALUES",
                )
                .map_err(|error| error.to_string())?;
            // A list of RGBA lists.
            let colors = d
                .number_lists(
                    "NInterface",
                    "COUNTERINTELLIGENCE_ACTIVITY_LEVEL_THRESHOLD_COLORS",
                )
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(|c| match c[..] {
                    [r, g, b, a] => Ok([r as f32, g as f32, b as f32, a as f32]),
                    _ => Err(
                        "NDefines.NInterface.COUNTERINTELLIGENCE_ACTIVITY_LEVEL_THRESHOLD_COLORS holds a colour that is not RGBA"
                            .to_owned(),
                    ),
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ActivityDefines { thresholds, colors })
        })();
        Ok(Self {
            cutoff: d.fixed5("NGraphics", "UNITS_ICONS_DISTANCE_CUTOFF")?,
            offset: [x, y].map(|value| value.trunc() as i32),
            max_stack: d.int32("NMapMode", "MAP_MODE_INTEL_MAX_HORIZONTAL_STACK")?,
            priorities: OperationPriorities {
                can_start: priority("OPERATION_PRIORITY_CAN_START")?,
                completed: priority("OPERATION_PRIORITY_COMPLETED")?,
                in_progress: priority("OPERATION_PRIORITY_IN_PROGRESS")?,
                prepared: priority("OPERATION_PRIORITY_PREPARED")?,
                default: priority("OPERATION_PRIORITY_DEFAULT")?,
            },
            counter: (|| -> Result<CounterDefines, String> {
                Ok(CounterDefines {
                    base: fixed("NOperatives", "BASE_COUNTER_INTELLIGENCE_RATING")?,
                    operative_base: fixed("NOperatives", "OPERATIVE_BASE_INTEL_AGENCY_DEFENSE")?,
                    stacking: fixed("NOperatives", "COUNTER_INTELLIGENCE_STACKING_FACTOR")?,
                    foreign_agent: fixed(
                        "NOperatives",
                        "COUNTER_INTELLIGENCE_FOREIGN_AGENT_FACTOR",
                    )?,
                    log_factor: fixed("NOperatives", "COUNTER_INTELLIGENCE_TO_DEFENSE_LOG_FACTOR")?,
                    divisor: fixed("NOperatives", "COUNTER_INTELLIGENCE_TO_DEFENSE_DIVISOR")?,
                })
            })(),
            activity,
            root_out: (|| -> Result<[i64; 2], String> {
                Ok([
                    fixed(
                        "NOperatives",
                        "OPERATIVE_BASE_ROOT_OUT_RESISTANCE_EFFICIENCY",
                    )?,
                    fixed("NOperatives", "ROOT_OUT_RESISTANCE_STACKING_FACTOR")?,
                ])
            })(),
            control_trade_max: fixed("NOperatives", "CONTROL_TRADE_MAX_INFLUENCE"),
            diplomatic_max: fixed(
                "NOperatives",
                "DIPLOMATIC_PRESSURE_MAX_AI_ACCEPTANCE_SCORE_INCREASE",
            ),
            propaganda_max: (|| -> Result<[i64; 2], String> {
                Ok([
                    fixed("NCountry", "MAX_PROPAGANDA_STABILITY_IMPACT")?,
                    fixed("NCountry", "MAX_PROPAGANDA_WAR_SUPPORT_IMPACT")?,
                ])
            })(),
        })
    }

    /// The eye height the icons show strictly below ([`values::visible`]).
    pub fn cutoff(&self) -> f32 {
        viewer_foundation::map::camera::fixed5_camera_distance(self.cutoff)
    }
}

/// Everything static the layer reads, from the recorded content stack.
pub struct IntelAssets {
    pub defines: Result<IntelDefines, String>,
    operations: Result<installed_content_core::intelligence::Intelligence, String>,
    phases: Result<installed_content_core::intelligence::OperationPhases, String>,
    /// The nationality modifier's root-out effectiveness factor, fixed5.
    nationality: Result<i64, String>,
    /// What the content refuses, once.
    pub warnings: Vec<String>,
}

impl std::fmt::Debug for IntelAssets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntelAssets")
            .field("defines", &self.defines)
            .field("nationality", &self.nationality)
            .field("warnings", &self.warnings)
            .finish_non_exhaustive()
    }
}

impl IntelAssets {
    /// Reads the defines, the operation and phase definitions and the nationality modifier.
    /// Each read that fails refuses only what reads it, named in [`Self::warnings`].
    pub fn load(layers: &ResolvedLayers, defines: &installed_content_core::defines::Defines) -> Self {
        let mut warnings = Vec::new();
        let defines = IntelDefines::read(defines).map_err(|error| error.to_string());
        if let Err(why) = &defines {
            warnings.push(format!(
                "Operatives map icons are not drawn: their defines cannot be read ({why})."
            ));
        }
        let operations =
            installed_content_core::intelligence::read(layers).map_err(|error| error.to_string());
        match &operations {
            Ok(read) => warnings.extend(
                read.warnings
                    .iter()
                    .map(|why| format!("Operatives map icons: operations: {why}")),
            ),
            Err(why) => warnings.push(format!(
                "Operatives map icons: the operation definitions cannot be read ({why}); operation icons are hidden."
            )),
        }
        let phases = installed_content_core::intelligence::read_operation_phases(layers)
            .map_err(|error| error.to_string());
        match &phases {
            Ok(read) => warnings.extend(
                read.warnings
                    .iter()
                    .map(|why| format!("Operatives map icons: operation phases: {why}")),
            ),
            Err(why) => warnings.push(format!(
                "Operatives map icons: the operation phase definitions cannot be read ({why}); phase icons are hidden."
            )),
        }
        let nationality = installed_content_core::static_modifiers::read(
            layers,
            &[installed_content_core::static_modifiers::Request {
                modifier: NATIONALITY_MODIFIER,
                key: NATIONALITY_KEY,
            }],
        )
        .map_err(|error| error.to_string())
        .and_then(|values| {
            values
                .values
                .into_iter()
                .next()
                .ok_or_else(|| "the static modifier reader answered no value".to_owned())?
                .map_err(|refusal| {
                    format!(
                        "{}: {}.{}: {}",
                        refusal.file, refusal.modifier, refusal.key, refusal.reason
                    )
                })
        });
        if let Err(why) = &nationality {
            warnings.push(format!(
                "Operatives map icons: {NATIONALITY_MODIFIER}'s {NATIONALITY_KEY} cannot be read ({why}); root-out efficiencies of operatives of the target's nationality are hidden."
            ));
        }
        Self {
            defines,
            operations,
            phases,
            nationality,
            warnings,
        }
    }

    /// Synthetic assets: `defines`, operations, phases and the nationality factor as given.
    #[cfg(any(test, feature = "test-support"))]
    pub fn test(
        defines: IntelDefines,
        operations: Vec<installed_content_core::intelligence::Operation>,
        phases: Vec<installed_content_core::intelligence::OperationPhase>,
        nationality: i64,
    ) -> Self {
        Self {
            defines: Ok(defines),
            operations: Ok(installed_content_core::intelligence::Intelligence {
                operations: operations
                    .into_iter()
                    .map(|operation| (operation.token.to_ascii_lowercase(), operation))
                    .collect(),
                ..Default::default()
            }),
            phases: Ok(installed_content_core::intelligence::OperationPhases {
                definitions: phases
                    .into_iter()
                    .map(|phase| (phase.token.to_ascii_lowercase(), phase))
                    .collect(),
                ..Default::default()
            }),
            nationality: Ok(nationality),
            warnings: Vec::new(),
        }
    }

    /// The sprites the code sets that no template names: each operation's map icon and each
    /// phase's map sprite, as `registry` declares them.
    pub fn gui_assets(
        &self,
        registry: &installed_content_core::gui::native::resources::Registry,
    ) -> native_gui::Assets {
        let mut assets = native_gui::Assets::default();
        let declared = |name: &str| registry.sprite(name.as_bytes()).is_some();
        if let Ok(operations) = &self.operations {
            assets.sprites.extend(
                operations
                    .operations
                    .values()
                    .map(|operation| operation.map_icon.as_str())
                    .filter(|name| declared(name))
                    .map(|name| name.as_bytes().to_vec()),
            );
        }
        if let Ok(phases) = &self.phases {
            assets.sprites.extend(
                phases
                    .definitions
                    .values()
                    .map(installed_content_core::intelligence::OperationPhase::map_sprite)
                    .filter(|name| declared(name))
                    .map(|name| name.as_bytes().to_vec()),
            );
        }
        if declared(OPERATIVE_UNKNOWN_PORTRAIT) {
            assets
                .sprites
                .insert(OPERATIVE_UNKNOWN_PORTRAIT.as_bytes().to_vec());
        }
        assets
    }

    fn operation(
        &self,
        token: &str,
    ) -> Result<Option<&installed_content_core::intelligence::Operation>, String> {
        self.operations
            .as_ref()
            .map_err(Clone::clone)?
            .operation(token)
    }

    fn phase(
        &self,
        token: &str,
    ) -> Result<Option<&installed_content_core::intelligence::OperationPhase>, String> {
        self.phases.as_ref().map_err(Clone::clone)?.phase(token)
    }
}

/// Where a state's container stands: a province's label anchor, or a world point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    Province(u32),
    Centre([f32; 3]),
}

/// An entry's window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntryKind {
    Shield,
    Operative,
    Operation,
    More,
}

impl EntryKind {
    pub fn window(self) -> &'static str {
        match self {
            Self::Shield => SHIELD_WINDOW,
            Self::Operative => OPERATIVE_WINDOW,
            Self::Operation => OPERATION_WINDOW,
            Self::More => MORE_WINDOW,
        }
    }
}

/// What an entry stands for, which keeps its window from hour to hour.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntryKey {
    /// A friendly country, by canonical slot.
    Shield(u32),
    /// An operative, by its roster's raw country and its handle.
    Operative(u32, u64),
    /// An operation instance, by its handle.
    Operation(u64),
    More,
}

/// One write the entry's update makes on a child, by name.
#[derive(Clone, Debug, PartialEq)]
pub enum Write {
    /// Shown (the hide bit cleared, then Show) or hidden (Hide, then the hide bit set).
    Shown(&'static str, bool),
    /// The child's sprite, replaced by name.
    Sprite(&'static str, Vec<u8>),
    /// A leader portrait: the sprite, or the unknown leader's when no interface file declares
    /// it.
    Portrait(&'static str, Vec<u8>),
    /// A one-based frame.
    Frame(&'static str, i32),
    Text(&'static str, String),
    Color(&'static str, [f32; 4]),
    /// A progress bar's value, in percent.
    Progress(&'static str, i32),
    Enabled(&'static str, bool),
    /// A write whose operands the hour lacks: the child is hidden and counted with why.
    Withheld(&'static str, String),
}

/// One entry of a state's list: its window, identity, update and hover text.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub kind: EntryKind,
    pub key: EntryKey,
    pub writes: Vec<Write>,
    pub tooltip: Arc<Tooltip>,
}

/// One state's container and its entries in list order.
#[derive(Clone, Debug, PartialEq)]
pub struct StateIcon {
    pub state: u32,
    pub anchor: Anchor,
    pub entries: Vec<Entry>,
}

/// What an hour's layer decided and what it could not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IntelReport {
    pub states: usize,
    pub entries: usize,
    /// Why parts or entries are hidden, with counts.
    pub reasons: BTreeMap<String, usize>,
}

impl IntelReport {
    pub(crate) fn note(&mut self, why: impl Into<String>) {
        *self.reasons.entry(why.into()).or_default() += 1;
    }

    /// The report's diagnostics lines.
    pub fn warnings(&self) -> Vec<String> {
        self.reasons
            .iter()
            .map(|(why, count)| format!("Operatives map icons: {why} ({count})"))
            .collect()
    }
}

/// An hour's icons for one viewing country.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IntelIcons {
    pub states: Vec<StateIcon>,
    pub report: IntelReport,
    /// The branch, hour and canonical country they were built for.
    pub built_for: Option<(replay_data::BranchId, replay_data::Hour, u32)>,
}

/// One view-level omission; the number of cached offers itself is unrecorded.
const OPERATION_OFFERS_UNRECORDED: &str =
    "operation offers are not drawn: their UI cache state is not recorded";
/// Counted once for each operative entry whose highlight is hidden.
const OPERATIVE_SELECTION_UNRECORDED: &str =
    "frame_select_hl_icn is hidden: the operative selection UI state is not recorded";
const OPERATIVE_WARNING_UNRECORDED: &str =
    "frame_warn_hl_icn is hidden: the operative warning UI state is not recorded";

/// The hour's icons for canonical country `ours`. `language` is the recording's client
/// language, which stored names are drawn in.
pub fn icons(
    assets: &IntelAssets,
    raids: &RaidAssets,
    frame: &DecodedFrame,
    ours: u32,
    localisation: &Localisation,
    language: &str,
) -> IntelIcons {
    let mut out = IntelIcons {
        built_for: Some((frame.branch, frame.hour, ours)),
        ..IntelIcons::default()
    };
    out.report.note(OPERATION_OFFERS_UNRECORDED);
    let defines = match &assets.defines {
        Ok(defines) => defines,
        Err(why) => {
            out.report
                .note(format!("the defines cannot be read: {why}"));
            return out;
        }
    };
    let world = match World::new(frame, raids.trigger_content.facts()) {
        Ok(world) => world,
        Err(why) => {
            out.report
                .note(format!("the core tables are incomplete: {why}"));
            return out;
        }
    };
    let hour = build::Hour {
        frame,
        world: &world,
        ours,
        defines,
        assets,
        raids,
        localisation,
        language,
    };
    hour.build(&mut out);
    out.report.states = out.states.len();
    out.report.entries = out.states.iter().map(|state| state.entries.len()).sum();
    out
}

/// A complete table's rows, or why the parts that read it are hidden.
fn complete<'a, T: replay_data::rows::TableSchema>(
    frame: &'a DecodedFrame,
    name: &str,
) -> Result<&'a [T::Row], String> {
    frame
        .complete::<T>()
        .map_err(|error| format!("{name} is not complete: {error}"))
}

/// The tables the layer reads, each complete or why not.
struct Tables<'a> {
    missions: Result<&'a [OperativeMissionRow], String>,
    operations: Result<&'a [OperationRow], String>,
    buffers: Result<&'a [replay_data::rows::AgencyOperationBuffersRow], String>,
    strategic: Result<&'a [replay_data::rows::StrategicOperativeRow], String>,
    defense: Result<&'a [replay_data::rows::RaidIntelInputRow], String>,
    networks: Result<&'a [replay_data::rows::IntelNetworkRow], String>,
    network_summaries: Result<&'a [replay_data::rows::IntelNetworkSummaryRow], String>,
    propaganda: Result<&'a [replay_data::rows::PropagandaEffortRow], String>,
    contributions: Result<&'a [replay_data::rows::IntelContributionRow], String>,
    sources: Result<&'a [replay_data::rows::IntelSourceRow], String>,
    stability: Result<&'a [replay_data::rows::CountryStabilityInputRow], String>,
    parties: Result<&'a [replay_data::rows::PoliticalPartyRow], String>,
    characters: Result<&'a [replay_data::rows::CharacterOperativeRow], String>,
    portraits: Result<&'a [replay_data::rows::CharacterPortraitRow], String>,
    states: Result<&'a [replay_data::rows::StateRow], String>,
}

impl<'a> Tables<'a> {
    fn read(frame: &'a DecodedFrame) -> Self {
        Self {
            missions: complete::<tables::OperativeMissions>(frame, "OperativeMissions"),
            operations: complete::<tables::Operations>(frame, "Operations"),
            buffers: complete::<tables::AgencyOperationBuffers>(frame, "AgencyOperationBuffers"),
            strategic: complete::<tables::StrategicOperatives>(frame, "StrategicOperatives"),
            defense: complete::<tables::RaidIntelInputs>(frame, "RaidIntelInputs"),
            networks: complete::<tables::IntelNetworks>(frame, "IntelNetworks"),
            network_summaries: complete::<tables::IntelNetworkSummaries>(
                frame,
                "IntelNetworkSummaries",
            ),
            propaganda: complete::<tables::PropagandaEfforts>(frame, "PropagandaEfforts"),
            contributions: complete::<tables::IntelContributions>(frame, "IntelContributions"),
            sources: complete::<tables::IntelSources>(frame, "IntelSources"),
            stability: complete::<tables::CountryStabilityInputs>(frame, "CountryStabilityInputs"),
            parties: complete::<tables::PoliticalParties>(frame, "PoliticalParties"),
            characters: complete::<tables::CharacterOperatives>(frame, "CharacterOperatives"),
            portraits: complete::<tables::CharacterPortraits>(frame, "CharacterPortraits"),
            states: complete::<tables::States>(frame, "States"),
        }
    }
}

/// Country slots the hour names, collected before the names are read.
type Slots = BTreeSet<u32>;

/// Raw country references by canonical slot, in raw order.
type RawBySlot = BTreeMap<u32, Vec<u32>>;

/// An operation's state: in preparation, running with its elapsed and remaining whole days,
/// or finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationState {
    Preparing,
    Running { elapsed: i32, remaining: i32 },
    Finished,
}

#[cfg(test)]
mod tests;
