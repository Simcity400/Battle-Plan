//! Resource balance facts, projected hour by hour from a columnar walk ([`replay::segments`]):
//! the pool rows and the deal rows of every holder slot whose rows changed since this segment
//! last sent them (every slot's at a whole hour), the resource names where they changed, and
//! the tags the walk's countries resolve ([`dashboard_world`]) with each canonical slot's
//! partner tag where they changed. What each tag holds is the ordered emit's ([`super::emit`]).
//! An hour that changes nothing the balance reads carries nothing but its hour.

use dashboard_series::hash::FastMap;
use dashboard_world::{Needs, Roster, Tagged, World};
use replay::{
    columns,
    segments::{self, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{CountryResourceRow, LexicalTokenRow, ResourceDefinitionRow, ResourceExchangeRow, TableId},
};
use std::{collections::HashSet, io, mem::size_of, sync::Arc};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::LexicalTokens,
    TableId::ResourceDefinitions,
    TableId::CountryResources,
    TableId::ResourceExchanges,
];

/// What a projection reads of its walk's world: the countries.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

/// A CountryResources row: its key (raw country, resource), its resource and its pools, as
/// `produced`, `adjustment`, `imported`, `to_export`, `exported`, `used`, `special_total` and
/// `production_demand`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Pool {
    pub key: u128,
    pub resource: u32,
    pub values: [i64; 8],
}

/// A ResourceExchanges row: its handle, the holder slots of its exporter and importer (none
/// where no link resolves one), and its terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Deal {
    pub id: u64,
    pub exporter: Option<u32>,
    pub importer: Option<u32>,
    pub resource: u32,
    pub delivered: i64,
    pub factories: i32,
    pub convoys_assigned: i32,
    pub convoys_required: i32,
}

/// The tags the complete Countries and CountryLinks resolve, as the row census reads them.
#[derive(Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Countries {
    /// Each UTF-8 tag a country holds, by tag, with its first holder by id the links resolve.
    pub tags: Vec<(Arc<str>, Option<(u32, u32)>)>,
    /// Each canonical slot's first country by id: its tag, none where it is not UTF-8.
    pub partners: FastMap<u32, Option<Arc<str>>>,
}

impl Countries {
    fn heap(&self) -> usize {
        self.tags.capacity() * size_of::<(Arc<str>, Option<(u32, u32)>)>()
            + self.tags.iter().map(|(tag, _)| tag.len()).sum::<usize>()
            + self.partners.capacity() * size_of::<(u32, Option<Arc<str>>)>()
    }
}

/// One table's rows by holder slot: every slot holding rows (the others hold none) when
/// `whole`, else the slots whose rows changed since this segment last sent them, an empty list
/// where a slot holds none now. Nothing while the table is not `complete`.
#[derive(replay_spool::Spooled)]
pub struct Listed<T> {
    pub complete: bool,
    pub whole: bool,
    pub slots: Vec<(u32, Vec<T>)>,
}

impl<T> Listed<T> {
    fn heap(&self) -> usize {
        self.slots.capacity() * size_of::<(u32, Vec<T>)>()
            + self
                .slots
                .iter()
                .map(|(_, rows)| rows.capacity() * size_of::<T>())
                .sum::<usize>()
    }
}

/// The resources the recorded definitions and tokens name: each native index with its token,
/// by index.
pub type Names = Arc<[(u32, Arc<str>)]>;

/// What the ordered emit reads of an hour that changes what the balance reads.
#[derive(replay_spool::Spooled)]
pub struct Facts {
    /// The hour opens a segment: what it restates whole goes on where it equals what was held.
    pub restart: bool,
    /// The countries, at a whole hour or where they changed.
    pub countries: Option<Arc<Countries>>,
    /// Whether this hour's item is the first to carry `countries`, which it is charged for.
    pub charged: bool,
    /// The names where they changed: none while either table is not complete.
    pub names: Option<Option<Names>>,
    pub pools: Option<Listed<Pool>>,
    pub deals: Option<Listed<Deal>>,
}

