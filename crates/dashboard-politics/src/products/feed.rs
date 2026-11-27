
mod cards;
mod cause;
pub(crate) mod decision_state;
mod families;
mod fold;
pub mod index;
mod nuclear;
mod row;
mod tails;
mod text;
#[cfg(test)]
mod tests;
mod window;

pub use index::ScriptStatus;
pub use tails::{
    DecisionsTail, EventChoicesTail, EventFeedActivity, EventFeedActivityReply, EventFeedFoldRow, EventFeedKinds, EventFeedKindsReply, EventFeedLine, EventFeedMoreRow, EventFeedPageRow, EventFeedTail, EventFeedTailParams, EventFeedTailReply, ScriptEventKind, ScriptOccurrenceTailReply, ScriptPageRow, ScriptTailParams,
    decisions_dated_panel, event_choices_dated_panel, event_feed_dated_panel,
};
pub use decision_state::{
    COUNTRY_DECISION_STATE_REFUSED, DECISION_CONTAINER_NOT_RECORDED, DECISION_INSTANCE_MISMATCH,
    DECISION_NO_MISSION_LIFECYCLE, DECISION_OCCURRENCE_NOT_FOUND, DecisionLifecycleState,
    EVENT_FEED_HISTORY_INCOMPLETE, EVENT_FEED_HISTORY_NOT_RECORDED,
};
pub(crate) use row::FeedEvent;

