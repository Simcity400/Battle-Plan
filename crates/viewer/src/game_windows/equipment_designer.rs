use super::{
    ComposedPlan, ComposedSubWindow, DEFAULT_VIEWPORT, GameWindowError, GameWindowRender,
    GameWindowRequest, GameWindowTooltip, Service, WindowCacheKey, container_holder,
    layout_window, strip_markup,
};
use content_reader_gui::gui::native::{self, Element, Fields, Kind};
use content_reader_image::native::Decode;
use content_reader_key_names::{
    equipment::{Database, Equipment},
    resources::Resources as ResourceDatabase,
};
use content_reader_layers::{ResolveOptions, ResolvedLayers, content::ResolvedContent};
use content_reader_localisation::localisation::Localisation;
use content_reader_parser::parser::c_string;
use content_reader_script::equipment_stats::category_mask;
use dashboard_base::{
    Source,
    products::equipment_icon::{Icon, IconHour, variant_icon},
};
use dashboard_text::metadata::link_slot;
use replay::campaign::SeekResult;
use replay_data::rows::{
    IndustrialOrganisationRow, LexicalTokenRow, Recorded, TableId, VariantDefinitionRow,
    VariantDesignState, VariantModuleRow, VariantStatsRow, VariantUpgradeRow, tables,
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, atomic::AtomicBool},
};
use viewer_native_gui::native_gui::{
    self, Assets, Layout, Resources, TextureFile, lifecycle::Instance,
};
use viewer_native_gui::native_text::{COLOR, ICON};

#[cfg(test)]
mod tests;

const SHIP_DLC_BIT: u64 = 1 << 4;
const TANK_DLC_BIT: u64 = 1 << 7;
const PLANE_DLC_BIT: u64 = 1 << 8;

const SHIP_CATEGORY_MASK: u64 = 0x8000_4001_c0;
const TANK_CATEGORY_MASK: u64 = 4;
const PLANE_CATEGORY_MASK: u64 = 0x1d00_26fc_00;

const FIXED5_SCALE: f64 = 100_000.0;
const NOT_RECORDED_VALUE: &str = "n/a";
const NOT_RECORDED_TOOLTIP: &str =
    "Not recorded: this stat is not recorded in the current recording format and is never guessed.";
const PREVIEW_3D_TOOLTIP: &str = "3D model preview is not rendered in BattlePlan.";

