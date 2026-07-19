//! Event popup chrome (U2 wave — REPLAY_PRODUCTION_PLAN §6a / UI_COVERAGE_MATRIX
//! §3 rows "Event window" + "Generic popup").
//!
//! the game's ~7,000 vanilla events are **data-driven**: every one renders through
//! the handful of chrome windows declared once in `interface/eventwindow.gui`
//! (`EventWindow`, `EventWindow_News`, `EventWindow_leader`,
//! `EventWindow_Operative`) plus the shared `event_option_entry` row template.
//! This module derives that chrome two ways:
//!
//!   1. **Layout gates** ([`load_event_layout`] + [`event_window_chrome`]) — the
//!      shipped `.gui` rects/sprites/fonts for each window variant's Title,
//!      Description, picture icon, and options grid, pulled through the shared
//!      [`crate::gui`] parser. FILE-derived, gated in `tests/event_chrome.rs`
//!      against the exact shipped values.
//!
//!   2. **Event-def → window resolution** ([`load_events`] +
//!      [`resolve_events`]) — each `events/*.txt` definition (`id`, `title`,
//!      `desc`, `picture`, `option`) is parsed and its localisation keys / GFX
//!      sprite are resolved against the loaded [`Localisation`] +
//!      [`SpriteRegistry`], measuring how many events resolve to a fully
//!      renderable window. FILE-derived (loc + gfx joins).
//!
//! ## Data dependency (NEW): the fired-event stream
//!
//! A replay renders the events that FIRED during the recording. That stream —
//! which event id fired, to whom, when, and the chosen option — is **not in any
//! recording today** (EXTRACTION_NEEDS §A A18 / REPLAY_DATA_REQUIREMENTS §14).
//! Until it lands, [`event_window_content`] is **fail-closed**: with no
//! [`FiredEvent`] the window binds the absent "no event data" form (Constraint
//! 3 — never a fabricated title/option). The layout + the event-def→window
//! mapping are complete and gated regardless; only the per-moment CONTENT waits
//! on the stream.
//!
//! ## Honesty labels
//!
//! * Window selection by event TYPE (`news_event`→`EventWindow_News`,
//!   `unit_leader_event`→`EventWindow_leader`, `operative_leader_event`→
//!   `EventWindow_Operative`, else `EventWindow`) is **FILE-motivated**: the
//!   engine picks the operative/leader variant from event *content* (a scoped
//!   character/operative), not purely from the top-level keyword, so a
//!   `country_event` that shows a leader is mapped to the plain window here
//!   (documented UNKNOWN edge, not fabricated).
//! * Which `desc` branch shows is `trigger`-gated (computed-not-stored); with a
//!   fired-event this module takes the FIRST loc-resolvable desc key and records
//!   the choice (FIT) — the recording's fired-event stream may later carry the
//!   exact shown desc.

use crate::assets::AssetResolver;
use crate::gfx::SpriteRegistry;
use crate::gui::{GuiElement, LayoutSet};
use crate::loc::Localisation;
use crate::paradox_text::{parse_file, Document, Value};
use crate::text_law::{resolve_localisation, RuntimeTextContext, TextIssue};
use crate::Result;
use replay_core::{FiredEventRow as RecordedFiredEventRow, FiredEventsView, Leaf};
use std::collections::{BTreeMap, BTreeSet};

/// The event-window layout file (FILE: shipped `interface/eventwindow.gui`).
pub const EVENT_WINDOW_GUI: &str = "interface/eventwindow.gui";

/// The four event-window variants declared in `eventwindow.gui`, in file order.
pub const EVENT_WINDOW_VARIANTS: [&str; 4] = [
    "EventWindow",
    "EventWindow_Operative",
    "EventWindow_leader",
    "EventWindow_News",
];

/// The shared option-row template window (FILE: `eventwindow.gui`
/// `event_option_entry`, populated once per `option` into `options_grid`).
pub const EVENT_OPTION_ENTRY: &str = "event_option_entry";

// ===========================================================================
// Event kind + window selection
// ===========================================================================

/// The Clausewitz event-definition keyword classes shipped in `events/*.txt`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Country,
    News,
    State,
    UnitLeader,
    OperativeLeader,
}

impl EventKind {
    /// Map a top-level definition key to its kind, or `None` for non-event keys
    /// (`add_namespace`, comments the parser dropped, …).
    pub fn from_key(key: &str) -> Option<EventKind> {
        match key.to_ascii_lowercase().as_str() {
            "country_event" => Some(EventKind::Country),
            "news_event" => Some(EventKind::News),
            "state_event" => Some(EventKind::State),
            "unit_leader_event" => Some(EventKind::UnitLeader),
            "operative_leader_event" => Some(EventKind::OperativeLeader),
            _ => None,
        }
    }

    /// The definition keyword (for reporting).
    pub fn keyword(&self) -> &'static str {
        match self {
            EventKind::Country => "country_event",
            EventKind::News => "news_event",
            EventKind::State => "state_event",
            EventKind::UnitLeader => "unit_leader_event",
            EventKind::OperativeLeader => "operative_leader_event",
        }
    }

    /// The `eventwindow.gui` chrome window this kind renders through (see the
    /// module honesty note — this is by TYPE; the engine's leader/operative
    /// selection is content-gated).
    pub fn window_name(&self) -> &'static str {
        match self {
            EventKind::News => "EventWindow_News",
            EventKind::UnitLeader => "EventWindow_leader",
            EventKind::OperativeLeader => "EventWindow_Operative",
            // country_event + state_event use the plain report window.
            EventKind::Country | EventKind::State => "EventWindow",
        }
    }
}

// ===========================================================================
// Event-definition model
// ===========================================================================

/// One option row of an event (the `option = { name = KEY ... }` block). Only
/// the localisation-key `name` is chrome-relevant; effects/triggers are
/// gameplay, not rendered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventOption {
    /// The `name` localisation key, or `None` for a nameless (hidden) option.
    pub name_key: Option<String>,
    /// A triggered option is not necessarily visible for a particular firing;
    /// XUI21 therefore admits its definition ordinal only from the recording.
    pub has_trigger: bool,
}

/// A parsed event definition — exactly the fields the chrome binds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventDef {
    pub id: String,
    pub kind: EventKind,
    /// `title` localisation key.
    pub title_key: Option<String>,
    /// Every `desc` / `desc = { text = KEY }` candidate key, in file order.
    pub desc_keys: Vec<String>,
    /// `picture` GFX sprite name, when the event declares one.
    pub picture_gfx: Option<String>,
    pub options: Vec<EventOption>,
    /// `hidden = yes` — the event fires effects but shows no window.
    pub hidden: bool,
    /// `major = yes` (news events; full-screen news chrome).
    pub major: bool,
    /// Scripted timeout in days. `None` means the engine's effective default;
    /// the producer records that resolved value instead of replay hard-coding it.
    pub timeout_days: Option<u32>,
    pub source_file: String,
}

impl EventDef {
    /// The chrome window this event renders through.
    pub fn window_name(&self) -> &'static str {
        self.kind.window_name()
    }
}

