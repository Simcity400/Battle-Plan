use crate::products::leaders::{Content, handle};
use dashboard_base::products::{Country, LoadContext, Product, at_hour::{self, Read}, validate_country};
use dashboard_base::Source;
use dashboard_core::dto::RecordingGap;
use dashboard_intelligence::products::intelligence::Tags;
use dashboard_query::read::{self, touched};
use dashboard_text::metadata::check_cancel;
use dashboard_world::products::{
    place_name_runs::{Answer, NameRun, Presence, RunBook},
};
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use replay_recording::cells::TableReader;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const LEADER_ABSENT: &str = "Not a leader of this country at this hour";
const LEADER_NAME_MISSING: &str = "Leader name not recorded";
const OWNER_UNNAMED: &str = "The leader's owner is not named at this hour";

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaderNameRunsParams {
    pub branch_id: String,
    pub owner: Country,
    pub references: Vec<String>,
    pub from: i32,
    pub to: i32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaderRuns {
    pub reference: String,
    pub runs: Vec<NameRun>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaderNameRunsReply {
    pub from: i32,
    pub to: i32,
    #[doc = " The hours inside `[from, to]` the recording does not hold, for every leader alike."]
    pub unknown: Vec<RecordingGap>,
    pub leaders: Vec<LeaderRuns>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(LeaderNameRunsReply);

fn references(params: &LeaderNameRunsParams) -> Result<Vec<u64>, &'static str> {
    let held: BTreeSet<u64> = params.references.iter().map(|reference| reference.parse::<u64>().map_err(|_| "Invalid leader reference")).collect::<Result<_, _>>()?;
    Ok(held.into_iter().collect())
}

fn read_hour(query: &dashboard_query::Query<'_>, frames: &mut at_hour::Frames<'_>, hour: i32, keys: &[u64]) -> Result<DecodedFrame, String> {
    let mut frame = DecodedFrame { branch: query.timeline.id, hour, tables: std::array::from_fn(|_| replay_frame::frame::not_selected()), metrics: Default::default() };
    let steps: [Vec<Read>; 2] = [
        vec![Read::these(TableId::UnitLeaders, keys.iter().map(|&key| u128::from(key))), Read::all(TableId::Countries), Read::all(TableId::CountryLinks)],
        Vec::new(),
    ];
    for (step, mut reads) in steps.into_iter().enumerate() {
        if step == 1 {
            let Ok(rows) = frame.complete::<tables::UnitLeaders>() else { break };
            let characters: BTreeSet<u32> = rows.iter().map(|row| row.character_id).collect();
            if characters.is_empty() {
                break;
            }
            reads = vec![Read::these(TableId::Characters, characters.into_iter().map(u128::from)), Read::all(TableId::LexicalTokens)];
        }
        let read = frames.frame(hour, &reads, "Leader names").map_err(|error| format!("Not in this recording: {error}"))?;
        for table in reads.iter().map(|read| read.table.index()) {
            frame.tables[table] = read.tables[table].clone();
        }
    }
    Ok(frame)
}

fn owned_by(owner: &Country, slot: Option<u32>, tag: Option<String>) -> Option<bool> {
    match owner {
        Country::Slot(asked) => slot.map(|held| held == *asked),
        Country::Tag(asked) => tag.map(|held| held == *asked),
    }
}

fn answers(frame: &DecodedFrame, content: &Content, owner: &Country, keys: &[u64]) -> Vec<Answer> {
    let leaders = match frame.complete::<tables::UnitLeaders>() {
        Ok(leaders) => leaders,
        Err(why) => return keys.iter().map(|_| Answer::unknown(why.to_string())).collect(),
    };
    let tags = Tags::read(frame).map_err(|error| format!("Leaders' owners are not named at this hour: {error}"));
    let characters = frame.complete::<tables::Characters>().ok();
    let lexical = frame.complete::<tables::LexicalTokens>().ok();
    keys.iter()
        .map(|&key| {
            let Some(row) = leaders.iter().find(|row| handle(row) == key) else {
                return Answer { name: None, missing: Some(LEADER_ABSENT.into()), presence: Presence::Absent };
            };
            let tags = match &tags {
                Ok(tags) => tags,
                Err(why) => return Answer::unknown(why.clone()),
            };
            match owned_by(owner, tags.slot(row.owner), tags.tag(row.owner)) {
                Some(true) => {}
                Some(false) => return Answer { name: None, missing: Some(LEADER_ABSENT.into()), presence: Presence::Absent },
                None => return Answer::unknown(OWNER_UNNAMED.into()),
            }
            let character = characters.and_then(|rows| rows.binary_search_by_key(&row.character_id, |character| character.id).ok().map(|at| &rows[at]));
            match content.person_name(character, lexical) {
                Some(name) => Answer { name: Some(name), missing: None, presence: Presence::Present },
                None => Answer { name: None, missing: Some(LEADER_NAME_MISSING.into()), presence: Presence::Present },
            }
        })
        .collect()
}

pub struct LeaderNameRuns;

impl Product for LeaderNameRuns {
    const KEY: &'static str = "leader-name-runs";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = LeaderNameRunsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, owner, from: Start, to: End, "references[]"];
    type Reply = LeaderNameRunsReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid leader name runs branch");
        }
        if params.from > params.to {
            return Err("Invalid leader name runs span");
        }
        validate_country(&params.owner)?;
        if references(params)?.is_empty() {
            return Err("Leader name runs name no leader");
        }
        Ok(())
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![LeaderNameRunsParams::decl(), LeaderRuns::decl(), LeaderNameRunsReply::decl()]
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid leader name runs branch"))?;
        let keys = references(params).map_err(dashboard_core::invalid)?;
        let query = dashboard_base::queries::query(source, branch)?;
        let content = Content::load(source, load);
        let (from, to) = (params.from.max(query.timeline.first), params.to.min(query.timeline.last));
        if from > to {
            return Err(dashboard_core::invalid("Leader name runs ask outside the branch"));
        }
        let unknown: Vec<RecordingGap> = query
            .timeline
            .unheld
            .iter()
            .filter(|gap| gap.first <= to && gap.last >= from)
            .map(|gap| RecordingGap { first_hour: gap.first.max(from), last_hour: gap.last.min(to), reason: format!("Not in this recording: {}", gap.reason) })
            .collect();
        let mut book = RunBook::new(keys.iter().copied());
        let recording = query.replay.recording();
        let mut frames = at_hour::Frames::new(&query)?;
        let tables: [(TableId, Option<&[usize]>); 3] = [(TableId::UnitLeaders, Some(&[1, 16])), (TableId::CountryLinks, None), (TableId::Countries, Some(&[0, 1]))];
        for stretch in query.timeline.stretches.iter().filter(|stretch| stretch.first <= to && stretch.last >= from) {
            let (first, last) = (stretch.first.max(from), stretch.last.min(to));
            read::walk(recording, stretch.run, &tables, first, last, |hour, readers: &[TableReader]| {
                check_cancel(cancelled)?;
                let countries_moved = hour == first || readers[1..].iter().any(|reader| !reader.changes().quiet());
                let dirty: Vec<u64> = keys.iter().copied().filter(|&key| countries_moved || touched(&readers[0], u128::from(key))).collect();
                if dirty.is_empty() {
                    return Ok(());
                }
                let named = match read_hour(&query, &mut frames, hour, &dirty) {
                    Ok(frame) => answers(&frame, &content, &params.owner, &dirty),
                    Err(reason) => dirty.iter().map(|_| Answer::unknown(reason.clone())).collect(),
                };
                for (key, answer) in dirty.into_iter().zip(named) {
                    book.settle(key, answer, hour, &unknown);
                }
                Ok(())
            })?;
            book.close(last, &unknown);
        }
        let leaders = book.runs.into_iter().map(|(key, runs)| LeaderRuns { reference: key.to_string(), runs }).collect();
        let mut warnings = content.missing();
        warnings.sort();
        warnings.dedup();
        let reply = LeaderNameRunsReply { from, to, unknown, leaders, warnings };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slot_owner_matches_on_the_slot_alone_and_an_unnamed_owner_is_unknown_not_another_country() {
        let slot = Country::Slot(7);
        let tag = Country::Tag("AAA".into());
        assert_eq!(owned_by(&slot, Some(7), None), Some(true), "a matching slot with an unnamed tag is the same owner");
        assert_eq!(owned_by(&slot, Some(8), Some("AAA".into())), Some(false));
        assert_eq!(owned_by(&slot, None, None), None, "an unlinked slot is not known to be another owner");
        assert_eq!(owned_by(&tag, Some(7), None), None, "a tag owner with an unnamed tag is unknown, not absent");
        assert_eq!(owned_by(&tag, Some(7), Some("AAA".into())), Some(true));
        assert_eq!(owned_by(&tag, Some(7), Some("BBB".into())), Some(false));
    }
}
