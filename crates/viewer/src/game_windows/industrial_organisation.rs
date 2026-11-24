use super::{
    ComposedPlan, ComposedSubWindow, DEFAULT_VIEWPORT, GameWindowError, GameWindowRender,
    GameWindowRequest, GameWindowTooltip, Prepared, RenderParams, Service, WindowCacheKey,
    container_holder,
    equipment_designer::{
        RenderContext, compute_read_hour, script_tables, witnessed_rows, witnessed_single_key,
    },
    layout_window, missing_window_refusal, strip_markup,
};
use super::compose::{
    collect_template_assets, disable_by_name, find_direct, hide_by_name, row_size, set_frame_by_name,
    set_sprite_by_name, set_text_by_name, spell,
};
use content_reader_defines::defines;
use content_reader_key_names::key_name_products::KeyNamesExt as _;
use content_reader_parser::parser::c_string;
use content_reader_gui::gui::native::{self, Element, Fields};
use content_reader_layers::ResolvedLayers;
use content_reader_localisation::localisation::Localisation;
use content_reader_military::{
    industrial_organisations::{self, Database as OrganisationDatabase, policies::Policy},
    mio_traits::{self, Bonus, TraitName, Tree, is_production_stat},
};
use dashboard_text::metadata::link_slot;
use content_reader_layers::content::ResolvedContent;
use dashboard_effects::effect_description::{Content, Context, MioText, mio_text};
use content_reader_parser::parser::Item;
use dashboard_text::organisation_quote;
use dashboard_text::scripted_text::{EventScopes, ScriptedContent, ScriptedText};
use dashboard_text::triggers::{Evaluator, OrganisationCondition, Undecided};
use replay::campaign::SeekResult;
use replay_frame::frame::DecodedFrame;
use std::cell::RefCell;
use replay_data::rows::{
    IndustrialOrganisationDetailsRow, IndustrialOrganisationHistoryRow,
    IndustrialOrganisationRow, LexicalTokenRow, Recorded,
    TableId, VariantDefinitionRow, tables,
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, atomic::AtomicBool},
};
use viewer_native_gui::native_gui::{
    self, Assets, Layout, Resources, lifecycle::Instance,
};

#[cfg(test)]
mod tests;

const FIXED5_SCALE: f64 = 100_000.0;
const UNSELECTED_POLICY: u32 = 0x4c17;
const NOT_RECORDED_VALUE: &str = "n/a";
const UNKNOWN_ICON_SPRITE: &str = "GFX_idea_unknown";
const DETAIL_WINDOW_NAME: &str = "industrial_organisation_detail_window";
const POLICY_WINDOW_NAME: &str = "industrial_organisation_policy_window";
const TREE_STEP_X: i32 = 88;
const TREE_STEP_Y: i32 = 112;
const TRAIT_ICON_CENTER_X: i32 = 43;
const TRAIT_ICON_TOP_Y: i32 = 18;
const TRAIT_ICON_BOTTOM_Y: i32 = 98;
const TRAIT_ICON_LEFT_X: i32 = 14;
const TRAIT_ICON_RIGHT_X: i32 = 72;
const TRAIT_ICON_MID_Y: i32 = 58;

