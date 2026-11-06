//! The manpower family's hours from a columnar walk ([`replay::segments`]): every country's
//! manpower operands ([`Operands`]) as `country_people::read` reads them from a frame, from
//! what the walk's tables hold. Each table the operands read is kept as the fields they read,
//! from the rows each hour wrote; the army, navy and air totals are summed again only at an
//! hour that changed what they sum. Only the countries whose operands changed since this
//! segment last sent them leave the projection. A segment's first hour, an hour after a skipped
//! one and an hour that changed which tables hold every row read everything whole.

use dashboard_base::products::country_people::{
    Operands, RECRUITMENT_MODIFIERS, StateOperands,
};
use dashboard_world::{Country as Member, Needs, Roster, Tag, Tagged, World};
use replay::{
    columns,
    segments::{self, Operator, Sink, State},
};
use replay_data::rows::{
    AirWingRow, CountryCounterInputRow, CountryStabilityInputRow, DivisionLandInputsRow,
    DivisionTemplateIconRow, FleetRow, LexicalTokenRow, ModifierDefinitionRow, Recorded,
    RowCodec, SentVolunteerRow, ShipRow, SimulationFeatureRow, StateCoreRow, StateResourceInputRow,
    StateRow, TableId, TaskForceRow,
};
use replay_derive::modifier_registry::{Effective, Registry};
use super::emit::{COUNTRY, STATE, Values};
use dashboard_series::hash::{FastMap, Words};
use std::{
    collections::HashSet,
    hash::{BuildHasherDefault, Hash},
    io,
    mem::size_of,
    sync::Arc,
};

/// The tables the operands read, besides those the world reads for its countries and divisions.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Divisions,
    TableId::DivisionStatus,
    TableId::DivisionLandInputs,
    TableId::DivisionTemplateIcons,
    TableId::SentVolunteers,
    TableId::CountryCounterInputs,
    TableId::CountryStabilityInputs,
    TableId::SliceReads,
    TableId::States,
    TableId::StateResourceInputs,
    TableId::StateCores,
    TableId::Ships,
    TableId::TaskForces,
    TableId::Fleets,
    TableId::AirWings,
    TableId::ModifierDefinitions,
    TableId::LexicalTokens,
    TableId::SimulationFeatures,
];

/// Where the tables whose completeness the parts read are in [`TABLES`].
mod at {
    pub const LAND: usize = 4;
    pub const TEMPLATES: usize = 5;
    pub const VOLUNTEERS: usize = 6;
    pub const STABILITY: usize = 8;
    pub const READS: usize = 9;
    pub const STATES: usize = 10;
    pub const RESOURCES: usize = 11;
    pub const CORES: usize = 12;
    pub const SHIPS: usize = 13;
    pub const TASKS: usize = 14;
    pub const FLEETS: usize = 15;
    pub const WINGS: usize = 16;
}

/// What a projection reads of its walk's world: the countries and the divisions' facts.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        divisions: true,
        ..Needs::default()
    }
}

/// One country's sent keys at a projected hour.
#[derive(Clone, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Country {
    /// Its tag's index in the projection ([`Hour::named`]).
    pub index: u32,
    /// Its canonical slot and id.
    pub holder: (u32, u32),
    /// Whether `values` is every key it holds (its first send in the segment, or its holder's),
    /// else the keys whose values changed since it was last sent.
    pub whole: bool,
    /// Each key's value, none where it holds none now.
    pub values: Values,
    /// Its controlled states in their recorded order, repeats kept, none where unknown: in a
    /// whole send, and where they changed since it was last sent.
    pub roster: Option<Option<Box<[u32]>>>,
}

/// One projected hour.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Tags the projection numbers from this hour on, by index.
    pub named: Vec<(u32, Arc<str>)>,
    /// The tags with a current holder, ascending, when they may have changed.
    pub valid: Option<Vec<u32>>,
    /// The countries whose keys changed since this segment last sent them.
    pub countries: Vec<Country>,
}

/// A country's recorded operands of its counters row: its controlled states, recruitment
/// modifiers and sent expeditionaries, each none where unread.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Counters {
    states: Option<Vec<u32>>,
    modifiers: Option<[Option<i64>; 4]>,
    expeditionaries: Option<Vec<u64>>,
}

/// A ship's task force, and its acquired and required manpower.
type Ship = (u64, Option<i32>, Option<i32>);
/// A wing's owner, exile country and acquired manpower.
type Wing = (u32, Option<i32>, Option<i32>);

fn scalar<T: Copy>(value: &Recorded<Option<T>>) -> Option<T> {
    value.require("people operand").ok().copied().flatten()
}

/// A States slot's id, available manpower, and locked manpower and population (none where
/// unrecorded), from its columns; from its row where they are not plain 32-bit words.
fn state_operands(state: &State, format: u16, slot: u32) -> io::Result<(u32, (i32, Option<i32>, Option<i32>))> {
    let held = columns::table(state, TableId::States)?;
    let fields = STATE_FIELDS.get_or_init(|| {
        ["manpower_available", "manpower_locked", "population"]
            .map(|name| columns::field(TableId::States, name).expect("a States field"))
    });
    let cell = |field: usize| {
        columns::plain::<i32>(held, field)
            .ok()
            .and_then(|values| values.get(slot as usize).copied())
    };
    let id = held
        .key(slot)
        .and_then(|key| u32::try_from(key).ok())
        .ok_or_else(|| io::Error::other("A manpower state's slot holds no row"))?;
    let recorded = |field: usize| held.recorded(field, slot);
    if recorded(fields[0])
        && let (Some(available), Some(locked), Some(population)) =
            (cell(fields[0]), cell(fields[1]), cell(fields[2]))
    {
        return Ok((
            id,
            (
                available,
                recorded(fields[1]).then_some(locked),
                recorded(fields[2]).then_some(population),
            ),
        ));
    }
    let row: StateRow = columns::row(state, TableId::States, slot, format)?;
    Ok((
        row.id,
        (
            row.manpower_available,
            row.manpower_locked.require("locked").ok().copied(),
            row.population.require("population").ok().copied(),
        ),
    ))
}

static STATE_FIELDS: std::sync::OnceLock<[usize; 3]> = std::sync::OnceLock::new();

/// Reads a slot's row and takes what `project` does of it.
fn rows<'s, R: RowCodec, K, V>(
    state: &'s State,
    table: TableId,
    format: u16,
    project: impl Fn(R) -> (K, V) + 's,
) -> impl Fn(u32) -> io::Result<(K, V)> + 's {
    move |slot| Ok(project(columns::row(state, table, slot, format)?))
}

/// One table kept by the field its readers find its rows by: each row's field, and each field's
/// value as the readers take it.
struct Keyed<K, V> {
    fields: FastMap<u128, K>,
    values: FastMap<K, V>,
}

impl<K, V> Default for Keyed<K, V> {
    fn default() -> Self {
        Self {
            fields: FastMap::default(),
            values: FastMap::default(),
        }
    }
}

