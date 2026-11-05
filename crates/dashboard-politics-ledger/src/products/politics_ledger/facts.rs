//! The ledger's facts, projected hour by hour from a columnar walk ([`replay::segments`]): the
//! native power, government, daily and cost rows the hour inserted or rewrote, typed from their
//! columns; the captures it changed, with those it only extended to the hour as one list; the
//! command power allocations it changed; and the census and links from the walk's world
//! ([`dashboard_world`]). A segment's first hour, and any after a skipped hour, hands every row.
//!
//! What only an hour's frame says of a row (its item's operands, names and scripted text,
//! [`items`]) is read for the rows dated at the hour, from the walk's frame asked at that hour
//! alone. Rows a capture may publish a read late are dated the hour before: after a read whose
//! capture interval starts at the hour, its frame is kept for the next hour.

use super::items::{self, Dated, FrameTexts, Item, NamedCountry};
use super::*;
use dashboard_base::products::idea_format;
use dashboard_base::products::politics_content::PoliticsContent;
use dashboard_world::{Hold, Needs, Roster, World};
use replay::{
    columns,
    segments::{self, Operator, Sink, State},
};
use replay_data::format::FormatVersion;
use std::sync::LazyLock;

/// The ledger's own tables: what the accounting reads besides the world's roster.
const NATIVE: [TableId; 8] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::PoliticsCaptureCoverage,
    TableId::CountryPowerChanges,
    TableId::GovernmentChanges,
    TableId::CountryPowerDaily,
    TableId::CountryPowerDailyCosts,
    TableId::CountryCommandPower,
];

/// The tables an item's operands, names and scripted text read at its hour.
fn kit() -> &'static [TableId] {
    static KIT: LazyLock<Vec<TableId>> = LazyLock::new(|| {
        let mut tables = vec![
            TableId::CharacterPortraits,
            TableId::AdvisorRoles,
            TableId::Characters,
            TableId::UnitLeaders,
            TableId::Countries,
            TableId::CountryLinks,
            TableId::LexicalTokens,
            TableId::ModifierDefinitions,
            TableId::CountryAppearance,
            TableId::Politics,
            TableId::PoliticalParties,
            TableId::Autonomy,
            TableId::CountryCivilWars,
            TableId::CountryCounterInputs,
            TableId::CountryWars,
            TableId::WarRelations,
            TableId::IndustrialOrganisations,
            TableId::States,
        ];
        tables.extend(installed_content_country_names::country_names::TABLES);
        tables.extend_from_slice(dashboard_text::triggers::RECORDED_TABLES);
        tables.sort_unstable();
        tables.dedup();
        tables
    });
    &KIT
}

/// Every table the family's operators read.
pub fn tables() -> &'static [TableId] {
    static TABLES: LazyLock<Vec<TableId>> = LazyLock::new(|| {
        let mut tables = vec![TableId::Campaign];
        tables.extend(NATIVE);
        tables.extend_from_slice(kit());
        tables.sort_unstable();
        tables.dedup();
        tables
    });
    &TABLES
}

/// What the family reads of its walk's world: the countries, and the frame of its item tables.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        frame: kit().iter().map(|&table| (table, Hold::Whole)).collect(),
        ..Needs::default()
    }
}

/// A table's completeness bit in [`Hour::complete`], in [`NATIVE`] order.
pub(super) fn bit(table: TableId) -> u16 {
    NATIVE
        .iter()
        .position(|&native| native == table)
        .map_or(0, |at| 1 << at)
}

/// LexicalTokens completeness, after the native tables' bits.
pub(super) const LEXICAL: u16 = 1 << 8;

/// A Countries row's identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CensusRow {
    pub id: u32,
    pub slot: u32,
    pub tag: Arc<[u8]>,
}

/// What the hour changed of the captures.
#[derive(Clone, Debug, Default)]
pub struct Captures {
    /// Rows the hour wrote other than by extending them to the hour; every row when whole.
    pub changed: Vec<PoliticsCaptureCoverageRow>,
    /// The raw countries whose capture the hour removed.
    pub deleted: Vec<u32>,
    /// The raw countries whose capture the hour only extended to the hour, ascending: the hour
    /// before's list while it stays the same.
    pub extended: Arc<[u32]>,
}

