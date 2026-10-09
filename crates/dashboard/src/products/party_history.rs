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
//!
//! The ruling party's name is the game's (`GetName@14118F860`): its stored key
//! (`PoliticalParties.name`) when `set_party_name` cleared the default flag,
//! then with the party tag's positive original reference (1406DF830), then the
//! ideology group's own name. [`party_key`] follows it.

use super::{
    CountryParams, LoadContext, Product, country_measure::FixedRun, idea_history::IdeaCharacter,
    identity, politics_content::politics_content,
};
use crate::{
    Source,
    catalogue::View,
    metadata::{link_slot, tagged_countries, tagged_country, validate_campaign},
    products::country_groups::{HourSpan, mark_unobserved},
};
use replay::{
    container::Replay,
    frame::{FrameDecoder, ViewIdentity},
    rows::{TableId, tables},
};
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
const CACHED_BRANCHES: usize = 2;
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

/// The ruling party's naming over consecutive recorded hours.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PartyNameRun {
    /// Index in `party_names`.
    pub name: u32,
    pub first_hour: i32,
    pub last_hour: i32,
}

/// What names a party: its tag and ideology group, or the keys `set_party_name` stored.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct PartyNaming {
    /// The ideology group token.
    pub token: String,
    /// The party's own tag; None when no recorded country has its slot.
    pub tag: Option<String>,
    /// The country's original tag, empty for none; None when not recorded.
    pub original_tag: Option<String>,
    /// The stored key, when the default flag is clear.
    pub key: Option<String>,
}

/// The party localisation key: the stored key, else
/// `<TAG>_<group>_party`, `<ORIGTAG>_<group>_party`, then the group token
/// (14118EAD0). None when the tag or original tag is not recorded.
pub fn party_key(naming: &PartyNaming, defines: impl Fn(&str) -> bool) -> Option<String> {
    party_key_checked(naming, defines).map(|(key, _)| key)
}

/// The same party ladder, retaining its fallback reason for the offline check.
pub(crate) fn party_key_checked(
    naming: &PartyNaming,
    defines: impl Fn(&str) -> bool,
) -> Option<(String, Option<String>)> {
    let selected_tag = match naming.original_tag.as_deref() {
        Some("") => naming.tag.as_deref(),
        original => original,
    };
    party_key_for_tag_checked(naming, selected_tag, defines)
}

/// The same ladder with GetOriginalTagOrTag's exact selected bytes. A positive
/// reference may spell empty text, which still probes `_<group>_party`.
pub(crate) fn party_key_with_selected_tag(
    naming: &PartyNaming,
    selected_tag: &str,
    defines: impl Fn(&str) -> bool,
) -> Option<String> {
    party_key_for_tag_checked(naming, Some(selected_tag), defines).map(|(key, _)| key)
}

fn party_key_for_tag_checked(
    naming: &PartyNaming,
    selected_tag: Option<&str>,
    defines: impl Fn(&str) -> bool,
) -> Option<(String, Option<String>)> {
    if let Some(key) = &naming.key {
        return Some((key.clone(), None));
    }
    let own = format!("{}_{}_party", naming.tag.as_deref()?, naming.token);
    let key = if defines(&own) {
        own
    } else {
        format!("{}_{}_party", selected_tag?, naming.token)
    };
    Some(if defines(&key) {
        (key, None)
    } else {
        (
            naming.token.clone(),
            Some(format!(
                "party name keys for {:?} / {} are missing; the ideology group name is shown",
                naming.tag.as_deref()?,
                naming.token
            )),
        )
    })
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

/// The chart's bytes of an ideology group's CColor (red, green, blue,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
/// only after the custom key and authored own-tag key fail (14118EAD0).
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
                .unwrap_or_else(|| "party own tag is unavailable (14118EAD0)".into())
        })
}

fn party_naming_rows<'a>(
    decoded: &'a replay::frame::DecodedFrame,
    countries: &'a [replay::rows::CountryRow],
) -> BTreeMap<
    u32,
    (
        &'a replay::rows::CountryRow,
        Option<&'a replay::rows::CountryAppearanceRow>,
    ),
> {
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
                (country, appearances.get(&country.id).copied()),
            )
        })
        .collect()
}

fn recorded_party_tags(
    decoded: &replay::frame::DecodedFrame,
    raw: u32,
    rows: Option<
        &BTreeMap<
            u32,
            (
                &replay::rows::CountryRow,
                Option<&replay::rows::CountryAppearanceRow>,
            ),
        >,
    >,
    current: (
        &replay::rows::CountryRow,
        u32,
        Option<&replay::rows::CountryAppearanceRow>,
    ),
) -> (Option<String>, Result<String, String>) {
    // ResolveCountryReference@140BA56A0 maps raw zero to the reserved singleton.
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
                .copied()
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
                // Ideology names are plain (140616BC0).
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
#[derive(Clone, Debug, Default)]
pub(crate) struct ScannedPartyBranch {
    pub countries: BTreeMap<String, ScannedParties>,
    pub observed: bool,
    pub refusals: Vec<(String, usize)>,
    absent: ScannedParties,
}

impl ScannedPartyBranch {
    #[cfg(test)]
    pub fn country(&self, tag: &str) -> &ScannedParties {
        self.countries.get(tag).unwrap_or(&self.absent)
    }
}

#[derive(Default)]
struct CountryScan {
    result: ScannedParties,
    last: Option<((u32, u32), i32)>,
    last_election: Option<i32>,
    people: BTreeMap<(u32, String, String, RecordedPortrait), u32>,
    namings: BTreeMap<(PartyNaming, Result<String, String>), u32>,
    refused_gaps: usize,
    unresolved: usize,
    unresolved_now: bool,
    invalid_ruling: usize,
    invalid_tokens: BTreeSet<Vec<u8>>,
}

struct CountryRead<'a> {
    country: &'a replay::rows::CountryRow,
    slot: u32,
    parties: Option<(&'a [replay::rows::PoliticalPartyRow], &'a [usize])>,
    appearance: Option<&'a replay::rows::CountryAppearanceRow>,
    politics: Option<&'a replay::rows::PoliticsRow>,
}

