//! AirNavalPatrolHistories: every country's nonzero window entries in the
//! regions its naval-patrol combats are in, `141237D20`'s second input.
//!
//! The histories' native writers are observed ([`crate::history_changes`]).
//! The AirMissions job, which captures the combats, opens the hour
//! ([`supply`]): every part gets the patrolled regions and one quiet and
//! site verdict against the last complete hour. Then [`PARTS`] worker-only
//! pool jobs, each owning every [`PARTS`]th slot, only copy ([`stage`]): a
//! quiet hour with the same regions and country air objects copies nothing.
//! Otherwise a country whose air object a writer touched, or whose object
//! changed, is copied afresh. Every add and every new history goes through
//! `140C40870`, which marks the country, so a history of an unmarked country
//! changed only by the advances its own hook counted: those are staged as
//! counts, without a read. A history whose count is unobserved is copied on
//! its own, and a newly wanted region is copied for every country. A writer
//! site with no calls since skips its receivers' checks. All reads are
//! gathers batched across countries and histories.
//!
//! An hour is certified only when every part completed; the game thread
//! then settles it ([`Captured::settle`]), and a failed part withdraws every
//! part's certificates. The decode thread applies all of an hour's copies
//! and advances to the held histories in hour order, or none of them, as
//! `1415050B0` advances (move the current slot, wrapping, and zero the
//! record it moves to), and emits the table by slice and on change
//! ([`crate::slicing`]). An advance shifts every summation index, so a
//! country holding histories is emitted again on the day they advance.
//! `history_verify` also copies everything and disables reuse on a
//! difference.

use crate::{
    history_changes::{self, Changes},
    sampling::qpc,
    slicing::{SliceFingerprints, SliceSelection},
    spooling::table,
    table_capture::{CaptureRows, grow_capacity},
};
use engine::{
    Memory,
    air_kamikaze::{self as native, WindowState},
    allocation, ccountry,
    walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{AirNavalPatrolHistoryRow, TableId, tables},
};
use std::sync::{Mutex, MutexGuard};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::AirNavalPatrolHistories];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

/// Pool jobs splitting the copies; slot `s` belongs to part `s % PARTS`.
pub const PARTS: usize = 8;

/// A notification `(epoch, count)`; `None` when unobserved.
type Sequence = Option<(u64, u64)>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    /// Countries reused and copied afresh.
    pub reused: u64,
    pub read: u64,
    /// Histories advanced without a read, and copied on their own.
    pub advanced: u64,
    pub refreshed: u64,
    /// Every part reused everything.
    pub quiet: bool,
}

impl Counts {
    fn add(&mut self, part: &Self) {
        self.reused += part.reused;
        self.read += part.read;
        self.advanced += part.advanced;
        self.refreshed += part.refreshed;
        self.quiet &= part.quiet;
    }
}

/// Buffer sizes for one part's copies and certificates: its slots from the
/// registry, the rest grown off-thread from what the hours used. A copy
/// larger than any before still grows in its hour rather than failing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    /// Slots one part owns.
    countries: usize,
    /// Histories one part copied or advanced in one hour.
    histories: usize,
    /// Record channels one part copied in one hour.
    channels: usize,
    /// Wanted regions.
    regions: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 256,
            countries: 128,
            histories: 128,
            channels: 4 * 1024,
            regions: 16,
        }
    }
}

impl Capacities {
    const VACANT: Self = Self {
        rows: 0,
        countries: 0,
        histories: 0,
        channels: 0,
        regions: 0,
    };

    pub fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(
            &mut self.countries,
            crate::country_scale::rows(slots.div_ceil(PARTS), 1),
        );
    }

    /// Raise each capacity to a third above what one part used in an hour.
    fn observe(&mut self, staging: &Staging) {
        for (capacity, used) in [
            (&mut self.countries, staging.owned),
            (
                &mut self.histories,
                staging.windows.len().max(staging.advances.len()),
            ),
            (&mut self.channels, staging.channels.len()),
            (&mut self.regions, staging.regions.len()),
        ] {
            grow_capacity(capacity, used.saturating_mul(4).div_ceil(3));
        }
    }
}

/// Raise `list`'s capacity to `capacity`.
fn reserve<T>(list: &mut Vec<T>, capacity: usize) -> Result<(), &'static str> {
    let additional = capacity.saturating_sub(list.len());
    allocation::reserve(list, additional).map_err(|_| "collector allocation failed")
}

fn reserve_pushes<T>(list: &mut Vec<T>, count: usize) -> Result<(), &'static str> {
    allocation::reserve_push_batch(list, count).map_err(|_| "collector allocation failed")
}

fn push<T>(list: &mut Vec<T>, value: T) -> Result<(), &'static str> {
    if list.len() == list.capacity() {
        reserve_pushes(list, 1)?;
    }
    list.push(value);
    Ok(())
}

/// One copied window, its channels at `offset` in the staged channels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Window {
    slot: u32,
    region: u32,
    state: WindowState,
    offset: usize,
}

/// The hour's verdicts against the last complete hour, the same for every
/// part.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Verdict {
    /// The last complete hour's certificates hold.
    complete: bool,
    /// Besides: no observed writer ran and the regions are the same.
    quiet: bool,
    /// Besides: each writer site made no call.
    sites: [bool; 2],
    regions_changed: bool,
}

/// What the AirMissions job left a part.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Input {
    #[default]
    Missing,
    Supplied,
    Failed(&'static str),
}

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

/// One part's copies for one hour: its regions from the AirMissions job,
/// then loaned to its own job and read by the decode thread. Buffers keep
/// their capacity and grow off-thread.
#[derive(Default)]
pub(crate) struct Staging {
    /// This hour's wanted regions, sorted, and those the last complete
    /// hour did not want.
    regions: Vec<u64>,
    added: Vec<u64>,
    verdict: Verdict,
    input: Input,
    state: State,
    /// The manager's country slots, and those this part owns.
    slots: usize,
    owned: usize,
    /// Countries whose held histories are replaced by this hour's windows.
    replaced: Vec<u32>,
    /// Slots whose country air object is gone.
    removed: Vec<u32>,
    /// `(slot, region, advances)` applied without a read.
    advances: Vec<(u32, u32, u64)>,
    windows: Vec<Window>,
    channels: Vec<[i64; 2]>,
    counts: Counts,
    ticks: u64,
    /// What the job presizes its certificates to.
    capacity: Option<Capacities>,
    /// A fresh copy of the part, for `history_verify`.
    verify: Option<Box<Staging>>,
}

