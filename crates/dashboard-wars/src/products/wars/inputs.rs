//! What the wars publication holds of each hour's tables, as its operator, emit and queries
//! read and write it.
//!
//! A change input is published at an hour where a table the wars read changed in a field they
//! compare ([`same_inputs`]), at the first hour after one not walked, and where a captured
//! capitulation's date is reached: the views of the tables that changed, and, for every input
//! table, the ordinal of the last input that published it, so the tables at any hour are those
//! of a handful of inputs. War relations are published without their casualty counters, which
//! are series of their own.
//!
//! The participation ledgers and lend-lease histories change every hour of a war: they are
//! published as each hour's changed and deleted rows by key, whole at a segment's first hour and
//! every [`WHOLE_EVERY`] hours after.
//!
//! Views are packed as only the tables they hold ([`put_views`]), never as a whole frame: a
//! frame's hundreds of unselected tables would outweigh an hour's few rows.

use super::*;
use dashboard_base::prepared::series::{compress, decompress};
use replay_data::{
    codec::Decoder,
    rows::{RowCodec as _, TableSchema},
    scope::CaptureMode,
};
use replay_frame::frame::{SharedView, not_selected};
use std::collections::BTreeMap;

/// The tables a change input holds: every table the wars read but the clock, in this order.
pub(crate) const INPUTS: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::WarRelations,
    TableId::WarOwnedRelations,
    TableId::WarRelationGoals,
    TableId::WarOverviewCountries,
    TableId::WarNameRules,
    TableId::Factions,
    TableId::FactionMembers,
    TableId::LocalPlayer,
    TableId::Capitulation,
    TableId::LexicalTokens,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Autonomy,
    TableId::States,
];

// A set of input tables is one bit per place.
const _: () = assert!(INPUTS.len() <= 32);

/// The tables whose change makes the operator join the war relations again.
pub(crate) const JOIN: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::LocalPlayer,
    TableId::WarOwnedRelations,
    TableId::WarRelations,
    TableId::Factions,
    TableId::FactionMembers,
    TableId::WarNameRules,
    TableId::WarOverviewCountries,
];

/// The per-hour participation tables, which only a requested hour's contributions read.
pub(crate) const PARTICIPATION: [TableId; 2] =
    [TableId::WarParticipation, TableId::WarLendLeaseHistories];

/// A participation diff chain is published whole again after this many hours of it.
pub(crate) const WHOLE_EVERY: u32 = 512;

/// The compression level of what is packed: an hour's packet is small and packed on every
/// hour of a war.
const LEVEL: i32 = 1;

/// `table`'s place in [`INPUTS`].
pub(crate) fn input(table: TableId) -> Option<usize> {
    INPUTS.iter().position(|&held| held == table)
}

/// The [`INPUTS`] places of a set of input tables, one bit each.
pub(crate) fn places(set: u32) -> impl Iterator<Item = usize> {
    (0..INPUTS.len()).filter(move |&place| set & 1 << place != 0)
}

fn malformed() -> io::Error {
    invalid("Malformed wars publication input")
}

/// The first recorded capitulation date after `hour`: the hour at which an already captured
/// capitulation becomes usable.
pub(crate) fn next_capitulation(view: &SharedView, hour: i32) -> Option<i32> {
    let TableView::Complete(rows) = &**view else {
        return None;
    };
    rows.rows::<tables::Capitulation>()?
        .iter()
        .filter(|row| row.capitulated != 0 && row.capitulated_hour > hour)
        .map(|row| row.capitulated_hour)
        .min()
}

/// Equality after removing the casualty counters, without copying the rows. Truncated evidence
/// still compares its missing count.
pub(crate) fn same_relation_structure(a: &SharedView, b: &SharedView) -> bool {
    let (a, b) = match (&**a, &**b) {
        (TableView::Complete(a), TableView::Complete(b)) => (a, b),
        (
            TableView::Truncated {
                rows: a,
                missing: am,
            },
            TableView::Truncated {
                rows: b,
                missing: bm,
            },
        ) if am == bm => (a, b),
        (a, b) if !matches!(a, TableView::Complete(_) | TableView::Truncated { .. }) => {
            return a == b;
        }
        _ => return false,
    };
    let (rows::TableRows::WarRelations(a), rows::TableRows::WarRelations(b)) = (a, b) else {
        return false;
    };
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            (
                a.first_country,
                a.second_country,
                a.start_hour,
                a.end_hour,
                a.cancel,
                a.first_was_instigator,
            ) == (
                b.first_country,
                b.second_country,
                b.start_hour,
                b.end_hour,
                b.cancel,
                b.first_was_instigator,
            )
        })
}