const REQUIRED_TABLES: &[TableId] = &[
    TableId::VariantDefinitions,
    TableId::VariantModules,
    TableId::VariantUpgrades,
    TableId::VariantStats,
    TableId::SimulationFeatures,
    TableId::LexicalTokens,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
    TableId::PoliticalParties,
    TableId::Politics,
    TableId::Factions,
    TableId::FactionMembers,
    TableId::States,
    TableId::IndustrialOrganisations,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpgradeDomain {
    Land,
    Air,
    Naval,
}

impl UpgradeDomain {
    pub fn sub_window(self) -> &'static str {
        match self {
            Self::Land => "land_equipment_designer_view",
            Self::Air => "air_equipment_designer_view",
            Self::Naval => "naval_equipment_designer_view",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesignerKind {
    Ship,
    Tank,
    Plane,
    Upgrade(UpgradeDomain),
}

impl DesignerKind {
    pub fn window_name(self) -> &'static str {
        match self {
            Self::Ship => "countryequipmentdesignerview",
            Self::Tank => "tank_designer_view",
            Self::Plane => "plane_designer_view",
            Self::Upgrade(_) => "equipment_upgrade_designer_window",
        }
    }

    pub fn title_key(self) -> &'static str {
        match self {
            Self::Ship => "SHIP_DESIGNER_TITLE",
            Self::Tank => "TANK_DESIGNER_TITLE",
            Self::Plane => "PLANE_DESIGNER_TITLE",
            Self::Upgrade(_) => "DESIGNER_CREATE_VARIANT",
        }
    }

    pub fn is_modular(self) -> bool {
        !matches!(self, Self::Upgrade(_))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Target {
    tag: String,
    variant: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlotIcon {
    Fitted {
        slot: String,
        module: String,
        sprite: String,
    },
    Empty {
        slot: String,
        sprite: String,
    },
    Unknown {
        slot: String,
        sprite: String,
        reason: String,
    },
    Locked {
        sprite: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordedStats {
    Available([Option<i64>; 10]),
    MissingRow,
    Unavailable(String),
}

impl RecordedStats {
    fn lane(&self, index: usize) -> Option<i64> {
        match self {
            Self::Available(values) => values[index],
            Self::MissingRow | Self::Unavailable(_) => None,
        }
    }

    fn missing_reason(&self) -> String {
        match self {
            Self::Available(_) => {
                "Not recorded: this stat was withheld or not recorded for this design at this hour."
                    .to_owned()
            }
            Self::MissingRow => {
                "Not recorded: this design has no variant stats row at this hour.".to_owned()
            }
            Self::Unavailable(reason) => {
                format!("Not recorded: variant stats are unavailable at this hour ({reason}).")
            }
        }
    }
}

#[derive(Clone, Debug)]
enum RecordedModules {
    Available(BTreeMap<String, FittedModule>),
    Unavailable(String),
}

#[derive(Clone, Debug)]
enum RecordedUpgrades {
    Available(Vec<(String, u8)>),
    Unavailable { keys: Vec<String>, reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatLine {
    pub label_key: &'static str,
    pub fallback_label: &'static str,
    pub value: Option<String>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatColumns {
    pub base: Vec<StatLine>,
    pub combat: Vec<StatLine>,
    pub misc: Vec<StatLine>,
}

#[derive(Clone, Debug)]
struct ResolvedMio {
    name: String,
    sprite: Option<String>,
}

#[derive(Clone, Debug)]
struct FittedModule {
    module: String,
    category: String,
    gui_category: String,
}

#[derive(Clone, Debug)]
struct EquipmentDesignData {
    kind: DesignerKind,
    tag: String,
    equipment_key: String,
    archetype_key: String,
    air_map_icon_frame: i32,
    category_mask: u64,
    base_name: String,
    version_suffix: String,
    append_version: bool,
    auto_upgraded: Option<bool>,
    design_state_reason: Option<String>,
    naming_group: Option<String>,
    niche_frame: Option<i32>,
    mio: Option<ResolvedMio>,
    module_slots: Vec<(String, String)>,
    fitted_modules: RecordedModules,
    upgrades: RecordedUpgrades,
    stats: RecordedStats,
    manpower: Option<i32>,
    build_cost: Option<i64>,
    build_resources: Option<Vec<(String, i64)>>,
}

pub fn classify_designer(
    database: &Database,
    equipment: &Equipment,
    dlc_mask: Option<u64>,
    dlc_unavailable: Option<&str>,
) -> Result<DesignerKind, String> {
    let category = database.category(equipment);
    if !equipment.module_slots.is_empty() {
        let modular_family = category & (SHIP_CATEGORY_MASK | TANK_CATEGORY_MASK | PLANE_CATEGORY_MASK) != 0;
        if modular_family {
            let Some(mask) = dlc_mask else {
                let reason = dlc_unavailable.unwrap_or("SimulationFeatures is not recorded at this hour");
                return Err(format!(
                    "Cannot determine whether '{}' uses the modular or classic designer because active DLC flags are unavailable ({reason}).",
                    String::from_utf8_lossy(&equipment.name)
                ));
            };
            if mask & SHIP_DLC_BIT != 0 && category & SHIP_CATEGORY_MASK != 0 {
                return Ok(DesignerKind::Ship);
            }
            if mask & TANK_DLC_BIT != 0 && category & TANK_CATEGORY_MASK != 0 {
                return Ok(DesignerKind::Tank);
            }
            if mask & PLANE_DLC_BIT != 0 && category & PLANE_CATEGORY_MASK != 0 {
                return Ok(DesignerKind::Plane);
            }
        }
    }
    let [land, air, naval] = database.designer_families(equipment)?;
    if naval {
        Ok(DesignerKind::Upgrade(UpgradeDomain::Naval))
    } else if air {
        Ok(DesignerKind::Upgrade(UpgradeDomain::Air))
    } else if land {
        Ok(DesignerKind::Upgrade(UpgradeDomain::Land))
    } else {
        Err(format!(
            "Equipment '{}' does not belong to a supported designer family.",
            String::from_utf8_lossy(&equipment.name)
        ))
    }
}

pub fn resolve_fitted_module_sprite(
    module_key: &str,
    module_gfx: &str,
    has_sprite: impl Fn(&str) -> bool,
) -> String {
    for prefix in ["GFX_EMI_", "GFX_SMI_"] {
        let direct = format!("{prefix}{module_key}");
        if has_sprite(&direct) {
            return direct;
        }
    }
    if !module_gfx.is_empty() {
        if module_gfx.starts_with("GFX_") && has_sprite(module_gfx) {
            return module_gfx.to_owned();
        }
        for prefix in ["GFX_EMI_", "GFX_SMI_"] {
            let custom = format!("{prefix}{module_gfx}");
            if has_sprite(&custom) {
                return custom;
            }
        }
    }
    "GFX_EMI_unknown".to_owned()
}

pub fn resolve_empty_slot_sprite(
    slot_key: &str,
    slot_gfx: &str,
    has_sprite: impl Fn(&str) -> bool,
) -> String {
    let direct = format!("GFX_EMSI_{slot_key}");
    if has_sprite(&direct) {
        return direct;
    }
    if !slot_gfx.is_empty() {
        let custom = if slot_gfx.starts_with("GFX_") {
            slot_gfx.to_owned()
        } else {
            format!("GFX_EMSI_{slot_gfx}")
        };
        if has_sprite(&custom) {
            return custom;
        }
    }
    "GFX_EMSI_empty".to_owned()
}

pub fn locked_slot_sprite() -> &'static str {
    "GFX_EMSI_locked"
}

pub fn format_fixed5(raw: i64, decimals: usize) -> String {
    format!("{:.decimals$}", raw as f64 / FIXED5_SCALE)
}

pub fn format_percent_fixed5(raw: i64, decimals: usize) -> String {
    format!("{:.decimals$}%", raw as f64 / (FIXED5_SCALE / 100.0))
}

fn recorded_stat(
    stats: &RecordedStats,
    lane: usize,
    format_value: impl FnOnce(i64) -> String,
) -> (Option<String>, Option<String>) {
    match stats.lane(lane) {
        Some(raw) => (Some(format_value(raw)), None),
        None => (None, Some(stats.missing_reason())),
    }
}

fn unrecorded_stat(label_key: &'static str, fallback_label: &'static str) -> StatLine {
    StatLine {
        label_key,
        fallback_label,
        value: None,
        detail: Some(NOT_RECORDED_TOOLTIP.to_owned()),
    }
}

pub fn build_stat_columns(
    kind: DesignerKind,
    stats: &RecordedStats,
    manpower: Option<i32>,
    design_state_reason: Option<&str>,
) -> StatColumns {
    match kind {
        DesignerKind::Ship => {
            let (reliability, reliability_detail) =
                recorded_stat(stats, VariantStatsRow::RELIABILITY, |raw| {
                    format_percent_fixed5(raw, 1)
                });
            let (fuel, fuel_detail) =
                recorded_stat(stats, VariantStatsRow::FUEL_CONSUMPTION, |raw| {
                    format_fixed5(raw, 2)
                });
            StatColumns {
                base: vec![
                    unrecorded_stat("STAT_NAVY_MAXIMUM_SPEED", "Max Speed:"),
                    unrecorded_stat("STAT_NAVY_RANGE", "Max Range:"),
                    unrecorded_stat("STAT_COMMON_MAX_ORG", "Organization:"),
                    unrecorded_stat("STAT_COMMON_MAX_STRENGTH", "HP:"),
                    StatLine {
                        label_key: "STAT_COMMON_RELIABILITY",
                        fallback_label: "Reliability:",
                        value: reliability,
                        detail: reliability_detail,
                    },
                    unrecorded_stat("STAT_ARMY_SUPPLY_CONSUMPTION", "Supply Use:"),
                    StatLine {
                        label_key: "STAT_COMMON_MANPOWER",
                        fallback_label: "Manpower",
                        value: manpower.map(|value| value.to_string()),
                        detail: manpower.is_none().then(|| {
                            design_state_reason
                                .map(str::to_owned)
                                .unwrap_or_else(|| NOT_RECORDED_TOOLTIP.to_owned())
                        }),
                    },
                ],
                combat: vec![
                    unrecorded_stat("STAT_NAVY_LG_ATTACK", "Light Attack:"),
                    unrecorded_stat("STAT_NAVY_LG_ARMOR_PIERCING", "Light Piercing:"),
                    unrecorded_stat("STAT_NAVY_HG_ATTACK", "Heavy Attack:"),
                    unrecorded_stat("STAT_NAVY_HG_ARMOR_PIERCING", "Heavy Piercing:"),
                    unrecorded_stat("STAT_NAVY_TORPEDO_ATTACK", "Torpedo Attack:"),
                    unrecorded_stat("STAT_NAVY_SUB_ATTACK", "Depth Charges:"),
                    unrecorded_stat("STAT_COMMON_ARMOR", "Armor:"),
                    unrecorded_stat("STAT_NAVY_ANTI_AIR_ATTACK", "Anti-Air:"),
                ],
                misc: vec![
                    StatLine {
                        label_key: "STAT_COMMON_FUEL_CONSUMPTION",
                        fallback_label: "Fuel Usage:",
                        value: fuel,
                        detail: fuel_detail,
                    },
                    unrecorded_stat("STAT_NAVY_SURFACE_VISIBILITY", "Surface Visibility:"),
                    unrecorded_stat("STAT_NAVY_SURFACE_DETECTION", "Surface Detection:"),
                    unrecorded_stat("STAT_NAVY_SUB_VISIBILITY", "Sub Visibility:"),
                    unrecorded_stat("STAT_NAVY_SUB_DETECTION", "Sub Detection:"),
                    unrecorded_stat("STAT_NAVY_MINES_PLANTING", "Minelaying:"),
                    unrecorded_stat("STAT_NAVY_MINES_SWEEPING", "Minesweeping:"),
                    unrecorded_stat("STAT_NAVY_WEATHER_PENALTY", "Weather Penalty:"),
                    unrecorded_stat("STAT_NAVY_HIT_PROFILE", "Hit Profile"),
                ],
            }
        }
        DesignerKind::Tank => {
            let (speed, speed_detail) =
                recorded_stat(stats, VariantStatsRow::MAXIMUM_SPEED, |raw| {
                    format!("{} km/h", format_fixed5(raw, 1))
                });
            let (reliability, reliability_detail) =
                recorded_stat(stats, VariantStatsRow::RELIABILITY, |raw| {
                    format_percent_fixed5(raw, 1)
                });
            let (air_attack, air_attack_detail) =
                recorded_stat(stats, VariantStatsRow::AIR_ATTACK, |raw| {
                    format_fixed5(raw, 1)
                });
            let (fuel, fuel_detail) =
                recorded_stat(stats, VariantStatsRow::FUEL_CONSUMPTION, |raw| {
                    format_fixed5(raw, 2)
                });
            StatColumns {
                base: vec![
                    StatLine {
                        label_key: "STAT_COMMON_MAXIMUM_SPEED",
                        fallback_label: "Max Speed:",
                        value: speed,
                        detail: speed_detail,
                    },
                    StatLine {
                        label_key: "STAT_COMMON_RELIABILITY",
                        fallback_label: "Reliability:",
                        value: reliability,
                        detail: reliability_detail,
                    },
                    unrecorded_stat("STAT_COMMON_MAX_STRENGTH", "HP:"),
                    unrecorded_stat("STAT_ARMY_SUPPLY_CONSUMPTION", "Supply Use:"),
                ],
                combat: vec![
                    unrecorded_stat("STAT_ARMY_SOFT_ATTACK", "Soft Attack:"),
                    unrecorded_stat("STAT_ARMY_HARD_ATTACK", "Hard Attack:"),
                    unrecorded_stat("STAT_COMMON_PIERCING", "Piercing:"),
                    unrecorded_stat("STAT_ARMY_HARDNESS", "Hardness:"),
                    unrecorded_stat("STAT_COMMON_ARMOR", "Armor:"),
                    unrecorded_stat("STAT_ARMY_BREAKTHROUGH", "Breakthrough:"),
                    unrecorded_stat("STAT_ARMY_DEFENSE", "Defense:"),
                    StatLine {
                        label_key: "STAT_AIR_ATTACK",
                        fallback_label: "Air Attack:",
                        value: air_attack,
                        detail: air_attack_detail,
                    },
                ],
                misc: vec![
                    unrecorded_stat("STAT_ARMY_FUEL_CAPACITY", "Fuel Capacity:"),
                    StatLine {
                        label_key: "STAT_COMMON_FUEL_CONSUMPTION",
                        fallback_label: "Fuel Usage:",
                        value: fuel,
                        detail: fuel_detail,
                    },
                    unrecorded_stat("STAT_ARMY_SUPRESSION", "Suppression:"),
                    unrecorded_stat("STAT_ARMY_RECON", "Reconnaissance:"),
                    unrecorded_stat("STAT_ARMY_ENTRENCHMENT", "Entrenchment:"),
                ],
            }
        }
        DesignerKind::Plane => {
            let (speed, speed_detail) =
                recorded_stat(stats, VariantStatsRow::MAXIMUM_SPEED, |raw| {
                    format!("{} km/h", format_fixed5(raw, 1))
                });
            let (range, range_detail) = recorded_stat(stats, VariantStatsRow::AIR_RANGE, |raw| {
                format!("{} km", format_fixed5(raw, 0))
            });
            let (weight, weight_detail) =
                recorded_stat(stats, VariantStatsRow::WEIGHT, |raw| format_fixed5(raw, 1));
            let (thrust, thrust_detail) =
                recorded_stat(stats, VariantStatsRow::THRUST, |raw| format_fixed5(raw, 1));
            let (air_def, air_def_detail) =
                recorded_stat(stats, VariantStatsRow::AIR_DEFENCE, |raw| {
                    format_fixed5(raw, 1)
                });
            let (air_atk, air_atk_detail) =
                recorded_stat(stats, VariantStatsRow::AIR_ATTACK, |raw| {
                    format_fixed5(raw, 1)
                });
            let (agility, agility_detail) =
                recorded_stat(stats, VariantStatsRow::AIR_AGILITY, |raw| {
                    format_fixed5(raw, 1)
                });
            let (reliability, reliability_detail) =
                recorded_stat(stats, VariantStatsRow::RELIABILITY, |raw| {
                    format_percent_fixed5(raw, 1)
                });
            let (fuel, fuel_detail) =
                recorded_stat(stats, VariantStatsRow::FUEL_CONSUMPTION, |raw| {
                    format_fixed5(raw, 2)
                });
            StatColumns {
                base: vec![
                    StatLine {
                        label_key: "STAT_COMMON_MAXIMUM_SPEED",
                        fallback_label: "Max Speed:",
                        value: speed,
                        detail: speed_detail,
                    },
                    StatLine {
                        label_key: "STAT_AIR_RANGE",
                        fallback_label: "Range:",
                        value: range,
                        detail: range_detail,
                    },
                    unrecorded_stat("STAT_ARMY_SUPPLY_CONSUMPTION", "Supply Use:"),
                    StatLine {
                        label_key: "STAT_COMMON_WEIGHT",
                        fallback_label: "Weight:",
                        value: weight,
                        detail: weight_detail,
                    },
                    StatLine {
                        label_key: "STAT_COMMON_THRUST",
                        fallback_label: "Thrust:",
                        value: thrust,
                        detail: thrust_detail,
                    },
                ],
                combat: vec![
                    StatLine {
                        label_key: "STAT_AIR_DEFENCE",
                        fallback_label: "Air Defense:",
                        value: air_def,
                        detail: air_def_detail,
                    },
                    StatLine {
                        label_key: "STAT_AIR_ATTACK",
                        fallback_label: "Air Attack:",
                        value: air_atk,
                        detail: air_atk_detail,
                    },
                    StatLine {
                        label_key: "STAT_AIR_AGILITY",
                        fallback_label: "Agility:",
                        value: agility,
                        detail: agility_detail,
                    },
                    unrecorded_stat("STAT_AIR_SUPERIORITY", "Air Superiority:"),
                    unrecorded_stat("STAT_AIR_NAVAL_STRIKE_ATTACK", "Naval Attack:"),
                    unrecorded_stat("STAT_AIR_NAVAL_STRIKE_TARGETTING", "Naval Targeting:"),
                    unrecorded_stat("STAT_AIR_GROUND_ATTACK", "Ground Attack:"),
                    unrecorded_stat("STAT_AIR_BOMBING", "Strat. Bombing:"),
                ],
                misc: vec![
                    StatLine {
                        label_key: "STAT_COMMON_RELIABILITY",
                        fallback_label: "Reliability:",
                        value: reliability,
                        detail: reliability_detail,
                    },
                    StatLine {
                        label_key: "STAT_COMMON_FUEL_CONSUMPTION",
                        fallback_label: "Fuel Usage:",
                        value: fuel,
                        detail: fuel_detail,
                    },
                    unrecorded_stat("STAT_NAVY_SURFACE_DETECTION", "Surface Detection:"),
                    unrecorded_stat("STAT_NAVY_SUB_DETECTION", "Sub Detection:"),
                    unrecorded_stat("STAT_COMMON_NIGHT_PENALTY", "Night Penalty:"),
                    unrecorded_stat("STAT_NAVY_MINES_PLANTING", "Minelaying:"),
                    unrecorded_stat("STAT_NAVY_MINES_SWEEPING", "Minesweeping:"),
                ],
            }
        }
        DesignerKind::Upgrade(domain) => {
            let mut rows = Vec::new();
            match domain {
                UpgradeDomain::Naval => {
                    rows.push(unrecorded_stat("STAT_NAVY_MAXIMUM_SPEED", "Max Speed:"));
                }
                UpgradeDomain::Land | UpgradeDomain::Air => {
                    let (speed, speed_detail) =
                        recorded_stat(stats, VariantStatsRow::MAXIMUM_SPEED, |raw| {
                            format!("{} km/h", format_fixed5(raw, 1))
                        });
                    rows.push(StatLine {
                        label_key: "STAT_COMMON_MAXIMUM_SPEED",
                        fallback_label: "Max Speed:",
                        value: speed,
                        detail: speed_detail,
                    });
                }
            }
            let (reliability, reliability_detail) =
                recorded_stat(stats, VariantStatsRow::RELIABILITY, |raw| {
                    format_percent_fixed5(raw, 1)
                });
            rows.push(StatLine {
                label_key: "STAT_COMMON_RELIABILITY",
                fallback_label: "Reliability:",
                value: reliability,
                detail: reliability_detail,
            });
            let (fuel, fuel_detail) =
                recorded_stat(stats, VariantStatsRow::FUEL_CONSUMPTION, |raw| {
                    format_fixed5(raw, 2)
                });
            rows.push(StatLine {
                label_key: "STAT_COMMON_FUEL_CONSUMPTION",
                fallback_label: "Fuel Usage:",
                value: fuel,
                detail: fuel_detail,
            });
            let (cost, cost_detail) =
                recorded_stat(stats, VariantStatsRow::BUILD_COST_IC, |raw| match domain {
                    UpgradeDomain::Naval => format_fixed5(raw, 0),
                    UpgradeDomain::Land | UpgradeDomain::Air => format_fixed5(raw, 2),
                });
            rows.push(StatLine {
                label_key: "STAT_COMMON_BUILD_COST_IC",
                fallback_label: "Production Cost:",
                value: cost,
                detail: cost_detail,
            });
            if matches!(domain, UpgradeDomain::Air) {
                let (range, range_detail) =
                    recorded_stat(stats, VariantStatsRow::AIR_RANGE, |raw| {
                        format!("{} km", format_fixed5(raw, 0))
                    });
                let (air_atk, air_atk_detail) =
                    recorded_stat(stats, VariantStatsRow::AIR_ATTACK, |raw| {
                        format_fixed5(raw, 1)
                    });
                let (air_def, air_def_detail) =
                    recorded_stat(stats, VariantStatsRow::AIR_DEFENCE, |raw| {
                        format_fixed5(raw, 1)
                    });
                let (agility, agility_detail) =
                    recorded_stat(stats, VariantStatsRow::AIR_AGILITY, |raw| {
                        format_fixed5(raw, 1)
                    });
                rows.push(StatLine {
                    label_key: "STAT_AIR_RANGE",
                    fallback_label: "Range:",
                    value: range,
                    detail: range_detail,
                });
                rows.push(StatLine {
                    label_key: "STAT_AIR_ATTACK",
                    fallback_label: "Air Attack:",
                    value: air_atk,
                    detail: air_atk_detail,
                });
                rows.push(StatLine {
                    label_key: "STAT_AIR_DEFENCE",
                    fallback_label: "Air Defense:",
                    value: air_def,
                    detail: air_def_detail,
                });
                rows.push(StatLine {
                    label_key: "STAT_AIR_AGILITY",
                    fallback_label: "Agility:",
                    value: agility,
                    detail: agility_detail,
                });
            }
            StatColumns {
                base: rows,
                combat: Vec::new(),
                misc: Vec::new(),
            }
        }
    }
}

pub fn compute_read_hour(request_hour: i32, samples: &[(TableId, i32)]) -> (i32, Option<String>) {
    let Some(min_hour) = samples.iter().map(|(_, hour)| *hour).min() else {
        return (request_hour, None);
    };
    let max_hour = samples.iter().map(|(_, hour)| *hour).max().unwrap_or(min_hour);
    let warning = (min_hour < max_hour).then(|| {
        let earlier: Vec<String> = samples
            .iter()
            .filter(|(_, hour)| *hour < max_hour)
            .map(|(table, hour)| format!("{table:?} at hour {hour}"))
            .collect();
        format!(
            "Some design tables were sampled earlier than hour {max_hour}: {}.",
            earlier.join(", ")
        )
    });
    (min_hour, warning)
}

#[allow(
    clippy::too_many_arguments,
    reason = "rendering resolves recorded tables, content layers, decode settings, generation, language, warnings, and cancellation independently"
)]
pub fn render(
    service: &Service,
    source: &Source,
    content: &Arc<ResolvedContent>,
    layers: &ResolvedLayers,
    resolve: &ResolveOptions,
    visual_revision: &str,
    language: &str,
    decode: Decode,
    admitted_gen: (u64, u64),
    mut extra_warnings: Vec<String>,
    request: GameWindowRequest,
    cancel: &AtomicBool,
) -> Result<GameWindowRender, GameWindowError> {
    let target: Target = serde_json::from_value(request.target.clone())
        .map_err(|error| GameWindowError::refused(format!("Invalid equipment designer target: {error}")))?;
    let variant_id: u64 = target
        .variant
        .parse()
        .map_err(|_| GameWindowError::refused(format!("Invalid equipment design id '{}'.", target.variant)))?;
    let branch_id: u64 = request
        .branch_id
        .parse()
        .map_err(|_| GameWindowError::refused(format!("Invalid branch id '{}'.", request.branch_id)))?;

    let frame = match source
        .replay_ref()
        .seek_selected(branch_id, request.hour, Some(REQUIRED_TABLES))
        .map_err(|error| {
            GameWindowError::failed(format!(
                "Could not seek recording to hour {}: {error}",
                request.hour
            ))
        })? {
        SeekResult::Frame(frame) => frame,
        SeekResult::Gap { reason, .. } => {
            return Err(GameWindowError::refused(format!(
                "Hour {} is not recorded on this branch: {reason}",
                request.hour
            )));
        }
    };

    let definitions = frame
        .complete::<tables::VariantDefinitions>()
        .map_err(|error| {
            GameWindowError::refused(format!(
                "Equipment definitions are not recorded at this hour: {error}"
            ))
        })?;
    let definition = definitions
        .binary_search_by_key(&variant_id, |row| row.id)
        .ok()
        .map(|index| &definitions[index])
        .ok_or_else(|| {
            GameWindowError::refused(format!(
                "Equipment design {} is not recorded at hour {}.",
                target.variant, request.hour
            ))
        })?;
    let lexical = frame
        .complete::<tables::LexicalTokens>()
        .map_err(|error| {
            GameWindowError::refused(format!(
                "Lexical tokens are not recorded at this hour: {error}"
            ))
        })?;
    let (dlc_mask, dlc_unavailable) = match frame.complete::<tables::SimulationFeatures>() {
        Ok(rows) => match rows.first() {
            Some(row) => (Some(u64::from(row.dlc_mask)), None),
            None => (
                None,
                Some("SimulationFeatures has no row at this hour".to_owned()),
            ),
        },
        Err(error) => (None, Some(error.to_string())),
    };

    let equipment_db = Arc::new(Database::load(layers).map_err(|error| {
        GameWindowError::failed(format!(
            "Could not load recorded equipment definitions: {error}"
        ))
    })?);
    let (resource_db, _) = content_reader_key_names::resources::read(layers).map_err(|error| {
        GameWindowError::failed(format!(
            "Could not load recorded resource definitions: {error}"
        ))
    })?;

    let equipment_key = spell(lexical, definition.equipment_token).ok_or_else(|| {
        GameWindowError::refused(format!(
            "Equipment design {} has an unknown equipment token {}.",
            target.variant, definition.equipment_token
        ))
    })?;
    let equipment = equipment_db.named(&equipment_key).ok_or_else(|| {
        GameWindowError::refused(format!(
            "Equipment '{equipment_key}' is not defined in the recorded mod stack."
        ))
    })?;
    let archetype_key = equipment
        .archetype
        .and_then(|index| equipment_db.equipment[index].name_str().ok())
        .map(str::to_owned)
        .or_else(|| spell(lexical, definition.archetype_token))
        .unwrap_or_else(|| equipment_key.clone());

    let kind = classify_designer(
        &equipment_db,
        equipment,
        dlc_mask,
        dlc_unavailable.as_deref(),
    )
    .map_err(GameWindowError::refused)?;
    let tag = resolve_variant_tag(&frame, definition, &target.tag);
    let (design_state, design_state_reason) = match &definition.design {
        Recorded::Value(state) => (Some(state), None),
        Recorded::NotRecorded => (
            None,
            Some("Not recorded: variant design state is not recorded in this recording.".to_owned()),
        ),
    };
    let (stats_row, recorded_stats) = match frame.complete::<tables::VariantStats>() {
        Ok(rows) => match rows.binary_search_by_key(&variant_id, |row| row.variant) {
            Ok(index) => {
                let row = &rows[index];
                (Some(row), RecordedStats::Available(row.values))
            }
            Err(_) => (None, RecordedStats::MissingRow),
        },
        Err(error) => (None, RecordedStats::Unavailable(error.to_string())),
    };
    let modules_table = frame.complete::<tables::VariantModules>();
    let module_rows = match &modules_table {
        Ok(rows) => Ok(slice_by_variant(rows, variant_id, |row| row.variant)),
        Err(error) => Err(error.to_string()),
    };
    let upgrades_table = frame.complete::<tables::VariantUpgrades>();
    let upgrade_rows = match &upgrades_table {
        Ok(rows) => Ok(slice_by_variant(rows, variant_id, |row| row.variant)),
        Err(error) => Err(error.to_string()),
    };
    let organisations_table = frame.complete::<tables::IndustrialOrganisations>();

    let mut data = extract_design_data(
        kind,
        &tag,
        &equipment_key,
        &archetype_key,
        equipment,
        &equipment_db,
        definition,
        design_state,
        design_state_reason,
        stats_row,
        recorded_stats,
        module_rows,
        upgrade_rows,
        lexical,
        organisations_table.as_ref().ok().copied(),
    );
    if let Some(group_key) = data.naming_group.as_deref()
        && let Some(loc_key) = resolve_ship_naming_group_loc_key(layers, group_key)
    {
        data.naming_group = Some(loc_key);
    }

    let mut sampled_tables = Vec::new();
    let mut record_sample = |id: TableId| {
        if let Some(hour) = frame.tables[id.index()].sampled_at() {
            sampled_tables.push((id, hour));
        }
    };
    record_sample(TableId::VariantDefinitions);
    record_sample(TableId::LexicalTokens);
    if matches!(data.stats, RecordedStats::Available(_) | RecordedStats::MissingRow) {
        record_sample(TableId::VariantStats);
    }
    if kind.is_modular() && modules_table.is_ok() {
        record_sample(TableId::VariantModules);
    }
    if matches!(kind, DesignerKind::Tank | DesignerKind::Upgrade(_)) && upgrades_table.is_ok() {
        record_sample(TableId::VariantUpgrades);
    }
    if data.mio.is_some() && organisations_table.is_ok() {
        record_sample(TableId::IndustrialOrganisations);
    }
    if !equipment.module_slots.is_empty() && dlc_mask.is_some() {
        record_sample(TableId::SimulationFeatures);
    }
    if frame.complete::<tables::Countries>().is_ok() {
        record_sample(TableId::Countries);
    }
    if frame.complete::<tables::CountryLinks>().is_ok() {
        record_sample(TableId::CountryLinks);
    }
    if frame.complete::<tables::CountryAppearance>().is_ok() {
        record_sample(TableId::CountryAppearance);
    }
    let (read_hour, mixed_hour_warning) = compute_read_hour(request.hour, &sampled_tables);
    if let Some(warning) = mixed_hour_warning {
        extra_warnings.push(warning);
    }
    if let Some(reason) = &data.design_state_reason {
        extra_warnings.push(reason.clone());
    }
    if let RecordedModules::Unavailable(reason) = &data.fitted_modules
        && kind.is_modular()
    {
        extra_warnings.push(format!(
            "Module slots are unknown because VariantModules is unavailable at this hour ({reason})."
        ));
    }
    if let RecordedUpgrades::Unavailable { reason, .. } = &data.upgrades
        && matches!(kind, DesignerKind::Tank | DesignerKind::Upgrade(_))
    {
        extra_warnings.push(format!(
            "Upgrade levels are unknown because VariantUpgrades is unavailable at this hour ({reason})."
        ));
    }

    let window_name = kind.window_name();
    let cache_key = WindowCacheKey {
        source_id: request.source_id.clone(),
        revision: request.revision.clone(),
        game_dir: resolve.game_dir.clone(),
        user_dir: resolve.user_dir.clone(),
        visual_revision: visual_revision.to_owned(),
        language: language.to_owned(),
        force_pow2_textures: decode.force_pow2_textures,
        window: window_name.to_owned(),
    };
    struct ResolvedVisuals {
        equipment_icon: Option<Icon>,
        blueprint_name: Option<String>,
        mio_sprite: Option<String>,
        prep_warnings: Vec<String>,
    }

    let empty_tooltips = BTreeMap::new();
    let recorded_language = language.to_owned();

    service.render_composed_from_layers_at(
        cache_key,
        layers,
        window_name,
        request.scale,
        Some(read_hour),
        true,
        &["close_button"],
        &empty_tooltips,
        extra_warnings,
        Some(admitted_gen),
        cancel,
        |shared| {
            let guis = shared.guis.as_ref();
            let registry = shared.registry.as_ref();
            let localisation = shared.localisation.as_ref();
            let mut extra = Assets::default();
            let mut visuals = ResolvedVisuals {
                equipment_icon: None,
                blueprint_name: None,
                mio_sprite: None,
                prep_warnings: Vec::new(),
            };
            add_sprite(&mut extra, "GFX_resources_strip");
            add_sprite(&mut extra, "GFX_production_cost");
            add_sprite(&mut extra, "GFX_navprod_cost");
            add_sprite(&mut extra, "GFX_EMSI_locked");
            add_sprite(&mut extra, "GFX_EMSI_empty");
            add_sprite(&mut extra, "GFX_EMI_unknown");

            let scripted = dashboard_base::catalogue::read_scripted_content(
                Arc::clone(content),
                recorded_language.clone(),
            );
            match shared.icon_content(layers, &equipment_db, &scripted) {
                Ok(icon_content) => match IconHour::read(&frame, &icon_content) {
                    Ok(icon_hour) => {
                        match variant_icon(&icon_content, &icon_hour, definition, &equipment_key) {
                            Ok(Some(icon)) => {
                                match &icon {
                                    Icon::Sprite(sprite) => add_sprite(&mut extra, sprite),
                                    Icon::Path(path) => {
                                        if !path.is_empty() {
                                            extra.textures.insert(path.as_bytes().to_vec());
                                        }
                                    }
                                }
                                visuals.equipment_icon = Some(icon);
                            }
                            Ok(None) => {}
                            Err(why) => {
                                visuals.prep_warnings.push(format!(
                                    "Equipment icon could not be resolved for '{}': {why}",
                                    equipment_key
                                ));
                            }
                        }
                    }
                    Err(error) => {
                        visuals.prep_warnings.push(format!(
                            "Equipment icon hour context is unavailable: {error}"
                        ));
                    }
                },
                Err(why) => {
                    visuals.prep_warnings.push(format!(
                        "Equipment icon rules are unavailable: {why}"
                    ));
                }
            }

            if let Some(mio) = &data.mio {
                if let Some(sprite) = &mio.sprite {
                    if registry.sprite(sprite.as_bytes()).is_some() {
                        add_sprite(&mut extra, sprite);
                        visuals.mio_sprite = Some(sprite.clone());
                    } else {
                        visuals.prep_warnings.push(format!(
                            "Manufacturer '{}' emblem sprite '{}' is missing from the loaded interface registry.",
                            loc_or(localisation, &mio.name),
                            sprite
                        ));
                    }
                }
            }

            if kind.is_modular() {
                let entry = guis
                    .template(b"equipment_designer_module_slot_entry")
                    .ok_or_else(|| {
                        GameWindowError::refused(
                            "Required template 'equipment_designer_module_slot_entry' is missing from the loaded interface files.",
                        )
                    })?;
                merge_assets(
                    &mut extra,
                    native_gui::assets_in(entry, &|name| guis.template(name), localisation),
                );
                let stat_entry = guis.template(b"stat_entry").ok_or_else(|| {
                    GameWindowError::refused(
                        "Required template 'stat_entry' is missing from the loaded interface files.",
                    )
                })?;
                merge_assets(
                    &mut extra,
                    native_gui::assets_in(stat_entry, &|name| guis.template(name), localisation),
                );
                if let Some(bp_name) = resolve_blueprint_window(&data, guis) {
                    if let Some(bp_root) = guis.template(bp_name.as_bytes()) {
                        merge_assets(
                            &mut extra,
                            native_gui::assets_in(
                                bp_root,
                                &|name| guis.template(name),
                                localisation,
                            ),
                        );
                    }
                    visuals.blueprint_name = Some(bp_name);
                }
                if let RecordedModules::Available(fitted_map) = &data.fitted_modules {
                    for (slot_key, slot_gfx) in &data.module_slots {
                        if let Some(fitted) = fitted_map.get(slot_key) {
                            let module_gfx = equipment_db
                                .module_design(&fitted.module)
                                .and_then(|design| std::str::from_utf8(&design.gfx).ok())
                                .unwrap_or("");
                            let sprite =
                                resolve_fitted_module_sprite(&fitted.module, module_gfx, |s| {
                                    registry.sprite(s.as_bytes()).is_some()
                                });
                            add_sprite(&mut extra, &sprite);
                        } else {
                            let sprite = resolve_empty_slot_sprite(slot_key, slot_gfx, |s| {
                                registry.sprite(s.as_bytes()).is_some()
                            });
                            add_sprite(&mut extra, &sprite);
                        }
                    }
                }
            } else if let DesignerKind::Upgrade(domain) = kind {
                let sub_name = domain.sub_window();
                let sub = guis.template(sub_name.as_bytes()).ok_or_else(|| {
                    GameWindowError::refused(format!(
                        "Required upgrade designer sub-window '{sub_name}' is missing from the loaded interface files."
                    ))
                })?;
                merge_assets(
                    &mut extra,
                    native_gui::assets_in(sub, &|name| guis.template(name), localisation),
                );
                let stat_entry = guis
                    .template(b"equipment_designer_stat_entry")
                    .ok_or_else(|| {
                        GameWindowError::refused(
                            "Required template 'equipment_designer_stat_entry' is missing from the loaded interface files.",
                        )
                    })?;
                merge_assets(
                    &mut extra,
                    native_gui::assets_in(stat_entry, &|name| guis.template(name), localisation),
                );
            }

            Ok((extra, visuals))
        },
        |root, _guis, resources, localisation, instance, tooltips, visuals| {
            configure_main_window(
                root,
                resources,
                localisation,
                instance,
                &data,
                &resource_db,
                visuals.equipment_icon.as_ref(),
                visuals.mio_sprite.as_deref(),
                tooltips,
            );
        },
        |main_layout, root, guis, resources, localisation, visuals| {
            let mut plan = compose_sub_windows(
                main_layout,
                root,
                guis,
                resources,
                localisation,
                &data,
                &equipment_db,
                &resource_db,
                visuals.blueprint_name.as_deref(),
                visuals.equipment_icon.as_ref(),
            )?;
            plan.warnings.extend(visuals.prep_warnings.iter().cloned());
            Ok(plan)
        },
    )
}

fn spell(lexical: &[LexicalTokenRow], token: u32) -> Option<String> {
    if token == 0 {
        return None;
    }
    let index = lexical.binary_search_by_key(&token, |row| row.token).ok()?;
    let bytes = c_string(&lexical[index].literal);
    std::str::from_utf8(bytes)
        .ok()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn slice_by_variant<T>(rows: &[T], variant_id: u64, key: impl Fn(&T) -> u64) -> &[T] {
    let start = rows.partition_point(|row| key(row) < variant_id);
    let end = rows.partition_point(|row| key(row) <= variant_id);
    &rows[start..end]
}

fn resolve_variant_tag(
    frame: &replay_frame::frame::DecodedFrame,
    definition: &VariantDefinitionRow,
    fallback: &str,
) -> String {
    let links = frame.complete::<tables::CountryLinks>().ok();
    let countries = frame.complete::<tables::Countries>().ok();
    let appearances = frame.complete::<tables::CountryAppearance>().ok();
    let link_id = if definition.origin_link != 0 {
        definition.origin_link
    } else {
        definition.creator_link
    };
    if let (Some(links), Some(countries)) = (links, countries)
        && let Some(slot) = link_slot(links, link_id)
    {
        if let Some(apps) = appearances
            && let Ok(index) = apps.binary_search_by_key(&slot, |row| row.country)
            && let Ok(tag) = std::str::from_utf8(c_string(&apps[index].name_tag))
            && !tag.is_empty()
        {
            return tag.to_owned();
        }
        if let Some(country) = countries.iter().find(|row| row.canonical_slot == slot)
            && let Ok(tag) = std::str::from_utf8(c_string(&country.tag))
            && !tag.is_empty()
        {
            return tag.to_owned();
        }
    }
    fallback.to_owned()
}

#[allow(
    clippy::too_many_arguments,
    reason = "assembling the view model gathers independent replay tables and content rows"
)]
fn extract_design_data(
    kind: DesignerKind,
    tag: &str,
    equipment_key: &str,
    archetype_key: &str,
    equipment: &Equipment,
    equipment_db: &Database,
    definition: &VariantDefinitionRow,
    design_state: Option<&VariantDesignState>,
    design_state_reason: Option<String>,
    stats_row: Option<&VariantStatsRow>,
    stats: RecordedStats,
    module_rows: Result<&[VariantModuleRow], String>,
    upgrade_rows: Result<&[VariantUpgradeRow], String>,
    lexical: &[LexicalTokenRow],
    organisations: Option<&[IndustrialOrganisationRow]>,
) -> EquipmentDesignData {
    let base_name = std::str::from_utf8(c_string(&definition.display_name))
        .ok()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
        .unwrap_or_default();
    let version_suffix = std::str::from_utf8(c_string(&definition.variant_name))
        .ok()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
        .unwrap_or_default();

    let naming_group = design_state
        .and_then(|state| state.division_names_group.as_deref())
        .and_then(|bytes| std::str::from_utf8(c_string(bytes)).ok())
        .filter(|text| !text.is_empty())
        .map(str::to_owned);

    let mio_id = design_state
        .map(|state| state.design_team)
        .filter(|&id| id != 0)
        .or_else(|| stats_row.map(|row| row.organisation).filter(|&id| id != 0));
    let mio = mio_id.and_then(|id| {
        let rows = organisations?;
        let row = rows
            .binary_search_by_key(&id, |entry| entry.organisation)
            .ok()
            .map(|index| &rows[index])?;
        let token = spell(lexical, row.token).unwrap_or_default();
        let raw_name = std::str::from_utf8(c_string(&row.name))
            .ok()
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| token.clone());
        let icon = std::str::from_utf8(c_string(&row.icon))
            .ok()
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
            .or_else(|| (!token.is_empty()).then(|| format!("GFX_idea_{token}")));
        Some(ResolvedMio {
            name: raw_name,
            sprite: icon,
        })
    });

    let module_slots: Vec<(String, String)> = equipment
        .module_slots
        .iter()
        .enumerate()
        .map(|(index, raw_name)| {
            let slot_name = String::from_utf8_lossy(raw_name).into_owned();
            let slot_gfx = equipment
                .module_slot_rules
                .get(index)
                .and_then(|rule| std::str::from_utf8(&rule.gfx).ok())
                .unwrap_or("")
                .to_owned();
            (slot_name, slot_gfx)
        })
        .collect();

    let fitted_modules = match module_rows {
        Ok(rows) => {
            let mut map = BTreeMap::new();
            for row in rows {
                if row.definition_active == 0 {
                    continue;
                }
                let Some(slot) = spell(lexical, row.slot) else {
                    continue;
                };
                let Some(module) = spell(lexical, row.definition_token) else {
                    continue;
                };
                let design = equipment_db.module_design(&module);
                let category = design
                    .and_then(|d| std::str::from_utf8(&d.category).ok())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let gui_category = design
                    .and_then(|d| std::str::from_utf8(&d.gui_category).ok())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                map.insert(
                    slot,
                    FittedModule {
                        module,
                        category,
                        gui_category,
                    },
                );
            }
            RecordedModules::Available(map)
        }
        Err(reason) => RecordedModules::Unavailable(reason),
    };

    let inherited_upgrades = collect_equipment_upgrades(equipment, equipment_db);
    let upgrades = match upgrade_rows {
        Ok(rows) => {
            let mut upgrade_levels: HashMap<String, u8> = HashMap::new();
            for row in rows {
                if let Ok(token) = u32::try_from(row.definition_token)
                    && let Some(key) = spell(lexical, token)
                {
                    upgrade_levels.insert(key.to_ascii_lowercase(), row.level);
                }
            }
            RecordedUpgrades::Available(
                inherited_upgrades
                    .into_iter()
                    .map(|key| {
                        let level = upgrade_levels
                            .get(&key.to_ascii_lowercase())
                            .copied()
                            .unwrap_or(0);
                        (key, level)
                    })
                    .collect(),
            )
        }
        Err(reason) => RecordedUpgrades::Unavailable {
            keys: inherited_upgrades,
            reason,
        },
    };

    let build_cost = stats.lane(VariantStatsRow::BUILD_COST_IC);
    let build_resources = design_state.map(|state| {
        state
            .build_resources
            .iter()
            .filter(|entry| entry.value > 0)
            .filter_map(|entry| {
                let token = spell(lexical, entry.index)?;
                Some((token, entry.value))
            })
            .collect()
    });

    let niche_frame = design_state.map(|state| state.role_icon_index.max(1));

    EquipmentDesignData {
        kind,
        tag: tag.to_owned(),
        equipment_key: equipment_key.to_owned(),
        archetype_key: archetype_key.to_owned(),
        air_map_icon_frame: collect_air_map_icon_frame(equipment_key, equipment, equipment_db),
        category_mask: equipment_db.category(equipment),
        base_name,
        version_suffix,
        append_version: definition.append_variant_name != 0,
        auto_upgraded: design_state.map(|state| state.auto_upgraded != 0),
        design_state_reason,
        naming_group,
        niche_frame,
        mio,
        module_slots,
        fitted_modules,
        upgrades,
        stats,
        manpower: design_state.map(|state| state.manpower),
        build_cost,
        build_resources,
    }
}

fn add_sprite(assets: &mut Assets, name: &str) {
    if !name.is_empty() {
        assets.sprites.insert(name.as_bytes().to_vec());
    }
}

fn merge_assets(target: &mut Assets, added: Assets) {
    target.sprites.extend(added.sprites);
    target.fonts.extend(added.fonts);
    target.textures.extend(added.textures);
}

fn collect_air_map_icon_frame(
    equipment_key: &str,
    equipment: &Equipment,
    database: &Database,
) -> i32 {
    if equipment.air_map_icon_frame > 0 {
        return equipment.air_map_icon_frame;
    }
    let stem = equipment_key
        .trim_end_matches(|ch: char| ch.is_ascii_digit())
        .trim_end_matches('_');
    if !stem.is_empty()
        && stem != equipment_key
        && let Some(base) = database.named(stem)
        && base.air_map_icon_frame > 0
    {
        return base.air_map_icon_frame;
    }
    let mut current = equipment;
    let mut visited = Vec::new();
    loop {
        if current.air_map_icon_frame > 0 {
            return current.air_map_icon_frame;
        }
        let parent = &database.equipment[current.parent];
        if !std::ptr::eq(parent, current) && !visited.contains(&current.parent) {
            visited.push(current.parent);
            current = parent;
            continue;
        }
        if let Some(arch_idx) = current.archetype {
            let arch = &database.equipment[arch_idx];
            if !std::ptr::eq(arch, current) && !visited.contains(&arch_idx) {
                visited.push(arch_idx);
                current = arch;
                continue;
            }
        }
        break;
    }
    0
}

fn collect_equipment_upgrades(equipment: &Equipment, database: &Database) -> Vec<String> {
    if !equipment.upgrades.is_empty() {
        return equipment
            .upgrades
            .iter()
            .map(|raw| String::from_utf8_lossy(raw).into_owned())
            .collect();
    }
    let mut current = equipment;
    let mut visited = Vec::new();
    loop {
        if !current.upgrades.is_empty() {
            return current
                .upgrades
                .iter()
                .map(|raw| String::from_utf8_lossy(raw).into_owned())
                .collect();
        }
        let parent = &database.equipment[current.parent];
        if !std::ptr::eq(parent, current) && !visited.contains(&current.parent) {
            visited.push(current.parent);
            current = parent;
            continue;
        }
        if let Some(arch_idx) = current.archetype {
            let arch = &database.equipment[arch_idx];
            if !std::ptr::eq(arch, current) && !visited.contains(&arch_idx) {
                visited.push(arch_idx);
                current = arch;
                continue;
            }
        }
        break;
    }
    Vec::new()
}

fn resolve_ship_naming_group_loc_key(layers: &ResolvedLayers, group_key: &str) -> Option<String> {
    let files = layers
        .directory_files("common/units/names_ships", ".txt")
        .ok()?;
    for path in files {
        let Ok(bytes) = path.read(layers) else {
            continue;
        };
        let Ok(doc) = content_reader_parser::parser::parse(&bytes) else {
            continue;
        };
        for item in &doc.items {
            if item.keyword_is(group_key)
                && let Some(fields) = item.value.items()
            {
                for field in fields {
                    if field.keyword_is("name")
                        && let Some(raw) = field.value.read_string()
                        && let Ok(text) = std::str::from_utf8(raw)
                        && !text.is_empty()
                    {
                        return Some(text.to_owned());
                    }
                }
            }
        }
    }
    None
}

fn resolve_blueprint_window(data: &EquipmentDesignData, guis: &native::Guis) -> Option<String> {
    let tag_lower = data.tag.to_ascii_lowercase();
    let candidates = [
        format!("equipment_designer_{}_{tag_lower}", data.equipment_key),
        format!("equipment_designer_{}", data.equipment_key),
        format!("equipment_designer_{}_{tag_lower}", data.archetype_key),
        format!("equipment_designer_{}", data.archetype_key),
    ];
    candidates
        .into_iter()
        .find(|name| guis.template(name.as_bytes()).is_some())
}

fn expand_loc_refs(localisation: &Localisation, text: &str, depth: usize) -> String {
    if depth == 0 || !text.contains('$') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('$') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(end) = after.find('$') {
            let token = &after[..end];
            let key = token.split('|').next().unwrap_or(token);
            if !key.is_empty()
                && let Some(resolved) = localisation.get(key)
            {
                let clean = strip_markup(resolved);
                out.push_str(&expand_loc_refs(localisation, &clean, depth - 1));
            } else {
                out.push('$');
                out.push_str(token);
                out.push('$');
            }
            rest = &after[end + 1..];
        } else {
            out.push('$');
            rest = after;
            break;
        }
    }
    out.push_str(rest);
    out
}

fn loc_or<'a>(localisation: &'a Localisation, key: &'a str) -> String {
    localisation
        .get(key)
        .map(|raw| expand_loc_refs(localisation, &strip_markup(raw), 4))
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| key.to_owned())
}

fn loc_for_tag(localisation: &Localisation, tag: &str, key: &str) -> String {
    if !tag.is_empty() {
        let prefixed = format!("{}_{key}", tag.to_ascii_uppercase());
        if let Some(text) = localisation.get(&prefixed) {
            let clean = strip_markup(text);
            if !clean.is_empty() {
                return clean;
            }
        }
    }
    loc_or(localisation, key)
}

fn resolve_display_name(data: &EquipmentDesignData, localisation: &Localisation) -> String {
    if !data.base_name.is_empty() {
        localisation
            .get(&data.base_name)
            .map(strip_markup)
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| data.base_name.clone())
    } else {
        loc_for_tag(localisation, &data.tag, &data.equipment_key)
    }
}

