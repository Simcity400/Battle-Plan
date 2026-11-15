//! Party history (family 23): one country's politics over a branch, for the ruling party widget:
//! its parties, which party ruled under which name, its first country leader, and when elections
//! were held, folded from the recording's own rows ([`read`]): per stretch and holder of the tag,
//! the history of its `PoliticalParties` (all but popularity), `CountryAppearance` and `Politics`
//! rows, read at the hours they change. Runs never bridge an hour not read or a change of holder.
//!
//! Parties are named by their ideology group token, joined to content for the group's name and
//! colour, with the party indices each was read under: popularity is the declared
//! `party-popularity` series of those parts, drawn from tiles, the engine's fixed-point percentage
//! ([`POPULARITY_DECIMALS`] places). An election is dated to the hour `Politics.last_election`
//! first shows a new date under the same holder. A party's name and a leader's character are read where their inputs change: a rename
//! of the same character, or a naming input of another country changing alone, is not seen.

use super::{
    CountryParams, LoadContext, Product, check_reply_budget,
    character_portraits::{RecordedPortrait, portrait_png, recorded_portrait},
    ideas::IdeaCharacter,
    politics_content::{PoliticsContent, politics_content},
};
use crate::{Source, metadata::check_cancel};
use dashboard_base::span::HourSpan;
use dashboard_query::{Query, cost, identity::Holder};
use dashboard_text::{catalogue::View, identity::character_script_token, metadata::link_slot, party_names::{PartyNaming, party_key_for_tag_checked}};
use replay::lanes::{self, Cell, History, Keys};
use replay_data::rows::{PORTRAIT_CIVILIAN, PORTRAIT_LARGE, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
};
use ts_rs::TS;

/// Decimal places of recorded popularity (a percentage).
pub const POPULARITY_DECIMALS: u8 = 5;

/// A party, by its ideology group token, and the parts of the `party-popularity` series it was
/// read under: its index in its country's parties.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PartySeries {
    pub token: String,
    pub parts: Vec<u32>,
}

/// A party ruling over consecutive read hours.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct RulingRun {
    pub token: String,
    pub first_hour: i32,
    pub last_hour: i32,
    /// Follows an hour not read or another holder, so the change before it is not a change of
    /// government.
    pub starts_segment: bool,
}

/// The ruling party's first country leader over consecutive read hours.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LeaderRun {
    /// The leader's ideology token; empty when the ruling party had no leader.
    pub ideology: String,
    /// The leader's index in `people`; null when the party had no leader.
    pub person: Option<u32>,
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
}

/// The ruling party's naming over consecutive read hours.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PartyNameRun {
    /// Index in `party_names`.
    pub name: u32,
    pub first_hour: i32,
    pub last_hour: i32,
}

/// An ideology group as content names and colours it.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct IdeologyInfo {
    pub token: String,
    pub name: Option<String>,
    /// sRGB bytes; null when the content's colour is refused (the warnings name why).
    pub color: Option<[u8; 3]>,
}

/// A sub-ideology a recorded leader holds, as content names it.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct IdeologyTypeInfo {
    pub token: String,
    pub name: Option<String>,
}

/// One recorded leader's name and portrait.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LeaderArtwork {
    pub character: IdeaCharacter,
    /// Null when the saved name needs runtime scope or nothing names the character.
    pub name: Option<String>,
    /// An artwork URL; null when the portrait is not known.
    pub portrait: Option<String>,
    /// Why `portrait` is null.
    pub portrait_missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PartyHistoryReply {
    pub branch_id: String,
    pub tag: String,
    pub decimals: u8,
    pub parties: Vec<PartySeries>,
    pub ruling: Vec<RulingRun>,
    /// The ruling party's leader; an hour without a run was not read.
    pub leaders: Vec<LeaderRun>,
    /// The ruling party's name; an hour without a run was not read.
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
    /// Hours the country's parties were not read: a change of government next to one is a gap,
    /// not a change.
    pub unobserved: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(PartyHistoryReply);

/// A leader as read: the character and the portrait its holder records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Leader {
    pub character: IdeaCharacter,
    pub portrait: RecordedPortrait,
}

