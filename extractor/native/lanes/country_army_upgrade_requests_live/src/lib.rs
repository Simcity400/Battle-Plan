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

compact_contract::wire_coded_codes! {
    no_discriminant AuthorityState: u32,
    code = raw,
    Ok = from_raw(value) -> Result<Self, CarrierError> { invalid("AUR1 authority is unknown") };

    SourceUnavailable = AUTHORITY_SOURCE_UNAVAILABLE,
    ExactA729Receipt = AUTHORITY_EXACT_A729_RECEIPT,
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
///
/// The unit the contract restarts is ONE COUNTRY, not the whole roster (lane
/// UD), so the window that has to hold still is one country's objects rather
/// than the whole 439-country graph, and the two reads a country is admitted
/// on are adjacent instead of a full roster sweep apart. A clean frame costs
/// two reads per country plus one membership scan of each -- the roster-wide
/// loop's clean cost plus the scan.
///
/// The worst case is this bound TIMES the roster bound, not this bound alone:
/// a country walked under all [`ROSTER_MEMBERSHIP_WITNESS_ATTEMPTS`] roster
/// walks can be read `ROSTER_MEMBERSHIP_WITNESS_ATTEMPTS *
/// COHERENT_READ_MAX_PASSES` = 24 times, with one membership scan per walk.
/// Both factors are constants and neither loop can be re-entered, so the walk
/// is bounded; it is not bounded by 8.
///
/// What the roster-wide comparison ALSO enforced, as a side effect of
/// demanding the whole graph be byte-identical, is that no `CArmy` moved
/// between two countries while the roster was being read. Per-country
/// coherence is satisfied on both sides of such a move, so that claim is now
/// made explicitly and separately by [`roster_membership_still_holds`], and a
/// roster whose membership will not hold still for one walk refuses under
/// [`ROSTER_MEMBERSHIP_UNWITNESSED`] rather than sealing a stitch.
pub const COHERENT_READ_MAX_PASSES: usize = 8;

/// Reads one country costs when nothing tears and nothing changes: the read
/// that proposes a value and the read that agrees with it. Reads beyond this
/// are what [`CoherenceCost::extra_country_reads`] counts.
pub const COHERENT_READS_A_SETTLED_COUNTRY_COSTS: usize = 2;

/// Whole-roster walks spent on getting one membership witness to hold. A
/// separate, smaller bound than [`COHERENT_READ_MAX_PASSES`]: it is reached
/// only when armies keep moving BETWEEN countries during the walk, and each
/// attempt costs a whole roster rather than one country. Sized to the budget
/// the collector's other whole-walk tear retries spend.
pub const ROSTER_MEMBERSHIP_WITNESS_ATTEMPTS: usize = 3;

/// The root at this address reads stably and is NOT the country the roster
/// admitted. Re-reading cannot make it so, so it refuses on the first read
/// like every other standing refusal.
pub const ROOT_IS_NOT_THIS_COUNTRY: &str =
    "AUR1 country root is not the country it was admitted as";
/// The root's own identity window would not read.
pub const ROOT_IDENTITY_UNREADABLE: &str = "AUR1 country root identity is unreadable";
/// Two successful reads of the root's identity window disagreed. Healable, so
/// it is one of [`TORN_READ_REASONS`].
const TORN_COUNTRY_IDENTITY: &str = "AUR1 country root identity changed during snapshot";
pub const TORN_COUNTRY_IDENTITY_EXHAUSTED: &str =
    "AUR1 country root identity kept tearing across every coherent-read restart";

const TORN_COUNTRY_GRAPH: &str = "AUR1 country/status graph changed during snapshot";
pub const TORN_COUNTRY_GRAPH_EXHAUSTED: &str =
    "AUR1 country/status graph kept tearing across every coherent-read restart";
const TORN_REQUEST_VECTOR: &str = "AUR1 request vector changed during snapshot";
pub const TORN_REQUEST_VECTOR_EXHAUSTED: &str =
    "AUR1 request vector kept tearing across every coherent-read restart";
const TORN_PRODUCED_VECTOR: &str = "AUR1 produced vector changed during snapshot";
pub const TORN_PRODUCED_VECTOR_EXHAUSTED: &str =
    "AUR1 produced vector kept tearing across every coherent-read restart";
const TORN_DELIVERY_VECTOR: &str = "AUR1 delivery vector changed during snapshot";
pub const TORN_DELIVERY_VECTOR_EXHAUSTED: &str =
    "AUR1 delivery vector kept tearing across every coherent-read restart";
/// The country's own status POINTER changed under the read. Its own name
/// rather than [`TORN_COUNTRY_GRAPH`]'s, because the two are different events:
/// one replaces the country's whole upgrade status, the other rewrites the
/// group vector inside it. Both halves of that close are lane QZ's, so both
/// are [`TearOrigin::Legacy`]; it is the NULL branch below that lane UD added.
const TORN_STATUS_POINTER: &str = "AUR1 country status pointer changed during snapshot";
pub const TORN_STATUS_POINTER_EXHAUSTED: &str =
    "AUR1 country status pointer kept tearing across every coherent-read restart";
/// A country whose structure was VALID on a completed pass and invalid on a
/// later one: a vector header past its capacity, a null pointer, a vtable that
/// no longer matches. The admission rules raise their own standing names for
/// these, and on the FIRST pass that is what they are. After a pass has
/// completed, the same observation is the engine writing a graph this walk
/// already read whole, and another read can settle it (Codex round 11).
///
/// It has no exhausted name of its own, and cannot have one: reaching it
/// requires a COMPLETED pass, a completed pass is a change, and a country that
/// both changed and tore exhausts as [`MIXED_RESTARTS_EXHAUSTED`]. A name for
/// the all-eight-reads-did-this case would be a unit no capture can carry
/// (Codex round 12, finding 2).
///
/// A country that read as having NO upgrade status had one by the time the
/// read closed. Distinct from [`TORN_STATUS_POINTER`], which is one status
/// object replaced by another: this is the country acquiring one, and it is
/// the only tear the null branch can raise.
const TORN_STATUS_POINTER_APPEARED: &str = "AUR1 country status pointer appeared during snapshot";
pub const TORN_STATUS_POINTER_APPEARED_EXHAUSTED: &str =
    "AUR1 country status pointer kept appearing across every coherent-read restart";

/// The AUR1 refusals raised by a re-verification: each one fires only because
/// a header this walk had already used was rewritten under it, which is the
/// same event the pass comparison detects and therefore takes the same
/// restart. Left of the pair is the reason a single read raises, right of it
/// is the reason the country takes when every restart tore -- distinct names
/// so a capture separates "our read straddled one mutation" (retried, and
/// absent from the wire when it settles) from "this country tore on all
/// [`COHERENT_READ_MAX_PASSES`] reads".
///
/// This table is one of THREE retry classes, and the whole rule has to be read
/// together or the boundary between them is easy to widen by accident, which
/// is how two defects reached rounds 11 and 12:
///
/// 1. A reason in this table is a tear on ANY pass, including the first: these
///    refusals only fire because a header this walk had already used was
///    rewritten under it, which cannot be a standing defect.
/// 2. Any other `Invalid` from the graph read -- vtable mismatches, bound
///    violations, null pointers, an unformable address -- is a standing
///    refusal on the FIRST pass, when nothing has validated those objects, and
///    a tear on a later one, when a completed pass has. That is
///    `convertible_after_admission`.
/// 3. Memory that will not serve a read, and the two STANDING root-identity
///    refusals, are never retried on any pass. Re-reading does not heal them,
///    and retrying would spend the bound and charge the capture gate's counter
///    for a refusal the engine did not make.
///
const TORN_READ_REASONS: [(&str, &str); 7] = [
    (TORN_COUNTRY_GRAPH, TORN_COUNTRY_GRAPH_EXHAUSTED),
    (TORN_REQUEST_VECTOR, TORN_REQUEST_VECTOR_EXHAUSTED),
    (TORN_PRODUCED_VECTOR, TORN_PRODUCED_VECTOR_EXHAUSTED),
    (TORN_DELIVERY_VECTOR, TORN_DELIVERY_VECTOR_EXHAUSTED),
    (TORN_COUNTRY_IDENTITY, TORN_COUNTRY_IDENTITY_EXHAUSTED),
    (TORN_STATUS_POINTER, TORN_STATUS_POINTER_EXHAUSTED),
    (
        TORN_STATUS_POINTER_APPEARED,
        TORN_STATUS_POINTER_APPEARED_EXHAUSTED,
    ),
];

/// Which detector raised a tear, and therefore what a restart of it is
/// evidence OF.
///
/// A `Legacy` detector existed before lane UD and propagated straight out of
/// the walk, so a frame that sealed after restarting one is a frame the old
/// code would have degraded. An `AddedByLaneUd` detector did not exist, so the
/// old code did not look and did not refuse; restarting one is a new
/// correctness check doing its job, not a converted refusal (Codex round 4,
/// finding 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TearOrigin {
    Legacy,
    AddedByLaneUd,
}

/// Which detector inside ONE COUNTRY'S WALK raises a given tear.
///
/// Origin is a property of the raise SITE, not of the cause: the same status
/// pointer changing is lane QZ's closing re-verification when the country had
/// a status object and lane UD's null close when it did not (Codex round 5,
/// finding 1). The membership witness does not consult this at all -- every
/// tear it raises is lane UD's, because the witness itself is.
fn country_walk_tear_origin(reason: &str) -> TearOrigin {
    match reason {
        TORN_COUNTRY_GRAPH | TORN_REQUEST_VECTOR | TORN_PRODUCED_VECTOR | TORN_DELIVERY_VECTOR
        | TORN_STATUS_POINTER => TearOrigin::Legacy,
        _ => TearOrigin::AddedByLaneUd,
    }
}


/// A country whose VALUES never settled -- every read completed and no two
/// agreed. Retained from lane QZ, and now wrapped in
/// [`CarrierError::CountryRead`] so it names which country did not settle.
const COHERENT_RESTARTS_EXHAUSTED: &str =
    "AUR1 object graph kept changing across every coherent-read restart";
/// A country where some reads completed and some tore, or which tore in more
/// than one place. Its own name, because calling it either of the other two
/// would put a cause on the wire that no read produced.
pub const MIXED_RESTARTS_EXHAUSTED: &str =
    "AUR1 country neither settled nor tore consistently across every coherent-read restart";
/// Armies kept moving between countries across every whole-roster walk, so no
/// walk could be witnessed as one membership.
pub const ROSTER_MEMBERSHIP_UNWITNESSED: &str =
    "AUR1 army membership kept moving between countries across every roster walk";
/// The roster walks did not all fail the same way: some found membership
/// moved, some were straddled, or different walks tore in different places or
/// different countries. Its own name for the same reason
/// [`MIXED_RESTARTS_EXHAUSTED`] has one.
pub const ROSTER_MEMBERSHIP_MIXED: &str =
    "AUR1 roster membership neither held nor tore consistently across every roster walk";
/// The membership witness tore on the same country's graph on every roster
/// walk. A ROSTER-level name: the country's own coherent read settled, and
/// only the later witness reads tore, so naming this after the country-level
/// restart would claim reads that did not happen (Codex round 4, finding 1).
pub const WITNESS_GRAPH_TEAR_EXHAUSTED: &str =
    "AUR1 membership witness kept tearing on the country/status graph across every roster walk";
/// The membership witness tore on the same country's status pointer on every
/// roster walk.
pub const WITNESS_STATUS_POINTER_TEAR_EXHAUSTED: &str =
    "AUR1 membership witness kept tearing on the country status pointer across every roster walk";