fn has_category(mask: u64, name: &str) -> bool {
    category_mask(name).is_some_and(|bit| bit != 0 && mask & bit != 0)
}

pub fn resolve_tank_role_and_frame(
    archetype_key: &str,
    category: u64,
    localisation: &Localisation,
) -> Option<(String, Option<i32>)> {
    if has_category(category, "flame") {
        return Some((loc_or(localisation, "tank_designer_flame"), Some(10)));
    }
    if has_category(category, "amphibious") {
        return Some((loc_or(localisation, "amphibious_armor"), Some(6)));
    }
    let is_aa = has_category(category, "anti_air");
    let is_art = has_category(category, "artillery");
    let is_at = has_category(category, "anti_tank");
    let (loc_key, frame) = match archetype_key {
        "light_tank_chassis" => {
            if is_aa {
                ("light_sp_anti_air_brigade", Some(3))
            } else if is_art {
                ("light_sp_artillery_brigade", Some(2))
            } else if is_at {
                ("light_tank_destroyer_brigade", Some(1))
            } else {
                ("light_armor", Some(7))
            }
        }
        "medium_tank_chassis" => {
            if is_aa {
                ("medium_sp_anti_air_brigade", Some(3))
            } else if is_art {
                ("medium_sp_artillery_brigade", Some(2))
            } else if is_at {
                ("medium_tank_destroyer_brigade", Some(1))
            } else {
                ("medium_armor", Some(9))
            }
        }
        "heavy_tank_chassis" => {
            if is_aa {
                ("heavy_sp_anti_air_brigade", Some(3))
            } else if is_art {
                ("heavy_sp_artillery_brigade", Some(2))
            } else if is_at {
                ("heavy_tank_destroyer_brigade", Some(1))
            } else {
                ("heavy_armor", Some(8))
            }
        }
        "super_heavy_tank_chassis" => {
            if is_aa {
                ("super_heavy_sp_anti_air_brigade", Some(3))
            } else if is_art {
                ("super_heavy_sp_artillery_brigade", Some(2))
            } else if is_at {
                ("super_heavy_tank_destroyer_brigade", Some(1))
            } else {
                ("super_heavy_armor", Some(8))
            }
        }
        "modern_tank_chassis" => {
            if is_aa {
                ("modern_sp_anti_air_brigade", Some(3))
            } else if is_art {
                ("modern_sp_artillery_brigade", Some(2))
            } else if is_at {
                ("modern_tank_destroyer_brigade", Some(1))
            } else {
                ("modern_armor", Some(9))
            }
        }
        "amphibious_tank_chassis" => ("amphibious_armor", Some(6)),
        "land_cruiser_chassis" => ("land_cruiser", None),
        _ => return None,
    };
    Some((loc_or(localisation, loc_key), frame))
}