/// A power or cost row and what its hour says of its item.
#[derive(Clone, Debug)]
pub struct Dating {
    /// Its item's facts at its hour, encoded; None where no frame of its hour was read.
    pub dated: Option<DatedFacts>,
    /// The category its payment is spent in, for a payment.
    pub category: Option<Arc<str>>,
}

/// An item's facts at its hour, MessagePack, and their digest.
#[derive(Clone, Debug)]
pub struct DatedFacts {
    pub bytes: Arc<[u8]>,
    pub digest: [u8; 32],
}

impl DatedFacts {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            digest: *blake3::hash(&bytes).as_bytes(),
            bytes: Arc::from(bytes),
        }
    }
}

impl Dating {
    fn digest(&self) -> Option<&[u8; 32]> {
        self.dated.as_ref().map(|dated| &dated.digest)
    }
}

#[derive(Clone, Debug)]
pub struct Power {
    pub row: CountryPowerChangeRow,
    pub dating: Dating,
    /// Its item and holder, worked out with its facts; None to work them out when recorded.
    pub resolved: Option<Resolved>,
}

/// What a power row's facts make of it ([`rows::power_item`], [`rows::holder`]).
#[derive(Clone, Debug)]
pub struct Resolved {
    pub(super) item: [u8; 32],
    pub(super) holder: Option<rows::Holder>,
}

impl Power {
    pub fn resolve(&self) -> Resolved {
        Resolved {
            item: rows::power_item(&self.row, self.dating.digest()),
            holder: rows::holder(
                &self.row,
                self.dating.dated.as_ref().map(|dated| &*dated.bytes),
            ),
        }
    }

    /// Its row as no frame of its hour dated it.
    pub fn undate(&mut self) {
        self.dating.dated = None;
        self.resolved = None;
    }
}

#[derive(Clone, Debug)]
pub struct Cost {
    pub row: CountryPowerDailyCostRow,
    pub dating: Dating,
    /// Its item ([`rows::cost_item`]), worked out with its facts; None to work it out when
    /// recorded.
    pub item: Option<[u8; 32]>,
}

impl Cost {
    pub fn resolve(&self) -> [u8; 32] {
        rows::cost_item(&self.row, self.dating.digest())
    }

    /// Its row as no frame of its hour dated it.
    pub fn undate(&mut self) {
        self.dating.dated = None;
        self.item = None;
    }
}

#[derive(Clone, Debug)]
pub struct Government {
    pub row: GovernmentChangeRow,
    /// Its ideologies' spellings at its hour.
    pub tokens: (Option<String>, Option<String>),
}

/// One projected hour.
#[derive(Default)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Every table is handed whole: one left out holds no row.
    pub whole: bool,
    /// The native tables complete this hour ([`bit`]), and LexicalTokens ([`LEXICAL`]).
    pub complete: u16,
    /// Every Countries row by id, when the census changed or `whole`.
    pub census: Option<Arc<[CensusRow]>>,
    /// Every link by raw id, when the links changed or `whole`.
    pub links: Option<Arc<[(u32, u32)]>>,
    pub captures: Captures,
    pub power: Vec<Power>,
    pub governments: Vec<Government>,
    pub daily: Vec<CountryPowerDailyRow>,
    pub costs: Vec<Cost>,
    /// The keys the hour deleted from the power, government, daily and cost tables.
    pub deleted: [Vec<u128>; 4],
    /// Each changed command power allocation by canonical slot; None where its row left.
    pub command: Vec<(u32, Option<i64>)>,
    /// `command` holds every live allocation: one held before that it omits left.
    pub command_whole: bool,
}