/// Collect the `desc` candidate keys from one event value. A `desc` is either a
/// bare scalar key (`desc = generic.1.d`) or a conditional block
/// (`desc = { text = KEY trigger = {...} }`); an event may declare several.
fn desc_keys(value: &Value) -> Vec<String> {
    let mut keys = Vec::new();
    for desc in value.values_ci("desc") {
        match desc {
            Value::Scalar(_) => {
                if let Some(key) = desc.as_scalar() {
                    let key = key.trim();
                    if !key.is_empty() {
                        keys.push(key.to_string());
                    }
                }
            }
            Value::Object(_) => {
                if let Some(key) = desc.first_scalar_ci("text") {
                    let key = key.trim();
                    if !key.is_empty() {
                        keys.push(key.to_string());
                    }
                }
            }
        }
    }
    keys
}

/// Collect the `option` rows (their `name` loc keys) from one event value.
fn option_rows(value: &Value) -> Vec<EventOption> {
    value
        .values_ci("option")
        .filter(|opt| opt.as_object().is_some())
        .map(|opt| EventOption {
            name_key: opt
                .first_scalar_ci("name")
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string),
            has_trigger: opt.values_ci("trigger").next().is_some(),
        })
        .collect()
}

/// Parse one event value into an [`EventDef`], or `None` if it has no `id`
/// (a malformed definition — fail-closed, not fabricated).
fn parse_event_value(kind: EventKind, source_file: &str, value: &Value) -> Option<EventDef> {
    let id = value.first_scalar_ci("id")?.trim().to_string();
    if id.is_empty() {
        return None;
    }
    Some(EventDef {
        id,
        kind,
        title_key: value
            .first_scalar_ci("title")
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_string),
        desc_keys: desc_keys(value),
        picture_gfx: value
            .first_scalar_ci("picture")
            .map(str::trim)
            .filter(|pic| !pic.is_empty())
            .map(str::to_string),
        options: option_rows(value),
        hidden: value
            .first_scalar_ci("hidden")
            .is_some_and(|v| v.eq_ignore_ascii_case("yes")),
        major: value
            .first_scalar_ci("major")
            .is_some_and(|v| v.eq_ignore_ascii_case("yes")),
        timeout_days: value
            .first_scalar_ci("timeout_days")
            .and_then(|value| value.parse::<u32>().ok()),
        source_file: source_file.to_string(),
    })
}

/// Parse every top-level event definition from one parsed `events/*.txt`
/// document. Only ROOT-level entries are definitions; nested
/// `country_event = { id = … days = N }` firing *effects* live inside option
/// blocks and are never at the document root, so they are excluded.
pub fn parse_events_document(source_file: &str, doc: &Document) -> Vec<EventDef> {
    let mut defs = Vec::new();
    for entry in doc.root.fields() {
        let Some(key) = entry.key.as_deref() else {
            continue;
        };
        let Some(kind) = EventKind::from_key(key) else {
            continue;
        };
        if let Some(def) = parse_event_value(kind, source_file, &entry.value) {
            defs.push(def);
        }
    }
    defs
}

/// The parsed corpus of all `events/*.txt` definitions plus parse bookkeeping.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventCorpus {
    pub defs: Vec<EventDef>,
    pub files_parsed: usize,
    pub parse_failures: Vec<String>,
}

/// Load and parse every `events/*.txt` definition through the resolver (mod
/// over vanilla — no bare joins).
pub fn load_events(resolver: &AssetResolver) -> Result<EventCorpus> {
    let files = resolver.list_files_recursive("events", Some("txt"))?;
    let mut corpus = EventCorpus::default();
    for file in files {
        match parse_file(&file.path) {
            Ok(doc) => {
                corpus.files_parsed += 1;
                corpus
                    .defs
                    .extend(parse_events_document(&file.rel_path, &doc));
            }
            Err(err) => corpus
                .parse_failures
                .push(format!("{}: {}", file.rel_path, err.message)),
        }
    }
    Ok(corpus)
}

// ===========================================================================
// Event-def → window resolution (loc + gfx joins)
// ===========================================================================

/// Per-event resolution outcome: does every chrome field resolve to renderable
/// content against the loaded localisation + sprite registry?
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventResolution {
    pub id: String,
    pub kind: EventKind,
    /// The chrome window this event maps to (always resolves via kind).
    pub window_name: &'static str,
    pub title_resolved: bool,
    pub desc_resolved: bool,
    /// The event declared a `picture` GFX.
    pub picture_declared: bool,
    /// Declared picture GFX is defined in the sprite registry (only meaningful
    /// when `picture_declared`).
    pub picture_resolved: bool,
    pub options_total: usize,
    /// Options whose `name` key resolves in localisation.
    pub options_resolved: usize,
    /// Fully renderable: title + ≥1 desc + picture-ok + ≥1 option all resolve.
    /// (picture-ok = declared→registry-present, or undeclared→engine default.)
    pub renderable: bool,
}

/// True when a localisation key is present and non-empty in `loc`.
fn loc_present(loc: &Localisation, key: &str) -> bool {
    loc.get(key).is_some_and(|value| !value.trim().is_empty())
}

/// Resolve one event definition against localisation + sprites.
pub fn resolve_event(
    def: &EventDef,
    loc: &Localisation,
    registry: &SpriteRegistry,
) -> EventResolution {
    let title_resolved = def
        .title_key
        .as_deref()
        .is_some_and(|key| loc_present(loc, key));
    let desc_resolved = def.desc_keys.iter().any(|key| loc_present(loc, key));
    let picture_declared = def.picture_gfx.is_some();
    let picture_resolved = def
        .picture_gfx
        .as_deref()
        .is_some_and(|gfx| registry.get(gfx).is_some());
    let picture_ok = !picture_declared || picture_resolved;
    let options_total = def.options.len();
    let options_resolved = def
        .options
        .iter()
        .filter(|opt| {
            opt.name_key
                .as_deref()
                .is_some_and(|key| loc_present(loc, key))
        })
        .count();
    let renderable = title_resolved && desc_resolved && picture_ok && options_resolved >= 1;
    EventResolution {
        id: def.id.clone(),
        kind: def.kind,
        window_name: def.window_name(),
        title_resolved,
        desc_resolved,
        picture_declared,
        picture_resolved,
        options_total,
        options_resolved,
        renderable,
    }
}

/// Aggregate resolution report over a whole corpus (the U2 gate numbers).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventResolutionReport {
    /// All parsed definitions.
    pub total: usize,
    /// `hidden = yes` events (fire effects, never show a window — excluded from
    /// the renderable denominator).
    pub hidden: usize,
    /// Non-hidden (visible) events = the renderable denominator.
    pub visible: usize,
    pub title_resolved: usize,
    pub desc_resolved: usize,
    pub picture_declared: usize,
    pub picture_resolved: usize,
    /// Visible events with ≥1 loc-resolved option.
    pub options_resolved_any: usize,
    /// Visible events fully renderable (title + desc + picture-ok + ≥1 option).
    pub renderable: usize,
    /// Per-kind visible totals, in [`EventKind`] declaration order.
    pub visible_by_kind: [usize; 5],
    /// Per-kind renderable totals.
    pub renderable_by_kind: [usize; 5],
}

