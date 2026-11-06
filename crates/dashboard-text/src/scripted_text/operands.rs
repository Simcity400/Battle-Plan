//! Scripted text expanded later from what it read at its hour, without the recording.
//! Preparation expands a text as [`ScriptedText`] does and records what each of its commands
//! read ([`Operands`]): the tag and naming operands of each country it names, the key each
//! scripted localisation selected, each variable's value, and any other property's text as the
//! preparation's language read it. A reader expands the text again in its own language from
//! them alone ([`expand`]), and refuses by name what the preparation did not read.

use super::{numeric::format_variable, properties::date_property_at, *};
use installed_content_country_names::country_names::Operands as Naming;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

/// The country properties read from a country's tag and naming operands alone.
const COUNTRY_TEXT: [&str; 15] = [
    "GetName",
    "GetNameDef",
    "GetNameDefCap",
    "GetAdjective",
    "GetAdjectiveCap",
    "GetTag",
    "GetFlag",
    "GetNameWithFlag",
    "GetNonIdeologyName",
    "GetNonIdeologyNameDef",
    "GetNonIdeologyNameDefCap",
    "GetNonIdeologyAdjective",
    "GetNonIdeologyAdjectiveCap",
    "GetRulingIdeology",
    "GetRulingIdeologyNoun",
];

/// [`COUNTRY_TEXT`]'s property `name` of the country tagged `tag`, from its names (`names`, by
/// key suffix) and its ruling party's ideology group (`group`) in `localisation`; None for any
/// other property. The hour's evaluation and a reader share it.
pub(super) fn country_text(
    name: &str,
    tag: &str,
    localisation: &Localisation,
    names: impl Fn(&str) -> Result<String, String>,
    group: impl FnOnce() -> Result<Option<String>, String>,
) -> Result<Option<String>, String> {
    let is = |entry: &str| named(name, entry);
    let localise = |key: &str| {
        localisation
            .localise(key.as_bytes())
            .map_err(|refusal| refusal.to_string())
    };
    // Non-ideology properties probe the current tag without ideology.
    let non_ideology = |suffix: &str| -> Result<String, String> {
        let key = format!("{tag}{suffix}");
        if localisation.get(&key).is_some() {
            localise(&key)
        } else {
            names(suffix)
        }
    };
    // The first byte's toupper in the "C" locale ([`crt_case_text`]).
    let upper_first = |text: String| crt_case_text(&text, true, true);
    Ok(Some(if is("GetName") {
        names("")?
    } else if is("GetNameDef") {
        names("_DEF")?
    } else if is("GetNameDefCap") {
        upper_first(names("_DEF")?)
    } else if is("GetAdjective") {
        names("_ADJ")?
    } else if is("GetAdjectiveCap") {
        upper_first(names("_ADJ")?)
    } else if is("GetTag") {
        tag.to_owned()
    } else if is("GetFlag") {
        format!("\u{12}{tag}")
    } else if is("GetNameWithFlag") {
        format!("\u{12}{tag} {}", upper_first(names("_DEF")?))
    } else if is("GetNonIdeologyName") {
        non_ideology("")?
    } else if is("GetNonIdeologyNameDef") {
        non_ideology("_DEF")?
    } else if is("GetNonIdeologyNameDefCap") {
        upper_first(non_ideology("_DEF")?)
    } else if is("GetNonIdeologyAdjective") {
        non_ideology("_ADJ")?
    } else if is("GetNonIdeologyAdjectiveCap") {
        upper_first(non_ideology("_ADJ")?)
    } else if is("GetRulingIdeology") || is("GetRulingIdeologyNoun") {
        let Some(group) = group()? else {
            return Err(format!("{name}: {tag} has no ruling party"));
        };
        let key = if is("GetRulingIdeologyNoun") {
            format!("{group}_noun")
        } else {
            group
        };
        // Lowercase with the "C" locale's tolower.
        crt_case_text(&localise(&key)?, false, false)
    } else {
        return Ok(None);
    }))
}