pub fn resolve_plane_role(category: u64, localisation: &Localisation) -> Option<String> {
    let loc_key = if has_category(category, "cas") {
        "plane_designer_cas"
    } else if has_category(category, "naval_bomber") {
        "plane_designer_naval_bomber"
    } else if has_category(category, "maritime_patrol_plane") {
        "plane_designer_maritime_patrol_plane"
    } else if has_category(category, "scout_plane") {
        "plane_designer_scout_plane"
    } else if has_category(category, "tactical_bomber") {
        "plane_designer_tactical_bomber"
    } else if has_category(category, "strategic_bomber") {
        "plane_designer_strategic_bomber"
    } else if has_category(category, "heavy_fighter") {
        "plane_designer_heavy_fighter"
    } else if has_category(category, "suicide") {
        "plane_designer_suicide_craft"
    } else if has_category(category, "interceptor") || has_category(category, "fighter") {
        "plane_designer_fighter"
    } else {
        return None;
    };
    Some(loc_or(localisation, loc_key))
}

pub fn format_resources_text(
    build_resources: Option<&[(String, i64)]>,
    resource_db: &ResourceDatabase,
    localisation: &Localisation,
    warnings: &mut Vec<String>,
) -> String {
    let Some(resources) = build_resources else {
        return NOT_RECORDED_VALUE.to_owned();
    };
    let mut out = String::new();
    let numbered = resource_db.numbered();
    for (token, raw_amount) in resources {
        let amount = (*raw_amount as f64 / FIXED5_SCALE).round() as i64;
        if amount <= 0 {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        if let Some(resource) = numbered
            .iter()
            .find(|entry| entry.token.eq_ignore_ascii_case(token))
            && let Some(frame) = resource.icon_frame
        {
            out.push_str(&format!("{amount}{ICON}resources_strip|{frame}"));
        } else {
            let label = loc_or(localisation, token);
            out.push_str(&format!("{amount} {label}"));
            warnings.push(format!(
                "Resource '{token}' has no icon frame in resources_strip; showing text amount."
            ));
        }
    }
    out
}

fn loc_fallback(localisation: &Localisation, key: &str, fallback: &str) -> String {
    localisation
        .get(key)
        .map(|raw| expand_loc_refs(localisation, &strip_markup(raw), 4))
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| fallback.to_owned())
}

