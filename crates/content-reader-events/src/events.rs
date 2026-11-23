use content_reader_key_names::key_name_products::KeyNamesExt as _;
use content_reader_script::native::{self, Shape};
use crate::{
    ResolvedContent, ResolvedLayers,
    key_names::KeyNames,
    script_files::ScriptPath,
    parser::{
        Item,
    },
};
use content_reader_layers::content::diagnostics::Observed;
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Deref,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Event {
    pub id: Vec<u8>,
    pub source: String,
    pub titles: Vec<Item>,
    pub descriptions: Vec<Item>,
    pub picture: Vec<u8>,
    pub option_triggers: Vec<Vec<Item>>,
    pub original_recipient_only: Vec<Option<bool>>,
    pub immediate: Vec<Item>,
    pub options: Vec<Vec<Item>>,
    pub option_names: Vec<Vec<u8>>,
    pub refused: Option<String>,
}

impl Event {
    fn title_key(item: &Item) -> &[u8] {
        crate::parser::c_string(
            item.value
                .scalar()
                .or_else(|| {
                    item.value
                        .items()?
                        .iter()
                        .rev()
                        .find(|field| field.keyword_is("text") || field.keyword_is("key"))?
                        .value
                        .scalar()
                })
                .unwrap_or_default(),
        )
    }

