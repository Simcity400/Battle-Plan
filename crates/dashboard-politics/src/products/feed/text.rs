
use super::{EventFeedItem, EventFeedKind, EventFeedOption, EventFeedUnavailable, row::FeedEvent, row::Slot};
use dashboard_base::{catalogue::{self, Catalogue}, products::artwork};
use dashboard_effects::{ScriptedEffects, effect_description};
use dashboard_text::{scripted_text::{EventObject, EventScope, EventScopes, Link, ScopeName, ScriptedText}};
use content_reader_layers::content::diagnostics::Diagnostics;
use dashboard_query::Query;
use replay_lanes::{at::Read, dated::Dated, lanes::Keys};
use replay_recording::columnar_frames::Recording;
use replay_data::rows::{CountryAppearanceRow, CountryRow, PoliticalPartyRow, PoliticsRow, ReservedCountryNamingRow, StateRow, TableId, mask, tables};
use replay_frame::frame::{DecodedFrame, HourFrame, Metrics, SharedView, TableView, discovering_fields, not_selected};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    io,
    rc::Rc,
    sync::{Arc, Mutex, PoisonError},
};

pub(super) const NAMING: [TableId; 9] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::PartyLeaders,
    TableId::LexicalTokens,
    TableId::ReservedCountryNaming,
    TableId::States,
];

#[derive(Clone, Default)]
pub(crate) struct Notices {
    list: Vec<EventFeedUnavailable>,
    keys: HashMap<(String, String, Option<String>, bool), (usize, Vec<String>)>,
}

impl Notices {
    pub fn unavailable(&mut self, kind: &str, label: &str, reason: impl Into<String>, count: Option<u32>) {
        let reason = reason.into();
        self.merge((kind.to_owned(), label.to_owned(), Some(reason.clone()), count.is_some()), reason, count, Vec::new());
    }

    pub fn sampled(&mut self, kind: &str, label: &str, reason: impl Into<String>, count: u32) {
        let reason: String = reason.into().chars().take(240).collect();
        self.merge((kind.to_owned(), label.to_owned(), None, true), reason.clone(), Some(count), vec![reason]);
    }

    fn merge(&mut self, key: (String, String, Option<String>, bool), reason: String, count: Option<u32>, samples: Vec<String>) {
        if let Some((index, kept)) = self.keys.get_mut(&key) {
            let held = &mut self.list[*index];
            if key.3 {
                held.count = held.count.zip(count).and_then(|(old, new)| old.checked_add(new));
            }
            for sample in samples {
                if kept.len() < 3 && !kept.contains(&sample) {
                    held.reason.push_str("; example: ");
                    held.reason.push_str(&sample);
                    kept.push(sample);
                }
            }
            return;
        }
        self.keys.insert(key.clone(), (self.list.len(), samples));
        self.list.push(EventFeedUnavailable { kind: key.0, label: key.1, reason, count });
    }

    pub fn into_list(self) -> Vec<EventFeedUnavailable> {
        self.list
    }
}

const LINK_COUNTRY: u64 = mask::<CountryRow>(&["canonical_slot"]);
const LINK_SLOT: u64 = mask::<replay_data::rows::CountryLinkRow>(&["canonical_slot"]);

struct Links<'f> {
    countries: &'f [replay_data::rows::CountryRow],
    links: &'f [replay_data::rows::CountryLinkRow],
}

impl<'f> Links<'f> {
    fn of(frame: &'f DecodedFrame) -> Option<Self> {
        Some(Self { countries: frame.complete_fields::<tables::Countries>(LINK_COUNTRY).ok()?, links: frame.complete_fields::<tables::CountryLinks>(LINK_SLOT).ok()? })
    }

    fn canonical(&self, reference: u32) -> Option<u32> {
        let at = self.links.binary_search_by_key(&reference, |row| row.id).ok()?;
        Some(self.links[at].canonical_slot)
    }

    fn slot(&self, reference: u32, id: Option<u32>) -> Option<u32> {
        if reference as i32 <= 0 {
            return Some(0);
        }
        let slot = self.canonical(reference)?;
        let Some(id) = id else { return Some(slot) };
        let at = self.countries.binary_search_by_key(&id, |row| row.id).ok()?;
        (self.countries[at].canonical_slot == slot && self.canonical(id)? == slot).then_some(slot)
    }
}

struct DecisionTarget {
    country: Option<u32>,
    numeric: Option<i32>,
    slot: Option<u32>,
}

struct ScopeInput {
    reference: u32,
    attempted: bool,
    slot: Option<u32>,
    state: i32,
    bindings: [u32; 3],
    random: [u32; 2],
    unsupported: bool,
    opaque: Option<String>,
    names: Option<[Vec<u8>; 3]>,
}

const OBJECTS: [&str; 9] = ["a character", "an operation", "an ace", "a unit", "an industrial organisation", "a purchase contract", "a raid", "a project", "a faction"];