fn kind_index(kind: EventKind) -> usize {
    match kind {
        EventKind::Country => 0,
        EventKind::News => 1,
        EventKind::State => 2,
        EventKind::UnitLeader => 3,
        EventKind::OperativeLeader => 4,
    }
}

/// Resolve every definition in a corpus and aggregate the gate counts. Hidden
/// events are counted but excluded from every "resolved / renderable" tally
/// (they intentionally show no chrome).
pub fn resolve_events(
    corpus: &EventCorpus,
    loc: &Localisation,
    registry: &SpriteRegistry,
) -> EventResolutionReport {
    let mut report = EventResolutionReport {
        total: corpus.defs.len(),
        ..EventResolutionReport::default()
    };
    for def in &corpus.defs {
        if def.hidden {
            report.hidden += 1;
            continue;
        }
        report.visible += 1;
        let ki = kind_index(def.kind);
        report.visible_by_kind[ki] += 1;
        let res = resolve_event(def, loc, registry);
        if res.title_resolved {
            report.title_resolved += 1;
        }
        if res.desc_resolved {
            report.desc_resolved += 1;
        }
        if res.picture_declared {
            report.picture_declared += 1;
        }
        if res.picture_resolved {
            report.picture_resolved += 1;
        }
        if res.options_resolved >= 1 {
            report.options_resolved_any += 1;
        }
        if res.renderable {
            report.renderable += 1;
            report.renderable_by_kind[ki] += 1;
        }
    }
    report
}

// ===========================================================================
// Layout gates — eventwindow.gui chrome rects/sprites/fonts
// ===========================================================================

/// One chrome element's FILE-derived layout facts (raw `.gui` strings, so a
/// `100%` percent axis or a signed offset survives verbatim for the test).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventChromeElement {
    pub name: String,
    pub element_type: String,
    pub position: Option<(String, String)>,
    pub size: Option<(String, String)>,
    /// First sprite reference (`spriteType` / `quadTextureSprite`).
    pub sprite: Option<String>,
    pub font: Option<String>,
    pub format: Option<String>,
    pub max_width: Option<String>,
}

/// One event-window variant's key chrome elements, located by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventWindowChrome {
    pub window_name: String,
    pub window_size: Option<(String, String)>,
    pub background_sprite: Option<String>,
    pub title: Option<EventChromeElement>,
    pub description: Option<EventChromeElement>,
    /// The `event_picture` icon (the event's GFX picture slot).
    pub picture: Option<EventChromeElement>,
    /// The `options_grid` gridBox that hosts the option-row template.
    pub options_grid: Option<EventChromeElement>,
    pub options_slot_size: Option<(String, String)>,
    pub options_max_slots_horizontal: Option<u32>,
}

fn raw_pair(pair: &Option<crate::gui::MeasurePair>) -> Option<(String, String)> {
    pair.as_ref().map(|p| (p.x.raw.clone(), p.y.raw.clone()))
}

/// Build an [`EventChromeElement`] from a parsed [`GuiElement`], pulling
/// `maxWidth` from the raw value (the parser structures position/size/sprite
/// but not the text-box max extents).
fn chrome_element(element: &GuiElement) -> EventChromeElement {
    EventChromeElement {
        name: element.name.clone(),
        element_type: element.element_type.clone(),
        position: raw_pair(&element.position),
        size: raw_pair(&element.size),
        sprite: element.sprite_refs.first().cloned(),
        font: element.font.clone(),
        format: element.format.clone(),
        max_width: element.raw.first_scalar_ci("maxWidth").map(str::to_string),
    }
}

/// Find the first descendant of `root` named `name` (case-insensitive), if any.
fn find_child<'a>(root: &'a GuiElement, name: &str) -> Option<&'a GuiElement> {
    let mut hits = Vec::new();
    root.find_by_name(name, &mut hits);
    hits.into_iter().next()
}

/// Parse `interface/eventwindow.gui` into a [`LayoutSet`] via the shared parser
/// (mod over vanilla).
pub fn load_event_layout(resolver: &AssetResolver) -> Result<LayoutSet> {
    let resolved = resolver.resolve_file(EVENT_WINDOW_GUI)?;
    let mut layouts = LayoutSet::default();
    layouts.parse_file(&resolved.path, &resolved.rel_path)?;
    Ok(layouts)
}

/// Locate one event-window variant's chrome elements from a parsed layout. The
/// `background`/`Title`/`Description`/`event_picture`/`options_grid` names are
/// the shipped `eventwindow.gui` element names. Returns `None` if the window
/// variant is absent (fail-closed).
pub fn event_window_chrome(layout: &LayoutSet, window_name: &str) -> Option<EventWindowChrome> {
    let window = layout
        .find_by_name(window_name)
        .into_iter()
        .find(|e| e.element_type.eq_ignore_ascii_case("containerWindowType"))?;
    let background_sprite =
        find_child(window, "Background").and_then(|bg| bg.sprite_refs.first().cloned());
    let options_grid = find_child(window, "options_grid");
    let options_slot_size = options_grid
        .and_then(|grid| grid.raw.first_value_ci("slotsize"))
        .and_then(|slot| {
            Some((
                slot.first_scalar_ci("width")?.to_string(),
                slot.first_scalar_ci("height")?.to_string(),
            ))
        });
    let options_max_slots_horizontal = options_grid
        .and_then(|grid| grid.raw.first_scalar_ci("max_slots_horizontal"))
        .and_then(|value| value.parse::<u32>().ok());
    Some(EventWindowChrome {
        window_name: window.name.clone(),
        window_size: raw_pair(&window.size),
        background_sprite,
        title: find_child(window, "Title").map(chrome_element),
        description: find_child(window, "Description").map(chrome_element),
        picture: find_child(window, "event_picture").map(chrome_element),
        options_grid: options_grid.map(chrome_element),
        options_slot_size,
        options_max_slots_horizontal,
    })
}

/// The shared option-row template (`event_option_entry`) and its `Name`
/// text-box, located from a parsed layout.
pub fn event_option_entry_chrome(
    layout: &LayoutSet,
) -> Option<(EventChromeElement, EventChromeElement)> {
    let entry = layout
        .find_by_name(EVENT_OPTION_ENTRY)
        .into_iter()
        .find(|e| e.element_type.eq_ignore_ascii_case("containerWindowType"))?;
    let name = find_child(entry, "Name")?;
    Some((chrome_element(entry), chrome_element(name)))
}

// ===========================================================================
// Fired-event binding (fail-closed; NEW recording dependency)
// ===========================================================================

/// One fired-event record — the NEW recording datum this chrome needs
/// (EXTRACTION_NEEDS §A A18 / REPLAY_DATA_REQUIREMENTS §14). Not present in any
/// recording today; this struct defines the shape the binding consumes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FiredEvent {
    /// The `id` of the event definition that fired (join key).
    pub event_id: String,
    /// The country the event fired to (for `[FROM]`/target loc — optional).
    pub target_tag: Option<String>,
    /// The `option` index the player/AI chose, when recorded.
    pub chosen_option_index: Option<usize>,
    /// The recording date/hour the event fired, when recorded.
    pub fire_date: Option<String>,
}

