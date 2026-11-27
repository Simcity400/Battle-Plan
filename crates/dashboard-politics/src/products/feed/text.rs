
use super::{EventFeedGap, EventFeedGapField, EventFeedItem, EventFeedKind, EventFeedOption, EventFeedUnavailable, row::FeedEvent, row::Slot};
use dashboard_base::{catalogue::{self, Catalogue}, products::artwork};
use dashboard_effects::{ScriptedEffects, effect_description::{self, DecisionNameKeys}};
use dashboard_text::scripted_text::{EventObject, EventScope, EventScopes, ExpandedText, Link, SCRIPTED_GAP, ScopeName, ScriptedContent, ScriptedText, expand_dates_in};
use content_reader_events::events::Event;
use content_reader_layers::content::diagnostics::Diagnostics;
use content_reader_politics::decisions::Decision;
use dashboard_query::Query;
use replay_lanes::{at::Read, dated::Dated, lanes::Keys};
use replay_recording::columnar_frames::Recording;
use replay_data::rows::{CountryAppearanceRow, CountryRow, PoliticalPartyRow, PoliticsRow, ReservedCountryNamingRow, StateRow, TableId, mask, tables};
use replay_frame::frame::{DecodedFrame, Discovery, HourFrame, Metrics, SharedView, TableView, discovering_fields, not_selected};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    hash::Hash,
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
    discard: bool,
    list: Vec<EventFeedUnavailable>,
    keys: HashMap<(String, String, Option<String>, bool), (usize, Vec<String>)>,
}

impl Notices {
    pub(super) fn discarded() -> Self {
        Self { discard: true, ..Self::default() }
    }

    pub fn unavailable(&mut self, kind: &str, label: &str, reason: impl Into<String>, count: Option<u32>) {
        if self.discard { return; }
        let reason = reason.into();
        self.merge((kind.to_owned(), label.to_owned(), Some(reason.clone()), count.is_some()), reason, count, Vec::new());
    }

