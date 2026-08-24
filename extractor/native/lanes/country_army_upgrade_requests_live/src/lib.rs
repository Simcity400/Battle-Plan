use std::collections::BTreeSet;

pub const DOMAIN_ID: u16 = 93;
pub const DOMAIN_NAME: &str = "country_army_upgrade_requests_live";
pub const MAGIC: u32 = u32::from_le_bytes(*b"AUR1");
pub const SCHEMA_VERSION: u32 = 1;

pub const AUTHORITY_SOURCE_UNAVAILABLE: u32 = 0;
pub const AUTHORITY_EXACT_A729_RECEIPT: u32 = 1;

pub const COUNTRY_UPGRADE_STATUS_PTR_OFF: u64 = 0xF80;
pub const STATUS_VTABLE_RVA: u64 = 0x29BB408;
pub const STATUS_PERSISTENT_VTABLE_OFF: u64 = 0x18;
pub const STATUS_PERSISTENT_VTABLE_RVA: u64 = 0x29BB440;
pub const STATUS_GROUP_BEGIN_OFF: u64 = 0x20;
pub const STATUS_GROUP_CAPACITY_OFF: u64 = 0x28;
pub const STATUS_GROUP_COUNT_OFF: u64 = 0x2C;

pub const REQUESTS_VTABLE_RVA: u64 = 0x29BB380;
pub const REQUESTS_VERSION_OFF: u64 = 0x08;
pub const REQUESTS_VERSION: u32 = 1;
pub const REQUESTS_PERSISTENT_VTABLE_OFF: u64 = 0x18;
pub const REQUESTS_PERSISTENT_VTABLE_RVA: u64 = 0x29BB3B8;
pub const REQUESTS_ARMY_PTR_OFF: u64 = 0x20;
pub const REQUESTS_STATUS_PTR_OFF: u64 = 0x28;
pub const REQUEST_ROW_BEGIN_OFF: u64 = 0x30;
pub const REQUEST_ROW_CAPACITY_OFF: u64 = 0x38;
pub const REQUEST_ROW_COUNT_OFF: u64 = 0x3C;
pub const REQUEST_ROW_STRIDE: u64 = 0x18;
pub const REQUEST_ROW_EQUIPMENT_PTR_OFF: u64 = 0x00;
pub const REQUEST_ROW_REQUEST_OFF: u64 = 0x08;
pub const REQUEST_ROW_TOTAL_OFF: u64 = 0x10;
pub const DELIVERY_BEGIN_OFF: u64 = 0x48;
pub const DELIVERY_CAPACITY_OFF: u64 = 0x50;
pub const DELIVERY_COUNT_OFF: u64 = 0x54;

pub const ARMY_VTABLE_RVA: u64 = 0x2943AD0;
pub const ARMY_PERSISTENT_VTABLE_OFF: u64 = 0x10;
pub const ARMY_PERSISTENT_VTABLE_RVA: u64 = 0x2943CB0;
pub const ARMY_ID_OFF: u64 = 0x18;
pub const ARMY_ID_TYPE: u32 = 4_713;
pub const ARMY_ID_ALT_TYPE: u32 = 51;

pub fn is_army_identity_type(kind: u32) -> bool {
    kind == ARMY_ID_TYPE || kind == ARMY_ID_ALT_TYPE
}

pub const EQUIPMENT_VARIANT_VTABLE_RVA: u64 = 0x293AFB8;
pub const EQUIPMENT_VARIANT_ID_OFF: u64 = 0x08;
pub const EQUIPMENT_VARIANT_ID_TYPE: u32 = 70;

pub const DELIVERY_VTABLE_RVA: u64 = 0x29BB2E0;
pub const DELIVERY_STATUS_OFF: u64 = 0x08;
pub const DELIVERY_PROGRESS_OFF: u64 = 0x10;
pub const DELIVERY_TOTAL_PROGRESS_OFF: u64 = 0x18;
pub const DELIVERY_PRODUCED_POOL_OFF: u64 = 0x20;
pub const EQUIPMENT_VARIANT_POOL_VTABLE_RVA: u64 = 0x27330A0;
pub const PRODUCED_ROW_BEGIN_OFF: u64 = 0x40;
pub const PRODUCED_ROW_CAPACITY_OFF: u64 = 0x48;
pub const PRODUCED_ROW_COUNT_OFF: u64 = 0x4C;
pub const PRODUCED_ROW_ALLOCATOR_OFF: u64 = 0x50;
pub const PRODUCED_ALLOW_ZERO_OFF: u64 = 0x58;
pub const PRODUCED_ROW_STRIDE: u64 = 0x10;

pub const FIXED_POINT_SCALE: i64 = 100_000;
pub const MAX_COUNTRIES: usize = i32::MAX as usize;

const MIN_COUNTRY_ENTRY_LEN: usize = 8;
pub const MAX_GROUPS_PER_COUNTRY: usize = 4_096;
pub const MAX_REQUEST_ROWS_PER_GROUP: usize = 4_096;
pub const MAX_DELIVERIES_PER_GROUP: usize = 4_096;
pub const MAX_PRODUCED_ROWS_PER_DELIVERY: usize = 4_096;
pub const MAX_TOTAL_LEAF_ROWS: usize = 250_000;

pub const EXECUTABLE_SHA256: [u8; 32] =
    hex32(b"870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad");
pub const STATUS_REQUEST_PERSISTENCE_MANIFEST_SHA256: [u8; 32] =
    hex32(b"316b91f5773aa57d8688233330fe014ebed49e6efac3d4a6e14bcad8d3c97dd4");
pub const DELIVERY_PERSISTENCE_MANIFEST_SHA256: [u8; 32] =
    hex32(b"637f5a3e3a9c92a9d7243c93fc2e729c6b4fec01dc9172a4d001657d1eda95d4");
pub const PRODUCED_POOL_PERSISTENCE_MANIFEST_SHA256: [u8; 32] =
    hex32(b"702cae9cd3537e2864eb83a4b8c2915ed6c30473dabfcf7dc24af2f42a45b556");
pub const CARMY_IDENTITY_MANIFEST_SHA256: [u8; 32] =
    hex32(b"f14032b6a225a4d5ff908b2296cd36d510a89aaa9670bd8f3597d5947e7fb6c7");
pub const LIVE_AST_ROWS_SHA256: [u8; 32] =
    hex32(b"5fe6a7095288b5df235578ddd414fc774858a42cc69f6ec14117b4baaa08c352");
pub const LIVE_AST_ARMY_IDS_SHA256: [u8; 32] =
    hex32(b"06e8772cb26c2963ec3113825590b386a197a8589b833866fc2e3cefeaaf8cdd");

