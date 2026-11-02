//! Ideas' emit: each country's holdings as runs on the family's series publication, one key per
//! idea it ever held, valued by the appointment holding it; the hours it was in the countries
//! unresolved; its identity starts with its naming, its advisors' templates and its warnings as
//! events; every hour whose inputs were complete; each law change or appointment as an event;
//! and the recorded modifier definitions where they changed.
//!
//! A country is observed at every hour its inputs are complete and the links resolve its holder.
//! A holding goes on while the same holder in the same seat holds it at each hour; an hour not
//! walked, one with its inputs incomplete, one where its holder is unresolved or gone, and a new
//! holder identity end every holding, and the next starts its holdings anew. An addition is a
//! holding started within an observation that goes on from the hour before, other than its
//! holder renamed in the same seat.

use super::facts::{self, Definitions, Facts, Read, Refusal, Role, Row, Template};
use dashboard_base::products::idea_history::{Addition, IdeaCharacter, IdeaSpan, refusal};
use dashboard_series::{
    Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
};
use installed_content_core::characters::BuiltTemplate;
use replay_data::rows::ModifierDefinitionRow;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// A country's holding of each idea, by the idea's token id, valued by the appointment holding
/// it ([`ROLES`], 0 for none); the hours it was unresolved; and its identity starts, templates
/// and warnings, as events.
pub const HOLD: u64 = 0;
pub const UNRESOLVED: u64 = 1;
pub const IDENTITY: u64 = 2;
pub const TEMPLATES: u64 = 3;
pub const WARNINGS: u64 = 4;
/// The family's keys: the hours its inputs were complete, the additions and the modifier
/// definitions.
pub const KNOWN: u64 = 0;
pub const ADDITIONS: u64 = 1;
pub const DEFINITIONS: u64 = 2;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
/// Family scalars: the feed's refusals, `[List [Text reason, Unsigned count]…]`; the additions
/// first observed after an unobserved stretch, `[Unsigned]`; each idea's token by id; and each
/// appointment by id from 1, `[Text seat, Unsigned character, Text token, Text name]`.
pub const REFUSALS: u64 = 0;
pub const COUNTS: u64 = 1;
pub const TOKENS: u64 = 1 << 24;
pub const ROLES: u64 = 1 << 28;
/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;

/// Event kinds, each `[Text json]`.
pub mod kind {
    /// A [`dashboard_text::identity::NamingCountry`], on a country's IDENTITY key.
    pub const IDENTITY: u64 = 0;
    /// Its templates, `[[character, Kept]…]`, on its TEMPLATES key.
    pub const TEMPLATES: u64 = 1;
    /// Its warnings, `[text…]`, on its WARNINGS key.
    pub const WARNINGS: u64 = 2;
    /// An [`dashboard_base::products::idea_history::Addition`], on the family's ADDITIONS key.
    pub const ADDITION: u64 = 3;
    /// The definitions, `[[key, Definition]…]`, on the family's DEFINITIONS key.
    pub const DEFINITIONS: u64 = 4;
}

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved ideas emit is not valid")
}

/// An advisor's template as published: built, refused, or unknown for want of its character's
/// row, as of `hour`, or of the last hour the inputs were complete while it stays so.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kept {
    Built(BuiltTemplate),
    Refused(String),
    Missing { complete: bool, hour: Option<i32> },
}

/// What a modifier definition's display reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
    pub index: u32,
    pub name_key: String,
    pub postfix_key: String,
    pub argument: String,
    pub flags: u32,
    pub precision: i32,
    pub feature: u32,
}

impl Definition {
    fn of(row: &ModifierDefinitionRow) -> Self {
        Self {
            index: row.index,
            name_key: String::from_utf8_lossy(&row.name_key).into_owned(),
            postfix_key: String::from_utf8_lossy(&row.postfix_key).into_owned(),
            argument: String::from_utf8_lossy(&row.argument).into_owned(),
            flags: row.flags,
            precision: row.precision,
            feature: row.feature_selector,
        }
    }