pub(super) struct Inputs {
    option: Option<i32>,
    actor_slot: Option<u32>,
    targeted: bool,
    target: Option<DecisionTarget>,
    missing: bool,
    scopes: Vec<ScopeInput>,
}

impl Inputs {
    fn of(event: &FeedEvent, frame: Option<&DecodedFrame>) -> Self {
        let links = frame.and_then(Links::of);
        let slot = |reference: u32, id: Option<u32>| links.as_ref().and_then(|links| links.slot(reference, id));
        let identities = [event.actor, event.target];
        let targeted = event.country_target.is_some() || event.numeric_target.is_some();
        Self {
            option: event.option,
            actor_slot: match event.actor.slot {
                Slot::Of(slot) => Some(slot),
                Slot::None => Some(0),
                Slot::NotRecorded => None,
            },
            targeted,
            target: targeted.then(|| DecisionTarget {
                country: event.country_target,
                numeric: event.numeric_target,
                slot: event.country_target.filter(|country| *country as i32 > 0).and_then(|country| slot(country, None)),
            }),
            missing: event.scopes.is_none(),
            scopes: event
                .scopes
                .iter()
                .flatten()
                .map(|scope| {
                    let opaque: Vec<&str> = OBJECTS
                        .iter()
                        .zip(scope.references)
                        .filter(|(_, reference)| *reference != 0)
                        .map(|(kind, _)| *kind)
                        .chain(scope.strategic_region.map(|_| "a strategic region"))
                        .chain(scope.combat_context_unrecorded.then_some("a combat"))
                        .collect();
                    let id = scope.country_id.or_else(|| identities.iter().find(|party| party.reference == scope.country).and_then(|party| party.id));
                    let resolved = if scope.country as i32 <= 0 {
                        Some(0)
                    } else {
                        id.and_then(|id| slot(scope.country, Some(id)))
                    };
                    ScopeInput {
                        reference: scope.country,
                        attempted: scope.country_attempted,
                        slot: resolved,
                        state: scope.state,
                        bindings: scope.bindings,
                        random: scope.random,
                        unsupported: !opaque.is_empty() || scope.target_storage_unrecorded || scope.tooltip_storage_unrecorded,
                        opaque: (!opaque.is_empty()).then(|| opaque.join(" and ")),
                        names: scope.country_names.map(|texts| texts.map(|text| text.as_bytes().to_vec())),
                    }
                })
                .collect(),
        }
    }

    fn graph(&self, decision: bool) -> Result<EventScopes, String> {
        if decision {
            let slot = self.actor_slot.ok_or("decision actor has no exact occurrence-hour canonical slot")?;
            let mut scope = EventScope::of(EventObject::Country(slot));
            scope.random_seed_refusal = Some("decision execution random seed was not recorded");
            if self.targeted {
                let target = self.target.as_ref().ok_or("targeted decision selector was not recorded")?;
                let object = if target.country.is_some_and(|country| country as i32 > 0) {
                    EventObject::Country(target.slot.ok_or("targeted decision country has no exact occurrence-hour slot")?)
                } else {
                    EventObject::State(target.numeric.ok_or("targeted decision numeric selector was not recorded")? as u32)
                };
                let mut from = EventScope::of(object);
                from.random_seed_refusal = scope.random_seed_refusal;
                scope.from = Link::Scope(1);
                from.from = Link::Scope(0);
                return Ok(EventScopes::new(vec![scope, from]));
            }
            return Ok(EventScopes::new(vec![scope]));
        }
        if self.missing || self.scopes.is_empty() {
            return Err("event execution scope graph was not recorded".into());
        }
        for (id, scope) in self.scopes.iter().enumerate() {
            if scope.unsupported {
                return Err(format!("scope {id} carries unsupported object, region, combat or saved/tooltip-target operands"));
            }
            if scope.slot.is_none() {
                return Err(format!("scope {id} country reference {} has no exact occurrence-hour canonical slot", scope.reference));
            }
        }
        Ok(self.scope_graph())
    }

    fn scope_graph(&self) -> EventScopes {
        let scopes = self
            .scopes
            .iter()
            .map(|scope| {
                let country = scope.slot.unwrap_or(0);
                let object = match (scope.state > 0, country) {
                    (true, 0) => EventObject::State(scope.state as u32),
                    (true, country) => EventObject::CountryState { country, state: scope.state as u32 },
                    (false, country) => EventObject::Country(country),
                };
                let [root, from, prev] = scope.bindings.map(|index| Link::Scope(index as usize));
                let mut resolved = EventScope::of(object);
                resolved.root = root;
                resolved.from = from;
                resolved.prev = prev;
                resolved.random_seed = scope.random[0];
                resolved.random_counter = scope.random[1];
                resolved
            })
            .collect();
        EventScopes::new(scopes)
    }

