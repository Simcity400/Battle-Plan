//! The place-names family's hours from a columnar walk ([`replay::segments`]): each province's
//! state and rename, each state's names and its owner's original-or-own tag, and how the name
//! tables hold them, as [`FrameInputs`] reads them from the walk's frame of those tables. Only
//! the provinces and states an hour's rows may have changed are read again, and only those
//! that changed since the segment last sent them leave the projection. A segment's first hour,
//! an hour after a skipped one and an hour a name table's status changed read every one.

use dashboard_base::products::place_names::{FrameInputs, Inputs, StateInputs};
use dashboard_world::{Hold, Needs, World};
use replay::{
    columns,
    segments::{self, Item, Operator, Sink, State},
};
use replay_data::rows::TableId;
use std::{collections::HashMap, io, mem::size_of};

/// The tables names are read from, and the fields of each they read.
const FIELDS: [(TableId, &[&str]); 7] = [
    (TableId::Provinces, &["state"]),
    (TableId::ProvinceNames, &["name"]),
    (TableId::States, &["owner", "custom_name", "owner_name"]),
    (TableId::Countries, &["canonical_slot", "tag"]),
    (TableId::CountryLinks, &["canonical_slot"]),
    (
        TableId::CountryAppearance,
        &["original_tag", "original_tag_reference"],
    ),
    (
        TableId::ReservedCountryNaming,
        &["tag", "original_tag", "original_tag_reference"],
    ),
];

/// Where each table is in [`FIELDS`].
mod at {
    pub const PROVINCES: usize = 0;
    pub const RENAMES: usize = 1;
    pub const STATES: usize = 2;
    /// The first of the tables owners' tags are read from.
    pub const COUNTRIES: usize = 3;
}

/// The tables the family reads.
pub const TABLES: &[TableId] = &[
    TableId::Provinces,
    TableId::ProvinceNames,
    TableId::States,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
];

fn mask(table: TableId, names: &[&str]) -> io::Result<u64> {
    names.iter().try_fold(0, |mask, name| {
        Ok(mask | 1_u64 << columns::field(table, name)?)
    })
}

fn asks() -> io::Result<Vec<(TableId, Hold)>> {
    FIELDS
        .iter()
        .map(|&(table, names)| Ok((table, Hold::Fields(mask(table, names)?).of(table))))
        .collect()
}

/// What a projection reads of its walk's world: its frame of the name tables.
pub fn needs() -> io::Result<Needs> {
    Ok(Needs {
        frame: asks()?,
        ..Needs::default()
    })
}

/// A province's inputs: its state, zero for none, and its ProvinceNames name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Province {
    pub state: u32,
    pub rename: Option<Box<[u8]>>,
}

impl Province {
    fn heap(&self) -> usize {
        self.rename.as_ref().map_or(0, |rename| rename.len())
    }
}

/// How the name tables hold their rows, as the readers word it, and slot zero's tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tables {
    pub renames_complete: bool,
    pub renames_gap: String,
    pub states_unrecorded: Option<String>,
    pub states_gap: String,
    pub reserved: Result<String, String>,
}

impl Tables {
    fn of(inputs: &impl Inputs) -> Self {
        Self {
            renames_complete: inputs.renames_complete(),
            renames_gap: inputs.renames_gap().to_owned(),
            states_unrecorded: inputs.states_unrecorded(),
            states_gap: inputs.states_gap().to_owned(),
            reserved: inputs
                .reserved_tag()
                .map(str::to_owned)
                .map_err(str::to_owned),
        }
    }

    fn heap(&self) -> usize {
        self.renames_gap.capacity()
            + self.states_unrecorded.as_ref().map_or(0, String::capacity)
            + self.states_gap.capacity()
            + match &self.reserved {
                Ok(text) | Err(text) => text.capacity(),
            }
    }
}

pub fn state_heap(state: &StateInputs) -> usize {
    state.custom.capacity()
        + state.owner_name.capacity()
        + match &state.owner {
            dashboard_base::products::place_names::Owner::Tag(Ok(text) | Err(text)) => {
                text.capacity()
            }
            dashboard_base::products::place_names::Owner::Unlinked(_) => 0,
        }
}

/// A province or state whose inputs the hour sends: none where it has no row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Province(u32, Option<Province>),
    State(u32, Option<StateInputs>),
}

/// One projected hour.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Whether `changes` holds every province and state with a row: a province or state it
    /// leaves out has none.
    pub whole: bool,
    /// The tables' holds, when they changed since the segment last sent them.
    pub tables: Option<Tables>,
    pub changes: Vec<Change>,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        self.tables.as_ref().map_or(0, Tables::heap)
            + self.changes.capacity() * size_of::<Change>()
            + self
                .changes
                .iter()
                .map(|change| match change {
                    Change::Province(_, province) => province.as_ref().map_or(0, Province::heap),
                    Change::State(_, state) => state.as_ref().map_or(0, state_heap),
                })
                .sum::<usize>()
    }
}

/// The fields each table's changes are read by.
struct Masks {
    provinces: u64,
    states: u64,
    countries: [u64; 4],
}

/// One segment's projection.
pub struct Projector {
    asks: Vec<(TableId, Hold)>,
    masks: Masks,
    cold: bool,
    last: Option<i32>,
    statuses: Vec<columns::Status>,
    provinces: HashMap<u32, Province>,
    states: HashMap<u32, StateInputs>,
    tables: Option<Tables>,
}

const SAVED: &[u8] = b"place-names projector 1";

