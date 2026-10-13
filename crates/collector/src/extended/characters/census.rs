//! Role fields of the roster job's admitted census, loaned to the role jobs
//! and the character-input consumer of the same stopped-world callback so
//! they need not walk the roster again.

use engine::allocation::{default_vec, owned_vec, reserve_exact, vec};
use engine::{
    Memory, ccharacter,
    ccharacter_manager::PortraitIdentity,
    memory::{RecordField, RecordLayout, RecordSources},
};

/// One occurrence: character id, then the exact bytes at `COUNTRY` and at
/// `LEADER_MAP_HEAD..OPERATIVE + 8`, which hold the leader map header, the
/// unit leader pointer, the advisor map header, the scientist pointer and the
/// unit-leader reference in that order.
pub(crate) const ROW: usize = 64;
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

/// The roster job gathers these fields once on role hours, after its roster
/// pass, from the occurrences of its complete portrait census. Worker storage
/// grows from the original roster before this census can be published.
#[derive(Debug)]
pub(crate) struct RoleCensus {
    pointers: Vec<usize>,
    rows: Vec<u8>,
    historical: usize,
    scope: (usize, usize),
    complete: bool,
    required: usize,
    /// The admitted content's [`Generation`]; zero for none.
    generation: u64,
    /// The admitted rows' indices by `id % BURNED_IN_ROTATION`, and each
    /// bucket's start, when storage allowed: a slice of either rotation
    /// covers whole buckets ([`Admitted::covered`]).
    by_key: Vec<u32>,
    starts: [u32; BUCKETS + 1],
    indexed: bool,
}

/// Buckets of the admitted rows' keys.
const BUCKETS: usize = crate::sampled_verify::BURNED_IN_ROTATION as usize;

/// Names the content of an admitted census: two censuses share a nonzero
/// generation only when their scope, historical length, occurrence pointers
/// and every gathered byte are equal. The roster job compares each admitted
/// census with the latest one ([`History`]) and keeps its generation when
/// they are byte-for-byte equal, so a role job whose last complete capture
/// used generation `g` knows, without reading or comparing anything, that
/// this hour's occurrences, identities, countries and role headers are the
/// ones it read then. Any difference, a held loan, or storage that cannot
/// grow takes a generation no earlier census had.
static NEXT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The latest admitted census's content and generation.
#[derive(Debug, Default)]
struct History {
    scope: (usize, usize),
    historical: usize,
    pointers: Vec<usize>,
    rows: Vec<u8>,
    /// Zero when the content is not held.
    generation: u64,
}

