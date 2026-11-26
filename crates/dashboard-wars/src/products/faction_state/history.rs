
use super::ordinals;
use dashboard_base::{Source, catalogue::{self, Catalogue}, products::{LoadContext, Product, at_hour::{self, Read}}, queries::country_ids::Names, span::HourSpan};
use dashboard_text::scripted_text::ScriptedText;
use dashboard_query::{Query, read, timeline::Join};
use replay_lanes::lanes::{self, Cell, Keys};
use replay_data::rows::{TableId, tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct FactionState;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "view", rename_all = "camelCase", from = "FactionStateAsked")]
pub enum FactionStateParams {
    Rules {
        #[serde(rename = "branchId")]
        branch_id: String,
        faction: String,
        #[doc = " None follows the recording client's playing or observer country."]
        viewpoint: Option<dashboard_base::products::Country>,
    },
    Manpower {
        #[serde(rename = "branchId")]
        branch_id: String,
        faction: String,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FactionStateAsked {
    view: FactionView,
    branch_id: String,
    faction: String,
    viewpoint: Option<dashboard_base::products::Country>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum FactionView {
    Rules,
    Manpower,
}

impl From<FactionStateAsked> for FactionStateParams {
    fn from(asked: FactionStateAsked) -> Self {
        match asked.view {
            FactionView::Rules => Self::Rules { branch_id: asked.branch_id, faction: asked.faction, viewpoint: asked.viewpoint },
            FactionView::Manpower => Self::Manpower { branch_id: asked.branch_id, faction: asked.faction },
        }
    }
}

impl FactionStateParams {
    fn identity(&self) -> (&str, &str) {
        match self {
            Self::Rules { branch_id, faction, .. } | Self::Manpower { branch_id, faction } => (branch_id, faction),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionSelectionRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub token: Option<String>,
    pub name: Option<String>,
    pub reason: Option<String>,
    pub starts_segment: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionSelectionLane {
    pub slot: u32,
    pub runs: Vec<FactionSelectionRun>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionMinimumLane {
    #[doc = " The upgrade group's lexer token: `groups` names it."]
    pub group: u32,
    pub runs: Vec<FactionSelectionRun>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionMemberUpgradeLane {
    #[doc = " Unique among the reply's lanes: the account, its holder and the group."]
    pub key: String,
    #[doc = " The member's tag at its last selection; none when no tag held its slot then."]
    pub tag: Option<String>,
    pub slot: u32,
    pub country_id: u32,
    pub group: u32,
    pub runs: Vec<FactionSelectionRun>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionUpgradeGroup {
    pub group: u32,
    pub name: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionRulesReply {
    pub rules: Vec<FactionSelectionLane>,
    pub programmes: Vec<FactionSelectionLane>,
    pub minimums: Vec<FactionMinimumLane>,
    pub members: Vec<FactionMemberUpgradeLane>,
    pub doctrines: Vec<FactionSelectionLane>,
    pub groups: Vec<FactionUpgradeGroup>,
    #[doc = " Hours whose faction rules table was not read whole: no count of rules is known then."]
    pub rules_unread: Vec<HourSpan>,
    #[doc = " Hours whose doctrine or programme tables were not read whole: no count of upgrades is known then."]
    pub upgrades_unread: Vec<HourSpan>,
    #[doc = " Hours of the timeline the recording does not hold."]
    pub unobserved: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionManpowerRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value: i32,
    pub starts_segment: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionManpowerMember {
    pub key: String,
    #[doc = " The country's tag at the occurrence's first hour; none when no tag held it then."]
    pub tag: Option<String>,
    pub slot: Option<u32>,
    pub country_id: u32,
    pub runs: Vec<FactionManpowerRun>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionManpowerReply {
    pub members: Vec<FactionManpowerMember>,
    pub unobserved: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(untagged)]
pub enum FactionStateReply {
    Rules(FactionRulesReply),
    Manpower(FactionManpowerReply),
}

impl dashboard_base::products::ReplyWarnings for FactionStateReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        match self {
            Self::Rules(reply) => &mut reply.warnings,
            Self::Manpower(reply) => &mut reply.warnings,
        }
    }
}

impl Product for FactionState {
    const KEY: &'static str = "faction-state";
    const CONTENT_DEPENDENT: bool = true;
    type Params = FactionStateParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![view, branchId, faction, viewpoint];
    type Reply = FactionStateReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        let (branch, faction) = params.identity();
        branch.parse::<u64>().map_err(|_| "Invalid faction state branch")?;
        super::faction_id(faction).map(|_| ())
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            FactionStateParams::decl(),
            FactionSelectionRun::decl(),
            FactionSelectionLane::decl(),
            FactionMinimumLane::decl(),
            FactionMemberUpgradeLane::decl(),
            FactionUpgradeGroup::decl(),
            FactionRulesReply::decl(),
            FactionManpowerRun::decl(),
            FactionManpowerMember::decl(),
            FactionManpowerReply::decl(),
            FactionStateReply::decl(),
        ]
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let (branch, faction) = params.identity();
        let branch = branch.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid faction state branch"))?;
        let faction = super::faction_id(faction).map_err(dashboard_core::invalid)?;
        let query = dashboard_base::queries::query(source, branch)?;
        let unobserved = query.timeline.unheld.iter().map(|gap| HourSpan { first_hour: gap.first, last_hour: gap.last }).collect();
        Ok(match params {
            FactionStateParams::Rules { viewpoint, .. } => {
                let cat = context.content().and_then(|options| catalogue::catalogue(source, options)).map_err(|why| why.to_string());
                let mut reply = rules(&query, faction, cat.as_deref().map_err(Clone::clone), viewpoint.as_ref(), cancelled)?;
                reply.unobserved = unobserved;
                FactionStateReply::Rules(reply)
            }
            FactionStateParams::Manpower { .. } => {
                let mut reply = manpower(&query, faction, cancelled)?;
                reply.unobserved = unobserved;
                FactionStateReply::Manpower(reply)
            }
        })
    }
}

type Steps = Vec<(i32, i32, Option<Box<[Cell]>>)>;

type Stretch = (u32, bool, i32, i32, BTreeMap<u128, Steps>, Vec<lanes::StatusRun>);

fn histories(query: &Query<'_>, table: TableId, columns: &[usize], keys: Keys<'_>, cancelled: &AtomicBool) -> io::Result<Vec<Stretch>> {
    let mut out = Vec::new();
    for (index, stretch) in query.timeline.stretches.iter().enumerate() {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let history = dashboard_query::cost::storage(|| lanes::history(query.replay.recording(), stretch.run, table, columns, keys, stretch.first, stretch.last))?;
        let mut held = BTreeMap::new();
        for (key, steps) in history.keys {
            let mut runs: Steps = Vec::with_capacity(steps.len());
            for (at, step) in steps.iter().enumerate() {
                let last = steps.get(at + 1).map_or(history.to, |next| next.hour - 1);
                runs.push((step.hour, last, step.cells.clone()));
            }
            held.insert(key, runs);
        }
        let continues = index > 0 && matches!(stretch.join, Join::Load { .. });
        out.push((stretch.run, !continues, stretch.first, stretch.last, held, history.statuses));
    }
    Ok(out)
}

fn number(cell: Option<&Cell>) -> Option<i64> {
    match cell {
        Some(Cell::Number(value)) => Some(*value),
        _ => None,
    }
}

#[derive(Default)]
pub(super) struct Lane {
    pub(super) runs: Vec<(i32, i32, u64, bool)>,
}

impl Lane {
    pub(super) fn push(&mut self, first: i32, last: i32, value: u64, starts: bool) {
        if let Some(held) = self.runs.last_mut()
            && held.2 == value
            && !starts
            && i64::from(held.1) + 1 == i64::from(first)
        {
            held.1 = last;
            return;
        }
        let starts = starts || self.runs.last().is_none_or(|held| i64::from(held.1) + 1 != i64::from(first));
        self.runs.push((first, last, value, starts));
    }

    fn push_witnessed(&mut self, first: i32, last: i32, value: u64, starts: bool) {
        if let Some(held) = self.runs.last_mut()
            && held.2 == value
            && !starts
            && i64::from(held.1) + 1 == i64::from(first)
        {
            held.1 = last;
            return;
        }
        self.runs.push((first, last, value, starts));
    }
}

fn read_whole(statuses: &[lanes::StatusRun], hour: i32) -> bool {
    statuses.iter().any(|run| run.first <= hour && hour <= run.last && replay_recording::columnar_frames::complete(run.status))
}

fn table_unread(query: &Query<'_>, table: TableId) -> io::Result<Vec<HourSpan>> {
    let mut stretches: Vec<Stretch> = Vec::new();
    for stretch in &query.timeline.stretches {
        let statuses = read::statuses(query.replay.recording(), stretch.run, table, stretch.first, stretch.last)?;
        stretches.push((stretch.run, false, stretch.first, stretch.last, BTreeMap::new(), statuses));
    }
    Ok(unread_spans(&stretches))
}

fn merged_spans(mut spans: Vec<HourSpan>) -> Vec<HourSpan> {
    spans.sort_unstable_by_key(|span| span.first_hour);
    let mut out: Vec<HourSpan> = Vec::with_capacity(spans.len());
    for span in spans {
        match out.last_mut() {
            Some(held) if i64::from(span.first_hour) <= i64::from(held.last_hour) + 1 => held.last_hour = held.last_hour.max(span.last_hour),
            _ => out.push(span),
        }
    }
    out
}

fn unread_spans(stretches: &[Stretch]) -> Vec<HourSpan> {
    let mut out: Vec<HourSpan> = Vec::new();
    for (_, _, first, last, _, statuses) in stretches {
        let mut hour = *first;
        while hour <= *last {
            let run = statuses.iter().find(|run| run.first <= hour && hour <= run.last);
            let end = match run {
                Some(run) => run.last.min(*last),
                None => statuses.iter().map(|run| run.first).filter(|&start| start > hour).min().map_or(*last, |start| start - 1).min(*last),
            };
            if !run.is_some_and(|run| replay_recording::columnar_frames::complete(run.status)) {
                match out.last_mut() {
                    Some(held) if held.last_hour + 1 == hour => held.last_hour = end,
                    _ => out.push(HourSpan { first_hour: hour, last_hour: end }),
                }
            }
            hour = match end.checked_add(1) {
                Some(next) => next,
                None => break,
            };
        }
    }
    out
}

fn ordinal_lanes(query: &Query<'_>, table: TableId, column: usize, faction: u64, cancelled: &AtomicBool) -> io::Result<(BTreeMap<u32, Lane>, Vec<HourSpan>)> {
    let (low, high) = ordinals(faction);
    let stretches = histories(query, table, &[column], Keys::Range(low, high), cancelled)?;
    Ok((witnessed_lanes(&stretches), unread_spans(&stretches)))
}

fn witnessed_lanes(stretches: &[Stretch]) -> BTreeMap<u32, Lane> {
    let mut lanes: BTreeMap<u32, Lane> = BTreeMap::new();
    for (index, (_, breaks, first, _, held, statuses)) in stretches.iter().enumerate() {
        let before: &[lanes::StatusRun] = if *breaks { &[] } else { index.checked_sub(1).map_or(&[][..], |at| stretches[at].5.as_slice()) };
        for (key, steps) in held {
            let lane = lanes.entry(*key as u32).or_default();
            for (a, b, cells) in steps {
                if let Some(value) = number(cells.as_deref().and_then(<[Cell]>::first)) {
                    let unwitnessed_before = if a == first { *breaks || !read_whole(before, a - 1) } else { !read_whole(statuses, a - 1) };
                    lane.push_witnessed(*a, *b, value as u64, unwitnessed_before);
                }
            }
        }
    }
    lanes
}

#[derive(Default)]
struct Lexicon {
    runs: HashMap<u32, HashMap<u32, String>>,
}

impl Lexicon {
    fn spelling(&mut self, query: &Query<'_>, hour: i32, token: u32) -> Result<String, String> {
        let run = query.timeline.at(hour).ok_or("The hour is not recorded")?.run;
        if let std::collections::hash_map::Entry::Vacant(vacant) = self.runs.entry(run) {
            let read = (|| -> io::Result<HashMap<u32, String>> {
                let recording = query.replay.recording();
                let held = recording.run(run)?;
                let end = held.first_hour + held.ordinals as i32 - 1;
                let snapshot = read::at(recording, run, &[read::Read::all(TableId::LexicalTokens)], end)?;
                let view = snapshot[0].view();
                Ok(view
                    .rows()
                    .filter_map(|slot| Some((view.key(slot)? as u32, String::from_utf8(read::text(&view, 0, slot)?.to_vec()).ok()?)))
                    .collect())
            })()
            .map_err(|why| why.to_string())?;
            vacant.insert(read);
        }
        self.runs[&run].get(&token).cloned().ok_or_else(|| format!("Faction token {token} has no lexical tokens spelling"))
    }
}

struct Naming<'q, 'r> {
    query: &'q Query<'r>,
    faction: u64,
    cat: Result<&'q Catalogue, String>,
    lexicon: Lexicon,
    names: Names,
    viewpoint: Option<&'q dashboard_base::products::Country>,
    frames: HashMap<(i32, bool), Arc<replay_frame::frame::DecodedFrame>>,
}

fn localise(cat: &Catalogue, key: &[u8]) -> Result<String, String> {
    cat.localisation.localise(key).map_err(|why| why.to_string())
}

fn naming_tables(scripted: bool) -> Vec<Read> {
    let mut tables: Vec<TableId> = if scripted { dashboard_text::scripted_text::TABLES.to_vec() } else { Vec::new() };
    tables.extend([TableId::LocalPlayer, TableId::CountryLinks, TableId::Countries, TableId::LexicalTokens]);
    if !scripted {
        tables.extend([TableId::ProjectPrograms, TableId::Projects]);
    }
    tables.sort_unstable();
    tables.dedup();
    tables.into_iter().map(Read::all).collect()
}

impl Naming<'_, '_> {
    fn frame(&mut self, hour: i32, scripted: bool) -> Result<Arc<replay_frame::frame::DecodedFrame>, String> {
        if let Some(frame) = self.frames.get(&(hour, scripted)) {
            return Ok(Arc::clone(frame));
        }
        let frame = Arc::new(at_hour::frame(self.query, hour, &naming_tables(scripted), "Faction state").map_err(|why| why.to_string())?);
        self.frames.insert((hour, scripted), Arc::clone(&frame));
        Ok(frame)
    }

    fn expand(&mut self, hour: i32, text: String) -> Result<String, String> {
        if !text.contains('[') {
            return Ok(text);
        }
        let cat = self.cat.clone()?;
        let frame = self.frame(hour, true)?;
        let root = super::goals::viewer(&frame, self.viewpoint)?;
        ScriptedText::new(&*frame, &cat.localisation, cat.scripted_content()?, root).viewing(root).expand_faction(&text, self.faction)
    }

    fn plain(&mut self, hour: i32, token: u32) -> (Option<String>, Result<String, String>) {
        let spelling = match self.lexicon.spelling(self.query, hour, token) {
            Ok(spelling) => spelling,
            Err(why) => return (None, Err(why)),
        };
        let name = (|| {
            let cat = self.cat.clone()?;
            let text = localise(cat, spelling.as_bytes())?;
            let text = self.expand(hour, text)?;
            catalogue::plain_text(&text)
        })();
        (Some(spelling), name)
    }

    fn upgrade(&mut self, hour: i32, token: u32) -> (Option<String>, Result<String, String>) {
        let spelling = match self.lexicon.spelling(self.query, hour, token) {
            Ok(spelling) => spelling,
            Err(why) => return (None, Err(why)),
        };
        let name = (|| {
            let cat = self.cat.clone()?;
            let definitions = content_reader_politics::faction_upgrades::cached(&cat.content.layers)?;
            let definition = definitions.upgrade(spelling.as_bytes())?;
            let text = localise(cat, definition.name.as_deref().unwrap_or(spelling.as_bytes()))?;
            if text.contains('[') {
                return Err(format!("Faction upgrade {spelling}: dynamic member name environment is unavailable"));
            }
            catalogue::plain_text(&text)
        })();
        (Some(spelling), name)
    }

    fn group(&mut self, hour: i32, group: u32) -> Result<String, String> {
        let spelling = self.lexicon.spelling(self.query, hour, group)?;
        let cat = self.cat.clone().map_err(|why| format!("Faction upgrade group {spelling}: {why}"))?;
        let definitions = content_reader_politics::faction_upgrades::cached(&cat.content.layers)?;
        let definition = definitions.group(spelling.as_bytes())?;
        let key = definition.name.as_deref().ok_or_else(|| format!("Faction upgrade group {spelling}: authored name is unavailable"))?;
        let text = localise(cat, key)?;
        if text.contains('[') {
            return Err(format!("Faction upgrade group {spelling}: dynamic name environment is unavailable"));
        }
        catalogue::plain_text(&text)
    }

    fn programme(&mut self, hour: i32, reference: u64) -> Result<Option<String>, String> {
        if reference as u32 != engine_projects::nproject_cprogram::REFERENCE_TYPE {
            return Err("Shared programme reference does not name a research programme".into());
        }
        let frame = self.frame(hour, false)?;
        let programme = frame
            .complete::<tables::ProjectPrograms>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| u64::from(row.id) == reference >> 32)
            .ok_or("Shared programme is not recorded")?
            .clone();
        let Some(active) = programme.active_project else { return Ok(None) };
        let project = frame
            .complete::<tables::Projects>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| row.owner == programme.owner && row.local_id == active && row.program_reference == reference)
            .ok_or("Shared programme's active project is not recorded")?
            .clone();
        let token = self.lexicon.spelling(self.query, hour, project.definition_id)?;
        let cat = self.cat.clone()?;
        let catalog = content_reader_military::special_projects::Catalog::cached(&cat.content.layers)?;
        if let Some(reason) = &catalog.unsettled {
            return Err(reason.clone());
        }
        let definition = catalog.projects.get(&token).ok_or("Shared project definition is absent from recorded content")?;
        if let Some(reason) = &definition.refusal {
            return Err(reason.clone());
        }
        let key = definition.name.as_deref().filter(|name| !name.is_empty()).unwrap_or(&token);
        let links = frame.complete::<tables::CountryLinks>().map_err(|why| why.to_string())?;
        let slot = dashboard_text::metadata::link_slot(links, programme.owner).ok_or("Shared project country is not linked")?;
        let country = self.names.slot_tag(slot, hour).ok_or("No country holds the shared project's slot at this hour")?;
        let country_key = format!("{country}_{key}");
        let key = if cat.localisation.get(&country_key).is_some() { country_key } else { key.to_owned() };
        let text = localise(cat, key.as_bytes())?;
        let text = if text.contains('[') {
            let frame = self.frame(hour, true)?;
            ScriptedText::new(&*frame, &cat.localisation, cat.scripted_content()?, slot).expand(&text)?
        } else {
            text
        };
        catalogue::plain_text(&text).map(Some)
    }
}

fn selection(first: i32, last: i32, starts: bool, token: Option<String>, name: Result<Option<String>, String>) -> FactionSelectionRun {
    let (name, reason) = match name {
        Ok(name) => (name, None),
        Err(why) => (None, Some(why)),
    };
    FactionSelectionRun { first_hour: first, last_hour: last, token, name, reason, starts_segment: starts }
}

#[derive(Clone, Copy)]
enum Named {
    Plain,
    Upgrade,
    Programme,
}

fn named(naming: &mut Naming<'_, '_>, runs: Vec<(i32, i32, u64, bool)>, how: Named) -> Vec<FactionSelectionRun> {
    runs.into_iter()
        .map(|(first, last, value, starts)| match how {
                Named::Plain => {
                    let (spelling, name) = naming.plain(first, value as u32);
                    selection(first, last, starts, spelling, name.map(Some))
                }
                Named::Upgrade => {
                    let (spelling, name) = naming.upgrade(first, value as u32);
                    selection(first, last, starts, spelling, name.map(Some))
                }
                Named::Programme => selection(first, last, starts, Some(value.to_string()), naming.programme(first, value)),
        })
        .collect()
}

fn coverage_edges(statuses: &[lanes::StatusRun], breaks: bool, before: &mut Option<bool>, edges: &mut Vec<(i32, bool)>) {
    if breaks {
        *before = None;
    }
    for run in statuses {
        let whole = replay_recording::columnar_frames::complete(run.status);
        edges.push((run.first, whole && *before == Some(false)));
        *before = Some(whole);
    }
}

fn programme_changes(query: &Query<'_>, lanes: &BTreeMap<u32, Lane>, cancelled: &AtomicBool) -> io::Result<HashMap<u64, Vec<(i32, bool)>>> {
    let mut ids: Vec<u128> = lanes.values().flat_map(|lane| lane.runs.iter().map(|run| u128::from(run.2 >> 32))).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut out: HashMap<u64, Vec<(i32, bool)>> = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let field = dashboard_declare::field;
    let columns = [field(TableId::ProjectPrograms, "owner")?, field(TableId::ProjectPrograms, "active_project")?];
    let mut coverage: Vec<(i32, bool)> = Vec::new();
    let mut projects: BTreeMap<u128, BTreeSet<u64>> = BTreeMap::new();
    let mut before: Option<bool> = None;
    for (_, breaks, _, _, held, statuses) in histories(query, TableId::ProjectPrograms, &columns, Keys::These(&ids), cancelled)? {
        coverage_edges(&statuses, breaks, &mut before, &mut coverage);
        for (key, steps) in held {
            for (first, _, cells) in &steps {
                out.entry(key as u64).or_default().push((*first, false));
                let cells = cells.as_deref().unwrap_or_default();
                if let (Some(owner), Some(active)) = (number(cells.first()), number(cells.get(1))) {
                    projects.entry(u128::from(owner as u32) << 32 | u128::from(active as u32)).or_default().insert(key as u64);
                }
            }
        }
    }
    if !projects.is_empty() {
        let keys: Vec<u128> = projects.keys().copied().collect();
        let columns = [field(TableId::Projects, "definition_id")?, field(TableId::Projects, "program_reference")?];
        let mut before: Option<bool> = None;
        for (_, breaks, _, _, held, statuses) in histories(query, TableId::Projects, &columns, Keys::These(&keys), cancelled)? {
            coverage_edges(&statuses, breaks, &mut before, &mut coverage);
            for (key, steps) in held {
                for programme in projects.get(&key).into_iter().flatten() {
                    out.entry(*programme).or_default().extend(steps.iter().map(|step| (step.0, false)));
                }
            }
        }
    }
    for hours in out.values_mut() {
        hours.extend(&coverage);
        hours.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        hours.dedup_by_key(|edge| edge.0);
    }
    Ok(out)
}

fn merge_unchanged(runs: Vec<FactionSelectionRun>) -> Vec<FactionSelectionRun> {
    let mut out: Vec<FactionSelectionRun> = Vec::with_capacity(runs.len());
    for run in runs {
        match out.last_mut() {
            Some(held) if !run.starts_segment && i64::from(held.last_hour) + 1 == i64::from(run.first_hour) && held.token == run.token && held.name == run.name && held.reason == run.reason => {
                held.last_hour = run.last_hour;
            }
            _ => out.push(run),
        }
    }
    out
}

fn split_programmes(lanes: BTreeMap<u32, Lane>, changes: &HashMap<u64, Vec<(i32, bool)>>) -> BTreeMap<u32, Lane> {
    lanes
        .into_iter()
        .map(|(slot, lane)| {
            let mut runs = Vec::with_capacity(lane.runs.len());
            for (first, last, value, starts) in lane.runs {
                let (mut from, mut opens) = (first, starts);
                for &(cut, restart) in changes.get(&(value >> 32)).map_or(&[][..], Vec::as_slice) {
                    if cut > from && cut <= last {
                        runs.push((from, cut - 1, value, opens));
                        (from, opens) = (cut, restart);
                    }
                }
                runs.push((from, last, value, opens));
            }
            (slot, Lane { runs })
        })
        .collect()
}

fn slot_lanes(naming: &mut Naming<'_, '_>, lanes: BTreeMap<u32, Lane>, how: Named) -> Vec<FactionSelectionLane> {
    lanes
        .into_iter()
        .filter(|(_, lane)| !lane.runs.is_empty())
        .map(|(slot, lane)| FactionSelectionLane { slot, runs: named(naming, lane.runs, how) })
        .collect()
}

fn rules(query: &Query<'_>, faction: u64, cat: Result<&Catalogue, String>, viewpoint: Option<&dashboard_base::products::Country>, cancelled: &AtomicBool) -> io::Result<FactionRulesReply> {
    let mut naming = Naming { query, faction, cat, lexicon: Lexicon::default(), names: Names::new(&query.holders), viewpoint, frames: HashMap::new() };
    let field = dashboard_declare::field;
    let (lanes, rules_unread) = ordinal_lanes(query, TableId::FactionRules, field(TableId::FactionRules, "token")?, faction, cancelled)?;
    let rules = slot_lanes(&mut naming, lanes, Named::Plain);
    let (lanes, doctrines_unread) = ordinal_lanes(query, TableId::FactionDoctrine, field(TableId::FactionDoctrine, "token")?, faction, cancelled)?;
    let doctrines = slot_lanes(&mut naming, lanes, Named::Plain);
    let (lanes, programmes_unread) = ordinal_lanes(query, TableId::FactionPrograms, field(TableId::FactionPrograms, "reference")?, faction, cancelled)?;
    let programmes = match programme_changes(query, &lanes, cancelled) {
        Ok(changes) => slot_lanes(&mut naming, split_programmes(lanes, &changes), Named::Programme)
            .into_iter()
            .map(|lane| FactionSelectionLane { runs: merge_unchanged(lane.runs), ..lane })
            .collect(),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
        Err(error) => {
            let reason = format!("Shared programme history was not read: {error}");
            lanes
                .into_iter()
                .filter(|(_, lane)| !lane.runs.is_empty())
                .map(|(slot, lane)| FactionSelectionLane {
                    slot,
                    runs: lane.runs.into_iter().map(|(first, last, value, starts)| selection(first, last, starts, Some(value.to_string()), Err(reason.clone()))).collect(),
                })
                .collect()
        }
    };
    let mut groups = BTreeSet::new();
    let mut minimums = Vec::new();
    for (group, lane) in minimum_lanes(query, faction, cancelled)? {
        groups.insert(group);
        minimums.push(FactionMinimumLane { group, runs: named(&mut naming, lane.runs, Named::Upgrade) });
    }
    let mut members = Vec::new();
    for (lane, runs) in member_lanes(query, faction, cancelled)? {
        groups.insert(lane.group);
        let tag = runs.last().and_then(|run| naming.names.slot_tag(lane.slot, run.0)).map(str::to_owned);
        let runs = runs.into_iter().map(|(first, last, token, starts)| (first, last, u64::from(token), starts)).collect();
        members.push(FactionMemberUpgradeLane {
            key: format!("{}:{}:{}:{}", lane.account, lane.slot, lane.id, lane.group),
            tag,
            slot: lane.slot,
            country_id: lane.id,
            group: lane.group,
            runs: named(&mut naming, runs, Named::Upgrade),
        });
    }
    let last = query.timeline.last;
    let groups = groups
        .into_iter()
        .map(|group| match naming.group(last, group) {
            Ok(name) => FactionUpgradeGroup { group, name: Some(name), reason: None },
            Err(why) => FactionUpgradeGroup { group, name: None, reason: Some(why) },
        })
        .collect();
    Ok(FactionRulesReply {
        rules,
        programmes,
        minimums,
        members,
        doctrines,
        groups,
        rules_unread,
        upgrades_unread: merged_spans(
            doctrines_unread
                .into_iter()
                .chain(programmes_unread)
                .chain(table_unread(query, TableId::FactionMinimumUpgrades)?)
                .chain(table_unread(query, TableId::FactionAccounts)?)
                .chain(table_unread(query, TableId::FactionAccountOwners)?)
                .collect(),
        ),
        unobserved: Vec::new(),
        warnings: Vec::new(),
    })
}

fn cell(columns: &[usize], cells: &[Cell], column: usize) -> Option<i64> {
    number(columns.iter().position(|held| *held == column).and_then(|index| cells.get(index)))
}

fn minimum_lanes(query: &Query<'_>, faction: u64, cancelled: &AtomicBool) -> io::Result<BTreeMap<u32, Lane>> {
    let table = TableId::FactionMinimumUpgrades;
    let (group_column, token_column) = (dashboard_declare::field(table, "group")?, dashboard_declare::field(table, "token")?);
    let mut columns = [group_column, token_column];
    columns.sort_unstable();
    let (low, high) = ordinals(faction);
    let mut runs: BTreeMap<u32, Vec<(i32, i32, u64, bool)>> = BTreeMap::new();
    for (_, breaks, first, _, held, _) in histories(query, table, &columns, Keys::Range(low, high), cancelled)? {
        for steps in held.into_values() {
            for (a, b, cells) in steps {
                let cells = cells.as_deref().unwrap_or_default();
                if let (Some(group), Some(token)) = (cell(&columns, cells, group_column), cell(&columns, cells, token_column)) {
                    runs.entry(group as u32).or_default().push((a, b, token as u64, breaks && a == first));
                }
            }
        }
    }
    Ok(runs
        .into_iter()
        .map(|(group, mut held)| {
            held.sort_unstable_by_key(|run| run.0);
            let mut lane = Lane::default();
            for (a, b, token, starts) in held {
                lane.push(a, b, token, starts);
            }
            (group, lane)
        })
        .collect())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct MemberLane {
    account: u32,
    slot: u32,
    id: u32,
    group: u32,
}

fn member_lanes(query: &Query<'_>, faction: u64, cancelled: &AtomicBool) -> io::Result<BTreeMap<MemberLane, Vec<(i32, i32, u32, bool)>>> {
    let owners = TableId::FactionAccountOwners;
    let field = |name| dashboard_declare::field(owners, name);
    let (faction_column, slot_column, id_column, member_column) = (field("faction")?, field("slot")?, field("id")?, field("member")?);
    let mut owner_columns = [faction_column, slot_column, id_column, member_column];
    owner_columns.sort_unstable();
    let accounts = TableId::FactionAccounts;
    let (groups_column, tokens_column) = (dashboard_declare::field(accounts, "upgrade_groups")?, dashboard_declare::field(accounts, "upgrade_tokens")?);
    let mut list_columns = [groups_column, tokens_column];
    list_columns.sort_unstable();
    let mut out: BTreeMap<MemberLane, Vec<(i32, i32, u32, bool)>> = BTreeMap::new();
    for (run, breaks, first, _, held, _) in histories(query, owners, &owner_columns, Keys::All, cancelled)? {
        let mut spans: BTreeMap<u32, Vec<(i32, i32, u32, u32)>> = BTreeMap::new();
        for (key, steps) in held {
            for (a, b, cells) in steps {
                let Some(cells) = cells.as_deref() else { continue };
                let at = |column: usize| cell(&owner_columns, cells, column);
                if at(faction_column).map(|value| value as u64) == Some(faction)
                    && at(member_column) == Some(1)
                    && let (Some(slot), Some(id)) = (at(slot_column), at(id_column))
                {
                    spans.entry(key as u32).or_default().push((a, b, slot as u32, id as u32));
                }
            }
        }
        if spans.is_empty() {
            continue;
        }
        let keys: Vec<u128> = spans.keys().map(|account| u128::from(*account) << 64).collect();
        let (from, to) = spans.values().flatten().fold((i32::MAX, i32::MIN), |(lo, hi), span| (lo.min(span.0), hi.max(span.1)));
        let lists = dashboard_query::cost::storage(|| lanes::history(query.replay.recording(), run, accounts, &list_columns, Keys::These(&keys), from, to))?;
        let mut asks = Vec::new();
        let mut shown: BTreeMap<u32, Vec<(i32, i32, Option<i32>)>> = BTreeMap::new();
        for (key, steps) in &lists.keys {
            let account = (key >> 64) as u32;
            for (at, step) in steps.iter().enumerate() {
                let last = steps.get(at + 1).map_or(lists.to, |next| next.hour - 1);
                let written = step.cells.as_ref().map(|_| step.hour);
                if let Some(hour) = written {
                    asks.push((hour, *key));
                }
                shown.entry(account).or_default().push((step.hour, last, written));
            }
        }
        let mut read: HashMap<(i32, u32), Vec<(u32, u32)>> = HashMap::new();
        dashboard_query::cost::storage(|| {
            lanes::at_each(query.replay.recording(), run, accounts, Some(&list_columns), &asks, &mut |hour, key, row| {
                if let Some((view, slot)) = row {
                    let items = |column: usize| -> Vec<u32> {
                        view.list(column, slot).unwrap_or_default().chunks_exact(4).map(|item| u32::from_le_bytes([item[0], item[1], item[2], item[3]])).collect()
                    };
                    let (groups, tokens) = (items(groups_column), items(tokens_column));
                    read.insert((hour, (key >> 64) as u32), groups.into_iter().zip(tokens).collect());
                }
                Ok(())
            })
        })?;
        for (account, spans) in spans {
            for (a, b, slot, id) in spans {
                for &(lo, hi, written) in shown.get(&account).map(Vec::as_slice).unwrap_or_default() {
                    let (lo, hi) = (lo.max(a), hi.min(b));
                    let Some(pairs) = written.and_then(|hour| read.get(&(hour, account))) else { continue };
                    if lo > hi {
                        continue;
                    }
                    for &(group, token) in pairs {
                        let lane = MemberLane { account, slot, id, group };
                        let runs = out.entry(lane).or_default();
                        let starts = (breaks && lo == first) || lo == a || runs.last().is_none_or(|run: &(i32, i32, u32, bool)| i64::from(run.1) + 1 != i64::from(lo));
                        match runs.last_mut() {
                            Some(run) if run.2 == token && !starts => run.1 = hi,
                            _ => runs.push((lo, hi, token, starts)),
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

fn manpower(query: &Query<'_>, faction: u64, cancelled: &AtomicBool) -> io::Result<FactionManpowerReply> {
    let table = TableId::FactionManpower;
    let (country_column, value_column) = (dashboard_declare::field(table, "country")?, dashboard_declare::field(table, "value")?);
    let mut columns = [country_column, value_column];
    columns.sort_unstable();
    let (low, high) = ordinals(faction);
    let names = Names::new(&query.holders);
    let mut members: BTreeMap<(u32, u32, Option<u32>), Vec<FactionManpowerRun>> = BTreeMap::new();
    let mut tags: HashMap<(u32, u32, Option<u32>), Option<String>> = HashMap::new();
    let mut unresolved = 0usize;
    for (run, breaks, first, _, held, _) in histories(query, table, &columns, Keys::Range(low, high), cancelled)? {
        let mut segments = Vec::new();
        for (key, steps) in held {
            for (a, b, cells) in steps {
                let Some(cells) = cells.as_deref() else { continue };
                if let (Some(country), Some(value)) = (cell(&columns, cells, country_column), cell(&columns, cells, value_column)) {
                    segments.push((key as u32, a, b, country as u32, value as i32));
                }
            }
        }
        segments.sort_unstable_by_key(|segment| (segment.1, segment.0));
        let asks: Vec<(i32, u128)> = segments.iter().map(|segment| (segment.1, u128::from(segment.3))).collect();
        let slot_column = dashboard_declare::field(TableId::CountryLinks, "canonical_slot")?;
        let mut slots: HashMap<(i32, u32), u32> = HashMap::new();
        dashboard_query::cost::storage(|| {
            lanes::at_each(query.replay.recording(), run, TableId::CountryLinks, Some(&[slot_column]), &asks, &mut |hour, key, row| {
                if let Some(slot) = row.and_then(|(view, slot)| view.number(slot_column, slot)) {
                    slots.insert((hour, key as u32), slot as u32);
                }
                Ok(())
            })
        })?;
        for &(ordinal, a, b, country, value) in &segments {
            let occurrence = segments.iter().filter(|other| other.3 == country && other.0 < ordinal && other.1 <= a && a <= other.2).count() as u32;
            let slot = slots.get(&(a, country)).copied();
            if slot.is_none() {
                unresolved += 1;
            }
            let member = (country, occurrence, slot);
            tags.entry(member).or_insert_with(|| slot.and_then(|slot| names.slot_tag(slot, a)).map(str::to_owned));
            let runs = members.entry(member).or_default();
            let starts = (breaks && a == first) || runs.last().is_none_or(|run| i64::from(run.last_hour) + 1 != i64::from(a));
            match runs.last_mut() {
                Some(run) if run.value == value && !starts => run.last_hour = b,
                _ => runs.push(FactionManpowerRun { first_hour: a, last_hour: b, value, starts_segment: starts }),
            }
        }
    }
    let mut members: Vec<FactionManpowerMember> = members
        .into_iter()
        .map(|((country, occurrence, slot), mut runs)| {
            runs.sort_unstable_by_key(|run| run.first_hour);
            let tag = tags.remove(&(country, occurrence, slot)).flatten();
            FactionManpowerMember {
                key: format!("{occurrence}:{}:{country}:{}", slot.map_or_else(String::new, |slot| slot.to_string()), tag.as_deref().unwrap_or_default()),
                tag,
                slot,
                country_id: country,
                runs,
            }
        })
        .collect();
    members.sort_by_key(|member| member.runs.first().map(|run| run.first_hour));
    let warnings = if unresolved > 0 { vec![format!("{unresolved} faction manpower runs name a country no link resolves at their first hour")] } else { Vec::new() };
    Ok(FactionManpowerReply { members, unobserved: Vec::new(), warnings })
}

#[cfg(test)]
mod witnessed_tests {
    use super::*;
    use replay_columnar::manifest::Status;

    #[test]
    fn coverage_recovered_after_a_continuous_load_restarts_the_programme_date() {
        let run = |first: i32, last: i32, status: Status| lanes::StatusRun { first, last, status };
        let (mut before, mut edges) = (None, Vec::new());
        coverage_edges(&[run(100, 110, Status::NotRecorded)], true, &mut before, &mut edges);
        coverage_edges(&[run(111, 120, Status::Complete)], false, &mut before, &mut edges);
        assert_eq!(edges, [(100, false), (111, true)], "coverage carried across a continuous load: the recovery hour starts a segment");
        coverage_edges(&[run(121, 130, Status::Complete)], true, &mut before, &mut edges);
        assert_eq!(edges.last(), Some(&(121, false)), "a break starts afresh");
    }

    #[test]
    fn a_programme_named_alike_across_a_boundary_keeps_one_run_and_its_first_date() {
        let run = |first_hour: i32, last_hour: i32, name: &str, starts_segment: bool| FactionSelectionRun { first_hour, last_hour, token: Some("5".into()), name: Some(name.into()), reason: None, starts_segment };
        let merged = merge_unchanged(vec![run(100, 149, "A", true), run(150, 199, "A", false), run(200, 250, "B", false), run(251, 300, "B", true)]);
        assert_eq!(merged.iter().map(|run| (run.first_hour, run.last_hour)).collect::<Vec<_>>(), [(100, 199), (200, 250), (251, 300)], "an unchanged name keeps its date; a renamed project and a new segment start their own");
    }

    #[test]
    fn a_programme_run_is_named_again_where_its_programme_changes_project() {
        let reference = 5u64 << 32 | 3;
        let lanes = BTreeMap::from([(0u32, Lane { runs: vec![(100, 200, reference, true)] })]);
        let changes = HashMap::from([(5u64, vec![(90, false), (150, false), (180, true)])]);
        assert_eq!(split_programmes(lanes, &changes)[&0].runs, [(100, 149, reference, true), (150, 179, reference, false), (180, 200, reference, true)], "a run read again after its coverage returns starts a segment, so its date is not recorded");
    }

    #[test]
    fn a_rule_first_seen_after_a_witnessed_empty_hour_is_dated_and_an_unread_table_is_not_zero_rules() {
        let run = |first: i32, last: i32, status: Status| lanes::StatusRun { first, last, status };
        let cells = |value: i64| Some(vec![Cell::Number(value)].into_boxed_slice());
        let held = BTreeMap::from([(3u128, vec![(150, 200, cells(42))]), (4u128, vec![(100, 200, cells(7))])]);
        let stretch: Stretch = (0, true, 100, 200, held, vec![run(100, 159, Status::Complete), run(160, 170, Status::NotRecorded), run(171, 200, Status::Complete)]);
        let lanes = witnessed_lanes(std::slice::from_ref(&stretch));
        assert_eq!(lanes[&3].runs, [(150, 200, 42, false)], "the slot was read empty at 149: the rule was enacted at 150");
        assert_eq!(lanes[&4].runs, [(100, 200, 7, true)], "held when the stretch began: when it began is not known");
        assert_eq!(unread_spans(&[stretch]), [HourSpan { first_hour: 160, last_hour: 170 }]);
    }
}
