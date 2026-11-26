use dashboard_effects::ScriptedEffects;
use dashboard_base::{
    Source,
    products::{
        LoadContext,
        at_hour::{At, AtHour, Clock, ScriptedInputs, warnings_through},
        game_text::{GameRun, runs},
        politics_content::{PoliticsContent, politics_content},
    },
};
use dashboard_base::products::Country;
use replay_data::rows::TableId;
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::Arc};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

pub struct TechnologyEffectsAt;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TechnologyEffectsParams {
    pub branch_id: String,
    pub country: Country,
    pub token: String,
    #[doc = " The template occurrence the research reads named; null for a technology named only by its token, as a game link names it."]
    pub database_index: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TechnologyEffects {
    #[serde(skip)]
    through: i32,
    pub hour: i32,
    #[doc = " The game's text for what researching the technology does, as it shows it; empty where nothing could be described."]
    pub runs: Vec<GameRun>,
    #[doc = " Why parts of the text were withheld, with how often each was met."]
    pub refusals: BTreeMap<String, usize>,
    pub unavailable: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(TechnologyEffects);

pub struct Context {
    inputs: ScriptedInputs,
    content: Result<Arc<PoliticsContent>, String>,
}

fn tables() -> Vec<TableId> {
    let mut tables: Vec<TableId> = dashboard_effects::effect_description::recorded_tables()
        .chain([TableId::TechnologyDefinitions, TableId::ResearchTechnologies, TableId::SimulationFeatures, TableId::CountryAppearance, TableId::LexicalTokens])
        .collect();
    tables.sort_unstable();
    tables.dedup();
    tables
}

impl AtHour for TechnologyEffectsAt {
    const KEY: &'static str = "technology-effects";
    const TITLE: &'static str = "Technology effects";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: Clock = Clock::Boundaries;
    const REPLY_DATES_REQUEST_HOUR: bool = true;
    type Params = TechnologyEffectsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, token, databaseIndex];
    type Reply = TechnologyEffects;
    type Context = Context;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        if params.token.is_empty() || params.database_index.is_some_and(|index| index <= 0) {
            return Err("Technology effects need a technology");
        }
        dashboard_base::products::validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![TechnologyEffectsParams::decl(), TechnologyEffects::decl(), GameRun::decl(), dashboard_base::products::game_text::EntityKind::decl()]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        Vec::new()
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        let content = match load.content.as_ref() {
            Some(options) => politics_content(source, options).map_err(|error| format!("Recorded technology content is unavailable: {error}")),
            None => Err("Technology effects need the recorded content stack".to_owned()),
        };
        Ok(Context { content, inputs: ScriptedInputs::new(source.replay()) })
    }

    fn valid_reply_through(frame: &DecodedFrame, _: &Self::Params, _: &At<'_, Self::Context>, reply: &Self::Reply, _: bool) -> i32 {
        reply.through.min(warnings_through(frame.hour, reply.refusals.keys().chain(&reply.warnings).chain(&reply.unavailable).map(String::as_str)))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Technology effects name no country"))?;
        let mut out = TechnologyEffects { through: i32::MAX, hour: at.hour, runs: Vec::new(), refusals: BTreeMap::new(), unavailable: None, warnings: Vec::new() };
        let content = match &at.context.content {
            Ok(content) => content,
            Err(why) => {
                out.unavailable = Some(why.clone());
                return Ok(out);
            }
        };
        let render = |frame: &DecodedFrame| -> Result<_, String> {
            let catalogue = &content.catalogue;
            let scripted = catalogue.scripted_content()?;
            let text = dashboard_text::scripted_text::ScriptedText::new(frame, &catalogue.localisation, scripted, owner.slot);
            let cache = scripted.effects()?;
            let context = dashboard_effects::effect_description::Context::new(&text, &catalogue.localisation, owner.slot, "technology effects", cache.view(None)).with_entity_links();
            Ok(dashboard_effects::effect_description::technology_description(&params.token, params.database_index, &context, owner.slot))
        };
        let (description, through) = at.context.inputs.evaluate(frame, &tables(), at.cancelled, render)?;
        out.through = through;
        match description {
            Ok(mut description) => {
                out.runs = runs(&description.text).unwrap_or_else(|why| {
                    *description.refusals.entry(why).or_default() += 1;
                    Vec::new()
                });
                out.refusals = description.refusals;
            }
            Err(why) => out.unavailable = Some(why),
        }
        Ok(out)
    }
}
