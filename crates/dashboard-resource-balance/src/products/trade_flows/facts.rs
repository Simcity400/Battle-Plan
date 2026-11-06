//! Trade flow facts, projected hour by hour from a columnar walk ([`replay::segments`]): at each
//! read the tables can say who trades what with, every flow whose summed terms changed since the
//! hour before (every listed flow at a whole hour), the deals left out by seller and buyer where
//! they changed, the holder of every slot where they changed, and the resources the definitions
//! name. What a flow held before the segment is the ordered emit's ([`super::emit`]). An hour
//! that changes nothing the flows read carries nothing but its hour.
//!
//! The exchanges are kept by table slot and grouped by flow from the hour's changed rows; a
//! flow's terms are summed again, in row key order, only where one of its rows changed. A cold
//! start, a gap, a completeness change, a changed country, link or resource name reads every
//! row again.

use super::{FlowKey, Terms};
use dashboard_base::products::country_history::Identity;
use dashboard_world::{Needs, Roster, Tagged, World};
use replay::{
    columns,
    segments::{self, Operator, Sink, State},
};
use replay_data::rows::{LexicalTokenRow, ResourceDefinitionRow, ResourceExchangeRow, TableId};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    mem::size_of,
    sync::Arc,
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::LexicalTokens,
    TableId::ResourceDefinitions,
    TableId::ResourceExchanges,
];

/// What a projection reads of its walk's world: the countries.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

/// Deals a read lists but leaves out: by seller and buyer where the links resolve both but the
/// definitions do not name the resource, none where the links do not resolve a side.
pub(crate) type Group = Option<(Identity, Identity)>;

/// A holder slot's holder: its first tag in tag order and its country id.
pub(crate) type Holder = (Arc<str>, u32);

/// What the ordered emit reads of an hour that changes what the flows read.
#[derive(replay_spool::Spooled)]
pub struct Facts {
    /// Whether the read can say who trades what: the countries, their links, the deals, the
    /// definitions and the tokens are all complete. An unknown read carries nothing else.
    pub(crate) known: bool,
    /// Whether the read is whole: `holders`, `flows` and `rejected` then list everything.
    pub(crate) whole: bool,
    /// Each slot's holder where it changed; every held slot at a whole read.
    pub(crate) holders: Vec<(u32, Option<Holder>)>,
    /// Each flow whose summed terms changed, none where no deal counts toward it any more.
    pub(crate) flows: Vec<(FlowKey, Option<Terms>)>,
    /// The deals left out by group, where they changed.
    pub(crate) rejected: Option<Vec<(Group, u32)>>,
    /// Each resource the definitions name with its token, not sent in this segment yet.
    pub(crate) names: Vec<(u32, String)>,
}

/// One hour of a segment.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// None when the hour changes nothing the flows read.
    pub(crate) facts: Option<Box<Facts>>,
}

/// Where a deal counts: toward a flow (by index), or left out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Flow(u32),
    Left(Group),
}

/// A deal: its row key, its seller and buyer links, its resource, its terms and where it counts.
#[derive(Clone, Copy)]
struct Row {
    key: u128,
    exporter: u32,
    importer: u32,
    resource: u32,
    terms: Terms,
    target: Target,
}

/// A flow's deals by row key, ascending, with their table slots, and the terms last sent.
struct Members {
    key: FlowKey,
    rows: Vec<(u128, u32)>,
    sent: Option<Terms>,
    dirty: bool,
}