/// The membership witness found the same country's root identity being written
/// on every roster walk.
pub const WITNESS_IDENTITY_TEAR_EXHAUSTED: &str =
    "AUR1 membership witness kept tearing on the country root identity across every roster walk";

/// Re-proves that a root is still the country the roster authenticated it as.
///
/// The lane owns WHEN the proof must hold -- at the close of every read of that
/// country and again at its membership witness. The collector owns the proof
/// itself, because the `CCountry` identity offsets and the tag canonicalisation
/// live with the roster builder that produced the roots.
///
/// WHAT IT PROVES is exactly the roster's own per-object predicate, re-run: the
/// country vtable and the `CCountry+0x08` tag the root was admitted on. It does
/// NOT re-prove the country table's slot, the tag canonicalisation, or the
/// object's lifetime -- those are the roster builder's once-per-frame witnesses
/// and this trait does not repeat them (Codex round 2, finding 1). So an object
/// detached from the table whose bytes still read, or a replacement at the same
/// address carrying the same vtable and tag, is admitted here: what is closed
/// is a recycled root that reads as a DIFFERENT country, not every form of
/// reuse. Closing the rest needs a lifetime or generation witness the a729
/// evidence does not currently name.
pub trait CountryRootIdentity {
    fn reprove_root(&self, root: &CountryRoot) -> RootIdentity;
}

/// The outcomes of re-proving a root, kept apart because they take different
/// answers.
///
/// An object that reads STABLY as a different country, and one whose identity
/// will not read at all, are standing refusals that no re-read can heal. An
/// identity window that two successful reads disagreed on is the opposite: it
/// was being written while it was read, which is the same event the rest of
/// the walk's re-verifications detect, so it takes the same restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootIdentity {
    Proved,
    Disproved,
    Unreadable,
    Torn,
}

/// What the coherence contract actually spent on a frame, published on every
/// d93 frame whose reader ran.
///
/// A capture with zero d93 residual frames is not by itself evidence that the
/// restart lever works: `rc_vanilla5` had zero under the code this replaces.
/// One of these counters is that evidence and the others are not, so they are
/// kept apart rather than summed (Codex round 4, finding 2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CoherenceCost {
    /// Reads of a country beyond the two a settled country costs.
    pub extra_country_reads: u64,
    /// Restarts of a [`TearOrigin::Legacy`] detector -- one of the four
    /// re-verifications lane QZ already had. Each one propagated out of the
    /// walk and refused the whole frame before lane UD, so a frame that SEALED
    /// with this non-zero is a frame the old code would have degraded. This is
    /// the counter the capture gate rests on.
    pub legacy_tears_restarted: u64,
    /// Restarts of a detector lane UD ADDED: the root-identity proof, the
    /// null-owner status-pointer close, and every tear the membership witness
    /// raises. The old code did not look, so it did not refuse on these, and a
    /// restart of one converts nothing -- it is a new check doing its job.
    pub added_detector_tears: u64,
    /// Whole-roster walks beyond the first. The membership witness is also new,
    /// so this converts nothing either: without it the old code SEALED the
    /// stitched frame.
    pub extra_roster_walks: u64,
}

impl CoherenceCost {
    fn charge_tear(&mut self, origin: TearOrigin) {
        match origin {
            TearOrigin::Legacy => self.legacy_tears_restarted += 1,
            TearOrigin::AddedByLaneUd => self.added_detector_tears += 1,
        }
    }
}

/// Whether a refusal raised AFTER a completed pass is one this walk may take
/// again.
///
/// Only [`CarrierError::Invalid`] converts: memory that will not serve a read
/// is not healed by another one. The two standing root-identity refusals never
/// convert whenever they are raised -- an object that reads stably as another
/// country, and one whose identity will not read, are not this country's graph
/// changing, and retrying them would spend the bound and charge the capture
/// gate's counter for a detector lane UD added (Codex round 12, finding 1).
fn convertible_after_admission(error: &CarrierError) -> bool {
    match error {
        CarrierError::Invalid(reason) => {
            reason != ROOT_IS_NOT_THIS_COUNTRY && reason != ROOT_IDENTITY_UNREADABLE
        }
        _ => false,
    }
}

/// The [`TORN_READ_REASONS`] row an error matches, if it is a tear at all.
fn torn_read_row(error: &CarrierError) -> Option<(&'static str, &'static str)> {
    let CarrierError::Invalid(reason) = error else {
        return None;
    };
    TORN_READ_REASONS
        .iter()
        .copied()
        .find(|(torn, _)| *torn == reason.as_str())
}

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

pub fn read_snapshot_from_country_roots<M: ArmyUpgradeMemory, I: CountryRootIdentity>(
    memory: &M,
    module_base: u64,
    source_sequence: u64,
    roots: &[CountryRoot],
    identity: &I,
    cost: &mut CoherenceCost,
) -> Result<ArmyUpgradeSnapshot, CarrierError> {
    if module_base == 0 || source_sequence == 0 || roots.len() > MAX_COUNTRIES {
        return invalid("AUR1 country-root snapshot header is invalid");
    }
    let mut buffers = WalkBuffers::new();
    let mut country_ids = BTreeSet::new();
    for root in roots {
        if root.tag_db_id == 0 || root.object_ptr == 0 || !country_ids.insert(root.tag_db_id) {
            return invalid("AUR1 country identity is zero or duplicated");
        }
    }
    let mut roster = RosterExhaustion::default();
    for walk in 0..ROSTER_MEMBERSHIP_WITNESS_ATTEMPTS {
        let another_walk_follows = walk + 1 < ROSTER_MEMBERSHIP_WITNESS_ATTEMPTS;
        if walk > 0 {
            cost.extra_roster_walks += 1;
        }
        let mut countries = Vec::with_capacity(roots.len());
        let mut total_leaf_rows = 0usize;
        for root in roots {
            let groups = read_country_groups_coherent(
                memory,
                module_base,
                root,
                identity,
                &mut total_leaf_rows,
                &mut buffers,
                cost,
            )
            .map_err(|source| CarrierError::CountryRead {
                tag_db_id: root.tag_db_id,
                object_ptr: root.object_ptr,
                source: Box::new(source),
            })?;
            countries.push(CountryArmyUpgradeRow {
                tag_db_id: root.tag_db_id,
                groups,
            });
        }
        match witness_roster_membership(memory, roots, &countries, identity)? {
            MembershipWitness::Holds => {}
            MembershipWitness::Moved => {
                roster.saw_move();
                continue;
            }
            MembershipWitness::Torn(torn) => {
                // The witness is a detector lane UD added, so its restarts are
                // never charged to the legacy counter whatever they tore on;
                // and the last walk's tear restarts nothing.
                if another_walk_follows {
                    cost.added_detector_tears += 1;
                }
                roster.saw_tear(torn);
                continue;
            }
        }
        countries.sort_by_key(|country| country.tag_db_id);
        let snapshot = ArmyUpgradeSnapshot {
            source_sequence,
            countries,
        };
        validate_snapshot(&snapshot)?;
        return Ok(snapshot);
    }
    Err(roster.refusal())
}

/// What every roster walk's closing witness did, reduced to the one name and
/// the one country the refusal takes when the attempts run out.
///
/// The country is part of the reduction, not decoration: the same tear in two
/// DIFFERENT countries across two walks is not one country that kept tearing,
/// and naming it as one would put a country on the wire that settled on one of
/// the walks (Codex round 4, finding 1).
#[derive(Default)]
struct RosterExhaustion {
    moved: bool,
    torn: Option<WitnessTear>,
    mixed: bool,
}

impl RosterExhaustion {
    fn saw_move(&mut self) {
        self.mixed |= self.torn.is_some();
        self.moved = true;
    }

    fn saw_tear(&mut self, torn: WitnessTear) {
        self.mixed |= self.moved || self.torn.is_some_and(|seen| seen != torn);
        self.torn = Some(torn);
    }

    fn refusal(&self) -> CarrierError {
        match (self.mixed, self.torn) {
            (false, Some(torn)) => CarrierError::CountryRead {
                tag_db_id: torn.tag_db_id,
                object_ptr: torn.object_ptr,
                source: Box::new(CarrierError::Invalid(torn.exhausted().to_string())),
            },
            (false, None) => CarrierError::Invalid(ROSTER_MEMBERSHIP_UNWITNESSED.to_string()),
            (true, _) => CarrierError::Invalid(ROSTER_MEMBERSHIP_MIXED.to_string()),
        }
    }
}

/// Whether the roster still holds the army membership the walk was admitted
/// on: each country's army list, re-derived after the LAST country was read,
/// equal to the list that country was admitted with.
///
/// This is what the retired roster-wide pass comparison used to enforce as a
/// side effect of demanding the whole graph be byte-identical, and it is the
/// only cross-country claim the AUR1 payload can express -- a `CArmy` moving
/// between two countries between their admissions would otherwise reach the
/// frame twice, or not at all, with per-country coherence satisfied on both
/// sides.
///
/// A list that DIFFERS walks the roster again, and so does a witness whose own
/// read was straddled: both are events another read can settle, and neither may
/// refuse the frame on one look (Codex round 3, finding 2). The witness is
/// never an authority in the other direction: it can refuse a walk, never admit
/// rows the walk did not read. That direction is what makes its own reads safe
/// without the walk's vtable and back-reference checks: a witness that reads a
/// pointer the walk would have rejected produces a list that does not match,
/// which costs a roster walk and can never seal one.
///
/// Each country's root is re-proved at the CLOSE of its witness read as well as
/// at the close of its own walk: the walk's proof is only current as of that
/// country's admission, and the last country's admission can be a whole roster
/// later.
fn witness_roster_membership<M: ArmyUpgradeMemory, I: CountryRootIdentity>(
    memory: &M,
    roots: &[CountryRoot],
    countries: &[CountryArmyUpgradeRow],
    identity: &I,
) -> Result<MembershipWitness, CarrierError> {
    for (root, admitted) in roots.iter().zip(countries) {
        debug_assert_eq!(root.tag_db_id, admitted.tag_db_id);
        let country_read = |source| CarrierError::CountryRead {
            tag_db_id: root.tag_db_id,
            object_ptr: root.object_ptr,
            source: Box::new(source),
        };
        let witness_tear = |place: WitnessTearPlace| {
            Ok(MembershipWitness::Torn(WitnessTear {
                place,
                tag_db_id: root.tag_db_id,
                object_ptr: root.object_ptr,
            }))
        };
        let armies = match read_country_army_membership(memory, root) {
            Ok(armies) => armies,
            Err(error) => {
                return match torn_read_row(&error) {
                    Some((TORN_STATUS_POINTER | TORN_STATUS_POINTER_APPEARED, _)) => {
                        witness_tear(WitnessTearPlace::StatusPointer)
                    }
                    Some(_) => witness_tear(WitnessTearPlace::CountryGraph),
                    None => Err(country_read(error)),
                }
            }
        };
        match identity.reprove_root(root) {
            RootIdentity::Proved => {}
            RootIdentity::Torn => return witness_tear(WitnessTearPlace::RootIdentity),
            RootIdentity::Disproved => {
                return Err(country_read(CarrierError::Invalid(
                    ROOT_IS_NOT_THIS_COUNTRY.to_string(),
                )))
            }
            RootIdentity::Unreadable => {
                return Err(country_read(CarrierError::Invalid(
                    ROOT_IDENTITY_UNREADABLE.to_string(),
                )))
            }
        }
        if armies.len() != admitted.groups.len()
            || armies
                .iter()
                .zip(&admitted.groups)
                .any(|(army, group)| *army != group.army)
        {
            return Ok(MembershipWitness::Moved);
        }
    }
    Ok(MembershipWitness::Holds)
}

