use content_reader_key_names::key_name_products::KeyNamesExt as _;

use std::{collections::BTreeMap, io, sync::Arc};

use content_reader_script::native::{self, Shape};
use crate::{
    ResolvedLayers,
    crt::{self, Scanned},
    defines::Defines,
    fields::{block, invalid},
    fixed5::decimal_i32,
    key_names::KeyNames,
    map::ProvinceDefinition,
    parser::{
        Item, Value,
    },
    scope::ScopeKind,
    script_files,
    tokens::{self, Token},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MisreadCause {
    InvalidImpassable,
    OffMapProvince,
    NonStrategicResource,
    InvalidVictoryPoints,
    NonBlockBuildings,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub kind: crate::DiagnosticKind,
    pub message: String,
    pub cause: Option<MisreadCause>,
}

impl Diagnostic {
    pub fn into_diagnostic(self) -> crate::Diagnostic {
        (self.kind, self.message)
    }
}

impl From<crate::Diagnostic> for Diagnostic {
    fn from((kind, message): crate::Diagnostic) -> Self {
        Self {
            kind,
            message,
            cause: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildingLevel {
    pub level: i32,
    pub conditional: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StateDefinition {
    pub id: u32,
    pub name_key: Result<String, String>,
    pub category: Option<String>,
    pub impassable: bool,
    pub manpower: Option<i32>,
    pub provinces: Vec<u32>,
    pub resources: Result<Vec<(String, i32)>, String>,
    pub victory_points: Vec<(i32, i32)>,
    pub buildings: BTreeMap<String, BuildingLevel>,
    pub province_buildings: BTreeMap<u32, BTreeMap<String, BuildingLevel>>,
    pub skipped_history: Vec<String>,
}

impl Default for StateDefinition {
    fn default() -> Self {
        Self {
            id: u32::MAX,
            name_key: Ok(String::new()),
            category: None,
            impassable: false,
            manpower: None,
            provinces: Vec::new(),
            resources: Ok(Vec::new()),
            victory_points: Vec::new(),
            buildings: BTreeMap::new(),
            province_buildings: BTreeMap::new(),
            skipped_history: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resource {
    Found,
    Null,
    Unknown(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resources {
    tokens: Vec<Token>,
    gap: Option<String>,
}

impl Resources {
    pub fn find(&self, name: &[u8]) -> Resource {
        let shown = String::from_utf8_lossy(crate::parser::c_string(name));
        match Token::existing(name, &self.tokens) {
            Some(token)
                if tokens::first(self.tokens.iter().map(|known| (known, ())), &token).is_some() =>
            {
                Resource::Found
            }
            _ if self.gap.is_some() => Resource::Unknown(format!(
                "`{shown}` names no strategic resource read, and {}",
                self.gap.as_deref().unwrap_or_default()
            )),
            Some(_) => Resource::Null,
            None if self.tokens.contains(&Token::Rejected) => Resource::Unknown(format!(
                "`{shown}` is no strategic resource's name, and a resource holds token 0 (a name empty or starting with a digit or `-`): token lookup finds that resource unless the lexer holds a token for `{shown}`, which depends on every name the game registered"
            )),
            None => Resource::Null,
        }
    }
}

pub struct Context<'a> {
    pub resource: &'a dyn Fn(&[u8]) -> Resource,
    pub max_building_levels: i32,
    pub province_count: u32,
    pub names: &'a KeyNames,
}

pub fn province_count(definitions: &BTreeMap<u32, ProvinceDefinition>) -> u32 {
    definitions.len() as u32
}

fn scanned(value: &Value) -> Option<i32> {
    match crt::scan_i32(value.scalar().unwrap_or_default()) {
        Scanned::Stored(value) => Some(value),
        Scanned::End | Scanned::NoMatch => None,
    }
}

fn building_level(value: &Value, context: &Context<'_>) -> BuildingLevel {
    let mut level = 0;
    let mut conditional = false;
    match value.items() {
        Some(items) => {
            for item in items {
                if item.keyword_is("level") {
                    level = scanned(&item.value).unwrap_or(level);
                } else if item.keyword_is("allowed") {
                    conditional = true;
                }
            }
        }
        None => level = scanned(value).unwrap_or(level),
    }
    BuildingLevel {
        level: level.min(context.max_building_levels),
        conditional,
    }
}

fn read_buildings(items: &[Item], state: &mut StateDefinition, context: &Context<'_>) {
    for item in items {
        let key = item.key.as_deref().unwrap_or_default();
        let province = decimal_i32(key);
        if province > 0 {
            let entry = state.province_buildings.entry(province as u32).or_default();
            for inner in item.value.items().unwrap_or_default() {
                let name = inner.key.as_deref().unwrap_or_default();
                entry.insert(
                    String::from_utf8_lossy(name).to_ascii_lowercase(),
                    building_level(&inner.value, context),
                );
            }
        } else {
            state.buildings.insert(
                String::from_utf8_lossy(key).to_ascii_lowercase(),
                building_level(&item.value, context),
            );
        }
    }
}

pub fn parse_states(
    input: &[u8],
    context: &Context<'_>,
) -> io::Result<(Vec<StateDefinition>, Vec<crate::Diagnostic>)> {
    let parsed = parse_states_attributed(input, context)?;
    Ok((
        parsed.states.into_iter().map(|(state, _)| state).collect(),
        parsed
            .diagnostics
            .into_iter()
            .map(Diagnostic::into_diagnostic)
            .collect(),
    ))
}

#[derive(Debug)]
struct ParsedStates {
    states: Vec<(StateDefinition, Vec<String>)>,
    diagnostics: Vec<Diagnostic>,
}

fn parse_states_attributed(input: &[u8], context: &Context<'_>) -> io::Result<ParsedStates> {
    #[derive(Clone, Copy)]
    enum Node {
        Top,
        State,
        Resources,
        History,
        Entry,
        Buildings,
        Province,
        Building,
    }
    fn command(field: &native::Field<'_>, names: &KeyNames) -> Shape<Node> {
        match field.keyword().as_deref() {
            Some(b"owner" | b"controller") => Shape::One,
            Some(b"victory_points") => Shape::List,
            Some(b"buildings") if field.opens() => Shape::Object(Node::Buildings),
            Some(b"buildings") => Shape::Skip,
            _ => crate::effect_keys::shape(Some(names), field).map(|()| Node::History),
        }
    }
    fn building() -> Shape<Node> {
        Shape::ObjectIfOpen(Node::Building)
    }
    let mut dispatched = 0_usize;
    let mut lost: Option<(usize, String)> = None;
    let read = native::read_in_effect_scope(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::Top),
        Some(context.names),
        ScopeKind::State.bit(),
        &mut |node, field| match node {
            Node::Top if field.keyword_is("state") => {
                dispatched += 1;
                Shape::Object(Node::State)
            }
            Node::Top => Shape::Skip,
            Node::State => match field.keyword().as_deref() {
                Some(
                    b"id"
                    | b"name"
                    | b"state_category"
                    | b"manpower"
                    | b"impassable"
                    | b"local_supplies"
                    | b"buildings_max_level_factor"
                    | b"force_link_ownership_to",
                ) => Shape::One,
                Some(b"provinces") => Shape::Tokens,
                Some(b"impassable_ignored_links") => Shape::List,
                Some(b"color") => Shape::SkipIfOpen,
                Some(b"resources") => Shape::Object(Node::Resources),
                Some(b"history") => Shape::Object(Node::History),
                Some(b"state_startup_effect") => Shape::EffectIn(ScopeKind::State.bit()),
                _ => Shape::Skip,
            },
            Node::Resources => match (context.resource)(&field.key.text) {
                Resource::Found => Shape::One,
                Resource::Null => Shape::Skip,
                Resource::Unknown(why) => {
                    if field.skips_block() && lost.is_none() {
                        lost = Some((
                            dispatched - 1,
                            format!(
                                "line {}: {why}; the block after it is its amount when found and skipped otherwise",
                                field.key.line
                            ),
                        ));
                    }
                    Shape::Skip
                }
            },
            Node::History if decimal_i32(&field.key.text) != 0 => Shape::Object(Node::Entry),
            Node::History | Node::Entry => command(field, context.names),
            Node::Buildings if decimal_i32(&field.key.text) > 0 => Shape::Object(Node::Province),
            Node::Buildings | Node::Province => building(),
            Node::Building if field.keyword_is("allowed") => Shape::Trigger,
            Node::Building if field.keyword_is("level") => Shape::One,
            Node::Building => Shape::Skip,
        },
    );
    let mut misreads: Vec<_> = read
        .misreads
        .iter()
        .map(|note| Diagnostic::from((note.kind, note.to_string())))
        .collect();
    let all: Vec<&Item> = read
        .items
        .iter()
        .filter(|item| item.keyword_is("state"))
        .collect();
    let unsettled = read.unsettled.as_ref().map(|unsettled| {
        let being_read = read
            .items
            .last()
            .is_some_and(|item| item.keyword_is("state"));
        (all.len() - usize::from(being_read), unsettled.to_string())
    });
    let gap = match (unsettled, lost) {
        (Some(a), Some(b)) => Some(if b.0 < a.0 { b } else { a }),
        (a, b) => a.or(b),
    };
    if let Some((cut, why)) = gap {
        let being_read = match all.get(cut) {
            Some(item) => match item
                .value
                .items()
                .unwrap_or_default()
                .iter()
                .rfind(|field| field.keyword_is("id"))
            {
                Some(id) => format!(
                    "the state being read there (id {} so far)",
                    scanned(&id.value).map_or(u32::MAX, |id| id as u32)
                ),
                None => "the state being read there (no id read yet)".to_owned(),
            },
            None => "the reading there".to_owned(),
        };
        return Err(invalid(format!(
            "{why}; {being_read} and every state after it in the file are unknown; the state inventory is unavailable because their ids and candidate provinces cannot be determined"
        )));
    }
    let file_notes: Vec<_> = read
        .unattributed_misreads
        .iter()
        .chain(
            read.items
                .iter()
                .zip(&read.item_misreads)
                .filter(|(item, _)| !item.keyword_is("state"))
                .flat_map(|(_, notes)| notes),
        )
        .map(ToString::to_string)
        .collect();
    let mut states = Vec::new();
    let mut deferred_notes = Vec::new();
    for (item, native_notes) in read
        .items
        .iter()
        .zip(&read.item_misreads)
        .filter(|(item, _)| item.keyword_is("state"))
    {
        let (state, notes) = parse_state_block(block(&item.value, "state")?, context)?;
        let state_notes = file_notes
            .iter()
            .cloned()
            .chain(deferred_notes.iter().cloned())
            .chain(native_notes.iter().map(ToString::to_string))
            .chain(notes.iter().map(|(_, message)| message.clone()))
            .collect();
        deferred_notes.extend(
            native_notes
                .iter()
                .filter(|note| note.what.starts_with("Deferred effect"))
                .map(|note| format!("after state {}'s unported reader: {note}", state.id)),
        );
        misreads.extend(notes.into_iter().map(|(cause, message)| Diagnostic {
            kind: crate::DiagnosticKind::Misread,
            message,
            cause: Some(cause),
        }));
        if (state.id as i32) > 0 {
            states.push((state, state_notes));
        } else {
            misreads.push(Diagnostic::from((
                crate::DiagnosticKind::Warning,
                format!(
                    "state id {} is not positive; the game discards the template",
                    state.id as i32
                ),
            )));
        }
    }
    Ok(ParsedStates {
        states,
        diagnostics: misreads,
    })
}

fn parse_state_block(
    items: &[Item],
    context: &Context<'_>,
) -> io::Result<(StateDefinition, Vec<(MisreadCause, String)>)> {
    let mut notes = Vec::new();
    let mut state = StateDefinition::default();
    let mut off_map = Vec::new();
    for item in items {
        if item.keyword_is("id") {
            state.id = scanned(&item.value).map_or(u32::MAX, |id| id as u32);
        } else if item.keyword_is("name") {
            state.name_key = item
                .value
                .read_str()
                .map(str::to_owned)
                .map_err(|why| format!("state name: {why}"));
        } else if item.keyword_is("impassable") {
            match item.value.scalar().and_then(crate::parser::read_bool) {
                Some(value) => state.impassable = value,
                _ => notes.push((
                    MisreadCause::InvalidImpassable,
                    format!(
                        "impassable `{}` is not yes or no; the game keeps the previous value",
                        item.value.scalar().unwrap_or_default().escape_ascii()
                    ),
                )),
            }
        } else if item.keyword_is("provinces") {
            for entry in item.value.items().unwrap_or_default() {
                if entry.number() {
                    let province = decimal_i32(entry.value.scalar().unwrap_or_default());
                    match u32::try_from(province) {
                        Ok(province) if province >= 1 && province < context.province_count => {
                            state.provinces.push(province);
                        }
                        _ => off_map.push(province),
                    }
                }
            }
        }
    }
    for province in off_map {
        notes.push((
            MisreadCause::OffMapProvince,
            format!(
                "state {}: province {province} is not on the map ({} provinces); the game reports it and keeps it out of the state",
                state.id, context.province_count
            ),
        ));
    }
    if (state.id as i32) > 0
        && let Some(category) = items.iter().rfind(|item| item.keyword_is("state_category"))
    {
        state.category = Some(
            category
                .value
                .c_str()
                .map_err(|why| invalid(format!("state_category: {why}")))?
                .to_owned(),
        );
    }
    for item in items.iter().filter(|item| item.keyword_is("manpower")) {
        state.manpower = scanned(&item.value).or(state.manpower);
    }
    for resources in items.iter().filter(|item| item.keyword_is("resources")) {
        for item in resources.value.items().unwrap_or_default() {
            let name = item.key.as_deref().unwrap_or_default();
            match (context.resource)(name) {
                Resource::Found => {
                    if let Ok(found) = &mut state.resources {
                        found.push((
                            String::from_utf8_lossy(name).into_owned(),
                            decimal_i32(item.value.scalar().unwrap_or_default()),
                        ));
                    }
                }
                Resource::Null => notes.push((
                    MisreadCause::NonStrategicResource,
                    format!(
                        "state {}: `{}` is no strategic resource; the game reports it and keeps no amount",
                        state.id,
                        String::from_utf8_lossy(name)
                    ),
                )),
                Resource::Unknown(why) => {
                    if state.resources.is_ok() {
                        state.resources = Err(format!("state {}: {why}", state.id));
                    }
                }
            }
        }
    }
    for history in items.iter().filter(|item| item.keyword_is("history")) {
        for item in history.value.items().unwrap_or_default() {
            let key = item.key.as_deref().unwrap_or_default();
            if decimal_i32(key) != 0 || item.keyword_is("if") {
                state
                    .skipped_history
                    .push(String::from_utf8_lossy(key).into_owned());
            } else if item.keyword_is("victory_points") {
                let values = item.value.items().unwrap_or_default();
                if let [province, value] = values {
                    state.victory_points.push((
                        scanned(&province.value).unwrap_or(0),
                        scanned(&value.value).unwrap_or(0),
                    ));
                } else {
                    notes.push((
                        MisreadCause::InvalidVictoryPoints,
                        format!(
                            "state {}: victory_points holds {} values, not a province and a value; the game records an error and keeps none",
                            state.id,
                            values.len()
                        ),
                    ));
                }
            } else if item.keyword_is("buildings") {
                match item.value.items() {
                    Some(entries) => read_buildings(entries, &mut state, context),
                    None => notes.push((
                        MisreadCause::NonBlockBuildings,
                        format!(
                            "state {}: buildings is not a block; the game reports it and reads no buildings",
                            state.id
                        ),
                    )),
                }
            }
        }
    }
    Ok((state, notes))
}

const DIRECTORY: &str = "history/states/";

fn resources(layers: &ResolvedLayers) -> io::Result<Resources> {
    let mut resources = Resources::default();
    for path in script_files::paths(layers, "common/resources/", Some(".txt"))? {
        read_resources(&path.read(layers)?, &path.text(), &mut resources);
    }
    Ok(resources)
}

fn read_resources(bytes: &[u8], path: &str, resources: &mut Resources) {
    #[derive(Clone, Copy)]
    enum Node {
        Top,
        Table,
        Resource,
        Modifier(crate::modifiers::Node),
    }
    let mut undecided = Vec::new();
    let read = native::read(
        bytes,
        native::Mode::KeepMark,
        native::Top::File(Node::Top),
        None,
        &mut |node, field| match node {
            Node::Top if field.keyword_is("resources") => Shape::Object(Node::Table),
            Node::Top => Shape::Skip,
            Node::Table => {
                resources.tokens.push(Token::c_str(&field.key.text));
                Shape::Object(Node::Resource)
            }
            Node::Resource => match field.keyword().as_deref() {
                Some(b"icon_frame" | b"cic" | b"convoys") => Shape::One,
                Some(b"modifier") => crate::modifiers::shape(
                    crate::modifiers::Node::Modifier,
                    field,
                    crate::modifiers::UNKNOWN,
                    &mut undecided,
                )
                .map(Node::Modifier),
                _ => Shape::Skip,
            },
            Node::Modifier(modifier) => {
                crate::modifiers::shape(modifier, field, crate::modifiers::UNKNOWN, &mut undecided)
                    .map(Node::Modifier)
            }
        },
    );
    let lost = undecided
        .first()
        .map(ToString::to_string)
        .or(read.unsettled.map(|unsettled| unsettled.to_string()));
    if let Some(lost) = lost
        && resources.gap.is_none()
    {
        resources.gap = Some(format!(
            "{path} {lost}; the strategic resources after it are unknown"
        ));
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct States {
    definitions: Arc<BTreeMap<u32, StateDefinition>>,
    sources: Arc<BTreeMap<u32, script_files::ScriptPath>>,
    diagnostics: Arc<BTreeMap<u32, Vec<String>>>,
}

impl States {
    pub fn get(&self, id: u32) -> Option<&StateDefinition> {
        self.definitions.get(&id)
    }

    pub fn definitions(&self) -> &BTreeMap<u32, StateDefinition> {
        &self.definitions
    }

    pub fn source(&self, id: u32) -> Option<&script_files::ScriptPath> {
        self.sources.get(&id)
    }

    pub fn diagnostics(&self, id: u32) -> &[String] {
        self.diagnostics.get(&id).map_or(&[], Vec::as_slice)
    }
}

pub fn read(
    layers: &ResolvedLayers,
    defines: &Defines,
    provinces: &BTreeMap<u32, ProvinceDefinition>,
) -> io::Result<(States, Vec<crate::Diagnostic>)> {
    read_with_diagnostics(layers, defines, provinces).map(|(states, notes)| {
        (
            states,
            notes.into_iter().map(Diagnostic::into_diagnostic).collect(),
        )
    })
}

pub fn read_with_diagnostics(
    layers: &ResolvedLayers,
    defines: &Defines,
    provinces: &BTreeMap<u32, ProvinceDefinition>,
) -> io::Result<(States, Vec<Diagnostic>)> {
    let maximum = defines.int32("NBuildings", "MAX_BUILDING_LEVELS")?;
    let counted = province_count(provinces);
    let parameters = [maximum.to_le_bytes(), counted.to_le_bytes()].concat();
    layers.recorded_product("state templates", 1, &parameters, |layers| {
        read_templates(layers, maximum, counted)
    }).map(|value| value.as_ref().clone())
}

fn read_templates(layers: &ResolvedLayers, max_building_levels: i32, counted: u32) -> io::Result<(States, Vec<Diagnostic>)> {
    let resources = resources(layers)?;
    let names = layers.key_names()?;
    let paths = script_files::paths(layers, DIRECTORY, Some(".txt"))?;
    let parsed = content_reader_layers::parallel::map(&paths, |listed| {
        let context = Context {
            resource: &|name| resources.find(name),
            max_building_levels,
            province_count: counted,
            names: &names,
        };
        listed
            .read(layers)
            .map(|input| parse_states_attributed(&input, &context))
    });
    let mut states = States::default();
    let mut conflicts = Vec::new();
    for (listed, file) in paths.into_iter().zip(parsed) {
        let path = listed.to_string();
        let file = file?.map_err(|error| invalid(format!("{path}: {error}")))?;
        let notes: Vec<_> = file
            .diagnostics
            .into_iter()
            .map(|mut note| {
                note.message = format!(
                    "{path} {}; the state follows the game's reading",
                    note.message
                );
                note
            })
            .collect();
        conflicts.extend(notes);
        for (state, diagnostics) in file.states {
            let id = state.id;
            if let Some(kept) = states.sources.get(&id) {
                conflicts.push(Diagnostic::from((
                    crate::DiagnosticKind::Warning,
                    format!(
                        "state {id} is defined by both {kept} and {path}; the game keeps the first"
                    ),
                )));
                continue;
            }
            Arc::make_mut(&mut states.sources).insert(id, listed.clone());
            Arc::make_mut(&mut states.diagnostics).insert(id, diagnostics);
            Arc::make_mut(&mut states.definitions).insert(id, state);
        }
    }
    Ok((states, conflicts))
}

#[cfg(test)]
mod tests;
