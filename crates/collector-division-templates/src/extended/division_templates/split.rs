use super::*;
use crate::template_changes::Changes;
use collector_workers::hour_stages::TemplateFallback as Fallback;
use engine_allocation::SharedBox;
use std::ops::Range;

const SPLIT_ROWS: usize = 2 * PART_ROWS;
const PART_ROWS: usize = 64;

#[derive(Default)]
pub(super) struct Lent {
    held: held::Store,
    core: super::core::Store,
    countries: countries::Store,
}

#[derive(Clone, Copy)]
struct Occurrence {
    country: u32,
    slot: u32,
    visit: u32,
    pointer: usize,
    template: u64,
    ordinal: u32,
    stamps: Option<held::Certificate>,
    certificate: Option<super::core::Certificate>,
    deployment: Option<usize>,
    details: bool,
    carried: bool,
}

enum Event {
    Batch,
    Demand(Occurrence),
    Placed([usize; 4]),
}

#[derive(Default)]
pub struct WalkPart {
    slots: Range<u32>,
    state: usize,
    lent: Option<SharedBox<Lent>>,
    rows: Vec<native::IconRow>,
    bytes: Vec<u8>,
    counts: Vec<native::SubunitCount>,
    variants: Vec<u64>,
    names: Vec<u8>,
    database: Vec<usize>,
    globals: Vec<native::GlobalSlot>,
    locals: Vec<native::LocalSlot>,
    walk: Option<native::IconWalk>,
    events: Vec<Event>,
    visits: Vec<countries::Visit>,
    scratch: Vec<GarrisonTemplateSelection>,
    speed_keys: Vec<u32>,
    speed_spans: Vec<(u32, Option<(usize, usize)>)>,
    row_offset: usize,
    country_ticks: u64,
    incomplete: bool,
}

impl WalkPart {
    pub fn capture(&mut self, memory: &impl Memory, base: usize, changes: &dyn Changes) {
        self.walk = None;
        self.events.clear();
        self.visits.clear();
        self.speed_keys.clear();
        self.speed_spans.clear();
        self.country_ticks = 0;
        self.incomplete = false;
        let Some(lent) = self.lent.clone() else {
            self.incomplete = true;
            return;
        };
        let Lent {
            held: held_store,
            core: core_store,
            countries: country_store,
        } = &*lent;
        let _reads = collector_memory::read_accounting::table(TABLES[1]);
        let (state, slots) = (self.state, self.slots.clone());
        let mut lost_visit = false;
        let slot = std::cell::Cell::new(0_u32);
        let visits = &mut self.visits;
        let scratch = &mut self.scratch;
        let country_ticks = &mut self.country_ticks;
        let mut visit_country = |country: engine_native_memory::country_identity::Country| {
            let start = qpc();
            slot.set(country.canonical_slot);
            let _reads = collector_memory::read_accounting::table(TABLES[2]);
            let visit = country_store.observe(memory, base, state, country, changes, scratch);
            if visits.len() < visits.capacity() || engine_allocation::reserve(visits, 1).is_ok() {
                visits.push(visit);
            } else {
                lost_visit = true;
            }
            *country_ticks += qpc().saturating_sub(start).max(0) as u64;
        };
        let mut policy = Observed {
            held: held_store,
            core: core_store,
            changes,
            stamps: held::Stamps::default(),
            events: &mut self.events,
            slot: &slot,
            countries: 0,
            incomplete: false,
        };
        let walk = native::walk_icons_range(
            memory,
            base,
            state,
            slots,
            &mut self.rows,
            &mut self.bytes,
            &mut self.counts,
            &mut self.variants,
            native::DefinitionSlots {
                database: &mut self.database,
                globals: &mut self.globals,
                locals: &mut self.locals,
            },
            &mut self.names,
            &mut visit_country,
            &mut policy,
        );
        self.incomplete = policy.incomplete || lost_visit;
        self.walk = Some(walk);
        if let Outcome::Rows { len } = walk.outcome {
            let carried = self.events.iter().filter_map(|event| match event {
                Event::Demand(occurrence) => Some(occurrence.carried),
                _ => None,
            });
            for ((at, row), carried) in self.rows[..len].iter().enumerate().zip(carried) {
                if carried {
                    continue;
                }
                let start = self.speed_keys.len();
                let span = engine_native::carmy::movement_speed_keys(
                    memory,
                    base,
                    row.pointer,
                    &mut self.speed_keys,
                )
                .ok()
                .map(|()| (start, self.speed_keys.len() - start));
                if self.speed_spans.len() == self.speed_spans.capacity()
                    && engine_allocation::reserve(&mut self.speed_spans, 1).is_err()
                {
                    self.incomplete = true;
                    break;
                }
                self.speed_spans.push((at as u32, span));
            }
        }
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * size_of::<native::IconRow>()
            + self.bytes.capacity()
            + self.counts.capacity() * size_of::<native::SubunitCount>()
            + self.variants.capacity() * size_of::<u64>()
            + self.names.capacity()
            + self.database.capacity() * size_of::<usize>()
            + self.globals.capacity() * size_of::<native::GlobalSlot>()
            + self.locals.capacity() * size_of::<native::LocalSlot>()
            + self.events.capacity() * size_of::<Event>()
            + self.visits.capacity() * size_of::<countries::Visit>()
            + self.scratch.capacity() * size_of::<GarrisonTemplateSelection>()
            + self.speed_keys.capacity() * size_of::<u32>()
            + self.speed_spans.capacity() * size_of::<(u32, Option<(usize, usize)>)>()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn visited(&self) -> usize {
        self.events
            .iter()
            .filter(|event| matches!(event, Event::Demand(_)))
            .count()
    }

    pub(super) fn staged_speed_keys(&self) -> impl Iterator<Item = (usize, Option<&[u32]>)> + '_ {
        self.speed_spans.iter().map(|&(at, span)| {
            (
                self.row_offset + at as usize,
                span.map(|(start, len)| &self.speed_keys[start..start + len]),
            )
        })
    }

