//! Division equipment's operator on a columnar walk ([`replay::segments`]): each hour's
//! changed rows, read from the walk's world ([`dashboard_world`]: the countries, the divisions
//! and the small tables' frame) and the walker's columns, stepped through the segment's own
//! engine. A reply stands on reads made up to two days before its hour, which a restart does
//! not hold: until the segment's reads are its own, its hours go to the emit as inputs
//! ([`Projected::Lead`]), which the engine of the segment before steps; from then on, as the
//! engine's answers. Every start is the operator's own, so segments walk at once.

use super::DESIGNS;
use super::engine::{
    COMPLETE, Countries, DAILY, DEFINITIONS, Engine, Entry, Input, NEW, Output, Rows, STOCK, Tokens,
};
use crate::products::army_content::Holder;
use dashboard_text::{catalogue::Catalogue, metadata::slot_read};
use dashboard_world::{Hold, Kept, Needs, Roster, Tagged, World};
use replay::{
    columns,
    segments::{self, Item, Operator, Sink, Start, State},
};
use replay_data::{display::stepped_format, format::FormatVersion, rows::TableId};
use std::{collections::BTreeSet, io, mem::size_of, sync::Arc};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// Every table a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::CountryLocalSubunits,
    TableId::DivisionTemplates,
    TableId::DivisionTemplateIcons,
    TableId::VariantDefinitions,
    TableId::Divisions,
    TableId::DivisionStatus,
    TableId::DivisionLandInputs,
    TableId::NewDivisionEquipment,
    TableId::DivisionEquipment,
    TableId::Stockpiles,
];

/// The small tables the needs, names and artwork read as rows, beside the countries'
/// identity.
const CONTEXT: [TableId; 4] = [
    TableId::LexicalTokens,
    TableId::CountryLocalSubunits,
    TableId::DivisionTemplates,
    TableId::DivisionTemplateIcons,
];

/// The context's tables as the walk's frame is asked for them: Countries by its identity.
fn context() -> io::Result<Vec<(TableId, Hold)>> {
    let mut held = dashboard_world::identity()?;
    held.extend(CONTEXT.iter().map(|&table| (table, Hold::Whole)));
    Ok(held)
}

/// [`DESIGNS`] as the frame is asked for them.
fn designs() -> io::Result<Vec<(TableId, Hold)>> {
    let mut held = dashboard_world::identity()?;
    held.extend(
        DESIGNS
            .iter()
            .filter(|table| !matches!(table, TableId::Countries | TableId::CountryLinks))
            .map(|&table| (table, Hold::Whole)),
    );
    Ok(held)
}

/// What a projection reads of its walk's world.
pub fn needs() -> io::Result<Needs> {
    let mut frame = context()?;
    frame.extend(designs()?);
    Ok(Needs {
        countries: true,
        divisions: true,
        frame,
        ..Needs::default()
    })
}

/// One projected item: an hour another engine answers, an hour answered, or the segment's
/// engine at its end.
pub(crate) enum Projected {
    /// An hour before the segment's reads are its own: the engine of the segment before
    /// steps it.
    Lead(Box<Input>),
    Hour(Box<Output>),
    /// The segment's engine after its last hour; none when it answered none of them.
    Handover(Option<Box<Engine>>),
}

/// A projected item with its branch.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    pub(crate) projected: Projected,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        match &self.projected {
            Projected::Lead(input) => size_of::<Input>() + input.heap(),
            Projected::Hour(output) => size_of::<Output>() + output.heap(),
            Projected::Handover(engine) => engine
                .as_ref()
                .map_or(0, |engine| size_of::<Engine>() + engine.heap()),
        }
    }
}

/// The fields a table's rows are kept by.
struct Fields {
    /// Design and amount of NewDivisionEquipment, DivisionEquipment and Stockpiles.
    entries: [[usize; 2]; 3],
}

pub struct Projector {
    format: FormatVersion,
    cat: Option<Arc<Catalogue>>,
    gaps: Vec<(u64, i32, i32)>,
    /// Each branch's first recorded hour.
    firsts: Vec<(u64, i32)>,
    fields: Fields,
    context: Vec<(TableId, Hold)>,
    designs: Vec<(TableId, Hold)>,
    branch: u64,
    last: Option<i32>,
    complete: [bool; 7],
    /// Designs whose definitions changed since the last designs read; none while unknown.
    definitions: Option<BTreeSet<u64>>,
    engine: Option<Box<Engine>>,
}