    fn common_key<'a>(
        &self,
        candidates: &'a [Item],
        field: &str,
    ) -> Result<Option<&'a [u8]>, String> {
        let Some(first) = candidates.first() else {
            return Ok(None);
        };
        let key = Self::title_key(first);
        if candidates.iter().all(|item| Self::title_key(item) == key) {
            return Ok((!key.is_empty()).then_some(key));
        }
        Err(format!(
            "{} event {} has different {field} candidates; exact selection needs historical triggers and an unrecorded high-resolution seed",
            self.source,
            String::from_utf8_lossy(&self.id)
        ))
    }
    pub fn sole_description_key(&self) -> Result<Option<&[u8]>, String> {
        self.common_key(&self.descriptions, "description")
    }
    pub fn sole_title_key(&self) -> Result<Option<&[u8]>, String> {
        self.common_key(&self.titles, "title")
    }

    pub fn deterministic_title_key(
        &self,
        mut matches: impl FnMut(&[Item]) -> Result<bool, String>,
    ) -> Result<TitleKey<'_>, String> {
        let Some(first) = self.titles.first() else {
            return Ok(TitleKey::default());
        };
        if self.titles.len() == 1 {
            return Ok(TitleKey {
                key: self.sole_title_key()?,
                unsettled: None,
            });
        }
        let common = self.sole_title_key().ok();
        let mut eligible = Vec::new();
        for item in &self.titles {
            let triggers: Vec<_> = item
                .value
                .items()
                .unwrap_or_default()
                .iter()
                .filter(|field| field.keyword_is("trigger"))
                .flat_map(|field| field.value.items().unwrap_or_default())
                .cloned()
                .collect();
            match (matches(&triggers), common) {
                (Ok(true), _) => eligible.push(Self::title_key(item)),
                (Ok(false), _) => {}
                (Err(why), Some(key)) => {
                    return Ok(TitleKey {
                        key,
                        unsettled: Some(why),
                    });
                }
                (Err(why), None) => return Err(why),
            }
        }
        let key = eligible
            .first()
            .copied()
            .unwrap_or_else(|| Self::title_key(first));
        if eligible.iter().any(|candidate| *candidate != key) {
            return Err(format!(
                "{} event {} has different eligible title keys; title selection needs an unrecorded high-resolution seed",
                self.source,
                String::from_utf8_lossy(&self.id)
            ));
        }
        Ok(TitleKey {
            key: (!key.is_empty()).then_some(key),
            unsettled: None,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TitleKey<'a> {
    pub key: Option<&'a [u8]>,
    pub unsettled: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Events {
    pub entries: BTreeMap<Vec<u8>, Event>,
    pub namespaces: BTreeSet<Vec<u8>>,
    pub refused: Vec<String>,
}
#[derive(Clone, Copy)]
enum Node {
    File,
    Event(Option<u32>),
    Option(Option<u32>),
    Title,
    Mtth,
}

pub fn identifier_key(id: &[u8]) -> Result<Vec<u8>, String> {
    let id = crate::parser::c_string(id);
    let dot = id.iter().rposition(|byte| *byte == b'.').ok_or_else(|| {
        format!(
            "event {} has no namespace; event lookup cannot resolve it",
            String::from_utf8_lossy(id)
        )
    })?;
    if dot == 0 {
        return Err("event namespace is empty".into());
    }
    let suffix = id[dot + 1..].iter().fold(0_i32, |value, byte| {
        value
            .wrapping_mul(10)
            .wrapping_add(i32::from(*byte as i8))
            .wrapping_sub(48)
    });
    if !(0..100_000).contains(&suffix) {
        return Err(format!(
            "event {} suffix {suffix} crosses a native namespace slot; registration order is unavailable",
            String::from_utf8_lossy(id)
        ));
    }
    let mut key: Vec<_> = id[..dot].iter().map(u8::to_ascii_lowercase).collect();
    key.push(b'.');
    key.extend(suffix.to_string().as_bytes());
    Ok(key)
}

impl Events {
    pub fn get(&self, id: &[u8]) -> Result<&Event, String> {
        let key = identifier_key(id)?;
        let dot = key.iter().rposition(|byte| *byte == b'.').unwrap();
        if !self.namespaces.contains(&key[..dot]) {
            return Err(format!(
                "event {} namespace is not registered through add_namespace",
                String::from_utf8_lossy(id)
            ));
        }
        self.entries.get(&key).ok_or_else(|| {
            format!(
                "event {} is absent from the resolved event database; {}",
                String::from_utf8_lossy(id),
                self.refused.join("; ")
            )
        })
    }
}

pub fn parse(bytes: &[u8], source: &str, names: &KeyNames) -> Events {
    read_events(bytes, names, None).selected(source)
}

fn event_kind(item: &Item) -> bool {
    matches!(
        item.keyword().map(|key| key.folded()).as_deref(),
        Some(
            b"country_event"
                | b"news_event"
                | b"state_event"
                | b"unit_leader_event"
                | b"operative_leader_event"
        )
    )
}

fn read_events(bytes: &[u8], names: &KeyNames, wanted: Option<&BTreeSet<usize>>) -> EventRead {
    let mut lines = Vec::new();
    let read = native::read(
        bytes,
        native::Mode::SkipMark,
        native::Top::File(Node::File),
        Some(names),
        &mut |node, field| match node {
            Node::File => match field.keyword().as_deref() {
                Some(
                    b"country_event"
                    | b"news_event"
                    | b"state_event"
                    | b"unit_leader_event"
                    | b"operative_leader_event",
                ) => {
                    let ordinal = lines.len();
                    lines.push(field.key.line);
                    if wanted.is_some_and(|wanted| !wanted.contains(&ordinal)) {
                        return Shape::Skip;
                    }
                    let scope = field
                        .keyword_is("operative_leader_event")
                        .then_some(crate::scope::ScopeKind::Character.bit());
                    Shape::Object(Node::Event(scope))
                }
                Some(b"add_namespace") => Shape::One,
                _ => Shape::Skip,
            },
            Node::Event(scope) => match field.keyword().as_deref() {
                Some(b"id") => Shape::One,
                Some(b"title" | b"desc") if field.opens() => Shape::Object(Node::Title),
                Some(b"title" | b"desc") => Shape::One,
                Some(b"immediate" | b"after") => scope.map_or(Shape::Effect, Shape::EffectIn),
                Some(b"option") => Shape::Object(Node::Option(scope)),
                Some(b"trigger" | b"show_major") => Shape::Trigger,
                Some(b"mean_time_to_happen") => Shape::Object(Node::Mtth),
                Some(
                    b"picture" | b"is_triggered_only" | b"major" | b"hidden" | b"fire_only_once"
                    | b"timeout_days" | b"exclusive" | b"fire_for_sender" | b"minor_flavor",
                ) => Shape::One,
                _ => Shape::Skip,
            },
            Node::Title => match field.keyword().as_deref() {
                Some(b"text" | b"key") => Shape::One,
                Some(b"trigger") => Shape::Trigger,
                _ => Shape::Skip,
            },
            Node::Option(scope) => match field.keyword().as_deref() {
                Some(b"name" | b"original_recipient_only") => Shape::One,
                Some(b"trigger") => Shape::Trigger,
                Some(b"ai_chance") => Shape::Object(Node::Mtth),
                _ => {
                    let field = native::Field {
                        effect_scope: scope.or(field.effect_scope),
                        ..*field
                    };
                    crate::effect_keys::shape(Some(names), &field).map(|_| Node::Option(scope))
                }
            },
            Node::Mtth => native::mean_time_to_happen(field),
        },
    );
    let event_items = read
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| event_kind(item))
        .map(|(index, _)| index)
        .collect();
    EventRead {
        read,
        lines,
        event_items,
    }
}

struct EventRead {
    read: native::Read,
    lines: Vec<u32>,
    event_items: Vec<usize>,
}

fn string_field<'a>(fields: &'a [Item], key: &str) -> &'a [u8] {
    fields
        .iter()
        .rfind(|field| field.keyword_is(key))
        .and_then(|field| field.value.scalar())
        .map(crate::parser::c_string)
        .unwrap_or_default()
}

