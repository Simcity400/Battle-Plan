
use crate::{
    clock::qpc,
    relation_changes::{self, Changes},
    slicing::{SliceFingerprints, SliceSelection},
    table_capture::{CaptureRows, grow_capacity},
    table_conversion::table,
};
use engine_air_history::air_participant_vectors as native;
use engine_allocation as allocation;
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::ccountry;
use replay_data::{rows::AirCountryRelationRow, rows::TableId, rows::tables};
use replay_frame::frame::TableView;
use std::sync::{Mutex, MutexGuard};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::AirCountryRelations];

pub const PARTS: usize = 8;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    bytes: usize,
    countries: usize,
}

collector_primitives::capacity_max!(Capacities {
    rows,
    bytes,
    countries
});

impl Capacities {
    pub fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(&mut self.countries, slots);
    }
}

type Sequence = Option<(u64, u64)>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Idle,
    Quiet,
    Copied,
    Failed(&'static str),
}

#[derive(Default)]
pub struct Staging {
    bytes: Vec<u8>,
    used: usize,
    staged: Vec<native::Staged>,
    certify: Vec<(usize, [Sequence; 2])>,
    removed: Vec<u32>,
    slots: u32,
    state: State,
    needed: usize,
    reused: u64,
    ticks: u64,
}

impl Staging {
    fn with_capacity(capacity: &Capacities) -> Self {
        let mut staging = Self {
            bytes: allocation::owned_vec(allocation::filled(capacity.bytes, 0)),
            ..Self::default()
        };
        staging.reserve_countries(capacity.countries);
        staging
    }

    fn reserve_countries(&mut self, slots: usize) {
        let owned = slots.div_ceil(PARTS);
        let _ = reserve(&mut self.staged, owned);
        let _ = reserve(&mut self.certify, owned);
        let _ = reserve(&mut self.removed, owned);
    }

    fn clear(&mut self) {
        self.used = 0;
        self.staged.clear();
        self.certify.clear();
        self.removed.clear();
        self.slots = 0;
        self.state = State::Idle;
        self.needed = 0;
        self.reused = 0;
        self.ticks = 0;
    }

    fn allocated_bytes(&self) -> usize {
        self.bytes.capacity()
            + self.staged.capacity() * size_of::<native::Staged>()
            + self.certify.capacity() * size_of::<(usize, [Sequence; 2])>()
            + self.removed.capacity() * 4
    }

    pub fn fail(&mut self, reason: &'static str) {
        self.state = State::Failed(reason);
    }
}

pub struct Captured {
    rows: CaptureRows<native::Row>,
    selection: SliceSelection,
    parts: Vec<Staging>,
    staged: bool,
}

