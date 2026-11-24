
use crate::{
    clock::qpc,
    history_changes::{self, Changes},
    slicing::{SliceFingerprints, SliceSelection},
    table_capture::{CaptureRows, grow_capacity},
    table_conversion::table,
};
use engine_air_history::{air_kamikaze as native, air_kamikaze::WindowState};
use engine_allocation as allocation;
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::ccountry;
use replay_data::{rows::AirNavalPatrolHistoryRow, rows::TableId, rows::tables};
use replay_frame::frame::TableView;
use std::sync::{
    Mutex, MutexGuard,
    atomic::{AtomicUsize, Ordering},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::AirNavalPatrolHistories];

pub const PARTS: usize = 8;

type Sequence = Option<(u64, u64)>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub advanced: u64,
    pub refreshed: u64,
    pub patched: u64,
    pub quiet: bool,
    pub scenario_refusals: [u64; native::SCENARIO_REFUSALS.len()],
    pub menu_refusals: [u64; engine_air_history::menu_history::REFUSALS.len()],
}

impl Counts {
    fn add(&mut self, part: &Self) {
        self.reused += part.reused;
        self.read += part.read;
        self.advanced += part.advanced;
        self.refreshed += part.refreshed;
        self.patched += part.patched;
        self.quiet &= part.quiet;
        for (total, count) in self
            .scenario_refusals
            .iter_mut()
            .zip(part.scenario_refusals)
        {
            *total += count;
        }
        for (total, count) in self.menu_refusals.iter_mut().zip(part.menu_refusals) {
            *total += count;
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    countries: usize,
    histories: usize,
    channels: usize,
}

collector_primitives::capacity_max!(Capacities {
    rows,
    countries,
    histories,
    channels
});

impl Capacities {
    const VACANT: Self = Self {
        rows: 0,
        countries: 0,
        histories: 0,
        channels: 0,
    };

    pub fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(
            &mut self.countries,
            crate::country_scale::rows(slots.div_ceil(PARTS), 1),
        );
    }

    fn observe(&mut self, staging: &Staging) {
        for (capacity, used) in [
            (&mut self.countries, staging.owned),
            (
                &mut self.histories,
                staging.windows.len().max(staging.advances.len()),
            ),
            (&mut self.channels, staging.channels.len()),
        ] {
            grow_capacity(capacity, used.saturating_mul(4).div_ceil(3));
        }
    }
}

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Window {
    slot: u32,
    region: u32,
    state: WindowState,
    offset: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Verdict {
    complete: bool,
    quiet: bool,
    sites: [bool; 2],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Input {
    #[default]
    Missing,
    Supplied,
}

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
    verdict: Verdict,
    input: Input,
    state: State,
    slots: usize,
    owned: usize,
    replaced: Vec<u32>,
    removed: Vec<u32>,
    advances: Vec<(u32, u32, u64)>,
    windows: Vec<Window>,
    menus: Vec<Option<engine_air_history::menu_history::History>>,
    patches: Vec<(u32, u32, engine_air_history::menu_history::Current)>,
    dropped: Vec<(u32, u32)>,
    fresh: bool,
    channels: Vec<[i64; 9]>,
    scenario_failures: Vec<Option<&'static str>>,
    counts: Counts,
    ticks: u64,
    capacity: Option<Capacities>,
}

impl Staging {
    fn with_capacities(capacity: &Capacities) -> Self {
        let mut staging = Self::default();
        let _ = staging.reserve(capacity);
        staging
    }

    fn reserve(&mut self, capacity: &Capacities) -> Result<(), &'static str> {
        reserve(&mut self.replaced, capacity.countries)?;
        reserve(&mut self.removed, capacity.countries)?;
        reserve(&mut self.advances, capacity.histories)?;
        reserve(&mut self.windows, capacity.histories)?;
        reserve(&mut self.menus, capacity.histories)?;
        reserve(&mut self.channels, capacity.channels)?;
        reserve(&mut self.scenario_failures, capacity.channels)?;
        self.capacity = Some(*capacity);
        Ok(())
    }

    fn clear(&mut self) {
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
        self.menus.clear();
        self.patches.clear();
        self.dropped.clear();
        self.fresh = false;
        self.channels.clear();
        self.scenario_failures.clear();
        self.counts = Counts::default();
        self.ticks = 0;
    }

    pub fn fail(&mut self, reason: &'static str) {
        self.state = State::Failed(reason);
    }

    fn completed(&self) -> bool {
        matches!(self.state, State::Quiet | State::Copied)
    }

    fn allocated_bytes(&self) -> usize {
        (self.replaced.capacity() + self.removed.capacity()) * 4
            + self.advances.capacity() * size_of::<(u32, u32, u64)>()
            + self.windows.capacity() * size_of::<Window>()
            + self.menus.capacity() * size_of::<Option<engine_air_history::menu_history::History>>()
            + self
                .menus
                .iter()
                .flatten()
                .map(engine_air_history::menu_history::History::allocated_bytes)
                .sum::<usize>()
            + self.patches.capacity()
                * size_of::<(u32, u32, engine_air_history::menu_history::Current)>()
            + self.dropped.capacity() * size_of::<(u32, u32)>()
            + self.channels.capacity() * 72
            + self.scenario_failures.capacity() * size_of::<Option<&'static str>>()
    }

    fn copy_windows(
        &mut self,
        memory: &impl Memory,
        base: usize,
        histories: &[usize],
        found: &[(u32, u32)],
        scratch: &mut CopyScratch<'_>,
    ) -> Result<(), &'static str> {
        let Self {
            windows,
            menus,
            channels,
            scenario_failures,
            counts,
            ..
        } = self;
        scratch.fallback.clear();
        reserve_pushes(windows, histories.len())?;
        reserve_pushes(menus, histories.len())?;
        let fallback = &mut *scratch.fallback;
        engine_air_history::menu_history::capture(
            memory,
            base,
            histories,
            &mut *scratch.menus,
            |at, menu| {
                match menu {
                    Ok(menu) => {
                        let (slot, region) = found[at];
                        let daily = menu.windows[0];
                        windows.push(Window {
                            slot,
                            region,
                            state: WindowState {
                                history: histories[at],
                                capacity: daily.capacity,
                                current: daily.current,
                                wrapped: daily.wrapped,
                            },
                            offset: channels.len(),
                        });
                        menus.push(Some(menu));
                    }
                    Err(reason) => {
                        let index = engine_air_history::menu_history::REFUSALS
                            .iter()
                            .position(|&name| name == reason)
                            .ok_or(reason)?;
                        counts.menu_refusals[index] += 1;
                        push(fallback, at)?;
                    }
                }
                Ok(())
            },
        )?;
        if scratch.fallback.is_empty() {
            return Ok(());
        }
        scratch.addresses.clear();
        reserve(scratch.addresses, scratch.fallback.len())?;
        scratch
            .addresses
            .extend(scratch.fallback.iter().map(|&at| histories[at]));
        let fallback = &*scratch.fallback;
        native::history_windows(
            memory,
            base,
            &scratch.addresses[..],
            &mut *scratch.windows,
            |at, state, copied, failures| {
                let (slot, region) = found[fallback[at]];
                let count = channels
                    .len()
                    .checked_add(copied.len())
                    .ok_or("air history channel count overflow")?;
                reserve(channels, count)?;
                reserve(scenario_failures, count)?;
                for &reason in failures.iter().flatten() {
                    let index = native::SCENARIO_REFUSALS
                        .iter()
                        .position(|&name| name == reason)
                        .unwrap();
                    counts.scenario_refusals[index] += 1;
                }
                reserve_pushes(windows, 1)?;
                reserve_pushes(menus, 1)?;
                windows.push(Window {
                    slot,
                    region,
                    state,
                    offset: channels.len(),
                });
                menus.push(None);
                channels.extend_from_slice(copied);
                scenario_failures.extend_from_slice(failures);
                Ok(())
            },
        )
    }

