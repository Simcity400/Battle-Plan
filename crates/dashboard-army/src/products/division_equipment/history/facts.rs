//! Division equipment's operator on a columnar walk ([`replay::segments`]): each hour's
//! changed rows, read from the walk's world ([`dashboard_world`]: the countries, the divisions
//! and its views) and the walker's columns, stepped through the segment's own engine. The
//! engine is given values, never frames: each template's facts, resolved at an hour that may
//! have changed them and given again only where they may differ ([`TemplateFacts`]), and at a
//! designs read the definitions it changed, the spellings of their tokens and the countries'
//! tags ([`Designs`]). A reply stands on reads made up to two days before its hour, which a restart does
//! not hold: until the segment's reads are its own, its hours go to the emit as inputs
//! ([`Projected::Lead`]), which the engine of the segment before steps; from then on, as the
//! engine's answers. That engine is handed on with its reads alone ([`Engine::hand_on`]): the
//! segment's first hour gives every other table whole. Every start is the operator's own, so
//! segments walk at once.

use super::engine::{
    Absent, COMPLETE, Context, Countries, DAILY, DEFINITIONS, DefinitionRows, DesignTags, Designs,
    Engine, Entry, Input, NEW, Output, Rows, STOCK, TemplateFacts, Tokens,
};
use super::{KeptReason, kept_artwork, kept_naming};
use crate::products::{
    army_content::{self, Holder},
    division_needs,
};
use dashboard_series::hash::FastMap;
use dashboard_text::{
    catalogue::Catalogue,
    metadata::slot_read,
    subunit_requirements::{Content, SourceIndex, Sources},
};
use dashboard_world::{Hold, IDENTITY, Kept, Needs, Roster, Tagged, World};
use replay::{
    columns,
    segments::{self, Operator, Sink, Start, State},
};
use replay_data::{
    display::stepped_format,
    format::FormatVersion,
    rows::{
        CountryAppearanceRow, CountryRow, DivisionTemplateIconRow, LexicalTokenRow, RowCodec, TableId,
        VariantDefinitionRow, mask, tables,
    },
};
use replay_frame::frame::{HourFrame, OperandStatus, Rowset, Tables};
use std::{
    borrow::Cow,
    cell::OnceCell,
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};

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

/// The fields a design's lookup reads of each table it reads by row.
const COUNTRY: u64 = mask::<CountryRow>(&["canonical_slot", "tag"]);
const APPEARANCE: u64 = mask::<CountryAppearanceRow>(&["name_tag"]);
const DEFINITION: u64 = mask::<VariantDefinitionRow>(&[
    "equipment_token",
    "creator_link",
    "origin_link",
    "append_variant_name",
    "display_name",
    "variant_name",
]);
const LITERAL: u64 = mask::<LexicalTokenRow>(&["literal"]);

/// What a projection reads of its walk's world.
pub fn needs() -> io::Result<Needs> {
    let mut frame = context()?;
    frame.extend([
        (TableId::CountryAppearance, Hold::Fields(APPEARANCE)),
        (TableId::VariantDefinitions, Hold::Fields(DEFINITION)),
    ]);
    Ok(Needs {
        countries: true,
        divisions: true,
        frame,
        ..Needs::default()
    })
}

/// The DivisionTemplateIcons fields a template's required manpower reads.
const SETTINGS: u64 = mask::<DivisionTemplateIconRow>(&["settings"]);

/// A context read's templates resolved as the engine keeps them, the needs' sources read once.
/// It reads the walk's views; a template's rows are keyed by its handle.
struct Resolver<'f> {
    content: &'f Result<Arc<Content>, String>,
    frame: HourFrame<'f>,
    /// The links and local sub-units an earlier read resolved, while they last.
    index: Option<Arc<SourceIndex>>,
    sources: OnceCell<Result<Sources<'f>, String>>,
}