impl<K: Copy + Eq + Hash, V: PartialEq> Keyed<K, V> {
    /// Follows `table` this hour, each row as `project` reads it from the columns of `mask`: all
    /// of it when `whole`, else the rows the hour inserted, deleted or wrote any of `mask` at;
    /// empty while it does not hold every row. Each field whose value changed is added to
    /// `touched`.
    fn follow(
        &mut self,
        state: &State,
        (table, mask): (TableId, u64),
        whole: bool,
        read: impl Fn(u32) -> io::Result<(K, V)>,
        touched: &mut Vec<K>,
    ) -> io::Result<()> {
        if !columns::complete(state, table) {
            touched.extend(self.values.keys().copied());
            self.fields.clear();
            self.values.clear();
            return Ok(());
        }
        let held = columns::table(state, table)?;
        let key = |slot: u32| {
            held.key(slot)
                .ok_or_else(|| io::Error::other("A manpower row's slot holds no row"))
        };
        if whole {
            touched.extend(self.values.keys().copied());
            self.fields.clear();
            self.values.clear();
            for slot in held.live().ones() {
                let (field, value) = read(slot)?;
                self.fields.insert(key(slot)?, field);
                self.values.insert(field, value);
                touched.push(field);
            }
            return Ok(());
        }
        let changes = columns::changes(state, table)?;
        for key in &changes.deleted_keys {
            if let Some(field) = self.fields.remove(key) {
                self.values.remove(&field);
                touched.push(field);
            }
        }
        let written = changes
            .changed
            .iter()
            .zip(&changes.masks)
            .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
            .filter(|&(_, &written)| written & mask != 0)
            .map(|(slot, _)| slot);
        for &slot in changes.inserted.iter().chain(written) {
            let (field, value) = read(slot)?;
            if let Some(old) = self.fields.insert(key(slot)?, field)
                && old != field
            {
                self.values.remove(&old);
                touched.push(old);
            }
            if self.values.get(&field) != Some(&value) {
                touched.push(field);
                self.values.insert(field, value);
            }
        }
        Ok(())
    }

    fn heap(&self, value: impl Fn(&V) -> usize) -> usize {
        self.fields.capacity() * (size_of::<(u128, K)>() + 1)
            + self.values.capacity() * (size_of::<(K, V)>() + 1)
            + self.values.values().map(value).sum::<usize>()
    }
}

/// The columns each kept table's rows are read from, a bit by column; every column of those
/// read whole.
struct Masks {
    division_land_inputs: u64,
    division_template_icons: u64,
    sent_volunteers: u64,
    country_counter_inputs: u64,
    country_stability_inputs: u64,
    states: u64,
    state_resource_inputs: u64,
    state_cores: u64,
    ships: u64,
    task_forces: u64,
    fleets: u64,
    air_wings: u64,
    modifier_definitions: u64,
    simulation_features: u64,
}

impl Masks {
    fn new() -> io::Result<Self> {
        let mask = |table: TableId, names: &[&str]| {
            names.iter().try_fold(0_u64, |mask, name| {
                Ok::<_, io::Error>(mask | 1 << columns::field(table, name)?)
            })
        };
        Ok(Self {
            division_land_inputs: mask(TableId::DivisionLandInputs, &["manpower"])?,
            division_template_icons: mask(TableId::DivisionTemplateIcons, &["manpower_exile"])?,
            sent_volunteers: mask(TableId::SentVolunteers, &["division"])?,
            country_counter_inputs: mask(TableId::CountryCounterInputs, &["controlled_states", "recruitment_modifiers", "sent_expeditionaries"])?,
            country_stability_inputs: mask(TableId::CountryStabilityInputs, &["recruitment_ratio"])?,
            states: mask(TableId::States, &["manpower_available", "manpower_locked", "population"])?,
            state_resource_inputs: mask(TableId::StateResourceInputs, &["local_manpower", "local_noncore_manpower", "recruitment_add", "recruitment_factor"])?,
            state_cores: mask(TableId::StateCores, &["cores"])?,
            ships: mask(TableId::Ships, &["task_force", "acquired_manpower", "required_manpower"])?,
            task_forces: mask(TableId::TaskForces, &["fleet"])?,
            fleets: mask(TableId::Fleets, &["owner"])?,
            air_wings: mask(TableId::AirWings, &["owner", "exile_country", "acquired_manpower"])?,
            modifier_definitions: u64::MAX,
            simulation_features: u64::MAX,
        })
    }
}

/// Each recruitment modifier's effective rule, as the hour's modifier definitions give it;
/// none where they do not ([`Registry`]).
type Rules = Option<[Option<Effective>; 4]>;

/// The fields a part's reads cover: what it read last.
#[derive(Default)]
struct Kept {
    land: Keyed<u64, Vec<(u32, i32)>>,
    templates: Keyed<u64, Option<bool>>,
    volunteers: Keyed<u128, (u32, u64)>,
    counters: Keyed<u32, Counters>,
    stability: Keyed<u32, Option<i64>>,
    states: Keyed<u32, (i32, Option<i32>, Option<i32>)>,
    resources: Keyed<u32, Option<[i64; 4]>>,
    cores: Keyed<u32, Vec<u32>>,
    ships: Keyed<u128, Ship>,
    tasks: Keyed<u64, u64>,
    fleets: Keyed<u64, u32>,
    wings: Keyed<u128, Wing>,
    definitions: Keyed<u128, ModifierDefinitionRow>,
    features: Keyed<u128, u32>,
}

impl Kept {
    fn heap(&self) -> usize {
        fn none<T>(_: &T) -> usize {
            0
        }
        self.land.heap(|entries: &Vec<(u32, i32)>| entries.capacity() * size_of::<(u32, i32)>())
            + self.templates.heap(none)
            + self.volunteers.heap(none)
            + self.counters.heap(|counters: &Counters| {
                counters.states.as_ref().map_or(0, |states| states.capacity() * 4)
                    + counters
                        .expeditionaries
                        .as_ref()
                        .map_or(0, |list| list.capacity() * 8)
            })
            + self.stability.heap(none)
            + self.states.heap(none)
            + self.resources.heap(none)
            + self.cores.heap(|cores: &Vec<u32>| cores.capacity() * 4)
            + self.ships.heap(none)
            + self.tasks.heap(none)
            + self.fleets.heap(none)
            + self.wings.heap(none)
            + self.definitions.heap(|row: &ModifierDefinitionRow| {
                row.name_key.capacity() + row.postfix_key.capacity() + row.argument.capacity()
            })
            + self.features.heap(none)
    }
}

/// What each hour's touched fields are, by table.
#[derive(Default)]
struct Touched {
    land: Vec<u64>,
    templates: Vec<u64>,
    volunteers: Vec<u128>,
    counters: Vec<u32>,
    stability: Vec<u32>,
    states: Vec<u32>,
    resources: Vec<u32>,
    cores: Vec<u32>,
    ships: Vec<u128>,
    tasks: Vec<u64>,
    fleets: Vec<u64>,
    wings: Vec<u128>,
    definitions: Vec<u128>,
    features: Vec<u128>,
}

/// A member country: its id and the raw references naming it, ascending.
struct MemberOf {
    id: u32,
    slot: u32,
    aliases: Vec<u32>,
}

/// A sum some of whose terms may be unknown: known while none is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Sum {
    total: i32,
    unknown: u32,
}

impl Sum {
    fn add(&mut self, term: Option<i32>) {
        match term {
            Some(value) => self.total = self.total.wrapping_add(value),
            None => self.unknown += 1,
        }
    }

    fn remove(&mut self, term: Option<i32>) {
        match term {
            Some(value) => self.total = self.total.wrapping_sub(value),
            None => self.unknown -= 1,
        }
    }

    fn value(self) -> Option<i32> {
        (self.unknown == 0).then_some(self.total)
    }
}

/// What a ship adds to its owner's navy and required crews: nothing where its owner has no
/// slot; every navy is unknown while a ship's task force or fleet is not held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShipTerm {
    Unresolved,
    Unowned,
    Owned(u32, Option<i32>, Option<i32>),
}

/// What a wing adds to its owner's ordinary crews and to its exile country's crews, by slot,
/// and whether its exile country is unread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WingTerm {
    ordinary: Option<(u32, Option<i32>)>,
    exiled: Option<(u32, Option<i32>)>,
    unread: bool,
}