impl Captured {
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            selection: SliceSelection::new(),
            parts: allocation::owned_vec(allocation::collect_vec(
                (0..PARTS).map(|_| Staging::with_capacity(capacity)),
            )),
            staged: false,
        }
    }

    pub fn reset(&mut self) {
        debug_assert!(!self.staged, "a frame was reused before its copies settled");
        self.rows.reset();
        for part in &mut self.parts {
            part.clear();
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self
                .parts
                .iter()
                .map(Staging::allocated_bytes)
                .sum::<usize>()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        let needed = self.parts.iter().map(|part| part.needed).max().unwrap_or(0);
        grow_capacity(&mut capacity.bytes, needed);
        let slots = self.parts.iter().map(|part| part.slots).max().unwrap_or(0);
        crate::country_scale::raise(&mut capacity.countries, slots as usize);
        for part in &mut self.parts {
            if part.bytes.len() < capacity.bytes {
                if reserve(&mut part.bytes, capacity.bytes).is_err() {
                    return;
                }
                part.bytes.resize(capacity.bytes, 0);
            }
            part.reserve_countries(capacity.countries);
        }
    }

    pub fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        self.selection.reset(hour, force_full);
    }

    pub fn selection(&self) -> &SliceSelection {
        &self.selection
    }

    pub fn take_part(&mut self, part: usize) -> Staging {
        std::mem::take(&mut self.parts[part])
    }

    pub fn restore_part(&mut self, part: usize, staging: Staging) {
        self.parts[part] = staging;
        self.staged = true;
    }

    pub fn ticks(&self) -> [u64; N] {
        [self.parts.iter().map(|part| part.ticks).sum()]
    }

    pub fn staged(&self) -> bool {
        self.staged
    }

    pub fn unstaged(&mut self) -> [u64; N] {
        for part in &mut self.parts {
            part.fail("air relation parts not dispatched");
        }
        self.staged = true;
        [0]
    }

    pub fn dropped(&mut self) {
        if std::mem::take(&mut self.staged) {
            forget_certificates();
        }
    }

    pub fn decode(&mut self) {
        if self.staged {
            self.decode_into(&mut HELD.lock().unwrap_or_else(|poisoned| poisoned.into_inner()));
        }
    }

    fn decode_into(&mut self, held: &mut Held) {
        self.staged = false;
        let mut fingerprints = match held.fingerprints.take() {
            Some(fingerprints) => fingerprints,
            None => {
                let fingerprints = SliceFingerprints::new();
                if allocation::capture_stopped() {
                    self.rows.fail(Outcome::Unavailable {
                        reason: "collector allocation failed",
                    });
                    return;
                }
                Box::new(fingerprints)
            }
        };
        self.apply(held, &mut fingerprints);
        held.fingerprints = Some(fingerprints);
    }

    fn apply(&mut self, held: &mut Held, fingerprints: &mut SliceFingerprints) {
        let (mut reused, mut read, mut quiet) = (0, 0, true);
        let mut failure = None;
        for part in &self.parts {
            match part.state {
                State::Idle => failure = Some("air relation part did not run"),
                State::Failed(reason) => failure = Some(reason),
                State::Quiet => reused += part.reused,
                State::Copied => {
                    quiet &= part.staged.is_empty() && part.removed.is_empty();
                    reused += part.reused;
                    read += part.staged.len() as u64;
                    if held.apply(part).is_err() {
                        failure = Some("collector allocation failed");
                        break;
                    }
                }
            }
        }
        relation_changes::record(reused, read, u64::from(quiet && failure.is_none()));
        if let Some(reason) = failure {
            self.rows.fail(Outcome::Unavailable { reason });
            self.selection.invalidate(fingerprints);
            return;
        }
        let begin = qpc();
        for (slot, country) in held.countries.iter().enumerate() {
            if country.country != 0 {
                self.selection
                    .scan(fingerprints, slot as u32, Some(country.generation));
            }
        }
        if allocation::capture_stopped() {
            self.rows.fail(Outcome::Unavailable {
                reason: "collector allocation failed",
            });
            self.selection.invalidate(fingerprints);
            return;
        }
        fingerprints.finish_scan();
        self.selection.finish_census(fingerprints);
        self.selection.scan_ticks = qpc().saturating_sub(begin).max(0) as u64;
        self.rows.outcome = held.publish(&self.selection, &mut self.rows.rows);
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }

    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::AirCountryRelations, _>(
            self.rows.outcome,
            &self.rows.rows,
            |r| AirCountryRelationRow {
                country: r.country,
                kind: r.kind,
                ordinal: r.ordinal,
                other: r.other,
            },
        )]
    }
}

#[derive(Clone, Copy, Default)]
struct Certificate {
    country: usize,
    diplomacy: usize,
    sequences: [(u64, u64); 2],
}

fn same_countries(
    countries: &ccountry::Reader<'_, impl Memory>,
    part: usize,
    known: &[Certificate],
) -> Result<bool, &'static str> {
    for (index, slot) in (part..countries.count() as usize)
        .step_by(PARTS)
        .enumerate()
    {
        if slot == 0 {
            continue;
        }
        let pointer = countries
            .at(slot as u32)?
            .map_or(0, |country| country.pointer);
        if known.get(index).map_or(0, |known| known.country) != pointer {
            return Ok(false);
        }
    }
    Ok(true)
}