/// One hour of a segment.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// None when the hour changes nothing the balance reads.
    pub facts: Option<Box<Facts>>,
}

/// A table's rows by walker slot and by holder slot, kept from the hour's changes.
struct Rows<T> {
    rows: Vec<Option<(T, [Option<u32>; 2])>>,
    holders: FastMap<u32, Vec<u32>>,
}

impl<T: Copy> Rows<T> {
    fn new() -> Self {
        Self {
            rows: Vec::new(),
            holders: FastMap::default(),
        }
    }

    fn clear(&mut self) {
        self.rows.clear();
        self.holders.clear();
    }

    /// Forgets the row at `slot`, marking its holders dirty.
    fn remove(&mut self, slot: u32, dirty: &mut HashSet<u32>) {
        let Some((_, holders)) = self.rows.get_mut(slot as usize).and_then(Option::take) else {
            return;
        };
        for holder in holders.into_iter().flatten() {
            if let Some(own) = self.holders.get_mut(&holder) {
                if let Some(at) = own.iter().position(|&held| held == slot) {
                    own.swap_remove(at);
                }
                if own.is_empty() {
                    self.holders.remove(&holder);
                }
            }
            dirty.insert(holder);
        }
    }

    /// Holds `row` at `slot` for `holders`, marking them dirty.
    fn insert(&mut self, slot: u32, row: T, holders: [Option<u32>; 2], dirty: &mut HashSet<u32>) {
        self.remove(slot, dirty);
        let index = slot as usize;
        if self.rows.len() <= index {
            self.rows.resize(index + 1, None);
        }
        for holder in holders.into_iter().flatten() {
            self.holders.entry(holder).or_default().push(slot);
            dirty.insert(holder);
        }
        self.rows[index] = Some((row, holders));
    }

    /// `holder`'s rows, by `order`.
    fn of<K: Ord>(&self, holder: u32, order: impl Fn(&T) -> K) -> Vec<T> {
        let mut rows: Vec<T> = self
            .holders
            .get(&holder)
            .into_iter()
            .flatten()
            .filter_map(|&slot| self.rows[slot as usize].map(|(row, _)| row))
            .collect();
        rows.sort_unstable_by_key(|row| order(row));
        rows
    }

    fn heap(&self) -> usize {
        self.rows.capacity() * size_of::<Option<(T, [Option<u32>; 2])>>()
            + self.holders.capacity() * size_of::<(u32, Vec<u32>)>()
            + self
                .holders
                .values()
                .map(|rows| rows.capacity() * 4)
                .sum::<usize>()
    }
}

/// The columns a projection reads.
struct Fields {
    pools: [usize; 8],
    exporter: usize,
    importer: usize,
    resource: usize,
    delivered: usize,
    factories: usize,
    assigned: usize,
    required: usize,
    token: usize,
    literal: usize,
    /// The CountryResources and ResourceExchanges columns those are, as change-mask bits.
    pool_mask: u64,
    deal_mask: u64,
}

