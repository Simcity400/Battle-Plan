use super::*;
use crate::products::occupation_trend::{
    OccupationCollaboration, OccupationCollaborationActor, OccupationTrendRun,
};
use dashboard_base::products::country_history::Identity;
use dashboard_text::triggers::inputs::{TableChange, TableChanges};
use dashboard_world::{Hold, Needs, Roster, World};
use replay::{
    columnar_frames::metrics,
    columns,
    segments::{self, Item, Operator, Sink, State},
};
use replay_data::rows::{DeltaRow, full_mask};
use replay_data::{
    format::FormatVersion,
    rows::{RowCodec, TABLE_COUNT},
    scope::CaptureMode,
};
use std::{
    cell::{Cell, OnceCell},
    collections::BTreeMap,
    mem::size_of,
    sync::{Mutex, PoisonError},
};

pub mod emit;
mod joins;

pub use emit::Emit;

/// What a projection reads of its walk's world: the countries, and its own tables whole. The
/// trigger tables a law reads are asked for at the hours it reads them.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        frame: super::super::prepared::observed_tables(false)
            .into_iter()
            .map(|table| (table, Hold::Whole))
            .collect(),
        ..Needs::default()
    }
}

/// Every text the projections' values name, each once, by an id the
/// projections and their ordered emits share.
#[derive(Default)]
pub(in crate::products::occupied_states) struct Texts(Mutex<TextTable>);

#[derive(Default)]
struct TextTable {
    ids: HashMap<Arc<str>, u32>,
    texts: Arc<Vec<Arc<str>>>,
}

impl Texts {
    fn id(&self, text: &str) -> u32 {
        let mut table = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(&id) = table.ids.get(text) {
            return id;
        }
        let id = u32::try_from(table.texts.len()).expect("occupation text ids fit 32 bits");
        let text: Arc<str> = Arc::from(text);
        Arc::make_mut(&mut table.texts).push(Arc::clone(&text));
        table.ids.insert(text, id);
        id
    }

    /// Every text named so far, by id.
    fn all(&self) -> Arc<Vec<Arc<str>>> {
        Arc::clone(&self.0.lock().unwrap_or_else(PoisonError::into_inner).texts)
    }
}

/// The ids of the texts one projection named.
struct Named {
    shared: Arc<Texts>,
    ids: HashMap<String, u32>,
}

impl Named {
    fn id(&mut self, text: &str) -> u32 {
        if let Some(&id) = self.ids.get(text) {
            return id;
        }
        let id = self.shared.id(text);
        self.ids.insert(text.to_owned(), id);
        id
    }
}

/// A state's values at a read, as [`OccupiedValues`] holds them, each text
/// by its id.
#[derive(Clone, Copy, PartialEq, Eq)]
struct StateValue {
    name: Option<u32>,
    name_missing: Option<u32>,
    owner: Option<u32>,
    occupier: Option<u32>,
    occupied: Option<u32>,
    resistance_raw: i64,
    compliance_raw: i64,
    resistance_speed_raw: i64,
    compliance_speed_raw: i64,
    garrison_strength_raw: Option<i64>,
    required_manpower: Option<i32>,
    garrison_required_raw: Option<i64>,
    garrison_alert: bool,
    garrison_red: Option<bool>,
    assigned_manpower: Option<i32>,
    garrison_missing: Option<u32>,
    law: Option<u32>,
    law_missing: Option<u32>,
}

impl StateValue {
    fn of(value: &OccupiedValues, named: &mut Named) -> Self {
        let mut id = |text: &Option<String>| text.as_deref().map(|text| named.id(text));
        Self {
            name: id(&value.name),
            name_missing: id(&value.name_missing),
            owner: id(&value.owner),
            occupier: id(&value.occupier),
            occupied: id(&value.occupied),
            resistance_raw: value.resistance_raw,
            compliance_raw: value.compliance_raw,
            resistance_speed_raw: value.resistance_speed_raw,
            compliance_speed_raw: value.compliance_speed_raw,
            garrison_strength_raw: value.garrison_strength_raw,
            required_manpower: value.required_manpower,
            garrison_required_raw: value.garrison_required_raw,
            garrison_alert: value.garrison_alert,
            garrison_red: value.garrison_red,
            assigned_manpower: value.assigned_manpower,
            garrison_missing: id(&value.garrison_missing),
            law: id(&value.law),
            law_missing: id(&value.law_missing),
        }
    }

    fn shown(&self, texts: &[Arc<str>]) -> OccupiedValues {
        let text = |id: Option<u32>| id.map(|id| texts[id as usize].to_string());
        OccupiedValues {
            name: text(self.name),
            name_missing: text(self.name_missing),
            owner: text(self.owner),
            occupier: text(self.occupier),
            occupied: text(self.occupied),
            resistance_raw: self.resistance_raw,
            compliance_raw: self.compliance_raw,
            resistance_speed_raw: self.resistance_speed_raw,
            compliance_speed_raw: self.compliance_speed_raw,
            garrison_strength_raw: self.garrison_strength_raw,
            required_manpower: self.required_manpower,
            garrison_required_raw: self.garrison_required_raw,
            garrison_alert: self.garrison_alert,
            garrison_red: self.garrison_red,
            assigned_manpower: self.assigned_manpower,
            garrison_missing: text(self.garrison_missing),
            law: text(self.law),
            law_missing: text(self.law_missing),
        }
    }
}

/// A States row as the emit projects an occupied state's owner and name
/// from it.
struct Shipped {
    owner: Option<u32>,
    /// Why the owner has no tag, when it has none.
    owner_refused: Option<Box<(String, String)>>,
    /// A digest of what the name is read from: the custom and owner names.
    name_key: [u8; 32],
    /// The name, or why it is missing, and what reading it noted.
    name: (Option<u32>, Option<u32>),
    named: catalogue::OwnRefusals,
}