impl segments::Item for Hour {
    fn heap(&self) -> usize {
        use std::mem::size_of;
        let dating = |dating: &Dating| {
            dating.dated.as_ref().map_or(0, |dated| dated.bytes.len())
                + dating.category.as_ref().map_or(0, |text| text.len())
        };
        let holder = |power: &Power| {
            power
                .resolved
                .as_ref()
                .and_then(|resolved| resolved.holder.as_ref())
                .map_or(0, |holder| {
                    holder.kind.capacity()
                        + holder.token.as_ref().map_or(0, String::capacity)
                        + holder.character.as_ref().map_or(0, |(script, saved)| {
                            script.as_ref().map_or(0, String::capacity) + saved.capacity()
                        })
                })
        };
        let identity = |identity: &PowerIdentity| {
            identity.item_key.capacity()
                + identity.slot_key.capacity()
                + identity.origin_key.capacity()
        };
        self.census.as_ref().map_or(0, |census| {
            census.len() * size_of::<CensusRow>()
                + census.iter().map(|row| row.tag.len()).sum::<usize>()
        }) + self
            .links
            .as_ref()
            .map_or(0, |links| links.len() * size_of::<(u32, u32)>())
            + self.captures.changed.capacity() * size_of::<PoliticsCaptureCoverageRow>()
            + self.captures.deleted.capacity() * size_of::<u32>()
            + self.power.capacity() * size_of::<Power>()
            + self
                .power
                .iter()
                .map(|power| {
                    dating(&power.dating) + identity(&power.row.identity) + holder(power)
                })
                .sum::<usize>()
            + self.governments.capacity() * size_of::<Government>()
            + self
                .governments
                .iter()
                .map(|government| {
                    government.row.origin_key.capacity()
                        + government.tokens.0.as_ref().map_or(0, String::len)
                        + government.tokens.1.as_ref().map_or(0, String::len)
                })
                .sum::<usize>()
            + self.daily.capacity() * size_of::<CountryPowerDailyRow>()
            + self.costs.capacity() * size_of::<Cost>()
            + self
                .costs
                .iter()
                .map(|cost| dating(&cost.dating) + identity(&cost.row.identity))
                .sum::<usize>()
            + self
                .deleted
                .iter()
                .map(|keys| keys.capacity() * size_of::<u128>())
                .sum::<usize>()
            + self.command.capacity() * size_of::<(u32, Option<i64>)>()
    }
}

/// Columns read in place.
struct Fields {
    through: usize,
    /// A capture's epoch, first hour, streams and instance.
    capture: [usize; 4],
    allocated: usize,
    literal: usize,
}

/// One segment's projection.
pub struct Projector {
    format: FormatVersion,
    content: Option<Arc<PoliticsContent>>,
    fields: Fields,
    last: Option<i32>,
    /// The native tables' completeness the hour before, in [`NATIVE`] order.
    complete: u16,
    /// The last extension list handed.
    extended: Arc<[u32]>,
    /// The item frame of the hour before, kept after a read whose capture may publish rows a
    /// read late.
    held: Option<DecodedFrame>,
    /// The recorded modifier definitions by name, as the item frame last indexed them, with
    /// the views they were read from.
    modifiers: Option<(
        replay_frame::frame::ViewIdentity,
        replay_frame::frame::ViewIdentity,
        Option<Arc<BTreeMap<String, ModifierDefinitionRow>>>,
    )>,
}

/// The slots `table`'s hour inserted, or wrote any of `mask` at, ascending.
fn written(state: &State, table: TableId, mask: u64) -> io::Result<Vec<u32>> {
    let changes = columns::changes(state, table)?;
    let mut slots: Vec<u32> = changes
        .changed
        .iter()
        .zip(&changes.masks)
        .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
        .filter(|&(_, &written)| written & mask != 0)
        .map(|(&slot, _)| slot)
        .chain(changes.inserted.iter().copied())
        .collect();
    slots.sort_unstable();
    slots.dedup();
    Ok(slots)
}

/// Every live slot of `table`, ascending.
fn live(state: &State, table: TableId) -> io::Result<Vec<u32>> {
    Ok(columns::table(state, table)?.live().ones().collect())
}

/// Whether a payment of this power row resolves an item a reply shows: a charge, a balance
/// line resolved by its item, or a hold naming its allocator.
fn named(row: &CountryPowerChangeRow) -> bool {
    !matches!(
        row.cause,
        PowerCause::CapCorrection | PowerCause::ResetSetup | PowerCause::Console
    ) && !(row.cause == PowerCause::DailyGain && row.currency == PowerCurrency::Political)
}

