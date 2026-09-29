//! The country vectors the flying-aircraft map walks for the viewing country.
//! The viewer builds the viewpoint's participant arrays from them.
//!
//! Their native writers are observed ([`crate::relation_changes`]). The hour's
//! work is split: [`PARTS`] pool jobs, each owning every [`PARTS`]th slot,
//! copy only the raw vectors of countries a writer touched (or whose slot
//! changed, or that were never observed) into their staging buffers, in
//! batched gathers and scattered copies ([`engine::air_participant_vectors`]).
//! A quiet hour copies nothing. The decode thread then turns the staged
//! entries into held rows off the game thread, in hour order.
//!
//! The table is sliced by canonical slot ([`crate::slicing`]): each hour
//! emits the held rows of that hour's slice and of every country whose rows
//! changed since it was last scanned. A country's fingerprint is the
//! generation of its held rows, which moves only when a copy differs.

use crate::{
    relation_changes::{self, Changes},
    sampling::qpc,
    slicing::{SliceFingerprints, SliceSelection},
    spooling::table,
    table_capture::{CaptureRows, grow_capacity},
};
use engine::{Memory, air_participant_vectors as native, ccountry, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{AirCountryRelationRow, TableId, tables},
};
use std::sync::{Mutex, MutexGuard};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::AirCountryRelations];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

/// Pool jobs splitting the copies; slot `s` belongs to part `s % PARTS`.
pub const PARTS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    /// Staging bytes per part.
    bytes: usize,
    /// Country slots, which bound each part's per-country staging.
    countries: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 4096,
            bytes: 16 * 1024,
            countries: 0,
        }
    }
}

/// Rows per country slot presized from the registry: a few donors, wartime
/// allies and enemies. Wars in blocs grow the buffers off-thread.
const ROWS_PER_COUNTRY: usize = 8;

impl Capacities {
    pub fn raise_for_countries(&mut self, slots: usize) {
        let rows = crate::country_scale::rows(slots, ROWS_PER_COUNTRY);
        crate::country_scale::raise(&mut self.rows, rows);
        crate::country_scale::raise(&mut self.bytes, rows * 4 / PARTS);
        crate::country_scale::raise(&mut self.countries, slots);
    }
}

type Sequence = Option<(u64, u64)>;

/// What one part's job did this hour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    /// Not dispatched for this frame.
    #[default]
    Idle,
    /// Every certificate held; nothing copied.
    Quiet,
    Copied,
    Failed(&'static str),
}

/// One part's copies for one hour, loaned to its job and read by the decode
/// thread. Buffers keep their capacity and grow off-thread.
#[derive(Default)]
pub(crate) struct Staging {
    bytes: Vec<u8>,
    used: usize,
    staged: Vec<native::Staged>,
    /// Each staged country's certificate index and sequences.
    certify: Vec<(usize, [Sequence; 2])>,
    /// Slots found empty whose earlier country is gone.
    removed: Vec<u32>,
    /// The registry's slot count.
    slots: u32,
    state: State,
    /// Bytes this hour's copies needed.
    needed: usize,
    reused: u64,
    ticks: u64,
    /// This hour's fresh copy of every country in the part.
    #[cfg(feature = "relation_verify")]
    verify: Option<Box<Verify>>,
}

/// `relation_verify`: a part's copy of every country, with no reuse.
#[cfg(feature = "relation_verify")]
pub(crate) struct Verify {
    staging: Staging,
    /// Forgotten before every copy.
    certificates: Certificates,
}

/// `relation_verify`: nothing is observed, so every country is copied.
#[cfg(feature = "relation_verify")]
struct Unobserved;

#[cfg(feature = "relation_verify")]
impl Changes for Unobserved {
    fn quiet(&self) -> Option<(u64, u64)> {
        None
    }
    fn sequence(&self, _: usize) -> Option<(u64, u64)> {
        None
    }
}

impl Staging {
    fn with_capacity(capacity: &Capacities) -> Self {
        Self {
            #[cfg(feature = "relation_verify")]
            verify: Some(Box::new(Verify {
                staging: Self::without_verify(capacity),
                certificates: Certificates::new(),
            })),
            ..Self::without_verify(capacity)
        }
    }