/// The countries' naming at an hour, as the emit reads it.
#[derive(Default)]
struct Naming {
    stamp: [u8; 32],
    refused: Refusals,
    tags: HashMap<Identity, u32>,
}

/// One occupied country's collaboration with its occupier, as
/// [`OccupationCollaboration`] shows it.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Collab {
    tag: Option<u32>,
    raw: Option<i64>,
    read_hour: Option<i32>,
    slot: u32,
    country_id: u32,
    name_hour: Option<i32>,
}

impl Collab {
    fn shown(&self, tag: Option<String>) -> OccupationCollaboration {
        OccupationCollaboration {
            tag,
            raw: self.raw.map(|raw| raw.to_string()),
            read_hour: self.read_hour,
            actor: Some(OccupationCollaborationActor {
                slot: self.slot,
                country_id: self.country_id,
                name_hour: self.name_hour,
            }),
        }
    }
}

/// Each (occupied, occupier) pair's collaboration progress and the hour it
/// was read.
type Progress = HashMap<(Identity, Identity), (i64, i32)>;

enum Collaboration {
    /// The table is unavailable: no progress is known.
    Unavailable,
    /// A read's progress, and the rows whose countries do not resolve.
    Read(Arc<Progress>, Refusals),
}

/// An occupier's means at a read, as [`OccupationTrendRun`] holds them.
struct Sample {
    identity: Identity,
    resistance: i64,
    compliance: i64,
    states: u32,
    held_states: u32,
    held_complete: bool,
    /// Its occupied countries, ascending.
    occupied: Arc<[Identity]>,
    /// Its collaboration at the read, once the projection read the progress.
    collaboration: Option<Arc<[Collab]>>,
    /// Its run's fact at the read, for a run its read starts.
    fact: Option<Fact>,
}

/// An occupier's run as its fact is published, encoded before its segment
/// start is known.
struct Fact {
    bytes: Vec<u8>,
    starts: usize,
}

/// The field of a run's fact that says whether it starts a segment.
const STARTS_SEGMENT: &[u8] = b"\"startsSegment\":";

impl Fact {
    fn new(bytes: Vec<u8>) -> io::Result<Self> {
        let starts = bytes
            .windows(STARTS_SEGMENT.len())
            .position(|field| field == STARTS_SEGMENT)
            .map(|at| at + STARTS_SEGMENT.len())
            .filter(|&at| bytes[at..].starts_with(b"false"))
            .ok_or_else(|| dashboard_core::invalid("An occupation fact lacks its segment start"))?;
        Ok(Self { bytes, starts })
    }

    fn finish(mut self, starts_segment: bool) -> Vec<u8> {
        if starts_segment {
            self.bytes
                .splice(self.starts..self.starts + "false".len(), *b"true");
        }
        self.bytes
    }
}

struct Read {
    reset: bool,
    values: Vec<(u32, Option<StateValue>)>,
    complete: bool,
    samples: Vec<Sample>,
    refused: Refusals,
    trend_refused: Refusals,
    collaboration_refused: Refusals,
    own: catalogue::OwnRefusals,
}

pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    valid: bool,
    naming: Arc<Naming>,
    /// Whether `states` lists every States row, not only those the hour
    /// changed.
    whole: bool,
    states: Vec<(u32, Option<Shipped>)>,
    read: Option<Read>,
    collaboration: Option<Collaboration>,
    /// The bytes of the naming made since an hour last carried it: the first to carry it
    /// counts it.
    shared: usize,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        // A read's progress goes to its hour alone; samples count the occupied countries they
        // share. Refusals are not counted.
        let progress = match &self.collaboration {
            Some(Collaboration::Read(progress, _)) => {
                progress.len() * size_of::<((Identity, Identity), (i64, i32))>()
            }
            _ => 0,
        };
        let states: usize = self
            .states
            .iter()
            .filter_map(|(_, shipped)| shipped.as_ref()?.owner_refused.as_ref())
            .map(|refused| {
                size_of::<(String, String)>() + refused.0.capacity() + refused.1.capacity()
            })
            .sum();
        let read = self.read.as_ref().map_or(0, |read| {
            let samples: usize = read
                .samples
                .iter()
                .map(|sample| {
                    let collaboration = sample.collaboration.as_ref().map_or(0, |held| held.len());
                    collaboration * size_of::<Collab>()
                        + sample.occupied.len() * size_of::<Identity>()
                        + sample.fact.as_ref().map_or(0, |fact| fact.bytes.capacity())
                })
                .sum();
            read.values.capacity() * size_of::<(u32, Option<StateValue>)>()
                + read.samples.capacity() * size_of::<Sample>()
                + samples
        });
        self.shared
            + progress
            + self.states.capacity() * size_of::<(u32, Option<Shipped>)>()
            + states
            + read
    }
}

#[cfg(test)]
impl Hour {
    pub(in crate::products::occupied_states) fn evaluated_states(&self) -> usize {
        self.read.as_ref().map_or(0, |read| read.values.len())
    }
}

enum Edit {
    Same,
    Any,
    Rows(Vec<DeltaRow>, Vec<u128>),
}

/// What the hour changed in each table the projection reads, each found
/// when first asked: the law memo asks only of the tables its laws read.
struct Changes<'h> {
    state: &'h State,
    format: FormatVersion,
    read: [bool; TABLE_COUNT],
    /// The tables the hour may have changed anywhere: read anew, or changed
    /// in status.
    whole: [bool; TABLE_COUNT],
    edits: [OnceCell<Edit>; TABLE_COUNT],
}

