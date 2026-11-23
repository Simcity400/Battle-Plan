use engine_allocation::{default_vec, owned_vec, reserve_exact, vec};
use engine_native_memory::{Memory, memory::{RecordField, RecordLayout, RecordSources}};
use engine_native::ccharacter::{self, manager::PortraitIdentity};

pub const ROW: usize = 64;
const ROLE_SPAN: usize = ccharacter::OPERATIVE + 8 - ccharacter::LEADER_MAP_HEAD;
const _: () = assert!(
    ccharacter::LEADER_MAP_SIZE == ccharacter::LEADER_MAP_HEAD + 8
        && ccharacter::UNIT_LEADER == ccharacter::LEADER_MAP_HEAD + 16
        && ccharacter::ADVISOR_MAP_HEAD == ccharacter::LEADER_MAP_HEAD + 24
        && ccharacter::SCIENTIST == ccharacter::LEADER_MAP_HEAD + 40
        && ccharacter::OPERATIVE == ccharacter::LEADER_MAP_HEAD + 48
        && ROLE_SPAN == 56
        && 8 + ROLE_SPAN == ROW
);
const FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: ccharacter::COUNTRY,
        destination_offset: 4,
        len: 4,
    },
    RecordField {
        source_offset: ccharacter::LEADER_MAP_HEAD,
        destination_offset: 8,
        len: ROLE_SPAN,
    },
];
const LAYOUT: RecordLayout<'static> = match RecordLayout::new(&FIELDS, ROW) {
    Some(layout) => layout,
    None => panic!("role census layout"),
};

#[derive(Debug)]
pub struct RoleCensus {
    pointers: Vec<usize>,
    rows: Vec<u8>,
    historical: usize,
    scope: (usize, usize),
    complete: bool,
    required: usize,
    generation: u64,
}

static NEXT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Debug, Default)]
struct History {
    scope: (usize, usize),
    historical: usize,
    pointers: Vec<usize>,
    rows: Vec<u8>,
    generation: u64,
}

impl History {
    fn generation(
        &mut self,
        scope: (usize, usize),
        historical: usize,
        pointers: &[usize],
        rows: &[u8],
    ) -> u64 {
        if self.generation != 0
            && self.scope == scope
            && self.historical == historical
            && self.pointers == pointers
            && self.rows == rows
        {
            return self.generation;
        }
        let generation = fresh_generation();
        self.generation = 0;
        self.pointers.clear();
        self.rows.clear();
        let held = engine_allocation::advisory(|| {
            reserve_exact(&mut self.pointers, pointers.len())?;
            reserve_exact(&mut self.rows, rows.len())
        });
        if held.is_ok() {
            self.pointers.extend_from_slice(pointers);
            self.rows.extend_from_slice(rows);
            self.scope = scope;
            self.historical = historical;
            self.generation = generation;
        }
        generation
    }
}