const REQUIRED_TABLES: &[TableId] = &[
    TableId::IndustrialOrganisations,
    TableId::IndustrialOrganisationDetails,
    TableId::IndustrialOrganisationHistory,
    TableId::IndustrialOrganisationPolicies,
    TableId::VariantDefinitions,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::ModifierDefinitions,
    TableId::SimulationFeatures,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubView {
    Traits,
    History,
    Policy,
}

impl SubView {
    fn from_request(sub_view: Option<&str>) -> Result<Self, GameWindowError> {
        match sub_view.unwrap_or("traits") {
            "traits" => Ok(Self::Traits),
            "history" => Ok(Self::History),
            "policy" => Ok(Self::Policy),
            other => Err(GameWindowError::refused(format!(
                "Unsupported military industrial organisation sub-view '{other}'."
            ))),
        }
    }

    fn window_name(self) -> &'static str {
        match self {
            Self::Traits | Self::History => DETAIL_WINDOW_NAME,
            Self::Policy => POLICY_WINDOW_NAME,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Target {
    organisation: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FundsProgress {
    pub funds: f64,
    pub threshold: Option<f64>,
    pub percent: Option<i32>,
    pub threshold_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicySummary {
    pub token: String,
    pub cost: Option<i32>,
    pub cooldown_days: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Spelled {
    Text(String),
    Unspelled { token: u32, reason: String },
}

impl Spelled {
    fn text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Unspelled { .. } => None,
        }
    }

    fn missing(&self, what: &str) -> Option<String> {
        match self {
            Self::Text(_) => None,
            Self::Unspelled { token, reason } => Some(format!(
                "Not recorded: the {what} with token {token} cannot be named ({reason})."
            )),
        }
    }
}

fn spell_token(lexical: &Result<&[LexicalTokenRow], String>, token: u32) -> Spelled {
    match lexical {
        Ok(rows) => match spell(rows, token) {
            Some(text) => Spelled::Text(text),
            None => Spelled::Unspelled {
                token,
                reason: "LexicalTokens does not spell it at this hour".to_owned(),
            },
        },
        Err(reason) => Spelled::Unspelled {
            token,
            reason: format!("LexicalTokens is not recorded at this hour: {reason}"),
        },
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesignDate {
    Designed(i32),
    Absent,
    NotRecorded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    pub variant_id: u64,
    pub name: Result<String, String>,
    pub archetype: String,
    pub units: i32,
    pub design: DesignDate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryList<'h> {
    Entries(Vec<&'h HistoryEntry>),
    Missing(String),
}

pub fn history_lists(entries: &[HistoryEntry]) -> [HistoryList<'_>; 2] {
    if entries.iter().any(|entry| entry.design == DesignDate::NotRecorded) {
        let reason = "Not recorded: this recording does not hold the organisation's design dates, so its research and designer history cannot be told apart from its production line history.".to_owned();
        return [HistoryList::Missing(reason.clone()), HistoryList::Missing(reason)];
    }
    let designed = entries
        .iter()
        .filter(|entry| matches!(entry.design, DesignDate::Designed(_)))
        .collect();
    let produced = entries
        .iter()
        .filter(|entry| entry.design == DesignDate::Absent)
        .collect();
    [HistoryList::Entries(designed), HistoryList::Entries(produced)]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraitStatus {
    Unlocked,
    Queued(usize),
    Available,
    Locked,
    Unknown(String),
}

pub fn trait_status(details: &RecordedDetails, token: &str) -> TraitStatus {
    let details = match details {
        RecordedDetails::Available(details) => details,
        RecordedDetails::Unavailable(reason) => {
            return TraitStatus::Unknown(format!(
                "Not recorded: organisation details are unavailable at this hour ({reason})."
            ));
        }
    };
    let named = |list: &[Spelled]| {
        list.iter()
            .position(|entry| entry.text().is_some_and(|text| text.eq_ignore_ascii_case(token)))
    };
    let unspelled = |list: &[Spelled], what: &str| {
        list.iter().find_map(|entry| entry.missing(what))
    };
    if named(&details.unlocked).is_some() {
        return TraitStatus::Unlocked;
    }
    if let Some(reason) = unspelled(&details.unlocked, "unlocked trait") {
        return TraitStatus::Unknown(reason);
    }
    if let Some(index) = named(&details.queued) {
        return TraitStatus::Queued(index + 1);
    }
    if let Some(reason) = unspelled(&details.queued, "queued trait") {
        return TraitStatus::Unknown(reason);
    }
    TraitStatus::Locked
}

#[derive(Clone, Debug)]
pub enum RecordedDetails {
    Available(Box<OrganisationDetails>),
    Unavailable(String),
}

#[derive(Clone, Debug)]
pub struct OrganisationDetails {
    pub funds_raw: i64,
    pub size: i32,
    pub points: i32,
    pub research_bonus_raw: i64,
    pub task_capacity: i32,
    pub research_assign_cost_raw: i64,
    pub production_assign_cost_raw: i64,
    pub design_team_change_cost: i32,
    pub funds_gain_factor_raw: i64,
    pub size_cost_factor_raw: i64,
    pub auto_upgrade: bool,
    pub policy: Option<Spelled>,
    pub cooldown_hour: Option<i32>,
    pub unlocked: Vec<Spelled>,
    pub queued: Vec<Spelled>,
    pub allowed_policies: Vec<Spelled>,
}

#[derive(Clone, Debug)]
pub struct OrganisationViewData {
    pub organisation_id: u64,
    pub request_hour: i32,
    pub read_hour: i32,
    pub owner_tag: Option<String>,
    pub token: Result<String, String>,
    pub custom_name: String,
    pub recorded_icon: String,
    pub details: RecordedDetails,
    pub policy_costs: Result<BTreeMap<String, PolicySummary>, String>,
    pub history: Option<Vec<HistoryEntry>>,
    pub history_reason: Option<String>,
    pub funds_defines: Result<FundsDefines, String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FundsDefines {
    pub base: f64,
    pub level_factor: f64,
    pub level_pow: f64,
}

impl FundsDefines {
    pub fn threshold_for(self, size: i32, size_cost_factor_raw: i64) -> Option<f64> {
        let level = f64::from(size.max(0));
        let raw = self.base + self.level_factor * level.powf(self.level_pow);
        let factor = (1.0 + size_cost_factor_raw as f64 / FIXED5_SCALE).max(0.0);
        let scaled = raw * factor;
        (scaled.is_finite() && scaled > 0.0).then_some(scaled)
    }
}

pub fn compute_funds_progress(
    funds_raw: i64,
    size: i32,
    size_cost_factor_raw: i64,
    defines: &Result<FundsDefines, String>,
) -> FundsProgress {
    let funds = funds_raw as f64 / FIXED5_SCALE;
    match defines {
        Ok(def) => match def.threshold_for(size, size_cost_factor_raw) {
            Some(threshold) => {
                let ratio = (funds / threshold).clamp(0.0, 1.0);
                FundsProgress {
                    funds,
                    threshold: Some(threshold),
                    percent: Some((ratio * 100.0).round() as i32),
                    threshold_reason: None,
                }
            }
            None => FundsProgress {
                funds,
                threshold: None,
                percent: None,
                threshold_reason: Some(
                    "Not recorded: size-up threshold is non-positive or non-finite.".to_owned(),
                ),
            },
        },
        Err(reason) => FundsProgress {
            funds,
            threshold: None,
            percent: None,
            threshold_reason: Some(format!(
                "Not recorded: funds threshold defines are unavailable ({reason})."
            )),
        },
    }
}

fn organisation_scripts_readable() -> Result<(), String> {
    script_tables().map_err(|withheld| {
        format!("organisation scripts are read through the recording's executable tables, and {withheld}")
    })
}

fn trait_catalog(layers: &ResolvedLayers) -> Result<Arc<mio_traits::Catalog>, String> {
    organisation_scripts_readable()?;
    mio_traits::cached(layers)
}

fn cached_organisation_database(
    layers: &ResolvedLayers,
) -> Result<Arc<OrganisationDatabase>, String> {
    organisation_scripts_readable()?;
    #[derive(Default)]
    struct Cached(
        content_reader_layers::content::diagnostics::Once<
            Result<Arc<OrganisationDatabase>, String>,
        >,
    );
    layers
        .derived::<Cached>()
        .0
        .get_or_init(|| {
            let names = layers.key_names().map_err(|error| error.to_string())?;
            industrial_organisations::read(layers, &names)
                .map(Arc::new)
                .map_err(|error| error.to_string())
        })
        .clone()
}

fn cached_scripted_content(
    layers: &ResolvedLayers,
    content: &Arc<ResolvedContent>,
    language: &str,
) -> Result<Arc<ScriptedContent>, String> {
    #[derive(Default)]
    struct Cached(std::sync::Mutex<BTreeMap<String, Result<Arc<ScriptedContent>, String>>>);
    let cached = layers.derived::<Cached>();
    let mut by_language = cached.0.lock().unwrap_or_else(|error| error.into_inner());
    by_language
        .entry(language.to_owned())
        .or_insert_with(|| {
            dashboard_base::catalogue::read_scripted_content(Arc::clone(content), language.to_owned())
                .map(Arc::new)
        })
        .clone()
}

fn read_funds_defines(layers: &ResolvedLayers) -> Result<FundsDefines, String> {
    let loaded = defines::read(layers).map_err(|error| error.to_string())?;
    let base = loaded
        .number("NIndustrialOrganisation", "FUNDS_FOR_SIZE_UP")
        .map_err(|error| error.to_string())?;
    let level_factor = loaded
        .number("NIndustrialOrganisation", "FUNDS_FOR_SIZE_UP_LEVEL_FACTOR")
        .map_err(|error| error.to_string())?;
    let level_pow = loaded
        .number("NIndustrialOrganisation", "FUNDS_FOR_SIZE_UP_LEVEL_POW")
        .map_err(|error| error.to_string())?;
    Ok(FundsDefines {
        base,
        level_factor,
        level_pow,
    })
}

fn resolve_owner_tag(
    row: &IndustrialOrganisationRow,
    countries: &[replay_data::rows::CountryRow],
    links: &[replay_data::rows::CountryLinkRow],
) -> Option<String> {
    let lookup = |raw: u32| -> Option<String> {
        let slot = link_slot(links, raw)?;
        let country = countries.iter().find(|entry| entry.canonical_slot == slot)?;
        std::str::from_utf8(&country.tag)
            .ok()
            .filter(|tag| !tag.is_empty())
            .map(str::to_owned)
    };
    lookup(row.owner).or_else(|| lookup(row.country))
}

fn format_days_remaining(cooldown_hour: Option<i32>, request_hour: i32) -> Option<i32> {
    let target = cooldown_hour?;
    let days = target.div_euclid(24) - request_hour.div_euclid(24);
    (days > 0).then_some(days)
}

pub fn extract_details(
    row: &IndustrialOrganisationDetailsRow,
    lexical: &Result<&[LexicalTokenRow], String>,
) -> OrganisationDetails {
    let spell_list = |tokens: &[u32]| -> Vec<Spelled> {
        tokens.iter().map(|&token| spell_token(lexical, token)).collect()
    };
    let policy = (row.state.policy != UNSELECTED_POLICY)
        .then(|| spell_token(lexical, row.state.policy));
    OrganisationDetails {
        funds_raw: row.state.funds_raw,
        size: row.state.size,
        points: row.state.points,
        research_bonus_raw: row.state.research_bonus_raw,
        task_capacity: row.state.task_capacity,
        research_assign_cost_raw: row.state.research_assign_cost_raw,
        production_assign_cost_raw: row.state.production_assign_cost_raw,
        design_team_change_cost: row.state.design_team_change_cost,
        funds_gain_factor_raw: row.state.funds_gain_factor_raw,
        size_cost_factor_raw: row.state.size_cost_factor_raw,
        auto_upgrade: row.state.upgrades != 0,
        policy,
        cooldown_hour: row.state.cooldown_hour,
        unlocked: spell_list(&row.unlocked),
        queued: spell_list(&row.queued),
        allowed_policies: spell_list(&row.allowed_policies),
    }
}

pub fn owner_policy_roster(
    frame: &DecodedFrame,
    organisation: &IndustrialOrganisationRow,
    lexical: Result<&[LexicalTokenRow], &String>,
) -> Result<BTreeMap<String, PolicySummary>, String> {
    let roster = organisation_quote::owner_roster(frame, organisation)?;
    let mut out = BTreeMap::new();
    for policy in &roster.policies {
        let Some(token) = lexical.ok().and_then(|rows| spell(rows, policy.token)) else {
            continue;
        };
        out.insert(
            token.to_ascii_lowercase(),
            PolicySummary {
                token,
                cost: Some(policy.cost),
                cooldown_days: Some(policy.days),
            },
        );
    }
    Ok(out)
}

pub fn extract_history(
    rows: &[IndustrialOrganisationHistoryRow],
    org_id: u64,
    variants: &Result<&[VariantDefinitionRow], String>,
    lexical: &Result<&[LexicalTokenRow], String>,
) -> Vec<HistoryEntry> {
    let first = rows.partition_point(|row| row.organisation < org_id);
    rows[first..]
        .iter()
        .take_while(|row| row.organisation == org_id)
        .map(|row| {
            let definition = variants.as_ref().map_err(Clone::clone).and_then(|defs| {
                defs.binary_search_by_key(&row.variant, |v| v.id)
                    .map(|idx| &defs[idx])
                    .map_err(|_| "VariantDefinitions has no row for this design at this hour".to_owned())
            });
            let (name, archetype) = match definition {
                Ok(def) => {
                    let equipment = spell_token(lexical, def.equipment_token);
                    let recorded = String::from_utf8_lossy(c_string(&def.display_name)).trim().to_owned();
                    let name = if !recorded.is_empty() {
                        Ok(recorded)
                    } else {
                        equipment.text().map(str::to_owned).ok_or_else(|| {
                            equipment.missing("design's equipment").unwrap_or_default()
                        })
                    };
                    (name, equipment.text().unwrap_or_default().to_owned())
                }
                Err(reason) => (
                    Err(format!("Not recorded: the design's name is unavailable ({reason}).")),
                    String::new(),
                ),
            };
            let design = match row.design_hour {
                Recorded::Value(Some(hour)) => DesignDate::Designed(hour),
                Recorded::Value(None) => DesignDate::Absent,
                Recorded::NotRecorded => DesignDate::NotRecorded,
            };
            HistoryEntry {
                variant_id: row.variant,
                name,
                archetype,
                units: row.units,
                design,
            }
        })
        .collect()
}

pub struct Loc<'a> {
    localisation: &'a Localisation,
    frame: &'a DecodedFrame,
    organisation: u64,
    scripted: Result<ScriptedText<'a>, String>,
    scopes: EventScopes,
    name: Result<String, String>,
    fallback_name: String,
    failures: &'a RefCell<Vec<String>>,
}

impl<'a> Loc<'a> {
    pub fn new(
        localisation: &'a Localisation,
        frame: &'a DecodedFrame,
        content: &'a Result<Arc<ScriptedContent>, String>,
        data: &OrganisationViewData,
        failures: &'a RefCell<Vec<String>>,
    ) -> Self {
        let scripted = content
            .as_deref()
            .map(|content| ScriptedText::new(frame, localisation, content, 0))
            .map_err(|why| format!("scripted text is unavailable ({why})"));
        let template = scripted
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|text| text.organisation_template(data.organisation_id));
        let (name, scopes) = match (&scripted, template) {
            (Ok(text), Ok((template, scopes))) => (
                text.expand_in(&template, &scopes).map(|name| strip_markup(&name)),
                scopes,
            ),
            (_, Err(why)) => (Err(why), EventScopes::organisation(data.organisation_id, None)),
            (Err(why), Ok((_, scopes))) => (Err(why.clone()), scopes),
        };
        let fallback_name = if data.custom_name.is_empty() {
            data.token.clone().unwrap_or_default()
        } else {
            data.custom_name.clone()
        };
        Self {
            localisation,
            frame,
            organisation: data.organisation_id,
            scripted,
            scopes,
            name,
            fallback_name,
            failures,
        }
    }

    fn fail(&self, shown: &str, why: &str) {
        let line = format!("Text '{shown}' is shown unresolved: {why}.");
        let mut failures = self.failures.borrow_mut();
        if !failures.contains(&line) {
            failures.push(line);
        }
    }

    fn expand(&self, raw: &str) -> Result<String, String> {
        let scripted = self.scripted.as_ref().map_err(Clone::clone)?;
        scripted.expand_in(raw, &self.scopes).map(|text| strip_markup(&text))
    }

    fn localise(&self, key: &str) -> Result<String, String> {
        let raw = self
            .localisation
            .get(key)
            .ok_or_else(|| "the recorded mod stack has no localisation for it".to_owned())?;
        self.expand(raw)
    }

    pub fn has(&self, key: &str) -> bool {
        self.localisation.get(key).is_some()
    }

    pub fn get(&self, key: &str) -> String {
        match self.localise(key) {
            Ok(text) if !text.is_empty() => text,
            Ok(_) => {
                self.fail(key, "its localisation is empty");
                key.to_owned()
            }
            Err(why) => {
                self.fail(key, &why);
                key.to_owned()
            }
        }
    }

    pub fn fill(&self, key: &str, values: &[(&str, &str)]) -> String {
        let Some(raw) = self.localisation.get(key) else {
            self.fail(key, "the recorded mod stack has no localisation for it");
            let shown: Vec<&str> = values.iter().map(|(_, value)| *value).collect();
            return format!("{key} {}", shown.join(" "));
        };
        let mut filled = String::with_capacity(raw.len());
        let mut rest = raw;
        while let Some(start) = rest.find('$') {
            let after = &rest[start + 1..];
            let Some(end) = after.find('$') else { break };
            let placeholder = &after[..end];
            let name = placeholder.split('|').next().unwrap_or(placeholder);
            filled.push_str(&rest[..start]);
            match values.iter().find(|(wanted, _)| *wanted == name) {
                Some((_, value)) => filled.push_str(value),
                None => {
                    filled.push('$');
                    filled.push_str(placeholder);
                    filled.push('$');
                }
            }
            rest = &after[end + 1..];
        }
        filled.push_str(rest);
        match self.expand(&filled) {
            Ok(text) => text,
            Err(why) => {
                self.fail(key, &why);
                strip_markup(&filled)
            }
        }
    }

    pub fn organisation_name(&self) -> String {
        match &self.name {
            Ok(name) if !name.is_empty() => name.clone(),
            Ok(_) => {
                self.fail(&self.fallback_name, "the organisation's name resolves to empty text");
                self.fallback_name.clone()
            }
            Err(why) => {
                self.fail(&self.fallback_name, why);
                self.fallback_name.clone()
            }
        }
    }

    pub fn mio_lines(&self, text: MioText<'_>) -> Vec<Result<String, String>> {
        let scripted = match &self.scripted {
            Ok(scripted) => scripted,
            Err(why) => return vec![Err(why.clone())],
        };
        let context = Context::new(
            scripted,
            self.localisation,
            0,
            "industrial organisation window",
            Content::default(),
        );
        mio_text(text, &context, &self.scopes)
            .into_iter()
            .map(|line| line.map(|text| strip_markup(&text)))
            .collect()
    }

    fn evaluate<R>(
        &self,
        work: impl FnOnce(&Evaluator<'_, '_>) -> Result<R, Undecided>,
    ) -> Result<R, String> {
        let scripted = self.scripted.as_ref().map_err(Clone::clone)?;
        let world = scripted.trigger_world()?;
        work(&Evaluator::new(world, None)).map_err(String::from)
    }

    pub fn trait_condition(
        &self,
        organisation: u64,
        token: &str,
        condition: OrganisationCondition,
    ) -> Result<bool, String> {
        self.evaluate(|evaluator| evaluator.trait_condition(organisation, token, condition))
    }

    pub fn organisation_conditions(
        &self,
        organisation: u64,
        order: &[(OrganisationCondition, &[Item])],
    ) -> Result<bool, String> {
        self.evaluate(|evaluator| evaluator.organisation_conditions(organisation, order))
    }

    pub fn policy_cost(
        &self,
        catalog: &Result<Arc<mio_traits::Catalog>, String>,
        policies: &industrial_organisations::policies::Catalog,
    ) -> Result<organisation_quote::PolicyCost, String> {
        let catalog = catalog.as_deref().map_err(Clone::clone)?;
        let organisation = witnessed_single_key::<tables::IndustrialOrganisations, _>(
            self.frame,
            self.organisation,
            |row| row.organisation,
        )?
        .ok_or("the organisation is not recorded at this hour")?;
        let details = witnessed_single_key::<tables::IndustrialOrganisationDetails, _>(
            self.frame,
            self.organisation,
            |row| row.organisation,
        )?
        .ok_or("the organisation's details are not recorded at this hour")?;
        let world = match &self.scripted {
            Ok(scripted) => scripted.trigger_world(),
            Err(why) => Err(why.clone()),
        };
        organisation_quote::policy_cost(self.frame, organisation, details, catalog, policies, world)
            .map_err(String::from)
    }

    pub fn date(&self, hour: i32) -> String {
        match self
            .scripted
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|text| text.date_text(hour))
        {
            Ok(text) => text.trim().to_owned(),
            Err(why) => {
                self.fail("DATE", &why);
                NOT_RECORDED_VALUE.to_owned()
            }
        }
    }
}

fn resolve_org_icon(
    data: &OrganisationViewData,
    tree: Option<&Tree<'_>>,
    has_sprite: impl Fn(&str) -> bool,
) -> String {
    let mut candidates = Vec::new();
    if !data.recorded_icon.is_empty() {
        candidates.push(data.recorded_icon.clone());
    }
    if let Some(icon) = tree.and_then(|t| t.icon)
        && !icon.is_empty()
    {
        candidates.push(icon.to_owned());
    }
    if let Ok(token) = &data.token {
        if let Some(tag) = &data.owner_tag {
            candidates.push(format!("GFX_{tag}_{token}"));
        }
        candidates.push(format!("GFX_{token}"));
    }
    candidates.push(UNKNOWN_ICON_SPRITE.to_owned());
    for candidate in candidates {
        if has_sprite(&candidate) {
            return candidate;
        }
    }
    UNKNOWN_ICON_SPRITE.to_owned()
}

fn resolve_policy_name(token: &str, policy: Option<&Policy>, loc: &Loc<'_>) -> String {
    if let Some(def) = policy
        && !def.name.is_empty()
    {
        let key = String::from_utf8_lossy(&def.name);
        if loc.has(&key) {
            return loc.get(&key);
        }
    }
    loc.get(token)
}

fn resolve_policy_icon(policy: Option<&Policy>, has_sprite: impl Fn(&str) -> bool) -> String {
    if let Some(def) = policy
        && !def.icon.is_empty()
    {
        let sprite = String::from_utf8_lossy(&def.icon).into_owned();
        if has_sprite(&sprite) {
            return sprite;
        }
    }
    UNKNOWN_ICON_SPRITE.to_owned()
}

fn resolve_trait_name(
    trait_def: &TraitName,
    org_token: &str,
    owner_tag: Option<&str>,
    loc: &Loc<'_>,
) -> String {
    let candidates = trait_def.name_candidates(org_token, &trait_def.token, owner_tag);
    match candidates.iter().find(|candidate| loc.has(candidate)) {
        Some(key) => loc.get(key),
        None => loc.get(candidates.last().map_or(trait_def.token.as_str(), String::as_str)),
    }
}

fn format_signed_percent_fixed5(raw: i64) -> String {
    let pct = raw as f64 * 100.0 / FIXED5_SCALE;
    let rounded = (pct * 10.0).round() / 10.0;
    if (rounded.fract()).abs() < 0.05 {
        format!("{:+.0}%", rounded)
    } else {
        format!("{:+.1}%", rounded)
    }
}

fn modifier_item(loc: &Loc<'_>, name_key: &str, value: &str) -> String {
    loc.fill(
        "INDUSTRIAL_ORG_DETAIL_MIO_MODIFIER_ITEM",
        &[("NAME", &loc.get(name_key)), ("VALUE", value)],
    )
}

fn format_points_line(loc: &Loc<'_>, value_str: &str) -> String {
    loc.fill("INDUSTRIAL_ORG_DETAIL_TRAIT_POINTS", &[("VALUE", value_str)])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BonusTarget {
    Organisation,
    Equipment(Vec<Vec<u8>>),
    Group(Vec<u8>),
}

fn trait_target(trait_def: &TraitName) -> BonusTarget {
    if trait_def.equipment_types.is_empty() {
        return BonusTarget::Organisation;
    }
    let mut types: Vec<Vec<u8>> = trait_def
        .equipment_types
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect();
    types.sort();
    types.dedup();
    BonusTarget::Equipment(types)
}

pub fn aggregate_bonuses(
    details: &OrganisationDetails,
    tree: &Tree<'_>,
    policy: Option<&Policy>,
) -> Vec<(BonusTarget, Vec<Bonus>)> {
    let mut totals: Vec<(BonusTarget, Vec<Bonus>)> = vec![(BonusTarget::Organisation, Vec::new())];
    let mut add = |target: BonusTarget, bonus: &Bonus| {
        let index = match totals.iter().position(|(kept, _)| *kept == target) {
            Some(index) => index,
            None => {
                totals.push((target, Vec::new()));
                totals.len() - 1
            }
        };
        let bonuses = &mut totals[index].1;
        match bonuses.iter_mut().find(|kept| kept.name == bonus.name) {
            Some(kept) => kept.value = kept.value.wrapping_add(bonus.value),
            None => bonuses.push(bonus.clone()),
        }
    };
    if let Some(initial) = tree.initial {
        for bonus in &initial.bonuses {
            add(trait_target(initial), bonus);
        }
    }
    let unlocked: BTreeSet<String> = details
        .unlocked
        .iter()
        .filter_map(Spelled::text)
        .map(str::to_ascii_lowercase)
        .collect();
    for trait_def in tree.traits {
        if unlocked.contains(&trait_def.token.to_ascii_lowercase()) {
            for bonus in &trait_def.bonuses {
                add(trait_target(trait_def), bonus);
            }
        }
    }
    if let Some(policy) = policy {
        for (group, group_bonuses) in &policy.groups {
            for bonus in group_bonuses {
                add(BonusTarget::Group(group.to_ascii_lowercase()), bonus);
            }
        }
    }
    for (_, bonuses) in &mut totals {
        bonuses.retain(|bonus| bonus.value != 0);
    }
    totals.retain(|(_, bonuses)| !bonuses.is_empty());
    totals
}

fn equipment_label(types: &[Vec<u8>], loc: &Loc<'_>) -> String {
    types
        .iter()
        .map(|name| loc.get(&String::from_utf8_lossy(name)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn shown_lines(lines: Vec<Result<String, String>>) -> impl Iterator<Item = String> {
    lines.into_iter().map(|line| match line {
        Ok(text) => text,
        Err(reason) => format!("{NOT_RECORDED_VALUE} ({reason})"),
    })
}

fn policy_lines(policy: &Policy, loc: &Loc<'_>) -> Vec<String> {
    let mut lines = Vec::new();
    for (group, bonuses) in &policy.groups {
        lines.push(loc.get(&String::from_utf8_lossy(group)));
        lines.extend(shown_lines(loc.mio_lines(MioText::Bonuses(bonuses))));
    }
    lines.extend(shown_lines(loc.mio_lines(MioText::Modifiers(&policy.organisation_modifiers))));
    lines
}

struct PreparedMio {
    data: OrganisationViewData,
    sub_view: SubView,
    catalog: Result<Arc<mio_traits::Catalog>, String>,
    org_db: Result<Arc<OrganisationDatabase>, String>,
    emblem_sprite: String,
    tree_background: Option<String>,
    attached_policy_sprite: Option<String>,
}

fn organisation_tree<'c>(
    catalog: &'c Result<Arc<mio_traits::Catalog>, String>,
    token: &Result<String, String>,
) -> Result<Tree<'c>, String> {
    let token = token
        .as_deref()
        .map_err(|why| format!("the organisation's script key cannot be named ({why})"))?;
    let catalog = catalog
        .as_deref()
        .map_err(|why| format!("the organisation scripts could not be read ({why})"))?;
    match catalog.tree(token) {
        Ok(Some(tree)) => Ok(tree),
        Ok(None) => Err(format!(
            "'{token}' is not defined in the recorded mod stack's organisation scripts"
        )),
        Err(why) => Err(format!("its trait tree could not be read ({why})")),
    }
}

fn trait_positions<'t>(
    catalog: &Result<Arc<mio_traits::Catalog>, String>,
    tree: &Tree<'t>,
    token: &str,
) -> Result<Vec<(&'t TraitName, [i32; 2])>, String> {
    let catalog = catalog
        .as_deref()
        .map_err(|why| format!("the organisation scripts could not be read ({why})"))?;
    let mut positions = Vec::new();
    for trait_def in tree.traits {
        match catalog.position(token, &trait_def.token) {
            Ok(Some(pos)) => positions.push((trait_def, pos)),
            Ok(None) => {}
            Err(why) => {
                return Err(format!(
                    "trait '{}' cannot be placed in the tree ({why})",
                    trait_def.token
                ));
            }
        }
    }
    Ok(positions)
}

fn attached_policy<'d>(
    details: &OrganisationDetails,
    org_db: &'d Result<Arc<OrganisationDatabase>, String>,
) -> Result<Option<&'d Policy>, String> {
    let token = match &details.policy {
        None => return Ok(None),
        Some(Spelled::Text(token)) => token,
        Some(unspelled) => return Err(unspelled.missing("attached policy").unwrap_or_default()),
    };
    let db = org_db.as_deref().map_err(|why| {
        format!("the attached policy '{token}' is not readable because the organisation scripts could not be read ({why})")
    })?;
    db.policies.policy(token.as_bytes()).map(Some).map_err(|why| {
        format!("the attached policy '{token}' is not readable from the recorded mod stack ({why})")
    })
}

fn subject(data: &OrganisationViewData) -> String {
    match &data.token {
        Ok(token) => token.clone(),
        Err(_) => data.organisation_id.to_string(),
    }
}

fn tab_refusal(token: &str, sub_view: SubView, reason: &str) -> GameWindowError {
    let tab = match sub_view {
        SubView::Traits => "trait tree",
        SubView::History => "history",
        SubView::Policy => "policies",
    };
    GameWindowError::refused(format!(
        "The {tab} of military industrial organisation '{token}' cannot be shown: {reason}."
    ))
}

fn require_template<'g>(
    guis: &'g native::Guis,
    name: &str,
) -> Result<&'g Element, GameWindowError> {
    guis.template(name.as_bytes())
        .ok_or_else(|| GameWindowError::refused(missing_window_refusal(name)))
}

fn require_path<'e>(
    root: &'e Element,
    window: &str,
    path: &str,
) -> Result<&'e Element, GameWindowError> {
    find_direct(root, path).ok_or_else(|| {
        GameWindowError::refused(format!(
            "Window '{window}' has no '{path}' in the loaded interface files, so what it shows cannot be drawn."
        ))
    })
}

fn require_holder<'l>(
    layout: &'l Layout,
    element: &Element,
) -> Result<&'l native_gui::Holder, GameWindowError> {
    layout
        .holders
        .iter()
        .find(|holder| holder.path == element.path)
        .ok_or_else(|| {
            GameWindowError::refused(format!(
                "'{}' produced no layout holder, so its rows cannot be placed.",
                element.path
            ))
        })
}