impl<'h> Changes<'h> {
    fn new(
        hour: &segments::Hour<'h, World>,
        format: FormatVersion,
        tables: &[TableId],
        shapes: &mut [Option<columns::Status>; TABLE_COUNT],
    ) -> Self {
        let mut read = [false; TABLE_COUNT];
        let mut whole = [false; TABLE_COUNT];
        for &id in tables {
            let now = columns::status(hour.state, id);
            read[id.index()] = true;
            whole[id.index()] = hour.starts || hour.opens || shapes[id.index()] != Some(now);
            shapes[id.index()] = Some(now);
        }
        Self {
            state: hour.state,
            format,
            read,
            whole,
            edits: std::array::from_fn(|_| OnceCell::new()),
        }
    }

    fn edit(&self, id: TableId) -> Edit {
        if !self.read[id.index()] {
            return Edit::Same;
        }
        let (Ok(changes), Ok(held)) = (
            columns::changes(self.state, id),
            columns::table(self.state, id),
        ) else {
            return Edit::Any;
        };
        if self.whole[id.index()] || changes.truncated {
            return Edit::Any;
        }
        let fields = id.last_change().fields(id, self.format.version());
        let mut rows = BTreeMap::<u128, u64>::new();
        for (&slot, &mask) in changes
            .changed
            .iter()
            .zip(&changes.masks)
            .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
        {
            if let Some(key) = held.key(slot) {
                *rows.entry(key).or_default() |= mask;
            }
        }
        for &slot in &changes.inserted {
            if let Some(key) = held.key(slot) {
                rows.insert(key, full_mask(fields));
            }
        }
        let mut deleted = changes.deleted_keys.clone();
        deleted.sort_unstable();
        if rows.is_empty() && deleted.is_empty() {
            return Edit::Same;
        }
        Edit::Rows(
            rows.into_iter()
                .map(|(key, mask)| DeltaRow {
                    format: self.format.version(),
                    key,
                    mask,
                    recorded_fields: fields,
                    values: Vec::new(),
                })
                .collect(),
            deleted,
        )
    }
}

impl TableChanges for Changes<'_> {
    fn table(&self, table: TableId) -> TableChange<'_> {
        match self.edits[table.index()].get_or_init(|| self.edit(table)) {
            Edit::Same => TableChange::Same,
            Edit::Any => TableChange::Any,
            Edit::Rows(rows, deleted) => TableChange::Rows(rows, deleted),
        }
    }
}

struct Rows<R> {
    rows: BTreeMap<u128, R>,
    previous: BTreeMap<u128, R>,
    status: Option<columns::Status>,
}

impl<R> Default for Rows<R> {
    fn default() -> Self {
        Self {
            rows: BTreeMap::new(),
            previous: BTreeMap::new(),
            status: None,
        }
    }
}

impl<R: RowCodec> Rows<R> {
    fn update(
        &mut self,
        state: &State,
        id: TableId,
        opens: bool,
        mask: u64,
        decode: impl Fn(u32) -> io::Result<R>,
    ) -> io::Result<BTreeSet<u128>> {
        let status = columns::status(state, id);
        self.previous.clear();
        let edits = columns::changes(state, id)?;
        let whole = opens || self.status != Some(status) || edits.truncated;
        self.status = Some(status);
        let held = columns::table(state, id)?;
        let mut dirty = BTreeSet::new();
        if whole {
            let slots: Vec<_> = match status {
                columns::Status::Complete => held.live().ones().collect(),
                columns::Status::Truncated { .. } => {
                    columns::truncated(state, id).unwrap_or_default().to_vec()
                }
                _ => Vec::new(),
            };
            let mut now = BTreeMap::new();
            for slot in slots {
                let row = decode(slot)?;
                if self.rows.get(&row.key()) != Some(&row) {
                    dirty.insert(row.key());
                }
                now.insert(row.key(), row);
            }
            dirty.extend(
                self.rows
                    .keys()
                    .filter(|key| !now.contains_key(*key))
                    .copied(),
            );
            for &key in &dirty {
                if let Some(row) = self.rows.remove(&key) {
                    self.previous.insert(key, row);
                }
            }
            self.rows = now;
        } else {
            for key in &edits.deleted_keys {
                if let Some(row) = self.rows.remove(key) {
                    self.previous.insert(*key, row);
                    dirty.insert(*key);
                }
            }
            let slots: BTreeSet<_> = edits
                .inserted
                .iter()
                .copied()
                .chain(
                    edits
                        .changed
                        .iter()
                        .zip(&edits.masks)
                        .filter(|(_, bits)| **bits & mask != 0)
                        .map(|(&slot, _)| slot),
                )
                .chain(
                    edits
                        .fill_slots
                        .iter()
                        .zip(&edits.fill_masks)
                        .filter(|(_, bits)| **bits & mask != 0)
                        .map(|(&slot, _)| slot),
                )
                .collect();
            for slot in slots {
                if matches!(
                    status,
                    columns::Status::Unavailable { .. } | columns::Status::NotRecorded
                ) || columns::truncated(state, id)
                    .is_some_and(|shown| shown.binary_search(&slot).is_err())
                {
                    continue;
                }
                let row = decode(slot)?;
                if self.rows.get(&row.key()) != Some(&row) {
                    dirty.insert(row.key());
                    if let Some(old) = self.rows.remove(&row.key()) {
                        self.previous.insert(row.key(), old);
                    }
                    self.rows.insert(row.key(), row);
                }
            }
        }
        Ok(dirty)
    }
}

/// The projection's countries: their index and links, and their naming as
/// it hands it on.
struct Countries {
    index: Arc<CountryIndex>,
    links: Vec<CountryLinkRow>,
    naming: Arc<Naming>,
}

#[derive(Default)]
struct Aggregate {
    resistance: i64,
    compliance: i64,
    states: HashSet<u32>,
    occupied: BTreeMap<Identity, usize>,
}

struct Contribution {
    controller: Identity,
    occupied: Option<Identity>,
    state: u32,
    resistance: i64,
    compliance: i64,
}

