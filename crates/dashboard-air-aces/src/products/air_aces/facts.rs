//! Air aces' operator on a columnar walk ([`replay::segments`]). At each roster read it hands on
//! the aces the read gives otherwise than the segment's read before, each with what the read
//! joins to it ([`super::join`]), and those it no longer holds: every ace at the segment's first
//! read and after a read the roster could not give. At each hour it hands on the variant
//! catalogue's evidence and, at its reads, the operands of each design named anew ([`Naming`]).
//! No table reaches the emit ([`super::emit`]), which carries what crosses segments; every start
//! is the operator's own, so segments walk at once.

use super::{Sighting, duplicated, join};
use dashboard_base::{
    prepared::prepass::HeldBytes,
    products::equipment::{
        RawVariant, VariantHour,
        columnar::{Naming, slots},
    },
};
use dashboard_text::{
    catalogue::Refusals,
    metadata::{LinkTags, slot_reads},
};
use dashboard_world::{Hold, Needs, World};
use replay::{
    columns,
    segments::{self, Item, Operator, Sink},
};
use replay_data::{
    format::FormatVersion,
    rows::{TABLE_COUNT, TableId, tables},
};
use replay_frame::frame::{DecodedFrame, Metrics, not_selected};
use std::{
    collections::{HashMap, HashSet},
    io,
    mem::size_of,
};

/// The tables the roster and the variant catalogue read.
pub const TABLES: &[TableId] = &[
    TableId::AirAces,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::AirWings,
    TableId::WingEquipment,
    TableId::Raids,
    TableId::RaidTooltipOperands,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::VariantDefinitions,
    TableId::VariantModules,
    TableId::VariantUpgrades,
];

/// The roster's tables, each by the fields it reads; none named, by every field.
const ROSTER: [(TableId, &[&str]); 7] = [
    // A run holds while its ace's row is equal in every field.
    (TableId::AirAces, &[]),
    (TableId::Countries, &["canonical_slot", "tag"]),
    (TableId::CountryLinks, &[]),
    (TableId::AirWings, &["name"]),
    (TableId::WingEquipment, &["variant"]),
    // Legacy wing names come from the active raids' wing, phase and unit and their operands.
    (TableId::Raids, &["phase", "unit", "air_wing"]),
    (TableId::RaidTooltipOperands, &[]),
];

/// The tables variant names are read from, whose status a read follows.
const NAMED: [TableId; 7] = [
    TableId::VariantDefinitions,
    TableId::VariantModules,
    TableId::VariantUpgrades,
    TableId::LexicalTokens,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
];

/// How the walk's frame is asked for the roster's tables.
fn asks() -> io::Result<Vec<(TableId, Hold)>> {
    ROSTER
        .iter()
        .map(|&(table, fields)| {
            if fields.is_empty() {
                return Ok((table, Hold::Whole));
            }
            let mut mask = 0_u64;
            for name in fields {
                mask |= 1_u64 << columns::field(table, name)?;
            }
            Ok((table, Hold::Fields(mask).of(table)))
        })
        .collect()
}

/// What the operators read of the walk's world.
pub fn needs() -> io::Result<Needs> {
    Ok(Needs {
        countries: true,
        naming: true,
        frame: asks()?,
        ..Needs::default()
    })
}

/// What an hour gives the roster.
pub(crate) enum Roster {
    /// AirAces held every row, unread.
    Unread,
    /// AirAces did not hold every row: what was known ends.
    Unknown,
    /// A read holding an ace twice: a gap.
    Duplicate,
    /// A read giving every ace as the segment's read before did.
    Same,
    Read(Box<Read>),
}

/// A roster read: every ace when `whole`, else those it gives otherwise than the read before and
/// those it no longer holds.
pub(crate) struct Read {
    pub(crate) whole: bool,
    /// In roster order, which first sightings are numbered by.
    pub(crate) seen: Vec<Sighting>,
    /// Ascending.
    pub(crate) gone: Vec<u64>,
}

/// What a segment's roster reads refused: of their fields, and of their wings' names.
#[derive(Default)]
pub(crate) struct Noted {
    pub(crate) roster: Refusals,
    pub(crate) wings: Refusals,
}

/// One hour as a segment's operator projected it.
pub struct Hour {
    pub hour: i32,
    pub(crate) roster: Roster,
    pub(crate) variants: VariantHour,
    /// The operands of each design the hour's variant read named anew, by id ascending.
    pub(crate) built: Vec<(u64, RawVariant)>,
    /// Whether `built` holds every live design.
    pub(crate) named: bool,
    /// What the hour refused first in its segment.
    pub(crate) noted: Option<Box<Noted>>,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        let roster = match &self.roster {
            Roster::Read(read) => {
                size_of::<Read>()
                    + read.seen.capacity() * size_of::<Sighting>()
                    + read.seen.iter().map(Sighting::heap).sum::<usize>()
                    + read.gone.capacity() * size_of::<u64>()
            }
            _ => 0,
        };
        let mut built = HeldBytes::default();
        built.heap(&self.built);
        roster
            + built.total() as usize
            + self.noted.as_ref().map_or(0, |_| size_of::<Noted>())
    }
}

/// One segment's operator.
pub struct Project {
    format: FormatVersion,
    asks: Vec<(TableId, Hold)>,
    /// The frame the world's views are lent into for a read, given back before its hour ends.
    frame: DecodedFrame,
    naming: Naming,
    /// Each named table's status the hour before.
    shapes: [Option<columns::Status>; TABLE_COUNT],
    last: Option<i32>,
    /// A field the roster reads may have changed since the segment's last read.
    dirty: bool,
    /// The segment's last read, by ace: none before its first, and after a read the roster
    /// could not give.
    held: Option<HashMap<u64, Sighting>>,
    tags: LinkTags,
    /// What the segment refused so far, each handed on once.
    noted: Noted,
}