impl Fields {
    fn new() -> io::Result<Self> {
        let pool = |name| columns::field(TableId::CountryResources, name);
        let deal = |name| columns::field(TableId::ResourceExchanges, name);
        let pools = [
            pool("produced")?,
            pool("adjustment")?,
            pool("imported")?,
            pool("to_export")?,
            pool("exported")?,
            pool("used")?,
            pool("special_total")?,
            pool("production_demand")?,
        ];
        let fields = Self {
            pools,
            exporter: deal("exporter")?,
            importer: deal("importer")?,
            resource: deal("resource")?,
            delivered: deal("delivered")?,
            factories: deal("assigned_factories")?,
            assigned: deal("convoys_assigned")?,
            required: deal("convoys_required")?,
            token: columns::field(TableId::ResourceDefinitions, "token")?,
            literal: columns::field(TableId::LexicalTokens, "literal")?,
            pool_mask: 0,
            deal_mask: 0,
        };
        let pool_mask = fields.pools.iter().fold(0, |mask, &field| mask | 1 << field);
        let deal_mask = [
            fields.exporter,
            fields.importer,
            fields.resource,
            fields.delivered,
            fields.factories,
            fields.assigned,
            fields.required,
        ]
        .iter()
        .fold(0, |mask, &field| mask | 1 << field);
        Ok(Self {
            pool_mask,
            deal_mask,
            ..fields
        })
    }
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: FormatVersion,
    fields: Fields,
    /// Nothing is held: the next hour is read whole, as a segment's first.
    cold: bool,
    last: Option<i32>,
    status: [Option<columns::Status>; 2],
    roster: bool,
    countries: Arc<Countries>,
    uncharged: bool,
    names: Option<Names>,
    pools: Rows<Pool>,
    deals: Rows<Deal>,
    /// Holder slots whose rows changed since this segment last sent them.
    dirty: [HashSet<u32>; 2],
    /// Holder slots whose pool rows name one resource more than once: observed again at every
    /// hour that changes the pool table or the names, as the row census observes them.
    repeated: HashSet<u32>,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds is
/// read again from the walker's tables, as at a segment's first hour.
const SAVED: &[u8] = b"resources projector: cold";

/// Whether the hour inserted, deleted or changed a row of `table`, or read it in part.
fn touched(state: &State, table: TableId) -> io::Result<bool> {
    let changes = columns::changes(state, table)?;
    Ok(changes.truncated
        || !changes.changed.is_empty()
        || !changes.fill_slots.is_empty()
        || !changes.inserted.is_empty()
        || !changes.deleted.is_empty())
}

/// The countries the walk's roster resolves.
fn countries(roster: &Roster) -> Countries {
    let utf8 = |tag| std::str::from_utf8(roster.tag(tag)).ok().map(Arc::<str>::from);
    let mut tags = Vec::new();
    for tag in roster.held() {
        let Some(text) = utf8(tag) else {
            continue;
        };
        let holder = match roster.tagged(tag) {
            Tagged::Found(country) => Some((country.slot, country.id)),
            Tagged::Absent | Tagged::Unresolved => None,
        };
        tags.push((text, holder));
    }
    tags.sort_unstable_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    let mut partners = FastMap::default();
    for country in roster.iter() {
        partners
            .entry(country.slot)
            .or_insert_with(|| utf8(country.tag));
    }
    Countries { tags, partners }
}

impl Projector {
    pub fn new(format: FormatVersion) -> io::Result<Self> {
        Ok(Self {
            format,
            fields: Fields::new()?,
            cold: true,
            last: None,
            status: [None; 2],
            roster: false,
            countries: Arc::new(Countries::default()),
            uncharged: true,
            names: None,
            pools: Rows::new(),
            deals: Rows::new(),
            dirty: Default::default(),
            repeated: HashSet::new(),
        })
    }

    /// The pool row at `slot`.
    fn pool(&self, state: &State, slot: u32) -> io::Result<Pool> {
        let table = TableId::CountryResources;
        let held = columns::table(state, table)?;
        let key = held
            .key(slot)
            .ok_or_else(|| io::Error::other("A CountryResources slot holds no row"))?;
        let mut values = [0; 8];
        if self.fields.pools.iter().all(|&field| held.recorded(field, slot)) {
            for (value, &field) in values.iter_mut().zip(&self.fields.pools) {
                *value = *columns::plain::<i64>(held, field)?
                    .get(slot as usize)
                    .ok_or_else(|| io::Error::other("A pool's slot holds no cell"))?;
            }
        } else {
            let row: CountryResourceRow = columns::row(state, table, slot, self.format.version())?;
            values = [
                row.produced,
                row.adjustment,
                row.imported,
                row.to_export,
                row.exported,
                row.used,
                row.special_total,
                row.production_demand,
            ];
        }
        Ok(Pool {
            key,
            resource: key as u32,
            values,
        })
    }