/// An occupier's sums at the read, over its held denominator.
struct Total {
    resistance: i64,
    compliance: i64,
    states: u32,
    held: u32,
    /// Its occupied countries, ascending.
    occupied: Arc<[Identity]>,
    held_complete: bool,
}

pub struct Projector {
    format: FormatVersion,
    catalogue: Option<Arc<Catalogue>>,
    scan: Scan<'static>,
    named: Named,
    naming: Option<Arc<Countries>>,
    state_mask: u64,
    resistance_mask: u64,
    state_fields: [usize; 4],
    resistance_fields: [usize; 6],
    states: Rows<StateRow>,
    resistance: Rows<StateResistanceRow>,
    pending: BTreeSet<u128>,
    /// Every table it reads; the hour's frame holds its own, and the trigger tables a law
    /// of this walk read. The others are asked for at an hour a law reads them.
    observed: Vec<TableId>,
    tables: Vec<TableId>,
    /// The hour's frame, dropped as the hour ends.
    frame: Option<DecodedFrame>,
    joins: joins::Joins,
    inventory_dirty: BTreeSet<u32>,
    inventory_all: bool,
    last: Option<(u64, i32, bool)>,
    shapes: [Option<columns::Status>; TABLE_COUNT],
    contributions: HashMap<u32, Contribution>,
    aggregates: HashMap<Identity, Aggregate>,
    totals: HashMap<Identity, Total>,
    membership_dirty: BTreeSet<Identity>,
    inventory: BTreeMap<Identity, (HashSet<u32>, BTreeSet<Identity>)>,
    inventory_issues: HashMap<u32, Refusals>,
    inventory_complete: bool,
    inventory_refused: Refusals,
    inventory_collaboration_refused: Refusals,
    trend_refused: Refusals,
    collaboration_refused: Refusals,
    bad_controllers: HashSet<u32>,
    missing_states: HashSet<u32>,
    incomplete: HashSet<u32>,
    evaluated: bool,
    garrisons_read: Option<bool>,
    /// The collaboration progress, once this walk read it or knows there is
    /// none; until then the emit's carries on.
    progress: Option<Arc<Progress>>,
    /// The bytes of the naming made since an hour last carried it.
    charged: usize,
}

fn mask(table: TableId, names: &[&str]) -> io::Result<u64> {
    names.iter().try_fold(0, |bits, name| {
        let field = columns::field(table, name)?;
        Ok(bits | 1_u64.checked_shl(field as u32).unwrap_or(u64::MAX))
    })
}

fn fields<const N: usize>(table: TableId, names: [&str; N]) -> io::Result<[usize; N]> {
    let mut found = [0; N];
    for (at, name) in names.into_iter().enumerate() {
        found[at] = columns::field(table, name)?;
    }
    Ok(found)
}

fn cell<T: columns::Plain + Copy>(
    state: &State,
    table: TableId,
    field: usize,
    slot: u32,
) -> io::Result<T> {
    let held = columns::table(state, table)?;
    if held.recorded(field, slot)
        && let Some(value) = columns::plain::<T>(held, field)?.get(slot as usize)
    {
        return Ok(*value);
    }
    Err(dashboard_core::refused(format!(
        "Occupation {table:?} field {field} is not recorded at slot {slot}"
    )))
}

fn text(state: &State, table: TableId, field: usize, slot: u32) -> io::Result<Vec<u8>> {
    let held = columns::table(state, table)?;
    if held.recorded(field, slot)
        && let Some(&id) = columns::text(held, field)?.get(slot as usize)
        && let Some(bytes) = state.dictionary().get(id)
    {
        return Ok(bytes.to_vec());
    }
    Err(dashboard_core::refused(format!(
        "Occupation {table:?} text field {field} is not recorded at slot {slot}"
    )))
}

fn key(state: &State, table: TableId, slot: u32) -> io::Result<u32> {
    columns::table(state, table)?
        .key(slot)
        .and_then(|key| u32::try_from(key).ok())
        .ok_or_else(|| {
            dashboard_core::invalid(format!("Occupation {table:?} slot {slot} has no state key"))
        })
}

fn state_row(state: &State, fields: [usize; 4], slot: u32) -> io::Result<StateRow> {
    Ok(StateRow {
        id: key(state, TableId::States, slot)?,
        owner: cell(state, TableId::States, fields[0], slot)?,
        controller: cell(state, TableId::States, fields[1], slot)?,
        custom_name: text(state, TableId::States, fields[2], slot)?,
        owner_name: text(state, TableId::States, fields[3], slot)?,
        ..Default::default()
    })
}

fn resistance_row(state: &State, fields: [usize; 6], slot: u32) -> io::Result<StateResistanceRow> {
    Ok(StateResistanceRow {
        state: key(state, TableId::StateResistance, slot)?,
        country: cell(state, TableId::StateResistance, fields[0], slot)?,
        level: cell(state, TableId::StateResistance, fields[1], slot)?,
        speed: cell(state, TableId::StateResistance, fields[2], slot)?,
        compliance: cell(state, TableId::StateResistance, fields[3], slot)?,
        compliance_speed: cell(state, TableId::StateResistance, fields[4], slot)?,
        active: cell(state, TableId::StateResistance, fields[5], slot)?,
        ..Default::default()
    })
}