/// The bound event-window content for one fired event. All content fields are
/// `None`/empty in the fail-closed (no-stream) form.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventWindowContent {
    /// `true` only when a [`FiredEvent`] was supplied for this event.
    pub available: bool,
    pub window_name: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub picture_gfx: Option<String>,
    /// Resolved option button texts, in file order.
    pub options: Vec<String>,
    /// The chosen option index carried by the fired-event record.
    pub chosen_option_index: Option<usize>,
}

/// Bind an event definition + its fired-event record into renderable window
/// content, resolving loc keys. Fail-closed: `fired == None` yields the empty
/// "no event data" content (Constraint 3 — no fabricated title/options).
///
/// The description is the FIRST loc-resolvable `desc` candidate (which desc
/// branch shows is `trigger`-gated and not in the def — documented FIT).
pub fn event_window_content(
    def: &EventDef,
    loc: &Localisation,
    fired: Option<&FiredEvent>,
) -> EventWindowContent {
    let Some(fired) = fired else {
        return EventWindowContent::default();
    };
    debug_assert_eq!(fired.event_id, def.id, "fired-event id must match def id");
    let title = def
        .title_key
        .as_deref()
        .and_then(|key| loc.get(key))
        .map(str::to_string);
    let description = def
        .desc_keys
        .iter()
        .find_map(|key| loc.get(key))
        .map(str::to_string);
    let options = def
        .options
        .iter()
        .filter_map(|opt| opt.name_key.as_deref())
        .filter_map(|key| loc.get(key))
        .map(str::to_string)
        .collect();
    EventWindowContent {
        available: true,
        window_name: Some(def.window_name().to_string()),
        title,
        description,
        picture_gfx: def.picture_gfx.clone(),
        options,
        chosen_option_index: fired.chosen_option_index,
    }
}

// ===========================================================================
// XUI21 production admission: ordered fired-event history + visible popup
// ===========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventPopupSubtype {
    Country,
    News,
    State,
    UnitLeader,
    OperativeLeader,
}

