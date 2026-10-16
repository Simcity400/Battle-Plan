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
//! portrait uses the civilian role and large size: the recorded holder's entry
//! [`replay::rows::portrait_entry`] finds, when it is set and content has its
//! sprite or texture. Otherwise the game generates a portrait from inputs
//! the recording lacks, so it is left out and said to be.
//!
//! The ruling party's name uses its stored key
//! (`PoliticalParties.name`) when `set_party_name` cleared the default flag,
//! otherwise `<tag>_<ideology group>_party`, then with the party tag's
//! positive original reference, then the
//! ideology group's own name. [`crate::party_names::party_key`] follows it.

use super::{
    CountryParams, LoadContext, Product, country_measure::FixedRun, idea_history::IdeaCharacter,
    politics_content::politics_content,
};
#[cfg(test)]
use crate::metadata::validate_campaign;
use crate::{
    Source,
    catalogue::View,
    identity,
    metadata::{CountryKeys, link_slot, tagged_countries},
    party_names::party_key_for_tag_checked,
    products::country_groups::{HourSpan, mark_unobserved},
};
use replay::rows::{TableId, tables};
#[cfg(test)]
use replay::{
    container::Replay,
    frame::{FrameDecoder, ViewIdentity},
};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::{collections::VecDeque, sync::Arc};

pub use crate::party_names::{PartyNaming, party_key};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

pub struct PartyHistory;
pub(crate) mod prepared;

/// Decimal places of recorded popularity (a percentage).
pub const POPULARITY_DECIMALS: u8 = 5;
#[cfg(test)]
const CACHED_BRANCHES: usize = 2;
#[cfg(test)]
const CACHED_COUNTRIES: usize = 8;

fn party_popularity(row: &replay::rows::PoliticalPartyRow) -> Option<(&str, i128)> {
    let token = std::str::from_utf8(&row.token)
        .ok()
        .filter(|token| !token.is_empty())?;
    Some((token, i128::from(row.popularity)))
}

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

/// The ruling party's naming over consecutive recorded hours.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PartyNameRun {
    /// Index in `party_names`.
    pub name: u32,
    pub first_hour: i32,
    pub last_hour: i32,
}

pub(crate) fn ideology_type_info(
    politics: Option<&super::politics_content::PoliticsContent>,
    token: &str,
) -> IdeologyTypeInfo {
    IdeologyTypeInfo {
        token: token.to_owned(),
        name: politics.and_then(|politics| politics.localised(token, View::Plain)),
    }
}

/// An ideology group as content names and colours it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct IdeologyInfo {
    pub token: String,
    pub name: Option<String>,
    /// sRGB bytes ([`chart_color`]); null when the content's colour is
    /// refused (the warnings name why).
    pub color: Option<[u8; 3]>,
}

/// The chart's bytes of an ideology group's colour (red, green, blue,
/// alpha, as installed_content::ideologies reads it): the widget draws its own
/// chart, so each channel is the stored float clamped to 0..1 and scaled to
/// a byte, rounded; the alpha is not drawn.
fn chart_color(color: [f32; 4]) -> [u8; 3] {
    [color[0], color[1], color[2]].map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)
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

pub use super::character_portraits::RecordedPortrait;