impl Projector {
    /// A segment's projector of a recording in `format`, reading content `cat`: each branch's
    /// first recorded hour and each recorded gap (branch, first and last hour) are the
    /// recording's extents.
    pub fn within(
        format: FormatVersion,
        (firsts, gaps): (Vec<(u64, i32)>, Vec<(u64, i32, i32)>),
        cat: Option<Arc<Catalogue>>,
    ) -> io::Result<Self> {
        let entry = |table| -> io::Result<[usize; 2]> {
            Ok([
                columns::field(table, "variant_handle")?,
                columns::field(table, "amount_raw")?,
            ])
        };
        Ok(Self {
            format,
            cat,
            gaps,
            firsts,
            fields: Fields {
                entries: [
                    entry(TableId::NewDivisionEquipment)?,
                    entry(TableId::DivisionEquipment)?,
                    entry(TableId::Stockpiles)?,
                ],
            },
            context: context()?,
            designs: designs()?,
            branch: 0,
            last: None,
            complete: [false; 7],
            definitions: None,
            engine: None,
        })
    }

    /// The pool or stockpile table at `COMPLETE[at]`'s rows the hour gives: every live row when
    /// `whole`, else those whose design or amount it wrote or that it inserted, and the keys
    /// it deleted.
    fn entries(&self, state: &State, at: usize, whole: bool) -> io::Result<Rows<Entry>> {
        let table = COMPLETE[at];
        if !self.complete_now(state, at) {
            return Ok(Rows::Same);
        }
        let held = columns::table(state, table)?;
        let [variant_field, amount_field] = self.fields.entries[at - NEW];
        let variants = columns::plain::<u64>(held, variant_field)?;
        let amounts = columns::plain::<i64>(held, amount_field)?;
        let version = self.format.version();
        let read = |slot: u32, key: u128| -> io::Result<Entry> {
            if !held.recorded(variant_field, slot) || !held.recorded(amount_field, slot) {
                // An unrecorded cell reads as the row decoder reads it.
                let (variant, amount) = if at == STOCK {
                    let row: replay_data::rows::StockpileRow =
                        columns::row(state, table, slot, version)?;
                    (row.variant_handle, row.amount_raw)
                } else {
                    let row: replay_data::rows::DivisionEquipmentRow =
                        columns::row(state, table, slot, version)?;
                    (row.variant_handle, row.amount_raw)
                };
                return Ok(Entry {
                    key,
                    variant,
                    amount,
                });
            }
            let missing = || invalid(format!("{table:?} slot {slot} holds no cell"));
            let units = *amounts.get(slot as usize).ok_or_else(missing)?;
            // A stepped column holds display units.
            let amount = match table.display_step(amount_field) {
                Some(step) if stepped_format(version) => step.value(units)?,
                _ => units,
            };
            Ok(Entry {
                key,
                variant: *variants.get(slot as usize).ok_or_else(missing)?,
                amount,
            })
        };
        if whole {
            let mut rows = engine_allocation::vec(held.live().count())?;
            for slot in held.live().ones() {
                let key = held
                    .key(slot)
                    .ok_or_else(|| invalid(format!("{table:?} slot {slot} holds no key")))?;
                rows.push(read(slot, key)?);
            }
            return Ok(Rows::Whole(rows));
        }
        let changes = columns::changes(state, table)?;
        let fields = 1_u64 << variant_field | 1_u64 << amount_field;
        let mut set = Vec::new();
        let written = changes
            .changed
            .iter()
            .zip(&changes.masks)
            .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
            .filter(|(_, mask)| **mask & fields != 0)
            .map(|(slot, _)| slot)
            .chain(&changes.inserted);
        for &slot in written {
            if let Some(key) = held.key(slot) {
                engine_allocation::reserve(&mut set, 1)?;
                set.push(read(slot, key)?);
            }
        }
        if set.is_empty() && changes.deleted_keys.is_empty() {
            return Ok(Rows::Same);
        }
        Ok(Rows::Changed(set, changes.deleted_keys.clone()))
    }

    /// Whether `COMPLETE[at]` holds every row this hour, and the walk reads it.
    fn complete_now(&self, state: &State, at: usize) -> bool {
        let table = COMPLETE[at];
        !state.skips(table.index()) && columns::complete(state, table)
    }