fn format_production_cost(
    kind: DesignerKind,
    build_cost: Option<i64>,
    localisation: &Localisation,
) -> String {
    let label = loc_fallback(localisation, "STAT_COMMON_BUILD_COST_IC", "Production Cost:");
    let Some(cost) = build_cost else {
        return format!("{label} {NOT_RECORDED_VALUE}");
    };
    match kind {
        DesignerKind::Ship => {
            format!("{label} {ICON}navprod_cost{ICON} {}", format_fixed5(cost, 0))
        }
        DesignerKind::Tank | DesignerKind::Plane | DesignerKind::Upgrade(_) => {
            format!("{label} {ICON}production_cost {}", format_fixed5(cost, 2))
        }
    }
}

fn apply_equipment_icon(
    instance: &mut Instance,
    root: &Element,
    control_name: &str,
    icon: Option<&Icon>,
    resources: &dyn Resources,
) {
    let Some(icon) = icon else {
        return;
    };
    let Some(control) = find_direct(root, control_name) else {
        return;
    };
    let Ok(bound) = instance.control(root, control) else {
        return;
    };
    match icon {
        Icon::Sprite(name) => {
            bound.sprite = Some(name.as_bytes().to_vec());
        }
        Icon::Path(path) => {
            if let Some(TextureFile::Loaded(texture)) = resources.texture_file(path.as_bytes()) {
                bound.texture = Some(texture);
            }
        }
    }
}

fn element_name(element: &Element) -> &str {
    std::str::from_utf8(element.name_bytes()).unwrap_or("")
}

fn find_child<'a>(parent: &'a Element, segment: &str) -> Option<&'a Element> {
    if let Some(found) = parent
        .children
        .iter()
        .find(|child| child.is_named(segment.as_bytes()))
    {
        return Some(found);
    }
    if let Fields::DropDown(dd) = &parent.fields {
        if let Some(expand) = dd.expand_button.as_deref()
            && expand.is_named(segment.as_bytes())
        {
            return Some(expand);
        }
        if let Some(found) = dd
            .inner
            .children
            .iter()
            .find(|child| child.is_named(segment.as_bytes()))
        {
            return Some(found);
        }
    }
    None
}

fn find_direct<'a>(root: &'a Element, path: &str) -> Option<&'a Element> {
    let mut current = root;
    for segment in path.split('/') {
        current = find_child(current, segment)?;
    }
    Some(current)
}

fn hide_by_name(instance: &mut Instance, root: &Element, name: &str) {
    if let Some(element) = find_direct(root, name) {
        let _ = instance.hide(root, element);
    }
}

fn disable_by_name(instance: &mut Instance, root: &Element, name: &str) {
    if let Some(element) = find_direct(root, name)
        && let Ok(bound) = instance.control(root, element)
    {
        bound.enabled = Some(false);
    }
}