    /// The definition as a row: the fields its display reads.
    pub fn row(&self) -> ModifierDefinitionRow {
        ModifierDefinitionRow {
            index: self.index,
            name_key: self.name_key.as_bytes().to_vec(),
            postfix_key: self.postfix_key.as_bytes().to_vec(),
            argument: self.argument.as_bytes().to_vec(),
            flags: self.flags,
            precision: self.precision,
            feature_selector: self.feature,
            ..Default::default()
        }
    }
}

/// A holding's holder as additions compare it: its seat and character.
type Holder = Option<(String, u32)>;

fn holder(role: &Option<Arc<Role>>) -> Holder {
    role.as_ref().map(|role| (role.seat.clone(), role.id))
}

/// Adds `first..=last` to merged `spans`.
fn mark(spans: &mut Vec<(i32, i32)>, first: i32, last: i32) {
    match spans.last_mut() {
        Some(span) if span.1.saturating_add(1) >= first => span.1 = span.1.max(last),
        _ => spans.push((first, last)),
    }
}

/// The first hour of the unobserved stretch `lists` hold through `at - 1`, merged across them.
fn blind_before(at: i32, lists: [&[(i32, i32)]; 2]) -> Option<i32> {
    let mut first = at;
    loop {
        let Some(reach) = first.checked_sub(1) else {
            break;
        };
        let mut moved = false;
        for list in lists {
            let index = list.partition_point(|&(from, _)| from <= reach);
            if let Some(&(from, to)) = index.checked_sub(1).and_then(|index| list.get(index))
                && to >= reach
                && from < first
            {
                first = from;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    (first < at).then_some(first)
}

/// What a country holds now, and its keys.
#[derive(Default)]
struct Country {
    /// UNRESOLVED, IDENTITY, TEMPLATES and WARNINGS.
    keys: [KeyHandle; 4],
    /// Each idea's HOLD key, by token id.
    holds: HashMap<u64, KeyHandle>,
    /// Whether the countries hold its tag.
    listed: bool,
    identity: Option<(u32, u32)>,
    /// Observed at the hour before, its holdings open.
    active: bool,
    open: BTreeMap<Arc<str>, Option<Arc<Role>>>,
    /// Each idea's last ended holding: its last hour and holder.
    ended: HashMap<Arc<str>, (i32, Holder)>,
    /// The first hour of its open unresolved stretch, and its ended stretches.
    unresolved: Option<i32>,
    stretches: Vec<(i32, i32)>,
    templates: BTreeMap<u32, Kept>,
    /// The characters whose template waits on their row, dated while it does.
    dated: Vec<u32>,
    refused: BTreeMap<Refusal, BTreeSet<Row>>,
    tokenless: BTreeSet<(u32, Box<str>)>,
    warnings: Vec<String>,
}

impl Country {
    /// Its warnings: appointments with no idea token, then each refusal's rows by reason.
    fn warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        if !self.tokenless.is_empty() {
            warnings.push(refusal::TOKENLESS_SEATS.to_owned());
        }
        let mut refused: Vec<_> = self
            .refused
            .iter()
            .map(|(why, rows)| (why.text(), rows.len()))
            .collect();
        refused.sort_unstable();
        warnings.extend(
            refused
                .into_iter()
                .map(|(reason, count)| format!("{reason} (count: {count})")),
        );
        warnings
    }
}

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    /// What each tag holds, by actor.
    countries: Vec<Country>,
    /// KNOWN, ADDITIONS and DEFINITIONS.
    family: Option<[KeyHandle; 3]>,
    /// Each idea's token id, and each appointment's id with the appointments by id less one.
    tokens: HashMap<Arc<str>, u64>,
    roles: HashMap<Arc<Role>, u64>,
    listed_roles: Vec<Arc<Role>>,
    /// Whether the inputs were complete at the last hour, and the last hour they were.
    known: bool,
    known_last: Option<i32>,
    /// The branch's unobserved stretches: hours not walked and hours its inputs were not.
    blind: Vec<(i32, i32)>,
    /// Every refused row by reason, the appointments without an idea token, and the
    /// additions first observed after an unobserved stretch.
    refused: BTreeMap<Refusal, BTreeSet<Row>>,
    tokenless: u64,
    gap_additions: u64,
    definitions: Option<Arc<Definitions>>,
    /// The scalars changed since the last cut.
    stale: bool,
    last: Option<i32>,
    /// The first hour since the last cut.
    first: Option<i32>,
}

/// One event of `kind` at `at` on `key`, its fields `value` as JSON.
fn event(
    writer: &mut Writer<'_>,
    key: KeyHandle,
    at: i32,
    kind: u64,
    value: &impl Serialize,
) -> io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    let text = writer.blob(&mut &bytes[..], bytes.len() as u64)?;
    let fields = writer.cells([Cell::Text(text)])?;
    let sequence = writer.sequence()?;
    writer.event(Event {
        key,
        native_hour: at,
        read_hour: Some(at),
        sequence,
        kind,
        fields,
    })
}

