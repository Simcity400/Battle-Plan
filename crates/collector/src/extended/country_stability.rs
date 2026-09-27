//! Each country's stability, war support and maximum command power inputs
//! and its modifier tree lines with their proven sources
//! (`engine::country_stability`). The `stability` sliced family: each hour
//! reads the hour's country slice, every country on a full hour and every
//! country whose change fingerprint moved. It runs in the Slow pool job; the
//! family's fingerprint store is loaned with that job.
use crate::{
    sampling::qpc,
    slicing::{SliceFingerprints, SliceSelection},
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{Memory, ccountry, country_stability as native, walk::Outcome};
use replay::{
    cadence::{SKIPPED_REASON, should_walk},
    frame::TableView,
    rows::{CountryStabilityInputRow, CountryStabilityLineRow, TableId, tables},
};

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [
    TableId::CountryStabilityInputs,
    TableId::CountryStabilityLines,
];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[(
    "stability inputs, fingerprints and tree lines",
    0,
    &[0, 1],
)];

/// Tree lines per country slot and identity/text bytes per line the first
/// capture is sized for. Most hours read one slice of the countries; a full
/// hour that does not fit truncates and grows both off-thread.
const LINES_PER_COUNTRY: usize = 4;
const BYTES_PER_LINE: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    inputs: usize,
    lines: usize,
    bytes: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            inputs: 512,
            lines: 512 * LINES_PER_COUNTRY,
            bytes: 512 * LINES_PER_COUNTRY * BYTES_PER_LINE,
        }
    }
}

impl Capacities {
    /// One input row and a few lines per country slot. Never lowers.
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        let inputs = crate::country_scale::rows(slots, 1);
        self.inputs = self.inputs.max(inputs);
        self.lines = self.lines.max(inputs.saturating_mul(LINES_PER_COUNTRY));
        self.bytes = self
            .bytes
            .max(self.lines.saturating_mul(BYTES_PER_LINE));
    }
}

pub struct Captured {
    inputs: CaptureRows<native::InputRow>,
    lines: CaptureRows<native::LineRow>,
    bytes: Arena<u8>,
    scratch: native::Scratch,
    /// Which countries this hour's rows cover.
    selection: SliceSelection,
    /// Tracked keys, resolved once per modifier definition table header.
    keys: Option<native::Keys>,
    /// The globals every input row of this read carries.
    context: Option<native::Context>,
}