    fn patch(
        &mut self,
        memory: &impl Memory,
        base: usize,
        histories: &[usize],
        found: &[(u32, u32)],
        menus: &mut engine_air_history::menu_history::Scratch,
        retry: &mut Vec<usize>,
    ) -> Result<(), &'static str> {
        retry.clear();
        reserve_pushes(&mut self.patches, histories.len())?;
        let (patches, counts) = (&mut self.patches, &mut self.counts);
        engine_air_history::menu_history::capture_current(
            memory,
            base,
            histories,
            menus,
            |at, current| {
                match current {
                    Ok(current) => {
                        let (slot, region) = found[at];
                        patches.push((slot, region, current));
                        counts.patched += 1;
                    }
                    Err(_) => push(retry, at)?,
                }
                Ok(())
            },
        )
    }
}

struct CopyScratch<'a> {
    windows: &'a mut native::WindowScratch,
    menus: &'a mut engine_air_history::menu_history::Scratch,
    fallback: &'a mut Vec<usize>,
    addresses: &'a mut Vec<usize>,
}

pub struct Opening {
    complete: bool,
    opened: bool,
    quiet: Sequence,
    sites: [Sequence; 2],
    forgotten: bool,
}

impl Default for Opening {
    fn default() -> Self {
        Self::new()
    }
}

impl Opening {
    pub const fn new() -> Self {
        Self {
            complete: false,
            opened: false,
            quiet: None,
            sites: [None; 2],
            forgotten: false,
        }
    }

    pub fn forget(&mut self) {
        self.complete = false;
        self.forgotten = true;
    }

    pub fn withdrawn(&self) -> bool {
        self.opened && !self.complete
    }
}

static OPENING: Mutex<Opening> = Mutex::new(Opening::new());

pub fn opening() -> MutexGuard<'static, Opening> {
    OPENING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn supply(parts: &mut [Staging], changes: &impl Changes, opening: &mut Opening) {
    if parts.is_empty() {
        return;
    }
    let quiet = changes.quiet();
    let sites = [changes.site(0), changes.site(1)];
    let complete = std::mem::replace(&mut opening.complete, false)
        & !std::mem::replace(&mut opening.forgotten, false);
    let site =
        |index: usize| complete && sites[index].is_some() && sites[index] == opening.sites[index];
    let verdict = Verdict {
        complete,
        quiet: complete && quiet.is_some() && quiet == opening.quiet,
        sites: [site(0), site(1)],
    };
    for part in parts {
        part.verdict = verdict;
        part.input = Input::Supplied;
    }
    opening.opened = true;
    opening.quiet = quiet;
    opening.sites = sites;
}

#[derive(Clone, Copy, Default)]
struct HistoryCertificate {
    region: u32,
    history: usize,
    sequence: Sequence,
    touch: Sequence,
    menu: bool,
}

#[derive(Default)]
struct CountryCertificate {
    root: usize,
    sequence: Sequence,
    histories: Vec<HistoryCertificate>,
}

impl CountryCertificate {
    fn reset(&mut self, root: usize, sequence: Sequence) {
        self.root = root;
        self.sequence = sequence;
        self.histories.clear();
    }
}

pub struct Certificates {
    roots: Vec<usize>,
    countries: Vec<CountryCertificate>,
    reserved: Capacities,
    scratch: Scratch,
}

#[derive(Default)]
struct Scratch {
    roots: Vec<usize>,
    read: Vec<usize>,
    refresh: Vec<usize>,
    targets: Vec<usize>,
    doubted: Vec<(usize, usize, Sequence)>,
    addresses: Vec<usize>,
    found: Vec<(u32, u32)>,
    sequences: Vec<(Sequence, Sequence)>,
    listed: Vec<(usize, u32, usize)>,
    merged: Vec<HistoryCertificate>,
    owners: Vec<(usize, usize)>,
    patch_addresses: Vec<usize>,
    patch_found: Vec<(u32, u32)>,
    patch_owners: Vec<(usize, usize)>,
    retry: Vec<usize>,
    fallback: Vec<usize>,
    fallback_addresses: Vec<usize>,
    windows: native::WindowScratch,
    menus: engine_air_history::menu_history::Scratch,
}

impl Default for Certificates {
    fn default() -> Self {
        Self::new()
    }
}

impl Certificates {
    pub const fn new() -> Self {
        Self {
            roots: Vec::new(),
            countries: Vec::new(),
            reserved: Capacities::VACANT,
            scratch: Scratch {
                roots: Vec::new(),
                read: Vec::new(),
                refresh: Vec::new(),
                targets: Vec::new(),
                doubted: Vec::new(),
                addresses: Vec::new(),
                found: Vec::new(),
                sequences: Vec::new(),
                listed: Vec::new(),
                merged: Vec::new(),
                owners: Vec::new(),
                patch_addresses: Vec::new(),
                patch_found: Vec::new(),
                patch_owners: Vec::new(),
                retry: Vec::new(),
                fallback: Vec::new(),
                fallback_addresses: Vec::new(),
                windows: native::WindowScratch::new(),
                menus: engine_air_history::menu_history::Scratch::new(),
            },
        }
    }