impl EventPopupSubtype {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "country" => Some(Self::Country),
            "news" => Some(Self::News),
            "state" => Some(Self::State),
            "unit_leader" => Some(Self::UnitLeader),
            "operative_leader" => Some(Self::OperativeLeader),
            _ => None,
        }
    }

    pub fn window_name(self) -> &'static str {
        match self {
            Self::News => "EventWindow_News",
            Self::UnitLeader => "EventWindow_leader",
            Self::OperativeLeader => "EventWindow_Operative",
            Self::Country | Self::State => "EventWindow",
        }
    }

    fn compatible_with(self, kind: EventKind) -> bool {
        match kind {
            EventKind::News => self == Self::News,
            EventKind::State => self == Self::State,
            EventKind::UnitLeader => self == Self::UnitLeader,
            EventKind::OperativeLeader => self == Self::OperativeLeader,
            // Some country events materialise the leader/operative chrome from
            // scoped content.  That exact choice is producer-authenticated.
            EventKind::Country => matches!(
                self,
                Self::Country | Self::UnitLeader | Self::OperativeLeader
            ),
        }
    }

    fn default_picture(self) -> &'static str {
        match self {
            Self::News => "GFX_news_event_001",
            _ => "GFX_report_event_001",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedEventOption {
    pub definition_ordinal: usize,
    pub text: String,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedEventPopup {
    pub subtype: EventPopupSubtype,
    pub window_name: &'static str,
    pub title: String,
    pub description: String,
    pub selected_desc_key: String,
    pub picture_gfx: String,
    pub options: Vec<AdmittedEventOption>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedFiredEvent {
    pub event_id: String,
    pub recipient_tag: String,
    pub fire_date: String,
    pub fire_hour: i64,
    pub fire_tick: i64,
    pub event_sequence: i64,
    pub definition_hidden: bool,
    pub chosen_option_ordinal: Option<usize>,
    pub effective_timeout_hours: i64,
    pub timeout_tick: i64,
    pub resolved_tick: Option<i64>,
    pub resolution_reason: String,
    pub runtime_text_context_id: Option<String>,
    /// Present only when the producer supplied enough exact materialisation
    /// evidence to render a popup. Hidden, AI-only and otherwise
    /// non-materialised lifecycle rows remain admitted with `None`.
    pub popup: Option<AdmittedEventPopup>,
}

impl AdmittedFiredEvent {
    /// Exact tick-domain visibility. No hour-to-tick conversion is attempted.
    pub fn visible_at(&self, recipient_tag: &str, tick: i64) -> bool {
        if self.popup.is_none() || self.recipient_tag != recipient_tag || tick < self.fire_tick {
            return false;
        }
        let close_tick = self.resolved_tick.map_or(self.timeout_tick, |resolved| {
            resolved.min(self.timeout_tick)
        });
        tick < close_tick
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventPopupPresentation {
    pub event: AdmittedFiredEvent,
    pub popup: AdmittedEventPopup,
    pub chrome: EventWindowChrome,
    pub option_entry: EventChromeElement,
    pub option_name: EventChromeElement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventPresentationRejection {
    NotMaterialised,
    NotVisible,
    MissingChrome,
    MissingOptionTemplate,
    InvalidOptionGrid,
}

/// Compose-ready visible state: exact fired-event content joined to the exact
/// authored variant and one vertical 300x47 option row per admitted ordinal.
/// XUI04/XUI06/XUI07 own widget state, text glyphs and runtime graphics.
pub fn event_popup_presentation(
    event: &AdmittedFiredEvent,
    layout: &LayoutSet,
    recipient_tag: &str,
    tick: i64,
) -> std::result::Result<EventPopupPresentation, EventPresentationRejection> {
    let popup = event
        .popup
        .as_ref()
        .ok_or(EventPresentationRejection::NotMaterialised)?;
    if !event.visible_at(recipient_tag, tick) {
        return Err(EventPresentationRejection::NotVisible);
    }
    let chrome = event_window_chrome(layout, popup.window_name)
        .ok_or(EventPresentationRejection::MissingChrome)?;
    if chrome
        .options_slot_size
        .as_ref()
        .map(|(width, height)| (width.as_str(), height.as_str()))
        != Some(("300", "47"))
        || chrome.options_max_slots_horizontal != Some(1)
    {
        return Err(EventPresentationRejection::InvalidOptionGrid);
    }
    let (option_entry, option_name) = event_option_entry_chrome(layout)
        .ok_or(EventPresentationRejection::MissingOptionTemplate)?;
    Ok(EventPopupPresentation {
        event: event.clone(),
        popup: popup.clone(),
        chrome,
        option_entry,
        option_name,
    })
}

pub struct EventHistoryRequest<'a> {
    pub source_record_id: &'a str,
    pub history: &'a FiredEventsView,
    pub definitions: &'a BTreeMap<String, EventDef>,
    pub localisation: &'a Localisation,
    pub sprites: &'a SpriteRegistry,
    /// Exact XUI06/A25 runtime values keyed by `runtime_text_context_id`.
    pub runtime_text: &'a BTreeMap<String, RuntimeTextContext>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventHistoryRejection {
    MissingProvenance,
    HistoryIncomplete,
    HistoryUnavailable,
    InvalidIdentity,
    MissingDefinition,
    InvalidChronology,
    InvalidPopupSubtype,
    InvalidDescriptionBranch,
    InvalidVisibleOptions,
    InvalidSelection,
    InvalidTimeout,
    InvalidResolution,
    PictureUnavailable,
    RuntimeTextContextUnavailable,
    TextUnresolved(Vec<TextIssue>),
}

pub fn event_definitions_by_id(
    corpus: &EventCorpus,
) -> std::result::Result<BTreeMap<String, EventDef>, EventHistoryRejection> {
    let mut out = BTreeMap::new();
    for def in &corpus.defs {
        if def.id.trim().is_empty() || out.insert(def.id.clone(), def.clone()).is_some() {
            return Err(EventHistoryRejection::InvalidIdentity);
        }
    }
    Ok(out)
}

pub fn admit_event_history(
    request: EventHistoryRequest<'_>,
) -> std::result::Result<Vec<AdmittedFiredEvent>, EventHistoryRejection> {
    if request.source_record_id.trim().is_empty() {
        return Err(EventHistoryRejection::MissingProvenance);
    }
    if request.history.history_complete.measured() != Some(&true) {
        return Err(EventHistoryRejection::HistoryIncomplete);
    }
    let rows = request
        .history
        .events
        .measured()
        .ok_or(EventHistoryRejection::HistoryUnavailable)?;
    let mut out = Vec::with_capacity(rows.len());
    let mut prior_sequence = None;
    let mut prior_tick = None;
    let mut identities = BTreeSet::new();
    for row in rows {
        let admitted = admit_fired_event(row, &request)?;
        if prior_sequence.is_some_and(|value| value >= admitted.event_sequence)
            || prior_tick.is_some_and(|value| value > admitted.fire_tick)
            || !identities.insert(admitted.event_sequence)
        {
            return Err(EventHistoryRejection::InvalidChronology);
        }
        prior_sequence = Some(admitted.event_sequence);
        prior_tick = Some(admitted.fire_tick);
        out.push(admitted);
    }
    Ok(out)
}

fn admit_fired_event(
    row: &RecordedFiredEventRow,
    request: &EventHistoryRequest<'_>,
) -> std::result::Result<AdmittedFiredEvent, EventHistoryRejection> {
    let event_id =
        measured_nonempty(&row.event_id).ok_or(EventHistoryRejection::InvalidIdentity)?;
    let recipient_tag =
        measured_nonempty(&row.recipient_tag).ok_or(EventHistoryRejection::InvalidIdentity)?;
    let fire_date =
        measured_nonempty(&row.fire_date).ok_or(EventHistoryRejection::InvalidIdentity)?;
    let fire_hour =
        measured_nonnegative(&row.fire_hour).ok_or(EventHistoryRejection::InvalidChronology)?;
    let fire_tick =
        measured_nonnegative(&row.fire_tick).ok_or(EventHistoryRejection::InvalidChronology)?;
    let event_sequence = measured_nonnegative(&row.event_sequence)
        .ok_or(EventHistoryRejection::InvalidChronology)?;
    let def = request
        .definitions
        .get(event_id)
        .filter(|def| def.id == event_id)
        .ok_or(EventHistoryRejection::MissingDefinition)?;
    let visible_ordinals = match &row.visible_option_ordinals {
        Leaf::Measured(values)
            if values.iter().all(|value| {
                *value >= 0 && usize::try_from(*value).is_ok_and(|i| i < def.options.len())
            }) && !values.windows(2).any(|pair| pair[0] >= pair[1]) =>
        {
            Some(
                values
                    .iter()
                    .map(|value| usize::try_from(*value).expect("validated option ordinal"))
                    .collect::<Vec<_>>(),
            )
        }
        Leaf::Measured(_) => return Err(EventHistoryRejection::InvalidVisibleOptions),
        Leaf::Absent | Leaf::Unobtained => None,
    };
    let visible_set: BTreeSet<usize> = visible_ordinals
        .as_deref()
        .unwrap_or_default()
        .iter()
        .copied()
        .collect();

    let chosen_option_ordinal = match &row.chosen_option_ordinal {
        Leaf::Measured(value) => usize::try_from(*value)
            .ok()
            .filter(|ordinal| *ordinal < def.options.len())
            .filter(|ordinal| visible_set.is_empty() || visible_set.contains(ordinal))
            .ok_or(EventHistoryRejection::InvalidSelection)
            .map(Some)?,
        Leaf::Absent | Leaf::Unobtained => None,
    };

    let effective_timeout_hours = measured_positive(&row.effective_timeout_hours)
        .ok_or(EventHistoryRejection::InvalidTimeout)?;
    if let Some(days) = def.timeout_days {
        let scripted = i64::from(days) * 24;
        if scripted != effective_timeout_hours {
            return Err(EventHistoryRejection::InvalidTimeout);
        }
    }
    let timeout_tick = measured_nonnegative(&row.timeout_tick)
        .filter(|value| *value > fire_tick)
        .ok_or(EventHistoryRejection::InvalidTimeout)?;
    let resolved_tick = match &row.resolved_tick {
        Leaf::Measured(value) if *value >= fire_tick => Some(*value),
        Leaf::Measured(_) => return Err(EventHistoryRejection::InvalidResolution),
        Leaf::Absent | Leaf::Unobtained => None,
    };
    let resolution_reason = measured_nonempty(&row.resolution_reason)
        .ok_or(EventHistoryRejection::InvalidResolution)?;
    match resolution_reason {
        "pending" if resolved_tick.is_none() && chosen_option_ordinal.is_none() => {}
        "selected" if resolved_tick.is_some() && chosen_option_ordinal.is_some() => {}
        "timeout" if resolved_tick.is_some() => {}
        "dismissed" if resolved_tick.is_some() && chosen_option_ordinal.is_none() => {}
        _ => return Err(EventHistoryRejection::InvalidResolution),
    }

    let runtime_text_context_id =
        measured_nonempty(&row.runtime_text_context_id).map(str::to_string);
    let popup = admit_popup_materialisation(
        row,
        def,
        visible_ordinals.as_deref(),
        chosen_option_ordinal,
        runtime_text_context_id.as_deref(),
        request,
    );

    Ok(AdmittedFiredEvent {
        event_id: event_id.to_string(),
        recipient_tag: recipient_tag.to_string(),
        fire_date: fire_date.to_string(),
        fire_hour,
        fire_tick,
        event_sequence,
        definition_hidden: def.hidden,
        chosen_option_ordinal,
        effective_timeout_hours,
        timeout_tick,
        resolved_tick,
        resolution_reason: resolution_reason.to_string(),
        runtime_text_context_id,
        popup,
    })
}

/// Admit popup presentation only from a complete exact materialisation tuple.
/// Failure here suppresses presentation without discarding the authenticated
/// lifecycle row. This is how hidden, AI-only, zero-description and otherwise
/// non-renderable events remain part of complete history without fabricated UI.
fn admit_popup_materialisation(
    row: &RecordedFiredEventRow,
    def: &EventDef,
    visible_ordinals: Option<&[usize]>,
    chosen_option_ordinal: Option<usize>,
    runtime_text_context_id: Option<&str>,
    request: &EventHistoryRequest<'_>,
) -> Option<AdmittedEventPopup> {
    if def.hidden {
        return None;
    }
    let measured_desc = measured_nonempty(&row.selected_desc_key);
    let measured_picture = measured_nonempty(&row.shown_picture_gfx);
    let visible_ordinals = visible_ordinals?;
    // A subtype alone is not a materialisation witness: lifecycle producers
    // may carry a class/default value for AI-only or hidden firings.
    if measured_desc.is_none() && measured_picture.is_none() && visible_ordinals.is_empty() {
        return None;
    }
    let subtype = measured_nonempty(&row.popup_subtype)
        .and_then(EventPopupSubtype::parse)
        .filter(|subtype| subtype.compatible_with(def.kind))?;
    let selected_desc_key = match measured_desc {
        Some(key) if def.desc_keys.iter().any(|candidate| candidate == key) => key.to_string(),
        Some(_) => return None,
        None if def.desc_keys.len() == 1 => def.desc_keys[0].clone(),
        None => return None,
    };
    let visible_set: BTreeSet<usize> = visible_ordinals.iter().copied().collect();
    if def
        .options
        .iter()
        .enumerate()
        .any(|(ordinal, option)| !option.has_trigger && !visible_set.contains(&ordinal))
        || chosen_option_ordinal.is_some_and(|ordinal| !visible_set.contains(&ordinal))
    {
        return None;
    }
    let empty_runtime = RuntimeTextContext::default();
    let runtime = match runtime_text_context_id {
        Some(id) => request.runtime_text.get(id)?,
        None => &empty_runtime,
    };
    let title =
        resolve_exact_text(request.localisation, def.title_key.as_deref()?, runtime).ok()?;
    let description = resolve_exact_text(request.localisation, &selected_desc_key, runtime).ok()?;
    let mut options = Vec::with_capacity(visible_ordinals.len());
    for &ordinal in visible_ordinals {
        let key = def.options[ordinal].name_key.as_deref()?;
        options.push(AdmittedEventOption {
            definition_ordinal: ordinal,
            text: resolve_exact_text(request.localisation, key, runtime).ok()?,
            selected: chosen_option_ordinal == Some(ordinal),
        });
    }
    // The authenticated runtime result may be a country-specific variant of
    // the authored base key. Exact sprite existence is the replay-side gate;
    // equality with the base key would incorrectly reject the proven variant.
    let picture_gfx = measured_picture
        .map(str::to_string)
        .or_else(|| def.picture_gfx.clone())
        .unwrap_or_else(|| subtype.default_picture().to_string());
    request.sprites.get(&picture_gfx)?;
    Some(AdmittedEventPopup {
        subtype,
        window_name: subtype.window_name(),
        title,
        description,
        selected_desc_key,
        picture_gfx,
        options,
    })
}

fn resolve_exact_text(
    localisation: &Localisation,
    key: &str,
    runtime: &RuntimeTextContext,
) -> std::result::Result<String, EventHistoryRejection> {
    let resolved = resolve_localisation(&localisation.entries, key, runtime);
    if resolved.admitted() {
        Ok(resolved.text)
    } else {
        Err(EventHistoryRejection::TextUnresolved(resolved.issues))
    }
}

fn measured_nonempty(value: &Leaf<String>) -> Option<&str> {
    value
        .measured()
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn measured_nonnegative(value: &Leaf<i64>) -> Option<i64> {
    value.measured().copied().filter(|value| *value >= 0)
}

fn measured_positive(value: &Leaf<i64>) -> Option<i64> {
    value.measured().copied().filter(|value| *value > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paradox_text::parse_str;

    fn sample_events_doc() -> Document {
        parse_str(
            r#"
            add_namespace = generic
            country_event = {
                id = generic.1
                title = generic.1.t
                desc = { text = generic.1.d_good trigger = { has_government = FROM } }
                desc = { text = generic.1.d_bad trigger = { NOT = { has_government = FROM } } }
                picture = GFX_report_event_generic_read_write
                is_triggered_only = yes
                option = { name = generic.1.a }
                option = { name = generic.1.b }
                immediate = {
                    country_event = { id = generic.99 days = 5 }
                }
            }
            news_event = {
                id = news.1
                title = news.1.t
                desc = news.1.d
                picture = GFX_news_event_001
                major = yes
                option = { name = news.1.a }
            }
            country_event = {
                id = hidden.1
                title = hidden.1.t
                hidden = yes
                option = { name = hidden.1.a }
            }
            "#,
        )
    }

    #[test]
    fn parses_root_defs_and_ignores_nested_firing_effects() {
        let defs = parse_events_document("events/test.txt", &sample_events_doc());
        // 3 root defs; the nested `country_event` firing effect is NOT counted.
        assert_eq!(defs.len(), 3);
        let generic = &defs[0];
        assert_eq!(generic.id, "generic.1");
        assert_eq!(generic.kind, EventKind::Country);
        assert_eq!(generic.title_key.as_deref(), Some("generic.1.t"));
        assert_eq!(
            generic.desc_keys,
            vec![
                "generic.1.d_good".to_string(),
                "generic.1.d_bad".to_string()
            ]
        );
        assert_eq!(
            generic.picture_gfx.as_deref(),
            Some("GFX_report_event_generic_read_write")
        );
        assert_eq!(generic.options.len(), 2);
        assert!(!generic.hidden);
        assert_eq!(generic.window_name(), "EventWindow");
    }

    #[test]
    fn news_event_maps_to_news_window_and_reads_bare_desc() {
        let defs = parse_events_document("events/test.txt", &sample_events_doc());
        let news = defs.iter().find(|d| d.id == "news.1").unwrap();
        assert_eq!(news.kind, EventKind::News);
        assert_eq!(news.window_name(), "EventWindow_News");
        assert_eq!(news.desc_keys, vec!["news.1.d".to_string()]);
        assert!(news.major);
    }

    #[test]
    fn hidden_event_excluded_from_renderable_denominator() {
        let corpus = EventCorpus {
            defs: parse_events_document("events/test.txt", &sample_events_doc()),
            files_parsed: 1,
            parse_failures: Vec::new(),
        };
        let mut loc = Localisation::default();
        for key in [
            "generic.1.t",
            "generic.1.d_good",
            "generic.1.a",
            "generic.1.b",
            "news.1.t",
            "news.1.d",
            "news.1.a",
            "hidden.1.t",
            "hidden.1.a",
        ] {
            loc.entries.insert(key.to_string(), format!("[{key}]"));
        }
        let mut registry = SpriteRegistry::new();
        for gfx in ["GFX_report_event_generic_read_write", "GFX_news_event_001"] {
            registry.insert(sprite_stub(gfx));
        }
        let report = resolve_events(&corpus, &loc, &registry);
        assert_eq!(report.total, 3);
        assert_eq!(report.hidden, 1);
        assert_eq!(report.visible, 2);
        // Both visible events fully resolve.
        assert_eq!(report.renderable, 2);
        assert_eq!(report.title_resolved, 2);
        assert_eq!(report.desc_resolved, 2);
        assert_eq!(report.picture_resolved, 2);
        // Country=1 renderable, News=1 renderable.
        assert_eq!(report.renderable_by_kind[0], 1);
        assert_eq!(report.renderable_by_kind[1], 1);
    }

    #[test]
    fn missing_picture_sprite_blocks_renderable() {
        let corpus = EventCorpus {
            defs: parse_events_document("events/test.txt", &sample_events_doc()),
            files_parsed: 1,
            parse_failures: Vec::new(),
        };
        let mut loc = Localisation::default();
        for key in ["news.1.t", "news.1.d", "news.1.a"] {
            loc.entries.insert(key.to_string(), format!("[{key}]"));
        }
        // Empty registry → declared picture GFX unresolved → not renderable.
        let registry = SpriteRegistry::new();
        let report = resolve_events(&corpus, &loc, &registry);
        assert!(report.picture_declared >= 1);
        assert_eq!(report.picture_resolved, 0);
        assert_eq!(report.renderable, 0);
    }

    #[test]
    fn fired_event_content_is_fail_closed_without_stream() {
        let defs = parse_events_document("events/test.txt", &sample_events_doc());
        let generic = &defs[0];
        let mut loc = Localisation::default();
        loc.entries
            .insert("generic.1.t".to_string(), "A Title".to_string());
        loc.entries
            .insert("generic.1.d_good".to_string(), "Good desc".to_string());
        loc.entries
            .insert("generic.1.a".to_string(), "Option A".to_string());
        loc.entries
            .insert("generic.1.b".to_string(), "Option B".to_string());

        // No fired-event → fail-closed empty (no fabricated title).
        let absent = event_window_content(generic, &loc, None);
        assert!(!absent.available);
        assert_eq!(absent.title, None);
        assert!(absent.options.is_empty());

        // Fired-event → bound content (first resolvable desc; window by kind).
        let fired = FiredEvent {
            event_id: "generic.1".to_string(),
            target_tag: Some("GER".to_string()),
            chosen_option_index: Some(1),
            fire_date: Some("1936.1.1.12".to_string()),
        };
        let bound = event_window_content(generic, &loc, Some(&fired));
        assert!(bound.available);
        assert_eq!(bound.window_name.as_deref(), Some("EventWindow"));
        assert_eq!(bound.title.as_deref(), Some("A Title"));
        assert_eq!(bound.description.as_deref(), Some("Good desc"));
        assert_eq!(
            bound.options,
            vec!["Option A".to_string(), "Option B".to_string()]
        );
        assert_eq!(bound.chosen_option_index, Some(1));
    }

    fn rich_row() -> RecordedFiredEventRow {
        RecordedFiredEventRow {
            event_id: Leaf::Measured("generic.1".into()),
            recipient_tag: Leaf::Measured("GER".into()),
            fire_date: Leaf::Measured("1936.1.1.12".into()),
            fire_hour: Leaf::Measured(12),
            fire_tick: Leaf::Measured(1200),
            event_sequence: Leaf::Measured(4),
            visible_option_ordinals: Leaf::Measured(vec![0, 1]),
            chosen_option_ordinal: Leaf::Measured(1),
            selected_desc_key: Leaf::Measured("generic.1.d_good".into()),
            shown_picture_gfx: Leaf::Measured("GFX_report_event_generic_read_write".into()),
            popup_subtype: Leaf::Measured("country".into()),
            effective_timeout_hours: Leaf::Measured(312),
            timeout_tick: Leaf::Measured(2000),
            resolved_tick: Leaf::Measured(1210),
            resolution_reason: Leaf::Measured("selected".into()),
            runtime_text_context_id: Leaf::Absent,
        }
    }

    #[test]
    fn xui21_admits_exact_history_and_visible_presentation() {
        let defs = parse_events_document("events/test.txt", &sample_events_doc());
        let definitions = event_definitions_by_id(&EventCorpus {
            defs,
            files_parsed: 1,
            parse_failures: Vec::new(),
        })
        .unwrap();
        let history = FiredEventsView {
            events: Leaf::Measured(vec![rich_row()]),
            history_complete: Leaf::Measured(true),
        };
        let mut loc = Localisation::default();
        for (key, text) in [
            ("generic.1.t", "A Title"),
            ("generic.1.d_good", "Good desc"),
            ("generic.1.a", "Option A"),
            ("generic.1.b", "Option B"),
        ] {
            loc.entries.insert(key.into(), text.into());
        }
        let mut sprites = SpriteRegistry::new();
        sprites.insert(sprite_stub("GFX_report_event_generic_read_write"));
        let admitted = admit_event_history(EventHistoryRequest {
            source_record_id: "record:1",
            history: &history,
            definitions: &definitions,
            localisation: &loc,
            sprites: &sprites,
            runtime_text: &BTreeMap::new(),
        })
        .expect("admit exact fired-event history");
        assert_eq!(admitted.len(), 1);
        let event = &admitted[0];
        let popup = event.popup.as_ref().expect("materialised popup");
        assert_eq!(popup.window_name, "EventWindow");
        assert_eq!(popup.selected_desc_key, "generic.1.d_good");
        assert_eq!(popup.options.len(), 2);
        assert_eq!(popup.options[0].definition_ordinal, 0);
        assert!(!popup.options[0].selected);
        assert!(popup.options[1].selected);
        assert!(event.visible_at("GER", 1200));
        assert!(event.visible_at("GER", 1209));
        assert!(!event.visible_at("GER", 1210));
        assert!(!event.visible_at("SOV", 1205));

        let mut layout = LayoutSet::default();
        layout.add_document(
            "interface/eventwindow.gui",
            &parse_str(
                r#"guiTypes = {
                    containerWindowType = {
                        name = "EventWindow"
                        size = { width = 581 height = 427 }
                        gridBoxType = {
                            name = "options_grid"
                            size = { width = 300 height = 30 }
                            slotsize = { width = 300 height = 47 }
                            max_slots_horizontal = 1
                        }
                    }
                    containerWindowType = {
                        name = "event_option_entry"
                        size = { width = 300 height = 47 }
                        instantTextBoxType = { name = "Name" font = "hoi_20bs" }
                    }
                }"#,
            ),
        );
        let presentation = event_popup_presentation(event, &layout, "GER", 1205)
            .expect("compose-ready popup presentation");
        assert_eq!(
            presentation.chrome.options_slot_size,
            Some(("300".into(), "47".into()))
        );
        assert_eq!(
            presentation.option_entry.size,
            Some(("300".into(), "47".into()))
        );
        assert_eq!(presentation.popup.options.len(), 2);
        assert_eq!(
            event_popup_presentation(event, &layout, "GER", 1210),
            Err(EventPresentationRejection::NotVisible)
        );
    }

    #[test]
    fn xui21_rejects_legacy_or_inferred_history() {
        let history = FiredEventsView {
            events: Leaf::Measured(vec![RecordedFiredEventRow {
                event_id: Leaf::Measured("generic.1".into()),
                ..rich_row()
            }]),
            history_complete: Leaf::Measured(false),
        };
        let result = admit_event_history(EventHistoryRequest {
            source_record_id: "record:1",
            history: &history,
            definitions: &BTreeMap::new(),
            localisation: &Localisation::default(),
            sprites: &SpriteRegistry::new(),
            runtime_text: &BTreeMap::new(),
        });
        assert_eq!(result, Err(EventHistoryRejection::HistoryIncomplete));
    }

    #[test]
    fn xui21_keeps_hidden_and_ai_rows_as_lifecycle_without_fabricated_popups() {
        let defs = parse_events_document("events/test.txt", &sample_events_doc());
        let definitions = event_definitions_by_id(&EventCorpus {
            defs,
            files_parsed: 1,
            parse_failures: Vec::new(),
        })
        .unwrap();
        let mut hidden = rich_row();
        hidden.event_id = Leaf::Measured("hidden.1".into());
        hidden.event_sequence = Leaf::Measured(4);
        hidden.visible_option_ordinals = Leaf::Measured(Vec::new());
        hidden.chosen_option_ordinal = Leaf::Measured(0);
        hidden.selected_desc_key = Leaf::Absent;
        hidden.shown_picture_gfx = Leaf::Absent;
        let mut ai = rich_row();
        ai.event_sequence = Leaf::Measured(5);
        ai.visible_option_ordinals = Leaf::Measured(Vec::new());
        ai.selected_desc_key = Leaf::Absent;
        ai.shown_picture_gfx = Leaf::Absent;
        let history = FiredEventsView {
            events: Leaf::Measured(vec![hidden, ai]),
            history_complete: Leaf::Measured(true),
        };
        let admitted = admit_event_history(EventHistoryRequest {
            source_record_id: "record:lifecycle",
            history: &history,
            definitions: &definitions,
            localisation: &Localisation::default(),
            sprites: &SpriteRegistry::new(),
            runtime_text: &BTreeMap::new(),
        })
        .expect("admit exact lifecycle rows");
        assert_eq!(admitted.len(), 2);
        assert!(admitted[0].definition_hidden);
        assert!(admitted.iter().all(|event| event.popup.is_none()));
        assert_eq!(admitted[1].chosen_option_ordinal, Some(1));
        assert_eq!(
            event_popup_presentation(&admitted[1], &LayoutSet::default(), "GER", 1205),
            Err(EventPresentationRejection::NotMaterialised)
        );
    }

    #[test]
    fn xui21_admits_zero_option_and_no_desc_lifecycle_separately() {
        let doc = parse_str(
            r#"
            country_event = {
                id = generic.2
                title = generic.2.t
                desc = generic.2.d
                picture = GFX_report_event_zero
            }
            country_event = {
                id = generic.3
                title = generic.3.t
                picture = GFX_report_event_no_desc
                option = { name = generic.3.a }
            }
            "#,
        );
        let definitions = event_definitions_by_id(&EventCorpus {
            defs: parse_events_document("events/edges.txt", &doc),
            files_parsed: 1,
            parse_failures: Vec::new(),
        })
        .unwrap();
        let mut zero = rich_row();
        zero.event_id = Leaf::Measured("generic.2".into());
        zero.visible_option_ordinals = Leaf::Measured(Vec::new());
        zero.chosen_option_ordinal = Leaf::Absent;
        zero.selected_desc_key = Leaf::Measured("generic.2.d".into());
        zero.shown_picture_gfx = Leaf::Measured("GFX_report_event_zero".into());
        zero.resolution_reason = Leaf::Measured("timeout".into());
        let mut no_desc = rich_row();
        no_desc.event_id = Leaf::Measured("generic.3".into());
        no_desc.event_sequence = Leaf::Measured(5);
        no_desc.visible_option_ordinals = Leaf::Measured(vec![0]);
        no_desc.chosen_option_ordinal = Leaf::Measured(0);
        no_desc.selected_desc_key = Leaf::Absent;
        no_desc.shown_picture_gfx = Leaf::Measured("GFX_report_event_no_desc".into());
        let history = FiredEventsView {
            events: Leaf::Measured(vec![zero, no_desc]),
            history_complete: Leaf::Measured(true),
        };
        let mut loc = Localisation::default();
        for (key, text) in [
            ("generic.2.t", "Zero title"),
            ("generic.2.d", "Zero description"),
            ("generic.3.t", "No desc title"),
            ("generic.3.a", "Only option"),
        ] {
            loc.entries.insert(key.into(), text.into());
        }
        let mut sprites = SpriteRegistry::new();
        sprites.insert(sprite_stub("GFX_report_event_zero"));
        sprites.insert(sprite_stub("GFX_report_event_no_desc"));
        let admitted = admit_event_history(EventHistoryRequest {
            source_record_id: "record:edges",
            history: &history,
            definitions: &definitions,
            localisation: &loc,
            sprites: &sprites,
            runtime_text: &BTreeMap::new(),
        })
        .expect("admit edge lifecycle rows");
        assert!(admitted[0]
            .popup
            .as_ref()
            .is_some_and(|popup| popup.options.is_empty()));
        assert!(admitted[1].popup.is_none());
    }

    #[test]
    fn xui21_accepts_timeout_default_ordinal_and_runtime_picture_variant() {
        let defs = parse_events_document("events/test.txt", &sample_events_doc());
        let definitions = event_definitions_by_id(&EventCorpus {
            defs,
            files_parsed: 1,
            parse_failures: Vec::new(),
        })
        .unwrap();
        let mut row = rich_row();
        row.chosen_option_ordinal = Leaf::Measured(0);
        row.resolution_reason = Leaf::Measured("timeout".into());
        row.shown_picture_gfx = Leaf::Measured("GFX_report_event_GER_generic_read_write".into());
        let history = FiredEventsView {
            events: Leaf::Measured(vec![row]),
            history_complete: Leaf::Measured(true),
        };
        let mut loc = Localisation::default();
        for (key, text) in [
            ("generic.1.t", "A Title"),
            ("generic.1.d_good", "Good desc"),
            ("generic.1.a", "Option A"),
            ("generic.1.b", "Option B"),
        ] {
            loc.entries.insert(key.into(), text.into());
        }
        let mut sprites = SpriteRegistry::new();
        sprites.insert(sprite_stub("GFX_report_event_GER_generic_read_write"));
        let admitted = admit_event_history(EventHistoryRequest {
            source_record_id: "record:timeout-variant",
            history: &history,
            definitions: &definitions,
            localisation: &loc,
            sprites: &sprites,
            runtime_text: &BTreeMap::new(),
        })
        .expect("admit timeout default and runtime picture variant");
        let popup = admitted[0].popup.as_ref().expect("materialised popup");
        assert_eq!(admitted[0].resolution_reason, "timeout");
        assert_eq!(admitted[0].chosen_option_ordinal, Some(0));
        assert!(popup.options[0].selected);
        assert_eq!(popup.picture_gfx, "GFX_report_event_GER_generic_read_write");
    }

    fn sprite_stub(name: &str) -> crate::gfx::SpriteDefinition {
        crate::gfx::SpriteDefinition {
            kind: "spriteType".to_string(),
            name: name.to_string(),
            texture_files: Vec::new(),
            no_of_frames: 1,
            effect_file: None,
            size: None,
            border_size: None,
            source_file: "test.gfx".to_string(),
            raw: parse_str("x = 1").root,
        }
    }
}
