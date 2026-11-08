//! Production lines from a native recording. Each segment's walk hands its ordered emit, hour by
//! hour, what the fold reads of the hour as a row frame would show it: which of the fold's tables
//! the hour built anew and whether the fold reads that change, their completeness and sample
//! hours, the variant catalogue's evidence and built operands and, only where the fold's input
//! changed, that input as the fold extracts it from a frame: the countries' identities, the
//! definitions a variant read compares, the designs, the organisations, the modifier refusal, the
//! icon operands and each census as a delta of the segment's census before ([`Census`]): every
//! row at its first, then only the rows that changed, came or went. Rows come from the walk's
//! world's views, built only at the hours that need them, and every operand is made with the
//! fold's own functions. The emit takes each census into the one it holds and folds each hour
//! through the fold a row preparation runs ([`Prepared`]), so every reply read from what it
//! publishes is that of the same hours' frames. Each segment starts cold, as a row block starts
//! at a keyframe.

use super::{
    Country, DesignRead, RawOrganisation, link_slot, organisation_name, organisation_state,
    pictures_hold_with,
    prepared::{Directory, Prepared, directory_section},
    raw_organisation, text,
};
use crate::{
    metadata::{TaggedCountry, slot_reads, tagged_countries},
    prepared::pages::Pages,
    products::{
        equipment::{
            self, RawVariant, Tags, Texts, VariantCatalogue, VariantHour,
            columnar::{Naming, slots},
        },
        equipment_icon::{IconContent, IconHour},
    },
};
use dashboard_world::{Hold, Needs, World};
use replay::{
    columnar_frames::metrics,
    columns::{self, Status},
    campaign::Replay,
    segments::{self, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        CountryLinkRow, CountryProductionModifiersRow, CountryRow,
        IndustrialOrganisationRow, LexicalTokenRow, ProductionLineModifiersRow,
        ProductionLineOutputRow, ProductionLineRow, RowCodec, TABLE_COUNT, TableId, TableRows,
        TableSchema, VariantDefinitionRow, VariantStatsRow, full_mask, mask, tables,
    },
};
use replay_frame::frame::{
    ColumnFrame, DecodedFrame, Metrics, Rowset, SharedView, TableView, Tables, not_selected,
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashSet},
    io,
    mem::size_of,
    sync::{Arc, LazyLock, atomic::AtomicBool},
};

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

/// The tables the fold reads without icon content, and with it.
static TABLES: LazyLock<[Vec<TableId>; 2]> = LazyLock::new(|| {
    [false, true].map(|pictured| {
        let mut tables = Prepared::tables(pictured);
        tables.sort_unstable();
        tables.dedup();
        tables
    })
});

/// The tables a walk reads for the fold, `pictured` when it chooses icons.
pub fn tables(pictured: bool) -> &'static [TableId] {
    &TABLES[usize::from(pictured)]
}

/// What a walk's world keeps for the fold: the countries, the spellings, and the views of the
/// tables its input is read from, `pictured` with the icon rule's.
pub fn needs(pictured: bool) -> io::Result<Needs> {
    Ok(Needs {
        countries: true,
        naming: true,
        frame: asks(pictured)?,
        ..Needs::default()
    })
}

/// The tables read whole as rows.
const ROWS: [TableId; 10] = [
    TableId::ProductionLines,
    TableId::ProductionLineOutput,
    TableId::CountryProductionModifiers,
    TableId::ProductionLineModifiers,
    TableId::VariantStats,
    TableId::IndustrialOrganisations,
    TableId::IndustrialOrganisationDetails,
    TableId::IndustrialOrganisationPolicies,
    TableId::VariantDefinitions,
    TableId::ModifierDefinitions,
];

/// The icon rule's tables beside the countries', in the payload fields it reads.
// Row keys are no fields: every held row has its key, so a table read by key alone lists none.
const ICON_FIELDS: [(TableId, &[&str]); 6] = [
    (
        TableId::CountryAppearance,
        &[
            "cosmetic_tag",
            "name_tag",
            "graphical_culture",
            "graphical_culture_2d",
        ],
    ),
    (TableId::PoliticalParties, &["is_ruling", "token"]),
    (TableId::Politics, &["capital", "original_capital"]),
    (TableId::Factions, &[]),
    (TableId::FactionMembers, &["faction", "ordinal"]),
    (TableId::States, &[]),
];

/// How the walk's world holds each table the fold's input is read from.
fn asks(pictured: bool) -> io::Result<Vec<(TableId, Hold)>> {
    let mut asks = dashboard_world::identity()?;
    asks.extend(ROWS.map(|table| (table, Hold::Whole)));
    if pictured {
        for (table, names) in ICON_FIELDS {
            let mut fields = 0_u64;
            for name in names {
                fields |= 1_u64 << columns::field(table, name)?;
            }
            asks.push((table, Hold::Fields(fields).of(table)));
        }
    }
    Ok(asks)
}

const WORDS: usize = TABLE_COUNT.div_ceil(64);

/// A flag for each table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub(super) struct Bits([u64; WORDS]);

impl Bits {
    pub(super) fn get(&self, table: TableId) -> bool {
        let index = table.index();
        self.0[index / 64] >> (index % 64) & 1 != 0
    }

    pub(super) fn set(&mut self, table: TableId, on: bool) {
        let index = table.index();
        let bit = 1_u64 << (index % 64);
        if on {
            self.0[index / 64] |= bit;
        } else {
            self.0[index / 64] &= !bit;
        }
    }

    pub(super) fn clear(&mut self) {
        self.0 = [0; WORDS];
    }

    fn of(flags: &[bool; TABLE_COUNT]) -> Self {
        let mut bits = Self::default();
        for (index, &flag) in flags.iter().enumerate() {
            bits.0[index / 64] |= u64::from(flag) << (index % 64);
        }
        bits
    }
}

/// The countries' identities as the fold reads them where they changed: each holder's tag, each
/// tag's country, the countries their links resolve, and each row's slot, id and tag in table
/// order. Empty where the tables they come from are not complete.
#[derive(Clone, replay_spool::Spooled)]
pub(super) struct Identities {
    pub holders: BTreeMap<(u32, u32), Option<String>>,
    /// None unless Countries and CountryLinks are complete.
    pub selected: Option<BTreeMap<String, Country>>,
    pub linked: BTreeSet<(u32, u32)>,
    pub keys: Vec<(u32, u32, Vec<u8>)>,
}

/// VariantStats' designs where the hour built it anew.
#[derive(Clone, replay_spool::Spooled)]
pub(super) enum Designs {
    /// It is not complete.
    Incomplete,
    /// Every row: the view was built whole.
    Whole(Vec<(u64, DesignRead)>),
    /// The rows the hour wrote, and the variants it deleted.
    Rows {
        changed: Vec<(u64, DesignRead)>,
        deleted: Vec<u64>,
    },
}

/// An organisation as the fold reads it: its operands with its state, without it, and its name.
#[derive(Clone, replay_spool::Spooled)]
pub(super) struct Organisation {
    pub raw: RawOrganisation,
    pub author: RawOrganisation,
    pub name: Option<String>,
}

/// What the hour did to the organisations the fold names.
#[derive(Clone, replay_spool::Spooled)]
pub(super) enum Organisations {
    /// They are not named this hour: none is held.
    Reset,
    /// Each organisation the hour may have changed, None where it is gone.
    Updates(Vec<(u64, Option<Box<Organisation>>)>),
}

/// One census table's entries as a census changed them from the segment's census before, each
/// list by slot and key: every entry in `added` when `whole`; else those in place of one held
/// under their key and slot, those under a key not held under their slot, and the slot and key
/// of each one gone or moved to another slot.
#[derive(Clone, replay_spool::Spooled)]
pub(super) struct Delta<E> {
    pub whole: bool,
    pub changed: Vec<E>,
    pub added: Vec<E>,
    pub gone: Vec<(Option<u32>, u128)>,
}

impl<E: Entry> Delta<E> {
    fn heap(&self) -> usize {
        self.changed
            .iter()
            .chain(&self.added)
            .map(Entry::heap)
            .sum::<usize>()
            + self.gone.len() * size_of::<(Option<u32>, u128)>()
    }
}

/// A census's rows as deltas of the segment's census before: each line and output row with the
/// canonical slot its link resolves, None where it resolves none; and every variant its line and
/// output rows produce, whatever country's, where that changed.
#[derive(Clone, replay_spool::Spooled)]
pub(super) struct Census {
    pub lines: Delta<(Option<u32>, ProductionLineRow)>,
    pub output: Delta<(Option<u32>, ProductionLineOutputRow)>,
    pub modifier_countries: Delta<Arc<CountryProductionModifiersRow>>,
    pub modifier_lines: Delta<ProductionLineModifiersRow>,
    pub variants: Option<Vec<u64>>,
}

impl Census {
    fn heap(&self) -> usize {
        self.lines.heap()
            + self.output.heap()
            + self.modifier_countries.heap()
            + self.modifier_lines.heap()
            + self.variants.as_ref().map_or(0, |variants| variants.len() * 8)
    }
}

/// A census entry: its row, and the slot it is held under.
pub(super) trait Entry: Clone {
    type Row: RowCodec + PartialEq;
    fn row(&self) -> &Self::Row;
    fn slot(&self) -> Option<u32>;
    fn of(slot: Option<u32>, row: &Self::Row) -> Self;
    fn heap(&self) -> usize;
}

impl<R: RowCodec + PartialEq + Clone> Entry for (Option<u32>, R) {
    type Row = R;
    fn row(&self) -> &R {
        &self.1
    }
    fn slot(&self) -> Option<u32> {
        self.0
    }
    fn of(slot: Option<u32>, row: &R) -> Self {
        (slot, row.clone())
    }
    fn heap(&self) -> usize {
        size_of::<Self>()
    }
}