    fn title_graph(&self) -> Result<(EventScopes, Vec<ScopeName>), String> {
        if !self.scopes.iter().any(|scope| scope.attempted) {
            return Ok((self.graph(false)?, Vec::new()));
        }
        let names = self
            .scopes
            .iter()
            .map(|scope| match (&scope.opaque, scope.slot, &scope.names) {
                (Some(kinds), ..) => ScopeName::Unproven(format!("it carries {kinds}")),
                (None, None, _) => ScopeName::Unproven(format!("its country reference {} has no exact occurrence-hour canonical slot", scope.reference)),
                (None, Some(_), Some(names)) => ScopeName::Recorded(names.clone()),
                (None, Some(_), None) if scope.reference as i32 <= 0 => ScopeName::Unproven("it holds no country".into()),
                (None, Some(_), None) => ScopeName::Unproven("its country's names were not recorded: unreadable, too long or possibly mid-rename".into()),
            })
            .collect();
        Ok((self.scope_graph(), names))
    }
}

#[derive(Default)]
struct ScriptContentIndex(Mutex<Option<Arc<content_reader_events::events::LazyEvents>>>);

fn script_events(catalogue: &Catalogue) -> Arc<content_reader_events::events::LazyEvents> {
    let cache = catalogue.content.layers.derived::<ScriptContentIndex>();
    let mut index = cache.0.lock().unwrap_or_else(PoisonError::into_inner);
    Arc::clone(index.get_or_insert_with(|| Arc::new(content_reader_events::events::LazyEvents::from_content(Arc::clone(&catalogue.content)))))
}

pub const SCRIPTED_GAP: char = '\u{FFFC}';

pub(super) fn elide_scripted(text: &str) -> String {
    let mut shown = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find(']') else { break };
        shown.push_str(&rest[..open]);
        shown.push(SCRIPTED_GAP);
        rest = &rest[open + close + 1..];
    }
    shown.push_str(rest);
    shown
}

pub(super) fn names_decision(kind: EventFeedKind) -> bool {
    use EventFeedKind::*;
    matches!(
        kind,
        DecisionSelected
            | DecisionRemoved
            | DecisionCancelled
            | DecisionTimedOut
            | DecisionActivated
            | DecisionCompleted
            | DecisionReenabled
            | DecisionRemovalEffect
            | DecisionCancellationEffect
            | DecisionCompletionEffect
            | DecisionTimeoutEffect
            | DecisionDurationChanged
    )
}

fn presents_event(kind: EventFeedKind) -> bool {
    matches!(kind, EventFeedKind::ScriptEventFired | EventFeedKind::NewsEventFired | EventFeedKind::EventOptionSelected)
}

pub(crate) struct Presenter<'q> {
    query: &'q Query<'q>,
    catalogue: Option<Arc<Catalogue>>,
    frames: RefCell<Frames>,
    kept: Arc<Mutex<KeptNaming>>,
}

const HELD: [(TableId, u64); 8] = [
    (TableId::Countries, mask::<CountryRow>(&["canonical_slot", "tag"])),
    (TableId::CountryLinks, u64::MAX),
    (
        TableId::CountryAppearance,
        mask::<CountryAppearanceRow>(&[
            "cosmetic_tag",
            "name_tag",
            "leader_ideology",
            "overlord",
            "graphical_culture",
            "old_name",
            "old_adjective",
            "old_definite_name",
            "graphical_culture_2d",
            "subjects",
            "original_tag",
            "original_tag_reference",
        ]),
    ),
    (TableId::Politics, mask::<PoliticsRow>(&["last_election", "election_frequency", "elections_allowed", "capital", "ruling_party", "original_capital", "has_elections"])),
    (TableId::PoliticalParties, mask::<PoliticalPartyRow>(&["is_ruling", "token", "tag", "name_default", "name", "long_name"])),
    (TableId::PartyLeaders, u64::MAX),
    (
        TableId::ReservedCountryNaming,
        mask::<ReservedCountryNamingRow>(&[
            "tag",
            "name_tag",
            "cosmetic_tag",
            "leader_ideology",
            "group",
            "overlord",
            "autonomy",
            "built",
            "original_tag",
            "original_tag_reference",
            "original_tag_buckets",
            "country_array",
            "owned_state_count",
            "dynamic_tag_boundary",
            "faction",
            "movement",
            "land_theatre_count",
            "attack",
        ]),
    ),
    (TableId::States, mask::<StateRow>(&["owner", "controller", "custom_name", "owner_name", "continent", "category", "best_vp_index", "infrastructure"])),
];

pub(crate) struct RunNaming {
    branch: u64,
    whole: Vec<(Dated, u64)>,
    lexical: Option<(SharedView, Vec<(i32, i32)>)>,
}

fn complete_spans(recording: &Recording, run: u32) -> Option<Vec<(i32, i32)>> {
    let table = TableId::LexicalTokens;
    if recording.predates(table.index()).is_some() || !matches!(recording.table_coverage(table.index(), run), replay_recording::columnar_frames::TableCoverage::Valid) {
        return None;
    }
    let statuses = replay_lanes::lanes::table_statuses(recording, run, table).ok()?;
    Some(statuses.into_iter().filter(|held| matches!(held.status, replay_columnar::manifest::Status::Complete)).map(|held| (held.first, held.last)).collect())
}