/// The columns the deals, definitions and tokens are read from.
struct Fields {
    exporter: usize,
    importer: usize,
    resource: usize,
    delivered: usize,
    factories: usize,
    assigned: usize,
    required: usize,
    /// Every operand column as change-mask bits.
    operands: u64,
    token: usize,
    literal: usize,
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: u16,
    fields: Fields,
    /// Nothing is held: the next hour is read whole, as a segment's first.
    cold: bool,
    last: Option<i32>,
    /// Whether the last hour was a known read.
    known: bool,
    /// The next known read reads every row again.
    stale: bool,
    holders: HashMap<u32, Holder>,
    names: BTreeMap<u32, Arc<str>>,
    rows: Vec<Option<Row>>,
    flows: Vec<Members>,
    index: HashMap<FlowKey, u32>,
    dirty: Vec<u32>,
    rejected: BTreeMap<Group, u32>,
    /// The deals left out as last sent, and whether they may have changed since.
    rejected_sent: BTreeMap<Group, u32>,
    recount: bool,
    /// The resources sent in this segment.
    spelled: HashSet<u32>,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds is
/// read again from the walker's tables, as at a segment's first hour.
const SAVED: &[u8] = b"trade projector: cold";

fn overflow() -> io::Error {
    dashboard_core::invalid("recorded trade terms overflow their sum")
}

/// Each holder slot's first tag in tag order that the countries resolve, with its country id,
/// as every per-country product reads the holders.
fn holders(roster: &Roster) -> HashMap<u32, Holder> {
    let mut tagged: Vec<(&str, u32, u32)> = roster
        .held()
        .filter_map(|tag| match roster.tagged(tag) {
            Tagged::Found(country) => std::str::from_utf8(roster.tag(tag))
                .ok()
                .map(|text| (text, country.slot, country.id)),
            Tagged::Absent | Tagged::Unresolved => None,
        })
        .collect();
    tagged.sort_unstable_by(|left, right| left.0.cmp(right.0));
    let mut holders = HashMap::with_capacity(tagged.len());
    for (tag, slot, id) in tagged {
        holders.entry(slot).or_insert_with(|| (Arc::from(tag), id));
    }
    holders
}

/// Whether the hour inserted, deleted or changed a row of `table`, or read it in part.
fn touched(state: &State, table: TableId) -> io::Result<bool> {
    let changes = columns::changes(state, table)?;
    Ok(changes.truncated
        || !changes.changed.is_empty()
        || !changes.fill_slots.is_empty()
        || !changes.inserted.is_empty()
        || !changes.deleted.is_empty())
}

/// `slot`'s cell of `table`'s `field`, a column held as `T`.
fn cell<T: columns::Plain + Copy>(
    state: &State,
    table: TableId,
    field: usize,
    slot: u32,
) -> io::Result<T> {
    columns::value::<T>(columns::table(state, table)?, field, slot)?
        .ok_or_else(|| io::Error::other("A trade column holds no cell at a live slot"))
}

impl Projector {
    pub fn new(format: u16) -> io::Result<Self> {
        let deal = |name| columns::field(TableId::ResourceExchanges, name);
        let fields = Fields {
            exporter: deal("exporter")?,
            importer: deal("importer")?,
            resource: deal("resource")?,
            delivered: deal("delivered")?,
            factories: deal("assigned_factories")?,
            assigned: deal("convoys_assigned")?,
            required: deal("convoys_required")?,
            operands: 0,
            token: columns::field(TableId::ResourceDefinitions, "token")?,
            literal: columns::field(TableId::LexicalTokens, "literal")?,
        };
        let operands = [
            fields.exporter,
            fields.importer,
            fields.resource,
            fields.delivered,
            fields.factories,
            fields.assigned,
            fields.required,
        ]
        .iter()
        .fold(0_u64, |mask, &field| mask | 1_u64 << field);
        Ok(Self {
            format,
            fields: Fields { operands, ..fields },
            cold: true,
            last: None,
            known: false,
            stale: true,
            holders: HashMap::new(),
            names: BTreeMap::new(),
            rows: Vec::new(),
            flows: Vec::new(),
            index: HashMap::new(),
            dirty: Vec::new(),
            rejected: BTreeMap::new(),
            rejected_sent: BTreeMap::new(),
            recount: true,
            spelled: HashSet::new(),
        })
    }