/// A leader as scanned: the character and the portrait entry of its runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    /// The ruling party's name; an hour without a run was not recorded.
    pub party_name_runs: Vec<PartyNameRun>,
    /// Each name on `party_name_runs`; null when it cannot be known.
    pub party_names: Vec<Option<String>>,
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScannedParties {
    pub parties: BTreeMap<String, PartySeries>,
    pub ruling: Vec<RulingRun>,
    pub leaders: Vec<LeaderRun>,
    pub people: Vec<ScannedLeader>,
    pub party_name_runs: Vec<PartyNameRun>,
    pub namings: Vec<PartyNaming>,
    selected_tags: Vec<Result<String, String>>,
    pub elections: Vec<i32>,
    pub unobserved: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

/// Widget naming uses exact recorded bytes; the public legacy wrapper keeps
/// its historical empty-original normalization. Missing operands are reached
/// only after the custom key and authored own-tag key fail.
pub(crate) fn recorded_party_key(
    scanned: &ScannedParties,
    index: usize,
    defines: impl Fn(&str) -> bool,
) -> Result<(String, Option<String>), String> {
    let naming = scanned
        .namings
        .get(index)
        .ok_or("party naming index is absent")?;
    let selected = scanned
        .selected_tags
        .get(index)
        .ok_or("party tag operands are absent")?;
    party_key_for_tag_checked(naming, selected.as_ref().ok().map(String::as_str), defines)
        .ok_or_else(|| {
            selected
                .as_ref()
                .err()
                .cloned()
                .unwrap_or_else(|| "party own tag is unavailable".into())
        })
}

type PartyNamingRows = BTreeMap<
    u32,
    (
        replay::rows::CountryRow,
        Option<replay::rows::CountryAppearanceRow>,
    ),
>;
#[cfg(test)]
fn party_naming_rows(
    decoded: &replay::frame::DecodedFrame,
    countries: &[replay::rows::CountryRow],
) -> PartyNamingRows {
    let appearances: BTreeMap<_, _> = decoded
        .complete::<tables::CountryAppearance>()
        .ok()
        .into_iter()
        .flatten()
        .map(|row| (row.country, row))
        .collect();
    countries
        .iter()
        .map(|country| {
            (
                country.canonical_slot,
                (
                    country.clone(),
                    appearances.get(&country.id).map(|row| (*row).clone()),
                ),
            )
        })
        .collect()
}

fn recorded_party_tags(
    decoded: &replay::frame::DecodedFrame,
    raw: u32,
    rows: Option<&PartyNamingRows>,
    current: (
        &replay::rows::CountryRow,
        u32,
        Option<&replay::rows::CountryAppearanceRow>,
    ),
) -> (Option<String>, Result<String, String>) {
    // Raw zero maps to the reserved singleton.
    if raw == 0 {
        return (
            installed_content::country_names::reserved_zero_tag(decoded)
                .ok()
                .map(str::to_owned),
            installed_content::country_names::original_or_own_tag(decoded, 0).map(str::to_owned),
        );
    }
    let resolved = (|| {
        let links = decoded
            .complete::<tables::CountryLinks>()
            .map_err(|why| format!("CountryLinks is unavailable: {why}"))?;
        let slot =
            link_slot(links, raw).ok_or_else(|| format!("party tag link {raw} is unresolved"))?;
        if slot == 0 {
            return Ok((
                installed_content::country_names::reserved_zero_tag(decoded)?,
                installed_content::country_names::original_or_own_tag(decoded, 0).map(str::to_owned),
            ));
        }
        // Selected-country scans already hold their owner/appearance. Only a
        // reached foreign party tag needs another lookup; never build a census
        // index for one holder on every hour.
        let (country, appearance) = if slot == current.1 {
            (current.0, current.2)
        } else if let Some(rows) = rows {
            rows.get(&slot)
                .map(|(country, appearance)| (country, appearance.as_ref()))
                .ok_or_else(|| format!("party tag slot {slot} has no Countries row"))?
        } else {
            let country = decoded
                .complete::<tables::Countries>()
                .map_err(|why| format!("Countries is unavailable: {why}"))?
                .iter()
                .rev()
                .find(|country| country.canonical_slot == slot)
                .ok_or_else(|| format!("party tag slot {slot} has no Countries row"))?;
            let appearance = decoded
                .complete::<tables::CountryAppearance>()
                .ok()
                .and_then(|rows| rows.iter().rev().find(|row| row.country == country.id));
            (country, appearance)
        };
        let tag = std::str::from_utf8(&country.tag).map_err(|_| "party tag is not UTF-8")?;
        let selected = appearance
            .ok_or_else(|| format!("party tag slot {slot} has no complete CountryAppearance row"))
            .and_then(|appearance| installed_content::country_names::original_tag(country, appearance))
            .map(str::to_owned);
        Ok::<_, String>((tag, selected))
    })();
    match resolved {
        Ok((tag, selected)) => (Some(tag.to_owned()), selected),
        Err(reason) => (None, Err(reason)),
    }
}

/// Content ideology names and colours, preserving the native coverage warning.
pub(crate) fn ideology_infos(
    politics: &super::politics_content::PoliticsContent,
    warnings: &mut Vec<String>,
) -> Vec<IdeologyInfo> {
    let (groups, gap) = politics.ideologies.groups();
    if let Some(gap) = gap {
        warnings.push(format!("Ideologies may be missing or out of date: {gap}."));
        politics.catalogue.note(crate::content_check::Finding::new(
            crate::content_check::Kind::Refused,
            "ideology inventory",
            gap.to_string(),
        ));
    }
    groups
        .iter()
        .map(|group| {
            if let Err(reason) = &group.color {
                let subject = format!("ideology {}", group.token);
                let message = format!("{subject}: {reason}; its colour is unavailable");
                politics
                    .catalogue
                    .note(crate::content_check::Finding::grouped(
                        crate::content_check::Kind::Warning,
                        &subject,
                        "ideology colour is unreadable; its colour is unavailable",
                        &message,
                    ));
                warnings.push(message);
            }
            IdeologyInfo {
                token: group.token.clone(),
                // Ideology names are plain.
                name: politics.localised(&group.token, View::Plain),
                color: group.color.as_ref().ok().map(|color| chart_color(*color)),
            }
        })
        .collect()
}

/// The portrait entry the leader views ask character `id`'s recorded holder for:
/// civilian, large, else the first large entry, used only when set.
fn leader_portrait(
    holders: Option<&[replay::rows::CharacterPortraitRow]>,
    id: u32,
) -> RecordedPortrait {
    super::character_portraits::recorded_portrait(
        holders,
        id,
        replay::rows::PORTRAIT_CIVILIAN,
        replay::rows::PORTRAIT_LARGE,
    )
}

/// The feed's ruling and name runs. Popularity, leaders and elections are only
/// retained by the selected-country topic scan, using the same observer.
#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub(crate) struct ScannedPartyBranch {
    pub countries: BTreeMap<String, ScannedParties>,
    pub observed: bool,
    pub refusals: Vec<(String, usize)>,
    absent: ScannedParties,
}

#[cfg(test)]
impl ScannedPartyBranch {
    #[cfg(test)]
    pub fn country(&self, tag: &str) -> &ScannedParties {
        self.countries.get(tag).unwrap_or(&self.absent)
    }
}

#[derive(Default, Serialize, Deserialize)]
struct CountryScan {
    result: ScannedParties,
    last: Option<((u32, u32), i32)>,
    last_election: Option<i32>,
    #[serde(with = "crate::prepared_production::map_pairs")]
    people: BTreeMap<(u32, String, String, RecordedPortrait), u32>,
    #[serde(with = "crate::prepared_production::map_pairs")]
    namings: BTreeMap<(PartyNaming, Result<String, String>), u32>,
    refused_gaps: usize,
    unresolved: usize,
    unresolved_now: bool,
    invalid_ruling: usize,
    invalid_tokens: BTreeSet<Vec<u8>>,
    open_parties: BTreeSet<String>,
}

struct CountryRead<'a> {
    country: &'a replay::rows::CountryRow,
    slot: u32,
    parties: Option<(&'a [replay::rows::PoliticalPartyRow], &'a [usize])>,
    appearance: Option<&'a replay::rows::CountryAppearanceRow>,
    politics: Option<&'a replay::rows::PoliticsRow>,
}