impl Default for Emit {
    fn default() -> Self {
        Self::new()
    }
}

impl Emit {
    pub fn new() -> Self {
        Self {
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            countries: Vec::new(),
            family: None,
            tokens: HashMap::new(),
            roles: HashMap::new(),
            listed_roles: Vec::new(),
            known: false,
            known_last: None,
            blind: Vec::new(),
            refused: BTreeMap::new(),
            tokenless: 0,
            gap_additions: 0,
            definitions: None,
            stale: true,
            last: None,
            first: None,
        }
    }

    fn key(&mut self, writer: &mut Writer<'_>, place: Place) -> io::Result<KeyHandle> {
        self.actors
            .key(writer, place, (RunMode::Exact, 0), Some(DOMAIN))
    }

    fn family(&mut self, writer: &mut Writer<'_>) -> io::Result<[KeyHandle; 3]> {
        if let Some(keys) = self.family {
            return Ok(keys);
        }
        let mut keys = [0; 3];
        for (key, measure) in keys.iter_mut().zip([KNOWN, ADDITIONS, DEFINITIONS]) {
            *key = self.key(
                writer,
                Place {
                    actor: FAMILY,
                    measure,
                    dimension: None,
                },
            )?;
        }
        self.family = Some(keys);
        Ok(keys)
    }

    /// The tag actor named `tag`, with its keys.
    fn country(&mut self, writer: &mut Writer<'_>, tag: &str, hour: i32) -> io::Result<usize> {
        if let Some(actor) = self.actors.find(tag.as_bytes()) {
            return Ok(actor as usize);
        }
        let actor = self.actors.actor(writer, tag.as_bytes(), hour)?;
        let index = actor as usize;
        if index >= self.countries.len() {
            self.countries.resize_with(index + 1, Country::default);
        }
        let mut keys = [0; 4];
        for (key, measure) in keys
            .iter_mut()
            .zip([UNRESOLVED, IDENTITY, TEMPLATES, WARNINGS])
        {
            *key = self.key(
                writer,
                Place {
                    actor,
                    measure,
                    dimension: None,
                },
            )?;
        }
        self.countries[index].keys = keys;
        Ok(index)
    }

    /// `actor`'s HOLD key of `token`.
    fn hold(&mut self, writer: &mut Writer<'_>, actor: usize, token: &Arc<str>) -> io::Result<KeyHandle> {
        let id = match self.tokens.get(token) {
            Some(&id) => id,
            None => {
                let id = self.tokens.len() as u64;
                if TOKENS + id >= ROLES {
                    return Err(dashboard_core::invalid("Idea token ids are exhausted"));
                }
                let text = family::cell(writer, &Scalar::Text(token.as_bytes().into()))?;
                writer.catalogue(TOKENS + id, text)?;
                self.tokens.insert(Arc::clone(token), id);
                id
            }
        };
        if let Some(&key) = self.countries[actor].holds.get(&id) {
            return Ok(key);
        }
        let key = self.key(
            writer,
            Place {
                actor: actor as u64,
                measure: HOLD,
                dimension: Some(id),
            },
        )?;
        self.countries[actor].holds.insert(id, key);
        Ok(key)
    }