impl RunNaming {
    fn read(recording: &Recording, run: u32) -> io::Result<Self> {
        let held = recording.run(run)?;
        let last = held.first_hour + held.ordinals as i32 - 1;
        let (whole, lexical) = std::thread::scope(|scope| {
            let lexical = scope.spawn(|| replay_lanes::at::frame(recording, run, last, &[Read::all(TableId::LexicalTokens)]));
            let whole = HELD.iter().map(|&(table, fields)| Dated::read(recording, run, table, fields, Keys::All).map(|dated| (dated, fields))).collect::<io::Result<Vec<_>>>();
            (whole, lexical.join().unwrap_or_else(|_| Err(io::Error::other("A lexical dictionary read panicked"))))
        });
        let lexical = lexical
            .ok()
            .map(|frame| frame.tables[TableId::LexicalTokens.index()].clone())
            .filter(|lexical| matches!(lexical.held(), TableView::Complete(_)))
            .and_then(|lexical| complete_spans(recording, run).map(|spans| (lexical, spans)));
        Ok(Self { branch: held.branch, whole: whole?, lexical })
    }
}

#[derive(Default)]
pub(crate) struct KeptNaming {
    runs: BTreeMap<u32, Arc<RunNaming>>,
    drawn: Option<(Arc<Catalogue>, HashMap<(String, String), Arc<std::sync::OnceLock<(catalogue::Drawn<Result<String, String>>, Diagnostics)>>>)>,
}

pub(super) struct Art<'a> {
    catalogue: &'a Arc<Catalogue>,
    kept: &'a Mutex<KeptNaming>,
}

impl Art<'_> {
    fn drawn(&self, key: (&str, &str), draw: impl FnOnce() -> catalogue::Drawn<Result<String, String>>) -> Result<String, String> {
        let once = {
            let mut held = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
            if !held.drawn.as_ref().is_some_and(|(catalogue, _)| Arc::ptr_eq(catalogue, self.catalogue)) {
                held.drawn = Some((Arc::clone(self.catalogue), HashMap::new()));
            }
            let Some((_, drawn)) = held.drawn.as_mut() else { return draw().get().clone() };
            Arc::clone(drawn.entry((key.0.to_owned(), key.1.to_owned())).or_default())
        };
        let (drawn, noted) = once.get_or_init(|| content_reader_layers::content::diagnostics::capture(draw));
        noted.credit();
        drawn.get().clone()
    }
}

pub(crate) fn kept_naming(source: &dashboard_base::Source) -> Arc<Mutex<KeptNaming>> {
    source.state::<KeptNaming>()
}

fn run_naming(kept: &Mutex<KeptNaming>, recording: &Recording, run: u32) -> io::Result<Arc<RunNaming>> {
    let mut held = kept.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(found) = held.runs.get(&run) {
        return Ok(Arc::clone(found));
    }
    let made = Arc::new(dashboard_query::cost::storage(|| RunNaming::read(recording, run))?);
    held.runs.insert(run, Arc::clone(&made));
    Ok(made)
}

pub fn warm_naming(source: &dashboard_base::Source, branch: u64) -> io::Result<()> {
    let query = dashboard_base::queries::query(source, branch)?;
    let Some(stretch) = query.timeline.stretches.last().copied() else { return Ok(()) };
    run_naming(&kept_naming(source), query.replay.recording(), stretch.run).map(drop)
}

#[derive(Default)]
struct Frames {
    views: BTreeMap<(u32, usize), (i32, SharedView)>,
    last: Option<(i32, Result<Rc<DecodedFrame>, String>)>,
}

impl<'q> Presenter<'q> {
    pub fn new(query: &'q Query<'q>, catalogue: Option<Arc<Catalogue>>, kept: Arc<Mutex<KeptNaming>>) -> Self {
        Self { query, catalogue, frames: RefCell::default(), kept }
    }

    pub fn catalogue(&self) -> Option<&Catalogue> {
        self.catalogue.as_deref()
    }

    fn dated(&self, run: u32, hour: i32, frames: &mut Frames) -> io::Result<DecodedFrame> {
        let recording = self.query.replay.recording();
        let naming = run_naming(&self.kept, recording, run)?;
        let mut frame = DecodedFrame { branch: naming.branch, hour, tables: std::array::from_fn(|_| not_selected()), metrics: Metrics::default() };
        for (dated, fields) in &naming.whole {
            let table = dated.table();
            let epoch = dated.epoch(hour);
            let held = |view: SharedView| if *fields & table.full_mask() == table.full_mask() { view } else { view.with_fields(*fields) };
            let view = match frames.views.get(&(run, table.index())) {
                _ if !dated.covers(hour) => held(dated.view(recording, hour)?),
                Some((at, view)) if *at == epoch => view.clone(),
                _ => {
                    let view = dated.view(recording, hour)?;
                    let view = held(view);
                    frames.views.insert((run, table.index()), (epoch, view.clone()));
                    view
                }
            };
            frame.tables[table.index()] = view;
        }
        let complete = |spans: &[(i32, i32)]| spans.get(spans.partition_point(|&(_, last)| last < hour)).is_some_and(|&(first, _)| first <= hour);
        frame.tables[TableId::LexicalTokens.index()] = match &naming.lexical {
            Some((lexical, spans)) if complete(spans) => lexical.clone(),
            _ => dashboard_query::cost::storage(|| replay_lanes::at::frame(recording, run, hour, &[Read::all(TableId::LexicalTokens)]))?.tables[TableId::LexicalTokens.index()].clone(),
        };
        Ok(frame)
    }