impl History {
    /// The generation of an admitted census with this content, keeping the
    /// held one when equal. Worker only: holding new content may allocate,
    /// advisorily; a failure holds nothing.
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
        let held = engine::allocation::advisory(|| {
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

/// Only the roster job, one per hour, takes it; a contended lock takes a
/// fresh generation instead of waiting.
#[cfg(not(test))]
fn generation_of(scope: (usize, usize), historical: usize, pointers: &[usize], rows: &[u8]) -> u64 {
    static HISTORY: std::sync::Mutex<History> = std::sync::Mutex::new(History {
        scope: (0, 0),
        historical: 0,
        pointers: Vec::new(),
        rows: Vec::new(),
        generation: 0,
    });
    match HISTORY.try_lock() {
        Ok(mut history) => history.generation(scope, historical, pointers, rows),
        Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned
            .into_inner()
            .generation(scope, historical, pointers, rows),
        Err(std::sync::TryLockError::WouldBlock) => fresh_generation(),
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: each test thread's own latest census.
    static HISTORY: std::cell::RefCell<History> = std::cell::RefCell::default();
}

#[cfg(test)]
fn generation_of(scope: (usize, usize), historical: usize, pointers: &[usize], rows: &[u8]) -> u64 {
    HISTORY.with_borrow_mut(|history| history.generation(scope, historical, pointers, rows))
}

/// The occurrence count of the most recent role census the roster job
/// acquired. Role jobs size their visit lists from it: a pass visits at most
/// every occurrence.
static LIVE_LEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static LIVE_HINT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub(super) fn remember_roster(count: usize) {
    LIVE_HINT.fetch_max(count, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn initial_len() -> usize {
    live_len().max(LIVE_HINT.load(std::sync::atomic::Ordering::Relaxed))
}

/// Occurrences in the live roster, as last observed; 0 before the first
/// role census.
pub(crate) fn live_len() -> usize {
    LIVE_LEN.load(std::sync::atomic::Ordering::Relaxed)
}

impl RoleCensus {
    pub fn new(capacity: usize) -> Self {
        Self {
            pointers: owned_vec(vec(capacity)),
            rows: capacity.checked_mul(ROW).map_or_else(
                || {
                    let _ = crate::allocation::oom();
                    Vec::new()
                },
                |bytes| owned_vec(default_vec(bytes)),
            ),
            historical: 0,
            scope: (0, 0),
            complete: false,
            required: 0,
            generation: 0,
            by_key: owned_vec(vec(capacity)),
            starts: [0; BUCKETS + 1],
            indexed: false,
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.pointers.capacity() * size_of::<usize>()
            + self.rows.capacity()
            + self.by_key.capacity() * size_of::<u32>()
    }

    pub fn needs_growth(&self) -> bool {
        self.required > self.fits()
    }

    /// Occurrences both vectors hold without reallocating: `pointers` must
    /// not grow in the callback, and its capacity may exceed the rows.
    fn fits(&self) -> usize {
        self.pointers.capacity().min(self.rows.len() / ROW)
    }

    /// Off-thread only: reserve the whole observed roster in one step.
    pub fn grow(&mut self) {
        if self.needs_growth() {
            let additional = self.required - self.pointers.len();
            if reserve_exact(&mut self.pointers, additional).is_err() {
                return;
            }
            let additional = self.required - self.by_key.len();
            if reserve_exact(&mut self.by_key, additional).is_err() {
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

    /// Gather every occurrence's fields in one guarded call. `census` must be
    /// complete and freshly certified for `scope` in this callback; the
    /// census is admitted only when every field of every occurrence copied.
    pub(crate) fn acquire_with_growth(
        &mut self,
        memory: &impl Memory,
        scope: (usize, usize),
        census: &[PortraitIdentity],
        historical: usize,
        growing: bool,
    ) -> bool {
        self.scope = scope;
        self.complete = false;
        self.generation = 0;
        self.indexed = false;
        self.required = census.len();
        LIVE_LEN.store(census.len(), std::sync::atomic::Ordering::Relaxed);
        self.historical = historical;
        if growing {
            self.grow();
        }
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
            self.generation = generation_of(scope, historical, &self.pointers, rows);
            self.indexed = self.index(growing);
        }
        self.complete
    }

    /// Buckets the admitted rows by key, unless storage cannot hold them
    /// (only a growing worker allocates).
    fn index(&mut self, growing: bool) -> bool {
        let len = self.pointers.len();
        let additional = len.saturating_sub(self.by_key.len());
        if self.by_key.capacity() < len
            && (!growing || reserve_exact(&mut self.by_key, additional).is_err())
        {
            return false;
        }
        self.by_key.clear();
        self.by_key.resize(len, 0);
        let rows = &self.rows[..len * ROW];
        let key = |row: &[u8]| u32::from_le_bytes(row[..4].try_into().unwrap()) as usize % BUCKETS;
        self.starts.fill(0);
        for row in rows.chunks_exact(ROW) {
            self.starts[key(row) + 1] += 1;
        }
        for bucket in 0..BUCKETS {
            self.starts[bucket + 1] += self.starts[bucket];
        }
        let mut next = self.starts;
        for (index, row) in rows.chunks_exact(ROW).enumerate() {
            let at = &mut next[key(row)];
            self.by_key[*at as usize] = index as u32;
            *at += 1;
        }
        true
    }

    /// Whether this admitted census was gathered in `scope` from exactly
    /// `census`'s occurrences, ids and historical length.
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

    /// The admitted content's generation, which names its scope too; zero
    /// for none.
    pub fn generation(&self) -> u64 {
        if self.complete { self.generation } else { 0 }
    }

    /// Occurrences in roster order, only for the scope they were acquired in.
    pub fn admitted(&self, base: usize, game_state: usize) -> Option<Admitted<'_>> {
        (self.complete && self.scope == (base, game_state)).then(|| Admitted {
            pointers: &self.pointers,
            rows: &self.rows[..self.pointers.len() * ROW],
            historical: self.historical,
            generation: self.generation,
            index: self.indexed.then_some((&self.by_key[..], &self.starts)),
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Admitted<'a> {
    pointers: &'a [usize],
    rows: &'a [u8],
    pub historical: usize,
    /// Equal nonzero generations name byte-identical censuses ([`History`]).
    pub generation: u64,
    index: Option<(&'a [u32], &'a [u32; BUCKETS + 1])>,
}

impl<'a> Admitted<'a> {
    /// The indices of the occurrences whose character id `check` covers: the
    /// covered buckets' rows (`covers` depends only on a key's bucket, as
    /// both rotations divide it), or every row filtered without an index.
    pub fn covered(self, check: crate::sampled_verify::Check) -> impl Iterator<Item = usize> + 'a {
        let indexed = self.index.map(move |(by_key, starts)| {
            (0..BUCKETS)
                .filter(move |&bucket| check.covers(bucket as u64))
                .flat_map(move |bucket| {
                    by_key[starts[bucket] as usize..starts[bucket + 1] as usize]
                        .iter()
                        .map(|&index| index as usize)
                })
        });
        let filtered = self.index.is_none().then(|| {
            (0..self.len())
                .filter(move |&index| check.covers(u64::from(self.occurrence(index).id())))
        });
        indexed
            .into_iter()
            .flatten()
            .chain(filtered.into_iter().flatten())
    }
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

/// One owned census occurrence; role payloads retain the ones they decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Occurrence {
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

    /// The role fields the character-input walk reads from.
    pub fn occupant(&self) -> engine::character_inputs::Occupant {
        engine::character_inputs::Occupant {
            pointer: self.pointer,
            character: self.id(),
            leaders: self.leaders(),
            advisors: self.advisors(),
            scientist: self.scientist(),
            operative: self.operative(),
        }
    }

    /// The fields `View::read` would copy for the given required domains
    /// (names, country-leader map, unit pointer, advisor map).
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

/// Map header fields that `RoleTree::read` checks before its sentinel read.
/// `None` fails before that read.
pub(crate) fn map_head(header: [u8; 16]) -> Option<(usize, u32)> {
    let head = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
        .ok()
        .filter(|&head| head != 0)?;
    let count = u32::try_from(u64::from_le_bytes(header[8..].try_into().unwrap())).ok()?;
    Some((head, count))
}

/// Whether the sentinel read lies inside the fields a character `View` owns;
/// such a read never reaches foreign memory and cannot be gathered.
pub(crate) fn inside_view(pointer: usize, head: usize, len: usize) -> bool {
    head < pointer.saturating_add(ccharacter::VIEW_EXTENT) && head.saturating_add(len) > pointer
}

/// Sentinel reads gathered per guarded call.
pub(crate) const SENTINEL_BATCH: usize = 128;
pub(crate) const SENTINEL: usize = 26;
const SENTINEL_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: SENTINEL,
}];
pub(crate) const SENTINEL_LAYOUT: RecordLayout<'static> =
    match RecordLayout::new(&SENTINEL_FIELDS, SENTINEL) {
        Some(layout) => layout,
        None => panic!("sentinel layout"),
    };

/// Gather up to `SENTINEL_BATCH` sentinels, hinting them all first. Returns
/// the completed prefix; the next one failed or was not addressable.
pub(crate) fn gather_sentinels(
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

/// The roster job's role census held between gathers
/// ([`crate::census_changes`]). A census is kept, without gathering it, when
/// the census domain's token equals the one taken before the gather that
/// admitted it and the hour's occurrences, ids, scope and historical length
/// are the ones it was gathered from: no observed writer of a gathered field
/// ran since. The hour's slice of its rows (by character id, every row in
/// verify mode) is still gathered and compared; a difference poisons the
/// domain and the hour gathers the whole census.
pub(crate) struct CensusCarry {
    changes: &'static dyn crate::census_changes::Changes,
    /// Tests: the slice; production follows the hour.
    check: Option<crate::sampled_verify::Check>,
    /// Taken before the gather that admitted the held census.
    token: Option<(u64, u64)>,
    /// The portrait census stamp that gather read its occurrences from.
    stamp: Option<u64>,
    /// The slice's rows, occurrences and fresh rows.
    slice: Vec<u32>,
    pointers: Vec<usize>,
    rows: Vec<u8>,
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
        Self::with(&crate::census_changes::Observer, None)
    }
}

impl CensusCarry {
    pub(crate) fn with(
        changes: &'static dyn crate::census_changes::Changes,
        check: Option<crate::sampled_verify::Check>,
    ) -> Self {
        Self {
            changes,
            check,
            token: None,
            stamp: None,
            slice: Vec::new(),
            pointers: Vec::new(),
            rows: Vec::new(),
        }
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        self.slice.capacity() * size_of::<u32>()
            + self.pointers.capacity() * size_of::<usize>()
            + self.rows.capacity()
    }

    #[cfg(test)]
    pub(crate) fn set_check(&mut self, check: crate::sampled_verify::Check) {
        self.check = Some(check);
    }

    /// The roster job's gather of `census`, the rows of the portrait census
    /// with `stamp`, into `roles`. Returns whether `roles` is admitted for
    /// the hour, and how.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn acquire(
        &mut self,
        memory: &impl Memory,
        roles: &mut RoleCensus,
        scope: (usize, usize),
        census: &[PortraitIdentity],
        stamp: u64,
        historical: usize,
        growing: bool,
    ) -> (bool, crate::census_changes::Counts) {
        let mut counts = crate::census_changes::Counts::default();
        let token = self.changes.quiet();
        // The rows it was gathered from need no comparison.
        let same_occurrences = || {
            if self.stamp == Some(stamp) {
                roles.complete && roles.scope == scope && roles.historical == historical
            } else {
                roles.holds(scope, census, historical)
            }
        };
        if token.is_some()
            && token == self.token
            && same_occurrences()
            && self.compare(memory, roles, &mut counts)
        {
            counts.carried = true;
            return (true, counts);
        }
        counts.gathered = true;
        let admitted = roles.acquire_with_growth(memory, scope, census, historical, growing);
        self.token = token.filter(|_| admitted);
        self.stamp = admitted.then_some(stamp);
        (admitted, counts)
    }

    /// Gathers the slice's rows afresh and compares them with the held ones.
    /// False at a difference, which poisons the domain, or when the slice
    /// cannot be gathered.
    fn compare(
        &mut self,
        memory: &impl Memory,
        roles: &RoleCensus,
        counts: &mut crate::census_changes::Counts,
    ) -> bool {
        let check = self.check.unwrap_or_else(|| {
            crate::sampled_verify::current_burned_in(crate::census_changes::VERIFY)
        });
        let Some(held) = roles.admitted(roles.scope.0, roles.scope.1) else {
            return false;
        };
        self.slice.clear();
        let covered = held.covered(check).count();
        if engine::allocation::advisory(|| engine::allocation::reserve(&mut self.slice, covered))
            .is_err()
        {
            return false;
        }
        self.slice
            .extend(held.covered(check).map(|index| index as u32));
        let len = self.slice.len();
        self.pointers.clear();
        let stored = engine::allocation::advisory(|| {
            engine::allocation::reserve(&mut self.pointers, len)?;
            engine::allocation::resize_default(&mut self.rows, len * ROW)
        });
        if stored.is_err() {
            return false;
        }
        self.pointers.extend(
            self.slice
                .iter()
                .map(|&index| roles.pointers[index as usize]),
        );
        let fresh = &mut self.rows[..len * ROW];
        if memory.copy_records(RecordSources::Pointers(&self.pointers), &LAYOUT, fresh)
            != len * FIELDS.len()
        {
            return false;
        }
        counts.compared = len as u64;
        counts.mismatches = self
            .slice
            .iter()
            .zip(fresh.chunks_exact(ROW))
            .filter(|&(&index, fresh)| {
                roles.rows[index as usize * ROW + 4..][..ROW - 4] != fresh[4..]
            })
            .count() as u64;
        if counts.mismatches != 0 {
            self.changes.mismatch();
            self.token = None;
        }
        counts.mismatches == 0
    }
}