    fn allocated_bytes(&self) -> usize {
        let scratch = &self.scratch;
        (self.roots.capacity()
            + scratch.roots.capacity()
            + scratch.read.capacity()
            + scratch.refresh.capacity()
            + scratch.targets.capacity()
            + scratch.addresses.capacity()
            + scratch.patch_addresses.capacity()
            + scratch.retry.capacity()
            + scratch.fallback.capacity()
            + scratch.fallback_addresses.capacity())
            * size_of::<usize>()
            + scratch.listed.capacity() * size_of::<(usize, u32, usize)>()
            + scratch.merged.capacity() * size_of::<HistoryCertificate>()
            + (scratch.owners.capacity() + scratch.patch_owners.capacity())
                * size_of::<(usize, usize)>()
            + scratch.patch_found.capacity() * size_of::<(u32, u32)>()
            + self.countries.capacity() * size_of::<CountryCertificate>()
            + self
                .countries
                .iter()
                .map(|country| country.histories.capacity() * size_of::<HistoryCertificate>())
                .sum::<usize>()
            + scratch.doubted.capacity() * size_of::<(usize, usize, Sequence)>()
            + scratch.found.capacity() * size_of::<(u32, u32)>()
            + scratch.sequences.capacity() * size_of::<(Sequence, Sequence)>()
            + scratch.windows.allocated_bytes()
            + scratch.menus.allocated_bytes()
    }

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
            &mut scratch.refresh,
            &mut scratch.targets,
        ] {
            reserve(list, capacity.countries)?;
        }
        for list in [
            &mut scratch.addresses,
            &mut scratch.retry,
            &mut scratch.fallback,
            &mut scratch.fallback_addresses,
        ] {
            reserve(list, capacity.histories)?;
        }
        reserve(
            &mut scratch.patch_addresses,
            capacity.histories.max(capacity.countries),
        )?;
        reserve(&mut scratch.doubted, capacity.histories)?;
        reserve(&mut scratch.found, capacity.histories)?;
        reserve(&mut scratch.patch_found, capacity.histories)?;
        reserve(&mut scratch.sequences, capacity.histories)?;
        reserve(&mut scratch.listed, capacity.histories)?;
        reserve(&mut scratch.merged, capacity.histories)?;
        reserve(&mut scratch.owners, capacity.histories)?;
        reserve(&mut scratch.patch_owners, capacity.histories)?;
        if countries.len() < capacity.countries {
            reserve(countries, capacity.countries)?;
            countries.resize_with(capacity.countries, Default::default);
        }
        self.reserved = *capacity;
        Ok(())
    }
}

static CERTIFICATE_BYTES: [AtomicUsize; PARTS] = [const { AtomicUsize::new(0) }; PARTS];
static HELD_BYTES: AtomicUsize = AtomicUsize::new(0);

pub fn retained_bytes() -> usize {
    HELD_BYTES.load(Ordering::Relaxed)
        + CERTIFICATE_BYTES
            .iter()
            .map(|bytes| bytes.load(Ordering::Relaxed))
            .sum::<usize>()
}

pub fn retained_buffers() -> impl serde::Serialize {
    crate::status::status!({
        "held_ring_bytes": HELD_BYTES.load(Ordering::Relaxed),
        "certificate_bytes": CERTIFICATE_BYTES.iter().map(|bytes| bytes.load(Ordering::Relaxed)).sum::<usize>(),
        "scope": "capacity bytes of held history rings and certificate vectors, included once in pool_bytes; excludes shared watch and fingerprint maps"
    })
}

static CERTIFICATES: [Mutex<Certificates>; PARTS] =
    [const { Mutex::new(Certificates::new()) }; PARTS];

pub fn certificates(part: usize) -> MutexGuard<'static, Certificates> {
    CERTIFICATES[part]
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn forget_certificates() {
    opening().forget();
}