/// One country's party history, before content is joined.
#[derive(Clone, Debug, Default)]
pub(crate) struct Parties {
    pub parties: BTreeMap<String, PartySeries>,
    pub ruling: Vec<RulingRun>,
    pub leaders: Vec<LeaderRun>,
    pub people: Vec<Leader>,
    pub party_name_runs: Vec<PartyNameRun>,
    /// Each name run's naming and the party tag's original tag (or why it is not known).
    pub namings: Vec<(PartyNaming, Result<String, String>)>,
    pub elections: Vec<i32>,
    pub unobserved: Vec<HourSpan>,
    /// The last hour read and its holder.
    last: Option<(i32, (u32, u32))>,
    /// The holder whose last election date is held, and the date.
    election: Option<((u32, u32), i64)>,
    /// The naming inputs of the last name run (token, party tag, stored key), with its index.
    naming: Option<((String, u32, Option<String>), u32)>,
    /// The leader inputs of the last leader run (character), with its person.
    leader: Option<(Option<u32>, Option<u32>)>,
}

struct Columns {
    party: [usize; 5],
    look: [usize; 2],
    election: usize,
}

impl Columns {
    fn new() -> io::Result<Self> {
        let field = dashboard_declare::field;
        let mut party = [0; 5];
        for (at, name) in ["is_ruling", "token", "tag", "name_default", "name"].into_iter().enumerate() {
            party[at] = field(TableId::PoliticalParties, name)?;
        }
        Ok(Self {
            party,
            look: [field(TableId::CountryAppearance, "leader_ideology")?, field(TableId::CountryAppearance, "leader_character")?],
            election: field(TableId::Politics, "last_election")?,
        })
    }
}

/// A history's keys' cells stepped through in hour order.
struct Stepper<'a> {
    history: &'a History,
    at: Vec<usize>,
    now: Vec<Option<&'a [Cell]>>,
}

impl<'a> Stepper<'a> {
    fn new(history: &'a History) -> Self {
        Self { history, at: vec![0; history.keys.len()], now: vec![None; history.keys.len()] }
    }

    fn hours(&self) -> impl Iterator<Item = i32> + '_ {
        self.history.keys.iter().flat_map(|(_, steps)| steps.iter().map(|step| step.hour))
    }

    fn advance(&mut self, hour: i32) {
        for (index, (_, steps)) in self.history.keys.iter().enumerate() {
            while let Some(step) = steps.get(self.at[index]).filter(|step| step.hour <= hour) {
                self.now[index] = step.cells.as_deref();
                self.at[index] += 1;
            }
        }
    }

    fn shown(&self) -> impl Iterator<Item = &'a [Cell]> + '_ {
        self.now.iter().filter_map(|cells| *cells)
    }

    /// The rows shown with their keys.
    fn keyed(&self) -> impl Iterator<Item = (u128, &'a [Cell])> + '_ {
        self.history.keys.iter().zip(&self.now).filter_map(|((key, _), cells)| Some((*key, (*cells)?)))
    }

    fn place(&self, column: usize) -> Option<usize> {
        self.history.columns.binary_search(&column).ok()
    }

    fn text(&self, cell: Cell) -> Option<&'a [u8]> {
        self.history.text(cell)
    }
}

fn number(cell: Cell) -> Option<i64> {
    match cell {
        Cell::Number(value) => Some(value),
        _ => None,
    }
}

fn mark(spans: &mut Vec<HourSpan>, first_hour: i32, last_hour: i32) {
    match spans.last_mut() {
        Some(span) if i64::from(span.last_hour) + 1 >= i64::from(first_hour) => {
            span.first_hour = span.first_hour.min(first_hour);
            span.last_hour = span.last_hour.max(last_hour);
        }
        _ => spans.push(HourSpan { first_hour, last_hour }),
    }
}

