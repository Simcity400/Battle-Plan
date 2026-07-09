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
use crate::Result;

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
    let renderable =
        title_resolved && desc_resolved && picture_ok && options_resolved >= 1;
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
        max_width: element
            .raw
            .first_scalar_ci("maxWidth")
            .map(str::to_string),
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
    let background_sprite = find_child(window, "Background")
        .and_then(|bg| bg.sprite_refs.first().cloned());
    Some(EventWindowChrome {
        window_name: window.name.clone(),
        window_size: raw_pair(&window.size),
        background_sprite,
        title: find_child(window, "Title").map(chrome_element),
        description: find_child(window, "Description").map(chrome_element),
        picture: find_child(window, "event_picture").map(chrome_element),
        options_grid: find_child(window, "options_grid").map(chrome_element),
    })
}

/// The shared option-row template (`event_option_entry`) and its `Name`
/// text-box, located from a parsed layout.
pub fn event_option_entry_chrome(layout: &LayoutSet) -> Option<(EventChromeElement, EventChromeElement)> {
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
            vec!["generic.1.d_good".to_string(), "generic.1.d_bad".to_string()]
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
        assert_eq!(bound.options, vec!["Option A".to_string(), "Option B".to_string()]);
        assert_eq!(bound.chosen_option_index, Some(1));
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