    pub(super) fn frame(&self, hour: i32) -> io::Result<Result<Rc<DecodedFrame>, String>> {
        let mut frames = self.frames.borrow_mut();
        if let Some((held, frame)) = &frames.last
            && *held == hour
        {
            return Ok(frame.clone());
        }
        let read = match self.query.timeline.at(hour) {
            None => Err(format!("the recording holds no hour {hour} on this timeline")),
            Some(stretch) => Ok(Rc::new(self.dated(stretch.run, hour, &mut frames)?)),
        };
        frames.last = Some((hour, read.clone()));
        Ok(read)
    }

    fn exact(&self, frame: &DecodedFrame, tables: &[TableId]) -> io::Result<DecodedFrame> {
        let Some(stretch) = self.query.timeline.at(frame.hour) else { return Ok(frame.clone()) };
        let reads: Vec<Read<'_>> = tables.iter().map(|&table| Read::all(table)).collect();
        let read = dashboard_query::cost::storage(|| replay_lanes::at::frame(self.query.replay.recording(), stretch.run, frame.hour, &reads))?;
        let mut exact = frame.clone();
        for &table in tables {
            exact.tables[table.index()] = read.tables[table.index()].clone();
        }
        Ok(exact)
    }

    pub fn picture(&self, token: &str, key: &str, notices: &mut Notices) -> Option<String> {
        draw_picture(notices, &Art { catalogue: self.catalogue.as_ref()?, kept: &self.kept }, token, key)
    }

    pub fn item(&self, event: &FeedEvent, kind: EventFeedKind, token: Option<String>, notices: &mut Notices) -> io::Result<EventFeedItem> {
        let mut item = EventFeedItem { token, ..Default::default() };
        let Some(shown) = self.catalogue.as_ref() else {
            item.name_missing = Some("the recorded content stack could not be read".into());
            return Ok(item);
        };
        let present = |frame: Result<&DecodedFrame, String>, notices: &mut Notices| -> io::Result<EventFeedItem> {
            let mut item = item.clone();
            let inputs = Inputs::of(event, frame.as_ref().ok().copied());
            presentation(notices, &Art { catalogue: shown, kept: &self.kept }, kind, &mut item, &inputs, frame.map(HourFrame::from))?;
            Ok(item)
        };
        let dated = match self.frame(event.hour)? {
            Ok(dated) => dated,
            Err(why) => return present(Err(why), notices),
        };
        let before = notices.clone();
        let (presented, missed) = discovering_fields(|| present(Ok(&dated), notices));
        let mut missed: Vec<TableId> = missed.into_iter().map(|(table, _)| table).filter(|table| NAMING.contains(table)).collect();
        missed.sort_unstable();
        missed.dedup();
        if missed.is_empty() {
            return presented;
        }
        *notices = before;
        present(Ok(&self.exact(&dated, &missed)?), notices)
    }
}