fn option_name(fields: &[Item]) -> &[u8] {
    crate::parser::read_string(string_field(fields, "name"))
}

fn namespaces(read: &native::Read) -> BTreeSet<Vec<u8>> {
    read.items
        .iter()
        .filter(|item| item.keyword_is("add_namespace"))
        .filter_map(|item| item.value.scalar())
        .map(|name| crate::parser::c_string(name).to_ascii_lowercase())
        .collect()
}

impl EventRead {
    fn event(&mut self, source: &str, ordinal: usize) -> Result<Event, String> {
        let read = &mut self.read;
        let item_index = *self.event_items.get(ordinal).ok_or_else(|| {
            format!("{source}: event ordinal {ordinal} is not resolved by its native reader")
        })?;
        let item = &mut read.items[item_index];
        let operative = item.keyword_is("operative_leader_event");
        let fields =
            match std::mem::replace(&mut item.value, crate::parser::Value::Block(Vec::new())) {
                crate::parser::Value::Block(fields) => fields,
                _ => Vec::new(),
            };
        let mut event = Event {
            id: string_field(&fields, "id").to_vec(),
            source: format!("{source}:{}", self.lines.get(ordinal).copied().unwrap_or(1)),
            picture: string_field(&fields, "picture").to_vec(),
            ..Default::default()
        };
        let blockers: Vec<_> = read.item_misreads.get(item_index).into_iter().flatten()
            .chain(&read.unattributed_misreads)
            .filter(|note| matches!(note.kind, crate::DiagnosticKind::ParseFailure
                | crate::DiagnosticKind::Unsettled | crate::DiagnosticKind::Unsupported
                | crate::DiagnosticKind::Refused))
            .map(|note| {
                let description = note.to_string();
                if operative
                    && (note.what.contains("parse scope unavailable")
                        || note.what.starts_with("Deferred effect `remove_unit_leader_role`"))
                {
                    format!("the operative-leader event effect needs the scoped-object parser helper for the option scope: {description}")
                } else { description }
            }).collect();
        if !blockers.is_empty() {
            event.refused = Some(format!("{}: {}", event.source, blockers.join("; ")));
        }
        if ordinal + 1 == self.lines.len()
            && let Some(why) = &read.unsettled
        {
            let partial = format!("{}: partial native event read: {why}", event.source);
            event.refused = Some(if operative && let Some(blockers) = event.refused.take() {
                format!("{blockers}; {partial}")
            } else {
                partial
            });
        }
        for field in fields {
            if field.keyword_is("title") {
                event.titles.push(field);
                continue;
            }
            if field.keyword_is("desc") {
                event.descriptions.push(field);
                continue;
            }
            if field.keyword_is("immediate") {
                if let crate::parser::Value::Block(body) = field.value {
                    event.immediate.extend(body);
                }
                continue;
            }
            if field.keyword_is("option") {
                let option = match field.value {
                    crate::parser::Value::Block(body) => body,
                    _ => Vec::new(),
                };
                event.option_names.push(option_name(&option).to_vec());
                event.option_triggers.push(
                    option
                        .iter()
                        .filter(|item| item.keyword_is("trigger"))
                        .flat_map(|item| item.value.items().unwrap_or_default())
                        .cloned()
                        .collect(),
                );
                event.original_recipient_only.push(
                    option
                        .iter()
                        .any(|item| item.keyword_is("original_recipient_only"))
                        .then(|| {
                            crate::parser::read_bool_field(
                                &option,
                                "original_recipient_only",
                                false,
                            )
                        }),
                );
                event.options.push(
                    option
                        .into_iter()
                        .filter(|item| {
                            !["name", "trigger", "ai_chance", "original_recipient_only"]
                                .iter()
                                .any(|key| item.keyword_is(key))
                        })
                        .collect(),
                );
            }
        }
        Ok(event)
    }