    fn demands(&self) -> impl Iterator<Item = &Occurrence> + '_ {
        self.events.iter().filter_map(|event| match event {
            Event::Demand(occurrence) => Some(occurrence),
            _ => None,
        })
    }

    fn failed(&self) -> bool {
        matches!(
            self.walk.map(|walk| walk.outcome),
            Some(Outcome::Unavailable { .. })
        )
    }
}

struct Observed<'a> {
    held: &'a held::Store,
    core: &'a super::core::Store,
    changes: &'a dyn Changes,
    stamps: held::Stamps,
    events: &'a mut Vec<Event>,
    slot: &'a std::cell::Cell<u32>,
    countries: u32,
    incomplete: bool,
}

impl Observed<'_> {
    fn record(&mut self, event: Event) {
        if self.events.len() < self.events.capacity()
            || engine_allocation::reserve(&mut *self.events, 1).is_ok()
        {
            self.events.push(event);
        } else {
            self.incomplete = true;
        }
    }
}

impl native::DetailPolicy for Observed<'_> {
    fn before_country(&mut self, country: u32, deployment: Option<usize>) {
        self.stamps.country(self.changes, country, deployment);
        self.countries += 1;
    }
    fn before_records(&mut self, templates: &[usize]) {
        self.stamps.records(self.changes, templates);
        self.record(Event::Batch);
    }
    fn demand(
        &mut self,
        country: u32,
        pointer: usize,
        template: u64,
        ordinal: u32,
    ) -> native::Acquire {
        let stamps = self.stamps.template(pointer, ordinal);
        let certificate = self.stamps.core(self.changes, pointer, ordinal);
        let details = self.held.decide(
            country,
            pointer,
            template,
            stamps,
            self.changes.fresh_only(),
        );
        let carried = !details
            && self
                .core
                .would_carry(template, pointer, country, certificate);
        self.record(Event::Demand(Occurrence {
            country,
            slot: self.slot.get(),
            visit: self.countries.saturating_sub(1),
            pointer,
            template,
            ordinal,
            stamps,
            certificate,
            deployment: self.stamps.deployment,
            details,
            carried,
        }));
        if carried {
            native::Acquire::Held
        } else {
            native::Acquire::Read { details }
        }
    }
    fn held(&mut self, template: u64) -> Option<native::IconCore<'_>> {
        self.core.core_of(template)
    }
    fn placed(&mut self, ends: [usize; 4]) {
        self.record(Event::Placed(ends));
    }
}