impl Entry for Arc<CountryProductionModifiersRow> {
    type Row = CountryProductionModifiersRow;
    fn row(&self) -> &CountryProductionModifiersRow {
        self
    }
    fn slot(&self) -> Option<u32> {
        None
    }
    fn of(_: Option<u32>, row: &CountryProductionModifiersRow) -> Self {
        Arc::new(row.clone())
    }
    fn heap(&self) -> usize {
        size_of::<Self>() + modifier_country_heap(self)
    }
}

impl Entry for ProductionLineModifiersRow {
    type Row = Self;
    fn row(&self) -> &Self {
        self
    }
    fn slot(&self) -> Option<u32> {
        None
    }
    fn of(_: Option<u32>, row: &Self) -> Self {
        row.clone()
    }
    fn heap(&self) -> usize {
        size_of::<Self>() + self.factory_efficiencies.len() * size_of::<i64>()
    }
}

/// A census row as the walk last handed it: its key, the slot it was held under, and a digest
/// of its fields, which the walker holds and a later census compares.
#[derive(Clone, Copy)]
struct Thin {
    key: u128,
    slot: Option<u32>,
    digest: u128,
    /// The variant a line or output row produces; 0 for a modifier row.
    variant: u64,
}

/// How many held line and output rows produce each variant, and whether a variant began or
/// ended since last asked.
#[derive(Default)]
struct Produced {
    counts: BTreeMap<u64, u32>,
    moved: bool,
}

impl Produced {
    fn add(&mut self, variant: u64) {
        let count = self.counts.entry(variant).or_default();
        self.moved |= *count == 0;
        *count += 1;
    }

    fn remove(&mut self, variant: u64) {
        if let std::collections::btree_map::Entry::Occupied(mut count) = self.counts.entry(variant)
        {
            *count.get_mut() -= 1;
            if *count.get() == 0 {
                count.remove();
                self.moved = true;
            }
        }
    }

    /// Counts `held` anew.
    fn recount<'h>(&mut self, held: impl IntoIterator<Item = &'h Thin>) {
        self.counts.clear();
        for thin in held {
            *self.counts.entry(thin.variant).or_default() += 1;
        }
        self.moved = true;
    }
}

/// A digest of every field of `row`, through `scratch`.
fn digest<R: RowCodec>(row: &R, scratch: &mut Vec<u8>) -> io::Result<u128> {
    use std::hash::{DefaultHasher, Hasher};
    scratch.clear();
    row.encode_change(None, 0, scratch)?;
    let half = |seed: u8| {
        let mut hasher = DefaultHasher::new();
        hasher.write_u8(seed);
        hasher.write(scratch);
        hasher.finish()
    };
    Ok(u128::from(half(0)) << 64 | u128::from(half(1)))
}

/// The delta `rows` (ascending by key, each with its slot) make of `held`, which then holds
/// them: every row when `whole`, and a whole empty table where none is left. Only a row whose
/// key `dirty` names (every row where None) can differ from the one held under its key.
fn diff<'r, E: Entry>(
    held: &mut Vec<Thin>,
    whole: bool,
    dirty: Option<&HashSet<u128>>,
    scratch: &mut Vec<u8>,
    variant: fn(&E::Row) -> u64,
    rows: impl IntoIterator<Item = io::Result<(Option<u32>, Cow<'r, E::Row>)>>,
) -> io::Result<Delta<E>>
where
    E::Row: 'r,
{
    let mut last: Option<u128> = None;
    let mut ascending = |key: u128| {
        let after = last.is_none_or(|last| last < key);
        last = Some(key);
        if after {
            Ok(())
        } else {
            Err(invalid("A production census table is out of key order"))
        }
    };
    let by_slot = |entry: &E| (entry.slot(), entry.row().key());
    if whole {
        let mut next = Vec::new();
        let mut added = Vec::new();
        for row in rows {
            let (slot, row) = row?;
            let key = row.key();
            ascending(key)?;
            next.push(Thin {
                key,
                slot,
                digest: digest(&*row, scratch)?,
                variant: variant(&row),
            });
            added.push(E::of(slot, &row));
        }
        added.sort_unstable_by_key(by_slot);
        *held = next;
        return Ok(Delta {
            whole: true,
            changed: Vec::new(),
            added,
            gone: Vec::new(),
        });
    }
    let mut next = Vec::with_capacity(held.len());
    let (mut changed, mut added, mut gone) = (Vec::new(), Vec::new(), Vec::new());
    let mut old = std::mem::take(held).into_iter().peekable();
    for row in rows {
        let (slot, row) = row?;
        let row = &*row;
        let key = row.key();
        ascending(key)?;
        while let Some(entry) = old.next_if(|entry| entry.key < key) {
            gone.push((entry.slot, entry.key));
        }
        let written = dirty.is_none_or(|dirty| dirty.contains(&key));
        match old.next_if(|entry| entry.key == key) {
            Some(entry) if entry.slot == slot && !written => next.push(entry),
            Some(entry) if entry.slot == slot => {
                let digest = digest(row, scratch)?;
                next.push(Thin {
                    digest,
                    variant: variant(row),
                    ..entry
                });
                if digest != entry.digest {
                    changed.push(E::of(slot, row));
                }
            }
            moved => {
                if let Some(entry) = moved {
                    gone.push((entry.slot, key));
                }
                next.push(Thin {
                    key,
                    slot,
                    digest: digest(row, scratch)?,
                    variant: variant(row),
                });
                added.push(E::of(slot, row));
            }
        }
    }
    gone.extend(old.map(|entry| (entry.slot, entry.key)));
    let emptied = next.is_empty() && !gone.is_empty();
    *held = next;
    if emptied {
        return Ok(Delta {
            whole: true,
            changed: Vec::new(),
            added: Vec::new(),
            gone: Vec::new(),
        });
    }
    changed.sort_unstable_by_key(by_slot);
    added.sort_unstable_by_key(by_slot);
    gone.sort_unstable();
    Ok(Delta {
        whole: false,
        changed,
        added,
        gone,
    })
}

/// [`diff`] of a census reading only the keys `dirty` names, `read` giving each key's row and
/// slot: every other key's row and slot are as the census before held them.
fn patch<'r, E: Entry>(
    held: &mut Vec<Thin>,
    dirty: &HashSet<u128>,
    scratch: &mut Vec<u8>,
    variant: fn(&E::Row) -> u64,
    mut produced: Option<&mut Produced>,
    mut read: impl FnMut(u128) -> io::Result<Option<(Option<u32>, Cow<'r, E::Row>)>>,
) -> io::Result<Delta<E>>
where
    E::Row: 'r,
{
    let mut keys: Vec<u128> = dirty.iter().copied().collect();
    keys.sort_unstable();
    let (mut changed, mut added, mut gone) = (Vec::new(), Vec::new(), Vec::new());
    // Each key's place in `held`, and what it holds there from now on.
    let mut updates: Vec<(Result<usize, usize>, Option<Thin>)> = Vec::new();
    let mut moves = false;
    for key in keys {
        let at = held.binary_search_by_key(&key, |thin| thin.key);
        let old = at.ok().map(|at| held[at]);
        let update = match (old, read(key)?) {
            (None, None) => continue,
            (Some(old), None) => {
                gone.push((old.slot, key));
                if let Some(produced) = produced.as_deref_mut() {
                    produced.remove(old.variant);
                }
                None
            }
            (Some(old), Some((slot, row))) if old.slot == slot => {
                let digest = digest(&*row, scratch)?;
                if digest == old.digest {
                    continue;
                }
                changed.push(E::of(slot, &row));
                let thin = Thin {
                    digest,
                    variant: variant(&row),
                    ..old
                };
                if let Some(produced) = produced.as_deref_mut() {
                    produced.remove(old.variant);
                    produced.add(thin.variant);
                }
                Some(thin)
            }
            (old, Some((slot, row))) => {
                if let Some(old) = old {
                    gone.push((old.slot, key));
                    if let Some(produced) = produced.as_deref_mut() {
                        produced.remove(old.variant);
                    }
                }
                let thin = Thin {
                    key,
                    slot,
                    digest: digest(&*row, scratch)?,
                    variant: variant(&row),
                };
                if let Some(produced) = produced.as_deref_mut() {
                    produced.add(thin.variant);
                }
                added.push(E::of(slot, &row));
                Some(thin)
            }
        };
        moves |= at.is_err() || update.is_none();
        updates.push((at, update));
    }
    if moves {
        let mut next = Vec::with_capacity(held.len() + updates.len());
        let mut from = 0;
        for (at, update) in updates {
            let (until, after) = match at {
                Ok(at) => (at, at + 1),
                Err(at) => (at, at),
            };
            next.extend_from_slice(&held[from..until]);
            from = after;
            next.extend(update);
        }
        next.extend_from_slice(&held[from..]);
        *held = next;
    } else {
        for (at, update) in updates {
            if let (Ok(at), Some(thin)) = (at, update) {
                held[at] = thin;
            }
        }
    }
    if held.is_empty() && !gone.is_empty() {
        return Ok(Delta {
            whole: true,
            changed: Vec::new(),
            added: Vec::new(),
            gone: Vec::new(),
        });
    }
    let by_slot = |entry: &E| (entry.slot(), entry.row().key());
    changed.sort_unstable_by_key(by_slot);
    added.sort_unstable_by_key(by_slot);
    gone.sort_unstable();
    Ok(Delta {
        whole: false,
        changed,
        added,
        gone,
    })
}

