//! Prepared per-country dockyard repair output rates, in fixed5 IC per day.
use crate::{
    metadata::link_slots,
    prepared::{
        frame_changes::{FrameChanges, field_mask},
        series::{self, Accumulator, Reader},
    },
    products::{
        country_groups::HourSpan,
        country_history::Identity,
        equipment::{VariantCatalogue, production::ProductionSpendingUnknown},
        force_census::Countries,
        production_lines::factory_output,
    },
};
use replay::container::Replay;
use replay_data::rows::{TableId, tables};
use replay_derive::{
    naval_repairs::Sources,
    visibility::{AccessLinks, Relations},
};
use replay_frame::frame::{DecodedFrame, TableView, ViewIdentity};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};

/// A sampled repair allocation's design or independently retained residual.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepairPart {
    Design(u64),
    Unattributed,
}

#[derive(Clone, Serialize, Deserialize)]
struct DesignKey {
    key: u64,
    origin: i32,
}

#[derive(Clone, Serialize, Deserialize)]
struct PartKeys {
    unattributed: u64,
    valued: bool,
    designs: BTreeMap<u64, DesignKey>,
}

/// The selector's supplying operands, shared with the pre-pass field report.
/// An empty field list means every field of that table.
pub const SELECTION_FIELDS: &[(TableId, &[&str])] = &[
    (TableId::CountryLinks, &[]),
    (TableId::NavalRepairBases, &["allocated_dockyards"]),
    (TableId::NavalRepairQueue, &[]),
    (
        TableId::NavalBases,
        &["country", "countries", "countries_unreadable"],
    ),
    (TableId::Ships, &["task_force", "strength"]),
    (
        TableId::ShipModels,
        &["variant", "counter", "has_criticals"],
    ),
    (TableId::TaskForces, &["roster_owner", "province"]),
    (TableId::Provinces, &["controller"]),
    (TableId::RelationLinks, &[]),
    (TableId::FactionMembers, &[]),
    (TableId::Subjects, &[]),
    (TableId::WarRelations, &[]),
];

/// Country joins and base and access indexes, rebuilt only when a table
/// they read changes, so a changing hour does no country-wide work.
#[derive(Default)]
struct Joins {
    /// Raw country to canonical slot, for every unambiguous link.
    links: BTreeMap<u32, u32>,
    /// Each slot's raw countries.
    raws: HashMap<u32, Vec<u32>>,
    /// Each holder slot's naval bases (rows of the indexed view).
    bases: Option<(ViewIdentity, HashMap<u32, Vec<usize>>)>,
    /// Port relations and access, with the views they read (none for a
    /// view that was not complete, which any other incomplete view repeats).
    #[allow(clippy::type_complexity)]
    access: Option<(
        Vec<Option<ViewIdentity>>,
        Result<Relations, &'static str>,
        Result<AccessLinks, &'static str>,
    )>,
    /// Country-wide index builds, for the churn regression.
    #[cfg(any(test, feature = "test-support"))]
    builds: usize,
}

impl Joins {
    fn link(&mut self, links: &[replay_data::rows::CountryLinkRow]) {
        self.links = link_slots(links).into_iter().collect();
        self.raws.clear();
        for (&raw, &slot) in &self.links {
            self.raws.entry(slot).or_default().push(raw);
        }
        // Base holders join through the links.
        self.bases = None;
        self.access = None;
        #[cfg(any(test, feature = "test-support"))]
        {
            self.builds += 1;
        }
    }