pub struct TemplateWorkers<'a> {
    pub lanes: usize,
    pub walk: &'a mut dyn FnMut(Vec<WalkPart>) -> Result<Vec<WalkPart>, &'static str>,
}

pub(super) struct Walked {
    pub walk: native::IconWalk,
    pub counts: [u64; 2],
    pub country_len: usize,
    pub country_failure: Option<&'static str>,
    pub country_ticks: u64,
}

struct Override {
    part: usize,
    row: usize,
    value: native::IconRow,
    carried: bool,
    at: [usize; 4],
    len: [usize; 4],
    replaced: [[usize; 4]; 2],
}

#[derive(Default)]
pub(super) struct Stitching {
    templates: Vec<u64>,
    decided: Vec<(Occurrence, bool, bool)>,
    overrides: Vec<Override>,
    bytes: Vec<u8>,
    counts: Vec<native::SubunitCount>,
    variants: Vec<u64>,
    names: Vec<u8>,
    read_rows: Vec<native::IconRow>,
    read_bytes: Vec<u8>,
    read_counts: Vec<native::SubunitCount>,
    read_variants: Vec<u64>,
    read_names: Vec<u8>,
}

impl Stitching {
    pub(super) fn allocated_bytes(&self) -> usize {
        self.templates.capacity() * size_of::<u64>()
            + self.decided.capacity() * size_of::<(Occurrence, bool, bool)>()
            + self.overrides.capacity() * size_of::<Override>()
            + self.bytes.capacity()
            + self.read_bytes.capacity()
            + (self.counts.capacity() + self.read_counts.capacity())
                * size_of::<native::SubunitCount>()
            + (self.variants.capacity() + self.read_variants.capacity()) * size_of::<u64>()
            + self.names.capacity()
            + self.read_names.capacity()
            + self.read_rows.capacity() * size_of::<native::IconRow>()
    }

    fn append(
        &mut self,
        bytes: &[u8],
        counts: &[native::SubunitCount],
        variants: &[u64],
        names: &[u8],
    ) -> Result<[usize; 4], &'static str> {
        let refused = |_| "template stitch allocation failed";
        engine_allocation::reserve(&mut self.bytes, bytes.len()).map_err(refused)?;
        engine_allocation::reserve(&mut self.counts, counts.len()).map_err(refused)?;
        engine_allocation::reserve(&mut self.variants, variants.len()).map_err(refused)?;
        engine_allocation::reserve(&mut self.names, names.len()).map_err(refused)?;
        engine_allocation::reserve(&mut self.overrides, 1).map_err(refused)?;
        let at = [
            self.bytes.len(),
            self.counts.len(),
            self.variants.len(),
            self.names.len(),
        ];
        self.bytes.extend_from_slice(bytes);
        self.counts.extend_from_slice(counts);
        self.variants.extend_from_slice(variants);
        self.names.extend_from_slice(names);
        Ok(at)
    }
}

fn occurrences(templates: &[u64], template: u64) -> usize {
    templates.partition_point(|&held| held <= template)
        - templates.partition_point(|&held| held < template)
}

impl Captured {
    #[cfg(any(test, feature = "test-support"))]
    pub fn taken_again(&self) -> usize {
        self.stitching.overrides.len()
    }

