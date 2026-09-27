//! Role fields of the roster job's admitted census, loaned to the role jobs
//! of the same stopped-world callback so they need not walk the roster again.

use engine::{
    Memory, ccharacter,
    ccharacter_manager::PortraitIdentity,
    memory::{RecordField, RecordLayout, RecordSources},
};

/// One occurrence: character id, then the exact bytes at `COUNTRY` and at
/// `LEADER_MAP_HEAD..ADVISOR_MAP_SIZE + 8`, which hold the leader map header,
/// the unit leader pointer and the advisor map header in that order.
pub(crate) const ROW: usize = 48;
const ROLE_SPAN: usize = ccharacter::ADVISOR_MAP_SIZE + 8 - ccharacter::LEADER_MAP_HEAD;
const _: () = assert!(
    ccharacter::LEADER_MAP_SIZE == ccharacter::LEADER_MAP_HEAD + 8
        && ccharacter::UNIT_LEADER == ccharacter::LEADER_MAP_HEAD + 16
        && ccharacter::ADVISOR_MAP_HEAD == ccharacter::LEADER_MAP_HEAD + 24
        && ROLE_SPAN == 40
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
/// pass, from the occurrences of its complete portrait census. Storage follows
/// the live roster: a longer roster is not published and grows off-thread.
#[derive(Debug)]
pub(crate) struct RoleCensus {
    pointers: Vec<usize>,
    rows: Vec<u8>,
    historical: usize,
    scope: (usize, usize),
    complete: bool,
    required: usize,
}

/// The occurrence count of the most recent role census the roster job
/// acquired. Role jobs size their visit lists from it: a pass visits at most
/// every occurrence.
static LIVE_LEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Occurrences in the live roster, as last observed; 0 before the first
/// role census.
pub(crate) fn live_len() -> usize {
    LIVE_LEN.load(std::sync::atomic::Ordering::Relaxed)
}

impl RoleCensus {
    /// Starting capacity only; the first capture contained about 13,000 characters.
    pub const INITIAL_CAPACITY: usize = 16_384;

    pub fn new(capacity: usize) -> Self {
        Self {
            pointers: Vec::with_capacity(capacity),
            rows: vec![0; capacity * ROW],
            historical: 0,
            scope: (0, 0),
            complete: false,
            required: 0,
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.pointers.capacity() * size_of::<usize>() + self.rows.capacity()
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
            self.pointers
                .reserve_exact(self.required - self.pointers.len());
            self.rows
                .resize((self.required * ROW).max(self.rows.len()), 0);
        }
    }

    /// Gather every occurrence's fields in one guarded call. `census` must be
    /// complete and freshly certified for `scope` in this callback; the
    /// census is admitted only when every field of every occurrence copied.
    pub fn acquire(
        &mut self,
        memory: &impl Memory,
        scope: (usize, usize),
        census: &[PortraitIdentity],
        historical: usize,
    ) -> bool {
        self.scope = scope;
        self.complete = false;
        self.required = census.len();
        LIVE_LEN.store(census.len(), std::sync::atomic::Ordering::Relaxed);
        self.historical = historical;
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
        self.complete
    }

    /// Occurrences in roster order, only for the scope they were acquired in.
    pub fn admitted(&self, base: usize, game_state: usize) -> Option<Admitted<'_>> {
        (self.complete && self.scope == (base, game_state)).then(|| Admitted {
            pointers: &self.pointers,
            rows: &self.rows[..self.pointers.len() * ROW],
            historical: self.historical,
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Admitted<'a> {
    pointers: &'a [usize],
    rows: &'a [u8],
    pub historical: usize,
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
