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
use std::{
    collections::{BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};
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

const UNIT_LEADERS: usize = 0;
const COUNTRY_TABLES: std::ops::Range<usize> = 1..3;
const CHARACTERS: usize = 3;
const TOKENS: usize = 4;
const LEADER_CHARACTER: usize = 1;
const CHARACTER_TOKEN: usize = 1;

pub const LEADER_TABLES: [(TableId, Option<&[usize]>); 5] = [
    (TableId::UnitLeaders, Some(&[LEADER_CHARACTER, 16])),
    (TableId::CountryLinks, None),
    (TableId::Countries, Some(&[0, 1])),
    (TableId::Characters, Some(&[CHARACTER_TOKEN, 4, 5])),
    (TableId::LexicalTokens, None),
];

#[derive(Default)]
pub struct LeaderInputs {
    character: HashMap<u64, u32>,
    token: HashMap<u32, u32>,
}

impl LeaderInputs {
    pub fn dirty(&mut self, readers: &[TableReader], opening: bool, keys: &[u64]) -> Vec<u64> {
        let countries_moved = opening || readers[COUNTRY_TABLES].iter().any(|reader| reader.changes().rows_moved());
        let dirty: Vec<u64> = keys
            .iter()
            .copied()
            .filter(|&key| {
                let character = self.character.get(&key).copied();
                countries_moved
                    || touched(&readers[UNIT_LEADERS], u128::from(key))
                    || character.is_some_and(|character| touched(&readers[CHARACTERS], u128::from(character)))
                    || character.and_then(|character| self.token.get(&character)).is_some_and(|&token| touched(&readers[TOKENS], u128::from(token)))
            })
            .collect();
        for &key in &dirty {
            match read::number(&readers[UNIT_LEADERS], u128::from(key), LEADER_CHARACTER).and_then(|character| u32::try_from(character).ok()) {
                Some(character) => {
                    self.character.insert(key, character);
                    match read::number(&readers[CHARACTERS], u128::from(character), CHARACTER_TOKEN).and_then(|token| u32::try_from(token).ok()) {
                        Some(token) => self.token.insert(character, token),
                        None => self.token.remove(&character),
                    };
                }
                None => {
                    self.character.remove(&key);
                }
            }
        }
        dirty
    }
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

enum Owners<'a> {
    Slot(u32, HashMap<u32, u32>),
    Tag(&'a str, Tags),
}

impl<'a> Owners<'a> {
    fn read(frame: &DecodedFrame, owner: &'a Country) -> Result<Self, String> {
        match owner {
            Country::Slot(asked) => frame
                .complete::<tables::CountryLinks>()
                .map(|links| Self::Slot(*asked, dashboard_text::metadata::link_slots(links)))
                .map_err(|error| format!("Leaders' owners are not linked at this hour: {error}")),
            Country::Tag(asked) => Tags::read(frame)
                .map(|tags| Self::Tag(asked, tags))
                .map_err(|error| format!("Leaders' owners are not named at this hour: {error}")),
        }
    }

    fn owns(&self, raw: u32) -> Option<bool> {
        match self {
            Self::Slot(asked, slots) => slots.get(&raw).map(|held| held == asked),
            Self::Tag(asked, tags) => tags.tag(raw).map(|held| held == *asked),
        }
    }
}

fn answers(frame: &DecodedFrame, content: &Content, owner: &Country, keys: &[u64]) -> Vec<Answer> {
    let leaders = match frame.complete::<tables::UnitLeaders>() {
        Ok(leaders) => leaders,
        Err(why) => return keys.iter().map(|_| Answer::unknown(why.to_string())).collect(),
    };
    let owners = Owners::read(frame, owner);
    let characters = frame.complete::<tables::Characters>().ok();
    let lexical = frame.complete::<tables::LexicalTokens>().ok();
    keys.iter()
        .map(|&key| {
            let Some(row) = leaders.iter().find(|row| handle(row) == key) else {
                return Answer { name: None, missing: Some(LEADER_ABSENT.into()), presence: Presence::Absent };
            };
            let owners = match &owners {
                Ok(owners) => owners,
                Err(why) => return Answer::unknown(why.clone()),
            };
            match owners.owns(row.owner) {
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
        for stretch in query.timeline.stretches.iter().filter(|stretch| stretch.first <= to && stretch.last >= from) {
            let (first, last) = (stretch.first.max(from), stretch.last.min(to));
            let mut inputs = LeaderInputs::default();
            read::walk(recording, stretch.run, &LEADER_TABLES, first, last, |hour, readers: &[TableReader]| {
                check_cancel(cancelled)?;
                let dirty = inputs.dirty(readers, hour == first, &keys);
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
    fn a_slot_owner_resolves_from_the_country_links_alone_and_an_unnamed_owner_is_unknown_not_another_country() {
        use replay_data::rows::{CountryLinkRow, CountryRow, TableRows};
        use replay_frame::frame::TableView;
        let mut frame = dashboard_core::test_fixtures::source(1, 1_000, Some(1));
        *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow { id: 9001, canonical_slot: 7 },
            CountryLinkRow { id: 9002, canonical_slot: 8 },
        ]));
        *frame.tables[TableId::Countries.index()] = TableView::Unavailable { reason: "Countries not read at this hour".into() };
        let slot = Country::Slot(7);
        let tag = Country::Tag("AAA".into());
        let owners = Owners::read(&frame, &slot).expect("the Countries table is not needed to compare slots");
        assert_eq!(owners.owns(9001), Some(true));
        assert_eq!(owners.owns(9002), Some(false));
        assert_eq!(owners.owns(9003), None, "an unlinked owner is not known to be another country");
        assert!(Owners::read(&frame, &tag).is_err(), "a tag owner needs the Countries table");
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![CountryRow { id: 7, canonical_slot: 7, tag: b"AAA".to_vec(), ..CountryRow::default() }]));
        let owners = Owners::read(&frame, &tag).unwrap();
        assert_eq!(owners.owns(9001), Some(true));
        assert_eq!(owners.owns(9002), None, "a tag owner with an unnamed tag is unknown, not absent");
    }

    #[test]
    fn a_leader_is_named_again_when_its_character_or_its_name_token_changes_and_never_for_a_fresh_sample_alone() {
        use replay_data::rows::{CharacterRow, LexicalTokenRow, TableRows, UnitLeaderRow};
        use replay_frame::frame::TableView;
        let frame = |hour: i32| {
            let mut frame = dashboard_core::test_fixtures::source(1, hour, Some(1));
            *frame.tables[TableId::UnitLeaders.index()] = TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow { id: 4, reference_type: 1, character_id: 5, owner: 9001, ..UnitLeaderRow::default() }]));
            let token = if hour < 1_040 { 10 } else { 11 };
            *frame.tables[TableId::Characters.index()] = TableView::Complete(TableRows::Characters(vec![CharacterRow { id: 5, name_token_id: token, token: b"SAMPLE_LEADER".to_vec(), ..CharacterRow::default() }]));
            let spelling: &[u8] = if hour < 1_070 { b"Ada Vance" } else { b"Ada Vane" };
            *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![
                LexicalTokenRow { token: 10, literal: b"Ada".to_vec() },
                LexicalTokenRow { token: 11, literal: spelling.to_vec() },
            ]));
            frame
        };
        let replay = replay::campaign::Replay::from_records(recorder_derive::writers(), dashboard_core::test_fixtures::CAMPAIGN.into(), &[dashboard_core::test_fixtures::records(1, (1_000..=1_100).map(frame).collect())])
            .unwrap()
            .into_timelines(&Default::default())
            .unwrap();
        let source = Source::from_replay(dashboard_base::SourceKey { source_id: "leaders".into(), revision: "1".into() }, "synthetic.battleplan".into(), std::sync::Arc::new(replay), &AtomicBool::new(false)).unwrap();
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let run = query.timeline.stretches[0].run;
        let key = (4_u64 << 32) | 1;
        let mut inputs = LeaderInputs::default();
        let mut named = Vec::new();
        read::walk(query.replay.recording(), run, &LEADER_TABLES, 1_000, 1_100, |hour, readers| {
            if !inputs.dirty(readers, hour == 1_000, &[key]).is_empty() {
                named.push(hour);
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(named, vec![1_000, 1_040, 1_070], "the opening hour, the character's new name token and that token's new spelling");
    }
}