    /// The deal at `slot`: its key, links, resource and terms, from its columns, or from its row
    /// where a cell it reads was not recorded.
    fn deal(&self, state: &State, slot: u32) -> io::Result<Row> {
        let table = TableId::ResourceExchanges;
        let held = columns::table(state, table)?;
        let key = held
            .key(slot)
            .ok_or_else(|| io::Error::other("A ResourceExchanges slot holds no row"))?;
        let f = &self.fields;
        let read = [
            f.exporter,
            f.importer,
            f.resource,
            f.delivered,
            f.factories,
            f.assigned,
            f.required,
        ];
        let (exporter, importer, resource, terms) =
            if read.iter().all(|&field| held.recorded(field, slot)) {
                (
                    cell::<u32>(state, table, f.exporter, slot)?,
                    cell::<u32>(state, table, f.importer, slot)?,
                    cell::<u32>(state, table, f.resource, slot)?,
                    Terms {
                        delivered: cell::<i64>(state, table, f.delivered, slot)?,
                        factories: cell::<i32>(state, table, f.factories, slot)?,
                        assigned: cell::<i32>(state, table, f.assigned, slot)?,
                        required: cell::<i32>(state, table, f.required, slot)?,
                        deals: 1,
                    },
                )
            } else {
                let row: ResourceExchangeRow = columns::row(state, table, slot, self.format)?;
                (
                    row.exporter,
                    row.importer,
                    row.resource,
                    Terms {
                        delivered: row.delivered,
                        factories: row.assigned_factories,
                        assigned: row.convoys_assigned,
                        required: row.convoys_required,
                        deals: 1,
                    },
                )
            };
        Ok(Row {
            key,
            exporter,
            importer,
            resource,
            terms,
            target: Target::Left(None),
        })
    }

    /// The resources the definitions name: each definition whose token the tokens spell, by its
    /// native index.
    fn named(&self, state: &State) -> io::Result<BTreeMap<u32, Arc<str>>> {
        let definitions = columns::table(state, TableId::ResourceDefinitions)?;
        let tokens = columns::table(state, TableId::LexicalTokens)?;
        let mut names = BTreeMap::new();
        for slot in definitions.live().ones() {
            let id = definitions
                .key(slot)
                .ok_or_else(|| io::Error::other("A ResourceDefinitions slot holds no row"))?
                as u32;
            let token = if definitions.recorded(self.fields.token, slot) {
                cell::<u32>(state, TableId::ResourceDefinitions, self.fields.token, slot)?
            } else {
                columns::row::<ResourceDefinitionRow>(
                    state,
                    TableId::ResourceDefinitions,
                    slot,
                    self.format,
                )?
                .token
            };
            let Some(spelled) = tokens.slot(u128::from(token)) else {
                continue;
            };
            let literal = if tokens.recorded(self.fields.literal, spelled) {
                let text = *columns::text(tokens, self.fields.literal)?
                    .get(spelled as usize)
                    .ok_or_else(|| io::Error::other("A token's slot holds no cell"))?;
                state
                    .dictionary()
                    .get(text)
                    .ok_or_else(|| io::Error::other("A token names no dictionary entry"))?
                    .to_vec()
            } else {
                columns::row::<LexicalTokenRow>(state, TableId::LexicalTokens, spelled, self.format)?
                    .literal
            };
            if let Ok(text) = std::str::from_utf8(&literal)
                && !text.is_empty()
            {
                names.insert(id, Arc::from(text));
            }
        }
        Ok(names)
    }

    /// Where `row` counts under the holders and names held now.
    fn target(&mut self, roster: &Roster, row: &Row) -> Target {
        let side = |raw: u32| {
            roster
                .slot_of(raw)
                .and_then(|slot| self.holders.get(&slot).map(|&(_, id)| (slot, id)))
        };
        let (Some(seller), Some(buyer)) = (side(row.exporter), side(row.importer)) else {
            return Target::Left(None);
        };
        if !self.names.contains_key(&row.resource) {
            return Target::Left(Some((seller, buyer)));
        }
        let key = (seller, buyer, row.resource);
        let flow = match self.index.get(&key) {
            Some(&flow) => flow,
            None => {
                let flow = self.flows.len() as u32;
                self.flows.push(Members {
                    key,
                    rows: Vec::new(),
                    sent: None,
                    dirty: false,
                });
                self.index.insert(key, flow);
                flow
            }
        };
        Target::Flow(flow)
    }

    fn mark(&mut self, flow: u32) {
        let members = &mut self.flows[flow as usize];
        if !members.dirty {
            members.dirty = true;
            self.dirty.push(flow);
        }
    }