/// A country an expansion names, as its hour held it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Country {
    /// Its tag, or why the hour holds none.
    pub(crate) tag: Result<String, String>,
    /// Its naming operands, or why the hour holds none.
    pub(crate) naming: Result<Naming, String>,
}

#[derive(Serialize)]
struct SavedCountry<'a> {
    tag: Result<&'a str, &'a str>,
    naming: Result<(Option<u32>, [&'a str; 6]), &'a str>,
}

#[derive(Deserialize)]
struct RestoredCountry {
    tag: Result<String, String>,
    naming: Result<(Option<u32>, [String; 6]), String>,
}

impl Serialize for Country {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        SavedCountry {
            tag: self.tag.as_deref().map_err(String::as_str),
            naming: self
                .naming
                .as_ref()
                .map(Naming::parts)
                .map_err(String::as_str),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Country {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let RestoredCountry { tag, naming } = RestoredCountry::deserialize(deserializer)?;
        Ok(Self {
            tag,
            naming: naming.map(|(overlord, parts)| Naming::from_parts(overlord, parts)),
        })
    }
}

/// A command's value, as its hour decided it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Value {
    /// A [`COUNTRY_TEXT`] property of the country of a canonical slot, named again from its
    /// tag and naming operands.
    Country(u32),
    /// The key a scripted localisation selected and the context its own commands were recorded
    /// in; none when it selected no text.
    Entry(Option<(Vec<u8>, u32)>),
    /// A variable's value.
    Number(i64),
    /// A date property of the hour.
    Date,
    /// Text the same in every language: an id, a null object's, or none.
    Text(String),
    /// Text as the preparation's language read it, and whether computing it evaluated what may
    /// set what later commands read.
    Read { text: String, stateful: bool },
    /// Why it has no value.
    Refused(String),
}

impl Value {
    /// `text`, `object`'s property `name`, whose computing evaluated what may set what later
    /// commands read when `stateful`: kept as the same in every language where it is.
    pub(super) fn of_property(object: Object, name: &str, text: &str, stateful: bool) -> Self {
        let same = match object {
            Object::FactionEnvironment(_) => true,
            Object::Faction(faction) => faction.is_none() || !named(name, "GetName"),
            Object::State(state) => state.is_none() || named(name, "GetID"),
            Object::Event | Object::Country(_) | Object::Character(_) => false,
        };
        if same && !stateful {
            Self::Text(text.to_owned())
        } else {
            Self::Read {
                text: text.to_owned(),
                stateful,
            }
        }
    }

    /// Whether it may read or set what the commands before and after it read: a scripted
    /// localisation's triggers, a variable read, and a property computed by either.
    fn stateful(&self) -> bool {
        matches!(
            self,
            Self::Entry(_) | Self::Number(_) | Self::Read { stateful: true, .. }
        )
    }

    fn heap_bytes(&self) -> usize {
        match self {
            Self::Entry(Some((key, _))) => key.capacity(),
            Self::Text(text) | Self::Read { text, .. } | Self::Refused(text) => text.capacity(),
            Self::Country(_) | Self::Entry(None) | Self::Number(_) | Self::Date => 0,
        }
    }
}

/// A command between brackets, and its value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Command {
    pub(crate) inner: String,
    pub(crate) value: Value,
}

/// What expansions read at one hour: one text per localisation key. Texts expanded in other
/// scopes at the same hour take operands of their own.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Operands {
    hour: i32,
    /// The preparation's language, which [`Value::Read`] texts are in.
    language: String,
    /// Each country the expansions name, sorted by canonical slot.
    countries: Vec<(u32, Country)>,
    /// Which of slot zero's names its reserved row had built.
    reserved: Option<u8>,
    /// Each text recorded, sorted by its localisation key: the context of its commands, or why
    /// none were read.
    texts: Vec<(Vec<u8>, Result<u32, String>)>,
    /// The commands each context evaluated, in order: a text's, and each scripted
    /// localisation's it selected.
    contexts: Vec<Vec<Command>>,
}

