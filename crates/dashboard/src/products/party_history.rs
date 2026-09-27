//! One country's politics over a branch, for the Ruling party widget: each
//! party's popularity, which party ruled, and when elections were held.
//!
//! Parties are named by their ideology group token, joined to content for the
//! group's name and colour, so a mod's ideologies chart with their own colours.
//! Popularity is the engine's fixed-point percentage (`PoliticalParties.
//! popularity`, [`POPULARITY_DECIMALS`] places). Runs never bridge a gap, a
//! missing row or a change of country identity. An election is dated to the
//! hour `Politics.last_election` first shows a new date.
//!
//! The leader is the ruling party's first country leader as the country's
//! appearance records it (`CountryAppearance.leader_ideology` and
//! `leader_character`), named through `Characters` like an advisor. The
//! portrait is the one the game's leader views ask for
//! (`SetCountryLeaderPortrait@140B38140` calls `GetCharacterPortrait@140B35360`
//! with role civilian, size large): the recorded holder's entry
//! [`replay::rows::portrait_entry`] finds, when it is set and content has its
//! sprite or texture. Otherwise the game generates a portrait
//! (`ResolveOrGeneratePortrait@141388EC0`) from inputs the recording lacks, so it
//! is left out and said to be.

use super::{
    CountryParams, LoadContext, Product, country_measure::FixedRun, idea_history::IdeaCharacter,
    politics_content::politics_content,
};
use crate::{
    Source,
    metadata::{mapped_slot, validate_campaign},
    products::country_groups::HourSpan,
};
use replay::{container::Replay, frame::FrameDecoder, rows::tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct PartyHistory;

/// Decimal places of recorded popularity (a percentage).
pub const POPULARITY_DECIMALS: u8 = 5;
const MAX_RUNS: usize = 400_000;
const CACHED_COUNTRIES: usize = 8;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PartySeries {
    /// The party's ideology group token.
    pub token: String,
    pub runs: Vec<FixedRun>,
}

/// A party ruling over consecutive recorded hours.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct RulingRun {
    pub token: String,
    pub first_hour: i32,
    pub last_hour: i32,
    /// Follows a gap or a change of country identity rather than the previous run,
    /// so the change between them is not a change of government.
    pub starts_segment: bool,
}

/// The ruling party's first country leader over consecutive recorded hours.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LeaderRun {
    /// The leader's ideology token (a sub-ideology); empty when the ruling party
    /// had no leader.
    pub ideology: String,
    /// The leader's index in `people`; null when the party had no leader or the
    /// character could not be read.
    pub person: Option<u32>,
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
}

/// An ideology group as content names and colours it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct IdeologyInfo {
    pub token: String,
    pub name: Option<String>,
    /// sRGB bytes; null when content gives no usable colour.
    pub color: Option<[u8; 3]>,
}

/// A sub-ideology a recorded leader holds, as content names it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct IdeologyTypeInfo {
    pub token: String,
    pub name: Option<String>,
}

/// One recorded leader's name and portrait: a rename, a token that resolves
/// later or a new portrait is another entry.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LeaderArtwork {
    pub character: IdeaCharacter,
    /// Null when the saved name needs runtime scope or nothing names the character.
    pub name: Option<String>,
    /// A PNG data URL; null when the portrait is not known.
    pub portrait: Option<String>,
    /// Why `portrait` is null.
    pub portrait_missing: Option<String>,
}

/// The portrait entry a leader view asks the recorded holder for.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RecordedPortrait {
    /// A set entry: kind 0 names a sprite, kind 1 a texture path.
    Entry { kind: u32, identity: Vec<u8> },
    /// No set entry of that size: the game generates the portrait.
    Generated,
    /// The character's portrait holder was not recorded this hour.
    Unrecorded,
}