    /// The one row of a key-sorted country table that names `slot` through
    /// any of its raw countries; none when none or several do.
    fn row<'r, T>(
        &self,
        rows: Option<&'r [T]>,
        slot: u32,
        key: impl Fn(&T) -> u32,
    ) -> Option<&'r T> {
        let rows = rows?;
        let mut found = None;
        for &raw in self.raws.get(&slot)? {
            if let Ok(at) = rows.binary_search_by_key(&raw, &key) {
                if found.is_some() {
                    return None;
                }
                found = Some(&rows[at]);
            }
        }
        found
    }

    /// Indexes the bases by holder and reads the port access again, each
    /// only when a table it reads changed.
    fn refresh(&mut self, frame: &DecodedFrame) {
        let view = &frame.tables[TableId::NavalBases.index()];
        if let Ok(rows) = frame.complete::<tables::NavalBases>()
            && !self.bases.as_ref().is_some_and(|(held, _)| held.is(view))
        {
            let mut by_holder: HashMap<u32, Vec<usize>> = HashMap::new();
            for (index, base) in rows.iter().enumerate() {
                if let Some(&slot) = self.links.get(&base.country) {
                    by_holder.entry(slot).or_default().push(index);
                }
            }
            self.bases = Some((view.identity(), by_holder));
            #[cfg(any(test, feature = "test-support"))]
            {
                self.builds += 1;
            }
        }
        const READ: [TableId; 4] = [
            TableId::FactionMembers,
            TableId::WarRelations,
            TableId::Subjects,
            TableId::RelationLinks,
        ];
        let complete = |table: TableId| {
            let view = &frame.tables[table.index()];
            matches!(**view, TableView::Complete(_)).then(|| view.identity())
        };
        let current = self.access.as_ref().is_some_and(|(held, ..)| {
            held.iter().zip(READ).all(|(held, table)| {
                let view = &frame.tables[table.index()];
                match held {
                    Some(held) => held.is(view),
                    None => !matches!(**view, TableView::Complete(_)),
                }
            })
        });
        if !current {
            self.access = Some((
                READ.map(complete).to_vec(),
                Relations::read(frame, &self.links),
                AccessLinks::read(frame, &self.links),
            ));
            #[cfg(any(test, feature = "test-support"))]
            {
                self.builds += 1;
            }
        }
    }
}

#[derive(Default)]
struct Selection {
    /// The base and assignment tables were read for this holder.
    readable: bool,
    /// Its bases' assigned dockyards, which reconcile with its use.
    allocated: i128,
    occurrences: i128,
    designs: BTreeMap<u64, i128>,
}

#[derive(Default, Serialize, Deserialize)]
struct Catalogue {
    recorded: bool,
    gaps: Vec<HourSpan>,
    origins: BTreeMap<String, i32>,
    parts: BTreeMap<String, PartKeys>,
    variants: VariantCatalogue,
    /// Observed hours at which a holder's base assignments did not sum to
    /// its dockyards in use; its split was not known there.
    allocation_mismatches: u64,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    keys: BTreeMap<String, u64>,
    next_key: u64,
    catalogue: Catalogue,
    holders: BTreeMap<String, Option<Identity>>,
    rates: Accumulator,
    last: Option<i32>,
    modifiers_valid: bool,
    #[serde(skip)]
    countries: Countries,
    #[serde(skip)]
    inputs: Option<[ViewIdentity; 2]>,
    #[serde(skip)]
    catalogue_writer: series::CatalogueWriter,
    #[serde(skip)]
    selection_inputs: Vec<(ViewIdentity, bool)>,
    #[serde(skip)]
    selections: BTreeMap<u32, Selection>,
    /// Slots whose last observed use was positive, revisited when a
    /// selection operand changes.
    #[serde(skip)]
    using: BTreeSet<u32>,
    #[serde(skip)]
    joins: Joins,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub country_visits: usize,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub base_visits: usize,
}

