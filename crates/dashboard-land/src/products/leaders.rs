
pub mod career;
#[cfg(test)]
mod tests;

use dashboard_base::products::{LoadContext, Product, at_hour::{At, AtHour, Read, ReadsAt}, politics_content::{PoliticsContent, recorded_politics_content}};
use dashboard_intelligence::products::intelligence::Tags;
use dashboard_base::{Source, catalogue::{self, Catalogue, View}};
use dashboard_query::Query;
use dashboard_text::triggers::{Evaluator, LeaderVisibility, RECORDED_TABLES, World, core_gap};
use content_reader_key_names::unit_leader::Leaders;
use replay::campaign::Replay;
use replay_data::rows::{CharacterRow, LexicalTokenRow, TableId, UnitLeaderRow, UnitLeaderTraitRow, tables};
use replay_columnar::store::TableView as StoreView;
use replay_frame::frame::{DecodedFrame, Rowset};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const KIND_ARMY: u8 = engine_native::cunit_leader::KIND_ARMY;
pub const KIND_NAVY: u8 = engine_native::cunit_leader::KIND_NAVY;
const PORTRAIT_ARMY: u32 = 1;
const PORTRAIT_NAVY: u32 = 2;
pub const RESULTS_MISSING: &str = "Outcomes require a recorded native result linked to the observed combat. Unfinished combats and missing result coverage remain unknown.";
const UNCONFIGURED: &str = "Game content is not configured";

pub fn handle(row: &UnitLeaderRow) -> u64 {
    (u64::from(row.id) << 32) | u64::from(row.reference_type)
}

pub fn parse_branch(branch_id: &str) -> Result<u64, &'static str> {
    branch_id.parse::<u64>().map_err(|_| "Invalid branch")
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum LeaderKinds {
    #[default]
    All,
    Land,
    Navy,
}