/// What one whole-roster membership witness found. `Moved` and `Torn` both
/// take another roster walk; they are kept apart so the name the roster takes
/// when the attempts run out is the one that actually happened.
enum MembershipWitness {
    Holds,
    Moved,
    Torn(WitnessTear),
}

/// A tear the witness raised, WITH the country it read when it tore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WitnessTear {
    place: WitnessTearPlace,
    tag_db_id: u32,
    object_ptr: u64,
}

/// Where a witness tear happened. Its own enum rather than the single-read
/// reason strings, because those name a COUNTRY-level restart the witness does
/// not take, and reaching the wire with one would resurrect a spelling that is
/// otherwise unreachable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WitnessTearPlace {
    CountryGraph,
    StatusPointer,
    RootIdentity,
}

impl WitnessTear {
    fn exhausted(&self) -> &'static str {
        match self.place {
            WitnessTearPlace::CountryGraph => WITNESS_GRAPH_TEAR_EXHAUSTED,
            WitnessTearPlace::StatusPointer => WITNESS_STATUS_POINTER_TEAR_EXHAUSTED,
            WitnessTearPlace::RootIdentity => WITNESS_IDENTITY_TEAR_EXHAUSTED,
        }
    }
}

/// One country's army identities, ascending, read through the same offsets and
/// helpers the full walk uses so the two cannot name different armies -- which
/// `the_membership_witness_names_the_walks_armies` pins on a static graph.
///
/// The slot pointers are taken one at a time rather than through an
/// [`ArrayWindow`]: the witness carries no widened-copy fallback of its own,
/// and a window a memory will not serve whole must not turn a readable array
/// into a refusal. It closes on the same re-verification the walk closes on,
/// so a witness that was itself straddled reports the tear rather than
/// comparing a list assembled out of two states.
///
/// That close detects a header that changed and STAYED changed. A header
/// rewritten and restored to the same begin/count while the slots underneath
/// were refilled -- an ABA within one witness -- reads as unchanged, here and
/// in the walk's own closing re-verification, which uses the same comparison.
/// Detecting it needs a generation counter, and no a729 evidence names one on
/// this vector.
fn read_country_army_membership<M: ArmyUpgradeMemory>(
    memory: &M,
    root: &CountryRoot,
) -> Result<Vec<PersistentId>, CarrierError> {
    match witness_membership(memory, root) {
        // EVERY structural refusal raised inside the witness is that graph
        // being written under it. Nothing in there is an admission: the
        // country's two coherent reads accepted and closed the same objects
        // moments earlier, so a header, pointer, identity or ADDRESS the
        // witness cannot make sense of is instability another roster walk can
        // settle. Converting at this ONE boundary is what makes that true of
        // the `?` on an address calculation as well, which runs before any
        // wrapper the call sits inside could see it (Codex round 10).
        //
        // Reasons that are already tears keep their own name: the witness
        // distinguishes a rewritten group vector from a replaced or newly
        // appeared status object, and that distinction reaches the wire.
        Err(CarrierError::Invalid(reason)) => {
            if TORN_READ_REASONS.iter().any(|(torn, _)| *torn == reason) {
                Err(CarrierError::Invalid(reason))
            } else {
                invalid(TORN_COUNTRY_GRAPH)
            }
        }
        // Memory that will not serve a read, and a truncated payload, are not
        // healed by another walk.
        other => other,
    }
}

fn witness_membership<M: ArmyUpgradeMemory>(
    memory: &M,
    root: &CountryRoot,
) -> Result<Vec<PersistentId>, CarrierError> {
    let status_ptr_at = add(root.object_ptr, COUNTRY_UPGRADE_STATUS_PTR_OFF)?;
    let owner = read_u64(memory, status_ptr_at)?;
    let mut armies = Vec::new();
    if owner == 0 {
        // A country with no upgrade status closes on the pointer that said so.
        // Returning the empty list on one read would let a null-to-non-null
        // transition inside the witness pass as "this country holds nothing"
        // (Codex round 3, finding 3).
        if read_u64(memory, status_ptr_at)? != 0 {
            return invalid(TORN_STATUS_POINTER_APPEARED);
        }
        return Ok(armies);
    }
    let (begin, count) = membership_group_header(memory, owner)?;
    for index in 0..count {
        let group_ptr = read_u64(memory, add(begin, (index as u64) * 8)?)?;
        if group_ptr == 0 {
            return invalid(TORN_COUNTRY_GRAPH);
        }
        let army_ptr = read_u64(memory, add(group_ptr, REQUESTS_ARMY_PTR_OFF)?)?;
        if army_ptr == 0 {
            return invalid(TORN_COUNTRY_GRAPH);
        }
        armies.push(read_persistent_id(memory, add(army_ptr, ARMY_ID_OFF)?)?);
    }
    let closing = membership_group_header(memory, owner)?;
    if closing != (begin, count) {
        return invalid(TORN_COUNTRY_GRAPH);
    }
    if read_u64(memory, status_ptr_at)? != owner {
        return invalid(TORN_STATUS_POINTER);
    }
    armies.sort();
    Ok(armies)
}

fn membership_group_header<M: ArmyUpgradeMemory>(
    memory: &M,
    owner: u64,
) -> Result<(u64, usize), CarrierError> {
    reread_vector_header(
        memory,
        owner,
        STATUS_GROUP_BEGIN_OFF,
        STATUS_GROUP_CAPACITY_OFF,
        STATUS_GROUP_COUNT_OFF,
        MAX_GROUPS_PER_COUNTRY,
        "army-upgrade group",
    )
}

/// One country, read until two consecutive reads of it agree.
///
/// The running leaf total is only advanced by the read that is ACCEPTED, so a
/// restarted country contributes its rows to the bound once.
fn read_country_groups_coherent<M: ArmyUpgradeMemory, I: CountryRootIdentity>(
    memory: &M,
    module_base: u64,
    root: &CountryRoot,
    identity: &I,
    total_leaf_rows: &mut usize,
    buffers: &mut WalkBuffers,
    cost: &mut CoherenceCost,
) -> Result<Vec<ArmyUpgradeGroup>, CarrierError> {
    let mut previous: Option<Vec<ArmyUpgradeGroup>> = None;
    // Whether any pass of THIS country has completed. Before the first one,
    // the objects are being admitted and a structural refusal is a standing
    // defect; after it, the same refusal is that structure changing under a
    // walk that already read it whole.
    let mut a_pass_completed = false;
    let mut exhaustion = Exhaustion::default();
    for read in 0..COHERENT_READ_MAX_PASSES {
        let another_read_follows = read + 1 < COHERENT_READ_MAX_PASSES;
        if read >= COHERENT_READS_A_SETTLED_COUNTRY_COSTS {
            cost.extra_country_reads += 1;
        }
        let mut leaf_rows = *total_leaf_rows;
        let graph = read_country_groups(memory, module_base, root, &mut leaf_rows, buffers);
        // The root is re-proved on EVERY pass, including passes whose graph
        // read refused, and its answer OUTRANKS that refusal: a graph that
        // stopped making sense because the object is no longer this country
        // has to say so, not be filed as the graph tearing (Codex round 12,
        // finding 1).
        let read = match identity.reprove_root(root) {
            RootIdentity::Proved => graph,
            RootIdentity::Disproved => invalid(ROOT_IS_NOT_THIS_COUNTRY),
            RootIdentity::Unreadable => invalid(ROOT_IDENTITY_UNREADABLE),
            RootIdentity::Torn => invalid(TORN_COUNTRY_IDENTITY),
        };
        match read {
            Ok(mut groups) => {
                a_pass_completed = true;
                groups.sort_by_key(|group| group.army);
                if previous.as_deref() == Some(groups.as_slice()) {
                    *total_leaf_rows = leaf_rows;
                    return Ok(groups);
                }
                previous = Some(groups);
                exhaustion.saw_change();
            }
            Err(error) => {
                if let Some((torn_now, torn)) = torn_read_row(&error) {
                    // Not charged on the last pass: the counters are named for
                    // restarts, and the loop is about to exhaust without
                    // taking another read (Codex round 9, finding 2).
                    if another_read_follows {
                        cost.charge_tear(country_walk_tear_origin(torn_now));
                    }
                    previous = None;
                    exhaustion.saw_tear(torn);
                } else if a_pass_completed && convertible_after_admission(&error) {
                    // A structure that was valid on a completed pass and is
                    // not now was written under a walk that already read it
                    // whole. Charged Legacy: the detector that raised it is
                    // lane QZ's own admission validator, and before lane UD
                    // that refusal propagated out and cost the hour.
                    if another_read_follows {
                        cost.charge_tear(TearOrigin::Legacy);
                    }
                    previous = None;
                    exhaustion.saw_structural_regression();
                } else {
                    return Err(error);
                }
            }
        }
    }
    invalid(exhaustion.reason())
}

/// What every read of one country did, reduced to the one name the wire takes
/// when the bound runs out.
///
/// A country that tore on every read, a country whose values never settled,
/// and a country that did some of each are three different states, and naming
/// any of them after the LAST read alone would put a cause on the wire that
/// did not happen.
#[derive(Default)]
struct Exhaustion {
    changed: bool,
    torn: Option<&'static str>,
    mixed: bool,
}

impl Exhaustion {
    fn saw_change(&mut self) {
        self.mixed |= self.torn.is_some();
        self.changed = true;
    }

    /// A structural refusal that only happens after a completed pass, so the
    /// country has already changed and this reduces to mixed by construction.
    /// It carries no tear name because no reachable outcome would use one.
    fn saw_structural_regression(&mut self) {
        debug_assert!(self.changed);
        self.mixed = true;
    }

    fn saw_tear(&mut self, torn: &'static str) {
        self.mixed |= self.changed || self.torn.is_some_and(|seen| seen != torn);
        self.torn = Some(torn);
    }