/// `view` of WarRelations with every casualty counter zero, as change inputs hold it.
pub(crate) fn zeroed(view: &SharedView) -> SharedView {
    let mut view = view.clone();
    if let TableView::Complete(rows::TableRows::WarRelations(rows))
    | TableView::Truncated {
        rows: rows::TableRows::WarRelations(rows),
        ..
    } = &mut *view
    {
        for row in rows {
            row.first_casualties = 0;
            row.second_casualties = 0;
            row.first_unknown_casualties = 0;
            row.second_unknown_casualties = 0;
        }
    }
    view
}

/// Whether two views of an input `table` agree on every field the wars compare: the fields
/// their readers read.
pub(crate) fn same_inputs(table: TableId, a: &SharedView, b: &SharedView) -> bool {
    if SharedView::ptr_eq(a, b) {
        return true;
    }
    if table == TableId::WarRelations {
        return same_relation_structure(a, b);
    }
    if let (TableView::Complete(a), TableView::Complete(b)) = (&**a, &**b) {
        match (a, b) {
            (rows::TableRows::States(a), rows::TableRows::States(b)) => {
                return a.len() == b.len()
                    && a.iter().zip(b).all(|(a, b)| {
                        (a.id, &a.custom_name, &a.owner_name)
                            == (b.id, &b.custom_name, &b.owner_name)
                    });
            }
            (rows::TableRows::Capitulation(a), rows::TableRows::Capitulation(b)) => {
                return a.len() == b.len()
                    && a.iter().zip(b).all(|(a, b)| {
                        (a.country, a.capitulated, a.capitulated_hour)
                            == (b.country, b.capitulated, b.capitulated_hour)
                    });
            }
            (rows::TableRows::Factions(_), rows::TableRows::Factions(_)) => return true,
            _ => {}
        }
    }
    crate::products::war_casualties::same_name_rows(table, a, b)
}