impl<'f> Resolver<'f> {
    fn sources(&self) -> &Result<Sources<'f>, String> {
        self.sources.get_or_init(|| match &self.index {
            Some(index) => division_needs::reused_sources_in(self.frame, Arc::clone(index)),
            None => division_needs::sources_in(self.frame),
        })
    }

    fn complete(&self, table: TableId) -> bool {
        matches!(self.frame.status(table), OperandStatus::Complete)
    }

    /// A template's need, or its refusal as kept: refused at its composition read, after its
    /// content and sources were read, that read's refusal, which names the hour.
    fn need(&self, handle: u64) -> Result<Vec<(String, i64)>, KeptReason> {
        let frame = self.frame;
        let need = division_needs::template_in(self.content, || self.sources(), frame, handle);
        need.map_err(|why| {
            if self.content.is_ok()
                && self.sources().is_ok()
                && let Err(error) = frame.complete::<tables::DivisionTemplateIcons>(0)
            {
                let kept = KeptReason::unreadable(
                    division_needs::COMPOSITION_UNAVAILABLE,
                    frame.hour(),
                    TableId::DivisionTemplateIcons,
                    &error,
                );
                if kept.at(frame.hour()) == why {
                    return kept;
                }
            }
            KeptReason::literal(why)
        })
    }

    fn facts(&self, handle: u64) -> TemplateFacts {
        let need = self.need(handle);
        let required = self
            .frame
            .complete::<tables::DivisionTemplateIcons>(SETTINGS)
            .ok()
            .and_then(|rows| rows.get(u128::from(handle)).ok().flatten())
            .and_then(|icon| {
                icon.settings
                    .require("template settings")
                    .ok()?
                    .as_ref()
                    .map(|settings| i64::from(settings.total_manpower))
            });
        TemplateFacts {
            need,
            required,
            naming: kept_naming(self.frame, handle),
            artwork: kept_artwork(self.frame, handle),
            hourly: army_content::template_naming_hourly_in(self.frame, handle),
        }
    }

    /// Whether a whole table holds a row of `handle`.
    fn holds(&self, handle: u64) -> bool {
        let key = u128::from(handle);
        let templates = self.frame.complete::<tables::DivisionTemplates>(0);
        let icons = self.frame.complete::<tables::DivisionTemplateIcons>(0);
        templates.is_ok_and(|rows| rows.contains(key)) || icons.is_ok_and(|rows| rows.contains(key))
    }

    /// Every template a whole table holds a row of.
    fn held(&self) -> BTreeSet<u64> {
        let mut handles = BTreeSet::new();
        if let Ok(rows) = self.frame.complete::<tables::DivisionTemplates>(0) {
            handles.extend(rows.keys().map(|key| key as u64));
        }
        if let Ok(rows) = self.frame.complete::<tables::DivisionTemplateIcons>(0) {
            handles.extend(rows.keys().map(|key| key as u64));
        }
        handles
    }

    /// What the tables say of a template they hold no row of, checked against the readers at
    /// one such template.
    fn absent(&self) -> io::Result<Absent> {
        let icons = self.complete(TableId::DivisionTemplateIcons);
        let templates = self.complete(TableId::DivisionTemplates);
        let absent = Absent {
            // Before the row is looked up a need's refusal names no template.
            need: if self.content.is_ok() && self.sources().is_ok() && icons {
                None
            } else {
                self.need(0).err()
            },
            naming: (!templates)
                .then(|| kept_naming(self.frame, 0).err())
                .flatten(),
            artwork: (!icons)
                .then(|| kept_artwork(self.frame, 0).err())
                .flatten(),
        };
        let probe = u64::MAX;
        if !self.holds(probe) && self.facts(probe) != absent.facts(probe) {
            return Err(invalid(
                "Division equipment's facts of a template no table holds differ from its readers'",
            ));
        }
        Ok(absent)
    }

    /// The sources it read, to keep for the reads after while their tables last.
    fn index(&self) -> Option<Arc<SourceIndex>> {
        match self.sources.get() {
            Some(Ok(sources)) => Some(Arc::clone(sources.index())),
            _ => None,
        }
    }
}

/// The templates the engine was given facts of, with those named for an hour alone and those
/// whose need was refused: what a read given in part asks again.
#[derive(Default)]
struct Held {
    templates: BTreeSet<u64>,
    hourly: BTreeSet<u64>,
    refused: BTreeSet<u64>,
}

impl Held {
    fn note(&mut self, handle: u64, facts: &TemplateFacts) {
        self.templates.insert(handle);
        for (set, holds) in [
            (&mut self.hourly, facts.hourly),
            (&mut self.refused, facts.need.is_err()),
        ] {
            if holds {
                set.insert(handle);
            } else {
                set.remove(&handle);
            }
        }
    }

    /// Lets go of `handle`: whether it was held.
    fn forget(&mut self, handle: u64) -> bool {
        self.hourly.remove(&handle);
        self.refused.remove(&handle);
        self.templates.remove(&handle)
    }
}