impl Staging {
    fn with_capacities(capacity: &Capacities) -> Self {
        let mut staging = Self::default();
        let _ = staging.reserve(capacity);
        staging
    }

    fn reserve(&mut self, capacity: &Capacities) -> Result<(), &'static str> {
        reserve(&mut self.regions, capacity.regions)?;
        reserve(&mut self.added, capacity.regions)?;
        reserve(&mut self.replaced, capacity.countries)?;
        reserve(&mut self.removed, capacity.countries)?;
        reserve(&mut self.advances, capacity.histories)?;
        reserve(&mut self.windows, capacity.histories)?;
        reserve(&mut self.channels, capacity.channels)?;
        self.capacity = Some(*capacity);
        Ok(())
    }

    /// A new frame: no regions yet.
    fn clear(&mut self) {
        self.regions.clear();
        self.added.clear();
        self.verdict = Verdict::default();
        self.input = Input::Missing;
        self.clear_copies();
    }

    fn clear_copies(&mut self) {
        self.state = State::Idle;
        self.slots = 0;
        self.owned = 0;
        self.replaced.clear();
        self.removed.clear();
        self.advances.clear();
        self.windows.clear();
        self.channels.clear();
        self.counts = Counts::default();
        self.ticks = 0;
        if let Some(verify) = &mut self.verify {
            verify.clear_copies();
        }
    }

    /// The AirMissions job's regions and verdict; fails only when the
    /// buffers cannot grow.
    fn open(
        &mut self,
        regions: &[u64],
        held: &[u64],
        verdict: Verdict,
    ) -> Result<(), &'static str> {
        self.regions.clear();
        self.added.clear();
        reserve(&mut self.regions, regions.len())?;
        reserve(&mut self.added, regions.len())?;
        self.regions.extend_from_slice(regions);
        self.added.extend(
            regions
                .iter()
                .filter(|region| held.binary_search(region).is_err()),
        );
        self.verdict = verdict;
        Ok(())
    }

    /// A failed job: nothing it staged may be applied.
    pub(crate) fn fail(&mut self, reason: &'static str) {
        self.state = State::Failed(reason);
    }

    fn completed(&self) -> bool {
        matches!(self.state, State::Quiet | State::Copied)
    }

    fn allocated_bytes(&self) -> usize {
        (self.regions.capacity() + self.added.capacity()) * 8
            + (self.replaced.capacity() + self.removed.capacity()) * 4
            + self.advances.capacity() * size_of::<(u32, u32, u64)>()
            + self.windows.capacity() * size_of::<Window>()
            + self.channels.capacity() * 16
            + self
                .verify
                .as_ref()
                .map_or(0, |verify| verify.allocated_bytes())
    }

    /// The copied windows of `histories`, each found at `(slot, region)`.
    fn copy_windows(
        &mut self,
        memory: &impl Memory,
        base: usize,
        histories: &[usize],
        found: &[(u32, u32)],
        scratch: &mut native::WindowScratch,
    ) -> Result<(), &'static str> {
        let Self {
            windows, channels, ..
        } = self;
        reserve_pushes(windows, histories.len())?;
        native::history_windows(memory, base, histories, scratch, |at, state, copied| {
            let (slot, region) = found[at];
            let count = channels
                .len()
                .checked_add(copied.len())
                .ok_or("air history channel count overflow")?;
            reserve(channels, count)?;
            windows.push(Window {
                slot,
                region,
                state,
                offset: channels.len(),
            });
            channels.extend_from_slice(copied);
            Ok(())
        })
    }
}

/// The last opened hour's regions and whole-domain certificates, which
/// every part shares. Only the AirMissions job and the game thread, after
/// the hour's jobs joined, use it.
pub(crate) struct Opening {
    /// The hour they belong to completed in every part and was not dropped.
    complete: bool,
    /// An hour was opened since attaching.
    opened: bool,
    quiet: Sequence,
    /// Per writer site, taken with it.
    sites: [Sequence; 2],
    regions: Vec<u64>,
}

impl Opening {
    pub(crate) const fn new() -> Self {
        Self {
            complete: false,
            opened: false,
            quiet: None,
            sites: [None; 2],
            regions: Vec::new(),
        }
    }

    /// Nothing observed so far certifies anything: every part copies afresh.
    pub(crate) fn forget(&mut self) {
        self.complete = false;
    }

    /// The last opened hour failed or was dropped, so the next copies
    /// every country afresh.
    pub(crate) fn withdrawn(&self) -> bool {
        self.opened && !self.complete
    }
}

static OPENING: Mutex<Opening> = Mutex::new(Opening::new());

pub(crate) fn opening() -> MutexGuard<'static, Opening> {
    OPENING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The AirMissions job opens the hour: every part gets the patrolled
/// `regions` (sorted, distinct IDs in the low 32 bits), or why there are
/// none, and the verdicts against the last complete hour. The opening then
/// holds this hour, uncertified until [`Captured::settle`].
pub(crate) fn supply(
    parts: &mut [Staging],
    regions: Result<&[u64], &'static str>,
    changes: &impl Changes,
    opening: &mut Opening,
) {
    let regions = match regions {
        Ok(regions) => regions,
        Err(reason) => {
            for part in parts {
                part.input = Input::Failed(reason);
            }
            return;
        }
    };
    if parts.is_empty() {
        return;
    }
    if reserve(&mut opening.regions, regions.len()).is_err() {
        opening.complete = false;
        for part in parts {
            part.input = Input::Failed("collector allocation failed");
        }
        return;
    }
    let quiet = changes.quiet();
    let sites = [changes.site(0), changes.site(1)];
    let complete = std::mem::replace(&mut opening.complete, false);
    // A site with no calls since touched none of its receivers.
    let site =
        |index: usize| complete && sites[index].is_some() && sites[index] == opening.sites[index];
    let verdict = Verdict {
        complete,
        quiet: complete && quiet.is_some() && quiet == opening.quiet && regions == opening.regions,
        sites: [site(0), site(1)],
        regions_changed: regions != opening.regions,
    };
    for part in parts {
        part.input = match part.open(regions, &opening.regions, verdict) {
            Ok(()) => Input::Supplied,
            Err(reason) => Input::Failed(reason),
        };
    }
    opening.opened = true;
    opening.quiet = quiet;
    opening.sites = sites;
    opening.regions.clear();
    opening.regions.extend_from_slice(regions);
}

/// What a part's jobs last observed of one history.
#[derive(Clone, Copy, Default)]
struct HistoryCertificate {
    region: u32,
    history: usize,
    sequence: Sequence,
}