impl Projector {
    pub fn new(
        format: FormatVersion,
        content: &Result<Option<Arc<PoliticsContent>>, String>,
    ) -> io::Result<Self> {
        Ok(Self {
            format,
            content: content.as_ref().ok().cloned().flatten(),
            fields: Fields {
                through: columns::field(TableId::PoliticsCaptureCoverage, "through_hour")?,
                capture: [
                    columns::field(TableId::PoliticsCaptureCoverage, "epoch")?,
                    columns::field(TableId::PoliticsCaptureCoverage, "since_hour")?,
                    columns::field(TableId::PoliticsCaptureCoverage, "streams")?,
                    columns::field(TableId::PoliticsCaptureCoverage, "instance")?,
                ],
                allocated: columns::field(
                    TableId::CountryCommandPower,
                    "allocated_command_power_raw",
                )?,
                literal: columns::field(TableId::LexicalTokens, "literal")?,
            },
            last: None,
            complete: 0,
            extended: Arc::from(Vec::new()),
            held: None,
            modifiers: None,
        })
    }

    /// `table`'s rows the hour inserted or wrote, every live row when `whole`, and the keys it
    /// deleted; none of a table that is not complete.
    fn rows<R: RowCodec>(
        &self,
        state: &State,
        table: TableId,
        whole: bool,
    ) -> io::Result<(Vec<R>, Vec<u128>)> {
        if !columns::complete(state, table) {
            return Ok((Vec::new(), Vec::new()));
        }
        let slots = if whole {
            live(state, table)?
        } else {
            written(state, table, u64::MAX)?
        };
        let mut rows = Vec::with_capacity(slots.len());
        for slot in slots {
            rows.push(columns::row::<R>(
                state,
                table,
                slot,
                self.format.version(),
            )?);
        }
        let deleted = if whole {
            Vec::new()
        } else {
            columns::changes(state, table)?.deleted_keys.clone()
        };
        Ok((rows, deleted))
    }

    /// The capture `slot` holds, read in place where its columns are recorded and valid, else
    /// as the row decoder reads it.
    fn capture(&self, state: &State, slot: u32) -> io::Result<PoliticsCaptureCoverageRow> {
        let table = TableId::PoliticsCaptureCoverage;
        let held = columns::table(state, table)?;
        let [epoch, since, streams, instance] = self.fields.capture;
        let at = slot as usize;
        let fast = (|| {
            if ![epoch, since, self.fields.through, streams, instance]
                .iter()
                .all(|&field| held.recorded(field, slot))
            {
                return None;
            }
            let row = PoliticsCaptureCoverageRow {
                country: u32::try_from(held.key(slot)?).ok()?,
                instance: *columns::plain::<u32>(held, instance).ok()?.get(at)?,
                epoch: *columns::plain::<u64>(held, epoch).ok()?.get(at)?,
                since_hour: *columns::plain::<i32>(held, since).ok()?.get(at)?,
                through_hour: *columns::plain::<i32>(held, self.fields.through)
                    .ok()?
                    .get(at)?,
                streams: *columns::plain::<u8>(held, streams).ok()?.get(at)?,
            };
            (row.since_hour <= row.through_hour
                && row.streams & !PoliticsCaptureCoverageRow::ALL_STREAMS == 0)
                .then_some(row)
        })();
        match fast {
            Some(row) => Ok(row),
            None => columns::row(state, table, slot, self.format.version()),
        }
    }