use dashboard_base::{products::LoadContext, queries::country_ids::Names};
use dashboard_base::{Source, catalogue::{self, Catalogue}};
use dashboard_query::{
    Query,
    log::{self, FacetFilter},
};
use replay_data::rows::FeedEventRow;
use row::{Columns, Slot};
use serde::{Deserialize, Serialize};
use std::{io, sync::{Arc, atomic::AtomicBool}};
use text::{Notices, Presenter};
pub use text::warm_naming;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub(crate) const LOG: &str = "feed-events";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum EventFeedCategory {
    WarsPeace,
    Diplomacy,
    Politics,
    Milestones,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum EventFeedKind {
    WarStarted,
    WarDeclared,
    WarJoined,
    CivilWarWarTransferred,
    Annexed,
    WarEnded,
    Released,
    StateTransferred,
    PeaceConferenceStarted,
    Peace,
    Capitulated,
    CeasedToExist,
    FactionJoined,
    FactionLeft,
    BecameSubject,
    GuaranteeGranted,
    RulingPartyChanged,
    LawChanged,
    AdvisorHired,
    FocusCompleted,
    ProjectFinished,
    NukesIncreased,
    NuclearStrike,
    ScriptEventFired,
    NewsEventFired,
    EventOptionSelected,
    DecisionSelected,
    DecisionRemoved,
    DecisionCancelled,
    DecisionTimedOut,
    DecisionActivated,
    DecisionCompleted,
    DecisionReenabled,
    DecisionRemovalEffect,
    DecisionCancellationEffect,
    DecisionCompletionEffect,
    DecisionTimeoutEffect,
    DecisionDurationChanged,
    EventWindowPresented,
}

const CODES: [(u8, EventFeedKind); 26] = {
    use EventFeedKind::*;
    [
        (0, WarDeclared),
        (1, WarJoined),
        (2, CivilWarWarTransferred),
        (3, Annexed),
        (4, WarEnded),
        (5, Released),
        (6, StateTransferred),
        (7, PeaceConferenceStarted),
        (8, ScriptEventFired),
        (9, NewsEventFired),
        (10, EventOptionSelected),
        (11, DecisionSelected),
        (12, DecisionRemoved),
        (13, DecisionCancelled),
        (14, DecisionTimedOut),
        (15, DecisionActivated),
        (16, DecisionCompleted),
        (18, DecisionReenabled),
        (19, DecisionRemovalEffect),
        (20, DecisionCancellationEffect),
        (21, DecisionCompletionEffect),
        (22, DecisionTimeoutEffect),
        (23, DecisionDurationChanged),
        (FeedEventRow::CEASED_TO_EXIST, CeasedToExist),
        (FeedEventRow::GUARANTEE_GRANTED, GuaranteeGranted),
        (FeedEventRow::WINDOW_PRESENTED, EventWindowPresented),
    ]
};

impl EventFeedKind {
    pub const ALL: [EventFeedKind; 39] = {
        use EventFeedKind::*;
        [
            WarStarted, WarDeclared, WarJoined, CivilWarWarTransferred, Annexed, WarEnded, Released, StateTransferred, PeaceConferenceStarted, Peace, Capitulated, CeasedToExist, FactionJoined,
            FactionLeft, BecameSubject, GuaranteeGranted, RulingPartyChanged, LawChanged, AdvisorHired, FocusCompleted, ProjectFinished, NukesIncreased, NuclearStrike, ScriptEventFired,
            NewsEventFired, EventOptionSelected, DecisionSelected, DecisionRemoved, DecisionCancelled, DecisionTimedOut, DecisionActivated, DecisionCompleted, DecisionReenabled,
            DecisionRemovalEffect, DecisionCancellationEffect, DecisionCompletionEffect, DecisionTimeoutEffect, DecisionDurationChanged, EventWindowPresented,
        ]
    };

    pub fn of_index(index: u16) -> Option<Self> {
        Self::ALL.get(usize::from(index)).copied()
    }

    pub fn row_label(self) -> Option<&'static str> {
        use EventFeedKind::*;
        match self {
            WarStarted | WarDeclared => Some("War declared"),
            WarJoined => Some("Joined a war"),
            CivilWarWarTransferred => Some("Civil war transferred"),
            WarEnded | Peace => Some("Peace"),
            PeaceConferenceStarted => Some("Peace conference"),
            Capitulated => Some("Capitulated"),
            Annexed => Some("Annexed"),
            Released => Some("Released"),
            StateTransferred => Some("State transferred"),
            CeasedToExist => Some("Ceased to exist"),
            FactionJoined => Some("Joined faction"),
            FactionLeft => Some("Left faction"),
            BecameSubject => Some("Became subject"),
            GuaranteeGranted => Some("Guaranteed"),
            RulingPartyChanged => Some("New government"),
            LawChanged => Some("Law changed"),
            AdvisorHired => Some("Advisor hired"),
            FocusCompleted => Some("Focus completed"),
            ProjectFinished => Some("Project finished"),
            NukesIncreased => Some("Nuclear stockpile"),
            NuclearStrike => Some("Nuclear strike"),
            DecisionSelected => Some("Decision selected"),
            DecisionActivated => Some("Mission activated"),
            DecisionCompleted => Some("Decision completed"),
            DecisionTimedOut => Some("Decision timed out"),
            DecisionCancelled => Some("Decision cancelled"),
            DecisionRemoved => Some("Decision removed"),
            DecisionReenabled => Some("Decision reenabled"),
            DecisionDurationChanged => Some("Duration changed"),
            DecisionRemovalEffect | DecisionCancellationEffect | DecisionCompletionEffect | DecisionTimeoutEffect => Some("Decision effect"),
            ScriptEventFired | NewsEventFired | EventOptionSelected | EventWindowPresented => None,
        }
    }

    pub fn category(self) -> EventFeedCategory {
        use EventFeedKind::*;
        match self {
            WarStarted | WarDeclared | WarJoined | CivilWarWarTransferred | Annexed | WarEnded | Released | StateTransferred | PeaceConferenceStarted | Peace | Capitulated
            | CeasedToExist | NuclearStrike => EventFeedCategory::WarsPeace,
            FactionJoined | FactionLeft | BecameSubject | GuaranteeGranted => EventFeedCategory::Diplomacy,
            FocusCompleted | ProjectFinished | NukesIncreased => EventFeedCategory::Milestones,
            _ => EventFeedCategory::Politics,
        }
    }

    pub fn of_code(code: u8) -> Option<Self> {
        CODES.iter().find(|(held, _)| *held == code).map(|(_, kind)| *kind)
    }

    pub fn code(self) -> Option<u8> {
        CODES.iter().find(|(_, kind)| *kind == self).map(|(code, _)| *code)
    }

    pub fn scripted(self) -> bool {
        self >= EventFeedKind::ScriptEventFired
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedRow {
    pub id: String,
    pub hour: i32,
    #[doc = " The read supplying this fact; the head never shows it before this hour."]
    pub read_hour: i32,
    #[doc = " Country naming and flags use this hour: the event's, or a vanished country's last seen."]
    pub name_hour: i32,
    pub exact: bool,
    pub category: EventFeedCategory,
    pub kind: EventFeedKind,
    pub tags: Vec<String>,
    #[doc = " Native action roles, actor then target. Null keeps a role that names no country or that"]
    #[doc = " no link and holder witnessed."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub participants: Option<[Option<String>; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub participants_missing: Option<[Option<String>; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub owner_missing: Option<String>,
    pub item: Option<EventFeedItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional = nullable))]
    pub amount: Option<u32>,
    #[doc = " A native event's recorded cause; absent when the recording holds none."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub cause: Option<EventFeedCause>,
    #[doc = " A nuclear strike's target and outcome; absent for every other kind."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub strike: Option<EventFeedStrike>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub decision_state: Option<DecisionLifecycleState>,
    #[doc = " A fired event's window decision, or a presented window's recorded content; absent for"]
    #[doc = " every other kind."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub window: Option<EventFeedWindow>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum EventWindowDecision {
    Requested,
    AiControlled,
    HiddenOrAiControlled,
    NotViewingCountry,
    SenderExcluded,
    PlayerRestrictedNotMajor,
    PlayerRestrictedOptionTrigger,
    ExistingInstance,
    AlreadyOpen,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum EventWindowKind {
    Normal,
    News,
    Leader,
    Operative,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventWindowOption {
    #[doc = " The option's place in the live event's own list; null for the fallback OK button."]
    pub ordinal: Option<u32>,
    #[doc = " Null when the label's copy was refused; the button was still offered."]
    pub label: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventWindowPresented {
    pub kind: EventWindowKind,
    #[doc = " False when the user's popup settings hid the built window; null when not recorded."]
    pub shown: Option<bool>,
    #[doc = " Built again by an interface reload, with the same scope."]
    pub rebuilt: bool,
    pub title: Option<String>,
    pub description: Option<String>,
    #[doc = " The resolved picture key; null with `pictureHidden` when the window showed none."]
    pub picture: Option<String>,
    pub picture_hidden: bool,
    #[doc = " The offered options in button order."]
    pub options: Option<Vec<EventWindowOption>>,
    #[doc = " The event's options the window did not offer, from the recorded content; null when"]
    #[doc = " the offered options or the content's option list could not prove it."]
    pub not_offered: Option<Vec<EventFeedOption>>,
    #[doc = " Each field the recording does not hold, with why."]
    pub not_recorded: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedWindow {
    #[doc = " The native pending instance joining a firing to the window built for it."]
    pub instance: Option<u32>,
    #[doc = " On a fired event: whether this client asked for a window, or the gate that refused it."]
    pub decision: Option<EventWindowDecision>,
    #[doc = " On a presented window: what it showed."]
    pub presented: Option<EventWindowPresented>,
    #[doc = " Why `decision` or `presented` is null where the kind carries one."]
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedStrike {
    pub province: u32,
    #[doc = " None for a province without a state."]
    pub state: Option<u32>,
    pub building_levels: Option<f64>,
    #[doc = " The owner's war support change, in percent."]
    pub war_support: Option<f64>,
    pub units: Option<u32>,
    pub manpower_lost: Option<f64>,
    pub equipment_lost: Option<f64>,
    pub not_recorded: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedCause {
    #[doc = " The native operation the event happened in."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub how: Option<String>,
    #[doc = " The operation that one ran inside."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub within: Option<String>,
    #[doc = " The peace conference starting or applying, as its reference in decimal."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub conference: Option<String>,
    #[doc = " A state whose owner changed."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub state: Option<u32>,
    #[doc = " A join's ally."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub ally: Option<String>,
    #[doc = " A join's hostility reason."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub reason: Option<String>,
    #[doc = " The native operands that change what the event means; empty in a list row, read with the opened row."]
    pub notes: Vec<String>,
    #[doc = " A scripted event's selected native option ordinal, where recorded."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub selected_option: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub target: Option<EventFeedScriptTarget>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EventFeedScriptTarget {
    Country,
    State { id: Option<u32> },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum EventFeedGapField {
    Title,
    Description,
    Option,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedGap {
    pub field: EventFeedGapField,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub ordinal: Option<u32>,
    pub key: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedItem {
    pub token: Option<String>,
    pub name: Option<String>,
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional = nullable))]
    pub description: Option<String>,
    #[doc = " A scripted event's authored options in the game's order; absent for other kinds and when"]
    #[doc = " the event's content does not resolve."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub options: Option<Vec<EventFeedOption>>,
    #[doc = " Why `name` is null, where a script item's is."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub name_missing: Option<String>,
    #[doc = " Unresolved scripted-text placeholders in the name, description or options, in order per"]
    #[doc = " field, each with its original key and reason."]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional, as = "Option<_>"))]
    pub gaps: Vec<EventFeedGap>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedOption {
    pub ordinal: u32,
    #[doc = " In a list row only the chosen option's name; the opened row holds every option's."]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventFeedUnavailable {
    pub kind: String,
    pub label: String,
    pub reason: String,
    pub count: Option<u32>,
}

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub(crate) fn branch(text: &str) -> Result<(), &'static str> {
    text.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
}

pub(crate) fn resolve_content(source: &Source, context: &LoadContext) -> Result<Arc<Catalogue>, String> {
    context.content().and_then(|options| catalogue::catalogue(source, options)).map_err(|reason| reason.to_string())
}

pub(crate) fn content_from(resolved: &Result<Arc<Catalogue>, String>, notices: &mut Notices) -> Option<Arc<Catalogue>> {
    match resolved {
        Ok(catalogue) => Some(Arc::clone(catalogue)),
        Err(reason) => {
            notices.unavailable("contentStack", "Recorded content stack", reason.clone(), Some(1));
            None
        }
    }
}

fn content_refusals(presenter: &Presenter<'_>, notices: &mut Notices) -> io::Result<()> {
    if let Some(catalogue) = presenter.catalogue() {
        for (reason, count) in catalogue.refusal_entries() {
            let count = count.map(u32::try_from).transpose().map_err(|_| invalid("Content refusal count exceeds u32"))?;
            notices.unavailable("contentPresentation", "Content presentation", reason, count);
        }
    }
    Ok(())
}

fn tag(names: &Names, slot: Slot, hour: i32) -> Option<String> {
    match slot {
        Slot::Of(slot) => names.slot_tag(slot, hour).map(str::to_owned),
        Slot::None | Slot::NotRecorded => None,
    }
}

pub(crate) fn present_all(query: &Query<'_>, scanned: &index::Scanned, events: &[&FeedEvent], names: &Names, presenter: &Presenter<'_>, notices: &mut Notices, cancelled: &AtomicBool) -> io::Result<Vec<Option<EventFeedRow>>> {
    let states = scanned.decision_states(query, events)?;
    let mut order: Vec<usize> = (0..events.len()).collect();
    order.sort_by_key(|&at| events[at].hour);
    let (request, mut held) = scanned.held_rows(presenter.content(), presenter.effects(), events);
    let mut made = Vec::new();
    for &at in &order {
        dashboard_text::metadata::check_cancel(cancelled)?;
        if held[at].is_none() {
            let (presented, own) = catalogue::capture(|| {
                let mut said = Notices::default();
                present(events[at], names, presenter, &mut said).map(|row| (row, said))
            });
            let (row, said) = presented?;
            let kept: index::HeldRow = Arc::new((row, said, own));
            made.push((events[at], Arc::clone(&kept)));
            held[at] = Some(kept);
        }
    }
    scanned.keep_rows(presenter.content(), presenter.effects(), request, made);
    let mut out: Vec<Option<EventFeedRow>> = vec![None; events.len()];
    for &at in &order {
        let Some(kept) = held[at].as_ref() else { continue };
        catalogue::credit(&kept.2);
        notices.absorb(&kept.1);
        let mut presented = kept.0.clone();
        if let Some(row) = presented.as_mut() {
            row.decision_state = states[at].clone();
        }
        out[at] = presented;
    }
    Ok(out)
}

pub(crate) fn present(event: &FeedEvent, names: &Names, presenter: &Presenter<'_>, notices: &mut Notices) -> io::Result<Option<EventFeedRow>> {
    if event.code == FeedEventRow::WINDOWS_UNRECORDED {
        notices.unavailable("eventWindows", "Event windows", format!("From read {}: {}", event.read_hour, String::from_utf8_lossy(&event.failure)), Some(1));
        return Ok(None);
    }
    let Some(kind) = EventFeedKind::of_code(event.code) else { return Ok(None) };
    let id = event.id();
    if kind == EventFeedKind::EventWindowPresented {
        return window::presented(event, id, names, presenter, notices);
    }
    if event.derived() {
        return Ok(Some(derived(event, kind, id, names, notices)));
    }
    let participants = [event.actor, event.target].map(|party| tag(names, party.slot, event.hour));
    for ((role, named), party) in ["actor", "target"].into_iter().zip(&participants).zip([event.actor, event.target]) {
        if named.is_none() && !(kind.scripted() && party.slot == Slot::None) {
            notices.sampled("nativeEventCountry", "Native event country", format!("The {role} of event {} at {} has no exact country-reference and published-link witness.", event.sequence, event.hour), 1);
        }
    }
    let ally = event.cause.and_then(|cause| cause.ally).map(|ally| tag(names, ally.slot, event.hour));
    if ally == Some(None) {
        notices.sampled("nativeEventCountry", "Native event country", format!("The ally of event {} at {} has no exact country-reference and published-link witness.", event.sequence, event.hour), 1);
    }
    let item = if kind.scripted() {
        let token = match &event.token {
            None => {
                notices.sampled("scriptEventToken", "Script content identity", format!("Occurrence {} content token was not captured.", event.sequence), 1);
                None
            }
            Some(bytes) if bytes.is_empty() => event.native_event.map(|(_, id)| id.to_string()),
            Some(bytes) => match std::str::from_utf8(bytes) {
                Ok(token) => Some(token.to_owned()),
                Err(_) => {
                    notices.sampled("scriptEventToken", "Script content identity", format!("Occurrence {} has a non-UTF-8 content token.", event.sequence), 1);
                    None
                }
            },
        };
        match &event.scopes {
            None => notices.sampled("scriptEventScope", "Bound script scope", format!("Occurrence {} scope graph was not fully captured.", event.sequence), 1),
            Some(scopes) => {
                for (at, scope) in scopes.iter().enumerate() {
                    for (missing, operand) in [(scope.combat_context_unrecorded, "combat context"), (scope.target_storage_unrecorded, "saved targets"), (scope.tooltip_storage_unrecorded, "tooltip targets")] {
                        if missing {
                            notices.sampled("scriptEventScope", "Bound script scope", format!("Occurrence {} scope {at}: {operand} operands were not captured.", event.sequence), 1);
                        }
                    }
                }
            }
        }
        Some(presenter.item(event, kind, token, notices)?)
    } else {
        None
    };
    Ok(Some(EventFeedRow {
        id,
        hour: event.hour,
        read_hour: event.read_hour,
        name_hour: event.hour,
        exact: true,
        category: kind.category(),
        kind,
        tags: participants.iter().flatten().cloned().collect(),
        participants: Some(participants),
        participants_missing: None,
        owner_missing: None,
        item,
        amount: None,
        cause: cause::words(event, ally.flatten()),
        strike: None,
        decision_state: None,
        window: window::decided(event, kind),
    }))
}

fn derived(event: &FeedEvent, kind: EventFeedKind, id: String, names: &Names, notices: &mut Notices) -> EventFeedRow {
    let row = |hour: i32, name_hour: i32, exact: bool, participants: [Option<String>; 2]| EventFeedRow {
        id: id.clone(),
        hour,
        read_hour: event.read_hour,
        name_hour,
        exact,
        category: kind.category(),
        kind,
        tags: participants.iter().flatten().cloned().collect(),
        participants: Some(participants),
        participants_missing: None,
        owner_missing: None,
        item: None,
        amount: None,
        cause: None,
        strike: None,
        decision_state: None,
        window: None,
    };
    match kind {
        EventFeedKind::CeasedToExist => {
            let named = event.dated.unwrap_or(event.read_hour);
            let country = tag(names, event.actor.slot, named);
            if country.is_none() {
                notices.sampled("ceasedToExist", "Country existence", format!("The country that ceased to exist at {} holds no tag at its last read with states.", event.read_hour), 1);
            }
            row(event.read_hour, named, false, [country, None])
        }
        _ => {
            let at_read = [event.actor.slot, event.target.slot].map(|slot| tag(names, slot, event.read_hour));
            if at_read.iter().any(Option::is_none) {
                notices.sampled("guaranteeGranted", "Guarantees", format!("A guarantee read at {} names a country that holds no tag then.", event.read_hour), 1);
            }
            let start = event.dated.unwrap_or(event.read_hour);
            let exact = at_read.iter().all(Option::is_some) && [event.actor.slot, event.target.slot].map(|slot| tag(names, slot, start)) == at_read;
            let hour = if exact { start } else { event.read_hour };
            row(hour, hour, exact, at_read)
        }
    }
}

pub(crate) fn annexations(query: &Query<'_>) -> io::Result<Vec<super::politics::civil_wars::Annexation>> {
    let declared = dashboard_declare::log(LOG).ok_or_else(|| invalid("The feed's log is not declared"))?;
    let reader = log::Reader::new(query, declared)?;
    let annexed = EventFeedKind::Annexed.code().map(i64::from).ok_or_else(|| invalid("An annexation has no feed code"))?;
    let selection = reader.select(&[FacetFilter { name: "kind".into(), values: vec![annexed], ..Default::default() }], query.timeline.first, query.timeline.last)?;
    let (columns, names) = (Columns::new()?, Names::new(&query.holders));
    let mut out = Vec::new();
    let mut cursor = 0;
    loop {
        let page = reader.page(&selection.patterns, selection.groups.len(), query.timeline.first, query.timeline.last, cursor, dashboard_query::series::PAGE_LIMIT)?;
        for paged in &page.rows {
            let event = columns.read(&paged.row)?;
            out.push(super::politics::civil_wars::Annexation {
                hour: event.hour,
                read_hour: event.read_hour,
                annexer: tag(&names, event.actor.slot, event.hour),
                annexed: tag(&names, event.target.slot, event.hour),
                civil_war: cause::civil_war(&event),
            });
        }
        match page.cursor() {
            Some(next) => cursor = next,
            None => return Ok(out),
        }
    }
}