/// One segment's projection.
pub struct Projector {
    format: u16,
    residual: usize,
    sample_hour: usize,
    /// Nothing is held: the next hour is read whole, as a segment's first.
    cold: bool,
    last: Option<i32>,
    complete: Vec<bool>,
    masks: Masks,
    kept: Kept,
    rules: Rules,
    members: Vec<MemberOf>,
    /// Each member's place in `members` by its canonical slot.
    by_member_slot: FastMap<u32, usize>,
    /// Each country raw's controlled states, and each state's controllers' raws.
    rosters: FastMap<u32, Vec<u32>>,
    controllers: FastMap<u32, Vec<u32>>,
    /// Each division's canonical roster slot and the term it adds to that slot's army, if any;
    /// each slot's sum of them.
    division_terms: FastMap<u64, (u32, Option<Option<i32>>)>,
    slot_armies: FastMap<u32, Sum>,
    /// Each ship's and wing's term, each slot's sums of them, and how many ships no owner
    /// resolves and wings read no exile country.
    ship_terms: FastMap<u128, ShipTerm>,
    slot_navies: FastMap<u32, (Sum, Sum)>,
    unresolved_ships: u32,
    /// Each ship's task force as its term was read, each task force's ships, and each task
    /// force's fleet and each fleet's task forces: whom a task force or fleet change reaches.
    ship_task: FastMap<u128, u64>,
    ships_of_task: FastMap<u64, Vec<u128>>,
    task_fleet: FastMap<u64, u64>,
    tasks_of_fleet: FastMap<u64, Vec<u64>>,
    wing_terms: FastMap<u128, WingTerm>,
    slot_crews: FastMap<u32, (Sum, Sum)>,
    unread_exiles: u32,
    /// The country raws whose expeditionaries or volunteers list each division.
    listers: FastMap<u64, Vec<u32>>,
    army: FastMap<u32, Option<i32>>,
    navy: FastMap<u32, (Option<i32>, Option<i32>)>,
    air: FastMap<u32, (Option<i32>, Option<i32>)>,
    numbers: FastMap<Tag, u32>,
    /// Each country's tag, slot and resolution, and each link's slot, as membership last read
    /// them: a rewrite that changes none of them changes no member.
    identities: FastMap<u32, (Tag, u32, bool)>,
    links: FastMap<u32, u32>,
    /// The tokens the kept modifier definitions name.
    rule_tokens: FastSet<u32>,
    /// Each member's holder, tag, keys and controlled states as last sent in this segment.
    sent: FastMap<u32, Sent>,
}

type FastSet<T> = HashSet<T, BuildHasherDefault<Words>>;

/// What a member was last sent as: its own keys' values, its roster, and each state's of it.
struct Sent {
    holder: (u32, u32),
    tag: Tag,
    country: [Option<i64>; 12],
    roster: Option<Vec<u32>>,
    states: FastMap<u32, [Option<i64>; 8]>,
}

/// What the hour may have changed of each member, by its place in the members: its own keys,
/// its roster (and with it every state's keys), or some of its states' keys.
struct Due {
    country: Vec<bool>,
    roster: Vec<bool>,
    states: Vec<(usize, u32)>,
    /// Every member's every listed state too: the hour read everything or moved the roster.
    every: bool,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds
/// is read again from the walker's tables, as at a segment's first hour.
const SAVED: &[u8] = b"manpower projector: cold";

/// The country of `id` and its tag, when it holds the roster's place for its tag: the tag's
/// first holder by id the links resolve, with a UTF-8 tag.
fn member(roster: &Roster, id: u32) -> Option<(&Member, &str)> {
    let country = roster.get(id)?;
    match roster.tagged(country.tag) {
        Tagged::Found(found) if found.id == id => {}
        _ => return None,
    }
    Some((country, std::str::from_utf8(roster.tag(country.tag)).ok()?))
}

/// Whether two raw country references name one country, as the frame reader compares them.
fn equal(roster: &Roster, first: u32, second: u32) -> bool {
    first == second
        || (first != 0
            && second != 0
            && roster
                .slot_of(first)
                .is_some_and(|slot| roster.slot_of(second) == Some(slot)))
}

/// The raw references naming `country`, ascending: those its rows can be keyed by.
fn aliases(roster: &Roster, country: &Member) -> Vec<u32> {
    let mut raw: Vec<u32> = roster.aliases(country.slot).to_vec();
    raw.push(country.id);
    raw.sort_unstable();
    raw.dedup();
    raw.retain(|&raw| equal(roster, raw, country.id));
    raw
}

/// The first of `aliases` (ascending, as rows are in key order) holding a value of `values`.
fn first<'v, V>(values: &'v FastMap<u32, V>, aliases: &[u32]) -> Option<(u32, &'v V)> {
    aliases
        .iter()
        .find_map(|&raw| values.get(&raw).map(|value| (raw, value)))
}

fn put<K: Eq + Hash, V>(map: &mut FastMap<K, Vec<V>>, key: K, value: V) {
    map.entry(key).or_default().push(value);
}

fn take<K: Eq + Hash + Copy, V: PartialEq>(map: &mut FastMap<K, Vec<V>>, key: K, value: &V) {
    if let Some(values) = map.get_mut(&key) {
        values.retain(|held| held != value);
        if values.is_empty() {
            map.remove(&key);
        }
    }
}

impl Projector {
    pub fn new(format: u16) -> io::Result<Self> {
        Ok(Self {
            format,
            residual: columns::field(TableId::Countries, "manpower_residual")?,
            sample_hour: columns::field(TableId::SliceReads, "sample_hour")?,
            cold: true,
            last: None,
            complete: Vec::new(),
            masks: Masks::new()?,
            kept: Kept::default(),
            rules: None,
            members: Vec::new(),
            by_member_slot: FastMap::default(),
            rosters: FastMap::default(),
            controllers: FastMap::default(),
            division_terms: FastMap::default(),
            slot_armies: FastMap::default(),
            ship_terms: FastMap::default(),
            slot_navies: FastMap::default(),
            unresolved_ships: 0,
            ship_task: FastMap::default(),
            ships_of_task: FastMap::default(),
            task_fleet: FastMap::default(),
            tasks_of_fleet: FastMap::default(),
            wing_terms: FastMap::default(),
            slot_crews: FastMap::default(),
            unread_exiles: 0,
            listers: FastMap::default(),
            army: FastMap::default(),
            navy: FastMap::default(),
            air: FastMap::default(),
            numbers: FastMap::default(),
            identities: FastMap::default(),
            links: FastMap::default(),
            rule_tokens: FastSet::default(),
            sent: FastMap::default(),
        })
    }

    fn number(&mut self, tag: Tag, text: &str, named: &mut Vec<(u32, Arc<str>)>) -> u32 {
        if let Some(&index) = self.numbers.get(&tag) {
            return index;
        }
        let index = self.numbers.len() as u32;
        self.numbers.insert(tag, index);
        named.push((index, Arc::from(text)));
        index
    }