impl Captured {
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.inputs
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
        self.lines
            .fail(super::failed_capture(TABLES[1], hour, force_full, reason));
        self.selection.reset(hour, force_full);
    }
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self {
            inputs: CaptureRows::new(0),
            lines: CaptureRows::new(0),
            bytes: Arena::new(0),
            scratch: native::Scratch::vacant(),
            selection: SliceSelection::new(false),
            keys: None,
            context: None,
        }
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            inputs: CaptureRows::new(capacity.inputs),
            lines: CaptureRows::new(capacity.lines),
            bytes: Arena::new(capacity.bytes),
            scratch: native::Scratch::new(),
            selection: SliceSelection::new(true),
            keys: None,
            context: None,
        }
    }
    pub fn reset(&mut self) {
        self.inputs.reset();
        self.lines.reset();
        self.bytes.reset();
        self.context = None;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.inputs.allocated_bytes()
            + self.lines.allocated_bytes()
            + self.bytes.allocated_bytes()
            + self.scratch.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.inputs.grow(&mut capacity.inputs);
        self.lines.grow(&mut capacity.lines);
        self.bytes.grow(&mut capacity.bytes);
    }

    /// The countries whose rows this frame's read replaced.
    pub(crate) fn selection(&self) -> &SliceSelection {
        &self.selection
    }

    /// Start a frame's selection at its hour; the job resets it again.
    pub(crate) fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        self.selection.reset(hour, force_full);
    }

    /// Reads this hour's countries. Without a fingerprint store (the inline
    /// fallback) only the hour's slice is read: a fingerprint only adds reads.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        fingerprints: Option<&mut SliceFingerprints>,
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
        let result = self.walk(memory, base, game_state, fingerprints);
        if let Err(reason) = result {
            self.inputs.outcome = Outcome::Unavailable { reason };
            self.lines.outcome = Outcome::Unavailable { reason };
        }
        [qpc().saturating_sub(started).max(0) as u64, 0]
    }

    fn walk(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        fingerprints: Option<&mut SliceFingerprints>,
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
        if let Some(fingerprints) = fingerprints {
            // Scan first, so a changed country joins this hour's read.
            let started = qpc();
            let mut census = true;
            for slot in 1..countries.count() {
                let country = match countries.at(slot) {
                    Ok(Some(country)) => country,
                    Ok(None) => continue,
                    Err(_) => {
                        census = false;
                        continue;
                    }
                };
                let value =
                    native::fingerprint_country(memory, base, country.pointer, &keys).ok();
                self.selection.scan(fingerprints, country.id, value);
            }
            fingerprints.finish_scan();
            if census {
                self.selection.finish_census(fingerprints);
            }
            self.selection.scan_ticks = qpc().saturating_sub(started).max(0) as u64;
        }
        let context = native::Context::read(memory, base, game_state, keys)?;
        self.context = Some(context);
        let mut inputs = 0;
        let mut unread_inputs = 0_u32;
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
            let walk = native::walk_country(
                memory,
                base,
                &countries,
                country,
                &context,
                &mut self.scratch,
                &mut input,
                &mut self.lines.rows[start..],
                &mut self.bytes.first[self.bytes.len..],
            )?;
            // Spans are relative to this country's arena window.
            let offset = self.bytes.len;
            if let Outcome::Rows { len } = walk.lines {
                for row in &mut self.lines.rows[start..start + len] {
                    row.identity.start += offset;
                    row.text.start += offset;
                }
            }
            self.bytes.required = self
                .bytes
                .required
                .max(offset.saturating_add(walk.required_bytes));
            self.bytes.len += walk.bytes_len;
            lines = combine(lines, walk.lines)?;
            if inputs < self.inputs.rows.len() {
                self.inputs.rows[inputs] = input;
                inputs += 1;
            } else {
                unread_inputs += 1;
            }
        }
        self.inputs.outcome = if unread_inputs == 0 {
            Outcome::Rows { len: inputs }
        } else {
            Outcome::Truncated {
                len: inputs,
                unread_rows: unread_inputs,
            }
        };
        self.lines.outcome = lines;
        Ok(())
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [self.inputs.outcome, self.lines.outcome]
    }

    pub fn tables(&self) -> [TableView; N] {
        let Some(context) = self.context else {
            // The read failed or was skipped before it reached any country.
            return self.outcomes().map(|outcome| match outcome {
                Outcome::Unavailable { reason } => unavailable(reason),
                _ => unavailable("stability rows without their read's globals"),
            });
        };
        let keys: Vec<u32> = context
            .keys
            .keys
            .iter()
            .map(|key| key.unwrap_or(u32::MAX))
            .collect();
        let language = context.language.as_bytes().to_vec();
        let d = context.defines;
        let inputs = table::<tables::CountryStabilityInputs, _>(
            self.inputs.outcome,
            &self.inputs.rows,
            |row| {
                let t = row.totals;
                CountryStabilityInputRow {
                    country: row.country,
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
                    max_command_power: t[8],
                    max_command_power_mult: t[9],
                    keys: keys.clone(),
                    children: row.children,
                    language: language.clone(),
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
            return [inputs, unavailable("stability line strings outside captured arena")];
        }
        let lines = table::<tables::CountryStabilityLines, _>(self.lines.outcome, rows, |row| {
            CountryStabilityLineRow {
                country: row.country,
                line: row.line,
                key: row.key,
                kind: kind(row.kind),
                identity: text(row.identity).unwrap().to_vec(),
                identity_id: row.identity_id,
                secondary: row.secondary,
                value: row.value,
                text: text(row.text).unwrap().to_vec(),
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
    use replay::rows::TableRows;

    #[test]
    fn the_engine_and_replay_track_the_same_names_in_the_same_order() {
        assert_eq!(native::KEY_NAMES, CountryStabilityInputRow::KEY_NAMES);
    }

    #[test]
    fn a_walk_without_readable_memory_is_unavailable_not_empty() {
        let memory = engine::memory::SliceMemory {
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
        captured.fail_capture(5, false, "worker stopped");
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
        assert!(capacity.lines >= 20_000 * LINES_PER_COUNTRY);
        let raised = capacity;
        capacity.raise_for_countries(10);
        assert!(capacity == raised);
    }

    #[test]
    fn rows_without_their_reads_globals_are_not_published() {
        let mut captured = Captured::with_capacities(&Capacities::default());
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
        assert!(matches!(captured.tables()[1], TableView::Unavailable { .. }));
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