    pub(super) fn walk_split(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        changes: &dyn Changes,
        workers: &mut TemplateWorkers<'_>,
    ) -> Result<Walked, Fallback> {
        use collector_workers::hour_stages::{Part, record_part};
        if workers.lanes < 2 {
            return Err(Fallback::OneLane);
        }
        let census = qpc();
        let ranges = self
            .walk_ranges(memory, base, state, workers.lanes)
            .ok_or(Fallback::Census)?;
        let lent = self.lend().ok_or(Fallback::Lent)?;
        let mut parts = std::mem::take(&mut self.walk_parts);
        parts.truncate(ranges.len());
        let added = ranges.len() - parts.len();
        if engine_allocation::advisory(|| engine_allocation::reserve(&mut parts, added)).is_err() {
            self.walk_parts = parts;
            self.take_back(lent);
            return Err(Fallback::Room);
        }
        parts.resize_with(ranges.len(), WalkPart::default);
        for (part, slots) in parts.iter_mut().zip(ranges) {
            part.slots = slots;
            part.state = state;
            part.lent = Some(SharedBox::clone(&lent));
            engine_allocation::advisory(|| {
                collector_primitives::table_capture::grow_vec(
                    &mut part.database,
                    self.database.first.len(),
                );
                collector_primitives::table_capture::grow_vec(
                    &mut part.globals,
                    self.globals.first.len(),
                );
                collector_primitives::table_capture::grow_vec(
                    &mut part.locals,
                    self.locals.first.len(),
                );
            });
        }
        let count = parts.len();
        let dispatched = qpc();
        record_part(Part::TemplateCensus, census, dispatched);
        let returned = (workers.walk)(parts);
        let returned_at = qpc();
        record_part(Part::TemplateRanges, dispatched, returned_at);
        let mut parts = match returned {
            Ok(parts) => parts,
            Err(_) => {
                self.take_back(lent);
                return Err(Fallback::Dispatch);
            }
        };
        for part in &mut parts {
            part.lent = None;
        }
        if !self.take_back(lent) {
            self.walk_parts = parts;
            return Err(Fallback::Lent);
        }
        parts.sort_unstable_by_key(|part| part.slots.start);
        let complete = parts.len() == count
            && parts.iter().all(|part| part.walk.is_some() && !part.incomplete);
        let applied = parts
            .iter()
            .position(WalkPart::failed)
            .map_or(parts.len(), |failed| failed + 1);
        let walked = if complete {
            self.admit_decisions(&parts[..applied])
                .map(|()| self.stitch(memory, base, state, &mut parts[..applied], changes))
        } else {
            Err(Fallback::Incomplete)
        };
        record_part(Part::TemplateApply, returned_at, qpc());
        self.staged_parts = if walked
            .as_ref()
            .is_ok_and(|walked| matches!(walked.walk.outcome, Outcome::Rows { .. }))
        {
            applied
        } else {
            0
        };
        self.walk_parts = parts;
        walked
    }

    fn walk_ranges(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        lanes: usize,
    ) -> Option<Vec<Range<u32>>> {
        self.slot_weights.clear();
        let mut refused = false;
        let weights = &mut self.slot_weights;
        native::roster_lengths(memory, base, state, |slot, templates| {
            let weight = templates.map_or(0, |templates| u64::from(templates) + 1);
            if engine_allocation::advisory(|| engine_allocation::reserve(weights, 1)).is_ok() {
                weights.push((slot, weight, templates.unwrap_or(0)));
            } else {
                refused = true;
            }
        })
        .ok()?;
        let templates = self
            .slot_weights
            .iter()
            .map(|&(_, _, templates)| templates as usize)
            .sum::<usize>();
        let countries = self.slot_weights.iter().any(|&(_, weight, _)| weight != 0);
        if refused || !countries || templates < SPLIT_ROWS {
            return None;
        }
        let count = lanes.min(templates / PART_ROWS);
        let total = self.slot_weights.iter().map(|&(_, weight, _)| weight).sum::<u64>();
        let mut ranges =
            engine_allocation::advisory(|| engine_allocation::vec(count)).ok()?;
        let (mut start, mut sum) = (1_u32, 0_u64);
        for &(slot, weight, _) in &self.slot_weights {
            sum += weight;
            let index = ranges.len() as u64;
            if index + 1 < count as u64 && sum * count as u64 >= total * (index + 1) {
                ranges.push(start..slot + 1);
                start = slot + 1;
            }
        }
        ranges.push(start..u32::MAX);
        (ranges.len() >= 2).then_some(ranges)
    }