    fn reason(&self) -> &'static str {
        match (self.mixed, self.torn) {
            (false, Some(torn)) => torn,
            (false, None) => COHERENT_RESTARTS_EXHAUSTED,
            (true, _) => MIXED_RESTARTS_EXHAUSTED,
        }
    }
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
    let status_ptr_at = add(root.object_ptr, COUNTRY_UPGRADE_STATUS_PTR_OFF)?;
    let owner = read_u64(memory, status_ptr_at)?;
    let mut groups = Vec::new();
    if owner == 0 {
        // Closed on the same pointer, for the same reason the witness closes
        // its null branch: an empty country is a claim, and one read of a
        // pointer that is being written does not support it. This close is
        // lane UD's, unlike the non-null close below, which is lane QZ's --
        // which is why the two take different names and different counters.
        if read_u64(memory, status_ptr_at)? != 0 {
            return invalid(TORN_STATUS_POINTER_APPEARED);
        }
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
    // Lane QZ's close, split by FIELD: one name per event, so a status object
    // that was replaced is not reported as its group vector being rewritten
    // (Codex round 5, finding 1). Both halves stay Legacy -- the split changes
    // what the refusal is CALLED, not which detector found it.
    if reread_vector_header_or_tear(
        memory,
        owner,
        STATUS_GROUP_BEGIN_OFF,
        STATUS_GROUP_CAPACITY_OFF,
        STATUS_GROUP_COUNT_OFF,
        MAX_GROUPS_PER_COUNTRY,
        "army-upgrade group",
        TORN_COUNTRY_GRAPH,
    )? != (begin, count)
    {
        return invalid(TORN_COUNTRY_GRAPH);
    }
    if read_u64(memory, status_ptr_at)? != owner {
        return invalid(TORN_STATUS_POINTER);
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
    if reread_vector_header_or_tear(
        memory,
        group_ptr,
        REQUEST_ROW_BEGIN_OFF,
        REQUEST_ROW_CAPACITY_OFF,
        REQUEST_ROW_COUNT_OFF,
        MAX_REQUEST_ROWS_PER_GROUP,
        "army-upgrade request",
        TORN_REQUEST_VECTOR,
    )? != (begin, count)
    {
        return invalid(TORN_REQUEST_VECTOR);
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
        if reread_vector_header_or_tear(
            memory,
            delivery,
            PRODUCED_ROW_BEGIN_OFF,
            PRODUCED_ROW_CAPACITY_OFF,
            PRODUCED_ROW_COUNT_OFF,
            MAX_PRODUCED_ROWS_PER_DELIVERY,
            "army-upgrade produced equipment",
            TORN_PRODUCED_VECTOR,
        )? != (produced_begin, produced_count)
        {
            return invalid(TORN_PRODUCED_VECTOR);
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
    if reread_vector_header_or_tear(
        memory,
        group_ptr,
        DELIVERY_BEGIN_OFF,
        DELIVERY_CAPACITY_OFF,
        DELIVERY_COUNT_OFF,
        MAX_DELIVERIES_PER_GROUP,
        "army-upgrade delivery",
        TORN_DELIVERY_VECTOR,
    )? != (begin, count)
    {
        return invalid(TORN_DELIVERY_VECTOR);
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

/// A CLOSING re-verification of a vector header.
///
/// The admitted header was already valid and its entries were walked, so a
/// fresh observation that is structurally invalid -- a count past its
/// capacity, a negative field, a null begin under a nonzero count -- is not a
/// malformed vector. It is that header being written while it was read, which
/// is the same event a merely DIFFERENT header is, and it takes the same
/// restart (Codex round 8, finding 1). Admission is untouched: a header that
/// is invalid on the FIRST read still refuses on the first read, under its own
/// standing name.
///
/// Read failures pass through unchanged. Memory that will not serve the
/// closing read is not a tear, and re-reading does not fix it.
fn reread_vector_header_or_tear<M: ArmyUpgradeMemory>(
    memory: &M,
    object: u64,
    begin_off: u64,
    capacity_off: u64,
    count_off: u64,
    maximum: usize,
    label: &str,
    torn: &'static str,
) -> Result<(u64, usize), CarrierError> {
    match reread_vector_header(
        memory,
        object,
        begin_off,
        capacity_off,
        count_off,
        maximum,
        label,
    ) {
        Err(CarrierError::Invalid(_)) => invalid(torn),
        other => other,
    }
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
        &StableRoots,
        &mut CoherenceCost::default(),
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

    /// Every root stays the country it was authenticated as. The tests that
    /// care about a recycled root use `RecycledRoot` instead.
    struct StableRoots;

    impl CountryRootIdentity for StableRoots {
        fn reprove_root(&self, _root: &CountryRoot) -> RootIdentity {
            RootIdentity::Proved
        }
    }

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

    /// One country chain -- status, one group, one CArmy -- stamped at every
    /// fixture address shifted by `delta`, so a roster can hold two countries
    /// that share no object.
    fn stamp_country(memory: &mut SparseMemory, delta: u64, army_kind: u32, army_id: u32) {
        let country = FIXTURE_COUNTRY + delta;
        let status = FIXTURE_STATUS + delta;
        let group_slots = FIXTURE_GROUP_SLOTS + delta;
        let group = FIXTURE_GROUP + delta;
        let army = FIXTURE_ARMY + delta;

        memory.present(status, STATUS_BLOCK_LEN);
        memory.present(group, GROUP_BLOCK_LEN);
        memory.present(army, ARMY_BLOCK_LEN);
        memory.u64(country + COUNTRY_UPGRADE_STATUS_PTR_OFF, status);

        memory.u64(status, FIXTURE_BASE + STATUS_VTABLE_RVA);
        memory.u64(
            status + STATUS_PERSISTENT_VTABLE_OFF,
            FIXTURE_BASE + STATUS_PERSISTENT_VTABLE_RVA,
        );
        memory.u64(status + STATUS_GROUP_BEGIN_OFF, group_slots);
        memory.u32(status + STATUS_GROUP_CAPACITY_OFF, 1);
        memory.u32(status + STATUS_GROUP_COUNT_OFF, 1);
        memory.u64(group_slots, group);

        memory.u64(group, FIXTURE_BASE + REQUESTS_VTABLE_RVA);
        memory.u64(
            group + REQUESTS_PERSISTENT_VTABLE_OFF,
            FIXTURE_BASE + REQUESTS_PERSISTENT_VTABLE_RVA,
        );
        memory.u32(group + REQUESTS_VERSION_OFF, REQUESTS_VERSION);
        memory.u64(group + REQUESTS_ARMY_PTR_OFF, army);
        memory.u64(group + REQUESTS_STATUS_PTR_OFF, status);
        memory.u64(group + REQUEST_ROW_BEGIN_OFF, 0);
        memory.u32(group + REQUEST_ROW_CAPACITY_OFF, 0);
        memory.u32(group + REQUEST_ROW_COUNT_OFF, 0);
        memory.u64(group + DELIVERY_BEGIN_OFF, 0);
        memory.u32(group + DELIVERY_CAPACITY_OFF, 0);
        memory.u32(group + DELIVERY_COUNT_OFF, 0);

        memory.u64(army, FIXTURE_BASE + ARMY_VTABLE_RVA);
        memory.u64(
            army + ARMY_PERSISTENT_VTABLE_OFF,
            FIXTURE_BASE + ARMY_PERSISTENT_VTABLE_RVA,
        );
        memory.u32(army + ARMY_ID_OFF, army_kind);
        memory.u32(army + ARMY_ID_OFF + 4, army_id);
    }

    fn single_group_memory(army_kind: u32, army_id: u32) -> SparseMemory {
        let mut memory = SparseMemory::default();
        stamp_country(&mut memory, 0, army_kind, army_id);
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
        &StableRoots,
        &mut CoherenceCost::default(),
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
        &StableRoots,
        &mut CoherenceCost::default(),
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
        &StableRoots,
        &mut CoherenceCost::default(),
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
        // Three walk reads, then the closing membership witness reads the
        // CArmy identity once more.
        assert_eq!(memory.watch_reads.get(), 4);
    }

    // kills: any unbounded retry, and any collapse of a persistently torn
    // graph into measured data: a graph that changes on every pass must
    // exhaust exactly COHERENT_READ_MAX_PASSES passes and refuse under its own
    // restart-exhausted name. The elapsed-time bound kills retries that WAIT:
    // the eight fixture passes are memory-only and complete in microseconds,
    // so any per-pass sleep/yield of meaningful size blows the ceiling. That
    // no production path sleeps is additionally established by inspection --
    // the loop body is only `read_country_groups` calls.
    #[test]
    fn coherent_read_refuses_by_name_when_the_graph_never_settles() {
        let memory = mutating_memory(vec![14_646, 14_647], true);
        let started = std::time::Instant::now();
        let error = read_mutating(&memory)
            .err()
            .expect("a graph that changes on every pass must stay refused");
        let elapsed = started.elapsed();
        assert!(
            matches!(&error, CarrierError::CountryRead { tag_db_id: 1, source, .. }
                if matches!(source.as_ref(), CarrierError::Invalid(reason)
                    if reason
                        == "AUR1 object graph kept changing across every coherent-read restart")),
            "never-settling graph produced {error:?}"
        );
        assert_eq!(memory.watch_reads.get(), COHERENT_READ_MAX_PASSES);
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "the bounded refusal must not wait (took {elapsed:?})"
        );
    }

    // --- lane UD: the tear detectors take the same restart ----------------

    const FIXTURE_DELTA_B: u64 = 0x0010_0000;

    /// The country's status pointer as the walk's OPENING read, and something
    /// else as its closing re-verification read on the first `tear_attempts`
    /// reads of the country -- the engine relocating `CCountryArmyUpgradeStatus`
    /// under the walk, which is what [`TORN_STATUS_POINTER`] reports. It was
    /// `TORN_COUNTRY_GRAPH` until the closing comparison was split by field,
    /// because that one name covered both the pointer and the group vector.
    ///
    /// The two reads at that address per walk are the whole protocol: index
    /// `2n` opens attempt `n` and index `2n+1` closes it. A torn attempt stops
    /// AT the closing read, so the pairing holds across restarts.
    struct TearingMemory {
        inner: SparseMemory,
        watch_address: u64,
        tear_attempts: usize,
        reads: std::cell::Cell<usize>,
    }

    impl ArmyUpgradeMemory for TearingMemory {
        fn read_exact(&self, address: u64, out: &mut [u8]) -> Result<(), ()> {
            self.inner.read_exact(address, out)?;
            if address == self.watch_address && out.len() == 8 {
                let index = self.reads.get();
                self.reads.set(index + 1);
                if index % 2 == 1 && index / 2 < self.tear_attempts {
                    out.copy_from_slice(&(FIXTURE_STATUS + 0x8000).to_le_bytes());
                }
            }
            Ok(())
        }
    }

    fn tearing_memory(inner: SparseMemory, tear_attempts: usize) -> TearingMemory {
        TearingMemory {
            inner,
            watch_address: FIXTURE_COUNTRY + COUNTRY_UPGRADE_STATUS_PTR_OFF,
            tear_attempts,
            reads: std::cell::Cell::new(0),
        }
    }

    fn read_tearing(memory: &TearingMemory) -> Result<ArmyUpgradeSnapshot, CarrierError> {
        read_snapshot_from_country_roots(
            memory,
            FIXTURE_BASE,
            1,
            &[CountryRoot {
                tag_db_id: 1,
                object_ptr: FIXTURE_COUNTRY,
            }],
        &StableRoots,
        &mut CoherenceCost::default(),
        )
    }

    // kills: the `?` that carried a tear detector straight out of the walk, so
    // the four re-verification refusals spent ZERO restarts and refused the
    // frame on one straddled read -- the rc_vanilla7 hour-60831057 shape.
    #[test]
    fn a_torn_read_restarts_and_settles() {
        let memory = tearing_memory(single_group_memory(ARMY_ID_TYPE, 14_646), 1);
        let snapshot = read_tearing(&memory)
            .unwrap_or_else(|error| panic!("a settling tear must still seal: {error:?}"));
        assert_eq!(snapshot.countries[0].groups[0].army.id, 14_646);
        // Three reads of the country at two status-pointer reads each, then
        // TWO more for the closing membership witness, which opens on that
        // pointer and closes on it exactly as a pass does.
        // `the_status_pointer_read_schedule_is_what_the_tear_tests_assume`
        // pins the same shape on a graph that never tears.
        assert_eq!(memory.reads.get(), 3 * 2 + 2);
    }

    // kills: retrying a tear forever, and renaming a standing tear as the
    // value-churn refusal -- a country that tears on every read must exhaust
    // exactly COHERENT_READ_MAX_PASSES reads and refuse under a name only that
    // state can reach, still without waiting.
    #[test]
    fn a_standing_tear_refuses_under_its_own_exhausted_name() {
        let memory = tearing_memory(single_group_memory(ARMY_ID_TYPE, 14_646), usize::MAX);
        let started = std::time::Instant::now();
        let error = read_tearing(&memory)
            .err()
            .expect("a country that tears on every read must stay refused");
        let elapsed = started.elapsed();
        assert!(
            matches!(&error, CarrierError::CountryRead { tag_db_id: 1, source, .. }
                if matches!(source.as_ref(), CarrierError::Invalid(reason)
                    if reason == TORN_STATUS_POINTER_EXHAUSTED)),
            "standing tear produced {error:?}"
        );
        assert_eq!(memory.reads.get(), 2 * COHERENT_READ_MAX_PASSES);
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "the bounded refusal must not wait (took {elapsed:?})"
        );
    }

    // kills: retrying refusals the engine cannot fix by being read again --
    // which would both spend the bound on a dead read and rename a standing
    // defect as a tear.
    #[test]
    fn a_refusal_that_is_not_a_tear_is_not_retried() {
        let memory = tearing_memory(single_group_memory(ARMY_ID_TYPE, 0), 0);
        let error = read_tearing(&memory)
            .err()
            .expect("a null CArmy identity must refuse");
        assert!(
            matches!(&error, CarrierError::CountryRead { source, .. }
                if matches!(source.as_ref(), CarrierError::Invalid(reason)
                    if reason == "AUR1 CArmy persistent identity is invalid or duplicated")),
            "a non-tear refusal produced {error:?}"
        );
        assert_eq!(memory.reads.get(), 1);
    }

    /// A roster whose FIRST country is static and whose SECOND never settles.
    struct RosterMemory {
        inner: SparseMemory,
        churn_address: u64,
        churn_reads: std::cell::Cell<usize>,
        settled_address: u64,
        settled_reads: std::cell::Cell<usize>,
    }

    impl ArmyUpgradeMemory for RosterMemory {
        fn read_exact(&self, address: u64, out: &mut [u8]) -> Result<(), ()> {
            self.inner.read_exact(address, out)?;
            if address == self.settled_address && out.len() == 8 {
                self.settled_reads.set(self.settled_reads.get() + 1);
            }
            let covers = address <= self.churn_address
                && self
                    .churn_address
                    .checked_add(4)
                    .is_some_and(|end| end <= address + out.len() as u64);
            if covers {
                let index = self.churn_reads.get();
                self.churn_reads.set(index + 1);
                let at = (self.churn_address - address) as usize;
                out[at..at + 4].copy_from_slice(&(14_646 + index as u32).to_le_bytes());
            }
            Ok(())
        }
    }

    // kills: restarting the WHOLE roster when one country moves. Country 1 is
    // static and country 2 changes on every read; country 1 must be admitted
    // on its own two reads and never walked again, which is the granularity
    // claim behind COHERENT_READ_MAX_PASSES -- the roster-wide loop read it
    // COHERENT_READ_MAX_PASSES times for a neighbour's churn.
    #[test]
    fn one_country_that_never_settles_does_not_re_walk_its_neighbours() {
        let mut inner = SparseMemory::default();
        stamp_country(&mut inner, FIXTURE_DELTA_B, ARMY_ID_TYPE, 14_646);
        stamp_country(&mut inner, 0, ARMY_ID_TYPE, 14_646);
        let memory = RosterMemory {
            inner,
            churn_address: FIXTURE_ARMY + ARMY_ID_OFF + 4,
            churn_reads: std::cell::Cell::new(0),
            settled_address: FIXTURE_COUNTRY + FIXTURE_DELTA_B + COUNTRY_UPGRADE_STATUS_PTR_OFF,
            settled_reads: std::cell::Cell::new(0),
        };
        let error = read_snapshot_from_country_roots(
            &memory,
            FIXTURE_BASE,
            1,
            &[
                CountryRoot {
                    tag_db_id: 1,
                    object_ptr: FIXTURE_COUNTRY + FIXTURE_DELTA_B,
                },
                CountryRoot {
                    tag_db_id: 2,
                    object_ptr: FIXTURE_COUNTRY,
                },
            ],
        &StableRoots,
        &mut CoherenceCost::default(),
        )
        .err()
        .expect("the churning country must refuse the frame");
        assert!(
            matches!(&error, CarrierError::CountryRead { tag_db_id: 2, source, .. }
                if matches!(source.as_ref(), CarrierError::Invalid(reason)
                    if reason
                        == "AUR1 object graph kept changing across every coherent-read restart")),
            "the churning country must be the one named: {error:?}"
        );
        assert_eq!(memory.churn_reads.get(), COHERENT_READ_MAX_PASSES);
        assert_eq!(memory.settled_reads.get(), 4);
    }

    // kills: an exhausted-tear name with no exact arm in the collector's
    // mapper, which would file a standing tear under
    // `d93_fail_reason_other_invalid`; and a single-read tear reason that
    // collides with the exhausted family's suffix, which would file a retried
    // straddle as a standing one.
    #[test]
    fn the_exhausted_tear_names_are_exactly_the_suffix_family() {
        // The tail every right-hand name ends with and no left-hand name
        // does. The VERB is cause-specific -- a status object that keeps being
        // replaced tears, one that keeps appearing appears -- so the shared
        // part is the tail alone. The collector maps each exhausted cause by
        // its own exact name, so a spelling that drifted into or out of this
        // family would silently change which unit it lands on; this is where
        // that drift is caught, which is why the constant lives with the
        // assertion.
        const TORN_READ_EXHAUSTED_SUFFIX: &str = " across every coherent-read restart";
        for (torn, exhausted) in TORN_READ_REASONS {
            assert!(
                exhausted.ends_with(TORN_READ_EXHAUSTED_SUFFIX),
                "{exhausted} must join the collector's one arm"
            );
            assert!(
                !torn.ends_with(TORN_READ_EXHAUSTED_SUFFIX),
                "{torn} is one straddled read, not a standing tear"
            );
            assert_eq!(
                torn_read_row(&CarrierError::Invalid(torn.to_string()))
                    .map(|(_, exhausted)| exhausted),
                Some(exhausted)
            );
        }
        assert!(torn_read_row(&CarrierError::Invalid(
            "AUR1 CArmy persistent identity is invalid or duplicated".to_string()
        ))
        .is_none());
        assert!(torn_read_row(&CarrierError::Truncated).is_none());
    }

    // kills: naming an exhausted country after its LAST read alone. Seven
    // value changes then one tear is not "kept tearing"; seven tears then one
    // completed read is not value churn; two different tears are neither. Each
    // of those would publish a cause no read produced.
    #[test]
    fn exhaustion_is_named_after_every_read_not_the_last_one() {
        let all_torn = |torn: &'static str| {
            let mut seen = Exhaustion::default();
            for _ in 0..COHERENT_READ_MAX_PASSES {
                seen.saw_tear(torn);
            }
            seen
        };
        assert_eq!(
            all_torn(TORN_COUNTRY_GRAPH_EXHAUSTED).reason(),
            TORN_COUNTRY_GRAPH_EXHAUSTED
        );

        let mut only_changed = Exhaustion::default();
        for _ in 0..COHERENT_READ_MAX_PASSES {
            only_changed.saw_change();
        }
        assert_eq!(only_changed.reason(), COHERENT_RESTARTS_EXHAUSTED);

        let mut changed_then_torn = Exhaustion::default();
        for _ in 1..COHERENT_READ_MAX_PASSES {
            changed_then_torn.saw_change();
        }
        changed_then_torn.saw_tear(TORN_COUNTRY_GRAPH_EXHAUSTED);
        assert_eq!(changed_then_torn.reason(), MIXED_RESTARTS_EXHAUSTED);

        let mut torn_then_changed = all_torn(TORN_COUNTRY_GRAPH_EXHAUSTED);
        torn_then_changed.saw_change();
        assert_eq!(torn_then_changed.reason(), MIXED_RESTARTS_EXHAUSTED);

        let mut two_tears = all_torn(TORN_COUNTRY_GRAPH_EXHAUSTED);
        two_tears.saw_tear(TORN_REQUEST_VECTOR_EXHAUSTED);
        assert_eq!(two_tears.reason(), MIXED_RESTARTS_EXHAUSTED);
    }

    /// A roster of two countries where the first country's army changes only
    /// on the closing witness read -- the shape of an army leaving a country
    /// AFTER that country was admitted, which per-country coherence cannot
    /// see because both of its reads happened before the move.
    ///
    /// Each roster walk reads this army three times: twice to admit the
    /// country, once for the witness. `settle` decides whether the move
    /// happens once (the next walk admits the new membership) or on every
    /// walk's witness (no walk is ever witnessable).
    struct MovingArmyMemory {
        inner: SparseMemory,
        watched: u64,
        reads: std::cell::Cell<usize>,
        settle: bool,
    }

    impl ArmyUpgradeMemory for MovingArmyMemory {
        fn read_exact(&self, address: u64, out: &mut [u8]) -> Result<(), ()> {
            self.inner.read_exact(address, out)?;
            let covers = address <= self.watched
                && self
                    .watched
                    .checked_add(4)
                    .is_some_and(|end| end <= address + out.len() as u64);
            if covers {
                let index = self.reads.get();
                self.reads.set(index + 1);
                let moved = if self.settle {
                    index >= 2
                } else {
                    index % 3 == 2
                };
                let at = (self.watched - address) as usize;
                let id: u32 = if moved { 14_999 } else { 14_646 };
                out[at..at + 4].copy_from_slice(&id.to_le_bytes());
            }
            Ok(())
        }
    }

    fn read_two_country_roster<M: ArmyUpgradeMemory>(
        memory: &M,
    ) -> Result<ArmyUpgradeSnapshot, CarrierError> {
        read_snapshot_from_country_roots(
            memory,
            FIXTURE_BASE,
            1,
            &[
                CountryRoot {
                    tag_db_id: 1,
                    object_ptr: FIXTURE_COUNTRY,
                },
                CountryRoot {
                    tag_db_id: 2,
                    object_ptr: FIXTURE_COUNTRY + FIXTURE_DELTA_B,
                },
            ],
            &StableRoots,
            &mut CoherenceCost::default(),
        )
    }

    fn moving_army_memory(settle: bool) -> MovingArmyMemory {
        let mut inner = SparseMemory::default();
        stamp_country(&mut inner, 0, ARMY_ID_TYPE, 14_646);
        stamp_country(&mut inner, FIXTURE_DELTA_B, ARMY_ID_TYPE, 14_646);
        MovingArmyMemory {
            inner,
            watched: FIXTURE_ARMY + ARMY_ID_OFF + 4,
            reads: std::cell::Cell::new(0),
            settle,
        }
    }

    // kills: publishing a roster stitched across a membership change. Per-
    // country coherence is satisfied on both sides of an army that leaves the
    // first country after it was admitted, so without the closing witness the
    // frame seals a country holding an army it no longer holds. The retired
    // roster-wide pass comparison caught this as a side effect; the witness is
    // what replaces it.
    #[test]
    fn a_roster_whose_membership_moved_is_walked_again() {
        let memory = moving_army_memory(true);
        let snapshot = read_two_country_roster(&memory)
            .unwrap_or_else(|error| panic!("a settled second walk must seal: {error:?}"));
        assert_eq!(snapshot.countries[0].groups[0].army.id, 14_999);
        assert_eq!(memory.reads.get(), 6);
    }

    /// Two countries where one `CArmy` TRANSFERS from the second to the first
    /// between their admissions, by the two group counts changing together.
    ///
    /// The move is triggered by the first read of the second country's status
    /// object, which is the moment the walk leaves country 1 -- so country 1 is
    /// admitted empty, country 2 is admitted empty, and the army the engine
    /// holds reaches the frame in NEITHER country. No per-country comparison
    /// can see it: both countries were coherent on both of their own reads.
    struct TransferredArmyMemory {
        inner: SparseMemory,
        first_count_at: u64,
        second_count_at: u64,
        second_status_at: u64,
        moved: std::cell::Cell<bool>,
    }

    impl TransferredArmyMemory {
        /// Overwrite a u32 in an already-served buffer when the read covered
        /// it, whatever width the caller asked for.
        fn patch_u32(address: u64, out: &mut [u8], field: u64, value: u32) {
            let Some(end) = field.checked_add(4) else {
                return;
            };
            if address <= field && end <= address + out.len() as u64 {
                let at = (field - address) as usize;
                out[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
    }

    impl ArmyUpgradeMemory for TransferredArmyMemory {
        fn read_exact(&self, address: u64, out: &mut [u8]) -> Result<(), ()> {
            self.inner.read_exact(address, out)?;
            let moved = self.moved.get();
            Self::patch_u32(address, out, self.first_count_at, u32::from(moved));
            Self::patch_u32(address, out, self.second_count_at, u32::from(!moved));
            if !moved && address <= self.second_status_at
                && self.second_status_at < address + out.len() as u64
            {
                self.moved.set(true);
            }
            Ok(())
        }
    }

    // kills: sealing a frame that LOSES an army. An army moving between two
    // countries after the first was admitted leaves per-country coherence
    // satisfied on both sides, and -- unlike the double-count direction --
    // leaves nothing behind for the payload's own duplicate check to find.
    // The closing witness is the only reader that can see it.
    #[test]
    fn an_army_transferred_between_countries_is_walked_again() {
        let mut inner = SparseMemory::default();
        stamp_country(&mut inner, 0, ARMY_ID_TYPE, 14_646);
        stamp_country(&mut inner, FIXTURE_DELTA_B, ARMY_ID_TYPE, 14_646);
        let memory = TransferredArmyMemory {
            inner,
            first_count_at: FIXTURE_STATUS + STATUS_GROUP_COUNT_OFF,
            second_count_at: FIXTURE_STATUS + FIXTURE_DELTA_B + STATUS_GROUP_COUNT_OFF,
            second_status_at: FIXTURE_STATUS + FIXTURE_DELTA_B,
            moved: std::cell::Cell::new(false),
        };
        let mut cost = CoherenceCost::default();
        let snapshot = read_snapshot_from_country_roots(
            &memory,
            FIXTURE_BASE,
            1,
            &[
                CountryRoot {
                    tag_db_id: 1,
                    object_ptr: FIXTURE_COUNTRY,
                },
                CountryRoot {
                    tag_db_id: 2,
                    object_ptr: FIXTURE_COUNTRY + FIXTURE_DELTA_B,
                },
            ],
            &StableRoots,
            &mut cost,
        )
        .unwrap_or_else(|error| panic!("the settled second walk must seal: {error:?}"));
        assert_eq!(snapshot.countries[0].groups.len(), 1);
        assert_eq!(snapshot.countries[0].groups[0].army.id, 14_646);
        assert!(snapshot.countries[1].groups.is_empty());
        assert_eq!(cost.extra_roster_walks, 1);
    }

    // kills: an unbounded outer loop, and sealing a roster whose membership
    // never held still for one walk.
    #[test]
    fn membership_that_never_holds_refuses_by_name() {
        let memory = moving_army_memory(false);
        let started = std::time::Instant::now();
        let error = read_two_country_roster(&memory)
            .err()
            .expect("membership that keeps moving must refuse");
        assert!(
            matches!(&error, CarrierError::Invalid(reason)
                if reason == ROSTER_MEMBERSHIP_UNWITNESSED),
            "moving membership produced {error:?}"
        );
        assert_eq!(
            memory.reads.get(),
            3 * ROSTER_MEMBERSHIP_WITNESS_ATTEMPTS,
            "exactly ROSTER_MEMBERSHIP_WITNESS_ATTEMPTS whole-roster walks"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_millis(500),
            "the bounded refusal must not wait"
        );
    }

    // kills: a membership witness that reads a different set of armies than
    // the walk admits, which would either refuse every clean frame or witness
    // nothing at all.
    #[test]
    fn the_membership_witness_names_the_walks_armies() {
        let memory = single_group_memory(ARMY_ID_TYPE, 14_646);
        let root = CountryRoot {
            tag_db_id: 1,
            object_ptr: FIXTURE_COUNTRY,
        };
        let mut buffers = WalkBuffers::new();
        let mut leaf_rows = 0usize;
        let groups = read_country_groups(&memory, FIXTURE_BASE, &root, &mut leaf_rows, &mut buffers)
            .expect("a static graph must walk");
        let witnessed =
            read_country_army_membership(&memory, &root).expect("a static graph must witness");
        let mut walked = groups.iter().map(|group| group.army).collect::<Vec<_>>();
        walked.sort();
        assert_eq!(witnessed, walked);
        assert_eq!(witnessed.len(), 1);
    }

    /// A root that stops authenticating as the country it was admitted as,
    /// after `holds_for` proving reads.
    struct RecycledRoot {
        holds_for: usize,
        after: RootIdentity,
        checks: std::cell::Cell<usize>,
    }

    impl CountryRootIdentity for RecycledRoot {
        fn reprove_root(&self, _root: &CountryRoot) -> RootIdentity {
            let index = self.checks.get();
            self.checks.set(index + 1);
            if index < self.holds_for {
                RootIdentity::Proved
            } else {
                self.after
            }
        }
    }

    // kills: publishing a recycled country object's graph under the tag the
    // roster authenticated before the walk. The roster proves each root once
    // per frame; a restart that re-reads only the graph would settle on the
    // NEW object and seal it under the OLD tag.
    #[test]
    fn a_root_that_stops_authenticating_refuses_by_name() {
        let memory = single_group_memory(ARMY_ID_TYPE, 14_646);
        let identity = RecycledRoot {
            holds_for: 0,
            after: RootIdentity::Disproved,
            checks: std::cell::Cell::new(0),
        };
        let error = read_snapshot_from_country_roots(
            &memory,
            FIXTURE_BASE,
            1,
            &[CountryRoot {
                tag_db_id: 1,
                object_ptr: FIXTURE_COUNTRY,
            }],
            &identity,
            &mut CoherenceCost::default(),
        )
        .err()
        .expect("a root that no longer authenticates must refuse");
        assert!(
            matches!(&error, CarrierError::CountryRead { tag_db_id: 1, source, .. }
                if matches!(source.as_ref(), CarrierError::Invalid(reason)
                    if reason == ROOT_IS_NOT_THIS_COUNTRY)),
            "a recycled root produced {error:?}"
        );
        assert_eq!(identity.checks.get(), 1);
    }

    // kills: spending the restart bound on a root that reads stably as another
    // country. Re-reading cannot turn it back, so the refusal must be the
    // first read's, not an exhausted one eight reads later.
    #[test]
    fn a_disproved_root_does_not_spend_the_restart_bound() {
        let memory = single_group_memory(ARMY_ID_TYPE, 14_646);
        let mut cost = CoherenceCost::default();
        let identity = RecycledRoot {
            holds_for: 0,
            after: RootIdentity::Disproved,
            checks: std::cell::Cell::new(0),
        };
        let _ = read_snapshot_from_country_roots(
            &memory,
            FIXTURE_BASE,
            1,
            &[CountryRoot {
                tag_db_id: 1,
                object_ptr: FIXTURE_COUNTRY,
            }],
            &identity,
            &mut cost,
        );
        assert_eq!(cost, CoherenceCost::default());
    }

    // kills: renaming an unreadable identity window as the recycled-root
    // refusal. The two take different answers from a reader -- one is a
    // standing defect at a known address, the other is a live object that is
    // not this country -- so they cannot share a name.
    #[test]
    fn an_unreadable_root_identity_refuses_under_its_own_name() {
        let memory = single_group_memory(ARMY_ID_TYPE, 14_646);
        let identity = RecycledRoot {
            holds_for: 0,
            after: RootIdentity::Unreadable,
            checks: std::cell::Cell::new(0),
        };
        let error = read_snapshot_from_country_roots(
            &memory,
            FIXTURE_BASE,
            1,
            &[CountryRoot {
                tag_db_id: 1,
                object_ptr: FIXTURE_COUNTRY,
            }],
            &identity,
            &mut CoherenceCost::default(),
        )
        .err()
        .expect("an unreadable root identity must refuse");
        assert!(
            matches!(&error, CarrierError::CountryRead { source, .. }
                if matches!(source.as_ref(), CarrierError::Invalid(reason)
                    if reason == ROOT_IDENTITY_UNREADABLE)),
            "an unreadable identity produced {error:?}"
        );
    }

    /// A root whose identity window is being WRITTEN for the first
    /// `torn_for` reads and reads stably afterwards.
    struct TornThenStable {
        torn_for: usize,
        checks: std::cell::Cell<usize>,
    }

    impl CountryRootIdentity for TornThenStable {
        fn reprove_root(&self, _root: &CountryRoot) -> RootIdentity {
            let index = self.checks.get();
            self.checks.set(index + 1);
            if index < self.torn_for {
                RootIdentity::Torn
            } else {
                RootIdentity::Proved
            }
        }
    }

    // kills: treating an identity window that two successful reads disagreed
    // on as a standing refusal. That is the one identity outcome another read
    // can settle, so it must take the restart and the frame must seal --
    // refusing it would name a defect the engine does not have.
    #[test]
    fn a_torn_root_identity_is_retried_and_settles() {
        let memory = single_group_memory(ARMY_ID_TYPE, 14_646);
        let identity = TornThenStable {
            torn_for: 2,
            checks: std::cell::Cell::new(0),
        };
        let mut cost = CoherenceCost::default();
        let snapshot = read_snapshot_from_country_roots(
            &memory,
            FIXTURE_BASE,
            1,
            &[CountryRoot {
                tag_db_id: 1,
                object_ptr: FIXTURE_COUNTRY,
            }],
            &identity,
            &mut cost,
        )
        .unwrap_or_else(|error| panic!("a settling identity must seal: {error:?}"));
        assert_eq!(snapshot.countries[0].groups.len(), 1);
        // The identity proof is lane UD's own detector, so its restarts are
        // never evidence that a former refusal was converted.
        assert_eq!(cost.added_detector_tears, 2);
        assert_eq!(cost.legacy_tears_restarted, 0);
        assert!(cost.extra_country_reads > 0);
    }

    /// A memory that serves one country's status POINTER according to a
    /// schedule: `plan(n)` may replace the value the nth read of that pointer
    /// sees, counting from 1 across the whole call.
    ///
    /// The schedule for a one-country roster, which
    /// `the_status_pointer_read_schedule_is_what_the_tear_tests_assume` pins:
    /// each coherent pass opens on the pointer and closes on it, so two
    /// settling passes take reads 1-4; the witness opens on 5 and closes on 6.
    struct PointerScheduleMemory {
        inner: SparseMemory,
        pointer_at: u64,
        plan: fn(usize) -> Option<u64>,
        reads: std::cell::Cell<usize>,
    }

    impl PointerScheduleMemory {
        fn new(plan: fn(usize) -> Option<u64>) -> Self {
            Self {
                inner: single_group_memory(ARMY_ID_TYPE, 14_646),
                pointer_at: FIXTURE_COUNTRY + COUNTRY_UPGRADE_STATUS_PTR_OFF,
                plan,
                reads: std::cell::Cell::new(0),
            }
        }

        fn read_one_country(&self, cost: &mut CoherenceCost) -> Result<ArmyUpgradeSnapshot, CarrierError> {
            read_snapshot_from_country_roots(
                self,
                FIXTURE_BASE,
                1,
                &[CountryRoot {
                    tag_db_id: 1,
                    object_ptr: FIXTURE_COUNTRY,
                }],
                &StableRoots,
                cost,
            )
        }
    }

    /// Some other live status object: a REPLACEMENT, not a null.
    const OTHER_STATUS: u64 = FIXTURE_STATUS + 0x40;

    impl ArmyUpgradeMemory for PointerScheduleMemory {
        fn read_exact(&self, address: u64, out: &mut [u8]) -> Result<(), ()> {
            self.inner.read_exact(address, out)?;
            if address == self.pointer_at && out.len() == 8 {
                let index = self.reads.get() + 1;
                self.reads.set(index);
                if let Some(value) = (self.plan)(index) {
                    out.copy_from_slice(&value.to_le_bytes());
                }
            }
            Ok(())
        }
    }

    // kills: a tear raised by the membership witness refusing the frame on one
    // look. The witness's own reads race the engine exactly as the walk's do,
    // so a straddled witness must cost a roster walk, not an hour -- and the
    // old single-read name must not reach the wire from this path.
    #[test]
    fn a_torn_membership_witness_walks_the_roster_again() {
        let memory =
            PointerScheduleMemory::new(|read| (read == 6).then_some(OTHER_STATUS));
        let mut cost = CoherenceCost::default();
        let snapshot = memory
            .read_one_country(&mut cost)
            .unwrap_or_else(|error| panic!("a witness tear must be retried, not refused: {error:?}"));
        assert_eq!(snapshot.countries[0].groups.len(), 1);
        assert_eq!(cost.extra_roster_walks, 1);
        // The witness is lane UD's own detector: a tear it catches is not a
        // refusal the old code would have made.
        assert_eq!(cost.added_detector_tears, 1);
        assert_eq!(cost.legacy_tears_restarted, 0);
        // Two settled walks of two passes, each pass opening and closing on
        // the pointer, plus two witnesses: 12. Asserted so that a change to
        // the read schedule fails HERE rather than silently moving `tear_on`
        // onto some other read (Codex round 4, finding 3).
        assert_eq!(memory.reads.get(), 12);
    }

    /// A memory that makes the group vector header read as STRUCTURALLY
    /// INVALID -- a count past its capacity -- on selected reads of the
    /// closing header address, counting from 1.
    ///
    /// Only closing reads land on that address: the walk admits the header out
    /// of the widened status block. For a one-country roster the reads are
    /// pass 1's close, pass 2's close, then the witness's open and close.
    struct InvalidClosingHeaderMemory {
        inner: SparseMemory,
        header_at: u64,
        corrupt: fn(usize) -> bool,
        reads: std::cell::Cell<usize>,
    }

    impl InvalidClosingHeaderMemory {
        fn new(corrupt: fn(usize) -> bool) -> Self {
            Self {
                inner: single_group_memory(ARMY_ID_TYPE, 14_646),
                header_at: FIXTURE_STATUS + STATUS_GROUP_BEGIN_OFF,
                corrupt,
                reads: std::cell::Cell::new(0),
            }
        }

        fn read_one_country(
            &self,
            cost: &mut CoherenceCost,
        ) -> Result<ArmyUpgradeSnapshot, CarrierError> {
            read_snapshot_from_country_roots(
                self,
                FIXTURE_BASE,
                1,
                &[CountryRoot {
                    tag_db_id: 1,
                    object_ptr: FIXTURE_COUNTRY,
                }],
                &StableRoots,
                cost,
            )
        }
    }

    impl ArmyUpgradeMemory for InvalidClosingHeaderMemory {
        fn read_exact(&self, address: u64, out: &mut [u8]) -> Result<(), ()> {
            self.inner.read_exact(address, out)?;
            if address == self.header_at && out.len() == VECTOR_HEADER_LEN {
                let index = self.reads.get() + 1;
                self.reads.set(index);
                if (self.corrupt)(index) {
                    // count past capacity: what a half-written header looks
                    // like, and what the admission rules reject.
                    out[12..16].copy_from_slice(&i32::MAX.to_le_bytes());
                }
            }
            Ok(())
        }
    }

    // kills: refusing a frame on ONE read because the closing re-verification
    // caught a header mid-write. The admitted header was valid and its entries
    // were walked, so a structurally invalid FRESH header is a tear, and the
    // whole point of this lane is that a tear is re-read rather than published
    // as a refusal.
    #[test]
    fn a_structurally_invalid_closing_header_is_retried_not_refused() {
        let memory = InvalidClosingHeaderMemory::new(|read| read == 1);
        let mut cost = CoherenceCost::default();
        let snapshot = memory
            .read_one_country(&mut cost)
            .unwrap_or_else(|error| panic!("a torn closing header must be retried: {error:?}"));
        assert_eq!(snapshot.countries[0].groups.len(), 1);
        // Lane QZ's own close found it, so it is the capture gate's counter.
        assert_eq!(cost.legacy_tears_restarted, 1);
        assert_eq!(cost.added_detector_tears, 0);
    }

    // kills: naming a closing header tear after the ADMISSION rule it tripped.
    // "AUR1 army-upgrade group vector header is invalid" says the vector is
    // malformed; this one was walked successfully and then rewritten.
    #[test]
    fn a_closing_header_that_never_settles_exhausts_as_a_tear() {
        let memory = InvalidClosingHeaderMemory::new(|_| true);
        let mut cost = CoherenceCost::default();
        let error = memory
            .read_one_country(&mut cost)
            .err()
            .expect("a header that is never coherent must refuse");
        assert!(
            matches!(&error, CarrierError::CountryRead { source, .. }
                if matches!(source.as_ref(), CarrierError::Invalid(reason)
                    if reason == TORN_COUNTRY_GRAPH_EXHAUSTED)),
            "a permanently torn closing header produced {error:?}"
        );
        assert_eq!(
            cost.legacy_tears_restarted,
            COHERENT_READ_MAX_PASSES as u64 - 1
        );
    }

    // kills: the witness's own closing header check refusing the frame on one
    // look when it catches the same mid-write. Reads 1 and 2 are the two
    // coherent passes' closes; 3 and 4 are the witness opening and closing.
    #[test]
    fn a_structurally_invalid_witness_close_walks_the_roster_again() {
        let memory = InvalidClosingHeaderMemory::new(|read| read == 4);
        let mut cost = CoherenceCost::default();
        let snapshot = memory
            .read_one_country(&mut cost)
            .unwrap_or_else(|error| panic!("a torn witness close must be retried: {error:?}"));
        assert_eq!(snapshot.countries[0].groups.len(), 1);
        assert_eq!(cost.extra_roster_walks, 1);
        assert_eq!(cost.added_detector_tears, 1);
        assert_eq!(cost.legacy_tears_restarted, 0);
    }

    // kills: the witness's OPENING header refusing the frame on one look. Two
    // coherent reads have already accepted and closed this vector, so a header
    // the witness finds malformed is one being written, and it must cost a
    // roster walk like any other witness tear.
    #[test]
    fn a_structurally_invalid_witness_open_walks_the_roster_again() {
        let memory = InvalidClosingHeaderMemory::new(|read| read == 3);
        let mut cost = CoherenceCost::default();
        let snapshot = memory
            .read_one_country(&mut cost)
            .unwrap_or_else(|error| panic!("a torn witness open must be retried: {error:?}"));
        assert_eq!(snapshot.countries[0].groups.len(), 1);
        assert_eq!(cost.extra_roster_walks, 1);
        assert_eq!(cost.added_detector_tears, 1);
        assert_eq!(cost.legacy_tears_restarted, 0);
    }

    /// A memory that serves one country's single group-slot pointer as
    /// `u64::MAX` on a chosen read of that slot, counting from 1.
    ///
    /// Both the walk and the witness read that address, so the schedule for a
    /// one-country roster is: pass 1, pass 2, then the witness.
    struct OverflowingSlotMemory {
        inner: SparseMemory,
        slot_at: u64,
        overflow_on: usize,
        reads: std::cell::Cell<usize>,
    }

    impl ArmyUpgradeMemory for OverflowingSlotMemory {
        fn read_exact(&self, address: u64, out: &mut [u8]) -> Result<(), ()> {
            self.inner.read_exact(address, out)?;
            if address == self.slot_at && out.len() == 8 {
                let index = self.reads.get() + 1;
                self.reads.set(index);
                if index == self.overflow_on {
                    out.copy_from_slice(&u64::MAX.to_le_bytes());
                }
            }
            Ok(())
        }
    }

    fn read_one_country_over<M: ArmyUpgradeMemory>(
        memory: &M,
        cost: &mut CoherenceCost,
    ) -> Result<ArmyUpgradeSnapshot, CarrierError> {
        read_snapshot_from_country_roots(
            memory,
            FIXTURE_BASE,
            1,
            &[CountryRoot {
                tag_db_id: 1,
                object_ptr: FIXTURE_COUNTRY,
            }],
            &StableRoots,
            cost,
        )
    }

    // kills: an ADDRESS that cannot be formed inside the witness refusing the
    // hour on one look. `add(..)?` runs before any wrapper the call sits
    // inside, so a pointer read as u64::MAX during the witness used to escape
    // as a standing arithmetic overflow -- the same unit a genuinely
    // impossible address gets -- instead of costing a roster walk.
    #[test]
    fn a_witness_only_pointer_overflow_walks_the_roster_again() {
        let memory = OverflowingSlotMemory {
            inner: single_group_memory(ARMY_ID_TYPE, 14_646),
            slot_at: FIXTURE_GROUP_SLOTS,
            overflow_on: 3,
            reads: std::cell::Cell::new(0),
        };
        let mut cost = CoherenceCost::default();
        let snapshot = read_one_country_over(&memory, &mut cost)
            .unwrap_or_else(|error| panic!("a witness-only overflow must be retried: {error:?}"));
        assert_eq!(snapshot.countries[0].groups.len(), 1);
        assert_eq!(cost.extra_roster_walks, 1);
        assert_eq!(cost.added_detector_tears, 1);
        assert_eq!(cost.legacy_tears_restarted, 0);
        // Two walks of two passes plus a witness each: 6.
        assert_eq!(memory.reads.get(), 6);
    }

    // kills: an address the WALK cannot form being renamed as a tear. Nothing
    // has validated this root, so no re-read is owed -- which is the boundary
    // the witness's conversion must not widen into.
    //
    // The root itself overflows, rather than a slot pointer: a slot read as
    // u64::MAX is declined by the widened copy and then refuses as a MEMORY
    // read at that address, before any nonzero-offset add can overflow (Codex
    // round 11, finding 2).
    #[test]
    fn a_walk_address_overflow_still_refuses_on_the_first_read() {
        let memory = single_group_memory(ARMY_ID_TYPE, 14_646);
        let mut cost = CoherenceCost::default();
        let error = read_snapshot_from_country_roots(
            &memory,
            FIXTURE_BASE,
            1,
            &[CountryRoot {
                tag_db_id: 1,
                object_ptr: u64::MAX,
            }],
            &StableRoots,
            &mut cost,
        )
        .err()
        .expect("an unformable address must refuse");
        assert!(
            matches!(&error, CarrierError::CountryRead { source, .. }
                if matches!(source.as_ref(), CarrierError::Invalid(reason)
                    if reason == "AUR1 address overflow")),
            "a walk-time overflow produced {error:?}"
        );
        assert_eq!(cost.legacy_tears_restarted, 0);
        assert_eq!(cost.added_detector_tears, 0);
    }

    // kills: retrying an UNREADABLE root identity once a pass has completed.
    // The post-admission conversion must not reach the two identity refusals
    // no re-read can heal, or the bound is spent on a dead read.
    #[test]
    fn an_unreadable_identity_after_a_completed_pass_still_refuses_at_once() {
        let memory = single_group_memory(ARMY_ID_TYPE, 14_646);
        let identity = RecycledRoot {
            holds_for: 1,
            after: RootIdentity::Unreadable,
            checks: std::cell::Cell::new(0),
        };
        let mut cost = CoherenceCost::default();
        let error = read_snapshot_from_country_roots(
            &memory,
            FIXTURE_BASE,
            1,
            &[CountryRoot {
                tag_db_id: 1,
                object_ptr: FIXTURE_COUNTRY,
            }],
            &identity,
            &mut cost,
        )
        .err()
        .expect("an unreadable identity must refuse whenever it is raised");
        assert!(
            matches!(&error, CarrierError::CountryRead { source, .. }
                if matches!(source.as_ref(), CarrierError::Invalid(reason)
                    if reason == ROOT_IDENTITY_UNREADABLE)),
            "an unreadable identity after a completed pass produced {error:?}"
        );
        assert_eq!(identity.checks.get(), 2);
        assert_eq!(cost.legacy_tears_restarted, 0);
    }

    // kills: filing a RECYCLED root as a structural tear because its
    // replacement's graph refused first. The identity answer outranks the
    // graph refusal, so the frame names the object that is no longer this
    // country rather than the vector that stopped making sense because of it.
    #[test]
    fn a_recycled_root_outranks_the_graph_refusal_it_caused() {
        let memory = StatusBlockRegressionMemory {
            inner: single_group_memory(ARMY_ID_TYPE, 14_646),
            block_at: FIXTURE_STATUS,
            invalid_on: 2,
            reads: std::cell::Cell::new(0),
        };
        let identity = RecycledRoot {
            holds_for: 1,
            after: RootIdentity::Disproved,
            checks: std::cell::Cell::new(0),
        };
        let mut cost = CoherenceCost::default();
        let error = read_snapshot_from_country_roots(
            &memory,
            FIXTURE_BASE,
            1,
            &[CountryRoot {
                tag_db_id: 1,
                object_ptr: FIXTURE_COUNTRY,
            }],
            &identity,
            &mut cost,
        )
        .err()
        .expect("a recycled root must refuse");
        assert!(
            matches!(&error, CarrierError::CountryRead { source, .. }
                if matches!(source.as_ref(), CarrierError::Invalid(reason)
                    if reason == ROOT_IS_NOT_THIS_COUNTRY)),
            "a recycled root behind a graph refusal produced {error:?}"
        );
        assert_eq!(cost.legacy_tears_restarted, 0);
        assert_eq!(cost.added_detector_tears, 0);
    }

    // kills: a later pass refusing the hour on one look after an earlier pass
    // read the same country whole. Read 1 admits it, read 2 sees a header the
    // admission rules reject, and read 3 settles: that is a graph changing
    // under the walk, not a malformed vector.
    #[test]
    fn a_structure_that_regresses_after_a_completed_pass_is_retried() {
        let regressing = StatusBlockRegressionMemory {
            inner: single_group_memory(ARMY_ID_TYPE, 14_646),
            block_at: FIXTURE_STATUS,
            invalid_on: 2,
            reads: std::cell::Cell::new(0),
        };
        let mut cost = CoherenceCost::default();
        let snapshot = read_snapshot_from_country_roots(
            &regressing,
            FIXTURE_BASE,
            1,
            &[CountryRoot {
                tag_db_id: 1,
                object_ptr: FIXTURE_COUNTRY,
            }],
            &StableRoots,
            &mut cost,
        )
        .unwrap_or_else(|error| panic!("a regressed structure must be retried: {error:?}"));
        assert_eq!(snapshot.countries[0].groups.len(), 1);
        // Lane QZ's admission validator raised it and lane QZ refused the hour
        // on it, so healing it is a conversion the capture gate counts.
        assert_eq!(cost.legacy_tears_restarted, 1);
        assert_eq!(cost.added_detector_tears, 0);
    }

    /// A memory that serves one country's STATUS BLOCK with a group count past
    /// its capacity on a chosen read of that block, counting from 1. The block
    /// is what each coherent pass admits its header from, so read n is pass n.
    struct StatusBlockRegressionMemory {
        inner: SparseMemory,
        block_at: u64,
        invalid_on: usize,
        reads: std::cell::Cell<usize>,
    }

    impl ArmyUpgradeMemory for StatusBlockRegressionMemory {
        fn read_exact(&self, address: u64, out: &mut [u8]) -> Result<(), ()> {
            self.inner.read_exact(address, out)?;
            if address == self.block_at && out.len() == STATUS_BLOCK_LEN {
                let index = self.reads.get() + 1;
                self.reads.set(index);
                if index == self.invalid_on {
                    let at = STATUS_GROUP_COUNT_OFF as usize;
                    out[at..at + 4].copy_from_slice(&i32::MAX.to_le_bytes());
                }
            }
            Ok(())
        }
    }

    // kills: the tests above indexing onto reads other than the ones their
    // comments name. It pins the closing-header schedule.
    #[test]
    fn the_closing_header_read_schedule_is_what_those_tests_assume() {
        let memory = InvalidClosingHeaderMemory::new(|_| false);
        memory
            .read_one_country(&mut CoherenceCost::default())
            .expect("a static graph must seal on the first walk");
        assert_eq!(memory.reads.get(), 4);
    }

    // kills: charging a LEGACY tear to the added-detector counter, which would
    // empty the one counter the capture gate in FINDING.md section 5a rests
    // on. Read 2 is the first coherent pass's closing re-verification, which
    // lane QZ already had and which refused the whole frame before lane UD.
    #[test]
    fn a_legacy_tear_that_settles_is_charged_to_the_legacy_counter() {
        let memory =
            PointerScheduleMemory::new(|read| (read == 2).then_some(OTHER_STATUS));
        let mut cost = CoherenceCost::default();
        memory
            .read_one_country(&mut cost)
            .unwrap_or_else(|error| panic!("a legacy tear must be retried: {error:?}"));
        assert_eq!(cost.legacy_tears_restarted, 1);
        assert_eq!(cost.added_detector_tears, 0);
        assert_eq!(cost.extra_roster_walks, 0);
    }

    // kills: reporting a REPLACED status object as its group vector being
    // rewritten. The two are different events and lane QZ's close used to
    // raise one name for both, so the exhausted name a capture carries would
    // have sent a reader to the wrong field (Codex round 5, finding 1).
    #[test]
    fn a_replaced_status_pointer_exhausts_under_the_pointer_name() {
        // Every closing read of the pointer, which is every even read.
        let memory =
            PointerScheduleMemory::new(|read| (read % 2 == 0).then_some(OTHER_STATUS));
        let mut cost = CoherenceCost::default();
        let error = memory
            .read_one_country(&mut cost)
            .err()
            .expect("a pointer that never settles must refuse");
        assert!(
            matches!(&error, CarrierError::CountryRead { source, .. }
                if matches!(source.as_ref(), CarrierError::Invalid(reason)
                    if reason == TORN_STATUS_POINTER_EXHAUSTED)),
            "a replaced status pointer produced {error:?}"
        );
        // Lane QZ's close found it, so the capture gate's counter is the one
        // that moves.
        // Eight reads tore; seven of them were followed by another read, and
        // the eighth exhausted.
        assert_eq!(
            cost.legacy_tears_restarted,
            COHERENT_READ_MAX_PASSES as u64 - 1
        );
        assert_eq!(cost.added_detector_tears, 0);
    }

    // kills: charging lane UD's null close to the legacy counter, and naming a
    // country that ACQUIRED a status object as one whose object was replaced.
    #[test]
    fn an_appearing_status_pointer_exhausts_under_its_own_name_and_counter() {
        // The opening read of every pass sees no status object; the closing
        // read sees the one the fixture really has.
        let memory = PointerScheduleMemory::new(|read| (read % 2 == 1).then_some(0));
        let mut cost = CoherenceCost::default();
        let error = memory
            .read_one_country(&mut cost)
            .err()
            .expect("a status object that keeps appearing must refuse");
        assert!(
            matches!(&error, CarrierError::CountryRead { source, .. }
                if matches!(source.as_ref(), CarrierError::Invalid(reason)
                    if reason == TORN_STATUS_POINTER_APPEARED_EXHAUSTED)),
            "an appearing status pointer produced {error:?}"
        );
        assert_eq!(
            cost.added_detector_tears,
            COHERENT_READ_MAX_PASSES as u64 - 1
        );
        assert_eq!(cost.legacy_tears_restarted, 0);
    }

    // kills: the two tear tests above silently testing a read other than the
    // one their comments name. It pins the schedule they index into.
    #[test]
    fn the_status_pointer_read_schedule_is_what_the_tear_tests_assume() {
        let memory = PointerScheduleMemory::new(|_| None);
        memory
            .read_one_country(&mut CoherenceCost::default())
            .expect("a static graph must seal on the first walk");
        // Two coherent passes opening and closing on the pointer, then one
        // witness doing the same: reads 1-4 are the walk, 5 and 6 the witness.
        assert_eq!(memory.reads.get(), 6);
    }

    // kills: an identity proof taken once and trusted for the rest of the
    // country's restarts -- the proof must hold on the read that is ACCEPTED,
    // not merely on some earlier one.
    #[test]
    fn the_root_identity_must_hold_on_the_accepted_read() {
        let memory = single_group_memory(ARMY_ID_TYPE, 14_646);
        let identity = RecycledRoot {
            holds_for: 1,
            after: RootIdentity::Disproved,
            checks: std::cell::Cell::new(0),
        };
        let error = read_snapshot_from_country_roots(
            &memory,
            FIXTURE_BASE,
            1,
            &[CountryRoot {
                tag_db_id: 1,
                object_ptr: FIXTURE_COUNTRY,
            }],
            &identity,
            &mut CoherenceCost::default(),
        )
        .err()
        .expect("one holding read cannot admit a country");
        assert!(
            matches!(&error, CarrierError::CountryRead { source, .. }
                if matches!(source.as_ref(), CarrierError::Invalid(reason)
                    if reason == ROOT_IS_NOT_THIS_COUNTRY)),
            "one hold then a disproved root produced {error:?}"
        );
        assert_eq!(identity.checks.get(), 2);
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
        &StableRoots,
        &mut CoherenceCost::default(),
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
        &StableRoots,
        &mut CoherenceCost::default(),
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
        // The group vector header is the one address BOTH the walk and the
        // membership witness re-read: once per coherent pass at the close of
        // the walk, and twice in the witness, which opens on it and closes by
        // re-reading it. Two passes and one witness is four.
        assert_eq!(
            widened_reads
                .iter()
                .filter(|(at, served)| *at == fixture.status + STATUS_GROUP_BEGIN_OFF
                    && *served == VECTOR_HEADER_LEN)
                .count(),
            2 + 2,
            "the group vector header must be served once per pass and twice per witness"
        );

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
        &StableRoots,
        &mut CoherenceCost::default(),
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
        &StableRoots,
        &mut CoherenceCost::default(),
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
        &StableRoots,
        &mut CoherenceCost::default(),
        )
        .unwrap();
        let rich = receipt_frame(snapshot);
        assert_eq!(encode_payload(&rich).unwrap().len(), encoded_length(&rich));
    }
}