/// The census rows the segment last handed on, which its next census is a delta of.
#[derive(Default)]
struct Held {
    lines: Vec<Thin>,
    output: Vec<Thin>,
    modifier_countries: Vec<Thin>,
    modifier_lines: Vec<Thin>,
    /// The modifier tables were read at that census.
    modifiers: bool,
    /// The lines and output were complete at that census.
    lines_read: bool,
    output_read: bool,
    produced: Produced,
}

/// The census tables, in the order the keys each wrote since its last census are kept.
const CENSUS_TABLES: [TableId; 4] = [
    TableId::ProductionLines,
    TableId::ProductionLineOutput,
    TableId::CountryProductionModifiers,
    TableId::ProductionLineModifiers,
];

/// The icon rule's country operands, and each equipment type token's spelling.
#[derive(replay_spool::Spooled)]
pub(super) struct Icons {
    pub hour: Arc<Result<IconHour, String>>,
    pub tokens: Arc<BTreeMap<u32, String>>,
}

/// The fold's input an hour changed, beside its flags.
#[derive(Default)]
#[derive(replay_spool::Spooled)]
pub(super) struct Facts {
    pub identities: Option<Identities>,
    /// The definitions a variant read compares: every one since the last such read anew.
    pub definitions: Option<Vec<VariantDefinitionRow>>,
    /// The spellings of their equipment type tokens, which an icon is chosen by while the fold
    /// holds the definition, live or not.
    pub spellings: Vec<(u32, String)>,
    /// The spellings of the tokens the hour wrote, added or removed: a definition the fold holds
    /// may be spelled by one.
    pub respellings: Vec<(u32, String)>,
    /// The hour begins a run from empty tables: every token it holds is among `respellings`, and
    /// any other has no row.
    pub respell_all: bool,
    pub designs: Option<Designs>,
    pub organisations: Option<Organisations>,
    /// At a census the emit takes, a delta of the segment's census before.
    pub census: Option<Census>,
    pub refusal: Option<String>,
    pub icons: Option<Icons>,
}

/// What a segment's walk hands its emit of an hour.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub hour: i32,
    pub(super) read: Box<HourRead>,
}

/// One hour of a segment, as the fold reads it.
#[derive(replay_spool::Spooled)]
pub struct HourRead {
    pub hour: i32,
    /// The tables read in their slot ([`slot_reads`]).
    pub(super) reads: Bits,
    pub(super) complete: Bits,
    /// The fold's tables whose view the hour built anew, as a row frame's allocation changes.
    pub(super) rebuilt: Bits,
    /// Those whose change the fold reads: any, but Countries' only in a slot or a tag.
    pub(super) relevant: Bits,
    /// The sample hours of complete VariantStats, VariantModules and organisation details.
    pub(super) stats_sampled: Option<i32>,
    pub(super) modules_sampled: Option<i32>,
    pub(super) details_sampled: Option<i32>,
    /// VariantChanges is complete and holds a row.
    pub(super) variant_changes: bool,
    /// VariantDefinitions holds no row, or is not complete.
    pub(super) definitions_empty: bool,
    pub(super) variants: VariantHour,
    pub(super) raws: Vec<(u64, RawVariant)>,
    pub(super) facts: Option<Box<Facts>>,
    bytes: usize,
}

fn string_bytes(text: &str) -> usize {
    text.len()
}

fn organisation_heap(raw: &RawOrganisation) -> usize {
    size_of::<RawOrganisation>()
        + raw.token.len()
        + raw.name_key.len()
        + raw.icon.len()
        + raw.owner_tag.as_ref().map_or(0, String::len)
        + raw.state.as_ref().map_or(0, |state| {
            state.funds_raw.len()
                + state
                    .unlocked
                    .iter()
                    .chain(&state.planned)
                    .map(|token| token.len() + size_of::<String>())
                    .sum::<usize>()
        })
}

fn modifier_country_heap(row: &CountryProductionModifiersRow) -> usize {
    size_of::<CountryProductionModifiersRow>()
        + row
            .trees
            .iter()
            .map(|tree| {
                size_of_val(tree)
                    + tree.failure.len()
                    + tree.nodes.len() * size_of::<replay_data::rows::ModifierBreakdownNode>()
                    + tree.nodes.iter().map(|node| node.name.len()).sum::<usize>()
            })
            .sum::<usize>()
}

fn definition_heap(row: &VariantDefinitionRow) -> usize {
    size_of::<VariantDefinitionRow>()
        + row.display_name.len()
        + row.variant_name.len()
        + row.icon.len()
}

impl Facts {
    /// The bytes it holds, `new_icons` those of the icon operands first held here.
    fn heap(&self, new_icons: usize) -> usize {
        let identities = self.identities.as_ref().map_or(0, |identities| {
            identities.holders.len() * (size_of::<((u32, u32), Option<String>)>() + 8)
                + identities.selected.as_ref().map_or(0, |selected| {
                    selected.len() * (size_of::<(String, Country)>() + 8)
                })
                + identities.linked.len() * size_of::<(u32, u32)>()
                + identities
                    .keys
                    .iter()
                    .map(|(_, _, tag)| size_of::<(u32, u32, Vec<u8>)>() + tag.len())
                    .sum::<usize>()
        });
        let definitions = self
            .definitions
            .as_ref()
            .map_or(0, |rows| rows.iter().map(definition_heap).sum::<usize>())
            + self
                .spellings
                .iter()
                .chain(&self.respellings)
                .map(|(_, spelling)| size_of::<(u32, String)>() + spelling.len())
                .sum::<usize>();
        let designs = match &self.designs {
            Some(Designs::Whole(rows)) => rows.capacity() * size_of::<(u64, DesignRead)>(),
            Some(Designs::Rows { changed, deleted }) => {
                changed.capacity() * size_of::<(u64, DesignRead)>() + deleted.capacity() * 8
            }
            _ => 0,
        };
        let organisations = match &self.organisations {
            Some(Organisations::Updates(rows)) => rows
                .iter()
                .map(|(_, organisation)| {
                    size_of::<(u64, Option<Box<Organisation>>)>()
                        + organisation.as_ref().map_or(0, |organisation| {
                            organisation_heap(&organisation.raw)
                                + organisation_heap(&organisation.author)
                                + organisation.name.as_deref().map_or(0, string_bytes)
                        })
                })
                .sum(),
            _ => 0,
        };
        let census = self.census.as_ref().map_or(0, Census::heap);
        size_of::<Self>()
            + identities
            + definitions
            + designs
            + organisations
            + census
            + self.refusal.as_deref().map_or(0, string_bytes)
            + new_icons
    }
}

/// The keys of `table` the hour wrote or inserted, and those it deleted and did not set again,
/// each ascending, as a recorded delta names them.
fn edited(state: &State, table: TableId) -> io::Result<(Vec<u128>, Vec<u128>)> {
    let held = columns::table(state, table)?;
    let changes = columns::changes(state, table)?;
    let mut written: Vec<u128> = changes
        .changed
        .iter()
        .zip(&changes.masks)
        .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
        .filter(|&(_, &mask)| mask != 0)
        .map(|(&slot, _)| slot)
        .chain(changes.inserted.iter().copied())
        .filter_map(|slot| held.key(slot))
        .collect();
    written.sort_unstable();
    written.dedup();
    let mut deleted = changes.deleted_keys.clone();
    deleted.sort_unstable();
    deleted.dedup();
    deleted.retain(|key| written.binary_search(key).is_err());
    Ok((written, deleted))
}

/// What the caches keep of an organisation they hold: its country, the tokens its row and
/// detail name, and whether a detail of its country was held with it.
struct Organised {
    country: u32,
    tokens: Box<[u32]>,
    detailed: bool,
}

/// The organisations' caches the fold keeps to find those an hour changes, as the fold's own.
#[derive(Default)]
struct Mio {
    valid: bool,
    rows: BTreeMap<u64, Organised>,
    tokens: BTreeMap<u32, BTreeSet<u64>>,
    countries: BTreeMap<u32, BTreeSet<u64>>,
    owners: BTreeMap<u32, BTreeSet<u64>>,
    owner_slots: BTreeMap<u64, u32>,
}

impl Mio {
    fn clear(&mut self) {
        *self = Self::default();
    }
}

/// The organisations held with a detail.
fn detailed(rows: &BTreeMap<u64, Organised>) -> impl Iterator<Item = u64> + '_ {
    rows.iter()
        .filter(|(_, held)| held.detailed)
        .map(|(&id, _)| id)
}

/// The spellings of `tokens`, as LexicalTokens' rows of them, sorted by token.
fn spelled(
    naming: &dashboard_world::Naming,
    state: &State,
    tokens: impl IntoIterator<Item = u32>,
) -> io::Result<Vec<LexicalTokenRow>> {
    let mut tokens: Vec<u32> = tokens.into_iter().collect();
    tokens.sort_unstable();
    tokens.dedup();
    let mut rows = Vec::with_capacity(tokens.len());
    for token in tokens {
        if let Some(literal) = naming.spelling(state, token)? {
            rows.push(LexicalTokenRow {
                token,
                literal: literal.into_owned(),
            });
        }
    }
    Ok(rows)
}

/// Each of `tokens`' equipment type spelling, as a definition's is read ([`equipment::variant_token`]),
/// from the rows LexicalTokens holds, whatever its status this hour: what a later complete hour
/// shows unless an hour between edits it. A token it holds no row of is spelled empty, as there.
fn spellings(
    state: &State,
    format: FormatVersion,
    tokens: impl IntoIterator<Item = u32>,
) -> io::Result<Vec<(u32, String)>> {
    let mut tokens: Vec<u32> = tokens.into_iter().collect();
    tokens.sort_unstable();
    tokens.dedup();
    let held = columns::table(state, TableId::LexicalTokens)?;
    let mut spelled = Vec::with_capacity(tokens.len());
    for token in tokens {
        let row = match held.slot(u128::from(token)) {
            Some(slot) => Some(columns::row::<LexicalTokenRow>(
                state,
                TableId::LexicalTokens,
                slot,
                format.version(),
            )?),
            None => None,
        };
        let spelling = equipment::variant_token(
            &VariantDefinitionRow {
                equipment_token: token,
                ..Default::default()
            },
            row.as_slice(),
        );
        spelled.push((token, spelling));
    }
    Ok(spelled)
}

