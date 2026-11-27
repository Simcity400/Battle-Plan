use crate::{
    clock::qpc,
    country_input_changes::{self as changes, Changes, Table},
    slicing::{self, Changed, SliceFingerprints, SliceSelection},
    table_capture::{Arena, CaptureRows},
    table_conversion::{table, unavailable},
};
use engine_allocation::{SharedBox, copy_vec, owned_vec};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::ccountry;
use engine_stability::country_stability as native;
use replay_data::{
    cadence::SKIPPED_REASON, cadence::should_walk, rows::CountryStabilityInputRow,
    rows::CountryStabilityLineRow, rows::TableId, rows::tables,
};
use replay_frame::frame::TableView;
use std::sync::atomic::{AtomicU64, Ordering};

static BATCH_FALLBACKS: AtomicU64 = AtomicU64::new(0);

pub fn status() -> impl serde::Serialize {
    crate::status::status!({ "batch_fallbacks": BATCH_FALLBACKS.load(Ordering::Relaxed) })
}

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [
    TableId::CountryStabilityInputs,
    TableId::CountryStabilityLines,
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    inputs: usize,
    lines: usize,
    bytes: usize,
    demand: native::Demand,
}

collector_primitives::capacity_max!(Capacities {
    inputs,
    lines,
    bytes,
    demand { inputs, lines, bytes, children, candidates, values },
});
impl Capacities {
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(inputs: usize, lines: usize, bytes: usize) -> Self {
        Self {
            inputs,
            lines,
            bytes,
            demand: native::Demand::default(),
        }
    }

    pub fn raise_for_countries(&mut self, slots: usize) {
        let inputs = crate::country_scale::rows(slots, 1);
        self.inputs = self.inputs.max(inputs);
    }

    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let demand = native::demand(memory, base, state)?;
        self.inputs = self.inputs.max(demand.inputs);
        self.lines = self.lines.max(demand.lines);
        self.bytes = self.bytes.max(demand.bytes);
        self.demand.children = self.demand.children.max(demand.children);
        self.demand.candidates = self.demand.candidates.max(demand.candidates);
        self.demand.values = self.demand.values.max(demand.values);
        Ok(())
    }
}

pub struct StabilityDispatch<'a> {
    pub lanes: usize,
    pub acquire: &'a mut dyn FnMut(Vec<StabilityPart>) -> Result<Vec<StabilityPart>, &'static str>,
}

enum ContextSlot {
    Owned(native::Context),
    Shared(SharedBox<native::Context>),
}

impl std::ops::Deref for ContextSlot {
    type Target = native::Context;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Owned(context) => context,
            Self::Shared(context) => context,
        }
    }
}

pub struct StabilityPart {
    ordinal: usize,
    countries: Vec<(engine_native_memory::country_identity::Country, Option<u64>)>,
    context: Option<SharedBox<native::Context>>,
    memo: native::Memo,
    scratch: native::Scratch,
    inputs: Vec<native::InputRow>,
    lines: Vec<native::LineRow>,
    bytes: Vec<u8>,
    bytes_len: usize,
    bytes_required: usize,
    lines_outcome: Outcome,
    unread: Vec<u32>,
    carried: Vec<u32>,
    fingerprints: Vec<(u32, u64)>,
    error: Option<&'static str>,
    phase_ticks: [u64; 4],
    acquired: bool,
}

impl StabilityPart {
    fn new(demand: native::Demand) -> Self {
        Self {
            ordinal: 0,
            countries: Vec::new(),
            context: None,
            memo: native::Memo::default(),
            scratch: native::Scratch::with_demand(demand)
                .map_err(crate::allocation::io)
                .unwrap_or_else(|_| native::Scratch::vacant()),
            inputs: Vec::new(),
            lines: Vec::new(),
            bytes: Vec::new(),
            bytes_len: 0,
            bytes_required: 0,
            lines_outcome: Outcome::Rows { len: 0 },
            unread: Vec::new(),
            carried: Vec::new(),
            fingerprints: Vec::new(),
            error: None,
            phase_ticks: [0; 4],
            acquired: false,
        }
    }

    fn reset(&mut self) {
        self.countries.clear();
        self.context = None;
        self.memo.clear();
        self.inputs.clear();
        self.lines.clear();
        self.bytes_len = 0;
        self.bytes_required = 0;
        self.lines_outcome = Outcome::Rows { len: 0 };
        self.unread.clear();
        self.carried.clear();
        self.fingerprints.clear();
        self.error = None;
        self.phase_ticks = [0; 4];
        self.acquired = false;
    }

    fn allocated_bytes(&self) -> usize {
        self.countries.capacity()
            * size_of::<(engine_native_memory::country_identity::Country, Option<u64>)>()
            + self.memo.allocated_bytes()
            + self.scratch.allocated_bytes()
            + self.inputs.capacity() * size_of::<native::InputRow>()
            + self.lines.capacity() * size_of::<native::LineRow>()
            + self.bytes.capacity()
            + (self.unread.capacity() + self.carried.capacity()) * size_of::<u32>()
            + self.fingerprints.capacity() * size_of::<(u32, u64)>()
    }

    pub fn capture(&mut self, memory: &impl Memory, base: usize, game_state: usize) {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let started = qpc();
        let mut timing = native::WalkTiming::new(started);
        self.acquired = false;
        self.error = None;
        self.memo.clear();
        self.inputs.clear();
        self.lines.clear();
        self.bytes_len = 0;
        self.bytes_required = 0;
        self.lines_outcome = Outcome::Rows { len: 0 };
        self.unread.clear();
        self.carried.clear();
        self.fingerprints.clear();
        let Some(context) = self.context.as_deref() else {
            self.error = Some("stability partition missing context");
            self.phase_ticks = timing.finish_at(qpc());
            return;
        };
        let countries = match ccountry::Reader::new(memory, base, game_state) {
            Ok(countries) => countries,
            Err(reason) => {
                self.error = Some(reason);
                self.phase_ticks = timing.finish_at(qpc());
                return;
            }
        };
        if engine_allocation::reserve(&mut self.inputs, self.countries.len()).is_err() {
            self.error = Some("stability input storage allocation failed");
            self.phase_ticks = timing.finish_at(qpc());
            return;
        }
        let mut lines = Outcome::Rows { len: 0 };
        for &(country, expected_line_fp) in &self.countries {
            let mut input = native::InputRow::default();
            let start = lines.len();
            let walk = native::walk_country_memo_timed_or_carry(
                memory,
                base,
                &countries,
                country,
                context,
                &mut self.memo,
                &mut self.scratch,
                &mut input,
                &mut self.lines,
                &mut self.bytes,
                start,
                self.bytes_len,
                expected_line_fp,
                &mut timing,
            );
            let fallbacks = self.scratch.batch_fallbacks();
            if fallbacks != 0 {
                BATCH_FALLBACKS.fetch_add(fallbacks, Ordering::Relaxed);
            }
            let carry = match walk {
                Ok(carry) => carry,
                Err(reason) => {
                    self.error = Some(reason);
                    self.phase_ticks = timing.finish_at(qpc());
                    return;
                }
            };
            if carry.carried_lines {
                if self.carried.len() == self.carried.capacity()
                    && engine_allocation::reserve(&mut self.carried, 1).is_err()
                {
                    self.error = Some("stability carried owner allocation failed");
                    self.phase_ticks = timing.finish_at(qpc());
                    return;
                }
                self.carried.push(country.id);
            } else {
                let walk = carry.walk;
                let offset = self.bytes_len;
                self.bytes_required = self
                    .bytes_required
                    .max(offset.saturating_add(walk.required_bytes));
                self.bytes_len += walk.bytes_len;
                lines = match combine(lines, walk.lines) {
                    Ok(lines) => lines,
                    Err(reason) => {
                        self.error = Some(reason);
                        self.phase_ticks = timing.finish_at(qpc());
                        return;
                    }
                };
                if matches!(walk.lines, Outcome::Rows { .. }) {
                    if self.fingerprints.len() == self.fingerprints.capacity()
                        && engine_allocation::reserve(&mut self.fingerprints, 1).is_err()
                    {
                        self.error = Some("stability line fingerprint allocation failed");
                        self.phase_ticks = timing.finish_at(qpc());
                        return;
                    }
                    self.fingerprints.push((country.id, carry.line_fingerprint));
                } else {
                    if self.unread.len() == self.unread.capacity()
                        && engine_allocation::reserve(&mut self.unread, 1).is_err()
                    {
                        self.error = Some("stability unread allocation failed");
                        self.phase_ticks = timing.finish_at(qpc());
                        return;
                    }
                    self.unread.push(country.id);
                }
            }
            self.inputs.push(input);
        }
        self.lines_outcome = lines;
        self.phase_ticks = timing.finish_at(qpc());
        self.acquired = true;
    }
}