pub fn refuse(staging: &mut Staging) {
    staging.fail("naval-patrol history worker admission failed");
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
    let previous_capacity = certificates.reserved;
    staging.clear_copies();
    staging.state = match staging.input {
        Input::Missing => State::Failed("air history hour not supplied"),
        Input::Supplied => {
            if let Some(capacity) = staging.capacity
                && let Err(reason) = certificates.reserve(&capacity)
            {
                staging.fail(reason);
                CERTIFICATE_BYTES[part].store(certificates.allocated_bytes(), Ordering::Relaxed);
                return;
            }
            let state = stage_part(
                memory,
                base,
                game_state,
                part,
                staging.verdict,
                changes,
                certificates,
                staging,
            );
            state.unwrap_or_else(State::Failed)
        }
    };
    staging.ticks = qpc().saturating_sub(begin).max(0) as u64;
    if staging.state != State::Quiet || certificates.reserved != previous_capacity {
        CERTIFICATE_BYTES[part].store(certificates.allocated_bytes(), Ordering::Relaxed);
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one part's read operands, its verdict and the change, certificate and staging owners are separate"
)]
fn stage_part(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    part: usize,
    verdict: Verdict,
    changes: &impl Changes,
    certificates: &mut Certificates,
    staging: &mut Staging,
) -> Result<State, &'static str> {
    enum Step {
        Keep,
        Advance(u64),
        Patch,
        Copy,
    }
    let Certificates {
        roots,
        countries,
        scratch,
        ..
    } = certificates;
    let Scratch {
        roots: live,
        read,
        refresh,
        targets,
        doubted,
        addresses,
        found,
        sequences,
        listed,
        merged,
        owners,
        patch_addresses,
        patch_found,
        patch_owners,
        retry,
        fallback,
        fallback_addresses,
        windows,
        menus,
    } = scratch;
    staging.slots = native::country_air_roots(memory, base, game_state, part, PARTS, live)?;
    staging.owned = live.len();
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
    if verdict.quiet && *live == *roots && watched() {
        staging.counts = Counts {
            reused: roots.iter().filter(|&&root| root != 0).count() as u64,
            quiet: true,
            ..Counts::default()
        };
        return Ok(State::Quiet);
    }
    staging.fresh = !verdict.complete;
    let mut copy = CopyScratch {
        windows,
        menus,
        fallback,
        addresses: fallback_addresses,
    };
    let slot = |index: usize| (part + index * PARTS) as u32;
    let [roots_quiet, histories_quiet] = verdict.sites;
    std::mem::swap(roots, live);
    if countries.len() < roots.len() {
        reserve(countries, roots.len())?;
        countries.resize_with(roots.len(), Default::default);
    }
    for country in countries.iter_mut().take(live.len()).skip(roots.len()) {
        country.reset(0, None);
    }
    read.clear();
    refresh.clear();
    doubted.clear();
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
        let held = verdict.complete && country.root == root && sequence.is_some();
        if !(held && sequence == country.sequence) {
            if held
                && country
                    .histories
                    .iter()
                    .all(|history| history.touch.is_some())
            {
                country.sequence = sequence;
                push(refresh, index)?;
            } else {
                country.reset(root, sequence);
                push(read, index)?;
            }
            continue;
        }
        counts.reused += 1;
        for (at, history) in country.histories.iter_mut().enumerate() {
            if histories_quiet && history.sequence.is_some() {
                continue;
            }
            let now = changes.sequence(history.history);
            match (history.sequence, now) {
                _ if now.is_some() && now == history.sequence => {}
                (Some((epoch, old)), Some((current, new))) if epoch == current && new > old => {
                    push(
                        &mut staging.advances,
                        (slot(index), history.region, new - old),
                    )?;
                    history.sequence = now;
                    counts.advanced += 1;
                }
                _ => push(doubted, (index, at, now))?,
            }
        }
    }
    counts.refreshed = doubted.len() as u64;
    counts.read = read.len() as u64;
    counts.reused += refresh.len() as u64;
    reserve(&mut staging.replaced, read.len())?;
    staging
        .replaced
        .extend(read.iter().map(|&index| slot(index)));
    addresses.clear();
    found.clear();
    reserve_pushes(addresses, doubted.len())?;
    reserve_pushes(found, doubted.len())?;
    for &(index, at, sequence) in doubted.iter() {
        let history = &mut countries[index].histories[at];
        history.sequence = sequence;
        addresses.push(history.history);
        found.push((slot(index), history.region));
    }
    staging.copy_windows(memory, base, addresses, found, &mut copy)?;
    for (copied, &(index, at, _)) in doubted.iter().enumerate() {
        countries[index].histories[at].menu = copy.fallback.binary_search(&copied).is_err();
    }
    targets.clear();
    reserve(targets, read.len())?;
    targets.extend(read.iter().map(|&index| roots[index]));
    if !read.is_empty() || !refresh.is_empty() {
        let readers = ccountry::Reader::new(memory, base, game_state)?;
        native::authenticate_roots(memory, base, &readers, targets, |at| slot(read[at]))?;
        let refreshed = &mut *patch_addresses;
        refreshed.clear();
        reserve(refreshed, refresh.len())?;
        refreshed.extend(refresh.iter().map(|&index| roots[index]));
        native::authenticate_roots(memory, base, &readers, refreshed, |at| slot(refresh[at]))?;
    }
    addresses.clear();
    found.clear();
    sequences.clear();
    native::region_slots(memory, targets, |at, region, history| {
        push(
            sequences,
            (
                changes.sequence(history),
                changes.sequence(history_changes::touch_key(targets[at], region)),
            ),
        )?;
        push(addresses, history)?;
        push(found, (slot(read[at]), region))?;
        Ok(())
    })?;
    staging.copy_windows(memory, base, addresses, found, &mut copy)?;
    for found in found.chunk_by(|a, b| a.0 == b.0) {
        reserve_pushes(
            &mut countries[found[0].0 as usize / PARTS].histories,
            found.len(),
        )?;
    }
    for (copied, ((&(slot, region), &history), &(sequence, touch))) in found
        .iter()
        .zip(addresses.iter())
        .zip(sequences.iter())
        .enumerate()
    {
        countries[slot as usize / PARTS]
            .histories
            .push(HistoryCertificate {
                region,
                history,
                sequence,
                touch,
                menu: copy.fallback.binary_search(&copied).is_err(),
            });
    }
    if refresh.is_empty() {
        return Ok(State::Copied);
    }
    targets.clear();
    reserve(targets, refresh.len())?;
    targets.extend(refresh.iter().map(|&index| roots[index]));
    listed.clear();
    native::region_slots(memory, targets, |at, region, history| {
        push(listed, (at, region, history))
    })?;
    addresses.clear();
    found.clear();
    owners.clear();
    patch_addresses.clear();
    patch_found.clear();
    patch_owners.clear();
    let mut cursor = 0;
    for (at, &index) in refresh.iter().enumerate() {
        let end = cursor
            + listed[cursor..]
                .iter()
                .take_while(|entry| entry.0 == at)
                .count();
        let root = roots[index];
        let country = &mut countries[index];
        merged.clear();
        reserve_pushes(merged, end - cursor)?;
        let mut held = 0;
        for &(_, region, history) in &listed[cursor..end] {
            while country
                .histories
                .get(held)
                .is_some_and(|certificate| certificate.region < region)
            {
                push(
                    &mut staging.dropped,
                    (slot(index), country.histories[held].region),
                )?;
                held += 1;
            }
            let previous = country
                .histories
                .get(held)
                .filter(|certificate| certificate.region == region)
                .copied();
            if previous.is_some() {
                held += 1;
            }
            let now = changes.sequence(history);
            let touch = changes.sequence(history_changes::touch_key(root, region));
            let mut certificate = HistoryCertificate {
                region,
                history,
                sequence: now,
                touch,
                menu: false,
            };
            let step = match previous {
                Some(old) if old.history == history => {
                    certificate.menu = old.menu;
                    let advanced = match (old.sequence, now) {
                        (Some(before), Some(after)) if before == after => Some(0),
                        (Some((epoch, before)), Some((current, after)))
                            if epoch == current && after > before =>
                        {
                            Some(after - before)
                        }
                        _ => None,
                    };
                    match (advanced, touch.is_some() && touch == old.touch) {
                        (Some(0), true) => Step::Keep,
                        (Some(count), true) => Step::Advance(count),
                        (Some(0), false) if old.menu && touch.is_some() => Step::Patch,
                        _ => Step::Copy,
                    }
                }
                _ => Step::Copy,
            };
            let position = merged.len();
            match step {
                Step::Keep => {}
                Step::Advance(count) => {
                    push(&mut staging.advances, (slot(index), region, count))?;
                    staging.counts.advanced += 1;
                }
                Step::Patch => {
                    push(patch_addresses, history)?;
                    push(patch_found, (slot(index), region))?;
                    push(patch_owners, (index, position))?;
                }
                Step::Copy => {
                    push(addresses, history)?;
                    push(found, (slot(index), region))?;
                    push(owners, (index, position))?;
                }
            }
            merged.push(certificate);
        }
        while let Some(certificate) = country.histories.get(held) {
            push(&mut staging.dropped, (slot(index), certificate.region))?;
            held += 1;
        }
        country.histories.clear();
        reserve_pushes(&mut country.histories, merged.len())?;
        country.histories.extend_from_slice(merged);
        cursor = end;
    }
    staging.patch(
        memory,
        base,
        patch_addresses,
        patch_found,
        &mut *copy.menus,
        retry,
    )?;
    for &at in retry.iter() {
        push(addresses, patch_addresses[at])?;
        push(found, patch_found[at])?;
        push(owners, patch_owners[at])?;
    }
    staging.copy_windows(memory, base, addresses, found, &mut copy)?;
    for (copied, &(index, position)) in owners.iter().enumerate() {
        countries[index].histories[position].menu = copy.fallback.binary_search(&copied).is_err();
    }
    Ok(State::Copied)
}

pub struct Captured {
    rows: CaptureRows<AirNavalPatrolHistoryRow>,
    selection: SliceSelection,
    parts: Vec<Staging>,
    staged: bool,
}