    /// The value a holding by `role` runs with: its appointment's id, 0 for none.
    fn role(&mut self, writer: &mut Writer<'_>, role: &Option<Arc<Role>>) -> io::Result<i128> {
        let Some(role) = role else {
            return Ok(0);
        };
        if let Some(&id) = self.roles.get(role) {
            return Ok(i128::from(id));
        }
        let id = self.listed_roles.len() as u64 + 1;
        if ROLES + id >= family::ACTORS {
            return Err(dashboard_core::invalid("Idea appointment ids are exhausted"));
        }
        let cell = family::cell(
            writer,
            &Scalar::List(vec![
                Scalar::Text(role.seat.as_bytes().into()),
                Scalar::Unsigned(u64::from(role.id)),
                Scalar::Text(role.token.as_bytes().into()),
                Scalar::Text(role.name.as_bytes().into()),
            ]),
        )?;
        writer.catalogue(ROLES + id, cell)?;
        self.roles.insert(Arc::clone(role), id);
        self.listed_roles.push(Arc::clone(role));
        Ok(i128::from(id))
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: facts::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("ideas.emit", None);
        let at = hour.hour;
        let adjacent = match self.last {
            Some(last) if last.checked_add(1) == Some(at) => true,
            Some(last) if at <= last => {
                return Err(dashboard_core::invalid(
                    "An ideas hour does not follow the hour before",
                ));
            }
            // An hour not walked ends every holding and stretch.
            Some(last) => {
                self.close(writer, last)?;
                mark(&mut self.blind, last + 1, at - 1);
                false
            }
            None => false,
        };
        self.last = Some(at);
        self.first.get_or_insert(at);
        // What the hour before left, before this hour's facts.
        let before = self.known_last;
        let Some(facts) = hour.facts else {
            if self.known && adjacent {
                self.known_last = Some(at);
            } else {
                // No facts follow an hour not walked: the projection reads it anew.
                mark(&mut self.blind, at, at);
                self.known = false;
            }
            return Ok(());
        };
        let Facts {
            known,
            roster,
            reads,
            refused,
            definitions,
        } = *facts;
        if let Some(definitions) = definitions {
            self.define(writer, at, definitions)?;
        }
        let [known_key, ..] = self.family(writer)?;
        if !known {
            if self.known {
                self.unknown(writer, at, before)?;
            }
            self.known = false;
            mark(&mut self.blind, at, at);
            return self.carry.value(writer, known_key, at, None, false).map(drop);
        }
        self.known = true;
        self.known_last = Some(at);
        self.carry
            .value(writer, known_key, at, Some(1), false)?;
        for (why, row) in refused {
            self.stale |= self.refused.entry(why).or_default().insert(row);
        }
        if let Some(roster) = roster {
            self.roster(writer, at, before, &roster)?;
        }
        for read in reads {
            self.observe(writer, at, adjacent, before, read)?;
        }
        Ok(())
    }

    /// The definitions recorded at `at`, published where they changed.
    fn define(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        definitions: Arc<Definitions>,
    ) -> io::Result<()> {
        if self
            .definitions
            .as_deref()
            .is_some_and(|held| *held == *definitions)
        {
            return Ok(());
        }
        let [_, _, key] = self.family(writer)?;
        let listed: Vec<(&String, Definition)> = definitions
            .iter()
            .map(|(name, row)| (name, Definition::of(row)))
            .collect();
        event(writer, key, at, kind::DEFINITIONS, &listed)?;
        self.definitions = Some(definitions);
        Ok(())
    }

    /// Ends `actor`'s holdings before `at`.
    fn end(&mut self, writer: &mut Writer<'_>, actor: usize, at: i32) -> io::Result<()> {
        let country = &mut self.countries[actor];
        if !std::mem::take(&mut country.active) {
            return Ok(());
        }
        let open = std::mem::take(&mut country.open);
        for (token, role) in open {
            let key = self.hold(writer, actor, &token)?;
            self.carry.value(writer, key, at, None, false)?;
            self.countries[actor]
                .ended
                .insert(token, (at - 1, holder(&role)));
        }
        Ok(())
    }