/// The hours `table` of run `run` holds rows read over `first..=last`: the hours it was read where
/// its blocks name the hour each read was taken; where they name none (a table whose status runs
/// carry no sample hour), the hours it was available, its rows as last read.
pub(crate) fn observed_reads(recording: &replay::columnar_frames::Recording, run: u32, table: TableId, first: i32, last: i32) -> io::Result<Vec<(i32, i32)>> {
    let reads = lanes::available_reads(recording, run, table, first, last)?;
    if !reads.is_empty() {
        return Ok(reads);
    }
    Ok(lanes::statuses(recording, run, table, first, last)?
        .into_iter()
        .filter(|held| replay::columnar_frames::available(held.status))
        .map(|held| (held.first, held.last))
        .collect())
}

/// `tag`'s party history over `query`'s timeline.
pub(crate) fn read(query: &Query<'_>, tag: &str, cancelled: &AtomicBool) -> io::Result<Parties> {
    let columns = Columns::new()?;
    let mut parties = Parties::default();
    let runs = query.holders.tags.get(tag).map(Vec::as_slice).unwrap_or_default();
    let mut holes: Vec<(i32, i32)> = query.timeline.unheld.iter().map(|unheld| (unheld.first, unheld.last)).collect();
    for stretch in &query.timeline.stretches {
        check_cancel(cancelled)?;
        parties.last = None;
        let mut covered = stretch.first;
        for run in runs.iter().filter(|run| run.last >= stretch.first && run.first <= stretch.last) {
            let (first, last) = (run.first.max(stretch.first), run.last.min(stretch.last));
            if covered < first {
                holes.push((covered, first - 1));
                parties.last = None;
            }
            covered = last.saturating_add(1);
            match run.holder {
                Holder::Found { slot, id } => parties.country(query, &columns, stretch.run, (slot, id), tag, first, last, cancelled)?,
                Holder::Unresolved => {
                    holes.push((first, last));
                    parties.last = None;
                }
            }
        }
        if covered <= stretch.last {
            holes.push((covered, stretch.last));
        }
    }
    holes.extend(parties.unobserved.drain(..).map(|span| (span.first_hour, span.last_hour)));
    holes.sort_unstable();
    for (first, last) in holes {
        mark(&mut parties.unobserved, first, last);
    }
    Ok(parties)
}