impl Operands {
    pub fn new(hour: i32) -> Self {
        Self {
            hour,
            ..Self::default()
        }
    }

    pub fn hour(&self) -> i32 {
        self.hour
    }

    /// Whether it records no text.
    pub fn is_empty(&self) -> bool {
        self.texts.is_empty()
    }

    /// The bytes it holds on the heap.
    pub fn heap_bytes(&self) -> usize {
        let text = |text: &Result<String, String>| match text {
            Ok(text) | Err(text) => text.capacity(),
        };
        self.language.capacity()
            + self.countries.capacity() * size_of::<(u32, Country)>()
            + self
                .countries
                .iter()
                .map(|(_, country)| {
                    text(&country.tag)
                        + match &country.naming {
                            Ok(naming) => naming.heap_bytes(),
                            Err(reason) => reason.capacity(),
                        }
                })
                .sum::<usize>()
            + self.texts.capacity() * size_of::<(Vec<u8>, Result<u32, String>)>()
            + self
                .texts
                .iter()
                .map(|(key, context)| {
                    key.capacity() + context.as_ref().err().map_or(0, String::capacity)
                })
                .sum::<usize>()
            + self.contexts.capacity() * size_of::<Vec<Command>>()
            + self
                .contexts
                .iter()
                .flatten()
                .map(|command| command.inner.capacity() + command.value.heap_bytes())
                .sum::<usize>()
            + self
                .contexts
                .iter()
                .map(|commands| commands.capacity() * size_of::<Command>())
                .sum::<usize>()
    }

    /// The country of canonical `slot`, when recorded.
    fn country(&self, slot: u32) -> Option<&Country> {
        self.countries
            .binary_search_by_key(&slot, |(held, _)| *held)
            .ok()
            .map(|at| &self.countries[at].1)
    }

    /// The recording of the text of `key`.
    fn text(&self, key: &[u8]) -> Option<&Result<u32, String>> {
        self.texts
            .binary_search_by(|(held, _)| held.as_slice().cmp(key))
            .ok()
            .map(|at| &self.texts[at].1)
    }

    /// Records the text of `key` as `recorded`, unless one is recorded already.
    fn begin_text(&mut self, key: &[u8], recorded: Result<u32, String>) -> bool {
        match self
            .texts
            .binary_search_by(|(held, _)| held.as_slice().cmp(key))
        {
            Ok(_) => false,
            Err(at) => {
                self.texts.insert(at, (key.to_vec(), recorded));
                true
            }
        }
    }

    /// A context for commands of its own.
    fn context(&mut self) -> u32 {
        self.contexts.push(Vec::new());
        (self.contexts.len() - 1) as u32
    }
}

/// An expansion recording what it reads: into `operands`, its commands into context `at`.
#[derive(Default)]
pub(super) struct Recording {
    operands: Operands,
    at: u32,
    /// The key the scripted localisation being evaluated selected, and its context.
    selected: Option<(Vec<u8>, u32)>,
    /// Properties being computed, whose texts are recorded whole ([`ExpansionScopes::muted`]),
    /// and whether what they evaluated may set what later commands read.
    muted: u32,
    stateful: bool,
}

