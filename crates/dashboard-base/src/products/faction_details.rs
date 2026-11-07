//! One faction's retained goal state at the requested recorded hour.
//! Definitions and faction-scoped text use the recording's content stack.
use super::{LoadContext, Product, check_reply_budget};
use crate::{
    Source, catalogue, content, faction_names, invalid, metadata::validate_campaign,
    scripted_text::ScriptedText,
};
use replay_data::rows::{TableId, tables};
use replay_frame::frame::{DecodedFrame, HourFrame};
use serde::{Deserialize, Serialize};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

mod accounts;
mod companions;
mod derived;
mod manpower;
mod pools;
mod presentations;
pub mod selectors;
mod viewer;
pub use accounts::{FactionAccountDetail, FactionUpgradeDetail};
pub use companions::{FactionManpowerDetail, FactionProgramDetail, FactionRuleDetail};
pub use pools::FactionResourceDetail;
pub use viewer::ViewerText;
/// The tables every faction detail reads at its hour; the others only when the hour needs them.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Clock,
    TableId::SimulationFeatures,
    TableId::ModifierDefinitions,
    TableId::LocalPlayer,
    TableId::Factions,
    TableId::FactionGoalStatus,
    TableId::FactionGoalEntries,
    TableId::FactionRules,
    TableId::FactionPrograms,
    TableId::FactionManpower,
    TableId::FactionUnlocks,
    TableId::FactionVariables,
    TableId::FactionAccounts,
    TableId::FactionResearch,
    TableId::FactionMinimumUpgrades,
    TableId::FactionDoctrine,
    TableId::FactionProjection,
    TableId::FactionResources,
    TableId::FactionAdvisorModifiers,
    TableId::CountryLinks,
    TableId::Countries,
    TableId::CountryStabilityInputs,
    TableId::SliceReads,
    TableId::FactionMembers,
    TableId::DoctrineCountries,
    TableId::DoctrineFolders,
    TableId::DoctrineTracks,
    TableId::LexicalTokens,
];
pub struct FactionDetails;
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactionDetailParams {
    pub branch_id: String,
    pub hour: i32,
    pub faction: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionGoalDefinition {
    pub progress: Vec<FactionRewardDetail>,
    pub sections: Vec<FactionRewardDetail>,
    pub bands: Vec<FactionProgressBand>,
    pub conditions: Vec<FactionConditionDetail>,
    pub rewards: Vec<FactionRewardDetail>,
    pub flags: Vec<FactionConditionDetail>,
    pub token: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub category: Option<String>,
    /// The term's name as the game's interface names it, from the localisation key
    /// the native term carries; null when the content does not define it.
    pub category_name: Option<String>,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionConditionDetail {
    pub kind: String,
    pub result: Option<bool>,
    pub reasons: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionRewardDetail {
    pub kind: String,
    pub text: String,
    pub reasons: Vec<String>,
}
/// A progress section the game activates while the goal's progress lies in its
/// range, as fractions of that progress, named when the content localises its key.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionProgressBand {
    pub name: Option<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionGoalDetail {
    pub ordinal: u32,
    pub definition: FactionGoalDefinition,
    pub status: i32,
    /// Exact retained fixed-point cache as decimal text.
    pub progress_raw: String,
    pub active_sections: Vec<i32>,
    pub progress_percent: Option<f64>,
    pub date: i32,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionDetailReply {
    pub presentations: Vec<FactionRewardDetail>,
    pub projection_raw: Option<String>,
    pub influence_total_raw: Option<String>,
    pub projection_effects_raw: Option<String>,
    pub resources: Option<Vec<FactionResourceDetail>>,
    pub system_resources: Option<Vec<FactionResourceDetail>>,
    pub refusals: Vec<FactionRefusalDetail>,
    pub minimum_upgrades: Option<Vec<FactionUpgradeDetail>>,
    pub doctrine_folders: Option<Vec<FactionRuleDetail>>,
    pub research_base_raw: Option<String>,
    pub research_countries: Option<Vec<String>>,
    pub research_bonuses: Option<Vec<String>>,
    /// The faction's member accounts, and those of countries outside every faction whose
    /// account names this faction as its supplying one.
    pub accounts: Option<Vec<FactionAccountDetail>>,
    /// Retained accounts of countries outside every faction that name no supplying
    /// faction: nothing recorded at the hour ties them to this faction or another.
    pub unassociated_accounts: Option<Vec<FactionAccountDetail>>,
    pub variable_seed: Option<u32>,
    pub variables: Option<Vec<FactionVariableDetail>>,
    pub branch_id: String,
    pub hour: i32,
    pub faction: String,
    pub goal_reason: Option<String>,
    pub research_unlocked: Option<bool>,
    pub manpower_unlocked: Option<bool>,
    pub rules: Option<Vec<FactionRuleDetail>>,
    pub programmes: Option<Vec<FactionProgramDetail>>,
    pub programmes_read_hour: Option<i32>,
    pub manpower: Option<Vec<FactionManpowerDetail>>,
    pub manpower_total: Option<i32>,
    pub manifest: Option<FactionGoalDefinition>,
    pub goals: Vec<FactionGoalDetail>,
    pub completed: Vec<FactionGoalDefinition>,
    pub cancelled: Vec<FactionGoalDefinition>,
    pub extra_slots: Vec<i32>,
    pub term_dates: Vec<i32>,
    /// The faction's logo as the game draws it: its template's sprite, else the
    /// content's default faction icon.
    pub logo: Option<String>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionRefusalDetail {
    pub fact: String,
    pub count: u32,
    pub reason: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionVariableDetail {
    pub ordinal: u32,
    /// Literal bytes remain available when the key is not UTF-8.
    pub name_bytes: Vec<u8>,
    pub name: String,
    pub value_raw: String,
}
impl Product for FactionDetails {
    const KEY: &'static str = "faction-details";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = FactionDetailParams;
    type Reply = FactionDetailReply;
    fn types() -> Vec<String> {
        vec![
            FactionDetailParams::decl(),
            FactionGoalDefinition::decl(),
            FactionConditionDetail::decl(),
            FactionRewardDetail::decl(),
            FactionProgressBand::decl(),
            FactionGoalDetail::decl(),
            FactionRuleDetail::decl(),
            FactionProgramDetail::decl(),
            FactionManpowerDetail::decl(),
            FactionAccountDetail::decl(),
            FactionUpgradeDetail::decl(),
            FactionVariableDetail::decl(),
            FactionResourceDetail::decl(),
            FactionRefusalDetail::decl(),
            FactionDetailReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err()
            || !params.faction.parse::<u64>().is_ok_and(|id| id != 0)
        {
            return Err("Invalid faction detail identity");
        }
        Ok(())
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        if cancelled.load(Ordering::Acquire) {
            return Err(invalid("Faction details cancelled"));
        }
        let branch = params
            .branch_id
            .parse()
            .map_err(|_| invalid("Invalid faction branch"))?;
        let faction = params
            .faction
            .parse()
            .map_err(|_| invalid("Invalid faction identity"))?;
        let mut selected = TABLES.to_vec();
        let mut frame = crate::prepared::query::native::frame_at(
            source,
            branch,
            params.hour,
            &selected,
            cancelled,
        )?
        .ok_or_else(|| dashboard_core::refused("Faction details unavailable at this hour"))?;
        validate_campaign(&frame, &source.replay())?;
        let mut extra = companions::extra_tables(&frame, faction);
        if frame
            .complete::<tables::FactionResources>()
            .is_ok_and(|rows| !rows.is_empty())
        {
            extra.push(TableId::ResourceDefinitions);
        }
        if !extra.is_empty() {
            selected.extend(extra);
            selected.sort_unstable();
            selected.dedup();
            frame = crate::prepared::query::native::frame_at(
                source,
                branch,
                params.hour,
                &selected,
                cancelled,
            )?
            .ok_or_else(|| dashboard_core::refused("Faction details are unavailable at this hour"))?;
        }
        if let Ok(rows) = frame.complete::<tables::Factions>()
            && !rows.iter().any(|row| row.id == faction)
        {
            return Err(invalid("Faction is absent at this hour"));
        }
        let lexical =
            faction_names::LexicalNames::new(frame.tables[TableId::LexicalTokens.index()].clone());
        let mut warnings = Vec::new();
        // Programme details keep their existing daily cadence. Bind every
        // dependent join/text to that supplying hour, independent of current refs.
        let mut programme_frame = if companions::has_programmes(&frame, faction) {
            match crate::prepared::query::native::latest_read(
                source,
                branch,
                params.hour,
                TableId::ProjectPrograms,
                replay_data::cadence::STAGGER_PERIOD,
                companions::PROGRAMME_TABLES,
                cancelled,
            ) {
                Ok(value) => value,
                Err(why) => {
                    warnings.push(format!("Shared programme supplying read: {why}"));
                    None
                }
            }
        } else {
            None
        };

        let cat = match context
            .content
            .as_ref()
            .map(|options| catalogue::catalogue(source, options))
            .transpose()
        {
            Ok(cat) => cat,
            Err(reason) => {
                warnings.push(format!("Faction goal content: {reason}"));
                None
            }
        };
        let definitions = match cat
            .as_ref()
            .map(|cat| installed_content_core::faction_goals::cached(&cat.content.layers))
            .transpose()
        {
            Ok(definitions) => definitions,
            Err(reason) => {
                warnings.push(reason);
                None
            }
        };
        if let Some(cat) = cat.as_deref()
            && (definitions.as_deref().is_some_and(|definitions| {
                needs_environment(&frame, faction, cat, definitions, &lexical)
            }) || companions::needs_environment(&frame, faction, cat))
        {
            // Only reached scripted text and goal rewards need the wider world, every
            // effect operand included. The shared frame cache reads the missing tables
            // alongside the held ones.
            selected.extend(crate::effect_description::recorded_tables());
            selected.sort_unstable();
            selected.dedup();
            frame = crate::prepared::query::native::frame_at(
                source,
                branch,
                params.hour,
                &selected,
                cancelled,
            )?
            .ok_or_else(|| dashboard_core::refused("Faction text unavailable at this hour"))?;
        }
        if let (Some(cat), Some(details)) = (cat.as_deref(), programme_frame.as_deref())
            && companions::programmes_need_environment(
                replay_frame::frame::HourFrame::from(&*frame),
                replay_frame::frame::HourFrame::from(details),
                faction,
                cat,
            )
        {
            let tables = selectors::programme_text_tables();
            match crate::prepared::query::native::frame_at(
                source,
                branch,
                details.hour,
                &tables,
                cancelled,
            ) {
                Ok(Some(value)) => programme_frame = Some(value),
                Ok(None) => warnings.push("Shared programme text frame is unavailable".into()),
                Err(why) => warnings.push(format!("Shared programme text frame: {why}")),
            }
        }
        let defines = SharedDefines::default();
        let texts = cat
            .as_deref()
            .map(|cat| presentations::Texts::new(&frame, faction, cat, &defines));
        let mut rendered = std::collections::BTreeMap::new();
        let mut reply = match read(&frame, params, faction, |token| {
            rendered
                .entry(token)
                .or_insert_with(|| {
                    definition(
                        &frame,
                        token,
                        texts.as_ref(),
                        definitions.as_deref(),
                        &lexical,
                    )
                })
                .clone()
        }) {
            Ok(reply) => reply,
            Err(why) => FactionDetailReply {
                branch_id: params.branch_id.clone(),
                hour: params.hour,
                faction: params.faction.clone(),
                goal_reason: Some(why.to_string()),
                warnings: vec![format!("Faction goals: {why}")],
                ..Default::default()
            },
        };
        pools::attach(source, context, cancelled, &frame, faction, &mut reply)?;
        accounts::attach(
            &frame,
            faction,
            texts.as_ref(),
            definitions.as_deref(),
            &lexical,
            &mut reply,
        );
        companions::attach(
            &frame,
            faction,
            cat.as_deref(),
            programme_frame.as_deref(),
            &mut reply,
        );
        if let Some(options) = context.content.as_ref() {
            content::check_replay(
                &source.replay(),
                options.allow_unverified,
                &mut reply.warnings,
            )?;
        }
        if let Some(definitions) = definitions {
            reply.warnings.extend(definitions.warnings.iter().cloned());
        }
        derived::attach(
            &frame,
            programme_frame.as_deref(),
            faction,
            cat.as_deref(),
            &mut reply,
        );
        manpower::attach(
            source,
            branch,
            cancelled,
            &frame,
            faction,
            cat.as_deref(),
            &defines,
            &mut reply,
        )?;
        collect_icon_refusals(&mut reply);
        if let Some(cat) = cat.as_deref() {
            match faction_logo(&frame, faction, cat, &defines) {
                Ok(logo) => reply.logo = Some(logo),
                Err(reason) => reply.refusals.push(FactionRefusalDetail {
                    fact: "faction logo".into(),
                    count: 1,
                    reason,
                }),
            }
        }
        reply.warnings.extend(warnings);
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}
/// The content stack's defines, read at most once per request: even a cached
/// read rereads every define script.
#[derive(Default)]
struct SharedDefines(std::cell::OnceCell<Result<installed_content_core::defines::Defines, String>>);
impl SharedDefines {
    fn get(&self, cat: &catalogue::Catalogue) -> Result<&installed_content_core::defines::Defines, String> {
        self.0
            .get_or_init(|| {
                installed_content_core::defines::read(&cat.content.layers).map_err(|why| why.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}
fn needs_environment(
    frame: &DecodedFrame,
    faction: u64,
    cat: &catalogue::Catalogue,
    definitions: &installed_content_core::faction_goals::Catalog,
    lexical: &faction_names::LexicalNames,
) -> bool {
    if frame
        .complete::<tables::FactionGoalEntries>()
        .ok()
        .is_some_and(|rows| {
            rows.iter()
                .any(|row| row.faction == faction && row.kind == 0)
        })
        && definitions.definitions().any(|(_, definition)| {
            ["available", "allowed"].iter().any(|kind| {
                definition
                    .conditions
                    .get(*kind)
                    .is_some_and(|body| !body.is_empty())
            })
        })
    {
        return true;
    }
    let tokens = frame
        .complete::<tables::FactionGoalStatus>()
        .ok()
        .and_then(|rows| {
            rows.binary_search_by_key(&faction, |row| row.faction)
                .ok()
                .map(|index| &rows[index])
        })
        .and_then(|row| row.manifest)
        .into_iter()
        .chain(
            frame
                .complete::<tables::FactionGoalEntries>()
                .ok()
                .into_iter()
                .flat_map(|rows| companions::selected(rows, faction, |row| row.faction))
                .take_while(|row| row.kind <= 2)
                .filter_map(|row| row.token),
        )
        // Accounts name their completed goals through the same goal renderer.
        .chain(
            frame
                .complete::<tables::FactionAccounts>()
                .ok()
                .into_iter()
                .flatten()
                .filter(|row| row.kind == 0)
                .flat_map(|row| &row.completed_goals)
                .filter(|&&token| token != u64::MAX)
                .map(|&token| token as u32),
        );
    for token in tokens {
        let Ok(Some(spelling)) = faction_names::template_spelling(frame, Some(token)) else {
            continue;
        };
        let Ok(Some(definition)) = definitions.get(&spelling) else {
            continue;
        };
        if definition
            .conditions
            .values()
            .chain(definition.effects.values())
            .any(|body| !body.is_empty())
            || definition.progress_sections.iter().flatten().any(|field| {
                (field.keyword_is("on_activate") || field.keyword_is("on_deactivate"))
                    && field.value.items().is_some_and(|body| !body.is_empty())
            })
            || expands(definition, cat, lexical)
        {
            return true;
        }
    }
    false
}
/// A goal term's name as the interface shows it: the native terms' interface names
/// are keyed by the term in capitals. None when the content leaves it undefined or empty.
fn term_name(
    localisation: &installed_content_localisation::localisation::Localisation,
    category: &str,
) -> Option<String> {
    let key = category.to_ascii_uppercase();
    localisation.get(&key)?;
    catalogue::localised(localisation, &key, catalogue::View::Plain)
        .ok()
        .filter(|name| !name.is_empty())
}

fn goal_icon(cat: &catalogue::Catalogue, group: Option<&[u8]>) -> Result<Option<String>, String> {
    let group = group.ok_or("Faction goal artwork: group is not authored")?;
    let group =
        std::str::from_utf8(group).map_err(|_| "Faction goal artwork: group is not UTF-8")?;
    let name = format!("GFX_{group}");
    // The sprite is the goal group's; drawn as the selection shows it, else the recording's
    // own, named.
    let read = |cat: &catalogue::Catalogue| -> io::Result<String> {
        let sprite = cat.sprites.lookup(&name).map_err(invalid)?;
        let decode = cat.gui_decode.as_ref().map_err(invalid)?;
        catalogue::sprite_frame_png(&cat.content.layers, sprite, *decode, 0, 64)
    };
    catalogue::draw(
        || format!("faction goal {name} icon"),
        cat,
        |cat| read(cat).map_err(|why| format!("Faction goal artwork {name}: {why}")),
    )
    .get()
    .clone()
    .map(Some)
}

/// The faction's logo as the game draws it: its template's sprite, else the
/// content's default for a faction without one.
fn faction_logo(
    frame: &DecodedFrame,
    faction: u64,
    cat: &catalogue::Catalogue,
    defines: &SharedDefines,
) -> Result<String, String> {
    let row = frame
        .complete::<tables::Factions>()
        .map_err(|why| why.to_string())?
        .iter()
        .find(|row| row.id == faction)
        .ok_or("Faction logo: the faction is absent at this hour")?
        .clone();
    let templated = match faction_names::template_spelling(frame, row.template_token)? {
        Some(spelling) => cat
            .content
            .layers
            .faction_templates()?
            .icon(&spelling)?
            .map(<[u8]>::to_vec),
        None => None,
    };
    let name = match templated {
        Some(name) => {
            String::from_utf8(name).map_err(|_| "Faction logo sprite name is not UTF-8".to_owned())?
        }
        None => defines
            .get(cat)?
            .text("NFactions", "FACTION_DEFAULT_ICON")
            .map_err(|why| format!("Faction logo: {why}"))?,
    };
    let read = |cat: &catalogue::Catalogue| -> io::Result<String> {
        let sprite = cat.sprites.lookup(&name).map_err(invalid)?;
        let decode = cat.gui_decode.as_ref().map_err(invalid)?;
        catalogue::sprite_frame_png(&cat.content.layers, sprite, *decode, 0, 128)
    };
    catalogue::draw(
        || format!("faction logo {name}"),
        cat,
        |cat| read(cat).map_err(|why| format!("Faction logo {name}: {why}")),
    )
    .get()
    .clone()
}

fn collect_icon_refusals(reply: &mut FactionDetailReply) {
    let mut refused = std::collections::BTreeMap::<String, u32>::new();
    for goal in reply
        .manifest
        .iter()
        .chain(reply.goals.iter().map(|goal| &goal.definition))
        .chain(&reply.completed)
        .chain(&reply.cancelled)
        .chain(
            reply
                .accounts
                .iter()
                .chain(&reply.unassociated_accounts)
                .flatten()
                .flat_map(|account| &account.completed),
        )
    {
        // Artwork is evaluated before the independent text fields. Preserve
        // the complete refusal: authored sprite names may contain separators.
        if let Some(reason) = goal
            .reason
            .as_ref()
            .filter(|reason| reason.starts_with("Faction goal artwork"))
        {
            *refused.entry(reason.clone()).or_default() += 1;
        }
    }
    reply.refusals.extend(
        refused
            .into_iter()
            .map(|(reason, count)| FactionRefusalDetail {
                fact: "goal artwork".into(),
                count,
                reason,
            }),
    );
}

/// A goal token's texts at an hour; its artwork depends on content alone ([`finish_goal`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub struct GoalLabel {
    pub token: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub category_name: Option<String>,
    /// Whether its definition was reached, so its artwork is drawn.
    pub reached: bool,
    pub group: Option<Vec<u8>>,
    /// The refusal before the definition, or each text that failed.
    pub failures: Vec<String>,
}
/// What a goal label expands its scripted texts with.
trait GoalText {
    fn cat(&self) -> &catalogue::Catalogue;
    fn expand(&self, text: &str) -> Result<String, String>;
}
impl GoalText for presentations::Texts<'_> {
    fn cat(&self) -> &catalogue::Catalogue {
        presentations::Texts::cat(self)
    }
    fn expand(&self, text: &str) -> Result<String, String> {
        self.viewer()?.expand_faction(text, self.faction())
    }
}
/// A faction's goal texts expanded in the hour's shared [`ViewerText`].
struct HeldGoalText<'f, 'h> {
    faction: u64,
    held: &'h ViewerText<'f>,
}
impl GoalText for HeldGoalText<'_, '_> {
    fn cat(&self) -> &catalogue::Catalogue {
        self.held.cat()
    }
    fn expand(&self, text: &str) -> Result<String, String> {
        let root = presentations::viewer_root_in(self.held.frame())?;
        self.held
            .with(root, |scripted| scripted.expand_faction(text, self.faction))?
    }
}
fn label<'d>(
    frame: HourFrame<'_>,
    token: Option<u32>,
    texts: Option<&dyn GoalText>,
    definitions: Option<&'d installed_content_core::faction_goals::Catalog>,
    lexical: &faction_names::LexicalNames,
) -> (
    GoalLabel,
    Option<&'d installed_content_core::faction_goals::Definition>,
) {
    let mut label = GoalLabel::default();
    let Some(token) = token else {
        return (label, None);
    };
    let mut reach = || -> Result<_, String> {
        let spelling = faction_names::template_spelling_in(&frame, Some(token))?
            .ok_or("Faction goal token is absent")?;
        label.token = Some(spelling.clone());
        let definition = definitions
            .ok_or("Faction goal names need the recorded game folders")?
            .get(&spelling)?
            .ok_or_else(|| format!("Faction goal {spelling:?} is absent from recorded content"))?;
        Ok((
            definition,
            texts.ok_or("Faction goal names need localisation")?,
        ))
    };
    let (definition, texts) = match reach() {
        Ok(reached) => reached,
        Err(why) => {
            label.failures.push(why);
            return (label, None);
        }
    };
    label.reached = true;
    label.group = definition.group.clone();
    let cat = texts.cat();
    let render =
        |bound: &installed_content_core::gui::native::BoundLocalization| -> Result<String, String> {
            if bound.is_empty() {
                return Ok(String::new());
            }
            let text =
                faction_names::bound_name(bound, &cat.localisation, |name| lexical.token_of(name))?;
            let text = if text.contains('[') {
                texts.expand(&text)?
            } else {
                text
            };
            catalogue::plain_text(&text)
        };
    match render(&definition.name) {
        Ok(text) => label.name = Some(text),
        Err(reason) => label.failures.push(reason),
    };
    match render(&definition.description) {
        Ok(text) => label.description = Some(text),
        Err(reason) => label.failures.push(reason),
    };
    match definition
        .category
        .as_deref()
        .map(|value| std::str::from_utf8(value).map(str::to_owned))
        .transpose()
    {
        Ok(category) => label.category = category,
        Err(_) => label
            .failures
            .push("Faction goal category is not UTF-8".into()),
    }
    label.category_name = label
        .category
        .as_deref()
        .and_then(|category| term_name(&cat.localisation, category));
    (label, Some(definition))
}
/// A goal as faction details show it without its checks, artwork refusals first.
pub fn finish_goal(label: &GoalLabel, cat: Option<&catalogue::Catalogue>) -> FactionGoalDefinition {
    let mut failures = Vec::new();
    let mut icon = None;
    if label.reached {
        match cat.map(|cat| goal_icon(cat, label.group.as_deref())) {
            Some(Ok(drawn)) => icon = drawn,
            Some(Err(why)) => failures.push(why),
            None => failures.push("Faction goal artwork needs the recorded content".into()),
        }
    }
    failures.extend(label.failures.iter().cloned());
    FactionGoalDefinition {
        progress: Vec::new(),
        sections: Vec::new(),
        bands: Vec::new(),
        conditions: Vec::new(),
        rewards: Vec::new(),
        flags: Vec::new(),
        token: label.token.clone(),
        name: label.name.clone(),
        description: label.description.clone(),
        icon,
        category: label.category.clone(),
        category_name: label.category_name.clone(),
        reason: (!failures.is_empty()).then(|| failures.join("; ")),
    }
}
fn definition(
    frame: &DecodedFrame,
    token: Option<u32>,
    texts: Option<&presentations::Texts<'_>>,
    definitions: Option<&installed_content_core::faction_goals::Catalog>,
    lexical: &faction_names::LexicalNames,
) -> FactionGoalDefinition {
    let (label, reached) = label(
        HourFrame::from(frame),
        token,
        texts.map(|texts| texts as &dyn GoalText),
        definitions,
        lexical,
    );
    let mut reply = finish_goal(&label, texts.map(presentations::Texts::cat));
    if let (Some(definition), Some(texts)) = (reached, texts) {
        presentations::attach(texts, definition, &mut reply);
    }
    reply
}
/// Each token's goal label at the frame's hour, as faction details label it.
pub fn goal_labels(
    frame: &DecodedFrame,
    faction: u64,
    cat: Option<&catalogue::Catalogue>,
    tokens: impl IntoIterator<Item = Option<u32>>,
) -> Vec<GoalLabel> {
    let definitions =
        cat.and_then(|cat| installed_content_core::faction_goals::cached(&cat.content.layers).ok());
    let lexical =
        faction_names::LexicalNames::new(frame.tables[TableId::LexicalTokens.index()].clone());
    let defines = SharedDefines::default();
    let texts = cat.map(|cat| presentations::Texts::new(frame, faction, cat, &defines));
    tokens
        .into_iter()
        .map(|token| {
            label(
                HourFrame::from(frame),
                token,
                texts.as_ref().map(|texts| texts as &dyn GoalText),
                definitions.as_deref(),
                &lexical,
            )
            .0
        })
        .collect()
}
/// [`goal_labels`] of any frame's tables: a walk's scripted texts expand in the selected
/// country's text, as a decoded frame's do.
pub fn goal_labels_in(
    frame: HourFrame<'_>,
    faction: u64,
    cat: Option<&catalogue::Catalogue>,
    tokens: impl IntoIterator<Item = Option<u32>>,
) -> Vec<GoalLabel> {
    if let Some(decoded) = frame.decoded() {
        return goal_labels(decoded.decoded_frame(), faction, cat, tokens);
    }
    let definitions =
        cat.and_then(|cat| installed_content_core::faction_goals::cached(&cat.content.layers).ok());
    let lexical = faction_names::LexicalNames::from_tables(frame);
    let viewer = cat.map(|cat| ViewerText::new_in(frame, cat));
    let texts = viewer
        .as_ref()
        .map(|held| HeldGoalText { faction, held });
    tokens
        .into_iter()
        .map(|token| {
            label(
                frame,
                token,
                texts.as_ref().map(|texts| texts as &dyn GoalText),
                definitions.as_deref(),
                &lexical,
            )
            .0
        })
        .collect()
}
/// `token`'s goal label at the frame's hour, as [`goal_labels`] labels it, its scripted texts
/// expanded in `held`, the shared text of `frame` and `cat`.
pub fn goal_label_in(
    frame: HourFrame<'_>,
    faction: u64,
    cat: Option<&catalogue::Catalogue>,
    token: Option<u32>,
    held: &ViewerText<'_>,
) -> GoalLabel {
    let definitions =
        cat.and_then(|cat| installed_content_core::faction_goals::cached(&cat.content.layers).ok());
    let lexical = faction_names::LexicalNames::from_tables(frame);
    let texts = cat.map(|_| HeldGoalText { faction, held });
    label(
        frame,
        token,
        texts.as_ref().map(|texts| texts as &dyn GoalText),
        definitions.as_deref(),
        &lexical,
    )
    .0
}
/// The goal content's own refusals, as faction details report them.
pub fn goal_content_warnings(cat: &catalogue::Catalogue) -> Vec<String> {
    match installed_content_core::faction_goals::cached(&cat.content.layers) {
        Ok(definitions) => definitions.warnings.clone(),
        Err(reason) => vec![reason],
    }
}
/// Whether a goal's texts expand scripted text; None while its spelling is unread.
pub fn goal_label_expands(
    frame: HourFrame<'_>,
    cat: &catalogue::Catalogue,
    token: u32,
) -> Option<bool> {
    let Ok(definitions) = installed_content_core::faction_goals::cached(&cat.content.layers) else {
        return Some(false);
    };
    let spelling = faction_names::template_spelling_in(&frame, Some(token)).ok()??;
    let Ok(Some(definition)) = definitions.get(&spelling) else {
        return Some(false);
    };
    let lexical = faction_names::LexicalNames::from_tables(frame);
    Some(expands(definition, cat, &lexical))
}
fn expands(
    definition: &installed_content_core::faction_goals::Definition,
    cat: &catalogue::Catalogue,
    lexical: &faction_names::LexicalNames,
) -> bool {
    [&definition.name, &definition.description]
        .into_iter()
        .any(|bound| {
            !bound.is_empty()
                && faction_names::bound_name(bound, &cat.localisation, |name| {
                    lexical.token_of(name)
                })
                .is_ok_and(|text| text.contains('['))
        })
}
/// One faction's goal lists at an hour, each kind's ordinals proven contiguous.
pub struct HeldGoals<'a> {
    pub manifest: Option<u32>,
    /// Ordered by list kind, then ordinal.
    pub entries: &'a [replay_data::rows::FactionGoalEntryRow],
}
pub fn held_goals(frame: &DecodedFrame, faction: u64) -> io::Result<HeldGoals<'_>> {
    let headers = frame.complete::<tables::FactionGoalStatus>()?;
    let entries = frame.complete::<tables::FactionGoalEntries>()?;
    let header = headers
        .binary_search_by_key(&faction, |row| row.faction)
        .ok()
        .map(|index| &headers[index])
        .ok_or_else(|| invalid("Faction goal status is absent"))?;
    let entries = companions::selected(entries, faction, |row| row.faction);
    contiguous(entries.iter())?;
    Ok(HeldGoals {
        manifest: header.manifest,
        entries,
    })
}
/// [`HeldGoals`] of any frame's tables, a walk's entries built.
pub struct HeldGoalsIn<'a> {
    pub manifest: Option<u32>,
    /// Ordered by list kind, then ordinal.
    pub entries: Vec<std::borrow::Cow<'a, replay_data::rows::FactionGoalEntryRow>>,
}
/// [`held_goals`] of any frame's tables, a walk's views among them.
pub fn held_goals_in(frame: HourFrame<'_>, faction: u64) -> io::Result<HeldGoalsIn<'_>> {
    let headers = frame.complete::<tables::FactionGoalStatus>(u64::MAX)?;
    let entries = frame.complete::<tables::FactionGoalEntries>(u64::MAX)?;
    let header = headers
        .get(u128::from(faction))?
        .ok_or_else(|| invalid("Faction goal status is absent"))?;
    // An entry's key holds its faction above its list kind and ordinal.
    let low = u128::from(faction) << 40;
    let entries = entries
        .span(low, low | ((1_u128 << 40) - 1))
        .iter()
        .collect::<io::Result<Vec<_>>>()?;
    contiguous(entries.iter().map(|row| &**row))?;
    Ok(HeldGoalsIn {
        manifest: header.manifest,
        entries,
    })
}
/// Refuses a faction's goal entries unless each kind's ordinals count up from zero.
fn contiguous<'r>(
    entries: impl Iterator<Item = &'r replay_data::rows::FactionGoalEntryRow>,
) -> io::Result<()> {
    let mut next = [0; 5];
    for row in entries {
        let expected = next
            .get_mut(row.kind as usize)
            .ok_or_else(|| invalid("Unknown faction goal list kind"))?;
        if row.ordinal != *expected {
            return Err(invalid("Faction goal ordinals are not contiguous"));
        }
        *expected += 1;
    }
    Ok(())
}
fn read(
    frame: &DecodedFrame,
    params: &FactionDetailParams,
    faction: u64,
    mut definition: impl FnMut(Option<u32>) -> FactionGoalDefinition,
) -> io::Result<FactionDetailReply> {
    let held = held_goals(frame, faction)?;
    let mut reply = FactionDetailReply {
        branch_id: params.branch_id.clone(),
        hour: params.hour,
        faction: params.faction.clone(),
        manifest: held.manifest.map(|token| definition(Some(token))),
        goals: Vec::new(),
        completed: Vec::new(),
        cancelled: Vec::new(),
        extra_slots: Vec::new(),
        term_dates: Vec::new(),
        warnings: Vec::new(),
        ..Default::default()
    };
    for row in held.entries {
        match row.kind {
            0 => reply.goals.push(FactionGoalDetail {
                ordinal: row.ordinal,
                definition: definition(row.token),
                status: row.status,
                progress_raw: row.progress_raw.to_string(),
                active_sections: row
                    .active_sections
                    .iter()
                    .map(|index| *index as i32)
                    .collect(),
                progress_percent: super::faction_goal_progress_percent(row.progress_raw),
                date: row.date,
            }),
            1 => reply.completed.push(definition(row.token)),
            2 => reply.cancelled.push(definition(row.token)),
            3 => reply.extra_slots.push(row.slots),
            4 => reply.term_dates.push(row.date),
            _ => unreachable!(),
        }
    }
    let mut refusals = std::collections::BTreeMap::<String, usize>::new();
    for definition in reply
        .manifest
        .iter()
        .chain(reply.goals.iter().map(|goal| &goal.definition))
        .chain(&reply.completed)
        .chain(&reply.cancelled)
    {
        for reason in definition
            .conditions
            .iter()
            .chain(&definition.flags)
            .flat_map(|condition| &condition.reasons)
            .chain(
                definition
                    .progress
                    .iter()
                    .chain(&definition.sections)
                    .chain(&definition.rewards)
                    .flat_map(|reward| &reward.reasons),
            )
        {
            *refusals.entry(reason.clone()).or_default() += 1;
        }
        if let Some(reason) = &definition.reason {
            *refusals.entry(reason.clone()).or_default() += 1;
        }
    }
    reply.warnings.extend(
        refusals
            .into_iter()
            .map(|(reason, count)| format!("{count} faction goal texts: {reason}")),
    );
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{FactionGoalEntryRow, FactionGoalStatusRow, FactionRow, TableRows};
    use replay_frame::frame::TableView;
    #[test]
    fn goal_term_names_come_from_the_recorded_localisation() {
        let localisation: installed_content_localisation::localisation::Localisation = [
            ("SHORT_TERM".to_owned(), "A mod's short horizon".to_owned()),
            ("LONG_TERM".to_owned(), String::new()),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            super::term_name(&localisation, "short_term").as_deref(),
            Some("A mod's short horizon")
        );
        assert_eq!(
            super::term_name(&localisation, "long_term"),
            None,
            "an empty name is no name"
        );
        assert_eq!(
            super::term_name(&localisation, "medium_term"),
            None,
            "an undefined key stays unnamed"
        );
    }

    #[test]
    fn faction_goal_details_select_one_owner_and_preserve_independent_lists() {
        let mut frame = crate::test_fixtures::source(1, 100, Some(0));
        let selected = u64::MAX;
        *frame.tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(
            (1..24)
                .chain([selected])
                .map(|id| FactionRow {
                    id,
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::FactionGoalStatus.index()] =
            TableView::Complete(TableRows::FactionGoalStatus(
                (1..24)
                    .chain([selected])
                    .map(|faction| FactionGoalStatusRow {
                        faction,
                        manifest: (faction == selected).then_some(7),
                    })
                    .collect(),
            ));
        *frame.tables[TableId::FactionGoalEntries.index()] =
            TableView::Complete(TableRows::FactionGoalEntries(vec![
                FactionGoalEntryRow {
                    faction: 1,
                    kind: 0,
                    token: Some(8),
                    ..Default::default()
                },
                FactionGoalEntryRow {
                    faction: selected,
                    kind: 0,
                    token: Some(9),
                    status: 4,
                    progress_raw: -25000,
                    date: 123,
                    ..Default::default()
                },
                FactionGoalEntryRow {
                    faction: selected,
                    kind: 1,
                    token: None,
                    ..Default::default()
                },
                FactionGoalEntryRow {
                    faction: selected,
                    kind: 3,
                    slots: -2,
                    ..Default::default()
                },
                FactionGoalEntryRow {
                    faction: selected,
                    kind: 4,
                    date: 456,
                    ..Default::default()
                },
            ]));
        let mut asked = Vec::new();
        let params = FactionDetailParams {
            branch_id: "1".into(),
            hour: 100,
            faction: selected.to_string(),
        };
        let reply = read(&frame, &params, selected, |token| {
            asked.push(token);
            FactionGoalDefinition {
                progress: Vec::new(),
                sections: Vec::new(),
                bands: Vec::new(),
                conditions: Vec::new(),
                rewards: Vec::new(),
                flags: Vec::new(),
                token: token.map(|id| id.to_string()),
                name: None,
                description: None,
                icon: None,
                category: None,
                category_name: None,
                reason: None,
            }
        })
        .unwrap();
        assert_eq!(asked, [Some(7), Some(9), None]);
        assert_eq!(reply.goals.len(), 1);
        assert_eq!(reply.goals[0].status, 4);
        assert_eq!(reply.goals[0].progress_percent, Some(-25.0));
        assert_eq!(reply.goals[0].progress_raw, "-25000");
        assert_eq!(reply.completed.len(), 1);
        assert_eq!(reply.extra_slots, [-2]);
        assert_eq!(reply.term_dates, [456]);
        *frame.tables[TableId::FactionGoalEntries.index()] = TableView::Unavailable {
            reason: "writer not observed".into(),
        };
        assert!(read(&frame, &params, selected, |_| unreachable!()).is_err());
    }
}

#[cfg(test)]
mod artwork_tests {
    use super::*;
    #[test]
    fn faction_goal_missing_group_and_sprite_are_named_counted_refusals() {
        let (cat, _guard) = catalogue::tests::folder_catalogue(&[]);
        assert!(
            goal_icon(&cat, None)
                .unwrap_err()
                .contains("group is not authored")
        );
        let reason = goal_icon(&cat, Some(b"mod_missing; group")).unwrap_err();
        assert!(reason.contains("GFX_mod_missing; group"));
        let mut reply = FactionDetailReply::default();
        let goal: FactionGoalDefinition = serde_json::from_value(serde_json::json!({
            "progress":[], "sections":[], "bands":[], "conditions":[], "rewards":[], "flags":[],
            "token":"mod_goal", "name":"Still known", "description":null, "icon":null,
            "category":null, "categoryName":null, "reason":reason,
        }))
        .unwrap();
        reply.completed = vec![goal.clone(), goal.clone()];
        collect_icon_refusals(&mut reply);
        assert_eq!(reply.refusals.len(), 1);
        assert_eq!(reply.refusals[0].count, 2);
        assert!(reply.refusals[0].reason.contains("GFX_mod_missing; group"));
        assert_eq!(reply.completed[0].name.as_deref(), Some("Still known"));
    }
    #[test]
    fn a_finished_goal_names_artwork_before_its_texts_and_an_unreached_one_draws_none() {
        let (cat, _guard) = catalogue::tests::folder_catalogue(&[]);
        let reached = GoalLabel {
            token: Some("mod_goal".into()),
            name: Some("Mod goal".into()),
            reached: true,
            group: None,
            failures: vec!["description failed".into()],
            ..Default::default()
        };
        let shown = finish_goal(&reached, Some(&cat));
        assert_eq!(shown.name.as_deref(), Some("Mod goal"));
        let reason = shown.reason.unwrap();
        assert!(reason.starts_with("Faction goal artwork: group is not authored"));
        assert!(reason.ends_with("; description failed"));
        let unreached = GoalLabel {
            token: Some("mod_goal".into()),
            failures: vec!["Faction goal names need localisation".into()],
            ..Default::default()
        };
        let shown = finish_goal(&unreached, Some(&cat));
        assert!(shown.icon.is_none());
        assert_eq!(
            shown.reason.as_deref(),
            Some("Faction goal names need localisation")
        );
        assert!(finish_goal(&GoalLabel::default(), None).reason.is_none());
    }
    /// A scripted completed account goal alone selects the text environment.
    #[test]
    fn a_scripted_completed_account_goal_selects_the_text_environment() {
        use replay_data::rows::{FactionAccountRow, LexicalTokenRow, TableRows};
        use replay_frame::frame::TableView;
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (cat, _folder) = catalogue::tests::folder_catalogue(&[
            (
                "common/factions/goals/mod.txt",
                "mod_plain = { name = MOD_PLAIN category = long_term }\nmod_scripted = { name = MOD_SCRIPTED category = long_term }\n",
            ),
            (
                "localisation/mod_l_english.yml",
                "\u{feff}l_english:\n MOD_PLAIN:0 \"Plain pact goal\"\n MOD_SCRIPTED:0 \"[ROOT.GetTag] pact goal\"\n",
            ),
        ]);
        let definitions = installed_content_core::faction_goals::cached(&cat.content.layers).unwrap();
        let mut frame = crate::test_fixtures::source(1, 100, Some(0));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(
                ["mod_plain", "mod_scripted"]
                    .into_iter()
                    .zip(1..)
                    .map(|(literal, token)| LexicalTokenRow {
                        token,
                        literal: literal.as_bytes().to_vec(),
                    })
                    .collect(),
            ));
        let lexical =
            faction_names::LexicalNames::new(frame.tables[TableId::LexicalTokens.index()].clone());
        for (completed, expected) in [(vec![1, u64::MAX], false), (vec![1, 2], true)] {
            *frame.tables[TableId::FactionAccounts.index()] =
                TableView::Complete(TableRows::FactionAccounts(vec![FactionAccountRow {
                    kind: 0,
                    country: 9001,
                    completed_goals: completed.clone(),
                    ..Default::default()
                }]));
            assert_eq!(
                needs_environment(&frame, 7, &cat, &definitions, &lexical),
                expected,
                "{completed:?}"
            );
        }
    }
    #[test]
    fn goal_labels_without_content_keep_the_token_and_the_detail_refusal() {
        use replay_data::rows::{LexicalTokenRow, TableRows};
        use replay_frame::frame::TableView;
        let mut frame = crate::test_fixtures::source(1, 100, Some(0));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 1,
                literal: b"mod_goal".to_vec(),
            }]));
        let labels = goal_labels(&frame, 9, None, [Some(1), None, Some(2)]);
        let lexical =
            faction_names::LexicalNames::new(frame.tables[TableId::LexicalTokens.index()].clone());
        for (label, token) in labels.iter().zip([Some(1), None, Some(2)]) {
            assert_eq!(
                serde_json::to_value(finish_goal(label, None)).unwrap(),
                serde_json::to_value(definition(&frame, token, None, None, &lexical)).unwrap()
            );
        }
        assert_eq!(labels[0].token.as_deref(), Some("mod_goal"));
        assert_eq!(
            labels[0].failures,
            ["Faction goal names need the recorded game folders"]
        );
        assert_eq!(labels[1], GoalLabel::default());
        assert!(labels[2].failures[0].contains("no LexicalTokens spelling"));
    }
    #[test]
    #[ignore = "requires DXC and a GPU; synthetic mod content only"]
    fn faction_goal_mod_group_sprite_resolves_through_shared_artwork() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let (cat, guards) = catalogue::texture_tests::texture_catalogue(
            &[
                (
                    "interface/art.gfx",
                    "spriteTypes={spriteType={name=GFX_mod_group textureFile=gfx/mod.png}}",
                ),
                (
                    "common/factions/goals/mod.txt",
                    "mod_goal={name=MOD_NAME group=mod_group} missing_art={name=MOD_NAME group=missing_mod_group}",
                ),
                (
                    "localisation/mod_l_english.yml",
                    "l_english:\n MOD_NAME:0 \"Mod goal\"\n",
                ),
            ],
            &[(
                "gfx/mod.png",
                catalogue::texture_tests::png_bytes(&installed_content_image::Rgba8Image {
                    width: 1,
                    height: 1,
                    pixels: vec![30, 60, 90, 255],
                }),
            )],
        );
        catalogue::texture_tests::write_sprite_program(&guards[1].0);
        use replay_data::rows::{LexicalTokenRow, TableRows};
        use replay_frame::frame::TableView;
        let definitions = installed_content_core::faction_goals::cached(&cat.content.layers).unwrap();
        let mut frame = crate::test_fixtures::source(1, 100, Some(0));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(
                ["mod_goal", "missing_art"]
                    .into_iter()
                    .enumerate()
                    .map(|(index, token)| LexicalTokenRow {
                        token: index as u32 + 1,
                        literal: token.as_bytes().to_vec(),
                    })
                    .collect(),
            ));
        let lexical =
            faction_names::LexicalNames::new(frame.tables[TableId::LexicalTokens.index()].clone());
        let defines = SharedDefines::default();
        let texts = presentations::Texts::new(&frame, 9, &cat, &defines);
        let resolved = definition(&frame, Some(1), Some(&texts), Some(&definitions), &lexical);
        assert!(dashboard_text::artwork::url_bytes(resolved.icon.as_ref().unwrap()).is_some());
        assert_eq!(resolved.name.as_deref(), Some("Mod goal"));
        let unresolved = definition(&frame, Some(2), Some(&texts), Some(&definitions), &lexical);
        assert!(unresolved.icon.is_none());
        assert_eq!(unresolved.name.as_deref(), Some("Mod goal"));
        let mut reply = FactionDetailReply {
            completed: vec![resolved, unresolved],
            ..Default::default()
        };
        collect_icon_refusals(&mut reply);
        assert_eq!(reply.refusals.len(), 1);
        assert_eq!(reply.refusals[0].count, 1);
        assert!(reply.refusals[0].reason.contains("GFX_missing_mod_group"));
    }
}
