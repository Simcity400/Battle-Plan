use super::{
    ComposedPlan, ComposedSubWindow, DEFAULT_VIEWPORT, GameWindowError, GameWindowRegion,
    GameWindowRender, GameWindowRequest, GameWindowTooltip, Prepared, RenderParams, Service,
    WindowCacheKey, container_holder, layout_window, strip_markup,
};
use content_reader_equipment_graphics::equipment_graphics::countries as graphics_countries;
use content_reader_gui::gui::native::{self, Element, Fields, Kind};
use content_reader_image::native::Decode;
use content_reader_key_names::{
    equipment::{Database, Equipment},
    resources::Resources as ResourceDatabase,
};
use content_reader_layers::{ResolveOptions, ResolvedLayers, content::ResolvedContent};
use content_reader_localisation::localisation::Localisation;
use content_reader_parser::parser::c_string;
use content_reader_script::equipment_stats::{
    STAT_COUNT, category_names, stat_index,
};
use dashboard_base::{
    Source,
    products::{
        artwork::resolver::{ArtProvenance, NativeIconResolution, resolve_native_equipment_icon},
        equipment_icon::{Icon, IconHour, variant_icon},
    },
};
use dashboard_text::metadata::link_slot;
use replay::campaign::SeekResult;
use replay_data::rows::{
    IndustrialOrganisationRow, LexicalTokenRow, Recorded, TableId, TableSchema,
    VariantDefinitionRow, VariantDesignState, VariantModuleRow, VariantStatsRow, VariantUpgradeRow,
    tables,
};
use replay_frame::frame::{DecodedFrame, TableView};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RecordedStats {
    Available {
        values: [Option<i64>; 10],
        residuals: BTreeMap<usize, i64>,
    },
    MissingRow {
        residuals: BTreeMap<usize, i64>,
    },
    Unavailable {
        reason: String,
        residuals: BTreeMap<usize, i64>,
    },
}

impl RecordedStats {
    fn lane(&self, index: usize) -> Option<i64> {
        match self {
            Self::Available { values, .. } => values[index],
            Self::MissingRow { .. } | Self::Unavailable { .. } => None,
        }
    }

    fn residual(&self, keyword: &str) -> Option<i64> {
        let idx = stat_index(keyword.as_bytes())?;
        let map = match self {
            Self::Available { residuals, .. }
            | Self::MissingRow { residuals }
            | Self::Unavailable { residuals, .. } => residuals,
        };
        map.get(&idx).copied()
    }