    fn lend(&mut self) -> Option<SharedBox<Lent>> {
        if self.lent.is_none() {
            self.lent = engine_allocation::advisory(|| engine_allocation::shared(Lent::default()))
                .ok();
        }
        let mut lent = self.lent.take()?;
        let Some(stores) = SharedBox::get_mut(&mut lent) else {
            return None;
        };
        std::mem::swap(&mut stores.held, &mut self.held);
        std::mem::swap(&mut stores.core, &mut self.core);
        std::mem::swap(&mut stores.countries, &mut self.country_cache);
        Some(lent)
    }

    fn take_back(&mut self, mut lent: SharedBox<Lent>) -> bool {
        let Some(stores) = SharedBox::get_mut(&mut lent) else {
            return false;
        };
        std::mem::swap(&mut stores.held, &mut self.held);
        std::mem::swap(&mut stores.core, &mut self.core);
        std::mem::swap(&mut stores.countries, &mut self.country_cache);
        self.lent = Some(lent);
        true
    }

    fn admit_decisions(&mut self, parts: &[WalkPart]) -> Result<(), Fallback> {
        let stitch = &mut self.stitching;
        stitch.templates.clear();
        let occurrences_of = |part: &WalkPart| part.demands().count();
        let total = parts.iter().map(occurrences_of).sum::<usize>();
        let most = parts.iter().map(occurrences_of).max().unwrap_or(0);
        engine_allocation::advisory(|| {
            engine_allocation::reserve(&mut stitch.templates, total)?;
            stitch.decided.clear();
            engine_allocation::reserve(&mut stitch.decided, most)
        })
        .map_err(|_| Fallback::Room)?;
        stitch.templates.extend(
            parts
                .iter()
                .flat_map(|part| part.demands().map(|o| o.template)),
        );
        stitch.templates.sort_unstable();
        if let Some(last) = parts.last().filter(|part| part.failed()) {
            let batch = last
                .events
                .iter()
                .rposition(|event| matches!(event, Event::Batch))
                .map_or(0, |at| at + 1);
            let repeated = last.events[batch..].iter().any(|event| {
                matches!(event, Event::Demand(o) if occurrences(&stitch.templates, o.template) > 1)
            });
            if repeated {
                return Err(Fallback::Redecided);
            }
        }
        self.held
            .reserve_new(&self.stitching.templates)
            .then_some(())
            .ok_or(Fallback::Room)
    }