/// `row` as the emit projects its state's owner and name, through `countries`.
fn shipped(
    row: &StateRow,
    countries: &Countries,
    named: &mut Named,
    cat: Option<&Catalogue>,
    names: &StateNames,
) -> Shipped {
    let owner = link_slot(&countries.links, row.owner)
        .and_then(|slot| countries.index.identities.get(&slot).map(|&id| (slot, id)))
        .and_then(|identity| countries.naming.tags.get(&identity).copied());
    let owner_refused = owner.is_none().then(|| {
        Box::new(link_refusal(
            &countries.index,
            "owner",
            row.owner,
            row.id,
            &countries.links,
        ))
    });
    let mut key = blake3::Hasher::new();
    key.update(&(row.custom_name.len() as u64).to_le_bytes());
    key.update(&row.custom_name);
    key.update(&row.owner_name);
    let (name, own) = catalogue::capture_isolated(|| name_of(row, cat, names));
    Shipped {
        owner,
        owner_refused,
        name_key: *key.finalize().as_bytes(),
        name: match name {
            Ok(name) => (Some(named.id(&name)), None),
            Err(reason) => (None, Some(named.id(&reason))),
        },
        named: own,
    }
}

impl Projector {
    pub(in crate::products::occupied_states) fn new(
        format: FormatVersion,
        _: CaptureMode,
        catalogue: Option<Arc<Catalogue>>,
        scan: Scan<'static>,
        texts: Arc<Texts>,
    ) -> io::Result<Self> {
        let tables = super::super::prepared::observed_tables(false);
        let observed = super::super::prepared::observed_tables(scan.reads_trigger_world());
        Ok(Self {
            format,
            catalogue,
            scan,
            named: Named {
                shared: texts,
                ids: HashMap::new(),
            },
            observed,
            tables,
            frame: None,
            joins: joins::Joins::default(),
            inventory_dirty: BTreeSet::new(),
            inventory_all: true,
            last: None,
            naming: None,
            state_mask: mask(
                TableId::States,
                &["owner", "controller", "custom_name", "owner_name"],
            )?,
            resistance_mask: mask(
                TableId::StateResistance,
                &[
                    "country",
                    "level",
                    "speed",
                    "compliance",
                    "compliance_speed",
                    "active",
                ],
            )?,
            state_fields: fields(
                TableId::States,
                ["owner", "controller", "custom_name", "owner_name"],
            )?,
            resistance_fields: fields(
                TableId::StateResistance,
                [
                    "country",
                    "level",
                    "speed",
                    "compliance",
                    "compliance_speed",
                    "active",
                ],
            )?,
            states: Rows::default(),
            resistance: Rows::default(),
            pending: BTreeSet::new(),
            shapes: [None; TABLE_COUNT],
            contributions: HashMap::new(),
            aggregates: HashMap::new(),
            totals: HashMap::new(),
            membership_dirty: BTreeSet::new(),
            inventory: BTreeMap::new(),
            inventory_issues: HashMap::new(),
            inventory_complete: false,
            inventory_refused: Refusals::default(),
            inventory_collaboration_refused: Refusals::default(),
            trend_refused: Refusals::default(),
            collaboration_refused: Refusals::default(),
            bad_controllers: HashSet::new(),
            missing_states: HashSet::new(),
            incomplete: HashSet::new(),
            evaluated: false,
            garrisons_read: None,
            progress: None,
            charged: 0,
        })
    }

    /// Moves the trigger tables a law read since into the frame every hour holds.
    fn grow(&mut self) {
        let observed = &self.observed;
        let tables = &mut self.tables;
        for table in self.scan.eligibility.watched() {
            if observed.binary_search(&table).is_ok() && !tables.contains(&table) {
                tables.push(table);
            }
        }
        tables.sort_unstable();
    }

    /// The countries as the walk's roster holds them: their index, links and naming.
    fn countries(&mut self, roster: &Roster) -> io::Result<Arc<Countries>> {
        let countries = roster.country_rows()?;
        let links = roster.links().to_vec();
        let mut refused = Refusals::default();
        let index = Arc::new(country_index(&countries, &links, &mut refused));
        let tags = index
            .names
            .iter()
            .map(|(&identity, tag)| (identity, self.named.id(tag)))
            .collect();
        Ok(Arc::new(Countries {
            naming: Arc::new(Naming {
                stamp: country_stamp(&countries, &links),
                refused,
                tags,
            }),
            index,
            links,
        }))
    }

    fn identity(&self, raw: u32) -> Option<Identity> {
        let names = self.naming.as_ref()?;
        let slot = link_slot(&names.links, raw)?;
        Some((slot, *names.index.identities.get(&slot)?))
    }

    fn remove(&mut self, id: u32, reached: &mut BTreeSet<Identity>) {
        self.scan.eligibility.forget_state(id);
        self.bad_controllers.remove(&id);
        self.missing_states.remove(&id);
        self.incomplete.remove(&id);
        let Some(old) = self.contributions.remove(&id) else {
            return;
        };
        let total = self
            .aggregates
            .get_mut(&old.controller)
            .expect("contribution total");
        total.resistance = total.resistance.wrapping_sub(old.resistance);
        total.compliance = total.compliance.wrapping_sub(old.compliance);
        total.states.remove(&old.state);
        if let Some(occupied) = old.occupied {
            let count = total.occupied.get_mut(&occupied).expect("occupied count");
            *count -= 1;
            if *count == 0 {
                total.occupied.remove(&occupied);
            }
        }
        if total.states.is_empty() {
            self.aggregates.remove(&old.controller);
        }
        reached.insert(old.controller);
    }

    fn add(
        &mut self,
        row: &StateResistanceRow,
        state: Option<&StateRow>,
        reached: &mut BTreeSet<Identity>,
    ) {
        if !occupied(row) {
            return;
        }
        let Some(state) = state else {
            self.missing_states.insert(row.state);
            self.trend_refused.note(
                "active resistance names a state absent from States",
                row.state.to_string(),
            );
            return;
        };
        let Some(controller) = self.identity(state.controller) else {
            self.bad_controllers.insert(state.id);
            self.trend_refused.note(
                "active resistance controller link is unresolved",
                format!("state {}, raw link {}", state.id, state.controller),
            );
            return;
        };
        let occupied = self.identity(row.country);
        if occupied.is_none() {
            self.collaboration_refused.note(
                "active resistance occupied-country link is unresolved",
                format!("state {}, raw link {}", state.id, row.country),
            );
        }
        let total = self.aggregates.entry(controller).or_default();
        total.resistance = total.resistance.wrapping_add(row.level);
        total.compliance = total.compliance.wrapping_add(row.compliance);
        total.states.insert(state.id);
        if let Some(occupied) = occupied {
            *total.occupied.entry(occupied).or_default() += 1;
        }
        self.contributions.insert(
            state.id,
            Contribution {
                controller,
                occupied,
                state: state.id,
                resistance: row.level,
                compliance: row.compliance,
            },
        );
        reached.insert(controller);
    }

