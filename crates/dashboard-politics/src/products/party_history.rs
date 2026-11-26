use dashboard_base::products::Country;
use dashboard_base::products::{CountryParams, LoadContext, Product, check_reply_budget, character_portraits::{RecordedPortrait, portrait_png, recorded_portrait}, politics_content::{PoliticsContent, politics_content}};
use super::ideas::IdeaCharacter;
use dashboard_base::Source;
use dashboard_text::metadata::check_cancel;
use dashboard_base::span::HourSpan;
use dashboard_query::{Query, cost, identity::Holder};
use dashboard_text::{catalogue::View, identity::character_script_token, metadata::link_slot, party_names::{PartyNaming, party_key_for_tag_checked}};
use replay_lanes::lanes::{self, Cell, History, Keys};
use replay_data::rows::{PORTRAIT_CIVILIAN, PORTRAIT_LARGE, TableId, tables};
use replay_frame::frame::{DecodedFrame, TableView};
use replay_recording::columnar_frames::{Damage, DamageScope};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const POPULARITY_DECIMALS: u8 = 5;

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PartySeries {
    pub token: String,
    #[doc = " The parts this party owned, each over the hours it was read owning it."]
    pub parts: Vec<PartOwnership>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PartOwnership {
    pub part: u32,
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Election {
    #[doc = " The recorded date of the election."]
    pub hour: i32,
    #[doc = " The hour the new date was first read; nothing shows the election before it."]
    pub witnessed: i32,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum GovernmentCauseKind {
    Election,
    Coup,
    CivilWar,
    Effect,
    PeaceConference,
    Overlord,
    Console,
    Reset,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct GovernmentChange {
    #[doc = " The recorded hour of the change."]
    pub hour: i32,
    #[doc = " The hour it was read; nothing shows the change before it."]
    pub witnessed: i32,
    #[doc = " The ruling party's ideology before and after; null when not recorded."]
    pub before: Option<String>,
    pub after: Option<String>,
    #[doc = " Null when the recording does not name the cause."]
    pub cause: Option<GovernmentCauseKind>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RulingRun {
    pub token: String,
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " Follows an hour not read or another holder, so the change before it is not a change of"]
    #[doc = " government."]
    pub starts_segment: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaderRun {
    #[doc = " The leader's ideology token; empty when the ruling party had no leader."]
    pub ideology: String,
    #[doc = " The leader's index in `people`; null when the party had no leader."]
    pub person: Option<u32>,
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PartyNameRun {
    #[doc = " Index in `party_names`."]
    pub name: u32,
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct IdeologyInfo {
    pub token: String,
    pub name: Option<String>,
    #[doc = " sRGB bytes; null when the content's colour is refused (the warnings name why)."]
    pub color: Option<[u8; 3]>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct IdeologyTypeInfo {
    pub token: String,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaderArtwork {
    pub character: IdeaCharacter,
    #[doc = " Null when the saved name needs runtime scope or nothing names the character."]
    pub name: Option<String>,
    #[doc = " An artwork URL; null when the portrait is not known."]
    pub portrait: Option<String>,
    #[doc = " Why `portrait` is null."]
    pub portrait_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PartyHistoryReply {
    pub branch_id: String,
    pub tag: String,
    #[doc = " The country the request named, which a reply is matched to: a slot follows its country through any tag."]
    pub asked: dashboard_base::products::Country,
    pub decimals: u8,
    #[doc = " Every party the country had, in stack order: the content's ideology order, then token."]
    pub parties: Vec<PartySeries>,
    pub ruling: Vec<RulingRun>,
    #[doc = " The ruling party's leader; an hour without a run was not read."]
    pub leaders: Vec<LeaderRun>,
    #[doc = " The ruling party's name; an hour without a run was not read."]
    pub party_name_runs: Vec<PartyNameRun>,
    #[doc = " Each name on `party_name_runs`; null when it cannot be known."]
    pub party_names: Vec<Option<String>>,
    #[doc = " Each character on `leaders`, named and pictured."]
    pub people: Vec<LeaderArtwork>,
    #[doc = " The leaders' sub-ideologies, named."]
    pub ideology_types: Vec<IdeologyTypeInfo>,
    #[doc = " Each election whose new date was read, at its recorded date."]
    pub elections: Vec<Election>,
    #[doc = " Each recorded change of government, in hour order."]
    pub government_changes: Vec<GovernmentChange>,
    #[doc = " Hours whose changes of government were not wholly recorded: a change in them may be missing."]
    pub government_unobserved: Vec<HourSpan>,
    #[doc = " Every ideology group in content order, for colours, names and legend order."]
    pub ideologies: Vec<IdeologyInfo>,
    #[doc = " Hours the country's parties were not read: a change of government next to one is a gap,"]
    #[doc = " not a change."]
    pub unobserved: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(PartyHistoryReply);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Leader {
    pub character: IdeaCharacter,
    pub portrait: RecordedPortrait,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Parties {
    pub parties: BTreeMap<String, PartySeries>,
    ruling: RulingHistory,
    any_party: bool,
    pub leaders: Vec<LeaderRun>,
    pub people: Vec<Leader>,
    pub elections: Vec<Election>,
    pub government_changes: Vec<GovernmentChange>,
    pub government_unobserved: Vec<HourSpan>,
    pub unobserved: Vec<HourSpan>,
    government_gaps: Vec<(i32, i32)>,
    last: Option<(i32, (u32, u32))>,
    look_last: Option<(i32, (u32, u32))>,
    election: Option<((u32, u32), i64)>,
    leader: Option<(Option<u32>, Option<u32>)>,
}

#[derive(Clone, Debug, Default)]
struct RulingHistory {
    runs: Vec<RulingRun>,
    name_runs: Vec<PartyNameRun>,
    namings: Vec<(PartyNaming, Result<String, String>)>,
    naming: Option<((String, u32, Option<String>), u32)>,
}

impl RulingHistory {
    fn observe(
        &mut self, token: &str, raw: u32, key: Option<String>, (hour, end): (i32, i32), continues: bool, renamed: bool,
        tags: impl FnOnce() -> io::Result<(Option<String>, Result<String, String>)>,
    ) -> io::Result<()> {
        let inputs = (token.to_owned(), raw, key.clone());
        let index = match &self.naming {
            Some((held, index)) if *held == inputs && continues && !renamed => *index,
            _ => {
                let (own, selected) = tags()?;
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
        match self.name_runs.last_mut() {
            Some(run) if continues && run.name == index && i64::from(run.last_hour) + 1 == i64::from(hour) => run.last_hour = end,
            _ => self.name_runs.push(PartyNameRun { name: index, first_hour: hour, last_hour: end }),
        }
        match self.runs.last_mut() {
            Some(run) if continues && run.token == token && i64::from(run.last_hour) + 1 == i64::from(hour) => run.last_hour = end,
            previous => {
                let starts_segment = !(continues && previous.is_some_and(|run| i64::from(run.last_hour) + 1 == i64::from(hour)));
                self.runs.push(RulingRun { token: token.to_owned(), first_hour: hour, last_hour: end, starts_segment });
            }
        }
        Ok(())
    }

    fn party_names(&self, politics: Option<&PoliticsContent>, warnings: &mut Vec<String>) -> Vec<Option<String>> {
        self.namings
            .iter()
            .map(|(naming, selected)| {
                let politics = politics?;
                let Some((key, fallback)) = party_key_for_tag_checked(naming, selected.as_ref().ok().map(String::as_str), |key| politics.defines(key)) else {
                    warnings.push(format!("party name: {}", selected.as_ref().err().cloned().unwrap_or_else(|| "party own tag is unavailable".into())));
                    return None;
                };
                warnings.extend(fallback);
                politics.localised(&key, View::Plain)
            })
            .collect()
    }
}

const SOURCES: [TableId; 3] = [TableId::PoliticalParties, TableId::CountryAppearance, TableId::Politics];

#[derive(Clone, Copy, PartialEq, Eq)]
enum PartyRead {
    Ruling,
    Full,
}

struct Columns {
    scope: PartyRead,
    party: [usize; 5],
    look: [usize; 2],
    election: usize,
    character: [usize; 2],
    link: usize,
    country: [usize; 2],
    original: [usize; 2],
    literal: usize,
    reserved: [usize; 3],
    government: government::Columns,
}

impl Columns {
    fn new(scope: PartyRead) -> io::Result<Self> {
        let field = dashboard_declare::field;
        let mut party = [0; 5];
        for (at, name) in ["is_ruling", "token", "tag", "name_default", "name"].into_iter().enumerate() {
            party[at] = field(TableId::PoliticalParties, name)?;
        }
        Ok(Self {
            scope,
            party,
            look: [field(TableId::CountryAppearance, "leader_ideology")?, field(TableId::CountryAppearance, "leader_character")?],
            election: field(TableId::Politics, "last_election")?,
            character: [field(TableId::Characters, "name_token_id")?, field(TableId::Characters, "token")?],
            link: field(TableId::CountryLinks, "canonical_slot")?,
            country: [field(TableId::Countries, "canonical_slot")?, field(TableId::Countries, "tag")?],
            original: [field(TableId::CountryAppearance, "original_tag")?, field(TableId::CountryAppearance, "original_tag_reference")?],
            literal: field(TableId::LexicalTokens, "literal")?,
            reserved: [
                field(TableId::ReservedCountryNaming, "tag")?,
                field(TableId::ReservedCountryNaming, "original_tag")?,
                field(TableId::ReservedCountryNaming, "original_tag_reference")?,
            ],
            government: government::Columns::new()?,
        })
    }
}

fn unless_withheld<T: Default>(read: io::Result<T>) -> io::Result<T> {
    match read {
        Err(error) if matches!(Damage::of(&error).map(|damage| &damage.scope), Some(DamageScope::Withheld { .. })) => Ok(T::default()),
        read => read,
    }
}

fn numbers(history: &History, column: usize) -> BTreeSet<u32> {
    let Ok(at) = history.columns.binary_search(&column) else { return BTreeSet::new() };
    history
        .keys
        .iter()
        .flat_map(|(_, steps)| steps.iter().filter_map(|step| step.cells.as_ref().and_then(|cells| number(cells[at]))))
        .filter_map(|value| u32::try_from(value).ok())
        .collect()
}

fn steps_into(history: &History, hours: &mut Vec<i32>) {
    hours.extend(history.keys.iter().flat_map(|(_, steps)| steps.iter().map(|step| step.hour)));
}

fn read_bounds_into(reads: &[(i32, i32)], hours: &mut Vec<i32>) {
    for &(start, end) in reads {
        hours.push(start);
        hours.push(end.saturating_add(1));
    }
}

fn settled(mut hours: Vec<i32>, first: i32, last: i32) -> Vec<i32> {
    hours.retain(|hour| (first..=last).contains(hour));
    hours.sort_unstable();
    hours.dedup();
    hours
}

const NAMING: [TableId; 4] = [TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance, TableId::ReservedCountryNaming];

type NamingReads = [Vec<(i32, i32)>; NAMING.len()];

struct Naming {
    hours: Vec<i32>,
    reads: NamingReads,
}

fn read_at(reads: &[(i32, i32)], hour: i32) -> bool {
    reads.get(reads.partition_point(|&(_, to)| to < hour)).is_some_and(|&(from, _)| from <= hour)
}

fn naming_changes(recording: &replay_recording::columnar_frames::Recording, run: u32, columns: &Columns, parties: &History, who: (u32, u32), first: i32, last: i32) -> io::Result<Naming> {
    let mut reads: NamingReads = Default::default();
    for (table, held) in NAMING.into_iter().zip(&mut reads) {
        *held = unless_withheld(complete_reads(recording, run, table, first, last))?;
    }
    let raws: Vec<u128> = numbers(parties, columns.party[2]).into_iter().map(u128::from).collect();
    if raws.is_empty() {
        return Ok(Naming { hours: Vec::new(), reads });
    }
    let mut hours = Vec::new();
    let links = unless_withheld(lanes::history(recording, run, TableId::CountryLinks, &[columns.link], Keys::These(&raws), first, last))?;
    steps_into(&links, &mut hours);
    let slots = numbers(&links, columns.link);
    let countries = unless_withheld(lanes::history(recording, run, TableId::Countries, &columns.country, Keys::All, first, last))?;
    let mut ids = Vec::new();
    if let Ok(slot) = countries.columns.binary_search(&columns.country[0]) {
        for (key, steps) in &countries.keys {
            let own = *key == u128::from(who.1);
            let linked = steps.iter().any(|step| step.cells.as_ref().and_then(|cells| number(cells[slot])).and_then(|slot| u32::try_from(slot).ok()).is_some_and(|slot| slots.contains(&slot)));
            if own || linked {
                ids.push(*key);
                hours.extend(steps.iter().map(|step| step.hour));
            }
        }
    }
    if !ids.is_empty() {
        steps_into(&unless_withheld(lanes::history(recording, run, TableId::CountryAppearance, &columns.original, Keys::These(&ids), first, last))?, &mut hours);
    }
    let reserved = raws.first() == Some(&0) || slots.contains(&0);
    if reserved {
        steps_into(&unless_withheld(lanes::history(recording, run, TableId::ReservedCountryNaming, &columns.reserved, Keys::All, first, last))?, &mut hours);
    }
    for held in &reads[..if reserved { NAMING.len() } else { NAMING.len() - 1 }] {
        read_bounds_into(held, &mut hours);
    }
    Ok(Naming { hours: settled(hours, first, last), reads })
}

fn character_changes(recording: &replay_recording::columnar_frames::Recording, run: u32, columns: &Columns, looks: &History, first: i32, last: i32) -> io::Result<Vec<i32>> {
    let ids: Vec<u128> = numbers(looks, columns.look[1]).into_iter().map(u128::from).collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut hours = Vec::new();
    let characters = unless_withheld(lanes::history(recording, run, TableId::Characters, &columns.character, Keys::These(&ids), first, last))?;
    steps_into(&characters, &mut hours);
    let words: Vec<u128> = numbers(&characters, columns.character[0]).into_iter().filter(|word| *word != 0).map(u128::from).collect();
    if !words.is_empty() {
        steps_into(&unless_withheld(lanes::history(recording, run, TableId::LexicalTokens, &[columns.literal], Keys::These(&words), first, last))?, &mut hours);
    }
    read_bounds_into(&unless_withheld(complete_reads(recording, run, TableId::Characters, first, last))?, &mut hours);
    Ok(settled(hours, first, last))
}

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

pub(crate) fn observed_reads(recording: &replay_recording::columnar_frames::Recording, run: u32, table: TableId, first: i32, last: i32) -> io::Result<Vec<(i32, i32)>> {
    reads_kept(recording, run, table, first, last, lanes::available_reads, replay_recording::columnar_frames::available)
}

pub(crate) fn complete_reads(recording: &replay_recording::columnar_frames::Recording, run: u32, table: TableId, first: i32, last: i32) -> io::Result<Vec<(i32, i32)>> {
    reads_kept(recording, run, table, first, last, lanes::complete_reads, replay_recording::columnar_frames::complete)
}

type Reads = fn(&replay_recording::columnar_frames::Recording, u32, TableId, i32, i32) -> io::Result<Vec<(i32, i32)>>;

fn reads_kept(recording: &replay_recording::columnar_frames::Recording, run: u32, table: TableId, first: i32, last: i32, reads: Reads, kept: fn(replay_columnar::manifest::Status) -> bool) -> io::Result<Vec<(i32, i32)>> {
    let marked = reads(recording, run, table, first, last)?;
    if !marked.is_empty() {
        return Ok(marked);
    }
    let held = recording.run(run)?;
    if !lanes::reads(recording, run, table, held.first_hour, held.last_hour())?.is_empty() {
        return Ok(marked);
    }
    Ok(lanes::statuses(recording, run, table, first, last)?
        .into_iter()
        .filter(|held| kept(held.status))
        .map(|held| (held.first, held.last))
        .collect())
}

fn read(query: &Query<'_>, tag: &Country, scope: PartyRead, cancelled: &AtomicBool) -> io::Result<Parties> {
    let columns = Columns::new(scope)?;
    let mut parties = Parties::default();
    let runs = query.holders.runs(tag);
    let mut holes: Vec<(i32, i32)> = query.timeline.unheld.iter().map(|unheld| (unheld.first, unheld.last)).collect();
    for stretch in &query.timeline.stretches {
        check_cancel(cancelled)?;
        parties.last = None;
        parties.look_last = None;
        let mut covered = stretch.first;
        let mut held = Vec::new();
        for run in runs.iter().filter(|run| run.last >= stretch.first && run.first <= stretch.last) {
            let (first, last) = (run.first.max(stretch.first), run.last.min(stretch.last));
            if covered < first {
                holes.push((covered, first - 1));
                parties.last = None;
                parties.look_last = None;
            }
            covered = last.saturating_add(1);
            match run.holder {
                Holder::Found { slot, id } => {
                    held.push((first, last, slot));
                    parties.country(query, &columns, stretch.run, (slot, id), tag, first, last, cancelled)?;
                }
                Holder::Unresolved => {
                    holes.push((first, last));
                    parties.last = None;
                    parties.look_last = None;
                }
            }
        }
        if covered <= stretch.last {
            holes.push((covered, stretch.last));
        }
        if scope == PartyRead::Full {
            check_cancel(cancelled)?;
            government::read(query, &columns.government, stretch.run, &held, (stretch.first, stretch.last), &mut parties.government_changes, &mut parties.government_gaps)?;
        }
    }
    let mut government = holes.clone();
    government.append(&mut parties.government_gaps);
    government.sort_unstable();
    for (first, last) in government {
        mark(&mut parties.government_unobserved, first, last);
    }
    parties.government_changes.sort_unstable_by_key(|change| (change.hour, change.witnessed));
    holes.extend(parties.unobserved.drain(..).map(|span| (span.first_hour, span.last_hour)));
    holes.sort_unstable();
    for (first, last) in holes {
        mark(&mut parties.unobserved, first, last);
    }
    Ok(parties)
}

impl Parties {
    fn country(&mut self, query: &Query<'_>, columns: &Columns, run: u32, who: (u32, u32), asked: &Country, first: i32, last: i32, cancelled: &AtomicBool) -> io::Result<()> {
        let recording = query.replay.recording();
        let key = u128::from(who.1);
        let mut observed = Vec::with_capacity(SOURCES.len());
        for table in SOURCES {
            let reads = if table == TableId::PoliticalParties || columns.scope == PartyRead::Full {
                cost::storage(|| observed_reads(recording, run, table, first, last))?
            } else {
                Vec::new()
            };
            observed.push(reads);
        }
        let held = cost::storage(|| lanes::history(recording, run, TableId::PoliticalParties, &columns.party, Keys::Range(key << 32, key << 32 | 0xFFFF_FFFF), first, last))?;
        let (looks, politics) = match columns.scope {
            PartyRead::Ruling => (History::default(), History::default()),
            PartyRead::Full => (
                cost::storage(|| lanes::history(recording, run, TableId::CountryAppearance, &columns.look, Keys::These(&[key]), first, last))?,
                cost::storage(|| lanes::history(recording, run, TableId::Politics, &[columns.election], Keys::These(&[key]), first, last))?,
            ),
        };
        let renames = cost::storage(|| naming_changes(recording, run, columns, &held, who, first, last))?;
        let recasts = cost::storage(|| character_changes(recording, run, columns, &looks, first, last))?;
        check_cancel(cancelled)?;
        let mut held = Stepper::new(&held);
        let mut looks = Stepper::new(&looks);
        let mut politics = Stepper::new(&politics);
        let mut hours: Vec<i32> = held.hours().chain(looks.hours()).chain(politics.hours()).chain(renames.hours.iter().copied()).chain(recasts.iter().copied()).chain([first]).collect();
        read_bounds_into(&observed.concat(), &mut hours);
        let hours = settled(hours, first, last);
        let mut unread = [0; SOURCES.len()];
        for (index, &hour) in hours.iter().enumerate() {
            check_cancel(cancelled)?;
            let end = hours.get(index + 1).map_or(last, |next| next - 1);
            held.advance(hour);
            looks.advance(hour);
            politics.advance(hour);
            let read = std::array::from_fn(|source| {
                let reads = &observed[source];
                while reads.get(unread[source]).is_some_and(|&(_, until)| until < hour) {
                    unread[source] += 1;
                }
                reads.get(unread[source]).is_some_and(|&(from, _)| from <= hour)
            });
            let changed = [renames.hours.binary_search(&hour).is_ok(), recasts.binary_search(&hour).is_ok()];
            self.observe(query, run, (hour, end), who, (asked, &renames.reads), columns, read, changed, [&held, &looks, &politics])?;
        }
        Ok(())
    }

    fn observe(&mut self, query: &Query<'_>, run: u32, (hour, end): (i32, i32), who: (u32, u32), naming: (&Country, &NamingReads), columns: &Columns, read: [bool; SOURCES.len()], [renamed, recast]: [bool; 2], [held, looks, politics]: [&Stepper<'_>; SOURCES.len()]) -> io::Result<()> {
        let [parties_read, looks_read, politics_read] = read;
        if parties_read {
            self.observe_parties(query, run, (hour, end), who, naming, columns, renamed, held)?;
        } else {
            mark(&mut self.unobserved, hour, end);
            self.last = None;
        }
        if looks_read {
            self.observe_leader(query, run, (hour, end), who, columns, recast, looks)?;
        } else {
            self.look_last = None;
        }
        if politics_read {
            self.observe_election(hour, who, politics);
        }
        Ok(())
    }

    fn observe_parties(&mut self, query: &Query<'_>, run: u32, (hour, end): (i32, i32), who: (u32, u32), (asked, reads): (&Country, &NamingReads), columns: &Columns, renamed: bool, held: &Stepper<'_>) -> io::Result<()> {
        let continues = self.last.is_some_and(|(at, seen)| seen == who && i64::from(at) + 1 == i64::from(hour));
        self.last = Some((end, who));
        let places = columns.party.map(|column| held.place(column));
        if let [Some(ruling), Some(token), Some(party_tag), Some(default), Some(name)] = places {
            for (key, cells) in held.keyed() {
                let Some(token) = held.text(cells[token]).filter(|token| !token.is_empty()).and_then(|token| std::str::from_utf8(token).ok()) else { continue };
                self.any_party = true;
                if columns.scope == PartyRead::Full {
                    let series = self.parties.entry(token.to_owned()).or_insert_with(|| PartySeries { token: token.to_owned(), parts: Vec::new() });
                    let part = key as u32;
                    match series.parts.iter_mut().rev().find(|owned| owned.part == part) {
                        Some(owned) if continues && i64::from(owned.last_hour) + 1 == i64::from(hour) => owned.last_hour = end,
                        _ => series.parts.push(PartOwnership { part, first_hour: hour, last_hour: end }),
                    }
                }
                if number(cells[ruling]).is_none_or(|ruling| ruling == 0) {
                    continue;
                }
                let raw = number(cells[party_tag]).and_then(|raw| u32::try_from(raw).ok()).unwrap_or_default();
                let key = (number(cells[default]) == Some(0)).then(|| held.text(cells[name]).map(|name| String::from_utf8_lossy(name).into_owned()).unwrap_or_default());
                self.ruling.observe(token, raw, key, (hour, end), continues, renamed, || party_tags(query, run, hour, raw, who, &query.holders.name(asked, hour), reads))?;
            }
        }
        Ok(())
    }

    fn observe_leader(&mut self, query: &Query<'_>, run: u32, (hour, end): (i32, i32), who: (u32, u32), columns: &Columns, recast: bool, looks: &Stepper<'_>) -> io::Result<()> {
        let continues = self.look_last.is_some_and(|(at, seen)| seen == who && i64::from(at) + 1 == i64::from(hour));
        self.look_last = Some((end, who));
        if let (Some(ideology), Some(character), Some(cells)) = (looks.place(columns.look[0]), looks.place(columns.look[1]), looks.shown().next()) {
            let ideology = looks.text(cells[ideology]).map(|text| String::from_utf8_lossy(text).into_owned()).unwrap_or_default();
            let id = number(cells[character]).and_then(|id| u32::try_from(id).ok());
            let person = match self.leader {
                Some((held, person)) if held == id && continues && !recast => person,
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
        Ok(())
    }

    fn observe_election(&mut self, hour: i32, who: (u32, u32), politics: &Stepper<'_>) {
        if self.election.is_some_and(|(seen, _)| seen != who) {
            self.election = None;
        }
        let Some(date) = politics.shown().next().and_then(|cells| number(cells[0])) else { return };
        if let Some((_, held)) = self.election
            && held != date
            && let Ok(at) = i32::try_from(date)
            && at <= hour
            && !engine_native_politics::cpolitical_status::needs_election_cache(at)
        {
            self.elections.push(Election { hour: at, witnessed: hour });
        }
        self.election = Some((who, date));
    }
}

fn party_tags(query: &Query<'_>, run: u32, hour: i32, raw: u32, who: (u32, u32), tag: &str, reads: &NamingReads) -> io::Result<(Option<String>, Result<String, String>)> {
    let mut frame = cost::storage(|| replay_lanes::at::frame(query.replay.recording(), run, hour, &NAMING.map(replay_lanes::at::Read::all)))?;
    for (table, held) in NAMING.into_iter().zip(reads) {
        if !read_at(held, hour) {
            frame.tables[table.index()] = TableView::Unavailable { reason: "not read at this hour".into() }.into();
        }
    }
    if raw == 0 {
        return Ok((
            content_reader_country_names::country_names::reserved_zero_tag(&frame).ok().map(str::to_owned),
            content_reader_country_names::country_names::original_or_own_tag(&frame, 0).map(str::to_owned),
        ));
    }
    let resolved = (|| -> Result<(String, Result<String, String>), String> {
        let links = frame.complete::<tables::CountryLinks>().map_err(|why| format!("country links is unavailable: {why}"))?;
        let slot = link_slot(links, raw).ok_or_else(|| "the party's country has no recorded country link".to_owned())?;
        if slot == 0 {
            return Ok((
                content_reader_country_names::country_names::reserved_zero_tag(&frame)?.to_owned(),
                content_reader_country_names::country_names::original_or_own_tag(&frame, 0).map(str::to_owned),
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
            .and_then(|appearance| content_reader_country_names::country_names::original_tag(country, appearance))
            .map(str::to_owned);
        Ok((own, selected))
    })();
    Ok(match resolved {
        Ok((own, selected)) => (Some(own), selected),
        Err(reason) => (None, Err(reason)),
    })
}

fn leader_at(query: &Query<'_>, run: u32, hour: i32, id: u32) -> io::Result<Leader> {
    let recording = query.replay.recording();
    let key = [u128::from(id)];
    let reads = [
        replay_lanes::at::Read { table: TableId::Characters, columns: None, keys: replay_lanes::at::Keys::These(&key) },
        replay_lanes::at::Read { table: TableId::CharacterPortraits, columns: None, keys: replay_lanes::at::Keys::These(&key) },
    ];
    let frame = cost::storage(|| replay_lanes::at::frame(recording, run, hour, &reads))?;
    let row = frame.complete::<tables::Characters>().ok().and_then(|rows| rows.iter().find(|row| row.id == id)).cloned();
    let token = match &row {
        Some(row) => {
            let words = u32::try_from(row.name_token_id).ok().map(u128::from).into_iter().collect::<Vec<_>>();
            let lexicon: DecodedFrame = cost::storage(|| replay_lanes::at::frame(recording, run, hour, &[replay_lanes::at::Read { table: TableId::LexicalTokens, columns: None, keys: replay_lanes::at::Keys::These(&words) }]))?;
            lexicon.complete::<tables::LexicalTokens>().ok().and_then(|lexical| character_script_token(row, lexical)).unwrap_or_default().to_owned()
        }
        None => String::new(),
    };
    let name = row.as_ref().and_then(|row| std::str::from_utf8(&row.token).ok()).unwrap_or_default().to_owned();
    let portrait = recorded_portrait(frame.complete::<tables::CharacterPortraits>().ok(), id, PORTRAIT_CIVILIAN, PORTRAIT_LARGE);
    Ok(Leader { character: IdeaCharacter { id, token, name }, portrait })
}

fn chart_color(color: [f32; 4]) -> [u8; 3] {
    [color[0], color[1], color[2]].map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)
}

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

pub(crate) fn stack_order(parties: &BTreeMap<String, PartySeries>, ideologies: &[IdeologyInfo]) -> Vec<PartySeries> {
    let mut order: Vec<&PartySeries> = parties.values().collect();
    order.sort_by_key(|party| ideologies.iter().position(|ideology| ideology.token == party.token).unwrap_or(usize::MAX));
    order.into_iter().cloned().collect()
}

fn ideology_content<'a>(politics: Result<&'a PoliticsContent, String>, warnings: &mut Vec<String>) -> (Option<&'a PoliticsContent>, Vec<IdeologyInfo>) {
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
    let ideologies = politics.map_or_else(Vec::new, |politics| ideology_infos(politics, warnings));
    (politics, ideologies)
}

fn finish_warnings(parties: &Parties, politics: Option<&PoliticsContent>, tag: &str, warnings: &mut Vec<String>) {
    if !parties.any_party {
        warnings.push(format!("{tag}: no recorded parties in this branch"));
    }
    if let Some(politics) = politics {
        warnings.extend(politics.catalogue.refusal_lines());
    }
}

pub(crate) fn present(politics: Result<&PoliticsContent, String>, params: &CountryParams, tag: &str, parties: &Parties, cancelled: &AtomicBool) -> io::Result<PartyHistoryReply> {
    let mut warnings = Vec::new();
    let (politics, ideologies) = ideology_content(politics, &mut warnings);
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
    let party_names = parties.ruling.party_names(politics, &mut warnings);
    if people.iter().any(|leader| leader.portrait.is_some()) {
        warnings.push(format!(
            "{}; visual assets are outside content hash coverage.",
            dashboard_base::products::artwork::CURRENT_INSTALL_ART_LABEL
        ));
    }
    finish_warnings(parties, politics, tag, &mut warnings);
    Ok(PartyHistoryReply {
        branch_id: params.branch_id.clone(),
        tag: tag.to_owned(),
        asked: params.country.clone(),
        decimals: POPULARITY_DECIMALS,
        parties: stack_order(&parties.parties, &ideologies),
        ruling: parties.ruling.runs.clone(),
        leaders: parties.leaders.clone(),
        party_name_runs: parties.ruling.name_runs.clone(),
        party_names,
        people,
        ideology_types,
        elections: parties.elections.clone(),
        government_changes: parties.government_changes.clone(),
        government_unobserved: parties.government_unobserved.clone(),
        ideologies,
        unobserved: parties.unobserved.clone(),
        warnings,
    })
}

#[derive(Default)]
struct Kept(Option<(u64, Country, PartyRead, Arc<Parties>)>);

impl Kept {
    fn for_country(&self, params: &CountryParams, want: PartyRead) -> Option<Arc<Parties>> {
        let (branch, country, held, parties) = self.0.as_ref()?;
        (*branch == params.branch() && *country == params.country && (*held == PartyRead::Full || *held == want)).then(|| Arc::clone(parties))
    }
}

fn history(source: &Source, query: &Query<'_>, params: &CountryParams, scope: PartyRead, cancelled: &AtomicBool) -> io::Result<Arc<Parties>> {
    let state = source.state::<Kept>();
    if let Some(read) = state.lock().unwrap_or_else(PoisonError::into_inner).for_country(params, scope) {
        return Ok(read);
    }
    let read = Arc::new(read(query, &params.country, scope, cancelled)?);
    let mut kept = state.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(ready) = kept.for_country(params, scope) {
        return Ok(ready);
    }
    kept.0 = Some((params.branch(), params.country.clone(), scope, Arc::clone(&read)));
    Ok(read)
}

pub(super) struct RulingParties {
    pub runs: Vec<RulingRun>,
    pub unobserved: Vec<HourSpan>,
    pub ideologies: Vec<IdeologyInfo>,
    pub warnings: Vec<String>,
}

fn present_ruling(politics: Result<&PoliticsContent, String>, tag: &str, parties: &Parties) -> RulingParties {
    let mut warnings = Vec::new();
    let (politics, ideologies) = ideology_content(politics, &mut warnings);
    parties.ruling.party_names(politics, &mut warnings);
    finish_warnings(parties, politics, tag, &mut warnings);
    RulingParties { runs: parties.ruling.runs.clone(), unobserved: parties.unobserved.clone(), ideologies, warnings }
}

pub(super) fn ruling(source: &Source, query: &Query<'_>, params: &CountryParams, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<RulingParties> {
    let parties = history(source, query, params, PartyRead::Ruling, cancelled)?;
    let politics = context.content().and_then(|content| politics_content(source, content)).map_err(|error| error.to_string());
    Ok(present_ruling(politics.as_deref().map_err(Clone::clone), &query.holders.nearest_name(&params.country, query.timeline.last), &parties))
}

pub struct PartyHistory;

impl Product for PartyHistory {
    const KEY: &'static str = "party-history";
    const CONTENT_DEPENDENT: bool = true;
    type Params = CountryParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = PartyHistoryReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            CountryParams::decl(),
            PartyHistoryReply::decl(),
            PartySeries::decl(),
            PartOwnership::decl(),
            Election::decl(),
            GovernmentChange::decl(),
            GovernmentCauseKind::decl(),
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
        let query = dashboard_base::queries::query(source, params.branch())?;
        let parties = history(source, &query, params, PartyRead::Full, cancelled)?;
        let politics = context.content().and_then(|content| politics_content(source, content)).map_err(|error| error.to_string());
        let reply = present(politics.as_deref().map_err(Clone::clone), params, &query.holders.nearest_name(&params.country, query.timeline.last), &parties, cancelled)?;
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

mod government;

#[cfg(test)]
mod tests;
