//! Prepared per-country dockyard repair output rates, in fixed5 IC per day.
use crate::{
    prepared::{
        prepass::HeldBytes,
        series::{self, Accumulator, Reader},
    },
    products::{
        country_groups::HourSpan,
        country_history::Identity,
        equipment::{
            RawVariant, VariantCatalogue, VariantHour, production::ProductionSpendingUnknown,
        },
    },
};
use replay::container::Replay;
use replay_data::rows::{NavalBaseRow, TableId};
use replay_derive::naval_repairs;
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

pub mod facts;

#[derive(Clone, PartialEq, Default, Debug)]
#[derive(replay_spool::Spooled)]
pub struct Selection {
    pub readable: bool,
    pub allocated: i128,
    pub occurrences: i128,
    pub designs: BTreeMap<u64, i128>,
}

#[derive(Clone, PartialEq, Default, Debug)]
#[derive(replay_spool::Spooled)]
pub struct Inputs {
    pub count: Option<i32>,
    pub output: Option<i64>,
    pub selection: Option<Selection>,
}

/// The countries as the census joins them: each held tag with its found country, and every
/// link naming a found country's slot, by raw id.
#[derive(replay_spool::Spooled)]
pub struct Roster {
    pub ok: bool,
    pub tags: Vec<(Vec<u8>, Option<Identity>)>,
    pub owners: Vec<(u32, u32)>,
}

pub struct Observed<'a> {
    pub hour: i32,
    pub whole: bool,
    pub roster: Option<Roster>,
    pub modifiers: (bool, bool),
    pub inputs: &'a [(u32, Inputs)],
    pub variants: VariantHour,
    pub built: Vec<(u64, RawVariant)>,
    pub variants_whole: bool,
}