impl Prepared {
    pub fn observe(
        &mut self,
        frame: &DecodedFrame,
        replay: &Replay,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        let continuous = self.last.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
        if self.inputs.is_none() {
            self.catalogue.recorded = replay
                .format
                .table_refusal(TableId::NavalRepairCountries)
                .is_none();
            self.catalogue.gaps.extend(
                replay
                    .gaps
                    .iter()
                    .filter(|gap| gap.branch == frame.branch)
                    .map(|gap| HourSpan {
                        first_hour: gap.first_hour,
                        last_hour: gap.last_hour,
                    }),
            );
            let mut end: Option<i32> = None;
            for block in replay
                .index
                .iter()
                .filter(|block| block.branch == frame.branch)
            {
                if let Some(first) = end.and_then(|hour| hour.checked_add(1))
                    && first < block.first_hour
                {
                    self.catalogue.gaps.push(HourSpan {
                        first_hour: first,
                        last_hour: block.first_hour - 1,
                    });
                }
                end = Some(block.last_hour);
            }
        }
        if !continuous
            && let Some(last) = self.last
            && let Some(first) = last.checked_add(1)
            && first < frame.hour
        {
            self.catalogue.gaps.push(HourSpan {
                first_hour: first,
                last_hour: frame.hour - 1,
            });
        }
        if self.inputs.is_none() || !continuous {
            self.catalogue.gaps.sort_by_key(|gap| gap.first_hour);
            let mut gaps = Vec::new();
            for gap in std::mem::take(&mut self.catalogue.gaps) {
                crate::products::country_groups::mark_unobserved(
                    &mut gaps,
                    gap.first_hour,
                    gap.last_hour,
                );
            }
            self.catalogue.gaps = gaps;
        }
        let before = self.modifiers_valid;
        if !continuous
            || frame
                .complete::<tables::CountryProductionModifiers>()
                .is_err()
        {
            self.modifiers_valid = false;
        }
        // Daily views already hold their last read between production slots.
        // A refusal withdraws that read; only a successful supplying read can
        // restore it, and no read is carried across a recording gap.
        if changes.slot_read[TableId::CountryProductionModifiers.index()] {
            self.modifiers_valid = frame
                .complete::<tables::CountryProductionModifiers>()
                .is_ok();
        }
        self.catalogue
            .variants
            .observe_changed(replay.format, frame, changes);
        let joined = self.countries.update_changed(frame, Some(changes));
        let selection_changed = !continuous
            || self.selection_inputs.len() != SELECTION_FIELDS.len()
            || SELECTION_FIELDS
                .iter()
                .enumerate()
                .any(|(index, (table, fields))| {
                    let (held, complete) = &self.selection_inputs[index];
                    let view = &frame.tables[table.index()];
                    // A repeated incomplete operand still cannot be read by
                    // the selector; it cannot change a numeric attribution.
                    !held.is(view)
                        && (*complete || matches!(**view, TableView::Complete(_)))
                        && changes.rows(*table).is_none_or(|(rows, deleted)| {
                            !deleted.is_empty()
                                || rows.iter().any(|row| {
                                    fields.is_empty() || row.mask & field_mask(*table, fields) != 0
                                })
                        })
                });
        self.selection_inputs = SELECTION_FIELDS
            .iter()
            .map(|(table, _)| {
                let view = &frame.tables[table.index()];
                (view.identity(), matches!(**view, TableView::Complete(_)))
            })
            .collect();
        if selection_changed {
            self.selections.clear();
        }
        let watched = [
            (TableId::NavalRepairCountries, &["num_used_dockyards"][..]),
            (
                TableId::CountryProductionModifiers,
                &["power_raw", "factory_speed", "trees"][..],
            ),
        ];
        let changed = !continuous
            || !matches!(joined, Ok(false))
            || before != self.modifiers_valid
            || self.inputs.as_ref().is_none_or(|inputs| {
                watched.iter().enumerate().any(|(index, (table, fields))| {
                    !inputs[index].is(&frame.tables[table.index()])
                        && changes.rows(*table).is_none_or(|(rows, deleted)| {
                            !deleted.is_empty()
                                || rows
                                    .iter()
                                    .any(|row| row.mask & field_mask(*table, fields) != 0)
                        })
                })
            });
        let first = self.inputs.is_none();
        self.inputs = Some(watched.map(|(table, _)| frame.tables[table.index()].identity()));
        self.rates.advance(frame.hour)?;
        self.last = Some(frame.hour);
        if !changed && !selection_changed {
            return Ok(());
        }
        let links = frame.complete::<tables::CountryLinks>().ok();
        if first || !continuous || !matches!(joined, Ok(false)) {
            self.joins.link(links.unwrap_or_default());
        }
        // The holders this hour can change: every one at the first hour,
        // after a gap, a join change or a daily-output validity change, or
        // when a watched table has no row facts; otherwise the countries
        // whose use or output rows changed and, when a selection operand
        // changed, the countries using dockyards. The rest hold their
        // published values without a visit.
        let mut dirty: Option<BTreeSet<u32>> =
            (!first && continuous && matches!(joined, Ok(false)) && before == self.modifiers_valid)
                .then(BTreeSet::new);
        let mut facts = true;
        if changed && let Some(set) = dirty.as_mut() {
            for (table, _) in &watched {
                match changes.rows(*table) {
                    Some((rows, deleted)) => set.extend(
                        rows.iter()
                            .map(|row| row.key)
                            .chain(deleted.iter().copied())
                            .filter_map(|raw| u32::try_from(raw).ok())
                            .filter_map(|raw| self.joins.links.get(&raw).copied()),
                    ),
                    None => facts = false,
                }
            }
        }
        if !facts {
            dirty = None;
        }
        if selection_changed && let Some(set) = dirty.as_mut() {
            set.extend(self.using.iter().copied());
        }
        let visit = match &dirty {
            Some(set) => self.countries.in_slots(set),
            None => self.countries.tags.clone(),
        };
        let repairs = frame.complete::<tables::NavalRepairCountries>().ok();
        let modifiers = frame
            .complete::<tables::CountryProductionModifiers>()
            .ok()
            .filter(|_| self.modifiers_valid);
        let count_of = |joins: &Joins, slot: u32| {
            joins
                .row(repairs, slot, |row| row.country)
                .map(|row| row.num_used_dockyards)
        };
        let output_of = |joins: &Joins, slot: u32| {
            joins
                .row(modifiers, slot, |row| row.country)
                .and_then(|row| factory_output(row, true))
        };
        // Only positive, valued use demands a queue walk. Selections are
        // cached apart from use and output: those changes reread no queue.
        let wanted: BTreeSet<u32> = visit
            .iter()
            .filter_map(|(_, identity)| identity.map(|(slot, _)| slot))
            .filter(|slot| !self.selections.contains_key(slot))
            .filter(|&slot| {
                count_of(&self.joins, slot).is_some_and(|count| count > 0)
                    && output_of(&self.joins, slot).is_some()
            })
            .collect();
        if joined.is_ok() && links.is_some() && !wanted.is_empty() {
            Self::read_selections(
                frame,
                &mut self.joins,
                &mut self.selections,
                &wanted,
                #[cfg(any(test, feature = "test-support"))]
                &mut self.base_visits,
            )?;
        }
        // A full pass rebuilds the holders; a partial one updates those it
        // visits in place and keeps the rest.
        let full = dirty.is_none();
        let mut current = BTreeMap::new();
        if joined.is_ok() && links.is_some() {
            for (tag, identity) in &visit {
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.country_visits += 1;
                }
                let Ok(tag) = std::str::from_utf8(tag) else {
                    continue;
                };
                let key = *self.keys.entry(tag.to_owned()).or_insert_with(|| {
                    let key = self.next_key;
                    self.next_key += 1;
                    key
                });
                let parts = self
                    .catalogue
                    .parts
                    .entry(tag.to_owned())
                    .or_insert_with(|| {
                        let key = self.next_key;
                        self.next_key += 1;
                        PartKeys {
                            unattributed: key,
                            valued: false,
                            designs: BTreeMap::new(),
                        }
                    });
                let starts = !continuous || self.holders.get(tag) != Some(identity);
                let slot = identity.map(|(slot, _)| slot);
                let count = slot.and_then(|slot| count_of(&self.joins, slot));
                let output = slot.and_then(|slot| output_of(&self.joins, slot));
                if let Some(slot) = slot {
                    if count.is_some_and(|count| count > 0) {
                        self.using.insert(slot);
                    } else {
                        self.using.remove(&slot);
                    }
                }
                // The writer admits every selecting country, including zero
                // use; its certificate never defines an omitted row as zero.
                let rate = match count {
                    Some(0) => Some(0),
                    Some(count @ 1..) => {
                        output.and_then(|output| i128::from(count).checked_mul(i128::from(output)))
                    }
                    _ => None,
                };
                if rate.is_some() {
                    self.catalogue
                        .origins
                        .entry(tag.to_owned())
                        .or_insert(frame.hour);
                }
                self.rates
                    .observe_if_changed(key, frame.hour, rate, starts, false);
                let selection = slot.and_then(|slot| self.selections.get(&slot));
                // The split needs the holder's base assignments to sum to its
                // use, and its selected occurrences to stay within them. A
                // mismatch is counted and named; the total stays and the split
                // is not known. Never clamp it or publish a negative residual.
                let reconciled = match count {
                    Some(0) => true,
                    Some(count) => selection.is_some_and(|selection| {
                        selection.readable
                            && selection.allocated == i128::from(count)
                            && selection.occurrences <= i128::from(count)
                    }),
                    None => false,
                };
                if rate.is_some_and(|rate| rate != 0)
                    && !reconciled
                    && selection.is_some_and(|selection| selection.readable)
                {
                    self.catalogue.allocation_mismatches += 1;
                }
                let split_known = rate.is_some() && reconciled;
                let selected_designs = selection
                    .filter(|_| count != Some(0))
                    .map(|selection| &selection.designs);
                if split_known {
                    parts.valued = true;
                    for &design in selected_designs.into_iter().flat_map(|rows| rows.keys()) {
                        parts.designs.entry(design).or_insert_with(|| {
                            let key = self.next_key;
                            self.next_key += 1;
                            DesignKey {
                                key,
                                origin: frame.hour,
                            }
                        });
                    }
                }
                let mut attributed = 0_i128;
                for (&design, part) in &parts.designs {
                    let value = if !split_known {
                        None
                    } else if count == Some(0) {
                        Some(0)
                    } else {
                        let slots = selected_designs
                            .and_then(|rows| rows.get(&design))
                            .copied()
                            .unwrap_or(0);
                        attributed = attributed
                            .checked_add(slots)
                            .ok_or_else(|| crate::invalid("Repair slot count overflow"))?;
                        slots.checked_mul(i128::from(output.expect("valued positive use")))
                    };
                    self.rates
                        .observe_if_changed(part.key, frame.hour, value, starts, false);
                }
                let residual = if !split_known {
                    None
                } else if count == Some(0) {
                    Some(0)
                } else {
                    (i128::from(count.expect("valued use")) - attributed)
                        .checked_mul(i128::from(output.expect("valued positive use")))
                };
                self.rates.observe_if_changed(
                    parts.unattributed,
                    frame.hour,
                    residual,
                    starts,
                    false,
                );
                if full {
                    current.insert(tag.to_owned(), *identity);
                } else {
                    self.holders.insert(tag.to_owned(), *identity);
                }
            }
        }
        // A full pass ends the holders it did not find.
        if full {
            for tag in self
                .holders
                .keys()
                .filter(|tag| !current.contains_key(*tag))
            {
                self.rates
                    .observe_if_changed(self.keys[tag], frame.hour, None, false, false);
                if let Some(parts) = self.catalogue.parts.get(tag) {
                    for key in std::iter::once(parts.unattributed)
                        .chain(parts.designs.values().map(|part| part.key))
                    {
                        self.rates
                            .observe_if_changed(key, frame.hour, None, false, false);
                    }
                }
            }
            let held: BTreeSet<u32> = current
                .values()
                .filter_map(|identity| identity.map(|(slot, _)| slot))
                .collect();
            self.using.retain(|slot| held.contains(slot));
            self.holders = current;
        }
        Ok(())
    }

    /// The holders' current selections: each base they hold with a positive
    /// assignment, through the cached base and access indexes.
    fn read_selections(
        frame: &DecodedFrame,
        joins: &mut Joins,
        selections: &mut BTreeMap<u32, Selection>,
        wanted: &BTreeSet<u32>,
        #[cfg(any(test, feature = "test-support"))] base_visits: &mut usize,
    ) -> io::Result<()> {
        selections.extend(wanted.iter().map(|&slot| (slot, Selection::default())));
        let (Ok(bases), Ok(allocations)) = (
            frame.complete::<tables::NavalBases>(),
            frame.complete::<tables::NavalRepairBases>(),
        ) else {
            return Ok(());
        };
        joins.refresh(frame);
        let Some((_, held)) = &joins.bases else {
            return Ok(());
        };
        let (_, relations, access) = joins.access.as_ref().expect("access refreshed");
        let sources = Sources::with_access(
            frame,
            &joins.links,
            relations.as_ref().map_err(|why| *why),
            access.as_ref().map_err(|why| *why),
        );
        let models = frame.complete::<tables::ShipModels>().ok();
        for &slot in wanted {
            let selection = selections.get_mut(&slot).expect("requested holder");
            selection.readable = true;
            for &index in held.get(&slot).into_iter().flatten() {
                let base = &bases[index];
                let allocated = allocations
                    .binary_search_by_key(&base.province, |row| row.province)
                    .ok()
                    .map_or(0, |at| allocations[at].allocated_dockyards);
                if allocated <= 0 {
                    continue;
                }
                #[cfg(any(test, feature = "test-support"))]
                {
                    *base_visits += 1;
                }
                selection.allocated = selection
                    .allocated
                    .checked_add(i128::from(allocated))
                    .ok_or_else(|| crate::invalid("Repair allocation sum overflow"))?;
                // A base whose selection refuses places none of its dockyards:
                // they stay unattributed.
                let Ok(selected) = sources.selected(base) else {
                    continue;
                };
                selection.occurrences =
                    selection
                        .occurrences
                        .checked_add(selected.len() as i128)
                        .ok_or_else(|| crate::invalid("Repair occurrence count overflow"))?;
                for occurrence in selected {
                    let design = models
                        .and_then(|rows| {
                            rows.binary_search_by_key(&occurrence.ship, |row| row.ship)
                                .ok()
                                .map(|at| rows[at].variant)
                        })
                        .filter(|&id| id != 0);
                    if let Some(design) = design {
                        let slots = selection.designs.entry(design).or_default();
                        *slots = slots
                            .checked_add(1)
                            .ok_or_else(|| crate::invalid("Repair design count overflow"))?;
                    }
                }
            }
        }
        Ok(())
    }

    /// An unchanged consecutive hour; no country or series key is visited.
    /// The extract retains frames at output and variant naming reads.
    pub fn advance_quiet(&mut self, hour: i32, supplying_read: bool) -> io::Result<bool> {
        if supplying_read
            || self.inputs.is_none()
            || self.last.and_then(|hour| hour.checked_add(1)) != Some(hour)
        {
            return Ok(false);
        }
        self.rates.advance(hour)?;
        self.catalogue.variants.advance(hour);
        self.last = Some(hour);
        Ok(true)
    }

    pub fn bytes(&self) -> usize {
        self.rates.bytes()
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.rates
            .flush(writer, &format!("dockyard-repairs-{branch}"), 5, cancelled)?;
        let designs: BTreeSet<_> = self
            .catalogue
            .parts
            .values()
            .flat_map(|parts| parts.designs.keys().copied())
            .collect();
        let catalogue = Catalogue {
            recorded: self.catalogue.recorded,
            gaps: self.catalogue.gaps.clone(),
            origins: self.catalogue.origins.clone(),
            parts: self.catalogue.parts.clone(),
            allocation_mismatches: self.catalogue.allocation_mismatches,
            // The continuation keeps supplying reads for newly selected designs;
            // query storage retains only designs that actually held repair keys.
            variants: VariantCatalogue::seeded(
                self.catalogue
                    .variants
                    .versions
                    .iter()
                    .filter(|(id, _)| designs.contains(*id))
                    .map(|(&id, versions)| (id, versions.clone()))
                    .collect(),
            ),
        };
        self.catalogue_writer
            .write(writer, "dockyard-repairs", branch, &self.keys, &catalogue)
    }
}