    /// Ends `actor`'s unresolved stretch before `at`.
    fn resolve(&mut self, writer: &mut Writer<'_>, actor: usize, at: i32) -> io::Result<()> {
        let country = &mut self.countries[actor];
        let Some(first) = country.unresolved.take() else {
            return Ok(());
        };
        mark(&mut country.stretches, first, at - 1);
        let key = country.keys[0];
        self.carry.value(writer, key, at, None, false).map(drop)
    }

    /// Stops dating `actor`'s waiting templates: they keep `before`, the last hour the inputs
    /// were complete.
    fn undate(&mut self, writer: &mut Writer<'_>, actor: usize, at: i32, before: Option<i32>) -> io::Result<()> {
        let country = &mut self.countries[actor];
        if country.dated.is_empty() {
            return Ok(());
        }
        for id in std::mem::take(&mut country.dated) {
            if let Some(Kept::Missing { hour, .. }) = country.templates.get_mut(&id) {
                *hour = before;
            }
        }
        self.templates(writer, actor, at)
    }

    fn templates(&mut self, writer: &mut Writer<'_>, actor: usize, at: i32) -> io::Result<()> {
        let country = &self.countries[actor];
        let listed: Vec<(u32, &Kept)> = country.templates.iter().map(|(&id, kept)| (id, kept)).collect();
        event(writer, country.keys[2], at, kind::TEMPLATES, &listed)
    }

    /// The inputs stop being complete at `at`: every holding and stretch ends before it.
    fn unknown(&mut self, writer: &mut Writer<'_>, at: i32, before: Option<i32>) -> io::Result<()> {
        for actor in 0..self.countries.len() {
            if !self.countries[actor].listed {
                continue;
            }
            self.end(writer, actor, at)?;
            self.resolve(writer, actor, at)?;
            self.undate(writer, actor, at, before)?;
        }
        Ok(())
    }

    /// The countries read whole at `at`: tags gone or unresolved end their holdings.
    fn roster(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        before: Option<i32>,
        roster: &[(Arc<str>, Option<(u32, u32)>)],
    ) -> io::Result<()> {
        let mut listed = vec![false; self.countries.len()];
        for (tag, holder) in roster {
            let actor = self.country(writer, tag, at)?;
            if actor >= listed.len() {
                listed.resize(actor + 1, false);
            }
            listed[actor] = true;
            self.countries[actor].listed = true;
            if holder.is_none() {
                self.end(writer, actor, at)?;
                self.undate(writer, actor, at, before)?;
                let country = &mut self.countries[actor];
                if country.unresolved.is_none() {
                    country.unresolved = Some(at);
                    let key = country.keys[0];
                    self.carry.value(writer, key, at, Some(1), false)?;
                }
            }
        }
        for actor in 0..self.countries.len() {
            if listed.get(actor).copied().unwrap_or(false) || !self.countries[actor].listed {
                continue;
            }
            self.countries[actor].listed = false;
            self.end(writer, actor, at)?;
            self.resolve(writer, actor, at)?;
            self.undate(writer, actor, at, before)?;
        }
        Ok(())
    }