/// `rows` as owned rows, in key order.
fn owned<R: RowCodec + Clone>(rows: Rowset<'_, R>) -> io::Result<Vec<R>> {
    rows.iter().map(|row| row.map(Cow::into_owned)).collect()
}

/// The countries a design's lookup names it for, from `frame`'s views; none where Countries or
/// CountryLinks does not hold every row.
fn design_tags<F: Tables + ?Sized>(frame: &F) -> io::Result<Option<Arc<DesignTags>>> {
    let (Ok(countries), Ok(links)) = (
        frame.complete_rows::<tables::Countries>(COUNTRY),
        frame.complete_rows::<tables::CountryLinks>(u64::MAX),
    ) else {
        return Ok(None);
    };
    let appearance = match frame.complete_rows::<tables::CountryAppearance>(APPEARANCE) {
        Ok(rows) => Some(owned(rows)?),
        Err(_) => None,
    };
    Ok(Some(Arc::new(DesignTags {
        countries: owned(countries)?,
        links: owned(links)?,
        appearance,
    })))
}

/// One projected item: an hour another engine answers, an hour answered, or the segment's
/// engine at its end.
#[derive(replay_spool::Spooled)]
pub(crate) enum Projected {
    /// An hour before the segment's reads are its own: the engine of the segment before
    /// steps it.
    Lead(Box<Input>),
    Hour(Box<Output>),
    /// The segment's engine after its last hour, holding its reads alone; none when it
    /// answered none of them.
    Handover(Option<Box<Engine>>),
}