/// One query's reads of a branch's repair rate series.
pub struct RepairReads<'a> {
    rates: Reader<'a>,
    keys: BTreeMap<String, u64>,
    catalogue: Catalogue,
    cancelled: &'a AtomicBool,
}

impl<'a> RepairReads<'a> {
    pub fn new(
        store: &'a store::Store,
        key: &'a store::SourceKey,
        branch: u64,
        cancelled: &'a AtomicBool,
    ) -> io::Result<Self> {
        crate::metadata::check_cancel(cancelled)?;
        let (keys, catalogue) =
            series::read_catalogue(store, key, "dockyard-repairs", branch)?.unwrap_or_default();
        Ok(Self {
            rates: Reader::new(store, key, format!("dockyard-repairs-{branch}"), cancelled)?,
            keys,
            catalogue,
            cancelled,
        })
    }

    /// First hour this tag's rate holds a value, including an observed zero.
    pub fn origin(&mut self, tag: &str) -> io::Result<Option<i32>> {
        crate::metadata::check_cancel(self.cancelled)?;
        Ok(self
            .catalogue
            .origins
            .get(tag)
            .copied()
            .filter(|_| self.catalogue.recorded))
    }

    /// Parts that ever held a value, in their series-key order.
    pub fn parts(&mut self, tag: &str) -> io::Result<Vec<RepairPart>> {
        crate::metadata::check_cancel(self.cancelled)?;
        let Some(parts) = self
            .catalogue
            .parts
            .get(tag)
            .filter(|_| self.catalogue.recorded)
        else {
            return Ok(Vec::new());
        };
        let mut keys: Vec<_> = parts
            .designs
            .iter()
            .map(|(&design, part)| (part.key, RepairPart::Design(design)))
            .collect();
        if parts.valued {
            keys.push((parts.unattributed, RepairPart::Unattributed));
        }
        keys.sort_by_key(|(key, _)| *key);
        Ok(keys.into_iter().map(|(_, part)| part).collect())
    }