fn lock(recording: &Mutex<Recording>) -> MutexGuard<'_, Recording> {
    recording.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ExpansionScopes<'_, '_> {
    /// The recording, unless none is made or a property being computed mutes it.
    fn recorded(&self) -> Option<MutexGuard<'_, Recording>> {
        self.recording
            .as_deref()
            .map(lock)
            .filter(|recording| recording.muted == 0)
    }

    /// Whether the expansion records what it reads.
    pub(super) fn recording(&self) -> bool {
        self.recorded().is_some()
    }

    /// `compute`, recording nothing it evaluates: a property's text, whatever it expanded inside,
    /// is recorded whole, so a reader never takes those commands for the text's own. Whether
    /// what it evaluated may set what later commands read is given back with it.
    pub(super) fn muted<T>(&self, compute: impl FnOnce() -> T) -> (T, bool) {
        let Some(recording) = self.recording.as_deref() else {
            return (compute(), false);
        };
        let outer = {
            let mut recording = lock(recording);
            recording.muted += 1;
            std::mem::take(&mut recording.stateful)
        };
        let value = compute();
        let mut recording = lock(recording);
        recording.muted -= 1;
        let stateful = recording.stateful;
        recording.stateful |= outer;
        (value, stateful)
    }

    /// Records `value` as the value of command `inner`, when the expansion records; while a
    /// property is computed, only whether it may set what later commands read.
    pub(super) fn note(&self, inner: &str, value: impl FnOnce() -> Value) {
        let Some(mut recording) = self.recording.as_deref().map(lock) else {
            return;
        };
        let value = value();
        if recording.muted != 0 {
            recording.stateful |= value.stateful();
            return;
        }
        let at = recording.at as usize;
        recording.operands.contexts[at].push(Command {
            inner: inner.to_owned(),
            value,
        });
    }

    /// Records `reason` as the value of command `inner`, and gives it back.
    pub(super) fn refuse(&self, inner: &str, reason: String) -> String {
        self.note(inner, || Value::Refused(reason.clone()));
        reason
    }

    /// Before a scripted localisation is evaluated: what it selects is kept apart from what
    /// the one being evaluated around it selected, which this gives back. Its triggers may set
    /// what later commands read, which a property computing it records.
    pub(super) fn begin_entry(&self) -> Option<(Vec<u8>, u32)> {
        let mut recording = self.recording.as_deref().map(lock)?;
        if recording.muted != 0 {
            recording.stateful = true;
            return None;
        }
        recording.selected.take()
    }

    /// After it: what it selected, the one around it's `outer` restored.
    pub(super) fn end_entry(&self, outer: Option<(Vec<u8>, u32)>) -> Option<(Vec<u8>, u32)> {
        self.recorded()
            .and_then(|mut recording| std::mem::replace(&mut recording.selected, outer))
    }

    /// The scripted localisation being evaluated selected `key`: the commands of its text go to
    /// a context of their own until [`Self::leave`] restores the outer one this gives back.
    pub(super) fn select(&self, key: &[u8]) -> Option<u32> {
        let mut recording = self.recorded()?;
        let context = recording.operands.context();
        recording.selected = Some((key.to_vec(), context));
        Some(std::mem::replace(&mut recording.at, context))
    }

    pub(super) fn leave(&self, outer: Option<u32>) {
        if let Some(at) = outer
            && let Some(mut recording) = self.recorded()
        {
            recording.at = at;
        }
    }
}

impl<'a> ScriptedText<'a> {
    /// What a reader reads again of `object`'s property `name` in its own language, recorded
    /// with what that reads; None for a property a reader is given as text.
    pub(super) fn rereads(
        &self,
        object: Object,
        name: &str,
        scopes: &ExpansionScopes<'_, 'a>,
    ) -> Option<Value> {
        match object {
            Object::Country(slot) if COUNTRY_TEXT.iter().any(|entry| named(name, entry)) => {
                if let Some(mut recording) = scopes.recorded() {
                    self.record_country(&mut recording.operands, slot);
                }
                Some(Value::Country(slot))
            }
            Object::Event if DATES.iter().any(|entry| named(name, entry)) => Some(Value::Date),
            _ => None,
        }
    }