    fn inventory(&mut self, names: &Countries) -> BTreeSet<Identity> {
        let mut reached = BTreeSet::new();
        if self.inventory_all {
            self.inventory_dirty
                .extend(self.inventory.keys().map(|identity| identity.0));
            self.inventory_dirty.extend(self.joins.inventories());
        }
        for country in std::mem::take(&mut self.inventory_dirty) {
            let gone: Vec<_> = self
                .inventory
                .range((country, 0)..=(country, u32::MAX))
                .map(|(&identity, _)| identity)
                .collect();
            for identity in gone {
                self.inventory.remove(&identity);
                reached.insert(identity);
            }
            self.inventory_issues.remove(&country);
            if let Some(&id) = names.index.identities.get(&country) {
                let mut issues = Refusals::default();
                let inventory = self.joins.inventory(country, names, &mut issues);
                if !issues.is_empty() {
                    self.inventory_issues.insert(country, issues);
                }
                if !inventory.0.is_empty() {
                    self.inventory.insert((country, id), inventory);
                }
                reached.insert((country, id));
            }
        }
        let mut refused = Refusals::default();
        self.inventory_complete = self.joins.inventory_complete(&mut refused);
        self.inventory_refused = refused;
        self.inventory_collaboration_refused = Refusals::default();
        for issues in self.inventory_issues.values() {
            self.inventory_collaboration_refused.merge(issues);
        }
        self.inventory_all = false;
        reached
    }

    fn total(&mut self, identity: Identity, garrisons: bool) {
        let active = self
            .aggregates
            .get(&identity)
            .filter(|total| !total.states.is_empty());
        let inventory = self.inventory.get(&identity).filter(|_| garrisons);
        if active.is_none() && inventory.is_none() {
            self.totals.remove(&identity);
            return;
        }
        let (resistance, compliance, states) = active.map_or((0, 0, 0), |active| {
            (
                active.resistance,
                active.compliance,
                active.states.len() as u32,
            )
        });
        if !self.membership_dirty.contains(&identity)
            && let Some(total) = self.totals.get_mut(&identity)
        {
            total.resistance = resistance;
            total.compliance = compliance;
            total.states = states;
            return;
        }
        let mut held = inventory.map_or_else(HashSet::new, |(held, _)| held.clone());
        let mut occupied = inventory.map_or_else(BTreeSet::new, |(_, occupied)| occupied.clone());
        let mut held_complete = inventory.is_some();
        if let Some(active) = active {
            for &state in &active.states {
                if held.insert(state) {
                    held_complete = false;
                }
            }
            occupied.extend(active.occupied.keys().copied());
        }
        self.totals.insert(
            identity,
            Total {
                resistance,
                compliance,
                states,
                held: held.len() as u32,
                occupied: occupied.into_iter().collect(),
                held_complete,
            },
        );
    }

    /// Every occupier's means at the read at `at`, each with its
    /// collaboration and its run's fact once the progress is known.
    fn samples(
        &self,
        at: i32,
        names: &Countries,
        inventory_complete: bool,
        refused: &mut Refusals,
    ) -> io::Result<Vec<Sample>> {
        let mut samples = Vec::with_capacity(self.totals.len());
        for (&identity, total) in &self.totals {
            if total.states == 0 {
                continue;
            }
            let Some((resistance, compliance)) = trend::mean(total.resistance, total.states)
                .zip(trend::mean(total.compliance, total.states))
            else {
                refused.note(
                    "native occupation mean is non-finite or not representable",
                    format!("country slot {}, read {at}", identity.0),
                );
                continue;
            };
            let mut sample = Sample {
                identity,
                resistance,
                compliance,
                states: total.states,
                held_states: total.held,
                held_complete: inventory_complete && total.held_complete,
                occupied: Arc::clone(&total.occupied),
                collaboration: None,
                fact: None,
            };
            if let Some(progress) = &self.progress {
                let mut entries: Vec<_> = total
                    .occupied
                    .iter()
                    .map(|&occupied| {
                        let tag = names.index.names.get(&occupied);
                        let read = progress.get(&(occupied, identity));
                        (
                            tag,
                            Collab {
                                tag: tag.and_then(|_| names.naming.tags.get(&occupied).copied()),
                                raw: read.map(|&(raw, _)| raw),
                                read_hour: read.map(|&(_, hour)| hour),
                                slot: occupied.0,
                                country_id: occupied.1,
                                name_hour: tag.map(|_| at),
                            },
                        )
                    })
                    .collect();
                entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
                if names.index.names.contains_key(&identity) {
                    sample.fact = Some(Fact::new(serde_json::to_vec(&OccupationTrendRun {
                        first_hour: at,
                        last_hour: at,
                        read_hour: at,
                        starts_segment: false,
                        resistance_raw: resistance.to_string(),
                        compliance_raw: compliance.to_string(),
                        states: sample.states,
                        held_states: sample.held_states,
                        held_complete: sample.held_complete,
                        collaboration: entries
                            .iter()
                            .map(|(tag, entry)| entry.shown(tag.cloned()))
                            .collect(),
                    })?)?);
                }
                sample.collaboration = Some(entries.into_iter().map(|(_, entry)| entry).collect());
            }
            samples.push(sample);
        }
        Ok(samples)
    }

