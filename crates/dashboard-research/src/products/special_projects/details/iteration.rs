use dashboard_effects::ScriptedEffects;
use super::{CHARACTER_REFERENCE, COMPLETION_MARKER, Context, OTHER_OWNER, ProjectContent, Reads, Withheld, branch, context, definition, effect_tables, owner_read, owners_of, accepted_projects, project_slices, spelled, supplied_project};
use dashboard_base::{
    Source,
    products::{
        LoadContext,
        at_hour::{At, AtHour, Keys, Owner, Read},
        game_text::{GameRun, runs},
    },
};
use dashboard_base::products::Country;
use dashboard_query::Query;
use replay_data::rows::{ProjectHistoryRow, ProjectRow, TableId, tables};
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
    #[doc = " The choice-history read that listed the iteration: the clicked project's own."]
    pub read: i32,
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
    #[doc = " The iteration's own hour: its game state is read at it, the choice and its project at the listing read."]
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

pub(super) struct Listing {
    read: i32,
    project: ProjectRow,
    row: ProjectHistoryRow,
    history: Vec<ProjectHistoryRow>,
    definition: String,
    reward: String,
    option: String,
}

const UNPROVEN_PROJECT: &str = "The project read at the choice hour is not proven to be this one";
const UNREAD_AT_CHOICE: &str = "The project was not read at the choice hour, so its values then are not recorded";

fn history_range(owner: u32, project: u32) -> Read {
    let first = u128::from(owner) << 64 | u128::from(project) << 32;
    Read { table: TableId::ProjectHistory, keys: Keys::Range(first, first | u128::from(u32::MAX)) }
}

fn project_history(rows: &[ProjectHistoryRow], owner: u32, project: u32) -> &[ProjectHistoryRow] {
    let first = rows.partition_point(|row| (row.owner, row.project) < (owner, project));
    let last = first + rows[first..].partition_point(|row| (row.owner, row.project) == (owner, project));
    &rows[first..last]
}

pub(super) fn listed(query: &Query<'_>, at: &At<'_, Context>, params: &SpecialProjectIterationParams, holder: Owner) -> io::Result<Result<Listing, String>> {
    let read = params.read;
    if read < at.hour {
        return Ok(Err("The iteration's choices were listed before its hour".into()));
    }
    let run = query.timeline.at(read).ok_or_else(|| dashboard_core::refused("The read listing this iteration is not recorded"))?.run;
    let reads = Reads { query, coverage: &at.context.coverage, links: &at.context.links, tag: &params.country, holder, run, hour: read };
    match reads.held_since(params.owner, at.hour) {
        Err(Withheld::Owner) => return Ok(Err(OTHER_OWNER.into())),
        Err(_) => return Ok(Err("This iteration's choice was listed after this country stopped holding the tag".into())),
        Ok(()) => {}
    }
    let project = u128::from(params.owner) << 32 | u128::from(params.project);
    let frame = dashboard_base::products::at_hour::frame(query, read, &[history_range(params.owner, params.project), Read::these(TableId::Projects, [project]), project_slices(params.owner)], SpecialProjectIterationAt::TITLE)?;
    let found = (|| {
        if owner_read(&frame, TableId::ProjectHistory, params.owner)? != read {
            return Err("This iteration's choices were not read at the hour that lists them".to_owned());
        }
        let rows = frame.complete::<tables::ProjectHistory>().map_err(|why| format!("Project choice history is unavailable: {why}"))?;
        if reads.usable(TableId::ProjectHistory, params.owner, read).is_err() {
            return Err("Project choice history is unavailable: the listing read was read only in part".to_owned());
        }
        let history = project_history(rows, params.owner, params.project);
        let row = history.iter().find(|row| row.ordinal == params.ordinal).ok_or("The read listing this iteration does not hold it")?;
        if row.prototype_reward_handle == COMPLETION_MARKER && row.option_handle == COMPLETION_MARKER {
            return Err("This history entry is the project's completion, not an iteration".to_owned());
        }
        if row.date != at.hour {
            return Err("The asked hour is not this iteration's hour".to_owned());
        }
        let (project, _) = supplied_project(&reads, params.owner, params.project)?;
        Ok((project, row.clone(), history.to_vec()))
    })();
    let (project, row, history) = match found {
        Ok(found) => found,
        Err(why) => return Ok(Err(why)),
    };
    let handles = [project.definition_id, row.prototype_reward_handle, row.option_handle];
    let lexical = dashboard_base::products::at_hour::frame(query, read, &[Read::these(TableId::LexicalTokens, handles.map(u128::from))], SpecialProjectIterationAt::TITLE)?;
    let [definition, reward, option] = handles.map(|handle| spelled(&lexical, handle));
    Ok((|| Ok(Listing { read, project, row, history, definition: definition?, reward: reward?, option: option? }))())
}

pub(super) fn same_project(frame: &DecodedFrame, reads: &Reads<'_, '_>, listing: &Listing) -> Result<(), String> {
    let (owner, id, definition) = (listing.project.owner, listing.project.local_id, listing.project.definition_id);
    let (row, read) = supplied_project(reads, owner, id)?;
    if read != reads.hour {
        return Err(UNREAD_AT_CHOICE.to_owned());
    }
    if row.definition_id != definition || owner_read(frame, TableId::ProjectHistory, owner)? != read {
        return Err(UNPROVEN_PROJECT.to_owned());
    }
    let rows = frame.complete::<tables::ProjectHistory>().map_err(|why| format!("Project choice history is unavailable: {why}"))?;
    let then = project_history(rows, owner, id).len();
    let clicked = listing.history.iter().position(|row| row.ordinal == listing.row.ordinal).ok_or(UNPROVEN_PROJECT)?;
    let between = if clicked < then { clicked + 1..then } else { then..clicked };
    let restarted = listing.history.get(between).unwrap_or_default().iter().any(|row| row.prototype_reward_handle == COMPLETION_MARKER && row.option_handle == COMPLETION_MARKER);
    if !listing.history.starts_with(project_history(rows, owner, id)) || restarted || replaced(reads, owner, id, listing.read)? {
        return Err(UNPROVEN_PROJECT.into());
    }
    Ok(())
}