    /// Records the tag and naming operands of the country of canonical `slot`, and of the
    /// overlords its names reach, once.
    fn record_country(&self, operands: &mut Operands, slot: u32) {
        let mut pending = vec![slot];
        while let Some(slot) = pending.pop() {
            let Err(at) = operands
                .countries
                .binary_search_by_key(&slot, |(held, _)| *held)
            else {
                continue;
            };
            let naming = self
                .names
                .operands(slot)
                .cloned()
                .unwrap_or_else(|| Err(format!("country slot {slot} has no naming operands")));
            if let Ok(naming) = &naming
                && let Some(overlord) = naming.overlord
            {
                pending.push(overlord);
            }
            if slot == 0 {
                operands.reserved = self.names.reserved_built();
            }
            let tag = self.country_tag(slot).map(str::to_owned);
            operands
                .countries
                .insert(at, (slot, Country { tag, naming }));
        }
    }

    /// [`Self::expand_in`], recording in `operands` what `text`, the localisation of `key`,
    /// reads: each command's value as this hour decides it, and each country its names read.
    /// A key `operands` records already keeps that recording; text without `[` reads nothing.
    pub fn record(
        &self,
        key: &[u8],
        text: &str,
        scopes: &EventScopes,
        operands: &mut Operands,
    ) -> Result<String, String> {
        self.record_with(key, text, scopes, None, operands)
    }

    /// [`Self::expand`], recorded as [`Self::record`] records: in the root country's own event
    /// scope.
    pub fn record_root(
        &self,
        key: &[u8],
        text: &str,
        operands: &mut Operands,
    ) -> Result<String, String> {
        let scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(self.root))]);
        self.record(key, text, &scopes, operands)
    }

    /// [`Self::expand_in_without_random_seed`], recorded as [`Self::record`] records.
    pub fn record_unseeded(
        &self,
        key: &[u8],
        text: &str,
        scopes: &EventScopes,
        reason: &'static str,
        operands: &mut Operands,
    ) -> Result<String, String> {
        self.record_with(key, text, scopes, Some(reason), operands)
    }

    fn record_with(
        &self,
        key: &[u8],
        text: &str,
        scopes: &EventScopes,
        unseeded: Option<&'static str>,
        operands: &mut Operands,
    ) -> Result<String, String> {
        if scopes.scopes.is_empty() {
            let reason = "scripted text needs an event scope".to_owned();
            operands.begin_text(key, Err(reason.clone()));
            return Err(reason);
        }
        if !text.contains('[') {
            return Ok(text.to_owned());
        }
        let mut expansion = ExpansionScopes::new(scopes);
        if let Some(reason) = unseeded {
            expansion.scopes.borrow_mut()[0].random_seed_refusal = Some(reason);
        }
        if operands.language.is_empty() {
            operands.language.clone_from(&self.localisation.language);
        }
        let hour = self.frame.hour();
        let mixed = if operands.language != self.localisation.language {
            Some(format!(
                "the text was prepared in {}, beside texts prepared in {}",
                self.localisation.language, operands.language
            ))
        } else if operands.hour != hour {
            Some(format!(
                "the text was read at hour {hour}, beside texts read at hour {}",
                operands.hour
            ))
        } else {
            None
        };
        if let Some(reason) = mixed {
            operands.begin_text(key, Err(reason));
        } else if operands.text(key).is_none() {
            let context = operands.context();
            operands.begin_text(key, Ok(context));
            expansion.recording = Some(Arc::new(Mutex::new(Recording {
                operands: std::mem::take(operands),
                at: context,
                selected: None,
                muted: 0,
                stateful: false,
            })));
        }
        let expanded = self.process(text, 0, &expansion, 0, Object::Event);
        if let Some(recording) = expansion.recording.take() {
            // Its copies on larger stacks are gone once the expansion returns.
            let recording = Arc::try_unwrap(recording)
                .map(|recording| {
                    recording
                        .into_inner()
                        .unwrap_or_else(PoisonError::into_inner)
                })
                .unwrap_or_else(|shared| std::mem::take(&mut *lock(&shared)));
            *operands = recording.operands;
        }
        expanded
    }
}

