//! One world feed composed from the topic products' witnessed histories.
//! An exact native date remains separate from the read that revealed it. No
//! cause is inferred from losing owned states, and no initial snapshot is
//! promoted to a change. Prepared readers compose backward cursor pages.

use super::{
    LoadContext, Product, artwork, country_groups, country_nukes,
    idea_format::{self, LocalisedText, ModifierFormat},
    idea_history, national_focus, politics_content, special_projects, subjects,
};
#[cfg(test)]
use super::{MAX_PRODUCT_REPLY_BYTES, party_history};
use crate::{
    Source,
    catalogue::{self, Catalogue},
    metadata::branch_bounds,
};
#[cfg(test)]
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::collections::VecDeque;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

mod diplomacy;

mod decision_roster;
mod native;
mod script_effects;

mod nuclear;
pub(crate) mod prepared;

pub struct EventFeed;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventFeedParams {
    pub branch_id: String,
    /// Both null discovers the bounded, event-bearing windows of the branch.
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub enum EventFeedCategory {
    WarsPeace,
    Diplomacy,
    Politics,
    Milestones,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
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
    DecisionRosterRebuilt,
    ScriptStateObserved,
}
impl EventFeedKind {
    fn category(self) -> EventFeedCategory {
        use EventFeedKind::*;
        match self {
            WarStarted
            | WarDeclared
            | WarJoined
            | CivilWarWarTransferred
            | Annexed
            | WarEnded
            | Released
            | StateTransferred
            | PeaceConferenceStarted
            | Peace
            | Capitulated
            | CeasedToExist
            | NuclearStrike => EventFeedCategory::WarsPeace,
            FactionJoined | FactionLeft | BecameSubject | GuaranteeGranted => {
                EventFeedCategory::Diplomacy
            }
            RulingPartyChanged
            | LawChanged
            | AdvisorHired
            | ScriptEventFired
            | NewsEventFired
            | EventOptionSelected
            | DecisionSelected
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
            | DecisionRosterRebuilt
            | ScriptStateObserved => EventFeedCategory::Politics,
            FocusCompleted | ProjectFinished | NukesIncreased => EventFeedCategory::Milestones,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventFeedRow {
    pub id: String,
    pub hour: i32,
    /// The read supplying this fact; Live must not expose it before this hour.
    pub read_hour: i32,
    /// Country naming/flags use the event hour; a vanished country's last seen hour.
    pub name_hour: i32,
    pub exact: bool,
    pub category: EventFeedCategory,
    pub kind: EventFeedKind,
    pub tags: Vec<String>,
    /// Native action roles, actor then target. Null retains an unidentified role;
    /// tags contains only the exact occurrence-hour identities proved here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub participants: Option<[Option<String>; 2]>,
    pub item: Option<u32>,
    pub amount: Option<u32>,
    /// A native event's recorded cause; absent when the recording holds none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cause: Option<EventFeedCause>,
    /// A nuclear strike's target and outcome; participants are its launcher
    /// and the owner of the struck state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub strike: Option<EventFeedStrike>,
}

/// What caused a native event, in words, from the operations the collector
/// observed running when it happened. Each absent fact was not observed.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventFeedCause {
    /// The native operation the event happened in.
    pub how: Option<String>,
    /// The operation that one ran inside.
    pub within: Option<String>,
    /// The peace conference starting or applying, as its PeaceConferences
    /// reference in decimal.
    pub conference: Option<String>,
    /// A state whose owner changed.
    pub state: Option<u32>,
    /// A join's ally, the exact occurrence-hour identity.
    pub ally: Option<String>,
    /// A join's hostility reason.
    pub reason: Option<String>,
    /// The native operands that change what the event means.
    pub notes: Vec<String>,
}

/// One recorded impact's target and outcome totals. A total is null when its
/// part was not fully recorded; `not_recorded` names why.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventFeedStrike {
    pub province: u32,
    /// Null for a province without a state.
    pub state: Option<u32>,
    /// Healthy building levels destroyed, in whole levels.
    pub building_levels: Option<f64>,
    /// The state owner's war support change, in percentage points.
    pub war_support: Option<f64>,
    /// Units the strike's damage calls reached.
    pub units: Option<u32>,
    pub manpower_lost: Option<f64>,
    /// Net equipment destroyed in division and air wing pools.
    pub equipment_lost: Option<f64>,
    pub not_recorded: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventFeedItem {
    pub token: Option<String>,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub description: Option<String>,
    /// The same content effects the Ideas widget supplies to its shared tooltip.
    pub idea: Option<idea_history::IdeaInfo>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventFeedWindow {
    pub first_hour: i32,
    pub last_hour: i32,
    pub count: u32,
    pub category_counts: EventFeedCounts,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventFeedCounts {
    pub wars_peace: u32,
    pub diplomacy: u32,
    pub politics: u32,
    pub milestones: u32,
}

#[cfg(test)]
impl EventFeedCounts {
    fn add(&mut self, category: EventFeedCategory) -> io::Result<()> {
        let count = match category {
            EventFeedCategory::WarsPeace => &mut self.wars_peace,
            EventFeedCategory::Diplomacy => &mut self.diplomacy,
            EventFeedCategory::Politics => &mut self.politics,
            EventFeedCategory::Milestones => &mut self.milestones,
        };
        *count = count
            .checked_add(1)
            .ok_or_else(|| io::Error::other("Event-feed category count exceeds u32"))?;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventFeedUnavailable {
    pub kind: String,
    pub label: String,
    pub reason: String,
    pub count: Option<u32>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventFeedReply {
    pub branch_id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub windows: Vec<EventFeedWindow>,
    pub rows: Vec<EventFeedRow>,
    pub items: Vec<EventFeedItem>,
    pub modifiers: Vec<ModifierFormat>,
    pub texts: Vec<LocalisedText>,
    pub unavailable: Vec<EventFeedUnavailable>,
    pub warnings: Vec<String>,
}

#[derive(Default)]
#[cfg(test)]
struct Cache {
    branches: VecDeque<(u64, CachedBranch)>,
}
#[derive(Clone)]
#[cfg(test)]
struct CachedBranch {
    catalogue: Option<Arc<Catalogue>>,
    feed: Arc<Feed>,
}
#[cfg(test)]
impl Cache {
    fn get(&mut self, branch: u64, catalogue: &Option<Arc<Catalogue>>) -> Option<CachedBranch> {
        let index = self.branches.iter().position(|(id, _)| *id == branch)?;
        let (_, cached) = self.branches.remove(index)?;
        let matches = match (&cached.catalogue, catalogue) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !matches {
            return None;
        }
        self.branches.push_back((branch, cached.clone()));
        Some(cached)
    }
    fn put(&mut self, branch: u64, cached: CachedBranch) {
        self.branches.retain(|(id, _)| *id != branch);
        self.branches.push_back((branch, cached));
        while self.branches.len() > CACHED_BRANCHES {
            self.branches.pop_front();
        }
    }
}
// Cache eviction only; never a limit on recorded branch facts.
#[cfg(test)]
const CACHED_BRANCHES: usize = 4;
type UnavailableKey = (String, String, Option<String>, bool);

#[derive(Serialize, Deserialize)]
struct Feed {
    first: i32,
    last: i32,
    rows: Vec<EventFeedRow>,
    items: Vec<EventFeedItem>,
    windows: Vec<EventFeedWindow>,
    modifiers: Vec<ModifierFormat>,
    texts: Vec<LocalisedText>,
    unavailable: Vec<EventFeedUnavailable>,
    warnings: Vec<String>,
}
#[derive(Serialize, Deserialize)]
struct Builder {
    feed: Feed,
    /// Every recorded native war end of the branch, by its hour and the two
    /// countries' tags, so a derived peace row the native row already shows
    /// is not repeated ([`peace_fallback`]).
    war_ends: BTreeSet<(i32, String, String)>,
    script_inputs: BTreeMap<String, script_effects::Inputs>,
    #[serde(with = "crate::prepared_production::map_pairs")]
    item_keys: BTreeMap<(Option<String>, Option<String>), Vec<u32>>,
    #[serde(with = "crate::prepared_production::map_pairs")]
    unavailable_keys: BTreeMap<UnavailableKey, (usize, Vec<String>)>,
}
/// A derived peace row's focal country and the counterparties no recorded
/// native war end covers. The derived row is first observed at `hour`, the
/// first read without the pair at war, so the relation ended between the
/// read of `hour - 1` and that of `hour`; a native end of the same pair dated
/// either hour (events drain with the hourly read, `EventFeedHistoryRow`
/// dates them no later than it) is the same ending, shown exactly.
fn peace_fallback(
    war_ends: &BTreeSet<(i32, String, String)>,
    hour: i32,
    tags: Vec<String>,
) -> Vec<String> {
    let mut tags = tags.into_iter();
    let Some(focal) = tags.next() else {
        return Vec::new();
    };
    let covered = |other: &String| {
        [hour.checked_sub(1), Some(hour)]
            .into_iter()
            .flatten()
            .any(|at| war_ends.contains(&war_end_key(at, &focal, other)))
    };
    let others: Vec<String> = tags.filter(|other| !covered(other)).collect();
    std::iter::once(focal).chain(others).collect()
}

/// A war end's key: its hour and the two tags in order.
fn war_end_key(hour: i32, first: &str, second: &str) -> (i32, String, String) {
    let (low, high) = if first <= second {
        (first, second)
    } else {
        (second, first)
    };
    (hour, low.to_owned(), high.to_owned())
}

struct RowDate {
    hour: i32,
    read_hour: i32,
    name_hour: i32,
    exact: bool,
}
impl RowDate {
    fn by(hour: i32) -> Self {
        Self {
            hour,
            read_hour: hour,
            name_hour: hour,
            exact: false,
        }
    }
}
impl Builder {
    fn new(first: i32, last: i32) -> Self {
        Self {
            war_ends: BTreeSet::new(),
            script_inputs: BTreeMap::new(),
            feed: Feed {
                first,
                last,
                rows: Vec::new(),
                items: Vec::new(),
                windows: Vec::new(),
                modifiers: Vec::new(),
                texts: Vec::new(),
                unavailable: Vec::new(),
                warnings: Vec::new(),
            },
            item_keys: BTreeMap::new(),
            unavailable_keys: BTreeMap::new(),
        }
    }
    fn item(&mut self, item: EventFeedItem) -> io::Result<u32> {
        let key = (item.token.clone(), item.name.clone());
        if let Some(indices) = self.item_keys.get(&key)
            && let Some(index) = indices
                .iter()
                .find(|index| self.feed.items[**index as usize] == item)
        {
            return Ok(*index);
        }
        let index = u32::try_from(self.feed.items.len())
            .map_err(|_| io::Error::other("Too many event-feed item identities"))?;
        self.feed.items.push(item);
        self.item_keys.entry(key).or_default().push(index);
        Ok(index)
    }
    fn row(
        &mut self,
        kind: EventFeedKind,
        date: RowDate,
        tags: Vec<String>,
        item: Option<u32>,
        amount: Option<u32>,
    ) {
        let RowDate {
            hour,
            read_hour,
            name_hour,
            exact,
        } = date;
        if hour < self.feed.first || hour > self.feed.last {
            return;
        }
        let id = format!("{kind:?}:{hour}:{read_hour}:{}", self.feed.rows.len());
        self.feed.rows.push(EventFeedRow {
            id,
            hour,
            read_hour,
            name_hour,
            exact,
            category: kind.category(),
            kind,
            tags,
            participants: None,
            item,
            amount,
            cause: None,
            strike: None,
        });
    }
    fn unavailable(
        &mut self,
        kind: &str,
        label: &str,
        reason: impl Into<String>,
        count: Option<u32>,
    ) {
        let reason = reason.into();
        let key = (
            kind.to_owned(),
            label.to_owned(),
            Some(reason.clone()),
            count.is_some(),
        );
        self.merge_unavailable(key, reason, count, Vec::new());
    }

    /// Only varying hour/pair diagnostics use bounded examples. Fixed reasons
    /// keep their own count, separate from standing notices without a count.
    fn sampled_unavailable(
        &mut self,
        kind: &str,
        label: &str,
        reason: impl Into<String>,
        count: u32,
    ) {
        let reason: String = reason.into().chars().take(240).collect();
        let key = (kind.to_owned(), label.to_owned(), None, true);
        self.merge_unavailable(key, reason.clone(), Some(count), vec![reason]);
    }

    fn merge_unavailable(
        &mut self,
        key: UnavailableKey,
        reason: String,
        count: Option<u32>,
        samples: Vec<String>,
    ) {
        if let Some((index, retained)) = self.unavailable_keys.get_mut(&key) {
            let existing = &mut self.feed.unavailable[*index];
            if key.3 {
                existing.count = existing
                    .count
                    .zip(count)
                    .and_then(|(old, new)| old.checked_add(new));
                if existing.count.is_none()
                    && !existing.reason.contains("the refusal count exceeds u32")
                {
                    existing
                        .reason
                        .push_str("; the refusal count exceeds u32 and is not representable");
                }
            }
            for sample in samples {
                if retained.len() < 3 && !retained.contains(&sample) {
                    existing.reason.push_str("; example: ");
                    existing.reason.push_str(&sample);
                    retained.push(sample);
                }
            }
        } else {
            let index = self.feed.unavailable.len();
            self.feed.unavailable.push(EventFeedUnavailable {
                kind: key.0.clone(),
                label: key.1.clone(),
                reason,
                count,
            });
            self.unavailable_keys.insert(key, (index, samples));
        }
    }

    #[cfg(test)]
    fn finish(mut self) -> io::Result<Feed> {
        self.feed.rows.sort_by(row_order);
        self.feed.warnings.sort();
        self.feed.warnings.dedup();
        self.feed.windows = windows(&self.feed, MAX_PRODUCT_REPLY_BYTES)?;
        Ok(self.feed)
    }
}

fn row_order(a: &EventFeedRow, b: &EventFeedRow) -> std::cmp::Ordering {
    (a.hour, a.read_hour, a.kind, &a.tags)
        .cmp(&(b.hour, b.read_hour, b.kind, &b.tags))
        .then_with(|| row_tie_order(a.kind, &a.id, &b.id))
}

// Native occurrence IDs define their tie order. Other rows retain their source
// insertion order, including the decimal boundary from ordinal 99 to 100.
fn row_tie_order(kind: EventFeedKind, first: &str, second: &str) -> std::cmp::Ordering {
    use EventFeedKind::*;
    if matches!(
        kind,
        WarDeclared
            | WarJoined
            | CivilWarWarTransferred
            | Annexed
            | WarEnded
            | Released
            | StateTransferred
            | PeaceConferenceStarted
            | NuclearStrike
    ) {
        return first.cmp(second);
    }
    match (
        first
            .rsplit(':')
            .next()
            .and_then(|value| value.parse::<u64>().ok()),
        second
            .rsplit(':')
            .next()
            .and_then(|value| value.parse::<u64>().ok()),
    ) {
        (Some(first), Some(second)) => first.cmp(&second),
        _ => first.cmp(second),
    }
}

fn check_cancel(cancelled: &AtomicBool) -> io::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Event feed request cancelled",
        ))
    } else {
        Ok(())
    }
}
fn plain_item(
    token: Option<String>,
    name: Option<String>,
    icon: Option<String>,
    description: Option<String>,
) -> EventFeedItem {
    EventFeedItem {
        token,
        name,
        icon,
        description,
        idea: None,
    }
}

#[derive(Default)]
struct ScriptContentIndex(std::sync::Mutex<Option<Arc<installed_content_events::events::LazyEvents>>>);

/// Event labels use the same recorded stack and event database lookup as
/// effect descriptions. A plain sole title needs no runtime trigger capture.
fn script_presentation(
    builder: &mut Builder,
    catalogue: &Catalogue,
    kind: EventFeedKind,
    item: &mut EventFeedItem,
    inputs: Option<&script_effects::Inputs>,
) -> io::Result<()> {
    if kind == EventFeedKind::DecisionRosterRebuilt {
        return decision_roster::present(builder, catalogue, item, inputs);
    }
    if !matches!(
        kind,
        EventFeedKind::ScriptEventFired
            | EventFeedKind::NewsEventFired
            | EventFeedKind::EventOptionSelected
    ) {
        if matches!(
            kind,
            EventFeedKind::DecisionSelected
                | EventFeedKind::DecisionRemoved
                | EventFeedKind::DecisionCancelled
                | EventFeedKind::DecisionTimedOut
                | EventFeedKind::DecisionActivated
                | EventFeedKind::DecisionCompleted
                | EventFeedKind::DecisionReenabled
                | EventFeedKind::DecisionRemovalEffect
                | EventFeedKind::DecisionCancellationEffect
                | EventFeedKind::DecisionCompletionEffect
                | EventFeedKind::DecisionTimeoutEffect
                | EventFeedKind::DecisionDurationChanged
        ) {
            let token = item.token.as_deref().unwrap_or("unidentified decision");
            builder.unavailable("decisionPresentation","Decision presentation",
                format!("Decision {token}: country-qualified title and triggered artwork require exact historical presentation inputs."),Some(1));
        }
        script_effects::present(builder, catalogue, kind, item, inputs)?;
        return Ok(());
    }
    let Some(token) = &item.token else {
        return Ok(());
    };
    let cache = catalogue.content.layers.derived::<ScriptContentIndex>();
    let events = {
        let mut index = cache
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(index.get_or_insert_with(|| {
            Arc::new(installed_content_events::events::LazyEvents::from_content(
                Arc::clone(&catalogue.content),
            ))
        }))
    };
    let event = match events.get(token.as_bytes()) {
        Ok(event) if event.refused.is_none() => event,
        Ok(event) => {
            builder.unavailable(
                "scriptEventContent",
                "Event content",
                event.refused.as_ref().unwrap(),
                Some(1),
            );
            script_effects::present(builder, catalogue, kind, item, inputs)?;
            return Ok(());
        }
        Err(reason) => {
            builder.unavailable("scriptEventContent", "Event content", reason, Some(1));
            script_effects::present(builder, catalogue, kind, item, inputs)?;
            return Ok(());
        }
    };
    let localise = |key: &[u8]| -> Result<String, String> {
        let key = std::str::from_utf8(key)
            .map_err(|_| format!("Event {token} localisation key is not UTF-8"))?;
        catalogue
            .localised(key, catalogue::View::Plain)
            .ok_or_else(|| {
                format!("Event {token}: {key} needs unavailable historical localisation operands")
            })
    };
    for (field, keys) in [
        ("title", event.sole_title_key()),
        ("body", event.sole_description_key()),
    ] {
        match keys.and_then(|key| key.map(localise).transpose()) {
            Ok(Some(value)) => {
                if field == "title" {
                    item.name = Some(value);
                } else {
                    item.description = Some(value);
                }
            }
            Ok(None) => {}
            Err(reason) => builder.unavailable(
                "scriptEventPresentation",
                &format!("Event {field}"),
                reason,
                Some(1),
            ),
        }
    }
    if event.picture.is_empty() {
        builder.unavailable(
            "scriptEventPicture",
            "Event picture",
            format!("Event {token}: default picture selection has no recorded presentation result"),
            Some(1),
        );
    } else if let Ok(picture) = std::str::from_utf8(&event.picture) {
        let image = (|| -> Result<String, String> {
            let sprite = catalogue
                .sprites
                .find(picture)?
                .ok_or_else(|| format!("Event {token}: picture {picture} is undeclared"))?;
            let decode = *catalogue.gui_decode.as_ref().map_err(Clone::clone)?;
            catalogue::sprite_frame_png(
                &catalogue.content.layers,
                sprite,
                decode,
                0,
                artwork::ICON_EDGE,
            )
            .map_err(|why| why.to_string())
        })();
        match image {
            Ok(icon) => item.icon = Some(icon),
            Err(reason) => {
                builder.unavailable("scriptEventPicture", "Event picture", reason, Some(1))
            }
        }
    } else {
        builder.unavailable(
            "scriptEventPicture",
            "Event picture",
            format!("Event {token}: picture key is not UTF-8"),
            Some(1),
        );
    }
    let mut options = Vec::new();
    for (ordinal, name) in event.option_names.iter().enumerate() {
        let name = match localise(name) {
            Ok(name) => name,
            Err(reason) => {
                builder.unavailable(
                    "scriptEventOptionName",
                    "Event option name",
                    reason,
                    Some(1),
                );
                String::from_utf8_lossy(name).into_owned()
            }
        };
        options.push(format!("{ordinal}: {name}"));
        for trigger in &event.option_triggers[ordinal] {
            builder.unavailable("scriptEventOptionAdmission","Offered event option",
                format!("Event {token} option {ordinal}: trigger {} needs exact presentation-time inputs",trigger.key.as_deref().map(String::from_utf8_lossy).unwrap_or_default()),Some(1));
        }
        if event.original_recipient_only[ordinal] {
            builder.unavailable("scriptEventOptionAdmission","Offered event option",format!("Event {token} option {ordinal}: original_recipient_only requires exact presentation ownership"),Some(1));
        }
    }
    if !options.is_empty() {
        let body = item.description.get_or_insert_default();
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str("Authored options (native ordinals):\n");
        body.push_str(&options.join("\n"));
    }
    script_effects::present(builder, catalogue, kind, item, inputs)?;
    Ok(())
}

#[cfg(test)]
fn build(
    source: &Source,
    branch: u64,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Feed> {
    let replay = source.replay();
    let start = replay
        .branches
        .iter()
        .find(|start| start.id == branch)
        .ok_or_else(|| io::Error::other("Unknown event-feed branch"))?;
    let (first, last) = branch_bounds(&replay, start);
    let mut builder = Builder::new(first, last);
    let catalogue = match context
        .content()
        .and_then(|content| catalogue::catalogue(source, content))
    {
        Ok(content) => Some(content),
        Err(reason) => {
            builder.unavailable(
                "contentStack",
                "Recorded content stack",
                reason.to_string(),
                Some(1),
            );
            None
        }
    };
    native::scan(&replay, branch, &mut builder, cancelled)?;
    let groups = country_groups::cached_scan(source, branch, cancelled)?;
    let diplomacy = diplomacy::scan(&replay, branch, &groups, cancelled)?;
    builder
        .feed
        .warnings
        .extend(groups.warnings.iter().cloned());
    builder.unavailable("warDeclarationOrJoin", "War declaration or join", "WarRelations supplies the pair becoming at war. Declarations and joins require the native event history; refused intervals cannot distinguish those actions.", None);
    // Recordings since EVENT_CAUSE_FORMAT record each war's end, release
    // and state owner change with its cause in the native history.
    if replay.format.version() < replay_data::rows::compatibility::EVENT_CAUSE_FORMAT {
        builder.unavailable("peaceOutcome", "Peace outcome", "A witnessed WarRelations removal records that the pair is no longer at war; the treaty, annexation or release cause is not recorded.", None);
        builder.unavailable("annexationCause", "Annexation or release", "CountryCounterInputs supplies owned-state existence. Its transitions have no cause; only a captured native annexation identifies that action and its roles.", None);
    }
    for (id, kind, label) in [
        (TableId::WarRelations, "warStarted", "Wars and peace"),
        (TableId::Capitulation, "capitulated", "Capitulations"),
        (
            TableId::CountryCounterInputs,
            "ceasedToExist",
            "Country existence",
        ),
        (TableId::FactionMembers, "factionJoined", "Faction changes"),
        (TableId::RelationTuples, "guaranteeGranted", "Guarantees"),
    ] {
        if !diplomacy.observed.contains(&id) {
            builder.unavailable(
                kind,
                label,
                format!("{id:?} was never recorded complete on this branch."),
                None,
            );
        }
    }
    for refusal in &diplomacy.unavailable {
        builder.unavailable(
            &refusal.key,
            &refusal.label,
            &refusal.reason,
            Some(
                u32::try_from(refusal.count)
                    .map_err(|_| io::Error::other("Diplomacy refusal count exceeds u32"))?,
            ),
        );
    }
    let faction_naming = diplomacy
        .moments
        .iter()
        .any(|moment| moment.faction.is_some())
        .then(|| {
            country_groups::load_faction_naming(
                source,
                &groups,
                context,
                &mut builder.feed.warnings,
            )
        });
    let mut faction_refusals = catalogue::Refusals::default();
    let mut faction_names = BTreeMap::new();
    for moment in &diplomacy.moments {
        check_cancel(cancelled)?;
        let name = if let Some(id) = moment.faction {
            let name = faction_names
                .entry((id, moment.name_hour))
                .or_insert_with(|| {
                    country_groups::faction_name_at(
                        &groups,
                        id,
                        moment.name_hour,
                        faction_naming
                            .as_ref()
                            .expect("faction moments need naming inputs"),
                        &mut faction_refusals,
                    )
                    .ok()
                });
            name.clone()
        } else {
            None
        };
        group_row(&mut builder, moment, name)?;
    }
    for (reason, count) in faction_refusals.counted_diagnostic_lines("faction names are not shown")
    {
        builder.unavailable(
            "factionName",
            "Faction name",
            reason,
            Some(
                u32::try_from(count)
                    .map_err(|_| io::Error::other("Faction name refusal count exceeds u32"))?,
            ),
        );
    }
    let subject_scan = subjects::scanned_branch(source, branch, cancelled)?;
    builder
        .feed
        .warnings
        .extend(subject_scan.warnings.iter().cloned());
    if !subject_scan.observed {
        builder.unavailable(
            "becameSubject",
            "Subjects",
            "No complete Subjects read with resolved countries was recorded.",
            None,
        );
    }
    for event in &subject_scan.starts {
        subject_row(&mut builder, event);
    }
    let parties = party_history::scanned_branch(source, branch, cancelled)?;
    for (reason, count) in &parties.refusals {
        builder.unavailable(
            "rulingPartyObservation",
            "Ruling party",
            reason.clone(),
            Some(
                u32::try_from(*count)
                    .map_err(|_| io::Error::other("Party refusal count exceeds u32"))?,
            ),
        );
    }
    if !parties.observed {
        builder.unavailable(
            "rulingPartyChanged",
            "Ruling party",
            "No complete PoliticalParties read with resolved countries was recorded.",
            None,
        );
    }
    let politics = match context
        .content()
        .and_then(|content| politics_content::politics_content(source, content))
    {
        Ok(content) => Some(content),
        Err(reason) => {
            builder.unavailable(
                "politicsContent",
                "Politics content",
                reason.to_string(),
                Some(1),
            );
            None
        }
    };
    if let Some(content) = &politics {
        builder
            .feed
            .warnings
            .extend(content.warnings.iter().cloned());
    }
    for (tag, scanned) in &parties.countries {
        for run in scanned.ruling.iter().filter(|run| !run.starts_segment) {
            // Ruling-party changes store no date.
            let name = scanned
                .party_name_runs
                .iter()
                .find(|name| name.first_hour <= run.first_hour && run.first_hour <= name.last_hour)
                .and_then(|name| {
                    politics.as_ref().and_then(|content| {
                        let (key, fallback) = match party_history::recorded_party_key(
                            scanned,
                            name.name as usize,
                            |key| content.defines(key),
                        ) {
                            Ok(value) => value,
                            Err(reason) => {
                                builder.unavailable(
                                    "partyName",
                                    "Ruling party names",
                                    reason,
                                    Some(1),
                                );
                                return None;
                            }
                        };
                        if let Some(reason) = fallback {
                            builder.feed.warnings.push(reason);
                        }
                        content.localised(&key, catalogue::View::Plain)
                    })
                });
            let item = builder.item(plain_item(Some(run.token.clone()), name, None, None))?;
            builder.row(
                EventFeedKind::RulingPartyChanged,
                RowDate::by(run.first_hour),
                vec![tag.clone()],
                Some(item),
                None,
            );
        }
    }
    let ideas = idea_history::scanned_branch(source, branch, cancelled)?;
    builder.feed.warnings.extend(ideas.warnings.iter().cloned());
    for (reason, count) in &ideas.refusals {
        builder.unavailable(
            "ideaObservation",
            "Laws and advisors",
            reason.clone(),
            Some(
                u32::try_from(*count)
                    .map_err(|_| io::Error::other("Idea refusal count exceeds u32"))?,
            ),
        );
    }
    if !ideas.observed {
        builder.unavailable("lawChanged", "Laws and advisors", "ActiveIdeas and HiredAdvisors were not recorded complete together with resolved countries.", None);
    }
    for scanned in ideas.countries.values() {
        let missing = idea_history::unavailable_additions(scanned);
        if missing > 0 {
            builder.unavailable("ideaAdditionAfterGap", "Law and advisor changes",
                "Changed holdings after an unrecorded stretch cannot establish whether a law changed or an advisor was hired.",
                Some(u32::try_from(missing).map_err(|_| io::Error::other("Idea-gap refusal count exceeds u32"))?));
        }
    }
    idea_rows(
        &mut builder,
        politics.as_deref(),
        ideas.countries.iter().flat_map(|(tag, scanned)| {
            idea_history::named_additions(scanned).map(move |(span, naming)| {
                (
                    tag.as_str(),
                    span,
                    naming,
                    span.character
                        .as_ref()
                        .and_then(|holder| scanned.templates.get(&holder.id)),
                )
            })
        }),
        cancelled,
    )?;
    focus_completion_rows(&mut builder, source, branch, context, cancelled)?;
    let projects = special_projects::completion_events(source, branch, context, cancelled)?;
    builder
        .feed
        .warnings
        .extend(projects.warnings.iter().cloned());
    for (reason, count) in &projects.refusals {
        builder.unavailable(
            "projectFinished",
            "Special project presentation",
            reason.clone(),
            Some(u32::try_from(*count).map_err(|_| {
                io::Error::other("Project refusal count exceeds the reply's recorded integer width")
            })?),
        );
    }
    if let Some(reason) = &projects.unavailable {
        builder.unavailable("projectFinished", "Special projects", reason.clone(), None);
    }
    for event in &projects.events {
        project_row(&mut builder, event)?;
    }
    let nukes = country_nukes::scanned_branch(source, branch, cancelled)?;
    if nukes.read_hours.is_empty() {
        builder.unavailable(
            "nukesIncreased",
            "Nuclear stockpile",
            "No complete Nukes read was recorded.",
            None,
        );
    }
    for (ordinal, count) in &nukes.refused {
        builder.unavailable(
            "nuclearType",
            "Nuclear type",
            format!("Nuclear type {ordinal} has no native content-name mapping."),
            Some(u32::try_from(*count).map_err(|_| {
                io::Error::other("Nuclear refusal count exceeds the reply's recorded integer width")
            })?),
        );
    }
    let nuke_art = catalogue
        .as_ref()
        .map(|catalogue| artwork::nuke_icon(catalogue))
        .transpose();
    let mut nuke_item = None;
    for (tag, runs) in &nukes.series {
        for pair in runs.windows(2) {
            let (old, new) = (&pair[0], &pair[1]);
            if new.starts_segment {
                continue;
            }
            let (Some(old), Some(new_total)) = (old.total, new.total) else {
                continue;
            };
            // Nuclear stockpile totals are signed i32. Their observed positive
            // difference can span u32; widening retains every rise.
            let increase = i64::from(new_total) - i64::from(old);
            if increase > 0 {
                let amount = u32::try_from(increase)
                    .map_err(|_| io::Error::other("Nuclear stockpile increase exceeds u32"))?;
                if let Err(reason) = &nuke_art {
                    builder.unavailable(
                        "nuclearArtwork",
                        "Nuclear stockpile artwork",
                        reason.to_string(),
                        Some(1),
                    );
                }
                let item = match nuke_item {
                    Some(item) => item,
                    None => {
                        let icon = nuke_art.as_ref().ok().cloned().flatten().flatten();
                        let item = nuclear_item(&mut builder, None, catalogue.as_deref(), icon)?;
                        nuke_item = Some(item);
                        item
                    }
                };
                builder.row(
                    EventFeedKind::NukesIncreased,
                    RowDate::by(new.first_hour),
                    vec![tag.clone()],
                    Some(item),
                    Some(amount),
                );
            }
        }
    }
    // Each native type's recorded spelling and name, independent of the stockpile.
    let mut nuke_items = BTreeMap::new();
    let icon = nuke_art.as_ref().ok().cloned().flatten().flatten();
    for info in country_nukes::native_type_info(&nukes, |token| {
        let content = catalogue.as_ref()?;
        content.localisation.get(token)?;
        content.localised(token, catalogue::View::Plain)
    }) {
        let item = nuclear_item(
            &mut builder,
            Some(&info),
            catalogue.as_deref(),
            icon.clone(),
        )?;
        nuke_items.insert(info.ordinal, item);
    }
    nuclear::scan(&replay, branch, &mut builder, &nuke_items, cancelled)?;
    if let Some(content) = &politics {
        let (modifier_keys, text_keys) = idea_history::effect_keys(
            builder
                .feed
                .items
                .iter()
                .filter_map(|item| item.idea.as_ref()),
        );
        let (formats, warnings) =
            idea_format::modifier_formats(&replay, branch, &modifier_keys, content, cancelled)?;
        builder.feed.modifiers = formats;
        builder.feed.texts = idea_format::texts(content, &text_keys);
        builder.feed.warnings.extend(warnings);
    }
    if let Some(catalogue) = &catalogue {
        let script_items: BTreeSet<_> = builder
            .feed
            .rows
            .iter()
            .filter_map(|row| row.item.map(|item| (item, row.kind, row.id.clone())))
            .collect();
        for (index, kind, id) in script_items {
            let mut item = builder.feed.items[index as usize].clone();
            let inputs = builder.script_inputs.get(&id).cloned();
            script_presentation(&mut builder, catalogue, kind, &mut item, inputs.as_ref())?;
            builder.feed.items[index as usize] = item;
        }
        for (reason, count) in catalogue.refusal_entries() {
            let count = count
                .map(u32::try_from)
                .transpose()
                .map_err(|_| io::Error::other("Content refusal count exceeds u32"))?;
            builder.unavailable("contentPresentation", "Content presentation", reason, count);
        }
    }
    builder.finish()
}

fn project_row(
    builder: &mut Builder,
    event: &special_projects::ProjectCompletion,
) -> io::Result<()> {
    if let Some(reason) = &event.icon_missing {
        builder.unavailable(
            "projectArtwork",
            "Special project artwork",
            format!(
                "Project {} (definition {}): {reason}",
                event.item_token.as_deref().unwrap_or("unknown token"),
                event.definition_id
            ),
            Some(1),
        );
    }
    let item = builder.item(plain_item(
        event.item_token.clone(),
        event.name.clone(),
        event.icon.clone(),
        None,
    ))?;
    builder.row(
        EventFeedKind::ProjectFinished,
        RowDate {
            hour: event.date.hour,
            read_hour: event.observed_hour,
            name_hour: event.date.hour,
            exact: event.date.exact,
        },
        vec![event.tag.clone()],
        Some(item),
        None,
    );
    Ok(())
}

fn nuclear_item(
    builder: &mut Builder,
    kind: Option<&country_nukes::NukeTypeInfo>,
    catalogue: Option<&Catalogue>,
    icon: Option<String>,
) -> io::Result<u32> {
    let token = kind.and_then(|kind| kind.token.clone());
    let name = kind.and_then(|kind| kind.name.clone()).or_else(|| {
        let token = token.as_deref()?;
        let content = catalogue?;
        content.localisation.get(token)?;
        content.localised(token, catalogue::View::Plain)
    });
    builder.item(plain_item(token, name, icon, None))
}

fn group_kind(kind: &str) -> io::Result<EventFeedKind> {
    use EventFeedKind::*;
    Ok(match kind {
        "warStarted" => WarStarted,
        "peace" => Peace,
        "capitulated" => Capitulated,
        "ceasedToExist" => CeasedToExist,
        "factionJoined" => FactionJoined,
        "factionLeft" => FactionLeft,
        "guaranteeGranted" => GuaranteeGranted,
        name => {
            return Err(io::Error::other(format!(
                "Unsupported shared group event {name}"
            )));
        }
    })
}
fn group_row(
    builder: &mut Builder,
    moment: &country_groups::changes::Change,
    name: Option<String>,
) -> io::Result<()> {
    let kind = group_kind(&moment.kind)?;
    let tags = if kind == EventFeedKind::Peace {
        peace_fallback(&builder.war_ends, moment.hour, moment.tags.clone())
    } else {
        moment.tags.clone()
    };
    if kind == EventFeedKind::Peace && tags.len() < 2 {
        return Ok(());
    }
    let item = moment
        .faction
        .map(|_| builder.item(plain_item(None, name, None, None)))
        .transpose()?;
    builder.row(
        kind,
        RowDate {
            hour: moment.hour,
            read_hour: moment.read_hour,
            name_hour: moment.name_hour,
            exact: moment.exact,
        },
        tags,
        item,
        None,
    );
    Ok(())
}
fn subject_row(builder: &mut Builder, event: &subjects::SubjectStart) {
    // The subject scan proves both tags at the stored relation date before using it.
    builder.row(
        EventFeedKind::BecameSubject,
        RowDate {
            hour: event.hour,
            read_hour: event.read_hour,
            name_hour: event.hour,
            exact: event.exact,
        },
        vec![event.subject.clone(), event.overlord.clone()],
        None,
        None,
    );
}
fn focus_row(
    builder: &mut Builder,
    event: &national_focus::FocusCompletion,
    presentation: Option<(&crate::Artwork, Option<&String>)>,
) -> io::Result<()> {
    let mut item = plain_item(Some(event.token.clone()), None, None, None);
    if let Some((art, refusal)) = presentation {
        item.name = art.name.clone();
        item.icon = art.icon.clone();
        item.description = art.description.clone();
        if let Some(reason) = refusal {
            builder.unavailable(
                "focusPresentation",
                "Focus presentation",
                format!("Focus {}: {reason}", event.token),
                Some(1),
            );
        }
    }
    let item = builder.item(item)?;
    builder.row(
        EventFeedKind::FocusCompleted,
        RowDate::by(event.hour),
        vec![event.tag.clone()],
        Some(item),
        None,
    );
    Ok(())
}

/// The same content/artwork builder consumes recorded additions or prepared pages.
fn idea_rows<'a>(
    builder: &mut Builder,
    politics: Option<&politics_content::PoliticsContent>,
    additions: impl IntoIterator<
        Item = (
            &'a str,
            &'a idea_history::IdeaSpan,
            Option<&'a crate::products::identity::NamingCountry>,
            Option<&'a Result<installed_content_core::characters::BuiltTemplate, String>>,
        ),
    >,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let mut refused_laws = BTreeMap::<String, u32>::new();
    let mut idea_infos = BTreeMap::new();
    for (tag, span, naming, built) in additions {
        check_cancel(cancelled)?;
        let mut item = plain_item(Some(span.token.clone()), None, None, None);
        let kind = if let Some(content) = politics {
            if span.character.is_none()
                && matches!(content.ideas.idea(&span.token), Ok(Some(idea)) if !idea.law)
            {
                continue;
            }
            let localise = |key: &str, view| content.localised(key, view);
            let joined = idea_history::Joined::of(content, &localise);
            let built = match built {
                Some(Ok(built)) => Ok(built),
                Some(Err(reason)) => Err(reason.as_str()),
                None => Err("no recorded holder names its template"),
            };
            let key = (
                span.token.clone(),
                span.slot.clone(),
                span.character.as_ref().map(|holder| holder.id),
            );
            let info = idea_infos
                .entry(key)
                .or_insert_with(|| {
                    idea_history::prepare_info(
                        &joined,
                        &span.token,
                        span.slot.as_deref(),
                        built,
                        false,
                    )
                    .0
                })
                .clone();
            // The idea group's authored law flag selects this branch.
            let kind = if span.character.is_some() {
                EventFeedKind::AdvisorHired
            } else if info.law {
                EventFeedKind::LawChanged
            } else if !info.effects_known {
                *refused_laws.entry(span.token.clone()).or_default() += 1;
                continue;
            } else {
                continue;
            };
            if let Some(naming) = naming {
                let holder = span.character.as_ref();
                let script = holder
                    .and_then(|holder| (!holder.token.is_empty()).then_some(holder.token.as_str()));
                let (art, problem) = match holder {
                    Some(holder) => content.held_idea_artwork(
                        &span.token,
                        script,
                        &holder.name,
                        naming.for_tag(tag),
                    ),
                    None => content.idea_artwork(&span.token, None, naming.for_tag(tag)),
                };
                if let Some(reason) = &problem {
                    builder.unavailable(
                        "ideaArtwork",
                        "Idea artwork",
                        format!("Idea {}: {reason}", span.token),
                        Some(1),
                    );
                }
                item.name = art.name;
                item.icon = art.icon;
                item.description = art.description;
                if let Some(holder) = holder
                    && script.is_none()
                {
                    item.icon = None;
                    let reason = format!(
                        "Advisor {} ({}) has no recorded script token; portrait refused.",
                        holder.id, span.token
                    );
                    builder.unavailable(
                        "advisorArtwork",
                        "Advisor portrait",
                        reason.clone(),
                        Some(1),
                    );
                }
            }
            item.idea = Some(info);
            kind
        } else if span.character.is_some() {
            EventFeedKind::AdvisorHired
        } else {
            *refused_laws.entry(span.token.clone()).or_default() += 1;
            continue;
        };
        let item = builder.item(item)?;
        builder.row(
            kind,
            RowDate::by(span.first_hour),
            vec![tag.to_owned()],
            Some(item),
            None,
        );
    }
    for (token, count) in refused_laws {
        builder.unavailable("lawChanged", "Law classification", format!("Idea {token}: the recorded content cannot establish whether this addition is a law."), Some(count));
    }
    Ok(())
}

/// The feed's focus completions as the focus family folds them hour by hour
/// over the branch, each with its art evaluated on its own hour's frame.
#[cfg(test)]
fn focus_completion_rows(
    builder: &mut Builder,
    source: &Source,
    branch: u64,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    use replay_frame::frame::FrameDecoder;
    let replay = source.replay_ref();
    let mut focus_art = None;
    if let Ok(content) = context.content()
        && let Err(reason) = artwork::ensure(&mut focus_art, source, content, artwork::Kind::Focus)
    {
        builder.unavailable(
            "focusPresentation",
            "Focus presentation",
            reason.to_string(),
            Some(1),
        );
    }
    let mut needed = BTreeSet::new();
    needed.extend(national_focus::FOCUS_COMPLETION_TABLES.iter().copied());
    needed.extend(crate::scripted_text::TABLES.iter().copied());
    needed.insert(TableId::Clock);
    needed.insert(TableId::Campaign);
    let needed: Vec<_> = needed.into_iter().collect();
    let mut focus = crate::prepared_focus::FocusIndex::default();
    let mut frames = BTreeMap::new();
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &needed,
        cancelled,
        |blocks| {
            for (_, read) in blocks {
                check_cancel(cancelled)?;
                let read = match read {
                    Ok(read) => read,
                    Err(reason) => {
                        builder.sampled_unavailable(
                            "completionRead",
                            "Completion observations",
                            reason.to_string(),
                            1,
                        );
                        continue;
                    }
                };
                let mut decoder = FrameDecoder::for_tables(&needed);
                for frame in read {
                    check_cancel(cancelled)?;
                    let decoded = match decoder.advance(&frame) {
                        Ok(decoded) => decoded,
                        Err(reason) => {
                            builder.sampled_unavailable(
                                "completionRead",
                                "Completion observations",
                                reason.to_string(),
                                1,
                            );
                            break;
                        }
                    };
                    if let Err(reason) = crate::metadata::validate_campaign(decoded, replay) {
                        builder.sampled_unavailable(
                            "completionRead",
                            "Completion observations",
                            reason.to_string(),
                            1,
                        );
                        break;
                    }
                    focus.observe_frame(replay, decoded, cancelled)?;
                    frames.insert(decoded.hour, decoded.clone());
                }
            }
            Ok(())
        },
    )?;
    focus.fold_held(cancelled)?;
    for event in focus.seen.clone() {
        let art = focus_art.as_mut().and_then(|art| {
            art.focuses_at(
                &[(event.token.as_str(), event.tag.as_str())],
                &frames[&event.hour],
            )
            .pop()
        });
        focus_row(
            builder,
            &event,
            art.as_ref().map(|(art, reason)| (art, reason.as_ref())),
        )?;
    }
    let (observed, unavailable) = focus.completion_metadata();
    if !observed {
        builder.unavailable(
            "focusCompleted",
            "Focus completions",
            "Focus and CompletedFocuses were not recorded complete with resolved countries.",
            None,
        );
    }
    for (reason, count) in unavailable {
        builder.unavailable(
            "focusCompleted",
            "Focus completions",
            reason.clone(),
            Some(
                u32::try_from(*count)
                    .map_err(|_| io::Error::other("Focus refusal count exceeds u32"))?,
            ),
        );
    }
    if let Some(art) = focus_art {
        builder.feed.warnings.extend(art.standing_warnings());
    }
    Ok(())
}

fn reply(feed: &Feed, branch: &str, first: i32, last: i32, with_rows: bool) -> EventFeedReply {
    let mut items = Vec::new();
    let mut remap = BTreeMap::new();
    let from = feed.rows.partition_point(|row| row.hour < first);
    let to = feed.rows.partition_point(|row| row.hour <= last);
    let rows = if with_rows {
        feed.rows[from..to]
            .iter()
            .map(|row| {
                let mut row = row.clone();
                row.item = row.item.map(|item| {
                    *remap.entry(item).or_insert_with(|| {
                        let index = items.len() as u32;
                        items.push(feed.items[item as usize].clone());
                        index
                    })
                });
                row
            })
            .collect()
    } else {
        Vec::new()
    };
    EventFeedReply {
        branch_id: branch.into(),
        first_hour: first,
        last_hour: last,
        windows: if with_rows {
            Vec::new()
        } else {
            feed.windows.clone()
        },
        rows,
        items,
        modifiers: if with_rows {
            feed.modifiers.clone()
        } else {
            Vec::new()
        },
        texts: if with_rows {
            feed.texts.clone()
        } else {
            Vec::new()
        },
        unavailable: if with_rows {
            Vec::new()
        } else {
            feed.unavailable.clone()
        },
        warnings: if with_rows {
            Vec::new()
        } else {
            feed.warnings.clone()
        },
    }
}

/// Admission serializes borrowed rows, before cloning any page. A remapped
/// reference has at most ten decimal digits (the recorded u32 field width).
#[cfg(test)]
fn window_fits(feed: &Feed, branch: &str, first: i32, last: i32, budget: usize) -> bool {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Measure<'a> {
        branch_id: &'a str,
        first_hour: i32,
        last_hour: i32,
        windows: &'a [EventFeedWindow],
        rows: &'a [EventFeedRow],
        items: Vec<&'a EventFeedItem>,
        modifiers: &'a [ModifierFormat],
        texts: &'a [LocalisedText],
        unavailable: &'a [EventFeedUnavailable],
        warnings: &'a [String],
    }
    let from = feed.rows.partition_point(|row| row.hour < first);
    let to = feed.rows.partition_point(|row| row.hour <= last);
    let rows = &feed.rows[from..to];
    let used: BTreeSet<_> = rows.iter().filter_map(|row| row.item).collect();
    let measure = Measure {
        branch_id: branch,
        first_hour: first,
        last_hour: last,
        windows: &[],
        rows,
        items: used
            .iter()
            .map(|item| &feed.items[*item as usize])
            .collect(),
        modifiers: &feed.modifiers,
        texts: &feed.texts,
        unavailable: &[],
        warnings: &[],
    };
    let remap_reserve = rows
        .iter()
        .filter_map(|row| row.item)
        .map(|item| 10 - item.checked_ilog10().map_or(1, |log| log as usize + 1))
        .sum::<usize>();
    crate::transport::json_len(&measure, budget.saturating_sub(remap_reserve)).is_some()
}

/// Count serialized rows and distinct artwork before allocating a window reply.
/// Entire hours stay together; a single oversized hour is refused with its count.
#[cfg(test)]
fn windows(feed: &Feed, budget: usize) -> io::Result<Vec<EventFeedWindow>> {
    let base = crate::transport::json_len(
        &EventFeedReply {
            branch_id: "0".into(),
            first_hour: feed.first,
            last_hour: feed.last,
            ..Default::default()
        },
        budget,
    )
    .ok_or_else(|| io::Error::other("Event-feed index exceeds the shared reply budget"))?;
    let shared = crate::transport::json_len(&feed.modifiers, budget)
        .and_then(|a| crate::transport::json_len(&feed.texts, budget).map(|b| a + b))
        .ok_or_else(|| {
            io::Error::other("Event-feed tooltip definitions exceed the shared reply budget")
        })?;
    let row_bytes: Vec<_> = feed
        .rows
        .iter()
        .map(|row| {
            crate::transport::json_len(row, budget)
                .map(|n| {
                    n + 1
                        + row.item.map_or(0, |item| {
                            10 - item.checked_ilog10().map_or(1, |log| log as usize + 1)
                        })
                })
                .ok_or_else(|| {
                    io::Error::other("One event-feed row exceeds the shared reply budget")
                })
        })
        .collect::<io::Result<_>>()?;
    let item_bytes: Vec<_> = feed
        .items
        .iter()
        .map(|item| {
            crate::transport::json_len(item, budget)
                .map(|n| n + 1)
                .ok_or_else(|| {
                    io::Error::other(format!(
                        "Event artwork {} exceeds the shared reply budget",
                        item.token.as_deref().unwrap_or("unknown")
                    ))
                })
        })
        .collect::<io::Result<_>>()?;
    let reserve = base + shared + 1024;
    let mut result: Vec<EventFeedWindow> = Vec::new();
    let mut bytes = reserve;
    let mut used = BTreeSet::new();
    let mut index = 0;
    while index < feed.rows.len() {
        let hour = feed.rows[index].hour;
        let end = index + feed.rows[index..].partition_point(|row| row.hour == hour);
        let count = u32::try_from(end - index)
            .map_err(|_| io::Error::other("Too many events at one hour"))?;
        let row_size = row_bytes[index..end].iter().sum::<usize>();
        let all_items: BTreeSet<_> = feed.rows[index..end]
            .iter()
            .filter_map(|row| row.item)
            .collect();
        let extra = |seen: &BTreeSet<u32>| {
            all_items
                .difference(seen)
                .map(|item| item_bytes[*item as usize])
                .sum::<usize>()
        };
        if bytes + row_size + extra(&used) > budget {
            bytes = reserve;
            used.clear();
            if bytes + row_size + extra(&used) > budget {
                return Err(io::Error::other(format!(
                    "Event feed hour {hour}: {count} events exceed the shared reply budget; no events were omitted"
                )));
            }
        }
        let new = bytes == reserve;
        bytes += row_size + extra(&used);
        used.extend(all_items);
        if new {
            result.push(EventFeedWindow {
                first_hour: hour,
                last_hour: hour,
                count,
                category_counts: EventFeedCounts::default(),
            });
        } else {
            let window = result.last_mut().expect("started window");
            window.last_hour = hour;
            window.count = window
                .count
                .checked_add(count)
                .ok_or_else(|| io::Error::other("Too many events in a window"))?;
        }
        let window = result.last_mut().expect("started window");
        for row in &feed.rows[index..end] {
            window.category_counts.add(row.category)?;
        }
        index = end;
    }
    Ok(result)
}

impl Product for EventFeed {
    const KEY: &'static str = "event-feed";
    const CONTENT_DEPENDENT: bool = true;
    type Params = EventFeedParams;
    type Reply = EventFeedReply;
    fn types() -> Vec<String> {
        vec![
            EventFeedParams::decl(),
            EventFeedCategory::decl(),
            EventFeedKind::decl(),
            EventFeedRow::decl(),
            EventFeedCause::decl(),
            EventFeedStrike::decl(),
            EventFeedItem::decl(),
            EventFeedWindow::decl(),
            EventFeedCounts::decl(),
            EventFeedUnavailable::decl(),
            EventFeedReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        match (params.first_hour, params.last_hour) {
            (None, None) => Ok(()),
            (Some(first), Some(last)) if first <= last => Ok(()),
            _ => Err("Invalid event-feed hour window"),
        }
    }
    #[cfg(test)]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        check_cancel(cancelled)?;
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        let catalogue = context
            .content()
            .and_then(|content| catalogue::catalogue(source, content))
            .ok();
        let state = source.state::<Cache>();
        let cached = state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(branch, &catalogue);
        let cached = match cached {
            Some(cached) => cached,
            None => CachedBranch {
                catalogue,
                feed: Arc::new(build(source, branch, context, cancelled)?),
            },
        };
        let feed = Arc::clone(&cached.feed);
        state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .put(branch, cached);
        let (first, last) = params
            .first_hour
            .zip(params.last_hour)
            .unwrap_or((feed.first, feed.last));
        if first < feed.first || last > feed.last {
            return Err(io::Error::other(
                "Event-feed hour window is outside the recorded branch",
            ));
        }
        if params.first_hour.is_some()
            && !window_fits(
                &feed,
                &params.branch_id,
                first,
                last,
                MAX_PRODUCT_REPLY_BYTES - 1024,
            )
        {
            return Err(io::Error::other(
                "The requested event-feed window exceeds the shared reply budget; request the indexed windows",
            ));
        }
        let result = reply(
            &feed,
            &params.branch_id,
            first,
            last,
            params.first_hour.is_some(),
        );
        super::check_reply_budget(&result).map_err(|_| {
            io::Error::other(
                "The requested event-feed window exceeds the shared reply budget; request the indexed windows",
            )
        })?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