pub struct Certificates {
    quiet: Sequence,
    complete: bool,
    slots: usize,
    known: Vec<Certificate>,
}

impl Default for Certificates {
    fn default() -> Self {
        Self::new()
    }
}

impl Certificates {
    pub const fn new() -> Self {
        Self {
            quiet: None,
            complete: false,
            slots: 0,
            known: Vec::new(),
        }
    }
    pub fn forget(&mut self) {
        self.complete = false;
    }
}

static CERTIFICATES: [Mutex<Certificates>; PARTS] =
    [const { Mutex::new(Certificates::new()) }; PARTS];

pub fn certificates(part: usize) -> MutexGuard<'static, Certificates> {
    CERTIFICATES[part]
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn forget_certificates() {
    for part in 0..PARTS {
        certificates(part).forget();
    }
}

pub fn stage(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    part: usize,
    changes: &impl Changes,
    certificates: &mut Certificates,
    staging: &mut Staging,
) {
    let begin = qpc();
    staging.clear();
    staging.state = match stage_part(
        memory,
        base,
        game_state,
        part,
        changes,
        certificates,
        staging,
    ) {
        Ok(state) => state,
        Err(reason) => {
            certificates.forget();
            State::Failed(reason)
        }
    };
    staging.ticks = qpc().saturating_sub(begin).max(0) as u64;
}

fn stage_part(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    part: usize,
    changes: &impl Changes,
    certificates: &mut Certificates,
    staging: &mut Staging,
) -> Result<State, &'static str> {
    let quiet = changes.quiet();
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let quiet_hour = certificates.complete
        && quiet.is_some()
        && quiet == certificates.quiet
        && countries.count() as usize == certificates.slots
        && same_countries(&countries, part, &certificates.known)?;
    if quiet_hour {
        staging.reused = certificates
            .known
            .iter()
            .filter(|known| known.country != 0)
            .count() as u64;
        return Ok(State::Quiet);
    }
    let complete = std::mem::replace(&mut certificates.complete, false);
    certificates.slots = countries.count() as usize;
    staging.slots = countries.count();
    let owned = (staging.slots as usize)
        .saturating_sub(part)
        .div_ceil(PARTS);
    reserve(&mut certificates.known, owned)?;
    reserve(&mut staging.staged, owned)?;
    reserve(&mut staging.certify, owned)?;
    reserve(&mut staging.removed, owned)?;
    certificates.known.resize(owned, Certificate::default());
    let mut pending = Pending {
        links: countries.arrays()[1].count,
        ..Pending::default()
    };
    for (index, slot) in (part..staging.slots as usize).step_by(PARTS).enumerate() {
        if slot == 0 {
            continue;
        }
        let known = certificates.known[index];
        let country = countries.at(slot as u32)?;
        let carried = complete
            && known.country != 0
            && country.is_some_and(|country| country.pointer == known.country)
            && [
                changes.sequence(known.country),
                changes.sequence(known.diplomacy),
            ] == [Some(known.sequences[0]), Some(known.sequences[1])];
        if carried {
            staging.reused += 1;
            continue;
        }
        let Some(country) = country else {
            if known.country != 0 || !complete {
                staging.removed.push(slot as u32);
            }
            certificates.known[index] = Certificate::default();
            continue;
        };
        pending.push(
            index,
            slot as u32,
            country.pointer,
            changes.sequence(country.pointer),
        );
        if pending.len == native::BATCH {
            pending.flush(memory, base, changes, staging)?;
        }
    }
    pending.flush(memory, base, changes, staging)?;
    staging.needed = staging.used;
    if staging.bytes.len() < staging.used {
        reserve(&mut staging.bytes, staging.used)?;
        staging.bytes.resize(staging.used, 0);
    }
    native::copy_entries(memory, &staging.staged, &mut staging.bytes)?;
    let mut observed = true;
    for (staged, &(index, sequences)) in staging.staged.iter().zip(&staging.certify) {
        observed &= sequences.iter().all(Option::is_some);
        certificates.known[index] = Certificate {
            country: staged.country,
            diplomacy: staged.diplomacy,
            sequences: sequences.map(|sequence| sequence.unwrap_or((0, u64::MAX))),
        };
    }
    certificates.quiet = quiet.filter(|_| observed);
    certificates.complete = true;
    Ok(State::Copied)
}