fn replaced(reads: &Reads<'_, '_>, owner: u32, id: u32, listed: i32) -> Result<bool, String> {
    use super::super::{Cursor, HISTORY_COLUMNS, facts::{Entry, Evidence}, history_entry, read_columns, read_key, sample};
    let (choice, recording) = (reads.hour, reads.query.replay.recording());
    if reads.query.timeline.at(listed).map(|stretch| stretch.run) != Some(reads.run) {
        return Ok(true);
    }
    let key = u128::from(owner) << 64 | u128::from(id) << 32;
    let history = dashboard_query::cost::storage(|| replay_lanes::lanes::history(recording, reads.run, TableId::ProjectHistory, &HISTORY_COLUMNS, replay_lanes::lanes::Keys::Range(key, key | u128::from(u32::MAX)), choice, listed))
        .map_err(|why| why.to_string())?;
    let columns = read_columns().map_err(|why| why.to_string())?;
    let stamps = dashboard_query::cost::storage(|| replay_lanes::lanes::history(recording, reads.run, TableId::SliceReads, &columns, replay_lanes::lanes::Keys::These(&[read_key(TableId::ProjectHistory, owner)]), choice, listed))
        .map_err(|why| why.to_string())?;
    let whole: Vec<(i32, i32)> = dashboard_query::cost::storage(|| replay_lanes::lanes::statuses(recording, reads.run, TableId::ProjectHistory, choice, listed))
        .map_err(|why| why.to_string())?
        .into_iter()
        .filter(|run| run.status == replay_columnar::manifest::Status::Complete)
        .map(|run| (run.first, run.last))
        .collect();
    let mut entries: Vec<(u32, Cursor<'_>)> = history.keys.iter().map(|(key, steps)| (*key as u32, Cursor::new(steps))).collect();
    let mut stamp = stamps.keys.first().map(|(_, steps)| Cursor::new(steps));
    let (mut evidence, mut last, mut replaced) = (Evidence::default(), None, false);
    for hour in choice..=listed {
        let Some(read) = stamp.as_mut().and_then(|cursor| sample(cursor.at(hour))) else { continue };
        if read < choice || last == Some(read) || !super::super::within(&whole, read) {
            continue;
        }
        last = Some(read);
        let listing: Vec<Entry> = entries.iter_mut().filter_map(|(ordinal, cursor)| cursor.at(read).and_then(|cells| history_entry(*ordinal, cells))).collect();
        replaced |= evidence.read(read, &listing, |_| false, |_| true, None);
    }
    Ok(replaced)
}

fn chosen<'c>(content: &'c ProjectContent, listing: &Listing) -> Result<&'c content_reader_core::special_projects::PrototypeOption, String> {
    let reward = content.definitions.reward_named(&listing.definition, &listing.reward)?;
    let mut options = reward.options.iter().filter(|candidate| candidate.token.eq_ignore_ascii_case(&listing.option));
    let found = options.next().ok_or("Chosen option is absent from recorded content")?;
    if options.any(|other| other.output != found.output) {
        return Err("Several options of this reward share the chosen token".into());
    }
    Ok(found)
}

fn scopes(frame: &DecodedFrame, row: &ProjectHistoryRow) -> dashboard_effects::effect_description::IterationScopes {
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
    dashboard_effects::effect_description::IterationScopes {
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

fn render(content: &ProjectContent, frame: &DecodedFrame, slot: u32, listing: &Listing, option: &content_reader_core::special_projects::PrototypeOption, accepted: dashboard_text::triggers::AcceptedProjects, out: &mut SpecialProjectIteration) -> Result<(), String> {
    let catalogue = &content.politics.catalogue;
    let scripted = catalogue.scripted_content()?;
    let project = &listing.project;
    let text = dashboard_text::scripted_text::ScriptedText::new(frame, &catalogue.localisation, scripted, slot).with_accepted_projects(accepted);
    let cache = scripted.effects()?;
    let context = dashboard_effects::effect_description::Context::new(&text, &catalogue.localisation, slot, "special-project iteration", cache.view(None)).with_entity_links();
    let description = dashboard_effects::effect_description::project_iteration_description(&option.output, &context, project.owner, project.local_id, scopes(frame, &listing.row));
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
    let listing = match listed(&query, at, params, holder)? {
        Ok(listing) => listing,
        Err(why) => return Ok(Err(why)),
    };
    let run = query.timeline.at(at.hour).ok_or_else(|| dashboard_core::refused("Special project iteration hour is not recorded"))?.run;
    let reads = Reads { query: &query, coverage: &at.context.coverage, links: &at.context.links, tag: &params.country, holder, run, hour: at.hour };
    let content = at.context.content.as_ref().map_err(|why| dashboard_core::refused(why.clone()))?;
    if definition(content, &listing.definition).is_none() {
        return Ok(Err(format!("Definition {} is absent from content", listing.definition)));
    }
    if let Err(why) = same_project(frame, &reads, &listing) {
        return Ok(Err(why));
    }
    let accepted = accepted_projects(&reads, frame);
    Ok(chosen(content, &listing).and_then(|option| render(content, frame, holder.slot, &listing, option, accepted, out)))
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
        effect_tables().into_iter().map(Read::all).chain([history_range(params.owner, params.project), Read::all(TableId::SliceReads)]).collect()
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