/// What [`put_views`] puts of a table: a view, or rows put as a complete view of them would be.
enum Put<'v> {
    View(&'v SharedView),
    Rows(&'v rows::TableRows),
}

/// Appends `views`: their count, then each one's table, sample hour, kind and rows.
fn put_views<'v>(
    out: &mut Vec<u8>,
    views: impl IntoIterator<Item = (TableId, Put<'v>)>,
) -> io::Result<()> {
    let at = out.len();
    out.extend_from_slice(&0_u16.to_le_bytes());
    let mut count = 0_u16;
    for (table, put) in views {
        count = count.checked_add(1).ok_or_else(malformed)?;
        let index = u16::try_from(table.index()).map_err(|_| malformed())?;
        out.extend_from_slice(&index.to_le_bytes());
        let view = match put {
            Put::Rows(rows) => {
                // No sample hour, complete.
                out.extend_from_slice(&[0, 0]);
                rows.encode_checkpoint_rows(out)?;
                continue;
            }
            Put::View(view) => view,
        };
        match view.sampled_at() {
            None => out.push(0),
            Some(hour) => {
                out.push(1);
                out.extend_from_slice(&hour.to_le_bytes());
            }
        }
        match &**view {
            TableView::Complete(rows) => {
                out.push(0);
                rows.encode_checkpoint_rows(out)?;
            }
            TableView::Truncated { rows, missing } => {
                out.push(1);
                out.extend_from_slice(&missing.to_le_bytes());
                rows.encode_checkpoint_rows(out)?;
            }
            TableView::Unavailable { reason } => {
                out.push(2);
                let len = u32::try_from(reason.len()).map_err(|_| malformed())?;
                out.extend_from_slice(&len.to_le_bytes());
                out.extend_from_slice(reason.as_bytes());
            }
            TableView::NotRecorded { mode } => {
                out.push(3);
                out.push(mode.byte());
            }
        }
    }
    out[at..at + 2].copy_from_slice(&count.to_le_bytes());
    Ok(())
}

/// Reads the views [`put_views`] put, handing each to `view`.
fn get_views(
    input: &mut Decoder<'_>,
    view: &mut dyn FnMut(TableId, SharedView) -> io::Result<()>,
) -> io::Result<()> {
    for _ in 0..input.u16()? {
        let index = usize::from(input.u16()?);
        let table = TableId::ALL
            .get(index)
            .copied()
            .filter(|table| table.index() == index)
            .ok_or_else(malformed)?;
        let sample = match input.u8()? {
            0 => None,
            1 => Some(input.i32()?),
            _ => return Err(malformed()),
        };
        let read = match input.u8()? {
            0 => TableView::Complete(table.decode_checkpoint_rows(input)?),
            1 => {
                let missing = input.u32()?;
                TableView::Truncated {
                    rows: table.decode_checkpoint_rows(input)?,
                    missing,
                }
            }
            2 => TableView::Unavailable {
                reason: input.string()?,
            },
            3 => TableView::NotRecorded {
                mode: CaptureMode::from_byte(input.u8()?)?,
            },
            _ => return Err(malformed()),
        };
        // A sample hour is a complete acquisition's.
        if sample.is_some() && !matches!(read, TableView::Complete(_)) {
            return Err(malformed());
        }
        view(table, SharedView::from(read).with_sampled_at(sample))?;
    }
    Ok(())
}

/// The views of `tables`, packed and compressed; `scratch` holds them uncompressed between
/// calls, so no hour allocates it again.
pub(crate) fn pack<'v>(
    tables: impl IntoIterator<Item = (TableId, &'v SharedView)>,
    scratch: &mut Vec<u8>,
) -> io::Result<Vec<u8>> {
    scratch.clear();
    let views = tables
        .into_iter()
        .map(|(table, view)| (table, Put::View(view)));
    put_views(scratch, views)?;
    compress(scratch, LEVEL)
}

/// The frame of `branch` at `hour` holding the views [`pack`] packed, every other table not
/// selected.
pub(crate) fn unpack(packed: &[u8], branch: u64, hour: i32) -> io::Result<DecodedFrame> {
    let mut frame = DecodedFrame {
        branch,
        hour,
        metrics: Default::default(),
        tables: std::array::from_fn(|_| not_selected()),
    };
    let bytes = decompress(packed)?;
    let mut input = Decoder::new(&bytes);
    get_views(&mut input, &mut |table, view| {
        frame.tables[table.index()] = view;
        Ok(())
    })?;
    input.finish()?;
    Ok(frame)
}

/// A change input's event bytes before its packet: its ordinal, the tables of `changed` (bits
/// by [`INPUTS`] place) and, for every input table, the ordinal of the last input publishing it.
pub(crate) fn put_input(
    out: &mut Vec<u8>,
    ordinal: u32,
    changed: u32,
    refs: &[u32],
) -> io::Result<()> {
    if refs.len() != INPUTS.len() {
        return Err(malformed());
    }
    out.extend_from_slice(&ordinal.to_le_bytes());
    out.extend_from_slice(&(changed.count_ones() as u16).to_le_bytes());
    for place in places(changed) {
        out.extend_from_slice(&(INPUTS[place] as u16).to_le_bytes());
    }
    for &held in refs {
        out.extend_from_slice(&held.to_le_bytes());
    }
    Ok(())
}

/// A change input as its event holds it: its ordinal, the tables it changed, for every input
/// table the ordinal of the last input publishing it ([`u32::MAX`] for none), and its packed
/// views (empty when it changed none).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Input {
    pub ordinal: u32,
    pub changed: Vec<TableId>,
    pub refs: Vec<u32>,
    pub packet: Vec<u8>,
}

impl Input {
    /// The input [`put_input`] and its packet make.
    pub(crate) fn decode(bytes: &[u8]) -> io::Result<Self> {
        let (ordinal, rest) = bytes.split_first_chunk::<4>().ok_or_else(malformed)?;
        let (count, mut rest) = rest.split_first_chunk::<2>().ok_or_else(malformed)?;
        let mut changed = Vec::with_capacity(usize::from(u16::from_le_bytes(*count)));
        for _ in 0..u16::from_le_bytes(*count) {
            let (table, after) = rest.split_first_chunk::<2>().ok_or_else(malformed)?;
            let table = u16::from_le_bytes(*table);
            changed.push(
                TableId::ALL
                    .iter()
                    .copied()
                    .find(|&id| id as u16 == table && input(id).is_some())
                    .ok_or_else(malformed)?,
            );
            rest = after;
        }
        let mut refs = Vec::with_capacity(INPUTS.len());
        for _ in INPUTS {
            let (held, after) = rest.split_first_chunk::<4>().ok_or_else(malformed)?;
            refs.push(u32::from_le_bytes(*held));
            rest = after;
        }
        Ok(Self {
            ordinal: u32::from_le_bytes(*ordinal),
            changed,
            refs,
            packet: rest.to_vec(),
        })
    }

    /// Its packed views; a frame of no table when it changed none.
    pub(crate) fn frame(&self, branch: u64, hour: i32) -> io::Result<DecodedFrame> {
        if self.packet.is_empty() {
            return Ok(DecodedFrame {
                branch,
                hour,
                metrics: Default::default(),
                tables: std::array::from_fn(|_| not_selected()),
            });
        }
        unpack(&self.packet, branch, hour)
    }
}

