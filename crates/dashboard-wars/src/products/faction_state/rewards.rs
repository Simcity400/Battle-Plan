
use dashboard_base::{Source, catalogue::{self, Catalogue}, products::{LoadContext, at_hour::{At, AtHour, Owner, Read}}};
use dashboard_effects::{ScriptedEffects, effect_description::{self, Context as Describing}};
use dashboard_text::{metadata, scripted_text::{EventObject, EventScope, EventScopes, ScriptedText}};
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io, sync::Arc};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct FactionGoalRewards;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactionGoalRewardsParams {
    pub branch_id: String,
    #[doc = " The recorded faction id, as a decimal string."]
    pub faction: String,
    #[doc = " The goal's token, as `faction-details` names it."]
    pub goal: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionGoalRewardsReply {
    pub goal: String,
    #[doc = " The completion effect's description with the game's markup; none where the goal authors no"]
    #[doc = " completion effect or `reason` says why it cannot be described."]
    pub text: Option<String>,
    #[doc = " Why the completion effect cannot be described at the hour."]
    pub reason: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(FactionGoalRewardsReply);

const MAX_GOAL: usize = 256;

const COMPLETION: &str = "complete_effect";

pub(super) fn tables() -> BTreeSet<TableId> {
    let mut out: BTreeSet<TableId> = effect_description::recorded_tables().collect();
    out.extend([TableId::FactionMembers, TableId::CountryLinks]);
    out
}

impl AtHour for FactionGoalRewards {
    const KEY: &'static str = "faction-goal-rewards";
    const TITLE: &'static str = "Faction goal rewards";
    const CONTENT_DEPENDENT: bool = true;
    type Params = FactionGoalRewardsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, faction, goal];
    type Reply = FactionGoalRewardsReply;
    type Context = Arc<Catalogue>;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid faction branch")?;
        super::faction_id(&params.faction)?;
        if params.goal.is_empty() || params.goal.len() > MAX_GOAL {
            return Err("Invalid faction goal");
        }
        Ok(())
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![FactionGoalRewardsParams::decl(), FactionGoalRewardsReply::decl()]
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        tables().into_iter().collect()
    }

    fn reads(params: &Self::Params, _owner: Option<Owner>, step: usize, _held: &DecodedFrame) -> Vec<Read> {
        if step > 0 {
            return Vec::new();
        }
        Self::tables(params).into_iter().map(Read::all).collect()
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        let options = load.content.as_ref().ok_or_else(|| dashboard_core::refused("Faction goal rewards need the recorded content stack"))?;
        catalogue::catalogue(source, options)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let faction = super::faction_id(&params.faction).map_err(dashboard_core::invalid)?;
        let reply = rewards(frame, faction, &params.goal, at.context)?;
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

fn leader(frame: &DecodedFrame, faction: u64) -> Result<u32, String> {
    let raw = frame
        .complete::<tables::FactionMembers>()
        .map_err(|why| format!("Faction rosters: {why}"))?
        .iter()
        .find(|member| member.faction == faction && member.ordinal == 0)
        .ok_or("The faction has no leader at this hour")?
        .country;
    let links = frame.complete::<tables::CountryLinks>().map_err(|why| format!("Country links: {why}"))?;
    metadata::link_slot(links, raw).ok_or_else(|| format!("The faction's leader (country {raw}) has no link at this hour"))
}

pub(super) fn rewards(frame: &DecodedFrame, faction: u64, goal: &str, cat: &Catalogue) -> io::Result<FactionGoalRewardsReply> {
    let mut reply = FactionGoalRewardsReply { goal: goal.to_owned(), text: None, reason: None, warnings: Vec::new() };
    let definitions = content_reader_politics::faction_goals::cached(&cat.content.layers).map_err(dashboard_core::refused)?;
    let definition = definitions
        .get(goal)
        .map_err(dashboard_core::refused)?
        .ok_or_else(|| dashboard_core::refused(format!("Faction goal {goal:?} is absent from recorded content")))?;
    let Some(body) = definition.effects.get(COMPLETION).filter(|body| !body.is_empty()) else { return Ok(reply) };
    let described = (|| -> Result<effect_description::Description, String> {
        let slot = leader(frame, faction)?;
        let content = cat.scripted_content()?;
        let scripted = ScriptedText::new(frame, &cat.localisation, content, slot);
        let describing = Describing::new(&scripted, &cat.localisation, slot, "faction goal", content.effects()?.view(None));
        let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(slot))]);
        Ok(effect_description::describe(body, &describing, &mut scopes))
    })();
    match described {
        Ok(described) => {
            reply.text = Some(described.text).filter(|text| !text.trim().is_empty());
            if reply.text.is_none() && !described.refusals.is_empty() {
                reply.reason = Some(described.refusals.iter().map(|(why, _)| why.as_str()).collect::<Vec<_>>().join("; "));
            }
            reply.warnings.extend(described.refusals.into_iter().map(|(why, count)| format!("{count} faction goal reward descriptions unavailable: {why}")));
        }
        Err(why) => reply.reason = Some(why),
    }
    Ok(reply)
}