const BONUS_LIST_PATH: &str = "aggregated_bonuses_window/aggregated_bonus_list";
const TREE_LIST_PATH: &str = "tree_scrollbar_window/tree_window/tree_list";
const HISTORY_LIST_PATH: &str = "history_window/variants_window/variant_history_list";
const PRODUCTION_LIST_PATH: &str = "history_window/prod_lines_window/prod_line_history_list";
const POLICY_LIST_PATH: &str = "policy_list_window/policy_list";
const TREE_WINDOW: &str = "industrial_organisation_tree_window";
const TREE_TRAITS_PATH: &str = "scrollbar_window/traits_window";
const TREE_HEADER_PATH: &str = "tree_header";

fn tree_needs_connectors(tree: &Tree<'_>) -> bool {
    tree.traits.iter().any(|trait_def| {
        !trait_def.parents.is_empty()
            || !trait_def.mutually_exclusive.is_empty()
            || !trait_def.bonuses.is_empty()
            || trait_def.organisation_modifiers.listed().next().is_some()
    })
}

fn tab_requirements(
    guis: &native::Guis,
    sub_view: SubView,
    tree: Option<&Tree<'_>>,
) -> Result<(), GameWindowError> {
    let window = sub_view.window_name();
    let root = require_template(guis, window)?;
    match sub_view {
        SubView::Traits | SubView::History => {
            require_template(guis, "progressbar_widget")?;
            require_path(root, window, BONUS_LIST_PATH)?;
        }
        SubView::Policy => {
            require_template(guis, "policy_item")?;
            require_path(root, window, POLICY_LIST_PATH)?;
        }
    }
    match sub_view {
        SubView::Traits => {
            require_path(root, window, TREE_LIST_PATH)?;
            require_template(guis, "industrial_organisation_trait_entry")?;
            let tree_root = require_template(guis, TREE_WINDOW)?;
            require_path(tree_root, TREE_WINDOW, TREE_TRAITS_PATH)?;
            if let Some(tree) = tree {
                if !tree.tree_header_texts.is_empty() {
                    require_path(tree_root, TREE_WINDOW, TREE_HEADER_PATH)?;
                    require_template(guis, "industrial_organisation_flavortext_window")?;
                }
                if tree_needs_connectors(tree) {
                    require_template(guis, "industrial_organisation_bonus_type_icon")?;
                }
            }
        }
        SubView::History => {
            require_template(guis, "industrial_organisation_history_item")?;
            require_path(root, window, HISTORY_LIST_PATH)?;
            require_path(root, window, PRODUCTION_LIST_PATH)?;
        }
        SubView::Policy => {}
    }
    Ok(())
}

