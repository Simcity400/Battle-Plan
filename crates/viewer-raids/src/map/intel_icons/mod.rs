
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};

use content_reader_layers::ResolvedLayers;
use content_reader_localisation::localisation::Localisation;
use replay_data::{rows::OperationRow, rows::OperativeMissionRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use viewer_native_gui::native_gui;
use viewer_foundation::tooltip_text::Tooltip;

use super::raids::{RaidAssets, script::World};

mod build;
mod missions;
mod tooltips;
pub mod values;
pub mod windows;

pub use windows::{IntelHit, IntelInputs, IntelView, PreparedIntel, hit_at, hit_seen, prepare};

use viewer_native_gui::native_windows;

pub const CONTAINER_WINDOW: &str = native_windows::INTEL_CONTAINER_WINDOW;
pub const ENTRIES_LIST: &str = "entries";
pub const SHIELD_WINDOW: &str = native_windows::INTEL_SHIELD_WINDOW;
pub const OPERATIVE_WINDOW: &str = native_windows::INTEL_OPERATIVE_WINDOW;
pub const OPERATION_WINDOW: &str = native_windows::INTEL_OPERATION_WINDOW;
pub const MORE_WINDOW: &str = native_windows::INTEL_MORE_WINDOW;
pub const OPERATIVE_UNKNOWN_PORTRAIT: &str = "GFX_mapicon_operative_unknown";

const NATIONALITY_MODIFIER: &str = "operative_nationality_mission";
const NATIONALITY_KEY: &str = "root_out_resistance_effectiveness_factor";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OperationPriorities {
    pub can_start: i32,
    pub completed: i32,
    pub in_progress: i32,
    pub prepared: i32,
    pub default: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CounterDefines {
    pub base: i64,
    pub operative_base: i64,
    pub stacking: i64,
    pub foreign_agent: i64,
    pub log_factor: i64,
    pub divisor: i64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActivityDefines {
    pub thresholds: Vec<i64>,
    pub colors: Vec<[f32; 4]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IntelDefines {
    pub missions: Result<missions::Defines, String>,
    pub cutoff: i64,
    pub offset: [i32; 2],
    pub max_stack: i32,
    pub priorities: OperationPriorities,
    pub counter: Result<CounterDefines, String>,
    pub activity: Result<ActivityDefines, String>,
    pub root_out: Result<[i64; 2], String>,
    pub control_trade_max: Result<i64, String>,
    pub diplomatic_max: Result<i64, String>,
    pub propaganda_max: Result<[i64; 2], String>,
}

impl IntelDefines {
    pub fn read(d: &content_reader_defines::defines::Defines) -> io::Result<Self> {
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
            missions: missions::Defines::read(d),
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

    pub fn cutoff(&self) -> f32 {
        viewer_gpu::camera::fixed5_camera_distance(self.cutoff)
    }
}

pub struct IntelAssets {
    pub defines: Result<IntelDefines, String>,
    pub scripted: Option<Arc<Result<dashboard_text::scripted_text::ScriptedContent, String>>>,
    operations: Result<content_reader_military::intelligence::Intelligence, String>,
    phases: Result<content_reader_military::intelligence::OperationPhases, String>,
    nationality: Result<i64, String>,
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
    pub fn load(layers: &ResolvedLayers, defines: &content_reader_defines::defines::Defines) -> Self {
        let mut warnings = Vec::new();
        let defines = IntelDefines::read(defines).map_err(|error| error.to_string());
        if let Err(why) = &defines {
            warnings.push(format!(
                "Operatives map icons are not drawn: their defines cannot be read ({why})."
            ));
        }
        let operations =
            content_reader_military::intelligence::read(layers).map_err(|error| error.to_string());
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
        let phases = content_reader_military::intelligence::read_operation_phases(layers)
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
        let nationality = content_reader_politics::static_modifiers::read(
            layers,
            &[content_reader_politics::static_modifiers::Request {
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
            scripted: None,
            operations,
            phases,
            nationality,
            warnings,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test(
        defines: IntelDefines,
        operations: Vec<content_reader_military::intelligence::Operation>,
        phases: Vec<content_reader_military::intelligence::OperationPhase>,
        nationality: i64,
    ) -> Self {
        Self {
            defines: Ok(defines),
            scripted: None,
            operations: Ok(content_reader_military::intelligence::Intelligence {
                operations: operations
                    .into_iter()
                    .map(|operation| (operation.token.to_ascii_lowercase(), operation))
                    .collect(),
                ..Default::default()
            }),
            phases: Ok(content_reader_military::intelligence::OperationPhases {
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

    pub fn gui_assets(
        &self,
        registry: &content_reader_gui::gui::native::resources::Registry,
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
                    .map(content_reader_military::intelligence::OperationPhase::map_sprite)
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
    ) -> Result<Option<&content_reader_military::intelligence::Operation>, String> {
        self.operations
            .as_ref()
            .map_err(Clone::clone)?
            .operation(token)
    }

    fn phase(
        &self,
        token: &str,
    ) -> Result<Option<&content_reader_military::intelligence::OperationPhase>, String> {
        self.phases.as_ref().map_err(Clone::clone)?.phase(token)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    Province(u32),
    Centre([f32; 3]),
}

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

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntryKey {
    Shield(u32),
    Operative(u32, u64),
    Operation(u64),
    Offer(u32, u32),
    More,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Write {
    Shown(&'static str, bool),
    Sprite(&'static str, Vec<u8>),
    Portrait(&'static str, Vec<u8>),
    Frame(&'static str, i32),
    Text(&'static str, String),
    Color(&'static str, [f32; 4]),
    Progress(&'static str, i32),
    Enabled(&'static str, bool),
    Withheld(&'static str, String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub kind: EntryKind,
    pub key: EntryKey,
    pub writes: Vec<Write>,
    pub tooltip: Arc<Tooltip>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StateIcon {
    pub state: u32,
    pub anchor: Anchor,
    pub entries: Vec<Entry>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IntelReport {
    pub states: usize,
    pub entries: usize,
    pub reasons: BTreeMap<String, usize>,
}

impl IntelReport {
    pub(crate) fn note(&mut self, why: impl Into<String>) {
        *self.reasons.entry(why.into()).or_default() += 1;
    }

    pub fn warnings(&self) -> Vec<String> {
        self.reasons
            .iter()
            .map(|(why, count)| format!("Operatives map icons: {why} ({count})"))
            .collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IntelIcons {
    pub states: Vec<StateIcon>,
    pub report: IntelReport,
    pub built_for: Option<(replay_data::BranchId, replay_data::Hour, u32)>,
}

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

fn complete<'a, T: replay_data::rows::TableSchema>(
    frame: &'a DecodedFrame,
    name: &str,
) -> Result<&'a [T::Row], String> {
    frame
        .complete::<T>()
        .map_err(|error| format!("{name} is not complete: {error}"))
}

struct Tables<'a> {
    missions: Result<&'a [OperativeMissionRow], String>,
    operations: Result<&'a [OperationRow], String>,
    buffers: Result<&'a [replay_data::rows::AgencyOperationBuffersRow], String>,
    strategic: Result<&'a [replay_data::rows::StrategicOperativeRow], String>,
    defense: Result<&'a [replay_data::rows::RaidIntelInputRow], String>,
    networks: Result<&'a [replay_data::rows::IntelNetworkRow], String>,
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
            stability: complete::<tables::CountryStabilityInputs>(frame, "CountryStabilityInputs"),
            parties: complete::<tables::PoliticalParties>(frame, "PoliticalParties"),
            characters: complete::<tables::CharacterOperatives>(frame, "CharacterOperatives"),
            portraits: complete::<tables::CharacterPortraits>(frame, "CharacterPortraits"),
            states: complete::<tables::States>(frame, "States"),
        }
    }
}

type Slots = BTreeSet<u32>;

type RawBySlot = BTreeMap<u32, Vec<u32>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationState {
    Preparing,
    Running { elapsed: i32, remaining: i32 },
    Finished,
}

#[cfg(test)]
mod tests;