    pub fn sampled(&mut self, kind: &str, label: &str, reason: impl Into<String>, count: u32) {
        if self.discard { return; }
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

    pub fn absorb(&mut self, other: &Notices) {
        let mut said: Vec<_> = other.keys.iter().collect();
        said.sort_by_key(|(_, (index, _))| *index);
        for (key, (index, samples)) in said {
            let entry = &other.list[*index];
            match samples.split_first() {
                None => self.merge(key.clone(), entry.reason.clone(), entry.count, Vec::new()),
                Some((first, rest)) => {
                    self.merge(key.clone(), first.clone(), entry.count, vec![first.clone()]);
                    if !rest.is_empty() {
                        self.merge(key.clone(), first.clone(), Some(0), rest.to_vec());
                    }
                }
            }
        }
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

#[derive(Clone, PartialEq, Eq, Hash)]
struct DecisionTarget {
    country: Option<u32>,
    numeric: Option<i32>,
    slot: Option<u32>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
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
    hour: i32,
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
            hour: event.hour,
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
        if self.missing || self.scopes.is_empty() {
            return Err("event execution scope graph was not recorded".into());
        }
        let attempted = self.scopes.iter().any(|scope| scope.attempted);
        let names = self
            .scopes
            .iter()
            .map(|scope| {
                if let Some(kinds) = &scope.opaque {
                    ScopeName::Unproven(format!("it carries {kinds}"))
                } else if !attempted && scope.unsupported {
                    ScopeName::Unproven("it carries unsupported saved or tooltip-target operands".into())
                } else if scope.slot.is_none() {
                    ScopeName::Unproven(format!("its country reference {} has no exact occurrence-hour canonical slot", scope.reference))
                } else if !attempted {
                    ScopeName::Hour
                } else if let Some(names) = &scope.names {
                    ScopeName::Recorded(names.clone())
                } else if scope.reference as i32 <= 0 {
                    ScopeName::Unproven("it holds no country".into())
                } else {
                    ScopeName::Unproven("its country's names were not recorded: unreadable, too long or possibly mid-rename".into())
                }
            })
            .collect();
        Ok((self.scope_graph(), names))
    }

    fn decision_title_graph(
        &self,
        owner: u32,
        definition: &content_reader_politics::decisions::Decision,
        state_selection_reason: Option<&str>,
    ) -> (EventScopes, Vec<ScopeName>) {
        let mut root = EventScope::of(EventObject::Country(owner));
        root.random_seed_refusal = Some("decision title random seed was not recorded");
        let mut scopes = vec![root];
        let mut names = vec![ScopeName::Hour];
        let push_unproven = |scopes: &mut Vec<EventScope>, names: &mut Vec<ScopeName>, reason: &str| {
            let mut unproven = EventScope::of(EventObject::Country(0));
            unproven.random_seed_refusal = Some("decision title random seed was not recorded");
            scopes.push(unproven);
            names.push(ScopeName::Unproven(reason.to_owned()));
            scopes.len() - 1
        };
        if let Some(target) = self.target.as_ref() {
            if target.country.is_some_and(|country| country as i32 > 0) {
                match target.slot {
                    Some(target_slot) => {
                        let mut from = EventScope::of(EventObject::Country(target_slot));
                        from.random_seed_refusal = Some("decision title random seed was not recorded");
                        scopes[0].from = Link::Scope(1);
                        scopes.push(from);
                        names.push(ScopeName::Hour);
                        if let Some(reason) = state_selection_reason {
                            let highlighted = push_unproven(&mut scopes, &mut names, reason);
                            scopes[1].from = Link::Scope(highlighted);
                        }
                    }
                    None => {
                        let from = push_unproven(&mut scopes, &mut names, "targeted decision country has no exact occurrence-hour slot");
                        scopes[0].from = Link::Scope(from);
                    }
                }
            } else if let Some(numeric) = target.numeric.filter(|&numeric| numeric > 0) {
                let mut from = EventScope::of(EventObject::State(numeric as u32));
                from.random_seed_refusal = Some("decision title random seed was not recorded");
                scopes[0].from = Link::Scope(1);
                scopes.push(from);
                names.push(ScopeName::Hour);
            } else {
                let from = push_unproven(&mut scopes, &mut names, "targeted decision numeric selector was not recorded");
                scopes[0].from = Link::Scope(from);
            }
        } else if let Some(reason) = state_selection_reason {
            let from = push_unproven(&mut scopes, &mut names, reason);
            scopes[0].from = Link::Scope(from);
        } else if definition.targeted {
            let from = push_unproven(&mut scopes, &mut names, "targeted decision selector was not recorded");
            scopes[0].from = Link::Scope(from);
        }
        (EventScopes::new(scopes), names)
    }
}

#[cfg(test)]
#[test]
fn the_event_index_releases_the_recording_with_its_catalogue() {
    use dashboard_base::test_fixtures::visual_mods::Machine;

    let machine = Machine::new("event_index_release");
    let (source, options) = machine.open();
    let replay = Arc::downgrade(&source.replay());
    let catalogue = catalogue::catalogue(&source, &options).unwrap();
    assert!(std::ptr::eq(catalogue.event_index(), catalogue.event_index()));
    drop(source);
    assert!(replay.upgrade().is_some());
    drop(catalogue);
    assert!(replay.upgrade().is_none());
}

fn inline_gaps(expanded: &ExpandedText) -> String {
    if expanded.gaps.is_empty() {
        return expanded.text.clone();
    }
    let mut out = String::with_capacity(expanded.text.len());
    let mut gaps = expanded.gaps.iter();
    for ch in expanded.text.chars() {
        if ch == SCRIPTED_GAP {
            match gaps.next() {
                Some(gap) => out.push_str(&format!("[not recorded: {}: {}]", gap.key, gap.reason)),
                None => out.push_str("[not recorded]"),
            }
        } else {
            out.push(ch);
        }
    }
    out
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
    catalogue: Option<&'q Arc<Catalogue>>,
    frames: RefCell<Frames>,
    kept: Arc<Mutex<KeptNaming>>,
    effects: bool,
    index: Option<IndexNames<'q>>,
}

type Held<K, V> = RefCell<HashMap<K, V>>;
type DecisionDefinition<'a> = (&'a ScriptedContent, &'a Decision);
type EventDefinition = (Arc<Event>, Result<Option<Arc<[u8]>>, String>);
type HeldExpansion = (Result<ExpandedText, String>, Vec<(TableId, Option<u64>)>);
type ScopedName = (String, String, i32, Option<u32>, Box<[TableId]>, NamingScope);

#[derive(PartialEq, Eq, Hash)]
enum NamingScope {
    Decision(Option<DecisionTarget>),
    Event(bool, Vec<ScopeInput>),
}

#[derive(Default)]
struct IndexNames<'a> {
    decisions: Held<String, Result<DecisionDefinition<'a>, String>>,
    events: Held<String, Result<EventDefinition, String>>,
    decision_keys: Held<String, DecisionNameKeys>,
    localisation: Held<(String, String), Option<Arc<str>>>,
    scoped: Held<ScopedName, HeldExpansion>,
}

fn remember<K: Eq + Hash, V: Clone>(held: &Held<K, V>, key: K, load: impl FnOnce() -> V) -> V {
    held.borrow_mut().entry(key).or_insert_with(load).clone()
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

struct Art<'a, 'q> {
    catalogue: &'q Arc<Catalogue>,
    kept: &'a Mutex<KeptNaming>,
    effects: bool,
    index: Option<&'a IndexNames<'q>>,
    exact: &'a [TableId],
}

impl Art<'_, '_> {
    fn localised(&self, token: &str, key: &str) -> Option<String> {
        let load = || self.catalogue.localised(key, catalogue::View::Plain);
        self.index.map_or_else(load, |index| remember(&index.localisation, (token.to_owned(), key.to_owned()), || load().map(Arc::from)).map(|text| text.to_string()))
    }

    fn expand(&self, token: &str, key: &str, inputs: &Inputs, decision: bool, expand: impl FnOnce() -> Result<ExpandedText, String>) -> Result<ExpandedText, String> {
        let Some(index) = self.index else { return expand() };
        let scope = if decision { NamingScope::Decision(inputs.target.clone()) } else { NamingScope::Event(inputs.missing, inputs.scopes.clone()) };
        let key = (token.to_owned(), key.to_owned(), inputs.hour, inputs.actor_slot, self.exact.into(), scope);
        let (expanded, missed) = remember(&index.scoped, key, || Discovery::current().run(expand));
        Discovery::credit(missed);
        expanded
    }

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
    pub fn new(query: &'q Query<'q>, catalogue: Option<&'q Arc<Catalogue>>, kept: Arc<Mutex<KeptNaming>>) -> Self {
        Self { query, catalogue, frames: RefCell::default(), kept, effects: true, index: None }
    }

    pub fn without_effects(self) -> Self {
        Self { effects: false, ..self }
    }

    pub(super) fn for_index(self) -> Self {
        Self { effects: false, index: Some(IndexNames::default()), ..self }
    }

    pub fn catalogue(&self) -> Option<&Catalogue> {
        self.catalogue.map(Arc::as_ref)
    }

    pub fn content(&self) -> Option<&Arc<Catalogue>> {
        self.catalogue
    }

    pub fn effects(&self) -> bool {
        self.effects
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
        draw_picture(notices, &Art { catalogue: self.catalogue?, kept: &self.kept, effects: self.effects, index: self.index.as_ref(), exact: &[] }, token, key)
    }

    pub fn item(&self, event: &FeedEvent, kind: EventFeedKind, token: Option<String>, notices: &mut Notices) -> io::Result<EventFeedItem> {
        let mut item = EventFeedItem { token, ..Default::default() };
        let Some(shown) = self.catalogue else {
            item.name_missing = Some("the recorded content stack could not be read".into());
            return Ok(item);
        };
        let present = |frame: Result<&DecodedFrame, String>, notices: &mut Notices, exact: &[TableId]| -> io::Result<EventFeedItem> {
            let mut item = item.clone();
            let inputs = Inputs::of(event, frame.as_ref().ok().copied());
            presentation(notices, &Art { catalogue: shown, kept: &self.kept, effects: self.effects, index: self.index.as_ref(), exact }, kind, &mut item, &inputs, frame.map(HourFrame::from))?;
            Ok(item)
        };
        let dated = match self.frame(event.hour)? {
            Ok(dated) => dated,
            Err(why) => return present(Err(why), notices, &[]),
        };
        let before = notices.clone();
        let (presented, missed) = discovering_fields(|| present(Ok(&dated), notices, &[]));
        let mut missed: Vec<TableId> = missed.into_iter().map(|(table, _)| table).filter(|table| NAMING.contains(table)).collect();
        missed.sort_unstable();
        missed.dedup();
        if missed.is_empty() {
            return presented;
        }
        *notices = before;
        present(Ok(&self.exact(&dated, &missed)?), notices, &missed)
    }
}

fn presentation(notices: &mut Notices, art: &Art<'_, '_>, kind: EventFeedKind, item: &mut EventFeedItem, inputs: &Inputs, frame: Result<HourFrame<'_>, String>) -> io::Result<()> {
    let catalogue: &Catalogue = art.catalogue;
    let indexing = art.index.is_some();
    if !presents_event(kind) {
        if names_decision(kind) {
            let token = item.token.clone().unwrap_or_else(|| "unidentified decision".to_owned());
            match frame.clone().and_then(|frame| decision_title(art, inputs, frame, &token)) {
                Ok(expanded) => {
                    item.name = Some(expanded.text);
                    for gap in expanded.gaps {
                        notices.unavailable("decisionPresentation", "Decision presentation", format!("Decision {token}: [{}]: {}", gap.key, gap.reason), Some(1));
                        item.gaps.push(EventFeedGap { field: EventFeedGapField::Title, ordinal: None, key: gap.key, reason: gap.reason });
                    }
                }
                Err(reason) => {
                    notices.unavailable("decisionPresentation", "Decision presentation", format!("Decision {token}: title unavailable: {reason}"), Some(1));
                    item.name_missing = Some(reason);
                }
            }
            if !indexing {
                let icon = art.drawn(("decision", &token), || catalogue::draw(|| format!("decision {token} icon"), catalogue, |catalogue| decision_icon(catalogue, &token)));
                match icon {
                    Ok(url) => item.icon = Some(url),
                    Err(reason) => notices.unavailable("decisionIcon", "Decision icon", format!("Decision {token}: {reason}"), Some(1)),
                }
            }
        }
        return if art.effects { effects(notices, catalogue, kind, item, inputs) } else { Ok(()) };
    }
    let Some(token) = item.token.clone() else {
        item.name_missing = Some("the event's identity was not recorded".into());
        return Ok(());
    };
    let lookup = || {
        let event = catalogue.event_index().get(token.as_bytes())?;
        let title = if event.refused.is_none() { event.sole_title_key().map(|key| key.map(Arc::from)) } else { Ok(None) };
        Ok((event, title))
    };
    let (event, title) = match art.index.map_or_else(lookup, |index| remember(&index.events, token.clone(), lookup)) {
        Ok((event, title)) if event.refused.is_none() => (event, title),
        Ok((event, _)) => {
            let reason = event.refused.clone().unwrap_or_default();
            item.name_missing = Some(reason.clone());
            notices.unavailable("scriptEventContent", "Event content", reason, Some(1));
            return if art.effects { effects(notices, catalogue, kind, item, inputs) } else { Ok(()) };
        }
        Err(reason) => {
            item.name_missing = Some(reason.clone());
            notices.unavailable("scriptEventContent", "Event content", reason, Some(1));
            return if art.effects { effects(notices, catalogue, kind, item, inputs) } else { Ok(()) };
        }
    };
    let localise = |key: &[u8]| -> Result<(String, ExpandedText), String> {
        let key = std::str::from_utf8(key).map_err(|_| format!("Event {token} localisation key is not UTF-8"))?;
        let text = art.localised(&token, key).ok_or_else(|| format!("Event {token}: {key} needs unavailable historical localisation operands"))?;
        if !text.contains('[') {
            return Ok((key.to_owned(), ExpandedText { text, gaps: Vec::new() }));
        }
        let expanded = art.expand(&token, key, inputs, false, || scoped_text(catalogue, inputs, frame.clone(), key))?;
        Ok((key.to_owned(), expanded))
    };
    let record_gaps = |notices: &mut Notices, item: &mut EventFeedItem, key: &str, field: EventFeedGapField, ordinal: Option<u32>, gaps: Vec<dashboard_text::scripted_text::ScriptedGap>| {
        for gap in gaps {
            notices.unavailable("scriptEventText", "Scripted event text", format!("Event {token}: {key}: [{}]: {}", gap.key, gap.reason), Some(1));
            item.gaps.push(EventFeedGap { field, ordinal, key: gap.key, reason: gap.reason });
        }
    };
    let title = ("title", EventFeedGapField::Title, title.as_ref().map(|key| key.as_deref()).map_err(Clone::clone));
    let description = (!indexing).then(|| ("body", EventFeedGapField::Description, event.sole_description_key()));
    for (field, gap_field, keys) in std::iter::once(title).chain(description) {
        match keys.and_then(|key| key.map(&localise).transpose()) {
            Ok(Some((key, expanded))) if field == "title" => {
                item.name = Some(expanded.text);
                record_gaps(notices, item, &key, gap_field, None, expanded.gaps);
            }
            Ok(Some((key, expanded))) => {
                item.description = Some(expanded.text);
                record_gaps(notices, item, &key, gap_field, None, expanded.gaps);
            }
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
    if !indexing && event.picture.is_empty() {
        notices.unavailable("scriptEventPicture", "Event picture", format!("Event {token}: default picture selection has no recorded presentation result"), Some(1));
    } else if !indexing && let Ok(picture) = std::str::from_utf8(&event.picture) {
        item.icon = draw_picture(notices, art, &token, picture);
    } else if !indexing {
        notices.unavailable("scriptEventPicture", "Event picture", format!("Event {token}: picture key is not UTF-8"), Some(1));
    }
    let mut options = Vec::new();
    let mut authored = Vec::new();
    let selected = inputs.option.and_then(|ordinal| usize::try_from(ordinal).ok());
    let (skip, take) = if indexing { (selected.unwrap_or(event.option_names.len()), 1) } else { (0, event.option_names.len()) };
    for (ordinal, name) in event.option_names.iter().enumerate().skip(skip).take(take) {
        let ordinal_u32 = u32::try_from(ordinal).map_err(|_| io::Error::other("Too many authored event options"))?;
        let resolved = localise(name);
        let display = match resolved {
            Ok((key, expanded)) => {
                let inline = inline_gaps(&expanded);
                authored.push(EventFeedOption { ordinal: ordinal_u32, name: Some(expanded.text) });
                record_gaps(notices, item, &key, EventFeedGapField::Option, Some(ordinal_u32), expanded.gaps);
                inline
            }
            Err(reason) => {
                authored.push(EventFeedOption { ordinal: ordinal_u32, name: None });
                notices.unavailable("scriptEventOptionName", "Event option name", reason, Some(1));
                String::from_utf8_lossy(name).into_owned()
            }
        };
        if indexing { continue; }
        options.push(format!("{ordinal}: {display}"));
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
    item.options = Some(authored);
    if !options.is_empty() {
        let body = item.description.get_or_insert_default();
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str("Authored options (native ordinals):\n");
        body.push_str(&options.join("\n"));
    }
    if art.effects { effects(notices, catalogue, kind, item, inputs) } else { Ok(()) }
}

fn draw_picture(notices: &mut Notices, art: &Art<'_, '_>, token: &str, picture: &str) -> Option<String> {
    if art.index.is_some() {
        return None;
    }
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

fn scoped_text(catalogue: &Catalogue, inputs: &Inputs, frame: Result<HourFrame<'_>, String>, key: &str) -> Result<ExpandedText, String> {
    let raw = catalogue.localisation.localise(key.as_bytes()).map_err(|why| why.to_string())?;
    let ready = frame
        .and_then(|frame| Ok((frame, catalogue.scripted_content()?)))
        .and_then(|(frame, content)| Ok((frame, content, inputs.title_graph()?)));
    let expanded = match ready {
        Ok((frame, content, (scopes, names))) => {
            let expander = ScriptedText::new_in(frame, &catalogue.localisation, content, inputs.actor_slot.unwrap_or(0));
            expander.expand_names_in(&raw, &scopes, &names)
        }
        Err(why) => expand_dates_in(&raw, inputs.hour, &catalogue.localisation, &why),
    };
    let text = catalogue::plain_text(&expanded.text)?;
    Ok(ExpandedText { text, gaps: expanded.gaps })
}

fn decision_definition<'a>(catalogue: &'a Catalogue, token: &str) -> Result<DecisionDefinition<'a>, String> {
    let content = catalogue.scripted_content()?;
    let effects = content.effects()?;
    let decisions = effects.view(None).decisions.ok_or("decision database is unavailable from the recorded content stack")?;
    let definition = decisions.decision(token.as_bytes())?.ok_or_else(|| format!("decision {token} is absent from recorded content"))?;
    Ok((content, definition))
}

fn decision_title(art: &Art<'_, '_>, inputs: &Inputs, frame: HourFrame<'_>, token: &str) -> Result<ExpandedText, String> {
    let catalogue = art.catalogue;
    let lookup = || decision_definition(catalogue, token);
    let (content, definition) = art.index.map_or_else(lookup, |index| remember(&index.decisions, token.to_owned(), lookup))?;
    if let Some(reason) = &definition.refused {
        return Err(reason.clone());
    }
    let owner = inputs.actor_slot.ok_or("decision owner has no exact occurrence-hour canonical slot")?;
    let text = ScriptedText::new_in(frame, &catalogue.localisation, content, owner);
    let mut keys = art.index.map(|index| index.decision_keys.borrow_mut());
    let held = keys.as_mut().map(|keys| keys.entry(token.to_owned()).or_default());
    let key = effect_description::decision_name_key_at(frame, text.recorded(), &catalogue.localisation, definition, owner, text.country_tag(owner)?, held)?;
    let name = art.localised(token, &key).ok_or_else(|| format!("decision title {key} is absent from recorded content"))?;
    if !name.contains('[') {
        return Ok(ExpandedText { text: name, gaps: Vec::new() });
    }
    art.expand(token, &key, inputs, true, || expand_decision_title(catalogue, inputs, &text, definition, owner, &key))
}

fn expand_decision_title(catalogue: &Catalogue, inputs: &Inputs, text: &ScriptedText<'_>, definition: &Decision, owner: u32, key: &str) -> Result<ExpandedText, String> {
    let raw = catalogue.localisation.localise(key.as_bytes()).map_err(|why| why.to_string())?;
    let state_selection_reason = match definition.name_selects_state(|body| text.trigger_world()?.has_children(body)) {
        Ok(true) => Some("it names a highlighted state the recording does not hold".to_owned()),
        Ok(false) => None,
        Err(why) => Some(why),
    };
    let (scopes, names) = inputs.decision_title_graph(owner, definition, state_selection_reason.as_deref());
    let expanded = text.expand_names_in(&raw, &scopes, &names);
    let plain = catalogue::plain_text(&expanded.text)?;
    Ok(ExpandedText { text: plain, gaps: expanded.gaps })
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
    let events = catalogue.event_index();
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
        let events = catalogue.event_index();
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