    /// Observes `read` at `at`.
    fn observe(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        adjacent: bool,
        before: Option<i32>,
        read: Read,
    ) -> io::Result<()> {
        let Read {
            tag,
            identity,
            held,
            refused,
            tokenless,
            templates,
            naming,
        } = read;
        let actor = self.country(writer, &tag, at)?;
        self.countries[actor].listed = true;
        let country = &self.countries[actor];
        let continues = country.active && adjacent && country.identity == Some(identity);
        let resumed = !continues && country.identity == Some(identity);
        self.countries[actor].identity = Some(identity);
        self.resolve(writer, actor, at)?;
        if !continues {
            self.end(writer, actor, at)?;
            let key = self.countries[actor].keys[1];
            event(writer, key, at, kind::IDENTITY, &naming)?;
        }
        // Templates waiting on their row are dated anew where the read still waits.
        let country = &mut self.countries[actor];
        let was = country.templates.clone();
        for id in std::mem::take(&mut country.dated) {
            if let Some(Kept::Missing { hour, .. }) = country.templates.get_mut(&id) {
                *hour = before;
            }
        }
        for (id, template) in templates {
            if matches!(country.templates.get(&id), Some(Kept::Built(_))) {
                continue;
            }
            let kept = match template {
                Template::Built(Ok(built)) => Kept::Built(built),
                Template::Built(Err(reason)) => Kept::Refused(reason),
                Template::Missing { complete } => {
                    country.dated.push(id);
                    Kept::Missing {
                        complete,
                        hour: None,
                    }
                }
            };
            country.templates.insert(id, kept);
        }
        if country.templates != was {
            self.templates(writer, actor, at)?;
        }
        let mut open = if continues {
            std::mem::take(&mut self.countries[actor].open)
        } else {
            BTreeMap::new()
        };
        let blind = if resumed {
            let country = &self.countries[actor];
            blind_before(at, [country.stretches.as_slice(), self.blind.as_slice()])
        } else {
            None
        };
        let mut next = BTreeMap::new();
        let mut additions = Vec::new();
        for holding in held {
            let previous = open.remove(&holding.token);
            if previous.as_ref() == Some(&holding.role) {
                next.insert(holding.token, holding.role);
                continue;
            }
            let now = holder(&holding.role);
            let same_holding = previous.as_ref().is_some_and(|role| holder(role) == now);
            if let Some(role) = &previous {
                self.countries[actor]
                    .ended
                    .insert(Arc::clone(&holding.token), (at - 1, holder(role)));
            }
            if let Some(first) = blind
                && self.countries[actor]
                    .ended
                    .get(&holding.token)
                    .is_none_or(|(last, held)| *held != now || last.checked_add(1) != Some(first))
            {
                self.gap_additions += 1;
                self.stale = true;
            }
            let key = self.hold(writer, actor, &holding.token)?;
            let value = self.role(writer, &holding.role)?;
            self.carry.value(writer, key, at, Some(value), true)?;
            if continues && !same_holding {
                additions.push(Addition {
                    tag: tag.to_string(),
                    span: IdeaSpan {
                        token: holding.token.to_string(),
                        first_hour: at,
                        last_hour: at,
                        slot: holding.role.as_ref().map(|role| role.seat.clone()),
                        character: holding.role.as_ref().map(|role| IdeaCharacter {
                            id: role.id,
                            token: role.token.clone(),
                            name: role.name.clone(),
                        }),
                    },
                    naming: Some(naming.clone()),
                });
            }
            next.insert(holding.token, holding.role);
        }
        for (token, role) in open {
            let key = self.hold(writer, actor, &token)?;
            self.carry.value(writer, key, at, None, false)?;
            self.countries[actor]
                .ended
                .insert(token, (at - 1, holder(&role)));
        }
        let country = &mut self.countries[actor];
        country.open = next;
        country.active = true;
        for (why, row) in refused {
            country
                .refused
                .entry(why)
                .or_default()
                .insert(row.clone());
            self.stale |= self.refused.entry(why).or_default().insert(row);
        }
        for appointment in tokenless {
            if country.tokenless.insert(appointment) {
                self.tokenless += 1;
                self.stale = true;
            }
        }
        let warnings = country.warnings();
        if warnings != country.warnings {
            let key = country.keys[3];
            event(writer, key, at, kind::WARNINGS, &warnings)?;
            self.countries[actor].warnings = warnings;
        }
        if !additions.is_empty() {
            let [_, key, _] = self.family(writer)?;
            for addition in &additions {
                event(writer, key, at, kind::ADDITION, addition)?;
            }
        }
        Ok(())
    }