impl Parties {
    #[allow(clippy::too_many_arguments, reason = "one holder's read over one span of one run")]
    fn country(&mut self, query: &Query<'_>, columns: &Columns, run: u32, who: (u32, u32), tag: &str, first: i32, last: i32, cancelled: &AtomicBool) -> io::Result<()> {
        let recording = query.replay.recording();
        let key = u128::from(who.1);
        let observed = cost::storage(|| observed_reads(recording, run, TableId::PoliticalParties, first, last))?;
        let held = cost::storage(|| lanes::history(recording, run, TableId::PoliticalParties, &columns.party, Keys::Range(key << 32, key << 32 | 0xFFFF_FFFF), first, last))?;
        let looks = cost::storage(|| lanes::history(recording, run, TableId::CountryAppearance, &columns.look, Keys::These(&[key]), first, last))?;
        let politics = cost::storage(|| lanes::history(recording, run, TableId::Politics, &[columns.election], Keys::These(&[key]), first, last))?;
        check_cancel(cancelled)?;
        let mut held = Stepper::new(&held);
        let mut looks = Stepper::new(&looks);
        let mut politics = Stepper::new(&politics);
        let mut hours: Vec<i32> = held.hours().chain(looks.hours()).chain(politics.hours()).chain([first]).collect();
        for &(start, end) in &observed {
            hours.push(start);
            if end < last {
                hours.push(end + 1);
            }
        }
        hours.retain(|hour| (first..=last).contains(hour));
        hours.sort_unstable();
        hours.dedup();
        let mut unread = 0;
        for (index, &hour) in hours.iter().enumerate() {
            check_cancel(cancelled)?;
            let end = hours.get(index + 1).map_or(last, |next| next - 1);
            held.advance(hour);
            looks.advance(hour);
            politics.advance(hour);
            while observed.get(unread).is_some_and(|&(_, until)| until < hour) {
                unread += 1;
            }
            if !observed.get(unread).is_some_and(|&(from, _)| from <= hour) {
                mark(&mut self.unobserved, hour, end);
                self.last = None;
                continue;
            }
            self.observe(query, run, hour, end, who, tag, columns, &held, &looks, &politics)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments, reason = "one hour's three row sets of one holder")]
    fn observe(&mut self, query: &Query<'_>, run: u32, hour: i32, end: i32, who: (u32, u32), tag: &str, columns: &Columns, held: &Stepper<'_>, looks: &Stepper<'_>, politics: &Stepper<'_>) -> io::Result<()> {
        let continues = self.last.is_some_and(|(at, seen)| seen == who && i64::from(at) + 1 == i64::from(hour));
        let same = self.last.is_some_and(|(_, seen)| seen == who) || self.election.is_some_and(|(seen, _)| seen == who);
        self.last = Some((end, who));
        let places = columns.party.map(|column| held.place(column));
        if let [Some(ruling), Some(token), Some(party_tag), Some(default), Some(name)] = places {
            for (key, cells) in held.keyed() {
                let Some(token) = held.text(cells[token]).filter(|token| !token.is_empty()).and_then(|token| std::str::from_utf8(token).ok()) else { continue };
                let series = self.parties.entry(token.to_owned()).or_insert_with(|| PartySeries { token: token.to_owned(), parts: Vec::new() });
                let part = key as u32;
                if let Err(at) = series.parts.binary_search(&part) {
                    series.parts.insert(at, part);
                }
                if number(cells[ruling]).is_none_or(|ruling| ruling == 0) {
                    continue;
                }
                let raw = number(cells[party_tag]).and_then(|raw| u32::try_from(raw).ok()).unwrap_or_default();
                let key = (number(cells[default]) == Some(0)).then(|| held.text(cells[name]).map(|name| String::from_utf8_lossy(name).into_owned()).unwrap_or_default());
                let inputs = (token.to_owned(), raw, key.clone());
                let index = match &self.naming {
                    Some((held, index)) if *held == inputs && continues => *index,
                    _ => {
                        let (own, selected) = party_tags(query, run, hour, raw, who, tag)?;
                        let naming = PartyNaming { token: token.to_owned(), tag: own, original_tag: selected.as_ref().ok().cloned(), key };
                        let index = match self.namings.iter().position(|(held, original)| *held == naming && *original == selected) {
                            Some(index) => index as u32,
                            None => {
                                self.namings.push((naming, selected));
                                (self.namings.len() - 1) as u32
                            }
                        };
                        self.naming = Some((inputs, index));
                        index
                    }
                };
                match self.party_name_runs.last_mut() {
                    Some(run) if continues && run.name == index && i64::from(run.last_hour) + 1 == i64::from(hour) => run.last_hour = end,
                    _ => self.party_name_runs.push(PartyNameRun { name: index, first_hour: hour, last_hour: end }),
                }
                match self.ruling.last_mut() {
                    Some(run) if continues && run.token == token && i64::from(run.last_hour) + 1 == i64::from(hour) => run.last_hour = end,
                    previous => {
                        let starts_segment = !(continues && previous.is_some_and(|run| i64::from(run.last_hour) + 1 == i64::from(hour)));
                        self.ruling.push(RulingRun { token: token.to_owned(), first_hour: hour, last_hour: end, starts_segment });
                    }
                }
            }
        }
        // The leader, when this hour's appearance row is read; named as an advisor is, and left
        // unnamed rather than misnamed.
        if let (Some(ideology), Some(character), Some(cells)) = (looks.place(columns.look[0]), looks.place(columns.look[1]), looks.shown().next()) {
            let ideology = looks.text(cells[ideology]).map(|text| String::from_utf8_lossy(text).into_owned()).unwrap_or_default();
            let id = number(cells[character]).and_then(|id| u32::try_from(id).ok());
            let person = match self.leader {
                Some((held, person)) if held == id && continues => person,
                _ => {
                    let person = match id {
                        Some(id) => {
                            let leader = leader_at(query, run, hour, id)?;
                            Some(match self.people.iter().position(|held| *held == leader) {
                                Some(index) => index as u32,
                                None => {
                                    self.people.push(leader);
                                    (self.people.len() - 1) as u32
                                }
                            })
                        }
                        None => None,
                    };
                    self.leader = Some((id, person));
                    person
                }
            };
            match self.leaders.last_mut() {
                Some(run) if continues && i64::from(run.last_hour) + 1 == i64::from(hour) && run.ideology == ideology && run.person == person => run.last_hour = end,
                previous => {
                    let starts_segment = !(continues && previous.is_some_and(|run| i64::from(run.last_hour) + 1 == i64::from(hour)));
                    self.leaders.push(LeaderRun { ideology, person, first_hour: hour, last_hour: end, starts_segment });
                }
            }
        }
        // A gap under the same holder still dates an election, to the hour it is first seen; the
        // first date seen is when the recording started, not an election.
        if !same {
            self.election = None;
        }
        if let Some(cells) = politics.shown().next()
            && let Some(date) = number(cells[0])
        {
            if self.election.is_some_and(|(_, held)| held != date) {
                self.elections.push(hour);
            }
            self.election = Some((who, date));
        }
        Ok(())
    }
}