    /// The deal row at `slot`, with its raw exporter and importer.
    fn deal(&self, state: &State, slot: u32) -> io::Result<(Deal, [u32; 2])> {
        let table = TableId::ResourceExchanges;
        let held = columns::table(state, table)?;
        let id = held
            .key(slot)
            .ok_or_else(|| io::Error::other("A ResourceExchanges slot holds no row"))?
            as u64;
        let fields = &self.fields;
        let all = [
            fields.exporter,
            fields.importer,
            fields.resource,
            fields.delivered,
            fields.factories,
            fields.assigned,
            fields.required,
        ];
        let row = if all.iter().all(|&field| held.recorded(field, slot)) {
            let missing = || io::Error::other("A deal's slot holds no cell");
            let at = slot as usize;
            let u32s = |field| -> io::Result<u32> {
                columns::plain::<u32>(held, field)?
                    .get(at)
                    .copied()
                    .ok_or_else(missing)
            };
            let i32s = |field| -> io::Result<i32> {
                columns::plain::<i32>(held, field)?
                    .get(at)
                    .copied()
                    .ok_or_else(missing)
            };
            ResourceExchangeRow {
                id,
                exporter: u32s(fields.exporter)?,
                importer: u32s(fields.importer)?,
                resource: u32s(fields.resource)?,
                delivered: columns::plain::<i64>(held, fields.delivered)?
                    .get(at)
                    .copied()
                    .ok_or_else(missing)?,
                assigned_factories: i32s(fields.factories)?,
                convoys_assigned: i32s(fields.assigned)?,
                convoys_required: i32s(fields.required)?,
                ..Default::default()
            }
        } else {
            columns::row(state, table, slot, self.format.version())?
        };
        Ok((
            Deal {
                id,
                exporter: None,
                importer: None,
                resource: row.resource,
                delivered: row.delivered,
                factories: row.assigned_factories,
                convoys_assigned: row.convoys_assigned,
                convoys_required: row.convoys_required,
            },
            [row.exporter, row.importer],
        ))
    }

    /// Holds the pool row at `slot`.
    fn put_pool(&mut self, state: &State, roster: &Roster, slot: u32) -> io::Result<()> {
        let pool = self.pool(state, slot)?;
        let holder = roster.slot_of((pool.key >> 32) as u32);
        self.pools.insert(slot, pool, [holder, None], &mut self.dirty[0]);
        Ok(())
    }

    /// Holds the deal row at `slot`, each holder slot it names once.
    fn put_deal(&mut self, state: &State, roster: &Roster, slot: u32) -> io::Result<()> {
        let (mut deal, [exporter, importer]) = self.deal(state, slot)?;
        deal.exporter = roster.slot_of(exporter);
        deal.importer = roster.slot_of(importer);
        let holders = [
            deal.exporter,
            deal.importer.filter(|&importer| Some(importer) != deal.exporter),
        ];
        self.deals.insert(slot, deal, holders, &mut self.dirty[1]);
        Ok(())
    }

    /// Reads `table` whole, or by the hour's changes to the fields read.
    fn index(&mut self, state: &State, roster: &Roster, at: usize, whole: bool) -> io::Result<()> {
        let table = [TableId::CountryResources, TableId::ResourceExchanges][at];
        let mask = [self.fields.pool_mask, self.fields.deal_mask][at];
        let put = |projector: &mut Self, slot| match at {
            0 => projector.put_pool(state, roster, slot),
            _ => projector.put_deal(state, roster, slot),
        };
        if whole {
            match at {
                0 => self.pools.clear(),
                _ => self.deals.clear(),
            }
            let live: Vec<u32> = columns::table(state, table)?.live().ones().collect();
            for slot in live {
                put(self, slot)?;
            }
            return Ok(());
        }
        let edits = columns::changes(state, table)?;
        let deleted = edits.deleted.clone();
        let mut written: Vec<u32> = edits.inserted.clone();
        written.extend(
            edits
                .changed
                .iter()
                .zip(&edits.masks)
                .chain(edits.fill_slots.iter().zip(&edits.fill_masks))
                .filter(|&(_, &written)| written & mask != 0)
                .map(|(&slot, _)| slot),
        );
        written.sort_unstable();
        written.dedup();
        for slot in deleted {
            match at {
                0 => self.pools.remove(slot, &mut self.dirty[0]),
                _ => self.deals.remove(slot, &mut self.dirty[1]),
            }
        }
        for slot in written {
            put(self, slot)?;
        }
        Ok(())
    }