    /// Ends every holding and stretch after `last`, the hour before one not walked.
    fn close(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.close(writer, last)?;
        for country in &mut self.countries {
            if std::mem::take(&mut country.active) {
                for (token, role) in std::mem::take(&mut country.open) {
                    country.ended.insert(token, (last, holder(&role)));
                }
            }
            if let Some(first) = country.unresolved.take() {
                mark(&mut country.stretches, first, last);
            }
        }
        self.known = false;
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, the hours since the last cut as known, and the scalars that changed.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)?;
        if let Some(first) = self.first.take() {
            writer.domain(
                DOMAIN,
                Evidence {
                    span: Span::new(first, last)?,
                    known: Knownness::Known,
                    supplying_read: None,
                    holder: None,
                    identity_first: None,
                },
            )?;
        }
        if self.stale {
            let mut refusals: Vec<(&'static str, u64)> = self
                .refused
                .iter()
                .map(|(why, rows)| (why.text(), rows.len() as u64))
                .collect();
            refusals.sort_unstable();
            if self.tokenless > 0 {
                refusals.push((refusal::TOKENLESS, self.tokenless));
            }
            let scalars = [
                (
                    REFUSALS,
                    Scalar::List(
                        refusals
                            .into_iter()
                            .map(|(reason, count)| {
                                Scalar::List(vec![
                                    Scalar::Text(reason.as_bytes().into()),
                                    Scalar::Unsigned(count),
                                ])
                            })
                            .collect(),
                    ),
                ),
                (
                    COUNTS,
                    Scalar::List(vec![Scalar::Unsigned(self.gap_additions)]),
                ),
            ];
            for (id, scalar) in scalars {
                let cell = family::cell(writer, &scalar)?;
                writer.catalogue(id, cell)?;
            }
            self.stale = false;
        }
        Ok(())
    }

