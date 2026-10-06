//! CEvent border-outcome tooltip content (1413D69D0), through the recorded stack.
use crate::{
    ResolvedLayers,
    key_names::KeyNames,
    parser::{
        Item,
        native::{self, Shape},
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Event {
    pub id: Vec<u8>,
    pub source: String,
    pub titles: Vec<Item>,
    pub immediate: Vec<Item>,
    pub options: Vec<Vec<Item>>,
    pub refused: Option<String>,
}

impl Event {
    /// 14116C180 returns its sole candidate before evaluating its trigger.
    /// Several candidates need the unrecorded high-resolution seed.
    pub fn sole_title_key(&self) -> Result<Option<&[u8]>, String> {
        match self.titles.as_slice() {
            [] => Ok(None),
            [item] => {
                let key = item
                    .value
                    .scalar()
                    .or_else(|| {
                        item.value
                            .items()?
                            .iter()
                            .rev()
                            .find(|item| item.keyword_is("text"))?
                            .value
                            .scalar()
                    })
                    .unwrap_or_default();
                let key = crate::parser::c_string(key);
                Ok((!key.is_empty()).then_some(key))
            }
            _ => Err(format!(
                "{} event {} has multiple title candidates; 14116C180 selects with an unrecorded high-resolution seed",
                self.source,
                String::from_utf8_lossy(&self.id)
            )),
        }
    }
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
    Event,
    Option,
    Title,
    Mtth,
}

/// 1401F03A0 compares namespace names without case; 1424B8700 reduces
/// signed bytes with wrapping arithmetic, without digit validation.
pub fn identifier_key(id: &[u8]) -> Result<Vec<u8>, String> {
    let id = crate::parser::c_string(id);
    let dot = id.iter().rposition(|byte| *byte == b'.').ok_or_else(|| {
        format!(
            "border event {} has no namespace; 1401F03A0 cannot resolve it",
            String::from_utf8_lossy(id)
        )
    })?;
    if dot == 0 {
        return Err("border event namespace is empty (1401F03A0)".into());
    }
    let suffix = id[dot + 1..].iter().fold(0_i32, |value, byte| {
        value
            .wrapping_mul(10)
            .wrapping_add(i32::from(*byte as i8))
            .wrapping_sub(48)
    });
    if !(0..100_000).contains(&suffix) {
        return Err(format!(
            "border event {} suffix {suffix} crosses a native namespace slot; registration order is unavailable (1401F03A0)",
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
                "border event {} namespace is not registered through add_namespace (1401F03A0)",
                String::from_utf8_lossy(id)
            ));
        }
        self.entries.get(&key).ok_or_else(|| {
            format!(
                "border event {} is absent from the resolved event database; {}",
                String::from_utf8_lossy(id),
                self.refused.join("; ")
            )
        })
    }
}

/// CEvent::ReadToken@14116BDB0 retains option order and delegates immediate
/// to CEffect::ReadScript. CEventOption::vf4@1415279D0 delegates other keys to
/// the effect reader; metadata is not an effect. Unknown descriptions remain
/// named at the consumer, while native effect parsing still consumes exactly.
pub fn parse(bytes: &[u8], source: &str, names: &KeyNames) -> Events {
    parse_selected(bytes, source, names, None)
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

fn parse_selected(bytes: &[u8], source: &str, names: &KeyNames, wanted: Option<&[u8]>) -> Events {
    let mut lines = Vec::new();
    let read = native::read(
        bytes,
        native::Mode::SkipMark, // Event file lexer mode 1, 140A03E00.
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
                    lines.push(field.key.line);
                    Shape::Object(Node::Event)
                }
                Some(b"add_namespace") => Shape::One,
                _ => Shape::Skip,
            },
            Node::Event => match field.keyword().as_deref() {
                Some(b"id") => Shape::One,
                Some(b"title" | b"desc") if field.opens() => Shape::Object(Node::Title),
                Some(b"title" | b"desc") => Shape::One,
                Some(b"immediate") => Shape::Effect,
                Some(b"option") => Shape::Object(Node::Option),
                Some(b"trigger" | b"show_major") => Shape::Trigger,
                Some(b"mean_time_to_happen") => Shape::Object(Node::Mtth),
                Some(b"after") => Shape::Effect,
                Some(
                    b"picture" | b"is_triggered_only" | b"major" | b"hidden" | b"fire_only_once"
                    | b"timeout_days" | b"exclusive" | b"fire_for_sender" | b"minor_flavor",
                ) => Shape::One,
                _ => Shape::Skip,
            },
            Node::Title => match field.keyword().as_deref() {
                Some(b"text") => Shape::One,
                Some(b"trigger") => Shape::Trigger,
                _ => Shape::Skip,
            },
            Node::Option => match field.keyword().as_deref() {
                Some(b"name" | b"original_recipient_only") => Shape::One,
                Some(b"trigger") => Shape::Trigger,
                Some(b"ai_chance") => Shape::Object(Node::Mtth),
                _ => crate::effect_keys::shape(Some(names), field).map(|_| Node::Option),
            },
            Node::Mtth => native::mean_time_to_happen(field),
        },
    );
    let mut result = Events::default();
    result
        .refused
        .extend(read.misreads.iter().map(|note| format!("{source}: {note}")));
    if let Some(why) = &read.unsettled {
        result.refused.push(format!("{source}: {why}"));
    }
    for item in &read.items {
        if item.keyword_is("add_namespace")
            && let Some(name) = item.value.scalar()
        {
            result.namespaces.insert(
                crate::parser::c_string(name)
                    .iter()
                    .map(u8::to_ascii_lowercase)
                    .collect(),
            );
        }
    }
    for (index, (item_index, item)) in read
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| event_kind(item))
        .enumerate()
    {
        let fields = item.value.items().unwrap_or_default();
        let id = fields
            .iter()
            .rfind(|item| item.keyword_is("id"))
            .and_then(|item| item.value.scalar())
            .map(crate::parser::c_string)
            .unwrap_or_default()
            .to_vec();
        if wanted.is_some_and(|wanted| identifier_key(&id).map_or(true, |key| key != wanted)) {
            continue;
        }
        let mut event = Event {
            id: id.clone(),
            source: format!("{source}:{}", lines.get(index).copied().unwrap_or(1)),
            ..Default::default()
        };
        if id.is_empty() {
            result
                .refused
                .push(format!("{}: event identifier absent", event.source));
            continue;
        }
        let blockers: Vec<_> = read.item_misreads[item_index]
            .iter()
            .chain(&read.unattributed_misreads)
            .filter(|note| {
                matches!(
                    note.kind,
                    crate::DiagnosticKind::ParseFailure
                        | crate::DiagnosticKind::Unsettled
                        | crate::DiagnosticKind::Unsupported
                        | crate::DiagnosticKind::Refused
                )
            })
            .map(ToString::to_string)
            .collect();
        if !blockers.is_empty() {
            event.refused = Some(format!("{}: {}", event.source, blockers.join("; ")));
        }
        if item.keyword_is("operative_leader_event") {
            event.refused = Some(format!(
                "{}: operative_leader_event reader is not ported (event loader 140A03E00)",
                event.source
            ));
            result.refused.push(event.refused.clone().unwrap());
        }
        if index + 1 == lines.len()
            && let Some(why) = &read.unsettled
        {
            event.refused = Some(format!(
                "{}: partial native event read: {why}",
                event.source
            ));
        }
        for field in fields {
            if field.keyword_is("title") {
                event.titles.push(field.clone());
            }
            if field.keyword_is("immediate") {
                event
                    .immediate
                    .extend_from_slice(field.value.items().unwrap_or_default());
            }
            if field.keyword_is("option") {
                event.options.push(
                    field
                        .value
                        .items()
                        .unwrap_or_default()
                        .iter()
                        .filter(|item| {
                            !["name", "trigger", "ai_chance", "original_recipient_only"]
                                .iter()
                                .any(|key| item.keyword_is(key))
                        })
                        .cloned()
                        .collect(),
                );
            }
        }
        let key = identifier_key(&id).unwrap_or_else(|why| {
            // Bare IDs are legal definitions but not resolved by this tail.
            if id.contains(&b'.') {
                event.refused = Some(format!("{}: {why}", event.source));
            }
            id.clone()
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

/// Only requested outcome events acquire title/effect trees. The first lookup
/// indexes ID/namespace headers; subsequent aliases share the cached result.
pub struct LazyEvents {
    layers: Arc<ResolvedLayers>,
    cache: Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
    index: Option<Result<Index, String>>,
    events: BTreeMap<Vec<u8>, Result<Arc<Event>, String>>,
}

#[derive(Default)]
struct Index {
    files: BTreeMap<Vec<u8>, Vec<String>>,
    dynamic: Vec<String>,
    namespaces: BTreeSet<Vec<u8>>,
    refused: Vec<String>,
}

impl Index {
    fn add(&mut self, bytes: &[u8], path: &str) {
        // A variable defined inside an event can affect a later ID or
        // namespace. Such files need the full reader for every demand.
        let tokens = native::lex(bytes, native::Mode::SkipMark).tokens;
        let mut depth = 0_usize;
        for pair in tokens.windows(2) {
            match pair[0].kind {
                native::Kind::Open => depth += 1,
                native::Kind::Close => depth = depth.saturating_sub(1),
                _ => {}
            }
            if depth != 0 && pair[0].starts_with_at() && pair[1].kind == native::Kind::Equals {
                self.dynamic.push(path.to_owned());
                // Header-only reads cannot resolve this file's later
                // references. Let the native reader supply its headers too.
                return;
            }
        }
        // An inventory, not an event reader: untouched bodies are skipped.
        let read = native::read(
            bytes,
            native::Mode::SkipMark,
            native::Top::File(false),
            None,
            &mut |event, field| {
                if event {
                    if field.keyword_is("id") {
                        Shape::One
                    } else {
                        Shape::Skip
                    }
                } else if matches!(
                    field.keyword().as_deref(),
                    Some(
                        b"country_event"
                            | b"news_event"
                            | b"state_event"
                            | b"unit_leader_event"
                            | b"operative_leader_event"
                    )
                ) {
                    Shape::Object(true)
                } else if field.keyword_is("add_namespace") {
                    Shape::One
                } else {
                    Shape::Skip
                }
            },
        );
        self.refused.extend(
            read.misreads
                .iter()
                .map(|note| format!("{path}: event index: {note}")),
        );
        if let Some(why) = &read.unsettled {
            self.refused.push(format!("{path}: event index: {why}"));
        }
        for item in read.items {
            if item.keyword_is("add_namespace") {
                if let Some(name) = item.value.scalar() {
                    self.namespaces.insert(
                        crate::parser::c_string(name)
                            .iter()
                            .map(u8::to_ascii_lowercase)
                            .collect(),
                    );
                }
            } else if event_kind(&item)
                && let Some(id) = item
                    .value
                    .items()
                    .unwrap_or_default()
                    .iter()
                    .rfind(|field| field.keyword_is("id"))
                    .and_then(|field| field.value.scalar())
                && let Ok(key) = identifier_key(id)
            {
                let paths = self.files.entry(key).or_default();
                if paths.last().is_none_or(|last| last != path) {
                    paths.push(path.to_owned());
                }
            }
        }
    }
}

impl LazyEvents {
    pub fn new(layers: Arc<ResolvedLayers>) -> Self {
        Self {
            layers,
            cache: Mutex::default(),
        }
    }

    pub fn get(&self, id: &[u8]) -> Result<Arc<Event>, String> {
        let key = identifier_key(id)?;
        let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(event) = cache.events.get(&key) {
            return event.clone();
        }
        if cache.index.is_none() {
            cache.index = Some((|| {
                let mut index = Index::default();
                for path in self
                    .layers
                    .directory_files("events", ".txt")
                    .map_err(|error| error.to_string())?
                {
                    index.add(
                        &self
                            .layers
                            .read(&path)
                            .map_err(|error| format!("{path}: {error}"))?,
                        &path,
                    );
                }
                Ok(index)
            })());
        }
        let result = (|| {
            let index = cache
                .index
                .as_ref()
                .unwrap()
                .as_ref()
                .map_err(Clone::clone)?;
            if !index.refused.is_empty() {
                return Err(format!(
                    "border event index is incomplete: {}",
                    index.refused.join("; ")
                ));
            }
            let paths: BTreeSet<_> = index
                .files
                .get(&key)
                .into_iter()
                .flatten()
                .chain(&index.dynamic)
                .collect();
            let names = self.layers.key_names().map_err(|error| error.to_string())?;
            let mut found = None;
            let dot = key.iter().rposition(|byte| *byte == b'.').unwrap();
            let mut registered = index.namespaces.contains(&key[..dot]);
            for path in paths {
                let bytes = self
                    .layers
                    .read(path)
                    .map_err(|error| format!("{path}: {error}"))?;
                let mut file = parse_selected(&bytes, path, &names, Some(&key));
                registered |= file.namespaces.contains(&key[..dot]);
                if let Some(mut event) = file.entries.remove(&key) {
                    if found.is_some() {
                        event.refused = Some(format!(
                            "{}: repeated event identifier {}; native registration order is not reconstructed",
                            event.source,
                            String::from_utf8_lossy(id)
                        ));
                    }
                    found = Some(event);
                } else if !file.refused.is_empty() {
                    return Err(file.refused.join("; "));
                }
            }
            if !registered {
                return Err(format!(
                    "border event {} namespace is not registered through add_namespace (1401F03A0)",
                    String::from_utf8_lossy(id)
                ));
            }
            found.map(Arc::new).ok_or_else(|| {
                format!(
                    "border event {} indexed but not resolved by its native reader",
                    String::from_utf8_lossy(id)
                )
            })
        })();
        cache.events.insert(key, result.clone());
        result
    }
}

#[cfg(test)]
mod tests;