/// Keys of a table's rows, `keys`, as the fold's candidates read them: none unless rebuilt, the
/// hour's edited keys where its delta is exact, else every row's.
fn candidates(
    rebuilt: bool,
    edits: Option<(Vec<u128>, Vec<u128>)>,
    keys: impl Iterator<Item = u128>,
) -> BTreeSet<u128> {
    if !rebuilt {
        return BTreeSet::new();
    }
    match edits {
        Some((written, deleted)) => written.into_iter().chain(deleted).collect(),
        None => keys.collect(),
    }
}

/// The rows of `T`'s complete table, each of the fields `mask` names; none when it is not
/// complete.
fn owned<T: TableSchema>(frame: &ColumnFrame<'_>, mask: u64) -> io::Result<Option<Vec<T::Row>>> {
    let Ok(rows) = frame.complete_rows::<T>(mask) else {
        return Ok(None);
    };
    rows.iter()
        .map(|row| row.map(Cow::into_owned))
        .collect::<io::Result<Vec<_>>>()
        .map(Some)
}

/// The countries' and links' fields the fold reads: their slots and tags.
const COUNTRY_FIELDS: u64 = mask::<CountryRow>(&["canonical_slot", "tag"]);
const LINK_FIELDS: u64 = mask::<CountryLinkRow>(&["canonical_slot"]);
/// The design fields a variant read keeps.
const DESIGN_FIELDS: u64 = mask::<VariantStatsRow>(&[
    "organisation",
    "organisation_trait_count",
    "organisation_bonuses",
]);
const DEFINITION_TOKEN: u64 = mask::<VariantDefinitionRow>(&["equipment_token"]);

/// A segment's walk of the fold's tables: what it hands on of each hour, and what it keeps to
/// tell what changed.
/// The floor of production lines' operator hours.
pub static FLOOR: dashboard_world::floor::Floor = dashboard_world::floor::Floor::new(&[
    "census hours",
    "census rows iterated",
    "census rows written since the census before",
    "census entries handed",
    "hours copying countries and links",
    "country and link rows copied",
    "organisation hours",
    "definition hours",
    "design hours",
    "icon hours",
    "token hours",
    "hours reading anything",
]);

pub struct Projector {
    format: FormatVersion,
    icons: Option<Arc<IconContent>>,
    tables: &'static [TableId],
    asks: Vec<(TableId, Hold)>,
    /// The fields of each icon table its rule reads, to tell when its operands may change.
    icon_fields: Vec<(TableId, u64)>,
    naming: Naming,
    cold: bool,
    /// Each table's status the hour before, by table index.
    statuses: Vec<Option<Status>>,
    countries_identity: u64,
    countries_full: u64,
    /// Scratch for folding a Countries delta's written columns by slot.
    written: Vec<(u32, u64)>,
    /// The definitions written since the last read that compared them; None for every one.
    pending: Option<BTreeSet<u128>>,
    definitions_changed: bool,
    mio: Mio,
    /// The countries' slots and ids in table order, and each slot's last row, as their last change
    /// left them.
    country_keys: Vec<(u32, u32)>,
    country_rows: BTreeMap<u32, usize>,
    /// The census rows last handed on; None before the segment's first census.
    held: Option<Held>,
    /// Each census table's keys the hours since its last census wrote; None where any row may
    /// have changed.
    dirty: [Option<HashSet<u128>>; 4],
    /// Room a row is encoded in for its digest.
    scratch: Vec<u8>,
    /// The modifier refusal while ModifierDefinitions and the spellings stay as they were.
    refusal: Option<Option<String>>,
    icon_hour: Option<Arc<Result<IconHour, String>>>,
    icon_dirty: bool,
    tokens: Option<Arc<BTreeMap<u32, String>>>,
    tokens_dirty: bool,
    /// Whether the organisations were named the hour before; None before the segment's first.
    named: Option<bool>,
    /// The walk begins at a run's start, from empty tables.
    empty: bool,
    /// Every variant the census last handed on produces.
    variants: Option<Vec<u64>>,
    /// The links changed since the last census.
    links_moved: bool,
}

impl Projector {
    pub fn new(format: FormatVersion, icons: Option<Arc<IconContent>>) -> io::Result<Self> {
        let pictured = icons.is_some();
        let asks = asks(pictured)?;
        let mut icon_fields = Vec::new();
        if pictured {
            for &(table, hold) in &asks {
                if table == TableId::Countries
                    || table == TableId::CountryLinks
                    || ICON_FIELDS.iter().any(|(icon, _)| *icon == table)
                {
                    let fields = match hold {
                        Hold::Whole => u64::MAX,
                        Hold::Fields(fields) => fields,
                    };
                    icon_fields.push((table, fields));
                }
            }
        }
        let countries = TableId::Countries;
        Ok(Self {
            format,
            icons,
            tables: tables(pictured),
            asks,
            icon_fields,
            naming: Naming::new()?,
            cold: true,
            statuses: vec![None; TABLE_COUNT],
            countries_identity: crate::prepared::frame_changes::field_mask(
                countries,
                &["canonical_slot", "tag"],
            ),
            countries_full: full_mask(countries.last_change().fields(countries, format.version())),
            written: Vec::new(),
            pending: None,
            definitions_changed: true,
            mio: Mio::default(),
            country_keys: Vec::new(),
            country_rows: BTreeMap::new(),
            held: None,
            dirty: [None, None, None, None],
            scratch: Vec::new(),
            refusal: None,
            icon_hour: None,
            icon_dirty: true,
            tokens: None,
            tokens_dirty: true,
            named: None,
            empty: false,
            variants: None,
            links_moved: true,
        })
    }

