//! Peace facts, projected hour by hour from a columnar walk ([`replay::segments`]): the
//! segment's operator reads the peace tables an hour may have changed as the observer does
//! ([`super::reads::Reader`]) and hands on only what those reads give otherwise than the
//! segment's reads before: an owner, a war pair, a victory-point total, a finalizer row first
//! read. A segment's first hour and an hour after one not walked read every table again and
//! hand each read on whole; an hour that changed no read hands on nothing, and the emit
//! ([`super::emit`]) steps through it on the reads it holds.

use super::reads::{HourReads, Reader};
use dashboard_text::metadata::slot_read;
use dashboard_world::{Hold, Needs, World};
use replay::{
    columns,
    segments::{self, Operator, Sink},
};
use replay_data::{
    format::FormatVersion,
    rows::{TABLE_COUNT, TableId},
};
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

/// One hour of a segment.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Whether the conference roster's slot was read.
    pub read: bool,
    /// What the hour's reads give otherwise than the reads before them; none when it changed
    /// none and follows the segment's hour before.
    pub(super) reads: Option<Box<HourReads>>,
}

impl Hour {
    /// Whether the hour handed its reads on.
    pub fn observed(&self) -> bool {
        self.reads.is_some()
    }
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: FormatVersion,
    asks: Vec<(TableId, Hold)>,
    /// The fields of each table the observer reads, as change-mask bits: every bit for a
    /// table read whole.
    masks: Vec<(TableId, u64)>,
    /// The asks of the hour's changed tables, kept from hour to hour.
    moved: Vec<(TableId, Hold)>,
    /// Nothing is held: the next hour reads every table, as a segment's first.
    cold: bool,
    last: Option<i32>,
    reader: Reader,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since its next hour reads
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
            moved: Vec::with_capacity(asks.len()),
            asks,
            masks,
            cold: true,
            last: None,
            reader: Reader::default(),
        })
    }

    fn restart(&mut self) {
        self.cold = true;
        self.last = None;
        self.reader = Reader::default();
    }
}

// Every segment starts cold: its first hour hands every read on; what came before is the emit's.
impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, _: segments::Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.restart();
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
        let mut changed = [false; TABLE_COUNT];
        for &(table, mask) in &self.masks {
            changed[table.index()] = opens || world.touched(table, mask);
        }
        // A finalizer row first read names its actions' tokens in the hour's dictionary.
        let dictionary =
            changed[TableId::LexicalTokens.index()] || changed[TableId::PeaceSettlements.index()];
        // Country tags resolve from both identity tables: either moving asks for the two.
        let identity =
            changed[TableId::Countries.index()] || changed[TableId::CountryLinks.index()];
        self.moved.clear();
        self.moved.extend(self.asks.iter().copied().filter(|&(table, _)| {
            changed[table.index()]
                || (table == TableId::LexicalTokens && dictionary)
                || (identity && matches!(table, TableId::Countries | TableId::CountryLinks))
        }));
        let read = slot_read(self.format, world.metrics(), TableId::PeaceConferences, at);
        let reads = if self.moved.is_empty() {
            None
        } else {
            let frame = world.columns(hour.state);
            let reads = self.reader.read(&frame, &changed)?;
            (opens || !reads.is_empty()).then(|| Box::new(reads))
        };
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            read,
            reads,
        })
    }

    fn carried(&self) -> Option<usize> {
        Some(
            self.asks.capacity() * size_of::<(TableId, Hold)>()
                + self.moved.capacity() * size_of::<(TableId, Hold)>()
                + self.masks.capacity() * size_of::<(TableId, u64)>()
                + self.reader.bytes(),
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
        self.restart();
        Ok(())
    }
}