    /// Follows every kept table, adding each changed field to `touched`.
    fn follow(&mut self, state: &State, whole: bool, touched: &mut Touched) -> io::Result<()> {
        let format = self.format;
        let masks = &self.masks;
        let kept = &mut self.kept;
        kept.land.follow(
            state,
            (TableId::DivisionLandInputs, masks.division_land_inputs),
            whole,
            rows(state, TableId::DivisionLandInputs, format, |row: DivisionLandInputsRow| {
                let entries = row
                    .manpower
                    .iter()
                    .map(|entry| (entry.country, entry.amount))
                    .collect();
                (row.division, entries)
            }),
            &mut touched.land,
        )?;
        kept.templates.follow(
            state,
            (TableId::DivisionTemplateIcons, masks.division_template_icons),
            whole,
            rows(state, TableId::DivisionTemplateIcons, format, |row: DivisionTemplateIconRow| {
                (
                    row.template,
                    row.manpower_exile
                        .require("template manpower exile")
                        .ok()
                        .copied(),
                )
            }),
            &mut touched.templates,
        )?;
        kept.volunteers.follow(
            state,
            (TableId::SentVolunteers, masks.sent_volunteers),
            whole,
            rows(state, TableId::SentVolunteers, format, |row: SentVolunteerRow| {
                (
                    u128::from(row.country) << 64 | u128::from(row.ordinal),
                    (row.country, row.division),
                )
            }),
            &mut touched.volunteers,
        )?;
        kept.counters.follow(
            state,
            (TableId::CountryCounterInputs, masks.country_counter_inputs),
            whole,
            rows(state, TableId::CountryCounterInputs, format, |row: CountryCounterInputRow| {
                (
                    row.country,
                    Counters {
                        states: row
                            .controlled_states
                            .require("controlled states")
                            .ok()
                            .cloned()
                            .flatten(),
                        modifiers: row
                            .recruitment_modifiers
                            .require("recruitment modifiers")
                            .ok()
                            .copied()
                            .flatten(),
                        expeditionaries: row
                            .sent_expeditionaries
                            .require("sent expeditionaries")
                            .ok()
                            .and_then(Option::as_ref)
                            .map(|list| list.iter().copied().collect()),
                    },
                )
            }),
            &mut touched.counters,
        )?;
        kept.stability.follow(
            state,
            (TableId::CountryStabilityInputs, masks.country_stability_inputs),
            whole,
            rows(state, TableId::CountryStabilityInputs, format, |row: CountryStabilityInputRow| (row.country, scalar(&row.recruitment_ratio))),
            &mut touched.stability,
        )?;
        kept.states.follow(
            state,
            (TableId::States, masks.states),
            whole,
            |slot| state_operands(state, format, slot),
            &mut touched.states,
        )?;
        kept.resources.follow(
            state,
            (TableId::StateResourceInputs, masks.state_resource_inputs),
            whole,
            rows(state, TableId::StateResourceInputs, format, |row: StateResourceInputRow| {
                let values = (|| {
                    Some([
                        scalar(&row.local_manpower)?,
                        scalar(&row.local_noncore_manpower)?,
                        scalar(&row.recruitment_add)?,
                        scalar(&row.recruitment_factor)?,
                    ])
                })();
                (row.state, values)
            }),
            &mut touched.resources,
        )?;
        kept.cores.follow(
            state,
            (TableId::StateCores, masks.state_cores),
            whole,
            rows(state, TableId::StateCores, format, |row: StateCoreRow| (row.state, row.cores)),
            &mut touched.cores,
        )?;
        kept.ships.follow(
            state,
            (TableId::Ships, masks.ships),
            whole,
            rows(state, TableId::Ships, format, |row: ShipRow| {
                (
                    u128::from(row.id),
                    (
                        row.task_force,
                        scalar(&row.acquired_manpower),
                        scalar(&row.required_manpower),
                    ),
                )
            }),
            &mut touched.ships,
        )?;
        kept.tasks.follow(
            state,
            (TableId::TaskForces, masks.task_forces),
            whole,
            rows(state, TableId::TaskForces, format, |row: TaskForceRow| (row.id, row.fleet)),
            &mut touched.tasks,
        )?;
        kept.fleets.follow(
            state,
            (TableId::Fleets, masks.fleets),
            whole,
            rows(state, TableId::Fleets, format, |row: FleetRow| (row.id, row.owner)),
            &mut touched.fleets,
        )?;
        kept.wings.follow(
            state,
            (TableId::AirWings, masks.air_wings),
            whole,
            rows(state, TableId::AirWings, format, |row: AirWingRow| {
                (
                    u128::from(row.id),
                    (
                        row.owner,
                        scalar(&row.exile_country),
                        scalar(&row.acquired_manpower),
                    ),
                )
            }),
            &mut touched.wings,
        )?;
        kept.definitions.follow(
            state,
            (TableId::ModifierDefinitions, masks.modifier_definitions),
            whole,
            rows(state, TableId::ModifierDefinitions, format, |row: ModifierDefinitionRow| (u128::from(row.index), row)),
            &mut touched.definitions,
        )?;
        kept.features.follow(
            state,
            (TableId::SimulationFeatures, masks.simulation_features),
            whole,
            rows(state, TableId::SimulationFeatures, format, |row: SimulationFeatureRow| (0, row.dlc_mask)),
            &mut touched.features,
        )?;
        Ok(())
    }

    /// Whether the hour wrote, added or removed a token the kept definitions name.
    fn rule_tokens_written(&self, state: &State) -> io::Result<bool> {
        let held = columns::table(state, TableId::LexicalTokens)?;
        let changes = columns::changes(state, TableId::LexicalTokens)?;
        let named = |key: u128| u32::try_from(key).is_ok_and(|token| self.rule_tokens.contains(&token));
        Ok(changes
            .changed
            .iter()
            .chain(&changes.inserted)
            .chain(&changes.fill_slots)
            .filter_map(|&slot| held.key(slot))
            .chain(changes.deleted_keys.iter().copied())
            .any(named))
    }

    /// The recruitment modifiers' rules the kept definitions give, as `Registry::from_frame`
    /// reads them: none where ModifierDefinitions or LexicalTokens does not hold every row.
    fn rules(&self, state: &State) -> Rules {
        if !columns::complete(state, TableId::ModifierDefinitions)
            || !columns::complete(state, TableId::LexicalTokens)
        {
            return None;
        }
        let mut definitions: Vec<ModifierDefinitionRow> =
            self.kept.definitions.values.values().cloned().collect();
        definitions.sort_unstable_by_key(|row| row.index);
        // Only the definitions' own tokens are joined to them.
        let held = columns::table(state, TableId::LexicalTokens).ok()?;
        let mut tokens: Vec<LexicalTokenRow> = definitions
            .iter()
            .filter_map(|definition| {
                let slot = held.slot(u128::from(definition.token))?;
                columns::row(state, TableId::LexicalTokens, slot, self.format).ok()
            })
            .collect();
        tokens.sort_unstable_by_key(|row| row.token);
        tokens.dedup_by_key(|row| row.token);
        let dlc_mask = columns::complete(state, TableId::SimulationFeatures)
            .then(|| self.kept.features.values.get(&0).copied())
            .flatten();
        let registry = Registry::new(&definitions, tokens.as_slice(), dlc_mask).ok()?;
        Some(RECRUITMENT_MODIFIERS.map(|name| {
            registry
                .named(name)
                .ok()
                .and_then(|definition| registry.rule(definition).ok())
        }))
    }

    /// The members whose operands the hour's touched fields may change.
    fn due(&self, roster: &Roster, touched: &Touched, every: bool, rules: bool) -> Due {
        let members = self.members.len();
        let mut due = Due {
            country: vec![every || rules; members],
            roster: vec![every; members],
            states: Vec::new(),
            every,
        };
        let at = |raw: u32| {
            roster
                .slot_of(raw)
                .and_then(|slot| self.by_member_slot.get(&slot))
                .copied()
        };
        for &raw in &touched.counters {
            if let Some(at) = at(raw) {
                due.country[at] = true;
                due.roster[at] = true;
            }
        }
        for &raw in &touched.stability {
            if let Some(at) = at(raw) {
                due.country[at] = true;
            }
        }
        if !every {
            for &state in touched.states.iter().chain(&touched.resources).chain(&touched.cores) {
                for &raw in self.controllers.get(&state).into_iter().flatten() {
                    if let Some(at) = at(raw) {
                        due.states.push((at, state));
                    }
                }
            }
        }
        due
    }