#[derive(Default)]
struct CountryCertificate {
    root: usize,
    sequence: Sequence,
    /// Non-null histories in the wanted regions, by region.
    histories: Vec<HistoryCertificate>,
}

impl CountryCertificate {
    /// Keeps the histories' capacity.
    fn reset(&mut self, root: usize, sequence: Sequence) {
        self.root = root;
        self.sequence = sequence;
        self.histories.clear();
    }
}

/// One part's certificates, valid while the [`Opening`] is complete.
pub(crate) struct Certificates {
    /// By slot / [`PARTS`].
    roots: Vec<usize>,
    /// By slot / [`PARTS`]; entries past `roots` are empty and presized.
    countries: Vec<CountryCertificate>,
    /// What the lists were last presized to.
    reserved: Capacities,
    scratch: Scratch,
}

/// Per-hour work lists, kept for their capacity. Countries are indices
/// into the part's certificates.
#[derive(Default)]
struct Scratch {
    roots: Vec<usize>,
    read: Vec<usize>,
    extend: Vec<usize>,
    targets: Vec<usize>,
    doubted: Vec<(usize, usize, Sequence)>,
    addresses: Vec<usize>,
    found: Vec<(u32, u32)>,
    sequences: Vec<Sequence>,
    windows: native::WindowScratch,
}

impl Certificates {
    pub(crate) const fn new() -> Self {
        Self {
            roots: Vec::new(),
            countries: Vec::new(),
            reserved: Capacities::VACANT,
            scratch: Scratch {
                roots: Vec::new(),
                read: Vec::new(),
                extend: Vec::new(),
                targets: Vec::new(),
                doubted: Vec::new(),
                addresses: Vec::new(),
                found: Vec::new(),
                sequences: Vec::new(),
                windows: native::WindowScratch::new(),
            },
        }
    }

    /// Presize every list the part's job fills; a country's histories grow
    /// when it first holds one. In the job, which owns them, once per
    /// capacity change.
    fn reserve(&mut self, capacity: &Capacities) -> Result<(), &'static str> {
        if self.reserved == *capacity {
            return Ok(());
        }
        let Self {
            roots,
            countries,
            scratch,
            ..
        } = self;
        for list in [
            roots,
            &mut scratch.roots,
            &mut scratch.read,
            &mut scratch.extend,
            &mut scratch.targets,
        ] {
            reserve(list, capacity.countries)?;
        }
        reserve(&mut scratch.addresses, capacity.histories)?;
        reserve(&mut scratch.doubted, capacity.histories)?;
        reserve(&mut scratch.found, capacity.histories)?;
        reserve(&mut scratch.sequences, capacity.histories)?;
        if countries.len() < capacity.countries {
            reserve(countries, capacity.countries)?;
            countries.resize_with(capacity.countries, Default::default);
        }
        scratch
            .windows
            .size()
            .map_err(|_| "collector allocation failed")?;
        self.reserved = *capacity;
        Ok(())
    }
}

static CERTIFICATES: [Mutex<Certificates>; PARTS] =
    [const { Mutex::new(Certificates::new()) }; PARTS];

/// Only the part's job locks them.
pub(crate) fn certificates(part: usize) -> MutexGuard<'static, Certificates> {
    CERTIFICATES[part]
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Nothing observed so far certifies anything: every part copies afresh.
pub(crate) fn forget_certificates() {
    opening().forget();
}

/// The pool refused the part's job, which never runs on the game thread: it
/// fails without a read, leaving the hour uncertified.
pub(crate) fn refuse(staging: &mut Staging) {
    staging.fail("naval-patrol history worker admission failed");
}

/// One part's hour: copy what changed since the last complete hour into
/// `staging`. A failure leaves the hour uncertified ([`Captured::settle`]).
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
    staging.clear_copies();
    staging.state = match staging.input {
        Input::Missing => State::Failed("naval-patrol history regions not supplied"),
        Input::Failed(reason) => State::Failed(reason),
        Input::Supplied => {
            if let Some(capacity) = staging.capacity
                && let Err(reason) = certificates.reserve(&capacity)
            {
                staging.fail(reason);
                return;
            }
            let (regions, added) = (
                std::mem::take(&mut staging.regions),
                std::mem::take(&mut staging.added),
            );
            let state = stage_part(
                memory,
                base,
                game_state,
                part,
                (&regions, &added, staging.verdict),
                changes,
                certificates,
                staging,
            );
            (staging.regions, staging.added) = (regions, added);
            state.unwrap_or_else(State::Failed)
        }
    };
    staging.ticks = qpc().saturating_sub(begin).max(0) as u64;
    if cfg!(feature = "history_verify")
        && staging.input == Input::Supplied
        && !allocation::capture_stopped()
    {
        let Staging {
            regions, verify, ..
        } = staging;
        let verify = verify.get_or_insert_with(Box::default);
        verify.clear_copies();
        let mut fresh = Certificates::new();
        verify.state = stage_part(
            memory,
            base,
            game_state,
            part,
            (regions, &[], Verdict::default()),
            &history_changes::Unobserved,
            &mut fresh,
            verify,
        )
        .unwrap_or_else(State::Failed);
    }
}

