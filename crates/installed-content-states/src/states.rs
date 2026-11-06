//! State definitions from `history/states/*.txt`.
//!
//! Only the authored start state is read. Dated blocks such as `1939.1.1 = {}`
//! and conditional `if` blocks are campaign history the replay already carries,
//! so they are listed by key and never applied.

use std::{collections::BTreeMap, io};

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
        native::{self, Shape},
    },
    scope::ScopeKind,
    script_files,
    tokens::{self, Token},
};

/// State-field recovery, kept apart from the shared reader's diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MisreadCause {
    /// Invalid yes/no conversion preserves the previous impassable value.
    InvalidImpassable,
    /// Reports a null province and appends only valid map provinces,
    /// checked against the definition-record count.
    OffMapProvince,
    /// Reports an unresolved resource and appends no amount.
    NonStrategicResource,
    /// A malformed province/value pair keeps zero/zero; applying a
    /// nonpositive province changes no victory points.
    InvalidVictoryPoints,
    /// Reports a non-block and reads no building entries to apply.
    NonBlockBuildings,
}

/// A state diagnostic retains its offline kind and wording. Only state-field
/// recovery has a cause; shared reader notes remain unclassified.
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

/// One building entry, either `name = level` or
/// `name = { level = N allowed = { ... } }`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildingLevel {
    /// Signed integer level, initially 0 and capped at
    /// `NBuildings.MAX_BUILDING_LEVELS`.
    pub level: i32,
    /// Whether the entry holds an `allowed` trigger, retained unevaluated.
    pub conditional: bool,
}

/// One state as `history/states` authors it at the campaign start.
#[derive(Clone, Debug, PartialEq)]
pub struct StateDefinition {
    pub id: u32,
    /// Last `name` read through file variables, normally a localisation
    /// key such as `STATE_<id>`, or why its consumption could not be followed.
    pub name_key: Result<String, String>,
    pub category: Option<String>,
    pub impassable: bool,
    /// Last successfully scanned signed `manpower` value.
    pub manpower: Option<i32>,
    /// Number tokens from every provinces block, converted in order with repeats.
    pub provinces: Vec<u32>,
    /// Resource entries in file order, each amount converted to a signed
    /// integer. `Err` names a lookup that depends on lexer registrations
    /// ([`Resource::Unknown`]).
    pub resources: Result<Vec<(String, i32)>, String>,
    /// `(province, value)` pairs in file order, each read by `%i`; a list
    /// holding other than two values keeps none.
    pub victory_points: Vec<(i32, i32)>,
    /// State-wide history building levels by case-folded name; last wins.
    pub buildings: BTreeMap<String, BuildingLevel>,
    /// Province-scoped levels from the positive numeric keys of the same
    /// block.
    pub province_buildings: BTreeMap<u32, BTreeMap<String, BuildingLevel>>,
    /// Keys of the dated and conditional history blocks this reader skipped.
    pub skipped_history: Vec<String>,
}

impl Default for StateDefinition {
    /// Initial id -1, empty name and impassable false.
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

/// Resource lookup by the lexer's existing token for a name, or token 0
/// when none exists: the first registered resource with that token wins,
/// otherwise the null resource is returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resource {
    Found,
    /// The null resource: report the key and skip its block.
    Null,
    /// A name no resource registered, while a resource holds token 0 (its
    /// name empty or starting with a digit or `-`): the name finds that
    /// resource when the lexer holds no token for it, which depends on
    /// every name any reader registered. Why, by name.
    Unknown(String),
}

/// Strategic resources in registration order, by token. Every resources
/// block key is registered as a C string and appends a new resource,
/// including repeated names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resources {
    tokens: Vec<Token>,
    /// Where a resource file's reading is lost: the names after it are
    /// unknown.
    gap: Option<String>,
}

impl Resources {
    /// Finds a resource from the key's C-string prefix.
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

/// What a state file's reading needs from other content.
pub struct Context<'a> {
    /// The resource lookup result for a key.
    pub resource: &'a dyn Fn(&[u8]) -> Resource,
    /// `NDefines.NBuildings.MAX_BUILDING_LEVELS`, the building-level cap.
    pub max_building_levels: i32,
    /// The map's province count; see [`province_count`].
    pub province_count: u32,
    /// Content names for building allowed-triggers; buildings load before states.
    pub names: &'a KeyNames,
}

/// Province count used to check state membership: one record per
/// `definition.csv` row, including the null row zero. Every other index
/// constructs an on-map province. The loader can also invent records for
/// undefined bitmap colours, which [`crate::map::decode_provinces_bmp`]
/// refuses; for a map this viewer loads, the count is therefore the row count.
pub fn province_count(definitions: &BTreeMap<u32, ProvinceDefinition>) -> u32 {
    definitions.len() as u32
}

/// The signed integer scan's stored value, if any.
fn scanned(value: &Value) -> Option<i32> {
    match crt::scan_i32(value.scalar().unwrap_or_default()) {
        Scanned::Stored(value) => Some(value),
        Scanned::End | Scanned::NoMatch => None,
    }
}