    /// Keeps each country's controlled states and each state's controllers as the counters give
    /// them now.
    fn index_rosters(&mut self, touched: &[u32], whole: bool) {
        if whole {
            self.rosters.clear();
            self.controllers.clear();
        }
        let raws: Vec<u32> = if whole {
            self.kept.counters.values.keys().copied().collect()
        } else {
            touched.to_vec()
        };
        for raw in raws {
            if let Some(old) = self.rosters.remove(&raw) {
                for state in old {
                    take(&mut self.controllers, state, &raw);
                }
            }
            let Some(states) = self
                .kept
                .counters
                .values
                .get(&raw)
                .and_then(|counters| counters.states.clone())
            else {
                continue;
            };
            for &state in &states {
                put(&mut self.controllers, state, raw);
            }
            self.rosters.insert(raw, states);
        }
    }

    fn heap(&self) -> usize {
        let lists = |map: &FastMap<u64, Vec<u32>>| {
            map.capacity() * size_of::<(u64, Vec<u32>)>()
                + map.values().map(|raws| raws.capacity() * 4).sum::<usize>()
        };
        self.kept.heap()
            + self.members.capacity() * size_of::<MemberOf>()
            + self
                .members
                .iter()
                .map(|member| member.aliases.capacity() * 4)
                .sum::<usize>()
            + self.by_member_slot.capacity() * size_of::<(u32, usize)>()
            + self.rosters.capacity() * size_of::<(u32, Vec<u32>)>()
            + self.rosters.values().map(|states| states.capacity() * 4).sum::<usize>()
            + self.controllers.capacity() * size_of::<(u32, Vec<u32>)>()
            + self.controllers.values().map(|raws| raws.capacity() * 4).sum::<usize>()
            + self.division_terms.capacity() * (size_of::<(u64, (u32, Option<Option<i32>>))>() + 1)
            + self.slot_armies.capacity() * (size_of::<(u32, Sum)>() + 1)
            + self.ship_terms.capacity() * (size_of::<(u128, ShipTerm)>() + 1)
            + self.ship_task.capacity() * (size_of::<(u128, u64)>() + 1)
            + self.ships_of_task.capacity() * (size_of::<(u64, Vec<u128>)>() + 1)
            + self.ships_of_task.values().map(|ships| ships.capacity() * 16).sum::<usize>()
            + self.task_fleet.capacity() * (size_of::<(u64, u64)>() + 1)
            + self.tasks_of_fleet.capacity() * (size_of::<(u64, Vec<u64>)>() + 1)
            + self.tasks_of_fleet.values().map(|tasks| tasks.capacity() * 8).sum::<usize>()
            + self.slot_navies.capacity() * (size_of::<(u32, (Sum, Sum))>() + 1)
            + self.wing_terms.capacity() * (size_of::<(u128, WingTerm)>() + 1)
            + self.slot_crews.capacity() * (size_of::<(u32, (Sum, Sum))>() + 1)
            + lists(&self.listers)
            + self.army.capacity() * size_of::<(u32, Option<i32>)>()
            + self.navy.capacity() * size_of::<(u32, (Option<i32>, Option<i32>))>()
            + self.air.capacity() * size_of::<(u32, (Option<i32>, Option<i32>))>()
            + self.numbers.capacity() * size_of::<(Tag, u32)>()
            + self.identities.capacity() * (size_of::<(u32, (Tag, u32, bool))>() + 1)
            + self.links.capacity() * (size_of::<(u32, u32)>() + 1)
            + self.rule_tokens.capacity() * (size_of::<u32>() + 1)
            + self.complete.capacity()
            + self.sent.capacity() * (size_of::<(u32, Sent)>() + 1)
            + self
                .sent
                .values()
                .map(|sent| {
                    sent.states.capacity() * (size_of::<(u32, [Option<i64>; 8])>() + 1)
                        + sent.roster.as_ref().map_or(0, |states| states.capacity() * 4)
                })
                .sum::<usize>()
    }
}

impl Projector {
    /// What `division`, in roster slot `slot`, adds to that slot's army as the frame reader sums
    /// it: nothing where it does not count, unknown where its template or land row is.
    fn army_term(
        &self,
        roster: &Roster,
        division: &dashboard_world::Division,
        slot: u32,
    ) -> Option<Option<i32>> {
        let same = |raw: u32| raw != 0 && roster.slot_of(raw) == Some(slot);
        let owner = replay_data::military_aid::owner(&division.row());
        if !same(owner) || !equal(roster, division.roster_owner, division.logical_owner) {
            return None;
        }
        match self.kept.templates.values.get(&division.template).copied().flatten() {
            None => Some(None),
            Some(true) => None,
            Some(false) => Some(self.kept.land.values.get(&division.id).map(|entries| {
                entries
                    .iter()
                    .find(|(contributor, _)| same(*contributor))
                    .map_or(0, |&(_, amount)| amount)
            })),
        }
    }

    /// Division `id`'s term as `division` (none where gone) gives it now, its old one taken out
    /// of its slot's sum; the slots it leaves and joins are `slots`.
    fn set_division(
        &mut self,
        roster: &Roster,
        id: u64,
        division: Option<&dashboard_world::Division>,
        slots: &mut Vec<u32>,
    ) {
        if let Some((slot, term)) = self.division_terms.remove(&id) {
            if let Some(term) = term
                && let Some(sum) = self.slot_armies.get_mut(&slot)
            {
                sum.remove(term);
            }
            slots.push(slot);
        }
        let Some(division) = division else {
            return;
        };
        let Some(slot) = roster.slot_of(division.roster_owner) else {
            return;
        };
        let term = self.army_term(roster, division, slot);
        if let Some(term) = term {
            self.slot_armies.entry(slot).or_default().add(term);
        }
        self.division_terms.insert(id, (slot, term));
        slots.push(slot);
    }

    /// Keeps each division's term in its roster slot's army: anew when `rebuild`, else for the
    /// divisions an edit moved, reassigned or retemplated and those whose land row the hour
    /// touched. The slots whose sum may have changed are `slots`; the raws listing an edited
    /// division, `listed`.
    fn index_armies(
        &mut self,
        roster: &Roster,
        divisions: &dashboard_world::Kept<dashboard_world::Division>,
        (rebuild, land): (bool, &[u64]),
        slots: &mut Vec<u32>,
        listed: &mut Vec<u32>,
    ) {
        if rebuild || divisions.whole() {
            self.division_terms.clear();
            self.slot_armies.clear();
            for (id, division) in divisions.iter() {
                self.set_division(roster, id, Some(division), slots);
            }
            return;
        }
        let fields = |division: &dashboard_world::Division| {
            (
                division.roster_owner,
                division.logical_owner,
                division.expeditionary_owner,
                division.template,
            )
        };
        for (id, before, after) in divisions.edits() {
            if before.as_ref().map(fields) == after.as_ref().map(fields) {
                continue;
            }
            listed.extend(self.listers.get(id).into_iter().flatten().copied());
            self.set_division(roster, *id, after.as_ref(), slots);
        }
        for &id in land {
            self.set_division(roster, id, divisions.get(id), slots);
        }
    }