impl Projector {
    pub fn new() -> io::Result<Self> {
        let field = |at: usize| mask(FIELDS[at].0, FIELDS[at].1);
        Ok(Self {
            asks: asks()?,
            masks: Masks {
                provinces: field(at::PROVINCES)?,
                states: field(at::STATES)?,
                countries: [
                    field(at::COUNTRIES)?,
                    field(at::COUNTRIES + 1)?,
                    field(at::COUNTRIES + 2)?,
                    field(at::COUNTRIES + 3)?,
                ],
            },
            cold: true,
            last: None,
            statuses: Vec::new(),
            provinces: HashMap::new(),
            states: HashMap::new(),
            tables: None,
        })
    }

    fn heap(&self) -> usize {
        self.provinces.capacity() * (size_of::<(u32, Province)>() + 1)
            + self.provinces.values().map(Province::heap).sum::<usize>()
            + self.states.capacity() * (size_of::<(u32, StateInputs)>() + 1)
            + self.states.values().map(state_heap).sum::<usize>()
            + self.tables.as_ref().map_or(0, Tables::heap)
            + self.asks.capacity() * size_of::<(TableId, Hold)>()
            + self.statuses.capacity() * size_of::<columns::Status>()
    }
}

/// The ids of the rows `table`'s changes wrote any of `mask` at, inserted or deleted.
fn touched(state: &State, table: TableId, mask: u64, out: &mut Vec<u32>) -> io::Result<()> {
    let held = columns::table(state, table)?;
    let changes = columns::changes(state, table)?;
    let written = changes
        .changed
        .iter()
        .zip(&changes.masks)
        .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
        .filter(|&(_, &written)| written & mask != 0)
        .map(|(&slot, _)| slot)
        .chain(changes.inserted.iter().copied());
    for slot in written {
        if let Some(key) = held.key(slot) {
            out.push(key as u32);
        }
    }
    out.extend(changes.deleted_keys.iter().map(|&key| key as u32));
    Ok(())
}

/// Whether `table`'s changes wrote any of `mask`, inserted or deleted a row.
fn written(state: &State, table: TableId, mask: u64) -> io::Result<bool> {
    let changes = columns::changes(state, table)?;
    Ok(!changes.inserted.is_empty()
        || !changes.deleted.is_empty()
        || changes.masks.iter().any(|&written| written & mask != 0)
        || changes.fill_masks.iter().any(|&written| written & mask != 0))
}

// Every segment starts cold: its first hour reads every row again; what came before is the
// emit's.
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
        let _cpu = replay_frame::cpu::scope("place_names.columnar", Some("place_names"));
        let state = hour.state;
        let statuses: Vec<columns::Status> = TABLES
            .iter()
            .map(|&table| columns::status(state, table))
            .collect();
        let skipped = self
            .last
            .is_some_and(|last| last.checked_add(1) != Some(hour.hour));
        self.last = Some(hour.hour);
        // A truncated read can show other rows under an equal status, with no row deleted.
        let truncated = statuses
            .iter()
            .any(|status| matches!(status, columns::Status::Truncated { .. }));
        let whole = self.cold || hour.starts || skipped || truncated || self.statuses != statuses;
        self.cold = false;
        self.statuses = statuses;
        let (mut provinces, mut states) = (Vec::new(), Vec::new());
        let mut retag = false;
        if !whole {
            touched(state, TableId::Provinces, self.masks.provinces, &mut provinces)?;
            touched(state, TableId::ProvinceNames, u64::MAX, &mut provinces)?;
            touched(state, TableId::States, self.masks.states, &mut states)?;
            for (at, &mask) in self.masks.countries.iter().enumerate() {
                retag |= written(state, TABLES[at::COUNTRIES + at], mask)?;
            }
            // Slot zero's tag refuses an unread naming row in the hour's own words.
            retag |= columns::status(state, TableId::ReservedCountryNaming)
                != columns::Status::Complete;
            if provinces.is_empty() && states.is_empty() && !retag {
                out.push(Hour {
                    branch: hour.branch,
                    hour: hour.hour,
                    whole: false,
                    tables: None,
                    changes: Vec::new(),
                })?;
                return Ok(());
            }
        }
        let frame = hour.world.frame_held(state, &self.asks)?;
        let inputs = FrameInputs::new(&frame);
        let tables = Tables::of(&inputs);
        let tables = (whole || self.tables.as_ref() != Some(&tables)).then(|| {
            self.tables = Some(tables.clone());
            tables
        });
        if whole {
            self.provinces.clear();
            self.states.clear();
            provinces.extend(inputs.provinces().keys().copied());
            states.extend(inputs.state_ids());
        } else if retag {
            states.extend(inputs.state_ids());
        }
        provinces.sort_unstable();
        provinces.dedup();
        states.sort_unstable();
        states.dedup();
        let mut changes = Vec::new();
        for id in provinces {
            let now = inputs.province(id).map(|state| Province {
                state,
                rename: inputs.rename(id).map(Box::from),
            });
            if self.provinces.get(&id) == now.as_ref() && !whole {
                continue;
            }
            match &now {
                Some(now) => self.provinces.insert(id, now.clone()),
                None => self.provinces.remove(&id),
            };
            changes.push(Change::Province(id, now));
        }
        for id in states {
            let now = inputs.state(id).map(|state| state.into_owned());
            if self.states.get(&id) == now.as_ref() && !whole {
                continue;
            }
            match &now {
                Some(now) => self.states.insert(id, now.clone()),
                None => self.states.remove(&id),
            };
            changes.push(Change::State(id, now));
        }
        out.push(Hour {
            branch: hour.branch,
            hour: hour.hour,
            whole,
            tables,
            changes,
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
            return Err(dashboard_core::invalid("Not a saved place-names projector"));
        }
        *self = Self::new()?;
        Ok(())
    }
}