/// The party tag `raw`'s own tag and original tag at `hour`, as the game names a party by them.
fn party_tags(query: &Query<'_>, run: u32, hour: i32, raw: u32, who: (u32, u32), tag: &str) -> io::Result<(Option<String>, Result<String, String>)> {
    let reads = [replay::at::Read::all(TableId::Countries), replay::at::Read::all(TableId::CountryLinks), replay::at::Read::all(TableId::CountryAppearance)];
    let frame = cost::storage(|| replay::at::frame(query.replay.recording(), run, hour, &reads))?;
    if raw == 0 {
        return Ok((
            installed_content_country_names::country_names::reserved_zero_tag(&frame).ok().map(str::to_owned),
            installed_content_country_names::country_names::original_or_own_tag(&frame, 0).map(str::to_owned),
        ));
    }
    let resolved = (|| -> Result<(String, Result<String, String>), String> {
        let links = frame.complete::<tables::CountryLinks>().map_err(|why| format!("country links is unavailable: {why}"))?;
        let slot = link_slot(links, raw).ok_or_else(|| "the party's country has no recorded country link".to_owned())?;
        if slot == 0 {
            return Ok((
                installed_content_country_names::country_names::reserved_zero_tag(&frame)?.to_owned(),
                installed_content_country_names::country_names::original_or_own_tag(&frame, 0).map(str::to_owned),
            ));
        }
        let countries = frame.complete::<tables::Countries>().map_err(|why| format!("countries is unavailable: {why}"))?;
        let country = if slot == who.0 {
            countries.iter().find(|country| country.id == who.1)
        } else {
            countries.iter().rev().find(|country| country.canonical_slot == slot)
        }
        .ok_or_else(|| "the party's country is not among the recorded countries".to_owned())?;
        let own = if slot == who.0 { tag.to_owned() } else { std::str::from_utf8(&country.tag).map_err(|_| "party tag is not UTF-8")?.to_owned() };
        let appearance = frame.complete::<tables::CountryAppearance>().ok().and_then(|rows| rows.iter().rev().find(|row| row.country == country.id));
        let selected = appearance
            .ok_or_else(|| "the party's country has no complete country appearance".to_owned())
            .and_then(|appearance| installed_content_country_names::country_names::original_tag(country, appearance))
            .map(str::to_owned);
        Ok((own, selected))
    })();
    Ok(match resolved {
        Ok((own, selected)) => (Some(own), selected),
        Err(reason) => (None, Err(reason)),
    })
}