/// Expands `text`, the localisation of `key`, from what its preparation recorded of its hour
/// ([`ScriptedText::record`]), `operands`, in `localisation`'s language; None when it recorded
/// nothing.
pub fn expand(
    text: &str,
    key: &[u8],
    operands: Option<&Operands>,
    localisation: &Localisation,
    content: &ScriptedContent,
) -> Result<String, String> {
    let empty = Operands::default();
    let operands = operands.unwrap_or(&empty);
    let context = match operands.text(key) {
        Some(Ok(context)) => Some(*context),
        // As the hour could not expand it at all.
        Some(Err(reason)) => return Err(reason.clone()),
        None => None,
    };
    let recorded = Recorded::new(operands, localisation, content);
    let mut cursor = Cursor::default();
    recorded.text(text, 0, context, &mut cursor, Nesting::default())
}

/// Scripted localisations a reader is inside, and whether it reads on a stack that holds as many
/// as the game nests ([`MAX_SCRIPTED_NESTING`]).
#[derive(Clone, Copy, Default)]
struct Nesting {
    entries: usize,
    large: bool,
}

/// How deep a reader nests scripted localisations on the stack it was called on, as the hour's
/// evaluation hands deeper ones to a larger stack.
const SHALLOW: usize = 8;

/// How far an expansion has read each context it reached.
#[derive(Default)]
struct Cursor<'r> {
    contexts: HashMap<u32, Reading<'r>>,
}

/// How far an expansion has read one context.
struct Reading<'r> {
    evaluated: usize,
    /// Whether the context holds a command whose value depends on those evaluated before it.
    stateful: bool,
    /// Once a command is read out of its recorded order: how many of each name were read, and
    /// the context's commands by name.
    unordered: Option<(HashMap<&'r str, usize>, HashMap<&'r str, Vec<usize>>)>,
}

/// An hour's recorded operands, named and localised in a reader's language.
struct Recorded<'r> {
    operands: &'r Operands,
    localisation: &'r Localisation,
    content: &'r ScriptedContent,
}

impl<'r> Recorded<'r> {
    fn new(
        operands: &'r Operands,
        localisation: &'r Localisation,
        content: &'r ScriptedContent,
    ) -> Self {
        Self {
            operands,
            localisation,
            content,
        }
    }

    fn localise(&self, key: &str) -> Result<String, String> {
        self.localisation
            .localise(key.as_bytes())
            .map_err(|refusal| refusal.to_string())
    }

    /// [`ScriptedText::process`] from the commands recorded in `context`; None where none were.
    fn text(
        &self,
        text: &str,
        depth: i32,
        context: Option<u32>,
        cursor: &mut Cursor<'r>,
        nesting: Nesting,
    ) -> Result<String, String> {
        if depth >= self.content.recursion {
            return Ok(text.to_owned());
        }
        let mut out = String::new();
        let mut rest = text;
        loop {
            let Some(open) = rest.find('[') else {
                out.push_str(rest);
                return Ok(out);
            };
            let Some(length) = rest[open + 1..].find(']') else {
                return Ok(out);
            };
            let inner = &rest[open + 1..open + 1 + length];
            out.push_str(&rest[..open]);
            if let Some(refusal) = conditional(inner) {
                return Err(refusal);
            }
            let value = self.command(context, inner, cursor)?;
            if let Some(variable) = inner.strip_prefix('?') {
                match value {
                    Value::Number(value) => out.push_str(&format_variable(variable, value)),
                    Value::Refused(reason) => return Err(reason),
                    _ => return Err(format!("[{inner}] was recorded as no variable")),
                }
                rest = &rest[open + 1 + length + 1..];
                continue;
            }
            let value = self.value(inner, value, cursor, nesting)?;
            out.push_str(&self.text(&value, depth + 1, context, cursor, nesting)?);
            rest = &rest[open + 1 + length + 1..];
        }
    }