    /// The resources the definitions and tokens name, as the row census learns them: each
    /// definition whose token the tokens spell, non-empty.
    fn names(&self, state: &State) -> io::Result<Names> {
        let definitions = columns::table(state, TableId::ResourceDefinitions)?;
        let lexical = columns::table(state, TableId::LexicalTokens)?;
        let mut names = Vec::new();
        let live: Vec<u32> = definitions.live().ones().collect();
        for slot in live {
            let id = definitions
                .key(slot)
                .ok_or_else(|| io::Error::other("A ResourceDefinitions slot holds no row"))?
                as u32;
            let token = if definitions.recorded(self.fields.token, slot) {
                *columns::plain::<u32>(definitions, self.fields.token)?
                    .get(slot as usize)
                    .ok_or_else(|| io::Error::other("A definition's slot holds no cell"))?
            } else {
                columns::row::<ResourceDefinitionRow>(
                    state,
                    TableId::ResourceDefinitions,
                    slot,
                    self.format.version(),
                )?
                .token
            };
            let Some(at) = lexical.slot(u128::from(token)) else {
                continue;
            };
            let literal = if lexical.recorded(self.fields.literal, at) {
                let text = *columns::text(lexical, self.fields.literal)?
                    .get(at as usize)
                    .ok_or_else(|| io::Error::other("A token's slot holds no cell"))?;
                state
                    .dictionary()
                    .get(text)
                    .ok_or_else(|| io::Error::other("A token names no dictionary entry"))?
                    .to_vec()
            } else {
                columns::row::<LexicalTokenRow>(state, TableId::LexicalTokens, at, self.format.version())?
                    .literal
            };
            if let Ok(token) = std::str::from_utf8(&literal)
                && !token.is_empty()
            {
                names.push((id, Arc::from(token)));
            }
        }
        names.sort_unstable_by_key(|&(id, _)| id);
        Ok(names.into())
    }

    /// The dirty holders' pools, every holder's when `whole`.
    fn listed_pools(&mut self, complete: bool, whole: bool) -> Option<Listed<Pool>> {
        if !complete {
            self.dirty[0].clear();
            self.repeated.clear();
            return whole.then(|| Listed {
                complete,
                whole,
                slots: Vec::new(),
            });
        }
        let mut slots: Vec<u32> = if whole {
            self.dirty[0].clear();
            self.repeated.clear();
            self.pools.holders.keys().copied().collect()
        } else {
            self.dirty[0].drain().collect()
        };
        slots.sort_unstable();
        let mut listed = Vec::with_capacity(slots.len());
        for slot in slots {
            let rows = self.pools.of(slot, |pool| pool.key);
            let repeats = rows
                .iter()
                .enumerate()
                .any(|(at, pool)| rows[..at].iter().any(|held| held.resource == pool.resource));
            if repeats {
                self.repeated.insert(slot);
            } else {
                self.repeated.remove(&slot);
            }
            listed.push((slot, rows));
        }
        (whole || !listed.is_empty()).then_some(Listed {
            complete,
            whole,
            slots: listed,
        })
    }

    /// The dirty holders' deals, every holder's when `whole`.
    fn listed_deals(&mut self, complete: bool, whole: bool) -> Option<Listed<Deal>> {
        if !complete {
            self.dirty[1].clear();
            return whole.then(|| Listed {
                complete,
                whole,
                slots: Vec::new(),
            });
        }
        let mut slots: Vec<u32> = if whole {
            self.dirty[1].clear();
            self.deals.holders.keys().copied().collect()
        } else {
            self.dirty[1].drain().collect()
        };
        slots.sort_unstable();
        let listed: Vec<_> = slots
            .into_iter()
            .map(|slot| (slot, self.deals.of(slot, |deal| deal.id)))
            .collect();
        (whole || !listed.is_empty()).then_some(Listed {
            complete,
            whole,
            slots: listed,
        })
    }