    fn without_verify(capacity: &Capacities) -> Self {
        let mut staging = Self {
            bytes: vec![0; capacity.bytes],
            ..Self::default()
        };
        staging.reserve_countries(capacity.countries);
        staging
    }

    /// Off the hour: room for a part's share of `slots` countries.
    fn reserve_countries(&mut self, slots: usize) {
        let owned = slots.div_ceil(PARTS);
        self.staged.reserve(owned.saturating_sub(self.staged.len()));
        self.certify.reserve(owned.saturating_sub(self.certify.len()));
        self.removed.reserve(owned.saturating_sub(self.removed.len()));
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
        #[cfg(feature = "relation_verify")]
        let verify = self
            .verify
            .as_ref()
            .map_or(0, |verify| verify.staging.allocated_bytes());
        #[cfg(not(feature = "relation_verify"))]
        let verify = 0;
        self.bytes.capacity()
            + self.staged.capacity() * size_of::<native::Staged>()
            + self.certify.capacity() * size_of::<(usize, [Sequence; 2])>()
            + self.removed.capacity() * 4
            + verify
    }

    /// A failed job: nothing it staged may be applied.
    pub(crate) fn fail(&mut self, reason: &'static str) {
        self.state = State::Failed(reason);
    }
}

pub struct Captured {
    rows: CaptureRows<native::Row>,
    /// The countries this hour's rows cover.
    selection: SliceSelection,
    parts: Vec<Staging>,
    /// The parts ran for this frame and their copies await decoding.
    staged: bool,
    /// `relation_verify`: this frame's check of the held rows.
    #[cfg(feature = "relation_verify")]
    verification: relation_changes::Verification,
}