impl LeaderKinds {
    pub fn includes(self, kind: u8) -> bool {
        match self {
            Self::All => kind == KIND_ARMY || kind == KIND_NAVY,
            Self::Land => kind == KIND_ARMY,
            Self::Navy => kind == KIND_NAVY,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", tag = "kind", try_from = "SelectionAsked")]
pub enum LeaderSelection {
    All,
    Countries { countries: Vec<dashboard_base::products::Country> },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SelectionAsked {
    kind: SelectionKind,
    countries: Option<Vec<dashboard_base::products::Country>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum SelectionKind {
    All,
    Countries,
}

impl TryFrom<SelectionAsked> for LeaderSelection {
    type Error = &'static str;

    fn try_from(asked: SelectionAsked) -> Result<Self, Self::Error> {
        match (asked.kind, asked.countries) {
            (SelectionKind::All, _) => Ok(Self::All),
            (SelectionKind::Countries, Some(countries)) => Ok(Self::Countries { countries }),
            (SelectionKind::Countries, None) => Err("A leader selection of countries names them"),
        }
    }
}

impl LeaderSelection {
    fn keeps(&self, owner: Option<(u32, &str)>) -> bool {
        match self {
            Self::All => true,
            Self::Countries { countries } => owner.is_some_and(|(slot, tag)| countries.iter().any(|country| dashboard_world::products::holder_tags::matches(country, (slot, 0), tag))),
        }
    }
}

fn owned(tags: &Tags, raw: u32) -> Option<(u32, String)> {
    Some((tags.slot(raw)?, tags.tag(raw)?))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaderTrait {
    pub token: String,
    pub name: Option<String>,
    pub icon: Option<String>,
    #[doc = " Why the icon is not drawn."]
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaderRole {
    pub owner: Option<String>,
    pub kind: u8,
    pub category: i32,
    pub rank: Option<String>,
    pub attribute_names: [Option<String>; 4],
    pub maximum_skill: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaderWithheld {
    pub reference: String,
    pub owner: Option<String>,
    pub kind: u8,
    pub name: Option<String>,
    pub reason: String,
}

pub struct Content {
    catalogue: Result<Arc<Catalogue>, String>,
    politics: Result<Arc<PoliticsContent>, String>,
    leaders: Result<Arc<Leaders>, String>,
}

type HeldLeaders = content_reader_layers::content::diagnostics::Once<Result<Arc<Leaders>, String>>;

impl Content {
    pub fn load(source: &Source, load: &LoadContext) -> Self {
        let Some(options) = load.content.as_ref() else {
            return Self { catalogue: Err(UNCONFIGURED.into()), politics: Err(UNCONFIGURED.into()), leaders: Err(UNCONFIGURED.into()) };
        };
        let catalogue = catalogue::catalogue(source, options).map_err(|error| error.to_string());
        let politics = recorded_politics_content(source, options).map_err(|error| error.to_string());
        let leaders = catalogue.as_ref().map_err(Clone::clone).and_then(|catalogue| {
            catalogue
                .content
                .layers
                .derived::<HeldLeaders>()
                .get_or_init(|| Leaders::load(&catalogue.content.layers, &|_| None).map(Arc::new).map_err(|error| error.to_string()))
                .clone()
        });
        Self { catalogue, politics, leaders }
    }

    fn localised(&self, key: &str) -> Option<String> {
        self.catalogue.as_ref().ok()?.localised(key, View::Plain)
    }

    pub fn role(&self, owner: Option<String>, kind: u8, category: i32, skill: Option<i32>) -> LeaderRole {
        let rank = match category {
            0 => self.localised("fm_leader"),
            1 => self.localised("cc_leader"),
            2 => self.localised("navy_leader"),
            _ => None,
        };
        let keys = if kind == KIND_NAVY {
            ["SKILL_NAVY_LEADER_ATTACK", "SKILL_NAVY_LEADER_DEFENSE", "SKILL_NAVY_LEADER_MANEUVERING", "SKILL_NAVY_LEADER_COORDINATION"]
        } else {
            ["SKILL_ATTACK", "SKILL_DEFENSE", "SKILL_PLANNING", "SKILL_LOGISTICS"]
        };
        let attribute_names = keys.map(|key| self.localised(key));
        let maximum_skill = skill.and_then(|level| self.maximum_skill(level, category));
        LeaderRole { owner, kind, category, rank, attribute_names, maximum_skill }
    }

    fn maximum_skill(&self, level: i32, category: i32) -> Option<i32> {
        let skills = &self.leaders.as_ref().ok()?.skills;
        if !skills.conditions().is_empty() || level < 0 {
            return None;
        }
        let mut ceiling = level;
        while skills.next_required_experience(ceiling, category)?.is_some() {
            ceiling = ceiling.checked_add(1)?;
        }
        Some(ceiling)
    }

    pub fn next_required(&self, level: i32, category: i32) -> Option<i64> {
        let required = self.leaders.as_ref().ok()?.skills.next_required_experience(level, category)??;
        Some(i64::from(required) * 100_000)
    }

    pub fn trait_required(&self, token: &str) -> Option<i64> {
        Some(i64::from(self.leaders.as_ref().ok()?.traits.get(token)?.required_experience()) * 100_000)
    }

    pub fn trait_name(&self, token: &str) -> Option<String> {
        self.localised(token)
    }

    pub fn trait_info(&self, token: &str) -> LeaderTrait {
        let name = self.trait_name(token);
        let drawn = match (&self.catalogue, &self.leaders) {
            (Ok(shown), Ok(leaders)) => catalogue::draw(|| format!("trait {token} icon"), shown, |catalogue| trait_icon(catalogue, leaders, token)).get().clone(),
            (Err(why), _) | (_, Err(why)) => Err(why.clone()),
        };
        let (icon, missing) = match drawn {
            Ok(icon) => (Some(icon), None),
            Err(why) => (None, Some(why)),
        };
        LeaderTrait { token: token.to_owned(), name, icon, missing }
    }

    pub fn person_name(&self, character: Option<&CharacterRow>, lexical: Option<&[LexicalTokenRow]>) -> Option<String> {
        let character = character?;
        let saved = std::str::from_utf8(&character.token).ok()?;
        let token = lexical.and_then(|rows| dashboard_text::identity::character_script_token_in(character, Rowset::Decoded(rows)));
        self.politics.as_ref().ok()?.character_name(token, saved)
    }

    pub fn character_name(&self, saved: &str) -> Option<String> {
        self.politics.as_ref().ok()?.character_name(None, saved)
    }

    pub fn stored_text(&self, bytes: &[u8]) -> Option<String> {
        let catalogue = self.catalogue.as_ref().ok()?;
        dashboard_text::scripted_text::stored_text(&catalogue.localisation, &catalogue.recorded_language, bytes, "formation name").ok()
    }

    pub fn missing(&self) -> Vec<String> {
        [&self.catalogue.as_ref().err(), &self.politics.as_ref().err(), &self.leaders.as_ref().err()]
            .into_iter()
            .flatten()
            .map(|why| format!("Leader content unavailable: {why}"))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

fn trait_icon(catalogue: &Catalogue, leaders: &Leaders, token: &str) -> Result<String, String> {
    let name = leaders.traits.assignment_sprite(token).map_err(str::to_owned)?.ok_or_else(|| format!("Trait {token} is not registered"))?;
    let sprite = catalogue::created_gui_sprite(&catalogue.sprites, &name).map_err(|error| error.to_string())?;
    let decode = *catalogue.gui_decode.as_ref().map_err(Clone::clone)?;
    catalogue::sprite_frame_png(&catalogue.content.layers, sprite, decode, 0, 32).map_err(|error| error.to_string())
}

fn world_of<'a, 'w>(frame: &'a DecodedFrame, content: &'a Content, world: &'w mut Option<Result<World<'a>, String>>) -> Result<&'w World<'a>, String> {
    world
        .get_or_insert_with(|| {
            let catalogue = content.catalogue.as_ref().map_err(|why| format!("Leader visibility needs the recorded content: {why}"))?;
            catalogue.scripted_content()?.recorded_world(frame)
        })
        .as_ref()
        .map_err(Clone::clone)
}

pub fn visible<'a>(row: &UnitLeaderRow, frame: &'a DecodedFrame, content: &'a Content, world: &mut Option<Result<World<'a>, String>>) -> Result<bool, String> {
    if row.kind == KIND_ARMY && (row.category as u32) < 2 && row.captured != 0 {
        return Ok(false);
    }
    if !needs_world(row) {
        return Ok(true);
    }
    let world = world_of(frame, content, world)?;
    match world.decoded_leader_visibility(row) {
        LeaderVisibility::Decided(value) => value.decided(),
        LeaderVisibility::Unconditioned => Ok(true),
        visibility => {
            if let Some(gap) = core_gap(frame) {
                return Err(gap);
            }
            Evaluator::new(world, None).decide_leader_visibility(row, visibility, true).decided()
        }
    }
}

fn conditioned(row: &UnitLeaderRow, world: &World<'_>) -> bool {
    needs_world(row) && matches!(world.decoded_leader_visibility(row), LeaderVisibility::Conditioned(_))
}

#[derive(Default)]
pub struct Verdicts {
    catalogue: Option<Arc<Catalogue>>,
    at: HashMap<(u32, i32), HashMap<u64, Result<bool, String>>>,
}

pub(crate) fn forget(source: &Source) {
    *source.state::<Verdicts>().lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Verdicts::default();
}

pub fn needs_world(row: &UnitLeaderRow) -> bool {
    row.visibility_choice != 0 || row.template_visibility != 0
}

#[derive(Default)]
pub struct Tally {
    counts: HashMap<u128, u64>,
    dates: HashMap<(u64, String), (i32, u8)>,
}

#[derive(Default)]
struct RunTally {
    battles: Vec<(u64, [u64; 3])>,
    dates: HashMap<(u64, String), ((i32, u8), Option<i32>)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asked {
    All,
    Id(u32),
}

impl Asked {
    fn keeps(self, handle: u64) -> bool {
        match self {
            Self::All => true,
            Self::Id(id) => (handle >> 32) as u32 == id,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Battles {
    pub fought: u64,
    pub won: u64,
    pub lost: u64,
}

const ENTRY_PARTICIPATION: u32 = 0;
const ENTRY_RESULT: u32 = 1;
const OUTCOME_WON: u32 = 1;
const OUTCOME_LOST: u32 = 2;

fn facet(id: u32, entry: u32, outcome: u32) -> u128 {
    (u128::from(id) << 96) | (u128::from(entry) << 64) | (u128::from(outcome) << 32)
}

fn run_tally(query: &Query<'_>, run: u32, hour: i32, asked: Asked) -> io::Result<RunTally> {
    let recording = query.replay.recording();
    let totals = dashboard_query::cost::storage(|| replay_lanes::lanes::at(recording, run, TableId::LeaderBattleTotals, hour, None))?;
    let dated = dashboard_query::cost::storage(|| replay_lanes::lanes::at(recording, run, TableId::LeaderTraitDates, hour, None))?;
    Ok(RunTally::of(&totals.view(), &dated.view(), asked))
}

impl RunTally {
    fn of(totals: &StoreView<'_>, dated: &StoreView<'_>, asked: Asked) -> Self {
        let mut out = Self::default();
        let view = totals;
        for slot in view.rows() {
            let Some(handle) = view.key(slot).map(|key| key as u64).filter(|&handle| asked.keeps(handle)) else { continue };
            out.battles.push((handle, [0, 1, 2].map(|column| view.number(column, slot).map_or(0, |count| count as u64))));
        }
        let view = dated;
        for slot in view.rows() {
            let Some(handle) = view.key(slot).map(|key| (key >> 32) as u64).filter(|&handle| asked.keeps(handle)) else { continue };
            let Some(spelling) = view.text(3, slot).and_then(|id| view.string(id)).map(|bytes| String::from_utf8_lossy(bytes).into_owned()) else { continue };
            let (Some(first), Some(class)) = (view.number(0, slot), view.number(1, slot)) else { continue };
            let first = (first as i32, class as u8);
            let gained = view.number(2, slot).map(|hour| hour as i32);
            let held = out.dates.entry((handle, spelling)).or_insert((first, gained));
            held.0 = held.0.min(first);
            held.1 = held.1.max(gained);
        }
        out
    }
}

impl Tally {
    pub fn battles(&self, id: u32) -> Battles {
        let mut out = Battles::default();
        for entry in [ENTRY_PARTICIPATION, ENTRY_RESULT] {
            for outcome in [OUTCOME_WON, OUTCOME_LOST, u32::MAX, 0] {
                let count = self.counts.get(&facet(id, entry, outcome)).copied().unwrap_or(0);
                match (entry, outcome) {
                    (ENTRY_PARTICIPATION, _) => out.fought += count,
                    (_, OUTCOME_WON) => out.won += count,
                    (_, OUTCOME_LOST) => out.lost += count,
                    _ => {}
                }
            }
        }
        out
    }

    pub fn date(&self, leader: u64, token: &str) -> Option<(i32, u8)> {
        self.dates.get(&(leader, token.to_owned())).copied()
    }

    pub fn read(query: &Query<'_>, hour: i32, asked: Asked) -> io::Result<Self> {
        let mut runs: Vec<(u32, i32, i32)> = Vec::new();
        for stretch in query.timeline.within(query.timeline.first, hour) {
            match runs.last_mut() {
                Some(held) if held.0 == stretch.run => held.2 = stretch.last,
                _ => runs.push((stretch.run, stretch.first, stretch.last)),
            }
        }
        let mut out = Self::default();
        for (run, first, last) in runs {
            let through = run_tally(query, run, last, asked)?;
            let before = match first > query.replay.recording().run(run)?.first_hour {
                true => Some(run_tally(query, run, first - 1, asked)?),
                false => None,
            };
            out.fold(through, before.as_ref(), first);
        }
        Ok(out)
    }

    fn fold(&mut self, through: RunTally, before: Option<&RunTally>, first: i32) {
        let mut add = |battles: &[(u64, [u64; 3])], sign: i64| {
            for &(handle, [fought, won, lost]) in battles {
                let id = (handle >> 32) as u32;
                for (key, count) in [(facet(id, ENTRY_PARTICIPATION, u32::MAX), fought), (facet(id, ENTRY_RESULT, OUTCOME_WON), won), (facet(id, ENTRY_RESULT, OUTCOME_LOST), lost)] {
                    let held = self.counts.entry(key).or_default();
                    *held = held.saturating_add_signed(sign * count as i64);
                }
            }
        };
        add(&through.battles, 1);
        if let Some(before) = before {
            add(&before.battles, -1);
        }
        let mut dates: Vec<_> = through.dates.into_iter().collect();
        dates.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        for (key, ((hour, class), gained)) in dates {
            fold_date(&mut self.dates, key.clone(), hour, class);
            if let Some(gained) = gained.filter(|&gained| gained >= first) {
                fold_date(&mut self.dates, key, gained, replay_data::rows::LeaderTraitGainRow::GAINED);
            }
        }
    }
}

use dashboard_query::log;

pub fn cell_text(cell: &log::Cell) -> Option<String> {
    match cell {
        log::Cell::Text(text) => Some(text.clone()),
        log::Cell::List(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        _ => None,
    }
}

pub fn cell_number(cell: &log::Cell) -> Option<i64> {
    match cell {
        log::Cell::Number(value) => Some(*value),
        _ => None,
    }
}

pub fn fold_date(dates: &mut HashMap<(u64, String), (i32, u8)>, key: (u64, String), hour: i32, class: u8) {
    use replay_data::rows::LeaderTraitGainRow as Gain;
    match (dates.get(&key), class) {
        (_, Gain::GAINED) | (None, _) => {
            dates.insert(key, (hour, class));
        }
        _ => {}
    }
}

pub fn battles_partial_from(query: &Query<'_>, from: i32, to: i32) -> io::Result<Option<i32>> {
    let recording = query.replay.recording();
    for stretch in query.timeline.within(from, to) {
        let mut at = stretch.first;
        for (first, last) in replay_lanes::lanes::reads(recording, stretch.run, TableId::LeaderBattles, stretch.first, stretch.last.min(to))? {
            if first > at {
                return Ok(Some(at));
            }
            at = at.max(last + 1);
        }
        if at <= stretch.last.min(to) {
            return Ok(Some(at));
        }
    }
    Ok(None)
}

pub struct LeadersContext {
    pub content: Content,
    replay: Arc<Replay>,
    index: Arc<dashboard_query::Index>,
    lifetimes: HashMap<u32, Arc<Vec<(u128, i64, i64)>>>,
    verdicts: Arc<std::sync::Mutex<Verdicts>>,
}

impl LeadersContext {
    pub fn load(source: &Source, load: &LoadContext) -> io::Result<Self> {
        let mut lifetimes = HashMap::new();
        if let Some(timelines) = source.replay_ref().timelines() {
            for stretch in timelines.list.iter().flat_map(|timeline| dashboard_base::queries::query(source, timeline.id).map(|query| query.timeline.stretches.clone()).unwrap_or_default()) {
                if let std::collections::hash_map::Entry::Vacant(entry) = lifetimes.entry(stretch.run) {
                    entry.insert(dashboard_base::queries::lifetimes(source, stretch.run, TableId::UnitLeaders)?);
                }
            }
        }
        let content = Content::load(source, load);
        let verdicts = source.state::<Verdicts>();
        {
            let mut held = verdicts.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let catalogue = content.catalogue.as_ref().ok();
            if held.catalogue.as_ref().map(Arc::as_ptr) != catalogue.map(Arc::as_ptr) {
                *held = Verdicts { catalogue: catalogue.cloned(), at: HashMap::new() };
            }
        }
        Ok(Self { content, replay: source.replay(), index: dashboard_base::queries::index(source), lifetimes, verdicts })
    }

    fn verdicts_at(&self, run: u32, hour: i32) -> HashMap<u64, Result<bool, String>> {
        self.verdicts.lock().unwrap_or_else(std::sync::PoisonError::into_inner).at.get(&(run, hour)).cloned().unwrap_or_default()
    }

    fn keep(&self, run: u32, hour: i32, made: HashMap<u64, Result<bool, String>>) {
        if made.is_empty() {
            return;
        }
        let mut held = self.verdicts.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let held_catalogue = held.catalogue.as_ref().map(Arc::as_ptr);
        if held_catalogue == self.content.catalogue.as_ref().ok().map(Arc::as_ptr) {
            held.at.entry((run, hour)).or_default().extend(made);
        }
    }

    pub fn query(&self, branch: u64) -> io::Result<Query<'_>> {
        self.index.query(&self.replay, branch)
    }

    pub fn first_seen(&self, run: u32, key: u128, hour: i32) -> Option<i32> {
        let lives = self.lifetimes.get(&run)?;
        let start = lives.partition_point(|life| life.0 < key);
        lives[start..].iter().take_while(|life| life.0 == key).find(|life| life.1 <= i64::from(hour) && i64::from(hour) <= life.2).and_then(|life| i32::try_from(life.1).ok())
    }

    pub fn tally(&self, query: &Query<'_>, hour: i32, asked: Asked) -> io::Result<Tally> {
        Tally::read(query, hour, asked)
    }

    #[cfg(feature = "test-support")]
    pub fn keep_test_verdict(&self, run: u32, hour: i32) {
        self.keep(run, hour, [(1, Ok(true))].into());
    }

    #[cfg(feature = "test-support")]
    pub fn has_cached_verdicts(&self, run: u32, hour: i32) -> bool {
        !self.verdicts_at(run, hour).is_empty()
    }
}

pub struct UnitLeaders;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnitLeadersParams {
    pub branch_id: String,
    pub selection: LeaderSelection,
    #[serde(default)]
    pub leaders: LeaderKinds,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaderRow {
    pub reference: String,
    pub name: Option<String>,
    pub role: LeaderRole,
    pub skill: Option<i32>,
    pub components: [i32; 4],
    #[doc = " The traits held, by place in the reply's `traits`; none where they were not recorded."]
    pub traits: Option<Vec<u32>>,
    #[doc = " Each held trait's date, aligned with `traits`: [hour, class] (0 held before, 1 gained, 2"]
    #[doc = " first observed); none where the logs hold none."]
    pub trait_dates: Vec<Option<(i32, u8)>>,
    pub first_seen_hour: Option<i32>,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number"))]
    pub fought: u64,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number"))]
    pub won: u64,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number"))]
    pub lost: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct UnitLeadersReply {
    pub leaders: Vec<LeaderRow>,
    pub traits: Vec<LeaderTrait>,
    pub withheld: Vec<LeaderWithheld>,
    #[doc = " Why the leaders are not shown at this hour, when they are not."]
    pub missing: Option<String>,
    #[doc = " The first hour up to this one the battle log was not recorded: counts are then lower bounds."]
    pub battles_partial_from: Option<i32>,
    pub results_missing: String,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(UnitLeadersReply, LeaderPortraitsReply);

const BASE_TABLES: [TableId; 4] = [TableId::UnitLeaders, TableId::UnitLeaderTraits, TableId::Countries, TableId::CountryLinks];

#[derive(Default)]
pub struct TraitList {
    pub traits: Vec<LeaderTrait>,
    at: HashMap<String, u32>,
}

impl TraitList {
    pub fn index(&mut self, token: &str, content: &Content) -> u32 {
        if let Some(&at) = self.at.get(token) {
            return at;
        }
        let at = self.traits.len() as u32;
        self.traits.push(content.trait_info(token));
        self.at.insert(token.to_owned(), at);
        at
    }
}

pub fn held_traits(rows: &[UnitLeaderTraitRow]) -> BTreeMap<u64, Vec<String>> {
    let mut out: BTreeMap<u64, Vec<(u32, String)>> = BTreeMap::new();
    for row in rows {
        out.entry(row.leader_reference).or_default().push((row.ordinal, String::from_utf8_lossy(&row.token).into_owned()));
    }
    out.into_iter()
        .map(|(leader, mut held)| {
            held.sort_by_key(|(ordinal, _)| *ordinal);
            (leader, held.into_iter().map(|(_, token)| token).collect())
        })
        .collect()
}

impl AtHour for UnitLeaders {
    const KEY: &'static str = "unit-leaders";
    const TITLE: &'static str = "Generals leaderboard";
    const CONTENT_DEPENDENT: bool = true;
    type Params = UnitLeadersParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, selection, leaders, "selection.countries[]"];
    type Reply = UnitLeadersReply;
    type Context = LeadersContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![UnitLeadersParams::decl(), LeaderSelection::decl(), LeaderKinds::decl(), LeaderTrait::decl(), LeaderRole::decl(), LeaderWithheld::decl(), LeaderRow::decl(), UnitLeadersReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        parse_branch(&params.branch_id)?;
        if let LeaderSelection::Countries { countries } = &params.selection {
            countries.iter().try_for_each(dashboard_base::products::validate_country)?;
        }
        Ok(())
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        let mut tables: Vec<TableId> = BASE_TABLES.to_vec();
        tables.extend(RECORDED_TABLES.iter().copied().filter(|table| !BASE_TABLES.contains(table)));
        tables.extend([TableId::LeaderBattleTotals, TableId::LeaderTraitDates]);
        tables
    }

    fn reads_in(params: &Self::Params, _owner: Option<dashboard_base::products::at_hour::Owner>, step: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> Vec<Read> {
        let kept = || -> Vec<&UnitLeaderRow> {
            let Ok(leaders) = held.complete::<tables::UnitLeaders>() else { return Vec::new() };
            let tags = Tags::read(held).ok();
            leaders
                .iter()
                .filter(|row| params.leaders.includes(row.kind) && params.selection.keeps(tags.as_ref().and_then(|tags| owned(tags, row.owner)).as_ref().map(|(slot, tag)| (*slot, tag.as_str()))))
                .collect()
        };
        match step {
            0 => BASE_TABLES.into_iter().map(Read::all).collect(),
            1 => {
                let kept = kept();
                if kept.is_empty() {
                    return Vec::new();
                }
                vec![Read::these(TableId::Characters, kept.iter().map(|row| u128::from(row.character_id))), Read::all(TableId::LexicalTokens)]
            }
            2 => {
                let kept = kept();
                if !kept.iter().any(|row| needs_world(row)) {
                    return Vec::new();
                }
                let decided = at.context.verdicts_at(at.run, at.hour);
                let mut world = None;
                let open = world_of(held, &at.context.content, &mut world).is_ok_and(|world| kept.iter().any(|row| !decided.contains_key(&handle(row)) && conditioned(row, world)));
                if open { RECORDED_TABLES.iter().copied().filter(|table| !BASE_TABLES.contains(table)).map(Read::all).collect() } else { Vec::new() }
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        LeadersContext::load(source, load)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let context = at.context;
        let content = &context.content;
        let mut warnings = content.missing();
        let leaders = match frame.complete::<tables::UnitLeaders>().map_err(|error| error.to_string()) {
            Ok(leaders) => leaders,
            Err(why) => {
                return Ok(UnitLeadersReply { leaders: Vec::new(), traits: Vec::new(), withheld: Vec::new(), missing: Some(why), battles_partial_from: None, results_missing: RESULTS_MISSING.into(), warnings });
            }
        };
        let branch = parse_branch(&params.branch_id).map_err(dashboard_core::invalid)?;
        let query = context.query(branch)?;
        let run = query.timeline.at(at.hour).map(|stretch| stretch.run);
        let tags = Tags::read(frame).map_err(|error| error.to_string());
        if let Err(why) = &tags {
            warnings.push(format!("Leaders' owners are not named at this hour: {why}"));
        }
        let held = frame.complete::<tables::UnitLeaderTraits>().map_err(|error| error.to_string()).map(held_traits);
        if let Err(why) = &held {
            warnings.push(format!("Leaders' traits are not recorded at this hour: {why}"));
        }
        let characters = frame.complete::<tables::Characters>().ok();
        let lexical = frame.complete::<tables::LexicalTokens>().ok();
        let mut shown: Vec<(&UnitLeaderRow, Option<String>, Option<String>, Option<Vec<String>>)> = Vec::new();
        let mut withheld = Vec::new();
        let mut world = None;
        let decided = run.map(|run| context.verdicts_at(run, at.hour)).unwrap_or_default();
        let evaluable = core_gap(frame).is_none();
        let mut made = HashMap::new();
        for row in leaders {
            dashboard_text::metadata::check_cancel(at.cancelled)?;
            if !params.leaders.includes(row.kind) {
                continue;
            }
            if row.owner == 0 {
                continue;
            }
            let owner = tags.as_ref().ok().and_then(|tags| tags.tag(row.owner));
            let character = characters.and_then(|rows| rows.binary_search_by_key(&row.character_id, |character| character.id).ok().map(|at| &rows[at]));
            let name = content.person_name(character, lexical);
            if owner.is_none() && tags.is_ok() {
                if matches!(params.selection, LeaderSelection::All) {
                    withheld.push(LeaderWithheld { reference: handle(row).to_string(), owner: None, kind: row.kind, name, reason: "Leader owner has no resolved recorded country tag".into() });
                }
                continue;
            }
            if !params.selection.keeps(tags.as_ref().ok().and_then(|tags| owned(tags, row.owner)).as_ref().map(|(slot, tag)| (*slot, tag.as_str()))) {
                continue;
            }
            let verdict = match decided.get(&handle(row)) {
                Some(verdict) => verdict.clone(),
                None => {
                    let verdict = visible(row, frame, content, &mut world);
                    if evaluable && world.as_ref().and_then(|world| world.as_ref().ok()).is_some_and(|world| conditioned(row, world)) {
                        made.insert(handle(row), verdict.clone());
                    }
                    verdict
                }
            };
            match verdict {
                Ok(true) => {}
                Ok(false) => continue,
                Err(reason) => {
                    withheld.push(LeaderWithheld { reference: handle(row).to_string(), owner, kind: row.kind, name, reason });
                    continue;
                }
            }
            let tokens = held.as_ref().ok().map(|held| held.get(&handle(row)).cloned().unwrap_or_default());
            shown.push((row, owner, name, tokens));
        }
        if let Some(run) = run {
            context.keep(run, at.hour, made);
        }
        let tally = context.tally(&query, at.hour, Asked::All)?;
        let tallied: Vec<(Battles, Vec<Option<(i32, u8)>>)> =
            shown.iter().map(|(row, _, _, tokens)| (tally.battles(row.id), tokens.iter().flatten().map(|token| tally.date(handle(row), token)).collect())).collect();
        let mut traits = TraitList::default();
        let mut out = Vec::with_capacity(shown.len());
        for ((row, owner, name, tokens), (battles, dates)) in shown.into_iter().zip(tallied) {
            let reference = handle(row);
            let skill = (row.skill_valid != 0).then_some(row.skill_level);
            out.push(LeaderRow {
                reference: reference.to_string(),
                name,
                role: content.role(owner, row.kind, row.category, skill),
                skill,
                components: row.components,
                traits: tokens.map(|tokens| tokens.iter().map(|token| traits.index(token, content)).collect()),
                trait_dates: dates,
                first_seen_hour: run.and_then(|run| context.first_seen(run, u128::from(reference), at.hour)),
                fought: battles.fought,
                won: battles.won,
                lost: battles.lost,
            });
        }
        let battles_partial_from = battles_partial_from(&query, query.timeline.first, at.hour)?;
        Ok(UnitLeadersReply { leaders: out, traits: traits.traits, withheld, missing: None, battles_partial_from, results_missing: RESULTS_MISSING.into(), warnings })
    }
}

pub struct LeaderPortraits;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaderPortraitsParams {
    pub branch_id: String,
    pub hour: i32,
    pub references: Vec<String>,
    pub edge: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaderPortrait {
    pub reference: String,
    pub image: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaderPortraitsReply {
    pub portraits: Vec<LeaderPortrait>,
    pub warnings: Vec<String>,
}

fn portrait(frame: &DecodedFrame, reference: u64, politics: &PoliticsContent, edge: u32) -> Result<String, String> {
    let leaders = frame.complete::<tables::UnitLeaders>().map_err(|error| error.to_string())?;
    let row = leaders.binary_search_by_key(&reference, handle).ok().map(|at| &leaders[at]).ok_or("Leader not recorded at this hour")?;
    let holders = frame.complete::<tables::CharacterPortraits>().map_err(|error| error.to_string())?;
    let holder = holders.binary_search_by_key(&row.character_id, |holder| holder.character).ok().map(|at| &holders[at]).ok_or("Character portrait holder not recorded")?;
    let category = if row.kind == KIND_NAVY { PORTRAIT_NAVY } else { PORTRAIT_ARMY };
    let entry = replay_data::rows::portrait_entry(&holder.portraits, category, replay_data::rows::PORTRAIT_LARGE)
        .filter(|entry| entry.is_set != 0)
        .ok_or("The game generates this portrait from unrecorded inputs")?;
    politics.recorded_portrait_sized(entry.kind, &entry.identity, edge)
}

impl Product for LeaderPortraits {
    const KEY: &'static str = "unit-leader-portraits";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = LeaderPortraitsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, hour: Read, references, edge];
    type Reply = LeaderPortraitsReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![LeaderPortraitsParams::decl(), LeaderPortrait::decl(), LeaderPortraitsReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        parse_branch(&params.branch_id)?;
        if params.references.is_empty() || params.edge == 0 || params.references.iter().any(|reference| reference.parse::<u64>().is_err()) {
            return Err("Invalid leader portrait request");
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, parse_branch(&params.branch_id).map_err(dashboard_core::invalid)?)?;
        let missing = |why: &str| params.references.iter().map(|reference| LeaderPortrait { reference: reference.clone(), image: None, missing: Some(why.to_owned()) }).collect();
        let Some(stretch) = query.timeline.at(params.hour) else {
            return Ok(LeaderPortraitsReply { portraits: missing("Portrait hour not recorded"), warnings: Vec::new() });
        };
        let politics = context.content().and_then(|options| recorded_politics_content(source, options));
        let politics = match politics {
            Ok(politics) => politics,
            Err(error) => return Ok(LeaderPortraitsReply { portraits: missing(&error.to_string()), warnings: Vec::new() }),
        };
        let keys: Vec<u128> = params.references.iter().filter_map(|reference| reference.parse::<u64>().ok()).map(u128::from).collect();
        let first = replay_lanes::at::frame(source.replay_ref().recording(), stretch.run, params.hour, &[replay_lanes::at::Read { table: TableId::UnitLeaders, columns: None, keys: replay_lanes::at::Keys::These(&{
            let mut keys = keys;
            keys.sort_unstable();
            keys.dedup();
            keys
        }) }])?;
        let characters: Vec<u128> = first.complete::<tables::UnitLeaders>().map(|rows| rows.iter().map(|row| u128::from(row.character_id)).collect()).unwrap_or_default();
        let mut characters = characters;
        characters.sort_unstable();
        characters.dedup();
        let second = replay_lanes::at::frame(source.replay_ref().recording(), stretch.run, params.hour, &[replay_lanes::at::Read { table: TableId::CharacterPortraits, columns: None, keys: replay_lanes::at::Keys::These(&characters) }])?;
        let mut frame = first;
        frame.tables[TableId::CharacterPortraits.index()] = second.tables[TableId::CharacterPortraits.index()].clone();
        let mut portraits = Vec::with_capacity(params.references.len());
        for reference in &params.references {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let drawn = portrait(&frame, reference.parse().unwrap_or_default(), &politics, params.edge);
            let (image, missing) = match drawn {
                Ok(image) => (Some(image), None),
                Err(why) => (None, Some(why)),
            };
            portraits.push(LeaderPortrait { reference: reference.clone(), image, missing });
        }
        let reply = LeaderPortraitsReply { portraits, warnings: Vec::new() };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