fn fresh_generation() -> u64 {
    NEXT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

static HISTORY: std::sync::Mutex<History> = std::sync::Mutex::new(History {
    scope: (0, 0),
    historical: 0,
    pointers: Vec::new(),
    rows: Vec::new(),
    generation: 0,
});

fn generation_of(scope: (usize, usize), historical: usize, pointers: &[usize], rows: &[u8]) -> u64 {
    match HISTORY.try_lock() {
        Ok(mut history) => history.generation(scope, historical, pointers, rows),
        Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned
            .into_inner()
            .generation(scope, historical, pointers, rows),
        Err(std::sync::TryLockError::WouldBlock) => fresh_generation(),
    }
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct FixtureHistory(History);

#[cfg(test)]
impl FixtureHistory {
    pub(crate) fn gather(
        &mut self,
        roles: &mut RoleCensus,
        memory: &impl Memory,
        scope: (usize, usize),
        census: &[PortraitIdentity],
        historical: usize,
    ) -> bool {
        roles.gather_using(memory, scope, census, historical, |scope, historical, pointers, rows| {
            self.0.generation(scope, historical, pointers, rows)
        })
    }
}

static LIVE_LEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static LIVE_HINT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
#[test]
fn role_generations_follow_rows_across_workers() {
    let _globals = super::test_globals();
    let generation = || generation_of((123, 456), 1, &[789], &[17; ROW]);
    let first = generation();
    assert_eq!(std::thread::spawn(generation).join().unwrap(), first);
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn forget_live() {
    LIVE_LEN.store(0, std::sync::atomic::Ordering::Relaxed);
    LIVE_HINT.store(0, std::sync::atomic::Ordering::Relaxed);
    *HISTORY.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = History::default();
}

pub(super) fn remember_roster(count: usize) {
    LIVE_HINT.fetch_max(count, std::sync::atomic::Ordering::Relaxed);
}

pub fn initial_len() -> usize {
    live_len().max(LIVE_HINT.load(std::sync::atomic::Ordering::Relaxed))
}

pub fn live_len() -> usize {
    LIVE_LEN.load(std::sync::atomic::Ordering::Relaxed)
}

impl RoleCensus {
    pub fn new(capacity: usize) -> Self {
        Self {
            pointers: owned_vec(vec(capacity)),
            rows: capacity.checked_mul(ROW).map_or_else(
                || {
                    let _ = collector_memory::terminal::oom();
                    Vec::new()
                },
                |bytes| owned_vec(default_vec(bytes)),
            ),
            historical: 0,
            scope: (0, 0),
            complete: false,
            required: 0,
            generation: 0,
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.pointers.capacity() * size_of::<usize>() + self.rows.capacity()
    }

    pub fn needs_growth(&self) -> bool {
        self.required > self.fits()
    }

    fn fits(&self) -> usize {
        self.pointers.capacity().min(self.rows.len() / ROW)
    }

    pub fn grow(&mut self) {
        if self.needs_growth() {
            let additional = self.required - self.pointers.len();
            if reserve_exact(&mut self.pointers, additional).is_err() {
                return;
            }
            let Some(bytes) = self.required.checked_mul(ROW) else {
                let _ = reserve_exact(&mut self.rows, usize::MAX);
                return;
            };
            let additional = bytes.saturating_sub(self.rows.len());
            if reserve_exact(&mut self.rows, additional).is_err() {
                return;
            }
            self.rows.resize(bytes.max(self.rows.len()), 0);
        }
    }

    pub fn gather(
        &mut self,
        memory: &impl Memory,
        scope: (usize, usize),
        census: &[PortraitIdentity],
        historical: usize,
    ) -> bool {
        LIVE_LEN.store(census.len(), std::sync::atomic::Ordering::Relaxed);
        self.gather_using(memory, scope, census, historical, generation_of)
    }

    fn gather_using(
        &mut self,
        memory: &impl Memory,
        scope: (usize, usize),
        census: &[PortraitIdentity],
        historical: usize,
        generation: impl FnOnce((usize, usize), usize, &[usize], &[u8]) -> u64,
    ) -> bool {
        self.scope = scope;
        self.complete = false;
        self.generation = 0;
        self.required = census.len();
        self.historical = historical;
        self.grow();
        if census.len() > self.fits() || historical > census.len() {
            return false;
        }
        self.pointers.clear();
        self.pointers.extend(census.iter().map(|row| row.pointer));
        let rows = &mut self.rows[..census.len() * ROW];
        for (row, identity) in rows.chunks_exact_mut(ROW).zip(census) {
            row[..4].copy_from_slice(&identity.character.to_le_bytes());
        }
        let completed = memory.copy_records(RecordSources::Pointers(&self.pointers), &LAYOUT, rows);
        self.complete = completed == census.len() * FIELDS.len();
        if self.complete {
            self.generation = generation(scope, historical, &self.pointers, rows);
        }
        self.complete
    }

    fn holds(&self, scope: (usize, usize), census: &[PortraitIdentity], historical: usize) -> bool {
        self.complete
            && self.scope == scope
            && self.historical == historical
            && self.pointers.len() == census.len()
            && self
                .pointers
                .iter()
                .zip(self.rows.chunks_exact(ROW))
                .zip(census)
                .all(|((&pointer, row), identity)| {
                    pointer == identity.pointer && row[..4] == identity.character.to_le_bytes()
                })
    }

    pub fn generation(&self) -> u64 {
        if self.complete { self.generation } else { 0 }
    }

    pub fn admitted(&self, base: usize, game_state: usize) -> Option<Admitted<'_>> {
        (self.complete && self.scope == (base, game_state)).then(|| Admitted {
            pointers: &self.pointers,
            rows: &self.rows[..self.pointers.len() * ROW],
            historical: self.historical,
            generation: self.generation,
        })
    }
}

#[derive(Clone, Copy)]
pub struct Admitted<'a> {
    pointers: &'a [usize],
    rows: &'a [u8],
    pub historical: usize,
    pub generation: u64,
}

impl Admitted<'_> {
    pub fn len(self) -> usize {
        self.pointers.len()
    }

    pub fn occurrence(self, index: usize) -> Occurrence {
        Occurrence {
            pointer: self.pointers[index],
            index: index as u32,
            row: self.rows[index * ROW..][..ROW].try_into().unwrap(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Occurrence {
    pub pointer: usize,
    pub index: u32,
    pub row: [u8; ROW],
}

impl Occurrence {
    pub fn id(&self) -> u32 {
        u32::from_le_bytes(self.row[..4].try_into().unwrap())
    }

    pub fn country(&self) -> [u8; 4] {
        self.row[4..8].try_into().unwrap()
    }

    pub fn leaders(&self) -> [u8; 16] {
        self.row[8..24].try_into().unwrap()
    }

    pub fn unit(&self) -> [u8; 8] {
        self.row[24..32].try_into().unwrap()
    }

    pub fn advisors(&self) -> [u8; 16] {
        self.row[32..48].try_into().unwrap()
    }

    pub fn scientist(&self) -> u64 {
        u64::from_le_bytes(self.row[48..56].try_into().unwrap())
    }

    pub fn operative(&self) -> u64 {
        u64::from_le_bytes(self.row[56..64].try_into().unwrap())
    }

    pub fn occupant(&self) -> engine_characters::character_inputs::Occupant {
        engine_characters::character_inputs::Occupant {
            pointer: self.pointer,
            character: self.id(),
            leaders: self.leaders(),
            advisors: self.advisors(),
            scientist: self.scientist(),
            operative: self.operative(),
        }
    }

    pub fn fields(&self, required: [bool; 4]) -> ccharacter::AcquiredFields {
        ccharacter::AcquiredFields {
            country: (required[0] || required[1] || required[3]).then(|| self.country()),
            leaders: required[1].then(|| self.leaders()),
            unit: required[2].then(|| self.unit()),
            advisors: required[3].then(|| self.advisors()),
        }
    }

    pub fn identity(&self, historical: usize) -> ccharacter::Occurrence {
        let index = self.index as usize;
        let (roster, ordinal) = if index < historical {
            (0, index)
        } else {
            (1, index - historical)
        };
        ccharacter::Occurrence {
            pointer: self.pointer,
            id: self.id(),
            roster,
            ordinal: ordinal as u32,
        }
    }
}

pub fn map_head(header: [u8; 16]) -> Option<(usize, u32)> {
    let head = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
        .ok()
        .filter(|&head| head != 0)?;
    let count = u32::try_from(u64::from_le_bytes(header[8..].try_into().unwrap())).ok()?;
    Some((head, count))
}

pub fn inside_view(pointer: usize, head: usize, len: usize) -> bool {
    head < pointer.saturating_add(ccharacter::VIEW_EXTENT) && head.saturating_add(len) > pointer
}

pub const SENTINEL_BATCH: usize = 128;
pub const SENTINEL: usize = 26;
const SENTINEL_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: SENTINEL,
}];
pub const SENTINEL_LAYOUT: RecordLayout<'static> =
    match RecordLayout::new(&SENTINEL_FIELDS, SENTINEL) {
        Some(layout) => layout,
        None => panic!("sentinel layout"),
    };

pub fn gather_sentinels(
    memory: &impl Memory,
    heads: &[usize],
    output: &mut [u8; SENTINEL_BATCH * SENTINEL],
) -> usize {
    if heads.is_empty() {
        return 0;
    }
    for &head in heads {
        memory.prefetch(head, SENTINEL);
    }
    memory.copy_records(
        RecordSources::Pointers(heads),
        &SENTINEL_LAYOUT,
        &mut output[..heads.len() * SENTINEL],
    )
}

pub struct CensusCarry {
    changes: &'static dyn collector_observers::census_changes::Changes,
    token: Option<(u64, u64)>,
    stamp: Option<u64>,
}

impl std::fmt::Debug for CensusCarry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CensusCarry")
            .field("token", &self.token)
            .finish()
    }
}

impl Default for CensusCarry {
    fn default() -> Self {
        Self::with(&collector_observers::census_changes::Observer)
    }
}

impl CensusCarry {
    pub fn with(changes: &'static dyn collector_observers::census_changes::Changes) -> Self {
        Self {
            changes,
            token: None,
            stamp: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn acquire(
        &mut self,
        memory: &impl Memory,
        roles: &mut RoleCensus,
        scope: (usize, usize),
        census: &[PortraitIdentity],
        stamp: u64,
        historical: usize,
    ) -> (bool, collector_observers::census_changes::Counts) {
        let mut counts = collector_observers::census_changes::Counts::default();
        let token = self.changes.quiet();
        let same_occurrences = || {
            if self.stamp == Some(stamp) {
                roles.complete && roles.scope == scope && roles.historical == historical
            } else {
                roles.holds(scope, census, historical)
            }
        };
        if token.is_some() && token == self.token && same_occurrences() {
            counts.carried = true;
            return (true, counts);
        }
        counts.gathered = true;
        let admitted = roles.gather(memory, scope, census, historical);
        self.token = token.filter(|_| admitted);
        self.stamp = admitted.then_some(stamp);
        (admitted, counts)
    }
}