    fn selected(&mut self, source: &str) -> Events {
        let mut result = Events {
            namespaces: namespaces(&self.read),
            ..Default::default()
        };
        result.refused.extend(
            self.read
                .misreads
                .iter()
                .map(|note| format!("{source}: {note}")),
        );
        if let Some(why) = &self.read.unsettled {
            result.refused.push(format!("{source}: {why}"));
        }
        for ordinal in 0..self.event_items.len() {
            let mut event = self
                .event(source, ordinal)
                .expect("enumerated event ordinal");
            let id = event.id.clone();
            if id.is_empty() {
                result
                    .refused
                    .push(format!("{}: event identifier absent", event.source));
                continue;
            }
            let key = identifier_key(&id).unwrap_or_else(|why| {
                if id.contains(&b'.') {
                    event.refused = Some(format!("{}: {why}", event.source));
                }
                id.to_vec()
            });
            if result.entries.contains_key(&key) {
                event.refused = Some(format!(
                    "{}: repeated event identifier {}; native registration order is not reconstructed",
                    event.source,
                    String::from_utf8_lossy(&id)
                ));
            }
            result.entries.insert(key, event);
        }
        result
    }
}

pub struct LazyEvents {
    layers: EventLayers,
    cache: Mutex<Cache>,
}

enum EventLayers {
    Layers(Arc<ResolvedLayers>),
    Content(Arc<ResolvedContent>),
}

impl Deref for EventLayers {
    type Target = ResolvedLayers;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Layers(layers) => layers,
            Self::Content(content) => &content.layers,
        }
    }
}

#[derive(Default)]
struct Cache {
    index: Option<Observed<Result<Index, String>>>,
    events: BTreeMap<Vec<u8>, Observed<Result<Arc<Event>, String>>>,
    files: BTreeMap<ScriptPath, Observed<EventRead>>,
    trees: BTreeMap<EventRef, Observed<Result<Arc<Event>, String>>>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct EventRef {
    path: ScriptPath,
    ordinal: usize,
}

#[derive(Default)]
struct Index {
    events: BTreeMap<Vec<u8>, Vec<EventRef>>,
    options: BTreeMap<Vec<u8>, Vec<(EventRef, usize)>>,
    namespaces: BTreeSet<Vec<u8>>,
    refused: Vec<String>,
}

#[derive(Clone, Copy)]
enum Header {
    File,
    Event,
    Option,
}

fn dynamic_headers(bytes: &[u8]) -> bool {
    let tokens = native::lex(bytes, native::Mode::SkipMark).tokens;
    let mut depth = 0_usize;
    for pair in tokens.windows(2) {
        match pair[0].kind {
            native::Kind::Open => depth += 1,
            native::Kind::Close => depth = depth.saturating_sub(1),
            _ => {}
        }
        if depth != 0 && pair[0].starts_with_at() && pair[1].kind == native::Kind::Equals {
            return true;
        }
    }
    false
}

fn read_headers(bytes: &[u8]) -> native::Read {
    native::read(
        bytes,
        native::Mode::SkipMark,
        native::Top::File(Header::File),
        None,
        &mut |node, field| match node {
            Header::File => {
                if matches!(
                    field.keyword().as_deref(),
                    Some(
                        b"country_event"
                            | b"news_event"
                            | b"state_event"
                            | b"unit_leader_event"
                            | b"operative_leader_event"
                    )
                ) {
                    Shape::Object(Header::Event)
                } else if field.keyword_is("add_namespace") {
                    Shape::One
                } else {
                    Shape::Skip
                }
            }
            Header::Event => {
                if field.keyword_is("id") {
                    Shape::One
                } else if field.keyword_is("option") {
                    Shape::Object(Header::Option)
                } else {
                    Shape::Skip
                }
            }
            Header::Option => {
                if field.keyword_is("name") {
                    Shape::One
                } else {
                    Shape::Skip
                }
            }
        },
    )
}

impl Index {
    fn add(&mut self, read: &native::Read, path: &ScriptPath, full: bool) {
        self.namespaces.extend(namespaces(read));
        if !full {
            self.refused.extend(
                read.misreads
                    .iter()
                    .map(|note| format!("{path}: event index: {note}")),
            );
        }
        if let Some(why) = &read.unsettled {
            self.refused.push(format!("{path}: event index: {why}"));
        }
        for (ordinal, item) in read
            .items
            .iter()
            .filter(|item| event_kind(item))
            .enumerate()
        {
            let fields = item.value.items().unwrap_or_default();
            let reference = EventRef {
                path: path.clone(),
                ordinal,
            };
            if let Ok(key) = identifier_key(string_field(fields, "id")) {
                self.events.entry(key).or_default().push(reference.clone());
            }
            if item.keyword_is("state_event") {
                continue;
            }
            for (option, field) in fields
                .iter()
                .filter(|field| field.keyword_is("option"))
                .enumerate()
            {
                let name = option_name(field.value.items().unwrap_or_default());
                self.options
                    .entry(name.to_vec())
                    .or_default()
                    .push((reference.clone(), option));
            }
        }
    }
}

impl LazyEvents {
    pub fn new(layers: Arc<ResolvedLayers>) -> Self {
        Self {
            layers: EventLayers::Layers(layers),
            cache: Mutex::default(),
        }
    }