fn configure_detail_main(
    root: &Element,
    loc: &Loc<'_>,
    instance: &mut Instance,
    tooltips: &mut BTreeMap<String, GameWindowTooltip>,
    prepared: &PreparedMio,
) {
    let data = &prepared.data;
    let org_db = prepared.org_db.as_deref().ok();
    let display_name = loc.organisation_name();
    set_text_by_name(instance, root, "industrial_org_name", display_name.clone());

    let traits_frame = if prepared.sub_view == SubView::Traits { 2 } else { 1 };
    let history_frame = if prepared.sub_view == SubView::History { 2 } else { 1 };
    set_frame_by_name(instance, root, "tabs/traits_tab", traits_frame);
    set_frame_by_name(instance, root, "tabs/history_tab", history_frame);

    set_sprite_by_name(instance, root, "top_left_window/icon", &prepared.emblem_sprite);
    if let Some(icon_el) = find_direct(root, "top_left_window/icon") {
        tooltips.insert(
            icon_el.path.clone(),
            GameWindowTooltip {
                title: display_name,
                body: Some(data.token.clone().unwrap_or_else(|why| why)),
                detail: None,
            },
        );
    }

    match &data.details {
        RecordedDetails::Available(details) => {
            set_text_by_name(
                instance,
                root,
                "top_left_window/size_icon_window/size",
                details.size.to_string(),
            );
            set_text_by_name(
                instance,
                root,
                "top_left_window/points",
                format_points_line(loc, &details.points.to_string()),
            );
            if let Some(size_el) = find_direct(root, "top_left_window/size_icon_window/size_icon") {
                let progress = compute_funds_progress(
                    details.funds_raw,
                    details.size,
                    details.size_cost_factor_raw,
                    &data.funds_defines,
                );
                let body = match (progress.percent, progress.threshold_reason) {
                    (Some(pct), _) => format!("Size: {} ({pct}%)", details.size),
                    (None, Some(reason)) => format!("Size: {}\n{reason}", details.size),
                    (None, None) => format!("Size: {}", details.size),
                };
                tooltips.insert(
                    size_el.path.clone(),
                    GameWindowTooltip {
                        title: "Size".to_owned(),
                        body: Some(body),
                        detail: None,
                    },
                );
            }
            if let Some(points_el) = find_direct(root, "top_left_window/points") {
                tooltips.insert(
                    points_el.path.clone(),
                    GameWindowTooltip {
                        title: format_points_line(loc, &details.points.to_string()),
                        body: Some(loc.get("INDUSTRIAL_ORG_DETAIL_TRAIT_POINTS_DELAYED")),
                        detail: None,
                    },
                );
            }
            if let Some(Spelled::Unspelled { .. }) = &details.policy {
                let reason = details
                    .policy
                    .as_ref()
                    .and_then(|policy| policy.missing("attached policy"))
                    .unwrap_or_default();
                hide_by_name(instance, root, "open_policies_window/open_policies_text");
                hide_by_name(instance, root, "open_policies_window/click_to_open_policies_text");
                hide_by_name(instance, root, "open_policies_window/attached_policy_icon");
                set_text_by_name(
                    instance,
                    root,
                    "open_policies_window/attached_policy_name",
                    NOT_RECORDED_VALUE.to_owned(),
                );
                if let Some(btn) = find_direct(root, "open_policies_window/open_policies") {
                    tooltips.insert(
                        btn.path.clone(),
                        GameWindowTooltip {
                            title: "Not recorded".to_owned(),
                            body: Some(reason),
                            detail: None,
                        },
                    );
                }
            } else if let Some(Spelled::Text(policy_token)) = &details.policy {
                hide_by_name(instance, root, "open_policies_window/open_policies_text");
                hide_by_name(
                    instance,
                    root,
                    "open_policies_window/click_to_open_policies_text",
                );
                let policy_def = org_db.and_then(|db| db.policies.policy(policy_token.as_bytes()).ok());
                let policy_name = resolve_policy_name(policy_token, policy_def, loc);
                set_text_by_name(
                    instance,
                    root,
                    "open_policies_window/attached_policy_name",
                    policy_name.clone(),
                );
                if let Some(sprite) = &prepared.attached_policy_sprite {
                    set_sprite_by_name(
                        instance,
                        root,
                        "open_policies_window/attached_policy_icon",
                        sprite,
                    );
                }
                if let Some(btn) = find_direct(root, "open_policies_window/open_policies") {
                    let mut lines = Vec::new();
                    if let Some(days) =
                        format_days_remaining(details.cooldown_hour, data.request_hour)
                    {
                        lines.push(format!("Policy cooldown: {days} days remaining"));
                    }
                    if let Some(def) = policy_def {
                        lines.extend(policy_lines(def, loc));
                    }
                    tooltips.insert(
                        btn.path.clone(),
                        GameWindowTooltip {
                            title: policy_name,
                            body: (!lines.is_empty()).then(|| lines.join("\n")),
                            detail: Some(policy_token.clone()),
                        },
                    );
                }
            } else {
                hide_by_name(instance, root, "open_policies_window/attached_policy_icon");
                hide_by_name(instance, root, "open_policies_window/attached_policy_name");
                if let Some(btn) = find_direct(root, "open_policies_window/open_policies") {
                    tooltips.insert(
                        btn.path.clone(),
                        GameWindowTooltip {
                            title: loc.get("INDUSTRIAL_ORG_POLICY_BUTTON"),
                            body: Some("No policy attached at this hour.".to_owned()),
                            detail: None,
                        },
                    );
                }
            }

            if let Some(queue_btn) =
                find_direct(root, "aggregated_bonuses_window/open_queue_button")
            {
                let tree = organisation_tree(&prepared.catalog, &data.token).ok();
                let org_token = data.token.as_deref().unwrap_or_default();
                let body = if details.queued.is_empty() {
                    "No traits queued at this hour.".to_owned()
                } else {
                    details
                        .queued
                        .iter()
                        .enumerate()
                        .map(|(idx, queued)| {
                            let name = match queued {
                                Spelled::Text(tok) => tree
                                    .as_ref()
                                    .and_then(|t| {
                                        t.traits
                                            .iter()
                                            .find(|tr| tr.token.eq_ignore_ascii_case(tok))
                                    })
                                    .map_or_else(
                                        || loc.get(tok),
                                        |tr| {
                                            resolve_trait_name(
                                                tr,
                                                org_token,
                                                data.owner_tag.as_deref(),
                                                loc,
                                            )
                                        },
                                    ),
                                Spelled::Unspelled { .. } => format!(
                                    "{NOT_RECORDED_VALUE} ({})",
                                    queued.missing("queued trait").unwrap_or_default()
                                ),
                            };
                            format!("{}. {name}", idx + 1)
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                tooltips.insert(
                    queue_btn.path.clone(),
                    GameWindowTooltip {
                        title: loc.get("INDUSTRIAL_ORG_DETAIL_WINDOW_OPEN_QUEUE_BUTTON"),
                        body: Some(body),
                        detail: None,
                    },
                );
            }

            set_frame_by_name(
                instance,
                root,
                "toggle_auto_designs",
                if details.auto_upgrade { 2 } else { 1 },
            );
        }
        RecordedDetails::Unavailable(reason) => {
            set_text_by_name(
                instance,
                root,
                "top_left_window/size_icon_window/size",
                NOT_RECORDED_VALUE.to_owned(),
            );
            set_text_by_name(
                instance,
                root,
                "top_left_window/points",
                format_points_line(loc, NOT_RECORDED_VALUE),
            );
            hide_by_name(instance, root, "open_policies_window/attached_policy_icon");
            hide_by_name(instance, root, "open_policies_window/attached_policy_name");
            set_text_by_name(
                instance,
                root,
                "open_policies_window/click_to_open_policies_text",
                NOT_RECORDED_VALUE.to_owned(),
            );
            let missing_tip = GameWindowTooltip {
                title: "Not recorded".to_owned(),
                body: Some(format!(
                    "Not recorded: organisation details are unavailable at this hour ({reason})."
                )),
                detail: None,
            };
            for path in [
                "top_left_window/size_icon_window/size_icon",
                "top_left_window/points",
                "open_policies_window/open_policies",
                "aggregated_bonuses_window/open_queue_button",
                "auto_design_text",
            ] {
                if let Some(el) = find_direct(root, path) {
                    tooltips.insert(el.path.clone(), missing_tip.clone());
                }
            }
            hide_by_name(instance, root, "toggle_auto_designs");
        }
    }

    disable_by_name(instance, root, "aggregated_bonuses_window/open_queue_button");
    disable_by_name(instance, root, "toggle_auto_designs");

    match prepared.sub_view {
        SubView::Traits => {
            hide_by_name(instance, root, "history_window");
        }
        SubView::History => {
            hide_by_name(instance, root, "tree_scrollbar_window");
            hide_by_name(instance, root, "industrial_organisation_flavortext_background");
            hide_by_name(instance, root, "toggle_auto_designs");
            hide_by_name(instance, root, "auto_design_text");
        }
        SubView::Policy => {}
    }
}

fn configure_policy_main(
    root: &Element,
    loc: &Loc<'_>,
    instance: &mut Instance,
) {
    let display_name = loc.organisation_name();
    set_text_by_name(instance, root, "title/title", display_name);
    set_text_by_name(
        instance,
        root,
        "title/instruction",
        loc.get("INDUSTRIAL_ORG_POLICY_BUTTON"),
    );
}

fn compose_funds_progressbar(
    main_layout: &Layout,
    root: &Element,
    guis: &native::Guis,
    resources: &dyn Resources,
    loc: &Loc<'_>,
    prepared: &PreparedMio,
    plan: &mut ComposedPlan,
) -> Result<(), GameWindowError> {
    let pb_root = require_template(guis, "progressbar_widget")?;
    let refuse = |what: &str| {
        GameWindowError::refused(format!(
            "Window '{DETAIL_WINDOW_NAME}' is missing required '{what}' for the funds bar."
        ))
    };
    let top_left = find_direct(root, "top_left_window").ok_or_else(|| refuse("top_left_window"))?;
    let holder = container_holder(main_layout, &top_left.path).ok_or_else(|| refuse("top_left_window layout"))?;
    let pos = find_direct(top_left, "funds_progressbar_position")
        .and_then(|el| match &el.fields {
            Fields::PositionType(p) => Some(p.position),
            _ => None,
        })
        .ok_or_else(|| refuse("top_left_window/funds_progressbar_position"))?;

    let (percent, tooltip) = match &prepared.data.details {
        RecordedDetails::Available(details) => {
            let progress = compute_funds_progress(
                details.funds_raw,
                details.size,
                details.size_cost_factor_raw,
                &prepared.data.funds_defines,
            );
            let pct = progress.percent.unwrap_or(0);
            let body = match (progress.threshold, progress.percent) {
                (Some(limit), Some(p)) => format!(
                    "{:.0} / {:.0} ({p}%)",
                    progress.funds.max(0.0),
                    limit
                ),
                _ => format!(
                    "{:.0} / {NOT_RECORDED_VALUE}\n{}",
                    progress.funds.max(0.0),
                    progress.threshold_reason.unwrap_or_default()
                ),
            };
            (
                pct,
                GameWindowTooltip {
                    title: loc.get("INDUSTRIAL_ORG_DETAIL_TOOLTIP_FUNDS_HEADER"),
                    body: Some(body),
                    detail: None,
                },
            )
        }
        RecordedDetails::Unavailable(reason) => (
            0,
            GameWindowTooltip {
                title: loc.get("INDUSTRIAL_ORG_DETAIL_TOOLTIP_FUNDS_HEADER"),
                body: Some(format!(
                    "Not recorded: funds are unavailable at this hour ({reason})."
                )),
                detail: None,
            },
        ),
    };

    let viewport = DEFAULT_VIEWPORT.map(|value| value as i32);
    let pb_layout = layout_window(pb_root, resources, viewport, |instance| {
        if let Some(bar) = find_direct(pb_root, "bar_progress")
            && let Ok(bound) = instance.control(pb_root, bar)
        {
            bound.progress = Some(percent.clamp(0, 100));
        }
        if percent <= 0 {
            hide_by_name(instance, pb_root, "bar_begin");
        }
        hide_by_name(instance, pb_root, "bar_end");
    })?;

    let mut overrides = BTreeMap::new();
    if let Some(bg) = find_direct(pb_root, "bar_background") {
        overrides.insert(bg.path.clone(), tooltip.clone());
    }
    if let Some(bar) = find_direct(pb_root, "bar_progress") {
        overrides.insert(bar.path.clone(), tooltip);
    }
    plan.sub_windows.push(ComposedSubWindow {
        before_part: holder.before,
        parts: holder.place(pos, &pb_layout),
        regions: holder.place_regions(pos, &pb_layout.regions),
        diagnostics: pb_layout.diagnostics,
        root_window: "progressbar_widget".to_owned(),
        path_prefix: "funds_progressbar/".to_owned(),
        close_paths: Vec::new(),
        tooltip_overrides: overrides,
    });
    Ok(())
}

struct BonusRow<'a> {
    template: &'a Element,
    window: &'static str,
    child: &'static str,
    text: String,
    tooltip: Option<GameWindowTooltip>,
}

struct BonusRows<'a> {
    separator: Option<&'a Element>,
    modifier: Option<&'a Element>,
    bonus: Option<&'a Element>,
    rows: Vec<BonusRow<'a>>,
}

impl<'a> BonusRows<'a> {
    fn push(
        &mut self,
        template: Option<&'a Element>,
        window: &'static str,
        child: &'static str,
        text: String,
        tooltip: Option<GameWindowTooltip>,
    ) -> Result<(), GameWindowError> {
        let template = template.ok_or_else(|| GameWindowError::refused(missing_window_refusal(window)))?;
        self.rows.push(BonusRow { template, window, child, text, tooltip });
        Ok(())
    }

    fn separator(&mut self, title: String) -> Result<(), GameWindowError> {
        self.push(self.separator, "industrial_organisation_detail_separator", "separator_name", title, None)
    }

    fn modifier(&mut self, text: String, tooltip: Option<GameWindowTooltip>) -> Result<(), GameWindowError> {
        self.push(self.modifier, "industrial_organisation_detail_modifier_item", "modifier_text", text, tooltip)
    }

    fn bonus(&mut self, text: String) -> Result<(), GameWindowError> {
        self.push(self.bonus, "industrial_organisation_detail_bonus_item", "bonus_text", text, None)
    }

    fn compose(
        self,
        list: &Element,
        holder: &native_gui::Holder,
        resources: &dyn Resources,
        plan: &mut ComposedPlan,
    ) -> Result<(), GameWindowError> {
        let sizes = self
            .rows
            .iter()
            .map(|row| row_size(row.template, row.window))
            .collect::<Result<Vec<_>, _>>()?;
        let places = plan.list_rows(list, holder, &sizes, resources)?;
        let viewport = DEFAULT_VIEWPORT.map(|value| value as i32);
        for (index, (row, place)) in self.rows.into_iter().zip(places).enumerate() {
            let Some(place) = place else { continue };
            let BonusRow { template, window, child, text, tooltip } = row;
            let layout = layout_window(template, resources, viewport, |instance| {
                set_text_by_name(instance, template, child, text);
            })?;
            let mut overrides = BTreeMap::new();
            if let Some(tip) = tooltip {
                overrides.insert(template.path.clone(), tip);
            }
            plan.sub_windows.push(ComposedSubWindow {
                before_part: holder.before,
                parts: holder.place(place, &layout),
                regions: holder.place_regions(place, &layout.regions),
                diagnostics: layout.diagnostics,
                root_window: window.to_owned(),
                path_prefix: format!("bonus:{index}/"),
                close_paths: Vec::new(),
                tooltip_overrides: overrides,
            });
        }
        Ok(())
    }
}

fn compose_aggregated_bonuses(
    main_layout: &Layout,
    root: &Element,
    guis: &native::Guis,
    resources: &dyn Resources,
    loc: &Loc<'_>,
    prepared: &PreparedMio,
    plan: &mut ComposedPlan,
) -> Result<(), GameWindowError> {
    let list_el = require_path(root, prepared.sub_view.window_name(), BONUS_LIST_PATH)?;
    let holder = require_holder(main_layout, list_el)?;
    let mut rows = BonusRows {
        separator: guis.template(b"industrial_organisation_detail_separator"),
        modifier: guis.template(b"industrial_organisation_detail_modifier_item"),
        bonus: guis.template(b"industrial_organisation_detail_bonus_item"),
        rows: Vec::new(),
    };

    match &prepared.data.details {
        RecordedDetails::Unavailable(reason) => {
            rows.separator(
                loc.get("INDUSTRIAL_ORG_DETAIL_MIO_MODIFIERS"),
            )?;
            rows.modifier(
                NOT_RECORDED_VALUE.to_owned(),
                Some(GameWindowTooltip {
                    title: "Not recorded".to_owned(),
                    body: Some(format!(
                        "Not recorded: organisation modifiers and unlocked traits are unavailable at this hour ({reason})."
                    )),
                    detail: None,
                }),
            )?;
        }
        RecordedDetails::Available(details) => {
            rows.separator(
                loc.get("INDUSTRIAL_ORG_DETAIL_MIO_MODIFIERS"),
            )?;
            rows.modifier(
                modifier_item(
                    loc,
                    "MODIFIER_MIO_RESEARCH_BONUS_MIO_CONTEXT",
                    &format_signed_percent_fixed5(details.research_bonus_raw),
                ),
                None,
            )?;
            if details.funds_gain_factor_raw != 0 {
                rows.modifier(
                    modifier_item(
                        loc,
                        "MODIFIER_MIO_FUNDS_GAIN",
                        &format_signed_percent_fixed5(details.funds_gain_factor_raw),
                    ),
                    None,
                )?;
            }
            if details.size_cost_factor_raw != 0 {
                rows.modifier(
                    modifier_item(
                        loc,
                        "MODIFIER_MIO_FUNDS_SIZE_UP_REQUIREMENT",
                        &format_signed_percent_fixed5(details.size_cost_factor_raw),
                    ),
                    None,
                )?;
            }
            if details.task_capacity != 0 {
                rows.modifier(
                    modifier_item(
                        loc,
                        "MODIFIER_MIO_TASK_CAPACITY",
                        &details.task_capacity.to_string(),
                    ),
                    None,
                )?;
            }

            let tree = organisation_tree(&prepared.catalog, &prepared.data.token);
            let policy = attached_policy(details, &prepared.org_db);
            if let Ok(Some(initial)) = tree.as_ref().map(|t| t.initial) {
                rows.separator(
                    loc.get("INDUSTRIAL_ORG_DETAIL_INITIAL_TRAIT_BONUSES"),
                )?;
                let initial_name = resolve_trait_name(
                    initial,
                    prepared.data.token.as_deref().unwrap_or_default(),
                    prepared.data.owner_tag.as_deref(),
                    loc,
                );
                rows.modifier(
                    loc.fill("INDUSTRIAL_ORG_DETAIL_MIO_INITIAL_TRAIT_ITEM",
                        &[("NAME", &initial_name)],
                    ),
                    None,
                )?;
            }

            let unlocked = details
                .unlocked
                .iter()
                .find_map(|entry| entry.missing("unlocked trait"))
                .map_or(Ok(()), Err);
            match (&tree, &policy, &unlocked) {
                (Ok(tree), Ok(policy), Ok(())) => {
                    let totals = aggregate_bonuses(details, tree, *policy);
                    if !totals.is_empty() {
                        rows.separator(
                            loc.get("INDUSTRIAL_ORG_DETAIL_EQUIPMENT_BONUSES"),
                        )?;
                        for (target, bonuses) in totals {
                            match &target {
                                BonusTarget::Organisation => {}
                                BonusTarget::Equipment(types) => {
                                    rows.separator(equipment_label(types, loc))?;
                                }
                                BonusTarget::Group(group) => {
                                    rows.separator(loc.get(&String::from_utf8_lossy(group)))?;
                                }
                            }
                            for line in loc.mio_lines(MioText::Bonuses(&bonuses)) {
                                match line {
                                    Ok(text) => rows.bonus(text)?,
                                    Err(reason) => rows.modifier(
                                        NOT_RECORDED_VALUE.to_owned(),
                                        Some(GameWindowTooltip {
                                            title: "Not available".to_owned(),
                                            body: Some(format!("This bonus cannot be described: {reason}.")),
                                            detail: None,
                                        }),
                                    )?,
                                }
                            }
                        }
                    }
                }
                (Err(reason), _, _) | (_, Err(reason), _) | (_, _, Err(reason)) => {
                    rows.separator(
                        loc.get("INDUSTRIAL_ORG_DETAIL_EQUIPMENT_BONUSES"),
                    )?;
                    rows.modifier(
                        NOT_RECORDED_VALUE.to_owned(),
                        Some(GameWindowTooltip {
                            title: "Not available".to_owned(),
                            body: Some(format!(
                                "The initial trait and equipment bonuses cannot be shown: {reason}."
                            )),
                            detail: None,
                        }),
                    )?;
                }
            }
        }
    }
    rows.compose(list_el, holder, resources, plan)
}

struct ConnectorPlace<'a> {
    holder: &'a native_gui::Holder,
    icon_tpl: &'a Element,
    resources: &'a dyn Resources,
    viewport: [i32; 2],
}