/// Character `id` as the leader views read it at `hour`: its script token and saved name, and the
/// civilian large portrait its holder records.
fn leader_at(query: &Query<'_>, run: u32, hour: i32, id: u32) -> io::Result<Leader> {
    let recording = query.replay.recording();
    let key = [u128::from(id)];
    let reads = [
        replay::at::Read { table: TableId::Characters, columns: None, keys: replay::at::Keys::These(&key) },
        replay::at::Read { table: TableId::CharacterPortraits, columns: None, keys: replay::at::Keys::These(&key) },
    ];
    let frame = cost::storage(|| replay::at::frame(recording, run, hour, &reads))?;
    let row = frame.complete::<tables::Characters>().ok().and_then(|rows| rows.iter().find(|row| row.id == id)).cloned();
    let token = match &row {
        Some(row) => {
            let words = u32::try_from(row.name_token_id).ok().map(u128::from).into_iter().collect::<Vec<_>>();
            let lexicon: DecodedFrame = cost::storage(|| replay::at::frame(recording, run, hour, &[replay::at::Read { table: TableId::LexicalTokens, columns: None, keys: replay::at::Keys::These(&words) }]))?;
            lexicon.complete::<tables::LexicalTokens>().ok().and_then(|lexical| character_script_token(row, lexical)).unwrap_or_default().to_owned()
        }
        None => String::new(),
    };
    let name = row.as_ref().and_then(|row| std::str::from_utf8(&row.token).ok()).unwrap_or_default().to_owned();
    let portrait = recorded_portrait(frame.complete::<tables::CharacterPortraits>().ok(), id, PORTRAIT_CIVILIAN, PORTRAIT_LARGE);
    Ok(Leader { character: IdeaCharacter { id, token, name }, portrait })
}