/// One participation table at an hour: its whole view, or the rows it changed and the keys it
/// deleted since the hour before.
#[derive(Clone, Debug)]
pub(crate) enum Part {
    Whole(SharedView),
    Diff {
        upserts: rows::TableRows,
        deleted: Vec<u128>,
    },
}

/// An hour's participation parts, by [`PARTICIPATION`]: none where the table did not change.
pub(crate) type Parts = [Option<Part>; 2];

/// A [`Part`] as the operator packs it, borrowing the walk's rows.
pub(crate) enum PartRef<'a> {
    Whole(&'a SharedView),
    Diff {
        upserts: &'a rows::TableRows,
        deleted: &'a [u128],
    },
}

/// `parts`, compressed: each table's kind and deleted keys, then the whole views and the changed
/// rows. `scratch` holds them uncompressed between calls.
pub(crate) fn pack_parts(
    parts: &[Option<PartRef<'_>>; 2],
    scratch: &mut Vec<u8>,
) -> io::Result<Vec<u8>> {
    scratch.clear();
    for part in parts {
        match part {
            None => scratch.push(0),
            Some(PartRef::Whole(_)) => scratch.push(1),
            Some(PartRef::Diff { deleted, .. }) => {
                scratch.push(2);
                let count = u32::try_from(deleted.len()).map_err(|_| malformed())?;
                scratch.extend_from_slice(&count.to_le_bytes());
                for key in *deleted {
                    scratch.extend_from_slice(&key.to_le_bytes());
                }
            }
        }
    }
    let views = parts
        .iter()
        .zip(PARTICIPATION)
        .filter_map(|(part, table)| match part {
            None => None,
            Some(PartRef::Whole(view)) => Some((table, Put::View(view))),
            Some(PartRef::Diff { upserts, .. }) => Some((table, Put::Rows(upserts))),
        });
    put_views(scratch, views)?;
    compress(scratch, LEVEL)
}

/// The parts [`pack_parts`] packed.
pub(crate) fn unpack_parts(packed: &[u8]) -> io::Result<Parts> {
    let bytes = decompress(packed)?;
    let mut rest = &bytes[..];
    let mut kinds: [(u8, Vec<u128>); 2] = Default::default();
    for kind in &mut kinds {
        let (&flag, after) = rest.split_first().ok_or_else(malformed)?;
        rest = after;
        kind.0 = flag;
        match flag {
            0 | 1 => {}
            2 => {
                let (count, after) = rest.split_first_chunk::<4>().ok_or_else(malformed)?;
                rest = after;
                for _ in 0..u32::from_le_bytes(*count) {
                    let (key, after) = rest.split_first_chunk::<16>().ok_or_else(malformed)?;
                    kind.1.push(u128::from_le_bytes(*key));
                    rest = after;
                }
            }
            _ => return Err(malformed()),
        }
    }
    let mut views: [Option<SharedView>; 2] = [None, None];
    let mut input = Decoder::new(rest);
    get_views(&mut input, &mut |table, view| {
        let at = PARTICIPATION
            .iter()
            .position(|&held| held == table)
            .ok_or_else(malformed)?;
        views[at] = Some(view);
        Ok(())
    })?;
    input.finish()?;
    let mut parts: Parts = [None, None];
    for ((part, (flag, deleted)), view) in parts.iter_mut().zip(kinds).zip(views) {
        *part = match (flag, view) {
            (0, None) => None,
            (1, Some(view)) => Some(Part::Whole(view)),
            (2, Some(view)) => {
                let TableView::Complete(upserts) = view.into_inner() else {
                    return Err(malformed());
                };
                Some(Part::Diff { upserts, deleted })
            }
            _ => return Err(malformed()),
        };
    }
    Ok(parts)
}

/// A participation table as a chain of its parts leaves it: its rows by key while complete.
#[derive(Clone, Debug)]
pub(crate) enum Folded {
    Rows(rows::TableRows),
    View(SharedView),
}

fn fold_table<T: TableSchema>(held: &mut Option<Folded>, part: Part) -> io::Result<()> {
    match part {
        Part::Whole(view) => {
            *held = Some(Folded::View(view));
        }
        Part::Diff { upserts, deleted } => {
            let mut rows: BTreeMap<u128, T::Row> = match held.take() {
                Some(Folded::View(view)) => match &*view {
                    TableView::Complete(rows) => rows
                        .rows::<T>()
                        .ok_or_else(malformed)?
                        .iter()
                        .map(|row| (row.key(), row.clone()))
                        .collect(),
                    _ => return Err(invalid("A participation diff follows an incomplete table")),
                },
                Some(Folded::Rows(rows)) => T::take(rows)
                    .map_err(|_| malformed())?
                    .into_iter()
                    .map(|row| (row.key(), row))
                    .collect(),
                None => return Err(invalid("A participation diff has no whole table before it")),
            };
            for key in deleted {
                rows.remove(&key);
            }
            for row in upserts.rows::<T>().ok_or_else(malformed)? {
                rows.insert(row.key(), row.clone());
            }
            *held = Some(Folded::Rows(T::wrap(rows.into_values().collect())));
        }
    }
    Ok(())
}

/// Applies `parts` to `held`, the participation tables a chain left.
pub(crate) fn fold(held: &mut [Option<Folded>; 2], parts: Parts) -> io::Result<()> {
    let [ledgers, histories] = parts;
    if let Some(part) = ledgers {
        fold_table::<tables::WarParticipation>(&mut held[0], part)?;
    }
    if let Some(part) = histories {
        fold_table::<tables::WarLendLeaseHistories>(&mut held[1], part)?;
    }
    Ok(())
}

/// The view a folded table stands for.
pub(crate) fn folded_view(held: &Option<Folded>) -> Option<SharedView> {
    match held {
        Some(Folded::View(view)) => Some(view.clone()),
        Some(Folded::Rows(rows)) => rows
            .try_clone_validated()
            .ok()
            .map(|rows| SharedView::from(TableView::Complete(rows))),
        None => None,
    }
}

/// A war relation's lifetime as its actor is named: both endpoints' identities, then its start.
pub(crate) fn relation_name(key: &WarRelationKey) -> [u8; 20] {
    let mut name = [0; 20];
    name[0..4].copy_from_slice(&key.first.0.to_le_bytes());
    name[4..8].copy_from_slice(&key.first.1.to_le_bytes());
    name[8..12].copy_from_slice(&key.second.0.to_le_bytes());
    name[12..16].copy_from_slice(&key.second.1.to_le_bytes());
    name[16..20].copy_from_slice(&key.start.to_le_bytes());
    name
}

/// The lifetime [`relation_name`] named.
pub(crate) fn relation_key(name: &[u8]) -> io::Result<WarRelationKey> {
    let bytes: &[u8; 20] = name.try_into().map_err(|_| malformed())?;
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"));
    Ok(WarRelationKey {
        first: (word(0), word(4)),
        second: (word(8), word(12)),
        start: word(16) as i32,
    })
}

/// A relation row's counters in its lifetime's endpoint order: the first endpoint's casualties
/// and unknown casualties, then the second's.
pub(crate) fn counters(row: &rows::WarRelationRow, reversed: bool) -> [i64; 4] {
    let raw = [
        row.first_casualties,
        row.first_unknown_casualties,
        row.second_casualties,
        row.second_unknown_casualties,
    ];
    if reversed {
        [raw[2], raw[3], raw[0], raw[1]]
    } else {
        raw
    }
}

/// `row`'s counters set from `held`, its lifetime's counters in endpoint order.
pub(crate) fn restore_counters(row: &mut rows::WarRelationRow, held: [i64; 4], reversed: bool) {
    let raw = if reversed {
        [held[2], held[3], held[0], held[1]]
    } else {
        held
    };
    row.first_casualties = raw[0];
    row.first_unknown_casualties = raw[1];
    row.second_casualties = raw[2];
    row.second_unknown_casualties = raw[3];
}

/// Each WarRelations row's lifetime and whether its endpoints are reversed in it, as the
/// countries of `frame` resolve them; none where a link or country does not.
pub(crate) fn lifetimes(
    frame: &DecodedFrame,
) -> Option<Vec<Option<(WarRelationKey, bool)>>> {
    let countries = frame.complete::<tables::Countries>().ok()?;
    let links = frame.complete::<tables::CountryLinks>().ok()?;
    let relations = frame.complete::<tables::WarRelations>().ok()?;
    let identities: foldhash::HashMap<u32, (u32, u32)> = countries
        .iter()
        .map(|row| (row.canonical_slot, (row.canonical_slot, row.id)))
        .collect();
    let slots: foldhash::HashMap<u32, u32> =
        links.iter().map(|row| (row.id, row.canonical_slot)).collect();
    let identity = |raw: u32| slots.get(&raw).and_then(|slot| identities.get(slot)).copied();
    Some(
        relations
            .iter()
            .map(|row| {
                let (a, b) = (identity(row.first_country)?, identity(row.second_country)?);
                let key = WarRelationKey::new(a, b, row.start_hour);
                Some((key.clone(), a != key.first))
            })
            .collect(),
    )
}