fn push_line_segment(
    ctx: &ConnectorPlace<'_>,
    sprite: &str,
    frame: i32,
    pos: [i32; 2],
    prefix: String,
    plan: &mut ComposedPlan,
) -> Result<(), GameWindowError> {
    let layout = layout_window(ctx.icon_tpl, ctx.resources, ctx.viewport, |instance| {
        set_sprite_by_name(instance, ctx.icon_tpl, "icon", sprite);
        set_frame_by_name(instance, ctx.icon_tpl, "icon", frame);
    })?;
    plan.sub_windows.push(ComposedSubWindow {
        before_part: ctx.holder.before,
        parts: ctx.holder.place(pos, &layout),
        regions: Vec::new(),
        diagnostics: layout.diagnostics,
        root_window: "industrial_organisation_bonus_type_icon".to_owned(),
        path_prefix: prefix,
        close_paths: Vec::new(),
        tooltip_overrides: BTreeMap::new(),
    });
    Ok(())
}

fn compose_parent_link(
    ctx: &ConnectorPlace<'_>,
    parent_pos: [i32; 2],
    child_pos: [i32; 2],
    active: bool,
    link_index: usize,
    plan: &mut ComposedPlan,
) -> Result<(), GameWindowError> {
    let frame = if active { 3 } else { 1 };
    let px = parent_pos[0] + TRAIT_ICON_CENTER_X - 8;
    let py_start = parent_pos[1] + TRAIT_ICON_BOTTOM_Y - 12;
    let cx = child_pos[0] + TRAIT_ICON_CENTER_X - 8;
    let cy_end = child_pos[1] + TRAIT_ICON_TOP_Y - 4;
    let mid_y = (py_start + cy_end) / 2;
    let step = 14i32;
    let mut seg = 0usize;

    let mut y = py_start;
    while y < mid_y {
        push_line_segment(
            ctx,
            "GFX_focus_link_up_down",
            frame,
            [px, y],
            format!("link:{link_index}:v1:{seg}/"),
            plan,
        )?;
        y += step;
        seg += 1;
    }

    if px != cx {
        let (x0, x1) = if px < cx { (px, cx) } else { (cx, px) };
        let mut x = x0;
        while x <= x1 {
            push_line_segment(
                ctx,
                "GFX_focus_link_left_right",
                frame,
                [x, mid_y],
                format!("link:{link_index}:h:{seg}/"),
                plan,
            )?;
            x += step;
            seg += 1;
        }
    }

    let mut y2 = mid_y;
    while y2 < cy_end {
        push_line_segment(
            ctx,
            "GFX_focus_link_up_down",
            frame,
            [cx, y2],
            format!("link:{link_index}:v2:{seg}/"),
            plan,
        )?;
        y2 += step;
        seg += 1;
    }
    Ok(())
}

fn compose_exclusive_link(
    ctx: &ConnectorPlace<'_>,
    left_pos: [i32; 2],
    right_pos: [i32; 2],
    pair_index: usize,
    plan: &mut ComposedPlan,
) -> Result<(), GameWindowError> {
    let x0 = left_pos[0] + TRAIT_ICON_RIGHT_X - 4;
    let x1 = right_pos[0] + TRAIT_ICON_LEFT_X - 12;
    let y = (left_pos[1] + right_pos[1]) / 2 + TRAIT_ICON_MID_Y - 8;
    let mut x = x0;
    let mut seg = 0usize;
    while x <= x1 {
        push_line_segment(
            ctx,
            "GFX_focus_exclusive_line1",
            1,
            [x, y],
            format!("excl:{pair_index}:{seg}/"),
            plan,
        )?;
        x += 14;
        seg += 1;
    }
    let mid_x = (x0 + x1) / 2 - 8;
    push_line_segment(
        ctx,
        "GFX_focus_link_exclusive",
        1,
        [mid_x, y - 8],
        format!("excl:{pair_index}:mid/"),
        plan,
    )?;
    Ok(())
}

fn format_trait_tooltip(
    trait_def: &TraitName,
    name: &str,
    status_line: &str,
    loc: &Loc<'_>,
) -> GameWindowTooltip {
    let mut lines = vec![status_line.to_owned()];
    if !trait_def.equipment_types.is_empty() {
        lines.push(equipment_label(&trait_def.equipment_types, loc));
    }
    lines.extend(shown_lines(loc.mio_lines(MioText::Bonuses(&trait_def.bonuses))));
    lines.extend(shown_lines(loc.mio_lines(MioText::TraitModifiers(trait_def))));
    GameWindowTooltip {
        title: name.to_owned(),
        body: Some(lines.join("\n")),
        detail: Some(trait_def.token.clone()),
    }
}