#[allow(clippy::too_many_arguments)]
fn stage_part(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    part: usize,
    (regions, added, verdict): (&[u64], &[u64], Verdict),
    changes: &impl Changes,
    certificates: &mut Certificates,
    staging: &mut Staging,
) -> Result<State, &'static str> {
    let Certificates {
        roots,
        countries,
        scratch,
        ..
    } = certificates;
    staging.slots =
        native::country_air_roots(memory, base, game_state, part, PARTS, &mut scratch.roots)?;
    staging.owned = scratch.roots.len();
    // A quiet hour may skip the part only when every receiver it holds is
    // watched: a refused one must still be read afresh every hour.
    let watched = || {
        roots.iter().zip(countries.iter()).all(|(&root, country)| {
            root == 0
                || (country.sequence.is_some()
                    && country
                        .histories
                        .iter()
                        .all(|history| history.sequence.is_some()))
        })
    };
    if verdict.quiet && scratch.roots == *roots && watched() {
        staging.counts = Counts {
            reused: roots.iter().filter(|&&root| root != 0).count() as u64,
            quiet: true,
            ..Counts::default()
        };
        return Ok(State::Quiet);
    }
    let slot = |index: usize| (part + index * PARTS) as u32;
    let [roots_quiet, histories_quiet] = verdict.sites;
    std::mem::swap(roots, &mut scratch.roots);
    if countries.len() < roots.len() {
        reserve(countries, roots.len())?;
        countries.resize_with(roots.len(), Default::default);
    }
    // Slots the registry no longer has certify nothing if it grows again.
    for country in countries
        .iter_mut()
        .take(scratch.roots.len())
        .skip(roots.len())
    {
        country.reset(0, None);
    }
    for list in [&mut scratch.read, &mut scratch.extend] {
        list.clear();
    }
    scratch.doubted.clear();
    let removed = roots
        .iter()
        .zip(countries.iter())
        .filter(|(root, country)| **root == 0 && (country.root != 0 || !verdict.complete))
        .count();
    reserve_pushes(&mut staging.removed, removed)?;
    let counts = &mut staging.counts;
    for (index, (&root, country)) in roots.iter().zip(countries.iter_mut()).enumerate() {
        if root == 0 {
            if country.root != 0 || !verdict.complete {
                staging.removed.push(slot(index));
            }
            country.reset(0, None);
            continue;
        }
        let sequence = if roots_quiet && country.root == root {
            country.sequence
        } else {
            changes.sequence(root)
        };
        // A changed air object is copied afresh.
        if !(verdict.complete
            && country.root == root
            && sequence.is_some()
            && sequence == country.sequence)
        {
            country.reset(root, sequence);
            push(&mut scratch.read, index)?;
            continue;
        }
        counts.reused += 1;
        if verdict.regions_changed {
            country
                .histories
                .retain(|history| regions.binary_search(&u64::from(history.region)).is_ok());
            if !added.is_empty() {
                push(&mut scratch.extend, index)?;
            }
        }
        for (at, history) in country.histories.iter_mut().enumerate() {
            // A quiet site proves a watched history unchanged; a refused one
            // is still read afresh every hour.
            if histories_quiet && history.sequence.is_some() {
                continue;
            }
            let now = changes.sequence(history.history);
            match (history.sequence, now) {
                _ if now.is_some() && now == history.sequence => {}
                // Each call counts twice: entry and exit.
                (Some((epoch, old)), Some((current, new)))
                    if epoch == current && new > old && (new - old) % 2 == 0 =>
                {
                    push(
                        &mut staging.advances,
                        (slot(index), history.region, (new - old) / 2),
                    )?;
                    history.sequence = now;
                    counts.advanced += 1;
                }
                _ => push(&mut scratch.doubted, (index, at, now))?,
            }
        }
    }
    counts.refreshed = scratch.doubted.len() as u64;
    counts.read = scratch.read.len() as u64;
    reserve(&mut staging.replaced, scratch.read.len())?;
    staging
        .replaced
        .extend(scratch.read.iter().map(|&index| slot(index)));
    // Histories whose count was not observed, copied on their own.
    scratch.addresses.clear();
    scratch.found.clear();
    reserve_pushes(&mut scratch.addresses, scratch.doubted.len())?;
    reserve_pushes(&mut scratch.found, scratch.doubted.len())?;
    for &(index, at, sequence) in &scratch.doubted {
        let history = &mut countries[index].histories[at];
        history.sequence = sequence;
        scratch.addresses.push(history.history);
        scratch.found.push((slot(index), history.region));
    }
    staging.copy_windows(
        memory,
        base,
        &scratch.addresses,
        &scratch.found,
        &mut scratch.windows,
    )?;
    // Countries copied afresh, authenticated first, in every wanted region;
    // unchanged ones in the newly wanted regions.
    for (list, wanted, fresh) in [
        (&scratch.read, regions, true),
        (&scratch.extend, added, false),
    ] {
        scratch.targets.clear();
        reserve(&mut scratch.targets, list.len())?;
        scratch
            .targets
            .extend(list.iter().map(|&index| roots[index]));
        if fresh && !list.is_empty() {
            let readers = ccountry::Reader::new(memory, base, game_state)?;
            native::authenticate_roots(memory, base, &readers, &scratch.targets, |at| {
                slot(list[at])
            })?;
        }
        scratch.addresses.clear();
        scratch.found.clear();
        scratch.sequences.clear();
        let (addresses, found, sequences) = (
            &mut scratch.addresses,
            &mut scratch.found,
            &mut scratch.sequences,
        );
        native::region_slots(memory, &scratch.targets, wanted, |at, region, history| {
            push(sequences, changes.sequence(history))?;
            push(addresses, history)?;
            push(found, (slot(list[at]), region))?;
            Ok(())
        })?;
        staging.copy_windows(
            memory,
            base,
            &scratch.addresses,
            &scratch.found,
            &mut scratch.windows,
        )?;
        for found in scratch.found.chunk_by(|a, b| a.0 == b.0) {
            reserve_pushes(
                &mut countries[found[0].0 as usize / PARTS].histories,
                found.len(),
            )?;
        }
        for ((&(slot, region), &history), &sequence) in scratch
            .found
            .iter()
            .zip(&scratch.addresses)
            .zip(&scratch.sequences)
        {
            countries[slot as usize / PARTS]
                .histories
                .push(HistoryCertificate {
                    region,
                    history,
                    sequence,
                });
        }
    }
    for &index in &scratch.extend {
        countries[index]
            .histories
            .sort_unstable_by_key(|history| history.region);
    }
    Ok(State::Copied)
}

pub struct Captured {
    rows: CaptureRows<AirNavalPatrolHistoryRow>,
    /// The countries this hour's rows cover.
    selection: SliceSelection,
    parts: Vec<Staging>,
    /// The parts ran for this frame and their copies await decoding.
    staged: bool,
}