pub struct Captured {
    inputs: CaptureRows<native::InputRow>,
    lines: CaptureRows<native::LineRow>,
    bytes: Arena<u8>,
    scratch: native::Scratch,
    memo: native::Memo,
    selection: SliceSelection,
    changed: Changed,
    lines_changed: Changed,
    lines_carried: Vec<u32>,
    keys: Option<native::Keys>,
    context: Option<ContextSlot>,
    phase_ticks: [u64; 4],
    demand: native::Demand,
    parts: Vec<StabilityPart>,
    selected_countries: Vec<(engine_native_memory::country_identity::Country, Option<u64>)>,
}

impl Captured {
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.inputs
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
        self.lines
            .fail(super::failed_capture(TABLES[1], hour, force_full, reason));
        self.selection.reset(hour, force_full);
        self.lines_carried.clear();
        self.phase_ticks = [0; 4];
    }
    pub fn vacant() -> Self {
        Self {
            inputs: CaptureRows::new(0),
            lines: CaptureRows::new(0),
            bytes: Arena::new(0),
            scratch: native::Scratch::vacant(),
            memo: native::Memo::default(),
            selection: SliceSelection::new(),
            changed: Changed::default(),
            lines_changed: Changed::default(),
            lines_carried: Vec::new(),
            keys: None,
            context: None,
            phase_ticks: [0; 4],
            demand: native::Demand::default(),
            parts: Vec::new(),
            selected_countries: Vec::new(),
        }
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            inputs: CaptureRows::new(capacity.inputs),
            lines: CaptureRows::new(capacity.lines),
            bytes: Arena::new(capacity.bytes),
            scratch: native::Scratch::with_demand(capacity.demand)
                .map_err(crate::allocation::io)
                .unwrap_or_else(|_| native::Scratch::vacant()),
            memo: native::Memo::default(),
            selection: SliceSelection::new(),
            changed: Changed::default(),
            lines_changed: Changed::default(),
            lines_carried: Vec::new(),
            keys: None,
            context: None,
            phase_ticks: [0; 4],
            demand: capacity.demand,
            parts: Vec::new(),
            selected_countries: Vec::new(),
        }
    }
    pub fn reset(&mut self) {
        self.inputs.reset();
        self.lines.reset();
        self.bytes.reset();
        self.lines_carried.clear();
        self.context = None;
        self.memo.clear();
        self.phase_ticks = [0; 4];
    }
    pub fn allocated_bytes(&self) -> usize {
        self.inputs.allocated_bytes()
            + self.lines.allocated_bytes()
            + self.bytes.allocated_bytes()
            + self.scratch.allocated_bytes()
            + self.memo.allocated_bytes()
            + self.lines_carried.capacity() * size_of::<u32>()
            + self.context.as_ref().map_or(0, |context| {
                context.language.capacity()
                    + usize::from(matches!(context, ContextSlot::Shared(_)))
                        * SharedBox::<native::Context>::BYTES
            })
            + self
                .parts
                .iter()
                .map(StabilityPart::allocated_bytes)
                .sum::<usize>()
            + self.selected_countries.capacity()
                * size_of::<(engine_native_memory::country_identity::Country, Option<u64>)>()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.inputs.grow(&mut capacity.inputs);
        self.lines.grow(&mut capacity.lines);
        self.bytes.grow(&mut capacity.bytes);
    }

    pub fn selection(&self) -> &SliceSelection {
        &self.selection
    }

    pub fn lines_carried_owners(&self) -> Option<&[u32]> {
        (!self.lines_carried.is_empty()).then_some(self.lines_carried.as_slice())
    }

    pub fn phase_ticks(&self) -> [u64; 4] {
        self.phase_ticks
    }

    pub fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        self.selection.reset(hour, force_full);
        self.lines_carried.clear();
        self.phase_ticks = [0; 4];
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_keys(&mut self, keys: native::Keys) {
        self.keys = Some(keys);
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        fingerprints: Option<&mut SliceFingerprints>,
    ) -> [u64; N] {
        self.capture_changed_partitioned(
            memory,
            base,
            game_state,
            hour,
            force_full,
            fingerprints,
            &changes::Observer,
            None,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the hourly capture operands travel beside fingerprints and worker dispatch"
    )]
    pub fn capture_partitioned(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        fingerprints: Option<&mut SliceFingerprints>,
        dispatch: Option<StabilityDispatch<'_>>,
    ) -> [u64; N] {
        self.capture_changed_partitioned(
            memory,
            base,
            game_state,
            hour,
            force_full,
            fingerprints,
            &changes::Observer,
            dispatch,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the hourly capture operands (memory, base, game state, hour, full flag) travel beside the fingerprints and change source"
    )]
    pub fn capture_changed(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        fingerprints: Option<&mut SliceFingerprints>,
        changes: &dyn Changes,
    ) -> [u64; N] {
        self.capture_changed_partitioned(
            memory,
            base,
            game_state,
            hour,
            force_full,
            fingerprints,
            changes,
            None,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the hourly capture operands travel beside fingerprints, change source and worker dispatch"
    )]
    pub fn capture_changed_partitioned(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        mut fingerprints: Option<&mut SliceFingerprints>,
        changes: &dyn Changes,
        dispatch: Option<StabilityDispatch<'_>>,
    ) -> [u64; N] {
        self.reset();
        self.selection.reset(hour, force_full);
        if !should_walk(TABLES[0], hour, force_full) {
            for rows in [&mut self.inputs.outcome, &mut self.lines.outcome] {
                *rows = Outcome::Unavailable {
                    reason: SKIPPED_REASON,
                };
            }
            return [0; N];
        }
        let _reads = crate::read_accounting::table(TABLES[0]);
        let started = qpc();
        let mut timing = native::WalkTiming::new(started);
        let result = self.walk(
            memory,
            base,
            game_state,
            fingerprints.as_deref_mut(),
            changes,
            dispatch,
            &mut timing,
        );
        if let Err(reason) = result {
            self.lines_carried.clear();
            if let Some(fingerprints) = fingerprints {
                fingerprints.forget_all_secondary();
            }
            self.inputs.outcome = Outcome::Unavailable { reason };
            self.lines.outcome = Outcome::Unavailable { reason };
        }
        let ended = qpc();
        let serial_ticks = timing.finish_at(ended);
        for (dst, src) in self.phase_ticks.iter_mut().zip(serial_ticks) {
            *dst = dst.saturating_add(src);
        }
        [ended.saturating_sub(started).max(0) as u64, 0]
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "walk carries memory, game state, fingerprints, change source, worker dispatch and timing"
    )]
    fn walk(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        mut fingerprints: Option<&mut SliceFingerprints>,
        changes: &dyn Changes,
        dispatch: Option<StabilityDispatch<'_>>,
        timing: &mut native::WalkTiming,
    ) -> Result<(), &'static str> {
        let cached = match self.keys {
            Some(keys) => keys.current(memory, base)?.then_some(keys),
            None => None,
        };
        let keys = match cached {
            Some(keys) => keys,
            None => {
                self.keys = None;
                let keys = native::Keys::resolve(memory, base)?;
                self.keys = Some(keys);
                keys
            }
        };
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let mut context = native::Context::read(memory, base, game_state, keys)?;
        let mut manpower_observed =
            fingerprints.is_none() && cfg!(any(test, feature = "test-support"));
        self.changed.reset();
        self.lines_changed.reset();
        let mut ids = None;
        if let Some(fingerprints) = fingerprints.as_deref_mut() {
            let started = qpc();
            let mut census = true;
            let mut bound = 0_usize;
            for slot in 1..countries.count() {
                match countries.at(slot) {
                    Ok(Some(country)) => {
                        bound = bound.max(country.id as usize + 1);
                        self.selection.scan(fingerprints, country.id, Some(0));
                    }
                    Ok(None) => {}
                    Err(_) => census = false,
                }
            }
            fingerprints.finish_scan();
            if census {
                self.selection.finish_census(fingerprints);
                ids = Some(bound.max(countries.arrays()[1].count as usize));
            }
            let digest = context.digest();
            slicing::select_changed(
                &mut self.selection,
                fingerprints,
                &mut self.changed,
                |visit| {
                    let epoch = changes.take(Table::Stability, visit);
                    manpower_observed = epoch.is_some();
                    epoch
                },
                digest,
            );
            fingerprints.certify_secondary(
                &mut self.lines_changed,
                |visit| changes.take(Table::StabilityLines, visit),
                digest,
            );
            self.selection.scan_ticks = qpc().saturating_sub(started).max(0) as u64;
        }
        context.manpower_enabled = manpower_observed;
        self.selected_countries.clear();
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            if !self.selection.contains(country.id) {
                continue;
            }
            let expected_line_fp = (!self.selection.force_full
                && self.lines_changed.carries(country.id))
            .then(|| {
                fingerprints
                    .as_deref()
                    .and_then(|fp| fp.secondary_held(country.id))
            })
            .flatten();
            if self.selected_countries.len() == self.selected_countries.capacity() {
                engine_allocation::reserve(&mut self.selected_countries, 1)
                    .map_err(|_| "stability country selection allocation failed")?;
            }
            self.selected_countries.push((country, expected_line_fp));
        }
        let full_walks = self
            .selected_countries
            .iter()
            .filter(|(_, fp)| fp.is_none())
            .count();
        if let Some(dispatch) = dispatch
            .filter(|d| d.lanes > 1 && (full_walks >= 4 || self.selected_countries.len() >= 32))
        {
            let context = engine_allocation::shared(context)
                .map_err(|_| "stability context allocation failed")?;
            let count = dispatch.lanes.min(self.selected_countries.len() / 2);
            let width = self.selected_countries.len().div_ceil(count);
            let prepared = engine_allocation::advisory(|| {
                let mut parts = engine_allocation::vec(count).ok()?;
                for (ordinal, chunk) in self.selected_countries.chunks(width).enumerate() {
                    let mut part = self
                        .parts
                        .pop()
                        .unwrap_or_else(|| StabilityPart::new(self.demand));
                    part.reset();
                    part.ordinal = ordinal;
                    part.context = Some(SharedBox::clone(&context));
                    engine_allocation::reserve(&mut part.countries, chunk.len()).ok()?;
                    part.countries.extend_from_slice(chunk);
                    parts.push(part);
                }
                Some(parts)
            });
            if let Some(parts) = prepared {
                let expected = parts.len();
                let mut returned = (dispatch.acquire)(parts)?;
                returned.sort_unstable_by_key(|part| part.ordinal);
                let result = (|| -> Result<(), &'static str> {
                    if returned.len() != expected
                        || returned
                            .iter()
                            .enumerate()
                            .any(|(i, part)| part.ordinal != i || !part.acquired)
                    {
                        if let Some(reason) = returned.iter().find_map(|part| part.error) {
                            return Err(reason);
                        }
                        return Err("stability partition acquisition incomplete");
                    }
                    if let Some(reason) = returned.iter().find_map(|part| part.error) {
                        return Err(reason);
                    }
                    let total_inputs: usize = returned.iter().map(|part| part.inputs.len()).sum();
                    let total_lines: usize =
                        returned.iter().map(|part| part.lines_outcome.len()).sum();
                    let total_bytes: usize = returned.iter().map(|part| part.bytes_len).sum();
                    let total_carried: usize =
                        returned.iter().map(|part| part.carried.len()).sum();
                    if self.inputs.rows.len() < total_inputs {
                        engine_allocation::resize_default(&mut self.inputs.rows, total_inputs)
                            .map_err(|_| "stability input storage allocation failed")?;
                    }
                    if self.lines.rows.len() < total_lines {
                        engine_allocation::resize_default(&mut self.lines.rows, total_lines)
                            .map_err(|_| "stability line storage allocation failed")?;
                    }
                    if self.bytes.first.len() < total_bytes {
                        engine_allocation::resize_default(&mut self.bytes.first, total_bytes)
                            .map_err(|_| "stability byte storage allocation failed")?;
                    }
                    if total_carried != 0 {
                        engine_allocation::reserve(&mut self.lines_carried, total_carried)
                            .map_err(|_| "stability carried owner allocation failed")?;
                    }
                    let mut input_at = 0_usize;
                    let mut line_at = 0_usize;
                    let mut byte_at = 0_usize;
                    let mut lines = Outcome::Rows { len: 0 };
                    for part in &returned {
                        let n_inputs = part.inputs.len();
                        self.inputs.rows[input_at..input_at + n_inputs]
                            .copy_from_slice(&part.inputs);
                        input_at += n_inputs;
                        let n_lines = part.lines_outcome.len();
                        for (dst, src) in self.lines.rows[line_at..line_at + n_lines]
                            .iter_mut()
                            .zip(&part.lines[..n_lines])
                        {
                            *dst = native::LineRow {
                                identity: native::Span {
                                    start: src.identity.start + byte_at,
                                    len: src.identity.len,
                                },
                                text: native::Span {
                                    start: src.text.start + byte_at,
                                    len: src.text.len,
                                },
                                ..*src
                            };
                        }
                        line_at += n_lines;
                        self.bytes.first[byte_at..byte_at + part.bytes_len]
                            .copy_from_slice(&part.bytes[..part.bytes_len]);
                        self.bytes.required = self
                            .bytes
                            .required
                            .max(byte_at.saturating_add(part.bytes_required));
                        byte_at += part.bytes_len;
                        lines = combine(lines, part.lines_outcome)?;
                        self.lines_carried.extend_from_slice(&part.carried);
                        if let Some(fingerprints) = fingerprints.as_deref_mut() {
                            for &(id, fp) in &part.fingerprints {
                                fingerprints.hold_secondary(id, fp);
                            }
                            for &id in &part.unread {
                                self.changed.unread(fingerprints, id);
                                fingerprints.forget_secondary(id);
                            }
                        }
                        for (dst, src) in self.phase_ticks.iter_mut().zip(part.phase_ticks) {
                            *dst = dst.saturating_add(src);
                        }
                    }
                    self.lines_carried.sort_unstable();
                    self.bytes.len = byte_at;
                    self.inputs.outcome = Outcome::Rows { len: input_at };
                    self.lines.outcome = lines;
                    self.context = Some(ContextSlot::Shared(context));
                    if fingerprints.is_some() {
                        let carried = self.lines_carried.len() as u64;
                        changes::record(
                            Table::Stability,
                            &changes::Counts {
                                marked: self.changed.marked() as u64,
                                read: input_at as u64,
                                full: !self.changed.certified,
                            },
                            ids,
                        );
                        changes::record(
                            Table::StabilityLines,
                            &changes::Counts {
                                marked: self.lines_changed.marked() as u64,
                                read: (input_at as u64).saturating_sub(carried),
                                full: !self.lines_changed.certified || self.selection.force_full,
                            },
                            None,
                        );
                        changes::record_carried(Table::StabilityLines, carried);
                    }
                    Ok(())
                })();
                for mut part in returned {
                    part.reset();
                    if self.parts.len() < self.parts.capacity()
                        || engine_allocation::reserve(&mut self.parts, 1).is_ok()
                    {
                        self.parts.push(part);
                    }
                }
                return result;
            }
            return self.walk_serial(
                memory,
                base,
                &countries,
                ContextSlot::Shared(context),
                fingerprints,
                ids,
                timing,
            );
        }
        self.walk_serial(
            memory,
            base,
            &countries,
            ContextSlot::Owned(context),
            fingerprints,
            ids,
            timing,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "serial stability walk receives memory, country reader, context slot, fingerprints and timing"
    )]
    fn walk_serial<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        countries: &ccountry::Reader<'_, M>,
        context: ContextSlot,
        mut fingerprints: Option<&mut SliceFingerprints>,
        ids: Option<usize>,
        timing: &mut native::WalkTiming,
    ) -> Result<(), &'static str> {
        let mut read = 0_u64;
        let mut inputs = 0;
        let mut lines = Outcome::Rows { len: 0 };
        for index in 0..self.selected_countries.len() {
            let (country, expected_line_fp) = self.selected_countries[index];
            let mut input = native::InputRow::default();
            let start = lines.len();
            let walk = native::walk_country_memo_timed_or_carry(
                memory,
                base,
                countries,
                country,
                &context,
                &mut self.memo,
                &mut self.scratch,
                &mut input,
                &mut self.lines.rows,
                &mut self.bytes.first,
                start,
                self.bytes.len,
                expected_line_fp,
                timing,
            );
            let fallbacks = self.scratch.batch_fallbacks();
            if fallbacks != 0 {
                BATCH_FALLBACKS.fetch_add(fallbacks, Ordering::Relaxed);
            }
            let carry = walk?;
            if carry.carried_lines {
                if self.lines_carried.len() == self.lines_carried.capacity() {
                    engine_allocation::reserve(&mut self.lines_carried, 1)
                        .map_err(|_| "stability carried owner allocation failed")?;
                }
                self.lines_carried.push(country.id);
            } else {
                let walk = carry.walk;
                let offset = self.bytes.len;
                self.bytes.required = self
                    .bytes
                    .required
                    .max(offset.saturating_add(walk.required_bytes));
                self.bytes.len += walk.bytes_len;
                lines = combine(lines, walk.lines)?;
                if let Some(fingerprints) = fingerprints.as_deref_mut() {
                    if matches!(walk.lines, Outcome::Rows { .. }) {
                        fingerprints.hold_secondary(country.id, carry.line_fingerprint);
                    } else {
                        self.changed.unread(fingerprints, country.id);
                        fingerprints.forget_secondary(country.id);
                    }
                }
            }
            read += 1;
            if self.inputs.rows.len() <= inputs {
                engine_allocation::resize_default(
                    &mut self.inputs.rows,
                    inputs
                        .checked_add(1)
                        .ok_or("stability input count overflow")?,
                )
                .map_err(|_| "stability input storage allocation failed")?;
            }
            self.inputs.rows[inputs] = input;
            inputs += 1;
        }
        self.lines_carried.sort_unstable();
        self.inputs.outcome = Outcome::Rows { len: inputs };
        self.lines.outcome = lines;
        self.context = Some(context);
        if fingerprints.is_some() {
            let carried = self.lines_carried.len() as u64;
            changes::record(
                Table::Stability,
                &changes::Counts {
                    marked: self.changed.marked() as u64,
                    read,
                    full: !self.changed.certified,
                },
                ids,
            );
            changes::record(
                Table::StabilityLines,
                &changes::Counts {
                    marked: self.lines_changed.marked() as u64,
                    read: read.saturating_sub(carried),
                    full: !self.lines_changed.certified || self.selection.force_full,
                },
                None,
            );
            changes::record_carried(Table::StabilityLines, carried);
        }
        Ok(())
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [self.inputs.outcome, self.lines.outcome]
    }

    pub fn tables(&self) -> [TableView; N] {
        let Some(context) = self.context.as_ref() else {
            return self.outcomes().map(|outcome| match outcome {
                Outcome::Unavailable { reason } => unavailable(reason),
                _ => unavailable("stability rows without their read's globals"),
            });
        };
        let keys = context.keys.keys.map(|key| key.unwrap_or(u32::MAX));
        let language = context.language.as_slice();
        let d = context.defines;
        let inputs = table::<tables::CountryStabilityInputs, _>(
            self.inputs.outcome,
            &self.inputs.rows,
            |row| {
                let t = row.totals;
                CountryStabilityInputRow {
                    country: row.country,
                    recruitment_ratio: replay_data::rows::Recorded::Value(row.manpower.ratio),
                    exile_host: replay_data::rows::Recorded::Value(row.manpower.host),
                    exile_legitimacy: replay_data::rows::Recorded::Value(row.manpower.legitimacy),
                    exile_leaders: replay_data::rows::Recorded::Value(row.manpower.leaders),
                    exile_manpower: replay_data::rows::Recorded::Value(row.manpower.exile_pool),
                    exile_manpower_cap: replay_data::rows::Recorded::Value(row.manpower.exile_cap),
                    stability_raw: row.stability_raw,
                    war_support_raw: row.war_support_raw,
                    at_war: u8::from(row.at_war),
                    offensive_war: u8::from(row.offensive_war),
                    defensive_war: u8::from(row.defensive_war),
                    ruling_party_popularity_raw: row.ruling_party_popularity_raw,
                    propaganda_stability_q15: row.propaganda_stability_q15,
                    coastal_protection_stability_raw: row.coastal_protection_stability_raw,
                    bombing_war_support_q15: row.bombing_q15,
                    heroes_war_support_q15: row.heroes_q15,
                    convoys_war_support_q15: row.convoys_q15,
                    propaganda_war_support_q15: row.propaganda_war_support_q15,
                    interception_sum_raw: row.interception_sum_raw,
                    world_tension_raw: context.world_tension_raw,
                    base_stability_war_factor: d[0],
                    base_stability_party_popularity_factor: d[1],
                    min_stability: d[2],
                    max_stability: d[3],
                    war_support_offensive_war: d[4],
                    war_support_defensive_war: d[5],
                    war_support_tension_impact: d[6],
                    min_war_support: d[7],
                    max_war_support: d[8],
                    interception_war_support_scale_q15: d[9],
                    interception_bombing_war_support_impact: d[10],
                    base_max_command_power: d[11],
                    stability_factor: t[0],
                    party_popularity_stability_factor: t[1],
                    war_stability_factor: t[2],
                    offensive_war_stability_factor: t[3],
                    defensive_war_stability_factor: t[4],
                    war_support_factor: t[5],
                    stability_weekly: t[6],
                    war_support_weekly: t[7],
                    keys: owned_vec(copy_vec(&keys)),
                    children: row.children,
                    language: owned_vec(copy_vec(language)),
                }
            },
        );
        let Some(bytes) = self.bytes.first.get(..self.bytes.len) else {
            return [inputs, unavailable("stability line arena invalid")];
        };
        let text = |span: native::Span| {
            span.start
                .checked_add(span.len)
                .and_then(|end| bytes.get(span.start..end))
        };
        let Some(rows) = self.lines.rows.get(..self.lines.outcome.len()) else {
            return [inputs, unavailable("stability line row count invalid")];
        };
        if rows
            .iter()
            .any(|row| text(row.identity).is_none() || text(row.text).is_none())
        {
            return [
                inputs,
                unavailable("stability line strings outside captured arena"),
            ];
        }
        let lines = table::<tables::CountryStabilityLines, _>(self.lines.outcome, rows, |row| {
            CountryStabilityLineRow {
                country: row.country,
                line: row.line,
                key: row.key,
                kind: kind(row.kind),
                identity: owned_vec(copy_vec(text(row.identity).unwrap())),
                identity_id: row.identity_id,
                secondary: row.secondary,
                value: row.value,
                text: owned_vec(copy_vec(text(row.text).unwrap())),
            }
        });
        [inputs, lines]
    }
}