const fn hex32(input: &[u8; 64]) -> [u8; 32] {
    const fn nibble(value: u8) -> u8 {
        match value {
            b'0'..=b'9' => value - b'0',
            b'a'..=b'f' => value - b'a' + 10,
            b'A'..=b'F' => value - b'A' + 10,
            _ => 0xff,
        }
    }
    let mut out = [0; 32];
    let mut index = 0;
    while index < 32 {
        out[index] = (nibble(input[index * 2]) << 4) | nibble(input[index * 2 + 1]);
        index += 1;
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PersistentId {
    pub kind: u32,
    pub id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmyUpgradeRequestRow {
    pub equipment_variant: PersistentId,
    pub request_raw: i64,
    pub total_raw: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducedEquipmentRow {
    pub equipment_variant: PersistentId,
    pub amount_raw: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradeDeliveryRow {
    pub status: u32,
    pub progress_raw: Option<i64>,
    pub total_progress_raw: i64,
    pub allow_zero_entries: bool,
    pub produced: Vec<ProducedEquipmentRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmyUpgradeGroup {
    pub army: PersistentId,
    pub requests: Vec<ArmyUpgradeRequestRow>,
    pub deliveries: Vec<UpgradeDeliveryRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountryArmyUpgradeRow {
    pub tag_db_id: u32,
    pub groups: Vec<ArmyUpgradeGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmyUpgradeSnapshot {
    pub source_sequence: u64,
    pub countries: Vec<CountryArmyUpgradeRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityState {
    SourceUnavailable,
    ExactA729Receipt,
}

impl AuthorityState {
    const fn raw(self) -> u32 {
        match self {
            Self::SourceUnavailable => AUTHORITY_SOURCE_UNAVAILABLE,
            Self::ExactA729Receipt => AUTHORITY_EXACT_A729_RECEIPT,
        }
    }

    fn from_raw(value: u32) -> Result<Self, CarrierError> {
        match value {
            AUTHORITY_SOURCE_UNAVAILABLE => Ok(Self::SourceUnavailable),
            AUTHORITY_EXACT_A729_RECEIPT => Ok(Self::ExactA729Receipt),
            _ => invalid("AUR1 authority is unknown"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmyUpgradeFrame {
    pub elapsed_wall_ms: u64,
    pub frame_hour: u64,
    pub status_flags: u32,
    pub authority: AuthorityState,
    pub snapshot: Option<ArmyUpgradeSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountryRoot {
    pub tag_db_id: u32,
    pub object_ptr: u64,
}

pub trait ArmyUpgradeMemory {
    fn read_exact(&self, address: u64, out: &mut [u8]) -> Result<(), ()>;
}

pub trait ExactArmyUpgradeSource {
    fn is_exact_a729_country_army_upgrade_source(&self) -> bool;
    fn read_country_army_upgrades(&self) -> Result<ArmyUpgradeSnapshot, CarrierError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CarrierError {
    Source(String),
    CountryRead {
        tag_db_id: u32,
        object_ptr: u64,
        source: Box<CarrierError>,
    },
    Memory {
        address: u64,
        width: usize,
    },
    Invalid(String),
    Truncated,
    TrailingBytes,
}

/// The engine mutates the AUR1 object graph concurrently with this walk, so a
/// single pass pair can straddle a mutation even though the graph settles
/// again inside the same frame bracket (lane QZ: 18 of 442 d93 frames in the
/// sealed pw_gate_final capture failed the old fixed two-pass check, every
/// one of them while the graph was mid-mutation). The coherent-read contract
/// is therefore restartable: repeat the pass until two consecutive passes
/// agree or the bound is hit. Restarts are plain re-reads -- they never wait,
/// sleep, pause the engine, or widen the frame bracket -- and exhausting the
/// bound still refuses under its own name so a persistently torn graph is
/// never published as measured data.
pub const COHERENT_READ_MAX_PASSES: usize = 8;

// --- widened guarded copies (lane SF) -----------------------------------
//
// Every scalar this walk wants used to cost one guarded read
// (`ArmyUpgradeMemory::read_exact` -> the collector's `read_into` -> the
// region gate plus the guarded copy). On a 439-country wartime frame the walk
// wants ~98.5k of them per pass, and the coherent-read contract above only
// settles when two consecutive passes agree, so a clean frame paid ~197k.
// The bytes each object contributes are contiguous, so they are now taken in
// ONE guarded copy per object, and array entries in one copy per window, with
// the fields decoded out of that copy.
//
// Three properties keep this free of wire consequences:
//
// * SUCCESS IS A SUBSET. A wide copy is served only when the region gate
//   confirms the WHOLE range inside one committed, readable, non-guard
//   region; every sub-range of that range is confirmed by the same test, so a
//   wide copy can only succeed where each scalar read it replaces would also
//   have succeeded.
// * REFUSAL FALLS BACK TO THE OLD STATEMENTS. When the wide copy is refused
//   (a range the gate will not confirm whole, or an address-space edge), the
//   fields are read individually, in the original order, through the original
//   helpers -- so the refusal's address, width and message are unchanged.
// * THE COPY INSTANT ONLY MOVES EARLIER. An object's copy is taken where the
//   old walk issued that object's FIRST read (its vtable, at every site); an
//   array window is copied where the old walk read the first entry it holds.
//   No field is observed LATER than it was, and fields that used to be
//   sampled at several instants -- a delivery's status and its progress, a
//   vector's begin/capacity/count -- now come from one instant. That is
//   strictly more coherent, and the restartable two-pass contract is
//   untouched: it still re-reads the whole graph and still refuses by name
//   when the graph never settles.
//
// On a memory that does not change under the walk -- every offline fixture,
// and the settled live graph -- the two paths produce identical bytes, which
// is what `widened_and_scalar_paths_are_byte_identical` proves.

/// Field extent of `CCountryArmyUpgradeStatus`: vtable, persistent-serializer
/// vtable and the group vector header.
const STATUS_BLOCK_LEN: usize = 0x30;
/// Field extent of the per-army request object: both vtables, the version
/// word, the CArmy and status back-pointers, and the request and delivery
/// vector headers.
const GROUP_BLOCK_LEN: usize = 0x58;
/// Field extent of `CArmy`: both vtables and the persistent identity.
const ARMY_BLOCK_LEN: usize = 0x20;
/// Field extent of the equipment variant: vtable and persistent identity.
const EQUIPMENT_BLOCK_LEN: usize = 0x10;
/// Field extent of a delivery: vtable, status, both progress words, the
/// embedded produced pool's vtable and vector header, and the allow-zero
/// flag. Every block extent here stops at its last decoded field rather than
/// at the object's natural size: a shorter range is a range the region gate
/// is likelier to confirm whole, and no byte is copied that no reader wants.
const DELIVERY_BLOCK_LEN: usize = 0x59;
/// `{begin: u64, capacity: i32, count: i32}` -- the re-verification extent.
const VECTOR_HEADER_LEN: usize = 0x10;
/// Bytes an array window holds. Sized so that every array this walk meets on
/// a production frame (group slots ~6, request rows ~2, delivery slots and
/// produced rows ~1) is covered by a single copy, while a pathological array
/// at the `MAX_*` bounds still streams through in windows instead of
/// demanding a 98 KB buffer.
const ARRAY_WINDOW_BYTES: usize = 4096;

// The block extents are pinned to the offsets they must cover: an offset edit
// that outruns its block is a compile error, not a silent narrow-read
// regression.
const _: () = {
    assert!(STATUS_PERSISTENT_VTABLE_OFF as usize + 8 <= STATUS_BLOCK_LEN);
    assert!(STATUS_GROUP_BEGIN_OFF as usize + 8 <= STATUS_BLOCK_LEN);
    assert!(STATUS_GROUP_CAPACITY_OFF as usize + 4 <= STATUS_BLOCK_LEN);
    assert!(STATUS_GROUP_COUNT_OFF as usize + 4 <= STATUS_BLOCK_LEN);
    assert!(REQUESTS_VERSION_OFF as usize + 4 <= GROUP_BLOCK_LEN);
    assert!(REQUESTS_PERSISTENT_VTABLE_OFF as usize + 8 <= GROUP_BLOCK_LEN);
    assert!(REQUESTS_ARMY_PTR_OFF as usize + 8 <= GROUP_BLOCK_LEN);
    assert!(REQUESTS_STATUS_PTR_OFF as usize + 8 <= GROUP_BLOCK_LEN);
    assert!(REQUEST_ROW_BEGIN_OFF as usize + 8 <= GROUP_BLOCK_LEN);
    assert!(REQUEST_ROW_CAPACITY_OFF as usize + 4 <= GROUP_BLOCK_LEN);
    assert!(REQUEST_ROW_COUNT_OFF as usize + 4 <= GROUP_BLOCK_LEN);
    assert!(DELIVERY_BEGIN_OFF as usize + 8 <= GROUP_BLOCK_LEN);
    assert!(DELIVERY_CAPACITY_OFF as usize + 4 <= GROUP_BLOCK_LEN);
    assert!(DELIVERY_COUNT_OFF as usize + 4 <= GROUP_BLOCK_LEN);
    assert!(ARMY_PERSISTENT_VTABLE_OFF as usize + 8 <= ARMY_BLOCK_LEN);
    assert!(ARMY_ID_OFF as usize + 8 <= ARMY_BLOCK_LEN);
    assert!(EQUIPMENT_VARIANT_ID_OFF as usize + 8 <= EQUIPMENT_BLOCK_LEN);
    assert!(DELIVERY_STATUS_OFF as usize + 4 <= DELIVERY_BLOCK_LEN);
    assert!(DELIVERY_PROGRESS_OFF as usize + 8 <= DELIVERY_BLOCK_LEN);
    assert!(DELIVERY_TOTAL_PROGRESS_OFF as usize + 8 <= DELIVERY_BLOCK_LEN);
    assert!(DELIVERY_PRODUCED_POOL_OFF as usize + 8 <= DELIVERY_BLOCK_LEN);
    assert!(PRODUCED_ROW_BEGIN_OFF as usize + 8 <= DELIVERY_BLOCK_LEN);
    assert!(PRODUCED_ROW_CAPACITY_OFF as usize + 4 <= DELIVERY_BLOCK_LEN);
    assert!(PRODUCED_ROW_COUNT_OFF as usize + 4 <= DELIVERY_BLOCK_LEN);
    assert!(PRODUCED_ALLOW_ZERO_OFF as usize + 1 <= DELIVERY_BLOCK_LEN);
    // The 16-byte header copy is only the same bytes as the three scalar
    // reads where the triple is contiguous. It is, at all four sites.
    assert!(STATUS_GROUP_BEGIN_OFF + 8 == STATUS_GROUP_CAPACITY_OFF);
    assert!(STATUS_GROUP_CAPACITY_OFF + 4 == STATUS_GROUP_COUNT_OFF);
    assert!(REQUEST_ROW_BEGIN_OFF + 8 == REQUEST_ROW_CAPACITY_OFF);
    assert!(REQUEST_ROW_CAPACITY_OFF + 4 == REQUEST_ROW_COUNT_OFF);
    assert!(DELIVERY_BEGIN_OFF + 8 == DELIVERY_CAPACITY_OFF);
    assert!(DELIVERY_CAPACITY_OFF + 4 == DELIVERY_COUNT_OFF);
    assert!(PRODUCED_ROW_BEGIN_OFF + 8 == PRODUCED_ROW_CAPACITY_OFF);
    assert!(PRODUCED_ROW_CAPACITY_OFF + 4 == PRODUCED_ROW_COUNT_OFF);
    // Both row strides must hold every field the window decodes out of them.
    assert!(REQUEST_ROW_STRIDE >= REQUEST_ROW_TOTAL_OFF + 8);
    assert!(PRODUCED_ROW_STRIDE >= 16);
};

/// One guarded copy of an engine object's field extent, or `None` when the
/// copy was refused and every field must be read on its own.
struct ObjectBlock<const N: usize> {
    base: u64,
    bytes: Option<[u8; N]>,
}

/// Take the copy. A base whose extent would leave the address space is not
/// copied: the per-field `add` checks then produce the address-overflow
/// refusal they always did.
fn read_block<const N: usize, M: ArmyUpgradeMemory>(memory: &M, base: u64) -> ObjectBlock<N> {
    let mut bytes = [0; N];
    let copied =
        base.checked_add(N as u64).is_some() && memory.read_exact(base, &mut bytes).is_ok();
    ObjectBlock {
        base,
        bytes: copied.then_some(bytes),
    }
}

impl<const N: usize> ObjectBlock<N> {
    #[inline]
    fn field(&self, offset: u64, width: usize) -> Option<&[u8]> {
        let start = usize::try_from(offset).ok()?;
        let end = start.checked_add(width)?;
        debug_assert!(
            end <= N,
            "AUR1 block field {} + {} is outside its {}-byte block",
            offset,
            width,
            N
        );
        self.bytes.as_ref()?.get(start..end)
    }

    fn u8_at<M: ArmyUpgradeMemory>(&self, memory: &M, offset: u64) -> Result<u8, CarrierError> {
        match self.field(offset, 1) {
            Some(raw) => Ok(raw[0]),
            None => read_u8(memory, add(self.base, offset)?),
        }
    }

    fn u32_at<M: ArmyUpgradeMemory>(&self, memory: &M, offset: u64) -> Result<u32, CarrierError> {
        match self.field(offset, 4) {
            Some(raw) => Ok(u32::from_le_bytes(raw.try_into().unwrap())),
            None => read_u32(memory, add(self.base, offset)?),
        }
    }

    fn i32_at<M: ArmyUpgradeMemory>(&self, memory: &M, offset: u64) -> Result<i32, CarrierError> {
        match self.field(offset, 4) {
            Some(raw) => Ok(i32::from_le_bytes(raw.try_into().unwrap())),
            None => read_i32(memory, add(self.base, offset)?),
        }
    }

    fn u64_at<M: ArmyUpgradeMemory>(&self, memory: &M, offset: u64) -> Result<u64, CarrierError> {
        match self.field(offset, 8) {
            Some(raw) => Ok(u64::from_le_bytes(raw.try_into().unwrap())),
            None => read_u64(memory, add(self.base, offset)?),
        }
    }

    fn i64_at<M: ArmyUpgradeMemory>(&self, memory: &M, offset: u64) -> Result<i64, CarrierError> {
        match self.field(offset, 8) {
            Some(raw) => Ok(i64::from_le_bytes(raw.try_into().unwrap())),
            None => read_i64(memory, add(self.base, offset)?),
        }
    }

    fn id_at<M: ArmyUpgradeMemory>(
        &self,
        memory: &M,
        offset: u64,
    ) -> Result<PersistentId, CarrierError> {
        match self.field(offset, 8) {
            Some(raw) => Ok(PersistentId {
                kind: u32::from_le_bytes(raw[..4].try_into().unwrap()),
                id: u32::from_le_bytes(raw[4..].try_into().unwrap()),
            }),
            None => read_persistent_id(memory, add(self.base, offset)?),
        }
    }

    /// Same refusal text and same address arithmetic order as `require_vtable`
    /// -- the object address first, then the expected image address.
    fn require_vtable_at<M: ArmyUpgradeMemory>(
        &self,
        memory: &M,
        offset: u64,
        module_base: u64,
        expected_rva: u64,
    ) -> Result<(), CarrierError> {
        match self.field(offset, 8) {
            Some(raw) => {
                let object = add(self.base, offset)?;
                let expected = add(module_base, expected_rva)?;
                if u64::from_le_bytes(raw.try_into().unwrap()) != expected {
                    return invalid(format!("AUR1 vtable mismatch at 0x{object:x}"));
                }
                Ok(())
            }
            None => require_vtable(memory, add(self.base, offset)?, module_base, expected_rva),
        }
    }

    fn vector_header_at<M: ArmyUpgradeMemory>(
        &self,
        memory: &M,
        begin_off: u64,
        capacity_off: u64,
        count_off: u64,
        maximum: usize,
        label: &str,
    ) -> Result<(u64, usize), CarrierError> {
        let begin = self.u64_at(memory, begin_off)?;
        let capacity = self.i32_at(memory, capacity_off)?;
        let count = self.i32_at(memory, count_off)?;
        vector_header_from_fields(begin, capacity, count, maximum, label)
    }
}

/// A sequential window over a fixed-stride engine array. Entries are served
/// out of one guarded copy of up to `buffer.len()` bytes, refilled when the
/// walk steps past the window; `None` means no window covers that entry and
/// the caller must read the entry's fields itself. A window copies exactly
/// the bytes the per-entry field reads would have read -- the entries are
/// adjacent and every field lies inside the stride -- so no byte outside the
/// array is touched.
struct ArrayWindow<'buffer> {
    begin: u64,
    stride: u64,
    count: usize,
    buffer: &'buffer mut [u8],
    first: usize,
    held: usize,
    refused: bool,
}

impl<'buffer> ArrayWindow<'buffer> {
    fn new(begin: u64, stride: u64, count: usize, buffer: &'buffer mut [u8]) -> Self {
        Self {
            begin,
            stride,
            count,
            buffer,
            first: 0,
            held: 0,
            refused: false,
        }
    }

    fn entry<M: ArmyUpgradeMemory>(&mut self, memory: &M, index: usize) -> Option<&[u8]> {
        let stride = usize::try_from(self.stride).ok()?;
        if stride == 0 || index >= self.count {
            return None;
        }
        if index < self.first || index >= self.first.saturating_add(self.held) {
            // One refusal disables the window for the rest of the array: a
            // range this gate will not confirm whole does not start being
            // confirmed, and re-asking once per entry would spend exactly the
            // reads the window exists to save.
            if self.refused {
                return None;
            }
            let want = (self.count - index).min(self.buffer.len() / stride);
            if want == 0 {
                // The buffer cannot hold one entry of this stride.
                self.refused = true;
                self.held = 0;
                return None;
            }
            let length = want * stride;
            let base = (index as u64)
                .checked_mul(self.stride)
                .and_then(|offset| self.begin.checked_add(offset))
                .filter(|base| base.checked_add(length as u64).is_some());
            let Some(base) = base else {
                self.refused = true;
                self.held = 0;
                return None;
            };
            if memory.read_exact(base, &mut self.buffer[..length]).is_err() {
                self.refused = true;
                self.held = 0;
                return None;
            }
            self.first = index;
            self.held = want;
        }
        let at = (index - self.first) * stride;
        self.buffer.get(at..at + stride)
    }
}

/// The four array windows, allocated once per coherent read and reused by
/// every pass and every object, so widening the array reads adds no
/// allocation to the walk.
struct WalkBuffers {
    group_slots: Vec<u8>,
    request_rows: Vec<u8>,
    delivery_slots: Vec<u8>,
    produced_rows: Vec<u8>,
}

impl WalkBuffers {
    fn new() -> Self {
        Self {
            group_slots: vec![0; ARRAY_WINDOW_BYTES],
            request_rows: vec![0; ARRAY_WINDOW_BYTES],
            delivery_slots: vec![0; ARRAY_WINDOW_BYTES],
            produced_rows: vec![0; ARRAY_WINDOW_BYTES],
        }
    }
}

pub fn read_snapshot_from_country_roots<M: ArmyUpgradeMemory>(
    memory: &M,
    module_base: u64,
    source_sequence: u64,
    roots: &[CountryRoot],
) -> Result<ArmyUpgradeSnapshot, CarrierError> {
    let mut buffers = WalkBuffers::new();
    let mut previous =
        read_snapshot_once(memory, module_base, source_sequence, roots, &mut buffers)?;
    for _pass in 1..COHERENT_READ_MAX_PASSES {
        let current =
            read_snapshot_once(memory, module_base, source_sequence, roots, &mut buffers)?;
        if current == previous {
            return Ok(current);
        }
        previous = current;
    }
    invalid("AUR1 object graph kept changing across every coherent-read restart")
}

fn read_snapshot_once<M: ArmyUpgradeMemory>(
    memory: &M,
    module_base: u64,
    source_sequence: u64,
    roots: &[CountryRoot],
    buffers: &mut WalkBuffers,
) -> Result<ArmyUpgradeSnapshot, CarrierError> {
    if module_base == 0 || source_sequence == 0 || roots.len() > MAX_COUNTRIES {
        return invalid("AUR1 country-root snapshot header is invalid");
    }
    let mut country_ids = BTreeSet::new();
    let mut countries = Vec::with_capacity(roots.len());
    let mut total_leaf_rows = 0usize;
    for root in roots {
        if root.tag_db_id == 0 || root.object_ptr == 0 || !country_ids.insert(root.tag_db_id) {
            return invalid("AUR1 country identity is zero or duplicated");
        }
        let mut groups =
            read_country_groups(memory, module_base, root, &mut total_leaf_rows, buffers).map_err(
                |source| CarrierError::CountryRead {
                    tag_db_id: root.tag_db_id,
                    object_ptr: root.object_ptr,
                    source: Box::new(source),
                },
            )?;
        groups.sort_by_key(|group| group.army);
        countries.push(CountryArmyUpgradeRow {
            tag_db_id: root.tag_db_id,
            groups,
        });
    }
    countries.sort_by_key(|country| country.tag_db_id);
    let snapshot = ArmyUpgradeSnapshot {
        source_sequence,
        countries,
    };
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

/// One country's army-upgrade groups. A named function rather than the
/// immediately-invoked closure it was, so the four array windows can be
/// borrowed field by field by the levels that use them.
fn read_country_groups<M: ArmyUpgradeMemory>(
    memory: &M,
    module_base: u64,
    root: &CountryRoot,
    total_leaf_rows: &mut usize,
    buffers: &mut WalkBuffers,
) -> Result<Vec<ArmyUpgradeGroup>, CarrierError> {
    let WalkBuffers {
        group_slots,
        request_rows,
        delivery_slots,
        produced_rows,
    } = buffers;
    let owner = read_u64(
        memory,
        add(root.object_ptr, COUNTRY_UPGRADE_STATUS_PTR_OFF)?,
    )?;
    let mut groups = Vec::new();
    if owner == 0 {
        return Ok(groups);
    }
    let status = read_block::<STATUS_BLOCK_LEN, _>(memory, owner);
    status.require_vtable_at(memory, 0, module_base, STATUS_VTABLE_RVA)?;
    status.require_vtable_at(
        memory,
        STATUS_PERSISTENT_VTABLE_OFF,
        module_base,
        STATUS_PERSISTENT_VTABLE_RVA,
    )?;
    let (begin, count) = status.vector_header_at(
        memory,
        STATUS_GROUP_BEGIN_OFF,
        STATUS_GROUP_CAPACITY_OFF,
        STATUS_GROUP_COUNT_OFF,
        MAX_GROUPS_PER_COUNTRY,
        "army-upgrade group",
    )?;
    let mut army_ids = BTreeSet::new();
    groups.reserve(count);
    let mut slots = ArrayWindow::new(begin, 8, count, group_slots.as_mut_slice());
    for index in 0..count {
        let group_ptr = match slots.entry(memory, index) {
            Some(raw) => u64::from_le_bytes(raw.try_into().unwrap()),
            None => read_u64(memory, add(begin, (index as u64) * 8)?)?,
        };
        if group_ptr == 0 {
            return invalid("AUR1 group pointer is null");
        }
        let group = read_block::<GROUP_BLOCK_LEN, _>(memory, group_ptr);
        group.require_vtable_at(memory, 0, module_base, REQUESTS_VTABLE_RVA)?;
        group.require_vtable_at(
            memory,
            REQUESTS_PERSISTENT_VTABLE_OFF,
            module_base,
            REQUESTS_PERSISTENT_VTABLE_RVA,
        )?;
        if group.u32_at(memory, REQUESTS_VERSION_OFF)? != REQUESTS_VERSION {
            return invalid("AUR1 request object version is not exact a729 version 1");
        }
        if group.u64_at(memory, REQUESTS_STATUS_PTR_OFF)? != owner {
            return invalid("AUR1 request object status back-reference disagrees");
        }
        let army_ptr = group.u64_at(memory, REQUESTS_ARMY_PTR_OFF)?;
        if army_ptr == 0 {
            return invalid("AUR1 request object has null CArmy identity");
        }
        let army_object = read_block::<ARMY_BLOCK_LEN, _>(memory, army_ptr);
        army_object.require_vtable_at(memory, 0, module_base, ARMY_VTABLE_RVA)?;
        army_object.require_vtable_at(
            memory,
            ARMY_PERSISTENT_VTABLE_OFF,
            module_base,
            ARMY_PERSISTENT_VTABLE_RVA,
        )?;
        let army = army_object.id_at(memory, ARMY_ID_OFF)?;
        if !is_army_identity_type(army.kind) || army.id == 0 || !army_ids.insert(army) {
            return invalid("AUR1 CArmy persistent identity is invalid or duplicated");
        }
        let requests =
            read_request_rows(memory, module_base, group_ptr, &group, &mut *request_rows)?;
        let deliveries = read_deliveries(
            memory,
            module_base,
            group_ptr,
            &group,
            &mut *delivery_slots,
            &mut *produced_rows,
        )?;
        *total_leaf_rows = total_leaf_rows
            .checked_add(requests.len())
            .and_then(|value| value.checked_add(deliveries.len()))
            .and_then(|value| {
                deliveries
                    .iter()
                    .try_fold(value, |sum, row| sum.checked_add(row.produced.len()))
            })
            .ok_or_else(|| CarrierError::Invalid("AUR1 leaf count overflow".into()))?;
        if *total_leaf_rows > MAX_TOTAL_LEAF_ROWS {
            return invalid("AUR1 total leaf count exceeds bound");
        }
        groups.push(ArmyUpgradeGroup {
            army,
            requests,
            deliveries,
        });
    }
    if reread_vector_header(
        memory,
        owner,
        STATUS_GROUP_BEGIN_OFF,
        STATUS_GROUP_CAPACITY_OFF,
        STATUS_GROUP_COUNT_OFF,
        MAX_GROUPS_PER_COUNTRY,
        "army-upgrade group",
    )? != (begin, count)
        || read_u64(
            memory,
            add(root.object_ptr, COUNTRY_UPGRADE_STATUS_PTR_OFF)?,
        )? != owner
    {
        return invalid("AUR1 country/status graph changed during snapshot");
    }
    Ok(groups)
}

fn read_request_rows<M: ArmyUpgradeMemory>(
    memory: &M,
    module_base: u64,
    group_ptr: u64,
    group: &ObjectBlock<GROUP_BLOCK_LEN>,
    window: &mut [u8],
) -> Result<Vec<ArmyUpgradeRequestRow>, CarrierError> {
    let (begin, count) = group.vector_header_at(
        memory,
        REQUEST_ROW_BEGIN_OFF,
        REQUEST_ROW_CAPACITY_OFF,
        REQUEST_ROW_COUNT_OFF,
        MAX_REQUEST_ROWS_PER_GROUP,
        "army-upgrade request",
    )?;
    let mut rows = Vec::with_capacity(count);
    let mut entries = ArrayWindow::new(begin, REQUEST_ROW_STRIDE, count, window);
    for index in 0..count {
        match entries.entry(memory, index) {
            Some(raw) => {
                let equipment_ptr = u64::from_le_bytes(
                    raw[REQUEST_ROW_EQUIPMENT_PTR_OFF as usize..][..8]
                        .try_into()
                        .unwrap(),
                );
                let request_raw = i64::from_le_bytes(
                    raw[REQUEST_ROW_REQUEST_OFF as usize..][..8]
                        .try_into()
                        .unwrap(),
                );
                let total_raw = i64::from_le_bytes(
                    raw[REQUEST_ROW_TOTAL_OFF as usize..][..8]
                        .try_into()
                        .unwrap(),
                );
                rows.push(ArmyUpgradeRequestRow {
                    equipment_variant: read_equipment_id(memory, module_base, equipment_ptr)?,
                    request_raw,
                    total_raw,
                });
            }
            None => {
                // The pre-window statements, in their original order.
                let row = add(begin, (index as u64) * REQUEST_ROW_STRIDE)?;
                let equipment_ptr = read_u64(memory, add(row, REQUEST_ROW_EQUIPMENT_PTR_OFF)?)?;
                let equipment_variant = read_equipment_id(memory, module_base, equipment_ptr)?;
                rows.push(ArmyUpgradeRequestRow {
                    equipment_variant,
                    request_raw: read_i64(memory, add(row, REQUEST_ROW_REQUEST_OFF)?)?,
                    total_raw: read_i64(memory, add(row, REQUEST_ROW_TOTAL_OFF)?)?,
                });
            }
        }
    }
    if reread_vector_header(
        memory,
        group_ptr,
        REQUEST_ROW_BEGIN_OFF,
        REQUEST_ROW_CAPACITY_OFF,
        REQUEST_ROW_COUNT_OFF,
        MAX_REQUEST_ROWS_PER_GROUP,
        "army-upgrade request",
    )? != (begin, count)
    {
        return invalid("AUR1 request vector changed during snapshot");
    }
    Ok(rows)
}

fn read_deliveries<M: ArmyUpgradeMemory>(
    memory: &M,
    module_base: u64,
    group_ptr: u64,
    group: &ObjectBlock<GROUP_BLOCK_LEN>,
    slot_window: &mut [u8],
    produced_window: &mut [u8],
) -> Result<Vec<UpgradeDeliveryRow>, CarrierError> {
    let (begin, count) = group.vector_header_at(
        memory,
        DELIVERY_BEGIN_OFF,
        DELIVERY_CAPACITY_OFF,
        DELIVERY_COUNT_OFF,
        MAX_DELIVERIES_PER_GROUP,
        "army-upgrade delivery",
    )?;
    let mut rows = Vec::with_capacity(count);
    let mut slots = ArrayWindow::new(begin, 8, count, slot_window);
    for index in 0..count {
        let delivery = match slots.entry(memory, index) {
            Some(raw) => u64::from_le_bytes(raw.try_into().unwrap()),
            None => read_u64(memory, add(begin, (index as u64) * 8)?)?,
        };
        if delivery == 0 {
            return invalid("AUR1 delivery pointer is null");
        }
        let block = read_block::<DELIVERY_BLOCK_LEN, _>(memory, delivery);
        block.require_vtable_at(memory, 0, module_base, DELIVERY_VTABLE_RVA)?;
        block.require_vtable_at(
            memory,
            DELIVERY_PRODUCED_POOL_OFF,
            module_base,
            EQUIPMENT_VARIANT_POOL_VTABLE_RVA,
        )?;
        let (produced_begin, produced_count) = block.vector_header_at(
            memory,
            PRODUCED_ROW_BEGIN_OFF,
            PRODUCED_ROW_CAPACITY_OFF,
            PRODUCED_ROW_COUNT_OFF,
            MAX_PRODUCED_ROWS_PER_DELIVERY,
            "army-upgrade produced equipment",
        )?;
        let mut produced = Vec::with_capacity(produced_count);
        let mut entries = ArrayWindow::new(
            produced_begin,
            PRODUCED_ROW_STRIDE,
            produced_count,
            &mut *produced_window,
        );
        for produced_index in 0..produced_count {
            match entries.entry(memory, produced_index) {
                Some(raw) => {
                    let equipment_ptr = u64::from_le_bytes(raw[..8].try_into().unwrap());
                    let amount_raw = i64::from_le_bytes(raw[8..16].try_into().unwrap());
                    produced.push(ProducedEquipmentRow {
                        equipment_variant: read_equipment_id(memory, module_base, equipment_ptr)?,
                        amount_raw,
                    });
                }
                None => {
                    // The pre-window statements, in their original order.
                    let row = add(
                        produced_begin,
                        (produced_index as u64) * PRODUCED_ROW_STRIDE,
                    )?;
                    let equipment_ptr = read_u64(memory, row)?;
                    produced.push(ProducedEquipmentRow {
                        equipment_variant: read_equipment_id(memory, module_base, equipment_ptr)?,
                        amount_raw: read_i64(memory, add(row, 8)?)?,
                    });
                }
            }
        }
        if reread_vector_header(
            memory,
            delivery,
            PRODUCED_ROW_BEGIN_OFF,
            PRODUCED_ROW_CAPACITY_OFF,
            PRODUCED_ROW_COUNT_OFF,
            MAX_PRODUCED_ROWS_PER_DELIVERY,
            "army-upgrade produced equipment",
        )? != (produced_begin, produced_count)
        {
            return invalid("AUR1 produced vector changed during snapshot");
        }
        let status = block.u32_at(memory, DELIVERY_STATUS_OFF)?;
        rows.push(UpgradeDeliveryRow {
            status,
            progress_raw: if status == 1 {
                Some(block.i64_at(memory, DELIVERY_PROGRESS_OFF)?)
            } else {
                None
            },
            total_progress_raw: block.i64_at(memory, DELIVERY_TOTAL_PROGRESS_OFF)?,
            allow_zero_entries: block.u8_at(memory, PRODUCED_ALLOW_ZERO_OFF)? != 0,
            produced,
        });
    }
    if reread_vector_header(
        memory,
        group_ptr,
        DELIVERY_BEGIN_OFF,
        DELIVERY_CAPACITY_OFF,
        DELIVERY_COUNT_OFF,
        MAX_DELIVERIES_PER_GROUP,
        "army-upgrade delivery",
    )? != (begin, count)
    {
        return invalid("AUR1 delivery vector changed during snapshot");
    }
    Ok(rows)
}

fn read_equipment_id<M: ArmyUpgradeMemory>(
    memory: &M,
    module_base: u64,
    equipment_ptr: u64,
) -> Result<PersistentId, CarrierError> {
    if equipment_ptr == 0 {
        return invalid("AUR1 equipment variant pointer is null");
    }
    let variant = read_block::<EQUIPMENT_BLOCK_LEN, _>(memory, equipment_ptr);
    variant.require_vtable_at(memory, 0, module_base, EQUIPMENT_VARIANT_VTABLE_RVA)?;
    let identity = variant.id_at(memory, EQUIPMENT_VARIANT_ID_OFF)?;
    if identity.kind != EQUIPMENT_VARIANT_ID_TYPE || identity.id == 0 {
        return invalid("AUR1 equipment_variant_index identity is invalid");
    }
    Ok(identity)
}

/// The header admission rules, shared by the block-served read and the
/// scalar-served re-read so the two cannot drift.
fn vector_header_from_fields(
    begin: u64,
    capacity: i32,
    count: i32,
    maximum: usize,
    label: &str,
) -> Result<(u64, usize), CarrierError> {
    if capacity < 0 || count < 0 || count > capacity || count as usize > maximum {
        return invalid(format!("AUR1 {label} vector header is invalid"));
    }
    if count != 0 && begin == 0 {
        return invalid(format!("AUR1 {label} vector begin is null"));
    }
    Ok((begin, count as usize))
}

/// A FRESH copy of a `{begin, capacity, count}` vector header: the
/// re-verification every AUR1 vector level performs after walking its
/// entries. Exactly the bytes the three scalar reads read (the triple is
/// contiguous at all four sites, asserted above), in one guarded copy, with
/// those same three scalar reads as the refusal fallback.
fn reread_vector_header<M: ArmyUpgradeMemory>(
    memory: &M,
    object: u64,
    begin_off: u64,
    capacity_off: u64,
    count_off: u64,
    maximum: usize,
    label: &str,
) -> Result<(u64, usize), CarrierError> {
    debug_assert_eq!(begin_off + 8, capacity_off);
    debug_assert_eq!(capacity_off + 4, count_off);
    let header = read_block::<VECTOR_HEADER_LEN, _>(memory, add(object, begin_off)?);
    header.vector_header_at(
        memory,
        0,
        capacity_off - begin_off,
        count_off - begin_off,
        maximum,
        label,
    )
}

fn require_vtable<M: ArmyUpgradeMemory>(
    memory: &M,
    object: u64,
    module_base: u64,
    expected_rva: u64,
) -> Result<(), CarrierError> {
    let expected = add(module_base, expected_rva)?;
    if read_u64(memory, object)? != expected {
        return invalid(format!("AUR1 vtable mismatch at 0x{object:x}"));
    }
    Ok(())
}

fn read_persistent_id<M: ArmyUpgradeMemory>(
    memory: &M,
    address: u64,
) -> Result<PersistentId, CarrierError> {
    Ok(PersistentId {
        kind: read_u32(memory, address)?,
        id: read_u32(memory, add(address, 4)?)?,
    })
}

/// The two row counts the collector publishes beside a D93 payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedCounts {
    pub countries: u32,
    pub entries: u32,
}

/// The counting the collector used to do over a freshly DECODED payload,
/// done over the snapshot the payload was encoded FROM. Identical by the
/// encoder/decoder round trip (`exact_raw_request_delivery_roundtrip`,
/// `observed_counts_match_the_counts_taken_from_the_decoded_payload`), and it
/// saves re-parsing a ~175 KB payload into a fresh object graph once per
/// frame for two integers.
pub fn observed_counts(snapshot: &ArmyUpgradeSnapshot) -> ObservedCounts {
    ObservedCounts {
        countries: snapshot.countries.len().min(u32::MAX as usize) as u32,
        entries: snapshot
            .countries
            .iter()
            .flat_map(|country| &country.groups)
            .map(|group| {
                1u64.saturating_add(group.requests.len() as u64)
                    .saturating_add(group.deliveries.len() as u64)
                    .saturating_add(
                        group
                            .deliveries
                            .iter()
                            .map(|delivery| delivery.produced.len() as u64)
                            .sum::<u64>(),
                    )
            })
            .sum::<u64>()
            .min(u64::from(u32::MAX)) as u32,
    }
}

fn collect_snapshot<S: ExactArmyUpgradeSource>(
    source: &S,
) -> Result<ArmyUpgradeSnapshot, CarrierError> {
    if !source.is_exact_a729_country_army_upgrade_source() {
        return Err(CarrierError::Source(
            "source did not authenticate exact a729 CCountry+0xF80 army upgrades".into(),
        ));
    }
    source.read_country_army_upgrades()
}

/// `collect_and_encode` plus the row counts its caller needs, taken from the
/// snapshot instead of from a decode of the bytes just produced.
pub fn collect_and_encode_with_counts<S: ExactArmyUpgradeSource>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<(Vec<u8>, ObservedCounts), CarrierError> {
    let snapshot = collect_snapshot(source)?;
    let counts = observed_counts(&snapshot);
    Ok((
        encode_receipt(snapshot, frame_hour, elapsed_wall_ms)?,
        counts,
    ))
}

fn encode_receipt(
    snapshot: ArmyUpgradeSnapshot,
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, CarrierError> {
    encode_payload(&ArmyUpgradeFrame {
        elapsed_wall_ms,
        frame_hour,
        status_flags: 0,
        authority: AuthorityState::ExactA729Receipt,
        snapshot: Some(snapshot),
    })
}

pub fn collect_and_encode<S: ExactArmyUpgradeSource>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, CarrierError> {
    encode_receipt(collect_snapshot(source)?, frame_hour, elapsed_wall_ms)
}

pub fn encode_source_unavailable(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, CarrierError> {
    encode_payload(&ArmyUpgradeFrame {
        elapsed_wall_ms,
        frame_hour,
        status_flags,
        authority: AuthorityState::SourceUnavailable,
        snapshot: None,
    })
}

pub fn validate_snapshot(snapshot: &ArmyUpgradeSnapshot) -> Result<(), CarrierError> {
    if snapshot.source_sequence == 0 || snapshot.countries.len() > MAX_COUNTRIES {
        return invalid("AUR1 snapshot header is invalid");
    }
    // No duplicate SETS here (lane SF). The strict-ascending test below is
    // the same rejection predicate on this walk: when entry `i` is reached
    // every earlier entry has been proven strictly ascending, so a value
    // equal to ANY earlier entry is <= the immediate predecessor and the
    // ordering arm rejects it -- with the identical message, since both arms
    // of the same `if` produce one refusal. Two `BTreeSet` families
    // (439 country tags plus one set per country of its army ids) were
    // therefore being built three times per frame -- once per coherent-read
    // pass and once more inside `encode_payload` -> `validate_frame` -- to
    // decide something the neighbouring comparison already decides.
    // `duplicate_identities_are_still_refused_without_the_sets` pins it.
    let mut leaves = 0usize;
    let mut prior_country = None;
    for country in &snapshot.countries {
        if country.tag_db_id == 0
            || prior_country.is_some_and(|prior| prior >= country.tag_db_id)
            || country.groups.len() > MAX_GROUPS_PER_COUNTRY
        {
            return invalid("AUR1 country order/identity/count is invalid");
        }
        prior_country = Some(country.tag_db_id);
        let mut prior_army = None;
        for group in &country.groups {
            if !is_army_identity_type(group.army.kind)
                || group.army.id == 0
                || prior_army.is_some_and(|prior| prior >= group.army)
                || group.requests.len() > MAX_REQUEST_ROWS_PER_GROUP
                || group.deliveries.len() > MAX_DELIVERIES_PER_GROUP
            {
                return invalid("AUR1 army group order/identity/count is invalid");
            }
            prior_army = Some(group.army);
            for row in &group.requests {
                validate_equipment_id(row.equipment_variant)?;
            }
            for delivery in &group.deliveries {
                if delivery.produced.len() > MAX_PRODUCED_ROWS_PER_DELIVERY {
                    return invalid("AUR1 produced row count exceeds bound");
                }
                if (delivery.status == 1) != delivery.progress_raw.is_some() {
                    return invalid("AUR1 delivery progress presence disagrees with status");
                }
                for row in &delivery.produced {
                    validate_equipment_id(row.equipment_variant)?;
                }
            }
            leaves = leaves
                .checked_add(group.requests.len())
                .and_then(|value| value.checked_add(group.deliveries.len()))
                .and_then(|value| {
                    group
                        .deliveries
                        .iter()
                        .try_fold(value, |sum, row| sum.checked_add(row.produced.len()))
                })
                .ok_or_else(|| CarrierError::Invalid("AUR1 leaf count overflow".into()))?;
        }
    }
    if leaves > MAX_TOTAL_LEAF_ROWS {
        return invalid("AUR1 total leaf count exceeds bound");
    }
    Ok(())
}

fn validate_equipment_id(identity: PersistentId) -> Result<(), CarrierError> {
    if identity.kind != EQUIPMENT_VARIANT_ID_TYPE || identity.id == 0 {
        return invalid("AUR1 equipment_variant_index identity is invalid");
    }
    Ok(())
}

pub fn validate_frame(frame: &ArmyUpgradeFrame) -> Result<(), CarrierError> {
    match frame.authority {
        AuthorityState::SourceUnavailable => {
            if frame.status_flags == 0 || frame.snapshot.is_some() {
                return invalid("unavailable AUR1 frame must be nonclean and rowless");
            }
        }
        AuthorityState::ExactA729Receipt => {
            if frame.status_flags != 0 || frame.snapshot.is_none() {
                return invalid("receipted AUR1 frame must be clean and measured");
            }
            validate_snapshot(frame.snapshot.as_ref().unwrap())?;
        }
    }
    Ok(())
}

/// Byte length of the fixed header every AUR1 payload carries, and the whole
/// length of a rowless one.
pub const ROWLESS_PAYLOAD_LEN: usize = 260;

/// The exact encoded length, so the ~175 KB production payload is built in
/// one allocation instead of growing 8 -> 16 -> ... -> 262,144 bytes: 15
/// reallocations presenting ~262 KB of existing contents to be moved (some
/// of which `realloc` may extend in place rather than copy). Saturating
/// throughout: this is a capacity hint, and a wrong hint can only cost a
/// reallocation, never a byte. `encoded_length_is_exact` and the
/// `debug_assert` in `encode_payload` pin it to the encoder.
fn encoded_length(frame: &ArmyUpgradeFrame) -> usize {
    let Some(snapshot) = &frame.snapshot else {
        return ROWLESS_PAYLOAD_LEN;
    };
    let mut len = ROWLESS_PAYLOAD_LEN.saturating_add(12);
    for country in &snapshot.countries {
        len = len
            .saturating_add(8)
            .saturating_add(country.groups.len().saturating_mul(16));
        for group in &country.groups {
            len = len.saturating_add(group.requests.len().saturating_mul(24));
            for delivery in &group.deliveries {
                len = len
                    .saturating_add(32)
                    .saturating_add(delivery.produced.len().saturating_mul(16));
            }
        }
    }
    len
}

pub fn encode_payload(frame: &ArmyUpgradeFrame) -> Result<Vec<u8>, CarrierError> {
    validate_frame(frame)?;
    let mut out = Vec::with_capacity(encoded_length(frame));
    u32v(&mut out, MAGIC);
    u32v(&mut out, SCHEMA_VERSION);
    u32v(&mut out, frame.authority.raw());
    u32v(&mut out, frame.status_flags);
    u64v(&mut out, frame.elapsed_wall_ms);
    u64v(&mut out, frame.frame_hour);
    out.extend_from_slice(&EXECUTABLE_SHA256);
    out.extend_from_slice(&STATUS_REQUEST_PERSISTENCE_MANIFEST_SHA256);
    out.extend_from_slice(&DELIVERY_PERSISTENCE_MANIFEST_SHA256);
    out.extend_from_slice(&PRODUCED_POOL_PERSISTENCE_MANIFEST_SHA256);
    out.extend_from_slice(&CARMY_IDENTITY_MANIFEST_SHA256);
    out.extend_from_slice(&LIVE_AST_ROWS_SHA256);
    out.extend_from_slice(&LIVE_AST_ARMY_IDS_SHA256);
    out.push(u8::from(frame.snapshot.is_some()));
    out.extend_from_slice(&[0; 3]);
    let Some(snapshot) = &frame.snapshot else {
        debug_assert_eq!(out.len(), encoded_length(frame));
        return Ok(out);
    };
    u64v(&mut out, snapshot.source_sequence);
    u32v(&mut out, snapshot.countries.len() as u32);
    for country in &snapshot.countries {
        u32v(&mut out, country.tag_db_id);
        u32v(&mut out, country.groups.len() as u32);
        for group in &country.groups {
            idv(&mut out, group.army);
            u32v(&mut out, group.requests.len() as u32);
            u32v(&mut out, group.deliveries.len() as u32);
            for row in &group.requests {
                idv(&mut out, row.equipment_variant);
                i64v(&mut out, row.request_raw);
                i64v(&mut out, row.total_raw);
            }
            for delivery in &group.deliveries {
                u32v(&mut out, delivery.status);
                u32v(&mut out, u32::from(delivery.progress_raw.is_some()));
                i64v(&mut out, delivery.progress_raw.unwrap_or(0));
                i64v(&mut out, delivery.total_progress_raw);
                out.push(u8::from(delivery.allow_zero_entries));
                out.extend_from_slice(&[0; 3]);
                u32v(&mut out, delivery.produced.len() as u32);
                for row in &delivery.produced {
                    idv(&mut out, row.equipment_variant);
                    i64v(&mut out, row.amount_raw);
                }
            }
        }
    }
    debug_assert_eq!(out.len(), encoded_length(frame));
    Ok(out)
}

pub fn decode_payload(payload: &[u8]) -> Result<ArmyUpgradeFrame, CarrierError> {
    let mut reader = Reader { payload, at: 0 };
    if reader.u32()? != MAGIC || reader.u32()? != SCHEMA_VERSION {
        return invalid("AUR1 magic/schema mismatch");
    }
    let authority = AuthorityState::from_raw(reader.u32()?)?;
    let status_flags = reader.u32()?;
    let elapsed_wall_ms = reader.u64()?;
    let frame_hour = reader.u64()?;
    for (expected, label) in [
        (EXECUTABLE_SHA256, "executable"),
        (
            STATUS_REQUEST_PERSISTENCE_MANIFEST_SHA256,
            "request persistence",
        ),
        (DELIVERY_PERSISTENCE_MANIFEST_SHA256, "delivery persistence"),
        (
            PRODUCED_POOL_PERSISTENCE_MANIFEST_SHA256,
            "produced pool persistence",
        ),
        (CARMY_IDENTITY_MANIFEST_SHA256, "CArmy identity"),
        (LIVE_AST_ROWS_SHA256, "live AST request rows"),
        (LIVE_AST_ARMY_IDS_SHA256, "live AST CArmy identities"),
    ] {
        if reader.bytes(32)? != expected {
            return invalid(format!("AUR1 {label} authority mismatch"));
        }
    }
    let has_snapshot = reader.bool()?;
    if reader.bytes(3)? != [0, 0, 0] {
        return invalid("AUR1 availability reserved bytes are nonzero");
    }
    let snapshot = if has_snapshot {
        let source_sequence = reader.u64()?;
        let country_count = reader.bounded_count(MAX_COUNTRIES, "country")?;
        if country_count > reader.remaining() / MIN_COUNTRY_ENTRY_LEN {
            return invalid("AUR1 country count exceeds the payload's capacity");
        }
        let mut countries = Vec::with_capacity(country_count);
        let mut leaves = 0usize;
        for _ in 0..country_count {
            let tag_db_id = reader.u32()?;
            let group_count = reader.bounded_count(MAX_GROUPS_PER_COUNTRY, "group")?;
            let mut groups = Vec::with_capacity(group_count);
            for _ in 0..group_count {
                let army = reader.id()?;
                let request_count = reader.bounded_count(MAX_REQUEST_ROWS_PER_GROUP, "request")?;
                let delivery_count = reader.bounded_count(MAX_DELIVERIES_PER_GROUP, "delivery")?;
                leaves = leaves
                    .checked_add(request_count)
                    .and_then(|value| value.checked_add(delivery_count))
                    .ok_or_else(|| CarrierError::Invalid("AUR1 leaf count overflow".into()))?;
                if leaves > MAX_TOTAL_LEAF_ROWS {
                    return invalid("AUR1 total leaf count exceeds bound");
                }
                let mut requests = Vec::with_capacity(request_count);
                for _ in 0..request_count {
                    requests.push(ArmyUpgradeRequestRow {
                        equipment_variant: reader.id()?,
                        request_raw: reader.i64()?,
                        total_raw: reader.i64()?,
                    });
                }
                let mut deliveries = Vec::with_capacity(delivery_count);
                for _ in 0..delivery_count {
                    let status = reader.u32()?;
                    let has_progress = match reader.u32()? {
                        0 => false,
                        1 => true,
                        _ => return invalid("AUR1 delivery progress marker is invalid"),
                    };
                    let progress_value = reader.i64()?;
                    if !has_progress && progress_value != 0 {
                        return invalid("AUR1 absent delivery progress has nonzero payload");
                    }
                    let progress_raw = has_progress.then_some(progress_value);
                    let total_progress_raw = reader.i64()?;
                    let allow_zero_entries = reader.bool()?;
                    if reader.bytes(3)? != [0, 0, 0] {
                        return invalid("AUR1 delivery reserved bytes are nonzero");
                    }
                    let produced_count =
                        reader.bounded_count(MAX_PRODUCED_ROWS_PER_DELIVERY, "produced")?;
                    leaves = leaves
                        .checked_add(produced_count)
                        .ok_or_else(|| CarrierError::Invalid("AUR1 leaf count overflow".into()))?;
                    if leaves > MAX_TOTAL_LEAF_ROWS {
                        return invalid("AUR1 total leaf count exceeds bound");
                    }
                    let mut produced = Vec::with_capacity(produced_count);
                    for _ in 0..produced_count {
                        produced.push(ProducedEquipmentRow {
                            equipment_variant: reader.id()?,
                            amount_raw: reader.i64()?,
                        });
                    }
                    deliveries.push(UpgradeDeliveryRow {
                        status,
                        progress_raw,
                        total_progress_raw,
                        allow_zero_entries,
                        produced,
                    });
                }
                groups.push(ArmyUpgradeGroup {
                    army,
                    requests,
                    deliveries,
                });
            }
            countries.push(CountryArmyUpgradeRow { tag_db_id, groups });
        }
        Some(ArmyUpgradeSnapshot {
            source_sequence,
            countries,
        })
    } else {
        None
    };
    if reader.at != payload.len() {
        return Err(CarrierError::TrailingBytes);
    }
    let frame = ArmyUpgradeFrame {
        elapsed_wall_ms,
        frame_hour,
        status_flags,
        authority,
        snapshot,
    };
    validate_frame(&frame)?;
    Ok(frame)
}

fn add(left: u64, right: u64) -> Result<u64, CarrierError> {
    left.checked_add(right)
        .ok_or_else(|| CarrierError::Invalid("AUR1 address overflow".into()))
}

fn read_bytes<const N: usize, M: ArmyUpgradeMemory>(
    memory: &M,
    address: u64,
) -> Result<[u8; N], CarrierError> {
    let mut out = [0; N];
    memory
        .read_exact(address, &mut out)
        .map_err(|_| CarrierError::Memory { address, width: N })?;
    Ok(out)
}

fn read_u8<M: ArmyUpgradeMemory>(memory: &M, address: u64) -> Result<u8, CarrierError> {
    Ok(read_bytes::<1, _>(memory, address)?[0])
}
fn read_u32<M: ArmyUpgradeMemory>(memory: &M, address: u64) -> Result<u32, CarrierError> {
    Ok(u32::from_le_bytes(read_bytes(memory, address)?))
}
fn read_i32<M: ArmyUpgradeMemory>(memory: &M, address: u64) -> Result<i32, CarrierError> {
    Ok(i32::from_le_bytes(read_bytes(memory, address)?))
}
fn read_u64<M: ArmyUpgradeMemory>(memory: &M, address: u64) -> Result<u64, CarrierError> {
    Ok(u64::from_le_bytes(read_bytes(memory, address)?))
}
fn read_i64<M: ArmyUpgradeMemory>(memory: &M, address: u64) -> Result<i64, CarrierError> {
    Ok(i64::from_le_bytes(read_bytes(memory, address)?))
}

fn invalid<T>(message: impl Into<String>) -> Result<T, CarrierError> {
    Err(CarrierError::Invalid(message.into()))
}
fn u32v(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn u64v(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn i64v(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn idv(out: &mut Vec<u8>, value: PersistentId) {
    u32v(out, value.kind);
    u32v(out, value.id);
}

struct Reader<'a> {
    payload: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, len: usize) -> Result<&'a [u8], CarrierError> {
        let end = self.at.checked_add(len).ok_or(CarrierError::Truncated)?;
        let value = self
            .payload
            .get(self.at..end)
            .ok_or(CarrierError::Truncated)?;
        self.at = end;
        Ok(value)
    }
    fn bool(&mut self) -> Result<bool, CarrierError> {
        match self.bytes(1)?[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => invalid("AUR1 bool byte is invalid"),
        }
    }
    fn u32(&mut self) -> Result<u32, CarrierError> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, CarrierError> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, CarrierError> {
        Ok(i64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    fn id(&mut self) -> Result<PersistentId, CarrierError> {
        Ok(PersistentId {
            kind: self.u32()?,
            id: self.u32()?,
        })
    }
    fn bounded_count(&mut self, maximum: usize, label: &str) -> Result<usize, CarrierError> {
        let value = self.u32()? as usize;
        if value > maximum {
            return invalid(format!("AUR1 {label} count exceeds bound"));
        }
        Ok(value)
    }

    fn remaining(&self) -> usize {
        self.payload.len().saturating_sub(self.at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MissingMemory;

    impl ArmyUpgradeMemory for MissingMemory {
        fn read_exact(&self, _address: u64, _out: &mut [u8]) -> Result<(), ()> {
            Err(())
        }
    }

    #[test]
    fn country_walk_failure_retains_country_address_and_width() {
        let error = read_snapshot_from_country_roots(
            &MissingMemory,
            0x1400_0000_0,
            1,
            &[CountryRoot {
                tag_db_id: 41,
                object_ptr: 0x2000_0000,
            }],
        )
        .unwrap_err();
        assert!(matches!(
            error,
            CarrierError::CountryRead {
                tag_db_id: 41,
                object_ptr: 0x2000_0000,
                source,
            } if matches!(
                *source,
                CarrierError::Memory {
                    address: 0x2000_0f80,
                    width: 8,
                }
            )
        ));
    }

    fn frame() -> ArmyUpgradeFrame {
        ArmyUpgradeFrame {
            elapsed_wall_ms: 7,
            frame_hour: 99,
            status_flags: 0,
            authority: AuthorityState::ExactA729Receipt,
            snapshot: Some(ArmyUpgradeSnapshot {
                source_sequence: 3,
                countries: vec![CountryArmyUpgradeRow {
                    tag_db_id: 67,
                    groups: vec![ArmyUpgradeGroup {
                        army: PersistentId {
                            kind: 4713,
                            id: 14646,
                        },
                        requests: vec![ArmyUpgradeRequestRow {
                            equipment_variant: PersistentId { kind: 70, id: 5149 },
                            request_raw: 700_000,
                            total_raw: 800_000,
                        }],
                        deliveries: vec![UpgradeDeliveryRow {
                            status: 1,
                            progress_raw: Some(25_000),
                            total_progress_raw: 100_000,
                            allow_zero_entries: false,
                            produced: vec![ProducedEquipmentRow {
                                equipment_variant: PersistentId { kind: 70, id: 5149 },
                                amount_raw: 3,
                            }],
                        }],
                    }],
                }],
            }),
        }
    }

    #[test]
    fn exact_raw_request_delivery_roundtrip() {
        let expected = frame();
        let encoded = encode_payload(&expected).unwrap();
        assert_eq!(decode_payload(&encoded).unwrap(), expected);
    }

    #[test]
    fn unavailable_cannot_masquerade_as_measured_empty() {
        let mut invalid_frame = frame();
        invalid_frame.authority = AuthorityState::SourceUnavailable;
        invalid_frame.status_flags = 1;
        assert!(encode_payload(&invalid_frame).is_err());
    }

    const FIXTURE_BASE: u64 = 0x1_4000_0000;
    const FIXTURE_COUNTRY: u64 = 0x2000_0000;
    const FIXTURE_STATUS: u64 = 0x2001_0000;
    const FIXTURE_GROUP_SLOTS: u64 = 0x2002_0000;
    const FIXTURE_GROUP: u64 = 0x2003_0000;
    const FIXTURE_ARMY: u64 = 0x2004_0000;

    #[derive(Default)]
    struct SparseMemory {
        bytes: std::collections::BTreeMap<u64, u8>,
    }

    impl SparseMemory {
        fn put(&mut self, address: u64, value: &[u8]) {
            for (index, byte) in value.iter().enumerate() {
                self.bytes.insert(address + index as u64, *byte);
            }
        }

        /// Present an object's whole field extent, so a widened copy of it is
        /// served rather than refused. Without this the fixture would only
        /// ever exercise the scalar fallback and the path production runs
        /// would go untested.
        fn present(&mut self, address: u64, len: usize) {
            for offset in 0..len as u64 {
                self.bytes.entry(address + offset).or_insert(0);
            }
        }

        fn u32(&mut self, address: u64, value: u32) {
            self.put(address, &value.to_le_bytes());
        }

        fn u64(&mut self, address: u64, value: u64) {
            self.put(address, &value.to_le_bytes());
        }
    }

    impl ArmyUpgradeMemory for SparseMemory {
        fn read_exact(&self, address: u64, out: &mut [u8]) -> Result<(), ()> {
            for (index, slot) in out.iter_mut().enumerate() {
                *slot = *self.bytes.get(&(address + index as u64)).ok_or(())?;
            }
            Ok(())
        }
    }

    fn single_group_memory(army_kind: u32, army_id: u32) -> SparseMemory {
        let mut memory = SparseMemory::default();
        memory.present(FIXTURE_STATUS, STATUS_BLOCK_LEN);
        memory.present(FIXTURE_GROUP, GROUP_BLOCK_LEN);
        memory.present(FIXTURE_ARMY, ARMY_BLOCK_LEN);
        memory.u64(
            FIXTURE_COUNTRY + COUNTRY_UPGRADE_STATUS_PTR_OFF,
            FIXTURE_STATUS,
        );

        memory.u64(FIXTURE_STATUS, FIXTURE_BASE + STATUS_VTABLE_RVA);
        memory.u64(
            FIXTURE_STATUS + STATUS_PERSISTENT_VTABLE_OFF,
            FIXTURE_BASE + STATUS_PERSISTENT_VTABLE_RVA,
        );
        memory.u64(FIXTURE_STATUS + STATUS_GROUP_BEGIN_OFF, FIXTURE_GROUP_SLOTS);
        memory.u32(FIXTURE_STATUS + STATUS_GROUP_CAPACITY_OFF, 1);
        memory.u32(FIXTURE_STATUS + STATUS_GROUP_COUNT_OFF, 1);
        memory.u64(FIXTURE_GROUP_SLOTS, FIXTURE_GROUP);

        memory.u64(FIXTURE_GROUP, FIXTURE_BASE + REQUESTS_VTABLE_RVA);
        memory.u64(
            FIXTURE_GROUP + REQUESTS_PERSISTENT_VTABLE_OFF,
            FIXTURE_BASE + REQUESTS_PERSISTENT_VTABLE_RVA,
        );
        memory.u32(FIXTURE_GROUP + REQUESTS_VERSION_OFF, REQUESTS_VERSION);
        memory.u64(FIXTURE_GROUP + REQUESTS_ARMY_PTR_OFF, FIXTURE_ARMY);
        memory.u64(FIXTURE_GROUP + REQUESTS_STATUS_PTR_OFF, FIXTURE_STATUS);
        memory.u64(FIXTURE_GROUP + REQUEST_ROW_BEGIN_OFF, 0);
        memory.u32(FIXTURE_GROUP + REQUEST_ROW_CAPACITY_OFF, 0);
        memory.u32(FIXTURE_GROUP + REQUEST_ROW_COUNT_OFF, 0);
        memory.u64(FIXTURE_GROUP + DELIVERY_BEGIN_OFF, 0);
        memory.u32(FIXTURE_GROUP + DELIVERY_CAPACITY_OFF, 0);
        memory.u32(FIXTURE_GROUP + DELIVERY_COUNT_OFF, 0);

        memory.u64(FIXTURE_ARMY, FIXTURE_BASE + ARMY_VTABLE_RVA);
        memory.u64(
            FIXTURE_ARMY + ARMY_PERSISTENT_VTABLE_OFF,
            FIXTURE_BASE + ARMY_PERSISTENT_VTABLE_RVA,
        );
        memory.u32(FIXTURE_ARMY + ARMY_ID_OFF, army_kind);
        memory.u32(FIXTURE_ARMY + ARMY_ID_OFF + 4, army_id);
        memory
    }

    fn read_single_group(army_kind: u32) -> Result<ArmyUpgradeSnapshot, CarrierError> {
        read_snapshot_from_country_roots(
            &single_group_memory(army_kind, 14_646),
            FIXTURE_BASE,
            1,
            &[CountryRoot {
                tag_db_id: 1,
                object_ptr: FIXTURE_COUNTRY,
            }],
        )
    }

    #[test]
    fn native_read_admits_the_wrapper_observed_alt_unit_handle_type() {
        for kind in [ARMY_ID_TYPE, ARMY_ID_ALT_TYPE] {
            let snapshot = read_single_group(kind)
                .unwrap_or_else(|error| panic!("handle type {kind} refused: {error:?}"));
            assert_eq!(snapshot.countries.len(), 1);
            assert_eq!(snapshot.countries[0].groups.len(), 1);
            assert_eq!(snapshot.countries[0].groups[0].army.kind, kind);
        }
    }

    #[test]
    fn native_read_still_refuses_a_handle_type_outside_the_unit_body_set() {
        for kind in [0, 1, EQUIPMENT_VARIANT_ID_TYPE, 52, 4_712] {
            let error =
                read_single_group(kind).expect_err("a non-unit-body handle type must stay refused");
            assert!(
                matches!(&error, CarrierError::CountryRead { source, .. }
                    if matches!(&**source, CarrierError::Invalid(reason)
                        if reason == "AUR1 CArmy persistent identity is invalid or duplicated")),
                "handle type {kind} produced {error:?}"
            );
        }
    }

    #[test]
    fn native_read_still_refuses_a_zero_army_id() {
        let error = read_snapshot_from_country_roots(
            &single_group_memory(ARMY_ID_ALT_TYPE, 0),
            FIXTURE_BASE,
            1,
            &[CountryRoot {
                tag_db_id: 1,
                object_ptr: FIXTURE_COUNTRY,
            }],
        )
        .expect_err("a zero CArmy id must stay refused");
        assert!(matches!(&error, CarrierError::CountryRead { source, .. }
            if matches!(&**source, CarrierError::Invalid(reason)
                if reason == "AUR1 CArmy persistent identity is invalid or duplicated")));
    }

    #[test]
    fn snapshot_validation_matches_the_native_handle_type_set() {
        let mut snapshot = frame().snapshot.unwrap();
        snapshot.countries[0].groups[0].army.kind = ARMY_ID_ALT_TYPE;
        validate_snapshot(&snapshot).expect("alt unit handle type must validate");
        snapshot.countries[0].groups[0].army.kind = EQUIPMENT_VARIANT_ID_TYPE;
        assert!(validate_snapshot(&snapshot).is_err());
    }

    /// Serves `inner` verbatim except that any read COVERING
    /// `watch_address + 4` (the CArmy id u32) has values from `watch_ids`
    /// spliced in: successively while any remain (clamped to the final
    /// value), or cycling forever when `cycle` is set. Each individual pass
    /// therefore sees a structurally valid graph, and the schedule decides
    /// whether the graph ever stops changing between passes.
    ///
    /// Coverage, not an exact `(address, width)` match: the walk takes the
    /// CArmy identity out of one widened copy of the object rather than as
    /// its own 4-byte read, and a fixture that only recognised the narrow
    /// read would silently stop mutating anything. One covering read per pass
    /// either way, so the read counts these tests assert still count passes.
    struct MutatingMemory {
        inner: SparseMemory,
        watch_address: u64,
        watch_ids: Vec<u32>,
        cycle: bool,
        watch_reads: std::cell::Cell<usize>,
    }

    impl ArmyUpgradeMemory for MutatingMemory {
        fn read_exact(&self, address: u64, out: &mut [u8]) -> Result<(), ()> {
            self.inner.read_exact(address, out)?;
            let watched = self.watch_address + 4;
            let covers = address <= watched
                && watched
                    .checked_add(4)
                    .is_some_and(|end| end <= address + out.len() as u64);
            if covers {
                let index = self.watch_reads.get();
                let id = if self.cycle {
                    self.watch_ids[index % self.watch_ids.len()]
                } else {
                    self.watch_ids[index.min(self.watch_ids.len() - 1)]
                };
                self.watch_reads.set(index + 1);
                let at = (watched - address) as usize;
                out[at..at + 4].copy_from_slice(&id.to_le_bytes());
            }
            Ok(())
        }
    }

    fn mutating_memory(ids: Vec<u32>, cycle: bool) -> MutatingMemory {
        MutatingMemory {
            inner: single_group_memory(ARMY_ID_TYPE, ids[0]),
            watch_address: FIXTURE_ARMY + ARMY_ID_OFF,
            watch_ids: ids,
            cycle,
            watch_reads: std::cell::Cell::new(0),
        }
    }

    fn read_mutating(memory: &MutatingMemory) -> Result<ArmyUpgradeSnapshot, CarrierError> {
        read_snapshot_from_country_roots(
            memory,
            FIXTURE_BASE,
            1,
            &[CountryRoot {
                tag_db_id: 1,
                object_ptr: FIXTURE_COUNTRY,
            }],
        )
    }

    // kills: the retired fixed two-pass check ("AUR1 object graph changed
    // across the coherent double-read"), which refused a frame whenever one
    // pass pair straddled an engine mutation even though the same bracket's
    // later passes agree -- the d93 BLOCKED cause on pw_gate_final (18 frames,
    // all d93_fail_reason_double_read_changed). The restartable contract must
    // settle such a frame as measured data.
    #[test]
    fn coherent_read_restarts_until_the_graph_settles() {
        // Pass 1 sees id 14646; the mutation lands; every later pass sees 14647.
        let memory = mutating_memory(vec![14_646, 14_647], false);
        let snapshot = read_mutating(&memory)
            .unwrap_or_else(|error| panic!("a settling graph must still seal: {error:?}"));
        assert_eq!(snapshot.countries[0].groups[0].army.id, 14_647);
        assert_eq!(memory.watch_reads.get(), 3);
    }

    // kills: any unbounded retry, and any collapse of a persistently torn
    // graph into measured data: a graph that changes on every pass must
    // exhaust exactly COHERENT_READ_MAX_PASSES passes and refuse under its own
    // restart-exhausted name. The elapsed-time bound kills retries that WAIT:
    // the eight fixture passes are memory-only and complete in microseconds,
    // so any per-pass sleep/yield of meaningful size blows the ceiling. That
    // no production path sleeps is additionally established by inspection --
    // the loop body is only `read_snapshot_once` calls.
    #[test]
    fn coherent_read_refuses_by_name_when_the_graph_never_settles() {
        let memory = mutating_memory(vec![14_646, 14_647], true);
        let started = std::time::Instant::now();
        let error = read_mutating(&memory)
            .err()
            .expect("a graph that changes on every pass must stay refused");
        let elapsed = started.elapsed();
        assert!(
            matches!(&error, CarrierError::Invalid(reason)
                if reason == "AUR1 object graph kept changing across every coherent-read restart"),
            "never-settling graph produced {error:?}"
        );
        assert_eq!(memory.watch_reads.get(), COHERENT_READ_MAX_PASSES);
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "the bounded refusal must not wait (took {elapsed:?})"
        );
    }

    // --- lane SF: the widened guarded copies -----------------------------

    /// A dense image with a bump allocator: every object the walk reads is a
    /// contiguous run of real bytes, which is the production shape and the
    /// only shape in which a widened copy is SERVED rather than refused.
    struct DenseMemory {
        base: u64,
        bytes: Vec<u8>,
        /// `(address, width)` of every read this image SERVED, in order.
        /// The byte-identity test asserts on it, because two walks that agree
        /// prove nothing if the widened one silently fell back to scalar
        /// reads throughout. The ADDRESS is in the log because widths alone
        /// cannot tell two same-width paths apart: an equipment block, a
        /// vector-header re-read and a two-entry pointer window are all 16
        /// bytes wide.
        served: std::cell::RefCell<Vec<(u64, usize)>>,
    }

    impl DenseMemory {
        fn new(base: u64) -> Self {
            // Leading padding so no object can be allocated at `base` itself
            // and be mistaken for a null pointer.
            Self {
                base,
                bytes: vec![0; 16],
                served: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn alloc(&mut self, len: usize) -> u64 {
            let at = (self.bytes.len() + 15) & !15;
            self.bytes.resize(at + len.max(1), 0);
            self.base + at as u64
        }

        fn put(&mut self, address: u64, value: &[u8]) {
            let at = (address - self.base) as usize;
            self.bytes[at..at + value.len()].copy_from_slice(value);
        }

        fn u8(&mut self, address: u64, value: u8) {
            self.put(address, &[value]);
        }
        fn u32(&mut self, address: u64, value: u32) {
            self.put(address, &value.to_le_bytes());
        }
        fn i32(&mut self, address: u64, value: i32) {
            self.put(address, &value.to_le_bytes());
        }
        fn u64(&mut self, address: u64, value: u64) {
            self.put(address, &value.to_le_bytes());
        }
        fn i64(&mut self, address: u64, value: i64) {
            self.put(address, &value.to_le_bytes());
        }

        fn vector(&mut self, object: u64, begin_off: u64, begin: u64, count: usize) {
            self.u64(object + begin_off, begin);
            self.i32(object + begin_off + 8, count as i32);
            self.i32(object + begin_off + 12, count as i32);
        }
    }

    impl ArmyUpgradeMemory for DenseMemory {
        fn read_exact(&self, address: u64, out: &mut [u8]) -> Result<(), ()> {
            let at = usize::try_from(address.checked_sub(self.base).ok_or(())?).map_err(|_| ())?;
            let end = at.checked_add(out.len()).ok_or(())?;
            out.copy_from_slice(self.bytes.get(at..end).ok_or(())?);
            self.served.borrow_mut().push((address, out.len()));
            Ok(())
        }
    }

    /// The same image behind a one-scalar ceiling: every read wider than 8
    /// bytes is refused, which is what the collector's region gate does to a
    /// range it will not confirm whole. The walk therefore runs its scalar
    /// fallback for every object block, every vector-header re-read and every
    /// multi-entry array window. (A single-entry pointer window is exactly 8
    /// bytes at exactly the address the scalar read would use, so there is no
    /// distinction left to force there.)
    struct ScalarOnlyMemory<'image> {
        inner: &'image DenseMemory,
    }

    impl ArmyUpgradeMemory for ScalarOnlyMemory<'_> {
        fn read_exact(&self, address: u64, out: &mut [u8]) -> Result<(), ()> {
            if out.len() > 8 {
                return Err(());
            }
            self.inner.read_exact(address, out)
        }
    }

    /// Refuses every read covering one byte, wide or narrow.
    struct HoleMemory<'image> {
        inner: &'image DenseMemory,
        hole: u64,
    }

    impl ArmyUpgradeMemory for HoleMemory<'_> {
        fn read_exact(&self, address: u64, out: &mut [u8]) -> Result<(), ()> {
            if address <= self.hole && self.hole < address + out.len() as u64 {
                return Err(());
            }
            self.inner.read_exact(address, out)
        }
    }

    struct GraphBuilder {
        memory: DenseMemory,
        module_base: u64,
        /// The most recently built `CArmy` and equipment variant, so the test
        /// can pin a widened read to each of them BY ADDRESS.
        last_army: u64,
        last_equipment: u64,
    }

    impl GraphBuilder {
        fn equipment(&mut self, id: u32) -> u64 {
            let ptr = self.memory.alloc(EQUIPMENT_BLOCK_LEN);
            self.memory
                .u64(ptr, self.module_base + EQUIPMENT_VARIANT_VTABLE_RVA);
            self.memory
                .u32(ptr + EQUIPMENT_VARIANT_ID_OFF, EQUIPMENT_VARIANT_ID_TYPE);
            self.memory.u32(ptr + EQUIPMENT_VARIANT_ID_OFF + 4, id);
            self.last_equipment = ptr;
            ptr
        }

        fn delivery(
            &mut self,
            status: u32,
            progress: i64,
            total: i64,
            allow_zero: bool,
            produced: &[(u32, i64)],
        ) -> u64 {
            let ptr = self.memory.alloc(DELIVERY_BLOCK_LEN);
            self.memory.u64(ptr, self.module_base + DELIVERY_VTABLE_RVA);
            self.memory.u64(
                ptr + DELIVERY_PRODUCED_POOL_OFF,
                self.module_base + EQUIPMENT_VARIANT_POOL_VTABLE_RVA,
            );
            self.memory.u32(ptr + DELIVERY_STATUS_OFF, status);
            self.memory.i64(ptr + DELIVERY_PROGRESS_OFF, progress);
            self.memory.i64(ptr + DELIVERY_TOTAL_PROGRESS_OFF, total);
            self.memory
                .u8(ptr + PRODUCED_ALLOW_ZERO_OFF, u8::from(allow_zero));
            let mut begin = 0;
            if !produced.is_empty() {
                begin = self
                    .memory
                    .alloc(produced.len() * PRODUCED_ROW_STRIDE as usize);
                for (index, (variant, amount)) in produced.iter().enumerate() {
                    let variant_ptr = self.equipment(*variant);
                    let row = begin + index as u64 * PRODUCED_ROW_STRIDE;
                    self.memory.u64(row, variant_ptr);
                    self.memory.i64(row + 8, *amount);
                }
            }
            self.memory
                .vector(ptr, PRODUCED_ROW_BEGIN_OFF, begin, produced.len());
            ptr
        }

        fn group(
            &mut self,
            status_ptr: u64,
            army_kind: u32,
            army_id: u32,
            requests: &[(u32, i64, i64)],
            deliveries: &[u64],
        ) -> u64 {
            let ptr = self.memory.alloc(GROUP_BLOCK_LEN);
            let army = self.memory.alloc(ARMY_BLOCK_LEN);
            self.memory.u64(army, self.module_base + ARMY_VTABLE_RVA);
            self.memory.u64(
                army + ARMY_PERSISTENT_VTABLE_OFF,
                self.module_base + ARMY_PERSISTENT_VTABLE_RVA,
            );
            self.memory.u32(army + ARMY_ID_OFF, army_kind);
            self.memory.u32(army + ARMY_ID_OFF + 4, army_id);
            self.memory.u64(ptr, self.module_base + REQUESTS_VTABLE_RVA);
            self.memory.u64(
                ptr + REQUESTS_PERSISTENT_VTABLE_OFF,
                self.module_base + REQUESTS_PERSISTENT_VTABLE_RVA,
            );
            self.memory
                .u32(ptr + REQUESTS_VERSION_OFF, REQUESTS_VERSION);
            self.memory.u64(ptr + REQUESTS_ARMY_PTR_OFF, army);
            self.memory.u64(ptr + REQUESTS_STATUS_PTR_OFF, status_ptr);
            let mut rows = 0;
            if !requests.is_empty() {
                rows = self
                    .memory
                    .alloc(requests.len() * REQUEST_ROW_STRIDE as usize);
                for (index, (variant, request, total)) in requests.iter().enumerate() {
                    let variant_ptr = self.equipment(*variant);
                    let row = rows + index as u64 * REQUEST_ROW_STRIDE;
                    self.memory
                        .u64(row + REQUEST_ROW_EQUIPMENT_PTR_OFF, variant_ptr);
                    self.memory.i64(row + REQUEST_ROW_REQUEST_OFF, *request);
                    self.memory.i64(row + REQUEST_ROW_TOTAL_OFF, *total);
                }
            }
            self.memory
                .vector(ptr, REQUEST_ROW_BEGIN_OFF, rows, requests.len());
            let mut slots = 0;
            if !deliveries.is_empty() {
                slots = self.memory.alloc(deliveries.len() * 8);
                for (index, delivery) in deliveries.iter().enumerate() {
                    self.memory.u64(slots + index as u64 * 8, *delivery);
                }
            }
            self.memory
                .vector(ptr, DELIVERY_BEGIN_OFF, slots, deliveries.len());
            self.last_army = army;
            ptr
        }

        fn status(&mut self) -> u64 {
            let ptr = self.memory.alloc(STATUS_BLOCK_LEN);
            self.memory.u64(ptr, self.module_base + STATUS_VTABLE_RVA);
            self.memory.u64(
                ptr + STATUS_PERSISTENT_VTABLE_OFF,
                self.module_base + STATUS_PERSISTENT_VTABLE_RVA,
            );
            ptr
        }

        fn finish_status(&mut self, status_ptr: u64, groups: &[u64]) {
            let mut slots = 0;
            if !groups.is_empty() {
                slots = self.memory.alloc(groups.len() * 8);
                for (index, group) in groups.iter().enumerate() {
                    self.memory.u64(slots + index as u64 * 8, *group);
                }
            }
            self.memory
                .vector(status_ptr, STATUS_GROUP_BEGIN_OFF, slots, groups.len());
        }

        fn country(&mut self, status_ptr: u64) -> u64 {
            let ptr = self
                .memory
                .alloc(COUNTRY_UPGRADE_STATUS_PTR_OFF as usize + 8);
            self.memory
                .u64(ptr + COUNTRY_UPGRADE_STATUS_PTR_OFF, status_ptr);
            ptr
        }
    }

    struct Fixture {
        memory: DenseMemory,
        module_base: u64,
        roots: Vec<CountryRoot>,
        /// Country 22's status object, its first group, and the objects that
        /// group hangs off - so the byte-identity test can assert that a
        /// widened copy of each KIND of block was served, at the address it
        /// belongs to rather than merely at its width. `group_equipment` is
        /// the LAST equipment variant built for that group's request rows
        /// (its third), not the first one in the graph; any one of them
        /// pins the equipment-block path.
        status: u64,
        first_group: u64,
        first_army: u64,
        group_equipment: u64,
        first_delivery: u64,
    }

    /// Three countries covering every arm of the walk: a country with no
    /// upgrade status at all, a country with two groups (one carrying
    /// deliveries with and without progress and with and without produced
    /// rows, one carrying a request array LONGER than a single array window
    /// so the window must refill mid-array), and a country with a group that
    /// has no requests and one delivery with three produced rows.
    fn upgrade_graph() -> Fixture {
        let module_base = 0x1_4000_0000;
        let mut builder = GraphBuilder {
            memory: DenseMemory::new(0x3000_0000),
            module_base,
            last_army: 0,
            last_equipment: 0,
        };
        let mut roots = Vec::new();

        let bare = builder.country(0);
        roots.push(CountryRoot {
            tag_db_id: 11,
            object_ptr: bare,
        });

        let first_status = builder.status();
        let status = first_status;
        let with_progress = builder.delivery(1, 25_000, 100_000, false, &[(5001, 3)]);
        let without_progress = builder.delivery(2, 0, 7, true, &[]);
        let first_group = builder.group(
            status,
            ARMY_ID_TYPE,
            100,
            &[
                (5001, 1_000, 2_000),
                (5002, -5, 6),
                (5003, i64::MAX, i64::MIN),
            ],
            &[with_progress, without_progress],
        );
        let first_army = builder.last_army;
        let group_equipment = builder.last_equipment;
        let long: Vec<(u32, i64, i64)> = (0..200)
            .map(|index| (5100 + index as u32, index as i64, index as i64 * 3))
            .collect();
        let long_group = builder.group(status, ARMY_ID_ALT_TYPE, 99, &long, &[]);
        builder.finish_status(status, &[first_group, long_group]);
        let country = builder.country(status);
        roots.push(CountryRoot {
            tag_db_id: 22,
            object_ptr: country,
        });

        let status = builder.status();
        let three_produced = builder.delivery(0, 0, 0, false, &[(5004, 1), (5005, 2), (5006, 3)]);
        let sole_group = builder.group(status, ARMY_ID_TYPE, 7, &[], &[three_produced]);
        builder.finish_status(status, &[sole_group]);
        let country = builder.country(status);
        roots.push(CountryRoot {
            tag_db_id: 33,
            object_ptr: country,
        });

        Fixture {
            memory: builder.memory,
            module_base,
            roots,
            status: first_status,
            first_group,
            first_army,
            group_equipment,
            first_delivery: with_progress,
        }
    }

    fn receipt_frame(snapshot: ArmyUpgradeSnapshot) -> ArmyUpgradeFrame {
        ArmyUpgradeFrame {
            elapsed_wall_ms: 4,
            frame_hour: 77,
            status_flags: 0,
            authority: AuthorityState::ExactA729Receipt,
            snapshot: Some(snapshot),
        }
    }

    // kills: any widening of the guarded reads that does not reproduce the
    // scalar walk's bytes -- a mis-sized object block, a field decoded at the
    // wrong offset inside a copy, an array window that skips, repeats or
    // mis-addresses an entry across a refill, or a fallback arm that reads
    // different fields than the widened arm it stands in for. Same image,
    // one served wide and one forced through the scalar statements, so the
    // two payloads must be identical byte for byte.
    #[test]
    fn widened_and_scalar_paths_are_byte_identical() {
        let fixture = upgrade_graph();
        let widened = read_snapshot_from_country_roots(
            &fixture.memory,
            fixture.module_base,
            5,
            &fixture.roots,
        )
        .expect("the widened walk must read the fixture graph");
        let widened_reads = std::mem::take(&mut *fixture.memory.served.borrow_mut());
        let scalar = read_snapshot_from_country_roots(
            &ScalarOnlyMemory {
                inner: &fixture.memory,
            },
            fixture.module_base,
            5,
            &fixture.roots,
        )
        .expect("the scalar walk must read the same fixture graph");
        let scalar_reads = std::mem::take(&mut *fixture.memory.served.borrow_mut());
        assert_eq!(widened, scalar);

        // The widened walk must actually have BEEN widened: two agreeing
        // walks prove nothing if the first one silently fell back to scalar
        // reads everywhere. Each block kind is pinned at the address it
        // belongs to, because three of the widths collide (an equipment
        // block, a vector-header re-read and a two-entry pointer window are
        // all 16 bytes), and a same-width regression would hide behind a
        // width-only assertion.
        assert!(
            scalar_reads.iter().all(|(_, width)| *width <= 8),
            "the scalar-only memory must serve nothing wider than one scalar"
        );
        for (address, width, what) in [
            (fixture.status, STATUS_BLOCK_LEN, "the status object block"),
            (
                fixture.first_group,
                GROUP_BLOCK_LEN,
                "the group object block",
            ),
            (fixture.first_army, ARMY_BLOCK_LEN, "the CArmy block"),
            (
                fixture.group_equipment,
                EQUIPMENT_BLOCK_LEN,
                "the equipment variant block",
            ),
            (
                fixture.first_delivery,
                DELIVERY_BLOCK_LEN,
                "the delivery block",
            ),
            (
                fixture.first_group + REQUEST_ROW_BEGIN_OFF,
                VECTOR_HEADER_LEN,
                "the request vector-header re-read",
            ),
            (
                fixture.first_group + DELIVERY_BEGIN_OFF,
                VECTOR_HEADER_LEN,
                "the delivery vector-header re-read",
            ),
            (
                fixture.first_delivery + PRODUCED_ROW_BEGIN_OFF,
                VECTOR_HEADER_LEN,
                "the produced vector-header re-read",
            ),
            (
                fixture.status + STATUS_GROUP_BEGIN_OFF,
                VECTOR_HEADER_LEN,
                "the group vector-header re-read",
            ),
        ] {
            assert_eq!(
                widened_reads
                    .iter()
                    .filter(|(at, served)| *at == address && *served == width)
                    .count(),
                2,
                "{what} must be served as one widened copy per coherent-read pass"
            );
        }

        // The 200-row request array is longer than one window, so a working
        // window takes it as a full window plus a short tail at the address
        // the tail starts at. The tail read is what proves the refill ran,
        // rather than the array falling back after its first window.
        let full_window = ARRAY_WINDOW_BYTES / REQUEST_ROW_STRIDE as usize;
        let tail_rows = 200 - full_window;
        let widest = widened_reads.iter().map(|(_, width)| *width).max();
        assert_eq!(
            widest,
            Some(full_window * REQUEST_ROW_STRIDE as usize),
            "the widest widened read must be one full request-row window"
        );
        let long_array = widened_reads
            .iter()
            .find(|(_, width)| Some(*width) == widest)
            .expect("the full window read was just asserted to exist")
            .0;
        assert_eq!(
            widened_reads
                .iter()
                .filter(
                    |(at, width)| *at == long_array + full_window as u64 * REQUEST_ROW_STRIDE
                        && *width == tail_rows * REQUEST_ROW_STRIDE as usize
                )
                .count(),
            2,
            "each pass must refill the window at the array's tail address"
        );
        assert!(
            widened_reads.len() * 3 < scalar_reads.len(),
            "widening must cut the read count several-fold: {} vs {}",
            widened_reads.len(),
            scalar_reads.len()
        );
        assert_eq!(
            encode_payload(&receipt_frame(widened.clone())).unwrap(),
            encode_payload(&receipt_frame(scalar)).unwrap()
        );

        // The graph really is populated -- an empty walk would satisfy the
        // comparison above without testing anything -- and its longest array
        // really does outrun one window.
        let groups: usize = widened.countries.iter().map(|c| c.groups.len()).sum();
        let requests: usize = widened
            .countries
            .iter()
            .flat_map(|c| &c.groups)
            .map(|g| g.requests.len())
            .sum();
        let deliveries: usize = widened
            .countries
            .iter()
            .flat_map(|c| &c.groups)
            .map(|g| g.deliveries.len())
            .sum();
        let produced: usize = widened
            .countries
            .iter()
            .flat_map(|c| &c.groups)
            .flat_map(|g| &g.deliveries)
            .map(|d| d.produced.len())
            .sum();
        assert_eq!(
            (
                widened.countries.len(),
                groups,
                requests,
                deliveries,
                produced
            ),
            (3, 3, 203, 3, 4)
        );
        assert!(200 * REQUEST_ROW_STRIDE as usize > ARRAY_WINDOW_BYTES);
    }

    // kills: a widened read whose refusal is reported at the WIDE range
    // instead of at the field the scalar walk would have failed on. A hole in
    // the CArmy pointer refuses the group's whole widened copy, so the
    // fallback walks the group's fields in the original order and must
    // surface the original `Memory { address, width }` for that one field --
    // the address and width `d93_fail_address` / `d93_fail_width` publish.
    #[test]
    fn the_scalar_fallback_keeps_the_refusal_address_and_width() {
        let fixture = upgrade_graph();
        let hole = fixture.first_group + REQUESTS_ARMY_PTR_OFF;
        let error = read_snapshot_from_country_roots(
            &HoleMemory {
                inner: &fixture.memory,
                hole,
            },
            fixture.module_base,
            5,
            &fixture.roots,
        )
        .expect_err("an unreadable CArmy pointer must refuse");
        assert!(
            matches!(&error, CarrierError::CountryRead { tag_db_id: 22, source, .. }
                if matches!(&**source, CarrierError::Memory { address, width }
                    if *address == hole && *width == 8)),
            "{error:?}"
        );
    }

    // kills: a counts helper that drifts from the decode-derived counting the
    // collector used to do for the D93 PERF record's country and entry
    // scalars.
    #[test]
    fn observed_counts_match_the_counts_taken_from_the_decoded_payload() {
        let fixture = upgrade_graph();
        let snapshot = read_snapshot_from_country_roots(
            &fixture.memory,
            fixture.module_base,
            5,
            &fixture.roots,
        )
        .unwrap();
        let counts = observed_counts(&snapshot);
        let payload = encode_payload(&receipt_frame(snapshot)).unwrap();
        let decoded = decode_payload(&payload).unwrap().snapshot.unwrap();
        // The arithmetic the collector ran over the decoded payload, restated.
        let countries = decoded.countries.len().min(u32::MAX as usize) as u32;
        let entries = decoded
            .countries
            .iter()
            .flat_map(|country| &country.groups)
            .map(|group| {
                1u64.saturating_add(group.requests.len() as u64)
                    .saturating_add(group.deliveries.len() as u64)
                    .saturating_add(
                        group
                            .deliveries
                            .iter()
                            .map(|delivery| delivery.produced.len() as u64)
                            .sum::<u64>(),
                    )
            })
            .sum::<u64>()
            .min(u64::from(u32::MAX)) as u32;
        assert_eq!(counts, ObservedCounts { countries, entries });
        assert_eq!(
            counts,
            ObservedCounts {
                countries: 3,
                entries: 213
            }
        );
    }

    // kills: dropping the duplicate SETS from `validate_snapshot` without the
    // strict-ascending test that subsumes them -- a duplicated country tag or
    // a duplicated army identity must still be refused, under the same name.
    #[test]
    fn duplicate_identities_are_still_refused_without_the_sets() {
        let mut snapshot = frame().snapshot.unwrap();
        let country = snapshot.countries[0].clone();
        snapshot.countries.push(country);
        let error = validate_snapshot(&snapshot).unwrap_err();
        assert!(
            matches!(&error, CarrierError::Invalid(reason)
                if reason == "AUR1 country order/identity/count is invalid"),
            "{error:?}"
        );

        let mut snapshot = frame().snapshot.unwrap();
        let group = snapshot.countries[0].groups[0].clone();
        snapshot.countries[0].groups.push(group);
        let error = validate_snapshot(&snapshot).unwrap_err();
        assert!(
            matches!(&error, CarrierError::Invalid(reason)
                if reason == "AUR1 army group order/identity/count is invalid"),
            "{error:?}"
        );
    }

    // kills: an `encoded_length` that drifts from the encoder. It is only a
    // capacity hint, so drift cannot corrupt a payload -- but a hint that is
    // quietly short puts the ~175 KB production payload back on the
    // reallocation path this lane took it off.
    #[test]
    fn encoded_length_is_exact_for_rowless_and_rich_frames() {
        let rowless = ArmyUpgradeFrame {
            elapsed_wall_ms: 1,
            frame_hour: 2,
            status_flags: 0x100,
            authority: AuthorityState::SourceUnavailable,
            snapshot: None,
        };
        let bytes = encode_payload(&rowless).unwrap();
        assert_eq!(bytes.len(), ROWLESS_PAYLOAD_LEN);
        assert_eq!(bytes.len(), encoded_length(&rowless));

        let fixture = upgrade_graph();
        let snapshot = read_snapshot_from_country_roots(
            &fixture.memory,
            fixture.module_base,
            5,
            &fixture.roots,
        )
        .unwrap();
        let rich = receipt_frame(snapshot);
        assert_eq!(encode_payload(&rich).unwrap().len(), encoded_length(&rich));
    }
}