    /// `member`'s deployed army, as the frame reader sums it.
    fn army_of(
        &self,
        divisions: &dashboard_world::Kept<dashboard_world::Division>,
        member: &MemberOf,
    ) -> Option<i32> {
        if !divisions.holds() || !self.complete[at::LAND] || !self.complete[at::TEMPLATES] {
            return None;
        }
        // The raws naming the member, ascending: those equal to its id.
        let same = |raw: u32| member.aliases.binary_search(&raw).is_ok();
        let (land, templates) = (&self.kept.land.values, &self.kept.templates.values);
        let ordinary = |division: &dashboard_world::Division| {
            templates
                .get(&division.template)
                .copied()
                .flatten()
                .map(|exile| !exile)
        };
        let amount = |division: u64| {
            land.get(&division).map(|entries| {
                entries
                    .iter()
                    .find(|(contributor, _)| same(*contributor))
                    .map_or(0, |&(_, amount)| amount)
            })
        };
        let mut total = self
            .slot_armies
            .get(&member.slot)
            .copied()
            .unwrap_or_default()
            .value()?;
        let counters = first(&self.kept.counters.values, &member.aliases)?.1;
        total = total.wrapping_add(replay_data::military_aid::listed_manpower_with(
            counters.expeditionaries.as_ref()?.iter().copied(),
            |reference| divisions.get(reference),
            ordinary,
            amount,
        )?);
        if !self.complete[at::VOLUNTEERS] {
            return None;
        }
        total = total.wrapping_add(replay_data::military_aid::listed_manpower_with(
            self.kept
                .volunteers
                .values
                .values()
                .filter(|(sender, _)| same(*sender))
                .map(|&(_, division)| division),
            |reference| divisions.get(reference),
            ordinary,
            amount,
        )?);
        Some(total)
    }

    /// What ship `id` adds now: none where gone.
    fn ship_term(&self, roster: &Roster, id: u128) -> Option<ShipTerm> {
        let &(task, acquired, required) = self.kept.ships.values.get(&id)?;
        let (tasks, fleets) = (&self.kept.tasks.values, &self.kept.fleets.values);
        Some(match tasks.get(&task).and_then(|fleet| fleets.get(fleet)) {
            None => ShipTerm::Unresolved,
            Some(&owner) => match roster.slot_of(owner) {
                Some(slot) => ShipTerm::Owned(slot, acquired, required),
                None => ShipTerm::Unowned,
            },
        })
    }

    /// Ship `id`'s term as it is now, its old one taken out; the slots it touches are `slots`.
    fn set_ship(&mut self, roster: &Roster, id: u128, slots: &mut Vec<u32>) {
        let task = self.kept.ships.values.get(&id).map(|&(task, _, _)| task);
        if self.ship_task.get(&id).copied() != task {
            if let Some(old) = self.ship_task.remove(&id) {
                take(&mut self.ships_of_task, old, &id);
            }
            if let Some(task) = task {
                self.ship_task.insert(id, task);
                put(&mut self.ships_of_task, task, id);
            }
        }
        match self.ship_terms.remove(&id) {
            Some(ShipTerm::Unresolved) => self.unresolved_ships -= 1,
            Some(ShipTerm::Owned(slot, acquired, required)) => {
                if let Some((total, needed)) = self.slot_navies.get_mut(&slot) {
                    total.remove(acquired);
                    needed.remove(required);
                }
                slots.push(slot);
            }
            Some(ShipTerm::Unowned) | None => {}
        }
        let Some(term) = self.ship_term(roster, id) else {
            return;
        };
        match term {
            ShipTerm::Unresolved => self.unresolved_ships += 1,
            ShipTerm::Owned(slot, acquired, required) => {
                let (total, needed) = self.slot_navies.entry(slot).or_default();
                total.add(acquired);
                needed.add(required);
                slots.push(slot);
            }
            ShipTerm::Unowned => {}
        }
        self.ship_terms.insert(id, term);
    }

    /// What wing `id` adds now: none where gone.
    fn wing_term(&self, roster: &Roster, id: u128) -> Option<WingTerm> {
        let &(owner, exile, acquired) = self.kept.wings.values.get(&id)?;
        let ordinary = roster.slot_of(owner).and_then(|slot| match exile {
            None => Some((slot, None)),
            Some(exile) if exile <= 0 => Some((slot, acquired)),
            Some(_) => None,
        });
        let exiled = exile
            .filter(|&exile| exile > 0)
            .and_then(|exile| roster.slot_of(exile as u32))
            .map(|slot| (slot, acquired));
        Some(WingTerm {
            ordinary,
            exiled,
            unread: exile.is_none(),
        })
    }

    /// Wing `id`'s term as it is now, its old one taken out; the slots it touches are `slots`.
    fn set_wing(&mut self, roster: &Roster, id: u128, slots: &mut Vec<u32>) {
        if let Some(old) = self.wing_terms.remove(&id) {
            if let Some((slot, term)) = old.ordinary {
                if let Some(sums) = self.slot_crews.get_mut(&slot) {
                    sums.0.remove(term);
                }
                slots.push(slot);
            }
            if let Some((slot, term)) = old.exiled {
                if let Some(sums) = self.slot_crews.get_mut(&slot) {
                    sums.1.remove(term);
                }
                slots.push(slot);
            }
            self.unread_exiles -= u32::from(old.unread);
        }
        let Some(term) = self.wing_term(roster, id) else {
            return;
        };
        if let Some((slot, value)) = term.ordinary {
            self.slot_crews.entry(slot).or_default().0.add(value);
            slots.push(slot);
        }
        if let Some((slot, value)) = term.exiled {
            self.slot_crews.entry(slot).or_default().1.add(value);
            slots.push(slot);
        }
        self.unread_exiles += u32::from(term.unread);
        self.wing_terms.insert(id, term);
    }

    /// `member`'s deployed navy and required crews, as the frame reader sums them: unknown for
    /// every country while a ship's task force or fleet is not held.
    fn navy_of(&self, member: &MemberOf) -> Option<(Option<i32>, Option<i32>)> {
        if !self.complete[at::SHIPS]
            || !self.complete[at::TASKS]
            || !self.complete[at::FLEETS]
            || self.unresolved_ships != 0
        {
            return None;
        }
        Some(
            self.slot_navies
                .get(&member.slot)
                .map_or((Some(0), Some(0)), |(total, needed)| (total.value(), needed.value())),
        )
    }

    /// `member`'s ordinary and exile air crews, as the frame reader sums them: exile crews
    /// unknown for every country while a wing's exile country is unread.
    fn air_of(&self, member: &MemberOf) -> Option<(Option<i32>, Option<i32>)> {
        if !self.complete[at::WINGS] {
            return None;
        }
        let sums = self.slot_crews.get(&member.slot).copied().unwrap_or_default();
        Some((
            sums.0.value(),
            (self.unread_exiles == 0).then(|| sums.1.value()).flatten(),
        ))
    }

    /// `member`'s controlled states in their recorded order: none where unknown.
    fn roster_of(&self, member: &MemberOf) -> Option<Vec<u32>> {
        first(&self.kept.counters.values, &member.aliases)
            .and_then(|(_, row)| row.states.clone())
            .filter(|_| self.complete[at::STATES])
    }

    /// State `state_id`'s operands as `member` controls it: none without its row.
    fn state_of(&self, roster: &Roster, member: &MemberOf, state_id: u32) -> Option<StateOperands> {
        let complete = &self.complete;
        let &(available, locked, total) = self.kept.states.values.get(&state_id)?;
        Some(StateOperands {
            available,
            locked,
            total,
            resources: complete[at::RESOURCES]
                .then(|| self.kept.resources.values.get(&state_id).copied().flatten())
                .flatten(),
            core: complete[at::CORES]
                .then(|| {
                    self.kept
                        .cores
                        .values
                        .get(&state_id)
                        .map(|cores| cores.iter().any(|&raw| equal(roster, raw, member.id)))
                })
                .flatten(),
        })
    }

