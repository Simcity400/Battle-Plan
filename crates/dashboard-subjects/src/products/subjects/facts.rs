//! Subject facts, projected hour by hour from a columnar walk ([`replay::segments`]): what the
//! subject scan reads of each table ([`Inputs`]), where the hour changed it. The countries' keys
//! come from the walk's countries ([`dashboard_world`]) where a country's slot or tag or a link
//! changed; the relations where a relation was added or removed or its start, end or cancel byte
//! written; the levels where a subject was added or removed or its autonomy state written. A
//! table whose status changed is read again, and a segment's first hour reads every table: the
//! emit's scan finds what did not change. An hour that changes nothing carries nothing.

use super::scan::{Input, Inputs, Relation};
use dashboard_text::metadata::{CountryKeys, link_slots};
use dashboard_world::{Needs, World};
use replay::{
    columns,
    segments::{self, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{TableId, TableRows},
};
use std::io;

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Subjects,
    TableId::Autonomy,
];

/// What a projection reads of its walk's world: the countries.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

/// One hour of a segment: none where it changes nothing the scan reads.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    pub inputs: Option<Box<Inputs>>,
}

/// The fields of `table` named `names`, as change-mask bits.
fn mask(table: TableId, names: &[&str]) -> io::Result<u64> {
    names.iter().try_fold(0, |mask, name| {
        let field = columns::field(table, name)?;
        Ok(mask | 1_u64.checked_shl(field as u32).unwrap_or(0))
    })
}

/// Whether the hour added, removed or truncated rows of `table`, or set any of `fields` in one.
fn edited(state: &State, table: TableId, fields: u64) -> io::Result<bool> {
    let changes = columns::changes(state, table)?;
    Ok(changes.truncated
        || !changes.inserted.is_empty()
        || !changes.deleted.is_empty()
        || changes
            .written
            .iter()
            .chain(&changes.fill_masks)
            .any(|written| written & fields != 0))
}

/// Every live row of `table`, in key order.
fn live_rows(state: &State, table: TableId, format: u16) -> io::Result<TableRows> {
    let live: Vec<u32> = columns::table(state, table)?.live().ones().collect();
    columns::rows_at(state, table, &live, format)
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: u16,
    /// A relation's start, end and cancel byte; a subject's autonomy state.
    relation_fields: u64,
    level_fields: u64,
    /// Nothing is held: the next hour reads every table, as a segment's first.
    cold: bool,
    last: Option<i32>,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since the next hour reads
/// every table again, as at a segment's first hour.
const SAVED: &[u8] = b"subjects projector: cold";

impl Projector {
    pub fn new(format: FormatVersion) -> io::Result<Self> {
        Ok(Self {
            format: format.version(),
            relation_fields: mask(TableId::Subjects, &["start_hour", "end_hour", "cancel"])?,
            level_fields: mask(TableId::Autonomy, &["state"])?,
            cold: true,
            last: None,
        })
    }

    fn relations(&self, state: &State) -> io::Result<Vec<Relation>> {
        let TableRows::Subjects(rows) = live_rows(state, TableId::Subjects, self.format)? else {
            return Err(io::Error::other("Subjects rows of another table"));
        };
        Ok(rows
            .into_iter()
            .map(|row| Relation {
                overlord: row.overlord,
                subject: row.subject,
                start_hour: row.start_hour,
                end_hour: row.end_hour,
                cancel: row.cancel,
            })
            .collect())
    }

    fn levels(&self, state: &State) -> io::Result<Vec<(u32, Vec<u8>)>> {
        let TableRows::Autonomy(rows) = live_rows(state, TableId::Autonomy, self.format)? else {
            return Err(io::Error::other("Autonomy rows of another table"));
        };
        Ok(rows.into_iter().map(|row| (row.subject, row.state)).collect())
    }

    /// `table` read anew where `changed`: its rows by `read` when it is complete.
    fn table<T>(
        state: &State,
        table: TableId,
        changed: bool,
        read: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<Input<T>> {
        Ok(if !changed {
            Input::Same
        } else if columns::complete(state, table) {
            Input::Read(read()?)
        } else {
            Input::Incomplete
        })
    }
}

// Every segment starts cold: its first hour reads every table again; what came before is the
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
        let _cpu = replay_frame::cpu::scope("subjects.columnar", Some("subjects"));
        let state = hour.state;
        let at = hour.hour;
        let world = hour.world;
        let roster = world.countries()?;
        let opens = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        self.cold = false;
        self.last = Some(at);
        let countries = if opens || roster.rebuilt() || !roster.changed_ids().is_empty() {
            if roster.countries_complete() {
                Input::Read(CountryKeys::from_keys(
                    roster
                        .iter()
                        .map(|country| (country.id, country.slot, roster.tag(country.tag).to_vec()))
                        .collect(),
                ))
            } else {
                Input::Incomplete
            }
        } else {
            Input::Same
        };
        let links = if opens || roster.rebuilt() || !roster.changed_links().is_empty() {
            if roster.links_complete() {
                Input::Read(link_slots(roster.links()))
            } else {
                Input::Incomplete
            }
        } else {
            Input::Same
        };
        let subjects = TableId::Subjects;
        let relations = Self::table(
            state,
            subjects,
            opens
                || world.restated(subjects)
                || edited(state, subjects, self.relation_fields)?,
            || self.relations(state),
        )?;
        let autonomy = TableId::Autonomy;
        let levels = Self::table(
            state,
            autonomy,
            opens || world.restated(autonomy) || edited(state, autonomy, self.level_fields)?,
            || self.levels(state),
        )?;
        let inputs = Inputs {
            countries,
            links,
            relations,
            levels,
        };
        let quiet = [
            matches!(inputs.countries, Input::Same),
            matches!(inputs.links, Input::Same),
            matches!(inputs.relations, Input::Same),
            matches!(inputs.levels, Input::Same),
        ]
        .into_iter()
        .all(|same| same);
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            inputs: (!quiet).then(|| Box::new(inputs)),
        })
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(SAVED);
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes != SAVED {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a saved subjects projector",
            ));
        }
        self.cold = true;
        self.last = None;
        Ok(())
    }
}