fn set_text_by_name(instance: &mut Instance, root: &Element, name: &str, text: String) {
    if let Some(element) = find_direct(root, name)
        && let Ok(bound) = instance.control(root, element)
    {
        bound.text = Some(text);
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "main window configuration binds independent design fields, resources, and resolved sprites"
)]
fn configure_main_window(
    root: &Element,
    resources: &dyn Resources,
    localisation: &Localisation,
    instance: &mut Instance,
    data: &EquipmentDesignData,
    resource_db: &ResourceDatabase,
    equipment_icon: Option<&Icon>,
    mio_sprite: Option<&str>,
    tooltips: &mut BTreeMap<String, GameWindowTooltip>,
) {
    let display_name = resolve_display_name(data, localisation);
    let version_suffix = if data.version_suffix.is_empty() {
        String::new()
    } else {
        localisation
            .get(&data.version_suffix)
            .map(strip_markup)
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| data.version_suffix.clone())
    };
    let mio_role_body = loc_fallback(
        localisation,
        "MILITARY_INDUSTRIAL_ORGANIZATIONS",
        "Assigned Military Industrial Organization",
    );

    if data.kind.is_modular() {
        for modal in [
            "module_selector_window",
            "equipment_select_model",
            "equipment_select_sprite",
            "historical_select_design",
            "niche_icon_selector",
            "designer_intel_container",
            "referenced_info",
            "select_model",
            "ship_category_icon",
        ] {
            hide_by_name(instance, root, modal);
        }

        set_text_by_name(
            instance,
            root,
            "designer_title",
            loc_or(localisation, data.kind.title_key()),
        );
        set_text_by_name(instance, root, "experience", String::new());
        set_text_by_name(instance, root, "refit_cost", String::new());
        let mut resource_warnings = Vec::new();
        set_text_by_name(
            instance,
            root,
            "production_resources",
            format_resources_text(
                data.build_resources.as_deref(),
                resource_db,
                localisation,
                &mut resource_warnings,
            ),
        );
        if data.build_resources.is_none()
            && let Some(res_el) = find_direct(root, "production_resources")
        {
            tooltips.insert(
                res_el.path.clone(),
                GameWindowTooltip {
                    title: loc_fallback(localisation, "RESOURCES", "Resources"),
                    body: Some(
                        data.design_state_reason
                            .clone()
                            .unwrap_or_else(|| NOT_RECORDED_TOOLTIP.to_owned()),
                    ),
                    detail: None,
                },
            );
        }
        set_text_by_name(
            instance,
            root,
            "production_value",
            format_production_cost(data.kind, data.build_cost, localisation),
        );
        if data.build_cost.is_none()
            && let Some(cost_el) = find_direct(root, "production_value")
        {
            tooltips.insert(
                cost_el.path.clone(),
                GameWindowTooltip {
                    title: loc_fallback(
                        localisation,
                        "STAT_COMMON_BUILD_COST_IC",
                        "Production Cost",
                    )
                    .trim_end_matches(':')
                    .trim()
                    .to_owned(),
                    body: Some(data.stats.missing_reason()),
                    detail: None,
                },
            );
        }

        set_text_by_name(instance, root, "equipments/name", display_name);
        hide_by_name(instance, root, "equipments/name_reset");
        set_text_by_name(instance, root, "position_version", version_suffix);

        if let Some(check) = find_direct(root, "dynamic_version_checkbox_button")
            && let Ok(bound) = instance.control(root, check)
        {
            bound.frame = Some(if data.append_version { 2 } else { 1 });
            bound.enabled = Some(false);
        }
        if let Some(check) = find_direct(root, "auto_upgraded_checkbox_button") {
            if let Ok(bound) = instance.control(root, check) {
                bound.frame = Some(match data.auto_upgraded {
                    Some(true) => 2,
                    Some(false) | None => 1,
                });
                bound.enabled = Some(false);
            }
            if data.auto_upgraded.is_none() {
                tooltips.insert(
                    check.path.clone(),
                    GameWindowTooltip {
                        title: loc_fallback(
                            localisation,
                            "EQUIPMENT_DESIGNER_AUTO_UPGRADED_CHECKBOX_BUTTON_DESC",
                            "Auto Upgrade",
                        )
                        .lines()
                        .next()
                        .unwrap_or("Auto Upgrade")
                        .to_owned(),
                        body: Some(
                            data.design_state_reason
                                .clone()
                                .unwrap_or_else(|| NOT_RECORDED_TOOLTIP.to_owned()),
                        ),
                        detail: None,
                    },
                );
            }
        }

        apply_equipment_icon(
            instance,
            root,
            "icon_selection_container/btn_equipment_icon",
            equipment_icon,
            resources,
        );

        if let Some(niche) = find_direct(root, "niche_button") {
            match data.niche_frame {
                Some(frame) => {
                    if let Ok(bound) = instance.control(root, niche) {
                        bound.frame = Some(frame);
                    }
                }
                None => {
                    let _ = instance.hide(root, niche);
                }
            }
        }

        let has_mio = if let Some(mio) = &data.mio {
            let body = if mio_sprite.is_some() {
                mio_role_body.clone()
            } else {
                format!("{mio_role_body} (emblem art missing in loaded interface registry)")
            };
            if let Some(team_button) = find_direct(root, "design_team_button") {
                if let Some(sprite) = mio_sprite
                    && let Ok(bound) = instance.control(root, team_button)
                {
                    bound.sprite = Some(sprite.as_bytes().to_vec());
                }
                tooltips.insert(
                    team_button.path.clone(),
                    GameWindowTooltip {
                        title: loc_or(localisation, &mio.name),
                        body: Some(body),
                        detail: None,
                    },
                );
                hide_by_name(instance, root, "design_company_icon");
            } else if let Some(company) = find_direct(root, "design_company_icon") {
                if let Some(sprite) = mio_sprite
                    && let Ok(bound) = instance.control(root, company)
                {
                    bound.sprite = Some(sprite.as_bytes().to_vec());
                }
                tooltips.insert(
                    company.path.clone(),
                    GameWindowTooltip {
                        title: loc_or(localisation, &mio.name),
                        body: Some(body),
                        detail: None,
                    },
                );
            }
            true
        } else {
            hide_by_name(instance, root, "design_company_icon");
            false
        };

        match data.kind {
            DesignerKind::Ship => {
                hide_by_name(instance, root, "nubbin");
                let group_label = data
                    .naming_group
                    .as_deref()
                    .map(|key| loc_or(localisation, key))
                    .unwrap_or_default();
                set_text_by_name(instance, root, "equipment_names/name", group_label);
                disable_by_name(instance, root, "equipment_names/expand_button");
                set_text_by_name(
                    instance,
                    root,
                    "info/ship_designer_naval_dominance_value",
                    NOT_RECORDED_VALUE.to_owned(),
                );
                if let Some(dom) = find_direct(root, "info/ship_designer_naval_dominance_value") {
                    tooltips.insert(
                        dom.path.clone(),
                        GameWindowTooltip {
                            title: loc_fallback(localisation, "NAVAL_DOMINANCE", "Naval Dominance"),
                            body: Some(NOT_RECORDED_TOOLTIP.to_owned()),
                            detail: None,
                        },
                    );
                }
                if let Some(preview_btn) = find_direct(root, "toggle_model_selector_button") {
                    tooltips.insert(
                        preview_btn.path.clone(),
                        GameWindowTooltip {
                            title: loc_fallback(localisation, "SELECT_3D_MODEL", "3D Model Preview"),
                            body: Some(PREVIEW_3D_TOOLTIP.to_owned()),
                            detail: None,
                        },
                    );
                }
            }
            DesignerKind::Tank => {
                set_text_by_name(
                    instance,
                    root,
                    "blueprint_type_name",
                    format!(
                        "{COLOR}W{}{COLOR}!",
                        loc_for_tag(localisation, &data.tag, &data.equipment_key)
                    ),
                );
                match resolve_tank_role_and_frame(&data.archetype_key, data.category_mask, localisation)
                {
                    Some((role_text, role_frame)) => {
                        set_text_by_name(instance, root, "dropdown_tank_roles/role_text", role_text);
                        if let Some(role_icon) =
                            find_direct(root, "dropdown_tank_roles/tank_role_icon")
                        {
                            match role_frame {
                                Some(frame) => {
                                    if let Ok(bound) = instance.control(root, role_icon) {
                                        bound.frame = Some(frame);
                                    }
                                }
                                None => {
                                    let _ = instance.hide(root, role_icon);
                                }
                            }
                        }
                    }
                    None => {
                        set_text_by_name(
                            instance,
                            root,
                            "dropdown_tank_roles/role_text",
                            NOT_RECORDED_VALUE.to_owned(),
                        );
                        hide_by_name(instance, root, "dropdown_tank_roles/tank_role_icon");
                        if let Some(role_el) = find_direct(root, "dropdown_tank_roles/role_text") {
                            tooltips.insert(
                                role_el.path.clone(),
                                GameWindowTooltip {
                                    title: "Not recorded".to_owned(),
                                    body: Some(format!(
                                        "Tank role could not be determined from recorded classification for archetype '{}'.",
                                        data.archetype_key
                                    )),
                                    detail: None,
                                },
                            );
                        }
                    }
                }
                disable_by_name(instance, root, "dropdown_tank_roles/expand_button");

                for slot_idx in 0..2 {
                    let container_name = format!("equipment_modules/equipment_upgrade_{slot_idx}");
                    match &data.upgrades {
                        RecordedUpgrades::Available(list) => {
                            if let Some((upgrade_key, level)) = list.get(slot_idx) {
                                set_text_by_name(
                                    instance,
                                    root,
                                    &format!("{container_name}/name"),
                                    loc_or(localisation, upgrade_key),
                                );
                                set_text_by_name(
                                    instance,
                                    root,
                                    &format!("{container_name}/level"),
                                    level.to_string(),
                                );
                                disable_by_name(instance, root, &format!("{container_name}/btn_dec"));
                                disable_by_name(instance, root, &format!("{container_name}/btn_inc"));
                            } else {
                                hide_by_name(instance, root, &container_name);
                            }
                        }
                        RecordedUpgrades::Unavailable { keys, reason } => {
                            if let Some(upgrade_key) = keys.get(slot_idx) {
                                let label = loc_or(localisation, upgrade_key);
                                set_text_by_name(
                                    instance,
                                    root,
                                    &format!("{container_name}/name"),
                                    label.clone(),
                                );
                                set_text_by_name(
                                    instance,
                                    root,
                                    &format!("{container_name}/level"),
                                    NOT_RECORDED_VALUE.to_owned(),
                                );
                                if let Some(level_el) =
                                    find_direct(root, &format!("{container_name}/level"))
                                {
                                    tooltips.insert(
                                        level_el.path.clone(),
                                        GameWindowTooltip {
                                            title: label,
                                            body: Some(format!(
                                                "Not recorded: upgrade levels are unavailable at this hour ({reason})."
                                            )),
                                            detail: None,
                                        },
                                    );
                                }
                                disable_by_name(instance, root, &format!("{container_name}/btn_dec"));
                                disable_by_name(instance, root, &format!("{container_name}/btn_inc"));
                            } else {
                                hide_by_name(instance, root, &container_name);
                            }
                        }
                    }
                }

                if let Some(preview_box) = find_direct(root, "border_container") {
                    tooltips.insert(
                        preview_box.path.clone(),
                        GameWindowTooltip {
                            title: loc_fallback(localisation, "SELECT_3D_MODEL", "3D Model Preview"),
                            body: Some(PREVIEW_3D_TOOLTIP.to_owned()),
                            detail: None,
                        },
                    );
                }
                if let Some(select_btn) = find_direct(root, "select3d_button") {
                    tooltips.insert(
                        select_btn.path.clone(),
                        GameWindowTooltip {
                            title: loc_fallback(localisation, "SELECT_3D_MODEL", "3D Model Preview"),
                            body: Some(PREVIEW_3D_TOOLTIP.to_owned()),
                            detail: None,
                        },
                    );
                }
            }
            DesignerKind::Plane => {
                set_text_by_name(
                    instance,
                    root,
                    "blueprint_type_name",
                    format!(
                        "{COLOR}W{}{COLOR}!",
                        loc_for_tag(localisation, &data.tag, &data.equipment_key)
                    ),
                );
                match resolve_plane_role(data.category_mask, localisation) {
                    Some(role_text) => {
                        set_text_by_name(instance, root, "plane_role/role_text", role_text);
                    }
                    None => {
                        set_text_by_name(
                            instance,
                            root,
                            "plane_role/role_text",
                            NOT_RECORDED_VALUE.to_owned(),
                        );
                        if let Some(role_el) = find_direct(root, "plane_role/role_text") {
                            tooltips.insert(
                                role_el.path.clone(),
                                GameWindowTooltip {
                                    title: "Not recorded".to_owned(),
                                    body: Some(format!(
                                        "Plane role could not be determined from recorded classification for '{}'.",
                                        data.equipment_key
                                    )),
                                    detail: None,
                                },
                            );
                        }
                    }
                }
                if let Some(role_icon) = find_direct(root, "plane_role/role_icon") {
                    if data.air_map_icon_frame > 0 {
                        if let Ok(bound) = instance.control(root, role_icon) {
                            bound.frame = Some(data.air_map_icon_frame * 2 - 1);
                        }
                    } else {
                        let _ = instance.hide(root, role_icon);
                    }
                }
                if let Some(preview_btn) = find_direct(root, "toggle_model_selector_button") {
                    tooltips.insert(
                        preview_btn.path.clone(),
                        GameWindowTooltip {
                            title: loc_fallback(localisation, "SELECT_3D_MODEL", "3D Model Preview"),
                            body: Some(PREVIEW_3D_TOOLTIP.to_owned()),
                            detail: None,
                        },
                    );
                }
            }
            DesignerKind::Upgrade(_) => {}
        }

        for disabled in [
            "reset_button",
            "duplicate_button",
            "auto_button",
            "auto_upgrade_button",
            "save_button",
            "save_to_file_button",
            "historical_design_button",
            "upgrade_assigned_industrial_org",
            "toggle_comparison_button",
            "toggle_model_selector_button",
            "select3d_button",
        ] {
            disable_by_name(instance, root, disabled);
        }
        if !has_mio {
            disable_by_name(instance, root, "design_team_button");
        }
    } else {
        hide_by_name(instance, root, "equipment_select_model");
        hide_by_name(instance, root, "select_model_button");

        let full_name = if data.append_version && !version_suffix.is_empty() {
            format!("{display_name} {version_suffix}")
        } else {
            display_name
        };
        set_text_by_name(instance, root, "variant_name", full_name);
        set_text_by_name(instance, root, "experience", String::new());

        if let Some(mio) = &data.mio
            && let Some(roster) = find_direct(root, "change_design_team/open_roster")
        {
            if let Some(sprite) = mio_sprite
                && let Ok(bound) = instance.control(root, roster)
            {
                bound.sprite = Some(sprite.as_bytes().to_vec());
            }
            let body = if mio_sprite.is_some() {
                mio_role_body
            } else {
                format!("{mio_role_body} (emblem art missing in loaded interface registry)")
            };
            tooltips.insert(
                roster.path.clone(),
                GameWindowTooltip {
                    title: loc_or(localisation, &mio.name),
                    body: Some(body),
                    detail: None,
                },
            );
        }

        set_text_by_name(
            instance,
            root,
            "save_button",
            loc_or(localisation, "SAVE"),
        );
        disable_by_name(instance, root, "save_button");
        disable_by_name(instance, root, "change_design_team/open_roster");
        disable_by_name(
            instance,
            root,
            "change_design_team/upgrade_assigned_industrial_org",
        );
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "sub-window composition places blueprint, module slots, upgrade panel, and stat ledgers"
)]
fn compose_sub_windows(
    main_layout: &Layout,
    root: &Element,
    guis: &native::Guis,
    resources: &dyn Resources,
    localisation: &Localisation,
    data: &EquipmentDesignData,
    equipment_db: &Database,
    resource_db: &ResourceDatabase,
    blueprint_name: Option<&str>,
    equipment_icon: Option<&Icon>,
) -> Result<ComposedPlan, GameWindowError> {
    let mut plan = ComposedPlan::default();

    if data.kind.is_modular() {
        let _ = format_resources_text(
            data.build_resources.as_deref(),
            resource_db,
            localisation,
            &mut plan.warnings,
        );
        if let Some(bp_name) = blueprint_name
            && let Some(bp_root) = guis.template(bp_name.as_bytes())
            && let Some(preview_el) = find_direct(root, "equipment_modules/equipment_preview")
        {
            if let Some(holder) = container_holder(main_layout, &preview_el.path) {
                let bp_layout = layout_window(bp_root, resources, holder.content, |instance| {
                    configure_blueprint(bp_root, instance, &data.fitted_modules);
                })?;
                plan.sub_windows.push(ComposedSubWindow {
                    before_part: holder.before,
                    parts: holder.place([0, 0], &bp_layout),
                    regions: Vec::new(),
                    diagnostics: bp_layout.diagnostics,
                    root_window: bp_name.to_owned(),
                    path_prefix: "blueprint/".to_owned(),
                    close_paths: Vec::new(),
                    tooltip_overrides: BTreeMap::new(),
                });
            }
        }

        let slot_root = guis
            .template(b"equipment_designer_module_slot_entry")
            .ok_or_else(|| {
                GameWindowError::refused(
                    "Required template 'equipment_designer_module_slot_entry' is missing from the loaded interface files.",
                )
            })?;
        compose_module_slots(
            main_layout,
            root,
            slot_root,
            resources,
            localisation,
            data,
            equipment_db,
            &mut plan,
        )?;

        let stat_root = guis.template(b"stat_entry").ok_or_else(|| {
            GameWindowError::refused(
                "Required template 'stat_entry' is missing from the loaded interface files.",
            )
        })?;
        let columns = build_stat_columns(
            data.kind,
            &data.stats,
            data.manpower,
            data.design_state_reason.as_deref(),
        );
        for (rel_grid, lines) in [
            ("info/base_stats/stats_grid", &columns.base),
            ("info/combat_stats/stats_grid", &columns.combat),
            ("info/misc_stats/stats_grid", &columns.misc),
        ] {
            let grid_el = find_direct(root, rel_grid).ok_or_else(|| {
                GameWindowError::refused(format!(
                    "Designer window '{}' is missing required stat grid '{rel_grid}'.",
                    data.kind.window_name()
                ))
            })?;
            compose_stat_grid(
                main_layout,
                grid_el,
                stat_root,
                "stat_entry",
                "value",
                lines,
                1,
                155,
                20,
                resources,
                localisation,
                &mut plan,
            )?;
        }
    } else if let DesignerKind::Upgrade(domain) = data.kind {
        let sub_name = domain.sub_window();
        let sub_root = guis.template(sub_name.as_bytes()).ok_or_else(|| {
            GameWindowError::refused(format!(
                "Required upgrade designer sub-window '{sub_name}' is missing from the loaded interface files."
            ))
        })?;
        let host_el = find_direct(root, "active_equipment_view").ok_or_else(|| {
            GameWindowError::refused(format!(
                "Designer window '{}' is missing required container 'active_equipment_view'.",
                data.kind.window_name()
            ))
        })?;
        let holder = container_holder(main_layout, &host_el.path).ok_or_else(|| {
            GameWindowError::refused(format!(
                "Designer window '{}' could not place 'active_equipment_view'.",
                data.kind.window_name()
            ))
        })?;
        let mut sub_tooltips = BTreeMap::new();
        let sub_layout = layout_window(sub_root, resources, holder.content, |instance| {
            configure_upgrade_view(
                sub_root,
                resources,
                localisation,
                instance,
                data,
                equipment_icon,
                &mut sub_tooltips,
            );
        })?;
        plan.sub_windows.push(ComposedSubWindow {
            before_part: holder.before,
            parts: holder.place([0, 0], &sub_layout),
            regions: holder.place_regions([0, 0], &sub_layout.regions),
            diagnostics: sub_layout.diagnostics,
            root_window: sub_name.to_owned(),
            path_prefix: "upgrade_view/".to_owned(),
            close_paths: Vec::new(),
            tooltip_overrides: sub_tooltips,
        });
        let stat_root = guis
            .template(b"equipment_designer_stat_entry")
            .ok_or_else(|| {
                GameWindowError::refused(
                    "Required template 'equipment_designer_stat_entry' is missing from the loaded interface files.",
                )
            })?;
        let grid_el = find_direct(root, "equipment_info/equipment_info_grid").ok_or_else(|| {
            GameWindowError::refused(format!(
                "Designer window '{}' is missing required stat grid 'equipment_info/equipment_info_grid'.",
                data.kind.window_name()
            ))
        })?;
        let columns = build_stat_columns(
            data.kind,
            &data.stats,
            data.manpower,
            data.design_state_reason.as_deref(),
        );
        compose_stat_grid(
            main_layout,
            grid_el,
            stat_root,
            "equipment_designer_stat_entry",
            "count",
            &columns.base,
            2,
            258,
            21,
            resources,
            localisation,
            &mut plan,
        )?;
    }

    Ok(plan)
}