fn compose_trait_tree(
    main_layout: &Layout,
    root: &Element,
    guis: &native::Guis,
    resources: &dyn Resources,
    loc: &Loc<'_>,
    prepared: &PreparedMio,
    plan: &mut ComposedPlan,
) -> Result<(), GameWindowError> {
    let named = subject(&prepared.data);
    let tree = organisation_tree(&prepared.catalog, &prepared.data.token)
        .map_err(|reason| tab_refusal(&named, SubView::Traits, &reason))?;
    let org_token = prepared.data.token.as_deref().unwrap_or_default();
    let positions = trait_positions(&prepared.catalog, &tree, org_token)
        .map_err(|reason| tab_refusal(&named, SubView::Traits, &reason))?;
    let tree_list = require_path(root, DETAIL_WINDOW_NAME, TREE_LIST_PATH)?;
    let tree_holder = require_holder(main_layout, tree_list)?;
    let tree_tpl = require_template(guis, TREE_WINDOW)?;
    let viewport = DEFAULT_VIEWPORT.map(|value| value as i32);
    let tree_layout = layout_window(tree_tpl, resources, viewport, |instance| {
        if let Some(bg) = &prepared.tree_background
            && let Ok(bound) = instance.control(tree_tpl, tree_tpl)
        {
            bound.sprite = Some(bg.as_bytes().to_vec());
        }
    })?;
    let traits_el = require_path(tree_tpl, TREE_WINDOW, TREE_TRAITS_PATH)?;
    let mut traits_box = container_holder(&tree_layout, &traits_el.path).ok_or_else(|| {
        GameWindowError::refused(format!(
            "'{TREE_WINDOW}/{TREE_TRAITS_PATH}' produced no layout holder, so the traits cannot be placed."
        ))
    })?;
    let header_holder = find_direct(tree_tpl, TREE_HEADER_PATH)
        .and_then(|el| container_holder(&tree_layout, &el.path));

    let tree_parts = tree_holder.place([0, 0], &tree_layout);
    let tree_regions = tree_holder.place_regions([0, 0], &tree_layout.regions);
    let tree_origin = tree_holder.origin;
    let tree_before = tree_holder.before;

    plan.sub_windows.push(ComposedSubWindow {
        before_part: tree_before,
        parts: tree_parts,
        regions: tree_regions,
        diagnostics: tree_layout.diagnostics,
        root_window: "industrial_organisation_tree_window".to_owned(),
        path_prefix: "mio_tree/".to_owned(),
        close_paths: Vec::new(),
        tooltip_overrides: BTreeMap::new(),
    });

    let min_x = positions.iter().map(|(_, p)| p[0]).min().unwrap_or(0);
    let min_y = positions.iter().map(|(_, p)| p[1]).min().unwrap_or(0);
    let max_x = positions.iter().map(|(_, p)| p[0]).max().unwrap_or(0);
    let max_y = positions.iter().map(|(_, p)| p[1]).max().unwrap_or(0);
    let cols = (max_x - min_x + 1).max(1);
    let rows = (max_y - min_y + 1).max(1);
    let avail_w = 870i32;
    let avail_h = 600i32;
    let step_x = if cols * TREE_STEP_X > avail_w {
        (avail_w / cols).max(66)
    } else {
        TREE_STEP_X
    };
    let step_y = if rows * TREE_STEP_Y > avail_h {
        (avail_h / rows).max(88)
    } else {
        TREE_STEP_Y
    };

    if !tree.tree_header_texts.is_empty() {
        let mut hdr = header_holder.ok_or_else(|| {
            GameWindowError::refused(format!(
                "'{TREE_WINDOW}/{TREE_HEADER_PATH}' produced no layout holder, so the tree's header texts cannot be placed."
            ))
        })?;
        let flavor_tpl = require_template(guis, "industrial_organisation_flavortext_window")?;
        hdr.origin = [tree_origin[0] + hdr.origin[0], tree_origin[1] + hdr.origin[1]];
        hdr.logical = [
            tree_holder.logical[0] + hdr.logical[0],
            tree_holder.logical[1] + hdr.logical[1],
        ];
        hdr.before = tree_before;
        for (idx, header_text) in tree.tree_header_texts.iter().enumerate() {
            let label = loc.get(&header_text.text);
            let layout = layout_window(flavor_tpl, resources, viewport, |instance| {
                set_text_by_name(instance, flavor_tpl, "flavortext", label);
            })?;
            let px = (header_text.x - min_x) * step_x + 45;
            plan.sub_windows.push(ComposedSubWindow {
                before_part: tree_before,
                parts: hdr.place([px, 0], &layout),
                regions: Vec::new(),
                diagnostics: layout.diagnostics,
                root_window: "industrial_organisation_flavortext_window".to_owned(),
                path_prefix: format!("mio_header:{idx}/"),
                close_paths: Vec::new(),
                tooltip_overrides: BTreeMap::new(),
            });
        }
    }

    traits_box.origin = [
        tree_origin[0] + traits_box.origin[0],
        tree_origin[1] + traits_box.origin[1],
    ];
    traits_box.logical = [
        tree_holder.logical[0] + traits_box.logical[0],
        tree_holder.logical[1] + traits_box.logical[1],
    ];
    traits_box.clip = Some([
        (tree_origin[0] + 45.0).round() as i32,
        (tree_origin[1] + 25.0).round() as i32,
        (tree_origin[0] + 950.0).round() as i32,
        (tree_origin[1] + 665.0).round() as i32,
    ]);
    traits_box.before = tree_before;

    let organisation = prepared.data.organisation_id;
    let status_of = |token: &str| match trait_status(&prepared.data.details, token) {
        TraitStatus::Locked => match loc.trait_condition(organisation, token, OrganisationCondition::Available) {
            Ok(true) => TraitStatus::Available,
            Ok(false) => TraitStatus::Locked,
            Err(why) => TraitStatus::Unknown(format!(
                "Not decided: whether the trait can be unlocked cannot be evaluated at this hour ({why})."
            )),
        },
        recorded => recorded,
    };
    let mut hidden_why: HashMap<String, String> = HashMap::new();
    let positions: Vec<(&TraitName, [i32; 2])> = positions
        .into_iter()
        .filter(|(trait_def, _)| {
            match loc.trait_condition(organisation, &trait_def.token, OrganisationCondition::Visible) {
                Ok(visible) => visible,
                Err(why) => {
                    hidden_why.insert(
                        trait_def.token.to_ascii_lowercase(),
                        format!("Not decided: whether the trait is visible cannot be evaluated at this hour ({why})."),
                    );
                    true
                }
            }
        })
        .collect();

    let pixel_of = |grid: [i32; 2]| -> [i32; 2] {
        [(grid[0] - min_x) * step_x, (grid[1] - min_y) * step_y]
    };
    let pos_by_token: HashMap<String, [i32; 2]> = positions
        .iter()
        .map(|(tr, grid)| (tr.token.to_ascii_lowercase(), pixel_of(*grid)))
        .collect();

    let bonus_icon_tpl = if tree_needs_connectors(&tree) {
        Some(require_template(guis, "industrial_organisation_bonus_type_icon")?)
    } else {
        None
    };
    if let Some(icon_tpl) = bonus_icon_tpl {
        let connector_ctx = ConnectorPlace {
            holder: &traits_box,
            icon_tpl,
            resources,
            viewport,
        };
        let mut link_idx = 0usize;
        for (trait_def, grid) in &positions {
            let child_px = pixel_of(*grid);
            let child_unlocked = status_of(&trait_def.token) == TraitStatus::Unlocked;
            for group in &trait_def.parents {
                for parent_token in &group.traits {
                    let key = parent_token.to_ascii_lowercase();
                    if let Some(&parent_px) = pos_by_token.get(&key) {
                        let both_unlocked =
                            child_unlocked && status_of(parent_token) == TraitStatus::Unlocked;
                        compose_parent_link(
                            &connector_ctx,
                            parent_px,
                            child_px,
                            both_unlocked,
                            link_idx,
                            plan,
                        )?;
                        link_idx += 1;
                    }
                }
            }
        }
        let mut seen_pairs: BTreeSet<(String, String)> = BTreeSet::new();
        let mut pair_idx = 0usize;
        for (trait_def, grid) in &positions {
            let left_key = trait_def.token.to_ascii_lowercase();
            let left_px = pixel_of(*grid);
            for other in &trait_def.mutually_exclusive {
                let right_key = other.to_ascii_lowercase();
                let canonical = if left_key < right_key {
                    (left_key.clone(), right_key.clone())
                } else {
                    (right_key.clone(), left_key.clone())
                };
                if !seen_pairs.insert(canonical) {
                    continue;
                }
                if let Some(&right_px) = pos_by_token.get(&right_key) {
                    let (a, b) = if left_px[0] <= right_px[0] {
                        (left_px, right_px)
                    } else {
                        (right_px, left_px)
                    };
                    compose_exclusive_link(&connector_ctx, a, b, pair_idx, plan)?;
                    pair_idx += 1;
                }
            }
        }
    }

    let trait_tpl = require_template(guis, "industrial_organisation_trait_entry")?;
    let has_sprite = |name: &str| resources.sprite_info(name.as_bytes()).is_some();

    for (idx, (trait_def, grid)) in positions.iter().enumerate() {
        let px = pixel_of(*grid);
        let status = match hidden_why.get(&trait_def.token.to_ascii_lowercase()) {
            Some(why) => TraitStatus::Unknown(why.clone()),
            None => status_of(&trait_def.token),
        };
        let queue_pos = match status {
            TraitStatus::Queued(position) => Some(position),
            _ => None,
        };
        let frame = match status {
            TraitStatus::Unlocked => Some(1),
            TraitStatus::Queued(_) => Some(2),
            TraitStatus::Available | TraitStatus::Locked => Some(3),
            TraitStatus::Unknown(_) => None,
        };
        let frame_sprite = if trait_def.special_background
            && has_sprite("GFX_country_spefific_org_trait_button")
        {
            "GFX_country_spefific_org_trait_button"
        } else {
            "GFX_industrial_org_trait_button"
        };
        let trait_sprite = if !trait_def.icon.is_empty() && has_sprite(&trait_def.icon) {
            trait_def.icon.as_str()
        } else {
            UNKNOWN_ICON_SPRITE
        };
        let trait_name =
            resolve_trait_name(trait_def, org_token, prepared.data.owner_tag.as_deref(), loc);
        let status_line = match &status {
            TraitStatus::Unknown(reason) => format!("Status: {NOT_RECORDED_VALUE} ({reason})"),
            TraitStatus::Unlocked => "Status: Unlocked".to_owned(),
            TraitStatus::Queued(pos) => format!("Status: Queued (#{pos})"),
            TraitStatus::Available => "Status: Available".to_owned(),
            TraitStatus::Locked => "Status: Locked".to_owned(),
        };

        let trait_layout = layout_window(trait_tpl, resources, viewport, |instance| {
            hide_by_name(instance, trait_tpl, "ai_value");
            set_sprite_by_name(
                instance,
                trait_tpl,
                "industrial_organisation_trait_icon/icon_frame",
                frame_sprite,
            );
            match frame {
                Some(frame) => set_frame_by_name(
                    instance,
                    trait_tpl,
                    "industrial_organisation_trait_icon/icon_frame",
                    frame,
                ),
                None => hide_by_name(
                    instance,
                    trait_tpl,
                    "industrial_organisation_trait_icon/icon_frame",
                ),
            }
            disable_by_name(
                instance,
                trait_tpl,
                "industrial_organisation_trait_icon/icon_frame",
            );
            set_sprite_by_name(
                instance,
                trait_tpl,
                "industrial_organisation_trait_icon/trait_icon",
                trait_sprite,
            );
            if let Some(pos) = queue_pos {
                set_text_by_name(
                    instance,
                    trait_tpl,
                    "industrial_organisation_trait_icon/queue_number_window/queue_number",
                    pos.to_string(),
                );
            } else {
                hide_by_name(
                    instance,
                    trait_tpl,
                    "industrial_organisation_trait_icon/queue_number_window",
                );
            }
        })?;

        let mut overrides = BTreeMap::new();
        let tooltip_path = if frame.is_some() {
            "industrial_organisation_trait_icon/icon_frame"
        } else {
            "industrial_organisation_trait_icon/trait_icon"
        };
        if let Some(btn) = find_direct(trait_tpl, tooltip_path) {
            overrides.insert(
                btn.path.clone(),
                format_trait_tooltip(trait_def, &trait_name, &status_line, loc),
            );
        }

        plan.sub_windows.push(ComposedSubWindow {
            before_part: tree_before,
            parts: traits_box.place(px, &trait_layout),
            regions: traits_box.place_regions(px, &trait_layout.regions),
            diagnostics: trait_layout.diagnostics,
            root_window: "industrial_organisation_trait_entry".to_owned(),
            path_prefix: format!("trait:{idx}/"),
            close_paths: Vec::new(),
            tooltip_overrides: overrides,
        });

        if let Some(icon_tpl) = bonus_icon_tpl {
            let mut badges: Vec<&str> = Vec::new();
            if trait_def
                .bonuses
                .iter()
                .any(|b| !is_production_stat(&b.name))
            {
                badges.push("GFX_design_team_icon");
            }
            if trait_def.bonuses.iter().any(|b| is_production_stat(&b.name)) {
                badges.push("GFX_industrial_manufacturer_icon");
            }
            if trait_def.organisation_modifiers.listed().next().is_some() {
                badges.push("GFX_organization_modifier_icon");
            }
            let total_w = badges.len() as i32 * 22;
            let start_x = px[0] + TRAIT_ICON_CENTER_X - total_w / 2;
            let badge_y = px[1] + 74;
            for (b_idx, badge_sprite) in badges.into_iter().enumerate() {
                if !has_sprite(badge_sprite) {
                    continue;
                }
                let b_layout = layout_window(icon_tpl, resources, viewport, |instance| {
                    set_sprite_by_name(instance, icon_tpl, "icon", badge_sprite);
                })?;
                plan.sub_windows.push(ComposedSubWindow {
                    before_part: tree_before,
                    parts: traits_box.place([start_x + b_idx as i32 * 22, badge_y], &b_layout),
                    regions: Vec::new(),
                    diagnostics: b_layout.diagnostics,
                    root_window: "industrial_organisation_bonus_type_icon".to_owned(),
                    path_prefix: format!("trait:{idx}:badge:{b_idx}/"),
                    close_paths: Vec::new(),
                    tooltip_overrides: BTreeMap::new(),
                });
            }
        }
    }
    Ok(())
}

enum HistoryKind {
    Designed,
    Produced,
}

struct HistoryRowText {
    name: String,
    archetype: String,
    fact: String,
    tooltip: Option<GameWindowTooltip>,
}

