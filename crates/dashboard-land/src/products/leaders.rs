
pub mod career;
mod membership;
mod ranking;
mod visibility;
#[cfg(test)]
mod tests;

use dashboard_base::products::{LoadContext, Product, artwork::{ArtResolution, sprites::SpriteFrame}, at_hour::{At, AtHour, Read, ReadsAt}, politics_content::{PoliticsContent, recorded_politics_content}};
use dashboard_intelligence::products::intelligence::Tags;
use dashboard_base::{Source, catalogue::{self, Catalogue, View}};
use dashboard_query::{Query, series::ranked::GroupSelection};
use membership::Roster;
use dashboard_text::triggers::{Evaluator, LeaderVisibility, RECORDED_TABLES, World, core_gap};
use content_reader_gui::gui::native::{Guis, window_icon_sprite};
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum LeaderKinds {
    Land,
    Navy,
}

impl LeaderKinds {
    pub fn includes(self, kind: u8) -> bool {
        match self {
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
    Group { group: GroupSelection },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SelectionAsked {
    kind: SelectionKind,
    countries: Option<Vec<dashboard_base::products::Country>>,
    group: Option<GroupSelection>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum SelectionKind {
    All,
    Countries,
    Group,
}

impl TryFrom<SelectionAsked> for LeaderSelection {
    type Error = &'static str;

    fn try_from(asked: SelectionAsked) -> Result<Self, Self::Error> {
        match (asked.kind, asked.countries) {
            (SelectionKind::All, _) => Ok(Self::All),
            (SelectionKind::Countries, Some(countries)) => Ok(Self::Countries { countries }),
            (SelectionKind::Countries, None) => Err("A leader selection of countries names them"),
            (SelectionKind::Group, _) => asked.group.map(|group| Self::Group { group }).ok_or("A leader group selection names its group"),
        }
    }
}

impl LeaderSelection {
    fn keeps(&self, slot: Option<u32>, tag: Option<&str>, roster: Option<&Roster>, hour: i32) -> bool {
        match self {
            Self::All => true,
            Self::Countries { countries } => slot.zip(tag).is_some_and(|(slot, tag)| countries.iter().any(|country| dashboard_world::products::holder_tags::matches(country, (slot, 0), tag))),
            Self::Group { .. } => slot.zip(roster).is_some_and(|(slot, roster)| roster.includes(slot, hour)),
        }
    }
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
    icons: std::sync::OnceLock<(LeaderSkillIcons, Vec<String>)>,
}

type HeldLeaders = content_reader_layers::content::diagnostics::Once<Result<Arc<Leaders>, String>>;

impl Content {
    pub fn load(source: &Source, load: &LoadContext) -> Self {
        let Some(options) = load.content.as_ref() else {
            return Self { catalogue: Err(UNCONFIGURED.into()), politics: Err(UNCONFIGURED.into()), leaders: Err(UNCONFIGURED.into()), icons: std::sync::OnceLock::new() };
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
        Self { catalogue, politics, leaders, icons: std::sync::OnceLock::new() }
    }

    pub fn skill_icons(&self) -> &(LeaderSkillIcons, Vec<String>) {
        self.icons.get_or_init(|| {
            let mut warnings = Vec::new();
            (skill_icons(self.catalogue.as_deref(), &mut warnings), warnings)
        })
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
    dashboard_base::products::artwork::resolve_sprite_type(catalogue, sprite, 0, 32)
        .into_result()?
        .ok_or_else(|| "Trait icon unavailable".to_owned())
}

fn world_of<'a>(frame: &'a DecodedFrame, content: &'a Content) -> Result<World<'a>, String> {
    let catalogue = content.catalogue.as_ref().map_err(|why| format!("Leader visibility needs the recorded content: {why}"))?;
    catalogue.scripted_content()?.recorded_world(frame)
}

fn visible<'a>(row: &UnitLeaderRow, frame: &'a DecodedFrame, content: &'a Content, held: &mut Option<(Result<World<'a>, String>, catalogue::OwnRefusals)>) -> Result<bool, String> {
    if row.kind == KIND_ARMY && (row.category as u32) < 2 && row.captured != 0 {
        return Ok(false);
    }
    if !needs_world(row) {
        return Ok(true);
    }
    if let Some((_, notes)) = held.as_ref() { catalogue::credit(notes); }
    let (world, _) = held.get_or_insert_with(|| catalogue::capture(|| world_of(frame, content)));
    let world = world.as_ref().map_err(Clone::clone)?;
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

#[derive(Clone, Default)]
pub struct Tally {
    counts: BTreeMap<u128, u64>,
    dates: BTreeMap<(u64, String), (i32, u8)>,
}

#[derive(Default)]
struct RunTally {
    battles: Vec<(u64, [u64; 3])>,
    dates: HashMap<(u64, String), ((i32, u8), Option<i32>)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Asked<'a> {
    All,
    Ids(&'a [u32]),
}

impl Asked<'_> {
    fn keeps(self, handle: u64) -> bool {
        match self {
            Self::All => true,
            Self::Ids(ids) => ids.binary_search(&((handle >> 32) as u32)).is_ok(),
        }
    }

    fn slices<T>(self, rows: &[T], owner: impl Fn(&T) -> u32, mut take: impl FnMut(&[T])) {
        match self {
            Self::All => take(rows),
            Self::Ids(ids) => for &id in ids {
                let first = rows.partition_point(|row| owner(row) < id);
                let end = first + rows[first..].partition_point(|row| owner(row) == id);
                take(&rows[first..end]);
            },
        }
    }
}

struct TallyHistory {
    branch: u64,
    stretch: i32,
    first: i32,
    past: Tally,
    before: Vec<(u64, [u64; 3])>,
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

fn run_tally(query: &Query<'_>, run: u32, hour: i32, asked: Asked<'_>) -> io::Result<RunTally> {
    #[cfg(test)]
    tests::TALLY_SNAPSHOTS.set(tests::TALLY_SNAPSHOTS.get() + 1);
    let recording = query.replay.recording();
    let totals = dashboard_query::cost::storage(|| replay_lanes::lanes::at(recording, run, TableId::LeaderBattleTotals, hour, None))?;
    let dated = dashboard_query::cost::storage(|| replay_lanes::lanes::at(recording, run, TableId::LeaderTraitDates, hour, None))?;
    Ok(RunTally::of(&totals.view(), &dated.view(), asked))
}

impl RunTally {
    fn staged(frame: &DecodedFrame, asked: Asked<'_>) -> Option<Self> {
        let totals = frame.complete::<tables::LeaderBattleTotals>().ok()?;
        let dates = frame.complete::<tables::LeaderTraitDates>().ok()?;
        let mut out = Self::default();
        asked.slices(totals, |row| (row.leader >> 32) as u32, |rows| {
            out.battles.extend(rows.iter().map(|row| (row.leader, [row.fought, row.won, row.lost].map(u64::from))));
        });
        asked.slices(dates, |row| (row.leader >> 32) as u32, |rows| {
            for row in rows {
                out.date(row.leader, String::from_utf8_lossy(&row.spelling).into_owned(), (row.first_hour, row.first_class), row.gained_hour);
            }
        });
        Some(out)
    }

    fn date(&mut self, handle: u64, spelling: String, first: (i32, u8), gained: Option<i32>) {
        let held = self.dates.entry((handle, spelling)).or_insert((first, gained));
        held.0 = held.0.min(first);
        held.1 = held.1.max(gained);
    }

    fn of(totals: &StoreView<'_>, dated: &StoreView<'_>, asked: Asked<'_>) -> Self {
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
            out.date(handle, spelling, first, gained);
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

    fn select(&self, asked: Asked<'_>) -> Self {
        let Asked::Ids(ids) = asked else { return self.clone() };
        let mut out = Self::default();
        for &id in ids {
            let first = u128::from(id) << 96;
            out.counts.extend(self.counts.range(first..).take_while(|(key, _)| (**key >> 96) as u32 == id));
            let first = (u64::from(id) << 32, String::new());
            out.dates.extend(self.dates.range(first..).take_while(|((leader, _), _)| (leader >> 32) as u32 == id)
                .map(|(key, &value)| (key.clone(), value)));
        }
        out
    }

    fn fold(&mut self, through: RunTally, before: &[(u64, [u64; 3])], first: i32) {
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
        add(before, -1);
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

impl TallyHistory {
    fn load(query: &Query<'_>, stretch: &dashboard_query::timeline::Stretch) -> io::Result<Self> {
        let mut spans: Vec<(u32, i32, i32)> = Vec::new();
        for part in query.timeline.within(query.timeline.first, stretch.first) {
            match spans.last_mut() {
                Some(last) if last.0 == part.run => last.2 = part.last,
                _ => spans.push((part.run, part.first, part.last)),
            }
        }
        let (run, first, _) = spans.pop().expect("the current stretch is included");
        let before_run = |run, first| -> io::Result<Vec<(u64, [u64; 3])>> {
            if first > query.replay.recording().run(run)?.first_hour {
                Ok(run_tally(query, run, first - 1, Asked::All)?.battles)
            } else { Ok(Vec::new()) }
        };
        let mut past = Tally::default();
        for (run, first, last) in spans {
            past.fold(run_tally(query, run, last, Asked::All)?, &before_run(run, first)?, first);
        }
        let mut before = before_run(run, first)?;
        before.sort_unstable_by_key(|&(handle, _)| handle);
        Ok(Self { branch: query.timeline.id, stretch: stretch.first, first, past, before })
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

fn fold_date(dates: &mut BTreeMap<(u64, String), (i32, u8)>, key: (u64, String), hour: i32, class: u8) {
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
    roster: Option<Arc<Roster>>,
    visibility: std::sync::Mutex<visibility::Visibility>,
    ranking: std::sync::Mutex<ranking::Ranking>,
    tally_history: std::sync::Mutex<Option<TallyHistory>>,
    battle_coverage: Option<(u64, Option<i32>)>,
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
        Ok(Self { content, replay: source.replay(), index: dashboard_base::queries::index(source), lifetimes, verdicts, roster: None, visibility: Default::default(), ranking: Default::default(), tally_history: Default::default(), battle_coverage: None })
    }

    fn for_selection(source: &Source, load: &LoadContext, branch: &str, selection: Option<&LeaderSelection>, cancelled: &AtomicBool) -> io::Result<Self> {
        let mut context = Self::load(source, load)?;
        let branch = parse_branch(branch).map_err(dashboard_core::invalid)?;
        let query = context.query(branch)?;
        let partial = battles_partial_from(&query, query.timeline.first, query.timeline.last)?;
        let roster = if let Some(LeaderSelection::Group { group }) = selection {
            Some(Roster::load(source, &query, load, group, cancelled)?)
        } else {
            None
        };
        context.battle_coverage = Some((branch, partial));
        context.roster = roster;
        Ok(context)
    }

    fn verdicts_at(&self, run: u32, hour: i32) -> HashMap<u64, Result<bool, String>> {
        let mut decisions = self.verdicts.lock().unwrap_or_else(std::sync::PoisonError::into_inner).at.get(&(run, hour)).cloned().unwrap_or_default();
        if let Some(held) = self.visibility.lock().unwrap_or_else(std::sync::PoisonError::into_inner).held.as_ref().filter(|held| held.covers(run, hour)) {
            decisions.extend(held.decisions.iter().map(|(&key, value)| (key, value.clone())));
        }
        decisions
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

    fn tally(&self, query: &Query<'_>, frame: &DecodedFrame, asked: Asked<'_>) -> io::Result<Tally> {
        let stretches = &query.timeline.stretches;
        let after = stretches.partition_point(|stretch| stretch.first <= frame.hour);
        let Some(stretch) = stretches[..after].last() else { return Ok(Tally::default()) };
        let mut held = self.tally_history.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if held.as_ref().is_none_or(|history| history.branch != query.timeline.id || history.stretch != stretch.first) {
            *held = Some(TallyHistory::load(query, stretch)?);
        }
        let history = held.as_ref().expect("loaded the current stretch");
        let hour = frame.hour.min(stretch.last);
        let current = match (hour == frame.hour).then(|| RunTally::staged(frame, asked)).flatten() {
            Some(current) => current,
            None => run_tally(query, stretch.run, hour, asked)?,
        };
        let mut tally = history.past.select(asked);
        let mut before = Vec::new();
        asked.slices(&history.before, |row| (row.0 >> 32) as u32, |rows| before.extend_from_slice(rows));
        tally.fold(current, &before, history.first);
        Ok(tally)
    }

    fn battles_partial(&self, query: &Query<'_>, hour: i32) -> io::Result<Option<i32>> {
        match self.battle_coverage {
            Some((branch, first)) if branch == query.timeline.id => Ok(first.filter(|&first| first <= hour)),
            _ => battles_partial_from(query, query.timeline.first, hour),
        }
    }

    fn through(&self, hour: i32) -> i32 {
        let membership = self.roster.as_ref().map_or(i32::MAX, |roster| roster.through(hour));
        let damage = self.battle_coverage.and_then(|(_, first)| first).filter(|&first| first > hour);
        membership.min(damage.map_or(i32::MAX, |first| first - 1))
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
    #[doc = " The leader window's own skill icons, from the installed interface."]
    pub skill_icons: LeaderSkillIcons,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(UnitLeadersReply, LeaderPortraitsReply);

const SKILL_WINDOW: &str = "unitleader_trait_tree_window";
const LAND_ENTRY_WINDOW: &str = "armyleaderentry";
const NAVY_ENTRY_WINDOW: &str = "navyleaderentry";
const LAND_SKILL_ICONS: [(&str, &str); 5] = [
    (SKILL_WINDOW, "skill_bg"),
    (LAND_ENTRY_WINDOW, "leader_stat_attack_bg"),
    (LAND_ENTRY_WINDOW, "leader_stat_defense_bg"),
    (LAND_ENTRY_WINDOW, "leader_stat_planning_bg"),
    (LAND_ENTRY_WINDOW, "leader_stat_logistics_bg"),
];
const NAVY_SKILL_ICONS: [(&str, &str); 5] = [
    (SKILL_WINDOW, "skill_bg"),
    (NAVY_ENTRY_WINDOW, "leader_stat_attack_bg"),
    (NAVY_ENTRY_WINDOW, "leader_stat_defense_bg"),
    (NAVY_ENTRY_WINDOW, "leader_stat_maneuvering_bg"),
    (NAVY_ENTRY_WINDOW, "leader_stat_coordination_bg"),
];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaderSkillIcons {
    #[doc = " Skill, attack, defense, planning and logistics from the installed leader interface; none where it draws none."]
    pub land: Vec<Option<SpriteFrame>>,
    #[doc = " Skill, attack, defense, maneuvering and coordination from the installed leader interface."]
    pub navy: Vec<Option<SpriteFrame>>,
}

pub(crate) fn window_icons(guis: Result<&Guis, String>, icons: [(&str, &str); 5], warnings: &mut Vec<String>) -> Vec<Option<SpriteFrame>> {
    let guis = match guis {
        Ok(guis) => guis,
        Err(why) => {
            warnings.push(format!("Leader skill icons are unavailable: {why}"));
            return vec![None; icons.len()];
        }
    };
    icons
        .iter()
        .map(|&(window, icon)| match window_icon_sprite(guis, window, icon, warnings) {
            Ok(key) => Some(SpriteFrame { key, frame: 0, recorded: None, text: None }),
            Err(why) => {
                warnings.push(format!("The installed interface draws no {icon} icon in {window}: {why}"));
                None
            }
        })
        .collect()
}

pub(crate) fn skill_icons(catalogue: Result<&Catalogue, &String>, warnings: &mut Vec<String>) -> LeaderSkillIcons {
    let guis = || catalogue.map_err(Clone::clone).and_then(|catalogue| catalogue.guis().map_err(|error| error.to_string()));
    LeaderSkillIcons { land: window_icons(guis(), LAND_SKILL_ICONS, warnings), navy: window_icons(guis(), NAVY_SKILL_ICONS, warnings) }
}

fn components_sum(components: [i32; 4]) -> i64 {
    components.iter().map(|&v| i64::from(v)).sum()
}

pub(crate) fn leader_rank_cmp(
    a_skill: Option<i32>,
    a_won: u64,
    a_fought: u64,
    a_components: [i32; 4],
    a_name: Option<&str>,
    a_handle: u64,
    b_skill: Option<i32>,
    b_won: u64,
    b_fought: u64,
    b_components: [i32; 4],
    b_name: Option<&str>,
    b_handle: u64,
) -> std::cmp::Ordering {
    b_skill.is_some().cmp(&a_skill.is_some())
        .then_with(|| b_skill.cmp(&a_skill))
        .then_with(|| b_won.cmp(&a_won))
        .then_with(|| b_fought.cmp(&a_fought))
        .then_with(|| components_sum(b_components).cmp(&components_sum(a_components)))
        .then_with(|| b_name.is_some().cmp(&a_name.is_some()))
        .then_with(|| a_name.cmp(&b_name))
        .then_with(|| a_handle.cmp(&b_handle))
}

pub(crate) struct EvaluatedLeader<'a> {
    pub row: &'a UnitLeaderRow,
    pub handle: u64,
    pub owner: Option<String>,
    pub name: Option<String>,
    pub skill: Option<i32>,
    pub battles: Battles,
}

pub(crate) enum TopLeader<'a> {
    Winner(&'a EvaluatedLeader<'a>),
    None,
    Missing(String),
}

pub(crate) struct EvaluatedLeaders<'a> {
    pub run: Option<u32>,
    pub leaders: Vec<EvaluatedLeader<'a>>,
    pub withheld: Vec<LeaderWithheld>,
    pub missing: Option<String>,
    pub tally: Option<Tally>,
    pub battles_partial_from: Option<i32>,
    pub warnings: Vec<String>,
}

impl EvaluatedLeaders<'_> {
    pub(crate) fn top(&self) -> TopLeader<'_> {
        if let Some(reason) = &self.missing {
            return TopLeader::Missing(reason.clone());
        }
        if let Some(reason) = ranking::top_missing(self.withheld.len(), self.withheld.first(), self.leaders.first().and_then(|l| l.skill).is_some(), self.leaders.last().is_some_and(|l| l.skill.is_none())) {
            return TopLeader::Missing(reason);
        }
        if self.leaders.is_empty() { return TopLeader::None; }
        TopLeader::Winner(&self.leaders[0])
    }
}

const BASE_TABLES: [TableId; 3] = [TableId::UnitLeaders, TableId::Countries, TableId::CountryLinks];

#[derive(Default)]
pub struct TraitList {
    pub traits: Vec<LeaderTrait>,
    at: HashMap<String, u32>,
}

impl TraitList {
    pub fn index(&mut self, token: &str, content: &Content) -> u32 {
        self.index_with(token, || content.trait_info(token))
    }

    fn index_with(&mut self, token: &str, build: impl FnOnce() -> LeaderTrait) -> u32 {
        if let Some(&at) = self.at.get(token) {
            return at;
        }
        let at = self.traits.len() as u32;
        self.traits.push(build());
        self.at.insert(token.to_owned(), at);
        at
    }
}

pub fn held_traits_for(rows: &[UnitLeaderTraitRow], leader: u64) -> Vec<String> {
    let first = rows.partition_point(|row| row.leader_reference < leader);
    let mut held: Vec<(u32, &[u8])> = rows[first..]
        .iter()
        .take_while(|row| row.leader_reference == leader)
        .map(|row| (row.ordinal, row.token.as_slice()))
        .collect();
    held.sort_by_key(|(ordinal, _)| *ordinal);
    held.into_iter().map(|(_, token)| String::from_utf8_lossy(token).into_owned()).collect()
}

impl UnitLeaders {
    fn board(frame: &DecodedFrame, evaluated: EvaluatedLeaders<'_>, at: &At<'_, LeadersContext>, mut notes: Option<&mut ranking::Notes>) -> io::Result<UnitLeadersReply> {
        let context = at.context;
        let content = &context.content;
        let mut warnings = evaluated.warnings;
        if let Some(why) = evaluated.missing {
            return Ok(UnitLeadersReply {
                leaders: Vec::new(),
                traits: Vec::new(),
                withheld: Vec::new(),
                missing: Some(why),
                battles_partial_from: None,
                results_missing: RESULTS_MISSING.into(),
                skill_icons: content.skill_icons().0.clone(),
                warnings,
            });
        }
        let held = frame.complete::<tables::UnitLeaderTraits>().map_err(|error| error.to_string());
        if let Err(why) = &held {
            warnings.push(format!("Leaders' traits are not recorded at this hour: {why}"));
        }
        let mut traits = TraitList::default();
        let mut out = Vec::with_capacity(evaluated.leaders.len());
        for leader in evaluated.leaders {
            let tokens = held.as_ref().ok().map(|held| held_traits_for(held, leader.handle));
            let dates = tokens
                .iter()
                .flatten()
                .map(|token| evaluated.tally.as_ref().and_then(|tally| tally.date(leader.handle, token)))
                .collect();
            let role = || content.role(leader.owner, leader.row.kind, leader.row.category, leader.skill);
            let role = match &mut notes { Some(notes) => notes.role(leader.handle, role), None => role() };
            out.push(LeaderRow {
                reference: leader.handle.to_string(),
                name: leader.name,
                role,
                skill: leader.skill,
                components: leader.row.components,
                traits: tokens.map(|tokens| tokens.iter().map(|token| traits.index_with(token, || match &mut notes {
                    Some(notes) => notes.trait_info(token, || content.trait_info(token)), None => content.trait_info(token),
                })).collect()),
                trait_dates: dates,
                first_seen_hour: evaluated.run.and_then(|run| context.first_seen(run, u128::from(leader.handle), at.hour)),
                fought: leader.battles.fought,
                won: leader.battles.won,
                lost: leader.battles.lost,
            });
        }
        let (icons, icon_warnings) = match notes { Some(notes) => notes.icons(|| content.skill_icons()), None => content.skill_icons() };
        warnings.extend(icon_warnings.iter().cloned());
        Ok(UnitLeadersReply {
            leaders: out,
            traits: traits.traits,
            withheld: evaluated.withheld,
            missing: None,
            battles_partial_from: evaluated.battles_partial_from,
            results_missing: RESULTS_MISSING.into(),
            skill_icons: icons.clone(),
            warnings,
        })
    }

    fn kept<'a>(params: &UnitLeadersParams, frame: &'a DecodedFrame, at: &ReadsAt<'_, LeadersContext>) -> Vec<&'a UnitLeaderRow> {
        let Ok(leaders) = frame.complete::<tables::UnitLeaders>() else { return Vec::new() };
        let roster = at.context.roster.as_deref();
        let tags = Tags::read(frame).ok();
        leaders.iter().filter(|row| params.leaders.includes(row.kind)).filter(|row| {
            let slot = tags.as_ref().and_then(|tags| tags.slot(row.owner));
            let tag = tags.as_ref().and_then(|tags| tags.tag(row.owner));
            params.selection.keeps(slot, tag.as_deref(), roster, at.hour)
        }).collect()
    }

    fn people(kept: &[&UnitLeaderRow]) -> Vec<Read> {
        if kept.is_empty() { return Vec::new(); }
        vec![Read::these(TableId::Characters, kept.iter().map(|row| u128::from(row.character_id))), Read::all(TableId::LexicalTokens)]
    }

    fn visibility_reads(params: &UnitLeadersParams, frame: &DecodedFrame, at: &ReadsAt<'_, LeadersContext>, cached: bool) -> (Vec<Read>, bool) {
        let kept = Self::kept(params, frame, at);
        if !kept.iter().any(|row| needs_world(row)) { return (Vec::new(), false); }
        let Ok(world) = world_of(frame, &at.context.content) else { return (Vec::new(), false) };
        let decided = cached.then(|| at.context.verdicts_at(at.run, at.hour));
        let mut clocked = false;
        for row in kept {
            let known = decided.as_ref().is_some_and(|known| known.contains_key(&handle(row)));
            if !conditioned(row, &world) { continue; }
            if !known {
                return (RECORDED_TABLES.iter().copied().filter(|table| !BASE_TABLES.contains(table)).map(Read::all).collect(), true);
            }
            clocked = true;
        }
        (Vec::new(), clocked)
    }
    pub(crate) fn evaluate<'a>(frame: &'a DecodedFrame, params: &UnitLeadersParams, at: &At<'_, LeadersContext>) -> io::Result<EvaluatedLeaders<'a>> {
        Self::evaluate_keys(frame, params, at, None, None)
    }

    fn evaluate_keys<'a>(frame: &'a DecodedFrame, params: &UnitLeadersParams, at: &At<'_, LeadersContext>, keys: Option<&std::collections::BTreeSet<u64>>, mut notes: Option<&mut ranking::Notes>) -> io::Result<EvaluatedLeaders<'a>> {
        let context = at.context;
        let content = &context.content;
        let mut warnings = content.missing();
        let roster = context.roster.as_deref();
        let tags = Tags::read(frame).map_err(|error| error.to_string());
        let rows = match roster.and_then(|roster| roster.missing(at.hour)) {
            Some(reason) => Err(reason.to_owned()),
            None if !matches!(params.selection, LeaderSelection::All) && let Err(why) = &tags => Err(why.clone()),
            None => frame.complete::<tables::UnitLeaders>().map_err(|error| error.to_string()),
        };
        let leaders = match rows {
            Ok(leaders) => leaders,
            Err(why) => {
                return Ok(EvaluatedLeaders {
                    run: None,
                    leaders: Vec::new(),
                    withheld: Vec::new(),
                    missing: Some(why),
                    tally: None,
                    battles_partial_from: None,
                    warnings,
                });
            }
        };
        let branch = parse_branch(&params.branch_id).map_err(dashboard_core::invalid)?;
        let query = context.query(branch)?;
        let run = query.timeline.at(at.hour).map(|stretch| stretch.run);
        #[cfg(test)]
        if roster.is_some() { dashboard_base::products::at_hour::count_rows(|| leaders.len() as u64); }
        if let Err(why) = &tags {
            warnings.push(format!("Leaders' owners are not named at this hour: {why}"));
        }
        let characters = frame.complete::<tables::Characters>().ok();
        let lexical = frame.complete::<tables::LexicalTokens>().ok();
        let mut shown: Vec<(&'a UnitLeaderRow, u64, Option<String>, Option<String>)> = Vec::new();
        let mut withheld = Vec::new();
        let mut world = None;
        let decided = run.map(|run| context.verdicts_at(run, at.hour)).unwrap_or_default();
        let evaluable = core_gap(frame).is_none();
        let mut made = HashMap::new();
        let rows: Box<dyn Iterator<Item = &UnitLeaderRow>> = match keys {
            Some(keys) => Box::new(keys.iter().filter_map(|key| leaders.binary_search_by_key(key, handle).ok().map(|i| &leaders[i]))),
            None => Box::new(leaders.iter()),
        };
        for row in rows {
            dashboard_text::metadata::check_cancel(at.cancelled)?;
            if !params.leaders.includes(row.kind) || row.owner == 0 {
                continue;
            }
            let reference = handle(row);
            let mut evaluate = || {
                let owner = tags.as_ref().ok().and_then(|tags| tags.tag(row.owner));
                let character = characters.and_then(|rows| rows.binary_search_by_key(&row.character_id, |character| character.id).ok().map(|at| &rows[at]));
                let name = content.person_name(character, lexical);
                if owner.is_none() && tags.is_ok() && roster.is_none() {
                    if matches!(params.selection, LeaderSelection::All) {
                        withheld.push(LeaderWithheld { reference: reference.to_string(), owner: None, kind: row.kind, name, reason: "Leader owner has no resolved recorded country tag".into() });
                    }
                    return;
                }
                let slot = tags.as_ref().ok().and_then(|tags| tags.slot(row.owner));
                if !params.selection.keeps(slot, owner.as_deref(), roster, at.hour) {
                    return;
                }
                let verdict = match decided.get(&reference) {
                    Some(verdict) => verdict.clone(),
                    None => {
                        let verdict = visible(row, frame, content, &mut world);
                        if evaluable && world.as_ref().and_then(|(world, _)| world.as_ref().ok()).is_some_and(|world| conditioned(row, world)) {
                            made.insert(reference, verdict.clone());
                        }
                        verdict
                    }
                };
                match verdict {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(reason) => {
                        withheld.push(LeaderWithheld { reference: reference.to_string(), owner, kind: row.kind, name, reason });
                        return;
                    }
                }
                shown.push((row, reference, owner, name));
            };
            match &mut notes { Some(notes) => notes.leader(reference, evaluate), None => evaluate() }
        }
        if let Some(run) = run {
            context.keep(run, at.hour, made);
        }
        let tally = if shown.is_empty() {
            None
        } else if keys.is_some() {
            let mut ids: Vec<_> = shown.iter().map(|(row, ..)| row.id).collect();
            ids.dedup();
            Some(context.tally(&query, frame, Asked::Ids(&ids))?)
        } else {
            Some(context.tally(&query, frame, Asked::All)?)
        };
        let mut evaluated: Vec<EvaluatedLeader<'a>> = shown
            .into_iter()
            .map(|(row, reference, owner, name)| {
                let skill = (row.skill_valid != 0).then_some(row.skill_level);
                let battles = tally.as_ref().map_or_else(Battles::default, |tally| tally.battles(row.id));
                EvaluatedLeader { row, handle: reference, owner, name, skill, battles }
            })
            .collect();
        evaluated.sort_by(|a, b| {
            leader_rank_cmp(
                a.skill, a.battles.won, a.battles.fought, a.row.components, a.name.as_deref(), a.handle,
                b.skill, b.battles.won, b.battles.fought, b.row.components, b.name.as_deref(), b.handle,
            )
        });
        let battles_partial_from = context.battles_partial(&query, at.hour)?;
        Ok(EvaluatedLeaders { run, leaders: evaluated, withheld, missing: None, tally, battles_partial_from, warnings })
    }
}

fn leader_plan_changes(stage: usize, edits: &dashboard_base::products::at_hour::Edits) -> bool {
    let table = TableId::UnitLeaders;
    match stage {
        1 | 2 => {
            let fields: &[&str] = if stage == 1 { &["owner", "kind", "character_id"] } else {
                &["owner", "kind", "category", "captured", "character_id", "visibility_choice", "template_visibility", "visible_state", "visible_origin"]
            };
            edits.fields(table, dashboard_base::products::at_hour::field_mask(table, fields))
                || edits.fields(TableId::Countries, u64::MAX) || edits.fields(TableId::CountryLinks, u64::MAX)
                || stage == 2 && (edits.fields(TableId::Characters, u64::MAX) || edits.fields(TableId::LexicalTokens, u64::MAX))
        }
        _ => false,
    }
}

fn reply_through(context: &LeadersContext, frame: &DecodedFrame, missing: Option<&str>, warnings: &[String]) -> i32 {
    context.through(frame.hour).min(dashboard_base::products::at_hour::warnings_through(frame.hour, missing.into_iter().chain(warnings.iter().map(String::as_str))))
}

impl AtHour for UnitLeaders {
    const KEY: &'static str = "unit-leaders";
    const TITLE: &'static str = "Generals leaderboard";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::Boundaries;
    type Params = UnitLeadersParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, selection, leaders, "selection.countries[]"];
    type Reply = UnitLeadersReply;
    type Context = LeadersContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![UnitLeadersParams::decl(), LeaderSelection::decl(), LeaderKinds::decl(), LeaderTrait::decl(), LeaderRole::decl(), LeaderWithheld::decl(), LeaderRow::decl(), LeaderSkillIcons::decl(), UnitLeadersReply::decl()]
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
        if !tables.contains(&TableId::UnitLeaderTraits) { tables.push(TableId::UnitLeaderTraits); }
        tables
    }

    fn reads_in(params: &Self::Params, _owner: Option<dashboard_base::products::at_hour::Owner>, step: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> Vec<Read> {
        if at.context.roster.as_ref().is_some_and(|roster| roster.missing(at.hour).is_some()) { return Vec::new(); }
        match step {
            0 => BASE_TABLES.into_iter().map(Read::all).collect(),
            1 => career::member_details(params, held, at, false),
            2 => Self::visibility_reads(params, held, at, true).0,
            _ => Vec::new(),
        }
    }

    fn retained_reads(params: &Self::Params, owner: Option<dashboard_base::products::at_hour::Owner>, stage: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> (Vec<Read>, i32) {
        let reads = match stage {
            1 => career::member_details(params, held, at, false),
            2 => return visibility::prepare(params, held, at),
            _ => Self::reads_in(params, owner, stage, held, at),
        };
        (reads, at.context.through(at.hour))
    }

    const UPDATES_REVISION: bool = true;
    const RESEED_ON_SEEK: bool = true;

    fn context_revision(context: &Self::Context) -> u64 { context.visibility.lock().unwrap_or_else(std::sync::PoisonError::into_inner).revision }

    fn plan_changes(stage: usize, edits: &dashboard_base::products::at_hour::Edits, _held: &DecodedFrame, _at: &ReadsAt<'_, Self::Context>) -> bool {
        leader_plan_changes(stage, edits)
    }

    fn projection(_params: &Self::Params, table: TableId, plan: &[(Vec<Read>, i32)], _context: &Self::Context) -> Option<Vec<usize>> {
        career::leader_columns(table, plan)
    }

    fn advance(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>, edits: &dashboard_base::products::at_hour::Edits, _live: &dashboard_follow::live::Live, data: &mut dashboard_base::products::at_hour::Held<'_>) -> io::Result<bool> {
        at.context.ranking.lock().unwrap_or_else(std::sync::PoisonError::into_inner).board_step(frame, params, at, edits, data)
    }

    fn playback_reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let mut notes = ranking::Notes::default();
        let evaluated = Self::evaluate_keys(frame, params, at, None, Some(&mut notes))?;
        let mut ranking = ranking::Ranking::seed(frame, &evaluated, at.context, notes);
        let reply = Self::board(frame, evaluated, at, Some(&mut ranking.notes))?;
        ranking.seed_traits(&reply);
        *at.context.ranking.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = ranking;
        Ok(reply)
    }

    fn valid_reply_through(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>, reply: &Self::Reply, _partial: bool) -> i32 {
        reply_through(at.context, frame, reply.missing.as_deref(), &reply.warnings)
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        LeadersContext::load(source, load)
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        LeadersContext::for_selection(source, load, &params.branch_id, Some(&params.selection), &AtomicBool::new(false))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        Self::board(frame, Self::evaluate(frame, params, at)?, at, None)
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
    pub resolution: ArtResolution,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LeaderPortraitsReply {
    pub portraits: Vec<LeaderPortrait>,
    pub warnings: Vec<String>,
}

fn portrait(frame: &DecodedFrame, reference: u64, politics: &PoliticsContent, edge: u32) -> ArtResolution {
    let unrecorded = |reason: String| ArtResolution::Unavailable { reason };
    let leaders = match frame.complete::<tables::UnitLeaders>() {
        Ok(leaders) => leaders,
        Err(error) => return unrecorded(error.to_string()),
    };
    let Some(row) = leaders.binary_search_by_key(&reference, handle).ok().map(|at| &leaders[at]) else {
        return ArtResolution::NoArtExpected;
    };
    let holders = match frame.complete::<tables::CharacterPortraits>() {
        Ok(holders) => holders,
        Err(error) => return unrecorded(error.to_string()),
    };
    let Some(holder) = holders.binary_search_by_key(&row.character_id, |holder| holder.character).ok().map(|at| &holders[at]) else {
        return unrecorded("Character portrait holder not recorded".into());
    };
    let category = if row.kind == KIND_NAVY { PORTRAIT_NAVY } else { PORTRAIT_ARMY };
    match replay_data::rows::portrait_entry(&holder.portraits, category, replay_data::rows::PORTRAIT_LARGE).filter(|entry| entry.is_set != 0) {
        Some(entry) => politics.recorded_portrait_art(entry.kind, &entry.identity, edge),
        None => unrecorded("The game generates this portrait from unrecorded inputs".into()),
    }
}

fn leader_portrait(reference: &str, resolution: ArtResolution) -> LeaderPortrait {
    LeaderPortrait {
        reference: reference.to_owned(),
        image: resolution.url().map(str::to_owned),
        missing: resolution.reason().map(str::to_owned),
        resolution,
    }
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
        let every = |resolution: ArtResolution| params.references.iter().map(|reference| leader_portrait(reference, resolution.clone())).collect();
        let Some(stretch) = query.timeline.at(params.hour) else {
            let unrecorded = ArtResolution::Unavailable { reason: format!("Portrait hour not recorded: {}", query.timeline.reason(params.hour)) };
            return Ok(LeaderPortraitsReply { portraits: every(unrecorded), warnings: Vec::new() });
        };
        let politics = context.content().and_then(|options| recorded_politics_content(source, options));
        let politics = match politics {
            Ok(politics) => politics,
            Err(error) => {
                let failed = ArtResolution::Failed { reason: error.to_string(), retryable: true };
                return Ok(LeaderPortraitsReply { portraits: every(failed), warnings: Vec::new() });
            }
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
            portraits.push(leader_portrait(reference, portrait(&frame, reference.parse().unwrap_or_default(), &politics, params.edge)));
        }
        let reply = LeaderPortraitsReply { portraits, warnings: Vec::new() };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