fn presentation(notices: &mut Notices, art: &Art<'_>, kind: EventFeedKind, item: &mut EventFeedItem, inputs: &Inputs, frame: Result<HourFrame<'_>, String>) -> io::Result<()> {
    let catalogue: &Catalogue = art.catalogue;
    if !presents_event(kind) {
        if names_decision(kind) {
            let token = item.token.clone().unwrap_or_else(|| "unidentified decision".to_owned());
            match frame.clone().and_then(|frame| decision_title(catalogue, inputs, frame, &token)) {
                Ok((name, elided)) => {
                    item.name = Some(name);
                    if let Some(reason) = elided {
                        notices.unavailable("decisionPresentation", "Decision presentation", format!("Decision {token}: {reason}; its scripted text is elided."), Some(1));
                        item.gaps = Some(reason);
                    }
                }
                Err(reason) => {
                    notices.unavailable("decisionPresentation", "Decision presentation", format!("Decision {token}: title unavailable: {reason}"), Some(1));
                    item.name_missing = Some(reason);
                }
            }
            let icon = art.drawn(("decision", &token), || catalogue::draw(|| format!("decision {token} icon"), catalogue, |catalogue| decision_icon(catalogue, &token)));
            match icon {
                Ok(url) => item.icon = Some(url),
                Err(reason) => notices.unavailable("decisionIcon", "Decision icon", format!("Decision {token}: {reason}"), Some(1)),
            }
        }
        return effects(notices, catalogue, kind, item, inputs);
    }
    let Some(token) = item.token.clone() else {
        item.name_missing = Some("the event's identity was not recorded".into());
        return Ok(());
    };
    let events = script_events(catalogue);
    let event = match events.get(token.as_bytes()) {
        Ok(event) if event.refused.is_none() => event,
        Ok(event) => {
            let reason = event.refused.clone().unwrap_or_default();
            item.name_missing = Some(reason.clone());
            notices.unavailable("scriptEventContent", "Event content", reason, Some(1));
            return effects(notices, catalogue, kind, item, inputs);
        }
        Err(reason) => {
            item.name_missing = Some(reason.clone());
            notices.unavailable("scriptEventContent", "Event content", reason, Some(1));
            return effects(notices, catalogue, kind, item, inputs);
        }
    };
    let mut scripted = Vec::new();
    let mut localise = |key: &[u8]| -> Result<String, String> {
        let key = std::str::from_utf8(key).map_err(|_| format!("Event {token} localisation key is not UTF-8"))?;
        let text = catalogue.localised(key, catalogue::View::Plain).ok_or_else(|| format!("Event {token}: {key} needs unavailable historical localisation operands"))?;
        if !text.contains('[') {
            return Ok(text);
        }
        let expanded = frame.clone().and_then(|frame| scoped_text(catalogue, inputs, frame, key));
        Ok(expanded.unwrap_or_else(|reason| {
            scripted.push(format!("Event {token}: {key} shows its scripted text elided: {reason}"));
            elide_scripted(&text)
        }))
    };
    for (field, keys) in [("title", event.sole_title_key()), ("body", event.sole_description_key())] {
        match keys.and_then(|key| key.map(&mut localise).transpose()) {
            Ok(Some(value)) if field == "title" => item.name = Some(value),
            Ok(Some(value)) => item.description = Some(value),
            Ok(None) if field == "title" => item.name_missing = Some(format!("event {token} has no title in its content")),
            Ok(None) => {}
            Err(reason) => {
                if field == "title" {
                    item.name_missing = Some(reason.clone());
                }
                notices.unavailable("scriptEventPresentation", &format!("Event {field}"), reason, Some(1));
            }
        }
    }
    if event.picture.is_empty() {
        notices.unavailable("scriptEventPicture", "Event picture", format!("Event {token}: default picture selection has no recorded presentation result"), Some(1));
    } else if let Ok(picture) = std::str::from_utf8(&event.picture) {
        item.icon = draw_picture(notices, art, &token, picture);
    } else {
        notices.unavailable("scriptEventPicture", "Event picture", format!("Event {token}: picture key is not UTF-8"), Some(1));
    }
    let mut options = Vec::new();
    let mut authored = Vec::new();
    for (ordinal, name) in event.option_names.iter().enumerate() {
        let resolved = localise(name);
        authored.push(EventFeedOption { ordinal: u32::try_from(ordinal).map_err(|_| io::Error::other("Too many authored event options"))?, name: resolved.as_ref().ok().cloned() });
        let name = match resolved {
            Ok(name) => name,
            Err(reason) => {
                notices.unavailable("scriptEventOptionName", "Event option name", reason, Some(1));
                String::from_utf8_lossy(name).into_owned()
            }
        };
        options.push(format!("{ordinal}: {name}"));
        for trigger in event.option_triggers.get(ordinal).into_iter().flatten() {
            let key = trigger.key.as_deref().map(String::from_utf8_lossy).unwrap_or_default();
            notices.unavailable("scriptEventOptionAdmission", "Offered event option", format!("Event {token} option {ordinal}: trigger {key} needs exact presentation-time inputs"), Some(1));
        }
        if let Some(Some(original)) = event.original_recipient_only.get(ordinal) {
            let only = if *original { "the original recipient" } else { "a recipient other than the original" };
            notices.unavailable(
                "scriptEventOptionAdmission",
                "Offered event option",
                format!("Event {token} option {ordinal}: original_recipient_only offers it only to {only}, which requires exact presentation ownership"),
                Some(1),
            );
        }
    }
    item.gaps = scripted.first().cloned();
    for reason in scripted {
        notices.unavailable("scriptEventText", "Scripted event text", reason, Some(1));
    }
    item.options = Some(authored);
    if !options.is_empty() {
        let body = item.description.get_or_insert_default();
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str("Authored options (native ordinals):\n");
        body.push_str(&options.join("\n"));
    }
    effects(notices, catalogue, kind, item, inputs)
}