    /// Counts `row`, at `slot`, where it counts.
    fn attach(&mut self, roster: &Roster, slot: u32, mut row: Row) {
        row.target = self.target(roster, &row);
        match row.target {
            Target::Flow(flow) => {
                let rows = &mut self.flows[flow as usize].rows;
                if let Err(at) = rows.binary_search(&(row.key, slot)) {
                    rows.insert(at, (row.key, slot));
                }
                self.mark(flow);
            }
            Target::Left(group) => {
                *self.rejected.entry(group).or_default() += 1;
                self.recount = true;
            }
        }
        let index = slot as usize;
        if self.rows.len() <= index {
            self.rows.resize(index + 1, None);
        }
        self.rows[index] = Some(row);
    }

    /// Forgets the deal at `slot`.
    fn detach(&mut self, slot: u32) {
        let Some(row) = self.rows.get_mut(slot as usize).and_then(Option::take) else {
            return;
        };
        match row.target {
            Target::Flow(flow) => {
                let rows = &mut self.flows[flow as usize].rows;
                if let Ok(at) = rows.binary_search(&(row.key, slot)) {
                    rows.remove(at);
                }
                self.mark(flow);
            }
            Target::Left(group) => {
                if let Some(count) = self.rejected.get_mut(&group) {
                    *count -= 1;
                    if *count == 0 {
                        self.rejected.remove(&group);
                    }
                }
                self.recount = true;
            }
        }
    }

    /// Every deal read again.
    fn index_whole(&mut self, state: &State, roster: &Roster) -> io::Result<()> {
        self.rows.clear();
        self.flows.clear();
        self.index.clear();
        self.dirty.clear();
        self.rejected.clear();
        self.recount = true;
        let live: Vec<u32> = columns::table(state, TableId::ResourceExchanges)?
            .live()
            .ones()
            .collect();
        for slot in live {
            let row = self.deal(state, slot)?;
            self.attach(roster, slot, row);
        }
        Ok(())
    }

    /// The deals the hour's changes reach, read again.
    fn index_changes(&mut self, state: &State, roster: &Roster) -> io::Result<()> {
        let edits = columns::changes(state, TableId::ResourceExchanges)?;
        for &slot in &edits.deleted {
            self.detach(slot);
        }
        for &slot in &edits.inserted {
            self.detach(slot);
            let row = self.deal(state, slot)?;
            self.attach(roster, slot, row);
        }
        let written = edits
            .changed
            .iter()
            .zip(&edits.masks)
            .chain(edits.fill_slots.iter().zip(&edits.fill_masks));
        for (&slot, &mask) in written {
            if mask & self.fields.operands == 0 {
                continue;
            }
            let Some(held) = self.rows.get(slot as usize).copied().flatten() else {
                continue;
            };
            let row = self.deal(state, slot)?;
            if (row.exporter, row.importer, row.resource)
                != (held.exporter, held.importer, held.resource)
            {
                self.detach(slot);
                self.attach(roster, slot, row);
            } else if row.terms != held.terms {
                self.rows[slot as usize] = Some(Row {
                    target: held.target,
                    ..row
                });
                if let Target::Flow(flow) = held.target {
                    self.mark(flow);
                }
            }
        }
        Ok(())
    }

    /// `flow`'s terms summed over its deals in row key order; none without a deal.
    fn sum(&self, flow: u32) -> io::Result<Option<Terms>> {
        let mut terms: Option<Terms> = None;
        for &(_, slot) in &self.flows[flow as usize].rows {
            let row = self.rows[slot as usize]
                .ok_or_else(|| io::Error::other("A flow's deal is not indexed"))?;
            terms = Some(
                terms
                    .unwrap_or_default()
                    .add(row.terms)
                    .ok_or_else(overflow)?,
            );
        }
        Ok(terms)
    }

    /// The flows whose terms changed since last sent: every listed flow, by key, when `whole`.
    fn flows(&mut self, whole: bool) -> io::Result<Vec<(FlowKey, Option<Terms>)>> {
        let mut flows = Vec::new();
        if whole {
            for flow in 0..self.flows.len() as u32 {
                let terms = self.sum(flow)?;
                let members = &mut self.flows[flow as usize];
                members.dirty = false;
                members.sent = terms;
                if let Some(terms) = terms {
                    flows.push((members.key, Some(terms)));
                }
            }
            self.dirty.clear();
        } else {
            for flow in std::mem::take(&mut self.dirty) {
                let terms = self.sum(flow)?;
                let members = &mut self.flows[flow as usize];
                members.dirty = false;
                if members.sent != terms {
                    members.sent = terms;
                    flows.push((members.key, terms));
                }
            }
        }
        flows.sort_unstable_by_key(|&(key, _)| key);
        Ok(flows)
    }