    /// `member`'s own operands from what is kept and summed; its states are read apart.
    fn country(&self, state: &State, member: &MemberOf) -> io::Result<Operands> {
        let complete = &self.complete;
        let id = member.id;
        let counters = first(&self.kept.counters.values, &member.aliases).map(|(_, row)| row);
        let input = complete[at::STABILITY]
            .then(|| first(&self.kept.stability.values, &member.aliases))
            .flatten();
        let read_hour = match input {
            Some((raw, _)) if complete[at::READS] => {
                let reads = columns::table(state, TableId::SliceReads)?;
                let key = (TableId::CountryStabilityInputs as u128) << 32 | u128::from(raw);
                match reads.slot(key) {
                    Some(slot) => columns::plain::<i32>(reads, self.sample_hour)?
                        .get(slot as usize)
                        .copied(),
                    None => None,
                }
            }
            _ => None,
        };
        let effective = counters.and_then(|row| row.modifiers).and_then(|values| {
            let mut result = [0; 4];
            for (at, value) in values.into_iter().enumerate() {
                result[at] = match value {
                    None => 0,
                    Some(raw) => self.rules.as_ref()?[at]?.of(raw),
                };
            }
            Some(result)
        });
        let residual = {
            let countries = columns::table(state, TableId::Countries)?;
            let slot = countries
                .slot(u128::from(id))
                .ok_or_else(|| io::Error::other("A manpower country's row is missing"))?;
            columns::plain::<i32>(countries, self.residual)?[slot as usize]
        };
        let navy = self.navy.get(&id).copied();
        let air = self.air.get(&id).copied();
        Ok(Operands {
            residual,
            states: None,
            applied: input.and_then(|(_, ratio)| *ratio),
            read_hour,
            effective,
            deployed: [
                self.army.get(&id).copied().flatten(),
                navy.and_then(|values| values.0),
                navy.and_then(|values| values.1),
                air.and_then(|values| values.0),
                air.and_then(|values| values.1),
            ],
        })
    }

    /// Keeps the raws whose expeditionaries or volunteers list each division.
    fn index_listers(&mut self) {
        self.listers.clear();
        for (&raw, counters) in &self.kept.counters.values {
            for &division in counters.expeditionaries.iter().flatten() {
                put(&mut self.listers, division, raw);
            }
        }
        for &(sender, division) in self.kept.volunteers.values.values() {
            put(&mut self.listers, division, sender);
        }
    }
}