impl CountryScan {
    fn extend(&mut self, hour: i32) {
        let Some((_, previous)) = self.last.as_mut() else {
            return;
        };
        if previous.checked_add(1) != Some(hour) {
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
        *previous = hour;
    }

    fn observe(
        &mut self,
        decoded: &replay::frame::DecodedFrame,
        read: CountryRead<'_>,
        naming_rows: Option<
            &BTreeMap<
                u32,
                (
                    &replay::rows::CountryRow,
                    Option<&replay::rows::CountryAppearanceRow>,
                ),
            >,
        >,
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
        for party in parties
            .into_iter()
            .flat_map(|(rows, indices)| indices.iter().map(move |&index| &rows[index]))
        {
            let Ok(token) = std::str::from_utf8(&party.token) else {
                if topic {
                    invalid_tokens.insert(party.token.clone());
                }
                if !topic && party.is_ruling != 0 {
                    *invalid_ruling += 1;
                }
                continue;
            };
            if token.is_empty() {
                if topic {
                    invalid_tokens.insert(party.token.clone());
                }
                if !topic && party.is_ruling != 0 {
                    *invalid_ruling += 1;
                }
                continue;
            }
            if topic {
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
                        series.runs.push(FixedRun {
                            first_hour: hour,
                            last_hour: hour,
                            value_raw: party.popularity.to_string(),
                            starts_segment,
                        });
                    }
                }
            }
            if party.is_ruling != 0 {
                // 14118EAD0 resolves the party's tag, not its current owner.
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

/// Scans and retains only the requested country's topic history.
pub fn scan(
    replay: &Replay,
    branch: u64,
    tag: &str,
    cancelled: &AtomicBool,
) -> io::Result<ScannedParties> {
    let mut scanned = scan_selected(replay, branch, Some(tag), cancelled)?;
    Ok(scanned.countries.remove(tag).unwrap_or(scanned.absent))
}

pub(crate) fn scan_all(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<ScannedPartyBranch> {
    scan_selected(replay, branch, None, cancelled)
}

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
    let mut previous_frame: Option<(i32, [ViewIdentity; 5])> = None;
    let mut party_views: Option<[ViewIdentity; 2]> = None;
    let mut parties_by_slot: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
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
            if let Some(tag) = requested {
                let Some((country, slot)) = tagged_country(countries, links, tag).found() else {
                    continue;
                };
                let owned: Vec<_> = parties
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .filter(|(_, party)| link_slot(links, party.country) == Some(slot))
                    .map(|(index, _)| index)
                    .collect();
                let appearance = decoded
                    .complete::<tables::CountryAppearance>()
                    .ok()
                    .and_then(|rows| rows.iter().rev().find(|row| row.country == country.id));
                let politics = decoded
                    .complete::<tables::Politics>()
                    .ok()
                    .and_then(|rows| {
                        rows.iter()
                            .find(|row| link_slot(links, row.country) == Some(slot))
                    });
                if !states.contains_key(tag) {
                    states.insert(tag.to_owned(), CountryScan::default());
                }
                states
                    .get_mut(tag)
                    .expect("selected country inserted")
                    .observe(
                        decoded,
                        CountryRead {
                            country,
                            slot,
                            parties: parties.map(|rows| (rows, owned.as_slice())),
                            appearance,
                            politics,
                        },
                        None,
                        true,
                    );
                continue;
            }
            if parties.is_some()
                && previous_frame.as_ref().is_some_and(|(previous, views)| {
                    previous.checked_add(1) == Some(hour)
                        && views
                            .iter()
                            .zip(FEED_VIEWS)
                            .all(|(view, table)| view.is(&decoded.tables[table.index()]))
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
            previous_frame = parties.is_some().then(|| {
                (
                    hour,
                    FEED_VIEWS.map(|table| decoded.tables[table.index()].identity()),
                )
            });
            let found = tagged_countries(countries, links);
            const PARTY_VIEWS: [TableId; 2] = [TableId::CountryLinks, TableId::PoliticalParties];
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
                state.observe(
                    decoded,
                    CountryRead {
                        country,
                        slot,
                        parties: parties.and_then(|rows| {
                            parties_by_slot
                                .get(&slot)
                                .map(|indices| (rows, indices.as_slice()))
                        }),
                        appearance: appearances.get(&country.id).copied(),
                        politics: None,
                    },
                    Some(&naming_rows),
                    false,
                );
            }
        }
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

#[derive(Default)]
struct BranchCache {
    entries: VecDeque<(u64, Arc<ScannedPartyBranch>)>,
}

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
                        match recorded_party_key(&scanned, index, |key| politics.defines(key)) {
                            Ok(value) => value,
                            Err(reason) => {
                                warnings.push(format!("party name: {reason}"));
                                return None;
                            }
                        };
                    if let Some(reason) = fallback {
                        warnings.push(reason);
                    }
                    // Party names are plain (14118F860).
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
}

#[cfg(test)]
mod tests;