    /// The value recorded for this evaluation of `inner` in `context`: its next one there.
    fn command(
        &self,
        context: Option<u32>,
        inner: &str,
        cursor: &mut Cursor<'r>,
    ) -> Result<Value, String> {
        let unevaluated = || format!("[{inner}] was not evaluated when the text was prepared");
        let Some(at) = context.map(|at| at as usize) else {
            return Err(unevaluated());
        };
        let Some(commands) = self.operands.contexts.get(at) else {
            return Err(unevaluated());
        };
        let reading = cursor.contexts.entry(at as u32).or_insert_with(|| Reading {
            evaluated: 0,
            stateful: commands.iter().any(|command| command.value.stateful()),
            unordered: None,
        });
        let position = reading.evaluated;
        reading.evaluated += 1;
        let (seen, by_name) = match &mut reading.unordered {
            Some(unordered) => unordered,
            // Read in order so far, the command at this position is this one's next of its name.
            None if commands
                .get(position)
                .is_some_and(|command| command.inner == inner) =>
            {
                return Ok(commands[position].value.clone());
            }
            unordered @ None => {
                let mut seen: HashMap<&'r str, usize> = HashMap::new();
                for command in &commands[..position.min(commands.len())] {
                    *seen.entry(command.inner.as_str()).or_default() += 1;
                }
                let mut by_name: HashMap<&'r str, Vec<usize>> = HashMap::new();
                for (index, command) in commands.iter().enumerate() {
                    by_name
                        .entry(command.inner.as_str())
                        .or_default()
                        .push(index);
                }
                unordered.insert((seen, by_name))
            }
        };
        let Some((&name, indices)) = by_name.get_key_value(inner) else {
            return Err(unevaluated());
        };
        let ordinal = seen.entry(name).or_default();
        let Some(&index) = indices.get(*ordinal) else {
            return Err(unevaluated());
        };
        *ordinal += 1;
        if index != position && reading.stateful {
            return Err(format!(
                "[{inner}]: this text evaluates its commands in another order than the text it was prepared from, and their values depend on that order"
            ));
        }
        Ok(commands[index].value.clone())
    }

    /// The text of command `inner`, whose recorded value is `value`.
    fn value(
        &self,
        inner: &str,
        value: Value,
        cursor: &mut Cursor<'r>,
        nesting: Nesting,
    ) -> Result<String, String> {
        let property = inner.rsplit('.').next().unwrap_or_default();
        let within = |reason: String| format!("[{inner}]: {reason}");
        match value {
            Value::Country(slot) => self
                .country(slot, property)
                .map_err(within)?
                .ok_or_else(|| within(format!("{property} is no country name"))),
            Value::Entry(None) => Ok(String::new()),
            Value::Entry(Some((key, context))) => {
                let text = self
                    .localisation
                    .localise(&key)
                    .map_err(|refusal| within(refusal.to_string()))?;
                let expanded = self
                    .entered(nesting, cursor, |cursor, nesting| {
                        self.text(&text, 0, Some(context), cursor, nesting)
                    })
                    .map_err(within)?;
                self.complete(context, cursor).map_err(within)?;
                Ok(expanded)
            }
            Value::Date => date_property_at(self.operands.hour, property, |key| self.localise(key))
                .map_err(within)?
                .ok_or_else(|| within(format!("{property} is no date property"))),
            Value::Text(text) => Ok(text),
            Value::Read { text, .. } if self.localisation.language == self.operands.language => {
                Ok(text)
            }
            Value::Read { .. } => Err(within(format!(
                "its text was read in {} when the text was prepared, and the viewer's language is {}",
                self.operands.language, self.localisation.language
            ))),
            Value::Refused(reason) => Err(reason),
            Value::Number(_) => Err(within("a variable's value was recorded for it".into())),
        }
    }