pub fn selection<O: naval_repairs::Operands>(
    ops: &O,
    base: &NavalBaseRow,
) -> io::Result<Selection> {
    let mut result = Selection {
        readable: true,
        ..Selection::default()
    };
    let allocated = ops.allocation(base.province).ok().flatten().unwrap_or(0);
    if allocated <= 0 {
        return Ok(result);
    }
    result.allocated = result
        .allocated
        .checked_add(i128::from(allocated))
        .ok_or_else(|| crate::invalid("Repair allocation sum overflow"))?;
    let Ok(selected) = naval_repairs::selected(ops, base) else {
        return Ok(result);
    };
    result.occurrences = result
        .occurrences
        .checked_add(selected.len() as i128)
        .ok_or_else(|| crate::invalid("Repair occurrence count overflow"))?;
    for occurrence in selected {
        if let Ok(Some(model)) = ops.model(occurrence.ship)
            && model.variant != 0
        {
            let count = result.designs.entry(model.variant).or_default();
            *count = count
                .checked_add(1)
                .ok_or_else(|| crate::invalid("Repair design count overflow"))?;
        }
    }
    Ok(result)
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
pub struct Publisher {
    keys: BTreeMap<String, u64>,
    next_key: u64,
    catalogue: Catalogue,
    holders: BTreeMap<String, Option<Identity>>,
    rates: Accumulator,
    last: Option<i32>,
    modifiers_valid: bool,
    mismatched: BTreeSet<String>,
    #[serde(skip)]
    tags: Vec<(Vec<u8>, Option<Identity>)>,
    #[serde(skip)]
    ok: bool,
    #[serde(skip)]
    owners: Vec<(u32, u32)>,
    #[serde(skip)]
    inputs: HashMap<u32, Inputs>,
    #[serde(skip)]
    slots: HashMap<u32, Vec<usize>>,
    #[serde(skip)]
    catalogue_writer: series::CatalogueWriter,
    #[serde(skip)]
    started: bool,
}

impl Publisher {
    pub fn observe(&mut self, replay: &Replay, branch: u64, hour: Observed<'_>) -> io::Result<()> {
        let continuous = self.last.and_then(|hour| hour.checked_add(1)) == Some(hour.hour);
        if !self.started {
            self.catalogue.recorded = replay
                .format
                .table_refusal(TableId::NavalRepairCountries)
                .is_none();
            self.catalogue
                .gaps
                .extend(
                    replay
                        .gaps
                        .iter()
                        .filter(|gap| gap.branch == branch)
                        .map(|gap| HourSpan {
                            first_hour: gap.first_hour,
                            last_hour: gap.last_hour,
                        }),
                );
            let mut end: Option<i32> = None;
            for block in replay.index.iter().filter(|block| block.branch == branch) {
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
            && first < hour.hour
        {
            self.catalogue.gaps.push(HourSpan {
                first_hour: first,
                last_hour: hour.hour - 1,
            });
        }
        if !self.started || !continuous {
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
        if !continuous || !hour.modifiers.0 {
            self.modifiers_valid = false;
        }
        if hour.modifiers.1 {
            self.modifiers_valid = hour.modifiers.0;
        }
        let mut short = false;
        self.catalogue
            .variants
            .observe_hour(hour.hour, hour.variants, |forgot| {
                short = forgot && !hour.variants_whole;
                hour.built
            });
        if short {
            return Err(crate::invalid(
                "Repair variant read after forgetting must be whole",
            ));
        }
        let mut joined = false;
        if let Some(roster) = hour.roster {
            joined =
                self.ok != roster.ok || self.tags != roster.tags || self.owners != roster.owners;
            self.ok = roster.ok;
            self.tags = roster.tags;
            self.owners = roster.owners;
            self.slots.clear();
            for (index, (_, identity)) in self.tags.iter().enumerate() {
                if let Some((slot, _)) = identity {
                    self.slots.entry(*slot).or_default().push(index);
                }
            }
        }
        let mut dirty = BTreeSet::new();
        if hour.whole {
            let inputs: HashMap<_, _> = hour.inputs.iter().cloned().collect();
            dirty.extend(
                self.inputs
                    .keys()
                    .filter(|slot| !inputs.contains_key(*slot))
                    .copied(),
            );
            dirty.extend(inputs.iter().filter_map(|(slot, input)| {
                (self.inputs.get(slot) != Some(input)).then_some(*slot)
            }));
            self.inputs = inputs;
        } else {
            for (slot, input) in hour.inputs {
                if self.inputs.get(slot) != Some(input) {
                    dirty.insert(*slot);
                    self.inputs.insert(*slot, input.clone());
                }
            }
        }
        self.rates.advance(hour.hour)?;
        let full = !self.started || !continuous || joined || before != self.modifiers_valid;
        let visit: Vec<_> = if full {
            (0..self.tags.len()).collect()
        } else {
            dirty
                .iter()
                .flat_map(|slot| self.slots.get(slot).into_iter().flatten().copied())
                .collect()
        };
        let mut current = BTreeMap::new();
        if self.ok {
            for index in visit {
                let (tag, identity) = &self.tags[index];
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
                let input = slot.and_then(|slot| self.inputs.get(&slot));
                let count = input.and_then(|input| input.count);
                let output = input
                    .and_then(|input| input.output)
                    .filter(|_| self.modifiers_valid);
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
                        .or_insert(hour.hour);
                }
                self.rates
                    .observe_if_changed(key, hour.hour, rate, starts, false);
                let selection = input.and_then(|input| input.selection.as_ref());
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
                    self.mismatched.insert(tag.to_owned());
                } else {
                    self.mismatched.remove(tag);
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
                                origin: hour.hour,
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
                        .observe_if_changed(part.key, hour.hour, value, starts, false);
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
                    hour.hour,
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
        if full {
            for tag in self
                .holders
                .keys()
                .filter(|tag| !current.contains_key(*tag))
            {
                self.mismatched.remove(tag);
                self.rates
                    .observe_if_changed(self.keys[tag], hour.hour, None, false, false);
                if let Some(parts) = self.catalogue.parts.get(tag) {
                    for key in std::iter::once(parts.unattributed)
                        .chain(parts.designs.values().map(|part| part.key))
                    {
                        self.rates
                            .observe_if_changed(key, hour.hour, None, false, false);
                    }
                }
            }
            self.holders = current;
        }
        self.catalogue.allocation_mismatches += self.mismatched.len() as u64;
        self.last = Some(hour.hour);
        self.started = true;
        Ok(())
    }

    pub fn bytes(&self) -> usize {
        let mut bytes = HeldBytes::default();
        bytes
            .heap(&self.keys)
            .heap(&self.holders)
            .heap(&self.tags)
            .heap(&self.owners)
            .heap(&self.inputs)
            .heap(&self.slots)
            .heap(&self.mismatched)
            .heap(&self.catalogue.gaps)
            .heap(&self.catalogue.origins)
            .heap(&self.catalogue.variants);
        bytes.tree::<String, PartKeys>(self.catalogue.parts.len());
        for (tag, parts) in &self.catalogue.parts {
            bytes.heap(tag).tree::<u64, DesignKey>(parts.designs.len());
        }
        self.rates.bytes() + bytes.total() as usize
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