impl Captured {
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            selection: SliceSelection::new(capacity.rows != 0),
            parts: (0..PARTS)
                .map(|_| Staging::with_capacity(capacity))
                .collect(),
            staged: false,
            #[cfg(feature = "relation_verify")]
            verification: Default::default(),
        }
    }

    pub fn reset(&mut self) {
        self.rows.reset();
        // Copies never decoded leave their certificates unbacked.
        self.dropped();
        for part in &mut self.parts {
            part.clear();
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.parts.iter().map(Staging::allocated_bytes).sum::<usize>()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        let needed = self.parts.iter().map(|part| part.needed).max().unwrap_or(0);
        grow_capacity(&mut capacity.bytes, needed);
        let slots = self.parts.iter().map(|part| part.slots).max().unwrap_or(0);
        crate::country_scale::raise(&mut capacity.countries, slots as usize);
        for part in &mut self.parts {
            if part.bytes.len() < capacity.bytes {
                part.bytes.resize(capacity.bytes, 0);
            }
            part.reserve_countries(capacity.countries);
            #[cfg(feature = "relation_verify")]
            if let Some(verify) = part.verify.as_deref_mut() {
                let fresh = &mut verify.staging;
                let bytes = capacity.bytes.max(fresh.needed);
                if fresh.bytes.len() < bytes {
                    fresh.bytes.resize(bytes, 0);
                }
                fresh.reserve_countries(capacity.countries);
            }
        }
    }

    /// Start a frame's selection at its hour.
    pub(crate) fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        self.selection.reset(hour, force_full);
    }

    /// The countries whose rows this frame emitted.
    pub(crate) fn selection(&self) -> &SliceSelection {
        &self.selection
    }

    /// Loan one part's staging to its job.
    pub(crate) fn take_part(&mut self, part: usize) -> Staging {
        std::mem::take(&mut self.parts[part])
    }

    /// Return a part's staging after its job.
    pub(crate) fn restore_part(&mut self, part: usize, staging: Staging) {
        self.parts[part] = staging;
        self.staged = true;
    }

    /// Worker time of the parts' copies.
    pub(crate) fn ticks(&self) -> [u64; N] {
        [self.parts.iter().map(|part| part.ticks).sum()]
    }

    /// Whether the parts ran for this frame.
    pub(crate) fn staged(&self) -> bool {
        self.staged
    }

    /// The parts did not run for this frame; production always dispatches
    /// them. Nothing is read or decoded here: the decode thread publishes
    /// the table unavailable and invalidates the frame's selection.
    pub(crate) fn unstaged(&mut self) -> [u64; N] {
        for part in &mut self.parts {
            part.fail("air relation parts not dispatched");
        }
        self.staged = true;
        [0]
    }

    /// The frame was dropped before decoding: its copies are lost, so every
    /// certificate they advanced is withdrawn.
    pub(crate) fn dropped(&mut self) {
        if std::mem::take(&mut self.staged) {
            forget_certificates();
        }
    }

    /// Apply the staged copies to the held rows, then scan and publish this
    /// hour's slice. Decode thread, in hour order; a frame whose parts did not
    /// run keeps its outcome.
    pub(crate) fn decode(&mut self) {
        if self.staged {
            self.decode_into(&mut HELD.lock().unwrap_or_else(|poisoned| poisoned.into_inner()));
        }
    }

    fn decode_into(&mut self, held: &mut Held) {
        self.staged = false;
        let mut fingerprints = held
            .fingerprints
            .take()
            .unwrap_or_else(|| Box::new(SliceFingerprints::new()));
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
                    quiet = false;
                    reused += part.reused;
                    read += part.staged.len() as u64;
                    held.apply(part);
                }
            }
        }
        relation_changes::record(reused, read, u64::from(quiet && failure.is_none()));
        if let Some(reason) = failure {
            self.rows.fail(Outcome::Unavailable { reason });
            self.selection.invalidate(fingerprints);
            return;
        }
        #[cfg(feature = "relation_verify")]
        {
            self.verification = held.verify(&self.parts);
            relation_changes::verified(self.verification);
        }
        let begin = qpc();
        for (slot, country) in held.countries.iter().enumerate() {
            if country.country != 0 {
                self.selection
                    .scan(fingerprints, slot as u32, Some(country.generation));
            }
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

/// What a part's jobs last observed of each of its slots: the country and
/// diplomacy status and their notification sequences taken before the copy.
#[derive(Clone, Copy, Default)]
struct Certificate {
    country: usize,
    diplomacy: usize,
    sequences: [(u64, u64); 2],
}

/// Whether each of the part's slots still holds the country object it
/// certified.
fn same_countries(
    countries: &ccountry::Reader<'_, impl Memory>,
    part: usize,
    known: &[Certificate],
) -> Result<bool, &'static str> {
    for (index, slot) in (part..countries.count() as usize).step_by(PARTS).enumerate() {
        if slot == 0 {
            continue;
        }
        let pointer = countries.at(slot as u32)?.map_or(0, |country| country.pointer);
        if known.get(index).map_or(0, |known| known.country) != pointer {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) struct Certificates {
    /// Whole-domain certificate of the last complete hour.
    quiet: Sequence,
    complete: bool,
    /// The registry's slot count when certified.
    slots: usize,
    /// Indexed by slot / [`PARTS`].
    known: Vec<Certificate>,
}

impl Certificates {
    pub(crate) const fn new() -> Self {
        Self {
            quiet: None,
            complete: false,
            slots: 0,
            known: Vec::new(),
        }
    }
    /// Nothing observed so far certifies anything: copy afresh.
    pub(crate) fn forget(&mut self) {
        self.complete = false;
    }
}

static CERTIFICATES: [Mutex<Certificates>; PARTS] =
    [const { Mutex::new(Certificates::new()) }; PARTS];

pub(crate) fn certificates(part: usize) -> MutexGuard<'static, Certificates> {
    CERTIFICATES[part]
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Nothing observed so far certifies anything: every part copies afresh.
pub(crate) fn forget_certificates() {
    for part in 0..PARTS {
        certificates(part).forget();
    }
}

/// One part's hour: copy its touched countries' vectors into `staging`.
/// A failure withdraws the part's certificates.
pub(crate) fn stage(
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
    staging.state = match stage_part(memory, base, game_state, part, changes, certificates, staging)
    {
        Ok(state) => state,
        Err(reason) => {
            certificates.forget();
            State::Failed(reason)
        }
    };
    staging.ticks = qpc().saturating_sub(begin).max(0) as u64;
    #[cfg(feature = "relation_verify")]
    if let Some(verify) = staging.verify.as_deref_mut() {
        verify.certificates.forget();
        stage(
            memory,
            base,
            game_state,
            part,
            &Unobserved,
            &mut verify.certificates,
            &mut verify.staging,
        );
    }
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
    // A quiet hour still checks every slot's country object against the
    // roster, which costs no guarded read.
    if certificates.complete
        && quiet.is_some()
        && quiet == certificates.quiet
        && countries.count() as usize == certificates.slots
        && same_countries(&countries, part, &certificates.known)?
    {
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
    let owned = (staging.slots as usize).saturating_sub(part).div_ceil(PARTS);
    // Presized off the hour; a larger registry grows here, failing the part
    // rather than aborting the game when memory runs out.
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
        // The roster cache answers this without a read in the hour.
        let country = countries.at(slot as u32)?;
        // A cached pair holds while the slot keeps its country object and
        // neither receiver's writers ran.
        if complete
            && known.country != 0
            && country.is_some_and(|country| country.pointer == known.country)
            && [
                changes.sequence(known.country),
                changes.sequence(known.diplomacy),
            ] == [Some(known.sequences[0]), Some(known.sequences[1])]
        {
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
    // Every header is known and no vector is longer than the country links:
    // size the destination, then copy. A change larger than any before grows
    // the buffer here instead of failing the hour; the pool's other buffers
    // grow off-thread from `needed`.
    staging.needed = staging.used;
    if staging.bytes.len() < staging.used {
        reserve(&mut staging.bytes, staging.used)?;
        staging.bytes.resize(staging.used, 0);
    }
    native::copy_entries(memory, &staging.staged, &mut staging.bytes)?;
    let mut observed = true;
    for (staged, &(index, sequences)) in staging.staged.iter().zip(&staging.certify) {
        // An unobserved (e.g. refused) receiver is copied again next hour.
        observed &= sequences.iter().all(Option::is_some);
        certificates.known[index] = Certificate {
            country: staged.country,
            diplomacy: staged.diplomacy,
            sequences: sequences.map(|sequence| sequence.unwrap_or((0, u64::MAX))),
        };
    }
    // A quiet hour may skip the part only when every receiver it holds is
    // watched: a writer of an unwatched one moves the revision, but the
    // refused receiver must still be read afresh every hour.
    certificates.quiet = quiet.filter(|_| observed);
    certificates.complete = true;
    Ok(State::Copied)
}

/// Room for `len` elements, allocated fallibly.
fn reserve<T>(vector: &mut Vec<T>, len: usize) -> Result<(), &'static str> {
    vector
        .try_reserve(len.saturating_sub(vector.len()))
        .map_err(|_| "air relation staging allocation failed")
}

/// Up to [`native::BATCH`] countries awaiting their headers.
struct Pending {
    /// The live country-link count, which bounds every vector.
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

    /// Read the pending countries' headers, taking each diplomacy sequence
    /// before its status is read, and give each its place in the buffer.
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

/// One country's vectors as last applied.
#[derive(Clone, Default)]
struct HeldCountry {
    /// Zero for an empty slot.
    country: usize,
    counts: [u32; 3],
    entries: Vec<u32>,
    /// Moves whenever the entries or country change.
    generation: u64,
}

impl HeldCountry {
    /// Whether these are a staged country's vectors, `bytes` its entries.
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

/// The decode thread's rows, applied in hour order, and the family's
/// fingerprints.
pub(crate) struct Held {
    countries: Vec<HeldCountry>,
    /// The last generation handed out, unique across slots.
    generation: u64,
    fingerprints: Option<Box<SliceFingerprints>>,
}

static HELD: Mutex<Held> = Mutex::new(Held::new());

impl Held {
    pub(crate) const fn new() -> Self {
        Self {
            countries: Vec::new(),
            generation: 0,
            fingerprints: None,
        }
    }

    /// Rows held.
    #[cfg(test)]
    pub(crate) fn rows(&self) -> usize {
        self.countries.iter().map(|country| country.entries.len()).sum()
    }

    fn apply(&mut self, part: &Staging) {
        let slots = part.slots as usize;
        if self.countries.len() != slots {
            self.countries.resize(slots, HeldCountry::default());
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
            country.entries.extend(
                bytes
                    .chunks_exact(4)
                    .map(|entry| u32::from_le_bytes(entry.try_into().unwrap())),
            );
            self.generation += 1;
            country.generation = self.generation;
        }
    }

    /// `relation_verify`: compare each part's fresh copy with the held rows
    /// after this hour's copies were applied.
    #[cfg(feature = "relation_verify")]
    fn verify(&self, parts: &[Staging]) -> relation_changes::Verification {
        let mut verification = relation_changes::Verification::default();
        let empty = HeldCountry::default();
        for (part, staging) in parts.iter().enumerate() {
            let Some(fresh) = staging.verify.as_deref().map(|verify| &verify.staging) else {
                continue;
            };
            if fresh.state != State::Copied {
                verification.unavailable_parts += 1;
                continue;
            }
            // Held slots of this part that the fresh copy found live.
            let mut covered = 0;
            for staged in &fresh.staged {
                let held = self.countries.get(staged.slot as usize).unwrap_or(&empty);
                covered += u64::from(held.country != 0);
                let bytes = &fresh.bytes[staged.offset..staged.offset + staged.bytes()];
                verification.mismatches += u64::from(!held.holds(staged, bytes));
            }
            let live = self
                .countries
                .iter()
                .skip(part)
                .step_by(PARTS)
                .filter(|country| country.country != 0)
                .count() as u64;
            verification.mismatches += live.saturating_sub(covered);
            verification.countries += fresh.staged.len() as u64;
            verification.ticks += fresh.ticks;
        }
        verification
    }

    /// Expand the selected countries' rows into the pack's buffer in slot
    /// order, growing it here on the decode thread when this hour needs more.
    fn publish(&self, selection: &SliceSelection, output: &mut Vec<native::Row>) -> Outcome {
        let selected = || {
            self.countries
                .iter()
                .enumerate()
                .filter(|(slot, country)| country.country != 0 && selection.contains(*slot as u32))
        };
        let needed = selected().map(|(_, country)| country.entries.len()).sum();
        crate::table_capture::grow_vec(output, needed);
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

/// Bench and test access to one hour outside the frame pool: stage every
/// part (in parallel when `threads`), then decode into `pack`.
#[cfg(test)]
pub(crate) mod hour {
    use super::*;

    pub(crate) struct Hour {
        pub pack: Captured,
        pub certificates: Vec<Certificates>,
        pub held: Held,
        /// Make the next hour a full hour, as the spool's recovery does;
        /// hour 0 always is one.
        pub force_full: bool,
    }

    impl Hour {
        pub(crate) fn new(rows: usize, bytes: usize) -> Self {
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

        /// Stage, returning the wall time of the parts, then decode.
        pub(crate) fn run<M: Memory + Sync>(
            &mut self,
            memory: &M,
            base: usize,
            game_state: usize,
            changes: &(impl Changes + Sync),
            hour: i32,
            threads: bool,
        ) -> std::time::Duration {
            let force_full = std::mem::take(&mut self.force_full) || hour == 0;
            self.pack.selection.reset(hour, force_full);
            let started = std::time::Instant::now();
            let parts = self.pack.parts.iter_mut().zip(&mut self.certificates);
            if threads {
                std::thread::scope(|scope| {
                    for (part, (staging, certificates)) in parts.enumerate() {
                        scope.spawn(move || {
                            stage(memory, base, game_state, part, changes, certificates, staging)
                        });
                    }
                });
            } else {
                for (part, (staging, certificates)) in parts.enumerate() {
                    stage(memory, base, game_state, part, changes, certificates, staging);
                }
            }
            let wall = started.elapsed();
            self.pack.decode_into(&mut self.held);
            wall
        }

        pub(crate) fn emitted(&self) -> Outcome {
            self.pack.rows.outcome
        }

        /// The rows this hour emitted, in slot order.
        pub(crate) fn emitted_rows(&self) -> &[native::Row] {
            &self.pack.rows.rows[..self.pack.rows.outcome.len()]
        }

        #[cfg(feature = "relation_verify")]
        pub(crate) fn verification(&self) -> relation_changes::Verification {
            self.pack.verification
        }

        pub(crate) fn staged_countries(&self) -> usize {
            self.pack.parts.iter().map(|part| part.staged.len()).sum()
        }

        /// Grow the buffers as the spool would.
        pub(crate) fn grow(&mut self) {
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