fn draw_picture(notices: &mut Notices, art: &Art<'_>, token: &str, picture: &str) -> Option<String> {
    let catalogue: &Catalogue = art.catalogue;
    let image = art.drawn((token, picture), || catalogue::draw(
        || format!("event {token} picture"),
        catalogue,
        |catalogue| {
            let sprite = catalogue.sprites.find(picture)?.ok_or_else(|| format!("Event {token}: picture {picture} is undeclared"))?;
            artwork::resolve_sprite_type(catalogue, sprite, 0, artwork::ICON_EDGE)
                .into_result()?
                .ok_or_else(|| format!("Event {token}: picture {picture} is unavailable"))
        },
    ));
    match image {
        Ok(icon) => Some(icon),
        Err(reason) => {
            notices.unavailable("scriptEventPicture", "Event picture", reason, Some(1));
            None
        }
    }
}

fn scoped_text(catalogue: &Catalogue, inputs: &Inputs, frame: HourFrame<'_>, key: &str) -> Result<String, String> {
    let (scopes, names) = inputs.title_graph()?;
    let text = catalogue.localisation.localise(key.as_bytes()).map_err(|why| why.to_string())?;
    let expander = ScriptedText::new_in(frame, &catalogue.localisation, catalogue.scripted_content()?, inputs.actor_slot.unwrap_or(0));
    catalogue::plain_text(&expander.expand_names_in(&text, &scopes, &names)?)
}

fn decision_title(catalogue: &Catalogue, inputs: &Inputs, frame: HourFrame<'_>, token: &str) -> Result<(String, Option<String>), String> {
    let content = catalogue.scripted_content()?;
    let effects = content.effects()?;
    let decisions = effects.view(None).decisions.ok_or("decision database is unavailable from the recorded content stack")?;
    let definition = decisions.decision(token.as_bytes())?.ok_or_else(|| format!("decision {token} is absent from recorded content"))?;
    if let Some(reason) = &definition.refused {
        return Err(reason.clone());
    }
    let owner = inputs.actor_slot.ok_or("decision owner has no exact occurrence-hour canonical slot")?;
    let text = ScriptedText::new_in(frame, &catalogue.localisation, content, owner);
    let key = effect_description::decision_name_key_at(frame, text.recorded(), &catalogue.localisation, definition, owner, text.country_tag(owner)?)?;
    let name = catalogue.localised(&key, catalogue::View::Plain).ok_or_else(|| format!("decision title {key} is absent from recorded content"))?;
    if !name.contains('[') {
        return Ok((name, None));
    }
    let expanded = if !definition.has_state_targets && definition.highlight_conditions.is_empty() {
        decision_title_text(catalogue, inputs, frame, &key)
    } else {
        Err("it names a highlighted state the recording does not hold".into())
    };
    Ok(match expanded {
        Ok(name) => (name, None),
        Err(reason) => (elide_scripted(&name), Some(format!("decision title {key}: {reason}"))),
    })
}

fn decision_icon(catalogue: &Catalogue, token: &str) -> Result<String, String> {
    let content = catalogue.scripted_content()?;
    let effects = content.effects()?;
    let decisions = effects.view(None).decisions.ok_or("decision database is unavailable from the recorded content stack")?;
    let definition = decisions.decision(token.as_bytes())?.ok_or_else(|| format!("decision {token} is absent from recorded content"))?;
    let selected: Result<Option<&str>, String> = match (&definition.icon_refusal, definition.icons.as_slice()) {
        (Some(reason), _) => Err(reason.clone()),
        (None, []) => Ok(None),
        (None, [one]) => Ok(Some(one.text.as_str())),
        (None, _) => Err("its icon is chosen by triggers whose historical inputs the recording does not hold".to_owned()),
    };
    let candidates = match &selected {
        Ok(chosen) => definition.sprite_candidates(*chosen)?,
        Err(_) => definition.sprite_candidates(None)?.into_iter().take(1).collect(),
    };
    for name in &candidates {
        if let Some(sprite) = catalogue.sprites.find(name)? {
            return artwork::resolve_sprite_type(catalogue, sprite, 0, artwork::ICON_EDGE)
                .into_result()?
                .ok_or_else(|| format!("its sprite {name} is unavailable in the installation"));
        }
    }
    Err(selected.err().unwrap_or_else(|| format!("no sprite of {} is declared in the installation", candidates.join(", "))))
}

fn decision_title_text(catalogue: &Catalogue, inputs: &Inputs, frame: HourFrame<'_>, key: &str) -> Result<String, String> {
    let mut scopes = inputs.graph(true)?;
    for scope in &mut scopes.scopes {
        scope.random_seed_refusal = Some("decision title random seed was not recorded");
    }
    if let Some(target) = scopes.scopes.get_mut(1) {
        target.from = Link::Itself;
    }
    let text = catalogue.localisation.localise(key.as_bytes()).map_err(|why| why.to_string())?;
    let expander = ScriptedText::new_in(frame, &catalogue.localisation, catalogue.scripted_content()?, inputs.actor_slot.unwrap_or(0));
    catalogue::plain_text(&expander.expand_names_in(&text, &scopes, &[])?)
}