/// A projected item with its branch.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    pub(crate) projected: Projected,
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
    content: Result<Arc<Content>, String>,
    branch: u64,
    last: Option<i32>,
    complete: [bool; 7],
    /// Designs whose definitions changed since the last designs read; none while unknown.
    definitions: Option<BTreeSet<u64>>,
    /// The templates the engine holds facts of; none before the walk's first context read.
    held: Option<Held>,
    /// The links and local sub-units the needs resolve through, while they last.
    sources: Option<Arc<SourceIndex>>,
    /// Countries or CountryLinks moved since the last context read.
    moved: bool,
    /// The walk gave a designs read: the next gives only the definitions it changed.
    designed: bool,
    /// The equipment token each design's last given definition names, and how many name each,
    /// as the engine's definitions do.
    design_tokens: FastMap<u64, u32>,
    named: BTreeMap<u32, u32>,
    /// The countries' tags as last read, and whether a table they are read from moved since.
    tags: Option<Option<Arc<DesignTags>>>,
    tags_moved: bool,
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
        let content = division_needs::content(cat.as_ref());
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
            content,
            branch: 0,
            last: None,
            complete: [false; 7],
            definitions: None,
            held: None,
            sources: None,
            moved: false,
            designed: false,
            design_tokens: FastMap::default(),
            named: BTreeMap::new(),
            tags: None,
            tags_moved: false,
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

    /// The templates' facts at an hour that may have changed them: every template's where any
    /// table they are read from but the templates' own may have moved, else those of the
    /// templates whose rows it wrote, those named for an hour alone, and after new tokens alone
    /// those whose need was refused.
    fn context(&mut self, world: &World, state: &State, whole: bool) -> io::Result<Context> {
        let frame = HourFrame::from(world.columns(state));
        let tokens = Self::tokens(state)?;
        let subunits = Self::wrote(state, TableId::CountryLocalSubunits)?;
        let changed = Self::templates(state)?;
        let moved = std::mem::take(&mut self.moved);
        let complete = |table: TableId| matches!(frame.status(table), OperandStatus::Complete);
        let wholes = CONTEXT.map(complete);
        let moved_all = whole
            || self.held.is_none()
            || moved
            || subunits
            || tokens == Tokens::Changed
            || changed.is_none()
            || !CONTEXT.into_iter().chain(IDENTITY).all(complete)
            || CONTEXT
                .into_iter()
                .chain(IDENTITY)
                .any(|table| world.restated(table));
        if moved_all || tokens != Tokens::Same {
            self.sources = None;
        }
        let resolver = Resolver {
            content: &self.content,
            frame,
            index: self.sources.clone(),
            sources: OnceCell::new(),
        };
        // A refused read of the sources may name the hour.
        let every = moved_all || (self.content.is_ok() && resolver.sources().is_err());
        let held = self.held.get_or_insert_with(Held::default);
        let mut facts = Vec::new();
        let mut gone = Vec::new();
        if every {
            *held = Held::default();
            for handle in resolver.held() {
                let read = resolver.facts(handle);
                held.note(handle, &read);
                facts.push((handle, Arc::new(read)));
            }
        } else {
            let mut asked: BTreeSet<u64> = changed.unwrap_or_default().into_iter().collect();
            asked.extend(held.hourly.iter().copied());
            if tokens == Tokens::Appended {
                asked.extend(held.refused.iter().copied());
            }
            for handle in asked {
                if resolver.holds(handle) {
                    let read = resolver.facts(handle);
                    held.note(handle, &read);
                    facts.push((handle, Arc::new(read)));
                } else if held.forget(handle) {
                    gone.push(handle);
                }
            }
        }
        let absent = resolver.absent()?;
        if self.sources.is_none() {
            self.sources = resolver.index();
        }
        Ok(Context {
            wholes,
            whole: every,
            facts,
            gone,
            absent: Arc::new(absent),
        })
    }

    /// What a designs read gives: the definitions changed since the last (every one at the
    /// walk's first, or while the changes are unknown), the spellings of the tokens the
    /// definitions name, and the countries' tags.
    fn designs(
        &mut self,
        world: &World,
        state: &State,
        changed: Option<&[u64]>,
    ) -> io::Result<Designs> {
        let views = world.columns(state);
        let rows = views.complete_rows::<tables::VariantDefinitions>(DEFINITION)?;
        let definitions = match changed.filter(|_| self.designed) {
            Some(changed) => {
                let mut set = engine_allocation::vec(changed.len())?;
                for &id in changed {
                    let row = rows
                        .get(u128::from(id))?
                        .map(|row| Arc::new(row.into_owned()));
                    self.name(id, row.as_ref().map(|row| row.equipment_token));
                    set.push((id, row));
                }
                DefinitionRows::Changed(set)
            }
            None => {
                self.design_tokens.clear();
                self.named.clear();
                let mut every = engine_allocation::vec(rows.len())?;
                for row in rows.iter() {
                    let row = Arc::new(row?.into_owned());
                    self.name(row.id, Some(row.equipment_token));
                    every.push(row);
                }
                DefinitionRows::Whole(every)
            }
        };
        self.designed = true;
        let tokens = match views.complete_rows::<tables::LexicalTokens>(LITERAL) {
            Ok(lexical) => {
                let mut spelled = engine_allocation::vec(self.named.len())?;
                for &token in self.named.keys() {
                    if let Some(row) = lexical.get(u128::from(token))? {
                        spelled.push(row.into_owned());
                    }
                }
                Some(spelled)
            }
            Err(_) => None,
        };
        if std::mem::take(&mut self.tags_moved) || self.tags.is_none() {
            self.tags = Some(design_tags(&views)?);
        }
        Ok(Designs {
            definitions,
            tokens,
            tags: self.tags.clone().flatten(),
        })
    }

    /// Design `id`'s definition, naming `token`, as given; none where it is gone.
    fn name(&mut self, id: u64, token: Option<u32>) {
        if let Some(was) = self.design_tokens.remove(&id)
            && let Some(count) = self.named.get_mut(&was)
        {
            *count -= 1;
            if *count == 0 {
                self.named.remove(&was);
            }
        }
        if let Some(token) = token {
            self.design_tokens.insert(id, token);
            *self.named.entry(token).or_default() += 1;
        }
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
        self.held = None;
        self.sources = None;
        self.moved = false;
        self.designed = false;
        self.design_tokens = FastMap::default();
        self.named = BTreeMap::new();
        self.tags = None;
        self.tags_moved = false;
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
        let identity = world.touched(TableId::Countries, COUNTRY)
            || world.touched(TableId::CountryLinks, u64::MAX);
        self.moved |= identity;
        self.tags_moved |= identity || world.touched(TableId::CountryAppearance, APPEARANCE);
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
        let context = if context_changed {
            Some(self.context(world, state, whole)?)
        } else {
            None
        };
        let designs = if reads_designs {
            Some(self.designs(world, state, definitions.as_deref())?)
        } else {
            None
        };
        let input = Input {
            hour: at,
            whole,
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
            context,
            designs,
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
        let engine = self.engine.take().filter(|engine| engine.own()).map(|mut engine| {
            engine.hand_on();
            engine
        });
        out.push(Hour {
            branch: self.branch,
            hour: last,
            projected: Projected::Handover(engine),
        })
    }
}