    fn read(
        &mut self,
        hour: &segments::Hour<'_, World>,
        countries: &Countries,
    ) -> io::Result<Read> {
        let _cpu = ::replay_frame::cpu::scope("occupation projection read", None);
        let reset = !self.evaluated;
        let garrisons = slot_read(
            self.format,
            &metrics(hour.manifest),
            TableId::StateGarrisons,
            hour.hour,
        );
        let mut reached = BTreeSet::new();
        if reset {
            self.pending.extend(self.resistance.rows.keys().copied());
            self.contributions.clear();
            self.aggregates.clear();
            self.totals.clear();
            self.membership_dirty.clear();
            self.bad_controllers.clear();
            self.missing_states.clear();
            self.incomplete.clear();
            self.garrisons_read = None;
        }
        // Check mode reads every state again, so a law a missed declaration kept is evaluated.
        if dashboard_text::triggers::inputs::check_reuse() {
            self.pending.extend(self.resistance.rows.keys().copied());
        }
        if garrisons {
            let inventory = self.inventory(countries);
            self.membership_dirty.extend(inventory.iter().copied());
            reached.extend(inventory);
        }
        let dirty = std::mem::take(&mut self.pending);
        let resistance: Vec<_> = dirty
            .iter()
            .filter_map(|key| self.resistance.rows.get(key).cloned())
            .collect();
        let states: Vec<_> = dirty
            .iter()
            .filter_map(|key| self.states.rows.get(key).cloned())
            .collect();
        let before: BTreeMap<_, _> = dirty
            .iter()
            .map(|&key| {
                (
                    key as u32,
                    self.contributions
                        .get(&(key as u32))
                        .map(|row| (row.controller, row.occupied)),
                )
            })
            .collect();
        for &key in &dirty {
            self.remove(key as u32, &mut reached);
        }
        for row in &resistance {
            self.add(
                row,
                self.states
                    .rows
                    .get(&u128::from(row.state))
                    .cloned()
                    .as_ref(),
                &mut reached,
            );
        }
        for (id, before) in before {
            let after = self
                .contributions
                .get(&id)
                .map(|row| (row.controller, row.occupied));
            if before != after {
                self.membership_dirty.extend(before.map(|row| row.0));
                self.membership_dirty.extend(after.map(|row| row.0));
            }
        }
        let own = self.frame.as_ref().expect("occupation inputs");
        self.joins.refuse(own, &mut self.scan.refused)?;
        recorded_rows::<tables::StateResistance>(own)?;
        let mut scan = std::mem::take(&mut self.scan).bind(self.catalogue.as_deref());
        scan.countries = Arc::clone(&countries.index);
        scan.result.states.clear();
        scan.positions.clear();
        // A law reading a table the hour's frame lacks reads every trigger table, asked once.
        let (tables, observed) = (&self.tables, &self.observed);
        let holds = |table: TableId| {
            tables.binary_search(&table).is_ok() || observed.binary_search(&table).is_err()
        };
        let failed = Cell::new(None);
        let whole = OnceCell::new();
        let every = || {
            whole
                .get_or_init(|| {
                    let _cpu = ::replay_frame::cpu::scope("occupation world frame", None);
                    hour.world.frame(hour.state, observed).map_err(|error| {
                        let reason = error.to_string();
                        failed.set(Some(error));
                        reason
                    })
                })
                .as_ref()
                .map_err(Clone::clone)
        };
        let built = || whole.get().is_some();
        scan.read_values(
            eligibility::Frames {
                hour: own,
                holds: &holds,
                every: &every,
                built: &built,
            },
            hour.hour,
            &states,
            &countries.links,
            &resistance,
            &self.joins,
            false,
        )?;
        if let Some(error) = failed.take() {
            return Err(error);
        }
        let mut values: BTreeMap<_, _> = dirty.into_iter().map(|key| (key as u32, None)).collect();
        for state in std::mem::take(&mut scan.result.states) {
            let value = &state.runs.last().expect("evaluated state").value;
            if value.occupier.is_none() || value.occupied.is_none() {
                self.incomplete.insert(state.state);
            }
            values.insert(state.state, Some(StateValue::of(value, &mut self.named)));
        }
        self.scan = scan.bind(None);
        if self.garrisons_read != Some(garrisons) || reset {
            reached.extend(self.aggregates.keys().copied());
            reached.extend(self.inventory.keys().copied());
            self.membership_dirty.extend(reached.iter().copied());
        }
        self.garrisons_read = Some(garrisons);
        self.evaluated = true;
        for identity in reached {
            self.total(identity, garrisons);
        }
        self.membership_dirty.clear();
        let mut trend_refused = std::mem::take(&mut self.trend_refused);
        let mut collaboration_refused = std::mem::take(&mut self.collaboration_refused);
        if garrisons {
            trend_refused.merge(&self.inventory_refused);
            collaboration_refused.merge(&self.inventory_collaboration_refused);
        }
        let samples = self.samples(
            hour.hour,
            countries,
            garrisons && self.inventory_complete && self.bad_controllers.is_empty(),
            &mut trend_refused,
        )?;
        Ok(Read {
            reset,
            values: values.into_iter().collect(),
            complete: columns::complete(hour.state, TableId::StateResistance)
                && self.missing_states.is_empty()
                && self.incomplete.is_empty(),
            samples,
            refused: std::mem::take(&mut self.scan.refused),
            trend_refused,
            collaboration_refused,
            own: catalogue::OwnRefusals::default(),
        })
    }