    fn missing_reason(&self) -> String {
        match self {
            Self::Available { .. } => {
                "Not recorded: this stat was withheld or not recorded for this design at this hour."
                    .to_owned()
            }
            Self::MissingRow { .. } => {
                "Not recorded: this design has no variant stats row at this hour.".to_owned()
            }
            Self::Unavailable { reason, .. } => {
                format!("Not recorded: variant stats are unavailable at this hour ({reason}).")
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FittedSlot {
    Resolved(FittedModule),
    Unresolved { token: u32, reason: String },
}

#[derive(Clone, Debug, Hash)]
enum RecordedModules {
    Available {
        slots: BTreeMap<String, FittedSlot>,
        unattributed_unresolved: Option<String>,
    },
    Unavailable(String),
}

#[derive(Clone, Debug, Hash)]
enum RecordedUpgrades {
    Available {
        entries: Vec<(String, Option<u8>)>,
        unresolved_reason: Option<String>,
    },
    Unavailable {
        keys: Vec<String>,
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RecordedResource {
    pub key: Option<String>,
    pub token: u32,
    pub amount: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RecordedMio {
    None,
    Resolved(ResolvedMio),
    Unresolved {
        id: u64,
        trait_count: Option<i32>,
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatLine {
    pub label_key: &'static str,
    pub value: Option<String>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatColumns {
    pub base: Vec<StatLine>,
    pub combat: Vec<StatLine>,
    pub misc: Vec<StatLine>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ResolvedMio {
    name: String,
    sprite: Option<String>,
    trait_count: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FittedModule {
    module: String,
    category: String,
    gui_category: String,
}

#[derive(Clone, Debug, Hash)]
enum ResolvedEquipmentIcon {
    Ready {
        icon: Icon,
        provenance: ArtProvenance,
    },
    Missing {
        reason: String,
    },
    Refused {
        reason: String,
        construct: String,
    },
    Failed {
        reason: String,
    },
    NoneExpected,
}

#[derive(Clone, Debug, Hash)]
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
    mio: RecordedMio,
    module_slots: Vec<(String, String)>,
    fitted_modules: RecordedModules,
    upgrades: RecordedUpgrades,
    stats: RecordedStats,
    manpower: Option<i32>,
    build_cost: Option<i64>,
    build_resources: Option<Vec<RecordedResource>>,
    extra_warnings: Vec<String>,
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

fn residual_stat(
    stats: &RecordedStats,
    label_key: &'static str,
    keyword: Option<&'static str>,
    format_value: impl FnOnce(i64) -> String,
) -> StatLine {
    if let Some(kw) = keyword
        && let Some(raw) = stats.residual(kw)
    {
        return StatLine {
            label_key,
            value: Some(format_value(raw)),
            detail: None,
        };
    }
    StatLine {
        label_key,
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
    let fixed1 = |raw: i64| format_fixed5(raw, 1);
    let fixed2 = |raw: i64| format_fixed5(raw, 2);
    let pct1 = |raw: i64| format_percent_fixed5(raw, 1);
    let km = |raw: i64| format!("{} km", format_fixed5(raw, 0));
    let kmh = |raw: i64| format!("{} km/h", format_fixed5(raw, 1));
    let kn = |raw: i64| format!("{} kn", format_fixed5(raw, 1));
    match kind {
        DesignerKind::Ship => {
            let (reliability, reliability_detail) =
                recorded_stat(stats, VariantStatsRow::RELIABILITY, pct1);
            let (fuel, fuel_detail) =
                recorded_stat(stats, VariantStatsRow::FUEL_CONSUMPTION, fixed2);
            StatColumns {
                base: vec![
                    residual_stat(stats, "STAT_NAVY_MAXIMUM_SPEED", Some("naval_speed"), kn),
                    residual_stat(stats, "STAT_NAVY_RANGE", Some("naval_range"), km),
                    residual_stat(stats, "STAT_COMMON_MAX_ORG", Some("max_organisation"), fixed1),
                    residual_stat(stats, "STAT_COMMON_MAX_STRENGTH", Some("max_strength"), fixed1),
                    StatLine {
                        label_key: "STAT_COMMON_RELIABILITY",
                        value: reliability,
                        detail: reliability_detail,
                    },
                    residual_stat(stats, "STAT_ARMY_SUPPLY_CONSUMPTION", Some("supply_consumption"), fixed2),
                    StatLine {
                        label_key: "MANPOWER",
                        value: manpower.map(|value| value.to_string()),
                        detail: manpower.is_none().then(|| {
                            design_state_reason
                                .map(str::to_owned)
                                .unwrap_or_else(|| NOT_RECORDED_TOOLTIP.to_owned())
                        }),
                    },
                ],
                combat: vec![
                    residual_stat(stats, "STAT_NAVY_LG_ATTACK", Some("lg_attack"), fixed1),
                    residual_stat(stats, "STAT_NAVY_LG_ARMOR_PIERCING", Some("lg_armor_piercing"), fixed1),
                    residual_stat(stats, "STAT_NAVY_HG_ATTACK", Some("hg_attack"), fixed1),
                    residual_stat(stats, "STAT_NAVY_HG_ARMOR_PIERCING", Some("hg_armor_piercing"), fixed1),
                    residual_stat(stats, "STAT_NAVY_TORPEDO_ATTACK", Some("torpedo_attack"), fixed1),
                    residual_stat(stats, "STAT_NAVY_SUB_ATTACK", Some("sub_attack"), fixed1),
                    residual_stat(stats, "STAT_COMMON_ARMOR", Some("armor_value"), fixed1),
                    residual_stat(stats, "STAT_NAVY_ANTI_AIR_ATTACK", Some("anti_air_attack"), fixed1),
                ],
                misc: vec![
                    StatLine {
                        label_key: "STAT_COMMON_FUEL_CONSUMPTION",
                        value: fuel,
                        detail: fuel_detail,
                    },
                    residual_stat(stats, "STAT_NAVY_SURFACE_VISIBILITY", Some("surface_visibility"), fixed1),
                    residual_stat(stats, "STAT_NAVY_SURFACE_DETECTION", Some("surface_detection"), fixed1),
                    residual_stat(stats, "STAT_NAVY_SUB_VISIBILITY", Some("sub_visibility"), fixed1),
                    residual_stat(stats, "STAT_NAVY_SUB_DETECTION", Some("sub_detection"), fixed1),
                    residual_stat(stats, "STAT_NAVY_MINES_PLANTING", Some("mines_planting"), fixed2),
                    residual_stat(stats, "STAT_NAVY_MINES_SWEEPING", Some("mines_sweeping"), fixed2),
                    residual_stat(stats, "STAT_NAVY_WEATHER_PENALTY", Some("weather_penalty"), pct1),
                    residual_stat(stats, "STAT_NAVY_HIT_PROFILE", None, fixed1),
                ],
            }
        }
        DesignerKind::Tank => {
            let (speed, speed_detail) =
                recorded_stat(stats, VariantStatsRow::MAXIMUM_SPEED, kmh);
            let (reliability, reliability_detail) =
                recorded_stat(stats, VariantStatsRow::RELIABILITY, pct1);
            let (air_attack, air_attack_detail) =
                recorded_stat(stats, VariantStatsRow::AIR_ATTACK, fixed1);
            let (fuel, fuel_detail) =
                recorded_stat(stats, VariantStatsRow::FUEL_CONSUMPTION, fixed2);
            StatColumns {
                base: vec![
                    StatLine {
                        label_key: "STAT_COMMON_MAXIMUM_SPEED",
                        value: speed,
                        detail: speed_detail,
                    },
                    StatLine {
                        label_key: "STAT_COMMON_RELIABILITY",
                        value: reliability,
                        detail: reliability_detail,
                    },
                    residual_stat(stats, "STAT_COMMON_MAX_STRENGTH", Some("max_strength"), fixed1),
                    residual_stat(stats, "STAT_ARMY_SUPPLY_CONSUMPTION", Some("supply_consumption"), fixed2),
                ],
                combat: vec![
                    residual_stat(stats, "STAT_ARMY_SOFT_ATTACK", Some("soft_attack"), fixed1),
                    residual_stat(stats, "STAT_ARMY_HARD_ATTACK", Some("hard_attack"), fixed1),
                    residual_stat(stats, "STAT_COMMON_PIERCING", Some("ap_attack"), fixed1),
                    residual_stat(stats, "STAT_ARMY_HARDNESS", Some("hardness"), pct1),
                    residual_stat(stats, "STAT_COMMON_ARMOR", Some("armor_value"), fixed1),
                    residual_stat(stats, "STAT_ARMY_BREAKTHROUGH", Some("breakthrough"), fixed1),
                    residual_stat(stats, "STAT_ARMY_DEFENSE", Some("defense"), fixed1),
                    StatLine {
                        label_key: "STAT_AIR_ATTACK",
                        value: air_attack,
                        detail: air_attack_detail,
                    },
                ],
                misc: vec![
                    residual_stat(stats, "STAT_ARMY_FUEL_CAPACITY", Some("fuel_capacity"), fixed1),
                    StatLine {
                        label_key: "STAT_COMMON_FUEL_CONSUMPTION",
                        value: fuel,
                        detail: fuel_detail,
                    },
                    residual_stat(stats, "STAT_ARMY_SUPRESSION", Some("suppression"), fixed1),
                    residual_stat(stats, "STAT_ARMY_RECON", Some("recon"), fixed1),
                    residual_stat(stats, "STAT_ARMY_ENTRENCHMENT", Some("entrenchment"), fixed1),
                ],
            }
        }
        DesignerKind::Plane => {
            let (speed, speed_detail) =
                recorded_stat(stats, VariantStatsRow::MAXIMUM_SPEED, kmh);
            let (range, range_detail) = recorded_stat(stats, VariantStatsRow::AIR_RANGE, km);
            let (weight, weight_detail) =
                recorded_stat(stats, VariantStatsRow::WEIGHT, fixed1);
            let (thrust, thrust_detail) =
                recorded_stat(stats, VariantStatsRow::THRUST, fixed1);
            let (air_def, air_def_detail) =
                recorded_stat(stats, VariantStatsRow::AIR_DEFENCE, fixed1);
            let (air_atk, air_atk_detail) =
                recorded_stat(stats, VariantStatsRow::AIR_ATTACK, fixed1);
            let (agility, agility_detail) =
                recorded_stat(stats, VariantStatsRow::AIR_AGILITY, fixed1);
            let (reliability, reliability_detail) =
                recorded_stat(stats, VariantStatsRow::RELIABILITY, pct1);
            let (fuel, fuel_detail) =
                recorded_stat(stats, VariantStatsRow::FUEL_CONSUMPTION, fixed2);
            StatColumns {
                base: vec![
                    StatLine {
                        label_key: "STAT_COMMON_MAXIMUM_SPEED",
                        value: speed,
                        detail: speed_detail,
                    },
                    StatLine {
                        label_key: "STAT_AIR_RANGE",
                        value: range,
                        detail: range_detail,
                    },
                    residual_stat(stats, "STAT_ARMY_SUPPLY_CONSUMPTION", Some("supply_consumption"), fixed2),
                    StatLine {
                        label_key: "STAT_COMMON_WEIGHT",
                        value: weight,
                        detail: weight_detail,
                    },
                    StatLine {
                        label_key: "STAT_COMMON_THRUST",
                        value: thrust,
                        detail: thrust_detail,
                    },
                ],
                combat: vec![
                    StatLine {
                        label_key: "STAT_AIR_DEFENCE",
                        value: air_def,
                        detail: air_def_detail,
                    },
                    StatLine {
                        label_key: "STAT_AIR_ATTACK",
                        value: air_atk,
                        detail: air_atk_detail,
                    },
                    StatLine {
                        label_key: "STAT_AIR_AGILITY",
                        value: agility,
                        detail: agility_detail,
                    },
                    residual_stat(stats, "STAT_AIR_SUPERIORITY", Some("air_superiority"), fixed1),
                    residual_stat(stats, "STAT_AIR_NAVAL_STRIKE_ATTACK", Some("naval_strike_attack"), fixed1),
                    residual_stat(stats, "STAT_AIR_NAVAL_STRIKE_TARGETTING", Some("naval_strike_targetting"), fixed1),
                    residual_stat(stats, "STAT_AIR_GROUND_ATTACK", Some("air_ground_attack"), fixed1),
                    residual_stat(stats, "STAT_AIR_BOMBING", Some("air_bombing"), fixed1),
                ],
                misc: vec![
                    StatLine {
                        label_key: "STAT_COMMON_RELIABILITY",
                        value: reliability,
                        detail: reliability_detail,
                    },
                    StatLine {
                        label_key: "STAT_COMMON_FUEL_CONSUMPTION",
                        value: fuel,
                        detail: fuel_detail,
                    },
                    residual_stat(stats, "STAT_NAVY_SURFACE_DETECTION", Some("surface_detection"), fixed1),
                    residual_stat(stats, "STAT_NAVY_SUB_DETECTION", Some("sub_detection"), fixed1),
                    residual_stat(stats, "STAT_COMMON_NIGHT_PENALTY", Some("night_penalty"), pct1),
                    residual_stat(stats, "STAT_NAVY_MINES_PLANTING", Some("mines_planting"), fixed2),
                    residual_stat(stats, "STAT_NAVY_MINES_SWEEPING", Some("mines_sweeping"), fixed2),
                ],
            }
        }
        DesignerKind::Upgrade(domain) => {
            let mut rows = Vec::new();
            match domain {
                UpgradeDomain::Naval => {
                    rows.push(residual_stat(
                        stats,
                        "STAT_NAVY_MAXIMUM_SPEED",
                        Some("naval_speed"),
                        kn,
                    ));
                }
                UpgradeDomain::Land | UpgradeDomain::Air => {
                    let (speed, speed_detail) =
                        recorded_stat(stats, VariantStatsRow::MAXIMUM_SPEED, kmh);
                    rows.push(StatLine {
                        label_key: "STAT_COMMON_MAXIMUM_SPEED",
                        value: speed,
                        detail: speed_detail,
                    });
                }
            }
            let (reliability, reliability_detail) =
                recorded_stat(stats, VariantStatsRow::RELIABILITY, pct1);
            rows.push(StatLine {
                label_key: "STAT_COMMON_RELIABILITY",
                value: reliability,
                detail: reliability_detail,
            });
            let (fuel, fuel_detail) =
                recorded_stat(stats, VariantStatsRow::FUEL_CONSUMPTION, fixed2);
            rows.push(StatLine {
                label_key: "STAT_COMMON_FUEL_CONSUMPTION",
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
                value: cost,
                detail: cost_detail,
            });
            if matches!(domain, UpgradeDomain::Air) {
                let (range, range_detail) = recorded_stat(stats, VariantStatsRow::AIR_RANGE, km);
                let (air_atk, air_atk_detail) =
                    recorded_stat(stats, VariantStatsRow::AIR_ATTACK, fixed1);
                let (air_def, air_def_detail) =
                    recorded_stat(stats, VariantStatsRow::AIR_DEFENCE, fixed1);
                let (agility, agility_detail) =
                    recorded_stat(stats, VariantStatsRow::AIR_AGILITY, fixed1);
                rows.push(StatLine {
                    label_key: "STAT_AIR_RANGE",
                    value: range,
                    detail: range_detail,
                });
                rows.push(StatLine {
                    label_key: "STAT_AIR_ATTACK",
                    value: air_atk,
                    detail: air_atk_detail,
                });
                rows.push(StatLine {
                    label_key: "STAT_AIR_DEFENCE",
                    value: air_def,
                    detail: air_def_detail,
                });
                rows.push(StatLine {
                    label_key: "STAT_AIR_AGILITY",
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

pub struct RenderContext<'a> {
    pub source: &'a Source,
    pub content: &'a Arc<ResolvedContent>,
    pub layers: &'a ResolvedLayers,
    pub resolve: &'a ResolveOptions,
    pub visual_revision: &'a str,
    pub language: &'a str,
    pub decode: Decode,
    pub admitted_gen: (u64, u64),
    pub extra_warnings: Vec<String>,
}

fn cached_resource_db(layers: &ResolvedLayers) -> Result<Arc<ResourceDatabase>, String> {
    #[derive(Default)]
    struct Cached(
        content_reader_layers::content::diagnostics::Once<Result<Arc<ResourceDatabase>, String>>,
    );
    layers
        .derived::<Cached>()
        .0
        .get_or_init(|| {
            content_reader_key_names::resources::read(layers)
                .map(|(db, _)| Arc::new(db))
                .map_err(|error| error.to_string())
        })
        .clone()
}

fn cached_ship_naming_groups(layers: &ResolvedLayers) -> Arc<HashMap<String, String>> {
    #[derive(Default)]
    struct Cached(
        content_reader_layers::content::diagnostics::Once<Arc<HashMap<String, String>>>,
    );
    Arc::clone(layers.derived::<Cached>().0.get_or_init(|| {
        let mut map = HashMap::new();
        if let Ok(files) = layers.directory_files("common/units/names_ships", ".txt") {
            for path in files {
                let Ok(bytes) = path.read(layers) else {
                    continue;
                };
                let Ok(doc) = content_reader_parser::parser::parse(&bytes) else {
                    continue;
                };
                for item in &doc.items {
                    let Ok(key) = item.key_str() else {
                        continue;
                    };
                    if let Some(fields) = item.value.items() {
                        for field in fields {
                            if field.keyword_is("name")
                                && let Some(raw) = field.value.read_string()
                                && let Ok(text) = std::str::from_utf8(raw)
                                && !text.is_empty()
                            {
                                map.entry(key.to_ascii_lowercase())
                                    .or_insert_with(|| text.to_owned());
                                break;
                            }
                        }
                    }
                }
            }
        }
        Arc::new(map)
    }))
}

pub(super) fn witnessed_single_key<'a, T, K>(
    frame: &'a DecodedFrame,
    target_key: K,
    key_of: impl Fn(&T::Row) -> K,
) -> Result<Option<&'a T::Row>, String>
where
    T: TableSchema,
    K: Ord,
{
    match frame.tables[T::ID.index()].held() {
        TableView::Complete(rows) => {
            let slice = rows
                .rows::<T>()
                .ok_or_else(|| format!("{:?} rows do not match table schema", T::ID))?;
            Ok(slice
                .binary_search_by_key(&target_key, key_of)
                .ok()
                .map(|idx| &slice[idx]))
        }
        TableView::Truncated { rows, missing } => {
            let slice = rows
                .rows::<T>()
                .ok_or_else(|| format!("{:?} rows do not match table schema", T::ID))?;
            match slice.binary_search_by_key(&target_key, key_of) {
                Ok(idx) => Ok(Some(&slice[idx])),
                Err(_) => Err(format!(
                    "{:?} is truncated ({missing} rows omitted) and does not witness this key",
                    T::ID
                )),
            }
        }
        TableView::Unavailable { reason } => Err(reason.clone()),
        TableView::NotRecorded { .. } => Err(format!("{:?} is not recorded at this hour", T::ID)),
    }
}

pub(super) fn witnessed_rows<'a, T: TableSchema>(frame: &'a DecodedFrame) -> Result<(&'a [T::Row], bool), String> {
    match frame.tables[T::ID.index()].held() {
        TableView::Complete(rows) => {
            let slice = rows
                .rows::<T>()
                .ok_or_else(|| format!("{:?} rows do not match table schema", T::ID))?;
            Ok((slice, true))
        }
        TableView::Truncated { rows, .. } => {
            let slice = rows
                .rows::<T>()
                .ok_or_else(|| format!("{:?} rows do not match table schema", T::ID))?;
            Ok((slice, false))
        }
        TableView::Unavailable { reason } => Err(reason.clone()),
        TableView::NotRecorded { .. } => Err(format!("{:?} is not recorded at this hour", T::ID)),
    }
}

fn extract_unresolved_residuals(
    design_state: Option<&VariantDesignState>,
    warnings: &mut Vec<String>,
) -> BTreeMap<usize, i64> {
    let mut map = BTreeMap::new();
    let Some(state) = design_state else {
        return map;
    };
    if state.named_bonuses.is_empty() || state.unresolved_stats.is_empty() {
        return map;
    }
    for entry in &state.unresolved_stats {
        let index = entry.index as usize;
        if index < STAT_COUNT {
            map.insert(index, entry.value);
        } else {
            warnings.push(format!(
                "Design residual stat index {index} exceeds recorded stat metadata ({STAT_COUNT}); ignored."
            ));
        }
    }
    map
}

struct DigestHasher(sha2::Sha256);

impl std::hash::Hasher for DigestHasher {
    fn finish(&self) -> u64 {
        0
    }

    fn write(&mut self, bytes: &[u8]) {
        sha2::Digest::update(&mut self.0, bytes);
    }
}

fn fingerprint_frame(data: &EquipmentDesignData, visuals: &impl std::hash::Hash) -> [u8; 32] {
    use std::hash::Hash;
    let mut hasher = DigestHasher(sha2::Digest::new());
    data.hash(&mut hasher);
    visuals.hash(&mut hasher);
    sha2::Digest::finalize(hasher.0).into()
}

pub fn render(
    service: &Service,
    ctx: RenderContext<'_>,
    request: GameWindowRequest,
    cancel: &AtomicBool,
) -> Result<GameWindowRender, GameWindowError> {
    let RenderContext {
        source,
        content,
        layers,
        resolve,
        visual_revision,
        language,
        decode,
        admitted_gen,
        mut extra_warnings,
    } = ctx;
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

    let definition = witnessed_single_key::<tables::VariantDefinitions, _>(&frame, variant_id, |row| row.id)
        .map_err(|error| {
            GameWindowError::refused(format!(
                "Equipment definitions are not recorded at this hour: {error}"
            ))
        })?
        .ok_or_else(|| {
            GameWindowError::refused(format!(
                "Equipment design {} is not recorded at hour {}.",
                target.variant, request.hour
            ))
        })?;
    let (lexical, _) = witnessed_rows::<tables::LexicalTokens>(&frame).map_err(|error| {
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

    let equipment_db = Database::shared(layers).map_err(|error| {
        GameWindowError::failed(format!(
            "Could not load recorded equipment definitions: {error}"
        ))
    })?;
    let resource_db = cached_resource_db(layers).map_err(|error| {
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
    let (tag, tag_tables) = resolve_variant_tag(&frame, definition);
    let (design_state, design_state_reason) = match &definition.design {
        Recorded::Value(state) => (Some(state), None),
        Recorded::NotRecorded => (
            None,
            Some("Not recorded: variant design state is not recorded in this recording.".to_owned()),
        ),
    };
    let residuals = extract_unresolved_residuals(design_state, &mut extra_warnings);
    let (stats_row, recorded_stats) =
        match witnessed_single_key::<tables::VariantStats, _>(&frame, variant_id, |row| row.variant) {
            Ok(Some(row)) => (
                Some(row),
                RecordedStats::Available {
                    values: row.values,
                    residuals,
                },
            ),
            Ok(None) => (None, RecordedStats::MissingRow { residuals }),
            Err(reason) => (None, RecordedStats::Unavailable { reason, residuals }),
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

    let mio_id = design_state
        .map(|state| state.design_team)
        .filter(|&id| id != 0)
        .or_else(|| stats_row.map(|row| row.organisation).filter(|&id| id != 0));
    let mio_lookup = mio_id.map(|id| {
        witnessed_single_key::<tables::IndustrialOrganisations, _>(&frame, id, |row| row.organisation)
    });

    let mut data = extract_design_data(ExtractInputs {
        kind,
        tag: &tag,
        equipment_key: &equipment_key,
        archetype_key: &archetype_key,
        equipment,
        equipment_db: &equipment_db,
        definition,
        design_state,
        design_state_reason,
        stats_row,
        stats: recorded_stats,
        module_rows,
        upgrade_rows,
        lexical,
        mio_lookup,
    });
    if let Some(group_key) = data.naming_group.as_deref() {
        let groups = cached_ship_naming_groups(layers);
        if let Some(loc_key) = groups.get(&group_key.to_ascii_lowercase()) {
            data.naming_group = Some(loc_key.clone());
        }
    }

    let mut base_sampled_tables = BTreeSet::new();
    base_sampled_tables.insert(TableId::VariantDefinitions);
    base_sampled_tables.insert(TableId::LexicalTokens);
    if matches!(
        data.stats,
        RecordedStats::Available { .. } | RecordedStats::MissingRow { .. }
    ) {
        base_sampled_tables.insert(TableId::VariantStats);
    }
    if kind.is_modular() && modules_table.is_ok() {
        base_sampled_tables.insert(TableId::VariantModules);
    }
    if matches!(data.mio, RecordedMio::Resolved(_)) {
        base_sampled_tables.insert(TableId::IndustrialOrganisations);
    }
    if !equipment.module_slots.is_empty() && dlc_mask.is_some() {
        base_sampled_tables.insert(TableId::SimulationFeatures);
    }
    for table in tag_tables {
        base_sampled_tables.insert(table);
    }

    extra_warnings.extend(data.extra_warnings.iter().cloned());
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

    let window_name = kind.window_name();
    let cache_key = WindowCacheKey {
        source_id: request.source_id.clone(),
        revision: request.revision.clone(),
        game_dir: resolve.game_dir.clone(),
        user_dir: resolve.user_dir.clone(),
        visual_revision: visual_revision.to_owned(),
        language: language.to_owned(),
        force_pow2_textures: decode.force_pow2_textures,
        art_stack: layers.art_stack_key(),
        window: window_name.to_owned(),
    };
    #[derive(Hash)]
    struct ResolvedVisuals {
        equipment_icon: ResolvedEquipmentIcon,
        blueprint_name: Option<String>,
        mio_sprite: Option<String>,
        prep_warnings: Vec<String>,
        read_hour: i32,
    }

    let empty_tooltips = BTreeMap::new();
    let recorded_language = language.to_owned();
    let base_samples: Vec<(TableId, i32)> = base_sampled_tables
        .iter()
        .filter_map(|&id| frame.tables[id.index()].sampled_at().map(|h| (id, h)))
        .collect();
    let (provisional_read_hour, _) = compute_read_hour(request.hour, &base_samples);

    service.render_composed_from_layers(
        cache_key,
        layers,
        RenderParams {
            window_name,
            scale: request.scale,
            read_hour: Some(provisional_read_hour),
            art_from_current_install: true,
            close_paths: &["close_button"],
            tooltip_overrides: &empty_tooltips,
            extra_warnings,
            admitted_gen: Some(admitted_gen),
        },
        cancel,
        |shared| {
            let guis = shared.guis.as_ref();
            let registry = shared.registry.as_ref();
            let localisation = shared.localisation.as_ref();
            let mut extra = Assets::default();
            let mut bound_tables = base_sampled_tables.clone();
            let mut visuals = ResolvedVisuals {
                equipment_icon: ResolvedEquipmentIcon::NoneExpected,
                blueprint_name: None,
                mio_sprite: None,
                prep_warnings: Vec::new(),
                read_hour: request.hour,
            };
            add_sprite(&mut extra, "GFX_resources_strip");
            add_sprite(&mut extra, "GFX_production_cost");
            add_sprite(&mut extra, "GFX_navprod_cost");
            add_sprite(&mut extra, "GFX_EMSI_locked");
            add_sprite(&mut extra, "GFX_EMSI_empty");
            add_sprite(&mut extra, "GFX_EMI_unknown");
            add_sprite(&mut extra, "GFX_idea_unknown");

            if reads_upgrades(kind, guis.template(window_name.as_bytes())) {
                if upgrades_table.is_ok() {
                    bound_tables.insert(TableId::VariantUpgrades);
                } else if let RecordedUpgrades::Unavailable { reason, .. } = &data.upgrades {
                    visuals.prep_warnings.push(format!(
                        "Upgrade levels are unknown because VariantUpgrades is unavailable at this hour ({reason})."
                    ));
                }
            }

            let icon_content_res = shared.icon_content(layers, &equipment_db, || {
                dashboard_base::catalogue::read_scripted_content(
                    Arc::clone(content),
                    recorded_language.clone(),
                )
            });
            match icon_content_res {
                Ok(icon_content) => {
                    match IconHour::read(&frame, &icon_content) {
                        Ok(icon_hour) => {
                            for table in [
                                TableId::Countries,
                                TableId::CountryLinks,
                                TableId::CountryAppearance,
                                TableId::ReservedCountryNaming,
                                TableId::PoliticalParties,
                                TableId::Politics,
                                TableId::Factions,
                                TableId::FactionMembers,
                                TableId::States,
                            ] {
                                if !matches!(
                                    frame.tables[table.index()].held(),
                                    TableView::Unavailable { .. } | TableView::NotRecorded { .. }
                                ) {
                                    bound_tables.insert(table);
                                }
                            }
                            match variant_icon(&icon_content, &icon_hour, definition, &equipment_key)
                            {
                                Ok(Some(selected)) => {
                                    match resolve_native_equipment_icon(
                                        layers,
                                        registry,
                                        shared.recorded_registry.as_deref(),
                                        &recorded_language,
                                        &selected,
                                        decode,
                                    ) {
                                        NativeIconResolution::Ready {
                                            icon,
                                            provenance,
                                            ..
                                        } => {
                                            match &icon {
                                                Icon::Sprite(sprite) => {
                                                    add_sprite(&mut extra, sprite);
                                                }
                                                Icon::Path(path) => {
                                                    if !path.is_empty() {
                                                        extra
                                                            .textures
                                                            .insert(path.as_bytes().to_vec());
                                                    }
                                                }
                                            }
                                            visuals.equipment_icon =
                                                ResolvedEquipmentIcon::Ready { icon, provenance };
                                        }
                                        NativeIconResolution::ConfirmedMissing {
                                            reason, ..
                                        } => {
                                            visuals.prep_warnings.push(format!(
                                                "Equipment icon for '{equipment_key}' is missing: {reason}"
                                            ));
                                            visuals.equipment_icon =
                                                ResolvedEquipmentIcon::Missing { reason };
                                        }
                                        NativeIconResolution::Refused {
                                            reason,
                                            construct,
                                            ..
                                        } => {
                                            visuals.prep_warnings.push(format!(
                                                "Equipment icon for '{equipment_key}' was refused ({construct}): {reason}"
                                            ));
                                            visuals.equipment_icon =
                                                ResolvedEquipmentIcon::Refused {
                                                    reason,
                                                    construct,
                                                };
                                        }
                                        NativeIconResolution::Failed { reason, .. } => {
                                            visuals.prep_warnings.push(format!(
                                                "Equipment icon for '{equipment_key}' failed to load: {reason}"
                                            ));
                                            visuals.equipment_icon =
                                                ResolvedEquipmentIcon::Failed { reason };
                                        }
                                    }
                                }
                                Ok(None) => {
                                    visuals.equipment_icon = ResolvedEquipmentIcon::NoneExpected;
                                }
                                Err(why) => {
                                    visuals.prep_warnings.push(format!(
                                        "Equipment icon could not be resolved for '{equipment_key}': {why}"
                                    ));
                                    visuals.equipment_icon =
                                        ResolvedEquipmentIcon::Missing { reason: why };
                                }
                            }
                        }
                        Err(error) => {
                            let reason = format!(
                                "Equipment icon hour context is unavailable: {error}"
                            );
                            visuals.prep_warnings.push(reason.clone());
                            visuals.equipment_icon = ResolvedEquipmentIcon::Missing { reason };
                        }
                    }
                }
                Err(why) => {
                    let reason = format!("Equipment icon rules are unavailable: {why}");
                    visuals.prep_warnings.push(reason.clone());
                    visuals.equipment_icon = ResolvedEquipmentIcon::Missing { reason };
                }
            }

            if let RecordedMio::Resolved(mio) = &data.mio
                && let Some(sprite) = &mio.sprite
            {
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
                if let RecordedModules::Available { slots, .. } = &data.fitted_modules {
                    for (slot_key, slot_gfx) in &data.module_slots {
                        match slots.get(slot_key) {
                            Some(FittedSlot::Resolved(fitted)) => {
                                let module_gfx = equipment_db
                                    .module_design(&fitted.module)
                                    .and_then(|design| std::str::from_utf8(&design.gfx).ok())
                                    .unwrap_or("");
                                let sprite = resolve_fitted_module_sprite(
                                    &fitted.module,
                                    module_gfx,
                                    |s| registry.sprite(s.as_bytes()).is_some(),
                                );
                                add_sprite(&mut extra, &sprite);
                            }
                            Some(FittedSlot::Unresolved { .. }) => {
                                add_sprite(&mut extra, "GFX_EMI_unknown");
                            }
                            None => {
                                let sprite = resolve_empty_slot_sprite(slot_key, slot_gfx, |s| {
                                    registry.sprite(s.as_bytes()).is_some()
                                });
                                add_sprite(&mut extra, &sprite);
                            }
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

            let samples: Vec<(TableId, i32)> = bound_tables
                .iter()
                .filter_map(|&id| frame.tables[id.index()].sampled_at().map(|h| (id, h)))
                .collect();
            let (read_hour, mixed_warning) = compute_read_hour(request.hour, &samples);
            visuals.read_hour = read_hour;
            visuals.prep_warnings.extend(mixed_warning);
            let fingerprint = fingerprint_frame(&data, &visuals);
            Ok(Prepared {
                assets: extra,
                state: visuals,
                fingerprint: Some(fingerprint),
            })
        },
        |root, _guis, resources, localisation, instance, tooltips, visuals| {
            configure_main_window(
                root,
                resources,
                localisation,
                instance,
                MainWindowContext {
                    data: &data,
                    resource_db: &resource_db,
                    equipment_icon: &visuals.equipment_icon,
                    mio_sprite: visuals.mio_sprite.as_deref(),
                },
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
                SubWindowContext {
                    data: &data,
                    equipment_db: &equipment_db,
                    resource_db: &resource_db,
                    blueprint_name: visuals.blueprint_name.as_deref(),
                    equipment_icon: &visuals.equipment_icon,
                },
            )?;
            plan.read_hour = Some(visuals.read_hour);
            plan.title = Some(resolve_full_title(&data, localisation));
            if let ResolvedEquipmentIcon::Ready { provenance, .. } = &visuals.equipment_icon {
                plan.art_label = Some(provenance.label.clone());
            }
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
    frame: &DecodedFrame,
    definition: &VariantDefinitionRow,
) -> (String, Vec<TableId>) {
    let mut tables = Vec::new();
    let link_id = if definition.origin_link != 0 {
        definition.origin_link
    } else {
        definition.creator_link
    };
    if link_id == 0 {
        return (String::new(), tables);
    }
    let Ok(links) = frame.complete::<tables::CountryLinks>() else {
        return (String::new(), tables);
    };
    tables.push(TableId::CountryLinks);
    let Some(slot) = link_slot(links, link_id) else {
        return (String::new(), tables);
    };
    if let Ok(model_countries) = graphics_countries::read(frame) {
        tables.push(TableId::Countries);
        if !matches!(
            frame.tables[TableId::CountryAppearance.index()].held(),
            TableView::Unavailable { .. } | TableView::NotRecorded { .. }
        ) {
            tables.push(TableId::CountryAppearance);
        }
        if let Some(country) = model_countries.get(&slot) {
            if let Some(orig) = country.original_tag.as_deref()
                && !orig.is_empty()
            {
                return (orig.to_owned(), tables);
            }
            if !country.tag.is_empty() {
                return (country.tag.clone(), tables);
            }
        }
    }
    if let Ok(countries) = frame.complete::<tables::Countries>() {
        tables.push(TableId::Countries);
        if let Some(country) = countries.iter().find(|row| row.canonical_slot == slot)
            && let Ok(tag) = std::str::from_utf8(c_string(&country.tag))
            && !tag.is_empty()
        {
            return (tag.to_owned(), tables);
        }
    }
    (String::new(), tables)
}

struct ExtractInputs<'a> {
    kind: DesignerKind,
    tag: &'a str,
    equipment_key: &'a str,
    archetype_key: &'a str,
    equipment: &'a Equipment,
    equipment_db: &'a Database,
    definition: &'a VariantDefinitionRow,
    design_state: Option<&'a VariantDesignState>,
    design_state_reason: Option<String>,
    stats_row: Option<&'a VariantStatsRow>,
    stats: RecordedStats,
    module_rows: Result<&'a [VariantModuleRow], String>,
    upgrade_rows: Result<&'a [VariantUpgradeRow], String>,
    lexical: &'a [LexicalTokenRow],
    mio_lookup: Option<Result<Option<&'a IndustrialOrganisationRow>, String>>,
}

fn extract_design_data(inputs: ExtractInputs<'_>) -> EquipmentDesignData {
    let ExtractInputs {
        kind,
        tag,
        equipment_key,
        archetype_key,
        equipment,
        equipment_db,
        definition,
        design_state,
        design_state_reason,
        stats_row,
        stats,
        module_rows,
        upgrade_rows,
        lexical,
        mio_lookup,
    } = inputs;
    let mut extra_warnings = Vec::new();
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
    let mio_trait_count = stats_row
        .filter(|row| row.organisation != 0)
        .map(|row| row.organisation_trait_count)
        .filter(|&count| count >= 0)
        .or_else(|| {
            design_state
                .filter(|state| state.design_team != 0)
                .map(|state| state.design_team_traits)
                .filter(|&count| count >= 0)
        });
    let mio = match (mio_id, mio_lookup) {
        (None, _) => RecordedMio::None,
        (Some(_), Some(Ok(Some(row)))) => {
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
            RecordedMio::Resolved(ResolvedMio {
                name: raw_name,
                sprite: icon,
                trait_count: mio_trait_count,
            })
        }
        (Some(id), Some(Ok(None))) => RecordedMio::Unresolved {
            id,
            trait_count: mio_trait_count,
            reason: format!(
                "Not recorded: manufacturer #{id} is absent from IndustrialOrganisations at this hour."
            ),
        },
        (Some(id), Some(Err(reason))) => RecordedMio::Unresolved {
            id,
            trait_count: mio_trait_count,
            reason: format!(
                "Not recorded: IndustrialOrganisations is unavailable at this hour ({reason})."
            ),
        },
        (Some(id), None) => RecordedMio::Unresolved {
            id,
            trait_count: mio_trait_count,
            reason: "Not recorded: IndustrialOrganisations was not read for this design.".to_owned(),
        },
    };

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
            let mut slots = BTreeMap::new();
            let mut unattributed_unresolved = None;
            for row in rows {
                if row.definition_active == 0 {
                    continue;
                }
                let Some(slot) = spell(lexical, row.slot) else {
                    let msg = format!(
                        "Not recorded: active module row slot token {} is not spelled in LexicalTokens.",
                        row.slot
                    );
                    extra_warnings.push(msg.clone());
                    unattributed_unresolved.get_or_insert(msg);
                    continue;
                };
                let Some(module) = spell(lexical, row.definition_token) else {
                    let msg = format!(
                        "Not recorded: fitted module token {} in slot '{slot}' is not spelled in LexicalTokens.",
                        row.definition_token
                    );
                    extra_warnings.push(msg.clone());
                    slots.insert(
                        slot,
                        FittedSlot::Unresolved {
                            token: row.definition_token,
                            reason: msg,
                        },
                    );
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
                slots.insert(
                    slot,
                    FittedSlot::Resolved(FittedModule {
                        module,
                        category,
                        gui_category,
                    }),
                );
            }
            RecordedModules::Available {
                slots,
                unattributed_unresolved,
            }
        }
        Err(reason) => RecordedModules::Unavailable(reason),
    };

    let inherited_upgrades = collect_equipment_upgrades(equipment, equipment_db);
    let upgrades = match upgrade_rows {
        Ok(rows) => {
            let mut upgrade_levels: HashMap<String, u8> = HashMap::new();
            let mut unresolved_reason: Option<String> = None;
            for row in rows {
                match u32::try_from(row.definition_token)
                    .ok()
                    .and_then(|token| spell(lexical, token))
                {
                    Some(key) => {
                        upgrade_levels.insert(key.to_ascii_lowercase(), row.level);
                    }
                    None => {
                        let msg = format!(
                            "Not recorded: upgrade token {} is not spelled in LexicalTokens.",
                            row.definition_token
                        );
                        extra_warnings.push(msg.clone());
                        unresolved_reason.get_or_insert(msg);
                    }
                }
            }
            let entries = inherited_upgrades
                .into_iter()
                .map(|key| {
                    let level = match upgrade_levels.get(&key.to_ascii_lowercase()).copied() {
                        Some(lvl) => Some(lvl),
                        None if unresolved_reason.is_some() => None,
                        None => Some(0),
                    };
                    (key, level)
                })
                .collect();
            RecordedUpgrades::Available {
                entries,
                unresolved_reason,
            }
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
            .map(|entry| {
                let key = spell(lexical, entry.index);
                if key.is_none() {
                    extra_warnings.push(format!(
                        "Not recorded: resource token {} is not spelled in LexicalTokens.",
                        entry.index
                    ));
                }
                RecordedResource {
                    key,
                    token: entry.index,
                    amount: entry.value,
                }
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
        category_mask: definition.classifier_flags,
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
        extra_warnings,
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

fn discover_upgrade_slots(root: &Element, kind: DesignerKind) -> Vec<usize> {
    let mut upgrade_slots: Vec<usize> = find_direct(root, "equipment_modules")
        .map(|modules| {
            modules
                .children
                .iter()
                .filter_map(|child| {
                    element_name(child)
                        .strip_prefix("equipment_upgrade_")
                        .and_then(|suffix| suffix.parse::<usize>().ok())
                })
                .collect()
        })
        .unwrap_or_default();
    upgrade_slots.sort_unstable();
    upgrade_slots.dedup();
    if upgrade_slots.is_empty() && matches!(kind, DesignerKind::Tank) {
        upgrade_slots.extend([0, 1]);
    }
    upgrade_slots
}

fn reads_upgrades(kind: DesignerKind, root: Option<&Element>) -> bool {
    match kind {
        DesignerKind::Upgrade(_) => true,
        DesignerKind::Ship | DesignerKind::Tank | DesignerKind::Plane => {
            root.is_some_and(|root| !discover_upgrade_slots(root, kind).is_empty())
        }
    }
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

fn resolve_full_title(data: &EquipmentDesignData, localisation: &Localisation) -> String {
    let display = resolve_display_name(data, localisation);
    let suffix = if data.version_suffix.is_empty() {
        String::new()
    } else {
        localisation
            .get(&data.version_suffix)
            .map(strip_markup)
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| data.version_suffix.clone())
    };
    if data.append_version && !suffix.is_empty() {
        format!("{display} {suffix}")
    } else {
        display
    }
}

fn loc_opt(localisation: &Localisation, key: &str) -> Option<String> {
    localisation
        .get(key)
        .map(|raw| expand_loc_refs(localisation, &strip_markup(raw), 4))
        .filter(|text| !text.is_empty())
}

pub fn resolve_tank_role_and_frame(
    _archetype_key: &str,
    category: u64,
    localisation: &Localisation,
) -> Option<(String, Option<i32>)> {
    if category == 0 {
        return None;
    }
    let names = category_names(category).ok()?;
    let mut fallback_cat: Option<String> = None;
    for name in &names {
        let designer_key = format!("tank_designer_{name}");
        if let Some(label) = loc_opt(localisation, &designer_key) {
            return Some((label, None));
        }
        if let Some(label) = loc_opt(localisation, name) {
            fallback_cat.get_or_insert(label);
        }
    }
    fallback_cat.map(|label| (label, None))
}

pub fn resolve_plane_role(category: u64, localisation: &Localisation) -> Option<String> {
    if category == 0 {
        return None;
    }
    let names = category_names(category).ok()?;
    let mut fallback_cat: Option<String> = None;
    for name in &names {
        let designer_key = format!("plane_designer_{name}");
        if let Some(label) = loc_opt(localisation, &designer_key) {
            return Some(label);
        }
        if let Some(label) = loc_opt(localisation, name) {
            fallback_cat.get_or_insert(label);
        }
    }
    fallback_cat
}

pub fn format_resources_text(
    build_resources: Option<&[RecordedResource]>,
    resource_db: &ResourceDatabase,
    localisation: &Localisation,
    warnings: &mut Vec<String>,
) -> String {
    let Some(resources) = build_resources else {
        return NOT_RECORDED_VALUE.to_owned();
    };
    let mut out = String::new();
    let numbered = resource_db.numbered();
    for entry in resources {
        let amount = (entry.amount as f64 / FIXED5_SCALE).round() as i64;
        if amount <= 0 {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        let Some(token) = entry.key.as_deref() else {
            out.push_str(&format!("{amount} {NOT_RECORDED_VALUE}"));
            warnings.push(format!(
                "Resource token {} is not spelled in LexicalTokens; showing amount with {NOT_RECORDED_VALUE}.",
                entry.token
            ));
            continue;
        };
        if let Some(resource) = numbered
            .iter()
            .find(|row| row.token.eq_ignore_ascii_case(token))
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

fn format_production_cost(
    kind: DesignerKind,
    build_cost: Option<i64>,
    localisation: &Localisation,
) -> String {
    let label = loc_or(localisation, "STAT_COMMON_BUILD_COST_IC");
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
    icon: &ResolvedEquipmentIcon,
    resources: &dyn Resources,
    tooltips: &mut BTreeMap<String, GameWindowTooltip>,
) {
    let Some(control) = find_direct(root, control_name) else {
        return;
    };
    match icon {
        ResolvedEquipmentIcon::Ready { icon, .. } => {
            let Ok(bound) = instance.control(root, control) else {
                return;
            };
            match icon {
                Icon::Sprite(name) => {
                    bound.sprite = Some(name.as_bytes().to_vec());
                }
                Icon::Path(path) => {
                    if let Some(TextureFile::Loaded(texture)) =
                        resources.texture_file(path.as_bytes())
                    {
                        bound.texture = Some(texture);
                    } else {
                        let _ = instance.hide(root, control);
                    }
                }
            }
        }
        ResolvedEquipmentIcon::Missing { reason }
        | ResolvedEquipmentIcon::Failed { reason } => {
            let _ = instance.hide(root, control);
            if let Some(parent) = control_name
                .rsplit_once('/')
                .and_then(|(prefix, _)| find_direct(root, prefix))
            {
                tooltips.insert(
                    parent.path.clone(),
                    GameWindowTooltip {
                        title: "Icon missing".to_owned(),
                        body: Some(reason.clone()),
                        detail: None,
                    },
                );
            }
        }
        ResolvedEquipmentIcon::Refused { reason, construct } => {
            let _ = instance.hide(root, control);
            if let Some(parent) = control_name
                .rsplit_once('/')
                .and_then(|(prefix, _)| find_direct(root, prefix))
            {
                tooltips.insert(
                    parent.path.clone(),
                    GameWindowTooltip {
                        title: format!("Icon refused ({construct})"),
                        body: Some(reason.clone()),
                        detail: None,
                    },
                );
            }
        }
        ResolvedEquipmentIcon::NoneExpected => {
            let _ = instance.hide(root, control);
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

struct MainWindowContext<'a> {
    data: &'a EquipmentDesignData,
    resource_db: &'a ResourceDatabase,
    equipment_icon: &'a ResolvedEquipmentIcon,
    mio_sprite: Option<&'a str>,
}

fn configure_main_window(
    root: &Element,
    resources: &dyn Resources,
    localisation: &Localisation,
    instance: &mut Instance,
    ctx: MainWindowContext<'_>,
    tooltips: &mut BTreeMap<String, GameWindowTooltip>,
) {
    let MainWindowContext {
        data,
        resource_db,
        equipment_icon,
        mio_sprite,
    } = ctx;
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
    let mio_role_body = loc_or(localisation, "INDUSTRIAL_ORG_LIST_WINDOW_TITLE");

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
        if let Some(res_el) = find_direct(root, "production_resources") {
            if data.build_resources.is_none() {
                tooltips.insert(
                    res_el.path.clone(),
                    GameWindowTooltip {
                        title: "Not recorded".to_owned(),
                        body: Some(
                            data.design_state_reason
                                .clone()
                                .unwrap_or_else(|| NOT_RECORDED_TOOLTIP.to_owned()),
                        ),
                        detail: None,
                    },
                );
            } else if let Some(list) = data.build_resources.as_deref()
                && list.iter().any(|entry| entry.key.is_none())
            {
                let missing_tokens: Vec<String> = list
                    .iter()
                    .filter(|entry| entry.key.is_none())
                    .map(|entry| entry.token.to_string())
                    .collect();
                tooltips.insert(
                    res_el.path.clone(),
                    GameWindowTooltip {
                        title: "Not recorded".to_owned(),
                        body: Some(format!(
                            "Not recorded: resource token(s) {} have no spelling in LexicalTokens.",
                            missing_tokens.join(", ")
                        )),
                        detail: None,
                    },
                );
            }
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
                    title: loc_or(localisation, "STAT_COMMON_BUILD_COST_IC")
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
            match data.auto_upgraded {
                Some(checked) => {
                    if let Ok(bound) = instance.control(root, check) {
                        bound.frame = Some(if checked { 2 } else { 1 });
                        bound.enabled = Some(false);
                    }
                }
                None => {
                    let _ = instance.hide(root, check);
                }
            }
        }

        apply_equipment_icon(
            instance,
            root,
            "icon_selection_container/btn_equipment_icon",
            equipment_icon,
            resources,
            tooltips,
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

        let has_mio = match &data.mio {
            RecordedMio::Resolved(mio) => {
                let body = if mio_sprite.is_some() {
                    mio_role_body
                } else {
                    format!("{mio_role_body} (emblem art missing in loaded interface registry)")
                };
                let detail = mio
                    .trait_count
                    .map(|count| format!("{count} applied MIO traits"));
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
                            detail,
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
                            detail,
                        },
                    );
                }
                true
            }
            RecordedMio::Unresolved {
                id,
                trait_count,
                reason,
            } => {
                let detail = trait_count.map(|count| format!("{count} applied MIO traits"));
                let tip = GameWindowTooltip {
                    title: format!("{NOT_RECORDED_VALUE} (MIO #{id})"),
                    body: Some(reason.clone()),
                    detail,
                };
                if let Some(team_button) = find_direct(root, "design_team_button") {
                    if let Ok(bound) = instance.control(root, team_button) {
                        bound.sprite = Some(b"GFX_idea_unknown".to_vec());
                        bound.enabled = Some(false);
                    }
                    tooltips.insert(team_button.path.clone(), tip);
                    hide_by_name(instance, root, "design_company_icon");
                } else if let Some(company) = find_direct(root, "design_company_icon") {
                    if let Ok(bound) = instance.control(root, company) {
                        bound.sprite = Some(b"GFX_idea_unknown".to_vec());
                    }
                    tooltips.insert(company.path.clone(), tip);
                }
                true
            }
            RecordedMio::None => {
                hide_by_name(instance, root, "design_company_icon");
                false
            }
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
                            title: loc_or(localisation, "SHIP_DESIGNER_NAVAL_DOMINANCE_NAME"),
                            body: Some(NOT_RECORDED_TOOLTIP.to_owned()),
                            detail: None,
                        },
                    );
                }
                if let Some(preview_btn) = find_direct(root, "toggle_model_selector_button") {
                    tooltips.insert(
                        preview_btn.path.clone(),
                        GameWindowTooltip {
                            title: loc_or(localisation, "SELECT_3D_MODEL"),
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
                                        "Tank role could not be determined from witnessed classification for '{}'.",
                                        data.equipment_key
                                    )),
                                    detail: None,
                                },
                            );
                        }
                    }
                }
                disable_by_name(instance, root, "dropdown_tank_roles/expand_button");

                if let Some(preview_box) = find_direct(root, "border_container") {
                    tooltips.insert(
                        preview_box.path.clone(),
                        GameWindowTooltip {
                            title: loc_or(localisation, "SELECT_3D_MODEL"),
                            body: Some(PREVIEW_3D_TOOLTIP.to_owned()),
                            detail: None,
                        },
                    );
                }
                if let Some(select_btn) = find_direct(root, "select3d_button") {
                    tooltips.insert(
                        select_btn.path.clone(),
                        GameWindowTooltip {
                            title: loc_or(localisation, "SELECT_3D_MODEL"),
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
                                        "Plane role could not be determined from witnessed classification for '{}'.",
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
                            title: loc_or(localisation, "SELECT_3D_MODEL"),
                            body: Some(PREVIEW_3D_TOOLTIP.to_owned()),
                            detail: None,
                        },
                    );
                }
            }
            DesignerKind::Upgrade(_) => {}
        }

        let upgrade_slots = discover_upgrade_slots(root, data.kind);
        for slot_idx in upgrade_slots {
            let container_name = format!("equipment_modules/equipment_upgrade_{slot_idx}");
            match &data.upgrades {
                RecordedUpgrades::Available {
                    entries,
                    unresolved_reason,
                } => {
                    if let Some((upgrade_key, maybe_level)) = entries.get(slot_idx) {
                        let label = loc_or(localisation, upgrade_key);
                        set_text_by_name(
                            instance,
                            root,
                            &format!("{container_name}/name"),
                            label.clone(),
                        );
                        match maybe_level {
                            Some(level) => {
                                set_text_by_name(
                                    instance,
                                    root,
                                    &format!("{container_name}/level"),
                                    level.to_string(),
                                );
                            }
                            None => {
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
                                            body: Some(
                                                unresolved_reason
                                                    .clone()
                                                    .unwrap_or_else(|| NOT_RECORDED_TOOLTIP.to_owned()),
                                            ),
                                            detail: None,
                                        },
                                    );
                                }
                            }
                        }
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

        match &data.mio {
            RecordedMio::Resolved(mio) => {
                if let Some(roster) = find_direct(root, "change_design_team/open_roster") {
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
                    let detail = mio
                        .trait_count
                        .map(|count| format!("{count} applied MIO traits"));
                    tooltips.insert(
                        roster.path.clone(),
                        GameWindowTooltip {
                            title: loc_or(localisation, &mio.name),
                            body: Some(body),
                            detail,
                        },
                    );
                }
            }
            RecordedMio::Unresolved {
                id,
                trait_count,
                reason,
            } => {
                if let Some(roster) = find_direct(root, "change_design_team/open_roster") {
                    if let Ok(bound) = instance.control(root, roster) {
                        bound.sprite = Some(b"GFX_idea_unknown".to_vec());
                    }
                    let detail = trait_count.map(|count| format!("{count} applied MIO traits"));
                    tooltips.insert(
                        roster.path.clone(),
                        GameWindowTooltip {
                            title: format!("{NOT_RECORDED_VALUE} (MIO #{id})"),
                            body: Some(reason.clone()),
                            detail,
                        },
                    );
                }
            }
            RecordedMio::None => {}
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

struct SubWindowContext<'a> {
    data: &'a EquipmentDesignData,
    equipment_db: &'a Database,
    resource_db: &'a ResourceDatabase,
    blueprint_name: Option<&'a str>,
    equipment_icon: &'a ResolvedEquipmentIcon,
}

struct StatGridSpec<'a> {
    stat_window_name: &'a str,
    value_child: &'a str,
    lines: &'a [StatLine],
}

fn compose_sub_windows(
    main_layout: &Layout,
    root: &Element,
    guis: &native::Guis,
    resources: &dyn Resources,
    localisation: &Localisation,
    ctx: SubWindowContext<'_>,
) -> Result<ComposedPlan, GameWindowError> {
    let SubWindowContext {
        data,
        equipment_db,
        resource_db,
        blueprint_name,
        equipment_icon,
    } = ctx;
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
            (data, equipment_db),
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
                StatGridSpec {
                    stat_window_name: "stat_entry",
                    value_child: "value",
                    lines,
                },
                resources,
                localisation,
                &mut plan,
            )?;
        }

        compose_unknown_auto_upgrade(
            main_layout,
            root,
            stat_root,
            resources,
            localisation,
            data,
            &mut plan,
        )?;
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
            StatGridSpec {
                stat_window_name: "equipment_designer_stat_entry",
                value_child: "count",
                lines: &columns.base,
            },
            resources,
            localisation,
            &mut plan,
        )?;
    }

    Ok(plan)
}

fn compose_unknown_auto_upgrade(
    main_layout: &Layout,
    root: &Element,
    stat_root: &Element,
    resources: &dyn Resources,
    localisation: &Localisation,
    data: &EquipmentDesignData,
    plan: &mut ComposedPlan,
) -> Result<(), GameWindowError> {
    if data.auto_upgraded.is_some() {
        return Ok(());
    }
    let Some(check) = find_direct(root, "auto_upgraded_checkbox_button") else {
        return Ok(());
    };
    let (Some(root_holder), Fields::Button(btn)) =
        (container_holder(main_layout, &root.path), &check.fields)
    else {
        return Ok(());
    };
    let pos = btn.position;
    let viewport = DEFAULT_VIEWPORT.map(|value| value as i32);
    let na_layout = layout_window(stat_root, resources, viewport, |instance| {
        set_text_by_name(instance, stat_root, "label", NOT_RECORDED_VALUE.to_owned());
        set_text_by_name(instance, stat_root, "value", String::new());
    })?;
    let tip = GameWindowTooltip {
        title: loc_or(
            localisation,
            "EQUIPMENT_DESIGNER_AUTO_UPGRADED_CHECKBOX_BUTTON_DESC",
        )
        .lines()
        .next()
        .unwrap_or("EQUIPMENT_DESIGNER_AUTO_UPGRADED_CHECKBOX_BUTTON_DESC")
        .to_owned(),
        body: Some(
            data.design_state_reason
            .clone()
            .unwrap_or_else(|| NOT_RECORDED_TOOLTIP.to_owned()),
        ),
        detail: None,
    };
    plan.sub_windows.push(ComposedSubWindow {
        before_part: root_holder.before,
        parts: root_holder.place(pos, &na_layout),
        regions: Vec::new(),
        diagnostics: na_layout.diagnostics,
        root_window: "stat_entry".to_owned(),
        path_prefix: "auto_upgraded_missing/".to_owned(),
        close_paths: Vec::new(),
        tooltip_overrides: BTreeMap::new(),
    });
    plan.extra_regions.push(GameWindowRegion {
        path: check.path.clone(),
        rect: [
            root_holder.origin[0] + pos[0] as f32,
            root_holder.origin[1] + pos[1] as f32,
            na_layout.size[0] as f32,
            na_layout.size[1] as f32,
        ],
        clip: root_holder.clip.map(|[l, t, r, b]| {
            [l as f32, t as f32, (r - l).max(0) as f32, (b - t).max(0) as f32]
        }),
        disabled: true,
        close: false,
        passthrough: false,
        view: None,
        tooltip: Some(tip),
        alpha_mask: None,
    });
    Ok(())
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
        RecordedModules::Available { slots, .. } => Some(slots),
        RecordedModules::Unavailable(_) => None,
    };
    for slot_container in &slots_container.children {
        if slot_container.kind != Kind::Container {
            continue;
        }
        let slot_name = element_name(slot_container);
        let fitted = fitted_map.and_then(|map| match map.get(slot_name) {
            Some(FittedSlot::Resolved(m)) => Some(m),
            _ => None,
        });
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

fn compose_module_slots(
    main_layout: &Layout,
    root: &Element,
    slot_root: &Element,
    resources: &dyn Resources,
    localisation: &Localisation,
    (data, equipment_db): (&EquipmentDesignData, &Database),
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

    let empty_slot_body = loc_or(localisation, "EQUIPMENT_DESIGNER_SLOT_EMPTY_TOOLTIP");
    let locked_slot_title = loc_or(localisation, "EQUIPMENT_DESIGNER_MODULE_SLOT_NOT_AVAILABLE");

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
                    body: Some(reason.clone()),
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
            reason: format!(
                "Not recorded: fitted module is unavailable at this hour ({reason})."
            ),
        },
        RecordedModules::Available {
            slots,
            unattributed_unresolved,
        } => match slots.get(slot_key) {
            Some(FittedSlot::Resolved(fitted)) => {
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
            }
            Some(FittedSlot::Unresolved { reason, .. }) => SlotIcon::Unknown {
                slot: slot_key.to_owned(),
                sprite: "GFX_EMI_unknown".to_owned(),
                reason: reason.clone(),
            },
            None => {
                if let Some(reason) = unattributed_unresolved {
                    SlotIcon::Unknown {
                        slot: slot_key.to_owned(),
                        sprite: "GFX_EMI_unknown".to_owned(),
                        reason: reason.clone(),
                    }
                } else {
                    let sprite = resolve_empty_slot_sprite(slot_key, slot_gfx, has_sprite);
                    SlotIcon::Empty {
                        slot: slot_key.to_owned(),
                        sprite,
                    }
                }
            }
        },
    }
}

fn configure_upgrade_view(
    sub_root: &Element,
    resources: &dyn Resources,
    localisation: &Localisation,
    instance: &mut Instance,
    data: &EquipmentDesignData,
    equipment_icon: &ResolvedEquipmentIcon,
    tooltips: &mut BTreeMap<String, GameWindowTooltip>,
) {
    apply_equipment_icon(instance, sub_root, "archetype", equipment_icon, resources, tooltips);
    match &data.upgrades {
        RecordedUpgrades::Available {
            entries,
            unresolved_reason,
        } => {
            let upgrade_map: HashMap<String, (&str, Option<u8>)> = entries
                .iter()
                .map(|(key, level)| (key.to_ascii_lowercase(), (key.as_str(), *level)))
                .collect();
            for child in &sub_root.children {
                if child.kind != Kind::Container {
                    continue;
                }
                let child_name = element_name(child);
                let name_lower = child_name.to_ascii_lowercase();
                if let Some((upgrade_key, maybe_level)) = upgrade_map.get(&name_lower) {
                    let label = loc_or(localisation, upgrade_key);
                    set_text_by_name(
                        instance,
                        sub_root,
                        &format!("{child_name}/label"),
                        label.clone(),
                    );
                    match maybe_level {
                        Some(level) => {
                            set_text_by_name(
                                instance,
                                sub_root,
                                &format!("{child_name}/level"),
                                level.to_string(),
                            );
                        }
                        None => {
                            set_text_by_name(
                                instance,
                                sub_root,
                                &format!("{child_name}/level"),
                                NOT_RECORDED_VALUE.to_owned(),
                            );
                            if let Some(level_el) =
                                find_direct(sub_root, &format!("{child_name}/level"))
                            {
                                tooltips.insert(
                                    level_el.path.clone(),
                                    GameWindowTooltip {
                                        title: label,
                                        body: Some(
                                            unresolved_reason
                                                .clone()
                                                .unwrap_or_else(|| NOT_RECORDED_TOOLTIP.to_owned()),
                                        ),
                                        detail: None,
                                    },
                                );
                            }
                        }
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

fn resolve_grid_metrics(
    grid: &native::Grid,
    holder: &native_gui::Holder,
    stat_root: &Element,
) -> (usize, i32, i32, bool) {
    let default_w = stat_root
        .container()
        .map(|c| c.size.size[0].value)
        .filter(|&w| w > 0)
        .unwrap_or(100);
    let default_h = stat_root
        .container()
        .map(|c| c.size.size[1].value)
        .filter(|&h| h > 0)
        .unwrap_or(20);
    let resolve_axis = |coord: native::Coordinate, parent: i32, fallback: i32| -> i32 {
        let px = if coord.percent && parent > 0 {
            (parent as f64 * coord.value as f64 / 100.0).round() as i32
        } else {
            coord.value
        };
        if px > 0 { px } else { fallback }
    };
    let slot_width = resolve_axis(grid.slot_size.size[0], holder.content[0], default_w);
    let slot_height = resolve_axis(grid.slot_size.size[1], holder.content[1], default_h);
    let max_h = grid.max_slots[0];
    let columns = if max_h > 0 {
        max_h as usize
    } else if holder.content[0] > 0 && slot_width > 0 {
        (holder.content[0] / slot_width).max(1) as usize
    } else {
        1
    };
    (columns, slot_width, slot_height, grid.add_horizontal)
}

fn compose_stat_grid(
    main_layout: &Layout,
    grid_el: &Element,
    stat_root: &Element,
    spec: StatGridSpec<'_>,
    resources: &dyn Resources,
    localisation: &Localisation,
    plan: &mut ComposedPlan,
) -> Result<(), GameWindowError> {
    let StatGridSpec {
        stat_window_name,
        value_child,
        lines,
    } = spec;
    let label_el = find_direct(stat_root, "label").ok_or_else(|| {
        GameWindowError::refused(format!(
            "Stat entry template '{stat_window_name}' is missing required child 'label' for grid '{}'.",
            grid_el.path
        ))
    })?;
    let val_el = find_direct(stat_root, value_child).ok_or_else(|| {
        GameWindowError::refused(format!(
            "Stat entry template '{stat_window_name}' is missing required child '{value_child}' for grid '{}'.",
            grid_el.path
        ))
    })?;
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
    let Fields::Grid(grid) = &grid_el.fields else {
        return Err(GameWindowError::refused(format!(
            "Designer stat grid '{}' is not a gridBoxType.",
            grid_el.path
        )));
    };
    let grid_offset = [grid.position.x.value, grid.position.y.value];
    let (columns, slot_width, slot_height, add_horizontal) =
        resolve_grid_metrics(grid, holder, stat_root);
    let max_v = if grid.max_slots[1] > 0 {
        grid.max_slots[1] as usize
    } else {
        lines.len().div_ceil(columns).max(1)
    };

    let viewport = DEFAULT_VIEWPORT.map(|value| value as i32);
    for (index, line) in lines.iter().enumerate() {
        let (col, row) = if add_horizontal {
            ((index % columns) as i32, (index / columns) as i32)
        } else {
            ((index / max_v) as i32, (index % max_v) as i32)
        };
        let place = [
            grid_offset[0] + col * slot_width,
            grid_offset[1] + row * slot_height,
        ];
        let (full_label, missing_loc_note) = match loc_opt(localisation, line.label_key) {
            Some(text) => (text, None),
            None => {
                plan.warnings.push(format!(
                    "Localisation key '{}' is missing from the recorded localisation.",
                    line.label_key
                ));
                (
                    line.label_key.to_owned(),
                    Some(format!(
                        "Missing localisation key '{}'.",
                        line.label_key
                    )),
                )
            }
        };
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
        let desc_text = loc_opt(localisation, &desc_key);
        let body = line.detail.clone().or(desc_text.clone()).or(missing_loc_note.clone());
        let detail = if line.detail.is_some() {
            desc_text.or(missing_loc_note)
        } else {
            missing_loc_note
        };
        let tooltip = GameWindowTooltip {
            title: full_label.trim_end_matches(':').trim().to_owned(),
            body,
            detail,
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
        overrides.insert(label_el.path.clone(), tooltip.clone());
        overrides.insert(val_el.path.clone(), tooltip);
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