    fn stitch(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        parts: &mut [WalkPart],
        changes: &dyn Changes,
    ) -> Walked {
        let fresh = changes.fresh_only();
        let mut stitch = std::mem::take(&mut self.stitching);
        stitch.overrides.clear();
        stitch.bytes.clear();
        stitch.counts.clear();
        stitch.variants.clear();
        stitch.names.clear();
        let mut counts = [0_u64; 2];
        let mut country_len = 0;
        let mut country_failure = None;
        let mut country_ticks = 0;
        let mut failure = None;
        for (index, part) in parts.iter_mut().enumerate() {
            stitch.decided.clear();
            let mut row = 0;
            let mut ends = [0_usize; 4];
            let mut visits = part.visits.len();
            for event in &part.events {
                match *event {
                    Event::Batch => {}
                    Event::Demand(o) => {
                        let details = self.held.request_certified(
                            o.country, o.pointer, o.template, o.ordinal, o.stamps, fresh,
                        );
                        let kept =
                            !details && self.core.carry(o.template, o.pointer, o.country, o.certificate);
                        counts[usize::from(!kept)] += 1;
                        stitch.decided.push((o, details, kept));
                    }
                    Event::Placed(end) => {
                        let Some(&(o, details, kept)) = stitch.decided.get(row) else {
                            failure = Some("template walk range rows unmatched");
                            break;
                        };
                        let replaced = [ends, end];
                        ends = end;
                        let taken = if kept {
                            if o.carried && !self.core.renewed(o.template) {
                                Ok(())
                            } else {
                                self.hold_again(&mut stitch, index, row, &o, replaced)
                            }
                        } else if !o.carried && o.details == details {
                            if let Some(value) = part.rows.get(row) {
                                self.core.remember(
                                    changes,
                                    o.certificate,
                                    o.deployment,
                                    value,
                                    &part.bytes,
                                    &part.counts,
                                    &part.variants,
                                    &part.names,
                                );
                            }
                            Ok(())
                        } else {
                            self.read_again(
                                memory, base, state, &mut stitch, changes, index, row, &o, details,
                                replaced,
                            )
                        };
                        if let Err(reason) = taken {
                            failure = Some(reason);
                            visits = visits.min(o.visit as usize + 1);
                            break;
                        }
                        row += 1;
                    }
                }
            }
            country_ticks += part.country_ticks;
            for visit in part.visits.drain(..visits.min(part.visits.len())) {
                if country_failure.is_some() {
                    break;
                }
                let garrison = visit.garrison_failure();
                let committed = self.country_cache.commit(visit).and_then(|row| {
                    if let Some(reason) = garrison {
                        self.model_failures.add(reason);
                    }
                    engine_native_memory::walk::Buffer::Growing(&mut self.country_rows.rows)
                        .ensure(country_len + 1)?;
                    self.country_rows.rows[country_len] = row;
                    country_len += 1;
                    Ok(())
                });
                country_failure = committed.err();
            }
            if failure.is_some() {
                break;
            }
        }
        let mut walk = self.join(parts, &stitch);
        if let Some(reason) = failure {
            walk.outcome = Outcome::Unavailable { reason };
        }
        self.stitching = stitch;
        Walked {
            walk,
            counts,
            country_len,
            country_failure,
            country_ticks,
        }
    }

