//! Each country's stability, war support and maximum command power inputs
//! and its modifier tree lines with their proven sources
//! (`engine_stability::country_stability`). The `stability` sliced family: each hour
//! reads the hour's country slice, every country on a full hour, a country
//! new to the census or whose last read was not published, and every country
//! a hooked writer marked since the previous hour
//! ([`crate::country_input_changes`]); a quiet country costs no read until
//! its slice. It runs in its own pool
//! job of the first burst, the Stability job, beside Slow; the family's store
//! is loaned with that job.
use crate::{
    clock::qpc,
    country_input_changes::{self as changes, Changes, Table},
    slicing::{self, Changed, SliceFingerprints, SliceSelection},
    table_capture::{Arena, CaptureRows},
    table_conversion::{table, unavailable},
};
use engine_allocation::{copy_vec, owned_vec};
use engine_native::{Memory, ccountry, walk::Outcome};
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
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("stability inputs, census and tree lines", 0, &[0, 1])];

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
    /// One input row per country slot. Native line/text demand is independent.
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

pub struct Captured {
    inputs: CaptureRows<native::InputRow>,
    lines: CaptureRows<native::LineRow>,
    bytes: Arena<u8>,
    scratch: native::Scratch,
    /// Shared static observations, invalidated before every hourly attempt.
    memo: native::Memo,
    /// Which countries this hour's rows cover.
    selection: SliceSelection,
    /// This hour's marked countries.
    changed: Changed,
    /// Tracked keys, resolved once per modifier definition table header.
    keys: Option<native::Keys>,
    /// The globals every input row of this read carries.
    context: Option<native::Context>,
    /// This capture's inputs/setup, children, candidates and pairing/emission.
    phase_ticks: [u64; 4],
}