// Every segment starts cold: its first hour reads every table whole again; what came before is
// the emit's.
impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, _: segments::Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.cold = true;
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("manpower.columnar", Some("manpower"));
        let state = hour.state;
        let roster = hour.world.countries()?;
        let divisions = hour.world.divisions()?.divisions();
        let complete: Vec<bool> = TABLES
            .iter()
            .map(|&table| columns::complete(state, table))
            .collect();
        let skipped = self
            .last
            .is_some_and(|last| last.checked_add(1) != Some(hour.hour));
        self.last = Some(hour.hour);
        let whole = self.cold
            || hour.starts
            || skipped
            || self.complete != complete
            || roster.rebuilt();
        if self.cold || hour.starts || skipped {
            // The emit ends every run at an unread hour: what follows is sent whole.
            self.sent.clear();
        }
        self.cold = false;
        self.complete = complete;
        let mut touched = Touched::default();
        self.follow(state, whole, &mut touched)?;
        // Only the definitions' own tokens are joined to them.
        let rules = whole
            || !touched.definitions.is_empty()
            || self.rule_tokens_written(state)?
            || !touched.features.is_empty();
        if rules {
            self.rules = self.rules(state);
            self.rule_tokens = self
                .kept
                .definitions
                .values
                .values()
                .map(|definition| definition.token)
                .collect();
        }
        let identity = |country: &Member| (country.tag, country.slot, country.resolved);
        let membership = whole
            || roster.rebuilt()
            || roster
                .changed_links()
                .iter()
                .any(|&raw| roster.slot_of(raw) != self.links.get(&raw).copied())
            || roster
                .changed_ids()
                .iter()
                .any(|&id| roster.get(id).map(identity) != self.identities.get(&id).copied());
        if membership {
            self.identities = roster.iter().map(|country| (country.id, identity(country))).collect();
            self.links = roster
                .links()
                .iter()
                .map(|link| (link.id, link.canonical_slot))
                .collect();
            self.members = if roster.complete() {
                roster
                    .iter()
                    .filter_map(|country| member(roster, country.id).map(|(country, _)| country))
                    .map(|country| MemberOf {
                        id: country.id,
                        slot: country.slot,
                        aliases: aliases(roster, country),
                    })
                    .collect()
            } else {
                Vec::new()
            };
            self.by_member_slot = self
                .members
                .iter()
                .enumerate()
                .map(|(at, member)| (member.slot, at))
                .collect();
            self.sent
                .retain(|id, _| self.members.iter().any(|member| member.id == *id));
        }
        self.index_rosters(&touched.counters, whole);
        if whole || !touched.counters.is_empty() || !touched.volunteers.is_empty() {
            self.index_listers();
        }
        let rebuild = whole || membership || !touched.templates.is_empty();
        let (mut army_slots, mut listed) = (Vec::new(), Vec::new());
        self.index_armies(roster, divisions, (rebuild, &touched.land), &mut army_slots, &mut listed);
        let mut due = self.due(roster, &touched, whole || membership, rules);
        let dirty = &mut due.country;
        let mark = |dirty: &mut Vec<bool>, by: &FastMap<u32, usize>, raw: u32| {
            if let Some(&at) = roster.slot_of(raw).and_then(|slot| by.get(&slot)) {
                dirty[at] = true;
            }
        };
        // A read hour or residual the hour wrote.
        let reads_table = columns::table(state, TableId::SliceReads)?;
        let reads = columns::changes(state, TableId::SliceReads)?;
        let of_inputs = |key: u128| (key >> 32) as u32 == TableId::CountryStabilityInputs as u32;
        for key in reads
            .changed
            .iter()
            .chain(&reads.inserted)
            .chain(&reads.fill_slots)
            .filter_map(|&slot| reads_table.key(slot))
            .chain(reads.deleted_keys.iter().copied())
            .filter(|&key| of_inputs(key))
        {
            mark(dirty, &self.by_member_slot, key as u32);
        }
        let countries = columns::table(state, TableId::Countries)?;
        let written = columns::changes(state, TableId::Countries)?;
        for (&slot, &mask) in written
            .changed
            .iter()
            .zip(&written.masks)
            .chain(written.fill_slots.iter().zip(&written.fill_masks))
        {
            if mask & (1 << self.residual) != 0
                && let Some(key) = countries.key(slot)
            {
                mark(dirty, &self.by_member_slot, key as u32);
            }
        }
        // The armies the hour may change: every one on a rebuild, else those of the slots whose
        // sums moved and the raws listing a division the hour touched.
        let armies: Vec<usize> = if rebuild {
            (0..self.members.len()).collect()
        } else {
            let mut armies = vec![false; self.members.len()];
            for slot in &army_slots {
                if let Some(&at) = self.by_member_slot.get(slot) {
                    armies[at] = true;
                }
            }
            for division in &touched.land {
                for &raw in self.listers.get(division).into_iter().flatten() {
                    mark(&mut armies, &self.by_member_slot, raw);
                }
            }
            for &raw in listed.iter().chain(&touched.counters) {
                mark(&mut armies, &self.by_member_slot, raw);
            }
            for &key in &touched.volunteers {
                mark(&mut armies, &self.by_member_slot, (key >> 64) as u32);
            }
            armies
                .into_iter()
                .enumerate()
                .filter_map(|(at, due)| due.then_some(at))
                .collect()
        };
        for at in armies {
            let member = &self.members[at];
            let army = self.army_of(divisions, member);
            if self.army.insert(member.id, army) != Some(army) {
                dirty[at] = true;
            }
        }
        // Ships and wings: anew when their chains or the roster may have moved, else those the
        // hour touched.
        let (mut navy_slots, every_navy_rebuilt) = (Vec::new(), whole || membership);
        let mut every_navy = every_navy_rebuilt;
        let unresolved = self.unresolved_ships;
        if every_navy_rebuilt {
            self.ship_terms.clear();
            self.slot_navies.clear();
            self.unresolved_ships = 0;
            self.ship_task.clear();
            self.ships_of_task.clear();
            self.task_fleet.clear();
            self.tasks_of_fleet.clear();
            let tasks: Vec<(u64, u64)> = self
                .kept
                .tasks
                .values
                .iter()
                .map(|(&task, &fleet)| (task, fleet))
                .collect();
            for (task, fleet) in tasks {
                self.task_fleet.insert(task, fleet);
                put(&mut self.tasks_of_fleet, fleet, task);
            }
            let ships: Vec<u128> = self.kept.ships.values.keys().copied().collect();
            for id in ships {
                self.set_ship(roster, id, &mut navy_slots);
            }
        } else {
            // The ships a task force's fleet, or a fleet's owner, changing reaches.
            let mut ships: Vec<u128> = touched.ships.clone();
            for &task in &touched.tasks {
                let fleet = self.kept.tasks.values.get(&task).copied();
                if self.task_fleet.get(&task).copied() != fleet {
                    if let Some(old) = self.task_fleet.remove(&task) {
                        take(&mut self.tasks_of_fleet, old, &task);
                    }
                    if let Some(fleet) = fleet {
                        self.task_fleet.insert(task, fleet);
                        put(&mut self.tasks_of_fleet, fleet, task);
                    }
                }
                ships.extend(self.ships_of_task.get(&task).into_iter().flatten().copied());
            }
            for fleet in &touched.fleets {
                for task in self.tasks_of_fleet.get(fleet).into_iter().flatten() {
                    ships.extend(self.ships_of_task.get(task).into_iter().flatten().copied());
                }
            }
            ships.sort_unstable();
            ships.dedup();
            for id in ships {
                self.set_ship(roster, id, &mut navy_slots);
            }
        }
        every_navy |= (unresolved == 0) != (self.unresolved_ships == 0);
        let (mut air_slots, mut every_air) = (Vec::new(), whole || membership);
        let unread = self.unread_exiles;
        if whole || membership {
            self.wing_terms.clear();
            self.slot_crews.clear();
            self.unread_exiles = 0;
            let wings: Vec<u128> = self.kept.wings.values.keys().copied().collect();
            for id in wings {
                self.set_wing(roster, id, &mut air_slots);
            }
        } else {
            for &id in &touched.wings {
                self.set_wing(roster, id, &mut air_slots);
            }
        }
        every_air |= (unread == 0) != (self.unread_exiles == 0);
        let mut changed = vec![false; self.members.len()];
        for (every, slots) in [(every_navy, &navy_slots), (every_air, &air_slots)] {
            if every {
                changed.fill(true);
            }
            for slot in slots {
                if let Some(&at) = self.by_member_slot.get(slot) {
                    changed[at] = true;
                }
            }
        }
        let updates: Vec<_> = self
            .members
            .iter()
            .enumerate()
            .filter(|&(at, _)| changed[at])
            .map(|(at, member)| (at, member.id, self.navy_of(member), self.air_of(member)))
            .collect();
        for (at, id, navy, air) in updates {
            if self.navy.get(&id).copied() != navy || self.air.get(&id).copied() != air {
                dirty[at] = true;
            }
            for (held, now) in [(&mut self.navy, navy), (&mut self.air, air)] {
                match now {
                    Some(now) => held.insert(id, now),
                    None => held.remove(&id),
                };
            }
        }
        let mut named = Vec::new();
        let valid = if membership {
            let mut valid = Vec::with_capacity(self.members.len());
            for at in 0..self.members.len() {
                if let Some((country, text)) = member(roster, self.members[at].id) {
                    valid.push(self.number(country.tag, text, &mut named));
                }
            }
            valid.sort_unstable();
            valid.dedup();
            Some(valid)
        } else {
            None
        };
        let mut states_due: FastMap<usize, Vec<u32>> = FastMap::default();
        for &(at, state) in &due.states {
            states_due.entry(at).or_default().push(state);
        }
        let mut countries = Vec::new();
        for at in 0..self.members.len() {
            let mut recompute = states_due.remove(&at).unwrap_or_default();
            if !due.country[at] && !due.roster[at] && recompute.is_empty() {
                continue;
            }
            let of = &self.members[at];
            let Some((country, text)) = member(roster, of.id) else {
                continue;
            };
            let (holder, tag) = ((country.slot, country.id), country.tag);
            // A tag, as a holder, changing starts its keys' segment anew.
            let kept = self
                .sent
                .remove(&of.id)
                .filter(|sent| sent.holder == holder && sent.tag == tag);
            let whole_send = kept.is_none();
            let mut sent = kept.unwrap_or_else(|| Sent {
                holder,
                tag,
                country: [None; 12],
                roster: None,
                states: FastMap::default(),
            });
            let mut values = Values::default();
            if whole_send || due.country[at] {
                let now = super::emit::values(&self.country(state, of)?);
                for (at, &measure) in COUNTRY.iter().enumerate() {
                    if now[at] != sent.country[at] {
                        values.push((measure, None), now[at]);
                    }
                }
                sent.country = now;
            }
            let mut send_roster = whole_send;
            if whole_send || due.roster[at] {
                let now = self.roster_of(of);
                if whole_send || now != sent.roster {
                    send_roster = true;
                    let listed: FastSet<u32> = now.iter().flatten().copied().collect();
                    let mut left: Vec<(u32, [Option<i64>; 8])> = sent
                        .states
                        .iter()
                        .filter(|(state, _)| !listed.contains(state))
                        .map(|(&state, held)| (state, *held))
                        .collect();
                    left.sort_unstable_by_key(|(state, _)| *state);
                    for (state, held) in left {
                        sent.states.remove(&state);
                        for (at, &measure) in STATE.iter().enumerate() {
                            if held[at].is_some() {
                                values.push((measure, Some(u64::from(state))), None);
                            }
                        }
                    }
                    for &state in &listed {
                        sent.states.entry(state).or_insert([None; 8]);
                    }
                    recompute = listed.into_iter().collect();
                    sent.roster = now;
                }
            }
            if due.every {
                recompute.extend(sent.states.keys().copied());
            }
            recompute.sort_unstable();
            recompute.dedup();
            for state in recompute {
                // A state the roster does not list holds no keys of the country's.
                if !sent.states.contains_key(&state) {
                    continue;
                }
                let now = super::emit::state_values(self.state_of(roster, of, state).as_ref());
                let held = sent.states.get_mut(&state).expect("a listed state");
                for (at, &measure) in STATE.iter().enumerate() {
                    if now[at] != held[at] {
                        values.push((measure, Some(u64::from(state))), now[at]);
                    }
                }
                *held = now;
            }
            let roster_sent = send_roster.then(|| sent.roster.as_deref().map(Box::from));
            self.sent.insert(of.id, sent);
            if values.is_empty() && roster_sent.is_none() {
                continue;
            }
            values.shrink();
            let index = self.number(tag, text, &mut named);
            countries.push(Country {
                index,
                holder,
                whole: whole_send,
                values,
                roster: roster_sent,
            });
        }
        out.push(Hour {
            branch: hour.branch,
            hour: hour.hour,
            named,
            valid,
            countries,
        })
    }

    fn carried(&self) -> Option<usize> {
        Some(self.heap())
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(SAVED);
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes != SAVED {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a saved manpower projector",
            ));
        }
        *self = Self::new(self.format)?;
        Ok(())
    }
}