    /// The captures the hour changed, and those it only extended to the hour.
    fn captures(&mut self, state: &State, hour: i32, whole: bool) -> io::Result<Captures> {
        let table = TableId::PoliticsCaptureCoverage;
        if !columns::complete(state, table) {
            self.extended = Arc::from(Vec::new());
            return Ok(Captures {
                extended: Arc::clone(&self.extended),
                ..Captures::default()
            });
        }
        if whole {
            let mut changed = Vec::new();
            for slot in live(state, table)? {
                changed.push(self.capture(state, slot)?);
            }
            self.extended = Arc::from(Vec::new());
            return Ok(Captures {
                changed,
                deleted: Vec::new(),
                extended: Arc::clone(&self.extended),
            });
        }
        let held = columns::table(state, table)?;
        let changes = columns::changes(state, table)?;
        let through = columns::plain::<i32>(held, self.fields.through)?;
        let only = 1_u64 << self.fields.through;
        // A slot's columns written this hour, by its runs and fills together.
        let mut masks: Vec<(u32, u64)> = changes
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
            .collect();
        masks.sort_unstable_by_key(|&(slot, _)| slot);
        let mut extended = Vec::new();
        let mut changed = Vec::new();
        let mut at = 0;
        while let Some(&(slot, _)) = masks.get(at) {
            let mut mask = 0;
            while let Some(&(_, bits)) = masks.get(at).filter(|&&(held, _)| held == slot) {
                mask |= bits;
                at += 1;
            }
            if mask == 0 {
                continue;
            }
            if mask == only
                && through.get(slot as usize) == Some(&hour)
                && changes.inserted.binary_search(&slot).is_err()
            {
                let key = held
                    .key(slot)
                    .ok_or_else(|| dashboard_core::invalid("A changed capture holds no row"))?;
                extended.push(key as u32);
            } else {
                changed.push(self.capture(state, slot)?);
            }
        }
        for &slot in &changes.inserted {
            if masks
                .binary_search_by_key(&slot, |&(slot, _)| slot)
                .is_err()
            {
                changed.push(self.capture(state, slot)?);
            }
        }
        extended.sort_unstable();
        if *self.extended != *extended {
            self.extended = Arc::from(extended);
        }
        Ok(Captures {
            changed,
            deleted: changes.deleted_keys.iter().map(|&key| key as u32).collect(),
            extended: Arc::clone(&self.extended),
        })
    }

    /// The command power allocations the hour changed, every one when `whole`.
    fn command(&self, state: &State, whole: bool) -> io::Result<Vec<(u32, Option<i64>)>> {
        let table = TableId::CountryCommandPower;
        if !columns::complete(state, table) {
            return Ok(Vec::new());
        }
        let held = columns::table(state, table)?;
        let allocated = columns::plain::<i64>(held, self.fields.allocated)?;
        let slots = if whole {
            live(state, table)?
        } else {
            written(state, table, 1 << self.fields.allocated)?
        };
        let mut command = Vec::with_capacity(slots.len());
        for slot in slots {
            let key = held
                .key(slot)
                .ok_or_else(|| dashboard_core::invalid("A command power row holds no key"))?;
            let value = if held.recorded(self.fields.allocated, slot) {
                allocated.get(slot as usize).copied().unwrap_or_default()
            } else {
                columns::row::<CountryCommandPowerRow>(state, table, slot, self.format.version())?
                    .allocated_command_power_raw
            };
            command.push((key as u32, Some(value)));
        }
        if !whole {
            command.extend(
                columns::changes(state, table)?
                    .deleted_keys
                    .iter()
                    .map(|&key| (key as u32, None)),
            );
        }
        Ok(command)
    }

    /// `token`'s spelling at the hour, as LexicalTokens holds it.
    fn spelling(&self, state: &State, token: u32) -> io::Result<Option<String>> {
        if !columns::complete(state, TableId::LexicalTokens) {
            return Ok(None);
        }
        let held = columns::table(state, TableId::LexicalTokens)?;
        let Some(slot) = held.slot(u128::from(token)) else {
            return Ok(None);
        };
        let bytes = if held.recorded(self.fields.literal, slot) {
            let id = *columns::text(held, self.fields.literal)?
                .get(slot as usize)
                .ok_or_else(|| dashboard_core::invalid("A token holds no spelling cell"))?;
            state
                .dictionary()
                .get(id)
                .ok_or_else(|| dashboard_core::invalid("A spelling names no dictionary entry"))?
                .to_vec()
        } else {
            columns::row::<LexicalTokenRow>(
                state,
                TableId::LexicalTokens,
                slot,
                self.format.version(),
            )?
            .literal
        };
        Ok(String::from_utf8(bytes).ok())
    }