    /// Whether a Countries delta the hour applied touches what the fold reads of it: a row in or
    /// out, a slot or tag written, or every field of a row written.
    fn countries_relevant(&mut self, state: &State) -> io::Result<bool> {
        let changes = columns::changes(state, TableId::Countries)?;
        if !changes.deleted_keys.is_empty() || !changes.inserted.is_empty() {
            return Ok(true);
        }
        let masks = changes.masks.iter().chain(&changes.fill_masks);
        if masks
            .clone()
            .any(|mask| mask & self.countries_identity != 0)
        {
            return Ok(true);
        }
        // No row's columns can add up to every field unless all of the hour's do.
        if masks.fold(0, |all, mask| all | mask) & self.countries_full != self.countries_full {
            return Ok(false);
        }
        let written = &mut self.written;
        written.clear();
        written.extend(
            changes
                .changed
                .iter()
                .copied()
                .zip(changes.masks.iter().copied())
                .chain(
                    changes
                        .fill_slots
                        .iter()
                        .copied()
                        .zip(changes.fill_masks.iter().copied()),
                )
                .filter(|&(_, mask)| mask != 0),
        );
        written.sort_unstable_by_key(|&(slot, _)| slot);
        let mut at = 0;
        while at < written.len() {
            let slot = written[at].0;
            let mut mask = 0;
            while at < written.len() && written[at].0 == slot {
                mask |= written[at].1;
                at += 1;
            }
            if mask == self.countries_full {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The asks of `tables` alone, as the world holds them.
    fn pick(&self, tables: &[TableId]) -> Vec<(TableId, Hold)> {
        self.asks
            .iter()
            .filter(|(table, _)| tables.contains(table))
            .copied()
            .collect()
    }
}

/// A frame of no table, its tables never selected.
fn empty(branch: u64) -> DecodedFrame {
    DecodedFrame {
        branch,
        hour: 0,
        tables: std::array::from_fn(|_| not_selected()),
        metrics: Metrics::default(),
    }
}

/// The hour's flags of the fold's tables, as [`Projector::hour`] reads them.
struct Flags {
    reads: [bool; TABLE_COUNT],
    complete: Bits,
    rebuilt: Bits,
    relevant: Bits,
    /// The table's view was built from the hour before's by the hour's delta alone.
    patched: Bits,
}

impl Flags {
    fn read(&self, table: TableId) -> bool {
        self.reads[table.index()]
    }

    /// The hour's edited keys of `table` where its delta is exact; None where any row may have
    /// changed.
    fn edits(&self, state: &State, table: TableId) -> io::Result<Option<(Vec<u128>, Vec<u128>)>> {
        if self.patched.get(table) {
            edited(state, table).map(Some)
        } else {
            Ok(None)
        }
    }
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, start: segments::Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.empty = matches!(start, segments::Start::Run { .. });
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        FLOOR.hour(hour.state, hour.manifest, self.tables);
        let read = self.read(hour)?;
        out.push(Hour {
            hour: hour.hour,
            read: Box::new(read),
        })
        .map_err(segments::family_failure)
    }
}

impl Projector {
    /// One hour of the walk, as the fold reads it.
    fn read(&mut self, hour: &segments::Hour<'_, World>) -> io::Result<HourRead> {
        let _cpu = replay_frame::cpu::scope("production-lines.columnar", Some("production-lines"));
        let (state, manifest, world) = (hour.state, hour.manifest, hour.world);
        let at = hour.hour;
        let restart = std::mem::take(&mut self.cold) || hour.starts || hour.opens;
        let reads = slot_reads(self.format, &metrics(manifest), at);
        let mut present = [false; TABLE_COUNT];
        for section in &manifest.sections {
            if let Some(present) = present.get_mut(section.table) {
                *present = true;
            }
        }
        let mut flags = Flags {
            reads,
            complete: Bits::default(),
            rebuilt: Bits::default(),
            relevant: Bits::default(),
            patched: Bits::default(),
        };
        let mut shaped = [false; TABLE_COUNT];
        let dictionary = state.dictionary();
        for &table in self.tables {
            let index = table.index();
            let status = columns::status(state, table);
            let previous = self.statuses[index].replace(status);
            shaped[index] = restart || previous != Some(status);
            let complete = status == Status::Complete;
            // A view is built anew as the row decoder's is: a complete table its sections patch, a
            // status it did not hold, or a truncated read.
            let rebuilt = restart
                || match (previous, status) {
                    (Some(Status::Complete), Status::Complete) => present[index],
                    (Some(Status::NotRecorded), Status::NotRecorded) => false,
                    (Some(Status::Unavailable { reason: was }), Status::Unavailable { reason }) => {
                        dictionary.get(was) != dictionary.get(reason)
                    }
                    _ => true,
                };
            let patched = !restart && rebuilt && complete && previous == Some(Status::Complete);
            let relevant = rebuilt
                && (table != TableId::Countries || !patched || self.countries_relevant(state)?);
            flags.complete.set(table, complete);
            flags.rebuilt.set(table, rebuilt);
            flags.relevant.set(table, relevant);
            flags.patched.set(table, patched);
        }
        self.links_moved |= flags.rebuilt.get(TableId::CountryLinks);
        for (dirty, table) in self.dirty.iter_mut().zip(CENSUS_TABLES) {
            if !flags.rebuilt.get(table) {
                continue;
            }
            match (flags.edits(state, table)?, dirty) {
                (Some((written, deleted)), Some(dirty)) => {
                    dirty.extend(written);
                    dirty.extend(deleted);
                }
                (Some(_), None) => {}
                (None, dirty) => *dirty = None,
            }
        }
        let complete = |table: TableId| flags.complete.get(table);
        let (roster, names) = (world.countries()?, world.naming()?);
        let naming_phase = replay_frame::cpu::scope_phase("pl.read:naming");
        let (variants, raws) = {
            let mut written = [false; TABLE_COUNT];
            for table in VariantCatalogue::TABLES {
                let index = table.index();
                written[index] = shaped[index] || slots::relevant(state, table, u64::MAX)?;
            }
            self.naming.project(
                state,
                (roster, names),
                self.format,
                at,
                restart,
                &shaped,
                &written,
                flags.read(TableId::VariantDefinitions),
            )?
        };
        drop(naming_phase);
        let _decide = replay_frame::cpu::scope_phase("pl.read:decide");
        let sampled = |table: TableId| {
            complete(table)
                .then(|| state.sampled_at(table.index()))
                .flatten()
        };
        let live = |table: TableId| -> io::Result<usize> {
            Ok(columns::table(state, table)?.live().count())
        };
        let variant_changes =
            complete(TableId::VariantChanges) && live(TableId::VariantChanges)? > 0;
        let definitions_empty =
            !complete(TableId::VariantDefinitions) || live(TableId::VariantDefinitions)? == 0;
        if flags.rebuilt.get(TableId::VariantDefinitions) {
            self.definitions_changed = true;
            match (
                flags.edits(state, TableId::VariantDefinitions)?,
                &mut self.pending,
            ) {
                (Some((written, deleted)), Some(pending)) => {
                    pending.extend(written);
                    pending.extend(deleted);
                }
                (Some(_), None) => {}
                (None, pending) => *pending = None,
            }
        }
        let pictured = self.icons.is_some();
        let tokens_moved = names.tokens_rebuilt() || !names.changed_tokens().is_empty();
        let mut respellings = Vec::new();
        let respell_all = pictured && std::mem::take(&mut self.empty);
        if pictured && flags.rebuilt.get(TableId::LexicalTokens) {
            // The hour's own edits, from the hour before's rows, even at a restart.
            let (written, deleted) = edited(state, TableId::LexicalTokens)?;
            respellings = spellings(
                state,
                self.format,
                written.into_iter().chain(deleted).map(|key| key as u32),
            )?;
            self.tokens_dirty |= !respellings.is_empty();
        }
        if pictured {
            for &(table, fields) in &self.icon_fields {
                self.icon_dirty |= world.touched(table, fields);
            }
            self.icon_dirty |= restart;
            self.tokens_dirty |=
                restart || flags.rebuilt.get(TableId::VariantDefinitions) || tokens_moved;
        }
        // A refusal read where both are complete names no hour, so it holds while they do.
        if restart
            || tokens_moved
            || flags.rebuilt.get(TableId::ModifierDefinitions)
            || !complete(TableId::ModifierDefinitions)
            || !complete(TableId::LexicalTokens)
        {
            self.refusal = None;
        }

        let relevant = |table: TableId| flags.relevant.get(table);
        let identities = relevant(TableId::Countries) || relevant(TableId::CountryLinks);
        let definitions_read = (variant_changes || flags.read(TableId::VariantDefinitions))
            && complete(TableId::VariantDefinitions);
        let definitions = definitions_read && self.definitions_changed;
        let designs = flags.rebuilt.get(TableId::VariantStats);
        let organisations_named = complete(TableId::IndustrialOrganisations)
            && complete(TableId::LexicalTokens)
            && complete(TableId::Countries)
            && complete(TableId::CountryLinks);
        let names_changed = restart
            || !self.mio.valid
            || [
                TableId::Countries,
                TableId::CountryLinks,
                TableId::LexicalTokens,
                TableId::IndustrialOrganisations,
                TableId::IndustrialOrganisationDetails,
                TableId::IndustrialOrganisationPolicies,
            ]
            .into_iter()
            .any(relevant);
        let organisations = names_changed && organisations_named;
        let census = flags.read(TableId::ProductionLines) && complete(TableId::CountryLinks);
        let modifiers_read = flags.read(TableId::CountryProductionModifiers)
            && flags.read(TableId::ProductionLineModifiers);
        let modifiers_complete = complete(TableId::CountryProductionModifiers)
            && complete(TableId::ProductionLineModifiers);
        let refusal_due = flags.read(TableId::ProductionLines)
            && complete(TableId::ProductionLines)
            && modifiers_read
            && modifiers_complete;
        let icons_due = pictured
            && (definitions_read || flags.read(TableId::ProductionLines))
            && complete(TableId::LexicalTokens);

        let mut wanted: Vec<TableId> = Vec::new();
        if identities || organisations || census {
            wanted.extend([TableId::Countries, TableId::CountryLinks]);
        }
        if organisations {
            wanted.extend([
                TableId::IndustrialOrganisations,
                TableId::IndustrialOrganisationDetails,
                TableId::IndustrialOrganisationPolicies,
            ]);
        }
        if census {
            wanted.extend([TableId::ProductionLines, TableId::ProductionLineOutput]);
            if modifiers_read && modifiers_complete {
                wanted.extend([
                    TableId::CountryProductionModifiers,
                    TableId::ProductionLineModifiers,
                ]);
            }
        }
        if definitions || (icons_due && self.tokens_dirty) {
            wanted.push(TableId::VariantDefinitions);
        }
        if designs && complete(TableId::VariantStats) {
            wanted.push(TableId::VariantStats);
        }
        let refuse = refusal_due && self.refusal.is_none();
        if refuse {
            wanted.push(TableId::ModifierDefinitions);
        }
        let icon_hour = icons_due && (self.icon_dirty || self.icon_hour.is_none());
        if icon_hour {
            wanted.push(TableId::Countries);
            wanted.push(TableId::CountryLinks);
            wanted.extend(ICON_FIELDS.map(|(table, _)| table));
        }
        let mut lent = self.pick(&wanted);
        // Spellings the walk keeps no view of are asked of it whole only where they are not complete.
        if refuse && !complete(TableId::LexicalTokens) {
            lent.push((TableId::LexicalTokens, Hold::Whole));
        }

        let mut facts = Facts {
            respellings,
            respell_all,
            ..Facts::default()
        };
        let mut new_icons = 0;
        if !lent.is_empty() {
            FLOOR.add(11, 1);
            for (counted, at) in [
                (organisations, 6),
                (definitions, 7),
                (designs, 8),
                (icon_hour, 9),
                (icons_due && self.tokens_dirty, 10),
            ] {
                if counted {
                    FLOOR.add(at, 1);
                }
            }
            let _facts = replay_frame::cpu::scope_phase("pl.read:facts");
            new_icons = self.facts(
                (state, world),
                names,
                &flags,
                Wants {
                    identities,
                    definitions,
                    designs,
                    organisations,
                    census,
                    refuse,
                    refusal_due,
                    icon_hour,
                    tokens: icons_due && self.tokens_dirty,
                    modifiers: modifiers_read && modifiers_complete,
                },
                at,
                &mut facts,
            )?;
        } else if designs {
            facts.designs = Some(Designs::Incomplete);
        }
        if definitions {
            self.definitions_changed = false;
            self.pending = Some(BTreeSet::new());
        }
        // Once on entering the unnamed state: the fold holds none from then until named again.
        if !organisations_named && self.named != Some(false) {
            self.mio.clear();
            facts.organisations = Some(Organisations::Reset);
        }
        self.named = Some(organisations_named);
        if refusal_due {
            facts.refusal = self.refusal.clone().flatten();
        }
        if icons_due && let (Some(hour), Some(tokens)) = (&self.icon_hour, &self.tokens) {
            facts.icons = Some(Icons {
                hour: Arc::clone(hour),
                tokens: Arc::clone(tokens),
            });
        }
        let raws_bytes = raws.capacity() * size_of::<(u64, RawVariant)>()
            + raws.iter().map(|(_, raw)| raw.heap()).sum::<usize>();
        let empty = facts.identities.is_none()
            && facts.definitions.is_none()
            && facts.spellings.is_empty()
            && facts.respellings.is_empty()
            && !facts.respell_all
            && facts.designs.is_none()
            && facts.organisations.is_none()
            && facts.census.is_none()
            && facts.refusal.is_none()
            && facts.icons.is_none();
        let facts = (!empty).then(|| Box::new(facts));
        let bytes = raws_bytes
            + facts
                .as_ref()
                .map_or(0, |facts| facts.heap(new_icons));
        Ok(HourRead {
            hour: at,
            reads: Bits::of(&flags.reads),
            complete: flags.complete,
            rebuilt: flags.rebuilt,
            relevant: flags.relevant,
            stats_sampled: sampled(TableId::VariantStats),
            modules_sampled: sampled(TableId::VariantModules),
            details_sampled: sampled(TableId::IndustrialOrganisationDetails),
            variant_changes,
            definitions_empty,
            variants,
            raws,
            facts,
            bytes,
        })
    }
}

/// What an hour hands on beside its flags.
#[derive(Clone, Copy)]
struct Wants {
    identities: bool,
    definitions: bool,
    designs: bool,
    organisations: bool,
    census: bool,
    /// The refusal is read anew, and whether it is read at all.
    refuse: bool,
    refusal_due: bool,
    icon_hour: bool,
    tokens: bool,
    modifiers: bool,
}

impl Projector {
    /// Fills `facts` from the walk's views of the hour; the bytes of the icon operands first
    /// held here.
    #[allow(clippy::too_many_arguments)]
    fn facts(
        &mut self,
        (state, world): (&State, &World),
        names: &dashboard_world::Naming,
        flags: &Flags,
        wants: Wants,
        at: i32,
        facts: &mut Facts,
    ) -> io::Result<usize> {
        let frame = world.columns(state);
        if wants.refuse {
            let _cpu = replay_frame::cpu::scope_phase("pl.facts:refusal");
            self.refusal = Some(self.refusal((state, world), &frame, names, at)?);
        }
        let identity_phase = replay_frame::cpu::scope_phase("pl.facts:identities");
        // The countries and links, of the fields the fold reads, where a read names countries.
        let (countries, links) = if wants.identities || wants.organisations || wants.census {
            (
                owned::<tables::Countries>(&frame, COUNTRY_FIELDS)?,
                owned::<tables::CountryLinks>(&frame, LINK_FIELDS)?,
            )
        } else {
            (None, None)
        };
        let (countries, links) = (countries.as_deref(), links.as_deref());
        if wants.identities || wants.organisations || wants.census {
            FLOOR.add(4, 1);
            FLOOR.add(
                5,
                (countries.map_or(0, <[_]>::len) + links.map_or(0, <[_]>::len)) as u64,
            );
        }
        let old_keys = if wants.identities {
            let identities = identities(countries, links);
            let keys = countries
                .into_iter()
                .flatten()
                .map(|row| (row.canonical_slot, row.id))
                .collect();
            self.country_rows = countries
                .into_iter()
                .flatten()
                .enumerate()
                .map(|(at, row)| (row.canonical_slot, at))
                .collect();
            facts.identities = Some(identities);
            Some(std::mem::replace(&mut self.country_keys, keys))
        } else {
            None
        };
        drop(identity_phase);
        if wants.definitions {
            let _cpu = replay_frame::cpu::scope_phase("pl.facts:definitions");
            let rows = frame.complete_rows::<tables::VariantDefinitions>(
                TableId::VariantDefinitions.full_mask(),
            )?;
            let handed: Vec<VariantDefinitionRow> = match &self.pending {
                None => rows
                    .iter()
                    .map(|row| row.map(Cow::into_owned))
                    .collect::<io::Result<_>>()?,
                Some(pending) => {
                    let mut handed = Vec::new();
                    for &key in pending {
                        if let Some(row) = rows.get(key)? {
                            handed.push(row.into_owned());
                        }
                    }
                    handed
                }
            };
            facts.spellings = spellings(
                state,
                self.format,
                handed.iter().map(|row| row.equipment_token),
            )?;
            facts.definitions = Some(handed);
        }
        if wants.designs {
            let _cpu = replay_frame::cpu::scope_phase("pl.facts:designs");
            facts.designs = Some(match frame.complete_rows::<tables::VariantStats>(DESIGN_FIELDS) {
                Err(_) => Designs::Incomplete,
                Ok(stats) => {
                    // A complete view is sampled at its last complete read.
                    let design_hour = state
                        .sampled_at(TableId::VariantStats.index())
                        .or_else(|| {
                            flags
                                .complete
                                .get(TableId::VariantModules)
                                .then(|| state.sampled_at(TableId::VariantModules.index()))
                                .flatten()
                        });
                    let read = |row: &VariantStatsRow| {
                        (
                            row.variant,
                            DesignRead {
                                first_hour: at,
                                read_hour: design_hour,
                                author: Some(row.organisation),
                                trait_count: Some(row.organisation_trait_count),
                                bonus_count: Some(row.organisation_bonuses.len() as u32),
                            },
                        )
                    };
                    match flags.edits(state, TableId::VariantStats)? {
                        None => {
                            let mut whole = Vec::with_capacity(stats.len());
                            for row in stats.iter() {
                                whole.push(read(&*row?));
                            }
                            Designs::Whole(whole)
                        }
                        Some((written, deleted)) => {
                            let mut changed = Vec::with_capacity(written.len());
                            let mut gone: Vec<u64> =
                                deleted.iter().map(|&key| key as u64).collect();
                            for key in written {
                                match stats.get(key)? {
                                    Some(row) => changed.push(read(&row)),
                                    None => gone.push(key as u64),
                                }
                            }
                            Designs::Rows {
                                changed,
                                deleted: gone,
                            }
                        }
                    }
                }
            });
        }
        if wants.organisations
            && let (Some(countries), Some(links)) = (countries, links)
        {
            let _cpu = replay_frame::cpu::scope_phase("pl.facts:organisations");
            let rows = frame.complete_rows::<tables::IndustrialOrganisations>(
                TableId::IndustrialOrganisations.full_mask(),
            )?;
            let updates = self.organisations(
                state,
                names,
                &frame,
                flags,
                at,
                (countries, links),
                rows,
                old_keys,
            )?;
            facts.organisations = Some(Organisations::Updates(updates));
        }
        if wants.census
            && let Some(links) = links
        {
            let _cpu = replay_frame::cpu::scope_phase("pl.facts:census");
            let whole = self.held.is_none();
            FLOOR.add(0, 1);
            if dashboard_world::floor::Floor::on() {
                let written = self.dirty.iter().map(|dirty| dirty.as_ref().map_or(0, HashSet::len)).sum::<usize>();
                FLOOR.add(2, written as u64);
            }
            let held = self.held.get_or_insert_with(Held::default);
            let (dirty, scratch) = (&mut self.dirty, &mut self.scratch);
            let lines = frame
                .complete_rows::<tables::ProductionLines>(TableId::ProductionLines.full_mask())
                .ok();
            let output = frame
                .complete_rows::<tables::ProductionLineOutput>(
                    TableId::ProductionLineOutput.full_mask(),
                )
                .ok();
            let refused = wants.refusal_due && !self.refusal.as_ref().is_some_and(Option::is_none);
            let modifiers = wants.modifiers && !refused;
            let (countries, modified) = if modifiers {
                (
                    Some(frame.complete_rows::<tables::CountryProductionModifiers>(
                        TableId::CountryProductionModifiers.full_mask(),
                    )?),
                    Some(frame.complete_rows::<tables::ProductionLineModifiers>(
                        TableId::ProductionLineModifiers.full_mask(),
                    )?),
                )
            } else {
                (None, None)
            };
            // A census reads only the keys its tables wrote since the census before, where that
            // census read them too and the links put every other row where it was.
            let links_moved = std::mem::take(&mut self.links_moved);
            let kept = |dirty: &Option<HashSet<u128>>, before: bool, now: bool, slotted: bool| {
                !whole && before && now && !(slotted && links_moved) && dirty.is_some()
            };
            let line_variant: fn(&ProductionLineRow) -> u64 = |row| row.variant;
            let output_variant: fn(&ProductionLineOutputRow) -> u64 = |row| row.variant;
            let lines_kept = kept(&dirty[0], held.lines_read, lines.is_some(), true);
            let output_kept = kept(&dirty[1], held.output_read, output.is_some(), true);
            let lines_delta = match (&dirty[0], lines.filter(|_| lines_kept)) {
                (Some(keys), Some(rows)) => patch(
                    &mut held.lines,
                    keys,
                    scratch,
                    line_variant,
                    Some(&mut held.produced),
                    |key| Ok(rows.get(key)?.map(|row| (link_slot(links, row.country), row))),
                )?,
                _ => diff(
                    &mut held.lines,
                    whole,
                    dirty[0].as_ref(),
                    scratch,
                    line_variant,
                    lines.iter().flat_map(|rows| rows.iter()).map(|row| {
                        let row = row?;
                        Ok((link_slot(links, row.country), row))
                    }),
                )?,
            };
            let output_delta = match (&dirty[1], output.filter(|_| output_kept)) {
                (Some(keys), Some(rows)) => patch(
                    &mut held.output,
                    keys,
                    scratch,
                    output_variant,
                    Some(&mut held.produced),
                    |key| Ok(rows.get(key)?.map(|row| (link_slot(links, row.country), row))),
                )?,
                _ => diff(
                    &mut held.output,
                    whole,
                    dirty[1].as_ref(),
                    scratch,
                    output_variant,
                    output.iter().flat_map(|rows| rows.iter()).map(|row| {
                        let row = row?;
                        Ok((link_slot(links, row.country), row))
                    }),
                )?,
            };
            held.lines_read = lines.is_some();
            held.output_read = output.is_some();
            if !lines_kept || !output_kept {
                held.produced.recount(held.lines.iter().chain(&held.output));
            }
            let variants = std::mem::take(&mut held.produced.moved)
                .then(|| held.produced.counts.keys().copied().collect::<Vec<u64>>())
                .filter(|variants| self.variants.as_ref() != Some(variants))
                .inspect(|variants| self.variants = Some(variants.clone()));
            let modifiers_kept = held.modifiers && modified.is_some();
            held.modifiers = modified.is_some();
            let unproduced: fn(&ProductionLineModifiersRow) -> u64 = |_| 0;
            let modifier_lines = match (&dirty[3], modified.filter(|_| {
                kept(&dirty[3], modifiers_kept, true, true)
            })) {
                (Some(keys), Some(rows)) => {
                    patch(&mut held.modifier_lines, keys, scratch, unproduced, None, |key| {
                        Ok(rows.get(key)?.map(|row| (None, row)))
                    })?
                }
                _ => diff(
                    &mut held.modifier_lines,
                    whole,
                    dirty[3].as_ref(),
                    scratch,
                    unproduced,
                    modified
                        .iter()
                        .flat_map(|rows| rows.iter())
                        .map(|row| Ok((None, row?))),
                )?,
            };
            let unread: fn(&CountryProductionModifiersRow) -> u64 = |_| 0;
            let modifier_countries = match (&dirty[2], countries.filter(|_| {
                kept(&dirty[2], modifiers_kept, true, false)
            })) {
                (Some(keys), Some(rows)) => patch(
                    &mut held.modifier_countries,
                    keys,
                    scratch,
                    unread,
                    None,
                    |key| Ok(rows.get(key)?.map(|row| (None, row))),
                )?,
                _ => diff(
                    &mut held.modifier_countries,
                    whole,
                    dirty[2].as_ref(),
                    scratch,
                    unread,
                    countries
                        .iter()
                        .flat_map(|rows| rows.iter())
                        .map(|row| Ok((None, row?))),
                )?,
            };
            let read_rows = [
                (lines_kept, 0, held.lines.len()),
                (output_kept, 1, held.output.len()),
                (modifiers_kept, 2, held.modifier_countries.len()),
                (modifiers_kept, 3, held.modifier_lines.len()),
            ]
            .into_iter()
            .map(|(kept, at, len)| match (&dirty[at], kept) {
                (Some(keys), true) => keys.len(),
                _ => len,
            })
            .sum::<usize>();
            facts.census = Some(Census {
                lines: lines_delta,
                output: output_delta,
                modifier_countries,
                modifier_lines,
                variants,
            });
            for dirty in dirty.iter_mut() {
                *dirty = Some(HashSet::new());
            }
            if dashboard_world::floor::Floor::on() {
                FLOOR.add(1, read_rows as u64);
                if let Some(census) = &facts.census {
                    let entries = |changed: usize, added: usize, gone: usize| changed + added + gone;
                    let handed = entries(census.lines.changed.len(), census.lines.added.len(), census.lines.gone.len())
                        + entries(census.output.changed.len(), census.output.added.len(), census.output.gone.len())
                        + entries(census.modifier_countries.changed.len(), census.modifier_countries.added.len(), census.modifier_countries.gone.len())
                        + entries(census.modifier_lines.changed.len(), census.modifier_lines.added.len(), census.modifier_lines.gone.len());
                    FLOOR.add(3, handed as u64);
                }
            }
        }
        let mut new_icons = 0;
        if wants.icon_hour
            && let Some(content) = self.icons.as_deref()
        {
            let _cpu = replay_frame::cpu::scope_phase("pl.facts:icons");
            // The icon rule reads decoded rows: its tables are copied in the fields it reads.
            let mut icon_frame = empty(frame.branch);
            let icon_tables = self.pick(
                &[TableId::Countries, TableId::CountryLinks]
                    .into_iter()
                    .chain(ICON_FIELDS.map(|(table, _)| table))
                    .collect::<Vec<_>>(),
            );
            world.copy_into(state, &icon_tables, &mut icon_frame)?;
            let hour = IconHour::read(&icon_frame, content).map_err(|error| error.to_string());
            new_icons += match &hour {
                Ok(hour) => hour.heap(),
                Err(why) => why.len(),
            };
            self.icon_hour = Some(Arc::new(hour));
            self.icon_dirty = false;
        }
        if wants.tokens {
            let _cpu = replay_frame::cpu::scope_phase("pl.facts:tokens");
            let mut live = Vec::new();
            if let Ok(rows) = frame.complete_rows::<tables::VariantDefinitions>(DEFINITION_TOKEN) {
                live.reserve(rows.len());
                for row in rows.iter() {
                    live.push(row?.equipment_token);
                }
            }
            let mut held = BTreeMap::new();
            for (token, spelling) in spellings(state, self.format, live)? {
                new_icons += size_of::<(u32, String)>() + spelling.len();
                held.insert(token, spelling);
            }
            self.tokens = Some(Arc::new(held));
            self.tokens_dirty = false;
        }
        Ok(new_icons)
    }

    /// The modifier refusal of the modifier tables' fixed layout, read from ModifierDefinitions
    /// and the spellings of its tokens, as the fold reads it of a frame.
    fn refusal(
        &self,
        (state, world): (&State, &World),
        frame: &ColumnFrame<'_>,
        names: &dashboard_world::Naming,
        at: i32,
    ) -> io::Result<Option<String>> {
        let mut checked = empty(frame.branch);
        checked.hour = at;
        let definitions = TableId::ModifierDefinitions;
        checked.tables[definitions.index()] = world.copy(state, definitions, Hold::Whole)?;
        let lexical = TableId::LexicalTokens;
        checked.tables[lexical.index()] = if names.tokens_complete() {
            let mut tokens = Vec::new();
            const TOKEN: u64 = mask::<replay_data::rows::ModifierDefinitionRow>(&["token"]);
            if let Ok(rows) = frame.complete_rows::<tables::ModifierDefinitions>(TOKEN) {
                for row in rows.iter() {
                    tokens.push(row?.token);
                }
            }
            SharedView::from(TableView::Complete(TableRows::LexicalTokens(spelled(
                names, state, tokens,
            )?)))
        } else if !columns::complete(state, lexical) {
            // The spellings are read whole only where they are not complete.
            world.copy(state, lexical, Hold::Whole)?
        } else {
            not_selected()
        };
        Ok(replay_derive::fixed_layout::validate_recording(
            &checked,
            replay_derive::fixed_layout::PRODUCTION_MODIFIERS,
        )
        .err())
    }

    /// Each organisation the hour may have changed, as the fold projects it from a frame.
    #[allow(clippy::too_many_arguments)]
    fn organisations(
        &mut self,
        state: &State,
        names: &dashboard_world::Naming,
        frame: &ColumnFrame<'_>,
        flags: &Flags,
        hour: i32,
        (countries, links): (&[CountryRow], &[CountryLinkRow]),
        rows: Rowset<'_, IndustrialOrganisationRow>,
        old_keys: Option<Vec<(u32, u32)>>,
    ) -> io::Result<Vec<(u64, Option<Box<Organisation>>)>> {
        let changed = |table: TableId| flags.relevant.get(table);
        let any = |table: TableId| flags.rebuilt.get(table) && !flags.patched.get(table);
        let mio = &mut self.mio;
        let mut affected: BTreeSet<u64> = if !mio.valid {
            rows.keys().map(|key| key as u64).collect()
        } else {
            candidates(
                flags.rebuilt.get(TableId::IndustrialOrganisations),
                flags.edits(state, TableId::IndustrialOrganisations)?,
                rows.keys(),
            )
            .into_iter()
            .map(|id| id as u64)
            .collect()
        };
        if any(TableId::IndustrialOrganisations) {
            affected.extend(mio.rows.keys().copied());
        }
        let details = frame
            .complete_rows::<tables::IndustrialOrganisationDetails>(
                TableId::IndustrialOrganisationDetails.full_mask(),
            )
            .ok();
        if changed(TableId::IndustrialOrganisationDetails) {
            if any(TableId::IndustrialOrganisationDetails) {
                affected.extend(detailed(&mio.rows));
            }
            affected.extend(
                candidates(
                    true,
                    flags.edits(state, TableId::IndustrialOrganisationDetails)?,
                    details.iter().flat_map(|rows| rows.keys()),
                )
                .into_iter()
                .map(|id| id as u64),
            );
            if details.is_none() {
                affected.extend(detailed(&mio.rows));
            }
        }
        let policies = frame
            .complete_rows::<tables::IndustrialOrganisationPolicies>(
                TableId::IndustrialOrganisationPolicies.full_mask(),
            )
            .ok();
        if changed(TableId::IndustrialOrganisationPolicies) {
            for id in candidates(
                true,
                flags.edits(state, TableId::IndustrialOrganisationPolicies)?,
                policies.iter().flat_map(|rows| rows.keys()),
            ) {
                affected.extend(
                    mio.countries
                        .get(&(id as u32))
                        .into_iter()
                        .flatten()
                        .copied(),
                );
            }
            if policies.is_none() || any(TableId::IndustrialOrganisationPolicies) {
                affected.extend(mio.rows.keys().copied());
            }
        }
        if changed(TableId::LexicalTokens) {
            match flags.edits(state, TableId::LexicalTokens)? {
                // Every organisation, which holds every one a token names.
                None => affected.extend(mio.rows.keys().copied()),
                Some((written, deleted)) => {
                    for token in written.into_iter().chain(deleted) {
                        affected.extend(
                            mio.tokens
                                .get(&(token as u32))
                                .into_iter()
                                .flatten()
                                .copied(),
                        );
                    }
                }
            }
        }
        if changed(TableId::CountryLinks) {
            affected.extend(mio.rows.keys().copied());
        } else if changed(TableId::Countries) {
            match flags.edits(state, TableId::Countries)? {
                None => affected.extend(mio.rows.keys().copied()),
                Some((written, deleted)) => {
                    let old = old_keys.as_deref().unwrap_or(&self.country_keys);
                    for key in written.into_iter().chain(deleted) {
                        let before = old
                            .binary_search_by_key(&(key as u32), |&(_, id)| id)
                            .ok()
                            .map(|at| old[at].0);
                        let now = countries
                            .binary_search_by_key(&key, RowCodec::key)
                            .ok()
                            .map(|at| countries[at].canonical_slot);
                        for slot in before.into_iter().chain(now) {
                            affected.extend(mio.owners.get(&slot).into_iter().flatten().copied());
                        }
                    }
                }
            }
        }
        let texts = self
            .icons
            .as_deref()
            .and_then(|icons| icons.catalogue.as_deref())
            .map(|catalogue| catalogue as &dyn Texts);
        // A complete view is sampled at its last complete read.
        let details_sampled = details
            .is_some()
            .then(|| state.sampled_at(TableId::IndustrialOrganisationDetails.index()))
            .flatten();
        let mut updates = Vec::with_capacity(affected.len());
        for id in affected {
            if let Some(held) = mio.rows.get(&id) {
                for &token in &held.tokens {
                    super::input::remove_user(&mut mio.tokens, token, id);
                }
                super::input::remove_user(&mut mio.countries, held.country, id);
            }
            if let Some(slot) = mio.owner_slots.remove(&id) {
                super::input::remove_user(&mut mio.owners, slot, id);
            }
            let Some(row) = rows.get(u128::from(id))? else {
                mio.rows.remove(&id);
                updates.push((id, None));
                continue;
            };
            let row = &*row;
            let detail = match &details {
                Some(details) => details.get(u128::from(id))?,
                None => None,
            }
            .filter(|detail| detail.country == row.country);
            let detail = detail.as_deref();
            mio.countries.entry(row.country).or_default().insert(id);
            let owner_slot = link_slot(links, row.owner);
            if let Some(slot) = owner_slot {
                mio.owners.entry(slot).or_default().insert(id);
                mio.owner_slots.insert(id, slot);
            }
            let tokens = super::input::organisation_tokens(Some(row), detail);
            for &token in &tokens {
                mio.tokens.entry(token).or_default().insert(id);
            }
            let held = Organised {
                country: row.country,
                tokens: tokens.iter().copied().collect(),
                detailed: detail.is_some(),
            };
            let lexical = spelled(names, state, tokens)?;
            let owner: Vec<_> = owner_slot
                .and_then(|slot| self.country_rows.get(&slot))
                .map(|&at| countries[at].clone())
                .into_iter()
                .collect();
            let tags = Tags::new(&owner, links, None);
            let author = raw_organisation(row, hour, &tags, &lexical);
            let name = organisation_name(&author, texts);
            let mut raw = author.clone();
            let policy = match (detail, &policies) {
                (Some(detail), Some(policies)) => policies.get(u128::from(detail.country))?,
                _ => None,
            };
            raw.state = detail.map(|detail| {
                let mut state = organisation_state(detail, details_sampled, &lexical);
                state.policy_name = policy
                    .as_deref()
                    .and_then(|row| {
                        row.policies
                            .iter()
                            .find(|policy| policy.token == detail.state.policy)
                    })
                    .map(|policy| text(&policy.name));
                state
            });
            mio.rows.insert(id, held);
            updates.push((id, Some(Box::new(Organisation { raw, author, name }))));
        }
        mio.valid = true;
        Ok(updates)
    }
}

/// The countries' identities of `countries` and `links`, as the fold reads them of a frame.
fn identities(
    countries: Option<&[replay_data::rows::CountryRow]>,
    links: Option<&[replay_data::rows::CountryLinkRow]>,
) -> Identities {
    let mut holders = BTreeMap::new();
    let mut selected = BTreeMap::new();
    if let Some(countries) = countries {
        if let Some(links) = links {
            for (tag, country) in tagged_countries(countries, links) {
                let Ok(tag) = std::str::from_utf8(tag) else {
                    continue;
                };
                selected.insert(
                    tag.to_owned(),
                    match country {
                        TaggedCountry::Absent => Country::Absent,
                        TaggedCountry::Unresolved => Country::Unresolved,
                        TaggedCountry::Found(row, slot) => Country::Found(slot, row.id),
                    },
                );
            }
        }
        for country in countries {
            holders.insert(
                (country.canonical_slot, country.id),
                std::str::from_utf8(&country.tag).ok().map(str::to_owned),
            );
        }
    }
    let linked = countries
        .zip(links)
        .map(|(countries, links)| {
            countries
                .iter()
                .filter(|country| link_slot(links, country.id) == Some(country.canonical_slot))
                .map(|country| (country.canonical_slot, country.id))
                .collect()
        })
        .unwrap_or_default();
    Identities {
        holders,
        selected: countries.zip(links).map(|_| selected),
        linked,
        keys: countries
            .into_iter()
            .flatten()
            .map(|row| (row.canonical_slot, row.id, row.tag.clone()))
            .collect(),
    }
}

/// Cancellation the fold's own checks never see: a run is cancelled where it walks.
static NEVER: AtomicBool = AtomicBool::new(false);

/// A branch's fold.
pub struct Branch {
    prepared: Prepared,
    branch: u64,
    /// The last hour folded.
    last: Option<i32>,
}

impl Branch {
    pub fn new(
        replay: &Replay,
        branch: u64,
        tags: impl IntoIterator<Item = String>,
        icons: Option<&IconContent>,
    ) -> Self {
        Self {
            prepared: Prepared::new(replay, branch, tags, icons),
            branch,
            last: None,
        }
    }

    /// The branch's fold going on from what it last published through `writer`, prepared under
    /// the same content as `icons`.
    pub fn resume<W: Pages + ?Sized>(
        writer: &mut W,
        replay: &Replay,
        branch: u64,
        icons: Option<&IconContent>,
    ) -> io::Result<Self> {
        let bytes = writer
            .read_at(&directory_section(branch), 0)?
            .ok_or_else(|| invalid("The production lines' directory is missing"))?;
        let directory: Directory =
            serde_json::from_slice(&crate::prepared::series::decompress(&bytes)?)
                .map_err(|error| invalid(&error.to_string()))?;
        if directory.executable != engine_executable::executable_tables::is_initialized()
            || !pictures_hold_with(icons, directory.pictured, directory.visual.as_deref())
        {
            return Err(dashboard_core::refused(
                "The production lines were prepared under other content",
            ));
        }
        let last = directory.coverage.last_hour;
        Ok(Self {
            prepared: Prepared::resume_native(directory, replay, writer)?,
            branch,
            last: Some(last),
        })
    }

    /// What it goes on with beside its committed publication: the spellings its definitions'
    /// icons are chosen by.
    pub fn save(&self) -> Vec<u8> {
        let spellings = self.prepared.spellings();
        let mut out = Vec::with_capacity(4 + spellings.len() * 16);
        out.extend_from_slice(&(spellings.len() as u32).to_le_bytes());
        for (token, spelling) in spellings {
            out.extend_from_slice(&token.to_le_bytes());
            out.extend_from_slice(&(spelling.len() as u32).to_le_bytes());
            out.extend_from_slice(spelling.as_bytes());
        }
        out
    }

    /// Takes up what [`Self::save`] wrote; nothing, from a run that saved none.
    pub fn load(&mut self, saved: &[u8]) -> io::Result<()> {
        if saved.is_empty() {
            return Ok(());
        }
        let short = || invalid("The production lines' saved spellings are cut short");
        let mut rest = saved;
        let mut take = |count: usize| -> io::Result<&[u8]> {
            let (taken, after) = rest.split_at_checked(count).ok_or_else(short)?;
            rest = after;
            Ok(taken)
        };
        let word = |bytes: &[u8]| u32::from_le_bytes(bytes.try_into().expect("four bytes"));
        let count = word(take(4)?);
        let mut spellings = BTreeMap::new();
        for _ in 0..count {
            let token = word(take(4)?);
            let length = word(take(4)?) as usize;
            let spelling = std::str::from_utf8(take(length)?)
                .map_err(|_| invalid("A saved production spelling is not text"))?;
            spellings.insert(token, spelling.to_owned());
        }
        if !rest.is_empty() {
            return Err(invalid(
                "The production lines' saved spellings run past their end",
            ));
        }
        self.prepared.take_spellings(spellings);
        Ok(())
    }

    /// Folds `hour`.
    pub fn observe<W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        hour: Hour,
        icons: Option<&IconContent>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope_phase("production-lines:emit_read");
        let at = hour.read.hour;
        self.prepared.observe_facts(writer, *hour.read, icons, &NEVER)?;
        self.last = Some(at);
        Ok(())
    }

    /// Publishes what it folded through `writer`; `through` is the branch's last hour once it
    /// folded the branch whole.
    pub fn publish<W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        replay: &Replay,
        through: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let last_block = through.and(super::last_block(replay, self.branch));
        self.prepared
            .publish_native(writer, through.or(self.last), last_block, cancelled)
    }
}