    pub fn from_content(content: Arc<ResolvedContent>) -> Self {
        Self {
            layers: EventLayers::Content(content),
            cache: Mutex::default(),
        }
    }

    fn index(&self, cache: &mut Cache) -> Result<(), String> {
        self.index_unless(cache, &AtomicBool::new(false))
    }

    fn index_unless(&self, cache: &mut Cache, cancelled: &AtomicBool) -> Result<(), String> {
        if cache.index.is_none() {
            let mut files = BTreeMap::new();
            let built = Observed::new(|| {
                let mut index = Index::default();
                for path in self
                    .layers
                    .directory_files("events", ".txt")
                    .map_err(|error| error.to_string())?
                {
                    if cancelled.load(Ordering::Acquire) {
                        return Ok(None);
                    }
                    let file = Observed::new(|| {
                        let bytes = path
                            .read(&self.layers)
                            .map_err(|error| format!("{path}: {error}"))?;
                        if dynamic_headers(&bytes) {
                            let names =
                                self.layers.key_names().map_err(|error| error.to_string())?;
                            Ok::<_, String>((read_events(&bytes, &names, None), true))
                        } else {
                            Ok((
                                EventRead {
                                    read: read_headers(&bytes),
                                    lines: Vec::new(),
                                    event_items: Vec::new(),
                                },
                                false,
                            ))
                        }
                    });
                    file.get().as_ref().map_err(Clone::clone)?;
                    let file = file.map(Result::unwrap);
                    let (read, full) = file.get();
                    index.add(&read.read, &path, *full);
                    if *full {
                        files.insert(path, file.map(|(read, _)| read));
                    }
                }
                Ok(Some(index))
            });
            if matches!(built.get(), Ok(None)) {
                return Err("event index read cancelled".into());
            }
            let index = built.map(|built| match built {
                Ok(Some(index)) => Ok(index),
                Ok(None) => unreachable!("cancelled index returned above"),
                Err(error) => Err(error),
            });
            cache.files.extend(files);
            cache.index = Some(index);
        }
        let index = cache
            .index
            .as_ref()
            .unwrap()
            .get()
            .as_ref()
            .map_err(Clone::clone)?;
        if index.refused.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "event index is incomplete: {}",
                index.refused.join("; ")
            ))
        }
    }

    fn cache_trees<'a>(
        &self,
        cache: &mut Cache,
        references: impl Iterator<Item = &'a EventRef>,
        apart: bool,
    ) {
        let mut wanted: BTreeMap<&ScriptPath, BTreeSet<usize>> = BTreeMap::new();
        for reference in references {
            if !cache.trees.contains_key(reference) {
                wanted
                    .entry(&reference.path)
                    .or_default()
                    .insert(reference.ordinal);
            }
        }
        for (path, ordinals) in wanted {
            if let Some(file) = cache.files.get_mut(path) {
                for ordinal in ordinals {
                    let event =
                        Observed::new(|| file.get_mut().event(&path.text(), ordinal).map(Arc::new));
                    cache.trees.insert(
                        EventRef {
                            path: path.clone(),
                            ordinal,
                        },
                        event,
                    );
                }
                continue;
            }
            let read = Observed::new(|| {
                let bytes = path
                    .read(&self.layers)
                    .map_err(|error| format!("{path}: {error}"))?;
                let names = self.layers.key_names().map_err(|error| error.to_string())?;
                Ok::<_, String>((bytes, names))
            });
            let walks = if apart {
                ordinals
                    .iter()
                    .map(|&ordinal| BTreeSet::from([ordinal]))
                    .collect()
            } else {
                vec![ordinals]
            };
            for wanted in walks {
                let mut file = read
                    .get()
                    .as_ref()
                    .map(|(bytes, names)| read_events(bytes, names, Some(&wanted)));
                for ordinal in wanted {
                    let event = Observed::new(|| {
                        let _ = read.get();
                        match &mut file {
                            Ok(file) => file.event(&path.text(), ordinal).map(Arc::new),
                            Err(error) => Err(error.to_string()),
                        }
                    });
                    cache.trees.insert(
                        EventRef {
                            path: path.clone(),
                            ordinal,
                        },
                        event,
                    );
                }
            }
        }
    }

    fn event(&self, cache: &mut Cache, reference: &EventRef) -> Result<Arc<Event>, String> {
        self.cache_trees(cache, std::iter::once(reference), false);
        cache
            .trees
            .get(reference)
            .expect("requested event cached")
            .get()
            .clone()
    }

    pub fn get(&self, id: &[u8]) -> Result<Arc<Event>, String> {
        let key = identifier_key(id)?;
        let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(event) = cache.events.get(&key) {
            return event.get().clone();
        }
        let result = Observed::new(|| {
            self.index(&mut cache)?;
            let index = cache.index.as_ref().unwrap().get().as_ref().unwrap();
            let dot = key.iter().rposition(|byte| *byte == b'.').unwrap();
            if !index.namespaces.contains(&key[..dot]) {
                return Err(format!(
                    "event {} namespace is not registered through add_namespace",
                    String::from_utf8_lossy(id)
                ));
            }
            let references = index.events.get(&key).cloned().unwrap_or_default();
            let reference = references.last().ok_or_else(|| {
                format!(
                    "event {} indexed but not resolved by its native reader",
                    String::from_utf8_lossy(id)
                )
            })?;
            let event = self.event(&mut cache, reference)?;
            if identifier_key(&event.id)? != key {
                return Err(format!(
                    "{}: event index differs from its native reader",
                    event.source
                ));
            }
            if references.len() == 1 {
                return Ok(event);
            }
            let mut event = (*event).clone();
            event.refused = Some(format!(
                "{}: repeated event identifier {}; native registration order is not reconstructed",
                event.source,
                String::from_utf8_lossy(id)
            ));
            Ok(Arc::new(event))
        });
        let value = result.get().clone();
        cache.events.insert(key, result);
        value
    }

    pub fn prefetch<'i>(&self, ids: impl IntoIterator<Item = &'i [u8]>, cancelled: &AtomicBool) {
        let keys: Vec<Vec<u8>> = ids
            .into_iter()
            .filter_map(|id| identifier_key(id).ok())
            .collect();
        if keys.is_empty() || cancelled.load(Ordering::Acquire) {
            return;
        }
        let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
        if self.index_unless(&mut cache, cancelled).is_err() {
            return;
        }
        let mut files: BTreeMap<ScriptPath, Vec<EventRef>> = BTreeMap::new();
        {
            let index = cache.index.as_ref().unwrap().get().as_ref().unwrap();
            for key in keys {
                let registered = key
                    .iter()
                    .rposition(|byte| *byte == b'.')
                    .is_some_and(|dot| index.namespaces.contains(&key[..dot]));
                if !registered || cache.events.contains_key(&key) {
                    continue;
                }
                if let Some(reference) = index.events.get(&key).and_then(|found| found.last()) {
                    files
                        .entry(reference.path.clone())
                        .or_default()
                        .push(reference.clone());
                }
            }
        }
        for references in files.values() {
            if cancelled.load(Ordering::Acquire) {
                return;
            }
            self.cache_trees(&mut cache, references.iter(), true);
        }
    }

    pub fn options(&self, name: &[u8]) -> Result<Vec<(Arc<Event>, usize)>, String> {
        let name = crate::parser::c_string(name);
        let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
        self.index(&mut cache)?;
        let references = cache
            .index
            .as_ref()
            .unwrap()
            .get()
            .as_ref()
            .unwrap()
            .options
            .get(name)
            .cloned()
            .unwrap_or_default();
        self.cache_trees(
            &mut cache,
            references.iter().map(|(reference, _)| reference),
            false,
        );
        references
            .iter()
            .map(|(reference, ordinal)| {
                let event = self.event(&mut cache, reference)?;
                if event.option_names.get(*ordinal).map(Vec::as_slice) != Some(name) {
                    return Err(format!(
                        "{}: option-name index differs from its native reader",
                        event.source
                    ));
                }
                Ok((event, *ordinal))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