    /// The hour's collaboration read, if it reads it or finds it unavailable.
    fn collaboration(
        &mut self,
        hour: &segments::Hour<'_, World>,
        timing: &replay_frame::frame::Metrics,
        names: &Countries,
    ) -> io::Result<Option<Collaboration>> {
        if columns::unavailable(hour.state, TableId::CountryCollaboration) {
            self.progress = Some(Arc::default());
            return Ok(Some(Collaboration::Unavailable));
        }
        if !slot_read(
            self.format,
            timing,
            TableId::CountryCollaboration,
            hour.hour,
        ) {
            return Ok(None);
        }
        let identity = |raw| {
            let slot = link_slot(&names.links, raw)?;
            Some((slot, *names.index.identities.get(&slot)?))
        };
        let mut progress = Progress::new();
        let mut refused = Refusals::default();
        for row in recorded_rows::<tables::CountryCollaboration>(
            self.frame.as_ref().expect("occupation inputs"),
        )? {
            if let Some(key) = identity(row.source).zip(identity(row.target)) {
                progress.insert(key, (row.progress, hour.hour));
            } else {
                refused.note(
                    "CountryCollaboration link is unresolved",
                    format!("{} → {}", row.source, row.target),
                );
            }
        }
        let progress = Arc::new(progress);
        self.progress = Some(Arc::clone(&progress));
        Ok(Some(Collaboration::Read(progress, refused)))
    }
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("occupation projection", None);
        let version = self.format.version();
        let whole = hour.starts || hour.opens;
        self.grow();
        let roster = hour.world.countries()?;
        let naming_changed = self.naming.is_none() || whole || roster.changed();
        if naming_changed {
            let countries = self.countries(roster)?;
            self.charged +=
                size_of::<Naming>() + countries.naming.tags.len() * size_of::<(Identity, u32)>();
            self.naming = Some(countries);
        }
        let names = Arc::clone(self.naming.as_ref().expect("country inputs"));
        let state_fields = self.state_fields;
        let resistance_fields = self.resistance_fields;
        let state_edits = self.states.update(
            hour.state,
            TableId::States,
            whole,
            self.state_mask,
            |slot| state_row(hour.state, state_fields, slot),
        )?;
        self.pending.extend(state_edits.iter().copied());
        self.pending.extend(self.resistance.update(
            hour.state,
            TableId::StateResistance,
            whole,
            self.resistance_mask,
            |slot| resistance_row(hour.state, resistance_fields, slot),
        )?);
        let valid = !columns::unavailable(hour.state, TableId::StateResistance)
            && [TableId::States, TableId::Countries, TableId::CountryLinks]
                .iter()
                .all(|&table| columns::complete(hour.state, table));
        let continues = self.last.is_some_and(|(branch, at, _)| {
            branch == hour.branch && at.checked_add(1) == Some(hour.hour)
        });
        let interrupted = !continues || self.last.is_some_and(|(_, _, before)| before != valid);
        if !valid || interrupted {
            self.evaluated = false;
            self.scan.eligibility.clear();
        }
        // The emit drops its progress at a gap or an hour it cannot read.
        if !valid || (self.last.is_some() && !continues) {
            self.progress = Some(Arc::default());
        }
        self.last = Some((hour.branch, hour.hour, valid));
        let edits = Changes::new(hour, self.format, &self.observed, &mut self.shapes);
        self.frame = Some(hour.world.frame(hour.state, &self.tables)?);
        let changed = self.joins.update(hour, version, &names, naming_changed)?;
        self.pending.extend(changed.states);
        if changed.all || naming_changed || interrupted {
            self.pending.extend(self.resistance.rows.keys().copied());
            self.pending
                .extend(self.contributions.keys().map(|&id| u128::from(id)));
            self.pending
                .extend(self.missing_states.iter().map(|&id| u128::from(id)));
            self.pending
                .extend(self.bad_controllers.iter().map(|&id| u128::from(id)));
        }
        for country in changed.countries {
            if let Some(&id) = names.index.identities.get(&country)
                && let Some(total) = self.aggregates.get(&(country, id))
            {
                self.pending
                    .extend(total.states.iter().map(|&state| u128::from(state)));
            }
        }
        self.inventory_dirty.extend(changed.inventory);
        self.inventory_all |= changed.inventory_all || naming_changed;
        if valid {
            self.pending.extend(
                self.scan
                    .eligibility
                    .step(self.frame.as_ref().expect("occupation inputs"), &edits),
            );
        }
        drop(edits);
        let timing = metrics(hour.manifest);
        let collaboration = if valid {
            self.collaboration(hour, &timing, &names)?
        } else {
            None
        };
        let reads = valid
            && (stored_resistance_read(self.format, &timing, hour.hour)
                || hourly_resistance(self.format)
                    && columns::complete(hour.state, TableId::StateResistance)
                    && hour.state.sampled_at(TableId::StateResistance.index()) == Some(hour.hour));
        let read = if reads {
            let (read, own) = catalogue::capture_isolated(|| self.read(hour, &names));
            let mut read = read?;
            read.own = own;
            Some(read)
        } else {
            None
        };
        // A changed naming changes every owner the emit projects.
        let all = whole || naming_changed;
        let keys: Vec<u128> = if all {
            self.states.rows.keys().copied().collect()
        } else {
            state_edits.into_iter().collect()
        };
        let cat = self.catalogue.as_deref();
        let states =
            keys.into_iter()
                .map(|key| {
                    let row =
                        self.states.rows.get(&key).map(|row| {
                            shipped(row, &names, &mut self.named, cat, &self.scan.names)
                        });
                    (key as u32, row)
                })
                .collect();
        // A frame held into the next hour would have its views copied.
        self.frame = None;
        out.push(Hour {
            branch: hour.branch,
            hour: hour.hour,
            valid,
            naming: Arc::clone(&names.naming),
            whole: all,
            states,
            read,
            collaboration,
            shared: std::mem::take(&mut self.charged),
        })
    }
}