fn reserve<T>(vector: &mut Vec<T>, len: usize) -> Result<(), &'static str> {
    let additional = len.saturating_sub(vector.len());
    allocation::reserve(vector, additional).map_err(|_| "air relation staging allocation failed")
}

struct Pending {
    links: u32,
    len: usize,
    index: [usize; native::BATCH],
    slot: [u32; native::BATCH],
    country: [usize; native::BATCH],
    sequence: [Sequence; native::BATCH],
}

impl Default for Pending {
    fn default() -> Self {
        Self {
            links: 0,
            len: 0,
            index: [0; native::BATCH],
            slot: [0; native::BATCH],
            country: [0; native::BATCH],
            sequence: [None; native::BATCH],
        }
    }
}

impl Pending {
    fn push(&mut self, index: usize, slot: u32, country: usize, sequence: Sequence) {
        self.index[self.len] = index;
        self.slot[self.len] = slot;
        self.country[self.len] = country;
        self.sequence[self.len] = sequence;
        self.len += 1;
    }

    fn flush(
        &mut self,
        memory: &impl Memory,
        base: usize,
        changes: &impl Changes,
        staging: &mut Staging,
    ) -> Result<(), &'static str> {
        let len = std::mem::take(&mut self.len);
        if len == 0 {
            return Ok(());
        }
        let mut diplomacy = [0; native::BATCH];
        native::diplomacy_pointers(memory, &self.country[..len], &mut diplomacy[..len])?;
        let mut batch = [(0, 0, 0); native::BATCH];
        for index in 0..len {
            batch[index] = (self.slot[index], self.country[index], diplomacy[index]);
            staging.certify.push((
                self.index[index],
                [self.sequence[index], changes.sequence(diplomacy[index])],
            ));
        }
        native::headers(memory, base, self.links, &batch[..len], |mut staged| {
            staged.offset = staging.used;
            staging.used += staged.bytes();
            staging.staged.push(staged);
        })
    }
}

#[derive(Default)]
struct HeldCountry {
    country: usize,
    counts: [u32; 3],
    entries: Vec<u32>,
    generation: u64,
}

impl HeldCountry {
    fn holds(&self, staged: &native::Staged, bytes: &[u8]) -> bool {
        self.country == staged.country
            && self.counts == staged.counts
            && self
                .entries
                .iter()
                .zip(bytes.chunks_exact(4))
                .all(|(&held, entry)| held.to_le_bytes() == entry)
    }
}

pub struct Held {
    countries: Vec<HeldCountry>,
    generation: u64,
    fingerprints: Option<Box<SliceFingerprints>>,
}

static HELD: Mutex<Held> = Mutex::new(Held::new());

impl Default for Held {
    fn default() -> Self {
        Self::new()
    }
}