impl Captured {
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.inputs
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
        self.lines
            .fail(super::failed_capture(TABLES[1], hour, force_full, reason));
        self.selection.reset(hour, force_full);
        self.phase_ticks = [0; 4];
    }
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub fn vacant() -> Self {
        Self {
            inputs: CaptureRows::new(0),
            lines: CaptureRows::new(0),
            bytes: Arena::new(0),
            scratch: native::Scratch::vacant(),
            memo: native::Memo::default(),
            selection: SliceSelection::new(false),
            changed: Changed::default(),
            keys: None,
            context: None,
            phase_ticks: [0; 4],
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
            selection: SliceSelection::new(true),
            changed: Changed::default(),
            keys: None,
            context: None,
            phase_ticks: [0; 4],
        }
    }
    pub fn reset(&mut self) {
        self.inputs.reset();
        self.lines.reset();
        self.bytes.reset();
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
            + self
                .context
                .as_ref()
                .map_or(0, |context| context.language.capacity())
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.inputs.grow(&mut capacity.inputs);
        self.lines.grow(&mut capacity.lines);
        self.bytes.grow(&mut capacity.bytes);
    }

    /// The countries whose rows this frame's read replaced.
    pub fn selection(&self) -> &SliceSelection {
        &self.selection
    }

    pub fn phase_ticks(&self) -> [u64; 4] {
        self.phase_ticks
    }

    /// Start a frame's selection at its hour; the job resets it again.
    pub fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        self.selection.reset(hour, force_full);
        self.phase_ticks = [0; 4];
    }

    /// Reads this hour's countries, as the hooks' marks select them
    /// ([`Self::capture_changed`]).
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        fingerprints: Option<&mut SliceFingerprints>,
    ) -> [u64; N] {
        self.capture_changed(
            memory,
            base,
            game_state,
            hour,
            force_full,
            fingerprints,
            &changes::Observer,
        )
    }

    /// Reads this hour's countries. Without a store (the inline fallback)
    /// only the hour's slice is read; with one, also the countries new to the
    /// census or whose read was not published and those `changes` marked,
    /// or every country when its take certifies nothing.
    #[allow(clippy::too_many_arguments)]
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
        let result = self.walk(memory, base, game_state, fingerprints, changes, &mut timing);
        if let Err(reason) = result {
            self.inputs.outcome = Outcome::Unavailable { reason };
            self.lines.outcome = Outcome::Unavailable { reason };
        }
        let ended = qpc();
        self.phase_ticks = timing.finish_at(ended);
        [ended.saturating_sub(started).max(0) as u64, 0]
    }

    fn walk(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        mut fingerprints: Option<&mut SliceFingerprints>,
        changes: &dyn Changes,
        timing: &mut native::WalkTiming,
    ) -> Result<(), &'static str> {
        // The first failure stands: a failed check is not retried by resolving again.
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
        let mut ids = None;
        if let Some(fingerprints) = fingerprints.as_deref_mut() {
            // The census scan reads no country's stability: a constant print
            // finds new and unpublished countries; the hooks' marks find
            // changed ones.
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
                // Every native id indexes the link table: its count bounds
                // the ids a writer can mark, dynamic countries included.
                ids = Some(bound.max(countries.arrays()[1].count as usize));
            }
            slicing::select_changed(
                &mut self.selection,
                fingerprints,
                &mut self.changed,
                |visit| {
                    let epoch = changes.take(Table::Stability, visit);
                    manpower_observed = epoch.is_some();
                    epoch
                },
                context.digest(),
            );
            self.selection.scan_ticks = qpc().saturating_sub(started).max(0) as u64;
        }
        context.manpower_enabled = manpower_observed;
        let mut read = 0_u64;
        let mut inputs = 0;
        let mut lines = Outcome::Rows { len: 0 };
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            if !self.selection.contains(country.id) {
                continue;
            }
            let mut input = native::InputRow::default();
            let start = lines.len();
            let walk = native::walk_country_growing_memo_timed(
                memory,
                base,
                &countries,
                country,
                &context,
                &mut self.memo,
                &mut self.scratch,
                &mut input,
                &mut self.lines.rows,
                &mut self.bytes.first,
                start,
                self.bytes.len,
                timing,
            );
            // Count failed speculative batches even when the serial fallback
            // refuses this country, before propagating that original reason.
            let fallbacks = self.scratch.batch_fallbacks();
            if fallbacks != 0 {
                BATCH_FALLBACKS.fetch_add(fallbacks, Ordering::Relaxed);
            }
            let walk = walk?;
            let offset = self.bytes.len;
            self.bytes.required = self
                .bytes
                .required
                .max(offset.saturating_add(walk.required_bytes));
            self.bytes.len += walk.bytes_len;
            lines = combine(lines, walk.lines)?;
            read += 1;
            if let Some(fingerprints) = fingerprints.as_deref_mut()
                && !matches!(walk.lines, Outcome::Rows { .. })
            {
                self.changed.unread(fingerprints, country.id);
            }
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
        self.inputs.outcome = Outcome::Rows { len: inputs };
        self.lines.outcome = lines;
        self.context = Some(context);
        if fingerprints.is_some() {
            changes::record(
                Table::Stability,
                &changes::Counts {
                    marked: self.changed.marked() as u64,
                    read,
                    full: !self.changed.certified,
                },
                ids,
            );
        }
        Ok(())
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [self.inputs.outcome, self.lines.outcome]
    }

    pub fn tables(&self) -> [TableView; N] {
        let Some(context) = self.context.as_ref() else {
            // The read failed or was skipped before it reached any country.
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

/// Sums two row outcomes; the unread counts add.
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
        use engine_native::{
            cdiplomacy_status, cgame_state, cidea, cideology_group, cpdx_array::Array,
            cpolitical_party, cpolitical_status,
        };
        use engine_stability::{cmodifier, cworld_threat};
        use std::{cell::Cell, collections::BTreeMap};

        // Independently authored, zero-initialized sparse synthetic memory.
        // Empty rosters/accumulators keep this about capture's error fold.
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
        memory.word(COUNTRY, (BASE + ccountry::VTABLE_RVA) as u64);
        memory.word(COUNTRY + ccountry::ID, 1);
        memory.word(COUNTRY + ccountry::DIPLOMACY_STATUS, DIPLOMACY as u64);
        memory.word(DIPLOMACY, (BASE + cdiplomacy_status::VTABLE_RVA) as u64);
        memory.word(DIPLOMACY + cdiplomacy_status::COUNTRY, COUNTRY as u64);
        memory.word(COUNTRY + ccountry::POLITICAL_STATUS, STATUS as u64);
        memory.word(STATUS, (BASE + cpolitical_status::VTABLE_RVA) as u64);
        memory.word(STATUS + cpolitical_status::RULING_PARTY, PARTY as u64);
        memory.word(PARTY, (BASE + cpolitical_party::VTABLE_RVA) as u64);
        memory.word(PARTY + cpolitical_party::IDEOLOGY_GROUP, GROUP as u64);
        memory.word(GROUP, (BASE + cideology_group::VTABLE_RVA) as u64);
        memory.word(
            GROUP
                + native::IDEOLOGY_GROUP_MODIFIER
                + cmodifier::DISPLAY_NAME
                + engine_native::msvc_string::CAPACITY,
            15,
        );
        memory.array(STATUS + cpolitical_status::IDEAS, 0x5000, 1);
        memory.word(0x5000, IDEA as u64);
        memory.word(IDEA, (BASE + cidea::VTABLE_RVA) as u64);
        memory.word(COUNTRY + native::AIR_STATISTICS, STATS as u64);
        memory.word(STATS + 0x10, RING as u64); // empty interception ring
        memory.word(STATE + cgame_state::WORLD_THREAT, WORLD as u64);
        memory.word(WORLD, (BASE + cworld_threat::VTABLE_RVA) as u64);
        memory.word(BASE + native::STATIC_DATABASE_RVA, 0x28060);
        let mut captured = Captured::with_capacities(&Capacities::default());
        // Bypass name resolution with an explicitly empty synthetic table.
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

        // A different country reuses refused-country scratch. Its inputs fail
        // before Observed exists, so the previous count must not be folded.
        const NEXT_COUNTRY: usize = 0x40000;
        memory.word(0x3008, NEXT_COUNTRY as u64);
        memory.word(NEXT_COUNTRY, (BASE + ccountry::VTABLE_RVA) as u64);
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
    fn the_engine_and_replay_track_the_same_names_in_the_same_order() {
        assert_eq!(native::KEY_NAMES, CountryStabilityInputRow::KEY_NAMES);
    }

    #[test]
    fn a_walk_without_readable_memory_is_unavailable_not_empty() {
        let memory = engine_native::memory::SliceMemory {
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
        // No context: the read never reached the countries.
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
}