fn render(catalogue: &Catalogue, inputs: &Inputs, decision: bool, source: &str, items: &[content_reader_parser::parser::Item]) -> Result<effect_description::Description, String> {
    let scripted = catalogue.scripted_content()?;
    let content = scripted.effects()?;
    let frame = DecodedFrame {
        branch: 0,
        hour: 0,
        metrics: Default::default(),
        tables: std::array::from_fn(|_| TableView::Unavailable { reason: "exact effect-execution state between samples was not recorded".into() }.into()),
    };
    let actor = inputs.actor_slot.unwrap_or(0);
    let text = ScriptedText::new(&frame, &catalogue.localisation, scripted, actor);
    let events = script_events(catalogue);
    let lookup = |id: &[u8]| events.get(id);
    let context = effect_description::Context::new(&text, &catalogue.localisation, actor, source, content.view(Some(&lookup)));
    Ok(effect_description::describe(items, &context, &mut inputs.graph(decision)?))
}

fn body(notices: &mut Notices, catalogue: &Catalogue, item: &mut EventFeedItem, inputs: &Inputs, decision: bool, label: &str, items: &[content_reader_parser::parser::Item]) -> io::Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let token = item.token.as_deref().unwrap_or("unidentified content");
    let source = format!("{token} {label}");
    let rendered = match render(catalogue, inputs, decision, &source, items) {
        Ok(rendered) => rendered,
        Err(reason) => {
            for effect in items {
                let key = effect.key.as_deref().map(String::from_utf8_lossy).unwrap_or_default();
                notices.unavailable("scriptEffectDescription", "Authored effects", format!("{source} effect {key}: {reason}"), Some(1));
            }
            return Ok(());
        }
    };
    for (reason, count) in rendered.refusals {
        notices.unavailable("scriptEffectDescription", "Authored effects", format!("{source}: {reason}"), Some(u32::try_from(count).map_err(|_| io::Error::other("Effect refusal count exceeds u32"))?));
    }
    if !rendered.text.is_empty() {
        let description = item.description.get_or_insert_default();
        if !description.is_empty() {
            description.push_str("\n\n");
        }
        description.push_str(&format!("Authored {label}:\n{}", rendered.text));
    }
    Ok(())
}

fn effects(notices: &mut Notices, catalogue: &Catalogue, kind: EventFeedKind, item: &mut EventFeedItem, inputs: &Inputs) -> io::Result<()> {
    use EventFeedKind::*;
    let Some(token) = item.token.clone() else { return Ok(()) };
    let unavailable = |notices: &mut Notices, reason: String| notices.unavailable("scriptEffectDescription", "Authored effects", reason, Some(1));
    if presents_event(kind) {
        let events = script_events(catalogue);
        let event = match events.get(token.as_bytes()) {
            Ok(event) => event,
            Err(reason) => {
                unavailable(notices, format!("Event {token}: {reason}"));
                return Ok(());
            }
        };
        if let Some(reason) = &event.refused {
            unavailable(notices, format!("Event {token}: {reason}"));
        } else if kind != EventOptionSelected {
            body(notices, catalogue, item, inputs, false, "immediate effects", &event.immediate)?;
        } else {
            match inputs.option {
                Some(ordinal) if ordinal < 0 => body(notices, catalogue, item, inputs, false, "immediate option effects", &event.immediate)?,
                Some(ordinal) => match event.options.get(ordinal as usize) {
                    Some(effects) => body(notices, catalogue, item, inputs, false, &format!("option {ordinal} effects"), effects)?,
                    None => unavailable(notices, format!("Event {token} option {ordinal}: body absent from recorded content")),
                },
                None => unavailable(notices, format!("Event {token}: selected option ordinal was not recorded")),
            }
        }
        return Ok(());
    }
    let field = match kind {
        DecisionSelected | DecisionCompleted | DecisionCompletionEffect => "complete_effect",
        DecisionRemovalEffect => "remove_effect",
        DecisionCancellationEffect => "cancel_effect",
        DecisionTimedOut | DecisionTimeoutEffect => "timeout_effect",
        _ => return Ok(()),
    };
    let found = (|| {
        let content = catalogue.scripted_content()?.effects()?;
        let decisions = content.view(None).decisions.ok_or("decision effect database is unavailable from the recorded content stack")?;
        decisions.decision(token.as_bytes()).map(|decision| decision.cloned())?.ok_or_else(|| format!("decision {token} is absent from recorded content"))
    })();
    match found {
        Ok(decision) => match decision.refused {
            Some(reason) => unavailable(notices, format!("Decision {token} {field}: {reason}")),
            None => {
                let effects = match field {
                    "complete_effect" => &decision.complete_effect,
                    "remove_effect" => &decision.remove_effect,
                    "cancel_effect" => &decision.cancel_effect,
                    _ => &decision.timeout_effect,
                };
                body(notices, catalogue, item, inputs, true, field, effects)?;
            }
        },
        Err(reason) => unavailable(notices, format!("Decision {token} {field}: {reason}")),
    }
    Ok(())
}