impl Captured {
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            selection: SliceSelection::new(capacity.rows != 0),
            parts: (0..PARTS)
                .map(|_| Staging::with_capacities(capacity))
                .collect(),
            staged: false,
        }
    }

    /// Game thread, at the start of a frame's hour: its last hour's copies
    /// were decoded or dropped before the frame was freed.
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

    /// Spool thread: grow from this hour's copies, then size the watches
    /// for every air object and a history in each wanted region.
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.grow_parts(capacity);
        history_changes::observe(self.receivers());
    }

    fn grow_parts(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        for part in &self.parts {
            capacity.observe(part);
        }
        for part in &mut self.parts {
            if part.reserve(capacity).is_err() {
                return;
            }
        }
    }

    fn receivers(&self) -> usize {
        let slots = self.parts.iter().map(|part| part.slots).max().unwrap_or(0);
        let regions = self.parts.first().map_or(0, |part| part.regions.len());
        slots.saturating_mul(regions + 1)
    }

    /// Start a frame's selection at its hour.
    pub(crate) fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        self.selection.reset(hour, force_full);
    }

    /// The countries whose rows this frame emitted.
    pub(crate) fn selection(&self) -> &SliceSelection {
        &self.selection
    }

    /// Loan every part to the AirMissions job, which supplies the regions.
    pub(crate) fn take_parts(&mut self) -> Vec<Staging> {
        std::mem::take(&mut self.parts)
    }

    pub(crate) fn restore_parts(&mut self, parts: Vec<Staging>) {
        self.parts = parts;
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

    /// Game thread, once every part joined and before the next hour opens:
    /// certify the hour when every part completed, else withdraw every
    /// part's certificates.
    pub(crate) fn settle(&self, opening: &mut Opening) {
        opening.complete = self.parts.iter().all(Staging::completed);
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
            part.fail("naval-patrol history parts not dispatched");
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

    /// Apply the hour's copies to the held histories, then scan and publish
    /// its slice. Decode thread, in hour order.
    pub(crate) fn decode(&mut self) {
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
        let failure = self.parts.iter().find_map(|part| match part.state {
            State::Idle => Some("naval-patrol history part did not run"),
            State::Failed(reason) => Some(reason),
            State::Quiet | State::Copied => None,
        });
        // The hour's certificates were withdrawn: none of its copies is
        // applied, and the next hour copies afresh.
        if let Some(reason) = failure {
            self.rows.fail(Outcome::Unavailable { reason });
            self.selection.invalidate(fingerprints);
            return;
        }
        let mut counts = Counts {
            quiet: true,
            ..Counts::default()
        };
        for part in &self.parts {
            counts.add(&part.counts);
        }
        history_changes::record(&counts);
        if held.apply(self.parts.iter()).is_err() {
            self.rows.fail(Outcome::Unavailable {
                reason: "collector allocation failed",
            });
            self.selection.invalidate(fingerprints);
            return;
        }
        if cfg!(feature = "history_verify")
            && self
                .parts
                .iter()
                .all(|part| part.verify.as_deref().is_some_and(Staging::completed))
        {
            let mut copy = Held::new();
            if copy
                .apply(self.parts.iter().filter_map(|part| part.verify.as_deref()))
                .is_err()
            {
                self.rows.fail(Outcome::Unavailable {
                    reason: "collector allocation failed",
                });
                return;
            }
            if !copy.same(held) {
                history_changes::mismatch();
            }
        }
        let begin = qpc();
        for (slot, country) in held.countries.iter().enumerate() {
            if !country.histories.is_empty() {
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
        [table::<tables::AirNavalPatrolHistories, _>(
            self.rows.outcome,
            &self.rows.rows,
            Clone::clone,
        )]
    }
}

#[derive(Default, PartialEq, Eq)]
struct History {
    region: u32,
    state: WindowState,
    /// Nonzero records as (slot, channels), by slot.
    records: Vec<(u32, [i64; 2])>,
}

impl History {
    fn new(region: u32, state: WindowState, channels: &[[i64; 2]]) -> Result<Self, &'static str> {
        let count = channels
            .iter()
            .filter(|&&channels| channels != [0, 0])
            .count();
        let mut records = Vec::new();
        reserve_pushes(&mut records, count)?;
        records.extend(
            channels
                .iter()
                .enumerate()
                .filter(|(_, channels)| **channels != [0, 0])
                .map(|(slot, &channels)| (slot as u32, channels)),
        );
        Ok(Self {
            region,
            state,
            records,
        })
    }

    /// Apply `advances` calls of `1415050B0`: each moves the current slot
    /// by one, wrapping to zero and setting the wrap flag, and zeroes the
    /// record at the new current slot.
    fn advance(&mut self, advances: u64) {
        let capacity = u64::from(self.state.capacity);
        let from = u64::from(self.state.current);
        self.records.retain(|&(slot, _)| {
            // Advance `distance` (1..=capacity) is the first to land on it.
            let distance = (u64::from(slot) + capacity - from) % capacity;
            let distance = if distance == 0 { capacity } else { distance };
            distance > advances
        });
        let next = from + advances;
        self.state.current = (next % capacity) as u32;
        self.state.wrapped |= next >= capacity;
    }
}

#[derive(Default)]
struct HeldCountry {
    /// By region.
    histories: Vec<History>,
    /// Moves whenever the histories change.
    generation: u64,
}

/// The decode thread's histories, applied in hour order, and the family's
/// fingerprints.
pub(crate) struct Held {
    countries: Vec<HeldCountry>,
    generation: u64,
    fingerprints: Option<Box<SliceFingerprints>>,
    /// Per slot, whether this hour changed its histories.
    changed: Vec<bool>,
}

static HELD: Mutex<Held> = Mutex::new(Held::new());

impl Held {
    pub(crate) const fn new() -> Self {
        Self {
            countries: Vec::new(),
            generation: 0,
            fingerprints: None,
            changed: Vec::new(),
        }
    }

    /// Apply one hour's parts, every one of which completed.
    fn apply<'a>(
        &mut self,
        parts: impl Iterator<Item = &'a Staging> + Clone,
    ) -> Result<(), &'static str> {
        // An hour every part reused changes nothing, its regions included.
        let Some(copied) = parts.clone().find(|part| part.state == State::Copied) else {
            return Ok(());
        };
        let slots = parts.clone().map(|part| part.slots).max().unwrap_or(0);
        if self.countries.len() != slots {
            reserve(&mut self.countries, slots)?;
            self.countries.resize_with(slots, HeldCountry::default);
        }
        self.changed.clear();
        reserve(&mut self.changed, slots)?;
        self.changed.resize(slots, false);
        let regions = &copied.regions;
        for (country, changed) in self.countries.iter_mut().zip(&mut self.changed) {
            let before = country.histories.len();
            country
                .histories
                .retain(|history| regions.binary_search(&u64::from(history.region)).is_ok());
            *changed |= country.histories.len() != before;
        }
        for part in parts.filter(|part| part.state == State::Copied) {
            for &slot in part.removed.iter().chain(&part.replaced) {
                let country = &mut self.countries[slot as usize];
                if !country.histories.is_empty() {
                    country.histories.clear();
                    self.changed[slot as usize] = true;
                }
            }
            for &(slot, region, advances) in &part.advances {
                let histories = &mut self.countries[slot as usize].histories;
                if let Ok(at) = histories.binary_search_by_key(&region, |history| history.region) {
                    histories[at].advance(advances);
                    self.changed[slot as usize] = true;
                }
            }
            for window in &part.windows {
                let channels =
                    &part.channels[window.offset..window.offset + window.state.capacity as usize];
                let history = History::new(window.region, window.state, channels)?;
                let histories = &mut self.countries[window.slot as usize].histories;
                match histories.binary_search_by_key(&window.region, |history| history.region) {
                    Ok(at) if histories[at] == history => continue,
                    Ok(at) => histories[at] = history,
                    Err(at) => {
                        if histories.len() == histories.capacity() {
                            reserve_pushes(histories, 1)?;
                        }
                        histories.insert(at, history);
                    }
                }
                self.changed[window.slot as usize] = true;
            }
        }
        for (country, &changed) in self.countries.iter_mut().zip(&self.changed) {
            if changed {
                self.generation += 1;
                country.generation = self.generation;
            }
        }
        Ok(())
    }

    /// Whether both hold the same histories.
    fn same(&self, other: &Self) -> bool {
        self.countries
            .iter()
            .enumerate()
            .filter(|(_, country)| !country.histories.is_empty())
            .map(|(slot, country)| (slot, &country.histories))
            .eq(other
                .countries
                .iter()
                .enumerate()
                .filter(|(_, country)| !country.histories.is_empty())
                .map(|(slot, country)| (slot, &country.histories)))
    }

    /// Expand the selected countries' entries in country, region and
    /// summation order, growing the buffer here on the decode thread.
    fn publish(
        &self,
        selection: &SliceSelection,
        output: &mut Vec<AirNavalPatrolHistoryRow>,
    ) -> Outcome {
        let needed = self
            .countries
            .iter()
            .enumerate()
            .filter(|(slot, _)| selection.contains(*slot as u32))
            .flat_map(|(_, country)| &country.histories)
            .try_fold(0usize, |count, history| {
                let emitted = history
                    .state
                    .order()
                    .filter(|(_, record)| {
                        history
                            .records
                            .binary_search_by_key(&(*record as u32), |&(slot, _)| slot)
                            .is_ok()
                    })
                    .count();
                count.checked_add(emitted)
            });
        let Some(needed) = needed else {
            return Outcome::Unavailable {
                reason: "air held history row count overflow",
            };
        };
        let additional = needed.saturating_sub(output.len());
        if reserve_pushes(output, additional).is_err() {
            return Outcome::Unavailable {
                reason: "collector allocation failed",
            };
        }
        let mut len = 0;
        for (slot, country) in self.countries.iter().enumerate() {
            if !selection.contains(slot as u32) {
                continue;
            }
            for history in &country.histories {
                for (index, record) in history.state.order() {
                    let Ok(at) = history
                        .records
                        .binary_search_by_key(&(record as u32), |&(slot, _)| slot)
                    else {
                        continue;
                    };
                    let [sunk, lost] = history.records[at].1;
                    let row = AirNavalPatrolHistoryRow {
                        country: slot as u32,
                        region: history.region,
                        index,
                        enemy_ships_sunk_raw: sunk,
                        our_fighters_lost_raw: lost,
                    };
                    match output.get_mut(len) {
                        Some(slot) => *slot = row,
                        None => output.push(row),
                    }
                    len += 1;
                }
            }
        }
        Outcome::Rows { len }
    }
}

