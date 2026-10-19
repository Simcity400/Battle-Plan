//! One faction's retained goal state at the requested recorded hour.
//! Definitions and faction-scoped text use the recording's content stack.
use super::{LoadContext, Product, check_reply_budget};
use crate::{
    Source, catalogue, content, faction_names, invalid, metadata::validate_campaign,
    scripted_text::ScriptedText,
};
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
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
pub use accounts::{FactionAccountDetail, FactionUpgradeDetail};
pub use companions::{FactionManpowerDetail, FactionProgramDetail, FactionRuleDetail};
pub use pools::FactionResourceDetail;
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
    pub conditions: Vec<FactionConditionDetail>,
    pub rewards: Vec<FactionRewardDetail>,
    pub flags: Vec<FactionConditionDetail>,
    pub token: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
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
    pub accounts: Option<Vec<FactionAccountDetail>>,
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
        let mut selected = vec![
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
        let mut frame = crate::prepared::query::progressive::frame_at(
            source,
            branch,
            params.hour,
            &selected,
            cancelled,
        )?
        .ok_or_else(|| invalid("Faction details unavailable at this hour"))?;
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
            frame = crate::prepared::query::progressive::frame_at(
                source,
                branch,
                params.hour,
                &selected,
                cancelled,
            )?
            .ok_or_else(|| invalid("Faction details are unavailable at this hour"))?;
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
            match crate::prepared::query::progressive::latest_read(
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
            // Only reached scripted text needs the wider world. The shared
            // frame cache reads the missing tables alongside the held ones.
            selected.extend_from_slice(crate::scripted_text::TABLES);
            selected.sort_unstable();
            selected.dedup();
            frame = crate::prepared::query::progressive::frame_at(
                source,
                branch,
                params.hour,
                &selected,
                cancelled,
            )?
            .ok_or_else(|| invalid("Faction text unavailable at this hour"))?;
        }
        if let (Some(cat), Some(details)) = (cat.as_deref(), programme_frame.as_deref())
            && companions::programmes_need_environment(&frame, details, faction, cat)
        {
            let mut tables = companions::PROGRAMME_TABLES.to_vec();
            tables.extend_from_slice(crate::scripted_text::TABLES);
            tables.sort_unstable();
            tables.dedup();
            match crate::prepared::query::progressive::frame_at(
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
        let mut rendered = std::collections::BTreeMap::new();
        let mut reply = match read(&frame, params, faction, |token| {
            rendered
                .entry(token)
                .or_insert_with(|| {
                    definition(
                        &frame,
                        faction,
                        token,
                        cat.as_deref(),
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
            cat.as_deref(),
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
            &mut reply,
        )?;
        reply.warnings.extend(warnings);
        check_reply_budget(&reply)?;
        Ok(reply)
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
        {
            return true;
        }
        for bound in [&definition.name, &definition.description] {
            if !bound.is_empty()
                && faction_names::bound_name(bound, &cat.localisation, |name| {
                    lexical.token_of(name)
                })
                .is_ok_and(|text| text.contains('['))
            {
                return true;
            }
        }
    }
    false
}
fn definition(
    frame: &DecodedFrame,
    faction: u64,
    token: Option<u32>,
    cat: Option<&catalogue::Catalogue>,
    definitions: Option<&installed_content_core::faction_goals::Catalog>,
    lexical: &faction_names::LexicalNames,
) -> FactionGoalDefinition {
    let mut reply = FactionGoalDefinition {
        progress: Vec::new(),
        sections: Vec::new(),
        conditions: Vec::new(),
        rewards: Vec::new(),
        flags: Vec::new(),
        token: None,
        name: None,
        description: None,
        category: None,
        reason: None,
    };
    let Some(token) = token else {
        return reply;
    };
    let result = (|| -> Result<(), String> {
        let spelling = faction_names::template_spelling(frame, Some(token))?
            .ok_or("Faction goal token is absent")?;
        reply.token = Some(spelling.clone());
        let definition = definitions
            .ok_or("Faction goal names need the recorded game folders")?
            .get(&spelling)?
            .ok_or_else(|| format!("Faction goal {spelling:?} is absent from recorded content"))?;
        let cat = cat.ok_or("Faction goal names need localisation")?;
        let render =
            |bound: &installed_content_core::gui::native::BoundLocalization| -> Result<String, String> {
                if bound.is_empty() {
                    return Ok(String::new());
                }
                let text = faction_names::bound_name(bound, &cat.localisation, |name| {
                    lexical.token_of(name)
                })?;
                let text = if text.contains('[') {
                    let raw = frame
                        .complete::<tables::LocalPlayer>()
                        .map_err(|why| why.to_string())?
                        .first()
                        .and_then(|row| row.viewing_country())
                        .ok_or("Faction goal text needs the selected country")?;
                    let root = crate::metadata::link_slot(
                        frame
                            .complete::<tables::CountryLinks>()
                            .map_err(|why| why.to_string())?,
                        raw,
                    )
                    .ok_or("Faction goal text needs the selected country's identity")?;
                    ScriptedText::new(frame, &cat.localisation, cat.scripted_content()?, root)
                        .expand_faction(&text, faction)?
                } else {
                    text
                };
                catalogue::plain_text(&text)
            };
        let mut failures = Vec::new();
        presentations::attach(frame, faction, cat, definition, &mut reply);
        match render(&definition.name) {
            Ok(text) => reply.name = Some(text),
            Err(reason) => failures.push(reason),
        };
        match render(&definition.description) {
            Ok(text) => reply.description = Some(text),
            Err(reason) => failures.push(reason),
        };
        match definition
            .category
            .as_deref()
            .map(|value| std::str::from_utf8(value).map(str::to_owned))
            .transpose()
        {
            Ok(category) => reply.category = category,
            Err(_) => failures.push("Faction goal category is not UTF-8".into()),
        }
        if !failures.is_empty() {
            return Err(failures.join("; "));
        }
        Ok(())
    })();
    if let Err(reason) = result {
        reply.reason = Some(reason);
    }
    reply
}
fn read(
    frame: &DecodedFrame,
    params: &FactionDetailParams,
    faction: u64,
    mut definition: impl FnMut(Option<u32>) -> FactionGoalDefinition,
) -> io::Result<FactionDetailReply> {
    let headers = frame.complete::<tables::FactionGoalStatus>()?;
    let entries = frame.complete::<tables::FactionGoalEntries>()?;
    let header = headers
        .binary_search_by_key(&faction, |row| row.faction)
        .ok()
        .map(|index| &headers[index])
        .ok_or_else(|| invalid("Faction goal status is absent"))?;
    let mut reply = FactionDetailReply {
        branch_id: params.branch_id.clone(),
        hour: params.hour,
        faction: params.faction.clone(),
        manifest: header.manifest.map(|token| definition(Some(token))),
        goals: Vec::new(),
        completed: Vec::new(),
        cancelled: Vec::new(),
        extra_slots: Vec::new(),
        term_dates: Vec::new(),
        warnings: Vec::new(),
        ..Default::default()
    };
    let mut next = [0; 5];
    for row in companions::selected(entries, faction, |row| row.faction) {
        let expected = next
            .get_mut(row.kind as usize)
            .ok_or_else(|| invalid("Unknown faction goal list kind"))?;
        if row.ordinal != *expected {
            return Err(invalid("Faction goal ordinals are not contiguous"));
        }
        *expected += 1;
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
                progress_percent: super::finite_fixed64(row.progress_raw)
                    .then_some(row.progress_raw as f64 / 1000.0),
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
    fn faction_goal_details_select_one_owner_among_five_thousand_and_preserve_independent_lists() {
        let mut frame = crate::test_fixtures::source(1, 100, Some(0));
        let selected = u64::MAX;
        *frame.tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(
            (1..5000)
                .chain([selected])
                .map(|id| FactionRow {
                    id,
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::FactionGoalStatus.index()] =
            TableView::Complete(TableRows::FactionGoalStatus(
                (1..5000)
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
                conditions: Vec::new(),
                rewards: Vec::new(),
                flags: Vec::new(),
                token: token.map(|id| id.to_string()),
                name: None,
                description: None,
                category: None,
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
