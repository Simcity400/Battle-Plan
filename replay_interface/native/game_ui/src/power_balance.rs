//! XUI18 power-balance definition, runtime admission, and presentation law.
//!
//! Static balance/range/modifier structure comes from the effective
//! `common/bop` catalog. Live value, side identities and country membership
//! come only from `replay_core::PowerBalanceStateExactView`. Engine-resolved runtime
//! leaves that cannot be reconstructed from those sources are supplied by an
//! authenticated all-country supplement and are rejected when incomplete.

use crate::assets::AssetResolver;
use crate::compose::ComposeOptions;
use crate::decision_chrome::{
    decision_card_binding, DecisionAvailability, DecisionCardBinding, DecisionSet,
    DecisionStateEntry,
};
use replay_core::{
    PowerBalanceStateExactView, PowerBalanceStateRecordExact, RuntimeTextIdentitySource,
    ViewStatus, A729_POWER_BALANCE_ALL_COUNTRY_PRESENTATION_MANIFEST_SHA256,
};
use serde::Deserialize;
use serde_json::Value;
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq)]
pub struct Threshold {
    pub raw: String,
    pub value: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PowerBalanceRangeDefinition {
    pub id: String,
    pub side_id: Option<String>,
    pub order: usize,
    pub min: Threshold,
    pub max: Threshold,
    pub modifier: BTreeMap<String, Value>,
    pub on_activate: Value,
    pub on_deactivate: Value,
    pub source_file: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PowerBalanceSideDefinition {
    pub id: String,
    pub order: usize,
    pub authored_icon: String,
    pub ranges: Vec<PowerBalanceRangeDefinition>,
    pub source_file: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PowerBalanceDefinition {
    pub id: String,
    pub initial_value: Threshold,
    pub left_side: String,
    pub right_side: String,
    pub decision_category: Option<String>,
    pub neutral_ranges: Vec<PowerBalanceRangeDefinition>,
    pub sides: Vec<PowerBalanceSideDefinition>,
    pub source_file: String,
}

impl PowerBalanceDefinition {
    pub fn range(&self, id: &str) -> Option<&PowerBalanceRangeDefinition> {
        self.neutral_ranges
            .iter()
            .chain(self.sides.iter().flat_map(|side| side.ranges.iter()))
            .find(|range| range.id == id)
    }

    pub fn side(&self, id: &str) -> Option<&PowerBalanceSideDefinition> {
        self.sides.iter().find(|side| side.id == id)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PowerBalanceDefinitionCatalog {
    balances: BTreeMap<String, PowerBalanceDefinition>,
}

impl PowerBalanceDefinitionCatalog {
    pub fn from_catalogs_json(input: &str) -> Result<Self, StaticCatalogError> {
        let root: RawCatalogRoot = serde_json::from_str(input)
            .map_err(|error| StaticCatalogError::Json(error.to_string()))?;
        let mut balances = BTreeMap::new();
        for (key, raw) in root.catalogs.power_balances {
            let definition = convert_definition(&key, raw)?;
            if balances.insert(key.clone(), definition).is_some() {
                return Err(StaticCatalogError::DuplicateBalance(key));
            }
        }
        if balances.is_empty() {
            return Err(StaticCatalogError::EmptyCatalog);
        }
        Ok(Self { balances })
    }

    pub fn from_effective_vfs(resolver: &AssetResolver) -> crate::Result<Self> {
        let files = resolver.list_files_recursive("common/bop", Some("txt"))?;
        let mut balances = BTreeMap::new();
        for file in files {
            let doc = crate::paradox_text::parse_file(&file.path)?;
            if !doc.recoveries.is_empty() {
                return Err(crate::GameUiError::parse(format!(
                    "XUI18 effective definition {:?} required parser recovery",
                    file.rel_path
                )));
            }
            for entry in doc.root.fields() {
                let Some(id) = entry.key.as_deref() else {
                    continue;
                };
                if id.starts_with('@') || entry.value.as_object().is_none() {
                    continue;
                }
                let definition = parse_effective_definition(id, &entry.value, &file.rel_path)?;
                if let Some(previous) = balances.insert(id.to_string(), definition) {
                    return Err(crate::GameUiError::parse(format!(
                        "XUI18 balance {id:?} is defined more than once in the effective VFS: {:?} and {:?}",
                        previous.source_file, file.rel_path
                    )));
                }
            }
        }
        if balances.is_empty() {
            return Err(crate::GameUiError::missing(
                "XUI18 effective common/bop catalog is empty",
            ));
        }
        Ok(Self { balances })
    }

    pub fn len(&self) -> usize {
        self.balances.len()
    }

    pub fn is_empty(&self) -> bool {
        self.balances.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&PowerBalanceDefinition> {
        self.balances.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &PowerBalanceDefinition)> {
        self.balances
            .iter()
            .map(|(id, definition)| (id.as_str(), definition))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StaticCatalogError {
    Json(String),
    EmptyCatalog,
    DuplicateBalance(String),
    InvalidIdentity(String),
    InvalidThreshold(String),
    InvalidSideOrder(String),
    InvalidRangeOrder(String),
    DuplicateRange(String),
    RangeCountMismatch(String),
    MissingDefaultSide(String),
}

#[derive(Deserialize)]
struct RawCatalogRoot {
    catalogs: RawCatalogs,
}

#[derive(Deserialize)]
struct RawCatalogs {
    /// Kept as ordered entries rather than a map so that duplicate textual
    /// keys in the extractor JSON survive deserialization and are refused as
    /// `StaticCatalogError::DuplicateBalance` instead of silently last-winning.
    #[serde(deserialize_with = "deserialize_balance_entries")]
    power_balances: Vec<(String, RawBalance)>,
}

fn deserialize_balance_entries<'de, D>(
    deserializer: D,
) -> Result<Vec<(String, RawBalance)>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct EntriesVisitor;

    impl<'de> serde::de::Visitor<'de> for EntriesVisitor {
        type Value = Vec<(String, RawBalance)>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a map of power balance definitions")
        }

        fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::MapAccess<'de>,
        {
            let mut entries = Vec::with_capacity(access.size_hint().unwrap_or(0));
            while let Some(entry) = access.next_entry::<String, RawBalance>()? {
                entries.push(entry);
            }
            Ok(entries)
        }
    }

    deserializer.deserialize_map(EntriesVisitor)
}

#[derive(Deserialize)]
struct RawBalance {
    id: String,
    initial_value: Value,
    left_side: String,
    right_side: String,
    decision_category: Option<String>,
    #[serde(default)]
    neutral_ranges: Vec<RawRange>,
    sides: Vec<RawSide>,
    range_count: usize,
    source_file: String,
}

#[derive(Deserialize)]
struct RawSide {
    id: String,
    order: usize,
    icon: String,
    ranges: Vec<RawRange>,
    range_count: usize,
    source_file: String,
}

#[derive(Deserialize)]
struct RawRange {
    id: String,
    side_id: Option<String>,
    order: usize,
    min: Value,
    max: Value,
    #[serde(default)]
    modifier: BTreeMap<String, Value>,
    #[serde(default)]
    on_activate: Value,
    #[serde(default)]
    on_deactivate: Value,
    source_file: String,
}

fn convert_definition(
    key: &str,
    raw: RawBalance,
) -> Result<PowerBalanceDefinition, StaticCatalogError> {
    if key.trim().is_empty()
        || raw.id != key
        || raw.left_side.trim().is_empty()
        || raw.right_side.trim().is_empty()
        || raw.left_side == raw.right_side
        || raw.source_file.trim().is_empty()
    {
        return Err(StaticCatalogError::InvalidIdentity(key.to_string()));
    }
    let mut sides = Vec::with_capacity(raw.sides.len());
    let mut range_ids = BTreeSet::new();
    for (expected_order, side) in raw.sides.into_iter().enumerate() {
        if side.order != expected_order
            || side.id.trim().is_empty()
            || side.icon.trim().is_empty()
            || side.source_file.trim().is_empty()
        {
            return Err(StaticCatalogError::InvalidSideOrder(key.to_string()));
        }
        if side.range_count != side.ranges.len() {
            return Err(StaticCatalogError::RangeCountMismatch(side.id));
        }
        let side_id = side.id.clone();
        let ranges = convert_ranges(side.ranges, Some(&side_id), &mut range_ids)?;
        sides.push(PowerBalanceSideDefinition {
            id: side.id,
            order: side.order,
            authored_icon: side.icon,
            ranges,
            source_file: side.source_file,
        });
    }
    if !sides.iter().any(|side| side.id == raw.left_side)
        || !sides.iter().any(|side| side.id == raw.right_side)
    {
        return Err(StaticCatalogError::MissingDefaultSide(key.to_string()));
    }
    let neutral_ranges = convert_ranges(raw.neutral_ranges, None, &mut range_ids)?;
    let actual_count =
        neutral_ranges.len() + sides.iter().map(|side| side.ranges.len()).sum::<usize>();
    if actual_count != raw.range_count {
        return Err(StaticCatalogError::RangeCountMismatch(key.to_string()));
    }
    Ok(PowerBalanceDefinition {
        id: raw.id,
        initial_value: threshold(&raw.initial_value, &format!("{key}.initial_value"))?,
        left_side: raw.left_side,
        right_side: raw.right_side,
        decision_category: raw
            .decision_category
            .filter(|value| !value.trim().is_empty()),
        neutral_ranges,
        sides,
        source_file: raw.source_file,
    })
}

fn convert_ranges(
    ranges: Vec<RawRange>,
    expected_side: Option<&str>,
    ids: &mut BTreeSet<String>,
) -> Result<Vec<PowerBalanceRangeDefinition>, StaticCatalogError> {
    let mut out = Vec::with_capacity(ranges.len());
    for (expected_order, range) in ranges.into_iter().enumerate() {
        if range.order != expected_order
            || range.id.trim().is_empty()
            || range.side_id.as_deref() != expected_side
            || range.source_file.trim().is_empty()
        {
            return Err(StaticCatalogError::InvalidRangeOrder(range.id));
        }
        if !ids.insert(range.id.clone()) {
            return Err(StaticCatalogError::DuplicateRange(range.id));
        }
        let min = threshold(&range.min, &format!("{}.min", range.id))?;
        let max = threshold(&range.max, &format!("{}.max", range.id))?;
        if min.value > max.value {
            return Err(StaticCatalogError::InvalidThreshold(range.id));
        }
        out.push(PowerBalanceRangeDefinition {
            id: range.id,
            side_id: range.side_id,
            order: range.order,
            min,
            max,
            modifier: range.modifier,
            on_activate: range.on_activate,
            on_deactivate: range.on_deactivate,
            source_file: range.source_file,
        });
    }
    Ok(out)
}

fn threshold(value: &Value, context: &str) -> Result<Threshold, StaticCatalogError> {
    let raw = match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => return Err(StaticCatalogError::InvalidThreshold(context.to_string())),
    };
    let parsed = raw
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && (-1.0..=1.0).contains(value))
        .ok_or_else(|| StaticCatalogError::InvalidThreshold(context.to_string()))?;
    Ok(Threshold { raw, value: parsed })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeCadence {
    Daily,
    Weekly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarState {
    StableLeft,
    StableRight,
    MovingLeft,
    MovingRight,
}

impl BarState {
    pub fn sprite(self) -> &'static str {
        match self {
            Self::StableLeft => "GFX_power_balance_left",
            Self::StableRight => "GFX_power_balance_right",
            Self::MovingLeft => "GFX_power_balance_left_moving",
            Self::MovingRight => "GFX_power_balance_right_moving",
        }
    }

    fn is_moving(self) -> bool {
        matches!(self, Self::MovingLeft | Self::MovingRight)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PowerBalanceTrendInput {
    pub cadence: ChangeCadence,
    pub magnitude: f64,
    pub resolved_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPowerBalanceText {
    pub balance_name: String,
    pub left_side_name: String,
    pub right_side_name: String,
    pub active_range_name: String,
    pub balance_tooltip: String,
    pub active_range_tooltip: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerBalanceDecisionInput {
    pub decision_id: String,
    pub source_record_id: String,
    pub availability: DecisionAvailability,
    pub resolved_name: String,
    pub cost_text: Option<String>,
    pub timer_text: Option<String>,
    pub target_tag: Option<String>,
    pub target_flag_sprite: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerBalanceRuntimeTextProvenance {
    pub context_id: String,
    pub frame_hour: i64,
    pub pov_country_or_global: String,
    pub view_id: String,
    pub element_id: String,
    pub setter_entry_rva: u32,
    pub setter_caller_rva: Option<u32>,
    pub formatter_entry_rva: Option<u32>,
    pub identity_source: RuntimeTextIdentitySource,
    /// Exact commitment carried by A25 for `final_text_utf8`.
    pub final_text_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PowerBalanceDecisionTextRole {
    Name,
    Cost,
    Timer,
}

/// Typed edge from one presentation field to the A25 context that produced
/// that field. Context records remain deduplicated; equal fields may lawfully
/// share one context while retaining two distinct edges.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PowerBalanceRuntimeTextSlot {
    LauncherPercentage,
    LauncherLevels,
    Trend,
    ModifierDescription,
    BalanceName,
    LeftSideName,
    RightSideName,
    ActiveRangeName,
    BalanceTooltip,
    ActiveRangeTooltip,
    Decision {
        decision_id: String,
        source_record_id: String,
        role: PowerBalanceDecisionTextRole,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerBalanceRuntimeTextBinding {
    pub slot: PowerBalanceRuntimeTextSlot,
    pub context_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerBalanceRuntimeProvenance {
    pub producer_manifest_sha256: String,
    pub completeness_record_identity: String,
    pub effective_content_manifest_sha256: String,
    pub state_semantic_receipt_sha256: String,
    pub presentation_semantic_receipt_sha256: String,
    pub frame_hour: i64,
    pub expected_country_count: u32,
    pub expected_balance_count: u32,
    pub runtime_text_contexts: Vec<PowerBalanceRuntimeTextProvenance>,
    pub runtime_text_bindings: Vec<PowerBalanceRuntimeTextBinding>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PowerBalanceRuntimeSupplement {
    pub provenance: PowerBalanceRuntimeProvenance,
    pub source_record_id: String,
    pub country_tag: String,
    pub balance_id: String,
    pub manager_ordinal: u32,
    pub display_ordinal: usize,
    pub active_side_id: String,
    pub trending_side_id: String,
    pub active_range_id: String,
    pub effective_left_icon: String,
    pub effective_right_icon: String,
    pub launcher_visible: bool,
    pub launcher_percentage_text: Option<String>,
    pub launcher_levels_text: Option<String>,
    pub launcher_icon_sprite: Option<String>,
    pub launcher_glow_visible: bool,
    pub bar_state: BarState,
    pub range_indicator_frame: usize,
    /// Engine-resolved final local top-left for the code-positioned
    /// `range_indicator` prototype. Its authored offset alone is insufficient
    /// to recover the engine-selected anchor.
    pub range_indicator_top_left: [i32; 2],
    pub trend: Option<PowerBalanceTrendInput>,
    /// Complete resolved output of `GetPowerBalanceModDesc`. An empty string is
    /// a measured empty aggregate, not missing data.
    pub power_balance_modifier_desc: String,
    pub decisions_complete: bool,
    pub decisions: Vec<PowerBalanceDecisionInput>,
    pub resolved_text: ResolvedPowerBalanceText,
}

pub struct PowerBalanceAdmissionRequest<'a> {
    pub country_tag: &'a str,
    pub view: &'a PowerBalanceStateExactView,
    pub catalog: &'a PowerBalanceDefinitionCatalog,
    pub decisions: &'a DecisionSet,
    pub runtime: &'a [PowerBalanceRuntimeSupplement],
}

#[derive(Clone, Debug, PartialEq)]
pub struct RangeBoundaryPresentation {
    pub value: f64,
    pub normalized: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecisionPresentation {
    pub source_record_id: String,
    pub resolved_name: String,
    pub target_flag_sprite: Option<String>,
    pub card: DecisionCardBinding,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PowerBalancePresentation {
    pub provenance: PowerBalanceRuntimeProvenance,
    pub source_record_id: String,
    pub country_tag: String,
    pub balance_id: String,
    pub display_ordinal: usize,
    pub value: f64,
    pub normalized_value: f32,
    pub active_side_id: String,
    pub trending_side_id: String,
    pub active_range_id: String,
    pub active_range_modifier: BTreeMap<String, Value>,
    pub left_icon: String,
    pub right_icon: String,
    pub launcher_visible: bool,
    pub launcher_percentage_text: Option<String>,
    pub launcher_levels_text: Option<String>,
    pub launcher_icon_sprite: Option<String>,
    pub launcher_glow_visible: bool,
    pub bar_sprite: String,
    pub range_indicator_frame: usize,
    pub range_indicator_top_left: [i32; 2],
    pub range_boundaries: Vec<RangeBoundaryPresentation>,
    pub trend: Option<PowerBalanceTrendInput>,
    pub power_balance_modifier_desc: String,
    pub decisions: Vec<DecisionPresentation>,
    pub resolved_text: ResolvedPowerBalanceText,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AdmittedPowerBalanceScreen {
    /// The GUI/GFX are in vanilla in a729. Runtime visibility is gated by an
    /// active measured balance for the current country, not by an entitlement.
    HiddenNoActiveBalance,
    Visible(Vec<PowerBalancePresentation>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PowerBalanceRejection {
    DomainUnavailable,
    InvalidCountry,
    InvalidLiveRow,
    DuplicateLiveBalance,
    MissingDefinition,
    InvalidDynamicSides,
    RuntimeSetMismatch,
    InvalidDisplayOrder,
    MissingRuntimeProvenance,
    InvalidActiveSide,
    InvalidActiveRange,
    InvalidRuntimeIcon,
    InvalidLauncher,
    InvalidBarState,
    InvalidIndicatorFrame,
    InvalidTrend,
    IncompleteDecisions,
    InvalidDecision,
    InvalidResolvedText,
    InvalidRuntimeTextBinding,
}

pub fn admit_power_balance_screen(
    request: PowerBalanceAdmissionRequest<'_>,
) -> Result<AdmittedPowerBalanceScreen, PowerBalanceRejection> {
    if request.country_tag.trim().is_empty() {
        return Err(PowerBalanceRejection::InvalidCountry);
    }
    if request.view.status != ViewStatus::Present {
        return Err(PowerBalanceRejection::DomainUnavailable);
    }

    let state = request
        .view
        .state
        .as_ref()
        .ok_or(PowerBalanceRejection::DomainUnavailable)?;
    let country = state
        .countries
        .iter()
        .find(|country| country.country_tag == request.country_tag)
        .ok_or(PowerBalanceRejection::InvalidCountry)?;
    let first = state
        .records
        .iter()
        .filter(|record| record.member_tag_db_ids.contains(&country.tag_db_id))
        .min_by_key(|record| record.manager_ordinal);
    if first.is_none() && request.runtime.is_empty() {
        return Ok(AdmittedPowerBalanceScreen::HiddenNoActiveBalance);
    }
    if request.runtime.len() != 1 {
        return Err(PowerBalanceRejection::RuntimeSetMismatch);
    }
    let runtime = &request.runtime[0];
    if runtime.display_ordinal != 0 {
        return Err(PowerBalanceRejection::InvalidDisplayOrder);
    }
    let row = measured_live_row(first.ok_or(PowerBalanceRejection::RuntimeSetMismatch)?)?;
    if runtime.provenance.frame_hour != state.frame_hour as i64
        || runtime.provenance.effective_content_manifest_sha256
            != state.effective_content_manifest_sha256
        || runtime.provenance.state_semantic_receipt_sha256 != state.semantic_receipt_sha256
        || runtime.provenance.expected_country_count != state.expected_country_count
        || runtime.manager_ordinal != row.manager_ordinal
    {
        return Err(PowerBalanceRejection::MissingRuntimeProvenance);
    }
    let presentation = admit_one(
        request.country_tag,
        &row,
        request
            .catalog
            .get(&runtime.balance_id)
            .ok_or(PowerBalanceRejection::MissingDefinition)?,
        request.decisions,
        runtime,
    )?;
    Ok(AdmittedPowerBalanceScreen::Visible(vec![presentation]))
}

struct MeasuredLiveRow<'a> {
    template: &'a str,
    manager_ordinal: u32,
    value: f64,
    left_side: &'a str,
    right_side: &'a str,
    trending_side: &'a str,
    active_range: &'a str,
    effective_catalog_sha256: &'a str,
}

fn parse_effective_definition(
    id: &str,
    value: &crate::paradox_text::Value,
    source_file: &str,
) -> crate::Result<PowerBalanceDefinition> {
    let required = |key: &str| {
        value
            .first_scalar_ci(key)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .ok_or_else(|| crate::GameUiError::parse(format!("XUI18 {id}.{key} is missing")))
    };
    let left_side = required("left_side")?;
    let right_side = required("right_side")?;
    if left_side == right_side {
        return Err(crate::GameUiError::parse(format!(
            "XUI18 {id} has identical default sides"
        )));
    }
    let initial_value = effective_threshold(
        value.first_scalar_ci("initial_value").ok_or_else(|| {
            crate::GameUiError::parse(format!("XUI18 {id}.initial_value is missing"))
        })?,
        &format!("{id}.initial_value"),
    )?;
    let mut range_ids = BTreeSet::new();
    let neutral_ranges =
        parse_effective_ranges(value.values_ci("range"), None, source_file, &mut range_ids)?;
    let mut sides = Vec::new();
    for (order, side) in value.values_ci("side").enumerate() {
        let side_id = side
            .first_scalar_ci("id")
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| crate::GameUiError::parse(format!("XUI18 {id} side id is missing")))?;
        let icon = side
            .first_scalar_ci("icon")
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                crate::GameUiError::parse(format!("XUI18 {id}.{side_id} icon is missing"))
            })?;
        sides.push(PowerBalanceSideDefinition {
            id: side_id.to_string(),
            order,
            authored_icon: icon.to_string(),
            ranges: parse_effective_ranges(
                side.values_ci("range"),
                Some(side_id),
                source_file,
                &mut range_ids,
            )?,
            source_file: source_file.to_string(),
        });
    }
    if !sides.iter().any(|side| side.id == left_side)
        || !sides.iter().any(|side| side.id == right_side)
    {
        return Err(crate::GameUiError::parse(format!(
            "XUI18 {id} default sides are absent from the effective definition"
        )));
    }
    Ok(PowerBalanceDefinition {
        id: id.to_string(),
        initial_value,
        left_side,
        right_side,
        decision_category: value
            .first_scalar_ci("decision_category")
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string),
        neutral_ranges,
        sides,
        source_file: source_file.to_string(),
    })
}

fn parse_effective_ranges<'a>(
    values: impl Iterator<Item = &'a crate::paradox_text::Value>,
    side_id: Option<&str>,
    source_file: &str,
    range_ids: &mut BTreeSet<String>,
) -> crate::Result<Vec<PowerBalanceRangeDefinition>> {
    let mut ranges = Vec::new();
    for (order, range) in values.enumerate() {
        let id = range
            .first_scalar_ci("id")
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| crate::GameUiError::parse("XUI18 range id is missing"))?;
        if !range_ids.insert(id.to_string()) {
            return Err(crate::GameUiError::parse(format!(
                "XUI18 duplicate range {id:?}"
            )));
        }
        let parse_bound = |key: &str| {
            effective_threshold(
                range.first_scalar_ci(key).ok_or_else(|| {
                    crate::GameUiError::parse(format!("XUI18 {id}.{key} is missing"))
                })?,
                &format!("{id}.{key}"),
            )
        };
        let min = parse_bound("min")?;
        let max = parse_bound("max")?;
        if min.value > max.value {
            return Err(crate::GameUiError::parse(format!(
                "XUI18 {id} has min greater than max"
            )));
        }
        ranges.push(PowerBalanceRangeDefinition {
            id: id.to_string(),
            side_id: side_id.map(str::to_string),
            order,
            min,
            max,
            modifier: clausewitz_object_to_json_map(range.first_value_ci("modifier"))?,
            on_activate: clausewitz_to_json(range.first_value_ci("on_activate")),
            on_deactivate: clausewitz_to_json(range.first_value_ci("on_deactivate")),
            source_file: source_file.to_string(),
        });
    }
    Ok(ranges)
}

fn effective_threshold(raw: &str, context: &str) -> crate::Result<Threshold> {
    let value = raw.parse::<f64>().map_err(|_| {
        crate::GameUiError::parse(format!("XUI18 {context} has invalid threshold {raw:?}"))
    })?;
    if !value.is_finite() || !(-1.0..=1.0).contains(&value) {
        return Err(crate::GameUiError::parse(format!(
            "XUI18 {context} threshold {raw:?} is outside [-1,1]"
        )));
    }
    Ok(Threshold {
        raw: raw.to_string(),
        value,
    })
}

fn clausewitz_object_to_json_map(
    value: Option<&crate::paradox_text::Value>,
) -> crate::Result<BTreeMap<String, Value>> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let object = clausewitz_to_json(Some(value));
    serde_json::from_value(object)
        .map_err(|error| crate::GameUiError::parse(format!("XUI18 modifier parse failed: {error}")))
}

fn clausewitz_to_json(value: Option<&crate::paradox_text::Value>) -> Value {
    let Some(value) = value else {
        return Value::Object(Default::default());
    };
    match value {
        crate::paradox_text::Value::Scalar(value) => Value::String(match value {
            crate::paradox_text::Scalar::Bare(value)
            | crate::paradox_text::Scalar::Quoted(value) => value.clone(),
        }),
        crate::paradox_text::Value::Object(entries) => {
            let mut out = serde_json::Map::new();
            for entry in entries {
                if let Some(key) = entry.key.as_ref() {
                    out.insert(key.clone(), clausewitz_to_json(Some(&entry.value)));
                }
            }
            Value::Object(out)
        }
    }
}

fn measured_live_row(
    row: &PowerBalanceStateRecordExact,
) -> Result<MeasuredLiveRow<'_>, PowerBalanceRejection> {
    let value = row.value_raw_fixed5 as f64 / 100_000.0;
    if row.balance_id.trim().is_empty()
        || row.left_side_id.trim().is_empty()
        || row.right_side_id.trim().is_empty()
        || row.left_side_id == row.right_side_id
        || (row.trending_side_id != row.left_side_id && row.trending_side_id != row.right_side_id)
        || !value.is_finite()
        || !(-1.0..=1.0).contains(&value)
        || row.active_range_id.trim().is_empty()
        || !crate::is_lowercase_sha256(&row.effective_catalog_sha256)
    {
        return Err(PowerBalanceRejection::InvalidLiveRow);
    }
    Ok(MeasuredLiveRow {
        template: &row.balance_id,
        manager_ordinal: row.manager_ordinal,
        value,
        left_side: &row.left_side_id,
        right_side: &row.right_side_id,
        trending_side: &row.trending_side_id,
        active_range: &row.active_range_id,
        effective_catalog_sha256: &row.effective_catalog_sha256,
    })
}

fn admit_one(
    country_tag: &str,
    row: &MeasuredLiveRow<'_>,
    definition: &PowerBalanceDefinition,
    decisions: &DecisionSet,
    runtime: &PowerBalanceRuntimeSupplement,
) -> Result<PowerBalancePresentation, PowerBalanceRejection> {
    let mut context_ids = BTreeSet::new();
    let mut context_hour = None;
    let provenance_valid = runtime.provenance.producer_manifest_sha256
        == A729_POWER_BALANCE_ALL_COUNTRY_PRESENTATION_MANIFEST_SHA256
        && !runtime
            .provenance
            .completeness_record_identity
            .trim()
            .is_empty()
        && runtime.provenance.effective_content_manifest_sha256 == row.effective_catalog_sha256
        && crate::is_lowercase_sha256(&runtime.provenance.state_semantic_receipt_sha256)
        && crate::is_lowercase_sha256(&runtime.provenance.presentation_semantic_receipt_sha256)
        && runtime.provenance.expected_country_count > 0
        && runtime.provenance.expected_balance_count > 0
        && !runtime.provenance.runtime_text_contexts.is_empty()
        && runtime
            .provenance
            .runtime_text_contexts
            .iter()
            .all(|context| {
                let same_hour = match context_hour {
                    Some(hour) => hour == context.frame_hour,
                    None => {
                        context_hour = Some(context.frame_hour);
                        true
                    }
                };
                !context.context_id.trim().is_empty()
                    && context_ids.insert(context.context_id.as_str())
                    && same_hour
                    && (context.pov_country_or_global == country_tag
                        || context.pov_country_or_global == "global")
                    && !context.view_id.trim().is_empty()
                    && !context.element_id.trim().is_empty()
                    && context.setter_entry_rva == replay_core::A729_TEXT_SETTER_RVA
                    && crate::is_lowercase_sha256(&context.final_text_sha256)
            })
        && context_hour == Some(runtime.provenance.frame_hour);
    if !provenance_valid
        || runtime.source_record_id.trim().is_empty()
        || runtime.country_tag != country_tag
        || runtime.balance_id != row.template
        || runtime.display_ordinal != 0
    {
        return Err(PowerBalanceRejection::MissingRuntimeProvenance);
    }
    if definition.side(row.left_side).is_none() || definition.side(row.right_side).is_none() {
        return Err(PowerBalanceRejection::InvalidDynamicSides);
    }
    let expected_active = if row.value < 0.0 {
        row.left_side
    } else {
        row.right_side
    };
    if runtime.active_side_id != expected_active {
        return Err(PowerBalanceRejection::InvalidActiveSide);
    }
    if runtime.trending_side_id != row.trending_side {
        return Err(PowerBalanceRejection::InvalidTrend);
    }
    let active_range = definition
        .range(&runtime.active_range_id)
        .filter(|range| {
            range
                .side_id
                .as_deref()
                .is_none_or(|side| side == row.left_side || side == row.right_side)
        })
        .ok_or(PowerBalanceRejection::InvalidActiveRange)?;
    if runtime.active_range_id != row.active_range {
        return Err(PowerBalanceRejection::InvalidActiveRange);
    }
    if runtime.effective_left_icon.trim().is_empty()
        || runtime.effective_right_icon.trim().is_empty()
    {
        return Err(PowerBalanceRejection::InvalidRuntimeIcon);
    }
    let launcher_values_valid = runtime.launcher_visible
        && [
            runtime.launcher_percentage_text.as_deref(),
            runtime.launcher_levels_text.as_deref(),
            runtime.launcher_icon_sprite.as_deref(),
        ]
        .into_iter()
        .all(|value| value.is_some_and(|value| !value.trim().is_empty()));
    if !launcher_values_valid {
        return Err(PowerBalanceRejection::InvalidLauncher);
    }
    if runtime.range_indicator_frame > 1 {
        return Err(PowerBalanceRejection::InvalidIndicatorFrame);
    }
    match (&runtime.trend, runtime.bar_state.is_moving()) {
        (None, false) => {}
        (Some(trend), true)
            if trend.magnitude.is_finite()
                && trend.magnitude > 0.0
                && !trend.resolved_text.trim().is_empty() => {}
        _ => return Err(PowerBalanceRejection::InvalidTrend),
    }
    let bar_matches_engine_state = match runtime.bar_state {
        BarState::StableLeft => row.value < 0.0,
        BarState::StableRight => row.value >= 0.0,
        // Moving direction is the independently observed change sign. The
        // trending-side definition is retained separately and must not be
        // reinterpreted as that sign by the replay layer.
        BarState::MovingLeft | BarState::MovingRight => true,
    };
    if !bar_matches_engine_state {
        return Err(PowerBalanceRejection::InvalidBarState);
    }
    if !runtime.decisions_complete {
        return Err(PowerBalanceRejection::IncompleteDecisions);
    }
    let category = definition.decision_category.as_deref();
    let mut decision_ids = BTreeSet::new();
    let mut decision_source_ids = BTreeSet::new();
    let mut decision_presentations = Vec::with_capacity(runtime.decisions.len());
    for decision in &runtime.decisions {
        if decision.decision_id.trim().is_empty()
            || decision.source_record_id.trim().is_empty()
            || decision.resolved_name.trim().is_empty()
            || decision.availability == DecisionAvailability::Unknown
            || !decision_ids.insert(decision.decision_id.as_str())
            || !decision_source_ids.insert(decision.source_record_id.as_str())
            || decision.target_tag.is_some() != decision.target_flag_sprite.is_some()
            || decision
                .target_flag_sprite
                .as_deref()
                .is_some_and(|sprite| sprite.trim().is_empty())
        {
            return Err(PowerBalanceRejection::InvalidDecision);
        }
        let Some(definition) = decisions
            .decisions
            .iter()
            .find(|row| row.id == decision.decision_id && Some(row.category.as_str()) == category)
        else {
            return Err(PowerBalanceRejection::InvalidDecision);
        };
        let state = DecisionStateEntry {
            decision_id: decision.decision_id.clone(),
            availability: decision.availability,
            cost_text: decision.cost_text.clone(),
            timer_text: decision.timer_text.clone(),
            target_tag: decision.target_tag.clone(),
        };
        decision_presentations.push(DecisionPresentation {
            source_record_id: decision.source_record_id.clone(),
            resolved_name: decision.resolved_name.clone(),
            target_flag_sprite: decision.target_flag_sprite.clone(),
            card: decision_card_binding(definition, Some(&state)),
        });
    }
    let text = &runtime.resolved_text;
    if [
        &text.balance_name,
        &text.left_side_name,
        &text.right_side_name,
        &text.active_range_name,
        &text.balance_tooltip,
        &text.active_range_tooltip,
    ]
    .into_iter()
    .any(|value| value.trim().is_empty())
    {
        return Err(PowerBalanceRejection::InvalidResolvedText);
    }
    if !runtime_text_bindings_are_exact(runtime) {
        return Err(PowerBalanceRejection::InvalidRuntimeTextBinding);
    }
    let normalized = normalize_value(row.value);
    let range_boundaries = current_boundaries(definition, row.left_side, row.right_side);
    Ok(PowerBalancePresentation {
        provenance: runtime.provenance.clone(),
        source_record_id: runtime.source_record_id.clone(),
        country_tag: country_tag.to_string(),
        balance_id: row.template.to_string(),
        display_ordinal: runtime.display_ordinal,
        value: row.value,
        normalized_value: normalized,
        active_side_id: runtime.active_side_id.clone(),
        trending_side_id: runtime.trending_side_id.clone(),
        active_range_id: runtime.active_range_id.clone(),
        active_range_modifier: active_range.modifier.clone(),
        left_icon: runtime.effective_left_icon.clone(),
        right_icon: runtime.effective_right_icon.clone(),
        launcher_visible: runtime.launcher_visible,
        launcher_percentage_text: runtime.launcher_percentage_text.clone(),
        launcher_levels_text: runtime.launcher_levels_text.clone(),
        launcher_icon_sprite: runtime.launcher_icon_sprite.clone(),
        launcher_glow_visible: runtime.launcher_glow_visible,
        bar_sprite: runtime.bar_state.sprite().to_string(),
        range_indicator_frame: runtime.range_indicator_frame,
        range_indicator_top_left: runtime.range_indicator_top_left,
        range_boundaries,
        trend: runtime.trend.clone(),
        power_balance_modifier_desc: runtime.power_balance_modifier_desc.clone(),
        decisions: decision_presentations,
        resolved_text: runtime.resolved_text.clone(),
    })
}

fn runtime_text_bindings_are_exact(runtime: &PowerBalanceRuntimeSupplement) -> bool {
    let mut expected = BTreeMap::<PowerBalanceRuntimeTextSlot, &str>::new();
    let mut insert = |slot, text| expected.insert(slot, text).is_none();
    if let Some(text) = runtime.launcher_percentage_text.as_deref() {
        if !insert(PowerBalanceRuntimeTextSlot::LauncherPercentage, text) {
            return false;
        }
    }
    if let Some(text) = runtime.launcher_levels_text.as_deref() {
        if !insert(PowerBalanceRuntimeTextSlot::LauncherLevels, text) {
            return false;
        }
    }
    if let Some(trend) = runtime.trend.as_ref() {
        if !insert(PowerBalanceRuntimeTextSlot::Trend, &trend.resolved_text) {
            return false;
        }
    }
    if !insert(
        PowerBalanceRuntimeTextSlot::ModifierDescription,
        &runtime.power_balance_modifier_desc,
    ) {
        return false;
    }
    for (slot, text) in [
        (
            PowerBalanceRuntimeTextSlot::BalanceName,
            runtime.resolved_text.balance_name.as_str(),
        ),
        (
            PowerBalanceRuntimeTextSlot::LeftSideName,
            runtime.resolved_text.left_side_name.as_str(),
        ),
        (
            PowerBalanceRuntimeTextSlot::RightSideName,
            runtime.resolved_text.right_side_name.as_str(),
        ),
        (
            PowerBalanceRuntimeTextSlot::ActiveRangeName,
            runtime.resolved_text.active_range_name.as_str(),
        ),
        (
            PowerBalanceRuntimeTextSlot::BalanceTooltip,
            runtime.resolved_text.balance_tooltip.as_str(),
        ),
        (
            PowerBalanceRuntimeTextSlot::ActiveRangeTooltip,
            runtime.resolved_text.active_range_tooltip.as_str(),
        ),
    ] {
        if !insert(slot, text) {
            return false;
        }
    }
    for decision in &runtime.decisions {
        let slot = |role| PowerBalanceRuntimeTextSlot::Decision {
            decision_id: decision.decision_id.clone(),
            source_record_id: decision.source_record_id.clone(),
            role,
        };
        if !insert(
            slot(PowerBalanceDecisionTextRole::Name),
            &decision.resolved_name,
        ) {
            return false;
        }
        if let Some(text) = decision.cost_text.as_deref() {
            if !insert(slot(PowerBalanceDecisionTextRole::Cost), text) {
                return false;
            }
        }
        if let Some(text) = decision.timer_text.as_deref() {
            if !insert(slot(PowerBalanceDecisionTextRole::Timer), text) {
                return false;
            }
        }
    }

    let contexts = runtime
        .provenance
        .runtime_text_contexts
        .iter()
        .map(|context| (context.context_id.as_str(), context))
        .collect::<BTreeMap<_, _>>();
    let mut bindings = BTreeMap::new();
    let mut referenced = BTreeSet::new();
    for binding in &runtime.provenance.runtime_text_bindings {
        let Some(context) = contexts.get(binding.context_id.as_str()) else {
            return false;
        };
        let Some(text) = expected.get(&binding.slot) else {
            return false;
        };
        if bindings
            .insert(binding.slot.clone(), binding.context_id.as_str())
            .is_some()
            || Sha256::hex(text.as_bytes()) != context.final_text_sha256
        {
            return false;
        }
        referenced.insert(binding.context_id.as_str());
    }
    bindings.len() == expected.len()
        && bindings.keys().eq(expected.keys())
        && referenced.len() == contexts.len()
}

fn current_boundaries(
    definition: &PowerBalanceDefinition,
    left_side: &str,
    right_side: &str,
) -> Vec<RangeBoundaryPresentation> {
    let mut values = definition
        .neutral_ranges
        .iter()
        .chain(
            definition
                .sides
                .iter()
                .filter(|side| side.id == left_side || side.id == right_side)
                .flat_map(|side| side.ranges.iter()),
        )
        .flat_map(|range| [range.min.value, range.max.value])
        .collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    values.dedup_by(|left, right| left.to_bits() == right.to_bits() || *left == *right);
    values
        .into_iter()
        .filter(|value| *value > -1.0 && *value < 1.0)
        .map(|value| RangeBoundaryPresentation {
            value,
            normalized: normalize_value(value),
        })
        .collect()
}

fn normalize_value(value: f64) -> f32 {
    ((value + 1.0) * 0.5) as f32
}

/// Bind one admitted panel to the authored element names. Range splitters and
/// decision cards are dynamic collections and remain typed rows for XUI03 to
/// materialize in their preserved order.
pub fn apply_power_balance_to_compose_options(
    panel: &PowerBalancePresentation,
    options: &mut ComposeOptions,
) {
    options.text_overrides.insert(
        "power_balance_name".to_string(),
        panel.resolved_text.balance_name.clone(),
    );
    options.text_overrides.insert(
        "active_range_name".to_string(),
        panel.resolved_text.active_range_name.clone(),
    );
    options
        .sprite_name_overrides
        .insert("power_balance_value".to_string(), panel.bar_sprite.clone());
    options
        .sprite_name_overrides
        .insert("left_power_icon".to_string(), panel.left_icon.clone());
    options
        .sprite_name_overrides
        .insert("right_power_icon".to_string(), panel.right_icon.clone());
    options
        .progress_overrides
        .insert("power_balance_value".to_string(), panel.normalized_value);
    options
        .frame_overrides
        .insert("range_indicator".to_string(), panel.range_indicator_frame);
    options.position_overrides.insert(
        "range_indicator".to_string(),
        panel.range_indicator_top_left,
    );
}

/// Bind the adjacent country-politics launcher owned by XUI11. These values
/// remain engine-resolved A37 inputs; the replay never derives the displayed
/// percentage/level strings, active icon, or glow state from the panel value.
pub fn apply_power_balance_launcher_to_compose_options(
    panel: &PowerBalancePresentation,
    options: &mut ComposeOptions,
) {
    if !panel.launcher_visible {
        options
            .hidden_elements
            .insert("power_balance_button".to_string());
        return;
    }
    options.hidden_elements.remove("power_balance_button");
    options.text_overrides.insert(
        "power_balance_percentage".to_string(),
        panel.launcher_percentage_text.clone().unwrap_or_default(),
    );
    options.text_overrides.insert(
        "power_balance_levels".to_string(),
        panel.launcher_levels_text.clone().unwrap_or_default(),
    );
    options.sprite_name_overrides.insert(
        "power_balance_ICON".to_string(),
        panel.launcher_icon_sprite.clone().unwrap_or_default(),
    );
    if panel.launcher_glow_visible {
        options.hidden_elements.remove("show_power_balance_glow");
    } else {
        options
            .hidden_elements
            .insert("show_power_balance_glow".to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::{PowerBalanceDefinitionCatalog, StaticCatalogError};

    /// A minimal balance body that passes `convert_definition` unchanged, so a
    /// duplicated key exercises the `DuplicateBalance` refusal rather than an
    /// earlier structural error.
    fn balance_body(id: &str) -> String {
        format!(
            r#"{{
                "id": "{id}",
                "initial_value": "0.0",
                "left_side": "left",
                "right_side": "right",
                "decision_category": null,
                "sides": [
                    {{
                        "id": "left",
                        "order": 0,
                        "icon": "GFX_left",
                        "ranges": [],
                        "range_count": 0,
                        "source_file": "common/bop/test.txt"
                    }},
                    {{
                        "id": "right",
                        "order": 1,
                        "icon": "GFX_right",
                        "ranges": [],
                        "range_count": 0,
                        "source_file": "common/bop/test.txt"
                    }}
                ],
                "range_count": 0,
                "source_file": "common/bop/test.txt"
            }}"#
        )
    }

    #[test]
    fn duplicate_power_balance_key_is_refused() {
        let body = balance_body("bop_test");
        let input = format!(
            r#"{{"catalogs":{{"power_balances":{{"bop_test":{body},"bop_test":{body}}}}}}}"#
        );
        assert_eq!(
            PowerBalanceDefinitionCatalog::from_catalogs_json(&input),
            Err(StaticCatalogError::DuplicateBalance("bop_test".to_string()))
        );
    }

    #[test]
    fn distinct_power_balance_keys_are_admitted() {
        let first = balance_body("bop_first");
        let second = balance_body("bop_second");
        let input = format!(
            r#"{{"catalogs":{{"power_balances":{{"bop_first":{first},"bop_second":{second}}}}}}}"#
        );
        let catalog = PowerBalanceDefinitionCatalog::from_catalogs_json(&input)
            .expect("distinct keys must parse");
        assert_eq!(catalog.len(), 2);
        assert!(catalog.get("bop_first").is_some());
        assert!(catalog.get("bop_second").is_some());
    }
}