    fn heap(&self) -> usize {
        size_of::<Countries>()
            + self.countries.heap()
            + self.names.as_ref().map_or(0, |names| {
                names
                    .iter()
                    .map(|(_, token)| size_of::<(u32, Arc<str>)>() + token.len())
                    .sum()
            })
            + self.pools.heap()
            + self.deals.heap()
            + self.dirty.iter().map(|dirty| dirty.capacity() * 4).sum::<usize>()
            + self.repeated.capacity() * 4
    }
}

// Every segment starts cold: its first hour reads every holder and country again; what came
// before is the emit's.
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
        let _cpu = replay_frame::cpu::scope("resources.columnar", Some("resources"));
        let state = hour.state;
        let at = hour.hour;
        let roster = hour.world.countries()?;
        let opens = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        self.cold = false;
        self.last = Some(at);
        // The countries, where they changed.
        let complete_roster = roster.complete();
        let became = opens || complete_roster != self.roster;
        self.roster = complete_roster;
        let mut countries = None;
        if became || roster.changed() {
            let resolved = self::countries(roster);
            if resolved != *self.countries {
                self.countries = Arc::new(resolved);
                self.uncharged = true;
            }
            countries = Some(Arc::clone(&self.countries));
        }
        let regrouped = opens || roster.rebuilt() || !roster.changed_links().is_empty();
        // The names, where they changed.
        let lexical = [TableId::ResourceDefinitions, TableId::LexicalTokens];
        let mut renamed = false;
        let mut names = None;
        if opens
            || lexical
                .iter()
                .any(|&table| hour.world.restated(table) || touched(state, table).unwrap_or(true))
        {
            renamed = true;
            let read = if lexical.iter().all(|&table| columns::complete(state, table)) {
                Some(self.names(state)?)
            } else {
                None
            };
            if opens || read != self.names {
                self.names = read.clone();
                names = Some(read);
            }
        }
        // Each table's rows by holder.
        let mut listed = [false; 2];
        let mut whole = [false; 2];
        let mut complete = [false; 2];
        for (index, table) in [TableId::CountryResources, TableId::ResourceExchanges]
            .into_iter()
            .enumerate()
        {
            let status = columns::status(state, table);
            complete[index] = status == columns::Status::Complete;
            let shaped = opens || self.status[index] != Some(status);
            self.status[index] = Some(status);
            if !complete[index] {
                match index {
                    0 => self.pools.clear(),
                    _ => self.deals.clear(),
                }
                whole[index] = shaped;
                listed[index] = shaped;
            } else if shaped || regrouped {
                self.index(state, roster, index, true)?;
                whole[index] = true;
                listed[index] = true;
            } else if touched(state, table)? {
                self.index(state, roster, index, false)?;
                listed[index] = true;
                if index == 0 {
                    self.dirty[0].extend(self.repeated.iter().copied());
                }
            }
        }
        if renamed && complete[0] && !whole[0] && !self.repeated.is_empty() {
            self.dirty[0].extend(self.repeated.iter().copied());
            listed[0] = true;
        }
        let pools = if listed[0] {
            self.listed_pools(complete[0], whole[0])
        } else {
            None
        };
        let deals = if listed[1] {
            self.listed_deals(complete[1], whole[1])
        } else {
            None
        };
        let quiet = countries.is_none() && names.is_none() && pools.is_none() && deals.is_none();
        let facts = (!quiet).then(|| {
            let charged = countries.is_some() && std::mem::take(&mut self.uncharged);
            Box::new(Facts {
                restart: opens,
                countries,
                charged,
                names,
                pools,
                deals,
            })
        });
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            facts,
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
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a saved resources projector",
            ));
        }
        *self = Self::new(self.format)?;
        Ok(())
    }
}
