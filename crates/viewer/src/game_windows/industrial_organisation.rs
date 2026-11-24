use super::{
    ComposedPlan, ComposedSubWindow, DEFAULT_VIEWPORT, GameWindowError, GameWindowRender,
    GameWindowRequest, GameWindowTooltip, Prepared, RenderParams, Service, WindowCacheKey,
    container_holder,
    equipment_designer::{RenderContext, compute_read_hour, witnessed_rows, witnessed_single_key},
    layout_window, missing_window_refusal, strip_markup,
};
use content_reader_defines::defines;
use content_reader_gui::gui::native::{self, Element, Fields};
use content_reader_layers::ResolvedLayers;
use content_reader_localisation::localisation::Localisation;
use content_reader_military::{
    industrial_organisations::{self, Database as OrganisationDatabase, policies::Policy},
    mio_traits::{self, Bonus, TraitName, Tree, is_production_stat},
};
use content_reader_script::equipment_stats;
use dashboard_text::metadata::link_slot;
use replay::campaign::SeekResult;
use replay_data::rows::{
    IndustrialOrganisationDetailsRow, IndustrialOrganisationHistoryRow,
    IndustrialOrganisationPoliciesRow, IndustrialOrganisationRow, LexicalTokenRow, Recorded,
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
use viewer_native_gui::native_text::COLOR;

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
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubView {
    Traits,
    History,
    Policy,
}

impl SubView {
    fn from_request(kind: &str, sub_view: Option<&str>) -> Result<Self, GameWindowError> {
        if kind == "mioPolicyChooser" {
            return Ok(Self::Policy);
        }
        match sub_view.unwrap_or("traits") {
            "traits" | "" => Ok(Self::Traits),
            "history" => Ok(Self::History),
            "policy" | "policies" => Ok(Self::Policy),
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
pub struct HistoryEntry {
    pub variant_id: u64,
    pub name: String,
    pub archetype: String,
    pub units: i32,
    pub design_hour: Option<i32>,
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
    pub policy_token: Option<String>,
    pub cooldown_hour: Option<i32>,
    pub unlocked: Vec<String>,
    pub queued: Vec<String>,
    pub allowed_policies: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct OrganisationViewData {
    pub organisation_id: u64,
    pub request_hour: i32,
    pub read_hour: i32,
    pub owner_tag: Option<String>,
    pub token: String,
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

fn cached_organisation_database(
    layers: &ResolvedLayers,
) -> Result<Arc<OrganisationDatabase>, String> {
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

fn spell(lexical: &[LexicalTokenRow], token: u32) -> Option<String> {
    let idx = lexical.binary_search_by_key(&token, |row| row.token).ok()?;
    std::str::from_utf8(&lexical[idx].literal)
        .ok()
        .map(str::to_owned)
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

fn extract_details(
    row: &IndustrialOrganisationDetailsRow,
    lexical: &[LexicalTokenRow],
    warnings: &mut Vec<String>,
) -> OrganisationDetails {
    let mut spell_list = |what: &str, tokens: &[u32]| -> Vec<String> {
        let mut spelled = Vec::with_capacity(tokens.len());
        for &token in tokens {
            match spell(lexical, token) {
                Some(text) => spelled.push(text),
                None => warnings.push(format!(
                    "Not recorded: {what} token {token} is not spelled in LexicalTokens; it is left out of the window."
                )),
            }
        }
        spelled
    };
    let policy_token = if row.state.policy == UNSELECTED_POLICY {
        None
    } else {
        let spelled = spell(lexical, row.state.policy);
        if spelled.is_none() {
            warnings.push(format!(
                "Not recorded: attached policy token {} is not spelled in LexicalTokens.",
                row.state.policy
            ));
        }
        spelled
    };
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
        policy_token,
        cooldown_hour: row.state.cooldown_hour,
        unlocked: spell_list("unlocked trait", &row.unlocked),
        queued: spell_list("queued trait", &row.queued),
        allowed_policies: spell_list("allowed policy", &row.allowed_policies),
    }
}

fn extract_policy_costs(
    rows: &[IndustrialOrganisationPoliciesRow],
    country_raw: u32,
    links: &[replay_data::rows::CountryLinkRow],
    lexical: &[LexicalTokenRow],
) -> BTreeMap<String, PolicySummary> {
    let target_slot = link_slot(links, country_raw);
    let mut out = BTreeMap::new();
    let Some(entry) = rows.iter().find(|row| {
        row.country == country_raw
            || (target_slot.is_some() && link_slot(links, row.country) == target_slot)
    }) else {
        return out;
    };
    for policy in &entry.policies {
        let Some(token) = spell(lexical, policy.token) else {
            continue;
        };
        let key = token.to_ascii_lowercase();
        out.insert(
            key,
            PolicySummary {
                token,
                cost: Some(policy.cost),
                cooldown_days: Some(policy.days),
            },
        );
    }
    out
}

fn extract_history(
    rows: &[IndustrialOrganisationHistoryRow],
    org_id: u64,
    variants: Option<&[VariantDefinitionRow]>,
    lexical: &[LexicalTokenRow],
) -> Vec<HistoryEntry> {
    let mut out = Vec::new();
    for row in rows.iter().filter(|row| row.organisation == org_id) {
        let (name, archetype) = variants
            .and_then(|defs| {
                defs.binary_search_by_key(&row.variant, |v| v.id)
                    .ok()
                    .map(|idx| &defs[idx])
            })
            .map(|def| {
                let raw_name = String::from_utf8_lossy(&def.name).trim().to_owned();
                let eq = spell(lexical, def.equipment_token).unwrap_or_default();
                let display = if raw_name.is_empty() { eq.clone() } else { raw_name };
                (display, eq)
            })
            .unwrap_or_else(|| (NOT_RECORDED_VALUE.to_owned(), String::new()));
        let design_hour = match row.design_hour {
            Recorded::Value(hour) => hour,
            Recorded::NotRecorded => None,
        };
        out.push(HistoryEntry {
            variant_id: row.variant,
            name,
            archetype,
            units: row.units,
            design_hour,
        });
    }
    out
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
        let Some(end) = after.find('$') else {
            out.push('$');
            out.push_str(after);
            return out;
        };
        let token = &after[..end];
        let key = token.split('|').next().unwrap_or(token);
        if !key.is_empty()
            && key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-')
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
    }
    out.push_str(rest);
    out
}

fn loc_or(localisation: &Localisation, key: &str) -> String {
    localisation
        .get(key)
        .map(|raw| expand_loc_refs(localisation, &strip_markup(raw), 4))
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| key.to_owned())
}

fn loc_for_owner(localisation: &Localisation, owner_tag: Option<&str>, key: &str) -> Option<String> {
    if let Some(tag) = owner_tag {
        let scoped = format!("{tag}_{key}");
        if let Some(raw) = localisation.get(&scoped) {
            let expanded = expand_loc_refs(localisation, &strip_markup(raw), 4);
            if !expanded.is_empty() {
                return Some(expanded);
            }
        }
    }
    localisation.get(key).and_then(|raw| {
        let expanded = expand_loc_refs(localisation, &strip_markup(raw), 4);
        (!expanded.is_empty()).then_some(expanded)
    })
}

fn resolve_org_name(
    data: &OrganisationViewData,
    org_db: Option<&OrganisationDatabase>,
    localisation: &Localisation,
) -> String {
    if !data.custom_name.is_empty() {
        if let Some(loc) = loc_for_owner(localisation, data.owner_tag.as_deref(), &data.custom_name)
        {
            return loc;
        }
        return data.custom_name.clone();
    }
    if let Some(db) = org_db
        && let Ok((template_name, _)) = db.display_fields(data.token.as_bytes())
        && !template_name.is_empty()
    {
        let key = String::from_utf8_lossy(&template_name);
        if let Some(loc) = loc_for_owner(localisation, data.owner_tag.as_deref(), &key) {
            return loc;
        }
    }
    loc_for_owner(localisation, data.owner_tag.as_deref(), &data.token)
        .unwrap_or_else(|| data.token.clone())
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
    if let Some(tag) = &data.owner_tag {
        candidates.push(format!("GFX_{tag}_{}", data.token));
    }
    candidates.push(format!("GFX_{}", data.token));
    candidates.push(UNKNOWN_ICON_SPRITE.to_owned());
    for candidate in candidates {
        if has_sprite(&candidate) {
            return candidate;
        }
    }
    UNKNOWN_ICON_SPRITE.to_owned()
}

fn resolve_policy_name(
    token: &str,
    policy: Option<&Policy>,
    localisation: &Localisation,
) -> String {
    if let Some(def) = policy
        && !def.name.is_empty()
    {
        let key = String::from_utf8_lossy(&def.name);
        if let Some(text) = loc_for_owner(localisation, None, &key) {
            return text;
        }
    }
    loc_or(localisation, token)
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
    localisation: &Localisation,
) -> String {
    for candidate in trait_def.name_candidates(org_token, &trait_def.token, owner_tag) {
        if let Some(raw) = localisation.get(&candidate) {
            let expanded = expand_loc_refs(localisation, &strip_markup(raw), 4);
            if !expanded.is_empty() {
                return expanded;
            }
        }
    }
    if !trait_def.name.is_empty() {
        trait_def.name.clone()
    } else {
        trait_def.token.clone()
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

fn format_signed_value_fixed5(raw: i64) -> String {
    let val = raw as f64 / FIXED5_SCALE;
    if (val.fract()).abs() < 0.005 {
        format!("{:+.0}", val)
    } else {
        format!("{:+.2}", val)
    }
}

fn stat_is_percent(name: &str) -> bool {
    if is_production_stat(name) || name.ends_with("_factor") || name.ends_with("_bonus") {
        return true;
    }
    equipment_stats::stat_metadata()
        .iter()
        .find(|(keyword, _, _)| *keyword == name)
        .is_some_and(|(_, _, is_pct)| *is_pct)
}

fn stat_label(name: &str, localisation: &Localisation) -> String {
    if let Some((_, loc_key, _)) = equipment_stats::stat_metadata()
        .iter()
        .find(|(keyword, _, _)| *keyword == name)
        && let Some(raw) = localisation.get(loc_key)
    {
        let clean = expand_loc_refs(localisation, &strip_markup(raw), 4);
        if !clean.is_empty() {
            return clean.trim_end_matches(':').trim().to_owned();
        }
    }
    let upper = name.to_ascii_uppercase();
    for candidate in [
        format!("MODIFIER_{upper}"),
        format!("STAT_COMMON_{upper}"),
        format!("STAT_ARMY_{upper}"),
        format!("STAT_NAVY_{upper}"),
        format!("STAT_AIR_{upper}"),
        upper,
        name.to_owned(),
    ] {
        if let Some(raw) = localisation.get(&candidate) {
            let clean = expand_loc_refs(localisation, &strip_markup(raw), 4);
            if !clean.is_empty() {
                return clean.trim_end_matches(':').trim().to_owned();
            }
        }
    }
    name.to_owned()
}

fn format_bonus_plain(name: &str, raw: i64, localisation: &Localisation) -> String {
    let value = if stat_is_percent(name) {
        format_signed_percent_fixed5(raw)
    } else {
        format_signed_value_fixed5(raw)
    };
    loc_fill(
        localisation,
        "INDUSTRIAL_ORG_DETAIL_MIO_MODIFIER_ITEM",
        &[("NAME", &stat_label(name, localisation)), ("VALUE", &value)],
    )
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

fn set_sprite_by_name(instance: &mut Instance, root: &Element, name: &str, sprite: &str) {
    if let Some(element) = find_direct(root, name)
        && let Ok(bound) = instance.control(root, element)
    {
        bound.sprite = Some(sprite.as_bytes().to_vec());
    }
}

fn set_frame_by_name(instance: &mut Instance, root: &Element, name: &str, frame: i32) {
    if let Some(element) = find_direct(root, name)
        && let Ok(bound) = instance.control(root, element)
    {
        bound.frame = Some(frame);
    }
}

fn loc_fill(localisation: &Localisation, key: &str, values: &[(&str, &str)]) -> String {
    let Some(raw) = localisation.get(key) else {
        let shown: Vec<&str> = values.iter().map(|(_, value)| *value).collect();
        return format!("{key} {}", shown.join(" "));
    };
    let text = strip_markup(raw);
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(start) = rest.find('$') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('$') else {
            out.push('$');
            out.push_str(after);
            return out;
        };
        let token = &after[..end];
        let name = token.split('|').next().unwrap_or(token);
        match values.iter().find(|(placeholder, _)| *placeholder == name) {
            Some((_, value)) => out.push_str(value),
            None => out.push_str(&expand_loc_refs(localisation, &format!("${token}$"), 4)),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

fn modifier_item(localisation: &Localisation, name_key: &str, value: &str) -> String {
    loc_fill(
        localisation,
        "INDUSTRIAL_ORG_DETAIL_MIO_MODIFIER_ITEM",
        &[("NAME", &loc_or(localisation, name_key)), ("VALUE", value)],
    )
}

fn format_points_line(localisation: &Localisation, value_str: &str) -> String {
    loc_fill(
        localisation,
        "INDUSTRIAL_ORG_DETAIL_TRAIT_POINTS",
        &[("VALUE", value_str)],
    )
}

fn collect_template_assets(
    guis: &native::Guis,
    localisation: &Localisation,
    names: &[&str],
    assets: &mut Assets,
) {
    for name in names {
        if let Some(template) = guis.template(name.as_bytes()) {
            let found = native_gui::assets_in(template, &|n| guis.template(n), localisation);
            assets.sprites.extend(found.sprites);
            assets.fonts.extend(found.fonts);
            assets.textures.extend(found.textures);
        }
    }
}

fn aggregate_bonuses(
    data: &OrganisationViewData,
    tree: Option<&Tree<'_>>,
    org_db: Option<&OrganisationDatabase>,
) -> Vec<(String, i64)> {
    let mut totals: Vec<(String, i64)> = Vec::new();
    let mut add = |bonus: &Bonus| {
        if let Some((_, existing)) = totals.iter_mut().find(|(k, _)| k == &bonus.name) {
            *existing = existing.wrapping_add(bonus.value);
        } else {
            totals.push((bonus.name.clone(), bonus.value));
        }
    };
    if let Some(tree) = tree {
        if let Some(initial) = tree.initial {
            for bonus in &initial.bonuses {
                add(bonus);
            }
        }
        if let RecordedDetails::Available(details) = &data.details {
            let unlocked: BTreeSet<String> = details
                .unlocked
                .iter()
                .map(|t| t.to_ascii_lowercase())
                .collect();
            for trait_def in tree.traits {
                if unlocked.contains(&trait_def.token.to_ascii_lowercase()) {
                    for bonus in &trait_def.bonuses {
                        add(bonus);
                    }
                }
            }
        }
    }
    if let RecordedDetails::Available(details) = &data.details
        && let Some(policy_token) = &details.policy_token
        && let Some(db) = org_db
        && let Ok(policy) = db.policies.policy(policy_token.as_bytes())
    {
        for (_, group_bonuses) in &policy.groups {
            for bonus in group_bonuses {
                add(bonus);
            }
        }
    }
    totals.retain(|(_, value)| *value != 0);
    totals
}

struct PreparedMio {
    data: OrganisationViewData,
    sub_view: SubView,
    catalog: Option<Arc<mio_traits::Catalog>>,
    org_db: Option<Arc<OrganisationDatabase>>,
    emblem_sprite: String,
    tree_background: Option<String>,
    attached_policy_sprite: Option<String>,
}

fn configure_detail_main(
    root: &Element,
    localisation: &Localisation,
    instance: &mut Instance,
    tooltips: &mut BTreeMap<String, GameWindowTooltip>,
    prepared: &PreparedMio,
) {
    let data = &prepared.data;
    let org_db = prepared.org_db.as_deref();
    let display_name = resolve_org_name(data, org_db, localisation);
    set_text_by_name(instance, root, "industrial_org_name", display_name.clone());

    let traits_frame = if prepared.sub_view == SubView::Traits { 2 } else { 1 };
    let history_frame = if prepared.sub_view == SubView::History { 2 } else { 1 };
    set_frame_by_name(instance, root, "tabs/traits_tab", traits_frame);
    set_frame_by_name(instance, root, "tabs/history_tab", history_frame);
    disable_by_name(instance, root, "tabs/traits_tab");
    disable_by_name(instance, root, "tabs/history_tab");

    set_sprite_by_name(instance, root, "top_left_window/icon", &prepared.emblem_sprite);
    if let Some(icon_el) = find_direct(root, "top_left_window/icon") {
        tooltips.insert(
            icon_el.path.clone(),
            GameWindowTooltip {
                title: display_name,
                body: Some(data.token.clone()),
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
                format_points_line(localisation, &details.points.to_string()),
            );
            if let Some(size_el) = find_direct(root, "top_left_window/size_icon_window/size_icon") {
                let progress = compute_funds_progress(
                    details.funds_raw,
                    details.size,
                    details.size_cost_factor_raw,
                    &data.funds_defines,
                );
                let body = match progress.percent {
                    Some(pct) => format!("Size: {} ({pct}%)", details.size),
                    None => format!("Size: {}", details.size),
                };
                tooltips.insert(
                    size_el.path.clone(),
                    GameWindowTooltip {
                        title: loc_or(localisation, "INDUSTRIAL_ORG_SIZE"),
                        body: Some(body),
                        detail: None,
                    },
                );
            }
            if let Some(points_el) = find_direct(root, "top_left_window/points") {
                tooltips.insert(
                    points_el.path.clone(),
                    GameWindowTooltip {
                        title: format_points_line(localisation, &details.points.to_string()),
                        body: Some(loc_or(localisation, "INDUSTRIAL_ORG_DETAIL_TRAIT_POINTS_DELAYED")),
                        detail: None,
                    },
                );
            }
            if let Some(policy_token) = &details.policy_token {
                hide_by_name(instance, root, "open_policies_window/open_policies_text");
                hide_by_name(
                    instance,
                    root,
                    "open_policies_window/click_to_open_policies_text",
                );
                let policy_def = org_db.and_then(|db| db.policies.policy(policy_token.as_bytes()).ok());
                let policy_name = resolve_policy_name(policy_token, policy_def, localisation);
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
                        for (_, group_bonuses) in &def.groups {
                            for bonus in group_bonuses {
                                lines.push(format_bonus_plain(
                                    &bonus.name,
                                    bonus.value,
                                    localisation,
                                ));
                            }
                        }
                        for modifier in def.organisation_modifiers.listed() {
                            lines.push(format_bonus_plain(
                                &modifier.name,
                                modifier.value,
                                localisation,
                            ));
                        }
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
                set_text_by_name(
                    instance,
                    root,
                    "open_policies_window/open_policies_text",
                    loc_or(localisation, "INDUSTRIAL_ORG_POLICY_BUTTON"),
                );
                set_text_by_name(
                    instance,
                    root,
                    "open_policies_window/click_to_open_policies_text",
                    loc_or(localisation, "NONE"),
                );
                if let Some(btn) = find_direct(root, "open_policies_window/open_policies") {
                    tooltips.insert(
                        btn.path.clone(),
                        GameWindowTooltip {
                            title: loc_or(localisation, "INDUSTRIAL_ORG_POLICY_BUTTON"),
                            body: Some("No policy attached at this hour.".to_owned()),
                            detail: None,
                        },
                    );
                }
            }

            if let Some(queue_btn) =
                find_direct(root, "aggregated_bonuses_window/open_queue_button")
            {
                let tree = prepared
                    .catalog
                    .as_deref()
                    .and_then(|cat| cat.tree(&data.token).ok().flatten());
                let body = if details.queued.is_empty() {
                    "No traits queued at this hour.".to_owned()
                } else {
                    details
                        .queued
                        .iter()
                        .enumerate()
                        .map(|(idx, tok)| {
                            let name = tree
                                .and_then(|t| {
                                    t.traits
                                        .iter()
                                        .find(|tr| tr.token.eq_ignore_ascii_case(tok))
                                })
                                .map(|tr| {
                                    resolve_trait_name(
                                        tr,
                                        &data.token,
                                        data.owner_tag.as_deref(),
                                        localisation,
                                    )
                                })
                                .unwrap_or_else(|| tok.clone());
                            format!("{}. {name}", idx + 1)
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                tooltips.insert(
                    queue_btn.path.clone(),
                    GameWindowTooltip {
                        title: loc_or(localisation, "INDUSTRIAL_ORG_DETAIL_WINDOW_OPEN_QUEUE_BUTTON"),
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
                format_points_line(localisation, NOT_RECORDED_VALUE),
            );
            hide_by_name(instance, root, "open_policies_window/attached_policy_icon");
            hide_by_name(instance, root, "open_policies_window/attached_policy_name");
            set_text_by_name(
                instance,
                root,
                "open_policies_window/open_policies_text",
                loc_or(localisation, "INDUSTRIAL_ORG_POLICY_BUTTON"),
            );
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
                "toggle_auto_designs",
            ] {
                if let Some(el) = find_direct(root, path) {
                    tooltips.insert(el.path.clone(), missing_tip.clone());
                }
            }
            set_frame_by_name(instance, root, "toggle_auto_designs", 1);
        }
    }

    disable_by_name(instance, root, "aggregated_bonuses_window/open_queue_button");
    disable_by_name(instance, root, "open_policies_window/open_policies");
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
    localisation: &Localisation,
    instance: &mut Instance,
    prepared: &PreparedMio,
) {
    let display_name = resolve_org_name(&prepared.data, prepared.org_db.as_deref(), localisation);
    set_text_by_name(instance, root, "title/title", display_name);
    set_text_by_name(
        instance,
        root,
        "title/instruction",
        loc_or(localisation, "INDUSTRIAL_ORG_POLICY_BUTTON"),
    );
}

fn compose_funds_progressbar(
    main_layout: &Layout,
    root: &Element,
    guis: &native::Guis,
    resources: &dyn Resources,
    localisation: &Localisation,
    prepared: &PreparedMio,
    plan: &mut ComposedPlan,
) -> Result<(), GameWindowError> {
    let Some(pb_root) = guis.template(b"progressbar_widget") else {
        return Ok(());
    };
    let Some(top_left) = find_direct(root, "top_left_window") else {
        return Ok(());
    };
    let Some(holder) = container_holder(main_layout, &top_left.path) else {
        return Ok(());
    };
    let pos = find_direct(top_left, "funds_progressbar_position")
        .and_then(|el| match &el.fields {
            Fields::Position(p) => Some([p.position.x.value, p.position.y.value]),
            _ => None,
        })
        .unwrap_or([120, 55]);

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
                    title: loc_or(localisation, "INDUSTRIAL_ORG_DETAIL_TOOLTIP_FUNDS_HEADER"),
                    body: Some(body),
                    detail: None,
                },
            )
        }
        RecordedDetails::Unavailable(reason) => (
            0,
            GameWindowTooltip {
                title: loc_or(localisation, "INDUSTRIAL_ORG_DETAIL_TOOLTIP_FUNDS_HEADER"),
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

fn compose_aggregated_bonuses(
    main_layout: &Layout,
    root: &Element,
    guis: &native::Guis,
    resources: &dyn Resources,
    localisation: &Localisation,
    prepared: &PreparedMio,
    plan: &mut ComposedPlan,
) -> Result<(), GameWindowError> {
    let Some(list_el) = find_direct(root, "aggregated_bonuses_window/aggregated_bonus_list") else {
        return Ok(());
    };
    let Some(holder) = main_layout
        .holders
        .iter()
        .find(|h| h.path == list_el.path)
    else {
        return Ok(());
    };
    let sep_tpl = guis.template(b"industrial_organisation_detail_separator");
    let mod_tpl = guis.template(b"industrial_organisation_detail_modifier_item");
    let bonus_tpl = guis.template(b"industrial_organisation_detail_bonus_item");
    let viewport = DEFAULT_VIEWPORT.map(|value| value as i32);
    let max_y = holder.content[1].max(0);
    let mut cursor_y = 0i32;
    let mut row_idx = 0usize;

    let mut push_separator = |title: String, plan: &mut ComposedPlan| -> Result<(), GameWindowError> {
        let Some(tpl) = sep_tpl else {
            return Ok(());
        };
        if cursor_y + 30 > max_y {
            return Ok(());
        }
        let layout = layout_window(tpl, resources, viewport, |instance| {
            set_text_by_name(instance, tpl, "separator_name", title);
        })?;
        plan.sub_windows.push(ComposedSubWindow {
            before_part: holder.before,
            parts: holder.place([0, cursor_y], &layout),
            regions: Vec::new(),
            diagnostics: layout.diagnostics,
            root_window: "industrial_organisation_detail_separator".to_owned(),
            path_prefix: format!("bonus_sep:{row_idx}/"),
            close_paths: Vec::new(),
            tooltip_overrides: BTreeMap::new(),
        });
        cursor_y += 32;
        row_idx += 1;
        Ok(())
    };

    let mut push_modifier = |text: String,
                             tooltip: Option<GameWindowTooltip>,
                             plan: &mut ComposedPlan|
     -> Result<(), GameWindowError> {
        let Some(tpl) = mod_tpl else {
            return Ok(());
        };
        if cursor_y + 20 > max_y {
            return Ok(());
        }
        let layout = layout_window(tpl, resources, viewport, |instance| {
            set_text_by_name(instance, tpl, "modifier_text", text);
        })?;
        let mut overrides = BTreeMap::new();
        if let Some(tip) = tooltip {
            overrides.insert(tpl.path.clone(), tip);
        }
        plan.sub_windows.push(ComposedSubWindow {
            before_part: holder.before,
            parts: holder.place([0, cursor_y], &layout),
            regions: holder.place_regions([0, cursor_y], &layout.regions),
            diagnostics: layout.diagnostics,
            root_window: "industrial_organisation_detail_modifier_item".to_owned(),
            path_prefix: format!("bonus_mod:{row_idx}/"),
            close_paths: Vec::new(),
            tooltip_overrides: overrides,
        });
        cursor_y += 22;
        row_idx += 1;
        Ok(())
    };

    let mut push_bonus = |text: String, plan: &mut ComposedPlan| -> Result<(), GameWindowError> {
        let Some(tpl) = bonus_tpl else {
            return Ok(());
        };
        if cursor_y + 20 > max_y {
            return Ok(());
        }
        let layout = layout_window(tpl, resources, viewport, |instance| {
            set_text_by_name(instance, tpl, "bonus_text", text);
        })?;
        plan.sub_windows.push(ComposedSubWindow {
            before_part: holder.before,
            parts: holder.place([0, cursor_y], &layout),
            regions: Vec::new(),
            diagnostics: layout.diagnostics,
            root_window: "industrial_organisation_detail_bonus_item".to_owned(),
            path_prefix: format!("bonus_val:{row_idx}/"),
            close_paths: Vec::new(),
            tooltip_overrides: BTreeMap::new(),
        });
        cursor_y += 22;
        row_idx += 1;
        Ok(())
    };

    match &prepared.data.details {
        RecordedDetails::Unavailable(reason) => {
            push_separator(
                loc_or(localisation, "INDUSTRIAL_ORG_DETAIL_MIO_MODIFIERS"),
                plan,
            )?;
            push_modifier(
                NOT_RECORDED_VALUE.to_owned(),
                Some(GameWindowTooltip {
                    title: "Not recorded".to_owned(),
                    body: Some(format!(
                        "Not recorded: organisation modifiers and unlocked traits are unavailable at this hour ({reason})."
                    )),
                    detail: None,
                }),
                plan,
            )?;
        }
        RecordedDetails::Available(details) => {
            push_separator(
                loc_or(localisation, "INDUSTRIAL_ORG_DETAIL_MIO_MODIFIERS"),
                plan,
            )?;
            push_modifier(
                modifier_item(
                    localisation,
                    "MODIFIER_MIO_RESEARCH_BONUS_MIO_CONTEXT",
                    &format_signed_percent_fixed5(details.research_bonus_raw),
                ),
                None,
                plan,
            )?;
            if details.funds_gain_factor_raw != 0 {
                push_modifier(
                    modifier_item(
                        localisation,
                        "MODIFIER_MIO_FUNDS_GAIN",
                        &format_signed_percent_fixed5(details.funds_gain_factor_raw),
                    ),
                    None,
                    plan,
                )?;
            }
            if details.size_cost_factor_raw != 0 {
                push_modifier(
                    modifier_item(
                        localisation,
                        "MODIFIER_MIO_FUNDS_SIZE_UP_REQUIREMENT",
                        &format_signed_percent_fixed5(details.size_cost_factor_raw),
                    ),
                    None,
                    plan,
                )?;
            }
            if details.task_capacity != 0 {
                push_modifier(
                    modifier_item(
                        localisation,
                        "MODIFIER_MIO_TASK_CAPACITY",
                        &details.task_capacity.to_string(),
                    ),
                    None,
                    plan,
                )?;
            }

            let tree = prepared
                .catalog
                .as_deref()
                .and_then(|cat| cat.tree(&prepared.data.token).ok().flatten());
            if let Some(initial) = tree.and_then(|t| t.initial) {
                push_separator(
                    loc_or(localisation, "INDUSTRIAL_ORG_DETAIL_INITIAL_TRAIT_BONUSES"),
                    plan,
                )?;
                let initial_name = resolve_trait_name(
                    initial,
                    &prepared.data.token,
                    prepared.data.owner_tag.as_deref(),
                    localisation,
                );
                push_modifier(
                    loc_fill(
                        localisation,
                        "INDUSTRIAL_ORG_DETAIL_MIO_INITIAL_TRAIT_ITEM",
                        &[("NAME", &initial_name)],
                    ),
                    None,
                    plan,
                )?;
            }

            let totals = aggregate_bonuses(&prepared.data, tree, prepared.org_db.as_deref());
            if !totals.is_empty() {
                push_separator(
                    loc_or(localisation, "INDUSTRIAL_ORG_DETAIL_EQUIPMENT_BONUSES"),
                    plan,
                )?;
                for (name, value) in totals {
                    push_bonus(format_bonus_plain(&name, value, localisation), plan)?;
                }
            }
        }
    }
    Ok(())
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
    localisation: &Localisation,
) -> GameWindowTooltip {
    let mut lines = vec![status_line.to_owned()];
    for bonus in &trait_def.bonuses {
        lines.push(format_bonus_plain(&bonus.name, bonus.value, localisation));
    }
    for modifier in trait_def.organisation_modifiers.listed() {
        lines.push(format_bonus_plain(
            &modifier.name,
            modifier.value,
            localisation,
        ));
    }
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
    localisation: &Localisation,
    prepared: &PreparedMio,
    plan: &mut ComposedPlan,
) -> Result<(), GameWindowError> {
    let Some(tree_list) = find_direct(root, "tree_scrollbar_window/tree_window/tree_list") else {
        return Ok(());
    };
    let Some(tree_holder) = main_layout
        .holders
        .iter()
        .find(|h| h.path == tree_list.path)
    else {
        return Ok(());
    };
    let Some(tree_tpl) = guis.template(b"industrial_organisation_tree_window") else {
        return Err(GameWindowError::refused(missing_window_refusal(
            "industrial_organisation_tree_window",
        )));
    };
    let viewport = DEFAULT_VIEWPORT.map(|value| value as i32);
    let tree_layout = layout_window(tree_tpl, resources, viewport, |instance| {
        if let Some(bg) = &prepared.tree_background
            && let Ok(bound) = instance.control(tree_tpl, tree_tpl)
        {
            bound.sprite = Some(bg.as_bytes().to_vec());
        }
    })?;
    let traits_holder = find_direct(tree_tpl, "scrollbar_window/traits_window")
        .and_then(|el| container_holder(&tree_layout, &el.path));
    let header_holder = find_direct(tree_tpl, "tree_header")
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

    let Some(catalog) = prepared.catalog.as_deref() else {
        return Ok(());
    };
    let tree = match catalog.tree(&prepared.data.token) {
        Ok(Some(tree)) => tree,
        Ok(None) => {
            plan.warnings.push(format!(
                "Military industrial organisation '{}' is not defined in loaded organisation scripts.",
                prepared.data.token
            ));
            return Ok(());
        }
        Err(why) => {
            plan.warnings.push(format!(
                "Military industrial organisation '{}' trait tree could not be loaded: {why}",
                prepared.data.token
            ));
            return Ok(());
        }
    };

    let mut positions: Vec<(&TraitName, [i32; 2])> = Vec::new();
    for trait_def in tree.traits {
        match catalog.position(&prepared.data.token, &trait_def.token) {
            Ok(Some(pos)) => positions.push((trait_def, pos)),
            Ok(None) => {}
            Err(why) => plan.warnings.push(why),
        }
    }
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

    if let Some(mut hdr) = header_holder
        && let Some(flavor_tpl) = guis.template(b"industrial_organisation_flavortext_window")
    {
        hdr.origin = [tree_origin[0] + hdr.origin[0], tree_origin[1] + hdr.origin[1]];
        hdr.logical = [
            tree_holder.logical[0] + hdr.logical[0],
            tree_holder.logical[1] + hdr.logical[1],
        ];
        hdr.before = tree_before;
        for (idx, header_text) in tree.tree_header_texts.iter().enumerate() {
            let label = loc_or(localisation, &header_text.text);
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

    let Some(mut traits_box) = traits_holder else {
        return Ok(());
    };
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

    let unlocked_set: BTreeSet<String> = match &prepared.data.details {
        RecordedDetails::Available(details) => details
            .unlocked
            .iter()
            .map(|t| t.to_ascii_lowercase())
            .collect(),
        RecordedDetails::Unavailable(_) => BTreeSet::new(),
    };
    let queued_map: HashMap<String, usize> = match &prepared.data.details {
        RecordedDetails::Available(details) => details
            .queued
            .iter()
            .enumerate()
            .map(|(i, t)| (t.to_ascii_lowercase(), i + 1))
            .collect(),
        RecordedDetails::Unavailable(_) => HashMap::new(),
    };

    let pixel_of = |grid: [i32; 2]| -> [i32; 2] {
        [(grid[0] - min_x) * step_x, (grid[1] - min_y) * step_y]
    };
    let pos_by_token: HashMap<String, [i32; 2]> = positions
        .iter()
        .map(|(tr, grid)| (tr.token.to_ascii_lowercase(), pixel_of(*grid)))
        .collect();

    if let Some(icon_tpl) = guis.template(b"industrial_organisation_bonus_type_icon") {
        let connector_ctx = ConnectorPlace {
            holder: &traits_box,
            icon_tpl,
            resources,
            viewport,
        };
        let mut link_idx = 0usize;
        for (trait_def, grid) in &positions {
            let child_px = pixel_of(*grid);
            let child_unlocked = unlocked_set.contains(&trait_def.token.to_ascii_lowercase());
            for group in &trait_def.parents {
                for parent_token in &group.traits {
                    let key = parent_token.to_ascii_lowercase();
                    if let Some(&parent_px) = pos_by_token.get(&key) {
                        let both_unlocked = child_unlocked && unlocked_set.contains(&key);
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

    let Some(trait_tpl) = guis.template(b"industrial_organisation_trait_entry") else {
        return Err(GameWindowError::refused(missing_window_refusal(
            "industrial_organisation_trait_entry",
        )));
    };
    let bonus_icon_tpl = guis.template(b"industrial_organisation_bonus_type_icon");
    let has_sprite = |name: &str| resources.sprite_info(name.as_bytes()).is_some();

    for (idx, (trait_def, grid)) in positions.iter().enumerate() {
        let px = pixel_of(*grid);
        let key = trait_def.token.to_ascii_lowercase();
        let is_unlocked = unlocked_set.contains(&key);
        let queue_pos = queued_map.get(&key).copied();
        let details_known = matches!(prepared.data.details, RecordedDetails::Available(_));
        let frame = if is_unlocked {
            1
        } else if queue_pos.is_some() {
            2
        } else {
            3
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
        let trait_name = resolve_trait_name(
            trait_def,
            &prepared.data.token,
            prepared.data.owner_tag.as_deref(),
            localisation,
        );
        let status_line = if !details_known {
            "Status: n/a (trait unlock state not recorded at this hour)".to_owned()
        } else if is_unlocked {
            "Status: Unlocked".to_owned()
        } else if let Some(pos) = queue_pos {
            format!("Status: Queued (#{pos})")
        } else {
            "Status: Locked".to_owned()
        };

        let trait_layout = layout_window(trait_tpl, resources, viewport, |instance| {
            hide_by_name(instance, trait_tpl, "ai_value");
            set_sprite_by_name(
                instance,
                trait_tpl,
                "industrial_organisation_trait_icon/icon_frame",
                frame_sprite,
            );
            set_frame_by_name(
                instance,
                trait_tpl,
                "industrial_organisation_trait_icon/icon_frame",
                frame,
            );
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
        if let Some(btn) = find_direct(trait_tpl, "industrial_organisation_trait_icon/icon_frame") {
            overrides.insert(
                btn.path.clone(),
                format_trait_tooltip(trait_def, &trait_name, &status_line, localisation),
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
            if !trait_def.organisation_modifiers.is_empty() {
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

fn compose_history_lists(
    main_layout: &Layout,
    root: &Element,
    guis: &native::Guis,
    resources: &dyn Resources,
    localisation: &Localisation,
    prepared: &PreparedMio,
    plan: &mut ComposedPlan,
) -> Result<(), GameWindowError> {
    let Some(item_tpl) = guis.template(b"industrial_organisation_history_item") else {
        return Ok(());
    };
    let Some(list_el) = find_direct(root, "history_window/variants_window/variant_history_list")
    else {
        return Ok(());
    };
    let Some(holder) = main_layout
        .holders
        .iter()
        .find(|h| h.path == list_el.path)
    else {
        return Ok(());
    };
    let viewport = DEFAULT_VIEWPORT.map(|value| value as i32);
    let max_y = holder.content[1].max(0);
    match &prepared.data.history {
        None => {
            let reason = prepared
                .data
                .history_reason
                .as_deref()
                .unwrap_or("History is not recorded at this hour.");
            let layout = layout_window(item_tpl, resources, viewport, |instance| {
                set_text_by_name(
                    instance,
                    item_tpl,
                    "equipment_name",
                    NOT_RECORDED_VALUE.to_owned(),
                );
                set_text_by_name(
                    instance,
                    item_tpl,
                    "equipment_archetype_name",
                    "Not recorded".to_owned(),
                );
                set_text_by_name(instance, item_tpl, "flavor_fact", reason.to_owned());
                hide_by_name(instance, item_tpl, "upgrade_variant_button");
            })?;
            plan.sub_windows.push(ComposedSubWindow {
                before_part: holder.before,
                parts: holder.place([0, 0], &layout),
                regions: holder.place_regions([0, 0], &layout.regions),
                diagnostics: layout.diagnostics,
                root_window: "industrial_organisation_history_item".to_owned(),
                path_prefix: "history:missing/".to_owned(),
                close_paths: Vec::new(),
                tooltip_overrides: BTreeMap::new(),
            });
        }
        Some(entries) => {
            let step = item_tpl
                .container()
                .map(|container| container.size.size[1].value)
                .filter(|&height| height > 0)
                .ok_or_else(|| {
                    GameWindowError::refused(
                        "Template 'industrial_organisation_history_item' declares no height, so its rows cannot be stacked.",
                    )
                })?;
            let visible = usize::try_from(max_y / step).unwrap_or(0);
            if entries.len() > visible {
                plan.warnings.push(format!(
                    "{} of {} history entries lie below the visible list and are not drawn: the game window scrolls, this picture does not.",
                    entries.len() - visible,
                    entries.len()
                ));
            }
            for (idx, entry) in entries.iter().take(visible).enumerate() {
                let y = idx as i32 * step;
                let archetype_label = if entry.archetype.is_empty() {
                    String::new()
                } else {
                    loc_or(localisation, &entry.archetype)
                };
                let fact = loc_fill(
                    localisation,
                    "INDUSTRIAL_ORG_DETAIL_WINDOW_HISTORY_ENTRY_FLAVOR_UNITS_PRODUCED",
                    &[("NUMBER", &entry.units.to_string())],
                );
                let layout = layout_window(item_tpl, resources, viewport, |instance| {
                    set_text_by_name(instance, item_tpl, "equipment_name", entry.name.clone());
                    set_text_by_name(
                        instance,
                        item_tpl,
                        "equipment_archetype_name",
                        archetype_label,
                    );
                    set_text_by_name(instance, item_tpl, "flavor_fact", fact);
                    disable_by_name(instance, item_tpl, "upgrade_variant_button");
                })?;
                plan.sub_windows.push(ComposedSubWindow {
                    before_part: holder.before,
                    parts: holder.place([0, y], &layout),
                    regions: holder.place_regions([0, y], &layout.regions),
                    diagnostics: layout.diagnostics,
                    root_window: "industrial_organisation_history_item".to_owned(),
                    path_prefix: format!("history:{idx}/"),
                    close_paths: Vec::new(),
                    tooltip_overrides: BTreeMap::new(),
                });
            }
        }
    }
    Ok(())
}

fn compose_policy_list(
    main_layout: &Layout,
    root: &Element,
    guis: &native::Guis,
    resources: &dyn Resources,
    localisation: &Localisation,
    prepared: &PreparedMio,
    plan: &mut ComposedPlan,
) -> Result<(), GameWindowError> {
    let Some(item_tpl) = guis.template(b"policy_item") else {
        return Err(GameWindowError::refused(missing_window_refusal(
            "policy_item",
        )));
    };
    let list_el = find_direct(root, "policy_list_window/policy_list").ok_or_else(|| {
        GameWindowError::refused(format!(
            "Window '{POLICY_WINDOW_NAME}' is missing required list 'policy_list_window/policy_list'."
        ))
    })?;
    let holder = main_layout
        .holders
        .iter()
        .find(|h| h.path == list_el.path)
        .ok_or_else(|| {
            GameWindowError::refused(format!(
                "Policy list '{}' produced no layout holder.",
                list_el.path
            ))
        })?;
    let step = item_tpl
        .container()
        .map(|container| container.size.size[1].value)
        .filter(|&height| height > 0)
        .ok_or_else(|| {
            GameWindowError::refused(
                "Template 'policy_item' declares no height, so its rows cannot be stacked.",
            )
        })?;
    let viewport = DEFAULT_VIEWPORT.map(|value| value as i32);
    let max_y = holder.content[1].max(0);
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
            plan.sub_windows.push(ComposedSubWindow {
                before_part: holder.before,
                parts: holder.place([0, 0], &layout),
                regions: holder.place_regions([0, 0], &layout.regions),
                diagnostics: layout.diagnostics,
                root_window: "policy_item".to_owned(),
                path_prefix: "policy:missing/".to_owned(),
                close_paths: Vec::new(),
                tooltip_overrides: overrides,
            });
        }
        RecordedDetails::Available(details) => {
            let attached_lower = details
                .policy_token
                .as_deref()
                .map(str::to_ascii_lowercase);
            let mut tokens: Vec<String> = details.allowed_policies.clone();
            if let Some(attached) = &details.policy_token
                && !tokens.iter().any(|t| t.eq_ignore_ascii_case(attached))
            {
                tokens.insert(0, attached.clone());
            }
            let visible = usize::try_from(max_y / step).unwrap_or(0);
            if tokens.len() > visible {
                plan.warnings.push(format!(
                    "{} of {} policies lie below the visible list and are not drawn: the game window scrolls, this picture does not.",
                    tokens.len() - visible,
                    tokens.len()
                ));
            }
            for (idx, token) in tokens.iter().take(visible).enumerate() {
                let y = idx as i32 * step;
                let key = token.to_ascii_lowercase();
                let policy_def = prepared
                    .org_db
                    .as_deref()
                    .and_then(|db| db.policies.policy(token.as_bytes()).ok());
                let name = resolve_policy_name(token, policy_def, localisation);
                let icon = resolve_policy_icon(policy_def, has_sprite);
                let is_attached = attached_lower.as_deref() == Some(key.as_str());
                let (cost_str, cost_missing) = match &prepared.data.policy_costs {
                    Ok(costs) => match costs.get(&key).and_then(|summary| summary.cost) {
                        Some(cost) => (cost.to_string(), None),
                        None => (
                            NOT_RECORDED_VALUE.to_owned(),
                            Some("Not recorded: this policy's attach cost is not in the recorded policy costs at this hour.".to_owned()),
                        ),
                    },
                    Err(reason) => (
                        NOT_RECORDED_VALUE.to_owned(),
                        Some(format!("Not recorded: policy costs are unavailable at this hour ({reason}).")),
                    ),
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
                    set_text_by_name(instance, item_tpl, "attach_cost", cost_str.clone());
                })?;
                let mut overrides = BTreeMap::new();
                if let Some(btn) = find_direct(item_tpl, "background_button") {
                    let mut lines = Vec::new();
                    lines.extend(cost_missing);
                    if let Some(def) = policy_def {
                        for (_, group_bonuses) in &def.groups {
                            for bonus in group_bonuses {
                                lines.push(format_bonus_plain(
                                    &bonus.name,
                                    bonus.value,
                                    localisation,
                                ));
                            }
                        }
                        for modifier in def.organisation_modifiers.listed() {
                            lines.push(format_bonus_plain(
                                &modifier.name,
                                modifier.value,
                                localisation,
                            ));
                        }
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
                    parts: holder.place([0, y], &layout),
                    regions: holder.place_regions([0, y], &layout.regions),
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
    sub_view: SubView,
    scale: f32,
    admitted_gen: Option<(u64, u64)>,
    cancel: &AtomicBool,
) -> Result<GameWindowRender, GameWindowError> {
    let window_name = sub_view.window_name();
    let catalog = mio_traits::cached(layers).ok();
    let org_db = cached_organisation_database(layers).ok();
    let read_hour = Some(data.read_hour);
    let extra_warnings = data.warnings.clone();

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
            let has_sprite = |name: &str| shared.registry.sprite(name.as_bytes()).is_some();
            let tree = catalog
                .as_deref()
                .and_then(|cat| cat.tree(&data.token).ok().flatten());
            let emblem_sprite = resolve_org_icon(&data, tree, has_sprite);
            assets.sprites.insert(emblem_sprite.as_bytes().to_vec());

            let tree_background = tree
                .and_then(|t| t.background)
                .filter(|bg| !bg.is_empty() && has_sprite(bg))
                .map(str::to_owned);
            if let Some(bg) = &tree_background {
                assets.sprites.insert(bg.as_bytes().to_vec());
            }

            let attached_policy_sprite = match &data.details {
                RecordedDetails::Available(details) => details.policy_token.as_deref().map(|tok| {
                    let def = org_db
                        .as_deref()
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
                    for tok in &details.allowed_policies {
                        let def = org_db
                            .as_deref()
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
        |root, _, _, localisation, instance, tooltips, prepared| match prepared.sub_view {
            SubView::Traits | SubView::History => {
                configure_detail_main(root, localisation, instance, tooltips, prepared);
            }
            SubView::Policy => {
                configure_policy_main(root, localisation, instance, prepared);
            }
        },
        |main_layout, root, guis, resources, localisation, prepared| {
            let mut plan = ComposedPlan::default();
            match prepared.sub_view {
                SubView::Traits => {
                    compose_funds_progressbar(
                        main_layout,
                        root,
                        guis,
                        resources,
                        localisation,
                        prepared,
                        &mut plan,
                    )?;
                    compose_aggregated_bonuses(
                        main_layout,
                        root,
                        guis,
                        resources,
                        localisation,
                        prepared,
                        &mut plan,
                    )?;
                    compose_trait_tree(
                        main_layout,
                        root,
                        guis,
                        resources,
                        localisation,
                        prepared,
                        &mut plan,
                    )?;
                }
                SubView::History => {
                    compose_funds_progressbar(
                        main_layout,
                        root,
                        guis,
                        resources,
                        localisation,
                        prepared,
                        &mut plan,
                    )?;
                    compose_aggregated_bonuses(
                        main_layout,
                        root,
                        guis,
                        resources,
                        localisation,
                        prepared,
                        &mut plan,
                    )?;
                    compose_history_lists(
                        main_layout,
                        root,
                        guis,
                        resources,
                        localisation,
                        prepared,
                        &mut plan,
                    )?;
                }
                SubView::Policy => {
                    compose_policy_list(
                        main_layout,
                        root,
                        guis,
                        resources,
                        localisation,
                        prepared,
                        &mut plan,
                    )?;
                }
            }
            plan.title = Some(resolve_org_name(
                &prepared.data,
                prepared.org_db.as_deref(),
                localisation,
            ));
            Ok(plan)
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
        content: _,
        layers,
        resolve,
        visual_revision,
        language,
        decode,
        admitted_gen,
        mut extra_warnings,
    } = ctx;
    let sub_view = SubView::from_request(&request.kind, request.sub_view.as_deref())?;
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
    let (lexical, _) = witnessed_rows::<tables::LexicalTokens>(&frame).map_err(|error| {
        GameWindowError::refused(format!(
            "Lexical tokens are not recorded at this hour: {error}"
        ))
    })?;
    let token = spell(lexical, org_row.token).ok_or_else(|| {
        GameWindowError::refused(format!(
            "Lexical tokens do not spell organisation {}'s script key.",
            target.organisation
        ))
    })?;
    let mut read_tables = vec![TableId::IndustrialOrganisations, TableId::LexicalTokens];
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
            RecordedDetails::Available(Box::new(extract_details(row, lexical, &mut extra_warnings)))
        }
        Ok(None) => {
            read_tables.push(TableId::IndustrialOrganisationDetails);
            RecordedDetails::Unavailable(
                "no organisation details row was recorded for this organisation at this hour".to_owned(),
            )
        }
        Err(error) => RecordedDetails::Unavailable(error),
    };

    let policy_costs = match frame.complete::<tables::IndustrialOrganisationPolicies>() {
        Ok(rows) => match frame.complete::<tables::CountryLinks>() {
            Ok(links) => {
                read_tables.push(TableId::IndustrialOrganisationPolicies);
                Ok(extract_policy_costs(rows, org_row.country, links, lexical))
            }
            Err(error) => Err(error.to_string()),
        },
        Err(error) => Err(error.to_string()),
    };

    let (history, history_reason) = match frame.complete::<tables::IndustrialOrganisationHistory>() {
        Ok(rows) if sub_view == SubView::History => {
            read_tables.push(TableId::IndustrialOrganisationHistory);
            let variants = frame.complete::<tables::VariantDefinitions>().ok();
            if variants.is_some() {
                read_tables.push(TableId::VariantDefinitions);
            } else {
                extra_warnings.push(
                    "Design names in the history are not recorded at this hour (VariantDefinitions is unavailable).".to_owned(),
                );
            }
            (Some(extract_history(rows, org_id, variants, lexical)), None)
        }
        Ok(_) => (None, None),
        Err(error) => (None, Some(error.to_string())),
    };

    let samples: Vec<(TableId, i32)> = read_tables
        .iter()
        .filter_map(|&id| frame.sampled_at(id).map(|hour| (id, hour)))
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
    render_data(
        service,
        layers,
        key,
        data,
        sub_view,
        request.scale,
        Some(admitted_gen),
        cancel,
    )
}