#[cfg(test)]
pub(crate) fn allocation_failure_exercise() {
    let capacity = Capacities {
        rows: 4,
        countries: 3,
        histories: 2,
        channels: 6,
        regions: 2,
    };
    let mut part = Staging::with_capacities(&capacity);
    if crate::allocation::failed() {
        return;
    }
    part.regions.extend_from_slice(&[7, 9]);
    part.channels
        .extend_from_slice(&[[1, 0], [0, 0], [2, 4], [0, 0], [3, 0], [0, 5]]);
    part.windows.extend_from_slice(&[
        Window {
            slot: 1,
            region: 7,
            state: WindowState {
                history: 0,
                capacity: 3,
                current: 2,
                wrapped: true,
            },
            offset: 0,
        },
        Window {
            slot: 2,
            region: 9,
            state: WindowState {
                history: 0,
                capacity: 3,
                current: 2,
                wrapped: true,
            },
            offset: 3,
        },
    ]);
    part.slots = 3;
    part.state = State::Copied;
    let mut held = Held::new();
    if held.apply(std::iter::once(&part)).is_err() {
        return;
    }
    let mut selection = SliceSelection::new(true);
    if crate::allocation::failed() {
        return;
    }
    selection.reset(0, true);
    let mut output = Vec::new();
    let outcome = held.publish(&selection, &mut output);
    if crate::allocation::failed() {
        return;
    }
    let _ = table::<tables::AirNavalPatrolHistories, _>(outcome, &output, Clone::clone);
    if crate::allocation::failed() {
        return;
    }
    // The next part inserts another region into an existing country's held list.
    part.windows[0].region = 9;
    part.windows.truncate(1);
    let _ = held.apply(std::iter::once(&part));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn qword(image: &mut [u8], at: usize, value: u64) {
        image[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn dword(image: &mut [u8], at: usize, value: u32) {
        image[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn vector(image: &mut [u8], at: usize, data: usize, count: u32) {
        qword(image, at, data as u64);
        dword(image, at + 8, count);
        dword(image, at + 12, count);
    }

    fn state(current: u32, wrapped: bool) -> WindowState {
        WindowState {
            history: 0x1000,
            capacity: 5,
            current,
            wrapped,
        }
    }

    #[test]
    fn an_advance_zeroes_each_new_current_record_and_wraps() {
        let channels = [[1, 0], [2, 0], [0, 0], [4, 4], [5, 0]];
        let mut history = History::new(7, state(3, false), &channels).unwrap();
        // Current 3 -> 4 -> 0: records 4 and 0 are zeroed and the window wraps.
        history.advance(2);
        assert_eq!(history.state, state(0, true));
        assert_eq!(history.records, [(1, [2, 0]), (3, [4, 4])]);
        // A whole lap zeroes every record, the old current one last.
        let mut lap = History::new(7, state(3, true), &channels).unwrap();
        lap.advance(5);
        assert_eq!(lap.state, state(3, true));
        assert!(lap.records.is_empty());
        let mut short = History::new(7, state(3, true), &channels).unwrap();
        short.advance(4);
        assert_eq!(short.records, [(3, [4, 4])]);
    }

    #[test]
    fn publish_follows_the_summation_order() {
        let channels = [[1, 0], [2, 0], [0, 0], [4, 4], [5, 0]];
        let mut held = Held::new();
        held.countries = vec![
            HeldCountry::default(),
            HeldCountry {
                histories: vec![History::new(7, state(3, true), &channels).unwrap()],
                generation: 1,
            },
        ];
        let mut selection = SliceSelection::new(true);
        selection.reset(0, true);
        let mut rows = Vec::new();
        assert_eq!(
            held.publish(&selection, &mut rows),
            Outcome::Rows { len: 4 }
        );
        let order: Vec<_> = rows
            .iter()
            .map(|row| (row.index, row.enemy_ships_sunk_raw))
            .collect();
        assert_eq!(order, [(0, 4), (1, 5), (2, 1), (3, 2)]);
    }

    #[test]
    fn reused_histories_do_not_grow_empty_copy_or_advance_lists() {
        const BASE: usize = 0x1_0000_0000;
        const GAME_STATE: usize = 0x100;
        const MANAGER: usize = 0x800;
        const ROOTS: usize = 0xa00;
        let mut image = vec![0; 0x1000];
        qword(
            &mut image,
            GAME_STATE + engine::cgame_state::STRATEGIC_AIR_MANAGER,
            MANAGER as u64,
        );
        qword(
            &mut image,
            MANAGER,
            (BASE + engine::cstrategic_air_manager::VTABLE_RVA) as u64,
        );
        vector(
            &mut image,
            MANAGER + engine::cstrategic_air_manager::COUNTRY_AIR,
            ROOTS,
            (PARTS + 1) as u32,
        );
        let mut certificates = Certificates::new();
        certificates.reserve(&Capacities::default()).unwrap();
        certificates.roots.extend_from_slice(&[0xc00, 0xc80]);
        for (index, country) in certificates.countries.iter_mut().take(2).enumerate() {
            qword(
                &mut image,
                ROOTS + index * PARTS * 8,
                (0xc00 + index * 0x80) as u64,
            );
            country.root = 0xc00 + index * 0x80;
            country.sequence = Some((1, 2));
            country.histories = (0..90)
                .map(|region| HistoryCertificate {
                    region,
                    history: 0x1000 + region as usize,
                    sequence: Some((1, 2)),
                })
                .collect();
        }
        let memory = engine::memory::SliceMemory {
            base: 0,
            bytes: &image,
        };
        for mut staging in [
            Staging::default(),
            Staging::with_capacities(&Capacities::default()),
        ] {
            let removed = staging.removed.capacity();
            let advances = staging.advances.capacity();
            assert_eq!(
                stage_part(
                    &memory,
                    BASE,
                    GAME_STATE,
                    0,
                    (
                        &[],
                        &[],
                        Verdict {
                            complete: true,
                            sites: [true; 2],
                            ..Verdict::default()
                        }
                    ),
                    &history_changes::Unobserved,
                    &mut certificates,
                    &mut staging,
                ),
                Ok(State::Copied),
            );
            assert!(staging.removed.is_empty());
            assert!(staging.advances.is_empty());
            assert_eq!(staging.removed.capacity(), removed);
            assert_eq!(staging.advances.capacity(), advances);
        }
    }

    #[test]
    fn copied_windows_and_published_rows_keep_sequential_push_growth() {
        const BASE: usize = 0x1_0000_0000;
        const HISTORY: usize = 0x100;
        const WINDOW: usize = 0x200;
        const RECORDS: usize = 0x300;
        const RECORD: usize = 0x400;
        const VALUES: usize = 0x500;
        let mut image = vec![0; 0x1000];
        qword(
            &mut image,
            HISTORY,
            (BASE + engine::cloop_history::VTABLE_RVA) as u64,
        );
        dword(
            &mut image,
            HISTORY + engine::cloop_history::FIELD_COUNT,
            engine::cloop_history::air_stats::FIELD_COUNT,
        );
        qword(
            &mut image,
            HISTORY + engine::cloop_history::DAILY_WINDOW,
            WINDOW as u64,
        );
        qword(
            &mut image,
            WINDOW,
            (BASE + engine::loop_history_window::VTABLE_RVA) as u64,
        );
        vector(
            &mut image,
            WINDOW + engine::loop_history_window::RECORDS,
            RECORDS,
            1,
        );
        dword(
            &mut image,
            WINDOW + engine::loop_history_window::CAPACITY,
            1,
        );
        image[WINDOW + engine::loop_history_window::WRAPPED] = 1;
        qword(&mut image, RECORDS, RECORD as u64);
        qword(&mut image, RECORD, VALUES as u64);
        qword(
            &mut image,
            VALUES + native::ENEMY_SHIPS_SUNK * engine::loop_history_record_fields::VALUE_STRIDE,
            1,
        );
        let memory = engine::memory::SliceMemory {
            base: 0,
            bytes: &image,
        };
        let mut staging = Staging::with_capacities(&Capacities::default());
        let histories = vec![HISTORY; 500];
        let found: Vec<_> = (0..500).map(|region| (0, region)).collect();
        let mut expected = Vec::with_capacity(staging.windows.capacity());
        staging
            .copy_windows(
                &memory,
                BASE,
                &histories,
                &found,
                &mut native::WindowScratch::new(),
            )
            .unwrap();
        for window in &staging.windows {
            expected.push(*window);
        }
        assert_eq!(staging.windows.len(), 500);
        assert_eq!(staging.windows.capacity(), expected.capacity());
        staging.slots = 1;
        staging.state = State::Copied;
        staging.regions.extend(0..500);
        let mut held = Held::new();
        held.apply(std::iter::once(&staging)).unwrap();
        let mut selection = SliceSelection::new(true);
        selection.reset(0, true);
        let mut output = vec![AirNavalPatrolHistoryRow::default(); 8];
        assert_eq!(
            held.publish(&selection, &mut output),
            Outcome::Rows { len: 500 }
        );
        let mut expected = vec![AirNavalPatrolHistoryRow::default(); 8];
        for row in &output[8..] {
            expected.push(row.clone());
        }
        assert_eq!(output.capacity(), expected.capacity());
    }

    #[test]
    fn publish_reserves_only_records_in_the_current_window() {
        let channels = [[1, 0]; 5];
        let mut held = Held::new();
        held.countries.push(HeldCountry {
            histories: vec![History::new(7, state(1, false), &channels).unwrap()],
            generation: 1,
        });
        let mut selection = SliceSelection::new(true);
        selection.reset(0, true);
        let mut rows = Vec::new();
        assert_eq!(
            held.publish(&selection, &mut rows),
            Outcome::Rows { len: 1 }
        );
        let mut expected = Vec::new();
        for row in &rows[..1] {
            expected.push(row.clone());
        }
        assert_eq!(rows.capacity(), expected.capacity());
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
                reason: "naval-patrol history parts not dispatched"
            }]
        );
    }

    #[test]
    fn missing_combats_leave_every_part_without_regions_and_the_opening_untouched() {
        let mut parts: Vec<_> = (0..PARTS).map(|_| Staging::default()).collect();
        let mut opening = Opening::new();
        opening.complete = true;
        supply(
            &mut parts,
            Err("naval-patrol histories lack their task-force combats"),
            &history_changes::Unobserved,
            &mut opening,
        );
        assert!(parts.iter().all(|part| part.input
            == Input::Failed("naval-patrol histories lack their task-force combats")));
        assert!(opening.complete);
    }
}

/// Bench and test access to hours outside the frame pool: the AirMissions
/// job's opening, every part (in parallel when `threads`), the game
/// thread's settlement and the decode, each on a frame of its own.
#[cfg(test)]
pub(crate) mod hour {
    use super::*;

    pub(crate) struct Hour {
        pub certificates: Vec<Certificates>,
        pub opening: Opening,
        pub held: Held,
    }

    /// A frame's pack, presized as the pool presizes it.
    pub(crate) fn frame(capacity: &Capacities) -> Captured {
        Captured::with_capacities(capacity)
    }

    /// Capacities presized for `slots` country slots.
    pub(crate) fn capacities(slots: usize) -> Capacities {
        let mut capacity = Capacities::default();
        capacity.raise_for_countries(slots);
        capacity
    }

    impl Hour {
        pub(crate) fn new() -> Self {
            Self {
                certificates: (0..PARTS).map(|_| Certificates::new()).collect(),
                opening: Opening::new(),
                held: Held::new(),
            }
        }

        /// The AirMissions job opens the hour on `frame`.
        pub(crate) fn supply(
            &mut self,
            frame: &mut Captured,
            regions: &[u64],
            changes: &impl Changes,
        ) {
            supply(&mut frame.parts, Ok(regions), changes, &mut self.opening);
        }

        /// Stage every part, returning their wall time.
        pub(crate) fn stage<M: Memory + Sync>(
            &mut self,
            frame: &mut Captured,
            memory: &M,
            base: usize,
            game_state: usize,
            changes: &(impl Changes + Sync),
            threads: bool,
        ) -> std::time::Duration {
            let started = std::time::Instant::now();
            let parts = frame.parts.iter_mut().zip(&mut self.certificates);
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
            frame.staged = true;
            wall
        }

        /// Stage every part but `refused`, one after another; the pool
        /// refused that one's job, so it never runs.
        pub(crate) fn stage_refusing<M: Memory>(
            &mut self,
            frame: &mut Captured,
            memory: &M,
            base: usize,
            game_state: usize,
            changes: &impl Changes,
            refused: usize,
        ) {
            let parts = frame.parts.iter_mut().zip(&mut self.certificates);
            for (part, (staging, certificates)) in parts.enumerate() {
                if part == refused {
                    refuse(staging);
                } else {
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
            frame.staged = true;
        }

        /// One part's job failed after staging.
        pub(crate) fn fail(&mut self, frame: &mut Captured, part: usize) {
            frame.parts[part].fail("naval-patrol history acquisition failed");
        }

        /// The game thread once every part joined.
        pub(crate) fn settle(&mut self, frame: &Captured) {
            frame.settle(&mut self.opening);
        }

        /// The decode thread's part for one frame, in hour order.
        pub(crate) fn decode(&mut self, frame: &mut Captured, at: i32) -> Outcome {
            frame.prepare_hour(at, at == 0);
            frame.decode_into(&mut self.held);
            frame.rows.outcome
        }

        /// One whole hour on `frame`: the parts' wall time, the decode's
        /// time and the table's outcome.
        #[allow(clippy::too_many_arguments)]
        pub(crate) fn run<M: Memory + Sync>(
            &mut self,
            frame: &mut Captured,
            memory: &M,
            base: usize,
            game_state: usize,
            regions: &[u64],
            changes: &(impl Changes + Sync),
            at: i32,
            threads: bool,
        ) -> (std::time::Duration, std::time::Duration, Outcome) {
            self.supply(frame, regions, changes);
            let wall = self.stage(frame, memory, base, game_state, changes, threads);
            self.settle(frame);
            let started = std::time::Instant::now();
            let outcome = self.decode(frame, at);
            (wall, started.elapsed(), outcome)
        }

        /// A recycle: grow `capacity` from the frame's last hour, then
        /// presize the frame. Returns the watch receivers the spool offers.
        pub(crate) fn grow(frame: &mut Captured, capacity: &mut Capacities) -> usize {
            frame.grow_parts(capacity);
            frame.receivers()
        }

        /// Bytes of every list the parts fill, the frame's included.
        pub(crate) fn allocated(&self, frame: &Captured) -> usize {
            let certificates: usize = self
                .certificates
                .iter()
                .map(|certificates| {
                    let Certificates {
                        roots,
                        countries,
                        scratch,
                        ..
                    } = certificates;
                    let words = [
                        roots.capacity(),
                        scratch.roots.capacity(),
                        scratch.read.capacity(),
                        scratch.extend.capacity(),
                        scratch.targets.capacity(),
                        scratch.addresses.capacity(),
                        scratch.found.capacity(),
                    ];
                    let histories: usize = countries
                        .iter()
                        .map(|country| country.histories.capacity())
                        .sum();
                    words.iter().sum::<usize>() * 8
                        + scratch.doubted.capacity() * size_of::<(usize, usize, Sequence)>()
                        + scratch.sequences.capacity() * size_of::<Sequence>()
                        + countries.capacity() * size_of::<CountryCertificate>()
                        + histories * size_of::<HistoryCertificate>()
                })
                .sum();
            frame.allocated_bytes() + certificates
        }

        /// The frame's counts over every part.
        pub(crate) fn counts(frame: &Captured) -> Counts {
            let mut counts = Counts {
                quiet: true,
                ..Counts::default()
            };
            for part in &frame.parts {
                counts.add(&part.counts);
            }
            counts
        }

        /// Countries the frame's parts copied afresh.
        pub(crate) fn copied(frame: &Captured) -> u64 {
            Self::counts(frame).read
        }

        /// Nonzero entries held.
        pub(crate) fn held_rows(&self) -> usize {
            let histories = self
                .held
                .countries
                .iter()
                .flat_map(|country| &country.histories);
            histories.map(|history| history.records.len()).sum()
        }

        /// Histories held after the last decode, as a fresh copy would hold them.
        pub(crate) fn same(&self, other: &Self) -> bool {
            self.held.same(&other.held)
        }
    }
}