fn kind(kind: native::Kind) -> u8 {
    use native::Kind;
    match kind {
        Kind::Unpaired => CountryStabilityLineRow::UNPAIRED,
        Kind::Idea => CountryStabilityLineRow::IDEA,
        Kind::CountryLeader => CountryStabilityLineRow::COUNTRY_LEADER,
        Kind::Ideology => CountryStabilityLineRow::IDEOLOGY,
        Kind::Advisor => CountryStabilityLineRow::ADVISOR,
        Kind::Static => CountryStabilityLineRow::STATIC,
        Kind::StaticScaled => CountryStabilityLineRow::STATIC_SCALED,
        Kind::DynamicModifier => CountryStabilityLineRow::DYNAMIC_MODIFIER,
        Kind::DynamicModifierValues => CountryStabilityLineRow::DYNAMIC_MODIFIER_VALUES,
    }
}

fn combine(total: Outcome, next: Outcome) -> Result<Outcome, &'static str> {
    let unread = |outcome| match outcome {
        Outcome::Truncated { unread_rows, .. } => unread_rows,
        _ => 0,
    };
    let len = total.len() + next.len();
    Ok(
        match unread(total)
            .checked_add(unread(next))
            .ok_or("stability line count overflow")?
        {
            0 => Outcome::Rows { len },
            unread_rows => Outcome::Truncated { len, unread_rows },
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TableRows;

    #[test]
    fn stability_capture_counts_refused_fallbacks_and_resets_before_inputs() {
        use engine_native_politics::cpolitical_status;
        use engine_native::cgame_state;
        use engine_native_parts::{cidea, cideology_group};
        use engine_native_memory::cpdx_array::Array;
        use engine_stability::{cmodifier, cworld_threat};
        use std::{cell::Cell, collections::BTreeMap};

        struct World {
            bytes: BTreeMap<usize, u8>,
            fault: Cell<Option<usize>>,
        }
        impl World {
            fn word(&mut self, at: usize, value: u64) {
                self.bytes.extend(
                    value
                        .to_le_bytes()
                        .into_iter()
                        .enumerate()
                        .map(|(i, byte)| (at + i, byte)),
                );
            }
            fn array(&mut self, at: usize, data: usize, count: u32) {
                self.word(at, data as u64);
                self.word(at + 8, u64::from(count) | (u64::from(count) << 32));
            }
        }
        impl Memory for World {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                if self
                    .fault
                    .get()
                    .is_some_and(|fault| at <= fault && fault - at < out.len())
                {
                    return false;
                }
                for (i, byte) in out.iter_mut().enumerate() {
                    *byte = self.bytes.get(&(at + i)).copied().unwrap_or(0);
                }
                true
            }
        }
        const BASE: usize = 0x1_4000_0000;
        const STATE: usize = 0x1000;
        const COUNTRY: usize = 0x10000;
        const STATUS: usize = 0x20000;
        const PARTY: usize = 0x21000;
        const IDEA: usize = 0x22000;
        const GROUP: usize = 0x23000;
        const DIPLOMACY: usize = 0x24000;
        const STATS: usize = 0x25000;
        const RING: usize = 0x26000;
        const WORLD: usize = 0x27000;
        let mut memory = World {
            bytes: BTreeMap::new(),
            fault: Cell::new(None),
        };
        memory.array(STATE + cgame_state::COUNTRIES, 0x3000, 2);
        memory.array(STATE + cgame_state::COUNTRY_LINKS, 0x4000, 2);
        memory.word(0x3008, COUNTRY as u64);
        memory.word(0x4004, 1);
        memory.word(COUNTRY, (BASE + ccountry::VTABLE_RVA.0) as u64);
        memory.word(COUNTRY + ccountry::ID, 1);
        memory.word(COUNTRY + ccountry::DIPLOMACY_STATUS, DIPLOMACY as u64);
        memory.word(DIPLOMACY, (BASE + engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0) as u64);
        memory.word(DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::COUNTRY, COUNTRY as u64);
        memory.word(COUNTRY + ccountry::POLITICAL_STATUS, STATUS as u64);
        memory.word(STATUS, (BASE + engine_native_parts::cpolitical_status_layout::VTABLE_RVA.0) as u64);
        memory.word(STATUS + engine_native_parts::cpolitical_status_layout::RULING_PARTY, PARTY as u64);
        memory.word(PARTY, (BASE + engine_native_parts::cpolitical_party_layout::VTABLE_RVA.0) as u64);
        memory.word(PARTY + engine_native_parts::cpolitical_party_layout::IDEOLOGY_GROUP, GROUP as u64);
        memory.word(GROUP, (BASE + cideology_group::VTABLE_RVA.0) as u64);
        memory.word(
            GROUP
                + native::IDEOLOGY_GROUP_MODIFIER
                + cmodifier::DISPLAY_NAME
                + engine_native_memory::msvc_string::CAPACITY,
            15,
        );
        memory.array(STATUS + cpolitical_status::IDEAS, 0x5000, 1);
        memory.word(0x5000, IDEA as u64);
        memory.word(IDEA, (BASE + cidea::VTABLE_RVA.0) as u64);
        memory.word(COUNTRY + native::AIR_STATISTICS, STATS as u64);
        memory.word(STATS + 0x10, RING as u64);
        memory.word(STATE + cgame_state::WORLD_THREAT, WORLD as u64);
        memory.word(WORLD, (BASE + cworld_threat::VTABLE_RVA.0) as u64);
        memory.word(BASE + native::STATIC_DATABASE_RVA.0, 0x28060);
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.keys = Some(native::Keys {
            keys: [None; native::TRACKED],
            table: Array { data: 0, count: 0 },
        });
        let count = || {
            serde_json::to_value(status()).unwrap()["batch_fallbacks"]
                .as_u64()
                .unwrap()
        };
        let before = count();
        let cost = captured.capture(&memory, BASE, STATE, 0, true, None);
        assert_eq!(cost[0], captured.phase_ticks().iter().sum::<u64>());
        assert_eq!(
            captured.outcomes(),
            [Outcome::Rows { len: 1 }, Outcome::Rows { len: 0 }]
        );
        assert_eq!(count(), before);

        memory
            .fault
            .set(Some(IDEA + native::IDEA_MODIFIER + cmodifier::ACCUMULATOR));
        let cost = captured.capture(&memory, BASE, STATE, 1, true, None);
        assert_eq!(cost[0], captured.phase_ticks().iter().sum::<u64>());
        assert_eq!(captured.phase_ticks()[3], 0);
        assert_eq!(
            captured.outcomes(),
            [Outcome::Unavailable {
                reason: "modifier source entries invalid"
            }; N]
        );
        assert_eq!(count(), before + 1);
        assert_eq!(captured.scratch.batch_fallbacks(), 1);

        const NEXT_COUNTRY: usize = 0x40000;
        memory.word(0x3008, NEXT_COUNTRY as u64);
        memory.word(NEXT_COUNTRY, (BASE + ccountry::VTABLE_RVA.0) as u64);
        memory.word(NEXT_COUNTRY + ccountry::ID, 1);
        memory
            .fault
            .set(Some(NEXT_COUNTRY + native::PROPAGANDA_STABILITY_PENALTY));
        let cost = captured.capture(&memory, BASE, STATE, 2, true, None);
        assert_eq!(cost[0], captured.phase_ticks().iter().sum::<u64>());
        assert_eq!(&captured.phase_ticks()[1..], &[0; 3]);
        assert_eq!(
            captured.outcomes(),
            [Outcome::Unavailable {
                reason: "country stability fields unreadable"
            }; N]
        );
        assert_eq!(captured.scratch.batch_fallbacks(), 0);
        assert_eq!(count(), before + 1);
    }

    #[test]
    fn failed_static_witness_read_keeps_both_stability_tables_available() {
        use engine_native_politics::{cdiplomacy_status, cpolitical_status};
        use engine_native::cgame_state;
        use engine_native_parts::{
            cidea,
            cideology_group,
            cstatic_modifier,
            modifier_definition,
        };
        use engine_native_memory::{cpdx_array::Array, msvc_string};
        use engine_stability::{cmodifier, cworld_threat};
        use std::{cell::Cell, collections::BTreeMap};

        struct World {
            bytes: BTreeMap<usize, u8>,
            fault: Cell<Option<usize>>,
        }
        impl World {
            fn put(&mut self, at: usize, bytes: &[u8]) {
                self.bytes
                    .extend(bytes.iter().enumerate().map(|(i, &b)| (at + i, b)));
            }
            fn word(&mut self, at: usize, value: u64) {
                self.put(at, &value.to_le_bytes());
            }
            fn u32(&mut self, at: usize, value: u32) {
                self.put(at, &value.to_le_bytes());
            }
            fn array(&mut self, at: usize, data: usize, count: u32) {
                self.word(at, data as u64);
                self.word(at + 8, u64::from(count) | (u64::from(count) << 32));
            }
            fn string(&mut self, at: usize, text: &[u8]) {
                let mut storage = [0_u8; 16];
                storage[..text.len()].copy_from_slice(text);
                self.put(at, &storage);
                self.word(at + msvc_string::LENGTH, text.len() as u64);
                self.word(at + msvc_string::CAPACITY, 15);
            }
        }
        impl Memory for World {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                if self
                    .fault
                    .get()
                    .is_some_and(|fault| at <= fault && fault - at < out.len())
                {
                    return false;
                }
                for (i, byte) in out.iter_mut().enumerate() {
                    *byte = self.bytes.get(&(at + i)).copied().unwrap_or(0);
                }
                true
            }
        }
        const BASE: usize = 0x1_4000_0000;
        const STATE: usize = 0x1000;
        const COUNTRY: usize = 0x10000;
        const STATUS: usize = 0x20000;
        const PARTY: usize = 0x21000;
        const IDEA: usize = 0x22000;
        const GROUP: usize = 0x23000;
        const DIPLOMACY: usize = 0x24000;
        const STATS: usize = 0x25000;
        const RING: usize = 0x26000;
        const WORLD: usize = 0x27000;
        const DATABASE: usize = 0x28000;
        const SUNK: usize = 0x29000;
        const SUNK_ENTRIES: usize = 0x29400;
        const CHILD: usize = 0x2a000;
        const CHILDREN: usize = 0x2a400;
        const CHILD_ENTRIES: usize = 0x2a800;
        const TABLE: usize = 0x2b000;
        const STABILITY_KEY: u32 = 5;

        let mut memory = World {
            bytes: BTreeMap::new(),
            fault: Cell::new(None),
        };
        memory.array(STATE + cgame_state::COUNTRIES, 0x3000, 2);
        memory.array(STATE + cgame_state::COUNTRY_LINKS, 0x4000, 2);
        memory.word(0x3008, COUNTRY as u64);
        memory.u32(0x4004, 1);
        memory.word(COUNTRY, (BASE + ccountry::VTABLE_RVA.0) as u64);
        memory.u32(COUNTRY + ccountry::ID, 1);
        memory.word(COUNTRY + ccountry::STABILITY, 50_000);
        memory.put(COUNTRY + native::PRIDE_OF_THE_FLEET_SUNK_ACTIVE, &[1]);
        memory.word(COUNTRY + ccountry::DIPLOMACY_STATUS, DIPLOMACY as u64);
        memory.word(DIPLOMACY, (BASE + engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0) as u64);
        memory.word(DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::COUNTRY, COUNTRY as u64);
        memory.word(COUNTRY + ccountry::POLITICAL_STATUS, STATUS as u64);
        memory.word(STATUS, (BASE + engine_native_parts::cpolitical_status_layout::VTABLE_RVA.0) as u64);
        memory.word(STATUS + engine_native_parts::cpolitical_status_layout::RULING_PARTY, PARTY as u64);
        memory.word(PARTY, (BASE + engine_native_parts::cpolitical_party_layout::VTABLE_RVA.0) as u64);
        memory.word(PARTY + engine_native_parts::cpolitical_party_layout::IDEOLOGY_GROUP, GROUP as u64);
        memory.word(GROUP, (BASE + cideology_group::VTABLE_RVA.0) as u64);
        memory.string(
            GROUP + native::IDEOLOGY_GROUP_MODIFIER + cmodifier::DISPLAY_NAME,
            b"",
        );
        memory.array(STATUS + cpolitical_status::IDEAS, 0x5000, 1);
        memory.word(0x5000, IDEA as u64);
        memory.word(IDEA, (BASE + cidea::VTABLE_RVA.0) as u64);
        memory.word(COUNTRY + native::AIR_STATISTICS, STATS as u64);
        memory.word(STATS + 0x10, RING as u64);
        memory.word(STATE + cgame_state::WORLD_THREAT, WORLD as u64);
        memory.word(WORLD, (BASE + cworld_threat::VTABLE_RVA.0) as u64);
        memory.array(BASE + modifier_definition::TABLE_RVA.0, TABLE, 8);
        memory.u32(
            TABLE + STABILITY_KEY as usize * modifier_definition::STRIDE + modifier_definition::FLAGS,
            3,
        );
        memory.word(BASE + native::STATIC_DATABASE_RVA.0, DATABASE as u64);
        memory.word(DATABASE + 0xc0, SUNK as u64);
        memory.word(SUNK, (BASE + cstatic_modifier::VTABLE_RVA.0) as u64);
        memory.u32(SUNK_ENTRIES, STABILITY_KEY);
        memory.word(SUNK_ENTRIES + 8, (-3_000_i64) as u64);
        memory.array(SUNK + cmodifier::ACCUMULATOR, SUNK_ENTRIES, 1);
        memory.string(SUNK + cmodifier::DISPLAY_NAME, b"Pride sunk");
        memory.string(SUNK + native::STATIC_NAME, b"pride_sunk");
        memory.array(COUNTRY + native::MODIFIER_CHILDREN, CHILDREN, 1);
        memory.word(CHILDREN, CHILD as u64);
        memory.word(CHILD, (BASE + cmodifier::VTABLE_RVA.0) as u64);
        memory.u32(CHILD_ENTRIES, STABILITY_KEY);
        memory.word(CHILD_ENTRIES + 8, (-3_000_i64) as u64);
        memory.array(CHILD + cmodifier::ACCUMULATOR, CHILD_ENTRIES, 1);
        memory.string(CHILD + cmodifier::DISPLAY_NAME, b"Pride sunk");

        let mut keys = [None; native::TRACKED];
        keys[0] = Some(STABILITY_KEY);
        let mut captured = Captured::with_capacities(&Capacities {
            inputs: 2,
            lines: 4,
            bytes: 64,
            ..Default::default()
        });
        captured.keys = Some(native::Keys {
            keys,
            table: Array {
                data: TABLE,
                count: 8,
            },
        });

        memory
            .fault
            .set(Some(COUNTRY + native::PRIDE_OF_THE_FLEET_SUNK_ACTIVE));
        captured.capture(&memory, BASE, STATE, 0, true, None);
        assert_eq!(
            captured.outcomes(),
            [Outcome::Rows { len: 1 }, Outcome::Rows { len: 1 }]
        );
        let [inputs_view, lines_view] = captured.tables();
        assert!(matches!(inputs_view, TableView::Complete(_)));
        let TableView::Complete(TableRows::CountryStabilityLines(rows)) = lines_view else {
            panic!("expected complete stability lines");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, CountryStabilityLineRow::UNPAIRED);
        assert_eq!(rows[0].text, b"Pride sunk");
        assert_eq!(rows[0].value, -3_000);

        memory
            .fault
            .set(Some(DIPLOMACY + cdiplomacy_status::ENEMY_COUNTRY_COUNT));
        captured.capture(&memory, BASE, STATE, 1, true, None);
        assert_eq!(
            captured.outcomes(),
            [Outcome::Rows { len: 1 }, Outcome::Rows { len: 1 }]
        );
        let [inputs_view, lines_view] = captured.tables();
        assert!(matches!(inputs_view, TableView::Complete(_)));
        let TableView::Complete(TableRows::CountryStabilityLines(rows)) = lines_view else {
            panic!("expected complete stability lines");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, CountryStabilityLineRow::STATIC);
        assert_eq!(rows[0].identity, b"pride_sunk");
        assert_eq!(rows[0].value, -3_000);
    }

    #[test]
    fn a_walk_without_readable_memory_is_unavailable_not_empty() {
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.capture(&memory, 0, 0x1000, 5, false, None);
        for outcome in captured.outcomes() {
            assert!(matches!(outcome, Outcome::Unavailable { reason } if reason != SKIPPED_REASON));
        }
        assert!(
            captured
                .tables()
                .iter()
                .all(|table| matches!(table, TableView::Unavailable { .. }))
        );
        captured.phase_ticks = [1, 2, 3, 4];
        captured.fail_capture(5, false, "worker stopped");
        assert_eq!(captured.phase_ticks(), [0; 4]);
        assert_eq!(
            captured.outcomes()[0],
            Outcome::Unavailable {
                reason: "worker stopped"
            }
        );
    }

    #[test]
    fn capacities_follow_the_live_country_slots() {
        let mut capacity = Capacities::default();
        capacity.raise_for_countries(20_000);
        assert!(capacity.inputs >= 20_000);
        assert_eq!(capacity.lines, 0);
        let raised = capacity;
        capacity.raise_for_countries(10);
        assert!(capacity == raised);
    }

    #[test]
    fn rows_without_their_reads_globals_are_not_published() {
        let mut captured = Captured::with_capacities(&Capacities {
            lines: 2,
            bytes: 9,
            ..Default::default()
        });
        captured.bytes.first[..9].copy_from_slice(b"idea_aDec");
        captured.bytes.len = 9;
        captured.lines.rows[0] = native::LineRow {
            country: 7,
            line: 0,
            key: 0x8d,
            kind: native::Kind::Idea,
            identity: native::Span { start: 0, len: 6 },
            value: 10_000,
            ..Default::default()
        };
        captured.lines.rows[1] = native::LineRow {
            country: 7,
            line: 2,
            key: 0x8d,
            kind: native::Kind::Unpaired,
            text: native::Span { start: 6, len: 3 },
            value: -5,
            ..Default::default()
        };
        captured.lines.outcome = Outcome::Rows { len: 2 };
        assert!(matches!(
            captured.tables()[1],
            TableView::Unavailable { .. }
        ));
    }

    #[test]
    fn line_outcomes_combine_their_unread_counts() {
        let rows = |len| Outcome::Rows { len };
        let truncated = Outcome::Truncated {
            len: 0,
            unread_rows: 4,
        };
        assert_eq!(combine(rows(2), rows(3)), Ok(rows(5)));
        assert_eq!(
            combine(truncated, rows(3)),
            Ok(Outcome::Truncated {
                len: 3,
                unread_rows: 4
            })
        );
        let _ = TableRows::empty(TableId::CountryStabilityLines);
    }

    #[test]
    fn scalar_only_stability_updates_carry_lines_without_walking_modifier_children() {
        use engine_native::cgame_state;
        use engine_native_memory::{cpdx_array::Array, msvc_string};
        use engine_native_parts::{cidea, cideology_group, cstatic_modifier, modifier_definition};
        use engine_native_politics::cpolitical_status;
        use engine_stability::{cmodifier, cworld_threat};
        use std::{cell::Cell, collections::BTreeMap};

        struct World {
            bytes: BTreeMap<usize, u8>,
            child_reads: Cell<usize>,
            child_range: std::ops::Range<usize>,
        }
        impl World {
            fn put(&mut self, at: usize, bytes: &[u8]) {
                self.bytes
                    .extend(bytes.iter().enumerate().map(|(i, &b)| (at + i, b)));
            }
            fn word(&mut self, at: usize, value: u64) {
                self.put(at, &value.to_le_bytes());
            }
            fn u32(&mut self, at: usize, value: u32) {
                self.put(at, &value.to_le_bytes());
            }
            fn array(&mut self, at: usize, data: usize, count: u32) {
                self.word(at, data as u64);
                self.word(at + 8, u64::from(count) | (u64::from(count) << 32));
            }
            fn string(&mut self, at: usize, text: &[u8]) {
                let mut storage = [0_u8; 16];
                storage[..text.len()].copy_from_slice(text);
                self.put(at, &storage);
                self.word(at + msvc_string::LENGTH, text.len() as u64);
                self.word(at + msvc_string::CAPACITY, 15);
            }
        }
        impl Memory for World {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                if self.child_range.contains(&at) {
                    self.child_reads.set(self.child_reads.get() + 1);
                }
                for (i, byte) in out.iter_mut().enumerate() {
                    *byte = self.bytes.get(&(at + i)).copied().unwrap_or(0);
                }
                true
            }
        }
        const BASE: usize = 0x1_4000_0000;
        const STATE: usize = 0x1000;
        const COUNTRY: usize = 0x10000;
        const STATUS: usize = 0x20000;
        const PARTY: usize = 0x21000;
        const IDEA: usize = 0x22000;
        const GROUP: usize = 0x23000;
        const DIPLOMACY: usize = 0x24000;
        const STATS: usize = 0x25000;
        const RING: usize = 0x26000;
        const WORLD: usize = 0x27000;
        const DATABASE: usize = 0x28000;
        const SUNK: usize = 0x29000;
        const SUNK_ENTRIES: usize = 0x29400;
        const CHILD: usize = 0x2a000;
        const CHILDREN: usize = 0x2a400;
        const CHILD_ENTRIES: usize = 0x2a800;
        const TABLE: usize = 0x2b000;
        const STABILITY_KEY: u32 = 5;

        let mut memory = World {
            bytes: BTreeMap::new(),
            child_reads: Cell::new(0),
            child_range: CHILD..CHILD + 0x1000,
        };
        memory.array(STATE + cgame_state::COUNTRIES, 0x3000, 2);
        memory.array(STATE + cgame_state::COUNTRY_LINKS, 0x4000, 2);
        memory.word(0x3008, COUNTRY as u64);
        memory.u32(0x4004, 1);
        memory.word(COUNTRY, (BASE + ccountry::VTABLE_RVA.0) as u64);
        memory.u32(COUNTRY + ccountry::ID, 1);
        memory.word(COUNTRY + ccountry::STABILITY, 50_000);
        memory.put(COUNTRY + native::PRIDE_OF_THE_FLEET_SUNK_ACTIVE, &[1]);
        memory.word(COUNTRY + ccountry::DIPLOMACY_STATUS, DIPLOMACY as u64);
        memory.word(
            DIPLOMACY,
            (BASE + engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0) as u64,
        );
        memory.word(
            DIPLOMACY + engine_native_parts::cdiplomacy_status_layout::COUNTRY,
            COUNTRY as u64,
        );
        memory.word(COUNTRY + ccountry::POLITICAL_STATUS, STATUS as u64);
        memory.word(
            STATUS,
            (BASE + engine_native_parts::cpolitical_status_layout::VTABLE_RVA.0) as u64,
        );
        memory.word(
            STATUS + engine_native_parts::cpolitical_status_layout::RULING_PARTY,
            PARTY as u64,
        );
        memory.word(
            PARTY,
            (BASE + engine_native_parts::cpolitical_party_layout::VTABLE_RVA.0) as u64,
        );
        memory.word(
            PARTY + engine_native_parts::cpolitical_party_layout::IDEOLOGY_GROUP,
            GROUP as u64,
        );
        memory.word(GROUP, (BASE + cideology_group::VTABLE_RVA.0) as u64);
        memory.string(
            GROUP + native::IDEOLOGY_GROUP_MODIFIER + cmodifier::DISPLAY_NAME,
            b"",
        );
        memory.array(STATUS + cpolitical_status::IDEAS, 0x5000, 1);
        memory.word(0x5000, IDEA as u64);
        memory.word(IDEA, (BASE + cidea::VTABLE_RVA.0) as u64);
        memory.word(COUNTRY + native::AIR_STATISTICS, STATS as u64);
        memory.word(STATS + 0x10, RING as u64);
        memory.word(STATE + cgame_state::WORLD_THREAT, WORLD as u64);
        memory.word(WORLD, (BASE + cworld_threat::VTABLE_RVA.0) as u64);
        memory.array(BASE + modifier_definition::TABLE_RVA.0, TABLE, 8);
        memory.u32(
            TABLE + STABILITY_KEY as usize * modifier_definition::STRIDE + modifier_definition::FLAGS,
            3,
        );
        memory.word(BASE + native::STATIC_DATABASE_RVA.0, DATABASE as u64);
        memory.word(DATABASE + 0xc0, SUNK as u64);
        memory.word(SUNK, (BASE + cstatic_modifier::VTABLE_RVA.0) as u64);
        memory.u32(SUNK_ENTRIES, STABILITY_KEY);
        memory.word(SUNK_ENTRIES + 8, (-3_000_i64) as u64);
        memory.array(SUNK + cmodifier::ACCUMULATOR, SUNK_ENTRIES, 1);
        memory.string(SUNK + cmodifier::DISPLAY_NAME, b"Pride sunk");
        memory.string(SUNK + native::STATIC_NAME, b"pride_sunk");
        memory.array(COUNTRY + native::MODIFIER_CHILDREN, CHILDREN, 1);
        memory.word(CHILDREN, CHILD as u64);
        memory.word(CHILD, (BASE + cmodifier::VTABLE_RVA.0) as u64);
        memory.u32(CHILD_ENTRIES, STABILITY_KEY);
        memory.word(CHILD_ENTRIES + 8, (-3_000_i64) as u64);
        memory.array(CHILD + cmodifier::ACCUMULATOR, CHILD_ENTRIES, 1);
        memory.string(CHILD + cmodifier::DISPLAY_NAME, b"Pride sunk");

        let mut keys = [None; native::TRACKED];
        keys[0] = Some(STABILITY_KEY);
        let mut captured = Captured::with_capacities(&Capacities {
            inputs: 2,
            lines: 4,
            bytes: 64,
            ..Default::default()
        });
        captured.keys = Some(native::Keys {
            keys,
            table: Array {
                data: TABLE,
                count: 8,
            },
        });
        let domain = changes::Domain::enabled(16);
        let mut fingerprints = SliceFingerprints::new();

        captured.capture_changed(&memory, BASE, STATE, 0, true, Some(&mut fingerprints), &domain);
        assert_eq!(
            captured.outcomes(),
            [Outcome::Rows { len: 1 }, Outcome::Rows { len: 1 }]
        );
        assert_eq!(captured.lines_carried_owners(), None);
        assert!(memory.child_reads.get() > 0);

        memory.word(COUNTRY + ccountry::STABILITY, 60_000);
        domain.write(Table::Stability, 1);
        memory.child_reads.set(0);
        captured.capture_changed(&memory, BASE, STATE, 1, false, Some(&mut fingerprints), &domain);
        assert_eq!(
            captured.outcomes(),
            [Outcome::Rows { len: 1 }, Outcome::Rows { len: 0 }]
        );
        assert_eq!(captured.lines_carried_owners(), Some(&[1_u32][..]));
        assert_eq!(memory.child_reads.get(), 0);

        domain.write(Table::Stability, 1);
        domain.write(Table::StabilityLines, 1);
        memory.child_reads.set(0);
        captured.capture_changed(&memory, BASE, STATE, 2, false, Some(&mut fingerprints), &domain);
        assert_eq!(
            captured.outcomes(),
            [Outcome::Rows { len: 1 }, Outcome::Rows { len: 1 }]
        );
        assert_eq!(captured.lines_carried_owners(), None);
        assert!(memory.child_reads.get() > 0);

        memory.put(COUNTRY + native::PRIDE_OF_THE_FLEET_SUNK_ACTIVE, &[0]);
        domain.write(Table::Stability, 1);
        memory.child_reads.set(0);
        captured.capture_changed(&memory, BASE, STATE, 3, false, Some(&mut fingerprints), &domain);
        assert_eq!(
            captured.outcomes(),
            [Outcome::Rows { len: 1 }, Outcome::Rows { len: 1 }]
        );
        assert_eq!(captured.lines_carried_owners(), None);
        assert!(memory.child_reads.get() > 0);
    }
}