fn history_row_text(entry: &HistoryEntry, kind: &HistoryKind, loc: &Loc<'_>) -> HistoryRowText {
    let archetype = if entry.archetype.is_empty() {
        String::new()
    } else {
        loc.get(&entry.archetype)
    };
    let (name, tooltip) = match &entry.name {
        Ok(name) if *name == entry.archetype => (loc.get(name), None),
        Ok(name) => (name.clone(), None),
        Err(reason) => (
            NOT_RECORDED_VALUE.to_owned(),
            Some(GameWindowTooltip {
                title: "Not recorded".to_owned(),
                body: Some(reason.clone()),
                detail: None,
            }),
        ),
    };
    let fact = match (kind, &entry.design) {
        (HistoryKind::Designed, DesignDate::Designed(hour)) => loc.fill(
            "INDUSTRIAL_ORG_DETAIL_WINDOW_HISTORY_ENTRY_FLAVOR_DESIGN_DATE",
            &[("DATE", &loc.date(*hour))],
        ),
        _ => loc.fill(
            "INDUSTRIAL_ORG_DETAIL_WINDOW_HISTORY_ENTRY_FLAVOR_UNITS_PRODUCED",
            &[("NUMBER", &entry.units.to_string())],
        ),
    };
    HistoryRowText { name, archetype, fact, tooltip }
}