/// A leader as scanned: the character and the portrait entry of its runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScannedLeader {
    pub character: IdeaCharacter,
    pub portrait: RecordedPortrait,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PartyHistoryReply {
    pub branch_id: String,
    pub tag: String,
    pub decimals: u8,
    pub parties: Vec<PartySeries>,
    pub ruling: Vec<RulingRun>,
    /// The ruling party's leader; an hour without a run was not recorded.
    pub leaders: Vec<LeaderRun>,
    /// Each character on `leaders`, named and pictured.
    pub people: Vec<LeaderArtwork>,
    /// The leaders' sub-ideologies, named.
    pub ideology_types: Vec<IdeologyTypeInfo>,
    /// Hours an election was first seen held.
    pub elections: Vec<i32>,
    /// Every ideology group in content order, for colours, names and legend order.
    pub ideologies: Vec<IdeologyInfo>,
    /// Hours the country's parties were not observed: a change of government next
    /// to one is a gap, not a change.
    pub unobserved: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ScannedParties {
    pub parties: BTreeMap<String, PartySeries>,
    pub ruling: Vec<RulingRun>,
    pub leaders: Vec<LeaderRun>,
    pub people: Vec<ScannedLeader>,
    pub elections: Vec<i32>,
    pub unobserved: Vec<HourSpan>,
}

fn mark(spans: &mut Vec<HourSpan>, first_hour: i32, last_hour: i32) {
    match spans.last_mut() {
        Some(span) if span.last_hour.checked_add(1) >= Some(first_hour) => {
            span.last_hour = span.last_hour.max(last_hour)
        }
        _ => spans.push(HourSpan {
            first_hour,
            last_hour,
        }),
    }
}

/// The portrait entry the leader views ask character `id`'s recorded holder for:
/// civilian, large, else the first large entry, used only when set.
fn leader_portrait(
    holders: Option<&[replay::rows::CharacterPortraitRow]>,
    id: u32,
) -> RecordedPortrait {
    let holder = holders.and_then(|rows| {
        rows.binary_search_by_key(&id, |row| row.character)
            .ok()
            .map(|index| &rows[index])
    });
    let Some(holder) = holder else {
        return RecordedPortrait::Unrecorded;
    };
    match replay::rows::portrait_entry(
        &holder.portraits,
        replay::rows::PORTRAIT_CIVILIAN,
        replay::rows::PORTRAIT_LARGE,
    ) {
        Some(entry) if entry.is_set != 0 => RecordedPortrait::Entry {
            kind: entry.kind,
            identity: entry.identity.clone(),
        },
        _ => RecordedPortrait::Generated,
    }
}

/// Scans one branch for one country's parties.
pub fn scan(
    replay: &Replay,
    branch: u64,
    tag: &str,
    cancelled: &AtomicBool,
) -> io::Result<ScannedParties> {
    use replay::rows::TableId;
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Politics,
        TableId::PoliticalParties,
        TableId::CountryAppearance,
        TableId::Characters,
        TableId::CharacterPortraits,
        TableId::LexicalTokens,
    ];
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Party request cancelled");
    let mut result = ScannedParties::default();
    let mut last: Option<((u32, u32), i32)> = None;
    let mut last_election: Option<i32> = None;
    let mut runs = 0_usize;
    let mut people: BTreeMap<(u32, String, String, RecordedPortrait), u32> = BTreeMap::new();
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        if cancelled.load(Ordering::Acquire) {
            return Err(cancel());
        }
        let mut decoder = FrameDecoder::for_tables(&selected);
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
            continue;
        };
        for frame in frames {
            if cancelled.load(Ordering::Acquire) {
                return Err(cancel());
            }
            let Ok(decoded) = decoder.advance(&frame) else {
                break;
            };
            if validate_campaign(decoded, replay).is_err() {
                break;
            }
            let hour = frame.hour;
            // Parties, elections (Politics) and the leader (CountryAppearance) may each be
            // missing on their own; each run breaks only where its own table is.
            let (Ok(countries), Ok(links)) = (
                decoded.complete::<tables::Countries>(),
                decoded.complete::<tables::CountryLinks>(),
            ) else {
                mark(&mut result.unobserved, hour, hour);
                continue;
            };
            let parties = decoded.complete::<tables::PoliticalParties>().ok();
            if parties.is_none() {
                mark(&mut result.unobserved, hour, hour);
            }
            let Some((country, slot)) = countries
                .iter()
                .filter(|country| country.tag == tag.as_bytes())
                .find_map(|country| mapped_slot(country, links).ok().map(|slot| (country, slot)))
            else {
                continue;
            };
            let identity = (slot, country.id);
            // Only a change of country identity starts politics over; a gap in the same
            // country still dates an election, to the hour it is first seen.
            let same_identity = last.is_some_and(|(seen, _)| seen == identity);
            let continues =
                last.is_some_and(|(seen, at)| seen == identity && at.checked_add(1) == Some(hour));
            if let Some((_, at)) = last
                && at.checked_add(1).is_some_and(|next| next < hour)
            {
                mark(&mut result.unobserved, at + 1, hour - 1);
            }
            last = Some((identity, hour));
            let owner = |raw: u32| {
                links
                    .binary_search_by_key(&raw, |link| link.id)
                    .ok()
                    .map(|index| links[index].canonical_slot)
                    == Some(slot)
            };
            for party in parties
                .into_iter()
                .flatten()
                .filter(|party| owner(party.country))
            {
                let Ok(token) = std::str::from_utf8(&party.token) else {
                    continue;
                };
                if token.is_empty() {
                    continue;
                }
                let series =
                    result
                        .parties
                        .entry(token.to_owned())
                        .or_insert_with(|| PartySeries {
                            token: token.to_owned(),
                            runs: Vec::new(),
                        });
                match series.runs.last_mut() {
                    Some(run)
                        if continues
                            && run.last_hour.checked_add(1) == Some(hour)
                            && run.value_raw.parse::<i64>() == Ok(party.popularity) =>
                    {
                        run.last_hour = hour
                    }
                    previous => {
                        let starts_segment = !(continues
                            && previous
                                .is_some_and(|run| run.last_hour.checked_add(1) == Some(hour)));
                        runs += 1;
                        if runs > MAX_RUNS {
                            return Err(io::Error::other(
                                "Party history exceeds its memory budget",
                            ));
                        }
                        series.runs.push(FixedRun {
                            first_hour: hour,
                            last_hour: hour,
                            value_raw: party.popularity.to_string(),
                            starts_segment,
                        });
                    }
                }
                if party.is_ruling != 0 {
                    match result.ruling.last_mut() {
                        Some(run)
                            if continues
                                && run.token == token
                                && run.last_hour.checked_add(1) == Some(hour) =>
                        {
                            run.last_hour = hour
                        }
                        previous => {
                            let starts_segment = !(continues
                                && previous
                                    .is_some_and(|run| run.last_hour.checked_add(1) == Some(hour)));
                            result.ruling.push(RulingRun {
                                token: token.to_owned(),
                                first_hour: hour,
                                last_hour: hour,
                                starts_segment,
                            })
                        }
                    }
                }
            }
            // The leader, when this hour's appearance row is recorded; the character is
            // named as an advisor is, and stays unnamed rather than misnamed.
            if let Ok(appearance) = decoded.complete::<tables::CountryAppearance>()
                && let Some(row) = appearance.iter().find(|row| row.country == country.id)
            {
                let characters = decoded.complete::<tables::Characters>().ok();
                let lexical = decoded.complete::<tables::LexicalTokens>().ok();
                let portraits = decoded.complete::<tables::CharacterPortraits>().ok();
                let person = row.leader_character.map(|id| {
                    let found = characters.and_then(|rows| {
                        rows.binary_search_by_key(&id, |character| character.id)
                            .ok()
                            .map(|index| &rows[index])
                    });
                    let character = IdeaCharacter {
                        id,
                        token: found
                            .zip(lexical)
                            .and_then(|(found, lexical)| {
                                super::character_script_token(found, lexical)
                            })
                            .unwrap_or_default()
                            .to_owned(),
                        name: found
                            .and_then(|found| std::str::from_utf8(&found.token).ok())
                            .unwrap_or_default()
                            .to_owned(),
                    };
                    let portrait = leader_portrait(portraits, id);
                    let next = u32::try_from(result.people.len()).unwrap_or(u32::MAX);
                    *people
                        .entry((
                            id,
                            character.token.clone(),
                            character.name.clone(),
                            portrait.clone(),
                        ))
                        .or_insert_with(|| {
                            result.people.push(ScannedLeader {
                                character,
                                portrait,
                            });
                            next
                        })
                });
                let ideology = String::from_utf8_lossy(&row.leader_ideology).into_owned();
                match result.leaders.last_mut() {
                    Some(run)
                        if continues
                            && run.last_hour.checked_add(1) == Some(hour)
                            && run.ideology == ideology
                            && run.person == person =>
                    {
                        run.last_hour = hour
                    }
                    previous => {
                        let starts_segment = !(continues
                            && previous
                                .is_some_and(|run| run.last_hour.checked_add(1) == Some(hour)));
                        runs += 1;
                        if runs > MAX_RUNS {
                            return Err(io::Error::other(
                                "Party history exceeds its memory budget",
                            ));
                        }
                        result.leaders.push(LeaderRun {
                            ideology,
                            person,
                            first_hour: hour,
                            last_hour: hour,
                            starts_segment,
                        });
                    }
                }
            }
            if !same_identity {
                last_election = None;
            }
            // When Politics is unrecorded this hour, the next recorded date still shows
            // any election since.
            if let Ok(politics) = decoded.complete::<tables::Politics>()
                && let Some(row) = politics.iter().find(|row| owner(row.country))
            {
                // The first date seen is when the recording started, not an election.
                if last_election.is_some_and(|previous| previous != row.last_election) {
                    result.elections.push(hour);
                }
                last_election = Some(row.last_election);
            }
        }
    }
    Ok(result)
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<((u64, String), Arc<ScannedParties>)>,
}