    pub fn variants(&self) -> &VariantCatalogue {
        &self.catalogue.variants
    }

    /// Observed hours at which a holder's base assignments did not sum to
    /// its dockyards in use, so its split by design was not known.
    pub fn allocation_mismatches(&self) -> u64 {
        self.catalogue.allocation_mismatches
    }

    /// Like `integrals`, for one sampled-allocation part. Before a design's
    /// first selection it is zero only where the common split was known.
    pub fn part_integrals(
        &mut self,
        tag: &str,
        part: RepairPart,
        spans: &[(i32, i32)],
    ) -> io::Result<Vec<Result<i128, ProductionSpendingUnknown>>> {
        let keys = self
            .catalogue
            .parts
            .get(tag)
            .filter(|_| self.catalogue.recorded);
        let unattributed = keys.map(|keys| keys.unattributed);
        let design = match part {
            RepairPart::Unattributed => return self.integrate(unattributed, spans),
            RepairPart::Design(id) => keys
                .and_then(|keys| keys.designs.get(&id))
                .map(|part| (part.key, part.origin)),
        };
        let Some((key, origin)) = design else {
            return self.integrate(None, spans);
        };
        let coverage = self.integrate(unattributed, spans)?;
        let clipped: Vec<_> = spans
            .iter()
            .filter_map(|&(first, last)| (last >= origin).then_some((first.max(origin), last)))
            .collect();
        let mut values = self.integrate(Some(key), &clipped)?.into_iter();
        Ok(spans
            .iter()
            .zip(coverage)
            .map(|(&(_, last), covered)| {
                let value = if last < origin {
                    Ok(0)
                } else {
                    values.next().expect("clipped span")
                };
                covered.and(value)
            })
            .collect())
    }

