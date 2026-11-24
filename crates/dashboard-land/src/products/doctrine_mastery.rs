
use dashboard_base::products::{Country, validate_country};
use dashboard_base::products::{LoadContext, at_hour::{At, AtHour, Keys, Owner, Read}};
use dashboard_base::{Source, catalogue::{self, Refusals, View}};
use dashboard_follow::live::{Cells, Source as Cell};
use content_reader_layers::content::ResolvedContent;
use content_reader_military::doctrines::Database;
use content_reader_parser::tokens::Token;
use content_reader_localisation::localisation::Localisation;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::{DecodedFrame, DecodedSource};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io, sync::Arc};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub mod rewards;
#[cfg(test)]
mod tests;

pub struct DoctrineMastery;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DoctrineMasteryParams {
    pub branch_id: String,
    pub country: Country,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MasteryChoice {
    pub id: String,
    pub name: Option<String>,
    pub icon: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MasteryReward {
    pub id: String,
    pub name: Option<String>,
    pub threshold: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MasteryTrack {
    pub ordinal: u32,
    pub name: Option<String>,
    pub sub: Option<MasteryChoice>,
    pub rewards: Vec<MasteryReward>,
    #[doc = " Why the subdoctrine's rewards are not known, where they are not: the list is then empty and"]
    #[doc = " only the recorded unlock count is known."]
    pub rewards_unknown: Option<String>,
    pub unlocked: i32,
}

pub const TRACK_CELLS: [(&str, &str); 4] = [("mastery", "mastery"), ("bank", "mastery_bank"), ("daily", "daily_mastery"), ("effective", "effective_daily_mastery")];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MasteryFolder {
    pub ordinal: u32,
    #[doc = " The folder token's spelling: stable across hours, for settings that choose folders."]
    pub id: String,
    pub name: Option<String>,
    pub icon: Option<String>,
    #[doc = " None where no grand doctrine is chosen."]
    pub grand: Option<MasteryChoice>,
    pub completed: Vec<MasteryChoice>,
    pub tracks: Vec<MasteryTrack>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DoctrineMasteryReply {
    pub folders: Vec<MasteryFolder>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(DoctrineMasteryReply);

pub struct Context {
    database: Arc<Database>,
    localisation: Arc<Localisation>,
    default_reward: Result<i64, String>,
}

const TABLES: [TableId; 3] = [TableId::DoctrineFolders, TableId::DoctrineTracks, TableId::LexicalTokens];

#[derive(Default)]
struct DefaultReward(Option<(Arc<ResolvedContent>, Result<i64, String>)>);

fn default_reward(source: &Source, resolved: &Arc<ResolvedContent>) -> Result<i64, String> {
    let shared = source.state::<DefaultReward>();
    let mut held = shared.lock().unwrap_or_else(|error| error.into_inner());
    if held.0.as_ref().is_none_or(|(old, _)| !Arc::ptr_eq(old, resolved)) {
        let read = content_reader_defines::defines::read(&resolved.layers).and_then(|defines| defines.fixed5("NDoctrines", "DEFAULT_REWARD_MASTERY")).map_err(|error| error.to_string());
        held.0 = Some((Arc::clone(resolved), read));
    }
    held.0.as_ref().map(|(_, read)| read.clone()).expect("default reward read")
}

fn context(source: &Source, load: &LoadContext, title: &str) -> io::Result<Context> {
    let options = load.content.as_ref().ok_or_else(|| dashboard_core::refused(format!("{title} needs the recorded content stack")))?;
    let resolved = source.content(&options.options)?;
    let database = super::doctrines::database(source, &resolved)?;
    Ok(Context { database, localisation: dashboard_base::content::localisation(source, options)?, default_reward: default_reward(source, &resolved) })
}

fn owner_reads(owner: Owner) -> Vec<Read> {
    let slot = u128::from(owner.slot);
    vec![
        Read { table: TableId::DoctrineFolders, keys: Keys::Range(slot << 32, slot << 32 | u128::from(u32::MAX)) },
        Read { table: TableId::DoctrineTracks, keys: Keys::Range(slot << 64, slot << 64 | u128::from(u64::MAX)) },
    ]
}

fn spelled(bytes: &[u8]) -> String {
    String::from_utf8_lossy(content_reader_parser::parser::c_string(bytes)).into_owned()
}

struct Naming<'c> {
    context: &'c Context,
    tokens: HashMap<u32, &'c [u8]>,
    refused: Refusals,
}

impl<'c> Naming<'c> {
    fn token(&mut self, id: u32) -> Option<&'c [u8]> {
        let token = self.tokens.get(&id).copied();
        if token.is_none() {
            self.refused.note("doctrine token is not recorded", id.to_string());
        }
        token
    }

    fn localised(&mut self, key: &[u8], what: &str) -> Option<String> {
        let context: &'c Context = self.context;
        match catalogue::localised_bytes(&context.localisation, key, View::Plain) {
            Ok(text) => Some(text),
            Err(reason) => {
                self.refused.note(format!("{what} name unavailable: {reason}"), spelled(key));
                None
            }
        }
    }

    fn name(&mut self, kind: u8, token: &[u8]) -> Option<String> {
        let context: &'c Context = self.context;
        match context.database.name(kind, token) {
            Ok(Some(name)) => self.localised(&name.value, "doctrine"),
            Ok(None) => {
                self.refused.note("doctrine has no content name", spelled(token));
                None
            }
            Err(reason) => {
                self.refused.note(format!("doctrine name unavailable: {reason}"), spelled(token));
                None
            }
        }
    }

    fn icon(&mut self, presentation: Result<Option<&content_reader_military::doctrines::Presentation>, String>, token: &[u8]) -> Option<String> {
        match presentation {
            Ok(Some(presentation)) if !presentation.icon.is_empty() => Some(spelled(&presentation.icon)),
            Ok(_) => {
                self.refused.note("doctrine has no content sprite", spelled(token));
                None
            }
            Err(reason) => {
                self.refused.note(format!("doctrine sprite unavailable: {reason}"), spelled(token));
                None
            }
        }
    }

    fn choice(&mut self, kind: u8, id: u32) -> MasteryChoice {
        let Some(token) = self.token(id) else { return MasteryChoice { id: id.to_string(), name: None, icon: None } };
        let context: &'c Context = self.context;
        let presentation = context.database.presentation(kind, token);
        MasteryChoice { id: spelled(token), name: self.name(kind, token), icon: self.icon(presentation, token) }
    }

    fn rewards(&mut self, token: &[u8]) -> Result<Vec<MasteryReward>, String> {
        let context: &'c Context = self.context;
        let Some(source) = context.database.tactic_sources.sub.get(&Token::c_str(token)) else {
            self.refused.note("subdoctrine rewards are not in the content", spelled(token));
            return Err("The subdoctrine's rewards are not in the content".into());
        };
        Ok(source
            .rewards
            .iter()
            .map(|reward| {
                let mut key = content_reader_parser::parser::c_string(token).to_vec();
                key.push(b'_');
                key.extend_from_slice(content_reader_parser::parser::c_string(&reward.token));
                let threshold = match (reward.mastery, &context.default_reward) {
                    (0, Ok(default)) => Some(*default),
                    (0, Err(reason)) => {
                        self.refused.note(format!("default reward mastery unavailable: {reason}"), spelled(&key));
                        None
                    }
                    (mastery, _) => Some(mastery),
                };
                MasteryReward { id: spelled(&reward.token), name: self.localised(&key, "reward"), threshold: threshold.map(|value| value.to_string()) }
            })
            .collect())
    }
}

fn folders(frame: &DecodedFrame, slot: u32, context: &Context, live: &Cells) -> io::Result<(Vec<MasteryFolder>, Refusals)> {
    let mut columns = [0; TRACK_CELLS.len()];
    for (column, (_, field)) in columns.iter_mut().zip(TRACK_CELLS) {
        *column = dashboard_declare::field(TableId::DoctrineTracks, field)?;
    }
    let unrecorded = |table: TableId| -> io::Result<()> {
        (frame as &dyn DecodedSource).complete_status(table).map_err(|_| dashboard_core::refused(format!("Doctrine mastery is not recorded at this hour: {}", dashboard_text::scope::missing(table, &frame.tables[table.index()]))))
    };
    unrecorded(TableId::DoctrineFolders)?;
    unrecorded(TableId::DoctrineTracks)?;
    let tokens = frame.complete::<tables::LexicalTokens>().map(|rows| rows.iter().map(|row| (row.token, row.literal.as_slice())).collect()).unwrap_or_default();
    let mut naming = Naming { context, tokens, refused: Refusals::default() };
    let tracks = frame.complete::<tables::DoctrineTracks>()?;
    let mut out = Vec::new();
    for folder in frame.complete::<tables::DoctrineFolders>()?.iter().filter(|row| row.slot == slot) {
        let token = naming.token(folder.folder);
        let (id, name, icon) = match token {
            Some(token) => {
                let name = match context.database.folder_exact(token) {
                    Ok(Some(entry)) => naming.localised(&entry.name, "doctrine folder"),
                    Ok(None) => {
                        naming.refused.note("doctrine folder has no content name", spelled(token));
                        None
                    }
                    Err(reason) => {
                        naming.refused.note(format!("doctrine folder name unavailable: {reason}"), spelled(token));
                        None
                    }
                };
                let presentation = context.database.folder_presentation(token);
                (spelled(token), name, naming.icon(presentation, token))
            }
            None => (folder.folder.to_string(), None, None),
        };
        let grand_source = folder.grand_doctrine.and_then(|id| naming.tokens.get(&id).copied()).and_then(|token| context.database.tactic_sources.grand.get(&Token::c_str(token)));
        let mut shown = Vec::new();
        for track in tracks.iter().filter(|row| row.slot == slot && row.folder == folder.ordinal) {
            let name = match grand_source.and_then(|grand| grand.tracks.get(track.ordinal as usize)).and_then(|key| context.database.tracks.get(key)) {
                Some(name) => naming.localised(&name.value, "doctrine track"),
                None => None,
            };
            let sub = track.sub_doctrine.map(|id| naming.choice(1, id));
            let rewards = match track.sub_doctrine.map(|id| naming.tokens.get(&id).copied()) {
                Some(Some(token)) => naming.rewards(token),
                Some(None) => Err("The subdoctrine's id is not named in the recording".into()),
                None => Ok(Vec::new()),
            };
            let (rewards, rewards_unknown) = match rewards {
                Ok(rewards) => (rewards, None),
                Err(reason) => (Vec::new(), Some(reason)),
            };
            let (row, key) = (format!("{}|{}", folder.ordinal, track.ordinal), u128::from(slot) << 64 | u128::from(track.folder) << 32 | u128::from(track.ordinal));
            for (cell, column) in TRACK_CELLS.iter().zip(columns) {
                live.value(&row, cell.0, Cell { table: TableId::DoctrineTracks, column, key }, 1.0);
            }
            shown.push(MasteryTrack { ordinal: track.ordinal, name, sub, rewards, rewards_unknown, unlocked: track.rewards });
        }
        out.push(MasteryFolder {
            ordinal: folder.ordinal,
            id,
            name,
            icon,
            grand: folder.grand_doctrine.map(|id| naming.choice(0, id)),
            completed: folder.completed_subdoctrines.iter().map(|&id| naming.choice(1, id)).collect(),
            tracks: shown,
        });
    }
    Ok((out, naming.refused))
}

impl AtHour for DoctrineMastery {
    const KEY: &'static str = "doctrine-mastery";
    const TITLE: &'static str = "Doctrine mastery";
    const CONTENT_DEPENDENT: bool = true;
    type Params = DoctrineMasteryParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = DoctrineMasteryReply;
    type Context = Context;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![DoctrineMasteryParams::decl(), MasteryChoice::decl(), MasteryReward::decl(), MasteryTrack::decl(), MasteryFolder::decl(), DoctrineMasteryReply::decl()]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }


    const CLOCK_DEPENDENT: bool = false;

    fn plan_changes(stage: usize, edits: &dashboard_base::products::at_hour::Edits, _held: &DecodedFrame, _at: &dashboard_base::products::at_hour::ReadsAt<'_, Self::Context>) -> bool {
        stage == 1 && (edits.fields(TableId::DoctrineFolders, u64::MAX) || edits.fields(TableId::DoctrineTracks, dashboard_base::products::at_hour::field_mask(TableId::DoctrineTracks, &["sub_doctrine"])))
    }

    fn advance(_frame: &DecodedFrame, _params: &Self::Params, _at: &At<'_, Self::Context>, edits: &dashboard_base::products::at_hour::Edits, _live: &dashboard_follow::live::Live, _data: &mut dashboard_base::products::at_hour::Held<'_>) -> io::Result<bool> {
        let mask = dashboard_base::products::at_hour::field_mask(TableId::DoctrineTracks, &TRACK_CELLS.map(|(_, field)| field));
        if !edits.only_columns(TableId::DoctrineTracks, mask) { return Ok(false); }
        Ok(true)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        TABLES.to_vec()
    }

    fn reads(_params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        match step {
            0 => owner_reads(owner),
            1 => {
                let mut ids = Vec::new();
                if let Ok(rows) = held.complete::<tables::DoctrineFolders>() {
                    for row in rows {
                        ids.push(row.folder);
                        ids.extend(row.grand_doctrine);
                        ids.extend(row.completed_subdoctrines.iter().copied());
                    }
                }
                if let Ok(rows) = held.complete::<tables::DoctrineTracks>() {
                    ids.extend(rows.iter().filter_map(|row| row.sub_doctrine));
                }
                vec![Read::these(TableId::LexicalTokens, ids.into_iter().map(u128::from))]
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        context(source, load, "Doctrine mastery")
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Doctrine mastery country is unavailable at this hour"))?;
        let (folders, refused) = folders(frame, owner.slot, at.context, at.live)?;
        let mut warnings = at.context.database.notes.clone();
        warnings.extend(refused.lines("doctrine mastery content"));
        let reply = DoctrineMasteryReply { folders, warnings };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