fn compose_history_list(
    main_layout: &Layout,
    root: &Element,
    item_tpl: &Element,
    resources: &dyn Resources,
    loc: &Loc<'_>,
    list: (&str, &str, HistoryKind, &HistoryList<'_>),
    plan: &mut ComposedPlan,
) -> Result<(), GameWindowError> {
    let (path, prefix, kind, rows) = list;
    let list_el = require_path(root, DETAIL_WINDOW_NAME, path)?;
    let holder = require_holder(main_layout, list_el)?;
    let viewport = DEFAULT_VIEWPORT.map(|value| value as i32);
    let size = row_size(item_tpl, "industrial_organisation_history_item")?;
    let texts: Vec<HistoryRowText> = match rows {
        HistoryList::Missing(reason) => vec![HistoryRowText {
            name: NOT_RECORDED_VALUE.to_owned(),
            archetype: "Not recorded".to_owned(),
            fact: reason.clone(),
            tooltip: Some(GameWindowTooltip {
                title: "Not recorded".to_owned(),
                body: Some(reason.clone()),
                detail: None,
            }),
        }],
        HistoryList::Entries(entries) => entries
            .iter()
            .map(|entry| history_row_text(entry, &kind, loc))
            .collect(),
    };
    let places = plan.list_rows(list_el, holder, &vec![size; texts.len()], resources)?;
    for (idx, (text, place)) in texts.into_iter().zip(places).enumerate() {
        let Some(place) = place else { continue };
        let HistoryRowText { name, archetype, fact, tooltip } = text;
        let layout = layout_window(item_tpl, resources, viewport, |instance| {
            hide_by_name(instance, item_tpl, "equipment_icon");
            set_text_by_name(instance, item_tpl, "equipment_name", name);
            set_text_by_name(instance, item_tpl, "equipment_archetype_name", archetype);
            set_text_by_name(instance, item_tpl, "flavor_fact", fact);
            disable_by_name(instance, item_tpl, "upgrade_variant_button");
        })?;
        let mut overrides = BTreeMap::new();
        if let Some(tooltip) = tooltip {
            overrides.insert(item_tpl.path.clone(), tooltip);
        }
        plan.sub_windows.push(ComposedSubWindow {
            before_part: holder.before,
            parts: holder.place(place, &layout),
            regions: holder.place_regions(place, &layout.regions),
            diagnostics: layout.diagnostics,
            root_window: "industrial_organisation_history_item".to_owned(),
            path_prefix: format!("{prefix}:{idx}/"),
            close_paths: Vec::new(),
            tooltip_overrides: overrides,
        });
    }
    Ok(())
}

fn compose_history_lists(
    main_layout: &Layout,
    root: &Element,
    guis: &native::Guis,
    resources: &dyn Resources,
    loc: &Loc<'_>,
    prepared: &PreparedMio,
    plan: &mut ComposedPlan,
) -> Result<(), GameWindowError> {
    let item_tpl = require_template(guis, "industrial_organisation_history_item")?;
    let lists = match &prepared.data.history {
        Some(entries) => history_lists(entries),
        None => {
            let reason = format!(
                "Not recorded: the organisation's history is unavailable at this hour ({}).",
                prepared
                    .data
                    .history_reason
                    .as_deref()
                    .unwrap_or("not read")
            );
            [HistoryList::Missing(reason.clone()), HistoryList::Missing(reason)]
        }
    };
    let [designed, produced] = lists;
    compose_history_list(
        main_layout,
        root,
        item_tpl,
        resources,
        loc,
        (HISTORY_LIST_PATH, "history", HistoryKind::Designed, &designed),
        plan,
    )?;
    compose_history_list(
        main_layout,
        root,
        item_tpl,
        resources,
        loc,
        (PRODUCTION_LIST_PATH, "production", HistoryKind::Produced, &produced),
        plan,
    )
}

fn compose_policy_list(
    main_layout: &Layout,
    root: &Element,
    guis: &native::Guis,
    resources: &dyn Resources,
    loc: &Loc<'_>,
    prepared: &PreparedMio,
    plan: &mut ComposedPlan,
) -> Result<(), GameWindowError> {
    let item_tpl = require_template(guis, "policy_item")?;
    let list_el = require_path(root, POLICY_WINDOW_NAME, POLICY_LIST_PATH)?;
    let holder = require_holder(main_layout, list_el)?;
    let org_db = prepared
        .org_db
        .as_deref()
        .map_err(|why| {
            tab_refusal(
                &subject(&prepared.data),
                SubView::Policy,
                &format!("the organisation scripts could not be read ({why})"),
            )
        })?;
    let size = row_size(item_tpl, "policy_item")?;
    let viewport = DEFAULT_VIEWPORT.map(|value| value as i32);
    let has_sprite = |name: &str| resources.sprite_info(name.as_bytes()).is_some();

    match &prepared.data.details {
        RecordedDetails::Unavailable(reason) => {
            let layout = layout_window(item_tpl, resources, viewport, |instance| {
                hide_by_name(instance, item_tpl, "ai_value");
                set_text_by_name(instance, item_tpl, "name", NOT_RECORDED_VALUE.to_owned());
                set_text_by_name(
                    instance,
                    item_tpl,
                    "attach_cost",
                    NOT_RECORDED_VALUE.to_owned(),
                );
                set_sprite_by_name(instance, item_tpl, "icon", UNKNOWN_ICON_SPRITE);
                disable_by_name(instance, item_tpl, "background_button");
            })?;
            let mut overrides = BTreeMap::new();
            if let Some(btn) = find_direct(item_tpl, "background_button") {
                overrides.insert(
                    btn.path.clone(),
                    GameWindowTooltip {
                        title: "Not recorded".to_owned(),
                        body: Some(format!(
                            "Not recorded: allowed policies are unavailable at this hour ({reason})."
                        )),
                        detail: None,
                    },
                );
            }
            let Some(place) = plan.list_rows(list_el, holder, &[size], resources)?[0] else {
                return Ok(());
            };
            plan.sub_windows.push(ComposedSubWindow {
                before_part: holder.before,
                parts: holder.place(place, &layout),
                regions: holder.place_regions(place, &layout.regions),
                diagnostics: layout.diagnostics,
                root_window: "policy_item".to_owned(),
                path_prefix: "policy:missing/".to_owned(),
                close_paths: Vec::new(),
                tooltip_overrides: overrides,
            });
        }
        RecordedDetails::Available(details) => {
            let mut undecided: HashMap<String, String> = HashMap::new();
            let mut entries: Vec<Spelled> = details
                .allowed_policies
                .iter()
                .filter(|entry| {
                    let Some(token) = entry.text() else { return true };
                    if details.policy.as_ref() == Some(*entry) {
                        return true;
                    }
                    let Ok(policy) = org_db.policies.policy(token.as_bytes()) else { return true };
                    match loc.organisation_conditions(
                        prepared.data.organisation_id,
                        &[(OrganisationCondition::Visible, policy.visible.as_slice())],
                    ) {
                        Ok(visible) => visible,
                        Err(why) => {
                            undecided.insert(
                                token.to_ascii_lowercase(),
                                format!("Not decided: whether this policy is visible cannot be evaluated at this hour ({why})."),
                            );
                            true
                        }
                    }
                })
                .cloned()
                .collect();
            if let Some(attached) = &details.policy
                && !entries.contains(attached)
            {
                entries.insert(0, attached.clone());
            }
            let quote = loc.policy_cost(&prepared.catalog, &org_db.policies);
            let places = plan.list_rows(list_el, holder, &vec![size; entries.len()], resources)?;
            for (idx, (entry, place)) in entries.iter().zip(places).enumerate() {
                let Some(place) = place else { continue };
                let is_attached = details.policy.as_ref() == Some(entry);
                let Spelled::Text(token) = entry else {
                    let reason = entry.missing("allowed policy").unwrap_or_default();
                    let layout = layout_window(item_tpl, resources, viewport, |instance| {
                        hide_by_name(instance, item_tpl, "ai_value");
                        hide_by_name(instance, item_tpl, "icon");
                        set_frame_by_name(
                            instance,
                            item_tpl,
                            "background_button",
                            if is_attached { 2 } else { 1 },
                        );
                        disable_by_name(instance, item_tpl, "background_button");
                        set_text_by_name(instance, item_tpl, "name", NOT_RECORDED_VALUE.to_owned());
                        set_text_by_name(
                            instance,
                            item_tpl,
                            "attach_cost",
                            NOT_RECORDED_VALUE.to_owned(),
                        );
                    })?;
                    let mut overrides = BTreeMap::new();
                    if let Some(btn) = find_direct(item_tpl, "background_button") {
                        overrides.insert(
                            btn.path.clone(),
                            GameWindowTooltip {
                                title: "Not recorded".to_owned(),
                                body: Some(reason),
                                detail: None,
                            },
                        );
                    }
                    plan.sub_windows.push(ComposedSubWindow {
                        before_part: holder.before,
                        parts: holder.place(place, &layout),
                        regions: holder.place_regions(place, &layout.regions),
                        diagnostics: layout.diagnostics,
                        root_window: "policy_item".to_owned(),
                        path_prefix: format!("policy:{idx}/"),
                        close_paths: Vec::new(),
                        tooltip_overrides: overrides,
                    });
                    continue;
                };
                let key = token.to_ascii_lowercase();
                let policy_def = org_db.policies.policy(token.as_bytes()).ok();
                let name = resolve_policy_name(token, policy_def, loc);
                let icon = resolve_policy_icon(policy_def, has_sprite);
                let base = match &prepared.data.policy_costs {
                    Ok(costs) => costs.get(&key).and_then(|summary| summary.cost).ok_or_else(|| {
                        "Not recorded: this policy's base cost is not in its owner's recorded policy roster at this hour.".to_owned()
                    }),
                    Err(reason) => Err(format!(
                        "Not recorded: the owner's policy roster is unavailable at this hour ({reason})."
                    )),
                };
                let quoted = base.and_then(|base| {
                    quote.as_ref().map(|cost| cost.quote(base)).map_err(|why| {
                        format!("Not recorded: the attach cost cannot be quoted at this hour ({why}).")
                    })
                });
                let (cost_str, cost_missing, cost_shown) = match quoted {
                    Ok(quoted) => (
                        quoted.to_string(),
                        None,
                        organisation_quote::shows_attach_cost(quoted, is_attached),
                    ),
                    Err(reason) => (NOT_RECORDED_VALUE.to_owned(), Some(reason), true),
                };
                let layout = layout_window(item_tpl, resources, viewport, |instance| {
                    hide_by_name(instance, item_tpl, "ai_value");
                    set_frame_by_name(
                        instance,
                        item_tpl,
                        "background_button",
                        if is_attached { 2 } else { 1 },
                    );
                    disable_by_name(instance, item_tpl, "background_button");
                    set_sprite_by_name(instance, item_tpl, "icon", &icon);
                    set_text_by_name(instance, item_tpl, "name", name.clone());
                    if cost_shown {
                        set_text_by_name(instance, item_tpl, "attach_cost", cost_str.clone());
                    } else {
                        hide_by_name(instance, item_tpl, "attach_cost");
                    }
                })?;
                let mut overrides = BTreeMap::new();
                if let Some(btn) = find_direct(item_tpl, "background_button") {
                    let mut lines = Vec::new();
                    lines.extend(undecided.get(&key).cloned());
                    lines.extend(cost_missing);
                    if let Some(def) = policy_def {
                        lines.extend(policy_lines(def, loc));
                    }
                    overrides.insert(
                        btn.path.clone(),
                        GameWindowTooltip {
                            title: name,
                            body: (!lines.is_empty()).then(|| lines.join("\n")),
                            detail: Some(token.clone()),
                        },
                    );
                }
                plan.sub_windows.push(ComposedSubWindow {
                    before_part: holder.before,
                    parts: holder.place(place, &layout),
                    regions: holder.place_regions(place, &layout.regions),
                    diagnostics: layout.diagnostics,
                    root_window: "policy_item".to_owned(),
                    path_prefix: format!("policy:{idx}/"),
                    close_paths: Vec::new(),
                    tooltip_overrides: overrides,
                });
            }
        }
    }
    Ok(())
}

pub fn render_data(
    service: &Service,
    layers: &ResolvedLayers,
    key: WindowCacheKey,
    data: OrganisationViewData,
    text: (&DecodedFrame, &Result<Arc<ScriptedContent>, String>),
    sub_view: SubView,
    scale: f32,
    scroll: &BTreeMap<String, f32>,
    admitted_gen: Option<(u64, u64)>,
    cancel: &AtomicBool,
) -> Result<GameWindowRender, GameWindowError> {
    let (frame, scripted) = text;
    let window_name = sub_view.window_name();
    let catalog = trait_catalog(layers);
    let org_db = cached_organisation_database(layers);
    let named = subject(&data);
    match sub_view {
        SubView::Traits => {
            let tree = organisation_tree(&catalog, &data.token)
                .map_err(|reason| tab_refusal(&named, sub_view, &reason))?;
            trait_positions(&catalog, &tree, data.token.as_deref().unwrap_or_default())
                .map_err(|reason| tab_refusal(&named, sub_view, &reason))?;
        }
        SubView::Policy => {
            org_db.as_ref().map_err(|why| {
                tab_refusal(
                    &named,
                    sub_view,
                    &format!("the organisation scripts could not be read ({why})"),
                )
            })?;
        }
        SubView::History => {}
    }
    let read_hour = Some(data.read_hour);
    let extra_warnings = data.warnings.clone();
    let failures = RefCell::new(Vec::new());

    service.render_composed_from_layers(
        key,
        layers,
        RenderParams {
            window_name,
            scale,
            read_hour,
            art_from_current_install: true,
            close_paths: &["close_button"],
            tooltip_overrides: &BTreeMap::new(),
            extra_warnings,
            admitted_gen,
            scroll,
        },
        cancel,
        |shared| {
            let mut assets = Assets::default();
            let templates: &[&str] = match sub_view {
                SubView::Traits => &[
                    "progressbar_widget",
                    "industrial_organisation_tree_window",
                    "industrial_organisation_flavortext_window",
                    "industrial_organisation_trait_entry",
                    "industrial_organisation_bonus_type_icon",
                    "industrial_organisation_detail_separator",
                    "industrial_organisation_detail_modifier_item",
                    "industrial_organisation_detail_bonus_item",
                ],
                SubView::History => &[
                    "progressbar_widget",
                    "industrial_organisation_detail_separator",
                    "industrial_organisation_detail_modifier_item",
                    "industrial_organisation_detail_bonus_item",
                    "industrial_organisation_history_item",
                ],
                SubView::Policy => &["policy_item"],
            };
            collect_template_assets(
                shared.guis.as_ref(),
                shared.localisation.as_ref(),
                templates,
                &mut assets,
            );
            let tree = organisation_tree(&catalog, &data.token).ok();
            tab_requirements(shared.guis.as_ref(), sub_view, tree.as_ref())?;
            let has_sprite = |name: &str| shared.registry.sprite(name.as_bytes()).is_some();
            let emblem_sprite = resolve_org_icon(&data, tree.as_ref(), has_sprite);
            assets.sprites.insert(emblem_sprite.as_bytes().to_vec());

            let tree_background = tree
                .as_ref()
                .and_then(|t| t.background)
                .filter(|bg| !bg.is_empty() && has_sprite(bg))
                .map(str::to_owned);
            if let Some(bg) = &tree_background {
                assets.sprites.insert(bg.as_bytes().to_vec());
            }

            let attached_policy_sprite = match &data.details {
                RecordedDetails::Available(details) => details.policy.as_ref().and_then(Spelled::text).map(|tok| {
                    let def = org_db
                        .as_deref()
                        .ok()
                        .and_then(|db| db.policies.policy(tok.as_bytes()).ok());
                    resolve_policy_icon(def, has_sprite)
                }),
                RecordedDetails::Unavailable(_) => None,
            };
            if let Some(sprite) = &attached_policy_sprite {
                assets.sprites.insert(sprite.as_bytes().to_vec());
            }

            if sub_view == SubView::Traits {
                for sprite in [
                    "GFX_industrial_org_trait_button",
                    "GFX_country_spefific_org_trait_button",
                    "GFX_design_team_icon",
                    "GFX_industrial_manufacturer_icon",
                    "GFX_organization_modifier_icon",
                    "GFX_focus_link_up_down",
                    "GFX_focus_link_left_right",
                    "GFX_focus_exclusive_line1",
                    "GFX_focus_link_exclusive",
                    UNKNOWN_ICON_SPRITE,
                ] {
                    if has_sprite(sprite) {
                        assets.sprites.insert(sprite.as_bytes().to_vec());
                    }
                }
                if let Some(tree) = tree {
                    for trait_def in tree.traits {
                        if !trait_def.icon.is_empty() && has_sprite(&trait_def.icon) {
                            assets.sprites.insert(trait_def.icon.as_bytes().to_vec());
                        }
                    }
                }
            } else if sub_view == SubView::Policy {
                assets
                    .sprites
                    .insert(UNKNOWN_ICON_SPRITE.as_bytes().to_vec());
                if let RecordedDetails::Available(details) = &data.details {
                    for tok in details.allowed_policies.iter().filter_map(Spelled::text) {
                        let def = org_db
                            .as_deref()
                            .ok()
                            .and_then(|db| db.policies.policy(tok.as_bytes()).ok());
                        let sprite = resolve_policy_icon(def, has_sprite);
                        assets.sprites.insert(sprite.as_bytes().to_vec());
                    }
                }
            }

            Ok(Prepared::uncached(
                assets,
                PreparedMio {
                    data,
                    sub_view,
                    catalog,
                    org_db,
                    emblem_sprite,
                    tree_background,
                    attached_policy_sprite,
                },
            ))
        },
        |root, _, _, localisation, instance, tooltips, prepared| {
            let loc = Loc::new(localisation, frame, scripted, &prepared.data, &failures);
            match prepared.sub_view {
                SubView::Traits | SubView::History => {
                    configure_detail_main(root, &loc, instance, tooltips, prepared);
                }
                SubView::Policy => {
                    configure_policy_main(root, &loc, instance);
                }
            }
        },
        |plan, main_layout, root, guis, resources, localisation, prepared| {
            let loc = Loc::new(localisation, frame, scripted, &prepared.data, &failures);
            match prepared.sub_view {
                SubView::Traits => {
                    compose_funds_progressbar(
                        main_layout,
                        root,
                        guis,
                        resources,
                        &loc,
                        prepared,
                        plan,
                    )?;
                    compose_aggregated_bonuses(
                        main_layout,
                        root,
                        guis,
                        resources,
                        &loc,
                        prepared,
                        plan,
                    )?;
                    compose_trait_tree(
                        main_layout,
                        root,
                        guis,
                        resources,
                        &loc,
                        prepared,
                        plan,
                    )?;
                }
                SubView::History => {
                    compose_funds_progressbar(
                        main_layout,
                        root,
                        guis,
                        resources,
                        &loc,
                        prepared,
                        plan,
                    )?;
                    compose_aggregated_bonuses(
                        main_layout,
                        root,
                        guis,
                        resources,
                        &loc,
                        prepared,
                        plan,
                    )?;
                    compose_history_lists(
                        main_layout,
                        root,
                        guis,
                        resources,
                        &loc,
                        prepared,
                        plan,
                    )?;
                }
                SubView::Policy => {
                    compose_policy_list(
                        main_layout,
                        root,
                        guis,
                        resources,
                        &loc,
                        prepared,
                        plan,
                    )?;
                }
            }
            for (path, view) in [
                ("tabs/traits_tab", "traits"),
                ("tabs/history_tab", "history"),
                ("open_policies_window/open_policies", "policy"),
            ] {
                if prepared.sub_view != SubView::Policy
                    && let Some(element) = find_direct(root, path)
                {
                    plan.view_links.insert(element.path.clone(), view.to_owned());
                }
            }
            plan.title = Some(loc.organisation_name());
            plan.warnings.extend(failures.borrow_mut().drain(..));
            Ok(())
        },
    )
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
    let sub_view = SubView::from_request(request.sub_view.as_deref())?;
    let target: Target = serde_json::from_value(request.target.clone()).map_err(|error| {
        GameWindowError::refused(format!(
            "Invalid military industrial organisation target: {error}"
        ))
    })?;
    let org_id: u64 = target.organisation.parse().map_err(|_| {
        GameWindowError::refused(format!(
            "Invalid military industrial organisation id '{}'.",
            target.organisation
        ))
    })?;
    let branch_id: u64 = request.branch_id.parse().map_err(|_| {
        GameWindowError::refused(format!("Invalid branch id '{}'.", request.branch_id))
    })?;

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

    let org_row = witnessed_single_key::<tables::IndustrialOrganisations, _>(&frame, org_id, |row| {
        row.organisation
    })
    .map_err(|error| {
        GameWindowError::refused(format!(
            "Military industrial organisations are not recorded at this hour: {error}"
        ))
    })?
    .ok_or_else(|| {
        GameWindowError::refused(format!(
            "Military industrial organisation {} is not recorded at hour {}.",
            target.organisation, request.hour
        ))
    })?;
    let lexical = witnessed_rows::<tables::LexicalTokens>(&frame).map(|(rows, _)| rows);
    let token = match spell_token(&lexical, org_row.token) {
        Spelled::Text(token) => Ok(token),
        Spelled::Unspelled { reason, .. } => Err(reason),
    };
    let mut read_tables = vec![TableId::IndustrialOrganisations];
    if lexical.is_ok() {
        read_tables.push(TableId::LexicalTokens);
    }
    let owner_tag = match (
        frame.complete::<tables::Countries>(),
        frame.complete::<tables::CountryLinks>(),
    ) {
        (Ok(countries), Ok(links)) => {
            read_tables.extend([TableId::Countries, TableId::CountryLinks]);
            let owner = resolve_owner_tag(org_row, countries, links);
            if owner.is_none() {
                extra_warnings.push(
                    "The organisation's owner has no recorded country at this hour: country-specific names and art are not used.".to_owned(),
                );
            }
            owner
        }
        (countries, links) => {
            let why = countries.err().or(links.err()).map(|error| error.to_string()).unwrap_or_default();
            extra_warnings.push(format!(
                "The organisation's owner is unknown because countries are not recorded at this hour ({why}): country-specific names and art are not used."
            ));
            None
        }
    };

    let details = match witnessed_single_key::<tables::IndustrialOrganisationDetails, _>(&frame, org_id, |row| row.organisation) {
        Ok(Some(row)) => {
            read_tables.push(TableId::IndustrialOrganisationDetails);
            RecordedDetails::Available(Box::new(extract_details(row, &lexical)))
        }
        Ok(None) => {
            read_tables.push(TableId::IndustrialOrganisationDetails);
            RecordedDetails::Unavailable(
                "no organisation details row was recorded for this organisation at this hour".to_owned(),
            )
        }
        Err(error) => RecordedDetails::Unavailable(error),
    };

    let policy_costs = owner_policy_roster(&frame, org_row, lexical.as_deref());
    if policy_costs.is_ok() {
        read_tables.push(TableId::IndustrialOrganisationPolicies);
    }

    let (history, history_reason) = match frame.complete::<tables::IndustrialOrganisationHistory>() {
        Ok(rows) if sub_view == SubView::History => {
            read_tables.push(TableId::IndustrialOrganisationHistory);
            let variants = frame
                .complete::<tables::VariantDefinitions>()
                .map_err(|error| format!("VariantDefinitions is unavailable: {error}"));
            if variants.is_ok() {
                read_tables.push(TableId::VariantDefinitions);
            }
            (Some(extract_history(rows, org_id, &variants, &lexical)), None)
        }
        Ok(_) => (None, None),
        Err(error) => (None, Some(error.to_string())),
    };

    let samples: Vec<(TableId, i32)> = read_tables
        .iter()
        .filter_map(|&id| frame.tables[id.index()].sampled_at().map(|hour| (id, hour)))
        .collect();
    let (read_hour, cadence_warning) = compute_read_hour(request.hour, &samples);
    if let Some(warning) = cadence_warning {
        extra_warnings.push(warning);
    }

    let funds_defines = read_funds_defines(layers);
    let data = OrganisationViewData {
        organisation_id: org_id,
        request_hour: request.hour,
        read_hour,
        owner_tag,
        token,
        custom_name: String::from_utf8_lossy(&org_row.name).trim().to_owned(),
        recorded_icon: String::from_utf8_lossy(&org_row.icon).trim().to_owned(),
        details,
        policy_costs,
        history,
        history_reason,
        funds_defines,
        warnings: extra_warnings,
    };

    let key = WindowCacheKey {
        source_id: request.source_id,
        revision: request.revision,
        game_dir: resolve.game_dir.clone(),
        user_dir: resolve.user_dir.clone(),
        visual_revision: visual_revision.to_owned(),
        language: language.to_owned(),
        force_pow2_textures: decode.force_pow2_textures,
        art_stack: layers.art_stack_key(),
        window: sub_view.window_name().to_owned(),
    };
    let scripted = cached_scripted_content(layers, content, language);
    render_data(
        service,
        layers,
        key,
        data,
        (&frame, &scripted),
        sub_view,
        request.scale,
        &request.scroll,
        Some(admitted_gen),
        cancel,
    )
}
