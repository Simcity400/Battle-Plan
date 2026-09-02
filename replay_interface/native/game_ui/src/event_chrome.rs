//! lifetime ledger plus independent A25 title/description/option text contexts

use crate::assets::AssetResolver;
use crate::compose::{ComposeOptions, ComposeOutput, Compositor, UiAssetCache};
use crate::gfx::SpriteRegistry;
use crate::gui::{GuiElement, LayoutSet};
use crate::gui_scene_util::{pair, prefix_element_names, set_position_upper_left};
use crate::loc::Localisation;
use crate::paradox_text::{
    decode_text, destructive_event_count, diagnose_text_decode, parse_bytes, Document, Value,
};
use crate::text_law::{
    resolve_element_bound_exact_runtime, resolve_localisation, RuntimeTextContext, TextIssue,
};
use crate::{GameUiError, Result};
use replay_core::{
    FiredEventRow as RecordedFiredEventRow, FiredEventsView, Leaf, ValidatedContentAdmission,
};
use std::collections::{BTreeMap, BTreeSet};

pub const EVENT_WINDOW_GUI: &str = "interface/eventwindow.gui";

pub const EVENT_WINDOW_VARIANTS: [&str; 4] = [
    "EventWindow",
    "EventWindow_Operative",
    "EventWindow_leader",
    "EventWindow_News",
];

pub const EVENT_OPTION_ENTRY: &str = "event_option_entry";

pub const EVENT_DEFINITION_NAMESPACE_CONSUMER: &str =
    "replay.xui21.event_definition_effective_namespace.complete.v1";


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Country,
    News,
    State,
    UnitLeader,
    OperativeLeader,
}

impl EventKind {
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

    pub fn keyword(&self) -> &'static str {
        match self {
            EventKind::Country => "country_event",
            EventKind::News => "news_event",
            EventKind::State => "state_event",
            EventKind::UnitLeader => "unit_leader_event",
            EventKind::OperativeLeader => "operative_leader_event",
        }
    }

    pub fn window_name(&self) -> &'static str {
        match self {
            EventKind::News => "EventWindow_News",
            EventKind::UnitLeader => "EventWindow_leader",
            EventKind::OperativeLeader => "EventWindow_Operative",
            EventKind::Country | EventKind::State => "EventWindow",
        }
    }
}


#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventOption {
    pub name_key: Option<String>,
    pub has_trigger: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventDef {
    pub id: String,
    pub kind: EventKind,
    pub title_key: Option<String>,
    pub desc_keys: Vec<String>,
    pub picture_gfx: Option<String>,
    pub options: Vec<EventOption>,
    pub hidden: bool,
    pub major: bool,
    pub timeout_days: Option<u32>,
    pub source_file: String,
}

impl EventDef {
    pub fn window_name(&self) -> &'static str {
        self.kind.window_name()
    }
}

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventCorpus {
    pub defs: Vec<EventDef>,
    pub files_parsed: usize,
    pub parse_failures: Vec<String>,
    pub decode_lossy_files: Vec<String>,
}

pub fn load_events(resolver: &AssetResolver) -> Result<EventCorpus> {
    let files = resolver.list_files_recursive("events", Some("txt"))?;
    let mut corpus = EventCorpus::default();
    for file in files {
        let bytes = match std::fs::read(&file.path) {
            Ok(bytes) => bytes,
            Err(err) => {
                corpus
                    .parse_failures
                    .push(format!("{}: {}", file.rel_path, err));
                continue;
            }
        };
        let decode_events = destructive_event_count(&diagnose_text_decode(&bytes));
        if decode_events != 0 {
            corpus.decode_lossy_files.push(format!(
                "{}: {decode_events} destructive decode events",
                file.rel_path
            ));
            continue;
        }
        corpus.files_parsed += 1;
        let doc = parse_bytes(&bytes);
        corpus
            .defs
            .extend(parse_events_document(&file.rel_path, &doc));
    }
    Ok(corpus)
}


#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventResolution {
    pub id: String,
    pub kind: EventKind,
    pub window_name: &'static str,
    pub title_resolved: bool,
    pub desc_resolved: bool,
    pub picture_declared: bool,
    pub picture_resolved: bool,
    pub options_total: usize,
    pub options_resolved: usize,
    pub renderable: bool,
}

fn loc_present(loc: &Localisation, key: &str) -> bool {
    loc.get(key).is_some_and(|value| !value.trim().is_empty())
}

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventResolutionReport {
    pub total: usize,
    pub hidden: usize,
    pub visible: usize,
    pub title_resolved: usize,
    pub desc_resolved: usize,
    pub picture_declared: usize,
    pub picture_resolved: usize,
    pub options_resolved_any: usize,
    pub renderable: usize,
    pub visible_by_kind: [usize; 5],
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


#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventChromeElement {
    pub name: String,
    pub element_type: String,
    pub position: Option<(String, String)>,
    pub size: Option<(String, String)>,
    pub sprite: Option<String>,
    pub font: Option<String>,
    pub format: Option<String>,
    pub max_width: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventWindowChrome {
    pub window_name: String,
    pub window_size: Option<(String, String)>,
    pub background_sprite: Option<String>,
    pub title: Option<EventChromeElement>,
    pub description: Option<EventChromeElement>,
    pub picture: Option<EventChromeElement>,
    pub options_grid: Option<EventChromeElement>,
    pub options_slot_size: Option<(String, String)>,
    pub options_max_slots_horizontal: Option<u32>,
}

fn raw_pair(pair: &Option<crate::gui::MeasurePair>) -> Option<(String, String)> {
    pair.as_ref().map(|p| (p.x.raw.clone(), p.y.raw.clone()))
}

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

fn find_child<'a>(root: &'a GuiElement, name: &str) -> Option<&'a GuiElement> {
    let mut hits = Vec::new();
    root.find_by_name(name, &mut hits);
    hits.into_iter().next()
}

pub fn load_event_layout(resolver: &AssetResolver) -> Result<LayoutSet> {
    let mut layouts = LayoutSet::default();
    let mut authority_recovery_files = Vec::new();
    for file in resolver.list_files_recursive("interface", Some("gui"))? {
        let bytes = std::fs::read(&file.path).map_err(|err| {
            GameUiError::parse(format!(
                "XUI21 effective GUI {} could not be read: {err}",
                file.rel_path
            ))
        })?;
        let contains_event_authority_name = {
            let text = decode_text(&bytes).to_ascii_lowercase();
            text.contains("eventwindow") || text.contains("event_option_entry")
        };
        layouts.parse_file(&file.path, &file.rel_path).map_err(|err| {
            GameUiError::parse(format!(
                "XUI21 effective GUI {} could not be read/parsed: {}",
                file.rel_path, err.message
            ))
        })?;
        if contains_event_authority_name
            && layouts
                .documents
                .last()
                .is_some_and(|document| !document.recoveries.is_empty())
        {
            authority_recovery_files.push(file.rel_path);
        }
    }
    if layouts.stats.parse_failures != 0
        || layouts.stats.decode_loss_count != 0
        || !authority_recovery_files.is_empty()
    {
        return Err(GameUiError::parse(format!(
            "XUI21 effective GUI corpus can hide event authority: parse_failures={} authority_recovery_files={:?} all_recoveries={} decode_losses={} files_with_recoveries={:?} files_with_decode_loss={:?}",
            layouts.stats.parse_failures,
            authority_recovery_files,
            layouts.stats.recovery_count,
            layouts.stats.decode_loss_count,
            layouts.stats.files_with_recoveries,
            layouts.stats.files_with_decode_loss,
        )));
    }
    Ok(layouts)
}

fn is_event_definition_path(path: &str) -> bool {
    let folded = path.replace('\\', "/").to_ascii_lowercase();
    folded.starts_with("events/")
        && folded.len() > "events/.txt".len()
        && folded.ends_with(".txt")
}

fn validate_event_definition_namespace_sets(
    declared_paths: &BTreeSet<String>,
    admitted_paths: &BTreeSet<String>,
    resolved_paths: &BTreeSet<String>,
) -> std::result::Result<(), String> {
    if declared_paths.is_empty() {
        return Err("XUI21 complete event-definition namespace is empty".into());
    }
    if declared_paths != admitted_paths {
        return Err(format!(
            "XUI21 event-definition capability differs from admitted event rows: declared={declared_paths:?} admitted={admitted_paths:?}"
        ));
    }
    if declared_paths != resolved_paths {
        return Err(format!(
            "XUI21 event-definition capability differs from the effective restricted VFS: declared={declared_paths:?} resolved={resolved_paths:?}"
        ));
    }
    Ok(())
}