    /// Templates whose DivisionTemplates or DivisionTemplateIcons rows the hour wrote; none
    /// when either holds less than every row.
    fn templates(state: &State) -> io::Result<Option<Vec<u64>>> {
        let mut keys = Vec::new();
        for table in [TableId::DivisionTemplates, TableId::DivisionTemplateIcons] {
            if state.skips(table.index()) || !columns::complete(state, table) {
                return Ok(None);
            }
            let changes = columns::changes(state, table)?;
            if changes.truncated {
                return Ok(None);
            }
            let held = columns::table(state, table)?;
            keys.extend(changes.deleted_keys.iter().map(|&key| key as u64));
            let written = changes
                .changed
                .iter()
                .zip(&changes.masks)
                .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
                .filter(|(_, mask)| **mask != 0)
                .map(|(slot, _)| slot)
                .chain(&changes.inserted);
            for &slot in written {
                if let Some(key) = held.key(slot) {
                    keys.push(key as u64);
                }
            }
        }
        keys.sort_unstable();
        keys.dedup();
        Ok(Some(keys))
    }

    /// What the hour wrote of the lexical tokens; a change of their status reaches the engine
    /// in its frame.
    fn tokens(state: &State) -> io::Result<Tokens> {
        let table = TableId::LexicalTokens;
        if state.skips(table.index()) || !columns::complete(state, table) {
            return Ok(Tokens::Same);
        }
        let changes = columns::changes(state, table)?;
        Ok(
            if !changes.changed.is_empty()
                || !changes.fill_slots.is_empty()
                || !changes.deleted.is_empty()
                || changes.truncated
            {
                Tokens::Changed
            } else if changes.inserted.is_empty() {
                Tokens::Same
            } else {
                Tokens::Appended
            },
        )
    }

    /// Whether the hour wrote rows of `table`.
    fn wrote(state: &State, table: TableId) -> io::Result<bool> {
        if state.skips(table.index()) {
            return Ok(false);
        }
        let changes = columns::changes(state, table)?;
        Ok(changes.truncated
            || !changes.changed.is_empty()
            || !changes.fill_slots.is_empty()
            || !changes.inserted.is_empty()
            || !changes.deleted.is_empty())
    }

    /// Designs whose definitions the hour wrote; none when any may have.
    fn definitions(state: &State) -> io::Result<Option<Vec<u64>>> {
        let table = TableId::VariantDefinitions;
        if state.skips(table.index()) || !columns::complete(state, table) {
            return Ok(None);
        }
        let changes = columns::changes(state, table)?;
        if changes.truncated {
            return Ok(None);
        }
        let held = columns::table(state, table)?;
        let mut keys: Vec<u64> = changes.deleted_keys.iter().map(|&key| key as u64).collect();
        for &slot in changes
            .changed
            .iter()
            .chain(&changes.fill_slots)
            .chain(&changes.inserted)
        {
            if let Some(key) = held.key(slot) {
                keys.push(key as u64);
            }
        }
        Ok(Some(keys))
    }
}

/// The countries an hour's replies read, from the walk's roster.
fn countries(roster: &Roster) -> Countries {
    let complete = roster.complete();
    let mut holders = Vec::new();
    if complete {
        for tag in roster.held() {
            let Ok(text) = std::str::from_utf8(roster.tag(tag)) else {
                continue;
            };
            if dashboard_base::products::validate_tag(text).is_err() {
                continue;
            }
            let holder = match roster.tagged(tag) {
                Tagged::Found(country) => Some(Holder {
                    slot: country.slot,
                    id: country.id,
                }),
                Tagged::Unresolved => None,
                Tagged::Absent => continue,
            };
            holders.push((Arc::<str>::from(text), holder));
        }
        holders.sort_unstable_by(|(left, _), (right, _)| left.as_bytes().cmp(right.as_bytes()));
    }
    let ids = roster.countries_complete().then(|| {
        let mut ids = std::collections::BTreeMap::new();
        for country in roster.iter() {
            ids.entry(country.slot).or_insert(country.id);
        }
        ids
    });
    Countries {
        complete,
        links: if roster.links_complete() {
            roster.links().to_vec()
        } else {
            Vec::new()
        },
        holders,
        ids,
    }
}