    /// What the emit goes on with after a cut. A restart goes on at a segment's first hour,
    /// whose projection reads every country anew.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let saved = Saved {
            last: self.last,
            known: self.known,
            known_last: self.known_last,
            blind: self.blind.clone(),
            countries: self
                .countries
                .iter()
                .enumerate()
                .map(|(actor, country)| SavedCountry {
                    actor: actor as u64,
                    listed: country.listed,
                    identity: country.identity,
                    active: country.active,
                    open: country
                        .open
                        .iter()
                        .map(|(token, role)| {
                            (
                                token.to_string(),
                                role.as_ref().map_or(0, |role| self.roles[role]),
                            )
                        })
                        .collect(),
                    ended: country
                        .ended
                        .iter()
                        .map(|(token, (last, held))| (token.to_string(), *last, held.clone()))
                        .collect(),
                    unresolved: country.unresolved,
                    stretches: country.stretches.clone(),
                    templates: country
                        .templates
                        .iter()
                        .map(|(&id, kept)| (id, kept.clone()))
                        .collect(),
                    dated: country.dated.clone(),
                    refused: saved_rows(&country.refused),
                    tokenless: country.tokenless.iter().cloned().collect(),
                    warnings: country.warnings.clone(),
                })
                .collect(),
            refused: saved_rows(&self.refused),
            tokenless: self.tokenless,
            gap_additions: self.gap_additions,
            definitions: self.definitions.as_deref().map(|definitions| {
                definitions
                    .iter()
                    .map(|(name, row)| (name.clone(), Definition::of(row)))
                    .collect()
            }),
        };
        let json = serde_json::to_vec(&saved)?;
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(&json);
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut emit = Self::new();
        emit.stale = false;
        emit.actors = Actors::resume(SCHEMA, directory)?;
        let text = |scalar: &Scalar| {
            scalar
                .text()
                .and_then(|text| std::str::from_utf8(text).ok())
                .map(str::to_owned)
                .ok_or_else(invalid)
        };
        let mut id = 0;
        while let Some(scalar) = directory.scalar(TOKENS + id) {
            emit.tokens.insert(Arc::from(text(scalar)?), id);
            id += 1;
        }
        let mut id = 1;
        while let Some(scalar) = directory.scalar(ROLES + id) {
            let [seat, character, token, name] = scalar.list() else {
                return Err(invalid());
            };
            let role = Arc::new(Role {
                seat: text(seat)?,
                id: character
                    .integer()
                    .and_then(|id| u32::try_from(id).ok())
                    .ok_or_else(invalid)?,
                token: text(token)?,
                name: text(name)?,
            });
            emit.roles.insert(Arc::clone(&role), id);
            emit.listed_roles.push(role);
            id += 1;
        }
        emit.countries = (0..directory.actors.len())
            .map(|_| Country::default())
            .collect();
        for (actor, country) in emit.countries.iter_mut().enumerate() {
            let actor = actor as u64;
            for (key, measure) in country
                .keys
                .iter_mut()
                .zip([UNRESOLVED, IDENTITY, TEMPLATES, WARNINGS])
            {
                *key = directory.key(actor, measure, None).ok_or_else(invalid)?;
            }
            country.holds = directory.dimensions(actor, HOLD).iter().copied().collect();
        }
        let mut family = [0; 3];
        for (key, measure) in family.iter_mut().zip([KNOWN, ADDITIONS, DEFINITIONS]) {
            *key = directory.key(FAMILY, measure, None).ok_or_else(invalid)?;
        }
        emit.family = Some(family);
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        emit.last = saved.last;
        emit.known = saved.known;
        emit.known_last = saved.known_last;
        emit.blind = saved.blind;
        emit.refused = loaded_rows(saved.refused);
        emit.tokenless = saved.tokenless;
        emit.gap_additions = saved.gap_additions;
        emit.definitions = saved.definitions.map(|definitions| {
            Arc::new(
                definitions
                    .into_iter()
                    .map(|(name, definition)| (name, definition.row()))
                    .collect(),
            )
        });
        let token = |token: &str| {
            emit.tokens
                .get_key_value(token)
                .map(|(token, _)| Arc::clone(token))
                .ok_or_else(invalid)
        };
        let mut countries = std::mem::take(&mut emit.countries);
        for saved in saved.countries {
            let country = countries
                .get_mut(saved.actor as usize)
                .ok_or_else(invalid)?;
            country.listed = saved.listed;
            country.identity = saved.identity;
            country.active = saved.active;
            for (held, role) in saved.open {
                let role = match role {
                    0 => None,
                    id => Some(Arc::clone(
                        emit.listed_roles
                            .get(id as usize - 1)
                            .ok_or_else(invalid)?,
                    )),
                };
                country.open.insert(token(&held)?, role);
            }
            for (held, last, holder) in saved.ended {
                country.ended.insert(token(&held)?, (last, holder));
            }
            country.unresolved = saved.unresolved;
            country.stretches = saved.stretches;
            country.templates = saved.templates.into_iter().collect();
            country.dated = saved.dated;
            country.refused = loaded_rows(saved.refused);
            country.tokenless = saved.tokenless.into_iter().collect();
            country.warnings = saved.warnings;
        }
        emit.countries = countries;
        Ok(emit)
    }
}

fn saved_rows(rows: &BTreeMap<Refusal, BTreeSet<Row>>) -> Vec<(Refusal, Vec<Row>)> {
    rows.iter()
        .map(|(&why, rows)| (why, rows.iter().cloned().collect()))
        .collect()
}

fn loaded_rows(rows: Vec<(Refusal, Vec<Row>)>) -> BTreeMap<Refusal, BTreeSet<Row>> {
    rows.into_iter()
        .map(|(why, rows)| (why, rows.into_iter().collect()))
        .collect()
}

#[derive(Serialize, Deserialize)]
struct SavedCountry {
    actor: u64,
    listed: bool,
    identity: Option<(u32, u32)>,
    active: bool,
    open: Vec<(String, u64)>,
    ended: Vec<(String, i32, Holder)>,
    unresolved: Option<i32>,
    stretches: Vec<(i32, i32)>,
    templates: Vec<(u32, Kept)>,
    dated: Vec<u32>,
    refused: Vec<(Refusal, Vec<Row>)>,
    tokenless: Vec<(u32, Box<str>)>,
    warnings: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    known: bool,
    known_last: Option<i32>,
    blind: Vec<(i32, i32)>,
    countries: Vec<SavedCountry>,
    refused: Vec<(Refusal, Vec<Row>)>,
    tokenless: u64,
    gap_additions: u64,
    definitions: Option<Vec<(String, Definition)>>,
}