pub fn load_admitted_event_definitions(
    resolver: &AssetResolver,
    admission: &ValidatedContentAdmission,
) -> std::result::Result<BTreeMap<String, EventDef>, String> {
    let declared = admission
        .require_used_content_for_consumer(EVENT_DEFINITION_NAMESPACE_CONSUMER)
        .map_err(|error| format!("XUI21 event-definition capability unavailable: {error}"))?;
    let mut declared_paths = BTreeSet::new();
    for content in declared {
        let path = content
            .normalized_game_relative_path
            .replace('\\', "/")
            .to_ascii_lowercase();
        if !is_event_definition_path(&path) {
            return Err(format!(
                "XUI21 event-definition capability contains out-of-namespace row {path:?}"
            ));
        }
        if !declared_paths.insert(path.clone()) {
            return Err(format!(
                "XUI21 event-definition capability repeats case-folded path {path:?}"
            ));
        }
    }
    let admitted_paths = admission
        .required_paths()
        .filter(|path| is_event_definition_path(path))
        .map(|path| path.replace('\\', "/").to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let resolved_paths = resolver
        .list_files_recursive("events", Some("txt"))
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|file| file.rel_path.replace('\\', "/").to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    validate_event_definition_namespace_sets(
        &declared_paths,
        &admitted_paths,
        &resolved_paths,
    )?;

    let corpus = load_events(resolver).map_err(|error| error.to_string())?;
    if !corpus.parse_failures.is_empty() || !corpus.decode_lossy_files.is_empty() {
        return Err(format!(
            "effective event corpus is not lossless: parse_failures={:?}; decode_loss={:?}",
            corpus.parse_failures, corpus.decode_lossy_files
        ));
    }
    event_definitions_by_id(&corpus)
        .map_err(|error| format!("effective event identities rejected: {error:?}"))
}

pub fn event_window_chrome(layout: &LayoutSet, window_name: &str) -> Option<EventWindowChrome> {
    let mut windows = layout
        .find_by_name(window_name)
        .into_iter()
        .filter(|e| e.element_type.eq_ignore_ascii_case("containerWindowType"));
    let window = windows.next()?;
    if windows.next().is_some() {
        return None;
    }
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

pub fn event_option_entry_chrome(
    layout: &LayoutSet,
) -> Option<(EventChromeElement, EventChromeElement)> {
    let mut entries = layout
        .find_by_name(EVENT_OPTION_ENTRY)
        .into_iter()
        .filter(|e| e.element_type.eq_ignore_ascii_case("containerWindowType"));
    let entry = entries.next()?;
    if entries.next().is_some() {
        return None;
    }
    let name = find_child(entry, "Name")?;
    Some((chrome_element(entry), chrome_element(name)))
}


#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FiredEvent {
    pub event_id: String,
    pub target_tag: Option<String>,
    pub chosen_option_index: Option<usize>,
    pub fire_date: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventWindowContent {
    pub available: bool,
    pub window_name: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub picture_gfx: Option<String>,
    pub options: Vec<String>,
    pub chosen_option_index: Option<usize>,
}

pub fn event_window_content(
    def: &EventDef,
    loc: &Localisation,
    fired: Option<&FiredEvent>,
) -> EventWindowContent {
    let Some(fired) = fired else {
        return EventWindowContent::default();
    };
    if fired.event_id != def.id {
        // Loud in debug/test builds; fail-closed (absent form) in release —
        debug_assert_eq!(fired.event_id, def.id, "fired-event id must match def id");
        return EventWindowContent::default();
    }
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


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventPopupSubtype {
    Country,
    News,
    State,
    UnitLeader,
    OperativeLeader,
}

impl EventPopupSubtype {
    fn from_producer_token(value: &str) -> Option<Self> {
        match value {
            "country" => Some(Self::Country),
            "news" => Some(Self::News),
            "state" => Some(Self::State),
            "unit_leader" => Some(Self::UnitLeader),
            "operative_leader" => Some(Self::OperativeLeader),
            _ => None,
        }
    }

    fn matches_definition(self, kind: EventKind) -> bool {
        matches!(
            (self, kind),
            (Self::Country, EventKind::Country)
                | (Self::News, EventKind::News)
                | (Self::State, EventKind::State)
                | (Self::UnitLeader, EventKind::UnitLeader)
                | (Self::OperativeLeader, EventKind::OperativeLeader)
        )
    }

    pub fn window_name(self) -> &'static str {
        match self {
            Self::News => "EventWindow_News",
            Self::UnitLeader => "EventWindow_leader",
            Self::OperativeLeader => "EventWindow_Operative",
            Self::Country | Self::State => "EventWindow",
        }
    }

}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedEventOption {
    pub definition_ordinal: usize,
    pub text: String,
    pub runtime_text: RuntimeTextContext,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventRuntimeImageIdentity {
    GfxName(String),
    FilePath(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedEventPopup {
    pub subtype: EventPopupSubtype,
    pub window_name: &'static str,
    pub title: String,
    pub title_runtime_text: RuntimeTextContext,
    pub description: String,
    pub description_runtime_text: RuntimeTextContext,
    pub selected_desc_key: String,
    pub picture_gfx: Option<String>,
    pub leader_portrait: Option<EventRuntimeImageIdentity>,
    pub leader_fullname: Option<(String, RuntimeTextContext)>,
    pub leader_codename: Option<(String, RuntimeTextContext)>,
    pub agency_insignia: Option<EventRuntimeImageIdentity>,
    pub options: Vec<AdmittedEventOption>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupSuppression {
    DefinitionHidden,
    VisibleOptionOrdinalsAbsent,
    VisibleOptionOrdinalsUnobtained,
    VisibleOptionOrdinalsEmpty,
    DescriptionBranchNotDeclared,
    DescriptionBranchUndetermined,
    WindowVariantUndetermined,
    PictureSelectionUnavailable,
    UnconditionalOptionNotVisible,
    ChosenOptionNotVisible,
    TitleRuntimeTextContextUnavailable,
    DescriptionRuntimeTextContextUnavailable,
    OptionRuntimeTextContextsUnavailable,
    OptionRuntimeTextContextNotVisible,
    TitleRuntimeTextContextMissing,
    DescriptionRuntimeTextContextMissing,
    OptionRuntimeTextContextMissing,
    TitleKeyMissing,
    TitleTextUnresolved,
    DescriptionTextUnresolved,
    OptionNameMissing,
    OptionTextUnresolved,
    PictureSpriteUnavailable,
    LeaderPortraitUnavailable,
    LeaderPortraitSpriteUnavailable,
    LeaderFullnameRuntimeTextUnavailable,
    LeaderCodenameRuntimeTextUnavailable,
    LeaderRuntimeTextContextMissing,
    LeaderRuntimeTextIdentityMismatch,
    AgencyInsigniaUnavailable,
    AgencyInsigniaSpriteUnavailable,
    ContradictoryLeaderPresentation,
}

impl PopupSuppression {
    pub fn is_measured_no_popup(self) -> bool {
        matches!(
            self,
            Self::DefinitionHidden | Self::VisibleOptionOrdinalsEmpty
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedFiredEvent {
    pub source_file: String,
    pub event_id: String,
    /// The recipient's ENGINE identity. Carried through rather than spent
    /// resolving the TAG: two ids can share a display TAG, and an admitted
    /// history accumulates rows across hours, so the TAG alone cannot say which
    /// country an event was addressed to.
    pub recipient_tag_db_id: i64,
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
    pub runtime_title_text_context_id: Option<String>,
    pub runtime_description_text_context_id: Option<String>,
    pub runtime_option_text_context_ids: BTreeMap<usize, String>,
    pub popup: Option<AdmittedEventPopup>,
    pub popup_suppression: Option<PopupSuppression>,
}

impl AdmittedFiredEvent {
    /// Open for the country whose ENGINE identity is `recipient_tag_db_id`.
    ///
    /// By id, not by TAG. A TAG is display text and what it denotes can change
    /// between hours, so a pending event recorded against `DUP/441` must not
    /// reach 442 on a later hour where `DUP` denotes 442. The admitted history
    /// proving one TAG per id within itself is not enough: it never sees the
    /// id the TAG denotes NOW.
    pub fn lifecycle_open_at(&self, recipient_tag_db_id: i64, tick: i64) -> bool {
        if self.recipient_tag_db_id != recipient_tag_db_id || tick < self.fire_tick {
            return false;
        }
        let close_tick = self.resolved_tick.map_or(self.timeout_tick, |resolved| {
            resolved.min(self.timeout_tick)
        });
        tick < close_tick
    }

    pub fn visible_at(&self, recipient_tag_db_id: i64, tick: i64) -> bool {
        self.popup.is_some() && self.lifecycle_open_at(recipient_tag_db_id, tick)
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
    MissingPictureSlot,
    MissingDefaultPictureSprite,
    MissingLeaderPortraitSlot,
    LeaderPortraitUnavailable,
    MissingOperativeTextSlot,
    MissingAgencyInsigniaSlot,
    MissingOptionTemplate,
    InvalidOptionGrid,
    InvalidTickDomain,
    MissingAuthoredRoot,
    DuplicateAuthoredRoot,
    MissingOptionGrid,
    InvalidAuthoredLayout,
    TextLayoutUnavailable,
    IncompleteComposition,
}

pub fn event_popup_presentation(
    event: &AdmittedFiredEvent,
    layout: &LayoutSet,
    recipient_tag_db_id: i64,
    tick: i64,
) -> std::result::Result<EventPopupPresentation, EventPresentationRejection> {
    let popup = event
        .popup
        .as_ref()
        .ok_or(EventPresentationRejection::NotMaterialised)?;
    if !event.visible_at(recipient_tag_db_id, tick) {
        return Err(EventPresentationRejection::NotVisible);
    }
    let chrome = event_window_chrome(layout, popup.window_name)
        .ok_or(EventPresentationRejection::MissingChrome)?;
    if matches!(
        popup.subtype,
        EventPopupSubtype::UnitLeader | EventPopupSubtype::OperativeLeader
    ) {
        if popup.leader_portrait.is_none() {
            return Err(EventPresentationRejection::LeaderPortraitUnavailable);
        }
    } else {
        let picture_slot = chrome
            .picture
            .as_ref()
            .ok_or(EventPresentationRejection::MissingPictureSlot)?;
        if popup.picture_gfx.is_none() && picture_slot.sprite.as_deref().is_none() {
            return Err(EventPresentationRejection::MissingDefaultPictureSprite);
        }
    }
    if chrome
        .options_slot_size
        .as_ref()
        .and_then(|(width, height)| {
            Some((width.parse::<i32>().ok()?, height.parse::<i32>().ok()?))
        })
        .is_none_or(|(width, height)| width <= 0 || height <= 0)
        || chrome.options_max_slots_horizontal.is_none_or(|value| value <= 0)
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventPopupViewerState {
    pub focused_event_sequence: Option<i64>,
    pub dismissed_event_sequences: BTreeSet<i64>,
}

impl EventPopupViewerState {
    pub fn cycle_focus(&mut self, visible_sequences: &[i64]) {
        if visible_sequences.is_empty() {
            self.focused_event_sequence = None;
            return;
        }
        let next = self
            .focused_event_sequence
            .and_then(|focused| visible_sequences.iter().position(|value| *value == focused))
            .map_or(0, |index| (index + 1) % visible_sequences.len());
        self.focused_event_sequence = Some(visible_sequences[next]);
    }

    pub fn focus(&mut self, sequence: i64, visible_sequences: &[i64]) -> bool {
        if visible_sequences.contains(&sequence) {
            self.focused_event_sequence = Some(sequence);
            true
        } else {
            false
        }
    }

    pub fn dismiss_focused(&mut self) -> Option<i64> {
        let sequence = self.focused_event_sequence.take()?;
        self.dismissed_event_sequences.insert(sequence);
        Some(sequence)
    }

    pub fn reset_for_seek_or_country_change(&mut self) {
        *self = Self::default();
    }
}

pub fn current_event_tick(
    frame_hour: i64,
    events: &[AdmittedFiredEvent],
) -> std::result::Result<i64, EventPresentationRejection> {
    if frame_hour < 0 {
        return Err(EventPresentationRejection::InvalidTickDomain);
    }
    let mut offset = None;
    for event in events {
        let candidate = event
            .fire_tick
            .checked_sub(event.fire_hour)
            .ok_or(EventPresentationRejection::InvalidTickDomain)?;
        if offset.is_some_and(|prior| prior != candidate) {
            return Err(EventPresentationRejection::InvalidTickDomain);
        }
        offset = Some(candidate);
    }
    frame_hour
        .checked_add(offset.unwrap_or(0))
        .ok_or(EventPresentationRejection::InvalidTickDomain)
}

pub fn visible_event_popup_stack<'a>(
    events: &'a [AdmittedFiredEvent],
    recipient_tag_db_id: i64,
    tick: i64,
    viewer: &EventPopupViewerState,
) -> (Vec<&'a AdmittedFiredEvent>, Option<&'a AdmittedFiredEvent>) {
    let stack = events
        .iter()
        .filter(|event| {
            !viewer
                .dismissed_event_sequences
                .contains(&event.event_sequence)
                && event.visible_at(recipient_tag_db_id, tick)
        })
        .collect::<Vec<_>>();
    let focused = viewer
        .focused_event_sequence
        .and_then(|sequence| {
            stack
                .iter()
                .copied()
                .find(|event| event.event_sequence == sequence)
        })
        .or_else(|| stack.last().copied());
    (stack, focused)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EventPopupInputRegion {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl EventPopupInputRegion {
    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x.saturating_add(self.width)
            && y < self.y.saturating_add(self.height)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventOptionHitTarget {
    pub event_sequence: i64,
    pub definition_ordinal: usize,
    pub region: EventPopupInputRegion,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EventPopupComposeOutput {
    pub output: ComposeOutput,
    pub source_files: BTreeSet<String>,
    pub root_source: String,
    pub root_window: String,
    pub event_sequence: i64,
    pub root_region: EventPopupInputRegion,
    pub minimize_region: EventPopupInputRegion,
    pub minimize_child_path: Vec<u32>,
    pub minimize_tooltip_key: Option<String>,
    pub option_targets: Vec<EventOptionHitTarget>,
}

fn collect_named_element<'a>(element: &'a GuiElement, name: &str, out: &mut Vec<&'a GuiElement>) {
    if element.name.eq_ignore_ascii_case(name) {
        out.push(element);
    }
    for child in &element.children {
        collect_named_element(child, name, out);
    }
}

fn unique_authored_element(
    layouts: &LayoutSet,
    name: &str,
) -> std::result::Result<(GuiElement, String), EventPresentationRejection> {
    let mut matches = Vec::new();
    for document in &layouts.documents {
        for root in &document.roots {
            let mut document_matches = Vec::new();
            collect_named_element(root, name, &mut document_matches);
            matches.extend(
                document_matches
                    .into_iter()
                    .map(|element| (element.clone(), document.source_file.clone())),
            );
        }
    }
    match matches.len() {
        0 => Err(EventPresentationRejection::MissingAuthoredRoot),
        1 => Ok(matches.pop().expect("one authored event element")),
        _ => Err(EventPresentationRejection::DuplicateAuthoredRoot),
    }
}

fn named_count(element: &GuiElement, name: &str) -> usize {
    usize::from(element.name.eq_ignore_ascii_case(name))
        + element
            .children
            .iter()
            .map(|child| named_count(child, name))
            .sum::<usize>()
}

fn named_child_mut<'a>(element: &'a mut GuiElement, name: &str) -> Option<&'a mut GuiElement> {
    if element.name.eq_ignore_ascii_case(name) {
        return Some(element);
    }
    for child in &mut element.children {
        if let Some(found) = named_child_mut(child, name) {
            return Some(found);
        }
    }
    None
}

fn exact_named_child_mut<'a>(
    element: &'a mut GuiElement,
    name: &str,
) -> std::result::Result<&'a mut GuiElement, EventPresentationRejection> {
    if named_count(element, name) != 1 {
        return Err(EventPresentationRejection::InvalidAuthoredLayout);
    }
    named_child_mut(element, name).ok_or(EventPresentationRejection::InvalidAuthoredLayout)
}

fn parsed_pair(element: &GuiElement, position: bool) -> Option<(i32, i32)> {
    let value = if position {
        element.position.as_ref()?
    } else {
        element.size.as_ref()?
    };
    Some((value.x.raw.parse().ok()?, value.y.raw.parse().ok()?))
}

fn region_for(
    output: &ComposeOutput,
    name: &str,
) -> std::result::Result<EventPopupInputRegion, EventPresentationRejection> {
    let mut matches = output.debug_rects.iter().filter(|rect| rect.name == name);
    let rect = matches
        .next()
        .ok_or(EventPresentationRejection::InvalidAuthoredLayout)?;
    if matches.next().is_some() || rect.w <= 0 || rect.h <= 0 {
        return Err(EventPresentationRejection::InvalidAuthoredLayout);
    }
    Ok(EventPopupInputRegion {
        x: rect.x,
        y: rect.y,
        width: rect.w,
        height: rect.h,
    })
}

fn child_path_for(
    output: &ComposeOutput,
    name: &str,
) -> std::result::Result<Vec<u32>, EventPresentationRejection> {
    let mut matches = output.debug_rects.iter().filter(|rect| rect.name == name);
    let path = matches
        .next()
        .map(|rect| rect.child_path.clone())
        .ok_or(EventPresentationRejection::InvalidAuthoredLayout)?;
    if matches.next().is_some() {
        return Err(EventPresentationRejection::InvalidAuthoredLayout);
    }
    Ok(path)
}

#[allow(clippy::too_many_arguments)]
pub fn compose_event_popup_surface(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    localisation: &Localisation,
    cache: &mut UiAssetCache,
    presentation: &EventPopupPresentation,
    layouts: &LayoutSet,
    viewport_width: usize,
    viewport_height: usize,
) -> std::result::Result<EventPopupComposeOutput, EventPresentationRejection> {
    let (mut root, root_source) =
        unique_authored_element(layouts, presentation.popup.window_name)?;
    let (option_template, option_source) = unique_authored_element(layouts, EVENT_OPTION_ENTRY)?;
    let mut options =
        ComposeOptions::new(viewport_width, viewport_height).with_localisation(localisation);
    options.runtime_view_id = Some(presentation.popup.window_name.to_string());
    options
        .text_overrides
        .insert("Title".into(), presentation.popup.title.clone());
    options.runtime_text_by_element.insert(
        "Title".into(),
        presentation.popup.title_runtime_text.clone(),
    );
    options
        .text_overrides
        .insert("Description".into(), presentation.popup.description.clone());
    options.runtime_text_by_element.insert(
        "Description".into(),
        presentation.popup.description_runtime_text.clone(),
    );
    let mut runtime_source_files = BTreeSet::new();
    if let Some(picture) = presentation.popup.picture_gfx.as_ref() {
        let picture_element_name = presentation
            .chrome
            .picture
            .as_ref()
            .map(|element| element.name.clone())
            .ok_or(EventPresentationRejection::MissingPictureSlot)?;
        options.sprite_name_overrides.insert(
            picture_element_name,
            picture.clone(),
        );
    }
    if let Some(portrait) = presentation.popup.leader_portrait.as_ref() {
        exact_named_child_mut(&mut root, "leader")
            .map_err(|_| EventPresentationRejection::MissingLeaderPortraitSlot)?;
        match portrait {
            EventRuntimeImageIdentity::GfxName(name) => {
                let sprite = registry
                    .get(name)
                    .ok_or(EventPresentationRejection::LeaderPortraitUnavailable)?;
                runtime_source_files.insert(sprite.source_file.clone());
                options
                    .sprite_name_overrides
                    .insert("leader".into(), name.clone());
            }
            EventRuntimeImageIdentity::FilePath(path) => {
                let decoded = resolver
                    .decode_texture(path)
                    .map_err(|_| EventPresentationRejection::LeaderPortraitUnavailable)?;
                runtime_source_files.insert(decoded.rel_path);
                options
                    .sprite_overrides
                    .insert("leader".into(), decoded.image);
            }
        }
    }
    for (element, value) in [
        ("leader_fullname", presentation.popup.leader_fullname.as_ref()),
        ("leader_codename", presentation.popup.leader_codename.as_ref()),
    ] {
        if let Some((text, runtime)) = value {
            exact_named_child_mut(&mut root, element)
                .map_err(|_| EventPresentationRejection::MissingOperativeTextSlot)?;
            options.text_overrides.insert(element.into(), text.clone());
            options
                .runtime_text_by_element
                .insert(element.into(), runtime.clone());
        }
    }
    if let Some(insignia) = presentation.popup.agency_insignia.as_ref() {
        exact_named_child_mut(&mut root, "agency_insignia")
            .map_err(|_| EventPresentationRejection::MissingAgencyInsigniaSlot)?;
        match insignia {
            EventRuntimeImageIdentity::GfxName(name) => {
                let sprite = registry
                    .get(name)
                    .ok_or(EventPresentationRejection::IncompleteComposition)?;
                runtime_source_files.insert(sprite.source_file.clone());
                options
                    .sprite_name_overrides
                    .insert("agency_insignia".into(), name.clone());
            }
            EventRuntimeImageIdentity::FilePath(path) => {
                let decoded = resolver
                    .decode_texture(path)
                    .map_err(|_| EventPresentationRejection::IncompleteComposition)?;
                runtime_source_files.insert(decoded.rel_path);
                options
                    .sprite_overrides
                    .insert("agency_insignia".into(), decoded.image);
            }
        }
    }

    let (slot_width, slot_height) = presentation
        .chrome
        .options_slot_size
        .as_ref()
        .and_then(|(width, height)| {
            Some((width.parse::<i32>().ok()?, height.parse::<i32>().ok()?))
        })
        .filter(|(width, height)| *width > 0 && *height > 0)
        .ok_or(EventPresentationRejection::InvalidOptionGrid)?;
    let slots_horizontal = usize::try_from(
        presentation
            .chrome
            .options_max_slots_horizontal
            .filter(|value| *value > 0)
            .ok_or(EventPresentationRejection::InvalidOptionGrid)?,
    )
    .map_err(|_| EventPresentationRejection::InvalidOptionGrid)?;
    {
        let grid = exact_named_child_mut(&mut root, "options_grid")?;
        if !grid.children.is_empty() {
            return Err(EventPresentationRejection::InvalidOptionGrid);
        }
        for (row_index, admitted) in presentation.popup.options.iter().enumerate() {
            let mut row = option_template.clone();
            let prefix = format!(
                "xui21_{}_{}_",
                presentation.event.event_sequence, row_index
            );
            prefix_element_names(&mut row, &prefix);
            let column = row_index % slots_horizontal;
            let grid_row = row_index / slots_horizontal;
            let x = i32::try_from(column)
                .ok()
                .and_then(|index| index.checked_mul(slot_width))
                .ok_or(EventPresentationRejection::InvalidOptionGrid)?;
            let y = i32::try_from(grid_row)
                .ok()
                .and_then(|index| index.checked_mul(slot_height))
                .ok_or(EventPresentationRejection::InvalidOptionGrid)?;
            set_position_upper_left(&mut row, x, y);
            let name = format!("{prefix}Name");
            options
                .text_overrides
                .insert(name.clone(), admitted.text.clone());
            options
                .runtime_text_by_element
                .insert(name.clone(), admitted.runtime_text.clone());
            options
                .runtime_text_authored_element_by_clone
                .insert(name, "Name".into());
            grid.children.push(row);
        }
    }

    let description = {
        let mut matches = Vec::new();
        collect_named_element(&root, "Description", &mut matches);
        if matches.len() != 1 {
            return Err(EventPresentationRejection::InvalidAuthoredLayout);
        }
        matches[0].clone()
    };
    let mut compositor = Compositor::with_cache(resolver, registry, cache);
    let description_height = compositor
        .measure_resolved_text_height(
            &description,
            &presentation.popup.description,
            &presentation.popup.description_runtime_text,
        )
        .map_err(|_| EventPresentationRejection::TextLayoutUnavailable)?;
    let (top_height, mid_y, authored_mid_height, bottom_height, end_height) = {
        let mut top = Vec::new();
        let mut mid = Vec::new();
        let mut bottom = Vec::new();
        let mut end = Vec::new();
        collect_named_element(&root, "top_Window", &mut top);
        collect_named_element(&root, "midsection", &mut mid);
        collect_named_element(&root, "bottom_Window", &mut bottom);
        collect_named_element(&root, "bottom_window_end", &mut end);
        if top.len() != 1 || mid.len() != 1 || bottom.len() != 1 || end.len() > 1 {
            return Err(EventPresentationRejection::InvalidAuthoredLayout);
        }
        let top_height = parsed_pair(top[0], false)
            .map(|value| value.1)
            .filter(|value| *value >= 0)
            .ok_or(EventPresentationRejection::InvalidAuthoredLayout)?;
        let mid_y = parsed_pair(mid[0], true)
            .map(|value| value.1)
            .ok_or(EventPresentationRejection::InvalidAuthoredLayout)?;
        let mid_height = parsed_pair(mid[0], false)
            .map(|value| value.1)
            .filter(|value| *value >= 0)
            .ok_or(EventPresentationRejection::InvalidAuthoredLayout)?;
        let bottom_height = parsed_pair(bottom[0], false)
            .map(|value| value.1)
            .filter(|value| *value >= 0)
            .ok_or(EventPresentationRejection::InvalidAuthoredLayout)?;
        let end_height = end
            .first()
            .and_then(|element| parsed_pair(element, false))
            .map(|value| value.1);
        (
            top_height,
            mid_y,
            mid_height,
            bottom_height,
            end_height,
        )
    };
    let mid_height = authored_mid_height.max(description_height.saturating_add(30));
    let leader = matches!(
        presentation.popup.subtype,
        EventPopupSubtype::UnitLeader | EventPopupSubtype::OperativeLeader
    );
    let row_count = i32::try_from(presentation.popup.options.len())
        .map_err(|_| EventPresentationRejection::InvalidOptionGrid)?;
    let effective_bottom_height = if leader {
        bottom_height.max(row_count.saturating_mul(slot_height))
    } else {
        bottom_height
    };
    let bottom_y = mid_y.saturating_add(mid_height);
    {
        let mid = exact_named_child_mut(&mut root, "midsection")?;
        let width = parsed_pair(mid, false)
            .map(|value| value.0)
            .ok_or(EventPresentationRejection::InvalidAuthoredLayout)?;
        mid.size = Some(pair(width, mid_height));
    }
    {
        let bottom = exact_named_child_mut(&mut root, "bottom_Window")?;
        let width = parsed_pair(bottom, false)
            .map(|value| value.0)
            .ok_or(EventPresentationRejection::InvalidAuthoredLayout)?;
        let x = parsed_pair(bottom, true).map(|value| value.0).unwrap_or(0);
        bottom.position = Some(pair(x, bottom_y));
        bottom.size = Some(pair(width, effective_bottom_height));
    }
    let mut root_height = top_height
        .saturating_add(mid_height)
        .saturating_add(effective_bottom_height);
    if leader {
        let end_height = end_height.ok_or(EventPresentationRejection::InvalidAuthoredLayout)?;
        let end = exact_named_child_mut(&mut root, "bottom_window_end")?;
        let end_x = parsed_pair(end, true).map(|value| value.0).unwrap_or(0);
        end.position = Some(pair(
            end_x,
            bottom_y.saturating_add(effective_bottom_height),
        ));
        root_height = root_height.saturating_add(end_height);
    }
    if presentation.popup.subtype != EventPopupSubtype::News {
        let root_width = parsed_pair(&root, false)
            .map(|value| value.0)
            .ok_or(EventPresentationRejection::InvalidAuthoredLayout)?;
        root.size = Some(pair(root_width, root_height));
    }
    let minimize_tooltip_key = {
        let minimize = exact_named_child_mut(&mut root, "btn_minimize")?;
        minimize.raw.first_scalar_ci("pdx_tooltip").map(str::to_string)
    };
    let output = compositor.compose(&root, &options);
    if !output.report.missing_sprites.is_empty()
        || !output.report.undecodable_sprites.is_empty()
        || !output.report.missing_fonts.is_empty()
        || !output.report.missing_glyphs.is_empty()
        || !output.report.unresolved_loc_keys.is_empty()
        || !output.report.unresolved_text_inputs.is_empty()
    {
        return Err(EventPresentationRejection::IncompleteComposition);
    }
    let root_region = region_for(&output, presentation.popup.window_name)?;
    let minimize_region = region_for(&output, "btn_minimize")?;
    let minimize_child_path = child_path_for(&output, "btn_minimize")?;
    let mut option_targets = Vec::with_capacity(presentation.popup.options.len());
    for (row_index, admitted) in presentation.popup.options.iter().enumerate() {
        let row_name = format!(
            "xui21_{}_{}_{}",
            presentation.event.event_sequence, row_index, EVENT_OPTION_ENTRY
        );
        option_targets.push(EventOptionHitTarget {
            event_sequence: presentation.event.event_sequence,
            definition_ordinal: admitted.definition_ordinal,
            region: region_for(&output, &row_name)?,
        });
    }
    let mut source_files = BTreeSet::from([
        root_source.clone(),
        option_source,
        presentation.event.source_file.clone(),
    ]);
    source_files.extend(runtime_source_files);
    Ok(EventPopupComposeOutput {
        output,
        root_source: root_source.clone(),
        root_window: presentation.popup.window_name.to_string(),
        source_files,
        event_sequence: presentation.event.event_sequence,
        root_region,
        minimize_region,
        minimize_child_path,
        minimize_tooltip_key,
        option_targets,
    })
}

pub struct EventHistoryRequest<'a> {
    pub source_record_id: &'a str,
    pub history: &'a FiredEventsView,
    pub definitions: &'a BTreeMap<String, EventDef>,
    pub localisation: &'a Localisation,
    pub sprites: &'a SpriteRegistry,
    pub runtime_text: &'a BTreeMap<String, RuntimeTextContext>,
}

enum ValidatedVisibleOptionOrdinals {
    Absent,
    Unobtained,
    Measured(Vec<usize>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventHistoryRejection {
    MissingProvenance,
    HistoryIncomplete,
    /// Events had already fired when the observer attached, so the rows cannot
    /// be the whole history however clean the hooks were. Carries those ids.
    EventsFiredBeforeObservation(Vec<String>),
    /// The detours were clean and the rows are measured, but the engine's
    /// retained fired set could not be named in full, so whether anything
    /// predates observation is unknown.
    PreAttachBaselineUnobtained,
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
        if let Some(prior) = request
            .history
            .prior_to_capture
            .measured()
            .filter(|ids| !ids.is_empty())
        {
            return Err(EventHistoryRejection::EventsFiredBeforeObservation(
                prior.clone(),
            ));
        }
        if request.history.events.measured().is_some()
            && request.history.hook_coverage_complete.measured() == Some(&true)
            && matches!(request.history.prior_to_capture, Leaf::Unobtained)
        {
            return Err(EventHistoryRejection::PreAttachBaselineUnobtained);
        }
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
    // ONE TAG, ONE COUNTRY, across the WHOLE admitted history. `replay_core`
    // refuses a single frame whose rows fuse two ids under one TAG; this is the
    // same rule over the ACCUMULATED history, where two hours could each be
    // internally consistent and still disagree about who "DUP" is. The join
    // below is by TAG, so this is what makes that join sound.
    let mut recipient_id_by_tag: BTreeMap<String, i64> = BTreeMap::new();
    for row in rows {
        let admitted = admit_fired_event(row, &request)?;
        if recipient_id_by_tag
            .insert(
                admitted.recipient_tag.clone(),
                admitted.recipient_tag_db_id,
            )
            .is_some_and(|seen| seen != admitted.recipient_tag_db_id)
        {
            return Err(EventHistoryRejection::InvalidIdentity);
        }
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
    let recipient_tag_db_id = *row
        .recipient_tag_db_id
        .measured()
        .filter(|id| **id > 0)
        .ok_or(EventHistoryRejection::InvalidIdentity)?;
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
            ValidatedVisibleOptionOrdinals::Measured(
                values
                    .iter()
                    .map(|value| usize::try_from(*value).expect("validated option ordinal"))
                    .collect::<Vec<_>>(),
            )
        }
        Leaf::Measured(_) => return Err(EventHistoryRejection::InvalidVisibleOptions),
        Leaf::Absent => ValidatedVisibleOptionOrdinals::Absent,
        Leaf::Unobtained => ValidatedVisibleOptionOrdinals::Unobtained,
    };
    let chosen_option_ordinal = match &row.chosen_option_ordinal {
        Leaf::Measured(value) => usize::try_from(*value)
            .ok()
            .filter(|ordinal| *ordinal < def.options.len())
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
        .filter(|value| {
            fire_tick
                .checked_add(effective_timeout_hours)
                .is_some_and(|expected| *value == expected)
        })
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
        "timeout" if resolved_tick.is_some_and(|value| value >= timeout_tick) => {}
        _ => return Err(EventHistoryRejection::InvalidResolution),
    }

    let runtime_title_text_context_id =
        optional_text_context_id(&row.runtime_title_text_context_id);
    let runtime_description_text_context_id =
        optional_text_context_id(&row.runtime_description_text_context_id);
    let runtime_option_text_context_ids = measured_option_text_context_ids(row);
    let popup_result = admit_popup_materialisation(
        row,
        def,
        &visible_ordinals,
        chosen_option_ordinal,
        runtime_title_text_context_id
            .as_ref()
            .map(|value| value.as_deref()),
        runtime_description_text_context_id
            .as_ref()
            .map(|value| value.as_deref()),
        runtime_option_text_context_ids.as_ref(),
        request,
    );
    let (popup, popup_suppression) = match popup_result {
        Ok(popup) => (Some(popup), None),
        Err(reason) => (None, Some(reason)),
    };

    Ok(AdmittedFiredEvent {
        source_file: def.source_file.clone(),
        event_id: event_id.to_string(),
        recipient_tag_db_id,
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
        runtime_title_text_context_id: runtime_title_text_context_id.flatten(),
        runtime_description_text_context_id: runtime_description_text_context_id.flatten(),
        runtime_option_text_context_ids: runtime_option_text_context_ids.unwrap_or_default(),
        popup,
        popup_suppression,
    })
}

fn admit_popup_materialisation(
    row: &RecordedFiredEventRow,
    def: &EventDef,
    visible_ordinals: &ValidatedVisibleOptionOrdinals,
    chosen_option_ordinal: Option<usize>,
    runtime_title_text_context_id: Option<Option<&str>>,
    runtime_description_text_context_id: Option<Option<&str>>,
    runtime_option_text_context_ids: Option<&BTreeMap<usize, String>>,
    request: &EventHistoryRequest<'_>,
) -> std::result::Result<AdmittedEventPopup, PopupSuppression> {
    if def.hidden {
        return Err(PopupSuppression::DefinitionHidden);
    }
    let measured_desc = measured_nonempty(&row.selected_desc_key);
    let visible_ordinals = match visible_ordinals {
        ValidatedVisibleOptionOrdinals::Absent => {
            return Err(PopupSuppression::VisibleOptionOrdinalsAbsent)
        }
        ValidatedVisibleOptionOrdinals::Unobtained => {
            return Err(PopupSuppression::VisibleOptionOrdinalsUnobtained)
        }
        ValidatedVisibleOptionOrdinals::Measured(values) if values.is_empty() => {
            return Err(PopupSuppression::VisibleOptionOrdinalsEmpty)
        }
        ValidatedVisibleOptionOrdinals::Measured(values) => values.as_slice(),
    };
    let subtype = measured_nonempty(&row.popup_subtype)
        .and_then(EventPopupSubtype::from_producer_token)
        .filter(|subtype| subtype.matches_definition(def.kind))
        .ok_or(PopupSuppression::WindowVariantUndetermined)?;
    let selected_desc_key = match measured_desc {
        Some(key) if def.desc_keys.iter().any(|candidate| candidate == key) => key.to_string(),
        Some(_) => return Err(PopupSuppression::DescriptionBranchNotDeclared),
        None => return Err(PopupSuppression::DescriptionBranchUndetermined),
    };
    let visible_set: BTreeSet<usize> = visible_ordinals.iter().copied().collect();
    if def
        .options
        .iter()
        .enumerate()
        .any(|(ordinal, option)| !option.has_trigger && !visible_set.contains(&ordinal))
    {
        return Err(PopupSuppression::UnconditionalOptionNotVisible);
    }
    if chosen_option_ordinal.is_some_and(|ordinal| !visible_set.contains(&ordinal)) {
        return Err(PopupSuppression::ChosenOptionNotVisible);
    }
    let title_context_id = runtime_title_text_context_id
        .ok_or(PopupSuppression::TitleRuntimeTextContextUnavailable)?;
    let description_context_id = runtime_description_text_context_id
        .ok_or(PopupSuppression::DescriptionRuntimeTextContextUnavailable)?;
    let option_context_ids = runtime_option_text_context_ids
        .ok_or(PopupSuppression::OptionRuntimeTextContextsUnavailable)?;
    if option_context_ids
        .keys()
        .any(|ordinal| !visible_set.contains(ordinal))
    {
        return Err(PopupSuppression::OptionRuntimeTextContextNotVisible);
    }
    if title_context_id.is_some_and(|id| !request.runtime_text.contains_key(id)) {
        return Err(PopupSuppression::TitleRuntimeTextContextMissing);
    }
    if description_context_id.is_some_and(|id| !request.runtime_text.contains_key(id)) {
        return Err(PopupSuppression::DescriptionRuntimeTextContextMissing);
    }
    if option_context_ids
        .values()
        .any(|id| !request.runtime_text.contains_key(id))
    {
        return Err(PopupSuppression::OptionRuntimeTextContextMissing);
    }
    let empty_runtime = RuntimeTextContext::default();
    let title_runtime = title_context_id
        .and_then(|id| request.runtime_text.get(id))
        .unwrap_or(&empty_runtime);
    let description_runtime = description_context_id
        .and_then(|id| request.runtime_text.get(id))
        .unwrap_or(&empty_runtime);
    let title = resolve_exact_text(
        request.localisation,
        def.title_key
            .as_deref()
            .ok_or(PopupSuppression::TitleKeyMissing)?,
        title_runtime,
    )
    .map_err(|_| PopupSuppression::TitleTextUnresolved)?;
    let description = resolve_exact_text(
        request.localisation,
        &selected_desc_key,
        description_runtime,
    )
    .map_err(|_| PopupSuppression::DescriptionTextUnresolved)?;
    let mut options = Vec::with_capacity(visible_ordinals.len());
    for &ordinal in visible_ordinals {
        let key = def.options[ordinal]
            .name_key
            .as_deref()
            .ok_or(PopupSuppression::OptionNameMissing)?;
        let option_runtime = option_context_ids
            .get(&ordinal)
            .and_then(|id| request.runtime_text.get(id))
            .unwrap_or(&empty_runtime);
        options.push(AdmittedEventOption {
            definition_ordinal: ordinal,
            text: resolve_exact_text(request.localisation, key, option_runtime)
                .map_err(|_| PopupSuppression::OptionTextUnresolved)?,
            runtime_text: option_runtime.clone(),
            selected: chosen_option_ordinal == Some(ordinal),
        });
    }
    let leader_window = matches!(
        subtype,
        EventPopupSubtype::UnitLeader | EventPopupSubtype::OperativeLeader
    );
    let picture_gfx = if leader_window {
        if row.shown_picture_gfx.is_measured() {
            return Err(PopupSuppression::ContradictoryLeaderPresentation);
        }
        None
    } else {
        match &row.shown_picture_gfx {
            Leaf::Measured(selected) if !selected.trim().is_empty() => {
                request
                    .sprites
                    .get(selected)
                    .ok_or(PopupSuppression::PictureSpriteUnavailable)?;
                Some(selected.clone())
            }
            Leaf::Absent | Leaf::Unobtained if def.picture_gfx.is_none() => None,
            Leaf::Absent | Leaf::Unobtained | Leaf::Measured(_) => {
                return Err(PopupSuppression::PictureSelectionUnavailable)
            }
        }
    };
    let leader_portrait = if leader_window {
        let kind = measured_nonempty(&row.leader_portrait_identity_kind)
            .ok_or(PopupSuppression::LeaderPortraitUnavailable)?;
        let identity = measured_nonempty(&row.leader_portrait_identity)
            .ok_or(PopupSuppression::LeaderPortraitUnavailable)?;
        match kind {
            "gfx_name" => {
                request
                    .sprites
                    .get(identity)
                    .ok_or(PopupSuppression::LeaderPortraitSpriteUnavailable)?;
                Some(EventRuntimeImageIdentity::GfxName(identity.to_string()))
            }
            "file_path" => Some(EventRuntimeImageIdentity::FilePath(identity.to_string())),
            _ => return Err(PopupSuppression::LeaderPortraitUnavailable),
        }
    } else {
        if row.leader_portrait_identity_kind.is_measured()
            || row.leader_portrait_identity.is_measured()
        {
            return Err(PopupSuppression::ContradictoryLeaderPresentation);
        }
        None
    };
    let resolve_operative_text =
        |context_id: &Leaf<String>, element: &str, unavailable: PopupSuppression| {
            let id = measured_nonempty(context_id).ok_or(unavailable)?;
            let runtime = request
                .runtime_text
                .get(id)
                .ok_or(PopupSuppression::LeaderRuntimeTextContextMissing)?;
            if !runtime.exact_for_element(Some("EventWindow_Operative"), element) {
                return Err(PopupSuppression::LeaderRuntimeTextIdentityMismatch);
            }
            let resolved = resolve_element_bound_exact_runtime(runtime);
            if !resolved.admitted() {
                return Err(PopupSuppression::LeaderRuntimeTextIdentityMismatch);
            }
            Ok((resolved.text, runtime.clone()))
        };
    let (leader_fullname, leader_codename, agency_insignia) =
        if subtype == EventPopupSubtype::OperativeLeader {
            let fullname = resolve_operative_text(
                &row.runtime_leader_fullname_text_context_id,
                "leader_fullname",
                PopupSuppression::LeaderFullnameRuntimeTextUnavailable,
            )?;
            let codename = resolve_operative_text(
                &row.runtime_leader_codename_text_context_id,
                "leader_codename",
                PopupSuppression::LeaderCodenameRuntimeTextUnavailable,
            )?;
            let kind = measured_nonempty(&row.agency_insignia_identity_kind)
                .ok_or(PopupSuppression::AgencyInsigniaUnavailable)?;
            let identity = measured_nonempty(&row.agency_insignia_identity)
                .ok_or(PopupSuppression::AgencyInsigniaUnavailable)?;
            let insignia = match kind {
                "gfx_name" => {
                    request
                        .sprites
                        .get(identity)
                        .ok_or(PopupSuppression::AgencyInsigniaSpriteUnavailable)?;
                    EventRuntimeImageIdentity::GfxName(identity.to_string())
                }
                "file_path" => EventRuntimeImageIdentity::FilePath(identity.to_string()),
                _ => return Err(PopupSuppression::AgencyInsigniaUnavailable),
            };
            (Some(fullname), Some(codename), Some(insignia))
        } else {
            if row.runtime_leader_fullname_text_context_id.is_measured()
                || row.runtime_leader_codename_text_context_id.is_measured()
                || row.agency_insignia_identity_kind.is_measured()
                || row.agency_insignia_identity.is_measured()
            {
                return Err(PopupSuppression::ContradictoryLeaderPresentation);
            }
            (None, None, None)
        };
    Ok(AdmittedEventPopup {
        subtype,
        window_name: subtype.window_name(),
        title,
        title_runtime_text: title_runtime.clone(),
        description,
        description_runtime_text: description_runtime.clone(),
        selected_desc_key,
        picture_gfx,
        leader_portrait,
        leader_fullname,
        leader_codename,
        agency_insignia,
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

fn measured_option_text_context_ids(
    row: &RecordedFiredEventRow,
) -> Option<BTreeMap<usize, String>> {
    let rows = match &row.runtime_option_text_contexts {
        Leaf::Absent => return Some(BTreeMap::new()),
        Leaf::Unobtained => return None,
        Leaf::Measured(rows) => rows,
    };
    let mut contexts = BTreeMap::new();
    for row in rows {
        let ordinal = usize::try_from(measured_nonnegative(&row.definition_ordinal)?).ok()?;
        let context_id = measured_nonempty(&row.context_id)?.to_string();
        if contexts.insert(ordinal, context_id).is_some() {
            return None;
        }
    }
    Some(contexts)
}

fn optional_text_context_id(value: &Leaf<String>) -> Option<Option<String>> {
    match value {
        Leaf::Absent => Some(None),
        Leaf::Unobtained => None,
        Leaf::Measured(value) if !value.trim().is_empty() => Some(Some(value.clone())),
        Leaf::Measured(_) => None,
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
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_asset_root(label: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "game_ui_event_chrome_{label}_{}_{nonce}",
            std::process::id()
        ))
    }

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
    fn event_layout_discovers_mod_chrome_outside_the_vanilla_filename() {
        let root = temporary_asset_root("layout_discovery");
        let mod_interface = root.join("interface").join("mod_popup");
        fs::create_dir_all(&mod_interface).expect("create temporary mod interface directory");
        fs::write(
            mod_interface.join("authored_popup_chrome.gui"),
            br#"
                guiTypes = {
                    containerWindowType = {
                        name = "EventWindow_News"
                        size = { width = 777 height = 555 }
                        iconType = {
                            name = "event_picture"
                            spriteType = "GFX_mod_authored_news_default"
                        }
                    }
                }
            "#,
        )
        .expect("write mod-authored event chrome");

        let layout = load_event_layout(&AssetResolver::new(vec![root.clone()]))
            .expect("discover recursive mod-authored event chrome");
        assert_eq!(layout.stats.file_count, 1);
        let chrome = event_window_chrome(&layout, "EventWindow_News")
            .expect("engine-named window discovered outside eventwindow.gui");
        assert_eq!(chrome.window_size, Some(("777".into(), "555".into())));
        assert_eq!(
            chrome.picture.and_then(|picture| picture.sprite),
            Some("GFX_mod_authored_news_default".into())
        );

        fs::write(
            root.join(EVENT_WINDOW_GUI),
            br#"
                guiTypes = {
                    containerWindowType = {
                        name = "EventWindow_News"
                        size = { width = 528 height = 595 }
                    }
                }
            "#,
        )
        .expect("write an ambiguous second engine-named root");
        let ambiguous_layout = load_event_layout(&AssetResolver::new(vec![root.clone()]))
            .expect("discover both engine-named roots");
        assert_eq!(
            event_window_chrome(&ambiguous_layout, "EventWindow_News"),
            None,
            "unproven cross-file duplicate precedence must fail closed"
        );

        portable_test_fs_retry::remove_dir_all(&root).expect("remove temporary event chrome fixture");
    }

    #[test]
    fn event_layout_refuses_any_effective_gui_recovery_or_decode_loss() {
        let recovery_root = temporary_asset_root("layout_recovery_refusal");
        let recovery_interface = recovery_root.join("interface");
        fs::create_dir_all(&recovery_interface).expect("create recovery fixture directory");
        fs::write(
            recovery_interface.join("damaged.gui"),
            br#"guiTypes = { containerWindowType = { name = "EventWindow""#,
        )
        .expect("write truncated GUI fixture");
        let recovery = load_event_layout(&AssetResolver::new(vec![recovery_root.clone()]))
            .expect_err("parser recovery can hide an effective event root");
        assert!(recovery.message.contains("can hide event authority"));
        assert!(recovery.message.contains("authority_recovery_files="));
        portable_test_fs_retry::remove_dir_all(&recovery_root).expect("remove recovery fixture");

        let unrelated_root = temporary_asset_root("unrelated_layout_recovery");
        let unrelated_interface = unrelated_root.join("interface");
        fs::create_dir_all(&unrelated_interface)
            .expect("create unrelated recovery fixture directory");
        fs::write(
            unrelated_interface.join("damaged.gui"),
            br#"guiTypes = { containerWindowType = { name = "UnrelatedWindow""#,
        )
        .expect("write unrelated truncated GUI fixture");
        let unrelated = load_event_layout(&AssetResolver::new(vec![unrelated_root.clone()]))
            .expect("recovery without an event authority token cannot hide an event root");
        assert_ne!(unrelated.stats.recovery_count, 0);
        portable_test_fs_retry::remove_dir_all(&unrelated_root).expect("remove unrelated recovery fixture");

        let decode_root = temporary_asset_root("layout_decode_refusal");
        let decode_interface = decode_root.join("interface");
        fs::create_dir_all(&decode_interface).expect("create decode fixture directory");
        let mut bytes = [
            b"\xef\xbb\xbf".as_slice(),
            br#"guiTypes = { containerWindowType = { name = "EventWindow" } } "#,
        ]
        .concat();
        bytes.extend_from_slice(&[0xe2, 0x82]);
        fs::write(decode_interface.join("damaged.gui"), bytes)
            .expect("write decode-damaged GUI fixture");
        let decode = load_event_layout(&AssetResolver::new(vec![decode_root.clone()]))
            .expect_err("decode loss can hide an effective event root");
        assert!(decode.message.contains("can hide event authority"));
        assert!(decode.message.contains("decode_losses="));
        portable_test_fs_retry::remove_dir_all(&decode_root).expect("remove decode fixture");
    }

    #[test]
    fn hidden_event_excluded_from_renderable_denominator() {
        let corpus = EventCorpus {
            defs: parse_events_document("events/test.txt", &sample_events_doc()),
            files_parsed: 1,
            parse_failures: Vec::new(),
            decode_lossy_files: Vec::new(),
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
        assert_eq!(report.renderable, 2);
        assert_eq!(report.title_resolved, 2);
        assert_eq!(report.desc_resolved, 2);
        assert_eq!(report.picture_resolved, 2);
        assert_eq!(report.renderable_by_kind[0], 1);
        assert_eq!(report.renderable_by_kind[1], 1);
    }

    #[test]
    fn missing_picture_sprite_blocks_renderable() {
        let corpus = EventCorpus {
            defs: parse_events_document("events/test.txt", &sample_events_doc()),
            files_parsed: 1,
            parse_failures: Vec::new(),
            decode_lossy_files: Vec::new(),
        };
        let mut loc = Localisation::default();
        for key in ["news.1.t", "news.1.d", "news.1.a"] {
            loc.entries.insert(key.to_string(), format!("[{key}]"));
        }
        let registry = SpriteRegistry::new();
        let report = resolve_events(&corpus, &loc, &registry);
        assert!(report.picture_declared >= 1);
        assert_eq!(report.picture_resolved, 0);
        assert_eq!(report.renderable, 0);
    }

    #[test]
    fn fired_event_content_is_fail_closed_without_stream() {
        let defs = parse_events_document("events/test.txt", &sample_events_doc());
        let generic = defs[0].clone();
        let mut loc = Localisation::default();
        loc.entries
            .insert("generic.1.t".to_string(), "A Title".to_string());
        loc.entries
            .insert("generic.1.d_good".to_string(), "Good desc".to_string());
        loc.entries
            .insert("generic.1.a".to_string(), "Option A".to_string());
        loc.entries
            .insert("generic.1.b".to_string(), "Option B".to_string());

        let absent = event_window_content(&generic, &loc, None);
        assert!(!absent.available);
        assert_eq!(absent.title, None);
        assert!(absent.options.is_empty());

        let fired = FiredEvent {
            event_id: "generic.1".to_string(),
            target_tag: Some("GER".to_string()),
            chosen_option_index: Some(1),
            fire_date: Some("1936.1.1.12".to_string()),
        };
        let bound = event_window_content(&generic, &loc, Some(&fired));
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

    #[test]
    fn country_event_content_uses_the_exact_plain_window_mapping() {
        let generic = parse_events_document("events/test.txt", &sample_events_doc())
            .into_iter()
            .find(|def| def.id == "generic.1")
            .expect("country-event fixture");
        assert_eq!(generic.kind, EventKind::Country);
        let mut localisation = Localisation::default();
        localisation
            .entries
            .insert("generic.1.t".into(), "Country title".into());
        let fired = FiredEvent {
            event_id: "generic.1".into(),
            target_tag: Some("GER".into()),
            chosen_option_index: Some(0),
            fire_date: Some("1936.1.1.12".into()),
        };
        let content = event_window_content(&generic, &localisation, Some(&fired));
        assert!(content.available);
        assert_eq!(content.window_name.as_deref(), Some("EventWindow"));
        assert_eq!(content.title.as_deref(), Some("Country title"));
    }

    /// release builds return the fail-closed absent form.
    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "fired-event id must match def id")
    )]
    fn fired_event_id_mismatch_is_fail_closed() {
        let defs = parse_events_document("events/test.txt", &sample_events_doc());
        let generic = defs[0].clone();
        let mut loc = Localisation::default();
        loc.entries
            .insert("generic.1.t".to_string(), "A Title".to_string());
        let fired = FiredEvent {
            event_id: "other.99".to_string(),
            target_tag: None,
            chosen_option_index: Some(0),
            fire_date: None,
        };
        let content = event_window_content(&generic, &loc, Some(&fired));
        // Release-only path (debug builds panic above): absent form, nothing bound.
        assert_eq!(content, EventWindowContent::default());
        assert!(!content.available);
    }

    fn rich_row() -> RecordedFiredEventRow {
        RecordedFiredEventRow {
            event_id: Leaf::Measured("generic.1".into()),
            recipient_tag_db_id: Leaf::Measured(1),
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
            leader_portrait_identity_kind: Leaf::Absent,
            leader_portrait_identity: Leaf::Absent,
            runtime_leader_fullname_text_context_id: Leaf::Absent,
            runtime_leader_codename_text_context_id: Leaf::Absent,
            agency_insignia_identity_kind: Leaf::Absent,
            agency_insignia_identity: Leaf::Absent,
            effective_timeout_hours: Leaf::Measured(312),
            timeout_tick: Leaf::Measured(1512),
            resolved_tick: Leaf::Measured(1210),
            resolution_reason: Leaf::Measured("selected".into()),
            runtime_title_text_context_id: Leaf::Absent,
            runtime_description_text_context_id: Leaf::Absent,
            runtime_option_text_contexts: Leaf::Absent,
        }
    }

    fn popup_fixture() -> (
        RecordedFiredEventRow,
        EventDef,
        Localisation,
        SpriteRegistry,
        BTreeMap<String, RuntimeTextContext>,
    ) {
        let def = parse_events_document("events/test.txt", &sample_events_doc())
            .into_iter()
            .find(|def| def.id == "generic.1")
            .expect("generic.1 fixture");
        let mut localisation = Localisation::default();
        for (key, text) in [
            ("generic.1.t", "A Title"),
            ("generic.1.d_good", "Good description"),
            ("generic.1.d_bad", "Other description"),
            ("generic.1.a", "Option A"),
            ("generic.1.b", "Option B"),
        ] {
            localisation.entries.insert(key.into(), text.into());
        }
        let mut sprites = SpriteRegistry::new();
        sprites.insert(sprite_stub("GFX_report_event_generic_read_write"));
        (rich_row(), def, localisation, sprites, BTreeMap::new())
    }

    fn admit_fixture(
        row: RecordedFiredEventRow,
        def: EventDef,
        localisation: &Localisation,
        sprites: &SpriteRegistry,
        runtime_text: &BTreeMap<String, RuntimeTextContext>,
    ) -> AdmittedFiredEvent {
        let definitions = BTreeMap::from([(def.id.clone(), def)]);
        let history = FiredEventsView {
            events: Leaf::Measured(vec![row]),
            history_complete: Leaf::Measured(true),
            hook_coverage_complete: Leaf::Measured(true),
            prior_to_capture: Leaf::Measured(Vec::new()),
        };
        admit_event_history(EventHistoryRequest {
            source_record_id: "record:popup-suppression",
            history: &history,
            definitions: &definitions,
            localisation,
            sprites,
            runtime_text,
        })
        .expect("lifecycle row remains admitted")
        .into_iter()
        .next()
        .expect("one admitted lifecycle row")
    }

    fn assert_suppression(
        row: RecordedFiredEventRow,
        def: EventDef,
        localisation: &Localisation,
        sprites: &SpriteRegistry,
        runtime_text: &BTreeMap<String, RuntimeTextContext>,
        expected: PopupSuppression,
    ) {
        let admitted = admit_fixture(row, def, localisation, sprites, runtime_text);
        assert_eq!(admitted.popup_suppression, Some(expected));
        assert_eq!(admitted.popup, None);
    }

    #[test]
    fn popup_suppression_definition_hidden() {
        let (row, mut def, loc, sprites, runtime) = popup_fixture();
        def.hidden = true;
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::DefinitionHidden,
        );
    }

    #[test]
    fn popup_suppression_visible_option_ordinals_absent() {
        let (mut row, def, loc, sprites, runtime) = popup_fixture();
        row.visible_option_ordinals = Leaf::Absent;
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::VisibleOptionOrdinalsAbsent,
        );
    }

    #[test]
    fn popup_suppression_visible_option_ordinals_unobtained() {
        let (mut row, def, loc, sprites, runtime) = popup_fixture();
        row.visible_option_ordinals = Leaf::Unobtained;
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::VisibleOptionOrdinalsUnobtained,
        );
    }

    #[test]
    fn popup_suppression_visible_option_ordinals_measured_empty() {
        let (mut row, def, loc, sprites, runtime) = popup_fixture();
        row.visible_option_ordinals = Leaf::Measured(Vec::new());
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::VisibleOptionOrdinalsEmpty,
        );
    }

    #[test]
    fn popup_suppression_description_branch_not_declared() {
        let (mut row, def, loc, sprites, runtime) = popup_fixture();
        row.selected_desc_key = Leaf::Measured("generic.1.d_unknown".into());
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::DescriptionBranchNotDeclared,
        );
    }

    #[test]
    fn popup_suppression_description_branch_undetermined() {
        let (mut row, def, loc, sprites, runtime) = popup_fixture();
        row.selected_desc_key = Leaf::Unobtained;
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::DescriptionBranchUndetermined,
        );
    }

    #[test]
    fn popup_suppression_missing_exact_subtype() {
        let (mut row, def, loc, sprites, runtime) = popup_fixture();
        row.popup_subtype = Leaf::Unobtained;
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::WindowVariantUndetermined,
        );
    }

    #[test]
    fn popup_suppression_unconditional_option_not_visible() {
        let (mut row, def, loc, sprites, runtime) = popup_fixture();
        row.visible_option_ordinals = Leaf::Measured(vec![1]);
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::UnconditionalOptionNotVisible,
        );
    }

    #[test]
    fn popup_suppression_chosen_option_not_visible() {
        let (mut row, mut def, loc, sprites, runtime) = popup_fixture();
        for option in &mut def.options {
            option.has_trigger = true;
        }
        row.visible_option_ordinals = Leaf::Measured(vec![0]);
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::ChosenOptionNotVisible,
        );
    }

    #[test]
    fn popup_suppression_title_runtime_text_context_unavailable() {
        let (mut row, def, loc, sprites, runtime) = popup_fixture();
        row.runtime_title_text_context_id = Leaf::Unobtained;
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::TitleRuntimeTextContextUnavailable,
        );
    }

    #[test]
    fn popup_suppression_description_runtime_text_context_unavailable() {
        let (mut row, def, loc, sprites, runtime) = popup_fixture();
        row.runtime_description_text_context_id = Leaf::Unobtained;
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::DescriptionRuntimeTextContextUnavailable,
        );
    }

    #[test]
    fn popup_suppression_option_runtime_text_contexts_unavailable() {
        let (mut row, def, loc, sprites, runtime) = popup_fixture();
        row.runtime_option_text_contexts = Leaf::Unobtained;
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::OptionRuntimeTextContextsUnavailable,
        );
    }

    #[test]
    fn popup_suppression_option_runtime_text_context_not_visible() {
        use replay_core::FiredEventOptionTextContextRow;

        let (mut row, def, loc, sprites, runtime) = popup_fixture();
        row.runtime_option_text_contexts = Leaf::Measured(vec![FiredEventOptionTextContextRow {
            definition_ordinal: Leaf::Measured(9),
            context_id: Leaf::Measured("ctx:extra".into()),
        }]);
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::OptionRuntimeTextContextNotVisible,
        );
    }

    #[test]
    fn popup_suppression_title_runtime_text_context_missing() {
        let (mut row, def, loc, sprites, runtime) = popup_fixture();
        row.runtime_title_text_context_id = Leaf::Measured("ctx:missing-title".into());
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::TitleRuntimeTextContextMissing,
        );
    }

    #[test]
    fn popup_suppression_description_runtime_text_context_missing() {
        let (mut row, def, loc, sprites, runtime) = popup_fixture();
        row.runtime_description_text_context_id = Leaf::Measured("ctx:missing-description".into());
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::DescriptionRuntimeTextContextMissing,
        );
    }

    #[test]
    fn popup_suppression_option_runtime_text_context_missing() {
        use replay_core::FiredEventOptionTextContextRow;

        let (mut row, def, loc, sprites, runtime) = popup_fixture();
        row.runtime_option_text_contexts = Leaf::Measured(vec![FiredEventOptionTextContextRow {
            definition_ordinal: Leaf::Measured(0),
            context_id: Leaf::Measured("ctx:missing-option".into()),
        }]);
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::OptionRuntimeTextContextMissing,
        );
    }

    #[test]
    fn popup_suppression_title_key_missing() {
        let (row, mut def, loc, sprites, runtime) = popup_fixture();
        def.title_key = None;
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::TitleKeyMissing,
        );
    }

    #[test]
    fn popup_suppression_title_text_unresolved() {
        let (row, def, mut loc, sprites, runtime) = popup_fixture();
        loc.entries.remove("generic.1.t");
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::TitleTextUnresolved,
        );
    }

    #[test]
    fn popup_suppression_description_text_unresolved() {
        let (row, def, mut loc, sprites, runtime) = popup_fixture();
        loc.entries.remove("generic.1.d_good");
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::DescriptionTextUnresolved,
        );
    }

    #[test]
    fn popup_suppression_option_name_missing() {
        let (row, mut def, loc, sprites, runtime) = popup_fixture();
        def.options[0].name_key = None;
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::OptionNameMissing,
        );
    }

    #[test]
    fn popup_suppression_option_text_unresolved() {
        let (row, def, mut loc, sprites, runtime) = popup_fixture();
        loc.entries.remove("generic.1.a");
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::OptionTextUnresolved,
        );
    }

    #[test]
    fn popup_suppression_picture_sprite_unavailable() {
        let (row, def, loc, _sprites, runtime) = popup_fixture();
        assert_suppression(
            row,
            def,
            &loc,
            &SpriteRegistry::new(),
            &runtime,
            PopupSuppression::PictureSpriteUnavailable,
        );
    }

    #[test]
    fn xui21_complete_event_namespace_requires_exact_declared_admitted_and_resolved_sets() {
        assert!(is_event_definition_path("events/Germany/test.TXT"));
        assert!(!is_event_definition_path("common/events/test.txt"));
        assert!(!is_event_definition_path("events/test.gui"));

        let complete = BTreeSet::from([
            "events/base.txt".to_string(),
            "events/mod/nested.txt".to_string(),
        ]);
        validate_event_definition_namespace_sets(&complete, &complete, &complete)
            .expect("exact complete event namespace");
        assert!(validate_event_definition_namespace_sets(
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeSet::new(),
        )
        .is_err());
        assert!(validate_event_definition_namespace_sets(
            &complete,
            &BTreeSet::from(["events/base.txt".to_string()]),
            &complete,
        )
        .is_err());
        assert!(validate_event_definition_namespace_sets(
            &complete,
            &complete,
            &BTreeSet::from(["events/base.txt".to_string()]),
        )
        .is_err());
    }

    #[test]
    fn popup_suppression_single_authored_description_still_requires_runtime_selection() {
        let (mut row, mut def, loc, sprites, runtime) = popup_fixture();
        def.desc_keys.truncate(1);
        assert_eq!(def.desc_keys.len(), 1);
        row.selected_desc_key = Leaf::Unobtained;
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::DescriptionBranchUndetermined,
        );
    }

    #[test]
    fn declared_picture_cannot_treat_wire_null_as_an_authored_default() {
        let (mut row, def, loc, sprites, runtime) = popup_fixture();
        assert!(def.picture_gfx.is_some(), "fixture must declare a picture");
        row.shown_picture_gfx = Leaf::Unobtained;
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::PictureSelectionUnavailable,
        );
    }

    #[test]
    fn visible_at_closes_at_timeout_when_resolution_is_late() {
        let (mut row, def, loc, sprites, runtime) = popup_fixture();
        row.resolved_tick = Leaf::Measured(2_500);
        let admitted = admit_fixture(row, def, &loc, &sprites, &runtime);
        assert_eq!(admitted.popup_suppression, None);
        assert!(admitted.visible_at(1, 1_511));
        assert!(!admitted.visible_at(1, 1_512));
        assert!(!admitted.visible_at(1, 2_499));
    }

    #[test]
    fn xui21_rejects_contradictory_timeout_arithmetic_and_pre_due_timeout_resolution() {
        let (row, def, loc, sprites, runtime) = popup_fixture();
        let definitions = BTreeMap::from([(def.id.clone(), def)]);
        let reject = |row: RecordedFiredEventRow| {
            let history = FiredEventsView {
                events: Leaf::Measured(vec![row]),
                history_complete: Leaf::Measured(true),
                hook_coverage_complete: Leaf::Measured(true),
                prior_to_capture: Leaf::Measured(Vec::new()),
            };
            admit_event_history(EventHistoryRequest {
                source_record_id: "record:timeout-negative",
                history: &history,
                definitions: &definitions,
                localisation: &loc,
                sprites: &sprites,
                runtime_text: &runtime,
            })
            .expect_err("contradictory lifecycle must fail closed")
        };

        let mut wrong_arithmetic = row.clone();
        wrong_arithmetic.timeout_tick = Leaf::Measured(1513);
        assert_eq!(
            reject(wrong_arithmetic),
            EventHistoryRejection::InvalidTimeout
        );

        let mut pre_due_timeout = row;
        pre_due_timeout.resolution_reason = Leaf::Measured("timeout".into());
        pre_due_timeout.resolved_tick = Leaf::Measured(1511);
        assert_eq!(
            reject(pre_due_timeout),
            EventHistoryRejection::InvalidResolution
        );
    }

    #[test]
    fn xui21_admits_exact_history_and_visible_presentation() {
        let defs = parse_events_document("events/test.txt", &sample_events_doc());
        let definitions = event_definitions_by_id(&EventCorpus {
            defs,
            files_parsed: 1,
            parse_failures: Vec::new(),
            decode_lossy_files: Vec::new(),
        })
        .unwrap();
        let history = FiredEventsView {
            events: Leaf::Measured(vec![rich_row()]),
            history_complete: Leaf::Measured(true),
            hook_coverage_complete: Leaf::Measured(true),
            prior_to_capture: Leaf::Measured(Vec::new()),
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
        assert_eq!(event.popup_suppression, None);
        let popup = event.popup.as_ref().expect("materialised popup");
        assert_eq!(popup.window_name, "EventWindow");
        assert_eq!(popup.selected_desc_key, "generic.1.d_good");
        assert_eq!(popup.options.len(), 2);
        assert_eq!(popup.options[0].definition_ordinal, 0);
        assert!(!popup.options[0].selected);
        assert!(popup.options[1].selected);
        assert!(event.visible_at(1, 1200));
        assert!(event.visible_at(1, 1209));
        assert!(!event.visible_at(1, 1210));
        assert!(!event.visible_at(2, 1205));
        assert_eq!(
            event.recipient_tag_db_id, 1,
            "the recipient identity reaches the UI rather than being spent on the TAG"
        );

        // THE CROSS-HOUR CASE, which the accumulated-history check cannot see.
        // The event was recorded against "GER"/1. On a later hour "GER" may
        // denote country 2 -- the history holds only one (TAG, id) pair, so it
        // admits, and a by-TAG join would show 1's event to 2. The join is by
        // id, so it does not.
        assert!(
            !event.visible_at(2, 1200),
            "an event addressed to country 1 is not shown to country 2, whatever              TAG either displays now"
        );
        assert!(event.visible_at(1, 1200), "and its own country still sees it");
        assert_eq!(
            event.recipient_tag_db_id, 1,
            "the recipient identity reaches the UI rather than being spent on the TAG"
        );

        // TWO IDS, ONE TAG, across the accumulated history. The join below is by
        // TAG, and each hour on its own is internally consistent, so this is the
        // check that makes the join sound: the whole history is refused rather
        // than showing one country's event to the other.
        let mut second = rich_row();
        second.recipient_tag_db_id = Leaf::Measured(2);
        second.event_sequence = Leaf::Measured(9);
        let fused = FiredEventsView {
            events: Leaf::Measured(vec![rich_row(), second]),
            history_complete: Leaf::Measured(true),
            hook_coverage_complete: Leaf::Measured(true),
            prior_to_capture: Leaf::Measured(Vec::new()),
        };
        assert!(
            matches!(
                admit_event_history(EventHistoryRequest {
                    source_record_id: "record:1",
                    history: &fused,
                    definitions: &definitions,
                    localisation: &loc,
                    sprites: &sprites,
                    runtime_text: &BTreeMap::new(),
                }),
                Err(EventHistoryRejection::InvalidIdentity)
            ),
            "one TAG reached by two ids is two countries, not one recipient"
        );

        let mut layout = LayoutSet::default();
        layout.add_document(
            "interface/eventwindow.gui",
            &parse_str(
                r#"guiTypes = {
                    containerWindowType = {
                        name = "EventWindow"
                        size = { width = 581 height = 427 }
                        iconType = {
                            name = "event_picture"
                            spriteType = "GFX_report_event_001"
                        }
                        gridBoxType = {
                            name = "options_grid"
                            size = { width = 300 height = 30 }
                            slotsize = { width = 173 height = 29 }
                            max_slots_horizontal = 2
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
        let presentation = event_popup_presentation(event, &layout, 1, 1205)
            .expect("compose-ready popup presentation");
        assert_eq!(
            presentation.chrome.options_slot_size,
            Some(("173".into(), "29".into()))
        );
        assert_eq!(presentation.chrome.options_max_slots_horizontal, Some(2));
        assert_eq!(
            presentation.option_entry.size,
            Some(("300".into(), "47".into()))
        );
        assert_eq!(presentation.popup.options.len(), 2);
        assert_eq!(
            event_popup_presentation(event, &layout, 1, 1210),
            Err(EventPresentationRejection::NotVisible)
        );
    }

    #[test]
    fn xui21_runtime_text_uses_independent_title_description_and_option_contexts() {
        use crate::text_law::ExactRuntimeText;
        use replay_core::{FiredEventOptionTextContextRow, RuntimeTextLayout, RuntimeTextSource};

        fn exact(key: &str, element_id: &str, text: &str) -> RuntimeTextContext {
            RuntimeTextContext {
                exact: Some(ExactRuntimeText {
                    view_id: "EventWindow".into(),
                    element_id: element_id.into(),
                    source: RuntimeTextSource::AuthoredToken(key.into()),
                    engine_utf8: text.into(),
                    layout: RuntimeTextLayout::AuthoredReproducible,
                }),
                ..Default::default()
            }
        }

        let defs = parse_events_document("events/test.txt", &sample_events_doc());
        let definitions = event_definitions_by_id(&EventCorpus {
            defs,
            files_parsed: 1,
            parse_failures: Vec::new(),
            decode_lossy_files: Vec::new(),
        })
        .unwrap();
        let mut row = rich_row();
        row.runtime_title_text_context_id = Leaf::Measured("ctx:title".into());
        row.runtime_description_text_context_id = Leaf::Measured("ctx:description".into());
        row.runtime_option_text_contexts = Leaf::Measured(vec![
            FiredEventOptionTextContextRow {
                definition_ordinal: Leaf::Measured(0),
                context_id: Leaf::Measured("ctx:option:0".into()),
            },
            FiredEventOptionTextContextRow {
                definition_ordinal: Leaf::Measured(1),
                context_id: Leaf::Measured("ctx:option:1".into()),
            },
        ]);
        let history = FiredEventsView {
            events: Leaf::Measured(vec![row.clone()]),
            history_complete: Leaf::Measured(true),
            hook_coverage_complete: Leaf::Measured(true),
            prior_to_capture: Leaf::Measured(Vec::new()),
        };
        let runtime_text = BTreeMap::from([
            (
                "ctx:title".into(),
                exact("generic.1.t", "Title", "Runtime title 17"),
            ),
            (
                "ctx:description".into(),
                exact("generic.1.d_good", "Description", "Scoped GER description"),
            ),
            (
                "ctx:option:0".into(),
                exact("generic.1.a", "Name", "Formatted option A 42%"),
            ),
            (
                "ctx:option:1".into(),
                exact("generic.1.b", "Name", "Formatted option B 73%"),
            ),
        ]);
        let mut sprites = SpriteRegistry::new();
        sprites.insert(sprite_stub("GFX_report_event_generic_read_write"));
        let admitted = admit_event_history(EventHistoryRequest {
            source_record_id: "record:dynamic-popup",
            history: &history,
            definitions: &definitions,
            localisation: &Localisation::default(),
            sprites: &sprites,
            runtime_text: &runtime_text,
        })
        .expect("admit lifecycle");
        let popup = admitted[0].popup.as_ref().expect("exact dynamic popup");
        assert_eq!(popup.title, "Runtime title 17");
        assert_eq!(popup.description, "Scoped GER description");
        assert_eq!(popup.options[0].text, "Formatted option A 42%");
        assert_eq!(popup.options[1].text, "Formatted option B 73%");

        row.runtime_title_text_context_id = Leaf::Measured("ctx:description".into());
        let mismatched = FiredEventsView {
            events: Leaf::Measured(vec![row]),
            history_complete: Leaf::Measured(true),
            hook_coverage_complete: Leaf::Measured(true),
            prior_to_capture: Leaf::Measured(Vec::new()),
        };
        let admitted = admit_event_history(EventHistoryRequest {
            source_record_id: "record:dynamic-popup-mismatch",
            history: &mismatched,
            definitions: &definitions,
            localisation: &Localisation::default(),
            sprites: &sprites,
            runtime_text: &runtime_text,
        })
        .expect("retain lifecycle on popup text mismatch");
        assert_eq!(
            admitted[0].popup_suppression,
            Some(PopupSuppression::TitleTextUnresolved)
        );
        assert_eq!(admitted[0].popup, None);
    }

    #[test]
    fn xui21_rejects_legacy_or_inferred_history() {
        let history = FiredEventsView {
            events: Leaf::Measured(vec![RecordedFiredEventRow {
                event_id: Leaf::Measured("generic.1".into()),
                ..rich_row()
            }]),
            history_complete: Leaf::Measured(false),
            hook_coverage_complete: Leaf::Measured(false),
            prior_to_capture: Leaf::Measured(Vec::new()),
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
            decode_lossy_files: Vec::new(),
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
            hook_coverage_complete: Leaf::Measured(true),
            prior_to_capture: Leaf::Measured(Vec::new()),
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
        assert_eq!(
            admitted[0].popup_suppression,
            Some(PopupSuppression::DefinitionHidden)
        );
        assert_eq!(
            admitted[1].popup_suppression,
            Some(PopupSuppression::VisibleOptionOrdinalsEmpty)
        );
        assert!(admitted.iter().all(|event| event.popup.is_none()));
        assert_eq!(admitted[1].chosen_option_ordinal, Some(1));
        assert_eq!(
            event_popup_presentation(&admitted[1], &LayoutSet::default(), 1, 1205),
            Err(EventPresentationRejection::NotMaterialised)
        );
    }

    #[test]
    fn xui21_pre_attach_fires_are_refused_by_name_not_as_a_generic_gap() {
        // Rows measured, hooks clean -- but the observer attached after these two
        // had fired. The rejection must carry them, so the observer gap says what
        // is missing instead of only that something is.
        let late = FiredEventsView {
            events: Leaf::Measured(Vec::new()),
            history_complete: Leaf::Measured(false),
            hook_coverage_complete: Leaf::Measured(true),
            prior_to_capture: Leaf::Measured(vec!["france.10".into(), "germany.1".into()]),
        };
        let definitions = BTreeMap::new();
        let localisation = Localisation::default();
        let sprites = SpriteRegistry::new();
        let runtime_text = BTreeMap::new();
        fn request<'a>(
            history: &'a FiredEventsView,
            definitions: &'a BTreeMap<String, EventDef>,
            localisation: &'a Localisation,
            sprites: &'a SpriteRegistry,
            runtime_text: &'a BTreeMap<String, RuntimeTextContext>,
        ) -> EventHistoryRequest<'a> {
            EventHistoryRequest {
                source_record_id: "record:late-attach",
                history,
                definitions,
                localisation,
                sprites,
                runtime_text,
            }
        }
        assert_eq!(
            admit_event_history(request(&late, &definitions, &localisation, &sprites, &runtime_text)),
            Err(EventHistoryRejection::EventsFiredBeforeObservation(vec![
                "france.10".into(),
                "germany.1".into()
            ]))
        );

        // A hook-coverage gap with nothing pre-dating the attach keeps the
        // generic cause: the three are different failures.
        let hook_gap = FiredEventsView {
            history_complete: Leaf::Measured(false),
            hook_coverage_complete: Leaf::Measured(false),
            prior_to_capture: Leaf::Measured(Vec::new()),
            ..late.clone()
        };
        assert_eq!(
            admit_event_history(request(&hook_gap, &definitions, &localisation, &sprites, &runtime_text)),
            Err(EventHistoryRejection::HistoryIncomplete)
        );

        // Clean detours, measured rows, but the retained set would not name
        // itself: whether anything predates observation is unknown, and that is
        // its own cause -- not the hook-coverage one.
        let unnameable = FiredEventsView {
            history_complete: Leaf::Measured(false),
            hook_coverage_complete: Leaf::Measured(true),
            prior_to_capture: Leaf::Unobtained,
            ..late.clone()
        };
        assert_eq!(
            admit_event_history(request(&unnameable, &definitions, &localisation, &sprites, &runtime_text)),
            Err(EventHistoryRejection::PreAttachBaselineUnobtained)
        );

        // And an attach with an empty retained set is admitted as before.
        let clean = FiredEventsView {
            history_complete: Leaf::Measured(true),
            hook_coverage_complete: Leaf::Measured(true),
            prior_to_capture: Leaf::Measured(Vec::new()),
            ..late
        };
        assert_eq!(admit_event_history(request(&clean, &definitions, &localisation, &sprites, &runtime_text)), Ok(Vec::new()));
    }

    #[test]
    fn xui21_measured_empty_complete_history_admits_zero_rows() {
        let history = FiredEventsView {
            events: Leaf::Measured(Vec::new()),
            history_complete: Leaf::Measured(true),
            hook_coverage_complete: Leaf::Measured(true),
            prior_to_capture: Leaf::Measured(Vec::new()),
        };
        let admitted = admit_event_history(EventHistoryRequest {
            source_record_id: "record:empty",
            history: &history,
            definitions: &BTreeMap::new(),
            localisation: &Localisation::default(),
            sprites: &SpriteRegistry::new(),
            runtime_text: &BTreeMap::new(),
        })
        .expect("measured-empty complete history is admissible");
        assert!(admitted.is_empty());

        let unobserved = FiredEventsView {
            events: Leaf::Unobtained,
            history_complete: Leaf::Measured(true),
            hook_coverage_complete: Leaf::Measured(true),
            prior_to_capture: Leaf::Measured(Vec::new()),
        };
        assert_eq!(
            admit_event_history(EventHistoryRequest {
                source_record_id: "record:empty",
                history: &unobserved,
                definitions: &BTreeMap::new(),
                localisation: &Localisation::default(),
                sprites: &SpriteRegistry::new(),
                runtime_text: &BTreeMap::new(),
            }),
            Err(EventHistoryRejection::HistoryUnavailable)
        );
    }

    #[test]
    fn xui21_leader_and_operative_popups_require_exact_runtime_bindings() {
        use crate::text_law::ExactRuntimeText;
        use replay_core::{RuntimeTextLayout, RuntimeTextSource};

        let (mut row, mut def, loc, mut sprites, mut runtime) = popup_fixture();
        sprites.insert(sprite_stub("GFX_portrait_unknown"));
        sprites.insert(sprite_stub("GFX_intelligence_agency_logo_ger"));
        row.shown_picture_gfx = Leaf::Unobtained;
        row.leader_portrait_identity_kind = Leaf::Measured("gfx_name".into());
        row.leader_portrait_identity = Leaf::Measured("GFX_portrait_unknown".into());
        row.popup_subtype = Leaf::Measured("unit_leader".into());
        def.kind = EventKind::UnitLeader;
        let admitted = admit_fixture(row.clone(), def.clone(), &loc, &sprites, &runtime);
        let popup = admitted.popup.expect("exact unit-leader popup admitted");
        assert_eq!(popup.window_name, "EventWindow_leader");
        assert_eq!(
            popup.leader_portrait,
            Some(EventRuntimeImageIdentity::GfxName(
                "GFX_portrait_unknown".into()
            ))
        );
        assert_eq!(popup.picture_gfx, None);

        let exact = |element: &str, text: &str| RuntimeTextContext {
            exact: Some(ExactRuntimeText {
                view_id: "EventWindow_Operative".into(),
                element_id: element.into(),
                source: RuntimeTextSource::SetterCallerRva(0x0122_5b30),
                engine_utf8: text.into(),
                layout: RuntimeTextLayout::AuthoredReproducible,
            }),
            ..Default::default()
        };
        row.popup_subtype = Leaf::Measured("operative_leader".into());
        row.runtime_leader_fullname_text_context_id = Leaf::Measured("ctx:fullname".into());
        row.runtime_leader_codename_text_context_id = Leaf::Measured("ctx:codename".into());
        row.agency_insignia_identity_kind = Leaf::Measured("file_path".into());
        row.agency_insignia_identity =
            Leaf::Measured("gfx/interface/agency_logo_ger.dds".into());
        runtime.insert(
            "ctx:fullname".into(),
            exact("leader_fullname", "Jane Doe"),
        );
        runtime.insert(
            "ctx:codename".into(),
            exact("leader_codename", "Sparrow"),
        );
        def.kind = EventKind::OperativeLeader;
        let admitted = admit_fixture(row.clone(), def.clone(), &loc, &sprites, &runtime);
        let popup = admitted.popup.expect("exact operative popup admitted");
        assert_eq!(popup.window_name, "EventWindow_Operative");
        assert_eq!(popup.leader_fullname.as_ref().unwrap().0, "Jane Doe");
        assert_eq!(popup.leader_codename.as_ref().unwrap().0, "Sparrow");
        assert_eq!(
            popup.agency_insignia,
            Some(EventRuntimeImageIdentity::FilePath(
                "gfx/interface/agency_logo_ger.dds".into()
            ))
        );

        row.runtime_leader_codename_text_context_id = Leaf::Unobtained;
        assert_suppression(
            row,
            def,
            &loc,
            &sprites,
            &runtime,
            PopupSuppression::LeaderCodenameRuntimeTextUnavailable,
        );
    }

    #[test]
    fn xui21_viewer_focus_dismiss_and_reset_never_mutate_lifecycle() {
        let mut viewer = EventPopupViewerState::default();
        let ordered = [4, 9, 12];
        viewer.cycle_focus(&ordered);
        assert_eq!(viewer.focused_event_sequence, Some(4));
        viewer.cycle_focus(&ordered);
        assert_eq!(viewer.focused_event_sequence, Some(9));
        assert!(viewer.focus(12, &ordered));
        assert_eq!(viewer.dismiss_focused(), Some(12));
        assert!(viewer.dismissed_event_sequences.contains(&12));
        assert!(!viewer.focus(99, &ordered));
        viewer.reset_for_seek_or_country_change();
        assert_eq!(viewer, EventPopupViewerState::default());
    }

    #[test]
    fn xui21_pending_only_rows_never_materialise_a_popup() {
        let doc = parse_str(
            r#"
            add_namespace = der3
            state_event = {
                id = der3.1
                title = der3.1.t
                desc = { text = der3.1.d_a trigger = { has_government = fascism } }
                desc = { text = der3.1.d_b trigger = { NOT = { has_government = fascism } } }
                picture = GFX_report_event_der3
                is_triggered_only = yes
                option = { name = der3.1.a }
                option = { name = der3.1.b trigger = { has_war = yes } }
            }
            "#,
        );
        let definitions = event_definitions_by_id(&EventCorpus {
            defs: parse_events_document("events/der3.txt", &doc),
            files_parsed: 1,
            parse_failures: Vec::new(),
            decode_lossy_files: Vec::new(),
        })
        .unwrap();
        let der3 = &definitions["der3.1"];
        assert_eq!(der3.desc_keys.len(), 2, "fixture needs two desc branches");
        assert_eq!(der3.options.len(), 2);
        assert!(!der3.options[0].has_trigger, "option 0 is unconditional");
        assert!(
            der3.options[1].has_trigger,
            "option 1 must be trigger-guarded"
        );
        assert!(!der3.hidden);

        let mut loc = Localisation::default();
        for (key, text) in [
            ("der3.1.t", "Title"),
            ("der3.1.d_a", "Desc A"),
            ("der3.1.d_b", "Desc B"),
            ("der3.1.a", "Option A"),
            ("der3.1.b", "Option B"),
        ] {
            loc.entries.insert(key.into(), text.into());
        }
        let mut sprites = SpriteRegistry::new();
        sprites.insert(sprite_stub("GFX_report_event_der3"));

        let save_derivable = || {
            let mut row = rich_row();
            row.event_id = Leaf::Measured("der3.1".into());
            row.recipient_tag = Leaf::Measured("GER".into());
            row.effective_timeout_hours = Leaf::Measured(312);
            row.timeout_tick = Leaf::Measured(2000);
            row.fire_tick = Leaf::Measured(2000 - 312);
            row.resolution_reason = Leaf::Measured("pending".into());
            row.resolved_tick = Leaf::Absent;
            row.chosen_option_ordinal = Leaf::Absent;
            row.visible_option_ordinals = Leaf::Absent;
            row.popup_subtype = Leaf::Absent;
            row.selected_desc_key = Leaf::Absent;
            row.shown_picture_gfx = Leaf::Absent;
            row.runtime_title_text_context_id = Leaf::Absent;
            row.runtime_description_text_context_id = Leaf::Absent;
            row.runtime_option_text_contexts = Leaf::Absent;
            row
        };
        let materialised = || {
            let mut row = save_derivable();
            row.visible_option_ordinals = Leaf::Measured(vec![0, 1]);
            row.popup_subtype = Leaf::Measured("state".into());
            row.selected_desc_key = Leaf::Measured("der3.1.d_a".into());
            row.shown_picture_gfx = Leaf::Measured("GFX_report_event_der3".into());
            row
        };
        let admit = |row: RecordedFiredEventRow| {
            let history = FiredEventsView {
                events: Leaf::Measured(vec![row]),
                history_complete: Leaf::Measured(true),
                hook_coverage_complete: Leaf::Measured(true),
                prior_to_capture: Leaf::Measured(Vec::new()),
            };
            admit_event_history(EventHistoryRequest {
                source_record_id: "record:save-derived",
                history: &history,
                definitions: &definitions,
                localisation: &loc,
                sprites: &sprites,
                runtime_text: &BTreeMap::new(),
            })
            .expect("save-derivable lifecycle is admissible as lifecycle")
        };

        let admitted = admit(save_derivable());
        assert_eq!(admitted.len(), 1);
        assert_eq!(admitted[0].resolution_reason, "pending");
        assert_eq!(admitted[0].fire_tick, 1688);
        assert_eq!(admitted[0].timeout_tick, 2000);
        assert_eq!(admitted[0].effective_timeout_hours, 312);
        assert_eq!(
            admitted[0].popup_suppression,
            Some(PopupSuppression::VisibleOptionOrdinalsAbsent)
        );
        assert_eq!(admitted[0].popup, None);
        assert!(!admitted[0].visible_at(1, 1700));
        assert_eq!(
            event_popup_presentation(&admitted[0], &LayoutSet::default(), 1, 1700),
            Err(EventPresentationRejection::NotMaterialised)
        );

        let admitted = admit(materialised());
        let popup = admitted[0].popup.as_ref().expect("materialised popup");
        assert_eq!(popup.selected_desc_key, "der3.1.d_a");
        assert_eq!(popup.options.len(), 2);
        assert!(admitted[0].visible_at(1, 1700));

        let mut missing_visible = materialised();
        missing_visible.visible_option_ordinals = Leaf::Absent;
        let admitted = admit(missing_visible);
        assert_eq!(
            admitted[0].popup_suppression,
            Some(PopupSuppression::VisibleOptionOrdinalsAbsent)
        );

        let mut missing_desc = materialised();
        missing_desc.selected_desc_key = Leaf::Absent;
        let admitted = admit(missing_desc);
        assert_eq!(
            admitted[0].popup_suppression,
            Some(PopupSuppression::DescriptionBranchUndetermined)
        );

        let mut contradictory_capture = materialised();
        contradictory_capture.popup_subtype = Leaf::Measured("operative_leader".into());
        contradictory_capture.shown_picture_gfx =
            Leaf::Measured("GFX_captured_country_variant".into());
        let admitted = admit(contradictory_capture);
        assert_eq!(
            admitted[0].popup_suppression,
            Some(PopupSuppression::WindowVariantUndetermined)
        );
        assert!(admitted[0].popup.is_none());
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
            decode_lossy_files: Vec::new(),
        })
        .unwrap();
        let mut zero = rich_row();
        zero.event_id = Leaf::Measured("generic.2".into());
        zero.visible_option_ordinals = Leaf::Measured(Vec::new());
        zero.chosen_option_ordinal = Leaf::Absent;
        zero.selected_desc_key = Leaf::Measured("generic.2.d".into());
        zero.shown_picture_gfx = Leaf::Measured("GFX_report_event_zero".into());
        zero.resolution_reason = Leaf::Measured("timeout".into());
        zero.resolved_tick = Leaf::Measured(1512);
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
            hook_coverage_complete: Leaf::Measured(true),
            prior_to_capture: Leaf::Measured(Vec::new()),
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
        assert_eq!(
            admitted[0].popup_suppression,
            Some(PopupSuppression::VisibleOptionOrdinalsEmpty)
        );
        assert_eq!(
            admitted[1].popup_suppression,
            Some(PopupSuppression::DescriptionBranchUndetermined)
        );
    }

    #[test]
    fn xui21_accepts_timeout_default_ordinal_and_uses_definition_picture() {
        let defs = parse_events_document("events/test.txt", &sample_events_doc());
        let definitions = event_definitions_by_id(&EventCorpus {
            defs,
            files_parsed: 1,
            parse_failures: Vec::new(),
            decode_lossy_files: Vec::new(),
        })
        .unwrap();
        let mut row = rich_row();
        row.chosen_option_ordinal = Leaf::Measured(0);
        row.resolution_reason = Leaf::Measured("timeout".into());
        row.resolved_tick = Leaf::Measured(1512);
        row.shown_picture_gfx = Leaf::Measured("GFX_report_event_GER_generic_read_write".into());
        let history = FiredEventsView {
            events: Leaf::Measured(vec![row]),
            history_complete: Leaf::Measured(true),
            hook_coverage_complete: Leaf::Measured(true),
            prior_to_capture: Leaf::Measured(Vec::new()),
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
        sprites.insert(sprite_stub("GFX_report_event_GER_generic_read_write"));
        let admitted = admit_event_history(EventHistoryRequest {
            source_record_id: "record:timeout-variant",
            history: &history,
            definitions: &definitions,
            localisation: &loc,
            sprites: &sprites,
            runtime_text: &BTreeMap::new(),
        })
        .expect("admit timeout default and definition picture");
        let popup = admitted[0].popup.as_ref().expect("materialised popup");
        assert_eq!(admitted[0].resolution_reason, "timeout");
        assert_eq!(admitted[0].chosen_option_ordinal, Some(0));
        assert!(popup.options[0].selected);
        assert_eq!(
            popup.picture_gfx.as_deref(),
            Some("GFX_report_event_GER_generic_read_write")
        );
    }

    #[test]
    fn xui21_pictureless_definition_uses_only_the_authored_chrome_default() {
        let definitions = event_definitions_by_id(&EventCorpus {
            defs: parse_events_document(
                "events/pictureless.txt",
                &parse_str(
                    r#"
                    state_event = {
                        id = pictureless.1
                        title = pictureless.1.t
                        desc = pictureless.1.d
                        option = { name = pictureless.1.a }
                    }
                    "#,
                ),
            ),
            files_parsed: 1,
            parse_failures: Vec::new(),
            decode_lossy_files: Vec::new(),
        })
        .expect("index pictureless definition");
        let definition = definitions
            .get("pictureless.1")
            .expect("pictureless definition parsed");
        assert_eq!(
            definition.picture_gfx, None,
            "fixture must not declare a picture"
        );

        let mut row = rich_row();
        row.event_id = Leaf::Measured("pictureless.1".into());
        row.popup_subtype = Leaf::Measured("state".into());
        row.visible_option_ordinals = Leaf::Measured(vec![0]);
        row.chosen_option_ordinal = Leaf::Measured(0);
        row.selected_desc_key = Leaf::Measured("pictureless.1.d".into());
        row.shown_picture_gfx = Leaf::Unobtained;
        let history = FiredEventsView {
            events: Leaf::Measured(vec![row]),
            history_complete: Leaf::Measured(true),
            hook_coverage_complete: Leaf::Measured(true),
            prior_to_capture: Leaf::Measured(Vec::new()),
        };
        let mut localisation = Localisation::default();
        for (key, text) in [
            ("pictureless.1.t", "Pictureless title"),
            ("pictureless.1.d", "Pictureless description"),
            ("pictureless.1.a", "Pictureless option"),
        ] {
            localisation.entries.insert(key.into(), text.into());
        }
        let admitted = admit_event_history(EventHistoryRequest {
            source_record_id: "record:pictureless",
            history: &history,
            definitions: &definitions,
            localisation: &localisation,
            sprites: &SpriteRegistry::new(),
            runtime_text: &BTreeMap::new(),
        })
        .expect("pictureless lifecycle remains admissible");
        assert_eq!(admitted[0].popup_suppression, None);
        let popup = admitted[0]
            .popup
            .as_ref()
            .expect("pictureless definition materialises");
        assert_eq!(
            popup.picture_gfx, None,
            "neither the retired capture nor a Rust fallback may invent a definition picture"
        );

        let mut layout = LayoutSet::default();
        layout.add_document(
            "interface/mod_popup/authored_popup_chrome.gui",
            &parse_str(
                r#"
                guiTypes = {
                    containerWindowType = {
                        name = "EventWindow"
                        iconType = {
                            name = "event_picture"
                            spriteType = "GFX_mod_authored_country_default"
                        }
                        gridBoxType = {
                            name = "options_grid"
                            slotsize = { width = 300 height = 47 }
                            max_slots_horizontal = 1
                        }
                    }
                    containerWindowType = {
                        name = "event_option_entry"
                        size = { width = 300 height = 47 }
                        instantTextBoxType = { name = "Name" }
                    }
                }
                "#,
            ),
        );
        let presentation = event_popup_presentation(&admitted[0], &layout, 1, 1205)
            .expect("pictureless event joins to authored chrome");
        assert_eq!(
            presentation
                .chrome
                .picture
                .and_then(|picture| picture.sprite),
            Some("GFX_mod_authored_country_default".into())
        );

        let mut missing_picture_slot = LayoutSet::default();
        missing_picture_slot.add_document(
            "interface/mod_popup/missing_picture_slot.gui",
            &parse_str(
                r#"
                guiTypes = {
                    containerWindowType = {
                        name = "EventWindow"
                        gridBoxType = {
                            name = "options_grid"
                            slotsize = { width = 300 height = 47 }
                            max_slots_horizontal = 1
                        }
                    }
                    containerWindowType = {
                        name = "event_option_entry"
                        size = { width = 300 height = 47 }
                        instantTextBoxType = { name = "Name" }
                    }
                }
                "#,
            ),
        );
        assert_eq!(
            event_popup_presentation(&admitted[0], &missing_picture_slot, 1, 1205),
            Err(EventPresentationRejection::MissingPictureSlot)
        );

        let mut missing_default_sprite = LayoutSet::default();
        missing_default_sprite.add_document(
            "interface/mod_popup/missing_default_sprite.gui",
            &parse_str(
                r#"
                guiTypes = {
                    containerWindowType = {
                        name = "EventWindow"
                        iconType = { name = "event_picture" }
                        gridBoxType = {
                            name = "options_grid"
                            slotsize = { width = 300 height = 47 }
                            max_slots_horizontal = 1
                        }
                    }
                    containerWindowType = {
                        name = "event_option_entry"
                        size = { width = 300 height = 47 }
                        instantTextBoxType = { name = "Name" }
                    }
                }
                "#,
            ),
        );
        assert_eq!(
            event_popup_presentation(&admitted[0], &missing_default_sprite, 1, 1205),
            Err(EventPresentationRejection::MissingDefaultPictureSprite)
        );
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