impl CountryScan {
    #[cfg(test)]
    fn extend(&mut self, hour: i32) {
        if self
            .last
            .is_some_and(|(_, previous)| previous.checked_add(1) == Some(hour))
        {
            self.advance(hour);
        }
    }
    fn advance(&mut self, hour: i32) {
        let Some((_, previous)) = self.last.as_mut() else {
            return;
        };
        if hour <= *previous {
            return;
        }
        if let Some(run) = self.result.ruling.last_mut()
            && run.last_hour == *previous
        {
            run.last_hour = hour;
        }
        if let Some(run) = self.result.party_name_runs.last_mut()
            && run.last_hour == *previous
        {
            run.last_hour = hour;
        }
        for token in &self.open_parties {
            if let Some(run) = self
                .result
                .parties
                .get_mut(token)
                .and_then(|series| series.runs.last_mut())
                && run.last_hour == *previous
            {
                run.last_hour = hour;
            }
        }
        if let Some(run) = self.result.leaders.last_mut()
            && run.last_hour == *previous
        {
            run.last_hour = hour;
        }
        *previous = hour;
    }

    fn observe(
        &mut self,
        decoded: &replay::frame::DecodedFrame,
        read: CountryRead<'_>,
        naming_rows: Option<&PartyNamingRows>,
        topic: bool,
    ) {
        let Self {
            result,
            last,
            last_election,
            people,
            namings,
            refused_gaps,
            invalid_ruling,
            invalid_tokens,
            open_parties,
            ..
        } = self;
        let CountryRead {
            country,
            slot,
            parties,
            appearance,
            politics,
        } = read;
        let hour = decoded.hour;
        let identity = (slot, country.id);
        // Only a change of country identity starts politics over; a gap in the same
        // country still dates an election, to the hour it is first seen.
        let same_identity = last.as_ref().is_some_and(|(seen, _)| *seen == identity);
        let continues = last
            .as_ref()
            .is_some_and(|(seen, at)| *seen == identity && at.checked_add(1) == Some(hour));
        if let Some((_, at)) = *last
            && at.checked_add(1).is_some_and(|next| next < hour)
        {
            mark_unobserved(&mut result.unobserved, at + 1, hour - 1);
        }
        *last = Some((identity, hour));
        open_parties.clear();
        for party in parties
            .into_iter()
            .flat_map(|(rows, indices)| indices.iter().map(move |&index| &rows[index]))
        {
            let Some((token, popularity)) = party_popularity(party) else {
                if topic {
                    invalid_tokens.insert(party.token.clone());
                }
                if !topic && party.is_ruling != 0 {
                    *invalid_ruling += 1;
                }
                continue;
            };
            if topic {
                open_parties.insert(token.to_owned());
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
                            && run.value_raw.parse::<i128>() == Ok(popularity) =>
                    {
                        run.last_hour = hour
                    }
                    previous => {
                        let starts_segment = !(continues
                            && previous
                                .is_some_and(|run| run.last_hour.checked_add(1) == Some(hour)));
                        series.runs.push(FixedRun {
                            first_hour: hour,
                            last_hour: hour,
                            value_raw: popularity.to_string(),
                            starts_segment,
                        });
                    }
                }
            }
            if party.is_ruling != 0 {
                // Resolve the party's tag, not its current owner.
                let (tag, selected_tag) = recorded_party_tags(
                    decoded,
                    party.tag,
                    naming_rows,
                    (country, slot, appearance),
                );
                let naming = PartyNaming {
                    token: token.to_owned(),
                    tag,
                    original_tag: selected_tag.as_ref().ok().cloned(),
                    key: (party.name_default == 0)
                        .then(|| String::from_utf8_lossy(&party.name).into_owned()),
                };
                let next = u32::try_from(result.namings.len()).unwrap_or(u32::MAX);
                let name = *namings
                    .entry((naming.clone(), selected_tag.clone()))
                    .or_insert_with(|| {
                        result.namings.push(naming);
                        result.selected_tags.push(selected_tag);
                        next
                    });
                match result.party_name_runs.last_mut() {
                    Some(run)
                        if continues
                            && run.name == name
                            && run.last_hour.checked_add(1) == Some(hour) =>
                    {
                        run.last_hour = hour
                    }
                    _ => {
                        result.party_name_runs.push(PartyNameRun {
                            name,
                            first_hour: hour,
                            last_hour: hour,
                        });
                    }
                }
                match result.ruling.last_mut() {
                    Some(run)
                        if continues
                            && run.token == token
                            && run.last_hour.checked_add(1) == Some(hour) =>
                    {
                        run.last_hour = hour
                    }
                    previous => {
                        if !topic
                            && same_identity
                            && previous.as_ref().is_some_and(|run| {
                                run.token != token && run.last_hour.checked_add(1) != Some(hour)
                            })
                        {
                            *refused_gaps += 1;
                        }
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
        if !topic {
            return;
        }
        // The leader, when this hour's appearance row is recorded; the character is
        // named as an advisor is, and stays unnamed rather than misnamed.
        if let Some(row) = appearance {
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
                            identity::character_script_token(found, lexical)
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
                        && previous.is_some_and(|run| run.last_hour.checked_add(1) == Some(hour)));
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
            *last_election = None;
        }
        // When Politics is unrecorded this hour, the next recorded date still shows
        // any election since.
        if let Some(row) = politics {
            // The first date seen is when the recording started, not an election.
            if last_election
                .as_ref()
                .is_some_and(|previous| *previous != row.last_election)
            {
                result.elections.push(hour);
            }
            *last_election = Some(row.last_election);
        }
    }
}

/// The existing topic observation, shared by its oracle walk and preparation.
#[derive(Default, Serialize, Deserialize)]
struct TopicScan {
    countries: BTreeMap<String, CountryScan>,
    unobserved: Vec<HourSpan>,
    #[cfg(test)]
    #[serde(skip)]
    calls: usize,
    #[serde(skip)]
    native: TopicInputs,
    hour: Option<i32>,
    active: BTreeSet<String>,
    #[serde(skip)]
    updated: Vec<String>,
    #[serde(skip)]
    removed: Vec<String>,
}

#[derive(Default)]
struct TopicInputs {
    keys: Option<CountryKeys>,
    countries: Option<Vec<replay::rows::CountryRow>>,
    links: Option<Vec<replay::rows::CountryLinkRow>>,
    holders: BTreeMap<String, (usize, u32)>,
    by_slot: BTreeMap<u32, Vec<String>>,
    by_id: BTreeMap<u32, Vec<String>>,
    parties: Option<Vec<replay::rows::PoliticalPartyRow>>,
    owned: BTreeMap<u32, Vec<usize>>,
    appearances: BTreeMap<u32, replay::rows::CountryAppearanceRow>,
    politics_rows: Vec<replay::rows::PoliticsRow>,
    politics: BTreeMap<u32, replay::rows::PoliticsRow>,
    naming: PartyNamingRows,
    naming_users: BTreeMap<u32, BTreeSet<String>>,
    leader_users: BTreeMap<u32, BTreeSet<String>>,
    characters: Option<BTreeMap<u32, replay::rows::CharacterRow>>,
    portraits: Option<BTreeMap<u32, replay::rows::CharacterPortraitRow>>,
    lexical: Option<BTreeMap<u32, Vec<u8>>>,
    script_users: BTreeMap<u32, BTreeSet<String>>,
    duplicates: BTreeSet<String>,
    #[cfg(test)]
    work: crate::prepared::observer_tests::Work,
}
impl TopicInputs {
    fn refresh(
        &mut self,
        frame: &replay::frame::DecodedFrame,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
    ) -> BTreeSet<String> {
        let changed =
            |table: TableId| changes.is_none_or(|changes| changes.rows_changed[table.index()]);
        let mut roster = false;
        let mut dirty = BTreeSet::new();
        if changed(TableId::Countries) {
            let rows = frame.complete::<tables::Countries>().ok();
            #[cfg(test)]
            {
                self.work.row_visits += rows.map_or(0, |rows| rows.len());
            }
            if !rows.is_some_and(|rows| self.keys.as_ref().is_some_and(|keys| keys.same(rows))) {
                self.keys = rows.map(CountryKeys::read);
                self.countries = rows.map(|rows| rows.to_vec());
                roster = true;
            }
        }
        if changed(TableId::CountryLinks) {
            let rows = frame.complete::<tables::CountryLinks>().ok();
            #[cfg(test)]
            {
                self.work.row_visits += rows.map_or(0, |rows| rows.len());
            }
            let next = rows.map(|rows| rows.to_vec());
            roster |= self.links != next;
            self.links = next;
        }
        if roster {
            dirty.extend(self.holders.keys().cloned());
            self.holders.clear();
            self.by_slot.clear();
            self.by_id.clear();
            if let Some((countries, links)) = self.countries.as_ref().zip(self.links.as_ref()) {
                for (tag, found) in tagged_countries(countries, links) {
                    if let (Ok(tag), Some((country, slot))) =
                        (std::str::from_utf8(tag), found.found())
                    {
                        let index = countries
                            .binary_search_by_key(&country.id, |row| row.id)
                            .expect("country index");
                        self.holders.insert(tag.to_owned(), (index, slot));
                        self.by_slot.entry(slot).or_default().push(tag.to_owned());
                        self.by_id
                            .entry(country.id)
                            .or_default()
                            .push(tag.to_owned());
                    }
                }
            }
            dirty.extend(self.holders.keys().cloned());
        }
        if changed(TableId::PoliticalParties) || roster {
            if changed(TableId::PoliticalParties) {
                let rows = frame.complete::<tables::PoliticalParties>().ok();
                #[cfg(test)]
                {
                    self.work.row_visits += rows.map_or(0, |rows| rows.len());
                }
                let next = rows.map(|rows| rows.to_vec());
                // Compare the product's current native row projection, not histories.
                let mut owners = BTreeSet::new();
                for row in self.parties.iter().flatten().chain(next.iter().flatten()) {
                    owners.insert(row.country);
                }
                for owner in owners {
                    let group = |rows: &[replay::rows::PoliticalPartyRow]| {
                        let first = rows.partition_point(|row| row.country < owner);
                        let last = rows.partition_point(|row| row.country <= owner);
                        (first, last)
                    };
                    let old = self.parties.as_deref().unwrap_or(&[]);
                    let new = next.as_deref().unwrap_or(&[]);
                    let (a, b) = group(old);
                    let (c, d) = group(new);
                    if old[a..b] != new[c..d]
                        && let Some(slot) = self
                            .links
                            .as_ref()
                            .and_then(|links| link_slot(links, owner))
                    {
                        dirty.extend(self.by_slot.get(&slot).into_iter().flatten().cloned());
                    }
                }
                if self.parties.is_some() != next.is_some() {
                    dirty.extend(self.holders.keys().cloned());
                }
                self.parties = next;
            }
            self.owned.clear();
            self.naming_users.clear();
            for (index, row) in self.parties.iter().flatten().enumerate() {
                if let Some(slot) = self
                    .links
                    .as_ref()
                    .and_then(|links| link_slot(links, row.country))
                {
                    self.owned.entry(slot).or_default().push(index);
                    if let Some(named) = self
                        .links
                        .as_ref()
                        .and_then(|links| link_slot(links, row.tag))
                    {
                        self.naming_users
                            .entry(named)
                            .or_default()
                            .extend(self.by_slot.get(&slot).into_iter().flatten().cloned());
                    } else if row.tag == 0 {
                        self.naming_users
                            .entry(0)
                            .or_default()
                            .extend(self.by_slot.get(&slot).into_iter().flatten().cloned());
                    }
                }
            }
        }
        if changed(TableId::CountryAppearance) {
            let rows = frame.complete::<tables::CountryAppearance>().ok();
            #[cfg(test)]
            {
                self.work.row_visits += rows.map_or(0, |rows| rows.len());
            }
            let next: BTreeMap<_, _> = rows
                .into_iter()
                .flatten()
                .map(|row| {
                    (
                        row.country,
                        replay::rows::CountryAppearanceRow {
                            country: row.country,
                            leader_character: row.leader_character,
                            leader_ideology: row.leader_ideology.clone(),
                            original_tag: row.original_tag.clone(),
                            original_tag_reference: row.original_tag_reference,
                            ..Default::default()
                        },
                    )
                })
                .collect();
            let ids: BTreeSet<_> = self
                .appearances
                .keys()
                .chain(next.keys())
                .copied()
                .filter(|id| self.appearances.get(id) != next.get(id))
                .collect();
            for id in ids {
                dirty.extend(self.by_id.get(&id).into_iter().flatten().cloned());
                if let Some(countries) = &self.countries
                    && let Ok(index) = countries.binary_search_by_key(&id, |row| row.id)
                {
                    let slot = countries[index].canonical_slot;
                    dirty.extend(self.naming_users.get(&slot).into_iter().flatten().cloned());
                    if let Some((country, appearance)) = self.naming.get_mut(&slot)
                        && country.id == id
                    {
                        *appearance = next.get(&id).cloned();
                    }
                }
            }
            self.appearances = next;
        }
        if changed(TableId::Politics) || roster {
            if changed(TableId::Politics) {
                let rows = frame.complete::<tables::Politics>().ok();
                #[cfg(test)]
                {
                    self.work.row_visits += rows.map_or(0, |rows| rows.len());
                }
                self.politics_rows = rows.map_or_else(Vec::new, |rows| rows.to_vec());
            }
            let mut next = BTreeMap::new();
            for row in &self.politics_rows {
                if let Some(slot) = self
                    .links
                    .as_ref()
                    .and_then(|links| link_slot(links, row.country))
                {
                    next.entry(slot).or_insert_with(|| row.clone());
                }
            }
            let slots: BTreeSet<_> = self
                .politics
                .keys()
                .chain(next.keys())
                .copied()
                .filter(|slot| self.politics.get(slot) != next.get(slot))
                .collect();
            for slot in slots {
                dirty.extend(self.by_slot.get(&slot).into_iter().flatten().cloned());
            }
            self.politics = next;
        }
        if roster {
            self.naming = self
                .countries
                .iter()
                .flatten()
                .map(|country| {
                    (
                        country.canonical_slot,
                        (country.clone(), self.appearances.get(&country.id).cloned()),
                    )
                })
                .collect();
        }
        if roster || changed(TableId::CountryAppearance) {
            self.leader_users.clear();
            for (id, appearance) in &self.appearances {
                if let Some(character) = appearance.leader_character {
                    self.leader_users
                        .entry(character)
                        .or_default()
                        .extend(self.by_id.get(id).into_iter().flatten().cloned());
                }
            }
        }
        if changed(TableId::Characters) {
            let rows = frame.complete::<tables::Characters>().ok();
            #[cfg(test)]
            {
                self.work.row_visits += rows.map_or(0, |rows| rows.len());
            }
            let next: Option<BTreeMap<_, _>> = rows.map(|rows| {
                rows.iter()
                    .map(|row| {
                        (
                            row.id,
                            replay::rows::CharacterRow {
                                id: row.id,
                                name_token_id: row.name_token_id,
                                token: row.token.clone(),
                                ..Default::default()
                            },
                        )
                    })
                    .collect()
            });
            if self.characters.is_some() != next.is_some() {
                dirty.extend(self.leader_users.values().flatten().cloned());
            }
            for id in self
                .characters
                .iter()
                .chain(&next)
                .flat_map(|rows| rows.keys())
            {
                if self.characters.as_ref().and_then(|rows| rows.get(id))
                    != next.as_ref().and_then(|rows| rows.get(id))
                {
                    dirty.extend(self.leader_users.get(id).into_iter().flatten().cloned());
                }
            }
            self.characters = next;
        }
        if changed(TableId::CharacterPortraits) {
            let rows = frame.complete::<tables::CharacterPortraits>().ok();
            #[cfg(test)]
            {
                self.work.row_visits += rows.map_or(0, |rows| rows.len());
            }
            let next: Option<BTreeMap<_, _>> = rows.map(|rows| {
                rows.iter()
                    .map(|row| (row.character, row.clone()))
                    .collect()
            });
            if self.portraits.is_some() != next.is_some() {
                dirty.extend(self.leader_users.values().flatten().cloned());
            }
            for id in self
                .portraits
                .iter()
                .chain(&next)
                .flat_map(|rows| rows.keys())
            {
                if self.portraits.as_ref().and_then(|rows| rows.get(id))
                    != next.as_ref().and_then(|rows| rows.get(id))
                {
                    dirty.extend(self.leader_users.get(id).into_iter().flatten().cloned());
                }
            }
            self.portraits = next;
        }
        if roster || changed(TableId::CountryAppearance) || changed(TableId::Characters) {
            self.script_users.clear();
            for (id, tags) in &self.leader_users {
                if let Some(row) = self.characters.as_ref().and_then(|rows| rows.get(id))
                    && let Ok(token) = u32::try_from(row.name_token_id)
                {
                    self.script_users
                        .entry(token)
                        .or_default()
                        .extend(tags.iter().cloned());
                }
            }
        }
        if changed(TableId::LexicalTokens) {
            let rows = frame.complete::<tables::LexicalTokens>().ok();
            #[cfg(test)]
            {
                self.work.row_visits += rows.map_or(0, |rows| rows.len());
            }
            let next: Option<BTreeMap<_, _>> = rows.map(|rows| {
                rows.iter()
                    .map(|row| (row.token, row.literal.clone()))
                    .collect()
            });
            for token in self
                .lexical
                .iter()
                .chain(&next)
                .flat_map(|rows| rows.keys())
            {
                if self.lexical.as_ref().and_then(|rows| rows.get(token))
                    != next.as_ref().and_then(|rows| rows.get(token))
                {
                    dirty.extend(self.script_users.get(token).into_iter().flatten().cloned());
                }
            }
            self.lexical = next;
        }
        if changed(TableId::ReservedCountryNaming) {
            dirty.extend(self.naming_users.get(&0).into_iter().flatten().cloned());
            dirty.extend(self.by_slot.get(&0).into_iter().flatten().cloned());
        }
        if changes.is_none_or(|changes| changes.rows_changed.iter().all(|changed| *changed)) {
            dirty.extend(self.holders.keys().cloned());
        }
        dirty.extend(self.duplicates.iter().cloned());
        dirty
    }
}
impl TopicScan {
    fn observe(
        &mut self,
        decoded: &replay::frame::DecodedFrame,
        requested: Option<&str>,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
    ) {
        let hour = decoded.hour;
        let previous = self.hour;
        let adjacent = previous.and_then(|at| at.checked_add(1)) == Some(hour);
        let mut dirty = self.native.refresh(decoded, changes);
        self.updated.clear();
        self.removed.clear();
        if !adjacent {
            self.seal(); // Close the old authenticated frontier before a gap.
            dirty.extend(self.native.holders.keys().cloned());
        }
        self.hour = Some(hour);
        let known = self.native.countries.is_some() && self.native.links.is_some();
        if !known || self.native.parties.is_none() {
            mark_unobserved(&mut self.unobserved, hour, hour);
        }
        for tag in dirty {
            if requested.is_some_and(|wanted| wanted != tag) {
                continue;
            }
            let Some(&(index, slot)) = self.native.holders.get(&tag) else {
                if self.active.remove(&tag) {
                    self.removed.push(tag);
                }
                continue;
            };
            let country = &self.native.countries.as_ref().expect("country holders")[index];
            let state = self.countries.entry(tag.clone()).or_default();
            if adjacent
                && self.active.contains(&tag)
                && let Some(previous) = previous
            {
                state.advance(previous);
            }
            let indices = self.native.owned.get(&slot).map_or(&[][..], Vec::as_slice);
            let mut tokens = BTreeSet::new();
            let mut ruling = 0;
            let mut unique = true;
            for row in self
                .native
                .parties
                .iter()
                .flat_map(|rows| indices.iter().map(move |&index| &rows[index]))
            {
                unique &= tokens.insert(&row.token);
                ruling += usize::from(row.is_ruling != 0);
            }
            if unique && ruling <= 1 {
                self.native.duplicates.remove(&tag);
            } else {
                self.native.duplicates.insert(tag.clone());
            }
            #[cfg(test)]
            {
                self.calls += 1;
                self.native.work.emitted += 1;
            }
            state.observe(
                decoded,
                CountryRead {
                    country,
                    slot,
                    parties: self.native.parties.as_deref().map(|rows| (rows, indices)),
                    appearance: self.native.appearances.get(&country.id),
                    politics: self.native.politics.get(&slot),
                },
                Some(&self.native.naming),
                true,
            );
            self.active.insert(tag.clone());
            self.updated.push(tag);
        }
    }

    fn seal(&mut self) {
        if let Some(hour) = self.hour {
            for tag in &self.active {
                if let Some(state) = self.countries.get_mut(tag) {
                    state.advance(hour);
                }
            }
        }
    }

    #[cfg(test)]
    fn country(&self, tag: &str) -> ScannedParties {
        let mut result = self
            .countries
            .get(tag)
            .map(|state| state.result.clone())
            .unwrap_or_default();
        result.unobserved.extend(self.unobserved.iter().cloned());
        result.unobserved.sort_by_key(|span| span.first_hour);
        let spans = std::mem::take(&mut result.unobserved);
        for span in spans {
            mark_unobserved(&mut result.unobserved, span.first_hour, span.last_hour);
        }
        if let Some(state) = self.countries.get(tag)
            && !state.invalid_tokens.is_empty()
        {
            result.warnings.push(format!("PoliticalParties ideology tokens are empty or not UTF-8; their popularity and names cannot be identified (count: {}).",state.invalid_tokens.len()));
        }
        result
    }
}

/// Scans and retains only the requested country's topic history.
#[cfg(test)]
pub fn scan(
    replay: &Replay,
    branch: u64,
    tag: &str,
    cancelled: &AtomicBool,
) -> io::Result<ScannedParties> {
    let mut scanned = scan_selected(replay, branch, Some(tag), cancelled)?;
    Ok(scanned.countries.remove(tag).unwrap_or(scanned.absent))
}

#[cfg(test)]
pub(crate) fn scan_all(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<ScannedPartyBranch> {
    scan_selected(replay, branch, None, cancelled)
}

#[cfg(test)]
fn scan_selected(
    replay: &Replay,
    branch: u64,
    requested: Option<&str>,
    cancelled: &AtomicBool,
) -> io::Result<ScannedPartyBranch> {
    let mut selected = vec![
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::PoliticalParties,
        TableId::CountryAppearance,
        TableId::ReservedCountryNaming,
    ];
    if requested.is_some() {
        selected.extend([
            TableId::Politics,
            TableId::Characters,
            TableId::CharacterPortraits,
            TableId::LexicalTokens,
        ]);
    }
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Party request cancelled");
    let mut topic = TopicScan::default();
    let mut states: BTreeMap<String, CountryScan> = BTreeMap::new();
    let mut unobserved = Vec::new();
    let mut observed = false;
    const FEED_VIEWS: [TableId; 5] = [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::PoliticalParties,
        TableId::CountryAppearance,
        TableId::ReservedCountryNaming,
    ];
    let mut previous_frame: Option<(i32, [ViewIdentity; 5], Option<CountryKeys>)> = None;
    let mut party_views: Option<[ViewIdentity; 2]> = None;
    let mut parties_by_slot: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancel());
                }
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
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
                    if let Some(tag) = requested {
                        topic.observe(decoded, Some(tag), None);
                        continue;
                    }
                    let (Ok(countries), Ok(links)) = (
                        decoded.complete::<tables::Countries>(),
                        decoded.complete::<tables::CountryLinks>(),
                    ) else {
                        mark_unobserved(&mut unobserved, hour, hour);
                        continue;
                    };
                    let parties = decoded.complete::<tables::PoliticalParties>().ok();
                    observed |= parties.is_some();
                    if parties.is_none() {
                        mark_unobserved(&mut unobserved, hour, hour);
                    }
                    // Hourly balances change Countries; the feed reads only its keys.
                    if parties.is_some()
                        && previous_frame
                            .as_ref()
                            .is_some_and(|(previous, views, keys)| {
                                previous.checked_add(1) == Some(hour)
                                && views.iter().zip(FEED_VIEWS).all(|(view, table)| {
                                    view.is(&decoded.tables[table.index()])
                                        || table == TableId::Countries
                                            && matches!(keys, Some(keys) if keys.same(countries))
                                })
                            })
                    {
                        for state in states.values_mut() {
                            if state.unresolved_now {
                                state.unresolved += 1;
                            } else {
                                state.extend(hour);
                            }
                        }
                        previous_frame.as_mut().expect("unchanged frame exists").0 = hour;
                        continue;
                    }
                    let found = tagged_countries(countries, links);
                    const PARTY_VIEWS: [TableId; 2] =
                        [TableId::CountryLinks, TableId::PoliticalParties];
                    // Row indexes remain valid while both the inventory and its owner links stand.
                    if party_views.as_ref().is_none_or(|views| {
                        views
                            .iter()
                            .zip(PARTY_VIEWS)
                            .any(|(view, table)| !view.is(&decoded.tables[table.index()]))
                    }) {
                        parties_by_slot.clear();
                        for (index, party) in parties.into_iter().flatten().enumerate() {
                            if party.is_ruling != 0
                                && let Some(slot) = link_slot(links, party.country)
                            {
                                parties_by_slot.entry(slot).or_default().push(index);
                            }
                        }
                        party_views =
                            Some(PARTY_VIEWS.map(|table| decoded.tables[table.index()].identity()));
                    }
                    let appearances: BTreeMap<_, _> = decoded
                        .complete::<tables::CountryAppearance>()
                        .ok()
                        .into_iter()
                        .flatten()
                        .map(|row| (row.country, row))
                        .collect();
                    let naming_rows = party_naming_rows(decoded, countries);
                    for state in states.values_mut() {
                        state.unresolved_now = false;
                    }
                    // Extending runs is observing the same rows again only while each
                    // holder has at most one ruling row with a valid token, so only
                    // then may Countries' keys stand in for its rows.
                    let mut extendable = true;
                    for (tag, found) in found {
                        let Ok(tag) = std::str::from_utf8(tag) else {
                            continue;
                        };
                        if !states.contains_key(tag) {
                            states.insert(tag.to_owned(), CountryScan::default());
                        }
                        let state = states.get_mut(tag).expect("feed country inserted");
                        let Some((country, slot)) = found.found() else {
                            if matches!(found, crate::metadata::TaggedCountry::Unresolved) {
                                state.unresolved_now = true;
                                state.unresolved += 1;
                            }
                            continue;
                        };
                        let ruling = parties.and_then(|rows| {
                            parties_by_slot
                                .get(&slot)
                                .map(|indices| (rows, indices.as_slice()))
                        });
                        extendable &= match ruling {
                            None => true,
                            Some((rows, [index])) => std::str::from_utf8(&rows[*index].token)
                                .is_ok_and(|token| !token.is_empty()),
                            Some(_) => false,
                        };
                        state.observe(
                            decoded,
                            CountryRead {
                                country,
                                slot,
                                parties: ruling,
                                appearance: appearances.get(&country.id).copied(),
                                politics: None,
                            },
                            Some(&naming_rows),
                            false,
                        );
                    }
                    previous_frame = parties.is_some().then(|| {
                        (
                            hour,
                            FEED_VIEWS.map(|table| decoded.tables[table.index()].identity()),
                            extendable.then(|| CountryKeys::read(countries)),
                        )
                    });
                }
            }
            Ok(())
        },
    )?;
    if let Some(tag) = requested {
        return Ok(ScannedPartyBranch {
            countries: BTreeMap::from([(tag.to_owned(), topic.country(tag))]),
            ..Default::default()
        });
    }
    let mut result = ScannedPartyBranch {
        observed,
        absent: ScannedParties {
            unobserved: unobserved.clone(),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut refusals: BTreeMap<&str, usize> = BTreeMap::new();
    for (tag, mut state) in states {
        if !state.invalid_tokens.is_empty() {
            state.result.warnings.push(format!(
                "PoliticalParties ideology tokens are empty or not UTF-8; their popularity and names cannot be identified (count: {}).",
                state.invalid_tokens.len()
            ));
        }
        let mut spans = std::mem::take(&mut state.result.unobserved);
        spans.extend(unobserved.iter().cloned());
        spans.sort_by_key(|span| span.first_hour);
        for span in spans {
            mark_unobserved(
                &mut state.result.unobserved,
                span.first_hour,
                span.last_hour,
            );
        }
        for (count, reason) in [
            (
                state.refused_gaps,
                "a different ruling party was first seen after unobserved hours; a change of government cannot be proven across the gap",
            ),
            (
                state.unresolved,
                "CountryLinks did not resolve the recorded country identity; ruling-party observations cannot be attributed",
            ),
            (
                state.invalid_ruling,
                "PoliticalParties ruling rows had an empty or non-UTF-8 ideology token; their names cannot be resolved",
            ),
        ] {
            if count > 0 {
                let total = refusals.entry(reason).or_default();
                *total = total.checked_add(count).ok_or_else(|| {
                    io::Error::other("Party unavailable observation count exceeds usize")
                })?;
            }
        }
        result.countries.insert(tag, state.result);
    }
    result.refusals = refusals
        .into_iter()
        .map(|(reason, count)| (format!("{reason}."), count))
        .collect();
    Ok(result)
}

#[cfg(test)]
#[derive(Default)]
struct BranchCache {
    entries: VecDeque<(u64, Arc<ScannedPartyBranch>)>,
}

#[cfg(test)]
pub(crate) fn scanned_branch(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<ScannedPartyBranch>> {
    let state = source.state::<BranchCache>();
    {
        let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(index) = cache.entries.iter().position(|(key, _)| *key == branch) {
            let entry = cache.entries.remove(index).expect("cached branch exists");
            let scanned = Arc::clone(&entry.1);
            cache.entries.push_back(entry);
            return Ok(scanned);
        }
    }
    let scanned = Arc::new(scan_all(&source.replay(), branch, cancelled)?);
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    cache.entries.retain(|(key, _)| *key != branch);
    cache.entries.push_back((branch, Arc::clone(&scanned)));
    while cache.entries.len() > CACHED_BRANCHES {
        cache.entries.pop_front();
    }
    Ok(scanned)
}

#[cfg(test)]
#[derive(Default)]
struct Cache {
    entries: VecDeque<((u64, String), Arc<ScannedParties>)>,
}

impl Product for PartyHistory {
    const KEY: &'static str = "party-history";
    const CONTENT_DEPENDENT: bool = true;
    type Params = CountryParams;
    type Reply = PartyHistoryReply;

    fn types() -> Vec<String> {
        vec![
            PartySeries::decl(),
            RulingRun::decl(),
            LeaderRun::decl(),
            PartyNameRun::decl(),
            IdeologyTypeInfo::decl(),
            LeaderArtwork::decl(),
            IdeologyInfo::decl(),
            PartyHistoryReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    #[cfg(test)]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let key = (params.branch(), params.tag.clone());
        let state = source.state::<Cache>();
        let cached = {
            let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
            cache
                .entries
                .iter()
                .position(|(cached, _)| *cached == key)
                .map(|index| {
                    let entry = cache.entries.remove(index).expect("cached country exists");
                    let scanned = Arc::clone(&entry.1);
                    cache.entries.push_back(entry);
                    scanned
                })
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
        reply(source, params, context, &scanned, None, cancelled)
    }
}

fn reply(
    source: &Source,
    params: &CountryParams,
    context: &LoadContext,
    scanned: &ScannedParties,
    historical_types: Option<&BTreeSet<String>>,
    cancelled: &AtomicBool,
) -> io::Result<PartyHistoryReply> {
    let mut warnings = scanned.warnings.clone();
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
    let ideologies = politics
        .as_ref()
        .map_or_else(Vec::new, |politics| ideology_infos(politics, &mut warnings));
    let types: BTreeSet<&str> = scanned
        .leaders
        .iter()
        .map(|run| run.ideology.as_str())
        .filter(|token| !token.is_empty())
        .collect();
    let types = historical_types
        .map(|types| types.iter().map(String::as_str).collect())
        .unwrap_or(types);
    let ideology_types = types
        .into_iter()
        .map(|token| ideology_type_info(politics.as_deref(), token))
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
                super::character_portraits::portrait_png(politics, &leader.portrait),
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
    let party_names = scanned
        .namings
        .iter()
        .enumerate()
        .map(|(index, _)| {
            politics.as_ref().and_then(|politics| {
                let (key, fallback) =
                    match recorded_party_key(scanned, index, |key| politics.defines(key)) {
                        Ok(value) => value,
                        Err(reason) => {
                            warnings.push(format!("party name: {reason}"));
                            return None;
                        }
                    };
                if let Some(reason) = fallback {
                    warnings.push(reason);
                }
                // Party names are plain.
                politics.localised(&key, View::Plain)
            })
        })
        .collect();
    if people.iter().any(|leader| leader.portrait.is_some()) {
        warnings.push("Widget artwork comes from the recorded content stack; visual assets are outside content hash coverage.".to_owned());
    }
    if scanned.parties.is_empty() {
        warnings.push(format!(
            "{}: no recorded parties in this branch",
            params.tag
        ));
    }
    // Names left out for a localisation refusal, by reason and key.
    if let Some(politics) = &politics {
        warnings.extend(politics.catalogue.refusal_lines());
    }
    Ok(PartyHistoryReply {
        branch_id: params.branch_id.clone(),
        tag: params.tag.clone(),
        decimals: POPULARITY_DECIMALS,
        parties: scanned.parties.values().cloned().collect(),
        ruling: scanned.ruling.clone(),
        leaders: scanned.leaders.clone(),
        party_name_runs: scanned.party_name_runs.clone(),
        party_names,
        people,
        ideology_types,
        elections: scanned.elections.clone(),
        ideologies,
        unobserved: scanned.unobserved.clone(),
        warnings,
    })
}

#[cfg(test)]
mod tests;