    fn hold_again(
        &self,
        stitch: &mut Stitching,
        part: usize,
        row: usize,
        o: &Occurrence,
        replaced: [[usize; 4]; 2],
    ) -> Result<(), &'static str> {
        let core = self
            .core
            .core_of(o.template)
            .ok_or("template held row unavailable")?;
        let at = stitch.append(core.bytes, core.counts, core.variants, core.names)?;
        let mut value = core.row;
        value.settings.roster_ordinal = o.ordinal;
        stitch.overrides.push(Override {
            part,
            row,
            value,
            carried: true,
            at,
            len: [
                core.bytes.len(),
                core.counts.len(),
                core.variants.len(),
                core.names.len(),
            ],
            replaced,
        });
        Ok(())
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the walk's context, the stitch's storage and the occurrence"
    )]
    fn read_again(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        stitch: &mut Stitching,
        changes: &dyn Changes,
        part: usize,
        row: usize,
        o: &Occurrence,
        details: bool,
        replaced: [[usize; 4]; 2],
    ) -> Result<(), &'static str> {
        let walk = native::read_icon_occurrence(
            memory,
            base,
            state,
            o.slot,
            o.ordinal,
            details,
            &mut stitch.read_rows,
            &mut stitch.read_bytes,
            &mut stitch.read_counts,
            &mut stitch.read_variants,
            &mut stitch.read_names,
        );
        match walk.outcome {
            Outcome::Rows { len: 1 } => {}
            Outcome::Unavailable { reason } => return Err(reason),
            _ => return Err("template occurrence read incomplete"),
        }
        let value = stitch.read_rows[0];
        let len = [
            walk.bytes_len,
            walk.counts_len,
            walk.variants_len,
            walk.name_bytes_len,
        ];
        self.core.remember(
            changes,
            o.certificate,
            o.deployment,
            &value,
            &stitch.read_bytes[..len[0]],
            &stitch.read_counts[..len[1]],
            &stitch.read_variants[..len[2]],
            &stitch.read_names[..len[3]],
        );
        let (bytes, counts, variants, names) = (
            std::mem::take(&mut stitch.read_bytes),
            std::mem::take(&mut stitch.read_counts),
            std::mem::take(&mut stitch.read_variants),
            std::mem::take(&mut stitch.read_names),
        );
        let at = stitch.append(
            &bytes[..len[0]],
            &counts[..len[1]],
            &variants[..len[2]],
            &names[..len[3]],
        );
        (
            stitch.read_bytes,
            stitch.read_counts,
            stitch.read_variants,
            stitch.read_names,
        ) = (bytes, counts, variants, names);
        stitch.overrides.push(Override {
            part,
            row,
            value,
            carried: false,
            at: at?,
            len,
            replaced,
        });
        Ok(())
    }

    fn join(&mut self, parts: &mut [WalkPart], stitch: &Stitching) -> native::IconWalk {
        let mut joined = native::IconWalk {
            outcome: Outcome::Rows { len: 0 },
            name_bytes_len: 0,
            required_name_bytes: 0,
            bytes_len: 0,
            required_bytes: 0,
            counts_len: 0,
            required_counts: 0,
            variants_len: 0,
            required_variants: 0,
            required_database: 0,
            required_locals: 0,
        };
        let mut failure = None;
        let mut len = 0;
        for (index, part) in parts.iter_mut().enumerate() {
            let Some(walk) = part.walk else {
                continue;
            };
            joined.required_name_bytes += walk.required_name_bytes;
            joined.required_bytes += walk.required_bytes;
            joined.required_counts += walk.required_counts;
            joined.required_variants += walk.required_variants;
            joined.required_database = joined.required_database.max(walk.required_database);
            joined.required_locals = joined.required_locals.max(walk.required_locals);
            let rows = match walk.outcome {
                Outcome::Unavailable { reason } => {
                    failure = Some(reason);
                    0
                }
                outcome => outcome.len(),
            };
            let own = if rows == 0 {
                &[][..]
            } else {
                let first = stitch.overrides.partition_point(|o| o.part < index);
                let end = stitch.overrides.partition_point(|o| o.part <= index);
                &stitch.overrides[first..end]
            };
            let part_len = [
                walk.bytes_len,
                walk.counts_len,
                walk.variants_len,
                walk.name_bytes_len,
            ];
            let mut extra = [0_usize; 4];
            for o in own {
                for (extra, len) in extra.iter_mut().zip(o.len) {
                    *extra += len;
                }
            }
            let at = [
                joined.bytes_len,
                joined.counts_len,
                joined.variants_len,
                joined.name_bytes_len,
            ];
            let grown = (|| {
                engine_native_memory::walk::Buffer::Growing(&mut self.icon_rows.rows).ensure(len + rows)?;
                engine_native_memory::walk::Buffer::Growing(&mut self.icon_bytes.first)
                    .ensure(at[0] + part_len[0] + extra[0])?;
                engine_native_memory::walk::Buffer::Growing(&mut self.counts.first)
                    .ensure(at[1] + part_len[1] + extra[1])?;
                engine_native_memory::walk::Buffer::Growing(&mut self.variants.first)
                    .ensure(at[2] + part_len[2] + extra[2])?;
                engine_native_memory::walk::Buffer::Growing(&mut self.name_bytes.first)
                    .ensure(at[3] + part_len[3] + extra[3])?;
                Ok::<(), &'static str>(())
            })();
            if let Err(reason) = grown {
                failure = Some(reason);
                break;
            }
            part.row_offset = len;
            let mut out = at;
            let mut cursor = [0_usize; 4];
            let mut next = 0;
            let mut carried = part.demands().map(|o| o.carried);
            let place = |this: &mut Self,
                         carried: &mut dyn Iterator<Item = bool>,
                         from: [usize; 4],
                         to: [usize; 4],
                         out: &mut [usize; 4],
                         rows: Range<usize>| {
                this.icon_bytes.first[out[0]..out[0] + to[0] - from[0]]
                    .copy_from_slice(&part.bytes[from[0]..to[0]]);
                this.counts.first[out[1]..out[1] + to[1] - from[1]]
                    .copy_from_slice(&part.counts[from[1]..to[1]]);
                this.variants.first[out[2]..out[2] + to[2] - from[2]]
                    .copy_from_slice(&part.variants[from[2]..to[2]]);
                this.name_bytes.first[out[3]..out[3] + to[3] - from[3]]
                    .copy_from_slice(&part.names[from[3]..to[3]]);
                let shift = std::array::from_fn(|i| out[i].wrapping_sub(from[i]));
                for row in rows {
                    let held = carried.next().unwrap_or(false);
                    this.icon_rows.rows[len + row] = shifted(part.rows[row], shift, held);
                }
                for i in 0..4 {
                    out[i] += to[i] - from[i];
                }
            };
            for o in own {
                let [start, end] = o.replaced;
                place(self, &mut carried, cursor, start, &mut out, next..o.row);
                self.icon_bytes.first[out[0]..out[0] + o.len[0]]
                    .copy_from_slice(&stitch.bytes[o.at[0]..o.at[0] + o.len[0]]);
                self.counts.first[out[1]..out[1] + o.len[1]]
                    .copy_from_slice(&stitch.counts[o.at[1]..o.at[1] + o.len[1]]);
                self.variants.first[out[2]..out[2] + o.len[2]]
                    .copy_from_slice(&stitch.variants[o.at[2]..o.at[2] + o.len[2]]);
                self.name_bytes.first[out[3]..out[3] + o.len[3]]
                    .copy_from_slice(&stitch.names[o.at[3]..o.at[3] + o.len[3]]);
                carried.next();
                self.icon_rows.rows[len + o.row] = shifted(o.value, out, o.carried);
                for i in 0..4 {
                    out[i] += o.len[i];
                }
                cursor = end;
                next = o.row + 1;
            }
            place(self, &mut carried, cursor, part_len, &mut out, next..rows);
            len += rows;
            joined.bytes_len = out[0];
            joined.counts_len = out[1];
            joined.variants_len = out[2];
            joined.name_bytes_len = out[3];
            if failure.is_some() {
                break;
            }
        }
        joined.outcome = match failure {
            Some(reason) => Outcome::Unavailable { reason },
            None => Outcome::Rows { len },
        };
        joined
    }
}

fn shifted(
    mut row: native::IconRow,
    [bytes, counts, variants, names]: [usize; 4],
    carried: bool,
) -> native::IconRow {
    let shift = |span: &mut native::Span, by: usize| span.start = span.start.wrapping_add(by);
    shift(&mut row.priority_unit_name, bytes);
    shift(&mut row.model_entity, bytes);
    shift(&mut row.counts, counts);
    for grid in &mut row.grids {
        shift(grid, counts);
    }
    shift(&mut row.excluded_variants, variants);
    if let Some(name) = &mut row.display_name {
        shift(&mut name.literal, names);
        if let Some(key) = &mut name.fallback_key {
            shift(key, names);
        }
    }
    if let Some(key) = &mut row.name_key {
        shift(key, names);
    }
    if !carried {
        if let Some(group) = &mut row.names_group {
            shift(group, bytes);
        }
        if row.equipment_failure.is_none() {
            shift(&mut row.forbidden_equipment, variants);
        }
        if row.details_fresh {
            for cells in &mut row.cells {
                shift(cells, counts);
            }
        }
    }
    row
}
