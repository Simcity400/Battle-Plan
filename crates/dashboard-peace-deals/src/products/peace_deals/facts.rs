//! Peace facts, projected hour by hour from a columnar walk ([`replay::segments`]): the walk's
//! frame of the peace tables, held in the fields the observer reads, at each hour one of those
//! fields changed, with which tables changed; and at every hour whether the conference roster's
//! slot was read. A segment's first hour, an hour after one not walked and an hour a table's
//! status changed hold every table changed. Every other hour holds the rows of the last hour
//! that held a frame, which the emit ([`super::emit`]) steps through without one.

use dashboard_base::prepared::{frame_changes::FrameChanges, prepass::HeldBytes};
use dashboard_text::metadata::slot_read;
use dashboard_world::{Hold, Needs, World};
use replay::{
    columns,
    segments::{self, Item, Operator, Sink},
};
use replay_data::{
    format::FormatVersion,
    rows::{TABLE_COUNT, TableId},
};
use replay_frame::frame::DecodedFrame;
use std::{io, mem::size_of};

/// The tables the observer reads, and the fields of each it reads; every field where none
/// are named.
const FIELDS: [(TableId, &[&str]); 13] = [
    // Country tags: the id (the row key), canonical slot and tag.
    (TableId::Countries, &["canonical_slot", "tag"]),
    // Every link, which the tags are resolved through.
    (TableId::CountryLinks, &[]),
    // Each state's owner (the id is the row key).
    (TableId::States, &["owner"]),
    // Victory points summed by state.
    (TableId::Provinces, &["state", "victory_points"]),
    // War operands: the pair (the row key), start and end.
    (TableId::WarRelations, &["start_hour", "end_hour"]),
    // The roster: each conference (the row key), its turn and whether it finalized.
    (TableId::PeaceConferences, &["current_turn", "finalized"]),
    // Every finalizer row.
    (TableId::PeaceSettlements, &[]),
    // Subject relations: the pair (the row key), start, end, cancel byte and level.
    (
        TableId::Subjects,
        &["start_hour", "end_hour", "cancel", "state"],
    ),
    // Each government's ruling party.
    (TableId::Politics, &["ruling_party"]),
    // Each party's token (the country and id are the row key).
    (TableId::PoliticalParties, &["token"]),
    // Each country's leader ideology.
    (TableId::CountryAppearance, &["leader_ideology"]),
    // Whether a country exists: its owned state count.
    (TableId::CountryCounterInputs, &["owned_state_count"]),
    // The state-taking keyword's spelling and every finalizer token's.
    (TableId::LexicalTokens, &[]),
];

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::States,
    TableId::Provinces,
    TableId::WarRelations,
    TableId::PeaceConferences,
    TableId::PeaceSettlements,
    TableId::Subjects,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::CountryAppearance,
    TableId::CountryCounterInputs,
    TableId::LexicalTokens,
];

/// Each table of [`FIELDS`] as the walk's frame holds it.
fn asks() -> io::Result<Vec<(TableId, Hold)>> {
    FIELDS
        .iter()
        .map(|&(table, names)| {
            if names.is_empty() {
                return Ok((table, Hold::Whole));
            }
            let mask = names.iter().try_fold(0_u64, |mask, name| {
                Ok::<_, io::Error>(mask | 1 << columns::field(table, name)?)
            })?;
            Ok((table, Hold::Fields(mask).of(table)))
        })
        .collect()
}

/// What a projection reads of its walk's world: its frame of the peace tables.
pub fn needs() -> io::Result<Needs> {
    Ok(Needs {
        frame: asks()?,
        ..Needs::default()
    })
}

/// An hour at which a field the observer reads changed: the walk's frame of the peace tables
/// and which of them changed.
pub struct Observed {
    pub frame: DecodedFrame,
    pub changes: FrameChanges,
    /// The views of the tables that changed, which this hour alone holds.
    held: usize,
}

/// One hour of a segment.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Whether the conference roster's slot was read.
    pub read: bool,
    /// None when no field the observer reads changed: the hour holds the last frame's rows.
    pub observed: Option<Box<Observed>>,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        self.observed
            .as_ref()
            .map_or(0, |observed| size_of::<Observed>() + observed.held)
    }
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: FormatVersion,
    asks: Vec<(TableId, Hold)>,
    /// The fields of each table the observer reads, as change-mask bits: every bit for a
    /// table read whole.
    masks: Vec<(TableId, u64)>,
    /// Nothing is held: the next hour holds every table changed, as a segment's first.
    cold: bool,
    last: Option<i32>,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since its next hour holds
/// every table again, as at a segment's first hour.
const SAVED: &[u8] = b"peace projector: cold";

impl Projector {
    pub fn new(format: FormatVersion) -> io::Result<Self> {
        let asks = asks()?;
        let masks = asks
            .iter()
            .map(|&(table, hold)| {
                (
                    table,
                    match hold {
                        Hold::Whole => u64::MAX,
                        Hold::Fields(fields) => fields,
                    },
                )
            })
            .collect();
        Ok(Self {
            format,
            asks,
            masks,
            cold: true,
            last: None,
        })
    }
}

// Every segment starts cold: its first hour holds every table changed; what came before is the
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
        let _cpu = replay_frame::cpu::scope("peace.columnar", Some("peace"));
        let at = hour.hour;
        let world = hour.world;
        let opens = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        self.cold = false;
        self.last = Some(at);
        let mut rows_changed = [false; TABLE_COUNT];
        let mut changed = opens;
        for &(table, mask) in &self.masks {
            let touched = opens || world.touched(table, mask);
            rows_changed[table.index()] = touched;
            changed |= touched;
        }
        let read = slot_read(self.format, world.metrics(), TableId::PeaceConferences, at);
        let observed = if changed {
            let frame = world.frame_held(hour.state, &self.asks)?;
            let mut held = HeldBytes::default();
            for &(table, _) in &self.asks {
                if rows_changed[table.index()] {
                    held.view(&frame.tables[table.index()]);
                }
            }
            let held = usize::try_from(held.total()).unwrap_or(usize::MAX);
            Some(Box::new(Observed {
                frame,
                changes: FrameChanges {
                    rows_changed,
                    table_read: [false; TABLE_COUNT],
                    slot_read: [false; TABLE_COUNT],
                    updates: None,
                },
                held,
            }))
        } else {
            None
        };
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            read,
            observed,
        })
    }

    fn carried(&self) -> Option<usize> {
        Some(
            self.asks.capacity() * size_of::<(TableId, Hold)>()
                + self.masks.capacity() * size_of::<(TableId, u64)>(),
        )
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(SAVED);
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes != SAVED {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a saved peace projector",
            ));
        }
        self.cold = true;
        self.last = None;
        Ok(())
    }
}