impl Product for PartyHistory {
    const KEY: &'static str = "party-history";
    type Params = CountryParams;
    type Reply = PartyHistoryReply;

    fn types() -> Vec<String> {
        vec![
            PartySeries::decl(),
            RulingRun::decl(),
            LeaderRun::decl(),
            IdeologyTypeInfo::decl(),
            LeaderArtwork::decl(),
            IdeologyInfo::decl(),
            PartyHistoryReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let key = (params.branch(), params.tag.clone());
        let state = source.state::<Cache>();
        let cached = {
            let cache = state.lock().unwrap_or_else(|error| error.into_inner());
            cache
                .entries
                .iter()
                .find(|(cached, _)| *cached == key)
                .map(|(_, scanned)| Arc::clone(scanned))
        };
        let scanned = match cached {
            Some(scanned) => scanned,
            None => {
                let scanned = Arc::new(scan(
                    &source.replay(),
                    params.branch(),
                    &params.tag,
                    cancelled,
                )?);
                let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
                cache.entries.retain(|(cached, _)| *cached != key);
                cache.entries.push_back((key, Arc::clone(&scanned)));
                while cache.entries.len() > CACHED_COUNTRIES {
                    cache.entries.pop_front();
                }
                scanned
            }
        };
        let mut warnings = Vec::new();
        // Without the game folders the chart still draws, in palette colours and with tokens for names.
        let politics = match context
            .content()
            .and_then(|content| politics_content(source, content))
        {
            Ok(politics) => {
                warnings.extend(politics.warnings.iter().cloned());
                Some(politics)
            }
            Err(error) => {
                warnings.push(format!(
                    "Ideology and leader names, colours and portraits are unavailable: {error}"
                ));
                None
            }
        };
        let ideologies = politics.as_ref().map_or_else(Vec::new, |politics| {
            politics
                .ideologies
                .groups
                .iter()
                .map(|group| IdeologyInfo {
                    token: group.token.clone(),
                    name: politics.localise(&group.token),
                    color: group.color,
                })
                .collect()
        });
        let types: BTreeSet<&str> = scanned
            .leaders
            .iter()
            .map(|run| run.ideology.as_str())
            .filter(|token| !token.is_empty())
            .collect();
        let ideology_types = types
            .into_iter()
            .map(|token| IdeologyTypeInfo {
                token: token.to_owned(),
                name: politics
                    .as_ref()
                    .and_then(|politics| politics.localise(token)),
            })
            .collect();
        let mut people = Vec::with_capacity(scanned.people.len());
        for leader in &scanned.people {
            if cancelled.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Party request cancelled",
                ));
            }
            let character = &leader.character;
            let script = (!character.token.is_empty()).then_some(character.token.as_str());
            let (name, portrait) = match &politics {
                Some(politics) => (
                    politics.character_name(script, &character.name),
                    match &leader.portrait {
                        RecordedPortrait::Entry { kind, identity } => {
                            politics.recorded_portrait(*kind, identity)
                        }
                        RecordedPortrait::Generated => Err(
                            "The game generates this portrait from inputs the recording lacks."
                                .to_owned(),
                        ),
                        RecordedPortrait::Unrecorded => {
                            Err("The leader's portraits were not recorded.".to_owned())
                        }
                    },
                ),
                None => (None, Err("Content is unavailable.".to_owned())),
            };
            let (portrait, portrait_missing) = match portrait {
                Ok(portrait) => (Some(portrait), None),
                Err(reason) => (None, Some(reason)),
            };
            people.push(LeaderArtwork {
                character: character.clone(),
                name,
                portrait,
                portrait_missing,
            });
        }
        if people.iter().any(|leader| leader.portrait.is_some()) {
            warnings.push("Widget artwork comes from the recorded content stack; visual assets are outside content hash coverage.".to_owned());
        }
        if scanned.parties.is_empty() {
            warnings.push(format!(
                "{}: no recorded parties in this branch",
                params.tag
            ));
        }
        Ok(PartyHistoryReply {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            decimals: POPULARITY_DECIMALS,
            parties: scanned.parties.values().cloned().collect(),
            ruling: scanned.ruling.clone(),
            leaders: scanned.leaders.clone(),
            people,
            ideology_types,
            elections: scanned.elections.clone(),
            ideologies,
            unobserved: scanned.unobserved.clone(),
            warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{RecordedPortrait, leader_portrait};
    use replay::rows::{CharacterPortrait, CharacterPortraitRow};

    #[test]
    fn a_leader_shows_the_civilian_large_entry_or_the_first_large_one_when_set() {
        let entry = |category, size, is_set, name: &str| CharacterPortrait {
            category,
            size,
            is_set,
            kind: 0,
            identity: name.as_bytes().to_vec(),
        };
        let row = |character, portraits| CharacterPortraitRow {
            character,
            portraits,
        };
        let holders = [
            row(
                1,
                vec![entry(0, 0, 1, "civ_small"), entry(0, 1, 1, "civ_large")],
            ),
            // No civilian large: the first large entry, as the game's search returns it.
            row(
                2,
                vec![entry(0, 0, 1, "civ_small"), entry(1, 1, 1, "army_large")],
            ),
            // An unset entry is not tried further: the game generates one.
            row(
                3,
                vec![entry(0, 1, 0, "unset"), entry(1, 1, 1, "army_large")],
            ),
            row(4, vec![entry(0, 0, 1, "civ_small")]),
        ];
        let named = |name: &str| RecordedPortrait::Entry {
            kind: 0,
            identity: name.as_bytes().to_vec(),
        };
        assert_eq!(leader_portrait(Some(&holders), 1), named("civ_large"));
        assert_eq!(leader_portrait(Some(&holders), 2), named("army_large"));
        assert_eq!(
            leader_portrait(Some(&holders), 3),
            RecordedPortrait::Generated
        );
        assert_eq!(
            leader_portrait(Some(&holders), 4),
            RecordedPortrait::Generated
        );
        // A holder this hour did not record is not a generated portrait.
        assert_eq!(
            leader_portrait(Some(&holders), 5),
            RecordedPortrait::Unrecorded
        );
        assert_eq!(leader_portrait(None, 1), RecordedPortrait::Unrecorded);
    }
}