/// The chart's bytes of an ideology group's colour: each channel clamped to 0..1 and scaled.
fn chart_color(color: [f32; 4]) -> [u8; 3] {
    [color[0], color[1], color[2]].map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Content ideology names and colours.
fn ideology_infos(politics: &PoliticsContent, warnings: &mut Vec<String>) -> Vec<IdeologyInfo> {
    let (groups, gap) = politics.ideologies.groups();
    if let Some(gap) = gap {
        warnings.push(format!("Ideologies may be missing or out of date: {gap}."));
    }
    groups
        .iter()
        .map(|group| {
            if let Err(reason) = &group.color {
                warnings.push(format!("ideology {}: {reason}; its colour is unavailable", group.token));
            }
            IdeologyInfo { token: group.token.clone(), name: politics.localised(&group.token, View::Plain), color: group.color.as_ref().ok().map(|color| chart_color(*color)) }
        })
        .collect()
}

/// The reply to `params` from `parties`, joined with `politics` (names, colours and portraits)
/// where the content loaded.
pub(crate) fn present(politics: Result<&PoliticsContent, String>, params: &CountryParams, parties: &Parties, cancelled: &AtomicBool) -> io::Result<PartyHistoryReply> {
    let mut warnings = Vec::new();
    // Without the game folders the chart still draws, in palette colours and with tokens for names.
    let politics = match politics {
        Ok(politics) => {
            warnings.extend(politics.warnings.iter().cloned());
            Some(politics)
        }
        Err(error) => {
            warnings.push(format!("Ideology and leader names, colours and portraits are unavailable: {error}"));
            None
        }
    };
    let ideologies = politics.map_or_else(Vec::new, |politics| ideology_infos(politics, &mut warnings));
    let types: BTreeSet<&str> = parties.leaders.iter().map(|run| run.ideology.as_str()).filter(|token| !token.is_empty()).collect();
    let ideology_types = types.into_iter().map(|token| IdeologyTypeInfo { token: token.to_owned(), name: politics.and_then(|politics| politics.localised(token, View::Plain)) }).collect();
    let mut people = Vec::with_capacity(parties.people.len());
    for leader in &parties.people {
        check_cancel(cancelled)?;
        let character = &leader.character;
        let script = (!character.token.is_empty()).then_some(character.token.as_str());
        let (name, portrait) = match politics {
            Some(politics) => (politics.character_name(script, &character.name), portrait_png(politics, &leader.portrait)),
            None => (None, Err("Content is unavailable.".to_owned())),
        };
        let (portrait, portrait_missing) = match portrait {
            Ok(portrait) => (Some(portrait), None),
            Err(reason) => (None, Some(reason)),
        };
        people.push(LeaderArtwork { character: character.clone(), name, portrait, portrait_missing });
    }
    let party_names = parties
        .namings
        .iter()
        .map(|(naming, selected)| {
            let politics = politics?;
            let Some((key, fallback)) = party_key_for_tag_checked(naming, selected.as_ref().ok().map(String::as_str), |key| politics.defines(key)) else {
                warnings.push(format!("party name: {}", selected.as_ref().err().cloned().unwrap_or_else(|| "party own tag is unavailable".into())));
                return None;
            };
            warnings.extend(fallback);
            // Party names are plain.
            politics.localised(&key, View::Plain)
        })
        .collect();
    if people.iter().any(|leader| leader.portrait.is_some()) {
        warnings.push("Widget artwork comes from the recorded content stack; visual assets are outside content hash coverage.".to_owned());
    }
    if parties.parties.is_empty() {
        warnings.push(format!("{}: no recorded parties in this branch", params.tag));
    }
    // Names left out for a localisation refusal, by reason and key.
    if let Some(politics) = politics {
        warnings.extend(politics.catalogue.refusal_lines());
    }
    Ok(PartyHistoryReply {
        branch_id: params.branch_id.clone(),
        tag: params.tag.clone(),
        decimals: POPULARITY_DECIMALS,
        parties: parties.parties.values().cloned().collect(),
        ruling: parties.ruling.clone(),
        leaders: parties.leaders.clone(),
        party_name_runs: parties.party_name_runs.clone(),
        party_names,
        people,
        ideology_types,
        elections: parties.elections.clone(),
        ideologies,
        unobserved: parties.unobserved.clone(),
        warnings,
    })
}

/// The last country read, kept for the next request of it.
#[derive(Default)]
struct Kept(Option<(u64, String, Arc<Parties>)>);

fn history(source: &Source, branch: u64, tag: &str, cancelled: &AtomicBool) -> io::Result<Arc<Parties>> {
    let state = source.state::<Kept>();
    let held = state.lock().unwrap_or_else(PoisonError::into_inner).0.as_ref().filter(|(at, held, _)| *at == branch && held == tag).map(|(_, _, read)| Arc::clone(read));
    if let Some(read) = held {
        return Ok(read);
    }
    let query = crate::answer::query(source, branch)?;
    let read = Arc::new(read(&query, tag, cancelled)?);
    state.lock().unwrap_or_else(PoisonError::into_inner).0 = Some((branch, tag.to_owned(), Arc::clone(&read)));
    Ok(read)
}

pub struct PartyHistory;

impl Product for PartyHistory {
    const KEY: &'static str = "party-history";
    const CONTENT_DEPENDENT: bool = true;
    type Params = CountryParams;
    type Reply = PartyHistoryReply;

    fn types() -> Vec<String> {
        vec![
            CountryParams::decl(),
            PartyHistoryReply::decl(),
            PartySeries::decl(),
            RulingRun::decl(),
            LeaderRun::decl(),
            PartyNameRun::decl(),
            IdeologyTypeInfo::decl(),
            LeaderArtwork::decl(),
            IdeologyInfo::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let parties = history(source, params.branch(), &params.tag, cancelled)?;
        let politics = context.content().and_then(|content| politics_content(source, content)).map_err(|error| error.to_string());
        let reply = present(politics.as_deref().map_err(Clone::clone), params, &parties, cancelled)?;
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;
