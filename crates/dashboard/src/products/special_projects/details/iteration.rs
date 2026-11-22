use super::{CHARACTER_REFERENCE, COMPLETION_MARKER, Context, ProjectContent, Reads, branch, context, definition, effect_tables, owner_read, owners_of, project_slices, slice_read, spelled, supplied_project};
use crate::{
    Source,
    products::{
        LoadContext,
        at_hour::{At, AtHour, Owner, Read},
        game_text::{GameRun, runs},
    },
};
use dashboard_base::products::Country;
use dashboard_query::cost;
use replay::lanes::{self, Keys};
use replay_data::{
    cadence::STAGGER_PERIOD,
    rows::{ProjectHistoryRow, ProjectRow, TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct SpecialProjectIterationAt;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpecialProjectIterationParams {
    pub branch_id: String,
    pub country: Country,
    #[doc = " The project's raw owner id, its id within it and the iteration's history ordinal."]
    pub owner: u32,
    pub project: u32,
    pub ordinal: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct IterationSection {
    #[doc = " The game's heading over this part of the output; null where the game shows none."]
    pub heading: Option<Vec<GameRun>>,
    pub runs: Vec<GameRun>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpecialProjectIteration {
    #[doc = " The iteration's own hour: every value is read at it."]
    pub hour: i32,
    #[doc = " The game's heading over the whole output; null when nothing is described."]
    pub heading: Option<Vec<GameRun>>,
    #[doc = " What the chosen option gave, in the game's order."]
    pub sections: Vec<IterationSection>,
    pub no_changes: bool,
    #[doc = " Why parts of the output were withheld, with how often each was met."]
    pub refusals: BTreeMap<String, usize>,
    pub unavailable: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(SpecialProjectIteration);

fn history_key(params: &SpecialProjectIterationParams) -> (u32, u32, u32) {
    (params.owner, params.project, params.ordinal)
}

fn history_read(params: &SpecialProjectIterationParams) -> Read {
    Read::these(TableId::ProjectHistory, [u128::from(params.owner) << 64 | u128::from(params.project) << 32 | u128::from(params.ordinal)])
}

fn history_row(rows: &[ProjectHistoryRow], key: (u32, u32, u32)) -> Option<&ProjectHistoryRow> {
    rows.binary_search_by_key(&key, |row| (row.owner, row.project, row.ordinal)).ok().map(|at| &rows[at])
}

fn later_history(reads: &Reads<'_, '_>, params: &SpecialProjectIterationParams) -> Result<ProjectHistoryRow, String> {
    let unread = "This iteration's choice was not read within a day of its hour";
    let recording = reads.query.replay.recording();
    let last = recording.run(reads.run).map_err(|why| why.to_string())?.last_hour();
    let until = reads.hour.saturating_add(STAGGER_PERIOD - 1).min(last);
    if until <= reads.hour {
        return Err(unread.into());
    }
    let key = slice_read(TableId::ProjectHistory, params.owner);
    let columns = super::super::read_columns().map_err(|why| why.to_string())?;
    let stamps = cost::storage(|| lanes::history(recording, reads.run, TableId::SliceReads, &columns, Keys::These(&[key]), reads.hour + 1, until)).map_err(|why| why.to_string())?;
    let mut later = stamps.keys.iter().flat_map(|(_, steps)| steps).filter_map(|step| Some((step.hour, super::super::sample(step.cells.as_deref())?))).filter(|&(_, read)| read > reads.hour);
    let Some((shown, read)) = later.next() else { return Err(unread.into()) };
    if !reads.follows(read, until) {
        return Err("This iteration's choice was next read after this country stopped holding the tag".into());
    }
    reads.owned(params.owner, read)?;
    let frame = dashboard_base::products::at_hour::frame(reads.query, shown, &[history_read(params), Read::these(TableId::SliceReads, [key])], SpecialProjectIterationAt::TITLE)
        .map_err(|why| why.to_string())?;
    if owner_read(&frame, TableId::ProjectHistory, params.owner)? != read {
        return Err(unread.into());
    }
    let rows = frame.complete::<tables::ProjectHistory>().map_err(|why| format!("Project choice history is unavailable: {why}"))?;
    history_row(rows, history_key(params)).cloned().ok_or_else(|| unread.into())
}

fn chosen<'c>(content: &'c ProjectContent, frame: &DecodedFrame, project: &str, row: &ProjectHistoryRow) -> Result<&'c content_reader_core::special_projects::PrototypeOption, String> {
    let reward = spelled(frame, row.prototype_reward_handle)?;
    let reward = content.definitions.reward_named(project, &reward)?;
    let option = spelled(frame, row.option_handle)?;
    let mut options = reward.options.iter().filter(|candidate| candidate.token.eq_ignore_ascii_case(&option));
    let found = options.next().ok_or("Chosen option is absent from recorded content")?;
    if options.any(|other| other.output != found.output) {
        return Err("Several options of this reward share the chosen token".into());
    }
    Ok(found)
}

fn scopes(frame: &DecodedFrame, row: &ProjectHistoryRow) -> dashboard_text::effect_description::IterationScopes {
    let context = &row.context;
    let scientist = match context.character_reference {
        0 => Ok(None),
        reference if reference as u32 == CHARACTER_REFERENCE => Ok(Some((reference >> 32) as u32)),
        _ => Err("Unknown scientist reference type".to_owned()),
    };
    let facility = match u32::try_from(context.province) {
        Ok(0) | Err(_) => Ok(None),
        Ok(province) => frame
            .complete::<tables::Provinces>()
            .map_err(|why| format!("Provinces are unavailable: {why}"))
            .and_then(|rows| rows.iter().find(|row| row.id == province).map(|row| Some((province, row.state))).ok_or_else(|| "Provinces lacks the iteration's facility province".to_owned())),
    };
    dashboard_text::effect_description::IterationScopes {
        country_seed: context.country_reference,
        scientist,
        scientist_seed: context.scientist_reference,
        facility,
        state_seed: context.state_reference,
    }
}

fn shown(text: &str, refusals: &mut BTreeMap<String, usize>) -> Option<Vec<GameRun>> {
    runs(text).map_err(|why| *refusals.entry(why).or_default() += 1).ok()
}

fn render(content: &ProjectContent, frame: &DecodedFrame, slot: u32, project: &ProjectRow, option: &content_reader_core::special_projects::PrototypeOption, row: &ProjectHistoryRow, out: &mut SpecialProjectIteration) -> Result<(), String> {
    let catalogue = &content.politics.catalogue;
    let scripted = catalogue.scripted_content()?;
    let text = dashboard_text::scripted_text::ScriptedText::new(frame, &catalogue.localisation, scripted, slot);
    let cache = scripted.effects()?;
    let context = dashboard_text::effect_description::Context::new(&text, &catalogue.localisation, slot, "special-project iteration", cache.view(None)).with_entity_links();
    let description = dashboard_text::effect_description::project_iteration_description(&option.output, &context, project.owner, project.local_id, scopes(frame, row));
    out.no_changes = description.no_changes;
    let mut refusals = description.refusals;
    for section in &description.sections {
        let heading = match &section.heading {
            Some(heading) => match shown(heading, &mut refusals) {
                Some(heading) => Some(heading),
                None => continue,
            },
            None => None,
        };
        if let Some(runs) = shown(&section.text, &mut refusals) {
            out.sections.push(IterationSection { heading, runs });
        }
    }
    if !out.sections.is_empty() {
        out.heading = description.heading.as_deref().and_then(|heading| shown(heading, &mut refusals));
    }
    out.refusals = refusals;
    Ok(())
}

fn empty(hour: i32) -> SpecialProjectIteration {
    SpecialProjectIteration { hour, heading: None, sections: Vec::new(), no_changes: false, refusals: BTreeMap::new(), unavailable: None, warnings: Vec::new() }
}

fn answer(frame: &DecodedFrame, params: &SpecialProjectIterationParams, at: &At<'_, Context>, holder: Owner, out: &mut SpecialProjectIteration) -> io::Result<Result<(), String>> {
    if !owners_of(frame, holder.slot)?.contains(&params.owner) {
        return Ok(Err("The project's owner is not this country at this iteration's hour".into()));
    }
    let query = at.context.index.query(&at.context.replay, branch(&params.branch_id)?)?;
    let run = query.timeline.at(at.hour).ok_or_else(|| dashboard_core::refused("Special project iteration hour is not recorded"))?.run;
    let reads = Reads { query: &query, coverage: &at.context.coverage, links: &at.context.links, tag: &params.country, holder, run, hour: at.hour };
    let project = match supplied_project(frame, &reads, params.owner, params.project) {
        Ok((project, _)) => project,
        Err(why) => return Ok(Err(why)),
    };
    let held = match owner_read(frame, TableId::ProjectHistory, params.owner).ok().filter(|&read| reads.supplies(read)) {
        Some(read) => {
            if let Err(why) = reads.owned(params.owner, read) {
                return Ok(Err(why));
            }
            history_row(frame.complete::<tables::ProjectHistory>()?, history_key(params)).cloned()
        }
        None => None,
    };
    let row = match held.map_or_else(|| later_history(&reads, params), Ok) {
        Ok(row) => row,
        Err(why) => return Ok(Err(why)),
    };
    if row.prototype_reward_handle == COMPLETION_MARKER && row.option_handle == COMPLETION_MARKER {
        return Ok(Err("This history entry is the project's completion, not an iteration".into()));
    }
    if row.date != at.hour {
        return Ok(Err("The asked hour is not this iteration's hour".into()));
    }
    let content = at.context.content.as_ref().map_err(|why| dashboard_core::refused(why.clone()))?;
    let token = match spelled(frame, project.definition_id) {
        Ok(token) => token,
        Err(why) => return Ok(Err(why)),
    };
    if definition(content, &token).is_none() {
        return Ok(Err(format!("Definition {token} is absent from content")));
    }
    Ok(chosen(content, frame, &token, &row).and_then(|option| render(content, frame, holder.slot, project, option, &row, out)))
}

impl AtHour for SpecialProjectIterationAt {
    const KEY: &'static str = "special-project-iteration";
    const TITLE: &'static str = "Special project iteration";
    const CONTENT_DEPENDENT: bool = true;
    type Params = SpecialProjectIterationParams;
    type Reply = SpecialProjectIteration;
    type Context = Context;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        dashboard_base::products::validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![SpecialProjectIterationParams::decl(), SpecialProjectIteration::decl(), IterationSection::decl()]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        let mut tables = effect_tables();
        tables.extend([TableId::ProjectHistory, TableId::SliceReads]);
        tables
    }

    fn reads(params: &Self::Params, _owner: Option<Owner>, step: usize, _held: &DecodedFrame) -> Vec<Read> {
        if step != 0 {
            return Vec::new();
        }
        effect_tables().into_iter().map(Read::all).chain([history_read(params), project_slices(params.owner)]).collect()
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(context(source, load))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Special project iteration names no country"))?;
        let mut out = empty(at.hour);
        if let Err(why) = answer(frame, params, at, owner, &mut out)? {
            out = SpecialProjectIteration { unavailable: Some(why), ..empty(at.hour) };
        }
        Ok(out)
    }
}