impl Captured {
    pub fn vacant() -> Self {
        Self {
            rows: CaptureRows::new(0),
            selection: SliceSelection::new(),
            parts: Vec::new(),
            staged: false,
        }
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            selection: SliceSelection::new(),
            parts: allocation::owned_vec(allocation::collect_vec(
                (0..PARTS).map(|_| Staging::with_capacities(capacity)),
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

    pub fn release_transient(&mut self) {
        const STEADY_ROWS: usize = 4096;
        if self.rows.rows.capacity() > STEADY_ROWS {
            self.rows.rows = Vec::new();
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
        self.grow_parts(capacity);
        history_changes::observe(self.receivers());
    }

    fn grow_parts(&mut self, capacity: &mut Capacities) {
        const STEADY_ROWS: usize = 4096;
        capacity.rows = capacity.rows.min(STEADY_ROWS);
        self.rows.grow(&mut capacity.rows);
        capacity.rows = capacity.rows.min(STEADY_ROWS);
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
        slots.saturating_add(
            self.parts
                .iter()
                .map(|part| part.windows.len())
                .sum::<usize>()
                .saturating_mul(2),
        )
    }

    pub fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        self.selection.reset(hour, force_full);
    }

    pub fn selection(&self) -> &SliceSelection {
        &self.selection
    }

    pub fn take_parts(&mut self) -> Vec<Staging> {
        std::mem::take(&mut self.parts)
    }

    pub fn restore_parts(&mut self, parts: Vec<Staging>) {
        self.parts = parts;
    }

    pub fn take_part(&mut self, part: usize) -> Staging {
        std::mem::take(&mut self.parts[part])
    }

    pub fn restore_part(&mut self, part: usize, staging: Staging) {
        self.parts[part] = staging;
        self.staged = true;
    }

    pub fn settle(&self, opening: &mut Opening) {
        opening.complete = self.parts.iter().all(Staging::completed);
    }

    pub fn ticks(&self) -> [u64; N] {
        [self.parts.iter().map(|part| part.ticks).sum()]
    }

    pub fn staged(&self) -> bool {
        self.staged
    }

    pub fn unstaged(&mut self) -> [u64; N] {
        for part in &mut self.parts {
            part.fail("naval-patrol history parts not dispatched");
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
        let changed = self.parts.iter().any(|part| part.state == State::Copied);
        self.apply(held, &mut fingerprints);
        held.fingerprints = Some(fingerprints);
        if changed {
            HELD_BYTES.store(held.allocated_bytes(), Ordering::Relaxed);
        }
    }

    fn apply(&mut self, held: &mut Held, fingerprints: &mut SliceFingerprints) {
        let mut counts = Counts {
            quiet: true,
            ..Counts::default()
        };
        for part in &self.parts {
            counts.add(&part.counts);
        }
        history_changes::record_scenario_refusals(&counts.scenario_refusals);
        history_changes::record_menu_refusals(&counts.menu_refusals);
        let failure = self.parts.iter().find_map(|part| match part.state {
            State::Idle => Some("naval-patrol history part did not run"),
            State::Failed(reason) => Some(reason),
            State::Quiet | State::Copied => None,
        });
        if let Some(reason) = failure {
            self.rows.fail(Outcome::Unavailable { reason });
            self.selection.invalidate(fingerprints);
            return;
        }
        history_changes::record(&counts);
        if let Err(reason) = held.apply(&mut self.parts) {
            held.poisoned = true;
            forget_certificates();
            self.rows.fail(Outcome::Unavailable { reason });
            self.selection.invalidate(fingerprints);
            return;
        }
        let begin = qpc();
        for (slot, country) in held.countries.iter().enumerate() {
            self.selection
                .scan(fingerprints, slot as u32, Some(country.generation));
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
    records: Vec<(u32, [i64; 9])>,
    scenario_failures: Vec<(u32, &'static str)>,
    menu: Option<engine_air_history::menu_history::History>,
}

impl History {
    #[cfg(test)]
    fn new(region: u32, state: WindowState, channels: &[[i64; 9]]) -> Result<Self, &'static str> {
        Self::copied(region, state, channels, &vec![None; channels.len()])
    }

    fn copied(
        region: u32,
        state: WindowState,
        channels: &[[i64; 9]],
        failures: &[Option<&'static str>],
    ) -> Result<Self, &'static str> {
        if channels.len() != failures.len() {
            return Err("air history scenario refusal arena incomplete");
        }
        let count = channels
            .iter()
            .zip(failures)
            .filter(|(channels, failure)| **channels != [0; 9] || failure.is_some())
            .count();
        let mut records = Vec::new();
        reserve_pushes(&mut records, count)?;
        records.extend(
            channels
                .iter()
                .zip(failures)
                .enumerate()
                .filter(|(_, (channels, failure))| **channels != [0; 9] || failure.is_some())
                .map(|(slot, (&channels, _))| (slot as u32, channels)),
        );
        let mut scenario_failures = Vec::new();
        reserve_pushes(&mut scenario_failures, failures.iter().flatten().count())?;
        scenario_failures.extend(
            failures
                .iter()
                .enumerate()
                .filter_map(|(slot, &reason)| reason.map(|reason| (slot as u32, reason))),
        );
        Ok(Self {
            region,
            state,
            records,
            scenario_failures,
            menu: None,
        })
    }

    fn advance(&mut self, advances: u64) -> Result<(), &'static str> {
        if let Some(menu) = &mut self.menu {
            menu.advance(advances)?;
        }
        let capacity = u64::from(self.state.capacity);
        let from = u64::from(self.state.current);
        self.records.retain(|&(slot, _)| {
            let distance = (u64::from(slot) + capacity - from) % capacity;
            let distance = if distance == 0 { capacity } else { distance };
            distance > advances
        });
        self.scenario_failures.retain(|&(slot, _)| {
            self.records
                .binary_search_by_key(&slot, |&(slot, _)| slot)
                .is_ok()
        });
        let next = from + advances;
        self.state.current = (next % capacity) as u32;
        self.state.wrapped |= next >= capacity;
        Ok(())
    }
}

#[derive(Default)]
struct HeldCountry {
    histories: Vec<History>,
    generation: u64,
}

pub struct Held {
    countries: Vec<HeldCountry>,
    generation: u64,
    fingerprints: Option<Box<SliceFingerprints>>,
    changed: Vec<bool>,
    poisoned: bool,
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
            changed: Vec::new(),
            poisoned: false,
        }
    }

    fn allocated_bytes(&self) -> usize {
        self.countries.capacity() * size_of::<HeldCountry>()
            + self.changed.capacity() * size_of::<bool>()
            + self
                .countries
                .iter()
                .map(|country| {
                    country.histories.capacity() * size_of::<History>()
                        + country
                            .histories
                            .iter()
                            .map(|history| {
                                history.records.capacity() * size_of::<(u32, [i64; 9])>()
                                    + history.scenario_failures.capacity()
                                        * size_of::<(u32, &'static str)>()
                                    + history.menu.as_ref().map_or(
                                        0,
                                        engine_air_history::menu_history::History::allocated_bytes,
                                    )
                            })
                            .sum::<usize>()
                })
                .sum::<usize>()
    }

    fn apply(&mut self, parts: &mut [Staging]) -> Result<(), &'static str> {
        if self.poisoned {
            if !parts
                .iter()
                .all(|part| part.state == State::Copied && part.fresh)
            {
                return Err("air history copies await an hour read afresh");
            }
            self.countries.clear();
            self.poisoned = false;
        }
        let Some(_) = parts.iter().find(|part| part.state == State::Copied) else {
            return Ok(());
        };
        let slots = parts.iter().map(|part| part.slots).max().unwrap_or(0);
        if self.countries.len() != slots {
            reserve(&mut self.countries, slots)?;
            self.countries.resize_with(slots, HeldCountry::default);
        }
        self.changed.clear();
        reserve(&mut self.changed, slots)?;
        self.changed.resize(slots, false);
        for part in parts.iter_mut().filter(|part| part.state == State::Copied) {
            for &slot in part.removed.iter().chain(&part.replaced) {
                let country = &mut self.countries[slot as usize];
                if !country.histories.is_empty() {
                    country.histories.clear();
                    self.changed[slot as usize] = true;
                }
            }
            for &(slot, region) in &part.dropped {
                let histories = &mut self.countries[slot as usize].histories;
                if let Ok(at) = histories.binary_search_by_key(&region, |history| history.region) {
                    histories.remove(at);
                    self.changed[slot as usize] = true;
                }
            }
            for &(slot, region, advances) in &part.advances {
                let histories = &mut self.countries[slot as usize].histories;
                let at = histories
                    .binary_search_by_key(&region, |history| history.region)
                    .map_err(|_| "air history advance has no held history")?;
                histories[at].advance(advances)?;
                self.changed[slot as usize] = true;
            }
            for (slot, region, current) in &part.patches {
                let histories = &mut self.countries[*slot as usize].histories;
                let history = histories
                    .binary_search_by_key(region, |history| history.region)
                    .ok()
                    .and_then(|at| histories[at].menu.as_mut())
                    .ok_or("air history current record has no held windows")?;
                if history.apply_current(current)? {
                    self.changed[*slot as usize] = true;
                }
            }
            for (menu_index, window) in part.windows.iter().enumerate() {
                let len = if part.menus.get(menu_index).is_some_and(Option::is_some) {
                    0
                } else {
                    window.state.capacity as usize
                };
                let channels = part
                    .channels
                    .get(window.offset..window.offset + len)
                    .ok_or("air history channel arena incomplete")?;
                let failures = part
                    .scenario_failures
                    .get(window.offset..window.offset + len)
                    .ok_or("air history scenario refusal arena incomplete")?;
                let mut history = History::copied(window.region, window.state, channels, failures)?;
                history.menu = part.menus.get_mut(menu_index).and_then(Option::take);
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

    #[cfg(any(test, feature = "test-support"))]
    fn same(&self, other: &Self) -> bool {
        self.countries.len() == other.countries.len()
            && self
                .countries
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
            .try_fold(0usize, |needed, (_, country)| {
                country
                    .histories
                    .iter()
                    .try_fold(needed.checked_add(1)?, |needed, history| {
                        let rows = match &history.menu {
                            Some(menu) => menu
                                .records
                                .len()
                                .checked_add(engine_air_history::menu_history::LEVELS),
                            None => history.records.len().checked_add(1),
                        }?;
                        needed.checked_add(rows)
                    })
            });
        let Some(needed) = needed else {
            return Outcome::Unavailable {
                reason: "air held history row count overflow",
            };
        };
        if reserve_pushes(output, needed.saturating_sub(output.len())).is_err() {
            return Outcome::Unavailable {
                reason: "collector allocation failed",
            };
        }
        let mut len = 0;
        let mut emit = |row| {
            match output.get_mut(len) {
                Some(slot) => *slot = row,
                None => output.push(row),
            }
            len += 1;
        };
        for (slot, country) in self.countries.iter().enumerate() {
            if !selection.contains(slot as u32) {
                continue;
            }
            for history in &country.histories {
                if let Some(menu) = &history.menu {
                    let mut records = menu.records.iter().peekable();
                    for (level, window) in menu.windows.iter().enumerate() {
                        while let Some(record) =
                            records.next_if(|record| usize::from(record.level) == level)
                        {
                            let mut row = AirNavalPatrolHistoryRow {
                                country: slot as u32,
                                region: history.region,
                                level: level as u8,
                                index: record.slot,
                                ..Default::default()
                            };
                            row.set_channels(&record.values);
                            emit(row);
                        }
                        emit(AirNavalPatrolHistoryRow::window_marker(
                            slot as u32,
                            history.region,
                            level as u8,
                            replay_data::rows::AirHistoryWindow {
                                capacity: window.capacity,
                                current: window.current,
                                wrapped: window.wrapped,
                                complete: true,
                            },
                            true,
                        ));
                    }
                } else {
                    for &(index, values) in &history.records {
                        emit(AirNavalPatrolHistoryRow {
                            country: slot as u32,
                            region: history.region,
                            index,
                            enemy_ships_sunk_raw: values[0],
                            our_fighters_lost_raw: values[1],
                            scenario_channels: replay_data::rows::Recorded::Value(
                                history
                                    .scenario_failures
                                    .binary_search_by_key(&index, |&(slot, _)| slot)
                                    .is_err()
                                    .then(|| std::array::from_fn(|at| values[at + 2])),
                            ),
                            menu: replay_data::rows::Recorded::Value(None),
                            ..Default::default()
                        });
                    }
                    emit(AirNavalPatrolHistoryRow::window_marker(
                        slot as u32,
                        history.region,
                        0,
                        replay_data::rows::AirHistoryWindow {
                            capacity: history.state.capacity,
                            current: history.state.current,
                            wrapped: history.state.wrapped,
                            complete: false,
                        },
                        false,
                    ));
                }
            }
            emit(AirNavalPatrolHistoryRow::availability_witness(slot as u32));
        }
        Outcome::Rows { len }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn allocation_failure_exercise() {
    let capacity = Capacities {
        rows: 4,
        countries: 3,
        histories: 2,
        channels: 6,
    };
    let mut part = Staging::with_capacities(&capacity);
    if crate::allocation::failed() {
        return;
    }
    part.channels
        .extend_from_slice(&[[1, 0], [0, 0], [2, 4], [0, 0], [3, 0], [0, 5]].map(test_channels));
    part.scenario_failures.resize(part.channels.len(), None);
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
    if held.apply(std::slice::from_mut(&mut part)).is_err() {
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
    let _ = table::<tables::AirNavalPatrolHistories, _>(outcome, &output, Clone::clone);
    if crate::allocation::failed() {
        return;
    }
    part.windows[0].region = 9;
    part.windows.truncate(1);
    let _ = held.apply(std::slice::from_mut(&mut part));
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let channels = [[1, 0], [2, 0], [0, 0], [4, 4], [5, 0]].map(test_channels);
        let mut history = History::new(7, state(3, false), &channels).unwrap();
        history.advance(2).unwrap();
        assert_eq!(history.state, state(0, true));
        assert_eq!(
            history.records,
            [(1, test_channels([2, 0])), (3, test_channels([4, 4]))]
        );
        let mut lap = History::new(7, state(3, true), &channels).unwrap();
        lap.advance(5).unwrap();
        assert_eq!(lap.state, state(3, true));
        assert!(lap.records.is_empty());
        let mut short = History::new(7, state(3, true), &channels).unwrap();
        short.advance(4).unwrap();
        assert_eq!(short.records, [(3, test_channels([4, 4]))]);
    }

    #[test]
    fn native_slot_rows_preserve_summation_order_through_the_cursor() {
        let channels = [[1, 0], [2, 0], [0, 0], [4, 4], [5, 0]].map(test_channels);
        let mut held = Held::new();
        held.countries = vec![
            HeldCountry::default(),
            HeldCountry {
                histories: vec![History::new(7, state(3, true), &channels).unwrap()],
                generation: 1,
            },
        ];
        let mut selection = SliceSelection::new();
        selection.reset(0, true);
        let mut rows = Vec::new();
        assert_eq!(
            held.publish(&selection, &mut rows),
            Outcome::Rows { len: 7 }
        );
        let order: Vec<_> = rows
            .iter()
            .filter(|row| !row.is_availability_witness() && !row.is_menu())
            .map(|row| (row.index, row.enemy_ships_sunk_raw))
            .collect();
        assert_eq!(order, [(0, 1), (1, 2), (3, 4), (4, 5)]);
        let mut chronological: Vec<_> = rows
            .iter()
            .filter(|row| !row.is_availability_witness() && !row.is_menu())
            .map(|row| {
                (
                    row.summation_index(&rows).unwrap().unwrap(),
                    row.enemy_ships_sunk_raw,
                )
            })
            .collect();
        chronological.sort_unstable();
        assert_eq!(chronological, [(0, 4), (1, 5), (2, 1), (3, 2)]);
        assert!(rows.windows(2).all(|pair| pair[0].key() < pair[1].key()));
        assert_eq!(
            rows.iter()
                .filter(|row| row.is_availability_witness())
                .count(),
            2
        );
    }

    #[test]
    fn refused_scenario_records_preserve_patrol_rows_and_native_advance_clears_them() {
        let channels = [test_channels([1, 2]), [0; 9], test_channels([3, 4]), [0; 9]];
        let failures = [None, Some(native::SCENARIO_REFUSALS[6]), None, None];
        let state = WindowState {
            history: 1,
            capacity: 4,
            current: 3,
            wrapped: false,
        };
        let mut held = Held::new();
        held.countries = vec![HeldCountry {
            histories: vec![History::copied(7, state, &channels, &failures).unwrap()],
            generation: 1,
        }];
        let mut selection = SliceSelection::new();
        selection.reset(0, true);
        let mut rows = Vec::new();
        assert_eq!(
            held.publish(&selection, &mut rows),
            Outcome::Rows { len: 5 }
        );
        assert_eq!(rows[0].enemy_ships_sunk_raw, 1);
        assert_eq!(rows[0].our_fighters_lost_raw, 2);
        assert_eq!(
            rows[0].scenario_channels,
            replay_data::rows::Recorded::Value(Some([0; 7]))
        );
        assert_eq!(
            rows[1].scenario_channels,
            replay_data::rows::Recorded::Value(None)
        );
        assert_eq!(
            (rows[1].enemy_ships_sunk_raw, rows[1].our_fighters_lost_raw),
            (0, 0)
        );
        assert!(
            rows[1].has_values(),
            "a refused zero record must keep its availability"
        );
        assert_eq!(rows[2].enemy_ships_sunk_raw, 3);
        assert_eq!(rows[2].our_fighters_lost_raw, 4);
        assert_eq!(
            held.countries[0].histories[0].scenario_failures,
            [(1, native::SCENARIO_REFUSALS[6])]
        );
        held.countries[0].histories[0].advance(3).unwrap();
        assert!(held.countries[0].histories[0].scenario_failures.is_empty());
        assert_eq!(
            held.publish(&selection, &mut rows),
            Outcome::Rows { len: 2 }
        );
        assert!(rows[0].is_menu());
        assert_eq!(rows[1], AirNavalPatrolHistoryRow::availability_witness(0));
    }

    #[test]
    fn scenario_only_records_and_captured_empty_countries_keep_their_availability() {
        let mut channels = [[0; 9]; 5];
        channels[1][2..].copy_from_slice(&[10, 17, 51, 58, 44, 24, 65]);
        let mut held = Held::new();
        held.countries = vec![
            HeldCountry::default(),
            HeldCountry {
                histories: vec![History::new(7, state(2, false), &channels).unwrap()],
                generation: 1,
            },
        ];
        let mut selection = SliceSelection::new();
        selection.reset(0, true);
        let mut rows = Vec::new();
        assert_eq!(
            held.publish(&selection, &mut rows),
            Outcome::Rows { len: 4 }
        );
        assert_eq!(rows[0], AirNavalPatrolHistoryRow::availability_witness(0));
        assert_eq!(
            rows[1].scenario_channels,
            replay_data::rows::Recorded::Value(Some([10, 17, 51, 58, 44, 24, 65]))
        );
        assert_eq!(rows[1].index, 1);
        assert!(rows[2].is_menu());
        assert_eq!(rows[3], AirNavalPatrolHistoryRow::availability_witness(1));
        held.countries[1].histories[0].advance(5).unwrap();
        assert_eq!(
            held.publish(&selection, &mut rows),
            Outcome::Rows { len: 3 }
        );
        assert!(rows[1].is_menu());
        assert_eq!(
            &[rows[0].clone(), rows[2].clone()],
            &[
                AirNavalPatrolHistoryRow::availability_witness(0),
                AirNavalPatrolHistoryRow::availability_witness(1),
            ]
        );
        let mut empty = Held::new();
        empty.countries.push(HeldCountry::default());
        assert!(
            !empty.same(&held),
            "a null-slot census change affects availability"
        );
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
    fn cold_histories_and_native_selection_publish_countries_then_more_regions() {
        const COUNTRIES: usize = 20;
        let capacity = Capacities::default();
        assert_eq!(
            [
                capacity.rows,
                capacity.countries,
                capacity.histories,
                capacity.channels
            ],
            [0; 4]
        );
        let mut pack = Captured::with_capacities(&capacity);
        let mut held = Held::new();
        for (hour, regions) in [2usize, 6].into_iter().enumerate() {
            pack.reset();
            pack.prepare_hour(hour as i32, hour == 0);
            for part in &mut pack.parts {
                part.slots = COUNTRIES + 1;
                part.state = State::Copied;
            }
            for slot in 1..=COUNTRIES {
                let part = &mut pack.parts[slot % PARTS];
                for region in 0..regions {
                    part.windows.push(Window {
                        slot: slot as u32,
                        region: region as u32,
                        state: state(0, true),
                        offset: part.channels.len(),
                    });
                    part.channels
                        .extend([test_channels([region as i64 + 1, slot as i64]); 5]);
                    part.scenario_failures.resize(part.channels.len(), None);
                }
            }
            pack.staged = true;
            pack.decode_into(&mut held);
            let witnesses = (0..=COUNTRIES)
                .filter(|&slot| pack.selection.contains(slot as u32))
                .count();
            assert_eq!(
                pack.outcomes(),
                [Outcome::Rows {
                    len: COUNTRIES * regions * 6 + witnesses
                }]
            );
            assert!(pack.selection.read_owners().count() >= COUNTRIES);
            let last = &pack.rows.rows[pack.rows.outcome.len() - 3];
            assert_eq!(last.country, COUNTRIES as u32);
            assert_eq!(last.region, regions as u32 - 1);
            assert_eq!(last.enemy_ships_sunk_raw, regions as i64);
        }
    }

    #[test]
    fn hour_accounting_includes_all_retained_window_scratch_arenas() {
        let frame = hour::frame(&Capacities::default());
        let mut hour = hour::Hour::new();
        let before = hour.allocated(&frame);
        for certificates in &mut hour.certificates {
            assert_eq!(certificates.scratch.windows.allocated_bytes(), 0);
            certificates.scratch.windows.size(5_000).unwrap();
        }
        let scratch: usize = hour
            .certificates
            .iter()
            .map(|certificates| certificates.scratch.windows.allocated_bytes())
            .sum();
        assert!(scratch > 0);
        assert_eq!(hour.allocated(&frame), before + scratch);
        for certificates in &mut hour.certificates {
            certificates.scratch.windows.size(1).unwrap();
        }
        assert_eq!(hour.allocated(&frame), before + scratch);
    }

    #[test]
    fn a_current_record_patch_applies_only_where_the_held_windows_stand() {
        use engine_air_history::menu_history::{
            CHANNELS, Current, History as Menu, Window as Level,
        };
        let windows = [30, 12, 5].map(|capacity| Level {
            capacity,
            current: 2,
            wrapped: false,
        });
        let daily = WindowState {
            history: 0x1000,
            capacity: 30,
            current: 2,
            wrapped: false,
        };
        let mut history = History::copied(7, daily, &[], &[]).unwrap();
        history.menu = Some(Menu {
            roll_up: 0,
            windows,
            records: Vec::new(),
        });
        let mut held = Held::new();
        held.countries = vec![
            HeldCountry::default(),
            HeldCountry {
                histories: vec![history],
                generation: 1,
            },
        ];
        let mut values = [0; CHANNELS];
        values[3] = 11;
        let mut part = Staging {
            slots: 2,
            state: State::Copied,
            ..Staging::default()
        };
        part.patches.push((
            1,
            7,
            Current {
                roll_up: 0,
                windows,
                values,
            },
        ));
        held.apply(std::slice::from_mut(&mut part)).unwrap();
        let menu = held.countries[1].histories[0].menu.as_ref().unwrap();
        assert_eq!(menu.records.len(), 1);
        assert_eq!((menu.records[0].level, menu.records[0].slot), (0, 2));
        assert!(held.changed[1]);
        let mut moved = windows;
        moved[0].current = 3;
        part.patches[0].2.windows = moved;
        assert!(held.apply(std::slice::from_mut(&mut part)).is_err());
        held.poisoned = true;
        part.patches.clear();
        assert!(held.apply(std::slice::from_mut(&mut part)).is_err());
        part.fresh = true;
        held.apply(std::slice::from_mut(&mut part)).unwrap();
        assert!(!held.poisoned);
        assert!(
            held.countries
                .iter()
                .all(|country| country.histories.is_empty())
        );
    }

    #[test]
    fn missing_combats_still_open_capture_of_every_allocated_history() {
        let mut parts: Vec<_> = (0..PARTS).map(|_| Staging::default()).collect();
        let mut opening = Opening::new();
        opening.complete = true;
        supply(&mut parts, &history_changes::Unobserved, &mut opening);
        assert!(parts.iter().all(|part| part.input == Input::Supplied));
        assert!(opening.opened);
        assert!(!opening.complete);
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod hour {
    use super::*;

    pub struct Hour {
        pub certificates: Vec<Certificates>,
        pub opening: Opening,
        pub held: Held,
    }

    pub fn frame(capacity: &Capacities) -> Captured {
        Captured::with_capacities(capacity)
    }

    pub fn capacities(slots: usize) -> Capacities {
        let mut capacity = Capacities::default();
        capacity.raise_for_countries(slots);
        capacity
    }

    impl Default for Hour {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Hour {
        pub fn new() -> Self {
            Self {
                certificates: (0..PARTS).map(|_| Certificates::new()).collect(),
                opening: Opening::new(),
                held: Held::new(),
            }
        }

        pub fn supply(&mut self, frame: &mut Captured, _regions: &[u64], changes: &impl Changes) {
            supply(&mut frame.parts, changes, &mut self.opening);
        }

        pub fn stage<M: Memory + Sync>(
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

        pub fn stage_refusing<M: Memory>(
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

        pub fn fail(&mut self, frame: &mut Captured, part: usize) {
            frame.parts[part].fail("naval-patrol history acquisition failed");
        }

        pub fn settle(&mut self, frame: &Captured) {
            frame.settle(&mut self.opening);
        }

        pub fn decode(&mut self, frame: &mut Captured, at: i32) -> Outcome {
            frame.prepare_hour(at, at == 0);
            frame.decode_into(&mut self.held);
            frame.rows.outcome
        }

        #[expect(
            clippy::too_many_arguments,
            reason = "the frame, read operands, regions, change source, hour and threading choice are separate owners"
        )]
        pub fn run<M: Memory + Sync>(
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

        pub fn grow(frame: &mut Captured, capacity: &mut Capacities) -> usize {
            frame.grow_parts(capacity);
            frame.receivers()
        }

        pub fn allocated(&self, frame: &Captured) -> usize {
            let certificates: usize = self
                .certificates
                .iter()
                .map(Certificates::allocated_bytes)
                .sum();
            frame.allocated_bytes() + certificates
        }

        pub fn counts(frame: &Captured) -> Counts {
            let mut counts = Counts {
                quiet: true,
                ..Counts::default()
            };
            for part in &frame.parts {
                counts.add(&part.counts);
            }
            counts
        }

        pub fn copied(frame: &Captured) -> u64 {
            Self::counts(frame).read
        }

        pub fn held_rows(&self) -> usize {
            let histories = self
                .held
                .countries
                .iter()
                .flat_map(|country| &country.histories);
            histories
                .map(|history| {
                    history
                        .menu
                        .as_ref()
                        .map_or(history.records.len(), |menu| menu.records.len())
                })
                .sum()
        }

        pub fn same(&self, other: &Self) -> bool {
            self.held.same(&other.held)
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
fn test_channels(pair: [i64; 2]) -> [i64; 9] {
    let mut channels = [0; 9];
    channels[..2].copy_from_slice(&pair);
    channels
}