fn configure_blueprint(
    bp_root: &Element,
    instance: &mut Instance,
    fitted_modules: &RecordedModules,
) {
    let Some(slots_container) = find_direct(bp_root, "module_slots") else {
        return;
    };
    let fitted_map = match fitted_modules {
        RecordedModules::Available(map) => Some(map),
        RecordedModules::Unavailable(_) => None,
    };
    for slot_container in &slots_container.children {
        if slot_container.kind != Kind::Container {
            continue;
        }
        let slot_name = element_name(slot_container);
        let fitted = fitted_map.and_then(|map| map.get(slot_name));
        let mut matched_specific = false;
        if let Some(fitted) = fitted {
            for child in &slot_container.children {
                if child.kind != Kind::Container {
                    continue;
                }
                let child_name = element_name(child);
                if child_name.ends_with("@highlight") {
                    continue;
                }
                if (!fitted.gui_category.is_empty()
                    && child_name.eq_ignore_ascii_case(&fitted.gui_category))
                    || (!fitted.category.is_empty()
                        && child_name.eq_ignore_ascii_case(&fitted.category))
                    || child_name.eq_ignore_ascii_case(&fitted.module)
                {
                    matched_specific = true;
                    break;
                }
            }
        }
        for child in &slot_container.children {
            if child.kind != Kind::Container {
                continue;
            }
            let child_name = element_name(child);
            let keep = if child_name.ends_with("@highlight") {
                false
            } else if let Some(fitted) = fitted {
                if matched_specific {
                    (!fitted.gui_category.is_empty()
                        && child_name.eq_ignore_ascii_case(&fitted.gui_category))
                        || (!fitted.category.is_empty()
                            && child_name.eq_ignore_ascii_case(&fitted.category))
                        || child_name.eq_ignore_ascii_case(&fitted.module)
                } else {
                    child_name.eq_ignore_ascii_case(slot_name)
                }
            } else {
                false
            };
            if !keep {
                let _ = instance.hide(bp_root, child);
            }
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "module slot placement maps recorded slots onto authored marker positions"
)]
fn compose_module_slots(
    main_layout: &Layout,
    root: &Element,
    slot_root: &Element,
    resources: &dyn Resources,
    localisation: &Localisation,
    data: &EquipmentDesignData,
    equipment_db: &Database,
    plan: &mut ComposedPlan,
) -> Result<(), GameWindowError> {
    let modules_container = find_direct(root, "equipment_modules").ok_or_else(|| {
        GameWindowError::refused(format!(
            "Designer window '{}' is missing required container 'equipment_modules'.",
            data.kind.window_name()
        ))
    })?;
    let mut fixed_markers: Vec<(usize, &Element, [i32; 2])> = Vec::new();
    let mut custom_markers: Vec<(usize, &Element, [i32; 2])> = Vec::new();
    for child in &modules_container.children {
        let Fields::PositionType(pos) = &child.fields else {
            continue;
        };
        let name = element_name(child);
        if let Some(suffix) = name.strip_prefix("pos_fixed_module_slot_window_")
            && let Ok(index) = suffix.parse::<usize>()
        {
            fixed_markers.push((index, child, pos.position));
        } else if let Some(suffix) = name.strip_prefix("pos_custom_module_slot_window_")
            && let Ok(index) = suffix.parse::<usize>()
        {
            custom_markers.push((index, child, pos.position));
        }
    }
    fixed_markers.sort_by_key(|(index, _, _)| *index);
    custom_markers.sort_by_key(|(index, _, _)| *index);

    let assignments: Vec<(&Element, [i32; 2], SlotIcon)> = match data.kind {
        DesignerKind::Tank => {
            if data.module_slots.len() > custom_markers.len() {
                return Err(GameWindowError::refused(format!(
                    "Equipment '{}' defines {} module slots, exceeding the {} slot markers in '{}'.",
                    data.equipment_key,
                    data.module_slots.len(),
                    custom_markers.len(),
                    data.kind.window_name()
                )));
            }
            let mut out = Vec::new();
            for (idx, (_, marker, pos)) in custom_markers.iter().enumerate() {
                let icon = match data.module_slots.get(idx) {
                    Some((slot_key, slot_gfx)) => {
                        slot_icon_for(slot_key, slot_gfx, data, equipment_db, resources)
                    }
                    None => SlotIcon::Locked {
                        sprite: locked_slot_sprite().to_owned(),
                    },
                };
                out.push((*marker, *pos, icon));
            }
            out
        }
        DesignerKind::Ship | DesignerKind::Plane => {
            let fixed_slots: Vec<&(String, String)> = data
                .module_slots
                .iter()
                .filter(|(name, _)| name.starts_with("fixed_"))
                .collect();
            let custom_slots: Vec<&(String, String)> = data
                .module_slots
                .iter()
                .filter(|(name, _)| !name.starts_with("fixed_"))
                .collect();
            if fixed_slots.len() > fixed_markers.len() || custom_slots.len() > custom_markers.len()
            {
                return Err(GameWindowError::refused(format!(
                    "Equipment '{}' defines {} fixed / {} custom module slots, exceeding the {} fixed / {} custom markers in '{}'.",
                    data.equipment_key,
                    fixed_slots.len(),
                    custom_slots.len(),
                    fixed_markers.len(),
                    custom_markers.len(),
                    data.kind.window_name()
                )));
            }
            let mut out = Vec::new();
            for (idx, (_, marker, pos)) in fixed_markers.iter().enumerate() {
                let icon = match fixed_slots.get(idx) {
                    Some((slot_key, slot_gfx)) => {
                        slot_icon_for(slot_key, slot_gfx, data, equipment_db, resources)
                    }
                    None => SlotIcon::Locked {
                        sprite: locked_slot_sprite().to_owned(),
                    },
                };
                out.push((*marker, *pos, icon));
            }
            for (idx, (_, marker, pos)) in custom_markers.iter().enumerate() {
                let icon = match custom_slots.get(idx) {
                    Some((slot_key, slot_gfx)) => {
                        slot_icon_for(slot_key, slot_gfx, data, equipment_db, resources)
                    }
                    None => SlotIcon::Locked {
                        sprite: locked_slot_sprite().to_owned(),
                    },
                };
                out.push((*marker, *pos, icon));
            }
            out
        }
        DesignerKind::Upgrade(_) => Vec::new(),
    };

    let empty_slot_body = loc_fallback(
        localisation,
        "EQUIPMENT_DESIGNER_SLOT_EMPTY_TOOLTIP",
        "Empty module slot",
    );
    let locked_slot_title = loc_fallback(
        localisation,
        "EQUIPMENT_DESIGNER_MODULE_SLOT_NOT_AVAILABLE",
        "Locked module slot",
    );

    let viewport = DEFAULT_VIEWPORT.map(|value| value as i32);
    for (ordinal, (marker, position, icon)) in assignments.into_iter().enumerate() {
        let Some(holder) = main_layout
            .holders
            .iter()
            .find(|holder| holder.path == marker.path)
        else {
            continue;
        };
        let (sprite, tooltip) = match &icon {
            SlotIcon::Fitted {
                slot,
                module,
                sprite,
            } => (
                sprite.as_str(),
                GameWindowTooltip {
                    title: loc_or(localisation, module),
                    body: Some(loc_or(localisation, slot)),
                    detail: None,
                },
            ),
            SlotIcon::Empty { slot, sprite } => (
                sprite.as_str(),
                GameWindowTooltip {
                    title: loc_or(localisation, slot),
                    body: Some(empty_slot_body.clone()),
                    detail: None,
                },
            ),
            SlotIcon::Unknown {
                slot,
                sprite,
                reason,
            } => (
                sprite.as_str(),
                GameWindowTooltip {
                    title: loc_or(localisation, slot),
                    body: Some(format!(
                        "Not recorded: fitted module is unavailable at this hour ({reason})."
                    )),
                    detail: None,
                },
            ),
            SlotIcon::Locked { sprite } => (
                sprite.as_str(),
                GameWindowTooltip {
                    title: locked_slot_title.clone(),
                    body: None,
                    detail: None,
                },
            ),
        };
        let slot_layout = layout_window(slot_root, resources, viewport, |instance| {
            hide_by_name(instance, slot_root, "icn_requirements");
            hide_by_name(instance, slot_root, "icn_warning");
            if let Some(button) = find_direct(slot_root, "btn_module_slot")
                && let Ok(bound) = instance.control(slot_root, button)
            {
                bound.sprite = Some(sprite.as_bytes().to_vec());
            }
        })?;
        let mut overrides = BTreeMap::new();
        if let Some(button) = find_direct(slot_root, "btn_module_slot") {
            overrides.insert(button.path.clone(), tooltip);
        }
        plan.sub_windows.push(ComposedSubWindow {
            before_part: holder.before,
            parts: holder.place(position, &slot_layout),
            regions: holder.place_regions(position, &slot_layout.regions),
            diagnostics: slot_layout.diagnostics,
            root_window: "equipment_designer_module_slot_entry".to_owned(),
            path_prefix: format!("slot:{ordinal}/"),
            close_paths: Vec::new(),
            tooltip_overrides: overrides,
        });
    }
    Ok(())
}

fn slot_icon_for(
    slot_key: &str,
    slot_gfx: &str,
    data: &EquipmentDesignData,
    equipment_db: &Database,
    resources: &dyn Resources,
) -> SlotIcon {
    let has_sprite = |name: &str| resources.sprite_info(name.as_bytes()).is_some();
    match &data.fitted_modules {
        RecordedModules::Unavailable(reason) => SlotIcon::Unknown {
            slot: slot_key.to_owned(),
            sprite: "GFX_EMI_unknown".to_owned(),
            reason: reason.clone(),
        },
        RecordedModules::Available(fitted_map) => {
            if let Some(fitted) = fitted_map.get(slot_key) {
                let module_gfx = equipment_db
                    .module_design(&fitted.module)
                    .and_then(|design| std::str::from_utf8(&design.gfx).ok())
                    .unwrap_or("");
                let sprite = resolve_fitted_module_sprite(&fitted.module, module_gfx, has_sprite);
                SlotIcon::Fitted {
                    slot: slot_key.to_owned(),
                    module: fitted.module.clone(),
                    sprite,
                }
            } else {
                let sprite = resolve_empty_slot_sprite(slot_key, slot_gfx, has_sprite);
                SlotIcon::Empty {
                    slot: slot_key.to_owned(),
                    sprite,
                }
            }
        }
    }
}

fn configure_upgrade_view(
    sub_root: &Element,
    resources: &dyn Resources,
    localisation: &Localisation,
    instance: &mut Instance,
    data: &EquipmentDesignData,
    equipment_icon: Option<&Icon>,
    tooltips: &mut BTreeMap<String, GameWindowTooltip>,
) {
    apply_equipment_icon(instance, sub_root, "archetype", equipment_icon, resources);
    match &data.upgrades {
        RecordedUpgrades::Available(list) => {
            let upgrade_map: HashMap<String, (&str, u8)> = list
                .iter()
                .map(|(key, level)| (key.to_ascii_lowercase(), (key.as_str(), *level)))
                .collect();
            for child in &sub_root.children {
                if child.kind != Kind::Container {
                    continue;
                }
                let child_name = element_name(child);
                let name_lower = child_name.to_ascii_lowercase();
                if let Some((upgrade_key, level)) = upgrade_map.get(&name_lower) {
                    set_text_by_name(
                        instance,
                        sub_root,
                        &format!("{child_name}/label"),
                        loc_or(localisation, upgrade_key),
                    );
                    set_text_by_name(
                        instance,
                        sub_root,
                        &format!("{child_name}/level"),
                        level.to_string(),
                    );
                    disable_by_name(
                        instance,
                        sub_root,
                        &format!("{child_name}/decrease_level"),
                    );
                    disable_by_name(
                        instance,
                        sub_root,
                        &format!("{child_name}/increase_level"),
                    );
                } else {
                    let _ = instance.hide(sub_root, child);
                }
            }
        }
        RecordedUpgrades::Unavailable { keys, reason } => {
            let upgrade_map: HashMap<String, &str> = keys
                .iter()
                .map(|key| (key.to_ascii_lowercase(), key.as_str()))
                .collect();
            for child in &sub_root.children {
                if child.kind != Kind::Container {
                    continue;
                }
                let child_name = element_name(child);
                let name_lower = child_name.to_ascii_lowercase();
                if let Some(upgrade_key) = upgrade_map.get(&name_lower) {
                    let label = loc_or(localisation, upgrade_key);
                    set_text_by_name(
                        instance,
                        sub_root,
                        &format!("{child_name}/label"),
                        label.clone(),
                    );
                    set_text_by_name(
                        instance,
                        sub_root,
                        &format!("{child_name}/level"),
                        NOT_RECORDED_VALUE.to_owned(),
                    );
                    if let Some(level_el) = find_direct(sub_root, &format!("{child_name}/level")) {
                        tooltips.insert(
                            level_el.path.clone(),
                            GameWindowTooltip {
                                title: label,
                                body: Some(format!(
                                    "Not recorded: upgrade levels are unavailable at this hour ({reason})."
                                )),
                                detail: None,
                            },
                        );
                    }
                    disable_by_name(
                        instance,
                        sub_root,
                        &format!("{child_name}/decrease_level"),
                    );
                    disable_by_name(
                        instance,
                        sub_root,
                        &format!("{child_name}/increase_level"),
                    );
                } else {
                    let _ = instance.hide(sub_root, child);
                }
            }
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "stat grid placement binds a template across grid geometry, values, and tooltips"
)]
fn compose_stat_grid(
    main_layout: &Layout,
    grid_el: &Element,
    stat_root: &Element,
    stat_window_name: &str,
    value_child: &str,
    lines: &[StatLine],
    columns: usize,
    slot_width: i32,
    slot_height: i32,
    resources: &dyn Resources,
    localisation: &Localisation,
    plan: &mut ComposedPlan,
) -> Result<(), GameWindowError> {
    let Some(holder) = main_layout
        .holders
        .iter()
        .find(|holder| holder.path == grid_el.path)
    else {
        return Err(GameWindowError::refused(format!(
            "Designer stat grid '{}' produced no layout holder.",
            grid_el.path
        )));
    };
    let grid_offset = match &grid_el.fields {
        Fields::Grid(grid) => [grid.position.x.value, grid.position.y.value],
        _ => [0, 0],
    };

    let viewport = DEFAULT_VIEWPORT.map(|value| value as i32);
    for (index, line) in lines.iter().enumerate() {
        let col = (index % columns.max(1)) as i32;
        let row = (index / columns.max(1)) as i32;
        let place = [
            grid_offset[0] + col * slot_width,
            grid_offset[1] + row * slot_height,
        ];
        let full_label = localisation
            .get(line.label_key)
            .map(strip_markup)
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| line.fallback_label.to_owned());
        let shown_value = line
            .value
            .clone()
            .unwrap_or_else(|| NOT_RECORDED_VALUE.to_owned());

        let row_layout = layout_window(stat_root, resources, viewport, |instance| {
            set_text_by_name(instance, stat_root, "label", full_label.clone());
            set_text_by_name(instance, stat_root, value_child, shown_value);
        })?;
        let mut overrides = BTreeMap::new();
        let desc_key = format!("{}_DESC", line.label_key);
        let desc_text = localisation
            .get(&desc_key)
            .map(strip_markup)
            .filter(|text| !text.is_empty());
        let tooltip = GameWindowTooltip {
            title: full_label.trim_end_matches(':').trim().to_owned(),
            body: line.detail.clone().or(desc_text.clone()),
            detail: line.detail.is_some().then_some(desc_text).flatten(),
        };
        let mut placed_regions = holder.place_regions(place, &row_layout.regions);
        if placed_regions.is_empty() {
            placed_regions.push(native_gui::Region {
                path: stat_root.path.clone(),
                owner: None,
                background_index: None,
                scroll: None,
                slide: None,
                rect: [
                    holder.origin[0] + place[0] as f32,
                    holder.origin[1] + place[1] as f32,
                    slot_width as f32,
                    slot_height as f32,
                ],
                clip: holder.clip,
                hit: None,
                disabled: false,
            });
        }
        overrides.insert(stat_root.path.clone(), tooltip.clone());
        if let Some(label_el) = find_direct(stat_root, "label") {
            overrides.insert(label_el.path.clone(), tooltip.clone());
        }
        if let Some(val_el) = find_direct(stat_root, value_child) {
            overrides.insert(val_el.path.clone(), tooltip);
        }
        plan.sub_windows.push(ComposedSubWindow {
            before_part: holder.before,
            parts: holder.place(place, &row_layout),
            regions: placed_regions,
            diagnostics: row_layout.diagnostics,
            root_window: stat_window_name.to_owned(),
            path_prefix: format!("stat:{}:{index}/", grid_el.path),
            close_paths: Vec::new(),
            tooltip_overrides: overrides,
        });
    }
    Ok(())
}