impl Held {
    pub const fn new() -> Self {
        Self {
            countries: Vec::new(),
            generation: 0,
            fingerprints: None,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn rows(&self) -> usize {
        self.countries
            .iter()
            .map(|country| country.entries.len())
            .sum()
    }

    fn apply(&mut self, part: &Staging) -> Result<(), &'static str> {
        let slots = part.slots as usize;
        if self.countries.len() != slots {
            reserve(&mut self.countries, slots)?;
            self.countries.resize_with(slots, HeldCountry::default);
        }
        for &slot in &part.removed {
            let country = &mut self.countries[slot as usize];
            country.country = 0;
            country.counts = [0; 3];
            country.entries.clear();
            self.generation += 1;
            country.generation = self.generation;
        }
        for staged in &part.staged {
            let bytes = &part.bytes[staged.offset..staged.offset + staged.bytes()];
            let country = &mut self.countries[staged.slot as usize];
            if country.holds(staged, bytes) {
                continue;
            }
            country.country = staged.country;
            country.counts = staged.counts;
            country.entries.clear();
            reserve(&mut country.entries, bytes.len() / 4)?;
            country.entries.extend(
                bytes
                    .chunks_exact(4)
                    .map(|entry| u32::from_le_bytes(entry.try_into().unwrap())),
            );
            self.generation += 1;
            country.generation = self.generation;
        }
        Ok(())
    }

    fn publish(&self, selection: &SliceSelection, output: &mut Vec<native::Row>) -> Outcome {
        let selected = || {
            self.countries
                .iter()
                .enumerate()
                .filter(|(slot, country)| country.country != 0 && selection.contains(*slot as u32))
        };
        let needed: usize = selected().map(|(_, country)| country.entries.len()).sum();
        if !crate::table_capture::grow_vec(output, needed) {
            return Outcome::Unavailable {
                reason: "collector allocation failed",
            };
        }
        let mut len = 0;
        for (slot, country) in selected() {
            let staged = native::Staged {
                slot: slot as u32,
                counts: country.counts,
                ..native::Staged::default()
            };
            native::rows(&staged, &country.entries, |row| {
                output[len] = row;
                len += 1;
            });
        }
        Outcome::Rows { len }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn allocation_failure_exercise() {
    let mut part = Staging::with_capacity(&Capacities {
        rows: 3,
        bytes: 12,
        countries: PARTS,
    });
    if crate::allocation::failed() {
        return;
    }
    for (bytes, value) in part.bytes.chunks_exact_mut(4).zip([2u32, 3, 4]) {
        bytes.copy_from_slice(&value.to_le_bytes());
    }
    part.staged.push(native::Staged {
        slot: 1,
        country: 0x1000,
        counts: [1, 1, 1],
        offset: 0,
        ..native::Staged::default()
    });
    part.slots = 3;
    part.state = State::Copied;
    let mut held = Held::new();
    if held.apply(&part).is_err() {
        return;
    }
    let mut selection = SliceSelection::new();
    if crate::allocation::failed() {
        return;
    }
    selection.reset(0, true);
    let mut output = Vec::new();
    let outcome = held.publish(&selection, &mut output);
    if crate::allocation::failed() {
        return;
    }
    let _ =
        table::<tables::AirCountryRelations, _>(outcome, &output, |row| AirCountryRelationRow {
            country: row.country,
            kind: row.kind,
            ordinal: row.ordinal,
            other: row.other,
        });
}

#[cfg(any(test, feature = "test-support"))]
pub mod hour {
    use super::*;

    pub struct Hour {
        pub pack: Captured,
        pub certificates: Vec<Certificates>,
        pub held: Held,
        pub force_full: bool,
    }

    impl Hour {
        pub fn new(rows: usize, bytes: usize) -> Self {
            Self {
                pack: Captured::with_capacities(&Capacities {
                    rows,
                    bytes,
                    countries: 0,
                }),
                certificates: (0..PARTS).map(|_| Certificates::new()).collect(),
                held: Held::new(),
                force_full: false,
            }
        }

        pub fn run<M: Memory + Sync>(
            &mut self,
            memory: &M,
            base: usize,
            game_state: usize,
            changes: &(impl Changes + Sync),
            hour: i32,
            threads: bool,
        ) -> std::time::Duration {
            let force_full = std::mem::take(&mut self.force_full) || hour == 0;
            self.pack.prepare_hour(hour, force_full);
            let started = std::time::Instant::now();
            let parts = self.pack.parts.iter_mut().zip(&mut self.certificates);
            if threads {
                std::thread::scope(|scope| {
                    for (part, (staging, certificates)) in parts.enumerate() {
                        scope.spawn(move || {
                            stage(
                                memory,
                                base,
                                game_state,
                                part,
                                changes,
                                certificates,
                                staging,
                            )
                        });
                    }
                });
            } else {
                for (part, (staging, certificates)) in parts.enumerate() {
                    stage(
                        memory,
                        base,
                        game_state,
                        part,
                        changes,
                        certificates,
                        staging,
                    );
                }
            }
            let wall = started.elapsed();
            self.pack.decode_into(&mut self.held);
            wall
        }

        pub fn emitted(&self) -> Outcome {
            self.pack.rows.outcome
        }

        pub fn emitted_rows(&self) -> &[native::Row] {
            &self.pack.rows.rows[..self.pack.rows.outcome.len()]
        }

        pub fn staged_countries(&self) -> usize {
            self.pack.parts.iter().map(|part| part.staged.len()).sum()
        }

        pub fn grow(&mut self) {
            let mut capacity = Capacities {
                rows: self.pack.rows.rows.len(),
                bytes: self.pack.parts[0].bytes.len(),
                countries: 0,
            };
            self.pack.grow(&mut capacity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_rows_and_native_selection_publish_countries_then_larger_vectors() {
        const COUNTRIES: usize = 20;
        let mut capacity = Capacities::default();
        capacity.raise_for_countries(COUNTRIES + 1);
        assert_eq!([capacity.rows, capacity.bytes], [0; 2]);
        let mut pack = Captured::with_capacities(&Capacities::default());
        assert!(pack.parts.iter().all(|part| part.bytes.is_empty()));
        let mut held = Held::new();
        for (hour, counts) in [[3u32, 5, 5], [7, 11, 11]].into_iter().enumerate() {
            pack.reset();
            pack.prepare_hour(hour as i32, hour == 0);
            for part in &mut pack.parts {
                part.slots = (COUNTRIES + 1) as u32;
                part.state = State::Copied;
            }
            let entries = counts.iter().map(|&count| count as usize).sum::<usize>();
            for slot in 1..=COUNTRIES {
                let part = &mut pack.parts[slot % PARTS];
                part.staged.push(native::Staged {
                    slot: slot as u32,
                    country: slot + 1,
                    counts,
                    offset: part.bytes.len(),
                    ..Default::default()
                });
                for ordinal in 0..entries {
                    part.bytes
                        .extend_from_slice(&((slot * 100 + ordinal) as u32).to_le_bytes());
                }
            }
            pack.staged = true;
            pack.decode_into(&mut held);
            assert_eq!(
                pack.outcomes(),
                [Outcome::Rows {
                    len: COUNTRIES * entries
                }]
            );
            assert_eq!(pack.selection.read_owners().count(), COUNTRIES);
            let last = pack.rows.rows[COUNTRIES * entries - 1];
            assert_eq!(last.country, COUNTRIES as u32);
            assert_eq!(last.kind, native::ENEMY);
            assert_eq!(last.ordinal, counts[2] - 1);
            assert_eq!(last.other, (COUNTRIES * 100 + entries - 1) as u32);
        }
    }

    #[test]
    fn a_frame_whose_parts_did_not_run_is_published_unavailable_by_the_decode() {
        let mut pack = Captured::with_capacities(&Capacities::default());
        pack.prepare_hour(3, false);
        assert_eq!(pack.unstaged(), [0]);
        pack.decode_into(&mut Held::new());
        assert_eq!(
            pack.outcomes(),
            [Outcome::Unavailable {
                reason: "air relation parts not dispatched"
            }]
        );
    }
}