    /// The modifier definitions `frame` records by name; None unless it holds both tables.
    fn modifier_index(
        &mut self,
        frame: &DecodedFrame,
    ) -> Option<Arc<BTreeMap<String, ModifierDefinitionRow>>> {
        let definitions = &frame.tables[TableId::ModifierDefinitions.index()];
        let lexical = &frame.tables[TableId::LexicalTokens.index()];
        if let Some((held_definitions, held_lexical, index)) = &self.modifiers
            && held_definitions.is(definitions)
            && held_lexical.is(lexical)
        {
            return index.clone();
        }
        let index = match (
            frame.complete::<tables::ModifierDefinitions>(),
            frame.complete::<tables::LexicalTokens>(),
        ) {
            (Ok(rows), Ok(tokens)) => Some(Arc::new(idea_format::index_recorded(rows, tokens))),
            _ => None,
        };
        self.modifiers = Some((definitions.identity(), lexical.identity(), index.clone()));
        index
    }

    /// What `frame`, the read at the rows' own hour, says of `rows`' items; `current` names
    /// countries through the walk's roster, the hour before's frame through its own links.
    fn date(
        &mut self,
        frame: &DecodedFrame,
        roster: &Roster,
        current: bool,
        rows: &mut [(
            Item<'_>,
            Option<u32>,
            u32,
            Option<PowerDailyCostKind>,
            &mut Dating,
        )],
    ) -> io::Result<()> {
        let content = self.content.clone();
        let mut raws = BTreeSet::new();
        for (item, _, country, _, _) in rows.iter() {
            raws.insert(*country);
            raws.extend(item.identity.target_country);
        }
        let names = self.names(frame, roster, current, &raws);
        let modifiers = if rows.iter().any(|(item, ..)| {
            matches!(
                item.cause,
                PowerCause::Idea | PowerCause::Advisor | PowerCause::AdvisorRemoval
            )
        }) {
            self.modifier_index(frame)
        } else {
            None
        };
        let texts = FrameTexts::default();
        for (item, _, country, component, dating) in rows.iter_mut() {
            let mut facts = items::event_facts(Some(frame), item.identity, item.cause, *component);
            let own = names.get(country).cloned();
            let target = item
                .identity
                .target_country
                .and_then(|raw| names.get(&raw).cloned());
            let mut modifier_rows = None;
            if let Some(content) = content.as_deref() {
                facts.text =
                    items::item_text(own.as_ref(), item, &facts, content, Some(frame), &texts);
                if matches!(
                    item.cause,
                    PowerCause::Idea | PowerCause::Advisor | PowerCause::AdvisorRemoval
                ) {
                    modifier_rows = items::tooltip_modifiers(
                        content,
                        &facts,
                        own.as_ref(),
                        item.identity,
                        modifiers.as_deref(),
                    );
                    if modifiers.is_none() {
                        modifier_rows = None;
                    }
                }
                dating.category = dating
                    .category
                    .take()
                    .map(|_| category(item, &facts, *component, Some(content)));
            }
            let value = Dated {
                facts,
                own,
                target,
                modifiers: modifier_rows,
            };
            dating.dated = Some(DatedFacts::new(dashboard_base::prepared::codec::encode(
                &value,
            )?));
        }
        Ok(())
    }

    /// The names at `frame`'s hour of `raws`.
    fn names(
        &self,
        frame: &DecodedFrame,
        roster: &Roster,
        current: bool,
        raws: &BTreeSet<u32>,
    ) -> BTreeMap<u32, NamedCountry> {
        let links = frame.complete::<tables::CountryLinks>().ok();
        let countries = frame.complete::<tables::Countries>().unwrap_or_default();
        let slots: Vec<(u32, &CountryRow)> = raws
            .iter()
            .filter_map(|&raw| {
                let country = if current {
                    let id = roster.at_slot(roster.slot_of(raw)?).last()?.id;
                    countries
                        .binary_search_by_key(&id, |row| row.id)
                        .ok()
                        .map(|index| &countries[index])
                } else {
                    let slot =
                        links.and_then(|links| dashboard_text::metadata::link_slot(links, raw))?;
                    countries
                        .iter()
                        .rev()
                        .find(|row| row.canonical_slot == slot)
                }?;
                Some((raw, country))
            })
            .collect();
        items::named_countries(frame, self.content.as_deref(), &slots)
            .into_iter()
            .collect()
    }
}