    /// `read`, one scripted localisation deeper: past [`SHALLOW`], on a stack that holds as many
    /// as the hour's evaluation nested, as that evaluation reads them.
    fn entered(
        &self,
        nesting: Nesting,
        cursor: &mut Cursor<'r>,
        read: impl FnOnce(&mut Cursor<'r>, Nesting) -> Result<String, String> + Send,
    ) -> Result<String, String> {
        let deeper = Nesting {
            entries: nesting.entries + 1,
            ..nesting
        };
        if nesting.large || deeper.entries < SHALLOW {
            return read(cursor, deeper);
        }
        let caller = job_priority::Caller::current();
        let cpu = replay_frame::cpu::Context::capture();
        let diagnostics = installed_content_core::content::diagnostics::Context::capture();
        let memory = engine_allocation::labels::current();
        std::thread::scope(|scope| {
            std::thread::Builder::new()
                .name("scripted-localisation".into())
                // As the hour's evaluation reserves: 64 KiB per backstop level.
                .stack_size(MAX_SCRIPTED_NESTING * 64 * 1024)
                .spawn_scoped(scope, move || {
                    let _memory = engine_allocation::labels::resume(memory);
                    diagnostics.enter(|| {
                        let _cpu = cpu.enter("scripted localisation");
                        caller.enter();
                        read(
                            cursor,
                            Nesting {
                                large: true,
                                ..deeper
                            },
                        )
                    })
                })
                .map_err(|error| {
                    format!("scripted localisation expansion thread could not start: {error}")
                })?
                .join()
                .map_err(|_| "scripted localisation expansion thread failed".to_owned())?
        })
    }

    /// Whether a scripted localisation's text read every command recorded in `context` that later
    /// values may depend on: those it left unread may have set what the hour's later commands read.
    fn complete(&self, context: u32, cursor: &Cursor<'r>) -> Result<(), String> {
        let evaluated = cursor
            .contexts
            .get(&context)
            .map_or(0, |reading| reading.evaluated);
        let unread = self.operands.contexts[context as usize]
            .get(evaluated..)
            .unwrap_or_default();
        if unread.iter().any(|command| command.value.stateful()) {
            return Err(
                "this text evaluates fewer of its commands than the text it was prepared from, and the values after them depend on those it leaves out"
                    .into(),
            );
        }
        Ok(())
    }

    /// [`country_text`] of the country of canonical `slot`, from its recorded tag and naming
    /// operands.
    fn country(&self, slot: u32, name: &str) -> Result<Option<String>, String> {
        let tag = match self.operands.country(slot) {
            Some(country) => country.tag.as_deref().map_err(Clone::clone)?,
            None => {
                return Err(format!(
                    "the tag of country slot {slot} at the text's hour was not recorded"
                ));
            }
        };
        // Built only for the properties that name it, not its tag or flag.
        let names = std::cell::OnceCell::new();
        let names = || names.get_or_init(|| self.names(slot));
        country_text(
            name,
            tag,
            self.localisation,
            |suffix| names().name_with(slot, suffix),
            || Ok(names().ruling_group(slot)?.map(str::to_owned)),
        )
    }

    /// Names of the country of canonical `slot` from its recorded naming operands and those of
    /// the overlords its names reach, which name it as all of them would.
    fn names(&self, slot: u32) -> CountryNames<'r> {
        let mut reached = BTreeMap::new();
        let mut next = Some(slot);
        while let Some(slot) = next.take()
            && !reached.contains_key(&slot)
            && let Some(country) = self.operands.country(slot)
        {
            next = country
                .naming
                .as_ref()
                .ok()
                .and_then(|naming| naming.overlord);
            reached.insert(slot, country.naming.clone());
        }
        CountryNames::recorded(self.localisation, reached, self.operands.reserved)
    }
}

#[cfg(test)]
#[path = "operands_tests.rs"]
mod tests;