/// One of the walk's division tables as the hour gives it: whole when it was read whole,
/// held again or `whole`; else the divisions it edited.
fn kept<T: Copy + PartialEq, R>(
    kept: &Kept<T>,
    whole: bool,
    row: impl Fn(u64, T) -> R,
) -> io::Result<Rows<R>> {
    if !kept.holds() {
        return Ok(Rows::Same);
    }
    if whole || kept.whole() {
        let mut rows = engine_allocation::vec(kept.len())?;
        rows.extend(kept.iter().map(|(id, &value)| row(id, value)));
        return Ok(Rows::Whole(rows));
    }
    if kept.edits().is_empty() {
        return Ok(Rows::Same);
    }
    let mut set = Vec::new();
    let mut gone = Vec::new();
    for &(id, _, after) in kept.edits() {
        match after {
            Some(value) => set.push(row(id, value)),
            None => gone.push(u128::from(id)),
        }
    }
    Ok(Rows::Changed(set, gone))
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, start: Start<'_>, branch: u64, first: i32) -> io::Result<()> {
        self.branch = branch;
        let begins = self
            .firsts
            .iter()
            .any(|&(held, hour)| held == branch && hour == first);
        let (resumed, cold) = match start {
            // A restart holds none of the reads made before it.
            Start::Restart(_) => (None, Some(first)),
            // A branch observed from a later hour knows no read for its first day.
            Start::Run { continues: false } => ((!begins).then_some(first), None),
            Start::Run { continues: true } => (None, None),
        };
        self.engine = Some(Box::new(Engine::new(self.cat.clone(), resumed, cold)));
        self.definitions = matches!(start, Start::Run { .. }).then(BTreeSet::new);
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("division_equipment.facts", None);
        let (state, world, at) = (hour.state, hour.world, hour.hour);
        let whole = self.last.and_then(|last| last.checked_add(1)) != Some(at);
        self.last = Some(at);
        let complete: [bool; 7] = std::array::from_fn(|index| self.complete_now(state, index));
        let before = std::mem::replace(&mut self.complete, complete);
        let again = |index: usize| whole || !before[index];
        let format = self.format;
        let metrics = world.metrics();
        let reads = [
            TableId::DivisionEquipment,
            TableId::Stockpiles,
            TableId::VariantDefinitions,
        ]
        .map(|table| slot_read(format, metrics, table, at));
        let walked = world.divisions()?;
        let roster = world.countries()?;
        let context_changed = whole
            || roster.changed()
            || CONTEXT.iter().any(|&table| world.touched(table, u64::MAX));
        let reads_designs = reads[2] && complete[DEFINITIONS];
        let changed = Self::definitions(state)?;
        self.definitions = match (self.definitions.take(), changed) {
            (Some(mut since), Some(now)) => {
                since.extend(now);
                Some(since)
            }
            _ => None,
        };
        // What changed since the last designs read, handed to this one.
        let definitions = if reads_designs {
            self.definitions
                .replace(BTreeSet::new())
                .map(|since| since.into_iter().collect())
        } else {
            None
        };
        // A view the engine still held would be copied, not changed in place.
        if context_changed && let Some(engine) = self.engine.as_mut() {
            engine.release();
        }
        let input = Input {
            hour: at,
            gap: self
                .gaps
                .iter()
                .any(|&(branch, first, last)| branch == hour.branch && first <= at && at <= last),
            reads,
            complete,
            divisions: kept(walked.divisions(), whole, |_, division| division)?,
            maxima: kept(walked.maxima(), whole, |id, value| (id, value))?,
            manpower: kept(walked.manpower(), whole, |id, value| (id, i64::from(value)))?,
            new: self.entries(state, NEW, again(NEW))?,
            daily: self.entries(state, DAILY, again(DAILY))?,
            stock: self.entries(state, STOCK, again(STOCK))?,
            countries: (whole || roster.changed()).then(|| Arc::new(countries(roster))),
            templates: Self::templates(state)?,
            tokens: Self::tokens(state)?,
            subunits: Self::wrote(state, TableId::CountryLocalSubunits)?,
            frame: if context_changed || self.engine.as_ref().is_some_and(|engine| !engine.own()) {
                Some(world.frame_held(state, &self.context)?)
            } else {
                None
            },
            designs: if reads_designs {
                Some(world.frame_held(state, &self.designs)?)
            } else {
                None
            },
            definitions,
        };
        let engine = self
            .engine
            .as_mut()
            .ok_or_else(|| invalid("A division equipment projector was not started"))?;
        let projected = match engine.step(&input)? {
            Some(output) => Projected::Hour(Box::new(output)),
            None => Projected::Lead(Box::new(input)),
        };
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            projected,
        })
    }

    fn end(&mut self, out: &mut dyn Sink<Hour>) -> io::Result<()> {
        let Some(last) = self.last else {
            return Ok(());
        };
        let engine = self.engine.take().filter(|engine| engine.own());
        out.push(Hour {
            branch: self.branch,
            hour: last,
            projected: Projected::Handover(engine),
        })
    }
}