/// The category a payment of `item` is spent in.
fn category(
    item: &Item<'_>,
    facts: &items::EventFacts,
    component: Option<PowerDailyCostKind>,
    content: Option<&PoliticsContent>,
) -> Arc<str> {
    if component.is_some() && item.cause == PowerCause::DailyGain {
        return Arc::from(match component {
            Some(PowerDailyCostKind::MioResearch | PowerDailyCostKind::MioProduction) => {
                "organisationUpkeep"
            }
            _ => "dailyCosts",
        });
    }
    let token = items::recorded_token(facts, item.identity);
    Arc::from(present::category_of(item.cause, token, content).0)
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn hour(
        &mut self,
        walked: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("politics_ledger.facts", None);
        let state = walked.state;
        let hour = walked.hour;
        let whole = walked.starts
            || self
                .last
                .is_none_or(|last| last.checked_add(1) != Some(hour));
        let held = std::mem::take(&mut self.held)
            .filter(|held| held.branch == walked.branch && held.hour.checked_add(1) == Some(hour));
        self.last = Some(hour);
        let roster = walked.world.countries()?;
        let mut complete = 0;
        for table in NATIVE {
            if columns::complete(state, table) {
                complete |= bit(table);
            }
        }
        if columns::complete(state, TableId::LexicalTokens) {
            complete |= LEXICAL;
        }
        // A table that becomes complete is read whole: its rows were not followed.
        let regained = complete & !self.complete;
        self.complete = complete & !LEXICAL;
        let whole_of = |table: TableId| whole || regained & bit(table) != 0;
        let census = (whole || roster.changed() || regained & bit(TableId::Countries) != 0).then(
            || -> Arc<[CensusRow]> {
                roster
                    .iter()
                    .map(|country| CensusRow {
                        id: country.id,
                        slot: country.slot,
                        tag: Arc::from(roster.tag(country.tag)),
                    })
                    .collect()
            },
        );
        let links = (whole
            || roster.rebuilt()
            || !roster.changed_links().is_empty()
            || regained & bit(TableId::CountryLinks) != 0)
            .then(|| -> Arc<[(u32, u32)]> {
                roster
                    .links()
                    .iter()
                    .map(|link| (link.id, link.canonical_slot))
                    .collect()
            });
        let captures = self.captures(state, hour, whole_of(TableId::PoliticsCaptureCoverage))?;
        let (power, power_deleted) = self.rows::<CountryPowerChangeRow>(
            state,
            TableId::CountryPowerChanges,
            whole_of(TableId::CountryPowerChanges),
        )?;
        let (governments, government_deleted) = self.rows::<GovernmentChangeRow>(
            state,
            TableId::GovernmentChanges,
            whole_of(TableId::GovernmentChanges),
        )?;
        let (daily, daily_deleted) = self.rows::<CountryPowerDailyRow>(
            state,
            TableId::CountryPowerDaily,
            whole_of(TableId::CountryPowerDaily),
        )?;
        let (costs, cost_deleted) = self.rows::<CountryPowerDailyCostRow>(
            state,
            TableId::CountryPowerDailyCosts,
            whole_of(TableId::CountryPowerDailyCosts),
        )?;
        let command_whole = whole_of(TableId::CountryCommandPower);
        let command = self.command(state, command_whole)?;
        let mut power: Vec<Power> = power
            .into_iter()
            .map(|row| Power {
                dating: Dating {
                    dated: None,
                    category: rows::charge(&row).map(|_| Arc::from("")),
                },
                row,
                resolved: None,
            })
            .collect();
        let mut costs: Vec<Cost> = costs
            .into_iter()
            .map(|row| Cost {
                dating: Dating {
                    dated: None,
                    category: (row.cost_raw != 0).then(|| Arc::from("")),
                },
                row,
                item: None,
            })
            .collect();
        // The rows dated at the hour, and at the hour before when its frame is held.
        for (at, frame) in [(hour, None), (hour.saturating_sub(1), held.as_ref())] {
            if at != hour && frame.is_none() {
                continue;
            }
            let wanted = power
                .iter()
                .any(|power| power.row.hour == at && named(&power.row))
                || costs
                    .iter()
                    .any(|cost| cost.row.day_hour == at && cost.row.cost_raw != 0);
            if !wanted {
                continue;
            }
            let current;
            let frame = match frame {
                Some(frame) => frame,
                None => {
                    current = walked.world.frame(state, kit())?;
                    &current
                }
            };
            let mut dated: Vec<(
                Item<'_>,
                Option<u32>,
                u32,
                Option<PowerDailyCostKind>,
                &mut Dating,
            )> = Vec::new();
            for power in power
                .iter_mut()
                .filter(|power| power.row.hour == at && named(&power.row))
            {
                dated.push((
                    Item {
                        cause: power.row.cause,
                        identity: &power.row.identity,
                    },
                    power.row.identity.target_country,
                    power.row.country,
                    None,
                    &mut power.dating,
                ));
            }
            for cost in costs
                .iter_mut()
                .filter(|cost| cost.row.day_hour == at && cost.row.cost_raw != 0)
            {
                dated.push((
                    Item {
                        cause: cost.row.cause,
                        identity: &cost.row.identity,
                    },
                    cost.row.identity.target_country,
                    cost.row.country,
                    Some(cost.row.component),
                    &mut cost.dating,
                ));
            }
            self.date(frame, roster, at == hour, &mut dated)?;
        }
        // A frameless payment's category reads its recorded key alone.
        let content = self.content.clone();
        for (row, dating) in power
            .iter_mut()
            .map(|power| {
                (
                    (
                        power.row.cause,
                        &power.row.identity,
                        None::<PowerDailyCostKind>,
                    ),
                    &mut power.dating,
                )
            })
            .chain(costs.iter_mut().map(|cost| {
                (
                    (cost.row.cause, &cost.row.identity, Some(cost.row.component)),
                    &mut cost.dating,
                )
            }))
        {
            if dating.dated.is_none() && dating.category.is_some() {
                let (cause, identity, component) = row;
                let facts = items::event_facts(None, identity, cause, component);
                dating.category = Some(category(
                    &Item { cause, identity },
                    &facts,
                    component,
                    content.as_deref(),
                ));
            }
        }
        for power in &mut power {
            power.resolved = Some(power.resolve());
        }
        for cost in &mut costs {
            cost.item = Some(cost.resolve());
        }
        let mut handed_governments = Vec::with_capacity(governments.len());
        for row in governments {
            let spell = |token: Option<u32>| -> io::Result<Option<String>> {
                let Some(token) = token else {
                    return Ok(None);
                };
                if row.hour == hour {
                    self.spelling(state, token)
                } else if let Some(frame) = held.as_ref().filter(|frame| frame.hour == row.hour) {
                    Ok(items::lexical_token(frame, token))
                } else {
                    Ok(None)
                }
            };
            let tokens = (spell(row.before_ideology)?, spell(row.after_ideology)?);
            handed_governments.push(Government { row, tokens });
        }
        // A capture interval starting at the hour may leave rows of the hour to the next read.
        let discontinuous = complete & bit(TableId::PoliticsCaptureCoverage) == 0
            || captures
                .changed
                .iter()
                .any(|row| row.since_hour == hour && row.through_hour == hour);
        if discontinuous {
            self.held = Some(walked.world.frame(state, kit())?);
        }
        out.push(Hour {
            branch: walked.branch,
            hour,
            whole,
            complete,
            census,
            links,
            captures,
            power,
            governments: handed_governments,
            daily,
            costs,
            deleted: [
                power_deleted,
                government_deleted,
                daily_deleted,
                cost_deleted,
            ],
            command,
            command_whole,
        })
    }

    fn carried(&self) -> Option<usize> {
        None
    }
}