impl Project {
    pub fn new(format: FormatVersion) -> io::Result<Self> {
        Ok(Self {
            format,
            asks: asks()?,
            frame: DecodedFrame {
                branch: 0,
                hour: 0,
                tables: std::array::from_fn(|_| not_selected()),
                metrics: Metrics::default(),
            },
            naming: Naming::dated()?,
            shapes: [None; TABLE_COUNT],
            last: None,
            dirty: true,
            held: None,
            tags: LinkTags::default(),
            noted: Noted::default(),
        })
    }

    /// The roster read at the hour, and what it refused first.
    fn read(
        &mut self,
        state: &segments::State,
        world: &World,
    ) -> io::Result<(Roster, Option<Box<Noted>>)> {
        let Self {
            frame,
            asks,
            held,
            tags,
            noted,
            dirty,
            ..
        } = self;
        // The views go back before the hour ends, however it ends.
        let lent = Lent {
            frame,
            asks: asks.as_slice(),
        };
        world.frame_held_into(state, lent.asks, lent.frame)?;
        let frame: &DecodedFrame = &*lent.frame;
        let rows = frame.complete::<tables::AirAces>()?;
        *dirty = false;
        let previous = held.take();
        if duplicated(rows) {
            return Ok((Roster::Duplicate, None));
        }
        let mut refused = Noted::default();
        let mut seen = Vec::new();
        let read = match previous {
            None => {
                let mut aces = HashMap::with_capacity(rows.len());
                seen.reserve_exact(rows.len());
                join(
                    frame,
                    rows,
                    tags,
                    &mut refused.roster,
                    &mut refused.wings,
                    |ace| {
                        let owned = ace.owned();
                        aces.insert(ace.id(), owned.clone());
                        seen.push(owned);
                    },
                );
                *held = Some(aces);
                Read {
                    whole: true,
                    seen,
                    gone: Vec::new(),
                }
            }
            Some(mut aces) => {
                join(
                    frame,
                    rows,
                    tags,
                    &mut refused.roster,
                    &mut refused.wings,
                    |ace| match aces.get_mut(&ace.id()) {
                        Some(last) if ace.is(last) => {}
                        Some(last) => {
                            let owned = ace.owned();
                            last.clone_from(&owned);
                            seen.push(owned);
                        }
                        None => {
                            let owned = ace.owned();
                            aces.insert(ace.id(), owned.clone());
                            seen.push(owned);
                        }
                    },
                );
                // The read holds each of its aces once, so any more held are gone.
                let mut gone = Vec::new();
                if aces.len() > rows.len() {
                    let ids: HashSet<u64> = rows.iter().map(|row| row.id).collect();
                    aces.retain(|id, _| {
                        let kept = ids.contains(id);
                        if !kept {
                            gone.push(*id);
                        }
                        kept
                    });
                    gone.sort_unstable();
                }
                *held = Some(aces);
                Read {
                    whole: false,
                    seen,
                    gone,
                }
            }
        };
        let roster = noted.roster.merge_new(&refused.roster);
        let wings = noted.wings.merge_new(&refused.wings);
        let fresh = (!roster.is_empty() || !wings.is_empty())
            .then(|| Box::new(Noted { roster, wings }));
        Ok((Roster::Read(Box::new(read)), fresh))
    }
}

impl Operator<World> for Project {
    type Output = Hour;

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("air_aces.facts", None);
        let (state, world, at) = (hour.state, hour.world, hour.hour);
        let fresh = hour.starts || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        let whole = fresh || hour.opens;
        self.last = Some(at);
        if fresh {
            self.held = None;
        }
        let mut shaped = [false; TABLE_COUNT];
        let mut changed = [false; TABLE_COUNT];
        for table in NAMED {
            let index = table.index();
            let status = columns::status(state, table);
            shaped[index] = whole || self.shapes[index] != Some(status);
            changed[index] = shaped[index] || slots::relevant(state, table, u64::MAX)?;
            self.shapes[index] = Some(status);
        }
        let reads = slot_reads(self.format, world.metrics(), at);
        let variants = self.naming.read(
            state,
            (world.countries()?, world.naming()?),
            self.format,
            at,
            whole,
            &shaped,
            &changed,
            reads[TableId::VariantDefinitions.index()],
        )?;
        self.dirty |= whole
            || self.asks.iter().any(|&(table, hold)| {
                world.touched(
                    table,
                    match hold {
                        Hold::Whole => u64::MAX,
                        Hold::Fields(fields) => fields,
                    },
                )
            });
        let (roster, noted) = if !columns::complete(state, TableId::AirAces) {
            self.held = None;
            (Roster::Unknown, None)
        } else if !reads[TableId::AirAces.index()] {
            (Roster::Unread, None)
        } else if !self.dirty && self.held.is_some() {
            (Roster::Same, None)
        } else {
            self.read(state, world)?
        };
        out.push(Hour {
            hour: at,
            roster,
            variants: variants.evidence,
            built: variants.built,
            named: variants.whole,
            noted,
        })
    }
}

/// The world's views lent into `frame` for `asks`, given back when dropped.
struct Lent<'o> {
    frame: &'o mut DecodedFrame,
    asks: &'o [(TableId, Hold)],
}

impl Drop for Lent<'_> {
    fn drop(&mut self) {
        dashboard_world::release(self.frame, self.asks);
    }
}
