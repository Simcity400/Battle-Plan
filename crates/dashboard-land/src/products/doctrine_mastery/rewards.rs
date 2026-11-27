
use dashboard_base::products::{Country, validate_country};
use super::{LoadContext, Owner, spelled};
use dashboard_base::{Source, catalogue::{self, Catalogue, View}, products::at_hour::{At, AtHour, Read, ScriptedInputs}};
use dashboard_effects::{ScriptedEffects, effect_description::{self, Context as Describing}};
use dashboard_text::{scripted_text::{EventObject, EventScope, EventScopes, ScriptedText}};
use content_reader_military::doctrines::{Database, DefinitionSources};
use content_reader_parser::tokens::Token;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io, sync::Arc};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct DoctrineRewards;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DoctrineRewardsParams {
    pub branch_id: String,
    pub country: Country,
    #[doc = " The folder's and the track's ordinals, as `doctrine-mastery` lists them."]
    pub folder: u32,
    pub track: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DescribedReward {
    pub id: String,
    pub name: Option<String>,
    pub threshold: Option<String>,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DoctrineRewardsReply {
    #[serde(skip)]
    through: i32,
    #[doc = " The track's subdoctrine's spelling at the hour; none where none is chosen."]
    pub sub: Option<String>,
    pub rewards: Vec<DescribedReward>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(DoctrineRewardsReply);

pub struct Context {
    catalogue: Arc<Catalogue>,
    database: Arc<Database>,
    default_reward: Result<i64, String>,
}

fn tables() -> BTreeSet<TableId> {
    let mut out: BTreeSet<TableId> = effect_description::recorded_tables().collect();
    out.extend([TableId::DoctrineFolders, TableId::DoctrineTracks, TableId::LexicalTokens]);
    out
}

pub(super) fn reward_key(sub: &[u8], reward: &[u8]) -> Vec<u8> {
    let mut key = content_reader_parser::parser::c_string(sub).to_vec();
    key.push(b'_');
    key.extend_from_slice(content_reader_parser::parser::c_string(reward));
    key
}

fn benefits(reward: &content_reader_military::doctrines::RewardSource, key: &[u8], catalogue: &Catalogue) -> content_reader_military::doctrines::Benefits {
    let mut benefits = reward.benefits.clone();
    let mut description = key.to_vec();
    description.extend_from_slice(b"_desc");
    if benefits.prose.is_none() && std::str::from_utf8(&description).ok().and_then(|key| catalogue.localisation.get(key)).is_some() {
        benefits.prose = Some(content_reader_script::localization_binding::BoundLocalization::from_value(&content_reader_parser::parser::Value::Scalar(description)));
    }
    benefits
}

impl AtHour for DoctrineRewards {
    const KEY: &'static str = "doctrine-rewards";
    const TITLE: &'static str = "Doctrine rewards";
    const CONTENT_DEPENDENT: bool = true;
    type Params = DoctrineRewardsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, folder, track];
    type Reply = DoctrineRewardsReply;
    type Context = (Context, ScriptedInputs);

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![DoctrineRewardsParams::decl(), DescribedReward::decl(), DoctrineRewardsReply::decl()]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        tables().into_iter().collect()
    }

    fn reads(params: &Self::Params, owner: Option<Owner>, step: usize, frame: &DecodedFrame) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        match step {
            0 => vec![Read::these(TableId::DoctrineTracks, [u128::from(owner.slot) << 64 | u128::from(params.folder) << 32 | u128::from(params.track)])],
            1 => {
                let token = frame.complete::<tables::DoctrineTracks>().ok().and_then(|rows| rows.first()?.sub_doctrine);
                vec![Read::these(TableId::LexicalTokens, token.map(u128::from))]
            }
            _ => Vec::new(),
        }
    }

    fn columns(table: TableId) -> Option<Vec<usize>> {
        (table == TableId::DoctrineTracks).then(|| dashboard_base::products::at_hour::field_columns(table, &["sub_doctrine"]))
    }

    fn holds_refusal(frame: &DecodedFrame) -> bool {
        frame.complete::<tables::DoctrineTracks>().is_ok() && frame.complete::<tables::LexicalTokens>().is_ok()
    }

    fn valid_reply_through(frame: &DecodedFrame, _params: &Self::Params, _at: &At<'_, Self::Context>, reply: &Self::Reply, _partial: bool) -> i32 {
        reply.through.min(dashboard_base::products::at_hour::warnings_through(frame.hour, reply.warnings.iter().map(String::as_str)))
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        let options = load.content.as_ref().ok_or_else(|| dashboard_core::refused("Doctrine rewards need the recorded content stack"))?;
        let resolved = source.content(&options.options)?;
        Ok((Context { database: crate::products::doctrines::database(source, &resolved)?, default_reward: super::default_reward(source, &resolved), catalogue: catalogue::catalogue(source, options)? }, ScriptedInputs::new(source.replay())))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Doctrine rewards country is unavailable at this hour"))?;
        let (context, inputs) = at.context;
        let reply = rewards(frame, params, owner.slot, context, |token, source, warnings| {
            let tables: Vec<_> = effect_description::recorded_tables().collect();
            let (reply, through) = inputs.evaluate(frame, &tables, at.cancelled, |world| describe(world, owner.slot, context, token, source, warnings))?;
            Ok(DoctrineRewardsReply { through, ..reply? })
        })?;
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

fn rewards(frame: &DecodedFrame, params: &DoctrineRewardsParams, slot: u32, context: &Context,
    present: impl FnOnce(&[u8], &DefinitionSources, Vec<String>) -> io::Result<DoctrineRewardsReply>,
) -> io::Result<DoctrineRewardsReply> {
    let unrecorded = |error: io::Error| dashboard_core::refused(format!("Doctrine rewards are not recorded at this hour: {error}"));
    let track = frame
        .complete::<tables::DoctrineTracks>()
        .map_err(unrecorded)?
        .iter()
        .find(|row| row.slot == slot && row.folder == params.folder && row.ordinal == params.track)
        .cloned()
        .ok_or_else(|| dashboard_core::refused("This doctrine track is not recorded for the country at this hour"))?;
    let Some(sub) = track.sub_doctrine else { return Ok(DoctrineRewardsReply { through: i32::MAX, sub: None, rewards: Vec::new(), warnings: Vec::new() }) };
    let tokens = frame.complete::<tables::LexicalTokens>().map_err(unrecorded)?;
    let token = tokens
        .iter()
        .find(|row| row.token == sub)
        .map(|row| row.literal.clone())
        .ok_or_else(|| dashboard_core::refused(format!("Doctrine token {sub} is not recorded")))?;
    let mut warnings = context.database.notes.clone();
    let Some(source) = context.database.tactic_sources.sub.get(&Token::c_str(&token)) else {
        warnings.push(format!("1 subdoctrine's rewards are not in the content: {}", spelled(&token)));
        return Ok(DoctrineRewardsReply { through: i32::MAX, sub: Some(spelled(&token)), rewards: Vec::new(), warnings });
    };
    present(&token, source, warnings)
}

fn describe(frame: &DecodedFrame, slot: u32, context: &Context, token: &[u8], source: &DefinitionSources, mut warnings: Vec<String>) -> io::Result<DoctrineRewardsReply> {
    let catalogue = &context.catalogue;
    let content = catalogue.scripted_content().map_err(dashboard_core::invalid)?;
    let scripted = ScriptedText::new(frame, &catalogue.localisation, content, slot);
    let describing = Describing::new(&scripted, &catalogue.localisation, slot, "doctrine benefits", content.effects().map_err(dashboard_core::invalid)?.view(None));
    let mut rewards = Vec::with_capacity(source.rewards.len());
    for reward in &source.rewards {
        let key = reward_key(token, &reward.token);
        let name = catalogue::localised_bytes(&catalogue.localisation, &key, View::Plain).map_err(|why| warnings.push(format!("1 doctrine reward name unavailable: {}: {why}", spelled(&key)))).ok();
        let threshold = match (reward.mastery, &context.default_reward) {
            (0, Ok(default)) => Some(default.to_string()),
            (0, Err(why)) => {
                warnings.push(format!("1 default reward mastery unavailable: {why}"));
                None
            }
            (mastery, _) => Some(mastery.to_string()),
        };
        let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(slot))]);
        let described = effect_description::doctrine_benefits(&benefits(reward, &key, catalogue), &describing, &mut scopes);
        warnings.extend(described.refusals.into_iter().map(|(why, count)| format!("{count} doctrine benefit descriptions unavailable: {why}")));
        let description = match catalogue::plain_text(&described.text) {
            Ok(text) => Some(text).filter(|text| !text.trim().is_empty()),
            Err(why) => {
                warnings.push(format!("1 doctrine benefit description unavailable: {why}"));
                None
            }
        };
        rewards.push(DescribedReward { id: spelled(&reward.token), name, threshold, description });
    }
    warnings.sort();
    warnings.dedup();
    Ok(DoctrineRewardsReply { through: i32::MAX, sub: Some(spelled(token)), rewards, warnings })
}

#[cfg(test)]
mod tests;