    /// The deals left out, where they changed since last sent: always when `whole`.
    fn rejected(&mut self, whole: bool) -> Option<Vec<(Group, u32)>> {
        if !std::mem::take(&mut self.recount) && !whole {
            return None;
        }
        if !whole && self.rejected == self.rejected_sent {
            return None;
        }
        self.rejected_sent = self.rejected.clone();
        Some(
            self.rejected
                .iter()
                .map(|(&group, &count)| (group, count))
                .collect(),
        )
    }

    fn heap(&self) -> usize {
        self.holders.capacity() * size_of::<(u32, Holder)>()
            + self.holders.values().map(|(tag, _)| tag.len()).sum::<usize>()
            + self.names.len() * size_of::<(u32, Arc<str>)>()
            + self.names.values().map(|token| token.len()).sum::<usize>()
            + self.rows.capacity() * size_of::<Option<Row>>()
            + self.flows.capacity() * size_of::<Members>()
            + self
                .flows
                .iter()
                .map(|members| members.rows.capacity() * size_of::<(u128, u32)>())
                .sum::<usize>()
            + self.index.capacity() * size_of::<(FlowKey, u32)>()
            + self.dirty.capacity() * 4
            + (self.rejected.len() + self.rejected_sent.len()) * size_of::<(Group, u32)>()
            + self.spelled.capacity() * 4
    }
}

// Every segment starts cold: its first hour reads every deal and holder again; what came before
// is the emit's.
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
        let _cpu = replay_frame::cpu::scope("trade.columnar", Some("trade"));
        let state = hour.state;
        let at = hour.hour;
        let roster = hour.world.countries()?;
        let opens = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        if opens {
            self.spelled.clear();
            self.stale = true;
        }
        self.cold = false;
        self.last = Some(at);
        let known = roster.complete()
            && [
                TableId::ResourceExchanges,
                TableId::ResourceDefinitions,
                TableId::LexicalTokens,
            ]
            .iter()
            .all(|&table| columns::complete(state, table));
        if !known {
            let changed = opens || self.known;
            self.known = false;
            self.stale = true;
            let facts = changed.then(|| {
                Box::new(Facts {
                    known: false,
                    whole: true,
                    holders: Vec::new(),
                    flows: Vec::new(),
                    rejected: None,
                    names: Vec::new(),
                })
            });
            return out.push(Hour {
                branch: hour.branch,
                hour: at,
                facts,
            });
        }
        let mut whole = !self.known || self.stale;
        self.known = true;
        if whole
            || touched(state, TableId::ResourceDefinitions)?
            || touched(state, TableId::LexicalTokens)?
        {
            let names = self.named(state)?;
            if names != self.names {
                self.names = names;
                whole = true;
            }
        }
        // A country, a link or a tag's holders changed: every deal is attributed again.
        whole |= roster.changed();
        let mut holders = Vec::new();
        if whole {
            self.holders = self::holders(roster);
            holders = self
                .holders
                .iter()
                .map(|(&slot, holder)| (slot, Some(holder.clone())))
                .collect();
            holders.sort_unstable_by_key(|&(slot, _)| slot);
            self.index_whole(state, roster)?;
            self.stale = false;
        } else if touched(state, TableId::ResourceExchanges)? {
            self.index_changes(state, roster)?;
        }
        let flows = self.flows(whole)?;
        let rejected = self.rejected(whole);
        let mut names = Vec::new();
        for (&id, token) in &self.names {
            if self.spelled.insert(id) {
                names.push((id, token.to_string()));
            }
        }
        let quiet = !whole && flows.is_empty() && rejected.is_none() && names.is_empty();
        let facts = (!quiet).then(|| {
            Box::new(Facts {
                known: true,
                whole,
                holders,
                flows,
                rejected,
                names,
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
                "Not a saved trade projector",
            ));
        }
        *self = Self::new(self.format)?;
        Ok(())
    }
}