    /// Sum rate times inclusive hours for each span, reading only this tag's
    /// runs once over their hull. Missing hours refuse as not recorded, a
    /// recording gap or unavailable spending; holder breaks refuse as a
    /// country change. The caller applies the daily divisor.
    pub fn integrals(
        &mut self,
        tag: &str,
        spans: &[(i32, i32)],
    ) -> io::Result<Vec<Result<i128, ProductionSpendingUnknown>>> {
        let key = self
            .keys
            .get(tag)
            .copied()
            .filter(|_| self.catalogue.recorded);
        self.integrate(key, spans)
    }

    fn integrate(
        &mut self,
        key: Option<u64>,
        spans: &[(i32, i32)],
    ) -> io::Result<Vec<Result<i128, ProductionSpendingUnknown>>> {
        use ProductionSpendingUnknown::{
            CountryChanged, NotRecorded, RecordingGap, SpendingUnavailable,
        };
        crate::metadata::check_cancel(self.cancelled)?;
        if spans.iter().any(|(first, last)| first > last) {
            return Err(crate::invalid("Repair integration span is reversed"));
        }
        let Some(first) = spans.iter().map(|span| span.0).min() else {
            return Ok(Vec::new());
        };
        let last = spans
            .iter()
            .map(|span| span.1)
            .max()
            .expect("nonempty repair spans");
        let Some(key) = key else {
            return Ok(vec![Err(NotRecorded); spans.len()]);
        };
        let runs = self.rates.runs(key, first, last)?;
        let missing =
            |first: i64, last: i64| {
                if self.catalogue.gaps.iter().any(|gap| {
                    i64::from(gap.first_hour) <= last && first <= i64::from(gap.last_hour)
                }) {
                    RecordingGap
                } else {
                    SpendingUnavailable
                }
            };
        spans
            .iter()
            .map(|&(first, last)| {
                crate::metadata::check_cancel(self.cancelled)?;
                let mut cursor = i64::from(first);
                let mut total = 0_i128;
                let start = runs.partition_point(|run| run.last_hour < first);
                for run in runs[start..]
                    .iter()
                    .take_while(|run| run.first_hour <= last)
                {
                    if i64::from(run.first_hour) > cursor {
                        return Ok(Err(missing(cursor, i64::from(run.first_hour) - 1)));
                    }
                    if run.break_before && run.first_hour > first {
                        return Ok(Err(CountryChanged));
                    }
                    let end = run.last_hour.min(last);
                    let hours = i128::from(i64::from(end) - cursor + 1);
                    total = run
                        .value
                        .checked_mul(hours)
                        .and_then(|amount| total.checked_add(amount))
                        .ok_or_else(|| crate::invalid("Repair output integral overflow"))?;
                    cursor = i64::from(end) + 1;
                }
                Ok(if cursor <= i64::from(last) {
                    Err(missing(cursor, i64::from(last)))
                } else {
                    Ok(total)
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