/// One building entry: a block reads level and an allowed-trigger;
/// otherwise the value is a signed integer. Level starts at zero and
/// is capped at the define.
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

/// A key with a positive integer prefix holds province-scoped entries;
/// any other key holds one state-wide entry.
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

/// Parses one history file's campaign-start state definitions and recovery
/// notes through one script reader ([`native`]). Every bare top-level state
/// keyword reads a template in file order; other keys, including quoted
/// spellings, are reported and their blocks skipped.
///
/// Unsettled consumption ([`native::Read::unsettled`]) or a resource block
/// depending on lexer registrations ([`Resource::Unknown`]) leaves the
/// current state and later definitions unknown. Refuse the file by line:
/// an unknown state could claim an id before a later file, and naval battle
/// names require every candidate province.
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

/// Attributes notes while the file loop still identifies its state.
/// Source lines cannot distinguish two states sharing a line or counted
/// effects appearing in several states.
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
    // History ownership fields take one value, victory_points an integer
    // list, and buildings a block. A non-block buildings value reports and
    // skips according to the current token. Other keys use named effect
    // dispatch; an unknown command is reported and skipped.
    fn command(field: &native::Field<'_>, names: &KeyNames) -> Shape<Node> {
        match field.keyword().as_deref() {
            Some(b"owner" | b"controller") => Shape::One,
            Some(b"victory_points") => Shape::List,
            Some(b"buildings") if field.opens() => Shape::Object(Node::Buildings),
            Some(b"buildings") => Shape::Skip,
            _ => crate::effect_keys::shape(Some(names), field).map(|()| Node::History),
        }
    }
    // A building block reads its fields; a scalar takes one value.
    fn building() -> Shape<Node> {
        Shape::ObjectIfOpen(Node::Building)
    }
    // The states dispatched so far, and the first whose resource block's
    // consumption is unknown, with why.
    let mut dispatched = 0_usize;
    let mut lost: Option<(usize, String)> = None;
    // Each file has one shared script reader. History effects inherit state scope.
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
            // Unexpected top-level fields report and skip their blocks.
            Node::Top => Shape::Skip,
            // State fields dispatch on bare keywords.
            Node::State => match field.keyword().as_deref() {
                // Integer, string, boolean, fixed5 and category fields
                // consume one value; projections below retain their own
                // conversion and lookup rules.
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
                // provinces counts nested braces.
                Some(b"provinces") => Shape::Tokens,
                // impassable_ignored_links reads an integer list.
                Some(b"impassable_ignored_links") => Shape::List,
                // color skips only a current block.
                Some(b"color") => Shape::SkipIfOpen,
                // resources reads its own keyed entries.
                Some(b"resources") => Shape::Object(Node::Resources),
                // history reads ordinary and dated entries on this reader.
                Some(b"history") => Shape::Object(Node::History),
                // Startup effects use named dispatch with state scope.
                Some(b"state_startup_effect") => Shape::EffectIn(ScopeKind::State.bit()),
                // Unexpected state fields report and skip.
                _ => Shape::Skip,
            },
            // Found resources consume a signed amount; unknown names
            // report and skip their blocks. An uncertain lookup also makes
            // a block's consumption uncertain.
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
            // A nonzero integer key opens a dated entry; other keys are
            // ordinary history commands.
            Node::History if decimal_i32(&field.key.text) != 0 => Shape::Object(Node::Entry),
            Node::History | Node::Entry => command(field, context.names),
            // Positive integer keys read province entries through the
            // closing brace whatever their value; other keys hold one entry.
            Node::Buildings if decimal_i32(&field.key.text) > 0 => Shape::Object(Node::Province),
            Node::Buildings | Node::Province => building(),
            // level takes one value, allowed takes an AND-trigger body;
            // other keys report and skip their blocks.
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
    // Every loop stops where an unsettled reading is lost, so the last
    // top-level item is the one being read; only a `state` reads by key.
    let unsettled = read.unsettled.as_ref().map(|unsettled| {
        let being_read = read
            .items
            .last()
            .is_some_and(|item| item.keyword_is("state"));
        (all.len() - usize::from(being_read), unsettled.to_string())
    });
    // The first state whose reading is lost, and why.
    let gap = match (unsettled, lost) {
        (Some(a), Some(b)) => Some(if b.0 < a.0 { b } else { a }),
        (a, b) => a.or(b),
    };
    if let Some((cut, why)) = gap {
        let being_read = match all.get(cut) {
            // The last id scanned so far.
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
    // A diagnostic outside a state has no narrower owner and affects
    // every retained definition, including earlier ones. Variable errors
    // are file-scoped because references share one table across nested
    // reads. Fields retaining no item have the same uncertain ownership.
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
        // Effects use their own key readers; fallback consumption can
        // change later templates in this file.
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
        // Keep a template only when its signed id is positive.
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

/// Projects one state block in keyword order: id and name take their last
/// value, while each provinces block appends number tokens with repeats,
/// which province initialization later skips. Values are already resolved
/// through file variables. Return recovery notes reflected by the definition.
fn parse_state_block(
    items: &[Item],
    context: &Context<'_>,
) -> io::Result<(StateDefinition, Vec<(MisreadCause, String)>)> {
    let mut notes = Vec::new();
    let mut state = StateDefinition::default();
    let mut off_map = Vec::new();
    for item in items {
        if item.keyword_is("id") {
            // A failed integer scan stores the nonpositive sentinel.
            state.id = scanned(&item.value).map_or(u32::MAX, |id| id as u32);
        } else if item.keyword_is("name") {
            // Cut at NUL and replace text holding LF with Unreadable String.
            state.name_key = item
                .value
                .read_str()
                .map(str::to_owned)
                .map_err(|why| format!("state name: {why}"));
        } else if item.keyword_is("impassable") {
            // Accept exact yes/no through the first NUL; a failed
            // conversion reports the field and preserves its previous value.
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
            // Keep number tokens only for map indices from 1 up to but
            // excluding the province count. Other indices select the null
            // province and are reported.
            for entry in item.value.items().unwrap_or_default() {
                // Bare number tokens only.
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
    // Category lookup uses C-string text without the name field's LF
    // check. Only the last assignment survives; nonpositive templates
    // are discarded before category lookup.
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
                // Keep exactly two values, each scanned into an initial
                // zero; any other list size records an error.
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

/// Strategic resource tokens from direct common/resources `.txt` files
/// ([`script_files::paths`]). Every key in a top-level resources block
/// registers a dynamic token and reads its resource block. Other top-level
/// keys are reported and their blocks skipped.
///
/// A modifier block with registry-dependent or unsettled consumption makes
/// later names in its file unknown: [`Resources::find`] returns that gap
/// when a lookup finds no resource already registered.
fn resources(layers: &ResolvedLayers) -> io::Result<Resources> {
    let mut resources = Resources::default();
    for path in script_files::paths(layers, "common/resources/", Some(".txt"))? {
        read_resources(&path.read(layers)?, &path.text(), &mut resources);
    }
    Ok(resources)
}

/// Appends resource declarations from one common/resources file.
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
            // Unexpected top-level fields report and skip their blocks.
            Node::Top => Shape::Skip,
            Node::Table => {
                resources.tokens.push(Token::c_str(&field.key.text));
                Shape::Object(Node::Resource)
            }
            // icon_frame, cic and convoys take one value. modifier passes
            // this same key to modifier dispatch, so its name remains
            // modifier; other keys report and skip.
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

/// State templates by id, keeping the first definition. A lost file read
/// refuses the whole inventory ([`read`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct States {
    definitions: BTreeMap<u32, StateDefinition>,
    sources: BTreeMap<u32, script_files::ScriptPath>,
    diagnostics: BTreeMap<u32, Vec<String>>,
}

impl States {
    /// The state of `id`: `None` when no file defines it.
    pub fn get(&self, id: u32) -> Option<&StateDefinition> {
        self.definitions.get(&id)
    }

    /// Every template.
    pub fn definitions(&self) -> &BTreeMap<u32, StateDefinition> {
        &self.definitions
    }

    pub fn source(&self, id: u32) -> Option<&script_files::ScriptPath> {
        self.sources.get(&id)
    }

    /// Reading diagnostics affecting the kept template. A note whose state
    /// ownership is unavailable affects every state defined by that file.
    pub fn diagnostics(&self, id: u32) -> &[String] {
        self.diagnostics.get(&id).map_or(&[], Vec::as_slice)
    }
}

/// Reads direct history/states `.txt` files from resolved layers in
/// [`script_files::paths`] order, with each state's file order retained.
/// A repeated id keeps its first definition and returns a conflict naming
/// both files. Recovery notes include file and line; an unsettled read
/// refuses the inventory instead of publishing a prefix.
///
/// `provinces` holds map definitions from [`crate::map::load`], loaded before states.
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

/// The same state read, retaining typed recovery causes for live consumers.
/// [`read`] keeps every diagnostic for the offline corpus.
pub fn read_with_diagnostics(
    layers: &ResolvedLayers,
    defines: &Defines,
    provinces: &BTreeMap<u32, ProvinceDefinition>,
) -> io::Result<(States, Vec<Diagnostic>)> {
    let resources = resources(layers)?;
    let names = layers.key_names()?;
    let max_building_levels = defines.int32("NBuildings", "MAX_BUILDING_LEVELS")?;
    let counted = province_count(provinces);
    let paths = script_files::paths(layers, DIRECTORY, Some(".txt"))?;
    // Each file reads and parses on its own; they merge below in listed order.
    let parsed = installed_content_core::parallel::map(&paths, |listed| {
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
            // A conflict discards only the new template, retaining the
            // earlier definition and its exact province list.
            if let Some(kept) = states.sources.get(&id) {
                conflicts.push(Diagnostic::from((
                    crate::DiagnosticKind::Warning,
                    format!(
                        "state {id} is defined by both {kept} and {path}; the game keeps the first"
                    ),
                )));
                continue;
            }
            states.sources.insert(id, listed.clone());
            states.diagnostics.insert(id, diagnostics);
            states.definitions.insert(id, state);
        }
    }
    Ok((states, conflicts))
}

#[cfg(test)]
mod tests;
